use std::cell::{Cell, RefCell};
use std::collections::{HashSet, VecDeque};
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use base64::Engine as _;
use futures::StreamExt;
use futures::channel::mpsc;
use gpui::{
    App, Bounds, Context, FocusHandle, KeyDownEvent, KeyUpEvent, ModifiersChangedEvent,
    MouseButton, MouseMoveEvent, Pixels, Point, RenderImage, SharedString, Subscription, Task,
    Window, div, prelude::*, px,
};
use zeron_proto::device_hub::{
    DeviceButton, DeviceFrame, DeviceInput, DeviceOrientation, DevicePlatform, DeviceScreenConfig,
    DeviceStreamStatus, DeviceVideoCodec, HingeCommand, StreamLinkStatus, TouchPhase,
};
use zeron_rpc::{RpcSubscription, methods};

use super::duo3d::{self, DuoPanel, DuoViewer};
use super::fold;
use super::geometry::{self, KeyPress};
use super::loading::{self, LoadingStage, LoadingView};
use super::scene3d::{self, Scene3d};
use crate::state::EngineHandle;
use crate::theme::Theme;

const RESUBSCRIBE_DELAY: Duration = Duration::from_secs(1);
const FPS_LOG_INTERVAL: Duration = Duration::from_secs(5);
const LINK_TICK: Duration = Duration::from_secs(1);
const LINK_FAILED_TICKS: u32 = 3;
const KEPT_SURFACES: usize = 3;
#[cfg(target_os = "macos")]
const RENDER_ATTEMPTS: u32 = 3;
pub const NO_VIDEO_TIMEOUT: Duration = Duration::from_secs(15);
pub const NO_VIDEO: &str = "No video received from the device. Reconnect to try again.";

gpui::actions!(device_panel, [SendCommandR]);

pub(crate) fn bind_keys(cx: &mut App) {
    cx.bind_keys([gpui::KeyBinding::new(
        &crate::settings::platform_combo("mod-r"),
        SendCommandR,
        Some("DeviceScreen"),
    )]);
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AxBox {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    pub oriented: bool,
}

impl AxBox {
    pub fn displayed(self, rotation: DeviceOrientation) -> AxBox {
        if self.oriented {
            return self;
        }
        let (x0, y0) = geometry::display_point(rotation, self.x, self.y);
        let (x1, y1) = geometry::display_point(rotation, self.x + self.width, self.y + self.height);
        AxBox {
            x: x0.min(x1),
            y: y0.min(y1),
            width: (x1 - x0).abs(),
            height: (y1 - y0).abs(),
            oriented: true,
        }
    }
}

#[derive(Clone)]
enum Picture {
    Image(Arc<RenderImage>),
    #[cfg(target_os = "macos")]
    Surface(super::h264::Surface),
}

struct DecodedFrame {
    picture: Picture,
    width: u32,
    height: u32,
    rotation: DeviceOrientation,
    codec: DeviceVideoCodec,
}

impl DecodedFrame {
    fn into_image(self) -> Option<Arc<RenderImage>> {
        match self.picture {
            Picture::Image(image) => Some(image),
            #[cfg(target_os = "macos")]
            Picture::Surface(_) => None,
        }
    }
}

fn codec_label(codec: DeviceVideoCodec) -> &'static str {
    match codec {
        DeviceVideoCodec::Mjpeg => "mjpeg",
        DeviceVideoCodec::H264 => "h264",
    }
}

fn codec_choice(platform: DevicePlatform, knob: Option<&str>) -> (DeviceVideoCodec, bool) {
    let h264 = cfg!(target_os = "macos");
    match knob {
        _ if platform == DevicePlatform::Android => (DeviceVideoCodec::H264, true),
        Some("mjpeg") => (DeviceVideoCodec::Mjpeg, true),
        Some("h264") if h264 => (DeviceVideoCodec::H264, true),
        _ if h264 => (DeviceVideoCodec::H264, false),
        _ => (DeviceVideoCodec::Mjpeg, false),
    }
}

#[derive(Default)]
struct FpsCounter {
    window_start: Option<Instant>,
    frames: u32,
    shown: f32,
    logged_at: Option<Instant>,
}

impl FpsCounter {
    fn tick(&mut self, device_id: &str, codec: DeviceVideoCodec) {
        let now = Instant::now();
        let start = *self.window_start.get_or_insert(now);
        self.frames += 1;
        let elapsed = now.duration_since(start);
        if elapsed >= Duration::from_secs(1) {
            self.shown = self.frames as f32 / elapsed.as_secs_f32();
            self.frames = 0;
            self.window_start = Some(now);
        }
        if self
            .logged_at
            .is_none_or(|at| now.duration_since(at) >= FPS_LOG_INTERVAL)
        {
            self.logged_at = Some(now);
            tracing::info!(
                device = device_id,
                fps = self.shown,
                codec = codec_label(codec),
                "device panel decoded fps"
            );
        }
    }
}

pub fn coalesce_moves(queue: &mut VecDeque<DeviceInput>) {
    let is_move = |input: &DeviceInput| {
        matches!(
            input,
            DeviceInput::Touch {
                phase: TouchPhase::Move,
                ..
            }
        )
    };
    let mut kept: VecDeque<DeviceInput> = VecDeque::with_capacity(queue.len());
    for input in queue.drain(..) {
        if is_move(&input) && kept.back().is_some_and(is_move) {
            kept.pop_back();
        }
        kept.push_back(input);
    }
    *queue = kept;
}

async fn send_inputs(
    engine: EngineHandle,
    target: serde_json::Value,
    mut inputs: mpsc::UnboundedReceiver<DeviceInput>,
) {
    let mut queue = VecDeque::new();
    loop {
        if queue.is_empty() {
            match inputs.next().await {
                Some(input) => queue.push_back(input),
                None => return,
            }
        }
        while let Ok(input) = inputs.try_recv() {
            queue.push_back(input);
        }
        coalesce_moves(&mut queue);
        let Some(input) = queue.pop_front() else {
            continue;
        };
        let mut params = target.clone();
        params["input"] = serde_json::json!(input);
        if let Err(error) = engine
            .client()
            .call(methods::SEND_HUB_DEVICE_INPUT, params)
            .await
        {
            tracing::debug!(%error, "device input not delivered");
        }
    }
}

fn decode_jpeg(
    jpeg: &[u8],
    rotation: DeviceOrientation,
    codec: DeviceVideoCodec,
) -> Option<DecodedFrame> {
    let mut pixels =
        geometry::rotate_for_display(image::load_from_memory(jpeg).ok()?.to_rgba8(), rotation);
    for pixel in pixels.pixels_mut() {
        pixel.0.swap(0, 2);
    }
    let (width, height) = pixels.dimensions();
    (width > 0 && height > 0).then(|| DecodedFrame {
        picture: Picture::Image(Arc::new(RenderImage::new([image::Frame::new(pixels)]))),
        width,
        height,
        rotation,
        codec,
    })
}

fn jpeg_of(frame: &DeviceFrame) -> Option<Vec<u8>> {
    (!frame.jpeg.is_empty())
        .then(|| {
            base64::engine::general_purpose::STANDARD
                .decode(&frame.jpeg)
                .ok()
        })
        .flatten()
}

pub fn undecodable(reason: &str) -> String {
    format!("This Mac cannot decode the Android stream: {reason}.")
}

struct FrameSink {
    slot: Arc<Mutex<Option<DecodedFrame>>>,
    nudge: mpsc::UnboundedSender<()>,
    rotation: Arc<Mutex<DeviceOrientation>>,
    codec: Arc<Mutex<DeviceVideoCodec>>,
    avcc_failed: Arc<AtomicBool>,
    failure: Arc<Mutex<Option<String>>>,
    turns: mpsc::UnboundedReceiver<()>,
}

impl FrameSink {
    fn fail(&self, error: String) {
        *self.failure.lock().unwrap_or_else(PoisonError::into_inner) = Some(error);
        let _ = self.nudge.unbounded_send(());
    }

    fn rotation(&self) -> DeviceOrientation {
        *self.rotation.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn publish(&self, frame: DecodedFrame) -> bool {
        *self.slot.lock().unwrap_or_else(PoisonError::into_inner) = Some(frame);
        self.nudge.unbounded_send(()).is_ok()
    }
}

async fn drain_mjpeg(frames: &mut RpcSubscription, sink: &mut FrameSink) {
    use futures::future::Either;
    let mut last: Option<(Vec<u8>, DeviceOrientation)> = None;
    loop {
        let item =
            match futures::future::select(Box::pin(frames.recv()), sink.turns.select_next_some())
                .await
            {
                Either::Left((Some(item), _)) => Some(item),
                Either::Left((None, _)) => return,
                Either::Right(_) => None,
            };
        let jpeg = match item {
            Some(mut item) => {
                while let Some(Some(newer)) = futures::FutureExt::now_or_never(frames.recv()) {
                    item = newer;
                }
                let Some(jpeg) = serde_json::from_value::<DeviceFrame>(item)
                    .ok()
                    .and_then(|frame| jpeg_of(&frame))
                else {
                    continue;
                };
                jpeg
            }
            None => match &last {
                Some((jpeg, turned)) if *turned != sink.rotation() => jpeg.clone(),
                _ => continue,
            },
        };
        let rotation = sink.rotation();
        let Some(frame) = decode_jpeg(&jpeg, rotation, DeviceVideoCodec::Mjpeg) else {
            continue;
        };
        last = Some((jpeg, rotation));
        if !sink.publish(frame) {
            return;
        }
    }
}

#[cfg(target_os = "macos")]
async fn drain_h264(
    executor: &gpui::BackgroundExecutor,
    frames: &mut RpcSubscription,
    sink: &mut FrameSink,
    forced: bool,
) -> Option<super::h264::FallbackReason> {
    use super::h264::{FallbackReason, Pipeline};
    use futures::future::Either;
    use zeron_proto::device_hub::{AVCC_DELTA, AVCC_DESCRIPTION, AVCC_KEYFRAME, AvccDemuxer};

    let mut pipeline = Pipeline::default();
    let mut demuxer = AvccDemuxer::default();
    let mut failed_ticks = 0;
    loop {
        let mut items = match futures::future::select(
            Box::pin(frames.recv()),
            futures::future::select(executor.timer(LINK_TICK), sink.turns.select_next_some()),
        )
        .await
        {
            Either::Left((Some(item), _)) => {
                failed_ticks = 0;
                vec![item]
            }
            Either::Left((None, _)) => return None,
            Either::Right((Either::Right(_), _)) => Vec::new(),
            Either::Right((Either::Left(_), _)) => {
                if !sink.avcc_failed.load(Ordering::SeqCst) {
                    failed_ticks = 0;
                } else if !forced {
                    failed_ticks += 1;
                    if failed_ticks >= LINK_FAILED_TICKS {
                        return Some(FallbackReason::LinkFailed);
                    }
                }
                continue;
            }
        };
        while let Some(Some(more)) = futures::FutureExt::now_or_never(frames.recv()) {
            items.push(more);
        }
        let mut envelopes = Vec::new();
        let mut pending = 0;
        let mut jpeg = None;
        for item in items {
            let Ok(frame) = serde_json::from_value::<DeviceFrame>(item) else {
                continue;
            };
            if let Some(bytes) = jpeg_of(&frame) {
                jpeg = Some(bytes);
                continue;
            }
            let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(&frame.avcc) else {
                continue;
            };
            let Some(batch) = demuxer.push(&bytes) else {
                return Some(FallbackReason::DecodeErrors);
            };
            if batch
                .first()
                .is_none_or(|envelope| envelope.tag != AVCC_DESCRIPTION)
            {
                pending += batch
                    .iter()
                    .filter(|envelope| matches!(envelope.tag, AVCC_KEYFRAME | AVCC_DELTA))
                    .count();
            }
            envelopes.extend(batch);
        }
        if let Some(reason) = pipeline.pending(pending) {
            return Some(reason);
        }
        let rotation = sink.rotation();
        let batch = pipeline.feed(envelopes, rotation);
        if batch.fallback.is_some() {
            return batch.fallback;
        }
        let frame = match (batch.picture, batch.seed.or(jpeg)) {
            (Some(surface), _) => Some(DecodedFrame {
                width: surface.width(),
                height: surface.height(),
                picture: Picture::Surface(surface),
                rotation,
                codec: DeviceVideoCodec::H264,
            }),
            (None, Some(jpeg)) => decode_jpeg(&jpeg, rotation, DeviceVideoCodec::H264),
            (None, None) => None,
        };
        if let Some(frame) = frame
            && !sink.publish(frame)
        {
            return None;
        }
    }
}

async fn drain_frames(
    executor: gpui::BackgroundExecutor,
    engine: EngineHandle,
    target: serde_json::Value,
    device_id: String,
    mut sink: FrameSink,
    (mut codec, forced): (DeviceVideoCodec, bool),
    android: bool,
) {
    loop {
        *sink.codec.lock().unwrap_or_else(PoisonError::into_inner) = codec;
        let mut params = target.clone();
        params["codec"] = serde_json::json!(codec);
        if let Ok(mut frames) = engine
            .client()
            .subscribe_scoped(methods::WATCH_HUB_DEVICE_FRAMES, params)
            .await
        {
            match codec {
                #[cfg(target_os = "macos")]
                DeviceVideoCodec::H264 => {
                    if let Some(reason) =
                        drain_h264(&executor, &mut frames, &mut sink, forced).await
                    {
                        tracing::warn!(
                            device = %device_id,
                            reason = reason.describe(),
                            forced,
                            "device panel H.264 fallback"
                        );
                        if android && reason != super::h264::FallbackReason::Backlog {
                            return sink.fail(undecodable(reason.describe()));
                        }
                        if !forced {
                            codec = DeviceVideoCodec::Mjpeg;
                            continue;
                        }
                    }
                }
                _ if android => return sink.fail(undecodable("H.264 decoding needs macOS")),
                _ => drain_mjpeg(&mut frames, &mut sink).await,
            }
        }
        if sink.nudge.is_closed() {
            return;
        }
        executor.timer(RESUBSCRIBE_DELAY).await;
    }
}

const PINCH_IDLE: Duration = Duration::from_millis(180);

struct ThreeD {
    viewer: Rc<RefCell<Scene3d>>,
    #[cfg(target_os = "macos")]
    renderer: Rc<RefCell<super::metal3d::Renderer>>,
    wheel: scene3d::Wheel,
    wheel_end: Task<()>,
    pinch_wheel: bool,
    pinch_end: Task<()>,
    handoff: Option<(f64, Task<()>)>,
}

impl ThreeD {
    fn new() -> Option<Self> {
        #[cfg(target_os = "macos")]
        return super::metal3d::Renderer::new().map(|renderer| Self {
            viewer: Rc::default(),
            renderer: Rc::new(RefCell::new(renderer)),
            wheel: scene3d::Wheel::default(),
            wheel_end: Task::ready(()),
            pinch_wheel: false,
            pinch_end: Task::ready(()),
            handoff: None,
        });
        #[cfg(not(target_os = "macos"))]
        None
    }
}

pub struct DeviceStreamView {
    engine: EngineHandle,
    host_id: String,
    device_id: String,
    platform: DevicePlatform,
    label: (SharedString, SharedString),
    image: Option<DecodedFrame>,
    frame_number: usize,
    painted: bool,
    retired: Vec<Arc<RenderImage>>,
    #[cfg(target_os = "macos")]
    kept_surfaces: VecDeque<super::h264::Surface>,
    slot: Arc<Mutex<Option<DecodedFrame>>>,
    nudge: mpsc::UnboundedSender<()>,
    rotation: Arc<Mutex<DeviceOrientation>>,
    turned: mpsc::UnboundedSender<()>,
    active_codec: Arc<Mutex<DeviceVideoCodec>>,
    avcc_failed: Arc<AtomicBool>,
    failure: Arc<Mutex<Option<String>>>,
    screen: Option<DeviceScreenConfig>,
    link: DeviceStreamStatus,
    awaiting_frame: bool,
    video_error: Option<SharedString>,
    wait: u64,
    frame_box: Rc<Cell<Option<Bounds<Pixels>>>>,
    pressed: Option<(f32, f32)>,
    held_keys: HashSet<u16>,
    composing: Option<String>,
    focus: FocusHandle,
    focus_watch: Option<Subscription>,
    inputs: mpsc::UnboundedSender<DeviceInput>,
    fps: Option<FpsCounter>,
    codec: (DeviceVideoCodec, bool),
    debug_tap: Option<(f32, f32)>,
    rail_inset: f32,
    ax_boxes: Vec<AxBox>,
    pub(super) corner_radius: f32,
    three_d: Option<ThreeD>,
    three_d_failed: bool,
    duo_sizes: [Option<(u32, u32)>; 2],
    duo_queued: Option<f64>,
    epoch: Instant,
    drain: Task<()>,
    timer: Task<()>,
    _tasks: [Task<()>; 2],
}

pub struct StreamChanged;

impl gpui::EventEmitter<StreamChanged> for DeviceStreamView {}

pub struct DuoCommand(pub HingeCommand);

impl gpui::EventEmitter<DuoCommand> for DeviceStreamView {}

fn debug_tap_from_env() -> Option<(f32, f32)> {
    let value = std::env::var("ZERON_DEVICE_TAP").ok()?;
    let (x, y) = value.split_once(',')?;
    Some((x.trim().parse().ok()?, y.trim().parse().ok()?))
}

impl DeviceStreamView {
    pub fn new(
        engine: EngineHandle,
        host_id: String,
        device_id: String,
        platform: DevicePlatform,
        cx: &mut Context<Self>,
    ) -> Self {
        let (nudge, mut nudged) = mpsc::unbounded::<()>();
        let (inputs, queued) = mpsc::unbounded::<DeviceInput>();
        let sender = cx.background_spawn(send_inputs(
            engine.clone(),
            super::device_target(&host_id, &device_id),
            queued,
        ));
        cx.on_release(|view: &mut Self, cx| {
            let held = view.release(true);
            if !held.is_empty() {
                let (engine, target) = (
                    view.engine.clone(),
                    super::device_target(&view.host_id, &view.device_id),
                );
                cx.background_spawn(async move {
                    for input in held {
                        let mut params = target.clone();
                        params["input"] = serde_json::json!(input);
                        if let Err(error) = engine
                            .client()
                            .call(methods::SEND_HUB_DEVICE_INPUT, params)
                            .await
                        {
                            tracing::debug!(%error, "device release not delivered");
                        }
                    }
                })
                .detach();
            }
            let current = view.image.take().and_then(DecodedFrame::into_image);
            for image in current.into_iter().chain(view.retired.drain(..)) {
                gpui::ImageSource::Render(image).evict(None, cx);
            }
        })
        .detach();
        let pump = cx.spawn(async move |this, cx| {
            while nudged.next().await.is_some() {
                while nudged.try_recv().is_ok() {}
                if this.update(cx, |view, cx| view.take_frame(cx)).is_err() {
                    return;
                }
            }
        });
        let codec = codec_choice(
            platform,
            std::env::var("ZERON_DEVICE_CODEC").ok().as_deref(),
        );
        let mut view = Self {
            engine,
            label: (
                SharedString::from(device_id.clone()),
                SharedString::default(),
            ),
            host_id,
            device_id,
            platform,
            image: None,
            frame_number: 0,
            painted: false,
            retired: Vec::new(),
            #[cfg(target_os = "macos")]
            kept_surfaces: VecDeque::new(),
            slot: Arc::default(),
            nudge,
            rotation: Arc::new(Mutex::new(DeviceOrientation::Portrait)),
            turned: mpsc::unbounded().0,
            active_codec: Arc::new(Mutex::new(codec.0)),
            avcc_failed: Arc::default(),
            failure: Arc::default(),
            screen: None,
            link: DeviceStreamStatus::default(),
            awaiting_frame: true,
            video_error: None,
            wait: 0,
            frame_box: Rc::default(),
            pressed: None,
            held_keys: HashSet::new(),
            composing: None,
            focus: cx.focus_handle(),
            focus_watch: None,
            inputs,
            fps: (std::env::var("ZERON_DEVICE_FPS").as_deref() == Ok("1"))
                .then(FpsCounter::default),
            codec,
            debug_tap: debug_tap_from_env(),
            rail_inset: geometry::RAIL_WIDTH,
            ax_boxes: Vec::new(),
            corner_radius: 0.0,
            three_d: None,
            three_d_failed: false,
            duo_sizes: [None; 2],
            duo_queued: None,
            epoch: Instant::now(),
            drain: Task::ready(()),
            timer: Task::ready(()),
            _tasks: [sender, pump],
        };
        view.subscribe(cx);
        view
    }

