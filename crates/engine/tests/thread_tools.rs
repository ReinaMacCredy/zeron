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
    attachments: Arc<Mutex<Vec<(String, Vec<String>)>>>,
    steers: Arc<Mutex<Vec<String>>>,
    steerable: bool,
    step_delay: Duration,
    reply: &'static str,
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
        self.attachments
            .lock()
            .unwrap()
            .push((request.prompt.clone(), request.attachments.clone()));
        let interrupt = controls.interrupt.clone();
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
        let reply = self.reply;
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
                AgentEvent::TextDelta { text: reply.into() },
                AgentEvent::Done {
                    status: DoneStatus::Completed,
                    result: None,
                    error: None,
                    session_id: Some("hs-rec".into()),
                },
            ];
            for event in script {
                tokio::select! {
                    () = tokio::time::sleep(delay) => {}
                    () = interrupt.cancelled() => {
                        let _ = tx
                            .send(Ok(AgentEvent::Done {
                                status: DoneStatus::Interrupted,
                                result: None,
                                error: None,
                                session_id: Some("hs-rec".into()),
                            }))
                            .await;
                        return;
                    }
                }
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
    attachments: Arc<Mutex<Vec<(String, Vec<String>)>>>,
    steers: Arc<Mutex<Vec<String>>>,
    _dir: tempfile::TempDir,
}

fn rig(step_delay: Duration) -> Rig {
    rig_with(step_delay, false)
}

fn rig_with(step_delay: Duration, steerable: bool) -> Rig {
    let prompts = Arc::new(Mutex::new(Vec::new()));
    let steers = Arc::new(Mutex::new(Vec::new()));
    let attachments = Arc::new(Mutex::new(Vec::new()));
    let rig = rig_on(Arc::new(RecordingHarness {
        prompts: prompts.clone(),
        attachments: attachments.clone(),
        steers: steers.clone(),
        steerable,
        step_delay,
        reply: "noted",
    }));
    Rig {
        prompts,
        steers,
        attachments,
        ..rig
    }
}

fn rig_on(harness: Arc<dyn Harness>) -> Rig {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("device-id"), "dev-a").unwrap();
    let registry = HarnessRegistry::new();
    registry.register(harness);
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
        prompts: Arc::default(),
        attachments: Arc::default(),
        steers: Arc::default(),
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

fn working(core: &EngineCore, chat_id: &str) -> bool {
    core.sessions
        .session_status(chat_id)
        .is_some_and(|s| s.status == SessionStatus::Working)
}

/// A person's own run on `chat_id`, the way a viewport queues it.
fn queue_human_run(core: &EngineCore, chat_id: &str, command_id: &str, message_id: &str) {
    queue_run(core, chat_id, command_id, message_id, "human prompt");
}

