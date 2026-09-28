mod client;
mod process;
mod seams;
mod support;
#[cfg(test)]
mod tests;

use std::cell::{Cell, RefCell};
use std::collections::{BTreeSet, HashMap, HashSet};
use std::future::Future;
use std::path::PathBuf;
use std::rc::{Rc, Weak};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use futures::FutureExt;
use futures::future::{LocalBoxFuture, Shared};
use serde_json::{Map, Value, json};
use tokio::sync::{Notify, mpsc, oneshot};
use zeron_proto::computer::{
    CUA_ACTION_TOOLS, CUA_BROWSER_MUTATION_TOOLS, CUA_BROWSER_TOOLS, CUA_DRIVER_VERSION,
    CUA_READ_TOOLS, CUA_SETUP_TIMEOUT_MS, ComputerDeliveryMode, ComputerHostCall,
    ComputerHostMethod, CuaActionErrorCode, CuaComputerTask, CuaEffect, CuaReply, CuaShieldArgs,
    CuaToolResult, cua_action_diagnostic_message, cua_cleanup_acknowledged, cua_computer_task_key,
    parse_cua_action_diagnostics, parse_cua_computer_task, parse_cua_shield_args,
};
use zeron_rpc::ComputerHost;

use client::{Abort, cua_request, cua_tool_request, tool_reply};
pub use process::sweep_orphaned_cua_drivers;
use process::{DriverChild, SpawnPlan, spawn_driver};
pub use seams::{
    ComputerFrameTapHost, ComputerInputMonitorState, ComputerShieldEngagement, ComputerShieldHost,
    CuaDriverHostOptions, HostFuture, HostPermissions, OverviewNormalizer, PermissionCheck,
    PhysicalComputerInput, PhysicalInputKind, TakeoverEvent,
};
use support::*;
pub use support::{
    CUA_CURSOR_IDLE_HIDE_MS, CuaCursorStyle, ESCAPE_INPUT_COOLDOWN, normalize_cua_cursor_style,
};

use crate::computer::runtime_ownership::create_cua_runtime_directory;

type Chain = Shared<LocalBoxFuture<'static, ()>>;
type Fallible = Shared<LocalBoxFuture<'static, Result<(), String>>>;
type Starting = Shared<LocalBoxFuture<'static, Result<Rc<Generation>, String>>>;

const REQUEST_IDLE_TIMEOUT: Duration = Duration::from_secs(60);
const DRIVER_ACTION_TIMEOUT: Duration = Duration::from_secs(30);
const SESSION_REVIVE_TIMEOUT: Duration = Duration::from_secs(10);
const CONTROL_SESSION_TIMEOUT: Duration = Duration::from_secs(10);
const CLEANUP_TIMEOUT: Duration = Duration::from_secs(5);
const CURSOR_STYLE_TIMEOUT: Duration = Duration::from_secs(5);
const CURSOR_ENABLE_TIMEOUT: Duration = Duration::from_millis(500);
const CURSOR_STATE_TIMEOUT: Duration = Duration::from_millis(250);
const HANDSHAKE_ATTEMPTS: usize = 80;
const HANDSHAKE_ATTEMPT_TIMEOUT: Duration = Duration::from_millis(200);
const HANDSHAKE_RETRY_DELAY: Duration = Duration::from_millis(50);
const EXIT_GRACE: Duration = Duration::from_millis(500);
const ENGINE_CALL_TIMEOUT: Duration =
    Duration::from_millis(zeron_engine::computer::cua_backend::CUA_HOST_CALL_TIMEOUT_MS);
const QUIET_POLL: Duration = Duration::from_millis(50);

const STOPPED_DURING_STARTUP: &str = "Cua Driver stopped during startup.";
const CANCELLED_BEFORE_DISPATCH: &str = "Cancelled before dispatch.";
const EXITED_WITHOUT_CLEANUP: &str = "Cua Driver exited during input without confirming native cleanup. Computer admission is closed.";
const CLEANUP_NOT_CONFIRMED: &str = "Cua Driver did not confirm native input cleanup. Computer admission is closed; the driver was not killed or replaced.";
const INTERRUPT_NOT_CONFIRMED: &str = "Cua Driver has not confirmed input interruption and cleanup. Input remains paused; a later attempt will recheck the native drain.";

fn log(message: &str) {
    tracing::info!(target: "desktop_cua", "{message}");
}

fn now_iso() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

fn eager<T: Clone + 'static>(
    future: impl Future<Output = T> + 'static,
) -> Shared<LocalBoxFuture<'static, T>> {
    let task = tokio::task::spawn_local(future);
    async move {
        match task.await {
            Ok(value) => value,
            Err(error) if error.is_panic() => std::panic::resume_unwind(error.into_panic()),
            Err(_) => std::future::pending().await,
        }
    }
    .boxed_local()
    .shared()
}

fn unit<T: Clone + 'static>(future: Shared<LocalBoxFuture<'static, T>>) -> Chain {
    future.map(|_| ()).boxed_local().shared()
}

fn resolved() -> Chain {
    async {}.boxed_local().shared()
}

fn task_key(task: Option<&CuaComputerTask>) -> String {
    task.map(cua_computer_task_key)
        .unwrap_or_else(|| ANONYMOUS_TASK.to_string())
}

type CloseListener = Box<dyn FnOnce()>;

struct Connection {
    destroyed: Cell<bool>,
    replied: Cell<bool>,
    next_listener: Cell<u64>,
    listeners: RefCell<Vec<(u64, CloseListener)>>,
    deadline: Cell<Instant>,
    deadline_moved: Notify,
}

impl Connection {
    fn new(timeout: Duration) -> Self {
        Self {
            destroyed: Cell::new(false),
            replied: Cell::new(false),
            next_listener: Cell::new(0),
            listeners: RefCell::new(Vec::new()),
            deadline: Cell::new(Instant::now() + timeout),
            deadline_moved: Notify::new(),
        }
    }

    fn destroyed(&self) -> bool {
        self.destroyed.get()
    }

    fn on_close(&self, listener: impl FnOnce() + 'static) -> u64 {
        let id = self.next_listener.get();
        self.next_listener.set(id + 1);
        if !self.destroyed.get() {
            self.listeners.borrow_mut().push((id, Box::new(listener)));
        }
        id
    }

    fn remove_listener(&self, id: u64) {
        self.listeners
            .borrow_mut()
            .retain(|(listener, _)| *listener != id);
    }

    fn close(&self) {
        if self.destroyed.replace(true) {
            return;
        }
        let listeners = std::mem::take(&mut *self.listeners.borrow_mut());
        for (_, listener) in listeners {
            listener();
        }
    }

    fn set_timeout(&self, timeout: Duration) {
        self.deadline.set(Instant::now() + timeout);
        self.deadline_moved.notify_one();
    }
}

struct TaskRequest {
    task: CuaComputerTask,
    stopped: Cell<bool>,
}

struct Admission {
    request: Option<Rc<TaskRequest>>,
    key: Option<String>,
}

impl Admission {
    fn stopped(&self, state: &State) -> bool {
        self.request.as_ref().is_some_and(|r| r.stopped.get())
            || self
                .key
                .as_ref()
                .is_some_and(|key| state.user_stopped_tasks.contains(key))
    }
}

#[derive(Clone)]
struct TaskCursor {
    task: CuaComputerTask,
    first_action_observed: bool,
    enabled: bool,
}

struct ControlSocket {
    destroyed: Cell<bool>,
    shutdown: Abort,
}

impl ControlSocket {
    fn destroy(&self) {
        self.destroyed.set(true);
        self.shutdown.abort();
    }
}

struct Generation {
    child: DriverChild,
    socket: PathBuf,
    session: String,
    control_session: String,
    retired: Cell<bool>,
    cancellation_ready: Cell<bool>,
    native_input_epoch: Cell<u64>,
    input_in_flight: Cell<bool>,
    input_task: RefCell<Option<CuaComputerTask>>,
    browser_input_in_flight: Cell<bool>,
    input_ever_dispatched: Cell<bool>,
    session_opening: RefCell<Option<Fallible>>,
    control_socket: RefCell<Option<Rc<ControlSocket>>>,
    ended_browser_sessions: RefCell<HashSet<String>>,
    live_browser_sessions: RefCell<HashSet<String>>,
    ended_task_sessions: RefCell<Bounded<()>>,
    applied_cursor_style: RefCell<String>,
    applied_session_cursor_styles: RefCell<Bounded<String>>,
    task_cursors: RefCell<Bounded<TaskCursor>>,
    retirement: RefCell<Option<Fallible>>,
}

impl Generation {
    fn new(child: DriverChild, socket: PathBuf) -> Self {
        Self {
            child,
            socket,
            session: format!("synara-{}", uuid::Uuid::new_v4()),
            control_session: format!("synara-transport-{}", uuid::Uuid::new_v4()),
            retired: Cell::new(false),
            cancellation_ready: Cell::new(false),
            native_input_epoch: Cell::new(0),
            input_in_flight: Cell::new(false),
            input_task: RefCell::new(None),
            browser_input_in_flight: Cell::new(false),
            input_ever_dispatched: Cell::new(false),
            session_opening: RefCell::new(None),
            control_socket: RefCell::new(None),
            ended_browser_sessions: RefCell::new(HashSet::new()),
            live_browser_sessions: RefCell::new(HashSet::new()),
            ended_task_sessions: RefCell::new(Bounded::default()),
            applied_cursor_style: RefCell::new(String::new()),
            applied_session_cursor_styles: RefCell::new(Bounded::default()),
            task_cursors: RefCell::new(Bounded::default()),
            retirement: RefCell::new(None),
        }
    }

    fn pid(&self) -> u32 {
        self.child.pid
    }

    fn did_exit(&self) -> bool {
        self.child.did_exit.get()
    }

    fn live(&self) -> bool {
        !self.retired.get() && !self.did_exit()
    }

    fn destroy_control_socket(&self) {
        if let Some(socket) = self.control_socket.borrow_mut().take() {
            socket.destroy();
        }
    }
}

struct PendingCheck {
    id: u64,
    cancel: Rc<Abort>,
    owner: Option<String>,
}

struct State {
    closed: bool,
    disposed: bool,
    suspended: bool,
    input_monitor_armed: bool,
    input_monitor_requested: bool,
    native_input_cleanup_pending: Option<Rc<Generation>>,
    active_foreground_input: bool,
    controlled_targets: Bounded<ControlledTarget>,
    takeover_targets: Bounded<ControlledTarget>,
    user_input: Bounded<(Instant, Option<String>)>,
    browser_targets: Bounded<ControlledTarget>,
    active_input_task_key: Option<String>,
    monitored_tasks: Bounded<String>,
    input_interrupt_cooldown_until: Option<Instant>,
    in_flight_input_interrupts: Vec<Rc<Abort>>,
    active_task_calls: Vec<(Rc<Abort>, String)>,
    desktop_pauses: BTreeSet<String>,
    desktop_observation_required: bool,
    browser_observation_required: bool,
    browser_recovery_observations: Bounded<u64>,
    desktop_epoch: u64,
    desktop_interruption_count: u64,
    observed_native_revision: Option<u64>,
    operations: Chain,
    stopping: Chain,
    cursor_style_updates: Chain,
    retiring: Chain,
    epoch: u64,
    input_monitor_epoch_changes: u64,
    connections: HashMap<String, Rc<Connection>>,
    permissions: Option<HostPermissions>,
    pending_permission_checks: Vec<PendingCheck>,
    user_stopped_tasks: Bounded<()>,
    admitted_task_requests: Vec<Rc<TaskRequest>>,
    known_tasks: Bounded<CuaComputerTask>,
    ended_frame_tasks: Bounded<()>,
    frame_tap_task: Option<CuaComputerTask>,
    generation: Option<Rc<Generation>>,
    starting: Option<Starting>,
    warm_attempted: bool,
}

impl State {
    fn new() -> Self {
        Self {
            closed: false,
            disposed: false,
            suspended: false,
            input_monitor_armed: false,
            input_monitor_requested: false,
            native_input_cleanup_pending: None,
            active_foreground_input: false,
            controlled_targets: Bounded::default(),
            takeover_targets: Bounded::default(),
            user_input: Bounded::default(),
            browser_targets: Bounded::default(),
            active_input_task_key: None,
            monitored_tasks: Bounded::default(),
            input_interrupt_cooldown_until: None,
            in_flight_input_interrupts: Vec::new(),
            active_task_calls: Vec::new(),
            desktop_pauses: BTreeSet::new(),
            desktop_observation_required: false,
            browser_observation_required: false,
            browser_recovery_observations: Bounded::default(),
            desktop_epoch: 0,
            desktop_interruption_count: 0,
            observed_native_revision: None,
            operations: resolved(),
            stopping: resolved(),
            cursor_style_updates: resolved(),
            retiring: resolved(),
            epoch: 0,
            input_monitor_epoch_changes: 0,
            connections: HashMap::new(),
            permissions: None,
            pending_permission_checks: Vec::new(),
            user_stopped_tasks: Bounded::default(),
            admitted_task_requests: Vec::new(),
            known_tasks: Bounded::default(),
            ended_frame_tasks: Bounded::default(),
            frame_tap_task: None,
            generation: None,
            starting: None,
            warm_attempted: false,
        }
    }

    fn cleanup_pending_is(&self, generation: &Rc<Generation>) -> bool {
        self.native_input_cleanup_pending
            .as_ref()
            .is_some_and(|pending| Rc::ptr_eq(pending, generation))
    }

    fn is_current(&self, generation: &Rc<Generation>) -> bool {
        self.generation
            .as_ref()
            .is_some_and(|current| Rc::ptr_eq(current, generation))
    }

