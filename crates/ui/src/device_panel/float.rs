use std::rc::Rc;

use gpui::{
    App, Context, Entity, EventEmitter, MouseButton, Pixels, Point, ResizeEdge, Size, Subscription,
    Window, canvas, div, point, prelude::*, px, size,
};
use serde::{Deserialize, Serialize};
use zeron_proto::device_hub::{DeviceOrientation, DevicePlatform, DeviceScreenConfig};

use super::feed_view;
use super::stream_view::DeviceStreamView;
use crate::icons::{self, icon};
use crate::settings::{self, SavePolicy};
use crate::state::AppState;
use crate::theme::Theme;

const EDGE_GAP: f32 = 12.0;
const CORNER_RADIUS: f32 = 12.0;
const DEFAULT_BOX: f32 = 320.0;
const MIN_WIDTH: f32 = 240.0;
const MIN_HEIGHT: f32 = 150.0;
const HANDLE_INSET: f32 = 8.0;

#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceFloatFrame {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl DeviceFloatFrame {
    fn new(origin: Point<f32>, player: Size<f32>) -> Self {
        Self {
            x: origin.x,
            y: origin.y,
            width: player.width,
            height: player.height,
        }
    }
}

fn source_size(platform: DevicePlatform, screen: Option<&DeviceScreenConfig>) -> Size<f32> {
    match screen.filter(|screen| screen.width > 0 && screen.height > 0) {
        Some(screen) => {
            let long = screen.width.max(screen.height) as f32;
            let short = screen.width.min(screen.height) as f32;
            match screen.orientation {
                DeviceOrientation::LandscapeLeft | DeviceOrientation::LandscapeRight => {
                    size(long, short)
                }
                DeviceOrientation::Portrait | DeviceOrientation::PortraitUpsideDown => {
                    size(short, long)
                }
            }
        }
        None => {
            let aspect = match platform {
                DevicePlatform::Ios => 9.0 / 19.5,
                DevicePlatform::Android => 9.0 / 20.0,
            };
            size(1000.0, 1000.0 / aspect)
        }
    }
}

fn fit_width(desired: f32, source: Size<f32>, max: Size<f32>) -> Size<f32> {
    let aspect = source.width / source.height;
    let width = desired
        .max(MIN_WIDTH)
        .max(MIN_HEIGHT * aspect)
        .min(max.width.max(1.0))
        .min((max.height * aspect).max(1.0));
    size(width.round(), (width / aspect).round())
}

fn clamp_origin(origin: Point<f32>, container: Size<f32>, player: Size<f32>) -> Point<f32> {
    point(
        origin
            .x
            .max(EDGE_GAP)
            .min((container.width - player.width - EDGE_GAP).max(EDGE_GAP)),
        origin
            .y
            .max(EDGE_GAP)
            .min((container.height - player.height - EDGE_GAP).max(EDGE_GAP)),
    )
}

fn resolve_frame(
    stored: Option<DeviceFloatFrame>,
    source: Size<f32>,
    container: Size<f32>,
) -> DeviceFloatFrame {
    let desired = stored.map_or_else(
        || DEFAULT_BOX.min(DEFAULT_BOX * source.width / source.height),
        |frame| frame.width,
    );
    let player = fit_width(
        desired,
        source,
        size(
            container.width - EDGE_GAP * 2.0,
            container.height - EDGE_GAP * 2.0,
        ),
    );
    let anchor = stored.map_or_else(
        || point(container.width - EDGE_GAP - player.width, EDGE_GAP),
        |frame| point(frame.x, frame.y),
    );
    DeviceFloatFrame::new(clamp_origin(anchor, container, player), player)
}

fn moved(start: DeviceFloatFrame, delta: Point<f32>, container: Size<f32>) -> DeviceFloatFrame {
    let player = size(start.width, start.height);
    let origin = clamp_origin(
        point(start.x + delta.x, start.y + delta.y),
        container,
        player,
    );
    DeviceFloatFrame::new(origin, player)
}

