use crate::ToolCall;
use serde_json::{Map, Value};

pub const COMPUTER_TOOL_TITLES: &[(&str, &str)] = &[
    ("computer_screenshot", "Take a screenshot"),
    ("computer_get_state", "Read the screen"),
    ("computer_get_screen_size", "Measure the screen"),
    ("computer_list_windows", "Find open windows"),
    ("computer_list_apps", "List apps"),
    ("computer_verify_state", "Verify state"),
    ("computer_zoom", "Zoom into a window"),
    ("computer_get_accessibility_tree", "List apps and windows"),
    ("computer_get_cursor_position", "Read the cursor position"),
    ("computer_help", "Read the Computer playbook"),
    ("computer_click", "Click"),
    ("computer_move_cursor", "Move the agent cursor"),
    ("computer_drag", "Drag"),
    ("computer_scroll", "Scroll"),
    ("computer_type_text", "Type"),
    ("computer_press_key", "Press a key"),
    ("computer_set_value", "Set a field"),
    ("computer_select_text", "Select text"),
    ("computer_perform_action", "Activate a control"),
    ("computer_launch_app", "Open an app"),
    ("computer_activate_window", "Activate a window"),
    ("computer_set_window_frame", "Move or resize a window"),
    ("computer_invoke_menu", "Invoke a menu item"),
    ("computer_kill_app", "Force-quit an app"),
    (
        "computer_set_window_minimized",
        "Minimize or restore a window",
    ),
    ("computer_set_app_visibility", "Hide or unhide an app"),
    ("computer_wait", "Wait"),
    ("computer_read_clipboard", "Read the clipboard"),
    ("computer_write_clipboard", "Write to the clipboard"),
    ("computer_paste", "Paste text"),
    ("computer_run", "Run a sequence"),
    ("computer_inspect", "Inspect the computer"),
    ("computer_spaces", "Inspect desktop Spaces"),
    ("computer_browser_state", "Read the browser page"),
    ("computer_browser_prepare", "Prepare a browser"),
    ("computer_browser_navigate", "Open a browser page"),
    ("computer_browser_click", "Click in the browser"),
    ("computer_browser_type", "Type in a browser field"),
    ("computer_browser_dialog", "Handle a browser dialog"),
    ("computer_browser_upload", "Attach files in the browser"),
    ("computer_browser_download", "Download a file"),
    ("computer_browser_pointer", "Use the pointer in the browser"),
    ("computer_browser_press", "Press Enter in the browser"),
    ("computer_zeron_browser", "Use the Zeron browser"),
    ("computer", "Use the computer"),
];

fn title(tool: &str) -> &'static str {
    COMPUTER_TOOL_TITLES
        .iter()
        .find(|(name, _)| *name == tool)
        .map_or("Computer", |(_, title)| title)
}

/// Providers wrap one tool differently: `mcp__zeron__computer_click`, `zeron/computer_click`.
pub fn computer_tool_name(candidate: &str) -> Option<&'static str> {
    let mut normalized = String::with_capacity(candidate.len());
    for ch in candidate.trim().chars().flat_map(char::to_lowercase) {
        if ch.is_ascii_alphanumeric() {
            normalized.push(ch);
        } else if !normalized.ends_with('_') {
            normalized.push('_');
        }
    }
    COMPUTER_TOOL_TITLES
        .iter()
        .map(|(name, _)| *name)
        .find(|name| {
            normalized == *name
                || normalized
                    .strip_suffix(name)
                    .is_some_and(|head| head.ends_with('_'))
        })
}

fn zeron_computer_tool_name(candidate: &str) -> Option<&'static str> {
    let tool = computer_tool_name(candidate)?;
    let normalized = candidate
        .trim()
        .to_lowercase()
        .replace(|ch: char| !ch.is_ascii_alphanumeric(), "_");
    let head = normalized.strip_suffix(tool)?.trim_end_matches('_');
    (head == "zeron" || head.ends_with("_zeron")).then_some(tool)
}

pub struct ComputerCall<'a> {
    pub tool: &'static str,
    pub args: Option<&'a Map<String, Value>>,
}

pub fn computer_call(call: &ToolCall) -> Option<ComputerCall<'_>> {
    let (tool, input) = match call {
        ToolCall::Mcp {
            server,
            tool,
            input,
        } if server == "zeron" => (computer_tool_name(tool), input.as_ref()),
        ToolCall::Unknown { name, input } => (zeron_computer_tool_name(name), input.as_ref()),
        _ => return None,
    };
    let raw = input.and_then(Value::as_object);
    if let Some(tool) = tool {
        return Some(ComputerCall { tool, args: raw });
    }
    let raw = raw?;
    let tool = ["tool", "toolName", "tool_name", "name", "_toolName"]
        .iter()
        .filter_map(|key| raw.get(*key).and_then(Value::as_str))
        .find_map(zeron_computer_tool_name)?;
    let args = ["arguments", "args", "input"]
        .iter()
        .find_map(|key| raw.get(*key).and_then(Value::as_object))
        .or(Some(raw));
    Some(ComputerCall { tool, args })
}

fn text<'a>(args: &'a Map<String, Value>, key: &str) -> Option<&'a str> {
    args.get(key)
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
}

fn number(args: &Map<String, Value>, key: &str) -> Option<f64> {
    args.get(key)
        .and_then(Value::as_f64)
        .filter(|value| value.is_finite())
}

fn strings<'a>(args: &'a Map<String, Value>, key: &str) -> Vec<&'a str> {
    args.get(key)
        .and_then(Value::as_array)
        .map(|items| items.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default()
}

fn format_number(value: f64) -> String {
    if value.fract() == 0.0 && value.abs() < 1e15 {
        format!("{}", value as i64)
    } else {
        format!("{value}")
    }
}