    fn cooldown_remaining(&self) -> Option<Duration> {
        self.input_interrupt_cooldown_until
            .and_then(|until| until.checked_duration_since(Instant::now()))
            .filter(|remaining| !remaining.is_zero())
    }

    fn user_paused(&self, task: Option<&CuaComputerTask>) -> bool {
        match task {
            Some(task) => self.user_input.contains(&cua_computer_task_key(task)),
            None => !self.user_input.is_empty(),
        }
    }

    fn last_user_input(&self, task: Option<&CuaComputerTask>) -> Option<Instant> {
        match task {
            Some(task) => self
                .user_input
                .get(&cua_computer_task_key(task))
                .map(|(at, _)| *at),
            None => self.user_input.iter().map(|(_, (at, _))| *at).max(),
        }
    }

    fn end_user_input(&mut self, keys: &[String]) -> Vec<TakeoverEvent> {
        let mut ended = Vec::new();
        for key in keys {
            if let Some((_, Some(thread))) = self.user_input.remove(key) {
                ended.push(thread);
            }
        }
        ended.sort();
        ended.dedup();
        ended
            .into_iter()
            .filter(|thread| {
                !self
                    .user_input
                    .iter()
                    .any(|(_, (_, other))| other.as_ref() == Some(thread))
            })
            .map(|thread_id| TakeoverEvent::Resumed { thread_id })
            .collect()
    }

    fn cancel_permission_checks(&self, owners: Option<&HashSet<String>>) {
        for check in &self.pending_permission_checks {
            let matches = match owners {
                None => true,
                Some(owners) => check.owner.as_ref().is_some_and(|o| owners.contains(o)),
            };
            if matches {
                check.cancel.abort();
            }
        }
    }
}

fn is_mutation_name(name: &str) -> bool {
    CUA_ACTION_TOOLS.contains(&name) || CUA_BROWSER_MUTATION_TOOLS.contains(&name)
}

struct CallScope {
    generation: RefCell<Option<Rc<Generation>>>,
    dispatched: Cell<bool>,
    cancelled_before_dispatch: Cell<bool>,
    cursor_enabled: Cell<Option<bool>>,
}

struct CallRequest {
    name: String,
    input: Value,
    conn: Rc<Connection>,
    model_observation: bool,
    task: Option<CuaComputerTask>,
    foreground_delivery: bool,
}

struct Host {
    options: CuaDriverHostOptions,
    directory: PathBuf,
    state: RefCell<State>,
    next_id: Cell<u64>,
}

impl Host {
    fn next_id(&self) -> u64 {
        let id = self.next_id.get();
        self.next_id.set(id + 1);
        id
    }

    fn monitor_state(&self) -> Option<ComputerInputMonitorState> {
        self.options
            .input_monitor_state
            .as_ref()
            .map(|state| state())
    }

    fn input_monitor_unavailable(&self, monitor: Option<ComputerInputMonitorState>) -> CuaReply {
        input_monitor_unavailable_reply(monitor.as_ref().and_then(|m| m.error.as_deref()))
    }

    fn emit_takeover(&self, events: Vec<TakeoverEvent>) {
        if let Some(on_takeover) = &self.options.on_takeover {
            for event in events {
                on_takeover(event);
            }
        }
    }

    async fn hold_until_quiet(
        &self,
        task: Option<&CuaComputerTask>,
        conn: &Connection,
        admission: &Admission,
    ) -> Option<CuaReply> {
        let quiet = self.options.input_quiet;
        if quiet.is_zero() {
            return None;
        }
        let started = Instant::now();
        let cap = started + self.options.input_quiet_cap;
        let mut held = false;
        loop {
            let release = {
                let state = self.state.borrow();
                if admission.stopped(&state) {
                    return Some(task_stopped_reply());
                }
                if state.closed || state.suspended || conn.destroyed() {
                    return Some(not_dispatched(CANCELLED_BEFORE_DISPATCH));
                }
                match state.last_user_input(task) {
                    Some(at) => at + quiet,
                    None => break,
                }
            };
            let now = Instant::now();
            if now >= release {
                break;
            }
            if now >= cap {
                return Some(user_input_pause_reply());
            }
            held = true;
            tokio::time::sleep((release.min(cap) - now).min(QUIET_POLL)).await;
        }
        if held {
            conn.deadline
                .set(conn.deadline.get().min(started + ENGINE_CALL_TIMEOUT));
            conn.deadline_moved.notify_one();
        }
        None
    }

    fn notify_armed(&self, armed: bool) {
        if let Some(changed) = &self.options.on_input_monitor_armed_change {
            changed(armed);
        }
    }

    fn accept(self: &Rc<Self>, call: ComputerHostCall, reply: oneshot::Sender<CuaReply>) {
        let conn = Rc::new(Connection::new(REQUEST_IDLE_TIMEOUT));
        let id = call.correlation_id.clone();
        self.state
            .borrow_mut()
            .connections
            .insert(id.clone(), conn.clone());
        let handling = tokio::task::spawn_local(self.clone().handle(call, conn.clone()));
        let host = self.clone();
        tokio::task::spawn_local(async move {
            let mut reply = reply;
            let mut handling = handling;
            let outcome = loop {
                let deadline = conn.deadline.get();
                tokio::select! {
                    outcome = &mut handling => break outcome,
                    () = reply.closed(), if !conn.destroyed() => conn.close(),
                    () = tokio::time::sleep_until(deadline.into()), if !conn.destroyed() => {
                        if Instant::now() >= conn.deadline.get() {
                            conn.close();
                        }
                    }
                    () = conn.deadline_moved.notified() => {}
                }
            };
            conn.replied.set(true);
            {
                let mut state = host.state.borrow_mut();
                if state
                    .connections
                    .get(&id)
                    .is_some_and(|current| Rc::ptr_eq(current, &conn))
                {
                    state.connections.remove(&id);
                }
            }
            let mut result = match outcome {
                Ok(Ok(reply)) => reply,
                Ok(Err(error)) => not_dispatched(error),
                Err(_) => not_dispatched("The computer host task failed."),
            };
            host.stamp_desktop_state(&mut result);
            let _ = reply.send(result);
        });
    }

    fn stamp_desktop_state(&self, reply: &mut CuaReply) {
        let state = self.state.borrow();
        reply.desktop_epoch = Some(state.desktop_epoch);
        reply.desktop_pauses = Some(state.desktop_pauses.iter().cloned().collect());
        reply.desktop_interruptions = Some(state.desktop_interruption_count);
        reply.host_platform = Some("darwin".into());
        if let Some(revision) = state.observed_native_revision {
            reply.driver_native_revision = Some(revision);
        }
    }

    async fn handle(
        self: Rc<Self>,
        call: ComputerHostCall,
        conn: Rc<Connection>,
    ) -> Result<CuaReply, String> {
        let name = call.name.clone().unwrap_or_default();
        if call.method != ComputerHostMethod::Call || !CUA_ACTION_TOOLS.contains(&name.as_str()) {
            return self.handle_request(call, conn).await;
        }
        let started = Instant::now();
        let reply = self.handle_request(call.clone(), conn).await;
        log(&computer_action_record(&call, reply.as_ref().ok(), started.elapsed()).to_string());
        reply
    }

    async fn handle_request(
        self: Rc<Self>,
        call: ComputerHostCall,
        conn: Rc<Connection>,
    ) -> Result<CuaReply, String> {
        let task = match &call.task {
            None => None,
            Some(task) => Some(
                serde_json::to_value(task)
                    .ok()
                    .and_then(|value| parse_cua_computer_task(&value))
                    .ok_or("Invalid computer task attribution.")?,
            ),
        };
        let admitted = match (&task, call.method) {
            (Some(task), ComputerHostMethod::Call) => Some(Rc::new(TaskRequest {
                task: task.clone(),
                stopped: Cell::new(false),
            })),
            _ => None,
        };
        if let Some(admitted) = &admitted {
            let mut state = self.state.borrow_mut();
            state.admitted_task_requests.push(admitted.clone());
            state
                .known_tasks
                .touch(cua_computer_task_key(&admitted.task), admitted.task.clone());
        }
        let admission = Admission {
            request: admitted.clone(),
            key: task.as_ref().map(cua_computer_task_key),
        };
        let reply = self
            .clone()
            .handle_authenticated(call, conn, task, admission)
            .await;
        if let Some(admitted) = admitted {
            self.state
                .borrow_mut()
                .admitted_task_requests
                .retain(|request| !Rc::ptr_eq(request, &admitted));
        }
        reply
    }

    async fn handle_authenticated(
        self: Rc<Self>,
        call: ComputerHostCall,
        conn: Rc<Connection>,
        task: Option<CuaComputerTask>,
        admission: Admission,
    ) -> Result<CuaReply, String> {
        match call.method {
            ComputerHostMethod::Stop => {
                if let Some(task) = task {
                    let scope = self.stop_task_input(task).await?;
                    return Ok(ok_with(json!({ "stop_scope": scope })));
                }
                let serving = {
                    let state = self.state.borrow();
                    !state.closed && !state.suspended
                };
                if serving {
                    self.interrupt_input().await?;
                } else {
                    self.stop().await?;
                }
                return Ok(ok_reply());
            }
            ComputerHostMethod::EndTask => {
                let task = task.ok_or("Computer task attribution is required.")?;
                let all_turns = task.turn_id.is_none();
                self.end_task(task, all_turns, true).await?;
                return Ok(ok_reply());
            }
            ComputerHostMethod::Shield => return self.handle_shield(&call, task).await,
            ComputerHostMethod::EndBrowserThread => {
                let task = task.ok_or("Computer task attribution is required.")?;
                self.end_browser_thread(task).await;
                return Ok(ok_reply());
            }
            _ => {}
        }
        {
            let state = self.state.borrow();
            if state.closed {
                return Err("Computer host is closed.".into());
            }
            if state.suspended {
                return Err("Computer host is suspended while the backend is stopping.".into());
            }
            if admission.stopped(&state) {
                return Ok(task_stopped_reply());
            }
        }
        let name = call.name.clone();
        let model_observation = call.model_observation == Some(true);
        let active_computer_work = call.method == ComputerHostMethod::Call
            && name.as_deref().is_some_and(|name| {
                name != "check_permissions"
                    && (is_mutation_name(name)
                        || (task.is_some()
                            && model_observation
                            && (CUA_READ_TOOLS.contains(&name)
                                || CUA_BROWSER_TOOLS.contains(&name))))
            });
        if active_computer_work {
            let (activation_epoch, activation_changes) = {
                let mut state = self.state.borrow_mut();
                state.input_monitor_requested = true;
                (state.epoch, state.input_monitor_epoch_changes)
            };
            if let Some(activate) = &self.options.activate_input_monitor {
                activate().await?;
            }
            let cancelled = {
                let state = self.state.borrow();
                activation_epoch != state.epoch
                    || conn.destroyed()
                    || state.closed
                    || state.suspended
                    || admission.stopped(&state)
            };
            if cancelled {
                let monitor = self.monitor_state();
                let state = self.state.borrow();
                if !conn.destroyed()
                    && !state.closed
                    && !state.suspended
                    && monitor.as_ref().is_some_and(|m| !m.ready)
                    && state.epoch - activation_epoch
                        == state.input_monitor_epoch_changes - activation_changes
                {
                    drop(state);
                    return Ok(self.input_monitor_unavailable(monitor));
                }
                return Ok(not_dispatched(
                    "Cancelled before listener activation completed.",
                ));
            }
            let mut state = self.state.borrow_mut();
            if self.options.activate_input_monitor.is_some() {
                state.input_monitor_armed = true;
            }
            if let Some(task) = &task {
                state
                    .monitored_tasks
                    .set(cua_computer_task_key(task), task.thread_id.clone());
            }
        }
        if self.options.check_permissions.is_none()
            && (call.method == ComputerHostMethod::Probe
                || (call.method == ComputerHostMethod::Call
                    && name.as_deref() == Some("check_permissions")))
        {
            self.warm();
        }
        if call.method == ComputerHostMethod::Probe {
            if !self.options.binary_path.exists() {
                return Ok(CuaReply {
                    ok: false,
                    error: Some(CUA_DRIVER_MISSING_MESSAGE.into()),
                    ..CuaReply::default()
                });
            }
            let running = self.state.borrow().generation.is_some();
            return Ok(ok_with(
                json!({ "version": CUA_DRIVER_VERSION, "running": running }),
            ));
        }
        if call.method == ComputerHostMethod::Setup {
            conn.set_timeout(Duration::from_millis(CUA_SETUP_TIMEOUT_MS));
            self.stop().await?;
            let cancelled = {
                let state = self.state.borrow();
                conn.destroyed() || state.closed || state.suspended
            };
            if cancelled {
                return Ok(not_dispatched("Cancelled before permission setup."));
            }
            (self.options.setup)().await?;
            return Ok(ok_reply());
        }
        let name = match name {
            Some(name)
                if call.method == ComputerHostMethod::Call
                    && (CUA_READ_TOOLS.contains(&name.as_str())
                        || CUA_ACTION_TOOLS.contains(&name.as_str())
                        || CUA_BROWSER_TOOLS.contains(&name.as_str())) =>
            {
                name
            }
            _ => return Err("Unsupported computer host request.".into()),
        };
        let is_browser = CUA_BROWSER_TOOLS.contains(&name.as_str());
        if is_browser && task.is_none() {
            return Err("Computer browser calls require task attribution.".into());
        }
        if is_browser
            && let Some(pid) = js_safe_integer(call.args.as_object().and_then(|a| a.get("pid")))
            && u32::try_from(pid).is_ok_and(|pid| self.own_pids().contains(&pid))
        {
            return Ok(browser_self_target_reply());
        }
        let (refuse, hold) = {
            let state = self.state.borrow();
            let paused = !self.options.input_quiet.is_zero() && state.user_paused(task.as_ref());
            let read_of_target = name == "get_window_state"
                && model_observation
                && task.as_ref().is_some_and(|task| {
                    state
                        .takeover_targets
                        .get(&cua_computer_task_key(task))
                        .zip(target_from_args(&call.args, None))
                        .is_some_and(|(held, read)| held.pid == read.pid)
                });
            let refuse = paused
                && (CUA_ACTION_TOOLS.contains(&name.as_str()) || name == "check_input_ready");
            let reply = if !refuse {
                None
            } else if admission.stopped(&state) {
                Some(task_stopped_reply())
            } else {
                Some(user_input_pause_reply())
            };
            (reply, paused && read_of_target)
        };
        if let Some(reply) = refuse {
            return Ok(reply);
        }
        if hold
            && let Some(reply) = self
                .hold_until_quiet(task.as_ref(), &conn, &admission)
                .await
        {
            return Ok(reply);
        }
        let (previous, stopping, epoch) = {
            let state = self.state.borrow();
            if !state.desktop_pauses.is_empty() {
                return Ok(desktop_pause_reply(true));
            }
            (
                state.operations.clone(),
                state.stopping.clone(),
                state.epoch,
            )
        };
        let request = CallRequest {
            name,
            input: call.args.clone(),
            conn,
            model_observation,
            task,
            foreground_delivery: call.delivery_mode == Some(ComputerDeliveryMode::Foreground),
        };
        let host = self.clone();
        let operation = eager(async move {
            previous.await;
            stopping.await;
            host.run_call_operation(request, admission, epoch).await
        });
        self.state.borrow_mut().operations = unit(operation.clone());
        operation.await
    }

