use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use futures::StreamExt;
use futures::stream::BoxStream;
use serde_json::json;

use zeron_engine::{EngineCore, HarnessRegistry};
use zeron_harness::{Harness, HarnessError, RunControls};
use zeron_proto::{
    AgentEvent, DoneStatus, HarnessId, Model, ReasoningLevel, RunRequest, SteeringMode, Task,
};
use zeron_rpc::{RpcReply, RpcService, methods};

struct Recording(Arc<Mutex<Vec<String>>>);

#[async_trait]
impl Harness for Recording {
    fn id(&self) -> HarnessId {
        HarnessId::Mock
    }
    fn display_name(&self) -> &str {
        "Recording"
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
        _controls: RunControls,
    ) -> Result<BoxStream<'static, Result<AgentEvent, HarnessError>>, HarnessError> {
        self.0.lock().unwrap().push(request.prompt);
        Ok(futures::stream::iter([
            Ok(AgentEvent::SessionStarted {
                harness: HarnessId::Mock,
                model: "rec".into(),
                tools: vec![],
                cwd: "/tmp".into(),
                session_id: "hs".into(),
                assistant_message_id: "a1".into(),
            }),
            Ok(AgentEvent::Done {
                status: DoneStatus::Completed,
                result: None,
                error: None,
                session_id: Some("hs".into()),
            }),
        ])
        .boxed())
    }
}

const APPROVAL_HOLD_MS: &str = "1500";

async fn wait_for(mut predicate: impl FnMut() -> bool, what: &str) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    while !predicate() {
        assert!(
            tokio::time::Instant::now() < deadline,
            "timed out waiting for {what}"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

#[tokio::test]
async fn a_started_task_opens_a_linked_chat_whose_first_turn_is_the_task() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("device-id"), "dev-a").unwrap();
    let prompts = Arc::new(Mutex::new(Vec::new()));
    let registry = HarnessRegistry::new();
    registry.register(Arc::new(Recording(prompts.clone())));
    let core = EngineCore::assemble(dir.path(), Arc::new(registry), HarnessId::Mock, None)
        .expect("engine core assembles");
    core.workspace
        .create_space("space-1", "dev-a", "/tmp", None, false)
        .unwrap();
    let rpc = core.rpc_service();
    let mutate = |params: serde_json::Value| {
        let rpc = rpc.clone();
        async move { rpc.handle(methods::MUTATE, params).await }
    };

    let RpcReply::Stream(mut tasks) = rpc.handle(methods::WATCH_TASKS, json!({})).await.unwrap()
    else {
        panic!("expected a task watch")
    };
    assert_eq!(tasks.next().await.unwrap(), json!([]));

    mutate(json!({ "op": "createTask", "taskId": "t1", "title": "Fix the login bug" }))
        .await
        .unwrap();
    mutate(json!({
        "op": "updateTask", "taskId": "t1",
        "patch": { "notes": "It loops on expired tokens", "dueOn": "2026-10-20" },
    }))
    .await
    .unwrap();
    let frame = tasks.next().await.unwrap();
    let listed: Vec<Task> = serde_json::from_value(frame).unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].title, "Fix the login bug");

    let start = json!({
        "op": "startTask", "taskId": "t1", "chatId": "chat-t1", "spaceId": "space-1",
        "config": {
            "harness": "mock", "model": "rec-model", "reasoning": "low",
            "modelOptions": { "fastMode": "on" }, "sandbox": "workspace-write",
            "permissionMode": "auto",
        },
    });
    mutate(start.clone()).await.unwrap();
    wait_for(
        || !prompts.lock().unwrap().is_empty(),
        "the task's first turn",
    )
    .await;
    assert_eq!(
        prompts.lock().unwrap().as_slice(),
        ["Fix the login bug\n\nIt loops on expired tokens"]
    );
    let task = core.workspace.task("t1").unwrap();
    assert_eq!(task.chat_id.as_deref(), Some("chat-t1"));
    assert_eq!(
        task.space_id.as_deref(),
        Some("space-1"),
        "a task without a project adopts the one it ran in"
    );
    let chat = core.workspace.chat("chat-t1").unwrap().unwrap();
    assert_eq!(chat.title.as_deref(), Some("Fix the login bug"));
    assert_eq!(chat.space_id.as_deref(), Some("space-1"));
    let config = chat
        .config
        .expect("the started chat carries the picked config");
    assert_eq!(config.model.as_deref(), Some("rec-model"));
    assert_eq!(config.reasoning, Some(ReasoningLevel::Low));
    assert_eq!(config.model_options["fastMode"], "on");
    assert_eq!(config.permission_mode, zeron_proto::PermissionMode::Auto);

    mutate(start).await.unwrap();
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(
        prompts.lock().unwrap().len(),
        1,
        "a retried start sends nothing"
    );

    let second = json!({ "op": "startTask", "taskId": "t1", "chatId": "chat-t1b" });
    assert!(
        mutate(second.clone()).await.is_err(),
        "a task whose chat still exists cannot start a second one"
    );
    assert!(core.workspace.chat("chat-t1b").unwrap().is_none());
    assert_eq!(
        core.workspace.task("t1").unwrap().chat_id.as_deref(),
        Some("chat-t1")
    );

    core.workspace.delete_chat("chat-t1").unwrap();
    mutate(second).await.unwrap();
    wait_for(
        || prompts.lock().unwrap().len() == 2,
        "the restarted task's first turn",
    )
    .await;
    assert_eq!(
        core.workspace.task("t1").unwrap().chat_id.as_deref(),
        Some("chat-t1b"),
        "a deleted chat may be replaced"
    );

    mutate(json!({ "op": "updateTask", "taskId": "t1", "patch": { "completed": true } }))
        .await
        .unwrap();
    assert!(core.workspace.task("t1").unwrap().completed_at.is_some());
    mutate(json!({ "op": "deleteTask", "taskId": "t1" }))
        .await
        .unwrap();
    assert!(core.workspace.task("t1").is_none());
    assert!(
        mutate(json!({ "op": "startTask", "taskId": "t1", "chatId": "chat-2" }))
            .await
            .is_err()
    );
}

