use std::time::Duration;

use serde_json::{Map, Value, json};
use zeron_proto::computer::{
    CuaComputerTask, CuaContent, CuaEffect, CuaPreviewTarget, CuaReply, CuaToolResult,
    cua_computer_task_key,
};

pub use crate::computer::cursor_preference::{CuaCursorStyle, normalize_cua_cursor_style};

pub const CUA_CURSOR_IDLE_HIDE_MS: u64 = 60_000;
pub const CUA_CURSOR_THEME: &str = "zeron.codex";
pub const ESCAPE_INPUT_COOLDOWN: Duration = Duration::from_millis(1_500);
pub(super) const BOUNDED_ENTRIES: usize = 256;
pub(super) const ANONYMOUS_TASK: &str = "anonymous";

pub(super) const CUA_DRIVER_MISSING_MESSAGE: &str = "Cua Driver is not bundled. Build it with `scripts/build-cua-driver.sh` (needs the pinned Rust toolchain) in this checkout, then relaunch Zeron.";

pub(super) fn browser_session_label(thread_id: &str) -> String {
    format!("synara-browser-{thread_id}")
}

const AGENT_SESSION_LABEL_PREFIX: &str = "agent·";
const AGENT_SESSION_LABEL_MAX_CHARS: usize = 120;

// std has no Unicode category lookup; this is \p{Cf} as of Unicode 16, as in zeron_proto's shield label.
fn is_format_char(c: char) -> bool {
    matches!(c as u32,
        0x00AD | 0x0600..=0x0605 | 0x061C | 0x06DD | 0x070F | 0x0890..=0x0891 | 0x08E2
        | 0x180E | 0x200B..=0x200F | 0x202A..=0x202E | 0x2060..=0x2064 | 0x2066..=0x206F
        | 0xFEFF | 0xFFF9..=0xFFFB | 0x110BD | 0x110CD | 0x13430..=0x1343F
        | 0x1BCA0..=0x1BCA3 | 0x1D173..=0x1D17A | 0xE0001 | 0xE0020..=0xE007F)
}

fn agent_badge_component(value: &str) -> String {
    let stripped: String = value
        .chars()
        .filter(|c| !c.is_control() && !is_format_char(*c))
        .collect();
    stripped.trim().to_string()
}

pub(super) fn agent_session_label(task: &CuaComputerTask) -> String {
    let thread_id = Some(agent_badge_component(&task.thread_id))
        .filter(|id| !id.is_empty())
        .unwrap_or_else(|| "task".into());
    let label: String = agent_badge_component(task.label.as_deref().unwrap_or(""))
        .chars()
        .take(AGENT_SESSION_LABEL_MAX_CHARS)
        .collect();
    if label.is_empty() {
        format!("{AGENT_SESSION_LABEL_PREFIX}{thread_id}")
    } else {
        format!("{AGENT_SESSION_LABEL_PREFIX}{label}·{thread_id}")
    }
}

pub(super) fn cursor_style_json(style: Option<&CuaCursorStyle>) -> String {
    style
        .map(|style| serde_json::to_string(style).unwrap_or_default())
        .unwrap_or_default()
}

pub(super) fn cursor_style_args(session: &str, style: Option<&CuaCursorStyle>) -> Value {
    let mut args = Map::new();
    args.insert("session".into(), json!(session));
    if let Some(Value::Object(colors)) = style.map(|s| serde_json::to_value(s).unwrap_or_default())
    {
        args.extend(colors);
    }
    Value::Object(args)
}

pub(super) fn js_safe_integer(value: Option<&Value>) -> Option<i64> {
    const MAX_SAFE_INTEGER: i64 = (1 << 53) - 1;
    let value = value?;
    if let Some(n) = value.as_i64() {
        return (n.abs() <= MAX_SAFE_INTEGER).then_some(n);
    }
    let n = value.as_f64()?;
    (n.fract() == 0.0 && n.abs() <= MAX_SAFE_INTEGER as f64).then_some(n as i64)
}

pub(super) fn safe_native_id(value: Option<&Value>) -> Option<u32> {
    js_safe_integer(value)
        .filter(|v| *v > 0 && *v <= 0xffff_ffff)
        .map(|v| v as u32)
}