    async fn run_call_operation(
        self: Rc<Self>,
        request: CallRequest,
        admission: Admission,
        epoch: u64,
    ) -> Result<CuaReply, String> {
        let name = request.name.clone();
        let conn = request.conn.clone();
        let task = request.task.clone();
        {
            let state = self.state.borrow();
            if state.closed || state.suspended || conn.destroyed() || epoch != state.epoch {
                return Ok(not_dispatched(CANCELLED_BEFORE_DISPATCH));
            }
            if !state.desktop_pauses.is_empty() {
                return Ok(desktop_pause_reply(true));
            }
        }
        if !self.input_monitor_available(&name) {
            return Ok(self.input_monitor_unavailable(self.monitor_state()));
        }
        let pending = self.state.borrow().native_input_cleanup_pending.clone();
        if is_mutation_name(&name)
            && let Some(pending) = pending
        {
            self.interrupt_native_input(&pending).await?;
            if epoch != self.state.borrow().epoch || conn.destroyed() {
                return Ok(not_dispatched(
                    "Cancelled while waiting for native input cleanup.",
                ));
            }
        }
        {
            let state = self.state.borrow();
            if is_mutation_name(&name)
                && let Some(remaining) = state.cooldown_remaining()
            {
                return Ok(input_interrupted_reply(
                    remaining,
                    state.user_paused(task.as_ref()),
                ));
            }
            if admission.stopped(&state) {
                return Ok(task_stopped_reply());
            }
        }
        if name == "check_permissions" && self.options.check_permissions.is_some() {
            return self
                .check_permissions_call(&conn, task.as_ref(), &admission, epoch)
                .await;
        }
        let browser_recovery_setup = is_isolated_browser_setup(&name, &request.input);
        let browser_recovery_observed =
            self.has_browser_recovery_observation(&request.input, task.as_ref());
        {
            let mut state = self.state.borrow_mut();
            let task_takeover = state.takeover_targets.contains(&task_key(task.as_ref()))
                || (task.is_none() && !state.takeover_targets.is_empty());
            let action = CUA_ACTION_TOOLS.contains(&name.as_str());
            let browser_mutation = CUA_BROWSER_MUTATION_TOOLS.contains(&name.as_str());
            let unrecovered_browser =
                browser_mutation && !browser_recovery_setup && !browser_recovery_observed;
            let takeover_gated =
                task_takeover && (action || unrecovered_browser || name == "check_input_ready");
            let gated = (state.desktop_observation_required
                && (action || name == "check_input_ready"))
                || (state.browser_observation_required && unrecovered_browser)
                || takeover_gated;
            if gated {
                let user_input = takeover_gated && state.user_paused(task.as_ref());
                drop(state);
                log(&format!(
                    "refused {name}: fresh desktop observation still required"
                ));
                return Ok(if user_input {
                    user_input_pause_reply()
                } else {
                    desktop_pause_reply(false)
                });
            }
            if let Some(task) = &task
                && (request.model_observation
                    || CUA_ACTION_TOOLS.contains(&name.as_str())
                    || CUA_BROWSER_TOOLS.contains(&name.as_str()))
            {
                state.frame_tap_task = Some(task.clone());
            }
        }
        if self.targets_protected_self(&name, &request.input) {
            log(&format!(
                "refused {name}: a protected surface is showing in this app"
            ));
            return Ok(self_protected_surface_reply());
        }
        let input = request.input.clone();
        let model_observation = request.model_observation;
        let reply = self.clone().call(request).await;
        let Some(task) = task else {
            return Ok(reply);
        };
        let frame_eligible = {
            let state = self.state.borrow();
            !state
                .ended_frame_tasks
                .contains(&cua_computer_task_key(&task))
                && !admission.stopped(&state)
                && epoch == state.epoch
        };
        let refused = structured(&reply).is_some_and(|s| {
            s.get("effect").and_then(Value::as_str) == Some("refused")
                || s.get("status").and_then(Value::as_str) == Some("refused")
        });
        if frame_eligible
            && !conn.destroyed()
            && reply.ok
            && !is_error(&reply)
            && !refused
            && (model_observation
                || CUA_ACTION_TOOLS.contains(&name.as_str())
                || CUA_BROWSER_TOOLS.contains(&name.as_str()))
        {
            if let Some(target) = frame_tap_target(&task, &input) {
                if let Some(tap) = &self.options.frame_tap
                    && let Err(error) = tap.update(target)
                {
                    log(&format!("computer frame tap update failed: {error}"));
                }
            } else if name == "launch_app" {
                let host = self.clone();
                tokio::task::spawn_local(async move {
                    host.prime_tap_after_launch(task, input, conn, epoch).await;
                });
            }
        }
        Ok(reply)
    }

    async fn check_permissions_call(
        self: &Rc<Self>,
        conn: &Rc<Connection>,
        task: Option<&CuaComputerTask>,
        admission: &Admission,
        epoch: u64,
    ) -> Result<CuaReply, String> {
        let cancelled = || {
            let state = self.state.borrow();
            state.closed
                || state.suspended
                || conn.destroyed()
                || epoch != state.epoch
                || admission.stopped(&state)
        };
        let cancelled_reply = || not_dispatched("Cancelled before permission check completed.");
        let Some(mut permissions) = self.check_permissions_wait(conn, false, task).await? else {
            return Ok(cancelled_reply());
        };
        if cancelled() {
            return Ok(cancelled_reply());
        }
        let known = self.state.borrow().permissions;
        if known.is_some_and(|known| known.changed_from(&permissions)) {
            let Some(confirmed) = self.check_permissions_wait(conn, true, task).await? else {
                return Ok(cancelled_reply());
            };
            if cancelled() {
                return Ok(cancelled_reply());
            }
            permissions = confirmed;
        }
        let known = self.state.borrow().permissions;
        if let Some(known) = known
            && known.changed_from(&permissions)
        {
            let generation = {
                let mut state = self.state.borrow_mut();
                state.epoch += 1;
                state.desktop_epoch += 1;
                state.desktop_observation_required = true;
                state.browser_observation_required = true;
                state.generation.clone()
            };
            log(&format!(
                "permission state changed accessibility {} -> {}, screen_recording {} -> {}; requiring fresh desktop observation",
                known.accessibility,
                permissions.accessibility,
                known.screen_recording,
                permissions.screen_recording
            ));
            if let Some(generation) = generation {
                self.retire(&generation).await?;
            }
        }
        self.state.borrow_mut().permissions = Some(permissions);
        if permissions.accessibility
            && permissions.screen_recording
            && permissions.input_monitoring != Some(false)
        {
            self.warm();
        }
        let monitor = if self.state.borrow().input_monitor_requested {
            self.monitor_state()
        } else {
            None
        };
        let mut content = Map::new();
        content.insert("accessibility".into(), json!(permissions.accessibility));
        content.insert(
            "screen_recording".into(),
            json!(permissions.screen_recording),
        );
        if let Some(input_monitoring) = permissions.input_monitoring {
            content.insert("input_monitoring".into(), json!(input_monitoring));
        }
        if let Some(monitor) = &monitor {
            content.insert("input_monitor_ready".into(), json!(monitor.ready));
            if let Some(error) = &monitor.error {
                content.insert("input_monitor_error".into(), json!(error));
            }
        }
        content.insert(
            "source".into(),
            json!({
                "attribution": "host",
                "host_bundle_id": self.options.bundle_id,
                "probe": "appsnap-permission-helper",
            }),
        );
        Ok(CuaReply {
            ok: true,
            result: Some(CuaToolResult {
                structured_content: Some(content),
                ..CuaToolResult::default()
            }),
            ..CuaReply::default()
        })
    }

    async fn check_permissions_wait(
        self: &Rc<Self>,
        conn: &Rc<Connection>,
        force: bool,
        task: Option<&CuaComputerTask>,
    ) -> Result<Option<HostPermissions>, String> {
        let Some(check) = &self.options.check_permissions else {
            return Ok(None);
        };
        let cancel = Rc::new(Abort::new());
        let id = self.next_id();
        self.state
            .borrow_mut()
            .pending_permission_checks
            .push(PendingCheck {
                id,
                cancel: cancel.clone(),
                owner: task.map(cua_computer_task_key),
            });
        let listener = conn.on_close({
            let cancel = cancel.clone();
            move || cancel.abort()
        });
        let running = tokio::task::spawn_local(check(force));
        let outcome = tokio::select! {
            outcome = running => Some(outcome),
            () = cancel.wait() => None,
        };
        self.state
            .borrow_mut()
            .pending_permission_checks
            .retain(|check| check.id != id);
        conn.remove_listener(listener);
        match outcome {
            None => Ok(None),
            Some(Ok(Ok(permissions))) => Ok(Some(permissions)),
            Some(Ok(Err(error))) => Err(error),
            Some(Err(_)) => Err("The permission check failed.".into()),
        }
    }

    async fn end_browser_thread(self: &Rc<Self>, task: CuaComputerTask) {
        let previous = {
            let mut state = self.state.borrow_mut();
            state
                .browser_targets
                .retain(|_, target| target.thread_id.as_deref() != Some(task.thread_id.as_str()));
            state.operations.clone()
        };
        let host = self.clone();
        let operation = eager(async move {
            previous.await;
            let stopping = host.state.borrow().stopping.clone();
            stopping.await;
            let (closed, generation) = {
                let state = host.state.borrow();
                (state.closed, state.generation.clone())
            };
            let label = browser_session_label(&task.thread_id);
            let Some(generation) = generation.filter(|g| !closed && g.live()) else {
                return;
            };
            let control_open = generation
                .control_socket
                .borrow()
                .as_ref()
                .is_some_and(|socket| !socket.destroyed.get());
            let known = generation.live_browser_sessions.borrow().contains(&label)
                || generation.ended_browser_sessions.borrow().contains(&label);
            if !control_open || !known {
                return;
            }
            let _ = cua_request(
                &generation.socket,
                &json!({
                    "method": "call",
                    "name": "end_session",
                    "args": { "session": label },
                    "session_id": generation.control_session,
                }),
                CLEANUP_TIMEOUT,
                false,
                None,
            )
            .await;
            generation.live_browser_sessions.borrow_mut().remove(&label);
            generation.ended_browser_sessions.borrow_mut().insert(label);
        });
        self.state.borrow_mut().operations = unit(operation.clone());
        operation.await;
    }

    async fn handle_shield(
        self: &Rc<Self>,
        call: &ComputerHostCall,
        task: Option<CuaComputerTask>,
    ) -> Result<CuaReply, String> {
        let args = parse_cua_shield_args(&call.args).ok_or("Invalid computer shield request.")?;
        let shield = self.options.shield.clone();
        match args {
            CuaShieldArgs::Engage {
                shield_id,
                frame,
                window_id,
                pid,
                label,
            } => {
                {
                    let state = self.state.borrow();
                    if state.closed || state.suspended {
                        return Ok(not_dispatched(
                            "The activation shield is unavailable while the computer host is stopped.",
                        ));
                    }
                    if !state.desktop_pauses.is_empty() {
                        return Ok(not_dispatched(
                            "The activation shield is unavailable while the desktop is paused. Observe the desktop again before activating windows.",
                        ));
                    }
                }
                let Some(shield) = shield else {
                    return Ok(not_dispatched(
                        "The activation shield is not available in this build.",
                    ));
                };
                let engagement = ComputerShieldEngagement {
                    shield_id: shield_id.clone(),
                    frame,
                    window_id,
                    pid,
                    label,
                };
                if let Err(error) = shield.engage(engagement, task).await {
                    return Ok(not_dispatched(format!(
                        "The activation shield could not be shown: {error}"
                    )));
                }
                Ok(ok_with(json!({ "engaged": true, "shield_id": shield_id })))
            }
            CuaShieldArgs::Release { shield_id } => {
                if let Some(shield) = shield {
                    shield.release(shield_id).await?;
                }
                Ok(ok_reply())
            }
            CuaShieldArgs::ReleaseAll => {
                let released = match shield {
                    Some(shield) => shield.release_all().await?,
                    None => 0,
                };
                Ok(ok_with(json!({ "released": released })))
            }
        }
    }