fn resized(
    start: DeviceFloatFrame,
    edge: ResizeEdge,
    delta: Point<f32>,
    source: Size<f32>,
    container: Size<f32>,
) -> DeviceFloatFrame {
    use ResizeEdge::*;
    let east = matches!(edge, Right | TopRight | BottomRight);
    let west = matches!(edge, Left | TopLeft | BottomLeft);
    let north = matches!(edge, Top | TopLeft | TopRight);
    let south = matches!(edge, Bottom | BottomLeft | BottomRight);
    let right = start.x + start.width;
    let bottom = start.y + start.height;
    let max = size(
        if west {
            right - EDGE_GAP
        } else if east {
            container.width - EDGE_GAP - start.x
        } else {
            container.width - EDGE_GAP * 2.0
        },
        if north {
            bottom - EDGE_GAP
        } else if south {
            container.height - EDGE_GAP - start.y
        } else {
            container.height - EDGE_GAP * 2.0
        },
    );
    let width = start.width
        + if east {
            delta.x
        } else if west {
            -delta.x
        } else {
            0.0
        };
    let height = start.height
        + if south {
            delta.y
        } else if north {
            -delta.y
        } else {
            0.0
        };
    let aspect = source.width / source.height;
    let desired = match (east || west, north || south) {
        (true, true) => (width + height / aspect) / (1.0 + 1.0 / (aspect * aspect)),
        (true, false) => width,
        _ => height * aspect,
    };
    let player = fit_width(desired, source, max);
    let anchor = point(
        if west { right - player.width } else { start.x },
        if north {
            bottom - player.height
        } else {
            start.y
        },
    );
    DeviceFloatFrame::new(clamp_origin(anchor, container, player), player)
}

pub enum DeviceFloatEvent {
    Close,
}

#[derive(Clone, Copy)]
struct Gesture {
    pointer: Point<Pixels>,
    start: DeviceFloatFrame,
    edge: Option<ResizeEdge>,
}

pub struct DeviceFloat {
    state: Entity<AppState>,
    chat_id: String,
    target: (String, String),
    platform: DevicePlatform,
    on_open: Rc<dyn Fn(&mut Window, &mut App)>,
    view: Option<Entity<DeviceStreamView>>,
    visible: bool,
    container: Option<Size<f32>>,
    stored: Option<DeviceFloatFrame>,
    gesture: Option<Gesture>,
    handle_hovered: bool,
    revision: Option<u64>,
    _state: Subscription,
}

impl EventEmitter<DeviceFloatEvent> for DeviceFloat {}

impl DeviceFloat {
    pub fn new(
        state: Entity<AppState>,
        chat_id: String,
        target: (String, String),
        on_open: impl Fn(&mut Window, &mut App) + 'static,
        cx: &mut Context<Self>,
    ) -> Self {
        let observer = cx.observe(&state, |float, _, cx| float.sync(cx));
        let this = cx.weak_entity();
        cx.defer(move |cx| {
            this.update(cx, |float, cx| float.sync(cx)).ok();
        });
        Self {
            state,
            chat_id,
            target,
            platform: DevicePlatform::Ios,
            on_open: Rc::new(on_open),
            view: None,
            visible: false,
            container: None,
            stored: None,
            gesture: None,
            handle_hovered: false,
            revision: None,
            _state: observer,
        }
    }

    pub fn target(&self) -> &(String, String) {
        &self.target
    }

    pub fn is_visible(&self) -> bool {
        self.visible
    }

    pub fn set_visible(&mut self, visible: bool, cx: &mut Context<Self>) {
        if self.visible == visible {
            return;
        }
        self.visible = visible;
        if visible {
            self.revision = None;
            self.sync(cx);
        } else {
            self.view = None;
            self.gesture = None;
            self.stored = None;
            cx.notify();
        }
    }

