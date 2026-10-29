mod launch;

use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use base64::Engine as _;
use serde_json::{Map, Value, json};
use zeron_proto::html_render::{CURRENT_FOLDER, workspace_page};
use zeron_proto::ui_driver::{
    DRAG_STEPS, DRIVE_ACTIONS, MAX_DRIVE_STEPS, MAX_WAIT_MS, OBSERVE_ACTIONS, PROCESS_ACTIONS,
    STEP_CAP_MS, UiDriverBuild, UiDriverDescriptor, UiDriverEndpoint, descriptor_pid, run_dir,
    socket_path,
};

pub const TOOL: &str = "ui_driver";
pub const DRIVE_TOOL: &str = "ui_driver_drive";

const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);
const REPLY_SLACK: Duration = Duration::from_secs(10);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(1);
const RECORDERS: &[&str] = &["frames", "track"];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tool {
    Observe,
    Drive,
}

impl Tool {
    pub fn named(name: &str) -> Option<Tool> {
        match name {
            TOOL => Some(Tool::Observe),
            DRIVE_TOOL => Some(Tool::Drive),
            _ => None,
        }
    }

    fn admits(self, action: &str, args: &Map<String, Value>) -> Result<(), String> {
        let during = args.get("during").is_some_and(|during| !during.is_null());
        let drives = DRIVE_ACTIONS.contains(&action);
        let process = PROCESS_ACTIONS.contains(&action);
        match self {
            Tool::Observe if process => Err(format!(
                "`{action}` starts or quits a Zeron Dev bundle, which {TOOL} never does: call {DRIVE_TOOL}"
            )),
            Tool::Observe if drives || during => Err(format!(
                "`{}` drives the UI, which {TOOL} never does: call {DRIVE_TOOL} with a Zeron Dev bundle as `target`",
                if during { "during" } else { action }
            )),
            Tool::Observe if args.get("path").is_some_and(|path| !path.is_null()) => Err(format!(
                "{TOOL} never writes where the caller picks, so `{action}` takes no `path`: a snapshot is saved under ~/.zeron/run/shots and returned inline, or by path with `inline: false`; a recording is saved under ~/.zeron/run/recordings and its path returned. `zeron ui snapshot` and `zeron ui record` take `path`."
            )),
            Tool::Drive if !drives && !process && !(during && RECORDERS.contains(&action)) => {
                Err(format!(
                    "{DRIVE_TOOL} takes a drive action ({}), `launch` or `quit`, or `frames` or `track` with `during`; call {TOOL} for `{action}`",
                    DRIVE_ACTIONS.join(", ")
                ))
            }
            _ => Ok(()),
        }
    }
}

