mod hub_client;
mod hub_process;
mod input;
mod mjpeg;
mod platform;
mod stream;
mod toolchain;

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError, Weak};

use tokio::sync::watch;
use zeron_proto::device_hub::{
    ConfigureDeviceHubParams, DeviceBooting, DeviceHostStatus, DeviceHubHost, DeviceHubSettings,
    DeviceHubState, DeviceInput, DevicePlatform, DeviceSession, LOCAL_DEVICE_HOST_ID,
    OpenHubDeviceParams,
};

use hub_client::HubClient;
use hub_process::HubProcess;
use stream::DeviceStreams;
pub(crate) use stream::FrameSubscription;

const SETTINGS_FILE: &str = "device-hub-settings.json";
pub const DEVICE_SUPPORT_OFF: &str = "Device support is off. Enable it in the Device panel before installing or starting device tools.";

pub struct DeviceHub {
    me: Weak<DeviceHub>,
    data_dir: PathBuf,
    http: reqwest::Client,
    state: watch::Sender<DeviceHubState>,
    lifecycle: tokio::sync::Mutex<()>,
    hub: Mutex<Option<HubProcess>>,
    streams: Arc<DeviceStreams>,
}

fn host_summary() -> DeviceHubHost {
    DeviceHubHost {
        id: LOCAL_DEVICE_HOST_ID.into(),
        kind: "local".into(),
        label: "This machine".into(),
        platforms: vec![platform::ios()],
        hub_installed: toolchain::hub_installed(),
    }
}