    async fn call(self: Rc<Self>, request: CallRequest) -> CuaReply {
        let is_browser = CUA_BROWSER_TOOLS.contains(&request.name.as_str());
        let mutation = if is_browser {
            CUA_BROWSER_MUTATION_TOOLS.contains(&request.name.as_str())
        } else {
            CUA_ACTION_TOOLS.contains(&request.name.as_str())
        };
        let call_cancel = Rc::new(Abort::new());
        {
            let mut state = self.state.borrow_mut();
            if mutation {
                state.in_flight_input_interrupts.push(call_cancel.clone());
            }
            if let Some(task) = &request.task {
                state
                    .active_task_calls
                    .push((call_cancel.clone(), cua_computer_task_key(task)));
            }
        }
        let scope = Rc::new(CallScope {
            generation: RefCell::new(None),
            dispatched: Cell::new(false),
            cancelled_before_dispatch: Cell::new(false),
            cursor_enabled: Cell::new(None),
        });
        let conn = request.conn.clone();
        let listener = conn.on_close({
            let host = Rc::downgrade(&self);
            let conn = Rc::downgrade(&conn);
            let scope = scope.clone();
            let call_cancel = call_cancel.clone();
            move || on_call_connection_closed(host, conn, scope, call_cancel, mutation)
        });
        let outcome = self
            .clone()
            .call_inner(&request, &scope, &call_cancel, is_browser, mutation)
            .await;
        let reply = match outcome {
            Ok(reply) => reply,
            Err(mut detail) => {
                let generation = scope.generation.borrow().clone();
                if let Some(generation) = generation
                    && !scope.cancelled_before_dispatch.get()
                    && !call_cancel.is_aborted()
                    && let Err(cleanup) = self.retire(&generation).await
                {
                    detail = format!("{detail}; {cleanup}");
                }
                CuaReply {
                    ok: false,
                    error: Some(detail),
                    effect: Some(if scope.dispatched.get() && mutation {
                        CuaEffect::DispatchedUnknown
                    } else {
                        CuaEffect::NotDispatched
                    }),
                    ..CuaReply::default()
                }
            }
        };
        {
            let mut state = self.state.borrow_mut();
            if mutation {
                state.active_foreground_input = false;
                state.active_input_task_key = None;
            }
            state
                .in_flight_input_interrupts
                .retain(|interrupt| !Rc::ptr_eq(interrupt, &call_cancel));
            state
                .active_task_calls
                .retain(|(interrupt, _)| !Rc::ptr_eq(interrupt, &call_cancel));
        }
        conn.remove_listener(listener);
        reply
    }

    async fn call_inner(
        self: Rc<Self>,
        request: &CallRequest,
        scope: &CallScope,
        call_cancel: &Rc<Abort>,
        is_browser: bool,
        mutation: bool,
    ) -> Result<CuaReply, String> {
        let name = request.name.as_str();
        let task = request.task.as_ref();
        let conn = &request.conn;
        let (admitted_epoch, admitted_desktop_epoch) = {
            let state = self.state.borrow();
            (state.epoch, state.desktop_epoch)
        };
        let label = task
            .filter(|_| is_browser)
            .map(|task| browser_session_label(&task.thread_id));
        let agent_label = task.filter(|_| !is_browser).map(agent_session_label);
        let cancelled = |scope: &CallScope| {
            scope.cancelled_before_dispatch.set(!scope.dispatched.get());
            Err::<CuaReply, String>(CANCELLED_BEFORE_DISPATCH.into())
        };
        let mut reply = None;
        let mut generation = None;
        for attempt in 0..2 {
            let current = self.ensure_started().await?;
            *scope.generation.borrow_mut() = Some(current.clone());
            generation = Some(current.clone());
            let blocked = {
                let state = self.state.borrow();
                conn.destroyed()
                    || current.retired.get()
                    || admitted_epoch != state.epoch
                    || !state.desktop_pauses.is_empty()
                    || call_cancel.is_aborted()
            };
            if blocked {
                return cancelled(scope);
            }
            let args = request.input.as_object().cloned().unwrap_or_default();
            let mut browser_session_id = None;
            if let Some(label) = &label {
                self.ensure_control_session(&current).await?;
                if current.ended_browser_sessions.borrow().contains(label) {
                    let revived = cua_tool_request(
                        &current.socket,
                        &json!({
                            "method": "call",
                            "name": "start_session",
                            "args": { "session": label },
                            "session_id": current.control_session,
                        }),
                        SESSION_REVIVE_TIMEOUT,
                    )
                    .await
                    .map_err(|e| e.message)?;
                    if revived.ok && !is_error(&revived) {
                        current.ended_browser_sessions.borrow_mut().remove(label);
                    }
                }
                browser_session_id = Some(current.control_session.clone());
            } else if let Some(agent_label) = &agent_label
                && current.ended_task_sessions.borrow().contains(agent_label)
            {
                let revived = cua_tool_request(
                    &current.socket,
                    &json!({
                        "method": "call",
                        "name": "start_session",
                        "args": { "session": agent_label },
                    }),
                    SESSION_REVIVE_TIMEOUT,
                )
                .await
                .map_err(|e| e.message)?;
                if revived.ok && !is_error(&revived) {
                    current.ended_task_sessions.borrow_mut().remove(agent_label);
                    current
                        .applied_session_cursor_styles
                        .borrow_mut()
                        .remove(agent_label);
                }
            }
            if let Some(agent_label) = &agent_label {
                self.apply_cursor_style_for_session(&current, agent_label)
                    .await;
                let enabled = current
                    .task_cursors
                    .borrow()
                    .get(agent_label)
                    .is_some_and(|cursor| cursor.enabled);
                if !enabled {
                    scope.cursor_enabled.set(Some(
                        self.set_task_cursor_enabled(&current, agent_label, true)
                            .await,
                    ));
                }
            }
            let blocked = admitted_epoch != self.state.borrow().epoch
                || conn.destroyed()
                || call_cancel.is_aborted();
            if blocked {
                return cancelled(scope);
            }
            if !self.input_monitor_available(name) {
                return Ok(self.input_monitor_unavailable(self.monitor_state()));
            }
            if (mutation || request.model_observation)
                && let Some(target) = self.controlled_target(&request.input, task, is_browser)
            {
                self.state
                    .borrow_mut()
                    .controlled_targets
                    .set(task_key(task), target);
            }
            if mutation {
                let mut state = self.state.borrow_mut();
                state.active_input_task_key = Some(task_key(task));
                state.active_foreground_input = request.foreground_delivery
                    || args.get("delivery_mode").and_then(Value::as_str) == Some("foreground")
                    || name == "bring_to_front";
            }
            scope.dispatched.set(true);
            if let Some(label) = &label {
                current
                    .live_browser_sessions
                    .borrow_mut()
                    .insert(label.clone());
            }
            if mutation {
                current.input_in_flight.set(true);
                current.browser_input_in_flight.set(is_browser);
                *current.input_task.borrow_mut() = task.cloned();
            }
            if current.input_in_flight.get() {
                current.input_ever_dispatched.set(true);
            }
            let mut driver_args = args;
            driver_args.insert(
                "session".into(),
                json!(
                    label
                        .as_deref()
                        .or(agent_label.as_deref())
                        .unwrap_or(&current.session)
                ),
            );
            let mut envelope = Map::new();
            envelope.insert("method".into(), json!("call"));
            envelope.insert("name".into(), json!(name));
            if mutation && self.options.native_revision.is_some() {
                envelope.insert(
                    "expected_input_epoch".into(),
                    json!(current.native_input_epoch.get()),
                );
            }
            envelope.insert("args".into(), Value::Object(driver_args));
            if let Some(session_id) = &browser_session_id {
                envelope.insert("session_id".into(), json!(session_id));
            }
            let raw = cua_request(
                &current.socket,
                &Value::Object(envelope),
                DRIVER_ACTION_TIMEOUT,
                mutation,
                Some(call_cancel),
            )
            .await
            .map_err(|e| e.message)?;
            let attempt_reply = tool_reply(raw, mutation).map_err(|e| e.message)?;
            let cleanup_unconfirmed = structured(&attempt_reply)
                .and_then(|s| s.get("input_cleanup_unconfirmed"))
                == Some(&Value::Bool(true));
            if cleanup_unconfirmed {
                current.input_in_flight.set(true);
                if is_browser {
                    current.browser_input_in_flight.set(true);
                }
                self.state.borrow_mut().native_input_cleanup_pending = Some(current.clone());
            } else if mutation && !self.state.borrow().cleanup_pending_is(&current) {
                current.input_in_flight.set(false);
                current.browser_input_in_flight.set(false);
                *current.input_task.borrow_mut() = None;
            }
            if is_driver_session_death(&attempt_reply) {
                if let Some(label) = &label {
                    current.live_browser_sessions.borrow_mut().remove(label);
                    current
                        .ended_browser_sessions
                        .borrow_mut()
                        .insert(label.clone());
                    if attempt == 0 {
                        continue;
                    }
                } else if let Some(agent_label) = &agent_label {
                    current
                        .ended_task_sessions
                        .borrow_mut()
                        .insert(agent_label.clone());
                    if attempt == 0 {
                        continue;
                    }
                } else if attempt == 0 {
                    let _ = self.retire(&current).await;
                    continue;
                }
            }
            reply = Some(attempt_reply);
            break;
        }
        let (Some(mut reply), Some(generation)) = (reply, generation) else {
            return Err(CANCELLED_BEFORE_DISPATCH.into());
        };
        let focus_restore_failed = structured(&reply).is_some_and(|s| {
            s.get("code").and_then(Value::as_str) == Some("focus_restore_failed")
                || parse_cua_action_diagnostics(&Value::Object(s.clone()))
                    .and_then(|d| d.error_code)
                    == Some(CuaActionErrorCode::FocusRestoreFailed)
        });
        if mutation && focus_restore_failed {
            let mut state = self.state.borrow_mut();
            state.desktop_observation_required = true;
            state.epoch += 1;
            state.desktop_epoch += 1;
            let key = task_key(task);
            if let Some(target) = state.controlled_targets.get(&key).cloned() {
                state.takeover_targets.set(key, target);
            }
        }
        let desktop_epoch = self.state.borrow().desktop_epoch;
        if admitted_desktop_epoch != desktop_epoch
            && (CUA_READ_TOOLS.contains(&name) || name == "get_browser_state")
        {
            log(&format!(
                "refused stale {name} read (desktop epoch {admitted_desktop_epoch} -> {desktop_epoch})"
            ));
            return Ok(desktop_pause_reply(
                !self.state.borrow().desktop_pauses.is_empty(),
            ));
        }
        if is_browser
            && let Some(task) = task
            && reply.ok
            && !is_error(&reply)
        {
            self.remember_browser_target(&request.input, reply.result.as_ref(), task);
        }
        let observation_admitted = {
            let state = self.state.borrow();
            request.model_observation
                && !conn.destroyed()
                && generation.live()
                && admitted_epoch == state.epoch
                && state.desktop_pauses.is_empty()
                && reply.ok
                && !is_error(&reply)
        };
        if observation_admitted && let Some(result) = &reply.result {
            let native_observation = (name == "get_window_state" || name == "get_desktop_state")
                && result
                    .structured_content
                    .as_ref()
                    .and_then(|s| s.get("screenshot_frame_valid"))
                    != Some(&Value::Bool(false))
                && (result.content.iter().flatten().any(|part| {
                    part.kind == "image" && part.data.as_deref().is_some_and(|d| !d.is_empty())
                }) || result
                    .structured_content
                    .as_ref()
                    .and_then(|s| s.get("elements"))
                    .is_some_and(Value::is_array));
            let browser_observation =
                name == "get_browser_state" && is_browser_snapshot(&request.input, result);
            if native_observation || browser_observation {
                let mut state = self.state.borrow_mut();
                if native_observation {
                    state.desktop_observation_required = false;
                }
                if browser_observation && let Some(key) = browser_recovery_key(&request.input, task)
                {
                    let epoch = state.desktop_epoch;
                    state.browser_recovery_observations.set(key, epoch);
                }
                let key = task_key(task);
                let mut resumed = Vec::new();
                if let Some(interrupted) = state.takeover_targets.get(&key)
                    && observation_matches_target(name, &request.input, interrupted, result)
                {
                    state.takeover_targets.remove(&key);
                    resumed = state.end_user_input(std::slice::from_ref(&key));
                }
                drop(state);
                self.emit_takeover(resumed);
                log(&format!(
                    "fresh model observation via {name}; matching input gate cleared"
                ));
            }
        }
        if name == "get_desktop_state"
            && let (Some(result), Some(normalize)) =
                (reply.result.as_mut(), &self.options.normalize_overview)
        {
            normalize(result);
        }
        if let (Some(agent_label), Some(task)) = (&agent_label, task)
            && !is_driver_session_death(&reply)
        {
            let cursor = generation.task_cursors.borrow().get(agent_label).cloned();
            let same_turn = cursor
                .as_ref()
                .is_some_and(|c| cua_computer_task_key(&c.task) == cua_computer_task_key(task));
            let first_action_observed = cursor.as_ref().is_some_and(|c| c.first_action_observed);
            let first_action = mutation && (!same_turn || !first_action_observed);
            generation.task_cursors.borrow_mut().push_unbounded(
                agent_label.clone(),
                TaskCursor {
                    task: task.clone(),
                    first_action_observed: mutation || (same_turn && first_action_observed),
                    enabled: scope
                        .cursor_enabled
                        .get()
                        .or(cursor.as_ref().map(|c| c.enabled))
                        .unwrap_or(false),
                },
            );
            if cursor.is_none() || first_action {
                let stage = if first_action {
                    "first-action"
                } else {
                    "session-created"
                };
                self.log_cursor_state(&generation, agent_label, task, stage)
                    .await;
            }
            loop {
                let oldest = {
                    let cursors = generation.task_cursors.borrow();
                    if cursors.len() <= BOUNDED_ENTRIES {
                        break;
                    }
                    cursors.oldest_key()
                };
                let Some(oldest) = oldest else { break };
                if !self.end_cursor_session(&generation, &oldest).await {
                    generation.task_cursors.borrow_mut().remove(&oldest);
                }
            }
        }
        Ok(reply)
    }

