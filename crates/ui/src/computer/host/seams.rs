use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use futures::future::LocalBoxFuture;
use zeron_engine::computer::visible_use::COMPUTER_USER_INTERACTION_QUIET_MS;
use zeron_proto::computer::{CuaComputerTask, CuaPreviewTarget, CuaShieldFrame, CuaToolResult};

use super::support::CuaCursorStyle;

pub type HostFuture<T> = LocalBoxFuture<'static, T>;

// The engine gives up on a host call after 35 s (CUA_HOST_CALL_TIMEOUT_MS); a held read still needs time to run.
const QUIET_WAIT_CAP: Duration = Duration::from_secs(20);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HostPermissions {
    pub accessibility: bool,
    pub screen_recording: bool,
    pub input_monitoring: Option<bool>,
}

impl HostPermissions {
    pub(super) fn changed_from(&self, other: &Self) -> bool {
        self.accessibility != other.accessibility || self.screen_recording != other.screen_recording
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ComputerInputMonitorState {
    pub ready: bool,
    pub error: Option<String>,
}

impl ComputerInputMonitorState {
    pub fn ready() -> Self {
        Self {
            ready: true,
            error: None,
        }
    }

    pub fn failed(error: &str) -> Self {
        Self {
            ready: false,
            error: Some(error.into()),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PhysicalInputKind {
    Keyboard,
    Pointer,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PhysicalComputerInput {
    pub kind: PhysicalInputKind,
    pub pid: Option<u32>,
    pub window_id: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TakeoverEvent {
    Paused {
        thread_id: String,
        pid: u32,
        window_id: Option<u32>,
    },
    Resumed {
        thread_id: String,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct ComputerShieldEngagement {
    pub shield_id: String,
    pub frame: CuaShieldFrame,
    pub window_id: u32,
    pub pid: u32,
    pub label: Option<String>,
}

#[async_trait(?Send)]
pub trait ComputerFrameTapHost: Send + Sync {
    fn update(&self, target: CuaPreviewTarget) -> Result<(), String>;
    fn touch(&self, task: &CuaComputerTask);
    async fn end_task(&self, task: CuaComputerTask) -> Result<(), String>;
    async fn stop(&self) -> Result<(), String>;
    async fn dispose(&self) -> Result<(), String>;
}

#[async_trait(?Send)]
pub trait ComputerShieldHost: Send + Sync {
    async fn engage(
        &self,
        request: ComputerShieldEngagement,
        task: Option<CuaComputerTask>,
    ) -> Result<(), String>;
    async fn release(&self, shield_id: String) -> Result<(), String>;
    async fn release_all(&self) -> Result<u64, String>;
    async fn end_task(&self, task: CuaComputerTask) -> Result<(), String>;
    async fn stop(&self) -> Result<(), String>;
    async fn dispose(&self) -> Result<(), String>;
}

type AsyncSeam<T> = Box<dyn Fn() -> HostFuture<T> + Send>;
pub type PermissionCheck = Box<dyn Fn(bool) -> HostFuture<Result<HostPermissions, String>> + Send>;
pub type OverviewNormalizer = Box<dyn Fn(&mut CuaToolResult) + Send>;

pub struct CuaDriverHostOptions {
    pub binary_path: PathBuf,
    pub bundle_id: String,
    pub setup: AsyncSeam<Result<(), String>>,
    pub check_permissions: Option<PermissionCheck>,
    pub release_held_input: Option<AsyncSeam<Result<(), String>>>,
    pub startup_timeout: Duration,
    pub normalize_overview: Option<OverviewNormalizer>,
    pub frame_tap: Option<Arc<dyn ComputerFrameTapHost>>,
    pub on_input_monitor_armed_change: Option<Box<dyn Fn(bool) + Send>>,
    pub input_monitor_state: Option<Box<dyn Fn() -> ComputerInputMonitorState + Send>>,
    pub activate_input_monitor: Option<AsyncSeam<Result<(), String>>>,
    pub shield: Option<Arc<dyn ComputerShieldHost>>,
    pub native_revision: Option<u64>,
    pub own_pids: Option<Box<dyn Fn() -> HashSet<u32> + Send>>,
    pub protected_surface: Option<Box<dyn Fn() -> bool + Send>>,
    pub cursor_style: Option<Box<dyn Fn() -> Option<CuaCursorStyle> + Send>>,
    pub warm_on_first_touch: bool,
    pub input_quiet: Duration,
    pub input_quiet_cap: Duration,
    pub on_takeover: Option<Box<dyn Fn(TakeoverEvent) + Send>>,
}

impl CuaDriverHostOptions {
    pub fn new(
        binary_path: PathBuf,
        bundle_id: String,
        setup: AsyncSeam<Result<(), String>>,
    ) -> Self {
        Self {
            binary_path,
            bundle_id,
            setup,
            check_permissions: None,
            release_held_input: None,
            startup_timeout: Duration::from_secs(5),
            normalize_overview: None,
            frame_tap: None,
            on_input_monitor_armed_change: None,
            input_monitor_state: None,
            activate_input_monitor: None,
            shield: None,
            native_revision: Some(zeron_proto::computer::CUA_NATIVE_REVISION),
            own_pids: None,
            protected_surface: None,
            cursor_style: None,
            warm_on_first_touch: warm_on_first_touch_from_env(),
            input_quiet: Duration::from_millis(COMPUTER_USER_INTERACTION_QUIET_MS),
            input_quiet_cap: QUIET_WAIT_CAP,
            on_takeover: None,
        }
    }
}

fn warm_on_first_touch_from_env() -> bool {
    warm_flag(
        std::env::var("ZERON_CUA_WARM_ON_FIRST_TOUCH")
            .ok()
            .as_deref(),
    )
}

fn warm_flag(raw: Option<&str>) -> bool {
    raw.is_some_and(|raw| {
        matches!(
            raw.trim().to_ascii_lowercase().as_str(),
            "1" | "true" | "on" | "yes"
        )
    })
}

#[cfg(test)]
mod tests {
    use super::warm_flag;

    #[test]
    fn warms_only_on_an_explicit_affirmative_flag() {
        for off in [None, Some("0"), Some("off"), Some(""), Some("no")] {
            assert!(!warm_flag(off), "{off:?}");
        }
        for on in ["1", "true", "on", "yes", " YES "] {
            assert!(warm_flag(Some(on)), "{on}");
        }
    }
}
