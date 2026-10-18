use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use base64::Engine as _;
use serde_json::{Map, Value, json};
use zeron_proto::ui_driver::{
    DRAG_STEPS, DRIVE_ACTIONS, MAX_WAIT_MS, OBSERVE_ACTIONS, STEP_CAP_MS, UiDriverBuild,
    UiDriverDescriptor, run_dir, socket_path,
};

pub const TOOL: &str = "ui_driver";

const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);
const REPLY_SLACK: Duration = Duration::from_secs(10);

pub fn tool() -> Value {
    let field = |kind: &str, description: &str| json!({ "type": kind, "description": description });
    let end = |which: &str| {
        json!({
            "type": "object",
            "properties": {
                "x": { "type": "number" },
                "y": { "type": "number" },
                "ref": { "type": "string" },
                "select": { "type": "string" },
                "nth": { "type": "integer" }
            },
            "additionalProperties": false,
            "description": format!("Drag {which}: a window point {{\"x\": 10, \"y\": 20}}, or an element {{\"ref\": \"e12\"}} or {{\"select\": \"Button \\\"Send\\\"\", \"nth\": 0}}. The server also takes [x, y].")
        })
    };
    let properties: Map<String, Value> = [
        ("action", json!({ "type": "string", "enum": std::iter::once(&"list").chain(OBSERVE_ACTIONS).chain(DRIVE_ACTIONS).collect::<Vec<_>>() })),
        ("target", field("string", "A pid or label from `list` (exact, or a unique substring such as `dev`), a socket path, or `self`. Defaults to the app serving this chat.")),
        ("window", field("integer", "Window id from `state`. Defaults to the frontmost window.")),
        ("x", field("number", "Window point in logical pixels.")),
        ("y", field("number", "Window point in logical pixels.")),
        ("ref", field("string", "Element ref from `tree`, such as `e12`.")),
        ("select", field("string", "Selector: `#e12`, `Role`, `Role \"label\"` or `\"label\"`. An exact label match beats a substring match, which beats value, then placeholder or description.")),
        ("nth", field("integer", "Which match of `select` when several match.")),
        ("text", field("string", "Text to type, or to wait for.")),
        ("keys", json!({ "type": "array", "items": { "type": "string" }, "description": "Keystrokes, such as [\"cmd-k\"] or [\"down\", \"enter\"]. The server also takes one string of keystrokes separated by spaces." })),
        ("name", field("string", "gpui action name, for `action`.")),
        ("args", field("object", "Arguments of the gpui action, for `action`.")),
        ("rect", field("object", "Crop of a snapshot in window points: {x, y, w, h}.")),
        ("path", field("string", "Absolute path for the snapshot PNG. Defaults to a file under ~/.zeron/run/shots.")),
        ("from", end("start")),
        ("to", end("end")),
        ("steps", field("integer", "For `drag`: pointer moves between `from` and `to`. Defaults to 12.")),
        ("holdMs", field("integer", "For `drag`: how long to hold the button at `to` before releasing.")),
        ("do", field("string", "For `press`: the accessibility action to perform, such as click (default), focus, setValue, increment, decrement, expand, collapse, showContextMenu or scrollIntoView.")),
        ("value", field("string", "For `press` with `do: setValue`: the new value.")),
        ("full", field("boolean", "For `tree`: show every node, without hoisting unnamed containers.")),
        ("during", field("object", "For `frames` and `track` (Dev builds only): a drive request such as {\"action\": \"click\", \"select\": \"Button \\\"Settings\\\"\"} that runs once recording has started.")),
        ("jumpPx", field("number", "For `track`: a per-frame move larger than this counts as a jump. Defaults to 40.")),
        ("dx", field("number", "Horizontal scroll in pixels.")),
        ("dy", field("number", "Vertical scroll in pixels.")),
        ("trackpad", field("boolean", "Scroll as a trackpad instead of a mouse wheel.")),
        ("button", json!({ "type": "string", "enum": ["left", "right", "middle"] })),
        ("count", field("integer", "Clicks; 2 double-clicks.")),
        ("modifiers", json!({ "type": "array", "items": { "type": "string", "enum": ["cmd", "shift", "alt", "ctrl"] } })),
        ("for", json!({ "type": "string", "enum": ["idle", "frame", "ms", "text", "select", "gone"], "description": "What `wait` waits for." })),
        ("quietMs", field("integer", "For `wait` idle: how long no frame may draw.")),
        ("timeoutMs", field("integer", "Longest wait.")),
        ("ms", field("integer", "Duration of `wait` ms; the window of `frames` (default 1000, max 10000) and `track` (default 800, max 5000).")),
        ("query", field("string", "Keep only `tree` lines that match this text.")),
        ("root", field("string", "Ref the `tree` starts from.")),
        ("depth", field("integer", "Levels of `tree` to print.")),
        ("json", field("boolean", "`tree` as JSON instead of text lines.")),
        ("inline", field("boolean", "For `snapshot`: false returns only the path, no image. Defaults to true.")),
        ("theme", json!({ "type": "string", "enum": ["light", "dark", "system"], "description": "For `set`: the appearance mode, as Settings > Appearance sets it (saved to the app's settings)." })),
        ("size", json!({ "type": "object", "properties": { "w": { "type": "number" }, "h": { "type": "number" } }, "required": ["w", "h"], "additionalProperties": false, "description": "For `set`: window content size {w, h} in points. The server also takes [w, h]." })),
        ("motionScale", field("number", "For `set`: animation duration multiplier, 1 is normal, 5 is five times slower. Applies to animations started afterwards; loaders, pulses and disclosure rows ignore it.")),
    ]
    .into_iter()
    .map(|(name, schema)| (name.to_owned(), schema))
    .collect();
    json!({
        "name": TOOL,
        "description": "Inspect and drive a running Zeron app from inside it: read its UI as text, act on elements, wait for it, and snapshot it. `list` shows the running apps (pid, label, build, ipcPort, input). Omit `target` for the app serving this chat; a release build is observe-only and refuses drive actions. To drive, pass a Zeron Dev bundle's label or pid from `list` as `target`. Prefer `tree` with `query` over snapshots: it is cheap text, and each line carries a `ref`. Act on an element by `ref` from `tree`, or by `select` (`Button \"New chat\"`, or a bare `\"label substring\"`, plus `nth`); fall back to `x`,`y` window points. Drive actions (click, move, drag, scroll, key, type, press, action, set) reply after the next frame is drawn; an action addressed by `ref` or `select` first waits up to 1 s for the element to hold still across two frames, and replies `unsettled: true` if it never did. `press` performs an accessibility action given in `do` (click, focus, setValue with `value`, expand, ...) without moving the pointer. `frames` measures drawn frames for `ms` (fps, interval percentiles, long frames, draw time); `track` samples one element's bounds every drawn frame and reports jumps and when it settled. Give either one a `during` drive request to measure the transition it starts. `set` changes theme, window size or motion scale at runtime. Use `wait` (`for`: idle, frame, ms, text, select, gone) instead of sleeping. Take a `snapshot` only for a visual check: it returns the PNG, or only its path with `inline: false`.",
        "inputSchema": {
            "type": "object",
            "properties": properties,
            "required": ["action"],
            "additionalProperties": false
        },
        "annotations": { "title": "Drive the Zeron UI", "readOnlyHint": false, "destructiveHint": false, "idempotentHint": false, "openWorldHint": false }
    })
}

