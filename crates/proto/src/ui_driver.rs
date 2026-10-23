use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub const OBSERVE_ACTIONS: &[&str] = &[
    "state", "tree", "snapshot", "wait", "audit", "frames", "track", "record",
];

pub const DRIVE_ACTIONS: &[&str] = &[
    "click", "move", "drag", "scroll", "key", "type", "press", "action", "set",
];

pub const PROCESS_ACTIONS: &[&str] = &["launch", "quit"];

pub const MAX_WAIT_MS: u64 = 60_000;

pub const STEP_CAP_MS: u64 = 100;

pub const MAX_DRIVE_STEPS: u64 = MAX_WAIT_MS / STEP_CAP_MS;

pub const DRAG_STEPS: u32 = 12;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum UiDriverBuild {
    Debug,
    Release,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum UiDriverEndpoint {
    Unix {
        socket: PathBuf,
    },
    Loopback {
        #[serde(rename = "driverPort")]
        port: u16,
        token: String,
    },
}

impl std::fmt::Display for UiDriverEndpoint {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unix { socket } => write!(f, "{}", socket.display()),
            Self::Loopback { port, .. } => write!(f, "127.0.0.1:{port}"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UiDriverDescriptor {
    pub pid: u32,
    #[serde(flatten)]
    pub endpoint: UiDriverEndpoint,
    pub label: String,
    pub bundle_path: Option<PathBuf>,
    pub data_dir: PathBuf,
    pub ipc_port: u16,
    pub build: UiDriverBuild,
    pub input: bool,
    pub version: String,
    pub started_at: DateTime<Utc>,
}

pub fn run_dir() -> PathBuf {
    let home = ["HOME", "USERPROFILE"]
        .into_iter()
        .find_map(|name| std::env::var_os(name).filter(|value| !value.is_empty()))
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/"));
    home.join(".zeron").join("run")
}

pub fn socket_path(dir: &Path, pid: u32) -> PathBuf {
    dir.join(format!("ui-{pid}.sock"))
}

pub fn descriptor_path(dir: &Path, pid: u32) -> PathBuf {
    dir.join(format!("ui-{pid}.json"))
}

pub fn descriptor_pid(path: &Path) -> Option<u32> {
    path.file_name()?
        .to_str()?
        .strip_prefix("ui-")?
        .strip_suffix(".json")?
        .parse()
        .ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_unix_descriptor_keeps_its_socket_key_and_a_loopback_one_carries_port_and_token() {
        let unix = UiDriverDescriptor {
            pid: 7,
            endpoint: UiDriverEndpoint::Unix {
                socket: "/run/ui-7.sock".into(),
            },
            label: "Zeron Dev 7".into(),
            bundle_path: None,
            data_dir: "/data".into(),
            ipc_port: 49787,
            build: UiDriverBuild::Debug,
            input: true,
            version: "0.1.0".into(),
            started_at: DateTime::from_timestamp(0, 0).unwrap(),
        };
        let json = serde_json::to_value(&unix).unwrap();
        assert_eq!(json["socket"], "/run/ui-7.sock");
        assert!(json.get("driverPort").is_none() && json.get("token").is_none());
        assert_eq!(
            serde_json::from_value::<UiDriverDescriptor>(json).unwrap(),
            unix
        );

        let loopback = UiDriverDescriptor {
            endpoint: UiDriverEndpoint::Loopback {
                port: 51234,
                token: "abc".into(),
            },
            ..unix
        };
        let json = serde_json::to_value(&loopback).unwrap();
        assert_eq!(
            (&json["driverPort"], &json["token"]),
            (&51234.into(), &"abc".into())
        );
        assert!(json.get("socket").is_none());
        assert_eq!(
            serde_json::from_value::<UiDriverDescriptor>(json).unwrap(),
            loopback
        );
        assert_eq!(loopback.endpoint.to_string(), "127.0.0.1:51234");
    }
}