    fn sync(&mut self, cx: &mut Context<Self>) {
        let Some(hub) = self.state.read(cx).device_hub.clone() else {
            return;
        };
        if self.revision == Some(hub.revision) {
            return;
        }
        self.revision = Some(hub.revision);
        let (host_id, device_id) = &self.target;
        let Some(session) = hub.session(&self.chat_id, host_id, device_id) else {
            self.view = None;
            cx.emit(DeviceFloatEvent::Close);
            return;
        };
        self.platform = session.platform;
        if self.view.is_none()
            && self.visible
            && let Some(engine) = self.state.read(cx).engine().cloned()
        {
            let id = device_id.clone();
            self.view = Some(cx.new(|cx| {
                DeviceStreamView::new(engine, id, cx)
                    .without_rail()
                    .with_corner_radius(CORNER_RADIUS)
            }));
        }
        if let Some(view) = &self.view {
            feed_view(view, &hub, host_id, device_id, cx);
        }
        cx.notify();
    }

    fn source(&self, cx: &App) -> Size<f32> {
        let screen = self
            .state
            .read(cx)
            .device_hub
            .as_ref()
            .and_then(|hub| hub.screens.get(&self.target.1));
        source_size(self.platform, screen)
    }

    fn frame(&self, cx: &App) -> Option<DeviceFloatFrame> {
        let stored = self.stored.or_else(|| settings::device_float_frame(cx));
        Some(resolve_frame(stored, self.source(cx), self.container?))
    }

    fn begin(&mut self, edge: Option<ResizeEdge>, pointer: Point<Pixels>, cx: &mut Context<Self>) {
        if let Some(start) = self.frame(cx) {
            self.stored = settings::device_float_frame(cx);
            self.gesture = Some(Gesture {
                pointer,
                start,
                edge,
            });
            cx.notify();
        }
    }

    fn track(&mut self, position: Point<Pixels>, pressed: bool, cx: &mut Context<Self>) {
        if !pressed {
            self.end(cx);
            return;
        }
        let (Some(gesture), Some(container)) = (self.gesture, self.container) else {
            return;
        };
        let delta = point(
            f32::from(position.x - gesture.pointer.x),
            f32::from(position.y - gesture.pointer.y),
        );
        let next = match gesture.edge {
            None => {
                let next = moved(gesture.start, delta, container);
                DeviceFloatFrame {
                    x: next.x,
                    y: next.y,
                    ..self.stored.unwrap_or(next)
                }
            }
            Some(edge) => resized(gesture.start, edge, delta, self.source(cx), container),
        };
        if self.stored != Some(next) {
            self.stored = Some(next);
            cx.notify();
        }
    }

    fn end(&mut self, cx: &mut Context<Self>) {
        if self.gesture.take().is_none() {
            return;
        }
        if let Some(frame) = self.stored.take() {
            settings::update(SavePolicy::Debounced, cx, |settings| {
                settings.device_float_frame = Some(frame)
            });
        }
        cx.notify();
    }

    fn render_handle(&self, theme: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        let popup = theme.for_popup();
        let open = self.on_open.clone();
        let button = |id: &'static str, glyph: &'static str, label: &'static str| {
            div()
                .id(id)
                .debug_selector(move || id.into())
                .role(gpui::Role::Button)
                .aria_label(label)
                .size(px(24.0))
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(6.0))
                .cursor_pointer()
                .hover(|s| s.bg(crate::theme::ink(0.08)))
                .tooltip(crate::settings::widgets::text_tooltip(label))
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .child(icon(glyph).size(px(14.0)).text_color(popup.text_muted))
        };
        let expanded = self.handle_hovered || self.gesture.is_some();
        let shown = if expanded { 1.0 } else { 0.0 };
        div()
            .id("device-float-handle")
            .debug_selector(|| "device-float-handle".into())
            .absolute()
            .top(px(HANDLE_INSET))
            .right(px(HANDLE_INSET))
            .h(px(32.0))
            .flex()
            .items_center()
            .gap(px(2.0))
            .p(px(2.0))
            .rounded(px(8.0))
            .border_1()
            .border_color(gpui::transparent_black())
            .when(expanded, |el| {
                el.border_color(popup.border)
                    .bg(popup.surface_dialog)
                    .shadow_lg()
            })
            .cursor_grab()
            .when(self.gesture.is_some(), |el| el.cursor_grabbing())
            .on_hover(cx.listener(|float, hovered: &bool, _, cx| {
                float.handle_hovered = *hovered;
                cx.notify();
            }))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|float, event: &gpui::MouseDownEvent, _, cx| {
                    cx.stop_propagation();
                    float.begin(None, event.position, cx);
                }),
            )
            .child(
                button(
                    "device-float-open",
                    icons::LUCIDE_PANEL_RIGHT,
                    "Open in right panel",
                )
                .opacity(shown)
                .on_click(move |_, window, cx| open(window, cx)),
            )
            .child(
                button(
                    "device-float-close",
                    icons::LUCIDE_X,
                    "Close floating preview",
                )
                .opacity(shown)
                .on_click(cx.listener(|_, _, _, cx| cx.emit(DeviceFloatEvent::Close))),
            )
            .child(
                div()
                    .debug_selector(|| "device-float-grip".into())
                    .self_start()
                    .size(px(12.0))
                    .flex()
                    .justify_end()
                    .child(
                        div()
                            .size(px(8.0))
                            .rounded_full()
                            .bg(gpui::white().opacity(0.25))
                            .opacity(1.0 - shown),
                    ),
            )
    }
}

