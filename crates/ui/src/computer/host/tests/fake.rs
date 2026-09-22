use std::cell::{Cell, RefCell};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Duration;

use serde_json::{Map, Value, json};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;
use tokio::net::unix::OwnedWriteHalf;
use tokio::sync::oneshot;
use zeron_proto::computer::{CUA_DRIVER_VERSION, CUA_NATIVE_REVISION};

use super::Driver;

pub(super) const FAKE_DIR_ENV: &str = "ZERON_FAKE_CUA_DIR";
pub(super) const FAKE_ARGV_ENV: &str = "ZERON_FAKE_CUA_ARGV";
pub(super) const FAKE_TEST_PATH: &str = "computer::host::tests::fake::fake_cua_driver_process";

#[test]
fn fake_cua_driver_process() {
    let Some(dir) = std::env::var_os(FAKE_DIR_ENV) else {
        return;
    };
    run(PathBuf::from(dir));
}

pub(super) fn write_event(log: &Path, event: &str) {
    let time = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or_default();
    let line = format!(
        "{}\n",
        json!({ "event": event, "pid": std::process::id(), "time": time })
    );
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(log)
    {
        let _ = file.write_all(line.as_bytes());
    }
}

struct Fake {
    options: Driver,
    log: PathBuf,
    death_flag: PathBuf,
    input_epoch: Cell<u64>,
    interruptions: Cell<u64>,
    browser_cleanup_pending: Cell<bool>,
    cursor_enables: Cell<u64>,
    cursor_hides: Cell<u64>,
    action: RefCell<Option<oneshot::Sender<String>>>,
    retiring: Cell<bool>,
    metadata_requests: Cell<u64>,
}

fn run(dir: PathBuf) -> ! {
    let options: Driver =
        serde_json::from_slice(&std::fs::read(dir.join("options.json")).unwrap()).unwrap();
    let log = dir.join("events.jsonl");
    let argv: Vec<String> = std::env::var(FAKE_ARGV_ENV)
        .unwrap_or_default()
        .split_whitespace()
        .map(str::to_string)
        .collect();
    write_event(&log, "start");
    let has = |flag: &str| argv.iter().any(|arg| arg == flag);
    let after = |flag: &str| {
        argv.iter()
            .position(|arg| arg == flag)
            .and_then(|i| argv.get(i + 1))
            .cloned()
    };
    if !options.unpatched
        && (!has("--compact-cursor") || after("--idle-hide-ms").as_deref() != Some("60000"))
    {
        std::process::exit(1);
    }
    if options.unpatched && (has("--compact-cursor") || has("--idle-hide-ms")) {
        std::process::exit(1);
    }
    let socket = after("--socket").unwrap();
    if options.log_socket {
        write_event(&log, &format!("socket:{socket}"));
    }
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let local = tokio::task::LocalSet::new();
    local.block_on(&runtime, async move {
        let fake = Rc::new(Fake {
            options,
            log,
            death_flag: dir.join("session-died"),
            input_epoch: Cell::new(0),
            interruptions: Cell::new(0),
            browser_cleanup_pending: Cell::new(false),
            cursor_enables: Cell::new(0),
            cursor_hides: Cell::new(0),
            action: RefCell::new(None),
            retiring: Cell::new(false),
            metadata_requests: Cell::new(0),
        });
        let listener = tokio::net::UnixListener::bind(&socket).unwrap();
        let on_eof = fake.clone();
        tokio::task::spawn_local(async move {
            let mut stdin = tokio::io::stdin();
            let mut sink = [0u8; 256];
            while matches!(stdin.read(&mut sink).await, Ok(n) if n > 0) {}
            if !on_eof.options.ignore_termination && !on_eof.options.ignore_stdin_eof {
                on_eof.retire();
            }
        });
        let on_term = fake.clone();
        tokio::task::spawn_local(async move {
            let mut term =
                tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()).unwrap();
            while term.recv().await.is_some() {
                if !on_term.options.ignore_termination {
                    on_term.retire();
                }
            }
        });
        loop {
            let Ok((stream, _)) = listener.accept().await else {
                continue;
            };
            tokio::task::spawn_local(fake.clone().serve(stream));
        }
    })
}

