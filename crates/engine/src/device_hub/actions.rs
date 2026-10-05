use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, PoisonError};
use std::time::Duration;

use base64::Engine as _;
use futures::StreamExt;
use futures::future::BoxFuture;
use zeron_proto::device_hub::{
    ColorFilter, DeviceAction, DeviceAppState, DeviceAppearance, DeviceAxTree, DeviceDetail,
    DeviceEventLogBatch, DeviceEventLogEntry, DevicePermission, DevicePlatform,
    DeviceSettingsSnapshot, DeviceTextSize, DeviceToggle, ForegroundApp, HubDevice,
    HubDeviceScreenshot, HubDeviceTargetParams, LOCAL_DEVICE_HOST_ID, LiquidGlass,
    PermissionDecision, RunHubDeviceActionParams,
};

use super::hub_client::{HubClient, SseStream};
use super::{
    DEVICE_SUPPORT_OFF, DeviceHub, HUB_NOT_RUNNING, IOS_ONLY, UNKNOWN_DEVICE, hub_client, toolchain,
};

pub const DEVICE_ACTION_UNAVAILABLE: &str =
    "DeviceActionUnavailable: this device does not support that action.";
const INVALID_LOCATION: &str =
    "Latitude must be between -90 and 90 and longitude between -180 and 180.";
const INVALID_APP_ID: &str = "That is not a valid bundle ID.";
const INVALID_URL: &str = "That is not a valid URL.";
const INVALID_PUSH: &str = "A push notification needs alert text or a JSON object payload.";
pub(crate) const ACTION_TIMEOUT: Duration = Duration::from_secs(20);
const FOREGROUND_READ: Duration = Duration::from_secs(1);

pub(crate) struct CommandOutput {
    pub code: i32,
    pub stdout: String,
    pub stderr: String,
}

pub(crate) type Runner =
    Arc<dyn Fn(Vec<String>) -> BoxFuture<'static, CommandOutput> + Send + Sync>;

pub(crate) fn system_runner() -> Runner {
    Arc::new(|argv| Box::pin(run_command(argv)))
}

async fn run_command(argv: Vec<String>) -> CommandOutput {
    let failed = |stderr: String| CommandOutput {
        code: 127,
        stdout: String::new(),
        stderr,
    };
    let Some((program, args)) = argv.split_first() else {
        return failed("no command".into());
    };
    let run = tokio::process::Command::new(program)
        .args(args)
        .stdin(std::process::Stdio::null())
        .kill_on_drop(true)
        .output();
    match tokio::time::timeout(ACTION_TIMEOUT, run).await {
        Err(_) => failed("timed out after 20 s".into()),
        Ok(Err(error)) => failed(error.to_string()),
        Ok(Ok(output)) => CommandOutput {
            code: output.status.code().unwrap_or(-1),
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        },
    }
}

#[derive(Debug, Clone, Default)]
pub(crate) struct Helpers {
    pub ax_settings: Option<String>,
    pub serve_sim: Option<(String, String)>,
}

impl Helpers {
    fn installed() -> Self {
        let Some(entry) = toolchain::installed_hub() else {
            return Self::default();
        };
        let node = toolchain::launch(&entry).ok().map(|(node, _)| node);
        Self::from_entry(&entry, node)
    }

    fn from_entry(entry: &Path, node: Option<PathBuf>) -> Self {
        let Some(dist) = entry
            .ancestors()
            .map(|dir| dir.join("vendor/serve-sim/dist"))
            .find(|dist| dist.is_dir())
        else {
            return Self::default();
        };
        let file = |path: PathBuf| path.is_file().then(|| path.to_string_lossy().into_owned());
        let node = node.map(|node| node.to_string_lossy().into_owned());
        Self {
            ax_settings: file(dist.join("simax/serve-sim-ax-settings")),
            serve_sim: node.zip(file(dist.join("serve-sim.js"))),
        }
    }
}

pub(crate) fn valid_app_id(id: &str) -> bool {
    !id.is_empty()
        && !id.starts_with('-')
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
}

pub(crate) fn push_payload(payload: &serde_json::Value) -> Result<String, String> {
    let payload = match payload {
        serde_json::Value::String(text) if !text.trim().is_empty() => {
            serde_json::json!({ "aps": { "alert": text } })
        }
        serde_json::Value::Object(_) => payload.clone(),
        _ => return Err(INVALID_PUSH.into()),
    };
    Ok(payload.to_string())
}

fn ios_text_size(size: DeviceTextSize) -> &'static str {
    match size {
        DeviceTextSize::Small => "small",
        DeviceTextSize::Default => "large",
        DeviceTextSize::Large => "extra-extra-large",
        DeviceTextSize::ExtraLarge => "accessibility-large",
    }
}

pub(crate) fn text_size_from_ios(category: &str) -> Option<DeviceTextSize> {
    Some(match category {
        "unknown" | "unsupported" | "" => return None,
        "small" | "extra-small" | "medium" => DeviceTextSize::Small,
        "large" => DeviceTextSize::Default,
        "extra-extra-large" => DeviceTextSize::Large,
        _ if category.starts_with("accessibility") => DeviceTextSize::ExtraLarge,
        _ if category.contains("extra") => DeviceTextSize::Large,
        _ => DeviceTextSize::Default,
    })
}

fn ax_toggle(setting: DeviceToggle) -> Option<&'static str> {
    Some(match setting {
        DeviceToggle::ReduceMotion => "reduce-motion",
        DeviceToggle::ReduceTransparency => "reduce-transparency",
        DeviceToggle::ShowBorders => "show-borders",
        DeviceToggle::VoiceOver => "voiceover",
        DeviceToggle::IncreaseContrast | DeviceToggle::NetworkEnabled => return None,
    })
}

fn color_filter(value: ColorFilter) -> &'static str {
    match value {
        ColorFilter::None => "none",
        ColorFilter::Grayscale => "grayscale",
        ColorFilter::RedGreen => "red-green",
        ColorFilter::GreenRed => "green-red",
        ColorFilter::BlueYellow => "blue-yellow",
    }
}

fn simctl_service(permission: DevicePermission) -> Option<&'static str> {
    Some(match permission {
        DevicePermission::Microphone => "microphone",
        DevicePermission::Photos => "photos",
        DevicePermission::Contacts => "contacts",
        DevicePermission::Calendar => "calendar",
        DevicePermission::Reminders => "reminders",
        DevicePermission::Location => "location",
        DevicePermission::Motion => "motion",
        DevicePermission::MediaLibrary => "media-library",
        DevicePermission::Camera | DevicePermission::FaceId | DevicePermission::Notifications => {
            return None;
        }
    })
}

