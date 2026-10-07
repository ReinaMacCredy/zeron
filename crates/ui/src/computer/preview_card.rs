use gpui::{
    Bounds, Context, Entity, EventEmitter, FontWeight, Hsla, ImageSource, InteractiveElement as _,
    IntoElement, MouseButton, MouseDownEvent, ObjectFit, ParentElement as _, Pixels, Point, Render,
    SharedString, Size, StatefulInteractiveElement as _, Styled as _, StyledImage as _,
    Subscription, Task, Window, div, img, point, prelude::FluentBuilder as _, px,
};
use zeron_proto::computer::{ComputerPreviewPlacement, ComputerPreviewSize, ComputerSettings};
use zeron_proto::view::Indicator;
use zeron_rpc::methods;

use super::preview::{
    self, BadgeTone, ComputerPreviewStore, PreviewPhase, PreviewView, STILL_POLL_INTERVAL,
    StatusBadge,
};
use crate::state::AppState;
use crate::theme::Theme;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CardCaps {
    pub min_width: f32,
    pub max_width: f32,
}

pub fn card_caps(size: ComputerPreviewSize) -> CardCaps {
    match size {
        ComputerPreviewSize::Compact => CardCaps {
            min_width: 240.0,
            max_width: 400.0,
        },
        ComputerPreviewSize::Large => CardCaps {
            min_width: 240.0,
            max_width: 560.0,
        },
    }
}

const SLOT_TOP: f32 = 16.0;
const SLOT_BOTTOM_RESERVE: f32 = 120.0;
const CARD_TOP: f32 = Theme::TITLEBAR_HEIGHT + 12.0;
const CARD_RIGHT: f32 = 16.0;
const FALLBACK_ASPECT: f32 = 16.0 / 10.0;
const FLOAT_MARGIN: f32 = 8.0;
const FLOAT_EXPAND_SCALE: f32 = 1.5;
const INSET_GAP: f32 = 24.0;
const DRAG_THRESHOLD: f32 = 4.0;

pub fn budget_width(caps: CardCaps, slot_width: f32) -> f32 {
    (slot_width - 520.0 - 24.0).min(caps.max_width).max(200.0)
}

pub fn fit_width(caps: CardCaps, slot: Size<Pixels>, aspect: f32) -> f32 {
    let budget = budget_width(caps, f32::from(slot.width));
    let by_height = (f32::from(slot.height) - SLOT_TOP - SLOT_BOTTOM_RESERVE) * aspect;
    budget
        .min(caps.max_width)
        .min(by_height)
        .max(caps.min_width)
}

pub fn float_width(caps: CardCaps, viewport: Size<Pixels>, aspect: f32) -> f32 {
    let margins = FLOAT_MARGIN * 2.0;
    (caps.max_width * FLOAT_EXPAND_SCALE)
        .min(f32::from(viewport.width) - margins)
        .min((f32::from(viewport.height) - margins) * aspect)
        .max(caps.min_width)
}

pub fn clamp_float(origin: Point<f32>, card: Size<f32>, viewport: Size<Pixels>) -> Point<f32> {
    let max_x = (f32::from(viewport.width) - card.width - FLOAT_MARGIN).max(FLOAT_MARGIN);
    let max_y = (f32::from(viewport.height) - card.height - FLOAT_MARGIN).max(FLOAT_MARGIN);
    point(
        origin.x.clamp(FLOAT_MARGIN, max_x),
        origin.y.clamp(FLOAT_MARGIN, max_y),
    )
}

fn view_aspect(view: &PreviewView) -> f32 {
    view.frame
        .as_ref()
        .map(|frame| frame.width as f32 / frame.height as f32)
        .unwrap_or(FALLBACK_ASPECT)
}

pub const SCREEN_RECORDING_REQUIRED: &str = "screen_recording_required";

pub fn body_copy(view: &PreviewView) -> Option<String> {
    if view.frame.is_some() {
        return None;
    }
    match &view.failure {
        Some(failure) if failure.code == SCREEN_RECORDING_REQUIRED => {
            Some("Screen Recording is off, so the preview can't show this window.".into())
        }
        Some(failure) => Some(failure.message.clone()),
        None => view.connecting.then(|| match &view.app {
            Some(app) => format!("Connecting to {app}…"),
            None => "Connecting to the desktop…".into(),
        }),
    }
}

