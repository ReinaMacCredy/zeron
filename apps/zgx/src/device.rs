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
//!                                     shared `device_frames`[key] = { seq, w, h },
//!                                     and `failed` (why) once the engine ends it
//!
//! The hub's watch and each stream are watched again on every connection to
//! the engine (one that restarts, or comes late at launch), as gpui's
//! WATCH_DEVICE_HUB is on each engine attach.
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
use std::sync::atomic::{AtomicU8, Ordering};
use std::time::Duration;

use serde_json::{Value as Json, json};
use zgx_desktop::WindowContext;
use zgx_script::{ScriptHost, Shared, Value};

use crate::auth::text;
use crate::device_video::{self, FallbackReason};
use crate::engine;
use zeron_proto::device_hub::{DeviceOrientation, DevicePlatform, DeviceScreenConfig, HingeCommand, HingePose};
use crate::device3d::{self, ThreeD};
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
    "SetHubDeviceHinge",
    "HubDeviceFold",
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

/// What a stream hands its decoder: a frame of `WatchHubDeviceFrames`
/// (a JPEG, or H.264 in avcC envelopes), or the device turned.
#[derive(Debug, PartialEq)]
pub(crate) enum Payload {
    Jpeg(u64, Vec<u8>),
    Avcc(u64, Vec<u8>),
    Turn,
}

pub(crate) fn frame_payload(item: &Json) -> Option<Payload> {
    use base64::Engine as _;
    let seq = item["seq"].as_u64().unwrap_or(0);
    let decode = |field: &str| {
        let text = item[field].as_str().filter(|s| !s.is_empty())?;
        base64::engine::general_purpose::STANDARD.decode(text).ok()
    };
    if let Some(jpeg) = decode("jpeg") {
        return Some(Payload::Jpeg(seq, jpeg));
    }
    decode("avcc").map(|avcc| Payload::Avcc(seq, avcc))
}

/// The codec a stream asks for first (gpui's `codec_choice`): H.264 where
/// VideoToolbox decodes it, MJPEG elsewhere.
fn first_codec() -> &'static str {
    if cfg!(target_os = "macos") { "h264" } else { "mjpeg" }
}

pub(crate) fn orientation_of(word: &str) -> DeviceOrientation {
    serde_json::from_value(Json::from(word)).unwrap_or_default()
}

fn orientation_byte(o: DeviceOrientation) -> u8 {
    match o {
        DeviceOrientation::Portrait => 0,
        DeviceOrientation::LandscapeLeft => 1,
        DeviceOrientation::PortraitUpsideDown => 2,
        DeviceOrientation::LandscapeRight => 3,
    }
}

fn byte_orientation(b: u8) -> DeviceOrientation {
    match b {
        1 => DeviceOrientation::LandscapeLeft,
        2 => DeviceOrientation::PortraitUpsideDown,
        3 => DeviceOrientation::LandscapeRight,
        _ => DeviceOrientation::Portrait,
    }
}

/// What the decoder thread hands the window: a picture, or why H.264 stopped.
pub(crate) enum Decoded {
    Picture(u64, Arc<zgx::image::ImageData>),
    Fallback(FallbackReason),
}

/// A stream's decoder, on a thread of its own: each batch of what has
/// arrived since the last becomes at most one picture (the newest), turned
/// to the device's orientation. After a fallback it decodes JPEG only.
pub(crate) fn decode_frames(frames: std::sync::mpsc::Receiver<Payload>, orientation: Arc<AtomicU8>, mut out: impl FnMut(Decoded)) {
    let mut last: Option<(u64, u32, u32, Arc<[u8]>)> = None;
    let mut last_jpeg: Option<Vec<u8>> = None;
    let mut demuxer = zeron_proto::device_hub::AvccDemuxer::default();
    #[cfg(target_os = "macos")]
    let mut pipeline = device_video::Pipeline::default();
    let mut h264 = true;
    while let Ok(first) = frames.recv() {
        let mut jpeg = None;
        let mut turned = false;
        let mut envelopes = Vec::new();
        let mut pending = 0;
        let mut avcc_seq = 0;
        for item in std::iter::once(first).chain(frames.try_iter()) {
            match item {
                Payload::Jpeg(seq, bytes) => jpeg = Some((seq, bytes)),
                Payload::Turn => turned = true,
                Payload::Avcc(_, _) if !h264 => {}
                Payload::Avcc(seq, bytes) => {
                    let Some(batch) = demuxer.push(&bytes) else {
                        h264 = false;
                        out(Decoded::Fallback(FallbackReason::DecodeErrors));
                        continue;
                    };
                    if batch.first().is_none_or(|e| e.tag != zeron_proto::device_hub::AVCC_DESCRIPTION) {
                        pending += batch
                            .iter()
                            .filter(|e| matches!(e.tag, zeron_proto::device_hub::AVCC_KEYFRAME | zeron_proto::device_hub::AVCC_DELTA))
                            .count();
                    }
                    avcc_seq = seq;
                    envelopes.extend(batch);
                }
            }
        }
        let mut picture: Option<(u64, device_video::Rgba)> = None;
        #[cfg(target_os = "macos")]
        if h264 && (!envelopes.is_empty() || pending > 0) {
            let batch = match pipeline.pending(pending) {
                Some(behind) => Err(behind),
                None => {
                    let batch = pipeline.feed(envelopes);
                    batch.fallback.map_or(Ok(batch), Err)
                }
            };
            match batch {
                Err(reason) => {
                    h264 = false;
                    out(Decoded::Fallback(reason));
                }
                Ok(batch) => {
                    if let Some(p) = batch.picture {
                        last_jpeg = None;
                        picture = Some((avcc_seq, p));
                    } else if let Some(seed) = batch.seed {
                        jpeg = jpeg.or(Some((avcc_seq, seed)));
                    }
                }
            }
        }
        #[cfg(not(target_os = "macos"))]
        let _ = (envelopes, pending, avcc_seq);
        // (A still screen's MJPEG resends the same picture: decoded and shown once.)
        if picture.is_none()
            && let Some((seq, bytes)) = jpeg
            && last_jpeg.as_deref() != Some(bytes.as_slice())
        {
            picture = device_video::decode_still(&bytes).map(|p| (seq, p));
            last_jpeg = Some(bytes);
        }
        let rotation = byte_orientation(orientation.load(Ordering::Acquire));
        let (seq, width, height, pixels): (u64, u32, u32, Arc<[u8]>) = match picture {
            Some((seq, p)) => (seq, p.width, p.height, p.pixels.into()),
            None if turned => match &last {
                Some(kept) => kept.clone(),
                None => continue,
            },
            None => continue,
        };
        let shown = if rotation == DeviceOrientation::Portrait {
            zgx::image::ImageData::new(width, height, pixels.clone())
        } else {
            let turned = device_video::rotate(&device_video::Rgba { width, height, pixels: pixels.to_vec() }, rotation);
            zgx::image::ImageData::new(turned.width, turned.height, turned.pixels)
        };
        if let Ok(image) = shown {
            out(Decoded::Picture(seq, Arc::new(image)));
        }
        last = Some((seq, width, height, pixels));
    }
}

