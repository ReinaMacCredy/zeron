use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, LazyLock, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use async_trait::async_trait;
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use futures::FutureExt;
use futures::future::{BoxFuture, Shared};
use regex::Regex;
use serde_json::{Map, Value, json};
use tokio::time::Instant;
use zeron_proto::computer::{
    COMPUTER_WINDOW_LIST_MAX_LENGTH, CUA_DEFAULT_TIMEOUT_MS, CUA_MAX_REQUEST_BYTES,
    CUA_SETUP_TIMEOUT_MS, ComputerAccessibilityReport, ComputerAccessibilityStatus,
    ComputerAccessibilityTreeApp, ComputerAccessibilityTreeWindow, ComputerApp,
    ComputerAvailability, ComputerAxLine, ComputerAxTree, ComputerBuildSignature,
    ComputerCapabilities, ComputerDeliveryMode, ComputerDeliveryVerification, ComputerHealth,
    ComputerHealthFailure, ComputerHealthStatus, ComputerHostCall, ComputerHostMethod,
    ComputerInputModifier, ComputerInputPause, ComputerJpegMimeType, ComputerLaunchAppResult,
    ComputerLaunchWindowReason, ComputerLaunchWindowStatus, ComputerPermission,
    ComputerPngMimeType, ComputerPoint, ComputerRect, ComputerScreenSize, ComputerScreenshot,
    ComputerScrollDelta, ComputerSpaceErrorCode, ComputerSpaceInventory, ComputerState,
    ComputerUiNode, ComputerVerifyStateResult, ComputerVerifyStatus, ComputerVisibleRows,
    ComputerWindow, ComputerZoomResult, CuaActuator, CuaComputerTask, CuaEffect, CuaReply,
    CuaToolResult, CuaTransportError, computer_permission_setup_message, cua_computer_task_key,
    list_computer_permissions, parse_cua_action_diagnostics,
};
use zeron_rpc::ComputerHost;

use super::backend::{
    ComputerAccessibilityTreeSnapshot, ComputerAgentDialect, ComputerBackend,
    ComputerBackendActionResult, ComputerBackendEvent, ComputerBackendEventListener,
    ComputerBackendListeners, ComputerBackendMethod, ComputerBackendSubscription,
    ComputerBrowserBackend, ComputerBrowserCall, ComputerBrowserCallResult, ComputerCaptureRequest,
    ComputerCursorReading, ComputerError, ComputerFrameListener, ComputerLaunchOptions,
    ComputerMenuBackendTarget, ComputerResolvedTarget, ComputerSettleOptions,
    ComputerSettleOutcome, ComputerShieldTarget, ComputerStateOptions, ComputerTextRange,
    CuaActionError, CuaErrorLayer, DEFAULT_COMPUTER_ID, assert_computer_clipboard_write_fits,
    js_slice,
};
use super::call_context::{
    ComputerCallContext, ZERON_CUA_PREVIEW_STILL_MS, cua_preview_still_ms_override,
    current_computer_call, timed_computer_leg, with_computer_call_context,
};
use super::cua_space_inventory::cua_space_inventory;
use super::desktop_operation_queue::{
    OperationSignal, assert_desktop_operation_active, desktop_delivery_mode,
    desktop_operation_signal, with_desktop_delivery_mode, with_desktop_operation_signal,
};
use super::element_identity::{observed_computer_target_identity, observed_computer_target_node};
use super::geometry::{jpeg_dimensions, png_dimensions};
use super::model_desktop_observation::{
    is_model_desktop_observation_active, with_model_desktop_observation,
};
use super::still_frame_publisher::{
    StillFramePublisher, StillFramePublisherOptions, resolve_still_interval_ms,
};
use super::task_context::{current_computer_task, with_computer_task};
use super::tools::action_words;
use super::ui_tree_targeting::{ComputerTargetError, activation_point_for_node};
use super::utf8_truncation::utf16_len;

// Longer than the driver's 25 s typing budget so the lane never reports a timeout while typing runs.
pub const CUA_SEMANTIC_TEXT_LANE_HOLD_MS: u64 = 28_000;
pub const CUA_SEMANTIC_TEXT_LANE_GAP_MS: u64 = 100;
pub const CUA_HOST_CALL_TIMEOUT_MS: u64 = 35_000;
// Until the host has answered once, the first calls also pay for starting the driver
// (spawn, handshake, session), whose own limits add up to about 70 s.
pub const CUA_COLD_HOST_CALL_TIMEOUT_MS: u64 = 80_000;
pub const CLIPBOARD_NOT_RESTORABLE_CODE: &str = "clipboard_not_restorable";
pub const CUA_SHIELD_TIMEOUT_MS: u64 = 5_000;
pub const CUA_BROWSER_STILL_TIMEOUT_MS: u64 = 20_000;
pub const CUA_STILL_FRAME_INTERVAL_MS: u64 = 1_000;
const RECENT_TREE_TTL: Duration = Duration::from_millis(5_000);
// The driver culls web content to about three viewports, so this cap still reaches on-screen controls.
const CUA_WALK_MAX_ELEMENTS: u64 = 3_000;
const CUA_TREE_ELEMENT_LIMIT: usize = 2_048;
const RECENT_TREE_LIMIT: usize = 8;
const AX_ERROR_CANNOT_COMPLETE: i32 = -25204;
const NATIVE_ELEMENT_LIMIT: usize = RECENT_TREE_LIMIT * 1_024;
const PREVIEW_TASK_LIMIT: usize = 256;
const SNAPSHOT_FRESH_FOR: Duration = Duration::from_millis(1_000);
const PERMISSION_REPROBE_DELAY: Duration = Duration::from_millis(600);
const KILL_POLL_DELAY: Duration = Duration::from_millis(250);
const DEFAULT_HOST_PLATFORM: &str = "darwin";
const GUI_HOST_REQUIRED: &str =
    "Open this session in a supported Zeron desktop app to use Computer.";
const MONITOR_MESSAGE: &str = "Computer control is paused because the Escape and human-input listener could not start. Reopen Zeron, then check Computer settings again.";

struct ElementAction {
    driver_action: String,
    ax_action: String,
}

fn cua_element_action(name: &str, advertised: Option<&HashSet<String>>) -> Option<ElementAction> {
    let known = match name.to_lowercase().as_str() {
        "axpress" | "press" => Some(("press", "AXPress")),
        "open" => Some(("open", "AXOpen")),
        "show_menu" | "menu" => Some(("show_menu", "AXShowMenu")),
        "pick" => Some(("pick", "AXPick")),
        "confirm" => Some(("confirm", "AXConfirm")),
        "cancel" => Some(("cancel", "AXCancel")),
        _ => None,
    };
    if let Some((driver_action, ax_action)) = known {
        return Some(ElementAction {
            driver_action: driver_action.to_owned(),
            ax_action: ax_action.to_owned(),
        });
    }
    let advertised = advertised?;
    let name = name.trim();
    let raw = advertised.get(name).or_else(|| {
        let wanted = name.to_lowercase();
        let mut named = advertised.iter().filter(|raw| {
            raw.to_lowercase() == wanted || action_words(raw).to_lowercase() == wanted
        });
        let only = named.next()?;
        named.next().is_none().then_some(only)
    })?;
    Some(ElementAction {
        driver_action: raw.clone(),
        ax_action: raw.clone(),
    })
}

fn cua_key(value: &str) -> Result<String, ComputerError> {
    let key = value.to_lowercase();
    if key == "insert" || key == "ins" {
        return Err(CuaActionError::new(
            "Cua 0.28.2 has no Insert key mapping on macOS.",
            CuaEffect::NotDispatched,
        )
        .code("unsupported_operation")
        .into());
    }
    let alias = match key.as_str() {
        "meta" | "super" | "super_l" | "win" => "command",
        "delete" | "del" => "forward_delete",
        "arrowleft" => "left",
        "arrowright" => "right",
        "arrowup" => "up",
        "arrowdown" => "down",
        " " => "space",
        "page_up" | "pgup" | "prior" => "pageup",
        "page_down" | "pgdn" | "next" => "pagedown",
        "caps_lock" => "capslock",
        "shift_l" => "shift",
        "ctrl_l" | "control_l" => "ctrl",
        "alt_l" | "option_l" => "alt",
        _ => return Ok(key),
    };
    Ok(alias.to_owned())
}

fn modifier_name(modifier: ComputerInputModifier) -> &'static str {
    match modifier {
        ComputerInputModifier::Ctrl => "ctrl",
        ComputerInputModifier::Alt => "alt",
        ComputerInputModifier::Shift => "shift",
        ComputerInputModifier::Meta => "meta",
    }
}

fn cua_modifiers(modifiers: &[ComputerInputModifier]) -> Result<Vec<Value>, ComputerError> {
    modifiers
        .iter()
        .map(|modifier| cua_key(modifier_name(*modifier)).map(Value::String))
        .collect()
}

fn record(value: Option<&Value>) -> Map<String, Value> {
    value
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default()
}

fn text(value: Option<&Value>, max: usize) -> String {
    value
        .and_then(Value::as_str)
        .map(|value| js_slice(value, max).to_owned())
        .unwrap_or_default()
}

fn text1024(value: Option<&Value>) -> String {
    text(value, 1024)
}

#[cfg(any(target_os = "macos", test))]
fn app_bundle_of_executable(executable: &str) -> Option<String> {
    let end = executable.rfind(".app/Contents/")? + ".app".len();
    Some(executable[..end].to_owned())
}

// The driver copies a running app's launch_path from any installed app with its bundle id.
#[cfg(all(target_os = "macos", not(test)))]
fn running_app_bundle(pid: u32) -> Option<String> {
    let pid = libc::c_int::try_from(pid).ok()?;
    let mut buffer = vec![0u8; libc::PROC_PIDPATHINFO_MAXSIZE as usize];
    let length =
        unsafe { libc::proc_pidpath(pid, buffer.as_mut_ptr().cast(), buffer.len() as u32) };
    let length = usize::try_from(length).ok().filter(|length| *length > 0)?;
    app_bundle_of_executable(std::str::from_utf8(&buffer[..length]).ok()?)
}

#[cfg(all(not(target_os = "macos"), not(test)))]
fn running_app_bundle(_pid: u32) -> Option<String> {
    None
}

fn same_file(a: &str, b: &str) -> bool {
    std::fs::canonicalize(a)
        .ok()
        .is_some_and(|a| std::fs::canonicalize(b).ok() == Some(a))
}

#[cfg(test)]
fn running_app_bundle(pid: u32) -> Option<String> {
    tests::running_executable(pid).and_then(|executable| app_bundle_of_executable(&executable))
}

fn ax_press_cannot_complete(error: &ComputerError) -> bool {
    error
        .cua()
        .and_then(|cua| cua.diagnostics.as_ref())
        .is_some_and(|diagnostics| {
            diagnostics.actuator == Some(CuaActuator::AxPress)
                && diagnostics.ax_error == Some(AX_ERROR_CANNOT_COMPLETE)
        })
}

type ObservedSurface = (
    Option<String>,
    Vec<(String, Option<String>, Option<String>, ComputerRect)>,
);

fn observed_surface(root: &ComputerUiNode) -> ObservedSurface {
    (
        root.label.clone(),
        root.children
            .iter()
            .map(|child| {
                (
                    child.role.clone(),
                    child.label.clone(),
                    child.value.clone(),
                    child.frame,
                )
            })
            .collect(),
    )
}

fn app_window_ids(windows: &[ComputerWindow], pid: Option<u32>) -> Vec<String> {
    let mut ids: Vec<String> = windows
        .iter()
        .filter(|window| pid.is_some() && window.pid == pid)
        .map(|window| window.id.clone())
        .collect();
    ids.sort();
    ids
}

fn elements_by_index(data: &Map<String, Value>) -> HashMap<u64, &Value> {
    data.get("elements")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|element| Some((element.get("element_index")?.as_u64()?, element)))
        .collect()
}

const DESCENDANT_LABEL_ROLES: &[&str] = &[
    "AXRow",
    "AXCell",
    "AXButton",
    "AXRadioButton",
    "AXCheckBox",
    "AXMenuButton",
    "AXPopUpButton",
    "AXDisclosureTriangle",
    "AXLink",
    "AXTab",
];

fn descendant_text(nodes: &[Value], position: usize) -> Option<String> {
    let node = nodes.get(position)?;
    if !DESCENDANT_LABEL_ROLES.contains(&node.get("role")?.as_str()?) {
        return None;
    }
    let depth = |node: &Value| node.get("depth").and_then(Value::as_u64).unwrap_or(0);
    let mut texts: Vec<String> = Vec::new();
    for child in nodes[position + 1..]
        .iter()
        .take(64)
        .take_while(|child| depth(child) > depth(node))
    {
        let text = ["title", "description", "value"]
            .into_iter()
            .map(|key| text1024(child.get(key)).trim().to_owned())
            .find(|text| !text.is_empty());
        if let Some(text) = text
            && !texts.contains(&text)
        {
            texts.push(text);
        }
    }
    let joined = texts.join(", ");
    (!joined.is_empty()).then(|| js_slice(&joined, 1024).to_owned())
}

fn node_label(nodes: &[Value], position: usize, element: Option<&Value>) -> String {
    let node = &nodes[position];
    let named = match element {
        Some(element) => text1024(element.get("label")),
        None => ["title", "description", "value"]
            .into_iter()
            .map(|key| text1024(node.get(key)))
            .find(|name| !name.is_empty())
            .unwrap_or_default(),
    };
    if !named.is_empty() {
        return named;
    }
    Some(text1024(node.get("help")))
        .filter(|help| !help.is_empty())
        .or_else(|| descendant_text(nodes, position))
        .unwrap_or_default()
}

fn cua_ax_tree(data: &Map<String, Value>, targets: &HashMap<usize, u32>) -> Option<ComputerAxTree> {
    let nodes = data.get("tree_nodes")?.as_array()?;
    let elements = elements_by_index(data);
    let positions: HashMap<u64, usize> = nodes
        .iter()
        .enumerate()
        .filter_map(|(position, node)| Some((node.get("element_index")?.as_u64()?, position)))
        .collect();
    let optional = |value: Option<&Value>, max: usize| {
        Some(text(value, max)).filter(|value| !value.is_empty())
    };
    let line = |node: &Value, position: Option<usize>| {
        let index = node.get("element_index").and_then(Value::as_u64);
        let element = index.and_then(|index| elements.get(&index));
        let field = |name: &str| element.and_then(|element| element.get(name));
        let flag = |name: &str| {
            field(name)
                .or_else(|| node.get(name))
                .and_then(Value::as_bool)
        };
        let description = optional(node.get("description"), 1024);
        let value = optional(node.get("value"), 16384).or_else(|| optional(field("value"), 16384));
        let title = optional(node.get("title"), 1024).or_else(|| {
            (description.is_none() && value.is_none())
                .then(|| position.and_then(|position| descendant_text(nodes, position)))
                .flatten()
        });
        let visible_rows = node.get("visible_rows").and_then(|rows| {
            let row = |name: &str| rows.get(name).and_then(Value::as_u64);
            Some(ComputerVisibleRows {
                first: row("first")?,
                last: row("last")?,
                total: row("total")?,
            })
        });
        ComputerAxLine {
            depth: node
                .get("depth")
                .and_then(Value::as_u64)
                .map_or(0, |depth| depth.min(64) as u32),
            role: text(node.get("role"), 128),
            role_description: optional(node.get("role_description"), 128),
            title,
            description,
            value,
            identifier: optional(node.get("identifier"), 1024),
            help: optional(node.get("help"), 1024),
            actions: field("actions")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .take(32)
                .map(|action| js_slice(action, 256).to_owned())
                .collect(),
            settable: node.get("settable") == Some(&Value::Bool(true)),
            selected: flag("selected"),
            enabled: flag("enabled"),
            expanded: node.get("expanded").and_then(Value::as_bool),
            visible_rows,
            target: position
                .and_then(|position| targets.get(&position))
                .map(|child| vec![*child]),
        }
    };
    Some(ComputerAxTree {
        url: optional(data.get("window_url"), 2048),
        lines: nodes
            .iter()
            .take(4096)
            .enumerate()
            .map(|(position, node)| line(node, Some(position)))
            .collect(),
        focused: data
            .get("focused_element")
            .filter(|focused| focused.is_object())
            .map(|focused| {
                let position = focused
                    .get("element_index")
                    .and_then(Value::as_u64)
                    .and_then(|index| positions.get(&index).copied());
                line(focused, position)
            }),
        selected_text: optional(data.get("selected_text"), 16384),
        truncated: data.get("tree_truncated") == Some(&Value::Bool(true)),
    })
}

fn number(value: Option<&Value>) -> f64 {
    value
        .and_then(Value::as_f64)
        .filter(|value| value.is_finite())
        .unwrap_or(f64::NAN)
}

fn safe_integer(value: f64) -> Option<i64> {
    (value.is_finite() && value.fract() == 0.0 && value.abs() <= 9_007_199_254_740_991.0)
        .then_some(value as i64)
}

fn positive_id(value: f64) -> Option<u64> {
    safe_integer(value)
        .filter(|value| *value > 0)
        .map(|value| value as u64)
}

pub(crate) fn num(value: f64) -> Value {
    if value.fract() == 0.0 && value.abs() <= 9_007_199_254_740_991.0 {
        json!(value as i64)
    } else {
        json!(value)
    }
}

fn rect_json(rect: &ComputerRect) -> Value {
    json!({
        "x": num(rect.x),
        "y": num(rect.y),
        "width": num(rect.width),
        "height": num(rect.height),
    })
}

fn optional_rect(value: Option<&Value>) -> Option<ComputerRect> {
    let r = record(value);
    let width = match r.get("width") {
        Some(Value::Null) | None => r.get("w"),
        width => width,
    };
    let height = match r.get("height") {
        Some(Value::Null) | None => r.get("h"),
        height => height,
    };
    let rect = ComputerRect {
        x: number(r.get("x")),
        y: number(r.get("y")),
        width: number(width),
        height: number(height),
    };
    (rect.x.is_finite()
        && rect.y.is_finite()
        && rect.width.is_finite()
        && rect.height.is_finite()
        && rect.width > 0.0
        && rect.height > 0.0)
        .then_some(rect)
}

fn rect(value: Option<&Value>) -> Result<ComputerRect, ComputerError> {
    optional_rect(value).ok_or_else(|| {
        CuaActionError::new("Cua returned invalid geometry.", CuaEffect::NotDispatched)
            .code("invalid_geometry")
            .into()
    })
}

fn same_rect(a: &ComputerRect, b: &ComputerRect) -> bool {
    a.x == b.x && a.y == b.y && a.width == b.width && a.height == b.height
}

fn evidence_kinds(data: &Map<String, Value>) -> Vec<String> {
    data.get("evidence")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .map(|item| text1024(record(Some(item)).get("kind")))
                .collect()
        })
        .unwrap_or_default()
}

fn confirmed_value_readback(data: &Map<String, Value>) -> bool {
    data.get("effect").and_then(Value::as_str) == Some("confirmed")
        && evidence_kinds(data)
            .iter()
            .any(|kind| kind == "value_readback")
}

fn effect_str(data: &Map<String, Value>) -> Option<&str> {
    data.get("effect").and_then(Value::as_str)
}

fn resolvable_still_tab(value: Option<&Value>) -> Option<String> {
    let tabs: Vec<(String, bool)> = value?
        .as_array()?
        .iter()
        .filter_map(|entry| {
            let tab = record(Some(entry));
            let id = text1024(tab.get("tab_id"));
            (!id.is_empty()).then(|| (id, tab.get("active") == Some(&Value::Bool(true))))
        })
        .collect();
    if tabs.len() == 1 {
        return Some(tabs[0].0.clone());
    }
    let active: Vec<&(String, bool)> = tabs.iter().filter(|tab| tab.1).collect();
    (active.len() == 1).then(|| active[0].0.clone())
}

fn capture_access_available(permission: &Map<String, Value>, platform: &str) -> bool {
    if platform != "linux"
        || permission
            .get("screen_recording")
            .is_some_and(Value::is_boolean)
    {
        return permission.get("screen_recording") == Some(&Value::Bool(true));
    }
    permission.get("x11") == Some(&Value::Bool(true))
        || (permission.get("wayland") == Some(&Value::Bool(true))
            && permission.get("wayland_enabled") == Some(&Value::Bool(true)))
}

fn missing_computer_permissions(
    permission: &Map<String, Value>,
    platform: &str,
) -> Vec<ComputerPermission> {
    let mut missing = Vec::new();
    let accessibility = if platform == "linux" {
        match permission.get("atspi") {
            None | Some(Value::Null) => permission.get("accessibility"),
            atspi => atspi,
        }
    } else {
        permission.get("accessibility")
    };
    if accessibility != Some(&Value::Bool(true)) {
        missing.push(ComputerPermission::Accessibility);
    }
    if !capture_access_available(permission, platform) {
        missing.push(ComputerPermission::ScreenRecording);
    }
    if platform == "darwin" && permission.get("input_monitoring") == Some(&Value::Bool(false)) {
        missing.push(ComputerPermission::InputMonitoring);
    }
    missing
}