#[tokio::test]
async fn an_orchestrated_start_briefs_its_lead_and_a_failed_send_keeps_the_link() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("device-id"), "dev-a").unwrap();
    let prompts = Arc::new(Mutex::new(Vec::new()));
    let registry = HarnessRegistry::new();
    registry.register(Arc::new(Recording(prompts.clone())));
    let core = EngineCore::assemble(dir.path(), Arc::new(registry), HarnessId::Mock, None)
        .expect("engine core assembles");
    let rpc = core.rpc_service();
    let mutate = |params: serde_json::Value| {
        let rpc = rpc.clone();
        async move { rpc.handle(methods::MUTATE, params).await }
    };
    for id in ["t1", "t2", "t3", "t4"] {
        mutate(json!({ "op": "createTask", "taskId": id, "title": "Shop backend" }))
            .await
            .unwrap();
    }

    mutate(json!({ "op": "startTask", "taskId": "t1", "chatId": "lead-1", "orchestrate": true }))
        .await
        .unwrap();
    wait_for(
        || prompts.lock().unwrap().len() == 1,
        "the first lead's turn",
    )
    .await;
    mutate(json!({
        "op": "startTask", "taskId": "t2", "chatId": "lead-2",
        "orchestrate": true, "approvePlan": false, "sameAgents": true,
    }))
    .await
    .unwrap();
    wait_for(
        || prompts.lock().unwrap().len() == 2,
        "the second lead's turn",
    )
    .await;
    let header = |id: &str| {
        format!(
            "Shop backend\n\nYou lead this task (task id {id}). Break it into steps and run each step on its own thread.\n\n1. Write the plan with the tasks tool: action \"plan\", taskId \"{id}\", steps [{{title, notes, role}}]. Keep each step small enough for one agent.\n"
        )
    };
    let run = "Run each step on its own thread with create_thread: prompt set to what the step must do, taskStepId set to the step's id, title optional. Steps that do not depend on each other can run at the same time.";
    let tail = "Zeron messages you when a step's thread finishes a turn or fails. Read the thread with read_thread, then mark the step done or skipped with the tasks tool, action \"step\", send the thread a fix with send_message_to_thread, or start the step over with create_thread and the same taskStepId.\n{n}. When a decision changes the goal or costs money the person has not approved, or you cannot settle it from the plan, use the tasks tool action \"escalate\" and keep working on what does not depend on it.\n{m}. When every step is done, check the whole result, tell the person what was built, and mark the task completed with the tasks tool, action \"update\", completed true.";
    assert_eq!(
        prompts.lock().unwrap().as_slice(),
        [
            format!(
                "{}2. Then stop. The person reviews the plan in Tasks and approves it; you get a message when it is approved.\n3. {run}\n4. Give each step a role (worker, validator, research, design or general); Zeron runs it on the route set in Settings > Routing. Pass harness or model only when a step needs a specific one.\n5. {}",
                header("t1"),
                tail.replace("{n}", "6").replace("{m}", "7")
            ),
            format!(
                "{}2. {run}\n3. Run every step on your own harness and model: do not pass harness or model to create_thread.\n4. {}",
                header("t2"),
                tail.replace("{n}", "5").replace("{m}", "6")
            ),
        ]
    );
    let led = core.workspace.task("t1").unwrap();
    assert_eq!(led.mode, zeron_proto::TaskMode::Orchestrated);
    assert!(led.plan_pending);
    assert!(!led.plan_doc, "a start without planDoc is steps only");

    mutate(json!({
        "op": "startTask", "taskId": "t4", "chatId": "lead-4", "orchestrate": true, "planDoc": true,
    }))
    .await
    .unwrap();
    wait_for(
        || prompts.lock().unwrap().len() == 3,
        "the plan.md lead's turn",
    )
    .await;
    let doc_brief = prompts.lock().unwrap()[2].clone();
    assert!(
        doc_brief.contains("taskId \"t4\", overview, steps [{title, notes, role}]. The overview is the goal, the approach, and how you will check the result. Each step's notes are its spec: what it builds, where, and how to tell it is done."),
        "{doc_brief}"
    );
    assert!(core.workspace.task("t4").unwrap().plan_doc);
    assert!(
        prompts.lock().unwrap()[..2]
            .iter()
            .all(|brief| !brief.contains("overview")),
        "only a plan.md lead is asked for an overview"
    );

    core.workspace
        .create_chat_from("talk", None, Some("dev-a"), None, None, None)
        .unwrap();
    core.workspace
        .rename_chat("talk", "Shop design chat")
        .unwrap();
    mutate(json!({
        "op": "createTask", "taskId": "t5", "title": "Shop frontend", "notes": "Next.js and Tailwind",
    }))
    .await
    .unwrap();
    let chats = core.workspace.read_chats().unwrap().len();
    mutate(json!({
        "op": "assignTask", "taskId": "t5", "chatId": "talk", "orchestrate": true, "planDoc": true,
    }))
    .await
    .unwrap();
    wait_for(
        || prompts.lock().unwrap().len() == 4,
        "the chat's lead brief",
    )
    .await;
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(prompts.lock().unwrap().len(), 4, "one brief, sent once");
    let brief = prompts.lock().unwrap()[3].clone();
    assert!(
        brief.starts_with(
            "Shop frontend\n\nNext.js and Tailwind\n\nYou lead this task (task id t5)."
        ) && brief.contains("taskId \"t5\", overview"),
        "{brief}"
    );
    let led = core.workspace.task("t5").unwrap();
    assert_eq!(led.chat_id.as_deref(), Some("talk"));
    assert_eq!(led.mode, zeron_proto::TaskMode::Orchestrated);
    assert!(led.plan_pending && led.plan_doc);
    assert_eq!(
        core.workspace.read_chats().unwrap().len(),
        chats,
        "the chat leads in place; no chat is created"
    );
    assert_eq!(
        core.workspace
            .chat("talk")
            .unwrap()
            .unwrap()
            .title
            .as_deref(),
        Some("Shop design chat"),
        "the lead chat keeps its title"
    );
    mutate(json!({ "op": "createTask", "taskId": "t6", "title": "Shop backend" }))
        .await
        .unwrap();
    assert!(
        mutate(json!({
            "op": "assignTask", "taskId": "t6", "chatId": "talk", "orchestrate": true,
        }))
        .await
        .is_err(),
        "a chat that leads an unfinished mission cannot lead a second one"
    );
    assert_eq!(core.workspace.task("t6").unwrap().chat_id, None);
    assert_eq!(prompts.lock().unwrap().len(), 4);
    let refusal = |reply: Result<RpcReply, zeron_rpc::RpcError>| match reply {
        Err(err) => err.to_string(),
        Ok(_) => panic!("a second open plan on one chat is refused"),
    };
    mutate(json!({ "op": "assignTask", "taskId": "t6", "chatId": "talk" }))
        .await
        .unwrap();
    let err = refusal(
        mutate(json!({
            "op": "planSteps", "taskId": "t6", "fromChatId": "talk",
            "steps": [{ "title": "Build the API" }],
        }))
        .await,
    );
    assert!(err.contains("already leads a mission"), "{err}");
    assert!(
        core.workspace.task_steps_of("t6").is_empty(),
        "a refused plan writes no steps"
    );
    assert_eq!(
        core.workspace.task("t6").unwrap().mode,
        zeron_proto::TaskMode::Single
    );
    mutate(json!({ "op": "createTask", "taskId": "t7", "title": "Shop admin" }))
        .await
        .unwrap();
    mutate(json!({ "op": "updateTask", "taskId": "t7", "patch": { "mode": "orchestrated" } }))
        .await
        .unwrap();
    let err = refusal(
        mutate(json!({ "op": "updateTask", "taskId": "t7", "patch": { "chatId": "talk" } })).await,
    );
    assert!(err.contains("already leads a mission"), "{err}");
    assert_eq!(core.workspace.task("t7").unwrap().chat_id, None);
    mutate(json!({ "op": "updateTask", "taskId": "t5", "patch": { "completed": true } }))
        .await
        .unwrap();
    mutate(json!({ "op": "updateTask", "taskId": "t7", "patch": { "chatId": "talk" } }))
        .await
        .unwrap();
    let err = refusal(
        mutate(json!({ "op": "updateTask", "taskId": "t5", "patch": { "completed": false } }))
            .await,
    );
    assert!(
        err.contains("already leads a mission"),
        "reopening a done plan on a chat that leads another: {err}"
    );
    core.workspace.create_sidechat("talk", "talk-side").unwrap();
    mutate(json!({ "op": "createTask", "taskId": "t8", "title": "Shop side" }))
        .await
        .unwrap();
    let err = refusal(
        mutate(json!({
            "op": "assignTask", "taskId": "t8", "chatId": "talk-side", "orchestrate": true,
        }))
        .await,
    );
    assert!(err.contains("side chat cannot lead"), "{err}");
    assert_eq!(core.workspace.task("t8").unwrap().chat_id, None);

    let unapproved = core.workspace.task("t2").unwrap();
    assert_eq!(unapproved.mode, zeron_proto::TaskMode::Orchestrated);
    assert!(!unapproved.plan_pending);
    assert!(
        mutate(json!({ "op": "approvePlan", "taskId": "t1" }))
            .await
            .is_err(),
        "a plan with no steps has nothing to approve"
    );

    assert!(
        mutate(json!({
            "op": "assignTask", "taskId": "t3", "chatId": "lead-1", "send": true, "fromChatId": "ghost",
        }))
        .await
        .is_err()
    );
    let unsent = core.workspace.task("t3").unwrap();
    assert_eq!(unsent.chat_id.as_deref(), Some("lead-1"), "the link stays");
    assert!(unsent.send_error.is_some());
    mutate(json!({ "op": "assignTask", "taskId": "t3", "chatId": "lead-1", "send": true }))
        .await
        .unwrap();
    assert_eq!(
        core.workspace.task("t3").unwrap().send_error,
        None,
        "a send that lands clears the old failure"
    );

    core.workspace
        .create_chat_from("quiet", None, Some("dev-a"), None, None, None)
        .unwrap();
    mutate(json!({ "op": "createTask", "taskId": "t9", "title": "Shop search" }))
        .await
        .unwrap();
    assert!(
        mutate(json!({
            "op": "assignTask", "taskId": "t9", "chatId": "quiet", "orchestrate": true,
            "fromChatId": "ghost",
        }))
        .await
        .is_err()
    );
    let unled = core.workspace.task("t9").unwrap();
    assert_eq!(
        unled.chat_id, None,
        "a lead that never got its brief is unlinked"
    );
    assert_eq!(unled.mode, zeron_proto::TaskMode::Single);
    assert!(!unled.plan_pending);
}

