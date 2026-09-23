use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use futures::FutureExt;
use futures::future::{BoxFuture, Shared};
use serde_json::{Map, Value, json};
use tokio::sync::oneshot;
use zeron_proto::computer::{
    ComputerDeliveryMode, ComputerHostCall, ComputerHostMethod, ComputerPoint, ComputerTarget,
    ComputerUiNode, CuaEffect, CuaReply,
};
use zeron_rpc::ComputerHost;

use super::{CuaComputerBackend, CuaComputerBackendOptions};
use crate::computer::backend::{
    ComputerBackend, ComputerCaptureRequest, ComputerError, ComputerResolvedTarget,
    ComputerStateOptions, ComputerTextRange,
};
use crate::computer::desktop_operation_queue::with_desktop_delivery_mode;
use crate::computer::model_desktop_observation::with_model_desktop_observation;

mod apps_windows_menus;
mod boundary;
mod hardening_preview_browser;
mod host_transport;
mod lock_resume;
mod permissions_readiness;
mod preview_eviction;
mod routes_keys_scroll_drag;
mod space_churn;
mod still_cadence;
mod text_lanes;

pub(crate) type ToolHandler = Arc<dyn Fn(&Map<String, Value>) -> Value + Send + Sync>;

#[derive(Clone)]
pub(crate) struct Gate(Shared<BoxFuture<'static, ()>>);

pub(super) struct GateOpener(Option<oneshot::Sender<()>>);

impl GateOpener {
    pub(super) fn open(mut self) {
        if let Some(sender) = self.0.take() {
            let _ = sender.send(());
        }
    }
}

pub(super) fn gate() -> (GateOpener, Gate) {
    let (sender, receiver) = oneshot::channel::<()>();
    let wait = async move {
        let _ = receiver.await;
    }
    .boxed()
    .shared();
    (GateOpener(Some(sender)), Gate(wait))
}

impl Gate {
    async fn wait(self) {
        self.0.await;
    }
}

pub(crate) struct Knobs {
    pub bounds: Value,
    pub live: bool,
    pub elements: Vec<Value>,
    pub failure: Option<Value>,
    pub native_refusal: bool,
    pub desktop_paused: bool,
    pub desktop_epoch: u64,
    pub missing_permissions: bool,
    pub screen_recording_missing: bool,
    pub monitor_permissions: Map<String, Value>,
    pub permission_wait: Option<Gate>,
    pub overview_failure: bool,
    pub capture_window_id: u64,
    pub capture_pid: u64,
    pub capture_frame_valid: bool,
    pub capture_frame_freshness: String,
    pub visible: bool,
    pub ready: Value,
    pub move_after_capture: bool,
    pub overview_wait: Option<Gate>,
    pub window_state_wait: Option<Gate>,
    pub type_gate: Option<Gate>,
    pub click_gate: Option<Gate>,
    pub typing_in_flight: usize,
    pub typing_max_in_flight: usize,
    pub extra_windows: Vec<Value>,
    pub tool_handlers: HashMap<String, ToolHandler>,
    pub set_value_swallowed: bool,
    pub action_result: Value,
    pub host_platform: String,
    pub native_revision: Option<u64>,
    pub interruptions: Option<u64>,
    pub pauses: Option<Vec<String>>,
}

#[derive(Default)]
pub(crate) struct FixtureOptions {
    pub lane_hold_ms: Option<u64>,
    pub lane_gap_ms: Option<u64>,
    pub host_platform: Option<&'static str>,
    pub native_revision: Option<Option<u64>>,
    pub still_interval_ms: Option<u64>,
}

pub(super) struct ScriptedHost {
    pub knobs: Arc<Mutex<Knobs>>,
    pub calls: Arc<Mutex<Vec<ComputerHostCall>>>,
    pub cancelled: Arc<Mutex<Vec<String>>>,
}

pub(super) fn png_header_base64() -> String {
    use base64::Engine as _;
    let mut header = vec![0u8; 24];
    header[..8].copy_from_slice(&[137, 80, 78, 71, 13, 10, 26, 10]);
    header[12..16].copy_from_slice(b"IHDR");
    header[16..20].copy_from_slice(&400u32.to_be_bytes());
    header[20..24].copy_from_slice(&200u32.to_be_bytes());
    base64::engine::general_purpose::STANDARD.encode(header)
}

fn is_typing(name: Option<&str>) -> bool {
    name == Some("type_text")
}

impl ScriptedHost {
    fn reply(&self, body: Value) -> CuaReply {
        let knobs = self.knobs.lock().unwrap();
        let mut body = body;
        if let Some(revision) = knobs.native_revision {
            body["driverNativeRevision"] = json!(revision);
        }
        if let Some(interruptions) = knobs.interruptions {
            body["desktopInterruptions"] = json!(interruptions);
        }
        if let Some(pauses) = &knobs.pauses {
            body["desktopPauses"] = json!(pauses);
        }
        serde_json::from_value(body).expect("scripted reply decodes")
    }

    async fn respond(&self, call: &ComputerHostCall) -> Value {
        let name = call.name.as_deref();
        let args = call.args.as_object().cloned().unwrap_or_default();
        let click_gate = self.knobs.lock().unwrap().click_gate.clone();
        if name == Some("click")
            && let Some(gate) = click_gate
        {
            gate.wait().await;
        }
        let (epoch, platform) = {
            let knobs = self.knobs.lock().unwrap();
            (knobs.desktop_epoch, knobs.host_platform.clone())
        };
        if matches!(
            call.method,
            ComputerHostMethod::Probe | ComputerHostMethod::Stop
        ) {
            return json!({ "ok": true, "desktopEpoch": epoch, "hostPlatform": platform });
        }
        let paused = self
            .knobs
            .lock()
            .unwrap()
            .pauses
            .as_ref()
            .is_some_and(|pauses| !pauses.is_empty());
        if paused && call.method == ComputerHostMethod::Call {
            return json!({
                "ok": true,
                "desktopEpoch": epoch,
                "result": {
                    "isError": true,
                    "structuredContent": {
                        "effect": "refused",
                        "code": "desktop_input_paused",
                        "message": "Desktop is locked.",
                    },
                },
            });
        }
        if self.knobs.lock().unwrap().desktop_paused && (name == Some("click") || is_typing(name)) {
            return json!({
                "ok": true,
                "desktopEpoch": epoch,
                "result": {
                    "isError": true,
                    "structuredContent": {
                        "effect": "refused",
                        "code": "desktop_input_paused",
                        "message": "Desktop is locked.",
                    },
                },
            });
        }
        if is_typing(name) {
            let (failure, gate) = {
                let mut knobs = self.knobs.lock().unwrap();
                knobs.typing_in_flight += 1;
                knobs.typing_max_in_flight = knobs.typing_max_in_flight.max(knobs.typing_in_flight);
                (knobs.failure.clone(), knobs.type_gate.clone())
            };
            if failure.is_none()
                && let Some(gate) = gate
            {
                gate.wait().await;
            }
            let refusal = {
                let mut knobs = self.knobs.lock().unwrap();
                knobs.typing_in_flight -= 1;
                knobs.native_refusal
            };
            if let Some(failure) = failure {
                return failure;
            }
            if refusal {
                return json!({
                    "ok": true,
                    "desktopEpoch": epoch,
                    "result": {
                        "isError": true,
                        "structuredContent": {
                            "effect": "refused",
                            "code": "same_pid_keyboard_ambiguity",
                        },
                        "content": [{ "type": "text", "text": "No actuator ran." }],
                    },
                });
            }
        }
        let mut data = json!({});
        if name == Some("check_permissions") {
            let wait = {
                let knobs = self.knobs.lock().unwrap();
                let mut permission = json!({
                    "accessibility": !knobs.missing_permissions,
                    "screen_recording": !knobs.missing_permissions && !knobs.screen_recording_missing,
                    "source": { "host_bundle_id": "com.synara.test" },
                });
                for (key, value) in &knobs.monitor_permissions {
                    permission[key] = value.clone();
                }
                data = permission;
                knobs.permission_wait.clone()
            };
            if let Some(wait) = wait {
                wait.wait().await;
            }
        }
        if name == Some("list_windows") {
            let knobs = self.knobs.lock().unwrap();
            data = if knobs.live {
                let mut windows = vec![json!({
                    "pid": 10,
                    "window_id": 20,
                    "title": "Owned fixture",
                    "bounds": knobs.bounds,
                    "is_on_screen": knobs.visible,
                    "on_current_space": knobs.visible,
                    "z_index": 1,
                })];
                windows.extend(knobs.extra_windows.iter().cloned());
                windows.push(json!({
                    "pid": 20,
                    "window_id": 30,
                    "bounds": { "x": 0, "y": 0, "width": 0, "height": 0 },
                }));
                json!({ "windows": windows })
            } else {
                json!({ "windows": [] })
            };
        }
        if name == Some("check_input_ready") {
            data = self.knobs.lock().unwrap().ready.clone();
        }
        if name == Some("get_screen_size") {
            data = json!({ "width": 1000, "height": 800, "scale_factor": 2 });
        }
        if name == Some("get_desktop_state") {
            let (failure, wait) = {
                let knobs = self.knobs.lock().unwrap();
                (knobs.overview_failure, knobs.overview_wait.clone())
            };
            if failure {
                return json!({ "ok": false, "error": "Capture denied before permission recovery" });
            }
            if let Some(wait) = wait {
                wait.wait().await;
            }
            return json!({
                "ok": true,
                "desktopEpoch": epoch,
                "result": {
                    "structuredContent": { "screen_width": 200, "screen_height": 100 },
                    "content": [{ "type": "image", "mimeType": "image/png", "data": png_header_base64() }],
                },
            });
        }
        if name == Some("get_window_state") {
            let wait = self.knobs.lock().unwrap().window_state_wait.clone();
            if let Some(wait) = wait {
                wait.wait().await;
            }
            let mut knobs = self.knobs.lock().unwrap();
            let result = json!({
                "structuredContent": {
                    "pid": knobs.capture_pid,
                    "window_id": knobs.capture_window_id,
                    "window_bounds": knobs.bounds,
                    "screenshot_frame_valid": knobs.capture_frame_valid,
                    "screenshot_frame_freshness": knobs.capture_frame_freshness,
                    "elements": knobs.elements,
                },
                "content": [{ "type": "image", "mimeType": "image/png", "data": png_header_base64() }],
            });
            if knobs.move_after_capture {
                knobs.bounds["x"] = json!(-250);
            }
            return json!({ "ok": true, "result": result, "desktopEpoch": epoch });
        }
        if name == Some("set_value") {
            let mut knobs = self.knobs.lock().unwrap();
            if !knobs.set_value_swallowed {
                let token = args.get("element_token").cloned();
                let written = args.get("value").and_then(Value::as_str).map(str::to_owned);
                let append = args.get("append") == Some(&Value::Bool(true));
                if let (Some(token), Some(written)) = (token, written)
                    && let Some(target) = knobs
                        .elements
                        .iter_mut()
                        .find(|element| element.get("element_token") == Some(&token))
                {
                    let next = if append {
                        format!("{}{written}", target["value"].as_str().unwrap_or(""))
                    } else {
                        written
                    };
                    target["value"] = json!(next);
                }
            }
        }
        if is_typing(name) {
            data = self.knobs.lock().unwrap().action_result.clone();
        }
        let handler =
            name.and_then(|name| self.knobs.lock().unwrap().tool_handlers.get(name).cloned());
        if let Some(handler) = handler {
            return json!({
                "ok": true,
                "result": handler(&args),
                "desktopEpoch": epoch,
                "hostPlatform": platform,
            });
        }
        json!({
            "ok": true,
            "result": { "structuredContent": data },
            "desktopEpoch": epoch,
            "hostPlatform": platform,
        })
    }
}

#[async_trait]
impl ComputerHost for ScriptedHost {
    async fn request(&self, call: ComputerHostCall) -> CuaReply {
        self.calls.lock().unwrap().push(call.clone());
        let body = self.respond(&call).await;
        self.reply(body)
    }

    fn cancel(&self, correlation_id: &str) {
        self.cancelled
            .lock()
            .unwrap()
            .push(correlation_id.to_owned());
    }
}

pub(crate) struct Fixture {
    pub backend: Arc<CuaComputerBackend>,
    pub knobs: Arc<Mutex<Knobs>>,
    pub calls: Arc<Mutex<Vec<ComputerHostCall>>>,
    pub cancelled: Arc<Mutex<Vec<String>>>,
}

pub(super) fn fixture() -> Fixture {
    fixture_with(FixtureOptions::default())
}

pub(crate) fn fixture_with(options: FixtureOptions) -> Fixture {
    let knobs = Arc::new(Mutex::new(Knobs {
        bounds: json!({ "x": -300, "y": 20, "width": 200, "height": 100 }),
        live: true,
        elements: Vec::new(),
        failure: None,
        native_refusal: false,
        desktop_paused: false,
        desktop_epoch: 0,
        missing_permissions: false,
        screen_recording_missing: false,
        monitor_permissions: Map::new(),
        permission_wait: None,
        overview_failure: false,
        capture_window_id: 20,
        capture_pid: 10,
        capture_frame_valid: true,
        capture_frame_freshness: "captured_current_space".into(),
        visible: true,
        ready: json!({ "ready": true, "pid": 10, "window_id": 20 }),
        move_after_capture: false,
        overview_wait: None,
        window_state_wait: None,
        type_gate: None,
        click_gate: None,
        typing_in_flight: 0,
        typing_max_in_flight: 0,
        extra_windows: Vec::new(),
        tool_handlers: HashMap::new(),
        set_value_swallowed: false,
        action_result: json!({
            "route": "synthetic_events",
            "delivery": { "mode": "background" },
            "effect": "unverifiable",
        }),
        host_platform: options.host_platform.unwrap_or("darwin").to_owned(),
        native_revision: options.native_revision.unwrap_or(Some(34)),
        interruptions: None,
        pauses: None,
    }));
    let calls = Arc::new(Mutex::new(Vec::new()));
    let cancelled = Arc::new(Mutex::new(Vec::new()));
    let host = Arc::new(ScriptedHost {
        knobs: knobs.clone(),
        calls: calls.clone(),
        cancelled: cancelled.clone(),
    });
    let backend = Arc::new(CuaComputerBackend::new(
        host,
        CuaComputerBackendOptions {
            semantic_text_lane_hold_ms: options.lane_hold_ms,
            semantic_text_lane_gap_ms: options.lane_gap_ms,
            still_interval_ms: options.still_interval_ms,
        },
    ));
    Fixture {
        backend,
        knobs,
        calls,
        cancelled,
    }
}

impl Fixture {
    pub(crate) fn knobs(&self) -> std::sync::MutexGuard<'_, Knobs> {
        self.knobs.lock().unwrap()
    }

    pub(crate) fn set_elements(&self, elements: Vec<Value>) {
        self.knobs().elements = elements;
    }

    pub(super) fn swallow_set_value(&self) {
        self.knobs().set_value_swallowed = true;
    }

    pub(super) fn set_windows(&self, windows: Vec<Value>) {
        self.knobs().extra_windows = windows;
    }

    pub(super) fn set_bounds(&self, bounds: Value) {
        self.knobs().bounds = bounds;
    }

    pub(crate) fn on_tool(
        &self,
        name: &str,
        handler: impl Fn(&Map<String, Value>) -> Value + Send + Sync + 'static,
    ) {
        self.knobs()
            .tool_handlers
            .insert(name.to_owned(), Arc::new(handler));
    }

    pub(super) fn gate_type_text(&self, gate: Option<Gate>) {
        self.knobs().type_gate = gate;
    }

    pub(super) fn gate_clicks(&self, gate: Option<Gate>) {
        self.knobs().click_gate = gate;
    }

    pub(super) fn typing_max_in_flight(&self) -> usize {
        self.knobs().typing_max_in_flight
    }

    pub(super) fn pause_desktop(&self, paused: bool) {
        self.knobs().desktop_paused = paused;
    }

    pub(super) fn change_desktop(&self) {
        self.knobs().desktop_epoch += 1;
    }

    pub(super) fn delay_overview(&self, gate: Gate) {
        self.knobs().overview_wait = Some(gate);
    }

    pub(super) fn delay_window_state(&self, gate: Option<Gate>) {
        self.knobs().window_state_wait = gate;
    }

    pub(super) fn set_visible(&self, visible: bool) {
        self.knobs().visible = visible;
    }

    pub(super) fn readiness(&self, value: Value) {
        self.knobs().ready = value;
    }

    pub(super) fn capture_window(&self, window_id: u64, pid: u64) {
        let mut knobs = self.knobs();
        knobs.capture_window_id = window_id;
        knobs.capture_pid = pid;
    }

    pub(super) fn invalidate_capture(&self) {
        self.knobs().capture_frame_valid = false;
    }

    pub(super) fn mark_off_space_capture_unverified(&self) {
        self.knobs().capture_frame_freshness = "unverified_off_space".into();
    }

    pub(super) fn action_result(&self, value: Value) {
        self.knobs().action_result = value;
    }

    pub(super) fn move_window(&self) {
        self.knobs().bounds["x"] = json!(-250);
    }

    pub(super) fn close(&self) {
        self.knobs().live = false;
    }

    pub(super) fn fail(&self, reply: Value) {
        self.knobs().failure = Some(reply);
    }

    pub(super) fn unfail(&self) {
        self.knobs().failure = None;
    }

    pub(super) fn refuse(&self) {
        self.knobs().native_refusal = true;
    }

    pub(super) fn deny_permissions(&self) {
        self.knobs().missing_permissions = true;
    }

    pub(super) fn grant_permissions(&self) {
        let mut knobs = self.knobs();
        knobs.missing_permissions = false;
        knobs.screen_recording_missing = false;
    }

    pub(super) fn deny_screen_recording(&self) {
        self.knobs().screen_recording_missing = true;
    }

    pub(super) fn set_input_monitor(&self, granted: bool, ready: bool) {
        let mut monitor = Map::new();
        monitor.insert("input_monitoring".into(), json!(granted));
        monitor.insert("input_monitor_ready".into(), json!(ready));
        self.knobs().monitor_permissions = monitor;
    }

    pub(super) fn wait_for_permission(&self, gate: Option<Gate>) {
        self.knobs().permission_wait = gate;
    }

    pub(super) fn fail_overview(&self) {
        self.knobs().overview_failure = true;
    }

    pub(crate) fn calls(&self) -> Vec<ComputerHostCall> {
        self.calls.lock().unwrap().clone()
    }

    pub(super) fn clear_calls(&self) {
        self.calls.lock().unwrap().clear();
    }

    pub(crate) fn call_count(&self) -> usize {
        self.calls.lock().unwrap().len()
    }

    pub(crate) fn named(&self, name: &str) -> Vec<ComputerHostCall> {
        self.calls()
            .into_iter()
            .filter(|call| call.name.as_deref() == Some(name))
            .collect()
    }

    pub(super) fn first_args(&self, name: &str) -> Value {
        self.named(name)
            .first()
            .map(|call| call.args.clone())
            .unwrap_or(Value::Null)
    }

    pub(crate) fn last_args(&self, name: &str) -> Value {
        self.named(name)
            .last()
            .map(|call| call.args.clone())
            .unwrap_or(Value::Null)
    }
}