pub fn docked_inset(view: &PreviewView, size: ComputerPreviewSize, slot: Size<Pixels>) -> f32 {
    if !view.visually_open()
        || view.floating.is_some()
        || view.placement == ComputerPreviewPlacement::Rail
    {
        return 0.0;
    }
    let caps = card_caps(size);
    fit_width(caps, slot, view_aspect(view)).min(budget_width(caps, f32::from(slot.width)))
        + INSET_GAP
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PreviewCardEvent {
    OpenSetup,
}

struct Drag {
    offset: Point<f32>,
    start: Point<Pixels>,
    detached: bool,
}

pub struct ComputerPreviewCard {
    state: Entity<AppState>,
    store: Entity<ComputerPreviewStore>,
    chat_id: Option<String>,
    slot: Size<Pixels>,
    viewport: Size<Pixels>,
    card_bounds: Option<Bounds<Pixels>>,
    dock_anchor: Option<Point<Pixels>>,
    drag: Option<Drag>,
    hovered: bool,
    still_poll: Option<(String, Task<()>)>,
    settings_fetch: Option<Task<()>>,
    seen_settings: Option<ComputerSettings>,
    menu_open: bool,
    completion_repaint: Option<Task<()>>,
    _observers: [Subscription; 2],
}

impl ComputerPreviewCard {
    pub fn new(
        state: Entity<AppState>,
        store: Entity<ComputerPreviewStore>,
        cx: &mut Context<Self>,
    ) -> Self {
        let observers = [
            cx.observe(&store, |_, _, cx| cx.notify()),
            cx.observe(&state, |_, _, cx| cx.notify()),
        ];
        Self {
            state,
            store,
            chat_id: None,
            slot: Size::default(),
            viewport: Size::default(),
            card_bounds: None,
            dock_anchor: None,
            drag: None,
            hovered: false,
            still_poll: None,
            settings_fetch: None,
            seen_settings: None,
            menu_open: false,
            completion_repaint: None,
            _observers: observers,
        }
    }

    pub fn set_target(
        &mut self,
        chat_id: Option<String>,
        slot: Size<Pixels>,
        cx: &mut Context<Self>,
    ) {
        if self.chat_id != chat_id || self.slot != slot {
            if self.chat_id != chat_id {
                self.hovered = false;
                self.drag = None;
                self.menu_open = false;
            }
            self.chat_id = chat_id;
            self.slot = slot;
            cx.notify();
        }
    }

    pub fn store(&self) -> &Entity<ComputerPreviewStore> {
        &self.store
    }

    pub fn hide(&mut self, cx: &mut Context<Self>) {
        let Some(chat) = self.chat_id.clone() else {
            return;
        };
        preview::update_store(&self.store, cx, |store| store.hide_for_task(&chat));
    }

    pub fn hide_all_active(&mut self, cx: &mut Context<Self>) {
        preview::update_store(&self.store, cx, |store| store.hide_all_active());
    }

    pub fn move_to_sidebar(&mut self, cx: &mut Context<Self>) {
        self.dock(cx);
        if self.store.read(cx).placement() == ComputerPreviewPlacement::Rail {
            return;
        }
        let Some(engine) = self.state.read(cx).engine().cloned() else {
            return;
        };
        let (store, state) = (self.store.clone(), self.state.clone());
        cx.spawn(async move |_, cx| {
            let Ok(mut settings) = engine
                .client()
                .call_as::<ComputerSettings>(methods::GET_COMPUTER_SETTINGS, serde_json::json!({}))
                .await
            else {
                return;
            };
            settings.computer_preview_placement = ComputerPreviewPlacement::Rail;
            let params = serde_json::to_value(&settings).unwrap_or_default();
            if let Err(error) = engine
                .client()
                .call(methods::SET_COMPUTER_SETTINGS, params)
                .await
            {
                tracing::warn!(target: "desktop_cua", "move the preview to the sidebar: {error}");
                return;
            }
            cx.update(|cx| {
                state.update(cx, |state, cx| {
                    state.computer_settings = Some(settings.clone());
                    cx.notify();
                });
                preview::update_store(&store, cx, |store| store.set_settings(settings));
            });
        })
        .detach();
    }

    fn set_floating(&mut self, origin: Option<Point<f32>>, cx: &mut Context<Self>) {
        let Some(chat) = self.chat_id.clone() else {
            return;
        };
        preview::update_store(&self.store, cx, |store| store.set_floating(&chat, origin));
    }

    pub fn pop_out(&mut self, cx: &mut Context<Self>) {
        if let Some(bounds) = self.card_bounds {
            let card = Size::new(f32::from(bounds.size.width), f32::from(bounds.size.height));
            let origin = point(f32::from(bounds.origin.x), f32::from(bounds.origin.y));
            self.set_floating(Some(clamp_float(origin, card, self.viewport)), cx);
            return;
        }
        let (Some(anchor), Some(chat)) = (self.dock_anchor, self.chat_id.as_deref()) else {
            return;
        };
        let (aspect, caps) = {
            let store = self.store.read(cx);
            (
                view_aspect(&store.view(chat)),
                card_caps(store.preview_size()),
            )
        };
        let width = float_width(caps, self.viewport, aspect);
        let origin = point(f32::from(anchor.x) - width, f32::from(anchor.y));
        let card = Size::new(width, width / aspect);
        self.set_floating(Some(clamp_float(origin, card, self.viewport)), cx);
    }

    pub fn dock(&mut self, cx: &mut Context<Self>) {
        self.drag = None;
        self.set_floating(None, cx);
    }

    fn begin_drag(&mut self, event: &MouseDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        let Some(bounds) = self.card_bounds else {
            return;
        };
        let detached = self
            .chat_id
            .as_ref()
            .is_some_and(|chat| self.store.read(cx).view(chat).floating.is_some());
        self.drag = Some(Drag {
            offset: point(
                f32::from(event.position.x - bounds.origin.x),
                f32::from(event.position.y - bounds.origin.y),
            ),
            start: event.position,
            detached,
        });
        cx.notify();
    }

    fn drag_to(&mut self, position: Point<Pixels>, pressed: bool, cx: &mut Context<Self>) {
        if !pressed {
            self.end_drag(cx);
            return;
        }
        let (Some(drag), Some(bounds)) = (self.drag.as_mut(), self.card_bounds) else {
            return;
        };
        if !drag.detached {
            let moved = position - drag.start;
            if f32::from(moved.x).hypot(f32::from(moved.y)) < DRAG_THRESHOLD {
                return;
            }
            drag.detached = true;
        }
        let origin = point(
            f32::from(position.x) - drag.offset.x,
            f32::from(position.y) - drag.offset.y,
        );
        let card = Size::new(f32::from(bounds.size.width), f32::from(bounds.size.height));
        let origin = clamp_float(origin, card, self.viewport);
        self.set_floating(Some(origin), cx);
    }

    fn end_drag(&mut self, cx: &mut Context<Self>) {
        if self.drag.take().is_some() {
            cx.notify();
        }
    }

    fn working(&self, chat: &str, cx: &Context<Self>) -> bool {
        self.state.read(cx).indicator_for(chat, chrono::Utc::now()) == Indicator::Working
    }

    fn reconcile(&mut self, chat: &str, view: &PreviewView, cx: &mut Context<Self>) {
        if view.phase == Some(PreviewPhase::Armed) {
            let store = self.store.clone();
            let chat = chat.to_owned();
            cx.defer(move |cx| preview::update_store(&store, cx, |store| store.mark_live(&chat)));
        }
        let shared = self.state.read(cx).computer_settings.clone();
        if let Some(settings) = shared.clone().filter(|_| shared != self.seen_settings) {
            self.seen_settings = shared;
            let store = self.store.clone();
            cx.defer(move |cx| {
                preview::update_store(&store, cx, |store| store.set_settings(settings))
            });
        }
        let engine = self.state.read(cx).engine().cloned();
        if self.store.read(cx).settings_stale()
            && self.settings_fetch.is_none()
            && let Some(engine) = engine.clone()
        {
            let store = self.store.clone();
            let fetch = preview::fetch_settings(engine, store, cx);
            self.settings_fetch = Some(cx.spawn(async move |this, cx| {
                fetch.await;
                this.update(cx, |this, _| this.settings_fetch = None).ok();
            }));
        }
        let wanted = view.wants_stills && (view.open || self.working(chat, cx));
        match (&self.still_poll, wanted, engine) {
            (Some((polled, _)), true, _) if polled == chat => {}
            (_, true, Some(engine)) => {
                let store = self.store.clone();
                let polled = chat.to_owned();
                let task = cx.spawn(async move |this, cx| {
                    loop {
                        let frame = match preview::read_still(&engine, &polled).await {
                            Some((seq, bytes)) => cx
                                .background_executor()
                                .spawn(async move { preview::decode_frame(seq, &bytes) })
                                .await
                                .map(Some),
                            None => Some(None),
                        };
                        if this.upgrade().is_none() {
                            return;
                        }
                        if let Some(frame) = frame {
                            cx.update(|cx| {
                                preview::update_store(&store, cx, |store| {
                                    store.accept_still(&polled, frame)
                                })
                            });
                        }
                        cx.background_executor().timer(STILL_POLL_INTERVAL).await;
                    }
                });
                self.still_poll = Some((chat.to_owned(), task));
            }
            (_, false, _) | (_, true, None) => self.still_poll = None,
        }
    }
}

pub(super) fn completion_check(theme: &Theme) -> gpui::Div {
    div()
        .debug_selector(|| "computer-preview-complete".into())
        .absolute()
        .inset_0()
        .flex()
        .items_center()
        .justify_center()
        .child(
            div()
                .size(px(44.0))
                .rounded_full()
                .bg(theme.success.opacity(0.92))
                .shadow_lg()
                .flex()
                .items_center()
                .justify_center()
                .child(
                    crate::icons::icon(crate::icons::CHECK)
                        .size(px(22.0))
                        .text_color(gpui::white()),
                ),
        )
}

fn menu_item(
    id: &'static str,
    label: &'static str,
    on_click: impl Fn(&gpui::ClickEvent, &mut Window, &mut gpui::App) + 'static,
) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id)
        .debug_selector(move || id.into())
        .px(px(10.0))
        .py(px(5.0))
        .rounded(px(6.0))
        .whitespace_nowrap()
        .cursor_pointer()
        .hover(|s| s.bg(gpui::white().opacity(0.14)))
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .on_click(on_click)
        .child(label)
}

