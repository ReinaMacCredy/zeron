//! Provider switch mid-chat: `setChatConfig` with another harness tombstones
//! the chat's session, the new harness's first turn carries the conversation
//! as text, later turns resume its own session, and the old harness's
//! checkpoints can no longer be branched at.

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use futures::StreamExt;
use futures::stream::BoxStream;

use zeron_doc::{
    HandoffRoute, MessagePart, MessageRole, MessageStatus, SessionCommandPayload,
    SessionMessageEntry,
};
use zeron_engine::{EngineCore, HarnessRegistry};
use zeron_harness::{Harness, HarnessError, RunControls};
use zeron_proto::{
    AgentEvent, AnchorRole, DoneStatus, HarnessId, Model, ReasoningLevel, RunRequest, SandboxLevel,
    SessionBranch, SteeringMode,
};

const CHAT: &str = "chat-1";
const FORK: &str = "chat-fork";

#[derive(Debug, Clone, PartialEq)]
struct Seen {
    prompt: String,
    resume: Option<String>,
    branch: Option<SessionBranch>,
}

/// One registered harness: records every run it is asked for, mints a
/// session named after itself, and emits anchors like a real driver.
struct TestHarness {
    id: HarnessId,
    name: &'static str,
    runs: Arc<Mutex<Vec<Seen>>>,
    turns: AtomicUsize,
    fail_next: AtomicBool,
}

impl TestHarness {
    fn new(id: HarnessId, name: &'static str) -> Self {
        Self {
            id,
            name,
            runs: Arc::new(Mutex::new(Vec::new())),
            turns: AtomicUsize::new(0),
            fail_next: AtomicBool::new(false),
        }
    }
}

#[async_trait]
impl Harness for TestHarness {
    fn id(&self) -> HarnessId {
        self.id
    }
    fn display_name(&self) -> &str {
        self.name
    }
    fn supports_steering(&self) -> bool {
        false
    }
    fn steering_mode(&self) -> SteeringMode {
        SteeringMode::TurnBoundary
    }
    fn reasoning_levels(&self) -> &[ReasoningLevel] {
        &[ReasoningLevel::Medium]
    }
    fn branching(&self) -> bool {
        true
    }
    async fn models(&self) -> Result<Vec<Model>, HarnessError> {
        Ok(vec![])
    }
    async fn run(
        &self,
        request: RunRequest,
        _controls: RunControls,
    ) -> Result<BoxStream<'static, Result<AgentEvent, HarnessError>>, HarnessError> {
        self.runs.lock().unwrap().push(Seen {
            prompt: request.prompt.clone(),
            resume: request.resume.clone(),
            branch: request.branch.clone(),
        });
        if self.fail_next.swap(false, Ordering::SeqCst) {
            let events = vec![
                AgentEvent::Error {
                    message: "start failed".into(),
                },
                AgentEvent::Done {
                    status: DoneStatus::Errored,
                    result: None,
                    error: Some("start failed".into()),
                    session_id: None,
                },
            ];
            return Ok(futures::stream::iter(events.into_iter().map(Ok)).boxed());
        }
        let n = self.turns.fetch_add(1, Ordering::SeqCst) + 1;
        let session_id = match (&request.resume, &request.branch) {
            (Some(_), Some(_)) => format!("{}-forked", self.name),
            (Some(resume), None) => resume.clone(),
            (None, _) => format!("{}-sess", self.name),
        };
        let events = vec![
            AgentEvent::SessionStarted {
                harness: self.id,
                model: "m".into(),
                tools: vec![],
                cwd: request.cwd.clone(),
                session_id: session_id.clone(),
                assistant_message_id: format!("{}-a-{n}", self.name),
            },
            AgentEvent::Anchor {
                role: AnchorRole::User,
                id: format!("{}-user-{n}", self.name),
                entry_id: None,
            },
            AgentEvent::TextDelta {
                text: format!("{} answer {n}", self.name),
            },
            AgentEvent::Anchor {
                role: AnchorRole::Assistant,
                id: format!("{}-reply-{n}", self.name),
                entry_id: None,
            },
            AgentEvent::Done {
                status: DoneStatus::Completed,
                result: None,
                error: None,
                session_id: Some(session_id),
            },
        ];
        Ok(futures::stream::iter(events.into_iter().map(Ok)).boxed())
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
            forked_at: None,
            computer_control: false,
            mcp: None,
            disabled_tools: Vec::new(),
        },
        message_id: message_id.into(),
    }
}