fn image_name(key: &str) -> String {
    format!("device/{key}")
}

struct Stream {
    generation: u64,
    host_id: Option<String>,
    device_id: String,
    /// An Android device's stream is H.264 only: no MJPEG to fall back to.
    android: bool,
    codec: &'static str,
    orientation: Arc<AtomicU8>,
    decode: std::sync::mpsc::Sender<Payload>,
    watch: Option<engine::rpc::Watch>,
    /// The engine ended its watch: a frame decoded since keeps the failure.
    ended: bool,
}

struct Window {
    host: ScriptHost,
    hub: Shared,
    frames: Shared,
    sizes: RefCell<HashMap<String, (u64, u32, u32)>>,
    shown: RefCell<HashMap<String, (u64, Arc<zgx::image::ImageData>)>>,
    /// Streams the engine ended, and why.
    failed: RefCell<HashMap<String, String>>,
    /// The engine's connection last heard: each new one watches the hub and
    /// every stream again (a watch ends with its connection).
    rpc: RefCell<Option<engine::rpc::Rpc>>,
    watch: RefCell<Option<engine::rpc::Watch>>,
    watching: Cell<bool>,
    /// Which arming of the hub's watch is the live one.
    armed: Cell<u64>,
    streams: RefCell<HashMap<String, Stream>>,
    next: Cell<u64>,
    three_d: RefCell<HashMap<String, Rc<ThreeD>>>,
    folds: Shared,
    fold_errors: RefCell<HashMap<String, String>>,
}

thread_local! {
    static MOCK: Rc<RefCell<Mock>> = Rc::new(RefCell::new(Mock::default()));
    static WINDOWS: RefCell<Vec<std::rc::Weak<Window>>> = const { RefCell::new(Vec::new()) };
}

impl Window {
    fn new(host: &ScriptHost) -> Rc<Self> {
        Rc::new(Window {
            host: host.clone(),
            hub: host.shared("device_hub", Value::Nil),
            frames: host.shared("device_frames", Value::record(std::iter::empty::<(&str, Value)>())),
            sizes: RefCell::default(),
            shown: RefCell::default(),
            failed: RefCell::default(),
            rpc: RefCell::default(),
            watch: RefCell::default(),
            watching: Cell::new(false),
            armed: Cell::new(0),
            streams: RefCell::default(),
            next: Cell::new(0),
            three_d: RefCell::default(),
            folds: host.shared("device_3d", Value::record(std::iter::empty::<(&str, Value)>())),
            fold_errors: RefCell::default(),
        })
    }

    fn set_fold_error(&self, key: &str, error: Option<String>) {
        let mut errors = self.fold_errors.borrow_mut();
        let changed = match error {
            Some(error) => errors.insert(key.to_string(), error.clone()).as_ref() != Some(&error),
            None => errors.remove(key).is_some(),
        };
        if changed {
            self.folds.set(Value::record(errors.iter().map(|(k, why)| (k.as_str(), Value::record([("fold_error", Value::from(why.as_str()))])))));
        }
    }

