//! The device hub: Settings → Device hub and a chat's Device tab
//! (ui/screens/device, ui/screens/settings/device_hub), over the engine's
//! device hub methods (gpui's crates/ui/src/device_panel and
//! settings/device_hub.rs make the same calls).
//!
//!   device.call({ method, params })   one of the engine's hub methods (METHODS),
//!                                     its answer; `outdated` from an engine
//!                                     without it, else an outcome word or the
//!                                     engine's own words
//!   device.watch(on)                  (invoke) the hub's every state in the
//!                                     window's shared `device_hub` while on
//!                                     (`ListHubDevices` first: the hub starts
//!                                     lazily); `{ outdated = true }` from an
//!                                     engine without devices
//!   device.stream({ key, host_id?, device_id })
//!                                     (invoke) the device's screen as JPEG
//!                                     frames (`WatchHubDeviceFrames`), each
//!                                     decoded off the window's thread and shown
//!                                     as `ui.image { src = "host:device/<key>" }`;
//!                                     shared `device_frames`[key] = { seq, w, h }
//!   device.unstream(key)              that stream stopped, its picture let go
//!
//! `ZERON_ZGX_MOCK=1` (a mock Dev): a hub of the host's own instead of the
//! engine's (`Mock`), three devices and three still frames from
//! ui/assets/device, so the picker, the tab and its screen can be driven with
//! no simulator. Its stream shows the three and then holds the last: a still
//! screen asks for no frames.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use serde_json::{Value as Json, json};
use zgx_desktop::WindowContext;
use zgx_script::{ScriptHost, Shared, Value};

use crate::auth::text;
use crate::engine;
use crate::plugins::{json_value, value_json};

/// The engine's device methods a window may call (the hub's settings, a
/// tab's device, its screenshot and its input). Tools, actions and the
/// accessibility tree are the engine's agents', not asked from here.
const METHODS: &[&str] = &[
    "ListHubDevices",
    "ConfigureDeviceHub",
    "TestDeviceHost",
    "OpenHubDevice",
    "CloseHubDevice",
    "HubDeviceScreenshot",
    "SendHubDeviceInput",
];

/// The frames a mock stream shows before it holds still.
const MOCK_FRAMES: [&[u8]; 3] = [
    include_bytes!("../ui/assets/device/frame-1.png"),
    include_bytes!("../ui/assets/device/frame-2.png"),
    include_bytes!("../ui/assets/device/frame-3.png"),
];
const MOCK_FRAME_EVERY: Duration = Duration::from_millis(400);

fn mock() -> bool {
    std::env::var("ZERON_ZGX_MOCK").is_ok_and(|v| v == "1")
}

/// An engine error said as a word the scripts read: an engine too old for
/// the method is `outdated`.
pub(crate) fn said(error: &str) -> String {
    let lower = error.to_ascii_lowercase();
    if lower.contains("unknown method") || lower.contains("method not found") {
        "outdated".into()
    } else if engine::outcome(error) == "offline" {
        "offline".into()
    } else {
        error.to_string()
    }
}

/// Whether an engine's `EngineInfo` says it has the device hub.
pub(crate) fn supports(info: &Json) -> bool {
    info["capabilities"]
        .as_array()
        .is_some_and(|all| all.iter().any(|c| c.as_str() == Some(zeron_proto::capabilities::DEVICE_HUB_V1)))
}

/// A frame from `WatchHubDeviceFrames`: its sequence and JPEG bytes.
pub(crate) fn frame_bytes(item: &Json) -> Option<(u64, Vec<u8>)> {
    use base64::Engine as _;
    let jpeg = item["jpeg"].as_str().filter(|s| !s.is_empty())?;
    let bytes = base64::engine::general_purpose::STANDARD.decode(jpeg).ok()?;
    Some((item["seq"].as_u64().unwrap_or(0), bytes))
}

fn image_name(key: &str) -> String {
    format!("device/{key}")
}

