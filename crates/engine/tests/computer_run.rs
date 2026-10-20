use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use futures::StreamExt;
use futures::stream::BoxStream;
use serde_json::json;

use zeron_doc::{
    MessagePart, MessageRole, MessageStatus, SessionCommandEntry, SessionCommandPayload,
    SessionCommandStatus, SessionMessageEntry,
};
use zeron_engine::computer::manager::{COMPUTER_CONTROL_ON_DISCLOSURE, ComputerCaller};
use zeron_engine::computer::runtime::{COMPUTER_DESKTOP_REVOKED_NOTE, COMPUTER_ESCAPE_STOP_NOTICE};
use zeron_engine::{EngineCore, HarnessRegistry, SteerOutcome};
use zeron_harness::{CancellationToken, Harness, HarnessError, RunControls};
use zeron_proto::computer::{
    ComputerConsentScope, ComputerHostMethod, ComputerHostStreamItem, ComputerPendingKind,
    ComputerSettings,
};
use zeron_proto::{
    AgentEvent, DoneStatus, HarnessId, Model, PermissionMode, ReasoningLevel, RunRequest,
    SandboxLevel, SessionStatus, SteeringMode, UserInputAnswer, UserInputKind, UserInputQuestion,
};
use zeron_rpc::methods;

const CHAT: &str = "chat-cu";
const LEAD: &str = "lead";

/// Every run carries the delegation guidance (d67); only a computer run
/// carries the computer chapter.
fn no_computer_guidance(seen: &Seen) -> bool {
    seen.guidance
        .as_deref()
        .is_none_or(|text| !text.contains("## Zeron computer use"))
}

#[derive(Debug, Clone, PartialEq)]
struct Seen {
    prompt: String,
    computer: bool,
    resumed: bool,
    secret: Option<String>,
    guidance: Option<String>,
    steer: bool,
}

#[derive(Default)]
struct RunCtl {
    finish: tokio::sync::Notify,
    exit: tokio::sync::Notify,
    speak: tokio::sync::Notify,
}

#[derive(Default)]
struct Probe {
    seen: Mutex<Vec<Seen>>,
    runs: Mutex<Vec<Arc<RunCtl>>>,
    resume: Option<bool>,
    swallow_steers: std::sync::atomic::AtomicBool,
    tools_unreachable: std::sync::atomic::AtomicBool,
}

struct ComputerHarness(Arc<Probe>);

fn done(status: DoneStatus) -> AgentEvent {
    AgentEvent::Done {
        status,
        result: None,
        error: None,
        session_id: Some("hs-1".into()),
    }
}