    pub fn without_rail(mut self) -> Self {
        self.rail_inset = 0.0;
        self
    }

    pub fn with_corner_radius(mut self, radius: f32) -> Self {
        self.corner_radius = radius;
        self
    }

    fn subscribe(&mut self, cx: &mut Context<Self>) {
        let (turned, turns) = mpsc::unbounded();
        self.turned = turned;
        self.drain = cx.background_spawn(drain_frames(
            cx.background_executor().clone(),
            self.engine.clone(),
            super::device_target(&self.host_id, &self.device_id),
            self.device_id.clone(),
            FrameSink {
                slot: self.slot.clone(),
                nudge: self.nudge.clone(),
                rotation: self.rotation.clone(),
                codec: self.active_codec.clone(),
                avcc_failed: self.avcc_failed.clone(),
                failure: self.failure.clone(),
                turns,
            },
            self.codec,
            self.platform == DevicePlatform::Android,
        ));
        self.await_frame(cx);
    }

    fn await_frame(&mut self, cx: &mut Context<Self>) {
        self.awaiting_frame = true;
        self.wait += 1;
        let wait = self.wait;
        self.timer = cx.spawn(async move |this, cx| {
            cx.background_executor().timer(NO_VIDEO_TIMEOUT).await;
            this.update(cx, |view, cx| {
                if view.wait == wait && view.awaiting_frame {
                    view.video_error = Some(NO_VIDEO.into());
                    view.drain = Task::ready(());
                    cx.notify();
                }
            })
            .ok();
        });
        cx.notify();
    }

    pub fn reconnect(&mut self, cx: &mut Context<Self>) {
        self.video_error = None;
        self.subscribe(cx);
    }

    pub fn set_screen(&mut self, screen: Option<DeviceScreenConfig>, cx: &mut Context<Self>) {
        if self.screen != screen {
            let screen_id = |screen: &Option<DeviceScreenConfig>| {
                screen.as_ref().and_then(|screen| screen.screen_id)
            };
            // A fold hands HID to the other display; a press must not continue there.
            if self.screen.is_some() && screen_id(&self.screen) != screen_id(&screen) {
                self.release_held(true, cx);
            }
            let rotation = geometry::display_rotation(screen.as_ref());
            let previous = std::mem::replace(
                &mut *self.rotation.lock().unwrap_or_else(PoisonError::into_inner),
                rotation,
            );
            if previous != rotation {
                let _ = self.turned.unbounded_send(());
            }
            if let Some(screen) = screen.as_ref().filter(|screen| fold::hinged(Some(screen))) {
                let size = Some((screen.width, screen.height));
                match DuoPanel::of(screen.screen_id) {
                    DuoPanel::Cover => self.duo_sizes[0] = size,
                    DuoPanel::Inner => self.duo_sizes[1] = size,
                }
            }
            self.screen = screen;
            let now = self.now();
            if let Some(three_d) = &self.three_d {
                let [cover, inner] = self.duo_sizes();
                let mut scene = three_d.viewer.borrow_mut();
                if let Some(duo) = scene.duo() {
                    duo.set_rig(cover, inner);
                    duo.set_screen(self.screen.clone(), now);
                }
            }
            cx.notify();
        }
    }

    fn duo_sizes(&self) -> [(u32, u32); 2] {
        [
            self.duo_sizes[0].unwrap_or(duo3d::COVER_SIZE),
            self.duo_sizes[1].unwrap_or(duo3d::INNER_SIZE),
        ]
    }

    fn duo_viewer(&self, now: f64) -> DuoViewer {
        let mut duo = DuoViewer::default();
        let [cover, inner] = self.duo_sizes();
        duo.set_rig(cover, inner);
        duo.set_screen(self.screen.clone(), now);
        duo.set_control(self.duo_queued, false, now);
        duo
    }

    fn scene_shown(&self) -> bool {
        self.three_d.is_some() && !(self.duo_named() && self.screen.is_none())
    }

    fn duo_named(&self) -> bool {
        self.platform == DevicePlatform::Ios
            && self.label.0.trim().eq_ignore_ascii_case("iPhone Duo")
    }

    fn emit_duo_requests(&mut self, cx: &mut Context<Self>) {
        let Some(three_d) = &self.three_d else {
            return;
        };
        let requests = three_d.viewer.borrow_mut().take_requests();
        for request in requests {
            cx.emit(DuoCommand(request));
        }
    }

    pub fn set_duo_control(&mut self, queued: Option<f64>, failed: bool, cx: &mut Context<Self>) {
        self.duo_queued = queued;
        let now = self.now();
        if let Some(three_d) = &mut self.three_d {
            if failed {
                three_d.pinch_wheel = false;
                three_d.pinch_end = Task::ready(());
            }
            if let Some(duo) = three_d.viewer.borrow_mut().duo() {
                duo.set_control(queued, failed, now);
            }
        }
        self.emit_duo_requests(cx);
        cx.notify();
    }

    fn pinch(&mut self, event: &gpui::PinchEvent, cx: &mut Context<Self>) {
        let now = self.now();
        let Some(three_d) = &mut self.three_d else {
            return;
        };
        if self.image.is_none() || !three_d.viewer.borrow().is_duo() {
            return;
        }
        cx.stop_propagation();
        {
            let mut scene = three_d.viewer.borrow_mut();
            match event.phase {
                gpui::TouchPhase::Started => {
                    let pinching = scene.duo().is_some_and(|duo| duo.pinching());
                    if !pinching {
                        three_d.wheel_end = Task::ready(());
                        scene.end_wheel(now);
                        let (x, y) = (
                            f64::from(f32::from(event.position.x)),
                            f64::from(f32::from(event.position.y)),
                        );
                        if let Some(duo) = scene.duo() {
                            duo.pinch_begin(x, y, now);
                        }
                    }
                }
                gpui::TouchPhase::Moved => {
                    if let (Some(duo), Some(step)) =
                        (scene.duo(), duo3d::pinch_log_scale(event.delta))
                    {
                        duo.pinch_move(step, now);
                    }
                }
                gpui::TouchPhase::Ended | gpui::TouchPhase::Cancelled => {
                    if let Some(duo) = scene.duo() {
                        duo.pinch_end(now);
                    }
                }
            }
        }
        self.emit_duo_requests(cx);
        cx.notify();
    }

    fn pinch_wheel(&mut self, event: &gpui::ScrollWheelEvent, cx: &mut Context<Self>) {
        let now = self.now();
        let Some(three_d) = &mut self.three_d else {
            return;
        };
        let (dy, unit) = match event.delta {
            gpui::ScrollDelta::Pixels(delta) => (f64::from(f32::from(delta.y)), 1.0),
            gpui::ScrollDelta::Lines(delta) => (f64::from(delta.y), 16.0),
        };
        {
            let mut scene = three_d.viewer.borrow_mut();
            three_d.wheel_end = Task::ready(());
            scene.end_wheel(now);
            let Some(duo) = scene.duo() else {
                return;
            };
            if !three_d.pinch_wheel {
                three_d.pinch_wheel = true;
                duo.pinch_begin(
                    f64::from(f32::from(event.position.x)),
                    f64::from(f32::from(event.position.y)),
                    now,
                );
            }
            duo.pinch_move(-dy * unit / 100.0, now);
        }
        three_d.pinch_end = cx.spawn(async move |this, cx| {
            cx.background_executor().timer(PINCH_IDLE).await;
            this.update(cx, |view, cx| view.end_pinch_wheel(cx)).ok();
        });
        self.emit_duo_requests(cx);
        cx.notify();
    }

    fn end_pinch_wheel(&mut self, cx: &mut Context<Self>) {
        let now = self.now();
        if let Some(three_d) = &mut self.three_d
            && std::mem::take(&mut three_d.pinch_wheel)
            && let Some(duo) = three_d.viewer.borrow_mut().duo()
        {
            duo.pinch_end(now);
        }
        self.emit_duo_requests(cx);
        cx.notify();
    }

    pub fn focus_handle(&self) -> FocusHandle {
        self.focus.clone()
    }

    pub fn screen(&self) -> Option<&DeviceScreenConfig> {
        self.screen.as_ref()
    }

    pub fn frame_size(&self) -> Option<(u32, u32)> {
        self.image.as_ref().map(|frame| (frame.width, frame.height))
    }

    pub fn set_label(&mut self, name: String, description: String, cx: &mut Context<Self>) {
        let label = (SharedString::from(name), SharedString::from(description));
        if self.label != label {
            self.label = label;
            cx.notify();
        }
    }

    pub fn set_link(&mut self, link: Option<DeviceStreamStatus>, cx: &mut Context<Self>) {
        let link = link.unwrap_or_default();
        if self.link == link {
            return;
        }
        match &link.avcc {
            // "could not connect" and "the video stream ended" mean the hub is gone, and MJPEG with it.
            StreamLinkStatus::Failed { detail }
                if detail.starts_with("HTTP ") || detail == "the video stream is malformed" =>
            {
                self.avcc_failed.store(true, Ordering::SeqCst)
            }
            StreamLinkStatus::Connecting => {}
            _ => self.avcc_failed.store(false, Ordering::SeqCst),
        }
        let codec = *self
            .active_codec
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let video = |status: &DeviceStreamStatus| match codec {
            DeviceVideoCodec::Mjpeg => status.video == StreamLinkStatus::Live,
            DeviceVideoCodec::H264 => status.avcc == StreamLinkStatus::Live,
        };
        let (was_live, live) = (video(&self.link), video(&link));
        let input_was_live = self.input_live();
        self.link = link;
        if input_was_live != self.input_live() {
            cx.emit(StreamChanged);
        }
        if was_live && !live && self.video_error.is_none() {
            self.await_frame(cx);
        } else if live && self.image.is_some() {
            self.awaiting_frame = false;
        }
        cx.notify();
    }

    pub fn set_ax_boxes(&mut self, boxes: Vec<AxBox>, cx: &mut Context<Self>) {
        if self.ax_boxes != boxes {
            self.ax_boxes = boxes;
            cx.notify();
        }
    }

    #[cfg(test)]
    pub(crate) fn ax_boxes(&self) -> &[AxBox] {
        &self.ax_boxes
    }

    pub fn platform(&self) -> DevicePlatform {
        self.platform
    }

    pub fn input_live(&self) -> bool {
        self.link.input == StreamLinkStatus::Live
    }

    pub fn has_frame(&self) -> bool {
        self.image.is_some()
    }

    pub fn press_button(&mut self, button: DeviceButton) {
        self.send(DeviceInput::Button { button });
    }

    pub fn orient(&mut self, orientation: DeviceOrientation) {
        self.send(DeviceInput::Orientation { orientation });
    }

    pub fn set_three_d(&mut self, on: bool, cx: &mut Context<Self>) {
        if on == self.three_d.is_some() || (on && self.three_d_failed) {
            return;
        }
        self.release_held(true, cx);
        self.three_d = if on { ThreeD::new() } else { None };
        self.three_d_failed = on && self.three_d.is_none();
        cx.notify();
    }

    #[cfg(target_os = "macos")]
    fn retry_three_d(&mut self, cx: &mut Context<Self>) {
        if self
            .three_d
            .as_ref()
            .is_some_and(|three_d| three_d.renderer.borrow().failures >= RENDER_ATTEMPTS)
        {
            tracing::warn!(device = %self.device_id, "device 3D rendering keeps failing; showing the flat view");
            self.set_three_d(false, cx);
            self.three_d_failed = true;
        }
        cx.notify();
    }

    pub fn three_d_shown(&self) -> bool {
        self.three_d.is_some()
    }

    pub fn three_d_failed(&self) -> bool {
        self.three_d_failed
    }

    pub fn restore_three_d(&mut self, cx: &mut Context<Self>) {
        let now = self.now();
        if let Some(three_d) = &self.three_d {
            three_d.viewer.borrow_mut().restore(now);
            self.emit_duo_requests(cx);
            cx.notify();
        }
    }

    fn now(&self) -> f64 {
        self.epoch.elapsed().as_secs_f64() * 1000.0
    }

    fn shown_rotation(&self) -> DeviceOrientation {
        self.image.as_ref().map_or_else(
            || geometry::display_rotation(self.screen.as_ref()),
            |shown| shown.rotation,
        )
    }

    fn scroll(&mut self, event: &gpui::ScrollWheelEvent, cx: &mut Context<Self>) {
        let now = self.now();
        let Some(three_d) = &mut self.three_d else {
            return;
        };
        if self.image.is_none() {
            return;
        }
        cx.stop_propagation();
        if event.modifiers.control && three_d.viewer.borrow().is_duo() {
            return self.pinch_wheel(event, cx);
        }
        if std::mem::take(&mut three_d.pinch_wheel) {
            three_d.pinch_end = Task::ready(());
            if let Some(duo) = three_d.viewer.borrow_mut().duo() {
                duo.pinch_end(now);
            }
        }
        let (dx, dy, lines) = match event.delta {
            gpui::ScrollDelta::Pixels(delta) => (f32::from(delta.x), f32::from(delta.y), false),
            gpui::ScrollDelta::Lines(delta) => (delta.x, delta.y, true),
        };
        let mut viewer = three_d.viewer.borrow_mut();
        match three_d.wheel.step(event.touch_phase, lines) {
            scene3d::WheelStep::Ignore => return,
            scene3d::WheelStep::End => {
                three_d.wheel_end = Task::ready(());
                viewer.end_wheel(now);
            }
            step @ (scene3d::WheelStep::Orbit | scene3d::WheelStep::Wheel) => {
                let Some((_, _, width, height)) = viewer.viewport() else {
                    return;
                };
                if let Some((x, y)) =
                    scene3d::wheel_orbit(f64::from(dx), f64::from(dy), lines, width, height)
                    && viewer.wheel(x, y, now)
                {
                    three_d.wheel_end = match step {
                        // A mouse wheel never reports the end of its gesture.
                        scene3d::WheelStep::Wheel => cx.spawn(async move |this, cx| {
                            cx.background_executor()
                                .timer(Duration::from_millis(scene3d::WHEEL_IDLE_MS))
                                .await;
                            this.update(cx, |view, cx| {
                                let now = view.now();
                                if let Some(three_d) = &view.three_d {
                                    three_d.viewer.borrow_mut().end_wheel(now);
                                    view.emit_duo_requests(cx);
                                    cx.notify();
                                }
                            })
                            .ok();
                        }),
                        _ => Task::ready(()),
                    };
                }
            }
        }
        drop(viewer);
        self.emit_duo_requests(cx);
        cx.notify();
    }

