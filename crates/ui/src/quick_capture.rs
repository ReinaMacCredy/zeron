use std::cell::Cell;
use std::rc::Rc;
use std::time::{Duration, Instant};

use gpui::{
    AnyElement, App, Bounds, ClickEvent, Context, Entity, EventEmitter, FocusHandle, Focusable,
    KeyDownEvent, Pixels, ScrollHandle, SharedString, Subscription, Task, Window, WindowHandle,
    div, prelude::*, px,
};

use crate::composer::{ComposerInput, ComposerInputEvent};
use crate::icons::{self, icon};
use crate::motion;
use crate::motion::AnimationExt as _;
use crate::popover;
use crate::state::AppState;
use crate::tasks::images;
use crate::theme::Theme;
use crate::typography::ui_rems;

#[cfg(target_os = "macos")]
pub(crate) mod macos;

pub(crate) const CARD_WIDTH: f32 = 640.0;
pub(crate) const BAR_HEIGHT: f32 = 96.0;
pub(crate) const FROM_TOP: f32 = 0.18;
pub(crate) const TOAST_WIDTH: f32 = 460.0;
pub(crate) const TOAST_HEIGHT: f32 = 36.0;
pub(crate) const TICK: Duration = Duration::from_millis(180);
pub(crate) const FADE: Duration = Duration::from_millis(120);
pub(crate) const TOAST_FOR: Duration = Duration::from_millis(2600);
const OPEN: motion::MotionSpec = motion::MotionSpec::new(160, motion::EASE_OUT);
const PLACEHOLDER: &str = "Jot down an idea…";
const MENU_WIDTH: f32 = 220.0;
const MENU_MAX_HEIGHT: f32 = 280.0;
const MENU_GAP: f32 = 6.0;

#[derive(Default)]
pub struct Captured {
    unseen: Vec<String>,
    new: Vec<String>,
}

impl Captured {
    pub(crate) fn record(&mut self, task_id: &str, on_tasks: bool) {
        if on_tasks {
            self.new.push(task_id.to_string());
        } else {
            self.unseen.push(task_id.to_string());
        }
    }

    pub(crate) fn forget(&mut self, task_id: &str) {
        self.unseen.retain(|id| id != task_id);
        self.new.retain(|id| id != task_id);
    }

    pub(crate) fn tasks_opened(&mut self) {
        self.new = std::mem::take(&mut self.unseen);
    }

    pub(crate) fn is_new(&self, task_id: &str) -> bool {
        self.new.iter().any(|id| id == task_id)
    }

    pub(crate) fn unseen_count(&self, exists: impl Fn(&str) -> bool) -> usize {
        self.unseen.iter().filter(|id| exists(id)).count()
    }
}

pub(crate) fn split_capture(text: &str) -> Option<(String, String)> {
    let mut lines = text.trim().lines();
    let title: String = lines
        .next()?
        .trim()
        .chars()
        .take(zeron_doc::TASK_TITLE_MAX_CHARS)
        .collect();
    if title.is_empty() {
        return None;
    }
    let notes = lines.collect::<Vec<_>>().join("\n").trim().to_string();
    Some((title, notes))
}

pub(crate) fn hash_query(text: &str) -> Option<(usize, &str)> {
    let at = text.rfind('#')?;
    let query = &text[at + 1..];
    let starts_word = text[..at]
        .chars()
        .next_back()
        .is_none_or(char::is_whitespace);
    (starts_word && !query.chars().any(char::is_whitespace)).then_some((at, query))
}

pub(crate) fn project_matches(state: &AppState, query: &str) -> Vec<(String, SharedString)> {
    let query = query.to_lowercase();
    state
        .spaces_sorted()
        .into_iter()
        .filter(|space| space.display_name().to_lowercase().contains(&query))
        .map(|space| {
            (
                space.id.clone(),
                SharedString::from(space.display_name().to_string()),
            )
        })
        .collect()
}