fn queue_run(core: &EngineCore, chat_id: &str, command_id: &str, message_id: &str, prompt: &str) {
    let handle = core.doc_host.open(chat_id).expect("open chat");
    handle
        .doc()
        .queue_command(&SessionCommandEntry {
            id: command_id.into(),
            payload: SessionCommandPayload::Run {
                request: RunRequest {
                    prompt: prompt.into(),
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
                    mcp: None,
                    disabled_tools: Vec::new(),
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
            &Default::default(),
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
        instructions: String::new(),
        profile_id: None,
        instance_id: None,
        disabled_tools: Vec::new(),
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
            &Default::default(),
            None,
            None,
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
        .create_thread(
            "picky",
            "task",
            None,
            None,
            None,
            None,
            &Default::default(),
            None,
            None,
        )
        .await
        .unwrap();
    assert_eq!(config_of(&copied), Some(creators.clone()));

    // Switching harness drops the inherited model (it names a model of the
    // old harness) but keeps the reasoning level, Zeron's own ladder.
    let switched = rig
        .core
        .doc_host
        .create_thread(
            "picky",
            "task",
            None,
            Some(HarnessId::Codex),
            None,
            None,
            &Default::default(),
            None,
            None,
        )
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
            &Default::default(),
            None,
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
        .create_thread(
            "lead",
            "task",
            None,
            None,
            Some("rec-3"),
            None,
            &Default::default(),
            None,
            None,
        )
        .await
        .unwrap();
    let config = config_of(&from_bare).expect("a config row");
    assert_eq!(config.harness, HarnessId::Mock);
    assert_eq!(config.model.as_deref(), Some("rec-3"));
    assert_eq!(config.reasoning, None);
    rig.core.shutdown().await;
}

#[tokio::test]
async fn create_thread_never_copies_the_creators_profile_and_applies_the_one_asked_for() {
    let rig = rig(Duration::from_millis(1));
    let options = |speed: &str| {
        serde_json::json!({ "speed": speed })
            .as_object()
            .cloned()
            .unwrap()
    };
    let creators = zeron_proto::ChatConfig {
        harness: HarnessId::Mock,
        model: Some("rec-1".into()),
        reasoning: Some(ReasoningLevel::High),
        model_options: options("fast"),
        sandbox: SandboxLevel::WorkspaceWrite,
        permission_mode: zeron_proto::PermissionMode::Auto,
        instructions: "Coordinate only.".into(),
        profile_id: Some("orchestrator".into()),
        instance_id: None,
        disabled_tools: vec!["mcp__xcode".into()],
    };
    rig.core
        .workspace
        .create_chat(
            "orchestrating",
            Some("space-tt"),
            None,
            Some(creators.clone()),
            None,
        )
        .unwrap();
    let config_of = |chat_id: &str| rig.core.workspace.chat(chat_id).unwrap().unwrap().config;
    let create = |profile: zeron_engine::doc_host::ProfileParams| {
        let core = &rig.core;
        async move {
            core.doc_host
                .create_thread(
                    "orchestrating",
                    "task",
                    None,
                    None,
                    None,
                    None,
                    &profile,
                    None,
                    None,
                )
                .await
        }
    };

    let plain = create(Default::default()).await.unwrap();
    assert_eq!(
        config_of(&plain),
        Some(zeron_proto::ChatConfig {
            instructions: String::new(),
            profile_id: None,
            ..creators.clone()
        }),
        "run settings, mode, options and tool set carry over; the profile does not"
    );

    let reviewer = create(zeron_engine::doc_host::ProfileParams {
        permission_mode: Some(zeron_proto::PermissionMode::Ask),
        model_options: Some(options("slow")),
        instructions: Some("Review only.".into()),
        profile_id: Some("reviewer".into()),
        disabled_tools: vec!["WebFetch".into()],
    })
    .await
    .unwrap();
    assert_eq!(
        config_of(&reviewer),
        Some(zeron_proto::ChatConfig {
            model_options: options("slow"),
            sandbox: zeron_proto::PermissionMode::Ask.sandbox(),
            permission_mode: zeron_proto::PermissionMode::Ask,
            instructions: "Review only.".into(),
            profile_id: Some("reviewer".into()),
            disabled_tools: vec!["mcp__xcode".into(), "WebFetch".into()],
            ..creators.clone()
        })
    );

    let wider = create(zeron_engine::doc_host::ProfileParams {
        permission_mode: Some(zeron_proto::PermissionMode::Full),
        profile_id: Some("builder".into()),
        ..Default::default()
    })
    .await
    .expect_err("a profile may not widen the creator's mode");
    assert!(
        wider.to_string().contains("permission_escalation_denied"),
        "{wider}"
    );
    rig.core.shutdown().await;
}

#[tokio::test]
async fn create_thread_can_open_in_another_project_on_the_device_or_an_existing_folder() {
    let rig = rig(Duration::from_millis(1));
    let other = tempfile::tempdir().unwrap();
    let other_path = other.path().to_str().unwrap().to_string();
    rig.core
        .workspace
        .create_space("space-other", "dev-a", &other_path, None, false)
        .unwrap();
    rig.core
        .workspace
        .create_space("space-far", "dev-b", "/tmp", None, false)
        .unwrap();
    let row = |chat_id: &str| rig.core.workspace.chat(chat_id).unwrap().unwrap();

    let moved = rig
        .core
        .doc_host
        .create_thread(
            "lead",
            "task",
            None,
            None,
            None,
            None,
            &Default::default(),
            Some("space-other"),
            None,
        )
        .await
        .unwrap();
    let moved = row(&moved);
    assert_eq!(moved.space_id.as_deref(), Some("space-other"));
    assert_eq!(moved.cwd.as_deref(), Some(other_path.as_str()));
    assert_eq!(moved.device_id, "dev-a");

    let nested = other.path().join("wt");
    std::fs::create_dir(&nested).unwrap();
    let pinned = rig
        .core
        .doc_host
        .create_thread(
            "lead",
            "task",
            None,
            None,
            None,
            None,
            &Default::default(),
            None,
            nested.to_str(),
        )
        .await
        .unwrap();
    let pinned = row(&pinned);
    assert_eq!(pinned.space_id.as_deref(), Some("space-tt"));
    assert_eq!(pinned.cwd.as_deref(), nested.to_str());
    assert_eq!(pinned.config, None, "no pick, no config row");

    for (space, cwd, expected) in [
        (Some("space-far"), None, "same device"),
        (Some("nope"), None, "no such project: nope"),
        (None, Some("/definitely/not/here"), "is not a directory"),
    ] {
        let err = rig
            .core
            .doc_host
            .create_thread(
                "lead",
                "task",
                None,
                None,
                None,
                None,
                &Default::default(),
                space,
                cwd,
            )
            .await
            .expect_err(expected);
        assert!(err.to_string().contains(expected), "{err}");
    }
    rig.core.shutdown().await;
}

#[tokio::test]
async fn self_and_cross_device_sends_are_refused_but_a_cross_project_send_is_delivered() {
    let rig = rig(Duration::from_millis(10));
    let err = rig
        .core
        .doc_host
        .queue_thread_message("lead", Some("lead"), "hi")
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
        .queue_thread_message("remote", Some("lead"), "hi")
        .await
        .expect_err("cross-device send");
    assert!(err.to_string().contains("same device"), "{err}");
    let err = rig
        .core
        .doc_host
        .queue_thread_message("nope", Some("lead"), "hi")
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
        .queue_thread_message("elsewhere", Some("lead"), "hi")
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
        .create_thread(
            "lead",
            "first task",
            None,
            None,
            None,
            None,
            &Default::default(),
            None,
            None,
        )
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
        .queue_thread_message(&peer, Some("lead"), "also cover the refresh race")
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
        .create_thread(
            "lead",
            "first task",
            None,
            None,
            None,
            None,
            &Default::default(),
            None,
            None,
        )
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
        .queue_thread_message(&peer, Some("lead"), "also cover the refresh race")
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
        .queue_thread_message("lead", Some("peer"), "done, 4 tests")
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
        .queue_thread_message("peer", Some("lead"), "now cover the refresh race")
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
            .create_thread(
                "lead",
                &format!("task {n}"),
                None,
                None,
                None,
                None,
                &Default::default(),
                None,
                None,
            )
            .await
            .unwrap_or_else(|e| panic!("thread {n}: {e}"));
    }
    let err = rig
        .core
        .doc_host
        .create_thread(
            "lead",
            "one too many",
            None,
            None,
            None,
            None,
            &Default::default(),
            None,
            None,
        )
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

fn delegate(task: &str) -> zeron_engine::doc_host::DelegateTaskParams {
    serde_json::from_value(serde_json::json!({ "ownerChatId": "lead", "task": task }))
        .expect("delegate params")
}

fn delegation(core: &EngineCore, chat_id: &str) -> zeron_proto::Delegation {
    core.workspace
        .chat(chat_id)
        .unwrap()
        .and_then(|chat| chat.delegation)
        .expect("a delegated chat")
}

fn queue_interrupt(core: &EngineCore, chat_id: &str) {
    core.doc_host
        .open(chat_id)
        .expect("open chat")
        .doc()
        .queue_command(&SessionCommandEntry {
            id: "stop-1".into(),
            payload: SessionCommandPayload::Interrupt {},
            issued_by: "viewer".into(),
            issued_at: chrono::Utc::now().timestamp_millis(),
            based_on: None::<CommandBasedOn>,
            expires_at: None,
            status: SessionCommandStatus::Pending,
            resolution: None,
            computer_generation: None,
        })
        .expect("queue interrupt");
}

#[tokio::test]
async fn a_delegated_child_is_owned_hidden_and_starts_from_the_task_alone() {
    let rig = rig(Duration::from_millis(5));
    let mut params = delegate("audit the login flow");
    params.role = Some("review".into());
    let view = rig
        .core
        .doc_host
        .delegate_task(params, Default::default())
        .await
        .expect("delegate");
    let row = rig.core.workspace.chat(&view.task_id).unwrap().unwrap();
    let d = row.delegation.expect("delegation");
    assert_eq!(d.delegated_by_chat_id, "lead");
    assert_eq!(d.depth, 1);
    assert_eq!(row.created_by_chat_id, None, "d29 stays grouping-only");
    assert_eq!(row.title.as_deref(), Some("audit the login flow"));
    wait_for(
        || !rig.prompts.lock().unwrap().is_empty(),
        "the child's first turn",
    )
    .await;
    assert_eq!(
        rig.prompts.lock().unwrap()[0],
        "Act as the review sub-agent for this task.\n\naudit the login flow",
        "no owner history and no peer framing"
    );
    rig.core.shutdown().await;
}

#[tokio::test]
async fn a_delegated_child_takes_the_model_options_it_was_given() {
    let rig = rig(Duration::from_millis(5));
    let mut params = delegate("try the fast tier");
    params.profile.model_options = Some(
        serde_json::json!({ "speed": "fast" })
            .as_object()
            .cloned()
            .unwrap(),
    );
    params.profile.instructions = Some("Review only.".into());
    let view = rig
        .core
        .doc_host
        .delegate_task(params, Default::default())
        .await
        .expect("delegate");
    let row = rig.core.workspace.chat(&view.task_id).unwrap().unwrap();
    let config = row.config.expect("child config");
    assert_eq!(config.model_options["speed"], "fast");
    assert_eq!(config.instructions, "Review only.");
    rig.core.shutdown().await;
}

#[tokio::test]
async fn a_delegated_child_names_its_own_profile_never_the_owners() {
    let rig = rig(Duration::from_millis(5));
    rig.core
        .workspace
        .set_chat_config(
            "lead",
            &zeron_proto::ChatConfig {
                harness: HarnessId::Mock,
                model: None,
                reasoning: None,
                model_options: Default::default(),
                sandbox: SandboxLevel::WorkspaceWrite,
                permission_mode: Default::default(),
                instructions: "Plan only.".into(),
                profile_id: Some("planner".into()),
                instance_id: None,
                disabled_tools: Vec::new(),
            },
        )
        .unwrap();
    let child_profile = |task: &str, profile: Option<&str>| {
        let mut params = delegate(task);
        params.profile.profile_id = profile.map(str::to_string);
        params.profile.instructions = profile.map(|_| "Review only.".to_string());
        let core = &rig.core;
        async move {
            let view = core
                .doc_host
                .delegate_task(params, Default::default())
                .await
                .expect("delegate");
            let row = core.workspace.chat(&view.task_id).unwrap().unwrap();
            row.config.expect("child config").profile_id
        }
    };
    assert_eq!(
        child_profile("review it", Some("reviewer"))
            .await
            .as_deref(),
        Some("reviewer")
    );
    assert_eq!(child_profile("just do it", None).await, None);
    rig.core.shutdown().await;
}

#[tokio::test]
async fn a_finished_child_wakes_its_idle_owner_once_and_task_status_acknowledges() {
    let rig = rig(Duration::from_millis(5));
    let view = rig
        .core
        .doc_host
        .delegate_task(delegate("count the tests"), Default::default())
        .await
        .expect("delegate");
    let child = view.task_id.clone();
    let wake = format!(
        "Delegated task {child} (\"count the tests\") completed.\n\n<agent-response>\nnoted\n</agent-response>\n\nUse task_status with taskId {child} to read the result."
    );
    wait_for(
        || {
            entries(&rig.core, "lead")
                .iter()
                .any(|e| e.from_chat_id.as_deref() == Some(child.as_str()))
        },
        "the owner's wake",
    )
    .await;
    wait_for(|| idle(&rig.core, "lead"), "the owner's wake turn").await;
    let d = delegation(&rig.core, &child);
    assert_eq!(d.status, zeron_proto::DelegationStatus::Completed);
    assert_eq!(d.delivery, Some(zeron_proto::DelegationDelivery::Delivered));
    let wakes = rig
        .prompts
        .lock()
        .unwrap()
        .iter()
        .filter(|p| **p == wake)
        .count();
    assert_eq!(wakes, 1, "one wake, carrying the child's reply");

    let read = rig
        .core
        .doc_host
        .task_status("lead", &child, true)
        .await
        .unwrap();
    assert_eq!(read.work_state, "result_available");
    assert_eq!(read.status, "completed");
    assert_eq!(read.summary.as_deref(), Some("noted"));
    assert_eq!(
        delegation(&rig.core, &child).delivery,
        Some(zeron_proto::DelegationDelivery::Acknowledged)
    );
    assert!(
        rig.core
            .doc_host
            .task_status("someone-else", &child, true)
            .await
            .is_err(),
        "only the owner reads its task"
    );
    rig.core.shutdown().await;
}

#[tokio::test]
async fn an_owner_stop_disposes_the_wake_but_leaves_the_child_running() {
    let rig = rig(Duration::from_millis(150));
    queue_human_run(&rig.core, "lead", "cmd-1", "msg-1");
    wait_for(
        || {
            rig.core
                .sessions
                .session_status("lead")
                .is_some_and(|s| s.status == SessionStatus::Working)
        },
        "the owner's turn",
    )
    .await;
    let child = rig
        .core
        .doc_host
        .delegate_task(delegate("long job"), Default::default())
        .await
        .unwrap()
        .task_id;
    queue_interrupt(&rig.core, "lead");
    wait_for(
        || {
            delegation(&rig.core, &child).delivery
                == Some(zeron_proto::DelegationDelivery::Disposed)
        },
        "the stop to dispose the wake",
    )
    .await;
    assert!(
        delegation(&rig.core, &child).is_running(),
        "the child keeps working"
    );
    wait_for(
        || !delegation(&rig.core, &child).is_running(),
        "the child to finish",
    )
    .await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(
        !entries(&rig.core, "lead")
            .iter()
            .any(|e| e.from_chat_id.as_deref() == Some(child.as_str())),
        "no wake after a stop"
    );
    rig.core.shutdown().await;
}

#[tokio::test]
async fn an_archived_owner_gets_no_wake_and_stays_archived() {
    let rig = rig(Duration::from_millis(30));
    let child = rig
        .core
        .doc_host
        .delegate_task(delegate("quiet job"), Default::default())
        .await
        .unwrap()
        .task_id;
    rig.core.workspace.set_chat_archived("lead", true).unwrap();
    wait_for(
        || !delegation(&rig.core, &child).is_running(),
        "the child to finish",
    )
    .await;
    assert_eq!(
        delegation(&rig.core, &child).delivery,
        Some(zeron_proto::DelegationDelivery::Disposed)
    );
    assert!(rig.core.workspace.chat("lead").unwrap().unwrap().archived);
    assert_eq!(queue_len(&rig.core, "lead"), 0);
    rig.core.shutdown().await;
}

#[tokio::test]
async fn a_never_wake_child_finishes_without_waking_its_idle_owner() {
    let rig = rig(Duration::from_millis(5));
    let mut params = delegate("review the diff");
    params.wake = Some(zeron_proto::DelegationWake::Never);
    let child = rig
        .core
        .doc_host
        .delegate_task(params, Default::default())
        .await
        .unwrap()
        .task_id;
    wait_for(
        || !delegation(&rig.core, &child).is_running(),
        "the child to finish",
    )
    .await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(
        delegation(&rig.core, &child).delivery,
        Some(zeron_proto::DelegationDelivery::Disposed)
    );
    assert_eq!(queue_len(&rig.core, "lead"), 0);
    assert!(
        !entries(&rig.core, "lead")
            .iter()
            .any(|e| e.from_chat_id.as_deref() == Some(child.as_str())),
        "no wake for a never-wake child"
    );
    let read = rig
        .core
        .doc_host
        .task_status("lead", &child, false)
        .await
        .unwrap();
    assert_eq!(read.summary.as_deref(), Some("noted"));
    rig.core.shutdown().await;
}

const REVIEW_REPLY: &str = "Looked it over.\n\n```zeron-review\n[{\"path\":\"src/a.rs\",\"line\":3,\"severity\":\"bug\",\"title\":\"Unwrap panics\"},{\"path\":\"src/b.rs\",\"line\":9,\"severity\":\"risk\",\"title\":\"Leaks a handle\"},{\"path\":\"src/c.rs\",\"line\":1,\"severity\":\"nit\",\"title\":\"Typo\"}]\n```\n";

fn review_rig() -> Rig {
    let prompts = Arc::new(Mutex::new(Vec::new()));
    let rig = rig_on(Arc::new(RecordingHarness {
        prompts: prompts.clone(),
        attachments: Arc::default(),
        steers: Arc::default(),
        steerable: false,
        step_delay: Duration::from_millis(5),
        reply: REVIEW_REPLY,
    }));
    Rig { prompts, ..rig }
}

async fn finished_child(rig: &Rig, request: &str, wake: zeron_proto::DelegationWake) -> String {
    let mut params = delegate("review the diff");
    params.client_request_id = Some(request.into());
    params.wake = Some(wake);
    let child = rig
        .core
        .doc_host
        .delegate_task(params, Default::default())
        .await
        .unwrap()
        .task_id;
    wait_for(
        || !delegation(&rig.core, &child).is_running(),
        "the child to finish",
    )
    .await;
    child
}

#[tokio::test]
async fn resolving_findings_checks_the_owner_the_review_the_ids_and_the_notes() {
    let rig = review_rig();
    let review = finished_child(
        &rig,
        "zeron-review:uncommitted:n1",
        zeron_proto::DelegationWake::Never,
    )
    .await;
    let plain = finished_child(&rig, "plain-1", zeron_proto::DelegationWake::Never).await;
    let client = zeron_rpc::memory_client(rig.core.rpc_service());
    let resolve = |owner: &str, task: &str, findings: serde_json::Value| {
        let client = &client;
        let params =
            serde_json::json!({ "ownerChatId": owner, "taskId": task, "findings": findings });
        async move {
            client
                .call(zeron_rpc::methods::RESOLVE_REVIEW_FINDINGS, params)
                .await
        }
    };
    let fixed = serde_json::json!([{ "id": "F1", "outcome": "fixed", "commit": "abc1234" }]);
    let refused = |result: Result<serde_json::Value, _>| match result {
        Ok(value) => panic!("accepted: {value}"),
        Err(err) => format!("{err:?}"),
    };
    assert!(refused(resolve("stranger", &review, fixed.clone()).await).contains("task_not_found"));
    assert!(refused(resolve("lead", &plain, fixed.clone()).await).contains("not_a_review"));
    assert!(
        refused(
            resolve(
                "lead",
                &review,
                serde_json::json!([{ "id": "F9", "outcome": "fixed" }])
            )
            .await
        )
        .contains("unknown_finding")
    );
    for outcome in ["skipped", "no_change"] {
        assert!(
            refused(
                resolve(
                    "lead",
                    &review,
                    serde_json::json!([
                        { "id": "F1", "outcome": "fixed" },
                        { "id": "F2", "outcome": outcome, "note": "  " }
                    ])
                )
                .await
            )
            .contains("note_required")
        );
    }
    for commit in ["abc123é", "not-hex", "abc"] {
        assert!(
            refused(
                resolve(
                    "lead",
                    &review,
                    serde_json::json!([{ "id": "F1", "outcome": "fixed", "commit": commit }])
                )
                .await
            )
            .contains("invalid_commit")
        );
    }
    assert!(
        delegation(&rig.core, &review).finding_outcomes.is_empty(),
        "a refused call writes nothing"
    );

    let reply = resolve(
        "lead",
        &review,
        serde_json::json!([
            { "id": "F1", "outcome": "fixed", "commit": "abc1234" },
            { "id": "F2", "outcome": "skipped", "note": "Owned by the caller" }
        ]),
    )
    .await
    .unwrap();
    assert_eq!(
        reply,
        serde_json::json!({ "resolved": ["F1", "F2"], "open": ["F3"] })
    );
    let reply = resolve(
        "lead",
        &review,
        serde_json::json!([{ "id": "F2", "outcome": "no_change", "note": "Already closed" }]),
    )
    .await
    .unwrap();
    assert_eq!(reply["open"], serde_json::json!(["F3"]));
    let outcomes = delegation(&rig.core, &review).finding_outcomes;
    assert_eq!(
        outcomes["F2"].outcome,
        zeron_proto::FindingOutcomeKind::NoChange,
        "a later call replaces the outcome"
    );
    assert_eq!(outcomes["F1"].commit.as_deref(), Some("abc1234"));

    let view = serde_json::to_value(
        rig.core
            .doc_host
            .task_status("lead", &review, false)
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(
        view["findings"],
        serde_json::json!([
            { "id": "F1", "severity": "bug", "title": "Unwrap panics", "location": "src/a.rs:3", "outcome": "fixed", "commit": "abc1234" },
            { "id": "F2", "severity": "risk", "title": "Leaks a handle", "location": "src/b.rs:9", "outcome": "no_change", "note": "Already closed" },
            { "id": "F3", "severity": "nit", "title": "Typo", "location": "src/c.rs:1" }
        ])
    );
    assert!(view["next"].as_str().unwrap().contains("resolve_findings"));
    resolve(
        "lead",
        &review,
        serde_json::json!([{ "id": "F3", "outcome": "skipped", "note": "x".repeat(2_500) }]),
    )
    .await
    .unwrap();
    let note = delegation(&rig.core, &review).finding_outcomes["F3"]
        .note
        .clone()
        .unwrap();
    assert_eq!(note.chars().count(), 2_000);
    let plain_view = serde_json::to_value(
        rig.core
            .doc_host
            .task_status("lead", &plain, false)
            .await
            .unwrap(),
    )
    .unwrap();
    assert!(plain_view.get("findings").is_none() && plain_view.get("next").is_none());
    rig.core.shutdown().await;
}

fn reminders(rig: &Rig) -> Vec<String> {
    prompts_with(rig, "call resolve_findings")
}

#[tokio::test]
async fn a_handed_finding_is_named_to_the_owner_in_one_reminder() {
    let rig = review_rig();
    let review = finished_child(
        &rig,
        "zeron-review:uncommitted:n1",
        zeron_proto::DelegationWake::Never,
    )
    .await;
    let client = zeron_rpc::memory_client(rig.core.rpc_service());
    client
        .call(
            zeron_rpc::methods::RESOLVE_REVIEW_FINDINGS,
            serde_json::json!({ "ownerChatId": "lead", "taskId": review, "findings": [{ "id": "F1", "outcome": "fixed" }] }),
        )
        .await
        .unwrap();
    assert!(
        client
            .call(
                zeron_rpc::methods::HAND_REVIEW_FINDINGS,
                serde_json::json!({ "ownerChatId": "lead", "taskId": review, "ids": ["F9"] }),
            )
            .await
            .is_err()
    );
    let handed = client
        .call(
            zeron_rpc::methods::HAND_REVIEW_FINDINGS,
            serde_json::json!({ "ownerChatId": "lead", "taskId": review, "ids": ["F1", "F3"] }),
        )
        .await
        .unwrap();
    assert_eq!(handed["handed"], serde_json::json!(["F1", "F3"]));
    assert!(
        reminders(&rig).is_empty(),
        "nothing reminds before a prompt"
    );

    queue_human_run(&rig.core, "lead", "c1", "m1");
    wait_for(
        || prompts_with(&rig, "human prompt").len() == 1,
        "the first run",
    )
    .await;
    wait_for(|| idle(&rig.core, "lead"), "the first run to end").await;
    queue_human_run(&rig.core, "lead", "c2", "m2");
    wait_for(
        || prompts_with(&rig, "human prompt").len() == 2,
        "the second run",
    )
    .await;
    let named = reminders(&rig);
    assert_eq!(named.len(), 1, "one reminder: {named:?}");
    assert!(named[0].contains(&format!(
        "Review task {review} has findings for you to resolve: F3."
    )));
    assert_eq!(
        delegation(&rig.core, &review).findings_announced,
        std::collections::BTreeSet::from(["F3".to_string()])
    );
    rig.core.shutdown().await;
}

#[tokio::test]
async fn an_agent_started_review_names_its_findings_on_the_wake() {
    let rig = review_rig();
    let review = finished_child(
        &rig,
        "zeron-review:uncommitted:n2",
        zeron_proto::DelegationWake::Always,
    )
    .await;
    wait_for(|| !reminders(&rig).is_empty(), "the wake with its reminder").await;
    wait_for(|| idle(&rig.core, "lead"), "the wake turn").await;
    let named = reminders(&rig);
    assert!(named[0].starts_with(&format!(
        "Delegated task {review} (\"review the diff\") completed.\n\n<agent-response>\n{}\n</agent-response>",
        REVIEW_REPLY.trim()
    )));
    assert!(named[0].contains("to resolve: F1, F2, F3."));
    queue_human_run(&rig.core, "lead", "c1", "m1");
    wait_for(
        || prompts_with(&rig, "human prompt").len() == 1,
        "the next run",
    )
    .await;
    assert_eq!(reminders(&rig).len(), 1, "named once");
    rig.core.shutdown().await;
}

#[tokio::test]
async fn delegation_limits_and_retries_follow_the_settings() {
    let rig = rig(Duration::from_millis(200));
    let limits = zeron_engine::registry::DelegationSettings {
        max_depth: 1,
        max_running_children: 1,
    };
    let mut first = delegate("one");
    first.client_request_id = Some("k1".into());
    let child = rig
        .core
        .doc_host
        .delegate_task(first.clone(), limits)
        .await
        .unwrap()
        .task_id;
    let again = rig
        .core
        .doc_host
        .delegate_task(first, limits)
        .await
        .unwrap()
        .task_id;
    assert_eq!(
        again, child,
        "a retried clientRequestId returns the same task"
    );
    let err = rig
        .core
        .doc_host
        .delegate_task(delegate("two"), limits)
        .await
        .unwrap_err()
        .to_string();
    assert!(err.starts_with("child_limit"), "{err}");
    let mut nested = delegate("grandchild");
    nested.owner_chat_id = child.clone();
    let err = rig
        .core
        .doc_host
        .delegate_task(nested, limits)
        .await
        .unwrap_err()
        .to_string();
    assert!(err.starts_with("depth_limit"), "{err}");
    let cancelled = rig.core.doc_host.task_cancel("lead", &child).await.unwrap();
    assert_eq!(cancelled, "cancel_requested");
    wait_for(
        || delegation(&rig.core, &child).status == zeron_proto::DelegationStatus::Cancelled,
        "the cancelled child to settle",
    )
    .await;
    rig.core.shutdown().await;
}

struct DelegatingHarness {
    release: Arc<tokio::sync::Notify>,
    steerable: bool,
    steers: Arc<Mutex<Vec<String>>>,
}

#[async_trait]
impl Harness for DelegatingHarness {
    fn id(&self) -> HarnessId {
        HarnessId::Mock
    }
    fn display_name(&self) -> &str {
        "Delegating"
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
        let (tx, rx) = tokio::sync::mpsc::channel::<Result<AgentEvent, HarnessError>>(8);
        let steers = self.steers.clone();
        let confirm = tx.clone();
        tokio::spawn(async move {
            while let Some(message) = controls.steering.recv().await {
                steers.lock().unwrap().push(message.prompt);
                let _ = confirm
                    .send(Ok(AgentEvent::Steered {
                        assistant_message_id: Some("a-del".into()),
                        next_assistant_message_id: Some("a-del-2".into()),
                    }))
                    .await;
            }
        });
        let owner = request.prompt == "human prompt";
        let release = self.release.clone();
        tokio::spawn(async move {
            let started = AgentEvent::SessionStarted {
                harness: HarnessId::Mock,
                model: "rec-1".into(),
                tools: vec![],
                cwd: "/tmp".into(),
                session_id: "hs-del".into(),
                assistant_message_id: "a-del".into(),
            };
            let _ = tx.send(Ok(started)).await;
            if owner {
                let calls = [
                    (
                        "tc-1",
                        "delegate_task",
                        serde_json::json!({ "task": "probe the cache" }),
                    ),
                    (
                        "tc-2",
                        "create_thread",
                        serde_json::json!({ "prompt": "draft the notes" }),
                    ),
                ];
                for (id, tool, input) in calls.clone() {
                    let call = zeron_proto::ToolCall::Mcp {
                        server: "zeron".into(),
                        tool: tool.into(),
                        input: Some(input),
                    };
                    let _ = tx
                        .send(Ok(AgentEvent::ToolCall {
                            id: id.into(),
                            call,
                        }))
                        .await;
                }
                release.notified().await;
                for (id, _, _) in calls {
                    let _ = tx
                        .send(Ok(AgentEvent::ToolResult {
                            id: id.into(),
                            is_error: false,
                            output: None,
                            diff: None,
                        }))
                        .await;
                }
            }
            let _ = tx
                .send(Ok(AgentEvent::TextDelta { text: "ok".into() }))
                .await;
            let _ = tx
                .send(Ok(AgentEvent::Done {
                    status: DoneStatus::Completed,
                    result: None,
                    error: None,
                    session_id: Some("hs-del".into()),
                }))
                .await;
        });
        Ok(futures::stream::unfold(rx, |mut rx| async move {
            rx.recv().await.map(|event| (event, rx))
        })
        .boxed())
    }
}

#[tokio::test]
async fn the_delegate_task_and_create_thread_chips_link_to_the_threads_they_opened() {
    let release = Arc::new(tokio::sync::Notify::new());
    let rig = rig_on(Arc::new(DelegatingHarness {
        release: release.clone(),
        steerable: false,
        steers: Arc::default(),
    }));
    let core = &rig.core;
    queue_human_run(core, "lead", "cmd-1", "msg-1");
    wait_for(|| working(core, "lead"), "the owner's turn").await;
    let child = core
        .doc_host
        .delegate_task(delegate(" probe the cache "), Default::default())
        .await
        .unwrap()
        .task_id;
    let peer = core
        .doc_host
        .create_thread(
            "lead",
            " draft the notes ",
            None,
            None,
            None,
            None,
            &Default::default(),
            None,
            None,
        )
        .await
        .unwrap();
    release.notify_one();
    let chip_ref = |tool: &str| {
        entries(&core, "lead").iter().find_map(|entry| {
            entry.parts.iter().find_map(|part| match part {
                zeron_doc::MessagePart::Tool {
                    id, subagent_ref, ..
                } if id == tool => Some(subagent_ref.clone()),
                _ => None,
            })
        })
    };
    wait_for(
        || chip_ref("tc-1").flatten().is_some() && chip_ref("tc-2").flatten().is_some(),
        "the chip bindings",
    )
    .await;
    assert_eq!(chip_ref("tc-1").flatten().as_deref(), Some(child.as_str()));
    assert_eq!(chip_ref("tc-2").flatten().as_deref(), Some(peer.as_str()));
    core.shutdown().await;
}

#[tokio::test]
async fn a_child_that_finishes_mid_turn_steers_its_wake_into_the_owners_turn() {
    let release = Arc::new(tokio::sync::Notify::new());
    let steers: Arc<Mutex<Vec<String>>> = Arc::default();
    let rig = rig_on(Arc::new(DelegatingHarness {
        release: release.clone(),
        steerable: true,
        steers: steers.clone(),
    }));
    let core = &rig.core;
    queue_human_run(core, "lead", "cmd-1", "msg-1");
    wait_for(|| working(core, "lead"), "the owner's turn").await;
    let child = core
        .doc_host
        .delegate_task(delegate("probe the cache"), Default::default())
        .await
        .unwrap()
        .task_id;
    wait_for(
        || {
            steers
                .lock()
                .unwrap()
                .iter()
                .any(|steer| {
                    steer.contains(&format!(
                        "Delegated task {child} (\"probe the cache\") completed.\n\n<agent-response>\nok\n</agent-response>"
                    ))
                })
        },
        "the wake steered into the live turn",
    )
    .await;
    assert_eq!(queue_len(&core, "lead"), 0, "steered, not queued");
    assert_eq!(
        delegation(&core, &child).delivery,
        Some(zeron_proto::DelegationDelivery::Delivered)
    );
    release.notify_one();
    core.shutdown().await;
}

#[tokio::test]
async fn children_finishing_while_the_wake_waits_share_one_wake() {
    let release = Arc::new(tokio::sync::Notify::new());
    let rig = rig_on(Arc::new(DelegatingHarness {
        release: release.clone(),
        steerable: false,
        steers: Arc::default(),
    }));
    let core = &rig.core;
    queue_human_run(core, "lead", "cmd-1", "msg-1");
    wait_for(|| working(core, "lead"), "the owner's turn").await;
    let mut children = Vec::new();
    for task in ["first", "second"] {
        children.push(
            core.doc_host
                .delegate_task(delegate(task), Default::default())
                .await
                .unwrap()
                .task_id,
        );
    }
    let queue = || {
        core.doc_host
            .open("lead")
            .unwrap()
            .doc()
            .read_queue()
            .unwrap()
    };
    wait_for(
        || queue().iter().any(|row| row.delegated_tasks.len() == 2),
        "one wake naming both children",
    )
    .await;
    let wakes: Vec<_> = queue()
        .into_iter()
        .filter(|row| !row.delegated_tasks.is_empty())
        .collect();
    assert_eq!(wakes.len(), 1);
    assert_eq!(wakes[0].delegated_tasks, children);
    for (child, task) in children.iter().zip(["first", "second"]) {
        assert!(
            wakes[0].text.contains(&format!(
                "Delegated task {child} (\"{task}\") completed.\n\n<agent-response>\nok\n</agent-response>"
            )),
            "{}",
            wakes[0].text
        );
    }
    assert!(
        wakes[0]
            .text
            .ends_with("Use task_status with each taskId to read the results.")
    );
    release.notify_one();
    let delivered = |children: &[String]| {
        children.iter().all(|child| {
            delegation(&core, child).delivery == Some(zeron_proto::DelegationDelivery::Delivered)
        })
    };
    wait_for(|| delivered(&children), "the shared wake delivered").await;
    wait_for(|| idle(core, "lead"), "the owner's wake turn").await;

    let mut pair = Vec::new();
    for task in ["third", "fourth"] {
        pair.push(
            core.doc_host
                .delegate_task(delegate(task), Default::default())
                .await
                .unwrap()
                .task_id,
        );
    }
    wait_for(|| delivered(&pair), "the idle owner's wake").await;
    wait_for(|| idle(core, "lead"), "the idle owner's wake turn").await;
    let naming_third: Vec<String> = entries(&core, "lead")
        .into_iter()
        .filter(|entry| entry.role == MessageRole::User)
        .filter_map(|entry| match entry.parts.first() {
            Some(zeron_doc::MessagePart::Text { text, .. }) => Some(text.clone()),
            _ => None,
        })
        .filter(|text| text.contains(&pair[0]))
        .collect();
    assert_eq!(
        naming_third.len(),
        1,
        "children finishing close together wake an idle owner once: {naming_third:?}"
    );
    assert!(naming_third[0].contains(&format!(
        "Delegated task {} (\"fourth\") completed.",
        pair[1]
    )));
    core.shutdown().await;
}

#[tokio::test]
async fn a_child_that_cannot_start_fails_the_delegation() {
    let rig = rig(Duration::from_millis(5));
    let mut params = delegate("needs codex");
    params.harness = Some(HarnessId::Codex);
    let err = rig
        .core
        .doc_host
        .delegate_task(params, Default::default())
        .await
        .unwrap_err()
        .to_string();
    assert!(err.starts_with("orchestration_error"), "{err}");
    let failed: Vec<_> = rig
        .core
        .workspace
        .read_chats()
        .unwrap()
        .into_iter()
        .filter_map(|chat| chat.delegation)
        .collect();
    assert_eq!(failed.len(), 1);
    assert_eq!(failed[0].status, zeron_proto::DelegationStatus::Failed);
    assert_eq!(
        failed[0].delivery,
        Some(zeron_proto::DelegationDelivery::Disposed)
    );
    rig.core.shutdown().await;
}

#[tokio::test]
async fn a_cancelled_child_settles_while_its_own_child_still_runs() {
    let rig = rig(Duration::from_millis(150));
    let child = rig
        .core
        .doc_host
        .delegate_task(delegate("parent job"), Default::default())
        .await
        .unwrap()
        .task_id;
    let mut nested = delegate("grandchild job");
    nested.owner_chat_id = child.clone();
    let grandchild = rig
        .core
        .doc_host
        .delegate_task(nested, Default::default())
        .await
        .unwrap()
        .task_id;
    rig.core.doc_host.task_cancel("lead", &child).await.unwrap();
    wait_for(
        || delegation(&rig.core, &child).status == zeron_proto::DelegationStatus::Cancelled,
        "the cancelled child to settle",
    )
    .await;
    assert!(
        delegation(&rig.core, &grandchild).is_running(),
        "cancel does not reach below the child"
    );
    wait_for(
        || !delegation(&rig.core, &grandchild).is_running(),
        "the grandchild to finish",
    )
    .await;
    assert_eq!(
        delegation(&rig.core, &grandchild).delivery,
        Some(zeron_proto::DelegationDelivery::Disposed),
        "no wake into a cancelled child"
    );
    rig.core.shutdown().await;
}

/// Starts streaming and never finishes, so the engine can die mid-run.
struct StallingHarness;

#[async_trait]
impl Harness for StallingHarness {
    fn id(&self) -> HarnessId {
        HarnessId::Mock
    }
    fn display_name(&self) -> &str {
        "Stalling"
    }
    fn supports_steering(&self) -> bool {
        false
    }
    fn steering_mode(&self) -> SteeringMode {
        SteeringMode::TurnBoundary
    }
    fn reasoning_levels(&self) -> &[ReasoningLevel] {
        &[]
    }
    async fn models(&self) -> Result<Vec<Model>, HarnessError> {
        Ok(vec![])
    }
    async fn run(
        &self,
        _request: RunRequest,
        _controls: RunControls,
    ) -> Result<BoxStream<'static, Result<AgentEvent, HarnessError>>, HarnessError> {
        Ok(futures::stream::iter([
            Ok(AgentEvent::SessionStarted {
                harness: HarnessId::Mock,
                model: "stall-1".into(),
                tools: vec![],
                cwd: "/tmp".into(),
                session_id: "hs-stall".into(),
                assistant_message_id: "a-stall".into(),
            }),
            Ok(AgentEvent::TextDelta {
                text: "working".into(),
            }),
        ])
        .chain(futures::stream::pending())
        .boxed())
    }
}

#[test]
fn a_child_revived_after_a_crash_settles_from_its_revived_run() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("device-id"), "dev-a").unwrap();

    let crashed = tokio::runtime::Runtime::new().unwrap();
    let (core, child) = crashed.block_on(async {
        let registry = HarnessRegistry::new();
        registry.register(Arc::new(StallingHarness));
        let core = EngineCore::assemble(dir.path(), Arc::new(registry), HarnessId::Mock, None)
            .expect("engine core assembles");
        core.workspace
            .create_space("space-tt", "dev-a", "/tmp", None, false)
            .expect("space");
        core.workspace
            .create_chat("lead", Some("space-tt"), None, None, None)
            .expect("lead chat");
        let child = core
            .doc_host
            .delegate_task(delegate("survive the crash"), Default::default())
            .await
            .expect("delegate")
            .task_id;
        wait_for(
            || {
                entries(&core, &child).iter().any(|e| {
                    e.role == zeron_doc::MessageRole::Assistant
                        && e.status == Some(zeron_doc::MessageStatus::Streaming)
                })
            },
            "the child mid-run",
        )
        .await;
        core.doc_host.flush_all();
        core.workspace.flush();
        (core, child)
    });
    crashed.shutdown_background();
    drop(core);

    let rebooted = tokio::runtime::Runtime::new().unwrap();
    rebooted.block_on(async {
        let prompts = Arc::new(Mutex::new(Vec::new()));
        let registry = HarnessRegistry::new();
        registry.register(Arc::new(RecordingHarness {
            prompts: prompts.clone(),
            attachments: Arc::default(),
            steers: Arc::new(Mutex::new(Vec::new())),
            steerable: false,
            step_delay: Duration::from_millis(50),
            reply: "noted",
        }));
        let core = EngineCore::assemble(dir.path(), Arc::new(registry), HarnessId::Mock, None)
            .expect("engine core reassembles");
        wait_for(
            || !delegation(&core, &child).is_running(),
            "the revived child to settle",
        )
        .await;
        assert!(
            prompts
                .lock()
                .unwrap()
                .iter()
                .any(|p| p.ends_with("survive the crash")),
            "the crashed child run was revived"
        );
        let d = delegation(&core, &child);
        assert_eq!(d.status, zeron_proto::DelegationStatus::Completed);
        assert_eq!(d.summary.as_deref(), Some("noted"));
        core.shutdown().await;
    });
}

/// Opens an `html_render` call, hands the test its run secret, and settles
/// the call once the test has published through the engine.
struct PublishingHarness {
    secret: Arc<Mutex<Option<String>>>,
    release: Arc<tokio::sync::Notify>,
}

#[async_trait]
impl Harness for PublishingHarness {
    fn id(&self) -> HarnessId {
        HarnessId::Mock
    }
    fn display_name(&self) -> &str {
        "Publishing"
    }
    fn supports_steering(&self) -> bool {
        false
    }
    fn steering_mode(&self) -> SteeringMode {
        SteeringMode::TurnBoundary
    }
    fn reasoning_levels(&self) -> &[ReasoningLevel] {
        &[]
    }
    async fn models(&self) -> Result<Vec<Model>, HarnessError> {
        Ok(vec![])
    }
    async fn run(
        &self,
        _request: RunRequest,
        controls: RunControls,
    ) -> Result<BoxStream<'static, Result<AgentEvent, HarnessError>>, HarnessError> {
        *self.secret.lock().unwrap() = controls.tool_server.as_ref().and_then(|server| {
            server
                .env
                .iter()
                .find(|(name, _)| name == zeron_rpc::COMPUTER_RUN_SECRET_ENV)
                .map(|(_, value)| value.clone())
        });
        let (tx, rx) = tokio::sync::mpsc::channel::<Result<AgentEvent, HarnessError>>(8);
        let release = self.release.clone();
        tokio::spawn(async move {
            let events = [
                AgentEvent::SessionStarted {
                    harness: HarnessId::Mock,
                    model: "rec-1".into(),
                    tools: vec![],
                    cwd: "/tmp".into(),
                    session_id: "hs-html".into(),
                    assistant_message_id: "a-html".into(),
                },
                AgentEvent::ToolCall {
                    id: "tc-html".into(),
                    call: zeron_proto::ToolCall::Mcp {
                        server: "zeron".into(),
                        tool: "html_render".into(),
                        input: Some(serde_json::json!({
                            "html": "<p>chart</p>", "title": "Chart", "height": 420,
                        })),
                    },
                },
            ];
            for event in events {
                let _ = tx.send(Ok(event)).await;
            }
            release.notified().await;
            for event in [
                AgentEvent::ToolResult {
                    id: "tc-html".into(),
                    is_error: false,
                    output: None,
                    diff: None,
                },
                AgentEvent::TextDelta {
                    text: "Bars are tokens per turn.".into(),
                },
                AgentEvent::Done {
                    status: DoneStatus::Completed,
                    result: None,
                    error: None,
                    session_id: Some("hs-html".into()),
                },
            ] {
                let _ = tx.send(Ok(event)).await;
            }
        });
        Ok(futures::stream::unfold(rx, |mut rx| async move {
            rx.recv().await.map(|event| (event, rx))
        })
        .boxed())
    }
}

#[tokio::test]
async fn a_published_page_lands_on_its_tool_part_and_leaves_with_the_chat() {
    use zeron_rpc::RpcService;
    let secret = Arc::new(Mutex::new(None));
    let release = Arc::new(tokio::sync::Notify::new());
    let rig = rig_on(Arc::new(PublishingHarness {
        secret: secret.clone(),
        release: release.clone(),
    }));
    let core = &rig.core;
    core.sessions.set_tool_server_port(1);
    queue_human_run(core, "lead", "cmd-1", "msg-1");
    wait_for(|| secret.lock().unwrap().is_some(), "the run secret").await;
    let secret = secret.lock().unwrap().clone().unwrap();
    let rpc = core.rpc_service();
    let publish = |html: &str, secret: &str| serde_json::json!({ "chatId": "lead", "runSecret": secret, "html": html, "title": "Chart", "height": 420 });
    let forged = rpc
        .handle(
            zeron_rpc::methods::PUBLISH_HTML_RENDER,
            publish("<p>chart</p>", "forged"),
        )
        .await;
    assert!(forged.is_err(), "a forged secret publishes nothing");
    let missing = rpc
        .handle(
            zeron_rpc::methods::PUBLISH_HTML_RENDER,
            publish(r#"<img src="/nope/shot.png">"#, &secret),
        )
        .await;
    assert!(
        matches!(&missing, Err(zeron_rpc::RpcError::Failed(m)) if m.contains("/nope/shot.png")),
        "an unreadable image names itself: {:?}",
        missing.err()
    );
    let reply = match rpc
        .handle(
            zeron_rpc::methods::PUBLISH_HTML_RENDER,
            publish("<p>chart</p>", &secret),
        )
        .await
        .expect("publish")
    {
        zeron_rpc::RpcReply::Value(value) => value,
        _ => panic!("publish answers once"),
    };
    let path = reply["htmlRender"]["path"].as_str().unwrap().to_owned();
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "<p>chart</p>");
    assert_eq!(reply["message"], zeron_proto::html_render::RENDERED_MESSAGE);
    release.notify_one();
    let stamped = || {
        entries(core, "lead").iter().find_map(|entry| {
            entry.parts.iter().find_map(|part| match part {
                zeron_doc::MessagePart::Tool {
                    id, html_render, ..
                } if id == "tc-html" => html_render.clone(),
                _ => None,
            })
        })
    };
    wait_for(
        || stamped().is_some(),
        "the page reference on the tool part",
    )
    .await;
    let page = stamped().unwrap();
    assert_eq!(page.path, path);
    assert_eq!(page.title, "Chart");
    assert_eq!(page.height, 420);
    core.rpc_service()
        .handle(
            zeron_rpc::methods::MUTATE,
            serde_json::json!({ "op": "deleteChat", "chatId": "lead" }),
        )
        .await
        .expect("delete chat");
    assert!(
        !std::path::Path::new(&path).exists(),
        "deleting the chat deletes its pages"
    );
    core.shutdown().await;
}

struct GuidanceHarness {
    guidance: Arc<Mutex<Option<Option<String>>>>,
}

#[async_trait]
impl Harness for GuidanceHarness {
    fn id(&self) -> HarnessId {
        HarnessId::Mock
    }
    fn display_name(&self) -> &str {
        "Guidance"
    }
    fn supports_steering(&self) -> bool {
        false
    }
    fn steering_mode(&self) -> SteeringMode {
        SteeringMode::TurnBoundary
    }
    fn reasoning_levels(&self) -> &[ReasoningLevel] {
        &[]
    }
    async fn models(&self) -> Result<Vec<Model>, HarnessError> {
        Ok(vec![])
    }
    async fn run(
        &self,
        _request: RunRequest,
        controls: RunControls,
    ) -> Result<BoxStream<'static, Result<AgentEvent, HarnessError>>, HarnessError> {
        *self.guidance.lock().unwrap() = Some(
            controls
                .tool_server
                .as_ref()
                .and_then(|server| server.guidance.clone()),
        );
        Ok(futures::stream::iter(vec![Ok(AgentEvent::Done {
            status: DoneStatus::Completed,
            result: None,
            error: None,
            session_id: None,
        })])
        .boxed())
    }
}