    fn pointer_3d(
        &mut self,
        phase: TouchPhase,
        position: Point<Pixels>,
        orbit: bool,
        cx: &mut Context<Self>,
    ) {
        let now = self.now();
        let shown = self.shown_rotation();
        let Some(three_d) = &self.three_d else {
            return;
        };
        let (x, y) = (
            f64::from(f32::from(position.x)),
            f64::from(f32::from(position.y)),
        );
        let touches = {
            let mut viewer = three_d.viewer.borrow_mut();
            match phase {
                TouchPhase::Begin if self.image.is_some() && self.video_error.is_none() => {
                    viewer.press(x, y, orbit, now, shown)
                }
                TouchPhase::Begin => return,
                TouchPhase::Move => viewer.drag(x, y, now, shown),
                TouchPhase::End => {
                    let mut touches = viewer.drag(x, y, now, shown);
                    touches.extend(viewer.lift(now, shown));
                    touches
                }
            }
        };
        for (phase, (x, y)) in touches {
            self.pressed = (phase != TouchPhase::End).then_some((x, y));
            self.send(DeviceInput::Touch { phase, x, y });
        }
        self.emit_duo_requests(cx);
    }

    fn dragging(&self) -> bool {
        self.pressed.is_some()
            || self
                .three_d
                .as_ref()
                .is_some_and(|three_d| three_d.viewer.borrow().interaction().mode().is_some())
    }

    pub fn rotate(&mut self) {
        let current = self
            .screen
            .as_ref()
            .map_or(DeviceOrientation::Portrait, |screen| screen.orientation);
        self.send(DeviceInput::Orientation {
            orientation: geometry::next_orientation(current),
        });
    }

    fn send(&self, input: DeviceInput) {
        let _ = self.inputs.unbounded_send(input);
    }

    fn release(&mut self, touch: bool) -> Vec<DeviceInput> {
        let mut inputs: Vec<DeviceInput> = self.held_keys.drain().map(geometry::key_up).collect();
        if touch && let Some(three_d) = &self.three_d {
            three_d
                .viewer
                .borrow_mut()
                .lift(self.now(), self.shown_rotation());
        }
        if touch && let Some((x, y)) = self.pressed.take() {
            inputs.push(DeviceInput::Touch {
                phase: TouchPhase::End,
                x,
                y,
            });
        }
        inputs
    }

    pub(super) fn release_held(&mut self, touch: bool, cx: &mut Context<Self>) {
        for input in self.release(touch) {
            self.send(input);
        }
        self.emit_duo_requests(cx);
        cx.notify();
    }

    fn take_frame(&mut self, cx: &mut Context<Self>) {
        if let Some(error) = self
            .failure
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take()
        {
            self.video_error = Some(error.into());
            self.awaiting_frame = false;
            cx.notify();
            return;
        }
        let Some(frame) = self
            .slot
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take()
        else {
            return;
        };
        self.awaiting_frame = false;
        let codec = frame.codec;
        self.frame_number += 1;
        if self.frame_size() != Some((frame.width, frame.height)) {
            cx.emit(StreamChanged);
        }
        if let Some(old) = self.image.replace(frame) {
            let painted = std::mem::take(&mut self.painted);
            match old.picture {
                Picture::Image(image) if painted => self.retired.push(image),
                Picture::Image(_) => {}
                // gpui drops its CVMetalTextures before the GPU is done, so keep the buffers out of the pool.
                #[cfg(target_os = "macos")]
                Picture::Surface(surface) => {
                    if self.kept_surfaces.len() == KEPT_SURFACES {
                        self.kept_surfaces.pop_front();
                    }
                    self.kept_surfaces.push_back(surface);
                }
            }
        }
        if let Some(fps) = &mut self.fps {
            fps.tick(&self.device_id, codec);
        }
        if let Some(frame) = self.frame_box.get()
            && let Some((x, y)) = self.debug_tap
            && let Some(position) = match &self.three_d {
                Some(three_d) => three_d
                    .viewer
                    .borrow()
                    .tap_position((x, y))
                    .map(|(x, y)| gpui::point(px(x as f32), px(y as f32))),
                None => Some(geometry::position_at(frame, x, y)),
            }
        {
            self.debug_tap = None;
            tracing::info!(device = %self.device_id, x, y, "ZERON_DEVICE_TAP: synthetic tap");
            self.pointer(TouchPhase::Begin, position, cx);
            self.pointer(TouchPhase::End, position, cx);
        }
        cx.notify();
    }

    fn aspect(&self) -> f32 {
        if let Some(frame) = &self.image
            && frame.width > 0
            && frame.height > 0
        {
            return frame.width as f32 / frame.height as f32;
        }
        match &self.screen {
            Some(screen) if screen.width > 0 && screen.height > 0 => {
                screen.width as f32 / screen.height as f32
            }
            _ if self.platform == DevicePlatform::Android => geometry::ANDROID_PLACEHOLDER_ASPECT,
            _ => geometry::IOS_PLACEHOLDER_ASPECT,
        }
    }

    fn pointer(&mut self, phase: TouchPhase, position: Point<Pixels>, cx: &mut Context<Self>) {
        if self.scene_shown() {
            return self.pointer_3d(phase, position, false, cx);
        }
        let Some(frame) = self.frame_box.get() else {
            return;
        };
        match phase {
            TouchPhase::Begin
                if self.image.is_some()
                    && self.video_error.is_none()
                    && frame.contains(&position) => {}
            TouchPhase::Move | TouchPhase::End if self.pressed.is_some() => {}
            _ => return,
        }
        let rotation = self.image.as_ref().map_or_else(
            || geometry::display_rotation(self.screen.as_ref()),
            |shown| shown.rotation,
        );
        let (x, y) = geometry::normalized(frame, position);
        let (x, y) = geometry::raw_point(rotation, x, y);
        self.pressed = (phase != TouchPhase::End).then_some((x, y));
        self.send(DeviceInput::Touch { phase, x, y });
    }

    fn key_down(&mut self, event: &KeyDownEvent, cx: &mut Context<Self>) {
        if event.keystroke.modifiers.platform {
            return;
        }
        if self.platform == DevicePlatform::Android {
            if let Some(input) = geometry::android_input(&event.keystroke) {
                cx.stop_propagation();
                self.send(input);
            }
            return;
        }
        let Some(press) = KeyPress::from_keystroke(&event.keystroke) else {
            return;
        };
        cx.stop_propagation();
        if event.is_held || !self.held_keys.insert(press.usage) {
            return;
        }
        for input in press.down() {
            self.send(input);
        }
    }

    fn key_up(&mut self, event: &KeyUpEvent, cx: &mut Context<Self>) {
        let Some((usage, _)) = geometry::hid_usage(&event.keystroke.key) else {
            return;
        };
        if self.held_keys.remove(&usage) {
            cx.stop_propagation();
            self.send(geometry::key_up(usage));
        }
    }

    fn modifiers_changed(&mut self, event: &ModifiersChangedEvent, cx: &mut Context<Self>) {
        // macOS sends no key up for a key released while Command is held.
        if event.modifiers.platform && !self.held_keys.is_empty() {
            self.release_held(false, cx);
        }
    }