pub(crate) fn project_label(state: &AppState, space_id: Option<&str>) -> SharedString {
    space_id
        .and_then(|id| state.space_row(id))
        .map(|space| SharedString::from(space.display_name().to_string()))
        .unwrap_or_else(|| "No project".into())
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Capture {
    pub task_id: String,
    pub title: String,
    pub notes: String,
    pub space_id: Option<String>,
    pub due_on: Option<String>,
    pub images: Vec<String>,
}

pub(crate) enum BarEvent {
    Saved { capture: Capture, open: bool },
    Dismiss,
    Blurred,
}

#[derive(Clone)]
pub(crate) struct CaptureToast {
    pub task_id: String,
    pub title: SharedString,
    pub shown: Instant,
}

pub(crate) struct QuickCaptureBar {
    state: Entity<AppState>,
    input: Entity<ComposerInput>,
    images: Vec<String>,
    uploads: Vec<Task<()>>,
    picking: Option<Task<()>>,
    project: Option<Option<String>>,
    today: bool,
    menu: Option<usize>,
    menu_from_chip: bool,
    menu_window: Option<WindowHandle<ProjectMenu>>,
    menu_opening: bool,
    menu_scroll: ScrollHandle,
    chip_bounds: Rc<Cell<Bounds<Pixels>>>,
    saved: Option<Instant>,
    opens: u64,
    seen_active: bool,
    activation: Option<Subscription>,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<BarEvent> for QuickCaptureBar {}

impl Focusable for QuickCaptureBar {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.input.focus_handle(cx)
    }
}

impl QuickCaptureBar {
    pub(crate) fn new(state: Entity<AppState>, cx: &mut Context<Self>) -> Self {
        let input = cx.new(|cx| ComposerInput::new(PLACEHOLDER, cx).with_text_metrics(16.0, 22.0));
        let events = cx.subscribe(&input, |bar, _, event: &ComposerInputEvent, cx| {
            if matches!(event, ComposerInputEvent::Submitted) {
                bar.enter(cx);
            } else if matches!(event, ComposerInputEvent::Edited) {
                bar.refresh_menu(cx);
            } else if let Some(staged) = images::pasted(event, cx) {
                bar.add_images(staged, cx);
            }
        });
        Self {
            state,
            input,
            images: Vec::new(),
            uploads: Vec::new(),
            picking: None,
            project: None,
            today: false,
            menu: None,
            menu_from_chip: false,
            menu_window: None,
            menu_opening: false,
            menu_scroll: ScrollHandle::new(),
            chip_bounds: Rc::default(),
            saved: None,
            opens: 0,
            seen_active: false,
            activation: None,
            _subscriptions: vec![events],
        }
    }

    pub(crate) fn opened(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.opens += 1;
        self.saved = None;
        self.menu = None;
        self.menu_from_chip = false;
        self.seen_active = false;
        self.activation = Some(cx.observe_window_activation(window, |bar, window, cx| {
            if window.is_window_active() {
                bar.seen_active = true;
            } else if bar.seen_active && bar.saved.is_none() {
                cx.emit(BarEvent::Blurred);
            }
        }));
        window.focus(&self.input.focus_handle(cx), cx);
        cx.notify();
    }

    fn space_id(&self, cx: &App) -> Option<String> {
        match &self.project {
            Some(picked) => picked.clone(),
            None => self
                .state
                .read(cx)
                .selected_space_row()
                .map(|space| space.id.clone()),
        }
    }

    fn add_images(
        &mut self,
        staged: Vec<crate::attachments::StagedAttachment>,
        cx: &mut Context<Self>,
    ) {
        if staged.is_empty() {
            return;
        }
        let upload = images::upload(&self.state, staged, cx);
        self.uploads.push(cx.spawn(async move |bar, cx| {
            let paths = upload.await;
            bar.update(cx, |bar, cx| {
                images::append(&mut bar.images, paths, cx);
                cx.notify();
            })
            .ok();
        }));
    }

    fn pick_images(&mut self, cx: &mut Context<Self>) {
        let picked = images::pick(cx);
        self.picking = Some(cx.spawn(async move |bar, cx| {
            let paths = picked.await;
            bar.update(cx, |bar, cx| {
                let staged = images::staged_files(&paths, cx);
                bar.add_images(staged, cx);
            })
            .ok();
        }));
    }

    fn menu_rows(&self, cx: &App) -> Vec<(String, SharedString)> {
        let text = self.input.read(cx).text();
        let query = if self.menu_from_chip {
            ""
        } else {
            hash_query(text).map_or("", |(_, query)| query)
        };
        project_matches(self.state.read(cx), query)
    }

    fn refresh_menu(&mut self, cx: &mut Context<Self>) {
        let typing = hash_query(self.input.read(cx).text()).is_some();
        if typing {
            let fresh = self.menu.is_none() || self.menu_from_chip;
            self.menu_from_chip = false;
            let current = self.space_id(cx);
            let rows = self.menu_rows(cx);
            let start = rows
                .iter()
                .position(|(id, _)| fresh && Some(id) == current.as_ref())
                .unwrap_or(0);
            self.menu = (!rows.is_empty()).then(|| {
                if fresh {
                    start
                } else {
                    self.menu.unwrap_or(0).min(rows.len() - 1)
                }
            });
        } else if !self.menu_from_chip {
            self.menu = None;
        }
        cx.notify();
    }

    fn toggle_menu(&mut self, cx: &mut Context<Self>) {
        if self.menu.is_some() {
            self.menu = None;
            self.menu_from_chip = false;
        } else {
            self.menu_from_chip = true;
            let current = self.space_id(cx);
            let rows = self.menu_rows(cx);
            self.menu = (!rows.is_empty()).then(|| {
                rows.iter()
                    .position(|(id, _)| Some(id) == current.as_ref())
                    .unwrap_or(0)
            });
            if let Some(active) = self.menu {
                self.menu_scroll.scroll_to_item(active);
            }
        }
        cx.notify();
    }

    fn step_menu(&mut self, delta: isize, cx: &mut Context<Self>) -> bool {
        let Some(active) = self.menu else {
            return false;
        };
        let count = self.menu_rows(cx).len();
        self.menu = popover::menu_step(Some(active), count, delta);
        if let Some(active) = self.menu {
            self.menu_scroll.scroll_to_item(active);
        }
        cx.notify();
        true
    }

    pub(crate) fn close_menu(&mut self, cx: &mut Context<Self>) {
        self.menu = None;
        self.menu_from_chip = false;
        if let Some(handle) = self.menu_window.take() {
            cx.defer(move |cx| {
                handle
                    .update(cx, |_, window, _| window.remove_window())
                    .ok();
            });
        }
    }

    fn menu_shown(&self) -> bool {
        self.menu_window.is_some() || self.menu_opening
    }

    fn sync_menu_window(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match (self.menu.is_some(), self.menu_shown()) {
            (true, false) => self.open_menu_window(window, cx),
            (false, true) => self.close_menu(cx),
            _ => {}
        }
    }

    fn open_menu_window(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let chip = self.chip_bounds.get();
        let anchor = gpui::point(chip.left(), chip.bottom() + px(MENU_GAP));
        let origin = window.bounds().origin + anchor;
        let options = gpui::WindowOptions {
            window_bounds: Some(gpui::WindowBounds::Windowed(Bounds::new(
                origin,
                gpui::size(px(MENU_WIDTH), px(MENU_MAX_HEIGHT)),
            ))),
            titlebar: None,
            focus: false,
            show: true,
            kind: gpui::WindowKind::PopUp,
            is_movable: false,
            is_resizable: false,
            is_minimizable: false,
            display_id: window.display(cx).map(|display| display.id()),
            window_background: gpui::WindowBackgroundAppearance::Blurred,
            ..Default::default()
        };
        #[cfg(target_os = "macos")]
        let place = macos::place_below(window, anchor);
        let bar = cx.entity();
        self.menu_opening = true;
        // Opening renders the menu at once, and its render reads this bar, so it cannot run inside the bar's update.
        cx.defer(move |cx| {
            let menu_bar = bar.clone();
            let opened = cx.open_window(options, move |window, cx| {
                #[cfg(target_os = "macos")]
                {
                    macos::style_panel(window, 10.0);
                    if let Some(place) = place {
                        place(window);
                    }
                }
                #[cfg(not(target_os = "macos"))]
                let _ = &window;
                cx.new(|cx| ProjectMenu {
                    _observe: cx.observe(&menu_bar, |_, _, cx| cx.notify()),
                    bar: menu_bar,
                })
            });
            bar.update(cx, |bar, cx| {
                bar.menu_opening = false;
                bar.menu_window = opened.ok();
                if bar.menu.is_none() {
                    bar.close_menu(cx);
                }
            });
        });
    }

    fn pick_project(&mut self, ix: usize, cx: &mut Context<Self>) {
        let Some((space_id, _)) = self.menu_rows(cx).into_iter().nth(ix) else {
            return;
        };
        self.project = Some(Some(space_id));
        if !self.menu_from_chip {
            let text = self.input.read(cx).text().to_string();
            if let Some((at, _)) = hash_query(&text) {
                let kept = text[..at].to_string();
                self.input.update(cx, |input, cx| input.set_text(kept, cx));
            }
        }
        self.close_menu(cx);
        cx.notify();
    }

    fn refocus(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.input.focus_handle(cx), cx);
        cx.notify();
    }

    fn enter(&mut self, cx: &mut Context<Self>) {
        if let Some(ix) = self.menu {
            self.pick_project(ix, cx);
        } else {
            self.save(false, cx);
        }
    }

    fn save(&mut self, open: bool, cx: &mut Context<Self>) {
        if self.saved.is_some() {
            return;
        }
        let text = self.input.read(cx).text().to_string();
        let text = match hash_query(&text) {
            Some((at, _)) => text[..at].to_string(),
            None => text,
        };
        let Some((title, notes)) = split_capture(&text) else {
            return;
        };
        let due_on = self.today.then(|| {
            chrono::Local::now()
                .date_naive()
                .format("%Y-%m-%d")
                .to_string()
        });
        let capture = Capture {
            task_id: uuid::Uuid::new_v4().to_string(),
            title,
            notes,
            space_id: self.space_id(cx),
            due_on,
            images: std::mem::take(&mut self.images),
        };
        self.input.update(cx, |input, cx| input.set_text("", cx));
        self.project = None;
        self.today = false;
        self.menu = None;
        self.saved = Some(motion::now());
        cx.emit(BarEvent::Saved { capture, open });
        cx.notify();
    }

    fn on_key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let keystroke = &event.keystroke;
        if keystroke.key == "escape" {
            cx.stop_propagation();
            if self.menu.take().is_some() {
                self.menu_from_chip = false;
                cx.notify();
            } else {
                cx.emit(BarEvent::Dismiss);
            }
        } else if keystroke.key == "enter" && keystroke.modifiers.platform {
            cx.stop_propagation();
            self.save(true, cx);
        }
        let _ = window;
    }

    fn render_card(&mut self, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let state = self.state.read(cx);
        let space_id = self.space_id(cx);
        let project = project_label(state, space_id.as_deref());
        let tick = self
            .saved
            .map(|saved| (motion::since(saved).as_secs_f32() / TICK.as_secs_f32()).clamp(0.0, 1.0));
        let image_count = self.images.len();
        let chip_bounds = self.chip_bounds.clone();
        let chip = |id: &'static str, on: bool| {
            div()
                .id(id)
                .debug_selector(move || id.into())
                .h(px(24.0))
                .px(px(8.0))
                .flex()
                .items_center()
                .gap(px(5.0))
                .rounded(px(7.0))
                .cursor_pointer()
                .text_size(ui_rems(12.0))
                .bg(if on {
                    theme.accent.opacity(0.18)
                } else {
                    theme.text.opacity(0.06)
                })
                .text_color(if on { theme.accent } else { theme.text_muted })
                .hover(|style| style.bg(theme.text.opacity(0.10)))
        };
        let hint = |cap: &'static str, label: &'static str| {
            div()
                .flex()
                .items_center()
                .gap(px(4.0))
                .child(
                    div()
                        .px(px(5.0))
                        .py(px(1.0))
                        .rounded(px(4.0))
                        .bg(theme.text.opacity(0.10))
                        .font_family(popover::KEY_GLYPH_FONT)
                        .text_size(ui_rems(11.0))
                        .child(cap),
                )
                .when(!label.is_empty(), |el| el.child(label))
        };
        let box_size = 18.0;
        let checkbox = div()
            .mt(px(2.0))
            .size(px(box_size))
            .flex_none()
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(5.0))
            .border(px(1.5))
            .map(|el| match tick {
                Some(t) => el
                    .border_color(theme.success)
                    .bg(theme.success.opacity(t))
                    .child(
                        icon(icons::CHECK)
                            .size(px(12.0))
                            .text_color(theme.on_accent.opacity(t)),
                    ),
                None => el.border_color(theme.text.opacity(0.35)),
            });
        div()
            .id("quick-capture-card")
            .debug_selector(|| "quick-capture-card".into())
            .relative()
            .w(px(CARD_WIDTH))
            .flex()
            .flex_col()
            .rounded(px(16.0))
            .border_1()
            .border_color(theme.text.opacity(0.12))
            .bg(panel_surface(theme))
            .text_color(theme.text)
            .child(fit_panel())
            .child(
                div()
                    .flex()
                    .items_start()
                    .gap(px(10.0))
                    .pt(px(14.0))
                    .px(px(16.0))
                    .pb(px(6.0))
                    .child(checkbox)
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .max_h(px(22.0 * 6.0))
                            .overflow_hidden()
                            .text_size(px(16.0))
                            .child(self.input.clone()),
                    ),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.0))
                    .pt(px(6.0))
                    .pb(px(10.0))
                    .pl(px(40.0))
                    .pr(px(10.0))
                    .text_size(ui_rems(12.0))
                    .text_color(theme.text_muted)
                    .child(
                        chip("quick-capture-project", false)
                            .relative()
                            .role(gpui::Role::Button)
                            .aria_label(SharedString::from(format!("Project: {project}")))
                            .child(
                                icon(icons::LUCIDE_FOLDER)
                                    .size(px(12.0))
                                    .text_color(theme.text_muted),
                            )
                            .child(project)
                            .child(
                                gpui::canvas(
                                    move |bounds, _, _| chip_bounds.set(bounds),
                                    |_, _, _, _| {},
                                )
                                .absolute()
                                .top_0()
                                .left_0()
                                .size_full(),
                            )
                            .on_click(cx.listener(|bar, _: &ClickEvent, window, cx| {
                                bar.toggle_menu(cx);
                                bar.refocus(window, cx);
                            })),
                    )
                    .child(
                        chip("quick-capture-today", self.today)
                            .role(gpui::Role::Button)
                            .aria_label("Due today")
                            .child("Today")
                            .on_click(cx.listener(|bar, _: &ClickEvent, window, cx| {
                                bar.today = !bar.today;
                                bar.refocus(window, cx);
                            })),
                    )
                    .child(
                        chip("quick-capture-image", image_count > 0)
                            .role(gpui::Role::Button)
                            .aria_label("Add image (or paste with ⌘V)")
                            .child(icon(icons::GALLERY).size(px(12.0)).text_color(
                                if image_count > 0 {
                                    theme.accent
                                } else {
                                    theme.text_muted
                                },
                            ))
                            .when(image_count > 0, |el| el.child(image_count.to_string()))
                            .on_click(cx.listener(|bar, _: &ClickEvent, window, cx| {
                                bar.pick_images(cx);
                                bar.refocus(window, cx);
                            })),
                    )
                    .child(div().flex_1())
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(8.0))
                            .child(hint("⏎", "save"))
                            .child(hint("⌘⏎", "save and open"))
                            .child(hint("⇧⏎", "notes"))
                            .child(hint("Esc", "")),
                    ),
            )
            .into_any_element()
    }
}