impl ComputerPreviewCard {
    fn render_menu(
        &self,
        floating: bool,
        view: &PreviewView,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let close = |this: &mut Self, cx: &mut Context<Self>| {
            this.menu_open = false;
            cx.notify();
        };
        div()
            .absolute()
            .top(px(46.0))
            .right(px(8.0))
            .p(px(4.0))
            .flex()
            .flex_col()
            .rounded(px(9.0))
            .border_1()
            .border_color(gpui::white().opacity(0.15))
            .bg(theme.surface_overlay.opacity(0.97))
            .shadow_lg()
            .text_size(px(12.0))
            .text_color(theme.text)
            .child(menu_item(
                "computer-preview-hide-task",
                "Hide for this task",
                cx.listener(move |this, _, _, cx| {
                    cx.stop_propagation();
                    close(this, cx);
                    this.hide(cx);
                }),
            ))
            .child(menu_item(
                "computer-preview-hide-all",
                "Hide for all active tasks",
                cx.listener(move |this, _, _, cx| {
                    cx.stop_propagation();
                    close(this, cx);
                    this.hide_all_active(cx);
                }),
            ))
            .when(!floating, |menu| {
                menu.child(menu_item(
                    "computer-preview-float",
                    "Move to floating window",
                    cx.listener(move |this, _, _, cx| {
                        cx.stop_propagation();
                        close(this, cx);
                        this.pop_out(cx);
                    }),
                ))
            })
            .when(
                floating || view.placement == ComputerPreviewPlacement::Card,
                |menu| {
                    menu.child(menu_item(
                        "computer-preview-sidebar",
                        "Move to sidebar",
                        cx.listener(move |this, _, _, cx| {
                            cx.stop_propagation();
                            close(this, cx);
                            this.move_to_sidebar(cx);
                        }),
                    ))
                },
            )
            .into_any_element()
    }
}

fn pill(color: Hsla, dot: Hsla, label: String) -> gpui::Div {
    div()
        .flex()
        .items_center()
        .gap(px(6.0))
        .max_w_full()
        .px(px(10.0))
        .py(px(4.0))
        .rounded_full()
        .border_1()
        .border_color(gpui::white().opacity(0.15))
        .bg(gpui::black().opacity(0.55))
        .text_size(px(11.0))
        .font_weight(FontWeight::MEDIUM)
        .text_color(color)
        .child(div().flex_none().size(px(6.0)).rounded_full().bg(dot))
        .child(div().truncate().child(label))
}

fn badge_colors(badge: &StatusBadge, theme: &Theme) -> Hsla {
    match badge.tone {
        BadgeTone::Danger => theme.danger,
        BadgeTone::Warning => theme.warning,
        BadgeTone::Muted => gpui::white().opacity(0.72),
        BadgeTone::Active => theme.success,
    }
}

fn control(
    id: &'static str,
    glyph: &'static str,
    on_click: impl Fn(&gpui::ClickEvent, &mut Window, &mut gpui::App) + 'static,
) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id)
        .debug_selector(move || id.into())
        .size(px(28.0))
        .rounded_full()
        .flex()
        .items_center()
        .justify_center()
        .cursor_pointer()
        .hover(|s| s.bg(gpui::white().opacity(0.2)))
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .on_click(on_click)
        .child(
            crate::icons::icon(glyph)
                .size(px(16.0))
                .text_color(gpui::white()),
        )
}

impl EventEmitter<PreviewCardEvent> for ComputerPreviewCard {}

