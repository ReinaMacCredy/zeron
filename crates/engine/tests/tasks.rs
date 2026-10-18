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

    let start =
        json!({ "op": "startTask", "taskId": "t1", "chatId": "chat-t1", "spaceId": "space-1" });
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
    for id in ["t1", "t2", "t3"] {
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
    let run = "Run each step with delegate_task: task set to what the step must do, taskStepId set to the step's id. Steps that do not depend on each other can run at the same time.";
    let tail = "When a step's thread finishes, check its result. Mark a step done or skipped with the tasks tool, action \"step\", or run it again with delegate_task and the same taskStepId.\n{n}. When every step is done, check the whole result, tell the person what was built, and mark the task completed with the tasks tool, action \"update\", completed true.";
    assert_eq!(
        prompts.lock().unwrap().as_slice(),
        [
            format!(
                "{}2. Then stop. The person reviews the plan in Tasks and approves it; you get a message when it is approved.\n3. {run}\n4. Pick the harness and model for each step; orchestrator_capabilities lists them.\n5. {}",
                header("t1"),
                tail.replace("{n}", "6")
            ),
            format!(
                "{}2. {run}\n3. Run every step on your own harness and model: do not pass harness or model to delegate_task.\n4. {}",
                header("t2"),
                tail.replace("{n}", "5")
            ),
        ]
    );
    let led = core.workspace.task("t1").unwrap();
    assert_eq!(led.mode, zeron_proto::TaskMode::Orchestrated);
    assert!(led.plan_pending);
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