#[tokio::test]
async fn a_run_carries_the_host_prompt_then_the_chat_instructions() {
    let guidance = Arc::new(Mutex::new(None));
    let rig = rig_on(Arc::new(GuidanceHarness {
        guidance: guidance.clone(),
    }));
    let core = &rig.core;
    core.sessions.set_tool_server_port(1);
    core.registry
        .set_system_prompt("Reply in Vietnamese.")
        .unwrap();
    core.workspace
        .set_chat_config(
            "lead",
            &zeron_proto::ChatConfig {
                harness: HarnessId::Mock,
                model: None,
                reasoning: None,
                model_options: Default::default(),
                sandbox: SandboxLevel::WorkspaceWrite,
                permission_mode: Default::default(),
                instructions: "Review only.".into(),
                profile_id: None,
                instance_id: None,
                disabled_tools: Vec::new(),
            },
        )
        .unwrap();
    queue_human_run(core, "lead", "cmd-1", "msg-1");
    wait_for(|| guidance.lock().unwrap().is_some(), "the run").await;
    let text = guidance.lock().unwrap().clone().unwrap().unwrap();
    assert!(
        text.starts_with("Reply in Vietnamese.\n\nReview only."),
        "{text}"
    );
    core.shutdown().await;
}

struct AskingHarness {
    prompts: Arc<Mutex<Vec<String>>>,
    approval: bool,
    steerable: bool,
}

