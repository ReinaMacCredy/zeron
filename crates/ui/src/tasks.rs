use std::cell::Cell;
use std::rc::Rc;
use std::time::{Duration, Instant};

use chrono::{DateTime, Local, NaiveDate, Utc};
use gpui::{
    AnyElement, Bounds, ClickEvent, Context, DragMoveEvent, Edges, Entity, EventEmitter,
    FocusHandle, Focusable, Hsla, IntoElement, KeyDownEvent, Modifiers, MouseButton,
    MouseDownEvent, Pixels, Point, Render, ScrollHandle, SharedString, Subscription, Window, div,
    prelude::*, px,
};
use zeron_proto::mission::column_title;
use zeron_proto::{
    ChatIndicator, PlanProgress, StepStatus, TaskPatch, TaskStatus, view::task_thread_status,
};

use crate::composer::{ComposerInput, ComposerInputEvent, MentionTab};
use crate::icons::{self, icon};
use crate::motion;
use crate::popover;
use crate::settings::{self, TasksView, widgets};
use crate::state::AppState;
use crate::theme::Theme;
use crate::typography::ui_rems;

mod card;
mod center;
mod drag;
#[cfg(test)]
mod image_paths_tests;
pub(crate) mod images;
mod index;

use crate::attachments::StagedAttachment;
use images::ImageGallery;
mod logic;
mod morph;
mod strip_tree;
mod thread_tree;
pub(crate) use thread_tree::{tree_rows, watched_docs};
mod tree;

pub use card::{CardFocus, Orchestrate, TaskCard, TaskCardEvent};
pub use center::{LiveThread, PlanEdit, Selection, TaskCenter, TaskCenterEvent};
pub(crate) use index::tree_index;
use logic::COLUMNS;
use morph::{Key, Morph, Painted, Rects};
pub(crate) use strip_tree::{SampleRow, StripTree, StripTreeEvent, TreeHost};
pub(crate) use tree::{Status as TreeStatus, StatusTone, glyph as tree_glyph, kind_badge};

pub(crate) const ADD_PLACEHOLDER: &str = "Add a task, or press Tab to add notes and start it";
const LIST_MAX_WIDTH: f32 = 640.0;
const CARD_WIDTH: f32 = 340.0;
const ROW_HEIGHT: f32 = 44.0;
const CHECK_SIZE: f32 = 18.0;
const ROW_BORDER: Edges<f32> = Edges {
    top: 2.0,
    right: 0.0,
    bottom: 0.0,
    left: 0.0,
};
const GUTTER: f32 = 56.0;
const ROW_GAP: f32 = 2.0;
const BAR_IN: motion::MotionSpec = motion::MotionSpec::new(180, motion::EASE_OUT_QUINT);
const BAR_RISE: f32 = 14.0;
const PENDING_ORDER: Duration = Duration::from_secs(3);
const SAVE_DEBOUNCE: Duration = Duration::from_millis(500);
const START_PENDING: Duration = Duration::from_secs(15);

pub enum TasksEvent {
    Create {
        task_id: String,
        title: String,
        space_id: Option<String>,
        images: Vec<String>,
    },
    Update {
        task_id: String,
        patch: TaskPatch,
    },
    Card(TaskCardEvent),
    OpenCenter(String),
    Gather {
        task_ids: Vec<String>,
        chat_id: Option<String>,
    },
}

#[derive(Clone)]
struct TaskDrag {
    task_id: String,
    title: SharedString,
    status: TaskStatus,
}

struct TaskDragGhost {
    title: SharedString,
}

struct LiftedRowGhost;

impl Render for LiftedRowGhost {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        gpui::Empty
    }
}

impl Render for TaskDragGhost {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::of(cx);
        div()
            .w(px(260.0))
            .h(px(36.0))
            .px(px(12.0))
            .flex()
            .items_center()
            .rounded(px(10.0))
            .bg(theme.surface_raised)
            .border_1()
            .border_color(theme.border_strong)
            .text_size(ui_rems(13.0))
            .text_color(theme.text)
            .opacity(0.9)
            .child(div().truncate().child(self.title.clone()))
    }
}

struct Linked {
    title: SharedString,
    indicator: ChatIndicator,
    brand: (&'static str, Option<Hsla>),
    since: Option<DateTime<Utc>>,
}

type Brand = (&'static str, Option<Hsla>);

pub(crate) type StepRow = zeron_proto::mission::StepRow<Brand>;
pub(crate) type Rollup = zeron_proto::mission::Rollup<Brand>;
pub(crate) use zeron_doc::tree::{linked_label, task_led_by};
pub(crate) use zeron_proto::mission::{
    MissionRole, MissionThread, PlanPhase, is_mission, linked_chat, plan_led_by, step_chat,
};

struct Row {
    task: zeron_proto::Task,
    status: TaskStatus,
    project: Option<SharedString>,
    linked: Option<Linked>,
    plan: Rollup,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TasksFilter {
    All,
    Missions,
    Trees,
    NeedsYou,
}

impl TasksFilter {
    fn label(self) -> &'static str {
        match self {
            Self::All => "All",
            Self::Missions => "Missions",
            Self::Trees => "Trees",
            Self::NeedsYou => "Needs you",
        }
    }

    fn keeps(self, row: &Row) -> bool {
        match self {
            Self::All => true,
            Self::Missions => is_mission(&row.task, &row.plan),
            Self::Trees => false,
            Self::NeedsYou => row.status == TaskStatus::NeedsYou,
        }
    }

    fn keeps_tree(self, row: &index::IndexRow) -> bool {
        match self {
            Self::All | Self::Trees => true,
            Self::Missions => false,
            Self::NeedsYou => row.asks,
        }
    }
}

impl Row {
    fn orchestrated(&self) -> bool {
        self.task.has_plan() && self.linked.is_some()
    }

    fn marks(&self) -> Vec<Brand> {
        let mut marks: Vec<Brand> = Vec::new();
        let lead = self.linked.as_ref().map(|linked| linked.brand);
        for brand in lead
            .into_iter()
            .chain(self.plan.steps.iter().filter_map(|s| s.brand))
        {
            if marks.len() < 3 && !marks.iter().any(|(mark, _)| *mark == brand.0) {
                marks.push(brand);
            }
        }
        marks
    }
}

pub struct TasksPage {
    state: Entity<AppState>,
    add: Entity<ComposerInput>,
    add_images: Vec<String>,
    add_uploads: Vec<gpui::Task<()>>,
    picking: Option<gpui::Task<()>>,
    gallery: Entity<ImageGallery>,
    card: Entity<TaskCard>,
    done_open: bool,
    filter: TasksFilter,
    open_first_card: bool,
    focus: FocusHandle,
    view_now: Option<TasksView>,
    morph: Option<Morph>,
    painted: Painted,
    body: Rc<Cell<Option<Bounds<Pixels>>>>,
    menu: popover::Popup<RowMenu>,
    drag: Option<drag::ListDrag>,
    shifts: drag::Shifts,
    pending: Option<(Vec<String>, Instant)>,
    picked: Vec<String>,
    anchor: Option<String>,
    press: f32,
    scroll: ScrollHandle,
    shown: Vec<String>,
    rects: Rects,
    autoscroll: Option<gpui::Task<()>>,
    bar: motion::Tween,
    bar_count: usize,
    thread_picker: bool,
    _subscriptions: Vec<Subscription>,
}

struct RowMenu {
    task_id: String,
    position: Point<Pixels>,
}

type MorphRow<'a> = (&'a Row, Option<Bounds<Pixels>>, Option<Bounds<Pixels>>);

impl EventEmitter<TasksEvent> for TasksPage {}

impl Focusable for TasksPage {
    fn focus_handle(&self, _: &gpui::App) -> FocusHandle {
        self.focus.clone()
    }
}

impl TasksPage {
    pub fn new(state: Entity<AppState>, cx: &mut Context<Self>) -> Self {
        let add = cx.new(|cx| {
            ComposerInput::new(ADD_PLACEHOLDER, cx)
                .with_single_line()
                .with_max_chars(zeron_doc::TASK_TITLE_MAX_CHARS)
        });
        let add_events = cx.subscribe(&add, |page, _, event: &ComposerInputEvent, cx| {
            if matches!(event, ComposerInputEvent::Submitted) {
                page.submit_add(false, None, cx);
            } else if let Some(staged) = images::pasted(event, cx) {
                page.add_images(staged, cx);
            }
        });
        let gallery = cx.new(|cx| ImageGallery::new(state.clone(), cx));
        let state_changes = cx.observe(&state, |page, _, cx| {
            page.open_knob_card(cx);
            cx.notify();
        });
        let card = cx.new(|cx| TaskCard::new(state.clone(), cx));
        let card_events = cx.subscribe(&card, |_, _, event: &TaskCardEvent, cx| {
            cx.emit(TasksEvent::Card(event.clone()))
        });
        let card_changes = cx.observe(&card, |_, _, cx| cx.notify());
        let settings_changes = cx.observe_global::<settings::SettingsStore>(|_, cx| cx.notify());
        let mut page = Self {
            state,
            add,
            add_images: Vec::new(),
            add_uploads: Vec::new(),
            picking: None,
            gallery,
            card,
            done_open: true,
            filter: if std::env::var("ZERON_OPEN_ROUTE").as_deref() == Ok("mission-control") {
                TasksFilter::Missions
            } else {
                TasksFilter::All
            },
            open_first_card: std::env::var("ZERON_OPEN_DIALOG").as_deref() == Ok("task-card"),
            focus: cx.focus_handle(),
            view_now: None,
            morph: None,
            painted: Painted::default(),
            body: Rc::default(),
            menu: popover::Popup::default(),
            drag: None,
            shifts: drag::Shifts::default(),
            pending: None,
            picked: Vec::new(),
            anchor: None,
            press: 0.0,
            scroll: ScrollHandle::new(),
            shown: Vec::new(),
            rects: Rects::default(),
            autoscroll: None,
            bar: motion::Tween::at(0.0, motion::now()),
            bar_count: 0,
            thread_picker: false,
            _subscriptions: vec![
                add_events,
                state_changes,
                settings_changes,
                card_events,
                card_changes,
            ],
        };
        page.open_knob_card(cx);
        page
    }

    pub fn card(&self) -> &Entity<TaskCard> {
        &self.card
    }

    pub(crate) fn set_filter(&mut self, filter: TasksFilter, cx: &mut Context<Self>) {
        self.filter = filter;
        cx.notify();
    }

    fn open_knob_card(&mut self, cx: &mut Context<Self>) {
        if !self.open_first_card {
            return;
        }
        let first = {
            let (open, _) = zeron_proto::mission::split_tasks(&self.state.read(cx).tasks);
            open.iter()
                .find(|task| task.chat_id.is_none())
                .or(open.first())
                .map(|task| task.id.clone())
        };
        if let Some(id) = first {
            self.open_first_card = false;
            self.card.update(cx, |card, cx| {
                card.open(id, None, CardFocus::Title, None, cx)
            });
        }
    }

    fn today() -> NaiveDate {
        Local::now().date_naive()
    }

