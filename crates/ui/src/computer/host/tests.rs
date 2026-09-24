mod fake;

use std::os::unix::fs::PermissionsExt;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use zeron_proto::computer::{
    ComputerHostCall, ComputerHostMethod, CuaComputerTask, CuaEffect, CuaReply,
};

use super::{
    CuaDriverHost, CuaDriverHostOptions, ESCAPE_INPUT_COOLDOWN, HostFuture, HostPermissions,
    PermissionCheck,
};
use fake::{FAKE_ARGV_ENV, FAKE_DIR_ENV, FAKE_TEST_PATH};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub(super) struct Driver {
    pub unpatched: bool,
    pub reported_revision: Option<u64>,
    pub metadata_pid_offset: i64,
    pub metadata_delay_ms: u64,
    pub metadata_first_delay_ms: Option<u64>,
    pub metadata_unanswered: bool,
    pub fail_action: bool,
    pub action_result: Option<Value>,
    pub cursor_unavailable: bool,
    pub log_cursor_state: bool,
    pub cursor_enable_failures: u64,
    pub cursor_hide_failures: u64,
    pub crash: bool,
    pub session_death_once: bool,
    pub session_death_transport: bool,
    pub log_sessions: bool,
    pub browser_refusal: bool,
    pub browser_hang: bool,
    pub browser_cleanup_unconfirmed: bool,
    pub browser_observations: bool,
    pub input_delay_ms: Option<u64>,
    pub delay_browser_observation: bool,
    pub delay_observation: bool,
    pub delay_list_windows_ms: Option<u64>,
    pub hang_session: bool,
    pub drop_cancel: bool,
    pub cleanup: Option<String>,
    pub interrupt_cleanup: Option<String>,
    pub list_windows: Option<Vec<Value>>,
    pub reported_version: Option<String>,
    pub not_embedded: bool,
    pub log_socket: bool,
    pub ignore_termination: bool,
    pub ignore_stdin_eof: bool,
}

#[derive(Debug, Clone, Deserialize)]
struct Event {
    event: String,
    pid: i32,
    time: u64,
}

struct Pending {
    id: String,
    handle: tokio::task::JoinHandle<CuaReply>,
}

impl Pending {
    async fn reply(self) -> CuaReply {
        self.handle.await.unwrap()
    }
}

struct Fixture {
    host: CuaDriverHost,
    dir: tempfile::TempDir,
    next: AtomicU64,
}

fn fixture(driver: Driver) -> Fixture {
    fixture_with(driver, |_| {})
}

fn fixture_with(driver: Driver, configure: impl FnOnce(&mut CuaDriverHostOptions)) -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("options.json"),
        serde_json::to_vec(&driver).unwrap(),
    )
    .unwrap();
    let binary = dir.path().join("driver");
    let exe = std::env::current_exe().unwrap();
    std::fs::write(
        &binary,
        format!(
            "#!/bin/sh\nexport {FAKE_DIR_ENV}='{}'\nexport {FAKE_ARGV_ENV}=\"$*\"\nexec '{}' --exact {FAKE_TEST_PATH} --nocapture --test-threads=1 -q\n",
            dir.path().display(),
            exe.display()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o755)).unwrap();
    let mut options = CuaDriverHostOptions::new(
        binary,
        "fixture".into(),
        Box::new(|| Box::pin(async { Ok(()) })),
    );
    options.warm_on_first_touch = false;
    configure(&mut options);
    Fixture {
        host: CuaDriverHost::new(options).unwrap(),
        dir,
        next: AtomicU64::new(0),
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let disposing = self.host.dispose();
        let (done, finished) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = done.send(futures::executor::block_on(disposing));
        });
        let _ = finished.recv_timeout(Duration::from_secs(10));
        for event in self.events() {
            if event.event == "start" {
                unsafe { libc::kill(event.pid, libc::SIGKILL) };
            }
        }
    }
}

impl Fixture {
    fn id(&self) -> String {
        format!("c{}", self.next.fetch_add(1, Ordering::SeqCst))
    }