pub async fn mcp_result(args: &Value, own_port: Option<u16>) -> Value {
    let image = args["action"] == "snapshot" && args["inline"] != false;
    let result = run(args, own_port).await.and_then(|value| {
        let mut content = vec![json!({ "type": "text", "text": text(&value) })];
        if image {
            content.push(
                json!({ "type": "image", "data": png_base64(&value)?, "mimeType": "image/png" }),
            );
        }
        Ok(content)
    });
    match result {
        Ok(content) => json!({ "content": content, "isError": false }),
        Err(message) => {
            json!({ "content": [{ "type": "text", "text": message }], "isError": true })
        }
    }
}

fn png_base64(snapshot: &Value) -> Result<String, String> {
    let path = snapshot["path"]
        .as_str()
        .ok_or_else(|| format!("snapshot reply names no path: {snapshot}"))?;
    let bytes = std::fs::read(path).map_err(|error| format!("{path}: {error}"))?;
    Ok(base64::engine::general_purpose::STANDARD.encode(bytes))
}

pub fn text(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        other => serde_json::to_string_pretty(other).unwrap_or_default(),
    }
}

pub async fn run(args: &Value, own_port: Option<u16>) -> Result<Value, String> {
    let mut args = match args {
        Value::Object(args) => args.clone(),
        Value::Null => Map::new(),
        other => return Err(format!("arguments must be a JSON object, not {other}")),
    };
    let action = args
        .get("action")
        .and_then(Value::as_str)
        .ok_or("`action` is required: `list` or a driver action")?
        .to_owned();
    let target = match args.remove("target") {
        None | Some(Value::Null) => None,
        Some(Value::String(target)) => Some(target),
        Some(Value::Number(pid)) => Some(pid.to_string()),
        Some(other) => return Err(format!("`target` must be a pid or a label, not {other}")),
    };
    args.remove("inline");
    let live = list();
    if action == "list" {
        return serde_json::to_value(live).map_err(|error| error.to_string());
    }
    let app = select(&live, target.as_deref(), own_port)?;
    request(app, args).await
}

