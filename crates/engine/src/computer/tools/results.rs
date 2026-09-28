use serde_json::{Map, Value, json};
use zeron_proto::computer::{
    ComputerAuditEffect, ComputerAvailability, CuaActionDiagnostics, McpContent, McpToolResult,
};

use super::tool_input::{ToolArgs, ToolInputError};
use crate::computer::audit_log::{ComputerAuditLayer, ComputerAuditTarget};
use crate::computer::backend::{ComputerError, CuaErrorLayer, cua_effect_name};
use crate::computer::setup_signal::{ComputerSetupSignal, computer_setup_tool_note};
use crate::computer::ui_tree_targeting::{ComputerTargetCandidate, ComputerTargetError};

pub const INPUT_PAUSE_REQUERY_HINT: &str = "Read fresh state with computer_get_state on the affected window, then continue the task from what it shows; never replay an uncertain action.";

#[derive(Debug, Clone, PartialEq)]
pub enum ToolValue {
    Json(Value),
    Result(McpToolResult),
}

impl ToolValue {
    pub fn payload(&self) -> Option<Map<String, Value>> {
        match self {
            Self::Json(Value::Object(map)) => Some(map.clone()),
            Self::Json(_) => None,
            Self::Result(result) => tool_result_payload(result),
        }
    }
}

impl From<Value> for ToolValue {
    fn from(value: Value) -> Self {
        Self::Json(value)
    }
}