fn fields() -> Vec<(&'static [Tool], &'static str, Value)> {
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
    let (observe, drive, both) = (
        &[Tool::Observe][..],
        &[Tool::Drive][..],
        &[Tool::Observe, Tool::Drive][..],
    );
    [
        (observe, "target", field("string", "A pid or label from `list` (exact, or a unique substring such as `dev`), a socket path, or `self`. Defaults to the app serving this chat.")),
        (drive, "target", field("string", "The Zeron Dev bundle to drive or `quit`: a pid or label from `ui_driver` `list` (exact, or a unique substring such as `dev`), or a socket path. Never the app serving this chat. `launch` takes none.")),
        (both, "window", field("integer", "Window id from `state`. Defaults to the frontmost window.")),
        (drive, "x", field("number", "Window point in logical pixels.")),
        (drive, "y", field("number", "Window point in logical pixels.")),
        (both, "ref", field("string", "Element ref from `tree`, such as `e12`.")),
        (both, "select", field("string", "Selector: `#e12`, `Role`, `Role \"label\"` or `\"label\"`. An exact label match beats a substring match, which beats value, then placeholder or description.")),
        (both, "nth", field("integer", "Which match of `select` when several match.")),
        (observe, "text", field("string", "Text to wait for, or to wait until it is gone.")),
        (drive, "text", field("string", "Text to type.")),
        (drive, "keys", json!({ "type": "array", "items": { "type": "string" }, "description": "Keystrokes, such as [\"cmd-k\"] or [\"down\", \"enter\"]. The server also takes one string of keystrokes separated by spaces." })),
        (drive, "name", field("string", "gpui action name, for `action`; for `launch`, the bundle's app name (default `Zeron Dev <n>`).")),
        (drive, "n", field("integer", "For `launch`: the Dev number 60-99, which sets target/macos-dev<n> and port 49800+n. Defaults to the lowest free one.")),
        (drive, "mock", field("boolean", "For `launch`: run the app with the mock harness (ZERON_HARNESS=mock).")),
        (drive, "appArgs", json!({ "type": "array", "items": { "type": "string" }, "description": "For `launch`: arguments passed to the app." })),
        (drive, "args", field("object", "Arguments of the gpui action, for `action`.")),
        (observe, "rect", field("object", "Crop of a snapshot, or the region `compare` checks, in window points: {x, y, w, h}.")),
        (observe, "html", field("string", "For `compare`: the approved mockup page. It is laid out in the target app's own web engine at the region's size and current theme, and its nodes are matched to the native ones by role and label.")),
        (observe, "path", field("string", "For `compare`, instead of `html`: the mockup as an .html or .htm file in this chat's folder, relative to it or absolute, read the way html_preview reads `path`.")),
        (observe, "width", field("number", "For `compare`: lay the mockup out at this width instead of the region's.")),
        (observe, "threshold", field("number", "For `compare`: a matched node whose x, y, w or h differs by more than this many points is listed as moved. Defaults to 4.")),
        (drive, "from", end("start")),
        (drive, "to", end("end")),
        (drive, "steps", field("integer", "For `drag`: pointer moves between `from` and `to`. Defaults to 12.")),
        (drive, "holdMs", field("integer", "For `drag`: how long to hold the button at `to` before releasing.")),
        (drive, "do", field("string", "For `press`: the accessibility action to perform, such as click (default), focus, setValue, increment, decrement, expand, collapse, showContextMenu or scrollIntoView. For `clock`: pause, resume or advance.")),
        (drive, "value", field("string", "For `press` with `do: setValue`: the new value.")),
        (observe, "do", json!({ "type": "string", "enum": ["start", "stop"], "description": "For `record`: start taps the window's real clicks, keys, typing and scrolls; stop writes them as a `zeron ui scenario` file under ~/.zeron/run/recordings and returns {path, steps, fallbacks, redacted}. Text typed into a key, token, secret, password or API field is written as `<redacted>`. Never records the app serving this chat; ask the user to run `zeron ui record` there." })),
        (observe, "includeDriven", field("boolean", "For `record` start, debug builds only: also record input that `ui_driver_drive` injects.")),
        (observe, "full", field("boolean", "For `tree`: show every node, without hoisting unnamed containers.")),
        (drive, "during", field("object", "For `frames` and `track`, required: a drive request such as {\"action\": \"click\", \"select\": \"Button \\\"Settings\\\"\"} that runs once recording has started.")),
        (both, "jumpPx", field("number", "For `track`: a per-frame move larger than this counts as a jump. Defaults to 40.")),
        (drive, "dx", field("number", "Horizontal scroll in pixels.")),
        (drive, "dy", field("number", "Vertical scroll in pixels.")),
        (drive, "trackpad", field("boolean", "Scroll as a trackpad instead of a mouse wheel.")),
        (drive, "button", json!({ "type": "string", "enum": ["left", "right", "middle"] })),
        (drive, "count", field("integer", "Clicks; 2 double-clicks.")),
        (drive, "modifiers", json!({ "type": "array", "items": { "type": "string", "enum": ["cmd", "shift", "alt", "ctrl"] } })),
        (observe, "for", json!({ "type": "string", "enum": ["idle", "frame", "ms", "text", "select", "gone"], "description": "What `wait` waits for: `text` a node that holds `text`, `select` an element, `gone` an element (`ref` or `select`) or `text` that is no longer there." })),
        (observe, "quietMs", field("integer", "For `wait` idle: how long no frame may draw.")),
        (observe, "timeoutMs", field("integer", "Longest wait.")),
        (observe, "ms", field("integer", "Duration of `wait` ms; the window of `frames` (default 1000, max 10000) and `track` (default 800, max 5000).")),
        (drive, "ms", field("integer", "The window of `frames` (default 1000, max 10000) and `track` (default 800, max 5000). For `clock` with `do: advance`: how far to step animation time (max 60000).")),
        (observe, "query", field("string", "Keep only `tree` lines that match this text.")),
        (observe, "root", field("string", "Ref the `tree` starts from.")),
        (observe, "depth", field("integer", "Levels of `tree` to print.")),
        (observe, "json", field("boolean", "`tree` or `compare` as JSON instead of text lines.")),
        (observe, "inline", field("boolean", "For `snapshot`: false returns only the path, no image. Defaults to true.")),
        (observe, "native", field("boolean", "For `snapshot`: capture at the display's backing scale (2x on Retina) instead of 1 pixel per point; `rect` stays in points. Defaults to false.")),
        (drive, "theme", json!({ "type": "string", "enum": ["light", "dark", "system"], "description": "For `set`: the appearance mode, as Settings > Appearance sets it (saved to the app's settings)." })),
        (drive, "size", json!({ "type": "object", "properties": { "w": { "type": "number" }, "h": { "type": "number" } }, "required": ["w", "h"], "additionalProperties": false, "description": "For `set`: window content size {w, h} in points. The server also takes [w, h]." })),
        (drive, "motionScale", field("number", "For `set`: animation duration multiplier, 1 is normal, 5 is five times slower. Applies to animations started afterwards; loaders, pulses and disclosure rows ignore it.")),
    ]
    .into_iter()
    .collect()
}