    fn add_images(&mut self, staged: Vec<StagedAttachment>, cx: &mut Context<Self>) {
        if staged.is_empty() {
            return;
        }
        let upload = images::upload(&self.state, staged, cx);
        self.add_uploads.push(cx.spawn(async move |page, cx| {
            let paths = upload.await;
            page.update(cx, |page, cx| {
                images::append(&mut page.add_images, paths, cx);
                cx.notify();
            })
            .ok();
        }));
    }

    fn pick_add_images(&mut self, cx: &mut Context<Self>) {
        let picked = images::pick(cx);
        self.picking = Some(cx.spawn(async move |page, cx| {
            let paths = picked.await;
            page.update(cx, |page, cx| {
                let staged = images::staged_files(&paths, cx);
                page.add_images(staged, cx);
            })
            .ok();
        }));
    }

    fn open_images(&mut self, paths: Vec<String>, index: usize, cx: &mut Context<Self>) {
        self.gallery
            .update(cx, |gallery, cx| gallery.open(paths, index, cx));
    }

    fn submit_add(
        &mut self,
        open_card: bool,
        window: Option<&mut Window>,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(title) = logic::quick_add_title(self.add.read(cx).text()) else {
            return false;
        };
        let space_id = self
            .state
            .read(cx)
            .selected_space_row()
            .map(|space| space.id.clone());
        self.add.update(cx, |input, cx| input.set_text("", cx));
        let task_id = uuid::Uuid::new_v4().to_string();
        cx.emit(TasksEvent::Create {
            task_id: task_id.clone(),
            title: title.clone(),
            space_id,
            images: std::mem::take(&mut self.add_images),
        });
        if open_card {
            self.card.update(cx, |card, cx| {
                card.open(task_id, Some(title), CardFocus::Notes, window, cx)
            });
        }
        true
    }

    fn open_row(
        &mut self,
        task_id: &str,
        paged: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if paged {
            cx.emit(TasksEvent::OpenCenter(task_id.to_string()));
            return;
        }
        self.open_card(task_id.to_string(), window, cx);
    }

    pub(crate) fn open_card(
        &mut self,
        task_id: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.card.update(cx, |card, cx| {
            card.open(task_id, None, CardFocus::Title, Some(window), cx)
        });
    }

    fn open_row_menu(
        &mut self,
        task_id: String,
        position: Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.focus.focus(window, cx);
        self.menu.open(RowMenu { task_id, position });
        cx.notify();
    }

    fn row_menu_listener(
        task_id: String,
        cx: &mut Context<Self>,
    ) -> impl Fn(&MouseDownEvent, &mut Window, &mut gpui::App) + 'static {
        cx.listener(move |page, event: &MouseDownEvent, window, cx| {
            cx.stop_propagation();
            window.prevent_default();
            page.open_row_menu(task_id.clone(), event.position, window, cx);
        })
    }

    fn close_row_menu(&mut self, cx: &mut Context<Self>) {
        if self.menu.begin_close() {
            popover::reap_popup(cx, |page: &mut Self| &mut page.menu);
            cx.notify();
        }
    }

    fn edit_row(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(task_id) = self.menu.as_open().map(|menu| menu.task_id.clone()) else {
            return;
        };
        self.close_row_menu(cx);
        self.card.update(cx, |card, cx| {
            card.open(task_id, None, CardFocus::Title, Some(window), cx)
        });
    }

    fn render_row_menu(&self, theme: &Theme, cx: &mut Context<Self>) -> Option<AnyElement> {
        let menu = self.menu.get()?;
        let theme = theme.for_popup();
        let card = popover::popover_card(&theme)
            .w(px(160.0))
            .flex()
            .flex_col()
            .on_mouse_down_out(cx.listener(|page, _, _, cx| page.close_row_menu(cx)))
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(
                popover::menu_row(&theme, false, "task-row-menu-edit")
                    .id("task-row-menu-edit")
                    .debug_selector(|| "task-row-menu-edit".into())
                    .role(gpui::Role::MenuItem)
                    .aria_label("Edit")
                    .on_click(
                        cx.listener(|page, _: &ClickEvent, window, cx| page.edit_row(window, cx)),
                    )
                    .child(icon(icons::PEN).size(px(16.0)).text_color(theme.text_muted))
                    .child("Edit"),
            );
        Some(popover::menu_at(
            "task-row-menu",
            menu.position,
            card.into_any_element(),
            self.menu.closing_since(),
        ))
    }

    fn update(&mut self, task_id: &str, patch: TaskPatch, cx: &mut Context<Self>) {
        cx.emit(TasksEvent::Update {
            task_id: task_id.to_string(),
            patch,
        });
    }

    fn set_completed(&mut self, task_id: &str, completed: bool, cx: &mut Context<Self>) {
        self.update(
            task_id,
            TaskPatch {
                completed: Some(completed),
                ..Default::default()
            },
            cx,
        );
    }

    fn open_ids(&self, cx: &gpui::App) -> Vec<String> {
        let (open, _) = zeron_proto::mission::split_tasks(&self.state.read(cx).tasks);
        let mut ids: Vec<String> = open.iter().map(|task| task.id.clone()).collect();
        if let Some((order, _)) = &self.pending {
            ids.sort_by_key(|id| {
                order
                    .iter()
                    .position(|known| known == id)
                    .unwrap_or(usize::MAX)
            });
        }
        ids
    }

    fn content_y(&self, window_y: f32) -> f32 {
        window_y - f32::from(self.scroll.offset().y)
    }

    fn begin_drag(&mut self, task_id: &str, now: Instant) -> Option<drag::ListDrag> {
        let offset = f32::from(self.scroll.offset().y);
        let rows = self
            .shown
            .iter()
            .map(|id| {
                let rect = self.rects.get(&Key::Task(id.clone()))?;
                let top = f32::from(rect.top()) - offset - self.shifts.offset(id, now);
                Some((top, f32::from(rect.size.height) + ROW_GAP))
            })
            .collect::<Option<Vec<_>>>()?;
        let primary = self.shown.iter().position(|id| id == task_id)?;
        drag::ListDrag::new(
            self.shown.clone(),
            rows,
            primary,
            &self.picked,
            self.content_y(self.press),
        )
    }

    fn drag_rows(&mut self, task_id: &str, y: f32, window: &mut Window, cx: &mut Context<Self>) {
        let now = motion::now();
        if self
            .drag
            .as_ref()
            .is_none_or(|drag| drag.primary_id() != task_id)
        {
            self.drag = self.begin_drag(task_id, now);
            if let Some(drag) = &self.drag {
                if drag.count() == 1 {
                    self.picked.clear();
                }
                window.focus(&self.focus, cx);
            }
        }
        let content = self.content_y(y);
        let Some(drag) = &mut self.drag else {
            return;
        };
        drag.pointer = y;
        drag.drag_to(content);
        self.aim_shifts(now, cx);
        self.autoscroll(cx);
        cx.notify();
    }

    fn aim_shifts(&mut self, now: Instant, cx: &gpui::App) {
        let Some(drag) = &self.drag else {
            return;
        };
        let mut targets = drag.shifts();
        let primary = drag.primary_id().to_string();
        if let Some(dy) = targets.remove(&primary) {
            self.shifts.snap(&primary, dy, now);
        }
        self.shifts
            .aim(&targets, drag::SHIFT, motion::reduced_motion(cx), now);
    }

