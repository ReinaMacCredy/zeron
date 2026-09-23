use std::borrow::Cow;

use base64::Engine as _;
use serde_json::Value;
use zeron_proto::computer::{
    COMPUTER_ID_MAX_LENGTH, COMPUTER_LABEL_MAX_LENGTH, COMPUTER_OCCLUDERS_MAX_LENGTH,
    COMPUTER_WINDOW_LIST_MAX_LENGTH, ComputerPngMimeType, ComputerPoint, ComputerRect,
    ComputerScreenSize, ComputerScreenshot, ComputerWindow,
};

use super::utf8_truncation::clamp_text_to_length;

const DEFAULT_CAPTURE_SOURCE: &str = "The desktop capture";

pub const POINTER_CLAMP_TOLERANCE_PX: f64 = 2.0;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct ComputerGeometryError(pub String);

pub fn parse_json_payload(value: &Value) -> Cow<'_, Value> {
    match value {
        Value::String(text) => Cow::Owned(serde_json::from_str(text).unwrap_or(Value::Null)),
        other => Cow::Borrowed(other),
    }
}

pub fn as_string(value: Option<&Value>) -> Option<&str> {
    value
        .and_then(Value::as_str)
        .filter(|text| !text.is_empty())
}

pub fn as_finite_number(value: Option<&Value>) -> Option<f64> {
    value.and_then(Value::as_f64).filter(|n| n.is_finite())
}

pub fn as_non_negative_int(value: Option<&Value>) -> Option<u64> {
    as_finite_number(value)
        .filter(|n| *n >= 0.0)
        .map(|n| n.trunc() as u64)
}

pub fn parse_computer_rect(value: &Value) -> Option<ComputerRect> {
    let x = as_finite_number(value.get("x"))?;
    let y = as_finite_number(value.get("y"))?;
    let width = as_finite_number(value.get("width"))?;
    let height = as_finite_number(value.get("height"))?;
    (width >= 0.0 && height >= 0.0).then_some(ComputerRect {
        x,
        y,
        width,
        height,
    })
}

pub fn parse_computer_point(value: &Value) -> Option<ComputerPoint> {
    Some(ComputerPoint {
        x: as_finite_number(value.get("x"))?,
        y: as_finite_number(value.get("y"))?,
    })
}

fn as_window_ids(value: Option<&Value>) -> Option<Vec<String>> {
    let ids: Vec<String> = value?
        .as_array()?
        .iter()
        .filter_map(Value::as_str)
        .filter(|id| !id.is_empty())
        .take(COMPUTER_OCCLUDERS_MAX_LENGTH)
        .map(|id| clamp_text_to_length(id, COMPUTER_ID_MAX_LENGTH))
        .collect();
    (!ids.is_empty()).then_some(ids)
}

pub fn windows_payload_fingerprint(payload: &Value, focused_window_id: Option<&str>) -> String {
    let body = match payload {
        Value::String(text) => text.clone(),
        other => other.to_string(),
    };
    format!("{} {body}", focused_window_id.unwrap_or_default())
}

fn number_or_blank(value: Option<f64>) -> String {
    value.map(|n| n.to_string()).unwrap_or_default()
}

pub fn windows_digest_fingerprint(
    windows: &[ComputerWindow],
    focused_window_id: Option<&str>,
) -> String {
    let mut parts = vec![focused_window_id.unwrap_or_default().to_owned()];
    for window in windows {
        let bounds = window.bounds.as_ref();
        let flag = |on: bool| if on { "1" } else { "0" }.to_owned();
        parts.push(
            [
                window.id.clone(),
                number_or_blank(bounds.map(|b| b.x)),
                number_or_blank(bounds.map(|b| b.y)),
                number_or_blank(bounds.map(|b| b.width)),
                number_or_blank(bounds.map(|b| b.height)),
                window
                    .stacking_index
                    .map(|index| index.to_string())
                    .unwrap_or_default(),
                flag(window.focused),
                window.active.map(flag).unwrap_or_default(),
                flag(window.minimized),
                flag(window.visible),
                window.title.clone(),
                window.app_name.clone().unwrap_or_default(),
            ]
            .join("\u{1}"),
        );
    }
    parts.join("\u{2}")
}

