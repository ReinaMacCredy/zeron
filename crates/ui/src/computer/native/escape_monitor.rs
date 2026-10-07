use std::cell::Cell;
use std::rc::Rc;
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use core_foundation::base::TCFType;
use core_foundation::mach_port::CFMachPortRef;
use core_foundation::runloop::{
    CFRunLoop, CFRunLoopSource, kCFRunLoopCommonModes, kCFRunLoopDefaultMode,
};
use core_graphics::event::{
    CGEvent, CGEventTap, CGEventTapLocation, CGEventTapOptions, CGEventTapPlacement, CGEventType,
    EventField,
};
use objc2_app_kit::NSWorkspace;
use tokio::sync::watch;

use super::escape_classifier::{Classified, EventSource, OBSERVED_EVENT_TYPES, classify};
use super::permissions::input_monitoring_granted;
use super::window_info::{on_screen_windows, window_at};
use crate::computer::host::{ComputerInputMonitorState, PhysicalComputerInput, PhysicalInputKind};

pub const INPUT_MONITOR_STARTING: &str = "input_monitor_starting";
pub const INPUT_MONITOR_IDLE: &str = "input_monitor_idle";
pub const INPUT_MONITOR_STOPPED: &str = "input_monitor_stopped";
pub const INPUT_MONITORING_REQUIRED: &str = "input-monitoring-required";
pub const EVENT_TAP_DISABLED: &str = "event_tap_disabled";
pub const EVENT_TAP_UNAVAILABLE: &str = "event_tap_unavailable";

const RETRY_INTERVAL: Duration = Duration::from_secs(5);
const RUN_SLICE: Duration = Duration::from_millis(500);
const ACTIVATION_WAIT: Duration = Duration::from_secs(1);

const INPUT_MONITORING_MESSAGE: &str =
    "Input Monitoring permission is required to notice when you use an app the agent is driving.";
const TAP_DISABLED_MESSAGE: &str =
    "macOS disabled the computer input listener; checking access before reconnecting.";
const TAP_CREATE_MESSAGE: &str = "macOS could not create the passive input listener.";
const TAP_ATTACH_MESSAGE: &str = "macOS could not attach the input listener to the run loop.";

pub trait EscapeMonitorSink: Send + Sync + 'static {
    fn physical_input(&self, input: PhysicalComputerInput);
    fn state_changed(&self, state: ComputerInputMonitorState);
    fn error(&self, message: &str);
}

fn starting() -> ComputerInputMonitorState {
    ComputerInputMonitorState::failed(INPUT_MONITOR_STARTING)
}

#[derive(Debug)]
pub(super) struct MonitorCore {
    armed: bool,
    disposed: bool,
    reinstall: bool,
    state: ComputerInputMonitorState,
    last_install_error: Option<&'static str>,
}

impl MonitorCore {
    pub(super) fn new() -> Self {
        Self {
            armed: false,
            disposed: false,
            reinstall: false,
            state: starting(),
            last_install_error: None,
        }
    }

    fn set_state(&mut self, next: ComputerInputMonitorState) -> Option<ComputerInputMonitorState> {
        if self.state == next {
            return None;
        }
        self.state = next.clone();
        Some(next)
    }

    pub(super) fn set_armed(&mut self, armed: bool) -> Option<ComputerInputMonitorState> {
        if self.disposed || self.armed == armed {
            return None;
        }
        self.armed = armed;
        if armed {
            self.last_install_error = None;
            self.set_state(starting())
        } else {
            self.set_state(ComputerInputMonitorState::failed(INPUT_MONITOR_IDLE))
        }
    }

    pub(super) fn ready(&mut self) -> Option<ComputerInputMonitorState> {
        if !self.armed || self.disposed {
            return None;
        }
        self.last_install_error = None;
        self.set_state(ComputerInputMonitorState::ready())
    }

    pub(super) fn install_failed(
        &mut self,
        code: &'static str,
    ) -> (Option<ComputerInputMonitorState>, bool) {
        if !self.armed || self.disposed {
            return (None, false);
        }
        let report = self.last_install_error != Some(code);
        self.last_install_error = Some(code);
        (
            self.set_state(ComputerInputMonitorState::failed(code)),
            report,
        )
    }

    pub(super) fn tap_disabled(&mut self) -> Option<ComputerInputMonitorState> {
        if !self.armed || self.disposed {
            return None;
        }
        self.set_state(ComputerInputMonitorState::failed(EVENT_TAP_DISABLED))
    }