pub(super) fn safe_pid(value: Option<&Value>) -> Option<u32> {
    js_safe_integer(value)
        .filter(|v| *v > 0 && *v <= 0x7fff_ffff)
        .map(|v| v as u32)
}

pub(super) fn js_strict_eq(a: Option<&Value>, b: Option<&Value>) -> bool {
    match (a, b) {
        (None, None) => true,
        (Some(Value::Number(a)), Some(Value::Number(b))) => a.as_f64() == b.as_f64(),
        (Some(a), Some(b)) => !a.is_object() && !a.is_array() && a == b,
        _ => false,
    }
}

pub(super) fn structured(reply: &CuaReply) -> Option<&Map<String, Value>> {
    reply.result.as_ref()?.structured_content.as_ref()
}

pub(super) fn is_error(reply: &CuaReply) -> bool {
    reply
        .result
        .as_ref()
        .is_some_and(|r| r.is_error == Some(true))
}

pub(super) const LOGGABLE_CUA_CODES: &[&str] = &[
    "computer_input_paused",
    "desktop_input_paused",
    "same_pid_keyboard_ambiguity",
    "cua_action_failed",
    "cua_refusal",
    "invalid_arguments",
    "input_admission_closed",
    "target_not_on_active_space",
    "target_unavailable",
    "auth_sheet_focused",
    "input_monitor_unavailable",
    "gui_host_required",
    "background_pixel_focus_unavailable",
];

pub(super) fn launch_app_match_names(input: &Value) -> Vec<String> {
    let Some(args) = input.as_object() else {
        return Vec::new();
    };
    let mut names = Vec::new();
    if let Some(name) = args
        .get("name")
        .and_then(Value::as_str)
        .filter(|n| !n.is_empty())
    {
        names.push(name.to_lowercase());
    }
    if let Some(bundle_id) = args
        .get("bundle_id")
        .and_then(Value::as_str)
        .filter(|n| !n.is_empty())
    {
        names.push(bundle_id.to_lowercase());
        if let Some(tail) = bundle_id.split('.').next_back().filter(|t| !t.is_empty()) {
            names.push(tail.to_lowercase());
        }
    }
    names
}

const DRIVER_SESSION_DEATH_CODES: &[&str] = &[
    "session_ended",
    "session-expired",
    "session_expired",
    "unknown_session",
    "session_not_found",
];

pub(super) fn is_driver_session_death(reply: &CuaReply) -> bool {
    if !reply.ok {
        return reply.effect != Some(CuaEffect::DispatchedUnknown)
            && reply
                .error
                .as_deref()
                .is_some_and(|e| e.contains("has ended") && e.contains("start_session"));
    }
    let Some(result) = reply.result.as_ref().filter(|r| r.is_error == Some(true)) else {
        return false;
    };
    let structured = result.structured_content.as_ref();
    if structured
        .and_then(|s| s.get("code"))
        .and_then(Value::as_str)
        .is_some_and(|code| DRIVER_SESSION_DEATH_CODES.contains(&code))
    {
        return true;
    }
    let mut texts: Vec<&str> = result
        .content
        .iter()
        .flatten()
        .filter_map(|part| part.text.as_deref())
        .collect();
    if let Some(message) = structured
        .and_then(|s| s.get("message"))
        .and_then(Value::as_str)
    {
        texts.push(message);
    }
    let joined = texts.join("\n");
    joined.contains("has ended") && joined.contains("start_session")
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ControlledTarget {
    pub pid: u32,
    pub window_id: Option<u32>,
    pub thread_id: Option<String>,
    pub browser_target_id: Option<String>,
    pub browser_tab_id: Option<String>,
}

pub(super) fn target_from_args(
    input: &Value,
    task: Option<&CuaComputerTask>,
) -> Option<ControlledTarget> {
    let args = input.as_object()?;
    let pid = safe_pid(args.get("pid"))?;
    Some(ControlledTarget {
        pid,
        window_id: safe_native_id(args.get("window_id")),
        thread_id: task.map(|t| t.thread_id.clone()),
        browser_target_id: None,
        browser_tab_id: None,
    })
}

pub(super) fn browser_target_key(thread_id: &str, target_id: &str) -> String {
    serde_json::to_string(&(thread_id, target_id)).unwrap_or_default()
}

pub(super) fn is_isolated_browser_setup(name: &str, input: &Value) -> bool {
    if name != "browser_prepare" {
        return false;
    }
    let Some(args) = input.as_object() else {
        return false;
    };
    let mode = args
        .get("profile")
        .and_then(Value::as_object)
        .and_then(|p| p.get("mode"))
        .and_then(Value::as_str);
    args.get("allow_launch") == Some(&Value::Bool(true))
        && !args.contains_key("pid")
        && !args.contains_key("window_id")
        && !args.contains_key("target_id")
        && !args.contains_key("strategy")
        && matches!(mode, Some("isolated_new" | "isolated_named"))
}

pub(super) fn browser_recovery_key(
    input: &Value,
    task: Option<&CuaComputerTask>,
) -> Option<String> {
    let task = task?;
    let args = input.as_object()?;
    let target_id = args
        .get("target_id")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())?;
    let tab_id = args
        .get("tab_id")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())?;
    serde_json::to_string(&(cua_computer_task_key(task), target_id, tab_id)).ok()
}

