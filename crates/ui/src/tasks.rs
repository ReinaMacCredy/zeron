use std::time::Duration;

use chrono::{DateTime, Local, NaiveDate, Utc};
use gpui::{
    AnyElement, ClickEvent, Context, Entity, EntityInputHandler, EventEmitter, FocusHandle,
    Focusable, Hsla, IntoElement, KeyDownEvent, Render, SharedString, Subscription, Task, Window,
    div, prelude::*, px,
};
use zeron_proto::{ChatIndicator, TaskPatch, TaskStatus, task_status};

use crate::composer::{ComposerInput, ComposerInputEvent, MentionTab, Submit};
use crate::icons::{self, icon};
use crate::settings::{self, TasksView, widgets};
use crate::state::AppState;
use crate::theme::Theme;
use crate::typography::ui_rems;

mod logic;

use logic::{COLUMNS, DuePick};

pub(crate) const ADD_PLACEHOLDER: &str = "Add a task, or press Tab to add notes and start it";
const LIST_MAX_WIDTH: f32 = 640.0;
const CARD_WIDTH: f32 = 340.0;
const ROW_HEIGHT: f32 = 44.0;
const CHECK_SIZE: f32 = 18.0;
const SAVE_DEBOUNCE: Duration = Duration::from_millis(500);

pub enum TasksEvent {
    Create {
        title: String,
        space_id: Option<String>,
        open_card: bool,
    },
    Update {
        task_id: String,
        patch: TaskPatch,
    },
    Delete(String),
    Start {
        task_id: String,
        patch: Option<TaskPatch>,
    },
    OpenChat(String),
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

struct Card {
    task_id: String,
    title: Entity<ComposerInput>,
    notes: Entity<ComposerInput>,
    saved_title: String,
    saved_notes: String,
    seen: bool,
    save: Option<Task<()>>,
    _subscriptions: [Subscription; 2],
}

struct Linked {
    indicator: ChatIndicator,
    brand: (&'static str, Option<Hsla>),
    since: Option<DateTime<Utc>>,
}

struct Row {
    task: zeron_proto::Task,
    status: TaskStatus,
    project: Option<SharedString>,
    linked: Option<Linked>,
}

pub struct TasksPage {
    state: Entity<AppState>,
    add: Entity<ComposerInput>,
    card: Option<Card>,
    project_select: widgets::SelectState,
    done_open: bool,
    open_first_card: bool,
    focus: FocusHandle,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<TasksEvent> for TasksPage {}

impl Focusable for TasksPage {
    fn focus_handle(&self, _: &gpui::App) -> FocusHandle {
        self.focus.clone()
    }
}

impl TasksPage {
    pub fn new(state: Entity<AppState>, cx: &mut Context<Self>) -> Self {
        let add = cx.new(|cx| ComposerInput::new(ADD_PLACEHOLDER, cx).with_single_line());
        let add_events = cx.subscribe(&add, |page, _, event: &ComposerInputEvent, cx| {
            if matches!(event, ComposerInputEvent::Submitted) {
                page.submit_add(false, cx);
            }
        });
        let state_changes = cx.observe(&state, |page, _, cx| {
            page.sync_card(cx);
            cx.notify();
        });
        let settings_changes = cx.observe_global::<settings::SettingsStore>(|_, cx| cx.notify());
        if std::env::var("ZERON_TASKS_VIEW").as_deref() == Ok("kanban") {
            settings::update(settings::SavePolicy::Debounced, cx, |s| {
                s.tasks_view = TasksView::Kanban;
            });
        }
        let mut page = Self {
            state,
            add,
            card: None,
            project_select: widgets::SelectState::default(),
            done_open: true,
            open_first_card: std::env::var("ZERON_OPEN_DIALOG").as_deref() == Ok("task-card"),
            focus: cx.focus_handle(),
            _subscriptions: vec![add_events, state_changes, settings_changes],
        };
        page.sync_card(cx);
        page
    }

    fn today() -> NaiveDate {
        Local::now().date_naive()
    }