pub struct WindowListChangeNotifier<F> {
    fingerprint: Option<String>,
    emit: F,
}

impl<F: FnMut(&[ComputerWindow])> WindowListChangeNotifier<F> {
    pub fn new(emit: F) -> Self {
        Self {
            fingerprint: None,
            emit,
        }
    }

    pub fn observe(&mut self, fingerprint: String, windows: &[ComputerWindow]) {
        if self.fingerprint.as_deref() == Some(fingerprint.as_str()) {
            return;
        }
        self.fingerprint = Some(fingerprint);
        (self.emit)(windows);
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PointerClampResult {
    pub point: ComputerPoint,
    pub clamped_to: Option<ComputerPoint>,
}

pub fn pointer_clamp_result(
    requested: ComputerPoint,
    actual: Option<ComputerPoint>,
) -> PointerClampResult {
    let clamped_to = actual.filter(|actual| {
        (actual.x - requested.x).abs() > POINTER_CLAMP_TOLERANCE_PX
            || (actual.y - requested.y).abs() > POINTER_CLAMP_TOLERANCE_PX
    });
    PointerClampResult {
        point: requested,
        clamped_to,
    }
}

pub fn parse_windows(value: &Value, focused_window_id: Option<&str>) -> Vec<ComputerWindow> {
    let parsed = parse_json_payload(value);
    let Some(items) = parsed.as_array() else {
        return Vec::new();
    };
    let mut windows = Vec::new();
    for record in items.iter().take(COMPUTER_WINDOW_LIST_MAX_LENGTH) {
        let raw_id = as_string(record.get("id")).or_else(|| as_string(record.get("windowId")));
        let bounds = record.get("bounds").and_then(parse_computer_rect);
        let (Some(raw_id), Some(bounds)) = (raw_id, bounds) else {
            continue;
        };
        let app_name = as_string(record.get("appName"))
            .or_else(|| as_string(record.get("appId")))
            .or_else(|| as_string(record.get("resourceClass")))
            .map(|name| clamp_text_to_length(name, COMPUTER_LABEL_MAX_LENGTH));
        let pid = record
            .get("pid")
            .and_then(Value::as_f64)
            .filter(|pid| *pid > 0.0)
            .map(f64::trunc)
            .filter(|pid| *pid >= 1.0 && *pid <= f64::from(u32::MAX))
            .map(|pid| pid as u32);
        windows.push(ComputerWindow {
            id: clamp_text_to_length(raw_id, COMPUTER_ID_MAX_LENGTH),
            title: clamp_text_to_length(
                as_string(record.get("title")).unwrap_or_default(),
                COMPUTER_LABEL_MAX_LENGTH,
            ),
            app_name,
            pid,
            bounds: Some(bounds),
            focused: record.get("focused") == Some(&Value::Bool(true))
                || Some(raw_id) == focused_window_id,
            keyboard_focused: None,
            active: record.get("active").and_then(Value::as_bool),
            minimized: record.get("minimized") == Some(&Value::Bool(true)),
            visible: record.get("visible") != Some(&Value::Bool(false)),
            space_ids: None,
            current_space_id: None,
            on_current_space: None,
            stacking_index: as_non_negative_int(record.get("stackingIndex")),
            occluded_by: as_window_ids(record.get("occludedBy")),
        });
    }
    windows
}

pub fn windows_covering_point<'a>(
    windows: &'a [ComputerWindow],
    window_id: &str,
    point: ComputerPoint,
) -> Vec<&'a ComputerWindow> {
    let Some(depth) = windows
        .iter()
        .find(|window| window.id == window_id)
        .and_then(|window| window.stacking_index)
    else {
        return Vec::new();
    };
    windows
        .iter()
        .filter(|window| {
            window.id != window_id
                && window.visible
                && !window.minimized
                && window.stacking_index.is_some_and(|index| index < depth)
                && rect_contains_point(window.bounds.as_ref(), point)
        })
        .collect()
}