pub fn list() -> Vec<UiDriverDescriptor> {
    let Ok(entries) = std::fs::read_dir(run_dir()) else {
        return Vec::new();
    };
    let mut live: Vec<UiDriverDescriptor> = entries
        .flatten()
        .filter_map(|entry| {
            let name = entry.file_name();
            let pid: u32 = name
                .to_str()?
                .strip_prefix("ui-")?
                .strip_suffix(".json")?
                .parse()
                .ok()?;
            let app: UiDriverDescriptor =
                serde_json::from_slice(&std::fs::read(entry.path()).ok()?).ok()?;
            if alive(&app) {
                return Some(app);
            }
            let _ = std::fs::remove_file(entry.path());
            let _ = std::fs::remove_file(socket_path(pid));
            None
        })
        .collect();
    live.sort_by_key(|app| app.started_at);
    live
}

#[cfg(unix)]
fn alive(app: &UiDriverDescriptor) -> bool {
    use std::io::ErrorKind;
    // SAFETY: signal 0 delivers nothing; kill only reports whether the pid exists.
    let exited = unsafe { libc::kill(app.pid as libc::pid_t, 0) } != 0
        && std::io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH);
    !exited
        && match std::os::unix::net::UnixStream::connect(&app.socket) {
            Ok(_) => true,
            Err(error) => !matches!(
                error.kind(),
                ErrorKind::ConnectionRefused | ErrorKind::NotFound
            ),
        }
}

#[cfg(not(unix))]
fn alive(_: &UiDriverDescriptor) -> bool {
    false
}

pub fn select<'a>(
    live: &'a [UiDriverDescriptor],
    target: Option<&str>,
    own_port: Option<u16>,
) -> Result<&'a UiDriverDescriptor, String> {
    if live.is_empty() {
        return Err(format!(
            "no running Zeron app serves the UI driver: {} has no live ui-<pid>.json",
            run_dir().display()
        ));
    }
    let target = target.map(str::trim).filter(|target| !target.is_empty());
    let (found, missing): (Vec<&UiDriverDescriptor>, String) = match (target, own_port) {
        (None | Some("self"), Some(port)) => {
            let mut serving: Vec<_> = live.iter().filter(|app| app.ipc_port == port).collect();
            serving.sort_by_key(|app| (app.build != UiDriverBuild::Release, app.started_at));
            serving.truncate(1);
            (serving, format!("no Zeron app serves engine port {port}"))
        }
        (Some("self"), None) => {
            return Err(format!(
                "`self` is the app serving a chat, which only the ui_driver tool knows; pass a pid or label\n{}",
                targets(live)
            ));
        }
        (None, None) => (
            live.iter().collect(),
            String::from("no Zeron app is running"),
        ),
        (Some(target), _) if target.parse::<u32>().is_ok() => (
            live.iter()
                .filter(|app| app.pid.to_string() == target)
                .collect(),
            format!("no Zeron app has pid {target}"),
        ),
        (Some(target), _) if target.contains('/') => (
            live.iter()
                .filter(|app| app.socket == Path::new(target))
                .collect(),
            format!("no Zeron app listens on {target}"),
        ),
        (Some(target), _) => {
            let wanted = target.to_lowercase();
            let exact: Vec<_> = live
                .iter()
                .filter(|app| app.label.to_lowercase() == wanted)
                .collect();
            let found = if exact.is_empty() {
                live.iter()
                    .filter(|app| app.label.to_lowercase().contains(&wanted))
                    .collect()
            } else {
                exact
            };
            (found, format!("no Zeron app label matches `{target}`"))
        }
    };
    match found.as_slice() {
        [app] => Ok(app),
        [] => Err(format!("{missing}\n{}", targets(live))),
        many => Err(format!(
            "{} Zeron apps match{}; pass a pid or the exact label as the target\n{}",
            many.len(),
            target.map(|t| format!(" `{t}`")).unwrap_or_default(),
            targets(live)
        )),
    }
}

