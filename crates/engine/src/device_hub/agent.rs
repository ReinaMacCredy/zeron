use std::path::{Path, PathBuf};
use std::sync::PoisonError;
use std::time::Duration;

use base64::Engine as _;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use zeron_proto::device_hub::{
    CloseHubDeviceParams, DevicePlatform, DeviceToolParams, HubDevice, LOCAL_DEVICE_HOST_ID,
    OpenHubDeviceParams,
};

use super::hub_process::{self, HubProcess};
use super::{DeviceHub, IOS_ONLY, UNKNOWN_DEVICE, hub_client, toolchain};

pub const AGENT_ACCESS_OFF: &str = "Agent device access is turned off for this environment.";
pub const TOOL_SUPPORT_OFF: &str = "Device support is off. Ask the user to enable it in the Device panel before installing or starting device tools.";
pub const BOTH_PLATFORMS: &str =
    "Both iOS and Android devices are available; pass platform or deviceId.";
const NO_OPEN_DEVICE: &str = "No device is open in this thread. Call device_open first.";
pub(crate) const LAUNCHER_REFUSAL: &str =
    "Call device_open first and include its --config and --session flags.";
pub const DEVICE_TOOL_INSTRUCTIONS: &str = "## Zeron devices

The `zeron` MCP server also exposes `device_*` tools for iOS Simulators and Android Emulators on this environment. For mobile verification, call `device_list`, then `device_open` so the user can watch the device in their Device panel; its result explains how to drive the device. Driving happens through the `agent-device` CLI, using the exact launcher path returned by `device_open`. Keep the host config and session flags returned by `device_open` on every command so concurrent devices stay independent: prefer `agent-device snapshot -i` refs over coordinates, and use `device_screenshot` when you need to see the screen. Prefer these tools and `agent-device` for opening and driving devices. Platform tools such as `xcrun simctl` and `adb` remain available for anything they do not cover, such as builds, logs, or port forwarding. If `device_list` reports a platform as unavailable, say so.";
const SHIM: &str = "agent-device";
const LAUNCHER: &str = "agent-device-launcher.mjs";
const DAEMON_FILE: &str = "daemon.json";
const DAEMON_READY_TIMEOUT: Duration = Duration::from_secs(30);
const SCREENSHOT_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Debug, Clone)]
pub(crate) struct AgentTool {
    pub node: PathBuf,
    pub entry: PathBuf,
}

impl AgentTool {
    fn daemon_entry(&self) -> PathBuf {
        self.entry
            .parent()
            .and_then(Path::parent)
            .unwrap_or(Path::new(""))
            .join("dist/src/internal/daemon.js")
    }
}

pub(crate) struct AgentDaemon {
    base_url: String,
    token: String,
    process: Option<HubProcess>,
    tool: AgentTool,
    state_dir: PathBuf,
}

impl AgentDaemon {
    fn alive(&mut self) -> bool {
        self.process.as_mut().is_none_or(HubProcess::alive)
    }

    pub(crate) fn signal_stop(&self) {
        if let Some(process) = &self.process {
            process.signal_stop();
        }
    }

