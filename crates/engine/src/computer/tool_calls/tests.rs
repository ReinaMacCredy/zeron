use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use futures::FutureExt;
use futures::StreamExt;
use futures::stream::BoxStream;
use serde_json::{Value, json};
use zeron_doc::{SessionCommandEntry, SessionCommandPayload, SessionCommandStatus};
use zeron_harness::{Harness, HarnessError, RunControls};
use zeron_proto::computer::{ComputerSettings, McpToolResult};
use zeron_proto::{
    AgentEvent, DoneStatus, HarnessId, Model, PermissionMode, ReasoningLevel, RunRequest,
    SandboxLevel, SteeringMode,
};
use zeron_rpc::{RpcClient, methods};

use super::*;
use crate::computer::backend::ComputerBackend;
use crate::computer::manager::tests::{Scripted, deferred};
use crate::{EngineCore, HarnessRegistry};

const CHAT: &str = "chat-tools";

struct Probe {
    secrets: Mutex<Vec<String>>,
    finishes: Mutex<Vec<Arc<tokio::sync::Notify>>>,
}

struct ProbeHarness(Arc<Probe>);

#[async_trait]
impl Harness for ProbeHarness {
    fn id(&self) -> HarnessId {
        HarnessId::Mock
    }
    fn display_name(&self) -> &str {
        "Computer tools probe"
    }
    fn supports_steering(&self) -> bool {
        false
    }
    fn steering_mode(&self) -> SteeringMode {
        SteeringMode::StepBoundary
    }
    fn resume_supported(&self) -> Option<bool> {
        Some(true)
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
        let secret = controls
            .tool_server
            .as_ref()
            .and_then(|server| {
                server
                    .env
                    .iter()
                    .find(|(name, _)| name == zeron_rpc::COMPUTER_RUN_SECRET_ENV)
                    .map(|(_, value)| value.clone())
            })
            .unwrap_or_default();
        self.0.secrets.lock().unwrap().push(secret);
        let finish = Arc::new(tokio::sync::Notify::new());
        self.0.finishes.lock().unwrap().push(finish.clone());
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        tokio::spawn(async move {
            let _ = tx.send(Ok(AgentEvent::SessionStarted {
                harness: HarnessId::Mock,
                model: "probe".into(),
                tools: vec![],
                cwd: "/tmp".into(),
                session_id: "hs-1".into(),
                assistant_message_id: "a-1".into(),
            }));
            let done = |status| AgentEvent::Done {
                status,
                result: None,
                error: None,
                session_id: Some("hs-1".into()),
            };
            tokio::select! {
                _ = controls.interrupt.cancelled() => {
                    let _ = tx.send(Ok(done(DoneStatus::Interrupted)));
                }
                _ = finish.notified() => {
                    let _ = tx.send(Ok(done(DoneStatus::Completed)));
                }
            }
        });
        Ok(
            futures::stream::unfold(rx, |mut rx| async move { rx.recv().await.map(|e| (e, rx)) })
                .boxed(),
        )
    }
}

struct Rig {
    core: EngineCore,
    probe: Arc<Probe>,
    client: RpcClient,
    _dir: tempfile::TempDir,
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

fn request(prompt: &str) -> RunRequest {
    RunRequest {
        prompt: prompt.into(),
        harness: None,
        model: None,
        reasoning: None,
        model_options: Default::default(),
        cwd: "/tmp".into(),
        sandbox: SandboxLevel::WorkspaceWrite,
        permission_mode: PermissionMode::Full,
        auto_approve: false,
        attachments: Vec::new(),
        worktree: None,
        branch: None,
        resume: None,
        forked_at: None,
        computer_control: false,
    }
}

impl Rig {
    async fn new(backend: Option<Arc<dyn ComputerBackend>>) -> Self {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("device-id"), "dev-a").unwrap();
        let probe = Arc::new(Probe {
            secrets: Mutex::default(),
            finishes: Mutex::default(),
        });
        let registry = HarnessRegistry::new();
        registry.register(Arc::new(ProbeHarness(probe.clone())));
        let core = EngineCore::assemble(dir.path(), Arc::new(registry), HarnessId::Mock, None)
            .expect("engine core assembles");
        core.workspace
            .create_space("space-tools", "dev-a", "/tmp", None, false)
            .expect("space");
        core.workspace
            .create_chat(CHAT, Some("space-tools"), None, None, None)
            .expect("chat");
        core.sessions.set_tool_server_port(1);
        let computer = core.computer.clone();
        wait_for(|| computer.service().is_some(), "the computer service").await;
        if let Some(backend) = backend {
            computer.install_backend(backend).await;
        }
        let client = zeron_rpc::memory_client(core.rpc_service());
        Rig {
            core,
            probe,
            client,
            _dir: dir,
        }
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

