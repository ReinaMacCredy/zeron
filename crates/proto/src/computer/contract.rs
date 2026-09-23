use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;

use super::bounds::{self, MAX_SAFE_INTEGER};
use super::driver::CuaEffect;

pub const COMPUTER_ID_MAX_LENGTH: usize = 128;
pub const COMPUTER_TEXT_MAX_LENGTH: usize = 16 * 1024;
pub const COMPUTER_LABEL_MAX_LENGTH: usize = 1_024;
pub const COMPUTER_MESSAGE_MAX_LENGTH: usize = 2_048;
pub const COMPUTER_DELIVERY_PATH_MAX_LENGTH: usize = 64;
pub const COMPUTER_WINDOW_LIST_MAX_LENGTH: usize = 512;
pub const COMPUTER_OCCLUDERS_MAX_LENGTH: usize = 32;
pub const COMPUTER_PROVISION_SUMMARY_MAX_LENGTH: usize = 4_096;
pub const COMPUTER_DRAG_MAX_DURATION_MS: u64 = 30_000;
pub const COMPUTER_HOTKEY_MAX_KEYS: usize = 16;
pub const COMPUTER_KEY_NAME_MAX_LENGTH: usize = 128;
pub const COMPUTER_SEMANTIC_ACTION_MAX_LENGTH: usize = 256;
pub const COMPUTER_SELECT_TEXT_RANGE_MAX: u64 = 4 * 1024 * 1024;
pub const COMPUTER_MODIFIERS_MAX_ITEMS: usize = 4;
pub const COMPUTER_WAIT_MAX_MS: u64 = 10_000;
pub const COMPUTER_INPUT_SCROLL_LIMIT: i64 = 4_096;
const COMPUTER_INPUT_COORDINATE_MAX: i64 = 32_767;
const COMPUTER_NODE_PATH_MAX_DEPTH: usize = 64;
const COMPUTER_UI_NODE_CHILDREN_MAX: usize = 2_048;
const COMPUTER_SCREEN_EXTENT_MAX: i64 = 32_768;
const COMPUTER_IMAGE_MAX_BYTES: i64 = 64 * 1024 * 1024;
const COMPUTER_IMAGE_BASE64_MAX_LENGTH: usize = 88 * 1024 * 1024;
const COMPUTER_PID_MAX: i64 = 0x7fff_ffff;
const COMPUTER_STATE_TEXT_MAX_LENGTH: usize = 4 * 1024 * 1024;

pub const COMPUTER_CONTROL_DENIED_ACTIVITY_KIND: &str = "computer.control-denied";
pub const COMPUTER_SETUP_REQUIRED_ACTIVITY_KIND: &str = "computer.setup-required";

pub const COMPUTER_KWIN_BACKEND: &str = "kwin";
pub const COMPUTER_NESTED_KWIN_BACKEND: &str = "nested-kwin";
pub const COMPUTER_HYPRLAND_BACKEND: &str = "hyprland";
pub const COMPUTER_MAC_BACKEND: &str = "mac";
pub const COMPUTER_RELEASE_CONTROL_HOTKEY: &str = "Meta+Shift+Esc";
pub const COMPUTER_RELEASE_HOTKEY_BACKENDS: &[&str] =
    &[COMPUTER_KWIN_BACKEND, COMPUTER_HYPRLAND_BACKEND];

pub const COMPUTER_FRAME_MAGIC: u16 = 0x5343;
pub const COMPUTER_FRAME_VERSION: u8 = 1;
pub const COMPUTER_FRAME_MAX_COMPUTER_ID_BYTES: usize = 255;

pub fn is_computer_id(value: &str) -> bool {
    bounds::chars_in(value, 1, COMPUTER_ID_MAX_LENGTH, |c| {
        c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | ':' | '-')
    })
}

fn computer_id<'de, D: Deserializer<'de>>(d: D) -> Result<String, D::Error> {
    let value = String::deserialize(d)?;
    let value = value.trim();
    if !is_computer_id(value) {
        return Err(serde::de::Error::custom(format!(
            "{value:?} is not a computer id"
        )));
    }
    Ok(value.to_string())
}

fn window_id<'de, D: Deserializer<'de>>(d: D) -> Result<String, D::Error> {
    bounds::trimmed::<D, COMPUTER_ID_MAX_LENGTH>(d)
}

fn opt_window_id<'de, D: Deserializer<'de>>(d: D) -> Result<Option<String>, D::Error> {
    bounds::opt_trimmed::<D, COMPUTER_ID_MAX_LENGTH>(d)
}

fn window_ids<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<String>, D::Error> {
    bounds::trimmed_items::<D, 0, { usize::MAX }, COMPUTER_ID_MAX_LENGTH>(d)
}

fn opt_window_ids<'de, D: Deserializer<'de>>(d: D) -> Result<Option<Vec<String>>, D::Error> {
    bounds::opt_trimmed_items::<D, COMPUTER_WINDOW_LIST_MAX_LENGTH, COMPUTER_ID_MAX_LENGTH>(d)
}

fn thread_id<'de, D: Deserializer<'de>>(d: D) -> Result<String, D::Error> {
    bounds::trimmed::<D, { usize::MAX }>(d)
}

fn opt_thread_id<'de, D: Deserializer<'de>>(d: D) -> Result<Option<String>, D::Error> {
    bounds::opt_trimmed::<D, { usize::MAX }>(d)
}

fn opt_pid<'de, D: Deserializer<'de>>(d: D) -> Result<Option<u32>, D::Error> {
    bounds::opt_int::<D, u32, 1, COMPUTER_PID_MAX>(d)
}

fn space_id<'de, D: Deserializer<'de>>(d: D) -> Result<u64, D::Error> {
    bounds::int::<D, u64, 1, MAX_SAFE_INTEGER>(d)
}

fn opt_space_id<'de, D: Deserializer<'de>>(d: D) -> Result<Option<u64>, D::Error> {
    bounds::opt_int::<D, u64, 1, MAX_SAFE_INTEGER>(d)
}

