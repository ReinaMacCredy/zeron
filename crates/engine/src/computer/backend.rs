use std::sync::{Arc, Mutex, PoisonError, Weak};

use async_trait::async_trait;
use serde::Serialize;
use serde_json::{Map, Value};
use zeron_proto::computer::{
    COMPUTER_DELIVERY_PATH_MAX_LENGTH, COMPUTER_MESSAGE_MAX_LENGTH, ComputerAccessibilityTreeApp,
    ComputerAccessibilityTreeWindow, ComputerActionResult, ComputerApp, ComputerAvailability,
    ComputerBuildSignature, ComputerCapabilities, ComputerDelivery, ComputerDeliveryVerification,
    ComputerHealth, ComputerInputModifier, ComputerInputPause, ComputerLaunchAppResult,
    ComputerPermission, ComputerPoint, ComputerRect, ComputerScreenSize, ComputerScreenshot,
    ComputerScrollDelta, ComputerSpaceInventory, ComputerState, ComputerUiNode,
    ComputerVerifyStateResult, ComputerWindow, ComputerZoomResult, CuaActionDiagnostics,
    CuaComputerTask, CuaEffect, CuaToolResult, CuaTransportError,
};

use super::approval_gate::ComputerApprovalError;
use super::audit_history::ComputerAuditHistoryError;
use super::denylist::ComputerDenylistError;
use super::desktop_operation_queue::{DesktopOperationError, OperationSignal};
use super::element_identity::ObservedComputerTarget;
use super::geometry::ComputerGeometryError;
use super::ui_tree_targeting::ComputerTargetError;
use super::utf8_truncation::utf16_len;

pub const COMPUTER_AGENT_IMAGE_MAX_DIMENSION: u32 = 1_536;
pub const DEFAULT_COMPUTER_CAPTURE_MAX_DIMENSION: u32 = COMPUTER_AGENT_IMAGE_MAX_DIMENSION;
pub const COMPUTER_ACTION_OBSERVATION_MAX_DIMENSION: u32 = COMPUTER_AGENT_IMAGE_MAX_DIMENSION;
pub const MAX_COMPUTER_CAPTURE_MAX_DIMENSION: u32 = 16_384;
pub const MAX_COMPUTER_CLIPBOARD_BYTES: usize = 1024 * 1024;
pub const DEFAULT_COMPUTER_ID: &str = "desktop";

pub const NO_COMPUTER_CAPABILITIES: ComputerCapabilities = ComputerCapabilities {
    windows: false,
    window_bounds: false,
    stacking: false,
    capture: false,
    input: false,
    clipboard: false,
    focus: false,
    raise: false,
    ghost_cursor: false,
    visible_desktop: false,
};

pub fn assert_computer_clipboard_write_fits(text: &str) -> Result<(), ComputerBackendError> {
    let bytes = text.len();
    if bytes <= MAX_COMPUTER_CLIPBOARD_BYTES {
        return Ok(());
    }
    Err(ComputerBackendError::new(format!(
        "Clipboard text is {bytes} bytes, past the {MAX_COMPUTER_CLIPBOARD_BYTES} byte limit this tool writes."
    )))
}

#[derive(Debug, Clone, PartialEq)]
pub enum ComputerCaptureRequest {
    Window {
        window_id: String,
        max_dimension: Option<u32>,
    },
    Region {
        region: ComputerRect,
        max_dimension: Option<u32>,
    },
}

