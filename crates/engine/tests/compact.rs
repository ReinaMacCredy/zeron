//! Manual context compaction: the engine runs the harness's own compaction,
//! appends exactly one `system` boundary entry, resets the context meter, and
//! refuses a second compaction or one on a busy chat.

use std::sync::atomic::{AtomicBool, Ordering};
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
    AgentEvent, Compaction, DoneStatus, HarnessId, Model, ReasoningLevel, RunRequest, SandboxLevel,
    SteeringMode,
};

const CHAT: &str = "chat-compact";
const WINDOW: u64 = 200_000;

/// A `/compact`-capable harness whose compaction run the test can gate and
/// whose every request is recorded.
struct TestHarness {
    requests: Arc<Mutex<Vec<RunRequest>>>,
    /// False makes the compaction run end with a result and no `Compacted`.
    compacts: AtomicBool,
    /// True makes a completed turn PARK its run in the chat's slot, the way
    /// every real steerable harness does.
    steerable: AtomicBool,
    /// Held by the next run until the test releases it.
    gate: Mutex<Option<tokio::sync::oneshot::Receiver<()>>>,
}

impl TestHarness {
    fn new(requests: Arc<Mutex<Vec<RunRequest>>>, steerable: bool) -> Self {
        Self {
            requests,
            compacts: AtomicBool::new(true),
            steerable: AtomicBool::new(steerable),
            gate: Mutex::new(None),
        }
    }
}