fn targets(live: &[UiDriverDescriptor]) -> String {
    let mut lines = String::from("live targets (pid label build ipcPort):");
    for app in live {
        let build = match app.build {
            UiDriverBuild::Debug => "debug",
            UiDriverBuild::Release => "release",
        };
        lines.push_str(&format!(
            "\n  {} {} {build} {}",
            app.pid, app.label, app.ipc_port
        ));
    }
    lines
}

fn reply_timeout(request: &Map<String, Value>) -> Duration {
    DEFAULT_TIMEOUT.max(Duration::from_millis(work_ms(request)).saturating_add(REPLY_SLACK))
}

fn work_ms(request: &Map<String, Value>) -> u64 {
    let number = |key: &str| request.get(key).and_then(Value::as_u64);
    let wait = ["timeoutMs", "ms", "quietMs"]
        .into_iter()
        .filter_map(number)
        .max()
        .unwrap_or(0)
        .min(MAX_WAIT_MS);
    let steps = match request.get("action").and_then(Value::as_str) {
        Some("drag") => number("steps").unwrap_or(u64::from(DRAG_STEPS)).max(1),
        Some("type") => request
            .get("text")
            .and_then(Value::as_str)
            .map_or(0, |text| text.chars().count() as u64),
        _ => 0,
    };
    let during = match request.get("during") {
        Some(Value::Object(during)) => work_ms(during),
        _ => 0,
    };
    wait.saturating_add(number("holdMs").unwrap_or(0))
        .saturating_add(steps.saturating_mul(STEP_CAP_MS))
        .saturating_add(during)
}

pub async fn request(
    app: &UiDriverDescriptor,
    mut request: Map<String, Value>,
) -> Result<Value, String> {
    static NEXT_ID: AtomicU64 = AtomicU64::new(1);
    let timeout = reply_timeout(&request);
    request.insert("id".into(), json!(NEXT_ID.fetch_add(1, Ordering::Relaxed)));
    let mut reply = tokio::time::timeout(timeout, exchange(&app.socket, Value::Object(request)))
        .await
        .map_err(|_| {
            format!(
                "{} (pid {}) did not answer within {} s",
                app.label,
                app.pid,
                timeout.as_secs()
            )
        })??;
    if let Some(error) = reply.get("error") {
        return Err(error
            .as_str()
            .map_or_else(|| error.to_string(), str::to_owned));
    }
    reply
        .get_mut("ok")
        .map(Value::take)
        .ok_or_else(|| format!("reply has neither ok nor error: {reply}"))
}

#[cfg(unix)]
async fn exchange(socket: &Path, request: Value) -> Result<Value, String> {
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    let failed = |error: std::io::Error| format!("{}: {error}", socket.display());
    let stream = tokio::net::UnixStream::connect(socket)
        .await
        .map_err(failed)?;
    let (reader, mut writer) = stream.into_split();
    let mut line = request.to_string();
    line.push('\n');
    writer.write_all(line.as_bytes()).await.map_err(failed)?;
    let mut reply = String::new();
    BufReader::new(reader)
        .read_line(&mut reply)
        .await
        .map_err(failed)?;
    if reply.trim().is_empty() {
        return Err(format!("{} closed without a reply", socket.display()));
    }
    serde_json::from_str(&reply).map_err(|error| format!("reply is not JSON: {error}"))
}

