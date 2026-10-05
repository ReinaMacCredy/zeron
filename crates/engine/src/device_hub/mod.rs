mod actions;
mod agent;
mod hub_client;
mod hub_process;
mod input;
mod mjpeg;
mod platform;
mod stream;
mod toolchain;

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError, Weak};
use std::time::Duration;

use tokio::sync::watch;
use zeron_proto::device_hub::{
    CloseHubDeviceParams, ConfigureDeviceHubParams, DeviceBooting, DeviceHostStatus, DeviceHubHost,
    DeviceHubSettings, DeviceHubState, DeviceInput, DevicePlatform, DevicePlatformAvailability,
    DeviceSession, DeviceVideoCodec, LOCAL_DEVICE_HOST_ID, OpenHubDeviceParams,
    ShutdownHubDeviceParams, StreamLinkStatus,
};

pub use actions::DEVICE_ACTION_UNAVAILABLE;
pub use agent::{
    AGENT_ACCESS_OFF, BOTH_PLATFORMS, DEVICE_CALLER_REFUSED, DEVICE_TOOL_INSTRUCTIONS,
    TOOL_SUPPORT_OFF,
};
use hub_client::HubClient;
use hub_process::{Backoff, HubProcess};
use stream::{AvccFeed, DeviceStreams, FrameSlot};
pub(crate) use stream::{AvccSubscription, FrameSubscription, avcc_batches};

const SETTINGS_FILE: &str = "device-hub-settings.json";
const HUB_POLL: Duration = Duration::from_secs(1);
const MAX_FAILED_RESTARTS: u32 = 3;
const UNKNOWN_DEVICE: &str = "That device is not available.";
const IOS_ONLY: &str = "Only iOS Simulators support this so far.";
const ANDROID_H264_ONLY: &str = "Android emulators stream H.264 only.";
const HUB_NOT_RUNNING: &str = "The device hub is not running.";
pub const DEVICE_SUPPORT_OFF: &str = "Device support is off. Enable it in the Device panel before installing or starting device tools.";

pub struct DeviceHub {
    me: Weak<DeviceHub>,
    data_dir: PathBuf,
    http: reqwest::Client,
    state: watch::Sender<DeviceHubState>,
    settings_lock: tokio::sync::Mutex<()>,
    lifecycle: tokio::sync::Mutex<()>,
    hub: Mutex<Option<HubProcess>>,
    streams: Arc<DeviceStreams>,
    backoff: Mutex<Backoff>,
    restart: Mutex<Option<tokio::task::JoinHandle<()>>>,
    closed: AtomicBool,
    agent: Mutex<Option<agent::AgentDaemon>>,
    agent_start: tokio::sync::Mutex<()>,
    action_locks: Mutex<std::collections::HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
    #[cfg(test)]
    test_spawn: Mutex<Option<TestSpawn>>,
    #[cfg(test)]
    test_actions: Mutex<Option<(actions::Runner, actions::Helpers)>>,
    #[cfg(test)]
    test_agent: Mutex<Option<agent::AgentTool>>,
    #[cfg(test)]
    test_platforms: Mutex<Option<Vec<DevicePlatformAvailability>>>,
}

#[cfg(test)]
type TestSpawn = Box<dyn Fn() -> Result<HubProcess, String> + Send + Sync>;

fn host_summary(platforms: Vec<DevicePlatformAvailability>, running: bool) -> DeviceHubHost {
    DeviceHubHost {
        id: LOCAL_DEVICE_HOST_ID.into(),
        kind: "local".into(),
        label: "This machine".into(),
        platforms,
        tools: Some(toolchain::versions(running)),
        hub_installed: toolchain::hub_installed(),
    }
}

fn running_tools(running: bool) -> impl Fn(&mut DeviceHubState) {
    let tools = toolchain::versions(running);
    let installed = toolchain::hub_installed();
    move |state| {
        for host in &mut state.hosts {
            host.tools = Some(tools.clone());
            host.hub_installed = installed;
        }
    }
}

fn host_diagnostics(errors: Vec<String>, android_available: bool) -> Option<String> {
    let kept: Vec<String> = errors
        .into_iter()
        .filter(|message| android_available || !message.contains("[android-utils]"))
        .collect();
    (!kept.is_empty()).then(|| kept.join("\n"))
}

fn available(platforms: &[DevicePlatformAvailability], platform: DevicePlatform) -> bool {
    platforms
        .iter()
        .any(|candidate| candidate.platform == platform && candidate.available)
}

fn stopped_avds(
    stdout: &str,
    devices: &[zeron_proto::device_hub::HubDevice],
) -> Vec<zeron_proto::device_hub::HubDevice> {
    stdout
        .lines()
        .map(str::trim)
        .filter(|name| hub_client::valid_device_id(name))
        .filter(|name| {
            !devices
                .iter()
                .any(|device| device.platform == DevicePlatform::Android && device.name == *name)
        })
        .map(|name| zeron_proto::device_hub::HubDevice {
            host_id: LOCAL_DEVICE_HOST_ID.into(),
            id: name.into(),
            platform: DevicePlatform::Android,
            name: name.into(),
            version: "Android".into(),
            booted: false,
            physical: false,
        })
        .collect()
}

enum HubWatch {
    Alive,
    Gone,
    Exited(Duration),
}

struct OnDrop<F: FnOnce()>(Option<F>);

impl<F: FnOnce()> Drop for OnDrop<F> {
    fn drop(&mut self) {
        if let Some(run) = self.0.take() {
            run();
        }
    }
}

impl DeviceHub {
    pub fn start(data_dir: &Path) -> Arc<Self> {
        let settings: DeviceHubSettings = std::fs::read_to_string(data_dir.join(SETTINGS_FILE))
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default();
        let agent_on = settings.enabled && settings.agent_access_enabled;
        let state = DeviceHubState {
            host_status: if settings.enabled {
                DeviceHostStatus::Idle
            } else {
                DeviceHostStatus::Disabled
            },
            agent_status: if agent_on {
                DeviceHostStatus::Idle
            } else {
                DeviceHostStatus::Disabled
            },
            settings,
            hosts: vec![host_summary(Vec::new(), false)],
            ..Default::default()
        };
        if let Some(entry) = toolchain::installed_hub() {
            hub_process::reap_stale(&data_dir.join("device"), &entry);
        }
        if !agent_on {
            agent::remove_agent_files(&data_dir.join("device"));
            zeron_harness::clear_child_path_prefix(&data_dir.join("device").join("bin"));
        }
        Arc::new_cyclic(|me| Self {
            me: me.clone(),
            data_dir: data_dir.to_path_buf(),
            http: reqwest::Client::builder()
                .no_proxy()
                .build()
                .unwrap_or_default(),
            state: watch::channel(state).0,
            settings_lock: tokio::sync::Mutex::new(()),
            lifecycle: tokio::sync::Mutex::new(()),
            hub: Mutex::new(None),
            backoff: Mutex::default(),
            restart: Mutex::new(None),
            closed: AtomicBool::new(false),
            agent: Mutex::new(None),
            agent_start: tokio::sync::Mutex::new(()),
            action_locks: Mutex::default(),
            #[cfg(test)]
            test_spawn: Mutex::new(None),
            #[cfg(test)]
            test_actions: Mutex::new(None),
            #[cfg(test)]
            test_agent: Mutex::new(None),
            #[cfg(test)]
            test_platforms: Mutex::new(None),
            streams: Arc::new(DeviceStreams::with_sink({
                let me = me.clone();
                move |streams| {
                    if let Some(hub) = me.upgrade() {
                        hub.state.send_if_modified(|state| {
                            let changed = state.streams != streams;
                            if changed {
                                state.streams = streams;
                                state.revision += 1;
                            }
                            changed
                        });
                    }
                }
            })),
        })
    }

    pub fn watch(&self) -> watch::Receiver<DeviceHubState> {
        self.state.subscribe()
    }

    pub fn snapshot(&self) -> DeviceHubState {
        self.state.borrow().clone()
    }

    fn enabled(&self) -> bool {
        self.state.borrow().settings.enabled
    }

    async fn detect_platforms(&self) -> Vec<DevicePlatformAvailability> {
        #[cfg(test)]
        if let Some(platforms) = self
            .test_platforms
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
        {
            return platforms;
        }
        tokio::task::spawn_blocking(platform::detect)
            .await
            .unwrap_or_default()
    }

    fn platform_of(&self, device_id: &str) -> DevicePlatform {
        let state = self.state.borrow();
        state
            .devices
            .iter()
            .find(|device| device.id == device_id)
            .map(|device| device.platform)
            .or_else(|| {
                state
                    .sessions
                    .iter()
                    .find(|session| session.device_id == device_id)
                    .map(|session| session.platform)
            })
            .unwrap_or_default()
    }

    fn publish(&self, change: impl FnOnce(&mut DeviceHubState)) {
        self.state.send_modify(|state| {
            change(state);
            if !state.settings.enabled {
                state.host_status = DeviceHostStatus::Disabled;
                state.host_status_detail = None;
            }
            if !(state.settings.enabled && state.settings.agent_access_enabled) {
                state.agent_status = DeviceHostStatus::Disabled;
                state.agent_status_detail = None;
            } else if state.agent_status == DeviceHostStatus::Disabled {
                state.agent_status = DeviceHostStatus::Idle;
            }
            state.revision += 1;
        });
    }

    fn set_status(&self, status: DeviceHostStatus, detail: Option<String>) {
        self.publish(|state| {
            state.host_status = status;
            state.host_status_detail = detail;
        });
    }

    fn device_dir(&self) -> PathBuf {
        self.data_dir.join("device")
    }

    fn hub_slot(&self) -> std::sync::MutexGuard<'_, Option<HubProcess>> {
        self.hub.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn restart_slot(&self) -> std::sync::MutexGuard<'_, Option<tokio::task::JoinHandle<()>>> {
        self.restart.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn live_origin(&self) -> Option<String> {
        let mut slot = self.hub_slot();
        let hub = slot.as_mut()?;
        hub.alive().then(|| hub.origin.clone())
    }

    fn take_dead(&self) -> Option<HubProcess> {
        let mut slot = self.hub_slot();
        if slot.as_mut().is_none_or(HubProcess::alive) {
            return None;
        }
        self.streams.pause();
        slot.take()
    }

