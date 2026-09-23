use serde_json::Value;
use zeron_proto::computer::{
    COMPUTER_DRAG_MAX_DURATION_MS, COMPUTER_HOTKEY_MAX_KEYS, COMPUTER_KEY_NAME_MAX_LENGTH,
    COMPUTER_SELECT_TEXT_RANGE_MAX, COMPUTER_SEMANTIC_ACTION_MAX_LENGTH, COMPUTER_TEXT_MAX_LENGTH,
    COMPUTER_WAIT_MAX_MS, ComputerInputModifier, ComputerRect, ComputerTarget,
};

use super::schemas::DEFAULT_DRAG_DURATION_MS;
use super::tool_input::{
    ToolArgs, ToolInputError, is_safe_integer, read_number_arg, read_record_arg,
    read_required_string_arg, read_string_arg, read_string_array_arg, read_verbatim_string_arg,
};
use crate::computer::backend::{
    ComputerCaptureRequest, ComputerMenuTarget, ComputerTextRange,
    DEFAULT_COMPUTER_CAPTURE_MAX_DIMENSION, MAX_COMPUTER_CLIPBOARD_BYTES,
};
use crate::computer::manager::{ComputerClickButton, ComputerClickGesture};
use crate::computer::utf8_truncation::utf16_len;

type Read<T> = Result<T, ToolInputError>;

fn input(message: impl Into<String>) -> ToolInputError {
    ToolInputError::new(message)
}

pub fn read_window_id_arg(args: &ToolArgs) -> Read<Option<String>> {
    match read_string_arg(args, "window_id", false)? {
        Some(id) => Ok(Some(id)),
        None => read_string_arg(args, "windowId", false),
    }
}

pub fn read_menu_target_arg(args: &ToolArgs) -> Read<ComputerMenuTarget> {
    let window_id = read_window_id_arg(args)?;
    let app = read_string_arg(args, "app", false)?;
    let pid = read_number_arg(args, "pid")?;
    let forms = [window_id.is_some(), app.is_some(), pid.is_some()]
        .iter()
        .filter(|present| **present)
        .count();
    if forms == 0 {
        return Err(input(
            "Name the target one way: \"window_id\" (one exact window), or \"app\"/\"pid\" (that running app's menu bar, no window needed).",
        ));
    }
    if forms > 1 {
        return Err(input(
            "Pass exactly one of \"window_id\", \"app\", or \"pid\" — they name different menu routes.",
        ));
    }
    if let Some(window_id) = window_id {
        return Ok(ComputerMenuTarget::Window(window_id));
    }
    if let Some(app) = app {
        return Ok(ComputerMenuTarget::App(app));
    }
    match pid {
        Some(pid) if is_safe_integer(pid) && pid > 0.0 && pid <= f64::from(u32::MAX) => {
            Ok(ComputerMenuTarget::Pid(pid as u32))
        }
        _ => Err(input("\"pid\" must be a positive integer.")),
    }
}

pub fn read_menu_path_arg(args: &ToolArgs, subject: &str) -> Read<Vec<String>> {
    let path = read_string_array_arg(args, "path")?.unwrap_or_default();
    if path.is_empty() || path.len() > 6 || path.iter().any(|title| title.trim().is_empty()) {
        return Err(input(format!(
            "{subject} needs a path of one to six non-empty menu titles."
        )));
    }
    Ok(path)
}