fn serve_sim_service(permission: DevicePermission) -> &'static str {
    match permission {
        DevicePermission::Camera => "camera",
        DevicePermission::FaceId => "faceid",
        _ => "notifications",
    }
}

fn decision(decision: PermissionDecision) -> &'static str {
    match decision {
        PermissionDecision::Grant => "grant",
        PermissionDecision::Revoke => "revoke",
        PermissionDecision::Reset => "reset",
    }
}

fn validate(action: &DeviceAction) -> Result<(), String> {
    match action {
        DeviceAction::SetLocation { lat, lon } => {
            let ok = lat.is_finite()
                && lon.is_finite()
                && (-90.0..=90.0).contains(lat)
                && (-180.0..=180.0).contains(lon);
            ok.then_some(()).ok_or_else(|| INVALID_LOCATION.into())
        }
        DeviceAction::SetPermission { app_id, .. }
        | DeviceAction::LaunchApp { app_id }
        | DeviceAction::TerminateApp { app_id } => valid_app_id(app_id)
            .then_some(())
            .ok_or_else(|| INVALID_APP_ID.into()),
        DeviceAction::SendPush { app_id, payload } => {
            if !valid_app_id(app_id) {
                return Err(INVALID_APP_ID.into());
            }
            push_payload(payload).map(|_| ())
        }
        DeviceAction::OpenUrl { url } => reqwest::Url::parse(url)
            .map(|_| ())
            .map_err(|_| INVALID_URL.into()),
        _ => Ok(()),
    }
}

pub(crate) fn action_argv(
    platform: DevicePlatform,
    udid: &str,
    action: &DeviceAction,
    helpers: &Helpers,
    push_file: &Path,
) -> Result<Vec<String>, String> {
    if platform != DevicePlatform::Ios || !hub_client::valid_device_id(udid) {
        return Err(DEVICE_ACTION_UNAVAILABLE.into());
    }
    validate(action)?;
    let simctl = |verb: &str, rest: &[&str]| {
        ["xcrun", "simctl", verb, udid]
            .iter()
            .chain(rest)
            .map(|arg| arg.to_string())
            .collect::<Vec<_>>()
    };
    let ax = |rest: &[&str]| -> Result<Vec<String>, String> {
        let helper = helpers
            .ax_settings
            .as_deref()
            .ok_or(DEVICE_ACTION_UNAVAILABLE)?;
        Ok(simctl("spawn", &[&[helper][..], rest].concat()))
    };
    let on_off = |on: bool| if on { "on" } else { "off" };
    Ok(match action {
        DeviceAction::SetAppearance { value } => simctl(
            "ui",
            &[
                "appearance",
                match value {
                    DeviceAppearance::Light => "light",
                    DeviceAppearance::Dark => "dark",
                },
            ],
        ),
        DeviceAction::SetTextSize { value } => {
            simctl("ui", &["content_size", ios_text_size(*value)])
        }
        DeviceAction::SetToggle {
            setting: DeviceToggle::IncreaseContrast,
            enabled,
        } => simctl(
            "ui",
            &[
                "increase_contrast",
                if *enabled { "enabled" } else { "disabled" },
            ],
        ),
        DeviceAction::SetToggle { setting, enabled } => {
            let option = ax_toggle(*setting).ok_or(DEVICE_ACTION_UNAVAILABLE)?;
            ax(&["set", option, on_off(*enabled)])?
        }
        DeviceAction::SetLiquidGlass { value } => ax(&[
            "set",
            "liquid-glass",
            match value {
                LiquidGlass::Clear => "clear",
                LiquidGlass::Tinted => "tinted",
            },
        ])?,
        DeviceAction::SetColorFilter { value } => {
            ax(&["set", "color-filter", color_filter(*value)])?
        }
        DeviceAction::SetOrientation { .. } => return Err(DEVICE_ACTION_UNAVAILABLE.into()),
        DeviceAction::SetLocation { lat, lon } => {
            simctl("location", &["set", &format!("{lat},{lon}")])
        }
        DeviceAction::ClearLocation => simctl("location", &["clear"]),
        DeviceAction::SetPermission {
            app_id,
            permission,
            decision: choice,
        } => match simctl_service(*permission) {
            Some(service) => simctl("privacy", &[decision(*choice), service, app_id]),
            None => {
                let (node, cli) = helpers
                    .serve_sim
                    .as_ref()
                    .ok_or(DEVICE_ACTION_UNAVAILABLE)?;
                [
                    node.as_str(),
                    cli.as_str(),
                    "permissions",
                    decision(*choice),
                    serve_sim_service(*permission),
                    app_id,
                    "-d",
                    udid,
                ]
                .iter()
                .map(|arg| arg.to_string())
                .collect()
            }
        },
        DeviceAction::OpenUrl { url } => simctl("openurl", &[url]),
        DeviceAction::LaunchApp { app_id } => simctl("launch", &[app_id]),
        DeviceAction::TerminateApp { app_id } => simctl("terminate", &[app_id]),
        DeviceAction::SendPush { app_id, .. } => {
            simctl("push", &[app_id, &push_file.to_string_lossy()])
        }
    })
}

pub(crate) fn settings_from(
    appearance: Option<&str>,
    content_size: Option<&str>,
    increase_contrast: Option<&str>,
    ax_status: Option<&str>,
) -> DeviceSettingsSnapshot {
    let status: BTreeMap<String, String> = ax_status
        .and_then(|text| serde_json::from_str(text).ok())
        .unwrap_or_default();
    let flag = |key: &str| match status.get(key).map(String::as_str) {
        Some("on") => Some(true),
        Some("off") => Some(false),
        _ => None,
    };
    DeviceSettingsSnapshot {
        appearance: match appearance {
            Some("light") => Some(DeviceAppearance::Light),
            Some("dark") => Some(DeviceAppearance::Dark),
            _ => None,
        },
        text_size: content_size.and_then(text_size_from_ios),
        increase_contrast: match increase_contrast {
            Some("enabled") => Some(true),
            Some("disabled") => Some(false),
            _ => None,
        },
        reduce_motion: flag("reduce-motion"),
        reduce_transparency: flag("reduce-transparency"),
        show_borders: flag("show-borders"),
        voice_over: flag("voiceover"),
        liquid_glass: match status.get("liquid-glass").map(String::as_str) {
            Some("clear") => Some(LiquidGlass::Clear),
            Some("tinted") => Some(LiquidGlass::Tinted),
            _ => None,
        },
        color_filter: match status.get("color-filter").map(String::as_str) {
            Some("none") => Some(ColorFilter::None),
            Some("grayscale") => Some(ColorFilter::Grayscale),
            Some("red-green") => Some(ColorFilter::RedGreen),
            Some("green-red") => Some(ColorFilter::GreenRed),
            Some("blue-yellow") => Some(ColorFilter::BlueYellow),
            _ => None,
        },
    }
}