struct Stream {
    generation: u64,
    _watch: Option<engine::rpc::Watch>,
}

struct Window {
    host: ScriptHost,
    hub: Shared,
    frames: Shared,
    sizes: RefCell<HashMap<String, (u64, u32, u32)>>,
    watch: RefCell<Option<engine::rpc::Watch>>,
    watching: Cell<bool>,
    streams: RefCell<HashMap<String, Stream>>,
    next: Cell<u64>,
}

thread_local! {
    static MOCK: Rc<RefCell<Mock>> = Rc::new(RefCell::new(Mock::default()));
    static WINDOWS: RefCell<Vec<std::rc::Weak<Window>>> = const { RefCell::new(Vec::new()) };
}

impl Window {
    fn publish_frames(&self) {
        let sizes = self.sizes.borrow();
        self.frames.set(Value::record(sizes.iter().map(|(k, (seq, w, h))| {
            (
                k.as_str(),
                Value::record([("seq", Value::from(*seq as f64)), ("w", Value::from(*w as f64)), ("h", Value::from(*h as f64))]),
            )
        })));
    }

    fn show(&self, key: &str, generation: u64, seq: u64, image: Arc<zgx::image::ImageData>) {
        if self.streams.borrow().get(key).map(|s| s.generation) != Some(generation) {
            return;
        }
        let (w, h) = (image.width(), image.height());
        self.host.image(&image_name(key), Some(image));
        self.sizes.borrow_mut().insert(key.to_string(), (seq, w, h));
        self.publish_frames();
    }

    fn unstream(&self, key: &str) {
        if self.streams.borrow_mut().remove(key).is_some() {
            self.host.image(&image_name(key), None);
            self.sizes.borrow_mut().remove(key);
            self.publish_frames();
        }
    }

    fn start_watch(self: &Rc<Self>) {
        if mock() {
            self.hub.set(json_value(&MOCK.with(|m| m.borrow().state())));
            return;
        }
        let me = Rc::downgrade(self);
        engine::get().spawn(async move {
            let Some(rpc) = engine::get().connection(Duration::from_secs(5)).await else { return };
            let info = rpc.call("EngineInfo", json!({})).await.unwrap_or(Json::Null);
            let Some(window) = me.upgrade() else { return };
            if !window.watching.get() {
                return;
            }
            if !info.is_null() && !supports(&info) {
                window.hub.set(Value::record([("outdated", Value::Bool(true))]));
                return;
            }
            let listed = rpc.call("ListHubDevices", json!({ "inspectOnly": true })).await;
            if let Err(error) = &listed
                && said(error) == "outdated"
            {
                window.hub.set(Value::record([("outdated", Value::Bool(true))]));
                return;
            }
            if !window.watching.get() {
                return;
            }
            let heard = Rc::downgrade(&window);
            let watch = rpc.watch("WatchDeviceHub", json!({}), move |state| {
                if let Some(window) = heard.upgrade() {
                    window.hub.set(json_value(&state));
                }
            });
            *window.watch.borrow_mut() = Some(watch);
        });
    }