impl ComputerCaptureRequest {
    pub fn max_dimension(&self) -> Option<u32> {
        match self {
            Self::Window { max_dimension, .. } | Self::Region { max_dimension, .. } => {
                *max_dimension
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ComputerStreamFrame {
    pub sequence: u64,
    pub timestamp_ms: f64,
    pub keyframe: bool,
    pub codec_config: bool,
    pub data: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ComputerResolvedTarget {
    pub target: ObservedComputerTarget,
    pub point: ComputerPoint,
    pub node: ComputerUiNode,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct ComputerTextRange {
    pub start: u64,
    pub length: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ComputerMenuTarget {
    Window(String),
    App(String),
    Pid(u32),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(untagged)]
pub enum ComputerMenuBackendTarget {
    Window {
        #[serde(rename = "windowId")]
        window_id: String,
    },
    Pid {
        pid: u32,
    },
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct ComputerBackendActionResult {
    pub point: Option<ComputerPoint>,
    pub clamped_to: Option<ComputerPoint>,
    pub window_id: Option<String>,
    pub value: Option<String>,
    pub delivery_path: Option<String>,
    pub verified: Option<ComputerDeliveryVerification>,
    pub effect: Option<CuaEffect>,
    pub scroll_delta: Option<ComputerScrollDelta>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ComputerBackendEvent {
    WindowsChanged(Vec<ComputerWindow>),
    HealthChanged(ComputerHealth),
    CapabilitiesChanged(ComputerCapabilities),
    DesktopInterrupted { pauses: Vec<String> },
    Frame(ComputerStreamFrame),
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ComputerShieldTarget {
    pub shield_id: String,
    pub window_id: String,
    pub frame: ComputerRect,
    pub label: String,
}

#[derive(Debug, Clone)]
pub struct ComputerBrowserCall {
    pub name: String,
    pub args: Map<String, Value>,
    pub task: CuaComputerTask,
    pub mutation: bool,
    pub signal: Option<OperationSignal>,
}

pub type ComputerBrowserCallResult = CuaToolResult;

#[async_trait]
pub trait ComputerBrowserBackend: Send + Sync {
    async fn call(
        &self,
        call: ComputerBrowserCall,
    ) -> Result<ComputerBrowserCallResult, ComputerError>;

    async fn end_thread(&self, _thread_id: &str) -> Result<(), ComputerError> {
        Ok(())
    }
}

pub type ComputerFrameListener = Arc<dyn Fn(ComputerStreamFrame) + Send + Sync>;
pub type ComputerBackendEventListener = Arc<dyn Fn(&ComputerBackendEvent) + Send + Sync>;

#[derive(Debug, Clone, Default, PartialEq, thiserror::Error)]
#[error("{message}")]
pub struct ComputerBackendError {
    pub message: String,
    pub retryable: bool,
    pub dormant: bool,
    pub rejected_operation: Option<String>,
    pub setup_required: bool,
    pub input_pause: Option<ComputerInputPause>,
    pub control_revoked: bool,
    pub cua: Option<Box<CuaActionFailure>>,
}

impl ComputerBackendError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            ..Self::default()
        }
    }

    pub fn retryable(mut self, retryable: bool) -> Self {
        self.retryable = retryable;
        self
    }

    pub fn effect(&self) -> Option<CuaEffect> {
        self.cua.as_ref().map(|cua| cua.effect)
    }

    pub fn code(&self) -> Option<&str> {
        self.cua.as_ref().map(|cua| cua.code.as_str())
    }
}

impl From<ComputerGeometryError> for ComputerBackendError {
    fn from(error: ComputerGeometryError) -> Self {
        Self::new(error.0)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CuaErrorLayer {
    DriverHost,
    NativeDriver,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CuaActionFailure {
    pub effect: CuaEffect,
    pub code: String,
    pub diagnostics: Option<CuaActionDiagnostics>,
    pub layer: Option<CuaErrorLayer>,
    pub wait_seconds: Option<f64>,
}

pub const CUA_ACTION_FAILED_CODE: &str = "cua_action_failed";

#[derive(Debug, Clone, PartialEq)]
pub struct CuaActionError {
    pub message: String,
    pub effect: CuaEffect,
    pub code: String,
    pub input_pause: Option<ComputerInputPause>,
    pub diagnostics: Option<CuaActionDiagnostics>,
    pub layer: Option<CuaErrorLayer>,
    pub wait_seconds: Option<f64>,
}

impl CuaActionError {
    pub fn new(message: impl Into<String>, effect: CuaEffect) -> Self {
        Self {
            message: message.into(),
            effect,
            code: CUA_ACTION_FAILED_CODE.into(),
            input_pause: None,
            diagnostics: None,
            layer: None,
            wait_seconds: None,
        }
    }

    pub fn code(mut self, code: impl Into<String>) -> Self {
        self.code = code.into();
        self
    }

    pub fn input_pause(mut self, pause: Option<ComputerInputPause>) -> Self {
        self.input_pause = pause;
        self
    }
}

pub fn cua_effect_name(effect: CuaEffect) -> &'static str {
    match effect {
        CuaEffect::NotDispatched => "not-dispatched",
        CuaEffect::DispatchedUnknown => "dispatched-unknown",
        CuaEffect::Verified => "verified",
    }
}

impl From<CuaActionError> for ComputerBackendError {
    fn from(error: CuaActionError) -> Self {
        let keeps_pause =
            error.effect == CuaEffect::NotDispatched || error.code == "focus_restore_failed";
        Self {
            message: format!(
                "{} [effect={}; automatic replay is forbidden]",
                error.message,
                cua_effect_name(error.effect)
            ),
            input_pause: error.input_pause.filter(|_| keeps_pause),
            cua: Some(Box::new(CuaActionFailure {
                effect: error.effect,
                code: error.code,
                diagnostics: error.diagnostics,
                layer: error.layer,
                wait_seconds: error.wait_seconds,
            })),
            ..Self::default()
        }
    }
}

impl From<CuaActionError> for ComputerError {
    fn from(error: CuaActionError) -> Self {
        Self::Backend(error.into())
    }
}

impl From<ComputerGeometryError> for ComputerError {
    fn from(error: ComputerGeometryError) -> Self {
        Self::Backend(error.into())
    }
}

pub const COMPUTER_CONTROLLED_BY_OTHER_THREAD_CODE: &str = "computer_controlled_by_other_thread";

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error(
    "{}no input was sent. Do not retry this blocked action or switch tools to bypass the lease. Wait until that conversation's turn ends. Reading the desktop still works. Background actions on independently owned applications remain available.",
    if *.target_only {
        "This application or window is controlled by another conversation; "
    } else {
        "The shared pointer and focused keyboard are controlled by another conversation; "
    }
)]
pub struct ComputerLeaseError {
    pub target_only: bool,
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum ComputerError {
    #[error(transparent)]
    Backend(#[from] ComputerBackendError),
    #[error(transparent)]
    Lease(#[from] ComputerLeaseError),
    #[error(transparent)]
    Target(#[from] ComputerTargetError),
    #[error(transparent)]
    Operation(#[from] DesktopOperationError),
    #[error(transparent)]
    Approval(#[from] ComputerApprovalError),
    #[error(transparent)]
    Denylist(#[from] ComputerDenylistError),
    #[error(transparent)]
    AuditHistory(#[from] ComputerAuditHistoryError),
    #[error(transparent)]
    Transport(#[from] CuaTransportError),
    #[error("{0}")]
    Other(String),
}

impl ComputerError {
    pub fn backend(&self) -> Option<&ComputerBackendError> {
        match self {
            Self::Backend(error) => Some(error),
            _ => None,
        }
    }

    pub fn cua(&self) -> Option<&CuaActionFailure> {
        self.backend().and_then(|error| error.cua.as_deref())
    }

    pub fn effect(&self) -> Option<CuaEffect> {
        match self {
            Self::Transport(error) => Some(error.effect),
            _ => self.cua().map(|cua| cua.effect),
        }
    }

    pub fn code(&self) -> Option<&str> {
        match self {
            Self::Backend(error) => error.code(),
            Self::Lease(_) => Some(COMPUTER_CONTROLLED_BY_OTHER_THREAD_CODE),
            Self::Target(error) => Some(error.code.as_str()),
            Self::Approval(error) => error.code(),
            Self::Denylist(error) => Some(error.code()),
            _ => None,
        }
    }

    pub fn retryable(&self) -> bool {
        match self {
            Self::Backend(error) => error.retryable,
            Self::Operation(error) => error.retryable(),
            Self::Approval(error) => error.retryable(),
            _ => false,
        }
    }

    pub fn input_pause(&self) -> Option<&ComputerInputPause> {
        self.backend().and_then(|error| error.input_pause.as_ref())
    }

    pub fn setup_required(&self) -> bool {
        self.backend().is_some_and(|error| error.setup_required)
    }

    pub fn control_revoked(&self) -> bool {
        self.backend().is_some_and(|error| error.control_revoked)
    }

    pub fn dormant(&self) -> bool {
        self.backend().is_some_and(|error| error.dormant)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ComputerAgentDialect {
    Linux,
    Macos,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ComputerBackendMethod {
    Provision,
    ListSpaces,
    RaiseWindow,
    WaitForSettle,
    ListApps,
    SetWindowFrame,
    InvokeMenu,
    VerifyState,
    ZoomWindow,
    GetAccessibilityTree,
    GetCursorPosition,
    SetWindowMinimized,
    SetAppVisibility,
    KillApp,
    TripleClick,
    ReadClipboard,
    WriteClipboard,
    EngageShield,
    ReleaseShield,
    ReleaseAllShields,
    CheckInputReady,
    RequestKeyframe,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ComputerStateOptions {
    pub include_screenshot: bool,
    pub include_tree: bool,
    pub window_id: Option<String>,
    pub reuse_recent_tree: bool,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ComputerLaunchOptions {
    pub hidden: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ComputerSettleOptions {
    pub window_id: String,
    pub timeout_ms: f64,
    pub quiet_ms: f64,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ComputerSettleOutcome {
    pub settled: bool,
    pub waited_ms: f64,
    pub events_seen: Option<f64>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ComputerAccessibilityTreeSnapshot {
    pub apps: Vec<ComputerAccessibilityTreeApp>,
    pub windows: Vec<ComputerAccessibilityTreeWindow>,
    pub truncated: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ComputerCursorReading {
    pub x: f64,
    pub y: f64,
    pub captured_at: String,
    pub window_id: Option<String>,
    pub inside_window: Option<bool>,
}

type ListenerSlots = Mutex<Vec<(u64, ComputerBackendEventListener)>>;

#[derive(Default)]
pub struct ComputerBackendListeners {
    slots: Arc<ListenerSlots>,
    next: Mutex<u64>,
}

impl ComputerBackendListeners {
    pub fn subscribe(&self, listener: ComputerBackendEventListener) -> ComputerBackendSubscription {
        let id = {
            let mut next = self.next.lock().unwrap_or_else(PoisonError::into_inner);
            *next += 1;
            *next
        };
        self.slots
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((id, listener));
        ComputerBackendSubscription {
            slots: Arc::downgrade(&self.slots),
            id,
        }
    }

    pub fn emit(&self, event: &ComputerBackendEvent) {
        let listeners: Vec<ComputerBackendEventListener> = self
            .slots
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .map(|(_, listener)| listener.clone())
            .collect();
        for listener in listeners {
            listener(event);
        }
    }

    pub fn clear(&self) {
        self.slots
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clear();
    }

    pub fn len(&self) -> usize {
        self.slots
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[must_use = "dropping the subscription unsubscribes the listener"]
pub struct ComputerBackendSubscription {
    slots: Weak<ListenerSlots>,
    id: u64,
}

impl ComputerBackendSubscription {
    pub fn detached() -> Self {
        Self {
            slots: Weak::new(),
            id: 0,
        }
    }

    pub fn unsubscribe(self) {}
}

impl Drop for ComputerBackendSubscription {
    fn drop(&mut self) {
        if let Some(slots) = self.slots.upgrade() {
            slots
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .retain(|(id, _)| *id != self.id);
        }
    }
}

fn unsupported<T>(method: &str) -> Result<T, ComputerError> {
    Err(ComputerBackendError::new(format!("This backend does not implement {method}.")).into())
}

#[async_trait]
pub trait ComputerBackend: Send + Sync {
    fn agent_dialect(&self) -> ComputerAgentDialect {
        ComputerAgentDialect::Linux
    }

    fn computer_id(&self) -> &str;

    fn supports(&self, _method: ComputerBackendMethod) -> bool {
        false
    }

    async fn probe_availability(&self) -> Result<ComputerAvailability, ComputerError>;

    async fn availability(&self, refresh: bool) -> Result<ComputerAvailability, ComputerError>;

    async fn provision(&self) -> Result<String, ComputerError> {
        unsupported("provision")
    }

    fn health(&self) -> ComputerHealth;

    fn capabilities(&self) -> ComputerCapabilities;

    async fn missing_permissions(&self) -> Vec<ComputerPermission> {
        Vec::new()
    }

    fn build_signature(&self) -> Option<ComputerBuildSignature> {
        None
    }

    fn driver_native_revision(&self) -> Option<u64> {
        None
    }

    async fn list_windows(&self) -> Result<Vec<ComputerWindow>, ComputerError>;

    async fn list_spaces(&self) -> Result<ComputerSpaceInventory, ComputerError> {
        unsupported("listSpaces")
    }

    async fn get_screen_size(&self) -> Result<ComputerScreenSize, ComputerError>;

    async fn get_state(
        &self,
        options: ComputerStateOptions,
    ) -> Result<ComputerState, ComputerError>;

    async fn capture_screenshot(
        &self,
        request: ComputerCaptureRequest,
    ) -> Result<ComputerScreenshot, ComputerError>;

    async fn focus_window(&self, _window_id: &str) -> Result<(), ComputerError> {
        Ok(())
    }

    async fn raise_window(&self, _window_id: &str) -> Result<(), ComputerError> {
        unsupported("raiseWindow")
    }

    async fn clear_focus_window(&self) -> Result<(), ComputerError> {
        Ok(())
    }

    async fn set_driving_agent(&self, _name: Option<&str>) -> Result<(), ComputerError> {
        Ok(())
    }

    async fn set_cursor_activity(&self, _text: Option<&str>) -> Result<(), ComputerError> {
        Ok(())
    }

    async fn launch_app(
        &self,
        app: &str,
        args: &[String],
        options: ComputerLaunchOptions,
    ) -> Result<ComputerLaunchAppResult, ComputerError>;

    async fn check_input_ready(&self, _window_id: &str) -> Result<(), ComputerError> {
        Ok(())
    }

    async fn wait_for_settle(
        &self,
        _options: ComputerSettleOptions,
    ) -> Result<ComputerSettleOutcome, ComputerError> {
        unsupported("waitForSettle")
    }

    async fn list_apps(&self) -> Result<Vec<ComputerApp>, ComputerError> {
        unsupported("listApps")
    }

    async fn set_window_frame(
        &self,
        _window_id: &str,
        _frame: ComputerRect,
    ) -> Result<ComputerBackendActionResult, ComputerError> {
        unsupported("setWindowFrame")
    }

    async fn invoke_menu(
        &self,
        _target: &ComputerMenuBackendTarget,
        _path: &[String],
    ) -> Result<ComputerBackendActionResult, ComputerError> {
        unsupported("invokeMenu")
    }

    async fn verify_state(
        &self,
        _window_id: &str,
        _expect: &[Value],
    ) -> Result<ComputerVerifyStateResult, ComputerError> {
        unsupported("verifyState")
    }

    async fn zoom_window(
        &self,
        _window_id: &str,
        _region: ComputerRect,
    ) -> Result<ComputerZoomResult, ComputerError> {
        unsupported("zoomWindow")
    }

    async fn get_accessibility_tree(
        &self,
        _window_id: Option<&str>,
    ) -> Result<ComputerAccessibilityTreeSnapshot, ComputerError> {
        unsupported("getAccessibilityTree")
    }

    async fn get_cursor_position(
        &self,
        _window_id: Option<&str>,
    ) -> Result<ComputerCursorReading, ComputerError> {
        unsupported("getCursorPosition")
    }

    async fn set_window_minimized(
        &self,
        _window_id: &str,
        _minimized: bool,
    ) -> Result<ComputerBackendActionResult, ComputerError> {
        unsupported("setWindowMinimized")
    }

    async fn set_app_visibility(
        &self,
        _pid: u32,
        _hidden: bool,
    ) -> Result<ComputerBackendActionResult, ComputerError> {
        unsupported("setAppVisibility")
    }

    async fn kill_app(&self, _pid: u32) -> Result<ComputerBackendActionResult, ComputerError> {
        unsupported("killApp")
    }

    fn supports_action(&self, _target: &ComputerResolvedTarget, _action: &str) -> bool {
        false
    }

    async fn click(
        &self,
        point: ComputerPoint,
        window_id: Option<&str>,
        modifiers: &[ComputerInputModifier],
    ) -> Result<ComputerBackendActionResult, ComputerError>;

    async fn double_click(
        &self,
        point: ComputerPoint,
        window_id: Option<&str>,
        modifiers: &[ComputerInputModifier],
    ) -> Result<ComputerBackendActionResult, ComputerError>;

    async fn triple_click(
        &self,
        _point: ComputerPoint,
        _window_id: Option<&str>,
        _modifiers: &[ComputerInputModifier],
    ) -> Result<ComputerBackendActionResult, ComputerError> {
        unsupported("tripleClick")
    }

    async fn right_click(
        &self,
        point: ComputerPoint,
        window_id: Option<&str>,
        modifiers: &[ComputerInputModifier],
    ) -> Result<ComputerBackendActionResult, ComputerError>;

    async fn move_cursor(
        &self,
        point: ComputerPoint,
        window_id: Option<&str>,
    ) -> Result<ComputerBackendActionResult, ComputerError>;

    async fn drag(
        &self,
        from: ComputerPoint,
        to: ComputerPoint,
        duration_ms: f64,
        window_id: Option<&str>,
    ) -> Result<ComputerBackendActionResult, ComputerError>;

    async fn scroll(
        &self,
        point: Option<ComputerPoint>,
        delta_x: f64,
        delta_y: f64,
        window_id: Option<&str>,
        modifiers: &[ComputerInputModifier],
        target: Option<&ComputerResolvedTarget>,
    ) -> Result<ComputerBackendActionResult, ComputerError>;

    fn focus_neutral_semantic_text(&self) -> bool {
        false
    }

    fn exact_target_background_input(&self) -> bool {
        false
    }

    async fn type_text(
        &self,
        text: &str,
        window_id: Option<&str>,
        target: Option<&ComputerResolvedTarget>,
    ) -> Result<ComputerBackendActionResult, ComputerError>;

    async fn press_key(
        &self,
        key: &str,
        window_id: Option<&str>,
        target: Option<&ComputerResolvedTarget>,
    ) -> Result<ComputerBackendActionResult, ComputerError>;

    async fn hotkey(
        &self,
        keys: &[String],
        window_id: Option<&str>,
        target: Option<&ComputerResolvedTarget>,
    ) -> Result<ComputerBackendActionResult, ComputerError>;

    async fn read_clipboard(&self) -> Result<String, ComputerError> {
        unsupported("readClipboard")
    }

    async fn write_clipboard(&self, _text: &str) -> Result<(), ComputerError> {
        unsupported("writeClipboard")
    }

    async fn set_value(
        &self,
        target: &ComputerResolvedTarget,
        value: &str,
    ) -> Result<ComputerBackendActionResult, ComputerError>;

    async fn perform_action(
        &self,
        target: &ComputerResolvedTarget,
        action: &str,
    ) -> Result<ComputerBackendActionResult, ComputerError>;

    async fn select_text(
        &self,
        target: &ComputerResolvedTarget,
        range: ComputerTextRange,
    ) -> Result<ComputerBackendActionResult, ComputerError>;

    fn on_event(&self, _listener: ComputerBackendEventListener) -> ComputerBackendSubscription {
        ComputerBackendSubscription::detached()
    }

    async fn attach_stream(&self, listener: ComputerFrameListener) -> Result<(), ComputerError>;

    async fn detach_stream(&self) -> Result<(), ComputerError>;

    async fn request_keyframe(&self) -> Result<(), ComputerError> {
        Ok(())
    }

    async fn engage_shield(&self, _target: &ComputerShieldTarget) -> Result<String, ComputerError> {
        unsupported("engageShield")
    }

    async fn release_shield(&self, _shield_id: &str) -> Result<(), ComputerError> {
        unsupported("releaseShield")
    }

    async fn release_all_shields(&self) -> Result<(), ComputerError> {
        unsupported("releaseAllShields")
    }

    async fn stop_input(&self, _task: Option<&CuaComputerTask>) -> Result<(), ComputerError> {
        Ok(())
    }

    async fn end_task(
        &self,
        _thread_id: &str,
        _turn_id: Option<&str>,
    ) -> Result<(), ComputerError> {
        Ok(())
    }

    fn browser(&self) -> Option<&dyn ComputerBrowserBackend> {
        None
    }

    fn native_element_identity(&self, _node: &ComputerUiNode) -> Option<String> {
        None
    }

    async fn dispose(&self);
}

pub fn intersect_computer_rects(first: ComputerRect, second: ComputerRect) -> Option<ComputerRect> {
    let left = first.x.max(second.x);
    let top = first.y.max(second.y);
    let right = (first.x + first.width).min(second.x + second.width);
    let bottom = (first.y + first.height).min(second.y + second.height);
    if right <= left || bottom <= top {
        return None;
    }
    Some(ComputerRect {
        x: left,
        y: top,
        width: right - left,
        height: bottom - top,
    })
}

pub(crate) fn js_slice(text: &str, max_units: usize) -> &str {
    let mut units = 0;
    for (index, c) in text.char_indices() {
        let width = c.len_utf16();
        if units + width > max_units {
            return &text[..index];
        }
        units += width;
    }
    text
}

pub fn clamp_computer_message(text: &str, fallback: &str) -> String {
    let trimmed = text.trim();
    let message = if trimmed.is_empty() {
        fallback
    } else {
        trimmed
    };
    if utf16_len(message) > COMPUTER_MESSAGE_MAX_LENGTH {
        format!("{}…", js_slice(message, COMPUTER_MESSAGE_MAX_LENGTH - 1))
    } else {
        message.to_owned()
    }
}

pub fn computer_backend_action_result(
    computer_id: &str,
    action: &str,
    result: ComputerBackendActionResult,
) -> ComputerActionResult {
    let delivery = match (result.delivery_path, result.verified) {
        (Some(path), Some(verified)) => Some(ComputerDelivery {
            path: js_slice(&path, COMPUTER_DELIVERY_PATH_MAX_LENGTH).to_owned(),
            verified,
            effect: result.effect,
        }),
        _ => None,
    };
    ComputerActionResult {
        computer_id: computer_id.to_owned(),
        action: action.to_owned(),
        point: result.point,
        clamped_to: result.clamped_to,
        window_id: result.window_id.filter(|id| !id.is_empty()),
        value: result.value,
        scroll: None,
        delivery,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(x: f64, y: f64, width: f64, height: f64) -> ComputerRect {
        ComputerRect {
            x,
            y,
            width,
            height,
        }
    }

    #[test]
    fn intersects_overlapping_rects_and_refuses_disjoint_ones() {
        assert_eq!(
            intersect_computer_rects(rect(0.0, 0.0, 100.0, 100.0), rect(50.0, 25.0, 100.0, 100.0)),
            Some(rect(50.0, 25.0, 50.0, 75.0))
        );
        assert_eq!(
            intersect_computer_rects(rect(0.0, 0.0, 10.0, 10.0), rect(10.0, 0.0, 10.0, 10.0)),
            None
        );
    }

    #[test]
    fn clamps_messages_to_the_contract_bound() {
        assert_eq!(clamp_computer_message("  ", "fallback"), "fallback");
        assert_eq!(clamp_computer_message(" hi ", "fallback"), "hi");
        let long = "x".repeat(COMPUTER_MESSAGE_MAX_LENGTH + 10);
        let clamped = clamp_computer_message(&long, "fallback");
        assert_eq!(utf16_len(&clamped), COMPUTER_MESSAGE_MAX_LENGTH);
        assert!(clamped.ends_with('…'));
    }

    #[test]
    fn projects_delivery_only_when_both_halves_are_present() {
        let path = "p".repeat(100);
        let result = computer_backend_action_result(
            "desktop",
            "computer_click",
            ComputerBackendActionResult {
                window_id: Some("cua:1:2".into()),
                delivery_path: Some(path),
                verified: Some(ComputerDeliveryVerification::Confirmed),
                effect: Some(CuaEffect::Verified),
                ..ComputerBackendActionResult::default()
            },
        );
        let delivery = result.delivery.unwrap();
        assert_eq!(delivery.path.len(), COMPUTER_DELIVERY_PATH_MAX_LENGTH);
        assert_eq!(delivery.effect, Some(CuaEffect::Verified));
        let bare = computer_backend_action_result(
            "desktop",
            "computer_click",
            ComputerBackendActionResult {
                delivery_path: Some("cua-x".into()),
                ..ComputerBackendActionResult::default()
            },
        );
        assert_eq!(bare.delivery, None);
        assert_eq!(bare.window_id, None);
    }

    #[test]
    fn refuses_a_clipboard_write_past_the_byte_limit() {
        assert!(
            assert_computer_clipboard_write_fits(&"a".repeat(MAX_COMPUTER_CLIPBOARD_BYTES)).is_ok()
        );
        let error =
            assert_computer_clipboard_write_fits(&"a".repeat(MAX_COMPUTER_CLIPBOARD_BYTES + 1))
                .unwrap_err();
        assert!(error.message.contains("byte limit"));
        assert!(!error.retryable);
    }

    #[test]
    fn cua_action_error_marks_replay_forbidden_and_keeps_pause_only_when_proven() {
        let pause = ComputerInputPause {
            window_id: Some("cua:1:2".into()),
            pid: Some(1),
            message: "paused".into(),
            user_takeover: false,
        };
        let refused: ComputerBackendError =
            CuaActionError::new("Desktop is locked.", CuaEffect::NotDispatched)
                .code("computer_input_paused")
                .input_pause(Some(pause.clone()))
                .into();
        assert_eq!(
            refused.message,
            "Desktop is locked. [effect=not-dispatched; automatic replay is forbidden]"
        );
        assert_eq!(refused.input_pause, Some(pause.clone()));
        assert!(!refused.retryable);
        assert_eq!(refused.code(), Some("computer_input_paused"));
        let uncertain: ComputerBackendError =
            CuaActionError::new("lost", CuaEffect::DispatchedUnknown)
                .input_pause(Some(pause.clone()))
                .into();
        assert_eq!(uncertain.input_pause, None);
        assert_eq!(uncertain.code(), Some(CUA_ACTION_FAILED_CODE));
        let restore: ComputerBackendError =
            CuaActionError::new("restore", CuaEffect::DispatchedUnknown)
                .code("focus_restore_failed")
                .input_pause(Some(pause.clone()))
                .into();
        assert_eq!(restore.input_pause, Some(pause));
    }

    #[test]
    fn listeners_unsubscribe_when_their_subscription_drops() {
        let listeners = ComputerBackendListeners::default();
        let seen = Arc::new(Mutex::new(0));
        let counter = seen.clone();
        let subscription = listeners.subscribe(Arc::new(move |_| {
            *counter.lock().unwrap() += 1;
        }));
        listeners.emit(&ComputerBackendEvent::DesktopInterrupted { pauses: vec![] });
        subscription.unsubscribe();
        listeners.emit(&ComputerBackendEvent::DesktopInterrupted { pauses: vec![] });
        assert_eq!(*seen.lock().unwrap(), 1);
        assert!(listeners.is_empty());
    }
}