fn is_snapshot_id(value: &str) -> bool {
    value.len() > 1 && value.starts_with('p') && value[1..].chars().all(|c| c.is_ascii_digit())
}

pub(super) fn is_browser_snapshot(input: &Value, result: &CuaToolResult) -> bool {
    let Some(args) = input.as_object() else {
        return false;
    };
    let Some(data) = result.structured_content.as_ref() else {
        return false;
    };
    let snapshot_id = data.get("snapshot_id").cloned().or_else(|| {
        data.get("snapshot")
            .and_then(Value::as_object)
            .and_then(|s| s.get("id"))
            .cloned()
    });
    let target_id = args
        .get("target_id")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty());
    let tab_id = args
        .get("tab_id")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty());
    data.get("status").and_then(Value::as_str) == Some("ok")
        && data.get("mode").and_then(Value::as_str) == Some("snapshot")
        && target_id.is_some()
        && tab_id.is_some()
        && data.get("target_id").and_then(Value::as_str) == target_id
        && data.get("tab_id").and_then(Value::as_str) == tab_id
        && snapshot_id
            .as_ref()
            .and_then(Value::as_str)
            .is_some_and(is_snapshot_id)
        && data.get("refs").is_some_and(Value::is_array)
}

pub(super) fn observation_matches_target(
    name: &str,
    input: &Value,
    target: &ControlledTarget,
    result: &CuaToolResult,
) -> bool {
    let Some(args) = input.as_object() else {
        return false;
    };
    if let Some(browser_target_id) = &target.browser_target_id {
        if name != "get_browser_state" {
            return false;
        }
        return args.get("target_id").and_then(Value::as_str) == Some(browser_target_id.as_str())
            && target.browser_tab_id.as_ref().is_none_or(|tab| {
                args.get("tab_id").and_then(Value::as_str) == Some(tab.as_str())
            });
    }
    let state = result.structured_content.as_ref();
    let field = |key: &str| state.and_then(|s| s.get(key));
    let target_pid = json!(target.pid);
    let target_window = target.window_id.map(|id| json!(id));
    let exact_window = js_strict_eq(args.get("window_id"), target_window.as_ref());
    let degraded = field("degraded").is_some_and(|d| match d {
        Value::Null | Value::Bool(false) => false,
        Value::String(s) => !s.is_empty(),
        Value::Number(n) => n.as_f64() != Some(0.0),
        _ => true,
    });
    name == "get_window_state"
        && js_strict_eq(args.get("pid"), Some(&target_pid))
        && js_strict_eq(field("pid"), Some(&target_pid))
        && safe_native_id(args.get("window_id")).is_some()
        && js_strict_eq(field("window_id"), args.get("window_id"))
        && !degraded
        && field("screenshot_frame_valid") != Some(&Value::Bool(false))
        && (target.window_id.is_none()
            || exact_window
            || (field("window_is_on_screen") == Some(&Value::Bool(true))
                && field("window_on_current_space") == Some(&Value::Bool(true))
                && field("elements")
                    .and_then(Value::as_array)
                    .is_some_and(|e| !e.is_empty())))
}

pub(super) fn frame_tap_target(task: &CuaComputerTask, input: &Value) -> Option<CuaPreviewTarget> {
    let args = input.as_object()?;
    let pid = safe_pid(args.get("pid"))?;
    let window_id = safe_native_id(args.get("window_id"))?;
    Some(CuaPreviewTarget {
        task: task.clone(),
        pid,
        window_id,
        cursor: None,
    })
}

