use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use async_trait::async_trait;
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use chrono::{SecondsFormat, Utc};
use futures::future::BoxFuture;
use serde_json::{Value, json};
use zeron_proto::computer::{
    ComputerAccessibilityTreeApp, ComputerAccessibilityTreeWindow, ComputerApp,
    ComputerAvailability, ComputerBuildSignature, ComputerCapabilities,
    ComputerDeliveryVerification, ComputerHealth, ComputerHealthStatus, ComputerInputModifier,
    ComputerJpegMimeType, ComputerLaunchAppResult, ComputerPermission, ComputerPngMimeType,
    ComputerPoint, ComputerRect, ComputerScreenSize, ComputerScreenshot, ComputerState,
    ComputerUiNode, ComputerVerifyStateResult, ComputerVerifyStatus, ComputerWindow,
    ComputerZoomResult, CuaContent, CuaEffect,
};

use super::backend::{
    ComputerAccessibilityTreeSnapshot, ComputerAgentDialect, ComputerBackend,
    ComputerBackendActionResult, ComputerBackendError, ComputerBackendEvent,
    ComputerBackendEventListener, ComputerBackendListeners, ComputerBackendMethod,
    ComputerBackendSubscription, ComputerBrowserBackend, ComputerBrowserCall,
    ComputerBrowserCallResult, ComputerCaptureRequest, ComputerCursorReading, ComputerError,
    ComputerFrameListener, ComputerLaunchOptions, ComputerMenuBackendTarget,
    ComputerResolvedTarget, ComputerSettleOptions, ComputerSettleOutcome, ComputerShieldTarget,
    ComputerStateOptions, ComputerStreamFrame, ComputerTextRange,
    DEFAULT_COMPUTER_CAPTURE_MAX_DIMENSION, DEFAULT_COMPUTER_ID, intersect_computer_rects,
    js_slice,
};
use super::geometry::require_window_bounds;
use super::ui_tree_targeting::{ComputerTargetError, ComputerTargetErrorCode};

const FAKE_SCREENSHOT_BASE64: &str =
    "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII=";

const FAKE_ZOOM_BASE64: &str = "/9j/4AAQSkZJRgABAQEASABIAAD/2wBDAAMCAgICAgMCAgIDAwMDBAYEBAQEBAgGBgUGCQgKCgkICQkKDA8MCgsOCwkJDRENDg8QEBEQCgwSExIQEw8QEBD/wAALCAABAAEBAREA/8QAFAABAAAAAAAAAAAAAAAAAAAACf/EABQQAQAAAAAAAAAAAAAAAAAAAAD/2gAIAQEAAD8AKwA//9k=";

const MAX_RECORDED_CALLS: usize = 1_000;

const DEFAULT_FAKE_CAPABILITIES: ComputerCapabilities = ComputerCapabilities {
    windows: true,
    window_bounds: true,
    stacking: true,
    capture: true,
    input: true,
    clipboard: true,
    focus: true,
    raise: true,
    ghost_cursor: true,
    visible_desktop: false,
};

#[derive(Debug, Clone, PartialEq)]
pub struct FakeComputerCall {
    pub method: String,
    pub args: Vec<Value>,
}

pub type FakeComputerNow = Arc<dyn Fn() -> String + Send + Sync>;
pub type FakeBrowserHandler = Arc<
    dyn Fn(
            ComputerBrowserCall,
        ) -> BoxFuture<'static, Result<Option<ComputerBrowserCallResult>, ComputerError>>
        + Send
        + Sync,
>;
pub type FakeSettleHandler = Arc<
    dyn Fn(
            ComputerSettleOptions,
        ) -> BoxFuture<'static, Result<ComputerSettleOutcome, ComputerError>>
        + Send
        + Sync,
>;
pub type FakeShieldHandler = Arc<
    dyn Fn(ComputerShieldTarget) -> BoxFuture<'static, Result<(), ComputerError>> + Send + Sync,
>;

#[derive(Clone, Default)]
pub enum FakeComputerOptIn<H> {
    #[default]
    Off,
    BuiltIn,
    Handler(H),
}

impl<H> FakeComputerOptIn<H> {
    fn opted_in(self) -> Option<Option<H>> {
        match self {
            Self::Off => None,
            Self::BuiltIn => Some(None),
            Self::Handler(handler) => Some(Some(handler)),
        }
    }
}

#[derive(Clone, Default)]
pub struct FakeComputerBackendOptions {
    pub computer_id: Option<String>,
    pub availability: Option<ComputerAvailability>,
    pub health: Option<ComputerHealth>,
    pub capabilities: Option<ComputerCapabilities>,
    pub screen_size: Option<ComputerScreenSize>,
    pub windows: Option<Vec<ComputerWindow>>,
    pub apps: Option<Vec<ComputerApp>>,
    pub root: Option<ComputerUiNode>,
    pub now: Option<FakeComputerNow>,
    pub browser: FakeComputerOptIn<FakeBrowserHandler>,
    pub wait_for_settle: FakeComputerOptIn<FakeSettleHandler>,
    pub shield: FakeComputerOptIn<FakeShieldHandler>,
    pub agent_dialect: Option<ComputerAgentDialect>,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

#[derive(Default)]
struct FakeRecorder {
    calls: Mutex<Vec<FakeComputerCall>>,
    failures: Mutex<HashMap<String, ComputerError>>,
}

impl FakeRecorder {
    fn record(&self, method: &str, args: Vec<Value>) {
        let mut calls = lock(&self.calls);
        calls.push(FakeComputerCall {
            method: method.to_owned(),
            args,
        });
        if calls.len() > MAX_RECORDED_CALLS {
            let excess = calls.len() - MAX_RECORDED_CALLS;
            calls.drain(..excess);
        }
    }

    fn throw_if_failed(&self, method: &str) -> Result<(), ComputerError> {
        match lock(&self.failures).remove(method) {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }
}

pub struct FakeComputerBrowser {
    recorder: Arc<FakeRecorder>,
    handler: Option<FakeBrowserHandler>,
}

#[async_trait]
impl ComputerBrowserBackend for FakeComputerBrowser {
    async fn call(
        &self,
        call: ComputerBrowserCall,
    ) -> Result<ComputerBrowserCallResult, ComputerError> {
        let method = format!("browser.{}", call.name);
        self.recorder.throw_if_failed(&method)?;
        let args = Value::Object(call.args.clone());
        let name = call.name.clone();
        let thread_id = call.task.thread_id.clone();
        let handled = match &self.handler {
            Some(handler) => handler(call).await?,
            None => None,
        };
        let result = handled.unwrap_or_else(|| {
            let structured = if name == "get_browser_state" {
                json!({ "target_id": format!("fake-browser-{thread_id}"), "tabs": [] })
            } else {
                json!({ "status": "completed" })
            };
            ComputerBrowserCallResult {
                content: Some(vec![CuaContent {
                    kind: "text".into(),
                    text: Some(format!("fake browser {name}")),
                    data: None,
                    mime_type: None,
                }]),
                structured_content: structured.as_object().cloned(),
                ..ComputerBrowserCallResult::default()
            }
        });
        self.recorder.record(&method, vec![args]);
        Ok(result)
    }