pub fn topmost_window_at_point(
    windows: &[ComputerWindow],
    point: ComputerPoint,
) -> Option<&ComputerWindow> {
    let candidates: Vec<&ComputerWindow> = windows
        .iter()
        .filter(|window| {
            window.visible
                && !window.minimized
                && rect_contains_point(window.bounds.as_ref(), point)
        })
        .collect();
    if candidates.len() <= 1 {
        return candidates.first().copied();
    }
    if candidates
        .iter()
        .any(|window| window.stacking_index.is_none())
    {
        return None;
    }
    candidates.into_iter().reduce(|top, window| {
        if window.stacking_index < top.stacking_index {
            window
        } else {
            top
        }
    })
}

pub fn rect_contains_point(rect: Option<&ComputerRect>, point: ComputerPoint) -> bool {
    rect.is_some_and(|rect| {
        point.x >= rect.x
            && point.y >= rect.y
            && point.x < rect.x + rect.width
            && point.y < rect.y + rect.height
    })
}

pub fn shift_rect(rect: ComputerRect, dx: f64, dy: f64) -> ComputerRect {
    ComputerRect {
        x: rect.x + dx,
        y: rect.y + dy,
        ..rect
    }
}

pub fn shift_point(point: ComputerPoint, dx: f64, dy: f64) -> ComputerPoint {
    ComputerPoint {
        x: point.x + dx,
        y: point.y + dy,
    }
}

pub fn window_in_agent_space(window: &ComputerWindow, origin: ComputerPoint) -> ComputerWindow {
    ComputerWindow {
        bounds: window
            .bounds
            .map(|bounds| shift_rect(bounds, -origin.x, -origin.y)),
        ..window.clone()
    }
}

pub fn workspace_rect_from_windows(
    windows: &[ComputerWindow],
    workspace: Option<&ComputerRect>,
) -> ComputerRect {
    if let Some(workspace) = workspace.filter(|w| w.width > 0.0 && w.height > 0.0) {
        return ComputerRect {
            x: workspace.x.floor(),
            y: workspace.y.floor(),
            width: workspace.width.ceil().max(1.0),
            height: workspace.height.ceil().max(1.0),
        };
    }
    let bounds: Vec<&ComputerRect> = windows.iter().filter_map(|w| w.bounds.as_ref()).collect();
    let left = bounds.iter().fold(0.0_f64, |acc, r| acc.min(r.x.floor()));
    let top = bounds.iter().fold(0.0_f64, |acc, r| acc.min(r.y.floor()));
    let right = bounds
        .iter()
        .fold(left + 1.0, |acc, r| acc.max((r.x + r.width).ceil()));
    let bottom = bounds
        .iter()
        .fold(top + 1.0, |acc, r| acc.max((r.y + r.height).ceil()));
    ComputerRect {
        x: left,
        y: top,
        width: right - left,
        height: bottom - top,
    }
}

pub fn require_window_bounds(
    window: &ComputerWindow,
    action: &str,
) -> Result<ComputerRect, ComputerGeometryError> {
    window.bounds.ok_or_else(|| {
        ComputerGeometryError(format!(
            "This desktop reports no geometry for window {}, so {action} cannot be resolved. \
             The display server exposes window titles and activation but no position or size \
             (capabilities.windowBounds is false); use full-screen capture and desktop coordinates instead.",
            serde_json::to_string(&window.id).unwrap_or_default()
        ))
    })
}

pub fn screen_size_from_windows(
    windows: &[ComputerWindow],
    workspace: Option<&ComputerRect>,
    scale: f64,
) -> ComputerScreenSize {
    let rect = workspace_rect_from_windows(windows, workspace);
    ComputerScreenSize {
        width: rect.width as u32,
        height: rect.height as u32,
        scale: Some(if scale > 0.0 { scale } else { 1.0 }),
    }
}