    fn with_id(&self, mut call: ComputerHostCall) -> ComputerHostCall {
        if call.correlation_id.is_empty() {
            call.correlation_id = self.id();
        }
        call
    }

    async fn send(&self, call: ComputerHostCall) -> CuaReply {
        self.host.submit(self.with_id(call)).await
    }

    fn spawn(&self, call: ComputerHostCall) -> Pending {
        let call = self.with_id(call);
        let id = call.correlation_id.clone();
        Pending {
            id,
            handle: tokio::spawn(self.host.submit(call)),
        }
    }

    fn cancel(&self, id: &str) {
        use zeron_rpc::ComputerHost;
        self.host.cancel(id);
    }

    fn events(&self) -> Vec<Event> {
        std::fs::read_to_string(self.dir.path().join("events.jsonl"))
            .unwrap_or_default()
            .lines()
            .filter_map(|line| serde_json::from_str(line).ok())
            .collect()
    }

    fn names(&self) -> Vec<String> {
        self.events().into_iter().map(|e| e.event).collect()
    }

    fn count(&self, name: &str) -> usize {
        self.names().iter().filter(|e| *e == name).count()
    }

    fn has(&self, name: &str) -> bool {
        self.count(name) > 0
    }

    fn starts(&self) -> Vec<Event> {
        self.events()
            .into_iter()
            .filter(|e| e.event == "start")
            .collect()
    }

    fn pid_events(&self, pid: i32) -> Vec<String> {
        self.events()
            .into_iter()
            .filter(|e| e.pid == pid)
            .map(|e| e.event)
            .collect()
    }

    fn style_payloads(&self) -> Vec<Value> {
        self.names()
            .iter()
            .filter_map(|e| e.strip_prefix("style:"))
            .map(|payload| serde_json::from_str(payload).unwrap())
            .collect()
    }

