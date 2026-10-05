use std::path::{Path, PathBuf};

const HUB_PIN: &str = "expo-device-hub@0.12.0";
const HUB_BIN: &str = "expo-device-hub";

pub(crate) fn hub_installed() -> bool {
    zeron_harness::installed_npm_tool(HUB_PIN, HUB_BIN).is_some()
}

pub(crate) async fn ensure_hub() -> Result<PathBuf, String> {
    zeron_harness::ensure_npm_tool(HUB_PIN, HUB_BIN, "device hub")
        .await
        .map_err(|e| e.to_string())
}

pub(crate) fn launch(entry: &Path) -> Result<(PathBuf, Vec<String>), String> {
    zeron_harness::npm_tool_launch(entry).map_err(|e| e.to_string())
}