    async fn log_cursor_state(
        &self,
        generation: &Rc<Generation>,
        label: &str,
        task: &CuaComputerTask,
        stage: &str,
    ) {
        if self.state.borrow().observed_native_revision == Some(0) {
            return;
        }
        let mut fields = Map::new();
        fields.insert("event".into(), json!("computer_cursor"));
        fields.insert("ts".into(), json!(now_iso()));
        fields.insert("thread".into(), json!(task.thread_id));
        if let Some(turn) = &task.turn_id {
            fields.insert("turn".into(), json!(turn));
        }
        fields.insert("stage".into(), json!(stage));
        match cua_tool_request(
            &generation.socket,
            &json!({
                "method": "call",
                "name": "get_agent_cursor_state",
                "args": { "session": label },
            }),
            CURSOR_STATE_TIMEOUT,
        )
        .await
        {
            Ok(reply) => {
                let status = if reply.ok && !is_error(&reply) {
                    "reported"
                } else {
                    "unavailable"
                };
                fields.insert("status".into(), json!(status));
                if let Some(state) = structured(&reply) {
                    if let Some(enabled) = state.get("enabled").and_then(Value::as_bool) {
                        fields.insert("enabled".into(), json!(enabled));
                    }
                    if let Some(position) = state.get("position") {
                        fields.insert("has_position".into(), json!(!position.is_null()));
                    }
                    if let Some(idle) = state
                        .get("motion")
                        .and_then(|m| m.get("idle_hide_ms"))
                        .and_then(Value::as_f64)
                        .filter(|v| v.is_finite())
                    {
                        fields.insert("idle_hide_ms".into(), json!(idle));
                    }
                    for key in ["overlay_ready", "render_visible"] {
                        if let Some(value) = state.get(key).and_then(Value::as_bool) {
                            fields.insert(key.into(), json!(value));
                        }
                    }
                    if state.get("overlay_scope").and_then(Value::as_str) == Some("main_display") {
                        fields.insert("overlay_scope".into(), json!("main_display"));
                    }
                }
            }
            Err(_) => {
                fields.insert("status".into(), json!("query-failed"));
            }
        }
        log(&Value::Object(fields).to_string());
    }

    async fn end_cursor_session(&self, generation: &Rc<Generation>, label: &str) -> bool {
        let cursor = generation.task_cursors.borrow().get(label).cloned();
        let hidden = self.set_task_cursor_enabled(generation, label, false).await;
        if hidden {
            generation.task_cursors.borrow_mut().remove(label);
        } else if let Some(cursor) = generation.task_cursors.borrow_mut().get_mut(label) {
            cursor.enabled = false;
        }
        let mut fields = Map::new();
        fields.insert("event".into(), json!("computer_cursor"));
        fields.insert("ts".into(), json!(now_iso()));
        if let Some(cursor) = &cursor {
            fields.insert("thread".into(), json!(cursor.task.thread_id));
            if let Some(turn) = &cursor.task.turn_id {
                fields.insert("turn".into(), json!(turn));
            }
        }
        fields.insert("stage".into(), json!("task-end"));
        fields.insert(
            "status".into(),
            json!(if hidden { "hidden" } else { "hide-failed" }),
        );
        log(&Value::Object(fields).to_string());
        hidden
    }

    async fn set_task_cursor_enabled(
        &self,
        generation: &Rc<Generation>,
        label: &str,
        enabled: bool,
    ) -> bool {
        cua_tool_request(
            &generation.socket,
            &json!({
                "method": "call",
                "name": "set_agent_cursor_enabled",
                "args": { "session": label, "enabled": enabled },
            }),
            CURSOR_ENABLE_TIMEOUT,
        )
        .await
        .is_ok_and(|reply| reply.ok && !is_error(&reply))
    }

    fn end_task_cursors(self: &Rc<Self>, task: CuaComputerTask, all_turns: bool) -> Fallible {
        let previous = self.state.borrow().operations.clone();
        let host = self.clone();
        let operation = eager(async move {
            previous.await;
            let generation = host.state.borrow().generation.clone();
            let Some(generation) = generation.filter(|g| g.live()) else {
                return Ok(());
            };
            let labels: Vec<String> = generation
                .task_cursors
                .borrow()
                .iter()
                .filter(|(_, cursor)| {
                    cursor.task.thread_id == task.thread_id
                        && (all_turns || cursor.task.turn_id == task.turn_id)
                })
                .map(|(label, _)| label.to_string())
                .collect();
            for label in labels {
                host.end_cursor_session(&generation, &label).await;
            }
            Ok(())
        });
        self.state.borrow_mut().operations = unit(operation.clone());
        operation
    }

    async fn end_task(
        self: &Rc<Self>,
        task: CuaComputerTask,
        all_turns: bool,
        wait_for_cursor: bool,
    ) -> Result<(), String> {
        let (disarm, resumed) = {
            let mut state = self.state.borrow_mut();
            let key = cua_computer_task_key(&task);
            let mut ended = vec![key.clone()];
            if all_turns {
                ended.extend(
                    state
                        .user_input
                        .iter()
                        .filter(|(_, (_, thread))| {
                            thread.as_deref() == Some(task.thread_id.as_str())
                        })
                        .map(|(key, _)| key.to_string()),
                );
            }
            let resumed = state.end_user_input(&ended);
            state.controlled_targets.remove(&key);
            state.takeover_targets.remove(&key);
            state.monitored_tasks.remove(&key);
            if all_turns {
                let thread = task.thread_id.as_str();
                state
                    .controlled_targets
                    .retain(|_, target| target.thread_id.as_deref() != Some(thread));
                state
                    .takeover_targets
                    .retain(|_, target| target.thread_id.as_deref() != Some(thread));
                state.monitored_tasks.retain(|_, owner| owner != thread);
            }
            let disarm = state.monitored_tasks.is_empty();
            if disarm {
                state.input_monitor_requested = false;
                state.input_monitor_armed = false;
            }
            state.ended_frame_tasks.insert(key);
            if let Some(tapped) = state.frame_tap_task.clone()
                && tapped.thread_id == task.thread_id
                && (all_turns || task.turn_id == tapped.turn_id)
            {
                state
                    .ended_frame_tasks
                    .insert(cua_computer_task_key(&tapped));
                state.frame_tap_task = None;
            }
            (disarm, resumed)
        };
        self.emit_takeover(resumed);
        if disarm {
            self.notify_armed(false);
        }
        let cursor_ended = self.end_task_cursors(task.clone(), all_turns);
        if !wait_for_cursor {
            let pending = cursor_ended.clone();
            tokio::task::spawn_local(async move {
                if let Err(error) = pending.await {
                    log(&format!("stopped task cursor cleanup failed: {error}"));
                }
            });
        }
        let frame = {
            let tap = self.options.frame_tap.clone();
            let task = task.clone();
            async move {
                match tap {
                    Some(tap) => tap.end_task(task).await,
                    None => Ok(()),
                }
            }
        };
        let shield = {
            let shield = self.options.shield.clone();
            let task = task.clone();
            async move {
                match shield {
                    Some(shield) => shield.end_task(task).await,
                    None => Ok(()),
                }
            }
        };
        let cursor = async move {
            if wait_for_cursor {
                cursor_ended.await
            } else {
                Ok(())
            }
        };
        futures::try_join!(frame, shield, cursor)?;
        Ok(())
    }

    fn warm(self: &Rc<Self>) {
        {
            let mut state = self.state.borrow_mut();
            if state.warm_attempted
                || state.closed
                || state.suspended
                || !self.options.warm_on_first_touch
            {
                return;
            }
            state.warm_attempted = true;
        }
        let starting = self.ensure_spawned();
        tokio::task::spawn_local(async move {
            if let Err(error) = starting.await {
                log(&format!("driver warm-up failed: {error}"));
            }
        });
    }

    async fn ensure_control_session(&self, generation: &Rc<Generation>) -> Result<(), String> {
        let existing = generation.control_socket.borrow().clone();
        if let Some(existing) = &existing {
            if !existing.destroyed.get() {
                return Ok(());
            }
            let live: Vec<String> = generation
                .live_browser_sessions
                .borrow_mut()
                .drain()
                .collect();
            generation.ended_browser_sessions.borrow_mut().extend(live);
        }
        let socket = Rc::new(ControlSocket {
            destroyed: Cell::new(false),
            shutdown: Abort::new(),
        });
        *generation.control_socket.borrow_mut() = Some(socket.clone());
        let (begun, begin_result) = oneshot::channel();
        let path = generation.socket.clone();
        let session_id = generation.control_session.clone();
        let owner = socket.clone();
        tokio::task::spawn_local(async move {
            let result = run_control_connection(&path, &session_id, &owner, begun).await;
            owner.destroyed.set(true);
            result
        });
        let outcome = begin_result
            .await
            .unwrap_or_else(|_| Err("session_begin connection closed".into()));
        if let Err(error) = outcome {
            socket.destroy();
            let mut slot = generation.control_socket.borrow_mut();
            if slot.as_ref().is_some_and(|s| Rc::ptr_eq(s, &socket)) {
                *slot = None;
            }
            return Err(error);
        }
        Ok(())
    }

    fn ensure_spawned(self: &Rc<Self>) -> Starting {
        if let Some(starting) = self.state.borrow().starting.clone() {
            return starting;
        }
        let retiring = self.state.borrow().retiring.clone();
        let host = self.clone();
        let starting = eager(async move {
            let result = host.clone().start(retiring).await;
            host.state.borrow_mut().starting = None;
            result
        });
        self.state.borrow_mut().starting = Some(starting.clone());
        starting
    }

    async fn start(self: Rc<Self>, retiring: Chain) -> Result<Rc<Generation>, String> {
        retiring.await;
        let current = {
            let state = self.state.borrow();
            if state.closed {
                return Err("Computer host is closed.".into());
            }
            state.generation.clone()
        };
        if let Some(current) = current {
            if current.live() {
                return Ok(current);
            }
            self.retire(&current).await?;
        }
        if !self.options.binary_path.exists() {
            return Err(CUA_DRIVER_MISSING_MESSAGE.into());
        }
        let suffix: String = uuid::Uuid::new_v4()
            .simple()
            .to_string()
            .chars()
            .take(8)
            .collect();
        let socket = self.directory.join(format!("driver-{suffix}.sock"));
        let child = spawn_driver(&SpawnPlan {
            binary: &self.options.binary_path,
            socket: &socket,
            runtime_dir: &self.directory,
            bundle_id: &self.options.bundle_id,
            patched: self.options.native_revision.is_some(),
        })
        .map_err(|error| error.to_string())?;
        let generation = Rc::new(Generation::new(child, socket));
        self.state.borrow_mut().generation = Some(generation.clone());
        self.update_input_monitor_armed();
        match self.handshake(&generation).await {
            Ok(()) => Ok(generation),
            Err(error) => {
                self.retire(&generation).await?;
                Err(error)
            }
        }
    }

