//! Sidechat: a chat that points at a source chat. Every prompt the engine
//! hands the harness is wrapped in the boundary instruction, the first turn
//! carries a bounded copy of the source transcript, the doc's user entry stays
//! the typed text, and the source chat is never touched.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use futures::StreamExt;
use futures::stream::BoxStream;

use zeron_doc::{
    MessagePart, MessageRole, MessageStatus, SessionCommandPayload, SessionMessageEntry,
};
use zeron_engine::{EngineCore, HarnessRegistry};
use zeron_harness::{Harness, HarnessError, RunControls};
use zeron_proto::{
    AgentEvent, Chat, DoneStatus, HarnessId, Model, ReasoningLevel, RunRequest, SandboxLevel,
    SteeringMode, ToolCall,
};

const SOURCE: &str = "chat-source";
const SIDE: &str = "chat-side";

const BOUNDARY: &str = "<sidechat_boundary>\nYou are in a sidechat. Treat all prior conversation as reference-only context. Do not continue any prior task automatically. Do not mutate files, git, or the workspace and do not run workspace-changing commands unless the latest user message explicitly asks you to do so after this boundary. Use this sidechat for focused explanation, safety checks, summaries, and alternatives.\n</sidechat_boundary>\n\n";

/// Records every prompt that crosses into the harness — through `run` and
/// through the steering mailbox — and answers with the test's chosen text plus
/// one tool call (tool parts must never reach a sidechat's context copy).
struct TestHarness {
    run_prompts: Arc<Mutex<Vec<String>>>,
    steer_prompts: Arc<Mutex<Vec<String>>>,
    reply: Mutex<String>,
    steerable: bool,
}

impl TestHarness {
    fn new(steerable: bool) -> Self {
        Self {
            run_prompts: Arc::new(Mutex::new(Vec::new())),
            steer_prompts: Arc::new(Mutex::new(Vec::new())),
            reply: Mutex::new("source answer".into()),
            steerable,
        }
    }
}

#[async_trait]
impl Harness for TestHarness {
    fn id(&self) -> HarnessId {
        HarnessId::Mock
    }
    fn display_name(&self) -> &str {
        "Sidechatter"
    }
    fn supports_steering(&self) -> bool {
        self.steerable
    }
    fn steering_mode(&self) -> SteeringMode {
        SteeringMode::TurnBoundary
    }
    fn reasoning_levels(&self) -> &[ReasoningLevel] {
        &[ReasoningLevel::Medium]
    }
    async fn models(&self) -> Result<Vec<Model>, HarnessError> {
        Ok(vec![])
    }
    async fn run(
        &self,
        request: RunRequest,
        mut controls: RunControls,
    ) -> Result<BoxStream<'static, Result<AgentEvent, HarnessError>>, HarnessError> {
        self.run_prompts
            .lock()
            .unwrap()
            .push(request.prompt.clone());
        let reply = self.reply.lock().unwrap().clone();
        let steer_prompts = self.steer_prompts.clone();
        let interrupt = controls.interrupt.clone();
        let (tx, rx) = tokio::sync::mpsc::channel::<Result<AgentEvent, HarnessError>>(64);
        for event in [
            AgentEvent::SessionStarted {
                harness: HarnessId::Mock,
                model: "mock-1".into(),
                tools: vec![],
                cwd: request.cwd.clone(),
                session_id: "sess-side".into(),
                assistant_message_id: "a-1".into(),
            },
            AgentEvent::ContextUsage {
                tokens: Some(1_000),
                window: Some(200_000),
            },
            AgentEvent::ToolCall {
                id: "tool-1".into(),
                call: ToolCall::Exec {
                    command: "grep-the-tree".into(),
                },
            },
            AgentEvent::TextDelta { text: reply },
            AgentEvent::Done {
                status: DoneStatus::Completed,
                result: None,
                error: None,
                session_id: Some("sess-side".into()),
            },
        ] {
            tx.send(Ok(event)).await.expect("buffered");
        }
        tokio::spawn(async move {
            let mut steering_open = true;
            loop {
                tokio::select! {
                    biased;
                    steer = controls.steering.recv(), if steering_open => match steer {
                        Some(message) => {
                            steer_prompts.lock().unwrap().push(message.prompt);
                            let boundary = AgentEvent::Steered {
                                assistant_message_id: None,
                                next_assistant_message_id: None,
                            };
                            if tx.send(Ok(boundary)).await.is_err() {
                                return;
                            }
                        }
                        None => steering_open = false,
                    },
                    () = interrupt.cancelled() => {
                        let _ = tx
                            .send(Ok(AgentEvent::Done {
                                status: DoneStatus::Interrupted,
                                result: None,
                                error: None,
                                session_id: None,
                            }))
                            .await;
                        return;
                    }
                }
            }
        });
        Ok(
            futures::stream::unfold(rx, |mut rx| async move { rx.recv().await.map(|e| (e, rx)) })
                .boxed(),
        )
    }
}

