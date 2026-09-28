use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use futures::FutureExt;
use futures::future::{BoxFuture, Shared};
use serde_json::{Value, json};
use tokio::sync::oneshot;
use zeron_proto::PermissionMode;
use zeron_proto::computer::{
    ComputerApp, ComputerAvailability, ComputerBuildSignature, ComputerCapabilities, ComputerEvent,
    ComputerHealth, ComputerInputModifier, ComputerLaunchAppResult, ComputerPermission,
    ComputerPoint, ComputerRect, ComputerScreenSize, ComputerScreenshot, ComputerSpaceInventory,
    ComputerState, ComputerTarget, ComputerUiNode, ComputerVerifyStateResult, ComputerWindow,
    ComputerZoomResult, CuaComputerTask, ThreadComputerState,
};

use super::{
    ComputerConsentCard, ComputerLeaseRecord, ComputerManager, ComputerManagerEnv,
    ComputerManagerEvent, ComputerManagerOptions, ComputerManagerSeams,
    ComputerManagerSubscription,
};
use crate::computer::approval_gate::ApprovalDecision;
use crate::computer::backend::{
    ComputerAccessibilityTreeSnapshot, ComputerAgentDialect, ComputerBackend,
    ComputerBackendActionResult, ComputerBackendEventListener, ComputerBackendMethod,
    ComputerBackendSubscription, ComputerBrowserBackend, ComputerCaptureRequest,
    ComputerCursorReading, ComputerError, ComputerFrameListener, ComputerLaunchOptions,
    ComputerMenuBackendTarget, ComputerResolvedTarget, ComputerSettleOptions,
    ComputerSettleOutcome, ComputerShieldTarget, ComputerStateOptions, ComputerTextRange,
};
use crate::computer::fake_backend::{FakeComputerBackend, FakeComputerBackendOptions};

mod approval_lease;
mod authority;
mod background;
mod consent;
mod control_invariants;
mod denylist_audit;
mod detached;
mod emergency_stop;
mod foreground;
mod gateway;
mod lease;
mod left_open;
mod observe;
mod provision;
mod publish;
mod revoke;
mod scroll;
mod spaces;
mod targeting;

pub(crate) type Hook<A, T> = Arc<
    dyn Fn(Arc<FakeComputerBackend>, A) -> BoxFuture<'static, Result<T, ComputerError>>
        + Send
        + Sync,
>;

type KeyArgs = (String, Option<String>, Option<ComputerResolvedTarget>);
type ChordArgs = (Vec<String>, Option<String>, Option<ComputerResolvedTarget>);
type ScrollArgs = (
    Option<ComputerPoint>,
    f64,
    f64,
    Option<String>,
    Vec<ComputerInputModifier>,
    Option<ComputerResolvedTarget>,
);

#[derive(Default)]
pub(crate) struct Hooks {
    pub press_key: Option<Hook<KeyArgs, ComputerBackendActionResult>>,
    pub hotkey: Option<Hook<ChordArgs, ComputerBackendActionResult>>,
    pub type_text: Option<Hook<KeyArgs, ComputerBackendActionResult>>,
    pub click: Option<Hook<(ComputerPoint, Option<String>), ComputerBackendActionResult>>,
    pub perform_action: Option<Hook<(ComputerResolvedTarget, String), ComputerBackendActionResult>>,
    pub set_value: Option<Hook<(ComputerResolvedTarget, String), ComputerBackendActionResult>>,
    pub launch_app: Option<Hook<LaunchArgs, ComputerLaunchAppResult>>,
    pub get_state: Option<Hook<ComputerStateOptions, ComputerState>>,
    pub list_windows: Option<Hook<(), Vec<ComputerWindow>>>,
    pub raise_window: Option<Hook<String, ()>>,
    pub scroll: Option<Hook<ScrollArgs, ComputerBackendActionResult>>,
    pub detach_stream: Option<Hook<(), ()>>,
    pub probe_availability: Option<Hook<(), ComputerAvailability>>,
    pub stop_input: Option<Hook<Option<CuaComputerTask>, ()>>,
    pub end_task: Option<Hook<(String, Option<String>), ()>>,
    pub set_driving_agent: Option<Hook<Option<String>, ()>>,
    pub check_input_ready: Option<Hook<String, ()>>,
    pub focus_window: Option<Hook<String, ()>>,
    pub provision: Option<Hook<(), String>>,
    pub list_spaces: Option<Hook<(), ComputerSpaceInventory>>,
    pub supports_action: Option<SupportsAction>,
    pub capture_screenshot: Option<Hook<ComputerCaptureRequest, ComputerScreenshot>>,
}

