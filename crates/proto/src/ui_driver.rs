use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

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

pub fn socket_path(pid: u32) -> PathBuf {
    run_dir().join(format!("ui-{pid}.sock"))
}

pub fn descriptor_path(pid: u32) -> PathBuf {
    run_dir().join(format!("ui-{pid}.json"))
}