#[async_trait]
impl Harness for ComputerHarness {
    fn id(&self) -> HarnessId {
        HarnessId::Mock
    }
    fn display_name(&self) -> &str {
        "Computer probe"
    }
    fn supports_steering(&self) -> bool {
        true
    }
    fn steering_mode(&self) -> SteeringMode {
        SteeringMode::StepBoundary
    }
    fn resume_supported(&self) -> Option<bool> {
        self.0.resume.or(Some(true))
    }
    fn computer_tools_reachable(&self) -> bool {
        !self
            .0
            .tools_unreachable
            .load(std::sync::atomic::Ordering::SeqCst)
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
        let secret = controls.tool_server.as_ref().and_then(|server| {
            server
                .env
                .iter()
                .find(|(name, _)| name == zeron_rpc::COMPUTER_RUN_SECRET_ENV)
                .map(|(_, value)| value.clone())
        });
        let computer = request.computer_control;
        let guidance = controls
            .tool_server
            .as_ref()
            .and_then(|server| server.guidance.clone());
        self.0.seen.lock().unwrap().push(Seen {
            prompt: request.prompt.clone(),
            computer,
            resumed: request.resume.is_some(),
            secret,
            guidance,
            steer: false,
        });
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        let probe = self.0.clone();
        let ctl = Arc::new(RunCtl::default());
        self.0.runs.lock().unwrap().push(ctl.clone());
        let ask_empty = request.prompt == "ask-empty";
        let ask_withdrawn = request.prompt == "ask-withdrawn";
        let ask_kept = request.prompt == "ask-kept";
        tokio::spawn(async move {
            let _ = tx.send(Ok(AgentEvent::SessionStarted {
                harness: HarnessId::Mock,
                model: "probe".into(),
                tools: vec![],
                cwd: "/tmp".into(),
                session_id: "hs-1".into(),
                assistant_message_id: "a-1".into(),
            }));
            let _ = tx.send(Ok(AgentEvent::TextDelta {
                text: "working".into(),
            }));
            let _asked = ask_empty.then(|| (controls.request_input)(Vec::new()));
            let _open = ask_kept.then(|| (controls.request_input)(vec![plain_question("open")]));
            let _kept = ask_withdrawn.then(|| {
                drop((controls.request_input)(vec![plain_question("withdrawn")]));
                (controls.request_input)(vec![plain_question("kept")])
            });
            loop {
                tokio::select! {
                    _ = controls.interrupt.cancelled() => {
                        let _ = tx.send(Ok(done(DoneStatus::Interrupted)));
                        break;
                    }
                    _ = ctl.finish.notified() => {
                        let _ = tx.send(Ok(done(DoneStatus::Completed)));
                    }
                    _ = ctl.exit.notified() => {
                        let _ = tx.send(Ok(done(DoneStatus::Completed)));
                        break;
                    }
                    _ = ctl.speak.notified() => {
                        let _ = tx.send(Ok(AgentEvent::TextDelta { text: "self-continued".into() }));
                    }
                    Some(message) = controls.steering.recv() => {
                        probe.seen.lock().unwrap().push(Seen {
                            prompt: message.prompt,
                            computer,
                            resumed: false,
                            secret: None,
                            guidance: None,
                            steer: true,
                        });
                        if probe.swallow_steers.load(std::sync::atomic::Ordering::SeqCst) {
                            continue;
                        }
                        let _ = tx.send(Ok(AgentEvent::Steered {
                            assistant_message_id: None,
                            next_assistant_message_id: None,
                        }));
                        let _ = tx.send(Ok(AgentEvent::TextDelta { text: "steered".into() }));
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

fn plain_question(id: &str) -> UserInputQuestion {
    UserInputQuestion {
        approval: false,
        id: id.into(),
        header: "Question".into(),
        question: format!("{id}?"),
        options: vec!["Yes".into(), "No".into()],
        multi_select: false,
        kind: None,
        prefill: None,
        multiline: false,
    }
}

struct Rig {
    core: EngineCore,
    probe: Arc<Probe>,
    _dir: tempfile::TempDir,
}

fn rig_with(resume: Option<bool>) -> Rig {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("device-id"), "dev-a").unwrap();
    rig_in(dir, resume)
}

fn rig_in(dir: tempfile::TempDir, resume: Option<bool>) -> Rig {
    let probe = Arc::new(Probe {
        resume,
        ..Probe::default()
    });
    let registry = HarnessRegistry::new();
    registry.register(Arc::new(ComputerHarness(probe.clone())));
    let core = EngineCore::assemble(dir.path(), Arc::new(registry), HarnessId::Mock, None)
        .expect("engine core assembles");
    core.workspace
        .create_space("space-cu", "dev-a", "/tmp", None, false)
        .expect("space");
    for chat in [CHAT, LEAD] {
        core.workspace
            .create_chat(chat, Some("space-cu"), None, None, None)
            .expect("chat");
    }
    core.sessions.set_tool_server_port(1);
    Rig {
        core,
        probe,
        _dir: dir,
    }
}

fn rig() -> Rig {
    rig_with(None)
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

fn request(prompt: &str, mode: PermissionMode) -> RunRequest {
    RunRequest {
        prompt: prompt.into(),
        harness: None,
        model: None,
        reasoning: None,
        model_options: Default::default(),
        cwd: "/tmp".into(),
        sandbox: SandboxLevel::WorkspaceWrite,
        permission_mode: mode,
        auto_approve: false,
        attachments: Vec::new(),
        worktree: None,
        branch: None,
        resume: None,
        forked_at: None,
        computer_control: false,
        mcp: None,
        disabled_tools: Vec::new(),
    }
}

impl Rig {
    fn seen(&self) -> Vec<Seen> {
        self.probe.seen.lock().unwrap().clone()
    }

    async fn saw(&self, count: usize) -> Seen {
        wait_for(|| self.seen().len() >= count, "the harness to see a prompt").await;
        self.seen()[count - 1].clone()
    }

    fn command(&self, id: &str, payload: SessionCommandPayload, generation: Option<u64>) {
        self.core
            .doc_host
            .open(CHAT)
            .unwrap()
            .doc()
            .queue_command(&SessionCommandEntry {
                id: id.into(),
                payload,
                issued_by: "viewer".into(),
                issued_at: chrono::Utc::now().timestamp_millis(),
                based_on: None,
                expires_at: None,
                status: SessionCommandStatus::Pending,
                resolution: None,
                computer_generation: generation,
            })
            .expect("queue command");
    }

    fn run(&self, id: &str, prompt: &str, mode: PermissionMode, generation: Option<u64>) {
        self.command(
            id,
            SessionCommandPayload::Run {
                request: request(prompt, mode),
                message_id: format!("m-{id}"),
            },
            generation,
        );
    }

    async fn applied(&self, id: &str) -> (SessionCommandStatus, Option<String>) {
        let read = || {
            self.core
                .doc_host
                .open(CHAT)
                .unwrap()
                .doc()
                .read_commands()
                .unwrap()
                .into_iter()
                .find(|entry| entry.id == id && entry.status != SessionCommandStatus::Pending)
        };
        wait_for(|| read().is_some(), "the command to settle").await;
        let entry = read().unwrap();
        (entry.status, entry.resolution)
    }

    fn in_flight(&self) -> bool {
        self.core.sessions.turn_in_flight(CHAT)
    }

    fn finish(&self) {
        self.ctl().finish.notify_one();
    }

    fn ctl(&self) -> Arc<RunCtl> {
        self.probe.runs.lock().unwrap().last().unwrap().clone()
    }

    async fn finish_turn(&self) {
        self.finish();
        wait_for(|| !self.in_flight(), "the turn to end").await;
    }

    fn settings(&self, on: bool) {
        self.core
            .computer
            .set_settings(ComputerSettings {
                computer_control_enabled: on,
                ..ComputerSettings::default()
            })
            .unwrap();
    }

    fn entries(&self, chat: &str) -> Vec<SessionMessageEntry> {
        self.core
            .doc_host
            .open(chat)
            .unwrap()
            .doc()
            .read_entries()
            .unwrap()
    }

    fn user_texts(&self) -> Vec<String> {
        self.entries(CHAT)
            .iter()
            .filter(|entry| entry.role == MessageRole::User)
            .map(entry_text)
            .collect()
    }

    fn generation(&self) -> u64 {
        self.core
            .doc_host
            .open(CHAT)
            .unwrap()
            .doc()
            .computer_generation()
    }

    fn system_notes(&self) -> Vec<String> {
        self.entries(CHAT)
            .iter()
            .filter(|entry| entry.role == MessageRole::System)
            .map(entry_text)
            .collect()
    }

    fn revoked_notes(&self) -> Vec<(String, Option<String>)> {
        self.system_notes()
            .into_iter()
            .filter_map(|note| {
                let mut lines = note.splitn(2, '\n');
                let text = lines.next()?.to_owned();
                (text == COMPUTER_DESKTOP_REVOKED_NOTE)
                    .then(|| (text, lines.next().map(str::to_owned)))
            })
            .collect()
    }

    fn consent_cards(&self) -> Vec<(String, ComputerConsentScope, bool)> {
        self.entries(CHAT)
            .iter()
            .flat_map(|entry| entry.parts.clone())
            .filter_map(|part| match part {
                MessagePart::Input {
                    request_id,
                    questions,
                    resolved,
                    ..
                } => match questions.first().and_then(|q| q.kind.clone()) {
                    Some(UserInputKind::Consent { scope, .. }) => {
                        Some((request_id, scope, resolved))
                    }
                    _ => None,
                },
                _ => None,
            })
            .collect()
    }
}

fn entry_text(entry: &SessionMessageEntry) -> String {
    entry
        .parts
        .iter()
        .filter_map(|part| match part {
            MessagePart::Text { text, .. } => Some(text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[tokio::test]
async fn settings_off_activates_only_the_user_authored_slash_turn_and_restarts_with_resume() {
    let rig = rig();
    rig.run("c1", "plain", PermissionMode::Full, None);
    let first = rig.saw(1).await;
    assert!(!first.computer && no_computer_guidance(&first));
    rig.finish_turn().await;

    rig.run(
        "c2",
        "/computer-use open Calculator",
        PermissionMode::Full,
        None,
    );
    let second = rig.saw(2).await;
    assert_eq!(second.prompt, "open Calculator");
    assert!(second.computer && second.resumed && !second.steer);
    assert!(
        second
            .guidance
            .as_deref()
            .is_some_and(|text| text.starts_with("## Zeron computer use\n"))
    );
    let (old, new) = (first.secret.unwrap(), second.secret.unwrap());
    assert_ne!(old, new);
    assert_eq!(rig.core.sessions.computer_run(CHAT, &old), None);
    let live = rig.core.sessions.computer_run(CHAT, &new).unwrap();
    assert!(live.run_computer_control && live.turn_computer_control);
    rig.finish_turn().await;

    rig.run("c3", "plain again", PermissionMode::Full, None);
    let third = rig.saw(3).await;
    assert!(!third.computer && third.resumed && !third.steer && no_computer_guidance(&third));
    rig.finish_turn().await;
    assert_eq!(
        rig.user_texts(),
        vec!["plain", "/computer-use open Calculator", "plain again"]
    );
    assert!(
        rig.entries(CHAT)
            .iter()
            .filter(|entry| entry.role == MessageRole::Assistant)
            .count()
            >= 3
    );
    rig.core.shutdown().await;
}

#[tokio::test]
async fn an_agent_delivered_slash_never_activates_while_settings_on_covers_every_turn() {
    let rig = rig();
    rig.core
        .doc_host
        .queue_thread_message(CHAT, Some(LEAD), "/computer-use open Calculator")
        .await
        .unwrap();
    let delivered = rig.saw(1).await;
    assert!(delivered.prompt.contains("/computer-use open Calculator"));
    assert!(!delivered.computer);
    rig.finish_turn().await;

    rig.settings(true);
    let peer = rig
        .core
        .doc_host
        .create_thread(
            LEAD,
            "check the app",
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
    let created = rig.saw(2).await;
    assert!(created.prompt.contains("check the app"));
    assert!(created.computer, "a create_thread turn gets the catalog");
    rig.finish();
    wait_for(
        || !rig.core.sessions.turn_in_flight(&peer),
        "the peer turn to end",
    )
    .await;

    rig.run("c1", "plain", PermissionMode::Full, None);
    let plain = rig.saw(3).await;
    assert!(plain.computer && !plain.steer);
    rig.finish_turn().await;
    rig.core.shutdown().await;
}

#[tokio::test]
async fn a_stop_fences_a_queued_slash_row_and_the_next_message_does_not_revive_it() {
    let rig = rig();
    rig.run("c1", "plain", PermissionMode::Full, None);
    rig.saw(1).await;
    let row = rig
        .core
        .doc_host
        .queue_message_with_behavior(CHAT, "/computer-use open Calculator", vec![], false)
        .unwrap();
    let queue = rig
        .core
        .doc_host
        .open(CHAT)
        .unwrap()
        .doc()
        .read_queue()
        .unwrap();
    assert_eq!(
        queue
            .iter()
            .find(|item| item.id == row)
            .unwrap()
            .computer_generation,
        Some(0)
    );

    rig.command("stop", SessionCommandPayload::Interrupt { teardown: false }, None);
    rig.applied("stop").await;
    assert!(!rig.in_flight());
    assert_eq!(rig.generation(), 1);

    rig.run("c2", "unrelated", PermissionMode::Full, Some(1));
    let unrelated = rig.saw(2).await;
    assert!(!unrelated.computer);
    rig.finish();
    let replayed = rig.saw(3).await;
    assert_eq!(replayed.prompt, "/computer-use open Calculator");
    assert!(replayed.steer && !replayed.computer);
    assert!(rig.seen().iter().all(|seen| !seen.computer));
    rig.finish_turn().await;
    rig.core.shutdown().await;
}

#[tokio::test]
async fn with_settings_on_a_turn_queued_before_stop_keeps_the_catalog() {
    let rig = rig();
    rig.settings(true);
    rig.run("c1", "plain", PermissionMode::Full, None);
    assert!(rig.saw(1).await.computer);
    rig.core
        .doc_host
        .queue_message_with_behavior(CHAT, "queued follow-up", vec![], false)
        .unwrap();
    rig.command("stop", SessionCommandPayload::Interrupt { teardown: false }, None);
    rig.applied("stop").await;
    rig.run("c2", "unrelated", PermissionMode::Full, Some(1));
    assert!(rig.saw(2).await.computer);
    rig.finish();
    let replayed = rig.saw(3).await;
    assert_eq!(replayed.prompt, "queued follow-up");
    assert!(replayed.computer);
    rig.finish_turn().await;
    rig.core.shutdown().await;
}

#[tokio::test]
async fn a_stop_before_any_dispatch_in_this_engine_lifetime_still_fences() {
    let rig = rig();
    rig.core
        .doc_host
        .open(CHAT)
        .unwrap()
        .doc()
        .set_computer_generation(3)
        .unwrap();
    rig.command("stop", SessionCommandPayload::Interrupt { teardown: false }, None);
    rig.applied("stop").await;
    assert_eq!(rig.generation(), 4);

    rig.run(
        "c1",
        "/computer-use open Calculator",
        PermissionMode::Full,
        Some(3),
    );
    let stale = rig.saw(1).await;
    assert_eq!(stale.prompt, "/computer-use open Calculator");
    assert!(!stale.computer, "a row stamped before the Stop runs off");
    rig.finish_turn().await;

    rig.run(
        "c2",
        "/computer-use open Calculator",
        PermissionMode::Full,
        Some(4),
    );
    assert!(rig.saw(2).await.computer);
    rig.finish_turn().await;
    rig.core.shutdown().await;
}

#[tokio::test]
async fn a_mid_turn_flip_waits_for_the_boundary_and_is_never_steered() {
    let rig = rig();
    rig.run("c1", "plain", PermissionMode::Full, None);
    rig.saw(1).await;
    assert_eq!(
        rig.core
            .sessions
            .steer_with(CHAT, "/computer-use open Calculator", &[], None, Some(true))
            .await
            .unwrap(),
        SteerOutcome::NotSteerable
    );
    rig.command(
        "steer",
        SessionCommandPayload::Steer {
            prompt: "/computer-use open Calculator".into(),
            message_id: Some("m-steer".into()),
        },
        Some(0),
    );
    let (status, resolution) = rig.applied("steer").await;
    assert_eq!(status, SessionCommandStatus::Applied);
    assert_eq!(resolution.as_deref(), Some("held until the turn ends"));
    rig.run(
        "c2",
        "/computer-use open Safari",
        PermissionMode::Full,
        Some(0),
    );
    let (_, resolution) = rig.applied("c2").await;
    assert_eq!(resolution.as_deref(), Some("held until the turn ends"));
    assert_eq!(rig.seen().len(), 1, "nothing reached the live plain run");
    assert!(rig.in_flight());

    rig.finish();
    let restarted = rig.saw(2).await;
    assert_eq!(restarted.prompt, "open Calculator");
    assert!(restarted.computer && restarted.resumed && !restarted.steer);
    rig.core.shutdown().await;
}

#[tokio::test]
async fn an_agent_that_cannot_reload_its_session_refuses_the_flip_visibly() {
    let rig = rig_with(Some(false));
    rig.run("c1", "plain", PermissionMode::Full, None);
    rig.saw(1).await;
    rig.finish_turn().await;
    rig.run(
        "c2",
        "/computer-use open Calculator",
        PermissionMode::Full,
        None,
    );
    let steered = rig.saw(2).await;
    assert!(steered.steer && !steered.computer);
    assert_eq!(steered.prompt, "/computer-use open Calculator");
    assert!(
        rig.system_notes()
            .iter()
            .any(|note| note.starts_with("Computer is unavailable for this turn"))
    );
    rig.core.shutdown().await;
}

#[tokio::test]
async fn an_agent_that_cannot_see_the_computer_tools_refuses_computer_visibly() {
    let rig = rig();
    rig.probe
        .tools_unreachable
        .store(true, std::sync::atomic::Ordering::SeqCst);
    rig.run(
        "c1",
        "/computer-use open Calculator",
        PermissionMode::Full,
        None,
    );
    let seen = rig.saw(1).await;
    assert!(!seen.computer, "no catalog for an agent that cannot see it");
    assert_eq!(seen.prompt, "/computer-use open Calculator");
    assert!(
        no_computer_guidance(&seen),
        "no guidance for tools it cannot see"
    );
    assert!(
        rig.system_notes()
            .iter()
            .any(|note| note.starts_with("Computer is unavailable for this turn"))
    );
    rig.core.shutdown().await;
}

#[tokio::test]
async fn the_run_secret_names_only_the_live_run_and_is_revoked_by_a_stop() {
    let rig = rig();
    rig.run("c1", "plain", PermissionMode::Ask, None);
    let secret = rig.saw(1).await.secret.expect("the run carries a secret");
    let live = rig.core.sessions.computer_run(CHAT, &secret).unwrap();
    assert_eq!(live.permission_mode, PermissionMode::Ask);
    assert_eq!(live.turn_id.as_deref(), Some("m-c1"));
    assert!(!live.run_computer_control);
    assert_eq!(rig.core.sessions.computer_run(CHAT, "forged"), None);
    assert_eq!(rig.core.sessions.computer_run(LEAD, &secret), None);
    assert_eq!(
        rig.core
            .sessions
            .computer_run(CHAT, &secret[..secret.len() - 1]),
        None
    );
    rig.command("stop", SessionCommandPayload::Interrupt { teardown: false }, None);
    rig.applied("stop").await;
    assert_eq!(rig.core.sessions.computer_run(CHAT, &secret), None);
    rig.core.shutdown().await;
}

async fn manager(rig: &Rig) -> Arc<zeron_engine::computer::manager::ComputerManager> {
    wait_for(
        || rig.core.computer.service().is_some(),
        "the computer service",
    )
    .await;
    rig.core.computer.service().unwrap().manager.clone()
}

async fn caller(rig: &Rig) -> ComputerCaller {
    let secret = rig.seen()[0].secret.clone().unwrap();
    let run = rig.core.sessions.computer_run(CHAT, &secret).unwrap();
    ComputerCaller {
        chat_id: CHAT.into(),
        turn_id: run.turn_id,
    }
}

fn authorize(
    manager: &Arc<zeron_engine::computer::manager::ComputerManager>,
    tool: &str,
    caller: &ComputerCaller,
) -> tokio::task::JoinHandle<bool> {
    let (manager, tool, caller) = (manager.clone(), tool.to_owned(), caller.clone());
    tokio::spawn(async move {
        manager
            .authorize_action(
                &tool,
                &serde_json::Map::new(),
                &caller,
                CancellationToken::new(),
            )
            .await
            .unwrap_or(false)
    })
}

async fn pending_consent(
    manager: &zeron_engine::computer::manager::ComputerManager,
    input_id: &str,
) -> (ComputerConsentScope, Option<String>) {
    wait_for(
        || {
            manager
                .pending_request(CHAT)
                .is_some_and(|pending| pending.input_id == input_id)
        },
        "the pending consent request",
    )
    .await;
    let pending = manager.pending_request(CHAT).unwrap();
    assert_eq!(pending.question_id, "consent");
    match pending.kind {
        ComputerPendingKind::Consent { scope, tool, .. } => (scope, tool),
        other => panic!("a consent request, got {other:?}"),
    }
}

async fn no_pending(manager: &zeron_engine::computer::manager::ComputerManager, what: &str) {
    wait_for(|| manager.pending_request(CHAT).is_none(), what).await;
}

fn answer(rig: &Rig, id: &str, request_id: &str, label: &str) {
    rig.command(
        id,
        SessionCommandPayload::RespondInput {
            request_id: request_id.into(),
            answers: vec![UserInputAnswer {
                question_id: "consent".into(),
                labels: vec![label.into()],
            }],
        },
        None,
    );
}

#[tokio::test]
async fn ask_mode_raises_one_task_card_a_clipboard_card_and_never_discloses_in_the_transcript() {
    let rig = rig();
    rig.run("c1", "plain", PermissionMode::Ask, None);
    rig.saw(1).await;
    let manager = manager(&rig).await;
    let caller = caller(&rig).await;

    let click = authorize(&manager, "computer_click", &caller);
    wait_for(|| rig.consent_cards().len() == 1, "the task card").await;
    let (card, scope, resolved) = rig.consent_cards()[0].clone();
    assert_eq!(scope, ComputerConsentScope::Task);
    assert!(!resolved);
    assert_eq!(
        rig.core.sessions.session_status(CHAT).unwrap().status,
        SessionStatus::Working,
        "a consent card never parks the composer as awaiting input"
    );
    assert_eq!(
        pending_consent(&manager, &card).await,
        (ComputerConsentScope::Task, Some("computer_click".into())),
        "the strip and menu bar get the card's request"
    );
    assert!(!rig.core.computer.settings().approved_once);
    answer(&rig, "allow", &card, "Allow");
    assert!(click.await.unwrap());
    no_pending(&manager, "an answer clears the pending request").await;
    wait_for(
        || rig.core.computer.settings().approved_once,
        "the first approval to be remembered for the one-time disclosure",
    )
    .await;
    let answered = || {
        rig.entries(CHAT)
            .iter()
            .flat_map(|e| e.parts.clone())
            .find_map(|part| match part {
                MessagePart::Input {
                    request_id,
                    resolved: true,
                    answers: Some(answers),
                    ..
                } if request_id == card => Some(answers),
                _ => None,
            })
    };
    wait_for(|| answered().is_some(), "the answered card in the doc").await;
    assert_eq!(answered().unwrap()[0].labels, vec!["Allow".to_string()]);

    assert!(
        authorize(&manager, "computer_type_text", &caller)
            .await
            .unwrap()
    );
    assert_eq!(rig.consent_cards().len(), 1, "one task card per turn");

    let clipboard = authorize(&manager, "computer_read_clipboard", &caller);
    wait_for(|| rig.consent_cards().len() == 2, "the clipboard card").await;
    let (clip_card, clip_scope, _) = rig.consent_cards()[1].clone();
    assert_eq!(clip_scope, ComputerConsentScope::Call);
    assert_eq!(
        pending_consent(&manager, &clip_card).await,
        (
            ComputerConsentScope::Call,
            Some("computer_read_clipboard".into())
        )
    );
    answer(&rig, "decline", &clip_card, "Decline");
    assert!(!clipboard.await.unwrap());
    no_pending(&manager, "a decline clears the pending request").await;

    assert!(
        !rig.system_notes()
            .iter()
            .any(|note| note.as_str() == COMPUTER_CONTROL_ON_DISCLOSURE),
        "the ON disclosure rides the first tool result, never a transcript note"
    );
    rig.finish_turn().await;
    rig.core.shutdown().await;
}

#[tokio::test]
async fn auto_and_full_raise_no_card_and_never_disclose_in_the_transcript() {
    for mode in [PermissionMode::Auto, PermissionMode::Full] {
        let rig = rig();
        rig.run("c1", "plain", mode, None);
        rig.saw(1).await;
        let manager = manager(&rig).await;
        let caller = caller(&rig).await;
        assert!(
            authorize(&manager, "computer_click", &caller)
                .await
                .unwrap()
        );
        assert!(
            authorize(&manager, "computer_read_clipboard", &caller)
                .await
                .unwrap()
        );
        assert!(rig.consent_cards().is_empty(), "{mode:?}");
        assert!(
            !rig.system_notes()
                .iter()
                .any(|note| note.as_str() == COMPUTER_CONTROL_ON_DISCLOSURE),
            "{mode:?}"
        );
        rig.finish_turn().await;
        rig.core.shutdown().await;
    }
}

#[tokio::test]
async fn a_consent_card_left_open_by_a_dead_run_is_never_answered_as_a_new_turn() {
    let rig = rig();
    let consent = UserInputQuestion {
        approval: false,
        id: "consent".into(),
        header: "Computer".into(),
        question: "Allow Computer for this task?".into(),
        options: vec!["Allow".into(), "Decline".into()],
        multi_select: false,
        kind: Some(UserInputKind::Consent {
            scope: ComputerConsentScope::Task,
            app: None,
            tool: None,
            params: None,
        }),
        prefill: None,
        multiline: false,
    };
    let visible = UserInputQuestion {
        approval: false,
        id: "q1".into(),
        header: "Foreground".into(),
        question: "Can I bring Safari to the front?".into(),
        options: vec!["Yes".into(), "No".into()],
        multi_select: false,
        kind: None,
        prefill: None,
        multiline: false,
    };
    let doc = rig.core.doc_host.open(CHAT).unwrap();
    for (id, question) in [("r-consent", consent), ("r-visible", visible)] {
        doc.doc()
            .push_message(&SessionMessageEntry {
                id: format!("a-{id}"),
                role: MessageRole::Assistant,
                parts: vec![MessagePart::Input {
                    id: id.into(),
                    request_id: id.into(),
                    questions: vec![question],
                    resolved: false,
                    answers: None,
                    asked_at_ms: None,
                    answered_at_ms: None,
                }],
                created_at: 1,
                device_id: "dev-a".into(),
                status: Some(MessageStatus::Aborted),
                continuation_of: None,
                from_chat_id: None,
                fork_import: false,
                answers_request_id: None,
                duration_ms: None,
            })
            .unwrap();
    }

    answer(&rig, "late-consent", "r-consent", "Allow");
    assert_eq!(
        rig.applied("late-consent").await.0,
        SessionCommandStatus::Rejected
    );
    assert!(rig.seen().is_empty());

    rig.command(
        "late-answer",
        SessionCommandPayload::RespondInput {
            request_id: "r-visible".into(),
            answers: vec![UserInputAnswer {
                question_id: "q1".into(),
                labels: vec!["No".into()],
            }],
        },
        None,
    );
    let (status, resolution) = rig.applied("late-answer").await;
    assert_eq!(status, SessionCommandStatus::Applied);
    assert_eq!(resolution.as_deref(), Some("answered as new turn"));
    rig.saw(1).await;
    let entries = rig.entries(CHAT);
    let marked = entries
        .iter()
        .find(|entry| entry.answers_request_id.as_deref() == Some("r-visible"))
        .expect("the fallback's user entry names the request it answers");
    assert_eq!(marked.role, MessageRole::User);
    assert!(
        entries
            .iter()
            .flat_map(|e| e.parts.iter())
            .any(|part| matches!(
                part,
                MessagePart::Input { request_id, resolved: true, answers: Some(answers), .. }
                    if request_id == "r-visible" && answers[0].labels == ["No"]
            ))
    );
    let walk = zeron_engine::computer::visible_use::visible_use_entries(&entries);
    assert!(
        !zeron_engine::computer::visible_use::computer_foreground_authorization_for_messages(
            &walk,
            Default::default(),
        )
        .user_requested_visible_use
    );
    rig.finish_turn().await;
    rig.core.shutdown().await;
}

#[tokio::test]
async fn a_physical_escape_revokes_the_desktop_and_the_turn_keeps_running() {
    let rig = rig();
    rig.settings(true);
    rig.run("c1", "plain", PermissionMode::Full, None);
    let secret = rig.saw(1).await.secret.unwrap();
    let manager = manager(&rig).await;
    let (release, released) = tokio::sync::oneshot::channel::<()>();
    let (entered_tx, entered) = tokio::sync::oneshot::channel::<()>();
    let activity = {
        let manager = manager.clone();
        tokio::spawn(async move {
            manager
                .with_agent_activity(
                    CHAT,
                    move || async move {
                        let _ = entered_tx.send(());
                        let _ = released.await;
                        Ok(())
                    },
                    None,
                    Some("m-c1"),
                    None,
                )
                .await
        })
    };
    entered.await.unwrap();

    let client = zeron_rpc::memory_client(rig.core.rpc_service());
    let reply = client
        .call(methods::COMPUTER_EMERGENCY_STOP, json!({}))
        .await
        .unwrap();
    assert_eq!(reply, json!({ "stoppedChatIds": [CHAT] }));
    assert!(rig.in_flight(), "the turn keeps running to sum up");
    assert!(rig.core.sessions.computer_turn_live(CHAT));
    assert_eq!(rig.generation(), 1);
    assert_eq!(
        rig.revoked_notes(),
        vec![(COMPUTER_DESKTOP_REVOKED_NOTE.to_owned(), Some("Esc".into()))]
    );
    assert!(
        !rig.system_notes()
            .iter()
            .any(|note| note == COMPUTER_ESCAPE_STOP_NOTICE)
    );
    let run = rig.core.sessions.computer_run(CHAT, &secret).unwrap();
    assert!(run.turn_desktop_revoked);
    let _ = release.send(());
    let _ = activity.await;

    rig.run("c2", "next", PermissionMode::Full, None);
    assert!(rig.saw(2).await.steer);
    caller_for(&rig, "m-c2").await;
    let run = rig.core.sessions.computer_run(CHAT, &secret).unwrap();
    assert!(
        !run.turn_desktop_revoked,
        "the next message clears the latch"
    );
    rig.core.shutdown().await;
}

#[tokio::test]
async fn the_strip_stop_revokes_only_its_chat() {
    let rig = rig();
    rig.settings(true);
    rig.run("c1", "plain", PermissionMode::Full, None);
    let secret = rig.saw(1).await.secret.unwrap();
    manager(&rig).await;
    let client = zeron_rpc::memory_client(rig.core.rpc_service());
    let reply = client
        .call(methods::COMPUTER_REVOKE_DESKTOP, json!({ "chatId": LEAD }))
        .await
        .unwrap();
    assert_eq!(reply, json!({ "revoked": false }));
    assert!(
        !rig.core
            .sessions
            .computer_run(CHAT, &secret)
            .unwrap()
            .turn_desktop_revoked
    );
    let reply = client
        .call(methods::COMPUTER_REVOKE_DESKTOP, json!({ "chatId": CHAT }))
        .await
        .unwrap();
    assert_eq!(reply, json!({ "revoked": true }));
    assert!(rig.in_flight());
    assert!(
        rig.core
            .sessions
            .computer_run(CHAT, &secret)
            .unwrap()
            .turn_desktop_revoked
    );
    assert_eq!(rig.generation(), 0, "the strip Stop never fences the chat");
    assert_eq!(
        rig.revoked_notes(),
        vec![(
            COMPUTER_DESKTOP_REVOKED_NOTE.to_owned(),
            Some("Stop".into())
        )]
    );
    let again = client
        .call(methods::COMPUTER_REVOKE_DESKTOP, json!({ "chatId": CHAT }))
        .await
        .unwrap();
    assert_eq!(again, json!({ "revoked": false }));
    assert_eq!(rig.revoked_notes().len(), 1, "one note per revoke");
    rig.finish_turn().await;
    rig.core.shutdown().await;
}

async fn caller_for(rig: &Rig, turn: &str) -> ComputerCaller {
    let secret = rig.seen()[0].secret.clone().unwrap();
    wait_for(
        || {
            rig.core
                .sessions
                .computer_run(CHAT, &secret)
                .and_then(|run| run.turn_id)
                .as_deref()
                == Some(turn)
        },
        "the live turn",
    )
    .await;
    ComputerCaller {
        chat_id: CHAT.into(),
        turn_id: Some(turn.into()),
    }
}

fn card_resolved(rig: &Rig, card: &str) -> bool {
    rig.consent_cards()
        .iter()
        .any(|(id, _, resolved)| id == card && *resolved)
}

#[tokio::test]
async fn a_decline_holds_for_the_turn_and_turn_end_and_stop_cancel_an_open_card() {
    let rig = rig();
    rig.run("c1", "plain", PermissionMode::Ask, None);
    rig.saw(1).await;
    let manager = manager(&rig).await;

    let first = caller_for(&rig, "m-c1").await;
    let click = authorize(&manager, "computer_click", &first);
    wait_for(|| rig.consent_cards().len() == 1, "the task card").await;
    let declined = rig.consent_cards()[0].0.clone();
    answer(&rig, "decline", &declined, "Decline");
    assert!(!click.await.unwrap());
    assert!(
        !authorize(&manager, "computer_type_text", &first)
            .await
            .unwrap()
    );
    assert_eq!(rig.consent_cards().len(), 1, "a decline is not re-asked");
    rig.finish_turn().await;

    rig.run("c2", "second", PermissionMode::Ask, None);
    assert!(rig.saw(2).await.steer);
    let second = caller_for(&rig, "m-c2").await;
    let click = authorize(&manager, "computer_click", &second);
    wait_for(|| rig.consent_cards().len() == 2, "a new turn asks again").await;
    let open_at_turn_end = rig.consent_cards()[1].0.clone();
    pending_consent(&manager, &open_at_turn_end).await;
    rig.finish_turn().await;
    assert!(!click.await.unwrap(), "turn end cancels the open card");
    no_pending(&manager, "turn end clears the pending request").await;
    wait_for(
        || card_resolved(&rig, &open_at_turn_end),
        "the turn-end card to settle",
    )
    .await;

    rig.run("c3", "third", PermissionMode::Ask, None);
    assert!(rig.saw(3).await.steer);
    let third = caller_for(&rig, "m-c3").await;
    let click = authorize(&manager, "computer_click", &third);
    wait_for(|| rig.consent_cards().len() == 3, "the third card").await;
    let open_at_stop = rig.consent_cards()[2].0.clone();
    pending_consent(&manager, &open_at_stop).await;
    rig.command("stop", SessionCommandPayload::Interrupt { teardown: false }, None);
    rig.applied("stop").await;
    assert!(!click.await.unwrap(), "Stop cancels the open card");
    no_pending(&manager, "Stop clears the pending request").await;
    wait_for(
        || card_resolved(&rig, &open_at_stop),
        "the stopped card to settle",
    )
    .await;
    assert_eq!(rig.generation(), 1);
    assert_ne!(
        rig.core.sessions.session_status(CHAT).unwrap().status,
        SessionStatus::AwaitingInput
    );
    rig.core.shutdown().await;
}

#[tokio::test]
async fn archiving_a_chat_suspends_its_computer_control_and_restoring_lifts_it() {
    let rig = rig();
    rig.run("c1", "plain", PermissionMode::Full, None);
    rig.saw(1).await;
    let manager = manager(&rig).await;
    let generation = manager.control_state().generation(CHAT);
    assert!(manager.can_activate_control(CHAT, generation));

    rig.core.workspace.set_chat_archived(CHAT, true).unwrap();
    wait_for(
        || !manager.can_activate_control(CHAT, generation),
        "archive to suspend the chat",
    )
    .await;
    rig.core.workspace.set_chat_archived(CHAT, false).unwrap();
    wait_for(
        || manager.can_activate_control(CHAT, generation),
        "restore to lift the suspension",
    )
    .await;
    rig.core.shutdown().await;
}

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

fn crashed_slash_turn(root: &std::path::Path, chat: &str, text: &str, stamp: Option<u64>) {
    let profile = root.join("orgs/dev-org/dev-user");
    let journal = zeron_engine::RunJournal::open(profile.join("journals")).unwrap();
    journal
        .append(
            chat,
            &AgentEvent::SessionStarted {
                harness: HarnessId::Mock,
                model: "probe".into(),
                tools: vec![],
                cwd: "/tmp".into(),
                session_id: format!("hs-{chat}"),
                assistant_message_id: format!("a-{chat}"),
            },
        )
        .unwrap();
    journal
        .append(
            chat,
            &AgentEvent::TextDelta {
                text: "doomed".into(),
            },
        )
        .unwrap();
    let doc = zeron_doc::SessionDoc::init(chat).unwrap();
    doc.set_computer_generation(1).unwrap();
    let user_id = format!("m-{chat}");
    doc.push_message(&SessionMessageEntry {
        id: user_id.clone(),
        role: MessageRole::User,
        parts: vec![MessagePart::Text {
            id: "t0".into(),
            text: text.into(),
        }],
        created_at: now_ms(),
        device_id: "dev-a".into(),
        status: Some(MessageStatus::Complete),
        continuation_of: None,
        from_chat_id: None,
        fork_import: false,
        answers_request_id: None,
        duration_ms: None,
    })
    .unwrap();
    doc.queue_command(&SessionCommandEntry {
        id: format!("c-{chat}"),
        payload: SessionCommandPayload::Run {
            request: request(text, PermissionMode::Full),
            message_id: user_id,
        },
        issued_by: "viewer".into(),
        issued_at: now_ms(),
        based_on: None,
        expires_at: None,
        status: SessionCommandStatus::Applied,
        resolution: None,
        computer_generation: stamp,
    })
    .unwrap();
    let mut writer =
        zeron_doc::SegmentWriter::begin(&doc, &format!("a-{chat}"), "dev-a", now_ms()).unwrap();
    writer
        .sync(&[MessagePart::Text {
            id: "t0".into(),
            text: "doomed".into(),
        }])
        .unwrap();
    zeron_sync::DocsStore::open(profile)
        .unwrap()
        .save_snapshot(chat, &doc.export_snapshot().unwrap())
        .unwrap();
}

#[tokio::test]
async fn a_restart_resumes_a_slash_turn_with_the_generation_it_was_dispatched_with() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("device-id"), "dev-a").unwrap();
    crashed_slash_turn(dir.path(), CHAT, "/computer-use open Calculator", Some(0));
    crashed_slash_turn(dir.path(), LEAD, "/computer-use open Safari", Some(1));
    let rig = rig_in(dir, None);
    wait_for(|| rig.seen().len() >= 2, "both crashed turns to resume").await;
    let seen = rig.seen();
    let fenced = seen
        .iter()
        .find(|seen| seen.prompt == "/computer-use open Calculator")
        .expect("the fenced turn resumed");
    assert!(!fenced.computer, "a turn the Stop fenced off resumes off");
    let current = seen
        .iter()
        .find(|seen| seen.prompt == "open Safari")
        .expect("the current turn resumed");
    assert!(
        current.computer,
        "a turn stamped at the current generation resumes on"
    );
    rig.core.shutdown().await;
}

fn fake_host(rig: &Rig, withheld: ComputerHostMethod) -> tokio::task::JoinHandle<()> {
    let client = zeron_rpc::memory_client(rig.core.rpc_service());
    tokio::spawn(async move {
        let mut host = client
            .subscribe_checked(
                methods::WATCH_COMPUTER_HOST_REQUESTS,
                json!({ "bundleId": "sh.zeron.test" }),
            )
            .await
            .expect("the host registers");
        let mut token = String::new();
        while let Some(value) = host.recv().await {
            match serde_json::from_value(value).expect("host stream item") {
                ComputerHostStreamItem::Registered { host_token } => token = host_token,
                ComputerHostStreamItem::Request(request) if request.call.method != withheld => {
                    let _ = client
                        .call(
                            methods::RESPOND_COMPUTER_HOST_REQUEST,
                            json!({
                                "hostToken": token,
                                "correlationId": request.call.correlation_id,
                                "result": { "ok": true },
                            }),
                        )
                        .await;
                }
                _ => {}
            }
        }
    })
}

#[tokio::test]
async fn a_stop_interrupts_the_turn_without_waiting_for_the_host_teardown() {
    let rig = rig();
    let host = fake_host(&rig, ComputerHostMethod::Stop);
    wait_for(
        || {
            rig.core
                .computer
                .service()
                .is_some_and(|service| service.supported)
        },
        "the hosted service",
    )
    .await;
    rig.run("c1", "plain", PermissionMode::Full, None);
    rig.saw(1).await;
    let manager = manager(&rig).await;
    let (release, released) = tokio::sync::oneshot::channel::<()>();
    let (entered_tx, entered) = tokio::sync::oneshot::channel::<()>();
    let activity = {
        let manager = manager.clone();
        tokio::spawn(async move {
            manager
                .with_agent_activity(
                    CHAT,
                    move || async move {
                        let _ = entered_tx.send(());
                        let _ = released.await;
                        Ok(())
                    },
                    None,
                    Some("m-c1"),
                    None,
                )
                .await
        })
    };
    entered.await.unwrap();

    let started = tokio::time::Instant::now();
    rig.command("stop", SessionCommandPayload::Interrupt { teardown: false }, None);
    rig.applied("stop").await;
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "Stop waited on the host teardown"
    );
    assert!(!rig.in_flight());
    assert_eq!(rig.generation(), 1);
    let _ = release.send(());
    host.abort();
    let _ = activity.await;
    rig.core.shutdown().await;
}

fn push_open_question(rig: &Rig, request_id: &str) {
    rig.core
        .doc_host
        .open(CHAT)
        .unwrap()
        .doc()
        .push_message(&SessionMessageEntry {
            id: format!("a-{request_id}"),
            role: MessageRole::Assistant,
            parts: vec![MessagePart::Input {
                id: request_id.into(),
                request_id: request_id.into(),
                questions: vec![UserInputQuestion {
                    approval: false,
                    id: "q1".into(),
                    header: "Foreground".into(),
                    question: "Can I bring Safari to the front?".into(),
                    options: vec!["Yes".into(), "No".into()],
                    multi_select: false,
                    kind: None,
                    prefill: None,
                    multiline: false,
                }],
                resolved: false,
                answers: None,
                asked_at_ms: None,
                answered_at_ms: None,
            }],
            created_at: 1,
            device_id: "dev-a".into(),
            status: Some(MessageStatus::Aborted),
            continuation_of: None,
            from_chat_id: None,
            fork_import: false,
            answers_request_id: None,
            duration_ms: None,
        })
        .unwrap();
}

#[tokio::test]
async fn an_orphan_answer_that_flips_computer_waits_for_the_turn_end() {
    let rig = rig();
    rig.run("c1", "plain", PermissionMode::Full, None);
    assert!(!rig.saw(1).await.computer);
    push_open_question(&rig, "r-visible");
    rig.settings(true);
    rig.command(
        "late-answer",
        SessionCommandPayload::RespondInput {
            request_id: "r-visible".into(),
            answers: vec![UserInputAnswer {
                question_id: "q1".into(),
                labels: vec!["No".into()],
            }],
        },
        None,
    );
    let (status, resolution) = rig.applied("late-answer").await;
    assert_eq!(status, SessionCommandStatus::Applied);
    assert_eq!(resolution.as_deref(), Some("held until the turn ends"));
    assert_eq!(rig.seen().len(), 1, "the live turn was not interrupted");
    assert!(rig.in_flight());

    rig.finish();
    let answered = rig.saw(2).await;
    assert!(answered.computer && !answered.steer);
    rig.finish_turn().await;
    rig.core.shutdown().await;
}

#[tokio::test]
async fn a_thread_message_held_for_a_computer_flip_leaves_no_delivered_entry() {
    let rig = rig();
    rig.run("c1", "plain", PermissionMode::Full, None);
    assert!(!rig.saw(1).await.computer);
    rig.settings(true);
    let row = rig
        .core
        .doc_host
        .queue_thread_message(CHAT, Some(LEAD), "note from lead")
        .await
        .unwrap();
    assert!(
        rig.entries(CHAT).iter().all(|entry| entry.id != row),
        "a row held for the turn end is not in the transcript"
    );
    let queue = rig
        .core
        .doc_host
        .open(CHAT)
        .unwrap()
        .doc()
        .read_queue()
        .unwrap();
    assert_eq!(
        queue.first().map(|item| item.id.as_str()),
        Some(row.as_str())
    );
    assert_eq!(rig.seen().len(), 1);

    rig.finish();
    let delivered = rig.saw(2).await;
    assert!(delivered.computer && delivered.prompt.contains("note from lead"));
    wait_for(
        || {
            rig.entries(CHAT)
                .iter()
                .any(|entry| entry.id == row && entry.from_chat_id.as_deref() == Some(LEAD))
        },
        "the delivered entry with its provenance",
    )
    .await;
    rig.finish_turn().await;
    rig.core.shutdown().await;
}

#[tokio::test]
async fn an_orphaned_steer_is_redispatched_with_its_own_computer_flag() {
    let rig = rig_with(Some(false));
    rig.probe
        .swallow_steers
        .store(true, std::sync::atomic::Ordering::SeqCst);
    rig.settings(true);
    rig.run("c1", "plain", PermissionMode::Full, None);
    assert!(rig.saw(1).await.computer);
    rig.finish_turn().await;

    rig.settings(false);
    rig.run("c2", "second", PermissionMode::Full, None);
    assert!(rig.saw(2).await.steer);
    rig.ctl().exit.notify_one();
    let redispatched = rig.saw(3).await;
    assert_eq!(redispatched.prompt, "second");
    assert!(!redispatched.steer);
    assert!(
        !redispatched.computer,
        "the orphan keeps the flag its turn resolved"
    );
    rig.finish_turn().await;
    rig.core.shutdown().await;
}

#[tokio::test]
async fn a_self_continued_turn_keeps_the_last_turns_computer_flag() {
    let rig = rig_with(Some(false));
    rig.settings(true);
    rig.run("c1", "plain", PermissionMode::Full, None);
    let first = rig.saw(1).await;
    assert!(first.computer);
    let secret = first.secret.unwrap();
    rig.finish_turn().await;

    rig.settings(false);
    rig.run("c2", "second", PermissionMode::Full, None);
    assert!(rig.saw(2).await.steer);
    let turn = || {
        rig.core
            .sessions
            .computer_run(CHAT, &secret)
            .and_then(|run| run.turn_id.map(|id| (id, run.turn_computer_control)))
    };
    wait_for(
        || turn().is_some_and(|(id, _)| id == "m-c2"),
        "the steered turn",
    )
    .await;
    assert_eq!(turn(), Some(("m-c2".to_owned(), false)));
    rig.finish_turn().await;
    wait_for(|| turn().is_none(), "the park").await;

    tokio::time::sleep(Duration::from_millis(1_200)).await;
    rig.ctl().speak.notify_one();
    wait_for(|| turn().is_some(), "the self-continued turn").await;
    assert_eq!(turn(), Some(("m-c2".to_owned(), false)));
    rig.core.shutdown().await;
}

#[tokio::test]
async fn an_input_request_with_no_questions_still_parks_the_composer() {
    let rig = rig();
    rig.run("c1", "ask-empty", PermissionMode::Full, None);
    rig.saw(1).await;
    wait_for(
        || {
            rig.core
                .sessions
                .session_status(CHAT)
                .is_some_and(|session| session.status == SessionStatus::AwaitingInput)
        },
        "AwaitingInput",
    )
    .await;
    rig.core.shutdown().await;
}

#[tokio::test]
async fn answering_a_consent_card_keeps_an_open_question_awaiting_input() {
    let rig = rig();
    rig.run("c1", "ask-kept", PermissionMode::Ask, None);
    rig.saw(1).await;
    let awaiting =
        || rig.core.sessions.session_status(CHAT).unwrap().status == SessionStatus::AwaitingInput;
    wait_for(awaiting, "the open question parks the composer").await;
    let manager = manager(&rig).await;
    let caller = caller(&rig).await;
    let click = authorize(&manager, "computer_click", &caller);
    wait_for(|| rig.consent_cards().len() == 1, "the task card").await;
    let card = rig.consent_cards()[0].0.clone();
    answer(&rig, "allow", &card, "Allow");
    assert!(click.await.unwrap());
    wait_for(|| card_resolved(&rig, &card), "the card to settle").await;
    assert!(awaiting(), "the ordinary question is still open");
    rig.core.shutdown().await;
}

#[tokio::test]
async fn a_question_the_agent_withdrew_does_not_hold_awaiting_input() {
    let rig = rig();
    rig.run("c1", "ask-withdrawn", PermissionMode::Full, None);
    rig.saw(1).await;
    let kept = || {
        rig.entries(CHAT)
            .iter()
            .flat_map(|entry| entry.parts.clone())
            .find_map(|part| match part {
                MessagePart::Input {
                    request_id,
                    questions,
                    ..
                } if questions[0].id == "kept" => Some(request_id),
                _ => None,
            })
    };
    wait_for(|| kept().is_some(), "the kept question in the doc").await;
    assert_eq!(
        rig.core.sessions.session_status(CHAT).unwrap().status,
        SessionStatus::AwaitingInput
    );
    let answered = rig
        .core
        .sessions
        .respond_input(
            CHAT,
            &kept().unwrap(),
            vec![UserInputAnswer {
                question_id: "kept".into(),
                labels: vec!["Yes".into()],
            }],
        )
        .unwrap();
    assert!(answered);
    wait_for(
        || {
            rig.core
                .sessions
                .session_status(CHAT)
                .is_some_and(|session| session.status == SessionStatus::Working)
        },
        "Working once the only live question is answered",
    )
    .await;
    rig.finish_turn().await;
    rig.core.shutdown().await;
}

#[tokio::test]
async fn ask_user_parks_a_multi_select_card_and_returns_every_picked_label() {
    let rig = rig();
    rig.run("c1", "plain", PermissionMode::Ask, None);
    let secret = rig.saw(1).await.secret.expect("the run carries a secret");
    assert_eq!(
        rig.core
            .sessions
            .ask_user(
                CHAT,
                "forged",
                vec![plain_question("q1")],
                Duration::from_secs(5)
            )
            .await
            .unwrap_err(),
        "This chat has no live run to ask in."
    );

    let mut question = plain_question("layers");
    question.options = vec!["UI".into(), "Engine".into(), "MCP".into()];
    question.multi_select = true;
    let asked = tokio::spawn({
        let (sessions, secret) = (rig.core.sessions.clone(), secret.clone());
        async move {
            sessions
                .ask_user(CHAT, &secret, vec![question], Duration::from_secs(10))
                .await
        }
    });
    let open = || {
        rig.entries(CHAT)
            .iter()
            .flat_map(|e| e.parts.clone())
            .find_map(|part| match part {
                MessagePart::Input {
                    request_id,
                    questions,
                    resolved: false,
                    ..
                } => Some((request_id, questions)),
                _ => None,
            })
    };
    wait_for(|| open().is_some(), "the question card").await;
    let (request_id, questions) = open().unwrap();
    assert!(questions[0].multi_select);
    wait_for(
        || rig.core.sessions.session_status(CHAT).unwrap().status == SessionStatus::AwaitingInput,
        "the composer to await input",
    )
    .await;
    rig.command(
        "pick",
        SessionCommandPayload::RespondInput {
            request_id,
            answers: vec![UserInputAnswer {
                question_id: "layers".into(),
                labels: vec!["UI".into(), "MCP".into()],
            }],
        },
        None,
    );
    let answers = asked.await.unwrap().unwrap();
    assert_eq!(answers[0].labels, vec!["UI".to_string(), "MCP".to_string()]);

    let unanswered = rig
        .core
        .sessions
        .ask_user(
            CHAT,
            &secret,
            vec![plain_question("late")],
            Duration::from_millis(50),
        )
        .await
        .unwrap();
    assert!(unanswered.is_empty());
    wait_for(|| open().is_none(), "the timed-out card to settle").await;
    rig.finish_turn().await;
    rig.core.shutdown().await;
}