#[tokio::test]
async fn a_mission_lead_reads_its_context_chats_first_and_the_list_locks_at_start() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("device-id"), "dev-a").unwrap();
    let prompts = Arc::new(Mutex::new(Vec::new()));
    let registry = HarnessRegistry::new();
    registry.register(Arc::new(Recording(prompts.clone())));
    let core = EngineCore::assemble(dir.path(), Arc::new(registry), HarnessId::Mock, None)
        .expect("engine core assembles");
    for (id, path) in [("home", "/tmp/home"), ("astra", "/tmp/astra")] {
        core.workspace
            .create_space(id, "dev-a", path, None, false)
            .unwrap();
    }
    for (id, space, title, device) in [
        ("review", Some("home"), "Deep Dive Code Review", "dev-a"),
        ("notes", Some("astra"), "Release notes", "dev-a"),
        ("far", None, "Windows build check", "dev-b"),
    ] {
        core.workspace
            .create_chat_from(id, space, Some(device), None, None, None)
            .unwrap();
        core.workspace.rename_chat(id, title).unwrap();
    }
    let rpc = core.rpc_service();
    let mutate = |params: serde_json::Value| {
        let rpc = rpc.clone();
        async move { rpc.handle(methods::MUTATE, params).await }
    };
    let six: Vec<String> = (1..=6).map(|n| format!("c{n}")).collect();
    assert!(
        mutate(json!({ "op": "createTask", "taskId": "t0", "title": "Too many", "context": six }))
            .await
            .is_err()
    );
    assert!(
        mutate(json!({ "op": "createTask", "taskId": "t0", "title": "Far", "context": ["far"] }))
            .await
            .is_err(),
        "read_thread only reaches chats on this device"
    );
    mutate(json!({
        "op": "createTask", "taskId": "t1", "title": "Fix the auth flow", "spaceId": "home",
        "context": ["review"],
    }))
    .await
    .unwrap();
    mutate(json!({
        "op": "updateTask", "taskId": "t1", "patch": { "context": ["review", "notes"] },
    }))
    .await
    .unwrap();
    mutate(json!({ "op": "startTask", "taskId": "t1", "chatId": "lead-1", "orchestrate": true }))
        .await
        .unwrap();
    wait_for(|| prompts.lock().unwrap().len() == 1, "the lead's turn").await;
    let first = prompts.lock().unwrap()[0].clone();
    assert!(
        first.starts_with(
            "Fix the auth flow\n\nBefore you plan, read these chats with read_thread. They are context only: do not message them or answer their questions.\n- \"Deep Dive Code Review\" (threadId review)\n- \"Release notes\" (threadId notes, project astra)\n\nYou lead this task (task id t1)."
        ),
        "{first}"
    );
    assert!(
        first.ends_with(
            "\n8. When a step needs one of the chats you read, name it with its threadId in that step's prompt."
        ),
        "{first}"
    );
    assert!(
        mutate(json!({
            "op": "updateTask", "taskId": "t1", "patch": { "context": ["review"] },
        }))
        .await
        .is_err(),
        "the list locks once the mission starts"
    );
    assert_eq!(
        core.workspace.task("t1").unwrap().context,
        ["review", "notes"]
    );

    core.workspace
        .create_chat_from("gone", None, Some("dev-a"), None, None, None)
        .unwrap();
    mutate(json!({
        "op": "createTask", "taskId": "t2", "title": "Later", "context": ["gone", "review"],
    }))
    .await
    .unwrap();
    core.workspace.delete_chat("gone").unwrap();
    for context in [json!(["gone"]), json!(["gone", "notes"])] {
        mutate(json!({ "op": "updateTask", "taskId": "t2", "patch": { "context": context } }))
            .await
            .expect("a deleted chat already on the list does not block edits to it");
    }
}

