use std::path::{Path, PathBuf};
use std::sync::PoisonError;
use std::time::Duration;

use base64::Engine as _;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use zeron_proto::device_hub::{
    CloseHubDeviceParams, DeviceHostStatus, DeviceOpener, DevicePlatform, DeviceSession,
    DeviceToolParams, HubDevice, LOCAL_DEVICE_HOST_ID, OpenHubDeviceParams,
};

use super::hub_process::{self, HubProcess};
use super::{DeviceHub, UNKNOWN_DEVICE, hub_client, toolchain};

pub const AGENT_ACCESS_OFF: &str = "Agent device access is turned off for this environment.";
pub const TOOL_SUPPORT_OFF: &str = "Device support is off. Ask the user to enable it in the Device panel before installing or starting device tools.";
pub const BOTH_PLATFORMS: &str =
    "Both iOS and Android devices are available; pass platform or deviceId.";
const NO_OPEN_DEVICE: &str = "No device is open in this thread. Call device_open first.";
const NOT_OPEN_HERE: &str = "That device is not open in this thread. Call device_open first.";
pub const DEVICE_CALLER_REFUSED: &str =
    "Device access refused: this call does not come from the chat's live run.";
const FOREIGN_DAEMON: &str =
    "An agent-device daemon that Zeron did not start is running and could not be stopped.";
pub(crate) const LAUNCHER_REFUSAL: &str =
    "Call device_open first and include its --config and --session flags.";
pub const DEVICE_TOOL_INSTRUCTIONS: &str = "## Zeron devices

The `zeron` MCP server also exposes `device_*` tools for iOS Simulators and Android Emulators on this environment. For mobile verification, call `device_list`, then `device_open` so the user can watch the device in their Device panel; its result explains how to drive the device. Driving happens through the `agent-device` CLI, using the exact launcher path returned by `device_open`. Keep the host config and session flags returned by `device_open` on every command so concurrent devices stay independent: prefer `agent-device snapshot -i` refs over coordinates, and use `device_screenshot` when you need to see the screen. Prefer these tools and `agent-device` for opening and driving devices. Platform tools such as `xcrun simctl` and `adb` remain available for anything they do not cover, such as builds, logs, or port forwarding. If `device_list` reports a platform as unavailable, say so.";
const SHIM: &str = "agent-device";
const LAUNCHER: &str = "agent-device-launcher.mjs";
const DAEMON_FILE: &str = "daemon.json";
const DAEMON_READY_TIMEOUT: Duration = Duration::from_secs(30);
const SCREENSHOT_TIMEOUT: Duration = Duration::from_secs(30);
const MAX_SCREENSHOT_SIDE: u32 = 1568;
const SCREENSHOT_LIMIT: usize = 64 << 20;
const AGENT_ENV: &str = "AGENT_DEVICE_";

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
    process: HubProcess,
    tool: AgentTool,
    android_root: Option<PathBuf>,
}

impl AgentDaemon {
    fn alive(&mut self) -> bool {
        self.process.alive()
    }

    pub(crate) fn signal_stop(&self) {
        self.process.signal_stop();
    }