#[async_trait]
impl Harness for AskingHarness {
    fn id(&self) -> HarnessId {
        HarnessId::Mock
    }
    fn display_name(&self) -> &str {
        "Asking"
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
        controls: RunControls,
    ) -> Result<BoxStream<'static, Result<AgentEvent, HarnessError>>, HarnessError> {
        self.prompts.lock().unwrap().push(request.prompt.clone());
        let asks = request.prompt.starts_with("please ask");
        let holds = request.prompt.contains("please hold");
        let approval = self.approval;
        let prompts = self.prompts.clone();
        let RunControls {
            request_input,
            mut steering,
            ..
        } = controls;
        let (tx, rx) = tokio::sync::mpsc::channel::<Result<AgentEvent, HarnessError>>(8);
        tokio::spawn(async move {
            let _ = tx
                .send(Ok(AgentEvent::SessionStarted {
                    harness: HarnessId::Mock,
                    model: "ask-1".into(),
                    tools: vec![],
                    cwd: "/tmp".into(),
                    session_id: "hs-ask".into(),
                    assistant_message_id: "a-ask".into(),
                }))
                .await;
            let mut text = "noted".to_string();
            if asks {
                let answers = (request_input)(vec![zeron_proto::UserInputQuestion {
                    id: "q-curl".into(),
                    header: "Agent question".into(),
                    question: "Execute `curl -o main.jpg https://example.com/a.jpg`".into(),
                    options: vec!["Yes, proceed".into(), "No".into()],
                    multi_select: false,
                    kind: None,
                    prefill: None,
                    multiline: false,
                    approval,
                }])
                .await
                .unwrap_or_default();
                text = answers
                    .first()
                    .and_then(|a| a.labels.first().cloned())
                    .unwrap_or_else(|| "no answer".into());
            }
            if holds {
                tokio::time::sleep(Duration::from_millis(800)).await;
            }
            while let Ok(message) = steering.try_recv() {
                prompts.lock().unwrap().push(message.prompt);
                let _ = tx
                    .send(Ok(AgentEvent::Steered {
                        assistant_message_id: None,
                        next_assistant_message_id: None,
                    }))
                    .await;
            }
            let _ = tx.send(Ok(AgentEvent::TextDelta { text })).await;
            let _ = tx
                .send(Ok(AgentEvent::Done {
                    status: DoneStatus::Completed,
                    result: None,
                    error: None,
                    session_id: Some("hs-ask".into()),
                }))
                .await;
        });
        Ok(futures::stream::unfold(rx, |mut rx| async move {
            rx.recv().await.map(|event| (event, rx))
        })
        .boxed())
    }
}