struct Rig {
    core: EngineCore,
    client: zeron_rpc::RpcClient,
    old: Arc<TestHarness>,
    new: Arc<TestHarness>,
    cwd: String,
}

impl Rig {
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
                    && !self.core.sessions.turn_in_flight(chat_id)
            },
            "turn settles",
        )
        .await;
        wait_for(
            || {
                self.session(chat_id)
                    .is_some_and(|session| !session.is_empty())
            },
            "session row",
        )
        .await;
    }

    fn session(&self, chat_id: &str) -> Option<String> {
        self.core
            .workspace
            .chat(chat_id)
            .expect("read row")
            .expect("row exists")
            .harness_session_id
    }

    /// The UI's provider switch: `setChatConfig` naming the other harness,
    /// with the transcript note it writes alongside.
    async fn switch_to(&self, harness: &str, note: bool) {
        let mut params = serde_json::json!({
            "op": "setChatConfig",
            "chatId": CHAT,
            "config": {
                "harness": harness,
                "model": null,
                "reasoning": null,
                "modelOptions": {},
                "sandbox": "workspace-write",
            },
        });
        if note {
            params["note"] = serde_json::json!({
                "text": "Provider changed from Old to New.",
                "hint": "The conversation so far is handed to New as text.",
            });
        }
        self.client
            .call(zeron_rpc::methods::MUTATE, params)
            .await
            .expect("setChatConfig");
    }

    fn reply_id(&self, chat_id: &str, ix: usize) -> String {
        let entries = entries(&self.core, chat_id);
        assert_eq!(entries[ix].role, MessageRole::Assistant);
        entries[ix].id.clone()
    }

    fn prompt_id(&self, chat_id: &str, ix: usize) -> String {
        let entries = entries(&self.core, chat_id);
        assert_eq!(entries[ix].role, MessageRole::User);
        entries[ix].id.clone()
    }
}

async fn rig(dir: &std::path::Path) -> Rig {
    let old = Arc::new(TestHarness::new(HarnessId::Mock, "Old"));
    let new = Arc::new(TestHarness::new(HarnessId::Pi, "New"));
    let registry = HarnessRegistry::new();
    registry.register(old.clone());
    registry.register(new.clone());
    let core = EngineCore::assemble(&dir.join("data"), Arc::new(registry), HarnessId::Mock, None)
        .expect("engine core assembles");
    let client = zeron_rpc::memory_client(core.rpc_service());
    client
        .call(
            zeron_rpc::methods::MUTATE,
            serde_json::json!({ "op": "createChat", "chatId": CHAT, "deviceId": core.device_id }),
        )
        .await
        .expect("createChat");
    core.workspace
        .rename_chat(CHAT, "Auth work")
        .expect("rename chat");
    Rig {
        core,
        client,
        old,
        new,
        cwd: dir.to_string_lossy().into_owned(),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn the_new_harness_starts_fresh_with_the_conversation_as_text_then_resumes_its_own() {
    let tmp = tempfile::tempdir().unwrap();
    let rig = rig(tmp.path()).await;
    rig.turn(CHAT, "first", "m1").await;
    rig.turn(CHAT, "second", "m2").await;
    assert_eq!(rig.session(CHAT).as_deref(), Some("Old-sess"));

    rig.switch_to("pi", true).await;
    assert_eq!(
        rig.session(CHAT).as_deref(),
        Some(""),
        "the old session is tombstoned, never handed to the new harness"
    );
    let divider = entries(&rig.core, CHAT).into_iter().last().unwrap();
    assert_eq!(divider.role, MessageRole::System);
    assert_eq!(
        entry_text(&divider),
        "Provider changed from Old to New.The conversation so far is handed to New as text."
    );
    // The packet is on the divider from the moment of the switch.
    let packet = match &divider.parts[2] {
        MessagePart::Handoff {
            harness: HarnessId::Pi,
            text,
            ..
        } => text.clone(),
        other => panic!("{other:?}"),
    };
    assert!(packet.ends_with("Assistant: Old answer 2\n</previous_conversation>"));

    rig.turn(CHAT, "third", "m3").await;
    let seen = rig.new.runs.lock().unwrap().clone();
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].resume, None);
    assert_eq!(seen[0].branch, None);
    assert_eq!(
        seen[0].prompt,
        "<provider_switch>\nThis chat was moved to you from another agent. Its conversation so far is below as plain text; that agent's tool results and file state were not carried over. Continue from where it left off.\n</provider_switch>\n\n<previous_conversation>\nUser: first\n\nAssistant: Old answer 1\n\nUser: second\n\nAssistant: Old answer 2\n</previous_conversation>\n\n<latest_user_message>\nthird\n</latest_user_message>"
    );
    assert_eq!(
        rig.old.runs.lock().unwrap().len(),
        2,
        "the old harness is not run again"
    );
    // The first turn sends the recorded packet verbatim; the doc keeps the
    // typed text and does not record a second copy on the user entry.
    assert!(seen[0].prompt.starts_with(packet.as_str()));
    let doc = entries(&rig.core, CHAT);
    let prompt = &doc[doc.len() - 2];
    assert_eq!(entry_text(prompt), "third");
    assert_eq!(prompt.parts.len(), 1);
    assert_eq!(rig.session(CHAT).as_deref(), Some("New-sess"));

    rig.turn(CHAT, "fourth", "m4").await;
    let seen = rig.new.runs.lock().unwrap().clone();
    assert_eq!(seen[1].prompt, "fourth");
    assert_eq!(seen[1].resume.as_deref(), Some("New-sess"));
    // Switching back hands the conversation over again, and the recorded
    // packet is not conversation: it never nests inside the next one.
    rig.switch_to("mock", true).await;
    rig.turn(CHAT, "fifth", "m5").await;
    let seen = rig.old.runs.lock().unwrap().clone();
    let last = &seen.last().unwrap().prompt;
    assert_eq!(last.matches("<provider_switch>").count(), 1, "{last}");
    assert!(
        last.contains("Assistant: New answer 1\n\nUser: fourth\n\nAssistant: New answer 2\n</previous_conversation>"),
        "{last}"
    );
}