type LaunchArgs = (String, Vec<String>, ComputerLaunchOptions);
type SupportsAction = Arc<dyn Fn(&ComputerResolvedTarget, &str) -> bool + Send + Sync>;
pub(crate) type TypedTarget = (String, Option<ComputerResolvedTarget>);

pub(crate) struct Scripted {
    pub fake: Arc<FakeComputerBackend>,
    pub hooks: Mutex<Hooks>,
    pub exact_target_background_input: AtomicBool,
    pub focus_neutral_semantic_text: AtomicBool,
    pub dialect: Mutex<Option<ComputerAgentDialect>>,
    pub unsupported: Mutex<HashSet<ComputerBackendMethod>>,
    pub proof: Mutex<ComputerBackendActionResult>,
    pub log: Mutex<Vec<(String, Value)>>,
}

impl Scripted {
    pub fn new(options: FakeComputerBackendOptions) -> Arc<Self> {
        Arc::new(Self {
            fake: Arc::new(FakeComputerBackend::new(options)),
            hooks: Mutex::default(),
            exact_target_background_input: AtomicBool::new(false),
            focus_neutral_semantic_text: AtomicBool::new(false),
            dialect: Mutex::new(None),
            unsupported: Mutex::default(),
            proof: Mutex::default(),
            log: Mutex::default(),
        })
    }

    pub fn default_fake() -> Arc<Self> {
        Self::new(FakeComputerBackendOptions::default())
    }

    pub fn hooks(&self) -> std::sync::MutexGuard<'_, Hooks> {
        self.hooks.lock().unwrap()
    }

    pub fn set_dialect(&self, dialect: ComputerAgentDialect) {
        *self.dialect.lock().unwrap() = Some(dialect);
    }

    pub fn set_background_target(&self, exact: bool, focus_neutral: bool) {
        self.exact_target_background_input
            .store(exact, Ordering::SeqCst);
        self.focus_neutral_semantic_text
            .store(focus_neutral, Ordering::SeqCst);
    }

    pub fn remove(&self, method: ComputerBackendMethod) {
        self.unsupported.lock().unwrap().insert(method);
    }

    pub fn set_proof(&self, proof: ComputerBackendActionResult) {
        *self.proof.lock().unwrap() = proof;
    }

    pub fn calls_for(&self, method: &str) -> Vec<crate::computer::fake_backend::FakeComputerCall> {
        self.fake.calls_for(method)
    }

    pub fn methods(&self) -> Vec<String> {
        self.fake
            .calls()
            .into_iter()
            .map(|call| call.method)
            .collect()
    }

    pub fn logged(&self, method: &str) -> Vec<Value> {
        self.log
            .lock()
            .unwrap()
            .iter()
            .filter(|(name, _)| name == method)
            .map(|(_, args)| args.clone())
            .collect()
    }

    fn record(&self, method: &str, args: Value) {
        self.log.lock().unwrap().push((method.to_owned(), args));
    }
}

macro_rules! hooked {
    ($self:ident, $name:ident, $args:expr, $fallback:expr) => {{
        let hook = $self.hooks().$name.clone();
        match hook {
            Some(hook) => hook($self.fake.clone(), $args).await,
            None => $fallback,
        }
    }};
}

#[async_trait]
impl ComputerBackend for Scripted {
    fn agent_dialect(&self) -> ComputerAgentDialect {
        self.dialect
            .lock()
            .unwrap()
            .unwrap_or_else(|| self.fake.agent_dialect())
    }

    fn computer_id(&self) -> &str {
        self.fake.computer_id()
    }

    fn supports(&self, method: ComputerBackendMethod) -> bool {
        if self.unsupported.lock().unwrap().contains(&method) {
            return false;
        }
        match method {
            ComputerBackendMethod::CheckInputReady => self.hooks().check_input_ready.is_some(),
            ComputerBackendMethod::Provision => self.hooks().provision.is_some(),
            ComputerBackendMethod::ListSpaces => self.hooks().list_spaces.is_some(),
            _ => self.fake.supports(method),
        }
    }