fn asking_rig(approval: bool) -> Rig {
    let prompts = Arc::new(Mutex::new(Vec::new()));
    let rig = rig_on(Arc::new(AskingHarness {
        prompts: prompts.clone(),
        approval,
        steerable: false,
    }));
    Rig { prompts, ..rig }
}

fn pending_request(core: &EngineCore, chat_id: &str) -> Option<String> {
    entries(core, chat_id).iter().rev().find_map(|entry| {
        entry.parts.iter().find_map(|part| match part {
            zeron_doc::MessagePart::Input {
                request_id,
                resolved: false,
                ..
            } => Some(request_id.clone()),
            _ => None,
        })
    })
}

fn prompts_with(rig: &Rig, needle: &str) -> Vec<String> {
    rig.prompts
        .lock()
        .unwrap()
        .iter()
        .filter(|prompt| prompt.contains(needle))
        .cloned()
        .collect()
}

#[tokio::test]
async fn a_child_blocked_on_a_tool_approval_wakes_its_owner_with_the_request() {
    let rig = asking_rig(true);
    let view = rig
        .core
        .doc_host
        .delegate_task(
            delegate("please ask before downloading"),
            Default::default(),
        )
        .await
        .expect("delegate");
    let child = view.task_id.clone();
    wait_for(
        || pending_request(&rig.core, &child).is_some(),
        "the child's approval card",
    )
    .await;
    let request = pending_request(&rig.core, &child).unwrap();
    wait_for(
        || !prompts_with(&rig, "is waiting for input").is_empty(),
        "the owner's input wake",
    )
    .await;
    let wake = prompts_with(&rig, "is waiting for input").remove(0);
    assert!(
        wake.starts_with(&format!("Delegated task {child} (")),
        "{wake}"
    );
    assert!(wake.contains(&format!("requestId {request}")), "{wake}");
    assert!(
        wake.contains("- tool approval q-curl: Execute `curl -o main.jpg"),
        "{wake}"
    );
    assert!(
        wake.contains("Options: \"Yes, proceed\" | \"No\""),
        "{wake}"
    );
    let woken = entries(&rig.core, "lead");
    assert!(
        woken
            .iter()
            .any(|e| e.from_chat_id.as_deref() == Some(child.as_str())),
        "the wake reads as a message from the child"
    );

    assert!(
        rig.core
            .sessions
            .respond_input(
                &child,
                &request,
                vec![zeron_proto::UserInputAnswer {
                    question_id: "q-curl".into(),
                    labels: vec!["Yes, proceed".into()],
                }],
            )
            .unwrap()
    );
    wait_for(
        || delegation(&rig.core, &child).status == zeron_proto::DelegationStatus::Completed,
        "the child to finish",
    )
    .await;
    wait_for(|| idle(&rig.core, "lead"), "the owner's turns").await;
    assert_eq!(
        prompts_with(&rig, "is waiting for input").len(),
        1,
        "one wake per request"
    );
    rig.core.shutdown().await;
}

#[tokio::test]
async fn a_watched_peer_that_asks_wakes_its_watcher_and_an_unwatched_one_does_not() {
    let rig = asking_rig(false);
    for peer in ["watched", "unwatched"] {
        rig.core
            .workspace
            .create_chat(peer, Some("space-tt"), None, None, None)
            .expect("peer chat");
    }
    assert!(
        rig.core
            .doc_host
            .set_thread_watches_for("lead", &["lead".into()], &[])
            .is_err(),
        "a chat cannot watch itself"
    );
    assert!(
        rig.core
            .doc_host
            .set_thread_watches_for("lead", &["nobody".into()], &[])
            .is_err(),
        "only live threads can be watched"
    );
    let watching = rig
        .core
        .doc_host
        .set_thread_watches_for("lead", &["watched".into()], &[])
        .expect("watch");
    assert_eq!(watching, vec!["watched".to_string()]);

    queue_run(
        &rig.core,
        "unwatched",
        "cmd-u",
        "msg-u",
        "please ask about the plan",
    );
    wait_for(
        || pending_request(&rig.core, "unwatched").is_some(),
        "the unwatched question",
    )
    .await;
    queue_run(
        &rig.core,
        "watched",
        "cmd-w",
        "msg-w",
        "please ask about the plan",
    );
    wait_for(
        || !prompts_with(&rig, "is waiting for input").is_empty(),
        "the watcher's wake",
    )
    .await;
    let wakes = prompts_with(&rig, "is waiting for input");
    assert_eq!(wakes.len(), 1, "{wakes:?}");
    assert!(
        wakes[0].starts_with("Watched thread watched ("),
        "{}",
        wakes[0]
    );
    assert!(wakes[0].contains("- question q-curl:"), "{}", wakes[0]);

    let watching = rig
        .core
        .doc_host
        .set_thread_watches_for("lead", &[], &["watched".into()])
        .expect("unwatch");
    assert!(watching.is_empty());
    rig.core.shutdown().await;
}

#[tokio::test]
async fn an_input_wake_answered_before_the_owner_reads_it_is_dropped() {
    let rig = asking_rig(true);
    queue_run(
        &rig.core,
        "lead",
        "cmd-l",
        "msg-l",
        "please hold for a second",
    );
    wait_for(|| working(&rig.core, "lead"), "the owner's own turn").await;
    let view = rig
        .core
        .doc_host
        .delegate_task(
            delegate("please ask before downloading"),
            Default::default(),
        )
        .await
        .expect("delegate");
    let child = view.task_id.clone();
    wait_for(
        || pending_request(&rig.core, &child).is_some() && queue_len(&rig.core, "lead") > 0,
        "the wake queued behind the owner's turn",
    )
    .await;
    let request = pending_request(&rig.core, &child).unwrap();
    assert!(
        rig.core
            .sessions
            .respond_input(
                &child,
                &request,
                vec![zeron_proto::UserInputAnswer {
                    question_id: "q-curl".into(),
                    labels: vec!["Yes, proceed".into()],
                }],
            )
            .unwrap()
    );
    wait_for(
        || delegation(&rig.core, &child).status == zeron_proto::DelegationStatus::Completed,
        "the child to finish",
    )
    .await;
    wait_for(
        || !prompts_with(&rig, "(\"please ask before downloading\") completed.").is_empty(),
        "the completion wake",
    )
    .await;
    wait_for(|| idle(&rig.core, "lead"), "the owner's turns").await;
    assert!(
        prompts_with(&rig, "is waiting for input").is_empty(),
        "the answered question never reached the owner"
    );
    rig.core.shutdown().await;
}

#[tokio::test]
async fn an_input_wake_into_an_owner_parked_on_its_own_question_leaves_that_question_open() {
    let prompts = Arc::new(Mutex::new(Vec::new()));
    let rig = rig_on(Arc::new(AskingHarness {
        prompts: prompts.clone(),
        approval: false,
        steerable: true,
    }));
    let rig = Rig { prompts, ..rig };
    queue_run(
        &rig.core,
        "lead",
        "cmd-l",
        "msg-l",
        "please ask about the scope, then please hold",
    );
    wait_for(
        || pending_request(&rig.core, "lead").is_some(),
        "the owner's own question",
    )
    .await;
    let own = pending_request(&rig.core, "lead").unwrap();
    let child = rig
        .core
        .doc_host
        .delegate_task(
            delegate("please ask before downloading"),
            Default::default(),
        )
        .await
        .expect("delegate")
        .task_id;
    wait_for(
        || {
            entries(&rig.core, "lead")
                .iter()
                .any(|e| e.from_chat_id.as_deref() == Some(child.as_str()))
                && queue_len(&rig.core, "lead") == 0
        },
        "the child's input wake to reach the owner",
    )
    .await;
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(pending_request(&rig.core, "lead"), Some(own.clone()));
    assert!(rig.core.sessions.awaits_input("lead", &own));
    assert!(prompts_with(&rig, "is waiting for input").is_empty());
    assert_eq!(prompts_with(&rig, "please ask about the scope").len(), 1);
    let status = || {
        rig.core
            .sessions
            .session_status("lead")
            .map(|session| session.status)
    };
    assert_eq!(
        status(),
        Some(SessionStatus::AwaitingInput),
        "the steered wake leaves the owner waiting on the person"
    );

    assert!(
        rig.core
            .sessions
            .respond_input(
                "lead",
                &own,
                vec![zeron_proto::UserInputAnswer {
                    question_id: "q-curl".into(),
                    labels: vec!["Yes, proceed".into()],
                }],
            )
            .unwrap()
    );
    wait_for(
        || status() == Some(SessionStatus::Working),
        "the answer to put the owner back to work",
    )
    .await;
    wait_for(
        || !prompts_with(&rig, "is waiting for input").is_empty(),
        "the wake to reach the owner after the answer",
    )
    .await;
    wait_for(
        || {
            entries(&rig.core, "lead").iter().any(|entry| {
                entry.role == MessageRole::Assistant
                    && entry.parts.iter().any(|part| {
                        matches!(part, zeron_doc::MessagePart::Text { text, .. } if text == "Yes, proceed")
                    })
            })
        },
        "the owner's turn to carry the person's answer",
    )
    .await;
    rig.core.shutdown().await;
}

