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
    /// False reports `Compaction::None`, like Pi or an ACP agent without `/compact`.
    can_compact: AtomicBool,
    in_place: AtomicBool,
    mailbox: Arc<Mutex<Vec<String>>>,
    /// Held by the next run until the test releases it.
    gate: Mutex<Option<tokio::sync::oneshot::Receiver<()>>>,
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
    core.registry
        .set_compaction_settings(zeron_engine::registry::CompactionSettings {
            nudge_tokens: 2_000,
            ..Default::default()
        })
        .unwrap();
    let (release, gate) = tokio::sync::oneshot::channel();
    *harness.gate.lock().unwrap() = Some(gate);
    core.doc_host
        .queue_command(CHAT, run_payload("msg-2", &tmp.path().to_string_lossy()))
        .expect("queue second run");
    wait_for(|| core.sessions.turn_in_flight(CHAT), "second turn starts").await;

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
        .find(|e| e.role == MessageRole::System)
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

    wait_for(|| prompts().iter().any(|p| p == "a new question"), "the person's message").await;
    wait_for(|| !core.sessions.turn_in_flight(CHAT), "its turn settles").await;
    tokio::time::sleep(Duration::from_secs(1)).await;
    assert_eq!(prompts()[1..], ["/compact", "a new question"]);
    assert!(
        entries(&core)
            .iter()
            .any(|e| e.role == MessageRole::System && entry_text(e).starts_with("Agent compacted"))
    );
}

/// A parked run can open its own turn (a finished background shell); interrupting it loses that turn.
#[tokio::test(flavor = "multi_thread")]
async fn an_agent_compaction_queues_into_the_parked_run() {
    let tmp = tempfile::tempdir().unwrap();
    let requests: Arc<Mutex<Vec<RunRequest>>> = Arc::new(Mutex::new(Vec::new()));
    let harness = Arc::new(TestHarness::new(requests.clone(), true));
    harness.in_place.store(true, Ordering::SeqCst);
    let core = engine_with_one_turn_by(tmp.path(), harness.clone()).await;

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
            .queue_auto_continue(CHAT, "go on", last_user.as_deref())
            .await
            .expect("queue auto-continue")
    );
    let row = core.doc_host.open(CHAT).unwrap().doc().read_queue().unwrap()[0].id.clone();
    assert!(core.doc_host.steer_queued_now(CHAT, &row).await.expect("steer"));
    core.doc_host
        .queue_command(CHAT, SessionCommandPayload::Interrupt {})
        .expect("stop");

    wait_for(|| entries(&core).iter().any(|e| e.id == row), "the cancelled row's entry").await;
    let entry = entries(&core).into_iter().find(|e| e.id == row).unwrap();
    assert_eq!(entry.from_chat_id.as_deref(), Some(CHAT));
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
    core.registry
        .set_compaction_settings(zeron_engine::registry::CompactionSettings {
            nudge_tokens: 20_000,
            ..Default::default()
        })
        .unwrap();
    let cwd = tmp.path().to_string_lossy().to_string();
    for (ix, id) in ["msg-2", "msg-3", "msg-4"].into_iter().enumerate() {
        if id == "msg-4" {
            harness.can_compact.store(false, Ordering::SeqCst);
        }
        core.doc_host
            .queue_command(CHAT, run_payload(id, &cwd))
            .expect("queue run");
        wait_for(|| requests.lock().unwrap().len() == ix + 2, "run starts").await;
        wait_for(|| !core.sessions.turn_in_flight(CHAT), "run settles").await;
    }
    let prompts: Vec<String> = requests.lock().unwrap().iter().map(|r| r.prompt.clone()).collect();
    let nudged = "hello\n\n<system-reminder>\nContext 22.8k. Before answering, decide whether this message starts a new task or topic or continues the current one. New task or topic: call compact first, with the message as continueWith, and do nothing else this turn. Continuing, including any follow-up question: do not compact, unless this turn finishes a piece of work and what comes next needs only its conclusions; then call compact last. It runs after the turn ends; name what to keep.\n</system-reminder>";
    assert_eq!(prompts[1], nudged);
    assert_eq!(prompts[2], nudged, "the reminder repeats until the chat compacts");
    assert_eq!(prompts[3], "hello", "no reminder when the harness cannot compact");
    let typed: Vec<String> = entries(&core)
        .iter()
        .filter(|e| e.role == MessageRole::User)
        .map(entry_text)
        .collect();
    assert_eq!(typed, ["hello", "hello", "hello", "hello"], "the transcript keeps what was typed");
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
