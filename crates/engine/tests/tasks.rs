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
    assert_eq!(config.access, zeron_proto::AccessMode::AutoAcceptEdits);

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
async fn gathered_tasks_share_one_new_thread_and_one_listing_message() {
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
    for (id, title) in [("g1", "Fix login"), ("g2", "Add tests"), ("g3", "Ship it")] {
        mutate(json!({ "op": "createTask", "taskId": id, "title": title, "spaceId": "space-1" }))
            .await
            .unwrap();
    }

    mutate(json!({
        "op": "startTasks", "taskIds": ["g1", "g2"], "chatId": "gather",
        "config": {
            "harness": "mock", "model": "rec-model", "reasoning": "low",
            "sandbox": "workspace-write", "permissionMode": "auto",
        },
    }))
    .await
    .unwrap();
    wait_for(|| !prompts.lock().unwrap().is_empty(), "the gathered turn").await;
    assert_eq!(
        prompts.lock().unwrap()[0],
        "Work through these 2 tasks in this thread, in order. Mark each one done with the \
         `tasks` tool when it is finished.\n\n1. [Fix login](zeron-task:g1)\n2. [Add tests](zeron-task:g2)"
    );
    for id in ["g1", "g2"] {
        assert_eq!(
            core.workspace.task(id).unwrap().chat_id.as_deref(),
            Some("gather")
        );
    }
    let chat = core.workspace.chat("gather").unwrap().unwrap();
    assert_eq!(chat.title.as_deref(), Some("Fix login +1"));
    assert_eq!(chat.space_id.as_deref(), Some("space-1"));

    mutate(json!({ "op": "addTasksToThread", "taskIds": ["g3"], "chatId": "gather" }))
        .await
        .unwrap();
    wait_for(|| prompts.lock().unwrap().len() == 2, "the added task").await;
    assert_eq!(
        prompts.lock().unwrap()[1],
        "Also take this task in this thread. Mark it done with the `tasks` tool when it is \
         finished.\n\n1. [Ship it](zeron-task:g3)"
    );
    assert_eq!(
        core.workspace.task("g3").unwrap().chat_id.as_deref(),
        Some("gather")
    );
    assert!(
        mutate(json!({ "op": "startTasks", "taskIds": [], "chatId": "none" }))
            .await
            .is_err()
    );
    assert!(core.workspace.chat("none").unwrap().is_none());
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
            "Shop backend\n\nYou lead this task (task id {id}). Break it into steps and run each step on its own thread.\n\n1. Write the plan with the plan tool: action \"write\", steps [{{title, notes, role, target}}]. Keep each step small enough for one agent.\n"
        )
    };
    let run = "Run each step on its own thread with delegate_task: task set to what the step must do, stepId set to the step's id. Steps that do not depend on each other can run at the same time.";
    let tail = "Zeron messages you when a step's thread finishes a turn or fails. Read the thread with read_thread, then mark the step done or skipped with the plan tool, action \"step\", send the thread a fix with send_message_to_thread, or start the step over with delegate_task and the same stepId. When a step fails, first make sure it can verify its own work, then raise its effort one level, then move it to the next model up at a lower effort.\n{n}. When every step is done, check the whole result, tell the person what was built, and mark the task completed with the tasks tool, action \"update\", completed true.";
    assert_eq!(
        prompts.lock().unwrap().as_slice(),
        [
            format!(
                "{}2. Then stop. The person reviews the plan and approves it; you get a message when it is approved.\n3. {run}\n4. Give each step a role (worker, validator, research, design or general). Choose each step's model and effort from the notes in orchestrator_capabilities, the cheapest that is enough, and write it as the step's target in the plan, with a one-line reason in its notes; a step with no target runs on its role's route in Settings > Routing.\n5. {}",
                header("t1"),
                tail.replace("{n}", "6")
            ),
            format!(
                "{}2. {run}\n3. Run every step on your own harness and model: do not pass target or profile to delegate_task.\n4. {}",
                header("t2"),
                tail.replace("{n}", "5")
            ),
        ]
    );
    let lead = |id: &str| core.workspace.chat(id).unwrap().unwrap();
    let led = lead("lead-1");
    assert!(
        led.plan_pending,
        "Plan first opens the lead chat waiting for approval"
    );
    assert!(
        !led.plan.expect("the lead chat holds the plan").plan_doc,
        "a start without planDoc is steps only"
    );
    assert_eq!(
        core.workspace.task("t1").unwrap().chat_id.as_deref(),
        Some("lead-1")
    );

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
        doc_brief.contains("action \"write\", overview, steps [{title, notes, role, target}]. The overview is the goal, the approach, and how you will check the result. Each step's notes are its spec: what it builds, where, and how to tell it is done."),
        "{doc_brief}"
    );
    assert!(lead("lead-4").plan.unwrap().plan_doc);
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
        ) && brief.contains("action \"write\", overview"),
        "{brief}"
    );
    assert_eq!(
        core.workspace.task("t5").unwrap().chat_id.as_deref(),
        Some("talk")
    );
    let talk = lead("talk");
    assert!(talk.plan_pending && talk.plan.unwrap().plan_doc);
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
    let refusal = |reply: Result<RpcReply, zeron_rpc::RpcError>| match reply {
        Err(err) => err.to_string(),
        Ok(_) => panic!("expected a refusal"),
    };
    let err = refusal(
        mutate(json!({
            "op": "assignTask", "taskId": "t6", "chatId": "talk", "orchestrate": true,
        }))
        .await,
    );
    assert!(err.contains("already has a plan"), "{err}");
    assert_eq!(core.workspace.task("t6").unwrap().chat_id, None);
    assert_eq!(prompts.lock().unwrap().len(), 4);
    let err = refusal(
        mutate(json!({
            "op": "planSteps", "chatId": "talk", "fromChatId": "lead-1",
            "steps": [{ "title": "Build the API" }],
        }))
        .await,
    );
    assert!(err.contains("writes only its own plan"), "{err}");
    assert!(
        core.workspace.steps_of("talk").is_empty(),
        "a refused plan writes no steps"
    );
    mutate(json!({
        "op": "planSteps", "chatId": "talk", "fromChatId": "talk",
        "steps": [{ "title": "Build the admin" }],
    }))
    .await
    .expect("a chat writes its own plan");
    let talk = lead("talk");
    assert!(
        talk.plan_pending && talk.plan.unwrap().plan_doc,
        "a plan write keeps the approval flag and the start's flags"
    );
    assert_eq!(core.workspace.steps_of("talk").len(), 1);
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

    let unapproved = lead("lead-2");
    assert!(unapproved.plan.unwrap().same_agents);
    assert!(!unapproved.plan_pending);
    assert!(
        mutate(json!({ "op": "approvePlan", "chatId": "lead-1" }))
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
    let Err(late) = mutate(json!({
        "op": "assignTask", "taskId": "t3", "chatId": "talk", "send": false, "claim": true,
    }))
    .await
    else {
        panic!("a claim on a held task is refused")
    };
    assert!(late.to_string().contains("lead-1"), "{late}");
    assert_eq!(
        core.workspace.task("t3").unwrap().chat_id.as_deref(),
        Some("lead-1"),
        "a claim that lost the race leaves the holder"
    );
    mutate(json!({
        "op": "assignTask", "taskId": "t3", "chatId": "lead-1", "send": false, "claim": true,
    }))
    .await
    .unwrap();

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
    let quiet = lead("quiet");
    assert!(quiet.plan.is_none() && !quiet.plan_pending);
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
            "\n7. When a step needs one of the chats you read, name it with its threadId in that step's task."
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

#[tokio::test]
async fn a_planning_to_do_lists_the_progress_of_its_lead_chats_plan() {
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
    for (id, title) in [("t1", "Ship the shop"), ("t2", "Plain to-do")] {
        mutate(json!({ "op": "createTask", "taskId": id, "title": title }))
            .await
            .unwrap();
    }
    mutate(json!({ "op": "startTask", "taskId": "t1", "chatId": "lead-1", "orchestrate": true }))
        .await
        .unwrap();
    wait_for(|| prompts.lock().unwrap().len() == 1, "the lead's turn").await;
    mutate(json!({
        "op": "planSteps", "chatId": "lead-1", "fromChatId": "lead-1",
        "steps": [{ "title": "API" }, { "title": "Docs" }, { "title": "Ads" }],
    }))
    .await
    .unwrap();
    let steps = core.workspace.steps_of("lead-1");
    for (step, outcome) in [(&steps[0], "done"), (&steps[2], "skipped")] {
        mutate(json!({
            "op": "updateTaskStep", "stepId": step.id, "patch": { "outcome": outcome },
            "fromChatId": "lead-1",
        }))
        .await
        .unwrap();
    }
    assert!(
        mutate(json!({
            "op": "updateTaskStep", "stepId": steps[1].id, "patch": { "outcome": "done" },
            "fromChatId": "t2-chat",
        }))
        .await
        .is_err(),
        "only the chat whose plan holds a step marks it"
    );

    let tools = zeron_mcp::ThreadTools::new(zeron_rpc::memory_client(rpc.clone()), "lead-1");
    let listed = tools
        .call("tasks", &json!({ "action": "list" }))
        .await
        .unwrap();
    let rows = listed["tasks"].as_array().unwrap();
    let row = |id: &str| rows.iter().find(|row| row["id"] == id).unwrap().clone();
    assert_eq!(row("t1")["progress"], json!({ "done": 1, "total": 2 }));
    assert!(
        row("t2").get("progress").is_none(),
        "a plain to-do has no progress"
    );
    let plan = tools
        .call("plan", &json!({ "action": "read" }))
        .await
        .unwrap();
    assert_eq!(plan["planPending"], json!(true));
    let statuses: Vec<&str> = plan["steps"]
        .as_array()
        .unwrap()
        .iter()
        .map(|step| step["status"].as_str().unwrap())
        .collect();
    assert_eq!(statuses, ["done", "planned", "skipped"]);
}
