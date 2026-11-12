#[allow(dead_code)]
pub(crate) mod duo3d;
pub(crate) mod geometry;
mod render;
#[allow(dead_code)]
pub(crate) mod scene3d;

use std::cell::RefCell;
use std::rc::{Rc, Weak};
use std::sync::Arc;
use std::time::{Duration, Instant};

use zeron_proto::device_hub::{
    DeviceOrientation, DevicePlatform, DeviceScreenConfig, HingeCommand, HingePose, TouchPhase,
};
use zgx::image::ImageData;
use zgx_gpu::viewport::{Viewport, ViewportFrame};

use duo3d::{DuoPanel, DuoViewer};
use render::{Draw, Gpu, Job, Picture};
use scene3d::{Scene3d, Wheel, WheelPhase, WheelStep};

const PINCH_IDLE: Duration = Duration::from_millis(180);
const SETTLE_POLL: Duration = Duration::from_millis(50);

pub(crate) fn view_name(key: &str) -> String {
    format!("device3d/{key}")
}

pub(crate) fn hinged(screen: Option<&DeviceScreenConfig>) -> bool {
    screen.is_some_and(|screen| screen.supports_hinge_angle == Some(true))
}

#[derive(Debug, Default, PartialEq)]
struct HingeQueue {
    active: Option<HingeCommand>,
    queued: Option<HingeCommand>,
}

impl HingeQueue {
    fn enqueue(&mut self, command: HingeCommand) -> Option<HingeCommand> {
        if let HingeCommand::Angle(angle) = command
            && !(angle.is_finite() && (0.0..=180.0).contains(&angle))
        {
            return None;
        }
        if self.active.is_some() {
            self.queued = Some(command);
            return None;
        }
        self.active = Some(command);
        self.active
    }

    fn finish(&mut self, ok: bool) -> Option<HingeCommand> {
        self.active = self.queued.take().filter(|_| ok);
        self.active
    }

    fn requested(&self) -> Option<HingeCommand> {
        self.queued.or(self.active)
    }
}

struct State {
    scene: Scene3d,
    epoch: Instant,
    platform: DevicePlatform,
    name: String,
    screen: Option<DeviceScreenConfig>,
    duo_sizes: [Option<(u32, u32)>; 2],
    duo_queued: Option<f64>,
    live: Option<(Arc<ImageData>, DeviceOrientation)>,
    frame: u64,
    kept: [Option<(Arc<ImageData>, DeviceOrientation, u64)>; 2],
    wheel: Wheel,
    wheel_generation: u64,
    pinch_wheel: bool,
    pinching: bool,
    pinch_generation: u64,
    animating: bool,
    drawn: u64,
    hinge: HingeQueue,
}

impl State {
    fn now(&self) -> f64 {
        self.epoch.elapsed().as_secs_f64() * 1000.0
    }

    fn shown_rotation(&self) -> DeviceOrientation {
        self.live
            .as_ref()
            .map_or_else(|| geometry::display_rotation(self.screen.as_ref()), |(_, rotation)| *rotation)
    }

    fn duo_sizes(&self) -> [(u32, u32); 2] {
        [self.duo_sizes[0].unwrap_or(duo3d::COVER_SIZE), self.duo_sizes[1].unwrap_or(duo3d::INNER_SIZE)]
    }

    fn duo_named(&self) -> bool {
        self.platform == DevicePlatform::Ios && self.name.trim().eq_ignore_ascii_case("iPhone Duo")
    }

    fn shown(&self) -> bool {
        self.live.is_some() && !(self.duo_named() && self.screen.is_none())
    }

    fn aspect(&self) -> f64 {
        if let Some((image, _)) = &self.live
            && image.width() > 0
            && image.height() > 0
        {
            return f64::from(image.width()) / f64::from(image.height());
        }
        match &self.screen {
            Some(screen) if screen.width > 0 && screen.height > 0 => f64::from(screen.width) / f64::from(screen.height),
            _ if self.platform == DevicePlatform::Android => 9.0 / 20.0,
            _ => f64::from(geometry::IOS_PLACEHOLDER_ASPECT),
        }
    }