fn actions(tool: Tool) -> Vec<&'static str> {
    match tool {
        Tool::Observe => std::iter::once(&"list")
            .chain(OBSERVE_ACTIONS)
            .copied()
            .collect(),
        Tool::Drive => DRIVE_ACTIONS
            .iter()
            .chain(RECORDERS)
            .chain(PROCESS_ACTIONS)
            .copied()
            .collect(),
    }
}

pub fn reference() -> String {
    let mut out = String::from("# ui_driver and ui_driver_drive\n");
    for tool in [Tool::Observe, Tool::Drive] {
        let name = if tool == Tool::Observe {
            TOOL
        } else {
            DRIVE_TOOL
        };
        out.push_str(&format!(
            "\n## {name}\nActions: {}\n",
            actions(tool).join(", ")
        ));
        for (uses, field, schema) in fields() {
            if uses.contains(&tool)
                && let Some(description) = schema["description"].as_str()
            {
                out.push_str(&format!("- `{field}`: {description}\n"));
            }
        }
    }
    out
}

fn without_descriptions(mut schema: Value) -> Value {
    if let Some(map) = schema.as_object_mut() {
        map.remove("description");
        for value in map.values_mut() {
            *value = without_descriptions(value.take());
        }
    }
    schema
}

pub fn tool(tool: Tool) -> Value {
    let mut properties: Map<String, Value> = Map::new();
    properties.insert(
        "action".into(),
        json!({ "type": "string", "enum": actions(tool) }),
    );
    for (uses, name, schema) in fields() {
        if uses.contains(&tool) {
            properties.insert(name.to_owned(), without_descriptions(schema));
        }
    }
    let (name, title, description) = match tool {
        Tool::Observe => (
            TOOL,
            "Inspect the Zeron UI",
            "Read a running Zeron app's UI from inside it. Never clicks or types: ui_driver_drive does, in Zeron Dev bundles. `list` the apps; omit `target` for the app serving this chat. Prefer `tree` (cheap text lines, each with a `ref`; filter with `query`) over a `snapshot` PNG. Name an element by `ref` or `select` (`Button \"New chat\"` or `\"label\"`, plus `nth`). `wait` instead of sleeping; `frames` and `track` measure motion; `compare` checks against a mockup. Field details: docs tool, read id ui_driver.",
        ),
        Tool::Drive => (
            DRIVE_TOOL,
            "Drive the Zeron UI",
            "Drive a Zeron Dev bundle's UI as a person would: click, drag, scroll, keys, type, accessibility `press`, gpui `action`, `set` theme, size or motion, `clock`, `launch` and `quit`. Refuses the app serving this chat: `target` is a Dev bundle's label or pid from ui_driver `list`. Address an element by `ref` or `select` from ui_driver, else `x`,`y` window points; it first waits up to 1 s for the element to settle (`unsettled: true` if not). Replies after the next frame. `frames` or `track` with `during` measure the transition it starts. Field details: docs tool, read id ui_driver.",
        ),
    };
    json!({
        "name": name,
        "description": description,
        "inputSchema": {
            "type": "object",
            "properties": properties,
            "required": ["action"],
            "additionalProperties": false
        },
        "annotations": {
            "title": title,
            "readOnlyHint": tool == Tool::Observe,
            "destructiveHint": tool == Tool::Drive,
            "idempotentHint": false,
            "openWorldHint": false
        }
    })
}