impl Render for ComputerPreviewCard {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(chat) = self.chat_id.clone() else {
            self.still_poll = None;
            return div().into_any_element();
        };
        let (view, size) = {
            let store = self.store.read(cx);
            (store.view(&chat), store.preview_size())
        };
        self.reconcile(&chat, &view, cx);
        self.viewport = window.viewport_size();
        let railed = view.placement == ComputerPreviewPlacement::Rail && view.floating.is_none();
        if !view.visually_open() || railed {
            self.drag = None;
            self.card_bounds = None;
            if !railed {
                return div().into_any_element();
            }
            let anchor = cx.entity().downgrade();
            return gpui::canvas(
                move |bounds, _, cx| {
                    let _ = anchor.update(cx, |this, _| this.dock_anchor = Some(bounds.origin));
                },
                |_, _, _, _| {},
            )
            .absolute()
            .top(px(CARD_TOP))
            .right(px(CARD_RIGHT))
            .size_0()
            .into_any_element();
        }
        let theme = Theme::of(cx).clone();
        if view.completed && self.completion_repaint.is_none() {
            self.completion_repaint = Some(cx.spawn(async move |this, cx| {
                cx.background_executor()
                    .timer(preview::COMPLETION_FLASH)
                    .await;
                this.update(cx, |this, cx| {
                    this.completion_repaint = None;
                    cx.notify();
                })
                .ok();
            }));
        }
        let aspect = view_aspect(&view);
        let caps = card_caps(size);
        let floating = view.floating.map(|stored| {
            let width = float_width(caps, self.viewport, aspect);
            let origin = clamp_float(stored, Size::new(width, width / aspect), self.viewport);
            (width, origin)
        });
        let width = floating.map_or_else(|| fit_width(caps, self.slot, aspect), |(w, _)| w);
        let height = width / aspect;
        let body = match &view.frame {
            Some(frame) => img(ImageSource::Render(frame.image.clone()))
                .size_full()
                .object_fit(ObjectFit::Contain)
                .into_any_element(),
            None => {
                let set_up = view
                    .failure
                    .as_ref()
                    .is_some_and(|failure| failure.code == SCREEN_RECORDING_REQUIRED);
                div()
                    .size_full()
                    .flex()
                    .flex_col()
                    .items_center()
                    .justify_center()
                    .gap(px(10.0))
                    .px(px(12.0))
                    .text_size(px(12.0))
                    .text_color(theme.text_muted)
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(8.0))
                            .when(view.connecting, |line| {
                                line.child(crate::loaders::mini_mono_spinner(
                                    "computer-preview-connecting",
                                    2.0,
                                    theme.text_muted,
                                    cx.entity_id(),
                                    cx,
                                ))
                            })
                            .child(SharedString::from(body_copy(&view).unwrap_or_default())),
                    )
                    .when(set_up, |body| {
                        body.child(
                            div()
                                .id("computer-preview-set-up")
                                .debug_selector(|| "computer-preview-set-up".into())
                                .px(px(12.0))
                                .py(px(4.0))
                                .rounded_full()
                                .border_1()
                                .border_color(gpui::white().opacity(0.2))
                                .bg(gpui::white().opacity(0.12))
                                .text_color(gpui::white())
                                .cursor_pointer()
                                .hover(|s| s.bg(gpui::white().opacity(0.2)))
                                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                                .on_click(cx.listener(|_, _, _, cx| {
                                    cx.stop_propagation();
                                    cx.emit(PreviewCardEvent::OpenSetup);
                                }))
                                .child("Set up"),
                        )
                    })
                    .into_any_element()
            }
        };
        let badge = view
            .badge
            .clone()
            .filter(|badge| badge.tone != BadgeTone::Active);
        let hovered = self.hovered;
        let dragging = self.drag.is_some();
        let measure = cx.entity().downgrade();
        let follow = cx.entity().downgrade();
        let tracker = gpui::canvas(
            move |bounds, _, cx| {
                let _ = measure.update(cx, |this, _| this.card_bounds = Some(bounds));
            },
            move |_, _, window, _| {
                if !dragging {
                    return;
                }
                let on_move = follow.clone();
                window.on_mouse_event(move |event: &gpui::MouseMoveEvent, phase, _, cx| {
                    if phase != gpui::DispatchPhase::Bubble {
                        return;
                    }
                    let pressed = event.pressed_button == Some(MouseButton::Left);
                    let _ =
                        on_move.update(cx, |this, cx| this.drag_to(event.position, pressed, cx));
                });
                let on_up = follow.clone();
                window.on_mouse_event(move |_: &gpui::MouseUpEvent, phase, _, cx| {
                    if phase == gpui::DispatchPhase::Bubble {
                        let _ = on_up.update(cx, |this, cx| this.end_drag(cx));
                    }
                });
            },
        )
        // Absolute children sit inside the card's 1px border; measure the outer rect.
        .absolute()
        .top(px(-1.0))
        .left(px(-1.0))
        .right(px(-1.0))
        .bottom(px(-1.0));
        let detach = if floating.is_some() {
            control(
                "computer-preview-dock",
                crate::icons::COLLAPSE_ARROWS,
                cx.listener(|this, _, _, cx| {
                    cx.stop_propagation();
                    this.dock(cx);
                }),
            )
        } else {
            control(
                "computer-preview-pop-out",
                crate::icons::EXPAND_ARROWS,
                cx.listener(|this, _, _, cx| {
                    cx.stop_propagation();
                    this.pop_out(cx);
                }),
            )
        };
        let card = div()
            .id("computer-preview-card")
            .debug_selector(|| "computer-preview-card".into())
            .w(px(width))
            .flex()
            .flex_col()
            .overflow_hidden()
            .rounded(px(16.0))
            .border_1()
            .border_color(gpui::white().opacity(0.1))
            .bg(theme.surface_overlay.opacity(0.95))
            .shadow_lg()
            .on_hover(cx.listener(|this, hovered: &bool, _, cx| {
                this.hovered = *hovered;
                cx.notify();
            }))
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(
                div()
                    .id("computer-preview-viewport")
                    .debug_selector(|| "computer-preview-viewport".into())
                    .relative()
                    .w_full()
                    .h(px(height))
                    .overflow_hidden()
                    .bg(gpui::black())
                    .when(floating.is_some(), |viewport| {
                        viewport
                            .cursor_grab()
                            .when(dragging, |el| el.cursor_grabbing())
                    })
                    .on_mouse_down(MouseButton::Left, cx.listener(Self::begin_drag))
                    .child(body)
                    .child(
                        div()
                            .absolute()
                            .inset_0()
                            .border_1()
                            .border_color(gpui::black().opacity(0.45)),
                    )
                    .child(
                        div()
                            .absolute()
                            .top_0()
                            .left_0()
                            .right_0()
                            .h(gpui::relative(0.45))
                            .bg(gpui::linear_gradient(
                                180.0,
                                gpui::linear_color_stop(gpui::white().opacity(0.09), 0.0),
                                gpui::linear_color_stop(gpui::white().opacity(0.0), 1.0),
                            )),
                    )
                    .when_some(badge, |viewport, badge| {
                        let color = badge_colors(&badge, &theme);
                        viewport.child(
                            div()
                                .absolute()
                                .top(px(8.0))
                                .left(px(8.0))
                                .max_w(px(width - 88.0))
                                .child(pill(color, color, badge.label)),
                        )
                    })
                    .when_some(view.status_label.clone(), |viewport, label| {
                        viewport.child(
                            div()
                                .absolute()
                                .bottom(px(8.0))
                                .left(px(8.0))
                                .max_w(px(width - 16.0))
                                .child(pill(gpui::white(), theme.text_muted, label)),
                        )
                    })
                    .when(view.completed, |viewport| {
                        viewport.child(completion_check(&theme))
                    })
                    .when(hovered || dragging || self.menu_open, |viewport| {
                        viewport.child(
                            div()
                                .absolute()
                                .top(px(8.0))
                                .right(px(8.0))
                                .flex()
                                .gap(px(4.0))
                                .p(px(4.0))
                                .rounded_full()
                                .border_1()
                                .border_color(gpui::white().opacity(0.2))
                                .bg(gpui::white().opacity(0.12))
                                .child(control(
                                    "computer-preview-menu",
                                    crate::icons::MORE_HORIZONTAL,
                                    cx.listener(|this, _, _, cx| {
                                        cx.stop_propagation();
                                        this.menu_open = !this.menu_open;
                                        cx.notify();
                                    }),
                                ))
                                .child(detach)
                                .child(control(
                                    "computer-preview-hide",
                                    crate::icons::CLOSE,
                                    cx.listener(|this, _, _, cx| {
                                        cx.stop_propagation();
                                        this.hide(cx);
                                    }),
                                )),
                        )
                    })
                    .when(self.menu_open, |viewport| {
                        viewport.child(self.render_menu(floating.is_some(), &view, &theme, cx))
                    }),
            )
            .child(tracker);
        match floating {
            Some((_, origin)) => gpui::deferred(
                gpui::anchored()
                    .position(point(px(origin.x), px(origin.y)))
                    .child(card),
            )
            .into_any_element(),
            None => card
                .absolute()
                .top(px(CARD_TOP))
                .right(px(CARD_RIGHT))
                .into_any_element(),
        }
    }
}

