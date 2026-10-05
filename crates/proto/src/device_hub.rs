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
    #[serde(default)]
    pub hub_installed: bool,
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
    pub screens: BTreeMap<String, DeviceScreenConfig>,
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

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DeviceInput {
    Touch { phase: TouchPhase, x: f32, y: f32 },
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

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
    fn open_params_boot_by_default_and_state_tolerates_missing_fields() {
        let open: OpenHubDeviceParams =
            serde_json::from_value(json!({"chatId": "c", "deviceId": "d", "platform": "ios"}))
                .unwrap();
        assert!(open.boot);
        let state: DeviceHubState = serde_json::from_value(json!({"revision": 3})).unwrap();
        assert_eq!(state.host_status, DeviceHostStatus::Disabled);
        assert!(!state.settings.enabled);
    }
}