impl From<McpToolResult> for ToolValue {
    fn from(result: McpToolResult) -> Self {
        Self::Result(result)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum ToolFailure {
    Input(ToolInputError),
    Computer(ComputerError),
    Message(String),
}

impl ToolFailure {
    pub fn message(message: impl Into<String>) -> Self {
        Self::Message(message.into())
    }

    pub fn computer(&self) -> Option<&ComputerError> {
        match self {
            Self::Computer(error) => Some(error),
            _ => None,
        }
    }

    pub fn text(&self) -> String {
        match self {
            Self::Input(error) => error.0.clone(),
            Self::Computer(error) => error.to_string(),
            Self::Message(message) => message.clone(),
        }
    }
}

impl From<ToolInputError> for ToolFailure {
    fn from(error: ToolInputError) -> Self {
        Self::Input(error)
    }
}

macro_rules! computer_failure {
    ($($error:ty),*) => {
        $(impl From<$error> for ToolFailure {
            fn from(error: $error) -> Self {
                Self::Computer(error.into())
            }
        })*
    };
}

computer_failure!(
    ComputerError,
    ComputerTargetError,
    crate::computer::backend::ComputerBackendError,
    crate::computer::backend::CuaActionError,
    crate::computer::desktop_operation_queue::DesktopOperationError
);

pub fn mcp_json(value: &Value) -> McpToolResult {
    McpToolResult::text(value.to_string())
}

pub fn mcp_json_error(value: &Value) -> McpToolResult {
    McpToolResult {
        is_error: true,
        ..mcp_json(value)
    }
}

pub fn tool_result_payload(result: &McpToolResult) -> Option<Map<String, Value>> {
    match serde_json::from_str::<Value>(result.first_text()?).ok()? {
        Value::Object(map) => Some(map),
        _ => None,
    }
}

pub fn result_availability(value: &ToolValue) -> Option<ComputerAvailability> {
    let availability = value.payload()?.remove("availability")?;
    if !availability.is_object() {
        return None;
    }
    serde_json::from_value(availability).ok()
}

pub fn with_setup_note(value: Value, signal: Option<&ComputerSetupSignal>) -> Value {
    let (Some(signal), Value::Object(mut map)) = (signal, value.clone()) else {
        return value;
    };
    if let Some(ComputerAvailability::PermissionRequired { missing, .. }) =
        result_availability(&ToolValue::Json(Value::Object(map.clone())))
    {
        map.insert(
            "availability".into(),
            json!({ "kind": "permission-required", "missing": missing }),
        );
    }
    map.insert(
        "setupRequired".into(),
        Value::String(computer_setup_tool_note(signal)),
    );
    Value::Object(map)
}

fn with_setup_note_in_text(text: &str, note: &str) -> String {
    match serde_json::from_str::<Value>(text) {
        Ok(Value::Object(mut map)) => {
            map.insert("setupRequired".into(), Value::String(note.to_owned()));
            Value::Object(map).to_string()
        }
        _ => format!("{text}\n\n{note}"),
    }
}

fn first_text_index(result: &McpToolResult) -> Option<usize> {
    result
        .content
        .iter()
        .position(|part| matches!(part, McpContent::Text { .. }))
}

fn replace_text(result: &mut McpToolResult, index: usize, text: String) {
    result.content[index] = McpContent::Text { text };
}

fn text_at(result: &McpToolResult, index: usize) -> &str {
    match &result.content[index] {
        McpContent::Text { text } => text,
        McpContent::Image { .. } => "",
    }
}

pub fn with_setup_note_on_result(
    mut result: McpToolResult,
    signal: Option<&ComputerSetupSignal>,
) -> McpToolResult {
    let Some(signal) = signal else {
        return result;
    };
    let note = computer_setup_tool_note(signal);
    match first_text_index(&result) {
        None => result.content.push(McpContent::Text { text: note }),
        Some(index) => {
            let text = with_setup_note_in_text(text_at(&result, index), &note);
            replace_text(&mut result, index, text);
        }
    }
    result
}

pub fn with_disclosure_on_result(mut result: McpToolResult, disclosure: &str) -> McpToolResult {
    let Some(index) = first_text_index(&result) else {
        result.content.push(McpContent::Text {
            text: disclosure.to_owned(),
        });
        return result;
    };
    let text = match serde_json::from_str::<Value>(text_at(&result, index)) {
        Ok(Value::Object(mut map)) => {
            map.insert("disclosure".into(), Value::String(disclosure.to_owned()));
            Value::Object(map).to_string()
        }
        _ => format!("{disclosure}\n\n{}", text_at(&result, index)),
    };
    replace_text(&mut result, index, text);
    result
}

pub fn with_app_on_result(mut result: McpToolResult, app: &str) -> McpToolResult {
    let Some(index) = first_text_index(&result) else {
        return result;
    };
    if let Ok(Value::Object(mut map)) = serde_json::from_str::<Value>(text_at(&result, index))
        && !map.contains_key("appName")
    {
        map.insert("appName".into(), Value::String(app.to_owned()));
        replace_text(&mut result, index, Value::Object(map).to_string());
    }
    result
}

pub fn with_guidance_on_result(mut result: McpToolResult, guidance: Option<&str>) -> McpToolResult {
    let Some(guidance) = guidance else {
        return result;
    };
    let Some(index) = first_text_index(&result) else {
        result.content.insert(
            0,
            McpContent::Text {
                text: guidance.to_owned(),
            },
        );
        return result;
    };
    let text = match serde_json::from_str::<Value>(text_at(&result, index)) {
        Ok(Value::Object(mut map)) => {
            let merged = match map.get("toolGuidance").and_then(Value::as_str) {
                Some(existing) if !existing.is_empty() => format!("{existing} {guidance}"),
                _ => guidance.to_owned(),
            };
            map.insert("toolGuidance".into(), Value::String(merged));
            Value::Object(map).to_string()
        }
        _ => format!("{guidance}\n{}", text_at(&result, index)),
    };
    replace_text(&mut result, index, text);
    result
}

pub fn computer_audit_target(
    args: &ToolArgs,
    result_window_id: Option<String>,
) -> Option<ComputerAuditTarget> {
    let window_id = args
        .get("window_id")
        .or_else(|| args.get("windowId"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .map(str::to_owned)
        .or(result_window_id);
    let pid = args
        .get("pid")
        .and_then(Value::as_f64)
        .filter(|pid| pid.fract() == 0.0 && *pid > 0.0 && *pid <= f64::from(u32::MAX))
        .map(|pid| pid as u32);
    let app = ["app", "app_name", "appName"]
        .into_iter()
        .find_map(|key| {
            args.get(key)
                .and_then(Value::as_str)
                .filter(|app| !app.trim().is_empty())
        })
        .map(str::to_owned);
    if window_id.is_none() && pid.is_none() && app.is_none() {
        return None;
    }
    Some(ComputerAuditTarget {
        window_id,
        pid,
        app,
        bundle_id: None,
    })
}

pub fn computer_audit_result_window_id(value: &ToolValue) -> Option<String> {
    let payload = value.payload()?;
    if let Some(id) = payload.get("windowId").and_then(Value::as_str) {
        return Some(id.to_owned());
    }
    payload
        .get("window")
        .and_then(|window| window.get("id"))
        .and_then(Value::as_str)
        .map(str::to_owned)
}

fn effect_named(name: &str) -> Option<ComputerAuditEffect> {
    match name {
        "verified" => Some(ComputerAuditEffect::Verified),
        "dispatched-unknown" => Some(ComputerAuditEffect::DispatchedUnknown),
        "not-dispatched" => Some(ComputerAuditEffect::NotDispatched),
        _ => None,
    }
}

pub fn computer_audit_success_effect(name: &str, value: &ToolValue) -> ComputerAuditEffect {
    let payload = value.payload();
    if let Some(effect) = payload
        .as_ref()
        .and_then(|payload| payload.get("delivery"))
        .and_then(|delivery| delivery.get("effect"))
        .and_then(Value::as_str)
        .and_then(effect_named)
    {
        return effect;
    }
    if name == "computer_run" {
        let steps = payload
            .as_ref()
            .and_then(|payload| payload.get("steps"))
            .and_then(Value::as_array);
        if steps.is_some_and(|steps| {
            steps.iter().any(|step| {
                step.get("error")
                    .and_then(|error| error.get("effect"))
                    .and_then(Value::as_str)
                    == Some("dispatched-unknown")
            })
        }) {
            return ComputerAuditEffect::DispatchedUnknown;
        }
        let completed = payload
            .as_ref()
            .and_then(|payload| payload.get("completed"))
            .and_then(Value::as_f64);
        return if completed.is_some_and(|completed| completed > 0.0) {
            ComputerAuditEffect::DispatchedUnknown
        } else {
            ComputerAuditEffect::NotDispatched
        };
    }
    ComputerAuditEffect::DispatchedUnknown
}

#[derive(Debug, Clone, PartialEq)]
pub struct AuditOutcome {
    pub effect: ComputerAuditEffect,
    pub code: Option<String>,
    pub diagnostics: Option<CuaActionDiagnostics>,
    pub layer: Option<ComputerAuditLayer>,
}

impl AuditOutcome {
    pub fn new(effect: ComputerAuditEffect, code: Option<&str>) -> Self {
        Self {
            effect,
            code: code.map(str::to_owned),
            diagnostics: None,
            layer: None,
        }
    }
}

fn audit_layer(layer: Option<CuaErrorLayer>) -> Option<ComputerAuditLayer> {
    layer.map(|layer| match layer {
        CuaErrorLayer::DriverHost => ComputerAuditLayer::DriverHost,
        CuaErrorLayer::NativeDriver => ComputerAuditLayer::NativeDriver,
    })
}

fn layer_name(layer: Option<CuaErrorLayer>) -> Option<&'static str> {
    layer.map(|layer| match layer {
        CuaErrorLayer::DriverHost => "driver-host",
        CuaErrorLayer::NativeDriver => "native-driver",
    })
}

fn effect_from_cua(effect: zeron_proto::computer::CuaEffect) -> ComputerAuditEffect {
    effect_named(cua_effect_name(effect)).unwrap_or(ComputerAuditEffect::DispatchedUnknown)
}

pub fn computer_audit_error_outcome(error: &ToolFailure) -> AuditOutcome {
    let computer = match error {
        ToolFailure::Input(_) => {
            return AuditOutcome::new(ComputerAuditEffect::Refused, Some("invalid_arguments"));
        }
        ToolFailure::Message(_) => return AuditOutcome::new(ComputerAuditEffect::Error, Some("error")),
        ToolFailure::Computer(error) => error,
    };
    if let Some(cua) = computer.cua() {
        return AuditOutcome {
            effect: effect_from_cua(cua.effect),
            code: Some(if cua.code.is_empty() {
                "cua_action_error".into()
            } else {
                cua.code.clone()
            }),
            diagnostics: cua.diagnostics.clone(),
            layer: audit_layer(cua.layer),
        };
    }
    match computer {
        ComputerError::Target(_) | ComputerError::Denylist(_) | ComputerError::Lease(_) => {
            AuditOutcome::new(ComputerAuditEffect::Refused, computer.code())
        }
        ComputerError::Approval(_) => AuditOutcome::new(
            ComputerAuditEffect::Refused,
            Some(computer.code().unwrap_or("approval_denied")),
        ),
        ComputerError::Backend(error) if error.input_pause.is_some() => AuditOutcome {
            layer: Some(ComputerAuditLayer::ServerManager),
            ..AuditOutcome::new(ComputerAuditEffect::Refused, Some("computer_input_paused"))
        },
        ComputerError::Backend(_) => {
            AuditOutcome::new(ComputerAuditEffect::Error, Some("computer_backend_error"))
        }
        _ => AuditOutcome::new(ComputerAuditEffect::Error, Some("error")),
    }
}

fn candidates_json(candidates: &[ComputerTargetCandidate]) -> Value {
    Value::Array(
        candidates
            .iter()
            .map(|candidate| {
                let mut entry = json!({
                    "label": candidate.label,
                    "role": candidate.role,
                    "onScreen": candidate.on_screen,
                    "frame": candidate.frame,
                });
                if let Some(window_id) = &candidate.window_id {
                    entry["windowId"] = json!(window_id);
                }
                entry
            })
            .collect(),
    )
}

fn target_error_fields(error: &ComputerTargetError) -> Value {
    json!({
        "code": error.code.as_str(),
        "message": error.to_string(),
        "notFound": error.not_found,
        "candidates": candidates_json(&error.candidates),
    })
}

fn input_pause_error(error: &ComputerError) -> Option<Map<String, Value>> {
    let pause = error.input_pause()?;
    let mut fields = Map::new();
    fields.insert("code".into(), json!("computer_input_paused"));
    if let Value::Object(pause) = serde_json::to_value(pause).unwrap_or(Value::Null) {
        fields.extend(pause);
    }
    let cua = error.cua();
    fields.insert(
        "layer".into(),
        json!(
            cua.map_or(Some("server-manager"), |cua| layer_name(cua.layer))
        ),
    );
    if let Some(cua) = cua {
        fields.insert("cause".into(), json!(cua.code));
        fields.insert("diagnostics".into(), json!(cua.diagnostics));
        if let Some(wait) = cua.wait_seconds {
            fields.insert("wait_seconds".into(), json!(wait));
        }
    }
    fields.insert("requery_hint".into(), json!(INPUT_PAUSE_REQUERY_HINT));
    Some(fields)
}

pub fn cua_action_error_payload(error: &ComputerError) -> Option<Value> {
    let cua = error.cua()?;
    let mut payload = json!({
        "error": cua.code,
        "effect": cua_effect_name(cua.effect),
        "message": error.to_string(),
        "retryAllowed": false,
    });
    if let Some(diagnostics) = &cua.diagnostics {
        payload["diagnostics"] = json!(diagnostics);
    }
    if let Some(layer) = layer_name(cua.layer) {
        payload["layer"] = json!(layer);
    }
    if let Some(wait) = cua.wait_seconds {
        payload["wait_seconds"] = json!(wait);
    }
    if cua.code == "computer_input_paused" {
        payload["requery_hint"] = json!(INPUT_PAUSE_REQUERY_HINT);
    }
    Some(payload)
}

pub fn failure_result(error: &ToolFailure) -> McpToolResult {
    let ToolFailure::Computer(computer) = error else {
        return McpToolResult::error(error.text());
    };
    if let Some(mut fields) = input_pause_error(computer) {
        fields.insert("retryable".into(), json!(false));
        let mut body = json!({ "error": Value::Object(fields) });
        if let Some(cua) = computer.cua() {
            body["effect"] = json!(cua_effect_name(cua.effect));
            body["retryAllowed"] = json!(false);
        }
        return mcp_json_error(&body);
    }
    if let Some(payload) = cua_action_error_payload(computer) {
        return mcp_json_error(&payload);
    }
    match computer {
        ComputerError::Target(target) => {
            mcp_json_error(&json!({ "error": target_error_fields(target) }))
        }
        ComputerError::Denylist(denied) => mcp_json_error(&json!({
            "error": {
                "code": denied.code(),
                "message": computer.to_string(),
                "notFound": false,
                "candidates": [],
            },
        })),
        ComputerError::Lease(_) => mcp_json_error(&json!({
            "error": {
                "code": computer.code(),
                "message": computer.to_string(),
                "retryable": false,
            },
        })),
        _ => McpToolResult::error(computer.to_string()),
    }
}

pub fn run_step_error(error: &ToolFailure) -> Value {
    let computer = match error {
        ToolFailure::Input(input) => {
            return json!({ "code": "invalid_step", "message": input.0 });
        }
        ToolFailure::Message(message) => {
            return json!({ "code": "step_failed", "message": message });
        }
        ToolFailure::Computer(computer) => computer,
    };
    if let Some(mut fields) = input_pause_error(computer) {
        if let Some(cua) = computer.cua() {
            fields.insert("effect".into(), json!(cua_effect_name(cua.effect)));
        }
        return Value::Object(fields);
    }
    if let Some(cua) = computer.cua() {
        let mut entry = json!({
            "code": cua.code,
            "effect": cua_effect_name(cua.effect),
            "message": computer.to_string(),
            "retryAllowed": false,
        });
        if let Some(diagnostics) = &cua.diagnostics {
            entry["diagnostics"] = json!(diagnostics);
        }
        if let Some(layer) = layer_name(cua.layer) {
            entry["layer"] = json!(layer);
        }
        return entry;
    }
    match computer {
        ComputerError::Target(target) => target_error_fields(target),
        ComputerError::Denylist(denied) => json!({
            "code": denied.code(),
            "message": computer.to_string(),
            "notFound": false,
            "candidates": [],
        }),
        ComputerError::Lease(_) => json!({
            "code": computer.code(),
            "message": computer.to_string(),
            "retryable": false,
        }),
        _ => {
            let mut entry = json!({ "code": "step_failed", "message": computer.to_string() });
            if matches!(computer, ComputerError::Backend(_)) && computer.retryable() {
                entry["retryable"] = json!(true);
            }
            entry
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn notes_land_in_the_json_text_part_or_trail_prose() {
        let signal = ComputerSetupSignal {
            missing: Vec::new(),
            blocking: true,
            build_signature: None,
            bundle_id: None,
        };
        let json_result = with_setup_note_on_result(mcp_json(&json!({ "a": 1 })), Some(&signal));
        let payload = tool_result_payload(&json_result).unwrap();
        assert!(payload["setupRequired"].as_str().unwrap().contains("Zeron needs"));
        let prose = with_setup_note_on_result(McpToolResult::error("boom"), Some(&signal));
        assert!(prose.first_text().unwrap().starts_with("boom\n\n"));
        let disclosed = with_disclosure_on_result(McpToolResult::text("plain"), "ON");
        assert_eq!(disclosed.first_text(), Some("ON\n\nplain"));
        let guided = with_guidance_on_result(
            mcp_json(&json!({ "toolGuidance": "first" })),
            Some("second"),
        );
        assert_eq!(
            tool_result_payload(&guided).unwrap()["toolGuidance"],
            json!("first second")
        );
    }

    #[test]
    fn the_audit_target_names_the_app_from_app_or_app_name() {
        let target = |args: Value| {
            computer_audit_target(args.as_object().unwrap(), None).and_then(|target| target.app)
        };
        assert_eq!(
            target(json!({ "app": "Finder" })).as_deref(),
            Some("Finder")
        );
        assert_eq!(
            target(json!({ "window_id": "cua:1:2", "app_name": "Ghostty" })).as_deref(),
            Some("Ghostty")
        );
        assert_eq!(
            target(json!({ "appName": "Calculator" })).as_deref(),
            Some("Calculator")
        );
        assert_eq!(target(json!({ "window_id": "w1", "app_name": " " })), None);
    }

    #[test]
    fn a_run_counts_as_dispatched_when_any_step_ran_or_one_step_was_uncertain() {
        let run = |value: Value| {
            computer_audit_success_effect("computer_run", &ToolValue::Json(value))
        };
        assert_eq!(
            run(json!({ "completed": 0, "steps": [] })),
            ComputerAuditEffect::NotDispatched
        );
        assert_eq!(
            run(json!({ "completed": 2, "steps": [] })),
            ComputerAuditEffect::DispatchedUnknown
        );
        assert_eq!(
            run(json!({ "completed": 0, "steps": [{ "error": { "effect": "dispatched-unknown" } }] })),
            ComputerAuditEffect::DispatchedUnknown
        );
        assert_eq!(
            computer_audit_success_effect(
                "computer_click",
                &ToolValue::Json(json!({ "delivery": { "effect": "verified" } }))
            ),
            ComputerAuditEffect::Verified
        );
    }
}