    async fn end_thread(&self, thread_id: &str) -> Result<(), ComputerError> {
        self.recorder
            .record("browser.endThread", vec![json!(thread_id)]);
        Ok(())
    }
}

struct FakeState {
    availability: ComputerAvailability,
    missing_permissions: Vec<ComputerPermission>,
    build_signature: Option<ComputerBuildSignature>,
    health: ComputerHealth,
    screen_size: ComputerScreenSize,
    windows: Vec<ComputerWindow>,
    apps: Vec<ComputerApp>,
    root: ComputerUiNode,
    next_sequence: u64,
    next_pid: u32,
    clipboard: String,
    queued_screenshots: VecDeque<String>,
    frame_applies: bool,
    refused_menu_paths: HashMap<Vec<String>, ComputerError>,
    verify_satisfied: bool,
    cursor: ComputerPoint,
    disposed: bool,
    live_shields: Vec<String>,
}

impl FakeState {
    fn refresh_root(&mut self) {
        self.root = default_root(&self.screen_size, &self.windows);
    }

    fn workspace_rect(&self) -> ComputerRect {
        rect(
            0.0,
            0.0,
            f64::from(self.screen_size.width),
            f64::from(self.screen_size.height),
        )
    }
}

pub struct FakeComputerBackend {
    computer_id: String,
    capabilities: ComputerCapabilities,
    now: FakeComputerNow,
    agent_dialect: Option<ComputerAgentDialect>,
    recorder: Arc<FakeRecorder>,
    state: Mutex<FakeState>,
    listeners: ComputerBackendListeners,
    frame_listener: Mutex<Option<ComputerFrameListener>>,
    browser: Option<FakeComputerBrowser>,
    settle: Option<Option<FakeSettleHandler>>,
    shield: Option<Option<FakeShieldHandler>>,
}

impl Default for FakeComputerBackend {
    fn default() -> Self {
        Self::new(FakeComputerBackendOptions::default())
    }
}

impl FakeComputerBackend {
    pub fn new(options: FakeComputerBackendOptions) -> Self {
        let screen_size = options.screen_size.unwrap_or(ComputerScreenSize {
            width: 1_920,
            height: 1_080,
            scale: Some(1.0),
        });
        let windows = options.windows.unwrap_or_else(default_windows);
        let apps = options.apps.unwrap_or_else(|| default_apps(&windows));
        let root = options
            .root
            .unwrap_or_else(|| default_root(&screen_size, &windows));
        let recorder = Arc::new(FakeRecorder::default());
        let browser = options
            .browser
            .opted_in()
            .map(|handler| FakeComputerBrowser {
                recorder: recorder.clone(),
                handler,
            });
        Self {
            computer_id: options
                .computer_id
                .unwrap_or_else(|| DEFAULT_COMPUTER_ID.to_owned()),
            capabilities: options.capabilities.unwrap_or(DEFAULT_FAKE_CAPABILITIES),
            now: options.now.unwrap_or_else(|| {
                Arc::new(|| Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true))
            }),
            agent_dialect: options.agent_dialect,
            recorder,
            state: Mutex::new(FakeState {
                availability: options
                    .availability
                    .unwrap_or(ComputerAvailability::Available {
                        backend: Some("fake".into()),
                    }),
                missing_permissions: Vec::new(),
                build_signature: None,
                health: options.health.unwrap_or(ComputerHealth {
                    status: ComputerHealthStatus::Connected,
                    consecutive_failures: 0,
                    reconnects: 0,
                    last_failure: None,
                    capture_available: true,
                    background_input_degraded: None,
                }),
                screen_size,
                windows,
                apps,
                root,
                next_sequence: 1,
                next_pid: 5_000,
                clipboard: String::new(),
                queued_screenshots: VecDeque::new(),
                frame_applies: true,
                refused_menu_paths: HashMap::new(),
                verify_satisfied: true,
                cursor: ComputerPoint { x: 0.0, y: 0.0 },
                disposed: false,
                live_shields: Vec::new(),
            }),
            listeners: ComputerBackendListeners::default(),
            frame_listener: Mutex::new(None),
            browser,
            settle: options.wait_for_settle.opted_in(),
            shield: options.shield.opted_in(),
        }
    }

    pub fn calls(&self) -> Vec<FakeComputerCall> {
        lock(&self.recorder.calls).clone()
    }

    pub fn calls_for(&self, method: &str) -> Vec<FakeComputerCall> {
        lock(&self.recorder.calls)
            .iter()
            .filter(|call| call.method == method)
            .cloned()
            .collect()
    }

    pub fn fail_next(&self, method: &str) {
        self.fail_next_with(
            method,
            ComputerBackendError::new(format!("{method} failed")),
        );
    }

    pub fn fail_next_with(&self, method: &str, error: impl Into<ComputerError>) {
        lock(&self.recorder.failures).insert(method.to_owned(), error.into());
    }

    pub fn set_missing_permissions(&self, permissions: Vec<ComputerPermission>) {
        self.state().missing_permissions = permissions;
    }

    pub fn set_build_signature(&self, signature: Option<ComputerBuildSignature>) {
        self.state().build_signature = signature;
    }

    pub fn set_cursor_position(&self, point: ComputerPoint) {
        self.state().cursor = point;
    }

    pub fn set_frame_applies(&self, applies: bool) {
        self.state().frame_applies = applies;
    }

    pub fn refuse_menu_path(&self, path: &[&str], error: impl Into<ComputerError>) {
        let path = path.iter().map(|segment| (*segment).to_owned()).collect();
        self.state().refused_menu_paths.insert(path, error.into());
    }

    pub fn set_verify_satisfied(&self, satisfied: bool) {
        self.state().verify_satisfied = satisfied;
    }

    pub fn set_availability(&self, availability: ComputerAvailability) {
        self.state().availability = availability;
    }

    pub fn set_screen_size(&self, screen_size: ComputerScreenSize) {
        self.state().screen_size = screen_size;
    }

    pub fn set_root(&self, root: ComputerUiNode) {
        self.state().root = root;
    }

    pub fn root(&self) -> ComputerUiNode {
        self.state().root.clone()
    }

    pub fn queue_screenshots<I, S>(&self, bytes_base64: I)
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.state()
            .queued_screenshots
            .extend(bytes_base64.into_iter().map(Into::into));
    }

    pub fn active_shields(&self) -> Vec<String> {
        self.state().live_shields.clone()
    }

    pub fn emit_frame(&self, keyframe: bool, codec_config: bool, data: Vec<u8>) {
        let Some(listener) = lock(&self.frame_listener).clone() else {
            return;
        };
        let sequence = {
            let mut state = self.state();
            if state.disposed {
                return;
            }
            let sequence = state.next_sequence;
            state.next_sequence += 1;
            sequence
        };
        listener(ComputerStreamFrame {
            sequence,
            timestamp_ms: Utc::now().timestamp_millis() as f64,
            keyframe,
            codec_config,
            data,
        });
    }

    pub fn emit_windows_changed(&self, windows: Vec<ComputerWindow>) {
        self.state().windows = windows.clone();
        self.listeners
            .emit(&ComputerBackendEvent::WindowsChanged(windows));
    }

    pub fn emit_health_changed(&self, health: ComputerHealth) {
        self.state().health = health.clone();
        self.listeners
            .emit(&ComputerBackendEvent::HealthChanged(health));
    }

    pub fn emit_desktop_interrupted(&self, pauses: &[&str]) {
        self.listeners
            .emit(&ComputerBackendEvent::DesktopInterrupted {
                pauses: pauses.iter().map(|pause| (*pause).to_owned()).collect(),
            });
    }