    fn set_three_d(self: &Rc<Self>, key: &str, on: bool, platform: DevicePlatform, name: String, host_id: Option<String>, device_id: String) {
        if !on {
            if self.three_d.borrow_mut().remove(key).is_some() {
                self.host.gpu_view(&device3d::view_name(key), None);
                self.set_fold_error(key, None);
            }
            return;
        }
        if let Some(shown) = self.three_d.borrow().get(key) {
            shown.set_device(platform, name);
            return;
        }
        let hinge = Rc::new(Hinge { window: Rc::downgrade(self), key: key.to_string(), host_id, device_id });
        let shown = ThreeD::new(platform, name, hinge);
        let current = self.streams.borrow().get(key).map(|s| byte_orientation(s.orientation.load(Ordering::Acquire)));
        if let (Some(rotation), Some((_, image))) = (current, self.shown.borrow().get(key).cloned()) {
            shown.show(image, rotation);
        }
        self.host.gpu_view(&device3d::view_name(key), Some(shown.view()));
        self.three_d.borrow_mut().insert(key.to_string(), shown);
    }

    fn shown_3d(&self, key: &str) -> Option<Rc<ThreeD>> {
        self.three_d.borrow().get(key).cloned()
    }

    fn publish_frames(&self) {
        let sizes = self.sizes.borrow();
        let failed = self.failed.borrow();
        let mut keys: Vec<&String> = sizes.keys().chain(failed.keys()).collect();
        keys.sort();
        keys.dedup();
        self.frames.set(Value::record(keys.into_iter().map(|k| {
            let mut fields = Vec::new();
            if let Some((seq, w, h)) = sizes.get(k) {
                fields.extend([("seq", Value::from(*seq as f64)), ("w", Value::from(*w as f64)), ("h", Value::from(*h as f64))]);
            }
            if let Some(why) = failed.get(k) {
                fields.push(("failed", Value::from(why.as_str())));
            }
            (k.as_str(), Value::record(fields))
        })));
    }

    fn show(&self, key: &str, generation: u64, seq: u64, image: Arc<zgx::image::ImageData>) {
        if self.streams.borrow().get(key).map(|s| s.generation) != Some(generation) {
            return;
        }
        let ended = self.streams.borrow().get(key).is_some_and(|s| s.ended);
        let (w, h) = (image.width(), image.height());
        if let Some(shown) = self.shown_3d(key) {
            let rotation = self.streams.borrow().get(key).map_or(DeviceOrientation::Portrait, |s| byte_orientation(s.orientation.load(Ordering::Acquire)));
            shown.show(image.clone(), rotation);
        }
        self.shown.borrow_mut().insert(key.to_string(), (seq, image.clone()));
        self.host.image(&image_name(key), Some(image));
        self.sizes.borrow_mut().insert(key.to_string(), (seq, w, h));
        if !ended {
            self.failed.borrow_mut().remove(key);
        }
        self.publish_frames();
    }

    fn unstream(&self, key: &str) {
        let failed = self.failed.borrow_mut().remove(key).is_some();
        if self.streams.borrow_mut().remove(key).is_some() || failed {
            self.host.image(&image_name(key), None);
            self.shown.borrow_mut().remove(key);
            self.sizes.borrow_mut().remove(key);
            self.publish_frames();
        }
    }

    /// The hub followed (`device.watch`): on the connection there is now,
    /// else on the next.
    fn set_watching(self: &Rc<Self>, on: bool) {
        if on == self.watching.get() {
            return;
        }
        self.watching.set(on);
        if !on {
            self.armed.set(self.armed.get() + 1);
            self.watch.borrow_mut().take();
            return;
        }
        if mock() {
            self.hub.set(json_value(&MOCK.with(|m| m.borrow().state())));
            return;
        }
        let rpc = self.rpc.borrow().clone().filter(|rpc| !rpc.is_closed());
        if let Some(work) = rpc.and_then(|rpc| self.arm(&rpc)) {
            engine::get().spawn(work);
        }
    }