    #[cfg(test)]
    pub(crate) fn frame_box(&self) -> Option<Bounds<Pixels>> {
        self.frame_box.get()
    }
}

impl gpui::EntityInputHandler for DeviceStreamView {
    fn text_for_range(
        &mut self,
        _range: std::ops::Range<usize>,
        _adjusted: &mut Option<std::ops::Range<usize>>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<String> {
        None
    }

    fn selected_text_range(
        &mut self,
        _ignore_disabled_input: bool,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<gpui::UTF16Selection> {
        let end = self
            .composing
            .as_ref()
            .map_or(0, |text| text.encode_utf16().count());
        Some(gpui::UTF16Selection {
            range: end..end,
            reversed: false,
        })
    }

    fn marked_text_range(
        &self,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<std::ops::Range<usize>> {
        self.composing
            .as_ref()
            .map(|text| 0..text.encode_utf16().count())
    }

    fn unmark_text(&mut self, _window: &mut Window, _cx: &mut Context<Self>) {
        self.composing = None;
    }

    fn replace_text_in_range(
        &mut self,
        _range: Option<std::ops::Range<usize>>,
        text: &str,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) {
        self.composing = None;
        if !text.is_empty() {
            self.send(DeviceInput::Text { text: text.into() });
        }
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        _range: Option<std::ops::Range<usize>>,
        text: &str,
        _selected: Option<std::ops::Range<usize>>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) {
        self.composing = (!text.is_empty()).then(|| text.to_string());
    }

    fn bounds_for_range(
        &mut self,
        _range: std::ops::Range<usize>,
        element_bounds: Bounds<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        Some(element_bounds)
    }

    fn character_index_for_point(
        &mut self,
        _point: Point<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<usize> {
        None
    }
}

#[cfg(target_os = "macos")]
#[allow(clippy::too_many_arguments)]
fn paint_3d(
    scene: &RefCell<Scene3d>,
    renderer: &RefCell<super::metal3d::Renderer>,
    frame: Bounds<Pixels>,
    picture: Option<&Picture>,
    live: Option<(DeviceOrientation, (u32, u32))>,
    frame_number: usize,
    clear: [f32; 3],
    window: &mut Window,
) -> bool {
    use super::metal3d::{Draw, Job, Source};
    let mut scene = scene.borrow_mut();
    let viewport = (
        f64::from(f32::from(frame.origin.x)),
        f64::from(f32::from(frame.origin.y)),
        f64::from(f32::from(frame.size.width)),
        f64::from(f32::from(frame.size.height)),
    );
    if viewport.2 < 1.0 || viewport.3 < 1.0 {
        return false;
    }
    let source = match picture {
        Some(Picture::Surface(surface)) => Source::Surface(surface),
        Some(Picture::Image(image)) => {
            let size = image.size(0);
            Source::Bgra {
                bytes: image.as_bytes(0).unwrap_or_default(),
                width: size.width.0.max(0) as usize,
                height: size.height.0.max(0) as usize,
            }
        }
        None => return false,
    };
    let scale = f64::from(window.scale_factor());
    let size = (
        (viewport.2 * scale).round() as usize,
        (viewport.3 * scale).round() as usize,
    );
    let surface = match &mut *scene {
        Scene3d::Phone(viewer) => {
            let (Some(camera), Some(rotation), Some(model)) =
                (viewer.fit(viewport), viewer.rotation(), viewer.model())
            else {
                return false;
            };
            let (mvp, rotation) = camera.matrices(rotation);
            let draws = [Draw {
                range: 0..model.vertices.len(),
                mvp,
                rotation,
            }];
            renderer.borrow_mut().render(Job {
                source,
                frame: frame_number,
                mesh: (model.id, &model.vertices),
                draws: &draws,
                live_panel: 1,
                live_rotation: 0,
                size,
                clear,
            })
        }
        Scene3d::Duo(duo) => {
            duo.fit(viewport);
            let (Some(rig), Some(matrices), Some((rotation, frame_size))) =
                (duo.rig(), duo.matrices(), live)
            else {
                return false;
            };
            let raw = duo3d::duo_raw_size(frame_size, rotation);
            let active = duo.active_panel();
            let other = match active {
                DuoPanel::Cover => DuoPanel::Inner,
                DuoPanel::Inner => DuoPanel::Cover,
            };
            let live_panel = [active, other].into_iter().find(|&panel| {
                let index = usize::from(panel == DuoPanel::Inner);
                duo3d::duo_frame_matches(raw, rig.sizes[index])
            });
            let draws = matrices.map(|(range, mvp, rotation)| Draw {
                range,
                mvp,
                rotation,
            });
            let surface = renderer.borrow_mut().render(Job {
                source,
                frame: frame_number,
                mesh: (rig.id, &rig.vertices),
                draws: &draws,
                live_panel: live_panel.map_or(0, |panel| panel as u8),
                live_rotation: duo3d::quarter_turns(rotation),
                size,
                clear,
            });
            if surface.is_some() && live_panel == Some(active) {
                let key = duo3d::duo_display_key(duo.screen());
                duo.frame_ready(key);
            }
            surface
        }
    };
    let Some(surface) = surface else {
        return false;
    };
    window.paint_surface(frame, surface.to_gpui());
    true
}

impl Render for DeviceStreamView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        for old in self.retired.drain(..) {
            let _ = window.drop_image(old);
        }
        if self.focus_watch.is_none() {
            // gpui also reports window deactivation as a blur of the focused handle.
            self.focus_watch = Some(cx.on_blur(&self.focus, window, |view, _, cx| {
                view.release_held(true, cx)
            }));
        }
        self.painted = self.image.is_some();
        let theme = Theme::of(cx).clone();
        let picture = self.image.as_ref().map(|frame| frame.picture.clone());
        let aspect = self.aspect();
        let frame_box = self.frame_box.clone();
        let fps = self.fps.as_ref().map(|fps| {
            let codec = self
                .image
                .as_ref()
                .map_or(self.codec.0, |frame| frame.codec);
            format!("{:.0} fps · {}", fps.shown, codec_label(codec))
        });
        let rail_inset = self.rail_inset;
        let ime =
            (self.platform == DevicePlatform::Android).then(|| (self.focus.clone(), cx.entity()));
        let corners = gpui::Corners::all(px(self.corner_radius));
        let rotation = self.image.as_ref().map_or_else(
            || geometry::display_rotation(self.screen.as_ref()),
            |shown| shown.rotation,
        );
        let ax_boxes: Vec<AxBox> = self
            .ax_boxes
            .iter()
            .map(|ax| ax.displayed(rotation))
            .collect();
        let ax_color = theme.accent;
        let dragging = self.dragging().then(|| cx.entity().downgrade());
        let now = self.now();
        // Without its first screen config a Duo would flash the phone body before the rig.
        let scene = self
            .three_d
            .as_ref()
            .filter(|_| self.image.is_some() && self.scene_shown())
            .map(|three_d| {
                let mut viewer = three_d.viewer.borrow_mut();
                if fold::hinged(self.screen.as_ref()) {
                    if !viewer.is_duo() {
                        *viewer = Scene3d::Duo(self.duo_viewer(now));
                    }
                    let [cover, inner] = self.duo_sizes();
                    if let Some(duo) = viewer.duo() {
                        duo.set_rig(cover, inner);
                    }
                } else {
                    if viewer.is_duo() {
                        *viewer = Scene3d::default();
                    }
                    if let Scene3d::Phone(viewer) = &mut *viewer {
                        let orientation = scene3d::body_orientation(rotation, self.screen.as_ref());
                        let aspect = f64::from(aspect);
                        let profile = scene3d::resolve_shape(
                            self.platform,
                            &self.label.0,
                            aspect.min(1.0 / aspect),
                        );
                        viewer.set_model(profile, aspect, orientation);
                    }
                }
                viewer.advance(now, cx.reduce_motion());
                if viewer.needs_frame() {
                    window.request_animation_frame();
                }
                #[cfg(target_os = "macos")]
                let scene = (three_d.viewer.clone(), three_d.renderer.clone());
                #[cfg(not(target_os = "macos"))]
                let scene = three_d.viewer.clone();
                scene
            });
        let deadline = self
            .three_d
            .as_ref()
            .and_then(|three_d| three_d.viewer.borrow().handoff_deadline());
        if let Some(three_d) = &mut self.three_d
            && three_d.handoff.as_ref().map(|(armed, _)| *armed) != deadline
        {
            three_d.handoff = deadline.map(|deadline| {
                let wait = Duration::from_millis((deadline - now).max(0.0).ceil() as u64 + 1);
                let timer = cx.spawn(async move |this, cx| {
                    cx.background_executor().timer(wait).await;
                    this.update(cx, |_, cx| cx.notify()).ok();
                });
                (deadline, timer)
            });
        }
        #[cfg(target_os = "macos")]
        let live = self
            .image
            .as_ref()
            .map(|frame| (frame.rotation, (frame.width, frame.height)));
        #[cfg(target_os = "macos")]
        let clear = gpui::Rgba::from(theme.bg);
        #[cfg(target_os = "macos")]
        let clear = [clear.r, clear.g, clear.b];
        #[cfg(target_os = "macos")]
        let (frame_number, this) = (self.frame_number, cx.entity().downgrade());
        let connecting = self.awaiting_frame && self.video_error.is_none();
        let input_down = (!self.awaiting_frame && self.video_error.is_none() && !self.input_live())
            .then(|| match &self.link.input {
                StreamLinkStatus::Failed { detail } if !detail.is_empty() => {
                    format!("Input disconnected ({detail}), reconnecting...")
                }
                _ => "Input disconnected, reconnecting...".to_string(),
            });
        div()
            .id("device-stream")
            .size_full()
            .relative()
            .key_context("DeviceScreen")
            .track_focus(&self.focus)
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, _, cx| this.key_down(event, cx)))
            .on_key_up(cx.listener(|this, event: &KeyUpEvent, _, cx| this.key_up(event, cx)))
            .on_modifiers_changed(cx.listener(|this, event: &ModifiersChangedEvent, _, cx| {
                this.modifiers_changed(event, cx)
            }))
            .on_action(cx.listener(|this, _: &SendCommandR, _, _| {
                if this.platform == DevicePlatform::Android {
                    return;
                }
                for input in geometry::command_r() {
                    this.send(input);
                }
            }))
            .child(
                gpui::canvas(
                    |_, _, _| (),
                    move |bounds, _, window, cx| {
                        let frame = match &scene {
                            Some(_) => Bounds::new(
                                bounds.origin,
                                gpui::size(bounds.size.width - px(rail_inset), bounds.size.height),
                            ),
                            None => geometry::centered_frame(bounds, aspect, rail_inset),
                        };
                        frame_box.set(Some(frame));
                        if let Some((focus, view)) = ime {
                            window.handle_input(
                                &focus,
                                gpui::ElementInputHandler::new(frame, view),
                                cx,
                            );
                        }
                        #[cfg(target_os = "macos")]
                        let shown_3d = scene.as_ref().is_some_and(|(viewer, renderer)| {
                            let shown = paint_3d(
                                viewer,
                                renderer,
                                frame,
                                picture.as_ref(),
                                live,
                                frame_number,
                                clear,
                                window,
                            );
                            if renderer.borrow().failures > 0 {
                                window.on_next_frame(move |_, cx| {
                                    this.update(cx, |view, cx| view.retry_three_d(cx)).ok();
                                });
                            }
                            shown
                        });
                        #[cfg(not(target_os = "macos"))]
                        let shown_3d = false;
                        let flat = match &scene {
                            Some(_) => geometry::centered_frame(bounds, aspect, rail_inset),
                            None => frame,
                        };
                        match picture.filter(|_| !shown_3d) {
                            Some(Picture::Image(image)) => {
                                let _ = window.paint_image(flat, corners, image, 0, false);
                            }
                            #[cfg(target_os = "macos")]
                            Some(Picture::Surface(surface)) => {
                                window.paint_surface(flat, surface.to_gpui())
                            }
                            None if scene.is_some() => {}
                            None => window.paint_quad(
                                gpui::fill(frame, crate::theme::ink(0.04)).corner_radii(corners),
                            ),
                        }
                        for ax in ax_boxes.iter().filter(|_| scene.is_none()) {
                            let at = |x: f32, y: f32| {
                                gpui::point(
                                    frame.origin.x + frame.size.width * x,
                                    frame.origin.y + frame.size.height * y,
                                )
                            };
                            window.paint_quad(gpui::quad(
                                Bounds::from_corners(
                                    at(ax.x, ax.y),
                                    at(ax.x + ax.width, ax.y + ax.height),
                                ),
                                px(2.0),
                                ax_color.opacity(0.1),
                                px(1.0),
                                ax_color.opacity(0.8),
                                gpui::BorderStyle::Solid,
                            ));
                        }
                        if let Some(view) = dragging {
                            window.on_mouse_event(move |event: &MouseMoveEvent, phase, _, cx| {
                                if phase == gpui::DispatchPhase::Bubble {
                                    view.update(cx, |view, cx| {
                                        view.pointer(TouchPhase::Move, event.position, cx);
                                        if view.three_d.is_some() {
                                            cx.notify();
                                        }
                                    })
                                    .ok();
                                }
                            });
                        }
                    },
                )
                .absolute()
                .inset_0(),
            )
            .when(connecting || self.video_error.is_some(), |el| {
                let error = self.video_error.clone();
                let reconnect = error.is_some().then(|| {
                    div()
                        .id("device-reconnect")
                        .debug_selector(|| "device-reconnect".into())
                        .role(gpui::Role::Button)
                        .aria_label("Reconnect")
                        .px(px(10.0))
                        .py(px(4.0))
                        .rounded(px(8.0))
                        .border_1()
                        .border_color(theme.border)
                        .text_size(crate::typography::ui_rems(12.0))
                        .text_color(theme.text)
                        .cursor_pointer()
                        .hover(|s| s.bg(crate::theme::ink(0.05)))
                        .child("Reconnect")
                        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .on_click(cx.listener(|this, _, _, cx| this.reconnect(cx)))
                        .into_any_element()
                });
                el.child(
                    div()
                        .absolute()
                        .inset_0()
                        .right(px(rail_inset))
                        .child(loading::render(
                            LoadingView {
                                name: self.label.0.clone(),
                                description: self.label.1.clone(),
                                message: error
                                    .clone()
                                    .unwrap_or_else(|| "Connecting video...".into()),
                                stage: LoadingStage::Stream,
                                error: error.is_some(),
                            },
                            &theme,
                            cx.entity_id(),
                            reconnect,
                            cx,
                        )),
                )
            })
            .when_some(input_down, |el, text| {
                el.child(
                    div()
                        .absolute()
                        .left_0()
                        .right(px(rail_inset))
                        .bottom_0()
                        .p(px(8.0))
                        .flex()
                        .justify_center()
                        .child(
                            div()
                                .rounded(px(6.0))
                                .bg(theme.bg.opacity(0.85))
                                .px(px(8.0))
                                .py(px(4.0))
                                .text_size(crate::typography::ui_rems(12.0))
                                .text_color(theme.text_muted)
                                .child(SharedString::from(text)),
                        ),
                )
            })
            .when_some(fps, |el, fps| {
                el.child(
                    div()
                        .absolute()
                        .bottom(px(6.0))
                        .right(px(8.0 + rail_inset))
                        .text_size(crate::typography::ui_rems(11.0))
                        .text_color(theme.text_muted)
                        .child(SharedString::from(fps)),
                )
            })
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, event: &gpui::MouseDownEvent, window, cx| {
                    window.focus(&this.focus, cx);
                    if this.scene_shown() {
                        this.pointer_3d(TouchPhase::Begin, event.position, event.modifiers.alt, cx);
                    } else {
                        this.pointer(TouchPhase::Begin, event.position, cx);
                    }
                    cx.stop_propagation();
                    cx.notify();
                }),
            )
            .on_scroll_wheel(
                cx.listener(|this, event: &gpui::ScrollWheelEvent, _, cx| this.scroll(event, cx)),
            )
            .on_pinch(cx.listener(|this, event: &gpui::PinchEvent, _, cx| this.pinch(event, cx)))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, event: &gpui::MouseUpEvent, _, cx| {
                    this.pointer(TouchPhase::End, event.position, cx);
                    cx.notify();
                }),
            )
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(|this, event: &gpui::MouseUpEvent, _, cx| {
                    this.pointer(TouchPhase::End, event.position, cx);
                    cx.notify();
                }),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{Entity, Keystroke, Modifiers, TestAppContext, VisualTestContext, point};
    use serde_json::{Value, json};
    use std::sync::atomic::{AtomicUsize, Ordering};

    type Inputs = Arc<Mutex<Vec<Value>>>;

    fn jpeg() -> String {
        jpeg_sized(18, 39)
    }

    fn jpeg_sized(width: u32, height: u32) -> String {
        let image = image::RgbImage::from_pixel(width, height, image::Rgb([40, 90, 200]));
        let mut bytes = Vec::new();
        image::codecs::jpeg::JpegEncoder::new(&mut bytes)
            .encode_image(&image)
            .unwrap();
        base64::engine::general_purpose::STANDARD.encode(bytes)
    }

    fn decode(item: Value, rotation: DeviceOrientation) -> Option<DecodedFrame> {
        let frame: DeviceFrame = serde_json::from_value(item).ok()?;
        decode_jpeg(&jpeg_of(&frame)?, rotation, DeviceVideoCodec::Mjpeg)
    }

    fn fake_engine(runtime: &tokio::runtime::Runtime) -> (EngineHandle, Inputs) {
        let (engine, inputs, _) = fake_engine_with(runtime, true);
        (engine, inputs)
    }

    fn fake_engine_with(
        runtime: &tokio::runtime::Runtime,
        frames_flow: bool,
    ) -> (EngineHandle, Inputs, Arc<AtomicUsize>) {
        let (engine, inputs, subscriptions, _) = hinge_engine(runtime, frames_flow, |_| None);
        (engine, inputs, subscriptions)
    }

    fn hinge_engine(
        runtime: &tokio::runtime::Runtime,
        frames_flow: bool,
        refuse: impl Fn(usize) -> Option<&'static str> + Send + 'static,
    ) -> (EngineHandle, Inputs, Arc<AtomicUsize>, Inputs) {
        let (out, mut frames) = tokio::sync::mpsc::channel::<String>(64);
        let (replies, inbound) = tokio::sync::mpsc::channel::<String>(64);
        let inputs: Inputs = Arc::default();
        let seen = inputs.clone();
        let hinges: Inputs = Arc::default();
        let hinged = hinges.clone();
        let subscriptions = Arc::new(AtomicUsize::new(0));
        let subscribed = subscriptions.clone();
        runtime.spawn(async move {
            while let Some(frame) = frames.recv().await {
                let frame: Value = serde_json::from_str(&frame).unwrap();
                let reply = match frame["method"].as_str() {
                    Some(methods::WATCH_HUB_DEVICE_FRAMES) => {
                        subscribed.fetch_add(1, Ordering::SeqCst);
                        if !frames_flow {
                            continue;
                        }
                        json!({ "id": frame["id"], "item": { "seq": 1, "jpeg": jpeg() } })
                    }
                    Some(methods::SEND_HUB_DEVICE_INPUT) => {
                        seen.lock().unwrap().push(frame["params"].clone());
                        json!({ "id": frame["id"], "ok": {} })
                    }
                    Some(methods::SET_HUB_DEVICE_HINGE) => {
                        let count = {
                            let mut hinged = hinged.lock().unwrap();
                            hinged.push(frame["params"].clone());
                            hinged.len()
                        };
                        match refuse(count - 1) {
                            Some(error) => json!({ "id": frame["id"], "err": error }),
                            None => json!({ "id": frame["id"], "ok": {} }),
                        }
                    }
                    _ => continue,
                };
                if replies.send(reply.to_string()).await.is_err() {
                    break;
                }
            }
            std::future::pending::<()>().await;
        });
        let client = zeron_rpc::RpcClient::new(out, inbound);
        (
            EngineHandle::from_test_client(client),
            inputs,
            subscriptions,
            hinges,
        )
    }

    fn wait(cx: &mut VisualTestContext, mut done: impl FnMut(&mut VisualTestContext) -> bool) {
        for _ in 0..400 {
            cx.run_until_parked();
            if done(cx) {
                return;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        panic!("condition never held");
    }

    fn touches(inputs: &Inputs) -> Vec<(String, f64, f64)> {
        inputs
            .lock()
            .unwrap()
            .iter()
            .map(|params| {
                assert_eq!(params["deviceId"], "SIM");
                let touch = &params["input"]["touch"];
                (
                    touch["phase"].as_str().unwrap().to_string(),
                    touch["x"].as_f64().unwrap(),
                    touch["y"].as_f64().unwrap(),
                )
            })
            .collect()
    }

    fn assert_touches(actual: &[(String, f64, f64)], expected: &[(&str, f64, f64)]) {
        assert_eq!(actual.len(), expected.len(), "{actual:?}");
        for ((phase, x, y), (want, wx, wy)) in actual.iter().zip(expected) {
            assert_eq!(phase, want);
            assert!((x - wx).abs() < 0.01 && (y - wy).abs() < 0.01, "{actual:?}");
        }
    }

    #[test]
    fn moves_behind_an_in_flight_send_collapse_to_the_newest_and_ends_survive() {
        let touch = |phase, x| DeviceInput::Touch { phase, x, y: 0.5 };
        let mut queue: VecDeque<DeviceInput> = [
            touch(TouchPhase::Move, 0.1),
            touch(TouchPhase::Move, 0.2),
            touch(TouchPhase::End, 0.3),
            touch(TouchPhase::Begin, 0.4),
            touch(TouchPhase::Move, 0.5),
            touch(TouchPhase::Move, 0.6),
        ]
        .into();
        coalesce_moves(&mut queue);
        assert_eq!(
            Vec::from(queue),
            vec![
                touch(TouchPhase::Move, 0.2),
                touch(TouchPhase::End, 0.3),
                touch(TouchPhase::Begin, 0.4),
                touch(TouchPhase::Move, 0.6),
            ]
        );
    }

    #[gpui::test]
    fn frames_that_were_never_painted_are_released_instead_of_retired(cx: &mut TestAppContext) {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let _guard = runtime.enter();
        cx.update(|cx| {
            gpui_base::init(cx);
            cx.set_global(Theme::default());
        });
        let (engine, _) = fake_engine(&runtime);
        let view = cx.new(|cx| {
            DeviceStreamView::new(
                engine,
                "local".into(),
                "SIM".into(),
                DevicePlatform::Ios,
                cx,
            )
        });
        view.update(cx, |view, cx| {
            for seq in 0..20 {
                let item = json!({ "seq": seq, "jpeg": jpeg() });
                *view.slot.lock().unwrap() = decode(item, DeviceOrientation::Portrait);
                view.take_frame(cx);
            }
            assert!(view.has_frame());
            assert!(view.retired.is_empty(), "{} retired", view.retired.len());
            view.painted = true;
            for seq in 20..40 {
                let item = json!({ "seq": seq, "jpeg": jpeg() });
                *view.slot.lock().unwrap() = decode(item, DeviceOrientation::Portrait);
                view.take_frame(cx);
            }
            assert_eq!(
                view.retired.len(),
                1,
                "only the painted image waits for the atlas"
            );
        });
    }

    #[gpui::test]
    fn a_pointer_drag_on_the_frame_sends_ordered_touches_in_device_space(cx: &mut TestAppContext) {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let _guard = runtime.enter();
        cx.update(|cx| {
            gpui_base::init(cx);
            cx.set_global(Theme::default());
        });
        cx.executor().allow_parking();
        let (engine, inputs) = fake_engine(&runtime);
        let (view, cx) = cx.add_window_view(|_, cx| {
            DeviceStreamView::new(
                engine,
                "local".into(),
                "SIM".into(),
                DevicePlatform::Ios,
                cx,
            )
        });
        wait(cx, |cx| {
            view.read_with(cx, |view, _| view.has_frame() && view.frame_box().is_some())
        });
        let frame = view.read_with(cx, |view, _| view.frame_box().unwrap());
        let at = |x, y| geometry::position_at(frame, x, y);
        let none = Modifiers::default();

        cx.simulate_mouse_move(at(0.1, 0.1), None, none);
        cx.simulate_mouse_down(at(0.25, 0.5), MouseButton::Left, none);
        cx.simulate_mouse_move(at(0.5, 0.5), MouseButton::Left, none);
        cx.simulate_mouse_up(at(0.75, 0.6), MouseButton::Left, none);
        wait(cx, |_| inputs.lock().unwrap().len() >= 3);
        assert_touches(
            &touches(&inputs),
            &[("begin", 0.25, 0.5), ("move", 0.5, 0.5), ("end", 0.75, 0.6)],
        );

        view.update(cx, |view, cx| {
            view.set_screen(
                Some(DeviceScreenConfig {
                    width: 588,
                    height: 1280,
                    orientation: DeviceOrientation::LandscapeLeft,
                    ..Default::default()
                }),
                cx,
            )
        });
        cx.run_until_parked();
        let tap = |cx: &mut VisualTestContext| {
            inputs.lock().unwrap().clear();
            let frame = view.read_with(cx, |view, _| view.frame_box().unwrap());
            let at = geometry::position_at(frame, 0.25, 0.75);
            cx.simulate_mouse_down(at, MouseButton::Left, none);
            cx.simulate_mouse_up(at, MouseButton::Left, none);
            wait(cx, |_| inputs.lock().unwrap().len() >= 2);
            touches(&inputs)
        };
        assert_touches(&tap(cx), &[("begin", 0.25, 0.75), ("end", 0.25, 0.75)]);
        view.update(cx, |view, cx| {
            let item = json!({ "seq": 2, "jpeg": jpeg() });
            *view.slot.lock().unwrap() = decode(item, DeviceOrientation::LandscapeLeft);
            view.take_frame(cx);
        });
        cx.run_until_parked();
        assert_touches(&tap(cx), &[("begin", 0.75, 0.75), ("end", 0.75, 0.75)]);
    }

    #[cfg(target_os = "macos")]
    #[gpui::test]
    fn the_3d_view_touches_through_its_screen_orbits_off_it_and_restores_square(
        cx: &mut TestAppContext,
    ) {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let _guard = runtime.enter();
        cx.update(|cx| {
            gpui_base::init(cx);
            cx.set_global(Theme::default());
        });
        cx.executor().allow_parking();
        let (engine, inputs) = fake_engine(&runtime);
        let (view, cx) = cx.add_window_view(|_, cx| {
            DeviceStreamView::new(
                engine,
                "local".into(),
                "SIM".into(),
                DevicePlatform::Ios,
                cx,
            )
        });
        view.update(cx, |view, cx| view.set_three_d(true, cx));
        let viewer = |cx: &mut VisualTestContext| {
            view.read_with(cx, |view, _| view.three_d.as_ref().unwrap().viewer.clone())
        };
        wait(cx, |cx| {
            view.read_with(cx, |view, _| view.has_frame())
                && viewer(cx).borrow().viewport().is_some()
        });
        let at = |cx: &mut VisualTestContext, x: f32, y: f32| {
            let (x, y) = viewer(cx).borrow().tap_position((x, y)).unwrap();
            point(px(x as f32), px(y as f32))
        };
        let none = Modifiers::default();
        let (start, middle, finish) = (at(cx, 0.25, 0.5), at(cx, 0.5, 0.5), at(cx, 0.75, 0.6));
        cx.simulate_mouse_down(start, MouseButton::Left, none);
        cx.simulate_mouse_move(middle, MouseButton::Left, none);
        cx.simulate_mouse_up(finish, MouseButton::Left, none);
        wait(cx, |_| inputs.lock().unwrap().len() >= 4);
        assert_touches(
            &touches(&inputs),
            &[
                ("begin", 0.25, 0.5),
                ("move", 0.5, 0.5),
                ("move", 0.75, 0.6),
                ("end", 0.75, 0.6),
            ],
        );

        inputs.lock().unwrap().clear();
        let area = view.read_with(cx, |view, _| view.frame_box().unwrap());
        let corner = area.origin + point(px(4.0), px(4.0));
        cx.simulate_mouse_down(corner, MouseButton::Left, none);
        cx.simulate_mouse_move(corner + point(px(160.0), px(40.0)), MouseButton::Left, none);
        cx.simulate_mouse_up(corner + point(px(160.0), px(40.0)), MouseButton::Left, none);
        cx.run_until_parked();
        assert!(
            inputs.lock().unwrap().is_empty(),
            "a press off the screen orbits"
        );
        let later = view.read_with(cx, |view, _| view.now()) + 300.0;
        viewer(cx).borrow_mut().advance(later, false);
        let turned = viewer(cx).borrow().motion().rotation;
        assert!(turned.angle_to(scene3d::Quat::IDENTITY) > 0.01);

        cx.simulate_event(gpui::ScrollWheelEvent {
            position: area.center(),
            delta: gpui::ScrollDelta::Pixels(point(px(120.0), px(0.0))),
            modifiers: none,
            touch_phase: gpui::TouchPhase::Moved,
        });
        assert!(viewer(cx).borrow().interaction().mode().is_none());
        view.update(cx, |view, cx| view.restore_three_d(cx));
        let started = std::time::Instant::now();
        while viewer(cx)
            .borrow()
            .motion()
            .rotation
            .angle_to(scene3d::Quat::IDENTITY)
            > 1e-6
        {
            assert!(
                started.elapsed() < Duration::from_secs(5),
                "restore never settled"
            );
            std::thread::sleep(Duration::from_millis(16));
            view.update(cx, |view, cx| {
                let now = view.now();
                view.three_d
                    .as_ref()
                    .unwrap()
                    .viewer
                    .borrow_mut()
                    .advance(now, false);
                cx.notify();
            });
            cx.run_until_parked();
        }

        view.update(cx, |view, cx| view.set_three_d(false, cx));
        cx.run_until_parked();
        let frame = view.read_with(cx, |view, _| view.frame_box().unwrap());
        assert!(
            frame.size.width < area.size.width,
            "flat again: the frame hugs the stream"
        );
    }

    #[cfg(target_os = "macos")]
    fn three_d_view<'a>(
        cx: &'a mut TestAppContext,
        runtime: &tokio::runtime::Runtime,
        tap: Option<(f32, f32)>,
    ) -> (Entity<DeviceStreamView>, Inputs, &'a mut VisualTestContext) {
        cx.update(|cx| {
            gpui_base::init(cx);
            cx.set_global(Theme::default());
        });
        cx.executor().allow_parking();
        let (engine, inputs, _) = fake_engine_with(runtime, false);
        let (view, cx) = cx.add_window_view(|_, cx| {
            let mut view = DeviceStreamView::new(
                engine,
                "local".into(),
                "SIM".into(),
                DevicePlatform::Ios,
                cx,
            );
            view.debug_tap = tap;
            view.three_d = ThreeD::new();
            view
        });
        cx.run_until_parked();
        (view, inputs, cx)
    }

    #[cfg(target_os = "macos")]
    fn push_frame(view: &Entity<DeviceStreamView>, cx: &mut VisualTestContext, seq: u64) {
        view.update(cx, |view, cx| {
            let item = json!({ "seq": seq, "jpeg": jpeg() });
            *view.slot.lock().unwrap() = decode(item, DeviceOrientation::Portrait);
            view.take_frame(cx);
        });
    }

    #[cfg(target_os = "macos")]
    #[gpui::test]
    fn the_debug_tap_waits_until_the_3d_view_has_placed_its_screen(cx: &mut TestAppContext) {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let _guard = runtime.enter();
        let (view, inputs, cx) = three_d_view(cx, &runtime, Some((0.5, 0.5)));
        push_frame(&view, cx, 1);
        assert!(view.read_with(cx, |view, _| view.frame_box().is_some()));
        assert!(inputs.lock().unwrap().is_empty());
        assert_eq!(
            view.read_with(cx, |view, _| view.debug_tap),
            Some((0.5, 0.5)),
            "no camera yet, so the tap is kept for a later frame"
        );
        cx.run_until_parked();
        push_frame(&view, cx, 2);
        wait(cx, |_| inputs.lock().unwrap().len() >= 3);
        assert_touches(
            &touches(&inputs),
            &[("begin", 0.5, 0.5), ("move", 0.5, 0.5), ("end", 0.5, 0.5)],
        );
        assert_eq!(view.read_with(cx, |view, _| view.debug_tap), None);
    }

    #[cfg(target_os = "macos")]
    #[gpui::test]
    fn a_3d_view_whose_renders_keep_failing_falls_back_to_flat(cx: &mut TestAppContext) {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let _guard = runtime.enter();
        let (view, _, cx) = three_d_view(cx, &runtime, None);
        push_frame(&view, cx, 1);
        cx.run_until_parked();
        let fail = |cx: &mut VisualTestContext, failures| {
            view.update(cx, |view, cx| {
                view.three_d
                    .as_ref()
                    .unwrap()
                    .renderer
                    .borrow_mut()
                    .failures = failures;
                view.retry_three_d(cx);
            });
            cx.run_until_parked();
            view.read_with(cx, |view, _| (view.three_d_shown(), view.three_d_failed()))
        };
        assert_eq!(fail(cx, RENDER_ATTEMPTS - 1), (true, false));
        assert_eq!(fail(cx, RENDER_ATTEMPTS), (false, true));
        view.update(cx, |view, cx| view.set_three_d(true, cx));
        assert!(!view.read_with(cx, |view, _| view.three_d_shown()));
    }

    fn duo_screen(
        screen_id: u32,
        hinge_pose: zeron_proto::device_hub::HingePose,
    ) -> DeviceScreenConfig {
        let inner = screen_id == 3;
        DeviceScreenConfig {
            width: if inner { 2006 } else { 1398 },
            height: if inner { 2852 } else { 2034 },
            orientation: if inner {
                DeviceOrientation::LandscapeLeft
            } else {
                DeviceOrientation::Portrait
            },
            screen_id: Some(screen_id),
            supports_hinge_angle: Some(true),
            hinge_pose: Some(hinge_pose),
            ..Default::default()
        }
    }

    #[gpui::test]
    fn a_fold_onto_the_other_screen_ends_a_held_touch(cx: &mut TestAppContext) {
        use zeron_proto::device_hub::HingePose;
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let _guard = runtime.enter();
        setup(cx);
        let (engine, inputs) = fake_engine(&runtime);
        let (view, cx) = cx.add_window_view(|_, cx| {
            DeviceStreamView::new(
                engine,
                "local".into(),
                "SIM".into(),
                DevicePlatform::Ios,
                cx,
            )
        });
        wait(cx, |cx| {
            view.read_with(cx, |view, _| view.has_frame() && view.frame_box().is_some())
        });
        let frame = view.read_with(cx, |view, _| view.frame_box().unwrap());
        let at = |x, y| geometry::position_at(frame, x, y);
        let none = Modifiers::default();
        cx.simulate_mouse_down(at(0.5, 0.5), MouseButton::Left, none);
        view.update(cx, |view, cx| {
            view.set_screen(Some(duo_screen(1, HingePose::Closed)), cx)
        });
        cx.simulate_mouse_move(at(0.6, 0.5), MouseButton::Left, none);
        wait(cx, |_| inputs.lock().unwrap().len() >= 2);
        let mut tent = duo_screen(1, HingePose::Tent);
        tent.hinge_angle = Some(80);
        view.update(cx, |view, cx| view.set_screen(Some(tent), cx));
        cx.run_until_parked();
        std::thread::sleep(Duration::from_millis(100));
        cx.run_until_parked();
        assert_eq!(
            inputs.lock().unwrap().len(),
            2,
            "a new pose on the same screen keeps the press"
        );
        view.update(cx, |view, cx| {
            view.set_screen(Some(duo_screen(3, HingePose::Open)), cx)
        });
        wait(cx, |_| inputs.lock().unwrap().len() >= 3);
        cx.simulate_mouse_move(at(0.7, 0.5), MouseButton::Left, none);
        cx.simulate_mouse_up(at(0.7, 0.5), MouseButton::Left, none);
        std::thread::sleep(Duration::from_millis(100));
        cx.run_until_parked();
        assert_touches(
            &touches(&inputs),
            &[("begin", 0.5, 0.5), ("move", 0.6, 0.5), ("end", 0.6, 0.5)],
        );
    }

    #[gpui::test]
    fn a_pose_request_ends_a_held_touch_before_it_leaves(cx: &mut TestAppContext) {
        use zeron_proto::device_hub::HingePose;
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let _guard = runtime.enter();
        setup(cx);
        let (engine, inputs) = fake_engine(&runtime);
        let fold_engine = engine.clone();
        let (view, cx) = cx.add_window_view(|_, cx| {
            DeviceStreamView::new(
                engine,
                "local".into(),
                "SIM".into(),
                DevicePlatform::Ios,
                cx,
            )
        });
        wait(cx, |cx| {
            view.read_with(cx, |view, _| view.has_frame() && view.frame_box().is_some())
        });
        let frame = view.read_with(cx, |view, _| view.frame_box().unwrap());
        let held = geometry::position_at(frame, 0.5, 0.5);
        cx.simulate_mouse_down(held, MouseButton::Left, Modifiers::default());
        wait(cx, |_| inputs.lock().unwrap().len() == 1);
        let fold = cx.update(|_, cx| {
            cx.new(|cx| {
                crate::device_panel::fold::FoldControls::new(
                    fold_engine,
                    "local".into(),
                    "SIM".into(),
                    DevicePlatform::Ios,
                    &view,
                    cx,
                )
            })
        });
        fold.update(cx, |fold, cx| fold.set_pose(HingePose::Open, cx));
        wait(cx, |_| inputs.lock().unwrap().len() == 2);
        assert_touches(&touches(&inputs), &[("begin", 0.5, 0.5), ("end", 0.5, 0.5)]);
    }

    #[cfg(target_os = "macos")]
    type Scene = Rc<RefCell<Scene3d>>;

    #[cfg(target_os = "macos")]
    fn duo_view<'a>(
        cx: &'a mut TestAppContext,
        runtime: &tokio::runtime::Runtime,
        name: &'static str,
        refuse: impl Fn(usize) -> Option<&'static str> + Send + 'static,
    ) -> (
        Entity<DeviceStreamView>,
        Inputs,
        Inputs,
        Entity<crate::device_panel::fold::FoldControls>,
        &'a mut VisualTestContext,
    ) {
        setup(cx);
        let (engine, inputs, _, hinges) = hinge_engine(runtime, false, refuse);
        let fold_engine = engine.clone();
        let (view, cx) = cx.add_window_view(|_, cx| {
            let mut view = DeviceStreamView::new(
                engine,
                "local".into(),
                "SIM".into(),
                DevicePlatform::Ios,
                cx,
            );
            view.set_label(name.into(), String::new(), cx);
            view.three_d = ThreeD::new();
            view
        });
        let fold = cx.update(|_, cx| {
            cx.new(|cx| {
                crate::device_panel::fold::FoldControls::new(
                    fold_engine,
                    "local".into(),
                    "SIM".into(),
                    DevicePlatform::Ios,
                    &view,
                    cx,
                )
            })
        });
        cx.run_until_parked();
        (view, inputs, hinges, fold, cx)
    }

    #[cfg(target_os = "macos")]
    fn push_duo_frame(
        view: &Entity<DeviceStreamView>,
        cx: &mut VisualTestContext,
        seq: u64,
        rotation: DeviceOrientation,
    ) {
        view.update(cx, |view, cx| {
            let item = json!({ "seq": seq, "jpeg": jpeg_sized(141, 200) });
            *view.slot.lock().unwrap() = decode(item, rotation);
            view.take_frame(cx);
        });
    }

    #[cfg(target_os = "macos")]
    fn scene_of(view: &Entity<DeviceStreamView>, cx: &mut VisualTestContext) -> Scene {
        view.read_with(cx, |view, _| view.three_d.as_ref().unwrap().viewer.clone())
    }

    #[cfg(target_os = "macos")]
    fn duo_ready(view: &Entity<DeviceStreamView>, cx: &mut VisualTestContext) -> Scene {
        use zeron_proto::device_hub::HingePose;
        view.update(cx, |view, cx| {
            view.set_screen(Some(duo_screen(3, HingePose::Open)), cx)
        });
        push_duo_frame(view, cx, 1, DeviceOrientation::LandscapeLeft);
        let scene = scene_of(view, cx);
        wait(cx, |_| {
            let mut scene = scene.borrow_mut();
            scene
                .duo()
                .is_some_and(|duo| duo.accepts_touch() && !duo.needs_frame())
        });
        scene
    }

    #[cfg(target_os = "macos")]
    fn settle_scene(view: &Entity<DeviceStreamView>, scene: &Scene, cx: &mut VisualTestContext) {
        let started = std::time::Instant::now();
        while scene.borrow().needs_frame() {
            assert!(
                started.elapsed() < Duration::from_secs(5),
                "the Duo never settled"
            );
            std::thread::sleep(Duration::from_millis(16));
            view.update(cx, |view, cx| {
                let now = view.now();
                scene.borrow_mut().advance(now, false);
                cx.notify();
            });
            cx.run_until_parked();
        }
    }

    #[cfg(target_os = "macos")]
    fn duo_point(scene: &Scene, panel: DuoPanel, raw: (f32, f32)) -> Point<Pixels> {
        let (x, y) = scene
            .borrow_mut()
            .duo()
            .unwrap()
            .position_of(panel, raw)
            .unwrap();
        point(px(x as f32), px(y as f32))
    }

    #[cfg(target_os = "macos")]
    fn angles(hinges: &Inputs) -> Vec<f64> {
        hinges
            .lock()
            .unwrap()
            .iter()
            .map(|params| {
                assert_eq!(params["deviceId"], "SIM");
                params["angle"].as_f64().unwrap()
            })
            .collect()
    }

    #[cfg(target_os = "macos")]
    fn pinch_at(
        cx: &mut VisualTestContext,
        position: Point<Pixels>,
        phase: gpui::TouchPhase,
        delta: f32,
    ) {
        cx.simulate_event(gpui::PinchEvent {
            position,
            delta,
            modifiers: Modifiers::default(),
            phase,
        });
    }

    #[cfg(target_os = "macos")]
    #[gpui::test]
    fn a_duo_opens_in_3d_on_its_two_leaf_rig_and_touches_the_active_display_in_raw_coordinates(
        cx: &mut TestAppContext,
    ) {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let _guard = runtime.enter();
        let (view, inputs, hinges, _fold, cx) = duo_view(cx, &runtime, "iPhone Duo", |_| None);
        let scene = duo_ready(&view, cx);
        assert!(scene.borrow().is_duo());
        let rig = scene.borrow_mut().duo().unwrap().rig().unwrap();
        assert!(!rig.left.is_empty() && !rig.right.is_empty());
        let at = duo_point(&scene, DuoPanel::Inner, (0.25, 0.3));
        let none = Modifiers::default();
        cx.simulate_mouse_down(at, MouseButton::Left, none);
        cx.simulate_mouse_up(at, MouseButton::Left, none);
        wait(cx, |_| inputs.lock().unwrap().len() >= 3);
        assert_touches(
            &touches(&inputs),
            &[
                ("begin", 0.25, 0.3),
                ("move", 0.25, 0.3),
                ("end", 0.25, 0.3),
            ],
        );
        assert!(hinges.lock().unwrap().is_empty());
    }

    #[cfg(target_os = "macos")]
    #[gpui::test]
    fn pinching_over_the_duo_sends_hinge_angles_and_off_device_pinch_does_nothing(
        cx: &mut TestAppContext,
    ) {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let _guard = runtime.enter();
        let (view, _, hinges, _fold, cx) = duo_view(cx, &runtime, "iPhone Duo", |_| None);
        let scene = duo_ready(&view, cx);
        let over = duo_point(&scene, DuoPanel::Inner, (0.5, 0.75));
        pinch_at(cx, over, gpui::TouchPhase::Started, 0.0);
        pinch_at(cx, over, gpui::TouchPhase::Moved, -0.1);
        pinch_at(cx, over, gpui::TouchPhase::Moved, -0.1);
        pinch_at(cx, over, gpui::TouchPhase::Ended, 0.0);
        wait(cx, |_| !angles(&hinges).is_empty());
        let pinched = angles(&hinges);
        assert!(
            pinched.iter().all(|angle| *angle < 180.0 && *angle > 140.0),
            "{pinched:?}"
        );
        settle_scene(&view, &scene, cx);

        let before = angles(&hinges).len();
        let over = duo_point(&scene, DuoPanel::Inner, (0.5, 0.75));
        cx.simulate_event(gpui::ScrollWheelEvent {
            position: over,
            delta: gpui::ScrollDelta::Pixels(point(px(0.0), px(30.0))),
            modifiers: Modifiers {
                control: true,
                ..Default::default()
            },
            touch_phase: gpui::TouchPhase::Moved,
        });
        wait(cx, |_| angles(&hinges).len() > before);
        assert!(
            (angles(&hinges)[before] - 144.0).abs() < 1e-6,
            "{:?}",
            angles(&hinges)
        );
        assert!(scene.borrow_mut().duo().unwrap().pinching());
        cx.executor().advance_clock(PINCH_IDLE);
        cx.run_until_parked();
        assert!(!scene.borrow_mut().duo().unwrap().pinching());
        settle_scene(&view, &scene, cx);

        let before = angles(&hinges).len();
        let corner = view
            .read_with(cx, |view, _| view.frame_box().unwrap())
            .origin
            + point(px(4.0), px(4.0));
        pinch_at(cx, corner, gpui::TouchPhase::Started, 0.0);
        pinch_at(cx, corner, gpui::TouchPhase::Moved, -0.2);
        pinch_at(cx, corner, gpui::TouchPhase::Ended, 0.0);
        std::thread::sleep(Duration::from_millis(100));
        cx.run_until_parked();
        assert_eq!(
            angles(&hinges).len(),
            before,
            "a pinch beside the device is ignored"
        );
    }

    #[cfg(target_os = "macos")]
    #[gpui::test]
    fn a_timed_out_hinge_command_ends_the_pinch_and_the_next_pinch_keeps_its_preview(
        cx: &mut TestAppContext,
    ) {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let _guard = runtime.enter();
        let (view, _, hinges, fold, cx) = duo_view(cx, &runtime, "iPhone Duo", |call| {
            (call == 0).then_some("Device control timed out. Its position is unknown.")
        });
        let scene = duo_ready(&view, cx);
        let over = duo_point(&scene, DuoPanel::Inner, (0.5, 0.75));
        pinch_at(cx, over, gpui::TouchPhase::Started, 0.0);
        pinch_at(cx, over, gpui::TouchPhase::Moved, -0.1);
        wait(cx, |cx| {
            fold.read_with(cx, |fold, _| fold.error().is_some())
        });
        assert!(
            !scene.borrow_mut().duo().unwrap().pinching(),
            "the failure ends the pinch"
        );
        settle_scene(&view, &scene, cx);
        assert_eq!(scene.borrow_mut().duo().unwrap().angle(), 180.0);

        let over = duo_point(&scene, DuoPanel::Inner, (0.5, 0.75));
        pinch_at(cx, over, gpui::TouchPhase::Started, 0.0);
        pinch_at(cx, over, gpui::TouchPhase::Moved, -0.1);
        wait(cx, |_| angles(&hinges).len() == 2);
        cx.run_until_parked();
        let mut scene = scene.borrow_mut();
        let duo = scene.duo().unwrap();
        assert!(
            duo.pinching(),
            "a stale error does not cancel the next pinch"
        );
        assert!(duo.angle() < 180.0);
    }

    #[cfg(target_os = "macos")]
    #[gpui::test]
    fn a_duo_named_device_waits_for_its_screen_config_before_choosing_a_3d_body(
        cx: &mut TestAppContext,
    ) {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let _guard = runtime.enter();
        let (view, inputs, _, _fold, cx) = duo_view(cx, &runtime, "iPhone 17", |_| None);
        push_duo_frame(&view, cx, 1, DeviceOrientation::Portrait);
        let scene = scene_of(&view, cx);
        wait(cx, |_| scene.borrow().viewport().is_some());
        assert!(
            !scene.borrow().is_duo(),
            "an iPhone enters 3D on its first frame"
        );

        view.update(cx, |view, cx| {
            view.set_three_d(false, cx);
            view.set_label("iPhone Duo".into(), String::new(), cx);
            view.set_three_d(true, cx);
        });
        push_duo_frame(&view, cx, 2, DeviceOrientation::Portrait);
        cx.run_until_parked();
        let scene = scene_of(&view, cx);
        assert!(
            scene.borrow().viewport().is_none(),
            "no body before the screen config"
        );
        assert!(!scene.borrow().is_duo());
        let flat = view.read_with(cx, |view, _| view.frame_box().unwrap());
        let at = geometry::position_at(flat, 0.5, 0.5);
        cx.simulate_mouse_down(at, MouseButton::Left, Modifiers::default());
        cx.simulate_mouse_up(at, MouseButton::Left, Modifiers::default());
        wait(cx, |_| inputs.lock().unwrap().len() >= 2);
        assert_touches(&touches(&inputs), &[("begin", 0.5, 0.5), ("end", 0.5, 0.5)]);

        duo_ready(&view, cx);
        assert!(scene.borrow().is_duo());
        let three_d = view.read_with(cx, |view, _| view.frame_box().unwrap());
        assert!(
            three_d.size.width > flat.size.width,
            "{three_d:?} vs {flat:?}"
        );
    }

    #[gpui::test]
    fn duo_commands_from_the_view_go_out_one_at_a_time_through_the_fold_queue(
        cx: &mut TestAppContext,
    ) {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let _guard = runtime.enter();
        setup(cx);
        let (engine, _, _, hinges) = hinge_engine(&runtime, true, |_| None);
        let fold_engine = engine.clone();
        let (view, cx) = cx.add_window_view(|_, cx| {
            DeviceStreamView::new(
                engine,
                "local".into(),
                "SIM".into(),
                DevicePlatform::Ios,
                cx,
            )
        });
        let fold = cx.update(|_, cx| {
            cx.new(|cx| {
                crate::device_panel::fold::FoldControls::new(
                    fold_engine,
                    "local".into(),
                    "SIM".into(),
                    DevicePlatform::Ios,
                    &view,
                    cx,
                )
            })
        });
        view.update(cx, |_, cx| {
            for angle in [45.0, 50.0, 60.0] {
                cx.emit(DuoCommand(HingeCommand::Angle(angle)));
            }
        });
        wait(cx, |_| hinges.lock().unwrap().len() == 2);
        wait(cx, |cx| !fold.read_with(cx, |fold, _| fold.hinge_pending()));
        assert_eq!(
            *hinges.lock().unwrap(),
            [
                json!({"deviceId": "SIM", "angle": 45.0}),
                json!({"deviceId": "SIM", "angle": 60.0})
            ],
            "50 was replaced while 45 was in flight"
        );
    }

    fn inputs_of(inputs: &Inputs, kind: &str) -> Vec<Value> {
        inputs
            .lock()
            .unwrap()
            .iter()
            .filter_map(|params| params["input"].get(kind).cloned())
            .collect()
    }

    fn setup(cx: &mut TestAppContext) {
        cx.update(|cx| {
            gpui_base::init(cx);
            cx.set_global(Theme::default());
        });
        cx.executor().allow_parking();
    }

    #[test]
    fn a_move_queued_behind_an_in_flight_send_is_replaced_by_the_newest() {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let _guard = runtime.enter();
        let (out, mut requests) = tokio::sync::mpsc::channel::<String>(64);
        let (replies, inbound) = tokio::sync::mpsc::channel::<String>(64);
        let (in_flight, mut first_arrived) = tokio::sync::mpsc::unbounded_channel::<()>();
        let release = Arc::new(tokio::sync::Notify::new());
        let delivered: Inputs = Arc::default();
        let (seen, held) = (delivered.clone(), release.clone());
        runtime.spawn(async move {
            let mut first = true;
            while let Some(frame) = requests.recv().await {
                let frame: Value = serde_json::from_str(&frame).unwrap();
                seen.lock().unwrap().push(frame["params"]["input"].clone());
                if std::mem::take(&mut first) {
                    in_flight.send(()).unwrap();
                    held.notified().await;
                }
                let reply = json!({ "id": frame["id"], "ok": {} });
                if replies.send(reply.to_string()).await.is_err() {
                    break;
                }
            }
        });
        let engine = EngineHandle::from_test_client(zeron_rpc::RpcClient::new(out, inbound));
        let (inputs, queued) = mpsc::unbounded::<DeviceInput>();
        runtime.spawn(send_inputs(
            engine,
            crate::device_panel::device_target("local", "SIM"),
            queued,
        ));
        let touch = |phase, x| DeviceInput::Touch { phase, x, y: 0.5 };
        inputs
            .unbounded_send(touch(TouchPhase::Begin, 0.1))
            .unwrap();
        runtime.block_on(first_arrived.recv()).unwrap();
        for x in [0.2, 0.3, 0.4] {
            inputs.unbounded_send(touch(TouchPhase::Move, x)).unwrap();
        }
        inputs.unbounded_send(touch(TouchPhase::End, 0.5)).unwrap();
        release.notify_one();
        runtime.block_on(async {
            for _ in 0..200 {
                if delivered.lock().unwrap().len() >= 3 {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        });
        let wire = |phase: &str, x: f64| json!({ "touch": { "phase": phase, "x": x, "y": 0.5 } });
        let delivered = delivered.lock().unwrap().clone();
        let rounded: Vec<Value> = delivered
            .iter()
            .map(|input| {
                let touch = &input["touch"];
                let x = (touch["x"].as_f64().unwrap() * 10.0).round() / 10.0;
                wire(touch["phase"].as_str().unwrap(), x)
            })
            .collect();
        assert_eq!(
            rounded,
            vec![wire("begin", 0.1), wire("move", 0.4), wire("end", 0.5)]
        );
    }

    struct Host(Entity<DeviceStreamView>);

    impl Render for Host {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            div()
                .size_full()
                .child(div().w(px(300.0)).h(px(400.0)).child(self.0.clone()))
        }
    }

    #[gpui::test]
    fn a_drag_that_leaves_the_view_keeps_moving_clamped_to_the_frame(cx: &mut TestAppContext) {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let _guard = runtime.enter();
        setup(cx);
        let (engine, inputs) = fake_engine(&runtime);
        let view = cx.new(|cx| {
            DeviceStreamView::new(
                engine,
                "local".into(),
                "SIM".into(),
                DevicePlatform::Ios,
                cx,
            )
        });
        let host = view.clone();
        let (_, cx) = cx.add_window_view(|_, _| Host(host));
        wait(cx, |cx| {
            view.read_with(cx, |view, _| view.has_frame() && view.frame_box().is_some())
        });
        let frame = view.read_with(cx, |view, _| view.frame_box().unwrap());
        let outside = point(px(700.0), geometry::position_at(frame, 0.0, 0.25).y);
        let none = Modifiers::default();
        cx.simulate_mouse_down(
            geometry::position_at(frame, 0.5, 0.5),
            MouseButton::Left,
            none,
        );
        cx.simulate_mouse_move(outside, MouseButton::Left, none);
        cx.simulate_mouse_up(outside, MouseButton::Left, none);
        wait(cx, |_| inputs.lock().unwrap().len() >= 3);
        assert_touches(
            &touches(&inputs),
            &[("begin", 0.5, 0.5), ("move", 1.0, 0.25), ("end", 1.0, 0.25)],
        );
    }

    #[gpui::test]
    fn no_video_for_fifteen_seconds_shows_the_error_and_reconnect_resubscribes(
        cx: &mut TestAppContext,
    ) {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let _guard = runtime.enter();
        setup(cx);
        let (engine, _, subscriptions) = fake_engine_with(&runtime, false);
        let (view, cx) = cx.add_window_view(|_, cx| {
            DeviceStreamView::new(
                engine,
                "local".into(),
                "SIM".into(),
                DevicePlatform::Ios,
                cx,
            )
        });
        wait(cx, |_| subscriptions.load(Ordering::SeqCst) == 1);
        assert!(view.read_with(cx, |view, _| view.video_error.is_none()));
        cx.executor()
            .advance_clock(NO_VIDEO_TIMEOUT - Duration::from_secs(1));
        cx.run_until_parked();
        assert!(view.read_with(cx, |view, _| view.video_error.is_none()));
        cx.executor().advance_clock(Duration::from_secs(1));
        wait(cx, |cx| {
            view.read_with(cx, |view, _| view.video_error.as_deref() == Some(NO_VIDEO))
        });
        let reconnect = cx
            .debug_bounds("device-reconnect")
            .expect("Reconnect is shown");
        cx.simulate_click(reconnect.center(), Modifiers::default());
        wait(cx, |_| subscriptions.load(Ordering::SeqCst) == 2);
        assert!(view.read_with(cx, |view, _| view.video_error.is_none()));
    }

    struct KeyHost {
        view: Option<Entity<DeviceStreamView>>,
        passed: Rc<std::cell::RefCell<Vec<String>>>,
        toggled: Rc<Cell<bool>>,
    }

    impl Render for KeyHost {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            let (passed, toggled) = (self.passed.clone(), self.toggled.clone());
            div()
                .size_full()
                .on_key_down(move |event: &KeyDownEvent, _, _| {
                    passed.borrow_mut().push(event.keystroke.key.clone())
                })
                .on_action(move |_: &crate::shell::ToggleChanges, _, _| toggled.set(true))
                .children(
                    self.view
                        .clone()
                        .map(|view| div().w(px(300.0)).h(px(400.0)).child(view)),
                )
        }
    }

    fn key_host<'a>(
        cx: &'a mut TestAppContext,
        runtime: &tokio::runtime::Runtime,
    ) -> (
        Entity<DeviceStreamView>,
        Entity<KeyHost>,
        Inputs,
        &'a mut VisualTestContext,
    ) {
        key_host_on(cx, runtime, DevicePlatform::Ios)
    }

    fn key_host_on<'a>(
        cx: &'a mut TestAppContext,
        runtime: &tokio::runtime::Runtime,
        platform: DevicePlatform,
    ) -> (
        Entity<DeviceStreamView>,
        Entity<KeyHost>,
        Inputs,
        &'a mut VisualTestContext,
    ) {
        setup(cx);
        let (engine, inputs) = fake_engine(runtime);
        let view =
            cx.new(|cx| DeviceStreamView::new(engine, "local".into(), "SIM".into(), platform, cx));
        let hosted = view.clone();
        let (host, cx) = cx.add_window_view(|_, _| KeyHost {
            view: Some(hosted),
            passed: Rc::default(),
            toggled: Rc::default(),
        });
        let focus = view.read_with(cx, |view, _| view.focus.clone());
        cx.update(|window, cx| {
            window.activate_window();
            window.focus(&focus, cx)
        });
        cx.run_until_parked();
        (view, host, inputs, cx)
    }

    fn key(phase: &str, usage: u16) -> Value {
        json!({ "phase": phase, "usage": usage })
    }

    #[gpui::test]
    fn keys_on_the_focused_screen_are_forwarded_and_other_keys_pass_through(
        cx: &mut TestAppContext,
    ) {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let _guard = runtime.enter();
        let (_view, host, inputs, cx) = key_host(cx, &runtime);
        cx.simulate_keystrokes("shift-a");
        cx.simulate_event(gpui::KeyUpEvent {
            keystroke: Keystroke::parse("shift-a").unwrap(),
        });
        cx.simulate_keystrokes("cmd-w f1");
        wait(cx, |_| inputs_of(&inputs, "key").len() >= 4);
        assert_eq!(
            inputs_of(&inputs, "key"),
            vec![
                key("down", 0xE1),
                key("down", 0x04),
                key("up", 0xE1),
                key("up", 0x04),
            ]
        );
        assert_eq!(
            *host.read_with(cx, |host, _| host.passed.clone()).borrow(),
            vec!["w".to_string(), "f1".to_string()]
        );
    }

    #[gpui::test]
    fn android_keys_go_out_as_keycodes_back_and_typed_text_without_key_ups(
        cx: &mut TestAppContext,
    ) {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let _guard = runtime.enter();
        let (_view, host, inputs, cx) = key_host_on(cx, &runtime, DevicePlatform::Android);
        cx.simulate_keystrokes("a enter escape cmd-w");
        cx.simulate_event(gpui::KeyUpEvent {
            keystroke: Keystroke::parse("a").unwrap(),
        });
        wait(cx, |_| inputs.lock().unwrap().len() >= 3);
        cx.run_until_parked();
        let sent: Vec<Value> = inputs
            .lock()
            .unwrap()
            .iter()
            .map(|params| params["input"].clone())
            .collect();
        assert_eq!(
            sent,
            vec![
                json!({"text": {"text": "a"}}),
                json!({"keycode": {"keycode": 66}}),
                json!({"button": {"button": "back"}}),
            ]
        );
        assert_eq!(
            *host.read_with(cx, |host, _| host.passed.clone()).borrow(),
            vec!["w".to_string()]
        );
    }

    #[gpui::test]
    fn committed_ime_text_on_android_goes_out_as_typed_text_and_composition_stays_local(
        cx: &mut TestAppContext,
    ) {
        use gpui::EntityInputHandler as _;
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let _guard = runtime.enter();
        let (view, _host, inputs, cx) = key_host_on(cx, &runtime, DevicePlatform::Android);
        view.update_in(cx, |view, window, cx| {
            view.replace_and_mark_text_in_range(None, "かn", None, window, cx);
            assert_eq!(view.marked_text_range(window, cx), Some(0..2));
        });
        cx.update(|window, cx| {
            window.dispatch_keystroke(
                Keystroke {
                    key: "a".into(),
                    key_char: Some("かな".into()),
                    ..Default::default()
                },
                cx,
            )
        });
        wait(cx, |_| !inputs.lock().unwrap().is_empty());
        cx.run_until_parked();
        let sent: Vec<Value> = inputs
            .lock()
            .unwrap()
            .iter()
            .map(|params| params["input"].clone())
            .collect();
        assert_eq!(sent, vec![json!({"text": {"text": "かな"}})]);
        view.update_in(cx, |view, window, cx| {
            assert_eq!(view.marked_text_range(window, cx), None)
        });
    }

    #[gpui::test]
    fn cmd_r_reaches_the_device_over_the_shell_toggle_changes_binding(cx: &mut TestAppContext) {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let _guard = runtime.enter();
        let (_view, host, inputs, cx) = key_host(cx, &runtime);
        cx.update(|_, cx| {
            crate::shell::apply_keymap(
                cx,
                &crate::settings::KeymapConfig::default(),
                crate::settings::ComposerSendBehavior::Enter,
                Default::default(),
            )
        });
        cx.simulate_keystrokes("cmd-r");
        wait(cx, |_| inputs_of(&inputs, "key").len() >= 4);
        assert_eq!(
            inputs_of(&inputs, "key"),
            vec![
                key("down", 0xE3),
                key("down", 0x15),
                key("up", 0x15),
                key("up", 0xE3),
            ]
        );
        assert!(!host.read_with(cx, |host, _| host.toggled.get()));
    }

    fn press_and_hold(
        view: &Entity<DeviceStreamView>,
        inputs: &Inputs,
        cx: &mut VisualTestContext,
    ) {
        wait(cx, |cx| {
            view.read_with(cx, |view, _| view.has_frame() && view.frame_box().is_some())
        });
        let frame = view.read_with(cx, |view, _| view.frame_box().unwrap());
        cx.simulate_mouse_down(
            geometry::position_at(frame, 0.5, 0.5),
            MouseButton::Left,
            Modifiers::default(),
        );
        cx.simulate_keystrokes("a");
        wait(cx, |_| inputs.lock().unwrap().len() >= 2);
    }

    fn released(inputs: &Inputs) -> bool {
        let at_centre = |touch: &Value| {
            touch["phase"] == "end"
                && (touch["x"].as_f64().unwrap() - 0.5).abs() < 0.01
                && (touch["y"].as_f64().unwrap() - 0.5).abs() < 0.01
        };
        inputs_of(inputs, "key").last() == Some(&key("up", 0x04))
            && inputs_of(inputs, "touch").last().is_some_and(at_centre)
    }

    #[gpui::test]
    fn held_keys_and_the_touch_are_released_when_focus_leaves(cx: &mut TestAppContext) {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let _guard = runtime.enter();
        let (view, _host, inputs, cx) = key_host(cx, &runtime);
        press_and_hold(&view, &inputs, cx);
        cx.update(|window, _| window.blur());
        wait(cx, |_| released(&inputs));
    }

    #[gpui::test]
    fn held_keys_and_the_touch_are_released_when_the_window_deactivates(cx: &mut TestAppContext) {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let _guard = runtime.enter();
        let (view, _host, inputs, cx) = key_host(cx, &runtime);
        press_and_hold(&view, &inputs, cx);
        cx.deactivate_window();
        wait(cx, |_| released(&inputs));
    }

    #[gpui::test]
    fn held_keys_and_the_touch_are_released_when_the_view_is_dropped(cx: &mut TestAppContext) {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let _guard = runtime.enter();
        let (view, host, inputs, cx) = key_host(cx, &runtime);
        press_and_hold(&view, &inputs, cx);
        let weak = view.downgrade();
        drop(view);
        host.update(cx, |host, cx| {
            host.view = None;
            cx.notify();
        });
        wait(cx, |_| released(&inputs));
        assert!(weak.upgrade().is_none());
    }

    #[gpui::test]
    fn command_down_releases_held_keys(cx: &mut TestAppContext) {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let _guard = runtime.enter();
        let (_view, _host, inputs, cx) = key_host(cx, &runtime);
        cx.simulate_keystrokes("a");
        cx.simulate_modifiers_change(Modifiers::command());
        cx.simulate_modifiers_change(Modifiers::default());
        cx.simulate_keystrokes("a");
        wait(cx, |_| inputs_of(&inputs, "key").len() >= 3);
        assert_eq!(
            inputs_of(&inputs, "key"),
            vec![key("down", 0x04), key("up", 0x04), key("down", 0x04)]
        );
    }

    #[gpui::test]
    fn home_and_rotate_follow_the_screen_orientation(cx: &mut TestAppContext) {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let _guard = runtime.enter();
        setup(cx);
        let (engine, inputs) = fake_engine(&runtime);
        let (view, cx) = cx.add_window_view(|_, cx| {
            DeviceStreamView::new(
                engine,
                "local".into(),
                "SIM".into(),
                DevicePlatform::Ios,
                cx,
            )
        });
        view.update(cx, |view, cx| {
            view.press_button(DeviceButton::Home);
            view.rotate();
            view.set_screen(
                Some(DeviceScreenConfig {
                    width: 588,
                    height: 1280,
                    orientation: DeviceOrientation::LandscapeRight,
                    ..Default::default()
                }),
                cx,
            );
            view.rotate();
        });
        wait(cx, |_| inputs.lock().unwrap().len() >= 3);
        assert_eq!(
            inputs_of(&inputs, "button"),
            vec![json!({ "button": "home" })]
        );
        assert_eq!(
            inputs_of(&inputs, "orientation"),
            vec![
                json!({ "orientation": "landscape_left" }),
                json!({ "orientation": "portrait" }),
            ]
        );
    }

    #[gpui::test]
    fn a_landscape_screen_rotates_the_decoded_frame(cx: &mut TestAppContext) {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let _guard = runtime.enter();
        setup(cx);
        let (engine, _) = fake_engine(&runtime);
        let (view, cx) = cx.add_window_view(|_, cx| {
            DeviceStreamView::new(
                engine,
                "local".into(),
                "SIM".into(),
                DevicePlatform::Ios,
                cx,
            )
        });
        view.update(cx, |view, cx| {
            view.set_screen(
                Some(DeviceScreenConfig {
                    width: 18,
                    height: 39,
                    orientation: DeviceOrientation::LandscapeLeft,
                    ..Default::default()
                }),
                cx,
            )
        });
        wait(cx, |cx| view.read_with(cx, |view, _| view.has_frame()));
        let (width, height) = view.read_with(cx, |view, _| {
            let frame = view.image.as_ref().unwrap();
            (frame.width, frame.height)
        });
        assert_eq!((width, height), (39, 18));
        let frame = view.read_with(cx, |view, _| view.frame_box().unwrap());
        assert!(frame.size.width > frame.size.height);
    }

    #[test]
    fn ax_boxes_turn_with_the_displayed_frame() {
        let raw = AxBox {
            x: 0.1,
            y: 0.2,
            width: 0.3,
            height: 0.4,
            oriented: false,
        };
        for (rotation, shown) in [
            (DeviceOrientation::Portrait, [0.1, 0.2, 0.3, 0.4]),
            (DeviceOrientation::LandscapeLeft, [0.4, 0.1, 0.4, 0.3]),
            (DeviceOrientation::LandscapeRight, [0.2, 0.6, 0.4, 0.3]),
            (DeviceOrientation::PortraitUpsideDown, [0.6, 0.4, 0.3, 0.4]),
        ] {
            let got = raw.displayed(rotation);
            let got = [got.x, got.y, got.width, got.height];
            assert!(
                got.iter().zip(shown).all(|(a, b)| (a - b).abs() < 1e-5),
                "{rotation:?}: {got:?}"
            );
            let oriented = AxBox {
                oriented: true,
                ..raw
            };
            assert_eq!(oriented.displayed(rotation), oriented, "{rotation:?}");
        }
    }

    #[test]
    fn the_codec_knob_forces_either_path_and_defaults_to_h264_on_macos() {
        let default = if cfg!(target_os = "macos") {
            DeviceVideoCodec::H264
        } else {
            DeviceVideoCodec::Mjpeg
        };
        let ios = |knob| codec_choice(DevicePlatform::Ios, knob);
        assert_eq!(ios(None), (default, false));
        assert_eq!(ios(Some("other")), (default, false));
        assert_eq!(ios(Some("mjpeg")), (DeviceVideoCodec::Mjpeg, true));
        assert_eq!(ios(Some("h264")), (default, cfg!(target_os = "macos")));
        for knob in [None, Some("mjpeg"), Some("h264")] {
            assert_eq!(
                codec_choice(DevicePlatform::Android, knob),
                (DeviceVideoCodec::H264, true),
                "Android has no MJPEG stream: {knob:?}"
            );
        }
    }

    #[cfg(target_os = "macos")]
    mod h264 {
        use super::*;
        use zeron_proto::device_hub::{AvccDemuxer, AvccEnvelope};

        const FIXTURE: &[u8] = include_bytes!("../../tests/fixtures/device-ios-588x1280.avcc");

        fn item(envelopes: &[AvccEnvelope]) -> Value {
            let mut bytes = Vec::new();
            for envelope in envelopes {
                envelope.encode_into(&mut bytes);
            }
            json!({ "seq": 1, "avcc": base64::engine::general_purpose::STANDARD.encode(bytes) })
        }

        fn video_engine(
            runtime: &tokio::runtime::Runtime,
            h264: Vec<Value>,
        ) -> (EngineHandle, Inputs) {
            let (out, mut frames) = tokio::sync::mpsc::channel::<String>(64);
            let (replies, inbound) = tokio::sync::mpsc::channel::<String>(64);
            let watches: Inputs = Arc::default();
            let seen = watches.clone();
            runtime.spawn(async move {
                while let Some(frame) = frames.recv().await {
                    let frame: Value = serde_json::from_str(&frame).unwrap();
                    if frame["method"] != methods::WATCH_HUB_DEVICE_FRAMES {
                        continue;
                    }
                    seen.lock().unwrap().push(frame["params"].clone());
                    let items = if frame["params"]["codec"] == "h264" {
                        h264.clone()
                    } else {
                        vec![json!({ "seq": 1, "jpeg": jpeg() })]
                    };
                    for item in items {
                        if item == "pause" {
                            tokio::time::sleep(Duration::from_millis(300)).await;
                            continue;
                        }
                        let reply = json!({ "id": frame["id"], "item": item });
                        if replies.send(reply.to_string()).await.is_err() {
                            return;
                        }
                    }
                }
            });
            let client = zeron_rpc::RpcClient::new(out, inbound);
            (EngineHandle::from_test_client(client), watches)
        }

        fn pump(cx: &mut VisualTestContext, duration: Duration) {
            let until = Instant::now() + duration;
            while Instant::now() < until {
                cx.run_until_parked();
                std::thread::sleep(Duration::from_millis(5));
            }
        }

        fn shown(
            view: &Entity<DeviceStreamView>,
            cx: &mut VisualTestContext,
        ) -> Option<(DeviceVideoCodec, u32, u32, bool)> {
            view.read_with(cx, |view, _| {
                view.image.as_ref().map(|frame| {
                    let surface = matches!(frame.picture, Picture::Surface(_));
                    (frame.codec, frame.width, frame.height, surface)
                })
            })
        }

        fn codecs(watches: &Inputs) -> Vec<Value> {
            watches
                .lock()
                .unwrap()
                .iter()
                .map(|params| params["codec"].clone())
                .collect()
        }

        #[gpui::test]
        fn an_h264_stream_paints_a_decoded_surface(cx: &mut TestAppContext) {
            let runtime = tokio::runtime::Runtime::new().unwrap();
            let _guard = runtime.enter();
            setup(cx);
            let envelopes = AvccDemuxer::default().push(FIXTURE).unwrap();
            let mut items = vec![item(&envelopes[..2])];
            items.extend(
                envelopes[2..]
                    .iter()
                    .map(|envelope| item(std::slice::from_ref(envelope))),
            );
            let (engine, watches) = video_engine(&runtime, items);
            let (view, cx) = cx.add_window_view(|_, cx| {
                DeviceStreamView::new(
                    engine,
                    "local".into(),
                    "SIM".into(),
                    DevicePlatform::Ios,
                    cx,
                )
            });
            wait(cx, |cx| shown(&view, cx).is_some());
            assert_eq!(
                shown(&view, cx),
                Some((DeviceVideoCodec::H264, 588, 1280, true))
            );
            assert_eq!(codecs(&watches), [json!("h264")]);
            let frame = view.read_with(cx, |view, _| view.frame_box().unwrap());
            assert!((frame.size.width / frame.size.height - 588.0 / 1280.0).abs() < 0.01);
        }

        #[gpui::test]
        fn video_without_a_description_falls_back_to_mjpeg(cx: &mut TestAppContext) {
            let runtime = tokio::runtime::Runtime::new().unwrap();
            let _guard = runtime.enter();
            setup(cx);
            let envelopes = AvccDemuxer::default().push(FIXTURE).unwrap();
            let (engine, watches) = video_engine(&runtime, vec![item(&envelopes[1..2])]);
            let (view, cx) = cx.add_window_view(|_, cx| {
                DeviceStreamView::new(
                    engine,
                    "local".into(),
                    "SIM".into(),
                    DevicePlatform::Ios,
                    cx,
                )
            });
            wait(cx, |cx| shown(&view, cx).is_some());
            assert_eq!(
                shown(&view, cx),
                Some((DeviceVideoCodec::Mjpeg, 18, 39, false))
            );
            assert_eq!(codecs(&watches), [json!("h264"), json!("mjpeg")]);
        }

        fn backlog_engine(
            runtime: &tokio::runtime::Runtime,
            bursts: usize,
        ) -> (EngineHandle, Inputs) {
            let envelopes = AvccDemuxer::default().push(FIXTURE).unwrap();
            let burst: Vec<AvccEnvelope> = envelopes[2..].iter().cycle().take(9).cloned().collect();
            let mut items = vec![item(&envelopes[..2])];
            for _ in 0..bursts {
                items.extend([json!("pause"), item(&burst)]);
            }
            items.extend([json!("pause"), item(&envelopes[2..3])]);
            video_engine(runtime, items)
        }

        #[gpui::test]
        fn gop_replays_never_count_as_backlog(cx: &mut TestAppContext) {
            let runtime = tokio::runtime::Runtime::new().unwrap();
            let _guard = runtime.enter();
            setup(cx);
            let envelopes = AvccDemuxer::default().push(FIXTURE).unwrap();
            let replay: Vec<AvccEnvelope> = envelopes[..2]
                .iter()
                .chain(envelopes[2..].iter().cycle().take(20))
                .cloned()
                .collect();
            let mut items = vec![item(&envelopes[..2])];
            for _ in 0..14 {
                items.extend([json!("pause"), item(&replay)]);
            }
            let (engine, watches) = video_engine(&runtime, items);
            let (view, cx) = cx.add_window_view(|_, cx| {
                DeviceStreamView::new(
                    engine,
                    "local".into(),
                    "SIM".into(),
                    DevicePlatform::Ios,
                    cx,
                )
            });
            wait(cx, |cx| shown(&view, cx).is_some());
            pump(cx, Duration::from_millis(4500));
            assert_eq!(codecs(&watches), [json!("h264")]);
            assert_eq!(shown(&view, cx).unwrap().0, DeviceVideoCodec::H264);
        }

        #[gpui::test]
        fn a_failed_avcc_link_with_no_video_falls_back_to_mjpeg(cx: &mut TestAppContext) {
            let runtime = tokio::runtime::Runtime::new().unwrap();
            let _guard = runtime.enter();
            setup(cx);
            let (engine, watches) = video_engine(&runtime, Vec::new());
            let (view, cx) = cx.add_window_view(|_, cx| {
                DeviceStreamView::new(
                    engine,
                    "local".into(),
                    "SIM".into(),
                    DevicePlatform::Ios,
                    cx,
                )
            });
            wait(cx, |_| codecs(&watches).len() == 1);
            let failed = |detail: &str| DeviceStreamStatus {
                avcc: StreamLinkStatus::Failed {
                    detail: detail.into(),
                },
                ..Default::default()
            };
            view.update(cx, |view, cx| {
                view.set_link(Some(failed("the video stream ended")), cx)
            });
            for _ in 0..=LINK_FAILED_TICKS {
                cx.executor().advance_clock(LINK_TICK);
                cx.run_until_parked();
            }
            assert_eq!(
                codecs(&watches),
                [json!("h264")],
                "a hub that went away is not an H.264 failure"
            );
            view.update(cx, |view, cx| view.set_link(Some(failed("HTTP 404")), cx));
            for second in 0..LINK_FAILED_TICKS {
                assert_eq!(codecs(&watches), [json!("h264")], "after {second} s");
                if second == 1 {
                    let connecting = DeviceStreamStatus {
                        avcc: StreamLinkStatus::Connecting,
                        ..Default::default()
                    };
                    view.update(cx, |view, cx| view.set_link(Some(connecting), cx));
                }
                cx.executor().advance_clock(LINK_TICK);
                cx.run_until_parked();
            }
            wait(cx, |cx| {
                shown(&view, cx).is_some_and(|shown| shown.0 == DeviceVideoCodec::Mjpeg)
            });
            assert_eq!(codecs(&watches), [json!("h264"), json!("mjpeg")]);
        }

        #[gpui::test]
        fn each_view_watches_its_own_codecs_link(cx: &mut TestAppContext) {
            let runtime = tokio::runtime::Runtime::new().unwrap();
            let _guard = runtime.enter();
            setup(cx);
            let envelopes = AvccDemuxer::default().push(FIXTURE).unwrap();
            let (engine, _) = video_engine(&runtime, vec![item(&envelopes[..2])]);
            let (view, cx) = cx.add_window_view(|_, cx| {
                DeviceStreamView::new(
                    engine,
                    "local".into(),
                    "SIM".into(),
                    DevicePlatform::Ios,
                    cx,
                )
            });
            wait(cx, |cx| shown(&view, cx).is_some());
            let link = |video, avcc| DeviceStreamStatus {
                video,
                avcc,
                ..Default::default()
            };
            let failed = StreamLinkStatus::Failed {
                detail: "the video stream ended".into(),
            };
            let awaiting = |link: DeviceStreamStatus, cx: &mut VisualTestContext| {
                view.update(cx, |view, cx| {
                    view.set_link(Some(link), cx);
                    view.awaiting_frame
                })
            };
            use StreamLinkStatus::{Connecting, Live};
            assert!(!awaiting(link(Live, Live), cx));
            assert!(
                !awaiting(link(failed.clone(), Live), cx),
                "an MJPEG reader failure leaves the H.264 view alone"
            );
            assert!(awaiting(link(failed, Connecting), cx));
            assert!(
                !awaiting(link(Live, Live), cx),
                "back to Live with a frame to show"
            );
        }

        #[gpui::test]
        fn the_last_three_replaced_surfaces_stay_alive(cx: &mut TestAppContext) {
            let runtime = tokio::runtime::Runtime::new().unwrap();
            let _guard = runtime.enter();
            setup(cx);
            let (engine, _) = video_engine(&runtime, Vec::new());
            let view = cx.new(|cx| {
                DeviceStreamView::new(
                    engine,
                    "local".into(),
                    "SIM".into(),
                    DevicePlatform::Ios,
                    cx,
                )
            });
            let envelopes = AvccDemuxer::default().push(FIXTURE).unwrap();
            let decoder = crate::device_panel::h264::Decoder::new(&envelopes[0].payload).unwrap();
            let surfaces: Vec<_> = envelopes[1..]
                .iter()
                .map(|envelope| decoder.decode(&envelope.payload).unwrap().unwrap())
                .collect();
            view.update(cx, |view, cx| {
                for surface in &surfaces {
                    *view.slot.lock().unwrap() = Some(DecodedFrame {
                        width: surface.width(),
                        height: surface.height(),
                        picture: Picture::Surface(surface.clone()),
                        rotation: DeviceOrientation::Portrait,
                        codec: DeviceVideoCodec::H264,
                    });
                    view.take_frame(cx);
                }
                let kept: Vec<_> = view
                    .kept_surfaces
                    .iter()
                    .map(|surface| surface.buffer() as *const _)
                    .collect();
                let expected: Vec<_> = surfaces[1..4]
                    .iter()
                    .map(|surface| surface.buffer() as *const _)
                    .collect();
                assert_eq!(surfaces.len(), 5);
                assert_eq!(kept, expected);
            });
        }

        #[gpui::test]
        fn one_burst_of_queued_frames_keeps_h264(cx: &mut TestAppContext) {
            let runtime = tokio::runtime::Runtime::new().unwrap();
            let _guard = runtime.enter();
            setup(cx);
            let (engine, watches) = backlog_engine(&runtime, 1);
            let (view, cx) = cx.add_window_view(|_, cx| {
                DeviceStreamView::new(
                    engine,
                    "local".into(),
                    "SIM".into(),
                    DevicePlatform::Ios,
                    cx,
                )
            });
            wait(cx, |cx| shown(&view, cx).is_some());
            std::thread::sleep(Duration::from_millis(1200));
            cx.run_until_parked();
            assert_eq!(codecs(&watches), [json!("h264")]);
            assert_eq!(shown(&view, cx).unwrap().0, DeviceVideoCodec::H264);
        }

        const DUO_FOLD: &[u8] = include_bytes!("../../tests/fixtures/device-ios-duo-fold.avcc");

        #[gpui::test]
        fn a_duo_stream_follows_the_fold_onto_the_inner_screen_and_back(cx: &mut TestAppContext) {
            use zeron_proto::device_hub::HingePose;
            let runtime = tokio::runtime::Runtime::new().unwrap();
            let _guard = runtime.enter();
            setup(cx);
            let mut screens: Vec<Vec<AvccEnvelope>> = Vec::new();
            for envelope in AvccDemuxer::default().push(DUO_FOLD).unwrap() {
                if envelope.tag == zeron_proto::device_hub::AVCC_DESCRIPTION {
                    screens.push(Vec::new());
                }
                screens.last_mut().unwrap().push(envelope);
            }
            let screens: Vec<Value> = screens.iter().map(|screen| item(screen)).collect();
            assert_eq!(screens.len(), 3, "cover, inner, cover");
            let pause = || {
                [
                    json!("pause"),
                    json!("pause"),
                    json!("pause"),
                    json!("pause"),
                ]
            };
            let mut wire = vec![screens[0].clone()];
            wire.extend(pause());
            wire.push(screens[1].clone());
            wire.extend(pause());
            wire.push(screens[2].clone());
            let (engine, watches) = video_engine(&runtime, wire);
            let (view, cx) = cx.add_window_view(|_, cx| {
                view_with_screen(engine, duo_screen(1, HingePose::Closed), cx)
            });
            wait(cx, |cx| shown(&view, cx).is_some());
            assert_eq!(
                shown(&view, cx),
                Some((DeviceVideoCodec::H264, 1398, 2034, true))
            );
            view.update(cx, |view, cx| {
                view.set_screen(Some(duo_screen(3, HingePose::Open)), cx)
            });
            wait(cx, |cx| {
                shown(&view, cx).is_some_and(|shown| shown.1 == 2852)
            });
            assert_eq!(
                shown(&view, cx),
                Some((DeviceVideoCodec::H264, 2852, 2006, true)),
                "the inner display reports landscape_left and turns upright"
            );
            view.update(cx, |view, cx| {
                view.set_screen(Some(duo_screen(1, HingePose::Closed)), cx)
            });
            wait(cx, |cx| {
                shown(&view, cx).is_some_and(|shown| shown.1 == 1398)
            });
            assert_eq!(
                shown(&view, cx),
                Some((DeviceVideoCodec::H264, 1398, 2034, true))
            );
            assert_eq!(codecs(&watches), [json!("h264")]);
            assert!(view.read_with(cx, |view, _| view.video_error.is_none()));
        }

        #[gpui::test]
        fn the_inner_keyframe_before_its_screen_config_turns_upright_when_the_config_lands(
            cx: &mut TestAppContext,
        ) {
            use zeron_proto::device_hub::HingePose;
            let runtime = tokio::runtime::Runtime::new().unwrap();
            let _guard = runtime.enter();
            setup(cx);
            let mut screens: Vec<Vec<AvccEnvelope>> = Vec::new();
            for envelope in AvccDemuxer::default().push(DUO_FOLD).unwrap() {
                if envelope.tag == zeron_proto::device_hub::AVCC_DESCRIPTION {
                    screens.push(Vec::new());
                }
                screens.last_mut().unwrap().push(envelope);
            }
            let (engine, watches) = video_engine(
                &runtime,
                vec![item(&screens[0]), json!("pause"), item(&screens[1])],
            );
            let (view, cx) = cx.add_window_view(|_, cx| {
                view_with_screen(engine, duo_screen(1, HingePose::Closed), cx)
            });
            wait(cx, |cx| {
                shown(&view, cx).is_some_and(|shown| (shown.1, shown.2) == (2006, 2852))
            });
            view.update(cx, |view, cx| {
                view.set_screen(Some(duo_screen(3, HingePose::Open)), cx)
            });
            wait(cx, |cx| {
                shown(&view, cx).is_some_and(|shown| (shown.1, shown.2) == (2852, 2006))
            });
            assert_eq!(codecs(&watches), [json!("h264")], "no new subscription");
        }

        #[gpui::test]
        fn an_mjpeg_frame_turns_when_its_screen_config_lands_after_it(cx: &mut TestAppContext) {
            let runtime = tokio::runtime::Runtime::new().unwrap();
            let _guard = runtime.enter();
            setup(cx);
            let (engine, watches) = video_engine(&runtime, Vec::new());
            let (view, cx) = cx.add_window_view(|_, cx| {
                DeviceStreamView::new(
                    engine,
                    "local".into(),
                    "SIM".into(),
                    DevicePlatform::Ios,
                    cx,
                )
            });
            wait(cx, |_| codecs(&watches).len() == 1);
            let failed = DeviceStreamStatus {
                avcc: StreamLinkStatus::Failed {
                    detail: "HTTP 404".into(),
                },
                ..Default::default()
            };
            view.update(cx, |view, cx| view.set_link(Some(failed), cx));
            for _ in 0..LINK_FAILED_TICKS {
                cx.executor().advance_clock(LINK_TICK);
                cx.run_until_parked();
            }
            wait(cx, |cx| {
                shown(&view, cx) == Some((DeviceVideoCodec::Mjpeg, 18, 39, false))
            });
            let landscape = DeviceScreenConfig {
                width: 18,
                height: 39,
                orientation: DeviceOrientation::LandscapeLeft,
                ..Default::default()
            };
            view.update(cx, |view, cx| view.set_screen(Some(landscape), cx));
            wait(cx, |cx| {
                shown(&view, cx) == Some((DeviceVideoCodec::Mjpeg, 39, 18, false))
            });
            assert_eq!(codecs(&watches), [json!("h264"), json!("mjpeg")]);
        }

        fn view_with_screen(
            engine: EngineHandle,
            screen: DeviceScreenConfig,
            cx: &mut Context<DeviceStreamView>,
        ) -> DeviceStreamView {
            let mut view = DeviceStreamView::new(
                engine,
                "local".into(),
                "SIM".into(),
                DevicePlatform::Ios,
                cx,
            );
            view.set_screen(Some(screen), cx);
            view
        }

        const ANDROID_CAPTURE: &[u8] =
            include_bytes!("../../../proto/tests/fixtures/device-android-576x1280.semu");

        fn android_items() -> Vec<Vec<AvccEnvelope>> {
            let mut items = Vec::new();
            let mut rest = ANDROID_CAPTURE;
            while let [kind, a, b, c, d, tail @ ..] = rest {
                let (payload, next) = tail.split_at(u32::from_be_bytes([*a, *b, *c, *d]) as usize);
                if *kind == 0 {
                    let (key, annexb) = zeron_proto::device_hub::semu_payload(payload).unwrap();
                    items.push(zeron_proto::device_hub::annexb_to_avcc(annexb, key));
                }
                rest = next;
            }
            items
        }

        #[gpui::test]
        fn an_android_capture_decodes_from_its_sps_and_pps_and_follows_the_encoder_restart(
            cx: &mut TestAppContext,
        ) {
            let runtime = tokio::runtime::Runtime::new().unwrap();
            let _guard = runtime.enter();
            setup(cx);
            let items = android_items();
            let mut wire: Vec<Value> = items[..4].iter().map(|envelopes| item(envelopes)).collect();
            wire.extend([json!("pause"), json!("pause")]);
            wire.extend(items[4..].iter().map(|envelopes| item(envelopes)));
            let (engine, watches) = video_engine(&runtime, wire);
            let (view, cx) = cx.add_window_view(|_, cx| {
                DeviceStreamView::new(
                    engine,
                    "local".into(),
                    "emulator-5554".into(),
                    DevicePlatform::Android,
                    cx,
                )
            });
            wait(cx, |cx| shown(&view, cx).is_some());
            assert_eq!(
                shown(&view, cx),
                Some((DeviceVideoCodec::H264, 576, 1280, true))
            );
            wait(cx, |cx| {
                shown(&view, cx).is_some_and(|shown| shown.1 == 1280)
            });
            assert_eq!(
                shown(&view, cx),
                Some((DeviceVideoCodec::H264, 1280, 576, true)),
                "the landscape session arrives already rotated"
            );
            let frame = view.read_with(cx, |view, _| view.frame_box().unwrap());
            assert!(frame.size.width > frame.size.height);
            assert_eq!(codecs(&watches), [json!("h264")]);
            assert!(view.read_with(cx, |view, _| view.video_error.is_none()));
        }

        #[gpui::test]
        fn an_android_stream_that_cannot_be_decoded_says_so_instead_of_asking_for_mjpeg(
            cx: &mut TestAppContext,
        ) {
            let runtime = tokio::runtime::Runtime::new().unwrap();
            let _guard = runtime.enter();
            setup(cx);
            let keyframe = android_items()[0][1].clone();
            let (engine, watches) = video_engine(&runtime, vec![item(&[keyframe])]);
            let (view, cx) = cx.add_window_view(|_, cx| {
                DeviceStreamView::new(
                    engine,
                    "local".into(),
                    "emulator-5554".into(),
                    DevicePlatform::Android,
                    cx,
                )
            });
            wait(cx, |cx| {
                view.read_with(cx, |view, _| view.video_error.is_some())
            });
            assert_eq!(
                view.read_with(cx, |view, _| view.video_error.clone())
                    .as_deref(),
                Some(
                    "This Mac cannot decode the Android stream: video arrived without an H.264 description."
                )
            );
            assert!(cx.debug_bounds("device-reconnect").is_some());
            pump(cx, Duration::from_millis(1500));
            assert_eq!(codecs(&watches), [json!("h264")], "no MJPEG request");
        }

        #[gpui::test]
        fn a_backlog_that_outlasts_the_window_falls_back_to_mjpeg(cx: &mut TestAppContext) {
            let runtime = tokio::runtime::Runtime::new().unwrap();
            let _guard = runtime.enter();
            setup(cx);
            let (engine, watches) = backlog_engine(&runtime, 14);
            let (view, cx) = cx.add_window_view(|_, cx| {
                DeviceStreamView::new(
                    engine,
                    "local".into(),
                    "SIM".into(),
                    DevicePlatform::Ios,
                    cx,
                )
            });
            pump(cx, Duration::from_millis(4500));
            wait(cx, |_| codecs(&watches).len() == 2);
            wait(cx, |cx| {
                shown(&view, cx).is_some_and(|shown| shown.0 == DeviceVideoCodec::Mjpeg)
            });
            assert_eq!(codecs(&watches), [json!("h264"), json!("mjpeg")]);
        }
    }
}
