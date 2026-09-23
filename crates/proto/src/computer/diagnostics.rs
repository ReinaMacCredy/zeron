use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::Value;

use super::bounds::safe_integer;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CuaDeliveryPath {
    Ax,
    Pixel,
    Semantic,
    Keyboard,
    Menu,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CuaActuator {
    AxPress,
    AxFocus,
    AxAction,
    PixelClick,
    PixelSelection,
    CgeventWheel,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CuaFocusMutation {
    None,
    WithoutRaise,
    Foreground,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CuaRestoreStatus {
    NotNeeded,
    Restored,
    Failed,
    Unobservable,
    UserChanged,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CuaObservation {
    FrameUnchanged,
    FreshFrame,
    TargetWindowClosed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CuaActionErrorCode {
    AxActionRefused,
    AxActionUncertain,
    AxDispatchFailed,
    PixelDispatchFailed,
    ActionTaskFailed,
    FocusRestoreUnavailable,
    FocusRestoreFailed,
}

impl CuaActionErrorCode {
    pub fn message(self) -> &'static str {
        match self {
            Self::AxActionRefused => "The accessibility action was refused before dispatch.",
            Self::AxActionUncertain => {
                "The accessibility action was submitted but its result is uncertain."
            }
            Self::AxDispatchFailed => "The accessibility actuator reported a native error.",
            Self::PixelDispatchFailed => "The pointer actuator reported a native error.",
            Self::ActionTaskFailed => "The native action task failed.",
            Self::FocusRestoreUnavailable => {
                "The previous key window could not be identified safely."
            }
            Self::FocusRestoreFailed => {
                "The previous key window could not be restored and verified."
            }
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct CuaActionDiagnostics {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delivery_path: Option<CuaDeliveryPath>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actuator: Option<CuaActuator>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub focus_mutation: Option<CuaFocusMutation>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub restore_status: Option<CuaRestoreStatus>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_code: Option<CuaActionErrorCode>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub observation: Option<CuaObservation>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scroll_delta_y: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ax_error: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prior_pid: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prior_window_id: Option<u32>,
}

fn member<T: DeserializeOwned>(value: Option<&Value>) -> Option<T> {
    value
        .filter(|v| v.is_string())
        .and_then(|v| serde_json::from_value(v.clone()).ok())
}

fn integer(value: Option<&Value>, min: i64, max: i64) -> Option<i64> {
    value
        .and_then(safe_integer)
        .filter(|v| *v >= min && *v <= max)
}

pub fn parse_cua_action_diagnostics(value: &Value) -> Option<CuaActionDiagnostics> {
    let source = value.as_object()?.get("diagnostics")?.as_object()?;
    let diagnostics = CuaActionDiagnostics {
        delivery_path: member(source.get("delivery_path")),
        actuator: member(source.get("actuator")),
        focus_mutation: member(source.get("focus_mutation")),
        restore_status: member(source.get("restore_status")),
        error_code: member(source.get("error_code")),
        observation: member(source.get("observation")),
        scroll_delta_y: source
            .get("scroll_delta_y")
            .and_then(Value::as_f64)
            .filter(|v| v.is_finite() && v.abs() <= 1_000_000.0),
        ax_error: integer(source.get("ax_error"), -0x8000_0000, 0x7fff_ffff).map(|v| v as i32),
        prior_pid: integer(source.get("prior_pid"), 1, 0x7fff_ffff).map(|v| v as u32),
        prior_window_id: integer(source.get("prior_window_id"), 1, 0xffff_ffff).map(|v| v as u32),
    };
    (diagnostics != CuaActionDiagnostics::default()).then_some(diagnostics)
}

pub fn cua_action_diagnostic_message(diagnostics: &CuaActionDiagnostics) -> Option<&'static str> {
    diagnostics.error_code.map(CuaActionErrorCode::message)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn keeps_finite_measured_scroll_evidence_without_trusting_prose() {
        assert_eq!(
            parse_cua_action_diagnostics(&json!({
                "diagnostics": { "scroll_delta_y": -120.5, "observation": "fresh-frame", "message": "private" },
            })),
            Some(CuaActionDiagnostics {
                scroll_delta_y: Some(-120.5),
                observation: Some(CuaObservation::FreshFrame),
                ..Default::default()
            })
        );
        assert_eq!(
            parse_cua_action_diagnostics(&json!({
                "diagnostics": { "scroll_delta_y": 1e300 * 10.0, "observation": "private window title" },
            })),
            None
        );
    }

    #[test]
    fn retains_actionable_native_evidence_without_app_content_or_raw_error_messages() {
        let diagnostics = parse_cua_action_diagnostics(&json!({
            "message": "secret selected text",
            "diagnostics": {
                "delivery_path": "ax",
                "actuator": "ax_press",
                "focus_mutation": "none",
                "restore_status": "not-needed",
                "error_code": "ax_dispatch_failed",
                "ax_error": -25204,
                "prior_pid": 123,
                "prior_window_id": 456,
                "message": "secret selected text",
                "title": "Private window",
                "value": "password",
            },
        }))
        .unwrap();
        assert_eq!(
            serde_json::to_value(&diagnostics).unwrap(),
            json!({
                "delivery_path": "ax",
                "actuator": "ax_press",
                "focus_mutation": "none",
                "restore_status": "not-needed",
                "error_code": "ax_dispatch_failed",
                "ax_error": -25204,
                "prior_pid": 123,
                "prior_window_id": 456,
            })
        );
        assert_eq!(
            cua_action_diagnostic_message(&diagnostics),
            Some("The accessibility actuator reported a native error.")
        );
    }

    #[test]
    fn drops_malformed_or_unrecognized_diagnostic_content() {
        for value in [
            Value::Null,
            json!([]),
            json!({}),
            json!({ "diagnostics": [] }),
            json!({
                "diagnostics": {
                    "actuator": "private field",
                    "error_code": "secret_token",
                    "delivery_path": "arbitrary",
                    "restore_status": "secret",
                    "focus_mutation": "password",
                    "ax_error": f64::NAN,
                    "prior_pid": -1,
                    "prior_window_id": 1u64 << 40,
                },
            }),
        ] {
            assert_eq!(parse_cua_action_diagnostics(&value), None, "{value}");
        }
    }

    #[test]
    fn keeps_useful_partial_evidence_and_rejects_noninteger_identity_values() {
        assert_eq!(
            parse_cua_action_diagnostics(&json!({
                "diagnostics": {
                    "restore_status": "failed",
                    "prior_pid": 1.5,
                    "prior_window_id": "12",
                    "ax_error": f64::INFINITY,
                },
            })),
            Some(CuaActionDiagnostics {
                restore_status: Some(CuaRestoreStatus::Failed),
                ..Default::default()
            })
        );
    }
}