    fn submit_add(&mut self, open_card: bool, cx: &mut Context<Self>) -> bool {
        let Some(title) = logic::quick_add_title(self.add.read(cx).text()) else {
            return false;
        };
        let space_id = self
            .state
            .read(cx)
            .selected_space_row()
            .map(|space| space.id.clone());
        self.add.update(cx, |input, cx| input.set_text("", cx));
        cx.emit(TasksEvent::Create {
            title,
            space_id,
            open_card,
        });
        true
    }

    /// `title` seeds the card of a task whose row has not arrived yet.
    pub fn open_card(
        &mut self,
        task_id: String,
        title: Option<String>,
        focus_notes: bool,
        window: Option<&mut Window>,
        cx: &mut Context<Self>,
    ) {
        if self
            .card
            .as_ref()
            .is_some_and(|card| card.task_id == task_id)
        {
            return;
        }
        self.flush_card(cx);
        let row = self
            .state
            .read(cx)
            .tasks
            .iter()
            .find(|task| task.id == task_id)
            .cloned();
        let saved_title = row
            .as_ref()
            .map(|task| task.title.clone())
            .or(title)
            .unwrap_or_default();
        let saved_notes = row
            .as_ref()
            .map(|task| task.notes.clone())
            .unwrap_or_default();
        let title_input = cx.new(|cx| {
            let mut input = ComposerInput::new("Task title", cx).with_single_line();
            input.set_text(saved_title.clone(), cx);
            input
        });
        let notes_input = cx.new(|cx| {
            let mut input = ComposerInput::new("Add notes", cx);
            input.set_text(saved_notes.clone(), cx);
            input
        });
        let edits = |page: &mut Self,
                     _: Entity<ComposerInput>,
                     event: &ComposerInputEvent,
                     cx: &mut Context<Self>| match event {
            ComposerInputEvent::Edited => page.schedule_save(cx),
            ComposerInputEvent::Submitted => page.flush_card(cx),
            _ => {}
        };
        let subscriptions = [
            cx.subscribe(&title_input, edits),
            cx.subscribe(&notes_input, edits),
        ];
        if let Some(window) = window {
            let target = if focus_notes {
                &notes_input
            } else {
                &title_input
            };
            window.focus(&target.focus_handle(cx), cx);
        }
        self.card = Some(Card {
            task_id,
            title: title_input,
            notes: notes_input,
            saved_title,
            saved_notes,
            seen: row.is_some(),
            save: None,
            _subscriptions: subscriptions,
        });
        cx.notify();
    }

    fn close_card(&mut self, cx: &mut Context<Self>) {
        self.flush_card(cx);
        self.card = None;
        cx.notify();
    }

    fn sync_card(&mut self, cx: &mut Context<Self>) {
        if self.open_first_card {
            let first = {
                let (open, _) = logic::split_tasks(&self.state.read(cx).tasks);
                open.iter()
                    .find(|task| task.chat_id.is_none())
                    .or(open.first())
                    .map(|task| task.id.clone())
            };
            if let Some(id) = first {
                self.open_first_card = false;
                self.open_card(id, None, false, None, cx);
            }
        }
        let Some(card) = self.card.as_mut() else {
            return;
        };
        let present = self
            .state
            .read(cx)
            .tasks
            .iter()
            .any(|task| task.id == card.task_id && !task.deleted);
        if present {
            card.seen = true;
        } else if card.seen {
            self.card = None;
        }
    }

    fn schedule_save(&mut self, cx: &mut Context<Self>) {
        let Some(card) = self.card.as_mut() else {
            return;
        };
        card.save = Some(cx.spawn(async move |page, cx| {
            cx.background_executor().timer(SAVE_DEBOUNCE).await;
            page.update(cx, |page, cx| page.flush_card(cx)).ok();
        }));
    }

    fn take_card_patch(&mut self, cx: &mut Context<Self>) -> Option<(String, TaskPatch)> {
        let card = self.card.as_mut()?;
        card.save = None;
        let title = card.title.read(cx).text().trim().to_string();
        let notes = card.notes.read(cx).text().to_string();
        let mut patch = TaskPatch::default();
        if !title.is_empty() && title != card.saved_title {
            card.saved_title = title.clone();
            patch.title = Some(title);
        }
        if notes != card.saved_notes {
            card.saved_notes = notes.clone();
            patch.notes = Some(notes);
        }
        (patch != TaskPatch::default()).then(|| (card.task_id.clone(), patch))
    }