    fn state(&self) -> MutexGuard<'_, FakeState> {
        lock(&self.state)
    }

    fn record(&self, method: &str, args: Vec<Value>) {
        self.recorder.record(method, args);
    }

    fn fail(&self, method: &str) -> Result<(), ComputerError> {
        self.recorder.throw_if_failed(method)
    }

    fn windows_changed(&self, windows: Vec<ComputerWindow>) {
        self.listeners
            .emit(&ComputerBackendEvent::WindowsChanged(windows));
    }

    fn capture_rect(
        &self,
        request: &ComputerCaptureRequest,
    ) -> Result<ComputerRect, ComputerError> {
        match request {
            ComputerCaptureRequest::Region { region, .. } => Ok(*region),
            ComputerCaptureRequest::Window { window_id, .. } => {
                let state = self.state();
                let window = state
                    .windows
                    .iter()
                    .find(|window| &window.id == window_id)
                    .ok_or_else(|| no_window(window_id))?;
                Ok(require_window_bounds(window, "a window screenshot")?)
            }
        }
    }

    fn screenshot_of_region(
        &self,
        region: ComputerRect,
        max_dimension: Option<u32>,
    ) -> ComputerScreenshot {
        let limit = f64::from(max_dimension.unwrap_or(DEFAULT_COMPUTER_CAPTURE_MAX_DIMENSION));
        let scale = (limit / region.width.max(region.height)).min(1.0);
        let bytes_base64 = self
            .state()
            .queued_screenshots
            .pop_front()
            .unwrap_or_else(|| FAKE_SCREENSHOT_BASE64.to_owned());
        ComputerScreenshot {
            mime_type: ComputerPngMimeType::Png,
            width: pixel_extent(region.width * scale),
            height: pixel_extent(region.height * scale),
            size_bytes: decoded_len(&bytes_base64),
            bytes_base64,
            window_id: None,
            region: Some(region),
            scale: Some(scale),
            captured_at: (self.now)(),
        }
    }

    async fn pointer_action(
        &self,
        method: &str,
        point: ComputerPoint,
        modifiers: &[ComputerInputModifier],
    ) -> Result<ComputerBackendActionResult, ComputerError> {
        let mut args = vec![json!(point)];
        if !modifiers.is_empty() {
            args.push(json!(modifiers));
        }
        self.record(method, args);
        self.fail(method)?;
        self.validate_point(point)?;
        self.state().cursor = point;
        Ok(ComputerBackendActionResult {
            point: Some(point),
            ..ComputerBackendActionResult::default()
        })
    }

    fn validate_point(&self, point: ComputerPoint) -> Result<(), ComputerError> {
        let screen = self.state().screen_size;
        if point.x < 0.0
            || point.y < 0.0
            || point.x >= f64::from(screen.width)
            || point.y >= f64::from(screen.height)
        {
            return Err(backend_error(format!(
                "Point ({}, {}) is outside the fake screen",
                point.x, point.y
            )));
        }
        Ok(())
    }
}

#[async_trait]
impl ComputerBackend for FakeComputerBackend {
    fn agent_dialect(&self) -> ComputerAgentDialect {
        self.agent_dialect.unwrap_or(ComputerAgentDialect::Linux)
    }

    fn computer_id(&self) -> &str {
        &self.computer_id
    }

    fn supports(&self, method: ComputerBackendMethod) -> bool {
        use ComputerBackendMethod as M;
        match method {
            M::Provision | M::ListSpaces | M::CheckInputReady => false,
            M::WaitForSettle => self.settle.is_some(),
            M::EngageShield | M::ReleaseShield | M::ReleaseAllShields => self.shield.is_some(),
            M::RaiseWindow
            | M::ListApps
            | M::SetWindowFrame
            | M::InvokeMenu
            | M::VerifyState
            | M::ZoomWindow
            | M::GetAccessibilityTree
            | M::GetCursorPosition
            | M::SetWindowMinimized
            | M::SetAppVisibility
            | M::KillApp
            | M::TripleClick
            | M::ReadClipboard
            | M::WriteClipboard
            | M::RequestKeyframe => true,
        }
    }

    async fn probe_availability(&self) -> Result<ComputerAvailability, ComputerError> {
        self.record("probeAvailability", Vec::new());
        self.fail("probeAvailability")?;
        Ok(self.state().availability.clone())
    }

    async fn availability(&self, _refresh: bool) -> Result<ComputerAvailability, ComputerError> {
        self.record("availability", Vec::new());
        self.fail("availability")?;
        Ok(self.state().availability.clone())
    }

    fn health(&self) -> ComputerHealth {
        self.state().health.clone()
    }

    fn capabilities(&self) -> ComputerCapabilities {
        self.capabilities
    }

    async fn missing_permissions(&self) -> Vec<ComputerPermission> {
        self.state().missing_permissions.clone()
    }

    fn build_signature(&self) -> Option<ComputerBuildSignature> {
        self.state().build_signature
    }

    async fn list_windows(&self) -> Result<Vec<ComputerWindow>, ComputerError> {
        self.record("listWindows", Vec::new());
        self.fail("listWindows")?;
        Ok(self.state().windows.clone())
    }

    async fn get_screen_size(&self) -> Result<ComputerScreenSize, ComputerError> {
        self.record("getScreenSize", Vec::new());
        self.fail("getScreenSize")?;
        Ok(self.state().screen_size)
    }

    async fn get_state(
        &self,
        options: ComputerStateOptions,
    ) -> Result<ComputerState, ComputerError> {
        self.record("getState", vec![state_options_json(&options)]);
        self.fail("getState")?;
        let screenshot = options.include_screenshot.then(|| {
            let workspace = self.state().workspace_rect();
            self.screenshot_of_region(workspace, None)
        });
        let windows = self.list_windows().await?;
        let (screen_size, root) = {
            let state = self.state();
            (state.screen_size, state.root.clone())
        };
        Ok(ComputerState {
            input_pause: None,
            preview_note: None,
            accessibility: None,
            computer_id: self.computer_id.clone(),
            windows,
            screen_size,
            availability: None,
            root: Some(root),
            text: None,
            screenshot,
            captured_at: (self.now)(),
        })
    }

    async fn capture_screenshot(
        &self,
        request: ComputerCaptureRequest,
    ) -> Result<ComputerScreenshot, ComputerError> {
        self.record("captureScreenshot", vec![capture_request_json(&request)]);
        self.fail("captureScreenshot")?;
        let captured = self.capture_rect(&request)?;
        let workspace = self.state().workspace_rect();
        let region = intersect_computer_rects(captured, workspace).ok_or_else(|| {
            backend_error("The capture request does not overlap the fake workspace.")
        })?;
        Ok(self.screenshot_of_region(region, request.max_dimension()))
    }

    async fn focus_window(&self, window_id: &str) -> Result<(), ComputerError> {
        self.record("focusWindow", vec![json!(window_id)]);
        self.fail("focusWindow")?;
        for window in &mut self.state().windows {
            window.focused = window.id == window_id;
        }
        Ok(())
    }

    async fn raise_window(&self, window_id: &str) -> Result<(), ComputerError> {
        self.record("raiseWindow", vec![json!(window_id)]);
        self.fail("raiseWindow")
    }

    async fn clear_focus_window(&self) -> Result<(), ComputerError> {
        self.record("clearFocusWindow", Vec::new());
        self.fail("clearFocusWindow")?;
        for window in &mut self.state().windows {
            window.focused = false;
        }
        Ok(())
    }