    pub(crate) async fn stop(self) {
        match self.process {
            Some(process) => process.stop().await,
            None => {
                let stop = tokio::process::Command::new(&self.tool.node)
                    .arg(&self.tool.entry)
                    .args(["daemon", "stop", "--state-dir"])
                    .arg(&self.state_dir)
                    .env("AGENT_DEVICE_NO_UPDATE_NOTIFIER", "1")
                    .kill_on_drop(true)
                    .output();
                let _ = tokio::time::timeout(Duration::from_secs(10), stop).await;
            }
        }
    }
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct DaemonInfo {
    http_port: u16,
    token: String,
}

fn read_daemon_info(path: &Path) -> Option<DaemonInfo> {
    serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()
}

async fn healthy(http: &reqwest::Client, base_url: &str) -> bool {
    http.get(format!("{base_url}/health"))
        .timeout(Duration::from_secs(2))
        .send()
        .await
        .is_ok_and(|response| response.status().as_u16() == 200)
}

pub(crate) async fn start_daemon(
    http: &reqwest::Client,
    tool: &AgentTool,
    state_dir: &Path,
    timeout: Duration,
) -> Result<AgentDaemon, String> {
    std::fs::create_dir_all(state_dir).map_err(|e| e.to_string())?;
    let info_path = state_dir.join(DAEMON_FILE);
    let daemon = |info: DaemonInfo, process| AgentDaemon {
        base_url: format!("http://127.0.0.1:{}", info.http_port),
        token: info.token,
        process,
        tool: tool.clone(),
        state_dir: state_dir.to_path_buf(),
    };
    if let Some(info) = read_daemon_info(&info_path)
        && healthy(http, &format!("http://127.0.0.1:{}", info.http_port)).await
    {
        return Ok(daemon(info, None));
    }
    let _ = std::fs::remove_file(&info_path);
    let mut command = hub_process::lifeline_command(&tool.node);
    command
        .arg(tool.daemon_entry())
        .env("AGENT_DEVICE_STATE_DIR", state_dir)
        .env("AGENT_DEVICE_DAEMON_SERVER_MODE", "http")
        .env("AGENT_DEVICE_DAEMON_IDLE_TIMEOUT_MS", "0")
        .env("AGENT_DEVICE_NO_UPDATE_NOTIFIER", "1")
        .env("FORCE_COLOR", "0")
        .env("NO_COLOR", "1");
    let child = command
        .spawn()
        .map_err(|e| format!("starting agent-device: {e}"))?;
    let mut process = HubProcess::owned(String::new(), child);
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        if let Some(info) = read_daemon_info(&info_path) {
            let mut daemon = daemon(info, None);
            process.origin = daemon.base_url.clone();
            daemon.process = Some(process);
            tracing::info!("agent-device daemon started");
            return Ok(daemon);
        }
        if !process.alive() {
            process.stop().await;
            return Err("agent-device exited before it was ready.".into());
        }
        if tokio::time::Instant::now() >= deadline {
            process.stop().await;
            return Err(format!(
                "Agent tools did not start within {} ms.",
                timeout.as_millis()
            ));
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

fn key(value: &str) -> String {
    Sha256::digest(value.as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>()[..24]
        .to_string()
}

pub(crate) fn session_name(chat_id: &str, host_id: &str, device_id: &str) -> String {
    let scope = serde_json::to_string(&[chat_id, host_id, device_id]).unwrap_or_default();
    format!("zeron-{}", key(&scope))
}

fn write_if_changed(path: &Path, content: &str) -> std::io::Result<bool> {
    if std::fs::read_to_string(path).is_ok_and(|current| current == content) {
        return Ok(false);
    }
    std::fs::write(path, content)?;
    Ok(true)
}

fn write_host_config(path: &Path, base_url: &str, token: &str) -> std::io::Result<()> {
    use std::io::Write as _;
    let content = json!({ "daemonBaseUrl": base_url, "daemonAuthToken": token }).to_string();
    if std::fs::read_to_string(path).is_ok_and(|current| current == content) {
        return Ok(());
    }
    let dir = path.parent().unwrap_or(Path::new("."));
    std::fs::create_dir_all(dir)?;
    let mut file = tempfile::Builder::new()
        .prefix(".endpoint-")
        .tempfile_in(dir)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.as_file()
            .set_permissions(std::fs::Permissions::from_mode(0o600))?;
    }
    file.write_all(content.as_bytes())?;
    file.persist(path).map_err(|e| e.error)?;
    Ok(())
}

fn sh_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\"'\"'"))
}

fn launcher_script(node: &Path, entry: &Path) -> String {
    let js = |path: &Path| serde_json::to_string(&path.to_string_lossy()).unwrap_or_default();
    format!(
        r#"import {{ spawn }} from "node:child_process";
const args = process.argv.slice(2);
const informational = args.length === 1 && ["help", "--help", "-h", "--version", "version"].includes(args[0]);
const hasValue = flag => {{ const index = args.indexOf(flag); return index >= 0 && !!args[index + 1] && !args[index + 1].startsWith("--"); }};
if (!informational && !(hasValue("--config") && hasValue("--session"))) {{
  console.error({refusal});
  process.exit(1);
}}
const env = {{ ...process.env }};
delete env.AGENT_DEVICE_DAEMON_BASE_URL;
delete env.AGENT_DEVICE_DAEMON_AUTH_TOKEN;
delete env.AGENT_DEVICE_CONFIG;
const child = spawn({node}, [{entry}, ...args], {{ stdio: "inherit", env }});
child.on("error", error => {{ console.error(error.message); process.exitCode = 1; }});
child.on("exit", code => {{ process.exitCode = code ?? 1; }});
"#,
        refusal = serde_json::to_string(LAUNCHER_REFUSAL).unwrap_or_default(),
        node = js(node),
        entry = js(entry),
    )
}

pub(crate) fn write_shim(dir: &Path, tool: &AgentTool) -> std::io::Result<PathBuf> {
    std::fs::create_dir_all(dir)?;
    let launcher = dir.join(LAUNCHER);
    write_if_changed(&launcher, &launcher_script(&tool.node, &tool.entry))?;
    let shim = dir.join(SHIM);
    let script = format!(
        "#!/bin/sh\nexec {} {} \"$@\"\n",
        sh_quote(&tool.node.to_string_lossy()),
        sh_quote(&launcher.to_string_lossy())
    );
    if write_if_changed(&shim, &script)? {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&shim, std::fs::Permissions::from_mode(0o755))?;
        }
    }
    Ok(shim)
}