impl Render for QuickCaptureBar {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::of(cx).for_popup();
        let fade = self.saved.map_or(1.0, |saved| {
            let after_tick = motion::since(saved).saturating_sub(TICK);
            1.0 - (after_tick.as_secs_f32() / FADE.as_secs_f32()).clamp(0.0, 1.0)
        });
        if self.saved.is_some() && fade > 0.0 {
            window.request_animation_frame();
        }
        if self.menu.is_some() != self.menu_shown() {
            cx.defer_in(window, Self::sync_menu_window);
        }
        let card = self.render_card(&theme, cx);
        let opens = self.opens;
        div()
            .id("quick-capture")
            .key_context("QuickCapture")
            .size_full()
            .font_family(theme.font_sans.clone())
            .on_key_down(cx.listener(Self::on_key))
            .capture_action(cx.listener(|bar, _: &crate::composer::Up, _, cx| {
                if bar.step_menu(-1, cx) {
                    cx.stop_propagation();
                }
            }))
            .capture_action(cx.listener(|bar, _: &crate::composer::Down, _, cx| {
                if bar.step_menu(1, cx) {
                    cx.stop_propagation();
                }
            }))
            .child(div().id("quick-capture-body").child(card).with_animation(
                ("quick-capture-open", opens),
                OPEN.animation(),
                move |el, t| el.opacity(t * fade),
            ))
    }
}