pub(super) fn not_dispatched(error: impl Into<String>) -> CuaReply {
    CuaReply {
        ok: false,
        error: Some(error.into()),
        effect: Some(CuaEffect::NotDispatched),
        ..CuaReply::default()
    }
}

pub(super) fn ok_reply() -> CuaReply {
    CuaReply {
        ok: true,
        ..CuaReply::default()
    }
}

pub(super) fn ok_with(fields: Value) -> CuaReply {
    CuaReply {
        ok: true,
        result: Some(CuaToolResult {
            extra: fields.as_object().cloned().unwrap_or_default(),
            ..CuaToolResult::default()
        }),
        ..CuaReply::default()
    }
}

fn refusal(message: &str, structured: Value) -> CuaReply {
    CuaReply {
        ok: true,
        result: Some(CuaToolResult {
            is_error: Some(true),
            content: Some(vec![CuaContent {
                kind: "text".into(),
                text: Some(message.into()),
                data: None,
                mime_type: None,
            }]),
            structured_content: structured.as_object().cloned(),
            extra: Map::new(),
        }),
        ..CuaReply::default()
    }
}

pub(super) fn browser_self_target_reply() -> CuaReply {
    let message = "Computer browser calls may never target this application's own processes; the integrated browser is a separate surface.";
    refusal(
        message,
        json!({ "effect": "refused", "code": "browser_self_target", "message": message }),
    )
}

pub(super) fn self_protected_surface_reply() -> CuaReply {
    let message = "Zeron is showing an approval, a question, or its Computer settings or permission controls, so agent input to Zeron's own windows is refused while that is on screen. Only the user may answer approvals or change these settings. Ask the user to answer or leave that page; once they have, read fresh state and retry.";
    refusal(
        message,
        json!({
            "effect": "refused",
            "code": "self_protected_surface",
            "layer": "driver-host",
            "message": message,
        }),
    )
}

pub(super) fn desktop_pause_reply(desktop_paused: bool) -> CuaReply {
    let message = if desktop_paused {
        "Computer input is paused because the desktop is locked, asleep or inactive. Return to the desktop, then read fresh state before continuing."
    } else {
        "Computer input was interrupted or the user changed the controlled window. Read fresh state with computer_get_state on that window, then continue from what it shows; do not replay an uncertain action."
    };
    refusal(
        message,
        json!({
            "effect": "refused",
            "code": "computer_input_paused",
            "layer": "driver-host",
            "message": message,
            "requery_hint": "Call computer_get_state with the affected window_id; that read clears the pause once physical input has stopped. Then continue the task. Do not replay an uncertain action.",
        }),
    )
}

pub(super) fn input_monitor_unavailable_reply(monitor_error: Option<&str>) -> CuaReply {
    let message = if monitor_error == Some("input-monitoring-required") {
        "Allow Input Monitoring in System Settings, then wait for the computer input listener to reconnect before continuing."
    } else {
        "The computer input listener is unavailable. Input remains paused until the listener reconnects."
    };
    let mut structured = json!({
        "effect": "refused",
        "code": "input_monitor_unavailable",
        "message": message,
    });
    if let Some(error) = monitor_error {
        structured["input_monitor_error"] = json!(error);
    }
    refusal(message, structured)
}

pub(super) const USER_INPUT_CAUSE: &str = "user_input";

pub(super) fn user_input_pause_reply() -> CuaReply {
    let message = "The user is using this window. Call computer_get_state on this window; it returns once the user's input goes quiet. Then continue the task from what it shows. Do not ask the user and do not replay the interrupted action.";
    refusal(
        message,
        json!({
            "effect": "refused",
            "code": "computer_input_paused",
            "cause": USER_INPUT_CAUSE,
            "layer": "driver-host",
            "message": message,
            "requery_hint": "Call computer_get_state with the affected window_id now; that read resumes input. Then continue the task. Do not replay the interrupted action.",
        }),
    )
}

