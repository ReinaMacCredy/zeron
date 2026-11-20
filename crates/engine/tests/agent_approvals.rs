use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use futures::StreamExt;
use futures::stream::BoxStream;
use serde_json::{Value, json};

use zeron_doc::{
    CommandBasedOn, MessagePart, SessionCommandEntry, SessionCommandPayload, SessionCommandStatus,
};
use zeron_engine::{EngineCore, HarnessRegistry};
use zeron_harness::{Harness, HarnessError, RunControls, approval};
use zeron_proto::{
    AccessMode, AgentEvent, ApprovalAction, DoneStatus, HarnessId, Model, ReasoningLevel, RunRequest, SandboxLevel,
    SessionStatus, SteeringMode, UserInputQuestion,
};
use zeron_rpc::{Caller, RpcService, methods, with_caller};

type Answers = Arc<Mutex<Vec<(String, bool)>>>;
type Ask = Arc<
    dyn Fn(
            Vec<UserInputQuestion>,
        ) -> tokio::sync::oneshot::Receiver<Vec<zeron_proto::UserInputAnswer>>
        + Send
        + Sync,
>;

struct ApprovalHarness {
    answers: Answers,
    steerable: bool,
    launches: Arc<AtomicUsize>,
}

async fn play(prompt: &str, request_input: &Ask, answers: &Answers) {
    let ask = |label: &str, question: UserInputQuestion| {
        let pending = request_input(vec![question.clone()]);
        let answers = answers.clone();
        let label = label.to_owned();
        async move {
            let given = pending.await.unwrap_or_default();
            let yes = approval::approved(&question, &given);
            answers.lock().unwrap().push((label, yes));
        }
    };
    if prompt.contains("edit and command") {
        let edit = ask("edit", approval::edit("Mock", &["a.rs"]));
        let command = ask("command", approval::command("Mock", "cargo test"));
        futures::join!(edit, command);
    } else if prompt.contains("command then tool and edit") {
        ask("first", approval::command("Mock", "ls")).await;
        ask("second", approval::command("Mock", "pwd")).await;
        ask("tool", approval::tool("Mock", "fetch", None)).await;
        ask("edit", approval::edit("Mock", &["a.rs"])).await;
    } else if prompt.contains("two commands") {
        ask("first", approval::command("Mock", "ls")).await;
        ask("second", approval::command("Mock", "pwd")).await;
    } else if let Some(command) = prompt.strip_prefix("auto: ") {
        ask("auto", approval::command("Mock", command)).await;
    } else if prompt.contains("auto edit") {
        ask("edit", approval::edit("Mock", &["a.rs"])).await;
    } else if prompt.contains("one command") {
        ask("next turn", approval::command("Mock", "whoami")).await;
    } else if prompt.contains("plan") {
        ask(
            "plan",
            approval::question(zeron_proto::PLAN_APPROVAL_HEADER, "Start it?"),
        )
        .await;
    }
}

fn done() -> Result<AgentEvent, HarnessError> {
    Ok(AgentEvent::Done {
        status: DoneStatus::Completed,
        result: None,
        error: None,
        session_id: Some("hs-approvals".into()),
    })
}

#[async_trait]
impl Harness for ApprovalHarness {
    fn id(&self) -> HarnessId {
        HarnessId::Mock
    }
    fn display_name(&self) -> &str {
        "Approvals"
    }
    fn access_modes(&self) -> &[AccessMode] {
        &AccessMode::ALL
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
    async fn run_one_shot(
        &self,
        request: RunRequest,
        controls: RunControls,
        instructions: &'static str,
    ) -> Result<BoxStream<'static, Result<AgentEvent, HarnessError>>, HarnessError> {
        zeron_harness::mock::MockHarness { script: Vec::new() }
            .run_one_shot(request, controls, instructions)
            .await
    }
    async fn run(
        &self,
        request: RunRequest,
        controls: RunControls,
    ) -> Result<BoxStream<'static, Result<AgentEvent, HarnessError>>, HarnessError> {
        self.launches.fetch_add(1, Ordering::SeqCst);
        let answers = self.answers.clone();
        let RunControls {
            request_input,
            mut steering,
            ..
        } = controls;
        let request_input: Ask = Arc::from(request_input);
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel::<Result<AgentEvent, HarnessError>>();
        if self.steerable {
            let (tx, request_input, answers) = (tx.clone(), request_input.clone(), answers.clone());
            tokio::spawn(async move {
                while let Some(message) = steering.recv().await {
                    let _ = tx.send(Ok(AgentEvent::Steered {
                        assistant_message_id: None,
                        next_assistant_message_id: None,
                    }));
                    tokio::time::sleep(Duration::from_millis(100)).await;
                    play(&message.prompt, &request_input, &answers).await;
                    let _ = tx.send(done());
                }
            });
        }
        tokio::spawn(async move {
            let _ = tx.send(Ok(AgentEvent::SessionStarted {
                harness: HarnessId::Mock,
                model: "approvals-1".into(),
                tools: vec![],
                cwd: "/tmp".into(),
                session_id: "hs-approvals".into(),
                assistant_message_id: "a-approvals".into(),
            }));
            play(&request.prompt, &request_input, &answers).await;
            let _ = tx.send(done());
        });
        Ok(futures::stream::unfold(rx, |mut rx| async move {
            rx.recv().await.map(|event| (event, rx))
        })
        .boxed())
    }
}

