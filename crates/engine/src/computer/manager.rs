mod actions;
mod authority;
mod consent_app;
mod foreground;
mod lease;
mod observe;
mod publish;
mod stream;
mod targeting;
#[cfg(test)]
pub(crate) mod tests;

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, Weak};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use futures::FutureExt;
use futures::future::{BoxFuture, Shared};
use serde_json::Value;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use zeron_proto::PermissionMode;
use zeron_proto::computer::{
    ComputerApp, ComputerAvailability, ComputerEvent, ComputerHealth, ComputerHealthStatus,
    ComputerInputModifier, ComputerInputPause, ComputerPoint, ComputerRect, ComputerScreenSize,
    ComputerScreenshot, ComputerWindow, CuaEffect,
};

use super::approval_gate::{ApprovalDecision, ComputerApprovalGate};
use super::audit_log::ComputerAuditLog;
use super::backend::{
    ComputerBackend, ComputerBackendError, ComputerBackendEvent, ComputerBackendMethod,
    ComputerBackendSubscription, ComputerError, ComputerResolvedTarget, ComputerStreamFrame,
    CuaActionError, clamp_computer_message,
};
use super::call_context::{
    ZERON_CUA_ACTION_SETTLE_MS, ZERON_CUA_CONDITIONAL_SETTLE, ZERON_CUA_TIMING_LOG,
    cua_action_settle_ms_override, cua_conditional_settle_enabled, cua_timing_log_enabled,
};
use super::control_state::ComputerControlState;
use super::cursor_activity::CursorActivity;
use super::desktop_operation_queue::{DesktopOperationQueue, assert_desktop_operation_active};
use super::scroll_calibration::{ScrollGearingStore, decode_png_luma, estimate_vertical_travel};
use super::scroll_gearing_file::ScrollGearingFile;
use super::shield::{
    ZERON_CUA_MASKED_ACTIVATION, ZERON_CUA_MASKED_APPS, cua_masked_activation_enabled,
    cua_masked_activation_opt_in,
};
use super::space_broker::{ComputerSpaceBroker, ComputerSpaceSnapshot};
use super::ui_tree_targeting::{ComputerTargetError, ComputerTargetErrorCode};

pub const COMPUTER_LEASE_IDLE_MS: u64 = 300_000;
pub const COMPUTER_CONTROL_TEARDOWN_TIMEOUT_MS: u64 = 30_000;
pub const COMPUTER_ACTION_SETTLE_MS: u64 = 300;
pub const COMPUTER_ACTION_OBSERVER_SETTLE_TIMEOUT_MS: u64 = 5_000;
pub const COMPUTER_ACTION_OBSERVER_SETTLE_QUIET_MS: u64 = COMPUTER_ACTION_SETTLE_MS;
pub const COMPUTER_PASTE_RESTORE_MS: u64 = 250;
pub const COMPUTER_WINDOWS_PUBLISH_DEBOUNCE_MS: u64 = 250;
pub const SCROLL_PROBE_PX: f64 = 48.0;
pub const SCROLL_PROBE_TRIGGER_PX: f64 = SCROLL_PROBE_PX;
pub const SCROLL_SETTLE_ARRIVAL_TOLERANCE: f64 = 0.15;
pub const SCROLL_SETTLE_ARRIVAL_MIN_PX: f64 = 4.0;
pub const COMPUTER_ERROR_REPUBLISH_DEBOUNCE_MS: u64 = 250;
pub const COMPUTER_CONTROL_ON_DISCLOSURE: &str = "Computer control ON for this turn: the agent is driving the desktop and the user can switch it off in Settings.";
const COMPUTER_DENYLIST_APP_CACHE_MS: u64 = 30_000;
const SURFACED_CONTROL_DISCLOSURES_MAX: usize = 512;
const THREAD_RECORD_LIMIT: usize = 256;
const OBSERVED_APP_NAME_LIMIT: usize = 256;

pub type ManagerClock = Arc<dyn Fn() -> u64 + Send + Sync>;
pub type MeasureScrollTravel =
    Arc<dyn Fn(Vec<u8>, Vec<u8>, Option<f64>) -> BoxFuture<'static, Option<f64>> + Send + Sync>;
pub type ComputerManagerListener = Arc<dyn Fn(&ComputerManagerEvent) + Send + Sync>;
pub type ComputerFrameSubscriber = Arc<dyn Fn(&ComputerStreamFrame) + Send + Sync>;

#[derive(Debug, Clone)]
pub struct ComputerManagerEnv {
    pub timing_log: bool,
    pub conditional_settle: bool,
    pub action_settle_ms: Option<f64>,
    pub masked_activation: bool,
    pub masked_apps: HashSet<String>,
}