    fn stream(self: &Rc<Self>, key: String, host_id: Option<String>, device_id: String) {
        let generation = self.next.get() + 1;
        self.next.set(generation);
        self.streams.borrow_mut().insert(key.clone(), Stream { generation, _watch: None });
        let me = Rc::downgrade(self);
        if mock() {
            engine::get().spawn(async move {
                for (n, png) in MOCK_FRAMES.iter().enumerate() {
                    let decoded = off_thread(move || zgx_gpu::assets::decode_image(png).ok()).await.flatten();
                    let Some(window) = me.upgrade() else { return };
                    let Some(image) = decoded else { return };
                    window.show(&key, generation, n as u64 + 1, image);
                    drop(window);
                    zgx::timer::sleep(MOCK_FRAME_EVERY).await;
                }
            });
            return;
        }
        let (decode, frames) = std::sync::mpsc::sync_channel::<(u64, Vec<u8>)>(1);
        let (shown, mut decoded) = zgx::task::latest::<(u64, Arc<zgx::image::ImageData>)>();
        std::thread::spawn(move || {
            while let Ok((seq, bytes)) = frames.recv() {
                if let Ok(image) = zgx_gpu::assets::decode_image(&bytes) {
                    shown.send((seq, image));
                }
            }
        });
        let showing = me.clone();
        let show_key = key.clone();
        engine::get().spawn(async move {
            while let Some((seq, image)) = decoded.recv().await {
                let Some(window) = showing.upgrade() else { return };
                window.show(&show_key, generation, seq, image);
            }
        });
        engine::get().spawn(async move {
            let Some(rpc) = engine::get().connection(Duration::from_secs(5)).await else { return };
            let mut params = json!({ "deviceId": device_id, "codec": "mjpeg" });
            if let Some(host) = host_id.filter(|h| h != zeron_proto::device_hub::LOCAL_DEVICE_HOST_ID) {
                params["hostId"] = host.into();
            }
            // (Latest wins: a frame the decoder hasn't taken yet is passed over.)
            let watch = rpc.watch("WatchHubDeviceFrames", params, move |item| {
                if let Some(frame) = frame_bytes(&item) {
                    let _ = decode.try_send(frame);
                }
            });
            let Some(window) = me.upgrade() else { return };
            if let Some(stream) = window.streams.borrow_mut().get_mut(&key).filter(|s| s.generation == generation) {
                stream._watch = Some(watch);
            }
        });
    }
}

async fn off_thread<T: Send + 'static>(work: impl FnOnce() -> T + Send + 'static) -> Option<T> {
    let (tx, mut rx) = zgx::task::latest();
    std::thread::spawn(move || {
        tx.send(work());
    });
    rx.recv().await
}

/// One of the hub's methods (METHODS), as `device.call` and Settings'
/// `settings.engine` ask it: the mock's hub on a mock Dev.
pub(crate) async fn call(method: String, params: Json) -> Result<Value, String> {
    if !METHODS.contains(&method.as_str()) {
        return Err("refused".into());
    }
    let params = if params.is_object() { params } else { json!({}) };
    if mock() {
        let answer = MOCK.with(|m| m.borrow_mut().call(&method, &params));
        publish_mock();
        return answer.map(|a| json_value(&a));
    }
    engine::get().call(&method, params).await.map(|a| json_value(&a)).map_err(|e| said(&e))
}

/// Whether Settings' `settings.engine` hands `method` to the hub.
pub(crate) fn is_hub_method(method: &str) -> bool {
    matches!(method, "ListHubDevices" | "ConfigureDeviceHub" | "TestDeviceHost")
}

/// What the mock hub's change means for every window watching it.
fn publish_mock() {
    let state = json_value(&MOCK.with(|m| m.borrow().state()));
    WINDOWS.with(|all| {
        for window in all.borrow().iter().filter_map(std::rc::Weak::upgrade) {
            if window.watching.get() {
                window.hub.set(state.clone());
            }
        }
    });
}