impl DeviceHub {
    pub fn start(data_dir: &Path) -> Arc<Self> {
        let settings: DeviceHubSettings = std::fs::read_to_string(data_dir.join(SETTINGS_FILE))
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default();
        let state = DeviceHubState {
            host_status: if settings.enabled {
                DeviceHostStatus::Idle
            } else {
                DeviceHostStatus::Disabled
            },
            settings,
            hosts: vec![host_summary()],
            ..Default::default()
        };
        Arc::new_cyclic(|me| Self {
            me: me.clone(),
            data_dir: data_dir.to_path_buf(),
            http: reqwest::Client::builder()
                .no_proxy()
                .build()
                .unwrap_or_default(),
            state: watch::channel(state).0,
            lifecycle: tokio::sync::Mutex::new(()),
            hub: Mutex::new(None),
            streams: Arc::default(),
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

    fn publish(&self, change: impl FnOnce(&mut DeviceHubState)) {
        self.state.send_modify(|state| {
            change(state);
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

    fn live_origin(&self) -> Option<String> {
        let mut slot = self.hub_slot();
        let alive = slot.as_mut().map(HubProcess::alive)?;
        if alive {
            return slot.as_ref().map(|hub| hub.origin.clone());
        }
        slot.take();
        self.streams.clear();
        None
    }

    pub async fn configure(
        &self,
        params: ConfigureDeviceHubParams,
    ) -> Result<DeviceHubState, String> {
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
        if !enabled {
            self.stop_hub().await;
        }
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
            }
        });
        if enabled {
            self.list(false).await?;
        }
        Ok(self.snapshot())
    }

    pub async fn list(&self, inspect_only: bool) -> Result<DeviceHubState, String> {
        if !self.enabled() {
            return Err(DEVICE_SUPPORT_OFF.into());
        }
        let host = host_summary();
        let ios_available = host.platforms.iter().any(|platform| platform.available);
        self.publish(|state| state.hosts = vec![host]);
        if inspect_only || !ios_available {
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
        let _lifecycle = self.lifecycle.lock().await;
        if let Some(origin) = self.live_origin() {
            return Ok(origin);
        }
        if !self.enabled() {
            return Err(DEVICE_SUPPORT_OFF.into());
        }
        let fail = |error: String| {
            self.set_status(DeviceHostStatus::Failed, Some(error.clone()));
            error
        };
        if !toolchain::hub_installed() {
            self.set_status(DeviceHostStatus::Installing, None);
        }
        let entry = toolchain::ensure_hub().await.map_err(fail)?;
        self.set_status(DeviceHostStatus::Starting, None);
        let (program, args) = toolchain::launch(&entry).map_err(fail)?;
        let hub = hub_process::spawn(
            &self.http,
            &program,
            &args,
            &entry,
            &self.device_dir(),
            hub_process::READY_TIMEOUT,
        )
        .await
        .map_err(fail)?;
        if !self.enabled() {
            hub.stop().await;
            return Err(DEVICE_SUPPORT_OFF.into());
        }
        let origin = hub.origin.clone();
        *self.hub_slot() = Some(hub);
        self.publish(|state| {
            state.host_status = DeviceHostStatus::Ready;
            state.host_status_detail = None;
            state.hosts = vec![host_summary()];
        });
        Ok(origin)
    }

    async fn refresh(&self, origin: &str) -> Result<(), String> {
        let (devices, detail) = HubClient {
            http: &self.http,
            origin,
        }
        .list()
        .await?;
        self.publish(|state| {
            state.devices = devices;
            state.host_status = DeviceHostStatus::Ready;
            state.host_status_detail = detail;
        });
        Ok(())
    }

    pub async fn open(&self, params: OpenHubDeviceParams) -> Result<DeviceSession, String> {
        if !self.enabled() {
            return Err(DEVICE_SUPPORT_OFF.into());
        }
        if params.platform != DevicePlatform::Ios {
            return Err("Only iOS Simulators are supported so far.".into());
        }
        let ios = platform::ios();
        if !ios.available {
            return Err(ios.reason.unwrap_or_default());
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
            .ok_or_else(|| format!("Device {} was not found.", params.device_id))?;
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
            let booted = async {
                let id = client.boot(&device).await?;
                client.attach_stream(&id).await?;
                Ok::<_, String>(id)
            }
            .await;
            self.publish(|state| state.booting.retain(|entry| entry.device.id != device.id));
            let id = booted?;
            self.refresh(&origin).await?;
            id
        } else {
            if device.booted {
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
        if !self.enabled() {
            return Err(DEVICE_SUPPORT_OFF.into());
        }
        let origin = self
            .live_origin()
            .ok_or_else(|| "The device hub is not running.".to_string())?;
        let client = HubClient {
            http: &self.http,
            origin: &origin,
        };
        let (video, input) = (client.mjpeg_url(device_id), client.input_url(device_id));
        let me = self.me.clone();
        let http = self.http.clone();
        let id = device_id.to_string();
        Ok(self.streams.subscribe(device_id, move |frames| {
            stream::spawn_reader(http, video, frames, move || {
                if let Some(hub) = me.upgrade() {
                    hub.open_input(&id, &input);
                }
            })
        }))
    }

    fn open_input(&self, device_id: &str, url: &str) {
        if self.streams.has_input(device_id) {
            return;
        }
        let me = self.me.clone();
        let id = device_id.to_string();
        let socket = input::InputSocket::connect(url.to_string(), move |screen| {
            if let Some(hub) = me.upgrade() {
                hub.publish(|state| {
                    state.screens.insert(id.clone(), screen);
                });
            }
        });
        self.streams.set_input(device_id, socket);
    }

    pub fn send_input(&self, device_id: &str, input: &DeviceInput) -> Result<(), String> {
        self.streams.send_input(device_id, input)
    }

    async fn stop_hub(&self) {
        let _lifecycle = self.lifecycle.lock().await;
        self.streams.clear();
        let hub = self.hub_slot().take();
        if let Some(hub) = hub {
            hub.stop().await;
        }
    }

    pub async fn shutdown(&self) {
        self.stop_hub().await;
    }

    #[cfg(test)]
    pub(crate) fn attach_hub_for_test(&self, origin: String) {
        *self.hub_slot() = Some(HubProcess::external(origin));
    }
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use std::sync::Arc;
    use std::time::Duration;

    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use zeron_proto::device_hub::DeviceHubState;
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

    async fn fake_hub() -> (String, Requests) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let requests: Requests = Arc::default();
        let seen = requests.clone();
        tokio::spawn(async move {
            loop {
                let (mut socket, _) = listener.accept().await.unwrap();
                let seen = seen.clone();
                tokio::spawn(async move {
                    let request = read_request(&mut socket).await;
                    let line = request.lines().next().unwrap_or_default().to_string();
                    seen.lock().unwrap().push(line.clone());
                    let body = if line.starts_with("GET /api/devices ") {
                        serde_json::json!({
                            "simulators": [{"id": "SIM-1", "name": "iPhone 17", "version": "iOS 27.0",
                                "platform": "ios", "booted": true, "physical": false}],
                            "emulators": []
                        })
                    } else {
                        serde_json::json!({"ok": true})
                    };
                    let body = body.to_string();
                    let reply = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                        body.len()
                    );
                    let _ = socket.write_all(reply.as_bytes()).await;
                });
            }
        });
        (origin, requests)
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
}