    fn arrange(&mut self, now: f64) {
        if hinged(self.screen.as_ref()) {
            if !self.scene.is_duo() {
                let mut duo = DuoViewer::default();
                let [cover, inner] = self.duo_sizes();
                duo.set_rig(cover, inner);
                duo.set_screen(self.screen.clone(), now);
                duo.set_control(self.duo_queued, false, now);
                self.scene = Scene3d::Duo(duo);
            }
            let [cover, inner] = self.duo_sizes();
            if let Some(duo) = self.scene.duo() {
                duo.set_rig(cover, inner);
            }
        } else {
            if self.scene.is_duo() {
                self.scene = Scene3d::default();
            }
            let rotation = self.shown_rotation();
            let aspect = self.aspect();
            let profile = scene3d::resolve_shape(self.platform, &self.name, aspect.min(1.0 / aspect));
            let orientation = scene3d::body_orientation(rotation, self.screen.as_ref());
            if let Scene3d::Phone(viewer) = &mut self.scene {
                viewer.set_model(profile, aspect, orientation);
            }
        }
    }
}

fn picture(image: &ImageData) -> Picture<'_> {
    Picture { id: image.id(), width: image.width(), height: image.height(), rgba: image.pixels() }
}

struct Painter {
    state: Rc<RefCell<State>>,
    gpu: Option<Gpu>,
}

impl Viewport for Painter {
    fn render(&mut self, frame: &mut ViewportFrame<'_>) {
        let mut state = self.state.borrow_mut();
        if !state.shown() || frame.size.0 == 0 || frame.size.1 == 0 {
            return;
        }
        let now = state.now();
        state.arrange(now);
        state.scene.advance(now, false);
        let scale = f64::from(frame.scale.max(0.01));
        let viewport = (0.0, 0.0, f64::from(frame.size.0) / scale, f64::from(frame.size.1) / scale);
        let gpu = self.gpu.get_or_insert_with(|| Gpu::new(frame.device, frame.queue, frame.format));
        let state = &mut *state;
        let Some((live, rotation)) = state.live.clone() else { return };
        match &mut state.scene {
            Scene3d::Phone(viewer) => {
                let (Some(camera), Some(turn), Some(model)) = (viewer.fit(viewport), viewer.rotation(), viewer.model()) else {
                    return;
                };
                let (mvp, rotation) = camera.matrices(turn);
                let draws = [Draw { range: 0..model.vertices.len(), mvp, rotation }];
                gpu.render(
                    frame,
                    &Job { live: picture(&live), mesh: (model.id, &model.vertices), draws: &draws, live_panel: 1, live_rotation: 0, retained: None },
                );
            }
            Scene3d::Duo(duo) => {
                duo.fit(viewport);
                let (Some(rig), Some(matrices)) = (duo.rig(), duo.matrices()) else { return };
                let raw = duo3d::duo_raw_size((live.width(), live.height()), rotation);
                let active = duo.active_panel();
                let other = match active {
                    DuoPanel::Cover => DuoPanel::Inner,
                    DuoPanel::Inner => DuoPanel::Cover,
                };
                let slot = |panel: DuoPanel| usize::from(panel == DuoPanel::Inner);
                let live_panel = [active, other].into_iter().find(|&panel| duo3d::duo_frame_matches(raw, rig.sizes[slot(panel)]));
                let draws = matrices.map(|(range, mvp, rotation)| Draw { range, mvp, rotation });
                let dim = if live_panel == Some(other) { active } else { other };
                let retained = state.kept[slot(dim)]
                    .as_ref()
                    .filter(|(image, turned, _)| {
                        duo3d::duo_frame_matches(duo3d::duo_raw_size((image.width(), image.height()), *turned), rig.sizes[slot(dim)])
                    })
                    .map(|(image, turned, _)| (picture(image), dim as u8, duo3d::quarter_turns(*turned)));
                gpu.render(
                    frame,
                    &Job {
                        live: picture(&live),
                        mesh: (rig.id, &rig.vertices),
                        draws: &draws,
                        live_panel: live_panel.map_or(0, |panel| panel as u8),
                        live_rotation: duo3d::quarter_turns(rotation),
                        retained,
                    },
                );
                if live_panel == Some(active) {
                    duo.frame_ready(duo3d::duo_display_key(duo.screen()));
                    state.kept[slot(active)] = Some((live.clone(), rotation, state.frame));
                }
            }
        }
        state.drawn += 1;
    }