fn now_iso() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

fn base64_byte_length(data: &str) -> u64 {
    let mut bytes = data.len();
    if data.ends_with('=') {
        bytes -= 1;
        if data[..bytes].ends_with('=') {
            bytes -= 1;
        }
    }
    ((bytes * 3) >> 2) as u64
}

fn decode_base64_prefix(data: &str, chars: usize) -> Vec<u8> {
    let prefix = js_slice(data, chars);
    let usable = prefix.len() - prefix.len() % 4;
    BASE64.decode(&prefix[..usable]).unwrap_or_default()
}

pub(crate) fn parse_cua_window_id(window_id: &str) -> Option<(u64, u64)> {
    let rest = window_id.strip_prefix("cua:")?;
    let (pid, window) = rest.split_once(':')?;
    let valid = |part: &str| {
        !part.is_empty() && part.bytes().all(|b| b.is_ascii_digit()) && !part.starts_with('0')
    };
    if !valid(pid) || !valid(window) {
        return None;
    }
    Some((pid.parse().ok()?, window.parse().ok()?))
}

fn cua_window_id(pid: u64, window_id: u64) -> String {
    format!("cua:{pid}:{window_id}")
}

fn error_text(error: &ComputerError) -> String {
    js_slice(&error.to_string(), 2048).to_owned()
}

fn transport_error(error: CuaTransportError) -> ComputerError {
    CuaActionError::new(error.message, error.effect).into()
}

static BUNDLE_ID: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^[a-zA-Z][\w-]*(\.[\w-]+)+$").expect("bundle id pattern"));

#[derive(Clone)]
struct CallScope {
    signal: Option<OperationSignal>,
    task: Option<CuaComputerTask>,
    delivery: ComputerDeliveryMode,
    call: Option<Arc<ComputerCallContext>>,
    model_observation: bool,
}

impl CallScope {
    fn capture() -> Self {
        Self {
            signal: desktop_operation_signal(),
            task: current_computer_task(),
            delivery: desktop_delivery_mode(),
            call: current_computer_call(),
            model_observation: is_model_desktop_observation_active(),
        }
    }

    async fn run<T: Send + 'static>(self, future: BoxFuture<'static, T>) -> T {
        let mut future = future;
        if self.model_observation {
            future = Box::pin(with_model_desktop_observation(future));
        }
        if let Some(call) = self.call {
            future = Box::pin(with_computer_call_context(call, future));
        }
        if let Some(task) = self.task {
            future = Box::pin(with_computer_task(task, future));
        }
        if let Some(signal) = self.signal {
            future = Box::pin(with_desktop_operation_signal(signal, future));
        }
        with_desktop_delivery_mode(self.delivery, future).await
    }
}

struct CancelOnDrop {
    host: Arc<dyn ComputerHost>,
    correlation_id: String,
    armed: bool,
}

impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        if self.armed {
            self.host.cancel(&self.correlation_id);
        }
    }
}

struct HostRequest {
    method: ComputerHostMethod,
    name: Option<String>,
    args: Value,
    task: Option<CuaComputerTask>,
    delivery_mode: Option<ComputerDeliveryMode>,
    model_observation: Option<bool>,
}

impl HostRequest {
    fn new(method: ComputerHostMethod) -> Self {
        Self {
            method,
            name: None,
            args: Value::Null,
            task: None,
            delivery_mode: None,
            model_observation: None,
        }
    }
}

#[derive(Debug, Clone, Default)]
struct NativeElement {
    token: String,
    actions: Option<Arc<HashSet<String>>>,
    in_web_content: bool,
}

struct RecentTree {
    at: Instant,
    reusable: bool,
    root: ComputerUiNode,
    elements: Vec<Option<NativeElement>>,
}

#[derive(Debug, Clone, PartialEq)]
enum StillTarget {
    Window {
        window_id: String,
    },
    Browser {
        target_id: String,
        tab_id: Option<String>,
        task: CuaComputerTask,
    },
}

#[derive(Debug, Clone)]
struct ResolvedWindow {
    pid: u64,
    window_id: u64,
    window: ComputerWindow,
    baseline: Option<u64>,
}

type Snapshot = Shared<BoxFuture<'static, Result<(), ComputerError>>>;
type LaneTail = Shared<BoxFuture<'static, ()>>;

struct CuaState {
    windows: Vec<ComputerWindow>,
    size: ComputerScreenSize,
    permissions: Vec<ComputerPermission>,
    availability: ComputerAvailability,
    health: ComputerHealth,
    capture_failed: bool,
    snapshot_at: Option<Instant>,
    had_missing_permissions: bool,
    snapshot: Option<(u64, Snapshot)>,
    snapshot_generation: u64,
    selected_window: Option<String>,
    recent_trees: VecDeque<(String, RecentTree)>,
    native_elements: HashMap<String, NativeElement>,
    native_element_order: VecDeque<String>,
    observed_geometry: HashMap<String, ComputerRect>,
    desktop_epoch: Option<u64>,
    desktop_interruptions: Option<u64>,
    driver_native_revision: Option<u64>,
    host_platform: Option<String>,
    host_warm: bool,
    preview_tasks: Vec<(String, CuaComputerTask)>,
    still_target: Option<StillTarget>,
    disposed: bool,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct CuaComputerBackendOptions {
    pub semantic_text_lane_hold_ms: Option<u64>,
    pub semantic_text_lane_gap_ms: Option<u64>,
    pub still_interval_ms: Option<u64>,
}

pub fn cua_still_interval_ms(option: Option<u64>, env: Option<&str>) -> u64 {
    resolve_still_interval_ms(Some(option.unwrap_or_else(|| {
        cua_preview_still_ms_override(env).map_or(CUA_STILL_FRAME_INTERVAL_MS, |ms| ms as u64)
    })))
}

struct CuaInner {
    host: Arc<dyn ComputerHost>,
    state: Mutex<CuaState>,
    listeners: ComputerBackendListeners,
    lane_hold: Duration,
    lane_gap: Duration,
    lanes: Mutex<HashMap<String, LaneTail>>,
}

pub struct CuaComputerBackend {
    inner: Arc<CuaInner>,
    stills: StillFramePublisher,
}

#[derive(Clone)]
struct LaneAdmission {
    abandoned: Arc<AtomicBool>,
    dispatched: Arc<AtomicBool>,
    deadline: Instant,
    signal: Option<OperationSignal>,
}

impl LaneAdmission {
    fn check(&self) -> Result<(), ComputerError> {
        if self.abandoned.load(Ordering::SeqCst) || Instant::now() >= self.deadline {
            return Err(CuaActionError::new(
                "Semantic text admission expired; nothing was sent.",
                CuaEffect::NotDispatched,
            )
            .into());
        }
        if let Some(signal) = &self.signal {
            signal.check()?;
        }
        Ok(())
    }

    fn admit(&self) -> Result<(), ComputerError> {
        self.check()?;
        self.dispatched.store(true, Ordering::SeqCst);
        Ok(())
    }
}

type LaneWrite = Box<
    dyn FnOnce(
            LaneAdmission,
        ) -> BoxFuture<'static, Result<ComputerBackendActionResult, ComputerError>>
        + Send,
>;

impl CuaComputerBackend {
    pub fn new(host: Arc<dyn ComputerHost>, options: CuaComputerBackendOptions) -> Self {
        let inner = Arc::new(CuaInner {
            host,
            state: Mutex::new(CuaState {
                windows: Vec::new(),
                size: ComputerScreenSize {
                    width: 1,
                    height: 1,
                    scale: None,
                },
                permissions: Vec::new(),
                availability: ComputerAvailability::BackendUnavailable {
                    message: "Computer has not connected to the Zeron desktop app.".into(),
                },
                health: ComputerHealth {
                    status: ComputerHealthStatus::Unavailable,
                    consecutive_failures: 0,
                    reconnects: 0,
                    last_failure: None,
                    capture_available: false,
                    background_input_degraded: None,
                },
                capture_failed: false,
                snapshot_at: None,
                had_missing_permissions: false,
                snapshot: None,
                snapshot_generation: 0,
                selected_window: None,
                recent_trees: VecDeque::new(),
                native_elements: HashMap::new(),
                native_element_order: VecDeque::new(),
                observed_geometry: HashMap::new(),
                desktop_epoch: None,
                desktop_interruptions: None,
                driver_native_revision: None,
                host_platform: None,
                host_warm: false,
                preview_tasks: Vec::new(),
                still_target: None,
                disposed: false,
            }),
            listeners: ComputerBackendListeners::default(),
            lane_hold: Duration::from_millis(
                options
                    .semantic_text_lane_hold_ms
                    .unwrap_or(CUA_SEMANTIC_TEXT_LANE_HOLD_MS),
            ),
            lane_gap: Duration::from_millis(
                options
                    .semantic_text_lane_gap_ms
                    .unwrap_or(CUA_SEMANTIC_TEXT_LANE_GAP_MS),
            ),
            lanes: Mutex::new(HashMap::new()),
        });
        let capture = Arc::downgrade(&inner);
        let prepare = Arc::downgrade(&inner);
        let available = Arc::downgrade(&inner);
        let stills = StillFramePublisher::new(StillFramePublisherOptions {
            capture: Arc::new(move |_force| {
                let inner = capture.upgrade();
                async move {
                    match inner {
                        Some(inner) => inner.capture_still().await,
                        None => Ok(None),
                    }
                }
                .boxed()
            }),
            is_capture_available: Arc::new(move || {
                available.upgrade().is_some_and(|inner| {
                    let state = inner.state();
                    !state.disposed
                        && !state
                            .permissions
                            .contains(&ComputerPermission::ScreenRecording)
                })
            }),
            prepare: Some(Arc::new(move || {
                let inner = prepare.upgrade();
                async move {
                    if let Some(inner) = inner {
                        inner.availability(false).await;
                    }
                }
                .boxed()
            })),
            emit: Arc::new(|_| {}),
            now: Arc::new(|| chrono::Utc::now().timestamp_millis() as f64),
            interval_ms: cua_still_interval_ms(
                options.still_interval_ms,
                std::env::var(ZERON_CUA_PREVIEW_STILL_MS).ok().as_deref(),
            ),
        });
        Self { inner, stills }
    }

    pub fn still_interval_ms(&self) -> u64 {
        self.stills.interval_ms()
    }

    pub async fn capture_still(&self) -> Result<Option<Vec<u8>>, ComputerError> {
        self.inner.capture_still().await
    }
}

impl CuaInner {
    fn state(&self) -> MutexGuard<'_, CuaState> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn platform(&self) -> String {
        self.state()
            .host_platform
            .clone()
            .unwrap_or_else(|| DEFAULT_HOST_PLATFORM.to_owned())
    }

    fn is_darwin(&self) -> bool {
        self.platform() == "darwin"
    }

    fn is_linux(&self) -> bool {
        self.platform() == "linux"
    }

    fn native_revision(&self) -> Option<u64> {
        self.state().driver_native_revision
    }

    fn focus_neutral_semantic_text(&self) -> bool {
        self.is_darwin() && self.native_revision() != Some(0)
    }

    fn exact_target_background_input(&self) -> bool {
        self.is_darwin() && self.native_revision().unwrap_or(0) >= 36
    }

    fn set_health(&self, update: impl FnOnce(&ComputerHealth) -> ComputerHealth) {
        let changed = {
            let mut state = self.state();
            let health = update(&state.health);
            if health == state.health {
                None
            } else {
                state.health = health.clone();
                Some(health)
            }
        };
        if let Some(health) = changed {
            self.listeners
                .emit(&ComputerBackendEvent::HealthChanged(health));
        }
    }

    fn mark_capture_failed(&self, error: &ComputerError) {
        self.state().capture_failed = true;
        let message = js_slice(&error.to_string(), 2048).to_owned();
        self.set_health(|current| ComputerHealth {
            status: ComputerHealthStatus::Unavailable,
            capture_available: false,
            consecutive_failures: current.consecutive_failures + 1,
            last_failure: Some(ComputerHealthFailure {
                at: now_iso(),
                message,
            }),
            ..current.clone()
        });
    }

    fn track_preview_task(&self, task: &CuaComputerTask) {
        let key = cua_computer_task_key(task);
        let mut state = self.state();
        state.preview_tasks.retain(|(existing, _)| *existing != key);
        state.preview_tasks.push((key.clone(), task.clone()));
        while state.preview_tasks.len() > PREVIEW_TASK_LIMIT {
            let Some(oldest) = state
                .preview_tasks
                .iter()
                .position(|(existing, _)| *existing != key)
            else {
                break;
            };
            state.preview_tasks.remove(oldest);
        }
    }

    fn observe_desktop_interruption(&self, reply: &CuaReply) {
        let Some(interruptions) = reply.desktop_interruptions else {
            return;
        };
        let changed = {
            let mut state = self.state();
            let changed = state
                .desktop_interruptions
                .is_some_and(|previous| previous != interruptions);
            state.desktop_interruptions = Some(interruptions);
            changed
        };
        if changed {
            self.listeners
                .emit(&ComputerBackendEvent::DesktopInterrupted {
                    pauses: reply.desktop_pauses.clone().unwrap_or_default(),
                });
        }
    }

    async fn request(
        &self,
        call: ComputerHostCall,
        signal: Option<OperationSignal>,
        mutation: bool,
        timeout: Duration,
    ) -> Result<CuaReply, CuaTransportError> {
        if signal.as_ref().is_some_and(OperationSignal::is_aborted) {
            return Err(CuaTransportError {
                message: "Cancelled before dispatch.".into(),
                effect: CuaEffect::NotDispatched,
            });
        }
        let encoded = serde_json::to_vec(&call).map(|bytes| bytes.len() + 1);
        if !matches!(encoded, Ok(bytes) if bytes <= CUA_MAX_REQUEST_BYTES) {
            return Err(CuaTransportError {
                message: "Invalid or oversized computer request.".into(),
                effect: CuaEffect::NotDispatched,
            });
        }
        let uncertain = if mutation {
            CuaEffect::DispatchedUnknown
        } else {
            CuaEffect::NotDispatched
        };
        let mut guard = CancelOnDrop {
            host: self.host.clone(),
            correlation_id: call.correlation_id.clone(),
            armed: true,
        };
        let aborted = async {
            match &signal {
                Some(signal) => signal.aborted().await,
                None => std::future::pending().await,
            }
        };
        let outcome = tokio::select! {
            reply = self.host.request(call) => {
                guard.armed = false;
                Ok(reply)
            }
            () = tokio::time::sleep(timeout) => {
                self.state().host_warm = false;
                Err(CuaTransportError {
                    message: if mutation {
                        "Computer request timed out; do not replay an uncertain action.".into()
                    } else {
                        "Computer read timed out; it changed nothing and is safe to retry once the app responds.".into()
                    },
                    effect: uncertain,
                })
            }
            () = aborted => Err(CuaTransportError {
                message: "Computer operation cancelled. Input already dispatched may have taken effect; do not replay.".into(),
                effect: uncertain,
            }),
        };
        if mutation {
            for (_, tree) in &mut self.state().recent_trees {
                tree.reusable = false;
            }
        }
        outcome
    }

    fn host_call(request: HostRequest) -> ComputerHostCall {
        ComputerHostCall {
            correlation_id: uuid::Uuid::new_v4().to_string(),
            method: request.method,
            name: request.name,
            args: request.args,
            task: request.task,
            delivery_mode: request.delivery_mode,
            model_observation: request.model_observation,
        }
    }

    async fn host(
        self: &Arc<Self>,
        mut request: HostRequest,
        mutation: bool,
        allow_model_observation: bool,
    ) -> Result<CuaReply, ComputerError> {
        if self.state().disposed {
            return Err(
                CuaActionError::new(GUI_HOST_REQUIRED, CuaEffect::NotDispatched)
                    .code("gui_host_required")
                    .into(),
            );
        }
        assert_desktop_operation_active()?;
        let is_call = request.method == ComputerHostMethod::Call;
        let task = if is_call {
            current_computer_task()
        } else {
            None
        };
        if let Some(task) = &task {
            self.track_preview_task(task);
        }
        let send_baseline = self.state().desktop_epoch;
        if let Some(timing) = current_computer_call()
            .as_ref()
            .and_then(|call| call.timing.as_ref())
        {
            timing.count("host_calls", 1);
        }
        if is_call {
            request.delivery_mode = Some(desktop_delivery_mode());
            if matches!(
                request.name.as_deref(),
                Some("get_window_state" | "get_desktop_state")
            ) {
                request.model_observation =
                    Some(allow_model_observation && is_model_desktop_observation_active());
            }
        }
        if task.is_some() {
            request.task = task;
        }
        let timeout = if request.method == ComputerHostMethod::Setup {
            CUA_SETUP_TIMEOUT_MS
        } else if self.state().host_warm {
            CUA_HOST_CALL_TIMEOUT_MS
        } else {
            CUA_COLD_HOST_CALL_TIMEOUT_MS
        };
        let call = Self::host_call(request);
        let reply = timed_computer_leg(
            "host",
            self.request(
                call,
                desktop_operation_signal(),
                mutation,
                Duration::from_millis(timeout),
            ),
        )
        .await
        .map_err(transport_error)?;
        self.observe_desktop_interruption(&reply);
        {
            let mut state = self.state();
            if let Some(revision) = reply.driver_native_revision {
                state.driver_native_revision = Some(revision);
            }
            if let Some(platform) = reply.host_platform.as_ref().filter(|p| !p.is_empty()) {
                state.host_platform = Some(platform.clone());
            }
            if let Some(epoch) = reply.desktop_epoch {
                let uncertain = if mutation {
                    CuaEffect::DispatchedUnknown
                } else {
                    CuaEffect::NotDispatched
                };
                let stale = || -> ComputerError {
                    CuaActionError::new(
                        "The desktop changed while this operation was in flight. Observe again before continuing.",
                        uncertain,
                    )
                    .code("stale_desktop_epoch")
                    .into()
                };
                if let (Some(baseline), Some(current)) = (send_baseline, state.desktop_epoch)
                    && current != baseline
                {
                    return Err(stale());
                }
                if send_baseline.is_some_and(|baseline| epoch < baseline) {
                    return Err(stale());
                }
                if state.desktop_epoch != Some(epoch) {
                    state.desktop_epoch = Some(epoch);
                    state.observed_geometry.clear();
                    state.snapshot_at = None;
                }
            }
        }
        if reply.ok {
            self.state().host_warm = true;
        }
        if !reply.ok {
            return Err(CuaActionError::new(
                reply
                    .error
                    .clone()
                    .unwrap_or_else(|| "Cua host failed.".into()),
                reply.effect.unwrap_or(CuaEffect::NotDispatched),
            )
            .into());
        }
        Ok(reply)
    }