async fn supervisor_snapshot(
    rpc: &Arc<zeron_engine::EngineRpc>,
    ready: impl Fn(&zeron_proto::supervisor::SupervisorSnapshot) -> bool,
) -> zeron_proto::supervisor::SupervisorSnapshot {
    let RpcReply::Stream(mut watch) = rpc
        .handle(methods::WATCH_SUPERVISOR, json!({}))
        .await
        .unwrap()
    else {
        panic!("a snapshot stream")
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
async fn a_lead_escalation_is_one_item_and_one_wake_and_the_answer_reaches_the_lead_once() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("device-id"), "dev-a").unwrap();
    let prompts = Arc::new(Mutex::new(Vec::new()));
    let registry = HarnessRegistry::new();
    registry.register(Arc::new(Recording(prompts.clone())));
    let core = EngineCore::assemble(dir.path(), Arc::new(registry), HarnessId::Mock, None)
        .expect("engine core assembles");
    let rpc = core.rpc_service();
    let mutate = |params: serde_json::Value| {
        let rpc = rpc.clone();
        async move { rpc.handle(methods::MUTATE, params).await }
    };
    let count = |needle: &str| {
        prompts
            .lock()
            .unwrap()
            .iter()
            .filter(|prompt| prompt.contains(needle))
            .count()
    };
    let missing = rpc
        .handle(
            methods::ENSURE_SUPERVISOR,
            json!({ "config": { "harness": "codex", "sandbox": "workspace-write", "profileId": "slp-supervisor" } }),
        )
        .await
        .err()
        .expect("no Supervisor on an agent this Mac cannot run")
        .to_string();
    assert_eq!(
        missing,
        "Codex is not installed on this Mac. Install it, or give the slp-supervisor profile another agent."
    );
    let RpcReply::Value(ensured) = rpc
        .handle(
            methods::ENSURE_SUPERVISOR,
            json!({ "config": { "harness": "mock", "sandbox": "workspace-write" } }),
        )
        .await
        .unwrap()
    else {
        panic!("a chat id")
    };
    let supervisor = ensured["chatId"].as_str().unwrap().to_string();
    mutate(json!({ "op": "createTask", "taskId": "t1", "title": "Shop backend" }))
        .await
        .unwrap();
    mutate(json!({ "op": "startTask", "taskId": "t1", "chatId": "lead-1", "orchestrate": true }))
        .await
        .unwrap();
    wait_for(
        || count("You lead this task (task id t1)") == 1,
        "the lead's brief",
    )
    .await;
    core.workspace
        .create_chat_from("other", None, Some("dev-a"), None, None, None)
        .unwrap();

    let escalate = |from: Option<&str>| {
        let mut params = json!({
            "op": "escalateTask", "taskId": "t1", "title": "Pay for a Stripe test account?",
            "why": "spend", "detail": "The checkout tests need a live key.",
            "options": ["Pay $20", "Use the mock"],
        });
        if let Some(from) = from {
            params["fromChatId"] = json!(from);
        }
        mutate(params)
    };
    for from in [None, Some("other"), Some(supervisor.as_str())] {
        let Err(err) = escalate(from).await else {
            panic!("{from:?} cannot escalate")
        };
        let err = err.to_string();
        assert!(
            err.contains("Only the task's lead thread escalates"),
            "{err}"
        );
    }
    let RpcReply::Value(raised) = escalate(Some("lead-1")).await.unwrap() else {
        panic!("an item id")
    };
    let item_id = raised["itemId"].as_str().unwrap().to_string();
    let Err(again) = escalate(Some("lead-1")).await else {
        panic!("the same open question is not raised twice")
    };
    assert!(
        again
            .to_string()
            .contains("already in the person's Needs you list"),
        "{again}"
    );
    let snapshot = supervisor_snapshot(&rpc, |s| s.needs_you_count() > 0).await;
    let [zeron_proto::supervisor::NeedsYouRow::Item(item)] = snapshot.needs_you.as_slice() else {
        panic!("one item: {:?}", snapshot.needs_you)
    };
    assert_eq!(item.id, item_id);
    assert_eq!(item.raised_by, zeron_proto::supervisor::Raiser::Lead);
    assert_eq!(item.options, ["Pay $20", "Use the mock"]);
    wait_for(|| count("Lead escalated") == 1, "the Supervisor's wake").await;
    let wake = prompts
        .lock()
        .unwrap()
        .iter()
        .find(|prompt| prompt.contains("Lead escalated"))
        .cloned()
        .unwrap();
    assert!(wake.starts_with("Supervisor wake 1 of 12"), "{wake}");
    assert!(
        wake.contains(&format!("itemId {item_id} · why: spend")),
        "{wake}"
    );
    assert!(
        wake.contains("Options: \"Pay $20\" | \"Use the mock\"\n</agent-response>"),
        "{wake}"
    );
    assert!(
        wake.contains(
            "   The person answers it on the Supervisor page; their choice goes to the lead.\n"
        ),
        "{wake}"
    );
    assert!(
        wake.contains("A change to a mission's goal or spend is the person's decision")
            && !wake.contains("action \"decide\""),
        "with the switches off the wake is unchanged: {wake}"
    );
    for action in [
        json!({ "action": "decide", "itemId": item_id, "choice": "Use the mock", "reason": "cheaper" }),
        json!({ "action": "approve", "taskId": "t1", "reason": "fine" }),
        json!({ "action": "goal", "taskId": "t1", "title": "Shop", "reason": "shorter" }),
    ] {
        let mut params = action.clone();
        params["fromChatId"] = json!(supervisor);
        let Err(err) = rpc.handle(methods::SUPERVISOR_ACT, params).await else {
            panic!("{action} is the person's while the switches are off")
        };
        assert!(
            err.to_string()
                .contains("is off in the Supervisor's Authority"),
            "{err}"
        );
    }

    let answered = "The person answered Needs-you item";
    mutate(json!({ "op": "resolveSupervisorItem", "itemId": item_id, "choice": "Use the mock" }))
        .await
        .unwrap();
    wait_for(|| count(answered) == 1, "the lead's answer").await;
    assert!(
        mutate(json!({ "op": "resolveSupervisorItem", "itemId": item_id, "choice": "Pay $20" }))
            .await
            .is_err(),
        "an item is answered once"
    );

    let RpcReply::Value(asked) = rpc
        .handle(
            methods::SUPERVISOR_ACT,
            json!({
                "fromChatId": supervisor, "action": "ask", "taskId": "t1",
                "title": "Drop the Adyen step?", "why": "goal", "options": ["Drop it", "Keep it"],
            }),
        )
        .await
        .unwrap()
    else {
        panic!("an item id")
    };
    let asked = asked["itemId"].as_str().unwrap().to_string();
    mutate(json!({ "op": "resolveSupervisorItem", "itemId": asked, "choice": "Keep it" }))
        .await
        .unwrap();
    wait_for(
        || count("The person answered your Needs-you item") == 1,
        "the Supervisor's answer",
    )
    .await;
    wait_for(|| count(answered) == 2, "the lead's second answer").await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    let to_lead: Vec<String> = prompts
        .lock()
        .unwrap()
        .iter()
        .filter(|prompt| prompt.contains(answered))
        .cloned()
        .collect();
    assert_eq!(
        to_lead,
        [
            format!(
                "{answered} {item_id} \"Pay for a Stripe test account?\", raised by the lead: Use the mock\n\nApply it to this mission (task id t1)."
            ),
            format!(
                "{answered} {asked} \"Drop the Adyen step?\", raised by the Supervisor: Keep it\n\nApply it to this mission (task id t1)."
            ),
        ]
    );
    assert_eq!(count("The person answered your Needs-you item"), 1);
    assert_eq!(
        count("Supervisor wake"),
        1,
        "the Supervisor's own item wakes nobody"
    );

    let mut batch = Vec::new();
    for title in ["Who reviews?", "Include tests?"] {
        let RpcReply::Value(raised) = mutate(json!({
            "op": "escalateTask", "taskId": "t1", "fromChatId": "lead-1", "title": title,
            "why": "decision", "options": ["Yes", "No"],
        }))
        .await
        .unwrap() else {
            panic!("an item id")
        };
        batch.push(raised["itemId"].as_str().unwrap().to_string());
    }
    supervisor_snapshot(&rpc, |s| s.needs_you_count() == 2).await;
    mutate(json!({ "op": "resolveSupervisorItems", "answers": [
        { "itemId": batch[0], "choice": "Codex" },
        { "itemId": batch[1], "choice": "No" },
    ] }))
    .await
    .unwrap();
    let together = "The person answered 2 Needs-you items";
    wait_for(|| count(together) == 1, "the lead's batched answer").await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(
        count(answered),
        2,
        "a batch is one message, not one per item"
    );
    let sent = prompts
        .lock()
        .unwrap()
        .iter()
        .find(|prompt| prompt.contains(together))
        .cloned()
        .unwrap();
    assert_eq!(
        sent,
        format!(
            "{together}:\n- item {} \"Who reviews?\", raised by the lead: Codex\n- item {} \"Include tests?\", raised by the lead: No\n\nApply them to this mission (task id t1).",
            batch[0], batch[1]
        )
    );
    assert!(
        mutate(json!({ "op": "resolveSupervisorItems", "answers": [
            { "itemId": batch[0], "choice": "Yes" },
        ] }))
        .await
        .is_err(),
        "a batched item is answered once"
    );
    let snapshot = supervisor_snapshot(&rpc, |s| s.needs_you_count() == 0).await;
    assert!(snapshot.needs_you.is_empty());
    core.shutdown().await;
}