    pub(super) fn refresh_denied(&mut self) -> Option<ComputerInputMonitorState> {
        if !self.armed
            || self.disposed
            || self.state.error.as_deref() != Some(INPUT_MONITORING_REQUIRED)
        {
            return None;
        }
        self.reinstall = true;
        self.last_install_error = None;
        self.set_state(starting())
    }

    pub(super) fn accepts_signals(&self) -> bool {
        self.armed && !self.disposed && self.state.ready
    }

    pub(super) fn dispose(&mut self) -> Option<ComputerInputMonitorState> {
        if self.disposed {
            return None;
        }
        self.disposed = true;
        self.armed = false;
        self.set_state(ComputerInputMonitorState::failed(INPUT_MONITOR_STOPPED))
    }
}

fn settled(state: &ComputerInputMonitorState) -> bool {
    state.ready || state.error.as_deref() != Some(INPUT_MONITOR_STARTING)
}

struct Shared {
    core: Mutex<Core>,
    wake: Condvar,
    state: watch::Sender<ComputerInputMonitorState>,
    sink: Arc<dyn EscapeMonitorSink>,
}

struct Core {
    monitor: MonitorCore,
    run_loop: Option<CFRunLoop>,
}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, Core> {
        self.core
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn publish(&self, changed: Option<ComputerInputMonitorState>) {
        if let Some(state) = changed {
            self.state.send_replace(state.clone());
            self.sink.state_changed(state);
        }
    }

    fn change(&self, update: impl FnOnce(&mut MonitorCore) -> Option<ComputerInputMonitorState>) {
        let (changed, armed_flipped, run_loop) = {
            let mut core = self.lock();
            let armed = core.monitor.armed;
            let changed = update(&mut core.monitor);
            (changed, core.monitor.armed != armed, core.run_loop.clone())
        };
        // The first arm leaves the state at `starting`, so a state diff alone never wakes the thread.
        if changed.is_some() || armed_flipped {
            self.wake.notify_all();
            if let Some(run_loop) = run_loop {
                run_loop.stop();
            }
        }
        self.publish(changed);
    }

    fn install_failed(&self, code: &'static str, message: &str) {
        let (changed, report) = self.lock().monitor.install_failed(code);
        self.publish(changed);
        if report {
            self.sink.error(message);
        }
    }

    fn tap_disabled(&self) {
        let changed = self.lock().monitor.tap_disabled();
        if changed.is_some() {
            self.publish(changed);
            self.sink.error(TAP_DISABLED_MESSAGE);
        }
    }

    fn ready(&self) {
        let changed = self.lock().monitor.ready();
        self.publish(changed);
    }

    fn accepts_signals(&self) -> bool {
        self.lock().monitor.accepts_signals()
    }
}

pub struct EscapeKillSwitchMonitor {
    shared: Arc<Shared>,
    thread: Mutex<Option<JoinHandle<()>>>,
}

impl EscapeKillSwitchMonitor {
    pub fn start(sink: Arc<dyn EscapeMonitorSink>) -> std::io::Result<Self> {
        let (state, _) = watch::channel(starting());
        let shared = Arc::new(Shared {
            core: Mutex::new(Core {
                monitor: MonitorCore::new(),
                run_loop: None,
            }),
            wake: Condvar::new(),
            state,
            sink,
        });
        let dispatch = tracing::dispatcher::get_default(Clone::clone);
        let thread = std::thread::Builder::new()
            .name("zeron-cua-escape".into())
            .spawn({
                let shared = shared.clone();
                move || tracing::dispatcher::with_default(&dispatch, || run(shared))
            })?;
        Ok(Self {
            shared,
            thread: Mutex::new(Some(thread)),
        })
    }

    pub fn state(&self) -> ComputerInputMonitorState {
        self.shared.state.borrow().clone()
    }

    pub fn set_armed(&self, armed: bool) {
        self.shared.change(|core| core.set_armed(armed));
    }

    pub async fn activate(&self) {
        self.set_armed(true);
        let mut state = self.shared.state.subscribe();
        if settled(&state.borrow()) {
            return;
        }
        let _ = tokio::time::timeout(ACTIVATION_WAIT, state.wait_for(settled)).await;
    }

    pub fn refresh_denied_listener(&self) {
        self.shared.change(MonitorCore::refresh_denied);
    }

    pub fn dispose(&self) {
        self.shared.change(MonitorCore::dispose);
        let thread = self
            .thread
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .take();
        if let Some(thread) = thread
            && thread.thread().id() != std::thread::current().id()
        {
            let _ = thread.join();
        }
    }
}

