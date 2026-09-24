//! Thread tools (peer chats, d25): a message from one chat lands on another
//! as a queued row tagged with its origin, opens a turn on an idle peer,
//! waits on a working one, and reaches the transcript with its provenance.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use futures::StreamExt;
use futures::stream::BoxStream;

use zeron_doc::{
    CommandBasedOn, MessageRole, SessionCommandEntry, SessionCommandPayload, SessionCommandStatus,
};
use zeron_engine::{EngineCore, HarnessRegistry};
use zeron_harness::{Harness, HarnessError, RunControls};
use zeron_proto::{
    AgentEvent, DoneStatus, HarnessId, Model, ReasoningLevel, RunRequest, SandboxLevel,
    SessionStatus, SteeringMode,
};

/// Answers every prompt after `step_delay`, recording what it was asked and,
/// when steerable, what reached its mid-turn mailbox.
struct RecordingHarness {
    prompts: Arc<Mutex<Vec<String>>>,
    steers: Arc<Mutex<Vec<String>>>,
    steerable: bool,
    step_delay: Duration,
}

#[async_trait]
impl Harness for RecordingHarness {
    fn id(&self) -> HarnessId {
        HarnessId::Mock
    }
    fn display_name(&self) -> &str {
        "Recording"
    }
    fn supports_steering(&self) -> bool {
        self.steerable
    }
    fn steering_mode(&self) -> SteeringMode {
        if self.steerable {
            SteeringMode::StepBoundary
        } else {
            SteeringMode::TurnBoundary
        }
    }
    fn reasoning_levels(&self) -> &[ReasoningLevel] {
        &[]
    }
    async fn models(&self) -> Result<Vec<Model>, HarnessError> {
        Ok(vec![])
    }
    async fn run(
        &self,
        request: RunRequest,
        mut controls: RunControls,
    ) -> Result<BoxStream<'static, Result<AgentEvent, HarnessError>>, HarnessError> {
        self.prompts.lock().unwrap().push(request.prompt.clone());
        let (tx, rx) = tokio::sync::mpsc::channel::<Result<AgentEvent, HarnessError>>(8);
        let steers = self.steers.clone();
        let confirm = tx.clone();
        tokio::spawn(async move {
            while let Some(message) = controls.steering.recv().await {
                steers.lock().unwrap().push(message.prompt);
                // A real harness confirms the fold; without it the engine
                // treats the steer as lost and re-runs it at turn end.
                let _ = confirm
                    .send(Ok(AgentEvent::Steered {
                        assistant_message_id: Some("a-rec".into()),
                        next_assistant_message_id: Some("a-rec-2".into()),
                    }))
                    .await;
            }
        });
        let delay = self.step_delay;
        tokio::spawn(async move {
            let script = vec![
                AgentEvent::SessionStarted {
                    harness: HarnessId::Mock,
                    model: "rec-1".into(),
                    tools: vec![],
                    cwd: "/tmp".into(),
                    session_id: "hs-rec".into(),
                    assistant_message_id: "a-rec".into(),
                },
                AgentEvent::TextDelta {
                    text: "noted".into(),
                },
                AgentEvent::Done {
                    status: DoneStatus::Completed,
                    result: None,
                    error: None,
                    session_id: Some("hs-rec".into()),
                },
            ];
            for event in script {
                tokio::time::sleep(delay).await;
                if tx.send(Ok(event)).await.is_err() {
                    return;
                }
            }
        });
        Ok(futures::stream::unfold(rx, |mut rx| async move {
            rx.recv().await.map(|event| (event, rx))
        })
        .boxed())
    }
}

struct Rig {
    core: EngineCore,
    prompts: Arc<Mutex<Vec<String>>>,
    steers: Arc<Mutex<Vec<String>>>,
    _dir: tempfile::TempDir,
}

fn rig(step_delay: Duration) -> Rig {
    rig_with(step_delay, false)
}

fn rig_with(step_delay: Duration, steerable: bool) -> Rig {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("device-id"), "dev-a").unwrap();
    let prompts = Arc::new(Mutex::new(Vec::new()));
    let steers = Arc::new(Mutex::new(Vec::new()));
    let registry = HarnessRegistry::new();
    registry.register(Arc::new(RecordingHarness {
        prompts: prompts.clone(),
        steers: steers.clone(),
        steerable,
        step_delay,
    }));
    let core = EngineCore::assemble(dir.path(), Arc::new(registry), HarnessId::Mock, None)
        .expect("engine core assembles");
    core.workspace
        .create_space("space-tt", "dev-a", "/tmp", None, false)
        .expect("space");
    core.workspace
        .create_chat("lead", Some("space-tt"), None, None, None)
        .expect("lead chat");
    Rig {
        core,
        prompts,
        steers,
        _dir: dir,
    }
}

