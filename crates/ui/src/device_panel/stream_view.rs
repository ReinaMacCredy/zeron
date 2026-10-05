use std::cell::Cell;
use std::collections::VecDeque;
use std::rc::Rc;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use base64::Engine as _;
use futures::StreamExt;
use futures::channel::mpsc;
use gpui::{
    Bounds, Context, MouseButton, Pixels, Point, RenderImage, SharedString, Task, Window, div,
    prelude::*, px,
};
use zeron_proto::device_hub::{DeviceFrame, DeviceInput, DeviceScreenConfig, TouchPhase};
use zeron_rpc::methods;

use super::geometry;
use crate::state::EngineHandle;
use crate::theme::Theme;

const RESUBSCRIBE_DELAY: Duration = Duration::from_secs(1);
const FPS_LOG_INTERVAL: Duration = Duration::from_secs(5);

struct DecodedFrame {
    image: Arc<RenderImage>,
    width: u32,
    height: u32,
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

fn decode(item: serde_json::Value) -> Option<DecodedFrame> {
    let frame: DeviceFrame = serde_json::from_value(item).ok()?;
    let jpeg = base64::engine::general_purpose::STANDARD
        .decode(frame.jpeg)
        .ok()?;
    let preview = crate::computer::preview::decode_frame(frame.seq, &jpeg)?;
    Some(DecodedFrame {
        image: preview.image,
        width: preview.width,
        height: preview.height,
    })
}

async fn drain_frames(
    executor: gpui::BackgroundExecutor,
    engine: EngineHandle,
    device_id: String,
    slot: Arc<Mutex<Option<DecodedFrame>>>,
    nudge: mpsc::UnboundedSender<()>,
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
                let Some(frame) = decode(item) else {
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
    device_id: String,
    image: Option<DecodedFrame>,
    painted: bool,
    retired: Vec<Arc<RenderImage>>,
    slot: Arc<Mutex<Option<DecodedFrame>>>,
    screen: Option<DeviceScreenConfig>,
    frame_box: Rc<Cell<Option<Bounds<Pixels>>>>,
    pressed: bool,
    inputs: mpsc::UnboundedSender<DeviceInput>,
    fps: Option<FpsCounter>,
    debug_tap: Option<(f32, f32)>,
    _tasks: [Task<()>; 3],
}

fn debug_tap_from_env() -> Option<(f32, f32)> {
    let value = std::env::var("ZERON_DEVICE_TAP").ok()?;
    let (x, y) = value.split_once(',')?;
    Some((x.trim().parse().ok()?, y.trim().parse().ok()?))
}

impl DeviceStreamView {
    pub fn new(engine: EngineHandle, device_id: String, cx: &mut Context<Self>) -> Self {
        let slot: Arc<Mutex<Option<DecodedFrame>>> = Arc::default();
        let (nudge, mut nudged) = mpsc::unbounded::<()>();
        let (inputs, queued) = mpsc::unbounded::<DeviceInput>();
        let drain = cx.background_spawn(drain_frames(
            cx.background_executor().clone(),
            engine.clone(),
            device_id.clone(),
            slot.clone(),
            nudge,
        ));
        let sender = cx.background_spawn(send_inputs(engine, device_id.clone(), queued));
        cx.on_release(|view: &mut Self, cx| {
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
        Self {
            device_id,
            image: None,
            painted: false,
            retired: Vec::new(),
            slot,
            screen: None,
            frame_box: Rc::default(),
            pressed: false,
            inputs,
            fps: (std::env::var("ZERON_DEVICE_FPS").as_deref() == Ok("1"))
                .then(FpsCounter::default),
            debug_tap: debug_tap_from_env(),
            _tasks: [drain, sender, pump],
        }
    }

    pub fn set_screen(&mut self, screen: Option<DeviceScreenConfig>, cx: &mut Context<Self>) {
        if self.screen != screen {
            self.screen = screen;
            cx.notify();
        }
    }

    pub fn has_frame(&self) -> bool {
        self.image.is_some()
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
            TouchPhase::Begin if self.image.is_some() && frame.contains(&position) => {
                self.pressed = true
            }
            TouchPhase::Move if self.pressed => {}
            TouchPhase::End if self.pressed => self.pressed = false,
            _ => return,
        }
        let (x, y) = geometry::normalized(frame, position);
        let (x, y) = geometry::raw_point(self.screen.as_ref(), x, y);
        let _ = self
            .inputs
            .unbounded_send(DeviceInput::Touch { phase, x, y });
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
        self.painted = self.image.is_some();
        let theme = Theme::of(cx).clone();
        let image = self.image.as_ref().map(|frame| frame.image.clone());
        let aspect = self.aspect();
        let frame_box = self.frame_box.clone();
        let fps = self.fps.as_ref().map(|fps| fps.shown);
        div()
            .id("device-stream")
            .size_full()
            .relative()
            .child(
                gpui::canvas(
                    |_, _, _| (),
                    move |bounds, _, window, _| {
                        let frame = geometry::centered_frame(bounds, aspect);
                        frame_box.set(Some(frame));
                        match image {
                            Some(image) => {
                                let _ = window.paint_image(
                                    frame,
                                    gpui::Corners::default(),
                                    image,
                                    0,
                                    false,
                                );
                            }
                            None => window.paint_quad(gpui::fill(frame, crate::theme::ink(0.04))),
                        }
                    },
                )
                .absolute()
                .inset_0(),
            )
            .when(self.image.is_none(), |el| {
                el.child(
                    div()
                        .absolute()
                        .inset_0()
                        .flex()
                        .items_center()
                        .justify_center()
                        .text_size(crate::typography::ui_rems(12.0))
                        .text_color(theme.text_muted)
                        .child("Connecting video..."),
                )
            })
            .when_some(fps, |el, fps| {
                el.child(
                    div()
                        .absolute()
                        .bottom(px(6.0))
                        .right(px(8.0))
                        .text_size(crate::typography::ui_rems(11.0))
                        .text_color(theme.text_muted)
                        .child(SharedString::from(format!("{fps:.0} fps"))),
                )
            })
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, event: &gpui::MouseDownEvent, _, cx| {
                    this.pointer(TouchPhase::Begin, event.position);
                    cx.stop_propagation();
                }),
            )
            .on_mouse_move(cx.listener(|this, event: &gpui::MouseMoveEvent, _, _| {
                this.pointer(TouchPhase::Move, event.position);
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, event: &gpui::MouseUpEvent, _, _| {
                    this.pointer(TouchPhase::End, event.position);
                }),
            )
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(|this, event: &gpui::MouseUpEvent, _, _| {
                    this.pointer(TouchPhase::End, event.position);
                }),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{Modifiers, TestAppContext, VisualTestContext};
    use serde_json::{Value, json};
    use zeron_proto::device_hub::DeviceOrientation;

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
        let (out, mut frames) = tokio::sync::mpsc::channel::<String>(64);
        let (replies, inbound) = tokio::sync::mpsc::channel::<String>(64);
        let inputs: Inputs = Arc::default();
        let seen = inputs.clone();
        runtime.spawn(async move {
            while let Some(frame) = frames.recv().await {
                let frame: Value = serde_json::from_str(&frame).unwrap();
                let reply = match frame["method"].as_str() {
                    Some(methods::WATCH_HUB_DEVICE_FRAMES) => {
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
        (EngineHandle::from_test_client(client), inputs)
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
                *view.slot.lock().unwrap() = decode(item);
                view.take_frame(cx);
            }
            assert!(view.has_frame());
            assert!(view.retired.is_empty(), "{} retired", view.retired.len());
            view.painted = true;
            for seq in 20..40 {
                let item = json!({ "seq": seq, "jpeg": jpeg() });
                *view.slot.lock().unwrap() = decode(item);
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

        inputs.lock().unwrap().clear();
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
        let frame = view.read_with(cx, |view, _| view.frame_box().unwrap());
        let at = geometry::position_at(frame, 0.25, 0.75);
        cx.simulate_mouse_down(at, MouseButton::Left, none);
        cx.simulate_mouse_up(at, MouseButton::Left, none);
        wait(cx, |_| inputs.lock().unwrap().len() >= 2);
        assert_touches(
            &touches(&inputs),
            &[("begin", 0.75, 0.75), ("end", 0.75, 0.75)],
        );
    }
}