    async fn handshake(&self, generation: &Rc<Generation>) -> Result<(), String> {
        let mut metadata = None;
        for _ in 0..HANDSHAKE_ATTEMPTS {
            if !generation.live() {
                return Err(STOPPED_DURING_STARTUP.into());
            }
            match cua_request(
                &generation.socket,
                &json!({ "method": "metadata" }),
                HANDSHAKE_ATTEMPT_TIMEOUT,
                false,
                None,
            )
            .await
            {
                Ok(reply) => {
                    metadata = Some(reply);
                    break;
                }
                Err(_) => tokio::time::sleep(HANDSHAKE_RETRY_DELAY).await,
            }
        }
        let expected = self.options.native_revision;
        let result = metadata.as_ref().and_then(|m| m.get("result"));
        let field = |key: &str| result.and_then(|r| r.get(key));
        let reported = field("synara_native_revision");
        let valid = metadata.as_ref().and_then(|m| m.get("ok")) == Some(&Value::Bool(true))
            && field("driver_version").and_then(Value::as_str) == Some(CUA_DRIVER_VERSION)
            && expected.is_none_or(|expected| js_strict_eq(reported, Some(&json!(expected))))
            && field("embedded") == Some(&Value::Bool(true))
            && js_strict_eq(field("pid"), Some(&json!(generation.pid())));
        if !valid {
            return Err("Cua Driver identity/version/native revision handshake failed.".into());
        }
        self.state.borrow_mut().observed_native_revision = Some(
            js_safe_integer(reported)
                .and_then(|r| u64::try_from(r).ok())
                .unwrap_or(0),
        );
        if !generation.live() {
            return Err(STOPPED_DURING_STARTUP.into());
        }
        generation.cancellation_ready.set(expected.is_some());
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&generation.socket, std::fs::Permissions::from_mode(0o600))
            .map_err(|error| error.to_string())?;
        if !generation.live() {
            return Err(STOPPED_DURING_STARTUP.into());
        }
        Ok(())
    }

    async fn open_session(self: &Rc<Self>, generation: &Rc<Generation>) -> Result<(), String> {
        let opening = {
            let mut slot = generation.session_opening.borrow_mut();
            slot.get_or_insert_with(|| {
                let host = self.clone();
                let generation = generation.clone();
                eager(async move { host.open_session_body(&generation).await })
            })
            .clone()
        };
        if let Err(error) = opening.await {
            self.retire(generation).await?;
            return Err(error);
        }
        Ok(())
    }

    async fn open_session_body(&self, generation: &Rc<Generation>) -> Result<(), String> {
        let timeout = self.options.startup_timeout;
        if !generation.live() {
            return Err(STOPPED_DURING_STARTUP.into());
        }
        let session = cua_tool_request(
            &generation.socket,
            &json!({
                "method": "call",
                "name": "start_session",
                "args": { "session": generation.session },
            }),
            timeout,
        )
        .await
        .map_err(|e| e.message)?;
        if !session.ok || is_error(&session) {
            return Err("Cua session initialization failed.".into());
        }
        if !generation.live() {
            return Err(STOPPED_DURING_STARTUP.into());
        }
        let motion = cua_tool_request(
            &generation.socket,
            &json!({
                "method": "call",
                "name": "set_agent_cursor_motion",
                "args": {
                    "session": generation.session,
                    "glide_duration_ms": 100,
                    "dwell_after_click_ms": 0,
                },
            }),
            timeout,
        )
        .await
        .map_err(|e| e.message)?;
        if !motion.ok || is_error(&motion) {
            return Err("Cua cursor initialization failed.".into());
        }
        let style = normalize_cua_cursor_style(self.cursor_style().as_ref());
        if let Some(style) = style
            && self.state.borrow().observed_native_revision != Some(0)
        {
            match cua_tool_request(
                &generation.socket,
                &json!({
                    "method": "call",
                    "name": "set_agent_cursor_style",
                    "args": cursor_style_args(&generation.session, Some(&style)),
                }),
                timeout,
            )
            .await
            {
                Ok(reply) if reply.ok && !is_error(&reply) => {
                    *generation.applied_cursor_style.borrow_mut() = cursor_style_json(Some(&style));
                }
                Ok(_) => log("shared cursor session style was refused; keeping the stock cursor"),
                Err(error) => log(&format!(
                    "shared cursor session style failed: {}",
                    error.message
                )),
            }
        }
        Ok(())
    }

    async fn ensure_started(self: &Rc<Self>) -> Result<Rc<Generation>, String> {
        let generation = self.ensure_spawned().await?;
        self.open_session(&generation).await?;
        Ok(generation)
    }

    fn retire(self: &Rc<Self>, generation: &Rc<Generation>) -> Fallible {
        if let Some(retirement) = generation.retirement.borrow().clone() {
            return retirement;
        }
        generation.retired.set(true);
        let previous = {
            let mut state = self.state.borrow_mut();
            state.browser_targets.clear();
            state.browser_recovery_observations.clear();
            if state.cleanup_pending_is(generation) {
                state.native_input_cleanup_pending = None;
            }
            state.retiring.clone()
        };
        self.update_input_monitor_armed();
        generation.destroy_control_socket();
        let host = self.clone();
        let retiring_generation = generation.clone();
        let retirement = eager(async move {
            previous.await;
            host.retire_body(&retiring_generation).await
        });
        *generation.retirement.borrow_mut() = Some(retirement.clone());
        self.state.borrow_mut().retiring = unit(retirement.clone());
        retirement
    }

    fn clear_generation(&self, generation: &Rc<Generation>) {
        {
            let mut state = self.state.borrow_mut();
            if state.is_current(generation) {
                state.generation = None;
            }
        }
        let _ = std::fs::remove_file(&generation.socket);
    }

    async fn release_held_input_after_retire(
        &self,
        generation: &Generation,
        input_uncertain: bool,
    ) -> bool {
        let Some(release) = &self.options.release_held_input else {
            return false;
        };
        if !input_uncertain || generation.browser_input_in_flight.get() {
            return false;
        }
        match release().await {
            Ok(()) => {
                log("released held input left by the dead driver generation");
                true
            }
            Err(error) => {
                log(&format!("held-input release failed: {error}"));
                false
            }
        }
    }

    async fn retire_body(&self, generation: &Rc<Generation>) -> Result<(), String> {
        let input_uncertain = generation.input_in_flight.get();
        if generation.did_exit() && generation.input_in_flight.get() {
            if self
                .release_held_input_after_retire(generation, input_uncertain)
                .await
            {
                self.clear_generation(generation);
                return Ok(());
            }
            return Err(EXITED_WITHOUT_CLEANUP.into());
        }
        if !generation.did_exit() && generation.cancellation_ready.get() {
            let confirmed = match cua_request(
                &generation.socket,
                &json!({
                    "method": "cancel_input",
                    "args": { "expected_pid": generation.pid() },
                }),
                CLEANUP_TIMEOUT,
                false,
                None,
            )
            .await
            {
                Ok(reply) => {
                    reply.get("ok") == Some(&Value::Bool(true))
                        && cua_cleanup_acknowledged(reply.get("result"), Some(generation.pid()))
                }
                Err(_) => false,
            };
            if !confirmed {
                if !generation.did_exit() {
                    tokio::select! {
                        () = generation.child.exited.clone() => {}
                        () = tokio::time::sleep(EXIT_GRACE) => {}
                    }
                }
                if generation.did_exit() {
                    let cleared = !generation.input_in_flight.get()
                        || self
                            .release_held_input_after_retire(generation, input_uncertain)
                            .await;
                    if cleared {
                        self.clear_generation(generation);
                        return Ok(());
                    }
                    return Err(EXITED_WITHOUT_CLEANUP.into());
                }
                if !generation.input_ever_dispatched.get() {
                    generation.child.terminate().await;
                    self.clear_generation(generation);
                    return Ok(());
                }
                self.release_held_input_after_retire(generation, input_uncertain)
                    .await;
                return Err(CLEANUP_NOT_CONFIRMED.into());
            }
            generation.input_in_flight.set(false);
        }
        generation.child.terminate().await;
        self.clear_generation(generation);
        Ok(())
    }

    fn cursor_style(&self) -> Option<CuaCursorStyle> {
        self.options.cursor_style.as_ref().and_then(|style| style())
    }

    fn set_cursor_style(self: &Rc<Self>, style: Option<CuaCursorStyle>) -> Chain {
        let next = normalize_cua_cursor_style(style.as_ref());
        let previous = self.state.borrow().cursor_style_updates.clone();
        let host = self.clone();
        let update = eager(async move {
            previous.await;
            host.push_live_cursor_style(next).await;
        });
        self.state.borrow_mut().cursor_style_updates = update.clone();
        update
    }

    async fn push_live_cursor_style(&self, next: Option<CuaCursorStyle>) {
        let next_json = cursor_style_json(next.as_ref());
        let generation = self.state.borrow().generation.clone();
        let Some(generation) = generation.filter(|g| g.live()) else {
            return;
        };
        let opening = generation.session_opening.borrow().clone();
        let Some(opening) = opening else {
            return;
        };
        let _ = opening.await;
        let skip = self.state.borrow().closed
            || !generation.live()
            || *generation.applied_cursor_style.borrow() == next_json
            || self.state.borrow().observed_native_revision == Some(0);
        if skip {
            return;
        }
        match cua_tool_request(
            &generation.socket,
            &json!({
                "method": "call",
                "name": "set_agent_cursor_style",
                "args": cursor_style_args(&generation.session, next.as_ref()),
            }),
            CURSOR_STYLE_TIMEOUT,
        )
        .await
        {
            Ok(reply) if reply.ok && !is_error(&reply) => {
                *generation.applied_cursor_style.borrow_mut() = next_json;
            }
            Ok(_) => log("live cursor style push was refused; keeping the previous style"),
            Err(error) => log(&format!("live cursor style push failed: {}", error.message)),
        }
    }

    async fn apply_cursor_style_for_session(&self, generation: &Rc<Generation>, session: &str) {
        let style = normalize_cua_cursor_style(self.cursor_style().as_ref());
        let previous = generation
            .applied_session_cursor_styles
            .borrow()
            .get(session)
            .cloned();
        if style.is_none() && previous.is_none() {
            return;
        }
        let style_json = cursor_style_json(style.as_ref());
        if previous.as_deref() == Some(style_json.as_str()) {
            return;
        }
        if self.state.borrow().observed_native_revision == Some(0) {
            return;
        }
        match cua_tool_request(
            &generation.socket,
            &json!({
                "method": "call",
                "name": "set_agent_cursor_style",
                "args": cursor_style_args(session, style.as_ref()),
            }),
            CURSOR_STYLE_TIMEOUT,
        )
        .await
        {
            Ok(reply) if reply.ok && !is_error(&reply) => {
                let mut applied = generation.applied_session_cursor_styles.borrow_mut();
                if style.is_some() {
                    applied.set(session.to_string(), style_json);
                } else {
                    applied.remove(session);
                }
            }
            Ok(_) => log("task cursor style was refused; keeping the previous cursor"),
            Err(error) => log(&format!(
                "task cursor style setup failed: {}",
                error.message
            )),
        }
    }

    fn stop(self: &Rc<Self>) -> Fallible {
        let (was_armed, admitted, previous, resumed) = {
            let mut state = self.state.borrow_mut();
            state.input_monitor_requested = false;
            let was_armed = std::mem::replace(&mut state.input_monitor_armed, false);
            let keys: Vec<String> = state
                .user_input
                .iter()
                .map(|(key, _)| key.to_string())
                .collect();
            let resumed = state.end_user_input(&keys);
            state.controlled_targets.clear();
            state.takeover_targets.clear();
            state.browser_targets.clear();
            state.browser_recovery_observations.clear();
            state.monitored_tasks.clear();
            state.epoch += 1;
            state.desktop_epoch += 1;
            state.cancel_permission_checks(None);
            (
                was_armed,
                state.operations.clone(),
                state.stopping.clone(),
                resumed,
            )
        };
        self.emit_takeover(resumed);
        if was_armed {
            self.notify_armed(false);
        }
        let frame_tap_stopped = self.stop_frame_tap();
        let shield_stopped = self.stop_shield();
        let host = self.clone();
        let stopping = eager(async move {
            previous.await;
            let generation = host.state.borrow().generation.clone();
            if let Some(generation) = generation {
                host.retire(&generation).await?;
            }
            let starting = host.state.borrow().starting.clone();
            if let Some(starting) = starting {
                let _ = starting.await;
            }
            let generation = host.state.borrow().generation.clone();
            if let Some(generation) = generation {
                host.retire(&generation).await?;
            }
            admitted.await;
            let retiring = host.state.borrow().retiring.clone();
            retiring.await;
            if let Some(stopped) = frame_tap_stopped {
                stopped.await?;
            }
            if let Some(stopped) = shield_stopped {
                stopped.await?;
            }
            Ok(())
        });
        self.state.borrow_mut().stopping = unit(stopping.clone());
        stopping
    }

    fn stop_frame_tap(&self) -> Option<Fallible> {
        let tap = self.options.frame_tap.clone()?;
        Some(eager(async move { tap.stop().await }))
    }

    fn stop_shield(&self) -> Option<Fallible> {
        let shield = self.options.shield.clone()?;
        Some(eager(async move { shield.stop().await }))
    }

    fn interrupt_input(self: &Rc<Self>) -> Fallible {
        let (admitted, previous) = {
            let mut state = self.state.borrow_mut();
            state.epoch += 1;
            state.desktop_epoch += 1;
            state.cancel_permission_checks(None);
            (state.operations.clone(), state.stopping.clone())
        };
        let frame_tap_stopped = self.stop_frame_tap();
        let shield_stopped = self.stop_shield();
        for interrupt in self.state.borrow().in_flight_input_interrupts.iter() {
            interrupt.abort();
        }
        let host = self.clone();
        let interrupting = eager(async move {
            previous.await;
            let starting = host.state.borrow().starting.clone();
            if let Some(starting) = starting {
                let _ = starting.await;
            }
            let generation = host.state.borrow().generation.clone();
            if let Some(generation) = generation {
                host.interrupt_native_input(&generation).await?;
            }
            admitted.await;
            let retiring = host.state.borrow().retiring.clone();
            retiring.await;
            if let Some(stopped) = frame_tap_stopped {
                stopped.await?;
            }
            if let Some(stopped) = shield_stopped {
                stopped.await?;
            }
            Ok(())
        });
        self.state.borrow_mut().stopping = unit(interrupting.clone());
        interrupting
    }

    async fn interrupt_native_input(&self, generation: &Rc<Generation>) -> Result<(), String> {
        if !generation.live() || self.options.native_revision.is_none() {
            return Ok(());
        }
        self.state.borrow_mut().native_input_cleanup_pending = Some(generation.clone());
        let confirmed = match cua_request(
            &generation.socket,
            &json!({
                "method": "interrupt_input",
                "args": { "expected_pid": generation.pid() },
            }),
            CLEANUP_TIMEOUT,
            false,
            None,
        )
        .await
        {
            Ok(reply) => interrupt_acknowledged(
                &reply,
                generation.pid(),
                generation.native_input_epoch.get(),
            ),
            Err(_) => None,
        };
        if let Some(epoch) = confirmed {
            generation.native_input_epoch.set(epoch);
        }
        if !generation.live() {
            return Ok(());
        }
        if confirmed.is_none() {
            if !generation.browser_input_in_flight.get()
                && let Some(release) = &self.options.release_held_input
                && let Err(error) = release().await
            {
                log(&format!("interrupted held-input release failed: {error}"));
            }
            return Err(INTERRUPT_NOT_CONFIRMED.into());
        }
        generation.input_in_flight.set(false);
        generation.browser_input_in_flight.set(false);
        *generation.input_task.borrow_mut() = None;
        let mut state = self.state.borrow_mut();
        if state.cleanup_pending_is(generation) {
            state.native_input_cleanup_pending = None;
        }
        Ok(())
    }

    fn spawn_interrupt(self: &Rc<Self>, context: &'static str) {
        let interrupting = self.interrupt_input();
        tokio::task::spawn_local(async move {
            if let Err(error) = interrupting.await {
                log(&format!("{context}: {error}"));
            }
        });
    }

    fn emergency_stop_input(self: &Rc<Self>) -> bool {
        {
            let mut state = self.state.borrow_mut();
            if state.closed || (state.generation.is_none() && state.starting.is_none()) {
                return false;
            }
            state.desktop_observation_required = true;
            state.browser_observation_required = true;
            state.input_interrupt_cooldown_until = Some(Instant::now() + ESCAPE_INPUT_COOLDOWN);
        }
        log("physical Escape: interrupting computer input");
        self.spawn_interrupt("emergency input interrupt failed");
        true
    }

    fn physical_input(self: &Rc<Self>, event: PhysicalComputerInput) -> bool {
        let mut state = self.state.borrow_mut();
        if state.closed || !state.generation.as_ref().is_some_and(|g| !g.retired.get()) {
            return false;
        }
        let foreground = state.active_foreground_input;
        let active_key = state.active_input_task_key.clone();
        let affected: Vec<(String, ControlledTarget)> = state
            .controlled_targets
            .iter()
            .filter(|(key, target)| {
                (foreground && active_key.as_deref() == Some(*key))
                    || (event.pid == Some(target.pid)
                        && (event.window_id.is_none()
                            || target.window_id.is_none()
                            || target.window_id == event.window_id))
            })
            .map(|(key, target)| (key.to_string(), target.clone()))
            .collect();
        if !foreground && affected.is_empty() {
            return false;
        }
        let already_paused = state.desktop_observation_required
            || state.browser_observation_required
            || !state.takeover_targets.is_empty();
        if !already_paused {
            let targets: Vec<Value> = affected
                .iter()
                .map(|(_, target)| {
                    json!({ "thread": target.thread_id, "pid": target.pid, "windowId": target.window_id })
                })
                .collect();
            log(&json!({
                "event": "computer_physical_input",
                "ts": now_iso(),
                "pid": event.pid,
                "windowId": event.window_id,
                "targets": targets,
                "foreground": foreground,
            })
            .to_string());
        }
        let now = Instant::now();
        let mut paused = Vec::new();
        for (key, target) in &affected {
            if !state.user_input.contains(key)
                && let Some(thread_id) = &target.thread_id
            {
                paused.push(TakeoverEvent::Paused {
                    thread_id: thread_id.clone(),
                    pid: target.pid,
                    window_id: target.window_id,
                });
            }
            state
                .user_input
                .set(key.clone(), (now, target.thread_id.clone()));
            state.takeover_targets.set(key.clone(), target.clone());
        }
        if foreground && affected.is_empty() {
            state.desktop_observation_required = true;
            state.browser_observation_required = true;
        }
        let affected_input_in_flight = (foreground
            || affected
                .iter()
                .any(|(key, _)| active_key.as_deref() == Some(key.as_str())))
            && state
                .in_flight_input_interrupts
                .iter()
                .any(|interrupt| !interrupt.is_aborted());
        state.input_interrupt_cooldown_until = Some(now + ESCAPE_INPUT_COOLDOWN);
        if already_paused && !affected_input_in_flight {
            state.epoch += 1;
            state.desktop_epoch += 1;
            drop(state);
            self.emit_takeover(paused);
            return true;
        }
        drop(state);
        self.emit_takeover(paused);
        self.spawn_interrupt("human takeover interrupt failed");
        true
    }

    fn input_monitor_state_changed(self: &Rc<Self>, monitor: ComputerInputMonitorState) {
        {
            let mut state = self.state.borrow_mut();
            if monitor.ready
                || matches!(
                    monitor.error.as_deref(),
                    Some("input_monitor_idle" | "input_monitor_starting")
                )
                || state.closed
                || !state.generation.as_ref().is_some_and(|g| !g.retired.get())
            {
                return;
            }
            state.desktop_observation_required = true;
            state.browser_observation_required = true;
            state.input_monitor_epoch_changes += 1;
        }
        self.spawn_interrupt("input listener interruption failed");
    }

    fn update_input_monitor_armed(&self) {
        let changed = {
            let mut state = self.state.borrow_mut();
            let armed = !state.closed
                && state.generation.as_ref().is_some_and(|g| !g.retired.get())
                && (self.options.activate_input_monitor.is_none() || state.input_monitor_requested);
            if armed == state.input_monitor_armed {
                None
            } else {
                state.input_monitor_armed = armed;
                Some(armed)
            }
        };
        if let Some(armed) = changed {
            self.notify_armed(armed);
        }
    }

    fn own_pids(&self) -> HashSet<u32> {
        match &self.options.own_pids {
            Some(pids) => pids(),
            None => HashSet::from([std::process::id()]),
        }
    }

    fn targets_protected_self(&self, name: &str, input: &Value) -> bool {
        CUA_ACTION_TOOLS.contains(&name)
            && js_safe_integer(input.as_object().and_then(|args| args.get("pid")))
                .and_then(|pid| u32::try_from(pid).ok())
                .is_some_and(|pid| self.own_pids().contains(&pid))
            && self
                .options
                .protected_surface
                .as_ref()
                .is_some_and(|showing| showing())
    }

    async fn stop_task_input(
        self: &Rc<Self>,
        task: CuaComputerTask,
    ) -> Result<&'static str, String> {
        let matches = |candidate: &CuaComputerTask| {
            candidate.thread_id == task.thread_id
                && (task.turn_id.is_none() || candidate.turn_id == task.turn_id)
        };
        let scope = {
            let mut state = self.state.borrow_mut();
            let mut stopped = HashSet::from([cua_computer_task_key(&task)]);
            state
                .user_stopped_tasks
                .insert(cua_computer_task_key(&task));
            let known: Vec<CuaComputerTask> = state
                .known_tasks
                .iter()
                .filter(|(_, known)| matches(known))
                .map(|(_, known)| known.clone())
                .collect();
            for known in known {
                stopped.insert(cua_computer_task_key(&known));
                state
                    .user_stopped_tasks
                    .insert(cua_computer_task_key(&known));
            }
            let admitted: Vec<Rc<TaskRequest>> = state
                .admitted_task_requests
                .iter()
                .filter(|request| matches(&request.task))
                .cloned()
                .collect();
            for request in admitted {
                request.stopped.set(true);
                stopped.insert(cua_computer_task_key(&request.task));
                state
                    .user_stopped_tasks
                    .insert(cua_computer_task_key(&request.task));
            }
            state.cancel_permission_checks(Some(&stopped));
            for (interrupt, owner) in &state.active_task_calls {
                if stopped.contains(owner) {
                    interrupt.abort();
                }
            }
            let owns_input = state.generation.as_ref().is_some_and(|generation| {
                generation.input_in_flight.get()
                    && generation.input_task.borrow().as_ref().is_some_and(matches)
            });
            if owns_input { "generation" } else { "task" }
        };
        let interrupted = if scope == "generation" {
            Some(self.interrupt_input())
        } else {
            None
        };
        let interrupted = async move {
            match interrupted {
                Some(interrupting) => interrupting.await,
                None => Ok(()),
            }
        };
        let all_turns = task.turn_id.is_none();
        futures::try_join!(interrupted, self.end_task(task.clone(), all_turns, false))?;
        let mut record = Map::new();
        record.insert("event".into(), json!("computer_task_stop"));
        record.insert("thread".into(), json!(task.thread_id));
        if let Some(turn) = &task.turn_id {
            record.insert("turn".into(), json!(turn));
        }
        record.insert("scope".into(), json!(scope));
        log(&Value::Object(record).to_string());
        Ok(scope)
    }

    fn input_monitor_available(&self, name: &str) -> bool {
        let required = CUA_ACTION_TOOLS.contains(&name)
            || (self.options.native_revision.is_some()
                && CUA_BROWSER_MUTATION_TOOLS.contains(&name));
        if !required {
            return true;
        }
        self.monitor_state().is_none_or(|monitor| monitor.ready)
    }

    fn has_browser_recovery_observation(
        &self,
        input: &Value,
        task: Option<&CuaComputerTask>,
    ) -> bool {
        let Some(key) = browser_recovery_key(input, task) else {
            return false;
        };
        let state = self.state.borrow();
        state.browser_recovery_observations.get(&key) == Some(&state.desktop_epoch)
    }

    fn controlled_target(
        &self,
        input: &Value,
        task: Option<&CuaComputerTask>,
        browser: bool,
    ) -> Option<ControlledTarget> {
        let args = input.as_object()?;
        if browser
            && let Some(task) = task
            && let Some(target_id) = args.get("target_id").and_then(Value::as_str)
            && let Some(bound) = self
                .state
                .borrow()
                .browser_targets
                .get(&browser_target_key(&task.thread_id, target_id))
        {
            let mut bound = bound.clone();
            if let Some(tab) = args.get("tab_id").and_then(Value::as_str) {
                bound.browser_tab_id = Some(tab.to_string());
            }
            return Some(bound);
        }
        target_from_args(input, task)
    }

    fn remember_browser_target(
        &self,
        input: &Value,
        result: Option<&CuaToolResult>,
        task: &CuaComputerTask,
    ) {
        let Some(data) = result.and_then(|r| r.structured_content.as_ref()) else {
            return;
        };
        let Some(target_id) = data.get("target_id").and_then(Value::as_str) else {
            return;
        };
        if data.get("status").and_then(Value::as_str) != Some("ok")
            || data.get("mode").and_then(Value::as_str) != Some("bind")
        {
            return;
        }
        let Some(target) = self.controlled_target(input, Some(task), false) else {
            return;
        };
        let bound = ControlledTarget {
            browser_target_id: Some(target_id.to_string()),
            ..target
        };
        let mut state = self.state.borrow_mut();
        state.browser_targets.set(
            browser_target_key(&task.thread_id, target_id),
            bound.clone(),
        );
        state
            .controlled_targets
            .push_unbounded(cua_computer_task_key(task), bound);
    }

    async fn prime_tap_after_launch(
        self: Rc<Self>,
        task: CuaComputerTask,
        input: Value,
        conn: Rc<Connection>,
        epoch: u64,
    ) {
        let candidates = launch_app_match_names(&input);
        let Some(tap) = self.options.frame_tap.clone() else {
            return;
        };
        if candidates.is_empty() {
            return;
        }
        let reply = self
            .clone()
            .call(CallRequest {
                name: "list_windows".into(),
                input: json!({}),
                conn,
                model_observation: false,
                task: None,
                foreground_delivery: false,
            })
            .await;
        let key = cua_computer_task_key(&task);
        let still_wanted = |host: &Host| {
            let state = host.state.borrow();
            epoch == state.epoch
                && !state.ended_frame_tasks.contains(&key)
                && !state.user_stopped_tasks.contains(&key)
        };
        if !still_wanted(&self) || !reply.ok || is_error(&reply) {
            return;
        }
        let Some(windows) = structured(&reply)
            .and_then(|s| s.get("windows"))
            .and_then(Value::as_array)
        else {
            return;
        };
        let mut best: Option<(u32, u32, f64)> = None;
        for row in windows {
            let Some(record) = row.as_object() else {
                continue;
            };
            let bounds = record.get("bounds");
            let dimension = |key: &str| {
                bounds
                    .and_then(|b| b.get(key))
                    .and_then(Value::as_f64)
                    .unwrap_or(0.0)
            };
            let (width, height) = (dimension("width"), dimension("height"));
            let pid = js_safe_integer(record.get("pid")).filter(|p| *p > 0);
            let window_id = js_safe_integer(record.get("window_id")).filter(|w| *w > 0);
            let (Some(pid), Some(window_id)) = (pid, window_id) else {
                continue;
            };
            if record.get("is_on_screen") != Some(&Value::Bool(true))
                || width <= 0.0
                || height <= 0.0
            {
                continue;
            }
            let app_name = record
                .get("app_name")
                .and_then(Value::as_str)
                .map(str::to_lowercase)
                .unwrap_or_default();
            if !candidates
                .iter()
                .any(|candidate| app_name == *candidate || app_name.contains(candidate.as_str()))
            {
                continue;
            }
            let area = width * height;
            if best.is_none_or(|(_, _, best_area)| area > best_area) {
                best = Some((pid as u32, window_id as u32, area));
            }
        }
        let Some((pid, window_id, _)) = best else {
            log("computer frame tap launch prime: no on-screen window matched the launched app");
            return;
        };
        if !still_wanted(&self) {
            return;
        }
        log(&format!(
            "computer frame tap launch prime: streaming pid {pid} window {window_id}"
        ));
        if let Err(error) = tap.update(zeron_proto::computer::CuaPreviewTarget {
            task,
            pid,
            window_id,
            cursor: None,
        }) {
            log(&format!("computer frame tap launch prime failed: {error}"));
        }
    }

    fn suspend(self: &Rc<Self>) -> Fallible {
        self.state.borrow_mut().suspended = true;
        self.stop()
    }

    fn resume(&self) {
        let mut state = self.state.borrow_mut();
        if !state.closed {
            state.suspended = false;
        }
    }

    fn pause_desktop(self: &Rc<Self>, reason: String) -> Fallible {
        {
            let mut state = self.state.borrow_mut();
            state.desktop_pauses.insert(reason.clone());
            state.desktop_interruption_count += 1;
            state.desktop_observation_required = true;
            state.browser_observation_required = true;
        }
        log(&format!(
            "desktop input paused ({reason}); requiring fresh desktop observation"
        ));
        self.stop()
    }

    fn resume_desktop(&self, reason: &str) {
        let mut state = self.state.borrow_mut();
        if state.desktop_pauses.remove(reason) {
            let remaining = state.desktop_pauses.len();
            drop(state);
            log(&format!(
                "desktop pause \"{reason}\" lifted; {remaining} pause(s) remain"
            ));
        }
    }

    async fn dispose(self: Rc<Self>) -> Result<(), String> {
        {
            let mut state = self.state.borrow_mut();
            state.closed = true;
            state.disposed = true;
        }
        self.update_input_monitor_armed();
        let stopped = self.stop().await;
        if let Some(tap) = &self.options.frame_tap {
            let _ = tap.dispose().await;
        }
        if let Some(shield) = &self.options.shield {
            let _ = shield.dispose().await;
        }
        let connections: Vec<Rc<Connection>> = self
            .state
            .borrow_mut()
            .connections
            .drain()
            .map(|(_, conn)| conn)
            .collect();
        for conn in connections {
            conn.close();
        }
        if self.state.borrow().generation.is_none() {
            let _ = std::fs::remove_dir_all(&self.directory);
        }
        stopped
    }
}