struct Rig {
    core: EngineCore,
    answers: Answers,
    launches: Arc<AtomicUsize>,
    _dir: tempfile::TempDir,
}

async fn rig(lead_access: &str) -> Rig {
    rig_with(lead_access, false).await
}

async fn rig_with(lead_access: &str, steerable: bool) -> Rig {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("device-id"), "dev-a").unwrap();
    let answers = Answers::default();
    let launches = Arc::new(AtomicUsize::new(0));
    let registry = HarnessRegistry::new();
    registry.register(Arc::new(ApprovalHarness {
        answers: answers.clone(),
        steerable,
        launches: launches.clone(),
    }));
    let core = EngineCore::assemble(dir.path(), Arc::new(registry), HarnessId::Mock, None)
        .expect("engine core assembles");
    core.workspace
        .create_space("space", "dev-a", "/tmp", None, false)
        .expect("space");
    let rpc = core.rpc_service();
    for (id, access, parent) in [
        ("lead", lead_access, None),
        ("child", "supervised", Some("lead")),
    ] {
        rpc.handle(
            methods::MUTATE,
            json!({"op":"createChat", "chatId":id, "spaceId":"space",
                "createdByChatId":parent, "childKind":parent.map(|_| "subagent"),
                "config":{"harness":"mock","sandbox":"workspace-write","access":access}}),
        )
        .await
        .unwrap();
    }
    Rig {
        core,
        answers,
        launches,
        _dir: dir,
    }
}

fn start_child(core: &EngineCore, prompt: &str) {
    start_child_with(core, prompt, AccessMode::Supervised);
}