    async fn launch_app(
        &self,
        app: &str,
        args: &[String],
        options: ComputerLaunchOptions,
    ) -> Result<ComputerLaunchAppResult, ComputerError> {
        let mut recorded = vec![json!(app), json!(args)];
        if let Some(hidden) = options.hidden {
            recorded.push(json!({ "hidden": hidden }));
        }
        self.record("launchApp", recorded);
        self.fail("launchApp")?;
        let (window, windows) = {
            let mut state = self.state();
            let mut window = fake_window(
                &format!("fake-window-{}", state.windows.len() + 1),
                app,
                app,
                state.next_pid,
                rect(120.0, 80.0, 900.0, 700.0),
                false,
            );
            window.visible = options.hidden != Some(true);
            state.next_pid += 1;
            state.windows.push(window.clone());
            state.refresh_root();
            (window, state.windows.clone())
        };
        self.windows_changed(windows);
        Ok(ComputerLaunchAppResult {
            computer_id: self.computer_id.clone(),
            app: app.to_owned(),
            focus_changed_during_launch: None,
            pid: None,
            window_status: None,
            window_reason: None,
            resolved_command: None,
            window: Some(window),
        })
    }

    async fn wait_for_settle(
        &self,
        options: ComputerSettleOptions,
    ) -> Result<ComputerSettleOutcome, ComputerError> {
        let Some(handler) = &self.settle else {
            return Err(not_implemented("waitForSettle"));
        };
        self.record("waitForSettle", vec![json!(options)]);
        self.fail("waitForSettle")?;
        let known = self
            .state()
            .windows
            .iter()
            .any(|window| window.id == options.window_id);
        if !known {
            return Err(ComputerTargetError::new(
                ComputerTargetErrorCode::NotFound,
                format!(
                    "No desktop window has id {}.",
                    json_string(&options.window_id)
                ),
                Vec::new(),
            )
            .into());
        }
        match handler {
            Some(handler) => handler(options).await,
            None => Ok(ComputerSettleOutcome {
                settled: true,
                waited_ms: 0.0,
                events_seen: None,
            }),
        }
    }

    async fn list_apps(&self) -> Result<Vec<ComputerApp>, ComputerError> {
        self.record("listApps", Vec::new());
        self.fail("listApps")?;
        Ok(self.state().apps.clone())
    }

    async fn set_window_frame(
        &self,
        window_id: &str,
        frame: ComputerRect,
    ) -> Result<ComputerBackendActionResult, ComputerError> {
        self.record("setWindowFrame", vec![json!(window_id), json!(frame)]);
        self.fail("setWindowFrame")?;
        let finite = [frame.x, frame.y, frame.width, frame.height]
            .iter()
            .all(|value| value.is_finite());
        if !finite || frame.width <= 0.0 || frame.height <= 0.0 {
            return Err(backend_error(
                "Window frame needs finite geometry and positive size.",
            ));
        }
        let windows = {
            let mut state = self.state();
            let applies = state.frame_applies;
            let Some(window) = state
                .windows
                .iter_mut()
                .find(|window| window.id == window_id)
            else {
                return Err(no_window(window_id));
            };
            if !applies {
                return Ok(delivered(
                    Some(window_id),
                    "fake-frame",
                    ComputerDeliveryVerification::Unconfirmed,
                    CuaEffect::DispatchedUnknown,
                ));
            }
            window.bounds = Some(frame);
            state.refresh_root();
            state.windows.clone()
        };
        self.windows_changed(windows);
        Ok(confirmed(Some(window_id), "fake-frame"))
    }

    async fn invoke_menu(
        &self,
        target: &ComputerMenuBackendTarget,
        path: &[String],
    ) -> Result<ComputerBackendActionResult, ComputerError> {
        self.record("invokeMenu", vec![json!(target), json!(path)]);
        self.fail("invokeMenu")?;
        let refusal = {
            let state = self.state();
            match target {
                ComputerMenuBackendTarget::Window { window_id } => {
                    if !state.windows.iter().any(|window| &window.id == window_id) {
                        return Err(no_window(window_id));
                    }
                }
                ComputerMenuBackendTarget::Pid { pid } => {
                    if !state.apps.iter().any(|app| app.pid == *pid && app.running) {
                        return Err(no_running_app(*pid));
                    }
                }
            }
            if path.is_empty() || path.iter().any(|segment| segment.trim().is_empty()) {
                return Err(backend_error(
                    "A menu path needs at least one non-empty title.",
                ));
            }
            state.refused_menu_paths.get(path).cloned()
        };
        if let Some(refusal) = refusal {
            return Err(refusal);
        }
        let window_id = match target {
            ComputerMenuBackendTarget::Window { window_id } => Some(window_id.as_str()),
            ComputerMenuBackendTarget::Pid { .. } => None,
        };
        Ok(confirmed(window_id, "fake-menu"))
    }

    async fn verify_state(
        &self,
        window_id: &str,
        expect: &[Value],
    ) -> Result<ComputerVerifyStateResult, ComputerError> {
        self.record("verifyState", vec![json!(window_id), json!(expect)]);
        self.fail("verifyState")?;
        let (known, satisfied) = {
            let state = self.state();
            (
                state.windows.iter().any(|window| window.id == window_id),
                state.verify_satisfied,
            )
        };
        if !known {
            return Err(no_window(window_id));
        }
        let status = if satisfied {
            ComputerVerifyStatus::Satisfied
        } else {
            ComputerVerifyStatus::Unsatisfied
        };
        Ok(ComputerVerifyStateResult {
            status,
            stable: true,
            samples: 1,
            elapsed_ms: 0,
            predicates: (0..expect.len())
                .map(|index| {
                    json!({
                        "index": index,
                        "status": status,
                        "unknown_reason": null,
                        "observed_json": "{}",
                    })
                })
                .collect(),
        })
    }

    async fn zoom_window(
        &self,
        window_id: &str,
        region: ComputerRect,
    ) -> Result<ComputerZoomResult, ComputerError> {
        self.record("zoomWindow", vec![json!(window_id), json!(region)]);
        self.fail("zoomWindow")?;
        let window = self
            .state()
            .windows
            .iter()
            .find(|window| window.id == window_id)
            .cloned()
            .ok_or_else(|| no_window(window_id))?;
        let bounds = require_window_bounds(&window, "a zoom capture")?;
        let finite = [region.x, region.y, region.width, region.height]
            .iter()
            .all(|value| value.is_finite());
        if !finite
            || region.width <= 0.0
            || region.height <= 0.0
            || region.x < 0.0
            || region.y < 0.0
            || region.x + region.width > bounds.width
            || region.y + region.height > bounds.height
        {
            return Err(backend_error(
                "The zoom region lies outside the target window.",
            ));
        }
        Ok(ComputerZoomResult {
            mime_type: ComputerJpegMimeType::Jpeg,
            width: 1,
            height: 1,
            size_bytes: decoded_len(FAKE_ZOOM_BASE64),
            bytes_base64: FAKE_ZOOM_BASE64.to_owned(),
            window_id: Some(window_id.to_owned()),
            captured_at: (self.now)(),
        })
    }

