//! Manual context compaction: the engine runs the harness's own compaction,
//! appends exactly one `system` boundary entry, resets the context meter, and
//! refuses a second compaction or one on a busy chat.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use futures::StreamExt;
use futures::stream::BoxStream;

use zeron_doc::{
    ClassifierRecord, ClassifierVerdict, DecidedBy, MessagePart, MessageRole, MessageStatus,
    SessionCommandPayload, SessionMessageEntry,
};
use zeron_engine::registry::{CompactionMode, CompactionSettings};
use zeron_engine::{EngineCore, HarnessRegistry};
use zeron_harness::{Harness, HarnessError, RunControls};
use zeron_proto::{
    AgentEvent, Compaction, DoneStatus, HarnessId, Model, ReasoningLevel, RunRequest, SandboxLevel,
    SteeringMode, Task, TaskPatch, TaskStep,
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
    /// False reports `Compaction::None`, like Pi or an ACP agent without `/compact`.
    can_compact: AtomicBool,
    in_place: AtomicBool,
    mailbox: Arc<Mutex<Vec<String>>>,
    /// Held by the next run until the test releases it.
    gate: Mutex<Option<tokio::sync::oneshot::Receiver<()>>>,
    /// The boundary classifier's answer; None fails the one-shot.
    verdict: Mutex<Option<String>>,
    replies: Mutex<VecDeque<String>>,
    one_shots: Mutex<Vec<String>>,
    first_reading: AtomicU64,
    tokens: AtomicU64,
    background: Mutex<Vec<zeron_proto::BackgroundTask>>,
    /// Background work counted on the runtime only, the way Codex counts its open commands.
    shells: std::sync::atomic::AtomicUsize,
}

