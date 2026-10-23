use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub const OBSERVE_ACTIONS: &[&str] = &[
    "state", "tree", "snapshot", "wait", "audit", "frames", "track", "record", "compare",
];

pub const DRIVE_ACTIONS: &[&str] = &[
    "click", "move", "drag", "scroll", "key", "type", "press", "action", "set", "clock",
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
#[serde(rename_all = "camelCase")]
pub struct UiDriverDescriptor {
    pub pid: u32,
    pub socket: PathBuf,
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