    /// A new connection to the engine: the hub watched on it (while
    /// followed), and each stream. What's handed back is the hub's watch
    /// to run on the connection's tasks.
    fn connected(self: &Rc<Self>, rpc: &engine::rpc::Rpc) -> Option<impl std::future::Future<Output = ()> + 'static + use<>> {
        *self.rpc.borrow_mut() = Some(rpc.clone());
        let mut keys: Vec<String> = self.streams.borrow().keys().cloned().collect();
        keys.sort();
        for key in keys {
            self.watch_frames(&key, rpc);
        }
        self.arm(rpc)
    }

    /// `EngineInfo`'s capability, then `ListHubDevices { inspectOnly }` (the
    /// hub starts lazily), then `WatchDeviceHub` into `device_hub`, on `rpc`.
    fn arm(self: &Rc<Self>, rpc: &engine::rpc::Rpc) -> Option<impl std::future::Future<Output = ()> + 'static + use<>> {
        if !self.watching.get() || mock() {
            return None;
        }
        let generation = self.armed.get() + 1;
        self.armed.set(generation);
        self.watch.borrow_mut().take();
        let me = Rc::downgrade(self);
        let rpc = rpc.clone();
        Some(async move {
            let live = |me: &std::rc::Weak<Window>| me.upgrade().filter(|w| w.watching.get() && w.armed.get() == generation && !rpc.is_closed());
            let info = rpc.call("EngineInfo", json!({})).await.unwrap_or(Json::Null);
            let Some(window) = live(&me) else { return };
            if !info.is_null() && !supports(&info) {
                window.hub.set(Value::record([("outdated", Value::Bool(true))]));
                return;
            }
            drop(window);
            let listed = rpc.call("ListHubDevices", json!({ "inspectOnly": true })).await;
            let Some(window) = live(&me) else { return };
            if let Err(error) = &listed
                && said(error) == "outdated"
            {
                window.hub.set(Value::record([("outdated", Value::Bool(true))]));
                return;
            }
            let heard = Rc::downgrade(&window);
            let watch = rpc.watch("WatchDeviceHub", json!({}), move |state| {
                if let Some(window) = heard.upgrade() {
                    window.hub.set(json_value(&state));
                }
            });
            *window.watch.borrow_mut() = Some(watch);
        })
    }

    /// The stream `key`'s frames watched on `rpc`. Ended by the engine, it
    /// says why (`device_frames[key].failed`); ended with its connection,
    /// the next one watches again.
    fn watch_frames(self: &Rc<Self>, key: &str, rpc: &engine::rpc::Rpc) {
        if mock() || rpc.is_closed() {
            return;
        }
        let mut streams = self.streams.borrow_mut();
        let Some(stream) = streams.get_mut(key) else { return };
        let decode = stream.decode.clone();
        let mut params = json!({ "deviceId": stream.device_id, "codec": stream.codec });
        if let Some(host) = stream.host_id.as_ref().filter(|h| *h != zeron_proto::device_hub::LOCAL_DEVICE_HOST_ID) {
            params["hostId"] = host.clone().into();
        }
        let generation = stream.generation;
        let (me, ended) = (Rc::downgrade(self), key.to_string());
        stream.watch.take();
        stream.ended = false;
        // (Every H.264 frame goes to the decoder, which takes the newest picture of each batch.)
        stream.watch = Some(rpc.watch_ending(
            "WatchHubDeviceFrames",
            params,
            move |item| {
                if let Some(frame) = frame_payload(&item) {
                    let _ = decode.send(frame);
                }
            },
            move |why| {
                if why != "closed"
                    && let Some(window) = me.upgrade()
                {
                    window.stream_ended(&ended, generation, &why);
                }
            },
        ));
        drop(streams);
        if self.failed.borrow_mut().remove(key).is_some() {
            self.publish_frames();
        }
    }

    fn stream_ended(&self, key: &str, generation: u64, why: &str) {
        let mut streams = self.streams.borrow_mut();
        let Some(stream) = streams.get_mut(key).filter(|s| s.generation == generation) else { return };
        stream.watch = None;
        stream.ended = true;
        drop(streams);
        self.failed.borrow_mut().insert(key.to_string(), said(why));
        self.publish_frames();
    }

    fn stream(self: &Rc<Self>, key: String, host_id: Option<String>, device_id: String, android: bool, orientation: DeviceOrientation) {
        let generation = self.next.get() + 1;
        self.next.set(generation);
        if self.failed.borrow_mut().remove(&key).is_some() {
            self.publish_frames();
        }
        let me = Rc::downgrade(self);
        let (decode, frames) = std::sync::mpsc::channel::<Payload>();
        let (shown, mut decoded) = zgx::task::latest::<Decoded>();
        let turned = Arc::new(AtomicU8::new(orientation_byte(orientation)));
        let held = turned.clone();
        std::thread::spawn(move || decode_frames(frames, held, |d| {
            shown.send(d);
        }));
        let show_key = key.clone();
        engine::get().spawn(async move {
            while let Some(d) = decoded.recv().await {
                let Some(window) = me.upgrade() else { return };
                match d {
                    Decoded::Picture(seq, image) => window.show(&show_key, generation, seq, image),
                    Decoded::Fallback(reason) => window.fell_back(&show_key, generation, reason),
                }
            }
        });
        if mock() && device_id == MOCK_DUO {
            let (me, key) = (Rc::downgrade(self), key.clone());
            engine::get().spawn(async move {
                if let Some(window) = me.upgrade() {
                    window.show_mock_duo(&key, generation);
                }
            });
        } else if mock() {
            let feed = decode.clone();
            engine::get().spawn(async move {
                for (n, png) in MOCK_FRAMES.iter().enumerate() {
                    if feed.send(Payload::Jpeg(n as u64 + 1, png.to_vec())).is_err() {
                        return;
                    }
                    zgx::timer::sleep(MOCK_FRAME_EVERY).await;
                }
            });
        }
        self.streams.borrow_mut().insert(
            key.clone(),
            Stream { generation, host_id, device_id, android, codec: first_codec(), orientation: turned, decode, watch: None, ended: false },
        );
        let rpc = self.rpc.borrow().clone();
        if let Some(rpc) = rpc {
            self.watch_frames(&key, &rpc);
        }
    }

    fn show_mock_duo(&self, key: &str, generation: u64) {
        let angle = MOCK.with(|m| m.borrow().hinge.0);
        if let Some(image) = mock_duo_frame(angle) {
            let seq = self.sizes.borrow().get(key).map_or(1, |(seq, ..)| seq + 1);
            self.show(key, generation, seq, image);
        }
    }

    /// The device turned: its picture turns at once, and each one after.
    fn orient(&self, key: &str, orientation: DeviceOrientation) {
        if let Some(stream) = self.streams.borrow().get(key)
            && stream.orientation.swap(orientation_byte(orientation), Ordering::AcqRel) != orientation_byte(orientation)
        {
            let _ = stream.decode.send(Payload::Turn);
        }
    }

    /// H.264 couldn't be shown: an iOS stream is asked again as MJPEG (gpui's
    /// fallback); an Android one has none, so the tab says why, with Try again.
    fn fell_back(self: &Rc<Self>, key: &str, generation: u64, reason: FallbackReason) {
        let mut streams = self.streams.borrow_mut();
        let Some(stream) = streams.get_mut(key).filter(|s| s.generation == generation && s.codec == "h264") else { return };
        if stream.android {
            stream.watch = None;
            stream.ended = true;
            drop(streams);
            self.failed.borrow_mut().insert(key.to_string(), device_video::undecodable(reason.describe()));
            self.publish_frames();
            return;
        }
        stream.codec = "mjpeg";
        drop(streams);
        let rpc = self.rpc.borrow().clone().filter(|rpc| !rpc.is_closed());
        if let Some(rpc) = rpc {
            self.watch_frames(key, &rpc);
        }
    }
}

