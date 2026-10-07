use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{Value, json};

use super::escape_monitor::{EscapeKillSwitchMonitor, EscapeMonitorSink};
use super::frame_tap::StreamSignal;
use super::shield::{AppKitShieldSurface, ComputerShield};
use super::window_frame_stream::{WindowFrameStream, stream_size};
use super::{desktop_events, permissions, release_held_input, window_info};
use crate::computer::host::{
    ComputerInputMonitorState, ComputerShieldEngagement, ComputerShieldHost, PhysicalComputerInput,
};

/// Proof knob (`ZERON_CUA_ESCAPE_PROBE=<file>`): arm the Escape listener on
/// its own thread and append every classified event to `<file>`;
/// `ZERON_CUA_BLOCK_MAIN_MS=<ms>` then blocks the main thread that long.
pub fn run_escape_probe_from_env() {
    let Some(path) = std::env::var_os("ZERON_CUA_ESCAPE_PROBE").map(PathBuf::from) else {
        return;
    };
    let probe = Arc::new(ProbeLog(path));
    let monitor = match EscapeKillSwitchMonitor::start(probe.clone()) {
        Ok(monitor) => Box::leak(Box::new(monitor)),
        Err(error) => {
            probe.write(json!({ "event": "error", "message": error.to_string() }));
            return;
        }
    };
    monitor.set_armed(true);
    let block_ms = std::env::var("ZERON_CUA_BLOCK_MAIN_MS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok());
    if let Some(block_ms) = block_ms {
        dispatch2::DispatchQueue::main().exec_async(move || {
            probe.write(json!({ "event": "main-blocked", "ms": block_ms }));
            std::thread::sleep(Duration::from_millis(block_ms));
            probe.write(json!({ "event": "main-unblocked" }));
        });
    }
}

struct ProbeLog(PathBuf);

impl ProbeLog {
    fn write(&self, mut line: Value) {
        use std::io::Write;
        line["ts"] = json!(chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true));
        line["thread"] = json!(std::thread::current().name().unwrap_or("?"));
        if let Ok(mut file) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.0)
        {
            let _ = writeln!(file, "{line}");
        }
    }
}

impl EscapeMonitorSink for ProbeLog {
    fn physical_input(&self, input: PhysicalComputerInput) {
        self.write(json!({
            "event": "physical",
            "kind": format!("{:?}", input.kind),
            "pid": input.pid,
            "window_id": input.window_id,
        }));
    }

    fn state_changed(&self, state: ComputerInputMonitorState) {
        self.write(json!({ "event": "state", "ready": state.ready, "error": state.error }));
    }

    fn error(&self, message: &str) {
        self.write(json!({ "event": "error", "message": message }));
    }
}

/// Proof knob (`ZERON_CUA_NATIVE_PROBE=<file>`): exercise the grant probes,
/// setup registration, lock probe, held-input release, a 3 s preview stream
/// of Zeron's own window (first frame saved beside `<file>`) and a 2 s
/// activation shield over it, then write one JSON report to `<file>`.
pub fn run_native_probe_from_env() {
    let Some(out) = std::env::var_os("ZERON_CUA_NATIVE_PROBE").map(PathBuf::from) else {
        return;
    };
    let _ = std::thread::Builder::new()
        .name("zeron-cua-native-probe".into())
        .spawn(move || {
            let report = match tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            {
                Ok(runtime) => runtime.block_on(native_probe(&out)),
                Err(error) => json!({ "error": format!("runtime: {error}") }),
            };
            let _ = std::fs::write(&out, serde_json::to_vec_pretty(&report).unwrap_or_default());
        });
}

async fn own_window() -> Option<window_info::WindowEntry> {
    let pid = std::process::id() as i32;
    for _ in 0..40 {
        if let Some(window) = window_info::on_screen_windows()
            .into_iter()
            .find(|window| window.pid == pid && window.layer == 0)
        {
            return Some(window);
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    None
}

async fn native_probe(out: &std::path::Path) -> Value {
    let grants = permissions::preflight_permissions();
    let registration = permissions::running_app_bundle()
        .map(|app| permissions::ensure_permission_setup_registration(&app).map_err(|f| f.code));
    let mut report = json!({
        "pid": std::process::id(),
        "permissions": {
            "accessibility": grants.accessibility,
            "input_monitoring": grants.input_monitoring,
            "screen_recording": grants.screen_recording,
        },
        "next_missing_grant": permissions::next_missing_grant(&grants).map(|grant| grant.pane()),
        "registration": format!("{registration:?}"),
        "screen_already_locked": desktop_events::screen_already_locked(),
        "release_held_input": format!("{:?}", release_held_input::release_held_input()),
    });
    let Some(window) = own_window().await else {
        report["error"] = json!("no on-screen Zeron window");
        return report;
    };
    report["window"] = json!({
        "id": window.id,
        "width": window.bounds.size.width,
        "height": window.bounds.size.height,
        "expected_frame": stream_size(window.bounds.size.width, window.bounds.size.height),
    });

    let signals = Arc::new(Mutex::new(Vec::<Result<Vec<u8>, String>>::new()));
    let recorded = signals.clone();
    let stream = WindowFrameStream::start(
        window.id,
        std::process::id(),
        Arc::new(move |signal| {
            recorded.lock().unwrap().push(match signal {
                StreamSignal::Frame(jpeg) => Ok(jpeg),
                StreamSignal::Failed(failure) => {
                    Err(format!("{}: {}", failure.code, failure.message))
                }
            });
        }),
    );
    report["stream_start"] = json!(stream.as_ref().err().map(|f| f.code));
    tokio::time::sleep(Duration::from_secs(3)).await;
    drop(stream);
    let signals = std::mem::take(&mut *signals.lock().unwrap());
    let frames: Vec<&Vec<u8>> = signals.iter().filter_map(|s| s.as_ref().ok()).collect();
    report["stream_frames"] = json!(frames.len());
    report["stream_failures"] = json!(
        signals
            .iter()
            .filter_map(|s| s.as_ref().err())
            .collect::<Vec<_>>()
    );
    if let Some(first) = frames.first() {
        let dims = image::load_from_memory(first)
            .map(|image| (image.width(), image.height()))
            .ok();
        report["first_frame"] = json!(dims);
        let _ = std::fs::write(out.with_extension("jpg"), first);
    }

    let shield = ComputerShield::new(Arc::new(AppKitShieldSurface));
    let engaged = shield
        .engage(
            ComputerShieldEngagement {
                shield_id: "probe".into(),
                frame: zeron_proto::computer::CuaShieldFrame {
                    x: window.bounds.origin.x,
                    y: window.bounds.origin.y,
                    width: window.bounds.size.width,
                    height: window.bounds.size.height,
                },
                window_id: window.id,
                pid: std::process::id(),
                label: Some("Zeron native probe".into()),
            },
            None,
        )
        .await;
    report["shield_engage"] = json!(format!("{engaged:?}"));
    tokio::time::sleep(Duration::from_secs(2)).await;
    report["shield_release_all"] = json!(format!("{:?}", shield.release_all().await));
    report
}