async fn wait_for<F: FnMut() -> bool>(mut predicate: F, what: &str) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(20);
    while !predicate() {
        assert!(
            tokio::time::Instant::now() < deadline,
            "timed out waiting for {what}"
        );
        tokio::time::sleep(Duration::from_millis(15)).await;
    }
}

fn entries(core: &EngineCore, chat_id: &str) -> Vec<SessionMessageEntry> {
    core.doc_host
        .open(chat_id)
        .ok()
        .and_then(|h| h.doc().read_entries().ok())
        .unwrap_or_default()
}

fn entry_text(entry: &SessionMessageEntry) -> String {
    entry
        .parts
        .iter()
        .filter_map(|part| match part {
            MessagePart::Text { text, .. } => Some(text.clone()),
            _ => None,
        })
        .collect()
}

fn run_payload(prompt: &str, message_id: &str, cwd: &str) -> SessionCommandPayload {
    SessionCommandPayload::Run {
        request: RunRequest {
            prompt: prompt.into(),
            harness: None,
            model: None,
            reasoning: None,
            model_options: Default::default(),
            cwd: cwd.into(),
            sandbox: SandboxLevel::WorkspaceWrite,
            permission_mode: zeron_proto::PermissionMode::Full,
            auto_approve: true,
            attachments: Vec::new(),
            resume: None,
            worktree: None,
            branch: None,
        },
        message_id: message_id.into(),
    }
}

struct Rig {
    core: EngineCore,
    harness: Arc<TestHarness>,
    cwd: String,
}

impl Rig {
    /// Queue a turn on `chat_id` and wait for it to settle.
    async fn turn(&self, chat_id: &str, prompt: &str, message_id: &str) {
        let before = entries(&self.core, chat_id).len();
        self.core
            .doc_host
            .queue_command(chat_id, run_payload(prompt, message_id, &self.cwd))
            .expect("queue run");
        wait_for(
            || entries(&self.core, chat_id).len() >= before + 2,
            "turn entries",
        )
        .await;
        wait_for(
            || {
                entries(&self.core, chat_id)
                    .last()
                    .is_some_and(|e| e.status == Some(MessageStatus::Complete))
            },
            "turn settles",
        )
        .await;
    }

    fn source_chat(&self) -> Chat {
        self.core
            .workspace
            .chat(SOURCE)
            .expect("read source row")
            .expect("source row exists")
    }
}