pub(crate) fn remove_shim(dir: &Path) {
    for name in [SHIM, LAUNCHER] {
        let _ = std::fs::remove_file(dir.join(name));
    }
}

pub(crate) fn pick_device<'a>(
    devices: &'a [HubDevice],
    device_id: Option<&str>,
    platform: Option<DevicePlatform>,
    host_id: Option<&str>,
) -> Result<&'a HubDevice, String> {
    let host_id = host_id.unwrap_or(LOCAL_DEVICE_HOST_ID);
    if let Some(device_id) = device_id {
        return devices
            .iter()
            .find(|device| device.host_id == host_id && device.id == device_id)
            .ok_or_else(|| {
                format!(
                    "No device {device_id} on host {host_id}. Call device_list for current ids."
                )
            });
    }
    let candidates: Vec<&HubDevice> = devices
        .iter()
        .filter(|device| device.host_id == host_id && platform.is_none_or(|p| device.platform == p))
        .collect();
    let Some(first) = candidates.first() else {
        return Err(match platform {
            None => "No simulators or emulators were found. Call device_list to see why.".into(),
            Some(platform) => format!(
                "No {} devices were found on host {host_id}. Call device_list to see why.",
                platform_name(platform)
            ),
        });
    };
    if platform.is_none() && candidates.iter().any(|d| d.platform != first.platform) {
        return Err(BOTH_PLATFORMS.into());
    }
    Ok(candidates
        .iter()
        .find(|device| device.booted)
        .unwrap_or(first))
}

fn platform_name(platform: DevicePlatform) -> &'static str {
    match platform {
        DevicePlatform::Ios => "ios",
        DevicePlatform::Android => "android",
    }
}

fn target_args(device: &HubDevice) -> Vec<String> {
    let (platform, flag) = match device.platform {
        DevicePlatform::Ios => ("ios", "--udid"),
        DevicePlatform::Android => ("android", "--serial"),
    };
    ["--platform", platform, flag, &device.id]
        .map(str::to_string)
        .to_vec()
}

fn shell_word(value: &str) -> String {
    let plain = !value.is_empty()
        && value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "_./:-".contains(c));
    if plain {
        value.to_string()
    } else {
        sh_quote(value)
    }
}

pub(crate) fn quick_start(device: &HubDevice, target_args: &[String], command: &str) -> String {
    let exe = shell_word(command);
    let target = target_args
        .iter()
        .map(|arg| shell_word(arg))
        .collect::<Vec<_>>()
        .join(" ");
    let platform_note = match device.platform {
        DevicePlatform::Ios => {
            "First use builds an XCTest runner and can take a couple of minutes; later commands are fast."
        }
        DevicePlatform::Android => "The Android snapshot helper installs itself on first use.",
    };
    [
        format!(
            "The user is watching {} ({}) in the Device panel.",
            device.name, device.version
        ),
        format!(
            "Drive it with {exe}. Use this exact executable path; login shells may reset PATH. Always pass {target}."
        ),
        "Typical loop:".into(),
        format!("  {exe} open <bundle-or-package-id> {target}     # or: open <app> <deep-link-url>"),
        format!("  {exe} snapshot -i {target}                     # accessibility tree with @eN refs"),
        format!("  {exe} click @e3 {target}"),
        format!("  {exe} fill @e5 \"text\" {target}"),
        format!("  {exe} screenshot /tmp/shot.png {target}        # or call device_screenshot"),
        format!("  {exe} install <app> <path-to-.app-or-.apk> {target}"),
        format!(
            "Prefer snapshot refs over coordinates. Run {exe} help for workflow guides and {exe} <command> --help for flags."
        ),
        "Prefer agent-device for driving this device. simctl, adb, and xcrun remain available for anything it does not cover.".into(),
        "For remote hosts, arrange builds, app installation, and any Metro reverse forwarding yourself. Zeron provides discovery, streaming, and control only.".into(),
        "Keep the returned --config and --session flags on every command. Other hosts can be used concurrently; opening one does not switch these commands.".into(),
        platform_note.into(),
    ]
    .join("\n")
}

pub(crate) fn png_dimensions(png: &[u8]) -> (u32, u32) {
    let word = |at: usize| u32::from_be_bytes([png[at], png[at + 1], png[at + 2], png[at + 3]]);
    let is_png = png.len() >= 24
        && png[..8] == [0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a]
        && &png[12..16] == b"IHDR";
    if is_png { (word(16), word(20)) } else { (0, 0) }
}