    async fn probe_availability(&self) -> Result<ComputerAvailability, ComputerError> {
        hooked!(
            self,
            probe_availability,
            (),
            self.fake.probe_availability().await
        )
    }

    async fn availability(&self, refresh: bool) -> Result<ComputerAvailability, ComputerError> {
        self.fake.availability(refresh).await
    }

    fn health(&self) -> ComputerHealth {
        self.fake.health()
    }

    fn capabilities(&self) -> ComputerCapabilities {
        self.fake.capabilities()
    }

    async fn missing_permissions(&self) -> Vec<ComputerPermission> {
        self.fake.missing_permissions().await
    }

    fn build_signature(&self) -> Option<ComputerBuildSignature> {
        self.fake.build_signature()
    }

    async fn list_windows(&self) -> Result<Vec<ComputerWindow>, ComputerError> {
        hooked!(self, list_windows, (), self.fake.list_windows().await)
    }

    async fn list_spaces(&self) -> Result<ComputerSpaceInventory, ComputerError> {
        hooked!(self, list_spaces, (), self.fake.list_spaces().await)
    }

    async fn provision(&self) -> Result<String, ComputerError> {
        hooked!(self, provision, (), self.fake.provision().await)
    }

    async fn get_screen_size(&self) -> Result<ComputerScreenSize, ComputerError> {
        self.fake.get_screen_size().await
    }

    async fn get_state(
        &self,
        options: ComputerStateOptions,
    ) -> Result<ComputerState, ComputerError> {
        hooked!(
            self,
            get_state,
            options.clone(),
            self.fake.get_state(options).await
        )
    }

    async fn capture_screenshot(
        &self,
        request: ComputerCaptureRequest,
    ) -> Result<ComputerScreenshot, ComputerError> {
        hooked!(
            self,
            capture_screenshot,
            request.clone(),
            self.fake.capture_screenshot(request).await
        )
    }

    async fn focus_window(&self, window_id: &str) -> Result<(), ComputerError> {
        hooked!(
            self,
            focus_window,
            window_id.to_owned(),
            self.fake.focus_window(window_id).await
        )
    }

    async fn raise_window(&self, window_id: &str) -> Result<(), ComputerError> {
        hooked!(
            self,
            raise_window,
            window_id.to_owned(),
            self.fake.raise_window(window_id).await
        )
    }

    async fn clear_focus_window(&self) -> Result<(), ComputerError> {
        self.fake.clear_focus_window().await
    }

    async fn set_driving_agent(&self, name: Option<&str>) -> Result<(), ComputerError> {
        self.record("setDrivingAgent", json!(name));
        hooked!(
            self,
            set_driving_agent,
            name.map(str::to_owned),
            self.fake.set_driving_agent(name).await
        )
    }

    async fn launch_app(
        &self,
        app: &str,
        args: &[String],
        options: ComputerLaunchOptions,
    ) -> Result<ComputerLaunchAppResult, ComputerError> {
        self.record("launchApp", json!(app));
        hooked!(
            self,
            launch_app,
            (app.to_owned(), args.to_vec(), options),
            self.fake.launch_app(app, args, options).await
        )
    }

    async fn check_input_ready(&self, window_id: &str) -> Result<(), ComputerError> {
        self.record("checkInputReady", json!(window_id));
        hooked!(self, check_input_ready, window_id.to_owned(), Ok(()))
    }

    async fn wait_for_settle(
        &self,
        options: ComputerSettleOptions,
    ) -> Result<ComputerSettleOutcome, ComputerError> {
        self.fake.wait_for_settle(options).await
    }

    async fn list_apps(&self) -> Result<Vec<ComputerApp>, ComputerError> {
        self.fake.list_apps().await
    }

    async fn set_window_frame(
        &self,
        window_id: &str,
        frame: ComputerRect,
    ) -> Result<ComputerBackendActionResult, ComputerError> {
        self.fake.set_window_frame(window_id, frame).await
    }

    async fn invoke_menu(
        &self,
        target: &ComputerMenuBackendTarget,
        path: &[String],
    ) -> Result<ComputerBackendActionResult, ComputerError> {
        self.fake.invoke_menu(target, path).await
    }

