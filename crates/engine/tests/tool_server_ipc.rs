//! The thread tools over real IPC, dialled with the token file a run's tool
//! server is given, as `zeron mcp` does: every tool's method must be open to it.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use futures::StreamExt;
use futures::stream::BoxStream;
use serde_json::json;

use zeron_doc::{SessionCommandEntry, SessionCommandPayload, SessionCommandStatus};
use zeron_engine::{EngineCore, HarnessRegistry};
use zeron_harness::{Harness, HarnessError, RunControls};
use zeron_proto::{
    AccessMode, AgentEvent, DoneStatus, HarnessId, Model, ReasoningLevel, RunRequest, SandboxLevel,
    SteeringMode,
};
use zeron_rpc::methods;

const LEAD: &str = "lead";

type Env = Vec<(String, String)>;

struct EnvHarness(Arc<Mutex<Vec<Env>>>);

#[async_trait]
impl Harness for EnvHarness {
    fn id(&self) -> HarnessId {
        HarnessId::Mock
    }
    fn display_name(&self) -> &str {
        "Env probe"
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
        let env = controls
            .tool_server
            .as_ref()
            .map(|server| server.env.clone())
            .unwrap_or_default();
        self.0.lock().unwrap().push(env);
        let interrupt = controls.interrupt.clone();
        Ok(futures::stream::once(async move {
            interrupt.cancelled().await;
            Ok(AgentEvent::Done {
                status: DoneStatus::Interrupted,
                result: None,
                error: None,
                session_id: None,
            })
        })
        .boxed())
    }
}

fn run_payload() -> SessionCommandPayload {
    SessionCommandPayload::Run {
        request: RunRequest {
            code_mode: false,
            zeron_mcp_only: false,
            prompt: "stay".into(),
            harness: None,
            model: None,
            reasoning: None,
            model_options: Default::default(),
            cwd: "/tmp".into(),
            sandbox: SandboxLevel::WorkspaceWrite,
            access: AccessMode::FullAccess,
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
        message_id: "m-lead".into(),
    }
}

fn value_of<'a>(env: &'a Env, name: &str) -> &'a str {
    env.iter()
        .find(|(key, _)| key == name)
        .map(|(_, value)| value.as_str())
        .unwrap_or_else(|| panic!("the tool server has no {name}"))
}