fn truncate(value: &str, max: usize) -> String {
    let collapsed = value.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.chars().count() <= max {
        return collapsed;
    }
    let mut out: String = collapsed.chars().take(max - 1).collect();
    out.push('…');
    out
}

fn app_label(args: &Map<String, Value>) -> Option<&str> {
    text(args, "app_name")
        .or_else(|| text(args, "application"))
        .or_else(|| text(args, "app"))
}

fn describe_target(
    args: &Map<String, Value>,
    preposition: &str,
    always_show_coordinates: bool,
) -> String {
    let mut parts = Vec::new();
    let coordinates = match (number(args, "x"), number(args, "y")) {
        (Some(x), Some(y)) => Some(format!("at ({}, {})", format_number(x), format_number(y))),
        _ => None,
    };
    if let Some(label) = text(args, "label") {
        parts.push(format!("{preposition} “{label}”"));
    } else if always_show_coordinates && let Some(coordinates) = &coordinates {
        parts.push(coordinates.clone());
    }
    if let Some(app) = app_label(args) {
        parts.push(format!("in {app}"));
    }
    if parts.is_empty()
        && let Some(coordinates) = coordinates
    {
        parts.push(coordinates);
    }
    parts.join(" ")
}

fn describe_drag_target(args: &Map<String, Value>) -> String {
    let (Some(from), Some(to)) = (
        args.get("from").and_then(Value::as_object),
        args.get("to").and_then(Value::as_object),
    ) else {
        return String::new();
    };
    let strip = |target: String| {
        target
            .strip_prefix("on ")
            .map(str::to_owned)
            .unwrap_or(target)
    };
    let from = strip(describe_target(from, "on", false));
    let to = strip(describe_target(to, "on", false));
    if from.is_empty() || to.is_empty() {
        String::new()
    } else {
        format!("from {from} to {to}")
    }
}

fn key_name(key: &str) -> String {
    let named = match key.to_lowercase().as_str() {
        "cmd" | "command" => "Command",
        "super" => "Super",
        "meta" => "Meta",
        "ctrl" | "control" => "Control",
        "alt" => "Alt",
        "option" => "Option",
        "shift" => "Shift",
        "return" | "enter" => "Enter",
        "esc" | "escape" => "Escape",
        "space" => "Space",
        "tab" => "Tab",
        "backspace" => "Backspace",
        "delete" => "Delete",
        "arrowup" => "Up arrow",
        "arrowdown" => "Down arrow",
        "arrowleft" => "Left arrow",
        "arrowright" => "Right arrow",
        _ if key.chars().count() == 1 => return key.to_uppercase(),
        _ => return key.to_owned(),
    };
    named.to_owned()
}

fn keyboard_shortcut(value: &str) -> String {
    let mut keys = Vec::new();
    let mut current = String::new();
    let mut chars = value.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '+' && !current.is_empty() && chars.peek().is_some() {
            keys.push(std::mem::take(&mut current));
        } else {
            current.push(ch);
        }
    }
    keys.push(current);
    keys.iter()
        .map(|key| key_name(key.trim()))
        .collect::<Vec<_>>()
        .join(" + ")
}

pub fn app_name(value: Option<&str>) -> Option<String> {
    let name = value?.trim();
    if name.is_empty() {
        return None;
    }
    let basename = name.rsplit(['/', '\\']).next().unwrap_or(name);
    let basename = if basename.to_lowercase().ends_with(".app") {
        &basename[..basename.len() - 4]
    } else {
        basename
    };
    let segments: Vec<&str> = basename.split('.').collect();
    let reverse_dns = segments.len() >= 3
        && segments[..segments.len() - 1].iter().all(|segment| {
            segment
                .chars()
                .next()
                .is_some_and(|c| c.is_ascii_alphabetic())
                && segment
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '-')
        });
    let label = if reverse_dns {
        segments.last().copied().unwrap_or(basename)
    } else {
        basename
    };
    let mut chars = label.chars();
    let capitalized = match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect::<String>(),
        None => return None,
    };
    Some(truncate(&capitalized, 80))
}

// The doc keeps only a 160-char prefix of tool output, so this reads possibly truncated JSON.
fn output_app_name(output: &str) -> Option<String> {
    use serde::Deserialize;
    let (mut depth, mut in_string, mut escaped) = (0usize, false, false);
    for (index, byte) in output.bytes().enumerate() {
        if in_string {
            match byte {
                _ if escaped => escaped = false,
                b'\\' => escaped = true,
                b'"' => in_string = false,
                _ => {}
            }
            continue;
        }
        match byte {
            b'{' | b'[' => depth += 1,
            b'}' | b']' => depth = depth.saturating_sub(1),
            b'"' => {
                if depth == 1
                    && let Some(rest) = output[index..].strip_prefix(r#""appName":"#)
                {
                    return String::deserialize(&mut serde_json::Deserializer::from_str(rest)).ok();
                }
                in_string = true;
            }
            _ => {}
        }
    }
    None
}

pub fn computer_call_app(
    tool: &str,
    args: Option<&Map<String, Value>>,
    output: Option<&str>,
) -> Option<String> {
    let from_args = args.and_then(|args| {
        app_label(args)
            .or_else(|| {
                (tool == "computer_launch_app")
                    .then(|| text(args, "name"))
                    .flatten()
            })
            .or_else(|| text(args, "bundle_id"))
    });
    app_name(from_args).or_else(|| app_name(output.and_then(output_app_name).as_deref()))
}

fn describe_payload(tool: &str, args: &Map<String, Value>) -> String {
    match tool {
        "computer_type_text"
        | "computer_set_value"
        | "computer_write_clipboard"
        | "computer_paste"
        | "computer_run" => String::new(),
        "computer_press_key" => text(args, "key").map(keyboard_shortcut).unwrap_or_default(),
        "computer_scroll" => {
            let dx = number(args, "delta_x").unwrap_or(0.0);
            let dy = number(args, "delta_y").unwrap_or(0.0);
            if dy != 0.0 {
                if dy > 0.0 { "down" } else { "up" }.into()
            } else if dx != 0.0 {
                if dx > 0.0 { "right" } else { "left" }.into()
            } else {
                String::new()
            }
        }
        "computer_launch_app" => app_name(text(args, "app"))
            .or_else(|| app_name(text(args, "name")))
            .or_else(|| app_name(text(args, "bundle_id")))
            .unwrap_or_default(),
        "computer_invoke_menu" => {
            let path = strings(args, "path");
            if path.is_empty() {
                String::new()
            } else {
                truncate(&path.join(" → "), 80)
            }
        }
        "computer_wait" if text(args, "label").is_none() => match number(args, "duration_ms") {
            Some(ms) if ms >= 1000.0 && ms % 1000.0 == 0.0 => {
                let seconds = ms / 1000.0;
                let unit = if ms == 1000.0 { "second" } else { "seconds" };
                format!("for {} {unit}", format_number(seconds))
            }
            Some(ms) => format!("for {} ms", format_number(ms)),
            None => String::new(),
        },
        _ => String::new(),
    }
}

fn browser_site(value: Option<&str>) -> Option<String> {
    let parsed = url::Url::parse(value?).ok()?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return None;
    }
    Some(truncate(parsed.host_str()?, 80))
}