    fn watch_hub(&self, origin: &str) -> HubWatch {
        let mut slot = self.hub_slot();
        let Some(hub) = slot.as_mut().filter(|hub| hub.origin == origin) else {
            return HubWatch::Gone;
        };
        if hub.alive() {
            return HubWatch::Alive;
        }
        let uptime = hub.uptime();
        self.streams.pause();
        let dead = slot.take();
        drop(slot);
        drop(dead);
        HubWatch::Exited(uptime)
    }

    fn adopt(&self, hub: HubProcess) -> Result<String, HubProcess> {
        let origin = hub.origin.clone();
        {
            let mut slot = self.hub_slot();
            if !self.enabled() || self.closed.load(Ordering::SeqCst) {
                return Err(hub);
            }
            *slot = Some(hub);
        }
        let me = self.me.clone();
        let watched = origin.clone();
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(HUB_POLL).await;
                let Some(hub) = me.upgrade() else {
                    return;
                };
                match hub.watch_hub(&watched) {
                    HubWatch::Alive => {}
                    HubWatch::Gone => return,
                    HubWatch::Exited(uptime) => {
                        hub.restart_after_exit(uptime);
                        return;
                    }
                }
            }
        });
        Ok(origin)
    }

    fn restart_after_exit(&self, uptime: Duration) {
        let stopped = running_tools(false);
        let mut slot = self.restart_slot();
        if !self.enabled() || self.closed.load(Ordering::SeqCst) {
            return;
        }
        tracing::warn!("the device hub exited");
        self.publish(|state| {
            state.host_status = DeviceHostStatus::Starting;
            state.host_status_detail = None;
            stopped(state);
        });
        let me = self.me.clone();
        let task = tokio::spawn(async move {
            let mut uptime = uptime;
            let mut failures = 0;
            loop {
                let Some(delay) = me.upgrade().map(|hub| {
                    hub.backoff
                        .lock()
                        .unwrap_or_else(PoisonError::into_inner)
                        .after_exit(uptime)
                }) else {
                    return;
                };
                tracing::info!(
                    delay_ms = delay.as_millis() as u64,
                    "restarting the device hub"
                );
                tokio::time::sleep(delay).await;
                let Some(hub) = me.upgrade() else {
                    return;
                };
                let error = match hub.start_hub(true).await {
                    Ok(_) => return,
                    Err(error) if error == HUB_NOT_RUNNING => return,
                    Err(error) => error,
                };
                failures += 1;
                if !hub.enabled() || hub.closed.load(Ordering::SeqCst) {
                    return;
                }
                if failures >= MAX_FAILED_RESTARTS {
                    tracing::warn!(%error, "the device hub could not be restarted");
                    hub.set_status(DeviceHostStatus::Failed, Some(error));
                    return;
                }
                uptime = Duration::ZERO;
            }
        });
        if let Some(previous) = slot.replace(task) {
            previous.abort();
        }
    }

    async fn reattach(&self, origin: &str) -> Result<(), String> {
        let before = self.snapshot();
        let mut ids: std::collections::BTreeSet<String> = before
            .sessions
            .iter()
            .map(|session| session.device_id.clone())
            .collect();
        ids.extend(self.streams.device_ids());
        if ids.is_empty() {
            return Ok(());
        }
        if let Err(error) = self.refresh(origin).await {
            tracing::warn!(%error, "device list unavailable after the hub restarted");
        } else {
            self.publish(|state| {
                let booted: Vec<String> = state
                    .devices
                    .iter()
                    .filter(|device| device.booted)
                    .map(|device| device.id.clone())
                    .collect();
                state
                    .sessions
                    .retain(|session| booted.contains(&session.device_id));
            });
        }
        let client = HubClient {
            http: &self.http,
            origin,
        };
        let state = self.snapshot();
        for id in &ids {
            if state.devices.iter().any(|device| {
                &device.id == id && device.booted && device.platform == DevicePlatform::Ios
            }) && let Err(error) = client.attach_stream(id).await
            {
                tracing::warn!(%error, "reattaching a device stream failed");
            }
        }
        let slot = self.hub_slot();
        if slot.as_ref().map(|hub| hub.origin.as_str()) != Some(origin) {
            return Err(HUB_NOT_RUNNING.into());
        }
        self.streams.resume(
            |device_id, frames, link| self.start_reader(origin, device_id, frames, link),
            |device_id, feed, link| self.start_avcc_reader(origin, device_id, feed, link),
        );
        drop(slot);
        Ok(())
    }

    fn known_device(&self, device_id: &str) -> bool {
        let state = self.state.borrow();
        hub_client::valid_device_id(device_id)
            && (state.devices.iter().any(|device| device.id == device_id)
                || state
                    .sessions
                    .iter()
                    .any(|session| session.device_id == device_id))
    }

    pub async fn configure(
        &self,
        params: ConfigureDeviceHubParams,
    ) -> Result<DeviceHubState, String> {
        let settings_lock = self.settings_lock.lock().await;
        let mut settings = self.state.borrow().settings.clone();
        if let Some(enabled) = params.enabled {
            settings.enabled = enabled;
        }
        if let Some(agent) = params.agent_access_enabled {
            settings.agent_access_enabled = agent;
        }
        if let Some(done) = params.onboarding_completed {
            settings.onboarding_completed = done;
        }
        if !settings.enabled {
            settings.agent_access_enabled = false;
        }
        let json = serde_json::to_string_pretty(&settings).map_err(|e| e.to_string())?;
        std::fs::create_dir_all(&self.data_dir).map_err(|e| e.to_string())?;
        std::fs::write(self.data_dir.join(SETTINGS_FILE), json)
            .map_err(|e| format!("Could not save device settings: {e}"))?;
        let enabled = settings.enabled;
        let agent_on = settings.agent_access_enabled;
        let stopped = running_tools(false);
        self.publish(|state| {
            state.settings = settings;
            state.host_status = if enabled {
                DeviceHostStatus::Idle
            } else {
                DeviceHostStatus::Disabled
            };
            state.host_status_detail = None;
            if !enabled {
                state.devices.clear();
                state.sessions.clear();
                state.booting.clear();
                state.screens.clear();
                stopped(state);
            }
        });
        if !agent_on {
            self.stop_agent().await;
        }
        if !enabled {
            self.stop_hub().await;
        }
        drop(settings_lock);
        if enabled {
            self.list(false).await?;
        }
        if agent_on
            && params.agent_access_enabled == Some(true)
            && let Err(error) = self.ensure_agent().await
        {
            tracing::warn!(%error, "agent-device could not be prepared");
        }
        Ok(self.snapshot())
    }

    pub async fn list(&self, inspect_only: bool) -> Result<DeviceHubState, String> {
        if !inspect_only && !self.enabled() {
            return Err(DEVICE_SUPPORT_OFF.into());
        }
        let platforms = self.detect_platforms().await;
        let host = host_summary(platforms, self.live_origin().is_some());
        let any_available = host.platforms.iter().any(|platform| platform.available);
        self.publish(|state| state.hosts = vec![host]);
        if inspect_only || !any_available {
            return Ok(self.snapshot());
        }
        if let Ok(origin) = self.ensure_ready().await
            && let Err(error) = self.refresh(&origin).await
        {
            self.set_status(DeviceHostStatus::Failed, Some(error));
        }
        Ok(self.snapshot())
    }

    async fn ensure_ready(&self) -> Result<String, String> {
        self.start_hub(false).await
    }

    async fn start_hub(&self, restart: bool) -> Result<String, String> {
        let lifecycle = self.lifecycle.lock().await;
        if let Some(origin) = self.live_origin() {
            return Ok(origin);
        }
        if !self.enabled() || self.closed.load(Ordering::SeqCst) {
            return Err(DEVICE_SUPPORT_OFF.into());
        }
        drop(self.take_dead());
        let fail = |error: String| {
            if !restart {
                self.set_status(DeviceHostStatus::Failed, Some(error.clone()));
            }
            error
        };
        let settle = (!restart).then(|| {
            OnDrop(Some(|| {
                self.state.send_if_modified(|state| {
                    let pending = matches!(
                        state.host_status,
                        DeviceHostStatus::Installing | DeviceHostStatus::Starting
                    );
                    if pending {
                        state.host_status = DeviceHostStatus::Idle;
                        state.revision += 1;
                    }
                    pending
                });
            }))
        });
        let hub = self.spawn_hub().await.map_err(fail)?;
        let origin = match self.adopt(hub) {
            Ok(origin) => origin,
            Err(hub) => {
                hub.stop().await;
                return Err(DEVICE_SUPPORT_OFF.into());
            }
        };
        if !restart {
            self.backoff
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .reset();
        }
        let running = running_tools(true);
        self.publish(|state| {
            state.host_status = DeviceHostStatus::Ready;
            state.host_status_detail = None;
            running(state);
        });
        drop((settle, lifecycle));
        self.reattach(&origin).await?;
        Ok(origin)
    }

    async fn spawn_hub(&self) -> Result<HubProcess, String> {
        #[cfg(test)]
        if let Some(spawn) = self
            .test_spawn
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .as_ref()
        {
            self.set_status(DeviceHostStatus::Starting, None);
            return spawn();
        }
        if !toolchain::hub_installed() {
            self.set_status(DeviceHostStatus::Installing, None);
        }
        let entry = toolchain::ensure_hub().await?;
        self.set_status(DeviceHostStatus::Starting, None);
        let (program, args) = toolchain::launch(&entry)?;
        let args = hub_process::guarded_args(&self.device_dir(), &args)?;
        let mut env = Vec::new();
        if let Some(root) = tokio::task::spawn_blocking(platform::android_root)
            .await
            .ok()
            .flatten()
        {
            env = platform::android_env(&root, std::env::var_os("PATH").as_deref());
            // Hub 0.12.0 boots AVDs with `-camera-* imagefile:`, which emulator 36.4 rejects; an uncreatable feed dir makes it leave those flags out.
            env.push(("SERVE_EMU_CAMERA_DIR", "/dev/null/zeron-no-camera".into()));
        }
        hub_process::spawn(
            &self.http,
            &program,
            &args,
            &entry,
            &self.device_dir(),
            hub_process::READY_TIMEOUT,
            env,
        )
        .await
    }

    async fn refresh(&self, origin: &str) -> Result<(), String> {
        let (mut devices, errors) = HubClient {
            http: &self.http,
            origin,
        }
        .list()
        .await?;
        let android = {
            let state = self.state.borrow();
            let platforms: Vec<_> = state
                .hosts
                .iter()
                .flat_map(|host| host.platforms.clone())
                .collect();
            available(&platforms, DevicePlatform::Android)
        };
        if android {
            let avds = self.list_avds().await;
            devices.extend(stopped_avds(&avds, &devices));
        }
        self.publish(|state| {
            if !state.settings.enabled {
                return;
            }
            state.devices = devices;
            state.host_status = DeviceHostStatus::Ready;
            state.host_status_detail = host_diagnostics(errors, android);
        });
        Ok(())
    }

    pub async fn open(&self, params: OpenHubDeviceParams) -> Result<DeviceSession, String> {
        if !self.enabled() {
            return Err(DEVICE_SUPPORT_OFF.into());
        }
        if !hub_client::valid_device_id(&params.device_id) {
            return Err(UNKNOWN_DEVICE.into());
        }
        let support = self
            .detect_platforms()
            .await
            .into_iter()
            .find(|candidate| candidate.platform == params.platform)
            .unwrap_or_default();
        if !support.available {
            return Err(support.reason.unwrap_or_default());
        }
        let host_id = params
            .host_id
            .unwrap_or_else(|| LOCAL_DEVICE_HOST_ID.into());
        let origin = self.ensure_ready().await?;
        self.refresh(&origin).await?;
        let device = self
            .snapshot()
            .device(&host_id, &params.device_id)
            .cloned()
            .ok_or_else(|| UNKNOWN_DEVICE.to_string())?;
        if device.platform != params.platform {
            return Err(UNKNOWN_DEVICE.into());
        }
        let ios = device.platform == DevicePlatform::Ios;
        let client = HubClient {
            http: &self.http,
            origin: &origin,
        };
        let device_id = if !device.booted && params.boot {
            let booting = DeviceBooting {
                device: device.clone(),
                chat_id: params.chat_id.clone(),
            };
            self.publish(|state| {
                state.booting.retain(|entry| entry.device.id != device.id);
                state.booting.push(booting);
            });
            let booted = {
                let _clear = OnDrop(Some(|| {
                    self.publish(|state| state.booting.retain(|entry| entry.device.id != device.id))
                }));
                async {
                    let id = client.boot(&device).await?;
                    if !hub_client::valid_device_id(&id) {
                        return Err(UNKNOWN_DEVICE.to_string());
                    }
                    if ios {
                        client.attach_stream(&id).await?;
                    }
                    Ok::<_, String>(id)
                }
                .await
            };
            let id = booted?;
            self.refresh(&origin).await?;
            id
        } else {
            if device.booted && ios {
                client.attach_stream(&device.id).await?;
            }
            device.id.clone()
        };
        if !self.enabled() {
            return Err("Device support was turned off while the device was opening.".into());
        }
        let session = DeviceSession {
            chat_id: params.chat_id,
            host_id,
            device_id,
            platform: device.platform,
            opened_at: chrono::Utc::now().to_rfc3339(),
        };
        let added = session.clone();
        self.publish(|state| {
            state.sessions.retain(|existing| {
                !(existing.chat_id == added.chat_id
                    && existing.host_id == added.host_id
                    && existing.device_id == added.device_id)
            });
            state.sessions.push(added);
        });
        Ok(session)
    }

    pub(crate) fn watch_frames(&self, device_id: &str) -> Result<FrameSubscription, String> {
        if self.platform_of(device_id) == DevicePlatform::Android {
            return Err(ANDROID_H264_ONLY.into());
        }
        self.with_live_hub(device_id, |origin| {
            self.streams.subscribe(device_id, |frames, link| {
                self.start_reader(origin, device_id, frames, link)
            })
        })
    }

    pub(crate) fn watch_avcc(&self, device_id: &str) -> Result<AvccSubscription, String> {
        self.with_live_hub(device_id, |origin| {
            self.streams.subscribe_avcc(device_id, |feed, link| {
                self.start_avcc_reader(origin, device_id, feed, link)
            })
        })
    }

    fn with_live_hub<T>(
        &self,
        device_id: &str,
        subscribe: impl FnOnce(&str) -> T,
    ) -> Result<T, String> {
        if !self.enabled() {
            return Err(DEVICE_SUPPORT_OFF.into());
        }
        if !self.known_device(device_id) {
            return Err(UNKNOWN_DEVICE.into());
        }
        let origin = self
            .live_origin()
            .ok_or_else(|| HUB_NOT_RUNNING.to_string())?;
        let slot = self.hub_slot();
        if slot.as_ref().map(|hub| hub.origin.as_str()) != Some(origin.as_str()) {
            return Err(HUB_NOT_RUNNING.into());
        }
        let subscription = subscribe(&origin);
        drop(slot);
        Ok(subscription)
    }

    fn start_reader(
        &self,
        origin: &str,
        device_id: &str,
        frames: FrameSlot,
        link: u64,
    ) -> tokio::task::JoinHandle<()> {
        let client = HubClient {
            http: &self.http,
            origin,
        };
        let (on_status, on_first_frame) = self.link_callbacks(
            device_id,
            link,
            DeviceVideoCodec::Mjpeg,
            client.input_url(device_id),
        );
        stream::spawn_reader(
            self.http.clone(),
            client.mjpeg_url(device_id),
            frames,
            on_status,
            on_first_frame,
        )
    }

    fn start_avcc_reader(
        &self,
        origin: &str,
        device_id: &str,
        feed: Arc<AvccFeed>,
        link: u64,
    ) -> tokio::task::JoinHandle<()> {
        let client = HubClient {
            http: &self.http,
            origin,
        };
        if self.platform_of(device_id) == DevicePlatform::Android {
            return self.start_semu_reader(client.semu_url(device_id), device_id, feed, link);
        }
        let (on_status, on_first_frame) = self.link_callbacks(
            device_id,
            link,
            DeviceVideoCodec::H264,
            client.input_url(device_id),
        );
        stream::spawn_avcc_reader(
            self.http.clone(),
            client.avcc_url(device_id),
            feed,
            on_status,
            on_first_frame,
        )
    }

    fn start_semu_reader(
        &self,
        url: String,
        device_id: &str,
        feed: Arc<AvccFeed>,
        link: u64,
    ) -> tokio::task::JoinHandle<()> {
        let (controls, receiver) = tokio::sync::mpsc::unbounded_channel();
        let live = Arc::new(AtomicBool::new(false));
        let (me, status_hub) = (self.me.clone(), self.me.clone());
        let (id, status_id) = (device_id.to_string(), device_id.to_string());
        let input_live = live.clone();
        stream::spawn_semu_reader(
            url,
            feed,
            receiver,
            live,
            move |status| {
                if let Some(hub) = status_hub.upgrade() {
                    hub.streams.set_status(
                        &status_id,
                        link,
                        Some((DeviceVideoCodec::H264, status.clone())),
                        Some(status),
                    );
                }
            },
            move || {
                if let Some(hub) = me.upgrade() {
                    let socket = input::InputSocket::semu(controls.clone(), input_live.clone());
                    hub.streams.set_input(&id, link, socket);
                }
            },
        )
    }

    fn link_callbacks(
        &self,
        device_id: &str,
        link: u64,
        codec: DeviceVideoCodec,
        input: String,
    ) -> (
        impl Fn(StreamLinkStatus) + Send + 'static,
        impl Fn() + Send + 'static,
    ) {
        let (me, status_hub) = (self.me.clone(), self.me.clone());
        let (id, status_id) = (device_id.to_string(), device_id.to_string());
        (
            move |status| {
                if let Some(hub) = status_hub.upgrade() {
                    hub.streams
                        .set_status(&status_id, link, Some((codec, status)), None);
                }
            },
            move || {
                if let Some(hub) = me.upgrade() {
                    hub.open_input(&id, link, &input);
                }
            },
        )
    }

    fn open_input(&self, device_id: &str, link: u64, url: &str) {
        if self.streams.has_input(device_id) {
            return;
        }
        let (me, status_hub) = (self.me.clone(), self.me.clone());
        let (id, status_id) = (device_id.to_string(), device_id.to_string());
        let socket = input::InputSocket::connect(
            url.to_string(),
            move |screen| {
                if let Some(hub) = me.upgrade() {
                    hub.publish(|state| {
                        state.screens.insert(id.clone(), screen);
                    });
                }
            },
            move |status| {
                if let Some(hub) = status_hub.upgrade() {
                    hub.streams.set_status(&status_id, link, None, Some(status));
                }
            },
        );
        self.streams.set_input(device_id, link, socket);
    }

    pub fn send_input(&self, device_id: &str, input: &DeviceInput) -> Result<(), String> {
        if !self.known_device(device_id) {
            return Err(UNKNOWN_DEVICE.into());
        }
        self.streams.send_input(device_id, input)
    }

    pub async fn close(&self, params: CloseHubDeviceParams) -> Result<(), String> {
        let closes = |session: &DeviceSession| {
            session.chat_id == params.chat_id
                && params
                    .host_id
                    .as_deref()
                    .is_none_or(|host| session.host_id == host)
                && params
                    .device_id
                    .as_deref()
                    .is_none_or(|device| session.device_id == device)
        };
        let closing: Vec<DeviceSession> = self
            .state
            .borrow()
            .sessions
            .iter()
            .filter(|session| closes(session))
            .cloned()
            .collect();
        if !params.shutdown {
            if !closing.is_empty() {
                self.publish(|state| state.sessions.retain(|session| !closes(session)));
            }
            return Ok(());
        }
        let mut targets: Vec<(String, DevicePlatform)> = closing
            .into_iter()
            .map(|session| (session.device_id, session.platform))
            .collect();
        if targets.is_empty()
            && let Some(device_id) = params.device_id
        {
            let host_id = params.host_id.as_deref().unwrap_or(LOCAL_DEVICE_HOST_ID);
            let platform = self
                .snapshot()
                .device(host_id, &device_id)
                .map(|device| device.platform)
                .ok_or_else(|| UNKNOWN_DEVICE.to_string())?;
            targets.push((device_id, platform));
        }
        for (device_id, platform) in targets {
            self.shutdown_device(&device_id, platform).await?;
        }
        Ok(())
    }

    pub async fn power_off(&self, params: ShutdownHubDeviceParams) -> Result<(), String> {
        self.shutdown_device(&params.device_id, params.platform)
            .await
    }

    async fn shutdown_device(
        &self,
        device_id: &str,
        platform: DevicePlatform,
    ) -> Result<(), String> {
        if !self.enabled() {
            return Err(DEVICE_SUPPORT_OFF.into());
        }
        if !self.known_device(device_id) {
            return Err(UNKNOWN_DEVICE.into());
        }
        let origin = self
            .live_origin()
            .ok_or_else(|| HUB_NOT_RUNNING.to_string())?;
        let client = HubClient {
            http: &self.http,
            origin: &origin,
        };
        let shut = match platform {
            DevicePlatform::Ios => client.shutdown_simulator(device_id).await,
            DevicePlatform::Android => client.shutdown_emulator(device_id).await,
        };
        // serve-sim's bare `simctl shutdown` fails for a simulator that is already off.
        if let Err(error) = shut {
            let (devices, _) = client.list().await.map_err(|_| error.clone())?;
            let off = match platform {
                DevicePlatform::Ios => devices
                    .iter()
                    .any(|device| device.id == device_id && !device.booted),
                DevicePlatform::Android => !devices
                    .iter()
                    .any(|device| device.id == device_id && device.booted),
            };
            if !off {
                return Err(error);
            }
        }
        if let Err(error) = self.refresh(&origin).await {
            tracing::warn!(%error, "device list unavailable after shutdown");
        }
        self.publish(|state| {
            for device in state.devices.iter_mut().filter(|d| d.id == device_id) {
                device.booted = false;
            }
            state
                .sessions
                .retain(|session| session.device_id != device_id);
        });
        Ok(())
    }

    async fn stop_hub(&self) {
        if let Some(restart) = self.restart_slot().take() {
            restart.abort();
        }
        let hub = {
            let mut slot = self.hub_slot();
            self.streams.clear();
            slot.take()
        };
        if let Some(hub) = hub {
            hub.stop().await;
        }
    }

    pub fn signal_stop(&self) {
        self.closed.store(true, Ordering::SeqCst);
        if let Some(restart) = self.restart_slot().take() {
            restart.abort();
        }
        if let Some(hub) = self.hub_slot().as_ref() {
            hub.signal_stop();
        }
        if let Some(daemon) = self.agent_slot().as_ref() {
            daemon.signal_stop();
        }
    }

    pub async fn shutdown(&self) {
        self.closed.store(true, Ordering::SeqCst);
        self.stop_hub().await;
        let daemon = self.agent_slot().take();
        if let Some(daemon) = daemon {
            daemon.stop().await;
        }
    }

    #[cfg(test)]
    pub(crate) fn attach_hub_for_test(&self, origin: String) {
        *self.hub_slot() = Some(HubProcess::external(origin));
    }

    #[cfg(test)]
    pub(crate) fn use_platforms_for_test(&self, platforms: Vec<DevicePlatformAvailability>) {
        *self
            .test_platforms
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Some(platforms);
    }

    #[cfg(test)]
    pub(crate) fn spawn_hubs_for_test(&self, spawn: TestSpawn) {
        *self
            .test_spawn
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Some(spawn);
    }
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use std::sync::Arc;
    use std::time::Duration;

    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use zeron_proto::device_hub::{DeviceHubState, DeviceInput};
    use zeron_rpc::{RpcReply, RpcService, methods};

    type Requests = Arc<std::sync::Mutex<Vec<String>>>;

    async fn read_request(socket: &mut tokio::net::TcpStream) -> String {
        let mut buf = Vec::new();
        let mut chunk = [0u8; 4096];
        loop {
            let n = socket.read(&mut chunk).await.unwrap_or(0);
            if n == 0 {
                break;
            }
            buf.extend_from_slice(&chunk[..n]);
            let text = String::from_utf8_lossy(&buf).to_string();
            if let Some(end) = text.find("\r\n\r\n") {
                let length = text[..end]
                    .lines()
                    .find_map(|line| {
                        let (name, value) = line.split_once(':')?;
                        name.eq_ignore_ascii_case("content-length")
                            .then(|| value.trim().parse::<usize>().ok())
                            .flatten()
                    })
                    .unwrap_or(0);
                if buf.len() >= end + 4 + length {
                    break;
                }
            }
        }
        String::from_utf8_lossy(&buf).to_string()
    }

    #[derive(Clone, Copy)]
    pub(super) struct Fake {
        list_status: u16,
        booted: bool,
        boot_status: Option<u16>,
        shutdown_status: u16,
        shutdown_flips: bool,
        frame: &'static str,
        attach_delay: Duration,
        stall_app_state: bool,
        huge_ax: bool,
    }

    const AVCC_RECORD: &[u8] = &[1, 0x64, 0, 0x20, 0xff, 0xe1, 0, 1, 0x67, 1, 0, 1, 0x68];

    fn avcc_envelope(tag: u8, payload: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        zeron_proto::device_hub::AvccEnvelope {
            tag,
            payload: payload.to_vec(),
        }
        .encode_into(&mut out);
        out
    }

    pub(super) const HEALTHY: Fake = Fake {
        list_status: 200,
        booted: true,
        boot_status: Some(200),
        shutdown_status: 200,
        shutdown_flips: true,
        frame: "frame",
        attach_delay: Duration::ZERO,
        stall_app_state: false,
        huge_ax: false,
    };

    pub(super) const STALLED_APP_STATE: Fake = Fake {
        stall_app_state: true,
        ..HEALTHY
    };

    pub(super) const HUGE_AX: Fake = Fake {
        huge_ax: true,
        ..HEALTHY
    };

    pub(super) async fn fake_hub() -> (String, Requests) {
        fake_hub_with(HEALTHY).await
    }

    const ANDROID_CAPTURE: &[u8] =
        include_bytes!("../../../proto/tests/fixtures/device-android-576x1280.semu");

    fn android_capture() -> Vec<tokio_tungstenite::tungstenite::Message> {
        use tokio_tungstenite::tungstenite::Message;
        let mut messages = Vec::new();
        let mut rest = ANDROID_CAPTURE;
        while let [kind, a, b, c, d, tail @ ..] = rest {
            let (payload, next) = tail.split_at(u32::from_be_bytes([*a, *b, *c, *d]) as usize);
            messages.push(match kind {
                0 => Message::Binary(payload.to_vec()),
                _ => Message::Text(String::from_utf8(payload.to_vec()).unwrap()),
            });
            rest = next;
        }
        messages
    }

    async fn serve_semu(socket: tokio::net::TcpStream, seen: Requests) {
        use futures::{SinkExt, StreamExt};
        use tokio_tungstenite::tungstenite::{Message, handshake::server};
        let uri: Requests = Arc::default();
        let captured = uri.clone();
        let Ok(mut ws) = tokio_tungstenite::accept_hdr_async(
            socket,
            move |request: &server::Request, response: server::Response| {
                captured.lock().unwrap().push(request.uri().to_string());
                Ok(response)
            },
        )
        .await
        else {
            return;
        };
        seen.lock()
            .unwrap()
            .push(format!("WS {}", uri.lock().unwrap().concat()));
        for message in android_capture() {
            if ws.send(message).await.is_err() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        while let Some(Ok(message)) = ws.next().await {
            if let Message::Text(text) = message {
                seen.lock().unwrap().push(format!("WS {text}"));
            }
        }
    }

    async fn fake_hub_with(fake: Fake) -> (String, Requests) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let requests: Requests = Arc::default();
        let seen = requests.clone();
        let booted = Arc::new(std::sync::atomic::AtomicBool::new(fake.booted));
        let emulator = Arc::new(std::sync::atomic::AtomicBool::new(false));
        tokio::spawn(async move {
            loop {
                let (mut socket, _) = listener.accept().await.unwrap();
                let seen = seen.clone();
                let booted = booted.clone();
                let emulator = emulator.clone();
                tokio::spawn(async move {
                    use std::sync::atomic::Ordering::SeqCst;
                    let mut head = [0u8; 32];
                    let n = socket.peek(&mut head).await.unwrap_or(0);
                    if head[..n].starts_with(b"GET /vendor/serve-emu/ws") {
                        serve_semu(socket, seen).await;
                        return;
                    }
                    let request = read_request(&mut socket).await;
                    let line = request.lines().next().unwrap_or_default().to_string();
                    let body = request.split("\r\n\r\n").nth(1).unwrap_or_default();
                    seen.lock().unwrap().push(format!("{line} {body}"));
                    if line.starts_with("GET /vendor/serve-sim/helper/SIM-1/stream.mjpeg ") {
                        let head = "HTTP/1.1 200 OK\r\nContent-Type: multipart/x-mixed-replace; boundary=frame\r\nConnection: close\r\n\r\n";
                        let mut alive = socket.write_all(head.as_bytes()).await.is_ok();
                        while alive {
                            alive = socket
                                .write_all(&super::mjpeg::part(fake.frame.as_bytes()))
                                .await
                                .is_ok();
                            tokio::time::sleep(Duration::from_millis(20)).await;
                        }
                        return;
                    }
                    if line.starts_with("GET /vendor/serve-sim/helper/SIM-1/stream.avcc ") {
                        let mut stream = b"HTTP/1.1 200 OK\r\nContent-Type: application/octet-stream\r\nConnection: close\r\n\r\n".to_vec();
                        for (tag, payload) in [(4, &b"seed"[..]), (1, AVCC_RECORD), (2, b"key")] {
                            stream.extend(avcc_envelope(tag, payload));
                        }
                        let mut alive = socket.write_all(&stream).await.is_ok();
                        let mut delta = 0u32;
                        while alive {
                            delta += 1;
                            alive = socket
                                .write_all(&avcc_envelope(3, &delta.to_be_bytes()))
                                .await
                                .is_ok();
                            tokio::time::sleep(Duration::from_millis(20)).await;
                        }
                        return;
                    }
                    let app_state =
                        line.starts_with("GET /vendor/serve-sim/appstate?device=SIM-1 ");
                    if app_state && fake.stall_app_state {
                        std::future::pending::<()>().await;
                    }
                    let sse = if app_state {
                        Some(
                            ": \n\ndata: {\"bundleId\":\"com.apple.Maps\",\"pid\":7,\"isReactNative\":false}\n\n",
                        )
                    } else if line.starts_with(
                        "GET /vendor/serve-sim/api/event-log/events?device=SIM-1&limit=100 ",
                    ) {
                        Some(
                            ": \n\ndata: {\"events\":[{\"id\":1,\"timestamp\":\"t1\",\"kind\":\"touch\",\"summary\":\"Touch begin\"}]}\n\ndata: {\"event\":{\"id\":2,\"timestamp\":\"t2\",\"kind\":\"button\",\"msg\":\"Button home\"}}\n\n",
                        )
                    } else {
                        None
                    };
                    if let Some(events) = sse {
                        let head = "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\n";
                        let _ = socket.write_all(format!("{head}{events}").as_bytes()).await;
                        std::future::pending::<()>().await;
                    }
                    let (status, body) = if line
                        .starts_with("GET /vendor/serve-sim/helper/SIM-1/ax ")
                    {
                        let pad = if fake.huge_ax {
                            "x".repeat(super::hub_client::AX_BODY_LIMIT + 1)
                        } else {
                            String::new()
                        };
                        let body = serde_json::json!([{
                            "frame": {"x": 0, "y": 0, "width": 402, "height": 874},
                            "AXValue": pad,
                            "AXLabel": "SpringBoard", "type": "Application",
                            "children": [{"frame": {"x": 20, "y": 60, "width": 64, "height": 64},
                                "AXLabel": "Maps", "AXUniqueId": "Maps", "type": "Icon"}]
                        }]);
                        (200, body)
                    } else if line.starts_with("GET /api/devices ") {
                        let emulators = if emulator.load(SeqCst) {
                            serde_json::json!([{"id": "emulator-5554", "name": "Pixel_9_API_36",
                                "version": "Android 16.0", "platform": "android", "booted": true,
                                "physical": false}])
                        } else {
                            serde_json::json!([])
                        };
                        let body = serde_json::json!({
                            "simulators": [{"id": "SIM-1", "name": "iPhone 17", "version": "iOS 27.0",
                                "platform": "ios", "booted": booted.load(SeqCst), "physical": false}],
                            "emulators": emulators
                        });
                        (fake.list_status, body)
                    } else if line.starts_with("POST /vendor/serve-sim/grid/api/shutdown ") {
                        if fake.shutdown_flips {
                            booted.store(false, SeqCst);
                        }
                        (fake.shutdown_status, serde_json::json!({"ok": true}))
                    } else if line.starts_with("POST /api/devices/boot ")
                        && body.contains(r#""platform":"android""#)
                    {
                        emulator.store(true, SeqCst);
                        (
                            200,
                            serde_json::json!({"ok": true, "id": "emulator-5554", "serial": "emulator-5554", "errors": []}),
                        )
                    } else if line.starts_with("POST /api/devices/shutdown ") {
                        emulator.store(false, SeqCst);
                        (200, serde_json::json!({"ok": true}))
                    } else if line.starts_with("POST /api/devices/boot ") {
                        let Some(status) = fake.boot_status else {
                            std::future::pending::<()>().await;
                            return;
                        };
                        (status, serde_json::json!({"ok": true, "id": "SIM-1"}))
                    } else if line.starts_with("POST /vendor/serve-sim/grid/api/start ") {
                        tokio::time::sleep(fake.attach_delay).await;
                        (200, serde_json::json!({"ok": true}))
                    } else {
                        (200, serde_json::json!({"ok": true}))
                    };
                    let body = body.to_string();
                    let reply = format!(
                        "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                        body.len()
                    );
                    let _ = socket.write_all(reply.as_bytes()).await;
                });
            }
        });
        (origin, requests)
    }

    pub(super) async fn enabled_hub(
        fake: Fake,
    ) -> (tempfile::TempDir, Arc<super::DeviceHub>, String) {
        let dir = tempfile::tempdir().unwrap();
        let hub = super::DeviceHub::start(dir.path());
        let (origin, _) = fake_hub_with(fake).await;
        hub.attach_hub_for_test(origin.clone());
        hub.configure(zeron_proto::device_hub::ConfigureDeviceHubParams {
            enabled: Some(true),
            ..Default::default()
        })
        .await
        .unwrap();
        (dir, hub, origin)
    }

    fn open_params(device_id: &str) -> zeron_proto::device_hub::OpenHubDeviceParams {
        serde_json::from_value(
            serde_json::json!({"chatId": "chat-1", "deviceId": device_id, "platform": "ios"}),
        )
        .unwrap()
    }

    pub(super) async fn eventually(mut done: impl FnMut() -> bool) {
        for _ in 0..500 {
            if done() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        panic!("condition never held");
    }

    async fn next_state(
        stream: &mut futures::stream::BoxStream<'static, serde_json::Value>,
    ) -> DeviceHubState {
        use futures::StreamExt;
        let value = tokio::time::timeout(Duration::from_secs(5), stream.next())
            .await
            .expect("state item")
            .expect("stream open");
        serde_json::from_value(value).unwrap()
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn open_registers_a_session_and_the_watch_sees_the_revision_bump() {
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
        let rpc = core.rpc_service();
        rpc.handle(
            methods::CONFIGURE_DEVICE_HUB,
            serde_json::json!({"enabled": true, "onboardingCompleted": true}),
        )
        .await
        .unwrap();
        let Ok(RpcReply::Stream(mut watch)) = rpc
            .handle(methods::WATCH_DEVICE_HUB, serde_json::json!({}))
            .await
        else {
            panic!("WatchDeviceHub streams");
        };
        let before = next_state(&mut watch).await;
        assert!(before.settings.enabled);
        assert_eq!(before.devices.len(), 1);
        assert!(before.sessions.is_empty());

        let Ok(RpcReply::Value(session)) = rpc
            .handle(
                methods::OPEN_HUB_DEVICE,
                serde_json::json!({"chatId": "chat-1", "deviceId": "SIM-1", "platform": "ios"}),
            )
            .await
        else {
            panic!("OpenHubDevice answers");
        };
        assert_eq!(session["deviceId"], "SIM-1");
        assert_eq!(session["chatId"], "chat-1");
        let after = loop {
            let state = next_state(&mut watch).await;
            if !state.sessions.is_empty() {
                break state;
            }
        };
        assert!(after.revision > before.revision);
        assert!(after.session("chat-1", "local", "SIM-1").is_some());
        let requests = requests.lock().unwrap().clone();
        assert!(
            requests
                .iter()
                .any(|line| line.starts_with("POST /vendor/serve-sim/grid/api/start ")),
            "a booted simulator still gets a helper attached: {requests:?}"
        );
        assert!(
            requests
                .iter()
                .all(|line| !line.contains(" /vendor/serve-sim ")
                    && !line.contains(" /vendor/serve-sim/ ")
                    && !line.contains("exec-ws")),
            "{requests:?}"
        );
        core.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn close_keeps_the_simulator_and_power_off_shuts_it_down() {
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
        let rpc = core.rpc_service();
        rpc.handle(
            methods::CONFIGURE_DEVICE_HUB,
            serde_json::json!({"enabled": true}),
        )
        .await
        .unwrap();
        let open = serde_json::json!({"chatId": "chat-1", "deviceId": "SIM-1", "platform": "ios"});
        let shutdowns = || {
            requests
                .lock()
                .unwrap()
                .iter()
                .filter(|line| line.starts_with("POST /vendor/serve-sim/grid/api/shutdown "))
                .cloned()
                .collect::<Vec<_>>()
        };
        rpc.handle(methods::OPEN_HUB_DEVICE, open.clone())
            .await
            .unwrap();
        rpc.handle(
            methods::OPEN_HUB_DEVICE,
            serde_json::json!({"chatId": "chat-2", "deviceId": "SIM-1", "platform": "ios"}),
        )
        .await
        .unwrap();
        rpc.handle(
            methods::CLOSE_HUB_DEVICE,
            serde_json::json!({"chatId": "chat-1"}),
        )
        .await
        .unwrap();
        let state = core.device_hub.snapshot();
        assert!(state.session("chat-1", "local", "SIM-1").is_none());
        assert!(state.session("chat-2", "local", "SIM-1").is_some());
        assert!(state.devices[0].booted);
        assert!(shutdowns().is_empty(), "closing alone leaves it running");

        rpc.handle(methods::OPEN_HUB_DEVICE, open).await.unwrap();
        rpc.handle(
            methods::CLOSE_HUB_DEVICE,
            serde_json::json!({"chatId": "chat-1", "hostId": "local", "deviceId": "SIM-1", "shutdown": true}),
        )
        .await
        .unwrap();
        assert_eq!(
            shutdowns(),
            vec![r#"POST /vendor/serve-sim/grid/api/shutdown HTTP/1.1 {"udid":"SIM-1"}"#]
        );
        let state = core.device_hub.snapshot();
        assert!(
            state.sessions.is_empty(),
            "power off ends every chat's session"
        );
        assert!(!state.devices[0].booted, "{state:?}");
        assert!(
            requests
                .lock()
                .unwrap()
                .last()
                .is_some_and(|line| line.starts_with("GET /api/devices ")),
            "the list is refreshed after the shutdown"
        );

        let refused = rpc
            .handle(
                methods::SHUTDOWN_HUB_DEVICE,
                serde_json::json!({"deviceId": "../SIM-1", "platform": "ios"}),
            )
            .await
            .err()
            .map(|error| error.to_string())
            .unwrap_or_default();
        assert!(refused.contains(super::UNKNOWN_DEVICE), "{refused}");
        rpc.handle(
            methods::SHUTDOWN_HUB_DEVICE,
            serde_json::json!({"deviceId": "SIM-1", "platform": "ios"}),
        )
        .await
        .unwrap();
        assert_eq!(shutdowns().len(), 2);
        core.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn power_off_ends_sessions_only_once_the_simulator_is_off() {
        let power_off = |device_id: &str| {
            serde_json::from_value::<zeron_proto::device_hub::CloseHubDeviceParams>(
                serde_json::json!({"chatId": "chat-1", "deviceId": device_id, "shutdown": true}),
            )
            .unwrap()
        };
        let (_dir, hub, _) = enabled_hub(Fake {
            shutdown_status: 500,
            shutdown_flips: false,
            ..HEALTHY
        })
        .await;
        hub.open(open_params("SIM-1")).await.unwrap();
        assert!(hub.close(power_off("SIM-1")).await.is_err());
        let state = hub.snapshot();
        assert!(state.session("chat-1", "local", "SIM-1").is_some());
        assert!(state.devices[0].booted);
        hub.shutdown().await;

        let (_dir, hub, _) = enabled_hub(Fake {
            shutdown_flips: false,
            ..HEALTHY
        })
        .await;
        hub.open(open_params("SIM-1")).await.unwrap();
        hub.close(power_off("SIM-1")).await.unwrap();
        let state = hub.snapshot();
        assert!(state.sessions.is_empty());
        assert!(
            !state.devices[0].booted,
            "a list taken mid-shutdown does not mark it booted again"
        );
        hub.shutdown().await;

        let (_dir, hub, _) = enabled_hub(HEALTHY).await;
        assert!(hub.snapshot().devices[0].booted);
        hub.close(power_off("SIM-1")).await.unwrap();
        assert!(
            !hub.snapshot().devices[0].booted,
            "power off without a session still shuts the simulator down"
        );
        assert_eq!(
            hub.close(power_off("SIM-2")).await.err().as_deref(),
            Some(super::UNKNOWN_DEVICE)
        );
        hub.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn hub_errors_never_carry_the_hub_origin() {
        let failing = Fake {
            list_status: 500,
            ..HEALTHY
        };
        let dir = tempfile::tempdir().unwrap();
        let core = crate::EngineCore::assemble(
            dir.path(),
            Arc::new(crate::HarnessRegistry::new()),
            zeron_proto::HarnessId::Mock,
            None,
        )
        .unwrap();
        let (origin, _) = fake_hub_with(failing).await;
        let port = origin.rsplit(':').next().unwrap().to_string();
        core.device_hub.attach_hub_for_test(origin);
        let rpc = core.rpc_service();
        let mut seen = Vec::new();
        let reply = |reply: Result<RpcReply, zeron_rpc::RpcError>| match reply {
            Ok(RpcReply::Value(value)) => value.to_string(),
            Ok(RpcReply::Stream(_)) => String::new(),
            Err(error) => error.to_string(),
        };
        seen.push(reply(
            rpc.handle(
                methods::CONFIGURE_DEVICE_HUB,
                serde_json::json!({"enabled": true}),
            )
            .await,
        ));
        seen.push(reply(
            rpc.handle(methods::LIST_HUB_DEVICES, serde_json::json!({}))
                .await,
        ));
        seen.push(reply(
            rpc.handle(
                methods::OPEN_HUB_DEVICE,
                serde_json::json!({"chatId": "c", "deviceId": "SIM-1", "platform": "ios"}),
            )
            .await,
        ));
        let state = core.device_hub.snapshot();
        assert_eq!(
            state.host_status,
            zeron_proto::device_hub::DeviceHostStatus::Failed
        );
        assert!(
            state
                .host_status_detail
                .as_deref()
                .is_some_and(|detail| detail.contains("500")),
            "{state:?}"
        );
        seen.push(serde_json::to_string(&state).unwrap());

        let (_dir, hub, origin) = enabled_hub(Fake {
            booted: false,
            boot_status: Some(500),
            ..HEALTHY
        })
        .await;
        let boot_port = origin.rsplit(':').next().unwrap().to_string();
        let error = hub.open(open_params("SIM-1")).await.unwrap_err();
        assert!(error.starts_with("Booting the device failed"), "{error}");
        seen.push(error);
        for text in &seen {
            assert!(
                !text.contains("127.0.0.1") && !text.contains(&port) && !text.contains(&boot_port),
                "{text}"
            );
        }
        core.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn frames_and_input_refuse_ids_that_are_not_a_known_device() {
        let (_dir, hub, _) = enabled_hub(HEALTHY).await;
        for id in ["..", ".", "ws", "SIM-2", "SIM-1/../x", "SIM-1?device=x"] {
            assert_eq!(
                hub.watch_frames(id).err().as_deref(),
                Some(super::UNKNOWN_DEVICE),
                "{id}"
            );
            assert_eq!(
                hub.send_input(
                    id,
                    &DeviceInput::Touch {
                        phase: zeron_proto::device_hub::TouchPhase::Begin,
                        x: 0.5,
                        y: 0.5,
                    }
                )
                .err()
                .as_deref(),
                Some(super::UNKNOWN_DEVICE),
                "{id}"
            );
        }
        assert_eq!(
            hub.open(open_params("../SIM-1")).await.err().as_deref(),
            Some(super::UNKNOWN_DEVICE)
        );
        assert!(hub.watch_frames("SIM-1").is_ok());
        hub.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_cancelled_open_clears_its_booting_entry() {
        let (_dir, hub, _) = enabled_hub(Fake {
            booted: false,
            boot_status: None,
            ..HEALTHY
        })
        .await;
        let opening =
            tokio::time::timeout(Duration::from_millis(300), hub.open(open_params("SIM-1"))).await;
        assert!(opening.is_err(), "boot never answers");
        assert!(hub.snapshot().booting.is_empty());
        hub.shutdown().await;
    }

    fn sleeper() -> (tokio::process::Child, i32) {
        let mut child = tokio::process::Command::new("/bin/sleep");
        child.arg("30").process_group(0).kill_on_drop(true);
        let child = child.spawn().unwrap();
        let pid = child.id().unwrap() as i32;
        (child, pid)
    }

    fn alive(pid: i32) -> bool {
        unsafe { libc::kill(pid, 0) == 0 }
    }

    fn spawn_fakes(
        hub: &super::DeviceHub,
        origins: Vec<String>,
    ) -> Arc<std::sync::Mutex<Vec<i32>>> {
        let pids: Arc<std::sync::Mutex<Vec<i32>>> = Arc::default();
        let spawned = pids.clone();
        let origins = std::sync::Mutex::new(origins.into_iter());
        hub.spawn_hubs_for_test(Box::new(move || {
            let origin = origins.lock().unwrap().next().ok_or("no fake hub left")?;
            let (child, pid) = sleeper();
            spawned.lock().unwrap().push(pid);
            Ok(super::HubProcess::with_child(origin, child))
        }));
        pids
    }

    async fn frame_from(
        frames: &mut tokio::sync::watch::Receiver<Option<Arc<super::stream::Frame>>>,
        body: &str,
    ) -> u64 {
        tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                if let Some(frame) = frames.borrow_and_update().clone()
                    && frame.jpeg == body.as_bytes()
                {
                    return frame.seq;
                }
                frames.changed().await.unwrap();
            }
        })
        .await
        .unwrap_or_else(|_| panic!("no frame from {body}"))
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn the_published_subscriber_count_follows_frame_watchers_down_to_none() {
        let dir = tempfile::tempdir().unwrap();
        let hub = super::DeviceHub::start(dir.path());
        let (origin, _) = fake_hub().await;
        let _pids = spawn_fakes(&hub, vec![origin]);
        hub.configure(zeron_proto::device_hub::ConfigureDeviceHubParams {
            enabled: Some(true),
            ..Default::default()
        })
        .await
        .unwrap();
        hub.open(open_params("SIM-1")).await.unwrap();
        let subscribers = || {
            hub.snapshot()
                .streams
                .get("SIM-1")
                .map(|status| status.subscribers)
        };
        let tab = hub.watch_frames("SIM-1").unwrap();
        let float = hub.watch_frames("SIM-1").unwrap();
        assert_eq!(subscribers(), Some(2));
        drop(tab);
        assert_eq!(subscribers(), Some(1));
        drop(float);
        assert_eq!(subscribers(), None, "no watcher, no stream entry");
        assert!(
            hub.snapshot().session("chat-1", "local", "SIM-1").is_some(),
            "the session outlives its last watcher"
        );
        hub.shutdown().await;
    }

    async fn next_tags(
        batches: &mut (impl futures::Stream<Item = Vec<u8>> + Unpin),
        count: usize,
    ) -> Vec<u8> {
        use futures::StreamExt as _;
        let mut demuxer = zeron_proto::device_hub::AvccDemuxer::default();
        let mut tags = Vec::new();
        tokio::time::timeout(Duration::from_secs(10), async {
            while tags.len() < count {
                let batch = batches.next().await.unwrap();
                tags.extend(demuxer.push(&batch).unwrap().iter().map(|e| e.tag));
            }
        })
        .await
        .expect("AVCC envelopes");
        tags
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn h264_watchers_share_one_avcc_read_late_joiners_get_the_gop_and_input_opens() {
        let dir = tempfile::tempdir().unwrap();
        let hub = super::DeviceHub::start(dir.path());
        let (origin, requests) = fake_hub_with(HEALTHY).await;
        hub.attach_hub_for_test(origin);
        hub.configure(zeron_proto::device_hub::ConfigureDeviceHubParams {
            enabled: Some(true),
            ..Default::default()
        })
        .await
        .unwrap();
        hub.open(open_params("SIM-1")).await.unwrap();
        let count = |route: &str| {
            requests
                .lock()
                .unwrap()
                .iter()
                .filter(|line| line.contains(route))
                .count()
        };
        let mut tab = Box::pin(super::avcc_batches(hub.watch_avcc("SIM-1").unwrap()));
        assert_eq!(next_tags(&mut tab, 5).await[..4], [4, 1, 2, 3]);
        eventually(|| count("helper/ws?device=SIM-1") > 0).await;
        assert_eq!(count("stream.mjpeg"), 0, "an AVCC read alone opens input");
        let status = hub.snapshot().streams["SIM-1"].clone();
        assert_eq!(
            (status.avcc, status.video),
            (
                zeron_proto::device_hub::StreamLinkStatus::Live,
                zeron_proto::device_hub::StreamLinkStatus::Idle
            ),
            "the AVCC read reports on its own status"
        );
        let mut float = Box::pin(super::avcc_batches(hub.watch_avcc("SIM-1").unwrap()));
        let replay = next_tags(&mut float, 3).await;
        assert_eq!(replay[..2], [1, 2], "description and keyframe first");
        assert!(replay[2..].iter().all(|tag| *tag == 3), "{replay:?}");
        let mjpeg = hub.watch_frames("SIM-1").unwrap();
        let streams = || hub.snapshot().streams.get("SIM-1").cloned();
        assert_eq!(streams().map(|status| status.subscribers), Some(3));
        assert_eq!(count("stream.avcc"), 1, "one shared hub read");
        drop((tab, float));
        assert_eq!(streams().map(|status| status.subscribers), Some(1));
        drop(mjpeg);
        assert_eq!(streams(), None);
        let mut again = Box::pin(super::avcc_batches(hub.watch_avcc("SIM-1").unwrap()));
        assert_eq!(
            next_tags(&mut again, 1).await[0],
            1,
            "cached description first"
        );
        hub.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn an_avd_boots_under_its_serial_and_streams_semu_video_with_input_on_the_same_socket() {
        use zeron_proto::device_hub::{
            DeviceButton, DeviceOrientation, DevicePlatform, DevicePlatformAvailability,
            StreamLinkStatus,
        };
        let dir = tempfile::tempdir().unwrap();
        let hub = super::DeviceHub::start(dir.path());
        hub.use_platforms_for_test(
            [DevicePlatform::Ios, DevicePlatform::Android]
                .map(|platform| DevicePlatformAvailability {
                    platform,
                    available: true,
                    reason: None,
                })
                .to_vec(),
        );
        let commands: Arc<std::sync::Mutex<Vec<Vec<String>>>> = Arc::default();
        let ran = commands.clone();
        hub.use_actions_for_test(
            Arc::new(move |argv: Vec<String>| {
                ran.lock().unwrap().push(argv.clone());
                Box::pin(async move {
                    super::actions::CommandOutput {
                        code: 0,
                        stdout: if argv.last().map(String::as_str) == Some("-list-avds") {
                            "INFO    | Storing crashdata in: /tmp/x\nPixel_9_API_36\n".into()
                        } else {
                            String::new()
                        },
                        stderr: String::new(),
                    }
                })
            }),
            super::actions::Helpers {
                android_sdk: Some("/sdk".into()),
                ..Default::default()
            },
        );
        let (origin, requests) = fake_hub_with(HEALTHY).await;
        hub.attach_hub_for_test(origin);
        hub.configure(zeron_proto::device_hub::ConfigureDeviceHubParams {
            enabled: Some(true),
            ..Default::default()
        })
        .await
        .unwrap();
        assert_eq!(
            commands.lock().unwrap()[0],
            ["/sdk/emulator/emulator", "-list-avds"]
        );
        let avd = hub
            .snapshot()
            .device("local", "Pixel_9_API_36")
            .cloned()
            .expect("the stopped AVD is listed under its name");
        assert_eq!(
            (avd.platform, avd.booted, avd.version.as_str()),
            (DevicePlatform::Android, false, "Android")
        );
        assert!(hub.snapshot().device("local", "INFO").is_none());

        let open = serde_json::json!({"chatId": "chat-1", "deviceId": "Pixel_9_API_36", "platform": "android"});
        let session = hub
            .open(serde_json::from_value(open).unwrap())
            .await
            .unwrap();
        assert_eq!(
            session.device_id, "emulator-5554",
            "the AVD name became its serial"
        );
        assert_eq!(session.platform, DevicePlatform::Android);
        let state = hub.snapshot();
        let named: Vec<_> = state
            .devices
            .iter()
            .filter(|device| device.name == "Pixel_9_API_36")
            .map(|device| (device.id.as_str(), device.booted))
            .collect();
        assert_eq!(named, [("emulator-5554", true)], "no stale stopped row");
        assert!(state.booting.is_empty());

        assert_eq!(
            hub.watch_frames("emulator-5554").err().as_deref(),
            Some(super::ANDROID_H264_ONLY)
        );
        let mut tab = Box::pin(super::avcc_batches(
            hub.watch_avcc("emulator-5554").unwrap(),
        ));
        assert_eq!(next_tags(&mut tab, 8).await, [1, 2, 3, 3, 3, 1, 2, 3]);
        eventually(|| {
            hub.snapshot()
                .streams
                .get("emulator-5554")
                .map(|status| (status.avcc.clone(), status.input.clone()))
                == Some((StreamLinkStatus::Live, StreamLinkStatus::Live))
        })
        .await;
        hub.send_input(
            "emulator-5554",
            &DeviceInput::Button {
                button: DeviceButton::Back,
            },
        )
        .unwrap();
        assert_eq!(
            hub.send_input(
                "emulator-5554",
                &DeviceInput::Orientation {
                    orientation: DeviceOrientation::LandscapeLeft
                }
            )
            .err()
            .as_deref(),
            Some(super::input::UNSUPPORTED_INPUT)
        );
        let reset = r#"WS {"type":"reset-video","ack":false}"#;
        eventually(|| {
            requests
                .lock()
                .unwrap()
                .iter()
                .any(|line| line == r#"WS {"type":"back"}"#)
        })
        .await;
        let socket: Vec<String> = requests
            .lock()
            .unwrap()
            .iter()
            .filter(|line| line.starts_with("WS "))
            .cloned()
            .collect();
        assert_eq!(
            socket,
            [
                "WS /vendor/serve-emu/ws?device=emulator-5554&frame-meta=1",
                reset,
                reset,
                r#"WS {"type":"back"}"#
            ],
            "a keyframe is asked for on connect and after the encoder restart"
        );
        let mut float = Box::pin(super::avcc_batches(
            hub.watch_avcc("emulator-5554").unwrap(),
        ));
        assert_eq!(
            next_tags(&mut float, 3).await,
            [1, 2, 3],
            "a late joiner gets the GOP"
        );
        eventually(|| {
            requests
                .lock()
                .unwrap()
                .iter()
                .filter(|line| *line == reset)
                .count()
                == 3
        })
        .await;
        drop((tab, float));

        let close =
            serde_json::json!({"chatId": "chat-1", "deviceId": "emulator-5554", "shutdown": true});
        hub.close(serde_json::from_value(close).unwrap())
            .await
            .unwrap();
        let requests = requests.lock().unwrap().clone();
        assert!(
            requests.iter().any(|line| line
                == r#"POST /api/devices/shutdown HTTP/1.1 {"id":"emulator-5554","platform":"android"}"#),
            "{requests:?}"
        );
        assert!(
            requests
                .iter()
                .any(|line| line.starts_with("POST /api/devices/boot ")
                    && line.ends_with(
                        r#"{"id":"Pixel_9_API_36","name":"Pixel_9_API_36","platform":"android"}"#
                    )),
            "{requests:?}"
        );
        assert!(
            requests.iter().all(|line| !line.contains("serve-sim")),
            "an emulator never touches serve-sim: {requests:?}"
        );
        let state = hub.snapshot();
        assert!(state.sessions.is_empty());
        assert!(state.device("local", "emulator-5554").is_none());
        assert!(
            state
                .device("local", "Pixel_9_API_36")
                .is_some_and(|device| !device.booted)
        );
        hub.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_restarted_hub_reattaches_its_sessions_and_the_frame_stream_resumes() {
        let dir = tempfile::tempdir().unwrap();
        let hub = super::DeviceHub::start(dir.path());
        let (first, _) = fake_hub_with(Fake {
            frame: "A",
            ..HEALTHY
        })
        .await;
        let (second, second_requests) = fake_hub_with(Fake {
            frame: "B",
            ..HEALTHY
        })
        .await;
        let pids = spawn_fakes(&hub, vec![first, second]);
        hub.configure(zeron_proto::device_hub::ConfigureDeviceHubParams {
            enabled: Some(true),
            ..Default::default()
        })
        .await
        .unwrap();
        hub.open(open_params("SIM-1")).await.unwrap();
        let mut subscription = hub.watch_frames("SIM-1").unwrap();
        let before = frame_from(&mut subscription.frames, "A").await;

        let seen: Arc<std::sync::Mutex<Vec<DeviceHubState>>> = Arc::default();
        let mut states = hub.watch();
        let recorder = {
            let seen = seen.clone();
            tokio::spawn(async move {
                while states.changed().await.is_ok() {
                    let state = states.borrow_and_update().clone();
                    seen.lock().unwrap().push(state);
                }
            })
        };
        let first_pid = pids.lock().unwrap()[0];
        unsafe { libc::kill(first_pid, libc::SIGKILL) };
        let after = frame_from(&mut subscription.frames, "B").await;
        assert!(after > before, "one subscription, frames keep counting");
        eventually(|| {
            hub.snapshot()
                .streams
                .get("SIM-1")
                .map(|status| status.video.clone())
                == Some(zeron_proto::device_hub::StreamLinkStatus::Live)
        })
        .await;
        recorder.abort();

        let seen = seen.lock().unwrap().clone();
        assert!(
            seen.iter()
                .any(|state| state.host_status == super::DeviceHostStatus::Starting),
            "the restart is published as Starting"
        );
        assert!(
            seen.iter()
                .all(|state| state.session("chat-1", "local", "SIM-1").is_some()),
            "the session survives the restart"
        );
        assert!(seen.iter().any(|state| {
            state.streams.get("SIM-1").map(|status| &status.video)
                == Some(&zeron_proto::device_hub::StreamLinkStatus::Connecting)
        }));
        let state = hub.snapshot();
        assert_eq!(state.host_status, super::DeviceHostStatus::Ready);
        assert_eq!(state.streams["SIM-1"].subscribers, 1);
        assert!(
            second_requests.lock().unwrap().iter().any(|line| line
                .starts_with("POST /vendor/serve-sim/grid/api/start ")
                && line.contains(r#""udid":"SIM-1""#)),
            "the booted simulator gets a helper on the new hub"
        );
        hub.shutdown().await;
        assert!(!alive(pids.lock().unwrap()[1]));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_hub_that_cannot_come_back_ends_failed_after_repeated_attempts() {
        let dir = tempfile::tempdir().unwrap();
        let hub = super::DeviceHub::start(dir.path());
        let (origin, _) = fake_hub().await;
        let pids = spawn_fakes(&hub, vec![origin]);
        hub.configure(zeron_proto::device_hub::ConfigureDeviceHubParams {
            enabled: Some(true),
            ..Default::default()
        })
        .await
        .unwrap();
        let mut states = hub.watch();
        unsafe { libc::kill(pids.lock().unwrap()[0], libc::SIGKILL) };
        let failed = tokio::time::timeout(
            Duration::from_secs(10),
            states.wait_for(|state| state.host_status == super::DeviceHostStatus::Failed),
        )
        .await
        .expect("three restarts at 0, 1 and 2 s give up")
        .unwrap()
        .clone();
        assert_eq!(
            failed.host_status_detail.as_deref(),
            Some("no fake hub left")
        );
        assert!(hub.hub_slot().is_none());
        hub.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn disabling_stops_the_hub_and_publishes_disabled() {
        let dir = tempfile::tempdir().unwrap();
        let hub = super::DeviceHub::start(dir.path());
        let (origin, _) = fake_hub().await;
        let pids = spawn_fakes(&hub, vec![origin]);
        let enabled = hub
            .configure(zeron_proto::device_hub::ConfigureDeviceHubParams {
                enabled: Some(true),
                ..Default::default()
            })
            .await
            .unwrap();
        assert_eq!(enabled.host_status, super::DeviceHostStatus::Ready);
        let pid = pids.lock().unwrap()[0];
        assert!(alive(pid));
        assert_eq!(
            enabled.hosts[0]
                .tools
                .as_ref()
                .unwrap()
                .hub
                .running
                .as_deref(),
            Some("0.12.0")
        );
        let disabled = hub
            .configure(zeron_proto::device_hub::ConfigureDeviceHubParams {
                enabled: Some(false),
                ..Default::default()
            })
            .await
            .unwrap();
        assert_eq!(disabled.host_status, super::DeviceHostStatus::Disabled);
        assert!(disabled.devices.is_empty());
        assert!(
            disabled.hosts[0]
                .tools
                .as_ref()
                .unwrap()
                .hub
                .running
                .is_none()
        );
        assert!(hub.hub_slot().is_none());
        eventually(|| !alive(pid)).await;
    }

    async fn reattaching(
        attach_delay: Duration,
    ) -> (
        tempfile::TempDir,
        Arc<super::DeviceHub>,
        Requests,
        i32,
        tokio::task::JoinHandle<Result<String, String>>,
    ) {
        let dir = tempfile::tempdir().unwrap();
        let hub = super::DeviceHub::start(dir.path());
        let (origin, requests) = fake_hub_with(Fake {
            attach_delay,
            ..HEALTHY
        })
        .await;
        let pids = spawn_fakes(&hub, vec![origin]);
        hub.state.send_modify(|state| {
            state.settings.enabled = true;
            state.host_status = super::DeviceHostStatus::Idle;
            state.sessions.push(zeron_proto::device_hub::DeviceSession {
                chat_id: "chat-1".into(),
                host_id: "local".into(),
                device_id: "SIM-1".into(),
                ..Default::default()
            });
        });
        let starting = tokio::spawn({
            let hub = hub.clone();
            async move { hub.start_hub(false).await }
        });
        eventually(|| {
            requests
                .lock()
                .unwrap()
                .iter()
                .any(|line| line.starts_with("POST /vendor/serve-sim/grid/api/start "))
        })
        .await;
        let pid = pids.lock().unwrap()[0];
        (dir, hub, requests, pid, starting)
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn disable_and_shutdown_do_not_wait_behind_a_start_that_is_reattaching() {
        for shutdown in [false, true] {
            let (_dir, hub, _, pid, starting) = reattaching(Duration::from_secs(60)).await;
            let stop = async {
                if shutdown {
                    hub.shutdown().await;
                } else {
                    hub.configure(zeron_proto::device_hub::ConfigureDeviceHubParams {
                        enabled: Some(false),
                        ..Default::default()
                    })
                    .await
                    .unwrap();
                }
            };
            tokio::time::timeout(Duration::from_secs(2), stop)
                .await
                .expect("stopping does not wait for the reattach");
            eventually(|| !alive(pid)).await;
            assert!(hub.hub_slot().is_none());
            if !shutdown {
                assert_eq!(
                    hub.snapshot().host_status,
                    super::DeviceHostStatus::Disabled
                );
            }
            starting.abort();
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_hub_that_dies_while_reattaching_is_neither_resumed_nor_reported_ready() {
        let (_dir, hub, requests, pid, starting) = reattaching(Duration::from_millis(2500)).await;
        let _subscription = hub
            .streams
            .subscribe("SIM-1", |_, _| tokio::spawn(async {}));
        unsafe { libc::kill(pid, libc::SIGKILL) };
        let started = starting.await.unwrap();
        assert_eq!(started.err().as_deref(), Some(super::HUB_NOT_RUNNING));
        assert_ne!(hub.snapshot().host_status, super::DeviceHostStatus::Ready);
        tokio::time::sleep(Duration::from_millis(300)).await;
        assert!(
            requests
                .lock()
                .unwrap()
                .iter()
                .all(|line| !line.contains("stream.mjpeg")),
            "no reader was resumed against the dead hub"
        );
        hub.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_hub_exit_seen_after_disable_or_shutdown_schedules_no_restart() {
        let dir = tempfile::tempdir().unwrap();
        let hub = super::DeviceHub::start(dir.path());
        let (origin, _) = fake_hub().await;
        let pids = spawn_fakes(&hub, vec![origin]);
        hub.restart_after_exit(Duration::ZERO);
        assert!(hub.restart_slot().is_none());
        hub.set_status(super::DeviceHostStatus::Failed, Some("late".into()));
        let state = hub.snapshot();
        assert_eq!(state.host_status, super::DeviceHostStatus::Disabled);
        assert!(state.host_status_detail.is_none());

        hub.state.send_modify(|state| {
            state.settings.enabled = true;
            state.host_status = super::DeviceHostStatus::Idle;
        });
        hub.closed.store(true, std::sync::atomic::Ordering::SeqCst);
        hub.restart_after_exit(Duration::ZERO);
        assert!(hub.restart_slot().is_none());
        assert_eq!(hub.snapshot().host_status, super::DeviceHostStatus::Idle);
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert!(pids.lock().unwrap().is_empty());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn inspect_only_reads_versions_and_platforms_and_starts_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let hub = super::DeviceHub::start(dir.path());
        let pids = spawn_fakes(&hub, Vec::new());
        let state = hub.list(true).await.unwrap();
        let host = &state.hosts[0];
        let tools = host.tools.as_ref().expect("versions are reported");
        assert_eq!(tools.hub.required, "0.12.0");
        assert!(tools.hub.running.is_none());
        assert_eq!(
            host.platforms
                .iter()
                .map(|platform| platform.platform)
                .collect::<Vec<_>>(),
            [
                zeron_proto::device_hub::DevicePlatform::Ios,
                zeron_proto::device_hub::DevicePlatform::Android
            ]
        );
        assert_eq!(state.host_status, super::DeviceHostStatus::Disabled);
        hub.configure(zeron_proto::device_hub::ConfigureDeviceHubParams {
            enabled: Some(false),
            onboarding_completed: Some(true),
            ..Default::default()
        })
        .await
        .unwrap();
        hub.state.send_modify(|state| state.settings.enabled = true);
        let inspected = hub.list(true).await.unwrap();
        assert_eq!(
            inspected.host_status,
            super::DeviceHostStatus::Disabled,
            "no start was attempted"
        );
        assert!(pids.lock().unwrap().is_empty(), "no hub was spawned");
        assert!(hub.hub_slot().is_none());
        assert!(!dir.path().join("device").exists());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn signal_stop_terminates_the_hub_group_without_waiting() {
        let dir = tempfile::tempdir().unwrap();
        let hub = super::DeviceHub::start(dir.path());
        let mut child = tokio::process::Command::new("/bin/sleep");
        child.arg("30").process_group(0).kill_on_drop(true);
        let child = child.spawn().unwrap();
        let pid = child.id().unwrap() as i32;
        *hub.hub_slot() = Some(super::HubProcess::with_child("http://x".into(), child));
        hub.signal_stop();
        eventually(|| hub.hub_slot().as_mut().is_some_and(|hub| !hub.alive())).await;
        assert_ne!(unsafe { libc::kill(pid, 0) }, 0);
    }

    #[test]
    fn android_list_errors_stay_out_of_host_diagnostics_while_android_is_unavailable() {
        let errors = || {
            vec![
                "[android-utils] Failed to run `avdmanager list avd`:".to_string(),
                "[apple-utils] Failed to run `xcrun simctl list`:".to_string(),
            ]
        };
        assert_eq!(
            super::host_diagnostics(errors(), false).as_deref(),
            Some("[apple-utils] Failed to run `xcrun simctl list`:")
        );
        assert_eq!(
            super::host_diagnostics(errors(), true).as_deref(),
            Some(
                "[android-utils] Failed to run `avdmanager list avd`:\n[apple-utils] Failed to run `xcrun simctl list`:"
            )
        );
        assert_eq!(
            super::host_diagnostics(vec![errors().remove(0)], false),
            None
        );
    }

    #[tokio::test]
    async fn concurrent_configures_never_lose_an_update() {
        let dir = tempfile::tempdir().unwrap();
        let hub = super::DeviceHub::start(dir.path());
        let held = hub.lifecycle.lock().await;
        let configure = |onboarding: Option<bool>| {
            let hub = hub.clone();
            tokio::spawn(async move {
                hub.configure(zeron_proto::device_hub::ConfigureDeviceHubParams {
                    enabled: Some(false),
                    onboarding_completed: onboarding,
                    ..Default::default()
                })
                .await
                .unwrap();
            })
        };
        let first = configure(Some(true));
        tokio::time::sleep(Duration::from_millis(50)).await;
        let second = configure(None);
        tokio::time::sleep(Duration::from_millis(50)).await;
        drop(held);
        first.await.unwrap();
        second.await.unwrap();
        assert!(hub.snapshot().settings.onboarding_completed);
        let saved: zeron_proto::device_hub::DeviceHubSettings = serde_json::from_str(
            &std::fs::read_to_string(dir.path().join(super::SETTINGS_FILE)).unwrap(),
        )
        .unwrap();
        assert_eq!(saved, hub.snapshot().settings);
    }
}