    async fn verify_state(
        &self,
        window_id: &str,
        expect: &[Value],
    ) -> Result<ComputerVerifyStateResult, ComputerError> {
        self.fake.verify_state(window_id, expect).await
    }

    async fn zoom_window(
        &self,
        window_id: &str,
        region: ComputerRect,
    ) -> Result<ComputerZoomResult, ComputerError> {
        self.fake.zoom_window(window_id, region).await
    }

    async fn get_accessibility_tree(
        &self,
        window_id: Option<&str>,
    ) -> Result<ComputerAccessibilityTreeSnapshot, ComputerError> {
        self.fake.get_accessibility_tree(window_id).await
    }

    async fn get_cursor_position(
        &self,
        window_id: Option<&str>,
    ) -> Result<ComputerCursorReading, ComputerError> {
        self.fake.get_cursor_position(window_id).await
    }

    async fn set_window_minimized(
        &self,
        window_id: &str,
        minimized: bool,
    ) -> Result<ComputerBackendActionResult, ComputerError> {
        self.fake.set_window_minimized(window_id, minimized).await
    }

    async fn set_app_visibility(
        &self,
        pid: u32,
        hidden: bool,
    ) -> Result<ComputerBackendActionResult, ComputerError> {
        self.fake.set_app_visibility(pid, hidden).await
    }

    async fn kill_app(&self, pid: u32) -> Result<ComputerBackendActionResult, ComputerError> {
        self.fake.kill_app(pid).await
    }

    fn supports_action(&self, target: &ComputerResolvedTarget, action: &str) -> bool {
        match self.hooks().supports_action.clone() {
            Some(hook) => hook(target, action),
            None => self.fake.supports_action(target, action),
        }
    }

    async fn click(
        &self,
        point: ComputerPoint,
        window_id: Option<&str>,
        modifiers: &[ComputerInputModifier],
    ) -> Result<ComputerBackendActionResult, ComputerError> {
        hooked!(
            self,
            click,
            (point, window_id.map(str::to_owned)),
            self.fake.click(point, window_id, modifiers).await
        )
    }

    async fn double_click(
        &self,
        point: ComputerPoint,
        window_id: Option<&str>,
        modifiers: &[ComputerInputModifier],
    ) -> Result<ComputerBackendActionResult, ComputerError> {
        self.fake.double_click(point, window_id, modifiers).await
    }

    async fn triple_click(
        &self,
        point: ComputerPoint,
        window_id: Option<&str>,
        modifiers: &[ComputerInputModifier],
    ) -> Result<ComputerBackendActionResult, ComputerError> {
        self.fake.triple_click(point, window_id, modifiers).await
    }

    async fn right_click(
        &self,
        point: ComputerPoint,
        window_id: Option<&str>,
        modifiers: &[ComputerInputModifier],
    ) -> Result<ComputerBackendActionResult, ComputerError> {
        self.fake.right_click(point, window_id, modifiers).await
    }

    async fn move_cursor(
        &self,
        point: ComputerPoint,
        window_id: Option<&str>,
    ) -> Result<ComputerBackendActionResult, ComputerError> {
        self.fake.move_cursor(point, window_id).await
    }

    async fn drag(
        &self,
        from: ComputerPoint,
        to: ComputerPoint,
        duration_ms: f64,
        window_id: Option<&str>,
    ) -> Result<ComputerBackendActionResult, ComputerError> {
        self.fake.drag(from, to, duration_ms, window_id).await
    }

    async fn scroll(
        &self,
        point: Option<ComputerPoint>,
        delta_x: f64,
        delta_y: f64,
        window_id: Option<&str>,
        modifiers: &[ComputerInputModifier],
        target: Option<&ComputerResolvedTarget>,
    ) -> Result<ComputerBackendActionResult, ComputerError> {
        hooked!(
            self,
            scroll,
            (
                point,
                delta_x,
                delta_y,
                window_id.map(str::to_owned),
                modifiers.to_vec(),
                target.cloned()
            ),
            self.fake
                .scroll(point, delta_x, delta_y, window_id, modifiers, target)
                .await
        )
    }

    fn focus_neutral_semantic_text(&self) -> bool {
        self.focus_neutral_semantic_text.load(Ordering::SeqCst)
    }