    fn command(&self, id: &str, payload: SessionCommandPayload) {
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
                computer_generation: None,
            })
            .expect("queue command");
    }

    async fn run(&self, id: &str, prompt: &str) -> String {
        let before = self.probe.secrets.lock().unwrap().len();
        self.command(
            id,
            SessionCommandPayload::Run {
                request: request(prompt),
                message_id: format!("m-{id}"),
            },
        );
        wait_for(
            || self.probe.secrets.lock().unwrap().len() > before,
            "the harness to start",
        )
        .await;
        self.probe.secrets.lock().unwrap().last().unwrap().clone()
    }

    async fn stop(&self, secret: &str) {
        self.command("stop", SessionCommandPayload::Interrupt {});
        wait_for(
            || self.core.sessions.computer_run(CHAT, secret).is_none(),
            "the secret to be revoked",
        )
        .await;
    }

    async fn catalog(&self, secret: &str) -> Result<Vec<String>, zeron_rpc::RpcError> {
        let reply = self
            .client
            .call(
                methods::GET_COMPUTER_CATALOG,
                json!({ "chatId": CHAT, "runSecret": secret }),
            )
            .await?;
        Ok(reply["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|tool| tool["name"].as_str().unwrap().to_owned())
            .collect())
    }

    async fn action(
        &self,
        secret: &str,
        tool: &str,
        arguments: Value,
        request_id: i64,
    ) -> Result<McpToolResult, zeron_rpc::RpcError> {
        let reply = self
            .client
            .call(
                methods::COMPUTER_ACTION,
                json!({
                    "chatId": CHAT,
                    "runSecret": secret,
                    "tool": tool,
                    "arguments": arguments,
                    "requestId": request_id,
                }),
            )
            .await?;
        Ok(serde_json::from_value(reply).expect("an MCP tool result"))
    }

    async fn cancel(&self, secret: &str, request_id: i64) -> Result<Value, zeron_rpc::RpcError> {
        self.client
            .call(
                methods::CANCEL_COMPUTER_ACTION,
                json!({ "chatId": CHAT, "runSecret": secret, "requestId": request_id }),
            )
            .await
    }
}

fn fake() -> Arc<dyn ComputerBackend> {
    Scripted::default_fake()
}