pub fn align_rect(rect: ComputerRect) -> ComputerRect {
    let x = rect.x.floor();
    let y = rect.y.floor();
    ComputerRect {
        x,
        y,
        width: ((rect.x + rect.width).ceil() - x).max(1.0),
        height: ((rect.y + rect.height).ceil() - y).max(1.0),
    }
}

pub fn format_rect(rect: &ComputerRect) -> String {
    format!("{}x{} at ({}, {})", rect.width, rect.height, rect.x, rect.y)
}

const PNG_SIGNATURE: [u8; 8] = [0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a];
const PNG_IHDR: [u8; 4] = *b"IHDR";
const PNG_HEADER_BYTES: usize = 24;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PngDimensions {
    pub width: u32,
    pub height: u32,
}

pub fn png_dimensions(bytes: &[u8]) -> Option<PngDimensions> {
    if bytes.len() < PNG_HEADER_BYTES || bytes[..8] != PNG_SIGNATURE || bytes[12..16] != PNG_IHDR {
        return None;
    }
    let width = u32::from_be_bytes(bytes[16..20].try_into().ok()?);
    let height = u32::from_be_bytes(bytes[20..24].try_into().ok()?);
    (width > 0 && height > 0).then_some(PngDimensions { width, height })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct JpegDimensions {
    pub width: u32,
    pub height: u32,
}

pub fn jpeg_dimensions(bytes: &[u8]) -> Option<JpegDimensions> {
    let standalone = |marker: u8| marker == 0x01 || (0xd0..=0xd9).contains(&marker);
    let start_of_frame =
        |marker: u8| matches!(marker, 0xc0..=0xc3 | 0xc5..=0xc7 | 0xc9..=0xcb | 0xcd..=0xcf);
    let u16_at = |offset: usize| u16::from_be_bytes([bytes[offset], bytes[offset + 1]]);
    if bytes.len() < 4 || bytes[0] != 0xff || bytes[1] != 0xd8 {
        return None;
    }
    let mut offset = 2;
    while offset + 4 <= bytes.len() {
        if bytes[offset] != 0xff {
            return None;
        }
        let mut marker = bytes[offset + 1];
        while marker == 0xff && offset + 2 < bytes.len() {
            offset += 1;
            marker = bytes[offset + 1];
        }
        if standalone(marker) {
            offset += 2;
            continue;
        }
        if offset + 4 > bytes.len() {
            return None;
        }
        let length = usize::from(u16_at(offset + 2));
        if length < 2 {
            return None;
        }
        if start_of_frame(marker) {
            if offset + 9 > bytes.len() {
                return None;
            }
            let height = u32::from(u16_at(offset + 5));
            let width = u32::from(u16_at(offset + 7));
            return (width > 0 && height > 0).then_some(JpegDimensions { width, height });
        }
        if marker == 0xda {
            return None;
        }
        offset += 2 + length;
    }
    None
}

pub fn read_png_dimensions(
    bytes: &[u8],
    source: Option<&str>,
) -> Result<PngDimensions, ComputerGeometryError> {
    png_dimensions(bytes).ok_or_else(|| {
        ComputerGeometryError(format!(
            "{} did not return a usable PNG image.",
            source.unwrap_or(DEFAULT_CAPTURE_SOURCE)
        ))
    })
}

pub struct ScreenshotFromPng<'a> {
    pub bytes: &'a [u8],
    pub region: ComputerRect,
    pub captured_at: String,
    pub source: Option<&'a str>,
    pub bytes_base64: Option<String>,
}