fn js_string(value: Option<&Value>) -> String {
    match value {
        None => "undefined".into(),
        Some(Value::String(s)) => s.clone(),
        Some(Value::Number(n)) => match n.as_f64() {
            Some(f) if f.fract() == 0.0 && f.abs() < 1e15 => format!("{}", f as i64),
            _ => n.to_string(),
        },
        Some(other) => other.to_string(),
    }
}

fn truthy(value: Option<&Value>) -> bool {
    match value {
        None | Some(Value::Null) | Some(Value::Bool(false)) => false,
        Some(Value::Number(n)) => n.as_f64() != Some(0.0),
        Some(Value::String(s)) => !s.is_empty(),
        _ => true,
    }
}

const CONTROL_NAMES: &[&str] = &[
    "start_session",
    "set_agent_cursor_motion",
    "set_agent_cursor_style",
    "set_agent_cursor_enabled",
    "get_agent_cursor_state",
];

impl Fake {
    fn write(&self, event: &str) {
        write_event(&self.log, event);
    }

    fn retire(self: &Rc<Self>) {
        if self.retiring.replace(true) {
            return;
        }
        self.write("retiring");
        let fake = self.clone();
        tokio::task::spawn_local(async move {
            tokio::time::sleep(Duration::from_millis(150)).await;
            fake.write("exit");
            std::process::exit(0);
        });
    }

    fn release_action(&self, message: &str) {
        if let Some(action) = self.action.borrow_mut().take() {
            self.write("release");
            let _ = action.send(message.to_string());
        }
    }

    async fn hold_action(&self, write: &mut OwnedWriteHalf, dispatched: &str, effect: &str) {
        self.write(dispatched);
        let (release, released) = oneshot::channel();
        *self.action.borrow_mut() = Some(release);
        let delay = Duration::from_millis(self.options.input_delay_ms.unwrap_or(10_000));
        tokio::select! {
            () = tokio::time::sleep(delay) => {
                self.write(effect);
                self.action.borrow_mut().take();
                reply(write, json!({})).await;
            }
            Ok(message) = released => {
                let _ = write.write_all(format!("{message}\n").as_bytes()).await;
                let _ = write.shutdown().await;
            }
        }
    }

    fn epoch_refused(&self, request: &Value) -> bool {
        request.get("expected_input_epoch").and_then(Value::as_u64) != Some(self.input_epoch.get())
    }