pub(crate) struct ProjectMenu {
    bar: Entity<QuickCaptureBar>,
    _observe: Subscription,
}

impl Render for ProjectMenu {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::of(cx).for_popup();
        let bar = self.bar.read(cx);
        let (rows, active, scroll) = (bar.menu_rows(cx), bar.menu, bar.menu_scroll.clone());
        div()
            .size_full()
            .flex()
            .items_start()
            .font_family(theme.font_sans.clone())
            .child(
                div()
                    .relative()
                    .flex_none()
                    .w(px(MENU_WIDTH))
                    .rounded(px(10.0))
                    .border_1()
                    .border_color(theme.text.opacity(0.12))
                    .bg(panel_surface(&theme))
                    .child(fit_panel())
                    .child(
                        div()
                            .id("quick-capture-projects")
                            .debug_selector(|| "quick-capture-projects".into())
                            .max_h(px(MENU_MAX_HEIGHT))
                            .overflow_y_scroll()
                            .track_scroll(&scroll)
                            .p(px(4.0))
                            .flex()
                            .flex_col()
                            .text_size(ui_rems(13.0))
                            .text_color(theme.text)
                            .children(rows.into_iter().enumerate().map(|(ix, (_, name))| {
                                let bar = self.bar.clone();
                                div()
                                    .id(("quick-capture-project-row", ix))
                                    .flex_none()
                                    .px(px(9.0))
                                    .py(px(6.0))
                                    .rounded(px(6.0))
                                    .cursor_pointer()
                                    .truncate()
                                    .when(Some(ix) == active, |el| el.bg(theme.text.opacity(0.08)))
                                    .hover(|style| style.bg(theme.text.opacity(0.08)))
                                    .child(name)
                                    .on_click(move |_: &ClickEvent, _, cx| {
                                        bar.update(cx, |bar, cx| bar.pick_project(ix, cx))
                                    })
                            })),
                    ),
            )
    }
}