pub(crate) fn app_state(value: &serde_json::Value) -> Option<DeviceAppState> {
    let bundle = value.get("bundleId")?;
    let foreground_app = match bundle.as_str() {
        Some(id) if !id.is_empty() => Some(ForegroundApp {
            id: id.to_string(),
            pid: value["pid"]
                .as_u64()
                .and_then(|pid| u32::try_from(pid).ok()),
        }),
        None if !bundle.is_null() => return None,
        _ => None,
    };
    Some(DeviceAppState { foreground_app })
}

fn event_entry(value: &serde_json::Value) -> Option<DeviceEventLogEntry> {
    let id = value["id"].as_u64()?;
    let text = |key: &str| value[key].as_str().unwrap_or_default().to_string();
    let summary = value["summary"]
        .as_str()
        .or_else(|| value["msg"].as_str())
        .unwrap_or_default();
    Some(DeviceEventLogEntry {
        id: id.to_string(),
        timestamp: text("timestamp"),
        kind: text("kind"),
        summary: summary.to_string(),
    })
}

pub(crate) fn event_batch(value: &serde_json::Value) -> Option<DeviceEventLogBatch> {
    if let Some(events) = value["events"].as_array() {
        return Some(DeviceEventLogBatch {
            reset: true,
            events: events.iter().filter_map(event_entry).collect(),
        });
    }
    Some(DeviceEventLogBatch {
        reset: false,
        events: vec![event_entry(&value["event"])?],
    })
}

fn private_dir() -> std::io::Result<tempfile::TempDir> {
    let dir = tempfile::tempdir()?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700))?;
    }
    Ok(dir)
}

fn write_private(path: &Path, text: &str) -> std::io::Result<()> {
    use std::io::Write;
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    options.open(path)?.write_all(text.as_bytes())
}

impl DeviceHub {
    fn action_env(&self) -> (Runner, Helpers) {
        #[cfg(test)]
        if let Some(env) = self
            .test_actions
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
        {
            return env;
        }
        (system_runner(), Helpers::installed())
    }

    #[cfg(test)]
    pub(crate) fn use_actions_for_test(&self, runner: Runner, helpers: Helpers) {
        *self
            .test_actions
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Some((runner, helpers));
    }

    fn device_lock(&self, device_id: &str) -> Arc<tokio::sync::Mutex<()>> {
        self.action_locks
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .entry(device_id.to_string())
            .or_default()
            .clone()
    }

    fn action_target(&self, host_id: Option<&str>, device_id: &str) -> Result<HubDevice, String> {
        if !self.enabled() {
            return Err(DEVICE_SUPPORT_OFF.into());
        }
        if !hub_client::valid_device_id(device_id) {
            return Err(UNKNOWN_DEVICE.into());
        }
        self.snapshot()
            .device(host_id.unwrap_or(LOCAL_DEVICE_HOST_ID), device_id)
            .cloned()
            .ok_or_else(|| UNKNOWN_DEVICE.into())
    }

    fn hub_client_origin(&self) -> Result<String, String> {
        self.live_origin()
            .ok_or_else(|| HUB_NOT_RUNNING.to_string())
    }

    pub async fn device_detail(
        &self,
        params: HubDeviceTargetParams,
    ) -> Result<DeviceDetail, String> {
        let device = self.action_target(params.host_id.as_deref(), &params.device_id)?;
        if device.platform != DevicePlatform::Ios {
            return Err(IOS_ONLY.into());
        }
        let lock = self.device_lock(&device.id);
        let _serial = lock.lock().await;
        Ok(self.read_detail(&device, true).await)
    }

    async fn read_detail(&self, device: &HubDevice, foreground: bool) -> DeviceDetail {
        let (runner, helpers) = self.action_env();
        let read = |argv: Vec<String>| {
            let run = runner(argv);
            async move {
                let output = run.await;
                (output.code == 0).then(|| output.stdout.trim().to_lowercase())
            }
        };
        let simctl_ui = |option: &str| {
            read(
                ["xcrun", "simctl", "ui", &device.id, option]
                    .map(str::to_string)
                    .to_vec(),
            )
        };
        let ax_status = async {
            match &helpers.ax_settings {
                Some(helper) => {
                    let argv = ["xcrun", "simctl", "spawn", &device.id, helper, "status"]
                        .map(str::to_string)
                        .to_vec();
                    read(argv).await
                }
                None => None,
            }
        };
        let (appearance, content_size, contrast, ax_status, foreground_app) = tokio::join!(
            simctl_ui("appearance"),
            simctl_ui("content_size"),
            simctl_ui("increase_contrast"),
            ax_status,
            async {
                if foreground {
                    self.foreground_once(&device.host_id, &device.id).await
                } else {
                    None
                }
            },
        );
        DeviceDetail {
            host_id: device.host_id.clone(),
            device_id: device.id.clone(),
            settings: settings_from(
                appearance.as_deref(),
                content_size.as_deref(),
                contrast.as_deref(),
                ax_status.as_deref(),
            ),
            foreground_app,
            read_at: chrono::Utc::now().to_rfc3339(),
        }
    }

    async fn foreground_once(&self, host_id: &str, device_id: &str) -> Option<ForegroundApp> {
        let mut feed = self.app_state_feed(Some(host_id), device_id).await.ok()?;
        tokio::time::timeout(FOREGROUND_READ, feed.next())
            .await
            .ok()
            .flatten()?
            .foreground_app
    }