    async fn serve(self: Rc<Self>, stream: UnixStream) {
        let (read, mut write) = stream.into_split();
        let mut reader = BufReader::new(read);
        let mut line = String::new();
        if !matches!(reader.read_line(&mut line).await, Ok(n) if n > 0) {
            return;
        }
        let Ok(request) = serde_json::from_str::<Value>(&line) else {
            return;
        };
        let options = &self.options;
        let method = request.get("method").and_then(Value::as_str).unwrap_or("");
        let name = request.get("name").and_then(Value::as_str).unwrap_or("");
        let args = request.get("args").cloned().unwrap_or(Value::Null);
        let arg = |key: &str| args.get(key);
        let session = arg("session").and_then(Value::as_str);
        let pid = std::process::id();
        if options.log_sessions
            && method == "call"
            && let Some(session) = session
        {
            self.write(&format!("session:{session}:{name}"));
        }
        let death_candidate = method == "call"
            && truthy(arg("session"))
            && !CONTROL_NAMES.contains(&name)
            && !self.death_flag.exists();
        if method == "metadata" {
            let traced = options.metadata_unanswered || options.metadata_first_delay_ms.is_some();
            if traced {
                self.write("metadata");
            }
            if options.metadata_unanswered {
                return;
            }
            let first = self
                .metadata_requests
                .replace(self.metadata_requests.get() + 1)
                == 0;
            let delay = match options.metadata_first_delay_ms {
                Some(delay) if first => delay,
                _ => options.metadata_delay_ms,
            };
            tokio::time::sleep(Duration::from_millis(delay)).await;
            let revision = options
                .reported_revision
                .or((!options.unpatched).then_some(CUA_NATIVE_REVISION));
            let mut result = json!({
                "driver_version": options.reported_version.as_deref().unwrap_or(CUA_DRIVER_VERSION),
                "embedded": !options.not_embedded,
                "pid": pid as i64 + options.metadata_pid_offset,
            });
            if let Some(revision) = revision {
                result["synara_native_revision"] = json!(revision);
            }
            reply(&mut write, result).await;
        } else if method == "interrupt_input" {
            self.write("interrupt");
            let cleanup = options.interrupt_cleanup.as_deref();
            if arg("expected_pid").and_then(Value::as_u64) != Some(u64::from(pid)) {
                std::process::exit(2);
            }
            if cleanup != Some("stale-epoch") {
                self.input_epoch.set(self.input_epoch.get() + 1);
            }
            self.interruptions.set(self.interruptions.get() + 1);
            self.release_action(r#"{"ok":false,"error":"interrupted"}"#);
            let incomplete = self.browser_cleanup_pending.get()
                || cleanup == Some("incomplete")
                || (cleanup == Some("once-incomplete") && self.interruptions.get() == 1);
            tokio::time::sleep(Duration::from_millis(30)).await;
            self.write("interrupt-ack");
            let mut result = json!({
                "pid": pid + u32::from(cleanup == Some("wrong-pid")),
                "input_interrupted": true,
                "cleanup_complete": !incomplete,
                "pending_input": if incomplete { 1 } else { 0 },
                "input_epoch": self.input_epoch.get(),
            });
            if cleanup != Some("missing-admission") {
                result["input_admission_open"] = json!(!incomplete);
            }
            reply(&mut write, result).await;
        } else if method == "cancel_input" {
            self.write("cancel");
            if options.drop_cancel {
                return;
            }
            if arg("expected_pid").and_then(Value::as_u64) != Some(u64::from(pid)) {
                std::process::exit(2);
            }
            self.release_action(r#"{"ok":false,"error":"cancelled"}"#);
            tokio::time::sleep(Duration::from_millis(30)).await;
            self.write("cleanup-ack");
            let cleanup = options.cleanup.as_deref();
            let incomplete = self.browser_cleanup_pending.get() || cleanup == Some("incomplete");
            let mut result = json!({
                "pid": pid + u32::from(cleanup == Some("wrong-pid")),
                "cleanup_complete": !incomplete,
                "pending_input": if incomplete { 1 } else { 0 },
            });
            if cleanup != Some("missing-admission") {
                result["input_admission_closed"] = json!(true);
            }
            reply(&mut write, result).await;
        } else if options.session_death_once && death_candidate {
            let _ = std::fs::write(&self.death_flag, "1");
            let session = session.unwrap_or_default();
            reply(
                &mut write,
                json!({
                    "isError": true,
                    "content": [{ "type": "text", "text": format!("session '{session}' has ended; tool call '{name}' was rejected. Call start_session with this id to revive it before issuing further actions, or use a new session id.") }],
                    "structuredContent": { "effect": "not-dispatched" },
                }),
            )
            .await;
        } else if options.session_death_transport && death_candidate {
            let _ = std::fs::write(&self.death_flag, "1");
            let session = session.unwrap_or_default();
            let message = json!({
                "ok": false,
                "error": format!("session '{session}' has ended; tool call '{name}' was rejected. Call start_session with this id to revive it before issuing further actions, or use a new session id."),
                "effect": "not-dispatched",
            });
            let _ = write.write_all(format!("{message}\n").as_bytes()).await;
        } else if name == "type_text" {
            if !options.unpatched && self.epoch_refused(&request) {
                return refused(&mut write).await;
            }
            if options.crash {
                self.write("dispatch");
                self.write("crash");
                std::process::exit(1);
            }
            if options.fail_action {
                self.write("dispatch");
                *self.action.borrow_mut() = Some(oneshot::channel().0);
                return;
            }
            self.hold_action(&mut write, "dispatch", "effect").await;
        } else if options.hang_session && name == "start_session" && !self.death_flag.exists() {
            let _ = std::fs::write(&self.death_flag, "1");
            self.write("session-hang");
            std::future::pending::<()>().await;
        } else if name == "set_agent_cursor_motion" {
            self.write(&format!(
                "motion-{}-{}",
                js_string(arg("glide_duration_ms")),
                js_string(arg("dwell_after_click_ms"))
            ));
            reply(&mut write, json!({})).await;
        } else if name == "set_agent_cursor_style" {
            self.write(&format!("style:{args}"));
            reply(&mut write, json!({})).await;
        } else if name == "set_agent_cursor_enabled" {
            let enabled = arg("enabled") == Some(&Value::Bool(true));
            if options.log_cursor_state {
                self.write(&format!(
                    "cursor-enabled:{enabled}:{}",
                    session.unwrap_or_default()
                ));
            }
            let failed = if enabled {
                let n = self.cursor_enables.get();
                self.cursor_enables.set(n + 1);
                n < options.cursor_enable_failures
            } else {
                let n = self.cursor_hides.get();
                self.cursor_hides.set(n + 1);
                n < options.cursor_hide_failures
            };
            reply(
                &mut write,
                if failed {
                    json!({ "isError": true })
                } else {
                    json!({})
                },
            )
            .await;
        } else if name == "get_agent_cursor_state" {
            if options.log_cursor_state {
                self.write(&format!("cursor-state:{}", session.unwrap_or_default()));
            }
            let result = if options.cursor_unavailable {
                json!({ "isError": true, "content": [{ "type": "text", "text": "private overlay error" }] })
            } else {
                json!({ "structuredContent": {
                    "session": session,
                    "enabled": true,
                    "position": { "x": 10, "y": 20 },
                    "motion": { "idle_hide_ms": 60000 },
                    "overlay_ready": true,
                    "render_visible": true,
                    "overlay_scope": "main_display",
                } })
            };
            reply(&mut write, result).await;
        } else if name == "press_key" {
            if !options.unpatched && self.epoch_refused(&request) {
                return refused(&mut write).await;
            }
            self.write("key");
            self.write(&format!(
                "observation-budget-{}",
                std::env::var("SYNARA_CUA_FOREGROUND_OBSERVATION_MS").unwrap_or_default()
            ));
            reply(
                &mut write,
                options.action_result.clone().unwrap_or_else(|| json!({})),
            )
            .await;
        } else if name == "get_window_state" && !truthy(arg("empty")) {
            self.write("observe");
            let usable = arg("fixture_usable") == Some(&Value::Bool(true));
            let mut state = Map::new();
            state.insert(
                "elements".into(),
                if truthy(arg("fixture_usable")) {
                    json!([{ "role": "AXWindow" }])
                } else {
                    json!([])
                },
            );
            state.insert("window_is_on_screen".into(), json!(usable));
            state.insert("window_on_current_space".into(), json!(usable));
            if let Some(degraded) = arg("fixture_degraded") {
                state.insert("degraded".into(), degraded.clone());
            }
            state.insert(
                "screenshot_frame_valid".into(),
                json!(arg("fixture_stale") != Some(&Value::Bool(true))),
            );
            if let Some(pid) = arg("pid") {
                state.insert("pid".into(), pid.clone());
            }
            if truthy(arg("fixture_wrong_window")) {
                state.insert("window_id".into(), json!(99999));
            } else if let Some(window) = arg("window_id") {
                state.insert("window_id".into(), window.clone());
            }
            if options.delay_observation {
                tokio::time::sleep(Duration::from_millis(60)).await;
            }
            reply(&mut write, json!({ "structuredContent": state })).await;
        } else if name == "get_desktop_state" {
            reply(
                &mut write,
                json!({ "content": [{ "type": "image", "data": "fixture-image" }] }),
            )
            .await;
        } else if name == "list_windows" {
            self.write("list-windows");
            tokio::time::sleep(Duration::from_millis(
                options.delay_list_windows_ms.unwrap_or(0),
            ))
            .await;
            let windows = options.list_windows.clone().unwrap_or_default();
            reply(
                &mut write,
                json!({ "structuredContent": { "windows": windows } }),
            )
            .await;
            if options.delay_list_windows_ms.is_some() {
                self.write("list-windows-replied");
            }
        } else if method == "session_begin" {
            let session_id = js_string(request.get("session_id"));
            self.write(&format!("session-begin:{session_id}"));
            let _ = write
                .write_all(b"{\"ok\":true,\"result\":{\"session_begin\":true}}\n")
                .await;
            let mut sink = Vec::new();
            let _ = reader.read_to_end(&mut sink).await;
        } else if (name == "start_session" || name == "end_session")
            && truthy(request.get("session_id"))
        {
            self.write(&format!(
                "{name}:{}:{}",
                js_string(arg("session")),
                js_string(request.get("session_id"))
            ));
            reply(&mut write, json!({})).await;
        } else if options.log_sessions && (name == "start_session" || name == "end_session") {
            self.write(&format!(
                "open_session:{name}:{}",
                js_string(arg("session"))
            ));
            reply(&mut write, json!({})).await;
        } else if name == "start_session" || name == "end_session" {
            reply(&mut write, json!({})).await;
        } else if name.starts_with("browser_") || name == "get_browser_state" {
            let session_id = request
                .get("session_id")
                .filter(|v| truthy(Some(v)))
                .map(|v| js_string(Some(v)))
                .unwrap_or_else(|| "-".into());
            self.write(&format!(
                "browser:{name}:{}:{session_id}",
                js_string(arg("session"))
            ));
            if name.starts_with("browser_") && !options.unpatched && self.epoch_refused(&request) {
                return refused(&mut write).await;
            }
            if options.browser_observations && name == "get_browser_state" {
                if truthy(arg("target_id")) {
                    self.write("browser-observe");
                    let mut data = json!({
                        "status": "ok",
                        "mode": if truthy(arg("fixture_bind_only")) { "bind" } else { "snapshot" },
                        "target_id": if truthy(arg("fixture_wrong_target")) { json!("wrong-target") } else { arg("target_id").cloned().unwrap_or(Value::Null) },
                        "refs": [],
                    });
                    if let Some(tab) = arg("tab_id") {
                        data["tab_id"] = tab.clone();
                    }
                    if arg("snapshot_format").and_then(Value::as_str) == Some("semantic_v2") {
                        data["snapshot"] = json!({ "id": "p1" });
                    } else {
                        data["snapshot_id"] = json!("p1");
                    }
                    if options.delay_browser_observation {
                        tokio::time::sleep(Duration::from_millis(60)).await;
                    }
                    reply(&mut write, json!({ "structuredContent": data })).await;
                } else {
                    let target_id = if truthy(arg("fixture_target_id")) {
                        js_string(arg("fixture_target_id"))
                    } else {
                        format!(
                            "target-{}-{}",
                            js_string(arg("pid")),
                            js_string(arg("window_id"))
                        )
                    };
                    reply(
                        &mut write,
                        json!({ "structuredContent": {
                            "status": "ok",
                            "mode": "bind",
                            "binding_quality": "exact",
                            "mutation_allowed": true,
                            "target_id": target_id,
                            "tabs": [],
                        } }),
                    )
                    .await;
                }
            } else if options.browser_hang && name == "browser_type" {
                self.hold_action(&mut write, "browser-dispatch", "browser-effect")
                    .await;
            } else if options.browser_cleanup_unconfirmed && name == "browser_type" {
                self.browser_cleanup_pending.set(true);
                reply(
                    &mut write,
                    json!({ "isError": true, "structuredContent": { "input_cleanup_unconfirmed": true, "effect": "unverifiable" } }),
                )
                .await;
            } else if options.browser_refusal {
                reply(
                    &mut write,
                    json!({
                        "structuredContent": { "status": "refused", "refusal": { "code": "browser_requires_setup" } },
                        "content": [{ "type": "text", "text": "refused (browser_requires_setup)" }],
                    }),
                )
                .await;
            } else {
                reply(&mut write, json!({})).await;
            }
        } else {
            reply(&mut write, json!({})).await;
        }
    }
}

async fn reply(write: &mut OwnedWriteHalf, result: Value) {
    let line = format!("{}\n", json!({ "ok": true, "result": result }));
    let _ = write.write_all(line.as_bytes()).await;
    let _ = write.shutdown().await;
}

async fn refused(write: &mut OwnedWriteHalf) {
    reply(
        write,
        json!({ "isError": true, "structuredContent": { "effect": "refused", "code": "input_admission_closed" } }),
    )
    .await;
}