impl Drop for EscapeKillSwitchMonitor {
    fn drop(&mut self) {
        self.dispose();
    }
}

fn run(shared: Arc<Shared>) {
    shared.lock().run_loop = Some(CFRunLoop::get_current());
    let mut tap: Option<InstalledTap> = None;
    let mut next_install = Instant::now();
    loop {
        let (armed, disposed, reinstall) = {
            let mut core = shared.lock();
            let reinstall = std::mem::take(&mut core.monitor.reinstall);
            (core.monitor.armed, core.monitor.disposed, reinstall)
        };
        if disposed {
            break;
        }
        if !armed {
            tap = None;
            let core = shared.lock();
            let _core = shared
                .wake
                .wait_while(core, |core| !core.monitor.armed && !core.monitor.disposed);
            next_install = Instant::now();
            continue;
        }
        if reinstall {
            tap = None;
            next_install = Instant::now();
        }
        if Instant::now() >= next_install {
            tap = install(&shared, tap.take());
            next_install = Instant::now() + RETRY_INTERVAL;
        }
        let slice = next_install
            .saturating_duration_since(Instant::now())
            .min(RUN_SLICE);
        if tap.is_some() {
            CFRunLoop::run_in_mode(unsafe { kCFRunLoopDefaultMode }, slice, false);
        } else {
            let core = shared.lock();
            let _core = shared.wake.wait_timeout_while(core, slice, |core| {
                core.monitor.armed && !core.monitor.disposed && !core.monitor.reinstall
            });
        }
    }
    drop(tap);
    shared.lock().run_loop = None;
}

fn install(shared: &Arc<Shared>, existing: Option<InstalledTap>) -> Option<InstalledTap> {
    if !input_monitoring_granted() {
        drop(existing);
        shared.install_failed(INPUT_MONITORING_REQUIRED, INPUT_MONITORING_MESSAGE);
        return None;
    }
    if let Some(tap) = existing {
        if tap.enabled() {
            return Some(tap);
        }
        drop(tap);
    }
    match InstalledTap::create(shared.clone()) {
        Ok(tap) => {
            shared.ready();
            Some(tap)
        }
        Err(message) => {
            shared.install_failed(EVENT_TAP_UNAVAILABLE, message);
            None
        }
    }
}

unsafe extern "C" {
    fn CGEventTapEnable(tap: CFMachPortRef, enable: bool);
    fn CGEventTapIsEnabled(tap: CFMachPortRef) -> bool;
    fn CFMachPortInvalidate(port: CFMachPortRef);
}

struct InstalledTap {
    tap: CGEventTap<'static>,
    source: CFRunLoopSource,
}

impl InstalledTap {
    fn create(shared: Arc<Shared>) -> Result<Self, &'static str> {
        let port: Rc<Cell<Option<CFMachPortRef>>> = Rc::new(Cell::new(None));
        let callback_port = port.clone();
        let tap = CGEventTap::new(
            CGEventTapLocation::Session,
            CGEventTapPlacement::HeadInsertEventTap,
            CGEventTapOptions::ListenOnly,
            // The tap-disabled notices arrive regardless of the mask; their raw values overflow it.
            OBSERVED_EVENT_TYPES.to_vec(),
            move |_, kind, event| {
                objc2::rc::autoreleasepool(|_| handle_event(&shared, &callback_port, kind, event));
                None
            },
        )
        .map_err(|()| TAP_CREATE_MESSAGE)?;
        port.set(Some(tap.mach_port.as_concrete_TypeRef()));
        let Ok(source) = tap.mach_port.create_runloop_source(0) else {
            unsafe { CFMachPortInvalidate(tap.mach_port.as_concrete_TypeRef()) };
            return Err(TAP_ATTACH_MESSAGE);
        };
        CFRunLoop::get_current().add_source(&source, unsafe { kCFRunLoopCommonModes });
        tap.enable();
        Ok(Self { tap, source })
    }

    fn enabled(&self) -> bool {
        unsafe { CGEventTapIsEnabled(self.tap.mach_port.as_concrete_TypeRef()) }
    }
}

impl Drop for InstalledTap {
    fn drop(&mut self) {
        CFRunLoop::get_current().remove_source(&self.source, unsafe { kCFRunLoopCommonModes });
        unsafe { CFMachPortInvalidate(self.tap.mach_port.as_concrete_TypeRef()) };
    }
}