    fn device_changed(&mut self) {
        self.gpu = None;
    }
}

pub(crate) trait Host {
    fn set_hinge(&self, command: HingeCommand, done: Box<dyn FnOnce(Result<(), String>)>);
    fn fold_error(&self, error: Option<String>);
}

pub(crate) struct ThreeD {
    state: Rc<RefCell<State>>,
    view: zgx::gpu_view::GpuView,
    host: Rc<dyn Host>,
}

impl ThreeD {
    pub(crate) fn new(platform: DevicePlatform, name: String, host: Rc<dyn Host>) -> Rc<Self> {
        let state = Rc::new(RefCell::new(State {
            scene: Scene3d::default(),
            epoch: Instant::now(),
            platform,
            name,
            screen: None,
            duo_sizes: [None, None],
            duo_queued: None,
            live: None,
            frame: 0,
            kept: [None, None],
            wheel: Wheel::default(),
            wheel_generation: 0,
            pinch_wheel: false,
            pinching: false,
            pinch_generation: 0,
            animating: false,
            drawn: 0,
            hinge: HingeQueue::default(),
        }));
        let view = zgx_gpu::viewport::view(Painter { state: state.clone(), gpu: None });
        Rc::new(ThreeD { state, view, host })
    }

    pub(crate) fn view(&self) -> zgx::gpu_view::GpuView {
        self.view.clone()
    }

    #[cfg(test)]
    pub(crate) fn drawn(&self) -> u64 {
        self.state.borrow().drawn
    }

    pub(crate) fn set_device(&self, platform: DevicePlatform, name: String) {
        let mut state = self.state.borrow_mut();
        if state.platform != platform || state.name != name {
            state.platform = platform;
            state.name = name;
            drop(state);
            self.view.invalidate();
        }
    }

    pub(crate) fn show(&self, image: Arc<ImageData>, rotation: DeviceOrientation) {
        let mut state = self.state.borrow_mut();
        state.live = Some((image, rotation));
        state.frame += 1;
        drop(state);
        self.view.invalidate();
    }

    pub(crate) fn set_screen(self: &Rc<Self>, screen: Option<DeviceScreenConfig>) -> Vec<(TouchPhase, (f32, f32))> {
        let mut touches = Vec::new();
        {
            let mut state = self.state.borrow_mut();
            if state.screen == screen {
                return touches;
            }
            let id = |screen: &Option<DeviceScreenConfig>| screen.as_ref().and_then(|s| s.screen_id);
            let now = state.now();
            // A fold hands HID to the other display; a press must not continue there.
            if state.screen.is_some() && id(&state.screen) != id(&screen) {
                let shown = state.shown_rotation();
                touches = state.scene.lift(now, shown);
            }
            if let Some(screen) = screen.as_ref().filter(|s| hinged(Some(s))) {
                let size = Some((screen.width, screen.height));
                match DuoPanel::of(screen.screen_id) {
                    DuoPanel::Cover => state.duo_sizes[0] = size,
                    DuoPanel::Inner => state.duo_sizes[1] = size,
                }
            }
            state.screen = screen;
            let [cover, inner] = state.duo_sizes();
            let screen = state.screen.clone();
            if let Some(duo) = state.scene.duo() {
                duo.set_rig(cover, inner);
                duo.set_screen(screen, now);
            }
        }
        self.changed();
        touches
    }

    pub(crate) fn pointer(self: &Rc<Self>, phase: TouchPhase, x: f64, y: f64, orbit: bool) -> Vec<(TouchPhase, (f32, f32))> {
        let touches = {
            let mut state = self.state.borrow_mut();
            let (now, shown) = (state.now(), state.shown_rotation());
            match phase {
                TouchPhase::Begin if state.live.is_some() => state.scene.press(x, y, orbit, now, shown),
                TouchPhase::Begin => return Vec::new(),
                TouchPhase::Move => state.scene.drag(x, y, now, shown),
                TouchPhase::End => {
                    let mut touches = state.scene.drag(x, y, now, shown);
                    touches.extend(state.scene.lift(now, shown));
                    touches
                }
            }
        };
        self.changed();
        touches
    }