    fn flush_card(&mut self, cx: &mut Context<Self>) {
        if let Some((task_id, patch)) = self.take_card_patch(cx) {
            cx.emit(TasksEvent::Update { task_id, patch });
        }
    }

    fn start_card(&mut self, cx: &mut Context<Self>) {
        let Some(card) = self.card.as_ref() else {
            return;
        };
        let task_id = card.task_id.clone();
        let startable = self.state.read(cx).tasks.iter().any(|task| {
            task.id == task_id && task.chat_id.is_none() && task.completed_at.is_none()
        });
        if !startable {
            return;
        }
        let patch = self.take_card_patch(cx).map(|(_, patch)| patch);
        cx.emit(TasksEvent::Start { task_id, patch });
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
        let row = |task: &zeron_proto::Task| {
            let chat = task
                .chat_id
                .as_deref()
                .and_then(|id| state.chats.iter().find(|chat| chat.id == id));
            let linked = chat.map(|chat| {
                let harness = chat.config.as_ref().map(|config| config.harness);
                let model = chat
                    .config
                    .as_ref()
                    .and_then(|config| config.model.as_deref());
                Linked {
                    indicator: state.display_status_for(chat, now),
                    brand: crate::pickers::agent_brand_icon(harness, model),
                    since: state.session_for(&chat.id).and_then(|s| s.started_at),
                }
            });
            Row {
                status: task_status(task, linked.as_ref().map(|l| l.indicator)),
                project: task
                    .space_id
                    .as_deref()
                    .and_then(|id| state.spaces.iter().find(|space| space.id == id))
                    .map(|space| SharedString::from(space.display_name().to_string())),
                linked,
                task: task.clone(),
            }
        };
        let (open, done) = logic::split_tasks(&state.tasks);
        (
            open.into_iter().map(row).collect(),
            done.into_iter().map(row).collect(),
        )
    }

    fn subtitle(open: usize) -> String {
        match open {
            0 => "Nothing to do".into(),
            n => format!("{n} to do"),
        }
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

    fn checkbox(&self, row: &Row, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let done = row.status == TaskStatus::Done;
        let task_id = row.task.id.clone();
        let accent = theme.accent;
        let ring = theme.text_faint;
        div()
            .id(SharedString::from(format!("task-check-{task_id}")))
            .role(gpui::Role::CheckBox)
            .aria_label(if done {
                "Mark as not done"
            } else {
                "Mark as done"
            })
            .flex_none()
            .size(px(CHECK_SIZE))
            .rounded_full()
            .flex()
            .items_center()
            .justify_center()
            .cursor_pointer()
            .when(done, |el| {
                el.bg(accent)
                    .child(icon(icons::CHECK).size(px(12.0)).text_color(gpui::white()))
            })
            .when(!done, |el| {
                el.border_1()
                    .border_color(ring)
                    .hover(move |s| s.border_color(accent).bg(accent.opacity(0.12)))
            })
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
        if let Some(linked) = row
            .linked
            .as_ref()
            .filter(|_| row.status != TaskStatus::Done)
        {
            let (_, _, tone) = crate::shell::chat_status_tone(linked.indicator, theme);
            let (mark, tint) = linked.brand;
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
                    .child(
                        div()
                            .text_color(tone)
                            .child(Self::linked_label(linked.indicator)),
                    )
                    .into_any_element(),
            );
        } else if row.task.chat_id.is_some() && row.linked.is_none() {
            fragments.push(div().child("Chat deleted").into_any_element());
        }
        fragments
    }

    fn list_row(
        &self,
        row: &Row,
        today: NaiveDate,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let task_id = row.task.id.clone();
        let done = row.status == TaskStatus::Done;
        let selected = self
            .card
            .as_ref()
            .is_some_and(|card| card.task_id == task_id);
        let meta = Self::meta(row, today, theme);
        let hover = theme.glass_hover();
        let accent = theme.accent;
        let open_id = task_id.clone();
        let drop_before = task_id.clone();
        let title = SharedString::from(row.task.title.clone());
        let drag = TaskDrag {
            task_id: task_id.clone(),
            title: title.clone(),
            status: row.status,
        };
        div()
            .id(SharedString::from(format!("task-row-{task_id}")))
            .debug_selector(|| format!("task-row-{}", row.task.id))
            .role(gpui::Role::ListItem)
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
            .when(selected, |el| el.bg(theme.wash(0.08)))
            .when(!selected, |el| el.hover(move |s| s.bg(hover)))
            .child(self.checkbox(row, theme, cx))
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
                            .child(title.clone()),
                    )
                    .when(!meta.is_empty(), |el| {
                        el.child(widgets::meta_line(theme, meta))
                    }),
            )
            .on_click(cx.listener(move |page, _: &ClickEvent, window, cx| {
                page.open_card(open_id.clone(), None, false, Some(window), cx);
            }))
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

