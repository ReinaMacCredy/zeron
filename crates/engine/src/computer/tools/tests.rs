use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;
use zeron_proto::PermissionMode;
use zeron_proto::computer::McpToolResult;

use super::results::tool_result_payload;
use super::{
    ComputerToolContext, ComputerToolSeams, ComputerTools, ToolArgs, computer_tool_families,
};
use crate::computer::approval_gate::{ApprovalDecision, ComputerApprovalGate};
use crate::computer::backend::{ComputerAgentDialect, ComputerError};
use crate::computer::fake_backend::FakeComputerBackendOptions;
use crate::computer::manager::tests::Scripted;
use crate::computer::manager::{
    ComputerConsentCard, ComputerManager, ComputerManagerEnv, ComputerManagerOptions,
    ComputerManagerSeams,
};
use crate::computer::setup_signal::ComputerSetupSignal;
use crate::computer::visible_use::ComputerForegroundAuthorization;

mod actions;
mod app_consent;
mod batch_and_diffs;
mod browser;
mod catalog;
mod hung_host;
mod invariants;
mod menus_frames;
mod perception;
mod refs_and_help;
mod setup_and_gates;
mod spaces;
mod visibility;

pub(crate) const THREAD: &str = "thread-computer";
pub(crate) const TURN: &str = "turn-computer";

pub(crate) type Decide = Arc<dyn Fn(&ComputerConsentCard) -> ApprovalDecision + Send + Sync>;

pub(crate) struct Approvals {
    pub mode: Mutex<Option<PermissionMode>>,
    pub decide: Mutex<Option<Decide>>,
    pub gate: ComputerApprovalGate,
    pub cards: Mutex<Vec<ComputerConsentCard>>,
    pub turn_inactive: AtomicBool,
}

#[async_trait]
impl ComputerManagerSeams for Approvals {
    async fn caller(
        &self,
        _chat_id: &str,
        _turn_id: Option<&str>,
    ) -> Result<Option<PermissionMode>, ComputerError> {
        if self.turn_inactive.load(Ordering::SeqCst) {
            return Err(ComputerError::Other(
                "The turn that made this call has ended.".into(),
            ));
        }
        Ok(*self.mode.lock().unwrap())
    }

    async fn publish_consent(
        &self,
        card: ComputerConsentCard,
        decision: Option<ApprovalDecision>,
    ) -> Result<(), String> {
        if decision.is_some() {
            return Ok(());
        }
        self.cards.lock().unwrap().push(card.clone());
        if let Some(decide) = self.decide.lock().unwrap().clone() {
            let gate = self.gate.clone();
            tokio::spawn(async move {
                tokio::task::yield_now().await;
                gate.respond(&card.chat_id, &card.request_id, decide(&card));
            });
        }
        Ok(())
    }
}

pub(crate) struct ToolSeams {
    pub turn_inactive: AtomicBool,
    pub authorization: Mutex<ComputerForegroundAuthorization>,
    pub authorization_reads: AtomicUsize,
    pub setup: Mutex<Vec<(String, ComputerSetupSignal)>>,
    pub designation: Mutex<Vec<u64>>,
    pub designation_script: Mutex<VecDeque<Vec<u64>>>,
    pub designation_reads: AtomicUsize,
    pub workspace_root: Mutex<Option<PathBuf>>,
}

impl Default for ToolSeams {
    fn default() -> Self {
        Self {
            turn_inactive: AtomicBool::new(false),
            authorization: Mutex::new(ComputerForegroundAuthorization {
                user_requested_visible_use: true,
            }),
            authorization_reads: AtomicUsize::new(0),
            setup: Mutex::default(),
            designation: Mutex::default(),
            designation_script: Mutex::default(),
            designation_reads: AtomicUsize::new(0),
            workspace_root: Mutex::default(),
        }
    }
}

#[async_trait]
impl ComputerToolSeams for ToolSeams {
    fn assert_turn_active(&self) -> Result<(), ComputerError> {
        if self.turn_inactive.load(Ordering::SeqCst) {
            return Err(ComputerError::Other(
                "The turn that made this call has ended.".into(),
            ));
        }
        Ok(())
    }

    async fn foreground_authorization(
        &self,
        _target: Option<&ToolArgs>,
    ) -> ComputerForegroundAuthorization {
        self.authorization_reads.fetch_add(1, Ordering::SeqCst);
        *self.authorization.lock().unwrap()
    }