impl Default for ComputerManagerEnv {
    fn default() -> Self {
        Self {
            timing_log: false,
            conditional_settle: cua_conditional_settle_enabled(None),
            action_settle_ms: None,
            masked_activation: false,
            masked_apps: HashSet::new(),
        }
    }
}

impl ComputerManagerEnv {
    pub fn from_env() -> Self {
        let read = |name: &str| std::env::var(name).ok();
        Self {
            timing_log: cua_timing_log_enabled(read(ZERON_CUA_TIMING_LOG).as_deref()),
            conditional_settle: cua_conditional_settle_enabled(
                read(ZERON_CUA_CONDITIONAL_SETTLE).as_deref(),
            ),
            action_settle_ms: cua_action_settle_ms_override(
                read(ZERON_CUA_ACTION_SETTLE_MS).as_deref(),
            ),
            masked_activation: cua_masked_activation_enabled(
                read(ZERON_CUA_MASKED_ACTIVATION).as_deref(),
            ),
            masked_apps: cua_masked_activation_opt_in(read(ZERON_CUA_MASKED_APPS).as_deref()),
        }
    }
}

pub use consent_app::{
    COMPUTER_APP_CONSENT_REQUIRED_CODE, COMPUTER_APP_WINDOW_REQUIRED_CODE,
    COMPUTER_APP_WINDOW_REQUIRED_MESSAGE,
};
pub use zeron_proto::computer::{ComputerConsentApp, ComputerConsentScope};