    pub(crate) async fn stop(self) {
        self.process.stop().await;
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

async fn stop_foreign_daemon(
    http: &reqwest::Client,
    tool: &AgentTool,
    state_dir: &Path,
    base_url: &str,
) -> Result<(), String> {
    let mut stop = tokio::process::Command::new(&tool.node);
    hub_process::scrub_env(
        &mut stop,
        AGENT_ENV,
        std::env::vars_os().map(|(name, _)| name),
    );
    stop.arg(&tool.entry)
        .args(["daemon", "stop", "--state-dir"])
        .arg(state_dir)
        .env("AGENT_DEVICE_NO_UPDATE_NOTIFIER", "1")
        .stdin(std::process::Stdio::null())
        .kill_on_drop(true);
    let _ = tokio::time::timeout(Duration::from_secs(10), stop.output()).await;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    while healthy(http, base_url).await {
        if tokio::time::Instant::now() >= deadline {
            return Err(FOREIGN_DAEMON.into());
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    Ok(())
}

fn daemon_command(
    tool: &AgentTool,
    state_dir: &Path,
    vars: impl IntoIterator<Item = std::ffi::OsString> + Clone,
    android_root: Option<&Path>,
) -> tokio::process::Command {
    let mut command = hub_process::lifeline_command(&tool.node);
    hub_process::scrub_env(&mut command, AGENT_ENV, vars.clone());
    hub_process::scrub_adb_env(&mut command, vars);
    if let Some(root) = android_root {
        command.envs(super::platform::android_env(
            root,
            std::env::var_os("PATH").as_deref(),
        ));
    }
    command
        .arg(tool.daemon_entry())
        .env("AGENT_DEVICE_STATE_DIR", state_dir)
        .env("AGENT_DEVICE_DAEMON_SERVER_MODE", "http")
        .env("AGENT_DEVICE_DAEMON_IDLE_TIMEOUT_MS", "0")
        .env("AGENT_DEVICE_NO_UPDATE_NOTIFIER", "1")
        .env("FORCE_COLOR", "0")
        .env("NO_COLOR", "1");
    command
}

pub(crate) async fn start_daemon(
    http: &reqwest::Client,
    tool: &AgentTool,
    state_dir: &Path,
    timeout: Duration,
    android_root: Option<PathBuf>,
) -> Result<AgentDaemon, String> {
    std::fs::create_dir_all(state_dir).map_err(|e| e.to_string())?;
    let info_path = state_dir.join(DAEMON_FILE);
    if let Some(info) = read_daemon_info(&info_path) {
        let base_url = format!("http://127.0.0.1:{}", info.http_port);
        if healthy(http, &base_url).await {
            stop_foreign_daemon(http, tool, state_dir, &base_url).await?;
        }
    }
    let _ = std::fs::remove_file(&info_path);
    let vars: Vec<_> = std::env::vars_os().map(|(name, _)| name).collect();
    let child = daemon_command(tool, state_dir, vars, android_root.as_deref())
        .spawn()
        .map_err(|e| format!("starting agent-device: {e}"))?;
    let mut process = HubProcess::owned(String::new(), child);
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        if let Some(info) = read_daemon_info(&info_path) {
            let base_url = format!("http://127.0.0.1:{}", info.http_port);
            process.origin = base_url.clone();
            tracing::info!("agent-device daemon started");
            return Ok(AgentDaemon {
                base_url,
                token: info.token,
                process,
                tool: tool.clone(),
                android_root,
            });
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

pub(super) fn sh_quote(value: &str) -> String {
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
for (const key of Object.keys(env)) if (key.startsWith("AGENT_DEVICE_")) delete env[key];
const child = spawn({node}, [{entry}, ...args], {{ stdio: "inherit", env }});
child.on("error", error => {{ console.error(error.message); process.exitCode = 1; }});
child.on("exit", code => {{ process.exitCode = code ?? 1; }});
"#,
        refusal = serde_json::to_string(LAUNCHER_REFUSAL).unwrap_or_default(),
        node = js(node),
        entry = js(entry),
    )
}

#[cfg(unix)]
pub(crate) fn prepare_tool_dir(dir: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::{DirBuilderExt, MetadataExt};
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(dir)?;
    let meta = std::fs::symlink_metadata(dir)?;
    if !meta.is_dir() || meta.uid() != unsafe { libc::geteuid() } || meta.mode() & 0o022 != 0 {
        return Err(std::io::Error::other(format!(
            "{} is a symlink, owned by another user, or writable by others",
            dir.display()
        )));
    }
    Ok(())
}

#[cfg(not(unix))]
pub(crate) fn prepare_tool_dir(dir: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)
}

pub(crate) fn write_shim(dir: &Path, tool: &AgentTool) -> std::io::Result<PathBuf> {
    prepare_tool_dir(dir)?;
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let (kind, name) = (entry.file_type()?, entry.file_name());
        if kind.is_file() && (name == SHIM || name == LAUNCHER) {
            continue;
        }
        if kind.is_dir() {
            std::fs::remove_dir_all(entry.path())?;
        } else {
            std::fs::remove_file(entry.path())?;
        }
    }
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

pub(crate) fn remove_agent_files(device_dir: &Path) {
    for name in [SHIM, LAUNCHER] {
        let _ = std::fs::remove_file(device_dir.join("bin").join(name));
    }
    for entry in std::fs::read_dir(device_dir.join("hosts"))
        .into_iter()
        .flatten()
        .flatten()
    {
        if entry.path().extension().is_some_and(|ext| ext == "json") {
            let _ = std::fs::remove_file(entry.path());
        }
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

fn fit_screenshot(png: Vec<u8>) -> Result<(Vec<u8>, u32, u32), String> {
    let (width, height) = png_dimensions(&png);
    if width.max(height) <= MAX_SCREENSHOT_SIDE {
        return Ok((png, width, height));
    }
    let unreadable =
        |e: image::ImageError| format!("The simulator screenshot could not be scaled: {e}");
    let scaled = image::load_from_memory_with_format(&png, image::ImageFormat::Png)
        .map_err(unreadable)?
        .resize(
            MAX_SCREENSHOT_SIDE,
            MAX_SCREENSHOT_SIDE,
            image::imageops::FilterType::Triangle,
        );
    let mut out = std::io::Cursor::new(Vec::new());
    scaled
        .write_to(&mut out, image::ImageFormat::Png)
        .map_err(unreadable)?;
    Ok((out.into_inner(), scaled.width(), scaled.height()))
}

pub(super) async fn capture_device(device: &HubDevice) -> Result<Vec<u8>, String> {
    match device.platform {
        DevicePlatform::Ios => capture_simulator(&device.id).await,
        DevicePlatform::Android => {
            let root = tokio::task::spawn_blocking(super::platform::android_root)
                .await
                .ok()
                .flatten()
                .ok_or(super::actions::ANDROID_SDK_MISSING)?;
            capture_emulator(&super::platform::adb(&root), &device.id).await
        }
    }
}

async fn capture_emulator(adb: &Path, serial: &str) -> Result<Vec<u8>, String> {
    let mut command = tokio::process::Command::new(adb);
    command.args(["-s", serial, "exec-out", "screencap", "-p"]);
    let run = super::actions::capped_output(command, SCREENSHOT_LIMIT);
    let output = tokio::time::timeout(SCREENSHOT_TIMEOUT, run)
        .await
        .map_err(|_| "The emulator screenshot timed out.".to_string())?
        .map_err(|e| format!("Could not run adb: {e}"))?;
    if !output.status.success() || !output.stdout.starts_with(b"\x89PNG") {
        let detail = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(format!("The emulator screenshot failed: {detail}"));
    }
    Ok(output.stdout)
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
        if !toolchain::agent_installed() {
            self.set_agent_status(DeviceHostStatus::Installing, None);
        }
        let entry = toolchain::ensure_agent().await?;
        let (node, _) = toolchain::launch(&entry)?;
        Ok(AgentTool { node, entry })
    }

    fn live_agent(&self, android_root: Option<&Path>) -> Option<(String, String, AgentTool)> {
        let mut slot = self.agent_slot();
        let daemon = slot.as_mut()?;
        if !daemon.alive() || daemon.android_root.as_deref() != android_root {
            return None;
        }
        Some((
            daemon.base_url.clone(),
            daemon.token.clone(),
            daemon.tool.clone(),
        ))
    }

    pub(super) fn set_agent_status(&self, status: DeviceHostStatus, detail: Option<String>) {
        let unchanged = {
            let state = self.state.borrow();
            state.agent_status == status && state.agent_status_detail == detail
        };
        if !unchanged {
            self.publish(|state| {
                state.agent_status = status;
                state.agent_status_detail = detail;
            });
        }
    }

    pub(crate) async fn ensure_agent(&self) -> Result<(), String> {
        let ready = self.provision_agent().await;
        match &ready {
            Ok(()) => self.set_agent_status(DeviceHostStatus::Ready, None),
            Err(error) => self.set_agent_status(DeviceHostStatus::Failed, Some(error.clone())),
        }
        ready
    }

    async fn provision_agent(&self) -> Result<(), String> {
        let _start = self.agent_start.lock().await;
        let android_root = tokio::task::spawn_blocking(super::platform::android_root)
            .await
            .ok()
            .flatten();
        let (base_url, token, tool) = match self.live_agent(android_root.as_deref()) {
            Some(live) => live,
            None => {
                self.agent_gate()?;
                let tool = self.agent_tool().await?;
                self.set_agent_status(DeviceHostStatus::Starting, None);
                let stale = self.agent_slot().take();
                if let Some(stale) = stale {
                    stale.stop().await;
                }
                let daemon = start_daemon(
                    &self.http,
                    &tool,
                    &self.device_dir().join("agent-device"),
                    DAEMON_READY_TIMEOUT,
                    android_root,
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
        write_host_config(&self.host_config(LOCAL_DEVICE_HOST_ID), &base_url, &token)
            .map_err(|e| format!("Could not write the agent-device host config: {e}"))?;
        zeron_harness::set_child_path_prefix(Some(self.shim_dir()));
        drop(slot);
        Ok(())
    }

    pub(crate) fn provision_agent_in_background(&self) {
        let (Some(hub), Ok(runtime)) = (self.me.upgrade(), tokio::runtime::Handle::try_current())
        else {
            return;
        };
        if hub.agent_access_enabled() {
            runtime.spawn(async move {
                if let Err(error) = hub.ensure_agent().await {
                    tracing::warn!(%error, "agent-device could not be prepared");
                }
            });
        }
    }

    pub(crate) async fn stop_agent(&self) {
        let daemon = {
            let mut slot = self.agent_slot();
            remove_agent_files(&self.device_dir());
            zeron_harness::clear_child_path_prefix(&self.shim_dir());
            slot.take()
        };
        if let Some(daemon) = daemon {
            daemon.stop().await;
        }
        for host in self.all_remotes() {
            let _ = std::fs::remove_file(self.host_config(&host.config.id));
        }
    }

    async fn ensure_remote_agent(&self, host_id: &str) -> Result<(), String> {
        let Some(host) = self.remote(host_id) else {
            return Ok(());
        };
        let (base_url, token) = host.ensure_agent().await?;
        self.agent_gate()?;
        write_host_config(&self.host_config(host_id), &base_url, &token)
            .map_err(|e| format!("Could not write the agent-device host config: {e}"))
    }

    pub(super) fn repin_remote_agent(&self, host_id: &str) {
        let agent = self
            .remote(host_id)
            .and_then(|host| host.current())
            .and_then(|ready| ready.agent);
        if let Some((base_url, token)) = agent
            && self.agent_gate().is_ok()
            && let Err(error) = write_host_config(&self.host_config(host_id), &base_url, &token)
        {
            tracing::warn!(%error, "could not repin agent-device to the reconnected host");
        }
    }

    pub(super) fn host_config(&self, host_id: &str) -> PathBuf {
        self.device_dir()
            .join("hosts")
            .join(format!("{}.json", key(host_id)))
    }

    fn agent_target(&self, session: &DeviceSession) -> Vec<String> {
        vec![
            "--config".into(),
            self.host_config(&session.host_id)
                .to_string_lossy()
                .into_owned(),
            "--session".into(),
            session_name(&session.chat_id, &session.host_id, &session.device_id),
        ]
    }

    pub async fn tool_list(&self, params: DeviceToolParams) -> Result<Value, String> {
        self.agent_gate()?;
        let state = self.list(false).await?;
        let host = params.host_id.as_deref();
        let keep = |id: &str| host.is_none_or(|host| host == id);
        let status_of = |id: &str| {
            let status = match state.host_statuses.get(id) {
                Some(remote) => remote.status,
                None => state.host_status,
            };
            serde_json::to_value(status).unwrap_or_default()
        };
        let hosts: Vec<_> = state.hosts.iter().filter(|h| keep(&h.id)).collect();
        Ok(json!({
            "hostStatuses": hosts
                .iter()
                .map(|h| (h.id.clone(), status_of(&h.id)))
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
        let mut state = self.list(false).await?;
        if let Some(host) = params.host_id.as_deref().and_then(|id| self.remote(id)) {
            let ready = host.ensure_ready().await?;
            self.refresh(&host.config.id, &ready.origin).await?;
            state = self.snapshot();
        }
        let device = pick_device(
            &state.devices,
            params.device_id.as_deref(),
            params.platform,
            params.host_id.as_deref(),
        )?
        .clone();
        self.ensure_agent().await?;
        self.ensure_remote_agent(&device.host_id).await?;
        let session = self
            .open_as(
                OpenHubDeviceParams {
                    chat_id: params.chat_id,
                    host_id: Some(device.host_id.clone()),
                    device_id: device.id.clone(),
                    platform: device.platform,
                    boot: true,
                },
                DeviceOpener::Agent,
            )
            .await?;
        let device = self
            .snapshot()
            .device(&session.host_id, &session.device_id)
            .cloned()
            .unwrap_or(device);
        let command = self.shim_dir().join(SHIM).to_string_lossy().into_owned();
        let mut args = target_args(&device);
        args.extend(self.agent_target(&session));
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
            Some(device_id) => {
                let host_id = params
                    .host_id
                    .unwrap_or_else(|| LOCAL_DEVICE_HOST_ID.into());
                if state
                    .session(&params.chat_id, &host_id, &device_id)
                    .is_none()
                {
                    return Err(NOT_OPEN_HERE.into());
                }
                (host_id, device_id)
            }
            None => state
                .sessions
                .iter()
                .filter(|s| s.chat_id == params.chat_id)
                .filter(|s| params.host_id.as_deref().is_none_or(|h| s.host_id == h))
                .next_back()
                .map(|s| (s.host_id.clone(), s.device_id.clone()))
                .ok_or_else(|| NO_OPEN_DEVICE.to_string())?,
        };
        if !hub_client::valid_device_id(&device_id) {
            return Err(UNKNOWN_DEVICE.into());
        }
        let device = match state.device(&host_id, &device_id) {
            Some(device) => device.clone(),
            None => self
                .list(false)
                .await?
                .device(&host_id, &device_id)
                .cloned()
                .ok_or_else(|| UNKNOWN_DEVICE.to_string())?,
        };
        let png = self.capture(&device).await?;
        let (png, width, height) = tokio::task::spawn_blocking(move || fit_screenshot(png))
            .await
            .map_err(|e| e.to_string())??;
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

    pub async fn tool_close(&self, params: DeviceToolParams) -> Result<(), String> {
        self.agent_gate()?;
        let (host, device) = (params.host_id.as_deref(), params.device_id.as_deref());
        let open = self.snapshot().sessions.iter().any(|session| {
            session.chat_id == params.chat_id
                && host.is_none_or(|host| session.host_id == host)
                && device.is_none_or(|device| session.device_id == device)
        });
        if !open {
            return Err(if device.is_some() {
                NOT_OPEN_HERE
            } else {
                NO_OPEN_DEVICE
            }
            .into());
        }
        self.close(CloseHubDeviceParams {
            chat_id: params.chat_id,
            host_id: params.host_id,
            device_id: params.device_id,
            shutdown: params.shutdown,
        })
        .await
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
pub(super) mod tests {
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
    const FAKE_CLI: &str = r#"import fs from "node:fs";
const args = process.argv.slice(2);
if (args[0] === "daemon" && args[1] === "stop") {
  try { process.kill(JSON.parse(fs.readFileSync(args[3] + "/daemon.json", "utf8")).pid); } catch {}
} else {
  console.log(JSON.stringify({ args, agentEnv: Object.keys(process.env).filter(key => key.startsWith("AGENT_DEVICE_")) }));
}
"#;

    static PROVISIONING: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

    fn child_path_head() -> PathBuf {
        let mut cmd = tokio::process::Command::new("/usr/bin/true");
        zeron_harness::compose_child_path(&mut cmd, Path::new("/opt/agents/bin/claude"));
        let path = cmd
            .as_std()
            .get_envs()
            .find(|(name, _)| *name == "PATH")
            .and_then(|(_, value)| value)
            .unwrap()
            .to_os_string();
        std::env::split_paths(&path).next().unwrap()
    }

    fn mode(path: &Path) -> u32 {
        use std::os::unix::fs::PermissionsExt;
        std::fs::metadata(path).unwrap().permissions().mode() & 0o777
    }

    pub(crate) fn node() -> Option<PathBuf> {
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
                json!({"chatId": "c", "platform": "ios", "runSecret": "guess"}),
            ),
            (
                methods::DEVICE_TOOL_SCREENSHOT,
                json!({"chatId": "c", "deviceId": "SIM-1", "runSecret": "guess"}),
            ),
            (
                methods::DEVICE_TOOL_CLOSE,
                json!({"chatId": "c", "shutdown": true, "runSecret": "guess"}),
            ),
        ];
        let refused = async || {
            for (method, params) in &calls {
                match rpc.handle(method, params.clone()).await {
                    Err(RpcError::Failed(message)) => {
                        assert_eq!(message, DEVICE_CALLER_REFUSED, "{method}")
                    }
                    Err(other) => panic!("{method}: {other}"),
                    Ok(_) => panic!("{method} must be refused"),
                }
            }
        };
        refused().await;
        rpc.handle(methods::CONFIGURE_DEVICE_HUB, json!({"enabled": true}))
            .await
            .unwrap();
        let hub = &core.device_hub;
        hub.state
            .send_modify(|state| state.settings.agent_access_enabled = true);
        refused().await;

        let gated = async |expected: &str| {
            let tool = || DeviceToolParams {
                chat_id: "c".into(),
                device_id: Some("SIM-1".into()),
                platform: Some(DevicePlatform::Ios),
                shutdown: true,
                ..Default::default()
            };
            assert_eq!(hub.tool_list(tool()).await, Err(expected.into()));
            assert_eq!(hub.tool_open(tool()).await, Err(expected.into()));
            assert_eq!(hub.tool_screenshot(tool()).await, Err(expected.into()));
            assert_eq!(hub.tool_close(tool()).await, Err(expected.into()));
        };
        hub.state
            .send_modify(|state| state.settings.agent_access_enabled = false);
        gated(AGENT_ACCESS_OFF).await;
        hub.state.send_modify(|state| {
            state.settings.enabled = false;
            state.settings.agent_access_enabled = true;
        });
        gated(TOOL_SUPPORT_OFF).await;
        assert!(hub.snapshot().sessions.is_empty());
        assert!(!dir.path().join("device").join("hosts").exists());
        assert!(!dir.path().join("device").join("bin").join(SHIM).exists());
        core.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn the_shim_exists_only_while_agent_access_is_on() {
        let _serial = PROVISIONING.lock().await;
        let (dir, hub, _) = enabled_hub(HEALTHY).await;
        let Some(tool) = fake_agent(dir.path()) else {
            return;
        };
        hub.use_agent_for_test(tool.clone());
        let bin = dir.path().join("device").join("bin");
        let state_dir = dir.path().join("device").join("agent-device");
        let config = dir
            .path()
            .join("device")
            .join("hosts")
            .join("25bf8e1a2393f1108d37029b.json");
        assert!(!bin.exists());
        assert_eq!(hub.snapshot().agent_status, DeviceHostStatus::Disabled);

        hub.configure(agent_access(true)).await.unwrap();
        assert!(hub.agent_access_enabled());
        assert_eq!(hub.snapshot().agent_status, DeviceHostStatus::Ready);
        let shim = std::fs::read_to_string(bin.join(SHIM)).unwrap();
        assert_eq!(
            shim,
            format!(
                "#!/bin/sh\nexec '{}' '{}' \"$@\"\n",
                tool.node.display(),
                bin.join(LAUNCHER).display()
            )
        );
        assert_eq!(mode(&bin.join(SHIM)), 0o755);
        assert_eq!(mode(&bin), 0o700);
        assert_eq!(child_path_head(), bin, "the PATH prefix is registered");
        assert!(config.exists(), "provisioning writes the host config");
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
        assert!(!config.exists(), "the daemon token goes with it");
        assert_ne!(child_path_head(), bin, "the PATH prefix is cleared");
        assert_eq!(hub.snapshot().agent_status, DeviceHostStatus::Disabled);
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
        assert_ne!(child_path_head(), bin);
        hub.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn an_unsafe_tool_directory_is_refused_and_strays_are_removed() {
        use std::os::unix::fs::PermissionsExt;
        let _serial = PROVISIONING.lock().await;
        let (dir, hub, _) = enabled_hub(HEALTHY).await;
        let Some(tool) = fake_agent(dir.path()) else {
            return;
        };
        hub.use_agent_for_test(tool);
        let bin = dir.path().join("device").join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o777)).unwrap();
        hub.configure(agent_access(true)).await.unwrap();
        let state = hub.snapshot();
        assert_eq!(state.agent_status, DeviceHostStatus::Failed);
        assert!(
            state
                .agent_status_detail
                .as_deref()
                .is_some_and(|detail| detail.contains("writable by others")),
            "{state:?}"
        );
        assert_eq!(
            state.host_status,
            DeviceHostStatus::Ready,
            "the hub is fine"
        );
        assert!(!bin.join(SHIM).exists());
        assert_ne!(child_path_head(), bin);

        std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o700)).unwrap();
        std::fs::write(bin.join("git"), "#!/bin/sh\n").unwrap();
        std::fs::create_dir(bin.join("node")).unwrap();
        std::os::unix::fs::symlink("/bin/sh", bin.join(LAUNCHER)).unwrap();
        hub.configure(agent_access(true)).await.unwrap();
        assert_eq!(hub.snapshot().agent_status, DeviceHostStatus::Ready);
        let mut left: Vec<_> = std::fs::read_dir(&bin)
            .unwrap()
            .flatten()
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .collect();
        left.sort();
        assert_eq!(left, [SHIM, LAUNCHER]);
        assert!(
            !std::fs::symlink_metadata(bin.join(LAUNCHER))
                .unwrap()
                .is_symlink()
        );

        let real = dir.path().join("elsewhere");
        std::fs::create_dir(&real).unwrap();
        hub.configure(agent_access(false)).await.unwrap();
        std::fs::remove_dir_all(&bin).unwrap();
        std::os::unix::fs::symlink(&real, &bin).unwrap();
        hub.configure(agent_access(true)).await.unwrap();
        assert_eq!(hub.snapshot().agent_status, DeviceHostStatus::Failed);
        assert!(
            !real.join(SHIM).exists(),
            "nothing is written through a link"
        );
        hub.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn access_turning_off_never_leaves_an_agent_failure() {
        let (_dir, hub, _) = enabled_hub(HEALTHY).await;
        hub.state
            .send_modify(|state| state.settings.agent_access_enabled = true);
        hub.set_agent_status(DeviceHostStatus::Failed, Some("boom".into()));
        assert_eq!(hub.snapshot().agent_status, DeviceHostStatus::Failed);
        hub.state
            .send_modify(|state| state.settings.agent_access_enabled = false);
        hub.set_agent_status(DeviceHostStatus::Failed, Some("late".into()));
        let state = hub.snapshot();
        assert_eq!(state.agent_status, DeviceHostStatus::Disabled);
        assert_eq!(state.agent_status_detail, None);
        hub.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn agent_access_on_at_start_provisions_in_the_background() {
        let _serial = PROVISIONING.lock().await;
        let dir = tempfile::tempdir().unwrap();
        let Some(tool) = fake_agent(dir.path()) else {
            return;
        };
        std::fs::write(
            dir.path().join(super::super::SETTINGS_FILE),
            r#"{"enabled":true,"agentAccessEnabled":true,"onboardingCompleted":true}"#,
        )
        .unwrap();
        let config = dir
            .path()
            .join("device")
            .join("hosts")
            .join("25bf8e1a2393f1108d37029b.json");
        std::fs::create_dir_all(config.parent().unwrap()).unwrap();
        std::fs::write(
            &config,
            r#"{"daemonBaseUrl":"http://127.0.0.1:1","daemonAuthToken":"dead"}"#,
        )
        .unwrap();
        let hub = super::super::DeviceHub::start(dir.path());
        hub.use_agent_for_test(tool);
        hub.provision_agent_in_background();
        eventually(|| hub.snapshot().agent_status == DeviceHostStatus::Ready).await;
        let endpoint: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&config).unwrap()).unwrap();
        assert_eq!(endpoint["daemonAuthToken"], "tok-1");
        assert_ne!(endpoint["daemonBaseUrl"], "http://127.0.0.1:1");
        assert!(dir.path().join("device").join("bin").join(SHIM).exists());
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
                .env("AGENT_DEVICE_HTTP_AUTH_HOOK", "/tmp/hook.mjs")
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
            json!({"args": ["snapshot", "-i", "--config", "c.json", "--session", "zeron-1"], "agentEnv": []})
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_daemon_zeron_did_not_start_is_stopped_and_replaced_on_the_lifeline() {
        let dir = tempfile::tempdir().unwrap();
        let Some(tool) = fake_agent(dir.path()) else {
            return;
        };
        let http = reqwest::Client::builder().no_proxy().build().unwrap();
        let state_dir = dir.path().join("state");
        let mut foreign = start_daemon(&http, &tool, &state_dir, Duration::from_secs(20), None)
            .await
            .expect("the daemon writes daemon.json");
        let foreign_pid = daemon_info(&state_dir)["pid"].as_u64().unwrap();
        assert!(foreign.base_url.starts_with("http://127.0.0.1:") && foreign.token == "tok-1");
        let (foreign_child, _foreign_lifeline) = foreign.process.detach_for_test();
        let mut foreign_child = foreign_child.unwrap();
        drop(foreign);
        assert!(pid_alive(foreign_pid), "a daemon nobody here owns");

        let mut daemon = start_daemon(&http, &tool, &state_dir, Duration::from_secs(20), None)
            .await
            .unwrap();
        let pid = daemon_info(&state_dir)["pid"].as_u64().unwrap();
        assert_ne!(pid, foreign_pid, "never adopted");
        tokio::time::timeout(Duration::from_secs(5), foreign_child.wait())
            .await
            .expect("daemon stop ended the foreign daemon")
            .unwrap();

        let (child, lifeline) = daemon.process.detach_for_test();
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
    async fn a_foreign_daemon_that_will_not_stop_fails_the_agent_status_not_the_hub() {
        let (dir, hub, origin) = enabled_hub(HEALTHY).await;
        let Some(tool) = fake_agent(dir.path()) else {
            return;
        };
        hub.use_agent_for_test(tool);
        let state_dir = dir.path().join("device").join("agent-device");
        std::fs::create_dir_all(&state_dir).unwrap();
        let port = origin.rsplit(':').next().unwrap();
        std::fs::write(
            state_dir.join(DAEMON_FILE),
            format!(r#"{{"httpPort":{port},"token":"t","pid":999999}}"#),
        )
        .unwrap();
        hub.configure(agent_access(true)).await.unwrap();
        let state = hub.snapshot();
        assert_eq!(state.agent_status, DeviceHostStatus::Failed);
        assert_eq!(state.agent_status_detail.as_deref(), Some(FOREIGN_DAEMON));
        assert_eq!(state.host_status, DeviceHostStatus::Ready);
        assert!(hub.agent_slot().is_none());
        hub.shutdown().await;
    }

    #[test]
    fn the_daemon_env_drops_every_inherited_agent_device_variable() {
        let tool = AgentTool {
            node: "/usr/bin/node".into(),
            entry: "/pkg/bin/agent-device.mjs".into(),
        };
        let env_of = |android_root: Option<&Path>| -> std::collections::BTreeMap<_, _> {
            daemon_command(
                &tool,
                Path::new("/state"),
                [
                    "AGENT_DEVICE_HTTP_AUTH_HOOK",
                    "AGENT_DEVICE_STATE_DIR",
                    "ANDROID_SERIAL",
                    "PATH",
                ]
                .map(std::ffi::OsString::from),
                android_root,
            )
            .as_std()
            .get_envs()
            .map(|(name, value)| {
                (
                    name.to_string_lossy().into_owned(),
                    value.map(|v| v.to_string_lossy().into_owned()),
                )
            })
            .collect()
        };
        let env = env_of(None);
        assert_eq!(env["AGENT_DEVICE_HTTP_AUTH_HOOK"], None);
        assert_eq!(env["AGENT_DEVICE_STATE_DIR"].as_deref(), Some("/state"));
        assert_eq!(env["ANDROID_SERIAL"], None);
        assert!(!env.contains_key("PATH") && !env.contains_key("ANDROID_HOME"));
        let env = env_of(Some(Path::new("/sdk")));
        assert_eq!(env["ANDROID_HOME"].as_deref(), Some("/sdk"));
        assert!(
            env["PATH"]
                .as_deref()
                .is_some_and(|path| path.starts_with("/sdk/platform-tools:/sdk/emulator")),
            "{env:?}"
        );
    }

    #[test]
    fn screenshots_are_scaled_to_1568_on_the_long_side() {
        let encode = |width, height| {
            let mut png = std::io::Cursor::new(Vec::new());
            image::DynamicImage::new_rgb8(width, height)
                .write_to(&mut png, image::ImageFormat::Png)
                .unwrap();
            png.into_inner()
        };
        let (scaled, width, height) = fit_screenshot(encode(1206, 2622)).unwrap();
        assert_eq!((width, height), (721, 1568));
        assert_eq!(png_dimensions(&scaled), (721, 1568));
        let small = encode(600, 1200);
        assert_eq!(fit_screenshot(small.clone()), Ok((small, 600, 1200)));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn device_open_needs_a_platform_once_simulators_and_emulators_are_both_listed() {
        let (dir, hub, _) = enabled_hub(HEALTHY).await;
        hub.use_platforms_for_test(
            [DevicePlatform::Ios, DevicePlatform::Android]
                .map(
                    |platform| zeron_proto::device_hub::DevicePlatformAvailability {
                        platform,
                        available: true,
                        reason: None,
                    },
                )
                .to_vec(),
        );
        hub.use_actions_for_test(
            Arc::new(|_| {
                Box::pin(async {
                    super::super::actions::CommandOutput {
                        code: 0,
                        stdout: "Pixel_9_API_36\n".into(),
                        stderr: String::new(),
                    }
                })
            }),
            super::super::actions::Helpers {
                android_sdk: Some("/sdk".into()),
                ..Default::default()
            },
        );
        hub.state
            .send_modify(|state| state.settings.agent_access_enabled = true);
        let listed = hub
            .tool_list(DeviceToolParams {
                chat_id: "chat-1".into(),
                ..Default::default()
            })
            .await
            .unwrap();
        assert_eq!(
            listed["devices"]
                .as_array()
                .unwrap()
                .iter()
                .map(|device| (
                    device["id"].as_str().unwrap(),
                    device["platform"].as_str().unwrap()
                ))
                .collect::<Vec<_>>(),
            [("SIM-1", "ios"), ("Pixel_9_API_36", "android")]
        );
        let opened = hub
            .tool_open(DeviceToolParams {
                chat_id: "chat-1".into(),
                ..Default::default()
            })
            .await;
        assert_eq!(opened, Err(BOTH_PLATFORMS.into()));
        assert!(hub.snapshot().sessions.is_empty());

        let _serial = PROVISIONING.lock().await;
        let Some(tool) = fake_agent(dir.path()) else {
            return;
        };
        hub.use_agent_for_test(tool);
        let opened = hub
            .tool_open(DeviceToolParams {
                chat_id: "chat-1".into(),
                platform: Some(DevicePlatform::Android),
                ..Default::default()
            })
            .await
            .unwrap();
        let args: Vec<&str> = opened["agentDevice"]["targetArgs"]
            .as_array()
            .unwrap()
            .iter()
            .map(|arg| arg.as_str().unwrap())
            .collect();
        let session = session_name("chat-1", "local", "emulator-5554");
        assert_eq!(
            (args[..4].to_vec(), args[7]),
            (
                vec!["--platform", "android", "--serial", "emulator-5554"],
                session.as_str()
            ),
            "the agent session follows the serial the AVD booted under"
        );
        hub.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_remote_device_pins_agent_device_to_the_hosts_forwarded_daemon() {
        let _serial = PROVISIONING.lock().await;
        let Some(fake) = super::super::ssh_host::tests::fake_ssh().await else {
            return;
        };
        let (dir, hub, _) = enabled_hub(HEALTHY).await;
        let Some(tool) = fake_agent(dir.path()) else {
            return;
        };
        let (remote, _) = super::super::tests::fake_hub().await;
        std::fs::write(
            fake.dir.path().join("hub-port-mini"),
            remote.rsplit(':').next().unwrap(),
        )
        .unwrap();
        hub.use_ssh_for_test(fake.ssh.clone(), Default::default());
        hub.use_agent_for_test(tool);
        hub.configure(ConfigureDeviceHubParams {
            agent_access_enabled: Some(true),
            device_hosts: Some(vec![super::super::ssh_host::tests::host("mini")]),
            ..Default::default()
        })
        .await
        .unwrap();
        let opened = hub
            .tool_open(DeviceToolParams {
                chat_id: "chat-1".into(),
                host_id: Some("mini".into()),
                device_id: Some("SIM-1".into()),
                ..Default::default()
            })
            .await
            .unwrap();
        assert_eq!(opened["device"]["hostId"], "mini");
        let config = hub.host_config("mini");
        assert_eq!(opened["agentDevice"]["targetArgs"][5], json!(config));
        let endpoint: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&config).unwrap()).unwrap();
        let forwarded = hub
            .remote("mini")
            .unwrap()
            .current()
            .unwrap()
            .agent
            .unwrap();
        assert_eq!(endpoint["daemonBaseUrl"], forwarded.0);
        assert_eq!(endpoint["daemonAuthToken"], "fixture");
        assert_ne!(
            forwarded.0, remote,
            "the daemon is reached through the SSH forward"
        );
        assert!(fake.modes().contains(&"agent-start".to_string()));
        let listed = hub
            .tool_list(DeviceToolParams {
                chat_id: "chat-1".into(),
                ..Default::default()
            })
            .await
            .unwrap();
        assert_eq!(
            listed["hostStatuses"],
            json!({"local": "ready", "mini": "ready"})
        );
        let pid: i32 = std::fs::read_to_string(fake.dir.path().join("forward.pid"))
            .unwrap()
            .parse()
            .unwrap();
        unsafe { libc::kill(pid, libc::SIGKILL) };
        let read = || {
            serde_json::from_str::<serde_json::Value>(&std::fs::read_to_string(&config).unwrap())
                .unwrap()["daemonBaseUrl"]
                .clone()
        };
        for _ in 0..200 {
            if read() != forwarded.0 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        let again = hub
            .remote("mini")
            .unwrap()
            .current()
            .unwrap()
            .agent
            .unwrap();
        assert_ne!(again.0, forwarded.0);
        assert_eq!(
            read(),
            again.0,
            "a reconnect repins agent-device to the new forward"
        );
        hub.configure(agent_access(false)).await.unwrap();
        assert!(!config.exists());
        eventually(|| fake.modes().contains(&"stop-agent".to_string())).await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_host_stuck_connecting_never_holds_up_local_devices_or_its_removal() {
        let _serial = PROVISIONING.lock().await;
        let Some(fake) = super::super::ssh_host::tests::fake_ssh().await else {
            return;
        };
        let (dir, hub, _) = enabled_hub(HEALTHY).await;
        let Some(tool) = fake_agent(dir.path()) else {
            return;
        };
        hub.use_ssh_for_test(fake.ssh.clone(), Default::default());
        hub.use_agent_for_test(tool);
        let quick = Duration::from_secs(5);
        let hang = || ConfigureDeviceHubParams {
            device_hosts: Some(vec![super::super::ssh_host::tests::host("hang")]),
            ..Default::default()
        };
        let pid_file = fake.dir.path().join("hang-probe.pid");
        let stuck = || eventually(|| pid_file.exists());
        tokio::time::timeout(quick, hub.configure(agent_access(true)))
            .await
            .unwrap()
            .unwrap();
        tokio::time::timeout(quick, hub.configure(hang()))
            .await
            .expect("saving a host does not wait for it to connect")
            .unwrap();
        stuck().await;
        let state = tokio::time::timeout(quick, hub.list(false))
            .await
            .expect("a local list does not wait for the stuck host")
            .unwrap();
        assert!(state.device("local", "SIM-1").is_some());
        let listed = tokio::time::timeout(
            quick,
            hub.tool_list(DeviceToolParams {
                chat_id: "chat-1".into(),
                ..Default::default()
            }),
        )
        .await
        .expect("device_list does not wait for the stuck host")
        .unwrap();
        assert!(
            listed["devices"]
                .as_array()
                .unwrap()
                .iter()
                .any(|device| device["hostId"] == "local")
        );
        tokio::time::timeout(
            quick,
            hub.tool_open(DeviceToolParams {
                chat_id: "chat-1".into(),
                device_id: Some("SIM-1".into()),
                ..Default::default()
            }),
        )
        .await
        .expect("device_open on a local device does not wait for the stuck host")
        .unwrap();
        tokio::time::timeout(quick, hub.configure(agent_access(false)))
            .await
            .expect("turning agent access off does not wait for the stuck host")
            .unwrap();

        let pid = |file: &Path| -> i32 { std::fs::read_to_string(file).unwrap().parse().unwrap() };
        let first = pid(&pid_file);
        tokio::time::timeout(
            quick,
            hub.configure(ConfigureDeviceHubParams {
                device_hosts: Some(Vec::new()),
                ..Default::default()
            }),
        )
        .await
        .expect("removing the stuck host returns at once")
        .unwrap();
        eventually(|| super::super::ssh_host::tests::exited(first)).await;

        std::fs::remove_file(&pid_file).unwrap();
        hub.configure(hang()).await.unwrap();
        stuck().await;
        let second = pid(&pid_file);
        tokio::time::timeout(
            quick,
            hub.configure(ConfigureDeviceHubParams {
                enabled: Some(false),
                ..Default::default()
            }),
        )
        .await
        .expect("disabling returns while the host is stuck")
        .unwrap();
        eventually(|| super::super::ssh_host::tests::exited(second)).await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn device_open_registers_the_session_and_pins_agent_device_to_it() {
        let _serial = PROVISIONING.lock().await;
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
        assert_eq!(
            hub.snapshot().sessions[0].opened_by,
            Some(DeviceOpener::Agent)
        );
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
        let other = |device_id: Option<&str>| DeviceToolParams {
            chat_id: "chat-2".into(),
            device_id: device_id.map(str::to_string),
            shutdown: true,
            ..Default::default()
        };
        assert_eq!(
            hub.tool_screenshot(other(Some("SIM-1"))).await,
            Err(NOT_OPEN_HERE.into()),
            "another chat's device cannot be captured"
        );
        assert_eq!(
            hub.tool_close(other(Some("SIM-1"))).await,
            Err(NOT_OPEN_HERE.into())
        );
        assert_eq!(
            hub.tool_close(other(None)).await,
            Err(NO_OPEN_DEVICE.into())
        );
        let state = hub.snapshot();
        assert!(state.session("chat-1", "local", "SIM-1").is_some());
        assert!(state.device("local", "SIM-1").is_some_and(|d| d.booted));

        hub.state.send_modify(|state| state.devices.clear());
        let refreshed = hub
            .tool_screenshot(DeviceToolParams {
                chat_id: "chat-1".into(),
                device_id: Some("SIM-1".into()),
                ..Default::default()
            })
            .await;
        assert_ne!(refreshed, Err(UNKNOWN_DEVICE.into()), "{refreshed:?}");
        assert!(hub.snapshot().device("local", "SIM-1").is_some());
        hub.shutdown().await;
    }
}