fn panel_surface(theme: &Theme) -> gpui::Hsla {
    if cfg!(target_os = "macos") {
        theme.surface_dialog.opacity(0.82)
    } else {
        theme.surface_dialog
    }
}

fn fit_panel() -> impl IntoElement {
    gpui::canvas(
        |bounds, window, cx| {
            #[cfg(all(target_os = "macos", not(test)))]
            {
                let have = window.viewport_size();
                let off = (bounds.size.width - have.width).abs() > px(0.5)
                    || (bounds.size.height - have.height).abs() > px(0.5);
                if off && let Some(resize) = macos::resize_panel(window) {
                    let (width, height) = (
                        f64::from(f32::from(bounds.size.width)),
                        f64::from(f32::from(bounds.size.height)),
                    );
                    cx.spawn(async move |_| resize(width, height)).detach();
                }
            }
            #[cfg(any(not(target_os = "macos"), test))]
            let _ = (bounds, window, cx);
        },
        |_, _, _, _| {},
    )
    .absolute()
    .top_0()
    .left_0()
    .size_full()
}

pub(crate) fn is_panel(window: gpui::AnyWindowHandle) -> bool {
    window.downcast::<QuickCaptureBar>().is_some()
        || window.downcast::<ProjectMenu>().is_some()
        || window.downcast::<ToastPanel>().is_some()
}