    fn autoscroll(&mut self, cx: &mut Context<Self>) {
        if self.autoscroll.is_some() || self.scroll_speed() == 0.0 {
            return;
        }
        self.autoscroll = Some(cx.spawn(async move |page, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(16))
                    .await;
                let going = page
                    .update(cx, |page, cx| {
                        let speed = page.scroll_speed();
                        if speed == 0.0 {
                            page.autoscroll = None;
                            return false;
                        }
                        let offset = page.scroll.offset();
                        let max = f32::from(page.scroll.max_offset().y);
                        let y = (f32::from(offset.y) - speed).clamp(-max, 0.0);
                        page.scroll.set_offset(gpui::point(offset.x, px(y)));
                        let now = motion::now();
                        if let Some(drag) = &mut page.drag {
                            let pointer = drag.pointer;
                            drag.drag_to(pointer - y);
                        }
                        page.aim_shifts(now, cx);
                        cx.notify();
                        true
                    })
                    .unwrap_or(false);
                if !going {
                    break;
                }
            }
        }));
    }

    fn scroll_speed(&self) -> f32 {
        let Some(drag) = &self.drag else {
            return 0.0;
        };
        let bounds = self.scroll.bounds();
        let (top, bottom) = (f32::from(bounds.top()), f32::from(bounds.bottom()));
        let offset = f32::from(self.scroll.offset().y);
        let max = f32::from(self.scroll.max_offset().y);
        let speed = drag::edge_speed(drag.pointer, top, bottom);
        if (speed < 0.0 && offset >= 0.0) || (speed > 0.0 && -offset >= max) {
            0.0
        } else {
            speed
        }
    }

    fn drop_rows(&mut self, cx: &mut Context<Self>) {
        let Some(drag) = self.drag.take() else {
            return;
        };
        self.autoscroll = None;
        let now = motion::now();
        let reduced = motion::reduced_motion(cx);
        let shown = self.shifts.current(now);
        if drag.noop() {
            let home = drag.flip(&drag.ids, &shown);
            self.shifts.settle(&home, reduced, now);
            cx.notify();
            return;
        }
        let moving: Vec<String> = drag
            .order()
            .into_iter()
            .filter(|id| drag.is_moving(id))
            .collect();
        let global = self.open_ids(cx);
        let before = drag.before().or_else(|| {
            let last = drag.last_rest()?;
            let rest: Vec<&String> = global.iter().filter(|id| !moving.contains(id)).collect();
            let at = rest.iter().position(|id| **id == last)?;
            rest.get(at + 1).map(|id| (*id).clone())
        });
        let keys = {
            let state = self.state.read(cx);
            let (open, _) = zeron_proto::mission::split_tasks(&state.tasks);
            logic::reorder_keys(
                &open,
                &moving,
                before.as_deref(),
                &uuid::Uuid::new_v4().to_string(),
            )
        };
        if keys.is_empty() {
            let home = drag.flip(&drag.ids, &shown);
            self.shifts.settle(&home, reduced, now);
            cx.notify();
            return;
        }
        let mut order: Vec<String> = global
            .into_iter()
            .filter(|id| !moving.contains(id))
            .collect();
        let at = before
            .as_ref()
            .and_then(|before| order.iter().position(|id| id == before))
            .unwrap_or(order.len());
        order.splice(at..at, moving.iter().cloned());
        self.pending = Some((order, Instant::now()));
        let landed = drag.order();
        let from = drag.flip(&landed, &shown);
        self.shifts.settle(&from, reduced, now);
        self.shifts.fade_in(
            moving
                .iter()
                .filter(|id| **id != drag.primary_id())
                .cloned(),
            reduced,
            now,
        );
        for (task_id, key) in keys {
            self.update(
                &task_id,
                TaskPatch {
                    order_key: Some(key),
                    ..Default::default()
                },
                cx,
            );
        }
        cx.notify();
    }

    fn cancel_drag(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let Some(drag) = self.drag.take() else {
            return false;
        };
        cx.stop_active_drag(window);
        self.autoscroll = None;
        let now = motion::now();
        let home = drag.flip(&drag.ids, &self.shifts.current(now));
        self.shifts.settle(&home, motion::reduced_motion(cx), now);
        self.shifts.fade_in(
            drag.ids
                .iter()
                .filter(|id| drag.is_moving(id) && *id != drag.primary_id())
                .cloned(),
            motion::reduced_motion(cx),
            now,
        );
        cx.notify();
        true
    }

    fn settle_pending(&mut self, cx: &gpui::App) {
        let Some((order, since)) = &self.pending else {
            return;
        };
        let (open, _) = zeron_proto::mission::split_tasks(&self.state.read(cx).tasks);
        let now: Vec<&str> = open.iter().map(|task| task.id.as_str()).collect();
        let wanted: Vec<&str> = order
            .iter()
            .map(String::as_str)
            .filter(|id| now.contains(id))
            .collect();
        if wanted == now || since.elapsed() > PENDING_ORDER {
            self.pending = None;
        }
    }

    fn click_row(
        &mut self,
        task_id: &str,
        paged: bool,
        done: bool,
        modifiers: Modifiers,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !done && modifiers.platform {
            self.toggle_pick(task_id, cx);
            return;
        }
        if !done && modifiers.shift && self.anchor.is_some() {
            self.pick_range(task_id, cx);
            return;
        }
        self.clear_picks(cx);
        self.anchor = Some(task_id.to_string());
        self.open_row(task_id, paged, window, cx);
    }

    fn toggle_pick(&mut self, task_id: &str, cx: &mut Context<Self>) {
        match self.picked.iter().position(|id| id == task_id) {
            Some(at) => {
                self.picked.remove(at);
            }
            None => self.picked.push(task_id.to_string()),
        }
        self.anchor = Some(task_id.to_string());
        self.thread_picker = false;
        cx.notify();
    }

    fn pick_range(&mut self, task_id: &str, cx: &mut Context<Self>) {
        let Some(anchor) = self.anchor.clone() else {
            return;
        };
        let at = |id: &str| self.shown.iter().position(|known| known == id);
        let (Some(a), Some(b)) = (at(&anchor), at(task_id)) else {
            return;
        };
        for id in &self.shown[a.min(b)..=a.max(b)] {
            if !self.picked.contains(id) {
                self.picked.push(id.clone());
            }
        }
        cx.notify();
    }

    fn clear_picks(&mut self, cx: &mut Context<Self>) {
        if !self.picked.is_empty() || self.thread_picker {
            self.picked.clear();
            self.thread_picker = false;
            cx.notify();
        }
    }

    fn ordered_picks(&self) -> Vec<String> {
        self.shown
            .iter()
            .filter(|id| self.picked.contains(id))
            .cloned()
            .collect()
    }

    fn gather(&mut self, chat_id: Option<String>, cx: &mut Context<Self>) {
        let task_ids = self.ordered_picks();
        if task_ids.is_empty() {
            return;
        }
        self.picked.clear();
        self.thread_picker = false;
        cx.emit(TasksEvent::Gather { task_ids, chat_id });
        cx.notify();
    }

    fn drop_on_column(&mut self, drag: &TaskDrag, column: TaskStatus, cx: &mut Context<Self>) {
        if let Some(completed) = logic::drop_completes(drag.status, column) {
            self.set_completed(&drag.task_id, completed, cx);
        }
    }

    fn rows(&self, cx: &gpui::App) -> (Vec<Row>, Vec<Row>) {
        let state = self.state.read(cx);
        let now = Utc::now();
        let (mut open, done) = zeron_proto::mission::split_tasks(&state.tasks);
        if let Some((order, _)) = &self.pending {
            open.sort_by_key(|task| {
                order
                    .iter()
                    .position(|id| *id == task.id)
                    .unwrap_or(usize::MAX)
            });
        }
        (
            open.into_iter()
                .map(|task| row_for(state, task, now))
                .collect(),
            done.into_iter()
                .map(|task| row_for(state, task, now))
                .collect(),
        )
    }

    fn subtitle(open: usize) -> String {
        match open {
            0 => "Nothing to do".into(),
            n => format!("{n} to do"),
        }
    }

    fn measured_header(&self, open: usize, morphing: bool, theme: &Theme) -> gpui::Div {
        Self::header(theme, open)
            .when(morphing, |el| el.opacity(0.0))
            .child(morph::record(&self.painted, Key::Header, Edges::default()))
    }

    fn header(theme: &Theme, open: usize) -> gpui::Div {
        div()
            .flex()
            .flex_col()
            .gap(px(4.0))
            .px(px(12.0))
            .child(
                div()
                    .text_size(ui_rems(24.0))
                    .line_height(ui_rems(30.0))
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .text_color(theme.text)
                    .child("Tasks"),
            )
            .child(
                div()
                    .debug_selector(|| "tasks-subtitle".into())
                    .text_size(ui_rems(13.0))
                    .text_color(theme.text_muted)
                    .child(Self::subtitle(open)),
            )
    }

    fn check_visual(row: &Row, theme: &Theme) -> gpui::Div {
        let done = row.status == TaskStatus::Done;
        let ring = theme.text_faint;
        let progress = (row.orchestrated() && !done).then(|| {
            let PlanProgress { done, total } = row.plan.progress;
            let percent = (done * 100).checked_div(total).unwrap_or(0).min(100) as u8;
            let (_, _, complete) = column_tone(TaskStatus::Done, theme);
            crate::loaders::upload_progress_arc(percent, CHECK_SIZE, ring.opacity(0.6), complete)
        });
        let plain = !done && progress.is_none();
        div()
            .flex_none()
            .size(px(CHECK_SIZE))
            .rounded_full()
            .flex()
            .items_center()
            .justify_center()
            .when(done, |el| {
                el.bg(theme.accent)
                    .child(icon(icons::CHECK).size(px(12.0)).text_color(gpui::white()))
            })
            .when(plain, |el| el.border_1().border_color(ring))
            .children(progress)
    }

    fn checkbox(&self, row: &Row, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let done = row.status == TaskStatus::Done;
        let task_id = row.task.id.clone();
        let accent = theme.accent;
        let ringed = row.orchestrated() && !done;
        let ring_id = task_id.clone();
        Self::check_visual(row, theme)
            .id(SharedString::from(format!("task-check-{task_id}")))
            .when(ringed, |el| {
                el.debug_selector(move || format!("task-ring-{ring_id}"))
            })
            .role(gpui::Role::CheckBox)
            .aria_label(if done {
                "Mark as not done"
            } else {
                "Mark as done"
            })
            .cursor_pointer()
            .when(!done && !ringed, |el| {
                el.hover(move |s| s.border_color(accent).bg(accent.opacity(0.12)))
            })
            .when(ringed, |el| el.hover(move |s| s.bg(accent.opacity(0.12))))
            .on_click(cx.listener(move |page, _: &ClickEvent, _, cx| {
                cx.stop_propagation();
                page.set_completed(&task_id, !done, cx);
            }))
            .into_any_element()
    }

    fn marks(marks: &[Brand], theme: &Theme) -> AnyElement {
        div()
            .flex()
            .items_center()
            .children(marks.iter().enumerate().map(|(ix, (mark, tint))| {
                div()
                    .when(ix > 0, |el| el.ml(px(-3.0)))
                    .rounded_full()
                    .bg(theme.bg)
                    .child(
                        icon(mark)
                            .size(px(12.0))
                            .text_color(tint.unwrap_or(theme.text_muted)),
                    )
            }))
            .into_any_element()
    }

    fn step_counts(plan: &Rollup, theme: &Theme, fragments: &mut Vec<AnyElement>) {
        let running = plan.count(StepStatus::Working);
        if running > 0 {
            let (_, _, tone) = column_tone(TaskStatus::Working, theme);
            fragments.push(
                div()
                    .text_color(tone)
                    .child(format!("{running} running"))
                    .into_any_element(),
            );
        }
        let need_you = plan.count(StepStatus::NeedsYou);
        if need_you > 0 {
            fragments.push(
                div()
                    .text_color(theme.warning)
                    .child(format!("{need_you} need you"))
                    .into_any_element(),
            );
        }
    }

    fn not_sent(theme: &Theme) -> AnyElement {
        div()
            .text_color(theme.warning)
            .child("Not sent")
            .into_any_element()
    }

    fn plan_meta(row: &Row, theme: &Theme, fragments: &mut Vec<AnyElement>) {
        fragments.push(tree::kind_badge(true, theme));
        let marks = row.marks();
        if !marks.is_empty() {
            fragments.push(Self::marks(&marks, theme));
        }
        let (label, tone) = row.plan.label(&row.task);
        let (_, _, tone) = column_tone(tone, theme);
        fragments.push(div().text_color(tone).child(label).into_any_element());
        if !row.plan.steps.is_empty() {
            let PlanProgress { done, total } = row.plan.progress;
            fragments.push(
                div()
                    .child(format!("{done}/{total} steps"))
                    .into_any_element(),
            );
        }
        Self::step_counts(&row.plan, theme, fragments);
    }

    fn meta(row: &Row, today: NaiveDate, theme: &Theme) -> Vec<AnyElement> {
        let mut fragments = Vec::new();
        if let Some(project) = row.project.clone() {
            fragments.push(div().truncate().child(project).into_any_element());
        }
        if let Some(due) = row
            .task
            .due_on
            .as_deref()
            .and_then(|due| logic::due_label(due, today))
        {
            let late =
                row.status != TaskStatus::Done && logic::overdue(row.task.due_on.as_deref(), today);
            fragments.push(
                div()
                    .when(late, |el| el.text_color(theme.danger))
                    .child(due)
                    .into_any_element(),
            );
        }
        if row.orchestrated() {
            Self::plan_meta(row, theme, &mut fragments);
        } else if let Some(linked) = row
            .linked
            .as_ref()
            .filter(|_| row.status != TaskStatus::Done)
        {
            let (_, _, tone) = crate::shell::chat_status_tone(linked.indicator, theme);
            let (mark, tint) = linked.brand;
            let label = match row.task.send_error {
                Some(_) => Self::not_sent(theme),
                None => div()
                    .text_color(tone)
                    .child(linked_label(linked.indicator))
                    .into_any_element(),
            };
            fragments.push(
                div()
                    .min_w_0()
                    .flex()
                    .items_center()
                    .gap(px(5.0))
                    .child(
                        icon(mark)
                            .flex_none()
                            .size(px(12.0))
                            .text_color(tint.unwrap_or(theme.text_muted)),
                    )
                    .child(
                        div()
                            .debug_selector(|| format!("task-row-thread-{}", row.task.id))
                            .max_w(px(220.0))
                            .truncate()
                            .child(linked.title.clone()),
                    )
                    .into_any_element(),
            );
            fragments.push(label);
        } else if row.task.chat_id.is_some() && row.linked.is_none() {
            fragments.push(div().child("Chat deleted").into_any_element());
        }
        fragments
    }

    fn row_content(
        row: &Row,
        check: AnyElement,
        selected: bool,
        today: NaiveDate,
        theme: &Theme,
    ) -> gpui::Div {
        let done = row.status == TaskStatus::Done;
        let meta = Self::meta(row, today, theme);
        div()
            .min_h(px(ROW_HEIGHT))
            .px(px(12.0))
            .py(px(6.0))
            .flex()
            .items_center()
            .gap(px(12.0))
            .rounded(px(12.0))
            .border_t_2()
            .border_color(gpui::transparent_black())
            .when(selected, |el| el.bg(theme.wash(0.08)))
            .child(check)
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .child(
                        div()
                            .truncate()
                            .text_size(ui_rems(13.5))
                            .line_height(ui_rems(18.0))
                            .text_color(if done { theme.text_muted } else { theme.text })
                            .when(done, |el| el.line_through())
                            .child(SharedString::from(row.task.title.clone())),
                    )
                    .when(!meta.is_empty(), |el| {
                        el.child(widgets::meta_line(theme, meta))
                    }),
            )
    }

    fn list_row(
        &self,
        row: &Row,
        today: NaiveDate,
        morphing: bool,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let task_id = row.task.id.clone();
        let done = row.status == TaskStatus::Done;
        let selected = self.card.read(cx).task_id() == Some(task_id.as_str());
        let picked = !done && self.picked.contains(&task_id);
        let hover = theme.glass_hover();
        let open_id = task_id.clone();
        let menu_id = task_id.clone();
        let paged = row.linked.is_some();
        let now = motion::now();
        let lifted = self
            .drag
            .as_ref()
            .filter(|drag| drag.primary_id() == task_id);
        let hidden = self
            .drag
            .as_ref()
            .is_some_and(|drag| drag.is_moving(&task_id) && drag.primary_id() != task_id);
        let lane_key = SharedString::from(format!("task-lane-{task_id}"));
        let gutter = (!done).then(|| {
            self.row_gutter(
                row,
                motion::hover_t(&lane_key),
                picked,
                lifted.is_some(),
                theme,
                cx,
            )
        });
        let check = self.checkbox(row, theme, cx);
        let thumbs = self.row_images("task-row", row, theme, cx);
        let content = Self::row_content(row, check, selected, today, theme)
            .children(thumbs)
            .id(SharedString::from(format!("task-row-{task_id}")))
            .debug_selector(|| format!("task-row-{}", row.task.id))
            .role(gpui::Role::ListItem)
            .aria_selected(picked)
            .relative()
            .flex_1()
            .min_w_0()
            .cursor_pointer()
            .when(picked, |el| el.bg(theme.accent.opacity(0.16)))
            .when(!selected && !picked && self.drag.is_none(), |el| {
                el.hover(move |s| s.bg(hover))
            })
            .when(morphing, |el| el.opacity(0.0))
            .when_some(lifted, |el, drag| {
                el.bg(theme.surface_raised)
                    .shadow(vec![
                        gpui::BoxShadow {
                            color: theme.border_strong,
                            offset: gpui::point(px(0.0), px(0.0)),
                            blur_radius: px(0.0),
                            spread_radius: px(1.0),
                            inset: false,
                        },
                        gpui::BoxShadow {
                            color: gpui::black().opacity(0.35),
                            offset: gpui::point(px(0.0), px(12.0)),
                            blur_radius: px(28.0),
                            spread_radius: px(0.0),
                            inset: false,
                        },
                    ])
                    .when(drag.count() > 1, |el| {
                        el.child(Self::drag_count(drag.count(), theme))
                    })
            })
            .child(morph::record(
                &self.painted,
                Key::Task(task_id.clone()),
                ROW_BORDER,
            ))
            .on_click(cx.listener(move |page, event: &ClickEvent, window, cx| {
                page.click_row(&open_id, paged, done, event.modifiers(), window, cx)
            }))
            .on_mouse_down(MouseButton::Right, Self::row_menu_listener(menu_id, cx));
        let lane = div()
            .id(lane_key.clone())
            .relative()
            .top(px(self.shifts.offset(&task_id, now)))
            .flex()
            .items_center()
            .ml(px(-GUTTER))
            .pl(px(GUTTER))
            .opacity(if hidden {
                0.0
            } else {
                self.shifts.opacity(&task_id, now)
            })
            .on_hover(motion::hover_listener(lane_key))
            .children(gutter)
            .child(content);
        if lifted.is_some() {
            gpui::deferred(lane).with_priority(1).into_any_element()
        } else {
            lane.into_any_element()
        }
    }

    fn drag_count(count: usize, theme: &Theme) -> AnyElement {
        div()
            .debug_selector(|| "task-drag-count".into())
            .absolute()
            .top(px(-8.0))
            .right(px(-8.0))
            .min_w(px(20.0))
            .h(px(20.0))
            .px(px(6.0))
            .rounded_full()
            .bg(theme.accent)
            .flex()
            .items_center()
            .justify_center()
            .text_size(ui_rems(11.0))
            .font_weight(gpui::FontWeight::SEMIBOLD)
            .text_color(theme.on_accent)
            .child(count.to_string())
            .into_any_element()
    }

    fn row_gutter(
        &self,
        row: &Row,
        hover: f32,
        picked: bool,
        lifted: bool,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> gpui::Div {
        let task_id = row.task.id.clone();
        let pick_t = if picked || !self.picked.is_empty() {
            1.0
        } else {
            hover
        };
        let grip_t = match (lifted, self.drag.is_some()) {
            (true, _) => 1.0,
            (false, true) => 0.0,
            (false, false) => hover,
        };
        let pick_id = task_id.clone();
        let drag = TaskDrag {
            task_id: task_id.clone(),
            title: SharedString::from(row.task.title.clone()),
            status: row.status,
        };
        let faint = theme.text_faint;
        let grip_hover = theme.glass_hover();
        let check = div()
            .id(SharedString::from(format!("task-pick-{task_id}")))
            .debug_selector(move || format!("task-pick-{pick_id}"))
            .role(gpui::Role::CheckBox)
            .aria_label("Select")
            .aria_selected(picked)
            .flex_none()
            .ml(px(4.0))
            .size(px(18.0))
            .rounded(px(5.0))
            .border_1()
            .border_color(if picked { theme.accent } else { faint })
            .bg(if picked { theme.accent } else { theme.bg })
            .flex()
            .items_center()
            .justify_center()
            .cursor_pointer()
            .opacity(pick_t)
            .when(picked, |el| {
                el.child(
                    icon(icons::CHECK)
                        .size(px(11.0))
                        .text_color(theme.on_accent),
                )
            })
            .on_click(cx.listener({
                let task_id = task_id.clone();
                move |page, _: &ClickEvent, _, cx| {
                    cx.stop_propagation();
                    page.toggle_pick(&task_id, cx);
                }
            }));
        let grip_id = task_id.clone();
        let grip = div()
            .id(SharedString::from(format!("task-grip-{task_id}")))
            .debug_selector(move || format!("task-grip-{grip_id}"))
            .role(gpui::Role::Button)
            .aria_label("Drag to reorder")
            .flex_none()
            .ml(px(8.0))
            .w(px(20.0))
            .h(px(24.0))
            .rounded(px(6.0))
            .flex()
            .items_center()
            .justify_center()
            .opacity(grip_t)
            .cursor(gpui::CursorStyle::OpenHand)
            .hover(move |s| s.bg(grip_hover))
            .child(icon(icons::DRAG_HANDLE).size(px(16.0)).text_color(faint))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|page, event: &MouseDownEvent, _, _| {
                    page.press = f32::from(event.position.y);
                }),
            )
            .on_click(|_, _, cx| cx.stop_propagation())
            .on_drag(drag, |_, _, _, cx| cx.new(|_| LiftedRowGhost));
        div()
            .absolute()
            .left_0()
            .top_0()
            .bottom_0()
            .w(px(GUTTER))
            .flex()
            .items_center()
            .child(check)
            .child(grip)
    }

    #[allow(clippy::too_many_arguments)]
    fn render_selection_bar(
        &mut self,
        view: TasksView,
        reduced: bool,
        now: Instant,
        theme: &Theme,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let show = view == TasksView::List && !self.picked.is_empty();
        if show {
            self.bar_count = self.picked.len();
        } else {
            self.thread_picker = false;
        }
        let target = if show { 1.0 } else { 0.0 };
        if self.bar.target() != target {
            let spec = if reduced { drag::REDUCED_FADE } else { BAR_IN };
            self.bar.retarget(target, spec, false, now);
        }
        if !self.bar.settled_at(now) {
            window.request_animation_frame();
        }
        let t = self.bar.value_at(now);
        if t <= 0.0 {
            return None;
        }
        let theme = theme.for_popup();
        let rise = if reduced { 0.0 } else { BAR_RISE * (1.0 - t) };
        let button = |id: &'static str, label: &'static str| {
            div()
                .id(id)
                .debug_selector(move || id.into())
                .role(gpui::Role::Button)
                .aria_label(label)
                .h(px(30.0))
                .px(px(10.0))
                .rounded(px(8.0))
                .flex()
                .items_center()
                .cursor_pointer()
                .child(label)
        };
        let hover = theme.glass_hover();
        let primary = theme.text;
        let bar = div()
            .debug_selector(|| "tasks-selection-bar".into())
            .h(px(44.0))
            .pl(px(14.0))
            .pr(px(6.0))
            .flex()
            .items_center()
            .gap(px(6.0))
            .whitespace_nowrap()
            .text_size(ui_rems(13.0))
            .text_color(theme.text)
            .child(
                div()
                    .debug_selector(|| "tasks-selection-count".into())
                    .min_w(px(70.0))
                    .mr(px(6.0))
                    .font_weight(gpui::FontWeight::MEDIUM)
                    .child(format!("{} selected", self.bar_count)),
            )
            .child(
                button("tasks-gather-start", "Start in one thread")
                    .bg(primary)
                    .text_color(theme.bg)
                    .hover(move |s| s.bg(primary.opacity(0.88)))
                    .on_click(cx.listener(|page, _: &ClickEvent, _, cx| page.gather(None, cx))),
            )
            .child(
                button("tasks-gather-add", "Add to thread…")
                    .when(self.thread_picker, |el| el.bg(hover))
                    .hover(move |s| s.bg(hover))
                    .on_click(cx.listener(|page, _: &ClickEvent, _, cx| {
                        page.thread_picker = !page.thread_picker;
                        cx.notify();
                    })),
            )
            .child(
                button("tasks-gather-clear", "Clear")
                    .text_color(theme.text_muted)
                    .hover(move |s| s.bg(hover))
                    .on_click(cx.listener(|page, _: &ClickEvent, _, cx| page.clear_picks(cx))),
            );
        let card = div()
            .rounded(px(12.0))
            .border_1()
            .border_color(theme.border_strong)
            .bg(popover::surface_bg(&theme))
            .shadow(vec![gpui::BoxShadow {
                color: gpui::black().opacity(0.3),
                offset: gpui::point(px(0.0), px(12.0)),
                blur_radius: px(32.0),
                spread_radius: px(0.0),
                inset: false,
            }])
            .child(bar);
        let picker = (self.thread_picker && show).then(|| self.render_thread_picker(&theme, cx));
        Some(
            div()
                .absolute()
                .left_0()
                .right_0()
                .bottom(px(22.0 - rise))
                .flex()
                .justify_center()
                .opacity(t)
                .child(
                    div()
                        .relative()
                        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .children(picker)
                        .child(crate::frost::frosted(
                            12.0,
                            crate::frost::MENU_BLUR * t,
                            card,
                        )),
                )
                .into_any_element(),
        )
    }

    fn render_thread_picker(&self, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let state = self.state.read(cx);
        let space = self
            .picked
            .first()
            .and_then(|id| state.tasks.iter().find(|task| task.id == *id))
            .and_then(|task| task.space_id.clone());
        let threads: Vec<(String, String, Brand)> = match state.local_device_id.clone() {
            Some(device) => crate::composer::thread_matches(
                state.visible_chats(),
                "",
                crate::composer::MentionScope {
                    device_id: &device,
                    space_id: space.as_deref(),
                    exclude: None,
                },
                |id| state.is_draft(id),
                |_| String::new(),
                |_| Vec::new(),
            )
            .into_iter()
            .filter_map(|thread| {
                let chat = state.chats.iter().find(|chat| chat.id == thread.chat_id)?;
                Some((thread.chat_id, thread.title, chat_brand(chat)))
            })
            .collect(),
            None => Vec::new(),
        };
        let now = Utc::now();
        let statuses: Vec<Option<&'static str>> = threads
            .iter()
            .map(|(id, ..)| {
                state
                    .chats
                    .iter()
                    .find(|chat| chat.id == *id)
                    .and_then(|chat| {
                        zeron_proto::chat_mentions::thread_status_label(
                            state.display_status_for(chat, now),
                        )
                    })
            })
            .collect();
        let mut rows: Vec<AnyElement> =
            vec![popover::menu_heading(theme, "Add to thread").into_any_element()];
        for (ix, ((chat_id, title, (mark, tint)), status)) in
            threads.into_iter().zip(statuses).enumerate()
        {
            rows.push(
                popover::menu_row(theme, false, format!("tasks-thread-pick-{ix}"))
                    .id(("tasks-thread-pick", ix))
                    .debug_selector(move || format!("tasks-thread-pick-{ix}"))
                    .gap(px(8.0))
                    .on_click(cx.listener(move |page, _: &ClickEvent, _, cx| {
                        page.gather(Some(chat_id.clone()), cx)
                    }))
                    .child(
                        icon(mark)
                            .flex_none()
                            .size(px(14.0))
                            .text_color(tint.unwrap_or(theme.text_muted)),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_size(px(13.0))
                            .text_color(theme.text)
                            .child(title),
                    )
                    .children(status.map(|status| {
                        div()
                            .flex_none()
                            .text_size(ui_rems(11.5))
                            .text_color(theme.text_faint)
                            .child(status)
                    }))
                    .into_any_element(),
            );
        }
        if rows.len() == 1 {
            rows.push(
                div()
                    .px(px(8.0))
                    .py(px(6.0))
                    .text_size(ui_rems(12.0))
                    .text_color(theme.text_muted)
                    .child("No threads on this device yet")
                    .into_any_element(),
            );
        }
        rows.push(
            div()
                .mt(px(4.0))
                .px(px(8.0))
                .pt(px(6.0))
                .border_t_1()
                .border_color(theme.border)
                .text_size(ui_rems(11.0))
                .text_color(theme.text_faint)
                .child("A running thread gets them when its turn ends")
                .into_any_element(),
        );
        div()
            .debug_selector(|| "tasks-thread-picker".into())
            .absolute()
            .bottom(px(52.0))
            .left(px(40.0))
            .w(px(300.0))
            .child(crate::frost::frosted(
                12.0,
                crate::frost::MENU_BLUR,
                popover::popover_card(theme)
                    .p(px(6.0))
                    .flex()
                    .flex_col()
                    .children(rows),
            ))
            .into_any_element()
    }

    fn row_images(
        &self,
        key: &str,
        row: &Row,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> Option<gpui::Div> {
        let paths = row.task.images.clone();
        (!paths.is_empty()).then(|| {
            let open = paths.clone();
            images::strip(
                &format!("{key}-{}", row.task.id),
                &paths,
                images::ROW_THUMB,
                &self.state,
                theme,
                move |page: &mut Self, ix, _, cx| page.open_images(open.clone(), ix, cx),
                None::<fn(&mut Self, usize, &mut Window, &mut Context<Self>)>,
                cx,
            )
            .flex_none()
            .flex_nowrap()
        })
    }

    fn add_tray(&self, theme: &Theme, cx: &mut Context<Self>) -> Option<AnyElement> {
        if self.add_images.is_empty() {
            return None;
        }
        let paths = self.add_images.clone();
        let open = paths.clone();
        Some(
            div()
                .debug_selector(|| "tasks-add-tray".into())
                .pl(px(12.0 + CHECK_SIZE + 12.0))
                .pb(px(6.0))
                .child(images::strip(
                    "tasks-add",
                    &paths,
                    images::SPEC_THUMB,
                    &self.state,
                    theme,
                    move |page: &mut Self, ix, _, cx| page.open_images(open.clone(), ix, cx),
                    Some(
                        |page: &mut Self, ix: usize, _: &mut Window, cx: &mut Context<Self>| {
                            if ix < page.add_images.len() {
                                page.add_images.remove(ix);
                                cx.notify();
                            }
                        },
                    ),
                    cx,
                ))
                .into_any_element(),
        )
    }

    fn add_ghost(&self, theme: &Theme, cx: &gpui::App) -> gpui::Div {
        let text = self.add.read(cx).text().to_string();
        let empty = text.is_empty();
        div()
            .min_h(px(ROW_HEIGHT))
            .px(px(12.0))
            .flex()
            .items_center()
            .gap(px(12.0))
            .border_t_2()
            .border_color(gpui::transparent_black())
            .child(dashed_circle(theme))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_size(ui_rems(13.5))
                    .text_color(if empty { theme.text_faint } else { theme.text })
                    .child(if empty {
                        ADD_PLACEHOLDER.to_string()
                    } else {
                        text
                    }),
            )
    }

    fn add_row(&self, fade: Option<f32>, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let accent = theme.accent;
        div()
            .id("tasks-add-row")
            .debug_selector(|| "tasks-add-row".into())
            .min_h(px(ROW_HEIGHT))
            .px(px(12.0))
            .flex()
            .items_center()
            .gap(px(12.0))
            .rounded(px(12.0))
            .border_t_2()
            .border_color(gpui::transparent_black())
            .when_some(fade, |el, fade| el.opacity(fade))
            .child(morph::record(&self.painted, Key::Add, ROW_BORDER))
            .drag_over::<gpui::ExternalPaths>(move |style, _, _, _| style.border_color(accent))
            .on_drop(cx.listener(|page, paths: &gpui::ExternalPaths, _, cx| {
                cx.stop_propagation();
                let staged = images::staged_files(paths.paths(), cx);
                page.add_images(staged, cx);
            }))
            .capture_action(cx.listener(|page, _: &MentionTab, window, cx| {
                if page.submit_add(true, Some(window), cx) {
                    cx.stop_propagation();
                }
            }))
            .on_key_down(cx.listener(|page, event: &KeyDownEvent, window, cx| {
                if event.keystroke.key == "escape"
                    && (!page.add.read(cx).text().is_empty() || !page.add_images.is_empty())
                {
                    cx.stop_propagation();
                    page.add.update(cx, |input, cx| input.set_text("", cx));
                    page.add_images.clear();
                    window.focus(&page.focus, cx);
                }
            }))
            .child(dashed_circle(theme))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .text_size(ui_rems(13.5))
                    .child(self.add.clone()),
            )
            .child(
                div()
                    .id("tasks-add-image")
                    .debug_selector(|| "tasks-add-image".into())
                    .role(gpui::Role::Button)
                    .aria_label("Add image")
                    .flex_none()
                    .size(px(26.0))
                    .rounded(px(6.0))
                    .flex()
                    .items_center()
                    .justify_center()
                    .cursor_pointer()
                    .hover(|s| s.bg(theme.wash(0.08)))
                    .tooltip(widgets::text_tooltip("Add image"))
                    .on_click(cx.listener(|page, _: &ClickEvent, _, cx| page.pick_add_images(cx)))
                    .child(
                        icon(icons::GALLERY)
                            .size(px(15.0))
                            .text_color(theme.text_faint),
                    ),
            )
            .into_any_element()
    }

    fn done_toggle(open: bool, count: usize, theme: &Theme) -> gpui::Div {
        div()
            .px(px(12.0))
            .py(px(6.0))
            .flex()
            .items_center()
            .gap(px(6.0))
            .rounded(px(8.0))
            .text_size(ui_rems(12.5))
            .font_weight(gpui::FontWeight::MEDIUM)
            .text_color(theme.text_muted)
            .child(
                icon(if open {
                    icons::LUCIDE_CHEVRON_DOWN
                } else {
                    icons::LUCIDE_CHEVRON_RIGHT
                })
                .size(px(14.0))
                .text_color(theme.text_faint),
            )
            .child("Done")
            .child(
                div()
                    .text_color(theme.text_faint)
                    .child(SharedString::from(count.to_string())),
            )
    }

    fn tree_row(
        &self,
        row: &index::IndexRow,
        reduced: bool,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let root = row.root.clone();
        let mut meta = vec![
            tree::kind_badge(false, theme),
            div()
                .when(row.asks, |el| el.text_color(theme.warning))
                .child(row.line.clone())
                .into_any_element(),
        ];
        if let Some(agents) = row.agents_line() {
            meta.push(div().child(agents).into_any_element());
        }
        let hover = theme.glass_hover();
        div()
            .id(SharedString::from(format!("task-tree-row-{root}")))
            .debug_selector(|| format!("task-tree-row-{}", row.root))
            .role(gpui::Role::ListItem)
            .aria_label(format!("{}: {}", row.title, row.line))
            .min_h(px(ROW_HEIGHT))
            .px(px(12.0))
            .py(px(6.0))
            .flex()
            .items_center()
            .gap(px(12.0))
            .rounded(px(12.0))
            .border_t_2()
            .border_color(gpui::transparent_black())
            .cursor_pointer()
            .hover(move |s| s.bg(hover))
            .child(
                div()
                    .flex_none()
                    .size(px(CHECK_SIZE))
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(tree::glyph(
                        row.status,
                        15.0,
                        row.status.tone(theme),
                        reduced,
                    )),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .child(
                        div()
                            .truncate()
                            .text_size(ui_rems(13.5))
                            .line_height(ui_rems(18.0))
                            .text_color(theme.text)
                            .child(row.title.clone()),
                    )
                    .child(widgets::meta_line(theme, meta)),
            )
            .on_click(cx.listener(move |_, _: &ClickEvent, _, cx| {
                cx.emit(TasksEvent::OpenCenter(root.clone()))
            }))
            .into_any_element()
    }

    fn filter_chips(
        &self,
        counts: Vec<(TasksFilter, usize)>,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let chips = counts.into_iter().map(|(filter, n)| {
            let on = self.filter == filter;
            let label = filter.label();
            let selector = format!("tasks-filter-{label}");
            div()
                .id(SharedString::from(selector.clone()))
                .debug_selector(move || selector.clone())
                .role(gpui::Role::Button)
                .aria_label(format!("{label} {n}"))
                .aria_selected(on)
                .flex()
                .items_center()
                .gap(px(6.0))
                .h(px(26.0))
                .px(px(10.0))
                .rounded(px(8.0))
                .border_1()
                .border_color(if on {
                    theme.border_strong
                } else {
                    theme.border
                })
                .when(on, |el| el.bg(theme.wash(0.06)))
                .cursor_pointer()
                .text_size(ui_rems(12.0))
                .text_color(if on { theme.text } else { theme.text_muted })
                .child(label)
                .child(
                    div()
                        .text_color(if filter == TasksFilter::NeedsYou && n > 0 {
                            theme.warning
                        } else {
                            theme.text_faint
                        })
                        .child(n.to_string()),
                )
                .on_click(cx.listener(move |page, _: &ClickEvent, _, cx| {
                    page.set_filter(filter, cx);
                }))
        });
        div()
            .debug_selector(|| "tasks-filters".into())
            .flex()
            .items_center()
            .gap(px(8.0))
            .px(px(12.0))
            .children(chips)
            .into_any_element()
    }

    fn done_group(
        &self,
        done: &[Row],
        trees: Vec<AnyElement>,
        today: NaiveDate,
        morph: Option<f32>,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        if done.is_empty() && trees.is_empty() {
            return None;
        }
        let open = self.done_open;
        let text = theme.text;
        let toggle = Self::done_toggle(open, done.len() + trees.len(), theme)
            .id("tasks-done-toggle")
            .debug_selector(|| "tasks-done-toggle".into())
            .role(gpui::Role::Button)
            .aria_expanded(open)
            .cursor_pointer()
            .hover(move |s| s.text_color(text))
            .when_some(morph, |el, elapsed| {
                el.opacity(morph::extra_t(elapsed, true))
            })
            .child(morph::record(&self.painted, Key::Done, Edges::default()))
            .on_click(cx.listener(|page, _: &ClickEvent, _, cx| {
                page.done_open = !page.done_open;
                cx.notify();
            }));
        let rows: Vec<AnyElement> = if open {
            done.iter()
                .map(|row| self.list_row(row, today, morph.is_some(), theme, cx))
                .chain(trees)
                .collect()
        } else {
            Vec::new()
        };
        Some(
            div()
                .flex()
                .flex_col()
                .gap(px(2.0))
                .child(toggle)
                .children(rows)
                .into_any_element(),
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn render_list(
        &self,
        open: &[Row],
        done: &[Row],
        trees: &index::IndexRows,
        chips: AnyElement,
        morph: Option<f32>,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let today = Self::today();
        let reduced = motion::reduced_motion(cx);
        let rows: Vec<AnyElement> = open
            .iter()
            .map(|row| self.list_row(row, today, morph.is_some(), theme, cx))
            .collect();
        let tree_rows: Vec<AnyElement> = trees
            .running
            .iter()
            .map(|row| self.tree_row(row, reduced, theme, cx))
            .collect();
        let done_trees: Vec<AnyElement> = trees
            .done_today
            .iter()
            .map(|row| self.tree_row(row, reduced, theme, cx))
            .collect();
        let adding = self.filter == TasksFilter::All;
        let empty = !adding && rows.is_empty() && tree_rows.is_empty();
        let column = div()
            .w_full()
            .max_w(px(LIST_MAX_WIDTH))
            .mx_auto()
            .px(px(28.0))
            .pt(px(Theme::TITLEBAR_HEIGHT + 28.0))
            .pb(px(64.0))
            .flex()
            .flex_col()
            .gap(px(24.0))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(16.0))
                    .child(self.measured_header(open.len(), morph.is_some(), theme))
                    .child(chips),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(ROW_GAP))
                    .child(
                        div()
                            .id("tasks-open-rows")
                            .flex()
                            .flex_col()
                            .gap(px(ROW_GAP))
                            .children(rows)
                            .on_drag_move::<TaskDrag>(cx.listener(
                                |page, event: &DragMoveEvent<TaskDrag>, window, cx| {
                                    let task_id = event.drag(cx).task_id.clone();
                                    page.drag_rows(
                                        &task_id,
                                        f32::from(event.event.position.y),
                                        window,
                                        cx,
                                    );
                                },
                            ))
                            .on_drop(cx.listener(|page, _: &TaskDrag, _, cx| page.drop_rows(cx)))
                            .on_mouse_up_out(
                                MouseButton::Left,
                                cx.listener(|page, _, _, cx| page.drop_rows(cx)),
                            ),
                    )
                    .when(adding, |el| {
                        el.child(self.add_row(
                            morph.map(|elapsed| morph::extra_t(elapsed, true)),
                            theme,
                            cx,
                        ))
                        .children(self.add_tray(theme, cx))
                    })
                    .children(tree_rows)
                    .when(empty, |el| {
                        el.child(
                            div()
                                .debug_selector(|| "tasks-filter-empty".into())
                                .px(px(12.0))
                                .py(px(10.0))
                                .text_size(ui_rems(13.0))
                                .text_color(theme.text_muted)
                                .child("Nothing matches this filter."),
                        )
                    }),
            )
            .children(self.done_group(done, done_trees, today, morph, theme, cx));
        widgets::scroll_faded(
            "tasks-list-scroll",
            div()
                .id("tasks-list")
                .debug_selector(|| "tasks-list".into())
                .size_full()
                .overflow_y_scroll()
                .track_scroll(&self.scroll)
                .child(column),
        )
        .into_any_element()
    }

    fn kanban_card(
        &self,
        row: &Row,
        morphing: bool,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let task_id = row.task.id.clone();
        let selected = self.card.read(cx).task_id() == Some(task_id.as_str());
        let drag = TaskDrag {
            task_id: task_id.clone(),
            title: SharedString::from(row.task.title.clone()),
            status: row.status,
        };
        let open_id = task_id.clone();
        let menu_id = task_id.clone();
        let paged = row.linked.is_some();
        let thumbs = self.row_images("task-kanban", row, theme, cx);
        Self::card_visual(row, selected, theme)
            .children(thumbs)
            .id(SharedString::from(format!("task-card-{task_id}")))
            .debug_selector(|| format!("task-kanban-{}", row.task.id))
            .cursor_pointer()
            .hover(|s| s.border_color(theme.border_strong))
            .when(morphing, |el| el.opacity(0.0))
            .child(morph::record(
                &self.painted,
                Key::Task(task_id.clone()),
                Edges::all(1.0),
            ))
            .on_click(cx.listener(move |page, _: &ClickEvent, window, cx| {
                page.open_row(&open_id, paged, window, cx)
            }))
            .on_mouse_down(MouseButton::Right, Self::row_menu_listener(menu_id, cx))
            .on_drag(drag, |drag, _, _, cx| {
                cx.stop_propagation();
                let title = drag.title.clone();
                cx.new(|_| TaskDragGhost { title })
            })
            .into_any_element()
    }

    fn card_visual(row: &Row, selected: bool, theme: &Theme) -> gpui::Div {
        let task_id = row.task.id.clone();
        let title = SharedString::from(row.task.title.clone());
        let orchestrated = row.orchestrated();
        let working = row.status == TaskStatus::Working;
        let bar = orchestrated.then(|| Self::step_bar(row, theme));
        let footer = if orchestrated {
            let mut fragments = Vec::new();
            let marks = row.marks();
            let PlanProgress { done, total } = row.plan.progress;
            fragments.push(div().child(format!("{done}/{total}")).into_any_element());
            Self::step_counts(&row.plan, theme, &mut fragments);
            div()
                .flex()
                .items_center()
                .gap(px(6.0))
                .min_w_0()
                .text_size(ui_rems(11.5))
                .text_color(theme.text_muted)
                .when(!marks.is_empty(), |el| el.child(Self::marks(&marks, theme)))
                .children(fragments)
        } else {
            let mut footer = div()
                .flex()
                .items_center()
                .gap(px(6.0))
                .min_w_0()
                .text_size(ui_rems(11.5))
                .text_color(theme.text_muted);
            if let Some(linked) = row.linked.as_ref() {
                let (mark, tint) = linked.brand;
                footer = footer.child(
                    icon(mark)
                        .size(px(12.0))
                        .text_color(tint.unwrap_or(theme.text_muted)),
                );
            }
            footer = footer.child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .child(row.project.clone().unwrap_or_else(|| "No project".into())),
            );
            if let Some(since) = row
                .linked
                .as_ref()
                .and_then(|l| l.since)
                .filter(|_| working)
            {
                let (_, _, tone) = crate::shell::chat_status_tone(ChatIndicator::Working, theme);
                footer = footer.child(crate::loaders::elapsed_label(
                    format!("task-elapsed-{task_id}"),
                    since,
                    ui_rems(11.5),
                    tone,
                ));
            }
            footer
        };
        div()
            .p(px(12.0))
            .flex()
            .flex_col()
            .gap(px(8.0))
            .rounded(px(10.0))
            .bg(theme.surface_card)
            .border_1()
            .border_color(if selected { theme.accent } else { theme.border })
            .child(
                div()
                    .text_size(ui_rems(13.0))
                    .line_height(ui_rems(17.0))
                    .text_color(if row.status == TaskStatus::Done {
                        theme.text_muted
                    } else {
                        theme.text
                    })
                    .line_clamp(3)
                    .child(title),
            )
            .children(bar)
            .child(footer)
    }

    fn step_bar(row: &Row, theme: &Theme) -> AnyElement {
        let id = row.task.id.clone();
        div()
            .debug_selector(move || format!("task-step-bar-{id}"))
            .flex()
            .gap(px(2.0))
            .children(
                row.plan
                    .steps
                    .iter()
                    .filter(|step| step.status != StepStatus::Skipped)
                    .map(|step| {
                        let fill = match step.status {
                            StepStatus::Done => column_tone(TaskStatus::Done, theme).2,
                            StepStatus::Working => column_tone(TaskStatus::Working, theme).2,
                            StepStatus::NeedsYou => theme.warning,
                            StepStatus::Failed => theme.danger,
                            StepStatus::Planned | StepStatus::Skipped => theme.wash(0.10),
                        };
                        div().flex_1().h(px(4.0)).rounded(px(2.0)).bg(fill)
                    }),
            )
            .into_any_element()
    }

    fn column_head(column: TaskStatus, count: usize, theme: &Theme) -> gpui::Div {
        let (_, _, tone) = column_tone(column, theme);
        div()
            .px(px(6.0))
            .pt(px(4.0))
            .flex()
            .items_center()
            .gap(px(8.0))
            .child(div().size(px(8.0)).rounded_full().bg(tone))
            .child(
                div()
                    .text_size(ui_rems(12.5))
                    .font_weight(gpui::FontWeight::MEDIUM)
                    .text_color(theme.text)
                    .child(column_title(column)),
            )
            .child(
                div()
                    .text_size(ui_rems(12.0))
                    .text_color(theme.text_faint)
                    .child(SharedString::from(count.to_string())),
            )
    }

    fn render_kanban(
        &self,
        open: &[Row],
        done: &[Row],
        chips: AnyElement,
        morph: Option<f32>,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let columns: Vec<AnyElement> = COLUMNS
            .iter()
            .enumerate()
            .map(|(ix, &column)| {
                let rows: Vec<&Row> = open
                    .iter()
                    .chain(done.iter())
                    .filter(|row| row.status == column)
                    .collect();
                let accepts = matches!(column, TaskStatus::ToDo | TaskStatus::Done);
                let accent = theme.accent;
                let cards: Vec<AnyElement> = rows
                    .iter()
                    .map(|row| self.kanban_card(row, morph.is_some(), theme, cx))
                    .collect();
                let rise = morph.map(|elapsed| {
                    (
                        morph::column_t(elapsed, ix, true),
                        morph::column_head_t(elapsed, ix, true),
                    )
                });
                div()
                    .id(SharedString::from(format!("tasks-column-{column:?}")))
                    .debug_selector(move || format!("tasks-column-{column:?}"))
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .flex()
                    .flex_col()
                    .gap(px(8.0))
                    .p(px(8.0))
                    .rounded(px(14.0))
                    .bg(widgets::block_fill(theme))
                    .border_1()
                    .border_color(gpui::transparent_black())
                    .when_some(rise, |el, (column, _)| {
                        el.relative()
                            .top(px(morph::COLUMN_RISE * (1.0 - column)))
                            .opacity(column)
                    })
                    .child(morph::record(
                        &self.painted,
                        Key::Column(ix),
                        Edges::all(1.0),
                    ))
                    .child(
                        Self::column_head(column, rows.len(), theme)
                            .when_some(rise, |el, (_, head)| {
                                el.relative()
                                    .top(px(-morph::HEAD_RISE * (1.0 - head)))
                                    .opacity(head)
                            })
                            .child(morph::record(
                                &self.painted,
                                Key::ColumnHead(ix),
                                Edges::default(),
                            )),
                    )
                    .child(
                        div()
                            .id(SharedString::from(format!(
                                "tasks-column-scroll-{column:?}"
                            )))
                            .flex_1()
                            .min_h_0()
                            .overflow_y_scroll()
                            .flex()
                            .flex_col()
                            .gap(px(8.0))
                            .children(cards),
                    )
                    .when(accepts, |el| {
                        el.drag_over::<TaskDrag>(move |style, drag, _, _| {
                            if logic::drop_completes(drag.status, column).is_some() {
                                style.border_color(accent).bg(accent.opacity(0.06))
                            } else {
                                style
                            }
                        })
                        .on_drop(cx.listener(
                            move |page, drag: &TaskDrag, _, cx| {
                                page.drop_on_column(drag, column, cx);
                            },
                        ))
                    })
                    .into_any_element()
            })
            .collect();
        div()
            .id("tasks-kanban")
            .debug_selector(|| "tasks-kanban".into())
            .size_full()
            .pt(px(Theme::TITLEBAR_HEIGHT + 20.0))
            .px(px(20.0))
            .pb(px(20.0))
            .flex()
            .flex_col()
            .gap(px(20.0))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(16.0))
                    .child(self.measured_header(open.len(), morph.is_some(), theme))
                    .child(chips),
            )
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .flex_row()
                    .gap(px(12.0))
                    .children(columns),
            )
            .into_any_element()
    }
}