    fn exact_target_background_input(&self) -> bool {
        self.exact_target_background_input.load(Ordering::SeqCst)
    }

    async fn type_text(
        &self,
        text: &str,
        window_id: Option<&str>,
        target: Option<&ComputerResolvedTarget>,
    ) -> Result<ComputerBackendActionResult, ComputerError> {
        self.record("typeText", json!(text));
        hooked!(
            self,
            type_text,
            (
                text.to_owned(),
                window_id.map(str::to_owned),
                target.cloned()
            ),
            self.fake.type_text(text, window_id, target).await
        )
    }

    async fn press_key(
        &self,
        key: &str,
        window_id: Option<&str>,
        target: Option<&ComputerResolvedTarget>,
    ) -> Result<ComputerBackendActionResult, ComputerError> {
        self.record(
            "pressKey",
            json!({ "key": key, "windowId": window_id, "target": target.map(|t| json!(t.node)) }),
        );
        let result = hooked!(
            self,
            press_key,
            (
                key.to_owned(),
                window_id.map(str::to_owned),
                target.cloned()
            ),
            self.fake.press_key(key, window_id, target).await
        )?;
        let proof = self.proof.lock().unwrap().clone();
        Ok(ComputerBackendActionResult {
            effect: proof.effect.or(result.effect),
            verified: proof.verified.or(result.verified),
            delivery_path: proof.delivery_path.or(result.delivery_path),
            ..result
        })
    }

    async fn hotkey(
        &self,
        keys: &[String],
        window_id: Option<&str>,
        target: Option<&ComputerResolvedTarget>,
    ) -> Result<ComputerBackendActionResult, ComputerError> {
        hooked!(
            self,
            hotkey,
            (keys.to_vec(), window_id.map(str::to_owned), target.cloned()),
            self.fake.hotkey(keys, window_id, target).await
        )
    }

    async fn read_clipboard(&self) -> Result<String, ComputerError> {
        self.fake.read_clipboard().await
    }

    async fn write_clipboard(&self, text: &str) -> Result<(), ComputerError> {
        self.fake.write_clipboard(text).await
    }

    async fn set_value(
        &self,
        target: &ComputerResolvedTarget,
        value: &str,
    ) -> Result<ComputerBackendActionResult, ComputerError> {
        hooked!(
            self,
            set_value,
            (target.clone(), value.to_owned()),
            self.fake.set_value(target, value).await
        )
    }

    async fn perform_action(
        &self,
        target: &ComputerResolvedTarget,
        action: &str,
    ) -> Result<ComputerBackendActionResult, ComputerError> {
        self.record("performAction", json!(action));
        hooked!(
            self,
            perform_action,
            (target.clone(), action.to_owned()),
            self.fake.perform_action(target, action).await
        )
    }

    async fn select_text(
        &self,
        target: &ComputerResolvedTarget,
        range: ComputerTextRange,
    ) -> Result<ComputerBackendActionResult, ComputerError> {
        self.fake.select_text(target, range).await
    }

    fn on_event(&self, listener: ComputerBackendEventListener) -> ComputerBackendSubscription {
        self.fake.on_event(listener)
    }

    async fn attach_stream(&self, listener: ComputerFrameListener) -> Result<(), ComputerError> {
        self.fake.attach_stream(listener).await
    }

    async fn detach_stream(&self) -> Result<(), ComputerError> {
        hooked!(self, detach_stream, (), self.fake.detach_stream().await)
    }

    async fn request_keyframe(&self) -> Result<(), ComputerError> {
        self.fake.request_keyframe().await
    }

    async fn engage_shield(&self, target: &ComputerShieldTarget) -> Result<String, ComputerError> {
        self.fake.engage_shield(target).await
    }

    async fn release_shield(&self, shield_id: &str) -> Result<(), ComputerError> {
        self.fake.release_shield(shield_id).await
    }

    async fn release_all_shields(&self) -> Result<(), ComputerError> {
        self.fake.release_all_shields().await
    }

    async fn stop_input(&self, task: Option<&CuaComputerTask>) -> Result<(), ComputerError> {
        self.record("stopInput", json!(task));
        hooked!(
            self,
            stop_input,
            task.cloned(),
            self.fake.stop_input(task).await
        )
    }