pub fn screenshot_from_png(
    input: ScreenshotFromPng<'_>,
) -> Result<ComputerScreenshot, ComputerGeometryError> {
    let dimensions = read_png_dimensions(input.bytes, input.source)?;
    Ok(ComputerScreenshot {
        mime_type: ComputerPngMimeType::Png,
        width: dimensions.width,
        height: dimensions.height,
        size_bytes: input.bytes.len() as u64,
        bytes_base64: input
            .bytes_base64
            .unwrap_or_else(|| base64::engine::general_purpose::STANDARD.encode(input.bytes)),
        window_id: None,
        region: Some(input.region),
        scale: Some(f64::from(dimensions.width) / input.region.width),
        captured_at: input.captured_at,
    })
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::computer::utf8_truncation::utf16_len;

    fn window(overrides: Value) -> Value {
        let mut base = json!({
            "id": "window-1",
            "title": "Terminal",
            "bounds": { "x": 0, "y": 0, "width": 800, "height": 600 },
            "focused": false,
            "minimized": false,
            "visible": true,
        });
        for (key, value) in overrides.as_object().unwrap() {
            base[key] = value.clone();
        }
        base
    }

    fn is_contract_window(window: &ComputerWindow) -> bool {
        serde_json::from_value::<ComputerWindow>(serde_json::to_value(window).unwrap()).is_ok()
    }

    #[test]
    fn passes_ordinary_windows_through_untouched() {
        let windows = parse_windows(&json!([window(json!({}))]), None);
        assert_eq!(windows.len(), 1);
        assert!(is_contract_window(&windows[0]));
    }

    #[test]
    fn truncates_a_title_and_app_name_past_the_label_maximum() {
        let long_text = "x".repeat(COMPUTER_LABEL_MAX_LENGTH + 500);
        let windows = parse_windows(
            &json!([window(
                json!({ "id": "w", "title": long_text, "appId": long_text })
            )]),
            None,
        );
        assert_eq!(utf16_len(&windows[0].title), COMPUTER_LABEL_MAX_LENGTH);
        assert!(windows[0].title.ends_with('…'));
        assert_eq!(
            utf16_len(windows[0].app_name.as_deref().unwrap()),
            COMPUTER_LABEL_MAX_LENGTH
        );
        assert!(is_contract_window(&windows[0]));
    }

    #[test]
    fn keeps_the_truncation_cut_outside_a_surrogate_pair() {
        let text = format!(
            "{}\u{1F984}{}",
            "a".repeat(COMPUTER_LABEL_MAX_LENGTH - 1),
            "b".repeat(100)
        );
        let windows = parse_windows(&json!([window(json!({ "id": "w", "title": text }))]), None);
        assert!(utf16_len(&windows[0].title) <= COMPUTER_LABEL_MAX_LENGTH);
        assert!(!windows[0].title.contains('\u{fffd}'));
        assert!(is_contract_window(&windows[0]));
    }

    #[test]
    fn caps_the_reported_window_list_at_the_contract_ceiling() {
        let items: Vec<Value> = (0..COMPUTER_WINDOW_LIST_MAX_LENGTH + 200)
            .map(|index| window(json!({ "id": format!("window-{index}") })))
            .collect();
        assert_eq!(
            parse_windows(&Value::Array(items), None).len(),
            COMPUTER_WINDOW_LIST_MAX_LENGTH
        );
    }

    #[test]
    fn caps_one_windows_occluder_list_and_each_occluder_id() {
        let occluders: Vec<String> = (0..COMPUTER_OCCLUDERS_MAX_LENGTH * 4)
            .map(|index| format!("occluder-{index}"))
            .collect();
        let windows = parse_windows(
            &json!([
                window(json!({ "id": "w", "occludedBy": occluders })),
                window(json!({ "id": "cover" })),
            ]),
            None,
        );
        assert_eq!(
            windows[0].occluded_by.as_ref().unwrap().len(),
            COMPUTER_OCCLUDERS_MAX_LENGTH
        );
        assert!(is_contract_window(&windows[0]));
    }

    #[test]
    fn clamps_an_oversized_window_id_instead_of_dropping_or_emitting_it_raw() {
        let long_id = "w".repeat(COMPUTER_ID_MAX_LENGTH + 100);
        let windows = parse_windows(&json!([window(json!({ "id": long_id }))]), None);
        assert_eq!(utf16_len(&windows[0].id), COMPUTER_ID_MAX_LENGTH);
        assert!(is_contract_window(&windows[0]));
    }

    #[test]
    fn still_matches_the_focused_window_when_its_id_needed_clamping() {
        let long_id = "w".repeat(COMPUTER_ID_MAX_LENGTH + 100);
        let windows = parse_windows(&json!([window(json!({ "id": long_id }))]), Some(&long_id));
        assert!(windows[0].focused);
    }

    fn stacked(id: &str, bounds: (f64, f64, f64, f64), stacking: Option<u64>) -> ComputerWindow {
        ComputerWindow {
            id: id.to_owned(),
            title: id.to_owned(),
            app_name: None,
            pid: None,
            bounds: Some(ComputerRect {
                x: bounds.0,
                y: bounds.1,
                width: bounds.2,
                height: bounds.3,
            }),
            focused: false,
            keyboard_focused: None,
            active: None,
            minimized: false,
            visible: true,
            space_ids: None,
            current_space_id: None,
            on_current_space: None,
            stacking_index: stacking,
            occluded_by: None,
        }
    }

    const fn at(x: f64, y: f64) -> ComputerPoint {
        ComputerPoint { x, y }
    }

    #[test]
    fn returns_the_only_containing_window_even_without_a_stacking_order() {
        let windows = [
            stacked("aside", (900.0, 0.0, 200.0, 200.0), None),
            stacked("hit", (0.0, 0.0, 800.0, 600.0), None),
        ];
        assert_eq!(
            topmost_window_at_point(&windows, at(400.0, 300.0)).map(|w| w.id.as_str()),
            Some("hit")
        );
    }

    #[test]
    fn ranks_overlapping_candidates_by_stacking_index_topmost_first() {
        let windows = [
            stacked("under", (0.0, 0.0, 800.0, 600.0), Some(2)),
            stacked("over", (200.0, 100.0, 400.0, 300.0), Some(0)),
            stacked("middle", (100.0, 50.0, 600.0, 500.0), Some(1)),
        ];
        assert_eq!(
            topmost_window_at_point(&windows, at(300.0, 200.0)).map(|w| w.id.as_str()),
            Some("over")
        );
    }

    #[test]
    fn refuses_to_rank_an_overlap_when_any_candidate_lacks_a_stacking_index() {
        let windows = [
            stacked("ranked", (0.0, 0.0, 800.0, 600.0), Some(0)),
            stacked("unranked", (200.0, 100.0, 400.0, 300.0), None),
        ];
        assert!(topmost_window_at_point(&windows, at(300.0, 200.0)).is_none());
    }

    #[test]
    fn skips_minimized_and_invisible_windows_and_misses_cleanly() {
        let mut minimized = stacked("minimized", (0.0, 0.0, 800.0, 600.0), None);
        minimized.minimized = true;
        let mut hidden = stacked("hidden", (0.0, 0.0, 800.0, 600.0), None);
        hidden.visible = false;
        assert!(topmost_window_at_point(&[minimized, hidden], at(400.0, 300.0)).is_none());
        assert!(topmost_window_at_point(&[], at(400.0, 300.0)).is_none());
    }

    #[test]
    fn reads_png_dimensions_from_the_header_and_maps_the_capture_scale() {
        let mut png = PNG_SIGNATURE.to_vec();
        png.extend_from_slice(&13_u32.to_be_bytes());
        png.extend_from_slice(b"IHDR");
        png.extend_from_slice(&200_u32.to_be_bytes());
        png.extend_from_slice(&100_u32.to_be_bytes());
        let shot = screenshot_from_png(ScreenshotFromPng {
            bytes: &png,
            region: ComputerRect {
                x: 10.0,
                y: 20.0,
                width: 100.0,
                height: 50.0,
            },
            captured_at: "2026-09-23T00:00:00.000Z".into(),
            source: None,
            bytes_base64: None,
        })
        .unwrap();
        assert_eq!((shot.width, shot.height, shot.scale), (200, 100, Some(2.0)));
        assert_eq!(
            read_png_dimensions(b"not a png", Some("The helper"))
                .unwrap_err()
                .to_string(),
            "The helper did not return a usable PNG image."
        );
    }
}