/// A deleted lead leaves the mode without a lead, so the card starts it again.
pub(crate) fn has_lead(chats: &[zeron_proto::Chat], task: &zeron_proto::Task) -> bool {
    task.has_plan() && linked_chat(chats, task).is_some()
}

pub(crate) fn chat_model_label(chat: &zeron_proto::Chat, cx: &gpui::App) -> Option<String> {
    let config = chat.config.as_ref()?;
    let model = config.model.as_deref()?;
    Some(crate::pickers::model_display_label(
        config.harness,
        model,
        cx,
    ))
}

pub(crate) fn chat_brand(chat: &zeron_proto::Chat) -> Brand {
    let harness = chat.config.as_ref().map(|config| config.harness);
    let model = chat
        .config
        .as_ref()
        .and_then(|config| config.model.as_deref());
    crate::pickers::agent_brand_icon(harness, model)
}

pub(crate) fn rollup(state: &AppState, task: &zeron_proto::Task, now: DateTime<Utc>) -> Rollup {
    zeron_proto::mission::rollup(
        task,
        &state.chats,
        &state.task_steps,
        state
            .supervisor
            .iter()
            .flat_map(|snapshot| &snapshot.needs_you),
        |chat| {
            task_thread_status(
                state.display_status_for(chat, now),
                state.session_for(&chat.id),
                now,
            )
        },
        chat_brand,
    )
}

