use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, PoisonError};
use std::time::Duration;

use futures::StreamExt;
use futures::future::BoxFuture;
use zeron_proto::device_hub::{
    ColorFilter, DeviceAction, DeviceAppState, DeviceAppearance, DeviceAxTree, DeviceDetail,
    DeviceEventLogBatch, DeviceEventLogEntry, DeviceFoldState, DeviceOrientation, DevicePermission,
    DevicePlatform, DeviceSettingsSnapshot, DeviceTextSize, DeviceToggle, FoldPosture,
    ForegroundApp, HubDevice, HubDeviceFoldParams, HubDeviceScreenshot, HubDeviceTargetParams,
    LOCAL_DEVICE_HOST_ID, LiquidGlass, PermissionDecision, RunHubDeviceActionParams,
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
const REFUSED_URL_SCHEME: &str =
    "file, data, javascript and about URLs cannot be opened on the device.";
const INVALID_PUSH: &str = "A push notification needs alert text or a JSON object payload.";
const PUSH_TOO_LARGE: &str = "A push notification payload must be at most 4096 bytes.";
const PUSH_LIMIT: usize = 4096;
pub(crate) const ANDROID_SDK_MISSING: &str = "The Android SDK is not available on this machine.";
const EVENT_SUMMARY_LIMIT: usize = 1000;
const EVENT_FIELD_LIMIT: usize = 64;
const SCREENSHOT_NAME_TRIES: usize = 100;
pub(crate) const ACTION_TIMEOUT: Duration = Duration::from_secs(20);
const OUTPUT_LIMIT: usize = 4 << 20;
const FOREGROUND_READ: Duration = Duration::from_secs(1);
pub(crate) const PORTRAIT_LOCKED: &str = "The foreground app is locked to portrait.";
const ROTATION_SETTLE: Duration = Duration::from_secs(2);
const ROTATION_POLL: Duration = Duration::from_millis(250);

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

async fn read_pipe(
    pipe: Option<impl tokio::io::AsyncRead + Unpin>,
    limit: usize,
) -> std::io::Result<Vec<u8>> {
    use tokio::io::AsyncReadExt as _;
    let mut bytes = Vec::new();
    if let Some(pipe) = pipe {
        pipe.take(limit as u64 + 1).read_to_end(&mut bytes).await?;
    }
    if bytes.len() > limit {
        return Err(std::io::Error::other(format!(
            "the command printed more than {limit} bytes"
        )));
    }
    Ok(bytes)
}

pub(crate) async fn capped_output(
    mut command: tokio::process::Command,
    limit: usize,
) -> std::io::Result<std::process::Output> {
    use std::process::Stdio;
    super::hub_process::scrub_adb_env(&mut command, std::env::vars_os().map(|(name, _)| name));
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()?;
    let (stdout, stderr) = tokio::try_join!(
        read_pipe(child.stdout.take(), limit),
        read_pipe(child.stderr.take(), limit)
    )?;
    Ok(std::process::Output {
        status: child.wait().await?,
        stdout,
        stderr,
    })
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
    let mut command = tokio::process::Command::new(program);
    command.args(args);
    let run = capped_output(command, OUTPUT_LIMIT);
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
    pub android_sdk: Option<PathBuf>,
}

impl Helpers {
    fn installed() -> Self {
        let android_sdk = super::platform::android_root();
        let Some(entry) = toolchain::installed_hub() else {
            return Self {
                android_sdk,
                ..Self::default()
            };
        };
        let node = toolchain::launch(&entry).ok().map(|(node, _)| node);
        Self {
            android_sdk,
            ..Self::from_entry(&entry, node)
        }
    }

    pub(crate) fn adb(&self) -> Option<String> {
        let root = self.android_sdk.as_deref()?;
        Some(super::platform::adb(root).to_string_lossy().into_owned())
    }

    fn from_entry(entry: &Path, node: Option<PathBuf>) -> Self {
        let Some(dist) = entry
            .ancestors()
            .find(|dir| dir.file_name() == Some("expo-device-hub".as_ref()))
            .map(|package| package.join("vendor/serve-sim/dist"))
            .filter(|dist| dist.is_dir())
        else {
            return Self::default();
        };
        let file = |path: PathBuf| path.is_file().then(|| path.to_string_lossy().into_owned());
        let node = node.map(|node| node.to_string_lossy().into_owned());
        Self {
            ax_settings: file(dist.join("simax/serve-sim-ax-settings")),
            serve_sim: node.zip(file(dist.join("serve-sim.js"))),
            android_sdk: None,
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
    let payload = payload.to_string();
    if payload.len() > PUSH_LIMIT {
        return Err(PUSH_TOO_LARGE.into());
    }
    Ok(payload)
}

fn open_url(url: &str) -> Result<String, String> {
    let parsed = reqwest::Url::parse(url).map_err(|_| INVALID_URL.to_string())?;
    if matches!(parsed.scheme(), "file" | "data" | "javascript" | "about") {
        return Err(REFUSED_URL_SCHEME.into());
    }
    Ok(parsed.into())
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
        _ => Ok(()),
    }
}

pub(crate) fn action_argv(
    platform: DevicePlatform,
    udid: &str,
    action: &DeviceAction,
    helpers: &Helpers,
    push_file: Option<&Path>,
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
        DeviceAction::OpenUrl { url } => simctl("openurl", &[&open_url(url)?]),
        DeviceAction::LaunchApp { app_id } => simctl("launch", &[app_id]),
        DeviceAction::TerminateApp { app_id } => simctl("terminate", &[app_id]),
        DeviceAction::SendPush { app_id, .. } => {
            let push_file = push_file.ok_or(DEVICE_ACTION_UNAVAILABLE)?;
            simctl("push", &[app_id, &push_file.to_string_lossy()])
        }
    })
}

fn android_text_size(size: DeviceTextSize) -> &'static str {
    match size {
        DeviceTextSize::Small => "0.85",
        DeviceTextSize::Default => "1.0",
        DeviceTextSize::Large => "1.15",
        DeviceTextSize::ExtraLarge => "1.3",
    }
}