    async fn get_accessibility_tree(
        &self,
        window_id: Option<&str>,
    ) -> Result<ComputerAccessibilityTreeSnapshot, ComputerError> {
        self.record(
            "getAccessibilityTree",
            window_id.map(|id| json!(id)).into_iter().collect(),
        );
        self.fail("getAccessibilityTree")?;
        let state = self.state();
        let scoped_pid = match window_id {
            Some(window_id) => Some(
                state
                    .windows
                    .iter()
                    .find(|window| window.id == window_id)
                    .ok_or_else(|| no_window(window_id))?
                    .pid,
            ),
            None => None,
        };
        let in_scope = |pid: Option<u32>| scoped_pid.is_none_or(|scoped| scoped == pid);
        let apps = state
            .apps
            .iter()
            .filter(|app| app.running && app.pid > 0 && in_scope(Some(app.pid)))
            .map(|app| ComputerAccessibilityTreeApp {
                pid: app.pid,
                name: app.name.clone(),
                bundle_id: app.bundle_id.clone().filter(|id| !id.is_empty()),
            })
            .take(1_024)
            .collect();
        let windows = state
            .windows
            .iter()
            .filter(|window| {
                window.visible
                    && !window.minimized
                    && window.pid.is_some_and(|pid| pid > 0)
                    && in_scope(window.pid)
            })
            .filter_map(|window| {
                Some(ComputerAccessibilityTreeWindow {
                    id: window.id.clone(),
                    pid: window.pid?,
                    app_name: window.app_name.clone().filter(|name| !name.is_empty()),
                    title: window.title.clone(),
                    bounds: window.bounds,
                    on_screen: Some(true),
                    z_index: window.stacking_index,
                })
            })
            .take(512)
            .collect();
        Ok(ComputerAccessibilityTreeSnapshot {
            apps,
            windows,
            truncated: false,
        })
    }

    async fn get_cursor_position(
        &self,
        window_id: Option<&str>,
    ) -> Result<ComputerCursorReading, ComputerError> {
        self.record(
            "getCursorPosition",
            window_id.map(|id| json!(id)).into_iter().collect(),
        );
        self.fail("getCursorPosition")?;
        let (window, cursor) = {
            let state = self.state();
            let window = match window_id {
                Some(window_id) => Some(
                    state
                        .windows
                        .iter()
                        .find(|window| window.id == window_id)
                        .cloned()
                        .ok_or_else(|| no_window(window_id))?,
                ),
                None => None,
            };
            (window, state.cursor)
        };
        let ComputerPoint { x, y } = cursor;
        Ok(ComputerCursorReading {
            x,
            y,
            captured_at: (self.now)(),
            window_id: window.as_ref().map(|window| window.id.clone()),
            inside_window: window.and_then(|window| window.bounds).map(|bounds| {
                x >= bounds.x
                    && x < bounds.x + bounds.width
                    && y >= bounds.y
                    && y < bounds.y + bounds.height
            }),
        })
    }

    async fn set_window_minimized(
        &self,
        window_id: &str,
        minimized: bool,
    ) -> Result<ComputerBackendActionResult, ComputerError> {
        self.record(
            "setWindowMinimized",
            vec![json!(window_id), json!(minimized)],
        );
        self.fail("setWindowMinimized")?;
        let windows = {
            let mut state = self.state();
            let Some(window) = state
                .windows
                .iter_mut()
                .find(|window| window.id == window_id)
            else {
                return Err(no_window(window_id));
            };
            window.minimized = minimized;
            window.visible = !minimized;
            state.refresh_root();
            state.windows.clone()
        };
        self.windows_changed(windows);
        Ok(confirmed(Some(window_id), "fake-minimize"))
    }

    async fn set_app_visibility(
        &self,
        pid: u32,
        hidden: bool,
    ) -> Result<ComputerBackendActionResult, ComputerError> {
        self.record("setAppVisibility", vec![json!(pid), json!(hidden)]);
        self.fail("setAppVisibility")?;
        let windows = {
            let mut state = self.state();
            if !state.apps.iter().any(|app| app.pid == pid && app.running) {
                return Err(no_running_app(pid));
            }
            for window in state
                .windows
                .iter_mut()
                .filter(|window| window.pid == Some(pid))
            {
                window.visible = !hidden && !window.minimized;
            }
            state.refresh_root();
            state.windows.clone()
        };
        self.windows_changed(windows);
        Ok(confirmed(None, "fake-app-visibility"))
    }

    async fn kill_app(&self, pid: u32) -> Result<ComputerBackendActionResult, ComputerError> {
        self.record("killApp", vec![json!(pid)]);
        self.fail("killApp")?;
        let (window_id, windows) = {
            let mut state = self.state();
            let Some(window_id) = state
                .windows
                .iter()
                .find(|window| window.pid == Some(pid))
                .map(|window| window.id.clone())
            else {
                return Err(backend_error(format!(
                    "No desktop window belongs to pid {pid}."
                )));
            };
            state.windows.retain(|window| window.pid != Some(pid));
            for app in state.apps.iter_mut().filter(|app| app.pid == pid) {
                app.running = false;
                app.active = false;
                app.pid = 0;
            }
            state.refresh_root();
            (window_id, state.windows.clone())
        };
        self.windows_changed(windows);
        Ok(confirmed(Some(&window_id), "fake-kill"))
    }

    async fn click(
        &self,
        point: ComputerPoint,
        _window_id: Option<&str>,
        modifiers: &[ComputerInputModifier],
    ) -> Result<ComputerBackendActionResult, ComputerError> {
        self.pointer_action("click", point, modifiers).await
    }

    async fn double_click(
        &self,
        point: ComputerPoint,
        _window_id: Option<&str>,
        modifiers: &[ComputerInputModifier],
    ) -> Result<ComputerBackendActionResult, ComputerError> {
        self.pointer_action("doubleClick", point, modifiers).await
    }

    async fn triple_click(
        &self,
        point: ComputerPoint,
        _window_id: Option<&str>,
        modifiers: &[ComputerInputModifier],
    ) -> Result<ComputerBackendActionResult, ComputerError> {
        self.pointer_action("tripleClick", point, modifiers).await
    }

    async fn right_click(
        &self,
        point: ComputerPoint,
        _window_id: Option<&str>,
        modifiers: &[ComputerInputModifier],
    ) -> Result<ComputerBackendActionResult, ComputerError> {
        self.pointer_action("rightClick", point, modifiers).await
    }

    async fn move_cursor(
        &self,
        point: ComputerPoint,
        _window_id: Option<&str>,
    ) -> Result<ComputerBackendActionResult, ComputerError> {
        self.pointer_action("moveCursor", point, &[]).await
    }

    async fn drag(
        &self,
        from: ComputerPoint,
        to: ComputerPoint,
        duration_ms: f64,
        _window_id: Option<&str>,
    ) -> Result<ComputerBackendActionResult, ComputerError> {
        self.record("drag", vec![json!(from), json!(to), json!(duration_ms)]);
        self.fail("drag")?;
        self.validate_point(from)?;
        self.validate_point(to)?;
        self.state().cursor = to;
        Ok(ComputerBackendActionResult {
            point: Some(to),
            ..ComputerBackendActionResult::default()
        })
    }

    async fn scroll(
        &self,
        point: Option<ComputerPoint>,
        delta_x: f64,
        delta_y: f64,
        _window_id: Option<&str>,
        modifiers: &[ComputerInputModifier],
        _target: Option<&ComputerResolvedTarget>,
    ) -> Result<ComputerBackendActionResult, ComputerError> {
        let mut args = vec![json!(point), json!(delta_x), json!(delta_y)];
        if !modifiers.is_empty() {
            args.push(json!(modifiers));
        }
        self.record("scroll", args);
        self.fail("scroll")?;
        if let Some(point) = point {
            self.validate_point(point)?;
        }
        Ok(ComputerBackendActionResult {
            point,
            ..ComputerBackendActionResult::default()
        })
    }