#[cfg(not(unix))]
async fn exchange(_: &Path, _: Value) -> Result<Value, String> {
    Err("the UI driver needs unix sockets, which this platform lacks".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{TimeZone, Utc};
    use std::path::PathBuf;

    fn app(
        pid: u32,
        label: &str,
        build: UiDriverBuild,
        port: u16,
        started: i64,
    ) -> UiDriverDescriptor {
        UiDriverDescriptor {
            pid,
            socket: PathBuf::from(format!("/run/ui-{pid}.sock")),
            label: label.into(),
            bundle_path: None,
            data_dir: PathBuf::from("/data"),
            ipc_port: port,
            build,
            input: build == UiDriverBuild::Debug,
            version: "0.0.0".into(),
            started_at: Utc.timestamp_opt(started, 0).unwrap(),
        }
    }

    fn fleet() -> Vec<UiDriverDescriptor> {
        vec![
            app(100, "Zeron", UiDriverBuild::Release, 27654, 10),
            app(200, "Zeron Dev", UiDriverBuild::Debug, 49871, 20),
            app(300, "Zeron Dev 2", UiDriverBuild::Debug, 49872, 30),
            app(400, "Zeron Dev 82", UiDriverBuild::Debug, 27654, 5),
        ]
    }

    fn pick(target: Option<&str>, port: Option<u16>) -> Result<u32, String> {
        let live = fleet();
        select(&live, target, port).map(|app| app.pid)
    }

    #[test]
    fn a_pid_or_socket_path_names_one_app() {
        assert_eq!(pick(Some("300"), None), Ok(300));
        assert_eq!(pick(Some("/run/ui-200.sock"), Some(27654)), Ok(200));
        let error = pick(Some("999"), None).unwrap_err();
        assert!(error.starts_with("no Zeron app has pid 999"), "{error}");
    }

    #[test]
    fn an_exact_label_beats_substrings_and_a_shared_substring_is_ambiguous() {
        assert_eq!(pick(Some("zeron dev"), None), Ok(200));
        assert_eq!(pick(Some("dev 2"), None), Ok(300));
        let error = pick(Some("dev"), None).unwrap_err();
        assert!(error.starts_with("3 Zeron apps match `dev`"), "{error}");
        assert!(error.contains("\n  200 Zeron Dev debug 49871"), "{error}");
        assert!(error.contains("\n  100 Zeron release 27654"), "{error}");
    }

    #[test]
    fn self_is_the_app_on_the_callers_port_preferring_release_then_the_earliest() {
        assert_eq!(pick(None, Some(27654)), Ok(100));
        assert_eq!(pick(Some("self"), Some(27654)), Ok(100));
        assert_eq!(pick(None, Some(49872)), Ok(300));
        let mut live = fleet();
        live.retain(|app| app.pid != 100);
        live.push(app(500, "Zeron Dev 83", UiDriverBuild::Debug, 27654, 1));
        assert_eq!(select(&live, None, Some(27654)).map(|app| app.pid), Ok(500));
    }

    #[test]
    fn no_match_lists_the_live_targets() {
        let error = pick(None, Some(1)).unwrap_err();
        assert!(
            error.starts_with("no Zeron app serves engine port 1\nlive targets"),
            "{error}"
        );
        let error = pick(Some("nightly"), None).unwrap_err();
        assert!(
            error.contains("`nightly`") && error.contains("400 Zeron Dev 82"),
            "{error}"
        );
        assert!(pick(Some("self"), None).unwrap_err().starts_with("`self`"));
        assert!(
            select(&[], Some("dev"), None)
                .unwrap_err()
                .starts_with("no running Zeron app")
        );
    }

    #[test]
    fn the_reply_timeout_covers_waits_drags_typing_and_a_nested_during() {
        let timeout = |request: Value| {
            let Value::Object(request) = request else {
                unreachable!()
            };
            reply_timeout(&request).as_millis() as u64
        };
        assert_eq!(timeout(json!({ "action": "tree" })), 30_000);
        assert_eq!(
            timeout(json!({ "action": "wait", "for": "ms", "ms": 45_000 })),
            55_000
        );
        assert_eq!(
            timeout(
                json!({ "action": "wait", "for": "idle", "quietMs": 50_000, "timeoutMs": 2000 })
            ),
            60_000
        );
        assert_eq!(
            timeout(json!({ "action": "wait", "for": "text", "timeoutMs": 900_000 })),
            MAX_WAIT_MS + 10_000
        );
        assert_eq!(
            timeout(json!({ "action": "drag", "holdMs": 25_000 })),
            25_000 + 12 * 100 + 10_000
        );
        assert_eq!(
            timeout(json!({ "action": "drag", "steps": 300 })),
            30_000 + 10_000
        );
        let text = "x".repeat(400);
        assert_eq!(
            timeout(json!({ "action": "type", "text": text })),
            40_000 + 10_000
        );
        assert_eq!(
            timeout(
                json!({ "action": "track", "ms": 5000, "during": { "action": "drag", "holdMs": 20_000, "steps": 10 } })
            ),
            5000 + 20_000 + 1000 + 10_000
        );
    }

    #[test]
    fn an_omitted_target_outside_a_chat_needs_a_single_app() {
        let live = vec![app(200, "Zeron Dev", UiDriverBuild::Debug, 49871, 20)];
        assert_eq!(select(&live, None, None).map(|app| app.pid), Ok(200));
        let error = pick(None, None).unwrap_err();
        assert!(error.starts_with("4 Zeron apps match;"), "{error}");
    }
}