    async fn end_task(&self, thread_id: &str, turn_id: Option<&str>) -> Result<(), ComputerError> {
        self.record("endTask", json!([thread_id, turn_id]));
        hooked!(
            self,
            end_task,
            (thread_id.to_owned(), turn_id.map(str::to_owned)),
            self.fake.end_task(thread_id, turn_id).await
        )
    }

    fn browser(&self) -> Option<&dyn ComputerBrowserBackend> {
        self.fake.browser()
    }

    fn native_element_identity(&self, node: &ComputerUiNode) -> Option<String> {
        self.fake.native_element_identity(node)
    }

    async fn dispose(&self) {
        self.fake.dispose().await;
    }
}

#[derive(Default)]
pub(crate) struct Seams {
    pub mode: Mutex<Option<PermissionMode>>,
    pub turn_inactive: AtomicBool,
    pub consent: Mutex<Vec<(ComputerConsentCard, Option<ApprovalDecision>)>>,
    pub bumps: Mutex<Vec<(String, u64)>>,
    pub leases: Mutex<Vec<ComputerLeaseRecord>>,
    pub warnings: Mutex<Vec<(String, Value)>>,
    pub notes: Mutex<Vec<(String, String)>>,
}

#[async_trait]
impl ComputerManagerSeams for Seams {
    async fn caller(
        &self,
        _chat_id: &str,
        _turn_id: Option<&str>,
    ) -> Result<Option<PermissionMode>, ComputerError> {
        if self.turn_inactive.load(Ordering::SeqCst) {
            return Err(ComputerError::Other("original turn ended".into()));
        }
        Ok(*self.mode.lock().unwrap())
    }

    async fn publish_consent(
        &self,
        card: ComputerConsentCard,
        decision: Option<ApprovalDecision>,
    ) -> Result<(), String> {
        self.consent.lock().unwrap().push((card, decision));
        Ok(())
    }

    fn generation_bumped(&self, chat_id: &str, generation: u64) {
        self.bumps
            .lock()
            .unwrap()
            .push((chat_id.to_owned(), generation));
    }

    fn lease_lifecycle(&self, record: &ComputerLeaseRecord) {
        self.leases.lock().unwrap().push(record.clone());
    }

    fn warn(&self, message: &str, detail: Value) {
        self.warnings
            .lock()
            .unwrap()
            .push((message.to_owned(), detail));
    }

    fn note(&self, chat_id: &str, text: &str) {
        self.notes
            .lock()
            .unwrap()
            .push((chat_id.to_owned(), text.to_owned()));
    }
}

pub(crate) struct Built {
    pub manager: Arc<ComputerManager>,
    pub backend: Arc<Scripted>,
    pub seams: Arc<Seams>,
}

#[derive(Default)]
pub(crate) struct Setup {
    pub action_settle_ms: Option<u64>,
    pub now: Option<super::ManagerClock>,
    pub lease_idle_ms: Option<u64>,
    pub windows_publish_debounce_ms: Option<u64>,
    pub measure_scroll_travel: Option<super::MeasureScrollTravel>,
    pub env: ComputerManagerEnv,
    pub data_dir: Option<std::path::PathBuf>,
}

pub(crate) fn build(backend: Arc<Scripted>, setup: Setup) -> Built {
    let seams = Arc::new(Seams::default());
    let mut options = ComputerManagerOptions::new(backend.clone());
    options.seams = seams.clone();
    options.env = setup.env;
    options.action_settle_ms = setup.action_settle_ms;
    options.now = setup.now;
    options.data_dir = setup.data_dir;
    options.measure_scroll_travel = setup.measure_scroll_travel;
    if let Some(ms) = setup.lease_idle_ms {
        options.lease_idle_ms = ms;
    }
    if let Some(ms) = setup.windows_publish_debounce_ms {
        options.windows_publish_debounce_ms = ms;
    }
    Built {
        manager: ComputerManager::new(options),
        backend,
        seams,
    }
}

pub(crate) fn manager(backend: Arc<Scripted>) -> Built {
    build(backend, Setup::default())
}

pub(crate) fn settled(backend: Arc<Scripted>) -> Built {
    build(
        backend,
        Setup {
            action_settle_ms: Some(0),
            ..Setup::default()
        },
    )
}