#[tokio::test]
async fn a_plan_edit_moves_unstarted_steps_and_names_the_new_order() {
    let rig = rig(Duration::from_millis(400));
    let rpc = rig.core.rpc_service();
    let call = |method: &'static str, params: serde_json::Value| {
        let rpc = rpc.clone();
        async move { zeron_rpc::RpcService::handle(&*rpc, method, params).await }
    };
    let mutate = |params| call(zeron_rpc::methods::MUTATE, params);
    mutate(serde_json::json!({ "op": "createTask", "taskId": "t1", "title": "Shop" }))
        .await
        .unwrap();
    mutate(serde_json::json!({
        "op": "startTask", "taskId": "t1", "chatId": "task-lead",
        "spaceId": "space-tt", "orchestrate": true,
    }))
    .await
    .unwrap();
    mutate(serde_json::json!({
        "op": "planSteps", "taskId": "t1", "fromChatId": "task-lead",
        "steps": [{ "title": "Scaffold" }, { "title": "Layout" }, { "title": "Grid" }, { "title": "Cart" }],
    }))
    .await
    .unwrap();
    mutate(serde_json::json!({ "op": "approvePlan", "taskId": "t1" }))
        .await
        .unwrap();
    let steps = rig.core.workspace.task_steps_of("t1");
    call(
        zeron_rpc::methods::CREATE_THREAD,
        serde_json::json!({ "fromChatId": "task-lead", "prompt": "do it", "taskStepId": steps[0].id }),
    )
    .await
    .unwrap();
    let ids = |order: [usize; 4]| order.map(|ix| steps[ix].id.clone());
    let Err(err) = mutate(serde_json::json!({
        "op": "editTaskPlan", "taskId": "t1", "order": ids([1, 0, 2, 3]),
    }))
    .await
    else {
        panic!("a started step keeps its place");
    };
    assert!(err.to_string().contains("have not started"), "{err}");

    mutate(serde_json::json!({
        "op": "editTaskPlan", "taskId": "t1", "order": ids([0, 3, 1, 2]),
    }))
    .await
    .unwrap();
    let after = rig.core.workspace.task_steps_of("t1");
    assert_eq!(
        after.iter().map(|s| s.title.as_str()).collect::<Vec<_>>(),
        ["Scaffold", "Cart", "Layout", "Grid"]
    );
    let key = |id: &str, rows: &[zeron_proto::TaskStep]| {
        rows.iter().find(|s| s.id == id).unwrap().order_key.clone()
    };
    for ix in [0, 1, 2] {
        assert_eq!(
            key(&steps[ix].id, &after),
            steps[ix].order_key,
            "only the moved step is written"
        );
    }
    let edited = "The person edited the plan in Mission Control (task id t1)";
    wait_for(
        || !prompts_with(&rig, edited).is_empty(),
        "the lead's edit message",
    )
    .await;
    let message = prompts_with(&rig, edited).remove(0);
    let line = format!(
        "- The steps are in a new order: 1. \"Scaffold\", 2. \"Cart\" (moved, taskStepId {}), 3. \"Layout\", 4. \"Grid\".",
        steps[3].id
    );
    assert!(message.contains(&line), "{line} in {message}");
}

fn attached_to(rig: &Rig, needle: &str) -> Vec<String> {
    rig.attachments
        .lock()
        .unwrap()
        .iter()
        .find(|(prompt, _)| prompt.contains(needle))
        .map(|(_, attachments)| attachments.clone())
        .unwrap_or_default()
}

#[tokio::test]
async fn task_and_step_images_ride_the_first_message_of_the_thread_that_runs_them() {
    let rig = rig(Duration::from_millis(50));
    let rpc = rig.core.rpc_service();
    let call = |method: &'static str, params: serde_json::Value| {
        let rpc = rpc.clone();
        async move { zeron_rpc::RpcService::handle(&*rpc, method, params).await }
    };
    let mutate = |params| call(zeron_rpc::methods::MUTATE, params);
    let trailer = "Attached images (local files — open them to view):";
    mutate(serde_json::json!({
        "op": "createTask", "taskId": "t1", "title": "Shop",
        "images": ["/tmp/uploads/aa-overview.png"],
    }))
    .await
    .unwrap();
    mutate(serde_json::json!({
        "op": "startTask", "taskId": "t1", "chatId": "task-lead",
        "spaceId": "space-tt", "orchestrate": true,
    }))
    .await
    .unwrap();
    wait_for(
        || !prompts_with(&rig, "You lead this task").is_empty(),
        "the lead's first turn",
    )
    .await;
    let first = prompts_with(&rig, "You lead this task").remove(0);
    assert!(
        first.ends_with(&format!("{trailer}\n- /tmp/uploads/aa-overview.png")),
        "{first}"
    );
    assert_eq!(
        attached_to(&rig, "You lead this task"),
        ["/tmp/uploads/aa-overview.png"]
    );

    mutate(serde_json::json!({
        "op": "planSteps", "taskId": "t1", "fromChatId": "task-lead",
        "steps": [{ "title": "Grid" }, { "title": "Cart" }],
    }))
    .await
    .unwrap();
    let steps = rig.core.workspace.task_steps_of("t1");
    let shots = ["/tmp/uploads/bb-grid.png", "/tmp/uploads/cc-grid-2.png"];
    mutate(serde_json::json!({
        "op": "editTaskPlan", "taskId": "t1",
        "overviewImages": ["/tmp/uploads/aa-overview.png", "/tmp/uploads/dd-flow.png"],
        "steps": [
            { "stepId": steps[0].id, "title": "Grid", "notes": "", "images": shots },
            { "stepId": steps[1].id, "title": "Cart", "notes": "", "images": ["/tmp/uploads/ee-cart.png"] },
        ],
    }))
    .await
    .unwrap();
    let edited = "The person edited the plan in Mission Control (task id t1)";
    wait_for(
        || !prompts_with(&rig, edited).is_empty(),
        "the lead's edit message",
    )
    .await;
    let message = prompts_with(&rig, edited).remove(0);
    assert!(
        message.contains(&format!(
            "- Step \"Grid\" (taskStepId {}) has 2 new images.",
            steps[0].id
        )),
        "{message}"
    );
    assert_eq!(attached_to(&rig, edited), ["/tmp/uploads/dd-flow.png"]);
    assert_eq!(rig.core.workspace.task("t1").unwrap().images.len(), 2);

    let Ok(zeron_rpc::RpcReply::Value(replanned)) = mutate(serde_json::json!({
        "op": "planSteps", "taskId": "t1", "fromChatId": "task-lead",
        "steps": [{ "title": " Grid " }, { "title": "Checkout" }],
    }))
    .await
    else {
        panic!("the lead re-plans");
    };
    let old = steps;
    let steps = rig.core.workspace.task_steps_of("t1");
    assert_eq!(steps.len(), 2);
    assert_eq!(steps[0].images, shots, "a step keeps its images by title");
    assert!(steps[1].images.is_empty());
    assert_eq!(
        rig.core.workspace.task("t1").unwrap().images,
        [
            "/tmp/uploads/aa-overview.png",
            "/tmp/uploads/dd-flow.png",
            "/tmp/uploads/ee-cart.png"
        ],
        "a step no new step names hands its images to the overview"
    );
    assert_eq!(
        replanned["imagesMoved"],
        serde_json::json!([
            format!(
                "Step \"Grid\" (taskStepId {}) had 2 images, now on the new step with its title (taskStepId {}).",
                old[0].id, steps[0].id
            ),
            format!(
                "Step \"Cart\" (taskStepId {}) had 1 image, now on the overview: no single new step has its title.",
                old[1].id
            ),
        ])
    );

    mutate(serde_json::json!({ "op": "approvePlan", "taskId": "t1" }))
        .await
        .unwrap();
    let Ok(zeron_rpc::RpcReply::Value(reply)) = call(
        zeron_rpc::methods::CREATE_THREAD,
        serde_json::json!({ "fromChatId": "task-lead", "prompt": "build the grid", "taskStepId": steps[0].id }),
    )
    .await
    else {
        panic!("the step thread starts");
    };
    let thread = reply["chatId"].as_str().unwrap().to_string();
    wait_for(
        || !prompts_with(&rig, "build the grid").is_empty(),
        "the step's first turn",
    )
    .await;
    assert_eq!(attached_to(&rig, "build the grid"), shots);
    let entry = entries(&rig.core, &thread).remove(0);
    let text = match &entry.parts[0] {
        zeron_doc::MessagePart::Text { text, .. } => text.clone(),
        other => panic!("a text part, not {other:?}"),
    };
    assert_eq!(
        text,
        format!(
            "build the grid\n\n{trailer}\n- {}\n- {}",
            shots[0], shots[1]
        ),
        "the step's transcript shows the images the way a composer message does"
    );
    rig.core.shutdown().await;
}