fn handle_event(
    shared: &Arc<Shared>,
    port: &Cell<Option<CFMachPortRef>>,
    kind: CGEventType,
    event: &CGEvent,
) {
    if matches!(
        kind,
        CGEventType::TapDisabledByTimeout | CGEventType::TapDisabledByUserInput
    ) {
        shared.tap_disabled();
        if input_monitoring_granted()
            && let Some(port) = port.get()
        {
            unsafe { CGEventTapEnable(port, true) };
            shared.ready();
        } else {
            shared.lock().monitor.reinstall = true;
            CFRunLoop::get_current().stop();
        }
        return;
    }
    if !shared.accepts_signals() {
        return;
    }
    let source = EventSource {
        process_id: event.get_integer_value_field(EventField::EVENT_SOURCE_UNIX_PROCESS_ID),
        state_id: event.get_integer_value_field(EventField::EVENT_SOURCE_STATE_ID),
        user_data: event.get_integer_value_field(EventField::EVENT_SOURCE_USER_DATA),
    };
    let key_code = event.get_integer_value_field(EventField::KEYBOARD_EVENT_KEYCODE);
    match input_kind(classify(
        kind as u32,
        key_code,
        event.get_flags(),
        source,
        true,
    )) {
        Some(PhysicalInputKind::Keyboard) => {
            shared.sink.physical_input(PhysicalComputerInput {
                kind: PhysicalInputKind::Keyboard,
                pid: frontmost_pid(),
                window_id: None,
            });
        }
        Some(PhysicalInputKind::Pointer) => {
            let target = window_at(&on_screen_windows(), event.location());
            shared.sink.physical_input(PhysicalComputerInput {
                kind: PhysicalInputKind::Pointer,
                pid: target.map(|(pid, _)| pid),
                window_id: target.map(|(_, window_id)| window_id),
            });
        }
        None => {}
    }
}

fn input_kind(classified: Classified) -> Option<PhysicalInputKind> {
    match classified {
        Classified::Escape | Classified::Physical { keyboard: true } => {
            Some(PhysicalInputKind::Keyboard)
        }
        Classified::Physical { keyboard: false } => Some(PhysicalInputKind::Pointer),
        Classified::Ignored => None,
    }
}