#[tokio::test]
async fn full_authority_lets_the_supervisor_decide_approve_and_change_the_goal_and_the_person_corrects_it()
 {
    use zeron_proto::supervisor::{InterventionKind, NeedsYouRow, PlanApproval};
    unsafe { std::env::set_var("ZERON_SUPERVISOR_APPROVAL_HOLD_MS", APPROVAL_HOLD_MS) };

    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("device-id"), "dev-a").unwrap();
    let prompts = Arc::new(Mutex::new(Vec::new()));
    let registry = HarnessRegistry::new();
    registry.register(Arc::new(Recording(prompts.clone())));
    let core = EngineCore::assemble(dir.path(), Arc::new(registry), HarnessId::Mock, None)
        .expect("engine core assembles");
    let rpc = core.rpc_service();
    let mutate = |params: serde_json::Value| {
        let rpc = rpc.clone();
        async move { rpc.handle(methods::MUTATE, params).await }
    };
    let count = |needle: &str| {
        prompts
            .lock()
            .unwrap()
            .iter()
            .filter(|prompt| prompt.contains(needle))
            .count()
    };
    let sent = |needle: &str| {
        prompts
            .lock()
            .unwrap()
            .iter()
            .find(|prompt| prompt.contains(needle))
            .cloned()
            .unwrap()
    };
    let act = |supervisor: String, action: serde_json::Value| {
        let rpc = rpc.clone();
        let mut params = action;
        params["fromChatId"] = json!(supervisor);
        async move { rpc.handle(methods::SUPERVISOR_ACT, params).await }
    };
    mutate(json!({ "op": "createTask", "taskId": "t1", "title": "Deep Dive review" }))
        .await
        .unwrap();
    mutate(json!({ "op": "startTask", "taskId": "t1", "chatId": "lead-1", "orchestrate": true }))
        .await
        .unwrap();
    wait_for(
        || count("You lead this task (task id t1)") == 1,
        "the lead's brief",
    )
    .await;
    let mut items = Vec::new();
    for (title, why) in [
        ("Who reviews?", "decision"),
        ("Pay for a second model?", "spend"),
    ] {
        let RpcReply::Value(raised) = mutate(json!({
            "op": "escalateTask", "taskId": "t1", "fromChatId": "lead-1", "title": title,
            "why": why, "options": ["Claude", "Codex"],
        }))
        .await
        .unwrap() else {
            panic!("an item id")
        };
        items.push(raised["itemId"].as_str().unwrap().to_string());
    }
    let (decision, spend) = (items[0].clone(), items[1].clone());
    mutate(json!({
        "op": "planSteps", "taskId": "t1", "fromChatId": "lead-1",
        "steps": [{ "title": "Review", "notes": "Read the diff" }],
    }))
    .await
    .unwrap();
    let RpcReply::Value(ensured) = rpc
        .handle(
            methods::ENSURE_SUPERVISOR,
            json!({ "config": { "harness": "mock", "sandbox": "workspace-write" } }),
        )
        .await
        .unwrap()
    else {
        panic!("a chat id")
    };
    let supervisor = ensured["chatId"].as_str().unwrap().to_string();

    let act = |action: serde_json::Value| act(supervisor.clone(), action);
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(
        count("Supervisor wake"),
        0,
        "a plan waiting for review wakes nobody while Decide for me is off"
    );

    rpc.handle(
        methods::SET_SUPERVISOR_SETTINGS,
        json!({ "decideForMe": true }),
    )
    .await
    .unwrap();
    wait_for(
        || count("Plan ready for review") == 1 && count("Lead escalated") == 1,
        "the wake that hands the Supervisor the plan and the decision",
    )
    .await;
    let wake = sent("Plan ready for review");
    assert!(wake.contains("Lead escalated"), "{wake}");
    assert!(wake.contains(&format!("itemId {decision}")), "{wake}");
    assert!(
        !wake.contains(&format!("itemId {spend}")),
        "a spend item stays with the person: {wake}"
    );
    assert!(
        wake.contains("   The person gave you this decision: decide it with the supervisor tool, action \"decide\""),
        "{wake}"
    );
    assert!(
        wake.contains("approve it with the supervisor tool, action \"approve\"")
            && wake.contains("action \"approve\" (taskId, reason) when it fits the mission: on")
            && wake.contains("A change to a mission's goal or spend is the person's decision"),
        "{wake}"
    );

    let refused =
        act(json!({ "action": "decide", "itemId": spend, "choice": "Codex", "reason": "cheap" }))
            .await
            .err()
            .expect("refused")
            .to_string();
    assert!(
        refused.contains("Changing a mission's goal or spend is off"),
        "{refused}"
    );
    let refused =
        act(json!({ "action": "decide", "itemId": decision, "choice": "Gemini", "reason": "x" }))
            .await
            .err()
            .expect("refused")
            .to_string();
    assert!(
        refused.contains("choice is one of the item's options"),
        "{refused}"
    );
    act(json!({
        "action": "decide", "itemId": decision, "choice": "Codex",
        "reason": "Codex reviewed the last two diffs of this repo",
    }))
    .await
    .unwrap();
    let decided = "The Supervisor decided Needs-you item";
    wait_for(
        || count(decided) == 1,
        "the lead's answer from the Supervisor",
    )
    .await;
    assert_eq!(
        sent(decided),
        format!(
            "{decided} {decision} \"Who reviews?\", raised by the lead: Codex\n\nWhy: Codex reviewed the last two diffs of this repo\n\nThe person gave the Supervisor this decision. Apply it to this mission (task id t1). The person may change it; a change reaches you as a correction."
        ),
        "the lead gets it the way it gets the person's answer, naming who decided and why"
    );

    let RpcReply::Value(approved) =
        act(json!({ "action": "approve", "taskId": "t1", "reason": "One small step, in scope" }))
            .await
            .unwrap()
    else {
        panic!("an approval")
    };
    assert!(approved["startsAt"].is_string(), "{approved}");
    assert!(!core.workspace.task("t1").unwrap().plan_pending);
    let held = supervisor_snapshot(&rpc, |s| !s.interventions.is_empty()).await;
    assert!(
        matches!(
            held.interventions[0].approval,
            Some(PlanApproval::Held { .. })
        ),
        "{:?}",
        held.interventions[0]
    );
    assert_eq!(
        count("The Supervisor approved the plan"),
        0,
        "the lead hears of a Supervisor approval only once its hold runs out"
    );
    wait_for(
        || count("The Supervisor approved the plan; the person gave it this decision. Run it.\n\nWhy: One small step, in scope") == 1,
        "the approval",
    )
    .await;
    let sent = supervisor_snapshot(&rpc, |s| {
        s.interventions[0].approval == Some(PlanApproval::Sent)
    })
    .await;
    assert!(sent.interventions[0].message_id.is_some());
    assert!(
        act(json!({ "action": "approve", "taskId": "t1", "reason": "again" }))
            .await
            .is_err(),
        "an approved plan is not waiting"
    );

    let snapshot = supervisor_snapshot(&rpc, |s| s.interventions.len() == 2).await;
    let [approve, decide] = snapshot.interventions.as_slice() else {
        unreachable!()
    };
    assert_eq!(approve.kind, InterventionKind::Approve);
    assert_eq!(approve.text, "One small step, in scope");
    assert_eq!(decide.kind, InterventionKind::Decide);
    assert_eq!(decide.target.as_deref(), Some("lead-1"));
    let item = decide.item.as_ref().expect("the decision carries its item");
    let answer = item.answer.as_ref().unwrap();
    assert_eq!(
        (answer.choice.as_str(), answer.corrected()),
        ("Codex", false)
    );
    assert!(
        snapshot
            .needs_you
            .iter()
            .all(|row| !matches!(row, NeedsYouRow::Item(item) if item.id == decision)),
        "a decided item leaves Needs you"
    );

    mutate(json!({ "op": "resolveSupervisorItem", "itemId": decision, "choice": "Claude" }))
        .await
        .unwrap();
    let corrected = format!(
        "The person answered Needs-you item {decision} \"Who reviews?\", raised by the lead: Claude (a correction: this replaces the Supervisor's \"Codex\")\n\nApply it to this mission (task id t1). If you already acted on the Supervisor's choice, undo or adjust what depends on it where you can, and tell the person what could not be undone."
    );
    wait_for(|| count(&corrected) == 1, "the lead's correction").await;
    wait_for(
        || count(&format!("The person changed your decisions; each lead got the correction:\n- item {decision} \"Who reviews?\": Claude, replacing your \"Codex\"")) == 1,
        "the Supervisor hears its decision changed",
    )
    .await;
    let snapshot = supervisor_snapshot(&rpc, |s| {
        s.interventions.iter().any(|row| {
            row.item
                .as_ref()
                .and_then(|i| i.answer.as_ref())
                .is_some_and(|a| a.corrected())
        })
    })
    .await;
    assert_eq!(snapshot.interventions.len(), 2, "a correction adds no row");
    assert!(
        mutate(json!({ "op": "resolveSupervisorItem", "itemId": decision, "choice": "Claude" }))
            .await
            .is_err(),
        "the same choice again is no correction"
    );

    rpc.handle(
        methods::SET_SUPERVISOR_SETTINGS,
        json!({ "decideForMe": true, "goalAndSpend": true }),
    )
    .await
    .unwrap();
    act(json!({ "action": "decide", "itemId": spend, "choice": "Codex", "reason": "Within the plan's budget" }))
        .await
        .unwrap();
    wait_for(
        || count(&format!("{decided} {spend} \"Pay for a second model?\"")) == 1,
        "the spend decision",
    )
    .await;
    act(json!({
        "action": "goal", "taskId": "t1", "title": "Deep Dive review of the auth diff",
        "reason": "The person said the auth diff is the one that matters",
    }))
    .await
    .unwrap();
    assert_eq!(
        core.workspace.task("t1").unwrap().title,
        "Deep Dive review of the auth diff"
    );
    wait_for(
        || count("The Supervisor changed this mission's goal (task id t1), with the person's leave:\n- The title is now \"Deep Dive review of the auth diff\" (it was \"Deep Dive review\").\n\nWhy: The person said the auth diff is the one that matters") == 1,
        "the goal change",
    )
    .await;
    let refusal = act(json!({ "action": "goal", "taskId": "t1", "title": "Deep Dive review of the auth diff", "reason": "same" }))
        .await
        .err().expect("refused")
        .to_string();
    assert!(
        refusal.contains("already this mission's title"),
        "{refusal}"
    );
    let snapshot = supervisor_snapshot(&rpc, |s| s.interventions.len() == 4).await;
    assert_eq!(
        snapshot
            .interventions
            .iter()
            .map(|row| row.kind)
            .collect::<Vec<_>>(),
        [
            InterventionKind::Goal,
            InterventionKind::Decide,
            InterventionKind::Approve,
            InterventionKind::Decide,
        ]
    );
    core.shutdown().await;
}