async fn wait_for<F>(mut predicate: F, what: &str)
where
    F: FnMut() -> bool,
{
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    while !predicate() {
        assert!(
            tokio::time::Instant::now() < deadline,
            "timed out waiting for {what}"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

fn entries(core: &EngineCore, chat_id: &str) -> Vec<zeron_doc::SessionMessageEntry> {
    core.doc_host
        .open(chat_id)
        .expect("open chat")
        .doc()
        .read_entries()
        .unwrap_or_default()
}

fn queue_len(core: &EngineCore, chat_id: &str) -> usize {
    core.doc_host
        .open(chat_id)
        .expect("open chat")
        .doc()
        .read_queue()
        .map(|q| q.len())
        .unwrap_or(0)
}

fn idle(core: &EngineCore, chat_id: &str) -> bool {
    core.sessions
        .session_status(chat_id)
        .is_some_and(|s| s.status == SessionStatus::Idle)
}

/// A person's own run on `chat_id`, the way a viewport queues it.
fn queue_human_run(core: &EngineCore, chat_id: &str, command_id: &str, message_id: &str) {
    let handle = core.doc_host.open(chat_id).expect("open chat");
    handle
        .doc()
        .queue_command(&SessionCommandEntry {
            id: command_id.into(),
            payload: SessionCommandPayload::Run {
                request: RunRequest {
                    prompt: "human prompt".into(),
                    harness: None,
                    model: None,
                    reasoning: None,
                    model_options: Default::default(),
                    cwd: "/tmp".into(),
                    sandbox: SandboxLevel::WorkspaceWrite,
                    permission_mode: zeron_proto::PermissionMode::Full,
                    auto_approve: true,
                    attachments: Vec::new(),
                    worktree: None,
                    branch: None,
                    resume: None,
                    forked_at: None,
                    computer_control: false,
                },
                message_id: message_id.into(),
            },
            issued_by: "viewer".into(),
            issued_at: chrono::Utc::now().timestamp_millis(),
            based_on: None::<CommandBasedOn>,
            expires_at: None,
            status: SessionCommandStatus::Pending,
            resolution: None,
            computer_generation: None,
        })
        .expect("queue command");
}

#[tokio::test]
async fn create_thread_opens_a_peer_whose_first_turn_is_a_message_from_its_creator() {
    let rig = rig(Duration::from_millis(10));
    let peer = rig
        .core
        .doc_host
        .create_thread(
            "lead",
            "write the login tests",
            Some("Login tests"),
            None,
            None,
            None,
        )
        .await
        .expect("create thread");
    // Drained before the call returned: the creator's very next wait_threads
    // sees a Working (or already finished) peer, never a not-yet-started one.
    assert!(
        rig.core.sessions.session_status(&peer).is_some(),
        "the peer's first turn was dispatched synchronously"
    );

    let row = rig.core.workspace.chat(&peer).unwrap().expect("peer row");
    assert_eq!(row.created_by_chat_id.as_deref(), Some("lead"));
    assert_eq!(row.title.as_deref(), Some("Login tests"));
    assert_eq!(row.space_id.as_deref(), Some("space-tt"));
    assert_eq!(row.sidechat_source_chat_id, None);

    wait_for(
        || entries(&rig.core, &peer).len() >= 2 && idle(&rig.core, &peer),
        "the peer's first turn",
    )
    .await;
    let first = &entries(&rig.core, &peer)[0];
    assert_eq!(first.role, MessageRole::User);
    assert_eq!(first.from_chat_id.as_deref(), Some("lead"));
    assert_eq!(
        first.parts,
        vec![zeron_doc::MessagePart::Text {
            id: "t0".into(),
            text: "write the login tests".into()
        }],
        "the transcript keeps the raw text; framing is for the harness only"
    );
    let prompts = rig.prompts.lock().unwrap().clone();
    assert_eq!(prompts.len(), 1);
    assert!(
        prompts[0].starts_with("[Message from thread lead.")
            && prompts[0].contains("send_message_to_thread (threadId lead)"),
        "{}",
        prompts[0]
    );
    assert!(prompts[0].ends_with("write the login tests"));
    assert_eq!(queue_len(&rig.core, &peer), 0);
    rig.core.shutdown().await;
}

#[tokio::test]
async fn create_thread_picks_the_peers_run_settings_and_copies_the_creators_otherwise() {
    let rig = rig(Duration::from_millis(1));
    let creators = zeron_proto::ChatConfig {
        harness: HarnessId::Mock,
        model: Some("rec-1".into()),
        reasoning: Some(ReasoningLevel::High),
        model_options: Default::default(),
        sandbox: SandboxLevel::ReadOnly,
        permission_mode: zeron_proto::PermissionMode::Auto,
    };
    rig.core
        .workspace
        .create_chat(
            "picky",
            Some("space-tt"),
            None,
            Some(creators.clone()),
            None,
        )
        .unwrap();
    let config_of = |chat_id: &str| rig.core.workspace.chat(chat_id).unwrap().unwrap().config;

    let chosen = rig
        .core
        .doc_host
        .create_thread(
            "picky",
            "task",
            None,
            Some(HarnessId::Mock),
            Some(" rec-2 "),
            Some(ReasoningLevel::Low),
        )
        .await
        .unwrap();
    assert_eq!(
        config_of(&chosen),
        Some(zeron_proto::ChatConfig {
            model: Some("rec-2".into()),
            reasoning: Some(ReasoningLevel::Low),
            ..creators.clone()
        }),
        "the picks land on the row; sandbox and permissions stay the creator's"
    );

    let copied = rig
        .core
        .doc_host
        .create_thread("picky", "task", None, None, None, None)
        .await
        .unwrap();
    assert_eq!(config_of(&copied), Some(creators.clone()));

    // Switching harness drops the inherited model (it names a model of the
    // old harness) but keeps the reasoning level, Zeron's own ladder.
    let switched = rig
        .core
        .doc_host
        .create_thread("picky", "task", None, Some(HarnessId::Codex), None, None)
        .await
        .unwrap();
    assert_eq!(
        config_of(&switched),
        Some(zeron_proto::ChatConfig {
            harness: HarnessId::Codex,
            model: None,
            ..creators.clone()
        })
    );
    let switched_with_model = rig
        .core
        .doc_host
        .create_thread(
            "picky",
            "task",
            None,
            Some(HarnessId::Codex),
            Some("gpt-5.6-luna"),
            None,
        )
        .await
        .unwrap();
    assert_eq!(
        config_of(&switched_with_model).and_then(|c| c.model),
        Some("gpt-5.6-luna".into())
    );

    // A creator without a config row of its own: the pick still lands, on
    // the engine's default harness.
    let from_bare = rig
        .core
        .doc_host
        .create_thread("lead", "task", None, None, Some("rec-3"), None)
        .await
        .unwrap();
    let config = config_of(&from_bare).expect("a config row");
    assert_eq!(config.harness, HarnessId::Mock);
    assert_eq!(config.model.as_deref(), Some("rec-3"));
    assert_eq!(config.reasoning, None);
    rig.core.shutdown().await;
}

#[tokio::test]
async fn self_and_cross_device_sends_are_refused_but_a_cross_project_send_is_delivered() {
    let rig = rig(Duration::from_millis(10));
    let err = rig
        .core
        .doc_host
        .queue_thread_message("lead", "lead", "hi")
        .await
        .expect_err("self-send");
    assert!(err.to_string().contains("cannot message itself"), "{err}");

    rig.core
        .workspace
        .create_chat("remote", Some("space-tt"), None, None, None)
        .unwrap();
    rig.core.workspace.set_chat_host("remote", "dev-b").unwrap();
    let err = rig
        .core
        .doc_host
        .queue_thread_message("remote", "lead", "hi")
        .await
        .expect_err("cross-device send");
    assert!(err.to_string().contains("same device"), "{err}");
    let err = rig
        .core
        .doc_host
        .queue_thread_message("nope", "lead", "hi")
        .await
        .expect_err("unknown target");
    assert!(err.to_string().contains("no such chat: nope"), "{err}");
    assert_eq!(rig.prompts.lock().unwrap().len(), 0);

    // Another project on the same device is in the sidebar the human sees,
    // so it is a peer: the idle target starts a turn with the message.
    rig.core
        .workspace
        .create_space("space-other", "dev-a", "/tmp/other", None, false)
        .unwrap();
    rig.core
        .workspace
        .create_chat("elsewhere", Some("space-other"), None, None, None)
        .unwrap();
    rig.core
        .doc_host
        .queue_thread_message("elsewhere", "lead", "hi")
        .await
        .expect("cross-project send");
    wait_for(
        || rig.prompts.lock().unwrap().len() == 1,
        "the cross-project message to start a turn",
    )
    .await;
    let from_lead: Vec<_> = entries(&rig.core, "elsewhere")
        .into_iter()
        .filter(|e| e.from_chat_id.as_deref() == Some("lead"))
        .collect();
    assert_eq!(from_lead.len(), 1);
    rig.core.shutdown().await;
}

#[tokio::test]
async fn a_message_to_a_working_peer_steers_its_turn_when_the_agent_takes_mid_turn_input() {
    let rig = rig_with(Duration::from_millis(300), true);
    let peer = rig
        .core
        .doc_host
        .create_thread("lead", "first task", None, None, None, None)
        .await
        .unwrap();
    wait_for(
        || rig.core.sessions.turn_in_flight(&peer),
        "the peer's turn to start",
    )
    .await;

    let queued = rig
        .core
        .doc_host
        .queue_thread_message(&peer, "lead", "also cover the refresh race")
        .await
        .unwrap();
    assert!(
        rig.core.sessions.turn_in_flight(&peer),
        "the turn was not interrupted"
    );
    assert_eq!(queue_len(&rig.core, &peer), 0, "nothing left held");
    let now = entries(&rig.core, &peer);
    assert_eq!(
        now.len(),
        2,
        "the tagged entry is in the transcript at once"
    );
    assert_eq!(now[1].id, queued);
    assert_eq!(now[1].from_chat_id.as_deref(), Some("lead"));
    wait_for(
        || rig.steers.lock().unwrap().len() == 1,
        "the framed message to reach the live turn",
    )
    .await;
    let steer = rig.steers.lock().unwrap()[0].clone();
    assert!(steer.starts_with("[Message from thread"), "{steer}");
    assert!(steer.ends_with("also cover the refresh race"), "{steer}");

    wait_for(|| idle(&rig.core, &peer), "the peer's turn to end").await;
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(
        rig.prompts.lock().unwrap().len(),
        1,
        "a steered message never runs again at turn end"
    );
    rig.core.shutdown().await;
}

#[tokio::test]
async fn a_message_to_a_working_peer_waits_and_take_moves_it_into_the_transcript() {
    // A slow harness that takes no mid-turn input keeps the peer Working
    // long enough to observe the hold.
    let rig = rig(Duration::from_millis(300));
    let peer = rig
        .core
        .doc_host
        .create_thread("lead", "first task", None, None, None, None)
        .await
        .unwrap();
    wait_for(
        || rig.core.sessions.turn_in_flight(&peer),
        "the peer's turn to start",
    )
    .await;

    let queued = rig
        .core
        .doc_host
        .queue_thread_message(&peer, "lead", "also cover the refresh race")
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(
        rig.core.sessions.turn_in_flight(&peer),
        "the turn was not interrupted"
    );
    assert_eq!(queue_len(&rig.core, &peer), 1, "held for turn end");
    assert_eq!(
        entries(&rig.core, &peer).len(),
        1,
        "not in the transcript yet"
    );

    // The peer's agent asks for its inbox mid-turn (wait_threads): the row
    // moves into the transcript, tagged, without a second turn.
    let taken = rig.core.doc_host.take_thread_messages(&peer).unwrap();
    assert_eq!(taken.len(), 1);
    assert_eq!(taken[0].id, queued);
    assert_eq!(taken[0].from_chat_id.as_deref(), Some("lead"));
    assert_eq!(queue_len(&rig.core, &peer), 0);
    let now = entries(&rig.core, &peer);
    assert_eq!(now.len(), 2);
    assert_eq!(now[1].id, queued);
    assert_eq!(now[1].from_chat_id.as_deref(), Some("lead"));

    wait_for(|| idle(&rig.core, &peer), "the peer's turn to end").await;
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(
        rig.prompts.lock().unwrap().len(),
        1,
        "a taken message never runs again at turn end"
    );
    assert_eq!(entries(&rig.core, &peer).len(), 3);
    rig.core.shutdown().await;
}

#[tokio::test]
async fn a_held_message_runs_at_turn_end_when_nobody_took_it() {
    let rig = rig(Duration::from_millis(150));
    queue_human_run(&rig.core, "lead", "cmd-1", "m-1");
    wait_for(
        || rig.core.sessions.turn_in_flight("lead"),
        "the lead's own turn",
    )
    .await;
    rig.core
        .workspace
        .create_chat("peer", Some("space-tt"), None, None, None)
        .unwrap();
    rig.core
        .doc_host
        .queue_thread_message("lead", "peer", "done, 4 tests")
        .await
        .unwrap();
    assert_eq!(queue_len(&rig.core, "lead"), 1);

    wait_for(
        || rig.prompts.lock().unwrap().len() == 2 && idle(&rig.core, "lead"),
        "the held message to run as the next turn",
    )
    .await;
    let prompts = rig.prompts.lock().unwrap().clone();
    assert_eq!(prompts[0], "human prompt");
    assert!(
        prompts[1].starts_with("[Message from thread"),
        "{}",
        prompts[1]
    );
    assert!(
        prompts[1].contains("send_message_to_thread (threadId peer)"),
        "{}",
        prompts[1]
    );
    let all = entries(&rig.core, "lead");
    let from_peer: Vec<_> = all
        .iter()
        .filter(|e| e.from_chat_id.as_deref() == Some("peer"))
        .collect();
    assert_eq!(from_peer.len(), 1);
    assert_eq!(
        from_peer[0].parts,
        vec![zeron_doc::MessagePart::Text {
            id: "t0".into(),
            text: "done, 4 tests".into()
        }]
    );
    assert_eq!(queue_len(&rig.core, "lead"), 0);
    rig.core.shutdown().await;
}

#[tokio::test]
async fn a_message_to_an_idle_peer_with_history_opens_its_next_turn() {
    let rig = rig(Duration::from_millis(10));
    rig.core
        .workspace
        .create_chat("peer", Some("space-tt"), None, None, None)
        .unwrap();
    queue_human_run(&rig.core, "peer", "cmd-1", "m-1");
    wait_for(
        || entries(&rig.core, "peer").len() >= 2 && idle(&rig.core, "peer"),
        "the peer's own turn",
    )
    .await;

    rig.core
        .doc_host
        .queue_thread_message("peer", "lead", "now cover the refresh race")
        .await
        .unwrap();
    wait_for(
        || rig.prompts.lock().unwrap().len() == 2 && idle(&rig.core, "peer"),
        "the message to open the peer's next turn",
    )
    .await;
    let prompts = rig.prompts.lock().unwrap().clone();
    assert!(
        prompts[1].starts_with("[Message from thread lead."),
        "{}",
        prompts[1]
    );
    assert!(prompts[1].ends_with("now cover the refresh race"));
    let all = entries(&rig.core, "peer");
    assert_eq!(all.len(), 4, "{all:#?}");
    assert_eq!(all[2].role, MessageRole::User);
    assert_eq!(all[2].from_chat_id.as_deref(), Some("lead"));
    assert_eq!(
        all[0].from_chat_id, None,
        "the human's own prompt stays unstamped"
    );
    assert_eq!(queue_len(&rig.core, "peer"), 0);
    rig.core.shutdown().await;
}

#[tokio::test]
async fn the_creation_budget_refuses_the_ninth_open_thread() {
    let rig = rig(Duration::from_millis(1));
    for n in 0..8 {
        rig.core
            .doc_host
            .create_thread("lead", &format!("task {n}"), None, None, None, None)
            .await
            .unwrap_or_else(|e| panic!("thread {n}: {e}"));
    }
    let err = rig
        .core
        .doc_host
        .create_thread("lead", "one too many", None, None, None, None)
        .await
        .expect_err("ninth");
    assert!(err.to_string().contains("8 open threads"), "{err}");
    let created = rig
        .core
        .workspace
        .read_chats()
        .unwrap()
        .into_iter()
        .filter(|c| c.created_by_chat_id.as_deref() == Some("lead"))
        .count();
    assert_eq!(created, 8);
    rig.core.shutdown().await;
}

#[tokio::test]
async fn summary_rpc_reads_a_bounded_preview_without_a_transcript_watch() {
    let rig = rig(Duration::from_millis(1));
    let handle = rig.core.doc_host.open("lead").unwrap();
    handle
        .doc()
        .push_message(&zeron_doc::SessionMessageEntry {
            id: "long".into(),
            role: MessageRole::User,
            parts: vec![zeron_doc::MessagePart::Text {
                id: "body".into(),
                text: "界".repeat(100_000),
            }],
            created_at: 0,
            device_id: "dev-a".into(),
            status: None,
            continuation_of: None,
            from_chat_id: None,
            fork_import: false,
            answers_request_id: None,
            duration_ms: None,
        })
        .unwrap();
    let client = zeron_rpc::memory_client(rig.core.rpc_service());
    let summary = client
        .call(
            zeron_rpc::methods::GET_THREAD_SUMMARY,
            serde_json::json!({"chatId": "lead"}),
        )
        .await
        .unwrap();
    assert_eq!(summary["entryCount"], 1);
    assert_eq!(summary["lastRole"], "user");
    assert_eq!(summary["lastText"], format!("{}…", "界".repeat(120)));
    assert!(summary.to_string().len() < 500);
}