async fn switch(rig: &Rig, harness: &str, from: &str, to: &str) {
    rig.client
        .call(
        zeron_rpc::methods::MUTATE,
        serde_json::json!({
            "op": "setChatConfig",
            "chatId": CHAT,
            "config": {
                "harness": harness, "model": null, "reasoning": null,
                "modelOptions": {}, "sandbox": "workspace-write",
            },
            "note": {
                "text": format!("Provider changed from {from} to {to}."),
                "hint": format!("{from} session closed. Handed to {to}; {from}'s files were not carried."),
            },
        }),
        )
        .await
        .expect("switch");
}

fn previous_harnesses(rig: &Rig) -> Vec<HarnessId> {
    rig.core
        .workspace
        .chat(CHAT)
        .unwrap()
        .unwrap()
        .previous_harnesses
}

#[tokio::test(flavor = "multi_thread")]
async fn a_second_switch_before_any_message_replaces_the_first_divider() {
    let tmp = tempfile::tempdir().unwrap();
    let rig = rig(tmp.path()).await;
    rig.turn(CHAT, "first", "m1").await;
    switch(&rig, "pi", "Mock", "Pi").await;
    switch(&rig, "codex", "Pi", "Codex").await;
    let doc = entries(&rig.core, CHAT);
    // user, reply, one divider
    assert_eq!(doc.len(), 3);
    let divider = doc.last().unwrap();
    assert_eq!(divider.role, MessageRole::System);
    assert_eq!(
        entry_text(divider),
        "Provider changed from Mock to Codex.Mock session closed. Handed to Codex; Mock's files were not carried."
    );
    assert!(matches!(
        &divider.parts[2],
        MessagePart::Handoff {
            harness: HarnessId::Codex,
            ..
        }
    ));
    // The harness in between never ran, so it leaves no mark.
    assert_eq!(previous_harnesses(&rig), vec![HarnessId::Mock]);
}