/// An engine with a pre-titled source chat (so the auto-titler never spawns a
/// side run) and nothing else.
async fn rig(dir: &std::path::Path, steerable: bool) -> Rig {
    let harness = Arc::new(TestHarness::new(steerable));
    let registry = HarnessRegistry::new();
    registry.register(harness.clone());
    let core = EngineCore::assemble(&dir.join("data"), Arc::new(registry), HarnessId::Mock, None)
        .expect("engine core assembles");
    let client = zeron_rpc::memory_client(core.rpc_service());
    client
        .call(
            zeron_rpc::methods::MUTATE,
            serde_json::json!({ "op": "createChat", "chatId": SOURCE, "deviceId": core.device_id }),
        )
        .await
        .expect("createChat");
    core.workspace
        .rename_chat(SOURCE, "Auth work")
        .expect("rename chat");
    Rig {
        core,
        harness,
        cwd: dir.to_string_lossy().into_owned(),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn the_first_turn_carries_the_source_copy_and_later_turns_do_not() {
    let tmp = tempfile::tempdir().unwrap();
    let rig = rig(tmp.path(), false).await;
    rig.turn(SOURCE, "why does login fail?", "src-1").await;

    rig.core
        .workspace
        .create_sidechat(SOURCE, SIDE)
        .expect("create sidechat");
    *rig.harness.reply.lock().unwrap() = "side answer".into();
    rig.turn(SIDE, "explain that in one line", "side-1").await;

    let first = rig.harness.run_prompts.lock().unwrap()[1].clone();
    assert!(first.starts_with(BOUNDARY), "{first}");
    assert_eq!(
        first,
        format!(
            "{BOUNDARY}<sidechat_context>\nUser: why does login fail?\n\nAssistant: source answer\n</sidechat_context>\n\n<latest_user_message>\nexplain that in one line\n</latest_user_message>"
        )
    );
    // The doc's user entry is the typed text, never the wrapped prompt.
    let side_entries = entries(&rig.core, SIDE);
    assert_eq!(side_entries[0].role, MessageRole::User);
    assert_eq!(entry_text(&side_entries[0]), "explain that in one line");

    rig.turn(SIDE, "and the fix?", "side-2").await;
    let second = rig.harness.run_prompts.lock().unwrap()[2].clone();
    assert_eq!(
        second,
        format!("{BOUNDARY}<latest_user_message>\nand the fix?\n</latest_user_message>")
    );
    assert!(!second.contains("<sidechat_context>"), "{second}");
}

#[tokio::test(flavor = "multi_thread")]
async fn the_copy_drops_tool_parts_and_respects_the_budget() {
    let tmp = tempfile::tempdir().unwrap();
    let rig = rig(tmp.path(), false).await;
    *rig.harness.reply.lock().unwrap() = "z".repeat(60_000);
    rig.turn(SOURCE, "the oldest question", "src-1").await;

    rig.core
        .workspace
        .create_sidechat(SOURCE, SIDE)
        .expect("create sidechat");
    *rig.harness.reply.lock().unwrap() = "side answer".into();
    rig.turn(SIDE, "summarize", "side-1").await;

    let prompt = rig.harness.run_prompts.lock().unwrap()[1].clone();
    let copy = prompt
        .split_once("<sidechat_context>\n")
        .and_then(|(_, rest)| rest.split_once("\n</sidechat_context>"))
        .expect("a context block")
        .0
        .to_string();
    assert_eq!(copy.chars().count(), 48_000, "the budget is spent exactly");
    assert!(
        copy.chars().all(|c| c == 'z'),
        "the oldest kept message is truncated from its start, so the `Assistant: ` label goes first"
    );
    assert!(
        !prompt.contains("the oldest question"),
        "the budget left no room for the older entry"
    );
    assert!(
        !prompt.contains("grep-the-tree"),
        "tool parts are not conversation"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn an_empty_source_yields_no_context_block() {
    let tmp = tempfile::tempdir().unwrap();
    let rig = rig(tmp.path(), false).await;
    rig.core
        .workspace
        .create_sidechat(SOURCE, SIDE)
        .expect("create sidechat");
    rig.turn(SIDE, "anything?", "side-1").await;

    let prompt = rig.harness.run_prompts.lock().unwrap()[0].clone();
    assert_eq!(
        prompt,
        format!("{BOUNDARY}<latest_user_message>\nanything?\n</latest_user_message>")
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn creation_inherits_the_source_and_refuses_nesting() {
    let tmp = tempfile::tempdir().unwrap();
    let rig = rig(tmp.path(), false).await;
    rig.turn(SOURCE, "why does login fail?", "src-1").await;
    let source = rig.source_chat();
    assert!(
        source.harness_session_id.is_some(),
        "the source ran, so it has a harness session to NOT copy"
    );

    let client = zeron_rpc::memory_client(rig.core.rpc_service());
    let created: Chat = serde_json::from_value(
        client
            .call(
                zeron_rpc::methods::CREATE_SIDECHAT,
                serde_json::json!({ "sourceChatId": SOURCE, "chatId": SIDE }),
            )
            .await
            .expect("CreateSidechat"),
    )
    .expect("a Chat row");

    assert_eq!(created.sidechat_source_chat_id.as_deref(), Some(SOURCE));
    assert_eq!(created.title.as_deref(), Some("Sidechat: Auth work"));
    assert_eq!(created.config, source.config);
    assert_eq!(created.cwd, source.cwd);
    assert_eq!(created.space_id, source.space_id);
    assert_eq!(created.checkout_id, source.checkout_id);
    assert_eq!(created.branch, source.branch);
    assert_eq!(created.device_id, source.device_id);
    assert_eq!(created.harness_session_id, None, "its own harness session");
    assert_eq!(created.harness_session_cwd, None);
    // The registry stores timestamps as epoch millis, so the reply's
    // sub-millisecond `createdAt` is the only field the round trip rounds.
    let stored = rig.core.workspace.chat(SIDE).unwrap().expect("stored row");
    assert_eq!(
        stored.created_at.timestamp_millis(),
        created.created_at.timestamp_millis()
    );
    assert_eq!(
        stored,
        Chat {
            created_at: stored.created_at,
            ..created
        }
    );

    let err = rig
        .core
        .workspace
        .create_sidechat(SIDE, "chat-side-side")
        .expect_err("no nesting");
    assert_eq!(err.to_string(), "nested sidechats are not supported");
    assert_eq!(rig.core.workspace.chat("chat-side-side").unwrap(), None);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_sidechat_turn_leaves_the_source_untouched() {
    let tmp = tempfile::tempdir().unwrap();
    let rig = rig(tmp.path(), false).await;
    rig.turn(SOURCE, "why does login fail?", "src-1").await;

    let before_entries = entries(&rig.core, SOURCE);
    let before_row = rig.source_chat();
    let before_usage = rig
        .core
        .doc_host
        .open(SOURCE)
        .unwrap()
        .doc()
        .context_usage();
    assert!(before_usage.is_some(), "the source has a meter to preserve");

    rig.core
        .workspace
        .create_sidechat(SOURCE, SIDE)
        .expect("create sidechat");
    rig.turn(SIDE, "explain that in one line", "side-1").await;

    assert_eq!(entries(&rig.core, SOURCE), before_entries);
    let after_row = rig.source_chat();
    assert_eq!(after_row.harness_session_id, before_row.harness_session_id);
    assert_eq!(
        after_row.harness_session_cwd,
        before_row.harness_session_cwd
    );
    assert_eq!(after_row, before_row, "the whole source row is untouched");
    assert_eq!(
        rig.core
            .doc_host
            .open(SOURCE)
            .unwrap()
            .doc()
            .context_usage(),
        before_usage
    );
}

/// The held queue and `SendQueuedMessageNow` reach the harness through
/// `SessionsEngine::steer`, which never enters `dispatch` — so the wrap has to
/// live on that path too.
#[tokio::test(flavor = "multi_thread")]
async fn a_steered_prompt_is_wrapped_too() {
    let tmp = tempfile::tempdir().unwrap();
    let rig = rig(tmp.path(), true).await;
    rig.turn(SOURCE, "why does login fail?", "src-1").await;
    rig.core
        .workspace
        .create_sidechat(SOURCE, SIDE)
        .expect("create sidechat");
    rig.turn(SIDE, "explain that in one line", "side-1").await;

    rig.core
        .sessions
        .steer(SIDE, "and the fix?", Some("side-2".into()))
        .await
        .expect("steer");
    wait_for(
        || !rig.harness.steer_prompts.lock().unwrap().is_empty(),
        "the mailbox receives the steer",
    )
    .await;

    let steered = rig.harness.steer_prompts.lock().unwrap()[0].clone();
    assert_eq!(
        steered,
        format!("{BOUNDARY}<latest_user_message>\nand the fix?\n</latest_user_message>")
    );
    assert!(
        entries(&rig.core, SIDE)
            .iter()
            .any(|entry| entry.role == MessageRole::User && entry_text(entry) == "and the fix?"),
        "the doc entry is the typed text"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn cached_context_tracks_source_edits_and_removed_answers() {
    let tmp = tempfile::tempdir().unwrap();
    let rig = rig(tmp.path(), false).await;
    rig.turn(SOURCE, "original context", "src-1").await;
    rig.core.workspace.create_sidechat(SOURCE, SIDE).unwrap();
    rig.turn(SIDE, "first question", "side-1").await;
    let side = rig.core.doc_host.open(SIDE).unwrap();
    let answer = entries(&rig.core, SIDE).last().unwrap().id.clone();
    side.doc()
        .set_message_status(&answer, MessageStatus::Aborted)
        .unwrap();
    rig.turn(SOURCE, "new source context", "src-2").await;
    rig.turn(SIDE, "retry question", "side-2").await;
    assert!(
        rig.harness
            .run_prompts
            .lock()
            .unwrap()
            .last()
            .unwrap()
            .contains("new source context")
    );
    rig.turn(SIDE, "follow-up", "side-3").await;
    assert!(
        !rig.harness
            .run_prompts
            .lock()
            .unwrap()
            .last()
            .unwrap()
            .contains("<sidechat_context>")
    );
    side.doc().truncate_from("side-1").unwrap();
    rig.turn(SIDE, "after rewind", "side-4").await;
    assert!(
        rig.harness
            .run_prompts
            .lock()
            .unwrap()
            .last()
            .unwrap()
            .contains("new source context")
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn expired_sidechat_rejects_dispatch_and_steering_without_removing_history() {
    let tmp = tempfile::tempdir().unwrap();
    let rig = rig(tmp.path(), true).await;
    let mut side = rig.core.workspace.create_sidechat(SOURCE, SIDE).unwrap();
    rig.turn(SIDE, "before expiry", "side-1").await;
    let before = entries(&rig.core, SIDE);
    side.created_at = chrono::Utc::now() - chrono::Duration::hours(24);
    rig.core.workspace.import_chat_row(&side).unwrap();
    let SessionCommandPayload::Run { request, .. } =
        run_payload("after expiry", "side-2", &rig.cwd)
    else {
        unreachable!()
    };
    let err = rig
        .core
        .sessions
        .dispatch(SIDE, HarnessId::Mock, request, Some("side-2".into()))
        .await
        .unwrap_err();
    assert!(err.to_string().contains("Sidechat expired"), "{err}");
    let err = rig
        .core
        .sessions
        .steer(SIDE, "after expiry", Some("side-3".into()))
        .await
        .unwrap_err();
    assert!(err.to_string().contains("Sidechat expired"), "{err}");
    assert_eq!(entries(&rig.core, SIDE), before);
    assert!(rig.harness.steer_prompts.lock().unwrap().is_empty());
    assert_eq!(rig.harness.run_prompts.lock().unwrap().len(), 1);
}