async fn capture_simulator(udid: &str) -> Result<Vec<u8>, String> {
    let dir = tempfile::tempdir().map_err(|e| e.to_string())?;
    let file = dir.path().join("screen.png");
    let run = tokio::process::Command::new("xcrun")
        .args(["simctl", "io", udid, "screenshot"])
        .arg(&file)
        .kill_on_drop(true)
        .output();
    let output = tokio::time::timeout(SCREENSHOT_TIMEOUT, run)
        .await
        .map_err(|_| "The simulator screenshot timed out.".to_string())?
        .map_err(|e| format!("Could not run simctl: {e}"))?;
    if !output.status.success() {
        let detail = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(format!("The simulator screenshot failed: {detail}"));
    }
    std::fs::read(&file).map_err(|e| format!("The simulator screenshot failed: {e}"))
}

impl DeviceHub {
    fn agent_gate(&self) -> Result<(), String> {
        let state = self.state.borrow();
        if !state.settings.agent_access_enabled {
            return Err(AGENT_ACCESS_OFF.into());
        }
        if !state.settings.enabled {
            return Err(TOOL_SUPPORT_OFF.into());
        }
        Ok(())
    }

    pub fn agent_access_enabled(&self) -> bool {
        let state = self.state.borrow();
        state.settings.enabled && state.settings.agent_access_enabled
    }

    fn shim_dir(&self) -> PathBuf {
        self.device_dir().join("bin")
    }