    pub async fn run_action(
        &self,
        params: RunHubDeviceActionParams,
    ) -> Result<DeviceDetail, String> {
        let device = self.action_target(params.host_id.as_deref(), &params.device_id)?;
        let (runner, helpers) = self.action_env();
        let dir = private_dir().map_err(|e| e.to_string())?;
        let push_file = dir.path().join("payload.json");
        let argv = action_argv(
            device.platform,
            &device.id,
            &params.action,
            &helpers,
            &push_file,
        )?;
        if let DeviceAction::SendPush { payload, .. } = &params.action {
            write_private(&push_file, &push_payload(payload)?).map_err(|e| e.to_string())?;
        }
        let lock = self.device_lock(&device.id);
        let _serial = lock.lock().await;
        let output = runner(argv).await;
        if output.code != 0 {
            let detail = output.stderr.trim();
            let detail = if detail.is_empty() {
                format!("exit code {}", output.code)
            } else {
                detail.chars().take(400).collect()
            };
            return Err(format!("The device action failed: {detail}"));
        }
        Ok(self.read_detail(&device, false).await)
    }

    pub async fn device_screenshot(
        &self,
        params: HubDeviceTargetParams,
    ) -> Result<HubDeviceScreenshot, String> {
        let device = self.action_target(params.host_id.as_deref(), &params.device_id)?;
        if device.platform != DevicePlatform::Ios {
            return Err(IOS_ONLY.into());
        }
        let png = super::agent::capture_simulator(&device.id).await?;
        let (width, height) = super::agent::png_dimensions(&png);
        Ok(HubDeviceScreenshot {
            png: base64::engine::general_purpose::STANDARD.encode(&png),
            width,
            height,
        })
    }

    pub async fn device_ax_tree(
        &self,
        params: HubDeviceTargetParams,
    ) -> Result<DeviceAxTree, String> {
        let device = self.action_target(params.host_id.as_deref(), &params.device_id)?;
        if device.platform != DevicePlatform::Ios {
            return Err(IOS_ONLY.into());
        }
        let origin = self.hub_client_origin()?;
        let nodes = HubClient {
            http: &self.http,
            origin: &origin,
        }
        .ax_tree(&device.id)
        .await?;
        Ok(DeviceAxTree { nodes })
    }