pub(crate) fn assert_subset(actual: &Value, expected: Value) {
    let Value::Object(expected) = expected else {
        assert_eq!(actual, &expected);
        return;
    };
    for (key, value) in expected {
        let found = actual.get(&key).unwrap_or(&Value::Null);
        match value {
            Value::Object(_) => assert_subset(found, value),
            _ => assert_eq!(found, &value, "field {key} of {actual}"),
        }
    }
}

pub(super) fn window_state(window_id: &str) -> ComputerStateOptions {
    ComputerStateOptions {
        window_id: Some(window_id.to_owned()),
        include_tree: true,
        ..ComputerStateOptions::default()
    }
}

pub(super) fn resolved(label: &str, node: &ComputerUiNode) -> ComputerResolvedTarget {
    resolved_in(label, None, node)
}

pub(super) fn resolved_in(
    label: &str,
    window_id: Option<&str>,
    node: &ComputerUiNode,
) -> ComputerResolvedTarget {
    ComputerResolvedTarget {
        target: ComputerTarget {
            label: Some(label.to_owned()),
            window_id: window_id.map(str::to_owned),
            ..ComputerTarget::default()
        }
        .into(),
        point: node
            .activation_point
            .unwrap_or(ComputerPoint { x: 0.0, y: 0.0 }),
        node: node.clone(),
    }
}