fn on_call_connection_closed(
    host: Weak<Host>,
    conn: Weak<Connection>,
    scope: Rc<CallScope>,
    call_cancel: Rc<Abort>,
    mutation: bool,
) {
    if conn.upgrade().is_some_and(|conn| conn.replied.get()) {
        return;
    }
    let already_interrupted = call_cancel.is_aborted();
    call_cancel.abort();
    if mutation
        && scope.dispatched.get()
        && !already_interrupted
        && let Some(host) = host.upgrade()
    {
        host.spawn_interrupt("disconnected input cleanup failed");
    }
}

async fn run_control_connection(
    path: &std::path::Path,
    session_id: &str,
    socket: &ControlSocket,
    begun: oneshot::Sender<Result<(), String>>,
) {
    use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
    let connect = async {
        let mut stream = tokio::net::UnixStream::connect(path)
            .await
            .map_err(|e| e.to_string())?;
        let begin = format!(
            "{}\n",
            json!({ "method": "session_begin", "session_id": session_id })
        );
        stream
            .write_all(begin.as_bytes())
            .await
            .map_err(|e| e.to_string())?;
        let mut reader = BufReader::new(stream);
        let mut line = Vec::new();
        reader
            .read_until(b'\n', &mut line)
            .await
            .map_err(|e| e.to_string())?;
        if line.pop() != Some(b'\n') {
            return Err("session_begin connection closed".to_string());
        }
        let reply: Value = serde_json::from_slice(&line).map_err(|e| e.to_string())?;
        if reply.get("ok") != Some(&Value::Bool(true)) {
            return Err(reply
                .get("error")
                .and_then(Value::as_str)
                .unwrap_or("session_begin refused.")
                .to_string());
        }
        Ok(reader)
    };
    let begun_reader = tokio::select! {
        result = connect => result,
        () = socket.shutdown.wait() => Err("session_begin connection closed".to_string()),
        () = tokio::time::sleep(CONTROL_SESSION_TIMEOUT) => Err("session_begin timed out".to_string()),
    };
    let mut reader = match begun_reader {
        Ok(reader) => {
            let _ = begun.send(Ok(()));
            reader
        }
        Err(error) => {
            let _ = begun.send(Err(error));
            return;
        }
    };
    let mut sink = [0u8; 4096];
    loop {
        tokio::select! {
            read = reader.read(&mut sink) => match read {
                Ok(0) | Err(_) => return,
                Ok(_) => {}
            },
            () = socket.shutdown.wait() => return,
        }
    }
}