    pub async fn app_state_feed(
        &self,
        host_id: Option<&str>,
        device_id: &str,
    ) -> Result<futures::stream::BoxStream<'static, DeviceAppState>, String> {
        let feed = self.ios_feed(host_id, device_id, true).await?;
        Ok(feed
            .filter_map(|value| async move { app_state(&value) })
            .boxed())
    }

    pub async fn event_log_feed(
        &self,
        host_id: Option<&str>,
        device_id: &str,
    ) -> Result<futures::stream::BoxStream<'static, DeviceEventLogBatch>, String> {
        let feed = self.ios_feed(host_id, device_id, false).await?;
        Ok(feed
            .filter_map(|value| async move { event_batch(&value) })
            .boxed())
    }

    async fn ios_feed(
        &self,
        host_id: Option<&str>,
        device_id: &str,
        app_state: bool,
    ) -> Result<SseStream, String> {
        let device = self.action_target(host_id, device_id)?;
        if device.platform != DevicePlatform::Ios {
            return Err(IOS_ONLY.into());
        }
        let origin = self.hub_client_origin()?;
        let client = HubClient {
            http: &self.http,
            origin: &origin,
        };
        if app_state {
            client.app_state(&device.id).await
        } else {
            client.event_log(&device.id).await
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use zeron_proto::device_hub::DeviceOrientation;

    const UDID: &str = "9BDC3515-D57F-4598-A96A-C53CE59E971B";

    fn helpers() -> Helpers {
        Helpers {
            ax_settings: Some("/hub/simax/serve-sim-ax-settings".into()),
            serve_sim: Some(("/bin/node".into(), "/hub/serve-sim.js".into())),
        }
    }

    fn argv(action: DeviceAction) -> Result<Vec<String>, String> {
        action_argv(
            DevicePlatform::Ios,
            UDID,
            &action,
            &helpers(),
            Path::new("/tmp/x/payload.json"),
        )
    }

    fn words(line: &str) -> Vec<String> {
        line.replace("<udid>", UDID)
            .split(' ')
            .map(str::to_string)
            .collect()
    }

    #[test]
    fn every_ios_action_maps_to_its_simctl_or_helper_argv() {
        let toggle = |setting, enabled| DeviceAction::SetToggle { setting, enabled };
        let permission = |permission, decision| DeviceAction::SetPermission {
            app_id: "com.apple.Maps".into(),
            permission,
            decision,
        };
        let ax = "xcrun simctl spawn <udid> /hub/simax/serve-sim-ax-settings set";
        let cases: Vec<(DeviceAction, String)> = vec![
            (
                DeviceAction::SetAppearance {
                    value: DeviceAppearance::Dark,
                },
                "xcrun simctl ui <udid> appearance dark".into(),
            ),
            (
                DeviceAction::SetAppearance {
                    value: DeviceAppearance::Light,
                },
                "xcrun simctl ui <udid> appearance light".into(),
            ),
            (
                DeviceAction::SetTextSize {
                    value: DeviceTextSize::Small,
                },
                "xcrun simctl ui <udid> content_size small".into(),
            ),
            (
                DeviceAction::SetTextSize {
                    value: DeviceTextSize::Default,
                },
                "xcrun simctl ui <udid> content_size large".into(),
            ),
            (
                DeviceAction::SetTextSize {
                    value: DeviceTextSize::Large,
                },
                "xcrun simctl ui <udid> content_size extra-extra-large".into(),
            ),
            (
                DeviceAction::SetTextSize {
                    value: DeviceTextSize::ExtraLarge,
                },
                "xcrun simctl ui <udid> content_size accessibility-large".into(),
            ),
            (
                toggle(DeviceToggle::IncreaseContrast, true),
                "xcrun simctl ui <udid> increase_contrast enabled".into(),
            ),
            (
                toggle(DeviceToggle::IncreaseContrast, false),
                "xcrun simctl ui <udid> increase_contrast disabled".into(),
            ),
            (
                toggle(DeviceToggle::ReduceMotion, true),
                format!("{ax} reduce-motion on"),
            ),
            (
                toggle(DeviceToggle::ReduceTransparency, false),
                format!("{ax} reduce-transparency off"),
            ),
            (
                toggle(DeviceToggle::ShowBorders, true),
                format!("{ax} show-borders on"),
            ),
            (
                toggle(DeviceToggle::VoiceOver, true),
                format!("{ax} voiceover on"),
            ),
            (
                DeviceAction::SetLiquidGlass {
                    value: LiquidGlass::Tinted,
                },
                format!("{ax} liquid-glass tinted"),
            ),
            (
                DeviceAction::SetColorFilter {
                    value: ColorFilter::RedGreen,
                },
                format!("{ax} color-filter red-green"),
            ),
            (
                DeviceAction::SetColorFilter {
                    value: ColorFilter::None,
                },
                format!("{ax} color-filter none"),
            ),
            (
                DeviceAction::SetLocation {
                    lat: 35.6762,
                    lon: 139.6503,
                },
                "xcrun simctl location <udid> set 35.6762,139.6503".into(),
            ),
            (
                DeviceAction::SetLocation {
                    lat: 37.7749,
                    lon: -122.4194,
                },
                "xcrun simctl location <udid> set 37.7749,-122.4194".into(),
            ),
            (
                DeviceAction::ClearLocation,
                "xcrun simctl location <udid> clear".into(),
            ),
            (
                permission(DevicePermission::Microphone, PermissionDecision::Grant),
                "xcrun simctl privacy <udid> grant microphone com.apple.Maps".into(),
            ),
            (
                permission(DevicePermission::Location, PermissionDecision::Revoke),
                "xcrun simctl privacy <udid> revoke location com.apple.Maps".into(),
            ),
            (
                permission(DevicePermission::MediaLibrary, PermissionDecision::Reset),
                "xcrun simctl privacy <udid> reset media-library com.apple.Maps".into(),
            ),
            (
                permission(DevicePermission::Photos, PermissionDecision::Grant),
                "xcrun simctl privacy <udid> grant photos com.apple.Maps".into(),
            ),
            (
                permission(DevicePermission::Contacts, PermissionDecision::Grant),
                "xcrun simctl privacy <udid> grant contacts com.apple.Maps".into(),
            ),
            (
                permission(DevicePermission::Calendar, PermissionDecision::Grant),
                "xcrun simctl privacy <udid> grant calendar com.apple.Maps".into(),
            ),
            (
                permission(DevicePermission::Reminders, PermissionDecision::Grant),
                "xcrun simctl privacy <udid> grant reminders com.apple.Maps".into(),
            ),
            (
                permission(DevicePermission::Motion, PermissionDecision::Grant),
                "xcrun simctl privacy <udid> grant motion com.apple.Maps".into(),
            ),
            (
                permission(DevicePermission::Camera, PermissionDecision::Grant),
                "/bin/node /hub/serve-sim.js permissions grant camera com.apple.Maps -d <udid>"
                    .into(),
            ),
            (
                permission(DevicePermission::FaceId, PermissionDecision::Revoke),
                "/bin/node /hub/serve-sim.js permissions revoke faceid com.apple.Maps -d <udid>"
                    .into(),
            ),
            (
                permission(DevicePermission::Notifications, PermissionDecision::Reset),
                "/bin/node /hub/serve-sim.js permissions reset notifications com.apple.Maps -d <udid>"
                    .into(),
            ),
            (
                DeviceAction::OpenUrl {
                    url: "https://example.com/a?b=c".into(),
                },
                "xcrun simctl openurl <udid> https://example.com/a?b=c".into(),
            ),
            (
                DeviceAction::OpenUrl {
                    url: "myapp://route".into(),
                },
                "xcrun simctl openurl <udid> myapp://route".into(),
            ),
            (
                DeviceAction::LaunchApp {
                    app_id: "com.apple.Preferences".into(),
                },
                "xcrun simctl launch <udid> com.apple.Preferences".into(),
            ),
            (
                DeviceAction::TerminateApp {
                    app_id: "com.apple.Preferences".into(),
                },
                "xcrun simctl terminate <udid> com.apple.Preferences".into(),
            ),
            (
                DeviceAction::SendPush {
                    app_id: "com.example.app".into(),
                    payload: json!("Hello"),
                },
                "xcrun simctl push <udid> com.example.app /tmp/x/payload.json".into(),
            ),
        ];
        for (action, expected) in cases {
            assert_eq!(argv(action.clone()), Ok(words(&expected)), "{action:?}");
        }
    }

    #[test]
    fn invalid_inputs_are_rejected_before_any_command() {
        let location = |lat, lon| argv(DeviceAction::SetLocation { lat, lon });
        for (lat, lon) in [
            (90.1, 0.0),
            (-90.1, 0.0),
            (0.0, 180.5),
            (0.0, -181.0),
            (f64::NAN, 0.0),
            (0.0, f64::INFINITY),
        ] {
            assert_eq!(
                location(lat, lon),
                Err(INVALID_LOCATION.into()),
                "{lat},{lon}"
            );
        }
        assert!(location(-90.0, 180.0).is_ok());
        for app_id in [
            "",
            "-w",
            "--console",
            "com.app id",
            "com/app",
            "a;b",
            "$(x)",
            "é.app",
        ] {
            for action in [
                DeviceAction::LaunchApp {
                    app_id: app_id.into(),
                },
                DeviceAction::TerminateApp {
                    app_id: app_id.into(),
                },
                DeviceAction::SetPermission {
                    app_id: app_id.into(),
                    permission: DevicePermission::Camera,
                    decision: PermissionDecision::Grant,
                },
                DeviceAction::SendPush {
                    app_id: app_id.into(),
                    payload: json!("hi"),
                },
            ] {
                assert_eq!(argv(action), Err(INVALID_APP_ID.into()), "{app_id:?}");
            }
        }
        for url in ["", "not a url", "example.com", "-x"] {
            assert_eq!(
                argv(DeviceAction::OpenUrl { url: url.into() }),
                Err(INVALID_URL.into()),
                "{url:?}"
            );
        }
        for payload in [json!(""), json!("  "), json!(42), json!(["a"]), json!(null)] {
            assert_eq!(
                argv(DeviceAction::SendPush {
                    app_id: "a.b".into(),
                    payload
                }),
                Err(INVALID_PUSH.into())
            );
        }
        assert_eq!(
            push_payload(&json!("Hello")).unwrap(),
            r#"{"aps":{"alert":"Hello"}}"#
        );
        assert_eq!(
            push_payload(&json!({"aps": {"badge": 3}})).unwrap(),
            r#"{"aps":{"badge":3}}"#
        );
        assert!(
            action_argv(
                DevicePlatform::Ios,
                "../SIM",
                &DeviceAction::ClearLocation,
                &helpers(),
                Path::new("/tmp/p.json")
            )
            .is_err()
        );
    }

    #[test]
    fn unsupported_actions_are_device_action_unavailable() {
        let unavailable = Err(DEVICE_ACTION_UNAVAILABLE.to_string());
        assert_eq!(
            argv(DeviceAction::SetOrientation {
                value: DeviceOrientation::LandscapeLeft
            }),
            unavailable
        );
        assert_eq!(
            argv(DeviceAction::SetToggle {
                setting: DeviceToggle::NetworkEnabled,
                enabled: true
            }),
            unavailable
        );
        let bare = Helpers::default();
        let without_helpers = |action: DeviceAction| {
            action_argv(
                DevicePlatform::Ios,
                UDID,
                &action,
                &bare,
                Path::new("/tmp/p.json"),
            )
        };
        assert_eq!(
            without_helpers(DeviceAction::SetToggle {
                setting: DeviceToggle::VoiceOver,
                enabled: true
            }),
            unavailable
        );
        assert_eq!(
            without_helpers(DeviceAction::SetColorFilter {
                value: ColorFilter::Grayscale
            }),
            unavailable
        );
        assert_eq!(
            without_helpers(DeviceAction::SetPermission {
                app_id: "a.b".into(),
                permission: DevicePermission::Notifications,
                decision: PermissionDecision::Grant,
            }),
            unavailable
        );
        assert!(
            without_helpers(DeviceAction::SetToggle {
                setting: DeviceToggle::IncreaseContrast,
                enabled: true
            })
            .is_ok(),
            "simctl-only actions need no helper"
        );
        for action in [
            DeviceAction::SetAppearance {
                value: DeviceAppearance::Dark,
            },
            DeviceAction::ClearLocation,
            DeviceAction::LaunchApp {
                app_id: "com.android.settings".into(),
            },
        ] {
            assert_eq!(
                action_argv(
                    DevicePlatform::Android,
                    "emulator-5554",
                    &action,
                    &helpers(),
                    Path::new("/tmp/p.json")
                ),
                unavailable
            );
        }
    }

    #[test]
    fn detail_parses_real_simctl_and_helper_output() {
        let settings = settings_from(
            Some("light"),
            Some("large"),
            Some("disabled"),
            Some(
                r#"{"reduce-motion":"off","show-borders":"off","reduce-transparency":"off","voiceover":"off","color-filter":"none","liquid-glass":"clear"}"#,
            ),
        );
        assert_eq!(
            settings,
            DeviceSettingsSnapshot {
                appearance: Some(DeviceAppearance::Light),
                text_size: Some(DeviceTextSize::Default),
                reduce_motion: Some(false),
                increase_contrast: Some(false),
                reduce_transparency: Some(false),
                show_borders: Some(false),
                voice_over: Some(false),
                liquid_glass: Some(LiquidGlass::Clear),
                color_filter: Some(ColorFilter::None),
            }
        );
        let dark = settings_from(
            Some("dark"),
            Some("accessibility-extra-large"),
            Some("enabled"),
            Some(
                r#"{"reduce-motion":"on","voiceover":"on","color-filter":"blue-yellow","liquid-glass":"tinted"}"#,
            ),
        );
        assert_eq!(dark.appearance, Some(DeviceAppearance::Dark));
        assert_eq!(dark.text_size, Some(DeviceTextSize::ExtraLarge));
        assert_eq!(
            (dark.increase_contrast, dark.reduce_motion, dark.voice_over),
            (Some(true), Some(true), Some(true))
        );
        assert_eq!(dark.show_borders, None);
        assert_eq!(dark.color_filter, Some(ColorFilter::BlueYellow));
        assert_eq!(dark.liquid_glass, Some(LiquidGlass::Tinted));
        let unknown = settings_from(Some("unsupported"), Some("unknown"), None, Some("not json"));
        assert_eq!(unknown, DeviceSettingsSnapshot::default());
        for (category, size) in [
            ("extra-small", DeviceTextSize::Small),
            ("small", DeviceTextSize::Small),
            ("medium", DeviceTextSize::Small),
            ("large", DeviceTextSize::Default),
            ("extra-large", DeviceTextSize::Large),
            ("extra-extra-large", DeviceTextSize::Large),
            ("extra-extra-extra-large", DeviceTextSize::Large),
            ("accessibility-medium", DeviceTextSize::ExtraLarge),
            ("accessibility-large", DeviceTextSize::ExtraLarge),
        ] {
            assert_eq!(text_size_from_ios(category), Some(size), "{category}");
        }
    }

    #[test]
    fn helpers_resolve_from_the_installed_hub_entry() {
        let dir = tempfile::tempdir().unwrap();
        let pkg = dir.path().join("node_modules/expo-device-hub");
        let entry = pkg.join("dist/server/cli.mjs");
        std::fs::create_dir_all(entry.parent().unwrap()).unwrap();
        std::fs::write(&entry, "").unwrap();
        assert!(
            Helpers::from_entry(&entry, Some("/bin/node".into()))
                .ax_settings
                .is_none()
        );
        let dist = pkg.join("vendor/serve-sim/dist");
        std::fs::create_dir_all(dist.join("simax")).unwrap();
        std::fs::write(dist.join("simax/serve-sim-ax-settings"), "").unwrap();
        std::fs::write(dist.join("serve-sim.js"), "").unwrap();
        let helpers = Helpers::from_entry(&entry, Some("/bin/node".into()));
        assert_eq!(
            helpers.ax_settings,
            Some(
                dist.join("simax/serve-sim-ax-settings")
                    .to_string_lossy()
                    .into_owned()
            )
        );
        assert_eq!(
            helpers.serve_sim,
            Some((
                "/bin/node".into(),
                dist.join("serve-sim.js").to_string_lossy().into_owned()
            ))
        );
        assert!(Helpers::from_entry(&entry, None).serve_sim.is_none());
    }

    #[test]
    fn hub_feeds_parse_appstate_and_event_log_payloads() {
        assert_eq!(
            app_state(&json!({"bundleId": "com.apple.Maps", "pid": 4321, "isReactNative": false})),
            Some(DeviceAppState {
                foreground_app: Some(ForegroundApp {
                    id: "com.apple.Maps".into(),
                    pid: Some(4321)
                })
            })
        );
        assert_eq!(
            app_state(&json!({"bundleId": null})),
            Some(DeviceAppState::default())
        );
        assert_eq!(
            app_state(&json!({"bundleId": ""})),
            Some(DeviceAppState::default())
        );
        assert_eq!(app_state(&json!({"other": 1})), None);
        let seed = event_batch(&json!({"events": [
            {"id": 7, "timestamp": "2026-10-05T10:00:00.000Z", "kind": "touch", "summary": "Touch begin 0.50,0.50", "msg": "x"},
            {"id": 8, "timestamp": "2026-10-05T10:00:01.000Z", "kind": "button", "msg": "Button home"},
            {"kind": "no id"}
        ]}))
        .unwrap();
        assert!(seed.reset);
        assert_eq!(
            seed.events
                .iter()
                .map(|e| (e.id.as_str(), e.summary.as_str()))
                .collect::<Vec<_>>(),
            vec![("7", "Touch begin 0.50,0.50"), ("8", "Button home")]
        );
        let live =
            event_batch(&json!({"event": {"id": 9, "kind": "key", "summary": "Key down 4"}}))
                .unwrap();
        assert!(!live.reset);
        assert_eq!(live.events[0].kind, "key");
        assert_eq!(event_batch(&json!({"event": {}})), None);
    }
}

#[cfg(all(test, target_os = "macos"))]
mod hub_tests {
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    use serde_json::json;
    use zeron_proto::device_hub::{
        DeviceAction, DeviceAppearance, HubDevice, HubDeviceTargetParams, RunHubDeviceActionParams,
    };

    use super::super::tests::{HEALTHY, enabled_hub, fake_hub};
    use super::*;
    use zeron_rpc::{RpcReply, RpcService, methods};

    type Log = Arc<Mutex<Vec<String>>>;

    fn recording_runner(log: Log, appearance: &'static str) -> Runner {
        Arc::new(move |argv: Vec<String>| {
            let log = log.clone();
            Box::pin(async move {
                let udid = argv.get(3).cloned().unwrap_or_default();
                let read = argv.len() == 5 || argv.last().map(String::as_str) == Some("status");
                if read {
                    log.lock().unwrap().push(format!("read {udid} {}", argv[4]));
                } else {
                    log.lock().unwrap().push(format!("start {udid}"));
                    tokio::time::sleep(Duration::from_millis(150)).await;
                    log.lock().unwrap().push(format!("end {udid}"));
                }
                let stdout = match argv.get(4).map(String::as_str) {
                    Some("appearance") => format!("{appearance}\n"),
                    Some("content_size") => "large\n".into(),
                    Some("increase_contrast") => "disabled\n".into(),
                    _ => r#"{"voiceover":"on"}"#.into(),
                };
                CommandOutput {
                    code: 0,
                    stdout,
                    stderr: String::new(),
                }
            })
        })
    }

    fn dark(device_id: &str) -> RunHubDeviceActionParams {
        RunHubDeviceActionParams {
            host_id: None,
            device_id: device_id.into(),
            action: DeviceAction::SetAppearance {
                value: DeviceAppearance::Dark,
            },
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn actions_are_serialized_per_device_and_return_the_refreshed_detail() {
        let (_dir, hub, _) = enabled_hub(HEALTHY).await;
        hub.publish(|state| {
            state.devices.push(HubDevice {
                host_id: "local".into(),
                id: "SIM-2".into(),
                name: "iPad".into(),
                ..Default::default()
            })
        });
        let log: Log = Arc::default();
        hub.use_actions_for_test(
            recording_runner(log.clone(), "dark"),
            Helpers {
                ax_settings: Some("/hub/ax".into()),
                serve_sim: None,
            },
        );
        let (a, b) = tokio::join!(hub.run_action(dark("SIM-1")), hub.run_action(dark("SIM-1")));
        let detail = a.unwrap();
        assert_eq!(detail.settings.appearance, Some(DeviceAppearance::Dark));
        assert_eq!(detail.settings.voice_over, Some(true));
        assert_eq!(
            (detail.host_id.as_str(), detail.device_id.as_str()),
            ("local", "SIM-1")
        );
        assert!(b.is_ok());
        let same: Vec<String> = log.lock().unwrap().clone();
        let first_end = same.iter().position(|line| line == "end SIM-1").unwrap();
        let second_start = same.iter().rposition(|line| line == "start SIM-1").unwrap();
        let reads_between = same[first_end..second_start]
            .iter()
            .filter(|line| line.starts_with("read SIM-1"))
            .count();
        assert!(
            first_end < second_start && reads_between == 4,
            "the second action waits for the first and its detail read: {same:?}"
        );

        log.lock().unwrap().clear();
        let (a, b) = tokio::join!(hub.run_action(dark("SIM-1")), hub.run_action(dark("SIM-2")));
        assert!(a.is_ok() && b.is_ok());
        let both = log.lock().unwrap().clone();
        let starts: Vec<usize> = ["start SIM-1", "start SIM-2"]
            .iter()
            .map(|line| both.iter().position(|seen| seen == line).unwrap())
            .collect();
        let first_end = both
            .iter()
            .position(|line| line.starts_with("end "))
            .unwrap();
        assert!(
            starts.iter().all(|start| *start < first_end),
            "different devices run side by side: {both:?}"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn unknown_devices_failures_and_disabled_hubs_are_refused() {
        let (_dir, hub, _) = enabled_hub(HEALTHY).await;
        let failing: Runner = Arc::new(|_| {
            Box::pin(async {
                CommandOutput {
                    code: 2,
                    stdout: String::new(),
                    stderr: "Invalid device state\n".into(),
                }
            })
        });
        hub.use_actions_for_test(failing, Helpers::default());
        assert_eq!(
            hub.run_action(dark("SIM-1")).await,
            Err("The device action failed: Invalid device state".into())
        );
        assert_eq!(
            hub.run_action(dark("SIM-9")).await,
            Err(UNKNOWN_DEVICE.into())
        );
        assert_eq!(
            hub.run_action(dark("../SIM-1")).await,
            Err(UNKNOWN_DEVICE.into())
        );
        let detail = hub
            .device_detail(HubDeviceTargetParams {
                host_id: None,
                device_id: "SIM-1".into(),
            })
            .await
            .unwrap();
        assert_eq!(
            detail.settings,
            DeviceSettingsSnapshot::default(),
            "failed reads are unknown"
        );
        hub.configure(zeron_proto::device_hub::ConfigureDeviceHubParams {
            enabled: Some(false),
            ..Default::default()
        })
        .await
        .unwrap();
        assert_eq!(
            hub.run_action(dark("SIM-1")).await,
            Err(DEVICE_SUPPORT_OFF.into())
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_push_payload_reaches_simctl_through_a_private_file() {
        let (_dir, hub, _) = enabled_hub(HEALTHY).await;
        type Seen = Option<(Vec<String>, String, (u32, u32))>;
        let seen: Arc<Mutex<Seen>> = Arc::default();
        let record = seen.clone();
        let runner: Runner = Arc::new(move |argv: Vec<String>| {
            if argv.get(2).map(String::as_str) == Some("push") {
                use std::os::unix::fs::PermissionsExt;
                let file = std::path::Path::new(&argv[5]);
                let mode = |path: &std::path::Path| {
                    std::fs::metadata(path).unwrap().permissions().mode() & 0o777
                };
                let mode = (mode(file.parent().unwrap()), mode(file));
                *record.lock().unwrap() =
                    Some((argv.clone(), std::fs::read_to_string(file).unwrap(), mode));
            }
            Box::pin(async {
                CommandOutput {
                    code: 0,
                    stdout: String::new(),
                    stderr: String::new(),
                }
            })
        });
        hub.use_actions_for_test(runner, Helpers::default());
        hub.run_action(RunHubDeviceActionParams {
            host_id: None,
            device_id: "SIM-1".into(),
            action: DeviceAction::SendPush {
                app_id: "com.example.app".into(),
                payload: json!("Build finished"),
            },
        })
        .await
        .unwrap();
        let (argv, body, mode) = seen.lock().unwrap().clone().expect("push ran");
        assert_eq!(
            argv[..5],
            ["xcrun", "simctl", "push", "SIM-1", "com.example.app"]
        );
        assert_eq!(body, r#"{"aps":{"alert":"Build finished"}}"#);
        assert_eq!(mode, (0o700, 0o600));
        assert!(
            !std::path::Path::new(&argv[5]).exists(),
            "the payload is removed"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn the_hub_reads_go_through_the_allowlisted_routes_only() {
        use futures::StreamExt;
        let dir = tempfile::tempdir().unwrap();
        let core = crate::EngineCore::assemble(
            dir.path(),
            Arc::new(crate::HarnessRegistry::new()),
            zeron_proto::HarnessId::Mock,
            None,
        )
        .unwrap();
        let (origin, requests) = fake_hub().await;
        core.device_hub.attach_hub_for_test(origin);
        core.device_hub.use_actions_for_test(
            recording_runner(Arc::default(), "light"),
            Helpers::default(),
        );
        let rpc = core.rpc_service();
        rpc.handle(methods::CONFIGURE_DEVICE_HUB, json!({"enabled": true}))
            .await
            .unwrap();
        let target = json!({"deviceId": "SIM-1"});
        let Ok(RpcReply::Value(tree)) = rpc
            .handle(methods::GET_HUB_DEVICE_AX_TREE, target.clone())
            .await
        else {
            panic!("the AX tree answers");
        };
        assert_eq!(tree["nodes"][0]["label"], "SpringBoard");
        assert_eq!(tree["nodes"][0]["children"][0]["frame"]["x"], 20.0);
        assert_eq!(tree["nodes"][0]["children"][0]["id"], "Maps");

        let Ok(RpcReply::Value(detail)) = rpc
            .handle(methods::GET_HUB_DEVICE_DETAIL, target.clone())
            .await
        else {
            panic!("the detail answers");
        };
        assert_eq!(detail["settings"]["appearance"], "light");
        assert_eq!(detail["settings"]["textSize"], "default");
        assert_eq!(
            detail["foregroundApp"],
            json!({"id": "com.apple.Maps", "pid": 7})
        );

        let next = |reply: Result<RpcReply, zeron_rpc::RpcError>| {
            let Ok(RpcReply::Stream(stream)) = reply else {
                panic!("a feed streams");
            };
            stream
        };
        let mut app = next(
            rpc.handle(methods::WATCH_HUB_DEVICE_APP_STATE, target.clone())
                .await,
        );
        let first = tokio::time::timeout(Duration::from_secs(5), app.next())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            first,
            json!({"foregroundApp": {"id": "com.apple.Maps", "pid": 7}})
        );
        let mut events = next(
            rpc.handle(methods::WATCH_HUB_DEVICE_EVENTS, target.clone())
                .await,
        );
        let mut batches = Vec::new();
        for _ in 0..2 {
            batches.push(
                tokio::time::timeout(Duration::from_secs(5), events.next())
                    .await
                    .unwrap()
                    .unwrap(),
            );
        }
        assert_eq!(
            batches,
            vec![
                json!({"reset": true, "events": [{"id": "1", "timestamp": "t1", "kind": "touch", "summary": "Touch begin"}]}),
                json!({"reset": false, "events": [{"id": "2", "timestamp": "t2", "kind": "button", "summary": "Button home"}]}),
            ]
        );
        for method in [
            methods::GET_HUB_DEVICE_AX_TREE,
            methods::WATCH_HUB_DEVICE_APP_STATE,
            methods::RUN_HUB_DEVICE_ACTION,
        ] {
            let params = if method == methods::RUN_HUB_DEVICE_ACTION {
                json!({"deviceId": "SIM-404", "action": "clearLocation"})
            } else {
                json!({"deviceId": "SIM-404"})
            };
            assert!(
                matches!(rpc.handle(method, params).await, Err(zeron_rpc::RpcError::Failed(m)) if m == UNKNOWN_DEVICE),
                "{method}"
            );
        }
        let requests = requests.lock().unwrap().clone();
        let reads: Vec<&String> = requests
            .iter()
            .filter(|line| !line.starts_with("GET /api/devices "))
            .collect();
        assert!(
            reads.iter().all(|line| {
                line.starts_with("GET /vendor/serve-sim/helper/SIM-1/ax ")
                    || line.starts_with("GET /vendor/serve-sim/appstate?device=SIM-1 ")
                    || line.starts_with(
                        "GET /vendor/serve-sim/api/event-log/events?device=SIM-1&limit=100 ",
                    )
            }),
            "{reads:?}"
        );
        core.shutdown().await;
    }
}