#[tokio::test]
async fn the_person_takes_back_a_supervisor_plan_approval_until_a_step_starts() {
    use zeron_proto::supervisor::PlanApproval;
    unsafe { std::env::set_var("ZERON_SUPERVISOR_APPROVAL_HOLD_MS", APPROVAL_HOLD_MS) };

    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("device-id"), "dev-a").unwrap();
    let prompts = Arc::new(Mutex::new(Vec::new()));
    let registry = HarnessRegistry::new();
    registry.register(Arc::new(Recording(prompts.clone())));
    let core = EngineCore::assemble(dir.path(), Arc::new(registry), HarnessId::Mock, None)
        .expect("engine core assembles");
    let rpc = core.rpc_service();
    let mutate = |params: serde_json::Value| {
        let rpc = rpc.clone();
        async move { rpc.handle(methods::MUTATE, params).await }
    };
    let count = |needle: &str| {
        prompts
            .lock()
            .unwrap()
            .iter()
            .filter(|prompt| prompt.contains(needle))
            .count()
    };
    let RpcReply::Value(ensured) = rpc
        .handle(
            methods::ENSURE_SUPERVISOR,
            json!({ "config": { "harness": "mock", "sandbox": "workspace-write" } }),
        )
        .await
        .unwrap()
    else {
        panic!("a chat id")
    };
    let supervisor = ensured["chatId"].as_str().unwrap().to_string();
    rpc.handle(
        methods::SET_SUPERVISOR_SETTINGS,
        json!({ "decideForMe": true }),
    )
    .await
    .unwrap();
    let approve = |task: &str| {
        let rpc = rpc.clone();
        let params = json!({
            "fromChatId": supervisor, "action": "approve", "taskId": task, "reason": "Small and in scope",
        });
        async move { rpc.handle(methods::SUPERVISOR_ACT, params).await }
    };
    let approval_of = |task: &'static str| {
        let rpc = rpc.clone();
        async move {
            let snapshot = supervisor_snapshot(&rpc, |_| true).await;
            snapshot
                .interventions
                .into_iter()
                .find(|row| row.task_id.as_deref() == Some(task))
                .and_then(|row| row.approval)
        }
    };
    for task in ["t1", "t2", "t3", "t4"] {
        let lead = format!("lead-{task}");
        mutate(json!({ "op": "createTask", "taskId": task, "title": format!("Mission {task}") }))
            .await
            .unwrap();
        mutate(json!({ "op": "startTask", "taskId": task, "chatId": lead, "orchestrate": true }))
            .await
            .unwrap();
        wait_for(
            || count(&format!("You lead this task (task id {task})")) == 1,
            "the lead's brief",
        )
        .await;
        mutate(json!({
            "op": "planSteps", "taskId": task, "fromChatId": lead,
            "steps": [{ "title": "Build", "notes": "Write the file" }],
        }))
        .await
        .unwrap();
    }
    let approved = "The Supervisor approved the plan";
    let took_back = "The person took back the Supervisor's approval";

    approve("t1").await.unwrap();
    assert!(!core.workspace.task("t1").unwrap().plan_pending);
    assert!(matches!(
        approval_of("t1").await,
        Some(PlanApproval::Held { .. })
    ));
    mutate(json!({ "op": "takeBackPlanApproval", "taskId": "t1" }))
        .await
        .unwrap();
    assert!(
        core.workspace.task("t1").unwrap().plan_pending,
        "the plan waits for review again"
    );
    assert!(matches!(
        approval_of("t1").await,
        Some(PlanApproval::TakenBack { .. })
    ));
    tokio::time::sleep(Duration::from_millis(2_500)).await;
    assert_eq!(
        (count(approved), count(took_back)),
        (0, 0),
        "a hold taken back never reaches the lead, so it hears nothing"
    );
    let refusal = approve("t1").await.err().expect("refused").to_string();
    assert!(refusal.contains("took back your approval"), "{refusal}");
    let again = mutate(json!({ "op": "takeBackPlanApproval", "taskId": "t1" }))
        .await
        .err()
        .expect("refused")
        .to_string();
    assert!(again.contains("already taken back"), "{again}");

    approve("t2").await.unwrap();
    wait_for(
        || count(approved) == 1,
        "the approval once the hold runs out",
    )
    .await;
    assert_eq!(approval_of("t2").await, Some(PlanApproval::Sent));
    mutate(json!({ "op": "takeBackPlanApproval", "taskId": "t2" }))
        .await
        .unwrap();
    assert!(core.workspace.task("t2").unwrap().plan_pending);
    wait_for(
        || count(took_back) == 1,
        "the lead hears the approval is gone",
    )
    .await;
    assert!(
        prompts.lock().unwrap().iter().any(|prompt| prompt
            == "The person took back the Supervisor's approval of this plan (task id t2). The plan waits for the person's review again: do not run any step until the approval message comes."),
    );

    approve("t3").await.unwrap();
    wait_for(|| count(approved) == 2, "the third approval").await;
    let step = core.workspace.task_steps_of("t3")[0].id.clone();
    mutate(json!({ "op": "updateTaskStep", "stepId": step, "patch": { "outcome": "done" } }))
        .await
        .unwrap();
    let late = mutate(json!({ "op": "takeBackPlanApproval", "taskId": "t3" }))
        .await
        .err()
        .expect("refused")
        .to_string();
    assert!(late.contains("A step already started"), "{late}");
    assert!(!core.workspace.task("t3").unwrap().plan_pending);
    assert_eq!(approval_of("t3").await, Some(PlanApproval::Sent));

    approve("t4").await.unwrap();
    assert!(matches!(
        approval_of("t4").await,
        Some(PlanApproval::Held { .. })
    ));
    mutate(json!({ "op": "editTaskPlan", "taskId": "t4", "overview": "Build it, then test it" }))
        .await
        .unwrap();
    assert!(
        core.workspace.task("t4").unwrap().plan_pending,
        "an edit during the hold sends the plan back to review"
    );
    assert!(!matches!(
        approval_of("t4").await,
        Some(PlanApproval::Held { .. })
    ));
    tokio::time::sleep(Duration::from_millis(2_500)).await;
    assert_eq!(
        count(approved),
        2,
        "the withdrawn approval never reaches the lead"
    );
    core.shutdown().await;
}