fn opt_space_ids<'de, D: Deserializer<'de>>(d: D) -> Result<Option<Vec<u64>>, D::Error> {
    #[derive(Deserialize)]
    struct SpaceId(#[serde(deserialize_with = "space_id")] u64);
    Ok(
        bounds::opt_items::<D, SpaceId, 0, COMPUTER_WINDOW_LIST_MAX_LENGTH>(d)?
            .map(|ids| ids.into_iter().map(|SpaceId(id)| id).collect()),
    )
}

fn screen_extent<'de, D: Deserializer<'de>>(d: D) -> Result<u32, D::Error> {
    bounds::int::<D, u32, 1, COMPUTER_SCREEN_EXTENT_MAX>(d)
}

fn image_bytes<'de, D: Deserializer<'de>>(d: D) -> Result<u64, D::Error> {
    bounds::int::<D, u64, 1, COMPUTER_IMAGE_MAX_BYTES>(d)
}

fn image_base64<'de, D: Deserializer<'de>>(d: D) -> Result<String, D::Error> {
    bounds::trimmed::<D, COMPUTER_IMAGE_BASE64_MAX_LENGTH>(d)
}

fn windows<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<ComputerWindow>, D::Error> {
    bounds::items::<D, ComputerWindow, 0, COMPUTER_WINDOW_LIST_MAX_LENGTH>(d)
}

fn modifiers<'de, D: Deserializer<'de>>(
    d: D,
) -> Result<Option<Vec<ComputerInputModifier>>, D::Error> {
    bounds::opt_items::<D, ComputerInputModifier, 0, COMPUTER_MODIFIERS_MAX_ITEMS>(d)
}

fn select_range<'de, D: Deserializer<'de>>(d: D) -> Result<u64, D::Error> {
    bounds::int::<D, u64, 0, { COMPUTER_SELECT_TEXT_RANGE_MAX as i64 }>(d)
}

fn input_coordinate<'de, D: Deserializer<'de>>(d: D) -> Result<u32, D::Error> {
    bounds::int::<D, u32, 0, COMPUTER_INPUT_COORDINATE_MAX>(d)
}

fn input_delta<'de, D: Deserializer<'de>>(d: D) -> Result<f64, D::Error> {
    bounds::within::<D, COMPUTER_INPUT_SCROLL_LIMIT>(d)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ComputerPermission {
    Accessibility,
    ScreenRecording,
    InputMonitoring,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ComputerBuildSignature {
    Adhoc,
    Signed,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ComputerSetupRequiredPayload {
    #[serde(deserialize_with = "bounds::trimmed::<_, 128>")]
    pub tool_name: String,
    #[serde(deserialize_with = "bounds::items::<_, _, 0, 8>")]
    pub missing: Vec<ComputerPermission>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub build_signature: Option<ComputerBuildSignature>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "bounds::opt_trimmed::<_, 256>"
    )]
    pub bundle_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "kebab-case",
    rename_all_fields = "camelCase"
)]
pub enum ComputerAvailability {
    Available {
        #[serde(
            default,
            skip_serializing_if = "Option::is_none",
            deserialize_with = "bounds::opt_trimmed::<_, 128>"
        )]
        backend: Option<String>,
    },
    UnsupportedPlatform {
        #[serde(deserialize_with = "bounds::trimmed::<_, 64>")]
        platform: String,
    },
    PermissionRequired {
        #[serde(deserialize_with = "bounds::items::<_, _, 1, 8>")]
        missing: Vec<ComputerPermission>,
        #[serde(deserialize_with = "bounds::trimmed::<_, COMPUTER_MESSAGE_MAX_LENGTH>")]
        message: String,
        build_signature: ComputerBuildSignature,
        #[serde(
            default,
            skip_serializing_if = "Option::is_none",
            deserialize_with = "bounds::opt_trimmed::<_, 256>"
        )]
        bundle_id: Option<String>,
    },
    BackendUnavailable {
        #[serde(deserialize_with = "bounds::trimmed::<_, COMPUTER_MESSAGE_MAX_LENGTH>")]
        message: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ComputerHealthStatus {
    Connected,
    Reconnecting,
    Unavailable,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ComputerHealthFailure {
    #[serde(deserialize_with = "bounds::trimmed::<_, COMPUTER_MESSAGE_MAX_LENGTH>")]
    pub message: String,
    pub at: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ComputerHealth {
    pub status: ComputerHealthStatus,
    pub consecutive_failures: u64,
    pub reconnects: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_failure: Option<ComputerHealthFailure>,
    pub capture_available: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub background_input_degraded: Option<bool>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ComputerCapabilities {
    pub windows: bool,
    pub window_bounds: bool,
    pub stacking: bool,
    pub capture: bool,
    pub input: bool,
    pub clipboard: bool,
    pub focus: bool,
    pub raise: bool,
    pub ghost_cursor: bool,
    pub visible_desktop: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ComputerRect {
    pub x: f64,
    pub y: f64,
    #[serde(deserialize_with = "bounds::non_negative")]
    pub width: f64,
    #[serde(deserialize_with = "bounds::non_negative")]
    pub height: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ComputerPoint {
    pub x: f64,
    pub y: f64,
}

pub type ComputerUiFrame = ComputerRect;
pub type ComputerUiPoint = ComputerPoint;

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ComputerScreenSize {
    #[serde(deserialize_with = "screen_extent")]
    pub width: u32,
    #[serde(deserialize_with = "screen_extent")]
    pub height: u32,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "bounds::positive"
    )]
    pub scale: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ComputerWindow {
    #[serde(deserialize_with = "window_id")]
    pub id: String,
    #[serde(deserialize_with = "bounds::text::<_, COMPUTER_LABEL_MAX_LENGTH>")]
    pub title: String,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "bounds::opt_text::<_, COMPUTER_LABEL_MAX_LENGTH>"
    )]
    pub app_name: Option<String>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "bounds::opt_int::<_, u32, 1, { u32::MAX as i64 }>"
    )]
    pub pid: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bounds: Option<ComputerRect>,
    pub focused: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub keyboard_focused: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active: Option<bool>,
    pub minimized: bool,
    pub visible: bool,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "opt_space_ids"
    )]
    pub space_ids: Option<Vec<u64>>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "opt_space_id"
    )]
    pub current_space_id: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub on_current_space: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stacking_index: Option<u64>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "opt_window_ids"
    )]
    pub occluded_by: Option<Vec<String>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ComputerAccessibilityRoot {
    Window,
    MenuBar,
    MenuBarExtra,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ComputerUiNode {
    #[serde(deserialize_with = "bounds::text::<_, 128>")]
    pub role: String,
    #[serde(deserialize_with = "bounds::opt_text::<_, COMPUTER_LABEL_MAX_LENGTH>")]
    pub label: Option<String>,
    #[serde(deserialize_with = "bounds::opt_text::<_, COMPUTER_TEXT_MAX_LENGTH>")]
    pub value: Option<String>,
    #[serde(deserialize_with = "bounds::opt_text::<_, COMPUTER_TEXT_MAX_LENGTH>")]
    pub description: Option<String>,
    pub frame: ComputerUiFrame,
    pub activation_point: Option<ComputerUiPoint>,
    pub on_screen: bool,
    #[serde(deserialize_with = "opt_window_id")]
    pub window_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub accessibility_root: Option<ComputerAccessibilityRoot>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "bounds::opt_items::<_, _, 0, COMPUTER_NODE_PATH_MAX_DEPTH>"
    )]
    pub node_path: Option<Vec<u32>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub editable: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub truncated: Option<bool>,
    #[serde(deserialize_with = "bounds::items::<_, _, 0, COMPUTER_UI_NODE_CHILDREN_MAX>")]
    pub children: Vec<ComputerUiNode>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ComputerPngMimeType {
    #[serde(rename = "image/png")]
    Png,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ComputerJpegMimeType {
    #[serde(rename = "image/jpeg")]
    Jpeg,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ComputerScreenshot {
    pub mime_type: ComputerPngMimeType,
    #[serde(deserialize_with = "screen_extent")]
    pub width: u32,
    #[serde(deserialize_with = "screen_extent")]
    pub height: u32,
    #[serde(deserialize_with = "image_bytes")]
    pub size_bytes: u64,
    #[serde(deserialize_with = "image_base64")]
    pub bytes_base64: String,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "opt_window_id"
    )]
    pub window_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub region: Option<ComputerRect>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "bounds::positive"
    )]
    pub scale: Option<f64>,
    pub captured_at: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ComputerInputPause {
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "opt_window_id"
    )]
    pub window_id: Option<String>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "opt_pid"
    )]
    pub pid: Option<u32>,
    #[serde(deserialize_with = "bounds::text::<_, COMPUTER_MESSAGE_MAX_LENGTH>")]
    pub message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ComputerAccessibilityStatus {
    Complete,
    Partial,
    Unavailable,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ComputerAccessibilityReport {
    pub status: ComputerAccessibilityStatus,
    #[serde(deserialize_with = "window_ids")]
    pub unavailable_window_ids: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ComputerState {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_pause: Option<ComputerInputPause>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "bounds::opt_text::<_, COMPUTER_MESSAGE_MAX_LENGTH>"
    )]
    pub preview_note: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub accessibility: Option<ComputerAccessibilityReport>,
    #[serde(deserialize_with = "computer_id")]
    pub computer_id: String,
    #[serde(deserialize_with = "windows")]
    pub windows: Vec<ComputerWindow>,
    pub screen_size: ComputerScreenSize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub availability: Option<ComputerAvailability>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub root: Option<ComputerUiNode>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "bounds::opt_text::<_, COMPUTER_STATE_TEXT_MAX_LENGTH>"
    )]
    pub text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub screenshot: Option<ComputerScreenshot>,
    pub captured_at: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadComputerState {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub control_generation: Option<u64>,
    #[serde(deserialize_with = "thread_id")]
    pub thread_id: String,
    pub version: u64,
    #[serde(deserialize_with = "computer_id")]
    pub computer_id: String,
    #[serde(deserialize_with = "windows")]
    pub windows: Vec<ComputerWindow>,
    pub screen_size: ComputerScreenSize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cursor: Option<ComputerPoint>,
    pub agent_active: bool,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "bounds::opt_text::<_, 128>"
    )]
    pub activity: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_pause: Option<ComputerInputPause>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "opt_thread_id"
    )]
    pub control_owner_thread_id: Option<String>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "bounds::opt_text::<_, COMPUTER_LABEL_MAX_LENGTH>"
    )]
    pub control_owner_label: Option<String>,
    pub controlled_by_other_thread: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shared_preview_unavailable: Option<bool>,
    pub availability: ComputerAvailability,
    pub health: ComputerHealth,
    pub capabilities: ComputerCapabilities,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_stopped: Option<bool>,
    #[serde(
        default,
        deserialize_with = "bounds::opt_text::<_, COMPUTER_MESSAGE_MAX_LENGTH>"
    )]
    pub last_error: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ComputerEmptyInput {}