fn resize_handle(edge: ResizeEdge) -> gpui::Div {
    use ResizeEdge::*;
    let handle = div().absolute();
    match edge {
        Top => handle
            .left_0()
            .right_0()
            .top(px(-4.0))
            .h(px(8.0))
            .cursor_ns_resize(),
        Bottom => handle
            .left_0()
            .right_0()
            .bottom(px(-4.0))
            .h(px(8.0))
            .cursor_ns_resize(),
        Left => handle
            .top_0()
            .bottom_0()
            .left(px(-4.0))
            .w(px(8.0))
            .cursor_ew_resize(),
        Right => handle
            .top_0()
            .bottom_0()
            .right(px(-4.0))
            .w(px(8.0))
            .cursor_ew_resize(),
        TopLeft => handle
            .top(px(-8.0))
            .left(px(-8.0))
            .size(px(16.0))
            .cursor_nwse_resize(),
        TopRight => handle
            .top(px(-8.0))
            .right(px(-8.0))
            .size(px(16.0))
            .cursor_nesw_resize(),
        BottomLeft => handle
            .bottom(px(-8.0))
            .left(px(-8.0))
            .size(px(16.0))
            .cursor_nesw_resize(),
        BottomRight => handle
            .bottom(px(-8.0))
            .right(px(-8.0))
            .size(px(16.0))
            .cursor_nwse_resize(),
    }
}

const RESIZE_EDGES: [(ResizeEdge, &str); 8] = [
    (ResizeEdge::Top, "device-float-resize-top"),
    (ResizeEdge::Bottom, "device-float-resize-bottom"),
    (ResizeEdge::Left, "device-float-resize-left"),
    (ResizeEdge::Right, "device-float-resize-right"),
    (ResizeEdge::TopLeft, "device-float-resize-top-left"),
    (ResizeEdge::TopRight, "device-float-resize-top-right"),
    (ResizeEdge::BottomLeft, "device-float-resize-bottom-left"),
    (ResizeEdge::BottomRight, "device-float-resize-bottom-right"),
];