    pub(super) fn agent_slot(&self) -> std::sync::MutexGuard<'_, Option<AgentDaemon>> {
        self.agent.lock().unwrap_or_else(PoisonError::into_inner)
    }

    async fn agent_tool(&self) -> Result<AgentTool, String> {
        #[cfg(test)]
        if let Some(tool) = self
            .test_agent
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
        {
            return Ok(tool);
        }
        let entry = toolchain::ensure_agent().await?;
        let (node, _) = toolchain::launch(&entry)?;
        Ok(AgentTool { node, entry })
    }

    fn live_agent(&self) -> Option<(String, String, AgentTool)> {
        let mut slot = self.agent_slot();
        let daemon = slot.as_mut()?;
        if !daemon.alive() {
            return None;
        }
        Some((
            daemon.base_url.clone(),
            daemon.token.clone(),
            daemon.tool.clone(),
        ))
    }

    pub(crate) async fn ensure_agent(&self) -> Result<(String, String), String> {
        let _start = self.agent_start.lock().await;
        let (base_url, token, tool) = match self.live_agent() {
            Some(live) => live,
            None => {
                self.agent_gate()?;
                let tool = self.agent_tool().await?;
                let stale = self.agent_slot().take();
                if let Some(stale) = stale {
                    stale.stop().await;
                }
                let daemon = start_daemon(
                    &self.http,
                    &tool,
                    &self.device_dir().join("agent-device"),
                    DAEMON_READY_TIMEOUT,
                )
                .await?;
                let live = (daemon.base_url.clone(), daemon.token.clone(), tool);
                let mut slot = self.agent_slot();
                if self.agent_gate().is_err()
                    || self.closed.load(std::sync::atomic::Ordering::SeqCst)
                {
                    drop(slot);
                    daemon.stop().await;
                    return Err(AGENT_ACCESS_OFF.into());
                }
                *slot = Some(daemon);
                live
            }
        };
        let slot = self.agent_slot();
        self.agent_gate()?;
        write_shim(&self.shim_dir(), &tool)
            .map_err(|e| format!("Could not prepare the agent-device launcher: {e}"))?;
        drop(slot);
        Ok((base_url, token))
    }

    pub(crate) async fn stop_agent(&self) {
        let daemon = {
            let mut slot = self.agent_slot();
            remove_shim(&self.shim_dir());
            slot.take()
        };
        if let Some(daemon) = daemon {
            daemon.stop().await;
        }
    }

    async fn agent_target(&self, chat_id: &str, device: &HubDevice) -> Result<Vec<String>, String> {
        let (base_url, token) = self.ensure_agent().await?;
        let config = self
            .device_dir()
            .join("hosts")
            .join(format!("{}.json", key(&device.host_id)));
        write_host_config(&config, &base_url, &token)
            .map_err(|e| format!("Could not write the agent-device host config: {e}"))?;
        Ok(vec![
            "--config".into(),
            config.to_string_lossy().into_owned(),
            "--session".into(),
            session_name(chat_id, &device.host_id, &device.id),
        ])
    }

    pub async fn tool_list(&self, params: DeviceToolParams) -> Result<Value, String> {
        self.agent_gate()?;
        let state = self.list(false).await?;
        let host = params.host_id.as_deref();
        let keep = |id: &str| host.is_none_or(|host| host == id);
        let status = serde_json::to_value(state.host_status).unwrap_or_default();
        let hosts: Vec<_> = state.hosts.iter().filter(|h| keep(&h.id)).collect();
        Ok(json!({
            "hostStatuses": hosts
                .iter()
                .map(|h| (h.id.clone(), status.clone()))
                .collect::<serde_json::Map<_, _>>(),
            "hosts": hosts,
            "devices": state.devices.iter().filter(|d| keep(&d.host_id)).collect::<Vec<_>>(),
            "open": state
                .sessions
                .iter()
                .filter(|s| s.chat_id == params.chat_id)
                .map(|s| json!({ "hostId": s.host_id, "deviceId": s.device_id }))
                .collect::<Vec<_>>(),
        }))
    }

    pub async fn tool_open(&self, params: DeviceToolParams) -> Result<Value, String> {
        self.agent_gate()?;
        let state = self.list(false).await?;
        let device = pick_device(
            &state.devices,
            params.device_id.as_deref(),
            params.platform,
            params.host_id.as_deref(),
        )?
        .clone();
        let agent_args = self.agent_target(&params.chat_id, &device).await?;
        let session = self
            .open(OpenHubDeviceParams {
                chat_id: params.chat_id,
                host_id: Some(device.host_id.clone()),
                device_id: device.id.clone(),
                platform: device.platform,
                boot: true,
            })
            .await?;
        let device = self
            .snapshot()
            .device(&session.host_id, &session.device_id)
            .cloned()
            .unwrap_or(device);
        let command = self.shim_dir().join(SHIM).to_string_lossy().into_owned();
        let mut args = target_args(&device);
        args.extend(agent_args);
        Ok(json!({
            "device": device,
            "agentDevice": { "command": command, "targetArgs": args },
            "quickStart": quick_start(&device, &args, &command),
        }))
    }

    pub async fn tool_screenshot(&self, params: DeviceToolParams) -> Result<Value, String> {
        self.agent_gate()?;
        let state = self.snapshot();
        let (host_id, device_id) = match params.device_id {
            Some(device_id) => (
                params
                    .host_id
                    .unwrap_or_else(|| LOCAL_DEVICE_HOST_ID.into()),
                device_id,
            ),
            None => state
                .sessions
                .iter()
                .filter(|s| s.chat_id == params.chat_id)
                .filter(|s| params.host_id.as_deref().is_none_or(|h| s.host_id == h))
                .next_back()
                .map(|s| (s.host_id.clone(), s.device_id.clone()))
                .ok_or_else(|| NO_OPEN_DEVICE.to_string())?,
        };
        let device = state
            .device(&host_id, &device_id)
            .filter(|_| hub_client::valid_device_id(&device_id))
            .cloned()
            .ok_or_else(|| UNKNOWN_DEVICE.to_string())?;
        if device.platform != DevicePlatform::Ios {
            return Err(IOS_ONLY.into());
        }
        let png = capture_simulator(&device.id).await?;
        let (width, height) = png_dimensions(&png);
        Ok(json!({
            "device": device,
            "screenshot": {
                "mimeType": "image/png",
                "data": base64::engine::general_purpose::STANDARD.encode(&png),
                "width": width,
                "height": height,
            },
        }))
    }

    pub async fn tool_close(&self, params: CloseHubDeviceParams) -> Result<(), String> {
        self.agent_gate()?;
        self.close(params).await
    }

    #[cfg(test)]
    pub(crate) fn use_agent_for_test(&self, tool: AgentTool) {
        *self
            .test_agent
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Some(tool);
    }
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use std::path::{Path, PathBuf};
    use std::sync::Arc;
    use std::time::Duration;

    use serde_json::json;
    use zeron_proto::device_hub::{ConfigureDeviceHubParams, DeviceToolParams, HubDevice};
    use zeron_rpc::{RpcError, RpcService, methods};

    use super::super::tests::{HEALTHY, enabled_hub, eventually};
    use super::*;

    const FAKE_DAEMON: &str = r#"const http = require("node:http"), fs = require("node:fs"), path = require("node:path");