    async fn wait_for(&self, name: &str) -> Vec<Event> {
        for _ in 0..300 {
            let events = self.events();
            if events.iter().any(|e| e.event == name) {
                return events;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        panic!(
            "timed out waiting for driver event {name}; saw {:?}",
            self.names()
        );
    }
}

fn call(name: &str) -> ComputerHostCall {
    ComputerHostCall {
        correlation_id: String::new(),
        method: ComputerHostMethod::Call,
        name: Some(name.into()),
        args: Value::Null,
        task: None,
        delivery_mode: None,
        model_observation: None,
    }
}

fn method(method: ComputerHostMethod) -> ComputerHostCall {
    ComputerHostCall {
        name: None,
        method,
        ..call("")
    }
}

trait Build {
    fn args(self, args: Value) -> Self;
    fn task(self, task: &CuaComputerTask) -> Self;
    fn observe(self, model_observation: bool) -> Self;
}

impl Build for ComputerHostCall {
    fn args(self, args: Value) -> Self {
        Self { args, ..self }
    }

    fn task(self, task: &CuaComputerTask) -> Self {
        Self {
            task: Some(task.clone()),
            ..self
        }
    }

    fn observe(self, model_observation: bool) -> Self {
        Self {
            model_observation: Some(model_observation),
            ..self
        }
    }
}

fn task(thread: &str, turn: Option<&str>) -> CuaComputerTask {
    CuaComputerTask {
        thread_id: thread.into(),
        turn_id: turn.map(Into::into),
        label: None,
    }
}

fn labelled(thread: &str, label: &str) -> CuaComputerTask {
    CuaComputerTask {
        label: Some(label.into()),
        ..task(thread, None)
    }
}

fn press() -> ComputerHostCall {
    call("press_key").args(json!({ "key": "enter" }))
}

fn structured(reply: &CuaReply) -> Value {
    reply
        .result
        .as_ref()
        .and_then(|r| r.structured_content.clone())
        .map(Value::Object)
        .unwrap_or(Value::Null)
}

fn code(reply: &CuaReply) -> Option<String> {
    structured(reply)
        .get("code")
        .and_then(Value::as_str)
        .map(str::to_string)
}

fn result_is_error(reply: &CuaReply) -> bool {
    reply.result.as_ref().and_then(|r| r.is_error) == Some(true)
}

fn extra(reply: &CuaReply, key: &str) -> Option<Value> {
    reply
        .result
        .as_ref()
        .and_then(|r| r.extra.get(key).cloned())
}

#[track_caller]
fn assert_not_dispatched(reply: &CuaReply) {
    assert!(!reply.ok, "{reply:?}");
    assert_eq!(reply.effect, Some(CuaEffect::NotDispatched), "{reply:?}");
}

#[track_caller]
fn assert_paused(reply: &CuaReply) {
    assert!(result_is_error(reply), "{reply:?}");
    assert_eq!(
        code(reply).as_deref(),
        Some("computer_input_paused"),
        "{reply:?}"
    );
}

#[track_caller]
fn assert_clean_ok(reply: &CuaReply) {
    assert!(reply.ok, "{reply:?}");
    assert!(!result_is_error(reply), "{reply:?}");
}

fn permissions(granted: bool) -> HostPermissions {
    HostPermissions {
        accessibility: granted,
        screen_recording: granted,
        input_monitoring: None,
    }
}

#[derive(Clone)]
struct Deferred<T: Clone>(Arc<tokio::sync::watch::Sender<Option<T>>>);

impl<T: Clone + Send + Sync + 'static> Deferred<T> {
    fn new() -> Self {
        Self(Arc::new(tokio::sync::watch::Sender::new(None)))
    }

    fn resolve(&self, value: T) {
        self.0.send_replace(Some(value));
    }

    async fn wait(&self) -> T {
        let mut receiver = self.0.subscribe();
        receiver
            .wait_for(Option::is_some)
            .await
            .unwrap()
            .clone()
            .unwrap()
    }
}

fn fixed_permissions(granted: Arc<Mutex<HostPermissions>>) -> PermissionCheck {
    Box::new(move |_| {
        let permissions = *granted.lock().unwrap();
        Box::pin(async move { Ok(permissions) })
    })
}

fn pending_permissions(
    entered: Deferred<()>,
    pending: Deferred<HostPermissions>,
) -> PermissionCheck {
    Box::new(move |_| {
        entered.resolve(());
        let pending = pending.clone();
        Box::pin(async move { Ok(pending.wait().await) })
    })
}

fn counting_release(
    count: Arc<AtomicUsize>,
) -> Box<dyn Fn() -> HostFuture<Result<(), String>> + Send> {
    Box::new(move || {
        count.fetch_add(1, Ordering::SeqCst);
        Box::pin(async { Ok(()) })
    })
}

fn pid_alive(pid: i32) -> bool {
    unsafe { libc::kill(pid, 0) == 0 }
}

async fn sleep_ms(ms: u64) {
    tokio::time::sleep(Duration::from_millis(ms)).await;
}

async fn past_cooldown() {
    tokio::time::sleep(ESCAPE_INPUT_COOLDOWN + Duration::from_millis(50)).await;
}

#[derive(Clone, Default)]
struct Logs(Arc<Mutex<Vec<u8>>>);

impl Logs {
    fn text(&self) -> String {
        String::from_utf8_lossy(&self.0.lock().unwrap()).into_owned()
    }

    async fn wait_for(&self, needle: &str) {
        for _ in 0..300 {
            if self.text().contains(needle) {
                return;
            }
            sleep_ms(10).await;
        }
        panic!("log never contained {needle}: {}", self.text());
    }
}

struct LogWriter(Arc<Mutex<Vec<u8>>>);

impl std::io::Write for LogWriter {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

fn logged_fixture(
    driver: Driver,
    configure: impl FnOnce(&mut CuaDriverHostOptions),
) -> (Fixture, Logs) {
    let logs = Logs::default();
    let sink = logs.0.clone();
    let subscriber = tracing_subscriber::fmt()
        .with_ansi(false)
        .with_writer(move || LogWriter(sink.clone()))
        .finish();
    let f = tracing::subscriber::with_default(subscriber, || fixture_with(driver, configure));
    (f, logs)
}

mod browser;
mod cursor_identity;
mod cursor_style;
mod escape;
mod frame_tap;
mod retirement;
mod risk;
mod self_guard;
mod shield;
mod task_stop;
mod warm;