fn describe_browser_action(tool: &str, args: &Map<String, Value>) -> String {
    let flag = |key: &str| args.get(key).and_then(Value::as_bool);
    match tool {
        "computer_browser_prepare" if flag("allow_launch") == Some(true) => {
            return if flag("windowed") == Some(true) {
                "Open an isolated browser window"
            } else {
                "Open an isolated browser in the background"
            }
            .into();
        }
        "computer_browser_navigate" => {
            if let Some(site) = browser_site(text(args, "url")) {
                return format!("Open {site} in the browser");
            }
        }
        "computer_browser_type" if flag("replace") == Some(true) => {
            let cleared = args.get("text").and_then(Value::as_str) == Some("");
            return if cleared {
                "Clear a browser field"
            } else {
                "Replace text in a browser field"
            }
            .into();
        }
        "computer_browser_dialog" => {
            let verb = match args.get("action").and_then(Value::as_str) {
                Some("inspect") => Some("Read"),
                Some("accept") => Some("Accept"),
                Some("dismiss") => Some("Dismiss"),
                _ => None,
            };
            if let Some(verb) = verb {
                return format!("{verb} a browser dialog");
            }
        }
        "computer_browser_upload" => {
            let count = strings(args, "files").len();
            if count == 1 {
                return "Attach a file in the browser".into();
            }
            if count > 1 {
                return format!("Attach {count} files in the browser");
            }
        }
        "computer_browser_pointer" => {
            let action = args.get("action").and_then(Value::as_str);
            if action == Some("scroll") {
                let direction = describe_payload("computer_scroll", args);
                return ["Scroll", direction.as_str(), "in the browser"]
                    .iter()
                    .filter(|part| !part.is_empty())
                    .copied()
                    .collect::<Vec<_>>()
                    .join(" ");
            }
            let described = match action {
                Some("hover") => Some("Hover over a browser control"),
                Some("right_click") => Some("Right-click in the browser"),
                Some("double_click") => Some("Double-click in the browser"),
                Some("drag") => Some("Drag in the browser"),
                _ => None,
            };
            if let Some(described) = described {
                return described.into();
            }
        }
        _ => {}
    }
    title(tool).into()
}

const INSPECT_DELEGATES: [&str; 5] = [
    "computer_read_clipboard",
    "computer_zoom",
    "computer_get_accessibility_tree",
    "computer_get_cursor_position",
    "computer_spaces",
];

fn inspect_delegate<'a>(
    tool: &str,
    args: &'a Map<String, Value>,
) -> Option<(&'static str, &'a Map<String, Value>)> {
    if tool != "computer_inspect" {
        return None;
    }
    let selected = text(args, "tool")?;
    let selected = INSPECT_DELEGATES.iter().find(|name| **name == selected)?;
    static EMPTY: std::sync::OnceLock<Map<String, Value>> = std::sync::OnceLock::new();
    let inner = args
        .get("arguments")
        .and_then(Value::as_object)
        .unwrap_or_else(|| EMPTY.get_or_init(Map::new));
    Some((selected, inner))
}

