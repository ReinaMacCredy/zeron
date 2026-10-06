use std::path::{Path, PathBuf};

use zeron_proto::device_hub::{DeviceToolVersion, DeviceToolVersions};

const HUB_PIN: &str = "expo-device-hub@0.12.0";
const HUB_BIN: &str = "expo-device-hub";

pub(crate) fn installed_hub() -> Option<PathBuf> {
    zeron_harness::installed_npm_tool(HUB_PIN, HUB_BIN)
}

pub(crate) fn hub_installed() -> bool {
    installed_hub().is_some()
}

fn pinned(pin: &'static str) -> &'static str {
    pin.rsplit_once('@').map_or("", |(_, v)| v)
}

pub(crate) fn hub_version() -> &'static str {
    pinned(HUB_PIN)
}

pub(crate) fn agent_version() -> &'static str {
    pinned(AGENT_PIN)
}

pub(crate) fn versions(running: bool) -> DeviceToolVersions {
    let required = hub_version().to_string();
    DeviceToolVersions {
        hub: DeviceToolVersion {
            installed: zeron_harness::installed_npm_versions(HUB_PIN),
            running: running.then(|| required.clone()),
            required,
        },
    }
}

pub(crate) async fn ensure_hub() -> Result<PathBuf, String> {
    zeron_harness::ensure_npm_tool(HUB_PIN, HUB_BIN, "device hub")
        .await
        .map_err(|e| e.to_string())
}

pub(crate) fn launch(entry: &Path) -> Result<(PathBuf, Vec<String>), String> {
    zeron_harness::npm_tool_launch(entry).map_err(|e| e.to_string())
}

const AGENT_PIN: &str = "agent-device@0.21.12";
const AGENT_BIN: &str = "agent-device";

pub(crate) async fn ensure_agent() -> Result<PathBuf, String> {
    zeron_harness::ensure_npm_tool(AGENT_PIN, AGENT_BIN, "agent-device")
        .await
        .map_err(|e| e.to_string())
}

pub(crate) fn agent_installed() -> bool {
    zeron_harness::installed_npm_tool(AGENT_PIN, AGENT_BIN).is_some()
}
