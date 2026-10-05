use std::cell::Cell;
use std::collections::{HashSet, VecDeque};
use std::rc::Rc;
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
    DeviceButton, DeviceFrame, DeviceInput, DeviceOrientation, DeviceScreenConfig,
    DeviceStreamStatus, StreamLinkStatus, TouchPhase,
};
use zeron_rpc::methods;

use super::geometry::{self, KeyPress};
use super::loading::{self, LoadingStage, LoadingView};
use crate::state::EngineHandle;
use crate::theme::Theme;

const RESUBSCRIBE_DELAY: Duration = Duration::from_secs(1);
const FPS_LOG_INTERVAL: Duration = Duration::from_secs(5);
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

struct DecodedFrame {
    image: Arc<RenderImage>,
    width: u32,
    height: u32,
    rotation: DeviceOrientation,
}

#[derive(Default)]
struct FpsCounter {
    window_start: Option<Instant>,
    frames: u32,
    shown: f32,
    logged_at: Option<Instant>,
}

impl FpsCounter {
    fn tick(&mut self, device_id: &str) {
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
    device_id: String,
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
        let params = serde_json::json!({ "deviceId": device_id, "input": input });
        if let Err(error) = engine
            .client()
            .call(methods::SEND_HUB_DEVICE_INPUT, params)
            .await
        {
            tracing::debug!(%error, "device input not delivered");
        }
    }
}

fn decode(item: serde_json::Value, rotation: DeviceOrientation) -> Option<DecodedFrame> {
    let frame: DeviceFrame = serde_json::from_value(item).ok()?;
    let jpeg = base64::engine::general_purpose::STANDARD
        .decode(frame.jpeg)
        .ok()?;
    let mut pixels =
        geometry::rotate_for_display(image::load_from_memory(&jpeg).ok()?.to_rgba8(), rotation);
    for pixel in pixels.pixels_mut() {
        pixel.0.swap(0, 2);
    }
    let (width, height) = pixels.dimensions();
    (width > 0 && height > 0).then(|| DecodedFrame {
        image: Arc::new(RenderImage::new([image::Frame::new(pixels)])),
        width,
        height,
        rotation,
    })
}

async fn drain_frames(
    executor: gpui::BackgroundExecutor,
    engine: EngineHandle,
    device_id: String,
    slot: Arc<Mutex<Option<DecodedFrame>>>,
    nudge: mpsc::UnboundedSender<()>,
    rotation: Arc<Mutex<DeviceOrientation>>,
) {
    loop {
        let params = serde_json::json!({ "deviceId": device_id });
        if let Ok(mut frames) = engine
            .client()
            .subscribe_scoped(methods::WATCH_HUB_DEVICE_FRAMES, params)
            .await
        {
            while let Some(mut item) = frames.recv().await {
                while let Some(Some(newer)) = futures::FutureExt::now_or_never(frames.recv()) {
                    item = newer;
                }
                let rotation = *rotation.lock().unwrap_or_else(PoisonError::into_inner);
                let Some(frame) = decode(item, rotation) else {
                    continue;
                };
                *slot.lock().unwrap_or_else(PoisonError::into_inner) = Some(frame);
                if nudge.unbounded_send(()).is_err() {
                    return;
                }
            }
        }
        if nudge.is_closed() {
            return;
        }
        executor.timer(RESUBSCRIBE_DELAY).await;
    }
}

pub struct DeviceStreamView {
    engine: EngineHandle,
    device_id: String,
    label: (SharedString, SharedString),
    image: Option<DecodedFrame>,
    painted: bool,
    retired: Vec<Arc<RenderImage>>,
    slot: Arc<Mutex<Option<DecodedFrame>>>,
    nudge: mpsc::UnboundedSender<()>,
    rotation: Arc<Mutex<DeviceOrientation>>,
    screen: Option<DeviceScreenConfig>,
    link: DeviceStreamStatus,
    awaiting_frame: bool,
    video_error: Option<SharedString>,
    wait: u64,
    frame_box: Rc<Cell<Option<Bounds<Pixels>>>>,
    pressed: Option<(f32, f32)>,
    held_keys: HashSet<u16>,
    focus: FocusHandle,
    focus_watch: Option<Subscription>,
    inputs: mpsc::UnboundedSender<DeviceInput>,
    fps: Option<FpsCounter>,
    debug_tap: Option<(f32, f32)>,
    rail_inset: f32,
    pub(super) corner_radius: f32,
    drain: Task<()>,
    timer: Task<()>,
    _tasks: [Task<()>; 2],
}