    async fn setup_required(&self, tool: &str, signal: &ComputerSetupSignal) {
        self.setup
            .lock()
            .unwrap()
            .push((tool.to_owned(), signal.clone()));
    }

    async fn space_designation(&self) -> Vec<u64> {
        self.designation_reads.fetch_add(1, Ordering::SeqCst);
        let scripted = self.designation_script.lock().unwrap().pop_front();
        scripted.unwrap_or_else(|| self.designation.lock().unwrap().clone())
    }

    async fn workspace_root(&self) -> Option<PathBuf> {
        self.workspace_root.lock().unwrap().clone()
    }
}

pub(crate) struct Harness {
    pub tools: Arc<ComputerTools>,
    pub manager: Arc<ComputerManager>,
    pub backend: Arc<Scripted>,
    pub approvals: Arc<Approvals>,
    pub seams: Arc<ToolSeams>,
}

pub(crate) struct HarnessOptions {
    pub backend: Arc<Scripted>,
    pub mode: PermissionMode,
    pub action_settle_ms: u64,
    pub dialect: Option<ComputerAgentDialect>,
}

impl Default for HarnessOptions {
    fn default() -> Self {
        Self {
            backend: Scripted::default_fake(),
            mode: PermissionMode::Full,
            action_settle_ms: 0,
            dialect: None,
        }
    }
}

pub(crate) fn harness_with(options: HarnessOptions) -> Harness {
    if let Some(dialect) = options.dialect {
        options.backend.set_dialect(dialect);
    }
    let gate = ComputerApprovalGate::default();
    let approvals = Arc::new(Approvals {
        mode: Mutex::new(Some(options.mode)),
        decide: Mutex::new(None),
        gate: gate.clone(),
        cards: Mutex::default(),
        turn_inactive: AtomicBool::new(false),
    });
    let mut manager_options = ComputerManagerOptions::new(options.backend.clone());
    manager_options.approval_gate = gate;
    manager_options.seams = approvals.clone();
    manager_options.action_settle_ms = Some(options.action_settle_ms);
    manager_options.env = ComputerManagerEnv::default();
    let manager = ComputerManager::new(manager_options);
    Harness {
        tools: ComputerTools::new(manager.clone(), computer_tool_families(&manager)),
        manager,
        backend: options.backend,
        approvals,
        seams: Arc::new(ToolSeams::default()),
    }
}

pub(crate) fn harness() -> Harness {
    harness_with(HarnessOptions::default())
}

pub(crate) fn fake_with(options: FakeComputerBackendOptions) -> Arc<Scripted> {
    Scripted::new(options)
}

pub(crate) fn args(value: Value) -> ToolArgs {
    match value {
        Value::Object(map) => map,
        _ => ToolArgs::new(),
    }
}

pub(crate) fn result_json(result: &McpToolResult) -> Value {
    tool_result_payload(result).map_or(Value::Null, Value::Object)
}

impl Harness {
    pub fn context_for(&self, thread: &str, label: Option<&str>) -> ComputerToolContext {
        ComputerToolContext {
            chat_id: thread.to_owned(),
            chat_label: label.map(str::to_owned),
            turn_id: Some(TURN.to_owned()),
            session_key: "session-computer".to_owned(),
            request_id: json!(1),
            signal: CancellationToken::new(),
            seams: self.seams.clone(),
            mutation_dispatched: Arc::default(),
        }
    }

    pub fn context(&self) -> ComputerToolContext {
        self.context_for(THREAD, None)
    }

    pub async fn call(&self, name: &str, arguments: Value) -> McpToolResult {
        self.tools.call(name, args(arguments), &self.context()).await
    }

    pub async fn call_in(&self, name: &str, arguments: Value, context: &ComputerToolContext) -> McpToolResult {
        self.tools.call(name, args(arguments), context).await
    }

    pub async fn see(&self) -> Value {
        self.see_in(&self.context()).await
    }

    pub async fn see_in(&self, context: &ComputerToolContext) -> Value {
        let state = self
            .call_in("computer_get_state", json!({ "include_screenshot": true }), context)
            .await;
        assert!(!state.is_error, "{state:?}");
        result_json(&state)["screenshot"].clone()
    }

    pub fn decide(&self, decision: ApprovalDecision) {
        *self.approvals.mode.lock().unwrap() = Some(PermissionMode::Ask);
        *self.approvals.decide.lock().unwrap() = Some(Arc::new(move |_| decision));
    }