pub(crate) type Clock = Arc<Mutex<u64>>;

pub(crate) fn clock(start: u64) -> (Clock, super::ManagerClock) {
    let now = Arc::new(Mutex::new(start));
    let read = now.clone();
    (now, Arc::new(move || *read.lock().unwrap()))
}

pub(crate) struct Events {
    pub events: Arc<Mutex<Vec<ComputerManagerEvent>>>,
    _subscription: ComputerManagerSubscription,
}

impl Events {
    pub fn all(&self) -> Vec<ComputerManagerEvent> {
        self.events.lock().unwrap().clone()
    }

    pub fn states(&self) -> Vec<ThreadComputerState> {
        self.all()
            .into_iter()
            .filter_map(|event| match event {
                ComputerEvent::ThreadState { state } => Some(*state),
                _ => None,
            })
            .collect()
    }

    pub fn states_for(&self, thread_id: &str) -> Vec<ThreadComputerState> {
        self.states()
            .into_iter()
            .filter(|state| state.thread_id == thread_id)
            .collect()
    }

    pub fn actions(&self) -> Vec<ComputerManagerEvent> {
        self.all()
            .into_iter()
            .filter(|event| matches!(event, ComputerEvent::Action { .. }))
            .collect()
    }

    pub fn open_pane_requests(&self) -> Vec<String> {
        self.all()
            .into_iter()
            .filter_map(|event| match event {
                ComputerEvent::OpenPaneRequested { thread_id } => Some(thread_id),
                _ => None,
            })
            .collect()
    }

    pub fn types(&self) -> Vec<&'static str> {
        self.all()
            .iter()
            .map(|event| match event {
                ComputerEvent::ThreadState { .. } => "computer.thread-state",
                ComputerEvent::WindowsChanged { .. } => "computer.windows-changed",
                ComputerEvent::Action { .. } => "computer.action",
                ComputerEvent::Frame { .. } => "computer.frame",
                ComputerEvent::OpenPaneRequested { .. } => "computer.open-pane-requested",
                ComputerEvent::InputStopped { .. } => "computer.input-stopped",
            })
            .collect()
    }
}

pub(crate) fn events(manager: &ComputerManager) -> Events {
    let events: Arc<Mutex<Vec<ComputerManagerEvent>>> = Arc::default();
    let sink = events.clone();
    let subscription = manager.on_event(Arc::new(move |event: &ComputerManagerEvent| {
        sink.lock().unwrap().push(event.clone());
    }));
    Events {
        events,
        _subscription: subscription,
    }
}

pub(crate) struct Deferred {
    sender: Mutex<Option<oneshot::Sender<()>>>,
    pub wait: Shared<BoxFuture<'static, ()>>,
}

impl Deferred {
    pub fn resolve(&self) {
        if let Some(sender) = self.sender.lock().unwrap().take() {
            let _ = sender.send(());
        }
    }

    pub fn wait(&self) -> Shared<BoxFuture<'static, ()>> {
        self.wait.clone()
    }
}

pub(crate) fn deferred() -> Arc<Deferred> {
    let (sender, receiver) = oneshot::channel::<()>();
    Arc::new(Deferred {
        sender: Mutex::new(Some(sender)),
        wait: receiver.map(|_| ()).boxed().shared(),
    })
}

pub(crate) async fn settle() {
    for _ in 0..64 {
        tokio::task::yield_now().await;
    }
}

pub(crate) fn point(x: f64, y: f64) -> ComputerTarget {
    ComputerTarget {
        x: Some(x),
        y: Some(y),
        ..ComputerTarget::default()
    }
}

pub(crate) fn scoped(x: f64, y: f64, window_id: &str) -> ComputerTarget {
    ComputerTarget {
        window_id: Some(window_id.to_owned()),
        ..point(x, y)
    }
}

pub(crate) fn labelled(label: &str, role: Option<&str>) -> ComputerTarget {
    ComputerTarget {
        label: Some(label.to_owned()),
        role: role.map(str::to_owned),
        ..ComputerTarget::default()
    }
}

pub(crate) fn rect(x: f64, y: f64, width: f64, height: f64) -> ComputerRect {
    ComputerRect {
        x,
        y,
        width,
        height,
    }
}

