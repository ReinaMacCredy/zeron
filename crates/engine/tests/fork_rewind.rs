//! Fork and rewind: one engine primitive (a pending branch applied on the
//! chat's next run) behind two products. The harness here records what each
//! run was asked to resume and where to cut it, and emits the anchors a real
//! driver would.

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
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
    AgentEvent, AnchorRole, Chat, DoneStatus, HarnessId, Model, ReasoningLevel, RunRequest,
    SandboxLevel, SessionBranch, SteeringMode,
};

const SOURCE: &str = "chat-source";
const FORK: &str = "chat-fork";

#[derive(Debug, Clone, PartialEq)]
struct Seen {
    resume: Option<String>,
    branch: Option<SessionBranch>,
}

struct TestHarness {
    runs: Arc<Mutex<Vec<Seen>>>,
    turns: AtomicUsize,
    branching: bool,
    /// When set, the next run reports no anchors (a driver from before
    /// anchors shipped).
    silent: AtomicBool,
}

impl TestHarness {
    fn new(branching: bool) -> Self {
        Self {
            runs: Arc::new(Mutex::new(Vec::new())),
            turns: AtomicUsize::new(0),
            branching,
            silent: AtomicBool::new(false),
        }
    }
}

#[async_trait]
impl Harness for TestHarness {
    fn id(&self) -> HarnessId {
        HarnessId::Mock
    }
    fn display_name(&self) -> &str {
        "Brancher"
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
        self.branching
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
            resume: request.resume.clone(),
            branch: request.branch.clone(),
        });
        let n = self.turns.fetch_add(1, Ordering::SeqCst) + 1;
        let session_id = match (&request.resume, &request.branch) {
            (Some(_), Some(_)) => "sess-forked".to_string(),
            (Some(resume), None) => resume.clone(),
            (None, _) => "sess-1".to_string(),
        };
        let silent = self.silent.swap(false, Ordering::SeqCst);
        let mut events = vec![AgentEvent::SessionStarted {
            harness: HarnessId::Mock,
            model: "mock-1".into(),
            tools: vec![],
            cwd: request.cwd.clone(),
            session_id: session_id.clone(),
            assistant_message_id: format!("a-{n}"),
        }];
        if !silent {
            events.push(AgentEvent::Anchor {
                role: AnchorRole::User,
                id: format!("uuid-user-{n}"),
                entry_id: None,
            });
        }
        events.push(AgentEvent::ContextUsage {
            tokens: Some(1_000 * n as u64),
            window: Some(200_000),
        });
        events.push(AgentEvent::TextDelta {
            text: format!("answer {n}"),
        });
        if !silent {
            events.push(AgentEvent::Anchor {
                role: AnchorRole::Assistant,
                id: format!("uuid-reply-{n}"),
                entry_id: None,
            });
        }
        events.push(AgentEvent::Done {
            status: DoneStatus::Completed,
            result: None,
            error: None,
            session_id: Some(session_id),
        });
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
        // The row write behind SessionStarted is what a later dispatch reads.
        wait_for(
            || {
                self.core
                    .workspace
                    .chat(chat_id)
                    .ok()
                    .flatten()
                    .and_then(|c| c.harness_session_id)
                    .is_some()
            },
            "session row",
        )
        .await;
    }

    fn chat(&self, chat_id: &str) -> Chat {
        self.core
            .workspace
            .chat(chat_id)
            .expect("read row")
            .expect("row exists")
    }

    fn seen(&self) -> Vec<Seen> {
        self.harness.runs.lock().unwrap().clone()
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

async fn rig(dir: &std::path::Path, branching: bool) -> Rig {
    let harness = Arc::new(TestHarness::new(branching));
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
async fn fork_copies_through_the_reply_and_branches_the_next_run() {
    let tmp = tempfile::tempdir().unwrap();
    let rig = rig(tmp.path(), true).await;
    rig.turn(SOURCE, "first", "m1").await;
    rig.turn(SOURCE, "second", "m2").await;
    let source_before = entries(&rig.core, SOURCE);
    let row_before = rig.chat(SOURCE);

    let forked = rig
        .core
        .sessions
        .fork_chat(SOURCE, &rig.reply_id(SOURCE, 1), FORK)
        .expect("fork");
    assert_eq!(forked.title.as_deref(), Some("Fork: Auth work"));
    assert_eq!(forked.sidechat_source_chat_id, None);
    assert_eq!(forked.space_id, row_before.space_id);
    let fork_entries = entries(&rig.core, FORK);
    assert_eq!(
        fork_entries.iter().map(entry_text).collect::<Vec<_>>(),
        vec!["first".to_string(), "answer 1".to_string()]
    );
    assert_eq!(fork_entries[0].id, source_before[0].id);
    assert_eq!(
        rig.chat(FORK).harness_session_id.as_deref(),
        Some("sess-1"),
        "the fork resumes the source's session until its first run branches it"
    );
    // The source is untouched.
    assert_eq!(entries(&rig.core, SOURCE), source_before);
    assert_eq!(rig.chat(SOURCE), row_before);

    rig.turn(FORK, "third, on the fork", "m3").await;
    let seen = rig.seen();
    assert_eq!(
        seen[2],
        Seen {
            resume: Some("sess-1".into()),
            branch: Some(SessionBranch {
                keep: Some("uuid-reply-1".into()),
                drop: Some("uuid-user-2".into()),
            }),
        }
    );
    assert_eq!(
        rig.chat(FORK).harness_session_id.as_deref(),
        Some("sess-forked")
    );
    assert_eq!(
        rig.chat(SOURCE).harness_session_id.as_deref(),
        Some("sess-1"),
        "branching the fork never moves the source's pointer"
    );

    // The branch is spent: the fork's later turns resume its own session.
    rig.turn(FORK, "fourth", "m4").await;
    assert_eq!(
        rig.seen()[3],
        Seen {
            resume: Some("sess-forked".into()),
            branch: None,
        }
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn fork_survives_an_engine_restart_before_its_first_run() {
    let tmp = tempfile::tempdir().unwrap();
    {
        let rig = rig(tmp.path(), true).await;
        rig.turn(SOURCE, "first", "m1").await;
        rig.core
            .sessions
            .fork_chat(SOURCE, &rig.reply_id(SOURCE, 1), FORK)
            .expect("fork");
        rig.core.shutdown().await;
    }
    let rig = rig(tmp.path(), true).await;
    rig.turn(FORK, "after restart", "m2").await;
    assert_eq!(
        rig.seen()[0],
        Seen {
            resume: Some("sess-1".into()),
            branch: Some(SessionBranch {
                keep: Some("uuid-reply-1".into()),
                drop: None,
            }),
        }
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn rewind_truncates_the_doc_and_branches_after_the_previous_reply() {
    let tmp = tempfile::tempdir().unwrap();
    let rig = rig(tmp.path(), true).await;
    rig.turn(SOURCE, "first", "m1").await;
    rig.turn(SOURCE, "second", "m2").await;
    rig.turn(SOURCE, "third", "m3").await;
    let doc = rig.core.doc_host.open(SOURCE).unwrap();
    assert!(doc.doc().context_usage().is_some());

    let outcome = rig
        .core
        .sessions
        .rewind_chat(SOURCE, &rig.prompt_id(SOURCE, 2))
        .expect("rewind");
    assert_eq!(outcome.prompt, "second");
    assert_eq!(outcome.removed, 4);
    assert_eq!(
        entries(&rig.core, SOURCE)
            .iter()
            .map(entry_text)
            .collect::<Vec<_>>(),
        vec!["first".to_string(), "answer 1".to_string()]
    );
    assert!(doc.doc().context_usage().is_none());

    rig.turn(SOURCE, "second, take two", "m4").await;
    assert_eq!(
        rig.seen()[3],
        Seen {
            resume: Some("sess-1".into()),
            branch: Some(SessionBranch {
                keep: Some("uuid-reply-1".into()),
                drop: Some("uuid-user-2".into()),
            }),
        }
    );
    assert_eq!(
        rig.chat(SOURCE).harness_session_id.as_deref(),
        Some("sess-forked")
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn rewind_before_the_first_prompt_starts_fresh() {
    let tmp = tempfile::tempdir().unwrap();
    let rig = rig(tmp.path(), true).await;
    rig.turn(SOURCE, "first", "m1").await;

    let outcome = rig
        .core
        .sessions
        .rewind_chat(SOURCE, &rig.prompt_id(SOURCE, 0))
        .expect("rewind");
    assert_eq!(outcome.prompt, "first");
    assert!(entries(&rig.core, SOURCE).is_empty());

    rig.turn(SOURCE, "first, again", "m2").await;
    assert_eq!(
        rig.seen()[1],
        Seen {
            resume: None,
            branch: None,
        }
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn refusals_leave_the_doc_alone() {
    let tmp = tempfile::tempdir().unwrap();
    let rig = rig(tmp.path(), true).await;
    rig.harness.silent.store(true, Ordering::SeqCst);
    rig.turn(SOURCE, "legacy", "m1").await;
    rig.turn(SOURCE, "tracked", "m2").await;
    let before = entries(&rig.core, SOURCE);

    let err = rig
        .core
        .sessions
        .fork_chat(SOURCE, &rig.reply_id(SOURCE, 1), FORK)
        .expect_err("no anchor for the legacy reply");
    assert!(err.to_string().contains("No checkpoint"), "{err}");
    assert!(rig.core.workspace.chat(FORK).unwrap().is_none());

    let err = rig
        .core
        .sessions
        .rewind_chat(SOURCE, &rig.prompt_id(SOURCE, 2))
        .expect_err("no anchor before the tracked prompt");
    assert!(err.to_string().contains("No checkpoint"), "{err}");
    assert_eq!(entries(&rig.core, SOURCE), before);

    let err = rig
        .core
        .sessions
        .rewind_chat(SOURCE, &rig.reply_id(SOURCE, 3))
        .expect_err("a reply is not a rewind point");
    assert!(err.to_string().contains("no longer"), "{err}");
    assert_eq!(entries(&rig.core, SOURCE), before);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_harness_without_branching_refuses_both() {
    let tmp = tempfile::tempdir().unwrap();
    let rig = rig(tmp.path(), false).await;
    rig.turn(SOURCE, "first", "m1").await;

    let err = rig
        .core
        .sessions
        .fork_chat(SOURCE, &rig.reply_id(SOURCE, 1), FORK)
        .expect_err("fork refused");
    assert!(err.to_string().contains("cannot fork or rewind"), "{err}");
    let err = rig
        .core
        .sessions
        .rewind_chat(SOURCE, &rig.prompt_id(SOURCE, 0))
        .expect_err("rewind refused");
    assert!(err.to_string().contains("cannot fork or rewind"), "{err}");
    assert_eq!(entries(&rig.core, SOURCE).len(), 2);
}