struct Hinge {
    window: std::rc::Weak<Window>,
    key: String,
    host_id: Option<String>,
    device_id: String,
}

impl device3d::Host for Hinge {
    fn set_hinge(&self, command: HingeCommand, done: Box<dyn FnOnce(Result<(), String>)>) {
        let mut params = json!({ "deviceId": self.device_id });
        if let Some(host) = self.host_id.as_ref().filter(|h| *h != zeron_proto::device_hub::LOCAL_DEVICE_HOST_ID) {
            params["hostId"] = host.clone().into();
        }
        if let (Some(target), Ok(Json::Object(fields))) = (params.as_object_mut(), serde_json::to_value(command)) {
            target.extend(fields);
        }
        engine::get().spawn(async move {
            done(call("SetHubDeviceHinge".into(), params).await.map(|_| ()));
        });
    }

    fn fold_error(&self, error: Option<String>) {
        if let Some(window) = self.window.upgrade() {
            window.set_fold_error(&self.key, error);
        }
    }
}

fn touches_value(touches: Vec<(zeron_proto::device_hub::TouchPhase, (f32, f32))>) -> Value {
    Value::List(
        touches
            .into_iter()
            .map(|(phase, (x, y))| {
                Value::record([("phase", Value::from(device3d::phase_word(phase))), ("x", Value::from(f64::from(x))), ("y", Value::from(f64::from(y)))])
            })
            .collect(),
    )
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
            let duos: Vec<(String, u64)> =
                window.streams.borrow().iter().filter(|(_, s)| s.device_id == MOCK_DUO).map(|(k, s)| (k.clone(), s.generation)).collect();
            for (key, generation) in duos {
                window.show_mock_duo(&key, generation);
            }
        }
    });
}

pub fn install(cx: &mut WindowContext, host: &ScriptHost) {
    let window = Window::new(host);
    WINDOWS.with(|all| {
        let mut all = all.borrow_mut();
        all.retain(|w| w.strong_count() > 0);
        all.push(Rc::downgrade(&window));
    });
    let heard = Rc::downgrade(&window);
    engine::get().on_connect(move |rpc| {
        if let Some(window) = heard.upgrade()
            && let Some(work) = window.connected(rpc)
        {
            engine::get().spawn(work);
        }
    });
    let closing = window.clone();
    cx.on_closed(move || {
        closing.watching.set(false);
        closing.watch.borrow_mut().take();
        closing.streams.borrow_mut().clear();
        closing.three_d.borrow_mut().clear();
        closing.rpc.borrow_mut().take();
    });
    host.async_action("device.call", |args| {
        let method = args.field("method").map(text).unwrap_or_default();
        let params = args.field("params").map(value_json).unwrap_or(json!({}));
        call(method, params)
    });
    let w = window.clone();
    host.action("device.watch", move |on| {
        w.set_watching(on.truthy());
    });
    let w = window.clone();
    host.action("device.stream", move |args| {
        let key = args.field("key").map(text).unwrap_or_default();
        let device = args.field("device_id").map(text).unwrap_or_default();
        if key.is_empty() || device.is_empty() {
            return;
        }
        let host_id = args.field("host_id").map(text).filter(|h| !h.is_empty());
        let android = args.field("platform").map(text).as_deref() == Some("android");
        let orientation = orientation_of(&args.field("orientation").map(text).unwrap_or_default());
        w.stream(key, host_id, device, android, orientation);
    });
    let w = window.clone();
    host.action("device.orient", move |args| {
        let key = args.field("key").map(text).unwrap_or_default();
        w.orient(&key, orientation_of(&args.field("orientation").map(text).unwrap_or_default()));
    });
    let w = window.clone();
    host.action("device.unstream", move |key| {
        w.unstream(&text(&key));
    });
    install_three_d(host, &window);
}