fn computer_action_record(
    call: &ComputerHostCall,
    reply: Option<&CuaReply>,
    elapsed: Duration,
) -> Value {
    let task = reply.and(call.task.as_ref());
    let args = call.args.as_object();
    let structured = reply.and_then(structured);
    let diagnostics =
        structured.and_then(|s| parse_cua_action_diagnostics(&Value::Object(s.clone())));
    let effect = structured
        .and_then(|s| s.get("effect").cloned())
        .or_else(|| reply.and_then(|r| r.effect).map(|e| json!(e)));
    let code = structured.and_then(|s| {
        s.get("code")
            .or_else(|| s.get("refusal").and_then(|r| r.get("code")))
            .and_then(Value::as_str)
    });
    let mut record = Map::new();
    record.insert("event".into(), json!("computer_action"));
    record.insert("ts".into(), json!(now_iso()));
    if let Some(task) = task {
        record.insert("thread".into(), json!(task.thread_id));
        if let Some(turn) = &task.turn_id {
            record.insert("turn".into(), json!(turn));
        }
    }
    record.insert("tool".into(), json!(call.name));
    record.insert("layer".into(), json!("driver-host"));
    if let Some(code) = code.filter(|code| LOGGABLE_CUA_CODES.contains(code)) {
        record.insert("code".into(), json!(code));
    }
    if let Some(pid) = safe_native_id(args.and_then(|a| a.get("pid"))) {
        record.insert("pid".into(), json!(pid));
    }
    if let Some(window_id) = safe_native_id(args.and_then(|a| a.get("window_id"))) {
        record.insert("windowId".into(), json!(window_id));
    }
    let effect = effect
        .as_ref()
        .and_then(Value::as_str)
        .filter(|e| {
            matches!(
                *e,
                "refused" | "not-dispatched" | "dispatched-unknown" | "verified"
            )
        })
        .unwrap_or("unknown");
    record.insert("effect".into(), json!(effect));
    record.insert(
        "failed".into(),
        json!(reply.is_none_or(|r| !r.ok || is_error(r))),
    );
    if let Some(diagnostics) = diagnostics {
        let reason = cua_action_diagnostic_message(&diagnostics);
        record.insert("diagnostics".into(), json!(diagnostics));
        if let Some(reason) = reason {
            record.insert("reason".into(), json!(reason));
        }
    }
    record.insert("ms".into(), json!(elapsed.as_millis() as u64));
    Value::Object(record)
}

enum Command {
    Request(ComputerHostCall, oneshot::Sender<CuaReply>),
    Cancel(String),
    Stop(oneshot::Sender<Result<(), String>>),
    Suspend(oneshot::Sender<Result<(), String>>),
    Resume,
    PauseDesktop(String, oneshot::Sender<Result<(), String>>),
    ResumeDesktop(String),
    EmergencyStopInput(oneshot::Sender<bool>),
    PhysicalInput(PhysicalComputerInput, oneshot::Sender<bool>),
    InputMonitorStateChanged(ComputerInputMonitorState),
    StopTaskByUser(CuaComputerTask, oneshot::Sender<Result<(), String>>),
    SetCursorStyle(Option<CuaCursorStyle>, oneshot::Sender<()>),
    IsInputMonitorRequested(oneshot::Sender<bool>),
    Dispose(oneshot::Sender<Result<(), String>>),
}

fn reply_when<T: Clone + 'static>(
    future: Shared<LocalBoxFuture<'static, T>>,
    reply: oneshot::Sender<T>,
) {
    tokio::task::spawn_local(async move {
        let _ = reply.send(future.await);
    });
}

impl Host {
    fn dispatch(self: &Rc<Self>, command: Command) {
        match command {
            Command::Request(call, reply) => self.accept(call, reply),
            Command::Cancel(correlation_id) => {
                let conn = self
                    .state
                    .borrow()
                    .connections
                    .get(&correlation_id)
                    .cloned();
                if let Some(conn) = conn {
                    conn.close();
                }
            }
            Command::Stop(reply) => reply_when(self.stop(), reply),
            Command::Suspend(reply) => reply_when(self.suspend(), reply),
            Command::Resume => self.resume(),
            Command::PauseDesktop(reason, reply) => reply_when(self.pause_desktop(reason), reply),
            Command::ResumeDesktop(reason) => self.resume_desktop(&reason),
            Command::EmergencyStopInput(reply) => {
                let _ = reply.send(self.emergency_stop_input());
            }
            Command::PhysicalInput(event, reply) => {
                let _ = reply.send(self.physical_input(event));
            }
            Command::InputMonitorStateChanged(state) => self.input_monitor_state_changed(state),
            Command::StopTaskByUser(task, reply) => {
                let host = self.clone();
                tokio::task::spawn_local(async move {
                    let _ = reply.send(host.stop_task_input(task).await.map(|_| ()));
                });
            }
            Command::SetCursorStyle(style, reply) => {
                reply_when(self.set_cursor_style(style), reply)
            }
            Command::IsInputMonitorRequested(reply) => {
                let _ = reply.send(self.state.borrow().input_monitor_requested);
            }
            Command::Dispose(reply) => {
                let disposing = eager(self.clone().dispose());
                reply_when(disposing, reply);
            }
        }
    }
}

fn run_host(
    options: CuaDriverHostOptions,
    directory: PathBuf,
    mut commands: mpsc::UnboundedReceiver<Command>,
) {
    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(error) => {
            log(&format!("computer host runtime failed to start: {error}"));
            return;
        }
    };
    let local = tokio::task::LocalSet::new();
    local.block_on(&runtime, async move {
        let host = Rc::new(Host {
            options,
            directory,
            state: RefCell::new(State::new()),
            next_id: Cell::new(0),
        });
        while let Some(command) = commands.recv().await {
            host.dispatch(command);
            tokio::task::yield_now().await;
        }
        let disposed = host.state.borrow().disposed;
        if !disposed {
            let _ = host.clone().dispose().await;
        }
    });
}

#[derive(Clone)]
pub struct CuaDriverHost {
    commands: mpsc::UnboundedSender<Command>,
}

type Answer<T> = futures::future::BoxFuture<'static, T>;

fn closed() -> String {
    "Computer host is closed.".into()
}

impl CuaDriverHost {
    pub fn new(options: CuaDriverHostOptions) -> std::io::Result<Self> {
        // sun_path is 104 bytes on macOS; a worktree data dir overflows it, so sockets live in a short temp dir.
        let directory = create_cua_runtime_directory(&std::env::temp_dir())?;
        let (commands, received) = mpsc::unbounded_channel();
        let dispatch = tracing::dispatcher::get_default(Clone::clone);
        std::thread::Builder::new()
            .name("zeron-cua-host".into())
            .spawn(move || {
                tracing::dispatcher::with_default(&dispatch, || {
                    run_host(options, directory, received)
                })
            })?;
        Ok(Self { commands })
    }

    fn ask<T: Send + 'static>(
        &self,
        command: impl FnOnce(oneshot::Sender<T>) -> Command,
        fallback: impl FnOnce() -> T + Send + 'static,
    ) -> Answer<T> {
        let (reply, answer) = oneshot::channel();
        let sent = self.commands.send(command(reply)).is_ok();
        Box::pin(async move {
            match sent {
                true => answer.await.unwrap_or_else(|_| fallback()),
                false => fallback(),
            }
        })
    }

    pub fn submit(&self, call: ComputerHostCall) -> Answer<CuaReply> {
        self.ask(
            |reply| Command::Request(call, reply),
            || not_dispatched(closed()),
        )
    }

    pub fn stop(&self) -> Answer<Result<(), String>> {
        self.ask(Command::Stop, || Err(closed()))
    }

    pub fn suspend(&self) -> Answer<Result<(), String>> {
        self.ask(Command::Suspend, || Err(closed()))
    }

    pub fn resume(&self) {
        let _ = self.commands.send(Command::Resume);
    }

    pub fn pause_desktop(&self, reason: &str) -> Answer<Result<(), String>> {
        let reason = reason.to_string();
        self.ask(
            |reply| Command::PauseDesktop(reason, reply),
            || Err(closed()),
        )
    }

    pub fn resume_desktop(&self, reason: &str) {
        let _ = self
            .commands
            .send(Command::ResumeDesktop(reason.to_string()));
    }

    pub fn emergency_stop_input(&self) -> Answer<bool> {
        self.ask(Command::EmergencyStopInput, || false)
    }

    pub fn physical_input(&self, event: PhysicalComputerInput) -> Answer<bool> {
        self.ask(|reply| Command::PhysicalInput(event, reply), || false)
    }

    pub fn input_monitor_state_changed(&self, state: ComputerInputMonitorState) {
        let _ = self.commands.send(Command::InputMonitorStateChanged(state));
    }

    pub fn stop_task_by_user(&self, task: CuaComputerTask) -> Answer<Result<(), String>> {
        self.ask(
            |reply| Command::StopTaskByUser(task, reply),
            || Err(closed()),
        )
    }

    pub fn set_cursor_style(&self, style: Option<CuaCursorStyle>) -> Answer<()> {
        self.ask(|reply| Command::SetCursorStyle(style, reply), || ())
    }

    pub fn is_input_monitor_requested(&self) -> Answer<bool> {
        self.ask(Command::IsInputMonitorRequested, || false)
    }

    pub fn dispose(&self) -> Answer<Result<(), String>> {
        self.ask(Command::Dispose, || Err(closed()))
    }
}

impl crate::computer::desktop_lifecycle::ComputerDesktopLifecycleHost for CuaDriverHost {
    fn pause_desktop(
        &self,
        reason: &'static str,
    ) -> futures::future::BoxFuture<'static, Result<(), String>> {
        CuaDriverHost::pause_desktop(self, reason)
    }

    fn resume_desktop(&self, reason: &'static str) {
        CuaDriverHost::resume_desktop(self, reason);
    }
}

#[async_trait]
impl ComputerHost for CuaDriverHost {
    async fn request(&self, call: ComputerHostCall) -> CuaReply {
        self.submit(call).await
    }

    fn cancel(&self, correlation_id: &str) {
        let _ = self
            .commands
            .send(Command::Cancel(correlation_id.to_string()));
    }
}