pub type ComputerGetStatusInput = ComputerEmptyInput;
pub type ComputerProvisionInput = ComputerEmptyInput;
pub type ComputerListWindowsInput = ComputerEmptyInput;
pub type ComputerGetScreenSizeInput = ComputerEmptyInput;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ComputerStatusResult {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provisionable: Option<bool>,
    #[serde(deserialize_with = "computer_id")]
    pub computer_id: String,
    pub availability: ComputerAvailability,
    pub health: ComputerHealth,
    pub capabilities: ComputerCapabilities,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_stopped: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ComputerProvisionResult {
    #[serde(deserialize_with = "bounds::trimmed::<_, COMPUTER_PROVISION_SUMMARY_MAX_LENGTH>")]
    pub summary: String,
    pub status: ComputerStatusResult,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ComputerListWindowsResult {
    #[serde(deserialize_with = "computer_id")]
    pub computer_id: String,
    #[serde(deserialize_with = "windows")]
    pub windows: Vec<ComputerWindow>,
    pub availability: ComputerAvailability,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ComputerApp {
    #[serde(deserialize_with = "bounds::int::<_, u32, 0, { u32::MAX as i64 }>")]
    pub pid: u32,
    #[serde(deserialize_with = "bounds::text::<_, COMPUTER_LABEL_MAX_LENGTH>")]
    pub name: String,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "bounds::opt_text::<_, 512>"
    )]
    pub bundle_id: Option<String>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "bounds::opt_text::<_, 128>"
    )]
    pub team_id: Option<String>,
    pub running: bool,
    pub active: bool,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "bounds::opt_text::<_, 4096>"
    )]
    pub launch_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub window_count: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_used: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ComputerListAppsResult {
    #[serde(deserialize_with = "computer_id")]
    pub computer_id: String,
    #[serde(deserialize_with = "bounds::items::<_, _, 0, 1024>")]
    pub apps: Vec<ComputerApp>,
    pub availability: ComputerAvailability,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ComputerVerifyStatus {
    Satisfied,
    Unsatisfied,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ComputerVerifyStateResult {
    pub status: ComputerVerifyStatus,
    pub stable: bool,
    pub samples: u64,
    pub elapsed_ms: u64,
    #[serde(deserialize_with = "bounds::items::<_, _, 0, 8>")]
    pub predicates: Vec<Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ComputerZoomResult {
    pub mime_type: ComputerJpegMimeType,
    #[serde(deserialize_with = "screen_extent")]
    pub width: u32,
    #[serde(deserialize_with = "screen_extent")]
    pub height: u32,
    #[serde(deserialize_with = "image_bytes")]
    pub size_bytes: u64,
    #[serde(deserialize_with = "image_base64")]
    pub bytes_base64: String,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "opt_window_id"
    )]
    pub window_id: Option<String>,
    pub captured_at: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ComputerAccessibilityTreeApp {
    #[serde(deserialize_with = "bounds::int::<_, u32, 1, { u32::MAX as i64 }>")]
    pub pid: u32,
    #[serde(deserialize_with = "bounds::text::<_, COMPUTER_LABEL_MAX_LENGTH>")]
    pub name: String,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "bounds::opt_text::<_, 512>"
    )]
    pub bundle_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ComputerAccessibilityTreeWindow {
    #[serde(deserialize_with = "window_id")]
    pub id: String,
    #[serde(deserialize_with = "bounds::int::<_, u32, 1, { u32::MAX as i64 }>")]
    pub pid: u32,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "bounds::opt_text::<_, COMPUTER_LABEL_MAX_LENGTH>"
    )]
    pub app_name: Option<String>,
    #[serde(deserialize_with = "bounds::text::<_, COMPUTER_LABEL_MAX_LENGTH>")]
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bounds: Option<ComputerRect>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub on_screen: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub z_index: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ComputerAccessibilityTreeResult {
    #[serde(deserialize_with = "computer_id")]
    pub computer_id: String,
    #[serde(deserialize_with = "bounds::items::<_, _, 0, 1024>")]
    pub apps: Vec<ComputerAccessibilityTreeApp>,
    #[serde(deserialize_with = "bounds::items::<_, _, 0, COMPUTER_WINDOW_LIST_MAX_LENGTH>")]
    pub windows: Vec<ComputerAccessibilityTreeWindow>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "opt_window_id"
    )]
    pub window_id: Option<String>,
    pub truncated: bool,
    pub availability: ComputerAvailability,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ComputerCursorPosition {
    #[serde(deserialize_with = "computer_id")]
    pub computer_id: String,
    pub x: f64,
    pub y: f64,
    pub captured_at: String,
    pub availability: ComputerAvailability,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "opt_window_id"
    )]
    pub window_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inside_window: Option<bool>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ComputerGetStateInput {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub include_screenshot: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub include_text: Option<bool>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "opt_window_id"
    )]
    pub window_id: Option<String>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "bounds::opt_trimmed::<_, COMPUTER_LABEL_MAX_LENGTH>"
    )]
    pub label_contains: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ComputerGetScreenSizeResult {
    #[serde(deserialize_with = "computer_id")]
    pub computer_id: String,
    pub screen_size: ComputerScreenSize,
    pub availability: ComputerAvailability,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ComputerSetControlEnabledInput {
    #[serde(deserialize_with = "thread_id")]
    pub thread_id: String,
    pub enabled: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ComputerControlEnabledResult {
    pub enabled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub generation: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ComputerThreadInput {
    #[serde(deserialize_with = "thread_id")]
    pub thread_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ComputerLaunchAppInput {
    #[serde(deserialize_with = "bounds::trimmed::<_, 512>")]
    pub app: String,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "bounds::opt_text_items::<_, 128, 4096>"
    )]
    pub arguments: Option<Vec<String>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ComputerLaunchWindowStatus {
    Ready,
    NoUsableWindow,
    NotChecked,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ComputerLaunchWindowReason {
    NoWindow,
    Ambiguous,
    OffSpace,
    Hidden,
    InputUnavailable,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ComputerLaunchAppResult {
    #[serde(deserialize_with = "computer_id")]
    pub computer_id: String,
    #[serde(deserialize_with = "bounds::trimmed::<_, 512>")]
    pub app: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub focus_changed_during_launch: Option<bool>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "opt_pid"
    )]
    pub pid: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub window_status: Option<ComputerLaunchWindowStatus>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub window_reason: Option<ComputerLaunchWindowReason>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "bounds::opt_text::<_, 4096>"
    )]
    pub resolved_command: Option<String>,
    pub window: Option<ComputerWindow>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ComputerInputModifier {
    Ctrl,
    Alt,
    Shift,
    Meta,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ComputerTarget {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub x: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub y: Option<f64>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "bounds::opt_trimmed::<_, COMPUTER_LABEL_MAX_LENGTH>"
    )]
    pub label: Option<String>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "bounds::opt_trimmed::<_, 128>"
    )]
    pub role: Option<String>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "opt_window_id"
    )]
    pub window_id: Option<String>,
    #[serde(rename = "ref", default, skip_serializing_if = "Option::is_none")]
    pub element_ref: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ref_ordinal: Option<u64>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ComputerModifiedTarget {
    #[serde(flatten)]
    pub target: ComputerTarget,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "modifiers"
    )]
    pub modifiers: Option<Vec<ComputerInputModifier>>,
}