fn install_three_d(host: &ScriptHost, window: &Rc<Window>) {
    let num = |args: &Value, name: &str| args.field(name).and_then(Value::as_f64).unwrap_or(0.);
    let key_of = |args: &Value| args.field("key").map(text).unwrap_or_default();
    let w = window.clone();
    host.action("device.three_d", move |args| {
        let key = key_of(&args);
        if key.is_empty() {
            return;
        }
        let platform = if args.field("platform").map(text).as_deref() == Some("android") { DevicePlatform::Android } else { DevicePlatform::Ios };
        let name = args.field("name").map(text).unwrap_or_default();
        let host_id = args.field("host_id").map(text).filter(|h| !h.is_empty());
        let device = args.field("device_id").map(text).unwrap_or_default();
        w.set_three_d(&key, args.field("on").is_some_and(Value::truthy), platform, name, host_id, device);
    });
    let w = window.clone();
    host.function("device.screen3d", move |args| {
        let Some(shown) = w.shown_3d(&key_of(&args)) else { return Value::List(Rc::from([])) };
        let screen = args.field("screen").map(value_json).and_then(|j| serde_json::from_value::<DeviceScreenConfig>(j).ok());
        touches_value(shown.set_screen(screen))
    });
    let w = window.clone();
    host.function("device.pointer3d", move |args| {
        let (Some(shown), Some(phase)) = (w.shown_3d(&key_of(&args)), device3d::touch_phase(&args.field("phase").map(text).unwrap_or_default())) else {
            return Value::List(Rc::from([]));
        };
        touches_value(shown.pointer(phase, num(&args, "x"), num(&args, "y"), args.field("orbit").is_some_and(Value::truthy)))
    });
    let w = window.clone();
    host.function("device.wheel3d", move |args| {
        let Some(shown) = w.shown_3d(&key_of(&args)) else { return Value::Bool(false) };
        Value::Bool(shown.wheel(num(&args, "dx"), num(&args, "dy"), num(&args, "x"), num(&args, "y"), args.field("ctrl").is_some_and(Value::truthy)))
    });
    let w = window.clone();
    host.function("device.pinch3d", move |args| {
        let Some(shown) = w.shown_3d(&key_of(&args)) else { return Value::Bool(false) };
        Value::Bool(shown.pinch(num(&args, "scale"), num(&args, "x"), num(&args, "y")))
    });
    let w = window.clone();
    host.action("device.restore3d", move |key| {
        if let Some(shown) = w.shown_3d(&text(&key)) {
            shown.restore();
        }
    });
    let w = window.clone();
    host.action("device.pose3d", move |args| {
        let pose = serde_json::from_value::<HingePose>(Json::from(args.field("pose").map(text).unwrap_or_default()));
        if let (Some(shown), Ok(pose)) = (w.shown_3d(&key_of(&args)), pose) {
            shown.pose(pose);
        }
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
    orientation: String,
    hinge: (u8, Option<HingePose>),
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
            orientation: "portrait".into(),
            hinge: (0, Some(HingePose::Closed)),
        }
    }
}

const MOCK_DEVICES: [(&str, &str, &str, &str); 4] = [
    ("mock-iphone", "ios", "iPhone 17", "iOS 27.0"),
    ("mock-ipad", "ios", "iPad Air", "iOS 27.0"),
    ("Pixel_9", "android", "Pixel 9", "Android 16"),
    (MOCK_DUO, "ios", "iPhone Duo", "iOS 27.0"),
];
const MOCK_DUO: &str = "mock-duo";

fn mock_duo_panel(angle: u8) -> (u32, u32, u32) {
    if angle == 0 { (1, device3d::duo3d::COVER_SIZE.0, device3d::duo3d::COVER_SIZE.1) } else { (3, device3d::duo3d::INNER_SIZE.0, device3d::duo3d::INNER_SIZE.1) }
}