    async fn type_text(
        &self,
        text: &str,
        _window_id: Option<&str>,
        _target: Option<&ComputerResolvedTarget>,
    ) -> Result<ComputerBackendActionResult, ComputerError> {
        self.record("typeText", vec![json!(text)]);
        self.fail("typeText")?;
        Ok(ComputerBackendActionResult {
            value: Some(text.to_owned()),
            ..ComputerBackendActionResult::default()
        })
    }

    async fn press_key(
        &self,
        key: &str,
        _window_id: Option<&str>,
        _target: Option<&ComputerResolvedTarget>,
    ) -> Result<ComputerBackendActionResult, ComputerError> {
        self.record("pressKey", vec![json!(key)]);
        self.fail("pressKey")?;
        Ok(ComputerBackendActionResult::default())
    }

    async fn hotkey(
        &self,
        keys: &[String],
        _window_id: Option<&str>,
        _target: Option<&ComputerResolvedTarget>,
    ) -> Result<ComputerBackendActionResult, ComputerError> {
        self.record("hotkey", vec![json!(keys)]);
        self.fail("hotkey")?;
        Ok(ComputerBackendActionResult::default())
    }

    async fn read_clipboard(&self) -> Result<String, ComputerError> {
        self.record("readClipboard", Vec::new());
        self.fail("readClipboard")?;
        Ok(self.state().clipboard.clone())
    }

    async fn write_clipboard(&self, text: &str) -> Result<(), ComputerError> {
        self.record("writeClipboard", vec![json!(text)]);
        self.fail("writeClipboard")?;
        self.state().clipboard = text.to_owned();
        Ok(())
    }

    async fn set_value(
        &self,
        target: &ComputerResolvedTarget,
        value: &str,
    ) -> Result<ComputerBackendActionResult, ComputerError> {
        self.record("setValue", vec![resolved_target_json(target), json!(value)]);
        self.fail("setValue")?;
        replace_node_value(&mut self.state().root, &target.node, value);
        Ok(ComputerBackendActionResult {
            point: Some(target.point),
            value: Some(value.to_owned()),
            ..ComputerBackendActionResult::default()
        })
    }

    async fn perform_action(
        &self,
        target: &ComputerResolvedTarget,
        action: &str,
    ) -> Result<ComputerBackendActionResult, ComputerError> {
        self.record(
            "performAction",
            vec![resolved_target_json(target), json!(action)],
        );
        self.fail("performAction")?;
        Ok(ComputerBackendActionResult {
            point: Some(target.point),
            value: Some(action.to_owned()),
            ..ComputerBackendActionResult::default()
        })
    }

    async fn select_text(
        &self,
        target: &ComputerResolvedTarget,
        range: ComputerTextRange,
    ) -> Result<ComputerBackendActionResult, ComputerError> {
        self.record(
            "selectText",
            vec![resolved_target_json(target), json!(range)],
        );
        self.fail("selectText")?;
        let value = target.node.value.as_deref().unwrap_or("");
        let units = |count: u64| usize::try_from(count).unwrap_or(usize::MAX);
        let through_end = js_slice(value, units(range.start.saturating_add(range.length)));
        let before_start = js_slice(through_end, units(range.start));
        Ok(ComputerBackendActionResult {
            point: Some(target.point),
            value: Some(through_end[before_start.len()..].to_owned()),
            ..ComputerBackendActionResult::default()
        })
    }

    fn on_event(&self, listener: ComputerBackendEventListener) -> ComputerBackendSubscription {
        self.listeners.subscribe(listener)
    }

    async fn attach_stream(&self, listener: ComputerFrameListener) -> Result<(), ComputerError> {
        self.record("attachStream", Vec::new());
        self.fail("attachStream")?;
        *lock(&self.frame_listener) = Some(listener);
        self.emit_frame(true, true, vec![0x01]);
        self.emit_frame(true, false, vec![0x01]);
        Ok(())
    }

    async fn detach_stream(&self) -> Result<(), ComputerError> {
        self.record("detachStream", Vec::new());
        self.fail("detachStream")?;
        *lock(&self.frame_listener) = None;
        Ok(())
    }

    async fn request_keyframe(&self) -> Result<(), ComputerError> {
        self.record("requestKeyframe", Vec::new());
        self.fail("requestKeyframe")?;
        if lock(&self.frame_listener).is_some() {
            self.emit_frame(true, true, vec![0x01]);
            self.emit_frame(true, false, vec![0x01]);
        }
        Ok(())
    }

    async fn engage_shield(&self, target: &ComputerShieldTarget) -> Result<String, ComputerError> {
        let Some(handler) = &self.shield else {
            return Err(not_implemented("engageShield"));
        };
        self.record("engageShield", vec![json!(target)]);
        self.fail("engageShield")?;
        if let Some(handler) = handler {
            handler(target.clone()).await?;
        }
        let mut state = self.state();
        if !state.live_shields.contains(&target.shield_id) {
            state.live_shields.push(target.shield_id.clone());
        }
        Ok(target.shield_id.clone())
    }

    async fn release_shield(&self, shield_id: &str) -> Result<(), ComputerError> {
        if self.shield.is_none() {
            return Err(not_implemented("releaseShield"));
        }
        self.record("releaseShield", vec![json!(shield_id)]);
        self.fail("releaseShield")?;
        self.state().live_shields.retain(|live| live != shield_id);
        Ok(())
    }

    async fn release_all_shields(&self) -> Result<(), ComputerError> {
        if self.shield.is_none() {
            return Err(not_implemented("releaseAllShields"));
        }
        self.record("releaseAllShields", Vec::new());
        self.state().live_shields.clear();
        Ok(())
    }

    fn browser(&self) -> Option<&dyn ComputerBrowserBackend> {
        self.browser
            .as_ref()
            .map(|browser| browser as &dyn ComputerBrowserBackend)
    }

    async fn dispose(&self) {
        self.record("dispose", Vec::new());
        self.state().disposed = true;
        *lock(&self.frame_listener) = None;
        self.listeners.clear();
    }
}

fn backend_error(message: impl Into<String>) -> ComputerError {
    ComputerBackendError::new(message).into()
}

fn not_implemented(method: &str) -> ComputerError {
    backend_error(format!("This backend does not implement {method}."))
}

fn json_string(text: &str) -> String {
    Value::String(text.to_owned()).to_string()
}

fn no_window(window_id: &str) -> ComputerError {
    backend_error(format!(
        "No desktop window has id {}.",
        json_string(window_id)
    ))
}

fn no_running_app(pid: u32) -> ComputerError {
    backend_error(format!("No running application has pid {pid}."))
}

fn delivered(
    window_id: Option<&str>,
    path: &str,
    verified: ComputerDeliveryVerification,
    effect: CuaEffect,
) -> ComputerBackendActionResult {
    ComputerBackendActionResult {
        window_id: window_id.map(str::to_owned),
        delivery_path: Some(path.to_owned()),
        verified: Some(verified),
        effect: Some(effect),
        ..ComputerBackendActionResult::default()
    }
}