pub fn install(cx: &mut WindowContext, host: &ScriptHost) {
    let window = Rc::new(Window {
        host: host.clone(),
        hub: host.shared("device_hub", Value::Nil),
        frames: host.shared("device_frames", Value::record(std::iter::empty::<(&str, Value)>())),
        sizes: RefCell::default(),
        watch: RefCell::default(),
        watching: Cell::new(false),
        streams: RefCell::default(),
        next: Cell::new(0),
    });
    WINDOWS.with(|all| {
        let mut all = all.borrow_mut();
        all.retain(|w| w.strong_count() > 0);
        all.push(Rc::downgrade(&window));
    });
    let closing = window.clone();
    cx.on_closed(move || {
        closing.watch.borrow_mut().take();
        closing.streams.borrow_mut().clear();
    });
    host.async_action("device.call", |args| {
        let method = args.field("method").map(text).unwrap_or_default();
        let params = args.field("params").map(value_json).unwrap_or(json!({}));
        call(method, params)
    });
    let w = window.clone();
    host.action("device.watch", move |on| {
        let on = on.truthy();
        if on == w.watching.get() {
            return;
        }
        w.watching.set(on);
        if on {
            w.start_watch();
        } else {
            w.watch.borrow_mut().take();
        }
    });
    let w = window.clone();
    host.action("device.stream", move |args| {
        let key = args.field("key").map(text).unwrap_or_default();
        let device = args.field("device_id").map(text).unwrap_or_default();
        if key.is_empty() || device.is_empty() {
            return;
        }
        let host_id = args.field("host_id").map(text).filter(|h| !h.is_empty());
        w.stream(key, host_id, device);
    });
    let w = window;
    host.action("device.unstream", move |key| {
        w.unstream(&text(&key));
    });
}

/// The mock Dev's hub: the engine's `DeviceHubState` as a hub would answer
/// it, kept here.
pub(crate) struct Mock {
    revision: u64,
    enabled: bool,
    agent: bool,
    onboarded: bool,
    hosts: Vec<zeron_proto::device_hub::SshDeviceHostConfig>,
    booted: Vec<String>,
    sessions: Vec<Json>,
    pub(crate) inputs: Vec<Json>,
}

impl Default for Mock {
    fn default() -> Self {
        Mock {
            revision: 1,
            enabled: false,
            agent: false,
            onboarded: false,
            hosts: Vec::new(),
            booted: vec!["mock-iphone".into()],
            sessions: Vec::new(),
            inputs: Vec::new(),
        }
    }
}

const MOCK_DEVICES: [(&str, &str, &str, &str); 3] = [
    ("mock-iphone", "ios", "iPhone 17", "iOS 27.0"),
    ("mock-ipad", "ios", "iPad Air", "iOS 27.0"),
    ("Pixel_9", "android", "Pixel 9", "Android 16"),
];

impl Mock {
    pub(crate) fn state(&self) -> Json {
        let status = if self.enabled { "ready" } else { "disabled" };
        let devices: Vec<Json> = MOCK_DEVICES
            .iter()
            .map(|(id, platform, name, version)| {
                json!({ "hostId": "local", "id": id, "platform": platform, "name": name, "version": version,
                    "booted": self.booted.iter().any(|b| b == id), "physical": false })
            })
            .collect();
        let mut hosts = vec![json!({
            "id": "local", "kind": "local", "label": "This machine", "hubInstalled": true,
            "platforms": [{ "platform": "ios", "available": true }, { "platform": "android", "available": true }],
            "tools": { "hub": { "required": "0.12.0", "installed": ["0.12.0"], "running": if self.enabled { Json::from("0.12.0") } else { Json::Null } } },
        })];
        let mut statuses = serde_json::Map::new();
        for host in &self.hosts {
            hosts.push(json!({ "id": host.id, "kind": "ssh", "label": host.label, "platforms": [], "hubInstalled": false }));
            statuses.insert(host.id.clone(), json!({ "status": "failed", "detail": "Mock hosts can't be reached." }));
        }
        let mut screens = serde_json::Map::new();
        let mut streams = serde_json::Map::new();
        for s in &self.sessions {
            let key = s["deviceId"].as_str().unwrap_or_default().to_string();
            screens.insert(key.clone(), json!({ "width": 196, "height": 426, "orientation": "portrait" }));
            streams.insert(key, json!({ "video": "live", "input": "live", "subscribers": 1 }));
        }
        json!({
            "revision": self.revision,
            "settings": { "enabled": self.enabled, "agentAccessEnabled": self.agent, "onboardingCompleted": self.onboarded,
                "deviceHosts": serde_json::to_value(&self.hosts).unwrap_or_default() },
            "hosts": hosts,
            "hostStatus": status,
            "hostStatuses": statuses,
            "devices": if self.enabled { Json::Array(devices) } else { json!([]) },
            "sessions": self.sessions,
            "booting": [],
            "streams": streams,
            "screens": screens,
            "agentStatus": if self.agent { "ready" } else { "disabled" },
        })
    }