const dir = process.env.AGENT_DEVICE_STATE_DIR;
const server = http.createServer((req, res) => { res.writeHead(req.url === "/health" ? 200 : 404); res.end(); });
server.listen(0, "127.0.0.1", () => fs.writeFileSync(path.join(dir, "daemon.json"), JSON.stringify({
  httpPort: server.address().port, token: "tok-1", pid: process.pid,
  serverMode: process.env.AGENT_DEVICE_DAEMON_SERVER_MODE,
  idle: process.env.AGENT_DEVICE_DAEMON_IDLE_TIMEOUT_MS,
  notifier: process.env.AGENT_DEVICE_NO_UPDATE_NOTIFIER,
})));
"#;
    const FAKE_CLI: &str = r#"console.log(JSON.stringify({ args: process.argv.slice(2), baseUrl: process.env.AGENT_DEVICE_DAEMON_BASE_URL ?? null }));
"#;

    fn node() -> Option<PathBuf> {
        std::env::var_os("PATH")
            .map(|path| std::env::split_paths(&path).collect::<Vec<_>>())
            .unwrap_or_default()
            .into_iter()
            .chain(["/opt/homebrew/bin", "/usr/local/bin"].map(PathBuf::from))
            .map(|dir| dir.join("node"))
            .find(|node| node.is_file())
    }

    fn fake_agent(dir: &Path) -> Option<AgentTool> {
        let Some(node) = node() else {
            eprintln!("node not found; skipping");
            return None;
        };
        let package = dir.join("agent-device");
        let entry = package.join("bin").join("agent-device.mjs");
        std::fs::create_dir_all(entry.parent().unwrap()).unwrap();
        std::fs::write(&entry, FAKE_CLI).unwrap();
        let tool = AgentTool { node, entry };
        std::fs::create_dir_all(tool.daemon_entry().parent().unwrap()).unwrap();
        std::fs::write(tool.daemon_entry(), FAKE_DAEMON).unwrap();
        Some(tool)
    }

    fn daemon_info(state_dir: &Path) -> serde_json::Value {
        serde_json::from_str(&std::fs::read_to_string(state_dir.join(DAEMON_FILE)).unwrap())
            .unwrap()
    }

    fn pid_alive(pid: u64) -> bool {
        unsafe { libc::kill(pid as i32, 0) == 0 }
    }

    fn device(id: &str, platform: DevicePlatform, booted: bool) -> HubDevice {
        HubDevice {
            host_id: "local".into(),
            id: id.into(),
            platform,
            name: id.into(),
            booted,
            ..Default::default()
        }
    }

    fn agent_access(on: bool) -> ConfigureDeviceHubParams {
        ConfigureDeviceHubParams {
            agent_access_enabled: Some(on),
            ..Default::default()
        }
    }

    #[test]
    fn opening_without_a_platform_while_both_platforms_are_present_is_refused() {
        let devices = [
            device("SIM-1", DevicePlatform::Ios, false),
            device("SIM-2", DevicePlatform::Ios, true),
            device("EMU-1", DevicePlatform::Android, true),
        ];
        assert_eq!(
            pick_device(&devices, None, None, None),
            Err(BOTH_PLATFORMS.into())
        );
        assert_eq!(
            pick_device(&devices, None, Some(DevicePlatform::Ios), None).map(|d| &d.id[..]),
            Ok("SIM-2"),
            "a booted device wins"
        );
        assert_eq!(
            pick_device(&devices[..1], None, None, None).map(|d| &d.id[..]),
            Ok("SIM-1")
        );
        assert_eq!(
            pick_device(&devices, Some("EMU-1"), None, None).map(|d| &d.id[..]),
            Ok("EMU-1")
        );
        assert_eq!(
            pick_device(&devices, Some("GONE"), None, None),
            Err("No device GONE on host local. Call device_list for current ids.".into())
        );
        assert_eq!(
            pick_device(&devices[..2], None, Some(DevicePlatform::Android), None),
            Err("No android devices were found on host local. Call device_list to see why.".into())
        );
        assert_eq!(
            pick_device(&[], None, None, None),
            Err("No simulators or emulators were found. Call device_list to see why.".into())
        );
    }

    #[test]
    fn the_session_name_and_png_size_match_t3code() {
        assert_eq!(
            session_name("chat-1", "local", "SIM-1"),
            "zeron-12499efcb641bc54fde3ba9f"
        );
        assert_eq!(key("local"), "25bf8e1a2393f1108d37029b");
        let mut png = vec![0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a, 0, 0, 0, 13];
        png.extend_from_slice(b"IHDR");
        png.extend_from_slice(&1206u32.to_be_bytes());
        png.extend_from_slice(&2622u32.to_be_bytes());
        assert_eq!(png_dimensions(&png), (1206, 2622));
        assert_eq!(png_dimensions(b"GIF89a not a png at all"), (0, 0));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn every_device_tool_is_refused_while_agent_access_is_off() {
        let dir = tempfile::tempdir().unwrap();
        let core = crate::EngineCore::assemble(
            dir.path(),
            Arc::new(crate::HarnessRegistry::new()),
            zeron_proto::HarnessId::Mock,
            None,
        )
        .unwrap();
        let (origin, _) = super::super::tests::fake_hub().await;
        core.device_hub.attach_hub_for_test(origin);
        let rpc = core.rpc_service();
        let calls = [
            (methods::DEVICE_TOOL_LIST, json!({"chatId": "c"})),
            (
                methods::DEVICE_TOOL_OPEN,
                json!({"chatId": "c", "platform": "ios"}),
            ),
            (
                methods::DEVICE_TOOL_SCREENSHOT,
                json!({"chatId": "c", "deviceId": "SIM-1"}),
            ),
            (
                methods::DEVICE_TOOL_CLOSE,
                json!({"chatId": "c", "shutdown": true}),
            ),
        ];
        let refusals = async |expected: &str| {
            for (method, params) in &calls {
                match rpc.handle(method, params.clone()).await {
                    Err(RpcError::Failed(message)) => assert_eq!(message, expected, "{method}"),
                    Err(other) => panic!("{method}: {other}"),
                    Ok(_) => panic!("{method} must be refused"),
                }
            }
        };
        refusals(AGENT_ACCESS_OFF).await;
        rpc.handle(methods::CONFIGURE_DEVICE_HUB, json!({"enabled": true}))
            .await
            .unwrap();
        refusals(AGENT_ACCESS_OFF).await;
        core.device_hub.state.send_modify(|state| {
            state.settings.enabled = false;
            state.settings.agent_access_enabled = true;
        });
        refusals(TOOL_SUPPORT_OFF).await;
        assert!(core.device_hub.snapshot().sessions.is_empty());
        assert!(!dir.path().join("device").join("hosts").exists());
        assert!(!dir.path().join("device").join("bin").join(SHIM).exists());
        core.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn the_shim_exists_only_while_agent_access_is_on() {
        let (dir, hub, _) = enabled_hub(HEALTHY).await;
        let Some(tool) = fake_agent(dir.path()) else {
            return;
        };
        hub.use_agent_for_test(tool.clone());
        let bin = dir.path().join("device").join("bin");
        let state_dir = dir.path().join("device").join("agent-device");
        assert!(bin.is_dir() && !bin.join(SHIM).exists());

        hub.configure(agent_access(true)).await.unwrap();
        assert!(hub.agent_access_enabled());
        let shim = std::fs::read_to_string(bin.join(SHIM)).unwrap();
        assert_eq!(
            shim,
            format!(
                "#!/bin/sh\nexec '{}' '{}' \"$@\"\n",
                tool.node.display(),
                bin.join(LAUNCHER).display()
            )
        );
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(bin.join(SHIM))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o755
        );
        let launcher = std::fs::read_to_string(bin.join(LAUNCHER)).unwrap();
        assert!(launcher.contains(&serde_json::to_string(&tool.entry).unwrap()));
        let info = daemon_info(&state_dir);
        assert_eq!(info["serverMode"], "http");
        assert_eq!(info["idle"], "0");
        assert_eq!(info["notifier"], "1");
        let pid = info["pid"].as_u64().unwrap();
        assert!(pid_alive(pid));

        hub.configure(agent_access(false)).await.unwrap();
        assert!(!bin.join(SHIM).exists() && !bin.join(LAUNCHER).exists());
        assert!(bin.is_dir(), "the PATH directory stays");
        eventually(|| !pid_alive(pid)).await;

        hub.configure(agent_access(true)).await.unwrap();
        assert!(bin.join(SHIM).exists());
        hub.configure(ConfigureDeviceHubParams {
            enabled: Some(false),
            ..Default::default()
        })
        .await
        .unwrap();
        assert!(
            !bin.join(SHIM).exists(),
            "disabling the hub revokes agent access"
        );
        hub.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn the_launcher_refuses_commands_without_the_device_open_flags() {
        let dir = tempfile::tempdir().unwrap();
        let Some(tool) = fake_agent(dir.path()) else {
            return;
        };
        let shim = write_shim(&dir.path().join("bin"), &tool).unwrap();
        let run = |args: &[&str]| {
            std::process::Command::new(&shim)
                .args(args)
                .env("AGENT_DEVICE_DAEMON_BASE_URL", "http://127.0.0.1:1")
                .output()
                .unwrap()
        };
        for args in [
            &["snapshot", "-i"][..],
            &["snapshot", "--config", "c.json"],
            &["snapshot", "--config", "--session", "s"],
        ] {
            let refused = run(args);
            assert_eq!(refused.status.code(), Some(1), "{args:?}");
            assert_eq!(
                String::from_utf8_lossy(&refused.stderr).trim(),
                LAUNCHER_REFUSAL
            );
            assert!(refused.stdout.is_empty());
        }
        let help = run(&["--help"]);
        assert!(help.status.success());
        let pinned = run(&[
            "snapshot",
            "-i",
            "--config",
            "c.json",
            "--session",
            "zeron-1",
        ]);
        assert!(pinned.status.success());
        let seen: serde_json::Value = serde_json::from_slice(&pinned.stdout).unwrap();
        assert_eq!(
            seen,
            json!({"args": ["snapshot", "-i", "--config", "c.json", "--session", "zeron-1"], "baseUrl": null})
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn the_daemon_is_reused_while_healthy_and_dies_when_its_lifeline_closes() {
        let dir = tempfile::tempdir().unwrap();
        let Some(tool) = fake_agent(dir.path()) else {
            return;
        };
        let http = reqwest::Client::builder().no_proxy().build().unwrap();
        let state_dir = dir.path().join("state");
        let mut daemon = start_daemon(&http, &tool, &state_dir, Duration::from_secs(20))
            .await
            .expect("the daemon writes daemon.json");
        let pid = daemon_info(&state_dir)["pid"].as_u64().unwrap();
        assert!(daemon.base_url.starts_with("http://127.0.0.1:") && daemon.token == "tok-1");

        let reused = start_daemon(&http, &tool, &state_dir, Duration::from_secs(20))
            .await
            .unwrap();
        assert!(reused.process.is_none());
        assert_eq!(reused.base_url, daemon.base_url);

        let (child, lifeline) = daemon.process.as_mut().unwrap().detach_for_test();
        let mut child = child.unwrap();
        drop(daemon);
        assert!(pid_alive(pid), "only the lifeline closing may stop it");
        drop(lifeline);
        tokio::time::timeout(Duration::from_secs(5), child.wait())
            .await
            .expect("the daemon exits once its lifeline closes")
            .unwrap();
        eventually(|| !pid_alive(pid)).await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn device_open_registers_the_session_and_pins_agent_device_to_it() {
        let (dir, hub, _) = enabled_hub(HEALTHY).await;
        let Some(tool) = fake_agent(dir.path()) else {
            return;
        };
        hub.use_agent_for_test(tool);
        hub.configure(agent_access(true)).await.unwrap();
        let opened = hub
            .tool_open(DeviceToolParams {
                chat_id: "chat-1".into(),
                ..Default::default()
            })
            .await
            .unwrap();
        let device_dir = dir.path().join("device");
        let config = device_dir
            .join("hosts")
            .join("25bf8e1a2393f1108d37029b.json");
        let command = device_dir.join("bin").join(SHIM);
        assert_eq!(opened["device"]["id"], "SIM-1");
        assert_eq!(opened["agentDevice"]["command"], json!(command));
        assert_eq!(
            opened["agentDevice"]["targetArgs"],
            json!([
                "--platform",
                "ios",
                "--udid",
                "SIM-1",
                "--config",
                config,
                "--session",
                "zeron-12499efcb641bc54fde3ba9f"
            ])
        );
        let quick_start = opened["quickStart"].as_str().unwrap();
        assert!(
            quick_start
                .starts_with("The user is watching iPhone 17 (iOS 27.0) in the Device panel.\n")
        );
        assert!(quick_start.contains("Zeron provides discovery, streaming, and control only."));
        assert!(quick_start.contains(&format!("Drive it with {}.", command.display())));

        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(&config).unwrap().permissions().mode() & 0o777,
            0o600
        );
        let endpoint: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&config).unwrap()).unwrap();
        let port = daemon_info(&device_dir.join("agent-device"))["httpPort"].clone();
        assert_eq!(
            endpoint["daemonBaseUrl"],
            format!("http://127.0.0.1:{port}")
        );
        assert_eq!(endpoint["daemonAuthToken"], "tok-1");
        assert!(hub.snapshot().session("chat-1", "local", "SIM-1").is_some());

        let listed = hub
            .tool_list(DeviceToolParams {
                chat_id: "chat-1".into(),
                ..Default::default()
            })
            .await
            .unwrap();
        assert_eq!(listed["hostStatuses"], json!({"local": "ready"}));
        assert_eq!(
            listed["open"],
            json!([{"hostId": "local", "deviceId": "SIM-1"}])
        );
        assert_eq!(listed["devices"][0]["id"], "SIM-1");

        let screenshot = hub
            .tool_screenshot(DeviceToolParams {
                chat_id: "chat-2".into(),
                ..Default::default()
            })
            .await;
        assert_eq!(screenshot, Err(NO_OPEN_DEVICE.into()));
        hub.shutdown().await;
    }
}