pub async fn mcp_result(tool: Tool, args: &Value, own_port: Option<u16>) -> Value {
    let image = args["action"] == "snapshot" && args["inline"] != false;
    let result = run(args, own_port, Some(tool)).await.and_then(|value| {
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
    let shots = run_dir().join("shots");
    let inside = std::fs::canonicalize(path)
        .ok()
        .zip(std::fs::canonicalize(&shots).ok())
        .is_some_and(|(path, shots)| path.starts_with(shots));
    if !inside {
        return Err(format!(
            "snapshot reply names {path}, which is not under {}",
            shots.display()
        ));
    }
    let bytes = std::fs::read(path).map_err(|error| format!("{path}: {error}"))?;
    Ok(base64::engine::general_purpose::STANDARD.encode(bytes))
}

pub fn text(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        other => serde_json::to_string_pretty(other).unwrap_or_default(),
    }
}

pub async fn run(args: &Value, own_port: Option<u16>, tool: Option<Tool>) -> Result<Value, String> {
    run_in(&run_dir(), args, own_port, tool).await
}

async fn run_in(
    dir: &Path,
    args: &Value,
    own_port: Option<u16>,
    tool: Option<Tool>,
) -> Result<Value, String> {
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
    if let Some(tool) = tool {
        tool.admits(&action, &args)?;
    }
    if tool.is_none()
        && let Some(path) = compare_path(&args)?
    {
        let html = compare_page(path)?;
        args.remove("path");
        args.insert("html".into(), json!(html));
    }
    if action == "launch" {
        if target.is_some() {
            return Err("`launch` starts a new Zeron Dev bundle, so it takes no `target`".into());
        }
        return launch::launch(dir, &args, own_port).await;
    }
    if action == "quit" && target.is_none() {
        return Err(format!(
            "`quit` needs a `target`\n{}",
            targets(&list_in(dir))
        ));
    }
    let live = list_in(dir);
    if action == "list" {
        return serde_json::to_value(live).map_err(|error| error.to_string());
    }
    let app = select(&live, target.as_deref(), own_port)?;
    if tool == Some(Tool::Drive) && own_port == Some(app.ipc_port) {
        return Err(format!(
            "{DRIVE_TOOL} does not drive {} (pid {}): it serves this chat's engine on port {}, so its input could approve this chat's own requests. Pass a Zeron Dev bundle with its own engine as `target`.\n{}",
            app.label,
            app.pid,
            app.ipc_port,
            targets(&live)
        ));
    }
    if tool.is_some() && action == "record" && own_port == Some(app.ipc_port) {
        return Err(format!(
            "{TOOL} does not record {} (pid {}): it serves this chat's engine on port {}, so a recording would capture what the user types here. Ask the user to run `zeron ui record` themselves, or pass a Zeron Dev bundle as `target`.\n{}",
            app.label,
            app.pid,
            app.ipc_port,
            targets(&live)
        ));
    }
    if action == "quit" {
        return launch::quit(dir, app).await;
    }
    request(app, args).await
}

pub fn compare_path(args: &Map<String, Value>) -> Result<Option<&str>, String> {
    let path = match args.get("path") {
        Some(path) if !path.is_null() && args.get("action") == Some(&json!("compare")) => path,
        _ => return Ok(None),
    };
    if args.get("html").is_some_and(|html| !html.is_null()) {
        return Err("give `html` or `path`, one of them".into());
    }
    path.as_str()
        .map(Some)
        .ok_or_else(|| "`path` must be a string".into())
}

fn compare_page(path: &str) -> Result<String, String> {
    let cwd = std::env::current_dir().map_err(|error| format!("no current folder: {error}"))?;
    let home = std::env::var_os("HOME").map(std::path::PathBuf::from);
    workspace_page(&cwd, home.as_deref(), CURRENT_FOLDER, path).map_err(|error| {
        format!(
            "`zeron ui compare` reads `path` from {}: {error}",
            cwd.display()
        )
    })
}

pub fn list() -> Vec<UiDriverDescriptor> {
    list_in(&run_dir())
}

fn list_in(dir: &Path) -> Vec<UiDriverDescriptor> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut live: Vec<UiDriverDescriptor> = entries
        .flatten()
        .filter_map(|entry| {
            let path = entry.path();
            let pid = descriptor_pid(&path)?;
            let app: UiDriverDescriptor =
                serde_json::from_slice(&std::fs::read(&path).ok()?).ok()?;
            if alive(&app) {
                return Some(app);
            }
            let _ = std::fs::remove_file(&path);
            let _ = std::fs::remove_file(socket_path(dir, pid));
            None
        })
        .collect();
    live.sort_by_key(|app| app.started_at);
    live
}