    async fn call(
        self: &Arc<Self>,
        name: &str,
        args: Map<String, Value>,
        mutation: bool,
        allow_model_observation: bool,
    ) -> Result<CuaToolResult, ComputerError> {
        if let Some(timing) = current_computer_call()
            .as_ref()
            .and_then(|call| call.timing.as_ref())
        {
            timing.count("native_calls", 1);
        }
        let request = HostRequest {
            name: Some(name.to_owned()),
            args: Value::Object(args.clone()),
            ..HostRequest::new(ComputerHostMethod::Call)
        };
        let reply = timed_computer_leg(
            "call",
            self.host(request, mutation, allow_model_observation),
        )
        .await?;
        let result = reply.result.unwrap_or_default();
        let structured = result.structured_content.clone().unwrap_or_default();
        let structured_effect = structured.get("effect").and_then(Value::as_str);
        let structured_status = structured.get("status").and_then(Value::as_str);
        if result.is_error != Some(true)
            && structured_effect != Some("refused")
            && structured_status != Some("refused")
        {
            return Ok(result);
        }
        let refused = matches!(structured_effect, Some("refused" | "not-dispatched"))
            || (!structured.contains_key("effect") && structured_status == Some("refused"));
        let refusal = record(structured.get("refusal"));
        let joined = result
            .content
            .as_deref()
            .unwrap_or_default()
            .iter()
            .map(|content| content.text.as_deref().unwrap_or(""))
            .collect::<Vec<_>>()
            .join("\n");
        let joined = js_slice(&joined, 2048).to_owned();
        let mut message = [
            joined,
            text1024(structured.get("message")),
            text1024(refusal.get("message")),
            text1024(structured.get("reason")),
        ]
        .into_iter()
        .find(|candidate| !candidate.is_empty())
        .unwrap_or_else(|| "The native operation could not complete.".into());
        let native_code = [
            text1024(structured.get("code")),
            text1024(refusal.get("code")),
        ]
        .into_iter()
        .find(|candidate| !candidate.is_empty())
        .unwrap_or_else(|| {
            if refused {
                "cua_refusal".into()
            } else {
                "cua_action_failed".into()
            }
        });
        let code = if native_code == "desktop_input_paused" {
            "computer_input_paused".to_owned()
        } else {
            native_code.clone()
        };
        if code == "same_pid_keyboard_ambiguity" {
            message.push_str(" Another window of this app, often a dialog such as a page alert, can receive the keys. Read the state: if a dialog is open, close it by pressing its button by element (click(index), or computer_click with its ref), then retry. To write text, set the field by element (setValue(index, text), computer_set_value, or computer_type_text with an observed ref). These semantic writes do not send keydown/keyup events. Do not retry physical keys in the background.");
        }
        if refused
            && ["stale_element_token", "stale_geometry", "stale_target"].contains(&code.as_str())
        {
            message.push_str(" The original element or coordinate frame is no longer valid. Read fresh state and select the intended control again; do not substitute another same-label control or replay uncertain input.");
        }
        if refused && code == "computer_input_paused" {
            self.state().observed_geometry.clear();
        }
        let pid = args
            .get("pid")
            .and_then(Value::as_f64)
            .and_then(safe_integer);
        let window = args
            .get("window_id")
            .and_then(Value::as_f64)
            .and_then(safe_integer);
        let pausing = (refused
            && matches!(
                code.as_str(),
                "target_not_on_active_space" | "computer_input_paused" | "auth_sheet_focused"
            ))
            || code == "focus_restore_failed";
        let user_takeover = refused
            && code == "computer_input_paused"
            && structured.get("cause").and_then(Value::as_str) == Some("user_input");
        let input_pause = match (pausing, pid, window) {
            (true, Some(pid), Some(window)) => Some(ComputerInputPause {
                window_id: Some(format!("cua:{pid}:{window}")),
                pid: matches!(
                    code.as_str(),
                    "computer_input_paused" | "target_not_on_active_space" | "focus_restore_failed"
                )
                .then(|| u32::try_from(pid).ok())
                .flatten(),
                message: message.clone(),
                user_takeover,
            }),
            _ => None,
        };
        let layer = if structured.get("layer").and_then(Value::as_str) == Some("driver-host")
            || native_code == "desktop_input_paused"
        {
            CuaErrorLayer::DriverHost
        } else {
            CuaErrorLayer::NativeDriver
        };
        let wait_seconds = structured
            .get("wait_seconds")
            .and_then(Value::as_f64)
            .filter(|seconds| refused && code == "computer_input_paused" && seconds.is_finite())
            .map(|seconds| seconds.clamp(0.0, 60.0));
        let effect = if mutation && !refused {
            CuaEffect::DispatchedUnknown
        } else {
            CuaEffect::NotDispatched
        };
        Err(CuaActionError {
            message,
            effect,
            code,
            input_pause,
            diagnostics: parse_cua_action_diagnostics(&Value::Object(structured)),
            layer: Some(layer),
            wait_seconds,
        }
        .into())
    }

    async fn call_structured(
        self: &Arc<Self>,
        name: &str,
        args: Map<String, Value>,
        mutation: bool,
    ) -> Result<Map<String, Value>, ComputerError> {
        Ok(self
            .call(name, args, mutation, true)
            .await?
            .structured_content
            .unwrap_or_default())
    }

    async fn probe_availability(self: &Arc<Self>) -> ComputerAvailability {
        match self
            .host(HostRequest::new(ComputerHostMethod::Probe), false, true)
            .await
        {
            Ok(_) => {
                let state = self.state();
                let pristine = matches!(
                    state.availability,
                    ComputerAvailability::BackendUnavailable { .. }
                ) && state.snapshot_at.is_none()
                    && state.health.consecutive_failures == 0
                    && state.health.last_failure.is_none();
                if pristine {
                    ComputerAvailability::Available {
                        backend: Some("cua".into()),
                    }
                } else {
                    state.availability.clone()
                }
            }
            Err(error) => ComputerAvailability::BackendUnavailable {
                message: error_text(&error),
            },
        }
    }

    fn in_flight_snapshot(&self) -> Option<Snapshot> {
        self.state().snapshot.as_ref().map(|(_, s)| s.clone())
    }

    async fn availability(self: &Arc<Self>, refresh: bool) -> ComputerAvailability {
        if refresh && let Some(snapshot) = self.in_flight_snapshot() {
            let _ = snapshot.await;
        }
        let _ = self.refresh(refresh, false).await;
        self.state().availability.clone()
    }

    fn capabilities(&self) -> ComputerCapabilities {
        let native_input_available = !self.is_linux();
        ComputerCapabilities {
            windows: true,
            window_bounds: true,
            stacking: true,
            capture: true,
            input: native_input_available,
            clipboard: true,
            focus: native_input_available,
            raise: native_input_available,
            ghost_cursor: self.is_darwin() && self.native_revision() != Some(0),
            visible_desktop: true,
        }
    }

    async fn provision(self: &Arc<Self>) -> Result<String, ComputerError> {
        if let Some(snapshot) = self.in_flight_snapshot() {
            let _ = snapshot.await;
        }
        self.host(HostRequest::new(ComputerHostMethod::Setup), false, true)
            .await?;
        self.state().snapshot_at = None;
        self.refresh(true, false).await?;
        let (availability, permissions) = {
            let state = self.state();
            (state.availability.clone(), state.permissions.clone())
        };
        if let ComputerAvailability::BackendUnavailable { message } = availability {
            return Ok(message);
        }
        if permissions.is_empty() {
            return Ok("Computer permissions are ready. Send a message to continue; no action is retried automatically.".into());
        }
        let missing = list_computer_permissions(&permissions);
        Ok(if self.is_darwin() {
            format!(
                "Allow {missing} for this copy of Zeron in System Settings. Return here to check again; if macOS asks you to quit and reopen the app, do so."
            )
        } else {
            format!(
                "The driver host reports missing {missing} access. Grant it at the OS level the platform uses (display-server access on Linux, integrity/UIAccess on Windows), then check again; no action is retried automatically."
            )
        })
    }