    // Scripts hear no wheel or pinch phases, so a wheel or a pinch ends when it goes quiet.
    pub(crate) fn wheel(self: &Rc<Self>, dx: f64, dy: f64, x: f64, y: f64, control: bool) -> bool {
        let mut state = self.state.borrow_mut();
        if state.live.is_none() {
            return false;
        }
        let now = state.now();
        if control && state.scene.is_duo() {
            state.wheel_generation += 1;
            state.scene.end_wheel(now);
            let begin = !state.pinch_wheel;
            state.pinch_wheel = true;
            if let Some(duo) = state.scene.duo() {
                if begin {
                    duo.pinch_begin(x, y, now);
                }
                duo.pinch_move(-dy / 100.0, now);
            }
            state.pinch_generation += 1;
            let generation = state.pinch_generation;
            drop(state);
            self.after(PINCH_IDLE, move |me| {
                let mut state = me.state.borrow_mut();
                if state.pinch_generation == generation && std::mem::take(&mut state.pinch_wheel) {
                    let now = state.now();
                    if let Some(duo) = state.scene.duo() {
                        duo.pinch_end(now);
                    }
                }
            });
            self.changed();
            return true;
        }
        if std::mem::take(&mut state.pinch_wheel)
            && let Some(duo) = state.scene.duo()
        {
            duo.pinch_end(now);
        }
        let step = state.wheel.step(WheelPhase::Moved, false);
        let taken = match step {
            WheelStep::Ignore => false,
            WheelStep::End => {
                state.scene.end_wheel(now);
                true
            }
            WheelStep::Orbit | WheelStep::Wheel => {
                let Some((_, _, width, height)) = state.scene.viewport() else { return true };
                match scene3d::wheel_orbit(dx, dy, false, width, height) {
                    Some((x, y)) if state.scene.wheel(x, y, now) => {
                        state.wheel_generation += 1;
                        let generation = state.wheel_generation;
                        drop(state);
                        self.after(Duration::from_millis(scene3d::WHEEL_IDLE_MS), move |me| {
                            let mut state = me.state.borrow_mut();
                            if state.wheel_generation == generation {
                                let now = state.now();
                                state.scene.end_wheel(now);
                            }
                        });
                        self.changed();
                        return true;
                    }
                    _ => true,
                }
            }
        };
        drop(state);
        self.changed();
        taken
    }

    pub(crate) fn pinch(self: &Rc<Self>, scale: f64, x: f64, y: f64) -> bool {
        let mut state = self.state.borrow_mut();
        if state.live.is_none() || !state.scene.is_duo() {
            return false;
        }
        let now = state.now();
        if !state.pinching {
            state.pinching = true;
            state.wheel_generation += 1;
            state.scene.end_wheel(now);
            if let Some(duo) = state.scene.duo() {
                duo.pinch_begin(x, y, now);
            }
        }
        if let (Some(step), Some(duo)) = (duo3d::pinch_log_scale(scale as f32), state.scene.duo()) {
            duo.pinch_move(step, now);
        }
        state.pinch_generation += 1;
        let generation = state.pinch_generation;
        drop(state);
        self.after(PINCH_IDLE, move |me| {
            let mut state = me.state.borrow_mut();
            if state.pinch_generation == generation && std::mem::take(&mut state.pinching) {
                let now = state.now();
                if let Some(duo) = state.scene.duo() {
                    duo.pinch_end(now);
                }
            }
        });
        self.changed();
        true
    }

    pub(crate) fn restore(self: &Rc<Self>) {
        {
            let mut state = self.state.borrow_mut();
            let now = state.now();
            state.scene.restore(now);
        }
        self.changed();
    }

    pub(crate) fn pose(self: &Rc<Self>, pose: HingePose) {
        self.command(HingeCommand::Pose(pose));
    }

    fn command(self: &Rc<Self>, command: HingeCommand) {
        let send = self.state.borrow_mut().hinge.enqueue(command);
        if let Some(command) = send {
            self.host.fold_error(None);
            self.send(command);
        }
        self.control(false, None);
    }