pub(crate) fn window(id: &str, title: &str, bounds: Option<ComputerRect>) -> ComputerWindow {
    ComputerWindow {
        id: id.to_owned(),
        title: title.to_owned(),
        app_name: None,
        pid: None,
        bounds,
        focused: false,
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

pub(crate) fn app(pid: u32, name: &str, bundle_id: &str, active: bool) -> ComputerApp {
    ComputerApp {
        pid,
        name: name.to_owned(),
        bundle_id: Some(bundle_id.to_owned()),
        team_id: None,
        running: true,
        active,
        launch_path: None,
        window_count: None,
        last_used: None,
    }
}

pub(crate) fn covered_calculator_windows() -> Vec<ComputerWindow> {
    vec![
        ComputerWindow {
            focused: true,
            stacking_index: Some(0),
            occluded_by: Some(Vec::new()),
            ..window(
                "fake-browser",
                "Browser",
                Some(rect(0.0, 0.0, 1_920.0, 1_080.0)),
            )
        },
        ComputerWindow {
            stacking_index: Some(1),
            occluded_by: Some(vec!["fake-browser".into()]),
            ..window(
                "fake-calculator",
                "Calculator",
                Some(rect(1_050.0, 120.0, 420.0, 620.0)),
            )
        },
    ]
}

fn ui_node(
    role: &str,
    label: Option<&str>,
    value: Option<&str>,
    frame: ComputerRect,
    activation: Option<ComputerPoint>,
    window_id: Option<&str>,
    children: Vec<ComputerUiNode>,
) -> ComputerUiNode {
    ComputerUiNode {
        role: role.to_owned(),
        label: label.map(str::to_owned),
        value: value.map(str::to_owned),
        description: None,
        frame,
        activation_point: activation,
        on_screen: true,
        window_id: window_id.map(str::to_owned),
        accessibility_root: None,
        node_path: None,
        editable: None,
        truncated: None,
        children,
    }
}

pub(crate) fn semantic_text_root(window_ids: &[&str]) -> ComputerUiNode {
    let children = window_ids
        .iter()
        .enumerate()
        .map(|(index, window_id)| {
            let x = index as f64 * 400.0;
            ui_node(
                "AXWindow",
                Some(&format!("Window {}", index + 1)),
                None,
                rect(x, 0.0, 360.0, 300.0),
                None,
                Some(window_id),
                vec![ui_node(
                    "AXTextArea",
                    None,
                    Some(""),
                    rect(x + 20.0, 40.0, 320.0, 220.0),
                    Some(ComputerPoint {
                        x: x + 180.0,
                        y: 150.0,
                    }),
                    Some(window_id),
                    Vec::new(),
                )],
            )
        })
        .collect();
    ComputerUiNode {
        description: Some("Semantic text test desktop".into()),
        ..ui_node(
            "desktop",
            None,
            None,
            rect(0.0, 0.0, 1_920.0, 1_080.0),
            None,
            None,
            children,
        )
    }
}

pub(crate) fn background_target_backend() -> Arc<Scripted> {
    let ids = ["editor-a", "editor-b", "editor-a-other"];
    let windows = ids
        .iter()
        .enumerate()
        .map(|(index, id)| ComputerWindow {
            app_name: Some(if index == 1 { "Editor B" } else { "Editor A" }.into()),
            pid: Some(if index == 1 { 220 } else { 110 }),
            ..window(id, id, Some(rect(index as f64 * 400.0, 0.0, 360.0, 300.0)))
        })
        .collect();
    let backend = Scripted::new(FakeComputerBackendOptions {
        windows: Some(windows),
        root: Some(semantic_text_root(&ids)),
        apps: Some(vec![
            app(110, "Editor A", "app.editor.a", false),
            app(220, "Editor B", "app.editor.b", false),
        ]),
        ..FakeComputerBackendOptions::default()
    });
    backend.set_background_target(true, true);
    backend.set_dialect(ComputerAgentDialect::Macos);
    backend
}

pub(crate) fn visible_use() -> Option<crate::computer::visible_use::ComputerForegroundAuthorization>
{
    Some(
        crate::computer::visible_use::ComputerForegroundAuthorization {
            user_requested_visible_use: true,
        },
    )
}

pub(crate) fn code(error: &ComputerError) -> Option<&str> {
    error.code()
}