    fn refresh(
        self: &Arc<Self>,
        force: bool,
        include_keyboard_focus: bool,
    ) -> BoxFuture<'static, Result<(), ComputerError>> {
        let snapshot = {
            let mut state = self.state();
            if let Some((_, snapshot)) = &state.snapshot {
                snapshot.clone()
            } else if !force
                && state
                    .snapshot_at
                    .is_some_and(|at| at.elapsed() < SNAPSHOT_FRESH_FOR)
            {
                return Box::pin(async { Ok(()) });
            } else {
                state.snapshot_generation += 1;
                let generation = state.snapshot_generation;
                let this = self.clone();
                let scope = CallScope::capture();
                let handle = tokio::spawn(scope.run(Box::pin(async move {
                    let outcome = this.take_snapshot(include_keyboard_focus).await;
                    if let Err(error) = &outcome {
                        let message = error_text(error);
                        this.state().availability = ComputerAvailability::BackendUnavailable {
                            message: message.clone(),
                        };
                        this.set_health(|current| ComputerHealth {
                            status: ComputerHealthStatus::Unavailable,
                            capture_available: false,
                            consecutive_failures: current.consecutive_failures + 1,
                            last_failure: Some(ComputerHealthFailure {
                                at: now_iso(),
                                message,
                            }),
                            ..current.clone()
                        });
                    }
                    let mut state = this.state();
                    if state
                        .snapshot
                        .as_ref()
                        .is_some_and(|(owner, _)| *owner == generation)
                    {
                        state.snapshot = None;
                    }
                    outcome
                })));
                let snapshot: Snapshot = async move {
                    handle.await.unwrap_or_else(|error| {
                        Err(ComputerError::Other(format!(
                            "Computer snapshot task failed: {error}"
                        )))
                    })
                }
                .boxed()
                .shared();
                state.snapshot = Some((generation, snapshot.clone()));
                snapshot
            }
        };
        Box::pin(snapshot)
    }

    async fn check_permissions(self: &Arc<Self>) -> Result<Map<String, Value>, ComputerError> {
        let mut args = Map::new();
        args.insert("prompt".into(), Value::Bool(false));
        self.call_structured("check_permissions", args, false).await
    }

    async fn take_snapshot(
        self: &Arc<Self>,
        include_keyboard_focus: bool,
    ) -> Result<(), ComputerError> {
        let mut permission = self.check_permissions().await?;
        let platform = self.platform();
        let had_missing = self.state().had_missing_permissions;
        if !missing_computer_permissions(&permission, &platform).is_empty() && !had_missing {
            for _ in 0..4 {
                tokio::time::sleep(PERMISSION_REPROBE_DELAY).await;
                permission = self.check_permissions().await?;
                if missing_computer_permissions(&permission, &platform).is_empty() {
                    break;
                }
            }
        }
        let permissions = missing_computer_permissions(&permission, &platform);
        let capture_failed = {
            let mut state = self.state();
            state.permissions = permissions.clone();
            state.had_missing_permissions = !permissions.is_empty();
            if platform != "linux" && permission.get("screen_recording") == Some(&Value::Bool(true))
            {
                state.capture_failed = false;
            }
            state.capture_failed
        };
        let bundle_id = text(record(permission.get("source")).get("host_bundle_id"), 256);
        let signature = ComputerBuildSignature::Unknown;
        let monitor_unavailable = platform == "darwin"
            && permission.get("input_monitoring") == Some(&Value::Bool(true))
            && permission.get("input_monitor_ready") == Some(&Value::Bool(false));
        let capture_available = capture_access_available(&permission, &platform) && !capture_failed;
        self.set_health(|current| ComputerHealth {
            status: if capture_failed || monitor_unavailable {
                ComputerHealthStatus::Unavailable
            } else {
                ComputerHealthStatus::Connected
            },
            capture_available,
            consecutive_failures: if capture_failed {
                current.consecutive_failures
            } else {
                0
            },
            last_failure: if monitor_unavailable {
                Some(ComputerHealthFailure {
                    at: now_iso(),
                    message: MONITOR_MESSAGE.into(),
                })
            } else {
                current.last_failure.clone()
            },
            ..current.clone()
        });
        let availability = if !permissions.is_empty() {
            let bundle = (!bundle_id.is_empty()).then(|| bundle_id.clone());
            ComputerAvailability::PermissionRequired {
                missing: permissions.clone(),
                message: if platform == "darwin" {
                    computer_permission_setup_message(&permissions, signature, bundle.as_deref())
                } else {
                    format!(
                        "Zeron's driver host reports missing {} access. Grant it at the OS level the platform uses — display-server access on Linux, integrity/UIAccess on Windows — then try again.",
                        list_computer_permissions(&permissions)
                    )
                },
                build_signature: signature,
                bundle_id: bundle,
            }
        } else if monitor_unavailable {
            ComputerAvailability::BackendUnavailable {
                message: MONITOR_MESSAGE.into(),
            }
        } else {
            ComputerAvailability::Available {
                backend: Some("cua".into()),
            }
        };
        let available = matches!(availability, ComputerAvailability::Available { .. });
        self.state().availability = availability;
        if available {
            self.read_windows(include_keyboard_focus).await?;
            let geometry = self
                .call_structured("get_screen_size", Map::new(), false)
                .await?;
            let width = number(geometry.get("width"));
            let height = number(geometry.get("height"));
            if !(width > 0.0 && height > 0.0) {
                return Err(ComputerError::Other(
                    "Cua returned no primary display geometry.".into(),
                ));
            }
            let scale = number(geometry.get("scale_factor"));
            self.state().size = ComputerScreenSize {
                width: width as u32,
                height: height as u32,
                scale: (scale.is_finite() && scale > 0.0).then_some(scale),
            };
        }
        self.state().snapshot_at = Some(Instant::now());
        Ok(())
    }

    async fn read_windows(
        self: &Arc<Self>,
        include_keyboard_focus: bool,
    ) -> Result<Vec<ComputerWindow>, ComputerError> {
        let mut args = Map::new();
        if self.is_darwin() && self.native_revision().unwrap_or(0) >= 37 && include_keyboard_focus {
            args.insert("include_keyboard_focus".into(), Value::Bool(true));
        }
        let data = self.call_structured("list_windows", args, false).await?;
        let Some(rows) = data.get("windows").and_then(Value::as_array) else {
            return Err(ComputerError::Other("Invalid Cua window list.".into()));
        };
        let mut rows: Vec<Map<String, Value>> = rows.iter().map(|row| record(Some(row))).collect();
        let z = |row: &Map<String, Value>| {
            let value = number(row.get("z_index"));
            if value.is_nan() { 0.0 } else { value }
        };
        rows.sort_by(|a, b| z(b).partial_cmp(&z(a)).unwrap_or(std::cmp::Ordering::Equal));
        let selected = self.state().selected_window.clone();
        let windows: Vec<ComputerWindow> = rows
            .iter()
            .enumerate()
            .filter_map(|(index, row)| {
                let pid = positive_id(number(row.get("pid")))?;
                let pid = u32::try_from(pid).ok()?;
                let window_id = positive_id(number(row.get("window_id")))?;
                let bounds = optional_rect(row.get("bounds"))?;
                let space_ids = row
                    .get("space_ids")
                    .and_then(Value::as_array)
                    .and_then(|ids| {
                        (ids.len() <= COMPUTER_WINDOW_LIST_MAX_LENGTH)
                            .then(|| {
                                ids.iter()
                                    .map(|id| id.as_f64().and_then(positive_id))
                                    .collect::<Option<Vec<u64>>>()
                            })
                            .flatten()
                    });
                let id = cua_window_id(u64::from(pid), window_id);
                let on_current_space = row.get("on_current_space").and_then(Value::as_bool);
                let on_screen = row.get("is_on_screen");
                let has_spaces = row
                    .get("space_ids")
                    .and_then(Value::as_array)
                    .is_some_and(|ids| !ids.is_empty());
                Some(ComputerWindow {
                    focused: selected.as_deref() == Some(id.as_str()),
                    id,
                    pid: Some(pid),
                    title: text1024(row.get("title")),
                    app_name: Some(text1024(row.get("app_name"))),
                    bounds: Some(bounds),
                    keyboard_focused: row.get("keyboard_focused").and_then(Value::as_bool),
                    active: None,
                    minimized: on_screen == Some(&Value::Bool(false))
                        && on_current_space != Some(false)
                        && has_spaces,
                    visible: on_screen == Some(&Value::Bool(true))
                        && on_current_space != Some(false),
                    space_ids,
                    current_space_id: positive_id(number(row.get("current_space_id"))),
                    on_current_space,
                    stacking_index: row
                        .get("z_index")
                        .and_then(Value::as_f64)
                        .is_some_and(|z| z.is_finite() && z.fract() == 0.0)
                        .then_some(index as u64),
                    occluded_by: None,
                })
            })
            .take(512)
            .collect();
        self.state().windows = windows.clone();
        Ok(windows)
    }

    async fn list_spaces(self: &Arc<Self>) -> Result<ComputerSpaceInventory, ComputerError> {
        match self.call_structured("list_spaces", Map::new(), false).await {
            Ok(result) => Ok(cua_space_inventory(&result)?),
            Err(error) => {
                assert_desktop_operation_active()?;
                Err(ComputerTargetError::space(
                    ComputerSpaceErrorCode::ComputerSpacesUnavailable,
                    format!(
                        "Managed Space inventory is unavailable: {error}. Drive an exact existing window in place instead."
                    ),
                )
                .into())
            }
        }
    }

    async fn target(
        self: &Arc<Self>,
        window_id: Option<&str>,
        fresh: bool,
    ) -> Result<ResolvedWindow, ComputerError> {
        let window_id = match window_id {
            Some(id) => Some(id.to_owned()),
            None => self.state().selected_window.clone(),
        };
        let Some((_, native_window)) = window_id.as_deref().and_then(parse_cua_window_id) else {
            return Err(CuaActionError::new(
                "Select an exact window before acting.",
                CuaEffect::NotDispatched,
            )
            .code("window_required")
            .into());
        };
        let window_id = window_id.unwrap_or_default();
        let windows = if fresh {
            self.read_windows(false).await?
        } else {
            self.state().windows.clone()
        };
        let window = windows
            .into_iter()
            .find(|window| window.id == window_id)
            .filter(|window| window.bounds.is_some() && window.pid.is_some());
        let Some(window) = window else {
            return Err(CuaActionError::new(
                "The target window closed or its identity changed.",
                CuaEffect::NotDispatched,
            )
            .code("stale_target")
            .into());
        };
        Ok(ResolvedWindow {
            pid: u64::from(window.pid.unwrap_or_default()),
            window_id: native_window,
            window,
            baseline: self.state().desktop_epoch,
        })
    }

    fn window_args(pid: u64, window_id: u64) -> Map<String, Value> {
        let mut args = Map::new();
        args.insert("pid".into(), json!(pid));
        args.insert("window_id".into(), json!(window_id));
        args
    }

    async fn focus_window(self: &Arc<Self>, window_id: &str) -> Result<(), ComputerError> {
        let known = self
            .state()
            .windows
            .iter()
            .any(|window| window.id == window_id);
        self.target(Some(window_id), !known).await?;
        let mut state = self.state();
        state.selected_window = Some(window_id.to_owned());
        state.still_target = Some(StillTarget::Window {
            window_id: window_id.to_owned(),
        });
        Ok(())
    }

    async fn check_input_ready(self: &Arc<Self>, window_id: &str) -> Result<(), ComputerError> {
        let resolved = self.target(Some(window_id), true).await?;
        if self.state().host_platform.as_deref() == Some("linux") {
            if !resolved.window.visible {
                return Err(CuaActionError::new(
                    "The Linux target is not visible in the current desktop session.",
                    CuaEffect::NotDispatched,
                )
                .code("target_not_on_active_space")
                .into());
            }
            return Ok(());
        }
        let data = self
            .call_structured(
                "check_input_ready",
                Self::window_args(resolved.pid, resolved.window_id),
                false,
            )
            .await?;
        if data.get("ready") != Some(&Value::Bool(true))
            || number(data.get("pid")) != resolved.pid as f64
            || number(data.get("window_id")) != resolved.window_id as f64
        {
            return Err(CuaActionError::new(
                "Cua did not confirm input readiness for the exact target window.",
                CuaEffect::NotDispatched,
            )
            .code("invalid_readiness")
            .into());
        }
        Ok(())
    }

    async fn wait_for_settle(
        self: &Arc<Self>,
        options: ComputerSettleOptions,
    ) -> Result<ComputerSettleOutcome, ComputerError> {
        let resolved = self.target(Some(&options.window_id), true).await?;
        let mut args = Self::window_args(resolved.pid, resolved.window_id);
        args.insert(
            "timeout_ms".into(),
            num(options.timeout_ms.floor().clamp(0.0, 30_000.0)),
        );
        args.insert(
            "quiet_ms".into(),
            num(options.quiet_ms.floor().clamp(0.0, 5_000.0)),
        );
        let data = self.call_structured("wait_for_settle", args, false).await?;
        let Some(settled) = data.get("settled").and_then(Value::as_bool) else {
            return Err(CuaActionError::new(
                "Cua did not return a settle verdict for the exact target window.",
                CuaEffect::NotDispatched,
            )
            .code("invalid_settle_read")
            .into());
        };
        let waited = number(data.get("waited_ms"));
        Ok(ComputerSettleOutcome {
            settled,
            waited_ms: if waited.is_finite() { waited } else { 0.0 },
            events_seen: data.get("events_seen").and_then(Value::as_f64),
        })
    }

    async fn raise_window(self: &Arc<Self>, window_id: &str) -> Result<(), ComputerError> {
        if desktop_delivery_mode() != ComputerDeliveryMode::Foreground {
            return Err(CuaActionError::new(
                "Window activation requires foreground delivery within an authorized Computer task.",
                CuaEffect::NotDispatched,
            )
            .code("foreground_required")
            .into());
        }
        let resolved = self.target(Some(window_id), true).await?;
        let data = self
            .call_structured(
                "bring_to_front",
                Self::window_args(resolved.pid, resolved.window_id),
                true,
            )
            .await?;
        if data.get("activated") != Some(&Value::Bool(true)) {
            return Err(CuaActionError::new(
                "Cua could not verify that the exact window became foreground. Do not repeat the activation blindly.",
                CuaEffect::DispatchedUnknown,
            )
            .into());
        }
        self.state().snapshot_at = None;
        Ok(())
    }

    fn clear_focus_window(&self) {
        let mut state = self.state();
        state.selected_window = None;
        state.still_target = None;
    }

    fn screenshot(
        &self,
        result: &CuaToolResult,
        fallback: Option<ComputerRect>,
    ) -> Result<ComputerScreenshot, ComputerError> {
        let data = result.structured_content.clone().unwrap_or_default();
        match data
            .get("screenshot_frame_freshness")
            .and_then(Value::as_str)
        {
            Some("unverified_off_space") => {
                return Err(CuaActionError::new(
                    "The exact window is on another macOS Space. Cua returned pixels, but their freshness cannot be proven without switching Spaces, so Zeron will not present them as a live observation.",
                    CuaEffect::NotDispatched,
                )
                .code("off_space_capture_unverified")
                .into());
            }
            Some("unverified_not_on_screen") => {
                return Err(CuaActionError::new(
                    "The exact window is minimized or its app is hidden, so macOS returns retained pixels that are not a live observation. Read it through get_state's accessibility tree, or ask before restoring the window.",
                    CuaEffect::NotDispatched,
                )
                .code("hidden_capture_unverified")
                .into());
            }
            _ => {}
        }
        let image = result
            .content
            .as_deref()
            .unwrap_or_default()
            .iter()
            .find(|content| {
                content.kind == "image"
                    && content.mime_type.as_deref() == Some("image/png")
                    && content.data.as_deref().is_some_and(|data| !data.is_empty())
            });
        let image_data = image.and_then(|image| image.data.clone());
        let Some(image_data) =
            image_data.filter(|_| data.get("screenshot_frame_valid") != Some(&Value::Bool(false)))
        else {
            return Err(CuaActionError::new(
                "Cua could not establish the screenshot geometry.",
                CuaEffect::NotDispatched,
            )
            .code("capture_unavailable")
            .into());
        };
        let dimensions = png_dimensions(&decode_base64_prefix(&image_data, 32));
        let region = match data.get("window_bounds") {
            Some(value) if !value.is_null() && value != &Value::Bool(false) => {
                Some(rect(Some(value))?)
            }
            _ => fallback,
        };
        let (Some(dimensions), Some(region)) = (dimensions, region) else {
            return Err(ComputerError::Other(
                "Cua screenshot is missing its coordinate frame.".into(),
            ));
        };
        let scale = f64::from(dimensions.width) / region.width;
        if (f64::from(dimensions.height) / region.height - scale).abs() > 0.01 {
            return Err(ComputerError::Other(
                "Cua screenshot dimensions disagree with its geometry.".into(),
            ));
        }
        if self.is_linux() && self.state().capture_failed {
            self.state().capture_failed = false;
            self.set_health(|current| ComputerHealth {
                status: ComputerHealthStatus::Connected,
                capture_available: true,
                consecutive_failures: 0,
                ..current.clone()
            });
        }
        Ok(ComputerScreenshot {
            mime_type: ComputerPngMimeType::Png,
            width: dimensions.width,
            height: dimensions.height,
            size_bytes: base64_byte_length(&image_data),
            bytes_base64: image_data,
            window_id: None,
            region: Some(region),
            scale: Some(scale),
            captured_at: now_iso(),
        })
    }

    async fn capture_overview(
        self: &Arc<Self>,
        allow_model_observation: bool,
    ) -> Result<ComputerScreenshot, ComputerError> {
        let outcome = async {
            let result = self
                .call(
                    "get_desktop_state",
                    Map::new(),
                    false,
                    allow_model_observation,
                )
                .await?;
            let data = result.structured_content.clone().unwrap_or_default();
            let image = self.screenshot(
                &result,
                Some(ComputerRect {
                    x: 0.0,
                    y: 0.0,
                    width: number(data.get("screen_width")),
                    height: number(data.get("screen_height")),
                }),
            )?;
            self.set_health(|current| ComputerHealth {
                status: ComputerHealthStatus::Connected,
                capture_available: true,
                consecutive_failures: 0,
                ..current.clone()
            });
            Ok(image)
        }
        .await;
        if let Err(error) = &outcome {
            self.mark_capture_failed(error);
        }
        outcome
    }

    fn assert_observed_window(
        result: &CuaToolResult,
        pid: u64,
        window_id: u64,
    ) -> Result<(), ComputerError> {
        let data = result.structured_content.clone().unwrap_or_default();
        if number(data.get("pid")) != pid as f64
            || number(data.get("window_id")) != window_id as f64
        {
            return Err(CuaActionError::new(
                "Cua observation belongs to a different window.",
                CuaEffect::NotDispatched,
            )
            .into());
        }
        Ok(())
    }

    async fn capture_screenshot(
        self: &Arc<Self>,
        request: ComputerCaptureRequest,
    ) -> Result<ComputerScreenshot, ComputerError> {
        let (window_id, max_dimension) = match request {
            ComputerCaptureRequest::Region { .. } => {
                return Err(CuaActionError::new(
                    "Region capture is not supported by this pinned Cua backend. Capture an exact window; the overview covers the primary display only.",
                    CuaEffect::NotDispatched,
                )
                .code("unsupported_operation")
                .into());
            }
            ComputerCaptureRequest::Window {
                window_id,
                max_dimension,
            } => (window_id, max_dimension),
        };
        let resolved = self.target(Some(&window_id), true).await?;
        let outcome = async {
            let mut args = Self::window_args(resolved.pid, resolved.window_id);
            args.insert("include_accessibility_tree".into(), Value::Bool(false));
            args.insert("include_screenshot".into(), Value::Bool(true));
            args.insert("max_dimension".into(), json!(max_dimension.unwrap_or(1536)));
            let result = self.call("get_window_state", args, false, true).await?;
            Self::assert_observed_window(&result, resolved.pid, resolved.window_id)?;
            let mut image = self.screenshot(&result, None)?;
            image.window_id = Some(resolved.window.id.clone());
            if let Some(region) = image.region {
                self.state()
                    .observed_geometry
                    .insert(resolved.window.id.clone(), region);
            }
            Ok::<_, ComputerError>(image)
        }
        .await;
        if let Err(error) = &outcome
            && !is_retained_capture(error)
        {
            self.mark_capture_failed(error);
        }
        outcome
    }

    async fn get_state(
        self: &Arc<Self>,
        options: ComputerStateOptions,
    ) -> Result<ComputerState, ComputerError> {
        self.refresh(
            false,
            !options.reuse_recent_tree && is_model_desktop_observation_active(),
        )
        .await?;
        let (windows, size, availability, has_missing) = {
            let state = self.state();
            (
                state.windows.clone(),
                state.size,
                state.availability.clone(),
                !state.permissions.is_empty(),
            )
        };
        let mut state = ComputerState {
            input_pause: None,
            preview_note: None,
            accessibility: None,
            computer_id: DEFAULT_COMPUTER_ID.to_owned(),
            windows: windows.clone(),
            screen_size: size,
            availability: Some(availability),
            root: None,
            text: None,
            screenshot: None,
            captured_at: now_iso(),
            ax_tree: None,
        };
        if has_missing {
            return Ok(state);
        }
        let Some(window_id) = options.window_id.clone() else {
            if options.include_screenshot {
                state.screenshot = Some(self.capture_overview(true).await?);
            }
            state.accessibility = Some(ComputerAccessibilityReport {
                status: ComputerAccessibilityStatus::Partial,
                unavailable_window_ids: windows.iter().map(|window| window.id.clone()).collect(),
            });
            return Ok(state);
        };
        let resolved = self.target(Some(&window_id), false).await?;
        state.windows = vec![resolved.window.clone()];
        if !options.include_tree && !options.include_screenshot {
            return Ok(state);
        }
        if options.include_tree && options.reuse_recent_tree && !options.include_screenshot {
            let cached = {
                let state = self.state();
                state
                    .recent_trees
                    .iter()
                    .find(|(id, _)| *id == window_id)
                    .filter(|(_, tree)| tree.reusable && tree.at.elapsed() < RECENT_TREE_TTL)
                    .map(|(_, tree)| tree.root.clone())
            };
            if let Some(root) = cached {
                state.root = Some(root);
                state.accessibility = Some(ComputerAccessibilityReport {
                    status: ComputerAccessibilityStatus::Partial,
                    unavailable_window_ids: Vec::new(),
                });
                return Ok(state);
            }
        }
        let wants_pixels = options.include_screenshot;
        let mut args = Self::window_args(resolved.pid, resolved.window_id);
        args.insert("include_screenshot".into(), Value::Bool(wants_pixels));
        args.insert("max_dimension".into(), json!(1536));
        args.insert(
            "include_accessibility_tree".into(),
            Value::Bool(options.include_tree),
        );
        args.insert("max_elements".into(), json!(CUA_WALK_MAX_ELEMENTS));
        args.insert("max_depth".into(), json!(25));
        let result = match async {
            let result = self.call("get_window_state", args, false, true).await?;
            Self::assert_observed_window(&result, resolved.pid, resolved.window_id)?;
            Ok::<_, ComputerError>(result)
        }
        .await
        {
            Ok(result) => result,
            Err(error) => {
                if wants_pixels {
                    self.mark_capture_failed(&error);
                }
                return Err(error);
            }
        };
        let data = result.structured_content.clone().unwrap_or_default();
        let window = &resolved.window;
        let mut children = Vec::new();
        let mut elements = Vec::new();
        let mut targets = HashMap::new();
        let indexed = elements_by_index(&data);
        let tree_nodes = data.get("tree_nodes").and_then(Value::as_array);
        // A driver without tree_nodes still numbers its actionable elements.
        let nodes = tree_nodes
            .or_else(|| data.get("elements").and_then(Value::as_array))
            .map_or(&[][..], Vec::as_slice);
        let (mut capped, mut actionable) = (false, 0);
        for (position, node) in nodes.iter().enumerate() {
            let element = match tree_nodes {
                Some(_) => node
                    .get("element_index")
                    .and_then(Value::as_u64)
                    .and_then(|index| indexed.get(&index).copied()),
                None => Some(node),
            };
            let field = |name: &str| element.and_then(|element| element.get(name));
            let token = field("element_token").is_some();
            if (token && actionable == CUA_TREE_ELEMENT_LIMIT)
                || (!token && children.len() >= CUA_TREE_ELEMENT_LIMIT)
            {
                capped = true;
                continue;
            }
            let Some(frame) =
                optional_rect(node.get("frame")).or_else(|| optional_rect(field("frame")))
            else {
                continue;
            };
            targets.insert(position, children.len() as u32);
            actionable += usize::from(token);
            let label = node_label(nodes, position, element);
            let description = text1024(field("value_description"));
            let role = text(node.get("role"), 128);
            // Only a settable AXComboBox accepts text; a read-only one is a select.
            let editable =
                (role == "AXComboBox").then(|| node.get("settable") == Some(&Value::Bool(true)));
            children.push(ComputerUiNode {
                role,
                label: (!label.is_empty()).then_some(label),
                value: field("value")
                    .or_else(|| node.get("value"))
                    .and_then(Value::as_str)
                    .map(|value| js_slice(value, 16384).to_owned()),
                description: (!description.is_empty()).then_some(description),
                frame,
                activation_point: Some(ComputerPoint {
                    x: frame.x + frame.width / 2.0,
                    y: frame.y + frame.height / 2.0,
                }),
                on_screen: window.visible,
                window_id: Some(window.id.clone()),
                accessibility_root: None,
                node_path: Some(vec![children.len() as u32]),
                editable,
                truncated: None,
                children: Vec::new(),
            });
            elements.push(field("element_token").and_then(Value::as_str).map(|token| {
                NativeElement {
                    token: token.to_owned(),
                    actions: field("actions").and_then(Value::as_array).map(|actions| {
                        Arc::new(
                            actions
                                .iter()
                                .filter_map(Value::as_str)
                                .map(str::to_owned)
                                .collect(),
                        )
                    }),
                    in_web_content: field("in_web_content") == Some(&Value::Bool(true)),
                }
            }));
        }
        let root = ComputerUiNode {
            role: "AXWindow".into(),
            label: Some(window.title.clone()),
            value: None,
            description: None,
            frame: window.bounds.unwrap_or(ComputerRect {
                x: 0.0,
                y: 0.0,
                width: 0.0,
                height: 0.0,
            }),
            activation_point: None,
            on_screen: window.visible,
            window_id: Some(window.id.clone()),
            accessibility_root: None,
            node_path: None,
            editable: None,
            truncated: Some(capped || data.get("elements_complete") != Some(&Value::Bool(true))),
            children,
        };
        self.remember_tree(&window_id, root.clone(), elements);
        state.ax_tree = cua_ax_tree(&data, &targets);
        if options.include_screenshot {
            match self.preview_image(&result, &window.id)? {
                Ok(screenshot) => {
                    if let Some(region) = screenshot.region {
                        self.state()
                            .observed_geometry
                            .insert(window.id.clone(), region);
                    }
                    state.screenshot = Some(screenshot);
                }
                Err(note) => state.preview_note = Some(note),
            }
        }
        state.root = Some(root);
        state.accessibility = Some(ComputerAccessibilityReport {
            status: ComputerAccessibilityStatus::Partial,
            unavailable_window_ids: Vec::new(),
        });
        Ok(state)
    }

    fn remember_tree(
        &self,
        window_id: &str,
        root: ComputerUiNode,
        elements: Vec<Option<NativeElement>>,
    ) {
        let mut state = self.state();
        for element in elements.iter().flatten() {
            if state
                .native_elements
                .insert(element.token.clone(), element.clone())
                .is_none()
            {
                state.native_element_order.push_back(element.token.clone());
            }
        }
        while state.native_element_order.len() > NATIVE_ELEMENT_LIMIT {
            if let Some(token) = state.native_element_order.pop_front() {
                state.native_elements.remove(&token);
            }
        }
        state.recent_trees.retain(|(id, _)| id != window_id);
        state.recent_trees.push_back((
            window_id.to_owned(),
            RecentTree {
                at: Instant::now(),
                reusable: true,
                root,
                elements,
            },
        ));
        while state.recent_trees.len() > RECENT_TREE_LIMIT {
            state.recent_trees.pop_front();
        }
    }

    fn node_element(&self, node: &ComputerUiNode) -> Option<NativeElement> {
        let window_id = node.window_id.as_deref()?;
        let [index] = node.node_path.as_deref()? else {
            return None;
        };
        let index = *index as usize;
        let state = self.state();
        let (_, tree) = state.recent_trees.iter().find(|(id, _)| id == window_id)?;
        let stored = tree.root.children.get(index)?;
        let same = ComputerUiNode {
            children: Vec::new(),
            ..stored.clone()
        } == ComputerUiNode {
            children: Vec::new(),
            ..node.clone()
        };
        if !same {
            return None;
        }
        tree.elements.get(index)?.clone()
    }

    fn native_element(&self, target: &ComputerResolvedTarget) -> Option<NativeElement> {
        if let Some(identity) = observed_computer_target_identity(&target.target) {
            let state = self.state();
            return Some(
                state
                    .native_elements
                    .get(identity)
                    .cloned()
                    .unwrap_or_else(|| NativeElement {
                        token: identity.to_owned(),
                        ..NativeElement::default()
                    }),
            );
        }
        self.node_element(&target.node)
    }

    fn preview_image(
        &self,
        result: &CuaToolResult,
        window_id: &str,
    ) -> Result<Result<ComputerScreenshot, String>, ComputerError> {
        match self.screenshot(result, None) {
            Ok(mut screenshot) => {
                screenshot.window_id = Some(window_id.to_owned());
                Ok(Ok(screenshot))
            }
            Err(error) => match error.code() {
                Some("off_space_capture_unverified") => Ok(Err("This window is on another macOS Space. Its preview is paused because frame freshness cannot be proven without switching Spaces; exact retained semantic text may still continue.".into())),
                Some("hidden_capture_unverified") => Ok(Err("This window is minimized or its app is hidden. Its preview is paused because macOS only returns retained pixels; the accessibility tree is still live.".into())),
                Some("capture_unavailable") => {
                    self.mark_capture_failed(&error);
                    Ok(Err("The preview for this window failed; input is unaffected. Reselect the window to resume.".into()))
                }
                _ => Err(error),
            },
        }
    }

    async fn input(
        self: &Arc<Self>,
        name: &str,
        args: Map<String, Value>,
        window_id: Option<&str>,
        point: Option<ComputerPoint>,
        prepared_bounds: Option<ComputerRect>,
    ) -> Result<ComputerBackendActionResult, ComputerError> {
        let resolved = self.target(window_id, true).await?;
        let foreground = desktop_delivery_mode() == ComputerDeliveryMode::Foreground;
        let semantic_lane_write = name == "select_text"
            || (name == "set_value"
                && (args.contains_key("element_token") || args.contains_key("element_index")))
            || (name == "type_text"
                && args.get("semantic_only") == Some(&Value::Bool(true))
                && !foreground
                && point.is_none());
        if semantic_lane_write {
            let this = self.clone();
            let name = name.to_owned();
            let window_id = window_id.map(str::to_owned);
            let pid = resolved.pid;
            let native_window = resolved.window_id;
            return self
                .semantic_text_in_lane(
                    pid,
                    native_window,
                    Box::new(move |admission| {
                        Box::pin(async move {
                            this.input_dispatch(
                                &name,
                                args,
                                window_id.as_deref(),
                                point,
                                prepared_bounds,
                                true,
                                resolved,
                                Some(&admission),
                            )
                            .await
                        })
                    }),
                )
                .await;
        }
        let exact_semantic_action = self.exact_target_background_input()
            && name == "click"
            && args.get("element_token").is_some_and(Value::is_string)
            && args.get("action").is_some_and(Value::is_string)
            && point.is_none()
            && !foreground;
        self.input_dispatch(
            name,
            args,
            window_id,
            point,
            prepared_bounds,
            exact_semantic_action,
            resolved,
            None,
        )
        .await
    }

    async fn semantic_text_in_lane(
        self: &Arc<Self>,
        pid: u64,
        window_id: u64,
        write: LaneWrite,
    ) -> Result<ComputerBackendActionResult, ComputerError> {
        let key = format!("semantic-text:{pid}:{window_id}");
        let signal = desktop_operation_signal();
        if let Some(signal) = &signal {
            signal.check()?;
        }
        let (release, released) = tokio::sync::oneshot::channel::<()>();
        let drained: LaneTail = async move {
            let _ = released.await;
        }
        .boxed()
        .shared();
        let predecessor = self
            .lanes
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(key.clone(), drained.clone());
        let started = Instant::now();
        let admission = LaneAdmission {
            abandoned: Arc::new(AtomicBool::new(false)),
            dispatched: Arc::new(AtomicBool::new(false)),
            deadline: started + self.lane_hold,
            signal: signal.clone(),
        };
        let this = self.clone();
        let task_admission = admission.clone();
        let scope = CallScope::capture();
        let gap = self.lane_gap;
        let mut handle = tokio::spawn(scope.run(Box::pin(async move {
            if let Some(predecessor) = predecessor {
                predecessor.await;
            }
            let outcome = async {
                task_admission.check()?;
                let lane_wait = started.elapsed();
                let delivery_started = Instant::now();
                let result = write(task_admission.clone()).await?;
                tracing::debug!(
                    pid,
                    window_id = %cua_window_id(pid, window_id),
                    lane_wait_ms = lane_wait.as_millis() as u64,
                    delivery_ms = delivery_started.elapsed().as_millis() as u64,
                    verified = ?result.verified,
                    effect = ?result.effect,
                    "[computer] semantic text lane write"
                );
                Ok(result)
            }
            .await;
            if task_admission.dispatched.load(Ordering::SeqCst) {
                tokio::time::sleep(gap).await;
            }
            {
                let mut lanes = this.lanes.lock().unwrap_or_else(PoisonError::into_inner);
                if lanes.get(&key).is_some_and(|tail| tail.ptr_eq(&drained)) {
                    lanes.remove(&key);
                }
            }
            let _ = release.send(());
            outcome
        })));
        let aborted = async {
            match &signal {
                Some(signal) => signal.aborted().await,
                None => std::future::pending().await,
            }
        };
        tokio::select! {
            biased;
            () = aborted => {
                admission.abandoned.store(true, Ordering::SeqCst);
                Err(super::desktop_operation_queue::DesktopOperationError::Aborted.into())
            }
            joined = &mut handle => joined.unwrap_or_else(|error| {
                Err(ComputerError::Other(format!("Semantic text lane task failed: {error}")))
            }),
            () = tokio::time::sleep_until(admission.deadline) => {
                admission.abandoned.store(true, Ordering::SeqCst);
                let dispatched = admission.dispatched.load(Ordering::SeqCst);
                Err(CuaActionError::new(
                    if dispatched {
                        "Semantic text delivery timed out; the write may have partially dispatched. Observe the target before acting; never retype blindly."
                    } else {
                        "Semantic text timed out before delivery; nothing was sent."
                    },
                    if dispatched { CuaEffect::DispatchedUnknown } else { CuaEffect::NotDispatched },
                )
                .into())
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    async fn input_dispatch(
        self: &Arc<Self>,
        name: &str,
        args: Map<String, Value>,
        _window_id: Option<&str>,
        point: Option<ComputerPoint>,
        prepared_bounds: Option<ComputerRect>,
        exact_semantic_target: bool,
        resolved: ResolvedWindow,
        admission: Option<&LaneAdmission>,
    ) -> Result<ComputerBackendActionResult, ComputerError> {
        let ResolvedWindow {
            pid,
            window_id,
            window,
            baseline,
        } = resolved;
        let linux = self.is_linux();
        let delivery_mode = desktop_delivery_mode();
        let mut native_args = args.clone();
        if linux {
            if name == "set_value"
                || name == "select_text"
                || args.get("semantic_only") == Some(&Value::Bool(true))
                || args.contains_key("element_token")
                || args.contains_key("element_index")
            {
                return Err(CuaActionError::new(
                    "This Linux route cannot preserve the observed semantic element's exact identity. Native input is unavailable until cancellation cleanup is supported; use observation or an existing debuggable browser when appropriate.",
                    CuaEffect::NotDispatched,
                )
                .code("linux_semantic_target_unproven")
                .into());
            }
            if delivery_mode != ComputerDeliveryMode::Foreground {
                return Err(CuaActionError::new(
                    "This Linux native route cannot guarantee background input without moving desktop focus or the human pointer. Foreground input is also unavailable until cancellation cleanup is supported; use observation or an existing debuggable browser when appropriate.",
                    CuaEffect::NotDispatched,
                )
                .code("linux_background_unavailable")
                .into());
            }
            if args.contains_key("action") {
                return Err(CuaActionError::new(
                    "This Linux driver does not implement the requested accessibility action.",
                    CuaEffect::NotDispatched,
                )
                .code("unsupported_linux_operation")
                .into());
            }
            native_args.remove("force_synthetic");
            native_args.remove("coordinate_space");
            native_args.remove("expected_window_bounds");
            if name == "scroll" {
                let dx = native_args
                    .get("delta_x")
                    .and_then(Value::as_f64)
                    .unwrap_or(0.0);
                let dy = native_args
                    .get("delta_y")
                    .and_then(Value::as_f64)
                    .unwrap_or(0.0);
                let modified = native_args
                    .get("modifiers")
                    .and_then(Value::as_array)
                    .is_some_and(|modifiers| !modifiers.is_empty());
                if (dx != 0.0 && dy != 0.0) || modified {
                    return Err(CuaActionError::new(
                        "This Linux driver supports one unmodified scroll axis per gesture. Diagonal and modified scroll gestures are unavailable.",
                        CuaEffect::NotDispatched,
                    )
                    .code("unsupported_linux_operation")
                    .into());
                }
                if dx != 0.0 || dy != 0.0 {
                    let amount = if dx != 0.0 { dx } else { dy };
                    native_args.insert("amount".into(), num(amount.abs()));
                    native_args.insert("by".into(), json!("line"));
                }
                native_args.remove("delta_x");
                native_args.remove("delta_y");
                native_args.remove("modifiers");
            }
        }
        if !window.visible && !exact_semantic_target {
            let message = "The target window is not on the current Space or not on screen. Only exact retained semantic text or advertised AX actions may operate there without activation; pointer and synthetic keyboard input require an available window and fresh state.";
            return Err(CuaActionError::new(message, CuaEffect::NotDispatched)
                .code("target_not_on_active_space")
                .input_pause(Some(ComputerInputPause {
                    window_id: Some(window.id.clone()),
                    pid: None,
                    message: message.into(),
                    user_takeover: false,
                }))
                .into());
        }
        let bounds = window.bounds.unwrap_or(ComputerRect {
            x: 0.0,
            y: 0.0,
            width: 0.0,
            height: 0.0,
        });
        if let Some(prepared) = prepared_bounds
            && !same_rect(&prepared, &bounds)
        {
            return Err(CuaActionError::new(
                "The target window moved since this target was observed; obtain a new screenshot.",
                CuaEffect::NotDispatched,
            )
            .code("stale_geometry")
            .into());
        }
        let (observed, current_epoch) = {
            let state = self.state();
            (
                state.observed_geometry.get(&window.id).copied(),
                state.desktop_epoch,
            )
        };
        if point.is_some()
            && prepared_bounds.is_none()
            && !observed.is_some_and(|observed| same_rect(&observed, &bounds))
        {
            return Err(CuaActionError::new(
                "Window geometry changed since observation; obtain a new screenshot.",
                CuaEffect::NotDispatched,
            )
            .code("stale_geometry")
            .into());
        }
        if let (Some(baseline), Some(current)) = (baseline, current_epoch)
            && baseline != current
        {
            return Err(CuaActionError::new(
                "The desktop changed after this target was resolved. Observe again before continuing.",
                CuaEffect::NotDispatched,
            )
            .code("stale_desktop_epoch")
            .into());
        }
        let mut pixel = Map::new();
        if let Some(point) = point {
            let x = point.x - bounds.x;
            let y = point.y - bounds.y;
            if !x.is_finite()
                || !y.is_finite()
                || x < 0.0
                || y < 0.0
                || x >= bounds.width
                || y >= bounds.height
            {
                return Err(CuaActionError::new(
                    "Point is outside the target window.",
                    CuaEffect::NotDispatched,
                )
                .into());
            }
            pixel.insert("x".into(), num(x));
            pixel.insert("y".into(), num(y));
            if !linux {
                pixel.insert("coordinate_space".into(), json!("window_points"));
            }
        }
        assert_desktop_operation_active()?;
        if let Some(admission) = admission {
            admission.admit()?;
        }
        let mut call_args = Self::window_args(pid, window_id);
        if name != "set_value" && name != "select_text" {
            call_args.insert(
                "delivery_mode".into(),
                json!(match delivery_mode {
                    ComputerDeliveryMode::Background => "background",
                    ComputerDeliveryMode::Foreground => "foreground",
                }),
            );
        }
        call_args.extend(native_args);
        call_args.extend(pixel);
        if !linux && (point.is_some() || prepared_bounds.is_some()) {
            call_args.insert(
                "expected_window_bounds".into(),
                rect_json(&prepared_bounds.unwrap_or(bounds)),
            );
        }
        let outcome = self.call(name, call_args, true, true).await;
        self.state().snapshot_at = None;
        let result = match outcome {
            Ok(result) => result,
            Err(error) => {
                if desktop_operation_signal().is_some_and(|signal| signal.is_aborted())
                    || error.effect() == Some(CuaEffect::DispatchedUnknown)
                {
                    self.state().observed_geometry.clear();
                }
                return Err(error);
            }
        };
        let data = result.structured_content.unwrap_or_default();
        let kinds = evidence_kinds(&data);
        let confirmed = effect_str(&data) == Some("confirmed")
            && kinds
                .iter()
                .any(|kind| kind == "value_readback" || kind == "window_change");
        let mode = text(record(data.get("delivery")).get("mode"), 32);
        let mode = if mode.is_empty() {
            "unknown".into()
        } else {
            mode
        };
        let route = match text(data.get("route"), 64) {
            route if !route.is_empty() => route,
            _ => text(data.get("path"), 64),
        };
        Ok(ComputerBackendActionResult {
            window_id: Some(window.id.clone()),
            point,
            delivery_path: Some(format!(
                "cua-{}-{mode}",
                if route.is_empty() { "unknown" } else { &route }
            )),
            verified: Some(if confirmed {
                ComputerDeliveryVerification::Confirmed
            } else if matches!(effect_str(&data), Some("unconfirmed" | "suspected_noop")) {
                ComputerDeliveryVerification::Unconfirmed
            } else {
                ComputerDeliveryVerification::Unverifiable
            }),
            effect: Some(if confirmed {
                CuaEffect::Verified
            } else {
                CuaEffect::DispatchedUnknown
            }),
            value: data
                .get("selected_text")
                .and_then(Value::as_str)
                .map(|text| js_slice(text, 16384).to_owned()),
            ..ComputerBackendActionResult::default()
        })
    }

    async fn click_gesture(
        self: &Arc<Self>,
        point: ComputerPoint,
        window_id: Option<&str>,
        modifiers: &[ComputerInputModifier],
        count: Option<u8>,
        right: bool,
        synthetic_by_default: bool,
    ) -> Result<ComputerBackendActionResult, ComputerError> {
        if right && self.is_darwin() && desktop_delivery_mode() != ComputerDeliveryMode::Foreground
        {
            return Err(CuaActionError::new(
                "A background right-click was not sent. Its context menu opens in a separate window that window screenshots, the window list and background element actions never see or address, so the menu could stay open unseen and absorb the next click. Use computer_invoke_menu for a menu-bar command, or the element's Show menu action (performSecondaryAction(index, \"Show menu\")) when its state lists one.",
                CuaEffect::NotDispatched,
            )
            .code("context_menu_unobservable")
            .into());
        }
        let mut args = Map::new();
        if synthetic_by_default || self.native_revision().unwrap_or(0) < 34 || !modifiers.is_empty()
        {
            args.insert("force_synthetic".into(), Value::Bool(true));
        }
        if right {
            args.insert("button".into(), json!("right"));
        }
        if let Some(count) = count {
            args.insert("count".into(), json!(count));
        }
        if !modifiers.is_empty() {
            args.insert("modifier".into(), Value::Array(cua_modifiers(modifiers)?));
        }
        self.input("click", args, window_id, Some(point), None)
            .await
    }

    async fn move_cursor(
        self: &Arc<Self>,
        point: ComputerPoint,
        window_id: Option<&str>,
    ) -> Result<ComputerBackendActionResult, ComputerError> {
        if let Some(window_id) = window_id {
            self.target(Some(window_id), true).await?;
        }
        let mut args = Map::new();
        args.insert("x".into(), num(point.x));
        args.insert("y".into(), num(point.y));
        self.call("move_cursor", args, true, true).await?;
        Ok(ComputerBackendActionResult {
            point: Some(point),
            delivery_path: Some("cua-overlay-only".into()),
            verified: Some(ComputerDeliveryVerification::Unverifiable),
            ..ComputerBackendActionResult::default()
        })
    }

    async fn drag(
        self: &Arc<Self>,
        from: ComputerPoint,
        to: ComputerPoint,
        duration_ms: f64,
        window_id: Option<&str>,
    ) -> Result<ComputerBackendActionResult, ComputerError> {
        if duration_ms > 10_000.0 {
            return Err(CuaActionError::new(
                "Cua drag duration is limited to 10 seconds.",
                CuaEffect::NotDispatched,
            )
            .code("unsupported_operation")
            .into());
        }
        let target = self.target(window_id, true).await?;
        let bounds = target.window.bounds.unwrap_or(ComputerRect {
            x: 0.0,
            y: 0.0,
            width: 0.0,
            height: 0.0,
        });
        let observed = self
            .state()
            .observed_geometry
            .get(&target.window.id)
            .copied();
        if !observed.is_some_and(|observed| same_rect(&observed, &bounds)) {
            return Err(CuaActionError::new(
                "Window geometry changed since observation; obtain a new screenshot.",
                CuaEffect::NotDispatched,
            )
            .code("stale_geometry")
            .into());
        }
        let local = |point: ComputerPoint| -> Result<(f64, f64), ComputerError> {
            let x = point.x - bounds.x;
            let y = point.y - bounds.y;
            if !x.is_finite()
                || !y.is_finite()
                || x < 0.0
                || y < 0.0
                || x >= bounds.width
                || y >= bounds.height
            {
                return Err(CuaActionError::new(
                    "Drag crosses outside its target window.",
                    CuaEffect::NotDispatched,
                )
                .into());
            }
            Ok((x, y))
        };
        let start = local(from)?;
        let end = local(to)?;
        let mut args = Map::new();
        args.insert("from_x".into(), num(start.0));
        args.insert("from_y".into(), num(start.1));
        args.insert("to_x".into(), num(end.0));
        args.insert("to_y".into(), num(end.1));
        args.insert("duration_ms".into(), num(duration_ms));
        args.insert("coordinate_space".into(), json!("window_points"));
        let window = target.window.id.clone();
        self.input("drag", args, Some(&window), None, Some(bounds))
            .await
    }

    #[allow(clippy::too_many_arguments)]
    async fn scroll(
        self: &Arc<Self>,
        point: Option<ComputerPoint>,
        dx: f64,
        dy: f64,
        window_id: Option<&str>,
        modifiers: &[ComputerInputModifier],
        target: Option<&ComputerResolvedTarget>,
        prepared_bounds: Option<ComputerRect>,
    ) -> Result<ComputerBackendActionResult, ComputerError> {
        let Some(point) = point else {
            return Err(CuaActionError::new(
                "Scroll requires a screenshot target point.",
                CuaEffect::NotDispatched,
            )
            .into());
        };
        if dx == 0.0 && dy == 0.0 {
            return Ok(ComputerBackendActionResult {
                window_id: window_id.map(str::to_owned),
                scroll_delta: Some(ComputerScrollDelta {
                    delta_x: 0.0,
                    delta_y: 0.0,
                }),
                delivery_path: Some("cua-no-op".into()),
                verified: Some(ComputerDeliveryVerification::Unverifiable),
                effect: Some(CuaEffect::NotDispatched),
                ..ComputerBackendActionResult::default()
            });
        }
        let ticks = |delta: f64| {
            if delta == 0.0 {
                0.0
            } else {
                ((delta.abs() / 120.0 + 0.5).floor()).max(1.0)
            }
        };
        let ticks_x = ticks(dx);
        let ticks_y = ticks(dy);
        if ticks_x > 50.0 || ticks_y > 50.0 {
            return Err(CuaActionError::new(
                "Scroll exceeds Cua's 50-notch limit.",
                CuaEffect::NotDispatched,
            )
            .code("unsupported_operation")
            .into());
        }
        let mods = if modifiers.is_empty() {
            None
        } else {
            Some(cua_modifiers(modifiers)?)
        };
        let wheel_sign = if self.is_darwin() { -1.0 } else { 1.0 };
        let token = target
            .and_then(|target| self.native_element(target))
            .map(|element| element.token);
        if let Some(target) = target
            && observed_computer_target_node(&target.target).is_some()
            && (token.is_none() || dx != 0.0 || mods.is_some())
        {
            return Err(CuaActionError::new(
                "This observed element only supports exact unmodified vertical scrolling. Use an explicit current screenshot target for other wheel gestures; no coordinate fallback was sent.",
                CuaEffect::NotDispatched,
            )
            .code("unsupported_operation")
            .into());
        }
        let mut args = Map::new();
        let element_scroll = token.is_some() && dx == 0.0 && mods.is_none();
        match token {
            Some(token) if element_scroll => {
                args.insert(
                    "direction".into(),
                    json!(if dy > 0.0 { "down" } else { "up" }),
                );
                args.insert("amount".into(), num(ticks_y));
                args.insert("by".into(), json!("line"));
                args.insert("delta_x".into(), num(0.0));
                args.insert("delta_y".into(), num(wheel_sign * dy.signum() * ticks_y));
                args.insert("element_token".into(), json!(token));
            }
            _ => {
                let direction = if ticks_y != 0.0 {
                    if dy > 0.0 { "down" } else { "up" }
                } else if dx > 0.0 {
                    "right"
                } else {
                    "left"
                };
                args.insert("direction".into(), json!(direction));
                args.insert(
                    "delta_x".into(),
                    num(if ticks_x != 0.0 {
                        wheel_sign * dx.signum() * ticks_x
                    } else {
                        0.0
                    }),
                );
                args.insert(
                    "delta_y".into(),
                    num(if ticks_y != 0.0 {
                        wheel_sign * dy.signum() * ticks_y
                    } else {
                        0.0
                    }),
                );
                if let Some(mods) = mods {
                    args.insert("modifiers".into(), Value::Array(mods));
                }
            }
        }
        let mut result = if element_scroll {
            self.input("scroll", args, window_id, None, None).await?
        } else {
            self.input("scroll", args, window_id, Some(point), prepared_bounds)
                .await?
        };
        result.point = Some(point);
        result.scroll_delta = Some(ComputerScrollDelta {
            delta_x: if ticks_x != 0.0 {
                dx.signum() * ticks_x * 120.0
            } else {
                0.0
            },
            delta_y: if ticks_y != 0.0 {
                dy.signum() * ticks_y * 120.0
            } else {
                0.0
            },
        });
        Ok(result)
    }

    async fn type_text(
        self: &Arc<Self>,
        value: &str,
        window_id: Option<&str>,
        target: Option<&ComputerResolvedTarget>,
    ) -> Result<ComputerBackendActionResult, ComputerError> {
        let element = target.and_then(|target| self.native_element(target));
        if let Some(target) = target
            && (element.is_none() || target.node.window_id.is_none())
        {
            return Err(CuaActionError::new(
                "The AX text target is not bound to a live Cua token.",
                CuaEffect::NotDispatched,
            )
            .code("stale_target")
            .into());
        }
        if let (Some(target), Some(element)) = (target, &element)
            && element.in_web_content
        {
            let node_window = target.node.window_id.clone().unwrap_or_default();
            let written = if observed_computer_target_node(&target.target).is_some() {
                if self.native_revision().unwrap_or(0) < 37 {
                    return Err(CuaActionError::new(
                        "This driver cannot append through a retained web element. Use computer_set_value to replace its complete value, or update the driver; no input was sent.",
                        CuaEffect::NotDispatched,
                    )
                    .code("unsupported_operation")
                    .into());
                }
                let mut args = Map::new();
                args.insert("element_token".into(), json!(element.token));
                args.insert("value".into(), json!(value));
                args.insert("append".into(), Value::Bool(true));
                self.input("set_value", args, Some(&node_window), None, None)
                    .await
            } else {
                self.web_content_type_text(&target.node, value).await
            };
            if !written
                .as_ref()
                .err()
                .and_then(ComputerError::cua)
                .is_some_and(|cua| cua.code == "rich_text_needs_keys")
            {
                return written;
            }
            let token = if observed_computer_target_node(&target.target).is_some() {
                element.token.clone()
            } else {
                let Some(field) = self.resolve_web_field(&node_window, &target.node).await? else {
                    return written;
                };
                field.token
            };
            let mut args = Map::new();
            args.insert("text".into(), json!(value));
            args.insert("element_token".into(), json!(token));
            args.insert("delay_ms".into(), json!(10));
            return self
                .input("type_text", args, Some(&node_window), None, None)
                .await;
        }
        let macos = self.is_darwin();
        let mut args = Map::new();
        args.insert("text".into(), json!(value));
        if let Some(element) = &element {
            args.insert("element_token".into(), json!(element.token));
            args.insert("semantic_only".into(), Value::Bool(true));
            if macos {
                args.insert("delay_ms".into(), json!(0));
            }
        } else if macos && desktop_delivery_mode() != ComputerDeliveryMode::Foreground {
            args.insert("delay_ms".into(), json!(10));
        } else {
            args.insert("force_synthetic".into(), Value::Bool(true));
        }
        let window = target
            .and_then(|target| target.node.window_id.clone())
            .or_else(|| window_id.map(str::to_owned));
        self.input("type_text", args, window.as_deref(), None, None)
            .await
    }

    async fn web_content_type_text(
        self: &Arc<Self>,
        node: &ComputerUiNode,
        value: &str,
    ) -> Result<ComputerBackendActionResult, ComputerError> {
        let window_id = node.window_id.clone().unwrap_or_default();
        let resolved = self.target(Some(&window_id), true).await?;
        let this = self.clone();
        let node = node.clone();
        let value = value.to_owned();
        self.semantic_text_in_lane(
            resolved.pid,
            resolved.window_id,
            Box::new(move |admission| {
                Box::pin(async move {
                    let Some(before) = this.resolve_web_field(&window_id, &node).await? else {
                        return Err(CuaActionError::new(
                            "The web text element is no longer present; observe fresh state.",
                            CuaEffect::NotDispatched,
                        )
                        .code("stale_target")
                        .into());
                    };
                    assert_desktop_operation_active()?;
                    // The snapshot value falls back to the placeholder for an empty field, so
                    // composing from it would write the placeholder; the driver appends to the live value.
                    if this.native_revision().unwrap_or(0) >= 37 {
                        return this
                            .web_set_value(&node, &window_id, before, &value, true, &admission)
                            .await;
                    }
                    let composed = format!("{}{value}", before.value.clone().unwrap_or_default());
                    this.web_set_value(&node, &window_id, before, &composed, false, &admission)
                        .await
                })
            }),
        )
        .await
    }

    async fn web_set_value(
        self: &Arc<Self>,
        node: &ComputerUiNode,
        window_id: &str,
        field: WebField,
        value: &str,
        append: bool,
        admission: &LaneAdmission,
    ) -> Result<ComputerBackendActionResult, ComputerError> {
        let before = field.value.clone().unwrap_or_default();
        let mut args = Map::new();
        args.insert("element_token".into(), json!(field.token));
        args.insert("element_index".into(), num(field.index));
        args.insert("value".into(), json!(value));
        if append {
            args.insert("append".into(), Value::Bool(true));
        }
        let resolved = self.target(Some(window_id), true).await?;
        let result = self
            .input_dispatch(
                "set_value",
                args,
                Some(window_id),
                None,
                None,
                true,
                resolved,
                Some(admission),
            )
            .await?;
        assert_desktop_operation_active()?;
        let after = self.resolve_web_field(window_id, node).await?;
        // An append only lands if the field changed; an unchanged field that already ended
        // with the text proves nothing.
        let landed = after.is_some_and(|after| {
            let after = after.value.unwrap_or_default();
            if append {
                after != before && after.ends_with(value)
            } else if node.role == "AXPopUpButton" {
                after.to_lowercase() == value.to_lowercase()
            } else {
                after == value
            }
        });
        if landed {
            return Ok(ComputerBackendActionResult {
                verified: Some(ComputerDeliveryVerification::Confirmed),
                effect: Some(CuaEffect::Verified),
                ..result
            });
        }
        Ok(ComputerBackendActionResult {
            verified: Some(ComputerDeliveryVerification::Unconfirmed),
            effect: Some(CuaEffect::DispatchedUnknown),
            ..result
        })
    }

    async fn resolve_web_field(
        self: &Arc<Self>,
        window_id: &str,
        node: &ComputerUiNode,
    ) -> Result<Option<WebField>, ComputerError> {
        let resolved = self.target(Some(window_id), true).await?;
        let mut args = Self::window_args(resolved.pid, resolved.window_id);
        args.insert("include_screenshot".into(), Value::Bool(false));
        args.insert("include_accessibility_tree".into(), Value::Bool(true));
        args.insert("max_elements".into(), json!(CUA_WALK_MAX_ELEMENTS));
        args.insert("max_depth".into(), json!(25));
        let result = self.call("get_window_state", args, false, true).await?;
        let Some(elements) = result
            .structured_content
            .as_ref()
            .and_then(|data| data.get("elements"))
            .and_then(Value::as_array)
        else {
            return Ok(None);
        };
        let mut found = None;
        for value in elements {
            let element = record(Some(value));
            if element.get("in_web_content") != Some(&Value::Bool(true)) {
                continue;
            }
            if text(element.get("role"), 128) != node.role {
                continue;
            }
            let label = text1024(element.get("label"));
            if (!label.is_empty()).then_some(label) != node.label {
                continue;
            }
            if !optional_rect(element.get("frame"))
                .is_some_and(|frame| same_rect(&frame, &node.frame))
            {
                continue;
            }
            let Some(token) = element.get("element_token").and_then(Value::as_str) else {
                continue;
            };
            if found.is_some() {
                return Ok(None);
            }
            let index = number(element.get("element_index"));
            found = Some(WebField {
                token: token.to_owned(),
                index: if index.is_finite() { index } else { 0.0 },
                value: element
                    .get("value")
                    .and_then(Value::as_str)
                    .map(str::to_owned),
            });
        }
        Ok(found)
    }

    fn keyboard_target(
        &self,
        target: Option<&ComputerResolvedTarget>,
        window_id: Option<&str>,
    ) -> Result<Option<String>, ComputerError> {
        let Some(target) = target else {
            return Ok(None);
        };
        let element = self.native_element(target);
        match (element, target.node.window_id.as_deref()) {
            (Some(element), Some(node_window))
                if window_id.is_none_or(|window_id| window_id == node_window) =>
            {
                Ok(Some(element.token))
            }
            _ => Err(CuaActionError::new(
                "The keyboard target is not bound to a live element in the requested window; observe fresh state.",
                CuaEffect::NotDispatched,
            )
            .code("stale_target")
            .into()),
        }
    }

    async fn press_key(
        self: &Arc<Self>,
        key: &str,
        window_id: Option<&str>,
        target: Option<&ComputerResolvedTarget>,
    ) -> Result<ComputerBackendActionResult, ComputerError> {
        let mut args = Map::new();
        args.insert("key".into(), json!(cua_key(key)?));
        if let Some(token) = self.keyboard_target(target, window_id)? {
            args.insert("element_token".into(), json!(token));
        }
        let window = window_id
            .map(str::to_owned)
            .or_else(|| target.and_then(|target| target.node.window_id.clone()));
        self.input("press_key", args, window.as_deref(), None, None)
            .await
    }

    async fn hotkey(
        self: &Arc<Self>,
        keys: &[String],
        window_id: Option<&str>,
        target: Option<&ComputerResolvedTarget>,
    ) -> Result<ComputerBackendActionResult, ComputerError> {
        let native = keys
            .iter()
            .map(|key| cua_key(key))
            .collect::<Result<Vec<_>, _>>()?;
        const MODIFIERS: [&str; 8] = [
            "command", "cmd", "shift", "option", "alt", "ctrl", "control", "fn",
        ];
        let others = native
            .iter()
            .filter(|key| !MODIFIERS.contains(&key.as_str()))
            .count();
        if native.len() < 2 || others != 1 {
            return Err(CuaActionError::new(
                "A shortcut requires modifiers and exactly one other key.",
                CuaEffect::NotDispatched,
            )
            .code("invalid_chord")
            .into());
        }
        let mut args = Map::new();
        args.insert("keys".into(), json!(native));
        if let Some(token) = self.keyboard_target(target, window_id)? {
            args.insert("element_token".into(), json!(token));
        }
        let window = window_id
            .map(str::to_owned)
            .or_else(|| target.and_then(|target| target.node.window_id.clone()));
        self.input("hotkey", args, window.as_deref(), None, None)
            .await
    }

    async fn set_value(
        self: &Arc<Self>,
        target: &ComputerResolvedTarget,
        value: &str,
    ) -> Result<ComputerBackendActionResult, ComputerError> {
        let element = self.native_element(target);
        let (Some(element), Some(window_id)) = (element, target.node.window_id.clone()) else {
            return Err(CuaActionError::new(
                "The AX target is not bound to a live Cua token.",
                CuaEffect::NotDispatched,
            )
            .code("stale_target")
            .into());
        };
        let mut args = Map::new();
        args.insert("element_token".into(), json!(element.token));
        args.insert("value".into(), json!(value));
        if observed_computer_target_node(&target.target).is_none() && element.in_web_content {
            let resolved = self.target(Some(&window_id), true).await?;
            let this = self.clone();
            let node = target.node.clone();
            let value = value.to_owned();
            return self
                .semantic_text_in_lane(
                    resolved.pid,
                    resolved.window_id,
                    Box::new(move |admission| {
                        Box::pin(async move {
                            let Some(field) = this.resolve_web_field(&window_id, &node).await?
                            else {
                                return Err(CuaActionError::new(
                                    "The web text element is no longer present; observe fresh state.",
                                    CuaEffect::NotDispatched,
                                )
                                .code("stale_target")
                                .into());
                            };
                            this.web_set_value(&node, &window_id, field, &value, false, &admission)
                                .await
                        })
                    }),
                )
                .await;
        }
        self.input("set_value", args, Some(&window_id), None, None)
            .await
    }

    fn supports_action(&self, target: &ComputerResolvedTarget, action: &str) -> bool {
        // The driver pointer-clicks web elements that ignore AXPress.
        self.native_element(target).is_some_and(|element| {
            let actions = element.actions.as_deref();
            cua_element_action(action, actions).is_some_and(|spec| {
                (element.in_web_content && spec.ax_action == "AXPress")
                    || actions.is_some_and(|actions| actions.contains(&spec.ax_action))
            })
        })
    }

    async fn perform_action(
        self: &Arc<Self>,
        target: &ComputerResolvedTarget,
        action: &str,
    ) -> Result<ComputerBackendActionResult, ComputerError> {
        let element = self.native_element(target);
        let (Some(element), Some(window_id)) = (element, target.node.window_id.clone()) else {
            return Err(CuaActionError::new(
                "The AX target is no longer valid.",
                CuaEffect::NotDispatched,
            )
            .code("stale_target")
            .into());
        };
        let Some(spec) = cua_element_action(action, element.actions.as_deref()) else {
            return Err(CuaActionError::new(
                format!("The element does not list the action {action}; nothing was dispatched."),
                CuaEffect::NotDispatched,
            )
            .code("unsupported_operation")
            .into());
        };
        if spec.ax_action != "AXPress"
            && !element
                .actions
                .as_ref()
                .is_some_and(|actions| actions.contains(&spec.ax_action))
        {
            return Err(CuaActionError::new(
                format!(
                    "The resolved element does not advertise {}; {action} was not dispatched.",
                    spec.ax_action
                ),
                CuaEffect::NotDispatched,
            )
            .code("unsupported_operation")
            .into());
        }
        let mut args = Map::new();
        args.insert("element_token".into(), json!(element.token));
        args.insert("action".into(), json!(spec.driver_action));
        let outcome = self
            .input("click", args, Some(&window_id), None, None)
            .await;
        match outcome {
            Err(error) if spec.ax_action == "AXPress" && ax_press_cannot_complete(&error) => {
                self.press_by_pointer_after_no_effect(target, &window_id, error)
                    .await
            }
            outcome => outcome,
        }
    }

    async fn press_by_pointer_after_no_effect(
        self: &Arc<Self>,
        target: &ComputerResolvedTarget,
        window_id: &str,
        error: ComputerError,
    ) -> Result<ComputerBackendActionResult, ComputerError> {
        let (before, pid, windows_before) = {
            let state = self.state();
            let before = state
                .recent_trees
                .iter()
                .find(|(id, _)| id == window_id)
                .map(|(_, tree)| observed_surface(&tree.root));
            let pid = state
                .windows
                .iter()
                .find(|window| window.id == window_id)
                .and_then(|window| window.pid);
            (before, pid, app_window_ids(&state.windows, pid))
        };
        let Some(before) = before else {
            return Err(error);
        };
        let reread = self
            .get_state(ComputerStateOptions {
                include_screenshot: false,
                include_tree: true,
                window_id: Some(window_id.to_owned()),
                reuse_recent_tree: false,
            })
            .await;
        let unchanged = reread.is_ok_and(|state| {
            state.root.as_ref().map(observed_surface) == Some(before)
                && app_window_ids(&self.state().windows, pid) == windows_before
        });
        if !unchanged {
            return Err(error);
        }
        let resolved = match self.target(Some(window_id), true).await {
            Ok(resolved) => resolved,
            Err(_) => return Err(error),
        };
        let Some(bounds) = resolved.window.bounds else {
            return Err(error);
        };
        let previous = self
            .state()
            .observed_geometry
            .insert(window_id.to_owned(), bounds);
        let point = activation_point_for_node(&target.node);
        let clicked = self
            .click_gesture(point, Some(window_id), &[], Some(1), false, false)
            .await;
        let keep_invalidation = clicked
            .as_ref()
            .is_err_and(|pixel| pixel.effect() != Some(CuaEffect::NotDispatched));
        if !keep_invalidation {
            let mut state = self.state();
            match previous {
                Some(previous) => state
                    .observed_geometry
                    .insert(window_id.to_owned(), previous),
                None => state.observed_geometry.remove(window_id),
            };
        }
        match clicked {
            Ok(mut result) => {
                result.delivery_path = Some(format!(
                    "cua-ax-then-pixel-{}",
                    match desktop_delivery_mode() {
                        ComputerDeliveryMode::Foreground => "foreground",
                        ComputerDeliveryMode::Background => "background",
                    }
                ));
                Ok(result)
            }
            Err(pixel) if pixel.effect() == Some(CuaEffect::NotDispatched) => Err(error),
            Err(pixel) => Err(pixel),
        }
    }

    async fn select_text(
        self: &Arc<Self>,
        target: &ComputerResolvedTarget,
        range: ComputerTextRange,
    ) -> Result<ComputerBackendActionResult, ComputerError> {
        let element = self.native_element(target);
        let (Some(element), Some(window_id)) = (element, target.node.window_id.clone()) else {
            return Err(CuaActionError::new(
                "The AX text target is not bound to a live Cua token.",
                CuaEffect::NotDispatched,
            )
            .code("stale_target")
            .into());
        };
        let mut args = Map::new();
        args.insert("element_token".into(), json!(element.token));
        args.insert("start".into(), json!(range.start));
        args.insert("length".into(), json!(range.length));
        self.input("select_text", args, Some(&window_id), None, None)
            .await
    }

    async fn read_clipboard(self: &Arc<Self>) -> Result<String, ComputerError> {
        let mut args = Map::new();
        args.insert("include_text".into(), Value::Bool(true));
        let data = self.call_structured("clipboard_read", args, true).await?;
        let not_restorable = |message: &str| -> ComputerError {
            CuaActionError::new(message, CuaEffect::NotDispatched)
                .code(CLIPBOARD_NOT_RESTORABLE_CODE)
                .into()
        };
        let Some(text) = data.get("text").and_then(Value::as_str) else {
            if data
                .get("types")
                .and_then(Value::as_array)
                .is_some_and(|types| !types.is_empty())
            {
                return Err(not_restorable(
                    "The clipboard holds non-text content (an image, files or rich data) that cannot be read back as text.",
                ));
            }
            return Err(ComputerError::Other(
                "The clipboard does not contain readable text.".into(),
            ));
        };
        if utf16_len(text) > 16384 {
            return Err(not_restorable("Clipboard exceeds the tool's text limit."));
        }
        Ok(text.to_owned())
    }

    async fn write_clipboard(self: &Arc<Self>, value: &str) -> Result<(), ComputerError> {
        assert_computer_clipboard_write_fits(value)?;
        let mut args = Map::new();
        args.insert("text".into(), json!(value));
        self.call("clipboard_write", args, true, true).await?;
        Ok(())
    }

    async fn write_clipboard_html(
        self: &Arc<Self>,
        text: &str,
        html: &str,
    ) -> Result<(), ComputerError> {
        assert_computer_clipboard_write_fits(text)?;
        assert_computer_clipboard_write_fits(html)?;
        let mut args = Map::new();
        args.insert("text".into(), json!(text));
        args.insert("html".into(), json!(html));
        self.call("clipboard_write", args, true, true).await?;
        Ok(())
    }

    async fn launch_app(
        self: &Arc<Self>,
        app: &str,
        launch_args: &[String],
        options: ComputerLaunchOptions,
    ) -> Result<ComputerLaunchAppResult, ComputerError> {
        if self.state().host_platform.is_none() {
            self.host(HostRequest::new(ComputerHostMethod::Probe), false, true)
                .await?;
        }
        let linux = self.is_linux();
        if linux && options.hidden != Some(false) {
            return Err(CuaActionError::new(
                "This Linux driver cannot guarantee a hidden app launch. Use an already open app, or request a visible launch only when the user's task asks to see the app.",
                CuaEffect::NotDispatched,
            )
            .code("unsupported_operation")
            .into());
        }
        if !linux && app.starts_with('/') {
            return Err(CuaActionError::new(
                "Cua cannot launch a macOS app by its path. Pass an installed app's name or bundle identifier, or start the app first and bind its open window.",
                CuaEffect::NotDispatched,
            )
            .code("unsupported_operation")
            .into());
        }
        if linux && app.starts_with('/') && app.chars().any(char::is_whitespace) {
            return Err(CuaActionError::new(
                "This Linux driver cannot launch an executable path containing whitespace. Use the installed application's desktop ID and pass arguments separately.",
                CuaEffect::NotDispatched,
            )
            .code("unsupported_operation")
            .into());
        }
        let mut args = Map::new();
        if linux {
            if app.starts_with('/') {
                args.insert("launch_path".into(), json!(app));
            } else {
                args.insert("name".into(), json!(app));
            }
        } else if BUNDLE_ID.is_match(app) {
            args.insert("bundle_id".into(), json!(app));
        } else {
            args.insert("name".into(), json!(app));
        }
        if !launch_args.is_empty() {
            args.insert("additional_arguments".into(), json!(launch_args));
        }
        if !linux && options.hidden == Some(true) {
            args.insert("hidden".into(), Value::Bool(true));
        }
        let data = self.call_structured("launch_app", args, true).await?;
        let pid = number(data.get("pid"));
        let window_reason = match data.get("window_reason").and_then(Value::as_str) {
            Some("hidden") => Some(ComputerLaunchWindowReason::Hidden),
            Some("off_space") => Some(ComputerLaunchWindowReason::OffSpace),
            Some("no_window") => Some(ComputerLaunchWindowReason::NoWindow),
            Some("input_unavailable") => Some(ComputerLaunchWindowReason::InputUnavailable),
            _ => None,
        };
        let unavailable = data.get("window_status").and_then(Value::as_str)
            == Some("no_usable_window")
            && window_reason.is_some();
        Ok(ComputerLaunchAppResult {
            computer_id: DEFAULT_COMPUTER_ID.to_owned(),
            app: app.to_owned(),
            window: None,
            focus_changed_during_launch: data
                .get("focus_changed_during_launch")
                .and_then(Value::as_bool),
            window_status: Some(if unavailable {
                ComputerLaunchWindowStatus::NoUsableWindow
            } else {
                ComputerLaunchWindowStatus::NotChecked
            }),
            window_reason: window_reason.filter(|_| unavailable),
            pid: safe_integer(pid)
                .filter(|pid| *pid > 0 && *pid <= 0x7fff_ffff)
                .map(|pid| pid as u32),
            resolved_command: None,
        })
    }

    async fn list_apps(self: &Arc<Self>) -> Result<Vec<ComputerApp>, ComputerError> {
        let data = self.call_structured("list_apps", Map::new(), false).await?;
        let Some(rows) = data.get("apps").and_then(Value::as_array) else {
            return Err(CuaActionError::new(
                "Cua returned an invalid app list.",
                CuaEffect::NotDispatched,
            )
            .code("invalid_response")
            .into());
        };
        let darwin = self.is_darwin();
        let mut apps = Vec::new();
        for value in rows {
            let row = record(Some(value));
            let pid = number(row.get("pid"));
            let name = match text(row.get("name"), 512) {
                name if !name.is_empty() => name,
                _ => text(row.get("app_name"), 512),
            };
            let Some(pid) = safe_integer(pid).filter(|pid| *pid >= 0) else {
                continue;
            };
            let Ok(pid) = u32::try_from(pid) else {
                continue;
            };
            if name.is_empty() {
                continue;
            }
            let bundle_id = text(row.get("bundle_id"), 512);
            let team_id = match text(row.get("team_id"), 128) {
                team if !team.is_empty() => team,
                _ => text(row.get("signing_team_id"), 128),
            };
            let reported = text(row.get("launch_path"), 4_096);
            let launch_path = match (darwin && pid > 0).then(|| running_app_bundle(pid)) {
                Some(Some(bundle)) if !same_file(&bundle, &reported) => bundle,
                _ => reported,
            };
            let last_used = text(row.get("last_used"), 64);
            apps.push(ComputerApp {
                pid,
                name: js_slice(&name, 256).to_owned(),
                running: row.get("running") == Some(&Value::Bool(true)) || pid > 0,
                active: row.get("active") == Some(&Value::Bool(true))
                    || row.get("is_active") == Some(&Value::Bool(true)),
                bundle_id: (!bundle_id.is_empty()).then_some(bundle_id),
                team_id: (!team_id.is_empty()).then_some(team_id),
                launch_path: (!launch_path.is_empty()).then_some(launch_path),
                window_count: row
                    .get("windows")
                    .and_then(Value::as_array)
                    .map(|windows| windows.len() as u64),
                last_used: (!last_used.is_empty()).then_some(last_used),
            });
        }
        apps.truncate(1_024);
        Ok(apps)
    }

    fn route_path(data: &Map<String, Value>, route: &str, mode: &str) -> String {
        let native_route = text(data.get("route"), 64);
        let native_mode = text(record(data.get("delivery")).get("mode"), 32);
        format!(
            "cua-{}-{}",
            if native_route.is_empty() {
                route
            } else {
                &native_route
            },
            if native_mode.is_empty() {
                mode
            } else {
                &native_mode
            }
        )
    }

    async fn set_window_frame(
        self: &Arc<Self>,
        window_id: &str,
        frame: ComputerRect,
    ) -> Result<ComputerBackendActionResult, ComputerError> {
        if !(frame.x.is_finite()
            && frame.y.is_finite()
            && frame.width.is_finite()
            && frame.height.is_finite())
            || frame.width <= 0.0
            || frame.height <= 0.0
        {
            return Err(CuaActionError::new(
                "A window frame needs finite coordinates and a positive size.",
                CuaEffect::NotDispatched,
            )
            .code("invalid_geometry")
            .into());
        }
        let resolved = self.target(Some(window_id), true).await?;
        let mut args = Self::window_args(resolved.pid, resolved.window_id);
        args.insert("x".into(), num(frame.x));
        args.insert("y".into(), num(frame.y));
        args.insert("width".into(), num(frame.width));
        args.insert("height".into(), num(frame.height));
        let data = self.call_structured("set_window_frame", args, true).await?;
        let observed = match self.read_windows(false).await {
            Ok(windows) => windows
                .into_iter()
                .find(|candidate| candidate.id == resolved.window.id)
                .and_then(|window| window.bounds),
            Err(_) => None,
        };
        let confirmed = observed.is_some_and(|observed| same_rect(&observed, &frame));
        {
            let mut state = self.state();
            match observed {
                Some(observed) => {
                    state
                        .observed_geometry
                        .insert(resolved.window.id.clone(), observed);
                }
                None => {
                    state.observed_geometry.remove(&resolved.window.id);
                }
            }
            state.snapshot_at = None;
        }
        Ok(ComputerBackendActionResult {
            window_id: Some(resolved.window.id),
            delivery_path: Some(Self::route_path(&data, "window_frame", "background")),
            verified: Some(if confirmed {
                ComputerDeliveryVerification::Confirmed
            } else {
                ComputerDeliveryVerification::Unconfirmed
            }),
            effect: Some(if confirmed {
                CuaEffect::Verified
            } else {
                CuaEffect::DispatchedUnknown
            }),
            ..ComputerBackendActionResult::default()
        })
    }

    async fn invoke_menu(
        self: &Arc<Self>,
        target: &ComputerMenuBackendTarget,
        path: &[String],
    ) -> Result<ComputerBackendActionResult, ComputerError> {
        if path.is_empty() || path.len() > 6 || path.iter().any(|segment| segment.trim().is_empty())
        {
            return Err(CuaActionError::new(
                "A menu path needs one to six non-empty titles.",
                CuaEffect::NotDispatched,
            )
            .code("invalid_arguments")
            .into());
        }
        let (data, window_id) = match target {
            ComputerMenuBackendTarget::Window { window_id } => {
                let resolved = self.target(Some(window_id), true).await?;
                let mut args = Self::window_args(resolved.pid, resolved.window_id);
                args.insert("path".into(), json!(path));
                (
                    self.call_structured("invoke_menu", args, true).await?,
                    Some(resolved.window.id),
                )
            }
            ComputerMenuBackendTarget::Pid { pid } => {
                if *pid == 0 {
                    return Err(CuaActionError::new(
                        "invoke_menu needs a positive integer pid for the windowless form.",
                        CuaEffect::NotDispatched,
                    )
                    .code("invalid_arguments")
                    .into());
                }
                let mut args = Map::new();
                args.insert("pid".into(), json!(pid));
                args.insert("path".into(), json!(path));
                (self.call_structured("invoke_menu", args, true).await?, None)
            }
        };
        let confirmed = effect_str(&data) == Some("confirmed");
        self.state().snapshot_at = None;
        Ok(ComputerBackendActionResult {
            window_id,
            delivery_path: Some(Self::route_path(&data, "menu", "background")),
            verified: Some(if confirmed {
                ComputerDeliveryVerification::Confirmed
            } else if effect_str(&data) == Some("unconfirmed") {
                ComputerDeliveryVerification::Unconfirmed
            } else {
                ComputerDeliveryVerification::Unverifiable
            }),
            effect: Some(if confirmed {
                CuaEffect::Verified
            } else {
                CuaEffect::DispatchedUnknown
            }),
            ..ComputerBackendActionResult::default()
        })
    }

    fn readback_result(
        data: &Map<String, Value>,
        route: &str,
        window_id: Option<String>,
    ) -> ComputerBackendActionResult {
        let confirmed = confirmed_value_readback(data);
        ComputerBackendActionResult {
            window_id,
            delivery_path: Some(Self::route_path(data, route, "background")),
            verified: Some(if confirmed {
                ComputerDeliveryVerification::Confirmed
            } else if matches!(effect_str(data), Some("unconfirmed" | "suspected_noop")) {
                ComputerDeliveryVerification::Unconfirmed
            } else {
                ComputerDeliveryVerification::Unverifiable
            }),
            effect: Some(if confirmed {
                CuaEffect::Verified
            } else {
                CuaEffect::DispatchedUnknown
            }),
            ..ComputerBackendActionResult::default()
        }
    }

    async fn set_window_minimized(
        self: &Arc<Self>,
        window_id: &str,
        minimized: bool,
    ) -> Result<ComputerBackendActionResult, ComputerError> {
        let resolved = self.target(Some(window_id), true).await?;
        let mut args = Self::window_args(resolved.pid, resolved.window_id);
        args.insert("minimized".into(), Value::Bool(minimized));
        let data = self
            .call_structured("set_window_minimized", args, true)
            .await?;
        self.state().snapshot_at = None;
        Ok(Self::readback_result(
            &data,
            "window_minimized",
            Some(resolved.window.id),
        ))
    }

    async fn set_app_visibility(
        self: &Arc<Self>,
        pid: u32,
        hidden: bool,
    ) -> Result<ComputerBackendActionResult, ComputerError> {
        if pid == 0 {
            return Err(CuaActionError::new(
                "set_app_visibility needs a positive integer pid and a boolean hidden flag.",
                CuaEffect::NotDispatched,
            )
            .code("invalid_arguments")
            .into());
        }
        let mut args = Map::new();
        args.insert("pid".into(), json!(pid));
        args.insert("hidden".into(), Value::Bool(hidden));
        let data = self
            .call_structured("set_app_visibility", args, true)
            .await?;
        self.state().snapshot_at = None;
        Ok(Self::readback_result(&data, "app_visibility", None))
    }

    async fn verify_state(
        self: &Arc<Self>,
        window_id: &str,
        expect: &[Value],
    ) -> Result<ComputerVerifyStateResult, ComputerError> {
        if expect.is_empty() || expect.len() > 8 || expect.iter().any(|item| !item.is_object()) {
            return Err(CuaActionError::new(
                "verify_state needs one to eight object predicates.",
                CuaEffect::NotDispatched,
            )
            .code("invalid_arguments")
            .into());
        }
        let resolved = self.target(Some(window_id), true).await?;
        let mut args = Self::window_args(resolved.pid, resolved.window_id);
        args.insert("expect".into(), Value::Array(expect.to_vec()));
        let data = self.call_structured("verify_state", args, false).await?;
        let status = match data.get("status").and_then(Value::as_str) {
            Some("satisfied") => ComputerVerifyStatus::Satisfied,
            Some("unsatisfied") => ComputerVerifyStatus::Unsatisfied,
            _ => ComputerVerifyStatus::Unknown,
        };
        let count = |key: &str| {
            let value = number(data.get(key));
            if value.is_finite() {
                value.trunc().max(0.0) as u64
            } else {
                0
            }
        };
        Ok(ComputerVerifyStateResult {
            status,
            stable: data.get("stable") == Some(&Value::Bool(true)),
            samples: count("samples"),
            elapsed_ms: count("elapsed_ms"),
            predicates: data
                .get("predicates")
                .and_then(Value::as_array)
                .map(|predicates| predicates.iter().take(8).cloned().collect())
                .unwrap_or_default(),
        })
    }

    async fn kill_app(
        self: &Arc<Self>,
        pid: u32,
    ) -> Result<ComputerBackendActionResult, ComputerError> {
        if pid == 0 {
            return Err(CuaActionError::new(
                "kill_app needs a positive integer pid.",
                CuaEffect::NotDispatched,
            )
            .code("invalid_arguments")
            .into());
        }
        let mut args = Map::new();
        args.insert("pid".into(), json!(pid));
        self.call("kill_app", args, true, true).await?;
        {
            let mut state = self.state();
            state.snapshot_at = None;
            let prefix = format!("cua:{pid}:");
            state
                .observed_geometry
                .retain(|window_id, _| !window_id.starts_with(&prefix));
        }
        let mut gone = false;
        for attempt in 0..4 {
            if gone {
                break;
            }
            if attempt > 0 {
                tokio::time::sleep(KILL_POLL_DELAY).await;
            }
            gone = !self
                .list_apps()
                .await?
                .iter()
                .any(|app| app.pid == pid && app.running);
        }
        Ok(ComputerBackendActionResult {
            delivery_path: Some("cua-process_signal-background".into()),
            verified: Some(if gone {
                ComputerDeliveryVerification::Confirmed
            } else {
                ComputerDeliveryVerification::Unconfirmed
            }),
            effect: Some(if gone {
                CuaEffect::Verified
            } else {
                CuaEffect::DispatchedUnknown
            }),
            ..ComputerBackendActionResult::default()
        })
    }

    async fn zoom_window(
        self: &Arc<Self>,
        window_id: &str,
        region: ComputerRect,
    ) -> Result<ComputerZoomResult, ComputerError> {
        if !(region.x.is_finite()
            && region.y.is_finite()
            && region.width.is_finite()
            && region.height.is_finite())
            || region.width <= 0.0
            || region.height <= 0.0
        {
            return Err(CuaActionError::new(
                "The zoom region needs finite geometry and positive size.",
                CuaEffect::NotDispatched,
            )
            .code("invalid_geometry")
            .into());
        }
        let resolved = self.target(Some(window_id), true).await?;
        let bounds = resolved.window.bounds.unwrap_or(ComputerRect {
            x: 0.0,
            y: 0.0,
            width: 0.0,
            height: 0.0,
        });
        if region.x < 0.0
            || region.y < 0.0
            || region.x + region.width > bounds.width
            || region.y + region.height > bounds.height
        {
            return Err(CuaActionError::new(
                "The zoom region lies outside the target window.",
                CuaEffect::NotDispatched,
            )
            .code("invalid_geometry")
            .into());
        }
        let state = async {
            let mut args = Self::window_args(resolved.pid, resolved.window_id);
            args.insert("include_accessibility_tree".into(), Value::Bool(false));
            args.insert("include_screenshot".into(), Value::Bool(true));
            let state = self.call("get_window_state", args, false, true).await?;
            Self::assert_observed_window(&state, resolved.pid, resolved.window_id)?;
            Ok::<_, ComputerError>(state)
        }
        .await;
        let state = match state {
            Ok(state) => state,
            Err(error) => {
                if !is_retained_capture(&error) {
                    self.mark_capture_failed(&error);
                }
                return Err(error);
            }
        };
        let state_data = state.structured_content.clone().unwrap_or_default();
        let fresh_bounds = match state_data.get("window_bounds") {
            Some(value) if !value.is_null() => optional_rect(Some(value)),
            _ => None,
        };
        let Some(fresh_bounds) = fresh_bounds.filter(|fresh| same_rect(fresh, &bounds)) else {
            return Err(CuaActionError::new(
                "The target window moved before the zoom capture.",
                CuaEffect::NotDispatched,
            )
            .code("stale_target")
            .into());
        };
        let reported_scale = number(state_data.get("screenshot_scale"));
        let fallback = self.screenshot(&state, Some(fresh_bounds))?;
        let scale = if reported_scale.is_finite() && reported_scale > 0.0 {
            reported_scale
        } else {
            fallback.scale.unwrap_or(0.0)
        };
        if scale.is_nan() || scale <= 0.0 {
            return Err(CuaActionError::new(
                "Cua could not establish the window's screenshot scale.",
                CuaEffect::NotDispatched,
            )
            .code("capture_unavailable")
            .into());
        }
        let mut args = Self::window_args(resolved.pid, resolved.window_id);
        args.insert("x1".into(), num(region.x * scale));
        args.insert("y1".into(), num(region.y * scale));
        args.insert("x2".into(), num((region.x + region.width) * scale));
        args.insert("y2".into(), num((region.y + region.height) * scale));
        let result = self.call("zoom", args, false, true).await?;
        let image = result
            .content
            .as_deref()
            .unwrap_or_default()
            .iter()
            .find(|content| {
                content.kind == "image"
                    && content.mime_type.as_deref() == Some("image/jpeg")
                    && content.data.as_deref().is_some_and(|data| !data.is_empty())
            })
            .and_then(|content| content.data.clone());
        let data = result.structured_content.unwrap_or_default();
        let Some(image) = image else {
            return Err(CuaActionError::new(
                "Cua returned no zoom image.",
                CuaEffect::NotDispatched,
            )
            .code("capture_unavailable")
            .into());
        };
        let bytes = BASE64.decode(image.as_bytes()).unwrap_or_default();
        let (width, height) = match jpeg_dimensions(&bytes) {
            Some(dimensions) => (f64::from(dimensions.width), f64::from(dimensions.height)),
            None => {
                let dimension = |key: &str| {
                    let value = number(data.get(key));
                    if value.is_finite() {
                        value.trunc()
                    } else {
                        0.0
                    }
                };
                (dimension("width"), dimension("height"))
            }
        };
        if width <= 0.0 || height <= 0.0 {
            return Err(CuaActionError::new(
                "Cua returned a zoom image without readable dimensions.",
                CuaEffect::NotDispatched,
            )
            .code("invalid_response")
            .into());
        }
        Ok(ComputerZoomResult {
            mime_type: ComputerJpegMimeType::Jpeg,
            width: width as u32,
            height: height as u32,
            size_bytes: bytes.len() as u64,
            bytes_base64: image,
            window_id: Some(resolved.window.id),
            captured_at: now_iso(),
        })
    }

    async fn get_accessibility_tree(
        self: &Arc<Self>,
        window_id: Option<&str>,
    ) -> Result<ComputerAccessibilityTreeSnapshot, ComputerError> {
        let scoped_pid = match window_id {
            Some(window_id) => Some(self.target(Some(window_id), true).await?.pid),
            None => None,
        };
        let data = self
            .call_structured("get_accessibility_tree", Map::new(), false)
            .await?;
        let (Some(app_rows), Some(window_rows)) = (
            data.get("apps").and_then(Value::as_array),
            data.get("windows").and_then(Value::as_array),
        ) else {
            return Err(CuaActionError::new(
                "Cua returned an invalid desktop inventory.",
                CuaEffect::NotDispatched,
            )
            .code("invalid_response")
            .into());
        };
        let mut apps = Vec::new();
        for value in app_rows {
            let row = record(Some(value));
            let name = text1024(row.get("name"));
            let Some(pid) = positive_id(number(row.get("pid"))) else {
                continue;
            };
            if name.is_empty() || scoped_pid.is_some_and(|scoped| scoped != pid) {
                continue;
            }
            let Ok(pid) = u32::try_from(pid) else {
                continue;
            };
            let bundle_id = text(row.get("bundle_id"), 512);
            apps.push(ComputerAccessibilityTreeApp {
                pid,
                name,
                bundle_id: (!bundle_id.is_empty()).then_some(bundle_id),
            });
        }
        let mut windows = Vec::new();
        for value in window_rows {
            let row = record(Some(value));
            let (Some(pid), Some(window)) = (
                positive_id(number(row.get("pid"))),
                positive_id(number(row.get("window_id"))),
            ) else {
                continue;
            };
            if scoped_pid.is_some_and(|scoped| scoped != pid) {
                continue;
            }
            let Ok(pid32) = u32::try_from(pid) else {
                continue;
            };
            let app_name = text1024(row.get("app_name"));
            let z_index = safe_integer(number(row.get("z_index"))).filter(|z| *z >= 0);
            windows.push(ComputerAccessibilityTreeWindow {
                id: cua_window_id(pid, window),
                pid: pid32,
                app_name: (!app_name.is_empty()).then_some(app_name),
                title: text1024(row.get("title")),
                bounds: optional_rect(row.get("bounds")),
                on_screen: row.get("is_on_screen").and_then(Value::as_bool),
                z_index: z_index.map(|z| z as u64),
            });
        }
        let truncated = apps.len() > 1_024 || windows.len() > COMPUTER_WINDOW_LIST_MAX_LENGTH;
        apps.truncate(1_024);
        windows.truncate(COMPUTER_WINDOW_LIST_MAX_LENGTH);
        Ok(ComputerAccessibilityTreeSnapshot {
            apps,
            windows,
            truncated,
        })
    }

    async fn get_cursor_position(
        self: &Arc<Self>,
        window_id: Option<&str>,
    ) -> Result<ComputerCursorReading, ComputerError> {
        let window = match window_id {
            Some(window_id) => Some(self.target(Some(window_id), true).await?.window),
            None => None,
        };
        let data = self
            .call_structured("get_cursor_position", Map::new(), false)
            .await?;
        let x = number(data.get("x"));
        let y = number(data.get("y"));
        if !x.is_finite() || !y.is_finite() {
            return Err(CuaActionError::new(
                "Cua returned no cursor position.",
                CuaEffect::NotDispatched,
            )
            .code("invalid_response")
            .into());
        }
        let bounds = window.as_ref().and_then(|window| window.bounds);
        Ok(ComputerCursorReading {
            x,
            y,
            captured_at: now_iso(),
            window_id: window.map(|window| window.id),
            inside_window: bounds.map(|bounds| {
                x >= bounds.x
                    && x < bounds.x + bounds.width
                    && y >= bounds.y
                    && y < bounds.y + bounds.height
            }),
        })
    }

    async fn capture_still(self: &Arc<Self>) -> Result<Option<Vec<u8>>, ComputerError> {
        let target = {
            let state = self.state();
            if state.disposed {
                return Ok(None);
            }
            state.still_target.clone()
        };
        match target {
            None => Ok(None),
            Some(StillTarget::Window { window_id }) => {
                self.capture_window_still(&window_id).await.map(Some)
            }
            Some(StillTarget::Browser {
                target_id,
                tab_id,
                task,
            }) => self.capture_browser_still(target_id, tab_id, task).await,
        }
    }

    async fn capture_window_still(
        self: &Arc<Self>,
        window_id: &str,
    ) -> Result<Vec<u8>, ComputerError> {
        let resolved = self.target(Some(window_id), true).await?;
        let mut args = Self::window_args(resolved.pid, resolved.window_id);
        args.insert("include_screenshot".into(), Value::Bool(true));
        args.insert("include_accessibility_tree".into(), Value::Bool(false));
        args.insert("max_dimension".into(), json!(1536));
        let result = self.call("get_window_state", args, false, false).await?;
        let screenshot = self.screenshot(&result, None)?;
        Ok(BASE64
            .decode(screenshot.bytes_base64.as_bytes())
            .unwrap_or_default())
    }

    async fn capture_browser_still(
        self: &Arc<Self>,
        target_id: String,
        tab_id: Option<String>,
        task: CuaComputerTask,
    ) -> Result<Option<Vec<u8>>, ComputerError> {
        let mut args = Map::new();
        args.insert("target_id".into(), json!(target_id));
        if let Some(tab_id) = tab_id {
            args.insert("tab_id".into(), json!(tab_id));
        }
        args.insert("include_screenshot".into(), Value::Bool(true));
        args.insert("screenshot_only".into(), Value::Bool(true));
        let call = Self::host_call(HostRequest {
            name: Some("get_browser_state".into()),
            args: Value::Object(args),
            task: Some(task),
            ..HostRequest::new(ComputerHostMethod::Call)
        });
        let reply = self
            .request(
                call,
                None,
                false,
                Duration::from_millis(CUA_BROWSER_STILL_TIMEOUT_MS),
            )
            .await?;
        if !reply.ok {
            return Ok(None);
        }
        let image = reply
            .result
            .and_then(|result| result.content)
            .unwrap_or_default()
            .into_iter()
            .find(|part| {
                part.kind == "image" && part.data.as_deref().is_some_and(|data| !data.is_empty())
            })
            .and_then(|part| part.data);
        Ok(image.map(|data| BASE64.decode(data.as_bytes()).unwrap_or_default()))
    }

    fn note_browser_still_target(
        &self,
        args: &Map<String, Value>,
        result: &CuaToolResult,
        task: &CuaComputerTask,
    ) {
        let structured = result.structured_content.clone().unwrap_or_default();
        let target_id = match text1024(structured.get("target_id")) {
            id if !id.is_empty() => id,
            _ => text1024(args.get("target_id")),
        };
        if target_id.is_empty() {
            return;
        }
        let resolved = match text1024(args.get("tab_id")) {
            tab if !tab.is_empty() => Some(tab),
            _ => resolvable_still_tab(structured.get("tabs")),
        };
        let mut state = self.state();
        let prior = match &state.still_target {
            Some(StillTarget::Browser {
                target_id: prior_target,
                tab_id,
                ..
            }) if *prior_target == target_id => tab_id.clone(),
            _ => None,
        };
        state.still_target = Some(StillTarget::Browser {
            target_id,
            tab_id: resolved.or(prior),
            task: task.clone(),
        });
    }

    fn task_matches(owned: &CuaComputerTask, thread_id: &str, turn_id: Option<&str>) -> bool {
        owned.thread_id == thread_id
            && turn_id.is_none_or(|turn| owned.turn_id.as_deref() == Some(turn))
    }

    async fn stop_input(
        self: &Arc<Self>,
        task: Option<&CuaComputerTask>,
    ) -> Result<(), ComputerError> {
        {
            let mut state = self.state();
            match task {
                Some(task) => {
                    let turn = task.turn_id.as_deref().filter(|turn| !turn.is_empty());
                    state
                        .preview_tasks
                        .retain(|(_, owned)| !Self::task_matches(owned, &task.thread_id, turn));
                    let clear = match &state.still_target {
                        Some(StillTarget::Window { .. }) => true,
                        Some(StillTarget::Browser { task: owned, .. }) => {
                            Self::task_matches(owned, &task.thread_id, turn)
                        }
                        None => false,
                    };
                    if clear {
                        state.still_target = None;
                    }
                }
                None => {
                    state.preview_tasks.clear();
                    state.still_target = None;
                }
            }
        }
        let call = Self::host_call(HostRequest {
            task: task.map(|task| CuaComputerTask {
                thread_id: task.thread_id.clone(),
                turn_id: task.turn_id.clone(),
                label: None,
            }),
            ..HostRequest::new(ComputerHostMethod::Stop)
        });
        let reply = self
            .request(
                call,
                None,
                false,
                Duration::from_millis(CUA_DEFAULT_TIMEOUT_MS),
            )
            .await?;
        self.observe_desktop_interruption(&reply);
        if !reply.ok {
            return Err(CuaActionError::new(
                reply
                    .error
                    .unwrap_or_else(|| "Computer stop was not acknowledged.".into()),
                CuaEffect::DispatchedUnknown,
            )
            .into());
        }
        let mut state = self.state();
        state.observed_geometry.clear();
        state.snapshot_at = None;
        Ok(())
    }

    async fn end_task(
        self: &Arc<Self>,
        thread_id: &str,
        turn_id: Option<&str>,
    ) -> Result<(), ComputerError> {
        let matches: Vec<String> = {
            let state = self.state();
            if state.disposed {
                return Ok(());
            }
            state
                .preview_tasks
                .iter()
                .filter(|(_, task)| Self::task_matches(task, thread_id, turn_id))
                .map(|(key, _)| key.clone())
                .collect()
        };
        if matches.is_empty() {
            return Ok(());
        }
        let call = Self::host_call(HostRequest {
            task: Some(CuaComputerTask {
                thread_id: thread_id.to_owned(),
                turn_id: turn_id.filter(|turn| !turn.is_empty()).map(str::to_owned),
                label: None,
            }),
            ..HostRequest::new(ComputerHostMethod::EndTask)
        });
        let reply = self
            .request(
                call,
                None,
                false,
                Duration::from_millis(CUA_DEFAULT_TIMEOUT_MS),
            )
            .await?;
        self.observe_desktop_interruption(&reply);
        if !reply.ok {
            return Err(ComputerError::Other(
                reply
                    .error
                    .unwrap_or_else(|| "Computer preview did not stop.".into()),
            ));
        }
        let mut state = self.state();
        state
            .preview_tasks
            .retain(|(key, _)| !matches.contains(key));
        if let Some(StillTarget::Browser { task, .. }) = &state.still_target
            && Self::task_matches(task, thread_id, turn_id)
        {
            state.still_target = None;
        }
        state.observed_geometry.clear();
        Ok(())
    }

    async fn engage_shield(
        self: &Arc<Self>,
        target: &ComputerShieldTarget,
    ) -> Result<String, ComputerError> {
        if self.state().disposed {
            return Err(CuaActionError::new(
                "Open this session in the Zeron macOS desktop app to use Computer.",
                CuaEffect::NotDispatched,
            )
            .code("gui_host_required")
            .into());
        }
        assert_desktop_operation_active()?;
        let Some((pid, window)) = parse_cua_window_id(&target.window_id) else {
            return Err(CuaActionError::new(
                format!(
                    "The activation shield cannot cover window {}: it is not a native window id.",
                    target.window_id
                ),
                CuaEffect::NotDispatched,
            )
            .code("invalid_target")
            .into());
        };
        let task = current_computer_task();
        if let Some(task) = &task {
            self.track_preview_task(task);
        }
        let call = Self::host_call(HostRequest {
            task,
            args: json!({
                "action": "engage",
                "shield_id": target.shield_id,
                "frame": rect_json(&target.frame),
                "window_id": window,
                "pid": pid,
                "label": target.label,
            }),
            ..HostRequest::new(ComputerHostMethod::Shield)
        });
        let reply = timed_computer_leg(
            "host",
            self.request(
                call,
                desktop_operation_signal(),
                true,
                Duration::from_millis(CUA_SHIELD_TIMEOUT_MS),
            ),
        )
        .await
        .map_err(|error| {
            ComputerError::from(
                CuaActionError::new(error.message, error.effect).code("mask_unavailable"),
            )
        })?;
        if !reply.ok {
            return Err(CuaActionError::new(
                reply
                    .error
                    .unwrap_or_else(|| "The activation shield was refused.".into()),
                CuaEffect::NotDispatched,
            )
            .code("mask_unavailable")
            .into());
        }
        Ok(target.shield_id.clone())
    }

    async fn release_shields(
        self: &Arc<Self>,
        args: Value,
        refusal: &str,
    ) -> Result<(), ComputerError> {
        if self.state().disposed {
            return Ok(());
        }
        let call = Self::host_call(HostRequest {
            args,
            ..HostRequest::new(ComputerHostMethod::Shield)
        });
        let reply = self
            .request(
                call,
                None,
                false,
                Duration::from_millis(CUA_SHIELD_TIMEOUT_MS),
            )
            .await?;
        if !reply.ok {
            return Err(CuaActionError::new(
                reply.error.unwrap_or_else(|| refusal.to_owned()),
                CuaEffect::NotDispatched,
            )
            .into());
        }
        Ok(())
    }

    async fn browser_call(
        self: &Arc<Self>,
        call: ComputerBrowserCall,
    ) -> Result<ComputerBrowserCallResult, ComputerError> {
        if self.state().disposed {
            return Err(
                CuaActionError::new(GUI_HOST_REQUIRED, CuaEffect::NotDispatched)
                    .code("gui_host_required")
                    .into(),
            );
        }
        assert_desktop_operation_active()?;
        let task = CuaComputerTask {
            thread_id: call.task.thread_id.clone(),
            turn_id: call.task.turn_id.clone().filter(|turn| !turn.is_empty()),
            label: call.task.label.clone().filter(|label| !label.is_empty()),
        };
        self.track_preview_task(&task);
        let request = Self::host_call(HostRequest {
            name: Some(call.name.clone()),
            args: Value::Object(call.args.clone()),
            task: Some(task.clone()),
            delivery_mode: Some(desktop_delivery_mode()),
            model_observation: (call.name == "get_browser_state")
                .then(is_model_desktop_observation_active),
            ..HostRequest::new(ComputerHostMethod::Call)
        });
        let reply = timed_computer_leg(
            "host",
            self.request(
                request,
                call.signal.clone(),
                call.mutation,
                Duration::from_millis(CUA_HOST_CALL_TIMEOUT_MS),
            ),
        )
        .await
        .map_err(transport_error)?;
        self.observe_desktop_interruption(&reply);
        if !reply.ok {
            return Err(CuaActionError::new(
                reply.error.unwrap_or_else(|| "Cua host failed.".into()),
                reply.effect.unwrap_or(CuaEffect::NotDispatched),
            )
            .into());
        }
        let result = reply.result.unwrap_or_default();
        self.note_browser_still_target(&call.args, &result, &task);
        Ok(result)
    }

    async fn end_browser_thread(self: &Arc<Self>, thread_id: &str) -> Result<(), ComputerError> {
        if self.state().disposed {
            return Ok(());
        }
        let call = Self::host_call(HostRequest {
            task: Some(CuaComputerTask {
                thread_id: thread_id.to_owned(),
                turn_id: None,
                label: None,
            }),
            ..HostRequest::new(ComputerHostMethod::EndBrowserThread)
        });
        let reply = self
            .request(
                call,
                None,
                false,
                Duration::from_millis(CUA_DEFAULT_TIMEOUT_MS),
            )
            .await?;
        self.observe_desktop_interruption(&reply);
        if !reply.ok {
            return Err(ComputerError::Other(reply.error.unwrap_or_else(|| {
                "Browser session teardown was not acknowledged.".into()
            })));
        }
        Ok(())
    }

    async fn dispose(self: &Arc<Self>) {
        let _ = self.stop_input(None).await;
        self.state().disposed = true;
        self.listeners.clear();
    }
}

#[derive(Debug, Clone)]
struct WebField {
    token: String,
    index: f64,
    value: Option<String>,
}

#[async_trait]
impl ComputerBrowserBackend for CuaComputerBackend {
    async fn call(
        &self,
        call: ComputerBrowserCall,
    ) -> Result<ComputerBrowserCallResult, ComputerError> {
        self.inner.browser_call(call).await
    }

    async fn end_thread(&self, thread_id: &str) -> Result<(), ComputerError> {
        self.inner.end_browser_thread(thread_id).await
    }
}

#[async_trait]
impl ComputerBackend for CuaComputerBackend {
    fn agent_dialect(&self) -> ComputerAgentDialect {
        if self.inner.is_darwin() {
            ComputerAgentDialect::Macos
        } else {
            ComputerAgentDialect::Linux
        }
    }

    fn computer_id(&self) -> &str {
        DEFAULT_COMPUTER_ID
    }

    fn supports(&self, _method: ComputerBackendMethod) -> bool {
        true
    }

    async fn probe_availability(&self) -> Result<ComputerAvailability, ComputerError> {
        Ok(self.inner.probe_availability().await)
    }

    async fn availability(&self, refresh: bool) -> Result<ComputerAvailability, ComputerError> {
        Ok(self.inner.availability(refresh).await)
    }

    async fn provision(&self) -> Result<String, ComputerError> {
        self.inner.provision().await
    }

    fn health(&self) -> ComputerHealth {
        self.inner.state().health.clone()
    }

    fn capabilities(&self) -> ComputerCapabilities {
        self.inner.capabilities()
    }

    async fn missing_permissions(&self) -> Vec<ComputerPermission> {
        self.inner.state().permissions.clone()
    }

    fn build_signature(&self) -> Option<ComputerBuildSignature> {
        Some(ComputerBuildSignature::Unknown)
    }

    fn driver_native_revision(&self) -> Option<u64> {
        self.inner.native_revision()
    }

    async fn list_windows(&self) -> Result<Vec<ComputerWindow>, ComputerError> {
        self.inner.refresh(false, false).await?;
        Ok(self.inner.state().windows.clone())
    }

    async fn list_spaces(&self) -> Result<ComputerSpaceInventory, ComputerError> {
        self.inner.list_spaces().await
    }

    async fn get_screen_size(&self) -> Result<ComputerScreenSize, ComputerError> {
        self.inner.refresh(false, false).await?;
        Ok(self.inner.state().size)
    }

    async fn get_state(
        &self,
        options: ComputerStateOptions,
    ) -> Result<ComputerState, ComputerError> {
        self.inner.get_state(options).await
    }

    async fn capture_screenshot(
        &self,
        request: ComputerCaptureRequest,
    ) -> Result<ComputerScreenshot, ComputerError> {
        self.inner.capture_screenshot(request).await
    }

    async fn focus_window(&self, window_id: &str) -> Result<(), ComputerError> {
        self.inner.focus_window(window_id).await
    }

    async fn raise_window(&self, window_id: &str) -> Result<(), ComputerError> {
        self.inner.raise_window(window_id).await
    }

    async fn clear_focus_window(&self) -> Result<(), ComputerError> {
        self.inner.clear_focus_window();
        Ok(())
    }

    async fn launch_app(
        &self,
        app: &str,
        args: &[String],
        options: ComputerLaunchOptions,
    ) -> Result<ComputerLaunchAppResult, ComputerError> {
        self.inner.launch_app(app, args, options).await
    }

    async fn check_input_ready(&self, window_id: &str) -> Result<(), ComputerError> {
        self.inner.check_input_ready(window_id).await
    }

    async fn wait_for_settle(
        &self,
        options: ComputerSettleOptions,
    ) -> Result<ComputerSettleOutcome, ComputerError> {
        self.inner.wait_for_settle(options).await
    }

    async fn list_apps(&self) -> Result<Vec<ComputerApp>, ComputerError> {
        self.inner.list_apps().await
    }

    async fn set_window_frame(
        &self,
        window_id: &str,
        frame: ComputerRect,
    ) -> Result<ComputerBackendActionResult, ComputerError> {
        self.inner.set_window_frame(window_id, frame).await
    }

    async fn invoke_menu(
        &self,
        target: &ComputerMenuBackendTarget,
        path: &[String],
    ) -> Result<ComputerBackendActionResult, ComputerError> {
        self.inner.invoke_menu(target, path).await
    }

    async fn verify_state(
        &self,
        window_id: &str,
        expect: &[Value],
    ) -> Result<ComputerVerifyStateResult, ComputerError> {
        self.inner.verify_state(window_id, expect).await
    }

    async fn zoom_window(
        &self,
        window_id: &str,
        region: ComputerRect,
    ) -> Result<ComputerZoomResult, ComputerError> {
        self.inner.zoom_window(window_id, region).await
    }

    async fn get_accessibility_tree(
        &self,
        window_id: Option<&str>,
    ) -> Result<ComputerAccessibilityTreeSnapshot, ComputerError> {
        self.inner.get_accessibility_tree(window_id).await
    }

    async fn get_cursor_position(
        &self,
        window_id: Option<&str>,
    ) -> Result<ComputerCursorReading, ComputerError> {
        self.inner.get_cursor_position(window_id).await
    }

    async fn set_window_minimized(
        &self,
        window_id: &str,
        minimized: bool,
    ) -> Result<ComputerBackendActionResult, ComputerError> {
        self.inner.set_window_minimized(window_id, minimized).await
    }

    async fn set_app_visibility(
        &self,
        pid: u32,
        hidden: bool,
    ) -> Result<ComputerBackendActionResult, ComputerError> {
        self.inner.set_app_visibility(pid, hidden).await
    }

    async fn kill_app(&self, pid: u32) -> Result<ComputerBackendActionResult, ComputerError> {
        self.inner.kill_app(pid).await
    }

    fn supports_action(&self, target: &ComputerResolvedTarget, action: &str) -> bool {
        self.inner.supports_action(target, action)
    }

    async fn click(
        &self,
        point: ComputerPoint,
        window_id: Option<&str>,
        modifiers: &[ComputerInputModifier],
    ) -> Result<ComputerBackendActionResult, ComputerError> {
        self.inner
            .click_gesture(point, window_id, modifiers, Some(1), false, false)
            .await
    }

    async fn double_click(
        &self,
        point: ComputerPoint,
        window_id: Option<&str>,
        modifiers: &[ComputerInputModifier],
    ) -> Result<ComputerBackendActionResult, ComputerError> {
        self.inner
            .click_gesture(point, window_id, modifiers, Some(2), false, true)
            .await
    }

    async fn triple_click(
        &self,
        point: ComputerPoint,
        window_id: Option<&str>,
        modifiers: &[ComputerInputModifier],
    ) -> Result<ComputerBackendActionResult, ComputerError> {
        self.inner
            .click_gesture(point, window_id, modifiers, Some(3), false, true)
            .await
    }

    async fn right_click(
        &self,
        point: ComputerPoint,
        window_id: Option<&str>,
        modifiers: &[ComputerInputModifier],
    ) -> Result<ComputerBackendActionResult, ComputerError> {
        self.inner
            .click_gesture(point, window_id, modifiers, None, true, true)
            .await
    }

    async fn move_cursor(
        &self,
        point: ComputerPoint,
        window_id: Option<&str>,
    ) -> Result<ComputerBackendActionResult, ComputerError> {
        self.inner.move_cursor(point, window_id).await
    }

    async fn drag(
        &self,
        from: ComputerPoint,
        to: ComputerPoint,
        duration_ms: f64,
        window_id: Option<&str>,
    ) -> Result<ComputerBackendActionResult, ComputerError> {
        self.inner.drag(from, to, duration_ms, window_id).await
    }

    async fn scroll(
        &self,
        point: Option<ComputerPoint>,
        delta_x: f64,
        delta_y: f64,
        window_id: Option<&str>,
        modifiers: &[ComputerInputModifier],
        target: Option<&ComputerResolvedTarget>,
        prepared_bounds: Option<ComputerRect>,
    ) -> Result<ComputerBackendActionResult, ComputerError> {
        self.inner
            .scroll(
                point,
                delta_x,
                delta_y,
                window_id,
                modifiers,
                target,
                prepared_bounds,
            )
            .await
    }

    fn focus_neutral_semantic_text(&self) -> bool {
        self.inner.focus_neutral_semantic_text()
    }

    fn exact_target_background_input(&self) -> bool {
        self.inner.exact_target_background_input()
    }

    async fn type_text(
        &self,
        text: &str,
        window_id: Option<&str>,
        target: Option<&ComputerResolvedTarget>,
    ) -> Result<ComputerBackendActionResult, ComputerError> {
        self.inner.type_text(text, window_id, target).await
    }

    async fn press_key(
        &self,
        key: &str,
        window_id: Option<&str>,
        target: Option<&ComputerResolvedTarget>,
    ) -> Result<ComputerBackendActionResult, ComputerError> {
        self.inner.press_key(key, window_id, target).await
    }

    async fn hotkey(
        &self,
        keys: &[String],
        window_id: Option<&str>,
        target: Option<&ComputerResolvedTarget>,
    ) -> Result<ComputerBackendActionResult, ComputerError> {
        self.inner.hotkey(keys, window_id, target).await
    }

    async fn read_clipboard(&self) -> Result<String, ComputerError> {
        self.inner.read_clipboard().await
    }

    async fn write_clipboard(&self, text: &str) -> Result<(), ComputerError> {
        self.inner.write_clipboard(text).await
    }

    async fn write_clipboard_html(&self, text: &str, html: &str) -> Result<(), ComputerError> {
        self.inner.write_clipboard_html(text, html).await
    }

    async fn set_value(
        &self,
        target: &ComputerResolvedTarget,
        value: &str,
    ) -> Result<ComputerBackendActionResult, ComputerError> {
        self.inner.set_value(target, value).await
    }

    async fn perform_action(
        &self,
        target: &ComputerResolvedTarget,
        action: &str,
    ) -> Result<ComputerBackendActionResult, ComputerError> {
        self.inner.perform_action(target, action).await
    }

    async fn select_text(
        &self,
        target: &ComputerResolvedTarget,
        range: ComputerTextRange,
    ) -> Result<ComputerBackendActionResult, ComputerError> {
        self.inner.select_text(target, range).await
    }

    fn on_event(&self, listener: ComputerBackendEventListener) -> ComputerBackendSubscription {
        self.inner.listeners.subscribe(listener)
    }

    async fn attach_stream(&self, listener: ComputerFrameListener) -> Result<(), ComputerError> {
        self.stills.attach(listener).await;
        Ok(())
    }

    async fn detach_stream(&self) -> Result<(), ComputerError> {
        self.stills.detach().await;
        Ok(())
    }

    async fn request_keyframe(&self) -> Result<(), ComputerError> {
        self.stills.request_keyframe().await;
        Ok(())
    }

    async fn engage_shield(&self, target: &ComputerShieldTarget) -> Result<String, ComputerError> {
        self.inner.engage_shield(target).await
    }

    async fn release_shield(&self, shield_id: &str) -> Result<(), ComputerError> {
        self.inner
            .release_shields(
                json!({ "action": "release", "shield_id": shield_id }),
                "The activation shield did not release.",
            )
            .await
    }

    async fn release_all_shields(&self) -> Result<(), ComputerError> {
        self.inner
            .release_shields(
                json!({ "action": "release_all" }),
                "The activation shields did not release.",
            )
            .await
    }

    async fn stop_input(&self, task: Option<&CuaComputerTask>) -> Result<(), ComputerError> {
        self.inner.stop_input(task).await
    }

    async fn end_task(&self, thread_id: &str, turn_id: Option<&str>) -> Result<(), ComputerError> {
        self.inner.end_task(thread_id, turn_id).await
    }

    fn browser(&self) -> Option<&dyn ComputerBrowserBackend> {
        Some(self)
    }

    fn native_element_identity(&self, node: &ComputerUiNode) -> Option<String> {
        self.inner.node_element(node).map(|element| element.token)
    }

    fn latest_tree(&self, window_id: &str) -> Option<ComputerUiNode> {
        let state = self.inner.state();
        state
            .recent_trees
            .iter()
            .find(|(id, _)| id == window_id)
            .map(|(_, tree)| tree.root.clone())
    }

    async fn dispose(&self) {
        self.stills.detach().await;
        self.inner.dispose().await;
    }
}

fn is_retained_capture(error: &ComputerError) -> bool {
    matches!(
        error.code(),
        Some("off_space_capture_unverified" | "hidden_capture_unverified")
    )
}

#[cfg(test)]
pub(crate) mod tests;