    fn send(self: &Rc<Self>, command: HingeCommand) {
        {
            // The hub moves touches to the new display before its screen config says so.
            let mut state = self.state.borrow_mut();
            let (now, shown) = (state.now(), state.shown_rotation());
            state.scene.lift(now, shown);
        }
        let me = Rc::downgrade(self);
        self.host.set_hinge(
            command,
            Box::new(move |reply| {
                let Some(me) = me.upgrade() else { return };
                if let Err(error) = &reply {
                    me.host.fold_error(Some(error.clone()));
                }
                let next = me.state.borrow_mut().hinge.finish(reply.is_ok());
                if let Some(next) = next {
                    me.send(next);
                }
                me.control(reply.is_err(), reply.is_ok().then_some(command));
            }),
        );
    }

    fn control(self: &Rc<Self>, failed: bool, accepted: Option<HingeCommand>) {
        {
            let mut state = self.state.borrow_mut();
            let queued = match state.hinge.requested() {
                Some(HingeCommand::Angle(angle)) => Some(angle),
                _ => None,
            };
            state.duo_queued = queued;
            let now = state.now();
            if failed {
                state.pinch_wheel = false;
                state.pinching = false;
            }
            if let Some(duo) = state.scene.duo() {
                duo.set_control(queued, failed, now);
                if let Some(HingeCommand::Orientation(orientation)) = accepted {
                    duo.orientation_accepted(orientation);
                }
            }
        }
        self.changed();
    }

    fn changed(self: &Rc<Self>) {
        let requests = self.state.borrow_mut().scene.take_requests();
        for request in requests {
            self.command(request);
        }
        self.view.invalidate();
        let mut state = self.state.borrow_mut();
        if state.animating || !(state.scene.needs_frame() || state.scene.handoff_deadline().is_some()) {
            return;
        }
        state.animating = true;
        drop(state);
        self.view.animate(true);
        let me = Rc::downgrade(self);
        crate::engine::get().spawn(async move {
            loop {
                zgx::timer::sleep(SETTLE_POLL).await;
                let Some(me) = me.upgrade() else { return };
                let requests = me.state.borrow_mut().scene.take_requests();
                for request in requests {
                    me.command(request);
                }
                let mut state = me.state.borrow_mut();
                let now = state.now();
                let waiting = state.scene.handoff_deadline().is_some_and(|at| at > now);
                if state.scene.needs_frame() || waiting {
                    continue;
                }
                state.animating = false;
                drop(state);
                me.view.animate(false);
                me.view.invalidate();
                return;
            }
        });
    }

    fn after(self: &Rc<Self>, wait: Duration, then: impl FnOnce(&Rc<Self>) + 'static) {
        let me: Weak<Self> = Rc::downgrade(self);
        crate::engine::get().spawn(async move {
            zgx::timer::sleep(wait).await;
            if let Some(me) = me.upgrade() {
                then(&me);
                me.changed();
            }
        });
    }
}

pub(crate) fn touch_phase(word: &str) -> Option<TouchPhase> {
    match word {
        "begin" => Some(TouchPhase::Begin),
        "move" => Some(TouchPhase::Move),
        "end" => Some(TouchPhase::End),
        _ => None,
    }
}