fn alive(app: &UiDriverDescriptor) -> bool {
    use std::io::ErrorKind;
    #[cfg(unix)]
    {
        // SAFETY: signal 0 delivers nothing; kill only reports whether the pid exists.
        let exited = unsafe { libc::kill(app.pid as libc::pid_t, 0) } != 0
            && std::io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH);
        if exited {
            return false;
        }
    }
    let connected = match &app.endpoint {
        #[cfg(unix)]
        UiDriverEndpoint::Unix { socket } => {
            std::os::unix::net::UnixStream::connect(socket).map(drop)
        }
        #[cfg(not(unix))]
        UiDriverEndpoint::Unix { .. } => return false,
        UiDriverEndpoint::Loopback { port, .. } => std::net::TcpStream::connect_timeout(
            &(std::net::Ipv4Addr::LOCALHOST, *port).into(),
            CONNECT_TIMEOUT,
        )
        .map(drop),
    };
    match connected {
        Ok(()) => true,
        Err(error) => !matches!(
            error.kind(),
            ErrorKind::ConnectionRefused | ErrorKind::NotFound
        ),
    }
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
                .filter(|app| {
                    matches!(&app.endpoint, UiDriverEndpoint::Unix { socket } if socket == Path::new(target))
                })
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
        Some("click") => number("count").unwrap_or(1).max(1),
        Some("type") => request
            .get("text")
            .and_then(Value::as_str)
            .map_or(0, |text| text.chars().count() as u64),
        Some("key") => match request.get("keys") {
            Some(Value::String(keys)) => keys.split_whitespace().count() as u64,
            Some(Value::Array(keys)) => keys
                .iter()
                .filter_map(Value::as_str)
                .map(|keys| keys.split_whitespace().count() as u64)
                .sum(),
            _ => 0,
        },
        _ => 0,
    };
    let during = match request.get("during") {
        Some(Value::Object(during)) => work_ms(during),
        _ => 0,
    };
    let hold = number("holdMs").unwrap_or(0).min(MAX_WAIT_MS);
    wait.saturating_add(hold)
        .saturating_add(steps.min(MAX_DRIVE_STEPS).saturating_mul(STEP_CAP_MS))
        .saturating_add(during)
}