pub(crate) fn toast_card(
    title: SharedString,
    theme: &Theme,
    on_undo: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    on_open: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> gpui::Stateful<gpui::Div> {
    let link = |id: &'static str, label: &'static str| {
        div()
            .id(id)
            .debug_selector(move || id.into())
            .role(gpui::Role::Button)
            .aria_label(label)
            .px(px(2.0))
            .cursor_pointer()
            .text_color(theme.accent)
            .hover(|style| style.text_color(theme.accent_strong))
            .child(label)
    };
    div()
        .id("quick-capture-toast")
        .debug_selector(|| "quick-capture-toast".into())
        .flex()
        .items_center()
        .gap(px(10.0))
        .pl(px(12.0))
        .pr(px(10.0))
        .py(px(8.0))
        .rounded(px(10.0))
        .border_1()
        .border_color(theme.border)
        .bg(theme.surface_dialog)
        .shadow_lg()
        .flex_none()
        .text_size(ui_rems(12.5))
        .text_color(theme.text)
        .child(
            icon(icons::CHECK)
                .size(px(13.0))
                .flex_none()
                .text_color(theme.success),
        )
        .child(div().flex_none().child("Added to To do:"))
        .child(div().max_w(px(300.0)).truncate().child(title))
        .child(link("quick-capture-undo", "Undo").on_click(on_undo))
        .child(link("quick-capture-open", "Open").on_click(on_open))
}

pub(crate) struct ToastPanel {
    pub toast: CaptureToast,
    pub on_undo: std::rc::Rc<dyn Fn(&mut Window, &mut App)>,
    pub on_open: std::rc::Rc<dyn Fn(&mut Window, &mut App)>,
}

impl Render for ToastPanel {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::of(cx).for_popup();
        let (undo, open) = (self.on_undo.clone(), self.on_open.clone());
        div()
            .size_full()
            .flex()
            .items_start()
            .font_family(theme.font_sans.clone())
            .child(motion::fade_quick(
                "quick-capture-toast-in",
                toast_card(
                    self.toast.title.clone(),
                    &theme,
                    move |_, window, cx| undo(window, cx),
                    move |_, window, cx| open(window, cx),
                )
                .relative()
                .bg(panel_surface(&theme))
                .child(fit_panel()),
            ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_line_is_the_title_and_the_rest_the_notes() {
        assert_eq!(split_capture("  \n "), None);
        assert_eq!(
            split_capture("Save carts\r\nfor later\n\nand email\n"),
            Some(("Save carts".into(), "for later\n\nand email".into()))
        );
        assert_eq!(
            split_capture("One line"),
            Some(("One line".into(), String::new()))
        );
    }

    #[test]
    fn hash_opens_the_project_menu_only_at_a_word_start() {
        assert_eq!(hash_query("Recolor the button #"), Some((19, "")));
        assert_eq!(hash_query("#ship"), Some((0, "ship")));
        assert_eq!(hash_query("issue#12"), None);
        assert_eq!(hash_query("#ship it"), None);
        assert_eq!(hash_query("no hash"), None);
    }

    #[gpui::test]
    fn escape_keeps_the_draft_and_a_save_clears_it(cx: &mut gpui::TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        cx.update(|cx| {
            gpui_base::init(cx);
            cx.set_global(Theme::dark());
            crate::settings::init(crate::settings::UiSettings::default(), dir.path(), cx);
            crate::composer::init(cx, Default::default());
        });
        let state = cx.new(|_| AppState::new());
        let (bar, cx) = cx.add_window_view(|_, cx| QuickCaptureBar::new(state, cx));
        let events = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        let sink = events.clone();
        cx.update(|_, cx| {
            cx.subscribe(&bar, move |_, event: &BarEvent, _| {
                sink.borrow_mut().push(match event {
                    BarEvent::Saved { capture, open } => {
                        format!("saved {} | {} | {open}", capture.title, capture.notes)
                    }
                    BarEvent::Dismiss => "dismiss".into(),
                    BarEvent::Blurred => "blurred".into(),
                })
            })
            .detach();
        });
        bar.update_in(cx, |bar, window, cx| {
            bar.opened(window, cx);
            bar.input
                .update(cx, |input, cx| input.set_text("Save carts\nfor later", cx));
        });
        cx.run_until_parked();
        cx.simulate_keystrokes("escape");
        let draft = bar.read_with(cx, |bar, cx| bar.input.read(cx).text().to_string());
        assert_eq!(draft, "Save carts\nfor later");
        cx.simulate_keystrokes("cmd-enter");
        let draft = bar.read_with(cx, |bar, cx| bar.input.read(cx).text().to_string());
        assert_eq!(draft, "");
        assert_eq!(
            *events.borrow(),
            ["dismiss", "saved Save carts | for later | true"]
        );
    }

    #[test]
    fn opening_tasks_tags_this_batch_once_and_clears_the_count() {
        let tasks = ["a", "b"];
        let exists = |id: &str| tasks.contains(&id);
        let mut captured = Captured::default();
        captured.record("a", false);
        captured.record("b", false);
        captured.record("gone", false);
        assert_eq!(captured.unseen_count(exists), 2);
        assert!(!captured.is_new("a"));
        captured.tasks_opened();
        assert_eq!(captured.unseen_count(exists), 0);
        assert!(captured.is_new("a") && captured.is_new("b"));
        captured.record("c", true);
        assert!(captured.is_new("c"));
        assert_eq!(captured.unseen_count(exists), 0);
        captured.forget("a");
        assert!(!captured.is_new("a"));
        captured.tasks_opened();
        assert!(!captured.is_new("b") && !captured.is_new("c"));
    }
}