pub(crate) fn phase_word(phase: TouchPhase) -> &'static str {
    match phase {
        TouchPhase::Begin => "begin",
        TouchPhase::Move => "move",
        TouchPhase::End => "end",
    }
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::*;
    use zgx_gpu::wgpu;

    fn block_on<F: std::future::Future>(future: F) -> F::Output {
        let mut future = std::pin::pin!(future);
        let mut cx = std::task::Context::from_waker(std::task::Waker::noop());
        loop {
            if let std::task::Poll::Ready(value) = future.as_mut().poll(&mut cx) {
                return value;
            }
            std::thread::yield_now();
        }
    }

    struct NoHub;
    impl Host for NoHub {
        fn set_hinge(&self, _: HingeCommand, done: Box<dyn FnOnce(Result<(), String>)>) {
            done(Ok(()));
        }
        fn fold_error(&self, _: Option<String>) {}
    }

    fn draw(shown: &ThreeD) -> Vec<u8> {
        let instance = wgpu::Instance::default();
        let adapter = block_on(instance.request_adapter(&Default::default())).expect("a GPU");
        let (device, queue) = block_on(adapter.request_device(&Default::default())).expect("a device");
        let (w, h) = (320u32, 600u32);
        let format = wgpu::TextureFormat::Bgra8Unorm;
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: None,
            size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = texture.create_view(&Default::default());
        let mut painter = Painter { state: shown.state.clone(), gpu: None };
        let mut encoder = device.create_command_encoder(&Default::default());
        painter.render(&mut ViewportFrame::new(&device, &queue, &mut encoder, &view, format, (w, h), 1.0));
        let row = w * 4;
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: u64::from(row * h),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        encoder.copy_texture_to_buffer(
            texture.as_image_copy(),
            wgpu::TexelCopyBufferInfo { buffer: &buffer, layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(row), rows_per_image: Some(h) } },
            wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
        );
        queue.submit([encoder.finish()]);
        buffer.slice(..).map_async(wgpu::MapMode::Read, |_| {});
        device.poll(wgpu::PollType::wait_indefinitely()).expect("drawn");
        let bytes = buffer.slice(..).get_mapped_range().to_vec();
        bytes
    }

    fn pixel(bytes: &[u8], x: u32, y: u32) -> [u8; 4] {
        let at = ((y * 320 + x) * 4) as usize;
        [bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]]
    }

    #[test]
    fn the_body_is_drawn_around_the_live_picture_and_a_press_on_it_lands_on_the_screen() {
        let shown = ThreeD::new(DevicePlatform::Ios, "iPhone 17".into(), Rc::new(NoHub));
        let red: Vec<u8> = std::iter::repeat_n([240u8, 20, 20, 255], 100 * 216).flatten().collect();
        shown.show(Arc::new(ImageData::new(100, 216, red).unwrap()), DeviceOrientation::Portrait);
        let bytes = draw(&shown);
        let [b, g, r, a] = pixel(&bytes, 160, 300);
        assert!(r > 200 && g < 60 && b < 60 && a == 255, "the screen shows the picture: {:?}", [b, g, r, a]);
        assert_eq!(pixel(&bytes, 2, 2)[3], 0, "around the body the box's own background shows");
        let body = (0..320).map(|x| pixel(&bytes, x, 300)).filter(|p| p[3] == 255 && !(p[2] > 200 && p[1] < 60)).count();
        assert!(body > 0, "the body's edge beside the screen is drawn");
        let touches = shown.pointer(TouchPhase::Begin, 160.0, 300.0, false);
        let [(phase, (x, y))] = touches[..] else { panic!("one touch: {touches:?}") };
        assert_eq!(phase, TouchPhase::Begin);
        assert!((x - 0.5).abs() < 0.02 && (y - 0.5).abs() < 0.02, "the middle of the screen: {x}, {y}");
        assert!(shown.pointer(TouchPhase::Begin, 2.0, 2.0, false).is_empty(), "held already: a second press is nothing");
    }

    #[test]
    fn a_hinge_request_waits_behind_the_one_sent_and_a_refusal_drops_it() {
        let mut queue = HingeQueue::default();
        assert_eq!(queue.enqueue(HingeCommand::Angle(200.0)), None, "out of range");
        assert_eq!(queue.enqueue(HingeCommand::Angle(90.0)), Some(HingeCommand::Angle(90.0)));
        assert_eq!(queue.enqueue(HingeCommand::Angle(120.0)), None);
        assert_eq!(queue.enqueue(HingeCommand::Angle(150.0)), None);
        assert_eq!(queue.requested(), Some(HingeCommand::Angle(150.0)));
        assert_eq!(queue.finish(true), Some(HingeCommand::Angle(150.0)), "the newest waiting goes next");
        assert_eq!(queue.enqueue(HingeCommand::Pose(HingePose::Open)), None);
        assert_eq!(queue.finish(false), None, "a refusal drops what waited");
        assert_eq!(queue.requested(), None);
    }
}