impl Render for DeviceFloat {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::of(cx).clone();
        let measure = cx.entity().downgrade();
        let follow = cx.entity().downgrade();
        let tracking = self.gesture.is_some();
        let tracker = canvas(
            move |bounds, _, cx| {
                let container = size(f32::from(bounds.size.width), f32::from(bounds.size.height));
                let changed = measure
                    .update(cx, |float, _| {
                        float.container.replace(container) != Some(container)
                    })
                    .unwrap_or(false);
                if changed {
                    let measure = measure.clone();
                    cx.defer(move |cx| {
                        measure.update(cx, |_, cx| cx.notify()).ok();
                    });
                }
            },
            move |_, _, window, _| {
                if !tracking {
                    return;
                }
                let on_move = follow.clone();
                window.on_mouse_event(move |event: &gpui::MouseMoveEvent, phase, _, cx| {
                    if phase == gpui::DispatchPhase::Bubble {
                        let pressed = event.pressed_button == Some(MouseButton::Left);
                        let _ = on_move
                            .update(cx, |float, cx| float.track(event.position, pressed, cx));
                    }
                });
                let on_up = follow.clone();
                window.on_mouse_event(move |_: &gpui::MouseUpEvent, phase, _, cx| {
                    if phase == gpui::DispatchPhase::Bubble {
                        let _ = on_up.update(cx, |float, cx| float.end(cx));
                    }
                });
            },
        )
        .absolute()
        .inset_0();
        let root = div().absolute().inset_0().child(tracker);
        let Some(frame) = self.frame(cx) else {
            return root;
        };
        let handles = RESIZE_EDGES.map(|(edge, id)| {
            resize_handle(edge)
                .id(id)
                .debug_selector(move || id.into())
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |float, event: &gpui::MouseDownEvent, _, cx| {
                        cx.stop_propagation();
                        float.begin(Some(edge), event.position, cx);
                    }),
                )
        });
        root.child(
            div()
                .id("device-float")
                .debug_selector(|| "device-float".into())
                .role(gpui::Role::Group)
                .aria_label("Floating device preview")
                .absolute()
                .left(px(frame.x))
                .top(px(frame.y))
                .w(px(frame.width))
                .h(px(frame.height))
                .occlude()
                .rounded(px(CORNER_RADIUS))
                .bg(gpui::black().opacity(0.9))
                .shadow_lg()
                .child(
                    div()
                        .size_full()
                        .rounded(px(CORNER_RADIUS))
                        .overflow_hidden()
                        .children(self.view.clone()),
                )
                .child(
                    div()
                        .absolute()
                        .inset_0()
                        .rounded(px(CORNER_RADIUS))
                        .border_1()
                        .border_color(theme.border.opacity(0.8)),
                )
                .children(handles)
                .child(self.render_handle(&theme, cx)),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::device_panel::tests::{app_state, hub, wait};
    use gpui::{Modifiers, TestAppContext, VisualTestContext};
    use zeron_proto::device_hub::DeviceSession;

    struct Host(Entity<DeviceFloat>);

    impl Render for Host {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            div().size_full().relative().child(self.0.clone())
        }
    }

    #[gpui::test]
    fn dragging_the_handle_and_an_edge_moves_and_resizes_the_float_and_persists_it(
        cx: &mut TestAppContext,
    ) {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let _guard = runtime.enter();
        let dir = tempfile::tempdir().unwrap();
        let mut state = hub(true);
        state.sessions.push(DeviceSession {
            chat_id: "chat".into(),
            host_id: "local".into(),
            device_id: "SIM".into(),
            ..Default::default()
        });
        state.screens.insert(
            "SIM".into(),
            DeviceScreenConfig {
                width: 600,
                height: 1200,
                ..Default::default()
            },
        );
        let (state, _calls) = app_state(cx, &runtime, state);
        cx.update(|cx| settings::init(settings::UiSettings::default(), dir.path(), cx));
        let opened = std::rc::Rc::new(std::cell::Cell::new(false));
        let on_open = opened.clone();
        let float = cx.new(|cx| {
            DeviceFloat::new(
                state,
                "chat".into(),
                ("local".into(), "SIM".into()),
                move |_, _| on_open.set(true),
                cx,
            )
        });
        float.update(cx, |float, cx| float.set_visible(true, cx));
        let host = float.clone();
        let (_, cx) = cx.add_window_view(|_, _| Host(host));
        cx.simulate_resize(size(px(1000.0), px(800.0)));
        let frame = |cx: &mut VisualTestContext| float.read_with(cx, |float, cx| float.frame(cx));
        wait(cx, |cx| {
            cx.debug_bounds("device-float")
                .is_some_and(|card| card.origin.x == px(748.0))
        });
        let start = frame(cx).unwrap();
        assert_eq!(
            (start.x, start.y, start.width, start.height),
            (748.0, 12.0, 240.0, 480.0)
        );
        assert_eq!(
            float.read_with(cx, |float, cx| float
                .view
                .as_ref()
                .map(|view| view.read(cx).corner_radius)),
            Some(CORNER_RADIUS),
            "the float streams with the frame clipped to the card's corners"
        );

        let drag = |cx: &mut VisualTestContext, selector: &'static str, by: Point<Pixels>| {
            let from = cx.debug_bounds(selector).unwrap().center();
            cx.simulate_mouse_down(from, MouseButton::Left, Modifiers::default());
            cx.simulate_mouse_move(from + by, MouseButton::Left, Modifiers::default());
            cx.simulate_mouse_up(from + by, MouseButton::Left, Modifiers::default());
            cx.run_until_parked();
        };
        drag(cx, "device-float-grip", point(px(-300.0), px(100.0)));
        let moved = frame(cx).unwrap();
        assert_eq!((moved.x, moved.y, moved.width), (448.0, 112.0, 240.0));
        drag(cx, "device-float-resize-right", point(px(60.0), px(0.0)));
        let resized = frame(cx).unwrap();
        assert_eq!(
            (resized.x, resized.y, resized.width, resized.height),
            (448.0, 112.0, 300.0, 600.0)
        );
        assert_eq!(
            cx.update(|_, cx| settings::current(cx).device_float_frame),
            Some(resized),
            "the last gesture is saved"
        );
        let card = cx.debug_bounds("device-float").unwrap();
        assert_eq!(
            (card.origin.x, card.origin.y, card.size.width),
            (px(448.0), px(112.0), px(300.0))
        );

        let closed = std::rc::Rc::new(std::cell::Cell::new(false));
        let seen = closed.clone();
        let _subscription = cx.update(|_, cx| {
            cx.subscribe(&float, move |_, event: &DeviceFloatEvent, _| {
                let DeviceFloatEvent::Close = event;
                seen.set(true);
            })
        });
        cx.simulate_mouse_move(point(px(5.0), px(5.0)), None, Modifiers::default());
        cx.run_until_parked();
        assert!(!float.read_with(cx, |float, _| float.handle_hovered));
        let open = cx
            .debug_bounds("device-float-open")
            .expect("the hidden pill keeps its buttons in the tree")
            .center();
        let close = cx.debug_bounds("device-float-close").unwrap().center();
        let grip = cx.debug_bounds("device-float-grip").unwrap();
        assert!(
            !grip.contains(&open) && !grip.contains(&close),
            "the drag grip never sits on a button"
        );
        cx.simulate_click(open, Modifiers::default());
        cx.run_until_parked();
        assert!(
            opened.get(),
            "Open in right panel reaches the shell callback"
        );
        assert_eq!(frame(cx), Some(resized), "a button press is not a drag");
        cx.simulate_click(close, Modifiers::default());
        cx.run_until_parked();
        assert!(closed.get(), "Close floating preview emits Close");

        cx.simulate_resize(size(px(300.0), px(800.0)));
        wait(cx, |cx| {
            cx.debug_bounds("device-float")
                .is_some_and(|card| card.size.width == px(276.0))
        });
        drag(cx, "device-float-grip", point(px(0.0), px(40.0)));
        let saved = cx.update(|_, cx| settings::current(cx).device_float_frame.unwrap());
        assert_eq!(
            (saved.y, saved.width, saved.height),
            (152.0, 300.0, 600.0),
            "a move in a narrow window keeps the stored size"
        );

        let elsewhere = DeviceFloatFrame {
            x: 20.0,
            y: 30.0,
            width: 260.0,
            height: 520.0,
        };
        cx.simulate_resize(size(px(1000.0), px(800.0)));
        cx.update(|_, cx| {
            settings::update(SavePolicy::Debounced, cx, |settings| {
                settings.device_float_frame = Some(elsewhere)
            })
        });
        wait(cx, |cx| frame(cx).is_some_and(|frame| frame.width == 260.0));
        assert_eq!(
            frame(cx),
            Some(elsewhere),
            "the float follows a frame saved by another float"
        );
    }

    const CONTAINER: Size<f32> = Size {
        width: 1000.0,
        height: 800.0,
    };

    fn portrait() -> Size<f32> {
        source_size(
            DevicePlatform::Ios,
            Some(&DeviceScreenConfig {
                width: 600,
                height: 1200,
                ..Default::default()
            }),
        )
    }

    #[test]
    fn a_fresh_float_fits_the_default_box_in_the_top_right_corner() {
        let frame = resolve_frame(None, portrait(), CONTAINER);
        assert_eq!(
            frame,
            DeviceFloatFrame {
                x: 1000.0 - 12.0 - 240.0,
                y: 12.0,
                width: 240.0,
                height: 480.0,
            },
            "320x320 fits 160 wide, the 240 minimum wins"
        );
        let landscape = source_size(
            DevicePlatform::Ios,
            Some(&DeviceScreenConfig {
                width: 600,
                height: 1200,
                orientation: DeviceOrientation::LandscapeLeft,
                ..Default::default()
            }),
        );
        assert_eq!((landscape.width, landscape.height), (1200.0, 600.0));
        let wide = resolve_frame(None, landscape, CONTAINER);
        assert_eq!((wide.width, wide.height), (320.0, 160.0));
        let placeholder = source_size(DevicePlatform::Ios, None);
        assert!((placeholder.width / placeholder.height - 9.0 / 19.5).abs() < 1e-4);
    }

    #[test]
    fn a_stored_frame_is_clamped_into_the_container_with_the_edge_gap() {
        let stored = DeviceFloatFrame {
            x: 900.0,
            y: -50.0,
            width: 300.0,
            height: 0.0,
        };
        let frame = resolve_frame(Some(stored), portrait(), CONTAINER);
        assert_eq!(
            frame,
            DeviceFloatFrame {
                x: 1000.0 - 12.0 - 300.0,
                y: 12.0,
                width: 300.0,
                height: 600.0,
            }
        );
        let tight = resolve_frame(Some(stored), portrait(), size(400.0, 300.0));
        assert_eq!(
            (tight.width, tight.height),
            (138.0, 276.0),
            "a container shorter than the minimum wins over it"
        );
        assert_eq!((tight.x, tight.y), (400.0 - 12.0 - 138.0, 12.0));
        let dragged = moved(frame, point(-2000.0, 5000.0), CONTAINER);
        assert_eq!((dragged.x, dragged.y), (12.0, 800.0 - 12.0 - 600.0));
        assert_eq!((dragged.width, dragged.height), (300.0, 600.0));
    }

    #[test]
    fn resizing_keeps_the_device_aspect_and_anchors_the_opposite_edge() {
        let start = DeviceFloatFrame {
            x: 400.0,
            y: 100.0,
            width: 240.0,
            height: 480.0,
        };
        let east = resized(
            start,
            ResizeEdge::Right,
            point(60.0, 0.0),
            portrait(),
            CONTAINER,
        );
        assert_eq!(
            east,
            DeviceFloatFrame {
                x: 400.0,
                y: 100.0,
                width: 300.0,
                height: 600.0,
            }
        );
        let north = resized(
            start,
            ResizeEdge::Top,
            point(0.0, -40.0),
            portrait(),
            CONTAINER,
        );
        assert_eq!(
            north,
            DeviceFloatFrame {
                x: 400.0,
                y: 60.0,
                width: 260.0,
                height: 520.0,
            },
            "the bottom edge stays put"
        );
        let west = resized(
            start,
            ResizeEdge::BottomLeft,
            point(-20.0, 40.0),
            portrait(),
            CONTAINER,
        );
        assert_eq!(west.x + west.width, 640.0, "the right edge stays put");
        assert_eq!(west.y, 100.0);
        assert_eq!(west.height, west.width * 2.0);
        assert_eq!(west.width, 260.0);
        let shrunk = resized(
            start,
            ResizeEdge::Right,
            point(-500.0, 0.0),
            portrait(),
            CONTAINER,
        );
        assert_eq!(
            (shrunk.width, shrunk.height),
            (240.0, 480.0),
            "minimum size"
        );
        let grown = resized(
            start,
            ResizeEdge::Bottom,
            point(0.0, 5000.0),
            portrait(),
            CONTAINER,
        );
        assert_eq!(grown.height, 800.0 - 12.0 - 100.0, "stops at the container");
        assert_eq!(grown.width, 344.0);
    }
}