fn android_permissions(permission: DevicePermission) -> Option<&'static [&'static str]> {
    Some(match permission {
        DevicePermission::Camera => &["android.permission.CAMERA"],
        DevicePermission::Microphone => &["android.permission.RECORD_AUDIO"],
        DevicePermission::Photos => &[
            "android.permission.READ_MEDIA_IMAGES",
            "android.permission.READ_EXTERNAL_STORAGE",
        ],
        DevicePermission::Contacts => &[
            "android.permission.READ_CONTACTS",
            "android.permission.WRITE_CONTACTS",
        ],
        DevicePermission::Calendar => &[
            "android.permission.READ_CALENDAR",
            "android.permission.WRITE_CALENDAR",
        ],
        DevicePermission::Location => &[
            "android.permission.ACCESS_FINE_LOCATION",
            "android.permission.ACCESS_COARSE_LOCATION",
        ],
        DevicePermission::Notifications => &["android.permission.POST_NOTIFICATIONS"],
        DevicePermission::Motion => &["android.permission.ACTIVITY_RECOGNITION"],
        DevicePermission::Reminders | DevicePermission::MediaLibrary | DevicePermission::FaceId => {
            return None;
        }
    })
}

fn android_orientation(orientation: DeviceOrientation) -> (&'static str, &'static str) {
    match orientation {
        DeviceOrientation::Portrait => ("0:9.81:0", "0"),
        DeviceOrientation::LandscapeLeft => ("9.81:0:0", "1"),
        DeviceOrientation::PortraitUpsideDown => ("0:-9.81:0", "2"),
        DeviceOrientation::LandscapeRight => ("-9.81:0:0", "3"),
    }
}

fn adb_shell(adb: &str, serial: &str, args: &[&str]) -> Vec<String> {
    [adb, "-s", serial, "shell"]
        .iter()
        .chain(args)
        .map(|arg| arg.to_string())
        .collect()
}

pub(crate) fn display_rotation(dumpsys: &str) -> Option<String> {
    let rotation = regex::Regex::new(r"\bmRotation=(\d)\b").ok()?;
    Some(rotation.captures(dumpsys)?.get(1)?.as_str().to_string())
}

pub(crate) fn android_action_argvs(
    adb: &str,
    serial: &str,
    action: &DeviceAction,
) -> Result<Vec<Vec<String>>, String> {
    if !hub_client::valid_device_id(serial) {
        return Err(DEVICE_ACTION_UNAVAILABLE.into());
    }
    validate(action)?;
    let adb_argv = |rest: &[&str]| -> Vec<String> {
        [adb, "-s", serial]
            .iter()
            .chain(rest)
            .map(|arg| arg.to_string())
            .collect()
    };
    let shell = |rest: &[&str]| adb_argv(&[&["shell"][..], rest].concat());
    let unavailable = || Err(DEVICE_ACTION_UNAVAILABLE.to_string());
    Ok(match action {
        DeviceAction::SetAppearance { value } => vec![shell(&[
            "cmd",
            "uimode",
            "night",
            match value {
                DeviceAppearance::Dark => "yes",
                DeviceAppearance::Light => "no",
            },
        ])],
        DeviceAction::SetTextSize { value } => vec![shell(&[
            "settings",
            "put",
            "system",
            "font_scale",
            android_text_size(*value),
        ])],
        DeviceAction::SetToggle {
            setting: DeviceToggle::NetworkEnabled,
            ..
        }
        | DeviceAction::SetLocation { .. }
            if !serial.starts_with("emulator-") =>
        {
            return unavailable();
        }
        DeviceAction::SetToggle {
            setting: DeviceToggle::NetworkEnabled,
            enabled,
        } => {
            let state = if *enabled { "enable" } else { "disable" };
            vec![
                shell(&["svc", "wifi", state]),
                shell(&["svc", "data", state]),
            ]
        }
        DeviceAction::SetToggle {
            setting: DeviceToggle::ReduceMotion,
            enabled,
        } => [
            "animator_duration_scale",
            "transition_animation_scale",
            "window_animation_scale",
        ]
        .iter()
        .map(|key| {
            shell(&[
                "settings",
                "put",
                "global",
                key,
                if *enabled { "0" } else { "1" },
            ])
        })
        .collect(),
        DeviceAction::SetOrientation { value } => {
            let (gravity, rotation) = android_orientation(*value);
            // Locking the user rotation leaves the encoded display unrotated on emulators; tilting the accelerometer does not.
            if serial.starts_with("emulator-") {
                vec![
                    shell(&["settings", "put", "system", "accelerometer_rotation", "1"]),
                    shell(&["cmd", "window", "user-rotation", "free"]),
                    adb_argv(&["emu", "sensor", "set", "acceleration", gravity]),
                ]
            } else {
                vec![shell(&["cmd", "window", "user-rotation", "lock", rotation])]
            }
        }
        DeviceAction::SetLocation { lat, lon } => vec![adb_argv(&[
            "emu",
            "geo",
            "fix",
            &lon.to_string(),
            &lat.to_string(),
        ])],
        DeviceAction::ClearLocation => Vec::new(),
        DeviceAction::SetPermission {
            app_id,
            permission,
            decision: choice,
        } => {
            let verb = match choice {
                PermissionDecision::Grant => "grant",
                PermissionDecision::Revoke | PermissionDecision::Reset => "revoke",
            };
            let Some(names) = android_permissions(*permission) else {
                return unavailable();
            };
            names
                .iter()
                .map(|name| shell(&["pm", verb, app_id, name]))
                .collect()
        }
        DeviceAction::OpenUrl { url } => {
            // adb joins its arguments into one command line for the device shell.
            let url = super::agent::sh_quote(&open_url(url)?);
            vec![shell(&[
                "am",
                "start",
                "-a",
                "android.intent.action.VIEW",
                "-d",
                &url,
            ])]
        }
        DeviceAction::LaunchApp { app_id } => vec![shell(&[
            "monkey",
            "-p",
            app_id,
            "-c",
            "android.intent.category.LAUNCHER",
            "1",
        ])],
        DeviceAction::TerminateApp { app_id } => vec![shell(&["am", "force-stop", app_id])],
        DeviceAction::SetToggle { .. }
        | DeviceAction::SetLiquidGlass { .. }
        | DeviceAction::SetColorFilter { .. }
        | DeviceAction::SendPush { .. } => return unavailable(),
    })
}