fn confirmed(window_id: Option<&str>, path: &str) -> ComputerBackendActionResult {
    delivered(
        window_id,
        path,
        ComputerDeliveryVerification::Confirmed,
        CuaEffect::Verified,
    )
}

fn state_options_json(options: &ComputerStateOptions) -> Value {
    let mut value = json!({ "includeTree": options.include_tree });
    if options.include_screenshot {
        value["includeScreenshot"] = json!(true);
    }
    if let Some(window_id) = &options.window_id {
        value["windowId"] = json!(window_id);
    }
    if options.reuse_recent_tree {
        value["reuseRecentTree"] = json!(true);
    }
    value
}

fn capture_request_json(request: &ComputerCaptureRequest) -> Value {
    let mut value = match request {
        ComputerCaptureRequest::Window { window_id, .. } => {
            json!({ "kind": "window", "windowId": window_id })
        }
        ComputerCaptureRequest::Region { region, .. } => {
            json!({ "kind": "region", "region": region })
        }
    };
    if let Some(max_dimension) = request.max_dimension() {
        value["maxDimension"] = json!(max_dimension);
    }
    value
}

fn resolved_target_json(target: &ComputerResolvedTarget) -> Value {
    json!({
        "target": target.target.target,
        "point": target.point,
        "node": target.node,
    })
}

fn decoded_len(bytes_base64: &str) -> u64 {
    BASE64
        .decode(bytes_base64)
        .map_or(0, |bytes| bytes.len() as u64)
}

fn pixel_extent(extent: f64) -> u32 {
    extent.round().max(1.0) as u32
}

fn rect(x: f64, y: f64, width: f64, height: f64) -> ComputerRect {
    ComputerRect {
        x,
        y,
        width,
        height,
    }
}

fn fake_window(
    id: &str,
    title: &str,
    app_name: &str,
    pid: u32,
    bounds: ComputerRect,
    focused: bool,
) -> ComputerWindow {
    ComputerWindow {
        id: id.to_owned(),
        title: title.to_owned(),
        app_name: Some(app_name.to_owned()),
        pid: Some(pid),
        bounds: Some(bounds),
        focused,
        keyboard_focused: None,
        active: None,
        minimized: false,
        visible: true,
        space_ids: None,
        current_space_id: None,
        on_current_space: None,
        stacking_index: None,
        occluded_by: None,
    }
}

fn default_windows() -> Vec<ComputerWindow> {
    vec![
        fake_window(
            "fake-terminal",
            "Terminal",
            "org.kde.konsole",
            1_001,
            rect(40.0, 40.0, 960.0, 720.0),
            true,
        ),
        fake_window(
            "fake-calculator",
            "Calculator",
            "org.kde.kcalc",
            1_002,
            rect(1_050.0, 120.0, 420.0, 620.0),
            false,
        ),
    ]
}

fn default_apps(windows: &[ComputerWindow]) -> Vec<ComputerApp> {
    windows
        .iter()
        .filter_map(|window| {
            let pid = window.pid?;
            let app_name = window.app_name.as_deref().filter(|name| !name.is_empty())?;
            let name = if window.title.is_empty() {
                app_name
            } else {
                window.title.as_str()
            };
            Some(ComputerApp {
                pid,
                name: name.to_owned(),
                bundle_id: Some(app_name.to_owned()),
                team_id: None,
                running: true,
                active: window.focused,
                launch_path: None,
                window_count: Some(1),
                last_used: None,
            })
        })
        .collect()
}

fn ui_node(
    role: &str,
    label: Option<&str>,
    value: Option<&str>,
    description: Option<&str>,
    frame: ComputerRect,
    window_id: Option<&str>,
    children: Vec<ComputerUiNode>,
) -> ComputerUiNode {
    ComputerUiNode {
        role: role.to_owned(),
        label: label.map(str::to_owned),
        value: value.map(str::to_owned),
        description: description.map(str::to_owned),
        frame,
        activation_point: None,
        on_screen: true,
        window_id: window_id.map(str::to_owned),
        accessibility_root: None,
        node_path: None,
        editable: None,
        truncated: None,
        children,
    }
}

fn default_root(screen_size: &ComputerScreenSize, windows: &[ComputerWindow]) -> ComputerUiNode {
    let calculator = windows
        .iter()
        .find(|window| window.id == "fake-calculator")
        .or_else(|| windows.first());
    let window_id = calculator.map(|window| window.id.as_str());
    let origin = calculator.and_then(|window| window.bounds);
    let (x, y) = origin.map_or((20.0, 20.0), |bounds| (bounds.x, bounds.y));
    let calculate = ui_node(
        "button",
        Some("Calculate"),
        None,
        Some("Calculate"),
        rect(x + 40.0, y + 80.0, 180.0, 56.0),
        window_id,
        Vec::new(),
    );
    let display = ComputerUiNode {
        activation_point: Some(ComputerPoint {
            x: x + 180.0,
            y: y + 44.0,
        }),
        ..ui_node(
            "text-field",
            Some("Display"),
            Some("0"),
            Some("Calculator display"),
            rect(x + 40.0, y + 20.0, 280.0, 48.0),
            window_id,
            Vec::new(),
        )
    };
    let window = ui_node(
        "window",
        Some(calculator.map_or("Calculator", |window| window.title.as_str())),
        None,
        None,
        origin.unwrap_or(rect(20.0, 20.0, 400.0, 400.0)),
        window_id,
        vec![calculate, display],
    );
    ui_node(
        "desktop",
        None,
        None,
        Some("Fake desktop"),
        rect(
            0.0,
            0.0,
            f64::from(screen_size.width),
            f64::from(screen_size.height),
        ),
        None,
        vec![window],
    )
}

fn replace_node_value(node: &mut ComputerUiNode, target: &ComputerUiNode, value: &str) {
    if node == target {
        node.value = Some(value.to_owned());
    }
    for child in &mut node.children {
        replace_node_value(child, target, value);
    }
}

#[cfg(test)]
mod tests {
    use zeron_proto::computer::{ComputerTarget, CuaComputerTask};

    use super::*;

    fn point(x: f64, y: f64) -> ComputerPoint {
        ComputerPoint { x, y }
    }

    fn display_target(node: &ComputerUiNode) -> ComputerResolvedTarget {
        ComputerResolvedTarget {
            target: ComputerTarget {
                label: Some("Display".into()),
                ..ComputerTarget::default()
            }
            .into(),
            point: node.activation_point.unwrap(),
            node: node.clone(),
        }
    }

    fn tree_options() -> ComputerStateOptions {
        ComputerStateOptions {
            include_tree: true,
            ..ComputerStateOptions::default()
        }
    }