#[tokio::test]
async fn an_orchestrated_task_runs_its_approved_steps_on_bound_threads() {
    const FOLD_MS: i64 = 8_000;
    unsafe { std::env::set_var("ZERON_SUPERVISOR_FOLD_MS", FOLD_MS.to_string()) };
    let rig = rig(Duration::from_millis(400));
    let rpc = rig.core.rpc_service();
    let call = |method: &'static str, params: serde_json::Value| {
        let rpc = rpc.clone();
        async move { zeron_rpc::RpcService::handle(&*rpc, method, params).await }
    };
    let mutate = |params| call(zeron_rpc::methods::MUTATE, params);
    mutate(serde_json::json!({ "op": "createTask", "taskId": "t1", "title": "Shop backend" }))
        .await
        .unwrap();
    mutate(serde_json::json!({
        "op": "startTask", "taskId": "t1", "chatId": "task-lead",
        "spaceId": "space-tt", "orchestrate": true,
    }))
    .await
    .unwrap();
    wait_for(
        || !prompts_with(&rig, "You lead this task (task id t1)").is_empty(),
        "the lead's brief",
    )
    .await;
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = format!("port:{}", listener.local_addr().unwrap().port());
    let plan = serde_json::json!({
        "op": "planSteps", "taskId": "t1", "fromChatId": "task-lead",
        "steps": [{ "title": "Build the API" }, { "title": "Write the docs", "notes": "README" }],
        "overview": "Ship the API", "resources": [port, port],
    });
    let mut stranger = plan.clone();
    stranger["fromChatId"] = "lead".into();
    assert!(mutate(stranger).await.is_err(), "only the lead plans");
    let mut bad = plan.clone();
    bad["resources"] = serde_json::json!(["port:0"]);
    let Err(err) = mutate(bad).await else {
        panic!("a bad resource key is refused");
    };
    assert!(err.to_string().contains("is not a resource"), "{err}");
    mutate(plan).await.unwrap();
    assert_eq!(
        rig.core.workspace.task("t1").unwrap().resources,
        [port.clone()]
    );
    let steps = rig.core.workspace.task_steps_of("t1");
    assert_eq!(
        steps.iter().map(|s| s.title.as_str()).collect::<Vec<_>>(),
        ["Build the API", "Write the docs"]
    );
    let run = |step: &zeron_proto::TaskStep, owner: &str| {
        call(
            zeron_rpc::methods::CREATE_THREAD,
            serde_json::json!({
                "fromChatId": owner, "prompt": format!("do: {}", step.title), "taskStepId": step.id,
            }),
        )
    };
    let started = |reply: Result<zeron_rpc::RpcReply, zeron_rpc::RpcError>| match reply {
        Ok(zeron_rpc::RpcReply::Value(value)) => value["chatId"].as_str().unwrap().to_string(),
        Ok(_) => panic!("a chat id"),
        Err(err) => panic!("{err}"),
    };
    let refused = |reply: Result<zeron_rpc::RpcReply, zeron_rpc::RpcError>| match reply {
        Err(err) => err.to_string(),
        Ok(_) => panic!("a refusal"),
    };
    let err = refused(run(&steps[0], "task-lead").await);
    assert!(err.contains("plan_pending"), "{err}");
    let err = refused(run(&steps[0], "lead").await);
    assert!(err.contains("Only the task's lead runs its steps"), "{err}");

    mutate(serde_json::json!({ "op": "approvePlan", "taskId": "t1" }))
        .await
        .unwrap();
    assert!(!rig.core.workspace.task("t1").unwrap().plan_pending);
    wait_for(
        || !prompts_with(&rig, "The plan is approved. Run it.").is_empty(),
        "the lead's approval message",
    )
    .await;

    let first = started(run(&steps[0], "task-lead").await);
    let turn_of = |step_id: &str| {
        rig.core
            .workspace
            .task_step(step_id)
            .unwrap()
            .turn
            .map(|turn| turn.ended)
    };
    wait_for(
        || turn_of(&steps[0].id) == Some(zeron_proto::StepTurnEnd::Completed),
        "the first step's thread to finish its turn",
    )
    .await;
    let handback = format!(
        "Step \"Build the API\" (taskStepId {}) finished a turn on its thread {first}.\n\n<agent-response>\nnoted\n</agent-response>\n\nRead it with read_thread, then mark the step done or skipped with the tasks tool, action \"step\", or tell the thread what is still missing with send_message_to_thread.",
        steps[0].id
    );
    wait_for(
        || !prompts_with(&rig, &handback).is_empty(),
        "the lead's step message",
    )
    .await;
    assert_eq!(
        prompts_with(&rig, &handback),
        [handback.clone()],
        "one handback carrying the step's reply, without peer framing"
    );
    let second = started(run(&steps[1], "task-lead").await);
    let err = refused(run(&steps[1], "task-lead").await);
    assert!(err.contains("still running"), "{err}");
    let status = |step_id: &str| {
        let step = rig.core.workspace.task_step(step_id).unwrap();
        let chat = step
            .chat_id
            .as_deref()
            .and_then(|id| rig.core.workspace.chat(id).unwrap());
        let indicator = chat.as_ref().map(|chat| {
            let session = rig.core.sessions.session_status(&chat.id);
            zeron_proto::view::display_status(chat, session.as_ref(), chrono::Utc::now())
        });
        (
            step.chat_id.clone(),
            zeron_proto::step_status(&step, chat.as_ref(), indicator),
        )
    };
    assert_eq!(
        status(&steps[0].id),
        (Some(first.clone()), zeron_proto::StepStatus::Done)
    );
    assert_eq!(
        status(&steps[1].id),
        (Some(second.clone()), zeron_proto::StepStatus::Working)
    );
    let thread = rig.core.workspace.chat(&second).unwrap().unwrap();
    assert_eq!(
        thread.title.as_deref(),
        Some("Write the docs"),
        "an untitled step thread takes the step's title"
    );
    assert_eq!(thread.created_by_chat_id.as_deref(), Some("task-lead"));
    assert!(
        thread.delegation.is_none(),
        "a step runs on an ordinary thread"
    );

    assert_eq!(rig.core.workspace.task("t1").unwrap().plan, "Ship the API");
    let edit = |remove: Vec<&str>| {
        serde_json::json!({
            "op": "editTaskPlan", "taskId": "t1", "overview": "Ship the API, then its docs",
            "steps": [
                { "stepId": steps[1].id, "title": "Write the docs", "notes": "README and a guide" },
                { "title": "Tag the release", "notes": "After both steps" },
            ],
            "removeStepIds": remove,
        })
    };
    let err = refused(mutate(edit(vec![steps[0].id.as_str()])).await);
    assert!(err.contains("has started"), "{err}");
    let task = rig.core.workspace.task("t1").unwrap();
    assert_eq!(task.plan, "Ship the API", "a refused edit applies nothing");
    let titles = |rig: &Rig| {
        rig.core
            .workspace
            .task_steps_of("t1")
            .iter()
            .map(|s| (s.title.clone(), s.notes.clone()))
            .collect::<Vec<_>>()
    };
    assert_eq!(titles(&rig).len(), 2);
    assert_eq!(titles(&rig)[1].1, "README");

    let ensure = || {
        call(
            zeron_rpc::methods::ENSURE_SUPERVISOR,
            serde_json::json!({ "config": { "harness": "mock", "sandbox": "workspace-write" } }),
        )
    };
    let supervisor = started(ensure().await);
    assert_eq!(
        started(ensure().await),
        supervisor,
        "one Supervisor per device"
    );
    let supervise = |on: bool| {
        mutate(
            serde_json::json!({ "op": "setMissionSupervised", "taskId": "t1", "supervised": on }),
        )
    };
    let wakes = |rig: &Rig| prompts_with(rig, "Supervisor wake ");
    supervise(false).await.unwrap();

    mutate(edit(Vec::new())).await.unwrap();
    let edited = "The person edited the plan in Mission Control (task id t1)";
    wait_for(
        || !prompts_with(&rig, edited).is_empty(),
        "the lead's edit message",
    )
    .await;
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert!(
        wakes(&rig).is_empty(),
        "a mission with the Supervisor off wakes nobody"
    );
    assert_eq!(
        rig.core.workspace.task("t1").unwrap().plan,
        "Ship the API, then its docs"
    );
    let after = titles(&rig);
    assert_eq!(after[1].1, "README and a guide");
    assert_eq!(after[2].0, "Tag the release");
    let added = rig
        .core
        .workspace
        .task_steps_of("t1")
        .into_iter()
        .find(|s| s.title == "Tag the release")
        .unwrap();
    let message = prompts_with(&rig, edited).remove(0);
    for change in [
        "- The overview changed.".to_string(),
        format!(
            "- Step \"Write the docs\" (taskStepId {}) has a new spec.",
            steps[1].id
        ),
        format!("- New step \"Tag the release\" (taskStepId {}).", added.id),
    ] {
        assert!(message.contains(&change), "{change} in {message}");
    }

    supervise(true).await.unwrap();
    let removed_at = chrono::Utc::now();
    mutate(serde_json::json!({
        "op": "editTaskPlan", "taskId": "t1", "removeStepIds": [added.id],
    }))
    .await
    .unwrap();
    wait_for(
        || prompts_with(&rig, edited).len() == 2,
        "the removal message",
    )
    .await;
    assert!(prompts_with(&rig, edited)[1].contains("is removed"));
    assert_eq!(titles(&rig).len(), 2, "a planned step can be removed");
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(
        prompts_with(&rig, edited).len(),
        2,
        "each edit reaches the lead as one message"
    );

    wait_for(|| wakes(&rig).len() == 1, "the Supervisor's first wake").await;
    let first = wakes(&rig).remove(0);
    for line in [
        "Plan edited by the person · Shop backend (task t1)".to_string(),
        format!(
            "- Step \"Tag the release\" (taskStepId {}) is removed.",
            added.id
        ),
        "Last reply from the lead:\n<agent-response>\nnoted\n</agent-response>".to_string(),
    ] {
        assert!(first.contains(&line), "{line} in {first}");
    }
    let held_from = chrono::Utc::now();
    mutate(serde_json::json!({
        "op": "editTaskPlan", "taskId": "t1", "overview": "Ship the API and document it",
    }))
    .await
    .unwrap();
    mutate(serde_json::json!({
        "op": "editTaskPlan", "taskId": "t1",
        "steps": [{ "stepId": steps[1].id, "title": "Write the docs", "notes": "README, a guide and a FAQ" }],
    }))
    .await
    .unwrap();
    wait_for(|| wakes(&rig).len() == 2, "the folded wake").await;
    let folded = wakes(&rig).remove(1);
    for line in [
        "2 events since".to_string(),
        "- The overview changed.".to_string(),
        format!("(taskStepId {}) has a new spec.", steps[1].id),
    ] {
        assert!(folded.contains(&line), "{line} in {folded}");
    }
    let zeron_rpc::RpcReply::Stream(mut watch) =
        call(zeron_rpc::methods::WATCH_SUPERVISOR, serde_json::json!({}))
            .await
            .unwrap()
    else {
        panic!("a snapshot stream");
    };
    let snapshot: zeron_proto::supervisor::SupervisorSnapshot =
        serde_json::from_value(watch.next().await.unwrap()).unwrap();
    assert_eq!(snapshot.chat_id.as_deref(), Some(supervisor.as_str()));
    let [second_wake, first_wake] = &snapshot.wakes[..] else {
        panic!("two wakes: {:?}", snapshot.wakes);
    };
    let fold = chrono::Duration::milliseconds(FOLD_MS);
    assert!(
        first_wake.at - removed_at < chrono::Duration::seconds(1),
        "the first edit wakes it at once"
    );
    assert!(
        held_from < first_wake.at + fold && second_wake.at - first_wake.at >= fold,
        "later edits wait out the window and share one wake"
    );
    let asked: Vec<String> = entries(&rig.core, &supervisor)
        .into_iter()
        .filter(|entry| entry.role == MessageRole::User)
        .map(|entry| entry.id)
        .collect();
    assert_eq!(asked, [first_wake.id.clone(), second_wake.id.clone()]);
    let row = rig.core.workspace.chat(&supervisor).unwrap().unwrap();
    assert!(!row.is_top_level(), "the Supervisor has no sidebar row");

    use zeron_proto::supervisor::{InterventionChip, InterventionKind, InterventionTrigger};
    let settings = |message: bool| {
        call(
            zeron_rpc::methods::SET_SUPERVISOR_SETTINGS,
            serde_json::json!({
                "messageThreads": message, "rebrief": true, "pauseSteps": true,
                "startMissions": false, "heartbeatMinutes": 0,
            }),
        )
    };
    let send = |to: String| {
        call(
            zeron_rpc::methods::SEND_THREAD_MESSAGE,
            serde_json::json!({ "chatId": to, "fromChatId": supervisor, "text": "Check the error codes" }),
        )
    };
    let step_thread = rig
        .core
        .workspace
        .task_step(&steps[0].id)
        .unwrap()
        .chat_id
        .unwrap();
    settings(false).await.unwrap();
    let err = refused(send(step_thread.clone()).await);
    assert!(
        err.contains("Messaging leads and step threads is off"),
        "{err}"
    );
    settings(true).await.unwrap();
    let sent = |reply: Result<zeron_rpc::RpcReply, zeron_rpc::RpcError>| match reply {
        Ok(zeron_rpc::RpcReply::Value(value)) => value["id"].as_str().unwrap().to_string(),
        Ok(_) => panic!("a message id"),
        Err(err) => panic!("{err}"),
    };
    let to_step = sent(send(step_thread).await);
    let completed = InterventionChip::StepCompletedTurn { steps: vec![1] };
    let snapshot = supervisor_until(&rpc, |s| {
        s.interventions
            .first()
            .is_some_and(|row| row.chips.contains(&completed))
    })
    .await;
    let [row] = snapshot.interventions.as_slice() else {
        panic!("one intervention: {:?}", snapshot.interventions);
    };
    assert_eq!(row.kind, InterventionKind::Send);
    assert_eq!(row.task_id.as_deref(), Some("t1"));
    assert_eq!(row.step_ids, [steps[0].id.clone()]);
    assert_eq!(row.message_id.as_deref(), Some(to_step.as_str()));
    assert_eq!(row.text, "Check the error codes");
    assert_eq!(
        row.trigger,
        Some(InterventionTrigger::Wake {
            wake_id: second_wake.id.clone()
        })
    );
    assert_eq!(row.chips, [InterventionChip::Sent, completed.clone()]);

    let to_lead = sent(send("task-lead".into()).await);
    let snapshot = supervisor_until(&rpc, |s| {
        s.interventions
            .first()
            .is_some_and(|row| row.chips.contains(&InterventionChip::LeadRanTurn))
    })
    .await;
    assert_eq!(
        snapshot.interventions[0].message_id.as_deref(),
        Some(to_lead.as_str())
    );
    assert_eq!(snapshot.interventions[0].step_ids.len(), 2);

    let err = refused(
        mutate(serde_json::json!({
            "op": "updateTask", "taskId": "t1", "patch": { "notes": "Use the cheaper gateway" },
            "fromChatId": supervisor,
        }))
        .await,
    );
    assert!(err.contains("the Supervisor cannot change them"), "{err}");
    assert_eq!(rig.core.workspace.task("t1").unwrap().notes, "");
    let err = refused(
        call(
            zeron_rpc::methods::QUEUE_COMMAND,
            serde_json::json!({
                "chatId": "task-lead", "command": { "kind": "interrupt" }, "fromChatId": supervisor,
            }),
        )
        .await,
    );
    assert!(err.contains("supervisor pause"), "{err}");

    if cfg!(unix) {
        use zeron_proto::supervisor::ResourceState;
        let act = |params: serde_json::Value| {
            let mut params = params;
            params["fromChatId"] = supervisor.as_str().into();
            call(zeron_rpc::methods::SUPERVISOR_ACT, params)
        };
        let pause = serde_json::json!({
            "action": "pause", "stepId": steps[1].id, "resource": port, "reason": "The frontend serves on it",
        });
        let err = refused(act(pause.clone()).await);
        assert!(err.contains("No verified contention"), "{err}");
        mutate(serde_json::json!({ "op": "createTask", "taskId": "t2", "title": "Shop frontend" }))
            .await
            .unwrap();
        mutate(serde_json::json!({
            "op": "startTask", "taskId": "t2", "chatId": "task-lead-2",
            "spaceId": "space-tt", "orchestrate": true,
        }))
        .await
        .unwrap();
        wait_for(
            || !prompts_with(&rig, "You lead this task (task id t2)").is_empty(),
            "the second lead's brief",
        )
        .await;
        mutate(serde_json::json!({
            "op": "planSteps", "taskId": "t2", "fromChatId": "task-lead-2",
            "steps": [{ "title": "Serve the shop" }], "resources": [port],
        }))
        .await
        .unwrap();
        let shared = |s: &zeron_proto::supervisor::SupervisorSnapshot, state| {
            s.resources
                .iter()
                .any(|row| row.key == port && row.state == state)
        };
        let snapshot = supervisor_until(&rpc, |s| shared(s, ResourceState::Contended)).await;
        let row = snapshot
            .resources
            .iter()
            .find(|row| row.key == port)
            .unwrap();
        assert_eq!(row.declared_by, ["t1", "t2"]);
        assert_eq!(
            row.check.as_ref().unwrap().uses[0].pid,
            std::process::id(),
            "lsof found this test's listener"
        );
        let contention = format!("Resource contention on {port}");
        let deadline = tokio::time::Instant::now() + Duration::from_secs(20);
        while !wakes(&rig).iter().any(|wake| wake.contains(&contention)) {
            assert!(
                tokio::time::Instant::now() < deadline,
                "timed out waiting for the contention wake"
            );
            tokio::time::sleep(Duration::from_millis(50)).await;
        }

        let paused = act(pause).await.unwrap();
        let zeron_rpc::RpcReply::Value(paused) = paused else {
            panic!("a pause reply");
        };
        assert_eq!(paused["stopped"], false, "the step's thread was idle");
        let err = refused(run(&steps[1], "task-lead").await);
        assert!(err.contains("step_paused"), "{err}");
        let told = "The Supervisor paused step \"Write the docs\"";
        wait_for(
            || !prompts_with(&rig, told).is_empty(),
            "the lead's pause message",
        )
        .await;
        drop(listener);
        act(serde_json::json!({ "action": "check" })).await.unwrap();
        let snapshot = supervisor_until(&rpc, |s| {
            s.interventions.first().is_some_and(|row| {
                row.chips.contains(&InterventionChip::StepPaused)
                    && row.chips.contains(&InterventionChip::ResourceFreed)
            })
        })
        .await;
        assert_eq!(snapshot.interventions[0].kind, InterventionKind::Pause);
        assert!(shared(&snapshot, ResourceState::Clear), "the port closed");
        act(serde_json::json!({
            "action": "resume", "stepId": steps[1].id, "reason": "The port is free",
        }))
        .await
        .unwrap();
        started(run(&steps[1], "task-lead").await);
    }
    rig.core.shutdown().await;
}