pub type ComputerClickInput = ComputerModifiedTarget;
pub type ComputerDoubleClickInput = ComputerModifiedTarget;
pub type ComputerTripleClickInput = ComputerModifiedTarget;
pub type ComputerRightClickInput = ComputerModifiedTarget;
pub type ComputerMoveCursorInput = ComputerTarget;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ComputerActivateWindowInput {
    #[serde(deserialize_with = "window_id")]
    pub window_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ComputerWaitInput {
    #[serde(deserialize_with = "bounds::int::<_, u64, 0, { COMPUTER_WAIT_MAX_MS as i64 }>")]
    pub duration_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ComputerDragInput {
    pub from: ComputerTarget,
    pub to: ComputerTarget,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "bounds::opt_int::<_, u64, 0, { COMPUTER_DRAG_MAX_DURATION_MS as i64 }>"
    )]
    pub duration_ms: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ComputerScrollInput {
    #[serde(flatten)]
    pub target: ComputerModifiedTarget,
    pub delta_x: f64,
    pub delta_y: f64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ComputerTypeTextInput {
    #[serde(deserialize_with = "bounds::text::<_, COMPUTER_TEXT_MAX_LENGTH>")]
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ComputerPressKeyInput {
    #[serde(deserialize_with = "bounds::trimmed::<_, 128>")]
    pub key: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ComputerHotkeyInput {
    #[serde(
        deserialize_with = "bounds::trimmed_items::<_, 1, COMPUTER_HOTKEY_MAX_KEYS, COMPUTER_KEY_NAME_MAX_LENGTH>"
    )]
    pub keys: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ComputerSetValueInput {
    #[serde(flatten)]
    pub target: ComputerTarget,
    #[serde(deserialize_with = "bounds::text::<_, COMPUTER_TEXT_MAX_LENGTH>")]
    pub value: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ComputerPerformActionInput {
    #[serde(flatten)]
    pub target: ComputerTarget,
    #[serde(deserialize_with = "bounds::trimmed::<_, COMPUTER_SEMANTIC_ACTION_MAX_LENGTH>")]
    pub action: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ComputerSelectTextInput {
    #[serde(flatten)]
    pub target: ComputerTarget,
    #[serde(deserialize_with = "select_range")]
    pub start: u64,
    #[serde(deserialize_with = "select_range")]
    pub length: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ComputerInputButton {
    Left,
    Right,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ComputerInputClickInput {
    #[serde(deserialize_with = "input_coordinate")]
    pub x: u32,
    #[serde(deserialize_with = "input_coordinate")]
    pub y: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub button: Option<ComputerInputButton>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "bounds::opt_int::<_, u8, 1, 2>"
    )]
    pub click_count: Option<u8>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ComputerInputScrollInput {
    #[serde(deserialize_with = "input_coordinate")]
    pub x: u32,
    #[serde(deserialize_with = "input_coordinate")]
    pub y: u32,
    #[serde(deserialize_with = "input_delta")]
    pub delta_x: f64,
    #[serde(deserialize_with = "input_delta")]
    pub delta_y: f64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ComputerInputKeyInput {
    #[serde(deserialize_with = "bounds::trimmed::<_, 64>")]
    pub key: String,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "modifiers"
    )]
    pub modifiers: Option<Vec<ComputerInputModifier>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ComputerDeliveryVerification {
    Confirmed,
    Unconfirmed,
    Unverifiable,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ComputerScrollDelta {
    pub delta_x: f64,
    pub delta_y: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ComputerScrollTelemetry {
    pub requested: ComputerScrollDelta,
    pub injected: ComputerScrollDelta,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limited_to: Option<ComputerScrollDelta>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub traveled_y: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gearing: Option<f64>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "bounds::opt_text_items::<_, 4, 32>"
    )]
    pub routes: Option<Vec<String>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ComputerDelivery {
    #[serde(deserialize_with = "bounds::text::<_, COMPUTER_DELIVERY_PATH_MAX_LENGTH>")]
    pub path: String,
    pub verified: ComputerDeliveryVerification,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effect: Option<CuaEffect>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ComputerActionResult {
    #[serde(deserialize_with = "computer_id")]
    pub computer_id: String,
    #[serde(deserialize_with = "bounds::trimmed::<_, 256>")]
    pub action: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub point: Option<ComputerPoint>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub clamped_to: Option<ComputerPoint>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "opt_window_id"
    )]
    pub window_id: Option<String>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "bounds::opt_text::<_, COMPUTER_TEXT_MAX_LENGTH>"
    )]
    pub value: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scroll: Option<ComputerScrollTelemetry>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delivery: Option<ComputerDelivery>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ComputerFrameHeader {
    #[serde(deserialize_with = "computer_id")]
    pub computer_id: String,
    pub sequence: u32,
    pub timestamp_ms: f64,
    pub keyframe: bool,
    pub codec_config: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ComputerFrameDecodeErrorReason {
    TooShort,
    BadMagic,
    UnsupportedVersion,
    TruncatedComputerId,
    InvalidComputerId,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ComputerRestoreStatus {
    Restored,
    RestoreMissed,
    AlreadyFrontmost,
    FrontmostUnobservable,
}

impl ComputerRestoreStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Restored => "restored",
            Self::RestoreMissed => "restore-missed",
            Self::AlreadyFrontmost => "already-frontmost",
            Self::FrontmostUnobservable => "frontmost-unobservable",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ComputerConsentScope {
    Task,
    Call,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all_fields = "camelCase")]
pub enum ComputerEvent {
    #[serde(rename = "computer.thread-state")]
    ThreadState { state: Box<ThreadComputerState> },
    #[serde(rename = "computer.windows-changed")]
    WindowsChanged {
        #[serde(deserialize_with = "windows")]
        windows: Vec<ComputerWindow>,
    },
    #[serde(rename = "computer.action")]
    Action {
        #[serde(
            default,
            skip_serializing_if = "Option::is_none",
            deserialize_with = "opt_window_id"
        )]
        window_id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        delivery: Option<ComputerDelivery>,
        #[serde(deserialize_with = "bounds::trimmed::<_, 256>")]
        action: String,
        ok: bool,
        #[serde(
            default,
            skip_serializing_if = "Option::is_none",
            deserialize_with = "bounds::opt_text::<_, COMPUTER_MESSAGE_MAX_LENGTH>"
        )]
        message: Option<String>,
        #[serde(
            default,
            skip_serializing_if = "Option::is_none",
            deserialize_with = "opt_thread_id"
        )]
        thread_id: Option<String>,
        #[serde(
            default,
            skip_serializing_if = "Option::is_none",
            deserialize_with = "opt_window_id"
        )]
        restored_window_id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        restore_status: Option<ComputerRestoreStatus>,
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        masked: bool,
    },
    #[serde(rename = "computer.frame")]
    Frame { header: ComputerFrameHeader },
    #[serde(rename = "computer.open-pane-requested")]
    OpenPaneRequested {
        #[serde(deserialize_with = "thread_id")]
        thread_id: String,
    },
    #[serde(rename = "computer.input-stopped")]
    InputStopped { stopped: bool },
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::de::DeserializeOwned;
    use serde_json::json;

    fn decode<T: DeserializeOwned>(value: &Value) -> Result<T, serde_json::Error> {
        serde_json::from_value(value.clone())
    }

    fn round_trips<T: DeserializeOwned + Serialize>(value: &Value) {
        let decoded: T = decode(value).unwrap_or_else(|e| panic!("{value}: {e}"));
        assert_eq!(&serde_json::to_value(decoded).unwrap(), value);
    }

    fn rejects<T: DeserializeOwned + std::fmt::Debug>(value: &Value) {
        assert!(decode::<T>(value).is_err(), "accepted {value}");
    }

    fn merge(base: &Value, patch: Value) -> Value {
        let mut merged = base.clone();
        for (key, value) in patch.as_object().unwrap() {
            merged[key] = value.clone();
        }
        merged
    }

    fn window() -> Value {
        json!({ "id": "cua:1:2", "title": "Fixture", "focused": false, "minimized": false, "visible": false })
    }

    #[test]
    fn window_preserves_per_display_membership_without_making_it_mandatory() {
        for patch in [
            json!({}),
            json!({ "spaceIds": [], "onCurrentSpace": false }),
            json!({ "focused": true, "keyboardFocused": false }),
            json!({ "keyboardFocused": true }),
            json!({ "spaceIds": [3, 8], "currentSpaceId": 8, "onCurrentSpace": true }),
        ] {
            round_trips::<ComputerWindow>(&merge(&window(), patch));
        }
    }

    #[test]
    fn window_rejects_invalid_or_lossy_native_space_identifiers() {
        for id in [
            json!(0),
            json!(-1),
            json!(1.5),
            json!(MAX_SAFE_INTEGER + 1),
            json!("3"),
        ] {
            rejects::<ComputerWindow>(&merge(&window(), json!({ "spaceIds": [id] })));
            rejects::<ComputerWindow>(&merge(&window(), json!({ "currentSpaceId": id })));
        }
    }

    fn permission_required() -> Value {
        json!({
            "kind": "permission-required",
            "missing": ["accessibility", "screenRecording"],
            "message": "Zeron needs Accessibility and Screen Recording to control this Mac.",
            "buildSignature": "adhoc",
        })
    }

    fn availability_decodes(value: Value) -> bool {
        decode::<ComputerAvailability>(&value).is_ok()
    }

    #[test]
    fn availability_round_trips_the_grants_the_message_and_the_build_signature() {
        round_trips::<ComputerAvailability>(&permission_required());
    }

    #[test]
    fn availability_keeps_the_other_kinds_decodable() {
        assert!(availability_decodes(
            json!({ "kind": "available", "backend": "mac" })
        ));
        assert!(availability_decodes(
            json!({ "kind": "unsupported-platform", "platform": "linux" })
        ));
        assert!(availability_decodes(
            json!({ "kind": "backend-unavailable", "message": "No helper." })
        ));
    }

    #[test]
    fn availability_preserves_the_responsible_app_across_the_wire() {
        let state = merge(
            &permission_required(),
            json!({ "bundleId": "com.emanueledipietro.synara.dev" }),
        );
        round_trips::<ComputerAvailability>(&state);
        assert!(!availability_decodes(merge(
            &state,
            json!({ "bundleId": "" })
        )));
        assert!(!availability_decodes(merge(
            &state,
            json!({ "bundleId": "x".repeat(257) })
        )));
    }

    #[test]
    fn availability_refuses_a_permission_state_that_names_no_grant() {
        assert!(!availability_decodes(merge(
            &permission_required(),
            json!({ "missing": [] })
        )));
    }

    #[test]
    fn availability_refuses_an_unknown_grant_name_and_an_unknown_signature() {
        let base = permission_required();
        assert!(availability_decodes(merge(
            &base,
            json!({ "missing": ["inputMonitoring"] })
        )));
        assert!(!availability_decodes(merge(
            &base,
            json!({ "missing": ["camera"] })
        )));
        assert!(!availability_decodes(merge(
            &base,
            json!({ "buildSignature": "notarized" })
        )));
    }

    #[test]
    fn setup_payload_round_trips_the_tool_grants_signature_and_responsible_app() {
        round_trips::<ComputerSetupRequiredPayload>(&json!({
            "toolName": "computer_list_windows",
            "missing": ["accessibility"],
            "buildSignature": "adhoc",
            "bundleId": "com.emanueledipietro.synara.dev",
        }));
    }

    #[test]
    fn setup_payload_accepts_no_signature_and_refuses_an_unknown_one() {
        let decodes = |value: Value| decode::<ComputerSetupRequiredPayload>(&value).is_ok();
        assert!(decodes(
            json!({ "toolName": "computer_click", "missing": [] })
        ));
        assert!(!decodes(
            json!({ "toolName": "computer_click", "missing": [], "buildSignature": "notarized" })
        ));
        assert!(decodes(
            json!({ "toolName": "computer_click", "missing": ["inputMonitoring"] })
        ));
        assert!(!decodes(
            json!({ "toolName": "computer_click", "missing": ["camera"] })
        ));
    }

    fn status() -> Value {
        json!({
            "computerId": "desktop",
            "availability": { "kind": "available", "backend": "mac" },
            "health": {
                "status": "connected",
                "consecutiveFailures": 0,
                "reconnects": 0,
                "captureAvailable": true,
            },
            "capabilities": {
                "windows": true,
                "windowBounds": true,
                "stacking": false,
                "capture": true,
                "input": true,
                "clipboard": false,
                "focus": true,
                "raise": true,
                "ghostCursor": true,
                "visibleDesktop": true,
            },
        })
    }

    fn state() -> Value {
        json!({
            "computerId": "desktop",
            "windows": [],
            "screenSize": { "width": 1440, "height": 900 },
            "capturedAt": "2026-09-08T00:00:00.000Z",
        })
    }

    fn thread_state() -> Value {
        merge(
            &status(),
            json!({
                "threadId": "computer-test-thread",
                "version": 1,
                "windows": [],
                "screenSize": state()["screenSize"],
                "agentActive": false,
                "controlledByOtherThread": false,
                "lastError": null,
            }),
        )
    }

    #[test]
    fn round_trips_pause_details_through_perception_and_thread_state() {
        let input_pause =
            json!({ "windowId": "cua:123:456", "message": "Waiting for this window." });
        round_trips::<ComputerState>(&merge(&state(), json!({ "inputPause": input_pause })));
        round_trips::<ThreadComputerState>(&merge(
            &thread_state(),
            json!({ "inputPause": input_pause, "activity": "Waiting for you" }),
        ));
    }

    #[test]
    fn keeps_older_state_and_status_payloads_valid_without_optional_additions() {
        round_trips::<ComputerState>(&state());
        round_trips::<ThreadComputerState>(&thread_state());
        round_trips::<ComputerStatusResult>(&status());
    }

    #[test]
    fn bounds_pause_messages_and_window_identifiers_at_the_protocol_boundary() {
        let window_id = "w".repeat(COMPUTER_ID_MAX_LENGTH);
        let message = "m".repeat(COMPUTER_MESSAGE_MAX_LENGTH);
        let pause = json!({ "windowId": window_id, "message": message });
        round_trips::<ComputerInputPause>(&pause);
        rejects::<ComputerInputPause>(&merge(&pause, json!({ "message": format!("{message}m") })));
        rejects::<ComputerInputPause>(&merge(
            &pause,
            json!({ "windowId": format!("{window_id}w") }),
        ));
        round_trips::<ComputerInputPause>(&json!({ "message": "Paused" }));
    }

    #[test]
    fn rejects_unbounded_activity_text() {
        let bounded = merge(&thread_state(), json!({ "activity": "a".repeat(128) }));
        round_trips::<ThreadComputerState>(&bounded);
        rejects::<ThreadComputerState>(&merge(&bounded, json!({ "activity": "a".repeat(129) })));
    }

    #[test]
    fn retains_an_explicit_provisionable_status() {
        for provisionable in [true, false] {
            round_trips::<ComputerStatusResult>(&merge(
                &status(),
                json!({ "provisionable": provisionable }),
            ));
        }
    }

    fn scroll_result() -> Value {
        json!({
            "computerId": "desktop",
            "action": "scroll",
            "scroll": {
                "requested": { "deltaX": 0.0, "deltaY": 1200.0 },
                "injected": { "deltaX": 0.0, "deltaY": 600.0 },
                "limitedTo": { "deltaX": 0.0, "deltaY": 600.0 },
            },
            "delivery": { "path": "cua", "verified": "unconfirmed", "effect": "dispatched-unknown" },
        })
    }

    #[test]
    fn action_result_preserves_the_overlap_limit_and_cua_effect_telemetry() {
        round_trips::<ComputerActionResult>(&scroll_result());
    }

    #[test]
    fn action_result_rejects_non_finite_limited_distances() {
        let mut result = scroll_result();
        result["scroll"]["limitedTo"]["deltaY"] = json!(f64::INFINITY);
        rejects::<ComputerActionResult>(&result);
    }

    #[test]
    fn select_text_accepts_a_semantic_target_with_an_in_bounds_range() {
        round_trips::<ComputerSelectTextInput>(
            &json!({ "label": "Display", "start": 2, "length": 5 }),
        );
        round_trips::<ComputerSelectTextInput>(
            &json!({ "label": "Display", "start": 4, "length": 0 }),
        );
        round_trips::<ComputerSelectTextInput>(
            &json!({ "label": "Display", "start": 0, "length": COMPUTER_SELECT_TEXT_RANGE_MAX }),
        );
    }

    #[test]
    fn select_text_requires_the_range_and_rejects_negative_fractional_and_over_max_values() {
        for bad in [
            json!({ "label": "Display" }),
            json!({ "label": "Display", "start": 0 }),
            json!({ "label": "Display", "start": -1, "length": 1 }),
            json!({ "label": "Display", "start": 0, "length": -1 }),
            json!({ "label": "Display", "start": 1.5, "length": 1 }),
            json!({ "label": "Display", "start": 0, "length": f64::NAN }),
            json!({ "label": "Display", "start": 0, "length": COMPUTER_SELECT_TEXT_RANGE_MAX + 1 }),
            json!({ "label": "Display", "start": COMPUTER_SELECT_TEXT_RANGE_MAX + 1, "length": 0 }),
        ] {
            rejects::<ComputerSelectTextInput>(&bad);
        }
    }

    #[test]
    fn launch_result_preserves_identity_and_failed_readiness_independently() {
        round_trips::<ComputerLaunchAppResult>(&json!({
            "computerId": "desktop",
            "app": "com.vendor.Helium",
            "pid": 42,
            "window": null,
            "windowStatus": "no_usable_window",
            "windowReason": "off_space",
            "focusChangedDuringLaunch": true,
        }));
        round_trips::<ComputerLaunchAppResult>(
            &json!({ "computerId": "desktop", "app": "Helium", "window": null }),
        );
    }

    #[test]
    fn pause_retains_the_affected_app_and_refuses_invalid_process_identities() {
        let pause = json!({ "windowId": "cua:42:7", "pid": 42, "message": "Observe again" });
        round_trips::<ComputerInputPause>(&pause);
        for pid in [json!(0), json!(-1), json!(1.5), json!(0x8000_0000u32)] {
            rejects::<ComputerInputPause>(&merge(&pause, json!({ "pid": pid })));
        }
    }

    #[test]
    fn trimmed_strings_trim_like_effect_schema_trim() {
        let payload: ComputerSetupRequiredPayload =
            decode(&json!({ "toolName": "  computer_click ", "missing": [] })).unwrap();
        assert_eq!(payload.tool_name, "computer_click");
        rejects::<ComputerSetupRequiredPayload>(&json!({ "toolName": "   ", "missing": [] }));
    }

    #[test]
    fn computer_id_requires_its_character_class() {
        assert!(is_computer_id("desktop"));
        assert!(is_computer_id("cua:1.2_x-y"));
        for bad in ["", " desktop", "has space", "slash/", &"x".repeat(129)] {
            assert!(!is_computer_id(bad), "{bad}");
        }
        rejects::<ComputerState>(&merge(&state(), json!({ "computerId": "bad id" })));
    }

    #[test]
    fn click_input_flattens_the_target_and_bounds_modifiers() {
        let click = json!({ "x": 10.0, "y": 20.0, "ref": 3, "modifiers": ["shift", "meta"] });
        round_trips::<ComputerClickInput>(&click);
        let decoded: ComputerClickInput = decode(&click).unwrap();
        assert_eq!(decoded.target.element_ref, Some(3));
        rejects::<ComputerClickInput>(
            &json!({ "modifiers": ["shift", "meta", "alt", "ctrl", "shift"] }),
        );
        rejects::<ComputerClickInput>(&json!({ "modifiers": ["hyper"] }));
    }

    #[test]
    fn hotkey_requires_one_to_sixteen_bounded_keys() {
        round_trips::<ComputerHotkeyInput>(&json!({ "keys": ["cmd", "c"] }));
        rejects::<ComputerHotkeyInput>(&json!({ "keys": [] }));
        rejects::<ComputerHotkeyInput>(&json!({ "keys": vec!["a"; 17] }));
        rejects::<ComputerHotkeyInput>(&json!({ "keys": [" "] }));
    }

    #[test]
    fn events_use_the_dotted_type_tags() {
        round_trips::<ComputerEvent>(&json!({ "type": "computer.input-stopped", "stopped": true }));
        round_trips::<ComputerEvent>(
            &json!({ "type": "computer.open-pane-requested", "threadId": "t1" }),
        );
        round_trips::<ComputerEvent>(&json!({
            "type": "computer.action",
            "windowId": "cua:1:2",
            "action": "click",
            "ok": true,
            "delivery": { "path": "ax", "verified": "confirmed" },
        }));
        round_trips::<ComputerEvent>(&json!({
            "type": "computer.action",
            "windowId": "cua:1:2",
            "action": "computer_activate_window",
            "ok": true,
            "threadId": "t1",
            "restoredWindowId": "cua:1:3",
            "restoreStatus": "restore-missed",
            "masked": true,
        }));
        round_trips::<ComputerEvent>(&json!({
            "type": "computer.frame",
            "header": { "computerId": "desktop", "sequence": 4, "timestampMs": 1.5, "keyframe": true, "codecConfig": false },
        }));
        round_trips::<ComputerEvent>(
            &json!({ "type": "computer.thread-state", "state": thread_state() }),
        );
        round_trips::<ComputerEvent>(
            &json!({ "type": "computer.windows-changed", "windows": [window()] }),
        );
    }

    #[test]
    fn ui_node_nests_and_keeps_null_fields_on_the_wire() {
        round_trips::<ComputerUiNode>(&json!({
            "role": "AXWindow",
            "label": null,
            "value": null,
            "description": null,
            "frame": { "x": 0.0, "y": 0.0, "width": 10.0, "height": 10.0 },
            "activationPoint": null,
            "onScreen": true,
            "windowId": null,
            "nodePath": [0, 2],
            "children": [{
                "role": "AXButton",
                "label": "OK",
                "value": null,
                "description": null,
                "frame": { "x": 1.0, "y": 1.0, "width": 2.0, "height": 2.0 },
                "activationPoint": { "x": 2.0, "y": 2.0 },
                "onScreen": true,
                "windowId": "cua:1:2",
                "accessibilityRoot": "menu-bar-extra",
                "children": [],
            }],
        }));
    }
}