pub(super) fn input_interrupted_reply(wait: Duration, user_input: bool) -> CuaReply {
    let message = "Computer input was interrupted by physical input. After wait_seconds, read fresh state with computer_get_state on the affected window, then continue from what it shows. Do not replay an uncertain action.";
    let mut structured = json!({
        "effect": "refused",
        "code": "computer_input_paused",
        "layer": "driver-host",
        "message": message,
        "wait_seconds": wait.as_secs_f64(),
        "requery_hint": "Wait wait_seconds, then call computer_get_state on the affected window; that read resumes input. Waiting alone does not.",
    });
    if user_input {
        structured["cause"] = json!(USER_INPUT_CAUSE);
    }
    refusal(message, structured)
}

pub(super) fn task_stopped_reply() -> CuaReply {
    not_dispatched("The user stopped computer use for this turn. Do not retry actions.")
}

pub(super) fn interrupt_acknowledged(reply: &Value, pid: u32, prior_epoch: u64) -> Option<u64> {
    let state = reply.get("result")?.as_object()?;
    let epoch = js_safe_integer(state.get("input_epoch"))?;
    let confirmed = reply.get("ok") == Some(&Value::Bool(true))
        && js_strict_eq(state.get("pid"), Some(&json!(pid)))
        && state.get("input_interrupted") == Some(&Value::Bool(true))
        && state.get("input_admission_open") == Some(&Value::Bool(true))
        && state.get("cleanup_complete") == Some(&Value::Bool(true))
        && js_strict_eq(state.get("pending_input"), Some(&json!(0)))
        && epoch > prior_epoch as i64;
    confirmed.then_some(epoch as u64)
}

pub(super) struct Bounded<V> {
    entries: Vec<(String, V)>,
}

impl<V> Default for Bounded<V> {
    fn default() -> Self {
        Self {
            entries: Vec::new(),
        }
    }
}

impl<V> Bounded<V> {
    pub fn get(&self, key: &str) -> Option<&V> {
        self.entries.iter().find(|(k, _)| k == key).map(|(_, v)| v)
    }

    pub fn get_mut(&mut self, key: &str) -> Option<&mut V> {
        self.entries
            .iter_mut()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v)
    }

    pub fn contains(&self, key: &str) -> bool {
        self.get(key).is_some()
    }

    pub fn set(&mut self, key: String, value: V) {
        match self.get_mut(&key) {
            Some(slot) => *slot = value,
            None => self.entries.push((key, value)),
        }
        self.trim();
    }

    pub fn touch(&mut self, key: String, value: V) {
        self.remove(&key);
        self.entries.push((key, value));
        self.trim();
    }

    pub fn push_unbounded(&mut self, key: String, value: V) {
        self.remove(&key);
        self.entries.push((key, value));
    }

    pub fn remove(&mut self, key: &str) -> Option<V> {
        let index = self.entries.iter().position(|(k, _)| k == key)?;
        Some(self.entries.remove(index).1)
    }

    pub fn retain(&mut self, mut keep: impl FnMut(&str, &V) -> bool) {
        self.entries.retain(|(k, v)| keep(k, v));
    }

    pub fn clear(&mut self) {
        self.entries.clear();
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn oldest_key(&self) -> Option<String> {
        self.entries.first().map(|(k, _)| k.clone())
    }

    pub fn iter(&self) -> impl Iterator<Item = (&str, &V)> {
        self.entries.iter().map(|(k, v)| (k.as_str(), v))
    }

    fn trim(&mut self) {
        while self.entries.len() > BOUNDED_ENTRIES {
            self.entries.remove(0);
        }
    }
}