    pub(crate) fn call(&mut self, method: &str, params: &Json) -> Result<Json, String> {
        self.revision += 1;
        match method {
            "ListHubDevices" => Ok(self.state()),
            "ConfigureDeviceHub" => {
                if let Some(hosts) = params.get("deviceHosts") {
                    let hosts: Vec<zeron_proto::device_hub::SshDeviceHostConfig> =
                        serde_json::from_value(hosts.clone()).map_err(|e| e.to_string())?;
                    zeron_proto::device_hub::validate_device_hosts(&hosts)?;
                    self.hosts = hosts;
                }
                if let Some(on) = params["enabled"].as_bool() {
                    self.enabled = on;
                    if !on {
                        self.sessions.clear();
                    }
                }
                if let Some(on) = params["agentAccessEnabled"].as_bool() {
                    self.agent = on && self.enabled;
                }
                if let Some(on) = params["onboardingCompleted"].as_bool() {
                    self.onboarded = on;
                }
                Ok(self.state())
            }
            "TestDeviceHost" => Ok(json!({ "local": false, "checks": [
                { "name": "SSH connection", "ok": true },
                { "name": "Xcode", "ok": false, "detail": "Not installed on the mock host" },
            ], "platforms": [] })),
            "OpenHubDevice" => {
                if !self.enabled {
                    return Err("The device hub is off.".into());
                }
                let device = params["deviceId"].as_str().unwrap_or_default().to_string();
                let Some((_, platform, ..)) = MOCK_DEVICES.iter().find(|(id, ..)| *id == device) else {
                    return Err(format!("No device {device}."));
                };
                let chat = params["chatId"].as_str().unwrap_or_default().to_string();
                if !self.booted.contains(&device) {
                    self.booted.push(device.clone());
                }
                let session = json!({ "chatId": chat, "hostId": "local", "deviceId": device, "platform": platform,
                    "openedAt": "2026-11-09T00:00:00Z", "openedBy": "user" });
                self.sessions.retain(|s| !(s["chatId"] == session["chatId"] && s["deviceId"] == session["deviceId"]));
                self.sessions.push(session.clone());
                Ok(session)
            }
            "CloseHubDevice" => {
                let chat = params["chatId"].as_str().unwrap_or_default();
                let device = params["deviceId"].as_str();
                self.sessions.retain(|s| s["chatId"] != chat || device.is_some_and(|d| s["deviceId"] != d));
                if params["shutdown"].as_bool() == Some(true)
                    && let Some(device) = device
                {
                    self.booted.retain(|b| b != device);
                }
                Ok(json!({}))
            }
            "HubDeviceScreenshot" => {
                let path = std::env::temp_dir().join("zeron-mock-device-screenshot.png");
                std::fs::write(&path, MOCK_FRAMES[2]).map_err(|e| e.to_string())?;
                Ok(json!({ "path": path.to_string_lossy() }))
            }
            "SendHubDeviceInput" => {
                serde_json::from_value::<zeron_proto::device_hub::SendHubDeviceInputParams>(params.clone())
                    .map_err(|e| e.to_string())?;
                self.inputs.push(params["input"].clone());
                Ok(json!({}))
            }
            _ => Err("refused".into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_old_engine_reads_outdated_and_one_without_devices_is_told_apart() {
        assert_eq!(said("unknown method: ListHubDevices"), "outdated");
        assert_eq!(said("closed"), "offline");
        assert_eq!(said("The device hub failed to start."), "The device hub failed to start.");
        assert!(supports(&json!({ "capabilities": ["device-hub-v1", "other"] })));
        assert!(!supports(&json!({ "capabilities": ["other"] })));
        assert!(!supports(&json!({})));
        for method in ["WatchDeviceHub", "RunHubDeviceAction", "DeviceToolOpen", "GetHubDeviceAxTree", "ComputerAction"] {
            assert!(!METHODS.contains(&method), "{method} is not a window's to call");
        }
    }

    #[test]
    fn a_frame_is_its_jpeg_and_a_codec_frame_is_none() {
        use base64::Engine as _;
        let jpeg = base64::engine::general_purpose::STANDARD.encode([0xFF, 0xD8, 1, 2]);
        assert_eq!(frame_bytes(&json!({ "seq": 7, "jpeg": jpeg })), Some((7, vec![0xFF, 0xD8, 1, 2])));
        assert_eq!(frame_bytes(&json!({ "seq": 8, "jpeg": "", "avcc": "AAAA" })), None);
        assert_eq!(frame_bytes(&json!({ "seq": 9, "jpeg": "not base64!" })), None);
    }

    #[test]
    fn the_mock_hub_sets_up_opens_a_device_and_streams_still_frames() {
        let mut hub = Mock::default();
        let off = hub.state();
        assert_eq!(off["hostStatus"], "disabled");
        assert_eq!(off["devices"], json!([]), "off, it lists nothing");
        let on = hub.call("ConfigureDeviceHub", &json!({ "enabled": true })).unwrap();
        assert_eq!((on["hostStatus"].as_str(), on["devices"].as_array().map(Vec::len)), (Some("ready"), Some(3)));
        assert_eq!(on["devices"][0]["booted"], true);
        assert_eq!(on["devices"][2]["booted"], false);
        let state: zeron_proto::device_hub::DeviceHubState = serde_json::from_value(on).unwrap();
        assert!(state.settings.enabled && !state.settings.onboarding_completed);
        assert!(hub.call("ConfigureDeviceHub", &json!({ "deviceHosts": [{ "id": "local", "label": "x", "target": "a@b" }] })).is_err());
        let hosts = hub
            .call("ConfigureDeviceHub", &json!({ "deviceHosts": [{ "id": "host-1", "label": "Mac mini", "target": "me@mini" }] }))
            .unwrap();
        assert_eq!(hosts["hostStatuses"]["host-1"]["status"], "failed");
        let session = hub.call("OpenHubDevice", &json!({ "chatId": "c1", "deviceId": "Pixel_9", "platform": "android" })).unwrap();
        assert_eq!(session["platform"], "android");
        let open = hub.state();
        assert_eq!(open["sessions"].as_array().map(Vec::len), Some(1));
        assert_eq!(open["devices"][2]["booted"], true);
        assert_eq!(open["screens"]["Pixel_9"]["height"], 426);
        hub.call("SendHubDeviceInput", &json!({ "deviceId": "Pixel_9", "input": { "touch": { "phase": "begin", "x": 0.5, "y": 0.25 } } }))
            .unwrap();
        assert!(hub.call("SendHubDeviceInput", &json!({ "deviceId": "Pixel_9", "input": { "nonsense": 1 } })).is_err());
        assert_eq!(hub.inputs, vec![json!({ "touch": { "phase": "begin", "x": 0.5, "y": 0.25 } })]);
        hub.call("CloseHubDevice", &json!({ "chatId": "c1", "deviceId": "Pixel_9", "shutdown": true })).unwrap();
        let closed = hub.state();
        assert_eq!(closed["sessions"], json!([]));
        assert_eq!(closed["devices"][2]["booted"], false);
        for png in MOCK_FRAMES {
            let image = zgx_gpu::assets::decode_image(png).unwrap();
            assert_eq!((image.width(), image.height()), (196, 426));
        }
    }
}