impl TestHarness {
    fn new(requests: Arc<Mutex<Vec<RunRequest>>>, steerable: bool) -> Self {
        Self {
            requests,
            compacts: AtomicBool::new(true),
            steerable: AtomicBool::new(steerable),
            can_compact: AtomicBool::new(true),
            in_place: AtomicBool::new(false),
            mailbox: Arc::new(Mutex::new(Vec::new())),
            gate: Mutex::new(None),
            verdict: Mutex::new(None),
            replies: Mutex::new(VecDeque::new()),
            one_shots: Mutex::new(Vec::new()),
            first_reading: AtomicU64::new(1_000),
            tokens: AtomicU64::new(22_762),
            background: Mutex::new(Vec::new()),
            shells: std::sync::atomic::AtomicUsize::new(0),
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
        if self.can_compact.load(Ordering::SeqCst) {
            Compaction::SlashText("/compact".into())
        } else {
            Compaction::None
        }
    }
    fn compacts_in_place(&self) -> bool {
        self.in_place.load(Ordering::SeqCst)
    }
    async fn models(&self) -> Result<Vec<Model>, HarnessError> {
        Ok(vec![])
    }
    async fn run_one_shot(
        &self,
        request: RunRequest,
        _controls: RunControls,
        _instructions: &'static str,
    ) -> Result<BoxStream<'static, Result<AgentEvent, HarnessError>>, HarnessError> {
        self.one_shots.lock().unwrap().push(request.prompt);
        let queued = self.replies.lock().unwrap().pop_front();
        let verdict = queued.or_else(|| self.verdict.lock().unwrap().clone());
        let Some(text) = verdict else {
            return Err(HarnessError::Protocol("no verdict".into()));
        };
        Ok(futures::stream::iter([
            Ok(AgentEvent::TextDelta { text }),
            Ok(AgentEvent::Done {
                status: DoneStatus::Completed,
                result: None,
                error: None,
                session_id: None,
            }),
        ])
        .boxed())
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
            controls
                .turn
                .set_background(self.shells.load(Ordering::SeqCst));
            let tasks = self.background.lock().unwrap().clone();
            if !tasks.is_empty() {
                events.push(Ok(AgentEvent::BackgroundTasks { tasks }));
            }
            for tokens in [&self.first_reading, &self.tokens] {
                events.push(Ok(AgentEvent::ContextUsage {
                    tokens: Some(tokens.load(Ordering::SeqCst)),
                    window: Some(WINDOW),
                }));
            }
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
        if self.in_place.load(Ordering::SeqCst) && !compacting {
            return Ok(live_run(events, controls, self.mailbox.clone()));
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

fn live_run(
    events: Vec<Result<AgentEvent, HarnessError>>,
    controls: RunControls,
    mailbox: Arc<Mutex<Vec<String>>>,
) -> BoxStream<'static, Result<AgentEvent, HarnessError>> {
    let (tx, rx) = futures::channel::mpsc::unbounded();
    let RunControls {
        mut steering,
        interrupt,
        turn,
        ..
    } = controls;
    tokio::spawn(async move {
        let send = |events: Vec<AgentEvent>| {
            for event in events {
                let _ = tx.unbounded_send(Ok(event));
            }
        };
        for event in events {
            let _ = tx.unbounded_send(event);
        }
        let steered = || AgentEvent::Steered {
            assistant_message_id: None,
            next_assistant_message_id: None,
        };
        let done = |result: Option<String>| AgentEvent::Done {
            status: DoneStatus::Completed,
            result,
            error: None,
            session_id: Some("sess-compact".into()),
        };
        loop {
            tokio::select! {
                _ = interrupt.cancelled() => {
                    send(vec![AgentEvent::Done {
                        status: DoneStatus::Interrupted,
                        result: None,
                        error: None,
                        session_id: None,
                    }]);
                    return;
                }
                command = turn.compact_requested() => {
                    mailbox.lock().unwrap().push(command);
                    send(vec![
                        steered(),
                        AgentEvent::Compacting { active: true },
                        AgentEvent::Compacted {
                            pre_tokens: Some(22_762),
                            post_tokens: Some(2_326),
                        },
                        done(Some(String::new())),
                    ]);
                }
                Some(steer) = steering.recv() => {
                    mailbox.lock().unwrap().push(steer.prompt.clone());
                    send(vec![
                        steered(),
                        AgentEvent::TextDelta { text: format!("ack: {}", steer.prompt) },
                        done(None),
                    ]);
                }
            }
        }
    });
    rx.boxed()
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
            permission_mode: zeron_proto::PermissionMode::Full,
            auto_approve: true,
            access: zeron_proto::PermissionMode::Full.access(),
            plan: false,
            attachments: Vec::new(),
            resume: None,
            worktree: None,
            branch: None,
            forked_at: None,
            computer_control: false,
            mcp: None,
            disabled_tools: Vec::new(),
            env: Default::default(),
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
    let core = engine_with_one_turn_by(dir, harness.clone()).await;
    (core, harness, requests)
}

async fn engine_with_one_turn_by(dir: &std::path::Path, harness: Arc<TestHarness>) -> EngineCore {
    let registry = HarnessRegistry::new();
    registry.register(harness.clone());
    let core = EngineCore::assemble(&dir.join("data"), Arc::new(registry), HarnessId::Mock, None)
        .expect("engine core assembles");
    let client = zeron_rpc::memory_client(core.rpc_service());
    client
        .call(
            zeron_rpc::methods::MUTATE,
            serde_json::json!({
                "op": "createChat",
                "chatId": CHAT,
                "deviceId": core.device_id,
                "cwd": dir.to_string_lossy(),
            }),
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
    core
}

#[tokio::test(flavor = "multi_thread")]
async fn set_chat_config_with_a_note_appends_one_system_entry() {
    let tmp = tempfile::tempdir().unwrap();
    let (core, _harness, _requests) = engine_with_one_turn(tmp.path()).await;
    let client = zeron_rpc::memory_client(core.rpc_service());
    let config = serde_json::json!({
        "harness": "mock",
        "model": "mock-b",
        "reasoning": null,
        "modelOptions": {},
        "sandbox": "workspace-write",
    });

    let before = entries(&core);
    client
        .call(
            zeron_rpc::methods::MUTATE,
            serde_json::json!({
                "op": "setChatConfig",
                "chatId": CHAT,
                "config": config,
                "note": { "text": "Model changed from Mock A to Mock B.", "hint": "Re-read next turn." },
            }),
        )
        .await
        .expect("setChatConfig with note");
    let after = entries(&core);
    assert_eq!(after.len(), before.len() + 1, "exactly one new entry");
    let note = after.last().expect("note entry");
    assert_eq!(note.role, MessageRole::System);
    let texts: Vec<&str> = note
        .parts
        .iter()
        .filter_map(|part| match part {
            MessagePart::Text { text, .. } => Some(text.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(
        texts,
        ["Model changed from Mock A to Mock B.", "Re-read next turn."]
    );
    assert_eq!(
        core.workspace.chat_config(CHAT).and_then(|c| c.model),
        Some("mock-b".into()),
        "the config landed with the note"
    );

    // Without a note the mutation stays a pure row write.
    client
        .call(
            zeron_rpc::methods::MUTATE,
            serde_json::json!({ "op": "setChatConfig", "chatId": CHAT, "config": config }),
        )
        .await
        .expect("setChatConfig without note");
    assert_eq!(entries(&core).len(), after.len());
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
async fn idle_compaction_says_why_it_ran() {
    let tmp = tempfile::tempdir().unwrap();
    let (core, _harness, _requests) = engine_with_one_turn(tmp.path()).await;

    assert!(core.sessions.compact_idle(CHAT).await.unwrap().compacted);
    assert_eq!(
        entry_text(entries(&core).last().expect("boundary entry")),
        "Compacted while idle, before the prompt cache expired · 22.8k → 2.3k tokens"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn an_agent_compaction_waits_for_its_turn_then_continues() {
    let tmp = tempfile::tempdir().unwrap();
    let (core, harness, requests) = engine_with_one_turn(tmp.path()).await;
    // Below the 2.3k the compaction leaves: the follow-up must still go out bare.
    threshold_mode(&core, 2_000);
    let (release, gate) = tokio::sync::oneshot::channel();
    *harness.gate.lock().unwrap() = Some(gate);
    core.doc_host
        .queue_command(CHAT, run_payload("msg-2", &tmp.path().to_string_lossy()))
        .expect("queue second run");
    wait_for(|| core.sessions.turn_in_flight(CHAT), "second turn starts").await;
    wait_for(|| requests.lock().unwrap().len() == 2, "its prompt is built").await;

    zeron_rpc::memory_client(core.rpc_service())
        .call(
            zeron_rpc::methods::REQUEST_COMPACT,
            serde_json::json!({ "chatId": CHAT, "focus": "keep the plan", "continueWith": "go on" }),
        )
        .await
        .expect("request compact");
    let plan = || core.sessions.session_status(CHAT).and_then(|s| s.compact_plan);
    assert_eq!(plan(), Some(zeron_proto::CompactPlan::Agent));
    let prompts = || -> Vec<String> {
        requests.lock().unwrap().iter().map(|r| r.prompt.clone()).collect()
    };
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert!(!prompts().iter().any(|p| p.starts_with("/compact")), "never under the asking turn");

    let _ = release.send(());
    wait_for(|| prompts().iter().any(|p| p == "go on"), "the agent's follow-up").await;
    assert_eq!(
        prompts()[prompts().len() - 2..],
        ["/compact keep the plan".to_string(), "go on".to_string()]
    );
    let entries = entries(&core);
    let boundary = entries
        .iter()
        .find(|e| e.role == MessageRole::System && verdict_of(e).is_none())
        .expect("boundary entry");
    assert_eq!(entry_text(boundary), "Agent compacted · 22.8k → 2.3k tokens");
    let follow_up = entries
        .iter()
        .find(|e| e.role == MessageRole::User && entry_text(e) == "go on")
        .expect("follow-up entry");
    assert_eq!(follow_up.from_chat_id.as_deref(), Some(CHAT));
    assert_eq!(plan(), None);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_message_before_the_agent_compaction_cancels_it() {
    let tmp = tempfile::tempdir().unwrap();
    let (core, harness, requests) = engine_with_one_turn(tmp.path()).await;
    let (release, gate) = tokio::sync::oneshot::channel();
    *harness.gate.lock().unwrap() = Some(gate);
    core.doc_host
        .queue_command(CHAT, run_payload("msg-2", &tmp.path().to_string_lossy()))
        .expect("queue second run");
    wait_for(|| core.sessions.turn_in_flight(CHAT), "second turn starts").await;
    wait_for(|| requests.lock().unwrap().len() == 2, "its prompt is built").await;

    let client = zeron_rpc::memory_client(core.rpc_service());
    let request = || {
        client.call(
            zeron_rpc::methods::REQUEST_COMPACT,
            serde_json::json!({ "chatId": CHAT, "continueWith": "wait for task X, then merge" }),
        )
    };
    let first = request().await.expect("request compact");
    assert_eq!(first["replaced"], false);
    assert_eq!(request().await.expect("again")["replaced"], true);
    core.doc_host
        .queue_message(CHAT, "task X is done, merge it now", Vec::new())
        .expect("the person writes");
    let _ = release.send(());

    let prompts = || -> Vec<String> {
        requests.lock().unwrap().iter().map(|r| r.prompt.clone()).collect()
    };
    wait_for(|| prompts().len() == 3, "the person's message").await;
    wait_for(|| !core.sessions.turn_in_flight(CHAT), "its turn settles").await;
    assert!(prompts()[2].starts_with(
        "task X is done, merge it now\n\n<system-reminder>\nYour queued compaction was cancelled"
    ));
    assert!(prompts()[2].contains("Its continueWith was: wait for task X, then merge"));
    assert_eq!(
        core.sessions.session_status(CHAT).and_then(|s| s.compact_plan),
        None
    );
    tokio::time::sleep(Duration::from_secs(11)).await;
    assert_eq!(prompts().len(), 3, "no compaction and no stale follow-up: {:?}", prompts());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_message_during_the_agent_compaction_drops_its_continue_with() {
    let tmp = tempfile::tempdir().unwrap();
    let (core, harness, requests) = engine_with_one_turn(tmp.path()).await;
    threshold_mode(&core, 150_000);
    let (release, gate) = tokio::sync::oneshot::channel();
    *harness.gate.lock().unwrap() = Some(gate);
    zeron_rpc::memory_client(core.rpc_service())
        .call(
            zeron_rpc::methods::REQUEST_COMPACT,
            serde_json::json!({ "chatId": CHAT, "continueWith": "go on" }),
        )
        .await
        .expect("request compact");
    let prompts = || -> Vec<String> {
        requests.lock().unwrap().iter().map(|r| r.prompt.clone()).collect()
    };
    wait_for(|| prompts().iter().any(|p| p == "/compact"), "the compaction starts").await;
    core.doc_host
        .queue_message(CHAT, "a new question", Vec::new())
        .expect("the person writes");
    let _ = release.send(());

    wait_for(|| prompts().iter().any(|p| p.starts_with("a new question")), "the person's message").await;
    wait_for(|| !core.sessions.turn_in_flight(CHAT), "its turn settles").await;
    tokio::time::sleep(Duration::from_secs(1)).await;
    assert_eq!(
        prompts()[1..],
        [
            "/compact",
            "a new question\n\n<system-reminder>\nYour compaction's continueWith was not sent because this message reached you first. It was: go on\n</system-reminder>",
        ]
    );
    assert!(
        entries(&core)
            .iter()
            .any(|e| e.role == MessageRole::System && entry_text(e).starts_with("Agent compacted"))
    );
}

fn sleeping_shell() -> Vec<zeron_proto::BackgroundTask> {
    vec![zeron_proto::BackgroundTask {
        id: "b7wb5vp5o".into(),
        kind: zeron_proto::BackgroundTaskKind::Shell,
        label: "sleep 300 && echo done".into(),
        started_at: None,
        output_path: None,
    }]
}

/// A parked run can open its own turn (a finished background shell); interrupting it loses that turn.
/// Its background work survives an in-place compaction, so the compaction does not wait for it.
#[tokio::test(flavor = "multi_thread")]
async fn an_agent_compaction_queues_into_the_parked_run() {
    let tmp = tempfile::tempdir().unwrap();
    let requests: Arc<Mutex<Vec<RunRequest>>> = Arc::new(Mutex::new(Vec::new()));
    let harness = Arc::new(TestHarness::new(requests.clone(), true));
    harness.in_place.store(true, Ordering::SeqCst);
    *harness.background.lock().unwrap() = sleeping_shell();
    let core = engine_with_one_turn_by(tmp.path(), harness.clone()).await;
    threshold_mode(&core, 150_000);

    zeron_rpc::memory_client(core.rpc_service())
        .call(
            zeron_rpc::methods::REQUEST_COMPACT,
            serde_json::json!({ "chatId": CHAT, "focus": "keep the plan", "continueWith": "go on" }),
        )
        .await
        .expect("request compact");
    wait_for(
        || entries(&core).iter().any(|e| e.role == MessageRole::System),
        "the boundary entry",
    )
    .await;
    assert_eq!(requests.lock().unwrap().len(), 1, "no second process");
    let mailbox = || harness.mailbox.lock().unwrap().clone();
    assert_eq!(mailbox()[0], "/compact keep the plan");

    wait_for(
        || {
            entries(&core)
                .iter()
                .any(|e| e.role == MessageRole::Assistant && entry_text(e) == "ack: go on")
        },
        "the follow-up's reply",
    )
    .await;
    assert_eq!(mailbox(), ["/compact keep the plan", "go on"]);
    let entries = entries(&core);
    assert_eq!(entries.len(), 5, "{entries:#?}");
    let boundaries: Vec<String> = entries
        .iter()
        .filter(|e| e.role == MessageRole::System)
        .map(entry_text)
        .collect();
    assert_eq!(boundaries, ["Agent compacted · 22.8k → 2.3k tokens"]);
    assert!(
        entries
            .iter()
            .all(|entry| entry.status != Some(MessageStatus::Aborted)),
        "{entries:#?}"
    );
    assert_eq!(
        core.sessions
            .session_status(CHAT)
            .and_then(|s| s.compact_plan),
        None
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_compaction_that_would_restart_over_background_work_is_refused() {
    let tmp = tempfile::tempdir().unwrap();
    let requests: Arc<Mutex<Vec<RunRequest>>> = Arc::new(Mutex::new(Vec::new()));
    let harness = Arc::new(TestHarness::new(requests.clone(), true));
    harness.shells.store(1, Ordering::SeqCst);
    let core = engine_with_one_turn_by(tmp.path(), harness.clone()).await;
    let refused = zeron_rpc::memory_client(core.rpc_service())
        .call(
            zeron_rpc::methods::REQUEST_COMPACT,
            serde_json::json!({ "chatId": CHAT, "continueWith": "go on" }),
        )
        .await
        .expect_err("refused");
    assert!(refused.to_string().contains("background work"), "{refused}");
    assert_eq!(
        core.sessions
            .session_status(CHAT)
            .and_then(|s| s.compact_plan),
        None
    );
    let refused = core.sessions.compact(CHAT, None).await.expect_err("refused");
    assert!(refused.to_string().contains("background work"), "{refused}");

    classifier_mode(&core);
    *harness.verdict.lock().unwrap() = Some("NEW".into());
    core.doc_host
        .queue_command(CHAT, run_payload("msg-2", &tmp.path().to_string_lossy()))
        .expect("queue run");
    wait_for(|| requests.lock().unwrap().len() == 2, "the message runs").await;
    assert!(
        harness.one_shots.lock().unwrap().is_empty(),
        "no classifier call"
    );
    assert_eq!(prompts(&requests), ["hello", "hello"]);
    assert!(system_entries(&core).is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_stopped_auto_continue_keeps_its_caption() {
    let tmp = tempfile::tempdir().unwrap();
    let (core, harness, _requests) = engine_with_one_turn_on(tmp.path(), true).await;
    let (_release, gate) = tokio::sync::oneshot::channel::<()>();
    *harness.gate.lock().unwrap() = Some(gate);
    core.doc_host
        .queue_command(CHAT, run_payload("msg-2", &tmp.path().to_string_lossy()))
        .expect("queue second run");
    wait_for(|| core.sessions.turn_in_flight(CHAT), "second turn starts").await;

    let last_user = core.doc_host.last_user_entry(CHAT).unwrap();
    assert!(
        core.doc_host
            .queue_auto_continue(CHAT, "go on", last_user.as_deref(), || true)
            .await
            .expect("queue auto-continue")
    );
    let row = core.doc_host.open(CHAT).unwrap().doc().read_queue().unwrap()[0].id.clone();
    assert!(core.doc_host.steer_queued_now(CHAT, &row).await.expect("steer"));
    core.doc_host
        .queue_command(CHAT, SessionCommandPayload::Interrupt { teardown: false })
        .expect("stop");

    wait_for(|| entries(&core).iter().any(|e| e.id == row), "the cancelled row's entry").await;
    let entry = entries(&core).into_iter().find(|e| e.id == row).unwrap();
    assert_eq!(entry.from_chat_id.as_deref(), Some(CHAT));
}

#[tokio::test(flavor = "multi_thread")]
async fn stop_cancels_the_agents_queued_compaction() {
    let tmp = tempfile::tempdir().unwrap();
    let (core, harness, requests) = engine_with_one_turn(tmp.path()).await;
    let (_release, gate) = tokio::sync::oneshot::channel::<()>();
    *harness.gate.lock().unwrap() = Some(gate);
    core.doc_host
        .queue_command(CHAT, run_payload("msg-2", &tmp.path().to_string_lossy()))
        .expect("queue second run");
    wait_for(|| requests.lock().unwrap().len() == 2, "second turn starts").await;
    zeron_rpc::memory_client(core.rpc_service())
        .call(
            zeron_rpc::methods::REQUEST_COMPACT,
            serde_json::json!({ "chatId": CHAT, "continueWith": "go on" }),
        )
        .await
        .expect("request compact");
    core.doc_host
        .queue_command(CHAT, SessionCommandPayload::Interrupt { teardown: false })
        .expect("stop");
    wait_for(|| !core.sessions.turn_in_flight(CHAT), "the turn stops").await;
    assert_eq!(
        core.sessions
            .session_status(CHAT)
            .and_then(|s| s.compact_plan),
        None
    );
    core.doc_host
        .queue_command(CHAT, run_payload("msg-3", &tmp.path().to_string_lossy()))
        .expect("queue third run");
    wait_for(|| requests.lock().unwrap().len() == 3, "the next message").await;
    wait_for(|| !core.sessions.turn_in_flight(CHAT), "its turn settles").await;
    let prompt = requests.lock().unwrap()[2].prompt.clone();
    assert!(
        prompt.contains("Your queued compaction was cancelled: this turn was stopped before it ran.")
            && prompt.contains("Its continueWith was: go on"),
        "{prompt}"
    );
    tokio::time::sleep(Duration::from_secs(2)).await;
    assert_eq!(requests.lock().unwrap().len(), 3, "no continueWith ran after the stop");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_running_turn_is_reminded_once_per_step_of_growth() {
    let tmp = tempfile::tempdir().unwrap();
    let (core, _harness, _requests) = engine_with_one_turn(tmp.path()).await;
    core.registry
        .set_compaction_settings(zeron_engine::registry::CompactionSettings {
            nudge_tokens: 20_000,
            ..Default::default()
        })
        .unwrap();
    let first = core.sessions.mid_turn_nudge(CHAT).expect("past the floor");
    assert!(first.starts_with("Context 22.8k, and this turn is still running."));
    assert_eq!(core.sessions.mid_turn_nudge(CHAT), None, "no growth since the last reminder");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_filling_context_nudges_the_agent_on_every_message() {
    let tmp = tempfile::tempdir().unwrap();
    let (core, harness, requests) = engine_with_one_turn(tmp.path()).await;
    let missions = |missions: bool| {
        core.registry
            .set_compaction_settings(CompactionSettings {
                nudge_tokens: 20_000,
                mode: CompactionMode::Threshold,
                missions,
                ..Default::default()
            })
            .unwrap();
    };
    missions(false);
    let cwd = tmp.path().to_string_lossy().to_string();
    let send = async |id: &str| {
        let before = requests.lock().unwrap().len();
        core.doc_host
            .queue_command(CHAT, run_payload(id, &cwd))
            .expect("queue run");
        wait_for(
            || requests.lock().unwrap().len() == before + 1,
            "run starts",
        )
        .await;
        wait_for(|| !core.sessions.turn_in_flight(CHAT), "run settles").await;
        requests.lock().unwrap().last().unwrap().prompt.clone()
    };
    let nudged = "hello\n\n<system-reminder>\nContext 22.8k. Before answering, decide whether this message starts a new task or topic or continues the current one. New task or topic: call compact first, with the message as continueWith, and do nothing else this turn. Continuing, including any follow-up question: do not compact, unless this turn finishes a piece of work and what comes next needs only its conclusions; then call compact last. It runs after the turn ends; name what to keep.\n</system-reminder>";
    assert_eq!(
        send("msg-2").await,
        nudged,
        "a plain chat ignores the missions switch"
    );
    assert_eq!(
        send("msg-3").await,
        nudged,
        "the reminder repeats until the chat compacts"
    );
    harness.can_compact.store(false, Ordering::SeqCst);
    assert_eq!(
        send("msg-4").await,
        "hello",
        "no reminder when the harness cannot compact"
    );
    harness.can_compact.store(true, Ordering::SeqCst);

    let lead: Task = serde_json::from_value(serde_json::json!({
        "id": "task-1", "title": "Mission", "chatId": CHAT, "mode": "orchestrated",
    }))
    .unwrap();
    core.workspace.create_task(&lead).unwrap();
    let watched = |task: bool| {
        let tasks = core.workspace.watch_tasks();
        let steps = core.workspace.watch_task_steps();
        move || {
            let lead = tasks
                .borrow()
                .iter()
                .any(|t| t.chat_id.as_deref() == Some(CHAT));
            let step = steps
                .borrow()
                .iter()
                .any(|s| s.chat_id.as_deref() == Some(CHAT));
            (lead, step) == (task, !task)
        }
    };
    wait_for(watched(true), "the chat leads a mission").await;
    assert_eq!(
        send("msg-5").await,
        "hello",
        "missions off: a lead is left alone"
    );
    missions(true);
    assert_eq!(
        send("msg-6").await,
        nudged,
        "missions on: a lead follows the same rule"
    );

    missions(false);
    core.workspace
        .update_task(
            "task-1",
            &TaskPatch {
                chat_id: Some(None),
                ..Default::default()
            },
        )
        .unwrap();
    let step: TaskStep = serde_json::from_value(serde_json::json!({
        "id": "step-1", "taskId": "task-1", "title": "Step", "chatId": CHAT,
    }))
    .unwrap();
    core.workspace.create_task_step(&step).unwrap();
    wait_for(watched(false), "the chat runs a step").await;
    assert_eq!(
        send("msg-7").await,
        "hello",
        "missions off: a step thread is left alone"
    );

    let typed: Vec<String> = entries(&core)
        .iter()
        .filter(|e| e.role == MessageRole::User)
        .map(entry_text)
        .collect();
    assert_eq!(typed, ["hello"; 7], "the transcript keeps what was typed");
}

#[tokio::test(flavor = "multi_thread")]
async fn the_classifier_compacts_before_a_new_task_only() {
    let tmp = tempfile::tempdir().unwrap();
    let (core, harness, requests) = engine_with_one_turn(tmp.path()).await;
    core.registry
        .set_compaction_settings(CompactionSettings {
            nudge_tokens: 20_000,
            ..Default::default()
        })
        .unwrap();
    let cwd = tmp.path().to_string_lossy().to_string();
    let send = |id: &'static str, verdict: Option<&str>| {
        *harness.verdict.lock().unwrap() = verdict.map(str::to_string);
        core.doc_host
            .queue_command(CHAT, run_payload(id, &cwd))
            .expect("queue run");
    };

    send("msg-2", Some("CONTINUE"));
    wait_for(|| requests.lock().unwrap().len() == 2, "follow-up runs").await;
    wait_for(|| !core.sessions.turn_in_flight(CHAT), "follow-up settles").await;
    send("msg-3", None);
    wait_for(
        || requests.lock().unwrap().len() == 3,
        "unclassified send runs",
    )
    .await;
    wait_for(|| !core.sessions.turn_in_flight(CHAT), "it settles").await;
    assert_eq!(
        entries(&core)
            .iter()
            .filter(|e| e.role == MessageRole::System)
            .map(verdict_of)
            .collect::<Vec<_>>(),
        [
            Some(ClassifierVerdict::Continue),
            Some(ClassifierVerdict::Failed)
        ],
        "a follow-up or a failed check never compacts"
    );

    send("msg-4", Some("NEW"));
    wait_for(
        || requests.lock().unwrap().len() == 5,
        "compaction, then the new task",
    )
    .await;
    wait_for(|| !core.sessions.turn_in_flight(CHAT), "new task settles").await;
    let prompts: Vec<String> = requests
        .lock()
        .unwrap()
        .iter()
        .map(|r| r.prompt.clone())
        .collect();
    assert_eq!(prompts[1], "hello", "no reminder in classifier mode");
    assert!(
        prompts[3].starts_with("/compact The next message starts a new task.")
            && prompts[3].ends_with("The message: hello"),
        "{}",
        prompts[3]
    );
    assert_eq!(prompts[4], "hello");
    let system: Vec<_> = entries(&core)
        .into_iter()
        .filter(|e| e.role == MessageRole::System)
        .collect();
    assert_eq!(verdict_of(&system[2]), Some(ClassifierVerdict::New));
    assert_eq!(
        entry_text(&system[3]),
        "Compacted before a new task · 22.8k → 2.3k tokens"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_supervisor_wake_is_never_classified() {
    let tmp = tempfile::tempdir().unwrap();
    let (core, harness, requests) = engine_with_one_turn(tmp.path()).await;
    classifier_mode(&core);
    let client = zeron_rpc::memory_client(core.rpc_service());
    let mutate = async |params: serde_json::Value| {
        client
            .call(zeron_rpc::methods::MUTATE, params)
            .await
            .expect("mutate")
    };
    let supervisor = client
        .call(
            zeron_rpc::methods::ENSURE_SUPERVISOR,
            serde_json::json!({ "config": { "harness": "mock", "sandbox": "workspace-write" } }),
        )
        .await
        .expect("ensureSupervisor")["chatId"]
        .as_str()
        .unwrap()
        .to_string();
    let cwd = tmp.path().to_string_lossy().to_string();
    let supervisor_entries = || {
        core.doc_host
            .open(&supervisor)
            .ok()
            .and_then(|h| h.doc().read_entries().ok())
            .unwrap_or_default()
    };
    let (requests, core, supervisor) = (&requests, &core, &supervisor);
    let settled = |n: usize| {
        move || {
            requests.lock().unwrap().len() == n
                && !core.sessions.turn_in_flight(&supervisor)
                && !core.sessions.turn_in_flight("lead-1")
        }
    };
    core.doc_host
        .queue_command(&supervisor, run_payload("sup-1", &cwd))
        .expect("queue run");
    wait_for(settled(2), "the person's first Supervisor turn").await;

    *harness.verdict.lock().unwrap() = Some("NEW".into());
    mutate(serde_json::json!({ "op": "createTask", "taskId": "t1", "title": "Shop backend" }))
        .await;
    mutate(serde_json::json!({
        "op": "startTask", "taskId": "t1", "chatId": "lead-1", "orchestrate": true,
    }))
    .await;
    wait_for(settled(3), "the lead's brief").await;
    zeron_rpc::RpcService::handle(
        &*core.rpc_service(),
        zeron_rpc::methods::MUTATE,
        serde_json::json!({
            "op": "escalateTask", "taskId": "t1", "fromChatId": "lead-1",
            "title": "Pay for a Stripe test account?", "why": "spend",
            "detail": "The checkout tests need a live key.", "options": ["Pay $20", "Use the mock"],
        }),
    )
    .await
    .expect("escalate");
    wait_for(settled(4), "the Supervisor's wake").await;
    let wake = requests.lock().unwrap()[3].prompt.clone();
    assert!(wake.starts_with("Supervisor wake 1 of 12"), "{wake}");
    assert!(
        harness.one_shots.lock().unwrap().is_empty(),
        "no classifier call"
    );
    let entries = supervisor_entries();
    assert!(
        entries.iter().all(|e| record_of(e).is_none()),
        "no classifier record on the Supervisor's chat"
    );
    let row = entries
        .iter()
        .find(|e| e.role == MessageRole::User && entry_text(e) == wake)
        .expect("the wake's entry");
    assert_eq!(row.from_chat_id.as_deref(), Some(supervisor.as_str()));

    core.doc_host
        .queue_command(&supervisor, run_payload("sup-2", &cwd))
        .expect("queue run");
    wait_for(settled(6), "the person's new topic compacts first").await;
    assert!(
        requests.lock().unwrap()[4]
            .prompt
            .starts_with("/compact The next message starts a new task."),
        "the person's own message is still classified"
    );
}

fn verdict_of(entry: &SessionMessageEntry) -> Option<ClassifierVerdict> {
    record_of(entry).map(|record| record.verdict)
}

fn record_of(entry: &SessionMessageEntry) -> Option<&ClassifierRecord> {
    entry.parts.iter().find_map(|part| match part {
        MessagePart::Classifier { record, .. } => Some(record),
        _ => None,
    })
}

fn threshold_mode(core: &EngineCore, nudge_tokens: u64) {
    core.registry
        .set_compaction_settings(CompactionSettings {
            nudge_tokens,
            mode: CompactionMode::Threshold,
            ..Default::default()
        })
        .unwrap();
}

fn classifier_mode(core: &EngineCore) {
    core.registry
        .set_compaction_settings(CompactionSettings {
            nudge_tokens: 20_000,
            ..Default::default()
        })
        .unwrap();
}

fn prompts(requests: &Mutex<Vec<RunRequest>>) -> Vec<String> {
    requests
        .lock()
        .unwrap()
        .iter()
        .map(|r| r.prompt.clone())
        .collect()
}

fn system_entries(core: &EngineCore) -> Vec<SessionMessageEntry> {
    entries(core)
        .into_iter()
        .filter(|e| e.role == MessageRole::System)
        .collect()
}

const CARRY_HEAD: &str = "<carried_over>\nThe last turns of this conversation from before the compaction, verbatim and oldest first: each turn's message and the agent's final reply. Tool calls, their output and thinking are left out. A later turn may have finished what an earlier one planned or listed as next, and the agent's last reply was already delivered to the person.\n\n";

#[tokio::test(flavor = "multi_thread")]
async fn a_carry_verdict_compacts_and_hands_the_last_turns_to_the_next_prompt() {
    let tmp = tempfile::tempdir().unwrap();
    let (core, harness, requests) = engine_with_one_turn(tmp.path()).await;
    classifier_mode(&core);
    harness.replies.lock().unwrap().extend([
        "CONTINUE".to_string(),
        "The plan is still open.\nCARRY".to_string(),
        "\"Asks about the plan the last reply laid out.\"".to_string(),
    ]);
    core.doc_host
        .queue_command(CHAT, run_payload("msg-2", &tmp.path().to_string_lossy()))
        .expect("queue run");
    wait_for(
        || requests.lock().unwrap().len() == 3,
        "compaction, then the message",
    )
    .await;
    wait_for(|| !core.sessions.turn_in_flight(CHAT), "it settles").await;
    let sent = prompts(&requests);
    assert!(
        sent[1].starts_with("/compact The last two turns go along verbatim after this compaction.")
            && sent[1].ends_with("The message: hello"),
        "{}",
        sent[1]
    );
    assert_eq!(
        sent[2],
        format!("{CARRY_HEAD}person: hello\nagent: ack: hello\n</carried_over>\n\nhello")
    );
    wait_for(
        || {
            record_of(&system_entries(&core)[0]).and_then(|record| record.reason.clone())
                == Some("Asks about the plan the last reply laid out.".into())
        },
        "the reason fills the record",
    )
    .await;
    let one_shots = harness.one_shots.lock().unwrap().clone();
    assert_eq!(one_shots.len(), 3);
    assert!(
        one_shots[1].starts_with(&format!("{}\n\n", one_shots[0])),
        "stage 2 shares stage 1's prompt as its prefix"
    );
    assert!(one_shots[0].ends_with("New message (from the person):\nuser: hello"));
    assert_eq!(
        one_shots[2],
        format!(
            "{}\n\nDecision: compact, but keep the last 2 user/agent turns word for word.",
            one_shots[0]
        )
    );
    let system = system_entries(&core);
    let record = record_of(&system[0]).expect("classifier record");
    assert_eq!(record.verdict, ClassifierVerdict::Carry);
    assert_eq!(record.decided_by, DecidedBy::Stage2);
    assert_eq!(record.reply, "CONTINUE");
    assert_eq!(
        record.stage2_reply.as_deref(),
        Some("The plan is still open.\nCARRY")
    );
    assert_eq!(
        entry_text(&system[1]),
        "Compacted, last turns carried over · 22.8k → 2.3k tokens"
    );
    let compaction = system[1]
        .parts
        .iter()
        .find_map(|part| match part {
            MessagePart::Compaction { record, .. } => Some(record.clone()),
            _ => None,
        })
        .expect("compaction record");
    assert_eq!(
        (compaction.pre_tokens, compaction.post_tokens),
        (Some(22_762), Some(2_326))
    );
    assert!(compaction.duration_ms.is_some());
    assert_eq!(
        compaction.carried.as_deref(),
        Some(format!("{CARRY_HEAD}person: hello\nagent: ack: hello\n</carried_over>").as_str())
    );

    *harness.verdict.lock().unwrap() = Some("CONTINUE".into());
    core.doc_host
        .queue_command(CHAT, run_payload("msg-3", &tmp.path().to_string_lossy()))
        .expect("queue run");
    wait_for(|| requests.lock().unwrap().len() == 4, "the next message").await;
    assert_eq!(prompts(&requests)[3], "hello", "the carry rides one prompt");
}

#[tokio::test(flavor = "multi_thread")]
async fn the_floor_ignores_context_a_compaction_cannot_free() {
    let tmp = tempfile::tempdir().unwrap();
    let requests: Arc<Mutex<Vec<RunRequest>>> = Arc::new(Mutex::new(Vec::new()));
    let harness = Arc::new(TestHarness::new(requests.clone(), false));
    harness.first_reading.store(15_000, Ordering::SeqCst);
    let core = engine_with_one_turn_by(tmp.path(), harness.clone()).await;
    classifier_mode(&core);
    *harness.verdict.lock().unwrap() = Some("NEW".into());
    core.doc_host
        .queue_command(CHAT, run_payload("msg-2", &tmp.path().to_string_lossy()))
        .expect("queue run");
    wait_for(|| requests.lock().unwrap().len() == 2, "the message runs").await;
    wait_for(|| !core.sessions.turn_in_flight(CHAT), "it settles").await;
    assert!(
        system_entries(&core).is_empty(),
        "22.8k with a 15k baseline frees under 20k"
    );
    assert!(harness.one_shots.lock().unwrap().is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn three_same_task_verdicts_on_a_growing_context_carry_the_next_message() {
    let tmp = tempfile::tempdir().unwrap();
    let (core, harness, requests) = engine_with_one_turn(tmp.path()).await;
    classifier_mode(&core);
    let cwd = tmp.path().to_string_lossy().to_string();
    *harness.verdict.lock().unwrap() = Some("CONTINUE".into());
    for (n, id) in ["msg-2", "msg-3", "msg-4"].into_iter().enumerate() {
        harness
            .tokens
            .store(25_000 + 2_000 * n as u64, Ordering::SeqCst);
        core.doc_host
            .queue_command(CHAT, run_payload(id, &cwd))
            .expect("queue run");
        wait_for(|| requests.lock().unwrap().len() == n + 2, "run starts").await;
        wait_for(|| !core.sessions.turn_in_flight(CHAT), "run settles").await;
    }
    *harness.verdict.lock().unwrap() = None;
    let asked = harness.one_shots.lock().unwrap().len();
    core.doc_host
        .queue_command(CHAT, run_payload("msg-5", &cwd))
        .expect("queue run");
    wait_for(
        || requests.lock().unwrap().len() == 6,
        "compaction, then the message",
    )
    .await;
    assert_eq!(
        harness.one_shots.lock().unwrap().len(),
        asked,
        "no classifier call"
    );
    let decided: Vec<_> = system_entries(&core)
        .iter()
        .filter_map(record_of)
        .map(|record| (record.verdict, record.decided_by))
        .collect();
    assert_eq!(
        decided,
        [
            (ClassifierVerdict::Continue, DecidedBy::Stage2),
            (ClassifierVerdict::Continue, DecidedBy::Stage2),
            (ClassifierVerdict::Continue, DecidedBy::Stage2),
            (ClassifierVerdict::Carry, DecidedBy::Streak),
        ]
    );
    assert!(prompts(&requests)[5].starts_with(CARRY_HEAD));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_same_task_agent_note_skips_the_compaction_and_says_why() {
    let tmp = tempfile::tempdir().unwrap();
    let (core, harness, requests) = engine_with_one_turn(tmp.path()).await;
    *harness.verdict.lock().unwrap() = Some("CONTINUE".into());
    zeron_rpc::memory_client(core.rpc_service())
        .call(
            zeron_rpc::methods::REQUEST_COMPACT,
            serde_json::json!({ "chatId": CHAT, "continueWith": "now run the tests" }),
        )
        .await
        .expect("request compact");
    wait_for(|| prompts(&requests).len() == 2, "the agent's note").await;
    assert_eq!(
        prompts(&requests)[1],
        "now run the tests\n\n<system-reminder>\nYour compaction did not run: the classifier judged that the next step still needs the recent details. Keep working, and call compact again at the next clean break.\n</system-reminder>"
    );
    let input = harness.one_shots.lock().unwrap()[0].clone();
    assert!(
        input.contains("New message (the agent's own note, written when it asked to compact):\nagent note: now run the tests\n\n"),
        "{input}"
    );
    assert_eq!(
        system_entries(&core)
            .iter()
            .map(verdict_of)
            .collect::<Vec<_>>(),
        [Some(ClassifierVerdict::Continue)]
    );
    assert_eq!(
        core.sessions
            .session_status(CHAT)
            .and_then(|s| s.compact_plan),
        None
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_carry_agent_note_compacts_and_resumes_with_the_last_turns() {
    let tmp = tempfile::tempdir().unwrap();
    let (core, harness, requests) = engine_with_one_turn(tmp.path()).await;
    *harness.verdict.lock().unwrap() = Some("CARRY".into());
    zeron_rpc::memory_client(core.rpc_service())
        .call(
            zeron_rpc::methods::REQUEST_COMPACT,
            serde_json::json!({ "chatId": CHAT, "focus": "keep the plan", "continueWith": "phase 2" }),
        )
        .await
        .expect("request compact");
    wait_for(
        || prompts(&requests).len() == 3,
        "compaction, then the note",
    )
    .await;
    assert_eq!(prompts(&requests)[1], "/compact keep the plan");
    assert_eq!(
        prompts(&requests)[2],
        format!("{CARRY_HEAD}person: hello\nagent: ack: hello\n</carried_over>\n\nphase 2")
    );
    let system = system_entries(&core);
    assert_eq!(verdict_of(&system[0]), Some(ClassifierVerdict::Carry));
    assert_eq!(
        entry_text(&system[1]),
        "Agent compacted, last turns carried over · 22.8k → 2.3k tokens"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_new_verdict_on_an_agent_note_runs_as_carry() {
    let tmp = tempfile::tempdir().unwrap();
    let (core, harness, requests) = engine_with_one_turn(tmp.path()).await;
    *harness.verdict.lock().unwrap() = Some("NEW".into());
    zeron_rpc::memory_client(core.rpc_service())
        .call(
            zeron_rpc::methods::REQUEST_COMPACT,
            serde_json::json!({ "chatId": CHAT, "continueWith": "phase 2" }),
        )
        .await
        .expect("request compact");
    wait_for(
        || prompts(&requests).len() == 3,
        "compaction, then the note",
    )
    .await;
    assert!(prompts(&requests)[2].starts_with(CARRY_HEAD));
    let system = system_entries(&core);
    let record = record_of(&system[0]).expect("classifier record");
    assert_eq!(
        (
            record.verdict,
            record.classifier_verdict,
            record.reply.as_str()
        ),
        (
            ClassifierVerdict::Carry,
            Some(ClassifierVerdict::New),
            "NEW"
        )
    );
    assert!(entry_text(&system[1]).starts_with("Agent compacted, last turns carried over"));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_carry_lists_the_jobs_still_running() {
    let tmp = tempfile::tempdir().unwrap();
    let requests: Arc<Mutex<Vec<RunRequest>>> = Arc::new(Mutex::new(Vec::new()));
    let harness = Arc::new(TestHarness::new(requests.clone(), true));
    harness.in_place.store(true, Ordering::SeqCst);
    *harness.background.lock().unwrap() = vec![zeron_proto::BackgroundTask {
        id: "b7wb5vp5o".into(),
        kind: zeron_proto::BackgroundTaskKind::Shell,
        label: "sleep 300 && echo done".into(),
        started_at: None,
        output_path: None,
    }];
    let core = engine_with_one_turn_by(tmp.path(), harness.clone()).await;
    let delegation: zeron_proto::Delegation = serde_json::from_value(serde_json::json!({
        "delegatedByChatId": CHAT,
        "depth": 1,
        "anchor": "task-message",
        "wake": "settledOnly",
        "status": "running",
        "startedAt": "2026-10-18T00:00:00Z",
    }))
    .unwrap();
    core.workspace
        .create_delegated_chat(
            "child-task",
            None,
            Some(&core.device_id),
            None,
            Some(tmp.path().to_string_lossy().to_string()),
            delegation,
        )
        .expect("delegated chat");
    core.workspace
        .rename_chat("child-task", "Audit the parser")
        .expect("title");
    classifier_mode(&core);
    *harness.verdict.lock().unwrap() = Some("CARRY".into());
    core.doc_host
        .queue_command(CHAT, run_payload("msg-2", &tmp.path().to_string_lossy()))
        .expect("queue run");
    let mailbox = || harness.mailbox.lock().unwrap().clone();
    wait_for(|| mailbox().len() == 2, "compaction, then the message").await;
    let sent = mailbox()[1].clone();
    assert!(
        sent.contains("agent: ack: hello\n\nStill running when the chat compacted:\n- background shell b7wb5vp5o: sleep 300 && echo done\n- delegated task child-task: Audit the parser\n</carried_over>\n\nhello"),
        "{sent}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_nearly_full_window_compacts_an_agent_note_without_asking() {
    let tmp = tempfile::tempdir().unwrap();
    let requests: Arc<Mutex<Vec<RunRequest>>> = Arc::new(Mutex::new(Vec::new()));
    let harness = Arc::new(TestHarness::new(requests.clone(), false));
    harness.tokens.store(WINDOW * 9 / 10, Ordering::SeqCst);
    let core = engine_with_one_turn_by(tmp.path(), harness.clone()).await;
    zeron_rpc::memory_client(core.rpc_service())
        .call(
            zeron_rpc::methods::REQUEST_COMPACT,
            serde_json::json!({ "chatId": CHAT, "continueWith": "phase 2" }),
        )
        .await
        .expect("request compact");
    wait_for(
        || prompts(&requests).len() == 3,
        "compaction, then the note",
    )
    .await;
    assert!(
        harness.one_shots.lock().unwrap().is_empty(),
        "no classifier call"
    );
    assert_eq!(prompts(&requests)[1], "/compact");
    assert!(prompts(&requests)[2].starts_with(CARRY_HEAD));
    let record = system_entries(&core)
        .iter()
        .find_map(|e| record_of(e).cloned())
        .expect("a record");
    assert_eq!(
        (record.verdict, record.decided_by, record.context_tokens),
        (
            ClassifierVerdict::Carry,
            DecidedBy::Ceiling,
            WINDOW * 9 / 10
        )
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn the_classifier_compacts_a_parked_run_in_place() {
    let tmp = tempfile::tempdir().unwrap();
    let requests: Arc<Mutex<Vec<RunRequest>>> = Arc::new(Mutex::new(Vec::new()));
    let harness = Arc::new(TestHarness::new(requests.clone(), true));
    harness.in_place.store(true, Ordering::SeqCst);
    let core = engine_with_one_turn_by(tmp.path(), harness.clone()).await;
    core.registry
        .set_compaction_settings(CompactionSettings {
            nudge_tokens: 20_000,
            ..Default::default()
        })
        .unwrap();
    *harness.verdict.lock().unwrap() = Some("NEW".into());
    core.doc_host
        .queue_command(CHAT, run_payload("msg-2", &tmp.path().to_string_lossy()))
        .expect("queue run");
    let mailbox = || harness.mailbox.lock().unwrap().clone();
    wait_for(|| mailbox().len() == 2, "compaction, then the message").await;
    assert!(mailbox()[0].starts_with("/compact The next message starts a new task."));
    assert_eq!(mailbox()[1], "hello");
    assert_eq!(requests.lock().unwrap().len(), 1, "no second process");
}

#[tokio::test(flavor = "multi_thread")]
async fn the_classifier_starts_fresh_on_a_harness_that_cannot_compact() {
    let tmp = tempfile::tempdir().unwrap();
    let (core, harness, requests) = engine_with_one_turn(tmp.path()).await;
    harness.can_compact.store(false, Ordering::SeqCst);
    core.registry
        .set_compaction_settings(CompactionSettings {
            nudge_tokens: 20_000,
            ..Default::default()
        })
        .unwrap();
    *harness.verdict.lock().unwrap() = Some("NEW".into());
    core.doc_host
        .queue_command(CHAT, run_payload("msg-2", &tmp.path().to_string_lossy()))
        .expect("queue run");
    wait_for(|| requests.lock().unwrap().len() == 2, "the new task runs").await;
    let next = requests.lock().unwrap()[1].clone();
    assert_eq!(next.resume, None, "a new session");
    assert!(
        next.prompt.starts_with("<fresh_session>"),
        "{}",
        next.prompt
    );
    assert!(
        next.prompt
            .ends_with("<latest_user_message>\nhello\n</latest_user_message>"),
        "{}",
        next.prompt
    );
    let system: Vec<_> = entries(&core)
        .into_iter()
        .filter(|e| e.role == MessageRole::System)
        .collect();
    assert_eq!(verdict_of(&system[0]), Some(ClassifierVerdict::New));
    assert!(entry_text(&system[1]).starts_with("Fresh session before a new task"));
    assert!(
        system[1]
            .parts
            .iter()
            .any(|part| matches!(part, MessagePart::Handoff { .. }))
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_provider_switch_hands_over_instead_of_compacting() {
    let tmp = tempfile::tempdir().unwrap();
    let (core, harness, requests) = engine_with_one_turn(tmp.path()).await;
    classifier_mode(&core);
    core.sessions.switch_harness(CHAT).await.expect("switch");
    let outcome = core.sessions.compact(CHAT, None).await.expect("compaction");
    assert!(!outcome.compacted);
    assert_eq!(requests.lock().unwrap().len(), 1, "no compaction run");

    *harness.verdict.lock().unwrap() = Some("NEW".into());
    core.doc_host
        .queue_command(CHAT, run_payload("msg-2", &tmp.path().to_string_lossy()))
        .expect("queue run");
    wait_for(|| requests.lock().unwrap().len() == 2, "the message runs").await;
    assert!(
        harness.one_shots.lock().unwrap().is_empty(),
        "no classifier call"
    );
    let next = requests.lock().unwrap()[1].clone();
    assert_eq!(next.resume, None, "the new harness's own session");
    assert!(
        next.prompt.starts_with("<provider_switch>"),
        "{}",
        next.prompt
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_fresh_session_carries_the_last_turns_ahead_of_its_packet() {
    let tmp = tempfile::tempdir().unwrap();
    let (core, harness, requests) = engine_with_one_turn(tmp.path()).await;
    harness.can_compact.store(false, Ordering::SeqCst);
    classifier_mode(&core);
    *harness.verdict.lock().unwrap() = Some("CARRY".into());
    core.doc_host
        .queue_command(CHAT, run_payload("msg-2", &tmp.path().to_string_lossy()))
        .expect("queue run");
    wait_for(|| requests.lock().unwrap().len() == 2, "the message runs").await;
    let next = requests.lock().unwrap()[1].clone();
    assert_eq!(next.resume, None, "a new session");
    let packet = next
        .prompt
        .strip_prefix(&format!(
            "{CARRY_HEAD}person: hello\nagent: ack: hello\n</carried_over>\n\n"
        ))
        .unwrap_or_else(|| panic!("{}", next.prompt));
    assert!(packet.starts_with("<fresh_session>"), "{packet}");
    assert!(
        entry_text(&system_entries(&core)[1]).starts_with("Fresh session, last turns carried over")
    );
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

#[tokio::test(flavor = "multi_thread")]
async fn cancelling_compaction_releases_its_run_slot() {
    let tmp = tempfile::tempdir().unwrap();
    let (core, harness, _requests) = engine_with_one_turn(tmp.path()).await;
    let (_release, gate) = tokio::sync::oneshot::channel();
    *harness.gate.lock().unwrap() = Some(gate);
    let sessions = core.sessions.clone();
    let compact = tokio::spawn(async move { sessions.compact(CHAT, None).await });
    wait_for(|| core.sessions.turn_in_flight(CHAT), "compaction starts").await;
    compact.abort();
    assert!(compact.await.unwrap_err().is_cancelled());
    assert!(!core.sessions.turn_in_flight(CHAT));
    assert!(core.sessions.compact(CHAT, None).await.unwrap().compacted);
}