pub async fn request(
    app: &UiDriverDescriptor,
    mut request: Map<String, Value>,
) -> Result<Value, String> {
    static NEXT_ID: AtomicU64 = AtomicU64::new(1);
    let timeout = reply_timeout(&request);
    request.insert("id".into(), json!(NEXT_ID.fetch_add(1, Ordering::Relaxed)));
    let mut reply = tokio::time::timeout(timeout, exchange(&app.endpoint, Value::Object(request)))
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

async fn exchange(endpoint: &UiDriverEndpoint, request: Value) -> Result<Value, String> {
    let failed = |error: std::io::Error| format!("{endpoint}: {error}");
    match endpoint {
        #[cfg(unix)]
        UiDriverEndpoint::Unix { socket } => {
            let stream = tokio::net::UnixStream::connect(socket)
                .await
                .map_err(failed)?;
            talk(stream, None, request, endpoint).await
        }
        #[cfg(not(unix))]
        UiDriverEndpoint::Unix { .. } => Err(format!(
            "{endpoint} is a unix socket, which this platform cannot open"
        )),
        UiDriverEndpoint::Loopback { port, token } => {
            let stream = tokio::net::TcpStream::connect((std::net::Ipv4Addr::LOCALHOST, *port))
                .await
                .map_err(failed)?;
            talk(stream, Some(token), request, endpoint).await
        }
    }
}

async fn talk(
    stream: impl tokio::io::AsyncRead + tokio::io::AsyncWrite,
    token: Option<&str>,
    request: Value,
    endpoint: &UiDriverEndpoint,
) -> Result<Value, String> {
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    let failed = |error: std::io::Error| format!("{endpoint}: {error}");
    let (reader, mut writer) = tokio::io::split(stream);
    let mut line = token.map(|token| format!("{token}\n")).unwrap_or_default();
    line.push_str(&request.to_string());
    line.push('\n');
    writer.write_all(line.as_bytes()).await.map_err(failed)?;
    let mut reply = String::new();
    BufReader::new(reader)
        .read_line(&mut reply)
        .await
        .map_err(failed)?;
    if reply.trim().is_empty() {
        return Err(format!("{endpoint} closed without a reply"));
    }
    serde_json::from_str(&reply).map_err(|error| format!("reply is not JSON: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{TimeZone, Utc};
    use std::path::PathBuf;
    #[cfg(unix)]
    use zeron_proto::ui_driver::descriptor_path;

    fn app(
        pid: u32,
        label: &str,
        build: UiDriverBuild,
        port: u16,
        started: i64,
    ) -> UiDriverDescriptor {
        UiDriverDescriptor {
            pid,
            endpoint: UiDriverEndpoint::Unix {
                socket: PathBuf::from(format!("/run/ui-{pid}.sock")),
            },
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
            timeout(json!({ "action": "click", "x": 1, "y": 2, "count": 400 })),
            40_000 + 10_000
        );
        let keys = vec!["down"; 350];
        assert_eq!(
            timeout(json!({ "action": "key", "keys": keys })),
            35_000 + 10_000
        );
        assert_eq!(
            timeout(json!({ "action": "key", "keys": "a ".repeat(320) })),
            32_000 + 10_000
        );
        assert_eq!(
            timeout(json!({ "action": "drag", "holdMs": u64::MAX, "steps": u64::MAX })),
            MAX_WAIT_MS + MAX_DRIVE_STEPS * STEP_CAP_MS + 10_000
        );
        assert_eq!(
            timeout(
                json!({ "action": "track", "ms": 5000, "during": { "action": "drag", "holdMs": 20_000, "steps": 10 } })
            ),
            5000 + 20_000 + 1000 + 10_000
        );
    }

    #[test]
    fn the_observe_tool_refuses_input_and_the_drive_tool_takes_only_input() {
        let Value::Object(during) = json!({ "during": { "action": "click", "x": 1, "y": 2 } })
        else {
            unreachable!()
        };
        for tool in [Tool::Observe, Tool::Drive] {
            for action in super::tool(tool)["inputSchema"]["properties"]["action"]["enum"]
                .as_array()
                .unwrap()
            {
                let action = action.as_str().unwrap();
                let args = match tool == Tool::Drive && RECORDERS.contains(&action) {
                    true => during.clone(),
                    false => Map::new(),
                };
                assert_eq!(tool.admits(action, &args), Ok(()), "{tool:?} {action}");
            }
        }
        for action in DRIVE_ACTIONS {
            let error = Tool::Observe.admits(action, &Map::new()).unwrap_err();
            assert!(error.starts_with(&format!("`{action}` drives")), "{error}");
            assert!(error.contains(DRIVE_TOOL), "{error}");
        }
        for action in PROCESS_ACTIONS {
            let error = Tool::Observe.admits(action, &Map::new()).unwrap_err();
            assert!(
                error.starts_with(&format!("`{action}` starts or quits")),
                "{error}"
            );
            assert!(error.ends_with(DRIVE_TOOL), "{error}");
        }
        for action in std::iter::once(&"list").chain(OBSERVE_ACTIONS) {
            let error = Tool::Drive.admits(action, &Map::new()).unwrap_err();
            assert!(
                error.ends_with(&format!("call {TOOL} for `{action}`")),
                "{error}"
            );
        }
        let error = Tool::Observe.admits("frames", &during).unwrap_err();
        assert!(error.starts_with("`during` drives"), "{error}");
        let Value::Object(null) = json!({ "during": null }) else {
            unreachable!()
        };
        assert_eq!(Tool::Observe.admits("track", &null), Ok(()));
        let Value::Object(path) = json!({ "path": "/tmp/shot.png" }) else {
            unreachable!()
        };
        let error = Tool::Observe.admits("snapshot", &path).unwrap_err();
        assert!(error.contains("takes no `path`"), "{error}");
        for (name, tool) in [(TOOL, Tool::Observe), (DRIVE_TOOL, Tool::Drive)] {
            assert_eq!(Tool::named(name), Some(tool));
            let schema = super::tool(tool);
            assert_eq!(schema["name"], name);
            assert_eq!(
                schema["inputSchema"]["properties"]["path"].is_null(),
                tool == Tool::Drive,
                "{name}"
            );
            let hints = &schema["annotations"];
            assert_eq!(hints["readOnlyHint"], tool == Tool::Observe, "{name}");
            assert_eq!(hints["destructiveHint"], tool == Tool::Drive, "{name}");
        }
    }

    #[cfg(unix)]
    fn answering(
        socket: &Path,
        replies: &'static [&'static str],
    ) -> tokio::task::JoinHandle<Vec<Value>> {
        use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
        let listener = tokio::net::UnixListener::bind(socket).unwrap();
        tokio::spawn(async move {
            let mut requests = Vec::new();
            while requests.len() < replies.len() {
                let (stream, _) = listener.accept().await.unwrap();
                let (reader, mut writer) = stream.into_split();
                let mut line = String::new();
                if BufReader::new(reader).read_line(&mut line).await.unwrap() == 0 {
                    continue;
                }
                let reply = replies[requests.len()];
                requests.push(serde_json::from_str::<Value>(&line).unwrap());
                if !reply.is_empty() {
                    let reply = format!("{reply}\n");
                    writer.write_all(reply.as_bytes()).await.unwrap();
                }
            }
            requests
        })
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn request_turns_each_reply_into_its_value_or_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let mut target = app(1, "Zeron Dev", UiDriverBuild::Debug, 49871, 1);
        let socket = dir.path().join("ui-1.sock");
        target.endpoint = UiDriverEndpoint::Unix {
            socket: socket.clone(),
        };
        let server = answering(
            &socket,
            &[
                r#"{"id":1,"ok":{"frame":3}}"#,
                r#"{"id":2,"error":"no window"}"#,
                r#"{"id":3,"error":{"code":1}}"#,
                r#"{"id":4}"#,
                "",
                "not json",
            ],
        );
        let mut replies = Vec::new();
        for _ in 0..6 {
            let state = Map::from_iter([("action".into(), json!("state"))]);
            replies.push(request(&target, state).await);
        }
        assert_eq!(replies[0], Ok(json!({ "frame": 3 })));
        assert_eq!(replies[1], Err("no window".into()));
        assert_eq!(replies[2], Err(r#"{"code":1}"#.into()));
        let error = |i: usize| replies[i].clone().unwrap_err();
        assert!(
            error(3).starts_with("reply has neither ok nor error"),
            "{}",
            error(3)
        );
        assert!(error(4).ends_with("closed without a reply"), "{}", error(4));
        assert!(error(5).starts_with("reply is not JSON"), "{}", error(5));
        let requests = server.await.unwrap();
        assert!(
            requests
                .iter()
                .all(|request| request["action"] == "state" && request["id"].is_u64()),
            "{requests:?}"
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn list_prunes_dead_apps_and_the_drive_tool_never_reaches_the_callers_own_app() {
        let dir = tempfile::tempdir().unwrap();
        let write = |app: &UiDriverDescriptor| {
            let path = descriptor_path(dir.path(), app.pid);
            std::fs::write(path, serde_json::to_vec(app).unwrap()).unwrap();
        };
        let mut own = app(
            std::process::id(),
            "Zeron Dev 7",
            UiDriverBuild::Debug,
            27654,
            2,
        );
        let own_socket = socket_path(dir.path(), own.pid);
        own.endpoint = UiDriverEndpoint::Unix {
            socket: own_socket.clone(),
        };
        write(&own);
        let mut dead = app(1, "Zeron Dev 3", UiDriverBuild::Debug, 49872, 1);
        let dead_socket = socket_path(dir.path(), 1);
        dead.endpoint = UiDriverEndpoint::Unix {
            socket: dead_socket.clone(),
        };
        drop(std::os::unix::net::UnixListener::bind(&dead_socket).unwrap());
        write(&dead);
        let server = answering(
            &own_socket,
            &[
                r#"{"id":0,"ok":"drawn"}"#,
                r#"{"id":0,"ok":{"windows":[]}}"#,
            ],
        );

        let live: Vec<u32> = list_in(dir.path()).iter().map(|app| app.pid).collect();
        assert_eq!(live, [own.pid]);
        assert!(!dead_socket.exists() && !descriptor_path(dir.path(), 1).exists());

        let click = json!({ "action": "click", "x": 1, "y": 2 });
        let mut aimed = click.clone();
        aimed["target"] = json!(own.pid);
        for request in [&click, &aimed] {
            let error = run_in(dir.path(), request, Some(27654), Some(Tool::Drive))
                .await
                .unwrap_err();
            assert!(
                error.starts_with("ui_driver_drive does not drive Zeron Dev 7"),
                "{error}"
            );
        }
        let quit = json!({ "action": "quit", "target": own.pid });
        let error = run_in(dir.path(), &quit, Some(27654), Some(Tool::Drive))
            .await
            .unwrap_err();
        assert!(
            error.starts_with("ui_driver_drive does not drive"),
            "{error}"
        );
        let error = run_in(dir.path(), &quit, None, None).await.unwrap_err();
        assert!(
            error.starts_with("`quit` never quits Zeron Dev 7"),
            "{error}"
        );
        let error = run_in(dir.path(), &json!({ "action": "quit" }), None, None)
            .await
            .unwrap_err();
        assert!(error.starts_with("`quit` needs a `target`"), "{error}");
        let record = json!({ "action": "record", "do": "start" });
        let mut aimed_record = record.clone();
        aimed_record["target"] = json!("Zeron Dev 7");
        for request in [&record, &aimed_record] {
            let error = run_in(dir.path(), request, Some(27654), Some(Tool::Observe))
                .await
                .unwrap_err();
            assert!(
                error.starts_with("ui_driver does not record Zeron Dev 7")
                    && error.contains("`zeron ui record`"),
                "{error}"
            );
        }
        let other_chat = run_in(dir.path(), &aimed, Some(49880), Some(Tool::Drive)).await;
        assert_eq!(other_chat, Ok(json!("drawn")));
        let state = json!({ "action": "state" });
        let observed = run_in(dir.path(), &state, Some(27654), Some(Tool::Observe)).await;
        assert_eq!(observed, Ok(json!({ "windows": [] })));
        let requests = server.await.unwrap();
        assert_eq!(requests[0]["action"], "click");
        assert_eq!(requests[1]["action"], "state");
    }

    #[test]
    fn an_omitted_target_outside_a_chat_needs_a_single_app() {
        let live = vec![app(200, "Zeron Dev", UiDriverBuild::Debug, 49871, 20)];
        assert_eq!(select(&live, None, None).map(|app| app.pid), Ok(200));
        let error = pick(None, None).unwrap_err();
        assert!(error.starts_with("4 Zeron apps match;"), "{error}");
    }

    #[tokio::test]
    async fn a_loopback_app_is_alive_while_it_listens_and_gets_its_token_before_each_request() {
        use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
        let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
            .await
            .unwrap();
        let port = listener.local_addr().unwrap().port();
        let mut target = app(std::process::id(), "Zeron", UiDriverBuild::Debug, 49871, 1);
        target.endpoint = UiDriverEndpoint::Loopback {
            port,
            token: "secret".into(),
        };
        let server = tokio::spawn(async move {
            loop {
                let (stream, _) = listener.accept().await.unwrap();
                let (reader, mut writer) = stream.into_split();
                let mut lines = BufReader::new(reader).lines();
                let Some(token) = lines.next_line().await.unwrap() else {
                    continue;
                };
                let request: Value =
                    serde_json::from_str(&lines.next_line().await.unwrap().unwrap()).unwrap();
                let reply = json!({ "id": request["id"], "ok": { "token": token } });
                writer
                    .write_all(format!("{reply}\n").as_bytes())
                    .await
                    .unwrap();
                return request;
            }
        });

        assert!(
            tokio::task::spawn_blocking({
                let target = target.clone();
                move || alive(&target)
            })
            .await
            .unwrap()
        );
        let state = Map::from_iter([("action".into(), json!("state"))]);
        assert_eq!(
            request(&target, state).await,
            Ok(json!({ "token": "secret" }))
        );
        assert_eq!(server.await.unwrap()["action"], "state");
        assert!(!alive(&target));
    }
}