pub(crate) fn android_settings_from(
    night: Option<&str>,
    font_scale: Option<&str>,
    animator: Option<&str>,
    wifi: Option<&str>,
) -> DeviceSettingsSnapshot {
    let number = |value: Option<&str>| value.and_then(|value| value.trim().parse::<f64>().ok());
    DeviceSettingsSnapshot {
        appearance: night.and_then(|night| {
            if night.contains("yes") {
                Some(DeviceAppearance::Dark)
            } else if night.contains("no") {
                Some(DeviceAppearance::Light)
            } else {
                None
            }
        }),
        text_size: number(font_scale).map(|scale| {
            if scale <= 0.9 {
                DeviceTextSize::Small
            } else if scale >= 1.25 {
                DeviceTextSize::ExtraLarge
            } else if scale >= 1.1 {
                DeviceTextSize::Large
            } else {
                DeviceTextSize::Default
            }
        }),
        reduce_motion: match animator.map(str::trim) {
            Some("null") => Some(false),
            animator => number(animator).map(|scale| scale == 0.0),
        },
        network_enabled: match wifi.map(str::trim) {
            Some("1" | "2") => Some(true),
            Some("0" | "3") => Some(false),
            _ => None,
        },
        ..Default::default()
    }
}

pub(crate) fn android_foreground(dumpsys: &str) -> Option<ForegroundApp> {
    let focus =
        regex::Regex::new(r"m(?:CurrentFocus|FocusedApp)=\w+\{[^ ]+ u\d+ ([^/ ]+)/").ok()?;
    let id = focus.captures(dumpsys)?.get(1)?.as_str().to_string();
    Some(ForegroundApp { id, pid: None })
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
        network_enabled: None,
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
    let cut = |text: &str, limit: usize| text.chars().take(limit).collect::<String>();
    let text = |key: &str| cut(value[key].as_str().unwrap_or_default(), EVENT_FIELD_LIMIT);
    let summary = value["summary"]
        .as_str()
        .or_else(|| value["msg"].as_str())
        .unwrap_or_default();
    Some(DeviceEventLogEntry {
        id: id.to_string(),
        timestamp: text("timestamp"),
        kind: text("kind"),
        summary: cut(summary, EVENT_SUMMARY_LIMIT),
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

pub(crate) fn screenshot_file_name(device_name: &str, millis: i64, attempt: usize) -> String {
    let base: String = device_name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' {
                c
            } else {
                '-'
            }
        })
        .collect();
    match attempt {
        0 => format!("{base}-{millis}.png"),
        n => format!("{base}-{millis}-{n}.png"),
    }
}