fn mock_duo_frame(angle: u8) -> Option<Arc<zgx::image::ImageData>> {
    let (screen, w, h) = mock_duo_panel(angle);
    let (w, h) = (w / 4, h / 4);
    let mut rgba = Vec::with_capacity((w * h * 4) as usize);
    for y in 0..h {
        for x in 0..w {
            let (fx, fy) = (x as f32 / w as f32, y as f32 / h as f32);
            let grid = x % 40 < 2 || y % 40 < 2;
            let (r, g, b) = if screen == 1 { (40.0 + 60.0 * fx, 80.0 + 80.0 * fy, 200.0) } else { (220.0, 120.0 + 80.0 * fx, 60.0 + 60.0 * fy) };
            let shade = if grid { 0.6 } else { 1.0 };
            rgba.extend([(r * shade) as u8, (g * shade) as u8, (b * shade) as u8, 255]);
        }
    }
    zgx::image::ImageData::new(w, h, rgba).ok().map(Arc::new)
}

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
            if key == MOCK_DUO {
                let (screen, width, height) = mock_duo_panel(self.hinge.0);
                screens.insert(key.clone(), json!({ "width": width, "height": height, "orientation": self.orientation,
                    "supportsHingeAngle": true, "screenId": screen, "hingeAngle": self.hinge.0, "hingePose": self.hinge.1 }));
                streams.insert(key, json!({ "video": "live", "input": "live", "subscribers": 1 }));
                continue;
            }
            screens.insert(key.clone(), json!({ "width": 196, "height": 426, "orientation": self.orientation }));
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
                if let Some(turned) = params["input"]["orientation"]["orientation"].as_str() {
                    self.orientation = turned.to_string();
                }
                self.inputs.push(params["input"].clone());
                Ok(json!({}))
            }
            "SetHubDeviceHinge" => {
                if params["deviceId"].as_str() != Some(MOCK_DUO) {
                    return Err("This device has no hinge.".into());
                }
                if let Ok(pose) = serde_json::from_value::<HingePose>(params["pose"].clone()) {
                    let angle = match pose {
                        HingePose::Closed => 0,
                        HingePose::Book => 90,
                        HingePose::Open => 180,
                        HingePose::Laptop => 110,
                        HingePose::Tent => 60,
                    };
                    self.hinge = (angle, Some(pose));
                } else if let Some(angle) = params["angle"].as_f64() {
                    self.hinge = (angle.round().clamp(0.0, 180.0) as u8, None);
                } else if let Some(turned) = params["orientation"].as_str() {
                    self.orientation = turned.to_string();
                }
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

    /// The hub's watch and a tab's stream, asked for before the engine
    /// answers (a slow launch), start on its first connection, and again on
    /// each one after (a restart): what the old one says is no longer heard.
    /// A stream the engine ends says why; the next connection watches it again.
    #[cfg(unix)]
    #[test]
    fn the_hub_and_each_stream_are_watched_again_on_every_connection() {
        use crate::engine::rpc::Rpc;
        use std::future::Future;
        if mock() {
            return;
        }
        let entry = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/activity_fixture.luau");
        let mut made = None;
        let _app = zgx_script::testing::TestApp::new(|runtime| {
            let host = ScriptHost::new(runtime, entry);
            made = Some(Window::new(&host));
            host
        });
        let window = made.unwrap();
        fn sent(rx: &std::sync::mpsc::Receiver<String>) -> Vec<Json> {
            rx.try_iter().map(|f| serde_json::from_str(&f).unwrap()).collect()
        }
        let waker = std::task::Waker::noop();
        let mut cx = std::task::Context::from_waker(waker);
        // (Each step of the hub's watch answered as an engine with devices would.)
        fn arm(rpc: &Rpc, rx: &std::sync::mpsc::Receiver<String>, work: Option<impl Future<Output = ()>>, cx: &mut std::task::Context) -> u64 {
            let mut work = Box::pin(work.expect("the hub watched on this connection"));
            assert!(work.as_mut().poll(cx).is_pending());
            let info = sent(rx);
            assert_eq!(info.last().unwrap()["method"], "EngineInfo", "{info:?}");
            rpc.heard(json!({ "id": info.last().unwrap()["id"], "ok": { "capabilities": ["device-hub-v1"] } }));
            assert!(work.as_mut().poll(cx).is_pending());
            let listed = sent(rx);
            assert_eq!((listed[0]["method"].as_str(), listed[0]["params"]["inspectOnly"].as_bool()), (Some("ListHubDevices"), Some(true)));
            rpc.heard(json!({ "id": listed[0]["id"], "ok": {} }));
            assert!(work.as_mut().poll(cx).is_ready());
            let watched = sent(rx);
            assert_eq!(watched.len(), 1, "{watched:?}");
            assert_eq!(watched[0]["method"], "WatchDeviceHub");
            watched[0]["id"].as_u64().unwrap()
        }
        let revision = |w: &Window| value_json(&w.hub.get())["revision"].as_u64();
        let frames = |w: &Window| value_json(&w.frames.get());

        window.set_watching(true);
        window.stream("tab-1".into(), None, "Pixel_9".into(), true, DeviceOrientation::Portrait);
        assert_eq!(revision(&window), None, "no engine yet: nothing heard");

        let (first, first_sent, _bell) = Rpc::recording();
        let work = window.connected(&first);
        let streamed = sent(&first_sent);
        assert_eq!(streamed.len(), 1, "{streamed:?}");
        assert_eq!(streamed[0]["method"], "WatchHubDeviceFrames");
        assert_eq!((streamed[0]["params"]["deviceId"].as_str(), streamed[0]["params"]["codec"].as_str()), (Some("Pixel_9"), Some(first_codec())));
        let hub_watch = arm(&first, &first_sent, work, &mut cx);
        first.heard(json!({ "id": hub_watch, "item": { "revision": 1 } }));
        assert_eq!(revision(&window), Some(1));

        // The engine restarted: a new connection, watched again.
        let (second, second_sent, _bell) = Rpc::recording();
        let work = window.connected(&second);
        let restreamed = sent(&second_sent);
        assert_eq!(restreamed[0]["method"], "WatchHubDeviceFrames");
        let frames_watch = restreamed[0]["id"].as_u64().unwrap();
        let hub_watch_again = arm(&second, &second_sent, work, &mut cx);
        second.heard(json!({ "id": hub_watch_again, "item": { "revision": 2 } }));
        assert_eq!(revision(&window), Some(2));
        first.heard(json!({ "id": hub_watch, "item": { "revision": 9 } }));
        assert_eq!(revision(&window), Some(2), "the old connection's watch let go");

        // The engine ends the stream: it says why, and the next connection tries again.
        second.heard(json!({ "id": frames_watch, "err": "The device stopped." }));
        assert_eq!(frames(&window)["tab-1"]["failed"], "The device stopped.");
        let live = window.streams.borrow()["tab-1"].generation;
        let late = zgx_gpu::assets::decode_image(MOCK_FRAMES[0]).unwrap();
        window.show("tab-1", live, 4, late);
        assert_eq!(frames(&window)["tab-1"]["seq"], 4);
        assert_eq!(frames(&window)["tab-1"]["failed"], "The device stopped.");
        let (third, third_sent, _bell) = Rpc::recording();
        let _ = window.connected(&third);
        assert_eq!(sent(&third_sent)[0]["method"], "WatchHubDeviceFrames");
        assert_eq!(frames(&window)["tab-1"]["failed"], Json::Null);

        // Let go: no watch on the next connection.
        window.set_watching(false);
        window.unstream("tab-1");
        let (fourth, fourth_sent, _bell) = Rpc::recording();
        assert!(window.connected(&fourth).is_none());
        assert!(sent(&fourth_sent).is_empty());
    }

    #[test]
    fn a_frame_is_its_jpeg_or_its_avcc() {
        use base64::Engine as _;
        let jpeg = base64::engine::general_purpose::STANDARD.encode([0xFF, 0xD8, 1, 2]);
        assert_eq!(frame_payload(&json!({ "seq": 7, "jpeg": jpeg })), Some(Payload::Jpeg(7, vec![0xFF, 0xD8, 1, 2])));
        assert_eq!(frame_payload(&json!({ "seq": 8, "jpeg": "", "avcc": "AAAA" })), Some(Payload::Avcc(8, vec![0, 0, 0])));
        assert_eq!(frame_payload(&json!({ "seq": 9, "jpeg": "not base64!" })), None);
        assert_eq!(orientation_of("landscape_left"), DeviceOrientation::LandscapeLeft);
        assert_eq!(orientation_of("sideways"), DeviceOrientation::Portrait);
    }

    /// The decoder thread: a still turns with the device, and turns again
    /// when only the orientation changes; H.264 it can't show says so once,
    /// and JPEG still shows after.
    #[test]
    fn the_decoder_turns_pictures_and_falls_back_from_h264() {
        let (tx, rx) = std::sync::mpsc::channel();
        let orientation = Arc::new(AtomicU8::new(orientation_byte(DeviceOrientation::Portrait)));
        let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
        let (o, out) = (orientation.clone(), seen.clone());
        let worker = std::thread::spawn(move || {
            decode_frames(rx, o, |d| {
                out.lock().unwrap().push(match d {
                    Decoded::Picture(seq, image) => format!("{seq}:{}x{}", image.width(), image.height()),
                    Decoded::Fallback(reason) => format!("fallback:{reason:?}"),
                })
            })
        });
        tx.send(Payload::Jpeg(1, MOCK_FRAMES[0].to_vec())).unwrap();
        orientation.store(orientation_byte(DeviceOrientation::LandscapeLeft), Ordering::Release);
        std::thread::sleep(Duration::from_millis(200));
        tx.send(Payload::Turn).unwrap();
        std::thread::sleep(Duration::from_millis(200));
        // A delta with no description before it: no H.264 to show.
        let mut delta = Vec::new();
        zeron_proto::device_hub::AvccEnvelope { tag: zeron_proto::device_hub::AVCC_DELTA, payload: vec![0, 0, 0, 1, 0x41] }.encode_into(&mut delta);
        tx.send(Payload::Avcc(2, delta.clone())).unwrap();
        std::thread::sleep(Duration::from_millis(200));
        tx.send(Payload::Avcc(3, delta)).unwrap();
        tx.send(Payload::Jpeg(4, MOCK_FRAMES[1].to_vec())).unwrap();
        drop(tx);
        worker.join().unwrap();
        let seen = seen.lock().unwrap().clone();
        let first = seen[0].clone();
        assert!(first == "1:196x426" || first == "1:426x196", "{seen:?}");
        assert!(seen.contains(&"1:426x196".to_string()), "turned on its side: {seen:?}");
        if cfg!(target_os = "macos") {
            assert_eq!(seen.iter().filter(|s| s.starts_with("fallback")).count(), 1, "{seen:?}");
            assert!(seen.contains(&"fallback:NoDescription".to_string()), "{seen:?}");
        }
        assert_eq!(seen.last().unwrap(), "4:426x196", "{seen:?}");
    }

    #[test]
    fn a_still_screen_s_repeated_jpeg_is_shown_once() {
        let (tx, rx) = std::sync::mpsc::channel();
        let orientation = Arc::new(AtomicU8::new(orientation_byte(DeviceOrientation::Portrait)));
        let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
        let out = seen.clone();
        let worker = std::thread::spawn(move || {
            decode_frames(rx, orientation, |d| {
                if let Decoded::Picture(seq, _) = d {
                    out.lock().unwrap().push(seq);
                }
            })
        });
        for (seq, frame) in [(1, 0), (2, 0), (3, 0), (4, 1), (5, 1), (6, 0)] {
            tx.send(Payload::Jpeg(seq, MOCK_FRAMES[frame].to_vec())).unwrap();
            std::thread::sleep(Duration::from_millis(100));
        }
        tx.send(Payload::Turn).unwrap();
        drop(tx);
        worker.join().unwrap();
        assert_eq!(*seen.lock().unwrap(), vec![1, 4, 6, 6], "a repeat is skipped, a turn still turns the last");
    }

    #[test]
    fn the_mock_hub_sets_up_opens_a_device_and_streams_still_frames() {
        let mut hub = Mock::default();
        let off = hub.state();
        assert_eq!(off["hostStatus"], "disabled");
        assert_eq!(off["devices"], json!([]), "off, it lists nothing");
        let on = hub.call("ConfigureDeviceHub", &json!({ "enabled": true })).unwrap();
        assert_eq!((on["hostStatus"].as_str(), on["devices"].as_array().map(Vec::len)), (Some("ready"), Some(4)));
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