#[derive(Debug, Clone, PartialEq)]
pub struct ComputerConsentCard {
    pub request_id: String,
    pub chat_id: String,
    pub turn_id: Option<String>,
    pub tool_name: String,
    pub params_display: String,
    pub scope: ComputerConsentScope,
    pub app: Option<ComputerConsentApp>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ComputerConsentTarget {
    Window(String),
    Pid(u32),
    App(String),
    Unscoped,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ComputerConsent {
    Granted,
    Denied(Option<ComputerConsentApp>),
    Unscoped,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ComputerLeaseLifecycle {
    Acquired,
    ReleaseRequested,
    Released,
    StaleReclaimed,
}

impl ComputerLeaseLifecycle {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Acquired => "acquired",
            Self::ReleaseRequested => "release-requested",
            Self::Released => "released",
            Self::StaleReclaimed => "stale-reclaimed",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ComputerLeaseRecord {
    pub ts: String,
    pub event: ComputerLeaseLifecycle,
    pub thread_id: String,
    pub turn_id: Option<String>,
    pub idle_ms: Option<u64>,
    pub next_thread_id: Option<String>,
}

#[async_trait]
pub trait ComputerManagerSeams: Send + Sync {
    async fn caller(
        &self,
        _chat_id: &str,
        _turn_id: Option<&str>,
    ) -> Result<Option<PermissionMode>, ComputerError> {
        Ok(None)
    }

    async fn publish_consent(
        &self,
        _card: ComputerConsentCard,
        _decision: Option<ApprovalDecision>,
    ) -> Result<(), String> {
        Ok(())
    }

    async fn disclose_control(&self, _chat_id: &str, _turn_id: Option<&str>) {}

    fn generation_bumped(&self, _chat_id: &str, _generation: u64) {}

    fn lease_lifecycle(&self, record: &ComputerLeaseRecord) {
        tracing::info!(
            ts = %record.ts,
            event = record.event.as_str(),
            thread_id = %record.thread_id,
            turn_id = ?record.turn_id,
            idle_ms = ?record.idle_ms,
            next_thread_id = ?record.next_thread_id,
            "[computer] desktop lease"
        );
    }

    fn warn(&self, message: &str, detail: Value) {
        tracing::warn!(%detail, "{message}");
    }
}

pub struct DefaultComputerManagerSeams;

impl ComputerManagerSeams for DefaultComputerManagerSeams {}

pub struct ComputerManagerOptions {
    pub backend: Arc<dyn ComputerBackend>,
    pub data_dir: Option<PathBuf>,
    pub control_state: Arc<ComputerControlState>,
    pub approval_gate: ComputerApprovalGate,
    pub seams: Arc<dyn ComputerManagerSeams>,
    pub now: Option<ManagerClock>,
    pub lease_idle_ms: u64,
    pub action_settle_ms: Option<u64>,
    pub windows_publish_debounce_ms: u64,
    pub measure_scroll_travel: Option<MeasureScrollTravel>,
    pub env: ComputerManagerEnv,
}

impl ComputerManagerOptions {
    pub fn new(backend: Arc<dyn ComputerBackend>) -> Self {
        Self {
            backend,
            data_dir: None,
            control_state: Arc::default(),
            approval_gate: ComputerApprovalGate::default(),
            seams: Arc::new(DefaultComputerManagerSeams),
            now: None,
            lease_idle_ms: COMPUTER_LEASE_IDLE_MS,
            action_settle_ms: None,
            windows_publish_debounce_ms: COMPUTER_WINDOWS_PUBLISH_DEBOUNCE_MS,
            measure_scroll_travel: None,
            env: ComputerManagerEnv::from_env(),
        }
    }
}

pub use zeron_proto::computer::ComputerRestoreStatus as ForegroundRestoreStatus;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ForegroundRestoreInfo {
    pub restored_window_id: Option<String>,
    pub restore_status: ForegroundRestoreStatus,
}

pub type ComputerManagerEvent = ComputerEvent;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ComputerClickButton {
    #[default]
    Left,
    Right,
    Middle,
}

impl ComputerClickButton {
    fn as_str(self) -> &'static str {
        match self {
            Self::Left => "left",
            Self::Right => "right",
            Self::Middle => "middle",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ComputerClickGesture {
    pub count: Option<u8>,
    pub button: Option<ComputerClickButton>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ComputerCapturedWindow {
    pub screenshot: ComputerScreenshot,
    pub window_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ComputerActionObservation {
    Captured(ComputerCapturedWindow),
    TargetWindowClosed,
}

impl ComputerActionObservation {
    pub fn window_id(&self) -> Option<&str> {
        match self {
            Self::Captured(captured) => captured.window_id.as_deref(),
            Self::TargetWindowClosed => None,
        }
    }

    pub fn screenshot(&self) -> Option<&ComputerScreenshot> {
        match self {
            Self::Captured(captured) => Some(&captured.screenshot),
            Self::TargetWindowClosed => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ComputerSettleMode {
    Observer,
    Fixed,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ComputerWaitSettleResult {
    pub settled: bool,
    pub waited_ms: f64,
    pub events_seen: Option<f64>,
    pub mode: ComputerSettleMode,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ComputerStateRequest {
    pub include_screenshot: Option<bool>,
    pub include_text: bool,
    pub include_tree: Option<bool>,
    pub window_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ComputerNotedResult {
    pub result: zeron_proto::computer::ComputerActionResult,
    pub note: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ComputerPasteResult {
    pub result: zeron_proto::computer::ComputerActionResult,
    pub clipboard_restored: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ComputerScrollOptions {
    pub observe: bool,
    pub modifiers: Vec<ComputerInputModifier>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ComputerScrollOutcome {
    pub result: zeron_proto::computer::ComputerActionResult,
    pub observation: Option<ComputerActionObservation>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ComputerCaller {
    pub chat_id: String,
    pub turn_id: Option<String>,
}

struct ThreadRuntime {
    record: u64,
    version: u64,
    last_error: Option<String>,
    reported_error: Option<String>,
    input_pause: Option<ComputerInputPause>,
    windows: Vec<ComputerWindow>,
    screen_size: ComputerScreenSize,
    availability: ComputerAvailability,
    cursor: Option<ComputerPoint>,
    pane_surfaced: bool,
}

#[derive(Debug, Clone)]
struct DesktopLease {
    thread_id: String,
    turn_id: Option<String>,
    last_activity_ms: u64,
    release_requested: bool,
    release_requested_turn_id: Option<String>,
}

#[derive(Debug, Clone)]
struct BackgroundTarget {
    key: String,
    pid: Option<u32>,
    window_id: Option<String>,
}

#[derive(Debug, Clone)]
struct BackgroundLease {
    lease: DesktopLease,
    target: BackgroundTarget,
}

#[derive(Clone)]
struct Authority {
    token: CancellationToken,
    reason: Arc<Mutex<Option<ComputerBackendError>>>,
}

impl Authority {
    fn new() -> Self {
        Self {
            token: CancellationToken::new(),
            reason: Arc::default(),
        }
    }

    fn abort(&self, reason: &ComputerBackendError) {
        lock(&self.reason).get_or_insert_with(|| reason.clone());
        self.token.cancel();
    }

    fn aborted(&self) -> bool {
        self.token.is_cancelled()
    }

    fn reason(&self) -> Option<ComputerBackendError> {
        lock(&self.reason).clone()
    }

    fn same(&self, other: &Authority) -> bool {
        Arc::ptr_eq(&self.reason, &other.reason)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ObserverSettle {
    Unknown,
    Supported,
    Unsupported,
}

struct PhysicalState {
    availability: ComputerAvailability,
    windows: Option<Vec<ComputerWindow>>,
    screen_size: Option<ComputerScreenSize>,
}

#[derive(Default)]
struct StreamState {
    attached: bool,
    desired: bool,
    epoch: u64,
    held: bool,
    owners: Vec<String>,
    frame_gen: u64,
    listener_gen: u64,
}

type PendingStop = Shared<BoxFuture<'static, Result<(), ComputerError>>>;

struct State {
    threads: Vec<(String, ThreadRuntime)>,
    next_record: u64,
    next_state_version: u64,
    agent_calls_in_flight: HashMap<String, usize>,
    background_leases: Vec<(String, BackgroundLease)>,
    known_app_names: Vec<String>,
    thread_labels: Vec<(String, String)>,
    lease: Option<DesktopLease>,
    backend_engaged: bool,
    last_user_desktop_input_at: Option<u64>,
    last_known_window_ids: Option<HashSet<String>>,
    last_known_windows: HashMap<String, ComputerWindow>,
    pre_action_window_ids: Option<HashSet<String>>,
    observer_settle: ObserverSettle,
    backend_health: ComputerHealth,
    activity: Option<String>,
    disposed: bool,
    suspended_threads: HashSet<String>,
    revoked_threads: HashSet<String>,
    authority_revocations: HashMap<String, Authority>,
    active_authorities: HashMap<String, Vec<Authority>>,
    authority_turns: HashMap<String, String>,
    pending_stops: HashMap<String, PendingStop>,
    physical_state: Option<PhysicalState>,
    physical_read: Option<Shared<BoxFuture<'static, ()>>>,
    physical_failure: Option<String>,
    denied_apps_cache: Option<(u64, Vec<ComputerApp>)>,
    consent_pid_misses: HashMap<u32, u64>,
    publish_all_depth: usize,
    windows_publish_pending: bool,
    windows_publish_timer: Option<JoinHandle<()>>,
    error_republish_timer: Option<JoinHandle<()>>,
    publish_locks: HashMap<String, (Arc<tokio::sync::Mutex<()>>, usize)>,
    surfaced_disclosures: VecDeque<String>,
    stream: StreamState,
    stream_tail: Option<Shared<BoxFuture<'static, ()>>>,
    frame_subscribers: Vec<(u64, ComputerFrameSubscriber)>,
    latest_frame: Option<ComputerStreamFrame>,
    scroll_gearing: ScrollGearingStore,
}

pub struct ComputerManager {
    weak: Weak<ComputerManager>,
    computer_id: String,
    backend: Arc<dyn ComputerBackend>,
    seams: Arc<dyn ComputerManagerSeams>,
    control_state: Arc<ComputerControlState>,
    approval_gate: ComputerApprovalGate,
    audit_log: ComputerAuditLog,
    scroll_gearing_file: Mutex<ScrollGearingFile>,
    operations: DesktopOperationQueue,
    cursor_activity: CursorActivity,
    space_broker: ComputerSpaceBroker<ComputerError>,
    now: ManagerClock,
    lease_idle_ms: u64,
    action_settle_ms: u64,
    windows_publish_debounce_ms: u64,
    measure_scroll_travel: MeasureScrollTravel,
    env: ComputerManagerEnv,
    state: Mutex<State>,
    listeners: Mutex<Vec<(u64, ComputerManagerListener)>>,
    next_listener: AtomicU64,
    backend_subscription: Mutex<Option<ComputerBackendSubscription>>,
    stream_transition: tokio::sync::Mutex<()>,
}

#[must_use = "dropping the subscription unsubscribes the listener"]
pub struct ComputerManagerSubscription {
    manager: Weak<ComputerManager>,
    id: u64,
    frames: bool,
}

impl Drop for ComputerManagerSubscription {
    fn drop(&mut self) {
        let Some(manager) = self.manager.upgrade() else {
            return;
        };
        if self.frames {
            manager.unsubscribe_frames(self.id);
        } else {
            lock(&manager.listeners).retain(|(id, _)| *id != self.id);
        }
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

fn system_now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_millis() as u64)
}

async fn measure_scroll_travel_from_png(
    before: Vec<u8>,
    after: Vec<u8>,
    column_center: Option<f64>,
) -> Option<f64> {
    tokio::task::spawn_blocking(move || {
        let before = decode_png_luma(&before)?;
        let after = decode_png_luma(&after)?;
        estimate_vertical_travel(&before, &after, None, column_center).map(|travel| travel as f64)
    })
    .await
    .ok()
    .flatten()
}

impl ComputerManager {
    pub fn new(options: ComputerManagerOptions) -> Arc<Self> {
        let backend = options.backend;
        let computer_id = backend.computer_id().to_owned();
        let backend_health = backend.health();
        let action_settle_ms = options
            .action_settle_ms
            .or_else(|| options.env.action_settle_ms.map(|ms| ms.round() as u64))
            .unwrap_or(COMPUTER_ACTION_SETTLE_MS);
        let manager = Arc::new_cyclic(|weak: &Weak<ComputerManager>| {
            let broker_weak = weak.clone();
            let space_broker = ComputerSpaceBroker::new(
                move || {
                    let manager = broker_weak.upgrade();
                    async move {
                        let Some(manager) = manager else {
                            return Err(ComputerTargetError::space(
                                zeron_proto::computer::ComputerSpaceErrorCode::ComputerSpaceReservationChanged,
                                "The Space broker has closed.",
                            )
                            .into());
                        };
                        manager.read_space_snapshot().await
                    }
                    .boxed()
                },
                || assert_desktop_operation_active().map_err(ComputerError::from),
            );
            let activity_weak = weak.clone();
            let cursor_activity = CursorActivity::new(move |text: Option<String>| {
                let manager = activity_weak.upgrade();
                async move {
                    let Some(manager) = manager else {
                        return;
                    };
                    let threads: Vec<String> = {
                        let mut state = manager.state();
                        state.activity = text.clone();
                        state.threads.iter().map(|(id, _)| id.clone()).collect()
                    };
                    for thread_id in threads {
                        manager.publish_cached(&thread_id);
                    }
                    let _ = manager.backend.set_cursor_activity(text.as_deref()).await;
                }
                .boxed()
            });
            let data_dir = options.data_dir.as_deref();
            ComputerManager {
                weak: weak.clone(),
                computer_id,
                backend: backend.clone(),
                seams: options.seams,
                control_state: options.control_state,
                approval_gate: options.approval_gate,
                audit_log: ComputerAuditLog::new(data_dir),
                scroll_gearing_file: Mutex::new(ScrollGearingFile::new(data_dir)),
                operations: DesktopOperationQueue::new(),
                cursor_activity,
                space_broker,
                now: options.now.unwrap_or_else(|| Arc::new(system_now_ms)),
                lease_idle_ms: options.lease_idle_ms,
                action_settle_ms,
                windows_publish_debounce_ms: options.windows_publish_debounce_ms,
                measure_scroll_travel: options.measure_scroll_travel.unwrap_or_else(|| {
                    Arc::new(|before, after, column_center| {
                        measure_scroll_travel_from_png(before, after, column_center).boxed()
                    })
                }),
                env: options.env,
                state: Mutex::new(State {
                    threads: Vec::new(),
                    next_record: 0,
                    next_state_version: 0,
                    agent_calls_in_flight: HashMap::new(),
                    background_leases: Vec::new(),
                    known_app_names: Vec::new(),
                    thread_labels: Vec::new(),
                    lease: None,
                    backend_engaged: false,
                    last_user_desktop_input_at: None,
                    last_known_window_ids: None,
                    last_known_windows: HashMap::new(),
                    pre_action_window_ids: None,
                    observer_settle: ObserverSettle::Unknown,
                    backend_health,
                    activity: None,
                    disposed: false,
                    suspended_threads: HashSet::new(),
                    revoked_threads: HashSet::new(),
                    authority_revocations: HashMap::new(),
                    active_authorities: HashMap::new(),
                    authority_turns: HashMap::new(),
                    pending_stops: HashMap::new(),
                    physical_state: None,
                    physical_read: None,
                    physical_failure: None,
                    denied_apps_cache: None,
                    consent_pid_misses: HashMap::new(),
                    publish_all_depth: 0,
                    windows_publish_pending: false,
                    windows_publish_timer: None,
                    error_republish_timer: None,
                    publish_locks: HashMap::new(),
                    surfaced_disclosures: VecDeque::new(),
                    stream: StreamState::default(),
                    stream_tail: None,
                    frame_subscribers: Vec::new(),
                    latest_frame: None,
                    scroll_gearing: ScrollGearingStore::default(),
                }),
                listeners: Mutex::new(Vec::new()),
                next_listener: AtomicU64::new(1),
                backend_subscription: Mutex::new(None),
                stream_transition: tokio::sync::Mutex::new(()),
            }
        });
        let events = Arc::downgrade(&manager);
        let subscription = backend.on_event(Arc::new(move |event: &ComputerBackendEvent| {
            if let Some(manager) = events.upgrade() {
                manager.handle_backend_event(event);
            }
        }));
        *lock(&manager.backend_subscription) = Some(subscription);
        manager
    }

    fn state(&self) -> MutexGuard<'_, State> {
        lock(&self.state)
    }

    fn strong(&self) -> Option<Arc<ComputerManager>> {
        self.weak.upgrade()
    }

    fn now_ms(&self) -> u64 {
        (self.now)()
    }

    pub fn computer_id(&self) -> &str {
        &self.computer_id
    }

    pub fn agent_dialect(&self) -> super::backend::ComputerAgentDialect {
        self.backend.agent_dialect()
    }

    pub fn supports_focus_neutral_semantic_text(&self) -> bool {
        self.backend.focus_neutral_semantic_text()
    }

    pub fn supports_browser(&self) -> bool {
        self.backend.browser().is_some()
    }

    pub fn approval_gate(&self) -> &ComputerApprovalGate {
        &self.approval_gate
    }

    pub fn control_state(&self) -> &ComputerControlState {
        &self.control_state
    }

    pub fn holds_desktop(&self, thread_id: &str) -> bool {
        let state = self.state();
        state
            .lease
            .as_ref()
            .is_some_and(|lease| lease.thread_id == thread_id)
            || state.owns_background_lease(thread_id)
            || state
                .active_authorities
                .get(thread_id)
                .is_some_and(|live| !live.is_empty())
    }

    pub fn space_broker(&self) -> &ComputerSpaceBroker<ComputerError> {
        &self.space_broker
    }

    pub fn cursor_activity(&self) -> &CursorActivity {
        &self.cursor_activity
    }

    pub fn observed_app_names(&self) -> Vec<String> {
        self.state().known_app_names.clone()
    }

    fn remember_observed_app_names(state: &mut State, windows: &[ComputerWindow]) {
        for window in windows {
            let Some(name) = window.app_name.as_deref().map(str::trim) else {
                continue;
            };
            if name.is_empty() {
                continue;
            }
            state.known_app_names.retain(|known| known != name);
            state.known_app_names.push(name.to_owned());
        }
        while state.known_app_names.len() > OBSERVED_APP_NAME_LIMIT {
            state.known_app_names.remove(0);
        }
    }

    fn remember_windows(state: &mut State, windows: &[ComputerWindow]) {
        state.last_known_window_ids =
            Some(windows.iter().map(|window| window.id.clone()).collect());
        state.last_known_windows = windows
            .iter()
            .map(|window| (window.id.clone(), window.clone()))
            .collect();
        Self::remember_observed_app_names(state, windows);
    }

    async fn read_windows(&self) -> Result<Vec<ComputerWindow>, ComputerError> {
        let windows = self.backend.list_windows().await?;
        Self::remember_windows(&mut self.state(), &windows);
        Ok(windows)
    }

    async fn read_space_snapshot(&self) -> Result<ComputerSpaceSnapshot, ComputerError> {
        if !self.backend.supports(ComputerBackendMethod::ListSpaces) {
            return Err(ComputerTargetError::space(
                zeron_proto::computer::ComputerSpaceErrorCode::ComputerSpacesUnavailable,
                "This backend does not expose managed Space inventory. Drive an exact existing window in place instead.",
            )
            .into());
        }
        self.engage_backend();
        let inventory = self.backend.list_spaces().await?;
        let windows = self.read_windows().await?;
        Ok(ComputerSpaceSnapshot { inventory, windows })
    }

    pub fn on_event(&self, listener: ComputerManagerListener) -> ComputerManagerSubscription {
        let id = self.next_listener.fetch_add(1, Ordering::Relaxed);
        lock(&self.listeners).push((id, listener));
        ComputerManagerSubscription {
            manager: self.weak.clone(),
            id,
            frames: false,
        }
    }

    fn emit(&self, event: ComputerEvent) {
        let listeners: Vec<ComputerManagerListener> = lock(&self.listeners)
            .iter()
            .map(|(_, listener)| listener.clone())
            .collect();
        for listener in listeners {
            let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| listener(&event)));
        }
    }

    fn handle_backend_event(&self, event: &ComputerBackendEvent) {
        match event {
            ComputerBackendEvent::WindowsChanged(windows) => {
                {
                    let mut state = self.state();
                    Self::remember_windows(&mut state, windows);
                    for (_, thread) in &mut state.threads {
                        thread.windows = windows.clone();
                    }
                }
                self.emit(ComputerEvent::WindowsChanged {
                    windows: windows.clone(),
                });
                self.schedule_windows_publish();
            }
            ComputerBackendEvent::HealthChanged(health) => {
                self.state().backend_health = health.clone();
                self.republish_all_threads();
            }
            ComputerBackendEvent::CapabilitiesChanged(_) => self.republish_all_threads(),
            ComputerBackendEvent::DesktopInterrupted { .. } => {
                self.approval_gate.revoke_task_grants();
            }
            ComputerBackendEvent::Frame(_) => {}
        }
    }

    fn engage_backend(&self) {
        {
            let mut state = self.state();
            if state.backend_engaged || state.disposed {
                return;
            }
            state.backend_engaged = true;
        }
        if let Some(manager) = self.strong() {
            tokio::spawn(async move {
                manager.publish_all_threads().await;
            });
        }
    }

    pub async fn missing_permissions(&self) -> Vec<zeron_proto::computer::ComputerPermission> {
        self.backend.missing_permissions().await
    }

    pub fn build_signature(&self) -> Option<zeron_proto::computer::ComputerBuildSignature> {
        self.backend.build_signature()
    }

    pub fn native_element_identity(
        &self,
        node: &zeron_proto::computer::ComputerUiNode,
    ) -> Option<String> {
        self.backend.native_element_identity(node)
    }

    pub async fn interrupt_thread_input(
        &self,
        thread_id: &str,
        turn_id: Option<&str>,
    ) -> Result<(), ComputerError> {
        self.backend
            .stop_input(Some(&zeron_proto::computer::CuaComputerTask {
                thread_id: thread_id.to_owned(),
                turn_id: turn_id.map(str::to_owned),
                label: None,
            }))
            .await
    }

    pub fn record_computer_audit(&self, entry: &super::audit_log::ComputerAuditEntry) {
        if let Some(thread_id) = entry.thread_id.as_deref()
            && self.state().suspended_threads.contains(thread_id)
        {
            return;
        }
        self.audit_log.record(entry);
    }

    pub async fn get_audit_history(
        &self,
        input: zeron_proto::computer::ComputerGetAuditHistoryInput,
    ) -> Result<zeron_proto::computer::ComputerGetAuditHistoryResult, ComputerError> {
        Ok(self.audit_log.read_history(input).await?)
    }

    fn corrected_availability(&self, availability: ComputerAvailability) -> ComputerAvailability {
        let (engaged, health) = {
            let state = self.state();
            (state.backend_engaged, state.backend_health.clone())
        };
        corrected_availability(engaged, &health, availability)
    }

    fn warn(&self, message: &str, detail: Value) {
        self.seams.warn(message, detail);
    }
}

fn corrected_availability(
    engaged: bool,
    health: &ComputerHealth,
    availability: ComputerAvailability,
) -> ComputerAvailability {
    if !engaged
        || health.status == ComputerHealthStatus::Connected
        || !matches!(availability, ComputerAvailability::Available { .. })
    {
        return availability;
    }
    ComputerAvailability::BackendUnavailable {
        message: health_unavailable_message(health),
    }
}

fn health_unavailable_message(health: &ComputerHealth) -> String {
    let reason = if health.status == ComputerHealthStatus::Reconnecting {
        "Reconnecting to the desktop."
    } else {
        "The desktop backend is not connected."
    };
    let message = match &health.last_failure {
        Some(failure) => format!("{reason} Last failure: {}", failure.message),
        None => reason.to_owned(),
    };
    clamp_computer_message(&message, reason)
}

pub(crate) fn agent_thread_id(thread_id: Option<&str>) -> Option<&str> {
    thread_id
        .map(str::trim)
        .filter(|trimmed| !trimmed.is_empty())
}

fn error_message(error: &ComputerError) -> String {
    error.to_string()
}

fn json_string(text: &str) -> String {
    Value::String(text.to_owned()).to_string()
}

fn window_id_set(windows: &[ComputerWindow]) -> HashSet<String> {
    windows.iter().map(|window| window.id.clone()).collect()
}

fn rects_overlap(first: &ComputerRect, second: &ComputerRect) -> bool {
    first.x < second.x + second.width
        && second.x < first.x + first.width
        && first.y < second.y + second.height
        && second.y < first.y + first.height
}

fn round2(value: f64) -> f64 {
    ((value * 100.0) + 0.5).floor() / 100.0
}

fn settle_permanently_unsupported(error: &ComputerError) -> bool {
    let message = error.to_string();
    message.starts_with("Unknown tool:") || message == "Unsupported computer host request."
}

fn window_not_found_error(window_id: &str) -> ComputerError {
    ComputerTargetError::new(
        ComputerTargetErrorCode::NotFound,
        format!(
            "No desktop window has id {}. Call computer_list_windows for the current window ids.",
            json_string(window_id)
        ),
        Vec::new(),
    )
    .into()
}

fn menu_app_not_found_error(app: &str) -> ComputerError {
    ComputerTargetError::new(
        ComputerTargetErrorCode::NotFound,
        format!(
            "No running application matches {}. Call computer_list_apps for the running apps and their pids, then name one by app or pid.",
            json_string(app)
        ),
        Vec::new(),
    )
    .into()
}

fn occluded_target_error(
    window_id: &str,
    point: ComputerPoint,
    covering: &[ComputerWindow],
    reason: &str,
) -> ComputerError {
    let blockers: Vec<String> = covering
        .iter()
        .take(4)
        .map(|window| {
            let name = if window.title.is_empty() {
                window.id.as_str()
            } else {
                window.title.as_str()
            };
            format!("{} ({})", json_string(name), window.id)
        })
        .collect();
    ComputerTargetError::new(
        ComputerTargetErrorCode::Occluded,
        format!(
            "Window {} is covered at ({}, {}) by {}, and this desktop could not raise it: {reason}. The input would go to the covering window. Aim at a part of the target window that nothing covers, or move the covering window out of the way first; or drop window_id to act on whatever is topmost at that point.",
            json_string(window_id),
            js_number(point.x),
            js_number(point.y),
            blockers.join(", ")
        ),
        Vec::new(),
    )
    .into()
}

fn refused_injection_error(action: &str, window_id: &str, point: ComputerPoint) -> ComputerError {
    ComputerTargetError::new(
        ComputerTargetErrorCode::Refused,
        format!(
            "The desktop refused to deliver {action} to window {} at ({}, {}), so no input was sent. The window is not accepting input at that point: a window's bounds include invisible resize and shadow margins, and the window may also have closed since it was listed. Aim nearer the middle of the control, target it by label instead of a coordinate, or drop window_id to act on whatever is topmost there.",
            json_string(window_id),
            js_number(point.x),
            js_number(point.y)
        ),
        Vec::new(),
    )
    .into()
}

fn triple_click_unsupported_error() -> ComputerError {
    ComputerBackendError::new(
        "This desktop backend cannot send a triple click. Select the line another way — click at its start and shift-click at its end, or use the application's own select-all shortcut with computer_press_key.",
    )
    .into()
}

fn click_gesture_unsupported_error(button: ComputerClickButton, count: u8) -> ComputerError {
    ComputerBackendError::new(format!(
        "This desktop backend cannot send a {} click with count {count}. Supported gestures are a left click with count 1-3 and a single right click.",
        button.as_str()
    ))
    .into()
}

fn activation_unsupported_error() -> ComputerError {
    ComputerBackendError::new(
        "This desktop backend cannot bring a window forward. Ask the user to click the window they want in front, or aim the action at it with window_id instead.",
    )
    .into()
}

fn clipboard_not_restorable_error() -> ComputerError {
    CuaActionError::new(
        "computer_paste would overwrite the user's clipboard, which holds content that cannot be restored (an image, files, rich data or very long text). Nothing was sent. Use computer_type_text or computer_set_value instead.",
        CuaEffect::NotDispatched,
    )
    .code(crate::computer::cua_backend::CLIPBOARD_NOT_RESTORABLE_CODE)
    .into()
}

fn clipboard_unsupported_error() -> ComputerError {
    ComputerBackendError::new("This computer backend does not support clipboard access.").into()
}

fn js_number(value: f64) -> String {
    if value.fract() == 0.0 && value.abs() < 1e15 {
        format!("{}", value as i64)
    } else {
        format!("{value}")
    }
}

async fn with_teardown_timeout<T>(
    action: impl std::future::Future<Output = Result<T, ComputerError>>,
) -> Result<T, ComputerError> {
    match tokio::time::timeout(
        Duration::from_millis(COMPUTER_CONTROL_TEARDOWN_TIMEOUT_MS),
        action,
    )
    .await
    {
        Ok(result) => result,
        Err(_) => Err(ComputerBackendError::new(
            "Computer control teardown timed out waiting on a wedged operation; the in-memory gate stays held.",
        )
        .into()),
    }
}

fn semantic_point_target(resolved: &ComputerResolvedTarget) -> targeting::PreparedTarget {
    targeting::PreparedTarget {
        point: Some(resolved.point),
        window_id: resolved.node.window_id.clone(),
        covering: None,
    }
}