pub(crate) fn mission_selection(role: &MissionRole) -> Selection {
    match role {
        MissionRole::Lead => Selection::Lead,
        MissionRole::Step { step_id, .. } => Selection::Step(step_id.clone()),
    }
}

pub(crate) fn mission_thread(
    state: &AppState,
    chat_id: &str,
    now: DateTime<Utc>,
) -> Option<MissionThread> {
    zeron_proto::mission::mission_thread(&state.tasks, &state.task_steps, chat_id, |task| {
        rollup(state, task, now)
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TabMember {
    pub task_id: String,
    pub lead: String,
    pub step: Option<usize>,
}

pub(crate) fn tab_members(state: &AppState) -> std::collections::HashMap<String, TabMember> {
    let mut tasks: Vec<&zeron_proto::Task> = state
        .tasks
        .iter()
        .filter(|task| !task.deleted && task.has_plan())
        .collect();
    tasks.sort_by_key(|task| task.completed_at.is_none());
    let mut members = std::collections::HashMap::new();
    for task in tasks {
        let Some(lead) = task.chat_id.clone() else {
            continue;
        };
        let member = |step| TabMember {
            task_id: task.id.clone(),
            lead: lead.clone(),
            step,
        };
        members.insert(lead.clone(), member(None));
        let steps = state
            .task_steps
            .iter()
            .filter(|step| step.task_id == task.id && !step.deleted);
        for (ix, step) in steps.enumerate() {
            if let Some(chat_id) = &step.chat_id {
                members.insert(chat_id.clone(), member(Some(ix)));
            }
        }
    }
    members
}

pub(crate) fn can_add_plan(state: &AppState, chat: &zeron_proto::Chat) -> bool {
    state.local_device_id.as_deref() == Some(chat.device_id.as_str())
        && !chat.archived
        && !chat.is_system()
        && chat.sidechat_source_chat_id.is_none()
        && !state.is_hidden_chat(&chat.id)
        && !state.is_draft(&chat.id)
        && zeron_proto::plan_of_chat(&state.tasks, &chat.id).is_none()
}

fn row_for(state: &AppState, task: &zeron_proto::Task, now: DateTime<Utc>) -> Row {
    let linked = linked_chat(&state.chats, task).map(|chat| Linked {
        title: SharedString::from(zeron_proto::chat_mentions::chat_mention_title(chat)),
        indicator: state.display_status_for(chat, now),
        brand: chat_brand(chat),
        since: state.session_for(&chat.id).and_then(|s| s.started_at),
    });
    let plan = rollup(state, task, now);
    Row {
        status: plan.status,
        plan,
        project: task
            .space_id
            .as_deref()
            .and_then(|id| state.spaces.iter().find(|space| space.id == id))
            .map(|space| SharedString::from(space.display_name().to_string())),
        linked,
        task: task.clone(),
    }
}

fn patched(row: &zeron_proto::Task, patch: &TaskPatch) -> zeron_proto::Task {
    let mut task = row.clone();
    if let Some(title) = &patch.title {
        task.title = title.clone();
    }
    if let Some(notes) = &patch.notes {
        task.notes = notes.clone();
    }
    task
}

pub(crate) struct Gathered {
    pub id: String,
    pub title: SharedString,
    pub status: TaskStatus,
}

/// Plain tasks handed to this thread together, in the person's order; a mission's lead is not a gathering.
pub(crate) fn gathered_tasks(state: &AppState, chat_id: &str, now: DateTime<Utc>) -> Vec<Gathered> {
    let mut tasks: Vec<&zeron_proto::Task> = state
        .tasks
        .iter()
        .filter(|task| {
            !task.deleted && !task.has_plan() && task.chat_id.as_deref() == Some(chat_id)
        })
        .collect();
    tasks.sort_by(|a, b| {
        (&a.order_key, a.created_at, &a.id).cmp(&(&b.order_key, b.created_at, &b.id))
    });
    tasks
        .into_iter()
        .map(|task| Gathered {
            id: task.id.clone(),
            title: SharedString::from(zeron_proto::view::single_line(&task.title)),
            status: rollup(state, task, now).status,
        })
        .collect()
}

pub(crate) fn needs_you_count(state: &AppState) -> usize {
    let now = Utc::now();
    state
        .tasks
        .iter()
        .filter(|task| !task.deleted)
        .filter(|task| rollup(state, task, now).status == TaskStatus::NeedsYou)
        .count()
}

pub(crate) fn column_tone(
    status: TaskStatus,
    theme: &Theme,
) -> (Option<SharedString>, &'static str, Hsla) {
    match status {
        TaskStatus::ToDo => (None, icons::LUCIDE_CIRCLE_DASHED, theme.text_faint),
        TaskStatus::Working => crate::shell::chat_status_tone(ChatIndicator::Working, theme),
        TaskStatus::NeedsYou => crate::shell::chat_status_tone(ChatIndicator::AwaitingInput, theme),
        TaskStatus::Done => crate::shell::chat_status_tone(ChatIndicator::Completed, theme),
    }
}

pub(crate) fn dashed_circle(theme: &Theme) -> AnyElement {
    let color = theme.text_faint;
    gpui::canvas(
        |_, _, _| {},
        move |bounds, _, window, _| {
            let center = bounds.center();
            let radius = f32::from(bounds.size.width.min(bounds.size.height)) / 2.0 - 1.0;
            let dashes = 12;
            for i in 0..dashes {
                let start = i as f32 / dashes as f32 * std::f32::consts::TAU;
                let end = start + std::f32::consts::TAU / dashes as f32 * 0.5;
                let mut path = gpui::PathBuilder::stroke(px(1.5));
                path.move_to(gpui::point(
                    center.x + px(radius * start.cos()),
                    center.y + px(radius * start.sin()),
                ));
                path.line_to(gpui::point(
                    center.x + px(radius * end.cos()),
                    center.y + px(radius * end.sin()),
                ));
                if let Ok(path) = path.build() {
                    window.paint_path(path, color);
                }
            }
        },
    )
    .flex_none()
    .size(px(CHECK_SIZE))
    .into_any_element()
}

impl TasksPage {
    fn morph_rows<'a>(
        &self,
        open: &'a [Row],
        done: &'a [Row],
        morph: &Morph,
        targets: &Rects,
    ) -> Vec<MorphRow<'a>> {
        let Some(body) = self.body.get() else {
            return Vec::new();
        };
        open.iter()
            .chain(done)
            .filter_map(|row| {
                let key = Key::Task(row.task.id.clone());
                let from = morph::visible(&morph.from, &key, body);
                let to = morph::visible(targets, &key, body);
                (from.is_some() || to.is_some()).then_some((row, from, to))
            })
            .collect()
    }

    fn rects_now(
        &self,
        morph: &Morph,
        open: &[Row],
        done: &[Row],
        targets: &Rects,
        now: Instant,
    ) -> Rects {
        let elapsed = morph.elapsed_ms(now);
        let mut rects = morph.from.clone();
        rects.extend(targets.iter().map(|(key, rect)| (key.clone(), *rect)));
        for (index, (row, from, to)) in self
            .morph_rows(open, done, morph, targets)
            .into_iter()
            .enumerate()
        {
            if let Some((from, to)) = from.zip(to) {
                let t = morph::task_frame(elapsed, index).rect;
                rects.insert(
                    Key::Task(row.task.id.clone()),
                    motion::lerp_bounds(from, to, t),
                );
            }
        }
        if let Some((from, to)) = morph.from.get(&Key::Header).zip(targets.get(&Key::Header)) {
            rects.insert(
                Key::Header,
                motion::lerp_bounds(*from, *to, morph::header_t(elapsed)),
            );
        }
        rects
    }

    fn advance_morph(
        &mut self,
        view: TasksView,
        open: &[Row],
        done: &[Row],
        targets: &mut Rects,
        reduced: bool,
        now: Instant,
    ) -> Option<f32> {
        if self
            .view_now
            .replace(view)
            .is_some_and(|shown| shown != view)
        {
            let previous = self.morph.take();
            self.morph = (!reduced).then(|| {
                let from = match &previous {
                    Some(morph) if morph.started.is_some() => {
                        self.rects_now(morph, open, done, targets, now)
                    }
                    Some(morph) => morph.from.clone(),
                    None => std::mem::take(targets),
                };
                Morph {
                    from,
                    started: None,
                }
            });
            targets.clear();
        }
        let morph = self.morph.as_mut()?;
        if morph.started.is_none() && !targets.is_empty() {
            morph.started = Some(now);
        }
        let morph = self.morph.as_ref()?;
        let tasks = self.morph_rows(open, done, morph, targets).len();
        let elapsed = morph.elapsed_ms(now);
        if elapsed < morph::end_ms(tasks, COLUMNS.len()) {
            Some(elapsed)
        } else {
            self.morph = None;
            None
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn render_morph(
        &self,
        elapsed: f32,
        to_kanban: bool,
        open: &[Row],
        done: &[Row],
        targets: &Rects,
        theme: &Theme,
        cx: &gpui::App,
    ) -> Option<AnyElement> {
        let morph = self.morph.as_ref()?;
        let body = self.body.get()?;
        let place = |rect: Bounds<Pixels>| {
            div()
                .absolute()
                .left(rect.left() - body.left())
                .top(rect.top() - body.top())
                .w(rect.size.width)
                .h(rect.size.height)
        };
        let leaving = |key: &Key| morph::visible(&morph.from, key, body);
        let mut ghosts: Vec<AnyElement> = Vec::new();
        if to_kanban {
            let fade = morph::extra_t(elapsed, false);
            if let Some(rect) = leaving(&Key::Add) {
                ghosts.push(
                    place(rect)
                        .opacity(fade)
                        .child(self.add_ghost(theme, cx))
                        .into_any_element(),
                );
            }
            if let Some(rect) = leaving(&Key::Done) {
                ghosts.push(
                    place(rect)
                        .opacity(fade)
                        .child(Self::done_toggle(self.done_open, done.len(), theme))
                        .into_any_element(),
                );
            }
        } else {
            for (ix, &column) in COLUMNS.iter().enumerate() {
                if let Some(rect) = leaving(&Key::Column(ix)) {
                    ghosts.push(
                        place(rect)
                            .rounded(px(14.0))
                            .bg(widgets::block_fill(theme))
                            .opacity(morph::column_t(elapsed, ix, false))
                            .into_any_element(),
                    );
                }
                if let Some(rect) = leaving(&Key::ColumnHead(ix)) {
                    let count = open
                        .iter()
                        .chain(done)
                        .filter(|row| row.status == column)
                        .count();
                    ghosts.push(
                        place(rect)
                            .opacity(morph::column_head_t(elapsed, ix, false))
                            .child(Self::column_head(column, count, theme))
                            .into_any_element(),
                    );
                }
            }
        }
        let selected = self.card.read(cx).task_id().map(str::to_string);
        let today = Self::today();
        for (index, (row, from, to)) in self
            .morph_rows(open, done, morph, targets)
            .into_iter()
            .enumerate()
        {
            let frame = morph::task_frame(elapsed, index);
            let Some(rect) = (match (from, to) {
                (Some(from), Some(to)) => Some(motion::lerp_bounds(from, to, frame.rect)),
                (from, to) => from.or(to),
            }) else {
                continue;
            };
            let outgoing = if from.is_some() { frame.outgoing } else { 0.0 };
            let incoming = if to.is_some() { frame.incoming } else { 0.0 };
            let (row_opacity, card_opacity) = if to_kanban {
                (outgoing, incoming)
            } else {
                (incoming, outgoing)
            };
            let chosen = selected.as_deref() == Some(row.task.id.as_str());
            let check = Self::check_visual(row, theme).into_any_element();
            ghosts.push(
                place(rect)
                    .rounded(px(10.0))
                    .overflow_hidden()
                    .when(row_opacity > 0.0, |el| {
                        el.child(
                            Self::row_content(row, check, chosen, today, theme)
                                .absolute()
                                .inset_0()
                                .opacity(row_opacity),
                        )
                    })
                    .when(card_opacity > 0.0, |el| {
                        el.child(
                            Self::card_visual(row, chosen, theme)
                                .absolute()
                                .inset_0()
                                .opacity(card_opacity),
                        )
                    })
                    .into_any_element(),
            );
        }
        let header = morph.from.get(&Key::Header).map(|from| {
            targets.get(&Key::Header).map_or(*from, |to| {
                motion::lerp_bounds(*from, *to, morph::header_t(elapsed))
            })
        });
        if let Some(rect) = header {
            ghosts.push(
                place(rect)
                    .child(Self::header(theme, open.len()))
                    .into_any_element(),
            );
        }
        Some(
            div()
                .absolute()
                .inset_0()
                .overflow_hidden()
                .children(ghosts)
                .into_any_element(),
        )
    }
}

pub(crate) fn tasks_view(cx: &gpui::App) -> TasksView {
    if std::env::var("ZERON_TASKS_VIEW").as_deref() == Ok("kanban") {
        return TasksView::Kanban;
    }
    settings::current(cx).tasks_view
}

impl Render for TasksPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::of(cx).clone();
        self.settle_pending(cx);
        let (mut open, mut done) = self.rows(cx);
        let view = tasks_view(cx);
        let now = motion::now();
        let reduced = motion::reduced_motion(cx);
        let mut trees = match view {
            TasksView::List => tree_index(self.state.read(cx), Utc::now()),
            TasksView::Kanban => index::IndexRows::default(),
        };
        if view == TasksView::Kanban && self.filter == TasksFilter::Trees {
            self.filter = TasksFilter::All;
        }
        let counts = [
            TasksFilter::All,
            TasksFilter::Missions,
            TasksFilter::Trees,
            TasksFilter::NeedsYou,
        ]
        .map(|filter| {
            let tasks = open.iter().chain(&done).filter(|row| filter.keeps(row));
            let trees = trees
                .running
                .iter()
                .chain(&trees.done_today)
                .filter(|row| filter.keeps_tree(row));
            (filter, tasks.count() + trees.count())
        });
        let counts: Vec<(TasksFilter, usize)> = counts
            .into_iter()
            .filter(|(filter, _)| view == TasksView::List || *filter != TasksFilter::Trees)
            .collect();
        let chips = self.filter_chips(counts, &theme, cx);
        let filter = self.filter;
        open.retain(|row| filter.keeps(row));
        done.retain(|row| filter.keeps(row));
        trees.running.retain(|row| filter.keeps_tree(row));
        trees.done_today.retain(|row| filter.keeps_tree(row));
        if trees.running.iter().any(|row| row.status.live()) && !reduced {
            motion::pulse_lease(cx.entity_id(), cx);
        }
        let mut targets = std::mem::take(&mut *self.painted.borrow_mut());
        self.rects = targets.clone();
        self.shown = match view {
            TasksView::List => open.iter().map(|row| row.task.id.clone()).collect(),
            TasksView::Kanban => Vec::new(),
        };
        self.picked
            .retain(|id| open.iter().any(|row| row.task.id == *id));
        let elapsed = self.advance_morph(view, &open, &done, &mut targets, reduced, now);
        if elapsed.is_some() || self.shifts.tick(now) {
            window.request_animation_frame();
        }
        let selection_bar = self.render_selection_bar(view, reduced, now, &theme, window, cx);
        let body = match view {
            TasksView::List => self.render_list(&open, &done, &trees, chips, elapsed, &theme, cx),
            TasksView::Kanban => self.render_kanban(&open, &done, chips, elapsed, &theme, cx),
        };
        let ghosts = elapsed.and_then(|elapsed| {
            self.render_morph(
                elapsed,
                view == TasksView::Kanban,
                &open,
                &done,
                &targets,
                &theme,
                cx,
            )
        });
        let measured = self.body.clone();
        let card = self.card.read(cx).frame(now);
        div()
            .id("tasks-page")
            .debug_selector(|| "tasks-page".into())
            .track_focus(&self.focus)
            .size_full()
            .flex()
            .flex_row()
            .on_key_down(cx.listener(|page, event: &KeyDownEvent, window, cx| {
                if event.keystroke.key == "escape" && page.cancel_drag(window, cx) {
                    cx.stop_propagation();
                } else if event.keystroke.key == "escape" && page.thread_picker {
                    cx.stop_propagation();
                    page.thread_picker = false;
                    cx.notify();
                } else if event.keystroke.key == "escape" && page.menu.is_open() {
                    cx.stop_propagation();
                    page.close_row_menu(cx);
                } else if event.keystroke.key == "escape" && page.card.read(cx).is_open() {
                    cx.stop_propagation();
                    page.card.update(cx, |card, cx| {
                        if !card.dismiss_popups(cx) {
                            card.close(Some(window), cx)
                        }
                    });
                } else if event.keystroke.key == "escape" && !page.picked.is_empty() {
                    cx.stop_propagation();
                    page.clear_picks(cx);
                }
            }))
            .child(
                div()
                    .relative()
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .overflow_hidden()
                    .child(
                        gpui::canvas(
                            move |bounds, _, _| measured.set(Some(bounds)),
                            |_, _, _, _| {},
                        )
                        .absolute()
                        .size_full(),
                    )
                    .child(body)
                    .children(ghosts)
                    .children(selection_bar),
            )
            .children(card.map(|frame| {
                div()
                    .relative()
                    .flex_none()
                    .w(px(CARD_WIDTH * frame.slot))
                    .h_full()
                    .child(
                        div()
                            .absolute()
                            .top_0()
                            .bottom_0()
                            .right(px(-frame.shift))
                            .w(px(CARD_WIDTH))
                            .pt(px(Theme::TITLEBAR_HEIGHT + 12.0))
                            .pr(px(16.0))
                            .pb(px(16.0))
                            .opacity(frame.panel)
                            .child(self.card.clone()),
                    )
            }))
            .children(self.render_row_menu(&theme, cx))
            .child(div().absolute().size_0().child(self.gallery.clone()))
    }
}