    pub fn authorize_visible_use(&self, authorized: bool) {
        *self.seams.authorization.lock().unwrap() = ComputerForegroundAuthorization {
            user_requested_visible_use: authorized,
        };
    }

    pub async fn dispose(&self) {
        self.manager.dispose().await;
    }
}

#[tokio::test]
async fn returns_perception_payloads_and_preserves_screenshot_image_content() {
    let h = harness();
    let state = h.call("computer_get_state", json!({ "include_screenshot": true })).await;
    assert!(!state.is_error);
    assert!(state.has_image());
    let payload = result_json(&state);
    assert!(payload["screenshot"]["screenshotId"].is_string());
    assert!(payload["screenshot"].get("bytesBase64").is_none());
    assert!(payload["elements"].as_array().is_some_and(|items| !items.is_empty()));
    let windows = h.call("computer_list_windows", json!({})).await;
    assert!(!windows.is_error);
    assert_eq!(result_json(&windows)["windows"].as_array().map(Vec::len), Some(2));
    h.dispose().await;
}

#[tokio::test]
async fn a_first_mutation_carries_the_disclosure_once_per_turn() {
    let h = harness();
    let first = h
        .call("computer_set_value", json!({ "label": "Display", "value": "468" }))
        .await;
    assert!(!first.is_error, "{first:?}");
    assert_eq!(
        result_json(&first)["disclosure"],
        json!(crate::computer::manager::COMPUTER_CONTROL_ON_DISCLOSURE)
    );
    let second = h
        .call(
            "computer_set_value",
            json!({ "label": "Display", "value": "469", "include_screenshot": false }),
        )
        .await;
    assert!(result_json(&second).get("disclosure").is_none());
    h.dispose().await;
}

#[tokio::test]
async fn a_denied_approval_dispatches_nothing() {
    let h = harness();
    h.decide(ApprovalDecision::Decline);
    let result = h
        .call(
            "computer_click",
            json!({ "label": "Calculate", "window_id": "fake-calculator" }),
        )
        .await;
    assert!(result.is_error);
    assert_eq!(result_json(&result)["error"]["code"], "app_consent_denied");
    assert!(h.backend.calls_for("click").is_empty());
    assert_eq!(h.approvals.cards.lock().unwrap().len(), 1);
    h.dispose().await;
}

#[tokio::test]
async fn args_that_fail_to_parse_refuse_before_the_backend() {
    let h = harness();
    let result = h
        .call("computer_press_key", json!({ "key": vec!["a"; 17].join("+") }))
        .await;
    assert!(result.is_error);
    assert!(result.first_text().unwrap().contains("at most 16 keys"));
    assert!(h.backend.calls_for("pressKey").is_empty());
    assert!(h.backend.calls_for("hotkey").is_empty());
    h.dispose().await;
}

#[tokio::test]
async fn a_run_executes_its_steps_in_order_and_closes_with_fresh_state() {
    let h = harness();
    let result = h
        .call(
            "computer_run",
            json!({ "steps": [
                { "type": "set_value", "label": "Display", "value": "12" },
                { "type": "click", "label": "Calculate" },
            ] }),
        )
        .await;
    assert!(!result.is_error, "{result:?}");
    let payload = result_json(&result);
    assert_eq!(payload["completed"], json!(2));
    assert_eq!(payload["stopped"], json!(false));
    assert!(payload["state"]["elements"].is_array());
    h.dispose().await;
}

#[tokio::test]
async fn a_window_result_names_its_app_inside_the_transcript_summary() {
    let h = harness();
    h.call("computer_list_windows", json!({})).await;
    for (tool, args) in [
        (
            "computer_get_state",
            json!({ "window_id": "fake-calculator" }),
        ),
        (
            "computer_type_text",
            json!({ "window_id": "fake-calculator", "text": "hi" }),
        ),
        (
            "computer_press_key",
            json!({ "window_id": "fake-calculator", "key": "enter" }),
        ),
    ] {
        let result = h.call(tool, args).await;
        assert!(!result.is_error, "{tool} {result:?}");
        assert_eq!(
            result_json(&result)["appName"],
            json!("org.kde.kcalc"),
            "{tool}"
        );
        let summary = zeron_doc::summarize_tool_output(result.first_text().unwrap()).unwrap();
        assert!(
            summary.contains(r#""appName":"org.kde.kcalc""#),
            "{tool}: {summary}"
        );
    }
    h.dispose().await;
}