impl Bounded<()> {
    pub fn insert(&mut self, key: String) {
        if !self.contains(&key) {
            self.set(key, ());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn task(thread: &str, label: Option<&str>) -> CuaComputerTask {
        CuaComputerTask {
            thread_id: thread.into(),
            turn_id: None,
            label: label.map(Into::into),
        }
    }

    #[test]
    fn agent_labels_lead_with_the_display_label_and_end_with_the_thread() {
        assert_eq!(
            agent_session_label(&task("t-1", Some("Research run"))),
            "agent·Research run·t-1"
        );
        assert_eq!(agent_session_label(&task("t-9", None)), "agent·t-9");
        assert_eq!(
            agent_session_label(&task("t-1", Some("Res\u{0}earch\nrun\u{200B}"))),
            "agent·Researchrun·t-1"
        );
        assert_eq!(agent_session_label(&task("\u{200B}", None)), "agent·task");
        let long = "x".repeat(200);
        assert_eq!(
            agent_session_label(&task("t", Some(&long))),
            format!("agent·{}·t", "x".repeat(120))
        );
    }

    #[test]
    fn cursor_style_args_carry_the_session_and_only_usable_channels() {
        let style = CuaCursorStyle {
            fill: Some("#ABCDEF".into()),
            rim: Some("not-a-color".into()),
            shadow: None,
        };
        assert_eq!(
            cursor_style_args("s", normalize_cua_cursor_style(Some(&style)).as_ref()),
            json!({ "session": "s", "fill": "#abcdef" })
        );
        assert_eq!(cursor_style_args("s", None), json!({ "session": "s" }));
    }

    #[test]
    fn session_death_is_read_from_codes_text_and_transport_errors() {
        let text = |t: &str| CuaReply {
            ok: true,
            result: Some(CuaToolResult {
                is_error: Some(true),
                content: Some(vec![CuaContent {
                    kind: "text".into(),
                    text: Some(t.into()),
                    data: None,
                    mime_type: None,
                }]),
                ..CuaToolResult::default()
            }),
            ..CuaReply::default()
        };
        assert!(is_driver_session_death(&text(
            "session 'x' has ended; Call start_session with this id"
        )));
        assert!(!is_driver_session_death(&text("session 'x' has ended")));
        let coded = CuaReply {
            ok: true,
            result: Some(CuaToolResult {
                is_error: Some(true),
                structured_content: json!({ "code": "unknown_session" }).as_object().cloned(),
                ..CuaToolResult::default()
            }),
            ..CuaReply::default()
        };
        assert!(is_driver_session_death(&coded));
        let transport = |effect| CuaReply {
            ok: false,
            error: Some("session 'x' has ended; call start_session".into()),
            effect: Some(effect),
            ..CuaReply::default()
        };
        assert!(is_driver_session_death(&transport(
            CuaEffect::NotDispatched
        )));
        assert!(!is_driver_session_death(&transport(
            CuaEffect::DispatchedUnknown
        )));
    }

    #[test]
    fn launch_names_include_the_bundle_tail() {
        assert_eq!(
            launch_app_match_names(&json!({ "bundle_id": "com.apple.Calculator" })),
            vec!["com.apple.calculator", "calculator"]
        );
        assert_eq!(
            launch_app_match_names(&json!({ "name": "Calculator" })),
            vec!["calculator"]
        );
        assert!(launch_app_match_names(&json!("Calculator")).is_empty());
    }

    #[test]
    fn interrupt_acks_need_all_six_fields_and_a_newer_epoch() {
        let ack = json!({
            "ok": true,
            "result": {
                "pid": 7,
                "input_interrupted": true,
                "input_admission_open": true,
                "cleanup_complete": true,
                "pending_input": 0,
                "input_epoch": 3,
            }
        });
        assert_eq!(interrupt_acknowledged(&ack, 7, 2), Some(3));
        assert_eq!(interrupt_acknowledged(&ack, 7, 3), None);
        assert_eq!(interrupt_acknowledged(&ack, 8, 2), None);
        for (key, value) in [
            ("input_interrupted", json!(false)),
            ("input_admission_open", Value::Null),
            ("cleanup_complete", json!(false)),
            ("pending_input", json!(1)),
            ("input_epoch", json!(3.5)),
        ] {
            let mut bad = ack.clone();
            bad["result"][key] = value;
            assert_eq!(interrupt_acknowledged(&bad, 7, 2), None, "{key}");
        }
        let mut refused = ack.clone();
        refused["ok"] = json!(false);
        assert_eq!(interrupt_acknowledged(&refused, 7, 2), None);
    }

    #[test]
    fn bounded_maps_evict_the_oldest_and_touch_moves_to_the_end() {
        let mut map = Bounded::default();
        for i in 0..=BOUNDED_ENTRIES {
            map.set(i.to_string(), i);
        }
        assert_eq!(map.len(), BOUNDED_ENTRIES);
        assert!(!map.contains("0"));
        map.set("1".into(), 99);
        assert_eq!(map.oldest_key().as_deref(), Some("1"));
        map.touch("1".into(), 100);
        assert_eq!(map.oldest_key().as_deref(), Some("2"));
    }
}
