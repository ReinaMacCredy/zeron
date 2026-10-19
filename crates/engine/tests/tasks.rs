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
            "Shop backend\n\nYou lead this task (task id {id}). Break it into steps and run each step on its own thread.\n\n1. Write the plan with the tasks tool: action \"plan\", taskId \"{id}\", steps [{{title, notes}}]. Keep each step small enough for one agent.\n"
        )
    };
    let run = "Run each step on its own thread with create_thread: prompt set to what the step must do, taskStepId set to the step's id, title optional. Steps that do not depend on each other can run at the same time.";
    let tail = "Zeron messages you when a step's thread finishes a turn or fails. Read the thread with read_thread, then mark the step done or skipped with the tasks tool, action \"step\", send the thread a fix with send_message_to_thread, or start the step over with create_thread and the same taskStepId.\n{n}. When a decision changes the goal or costs money the person has not approved, or you cannot settle it from the plan, use the tasks tool action \"escalate\" and keep working on what does not depend on it.\n{m}. When every step is done, check the whole result, tell the person what was built, and mark the task completed with the tasks tool, action \"update\", completed true.";
    assert_eq!(
        prompts.lock().unwrap().as_slice(),
        [
            format!(
                "{}2. Then stop. The person reviews the plan in Tasks and approves it; you get a message when it is approved.\n3. {run}\n4. Pick the harness and model for each step; orchestrator_capabilities lists them.\n5. {}",
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
        doc_brief.contains("taskId \"t4\", overview, steps [{title, notes}]. The overview is the goal, the approach, and how you will check the result. Each step's notes are its spec: what it builds, where, and how to tell it is done."),
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
    assert!(wake.contains("Options: Pay $20 | Use the mock"), "{wake}");

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
    let snapshot = supervisor_snapshot(&rpc, |s| s.needs_you_count() == 0).await;
    assert!(snapshot.needs_you.is_empty());
    core.shutdown().await;
}