fn start_child_with(core: &EngineCore, prompt: &str, access: AccessMode) {
    let handle = core.doc_host.open("child").expect("open chat");
    handle
        .doc()
        .queue_command(&SessionCommandEntry {
            id: format!("cmd-{prompt}"),
            payload: SessionCommandPayload::Run {
                request: RunRequest {
                    prompt: prompt.into(),
                    harness: None,
                    model: None,
                    reasoning: None,
                    model_options: Default::default(),
                    cwd: "/tmp".into(),
                    sandbox: SandboxLevel::WorkspaceWrite,
                    access,
                    plan: false,
                    auto_approve: false,
                    attachments: Vec::new(),
                    worktree: None,
                    branch: None,
                    resume: None,
                    forked_at: None,
                    computer_control: false,
                    mcp: None,
                    disabled_tools: Vec::new(),
                    env: Default::default(),
                    extra_mcp: Vec::new(),
                },
                message_id: format!("msg-{prompt}"),
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

async fn wait_for<F: FnMut() -> bool>(mut predicate: F, what: &str) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    while !predicate() {
        assert!(
            tokio::time::Instant::now() < deadline,
            "timed out waiting for {what}"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

fn inputs(core: &EngineCore) -> Vec<(String, Vec<UserInputQuestion>, bool)> {
    core.doc_host
        .open("child")
        .unwrap()
        .doc()
        .read_entries()
        .unwrap_or_default()
        .iter()
        .flat_map(|entry| entry.parts.clone())
        .filter_map(|part| match part {
            MessagePart::Input {
                request_id,
                questions,
                resolved,
                ..
            } => Some((request_id, questions, resolved)),
            _ => None,
        })
        .collect()
}

fn open_inputs(core: &EngineCore) -> Vec<(String, Vec<UserInputQuestion>)> {
    inputs(core)
        .into_iter()
        .filter(|(_, _, resolved)| !resolved)
        .map(|(id, questions, _)| (id, questions))
        .collect()
}

fn answered(rig: &Rig) -> Vec<(String, bool)> {
    let mut answers = rig.answers.lock().unwrap().clone();
    answers.sort();
    answers
}

fn child_access(core: &EngineCore) -> AccessMode {
    core.workspace
        .chat("child")
        .unwrap()
        .unwrap()
        .config
        .unwrap()
        .access
}

fn idle(core: &EngineCore) -> bool {
    core.sessions
        .session_status("child")
        .is_some_and(|s| s.status == SessionStatus::Idle)
}

async fn as_lead(rig: &Rig, method: &'static str, params: Value) -> Result<(), String> {
    let rpc = rig.core.rpc_service();
    with_caller(
        Caller::Agent {
            chat: Some("lead".into()),
        },
        async move {
            rpc.handle(method, params)
                .await
                .map(drop)
                .map_err(|error| error.to_string())
        },
    )
    .await
}

async fn raise_child(rig: &Rig, access: &str) -> Result<(), String> {
    as_lead(
        rig,
        methods::MUTATE,
        json!({"op":"setChatConfig", "chatId":"child", "config":{"access":access}}),
    )
    .await
}

fn respond(request_id: &str, question_id: &str, label: &str, scope: &str) -> Value {
    json!({"chatId":"child", "fromChatId":"lead", "scope":scope,
        "command":{"kind":"respondInput", "requestId":request_id,
            "answers":[{"questionId":question_id, "labels":[label]}]}})
}

#[tokio::test]
async fn a_raise_to_full_answers_the_childs_pending_command() {
    let rig = rig("full-access").await;
    start_child(&rig.core, "two commands");
    wait_for(
        || open_inputs(&rig.core).len() == 1,
        "the first command card",
    )
    .await;

    raise_child(&rig, "full-access").await.unwrap();
    assert_eq!(child_access(&rig.core), AccessMode::FullAccess);
    wait_for(|| idle(&rig.core), "the child's run to finish").await;
    assert_eq!(
        answered(&rig),
        [("first".into(), true), ("second".into(), true)]
    );
    assert_eq!(inputs(&rig.core).len(), 1, "the second command never asked");
    rig.core.shutdown().await;
}

#[tokio::test]
async fn a_raise_to_auto_accept_edits_answers_the_edit_and_leaves_the_command() {
    let rig = rig("full-access").await;
    start_child(&rig.core, "edit and command");
    wait_for(|| open_inputs(&rig.core).len() == 2, "both cards").await;

    raise_child(&rig, "auto-accept-edits").await.unwrap();
    wait_for(|| !answered(&rig).is_empty(), "the edit's answer").await;
    wait_for(
        || open_inputs(&rig.core).len() == 1,
        "the edit card resolved",
    )
    .await;
    assert_eq!(answered(&rig), [("edit".into(), true)]);
    let open = open_inputs(&rig.core);
    assert_eq!(open[0].1[0].header, "Approve command");

    assert!(
        rig.core
            .sessions
            .respond_input("child", &open[0].0, Vec::new())
            .unwrap()
    );
    wait_for(|| idle(&rig.core), "the child's run to finish").await;
    assert_eq!(
        answered(&rig),
        [("command".into(), false), ("edit".into(), true)]
    );
    rig.core.shutdown().await;
}

#[tokio::test]
async fn a_raise_is_capped_at_the_leads_own_access() {
    let rig = rig("auto-accept-edits").await;
    raise_child(&rig, "full-access").await.unwrap();
    assert_eq!(child_access(&rig.core), AccessMode::AutoAcceptEdits);
    rig.core.shutdown().await;
}

#[tokio::test]
async fn scope_run_lets_the_next_command_through_and_keeps_the_chat_config() {
    let rig = rig("full-access").await;
    start_child(&rig.core, "two commands");
    wait_for(
        || open_inputs(&rig.core).len() == 1,
        "the first command card",
    )
    .await;
    let (request_id, questions) = open_inputs(&rig.core).remove(0);

    let refused = as_lead(
        &rig,
        methods::QUEUE_COMMAND,
        respond(&request_id, &questions[0].id, "No", "run"),
    )
    .await
    .unwrap_err();
    assert!(refused.contains("answers Yes"), "{refused}");
    as_lead(
        &rig,
        methods::QUEUE_COMMAND,
        respond(&request_id, &questions[0].id, "Yes", "run"),
    )
    .await
    .unwrap();
    wait_for(|| idle(&rig.core), "the child's run to finish").await;
    assert_eq!(
        answered(&rig),
        [("first".into(), true), ("second".into(), true)]
    );
    assert_eq!(inputs(&rig.core).len(), 1, "the second command never asked");
    assert_eq!(child_access(&rig.core), AccessMode::Supervised);
    rig.core.shutdown().await;
}

#[tokio::test]
async fn scope_run_on_a_command_lets_only_commands_through() {
    let rig = rig("full-access").await;
    start_child(&rig.core, "command then tool and edit");
    wait_for(|| open_inputs(&rig.core).len() == 1, "the command card").await;
    let (request_id, questions) = open_inputs(&rig.core).remove(0);
    as_lead(
        &rig,
        methods::QUEUE_COMMAND,
        respond(&request_id, &questions[0].id, "Yes", "run"),
    )
    .await
    .unwrap();
    for kind in [ApprovalAction::Tool, ApprovalAction::Edit] {
        let card = || {
            open_inputs(&rig.core)
                .into_iter()
                .find(|(_, questions)| questions[0].action == Some(kind))
        };
        wait_for(|| card().is_some(), "the next kind's card").await;
        let (id, _) = card().unwrap();
        assert!(
            rig.core
                .sessions
                .respond_input("child", &id, Vec::new())
                .unwrap()
        );
    }
    wait_for(|| idle(&rig.core), "the child's run to finish").await;
    assert_eq!(
        answered(&rig),
        [
            ("edit".into(), false),
            ("first".into(), true),
            ("second".into(), true),
            ("tool".into(), false)
        ]
    );
    rig.core.shutdown().await;
}

#[tokio::test]
async fn scope_run_refuses_a_plan_approval() {
    let rig = rig("full-access").await;
    start_child(&rig.core, "plan");
    wait_for(|| open_inputs(&rig.core).len() == 1, "the plan card").await;
    let (request_id, questions) = open_inputs(&rig.core).remove(0);

    let refused = as_lead(
        &rig,
        methods::QUEUE_COMMAND,
        respond(&request_id, &questions[0].id, "Yes", "run"),
    )
    .await
    .unwrap_err();
    assert!(refused.contains("approved for the run"), "{refused}");
    assert_eq!(open_inputs(&rig.core).len(), 1, "the plan still waits");

    as_lead(
        &rig,
        methods::QUEUE_COMMAND,
        respond(&request_id, &questions[0].id, "No", "once"),
    )
    .await
    .unwrap();
    wait_for(|| idle(&rig.core), "the child's run to finish").await;
    assert_eq!(answered(&rig), [("plan".into(), false)]);
    rig.core.shutdown().await;
}

#[tokio::test]
async fn a_config_write_that_does_not_raise_access_answers_nothing() {
    let rig = rig("full-access").await;
    let set = |model: &str| {
        json!({"op":"setChatConfig", "chatId":"child",
            "config":{"harness":"mock","model":model,"sandbox":"workspace-write","access":"full-access"}})
    };
    let rpc = rig.core.rpc_service();
    rpc.handle(methods::MUTATE, set("m1")).await.unwrap();
    start_child(&rig.core, "two commands");
    wait_for(
        || open_inputs(&rig.core).len() == 1,
        "the first command card",
    )
    .await;

    rpc.handle(methods::MUTATE, set("m2")).await.unwrap();
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert!(answered(&rig).is_empty());
    let (request_id, _) = open_inputs(&rig.core).remove(0);
    assert!(
        rig.core
            .sessions
            .respond_input("child", &request_id, Vec::new())
            .unwrap()
    );
    rig.core.shutdown().await;
}

#[tokio::test]
async fn a_run_grant_ends_with_its_turn_on_a_runtime_that_takes_the_next() {
    let rig = rig_with("full-access", true).await;
    start_child(&rig.core, "two commands");
    wait_for(
        || open_inputs(&rig.core).len() == 1,
        "the first command card",
    )
    .await;
    let (request_id, questions) = open_inputs(&rig.core).remove(0);
    as_lead(
        &rig,
        methods::QUEUE_COMMAND,
        respond(&request_id, &questions[0].id, "Yes", "run"),
    )
    .await
    .unwrap();
    wait_for(
        || answered(&rig).len() == 2,
        "both commands of the first turn",
    )
    .await;
    wait_for(|| idle(&rig.core), "the first turn to end").await;
    assert_eq!(
        answered(&rig),
        [("first".into(), true), ("second".into(), true)]
    );

    start_child(&rig.core, "one command");
    wait_for(
        || open_inputs(&rig.core).len() == 1,
        "the next turn's command card",
    )
    .await;
    assert_eq!(
        rig.launches.load(Ordering::SeqCst),
        1,
        "the runtime was reused"
    );
    assert_eq!(answered(&rig).len(), 2, "the next turn's command waits");
    let (request_id, _) = open_inputs(&rig.core).remove(0);
    assert!(
        rig.core
            .sessions
            .respond_input("child", &request_id, Vec::new())
            .unwrap()
    );
    wait_for(|| answered(&rig).len() == 3, "the next turn's answer").await;
    rig.core.shutdown().await;
}

/// The Auto check's wait in these tests: the mock answers at once, and one
/// that never answers is given up on quickly.
fn short_auto_check() {
    // SAFETY: every test here that reads it sets the same value.
    unsafe { std::env::set_var("ZERON_AUTO_REVIEW_TIMEOUT_MS", "1500") };
}

fn answers_of(core: &EngineCore, request_id: &str) -> Vec<Vec<String>> {
    core.doc_host
        .open("child")
        .unwrap()
        .doc()
        .read_entries()
        .unwrap_or_default()
        .iter()
        .flat_map(|entry| entry.parts.clone())
        .filter_map(|part| match part {
            MessagePart::Input {
                request_id: id,
                answers,
                ..
            } if id == request_id => answers,
            _ => None,
        })
        .flatten()
        .map(|answer| answer.labels)
        .collect()
}

#[tokio::test]
async fn auto_lets_through_what_its_check_allows_and_records_it_as_answered_by_auto() {
    short_auto_check();
    let rig = rig("full-access").await;
    start_child_with(
        &rig.core,
        "auto: cargo test auto-allow",
        AccessMode::AutoReview,
    );
    wait_for(|| idle(&rig.core), "the child's run to finish").await;
    assert_eq!(answered(&rig), [("auto".into(), true)]);
    wait_for(|| inputs(&rig.core).len() == 1, "the recorded request").await;
    let (request_id, questions, _) = inputs(&rig.core).remove(0);
    assert_eq!(questions[0].header, "Approve command");
    wait_for(
        || !answers_of(&rig.core, &request_id).is_empty(),
        "its answer",
    )
    .await;
    assert_eq!(
        answers_of(&rig.core, &request_id),
        [vec![
            "Yes".to_owned(),
            zeron_proto::APPROVAL_BY_AUTO.to_owned()
        ]]
    );
    assert!(open_inputs(&rig.core).is_empty());
    rig.core.shutdown().await;
}

#[tokio::test]
async fn auto_asks_the_person_when_its_check_says_ask_answers_garbage_or_never_answers() {
    short_auto_check();
    for command in ["rm -rf build", "make auto-garbage", "deploy auto-hang"] {
        let rig = rig("full-access").await;
        start_child_with(
            &rig.core,
            &format!("auto: {command}"),
            AccessMode::AutoReview,
        );
        wait_for(|| open_inputs(&rig.core).len() == 1, "the command card").await;
        assert!(
            answered(&rig).is_empty(),
            "{command}: nothing answered for the person"
        );
        let (request_id, questions) = open_inputs(&rig.core).remove(0);
        assert!(questions[0].question.contains(command), "{command}");
        assert!(
            rig.core
                .sessions
                .respond_input("child", &request_id, Vec::new())
                .unwrap()
        );
        wait_for(|| idle(&rig.core), "the child's run to finish").await;
        assert_eq!(answered(&rig), [("auto".into(), false)], "{command}");
        rig.core.shutdown().await;
    }
}

#[tokio::test]
async fn auto_applies_an_edit_without_asking_anyone() {
    short_auto_check();
    let rig = rig("full-access").await;
    start_child_with(&rig.core, "auto edit", AccessMode::AutoReview);
    wait_for(|| idle(&rig.core), "the child's run to finish").await;
    assert_eq!(answered(&rig), [("edit".into(), true)]);
    assert!(
        inputs(&rig.core).is_empty(),
        "an edit is covered, not checked"
    );
    rig.core.shutdown().await;
}
