pub mod desktop_events;
pub mod escape_classifier;
pub mod escape_monitor;
pub mod frame_tap;
pub mod permissions;
mod probe;
pub mod release_held_input;
pub mod shield;
pub mod status_item;
pub mod window_frame_stream;
mod window_info;

pub use probe::{run_escape_probe_from_env, run_native_probe_from_env};

use std::path::PathBuf;
use std::sync::{Arc, OnceLock};

use desktop_events::DesktopLifecycleWatch;
use escape_monitor::{EscapeKillSwitchMonitor, EscapeMonitorSink, INPUT_MONITORING_REQUIRED};
use frame_tap::{ComputerFrameTap, PreviewSink};
use shield::{AppKitShieldSurface, ComputerShield};
use window_frame_stream::ScreenCaptureFrames;

use super::host::{
    ComputerInputMonitorState, CuaDriverHost, CuaDriverHostOptions, PhysicalComputerInput,
};

pub struct NativeHostConfig {
    pub binary_path: PathBuf,
    pub bundle_id: String,
    pub preview: PreviewSink,
    pub(crate) browser_admission: Arc<crate::browser::control_admission::Admission>,
    pub cursor_style: Option<Box<dyn Fn() -> Option<super::host::CuaCursorStyle> + Send>>,
    pub on_takeover: Option<Box<dyn Fn(super::host::TakeoverEvent) + Send>>,
}

struct HostEvents {
    browser_admission: Arc<crate::browser::control_admission::Admission>,
    host: OnceLock<CuaDriverHost>,
}

impl EscapeMonitorSink for HostEvents {
    fn physical_input(&self, input: PhysicalComputerInput) {
        if let Some(host) = self.host.get() {
            drop(host.physical_input(input));
        }
    }

    fn state_changed(&self, state: ComputerInputMonitorState) {
        if !state.ready && !matches!(state.error.as_deref(), Some("input_monitor_starting")) {
            self.browser_admission.stop(None);
        }
        if let Some(host) = self.host.get() {
            host.input_monitor_state_changed(state);
        }
    }

    fn error(&self, message: &str) {
        tracing::warn!(target: "desktop_cua", "Escape monitor: {message}");
    }
}

#[derive(Clone)]
pub struct NativeHostLink {
    events: Arc<HostEvents>,
    monitor: Arc<EscapeKillSwitchMonitor>,
}

impl NativeHostLink {
    pub fn attach(&self, host: CuaDriverHost) {
        let _ = self.events.host.set(host);
    }

    pub fn monitor(&self) -> &Arc<EscapeKillSwitchMonitor> {
        &self.monitor
    }

    pub fn watch_desktop_lifecycle(
        &self,
        runtime: tokio::runtime::Handle,
    ) -> Option<DesktopLifecycleWatch> {
        let host = self.events.host.get()?.clone();
        Some(DesktopLifecycleWatch::start(
            Arc::new(BrowserLifecycle {
                driver: host,
                admission: self.events.browser_admission.clone(),
            }),
            runtime,
            Arc::new(|error| {
                tracing::warn!(target: "desktop_cua", "computer input pause failed: {error}");
            }),
        ))
    }
}

pub fn native_host_options(
    config: NativeHostConfig,
) -> std::io::Result<(CuaDriverHostOptions, NativeHostLink)> {
    let events = Arc::new(HostEvents {
        browser_admission: config.browser_admission,
        host: OnceLock::new(),
    });
    let monitor = Arc::new(EscapeKillSwitchMonitor::start(events.clone())?);
    let link = NativeHostLink {
        events: events.clone(),
        monitor: monitor.clone(),
    };
    let mut options = CuaDriverHostOptions::new(
        config.binary_path,
        config.bundle_id,
        Box::new(|| Box::pin(async { permissions::start_permission_setup().map(|_| ()) })),
    );
    options.cursor_style = config.cursor_style;
    options.on_takeover = config.on_takeover;
    options.protected_surface = Some(Box::new(super::protected_surfaces::any_showing));
    options.check_permissions = Some(Box::new({
        let events = events.clone();
        let monitor = monitor.clone();
        move |_force| {
            let events = events.clone();
            let monitor = monitor.clone();
            Box::pin(async move {
                let permissions = permissions::preflight_permissions();
                if permissions.input_monitoring == Some(true)
                    && monitor.state().error.as_deref() == Some(INPUT_MONITORING_REQUIRED)
                    && let Some(host) = events.host.get()
                    && host.is_input_monitor_requested().await
                {
                    monitor.refresh_denied_listener();
                    monitor.activate().await;
                }
                Ok(permissions)
            })
        }
    }));
    options.release_held_input = Some(Box::new(|| {
        Box::pin(async { release_held_input::release_held_input() })
    }));
    options.activate_input_monitor = Some(Box::new({
        let monitor = monitor.clone();
        move || {
            let monitor = monitor.clone();
            Box::pin(async move {
                monitor.activate().await;
                Ok(())
            })
        }
    }));
    options.input_monitor_state = Some(Box::new({
        let monitor = monitor.clone();
        move || monitor.state()
    }));
    options.on_input_monitor_armed_change = Some(Box::new({
        let monitor = monitor.clone();
        let admission = events.browser_admission.clone();
        move |armed| monitor.set_armed(armed || admission.active())
    }));
    options.frame_tap = Some(Arc::new(ComputerFrameTap::new(
        ScreenCaptureFrames,
        config.preview,
    )));
    options.shield = Some(Arc::new(ComputerShield::new(Arc::new(AppKitShieldSurface))));
    Ok((options, link))
}

pub fn start_native_host(
    config: NativeHostConfig,
) -> std::io::Result<(CuaDriverHost, NativeHostLink)> {
    let (options, link) = native_host_options(config)?;
    let host = CuaDriverHost::new(options)?;
    link.attach(host.clone());
    Ok((host, link))
}

struct BrowserLifecycle {
    driver: CuaDriverHost,
    admission: Arc<crate::browser::control_admission::Admission>,
}
impl super::desktop_lifecycle::ComputerDesktopLifecycleHost for BrowserLifecycle {
    fn pause_desktop(
        &self,
        reason: &'static str,
    ) -> futures::future::BoxFuture<'static, Result<(), String>> {
        self.admission.pause(reason);
        super::desktop_lifecycle::ComputerDesktopLifecycleHost::pause_desktop(&self.driver, reason)
    }
    fn resume_desktop(&self, reason: &'static str) {
        self.admission.resume(reason);
        super::desktop_lifecycle::ComputerDesktopLifecycleHost::resume_desktop(
            &self.driver,
            reason,
        );
    }
}