#[async_trait]
impl Harness for TestHarness {
    fn id(&self) -> HarnessId {
        HarnessId::Mock
    }
    fn display_name(&self) -> &str {
        "Compactor"
    }
    fn supports_steering(&self) -> bool {
        self.steerable.load(Ordering::SeqCst)
    }
    fn steering_mode(&self) -> SteeringMode {
        SteeringMode::TurnBoundary
    }
    fn reasoning_levels(&self) -> &[ReasoningLevel] {
        &[ReasoningLevel::Medium]
    }
    fn compaction(&self) -> Compaction {
        Compaction::SlashText("/compact".into())
    }
    async fn models(&self) -> Result<Vec<Model>, HarnessError> {
        Ok(vec![])
    }
    async fn run(
        &self,
        request: RunRequest,
        controls: RunControls,
    ) -> Result<BoxStream<'static, Result<AgentEvent, HarnessError>>, HarnessError> {
        let interrupt = controls.interrupt.clone();
        let compacting = request.prompt.starts_with("/compact");
        self.requests.lock().unwrap().push(request.clone());
        let gate = self.gate.lock().unwrap().take();
        let mut events: Vec<Result<AgentEvent, HarnessError>> =
            vec![Ok(AgentEvent::SessionStarted {
                harness: HarnessId::Mock,
                model: "mock-1".into(),
                tools: vec![],
                cwd: request.cwd.clone(),
                session_id: "sess-compact".into(),
                assistant_message_id: "a-1".into(),
            })];
        if compacting {
            if self.compacts.load(Ordering::SeqCst) {
                events.push(Ok(AgentEvent::Compacted {
                    pre_tokens: Some(22_762),
                    post_tokens: Some(2_326),
                }));
                events.push(Ok(AgentEvent::Done {
                    status: DoneStatus::Completed,
                    result: Some(String::new()),
                    error: None,
                    session_id: Some("sess-compact".into()),
                }));
            } else {
                events.push(Ok(AgentEvent::Done {
                    status: DoneStatus::Completed,
                    result: Some("Not enough messages to compact.".into()),
                    error: None,
                    session_id: Some("sess-compact".into()),
                }));
            }
        } else {
            events.push(Ok(AgentEvent::ContextUsage {
                tokens: Some(22_762),
                window: Some(WINDOW),
            }));
            events.push(Ok(AgentEvent::TextDelta {
                text: format!("ack: {}", request.prompt),
            }));
            events.push(Ok(AgentEvent::Done {
                status: DoneStatus::Completed,
                result: None,
                error: None,
                session_id: Some("sess-compact".into()),
            }));
        }
        // The child stays warm after `Done` and ends only on its interrupt
        // token — the persistent-session shape a parked run depends on.
        Ok(futures::stream::once(async move {
            if let Some(gate) = gate {
                let _ = gate.await;
            }
            futures::stream::iter(events)
        })
        .flatten()
        .chain(futures::stream::once(async move {
            interrupt.cancelled().await;
            Ok(AgentEvent::Done {
                status: DoneStatus::Interrupted,
                result: None,
                error: None,
                session_id: None,
            })
        }))
        .boxed())
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

fn entries(core: &EngineCore) -> Vec<SessionMessageEntry> {
    core.doc_host
        .open(CHAT)
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

fn run_payload(message_id: &str, cwd: &str) -> SessionCommandPayload {
    SessionCommandPayload::Run {
        request: RunRequest {
            prompt: "hello".into(),
            harness: None,
            model: None,
            reasoning: None,
            model_options: Default::default(),
            cwd: cwd.into(),
            sandbox: SandboxLevel::WorkspaceWrite,
            auto_approve: true,
            attachments: Vec::new(),
            resume: None,
            worktree: None,
            branch: None,
        },
        message_id: message_id.into(),
    }
}

/// Engine + a pre-titled chat (so the auto-titler never spawns a run) whose
/// first turn has already completed.
async fn engine_with_one_turn(
    dir: &std::path::Path,
) -> (EngineCore, Arc<TestHarness>, Arc<Mutex<Vec<RunRequest>>>) {
    engine_with_one_turn_on(dir, false).await
}

async fn engine_with_one_turn_on(
    dir: &std::path::Path,
    steerable: bool,
) -> (EngineCore, Arc<TestHarness>, Arc<Mutex<Vec<RunRequest>>>) {
    let requests: Arc<Mutex<Vec<RunRequest>>> = Arc::new(Mutex::new(Vec::new()));
    let harness = Arc::new(TestHarness::new(requests.clone(), steerable));
    let registry = HarnessRegistry::new();
    registry.register(harness.clone());
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
        .rename_chat(CHAT, "Pre-titled")
        .expect("rename chat");
    core.doc_host
        .queue_command(CHAT, run_payload("msg-1", &dir.to_string_lossy()))
        .expect("queue run");
    wait_for(|| entries(&core).len() == 2, "first turn").await;
    wait_for(|| !core.sessions.turn_in_flight(CHAT), "first turn settles").await;
    (core, harness, requests)
}

#[tokio::test(flavor = "multi_thread")]
async fn compaction_appends_one_system_entry_and_resets_the_meter() {
    let tmp = tempfile::tempdir().unwrap();
    let (core, _harness, requests) = engine_with_one_turn(tmp.path()).await;

    let before = entries(&core);
    let usage_before = core.doc_host.open(CHAT).unwrap().doc().context_usage();
    assert_eq!(usage_before.and_then(|u| u.window), Some(WINDOW));

    let outcome = core.sessions.compact(CHAT, None).await.expect("compaction");
    assert!(outcome.compacted);
    assert_eq!(outcome.pre_tokens, Some(22_762));
    assert_eq!(outcome.post_tokens, Some(2_326));
    assert_eq!(outcome.message, None);

    // The compaction run is the chat's own session resumed, prompted with the
    // command alone — no user entry, no attachments.
    let compaction_request = requests.lock().unwrap().last().cloned().expect("recorded");
    assert_eq!(compaction_request.prompt, "/compact");
    assert_eq!(compaction_request.resume.as_deref(), Some("sess-compact"));

    let after = entries(&core);
    assert_eq!(after.len(), before.len() + 1, "exactly one new entry");
    assert_eq!(&after[..before.len()], &before[..], "history untouched");
    let boundary = after.last().expect("boundary entry");
    assert_eq!(boundary.role, MessageRole::System);
    assert_eq!(
        entry_text(boundary),
        "Context compacted · 22.8k → 2.3k tokens"
    );

    let usage = core
        .doc_host
        .open(CHAT)
        .unwrap()
        .doc()
        .context_usage()
        .expect("meter reset, not blanked");
    assert_eq!(usage.tokens, Some(2_326));
    assert_eq!(usage.window, Some(WINDOW), "the window survives the reset");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_second_compaction_is_refused_while_one_is_pending() {
    let tmp = tempfile::tempdir().unwrap();
    let (core, harness, _requests) = engine_with_one_turn(tmp.path()).await;

    let (release, gate) = tokio::sync::oneshot::channel();
    *harness.gate.lock().unwrap() = Some(gate);
    let first = {
        let sessions = core.sessions.clone();
        tokio::spawn(async move { sessions.compact(CHAT, None).await })
    };
    wait_for(|| core.sessions.turn_in_flight(CHAT), "compaction starts").await;

    let err = core
        .sessions
        .compact(CHAT, None)
        .await
        .expect_err("second compaction refused");
    assert_eq!(
        err.to_string(),
        "Context compaction is already in progress."
    );

    let _ = release.send(());
    assert!(first.await.unwrap().expect("first compaction").compacted);
    assert_eq!(entries(&core).len(), 3, "only the one boundary entry");
}

#[tokio::test(flavor = "multi_thread")]
async fn compaction_is_refused_while_a_run_is_live() {
    let tmp = tempfile::tempdir().unwrap();
    let (core, harness, _requests) = engine_with_one_turn(tmp.path()).await;

    let (release, gate) = tokio::sync::oneshot::channel();
    *harness.gate.lock().unwrap() = Some(gate);
    core.doc_host
        .queue_command(CHAT, run_payload("msg-2", &tmp.path().to_string_lossy()))
        .expect("queue second run");
    wait_for(|| core.sessions.turn_in_flight(CHAT), "second turn starts").await;

    let err = core
        .sessions
        .compact(CHAT, None)
        .await
        .expect_err("compaction refused on a busy chat");
    assert_eq!(err.to_string(), "Chat is busy");

    let _ = release.send(());
    wait_for(
        || !core.sessions.turn_in_flight(CHAT),
        "second turn settles",
    )
    .await;
    assert!(
        entries(&core)
            .iter()
            .all(|entry| entry.role != MessageRole::System),
        "a refused compaction writes nothing"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_compaction_run_without_a_boundary_writes_nothing() {
    let tmp = tempfile::tempdir().unwrap();
    let (core, harness, _requests) = engine_with_one_turn(tmp.path()).await;
    harness.compacts.store(false, Ordering::SeqCst);

    let before = entries(&core);
    let outcome = core
        .sessions
        .compact(CHAT, Some("  auth  ".into()))
        .await
        .expect("compaction run completes");
    assert!(!outcome.compacted);
    assert_eq!(
        outcome.message.as_deref(),
        Some("Not enough messages to compact.")
    );
    assert_eq!(entries(&core), before, "the doc is untouched");
    let usage = core.doc_host.open(CHAT).unwrap().doc().context_usage();
    assert_eq!(usage.and_then(|u| u.tokens), Some(22_762), "meter intact");
}

#[tokio::test(flavor = "multi_thread")]
async fn focus_rides_the_command_text() {
    let tmp = tempfile::tempdir().unwrap();
    let (core, _harness, requests) = engine_with_one_turn(tmp.path()).await;
    core.sessions
        .compact(CHAT, Some(" focus on auth ".into()))
        .await
        .expect("compaction");
    assert_eq!(
        requests.lock().unwrap().last().unwrap().prompt,
        "/compact focus on auth"
    );
}

/// The production shape: a completed turn on a steerable harness parks its
/// warm child in the chat's run slot for 30 minutes. Because this harness's
/// stream only ends when its interrupt token fires, nothing but the
/// compaction's own retirement of that parked run can free the slot.
#[tokio::test(flavor = "multi_thread")]
async fn compaction_retires_a_parked_warm_child_and_resumes_its_session() {
    let tmp = tempfile::tempdir().unwrap();
    let (core, _harness, requests) = engine_with_one_turn_on(tmp.path(), true).await;
    assert_eq!(
        requests.lock().unwrap().len(),
        1,
        "turn one parked, not ended"
    );

    let outcome = core.sessions.compact(CHAT, None).await.expect("compaction");
    assert!(outcome.compacted);

    let compaction_request = requests.lock().unwrap().last().cloned().expect("recorded");
    assert_eq!(compaction_request.prompt, "/compact");
    assert_eq!(compaction_request.resume.as_deref(), Some("sess-compact"));

    // Retiring the parked child is a clean end: nothing aborted, nothing errored.
    let after = entries(&core);
    assert!(
        after
            .iter()
            .all(|entry| entry.status != Some(MessageStatus::Aborted)),
        "{after:#?}"
    );
    assert!(
        after
            .iter()
            .flat_map(|entry| &entry.parts)
            .all(|part| !matches!(part, MessagePart::Error { .. })),
        "{after:#?}"
    );
    assert_eq!(
        after
            .iter()
            .filter(|entry| entry.role == MessageRole::System)
            .count(),
        1
    );
}