pub fn read_screenshot_id_arg(args: &ToolArgs) -> Read<Option<String>> {
    match read_string_arg(args, "screenshot_id", false)? {
        Some(id) => Ok(Some(id)),
        None => read_string_arg(args, "screenshotId", false),
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct ScreenshotTarget {
    pub target: ComputerTarget,
    pub screenshot_id: Option<String>,
}

fn read_non_negative_integer(args: &ToolArgs, names: &[&str], label: &str) -> Read<Option<u64>> {
    let mut value = None;
    for name in names {
        if let Some(found) = read_number_arg(args, name)? {
            value = Some(found);
            break;
        }
    }
    match value {
        None => Ok(None),
        Some(number) if is_safe_integer(number) && number >= 0.0 => Ok(Some(number as u64)),
        Some(_) => Err(input(format!(
            "Argument \"{label}\" must be a non-negative integer."
        ))),
    }
}

pub fn read_screenshot_target(args: &ToolArgs) -> Read<ScreenshotTarget> {
    let x = read_number_arg(args, "x")?;
    let y = read_number_arg(args, "y")?;
    let screenshot_id = read_screenshot_id_arg(args)?;
    let label = read_verbatim_string_arg(args, "label")?;
    let role = read_string_arg(args, "role", false)?;
    let window_id = read_window_id_arg(args)?;
    let element_ref = read_non_negative_integer(args, &["ref"], "ref")?;
    let ref_ordinal = read_non_negative_integer(args, &["ref_ordinal", "refOrdinal"], "ref_ordinal")?;
    Ok(ScreenshotTarget {
        target: ComputerTarget {
            x,
            y,
            label,
            role,
            window_id,
            element_ref,
            ref_ordinal,
        },
        screenshot_id,
    })
}

pub fn read_nested_screenshot_target(args: &ToolArgs, name: &str) -> Read<ScreenshotTarget> {
    let value = read_record_arg(args, name)?
        .ok_or_else(|| input(format!("Missing required argument \"{name}\".")))?;
    read_screenshot_target(value)
}

pub fn has_target_fields(target: &ComputerTarget) -> bool {
    target.x.is_some()
        || target.y.is_some()
        || target.label.is_some()
        || target.role.is_some()
        || target.window_id.is_some()
        || target.element_ref.is_some()
        || target.ref_ordinal.is_some()
}

pub fn read_delta(args: &ToolArgs, name: &str) -> Read<f64> {
    read_number_arg(args, name)?
        .ok_or_else(|| input(format!("Missing required argument \"{name}\".")))
}

pub fn read_rect(args: &ToolArgs) -> Read<ComputerRect> {
    Ok(ComputerRect {
        x: read_delta(args, "x")?,
        y: read_delta(args, "y")?,
        width: read_delta(args, "width")?,
        height: read_delta(args, "height")?,
    })
}

pub fn read_scroll_delta(args: &ToolArgs) -> Read<(f64, f64)> {
    let delta_x = read_number_arg(args, "delta_x")?.unwrap_or(0.0);
    let delta_y = read_number_arg(args, "delta_y")?.unwrap_or(0.0);
    if delta_x == 0.0 && delta_y == 0.0 {
        return Err(input("Scroll needs a nonzero \"delta_x\" or \"delta_y\"."));
    }
    Ok((delta_x, delta_y))
}

pub fn read_drag_duration_ms(args: &ToolArgs) -> Read<f64> {
    Ok(match read_number_arg(args, "duration_ms")? {
        None => DEFAULT_DRAG_DURATION_MS,
        Some(value) => value.clamp(0.0, COMPUTER_DRAG_MAX_DURATION_MS as f64),
    })
}

pub fn read_raw_required_string(args: &ToolArgs, name: &str) -> Read<String> {
    match args.get(name) {
        Some(Value::String(value)) => Ok(value.clone()),
        _ => Err(input(format!("Argument \"{name}\" must be a string."))),
    }
}

pub fn read_required_text(args: &ToolArgs) -> Read<String> {
    let value = read_raw_required_string(args, "text")?;
    if utf16_len(&value) > COMPUTER_TEXT_MAX_LENGTH {
        return Err(input("Argument \"text\" is too long."));
    }
    Ok(value)
}

pub fn read_set_value_value(args: &ToolArgs) -> Read<String> {
    let value = read_raw_required_string(args, "value")?;
    if utf16_len(&value) > COMPUTER_TEXT_MAX_LENGTH {
        return Err(input("Argument \"value\" is too long."));
    }
    Ok(value)
}

pub fn read_select_text_range(args: &ToolArgs) -> Read<ComputerTextRange> {
    let (Some(start), Some(length)) = (
        read_number_arg(args, "start")?,
        read_number_arg(args, "length")?,
    ) else {
        return Err(input("Arguments \"start\" and \"length\" are required."));
    };
    let max = COMPUTER_SELECT_TEXT_RANGE_MAX as f64;
    for (name, value) in [("start", start), ("length", length)] {
        if !is_safe_integer(value) || value < 0.0 || value > max {
            return Err(input(format!(
                "Argument \"{name}\" must be an integer between 0 and {COMPUTER_SELECT_TEXT_RANGE_MAX}."
            )));
        }
    }
    Ok(ComputerTextRange {
        start: start as u64,
        length: length as u64,
    })
}

pub fn read_select_text_target(args: &ToolArgs) -> Read<ScreenshotTarget> {
    let target = read_screenshot_target(args)?;
    if target.target.x.is_some() || target.target.y.is_some() || target.screenshot_id.is_some() {
        return Err(input(
            "computer_select_text targets a text element by label, role and window_id, not by x/y.",
        ));
    }
    Ok(target)
}

pub fn read_click_gesture(args: &ToolArgs) -> Read<ComputerClickGesture> {
    let count = read_number_arg(args, "count")?;
    let button = read_string_arg(args, "button", false)?;
    let count = match count {
        None => None,
        Some(count) if is_safe_integer(count) && (1.0..=3.0).contains(&count) => {
            Some(count as u8)
        }
        Some(_) => return Err(input("Argument \"count\" must be 1, 2 or 3.")),
    };
    let button = match button.as_deref() {
        None => None,
        Some("left") => Some(ComputerClickButton::Left),
        Some("right") => Some(ComputerClickButton::Right),
        Some("middle") => Some(ComputerClickButton::Middle),
        Some(_) => {
            return Err(input(
                "Argument \"button\" must be \"left\", \"right\" or \"middle\".",
            ));
        }
    };
    Ok(ComputerClickGesture { count, button })
}

pub struct KeyOrChord {
    pub key: String,
    pub chord: Option<Vec<String>>,
}

pub fn read_key_or_chord(args: &ToolArgs) -> Read<KeyOrChord> {
    let key = read_required_string_arg(args, "key")?;
    if !key.contains('+') {
        return Ok(KeyOrChord { key, chord: None });
    }
    let parts: Vec<String> = key.split('+').map(|part| part.trim().to_owned()).collect();
    if parts.iter().any(String::is_empty) {
        return Ok(KeyOrChord { key, chord: None });
    }
    if parts.len() > COMPUTER_HOTKEY_MAX_KEYS {
        return Err(input(format!(
            "A \"+\"-joined chord accepts at most {COMPUTER_HOTKEY_MAX_KEYS} keys; got {}.",
            parts.len()
        )));
    }
    if let Some(oversized) = parts
        .iter()
        .find(|part| utf16_len(part) > COMPUTER_KEY_NAME_MAX_LENGTH)
    {
        return Err(input(format!(
            "Each key in a chord is at most {COMPUTER_KEY_NAME_MAX_LENGTH} characters; got one of {}.",
            utf16_len(oversized)
        )));
    }
    Ok(KeyOrChord {
        key,
        chord: Some(parts),
    })
}

pub fn read_action_name(args: &ToolArgs) -> Read<String> {
    let value = read_required_string_arg(args, "action")?;
    if utf16_len(&value) > COMPUTER_SEMANTIC_ACTION_MAX_LENGTH {
        return Err(input(format!(
            "Argument \"action\" is longer than {COMPUTER_SEMANTIC_ACTION_MAX_LENGTH} characters."
        )));
    }
    Ok(value)
}

pub fn read_clipboard_text(args: &ToolArgs) -> Read<String> {
    let value = read_raw_required_string(args, "text")?;
    if value.len() > MAX_COMPUTER_CLIPBOARD_BYTES {
        return Err(input(format!(
            "Argument \"text\" is longer than the {MAX_COMPUTER_CLIPBOARD_BYTES} byte clipboard limit."
        )));
    }
    Ok(value)
}

pub enum ScreenshotRequest {
    Capture(ComputerCaptureRequest),
    Focused { max_dimension: Option<u32> },
}

const CAPTURE_REGION_KEYS: [&str; 4] = ["x", "y", "width", "height"];

pub fn read_capture_request<E>(
    args: &ToolArgs,
    map_region: impl FnOnce(ComputerRect) -> Result<ComputerRect, E>,
) -> Result<Result<ScreenshotRequest, E>, ToolInputError> {
    let window_id = read_window_id_arg(args)?;
    let present: Vec<&str> = CAPTURE_REGION_KEYS
        .into_iter()
        .filter(|key| args.get(*key).is_some_and(|value| !value.is_null()))
        .collect();
    let max_dimension = read_capture_max_dimension(args)?;
    if let Some(window_id) = window_id {
        if !present.is_empty() {
            return Err(input(
                "Pass either \"window_id\" or the region arguments \"x\", \"y\", \"width\" and \"height\", never both.",
            ));
        }
        return Ok(Ok(ScreenshotRequest::Capture(
            ComputerCaptureRequest::Window {
                window_id,
                max_dimension,
            },
        )));
    }
    if present.is_empty() {
        return Ok(Ok(ScreenshotRequest::Focused { max_dimension }));
    }
    if present.len() < CAPTURE_REGION_KEYS.len() {
        let missing: Vec<&str> = CAPTURE_REGION_KEYS
            .into_iter()
            .filter(|key| !present.contains(key))
            .collect();
        return Err(input(format!(
            "A screenshot region needs \"x\", \"y\", \"width\" and \"height\". Missing: {}.",
            missing.join(", ")
        )));
    }
    let region = read_rect(args)?;
    if region.width <= 0.0 || region.height <= 0.0 {
        return Err(input(
            "Arguments \"width\" and \"height\" must be greater than zero.",
        ));
    }
    Ok(map_region(region).map(|region| {
        ScreenshotRequest::Capture(ComputerCaptureRequest::Region {
            region,
            max_dimension,
        })
    }))
}

pub fn read_capture_max_dimension(args: &ToolArgs) -> Read<Option<u32>> {
    let Some(value) = read_number_arg(args, "max_dimension")? else {
        return Ok(None);
    };
    if value < 1.0 {
        return Err(input("Argument \"max_dimension\" must be at least 1."));
    }
    Ok(Some(
        (value.floor() as u64).min(u64::from(DEFAULT_COMPUTER_CAPTURE_MAX_DIMENSION)) as u32,
    ))
}

pub fn read_modifiers(args: &ToolArgs) -> Read<Vec<ComputerInputModifier>> {
    let Some(raw) = read_string_array_arg(args, "modifiers")? else {
        return Ok(Vec::new());
    };
    let mut modifiers = Vec::new();
    for entry in raw {
        let name = entry.trim().to_lowercase();
        let modifier = match name.as_str() {
            "ctrl" => ComputerInputModifier::Ctrl,
            "alt" => ComputerInputModifier::Alt,
            "shift" => ComputerInputModifier::Shift,
            "meta" => ComputerInputModifier::Meta,
            _ => {
                return Err(input(format!(
                    "Argument \"modifiers\" accepts only ctrl, alt, shift, meta; got {}.",
                    Value::String(name)
                )));
            }
        };
        if !modifiers.contains(&modifier) {
            modifiers.push(modifier);
        }
    }
    Ok(modifiers)
}

pub fn read_wait_duration_ms(args: &ToolArgs) -> Read<u64> {
    let value = read_number_arg(args, "duration_ms")?
        .ok_or_else(|| input("Missing required argument \"duration_ms\"."))?;
    Ok(value.floor().clamp(0.0, COMPUTER_WAIT_MAX_MS as f64) as u64)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn args(value: Value) -> ToolArgs {
        value.as_object().unwrap().clone()
    }

    #[test]
    fn targets_read_both_spellings_and_keep_labels_verbatim() {
        let target = read_screenshot_target(&args(json!({
            "x": 10, "y": 20, "screenshotId": "shot-1", "label": "Save ", "windowId": "w",
            "refOrdinal": 1,
        })))
        .unwrap();
        assert_eq!(target.screenshot_id.as_deref(), Some("shot-1"));
        assert_eq!(target.target.label.as_deref(), Some("Save "));
        assert_eq!(target.target.window_id.as_deref(), Some("w"));
        assert_eq!(target.target.ref_ordinal, Some(1));
        assert!(read_screenshot_target(&args(json!({ "ref": -1 }))).is_err());
        assert!(read_screenshot_target(&args(json!({ "ref": 1.5 }))).is_err());
        assert!(!has_target_fields(
            &read_screenshot_target(&args(json!({ "x": null }))).unwrap().target
        ));
    }

    #[test]
    fn menu_targets_take_exactly_one_route() {
        assert_eq!(
            read_menu_target_arg(&args(json!({ "pid": 42 }))).unwrap(),
            ComputerMenuTarget::Pid(42)
        );
        assert!(read_menu_target_arg(&args(json!({}))).is_err());
        assert!(read_menu_target_arg(&args(json!({ "app": "Notes", "pid": 1 }))).is_err());
        assert!(read_menu_target_arg(&args(json!({ "pid": 0 }))).is_err());
        assert!(read_menu_path_arg(&args(json!({ "path": vec!["a"; 7] })), "x").is_err());
    }

    #[test]
    fn chords_split_on_plus_unless_a_part_is_empty() {
        let chord = read_key_or_chord(&args(json!({ "key": "cmd + s" }))).unwrap();
        assert_eq!(chord.chord, Some(vec!["cmd".to_owned(), "s".to_owned()]));
        let plus = read_key_or_chord(&args(json!({ "key": "cmd++s" }))).unwrap();
        assert!(plus.chord.is_none());
        let long = vec!["a"; COMPUTER_HOTKEY_MAX_KEYS + 1].join("+");
        assert!(read_key_or_chord(&args(json!({ "key": long }))).is_err());
    }

    #[test]
    fn clamps_durations_and_capture_sizes() {
        assert_eq!(read_drag_duration_ms(&args(json!({}))).unwrap(), 250.0);
        assert_eq!(
            read_drag_duration_ms(&args(json!({ "duration_ms": 1e9 }))).unwrap(),
            COMPUTER_DRAG_MAX_DURATION_MS as f64
        );
        assert_eq!(
            read_wait_duration_ms(&args(json!({ "duration_ms": 99999.7 }))).unwrap(),
            COMPUTER_WAIT_MAX_MS
        );
        assert_eq!(
            read_capture_max_dimension(&args(json!({ "max_dimension": 5000 }))).unwrap(),
            Some(DEFAULT_COMPUTER_CAPTURE_MAX_DIMENSION)
        );
        assert!(read_capture_max_dimension(&args(json!({ "max_dimension": 0 }))).is_err());
    }

    #[test]
    fn modifiers_dedupe_and_refuse_unknown_names() {
        assert_eq!(
            read_modifiers(&args(json!({ "modifiers": ["Shift", "shift", "meta"] }))).unwrap(),
            vec![ComputerInputModifier::Shift, ComputerInputModifier::Meta]
        );
        assert!(read_modifiers(&args(json!({ "modifiers": ["hyper"] }))).is_err());
    }
}