fn frontmost_pid() -> Option<u32> {
    let app = NSWorkspace::sharedWorkspace().frontmostApplication()?;
    u32::try_from(app.processIdentifier()).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn physical_escape_is_ordinary_keyboard_input_not_a_stop() {
        assert_eq!(
            input_kind(Classified::Escape),
            Some(PhysicalInputKind::Keyboard)
        );
    }

    fn failed(code: &str) -> Option<ComputerInputMonitorState> {
        Some(ComputerInputMonitorState::failed(code))
    }

    #[test]
    fn arming_walks_starting_ready_and_idle_like_the_helper_gate() {
        let mut core = MonitorCore::new();
        assert_eq!(core.state, starting());
        assert!(!core.accepts_signals());
        assert_eq!(
            core.ready(),
            None,
            "a disarmed listener never reports ready"
        );
        assert_eq!(core.set_armed(false), None);
        assert_eq!(core.set_armed(true), None, "already starting");
        assert!(core.armed);
        assert_eq!(core.set_armed(true), None);
        assert_eq!(core.ready(), Some(ComputerInputMonitorState::ready()));
        assert!(core.accepts_signals());
        assert_eq!(core.ready(), None);
        assert_eq!(core.set_armed(false), failed(INPUT_MONITOR_IDLE));
        assert!(!core.accepts_signals());
        assert_eq!(core.set_armed(true), failed(INPUT_MONITOR_STARTING));
    }

    #[test]
    fn install_failures_close_admission_and_report_once_per_code() {
        let mut core = MonitorCore::new();
        assert_eq!(
            core.install_failed(INPUT_MONITORING_REQUIRED),
            (None, false),
            "disarmed installs are silent"
        );
        core.set_armed(true);
        assert_eq!(
            core.install_failed(INPUT_MONITORING_REQUIRED),
            (failed(INPUT_MONITORING_REQUIRED), true)
        );
        assert_eq!(
            core.install_failed(INPUT_MONITORING_REQUIRED),
            (None, false)
        );
        assert_eq!(
            core.install_failed(EVENT_TAP_UNAVAILABLE),
            (failed(EVENT_TAP_UNAVAILABLE), true)
        );
        assert!(!core.accepts_signals());
        core.ready();
        assert_eq!(
            core.install_failed(EVENT_TAP_UNAVAILABLE),
            (failed(EVENT_TAP_UNAVAILABLE), true),
            "a healthy install resets the dedupe"
        );
        core.set_armed(false);
        assert_eq!(
            core.set_armed(true),
            failed(INPUT_MONITOR_STARTING),
            "re-arming resets the dedupe"
        );
        assert_eq!(
            core.install_failed(EVENT_TAP_UNAVAILABLE),
            (failed(EVENT_TAP_UNAVAILABLE), true)
        );
    }

    #[test]
    fn a_disabled_tap_pauses_input_until_it_is_re_enabled() {
        let mut core = MonitorCore::new();
        assert_eq!(core.tap_disabled(), None);
        core.set_armed(true);
        core.ready();
        assert_eq!(core.tap_disabled(), failed(EVENT_TAP_DISABLED));
        assert!(!core.accepts_signals());
        assert_eq!(core.ready(), Some(ComputerInputMonitorState::ready()));
    }

    #[test]
    fn only_a_denied_listener_is_refreshed_after_a_fresh_grant() {
        let mut core = MonitorCore::new();
        core.set_armed(true);
        core.ready();
        assert_eq!(core.refresh_denied(), None);
        assert!(!core.reinstall);
        core.install_failed(INPUT_MONITORING_REQUIRED);
        assert_eq!(core.refresh_denied(), failed(INPUT_MONITOR_STARTING));
        assert!(core.reinstall);
        assert_eq!(core.last_install_error, None);
    }

    #[test]
    fn dispose_stops_for_good() {
        let mut core = MonitorCore::new();
        core.set_armed(true);
        core.ready();
        assert_eq!(core.dispose(), failed(INPUT_MONITOR_STOPPED));
        assert_eq!(core.dispose(), None);
        assert_eq!(core.set_armed(true), None);
        assert_eq!(core.ready(), None);
        assert!(!core.accepts_signals());
    }

    #[test]
    fn activation_waits_only_while_starting() {
        assert!(!settled(&starting()));
        assert!(settled(&ComputerInputMonitorState::ready()));
        assert!(settled(&ComputerInputMonitorState::failed(
            INPUT_MONITORING_REQUIRED
        )));
        assert!(settled(&ComputerInputMonitorState::failed(
            INPUT_MONITOR_IDLE
        )));
    }

    #[tokio::test]
    async fn the_first_arm_wakes_a_listener_thread_already_asleep() {
        let recorder = Arc::new(Recorder::default());
        let monitor = EscapeKillSwitchMonitor::start(recorder).unwrap();
        tokio::time::sleep(Duration::from_millis(300)).await;
        let started = Instant::now();
        monitor.activate().await;
        assert!(started.elapsed() < ACTIVATION_WAIT, "{:?}", monitor.state());
        assert!(settled(&monitor.state()), "{:?}", monitor.state());
        monitor.dispose();
    }

    #[derive(Default)]
    struct Recorder {
        states: Mutex<Vec<ComputerInputMonitorState>>,
    }

    impl EscapeMonitorSink for Recorder {
        fn physical_input(&self, _input: PhysicalComputerInput) {}
        fn state_changed(&self, state: ComputerInputMonitorState) {
            self.states.lock().unwrap().push(state);
        }
        fn error(&self, _message: &str) {}
    }

    #[tokio::test]
    async fn the_listener_thread_arms_settles_and_stops_without_the_main_thread() {
        let recorder = Arc::new(Recorder::default());
        let monitor = EscapeKillSwitchMonitor::start(recorder.clone()).unwrap();
        let started = Instant::now();
        monitor.activate().await;
        assert!(started.elapsed() < Duration::from_secs(2));
        let state = monitor.state();
        let expected_error = if input_monitoring_granted() {
            None
        } else {
            Some(INPUT_MONITORING_REQUIRED)
        };
        assert_eq!(state.error.as_deref(), expected_error, "{state:?}");
        monitor.set_armed(false);
        assert_eq!(
            monitor.state(),
            ComputerInputMonitorState::failed(INPUT_MONITOR_IDLE)
        );
        monitor.dispose();
        assert_eq!(
            monitor.state(),
            ComputerInputMonitorState::failed(INPUT_MONITOR_STOPPED)
        );
        let states = recorder.states.lock().unwrap().clone();
        assert!(states.contains(&ComputerInputMonitorState::failed(INPUT_MONITOR_IDLE)));
        assert_eq!(
            states.last(),
            Some(&ComputerInputMonitorState::failed(INPUT_MONITOR_STOPPED))
        );
    }
}