pub(super) fn point(x: f64, y: f64) -> ComputerPoint {
    ComputerPoint { x, y }
}

pub(super) fn window_capture(window_id: &str) -> ComputerCaptureRequest {
    ComputerCaptureRequest::Window {
        window_id: window_id.to_owned(),
        max_dimension: None,
    }
}

pub(super) fn range(start: u64, length: u64) -> ComputerTextRange {
    ComputerTextRange { start, length }
}

pub(super) fn code(error: &ComputerError) -> Option<&str> {
    error.code()
}

pub(super) fn effect(error: &ComputerError) -> Option<CuaEffect> {
    error.effect()
}

pub(super) async fn foreground<F: std::future::Future>(future: F) -> F::Output {
    with_desktop_delivery_mode(ComputerDeliveryMode::Foreground, future).await
}

pub(super) async fn observing<F: std::future::Future>(future: F) -> F::Output {
    with_model_desktop_observation(future).await
}

#[tokio::test]
async fn the_harness_answers_like_the_synara_fixture() {
    let f = fixture();
    let state = f
        .backend
        .get_state(window_state("cua:10:20"))
        .await
        .unwrap();
    assert_eq!(state.windows.len(), 1);
    assert_eq!(state.windows[0].id, "cua:10:20");
    let root = state.root.unwrap();
    assert_eq!(root.role, "AXWindow");
    assert!(root.children.is_empty());
    let windows = f.backend.list_windows().await.unwrap();
    assert_eq!(windows.len(), 1);
    assert_eq!(f.named("check_permissions").len(), 1);
}