#[tokio::test]
async fn the_tool_server_reaches_its_tools_with_the_token_it_is_given() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("device-id"), "dev-a").unwrap();
    let envs: Arc<Mutex<Vec<Env>>> = Arc::default();
    let registry = HarnessRegistry::new();
    registry.register(Arc::new(EnvHarness(envs.clone())));
    let core = EngineCore::assemble(dir.path(), Arc::new(registry), HarnessId::Mock, None).unwrap();
    core.workspace
        .create_space("space", "dev-a", "/tmp", None, false)
        .unwrap();
    core.workspace
        .create_chat(LEAD, Some("space"), None, None, None)
        .unwrap();

    let port = {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        listener.local_addr().unwrap().port()
    };
    let server = zeron_engine::serve_ipc(port, dir.path(), core.rpc_service())
        .await
        .unwrap();
    core.sessions.set_tool_server_port(port);
    core.sessions
        .set_ipc_token_file(zeron_rpc::ipc_auth::agent_token_path(dir.path()));

    core.doc_host
        .open(LEAD)
        .unwrap()
        .doc()
        .queue_command(&SessionCommandEntry {
            id: "c1".into(),
            payload: run_payload(),
            issued_by: "viewer".into(),
            issued_at: chrono::Utc::now().timestamp_millis(),
            based_on: None,
            expires_at: None,
            status: SessionCommandStatus::Pending,
            resolution: None,
            computer_generation: None,
        })
        .unwrap();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    while envs.lock().unwrap().is_empty() {
        assert!(tokio::time::Instant::now() < deadline, "no run started");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let env = envs.lock().unwrap()[0].clone();

    let token_file = value_of(&env, zeron_rpc::ipc_auth::TOKEN_FILE_ENV);
    assert!(token_file.ends_with(zeron_rpc::ipc_auth::AGENT_TOKEN_FILE_NAME), "{token_file}");
    let token = std::fs::read_to_string(token_file).expect("the token file the run names");
    let secret = value_of(&env, zeron_rpc::COMPUTER_RUN_SECRET_ENV).to_owned();
    let client = zeron_rpc::connect_ws(&format!("ws://127.0.0.1:{port}"), Some(token.trim()))
        .await
        .unwrap();
    client
        .call(
            methods::BIND_RUN,
            json!({ "chatId": LEAD, "runSecret": secret }),
        )
        .await
        .expect("the tool server speaks for its chat");
    let tools = zeron_mcp::ThreadTools::new(client, LEAD)
        .with_run_secret(Some(secret))
        .with_ipc_port(port);

    let created = tools
        .call("tasks", &json!({ "action": "create", "title": "Ship it" }))
        .await
        .expect("tasks create");
    let task_id = created["taskId"].as_str().expect("task id");
    assert_eq!(core.workspace.task(task_id).unwrap().title, "Ship it");
    tools
        .call(
            "tasks",
            &json!({ "action": "update", "taskId": task_id, "notes": "today" }),
        )
        .await
        .expect("tasks update");
    assert_eq!(core.workspace.task(task_id).unwrap().notes, "today");

    let thread = tools
        .call("create_thread", &json!({ "prompt": "look at this" }))
        .await
        .expect("create_thread");
    let peer = core
        .workspace
        .chat(thread["threadId"].as_str().expect("thread id"))
        .unwrap()
        .expect("the peer's row");
    assert_eq!(peer.created_by_chat_id.as_deref(), Some(LEAD));

    let opened = tools
        .call("terminal", &json!({ "action": "open", "waitMs": 0 }))
        .await
        .expect("terminal open");
    assert!(opened["terminal"]["id"].is_string(), "{opened}");

    let delegated = tools
        .call("delegate_task", &json!({ "task": "write the parser" }))
        .await
        .expect("delegate_task");
    let child = core
        .workspace
        .chat(delegated["taskId"].as_str().expect("task id"))
        .unwrap()
        .expect("the delegate's row");
    assert!(child.is_child_of(LEAD));

    let agent = zeron_rpc::connect_ws(&format!("ws://127.0.0.1:{port}"), Some(token.trim()))
        .await
        .unwrap();
    for method in [
        methods::DECIDE_PLUGIN,
        methods::REVIEW_PLUGIN,
        methods::SIGN_PLUGIN,
        methods::ACCESS_TOKEN,
        methods::SET_TOOL_SETTINGS,
    ] {
        let refused = agent.call(method, json!({})).await.unwrap_err().to_string();
        assert!(refused.contains("isn't open to agents"), "{method}: {refused}");
    }
    // A connection that hasn't proved a chat can't reach the terminal or speak for one.
    assert!(agent.call(methods::OPEN_TERMINAL, json!({})).await.is_err());
    assert!(
        agent
            .call(
                methods::SEND_THREAD_MESSAGE,
                json!({ "chatId": LEAD, "fromChatId": LEAD, "text": "forged" })
            )
            .await
            .unwrap_err()
            .to_string()
            .contains("BindRun")
    );

    let plugins = tools
        .call("list_plugins", &json!({}))
        .await
        .expect("list_plugins");
    assert!(plugins["plugins"].is_array(), "{plugins}");
    let dark = json!({
        "background": "#14181f", "raised": "#232a35", "text": "#d8dee9", "muted": "#a3acb9",
        "faint": "#7c8594", "accent": "#7aa2f7", "danger": "#f7768e", "warning": "#e0af68",
        "success": "#9ece6a",
        "syntax": [
            "#5c6773", "#c792ea", "#c3e88d", "#f78c6c", "#ffcb6b", "#82aaff",
            "#d8dee9", "#d8dee9", "#a3acb9", "#f7768e", "#ffcb6b", "#f7768e"
        ],
    });
    let made = tools
        .call("create_theme", &json!({ "name": "Ink", "dark": dark }))
        .await
        .expect("create_theme");
    let theme_id = made["id"].as_str().expect("theme id");
    let listed_theme_ids = async || {
        let listed = tools
            .call("list_themes", &json!({}))
            .await
            .expect("list_themes");
        listed["themes"]
            .as_array()
            .expect("themes")
            .iter()
            .map(|theme| theme["id"].as_str().unwrap().to_owned())
            .collect::<Vec<_>>()
    };
    assert!(listed_theme_ids().await.iter().any(|id| id == theme_id));
    tools
        .call("delete_theme", &json!({ "id": theme_id }))
        .await
        .expect("delete_theme");
    assert!(!listed_theme_ids().await.iter().any(|id| id == theme_id));

    server.abort();
    core.shutdown().await;
}