#[tokio::test(flavor = "multi_thread")]
async fn switching_back_before_any_message_leaves_no_divider() {
    let tmp = tempfile::tempdir().unwrap();
    let rig = rig(tmp.path()).await;
    rig.turn(CHAT, "first", "m1").await;
    switch(&rig, "pi", "Old", "New").await;
    switch(&rig, "mock", "New", "Old").await;
    let doc = entries(&rig.core, CHAT);
    assert_eq!(doc.len(), 2);
    assert_eq!(previous_harnesses(&rig), vec![HarnessId::Mock]);

    // The session was still closed: the next turn hands over once, on the
    // user entry, to the harness the chat never visibly left.
    rig.turn(CHAT, "second", "m2").await;
    assert!(rig.new.runs.lock().unwrap().is_empty());
    let seen = rig.old.runs.lock().unwrap().clone();
    assert_eq!(seen.len(), 2);
    assert_eq!(seen[1].resume, None);
    assert_eq!(seen[1].prompt.matches("<provider_switch>").count(), 1);
    let doc = entries(&rig.core, CHAT);
    assert!(doc[2].parts.iter().any(|part| matches!(
        part,
        MessagePart::Handoff {
            harness: HarnessId::Mock,
            ..
        }
    )));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_switch_on_an_empty_chat_hands_nothing_over() {
    let tmp = tempfile::tempdir().unwrap();
    let rig = rig(tmp.path()).await;
    rig.switch_to("pi", false).await;
    rig.turn(CHAT, "first", "m1").await;
    let seen = rig.new.runs.lock().unwrap().clone();
    assert_eq!(seen[0].prompt, "first");
    assert_eq!(seen[0].resume, None);
    assert!(rig.old.runs.lock().unwrap().is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn old_checkpoints_cannot_be_branched_after_the_switch() {
    let tmp = tempfile::tempdir().unwrap();
    let rig = rig(tmp.path()).await;
    rig.turn(CHAT, "first", "m1").await;
    rig.switch_to("pi", true).await;
    rig.turn(CHAT, "second", "m2").await;
    rig.turn(CHAT, "third", "m3").await;
    let doc = entries(&rig.core, CHAT);
    // user, reply, divider, user, reply, user, reply
    assert_eq!(doc.len(), 7);

    let err = rig
        .core
        .sessions
        .fork_chat(CHAT, &rig.reply_id(CHAT, 1), FORK)
        .expect_err("the old harness's reply is not a fork point");
    assert!(err.to_string().contains("previous provider"), "{err}");
    assert!(rig.core.workspace.chat(FORK).unwrap().is_none());

    // A reply the new harness produced branches as usual.
    rig.core
        .sessions
        .fork_chat(CHAT, &rig.reply_id(CHAT, 4), FORK)
        .expect("fork after the switch");
    rig.turn(FORK, "on the fork", "m4").await;
    let seen = rig.new.runs.lock().unwrap().clone();
    assert_eq!(
        seen.last().unwrap().branch,
        Some(SessionBranch {
            keep: Some("New-reply-1".into()),
            drop: Some("New-user-2".into()),
        })
    );

    // Rewinding to before the first prompt after the switch lands on the old
    // harness's reply, which cannot be branched: the session is tombstoned
    // again and the next run gets the remaining conversation as text.
    rig.core
        .sessions
        .rewind_chat(CHAT, &rig.prompt_id(CHAT, 3))
        .await
        .expect("rewind across the switch");
    assert_eq!(rig.session(CHAT).as_deref(), Some(""));
    rig.turn(CHAT, "second, again", "m5").await;
    let seen = rig.new.runs.lock().unwrap().clone();
    let last = seen.last().unwrap();
    assert_eq!(last.resume, None);
    assert!(
        last.prompt.contains("<previous_conversation>\nUser: first\n\nAssistant: Old answer 1\n</previous_conversation>"),
        "{}",
        last.prompt
    );
}

async fn set_config(rig: &Rig, harness: &str, model: &str, text: &str) {
    rig.client
        .call(
            zeron_rpc::methods::MUTATE,
            serde_json::json!({
                "op": "setChatConfig",
                "chatId": CHAT,
                "config": {
                    "harness": harness, "model": model, "reasoning": null,
                    "modelOptions": {}, "sandbox": "workspace-write",
                },
                "note": { "text": text, "hint": "hint" },
            }),
        )
        .await
        .expect("setChatConfig");
}

fn handoff_route(entry: &SessionMessageEntry) -> (HarnessId, String, HandoffRoute) {
    entry
        .parts
        .iter()
        .find_map(|part| match part {
            MessagePart::Handoff {
                harness,
                text,
                route,
                ..
            } => Some((*harness, text.clone(), route.clone())),
            _ => None,
        })
        .expect("a handoff part")
}

#[tokio::test(flavor = "multi_thread")]
async fn a_model_change_after_an_unsent_switch_folds_into_its_divider() {
    let tmp = tempfile::tempdir().unwrap();
    let rig = rig(tmp.path()).await;
    set_config(
        &rig,
        "mock",
        "old-1",
        "Model changed from default to old-1.",
    )
    .await;
    rig.turn(CHAT, "first", "m1").await;
    set_config(&rig, "pi", "pi-1", "Provider changed from Mock to Pi.").await;
    set_config(&rig, "pi", "pi-2", "Model changed from pi-1 to pi-2.").await;

    let doc = entries(&rig.core, CHAT);
    // model note, user, reply, one handoff divider
    assert_eq!(doc.len(), 4, "{doc:#?}");
    let divider = doc.last().unwrap();
    assert_eq!(entry_text(divider), "Provider changed from Mock to Pi.hint");
    let (to, _, route) = handoff_route(divider);
    assert_eq!(to, HarnessId::Pi);
    assert_eq!(route.from_harness, Some(HarnessId::Mock));
    assert_eq!(route.from_model.as_deref(), Some("old-1"));
    assert_eq!(route.to_model.as_deref(), Some("pi-2"));
    assert_eq!(previous_harnesses(&rig), vec![HarnessId::Mock]);

    // The first turn carries the recorded packet once and records no second.
    rig.turn(CHAT, "second", "m2").await;
    let seen = rig.new.runs.lock().unwrap().clone();
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].prompt.matches("<provider_switch>").count(), 1);
    let doc = entries(&rig.core, CHAT);
    assert!(
        doc.iter()
            .filter(|entry| entry.role == MessageRole::User)
            .all(|entry| !entry
                .parts
                .iter()
                .any(|part| matches!(part, MessagePart::Handoff { .. })))
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_continued_thread_resends_its_source_packet_after_a_failed_first_turn() {
    const CONT: &str = "chat-cont";
    let tmp = tempfile::tempdir().unwrap();
    let rig = rig(tmp.path()).await;
    rig.turn(CHAT, "first", "m1").await;
    let chat: zeron_proto::Chat = serde_json::from_value(
        rig.client
            .call(
                zeron_rpc::methods::CONTINUE_IN_NEW_THREAD,
                serde_json::json!({
                    "sourceChatId": CHAT, "chatId": CONT, "harness": "pi", "model": "pi-1",
                }),
            )
            .await
            .expect("ContinueInNewThread"),
    )
    .unwrap();
    assert_eq!(chat.title.as_deref(), Some("Auth work (cont.)"));
    assert_eq!(chat.created_by_chat_id, None);
    assert_eq!(chat.previous_harnesses, vec![HarnessId::Mock]);
    assert_eq!(chat.config.as_ref().map(|c| c.harness), Some(HarnessId::Pi));
    let same: zeron_proto::Chat = serde_json::from_value(
        rig.client
            .call(
                zeron_rpc::methods::CONTINUE_IN_NEW_THREAD,
                serde_json::json!({
                    "sourceChatId": CHAT, "chatId": "chat-same", "harness": "mock",
                }),
            )
            .await
            .expect("ContinueInNewThread on the same agent"),
    )
    .unwrap();
    assert!(same.previous_harnesses.is_empty(), "one agent, one mark");
    let doc = entries(&rig.core, CONT);
    assert_eq!(doc.len(), 1, "no source message is copied");
    let (to, packet, route) = handoff_route(&doc[0]);
    assert_eq!(to, HarnessId::Pi);
    assert_eq!(route.from_harness, Some(HarnessId::Mock));
    assert_eq!(route.to_model.as_deref(), Some("pi-1"));
    assert_eq!(route.source_chat_id.as_deref(), Some(CHAT));
    assert_eq!(route.source_title.as_deref(), Some("Auth work"));
    assert!(packet.contains("User: first\n\nAssistant: Old answer 1"));

    rig.new.fail_next.store(true, Ordering::SeqCst);
    rig.core
        .doc_host
        .queue_command(CONT, run_payload("hello", "c1", &rig.cwd))
        .expect("queue run");
    wait_for(
        || {
            entries(&rig.core, CONT).len() >= 3
                && entries(&rig.core, CONT)
                    .last()
                    .is_some_and(|e| e.status == Some(MessageStatus::Complete))
                && !rig.core.sessions.turn_in_flight(CONT)
        },
        "failed first turn settles",
    )
    .await;

    rig.core
        .doc_host
        .queue_command(CONT, run_payload("hello again", "c2", &rig.cwd))
        .expect("queue run");
    wait_for(|| rig.new.runs.lock().unwrap().len() >= 2, "re-sent turn").await;
    let seen = rig.new.runs.lock().unwrap().clone();
    assert_eq!(seen[1].resume, None);
    assert!(
        seen[1].prompt.starts_with(packet.as_str()),
        "{}",
        seen[1].prompt
    );
    assert!(
        seen[1]
            .prompt
            .ends_with("<latest_user_message>\nhello again\n</latest_user_message>")
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_chat_handed_off_to_a_project_points_at_itself_unless_read_thread_is_off() {
    const LINKED: &str = "chat-linked";
    const COPIED: &str = "chat-copied";
    let tmp = tempfile::tempdir().unwrap();
    let rig = rig(tmp.path()).await;
    rig.core.sessions.set_tool_server_port(1);
    let project = tmp.path().join("api");
    std::fs::create_dir(&project).unwrap();
    let project_path = project.to_string_lossy().into_owned();
    rig.core
        .workspace
        .create_space("space-api", &rig.core.device_id, &project_path, None, false)
        .unwrap();
    rig.turn(CHAT, "first", "m1").await;
    let source = rig.core.workspace.chat(CHAT).unwrap().unwrap();
    assert_eq!(source.space_id, None);
    let scratch = tmp
        .path()
        .join("data")
        .join("scratch")
        .join(source.cwd.as_deref().unwrap().rsplit('/').next().unwrap());
    let hand_off = |chat: &'static str, extra: serde_json::Value| {
        let mut params = serde_json::json!({
            "sourceChatId": CHAT, "chatId": chat, "targetSpaceId": "space-api",
        });
        params
            .as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        rig.client
            .call(zeron_rpc::methods::CONTINUE_IN_NEW_THREAD, params)
    };

    let linked: zeron_proto::Chat =
        serde_json::from_value(hand_off(LINKED, serde_json::json!({})).await.unwrap()).unwrap();
    assert_eq!(linked.space_id.as_deref(), Some("space-api"));
    assert_eq!(linked.cwd.as_deref(), Some(project_path.as_str()));
    assert_eq!(linked.branch, None);
    assert_eq!(linked.checkout_id, None);
    let (to, pointer, route) = handoff_route(&entries(&rig.core, LINKED)[0]);
    assert_eq!(to, HarnessId::Mock, "the source's agent by default");
    assert_eq!(route.kind, zeron_doc::HandoffKind::Pointer);
    assert_eq!(
        pointer,
        format!(
            "Handed off from chat \"Auth work\" (id {CHAT}). Read it with the zeron read_thread tool before acting. Its files are in {}.",
            scratch.display()
        )
    );
    let source_doc = entries(&rig.core, CHAT);
    let note = source_doc.last().unwrap();
    assert_eq!(note.role, MessageRole::System);
    assert_eq!(entry_text(note), "Handed off to api › Auth work (cont.)");
    assert!(
        !note
            .parts
            .iter()
            .any(|part| matches!(part, MessagePart::Handoff { .. }))
    );

    rig.turn(LINKED, "go on", "l1").await;
    let first = rig.old.runs.lock().unwrap().last().cloned().unwrap();
    assert_eq!(first.resume, None);
    assert_eq!(
        first.prompt,
        format!("{pointer}\n\n<latest_user_message>\ngo on\n</latest_user_message>")
    );

    hand_off(
        COPIED,
        serde_json::json!({ "harness": "pi", "disabledTools": ["mcp__zeron__read_thread"] }),
    )
    .await
    .unwrap();
    let (_, packet, route) = handoff_route(&entries(&rig.core, COPIED)[0]);
    assert_eq!(route.kind, zeron_doc::HandoffKind::Text);
    assert!(
        packet.contains("User: first\n\nAssistant: Old answer 1"),
        "{packet}"
    );
    assert!(!packet.contains("Handed off"), "{packet}");

    rig.core
        .workspace
        .create_space("space-far", "dev-far", "/tmp", None, false)
        .unwrap();
    let err = rig
        .client
        .call(
            zeron_rpc::methods::CONTINUE_IN_NEW_THREAD,
            serde_json::json!({
                "sourceChatId": CHAT, "chatId": "chat-far", "targetSpaceId": "space-far",
            }),
        )
        .await
        .unwrap_err();
    assert!(err.to_string().contains("same device"), "{err}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_thread_continued_twice_still_carries_the_first_conversation() {
    const B: &str = "chat-b";
    const C: &str = "chat-c";
    let tmp = tempfile::tempdir().unwrap();
    let rig = rig(tmp.path()).await;
    rig.turn(CHAT, "first", "m1").await;
    let continue_to = |source: &'static str, chat: &'static str, harness: &'static str| {
        rig.client.call(
            zeron_rpc::methods::CONTINUE_IN_NEW_THREAD,
            serde_json::json!({ "sourceChatId": source, "chatId": chat, "harness": harness }),
        )
    };
    continue_to(CHAT, B, "pi").await.expect("A to B");
    rig.turn(B, "hello", "b1").await;
    continue_to(B, C, "mock").await.expect("B to C");

    let doc = entries(&rig.core, C);
    assert_eq!(doc.len(), 1);
    let (_, packet, route) = handoff_route(&doc[0]);
    assert_eq!(route.source_chat_id.as_deref(), Some(B));
    assert_eq!(packet.matches("<provider_switch>").count(), 1, "{packet}");
    assert!(
        packet.contains(
            "<previous_conversation>\nUser: first\n\nAssistant: Old answer 1\n\nUser: hello\n\nAssistant: New answer 1\n</previous_conversation>"
        ),
        "{packet}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn moving_to_another_instance_of_the_same_driver_closes_the_session() {
    let tmp = tempfile::tempdir().unwrap();
    let rig = rig(tmp.path()).await;
    rig.turn(CHAT, "first", "m1").await;
    assert_eq!(rig.session(CHAT).as_deref(), Some("Old-sess"));
    rig.client
        .call(
            zeron_rpc::methods::MUTATE,
            serde_json::json!({
                "op": "setChatConfig",
                "chatId": CHAT,
                "config": {"harness": "mock", "sandbox": "workspace-write", "instanceId": "mock_work"},
            }),
        )
        .await
        .expect("setChatConfig");
    assert_eq!(rig.session(CHAT).as_deref(), Some(""));
}

#[tokio::test]
async fn an_instance_whose_home_changes_or_goes_away_closes_its_chats_sessions() {
    let tmp = tempfile::tempdir().unwrap();
    let rig = rig(tmp.path()).await;
    rig.turn(CHAT, "first", "m1").await;
    let upsert = |label: &str, home: serde_json::Value| {
        let params = serde_json::json!({"instance": {
            "id": "codex_work",
            "driver": "codex",
            "label": label,
            "config": {"kind": "native", "homeDir": home, "launchArgs": [], "env": {}},
        }});
        let client = &rig.client;
        async move {
            client
                .call(zeron_rpc::methods::UPSERT_PROVIDER_INSTANCE, params)
                .await
                .expect("upsert instance")
        }
    };
    upsert("Work", serde_json::json!({"kind": "managed"})).await;
    let mut config = rig.core.workspace.chat_config(CHAT).unwrap();
    config.instance_id = Some("codex_work".into());
    rig.core.workspace.set_chat_config(CHAT, &config).unwrap();
    assert_eq!(rig.session(CHAT).as_deref(), Some("Old-sess"));

    upsert("Renamed", serde_json::json!({"kind": "managed"})).await;
    assert_eq!(rig.session(CHAT).as_deref(), Some("Old-sess"));

    let custom = tmp.path().join("codex-home").display().to_string();
    upsert(
        "Renamed",
        serde_json::json!({"kind": "custom", "path": custom}),
    )
    .await;
    assert_eq!(rig.session(CHAT).as_deref(), Some(""));

    rig.core
        .workspace
        .set_chat_harness_session(CHAT, "Old-sess", &rig.cwd);
    rig.client
        .call(
            zeron_rpc::methods::DELETE_PROVIDER_INSTANCE,
            serde_json::json!({"instanceId": "codex_work"}),
        )
        .await
        .expect("delete instance");
    assert_eq!(rig.session(CHAT).as_deref(), Some(""));
}
