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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub opened_by: Option<DeviceOpener>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DeviceOpener {
    User,
    Agent,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub screen_id: Option<u32>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "hinge_degrees"
    )]
    pub hinge_angle: Option<u8>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "known_or_none"
    )]
    pub hinge_pose: Option<HingePose>,
}

fn known_or_none<'de, D: serde::Deserializer<'de>, T: serde::de::DeserializeOwned>(
    deserializer: D,
) -> Result<Option<T>, D::Error> {
    Ok(Option::<serde_json::Value>::deserialize(deserializer)?
        .and_then(|value| serde_json::from_value(value).ok()))
}

fn hinge_degrees<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<u8>, D::Error> {
    Ok(
        Option::<f64>::deserialize(deserializer)?
            .map(|angle| angle.clamp(0.0, 180.0).round() as u8),
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum HingePose {
    Closed,
    Book,
    Open,
    Laptop,
    Tent,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DeviceFacing {
    FaceUp,
    FaceDown,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum HingeCommand {
    Pose(HingePose),
    Angle(f64),
    Physical(DeviceFacing),
    Orientation(DeviceOrientation),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetHubDeviceHingeParams {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub host_id: Option<String>,
    #[serde(default)]
    pub device_id: String,
    #[serde(flatten)]
    pub command: HingeCommand,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FoldPosture {
    Closed,
    HalfOpened,
    Opened,
    Flipped,
    Tent,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceFoldState {
    #[serde(default)]
    pub supported: bool,
    #[serde(default, deserialize_with = "known_or_none")]
    pub posture: Option<FoldPosture>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HubDeviceFoldParams {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub host_id: Option<String>,
    #[serde(default)]
    pub device_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub posture: Option<FoldPosture>,
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

impl StreamLinkStatus {
    pub fn is_idle(&self) -> bool {
        *self == StreamLinkStatus::Idle
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceStreamStatus {
    #[serde(default)]
    pub video: StreamLinkStatus,
    #[serde(default, skip_serializing_if = "StreamLinkStatus::is_idle")]
    pub avcc: StreamLinkStatus,
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
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub device_hosts: Vec<SshDeviceHostConfig>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SshDeviceHostConfig {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub target: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub identity_file: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub port: Option<u16>,
}

fn plain_text(value: &str) -> bool {
    !value.is_empty() && !value.chars().any(|c| c.is_whitespace() || c.is_control())
}

impl SshDeviceHostConfig {
    pub fn validate(&self) -> Result<(), String> {
        let id_ok = self.id.len() <= 128
            && self.id.starts_with(|c: char| c.is_ascii_alphanumeric())
            && self
                .id
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-');
        if !id_ok {
            return Err("A device host id uses letters, digits, '-' and '_' only.".into());
        }
        if self.id == LOCAL_DEVICE_HOST_ID {
            return Err("The local host id is reserved.".into());
        }
        if self.label.trim().is_empty() || self.label.chars().any(char::is_control) {
            return Err("A device host needs a name.".into());
        }
        if !plain_text(&self.target) || self.target.starts_with('-') {
            return Err(
                "The SSH target must be user@host or an SSH alias, without spaces or a leading '-'."
                    .into(),
            );
        }
        if let Some(file) = &self.identity_file
            && (!file.starts_with('/') || file.chars().any(char::is_control))
        {
            return Err("The identity file must be an absolute path.".into());
        }
        if self.port == Some(0) {
            return Err("The SSH port must be between 1 and 65535.".into());
        }
        Ok(())
    }
}

pub fn validate_device_hosts(hosts: &[SshDeviceHostConfig]) -> Result<(), String> {
    for (index, host) in hosts.iter().enumerate() {
        host.validate()?;
        if hosts[..index].iter().any(|other| other.id == host.id) {
            return Err("Device host ids must be unique.".into());
        }
    }
    Ok(())
}

pub fn device_key(host_id: &str, device_id: &str) -> String {
    if host_id == LOCAL_DEVICE_HOST_ID {
        device_id.to_string()
    } else {
        format!("{host_id}/{device_id}")
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceHostState {
    #[serde(default)]
    pub status: DeviceHostStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceHostCheck {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub ok: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceHostTest {
    #[serde(default)]
    pub local: bool,
    #[serde(default)]
    pub checks: Vec<DeviceHostCheck>,
    #[serde(default)]
    pub platforms: Vec<DevicePlatformAvailability>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tools: Option<DeviceToolVersions>,
}

impl DeviceHostTest {
    pub fn passed(&self) -> bool {
        self.local || self.checks.iter().all(|check| check.ok)
    }
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
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub host_statuses: BTreeMap<String, DeviceHostState>,
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
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub avcc: String,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DeviceVideoCodec {
    #[default]
    Mjpeg,
    H264,
}

pub const AVCC_DESCRIPTION: u8 = 1;
pub const AVCC_KEYFRAME: u8 = 2;
pub const AVCC_DELTA: u8 = 3;
pub const AVCC_SEED: u8 = 4;
pub const AVCC_MAX_ENVELOPE: usize = 16 << 20;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AvccEnvelope {
    pub tag: u8,
    pub payload: Vec<u8>,
}

impl AvccEnvelope {
    pub fn encode_into(&self, out: &mut Vec<u8>) {
        out.extend_from_slice(&(self.payload.len() as u32 + 1).to_be_bytes());
        out.push(self.tag);
        out.extend_from_slice(&self.payload);
    }
}

#[derive(Debug, Default)]
pub struct AvccDemuxer {
    buffer: Vec<u8>,
}

impl AvccDemuxer {
    pub fn push(&mut self, bytes: &[u8]) -> Option<Vec<AvccEnvelope>> {
        self.buffer.extend_from_slice(bytes);
        let mut envelopes = Vec::new();
        let mut offset = 0;
        while let Some(head) = self.buffer.get(offset..offset + 4) {
            let length = u32::from_be_bytes([head[0], head[1], head[2], head[3]]) as usize;
            if length > AVCC_MAX_ENVELOPE {
                self.buffer.clear();
                return None;
            }
            let Some(body) = self.buffer.get(offset + 4..offset + 4 + length) else {
                break;
            };
            if let Some((&tag, payload)) = body.split_first()
                && (AVCC_DESCRIPTION..=AVCC_SEED).contains(&tag)
            {
                envelopes.push(AvccEnvelope {
                    tag,
                    payload: payload.to_vec(),
                });
            }
            offset += 4 + length;
        }
        self.buffer.drain(..offset);
        Some(envelopes)
    }
}

const SEMU_MAGIC: [u8; 4] = *b"SEMU";

pub fn semu_payload(message: &[u8]) -> Option<(Option<bool>, &[u8])> {
    if !message.starts_with(&SEMU_MAGIC) {
        return Some((None, message));
    }
    let key = message.get(5).map(|flags| flags & 1 == 1);
    match message.get(4) {
        Some(1) if message.len() > 16 => Some((key, &message[16..])),
        Some(2) if message.len() > 24 => Some((key, &message[24..])),
        _ => None,
    }
}

pub fn annexb_nals(data: &[u8]) -> Vec<&[u8]> {
    let mut starts = Vec::new();
    let mut i = 0;
    while i + 3 <= data.len() {
        if data[i..i + 3] == [0, 0, 1] {
            let code = if i > 0 && data[i - 1] == 0 { i - 1 } else { i };
            starts.push((code, i + 3));
            i += 3;
        } else {
            i += 1;
        }
    }
    starts
        .iter()
        .enumerate()
        .map(|(n, &(_, start))| {
            let end = starts.get(n + 1).map_or(data.len(), |&(code, _)| code);
            let mut nal = &data[start..end];
            while let [rest @ .., 0] = nal {
                nal = rest;
            }
            nal
        })
        .filter(|nal| !nal.is_empty())
        .collect()
}

pub fn avcc_record(sps: &[u8], pps: &[u8]) -> Option<Vec<u8>> {
    let sps_len = u16::try_from(sps.len()).ok().filter(|_| sps.len() >= 4)?;
    let pps_len = u16::try_from(pps.len()).ok().filter(|_| !pps.is_empty())?;
    let mut record = vec![1, sps[1], sps[2], sps[3], 0xFF, 0xE1];
    record.extend(sps_len.to_be_bytes());
    record.extend(sps);
    record.push(1);
    record.extend(pps_len.to_be_bytes());
    record.extend(pps);
    Some(record)
}

pub fn annexb_to_avcc(access_unit: &[u8], key: Option<bool>) -> Vec<AvccEnvelope> {
    let nals = annexb_nals(access_unit);
    let of_type = |kind: u8| nals.iter().find(|nal| nal[0] & 0x1F == kind);
    let mut envelopes = Vec::new();
    if let (Some(sps), Some(pps)) = (of_type(7), of_type(8))
        && let Some(record) = avcc_record(sps, pps)
    {
        envelopes.push(AvccEnvelope {
            tag: AVCC_DESCRIPTION,
            payload: record,
        });
    }
    let mut sample = Vec::with_capacity(access_unit.len());
    for nal in nals.iter().filter(|nal| !matches!(nal[0] & 0x1F, 7..=9)) {
        sample.extend((nal.len() as u32).to_be_bytes());
        sample.extend(*nal);
    }
    if !sample.is_empty() {
        let idr = of_type(5).is_some();
        envelopes.push(AvccEnvelope {
            tag: if key == Some(true) || idr {
                AVCC_KEYFRAME
            } else {
                AVCC_DELTA
            },
            payload: sample,
        });
    }
    envelopes
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
    Back,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum KeyPhase {
    Down,
    Up,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DeviceInput {
    Touch {
        phase: TouchPhase,
        x: f32,
        y: f32,
    },
    Button {
        button: DeviceButton,
    },
    Key {
        phase: KeyPhase,
        usage: u16,
    },
    Orientation {
        orientation: DeviceOrientation,
    },
    Keycode {
        keycode: u16,
        #[serde(default, rename = "metaState", skip_serializing_if = "is_zero")]
        meta_state: u32,
    },
    Text {
        text: String,
    },
}

fn is_zero(value: &u32) -> bool {
    *value == 0
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device_hosts: Option<Vec<SshDeviceHostConfig>>,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub host_id: Option<String>,
    #[serde(default)]
    pub device_id: String,
    #[serde(default)]
    pub codec: DeviceVideoCodec,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SendHubDeviceInputParams {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub host_id: Option<String>,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub network_enabled: Option<bool>,
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
            host_id: None,
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
    fn iphone_duo_screen_configs_carry_the_screen_and_hinge() {
        // 0x82 payloads captured from the iPhone Duo simulator (iOS 27.1, hub 0.12.0).
        let captured = [
            (
                r#"{"width":1398,"height":2034,"orientation":"portrait","screenId":1,"supportsPhysicalOrientation":true,"supportsHingeAngle":true,"hingePose":null,"tableModeAvailable":false}"#,
                (1398, 2034, DeviceOrientation::Portrait, Some(1), None, None),
            ),
            (
                r#"{"width":1398,"height":2034,"orientation":"portrait","screenId":1,"supportsPhysicalOrientation":true,"supportsHingeAngle":true,"hingeAngle":0,"hingePose":"closed","tableModeAvailable":false,"physicalOrientation":"portrait","tableMode":false}"#,
                (
                    1398,
                    2034,
                    DeviceOrientation::Portrait,
                    Some(1),
                    Some(0),
                    Some(HingePose::Closed),
                ),
            ),
            (
                r#"{"width":2007,"height":2853,"orientation":"landscape_left","screenId":3,"supportsPhysicalOrientation":true,"supportsHingeAngle":true,"hingeAngle":180,"hingePose":"open","tableModeAvailable":true,"physicalOrientation":"portrait","tableMode":false}"#,
                (
                    2007,
                    2853,
                    DeviceOrientation::LandscapeLeft,
                    Some(3),
                    Some(180),
                    Some(HingePose::Open),
                ),
            ),
            (
                r#"{"width":2006,"height":2852,"orientation":"landscape_left","screenId":3,"supportsPhysicalOrientation":true,"supportsHingeAngle":true,"hingeAngle":90,"hingePose":"book","tableModeAvailable":true,"physicalOrientation":"portrait","tableMode":false}"#,
                (
                    2006,
                    2852,
                    DeviceOrientation::LandscapeLeft,
                    Some(3),
                    Some(90),
                    Some(HingePose::Book),
                ),
            ),
            (
                r#"{"width":2006,"height":2852,"orientation":"portrait","screenId":3,"supportsPhysicalOrientation":true,"supportsHingeAngle":true,"hingeAngle":90,"hingePose":"laptop","tableModeAvailable":true,"physicalOrientation":"landscape-left","tableMode":false}"#,
                (
                    2006,
                    2852,
                    DeviceOrientation::Portrait,
                    Some(3),
                    Some(90),
                    Some(HingePose::Laptop),
                ),
            ),
            (
                r#"{"width":1398,"height":2034,"orientation":"portrait","screenId":1,"supportsPhysicalOrientation":true,"supportsHingeAngle":true,"hingeAngle":80,"hingePose":"tent","tableModeAvailable":true,"physicalOrientation":"facedown","tableMode":true}"#,
                (
                    1398,
                    2034,
                    DeviceOrientation::Portrait,
                    Some(1),
                    Some(80),
                    Some(HingePose::Tent),
                ),
            ),
            (
                r#"{"width":2007,"height":2853,"orientation":"landscape_left","screenId":3,"supportsPhysicalOrientation":true,"supportsHingeAngle":true,"hingeAngle":45,"hingePose":null,"tableModeAvailable":true,"physicalOrientation":"portrait","tableMode":false}"#,
                (
                    2007,
                    2853,
                    DeviceOrientation::LandscapeLeft,
                    Some(3),
                    Some(45),
                    None,
                ),
            ),
        ];
        for (wire, expected) in captured {
            let screen: DeviceScreenConfig = serde_json::from_str(wire).unwrap();
            assert_eq!(
                (
                    screen.width,
                    screen.height,
                    screen.orientation,
                    screen.screen_id,
                    screen.hinge_angle,
                    screen.hinge_pose
                ),
                expected,
                "{wire}"
            );
            assert_eq!(screen.supports_hinge_angle, Some(true));
        }
        let fractional: DeviceScreenConfig =
            serde_json::from_value(json!({"width": 1, "height": 2, "hingeAngle": 92.6})).unwrap();
        assert_eq!(fractional.hinge_angle, Some(93));
        let unknown: DeviceScreenConfig = serde_json::from_value(json!({
            "width": 1398, "height": 2034, "screenId": 1, "hingeAngle": 120, "hingePose": "flat"
        }))
        .unwrap();
        assert_eq!(
            (
                unknown.width,
                unknown.screen_id,
                unknown.hinge_angle,
                unknown.hinge_pose
            ),
            (1398, Some(1), Some(120), None),
            "a pose this build does not know keeps the rest of the config"
        );
        let iphone: DeviceScreenConfig = serde_json::from_value(json!({
            "width": 588, "height": 1280, "orientation": "portrait", "screenId": 1,
            "supportsPhysicalOrientation": false, "supportsHingeAngle": false
        }))
        .unwrap();
        assert_eq!(
            serde_json::to_value(&iphone).unwrap(),
            json!({
                "width": 588, "height": 1280, "orientation": "portrait", "screenId": 1,
                "supportsPhysicalOrientation": false, "supportsHingeAngle": false
            })
        );
    }

    #[test]
    fn duo_hinge_commands_wire_shapes() {
        let hinge = |command| SetHubDeviceHingeParams {
            host_id: None,
            device_id: "DUO".into(),
            command,
        };
        let cases = [
            (
                HingeCommand::Pose(HingePose::Book),
                r#"{"deviceId":"DUO","pose":"book"}"#,
            ),
            (
                HingeCommand::Angle(45.5),
                r#"{"deviceId":"DUO","angle":45.5}"#,
            ),
            (
                HingeCommand::Physical(DeviceFacing::FaceDown),
                r#"{"deviceId":"DUO","physical":"facedown"}"#,
            ),
            (
                HingeCommand::Physical(DeviceFacing::FaceUp),
                r#"{"deviceId":"DUO","physical":"faceup"}"#,
            ),
            (
                HingeCommand::Orientation(DeviceOrientation::LandscapeLeft),
                r#"{"deviceId":"DUO","orientation":"landscape_left"}"#,
            ),
        ];
        for (command, wire) in cases {
            assert_eq!(serde_json::to_string(&hinge(command)).unwrap(), wire);
            assert_eq!(
                serde_json::from_str::<SetHubDeviceHingeParams>(wire).unwrap(),
                hinge(command)
            );
        }
        let remote: SetHubDeviceHingeParams =
            serde_json::from_value(json!({"hostId": "mini", "deviceId": "DUO", "angle": 45}))
                .unwrap();
        assert_eq!(
            (remote.host_id.as_deref(), remote.command),
            (Some("mini"), HingeCommand::Angle(45.0))
        );
        for refused in [
            json!({"deviceId": "DUO"}),
            json!({"deviceId": "DUO", "angle": "45"}),
            json!({"deviceId": "DUO", "physical": "sideways"}),
            json!({"deviceId": "DUO", "table": true}),
        ] {
            assert!(
                serde_json::from_value::<SetHubDeviceHingeParams>(refused.clone()).is_err(),
                "{refused}"
            );
        }
    }

    #[test]
    fn hinge_fold_and_opener_wire_shapes() {
        assert_eq!(
            serde_json::to_value(SetHubDeviceHingeParams {
                host_id: None,
                device_id: "DUO".into(),
                command: HingeCommand::Pose(HingePose::Laptop),
            })
            .unwrap(),
            json!({"deviceId": "DUO", "pose": "laptop"})
        );
        assert_eq!(
            serde_json::to_value(HubDeviceFoldParams {
                device_id: "emulator-5554".into(),
                posture: Some(FoldPosture::Closed),
                ..Default::default()
            })
            .unwrap(),
            json!({"deviceId": "emulator-5554", "posture": "closed"})
        );
        let fold: DeviceFoldState = serde_json::from_value(
            json!({"supported": true, "posture": "half_opened", "hingeAngle": 92.5}),
        )
        .unwrap();
        assert_eq!(fold.posture, Some(FoldPosture::HalfOpened));
        let unsupported: DeviceFoldState = serde_json::from_value(
            json!({"supported": false, "posture": null, "hingeAngle": null}),
        )
        .unwrap();
        assert_eq!(unsupported, DeviceFoldState::default());
        let unknown: DeviceFoldState =
            serde_json::from_value(json!({"supported": true, "posture": "dual_display"})).unwrap();
        assert_eq!(
            unknown,
            DeviceFoldState {
                supported: true,
                posture: None
            }
        );
        let session: DeviceSession =
            serde_json::from_value(json!({"chatId": "c", "openedBy": "agent"})).unwrap();
        assert_eq!(session.opened_by, Some(DeviceOpener::Agent));
        assert!(
            serde_json::to_value(DeviceSession::default())
                .unwrap()
                .get("openedBy")
                .is_none()
        );
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
            (
                DeviceInput::Button {
                    button: DeviceButton::Back,
                },
                json!({"button": {"button": "back"}}),
            ),
            (
                DeviceInput::Keycode {
                    keycode: 66,
                    meta_state: 0,
                },
                json!({"keycode": {"keycode": 66}}),
            ),
            (
                DeviceInput::Keycode {
                    keycode: 61,
                    meta_state: 0x41,
                },
                json!({"keycode": {"keycode": 61, "metaState": 65}}),
            ),
            (
                DeviceInput::Text { text: "é".into() },
                json!({"text": {"text": "é"}}),
            ),
        ];
        for (input, wire) in cases {
            assert_eq!(serde_json::to_value(&input).unwrap(), wire);
            assert_eq!(serde_json::from_value::<DeviceInput>(wire).unwrap(), input);
        }
        let status = DeviceStreamStatus {
            video: StreamLinkStatus::Live,
            input: StreamLinkStatus::Failed {
                detail: "closed".into(),
            },
            subscribers: 2,
            ..Default::default()
        };
        let wire =
            json!({"video": "live", "input": {"failed": {"detail": "closed"}}, "subscribers": 2});
        assert_eq!(serde_json::to_value(&status).unwrap(), wire);
        assert_eq!(
            serde_json::from_value::<DeviceStreamStatus>(wire).unwrap(),
            status
        );
        let both = DeviceStreamStatus {
            video: StreamLinkStatus::Idle,
            avcc: StreamLinkStatus::Live,
            ..status
        };
        let wire = json!({"video": "idle", "avcc": "live", "input": {"failed": {"detail": "closed"}}, "subscribers": 2});
        assert_eq!(serde_json::to_value(&both).unwrap(), wire);
        assert_eq!(
            serde_json::from_value::<DeviceStreamStatus>(wire).unwrap(),
            both
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

    #[test]
    fn video_codec_and_avcc_items_default_to_the_mjpeg_wire() {
        let old: WatchHubDeviceFramesParams =
            serde_json::from_value(json!({"deviceId": "SIM"})).unwrap();
        assert_eq!(old.codec, DeviceVideoCodec::Mjpeg);
        let h264: WatchHubDeviceFramesParams =
            serde_json::from_value(json!({"deviceId": "SIM", "codec": "h264"})).unwrap();
        assert_eq!(h264.codec, DeviceVideoCodec::H264);
        let jpeg = DeviceFrame {
            seq: 1,
            jpeg: "AA==".into(),
            ..Default::default()
        };
        assert_eq!(
            serde_json::to_value(&jpeg).unwrap(),
            json!({"seq": 1, "jpeg": "AA=="})
        );
        let avcc: DeviceFrame =
            serde_json::from_value(json!({"seq": 2, "avcc": "AAAAAgEB"})).unwrap();
        assert_eq!(avcc.avcc, "AAAAAgEB");
        assert!(avcc.jpeg.is_empty());
    }

    fn envelope(tag: u8, payload: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        AvccEnvelope {
            tag,
            payload: payload.to_vec(),
        }
        .encode_into(&mut out);
        out
    }

    #[test]
    fn avcc_demuxer_yields_whole_envelopes_at_every_chunk_split_and_skips_unknown_tags() {
        let stream = [
            envelope(AVCC_SEED, b"jpeg"),
            envelope(AVCC_DESCRIPTION, &[1, 0x64, 0, 0x20, 0xff]),
            envelope(9, b"unknown"),
            vec![0, 0, 0, 0],
            envelope(AVCC_KEYFRAME, &[0, 0, 0, 1, 0x65]),
            envelope(AVCC_DELTA, &[]),
            envelope(AVCC_DELTA, &[0, 0, 0, 2, 0x41, 7]),
        ]
        .concat();
        let expected = vec![
            (AVCC_SEED, b"jpeg".to_vec()),
            (AVCC_DESCRIPTION, vec![1, 0x64, 0, 0x20, 0xff]),
            (AVCC_KEYFRAME, vec![0, 0, 0, 1, 0x65]),
            (AVCC_DELTA, vec![]),
            (AVCC_DELTA, vec![0, 0, 0, 2, 0x41, 7]),
        ];
        for first in 0..=stream.len() {
            for second in first..=stream.len() {
                let mut demuxer = AvccDemuxer::default();
                let got: Vec<(u8, Vec<u8>)> =
                    [&stream[..first], &stream[first..second], &stream[second..]]
                        .into_iter()
                        .flat_map(|chunk| demuxer.push(chunk).unwrap())
                        .map(|envelope| (envelope.tag, envelope.payload))
                        .collect();
                assert_eq!(got, expected, "split at {first} and {second}");
            }
        }
        let mut demuxer = AvccDemuxer::default();
        assert_eq!(demuxer.push(&stream[..stream.len() - 1]).unwrap().len(), 4);
        assert!(demuxer.push(&[]).unwrap().is_empty());
    }

    const ANDROID_CAPTURE: &[u8] = include_bytes!("../tests/fixtures/device-android-576x1280.semu");

    enum Message<'a> {
        Binary(&'a [u8]),
        Text(&'a str),
    }

    fn android_capture() -> Vec<Message<'static>> {
        let mut messages = Vec::new();
        let mut rest = ANDROID_CAPTURE;
        while let [kind, a, b, c, d, tail @ ..] = rest {
            let length = u32::from_be_bytes([*a, *b, *c, *d]) as usize;
            let (payload, next) = tail.split_at(length);
            messages.push(match kind {
                0 => Message::Binary(payload),
                _ => Message::Text(std::str::from_utf8(payload).unwrap()),
            });
            rest = next;
        }
        messages
    }

    fn binaries() -> Vec<&'static [u8]> {
        android_capture()
            .into_iter()
            .filter_map(|message| match message {
                Message::Binary(bytes) => Some(bytes),
                Message::Text(_) => None,
            })
            .collect()
    }

    fn nal_types(sample: &[u8]) -> Vec<u8> {
        let mut types = Vec::new();
        let mut rest = sample;
        while let [a, b, c, d, tail @ ..] = rest {
            let length = u32::from_be_bytes([*a, *b, *c, *d]) as usize;
            types.push(tail[0] & 0x1F);
            rest = &tail[length..];
        }
        assert!(
            rest.is_empty(),
            "the length prefixes cover the sample exactly"
        );
        types
    }

    #[test]
    fn semu_v1_and_v2_headers_strip_to_the_annexb_payload() {
        let captured = binaries();
        assert_eq!(captured.len(), 6);
        for (n, message) in captured.iter().enumerate() {
            assert_eq!(
                &message[..5],
                b"SEMU\x01",
                "the hub-mounted serve-emu writes v1"
            );
            let key = n == 0 || n == 4;
            let (flag, payload) = semu_payload(message).unwrap();
            assert_eq!(flag, Some(key), "message {n}");
            assert_eq!(payload, &message[16..]);
            assert_eq!(&payload[..4], [0, 0, 0, 1]);

            let mut v2 = message[..16].to_vec();
            v2[4] = 2;
            v2.extend(1_791_216_912_063_000u64.to_be_bytes());
            v2.extend(payload);
            assert_eq!(
                semu_payload(&v2),
                Some((Some(key), payload)),
                "message {n} as v2"
            );
        }
        let payload = &captured[1][16..];
        assert_eq!(semu_payload(payload), Some((None, payload)), "raw Annex-B");
        let mut unknown = captured[1].to_vec();
        unknown[4] = 3;
        assert_eq!(
            semu_payload(&unknown),
            None,
            "an unknown version is an error"
        );
        assert_eq!(semu_payload(&captured[1][..16]), None, "a bare v1 header");
        assert_eq!(semu_payload(b"SEMU"), None);
    }

    #[test]
    fn annexb_access_units_become_avcc_envelopes_with_a_description_from_sps_pps() {
        let mut tags = Vec::new();
        let mut descriptions = Vec::new();
        for (n, message) in android_capture().into_iter().enumerate() {
            let bytes = match message {
                Message::Text(text) => {
                    assert!(text.contains(r#""type":"video-session""#), "{text}");
                    continue;
                }
                Message::Binary(bytes) => bytes,
            };
            let (key, payload) = semu_payload(bytes).unwrap();
            let nals = annexb_nals(payload);
            for envelope in annexb_to_avcc(payload, key) {
                tags.push(envelope.tag);
                match envelope.tag {
                    AVCC_DESCRIPTION => {
                        let sps = nals.iter().find(|nal| nal[0] & 0x1F == 7).unwrap();
                        let pps = nals.iter().find(|nal| nal[0] & 0x1F == 8).unwrap();
                        assert_eq!(envelope.payload[..6], [1, 0x42, 0xC0, 0x20, 0xFF, 0xE1]);
                        assert_eq!(Some(envelope.payload.clone()), avcc_record(sps, pps));
                        descriptions.push(envelope.payload);
                    }
                    AVCC_KEYFRAME => {
                        let types = nal_types(&envelope.payload);
                        assert_eq!(types[0], 6, "SEI stays, parameter sets go");
                        assert!(types[1..].iter().all(|kind| *kind == 5), "{types:?}");
                    }
                    _ => {
                        let types = nal_types(&envelope.payload);
                        assert!(
                            types.iter().all(|kind| *kind == 1),
                            "message {n}: {types:?}"
                        );
                    }
                }
            }
        }
        assert_eq!(
            tags,
            [
                AVCC_DESCRIPTION,
                AVCC_KEYFRAME,
                AVCC_DELTA,
                AVCC_DELTA,
                AVCC_DELTA,
                AVCC_DESCRIPTION,
                AVCC_KEYFRAME,
                AVCC_DELTA
            ]
        );
        assert_ne!(
            descriptions[0], descriptions[1],
            "the encoder restart after rotation brings a new SPS"
        );
    }

    #[test]
    fn annexb_splits_three_and_four_byte_start_codes_and_drops_access_unit_delimiters() {
        let au = [
            &[0, 0, 0, 1, 0x09, 0xF0][..],
            &[0, 0, 1, 0x67, 0x42, 0xC0, 0x1F, 0xAA, 0, 0],
            &[0, 0, 0, 1, 0x68, 0xCE],
            &[0, 0, 1, 0x65, 0x88, 0x84],
        ]
        .concat();
        assert_eq!(
            annexb_nals(&au),
            [
                &[0x09, 0xF0][..],
                &[0x67, 0x42, 0xC0, 0x1F, 0xAA],
                &[0x68, 0xCE],
                &[0x65, 0x88, 0x84]
            ]
        );
        let envelopes = annexb_to_avcc(&au, None);
        assert_eq!(
            envelopes,
            [
                AvccEnvelope {
                    tag: AVCC_DESCRIPTION,
                    payload: vec![
                        1, 0x42, 0xC0, 0x1F, 0xFF, 0xE1, 0, 5, 0x67, 0x42, 0xC0, 0x1F, 0xAA, 1, 0,
                        2, 0x68, 0xCE
                    ],
                },
                AvccEnvelope {
                    tag: AVCC_KEYFRAME,
                    payload: vec![0, 0, 0, 3, 0x65, 0x88, 0x84],
                },
            ],
            "an IDR slice makes a keyframe even without a SEMU flag"
        );
        assert_eq!(
            annexb_to_avcc(&[0, 0, 1, 0x41, 0x9A], Some(false)),
            [AvccEnvelope {
                tag: AVCC_DELTA,
                payload: vec![0, 0, 0, 2, 0x41, 0x9A],
            }]
        );
        assert!(annexb_to_avcc(&[1, 2, 3], None).is_empty());
        assert_eq!(avcc_record(&[0x67, 0x42], &[0x68]), None);
    }

    #[test]
    fn avcc_demuxer_refuses_an_oversized_envelope() {
        let mut demuxer = AvccDemuxer::default();
        let length = (AVCC_MAX_ENVELOPE as u32 + 1).to_be_bytes();
        assert!(demuxer.push(&length).is_none());
        assert_eq!(
            demuxer.push(&envelope(AVCC_DELTA, b"x")).unwrap(),
            vec![AvccEnvelope {
                tag: AVCC_DELTA,
                payload: b"x".to_vec()
            }]
        );
    }

    fn ssh_host(id: &str, target: &str) -> SshDeviceHostConfig {
        SshDeviceHostConfig {
            id: id.into(),
            label: "Mac mini".into(),
            target: target.into(),
            ..Default::default()
        }
    }

    #[test]
    fn device_hosts_round_trip_with_the_settings_and_stay_off_the_wire_when_empty() {
        let bare = DeviceHubSettings {
            enabled: true,
            ..Default::default()
        };
        assert_eq!(
            serde_json::to_value(&bare).unwrap(),
            json!({"enabled": true, "agentAccessEnabled": false, "onboardingCompleted": false})
        );
        let settings = DeviceHubSettings {
            device_hosts: vec![SshDeviceHostConfig {
                identity_file: Some("/Users/me/.ssh/id_ed25519".into()),
                port: Some(2222),
                ..ssh_host("mini", "me@mini.local")
            }],
            ..bare
        };
        let wire = json!({"enabled": true, "agentAccessEnabled": false, "onboardingCompleted": false,
            "deviceHosts": [{"id": "mini", "label": "Mac mini", "target": "me@mini.local",
                "identityFile": "/Users/me/.ssh/id_ed25519", "port": 2222}]});
        assert_eq!(serde_json::to_value(&settings).unwrap(), wire);
        assert_eq!(
            serde_json::from_value::<DeviceHubSettings>(wire).unwrap(),
            settings
        );
        let old: DeviceHubSettings = serde_json::from_value(json!({"enabled": true})).unwrap();
        assert!(old.device_hosts.is_empty());
        let configure: ConfigureDeviceHubParams =
            serde_json::from_value(json!({"deviceHosts": []})).unwrap();
        assert_eq!(configure.device_hosts, Some(Vec::new()));
        assert_eq!(
            serde_json::to_value(ConfigureDeviceHubParams::default()).unwrap(),
            json!({})
        );
    }

    #[test]
    fn device_host_fields_are_refused_before_they_can_reach_an_ssh_argv() {
        assert_eq!(ssh_host("mini", "me@mini.local").validate(), Ok(()));
        assert_eq!(ssh_host("mac_2-b", "mini").validate(), Ok(()));
        for target in [
            "-oProxyCommand=touch /tmp/pwned",
            "-G",
            "me@mini local",
            "mini\n-oProxyCommand=x",
            "mini\t",
            "",
            " mini",
            "mini\u{0}",
        ] {
            assert!(ssh_host("mini", target).validate().is_err(), "{target:?}");
        }
        for id in ["", "local", "-x", "a b", "a/b", "_a", &"a".repeat(129)] {
            assert!(ssh_host(id, "mini").validate().is_err(), "{id:?}");
        }
        let with = |identity: &str, port: Option<u16>| SshDeviceHostConfig {
            identity_file: Some(identity.into()),
            port,
            ..ssh_host("mini", "mini")
        };
        assert!(
            with("/Users/me/.ssh/id with space", Some(22))
                .validate()
                .is_ok()
        );
        assert!(with("-i/etc/passwd", None).validate().is_err());
        assert!(with("id_ed25519", None).validate().is_err());
        assert!(
            with("/Users/me/id\n-oProxyCommand=x", None)
                .validate()
                .is_err()
        );
        assert!(with("/Users/me/id", Some(0)).validate().is_err());
        assert!(
            serde_json::from_value::<SshDeviceHostConfig>(json!({"id": "a", "port": 70000}))
                .is_err()
        );
        let blank = SshDeviceHostConfig {
            label: "  ".into(),
            ..ssh_host("mini", "mini")
        };
        assert!(blank.validate().is_err());
        assert_eq!(
            validate_device_hosts(&[ssh_host("a", "one"), ssh_host("a", "two")]),
            Err("Device host ids must be unique.".into())
        );
        assert!(validate_device_hosts(&[ssh_host("a", "one"), ssh_host("b", "two")]).is_ok());
    }

    #[test]
    fn device_keys_keep_local_ids_and_scope_remote_ones_by_host() {
        assert_eq!(
            device_key(LOCAL_DEVICE_HOST_ID, "emulator-5554"),
            "emulator-5554"
        );
        assert_eq!(device_key("mini", "emulator-5554"), "mini/emulator-5554");
        assert_ne!(
            device_key("mini", "emulator-5554"),
            device_key("studio", "emulator-5554")
        );
        let params: SendHubDeviceInputParams = serde_json::from_value(
            json!({"hostId": "mini", "deviceId": "SIM", "input": {"button": {"button": "home"}}}),
        )
        .unwrap();
        assert_eq!(params.host_id.as_deref(), Some("mini"));
        let state: DeviceHubState = serde_json::from_value(
            json!({"hostStatuses": {"mini": {"status": "failed", "detail": "offline"}}}),
        )
        .unwrap();
        assert_eq!(state.host_statuses["mini"].status, DeviceHostStatus::Failed);
        assert!(
            serde_json::to_value(DeviceHubState::default())
                .unwrap()
                .get("hostStatuses")
                .is_none()
        );
    }
}