    fn add_row(&self, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
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
            .drag_over::<TaskDrag>(move |style, _, _, _| style.border_color(accent))
            .on_drop(cx.listener(|page, drag: &TaskDrag, _, cx| {
                page.reorder(&drag.task_id, None, cx);
            }))
            .capture_action(cx.listener(|page, _: &MentionTab, _, cx| {
                if page.submit_add(true, cx) {
                    cx.stop_propagation();
                }
            }))
            .on_key_down(cx.listener(|page, event: &KeyDownEvent, window, cx| {
                if event.keystroke.key == "escape" && !page.add.read(cx).text().is_empty() {
                    cx.stop_propagation();
                    page.add.update(cx, |input, cx| input.set_text("", cx));
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
            .into_any_element()
    }

    fn done_group(
        &self,
        done: &[Row],
        today: NaiveDate,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        if done.is_empty() {
            return None;
        }
        let open = self.done_open;
        let toggle = div()
            .id("tasks-done-toggle")
            .debug_selector(|| "tasks-done-toggle".into())
            .role(gpui::Role::Button)
            .aria_expanded(open)
            .px(px(12.0))
            .py(px(6.0))
            .flex()
            .items_center()
            .gap(px(6.0))
            .rounded(px(8.0))
            .cursor_pointer()
            .text_size(ui_rems(12.5))
            .font_weight(gpui::FontWeight::MEDIUM)
            .text_color(theme.text_muted)
            .hover(|s| s.text_color(theme.text))
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
                    .child(SharedString::from(done.len().to_string())),
            )
            .on_click(cx.listener(|page, _: &ClickEvent, _, cx| {
                page.done_open = !page.done_open;
                cx.notify();
            }));
        let rows: Vec<AnyElement> = if open {
            done.iter()
                .map(|row| self.list_row(row, today, theme, cx))
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
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let today = Self::today();
        let rows: Vec<AnyElement> = open
            .iter()
            .map(|row| self.list_row(row, today, theme, cx))
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
            .child(Self::header(theme, open.len()))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(2.0))
                    .children(rows)
                    .child(self.add_row(theme, cx)),
            )
            .children(self.done_group(done, today, theme, cx));
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

    fn kanban_card(&self, row: &Row, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let task_id = row.task.id.clone();
        let selected = self
            .card
            .as_ref()
            .is_some_and(|card| card.task_id == task_id);
        let title = SharedString::from(row.task.title.clone());
        let drag = TaskDrag {
            task_id: task_id.clone(),
            title: title.clone(),
            status: row.status,
        };
        let open_id = task_id.clone();
        let working = row.status == TaskStatus::Working;
        let footer = {
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
            .id(SharedString::from(format!("task-card-{task_id}")))
            .debug_selector(|| format!("task-kanban-{}", row.task.id))
            .p(px(12.0))
            .flex()
            .flex_col()
            .gap(px(8.0))
            .rounded(px(10.0))
            .bg(theme.surface_card)
            .border_1()
            .border_color(if selected { theme.accent } else { theme.border })
            .cursor_pointer()
            .hover(|s| s.border_color(theme.border_strong))
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
            .child(footer)
            .on_click(cx.listener(move |page, _: &ClickEvent, window, cx| {
                page.open_card(open_id.clone(), None, false, Some(window), cx);
            }))
            .on_drag(drag, |drag, _, _, cx| {
                cx.stop_propagation();
                let title = drag.title.clone();
                cx.new(|_| TaskDragGhost { title })
            })
            .into_any_element()
    }

    fn render_kanban(
        &self,
        open: &[Row],
        done: &[Row],
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let columns: Vec<AnyElement> = COLUMNS
            .iter()
            .map(|&column| {
                let rows: Vec<&Row> = open
                    .iter()
                    .chain(done.iter())
                    .filter(|row| row.status == column)
                    .collect();
                let accepts = matches!(column, TaskStatus::ToDo | TaskStatus::Done);
                let accent = theme.accent;
                let cards: Vec<AnyElement> = rows
                    .iter()
                    .map(|row| self.kanban_card(row, theme, cx))
                    .collect();
                let (_, _, tone) = column_tone(column, theme);
                div()
                    .id(SharedString::from(format!("tasks-column-{column:?}")))
                    .debug_selector(move || format!("tasks-column-{column:?}"))
                    .flex_1()
                    .min_w(px(180.0))
                    .h_full()
                    .flex()
                    .flex_col()
                    .gap(px(8.0))
                    .p(px(8.0))
                    .rounded(px(14.0))
                    .bg(widgets::block_fill(theme))
                    .border_1()
                    .border_color(gpui::transparent_black())
                    .child(
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
                                    .child(SharedString::from(rows.len().to_string())),
                            ),
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
            .child(Self::header(theme, open.len()))
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

    fn render_card(
        &self,
        row: Option<&Row>,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let card = self.card.as_ref()?;
        let today = Self::today();
        let task_id = card.task_id.clone();
        let status = row.map_or(TaskStatus::ToDo, |row| row.status);
        let (_, _, tone) = column_tone(status, theme);
        let close = div()
            .id("task-card-close")
            .debug_selector(|| "task-card-close".into())
            .role(gpui::Role::Button)
            .aria_label("Close")
            .size(px(24.0))
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(6.0))
            .cursor_pointer()
            .text_color(theme.text_muted)
            .hover(|s| s.bg(theme.glass_hover()).text_color(theme.text))
            .child(icon(icons::LUCIDE_X).size(px(14.0)))
            .on_click(cx.listener(|page, _: &ClickEvent, _, cx| page.close_card(cx)));
        let header = div()
            .flex()
            .items_center()
            .gap(px(8.0))
            .child(div().size(px(8.0)).rounded_full().bg(tone))
            .child(
                div()
                    .flex_1()
                    .text_size(ui_rems(12.0))
                    .text_color(theme.text_muted)
                    .child(logic::column_title(status)),
            )
            .child(close);

        let spaces: Vec<(String, String)> = self
            .state
            .read(cx)
            .spaces
            .iter()
            .map(|space| (space.id.clone(), space.display_name().to_string()))
            .collect();
        let current_space = row.and_then(|row| row.task.space_id.clone());
        let selected_space = current_space
            .as_deref()
            .and_then(|id| spaces.iter().position(|(sid, _)| sid == id))
            .map_or(0, |ix| ix + 1);
        let space_ids: Vec<String> = spaces.iter().map(|(id, _)| id.clone()).collect();
        let project_task = task_id.clone();
        let project = widgets::select("task-card-project", "Project", theme, |page: &mut Self| {
            &mut page.project_select
        })
        .options(
            std::iter::once(widgets::SelectOption::new("No project")).chain(
                spaces
                    .iter()
                    .map(|(_, name)| widgets::SelectOption::new(name.clone())),
            ),
            selected_space,
        )
        .width(180.0)
        .menu_width(220.0)
        .on_select(move |page, ix, _, cx| {
            let space = ix.checked_sub(1).and_then(|i| space_ids.get(i).cloned());
            page.update(
                &project_task,
                TaskPatch {
                    space_id: Some(space),
                    ..Default::default()
                },
                cx,
            );
        })
        .render(&self.project_select, cx);

        let due_on = row.and_then(|row| row.task.due_on.clone());
        let pick = logic::due_pick(due_on.as_deref(), today);
        let page = cx.weak_entity();
        let due_task = task_id.clone();
        let due = widgets::segmented(
            theme,
            "task-card-due",
            &[
                (DuePick::None, "None"),
                (DuePick::Today, "Today"),
                (DuePick::Tomorrow, "Tomorrow"),
            ],
            pick,
            move |pick, _, cx| {
                let due_task = due_task.clone();
                page.update(cx, |page, cx| {
                    page.update(
                        &due_task,
                        TaskPatch {
                            due_on: Some(logic::due_value(pick, Self::today())),
                            ..Default::default()
                        },
                        cx,
                    )
                })
                .ok();
            },
        );
        let due_note = (pick == DuePick::Other)
            .then(|| due_on.as_deref().and_then(|d| logic::due_label(d, today)))
            .flatten();

        let property = |label: &'static str, control: AnyElement| {
            div()
                .flex()
                .items_center()
                .justify_between()
                .gap(px(12.0))
                .child(
                    div()
                        .text_size(ui_rems(12.0))
                        .text_color(theme.text_muted)
                        .child(label),
                )
                .child(control)
        };

        let linked_block = row.and_then(|row| {
            let linked = row.linked.as_ref()?;
            let (mark, tint) = linked.brand;
            let (_, _, tone) = crate::shell::chat_status_tone(linked.indicator, theme);
            Some(
                div()
                    .px(px(10.0))
                    .py(px(8.0))
                    .rounded(px(8.0))
                    .bg(widgets::block_fill(theme))
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .text_size(ui_rems(12.0))
                    .child(
                        icon(mark)
                            .size(px(14.0))
                            .text_color(tint.unwrap_or(theme.text_muted)),
                    )
                    .child(
                        div()
                            .flex_1()
                            .text_color(tone)
                            .child(Self::linked_label(linked.indicator)),
                    )
                    .children(
                        linked
                            .since
                            .filter(|_| linked.indicator == ChatIndicator::Working)
                            .map(|since| {
                                crate::loaders::elapsed_label(
                                    format!("task-card-elapsed-{task_id}"),
                                    since,
                                    ui_rems(12.0),
                                    tone,
                                )
                            }),
                    ),
            )
        });

        let chat_id = row.and_then(|row| row.task.chat_id.clone());
        let done = status == TaskStatus::Done;
        let primary = match chat_id.clone() {
            Some(chat) => widgets::text_action(theme, widgets::ActionTone::Solid, "Open thread")
                .id("task-card-open-thread")
                .debug_selector(|| "task-card-open-thread".into())
                .on_click(cx.listener(move |_, _: &ClickEvent, _, cx| {
                    cx.emit(TasksEvent::OpenChat(chat.clone()));
                }))
                .into_any_element(),
            None if !done => widgets::action_button(theme, widgets::ActionTone::Solid)
                .id("task-card-start")
                .debug_selector(|| "task-card-start".into())
                .when(row.is_none(), |el| el.opacity(0.5))
                .child("Start")
                .child(
                    div()
                        .text_size(ui_rems(11.0))
                        .opacity(0.7)
                        .child(crate::settings::badge_combo("mod-enter")),
                )
                .on_click(cx.listener(|page, _: &ClickEvent, _, cx| page.start_card(cx)))
                .into_any_element(),
            None => div().into_any_element(),
        };
        let done_task = task_id.clone();
        let toggle_done = widgets::text_action(
            theme,
            widgets::ActionTone::Filled,
            if done {
                "Mark as not done"
            } else {
                "Mark as done"
            },
        )
        .id("task-card-toggle-done")
        .on_click(cx.listener(move |page, _: &ClickEvent, _, cx| {
            page.set_completed(&done_task, !done, cx);
        }));
        let delete_task = task_id.clone();
        let delete = div()
            .id("task-card-delete")
            .debug_selector(|| "task-card-delete".into())
            .role(gpui::Role::Button)
            .aria_label("Delete task")
            .size(px(32.0))
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(8.0))
            .cursor_pointer()
            .text_color(theme.text_muted)
            .hover(|s| s.bg(theme.glass_hover()).text_color(theme.danger))
            .child(
                icon(icons::TRASH_BIN_MINIMALISTIC)
                    .size(px(14.0))
                    .text_color(theme.text_muted),
            )
            .on_click(cx.listener(move |page, _: &ClickEvent, _, cx| {
                if let Some(card) = page.card.as_mut() {
                    card.save = None;
                }
                page.card = None;
                cx.emit(TasksEvent::Delete(delete_task.clone()));
                cx.notify();
            }));

        Some(
            div()
                .id("task-card")
                .debug_selector(|| "task-card".into())
                .w_full()
                .max_h_full()
                .overflow_y_scroll()
                .p(px(16.0))
                .flex()
                .flex_col()
                .gap(px(14.0))
                .rounded(px(16.0))
                .bg(theme.surface_raised)
                .border_1()
                .border_color(theme.border)
                .on_key_down(cx.listener(|page, event: &KeyDownEvent, _, cx| {
                    let modifiers = &event.keystroke.modifiers;
                    if event.keystroke.key == "enter" && (modifiers.platform || modifiers.control) {
                        cx.stop_propagation();
                        page.start_card(cx);
                    }
                }))
                .child(header)
                .child(
                    div()
                        .text_size(ui_rems(15.0))
                        .font_weight(gpui::FontWeight::MEDIUM)
                        .child(card.title.clone()),
                )
                .child(
                    div()
                        .id("task-card-notes")
                        .debug_selector(|| "task-card-notes".into())
                        .px(px(10.0))
                        .py(px(8.0))
                        .rounded(px(8.0))
                        .bg(widgets::block_fill(theme))
                        .text_size(ui_rems(13.0))
                        .capture_action(cx.listener(|page, _: &Submit, window, cx| {
                            let Some(card) = page.card.as_ref() else {
                                return;
                            };
                            if card.notes.focus_handle(cx).is_focused(window) {
                                cx.stop_propagation();
                                card.notes.update(cx, |input, cx| {
                                    input.replace_text_in_range(None, "\n", window, cx)
                                });
                            }
                        }))
                        .child(crate::composer::growing_field(
                            card.notes.clone(),
                            60.0,
                            220.0,
                            cx,
                        )),
                )
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(10.0))
                        .child(property("Project", project.into_any_element()))
                        .child(property("Due", due.into_any_element()))
                        .children(due_note.map(|note| {
                            div()
                                .text_size(ui_rems(11.5))
                                .text_color(theme.text_faint)
                                .child(note)
                        })),
                )
                .children(linked_block)
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(8.0))
                        .child(primary)
                        .child(toggle_done)
                        .child(div().flex_1())
                        .child(delete),
                )
                .into_any_element(),
        )
    }
}

fn column_tone(status: TaskStatus, theme: &Theme) -> (Option<SharedString>, &'static str, Hsla) {
    match status {
        TaskStatus::ToDo => (None, icons::LUCIDE_CIRCLE_DASHED, theme.text_faint),
        TaskStatus::Working => crate::shell::chat_status_tone(ChatIndicator::Working, theme),
        TaskStatus::NeedsYou => crate::shell::chat_status_tone(ChatIndicator::AwaitingInput, theme),
        TaskStatus::Done => crate::shell::chat_status_tone(ChatIndicator::Completed, theme),
    }
}

fn dashed_circle(theme: &Theme) -> AnyElement {
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

impl Render for TasksPage {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::of(cx).clone();
        let (open, done) = self.rows(cx);
        let view = settings::current(cx).tasks_view;
        let body = match view {
            TasksView::List => self.render_list(&open, &done, &theme, cx),
            TasksView::Kanban => self.render_kanban(&open, &done, &theme, cx),
        };
        let selected = self.card.as_ref().map(|card| card.task_id.clone());
        let row = selected
            .as_deref()
            .and_then(|id| open.iter().chain(done.iter()).find(|row| row.task.id == id));
        let card = self.render_card(row, &theme, cx);
        div()
            .id("tasks-page")
            .debug_selector(|| "tasks-page".into())
            .track_focus(&self.focus)
            .size_full()
            .flex()
            .flex_row()
            .on_key_down(cx.listener(|page, event: &KeyDownEvent, _, cx| {
                if event.keystroke.key == "escape" && page.card.is_some() {
                    cx.stop_propagation();
                    page.close_card(cx);
                }
            }))
            .child(div().flex_1().min_w_0().h_full().child(body))
            .children(card.map(|card| {
                div()
                    .flex_none()
                    .w(px(CARD_WIDTH))
                    .h_full()
                    .pt(px(Theme::TITLEBAR_HEIGHT + 12.0))
                    .pr(px(16.0))
                    .pb(px(16.0))
                    .child(card)
            }))
    }
}
