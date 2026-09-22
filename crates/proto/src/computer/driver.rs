use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use super::bounds::{js_len, js_slice, safe_integer};

pub const CUA_DRIVER_VERSION: &str = "0.28.2";
pub const CUA_NATIVE_REVISION: u64 = 39;
pub const CUA_DRIVER_SOURCE: &str = "7fe7c33f741ee2dd5961ba80044d59f93b48ba47";
pub const CUA_DRIVER_ARCHIVE_SHA256: &str =
    "d0c276a933465e70765d60e1c244d803276b5b5805e6b9e4311841169620470e";
pub const CUA_SETUP_TIMEOUT_MS: u64 = 120_000;
pub const CUA_MAX_REQUEST_BYTES: usize = 1024 * 1024;
pub const CUA_MAX_RESPONSE_BYTES: usize = 96 * 1024 * 1024;
pub const CUA_DEFAULT_TIMEOUT_MS: u64 = 15_000;

pub const CUA_SHIELD_ID_MAX_LENGTH: usize = 64;
pub const CUA_SHIELD_MAX_EXTENT: f64 = 32_768.0;
pub const CUA_SHIELD_MAX_COORDINATE: f64 = 1_000_000.0;
const CUA_SHIELD_LABEL_MAX_LENGTH: usize = 160;
const CUA_TASK_IDENTIFIER_MAX_LENGTH: usize = 256;
const CUA_TASK_LABEL_MAX_LENGTH: usize = 160;

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CuaComputerTask {
    pub thread_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turn_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct CuaPoint {
    pub x: f64,
    pub y: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CuaPreviewTarget {
    pub task: CuaComputerTask,
    pub pid: u32,
    pub window_id: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cursor: Option<CuaPoint>,
}

pub fn cua_computer_task_key(task: &CuaComputerTask) -> String {
    serde_json::to_string(&(&task.thread_id, &task.turn_id)).unwrap_or_default()
}

pub fn is_cua_shield_id(value: &str) -> bool {
    super::bounds::chars_in(value, 1, CUA_SHIELD_ID_MAX_LENGTH, |c| {
        c.is_ascii_alphanumeric() || c == '_' || c == '-'
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct CuaShieldFrame {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum CuaShieldArgs {
    Engage {
        shield_id: String,
        frame: CuaShieldFrame,
        window_id: u32,
        pid: u32,
        #[serde(skip_serializing_if = "Option::is_none")]
        label: Option<String>,
    },
    Release {
        shield_id: String,
    },
    ReleaseAll,
}

fn shield_coordinate(value: Option<&Value>) -> Option<f64> {
    value
        .and_then(Value::as_f64)
        .filter(|v| v.is_finite() && v.abs() <= CUA_SHIELD_MAX_COORDINATE)
}

fn shield_extent(value: Option<&Value>) -> Option<f64> {
    value
        .and_then(Value::as_f64)
        .filter(|v| v.is_finite() && *v > 0.0 && *v <= CUA_SHIELD_MAX_EXTENT)
}

// Rust's std has no Unicode general-category lookup; this is \p{Cf} as of Unicode 16.
fn is_format_char(c: char) -> bool {
    matches!(c as u32,
        0x00AD | 0x0600..=0x0605 | 0x061C | 0x06DD | 0x070F | 0x0890..=0x0891 | 0x08E2
        | 0x180E | 0x200B..=0x200F | 0x202A..=0x202E | 0x2060..=0x2064 | 0x2066..=0x206F
        | 0xFEFF | 0xFFF9..=0xFFFB | 0x110BD | 0x110CD | 0x13430..=0x1343F
        | 0x1BCA0..=0x1BCA3 | 0x1D173..=0x1D17A | 0xE0001 | 0xE0020..=0xE007F)
}

pub fn parse_cua_shield_args(value: &Value) -> Option<CuaShieldArgs> {
    let args = value.as_object()?;
    let action = args.get("action").and_then(Value::as_str);
    if action == Some("release_all") {
        return Some(CuaShieldArgs::ReleaseAll);
    }
    let shield_id = args
        .get("shield_id")
        .and_then(Value::as_str)
        .filter(|id| is_cua_shield_id(id))?
        .to_string();
    match action {
        Some("release") => return Some(CuaShieldArgs::Release { shield_id }),
        Some("engage") => {}
        _ => return None,
    }
    let frame = args.get("frame").and_then(Value::as_object);
    let x = shield_coordinate(frame.and_then(|f| f.get("x")))?;
    let y = shield_coordinate(frame.and_then(|f| f.get("y")))?;
    let width = shield_extent(frame.and_then(|f| f.get("width")))?;
    let height = shield_extent(frame.and_then(|f| f.get("height")))?;
    let window_id = args
        .get("window_id")
        .and_then(safe_integer)
        .filter(|id| *id > 0 && *id <= 0xffff_ffff)?;
    let pid = args
        .get("pid")
        .and_then(safe_integer)
        .filter(|pid| *pid > 0 && *pid <= 0x7fff_ffff)?;
    let label = args.get("label").and_then(Value::as_str).and_then(|label| {
        let stripped: String = label
            .chars()
            .filter(|c| !c.is_control() && !is_format_char(*c))
            .collect();
        let label = js_slice(stripped.trim(), CUA_SHIELD_LABEL_MAX_LENGTH);
        (!label.is_empty()).then_some(label)
    });
    Some(CuaShieldArgs::Engage {
        shield_id,
        frame: CuaShieldFrame {
            x,
            y,
            width,
            height,
        },
        window_id: window_id as u32,
        pid: pid as u32,
        label,
    })
}

pub fn parse_cua_computer_task(value: &Value) -> Option<CuaComputerTask> {
    let task = value.as_object()?;
    let identifier = |v: &Value| {
        v.as_str()
            .filter(|s| !s.is_empty() && js_len(s) <= CUA_TASK_IDENTIFIER_MAX_LENGTH)
            .map(str::to_string)
    };
    let thread_id = identifier(task.get("threadId")?)?;
    let turn_id = match task.get("turnId") {
        None => None,
        Some(turn) => Some(identifier(turn)?),
    };
    let label = task
        .get("label")
        .and_then(Value::as_str)
        .map(|label| js_slice(label, CUA_TASK_LABEL_MAX_LENGTH));
    Some(CuaComputerTask {
        thread_id,
        turn_id,
        label,
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CuaEffect {
    NotDispatched,
    DispatchedUnknown,
    Verified,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CuaTransportError {
    pub message: String,
    pub effect: CuaEffect,
}

impl std::fmt::Display for CuaTransportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for CuaTransportError {}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CuaContent {
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mime_type: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CuaToolResult {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content: Option<Vec<CuaContent>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub is_error: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub structured_content: Option<Map<String, Value>>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct CuaCleanupAcknowledgement {
    pub pid: u32,
    pub input_admission_closed: bool,
    pub cleanup_complete: bool,
    pub pending_input: u64,
}

pub fn cua_cleanup_acknowledged(result: Option<&Value>, expected_pid: Option<u32>) -> bool {
    let (Some(expected_pid), Some(cleanup)) = (expected_pid, result.and_then(Value::as_object))
    else {
        return false;
    };
    cleanup.get("pid").and_then(Value::as_f64) == Some(f64::from(expected_pid))
        && cleanup.get("input_admission_closed") == Some(&Value::Bool(true))
        && cleanup.get("cleanup_complete") == Some(&Value::Bool(true))
        && cleanup.get("pending_input").and_then(Value::as_f64) == Some(0.0)
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CuaReply {
    pub ok: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub desktop_epoch: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub desktop_pauses: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub desktop_interruptions: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub driver_native_revision: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub driver_browser_input_control: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub host_platform: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<CuaToolResult>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effect: Option<CuaEffect>,
}

pub const CUA_READ_TOOLS: &[&str] = &[
    "check_permissions",
    "check_input_ready",
    "list_windows",
    "list_spaces",
    "list_apps",
    "get_window_state",
    "get_screen_size",
    "get_desktop_state",
    "get_accessibility_tree",
    "get_agent_cursor_state",
    "get_cursor_position",
    "verify_state",
    "wait_for_settle",
    "zoom",
];

pub const CUA_ACTION_TOOLS: &[&str] = &[
    "click",
    "move_cursor",
    "drag",
    "scroll",
    "type_text",
    "press_key",
    "hotkey",
    "set_value",
    "select_text",
    "clipboard_read",
    "clipboard_write",
    "launch_app",
    "bring_to_front",
    "invoke_menu",
    "set_window_frame",
    "set_window_minimized",
    "set_app_visibility",
    "kill_app",
];

pub const CUA_BROWSER_TOOLS: &[&str] = &[
    "get_browser_state",
    "browser_prepare",
    "browser_navigate",
    "browser_click",
    "browser_type",
    "browser_dialog",
    "browser_set_input_files",
    "browser_download",
    "browser_pointer",
];

pub const CUA_BROWSER_MUTATION_TOOLS: &[&str] = &[
    "browser_prepare",
    "browser_navigate",
    "browser_click",
    "browser_type",
    "browser_dialog",
    "browser_set_input_files",
    "browser_download",
    "browser_pointer",
];

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const REGISTERED_MACOS_TOOLS: &[&str] = &[
        "list_windows",
        "list_spaces",
        "list_apps",
        "get_window_state",
        "get_desktop_state",
        "get_screen_size",
        "get_accessibility_tree",
        "get_cursor_position",
        "verify_state",
        "zoom",
        "click",
        "move_cursor",
        "drag",
        "scroll",
        "type_text",
        "press_key",
        "hotkey",
        "set_value",
        "select_text",
        "clipboard_read",
        "clipboard_write",
        "launch_app",
        "bring_to_front",
        "invoke_menu",
        "set_window_frame",
        "kill_app",
        "set_app_visibility",
        "set_window_minimized",
        "wait_for_settle",
        "double_click",
        "right_click",
        "check_permissions",
        "check_input_ready",
        "get_agent_cursor_state",
        "start_session",
        "end_session",
        "escalate_session",
        "get_session",
        "list_sessions",
        "get_session_state",
        "set_agent_cursor_enabled",
        "set_agent_cursor_motion",
        "set_agent_cursor_theme",
        "get_config",
        "set_config",
        "start_recording",
        "stop_recording",
        "get_recording_state",
        "replay_trajectory",
        "install_ffmpeg",
        "health_report",
        "check_for_update",
        "history_status",
        "history_query",
        "page",
        "get_browser_state",
        "browser_prepare",
        "browser_navigate",
        "browser_click",
        "browser_type",
        "browser_dialog",
        "browser_set_input_files",
        "browser_download",
        "browser_pointer",
    ];

    fn sorted(names: &[&'static str]) -> Vec<&'static str> {
        let mut names = names.to_vec();
        names.sort_unstable();
        names
    }

    #[test]
    fn allowlists_exactly_the_audited_tool_set() {
        assert_eq!(
            sorted(CUA_READ_TOOLS),
            sorted(&[
                "check_input_ready",
                "check_permissions",
                "get_accessibility_tree",
                "get_agent_cursor_state",
                "get_cursor_position",
                "get_desktop_state",
                "get_screen_size",
                "get_window_state",
                "list_apps",
                "list_spaces",
                "list_windows",
                "verify_state",
                "wait_for_settle",
                "zoom",
            ])
        );
        assert_eq!(
            sorted(CUA_ACTION_TOOLS),
            sorted(&[
                "bring_to_front",
                "click",
                "clipboard_read",
                "clipboard_write",
                "drag",
                "hotkey",
                "invoke_menu",
                "kill_app",
                "launch_app",
                "move_cursor",
                "press_key",
                "scroll",
                "select_text",
                "set_app_visibility",
                "set_value",
                "set_window_minimized",
                "set_window_frame",
                "type_text",
            ])
        );
    }

    #[test]
    fn keeps_every_host_internal_and_browser_family_driver_tool_unreachable() {
        let unreachable: Vec<&str> = REGISTERED_MACOS_TOOLS
            .iter()
            .copied()
            .filter(|name| !CUA_READ_TOOLS.contains(name) && !CUA_ACTION_TOOLS.contains(name))
            .collect();
        assert_eq!(
            sorted(&unreachable),
            sorted(&[
                "double_click",
                "right_click",
                "start_session",
                "end_session",
                "escalate_session",
                "get_session",
                "list_sessions",
                "get_session_state",
                "set_agent_cursor_enabled",
                "set_agent_cursor_motion",
                "set_agent_cursor_theme",
                "get_config",
                "set_config",
                "start_recording",
                "stop_recording",
                "get_recording_state",
                "replay_trajectory",
                "install_ffmpeg",
                "health_report",
                "check_for_update",
                "history_status",
                "history_query",
                "page",
                "get_browser_state",
                "browser_prepare",
                "browser_navigate",
                "browser_click",
                "browser_type",
                "browser_dialog",
                "browser_set_input_files",
                "browser_download",
                "browser_pointer",
            ])
        );
    }

    #[test]
    fn admits_the_browser_family_only_through_its_own_tool_set() {
        assert_eq!(
            sorted(CUA_BROWSER_TOOLS),
            sorted(&[
                "get_browser_state",
                "browser_prepare",
                "browser_navigate",
                "browser_click",
                "browser_type",
                "browser_dialog",
                "browser_set_input_files",
                "browser_download",
                "browser_pointer",
            ])
        );
        let without_state: Vec<&str> = CUA_BROWSER_TOOLS
            .iter()
            .copied()
            .filter(|name| *name != "get_browser_state")
            .collect();
        assert_eq!(sorted(CUA_BROWSER_MUTATION_TOOLS), sorted(&without_state));
        for name in CUA_BROWSER_TOOLS {
            assert!(!CUA_READ_TOOLS.contains(name));
            assert!(!CUA_ACTION_TOOLS.contains(name));
        }
    }

    #[test]
    fn admits_only_names_the_macos_driver_actually_registers() {
        for name in CUA_READ_TOOLS
            .iter()
            .chain(CUA_ACTION_TOOLS)
            .chain(CUA_BROWSER_TOOLS)
        {
            assert!(REGISTERED_MACOS_TOOLS.contains(name), "{name}");
        }
    }

    const PID: u32 = 4242;

    fn complete() -> Value {
        json!({
            "pid": PID,
            "input_admission_closed": true,
            "cleanup_complete": true,
            "pending_input": 0,
        })
    }

    fn with(key: &str, value: Value) -> Value {
        let mut ack = complete();
        ack[key] = value;
        ack
    }

    #[test]
    fn accepts_only_the_complete_acknowledgement_for_the_spawned_child() {
        assert!(cua_cleanup_acknowledged(Some(&complete()), Some(PID)));
        let typed: CuaCleanupAcknowledgement = serde_json::from_value(complete()).unwrap();
        assert!(cua_cleanup_acknowledged(
            Some(&serde_json::to_value(typed).unwrap()),
            Some(PID)
        ));
    }

    #[test]
    fn rejects_every_incomplete_acknowledgement() {
        let cases = [
            ("a different pid", Some(with("pid", json!(PID + 1)))),
            (
                "admission still open",
                Some(with("input_admission_closed", json!(false))),
            ),
            (
                "cleanup incomplete",
                Some(with("cleanup_complete", json!(false))),
            ),
            ("pending input", Some(with("pending_input", json!(1)))),
            (
                "a missing admission flag",
                Some(json!({ "pid": PID, "cleanup_complete": true, "pending_input": 0 })),
            ),
            ("a success string", Some(json!("cleanup done"))),
            ("an empty result", Some(json!({}))),
            ("no result", None),
        ];
        for (label, result) in cases {
            assert!(
                !cua_cleanup_acknowledged(result.as_ref(), Some(PID)),
                "{label}"
            );
        }
    }

    #[test]
    fn rejects_an_unknown_child_pid() {
        assert!(!cua_cleanup_acknowledged(Some(&complete()), None));
    }

    fn engage() -> Value {
        json!({
            "action": "engage",
            "shield_id": "shield-a1b2c3d4",
            "frame": { "x": 1050.5, "y": 120, "width": 420, "height": 620 },
            "window_id": 4242,
            "pid": 777,
            "label": "Synara activating Calculator",
        })
    }

    fn engage_with(key: &str, value: Value) -> Value {
        let mut args = engage();
        args[key] = value;
        args
    }

    #[test]
    fn parses_an_engage_with_its_full_target_shape() {
        assert_eq!(
            parse_cua_shield_args(&engage()),
            Some(CuaShieldArgs::Engage {
                shield_id: "shield-a1b2c3d4".into(),
                frame: CuaShieldFrame {
                    x: 1050.5,
                    y: 120.0,
                    width: 420.0,
                    height: 620.0,
                },
                window_id: 4242,
                pid: 777,
                label: Some("Synara activating Calculator".into()),
            })
        );
    }

    #[test]
    fn serialized_engage_parses_back_to_itself() {
        let parsed = parse_cua_shield_args(&engage()).unwrap();
        let wire = serde_json::to_value(&parsed).unwrap();
        assert_eq!(wire["action"], "engage");
        assert_eq!(wire["shield_id"], "shield-a1b2c3d4");
        assert_eq!(parse_cua_shield_args(&wire), Some(parsed));
    }

    #[test]
    fn tolerates_a_missing_label_and_an_off_screen_frame_origin() {
        let mut without_label = engage();
        without_label.as_object_mut().unwrap().remove("label");
        match parse_cua_shield_args(&without_label) {
            Some(CuaShieldArgs::Engage {
                shield_id, label, ..
            }) => {
                assert_eq!(shield_id, "shield-a1b2c3d4");
                assert_eq!(label, None);
            }
            other => panic!("expected engage, got {other:?}"),
        }
        let off_screen = engage_with(
            "frame",
            json!({ "x": -4600, "y": -30, "width": 800, "height": 600 }),
        );
        match parse_cua_shield_args(&off_screen) {
            Some(CuaShieldArgs::Engage { frame, .. }) => assert_eq!(
                frame,
                CuaShieldFrame {
                    x: -4600.0,
                    y: -30.0,
                    width: 800.0,
                    height: 600.0,
                }
            ),
            other => panic!("expected engage, got {other:?}"),
        }
    }

    #[test]
    fn sanitizes_the_painted_label() {
        let label = format!(" line\nbreak\u{1b} {}", "x".repeat(300));
        let Some(CuaShieldArgs::Engage {
            label: Some(label), ..
        }) = parse_cua_shield_args(&engage_with("label", json!(label)))
        else {
            panic!("expected an engage with a label");
        };
        assert!(!label.contains('\n'));
        assert!(!label.contains('\u{1b}'));
        assert!(label.encode_utf16().count() <= 160);
    }

    #[test]
    fn strips_format_characters_and_drops_an_empty_label() {
        let Some(CuaShieldArgs::Engage { label, .. }) =
            parse_cua_shield_args(&engage_with("label", json!("\u{200b}Cal\u{202e}c")))
        else {
            panic!("expected engage");
        };
        assert_eq!(label.as_deref(), Some("Calc"));
        let Some(CuaShieldArgs::Engage { label, .. }) =
            parse_cua_shield_args(&engage_with("label", json!(" \u{200b}\n ")))
        else {
            panic!("expected engage");
        };
        assert_eq!(label, None);
    }

    #[test]
    fn refuses_malformed_shield_args() {
        let cases = [
            ("missing args", Value::Null),
            ("non-object args", json!("engage")),
            ("unknown action", json!({ "action": "expand" })),
            (
                "bad shield id characters",
                engage_with("shield_id", json!("shield has spaces")),
            ),
            ("empty shield id", engage_with("shield_id", json!(""))),
            (
                "oversized shield id",
                engage_with("shield_id", json!("s".repeat(65))),
            ),
            ("missing frame", engage_with("frame", Value::Null)),
            (
                "zero-area frame",
                engage_with(
                    "frame",
                    json!({ "x": 0, "y": 0, "width": 0, "height": 600 }),
                ),
            ),
            (
                "absurd extent",
                engage_with(
                    "frame",
                    json!({ "x": 0, "y": 0, "width": 100_000, "height": 600 }),
                ),
            ),
            (
                "non-finite origin",
                engage_with(
                    "frame",
                    json!({ "x": f64::NAN, "y": 0, "width": 800, "height": 600 }),
                ),
            ),
            ("zero window id", engage_with("window_id", json!(0))),
            ("fractional window id", engage_with("window_id", json!(1.5))),
            ("zero pid", engage_with("pid", json!(0))),
            ("negative pid", engage_with("pid", json!(-4))),
        ];
        for (name, args) in cases {
            assert_eq!(parse_cua_shield_args(&args), None, "{name}");
        }
    }

    #[test]
    fn parses_release_and_release_all() {
        assert_eq!(
            parse_cua_shield_args(&json!({ "action": "release", "shield_id": "shield-1" })),
            Some(CuaShieldArgs::Release {
                shield_id: "shield-1".into()
            })
        );
        assert_eq!(
            parse_cua_shield_args(&json!({ "action": "release_all" })),
            Some(CuaShieldArgs::ReleaseAll)
        );
        assert_eq!(
            parse_cua_shield_args(&json!({ "action": "release_all", "shield_id": 42 })),
            Some(CuaShieldArgs::ReleaseAll)
        );
    }

    #[test]
    fn refuses_release_without_a_valid_shield_id() {
        assert_eq!(parse_cua_shield_args(&json!({ "action": "release" })), None);
        assert_eq!(
            parse_cua_shield_args(&json!({ "action": "release", "shield_id": "" })),
            None
        );
    }

    #[test]
    fn parses_task_attribution_within_its_bounds() {
        assert_eq!(
            parse_cua_computer_task(
                &json!({ "threadId": "t1", "turnId": "u1", "label": "x".repeat(200) })
            ),
            Some(CuaComputerTask {
                thread_id: "t1".into(),
                turn_id: Some("u1".into()),
                label: Some("x".repeat(160)),
            })
        );
        assert_eq!(
            parse_cua_computer_task(&json!({ "threadId": "t1" })),
            Some(CuaComputerTask {
                thread_id: "t1".into(),
                turn_id: None,
                label: None,
            })
        );
        for bad in [
            Value::Null,
            json!({}),
            json!({ "threadId": "" }),
            json!({ "threadId": "t".repeat(257) }),
            json!({ "threadId": "t1", "turnId": "" }),
            json!({ "threadId": "t1", "turnId": null }),
            json!({ "threadId": 7 }),
        ] {
            assert_eq!(parse_cua_computer_task(&bad), None, "{bad}");
        }
    }

    #[test]
    fn task_key_matches_the_json_array_form() {
        let task = CuaComputerTask {
            thread_id: "t1".into(),
            turn_id: None,
            label: Some("ignored".into()),
        };
        assert_eq!(cua_computer_task_key(&task), r#"["t1",null]"#);
        let task = CuaComputerTask {
            turn_id: Some("u1".into()),
            ..task
        };
        assert_eq!(cua_computer_task_key(&task), r#"["t1","u1"]"#);
    }

    #[test]
    fn reply_round_trips_every_field_and_keeps_unknown_result_fields() {
        let wire = json!({
            "ok": true,
            "desktopEpoch": 3,
            "desktopPauses": ["screen-lock"],
            "desktopInterruptions": 2,
            "driverNativeRevision": 39,
            "driverBrowserInputControl": false,
            "hostPlatform": "darwin",
            "result": {
                "content": [{ "type": "image", "data": "AAAA", "mimeType": "image/png" }],
                "isError": false,
                "structuredContent": { "effect": "verified" },
                "pid": 4242,
                "input_admission_closed": true,
                "cleanup_complete": true,
                "pending_input": 0,
            },
            "error": "none",
            "effect": "dispatched-unknown",
        });
        let reply: CuaReply = serde_json::from_value(wire.clone()).unwrap();
        assert_eq!(reply.effect, Some(CuaEffect::DispatchedUnknown));
        assert_eq!(serde_json::to_value(&reply).unwrap(), wire);
        assert!(cua_cleanup_acknowledged(
            Some(&serde_json::to_value(reply.result).unwrap()),
            Some(PID)
        ));
        assert_eq!(
            serde_json::to_value(CuaReply {
                ok: false,
                ..CuaReply::default()
            })
            .unwrap(),
            json!({ "ok": false })
        );
    }
}