    #[tokio::test]
    async fn records_the_full_snapshot_and_action_surface_without_a_display() {
        let backend = FakeComputerBackend::default();
        let events = Arc::new(Mutex::new(Vec::new()));
        let sink = events.clone();
        let _subscription = backend.on_event(Arc::new(move |event: &ComputerBackendEvent| {
            sink.lock().unwrap().push(event.clone());
        }));

        backend.availability(false).await.unwrap();
        backend.list_windows().await.unwrap();
        backend.get_screen_size().await.unwrap();
        backend
            .get_state(ComputerStateOptions {
                include_screenshot: true,
                include_tree: true,
                ..ComputerStateOptions::default()
            })
            .await
            .unwrap();
        backend
            .launch_app(
                "org.example.Editor",
                &["--new".to_owned()],
                ComputerLaunchOptions::default(),
            )
            .await
            .unwrap();
        let at = point(20.0, 20.0);
        backend.click(at, None, &[]).await.unwrap();
        backend.double_click(at, None, &[]).await.unwrap();
        backend.right_click(at, None, &[]).await.unwrap();
        backend.move_cursor(at, None).await.unwrap();
        backend
            .drag(at, point(30.0, 30.0), 100.0, None)
            .await
            .unwrap();
        backend
            .scroll(None, 0.0, 500.0, None, &[], None)
            .await
            .unwrap();
        backend.type_text("hello", None, None).await.unwrap();
        backend.press_key("Enter", None, None).await.unwrap();
        backend
            .hotkey(&["Control".to_owned(), "L".to_owned()], None, None)
            .await
            .unwrap();

        let methods: Vec<String> = backend
            .calls()
            .into_iter()
            .map(|call| call.method)
            .collect();
        assert_eq!(
            methods,
            [
                "availability",
                "listWindows",
                "getScreenSize",
                "getState",
                "listWindows",
                "launchApp",
                "click",
                "doubleClick",
                "rightClick",
                "moveCursor",
                "drag",
                "scroll",
                "typeText",
                "pressKey",
                "hotkey",
            ]
        );
        assert!(
            events
                .lock()
                .unwrap()
                .iter()
                .any(|event| matches!(event, ComputerBackendEvent::WindowsChanged(_)))
        );
    }

    #[tokio::test]
    async fn selects_an_exact_range_by_slicing_the_target_elements_own_value() {
        let backend = FakeComputerBackend::default();
        let state = backend.get_state(tree_options()).await.unwrap();
        let display = state.root.unwrap().children[0].children[1].clone();
        backend
            .set_value(&display_target(&display), "4680")
            .await
            .unwrap();

        let refreshed = backend.get_state(tree_options()).await.unwrap();
        let updated = refreshed.root.unwrap().children[0].children[1].clone();
        let result = backend
            .select_text(
                &display_target(&updated),
                ComputerTextRange {
                    start: 1,
                    length: 2,
                },
            )
            .await
            .unwrap();

        assert_eq!(result.value.as_deref(), Some("68"));
        let calls = backend.calls_for("selectText");
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].args[1], json!({ "start": 1, "length": 2 }));
    }

    #[tokio::test]
    async fn emits_deterministic_codec_config_and_keyframe_frames_and_supports_failures() {
        let backend = FakeComputerBackend::default();
        let frames = Arc::new(Mutex::new(Vec::new()));
        let sink = frames.clone();
        backend
            .attach_stream(Arc::new(move |frame: ComputerStreamFrame| {
                sink.lock()
                    .unwrap()
                    .push((frame.keyframe, frame.codec_config));
            }))
            .await
            .unwrap();
        assert_eq!(frames.lock().unwrap()[..2], [(true, true), (true, false)]);

        backend.fail_next_with(
            "click",
            ComputerBackendError::new("synthetic click failure"),
        );
        let error = backend.click(point(1.0, 1.0), None, &[]).await.unwrap_err();
        assert_eq!(error.to_string(), "synthetic click failure");
        assert_eq!(
            backend.click(point(1.0, 1.0), None, &[]).await.unwrap(),
            ComputerBackendActionResult {
                point: Some(point(1.0, 1.0)),
                ..ComputerBackendActionResult::default()
            }
        );

        backend.request_keyframe().await.unwrap();
        assert_eq!(frames.lock().unwrap().len(), 4);
        backend.detach_stream().await.unwrap();
        backend.emit_frame(true, true, vec![0x01]);
        assert_eq!(frames.lock().unwrap().len(), 4);
    }

    #[tokio::test]
    async fn caps_recorded_calls_at_a_bounded_recent_window() {
        let backend = FakeComputerBackend::default();
        for index in 0..1_500 {
            backend
                .press_key(&format!("key-{index}"), None, None)
                .await
                .unwrap();
        }
        let calls = backend.calls();
        assert_eq!(calls.len(), 1_000);
        assert_eq!(calls[0].args, [json!("key-500")]);
        assert_eq!(calls.last().unwrap().args, [json!("key-1499")]);
    }

    #[test]
    fn default_windows_and_root_round_trip_through_the_proto_deserializers() {
        let backend = FakeComputerBackend::default();
        let state = backend.state();
        let windows: Vec<ComputerWindow> =
            serde_json::from_value(serde_json::to_value(&state.windows).unwrap()).unwrap();
        assert_eq!(windows, state.windows);
        let root: ComputerUiNode =
            serde_json::from_value(serde_json::to_value(&state.root).unwrap()).unwrap();
        assert_eq!(root, state.root);
        let apps: Vec<ComputerApp> =
            serde_json::from_value(serde_json::to_value(&state.apps).unwrap()).unwrap();
        assert_eq!(apps, state.apps);
        assert_eq!(
            state
                .windows
                .iter()
                .map(|w| w.id.as_str())
                .collect::<Vec<_>>(),
            ["fake-terminal", "fake-calculator"]
        );
        assert_eq!(
            state.apps.iter().map(|app| app.pid).collect::<Vec<_>>(),
            [1_001, 1_002]
        );
    }

    #[tokio::test]
    async fn opt_in_surfaces_exist_only_when_asked_for() {
        let bare = FakeComputerBackend::default();
        assert!(bare.supports(ComputerBackendMethod::ListApps));
        assert!(!bare.supports(ComputerBackendMethod::WaitForSettle));
        assert!(!bare.supports(ComputerBackendMethod::EngageShield));
        assert!(!bare.supports(ComputerBackendMethod::Provision));
        assert!(bare.browser().is_none());

        let backend = FakeComputerBackend::new(FakeComputerBackendOptions {
            browser: FakeComputerOptIn::BuiltIn,
            wait_for_settle: FakeComputerOptIn::BuiltIn,
            shield: FakeComputerOptIn::BuiltIn,
            ..FakeComputerBackendOptions::default()
        });
        assert!(backend.supports(ComputerBackendMethod::WaitForSettle));
        assert!(backend.supports(ComputerBackendMethod::ReleaseAllShields));

        let result = backend
            .browser()
            .unwrap()
            .call(ComputerBrowserCall {
                name: "get_browser_state".into(),
                args: serde_json::Map::new(),
                task: CuaComputerTask {
                    thread_id: "t1".into(),
                    turn_id: None,
                    label: None,
                },
                mutation: false,
                signal: None,
            })
            .await
            .unwrap();
        assert_eq!(
            result.structured_content.unwrap()["target_id"],
            json!("fake-browser-t1")
        );
        assert_eq!(backend.calls_for("browser.get_browser_state").len(), 1);

        let missing = backend
            .wait_for_settle(ComputerSettleOptions {
                window_id: "nope".into(),
                timeout_ms: 100.0,
                quiet_ms: 10.0,
            })
            .await
            .unwrap_err();
        assert_eq!(missing.code(), Some("computer_target_not_found"));

        let shield = ComputerShieldTarget {
            shield_id: "s1".into(),
            window_id: "fake-terminal".into(),
            frame: rect(0.0, 0.0, 10.0, 10.0),
            label: "Terminal".into(),
        };
        assert_eq!(backend.engage_shield(&shield).await.unwrap(), "s1");
        assert_eq!(backend.active_shields(), ["s1"]);
        backend.release_shield("s1").await.unwrap();
        assert!(backend.active_shields().is_empty());
    }
}