/// Never includes typed text or clipboard values: they can be credentials.
pub fn describe_computer_call(tool: &str, args: Option<&Map<String, Value>>) -> String {
    let empty = Map::new();
    let args = args.unwrap_or(&empty);
    if let Some((selected, inner)) = inspect_delegate(tool, args) {
        return describe_computer_call(selected, Some(inner));
    }
    if tool == "computer_spaces" {
        return match text(args, "operation").unwrap_or("list") {
            "list" => "Inspect desktop Spaces",
            "reserve" => "Reserve a desktop Space for this task",
            "release" => "Release the task's desktop Space",
            "select" => "Select a window in the task's Space",
            "peek" => "Inspect a window without switching Spaces",
            _ => "Check a desktop Space operation",
        }
        .into();
    }
    if tool.starts_with("computer_browser_") {
        return describe_browser_action(tool, args);
    }
    let flag = |key: &str| args.get(key).and_then(Value::as_bool);
    let direction = |value: Option<bool>, yes: &str, no: &str| match value {
        Some(true) => Some(yes.to_owned()),
        Some(false) => Some(no.to_owned()),
        None => None,
    };
    let verb = match tool {
        "computer_press_key" if text(args, "key").is_some() => "Press".to_owned(),
        "computer_launch_app" => "Open".to_owned(),
        "computer_activate_window" => format!("Switch to {}", app_label(args).unwrap_or_default())
            .trim_end()
            .to_owned(),
        "computer_set_window_minimized" => {
            direction(flag("minimized"), "Minimize a window", "Restore a window")
                .unwrap_or_else(|| title(tool).into())
        }
        "computer_set_app_visibility" => direction(flag("hidden"), "Hide an app", "Unhide an app")
            .unwrap_or_else(|| title(tool).into()),
        _ => title(tool).into(),
    };
    let target = match tool {
        "computer_launch_app" | "computer_activate_window" | "computer_set_app_visibility" => {
            String::new()
        }
        "computer_drag" => describe_drag_target(args),
        "computer_wait" => describe_target(args, "for", false),
        "computer_type_text" | "computer_set_value" => describe_target(args, "in", false),
        "computer_zoom" => describe_target(args, "on", true),
        _ => describe_target(args, "on", false),
    };
    let payload = describe_payload(tool, args);
    [verb, payload, target]
        .into_iter()
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

pub fn computer_call_rows(tool: &str, args: Option<&Map<String, Value>>) -> Vec<String> {
    let empty = Map::new();
    let args = args.unwrap_or(&empty);
    if let Some((selected, inner)) = inspect_delegate(tool, args) {
        return computer_call_rows(selected, Some(inner));
    }
    let mut rows = Vec::new();
    let mut row = |name: &str, value: String| rows.push(format!("{name}: {value}"));
    if tool == "computer_spaces"
        && let Some(space) = number(args, "space_id")
    {
        row("Space ID", format_number(space));
    }
    if let (Some(x), Some(y)) = (number(args, "x"), number(args, "y")) {
        let name = if tool == "computer_set_window_frame" {
            "New position"
        } else {
            "Position"
        };
        row(name, format!("{}, {}", format_number(x), format_number(y)));
    }
    if matches!(tool, "computer_set_window_frame" | "computer_zoom")
        && let (Some(width), Some(height)) = (number(args, "width"), number(args, "height"))
    {
        row(
            "Size",
            format!("{}×{}", format_number(width), format_number(height)),
        );
    }
    if tool == "computer_invoke_menu" {
        let path = strings(args, "path");
        if !path.is_empty() {
            row("Menu", truncate(&path.join(" → "), 200));
        }
    }
    if let Some(label) = text(args, "label") {
        row("Target", label.to_owned());
    }
    if let Some(role) = text(args, "role") {
        row("Role", role.to_owned());
    }
    if let Some(key) = text(args, "key") {
        row("Key", keyboard_shortcut(key));
    }
    let keys = strings(args, "keys");
    if !keys.is_empty() {
        row(
            "Shortcut",
            keys.iter()
                .map(|key| key_name(key))
                .collect::<Vec<_>>()
                .join(" + "),
        );
    }
    let (dx, dy) = (number(args, "delta_x"), number(args, "delta_y"));
    if dx.is_some() || dy.is_some() {
        row(
            "Scroll",
            format!(
                "{}, {}",
                format_number(dx.unwrap_or(0.0)),
                format_number(dy.unwrap_or(0.0))
            ),
        );
    }
    if tool == "computer_select_text"
        && let (Some(start), Some(length)) = (number(args, "start"), number(args, "length"))
    {
        row(
            "Range",
            format!("{}, {}", format_number(start), format_number(length)),
        );
    }
    if let Some(action) = text(args, "action")
        && !tool.starts_with("computer_browser_")
    {
        row("Action", action.to_owned());
    }
    if let Some(topic) = text(args, "topic") {
        row("Topic", topic.to_owned());
    }
    if let Some(app) = text(args, "app")
        .or_else(|| text(args, "name"))
        .or_else(|| text(args, "bundle_id"))
    {
        row("App", app.to_owned());
    }
    if tool.starts_with("computer_browser_") {
        if let Some(site) = browser_site(text(args, "url")) {
            row("Website", site);
        }
        let files = strings(args, "files").len();
        if files > 0 {
            row("Files", files.to_string());
        }
        if tool == "computer_browser_prepare"
            && args.get("allow_launch").and_then(Value::as_bool) == Some(true)
        {
            let windowed = args.get("windowed").and_then(Value::as_bool) == Some(true);
            row(
                "Browser",
                if windowed {
                    "Visible window"
                } else {
                    "Background"
                }
                .into(),
            );
        }
    }
    if tool == "computer_launch_app" && args.get("hidden").and_then(Value::as_bool) == Some(true) {
        row("Hidden", "yes".into());
    }
    if tool == "computer_set_app_visibility"
        && let Some(pid) = number(args, "pid")
    {
        row("PID", format_number(pid));
    }
    if tool == "computer_run"
        && let Some(steps) = args.get("steps").and_then(Value::as_array)
    {
        let kinds: Vec<&str> = steps
            .iter()
            .filter_map(|step| step.get("type").and_then(Value::as_str))
            .collect();
        let value = if kinds.is_empty() {
            steps.len().to_string()
        } else {
            kinds
                .iter()
                .map(|kind| computer_tool_name(&format!("computer_{kind}")).map_or("Action", title))
                .collect::<Vec<_>>()
                .join(" → ")
        };
        row("Steps", truncate(&value, 200));
    }
    rows
}

pub const ACTION_DETAIL_MAX: usize = 48;
pub const JS_TITLE_MAX: usize = 80;
const FALLBACK_APP: &str = "App";

pub fn computer_action_detail(text: &str) -> String {
    truncate(text, ACTION_DETAIL_MAX)
}

pub fn computer_action_label(
    tool: &str,
    args: Option<&Map<String, Value>>,
    app: Option<&str>,
    done: bool,
) -> String {
    let empty = Map::new();
    let args = args.unwrap_or(&empty);
    if let Some((selected, inner)) = inspect_delegate(tool, args) {
        return computer_action_label(selected, Some(inner), app, done);
    }
    let app = app
        .map(str::trim)
        .filter(|app| !app.is_empty())
        .unwrap_or(FALLBACK_APP);
    let pick = |running: &str, finished: &str| if done { finished } else { running }.to_owned();
    let quoted = |key: &str| {
        args.get(key)
            .and_then(Value::as_str)
            .map(computer_action_detail)
            .unwrap_or_default()
    };
    match tool {
        "computer" => text(args, "title").map_or_else(
            || pick("Using the computer", "Used the computer"),
            |title| truncate(title, JS_TITLE_MAX),
        ),
        "computer_click" | "computer_browser_click" => {
            format!("{} in {app}", pick("Clicking", "Clicked"))
        }
        "computer_get_state"
        | "computer_screenshot"
        | "computer_verify_state"
        | "computer_zoom"
        | "computer_get_accessibility_tree"
        | "computer_list_windows"
        | "computer_get_screen_size"
        | "computer_get_cursor_position"
        | "computer_browser_state" => format!("{} at {app}", pick("Looking", "Looked")),
        "computer_type_text" | "computer_browser_type" => format!(
            "{} text “{}” in {app}",
            pick("Typing", "Typed"),
            quoted("text")
        ),
        "computer_scroll" => {
            let direction = describe_payload("computer_scroll", args);
            let direction = if direction.is_empty() {
                "down".to_owned()
            } else {
                direction
            };
            format!("{} {direction} in {app}", pick("Scrolling", "Scrolled"))
        }
        "computer_set_value" => format!(
            "{} to “{}” in {app}",
            pick("Setting", "Set"),
            quoted("value")
        ),
        "computer_press_key" | "computer_browser_press" => {
            format!("{} key in {app}", pick("Pressing", "Pressed"))
        }
        "computer_drag" => format!("{} in {app}", pick("Dragging", "Dragged")),
        "computer_perform_action" | "computer_select_text" | "computer_invoke_menu" => format!(
            "{} accessibility action in {app}",
            pick("Performing", "Performed")
        ),
        "computer_list_apps" => "List Mac apps".to_owned(),
        _ => describe_computer_call(tool, Some(args)),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ComputerJsAction {
    pub tool: &'static str,
    pub app: Option<String>,
    pub detail: Option<String>,
    pub count: usize,
}

pub fn computer_js_actions(output: &str) -> Vec<ComputerJsAction> {
    let first_line = output
        .lines()
        .find(|line| !line.trim().is_empty())
        .unwrap_or_default();
    let Ok(Value::Object(payload)) = serde_json::from_str::<Value>(first_line.trim()) else {
        return Vec::new();
    };
    let Some(actions) = payload.get("actions").and_then(Value::as_array) else {
        return Vec::new();
    };
    let default_app = payload
        .get("appName")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|app| !app.is_empty())
        .map(str::to_owned);
    actions
        .iter()
        .filter_map(Value::as_object)
        .filter_map(|action| {
            let tool = action.get("tool").and_then(Value::as_str)?;
            let tool = computer_tool_name(tool)
                .or_else(|| computer_tool_name(&format!("computer_{tool}")))?;
            let field = |key: &str| {
                action
                    .get(key)
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(str::to_owned)
            };
            Some(ComputerJsAction {
                tool,
                app: field("app").or_else(|| default_app.clone()),
                detail: field("detail"),
                count: action
                    .get("n")
                    .and_then(Value::as_u64)
                    .map_or(1, |n| n.max(1) as usize),
            })
        })
        .collect()
}

pub fn computer_call_js_actions(
    computer: &ComputerCall<'_>,
    output: Option<&str>,
) -> Vec<ComputerJsAction> {
    if computer.tool != "computer" {
        return Vec::new();
    }
    let badge = computer.args.and_then(|args| {
        let actions = args.get("actions")?;
        Some(serde_json::json!({ "appName": args.get("app"), "actions": actions }).to_string())
    });
    badge
        .as_deref()
        .or(output)
        .map(computer_js_actions)
        .unwrap_or_default()
}

pub fn computer_js_action_label(action: &ComputerJsAction, done: bool) -> String {
    let key = match action.tool {
        "computer_set_value" => "value",
        _ => "text",
    };
    let mut args = Map::new();
    if let Some(detail) = &action.detail {
        args.insert(key.to_owned(), Value::String(detail.clone()));
        if action.tool == "computer_scroll" {
            let delta = match detail.as_str() {
                "up" => Some(("delta_y", -1.0)),
                "down" => Some(("delta_y", 1.0)),
                "left" => Some(("delta_x", -1.0)),
                "right" => Some(("delta_x", 1.0)),
                _ => None,
            };
            if let Some((axis, value)) = delta {
                args.insert(axis.to_owned(), Value::from(value));
            }
        }
    }
    let label = computer_action_label(action.tool, Some(&args), action.app.as_deref(), done);
    if action.count > 1 {
        format!("{label} ×{}", action.count)
    } else {
        label
    }
}

const TEXT_OUTPUT_TOOLS: [&str; 6] = [
    "computer_read_clipboard",
    "computer_type_text",
    "computer_set_value",
    "computer_paste",
    "computer_write_clipboard",
    "computer_browser_type",
];

/// A clipboard read's output is the clipboard itself; a typing result echoes what was typed.
pub fn computer_output_hidden(tool: &str, args: Option<&Map<String, Value>>) -> bool {
    let carries_text = |tool: &str| TEXT_OUTPUT_TOOLS.contains(&tool);
    carries_text(tool)
        || args
            .and_then(|args| inspect_delegate(tool, args))
            .is_some_and(|(selected, _)| carries_text(selected))
        || (tool == "computer_run"
            && args
                .and_then(|args| args.get("steps"))
                .and_then(Value::as_array)
                .is_some_and(|steps| {
                    steps.iter().any(|step| {
                        step.get("type")
                            .and_then(Value::as_str)
                            .is_some_and(|kind| carries_text(&format!("computer_{kind}")))
                    })
                }))
}

pub fn computer_group_segment(apps: &[&str], actions: usize) -> String {
    let apps = if apps.is_empty() {
        "the desktop".to_owned()
    } else {
        apps.join(", ")
    };
    let actions = if actions == 1 {
        "1 action".to_owned()
    } else {
        format!("{actions} actions")
    };
    format!("used {apps} · {actions}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn mcp(tool: &str, input: Value) -> ToolCall {
        ToolCall::Mcp {
            server: "zeron".into(),
            tool: tool.into(),
            input: Some(input),
        }
    }

    fn describe(call: &ToolCall) -> String {
        let found = computer_call(call).expect("a computer call");
        describe_computer_call(found.tool, found.args)
    }

    #[test]
    fn names_are_recovered_from_every_provider_wrapping() {
        assert_eq!(computer_tool_name("computer_click"), Some("computer_click"));
        assert_eq!(
            computer_tool_name("mcp__zeron__computer_click"),
            Some("computer_click")
        );
        assert_eq!(
            computer_tool_name("zeron/computer_browser_click"),
            Some("computer_browser_click")
        );
        assert_eq!(
            computer_tool_name("Tool: zeron/COMPUTER_TYPE_TEXT"),
            Some("computer_type_text")
        );
        assert_eq!(computer_tool_name("mycomputer_click"), None);
        assert_eq!(computer_tool_name("computer_clicker"), None);
        assert_eq!(computer_tool_name("read_thread"), None);
    }

    #[test]
    fn calls_are_recognized_on_claude_codex_and_acp_shapes() {
        let claude = mcp("computer_click", json!({ "x": 10, "y": 20 }));
        assert_eq!(computer_call(&claude).unwrap().tool, "computer_click");
        let acp_title = ToolCall::Unknown {
            name: "zeron/computer_press_key".into(),
            input: Some(json!({ "key": "cmd+s" })),
        };
        assert_eq!(describe(&acp_title), "Press Command + S");
        let acp_raw = ToolCall::Unknown {
            name: "Calling MCP tool".into(),
            input: Some(
                json!({ "_toolName": "mcp__zeron__computer_launch_app", "arguments": { "app": "com.apple.Safari" } }),
            ),
        };
        assert_eq!(describe(&acp_raw), "Open Safari");
        let other = mcp("read_thread", json!({}));
        assert!(computer_call(&other).is_none());
        assert!(
            computer_call(&ToolCall::Exec {
                command: "ls".into()
            })
            .is_none()
        );
    }

    #[test]
    fn only_zerons_own_server_is_recognized() {
        let unknown = |name: &str| ToolCall::Unknown {
            name: name.into(),
            input: Some(json!({})),
        };
        for name in ["mcp__zeron__computer_click", "zeron_computer_click"] {
            assert_eq!(
                computer_call(&unknown(name)).unwrap().tool,
                "computer_click"
            );
        }
        let other_server = ToolCall::Mcp {
            server: "other".into(),
            tool: "computer_click".into(),
            input: Some(json!({ "x": 1, "y": 2 })),
        };
        assert!(computer_call(&other_server).is_none());
        for name in [
            "computer_click",
            "mcp__other__computer_click",
            "notzeron_computer_click",
        ] {
            assert!(computer_call(&unknown(name)).is_none(), "{name}");
        }
        let bare_raw = ToolCall::Unknown {
            name: "Calling MCP tool".into(),
            input: Some(json!({ "tool": "computer_launch_app" })),
        };
        assert!(computer_call(&bare_raw).is_none());
    }

    #[test]
    fn summaries_follow_synara_wording() {
        let cases = [
            (
                mcp(
                    "computer_click",
                    json!({ "label": "Save", "app_name": "Notes" }),
                ),
                "Click on “Save” in Notes",
            ),
            (
                mcp("computer_click", json!({ "x": 812, "y": 344 })),
                "Click at (812, 344)",
            ),
            (
                mcp("computer_scroll", json!({ "delta_y": -3 })),
                "Scroll up",
            ),
            (
                mcp("computer_wait", json!({ "duration_ms": 2000 })),
                "Wait for 2 seconds",
            ),
            (
                mcp("computer_invoke_menu", json!({ "path": ["File", "Save"] })),
                "Invoke a menu item File → Save",
            ),
            (
                mcp(
                    "computer_set_window_minimized",
                    json!({ "minimized": false }),
                ),
                "Restore a window",
            ),
            (
                mcp("computer_activate_window", json!({ "app_name": "Safari" })),
                "Switch to Safari",
            ),
            (
                mcp(
                    "computer_browser_navigate",
                    json!({ "url": "https://user:pw@example.com/a?q=1" }),
                ),
                "Open example.com in the browser",
            ),
            (
                mcp("computer_browser_dialog", json!({ "action": "inspect" })),
                "Read a browser dialog",
            ),
            (
                mcp("computer_spaces", json!({ "operation": "reserve" })),
                "Reserve a desktop Space for this task",
            ),
            (
                mcp(
                    "computer_inspect",
                    json!({ "tool": "computer_read_clipboard", "arguments": {} }),
                ),
                "Read the clipboard",
            ),
            (mcp("computer_screenshot", json!({})), "Take a screenshot"),
        ];
        for (call, expected) in cases {
            assert_eq!(describe(&call), expected);
        }
    }

    #[test]
    fn typed_text_and_clipboard_values_never_surface() {
        let secret = "hunter2-secret";
        for (tool, args) in [
            (
                "computer_type_text",
                json!({ "text": secret, "label": "Password" }),
            ),
            ("computer_set_value", json!({ "value": secret })),
            ("computer_write_clipboard", json!({ "text": secret })),
            ("computer_paste", json!({ "text": secret })),
            ("computer_browser_type", json!({ "text": secret })),
            (
                "computer_run",
                json!({ "steps": [{ "type": "type_text", "text": secret }, { "type": "press_key", "key": "return" }] }),
            ),
        ] {
            let call = mcp(tool, args);
            let found = computer_call(&call).unwrap();
            let summary = describe_computer_call(found.tool, found.args);
            let rows = computer_call_rows(found.tool, found.args).join("\n");
            assert!(!summary.contains(secret), "{tool} summary: {summary}");
            assert!(!rows.contains(secret), "{tool} rows: {rows}");
        }
        let run = mcp(
            "computer_run",
            json!({ "steps": [{ "type": "type_text", "text": secret }, { "type": "press_key" }] }),
        );
        let found = computer_call(&run).unwrap();
        assert_eq!(
            computer_call_rows(found.tool, found.args),
            vec!["Steps: Type → Press a key".to_string()]
        );
        assert_eq!(
            describe_computer_call(
                "computer_type_text",
                Some(json!({ "label": "Name" }).as_object().unwrap())
            ),
            "Type in “Name”"
        );
    }

    #[test]
    fn clipboard_reads_hide_their_output() {
        assert!(computer_output_hidden("computer_read_clipboard", None));
        let inspect = json!({ "tool": "computer_read_clipboard" });
        assert!(computer_output_hidden(
            "computer_inspect",
            inspect.as_object()
        ));
        let zoom = json!({ "tool": "computer_zoom" });
        assert!(!computer_output_hidden(
            "computer_inspect",
            zoom.as_object()
        ));
        assert!(!computer_output_hidden("computer_screenshot", None));
    }

    #[test]
    fn typing_results_hide_their_output() {
        for tool in [
            "computer_type_text",
            "computer_set_value",
            "computer_paste",
            "computer_write_clipboard",
            "computer_browser_type",
        ] {
            assert!(computer_output_hidden(tool, None), "{tool}");
        }
        let typing =
            json!({ "steps": [{ "type": "click" }, { "type": "type_text", "text": "x" }] });
        assert!(computer_output_hidden("computer_run", typing.as_object()));
        let clicks = json!({ "steps": [{ "type": "click" }, { "type": "press_key" }] });
        assert!(!computer_output_hidden("computer_run", clicks.as_object()));
        assert!(!computer_output_hidden("computer_click", None));
    }

    #[test]
    fn the_zeron_browser_tool_has_a_title() {
        let call = ToolCall::Unknown {
            name: "mcp__zeron__computer_zeron_browser".into(),
            input: Some(json!({ "action": "navigate" })),
        };
        assert_eq!(computer_call(&call).unwrap().tool, "computer_zeron_browser");
        assert_eq!(describe(&call), "Use the Zeron browser");
    }

    #[test]
    fn the_app_comes_from_the_args_or_a_truncated_output() {
        let args = json!({ "window_id": "cua:1:2", "app_name": "com.mitchellh.ghostty" });
        assert_eq!(
            computer_call_app("computer_click", args.as_object(), None).as_deref(),
            Some("Ghostty")
        );
        let truncated = r#"{"action":"computer_type_text","appName":"Ghostty","computerId":"desktop","disclosure":"Computer control ON for this turn: the agent is driving the desktop and the user can switch …"#;
        let window_only = json!({ "window_id": "cua:1:2", "text": "hi" });
        assert_eq!(
            computer_call_app(
                "computer_type_text",
                window_only.as_object(),
                Some(truncated)
            )
            .as_deref(),
            Some("Ghostty")
        );
        let listing = r#"{"availability":{"backend":"fake","kind":"available"},"computerId":"desktop","windows":[{"appName":"org.kde.konsole","bounds":{"height":720.0,"width":960.0,"x":…"#;
        assert_eq!(
            computer_call_app("computer_list_windows", None, Some(listing)),
            None,
            "a nested window's app is not the call's app"
        );
        let cut_name = r#"{"action":"computer_click","appName":"Ghos…"#;
        assert_eq!(
            computer_call_app("computer_click", None, Some(cut_name)),
            None
        );
    }

    #[test]
    fn computer_rows_use_codex_label_pairs() {
        let label = |tool: &str, args: Value, done: bool| {
            computer_action_label(tool, args.as_object(), Some("Calculator"), done)
        };
        let cases = [
            (
                "computer_click",
                json!({}),
                "Clicking in Calculator",
                "Clicked in Calculator",
            ),
            (
                "computer_get_state",
                json!({}),
                "Looking at Calculator",
                "Looked at Calculator",
            ),
            (
                "computer_screenshot",
                json!({}),
                "Looking at Calculator",
                "Looked at Calculator",
            ),
            (
                "computer_type_text",
                json!({ "text": "12 × 4" }),
                "Typing text “12 × 4” in Calculator",
                "Typed text “12 × 4” in Calculator",
            ),
            (
                "computer_scroll",
                json!({ "delta_y": -3 }),
                "Scrolling up in Calculator",
                "Scrolled up in Calculator",
            ),
            (
                "computer_scroll",
                json!({ "delta_x": 2 }),
                "Scrolling right in Calculator",
                "Scrolled right in Calculator",
            ),
            (
                "computer_set_value",
                json!({ "value": "42" }),
                "Setting to “42” in Calculator",
                "Set to “42” in Calculator",
            ),
            (
                "computer_press_key",
                json!({ "key": "cmd+c" }),
                "Pressing key in Calculator",
                "Pressed key in Calculator",
            ),
            (
                "computer_drag",
                json!({}),
                "Dragging in Calculator",
                "Dragged in Calculator",
            ),
            (
                "computer_perform_action",
                json!({ "action": "AXPress" }),
                "Performing accessibility action in Calculator",
                "Performed accessibility action in Calculator",
            ),
            (
                "computer_list_apps",
                json!({}),
                "List Mac apps",
                "List Mac apps",
            ),
        ];
        for (tool, args, running, done) in cases {
            assert_eq!(label(tool, args.clone(), false), running, "{tool}");
            assert_eq!(label(tool, args, true), done, "{tool}");
        }
        assert_eq!(
            computer_action_label("computer_click", None, None, true),
            "Clicked in App",
            "the fallback app name"
        );
        assert_eq!(
            computer_action_label("computer_click", None, Some("  "), false),
            "Clicking in App"
        );
        assert_eq!(
            label("computer_launch_app", json!({ "app": "Safari" }), true),
            "Open Safari",
            "tools without a Codex pair keep the redacted summary"
        );
    }

    #[test]
    fn typed_detail_is_whitespace_collapsed_and_cut_at_48_characters() {
        let long = "one  two\nthree\tfour five six seven eight nine ten eleven twelve";
        let detail = computer_action_detail(long);
        assert_eq!(detail.chars().count(), 48);
        assert!(detail.ends_with('…'));
        assert_eq!(detail, "one two three four five six seven eight nine te…");
        assert_eq!(computer_action_detail(" short   text "), "short text");
        let exactly = "a".repeat(48);
        assert_eq!(computer_action_detail(&exactly), exactly, "48 fits uncut");
        assert_eq!(
            computer_action_label(
                "computer_type_text",
                json!({ "text": long }).as_object(),
                Some("Notes"),
                false
            ),
            format!("Typing text “{detail}” in Notes")
        );
    }

    #[test]
    fn the_js_computer_tool_is_labelled_by_its_title() {
        let call = mcp(
            "computer",
            json!({ "title": "Add 12 and 30 in Calculator", "code": "await cua.getApp('Calculator')" }),
        );
        let found = computer_call(&call).expect("the JS computer tool is a computer call");
        assert_eq!(found.tool, "computer");
        assert_eq!(
            computer_action_label(found.tool, found.args, None, false),
            "Add 12 and 30 in Calculator"
        );
        let long = "x".repeat(120);
        let titled = json!({ "title": long });
        assert_eq!(
            computer_action_label("computer", titled.as_object(), None, true)
                .chars()
                .count(),
            JS_TITLE_MAX
        );
        assert_eq!(
            computer_action_label("computer", None, None, true),
            "Used the computer"
        );
        assert_eq!(computer_tool_name("mycomputer"), None);
        assert_eq!(
            computer_tool_name("mcp__zeron__computer_click"),
            Some("computer_click")
        );
    }

    #[test]
    fn js_actions_parse_into_codex_rows_and_skip_what_they_cannot_name() {
        let output = json!({
            "appName": "Calculator",
            "actions": [
                { "tool": "click", "app": "Calculator", "n": 3 },
                { "tool": "computer_type_text", "app": "Calculator", "detail": "12+30" },
                { "tool": "scroll", "app": "Safari", "detail": "down" },
                { "tool": "set_value", "detail": "Ada" },
                { "tool": "teleport", "app": "Calculator" },
                { "app": "Calculator" },
                "not an object"
            ]
        })
        .to_string()
            + "\nWindow: \"Calculator\", App: Calculator.";
        let actions = computer_js_actions(&output);
        assert_eq!(
            actions,
            vec![
                ComputerJsAction {
                    tool: "computer_click",
                    app: Some("Calculator".into()),
                    detail: None,
                    count: 3,
                },
                ComputerJsAction {
                    tool: "computer_type_text",
                    app: Some("Calculator".into()),
                    detail: Some("12+30".into()),
                    count: 1,
                },
                ComputerJsAction {
                    tool: "computer_scroll",
                    app: Some("Safari".into()),
                    detail: Some("down".into()),
                    count: 1,
                },
                ComputerJsAction {
                    tool: "computer_set_value",
                    app: Some("Calculator".into()),
                    detail: Some("Ada".into()),
                    count: 1,
                },
            ]
        );
        assert_eq!(
            actions
                .iter()
                .map(|action| computer_js_action_label(action, true))
                .collect::<Vec<_>>(),
            [
                "Clicked in Calculator ×3",
                "Typed text “12+30” in Calculator",
                "Scrolled down in Safari",
                "Set to “Ada” in Calculator",
            ]
        );
        for broken in [
            "",
            "not json",
            "[]",
            r#"{"actions":"nope"}"#,
            r#"{"actions":[{"tool":"cli"#,
        ] {
            assert!(computer_js_actions(broken).is_empty(), "{broken}");
        }
    }

    #[test]
    fn the_redacted_summary_still_never_carries_typed_text() {
        let call = mcp("computer_type_text", json!({ "text": "hunter2-secret" }));
        let found = computer_call(&call).unwrap();
        assert!(!describe_computer_call(found.tool, found.args).contains("hunter2"));
        assert!(
            !computer_call_rows(found.tool, found.args)
                .join(" ")
                .contains("hunter2")
        );
    }
}
