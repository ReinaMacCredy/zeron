use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

pub const LOCAL_DEVICE_HOST_ID: &str = "local";

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DevicePlatform {
    #[default]
    Ios,
    Android,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DeviceHostStatus {
    #[default]
    Disabled,
    Idle,
    Installing,
    Starting,
    Ready,
    Failed,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HubDevice {
    #[serde(default)]
    pub host_id: String,
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub platform: DevicePlatform,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub booted: bool,
    #[serde(default)]
    pub physical: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DevicePlatformAvailability {
    #[serde(default)]
    pub platform: DevicePlatform,
    #[serde(default)]
    pub available: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceHubHost {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub kind: String,
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub platforms: Vec<DevicePlatformAvailability>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tools: Option<DeviceToolVersions>,
    #[serde(default)]
    pub hub_installed: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceToolVersion {
    #[serde(default)]
    pub required: String,
    #[serde(default)]
    pub installed: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub running: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceToolVersions {
    #[serde(default)]
    pub hub: DeviceToolVersion,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceSession {
    #[serde(default)]
    pub chat_id: String,
    #[serde(default)]
    pub host_id: String,
    #[serde(default)]
    pub device_id: String,
    #[serde(default)]
    pub platform: DevicePlatform,
    #[serde(default)]
    pub opened_at: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceBooting {
    #[serde(flatten)]
    pub device: HubDevice,
    #[serde(default)]
    pub chat_id: String,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeviceOrientation {
    #[default]
    Portrait,
    LandscapeLeft,
    PortraitUpsideDown,
    LandscapeRight,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceScreenConfig {
    #[serde(default)]
    pub width: u32,
    #[serde(default)]
    pub height: u32,
    #[serde(default)]
    pub orientation: DeviceOrientation,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub supports_physical_orientation: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub supports_hinge_angle: Option<bool>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum StreamLinkStatus {
    #[default]
    Idle,
    Connecting,
    Live,
    Failed {
        #[serde(default)]
        detail: String,
    },
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceStreamStatus {
    #[serde(default)]
    pub video: StreamLinkStatus,
    #[serde(default)]
    pub input: StreamLinkStatus,
    #[serde(default)]
    pub subscribers: u32,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceHubSettings {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub agent_access_enabled: bool,
    #[serde(default)]
    pub onboarding_completed: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceHubState {
    #[serde(default)]
    pub revision: u64,
    #[serde(default)]
    pub settings: DeviceHubSettings,
    #[serde(default)]
    pub hosts: Vec<DeviceHubHost>,
    #[serde(default)]
    pub host_status: DeviceHostStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub host_status_detail: Option<String>,
    #[serde(default)]
    pub devices: Vec<HubDevice>,
    #[serde(default)]
    pub sessions: Vec<DeviceSession>,
    #[serde(default)]
    pub booting: Vec<DeviceBooting>,
    #[serde(default)]
    pub streams: BTreeMap<String, DeviceStreamStatus>,
    #[serde(default)]
    pub screens: BTreeMap<String, DeviceScreenConfig>,
    #[serde(default)]
    pub agent_status: DeviceHostStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_status_detail: Option<String>,
}

impl DeviceHubState {
    pub fn device(&self, host_id: &str, device_id: &str) -> Option<&HubDevice> {
        self.devices
            .iter()
            .find(|device| device.host_id == host_id && device.id == device_id)
    }

    pub fn session(&self, chat_id: &str, host_id: &str, device_id: &str) -> Option<&DeviceSession> {
        self.sessions.iter().find(|session| {
            session.chat_id == chat_id
                && session.host_id == host_id
                && session.device_id == device_id
        })
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceFrame {
    #[serde(default)]
    pub seq: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub width: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub height: Option<u32>,
    #[serde(default)]
    pub jpeg: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TouchPhase {
    Begin,
    Move,
    End,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeviceButton {
    Home,
    AppSwitcher,
    Lock,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum KeyPhase {
    Down,
    Up,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DeviceInput {
    Touch { phase: TouchPhase, x: f32, y: f32 },
    Button { button: DeviceButton },
    Key { phase: KeyPhase, usage: u16 },
    Orientation { orientation: DeviceOrientation },
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfigureDeviceHubParams {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_access_enabled: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub onboarding_completed: Option<bool>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListHubDevicesParams {
    #[serde(default)]
    pub inspect_only: bool,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OpenHubDeviceParams {
    #[serde(default)]
    pub chat_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub host_id: Option<String>,
    #[serde(default)]
    pub device_id: String,
    #[serde(default)]
    pub platform: DevicePlatform,
    #[serde(default = "default_true")]
    pub boot: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WatchHubDeviceFramesParams {
    #[serde(default)]
    pub device_id: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SendHubDeviceInputParams {
    #[serde(default)]
    pub device_id: String,
    pub input: DeviceInput,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CloseHubDeviceParams {
    #[serde(default)]
    pub chat_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub host_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device_id: Option<String>,
    #[serde(default)]
    pub shutdown: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceToolParams {
    #[serde(default)]
    pub chat_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub host_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub platform: Option<DevicePlatform>,
    #[serde(default)]
    pub shutdown: bool,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub run_secret: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ShutdownHubDeviceParams {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub host_id: Option<String>,
    #[serde(default)]
    pub device_id: String,
    #[serde(default)]
    pub platform: DevicePlatform,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DeviceAppearance {
    Light,
    Dark,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum DeviceTextSize {
    Small,
    Default,
    Large,
    ExtraLarge,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DeviceToggle {
    ReduceMotion,
    IncreaseContrast,
    ReduceTransparency,
    ShowBorders,
    VoiceOver,
    NetworkEnabled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LiquidGlass {
    Clear,
    Tinted,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ColorFilter {
    None,
    Grayscale,
    RedGreen,
    GreenRed,
    BlueYellow,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum DevicePermission {
    Camera,
    Microphone,
    Photos,
    Contacts,
    Calendar,
    Reminders,
    Location,
    Notifications,
    Motion,
    MediaLibrary,
    #[serde(rename = "faceid")]
    FaceId,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PermissionDecision {
    Grant,
    Revoke,
    Reset,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum DeviceAction {
    SetAppearance {
        value: DeviceAppearance,
    },
    SetTextSize {
        value: DeviceTextSize,
    },
    SetToggle {
        setting: DeviceToggle,
        enabled: bool,
    },
    SetLiquidGlass {
        value: LiquidGlass,
    },
    SetColorFilter {
        value: ColorFilter,
    },
    SetOrientation {
        value: DeviceOrientation,
    },
    SetLocation {
        lat: f64,
        lon: f64,
    },
    ClearLocation,
    SetPermission {
        app_id: String,
        permission: DevicePermission,
        decision: PermissionDecision,
    },
    OpenUrl {
        url: String,
    },
    LaunchApp {
        app_id: String,
    },
    TerminateApp {
        app_id: String,
    },
    SendPush {
        app_id: String,
        payload: serde_json::Value,
    },
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceSettingsSnapshot {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub appearance: Option<DeviceAppearance>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text_size: Option<DeviceTextSize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reduce_motion: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub increase_contrast: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reduce_transparency: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub show_borders: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub voice_over: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub liquid_glass: Option<LiquidGlass>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color_filter: Option<ColorFilter>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ForegroundApp {
    #[serde(default)]
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pid: Option<u32>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceDetail {
    #[serde(default)]
    pub host_id: String,
    #[serde(default)]
    pub device_id: String,
    #[serde(default)]
    pub settings: DeviceSettingsSnapshot,
    #[serde(default)]
    pub foreground_app: Option<ForegroundApp>,
    #[serde(default)]
    pub read_at: String,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AxRect {
    #[serde(default)]
    pub x: f64,
    #[serde(default)]
    pub y: f64,
    #[serde(default)]
    pub width: f64,
    #[serde(default)]
    pub height: f64,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceAxNode {
    #[serde(default)]
    pub frame: AxRect,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub children: Vec<DeviceAxNode>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceAxTree {
    #[serde(default)]
    pub nodes: Vec<DeviceAxNode>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceAppState {
    #[serde(default)]
    pub foreground_app: Option<ForegroundApp>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceEventLogEntry {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub timestamp: String,
    #[serde(default)]
    pub kind: String,
    #[serde(default)]
    pub summary: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceEventLogBatch {
    #[serde(default)]
    pub reset: bool,
    #[serde(default)]
    pub events: Vec<DeviceEventLogEntry>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HubDeviceTargetParams {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub host_id: Option<String>,
    #[serde(default)]
    pub device_id: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunHubDeviceActionParams {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub host_id: Option<String>,
    #[serde(default)]
    pub device_id: String,
    pub action: DeviceAction,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HubDeviceScreenshot {
    #[serde(default)]
    pub path: String,
    #[serde(default)]
    pub width: u32,
    #[serde(default)]
    pub height: u32,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn device_actions_have_stable_wire_shapes() {
        let cases = [
            (
                DeviceAction::SetAppearance {
                    value: DeviceAppearance::Dark,
                },
                json!({"setAppearance": {"value": "dark"}}),
            ),
            (
                DeviceAction::SetTextSize {
                    value: DeviceTextSize::ExtraLarge,
                },
                json!({"setTextSize": {"value": "extra-large"}}),
            ),
            (
                DeviceAction::SetToggle {
                    setting: DeviceToggle::VoiceOver,
                    enabled: true,
                },
                json!({"setToggle": {"setting": "voiceOver", "enabled": true}}),
            ),
            (
                DeviceAction::SetColorFilter {
                    value: ColorFilter::BlueYellow,
                },
                json!({"setColorFilter": {"value": "blue-yellow"}}),
            ),
            (
                DeviceAction::SetLocation {
                    lat: 35.6762,
                    lon: 139.6503,
                },
                json!({"setLocation": {"lat": 35.6762, "lon": 139.6503}}),
            ),
            (DeviceAction::ClearLocation, json!("clearLocation")),
            (
                DeviceAction::SetPermission {
                    app_id: "com.apple.Maps".into(),
                    permission: DevicePermission::FaceId,
                    decision: PermissionDecision::Grant,
                },
                json!({"setPermission": {"appId": "com.apple.Maps", "permission": "faceid", "decision": "grant"}}),
            ),
            (
                DeviceAction::SendPush {
                    app_id: "a.b".into(),
                    payload: json!("hi"),
                },
                json!({"sendPush": {"appId": "a.b", "payload": "hi"}}),
            ),
        ];
        for (action, wire) in cases {
            assert_eq!(serde_json::to_value(&action).unwrap(), wire);
            assert_eq!(
                serde_json::from_value::<DeviceAction>(wire).unwrap(),
                action
            );
        }
        assert_eq!(
            serde_json::to_value(DevicePermission::MediaLibrary).unwrap(),
            json!("media-library")
        );
        let detail: DeviceDetail = serde_json::from_value(json!({"deviceId": "SIM"})).unwrap();
        assert_eq!(detail.settings, DeviceSettingsSnapshot::default());
        assert!(detail.foreground_app.is_none());
    }

    #[test]
    fn touch_input_is_externally_tagged() {
        let params = SendHubDeviceInputParams {
            device_id: "UDID".into(),
            input: DeviceInput::Touch {
                phase: TouchPhase::Begin,
                x: 0.5,
                y: 0.25,
            },
        };
        let value = serde_json::to_value(&params).unwrap();
        assert_eq!(
            value,
            json!({"deviceId": "UDID", "input": {"touch": {"phase": "begin", "x": 0.5, "y": 0.25}}})
        );
        assert_eq!(
            serde_json::from_value::<SendHubDeviceInputParams>(value).unwrap(),
            params
        );
    }

    #[test]
    fn hub_screen_config_and_device_list_parse() {
        let screen: DeviceScreenConfig = serde_json::from_value(json!({
            "width": 588, "height": 1280, "orientation": "landscape_left", "screenId": 1,
            "supportsPhysicalOrientation": false, "supportsHingeAngle": false
        }))
        .unwrap();
        assert_eq!(
            (screen.width, screen.height, screen.orientation),
            (588, 1280, DeviceOrientation::LandscapeLeft)
        );
        let device: HubDevice = serde_json::from_value(json!({
            "id": "9BDC", "name": "iPhone 17", "version": "iOS 27.0",
            "platform": "ios", "booted": false, "physical": false
        }))
        .unwrap();
        assert_eq!(device.platform, DevicePlatform::Ios);
        assert_eq!(device.host_id, "");
    }

    #[test]
    fn hardware_inputs_and_stream_status_have_stable_wire_shapes() {
        let cases = [
            (
                DeviceInput::Button {
                    button: DeviceButton::AppSwitcher,
                },
                json!({"button": {"button": "app_switcher"}}),
            ),
            (
                DeviceInput::Key {
                    phase: KeyPhase::Down,
                    usage: 0x04,
                },
                json!({"key": {"phase": "down", "usage": 4}}),
            ),
            (
                DeviceInput::Orientation {
                    orientation: DeviceOrientation::LandscapeRight,
                },
                json!({"orientation": {"orientation": "landscape_right"}}),
            ),
        ];
        for (input, wire) in cases {
            assert_eq!(serde_json::to_value(input).unwrap(), wire);
            assert_eq!(serde_json::from_value::<DeviceInput>(wire).unwrap(), input);
        }
        let status = DeviceStreamStatus {
            video: StreamLinkStatus::Live,
            input: StreamLinkStatus::Failed {
                detail: "closed".into(),
            },
            subscribers: 2,
        };
        let wire =
            json!({"video": "live", "input": {"failed": {"detail": "closed"}}, "subscribers": 2});
        assert_eq!(serde_json::to_value(&status).unwrap(), wire);
        assert_eq!(
            serde_json::from_value::<DeviceStreamStatus>(wire).unwrap(),
            status
        );
        let close: CloseHubDeviceParams =
            serde_json::from_value(json!({"chatId": "c", "deviceId": "d", "shutdown": true}))
                .unwrap();
        assert!(close.shutdown && close.host_id.is_none());
        assert!(
            !serde_json::from_value::<CloseHubDeviceParams>(json!({"chatId": "c"}))
                .unwrap()
                .shutdown
        );
    }

    #[test]
    fn open_params_boot_by_default_and_state_tolerates_missing_fields() {
        let open: OpenHubDeviceParams =
            serde_json::from_value(json!({"chatId": "c", "deviceId": "d", "platform": "ios"}))
                .unwrap();
        assert!(open.boot);
        let state: DeviceHubState = serde_json::from_value(json!({"revision": 3})).unwrap();
        assert_eq!(state.host_status, DeviceHostStatus::Disabled);
        assert!(!state.settings.enabled);
    }

    #[test]
    fn host_tool_versions_have_a_stable_wire_shape() {
        let host = DeviceHubHost {
            id: "local".into(),
            tools: Some(DeviceToolVersions {
                hub: DeviceToolVersion {
                    required: "0.12.0".into(),
                    installed: vec!["0.11.0".into(), "0.12.0".into()],
                    running: Some("0.12.0".into()),
                },
            }),
            ..Default::default()
        };
        let wire = json!({
            "id": "local", "kind": "", "label": "", "platforms": [], "hubInstalled": false,
            "tools": {"hub": {"required": "0.12.0", "installed": ["0.11.0", "0.12.0"], "running": "0.12.0"}}
        });
        assert_eq!(serde_json::to_value(&host).unwrap(), wire);
        assert_eq!(serde_json::from_value::<DeviceHubHost>(wire).unwrap(), host);
        let bare: DeviceHubHost = serde_json::from_value(json!({"id": "local"})).unwrap();
        assert!(bare.tools.is_none());
    }
}