pub(crate) fn save_screenshot(
    dir: &Path,
    device_name: &str,
    millis: i64,
    png: &[u8],
) -> Result<PathBuf, String> {
    use std::io::Write;
    let failed = |e: std::io::Error| format!("Could not save the screenshot: {e}");
    for attempt in 0..SCREENSHOT_NAME_TRIES {
        let path = dir.join(screenshot_file_name(device_name, millis, attempt));
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
        {
            Ok(mut file) => {
                file.write_all(png).map_err(failed)?;
                return Ok(path);
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(failed(e)),
        }
    }
    Err("Could not save the screenshot: too many files with that name.".into())
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

async fn read_android_detail(
    device: &HubDevice,
    runner: &Runner,
    adb: Option<String>,
) -> DeviceDetail {
    let shell = |args: &[&str]| {
        let run = adb.as_deref().map(|adb| {
            let argv = [adb, "-s", &device.id, "shell"]
                .iter()
                .chain(args)
                .map(|arg| arg.to_string())
                .collect();
            runner(argv)
        });
        async move {
            let output = run?.await;
            (output.code == 0).then_some(output.stdout)
        }
    };
    let (night, font_scale, animator, wifi, window) = tokio::join!(
        shell(&["cmd", "uimode", "night"]),
        shell(&["settings", "get", "system", "font_scale"]),
        shell(&["settings", "get", "global", "animator_duration_scale"]),
        shell(&["settings", "get", "global", "wifi_on"]),
        shell(&["dumpsys", "window"]),
    );
    DeviceDetail {
        host_id: device.host_id.clone(),
        device_id: device.id.clone(),
        settings: android_settings_from(
            night.as_deref(),
            font_scale.as_deref(),
            animator.as_deref(),
            wifi.as_deref(),
        ),
        foreground_app: window.as_deref().and_then(android_foreground),
        read_at: chrono::Utc::now().to_rfc3339(),
    }
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

    pub(super) async fn list_avds(&self) -> String {
        let (runner, helpers) = self.action_env();
        let Some(root) = helpers.android_sdk else {
            return String::new();
        };
        let emulator = super::platform::emulator(&root);
        let output = runner(vec![
            emulator.to_string_lossy().into_owned(),
            "-list-avds".into(),
        ])
        .await;
        if output.code != 0 {
            tracing::warn!(stderr = %output.stderr.trim(), "listing Android virtual devices failed");
            return String::new();
        }
        output.stdout
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
        let lock = self.device_lock(&device.id);
        let _serial = lock.lock().await;
        Ok(self.read_detail(&device, true).await)
    }

    async fn read_detail(&self, device: &HubDevice, foreground: bool) -> DeviceDetail {
        let (runner, helpers) = self.action_env();
        if device.platform == DevicePlatform::Android {
            return read_android_detail(device, &runner, helpers.adb()).await;
        }
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
        let first = async {
            let mut feed = self.app_state_feed(Some(host_id), device_id).await.ok()?;
            feed.next().await
        };
        tokio::time::timeout(FOREGROUND_READ, first)
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
        let dir = match &params.action {
            DeviceAction::SendPush { .. } => Some(private_dir().map_err(|e| e.to_string())?),
            _ => None,
        };
        let push_file = dir.as_ref().map(|dir| dir.path().join("payload.json"));
        let argvs = match device.platform {
            DevicePlatform::Ios => vec![action_argv(
                device.platform,
                &device.id,
                &params.action,
                &helpers,
                push_file.as_deref(),
            )?],
            DevicePlatform::Android => android_action_argvs(
                &helpers.adb().ok_or(ANDROID_SDK_MISSING)?,
                &device.id,
                &params.action,
            )?,
        };
        if let (DeviceAction::SendPush { payload, .. }, Some(push_file)) =
            (&params.action, &push_file)
        {
            write_private(push_file, &push_payload(payload)?).map_err(|e| e.to_string())?;
        }
        // Apps rarely declare every permission of a group; t3code ignores the refusals too.
        let tolerant = device.platform == DevicePlatform::Android
            && matches!(params.action, DeviceAction::SetPermission { .. });
        let steps = argvs.len();
        let mut succeeded = 0;
        let mut refusal = None;
        let rotation = match (&params.action, helpers.adb()) {
            (DeviceAction::SetOrientation { value }, Some(adb))
                if device.platform == DevicePlatform::Android
                    && device.id.starts_with("emulator-") =>
            {
                Some((*value, adb))
            }
            _ => None,
        };
        let lock = self.device_lock(&device.id);
        let _serial = lock.lock().await;
        if let Some((value, adb)) = &rotation
            && *value != DeviceOrientation::Portrait
        {
            self.save_auto_rotate(&runner, adb, &device.id).await;
        }
        for (step, argv) in argvs.into_iter().enumerate() {
            let output = runner(argv).await;
            if output.code == 0 {
                succeeded += 1;
                continue;
            }
            let detail = output.stderr.trim();
            let detail = if detail.is_empty() {
                format!("exit code {}", output.code)
            } else {
                detail.chars().take(400).collect()
            };
            if tolerant {
                refusal = Some(detail);
            } else if steps > 1 {
                return Err(format!(
                    "The device action failed at step {} of {steps}: {detail}",
                    step + 1
                ));
            } else {
                return Err(format!("The device action failed: {detail}"));
            }
        }
        if succeeded == 0
            && let Some(detail) = refusal
        {
            return Err(format!("The device action failed: {detail}"));
        }
        if let Some((value, adb)) = &rotation {
            self.settle_rotation(&runner, adb, &device.id, *value)
                .await?;
        }
        Ok(self.read_detail(&device, false).await)
    }

    async fn save_auto_rotate(&self, runner: &Runner, adb: &str, serial: &str) {
        let saved = |hub: &Self| {
            hub.auto_rotate
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .contains_key(serial)
        };
        if saved(self) {
            return;
        }
        let output = runner(adb_shell(
            adb,
            serial,
            &["settings", "get", "system", "accelerometer_rotation"],
        ))
        .await;
        let value = output.stdout.trim();
        if output.code == 0 && matches!(value, "0" | "1") {
            self.auto_rotate
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .insert(serial.to_string(), value.to_string());
        }
    }

    // Taps were misdirected live while the sensor stayed tilted under a display that did not rotate.
    async fn settle_rotation(
        &self,
        runner: &Runner,
        adb: &str,
        serial: &str,
        value: DeviceOrientation,
    ) -> Result<(), String> {
        let landscape_or_flipped = value != DeviceOrientation::Portrait;
        if landscape_or_flipped {
            let expected = android_orientation(value).1;
            let deadline = tokio::time::Instant::now() + ROTATION_SETTLE;
            loop {
                let output =
                    runner(adb_shell(adb, serial, &["dumpsys", "window", "displays"])).await;
                if output.code == 0 && display_rotation(&output.stdout).as_deref() == Some(expected)
                {
                    return Ok(());
                }
                if tokio::time::Instant::now() >= deadline {
                    break;
                }
                tokio::time::sleep(ROTATION_POLL).await;
            }
            let portrait = android_orientation(DeviceOrientation::Portrait).0;
            runner(
                [
                    adb,
                    "-s",
                    serial,
                    "emu",
                    "sensor",
                    "set",
                    "acceleration",
                    portrait,
                ]
                .map(str::to_string)
                .to_vec(),
            )
            .await;
        }
        let previous = self
            .auto_rotate
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(serial);
        if let Some(previous) = previous {
            runner(adb_shell(
                adb,
                serial,
                &[
                    "settings",
                    "put",
                    "system",
                    "accelerometer_rotation",
                    &previous,
                ],
            ))
            .await;
        }
        if landscape_or_flipped {
            Err(PORTRAIT_LOCKED.into())
        } else {
            Ok(())
        }
    }

    pub async fn device_screenshot(
        &self,
        params: HubDeviceTargetParams,
    ) -> Result<HubDeviceScreenshot, String> {
        let device = self.action_target(params.host_id.as_deref(), &params.device_id)?;
        let png = super::agent::capture_device(&device).await?;
        let (width, height) = super::agent::png_dimensions(&png);
        let downloads = crate::repos::session_home_dir()
            .map_err(|e| format!("Could not save the screenshot: {e}"))?
            .join("Downloads");
        let path = save_screenshot(
            &downloads,
            &device.name,
            chrono::Utc::now().timestamp_millis(),
            &png,
        )?;
        Ok(HubDeviceScreenshot {
            path: path.to_string_lossy().into_owned(),
            width,
            height,
        })
    }

    pub async fn device_fold(
        &self,
        params: HubDeviceFoldParams,
    ) -> Result<DeviceFoldState, String> {
        let device = self.action_target(params.host_id.as_deref(), &params.device_id)?;
        if device.platform != DevicePlatform::Android {
            return Err(DEVICE_ACTION_UNAVAILABLE.into());
        }
        if params
            .posture
            .is_some_and(|posture| !matches!(posture, FoldPosture::Closed | FoldPosture::Opened))
        {
            return Err("A device can only be folded or unfolded.".into());
        }
        let origin = self.hub_client_origin()?;
        HubClient {
            http: &self.http,
            origin: &origin,
        }
        .fold(&device.id, params.posture)
        .await
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
            android_sdk: None,
        }
    }

    fn argv(action: DeviceAction) -> Result<Vec<String>, String> {
        action_argv(
            DevicePlatform::Ios,
            UDID,
            &action,
            &helpers(),
            Some(Path::new("/tmp/x/payload.json")),
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
        let wrapper = r#"{"aps":{"alert":""}}"#.len();
        assert!(push_payload(&json!("a".repeat(PUSH_LIMIT - wrapper))).is_ok());
        assert_eq!(
            argv(DeviceAction::SendPush {
                app_id: "a.b".into(),
                payload: json!("a".repeat(PUSH_LIMIT - wrapper + 1))
            }),
            Err(PUSH_TOO_LARGE.into())
        );
        assert!(
            action_argv(
                DevicePlatform::Ios,
                "../SIM",
                &DeviceAction::ClearLocation,
                &helpers(),
                Some(Path::new("/tmp/p.json"))
            )
            .is_err()
        );
    }

    #[test]
    fn open_url_passes_the_parsed_url_and_refuses_local_schemes() {
        for url in [
            "file:///etc/passwd",
            "FILE:///etc/passwd",
            "data:text/html,<b>x</b>",
            "javascript:alert(1)",
            "JavaScript:alert(1)",
            "about:blank",
        ] {
            assert_eq!(
                argv(DeviceAction::OpenUrl { url: url.into() }),
                Err(REFUSED_URL_SCHEME.into()),
                "{url:?}"
            );
        }
        assert_eq!(
            argv(DeviceAction::OpenUrl {
                url: "HTTPS://Example.com".into()
            }),
            Ok(words("xcrun simctl openurl <udid> https://example.com/"))
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
                Some(Path::new("/tmp/p.json")),
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
                    Some(Path::new("/tmp/p.json"))
                ),
                unavailable
            );
        }
    }

    #[test]
    fn every_android_action_maps_to_its_adb_argv_sequence() {
        let adb = "/sdk/platform-tools/adb";
        let run = |serial: &str, action: DeviceAction| android_action_argvs(adb, serial, &action);
        let lines = |serial: &str, lines: &[&str]| -> Vec<Vec<String>> {
            lines
                .iter()
                .map(|line| {
                    format!("{adb} -s {serial} {line}")
                        .split(' ')
                        .map(str::to_string)
                        .collect()
                })
                .collect()
        };
        let emu = "emulator-5554";
        let toggle = |setting, enabled| DeviceAction::SetToggle { setting, enabled };
        let cases: Vec<(DeviceAction, Vec<&str>)> = vec![
            (
                DeviceAction::SetAppearance {
                    value: DeviceAppearance::Dark,
                },
                vec!["shell cmd uimode night yes"],
            ),
            (
                DeviceAction::SetAppearance {
                    value: DeviceAppearance::Light,
                },
                vec!["shell cmd uimode night no"],
            ),
            (
                DeviceAction::SetTextSize {
                    value: DeviceTextSize::Small,
                },
                vec!["shell settings put system font_scale 0.85"],
            ),
            (
                DeviceAction::SetTextSize {
                    value: DeviceTextSize::ExtraLarge,
                },
                vec!["shell settings put system font_scale 1.3"],
            ),
            (
                toggle(DeviceToggle::NetworkEnabled, false),
                vec!["shell svc wifi disable", "shell svc data disable"],
            ),
            (
                toggle(DeviceToggle::NetworkEnabled, true),
                vec!["shell svc wifi enable", "shell svc data enable"],
            ),
            (
                toggle(DeviceToggle::ReduceMotion, true),
                vec![
                    "shell settings put global animator_duration_scale 0",
                    "shell settings put global transition_animation_scale 0",
                    "shell settings put global window_animation_scale 0",
                ],
            ),
            (
                DeviceAction::SetOrientation {
                    value: DeviceOrientation::LandscapeLeft,
                },
                vec![
                    "shell settings put system accelerometer_rotation 1",
                    "shell cmd window user-rotation free",
                    "emu sensor set acceleration 9.81:0:0",
                ],
            ),
            (
                DeviceAction::SetOrientation {
                    value: DeviceOrientation::Portrait,
                },
                vec![
                    "shell settings put system accelerometer_rotation 1",
                    "shell cmd window user-rotation free",
                    "emu sensor set acceleration 0:9.81:0",
                ],
            ),
            (
                DeviceAction::SetLocation {
                    lat: 35.6762,
                    lon: 139.6503,
                },
                vec!["emu geo fix 139.6503 35.6762"],
            ),
            (DeviceAction::ClearLocation, vec![]),
            (
                DeviceAction::SetPermission {
                    app_id: "com.example.app".into(),
                    permission: DevicePermission::Location,
                    decision: PermissionDecision::Revoke,
                },
                vec![
                    "shell pm revoke com.example.app android.permission.ACCESS_FINE_LOCATION",
                    "shell pm revoke com.example.app android.permission.ACCESS_COARSE_LOCATION",
                ],
            ),
            (
                DeviceAction::SetPermission {
                    app_id: "com.example.app".into(),
                    permission: DevicePermission::Camera,
                    decision: PermissionDecision::Grant,
                },
                vec!["shell pm grant com.example.app android.permission.CAMERA"],
            ),
            (
                DeviceAction::LaunchApp {
                    app_id: "com.android.settings".into(),
                },
                vec!["shell monkey -p com.android.settings -c android.intent.category.LAUNCHER 1"],
            ),
            (
                DeviceAction::TerminateApp {
                    app_id: "com.android.settings".into(),
                },
                vec!["shell am force-stop com.android.settings"],
            ),
            (
                DeviceAction::OpenUrl {
                    url: "https://example.com/a?b=c".into(),
                },
                vec!["shell am start -a android.intent.action.VIEW -d 'https://example.com/a?b=c'"],
            ),
        ];
        for (action, expected) in cases {
            assert_eq!(
                run(emu, action.clone()),
                Ok(lines(emu, &expected)),
                "{action:?}"
            );
        }
        assert_eq!(
            run(
                "R5CT1234",
                DeviceAction::SetOrientation {
                    value: DeviceOrientation::LandscapeRight
                }
            ),
            Ok(lines(
                "R5CT1234",
                &["shell cmd window user-rotation lock 3"]
            )),
            "a physical device locks the window rotation"
        );
    }

    #[test]
    fn android_urls_reach_the_device_shell_as_one_quoted_word() {
        let url = |url: &str| {
            android_action_argvs(
                "adb",
                "emulator-5554",
                &DeviceAction::OpenUrl { url: url.into() },
            )
            .map(|argvs| argvs[0].last().cloned().unwrap())
        };
        assert_eq!(
            url("https://example.com/a?b=c&d=$(reboot);x"),
            Ok("'https://example.com/a?b=c&d=$(reboot);x'".into())
        );
        assert_eq!(
            url("myapp://x?q=it's"),
            Ok(r#"'myapp://x?q=it'"'"'s'"#.into())
        );
        assert_eq!(url("file:///sdcard/x"), Err(REFUSED_URL_SCHEME.into()));
        assert_eq!(url("not a url"), Err(INVALID_URL.into()));
    }

    #[test]
    fn android_rejects_ios_only_actions_bad_ids_and_bad_serials() {
        let unavailable = Err(DEVICE_ACTION_UNAVAILABLE.to_string());
        let run = |serial: &str, action: DeviceAction| android_action_argvs("adb", serial, &action);
        for action in [
            DeviceAction::SetLiquidGlass {
                value: LiquidGlass::Clear,
            },
            DeviceAction::SetColorFilter {
                value: ColorFilter::Grayscale,
            },
            DeviceAction::SendPush {
                app_id: "a.b".into(),
                payload: json!("hi"),
            },
            DeviceAction::SetToggle {
                setting: DeviceToggle::VoiceOver,
                enabled: true,
            },
            DeviceAction::SetToggle {
                setting: DeviceToggle::IncreaseContrast,
                enabled: true,
            },
            DeviceAction::SetPermission {
                app_id: "a.b".into(),
                permission: DevicePermission::Reminders,
                decision: PermissionDecision::Grant,
            },
        ] {
            assert_eq!(
                run("emulator-5554", action.clone()),
                unavailable,
                "{action:?}"
            );
        }
        assert_eq!(
            run("../x", DeviceAction::ClearLocation),
            unavailable,
            "a serial that is not a device id"
        );
        assert_eq!(
            run(
                "emulator-5554",
                DeviceAction::LaunchApp {
                    app_id: "com.app;reboot".into()
                }
            ),
            Err(INVALID_APP_ID.into())
        );
        assert_eq!(
            run(
                "emulator-5554",
                DeviceAction::SetLocation {
                    lat: 91.0,
                    lon: 0.0
                }
            ),
            Err(INVALID_LOCATION.into())
        );
        for action in [
            DeviceAction::SetToggle {
                setting: DeviceToggle::NetworkEnabled,
                enabled: false,
            },
            DeviceAction::SetLocation {
                lat: 35.0,
                lon: 139.0,
            },
        ] {
            assert_eq!(
                run("192.168.1.20:5555", action.clone()),
                unavailable,
                "{action:?} on a physical device"
            );
        }
    }

    #[test]
    fn android_detail_parses_real_adb_output() {
        let settings = android_settings_from(
            Some("Night mode: no\n"),
            Some("1.0\n"),
            Some("null\n"),
            Some("1\n"),
        );
        assert_eq!(
            settings,
            DeviceSettingsSnapshot {
                appearance: Some(DeviceAppearance::Light),
                text_size: Some(DeviceTextSize::Default),
                reduce_motion: Some(false),
                network_enabled: Some(true),
                ..Default::default()
            },
            "an unset animator scale is the default 1, so motion is not reduced"
        );
        for (wifi_on, enabled) in [("2", true), ("3", false)] {
            assert_eq!(
                android_settings_from(None, None, None, Some(wifi_on)).network_enabled,
                Some(enabled),
                "wifi_on {wifi_on} (airplane mode states)"
            );
        }
        let dark =
            android_settings_from(Some("Night mode: yes"), Some("1.3"), Some("0.0"), Some("0"));
        assert_eq!(
            (
                dark.appearance,
                dark.text_size,
                dark.reduce_motion,
                dark.network_enabled
            ),
            (
                Some(DeviceAppearance::Dark),
                Some(DeviceTextSize::ExtraLarge),
                Some(true),
                Some(false)
            )
        );
        for (scale, size) in [
            ("0.85", DeviceTextSize::Small),
            ("1.15", DeviceTextSize::Large),
            ("1.0", DeviceTextSize::Default),
        ] {
            assert_eq!(
                android_settings_from(None, Some(scale), None, None).text_size,
                Some(size)
            );
        }
        let dumpsys = "  mCurrentFocus=Window{c17dad0 u0 Application Not Responding: com.android.systemui}\n  mFocusedApp=ActivityRecord{154543861 u0 com.google.android.apps.nexuslauncher/.NexusLauncherActivity t6}\n";
        assert_eq!(
            android_foreground(dumpsys),
            Some(ForegroundApp {
                id: "com.google.android.apps.nexuslauncher".into(),
                pid: None
            })
        );
        assert_eq!(android_foreground("mCurrentFocus=null"), None);
        let displays = "  overrideConfig={ winConfig={ mDisplayRotation=ROTATION_0 mRotation=ROTATION_0} }\n  DisplayRotation\n    mCurrentAppOrientation=SCREEN_ORIENTATION_UNSPECIFIED\n    mRotation=1 mDeferredRotationPauseCount=0\n    WindowOrientationListener\n      mCurrentRotation=ROTATION_0\n";
        assert_eq!(display_rotation(displays).as_deref(), Some("1"));
        assert_eq!(display_rotation("mRotation=ROTATION_90"), None);
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
                network_enabled: None,
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

        let outside = tempfile::tempdir().unwrap();
        let decoy = outside.path().join("vendor/serve-sim/dist");
        std::fs::create_dir_all(decoy.join("simax")).unwrap();
        std::fs::write(decoy.join("simax/serve-sim-ax-settings"), "").unwrap();
        let entry = outside
            .path()
            .join("node_modules/expo-device-hub/dist/server/cli.mjs");
        std::fs::create_dir_all(entry.parent().unwrap()).unwrap();
        std::fs::write(&entry, "").unwrap();
        assert!(
            Helpers::from_entry(&entry, Some("/bin/node".into()))
                .ax_settings
                .is_none(),
            "a vendor dir above the package is never used"
        );
    }

    #[test]
    fn screenshots_are_saved_under_a_fresh_name_and_never_through_a_symlink() {
        assert_eq!(
            screenshot_file_name("iPhone 17 Pro", 1_759_651_200_123, 0),
            "iPhone-17-Pro-1759651200123.png"
        );
        assert_eq!(screenshot_file_name("../é/x", 5, 2), "-----x-5-2.png");
        let dir = tempfile::tempdir().unwrap();
        let png = [0x89, b'P', b'N', b'G', 1, 2, 3];
        let first = save_screenshot(dir.path(), "iPhone 17", 42, &png).unwrap();
        assert_eq!(first, dir.path().join("iPhone-17-42.png"));
        assert_eq!(std::fs::read(&first).unwrap(), png);
        let second = save_screenshot(dir.path(), "iPhone 17", 42, b"two").unwrap();
        assert_eq!(second, dir.path().join("iPhone-17-42-1.png"));
        assert_eq!(std::fs::read(&first).unwrap(), png, "never overwritten");

        let sentinel = dir.path().join("sentinel");
        std::fs::write(&sentinel, "keep").unwrap();
        std::os::unix::fs::symlink(&sentinel, dir.path().join("Pad-7.png")).unwrap();
        std::os::unix::fs::symlink(dir.path().join("missing"), dir.path().join("Pad-7-1.png"))
            .unwrap();
        let saved = save_screenshot(dir.path(), "Pad", 7, b"shot").unwrap();
        assert_eq!(saved, dir.path().join("Pad-7-2.png"));
        assert_eq!(std::fs::read_to_string(&sentinel).unwrap(), "keep");
        assert!(!dir.path().join("missing").exists());
        assert!(
            save_screenshot(&dir.path().join("absent"), "Pad", 7, b"shot")
                .unwrap_err()
                .starts_with("Could not save the screenshot")
        );
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
        let long = event_batch(&json!({"event": {
            "id": 10, "timestamp": "t".repeat(500), "kind": "k".repeat(500), "summary": "s".repeat(50_000)
        }}))
        .unwrap();
        let entry = &long.events[0];
        assert_eq!(
            (entry.timestamp.len(), entry.kind.len(), entry.summary.len()),
            (EVENT_FIELD_LIMIT, EVENT_FIELD_LIMIT, EVENT_SUMMARY_LIMIT)
        );
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

    use super::super::tests::{HEALTHY, HUGE_AX, STALLED_APP_STATE, enabled_hub, fake_hub};
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
                android_sdk: None,
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
    async fn an_emulator_rotation_the_display_ignores_is_undone_and_auto_rotate_comes_back() {
        use zeron_proto::device_hub::DeviceOrientation;
        let (_dir, hub, _) = enabled_hub(HEALTHY).await;
        hub.publish(|state| {
            state.devices.push(HubDevice {
                host_id: "local".into(),
                id: "emulator-5554".into(),
                name: "Pixel_9".into(),
                platform: DevicePlatform::Android,
                booted: true,
                ..Default::default()
            })
        });
        let log: Log = Arc::default();
        let display = Arc::new(Mutex::new("0"));
        let runner: Runner = {
            let (log, display) = (log.clone(), display.clone());
            Arc::new(move |argv: Vec<String>| {
                let line = argv[3..].join(" ");
                log.lock().unwrap().push(line.clone());
                let stdout = match line.as_str() {
                    "shell dumpsys window displays" => format!(
                        "    mRotation={} mDeferredRotationPauseCount=0\n  winConfig={{ mDisplayRotation=ROTATION_0 mRotation=ROTATION_0}}\n",
                        display.lock().unwrap()
                    ),
                    "shell settings get system accelerometer_rotation" => "0\n".into(),
                    _ => String::new(),
                };
                Box::pin(async move {
                    CommandOutput {
                        code: 0,
                        stdout,
                        stderr: String::new(),
                    }
                })
            })
        };
        hub.use_actions_for_test(
            runner,
            Helpers {
                android_sdk: Some("/sdk".into()),
                ..Default::default()
            },
        );
        let rotate = |value| RunHubDeviceActionParams {
            host_id: None,
            device_id: "emulator-5554".into(),
            action: DeviceAction::SetOrientation { value },
        };
        let ran = |log: &Log| -> Vec<String> {
            let reads = [
                "shell cmd uimode night",
                "shell settings get system font_scale",
                "shell settings get global animator_duration_scale",
                "shell settings get global wifi_on",
                "shell dumpsys window",
            ];
            let mut lines = std::mem::take(&mut *log.lock().unwrap());
            lines.retain(|line| !reads.contains(&line.as_str()));
            lines.dedup();
            lines
        };
        let tilt = [
            "shell settings put system accelerometer_rotation 1",
            "shell cmd window user-rotation free",
        ];

        let started = std::time::Instant::now();
        assert_eq!(
            hub.run_action(rotate(DeviceOrientation::LandscapeLeft))
                .await,
            Err(PORTRAIT_LOCKED.into())
        );
        assert!(started.elapsed() >= ROTATION_SETTLE);
        assert_eq!(
            ran(&log),
            [
                "shell settings get system accelerometer_rotation",
                tilt[0],
                tilt[1],
                "emu sensor set acceleration 9.81:0:0",
                "shell dumpsys window displays",
                "emu sensor set acceleration 0:9.81:0",
                "shell settings put system accelerometer_rotation 0",
            ]
        );

        *display.lock().unwrap() = "1";
        hub.run_action(rotate(DeviceOrientation::LandscapeLeft))
            .await
            .expect("the display followed the sensor");
        assert_eq!(
            ran(&log),
            [
                "shell settings get system accelerometer_rotation",
                tilt[0],
                tilt[1],
                "emu sensor set acceleration 9.81:0:0",
                "shell dumpsys window displays",
            ]
        );
        *display.lock().unwrap() = "3";
        hub.run_action(rotate(DeviceOrientation::LandscapeRight))
            .await
            .unwrap();
        assert_eq!(
            ran(&log),
            [
                tilt[0],
                tilt[1],
                "emu sensor set acceleration -9.81:0:0",
                "shell dumpsys window displays",
            ],
            "the setting from before the first rotation is the one kept"
        );
        hub.run_action(rotate(DeviceOrientation::Portrait))
            .await
            .unwrap();
        assert_eq!(
            ran(&log),
            [
                tilt[0],
                tilt[1],
                "emu sensor set acceleration 0:9.81:0",
                "shell settings put system accelerometer_rotation 0",
            ]
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn android_multi_step_actions_name_the_failed_step_and_permissions_fail_only_when_all_do()
    {
        use zeron_proto::device_hub::{DevicePermission, DeviceToggle, PermissionDecision};
        let (_dir, hub, _) = enabled_hub(HEALTHY).await;
        hub.publish(|state| {
            state.devices.push(HubDevice {
                host_id: "local".into(),
                id: "emulator-5554".into(),
                name: "Pixel_9".into(),
                platform: DevicePlatform::Android,
                booted: true,
                ..Default::default()
            })
        });
        let refusing: Arc<Mutex<Vec<&'static str>>> = Arc::default();
        let runner: Runner = {
            let refusing = refusing.clone();
            Arc::new(move |argv: Vec<String>| {
                let refused = refusing
                    .lock()
                    .unwrap()
                    .iter()
                    .any(|word| argv.iter().any(|arg| arg == word));
                Box::pin(async move {
                    CommandOutput {
                        code: if refused { 255 } else { 0 },
                        stdout: String::new(),
                        stderr: if refused {
                            "Error: refused".into()
                        } else {
                            String::new()
                        },
                    }
                })
            })
        };
        hub.use_actions_for_test(
            runner,
            Helpers {
                android_sdk: Some("/sdk".into()),
                ..Default::default()
            },
        );
        let run = |action| {
            hub.run_action(RunHubDeviceActionParams {
                host_id: None,
                device_id: "emulator-5554".into(),
                action,
            })
        };
        let network_off = DeviceAction::SetToggle {
            setting: DeviceToggle::NetworkEnabled,
            enabled: false,
        };
        *refusing.lock().unwrap() = vec!["data"];
        assert_eq!(
            run(network_off.clone()).await,
            Err("The device action failed at step 2 of 2: Error: refused".into())
        );
        let location = |decision| DeviceAction::SetPermission {
            app_id: "com.example.app".into(),
            permission: DevicePermission::Location,
            decision,
        };
        *refusing.lock().unwrap() = vec!["android.permission.ACCESS_COARSE_LOCATION"];
        run(location(PermissionDecision::Grant))
            .await
            .expect("one granted permission of the group is enough");
        *refusing.lock().unwrap() = vec!["pm"];
        assert_eq!(
            run(location(PermissionDecision::Revoke)).await,
            Err("The device action failed: Error: refused".into())
        );
    }

    #[tokio::test]
    async fn command_output_past_the_cap_is_refused_without_waiting_for_the_command() {
        let sh = |script: &str| {
            let mut command = tokio::process::Command::new("/bin/sh");
            command.args(["-c", script]);
            command
        };
        let output = capped_output(sh("head -c 2048 /dev/zero"), 4096)
            .await
            .unwrap();
        assert_eq!(output.stdout.len(), 2048);
        assert!(
            capped_output(sh("head -c 2048 /dev/zero"), 1024)
                .await
                .is_err()
        );
        tokio::time::timeout(
            Duration::from_secs(5),
            capped_output(sh("exec yes"), 1 << 20),
        )
        .await
        .expect("an endless command is cut off at the cap")
        .unwrap_err();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_stuck_app_state_feed_never_holds_the_device_lock() {
        let (_dir, hub, _) = enabled_hub(STALLED_APP_STATE).await;
        hub.use_actions_for_test(
            recording_runner(Arc::default(), "light"),
            Helpers::default(),
        );
        let detail = tokio::time::timeout(
            Duration::from_secs(3),
            hub.device_detail(HubDeviceTargetParams {
                host_id: None,
                device_id: "SIM-1".into(),
            }),
        )
        .await
        .expect("the foreground read is capped, connect included")
        .unwrap();
        assert_eq!(detail.foreground_app, None);
        assert_eq!(detail.settings.appearance, Some(DeviceAppearance::Light));
        tokio::time::timeout(Duration::from_secs(3), hub.run_action(dark("SIM-1")))
            .await
            .expect("the device lock was released")
            .unwrap();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn an_oversized_ax_body_is_refused() {
        let (_dir, hub, _) = enabled_hub(HUGE_AX).await;
        assert_eq!(
            hub.device_ax_tree(HubDeviceTargetParams {
                host_id: None,
                device_id: "SIM-1".into(),
            })
            .await,
            Err(hub_client::AX_TOO_LARGE.into())
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