async fn supervisor_until(
    rpc: &zeron_engine::EngineRpc,
    ready: impl Fn(&zeron_proto::supervisor::SupervisorSnapshot) -> bool,
) -> zeron_proto::supervisor::SupervisorSnapshot {
    let zeron_rpc::RpcReply::Stream(mut watch) = zeron_rpc::RpcService::handle(
        rpc,
        zeron_rpc::methods::WATCH_SUPERVISOR,
        serde_json::json!({}),
    )
    .await
    .unwrap() else {
        panic!("a snapshot stream");
    };
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    loop {
        let item = tokio::time::timeout_at(deadline, watch.next())
            .await
            .expect("timed out waiting for the Supervisor snapshot")
            .unwrap();
        let snapshot = serde_json::from_value(item).unwrap();
        if ready(&snapshot) {
            return snapshot;
        }
    }
}

#[tokio::test]
async fn a_wake_into_a_supervisor_the_person_stopped_waits_for_their_next_message() {
    let rig = rig(Duration::from_millis(400));
    let rpc = rig.core.rpc_service();
    let call = |method: &'static str, params: serde_json::Value| {
        let rpc = rpc.clone();
        async move { zeron_rpc::RpcService::handle(&*rpc, method, params).await }
    };
    let mutate = |params| call(zeron_rpc::methods::MUTATE, params);
    let zeron_rpc::RpcReply::Value(ensured) = call(
        zeron_rpc::methods::ENSURE_SUPERVISOR,
        serde_json::json!({ "config": { "harness": "mock", "sandbox": "workspace-write" } }),
    )
    .await
    .unwrap() else {
        panic!("a chat id");
    };
    let supervisor = ensured["chatId"].as_str().unwrap().to_string();
    mutate(serde_json::json!({ "op": "createTask", "taskId": "t1", "title": "Shop backend" }))
        .await
        .unwrap();
    mutate(serde_json::json!({
        "op": "startTask", "taskId": "t1", "chatId": "task-lead",
        "spaceId": "space-tt", "orchestrate": true,
    }))
    .await
    .unwrap();
    wait_for(
        || !prompts_with(&rig, "You lead this task (task id t1)").is_empty(),
        "the lead's brief",
    )
    .await;

    queue_human_run(&rig.core, &supervisor, "cmd-h1", "msg-h1");
    wait_for(|| working(&rig.core, &supervisor), "the person's turn").await;
    queue_interrupt(&rig.core, &supervisor);
    wait_for(|| !working(&rig.core, &supervisor), "the person's Stop").await;
    mutate(serde_json::json!({
        "op": "escalateTask", "taskId": "t1", "fromChatId": "task-lead",
        "title": "Use the mock gateway?", "why": "decision", "options": ["Yes", "No"],
    }))
    .await
    .unwrap();
    let snapshot = supervisor_until(&rpc, |s| {
        s.state == zeron_proto::supervisor::SupervisorState::Stopped && s.waiting == 1
    })
    .await;
    assert_eq!(
        snapshot.wakes[0].delivery,
        zeron_proto::supervisor::WakeDelivery::HeldByStop
    );
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert!(
        prompts_with(&rig, "Supervisor wake").is_empty(),
        "a Stop holds the wake"
    );
    assert_eq!(queue_len(&rig.core, &supervisor), 1);

    queue_human_run(&rig.core, &supervisor, "cmd-h2", "msg-h2");
    wait_for(
        || !prompts_with(&rig, "Supervisor wake").is_empty(),
        "the wake after the person's next message",
    )
    .await;
    let prompts = rig.prompts.lock().unwrap().clone();
    let human = prompts
        .iter()
        .rposition(|prompt| prompt == "human prompt")
        .unwrap();
    let wake = prompts
        .iter()
        .position(|prompt| prompt.contains("Supervisor wake"))
        .unwrap();
    assert!(human < wake, "the person's message runs first: {prompts:?}");
    supervisor_until(&rpc, |s| {
        s.state != zeron_proto::supervisor::SupervisorState::Stopped
    })
    .await;
    rig.core.shutdown().await;
}

struct GatedHarness {
    prompts: Arc<Mutex<Vec<String>>>,
    gates: std::collections::HashMap<&'static str, tokio::sync::watch::Receiver<bool>>,
}

#[async_trait]
impl Harness for GatedHarness {
    fn id(&self) -> HarnessId {
        HarnessId::Mock
    }
    fn display_name(&self) -> &str {
        "Gated"
    }
    fn supports_steering(&self) -> bool {
        false
    }
    fn steering_mode(&self) -> SteeringMode {
        SteeringMode::TurnBoundary
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
        controls: RunControls,
    ) -> Result<BoxStream<'static, Result<AgentEvent, HarnessError>>, HarnessError> {
        let prompt = request.prompt.clone();
        self.prompts.lock().unwrap().push(prompt.clone());
        let step = prompt
            .lines()
            .last()
            .and_then(|line| line.strip_prefix("do: "))
            .map(str::to_string);
        let (gate, reply) = match step.as_deref() {
            Some("late") => (self.gates.get("late").cloned(), "z".repeat(4_010)),
            Some(title) => (self.gates.get(title).cloned(), format!("{title} is done")),
            None if prompt == "human prompt" => {
                (self.gates.get("lead").cloned(), "lead done".into())
            }
            None => (None, "noted".into()),
        };
        let (tx, rx) = tokio::sync::mpsc::channel::<Result<AgentEvent, HarnessError>>(8);
        tokio::spawn(async move {
            let _controls = controls;
            let _ = tx
                .send(Ok(AgentEvent::SessionStarted {
                    harness: HarnessId::Mock,
                    model: "gated-1".into(),
                    tools: vec![],
                    cwd: "/tmp".into(),
                    session_id: "hs-gated".into(),
                    assistant_message_id: "a-gated".into(),
                }))
                .await;
            if let Some(mut gate) = gate {
                let _ = gate.wait_for(|open| *open).await;
            }
            let _ = tx.send(Ok(AgentEvent::TextDelta { text: reply })).await;
            let _ = tx
                .send(Ok(AgentEvent::Done {
                    status: DoneStatus::Completed,
                    result: None,
                    error: None,
                    session_id: Some("hs-gated".into()),
                }))
                .await;
        });
        Ok(futures::stream::unfold(rx, |mut rx| async move {
            rx.recv().await.map(|event| (event, rx))
        })
        .boxed())
    }
}

#[tokio::test]
async fn step_handbacks_fold_into_one_lead_message() {
    let prompts = Arc::new(Mutex::new(Vec::new()));
    let (lead, lead_rx) = tokio::sync::watch::channel(false);
    let (second, second_rx) = tokio::sync::watch::channel(false);
    let (steps_gate, steps_rx) = tokio::sync::watch::channel(false);
    let (late, late_rx) = tokio::sync::watch::channel(false);
    let rig = rig_on(Arc::new(GatedHarness {
        prompts: prompts.clone(),
        gates: [
            ("lead", lead_rx),
            ("second", second_rx),
            ("alpha", steps_rx.clone()),
            ("beta", steps_rx),
            ("late", late_rx),
        ]
        .into_iter()
        .collect(),
    }));
    let rig = Rig { prompts, ..rig };
    let core = &rig.core;
    let rpc = core.rpc_service();
    let call = |method: &'static str, params: serde_json::Value| {
        let rpc = rpc.clone();
        async move { zeron_rpc::RpcService::handle(&*rpc, method, params).await }
    };
    let mutate = |params| call(zeron_rpc::methods::MUTATE, params);
    mutate(serde_json::json!({ "op": "createTask", "taskId": "t1", "title": "Shop backend" }))
        .await
        .unwrap();
    mutate(serde_json::json!({
        "op": "startTask", "taskId": "t1", "chatId": "task-lead",
        "spaceId": "space-tt", "orchestrate": true,
    }))
    .await
    .unwrap();
    mutate(serde_json::json!({
        "op": "planSteps", "taskId": "t1", "fromChatId": "task-lead",
        "steps": [
            { "title": "first" }, { "title": "second" },
            { "title": "alpha" }, { "title": "beta" }, { "title": "late" },
        ],
    }))
    .await
    .unwrap();
    mutate(serde_json::json!({ "op": "approvePlan", "taskId": "t1" }))
        .await
        .unwrap();
    wait_for(
        || !prompts_with(&rig, "The plan is approved. Run it.").is_empty(),
        "the lead's approval message",
    )
    .await;
    wait_for(|| idle(core, "task-lead"), "the lead's approval turn").await;

    let steps = core.workspace.task_steps_of("t1");
    let run = |step: &zeron_proto::TaskStep| {
        let reply = call(
            zeron_rpc::methods::CREATE_THREAD,
            serde_json::json!({
                "fromChatId": "task-lead", "prompt": format!("do: {}", step.title), "taskStepId": step.id,
            }),
        );
        let title = step.title.clone();
        async move {
            let Ok(zeron_rpc::RpcReply::Value(value)) = reply.await else {
                panic!("a step thread for {title}");
            };
            value["chatId"].as_str().unwrap().to_string()
        }
    };
    let step_rows = || {
        core.doc_host
            .open("task-lead")
            .unwrap()
            .doc()
            .read_queue()
            .unwrap()
            .into_iter()
            .filter(|row| !row.task_steps.is_empty())
            .collect::<Vec<_>>()
    };
    let handbacks = |rig: &Rig| prompts_with(rig, "(taskStepId ");

    let first = run(&steps[0]).await;
    let second_thread = run(&steps[1]).await;
    wait_for(
        || {
            step_rows()
                .first()
                .is_some_and(|row| row.task_steps.len() == 1)
        },
        "the first step's handback held in the idle lead's queue",
    )
    .await;
    second.send(true).unwrap();
    wait_for(|| !handbacks(&rig).is_empty(), "the idle lead's handback").await;
    wait_for(|| idle(core, "task-lead"), "the lead's handback turn").await;
    let early = handbacks(&rig);
    assert_eq!(
        early.len(),
        1,
        "steps settling apart reach an idle lead once: {early:?}"
    );
    for (step, thread) in steps.iter().zip([&first, &second_thread]) {
        assert!(
            early[0].contains(&format!(
                "Step \"{}\" (taskStepId {}) finished a turn on its thread {thread}.\n\n<agent-response>\n{} is done\n</agent-response>",
                step.title, step.id, step.title
            )),
            "{}",
            early[0]
        );
    }

    queue_human_run(core, "task-lead", "cmd-1", "msg-1");
    wait_for(|| working(core, "task-lead"), "the lead's held turn").await;
    let mut threads = Vec::new();
    for step in &steps[2..] {
        threads.push(run(step).await);
    }
    steps_gate.send(true).unwrap();
    wait_for(
        || {
            step_rows()
                .first()
                .is_some_and(|row| row.task_steps.len() == 2)
        },
        "alpha and beta in one queued handback",
    )
    .await;
    let row = step_rows().remove(0);
    let mut both = row.task_steps.clone();
    both.sort();
    let mut expected = vec![steps[2].id.clone(), steps[3].id.clone()];
    expected.sort();
    assert_eq!(both, expected);

    late.send(true).unwrap();
    wait_for(
        || {
            step_rows()
                .first()
                .is_some_and(|row| row.task_steps.len() == 3)
        },
        "the late step joining the queued handback",
    )
    .await;
    let joined = step_rows();
    assert_eq!(joined.len(), 1, "one handback row for the lead");
    assert_eq!(joined[0].id, row.id, "the late step joined the queued row");

    lead.send(true).unwrap();
    wait_for(
        || !prompts_with(&rig, &format!("(taskStepId {})", steps[4].id)).is_empty(),
        "the folded handback's turn",
    )
    .await;
    wait_for(|| idle(core, "task-lead"), "the lead's handback turn").await;
    let all = handbacks(&rig);
    assert_eq!(all.len(), 2, "one lead message per fold: {all:?}");
    let message = &all[1];
    for (step, thread) in steps[2..].iter().zip(&threads).take(2) {
        assert!(
            message.contains(&format!(
                "Step \"{}\" (taskStepId {}) finished a turn on its thread {thread}.\n\n<agent-response>\n{} is done\n</agent-response>",
                step.title, step.id, step.title
            )),
            "{message}"
        );
    }
    assert!(
        message.contains(&format!(
            "Step \"late\" (taskStepId {}) finished a turn on its thread {}.\n\n<agent-response>\n{}\n[truncated 10 chars; use read_thread for the full response]\n</agent-response>",
            steps[4].id,
            threads[2],
            "z".repeat(4_000)
        )),
        "{message}"
    );
    assert!(message.ends_with(
        "Read them with read_thread, then mark each step done or skipped with the tasks tool, action \"step\", or tell its thread what is still missing with send_message_to_thread."
    ));
    let woken: Vec<_> = entries(core, "task-lead")
        .into_iter()
        .filter(|entry| entry.id == row.id)
        .collect();
    assert_eq!(woken.len(), 1);
    assert!(
        threads.contains(woken[0].from_chat_id.as_ref().unwrap()),
        "the handback reads as a message from a step thread"
    );
    core.shutdown().await;
}