#[cfg(test)]
mod tests {
    use gpui::{AppContext as _, TestAppContext, VisualTestContext, point, size};
    use zeron_proto::computer::ComputerEvent;
    use zeron_rpc::methods;

    use super::*;
    use crate::computer::preview::PreviewFailure;
    use crate::computer::preview::tests::{driving, image};

    struct Fixture(Entity<ComputerPreviewCard>);

    impl Render for Fixture {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            div()
                .relative()
                .w(px(1200.0))
                .h(px(800.0))
                .child(self.0.clone())
        }
    }

    struct Wire(tokio::sync::mpsc::Receiver<String>);

    impl Wire {
        fn methods(&mut self, cx: &mut VisualTestContext) -> Vec<(String, serde_json::Value)> {
            cx.run_until_parked();
            let mut methods = Vec::new();
            while let Ok(frame) = self.0.try_recv() {
                let frame: serde_json::Value = serde_json::from_str(&frame).unwrap();
                methods.push((
                    frame["method"].as_str().unwrap_or_default().to_owned(),
                    frame["params"].clone(),
                ));
            }
            methods
        }
    }

    fn card_placement() -> ComputerSettings {
        ComputerSettings {
            computer_preview_placement: ComputerPreviewPlacement::Card,
            ..ComputerSettings::default()
        }
    }

    fn mount<'a>(
        cx: &'a mut TestAppContext,
        dir: &std::path::Path,
    ) -> (
        Entity<ComputerPreviewStore>,
        Entity<ComputerPreviewCard>,
        Wire,
        &'a mut VisualTestContext,
    ) {
        cx.update(|cx| {
            gpui_base::init(cx);
            cx.set_global(Theme::dark());
            crate::settings::init(crate::settings::UiSettings::default(), dir, cx);
        });
        let (out, frames) = tokio::sync::mpsc::channel::<String>(64);
        let (replies, inbound) = tokio::sync::mpsc::channel::<String>(64);
        // A closed inbound fails the pending call from a tokio thread.
        std::mem::forget(replies);
        let state = cx.new(|_| AppState::new());
        state.update(cx, |state, _| {
            state.set_test_engine(crate::state::EngineHandle::from_test_client(
                zeron_rpc::RpcClient::new(out, inbound),
            ))
        });
        let store = cx.new(|_| {
            let mut store = ComputerPreviewStore::default();
            store.set_settings(card_placement());
            store
        });
        let card = cx.new(|cx| ComputerPreviewCard::new(state, store.clone(), cx));
        card.update(cx, |card, cx| {
            card.set_target(Some("a".into()), size(px(1200.0), px(800.0)), cx)
        });
        let (_, cx) = cx.add_window_view(|_, _| Fixture(card.clone()));
        cx.simulate_resize(size(px(1200.0), px(800.0)));
        (store, card, Wire(frames), cx)
    }

    fn change(
        store: &Entity<ComputerPreviewStore>,
        cx: &mut VisualTestContext,
        change: impl FnOnce(&mut ComputerPreviewStore) -> bool,
    ) {
        cx.update(|_, cx| preview::update_store(store, cx, change));
        cx.run_until_parked();
    }

    fn phase(
        store: &Entity<ComputerPreviewStore>,
        cx: &mut VisualTestContext,
    ) -> Option<PreviewPhase> {
        cx.update(|_, cx| store.read(cx).view("a").phase)
    }

    #[gpui::test]
    fn the_card_says_connecting_before_the_first_frame(cx: &mut TestAppContext) {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let _guard = runtime.enter();
        let dir = tempfile::tempdir().unwrap();
        let (store, _card, mut wire, cx) = mount(cx, dir.path());
        change(&store, cx, |store| {
            store.set_events_live(true);
            false
        });
        assert!(cx.debug_bounds("computer-preview-card").is_none());
        change(&store, cx, |store| {
            driving(store, "a");
            true
        });
        assert_eq!(
            phase(&store, cx),
            Some(PreviewPhase::Live),
            "rendering the chat views it"
        );
        let connecting = cx
            .debug_bounds("computer-preview-card")
            .expect("a connecting card before the first frame");
        assert_eq!(f32::from(connecting.size.width), 400.0);
        let copy =
            |cx: &mut VisualTestContext| cx.update(|_, cx| body_copy(&store.read(cx).view("a")));
        assert_eq!(copy(cx).as_deref(), Some("Connecting to the desktop…"));
        change(&store, cx, |store| {
            store.apply_event(&ComputerEvent::Action {
                window_id: Some("w1".into()),
                delivery: None,
                action: "computer_type_text".into(),
                ok: true,
                message: Some("typed hunter2".into()),
                thread_id: Some("a".into()),
                restored_window_id: None,
                restore_status: None,
                masked: false,
            })
        });
        assert_eq!(copy(cx).as_deref(), Some("Connecting to Calculator…"));
        let asked: Vec<_> = wire.methods(cx).into_iter().map(|(m, _)| m).collect();
        assert!(asked.contains(&methods::GET_COMPUTER_SETTINGS.to_owned()));
        change(&store, cx, |store| {
            store.accept_tap_frame("a", image(1, 960, 600))
        });
        let compact = cx
            .debug_bounds("computer-preview-card")
            .expect("card painted");
        assert_eq!(f32::from(compact.size.width), 400.0);
        change(&store, cx, |store| {
            store.set_settings(ComputerSettings {
                computer_preview_size: ComputerPreviewSize::Large,
                ..card_placement()
            })
        });
        let large = cx
            .debug_bounds("computer-preview-card")
            .expect("card painted");
        assert_eq!(f32::from(large.size.width), 560.0);
        assert_eq!(f32::from(large.size.height).round(), 352.0);
        change(&store, cx, |store| {
            store.accept_tap_failure(
                "a",
                PreviewFailure {
                    code: "target_unavailable".into(),
                    message: "The selected window is no longer available.".into(),
                },
            )
        });
        assert!(
            cx.debug_bounds("computer-preview-card").is_some(),
            "a closed window keeps the last frame up"
        );
        change(&store, cx, |store| {
            store.set_settings(ComputerSettings {
                auto_open_computer_pane: false,
                ..card_placement()
            })
        });
        assert!(cx.debug_bounds("computer-preview-card").is_none());
    }

    #[gpui::test]
    fn hiding_closes_the_card_and_leaves_the_task_running(cx: &mut TestAppContext) {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let _guard = runtime.enter();
        let dir = tempfile::tempdir().unwrap();
        let (store, _card, mut wire, cx) = mount(cx, dir.path());
        change(&store, cx, |store| {
            store.set_events_live(true);
            driving(store, "a");
            store.accept_tap_frame("a", image(1, 960, 600))
        });
        wire.methods(cx);
        let card = cx
            .debug_bounds("computer-preview-card")
            .expect("card painted");
        assert!(
            cx.debug_bounds("computer-preview-hide").is_none(),
            "controls reveal on hover"
        );
        cx.simulate_mouse_move(card.center(), None, gpui::Modifiers::default());
        cx.run_until_parked();
        let hide = cx
            .debug_bounds("computer-preview-hide")
            .expect("hide on hover");
        cx.simulate_click(hide.center(), gpui::Modifiers::default());
        cx.run_until_parked();
        assert!(cx.debug_bounds("computer-preview-card").is_none());
        assert_eq!(phase(&store, cx), Some(PreviewPhase::HiddenForTask));
        assert!(
            wire.methods(cx).is_empty(),
            "hiding sends nothing to the engine"
        );
        change(&store, cx, |store| {
            store.accept_tap_frame("a", image(2, 960, 600))
        });
        assert!(
            cx.debug_bounds("computer-preview-card").is_none(),
            "frames keep flowing into a hidden card without reopening it"
        );
        change(&store, cx, |store| {
            store.apply_event(&ComputerEvent::OpenPaneRequested {
                thread_id: "a".into(),
            })
        });
        assert_eq!(
            phase(&store, cx),
            Some(PreviewPhase::Live),
            "the next lease re-arms"
        );
    }

    #[test]
    fn float_and_inset_follow_synaras_popover_logic() {
        let compact = card_caps(ComputerPreviewSize::Compact);
        let window = size(px(1200.0), px(800.0));
        assert_eq!(
            float_width(compact, window, 1.6),
            600.0,
            "detaching grows the card 1.5x"
        );
        assert_eq!(
            float_width(compact, size(px(500.0), px(800.0)), 1.6),
            484.0,
            "the viewport bounds a detached card"
        );
        assert_eq!(
            float_width(compact, size(px(1200.0), px(150.0)), 1.6),
            240.0,
            "never below the footprint floor"
        );
        let card = Size::new(600.0, 375.0);
        assert_eq!(
            clamp_float(point(-50.0, 900.0), card, window),
            point(8.0, 417.0)
        );
        assert_eq!(
            clamp_float(point(100.0, 60.0), card, window),
            point(100.0, 60.0)
        );
        assert_eq!(
            clamp_float(point(100.0, 60.0), Size::new(2000.0, 900.0), window),
            point(8.0, 8.0)
        );

        let mut store = ComputerPreviewStore::default();
        store.set_settings(card_placement());
        let slot = size(px(1200.0), px(800.0));
        assert_eq!(
            docked_inset(&store.view("a"), ComputerPreviewSize::Compact, slot),
            0.0,
            "no gutter for a chat that never drove"
        );
        driving(&mut store, "a");
        store.mark_live("a");
        assert_eq!(
            docked_inset(&store.view("a"), ComputerPreviewSize::Compact, slot),
            424.0,
            "a connecting card holds its gutter"
        );
        store.accept_tap_frame("a", image(1, 960, 600));
        assert_eq!(
            docked_inset(&store.view("a"), ComputerPreviewSize::Compact, slot),
            424.0
        );
        assert_eq!(
            docked_inset(&store.view("a"), ComputerPreviewSize::Large, slot),
            584.0
        );
        assert_eq!(
            docked_inset(
                &store.view("a"),
                ComputerPreviewSize::Compact,
                size(px(700.0), px(800.0))
            ),
            224.0,
            "a narrow slot caps the gutter at the budget"
        );
        store.set_floating("a", Some(point(10.0, 10.0)));
        assert_eq!(
            docked_inset(&store.view("a"), ComputerPreviewSize::Compact, slot),
            0.0,
            "a floating card frees the gutter"
        );
        store.set_floating("a", None);
        store.set_settings(ComputerSettings::default());
        assert_eq!(
            docked_inset(&store.view("a"), ComputerPreviewSize::Compact, slot),
            0.0,
            "the rail placement never docks"
        );
    }

    #[gpui::test]
    fn rail_placement_never_docks_but_pops_out(cx: &mut TestAppContext) {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let _guard = runtime.enter();
        let dir = tempfile::tempdir().unwrap();
        let (store, card, _wire, cx) = mount(cx, dir.path());
        change(&store, cx, |store| {
            store.set_settings(ComputerSettings::default());
            store.set_events_live(true);
            driving(store, "a");
            store.accept_tap_frame("a", image(1, 960, 600))
        });
        assert_eq!(
            phase(&store, cx),
            Some(PreviewPhase::Live),
            "the rail still views it"
        );
        assert!(cx.debug_bounds("computer-preview-card").is_none());
        assert!(cx.update(|_, cx| store.read(cx).thumbnail_shown("a")));
        card.update(cx, |card, cx| card.pop_out(cx));
        cx.run_until_parked();
        let floating = cx
            .debug_bounds("computer-preview-card")
            .expect("the thumbnail pops out a floating card");
        assert_eq!(f32::from(floating.size.width), 600.0);
        assert_eq!(
            (f32::from(floating.origin.x), f32::from(floating.origin.y)),
            (1200.0 - CARD_RIGHT - 600.0, CARD_TOP),
            "it opens where the docked card would sit"
        );
        assert!(!cx.update(|_, cx| store.read(cx).thumbnail_shown("a")));
        let dock = reveal(cx, "computer-preview-dock");
        cx.simulate_click(dock.center(), gpui::Modifiers::default());
        cx.run_until_parked();
        assert!(
            cx.debug_bounds("computer-preview-card").is_none(),
            "docking goes back to the rail"
        );
        assert!(cx.update(|_, cx| store.read(cx).thumbnail_shown("a")));
    }

    #[gpui::test]
    fn a_screen_recording_failure_offers_set_up(cx: &mut TestAppContext) {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let _guard = runtime.enter();
        let dir = tempfile::tempdir().unwrap();
        let (store, card, _wire, cx) = mount(cx, dir.path());
        let events = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        let seen = events.clone();
        cx.update(|_, cx| {
            cx.subscribe(&card, move |_, event: &PreviewCardEvent, _| {
                seen.borrow_mut().push(*event)
            })
            .detach()
        });
        change(&store, cx, |store| {
            store.set_events_live(true);
            driving(store, "a");
            store.accept_tap_failure(
                "a",
                PreviewFailure {
                    code: "target_unavailable".into(),
                    message: "The selected window is no longer available.".into(),
                },
            )
        });
        assert!(cx.debug_bounds("computer-preview-card").is_some());
        assert!(cx.debug_bounds("computer-preview-set-up").is_none());
        change(&store, cx, |store| {
            store.accept_tap_failure(
                "a",
                PreviewFailure {
                    code: SCREEN_RECORDING_REQUIRED.into(),
                    message: "screen_recording_required: grant missing".into(),
                },
            )
        });
        assert_eq!(
            cx.update(|_, cx| body_copy(&store.read(cx).view("a")))
                .as_deref(),
            Some("Screen Recording is off, so the preview can't show this window.")
        );
        let set_up = cx
            .debug_bounds("computer-preview-set-up")
            .expect("Set up on a missing grant");
        cx.simulate_click(set_up.center(), gpui::Modifiers::default());
        cx.run_until_parked();
        assert_eq!(*events.borrow(), vec![PreviewCardEvent::OpenSetup]);
    }

    fn live_card(
        store: &Entity<ComputerPreviewStore>,
        cx: &mut VisualTestContext,
    ) -> Bounds<Pixels> {
        change(store, cx, |store| {
            store.set_events_live(true);
            driving(store, "a");
            store.accept_tap_frame("a", image(1, 960, 600))
        });
        cx.debug_bounds("computer-preview-card")
            .expect("card painted")
    }

    fn reveal(cx: &mut VisualTestContext, selector: &'static str) -> Bounds<Pixels> {
        let card = cx
            .debug_bounds("computer-preview-card")
            .expect("card painted");
        cx.simulate_mouse_move(card.center(), None, gpui::Modifiers::default());
        cx.run_until_parked();
        cx.debug_bounds(selector).expect("control on hover")
    }

    #[gpui::test]
    fn the_preview_menu_hides_floats_and_moves_to_the_sidebar(cx: &mut TestAppContext) {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let _guard = runtime.enter();
        let dir = tempfile::tempdir().unwrap();
        let (store, _card, mut wire, cx) = mount(cx, dir.path());
        live_card(&store, cx);
        wire.methods(cx);
        let pick = |cx: &mut VisualTestContext, item: &'static str| {
            let menu = reveal(cx, "computer-preview-menu");
            cx.simulate_click(menu.center(), gpui::Modifiers::default());
            cx.run_until_parked();
            let bounds = cx
                .debug_bounds(item)
                .unwrap_or_else(|| panic!("{item} in the menu"));
            cx.simulate_click(bounds.center(), gpui::Modifiers::default());
            cx.run_until_parked();
        };

        let menu = reveal(cx, "computer-preview-menu");
        cx.simulate_click(menu.center(), gpui::Modifiers::default());
        cx.run_until_parked();
        for item in [
            "computer-preview-hide-task",
            "computer-preview-hide-all",
            "computer-preview-float",
            "computer-preview-sidebar",
        ] {
            assert!(cx.debug_bounds(item).is_some(), "{item}");
        }
        cx.simulate_click(menu.center(), gpui::Modifiers::default());
        cx.run_until_parked();
        assert!(
            cx.debug_bounds("computer-preview-float").is_none(),
            "closed"
        );

        pick(cx, "computer-preview-float");
        assert!(cx.update(|_, cx| store.read(cx).view("a").floating.is_some()));
        let menu = reveal(cx, "computer-preview-menu");
        cx.simulate_click(menu.center(), gpui::Modifiers::default());
        cx.run_until_parked();
        assert!(
            cx.debug_bounds("computer-preview-float").is_none(),
            "already floating"
        );
        let sidebar = cx
            .debug_bounds("computer-preview-sidebar")
            .expect("Move to sidebar");
        cx.simulate_click(sidebar.center(), gpui::Modifiers::default());
        cx.run_until_parked();
        assert!(cx.update(|_, cx| store.read(cx).view("a").floating.is_none()));
        assert!(
            wire.methods(cx)
                .iter()
                .any(|(method, _)| method == methods::GET_COMPUTER_SETTINGS),
            "the card placement switches to the rail through a fresh settings read"
        );

        change(&store, cx, |store| {
            driving(store, "b");
            store.mark_live("b")
        });
        pick(cx, "computer-preview-hide-all");
        assert_eq!(phase(&store, cx), Some(PreviewPhase::HiddenForTask));
        assert_eq!(
            cx.update(|_, cx| store.read(cx).view("b").phase),
            Some(PreviewPhase::HiddenForTask)
        );
        assert!(cx.debug_bounds("computer-preview-card").is_none());
    }

    #[gpui::test]
    fn a_finished_turn_flashes_a_checkmark_on_the_card(cx: &mut TestAppContext) {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let _guard = runtime.enter();
        let dir = tempfile::tempdir().unwrap();
        let (store, _card, _wire, cx) = mount(cx, dir.path());
        live_card(&store, cx);
        assert!(cx.debug_bounds("computer-preview-complete").is_none());
        change(&store, cx, |store| {
            store.apply_event(&ComputerEvent::ThreadState {
                state: Box::new(crate::computer::preview::tests::thread_state("a", false)),
            })
        });
        assert!(
            cx.debug_bounds("computer-preview-complete").is_some(),
            "the checkmark plays when the driving turn finishes"
        );
    }
    #[gpui::test]
    fn the_card_pops_out_into_a_floating_card_and_docks_back(cx: &mut TestAppContext) {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let _guard = runtime.enter();
        let dir = tempfile::tempdir().unwrap();
        let (store, _card, mut wire, cx) = mount(cx, dir.path());
        let docked = live_card(&store, cx);
        wire.methods(cx);
        assert_eq!(f32::from(docked.size.width), 400.0);
        assert!(cx.debug_bounds("computer-preview-dock").is_none());
        let pop = reveal(cx, "computer-preview-pop-out");
        cx.simulate_click(pop.center(), gpui::Modifiers::default());
        cx.run_until_parked();
        let floating = cx
            .debug_bounds("computer-preview-card")
            .expect("floating card painted");
        assert_eq!(f32::from(floating.size.width), 600.0);
        assert_eq!(
            f32::from(floating.origin.x),
            1200.0 - 600.0 - 8.0,
            "the detached card starts at its docked rect, clamped on screen"
        );
        assert_eq!(floating.origin.y, docked.origin.y);
        assert_eq!(phase(&store, cx), Some(PreviewPhase::Live));
        assert!(wire.methods(cx).is_empty(), "floating is UI-only");

        let grab = floating.center();
        cx.simulate_mouse_down(grab, MouseButton::Left, gpui::Modifiers::default());
        cx.simulate_mouse_move(
            grab - point(px(200.0), px(-40.0)),
            MouseButton::Left,
            gpui::Modifiers::default(),
        );
        cx.simulate_mouse_up(
            grab - point(px(200.0), px(-40.0)),
            MouseButton::Left,
            gpui::Modifiers::default(),
        );
        cx.run_until_parked();
        let dragged = cx.debug_bounds("computer-preview-card").unwrap();
        assert_eq!(
            dragged.origin,
            floating.origin - point(px(200.0), px(-40.0))
        );
        cx.simulate_mouse_move(
            grab - point(px(400.0), px(0.0)),
            None,
            gpui::Modifiers::default(),
        );
        cx.run_until_parked();
        assert_eq!(
            cx.debug_bounds("computer-preview-card").unwrap().origin,
            dragged.origin,
            "a released card stays put"
        );

        let dock = reveal(cx, "computer-preview-dock");
        cx.simulate_click(dock.center(), gpui::Modifiers::default());
        cx.run_until_parked();
        assert_eq!(cx.debug_bounds("computer-preview-card").unwrap(), docked);
        assert!(cx.update(|_, cx| store.read(cx).view("a").floating.is_none()));
    }

    #[gpui::test]
    fn dragging_the_docked_card_detaches_it(cx: &mut TestAppContext) {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let _guard = runtime.enter();
        let dir = tempfile::tempdir().unwrap();
        let (store, _card, _wire, cx) = mount(cx, dir.path());
        let docked = live_card(&store, cx);
        let grab = docked.center();
        cx.simulate_mouse_down(grab, MouseButton::Left, gpui::Modifiers::default());
        cx.simulate_mouse_move(
            grab + point(px(2.0), px(1.0)),
            MouseButton::Left,
            gpui::Modifiers::default(),
        );
        cx.run_until_parked();
        assert!(
            cx.update(|_, cx| store.read(cx).view("a").floating.is_none()),
            "a press that barely moves stays docked"
        );
        let to = grab - point(px(300.0), px(-100.0));
        cx.simulate_mouse_move(to, MouseButton::Left, gpui::Modifiers::default());
        cx.simulate_mouse_up(to, MouseButton::Left, gpui::Modifiers::default());
        cx.run_until_parked();
        let floating = cx.update(|_, cx| store.read(cx).view("a").floating);
        assert_eq!(
            floating,
            Some(point(
                f32::from(docked.origin.x) - 300.0,
                f32::from(docked.origin.y) + 100.0
            ))
        );
        let card = cx.debug_bounds("computer-preview-card").unwrap();
        assert_eq!(f32::from(card.size.width), 600.0);
    }

    #[gpui::test]
    fn a_viewer_without_the_tap_polls_the_still_frame_for_its_chat(cx: &mut TestAppContext) {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let _guard = runtime.enter();
        let dir = tempfile::tempdir().unwrap();
        let (store, _card, mut wire, cx) = mount(cx, dir.path());
        cx.run_until_parked();
        assert!(
            wire.methods(cx).is_empty(),
            "an idle remote chat does not poll"
        );
        change(&store, cx, |store| {
            store.apply_event(&ComputerEvent::OpenPaneRequested {
                thread_id: "a".into(),
            })
        });
        let calls = wire.methods(cx);
        let still = calls
            .iter()
            .find(|(method, _)| method == methods::GET_COMPUTER_STILL_FRAME)
            .expect("the still fallback is read");
        assert_eq!(still.1["chatId"], "a");
        change(&store, cx, |store| {
            store.accept_still("a", Some(image(3, 640, 400)))
        });
        let card = cx
            .debug_bounds("computer-preview-card")
            .expect("the still is shown");
        assert_eq!(f32::from(card.size.width), 400.0);
    }
}