fn debug_tap_from_env() -> Option<(f32, f32)> {
    let value = std::env::var("ZERON_DEVICE_TAP").ok()?;
    let (x, y) = value.split_once(',')?;
    Some((x.trim().parse().ok()?, y.trim().parse().ok()?))
}

impl DeviceStreamView {
    pub fn new(engine: EngineHandle, device_id: String, cx: &mut Context<Self>) -> Self {
        let (nudge, mut nudged) = mpsc::unbounded::<()>();
        let (inputs, queued) = mpsc::unbounded::<DeviceInput>();
        let sender = cx.background_spawn(send_inputs(engine.clone(), device_id.clone(), queued));
        cx.on_release(|view: &mut Self, cx| {
            let held = view.release(true);
            if !held.is_empty() {
                let (engine, device_id) = (view.engine.clone(), view.device_id.clone());
                cx.background_spawn(async move {
                    for input in held {
                        let params = serde_json::json!({ "deviceId": device_id, "input": input });
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
            let current = view.image.take().map(|frame| frame.image);
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
        let mut view = Self {
            engine,
            label: (
                SharedString::from(device_id.clone()),
                SharedString::default(),
            ),
            device_id,
            image: None,
            painted: false,
            retired: Vec::new(),
            slot: Arc::default(),
            nudge,
            rotation: Arc::new(Mutex::new(DeviceOrientation::Portrait)),
            screen: None,
            link: DeviceStreamStatus::default(),
            awaiting_frame: true,
            video_error: None,
            wait: 0,
            frame_box: Rc::default(),
            pressed: None,
            held_keys: HashSet::new(),
            focus: cx.focus_handle(),
            focus_watch: None,
            inputs,
            fps: (std::env::var("ZERON_DEVICE_FPS").as_deref() == Ok("1"))
                .then(FpsCounter::default),
            debug_tap: debug_tap_from_env(),
            rail_inset: geometry::RAIL_WIDTH,
            corner_radius: 0.0,
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
        self.drain = cx.background_spawn(drain_frames(
            cx.background_executor().clone(),
            self.engine.clone(),
            self.device_id.clone(),
            self.slot.clone(),
            self.nudge.clone(),
            self.rotation.clone(),
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
            *self.rotation.lock().unwrap_or_else(PoisonError::into_inner) =
                geometry::display_rotation(screen.as_ref());
            self.screen = screen;
            cx.notify();
        }
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
        let stalled =
            self.link.video == StreamLinkStatus::Live && link.video != StreamLinkStatus::Live;
        self.link = link;
        if stalled && self.video_error.is_none() {
            self.await_frame(cx);
        }
        cx.notify();
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
        if touch && let Some((x, y)) = self.pressed.take() {
            inputs.push(DeviceInput::Touch {
                phase: TouchPhase::End,
                x,
                y,
            });
        }
        inputs
    }

    fn release_held(&mut self, touch: bool, cx: &mut Context<Self>) {
        for input in self.release(touch) {
            self.send(input);
        }
        cx.notify();
    }

    fn take_frame(&mut self, cx: &mut Context<Self>) {
        let Some(frame) = self
            .slot
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take()
        else {
            return;
        };
        self.awaiting_frame = false;
        if let Some(old) = self.image.replace(frame)
            && std::mem::take(&mut self.painted)
        {
            self.retired.push(old.image);
        }
        if let Some(fps) = &mut self.fps {
            fps.tick(&self.device_id);
        }
        if let Some(frame) = self.frame_box.get()
            && let Some((x, y)) = self.debug_tap.take()
        {
            let position = geometry::position_at(frame, x, y);
            tracing::info!(device = %self.device_id, x, y, "ZERON_DEVICE_TAP: synthetic tap");
            self.pointer(TouchPhase::Begin, position);
            self.pointer(TouchPhase::End, position);
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
            _ => geometry::IOS_PLACEHOLDER_ASPECT,
        }
    }

    fn pointer(&mut self, phase: TouchPhase, position: Point<Pixels>) {
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
        let image = self.image.as_ref().map(|frame| frame.image.clone());
        let aspect = self.aspect();
        let frame_box = self.frame_box.clone();
        let fps = self.fps.as_ref().map(|fps| fps.shown);
        let rail_inset = self.rail_inset;
        let corners = gpui::Corners::all(px(self.corner_radius));
        let dragging = self.pressed.is_some().then(|| cx.entity().downgrade());
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
                for input in geometry::command_r() {
                    this.send(input);
                }
            }))
            .child(
                gpui::canvas(
                    |_, _, _| (),
                    move |bounds, _, window, _| {
                        let frame = geometry::centered_frame(bounds, aspect, rail_inset);
                        frame_box.set(Some(frame));
                        match image {
                            Some(image) => {
                                let _ = window.paint_image(frame, corners, image, 0, false);
                            }
                            None => window.paint_quad(
                                gpui::fill(frame, crate::theme::ink(0.04)).corner_radii(corners),
                            ),
                        }
                        if let Some(view) = dragging {
                            window.on_mouse_event(move |event: &MouseMoveEvent, phase, _, cx| {
                                if phase == gpui::DispatchPhase::Bubble {
                                    view.update(cx, |view, _| {
                                        view.pointer(TouchPhase::Move, event.position)
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
                        .child(SharedString::from(format!("{fps:.0} fps"))),
                )
            })
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, event: &gpui::MouseDownEvent, window, cx| {
                    window.focus(&this.focus, cx);
                    this.pointer(TouchPhase::Begin, event.position);
                    cx.stop_propagation();
                    cx.notify();
                }),
            )
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, event: &gpui::MouseUpEvent, _, cx| {
                    this.pointer(TouchPhase::End, event.position);
                    cx.notify();
                }),
            )
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(|this, event: &gpui::MouseUpEvent, _, cx| {
                    this.pointer(TouchPhase::End, event.position);
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
        let image = image::RgbImage::from_pixel(18, 39, image::Rgb([40, 90, 200]));
        let mut bytes = Vec::new();
        image::codecs::jpeg::JpegEncoder::new(&mut bytes)
            .encode_image(&image)
            .unwrap();
        base64::engine::general_purpose::STANDARD.encode(bytes)
    }

    fn fake_engine(runtime: &tokio::runtime::Runtime) -> (EngineHandle, Inputs) {
        let (engine, inputs, _) = fake_engine_with(runtime, true);
        (engine, inputs)
    }

    fn fake_engine_with(
        runtime: &tokio::runtime::Runtime,
        frames_flow: bool,
    ) -> (EngineHandle, Inputs, Arc<AtomicUsize>) {
        let (out, mut frames) = tokio::sync::mpsc::channel::<String>(64);
        let (replies, inbound) = tokio::sync::mpsc::channel::<String>(64);
        let inputs: Inputs = Arc::default();
        let seen = inputs.clone();
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
        let view = cx.new(|cx| DeviceStreamView::new(engine, "SIM".into(), cx));
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
        let (view, cx) =
            cx.add_window_view(|_, cx| DeviceStreamView::new(engine, "SIM".into(), cx));
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
        runtime.spawn(send_inputs(engine, "SIM".into(), queued));
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
        let view = cx.new(|cx| DeviceStreamView::new(engine, "SIM".into(), cx));
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
        let (view, cx) =
            cx.add_window_view(|_, cx| DeviceStreamView::new(engine, "SIM".into(), cx));
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
        setup(cx);
        let (engine, inputs) = fake_engine(runtime);
        let view = cx.new(|cx| DeviceStreamView::new(engine, "SIM".into(), cx));
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
        let (view, cx) =
            cx.add_window_view(|_, cx| DeviceStreamView::new(engine, "SIM".into(), cx));
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
        let (view, cx) =
            cx.add_window_view(|_, cx| DeviceStreamView::new(engine, "SIM".into(), cx));
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
}
