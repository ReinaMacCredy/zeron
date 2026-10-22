use std::cell::Cell;
use std::rc::Rc;
use std::time::{Duration, Instant};

use chrono::{DateTime, Local, NaiveDate, Utc};
use gpui::{
    AnyElement, Bounds, ClickEvent, Context, Edges, Entity, EventEmitter, FocusHandle, Focusable,
    Hsla, IntoElement, KeyDownEvent, MouseButton, MouseDownEvent, Pixels, Point, Render,
    SharedString, Subscription, Window, div, prelude::*, px,
};
use zeron_proto::supervisor::NeedsYouRow;
use zeron_proto::{
    ChatIndicator, PlanProgress, StepStatus, TaskPatch, TaskStatus, plan_progress, step_status,
    task_status,
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
pub(crate) use thread_tree::tree_rows;
mod tree;

pub use card::{CardFocus, Orchestrate, TaskCard, TaskCardEvent};
pub use center::{LiveThread, PlanEdit, Selection, TaskCenter, TaskCenterEvent};
pub(crate) use index::{Kind as IndexKind, MissionIndex, MissionIndexEvent, index_rows};
use logic::COLUMNS;
use morph::{Key, Morph, Painted, Rects};
pub(crate) use strip_tree::{SampleRow, StripTree, StripTreeEvent, TreeHost};
pub(crate) use tree::{Status as TreeStatus, glyph as tree_glyph, kind_badge};

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
    indicator: ChatIndicator,
    brand: (&'static str, Option<Hsla>),
    since: Option<DateTime<Utc>>,
}

type Brand = (&'static str, Option<Hsla>);

pub(crate) struct StepRow {
    pub step: zeron_proto::TaskStep,
    pub status: StepStatus,
    pub indicator: Option<ChatIndicator>,
    pub brand: Option<Brand>,
}

pub(crate) struct Rollup {
    pub lead: Option<ChatIndicator>,
    pub steps: Vec<StepRow>,
    pub status: TaskStatus,
    pub progress: PlanProgress,
    pub questions: usize,
}

impl Rollup {
    pub fn count(&self, status: StepStatus) -> usize {
        self.steps.iter().filter(|row| row.status == status).count()
    }

    pub fn step_label(&self, step_id: &str) -> Option<String> {
        let ix = self.steps.iter().position(|row| row.step.id == step_id)?;
        Some(format!("Step {} of {}", ix + 1, self.steps.len()))
    }

    pub fn phase(&self, task: &zeron_proto::Task) -> PlanPhase {
        let lead_working = self.lead == Some(ChatIndicator::Working);
        if task.completed_at.is_none()
            && task.plan_pending
            && !self.steps.is_empty()
            && !lead_working
        {
            PlanPhase::PlanReady
        } else if task.completed_at.is_none() && lead_working && self.steps.is_empty() {
            PlanPhase::Planning
        } else {
            PlanPhase::Status(self.status)
        }
    }

    pub fn label(&self, task: &zeron_proto::Task) -> (&'static str, TaskStatus) {
        match self.phase(task) {
            PlanPhase::PlanReady => ("Plan ready", TaskStatus::NeedsYou),
            PlanPhase::Planning => ("Planning", TaskStatus::Working),
            PlanPhase::Status(status) => (logic::column_title(status), status),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PlanPhase {
    PlanReady,
    Planning,
    Status(TaskStatus),
}

struct Row {
    task: zeron_proto::Task,
    status: TaskStatus,
    project: Option<SharedString>,
    linked: Option<Linked>,
    plan: Rollup,
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
    open_first_card: bool,
    focus: FocusHandle,
    view_now: Option<TasksView>,
    morph: Option<Morph>,
    painted: Painted,
    body: Rc<Cell<Option<Bounds<Pixels>>>>,
    menu: popover::Popup<RowMenu>,
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
            open_first_card: std::env::var("ZERON_OPEN_DIALOG").as_deref() == Ok("task-card"),
            focus: cx.focus_handle(),
            view_now: None,
            morph: None,
            painted: Painted::default(),
            body: Rc::default(),
            menu: popover::Popup::default(),
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

    fn open_knob_card(&mut self, cx: &mut Context<Self>) {
        if !self.open_first_card {
            return;
        }
        let first = {
            let (open, _) = logic::split_tasks(&self.state.read(cx).tasks);
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
        orchestrated: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if orchestrated {
            cx.emit(TasksEvent::OpenCenter(task_id.to_string()));
            return;
        }
        self.card.update(cx, |card, cx| {
            card.open(
                task_id.to_string(),
                None,
                CardFocus::Title,
                Some(window),
                cx,
            )
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

    fn reorder(&mut self, task_id: &str, before: Option<&str>, cx: &mut Context<Self>) {
        let key = {
            let state = self.state.read(cx);
            let (open, _) = logic::split_tasks(&state.tasks);
            logic::reorder_key(&open, task_id, before, &uuid::Uuid::new_v4().to_string())
        };
        if let Some(key) = key {
            self.update(
                task_id,
                TaskPatch {
                    order_key: Some(key),
                    ..Default::default()
                },
                cx,
            );
        }
    }

    fn drop_on_column(&mut self, drag: &TaskDrag, column: TaskStatus, cx: &mut Context<Self>) {
        if let Some(completed) = logic::drop_completes(drag.status, column) {
            self.set_completed(&drag.task_id, completed, cx);
        }
    }

    fn rows(&self, cx: &gpui::App) -> (Vec<Row>, Vec<Row>) {
        let state = self.state.read(cx);
        let now = Utc::now();
        let (open, done) = logic::split_tasks(&state.tasks);
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

    fn linked_label(indicator: ChatIndicator) -> &'static str {
        match indicator {
            ChatIndicator::Working => "Working",
            ChatIndicator::AwaitingInput => "Asked you a question",
            ChatIndicator::Errored => "Failed",
            ChatIndicator::Completed | ChatIndicator::Idle => "Ready for you",
        }
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
        fragments.push(
            div()
                .flex()
                .items_center()
                .gap(px(4.0))
                .child(
                    icon(icons::MISSION_CONTROL)
                        .size(px(11.0))
                        .text_color(theme.text_muted),
                )
                .child("Mission Control")
                .into_any_element(),
        );
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
        if row.task.send_error.is_some() {
            fragments.push(Self::not_sent(theme));
        }
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
                    .child(Self::linked_label(linked.indicator))
                    .into_any_element(),
            };
            fragments.push(
                div()
                    .flex()
                    .items_center()
                    .gap(px(5.0))
                    .child(
                        icon(mark)
                            .size(px(12.0))
                            .text_color(tint.unwrap_or(theme.text_muted)),
                    )
                    .child(label)
                    .into_any_element(),
            );
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
        let hover = theme.glass_hover();
        let accent = theme.accent;
        let open_id = task_id.clone();
        let menu_id = task_id.clone();
        let orchestrated = row.orchestrated();
        let drop_before = task_id.clone();
        let drag = TaskDrag {
            task_id: task_id.clone(),
            title: SharedString::from(row.task.title.clone()),
            status: row.status,
        };
        let check = self.checkbox(row, theme, cx);
        let thumbs = self.row_images("task-row", row, theme, cx);
        Self::row_content(row, check, selected, today, theme)
            .children(thumbs)
            .id(SharedString::from(format!("task-row-{task_id}")))
            .debug_selector(|| format!("task-row-{}", row.task.id))
            .role(gpui::Role::ListItem)
            .cursor_pointer()
            .when(!selected, |el| el.hover(move |s| s.bg(hover)))
            .when(morphing, |el| el.opacity(0.0))
            .child(morph::record(
                &self.painted,
                Key::Task(task_id.clone()),
                ROW_BORDER,
            ))
            .on_click(cx.listener(move |page, _: &ClickEvent, window, cx| {
                page.open_row(&open_id, orchestrated, window, cx)
            }))
            .on_mouse_down(MouseButton::Right, Self::row_menu_listener(menu_id, cx))
            .when(!done, |el| {
                el.on_drag(drag, |drag, _, _, cx| {
                    cx.stop_propagation();
                    let title = drag.title.clone();
                    cx.new(|_| TaskDragGhost { title })
                })
                .drag_over::<TaskDrag>(move |style, drag, _, _| {
                    if drag.status == TaskStatus::Done {
                        style
                    } else {
                        style.border_color(accent)
                    }
                })
                .on_drop(cx.listener(move |page, drag: &TaskDrag, _, cx| {
                    page.reorder(&drag.task_id, Some(&drop_before), cx);
                }))
            })
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
            .drag_over::<TaskDrag>(move |style, _, _, _| style.border_color(accent))
            .on_drop(cx.listener(|page, drag: &TaskDrag, _, cx| {
                page.reorder(&drag.task_id, None, cx);
            }))
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

    fn done_group(
        &self,
        done: &[Row],
        today: NaiveDate,
        morph: Option<f32>,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        if done.is_empty() {
            return None;
        }
        let open = self.done_open;
        let text = theme.text;
        let toggle = Self::done_toggle(open, done.len(), theme)
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

    fn render_list(
        &self,
        open: &[Row],
        done: &[Row],
        morph: Option<f32>,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let today = Self::today();
        let rows: Vec<AnyElement> = open
            .iter()
            .map(|row| self.list_row(row, today, morph.is_some(), theme, cx))
            .collect();
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
            .child(self.measured_header(open.len(), morph.is_some(), theme))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(2.0))
                    .children(rows)
                    .child(self.add_row(
                        morph.map(|elapsed| morph::extra_t(elapsed, true)),
                        theme,
                        cx,
                    ))
                    .children(self.add_tray(theme, cx)),
            )
            .children(self.done_group(done, today, morph, theme, cx));
        widgets::scroll_faded(
            "tasks-list-scroll",
            div()
                .id("tasks-list")
                .debug_selector(|| "tasks-list".into())
                .size_full()
                .overflow_y_scroll()
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
        let orchestrated = row.orchestrated();
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
                page.open_row(&open_id, orchestrated, window, cx)
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
                    .child(logic::column_title(column)),
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
            .child(self.measured_header(open.len(), morph.is_some(), theme))
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

pub(crate) fn step_chat<'a>(
    chats: &'a [zeron_proto::Chat],
    step: &zeron_proto::TaskStep,
) -> Option<&'a zeron_proto::Chat> {
    let id = step.chat_id.as_deref()?;
    chats.iter().find(|chat| chat.id == id)
}

pub(crate) fn rollup(state: &AppState, task: &zeron_proto::Task, now: DateTime<Utc>) -> Rollup {
    let lead = linked_chat(&state.chats, task).map(|chat| state.display_status_for(chat, now));
    let steps: Vec<StepRow> = state
        .task_steps
        .iter()
        .filter(|step| step.task_id == task.id && !step.deleted)
        .map(|step| {
            let chat = step_chat(&state.chats, step);
            let indicator = chat.map(|chat| state.display_status_for(chat, now));
            StepRow {
                step: step.clone(),
                status: step_status(step, chat, indicator),
                indicator,
                brand: chat.map(chat_brand),
            }
        })
        .collect();
    let statuses: Vec<StepStatus> = steps.iter().map(|row| row.status).collect();
    let questions = state.supervisor.as_ref().map_or(0, |snapshot| {
        snapshot
            .needs_you
            .iter()
            .filter(|row| {
                let task_id = match row {
                    NeedsYouRow::Item(item) => item.task_id.as_deref(),
                    NeedsYouRow::Input(input) => input.task_id.as_deref(),
                };
                task_id == Some(task.id.as_str())
            })
            .count()
    });
    Rollup {
        lead,
        status: task_status(task, lead, &statuses),
        progress: plan_progress(&statuses),
        steps,
        questions,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum MissionRole {
    Lead,
    Step { step_id: String, label: String },
}

impl MissionRole {
    pub fn label(&self) -> String {
        match self {
            Self::Lead => "Lead".into(),
            Self::Step { label, .. } => label.clone(),
        }
    }

    pub fn selection(&self) -> Selection {
        match self {
            Self::Lead => Selection::Lead,
            Self::Step { step_id, .. } => Selection::Step(step_id.clone()),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct MissionThread {
    pub task_id: String,
    pub title: String,
    pub role: MissionRole,
    pub progress: PlanProgress,
}

pub(crate) fn mission_thread(
    state: &AppState,
    chat_id: &str,
    now: DateTime<Utc>,
) -> Option<MissionThread> {
    let mission = |task: &&zeron_proto::Task| task.has_plan() && !task.deleted;
    let (task, step_id) = match plan_led_by(&state.tasks, chat_id) {
        Some(task) => (task, None),
        None => state
            .task_steps
            .iter()
            .filter(|step| !step.deleted && step.chat_id.as_deref() == Some(chat_id))
            .find_map(|step| {
                let task = state
                    .tasks
                    .iter()
                    .filter(mission)
                    .find(|task| task.id == step.task_id)?;
                Some((task, Some(step.id.clone())))
            })?,
    };
    let plan = rollup(state, task, now);
    let role = match step_id {
        None => MissionRole::Lead,
        Some(step_id) => MissionRole::Step {
            label: plan.step_label(&step_id)?,
            step_id,
        },
    };
    Some(MissionThread {
        task_id: task.id.clone(),
        title: task.title.clone(),
        role,
        progress: plan.progress,
    })
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

/// The open plan, else the one that finished last, so a done lead keeps its strip.
pub(crate) fn plan_led_by<'a>(
    tasks: &'a [zeron_proto::Task],
    chat_id: &str,
) -> Option<&'a zeron_proto::Task> {
    zeron_proto::plan_of_chat(tasks, chat_id).or_else(|| {
        tasks
            .iter()
            .filter(|task| {
                !task.deleted && task.has_plan() && task.chat_id.as_deref() == Some(chat_id)
            })
            .max_by_key(|task| task.completed_at)
    })
}

fn row_for(state: &AppState, task: &zeron_proto::Task, now: DateTime<Utc>) -> Row {
    let linked = linked_chat(&state.chats, task).map(|chat| Linked {
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

/// A link whose chat is gone (deleted) reads as no link: the task can start again.
pub(crate) fn linked_chat<'a>(
    chats: &'a [zeron_proto::Chat],
    task: &zeron_proto::Task,
) -> Option<&'a zeron_proto::Chat> {
    let id = task.chat_id.as_deref()?;
    chats.iter().find(|chat| chat.id == id)
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

pub(crate) fn needs_you_count(state: &AppState) -> usize {
    let now = Utc::now();
    state
        .tasks
        .iter()
        .filter(|task| !task.deleted)
        .filter(|task| rollup(state, task, now).status == TaskStatus::NeedsYou)
        .count()
}

fn column_tone(status: TaskStatus, theme: &Theme) -> (Option<SharedString>, &'static str, Hsla) {
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
        let (open, done) = self.rows(cx);
        let view = tasks_view(cx);
        let now = Instant::now();
        let reduced = motion::reduced_motion(cx);
        let mut targets = std::mem::take(&mut *self.painted.borrow_mut());
        let elapsed = self.advance_morph(view, &open, &done, &mut targets, reduced, now);
        if elapsed.is_some() {
            window.request_animation_frame();
        }
        let body = match view {
            TasksView::List => self.render_list(&open, &done, elapsed, &theme, cx),
            TasksView::Kanban => self.render_kanban(&open, &done, elapsed, &theme, cx),
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
                if event.keystroke.key == "escape" && page.menu.is_open() {
                    cx.stop_propagation();
                    page.close_row_menu(cx);
                } else if event.keystroke.key == "escape" && page.card.read(cx).is_open() {
                    cx.stop_propagation();
                    page.card.update(cx, |card, cx| {
                        if !card.dismiss_popups(cx) {
                            card.close(Some(window), cx)
                        }
                    });
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
                    .children(ghosts),
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