fn payload(result: &McpToolResult) -> Value {
    tool_result_payload(result).map_or(Value::Null, Value::Object)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn every_computer_rpc_refuses_a_missing_wrong_or_revoked_secret() {
    let rig = Rig::new(Some(fake())).await;
    rig.settings(true);
    let secret = rig.run("c1", "hello").await;
    assert!(!rig.catalog(&secret).await.unwrap().is_empty());
    for (method, params) in [
        (methods::GET_COMPUTER_CATALOG, json!({ "chatId": CHAT })),
        (
            methods::COMPUTER_ACTION,
            json!({ "chatId": CHAT, "tool": "computer_list_windows", "requestId": 1 }),
        ),
        (
            methods::CANCEL_COMPUTER_ACTION,
            json!({ "chatId": CHAT, "requestId": 1 }),
        ),
    ] {
        assert!(
            rig.client.call(method, params).await.is_err(),
            "{method} without a secret"
        );
    }
    let wrong = format!("{secret}x");
    assert!(rig.catalog(&wrong).await.is_err());
    let refused = rig
        .action(&wrong, "computer_list_windows", json!({}), 1)
        .await
        .unwrap_err();
    assert!(refused.to_string().contains("refused"), "{refused}");
    assert!(rig.cancel(&wrong, 1).await.is_err());
    rig.stop(&secret).await;
    assert!(rig.catalog(&secret).await.is_err());
    assert!(
        rig.action(&secret, "computer_list_windows", json!({}), 2)
            .await
            .is_err()
    );
    assert!(rig.cancel(&secret, 2).await.is_err());
    rig.core.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_catalog_follows_the_turn_flag_and_lists_the_advertised_desktop_tools() {
    let rig = Rig::new(Some(fake())).await;
    let plain = rig.run("c1", "plain").await;
    assert!(rig.catalog(&plain).await.unwrap().is_empty());
    let denied = rig
        .action(&plain, "computer_list_windows", json!({}), 1)
        .await
        .unwrap();
    assert!(denied.is_error);
    assert_eq!(
        payload(&denied)["error"]["code"],
        json!("capability_denied")
    );
    rig.probe
        .finishes
        .lock()
        .unwrap()
        .last()
        .unwrap()
        .notify_one();
    wait_for(
        || !rig.core.sessions.turn_in_flight(CHAT),
        "the plain turn to end",
    )
    .await;

    let slash = rig.run("c2", "/computer-use open Calculator").await;
    let listed = rig.catalog(&slash).await.unwrap();
    assert_eq!(listed.len(), 18, "{listed:?}");
    assert!(listed.contains(&"computer_run".to_owned()));
    assert!(!listed.contains(&"computer_drag".to_owned()));
    rig.probe
        .finishes
        .lock()
        .unwrap()
        .last()
        .unwrap()
        .notify_one();
    wait_for(
        || !rig.core.sessions.turn_in_flight(CHAT),
        "the slash turn to end",
    )
    .await;

    rig.settings(true);
    let chat = rig.run("c3", "plain again").await;
    assert_eq!(rig.catalog(&chat).await.unwrap(), listed);
    rig.core.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_revoked_turn_is_refused_with_user_revoked_not_capability_denied() {
    let backend = Scripted::default_fake();
    let rig = Rig::new(Some(backend.clone())).await;
    rig.settings(true);
    let secret = rig.run("c1", "type").await;
    let typed = rig
        .action(
            &secret,
            "computer_type_text",
            json!({ "text": "a", "window_id": "fake-terminal", "include_screenshot": false }),
            1,
        )
        .await
        .unwrap();
    assert!(!typed.is_error, "{typed:?}");
    let reply = rig
        .client
        .call(methods::COMPUTER_REVOKE_DESKTOP, json!({ "chatId": CHAT }))
        .await
        .unwrap();
    assert_eq!(reply, json!({ "revoked": true }));
    for (id, tool) in [(2, "computer_type_text"), (3, "computer_list_windows")] {
        let refused = rig
            .action(
                &secret,
                tool,
                json!({ "text": "b", "window_id": "fake-terminal", "include_screenshot": false }),
                id,
            )
            .await
            .unwrap();
        assert!(refused.is_error, "{tool}");
        assert_eq!(payload(&refused)["error"]["code"], json!("user_revoked"));
        assert_eq!(
            payload(&refused)["error"]["message"],
            json!(crate::computer::manager::COMPUTER_USER_REVOKED)
        );
    }
    assert_eq!(backend.calls_for("typeText").len(), 1);
    assert!(rig.core.sessions.turn_in_flight(CHAT));
    rig.core.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_unsupported_service_lists_nothing_and_refuses_every_call() {
    let rig = Rig::new(None).await;
    rig.settings(true);
    let secret = rig.run("c1", "hello").await;
    assert!(rig.catalog(&secret).await.unwrap().is_empty());
    let refused = rig
        .action(&secret, "computer_list_windows", json!({}), 1)
        .await
        .unwrap();
    assert!(refused.is_error);
    assert_eq!(
        payload(&refused)["error"]["code"],
        json!("computer_unavailable")
    );
    rig.core.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn hidden_tools_dispatch_by_exact_name_and_results_carry_images_and_structure() {
    let backend = Scripted::default_fake();
    let rig = Rig::new(Some(backend.clone())).await;
    rig.settings(true);
    let secret = rig.run("c1", "drag it").await;
    let dragged = rig
        .action(
            &secret,
            "computer_drag",
            json!({
                "from": { "label": "Calculate", "role": "button" },
                "to": { "label": "Display", "role": "text-field" },
                "include_screenshot": false,
            }),
            1,
        )
        .await
        .unwrap();
    assert!(!dragged.is_error, "{dragged:?}");
    assert_eq!(backend.calls_for("drag").len(), 1);
    assert_eq!(
        dragged.structured_content.as_ref(),
        tool_result_payload(&dragged).map(Value::Object).as_ref()
    );
    let help = rig
        .action(
            &secret,
            "computer_help",
            json!({ "tool": "computer_drag" }),
            2,
        )
        .await
        .unwrap();
    assert_eq!(payload(&help)["definition"]["name"], json!("computer_drag"));
    assert_eq!(payload(&help)["batchStep"]["type"], json!("drag"));
    let shot = rig
        .action(
            &secret,
            "computer_get_state",
            json!({ "include_screenshot": true }),
            3,
        )
        .await
        .unwrap();
    assert!(shot.has_image());
    assert!(shot.structured_content.is_none());
    let unknown = rig
        .action(&secret, "computer_future_tool", json!({}), 4)
        .await
        .unwrap_err();
    assert!(unknown.to_string().contains("Unknown computer tool"));
    rig.core.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_cancel_reaches_its_call_while_another_call_is_outstanding() {
    let rig = Arc::new(Rig::new(Some(fake())).await);
    rig.settings(true);
    let secret = rig.run("c1", "wait").await;
    let long = {
        let (rig, secret) = (rig.clone(), secret.clone());
        tokio::spawn(async move {
            rig.action(
                &secret,
                "computer_wait",
                json!({ "duration_ms": 10_000 }),
                7,
            )
            .await
        })
    };
    let short = {
        let (rig, secret) = (rig.clone(), secret.clone());
        tokio::spawn(async move {
            rig.action(&secret, "computer_wait", json!({ "duration_ms": 400 }), 8)
                .await
        })
    };
    let calls = rig.core.computer.calls().clone();
    wait_for(|| calls.in_flight() == 2, "both calls to be in flight").await;
    let started = tokio::time::Instant::now();
    assert_eq!(
        rig.cancel(&secret, 7).await.unwrap(),
        json!({ "cancelled": true })
    );
    let cancelled = long.await.unwrap().unwrap();
    assert!(cancelled.is_error, "{cancelled:?}");
    assert!(started.elapsed() < Duration::from_secs(5));
    assert!(!short.is_finished() || calls.in_flight() == 0);
    let finished = short.await.unwrap().unwrap();
    assert!(!finished.is_error, "{finished:?}");
    assert_eq!(payload(&finished)["waitedMs"], json!(400));
    assert_eq!(
        rig.cancel(&secret, 7).await.unwrap(),
        json!({ "cancelled": false })
    );
    rig.core.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_dropped_caller_mid_mutation_interrupts_input_and_the_action_drains() {
    let backend = Scripted::default_fake();
    let entered = deferred();
    let release = deferred();
    {
        let (entered, release) = (entered.clone(), release.clone());
        backend.hooks().type_text = Some(Arc::new(move |fake, (text, window, target)| {
            let (entered, release) = (entered.clone(), release.clone());
            async move {
                entered.resolve();
                release.wait().await;
                fake.type_text(&text, window.as_deref(), target.as_ref())
                    .await
            }
            .boxed()
        }));
    }
    let rig = Rig::new(Some(backend.clone())).await;
    rig.settings(true);
    let secret = rig.run("c1", "type").await;
    let call = computer_action(
        &rig.core.computer,
        &rig.core.sessions,
        serde_json::from_value(json!({
            "chatId": CHAT,
            "runSecret": secret,
            "tool": "computer_type_text",
            "arguments": { "text": "abc", "window_id": "fake-terminal", "include_screenshot": false },
            "requestId": "req-drop",
        }))
        .unwrap(),
    );
    tokio::select! {
        _ = call => panic!("the call finished before the backend was entered"),
        () = entered.wait() => {}
    }
    wait_for(
        || !backend.logged("stopInput").is_empty(),
        "the caller-gone guard to interrupt input",
    )
    .await;
    assert_eq!(backend.logged("stopInput")[0]["threadId"], json!(CHAT));
    let calls = rig.core.computer.calls().clone();
    assert_eq!(calls.in_flight(), 1);
    release.resolve();
    wait_for(|| calls.in_flight() == 0, "the detached action to drain").await;
    rig.core.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_dropped_caller_before_any_mutation_interrupts_nothing() {
    let backend = Scripted::default_fake();
    let rig = Rig::new(Some(backend.clone())).await;
    rig.settings(true);
    let secret = rig.run("c1", "wait").await;
    let call = computer_action(
        &rig.core.computer,
        &rig.core.sessions,
        serde_json::from_value(json!({
            "chatId": CHAT,
            "runSecret": secret,
            "tool": "computer_wait",
            "arguments": { "duration_ms": 300 },
            "requestId": 3,
        }))
        .unwrap(),
    );
    let calls = rig.core.computer.calls().clone();
    tokio::select! {
        _ = call => panic!("the wait finished before the caller dropped"),
        () = wait_for(|| calls.in_flight() == 1, "the wait to start") => {}
    }
    wait_for(|| calls.in_flight() == 0, "the detached wait to finish").await;
    assert!(backend.logged("stopInput").is_empty());
    rig.core.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn turn_end_and_stop_cancel_the_chats_in_flight_calls() {
    let calls = ComputerCalls::default();
    let tokens: Vec<CancellationToken> = (0..3).map(|_| CancellationToken::new()).collect();
    assert!(calls.register(
        (CHAT.into(), McpRequestId::Number(1)),
        Some("t1".into()),
        tokens[0].clone()
    ));
    assert!(!calls.register(
        (CHAT.into(), McpRequestId::Number(1)),
        Some("t1".into()),
        tokens[0].clone()
    ));
    assert!(calls.register(
        (CHAT.into(), McpRequestId::Number(2)),
        Some("t2".into()),
        tokens[1].clone()
    ));
    assert!(calls.register(
        ("other".into(), McpRequestId::Number(1)),
        Some("t1".into()),
        tokens[2].clone()
    ));
    assert_eq!(calls.cancel_chat(CHAT, Some("t1")), 1);
    assert!(tokens[0].is_cancelled() && !tokens[1].is_cancelled());
    assert_eq!(calls.cancel_chat(CHAT, None), 1);
    assert!(tokens[1].is_cancelled() && !tokens[2].is_cancelled());
    assert!(!calls.cancel(CHAT, &McpRequestId::Number(2)));
    assert!(calls.cancel("other", &McpRequestId::Number(1)));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn run_seams_bound_browser_files_to_the_run_cwd_and_read_space_designation_from_the_chat() {
    let backend = Scripted::new(crate::computer::fake_backend::FakeComputerBackendOptions {
        browser: crate::computer::fake_backend::FakeComputerOptIn::BuiltIn,
        ..Default::default()
    });
    backend.hooks().list_spaces = Some(Arc::new(|_, ()| {
        async {
            Ok(zeron_proto::computer::ComputerSpaceInventory {
                source: zeron_proto::computer::ComputerSpaceSource::MacosManagedSpaces,
                complete: true,
                spaces: vec![zeron_proto::computer::ComputerSpace {
                    id: 2,
                    uuid: Some("uuid-2".into()),
                    display_id: "display-a".into(),
                    kind: zeron_proto::computer::ComputerSpaceKind::Desktop,
                    current: Some(false),
                }],
            })
        }
        .boxed()
    }));
    let rig = Rig::new(Some(backend.clone())).await;
    rig.settings(true);
    let secret = rig.run("c1", "Use Space ID 2 for this task.").await;
    assert_eq!(
        rig.catalog(&secret).await.unwrap().len(),
        if cfg!(target_os = "macos") { 29 } else { 28 }
    );
    let outside = tempfile::tempdir().unwrap();
    let stray = outside.path().join("stray.txt");
    std::fs::write(&stray, "x").unwrap();
    let inside = tempfile::tempdir_in("/tmp").unwrap();
    let attach = inside.path().join("attach.txt");
    std::fs::write(&attach, "x").unwrap();
    let upload = |file: &std::path::Path| json!({ "target_id": "t", "tab_id": "tab", "ref": "p1:0", "files": [file] });
    let refused = rig
        .action(&secret, "computer_browser_upload", upload(&stray), 1)
        .await
        .unwrap();
    assert!(refused.is_error);
    assert!(
        refused
            .first_text()
            .unwrap_or_default()
            .contains("outside the active workspace"),
        "{refused:?}"
    );
    let uploaded = rig
        .action(&secret, "computer_browser_upload", upload(&attach), 2)
        .await
        .unwrap();
    assert!(!uploaded.is_error, "{uploaded:?}");
    assert_eq!(
        backend.calls_for("browser.browser_set_input_files").len(),
        1
    );
    let reserved = rig
        .action(
            &secret,
            "computer_inspect",
            json!({ "tool": "computer_spaces", "arguments": { "operation": "reserve", "space_id": 2 } }),
            3,
        )
        .await
        .unwrap();
    assert!(!reserved.is_error, "{reserved:?}");
    assert_eq!(payload(&reserved)["reservation"]["spaceId"], json!(2));
    rig.core.shutdown().await;
}

impl Rig {
    fn manager(&self) -> Arc<ComputerManager> {
        self.core.computer.service().unwrap().manager.clone()
    }

    fn foreground_inputs(&self) -> Vec<(String, bool)> {
        self.core
            .doc_host
            .open(CHAT)
            .unwrap()
            .doc()
            .read_entries()
            .unwrap()
            .iter()
            .flat_map(|entry| entry.parts.iter())
            .filter_map(|part| match part {
                zeron_doc::MessagePart::Input {
                    request_id,
                    questions,
                    resolved,
                    ..
                } if questions.iter().any(|question| {
                    matches!(
                        question.kind,
                        Some(zeron_proto::UserInputKind::Foreground { .. })
                    )
                }) =>
                {
                    Some((request_id.clone(), *resolved))
                }
                _ => None,
            })
            .collect()
    }

    async fn pending_foreground(&self) -> zeron_proto::computer::ComputerPendingRequest {
        let manager = self.manager();
        wait_for(
            || manager.pending_request(CHAT).is_some(),
            "the foreground question",
        )
        .await;
        let pending = manager.pending_request(CHAT).unwrap();
        wait_for(
            || {
                self.foreground_inputs()
                    .iter()
                    .any(|(id, resolved)| *id == pending.input_id && !resolved)
            },
            "the question in the transcript",
        )
        .await;
        pending
    }

    fn answer_foreground(&self, id: &str, input_id: &str, label: &str) {
        self.command(
            id,
            SessionCommandPayload::RespondInput {
                request_id: input_id.into(),
                answers: vec![zeron_proto::UserInputAnswer {
                    question_id: "foreground".into(),
                    labels: vec![label.into()],
                }],
            },
        );
    }

    async fn finish_turn(&self) {
        self.probe
            .finishes
            .lock()
            .unwrap()
            .last()
            .unwrap()
            .notify_one();
        wait_for(
            || !self.core.sessions.turn_in_flight(CHAT),
            "the turn to end",
        )
        .await;
    }
}

fn foreground_key() -> Value {
    json!({ "key": "enter", "window_id": "fake-terminal", "delivery_mode": "foreground", "include_screenshot": false })
}

fn spawn_action(
    rig: &Arc<Rig>,
    secret: &str,
    arguments: Value,
    request_id: i64,
) -> tokio::task::JoinHandle<McpToolResult> {
    let (rig, secret) = (rig.clone(), secret.to_owned());
    tokio::spawn(async move {
        rig.action(&secret, "computer_press_key", arguments, request_id)
            .await
            .unwrap()
    })
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_foreground_refusal_asks_once_and_allow_authorizes_the_rest_of_the_turn() {
    let backend = Scripted::default_fake();
    let rig = Arc::new(Rig::new(Some(backend.clone())).await);
    rig.settings(true);
    let secret = rig.run("c1", "press enter in the terminal").await;
    let call = spawn_action(&rig, &secret, foreground_key(), 1);
    let pending = rig.pending_foreground().await;
    assert_eq!(pending.question_id, "foreground");
    assert!(!call.is_finished(), "the call waits for the user");
    assert!(backend.calls_for("pressKey").is_empty());
    assert_eq!(
        rig.core.sessions.computer_turn_foreground(CHAT, None),
        None,
        "Full access does not grant foreground by itself"
    );

    rig.answer_foreground("allow", &pending.input_id, "Allow for this turn");
    let allowed = call.await.unwrap();
    assert!(!allowed.is_error, "{allowed:?}");
    assert_eq!(backend.calls_for("pressKey").len(), 1);
    assert_eq!(rig.manager().pending_request(CHAT), None);

    let again = rig
        .action(&secret, "computer_press_key", foreground_key(), 2)
        .await
        .unwrap();
    assert!(!again.is_error, "{again:?}");
    assert_eq!(backend.calls_for("pressKey").len(), 2);
    wait_for(
        || rig.foreground_inputs() == vec![(pending.input_id.clone(), true)],
        "one resolved question for the turn",
    )
    .await;
    rig.core.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_foreground_no_returns_foreground_declined_and_never_asks_again_this_turn() {
    let backend = Scripted::default_fake();
    let rig = Arc::new(Rig::new(Some(backend.clone())).await);
    rig.settings(true);
    let secret = rig.run("c1", "press enter in the terminal").await;
    let call = spawn_action(&rig, &secret, foreground_key(), 1);
    let pending = rig.pending_foreground().await;
    rig.answer_foreground("no", &pending.input_id, "No");
    let declined = call.await.unwrap();
    assert!(declined.is_error);
    assert_eq!(
        payload(&declined)["error"]["code"],
        json!(COMPUTER_FOREGROUND_DECLINED_CODE)
    );
    let message = payload(&declined)["error"]["message"]
        .as_str()
        .unwrap()
        .to_owned();
    assert!(message.contains("tell the user what is left"), "{message}");
    let again = rig
        .action(&secret, "computer_press_key", foreground_key(), 2)
        .await
        .unwrap();
    assert_eq!(
        payload(&again)["error"]["code"],
        json!(COMPUTER_FOREGROUND_DECLINED_CODE)
    );
    assert_eq!(rig.foreground_inputs().len(), 1, "never asked again");
    assert!(backend.calls_for("pressKey").is_empty());
    let background = rig
        .action(
            &secret,
            "computer_type_text",
            json!({ "text": "a", "window_id": "fake-terminal", "include_screenshot": false }),
            3,
        )
        .await
        .unwrap();
    assert!(
        !background.is_error,
        "background input still works: {background:?}"
    );
    rig.core.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_delegated_thread_is_authorized_by_the_click() {
    let backend = Scripted::default_fake();
    let rig = Arc::new(Rig::new(Some(backend.clone())).await);
    rig.settings(true);
    let secret = rig.run("c1", "wait for the lead").await;
    rig.core
        .doc_host
        .open(CHAT)
        .unwrap()
        .write_user_message_from(
            "m-lead",
            "Show me the terminal window.",
            chrono::Utc::now().timestamp_millis(),
            Some("lead-chat"),
        )
        .unwrap();
    let call = spawn_action(&rig, &secret, foreground_key(), 1);
    let pending = rig.pending_foreground().await;
    assert!(
        backend.calls_for("pressKey").is_empty(),
        "another thread's words never authorize"
    );
    rig.answer_foreground("allow", &pending.input_id, "Allow for this turn");
    let allowed = call.await.unwrap();
    assert!(!allowed.is_error, "{allowed:?}");
    assert_eq!(backend.calls_for("pressKey").len(), 1);
    rig.core.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_foreground_grant_ends_with_the_turn_and_a_turn_end_settles_an_open_question() {
    let backend = Scripted::default_fake();
    let rig = Arc::new(Rig::new(Some(backend.clone())).await);
    rig.settings(true);
    let secret = rig.run("c1", "press enter in the terminal").await;
    let call = spawn_action(&rig, &secret, foreground_key(), 1);
    let pending = rig.pending_foreground().await;
    rig.answer_foreground("allow", &pending.input_id, "Allow for this turn");
    assert!(!call.await.unwrap().is_error);
    rig.finish_turn().await;

    let secret = rig.run("c2", "press enter again").await;
    assert_eq!(rig.core.sessions.computer_turn_foreground(CHAT, None), None);
    let call = spawn_action(&rig, &secret, foreground_key(), 2);
    let next = rig.pending_foreground().await;
    assert_ne!(next.input_id, pending.input_id, "a new turn asks again");
    rig.finish_turn().await;
    let unanswered = call.await.unwrap();
    assert!(unanswered.is_error);
    assert_eq!(rig.manager().pending_request(CHAT), None);
    wait_for(
        || {
            rig.foreground_inputs()
                .iter()
                .any(|(id, resolved)| *id == next.input_id && *resolved)
        },
        "the open question to settle with the turn",
    )
    .await;
    assert_eq!(backend.calls_for("pressKey").len(), 1);
    rig.core.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_revoke_while_foreground_is_asked_settles_the_question_and_returns_user_revoked() {
    let backend = Scripted::default_fake();
    let rig = Arc::new(Rig::new(Some(backend.clone())).await);
    rig.settings(true);
    let secret = rig.run("c1", "press enter in the terminal").await;
    let call = spawn_action(&rig, &secret, foreground_key(), 1);
    let pending = rig.pending_foreground().await;
    rig.client
        .call(methods::COMPUTER_REVOKE_DESKTOP, json!({ "chatId": CHAT }))
        .await
        .unwrap();
    let revoked = call.await.unwrap();
    assert_eq!(payload(&revoked)["error"]["code"], json!("user_revoked"));
    assert_eq!(rig.manager().pending_request(CHAT), None);
    wait_for(
        || {
            rig.foreground_inputs()
                .iter()
                .any(|(id, resolved)| *id == pending.input_id && *resolved)
        },
        "the question to settle",
    )
    .await;
    assert!(backend.calls_for("pressKey").is_empty());
    rig.core.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_granted_turn_authorizes_before_the_message_walk() {
    let rig = Rig::new(Some(fake())).await;
    rig.settings(true);
    let secret = rig.run("c1", "keep the terminal in the background").await;
    let run = rig.core.sessions.computer_run(CHAT, &secret).unwrap();
    let seams = RunToolSeams {
        sessions: rig.core.sessions.clone(),
        calls: rig.core.computer.calls().clone(),
        manager: Arc::downgrade(&rig.manager()),
        chat_id: CHAT.into(),
        turn_id: run.turn_id.clone(),
    };
    assert!(
        !seams
            .foreground_authorization()
            .await
            .user_requested_visible_use
    );
    assert!(
        !rig.core
            .sessions
            .answer_computer_foreground(CHAT, Some("another-turn"), true)
    );
    assert!(
        !seams
            .foreground_authorization()
            .await
            .user_requested_visible_use
    );
    assert!(
        rig.core
            .sessions
            .answer_computer_foreground(CHAT, run.turn_id.as_deref(), true)
    );
    assert!(
        seams
            .foreground_authorization()
            .await
            .user_requested_visible_use
    );
    rig.core.shutdown().await;
}

#[test]
fn only_a_refused_foreground_raise_with_nothing_dispatched_asks() {
    let refused = |error: &str| {
        crate::computer::tools::results::mcp_json_error(
            &json!({ "error": error, "effect": "not-dispatched" }),
        )
    };
    let args = |value: Value| value.as_object().unwrap().clone();
    let key = args(foreground_key());
    assert_eq!(
        foreground_refusal(
            "computer_press_key",
            &key,
            &refused("foreground_not_requested")
        ),
        Some(ComputerConsentTarget::Window("fake-terminal".into()))
    );
    assert_eq!(
        foreground_refusal(
            "computer_press_key",
            &key,
            &refused("same_pid_keyboard_ambiguity")
        ),
        None
    );
    let background = args(json!({ "key": "enter", "window_id": "w" }));
    assert_eq!(
        foreground_refusal(
            "computer_launch_app",
            &args(json!({ "app": "Ghostty", "hidden": false })),
            &refused("foreground_not_requested")
        ),
        None,
        "a visible launch is not a keyboard raise"
    );
    assert_eq!(
        foreground_refusal(
            "computer_press_key",
            &background,
            &refused("foreground_not_requested")
        ),
        None
    );
    assert_eq!(
        foreground_refusal(
            "computer_invoke_menu",
            &args(json!({ "pid": 42, "path": ["File"] })),
            &refused("foreground_not_requested")
        ),
        Some(ComputerConsentTarget::Pid(42))
    );
    let run = |completed: u64| {
        crate::computer::tools::results::mcp_json(&json!({
            "completed": completed,
            "stopped": true,
            "steps": [{ "ok": false, "error": { "code": "foreground_not_requested", "effect": "not-dispatched" } }],
        }))
    };
    let steps = args(json!({ "steps": [{ "type": "activate_window", "window_id": "w9" }] }));
    assert_eq!(
        foreground_refusal("computer_run", &steps, &run(0)),
        Some(ComputerConsentTarget::Window("w9".into()))
    );
    assert_eq!(
        foreground_refusal("computer_run", &steps, &run(1)),
        None,
        "a run that already dispatched a step is never replayed"
    );
}
