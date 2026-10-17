use std::collections::HashMap;
use std::time::Instant;

use chrono::Utc;
use gpui::{
    AnyElement, ClickEvent, Context, Entity, EntityInputHandler, EventEmitter, Focusable,
    IntoElement, KeyDownEvent, Render, Subscription, Task, Window, div, prelude::*, px,
};
use zeron_proto::{ChatIndicator, HarnessId, TaskPatch, TaskStatus};

use super::logic::{self, DuePick};
use super::{Row, SAVE_DEBOUNCE, START_PENDING, column_tone, linked_chat, patched, row_for};
use crate::composer::{ComposerInput, ComposerInputEvent, Submit};
use crate::icons::{self, icon};
use crate::pickers::Pickers;
use crate::settings::widgets;
use crate::state::AppState;
use crate::theme::Theme;
use crate::typography::ui_rems;

#[derive(Clone)]
pub enum TaskCardEvent {
    Update {
        task_id: String,
        patch: TaskPatch,
    },
    Delete(String),
    Start {
        task_id: String,
        patch: Option<TaskPatch>,
        harness: Option<HarnessId>,
        model: Option<String>,
    },
    OpenChat(String),
    Refused(String),
}

struct Open {
    task_id: String,
    title: Entity<ComposerInput>,
    notes: Entity<ComposerInput>,
    saved_title: String,
    saved_notes: String,
    refused: bool,
    seen: bool,
    save: Option<Task<()>>,
    _subscriptions: [Subscription; 2],
}

struct Fields {
    task_id: String,
    title: Entity<ComposerInput>,
    notes: Entity<ComposerInput>,
}

pub struct TaskCard {
    state: Entity<AppState>,
    open: Option<Open>,
    project_select: widgets::SelectState,
    starting: HashMap<String, Instant>,
    agent: Entity<Pickers>,
    _state_changes: Subscription,
}

impl EventEmitter<TaskCardEvent> for TaskCard {}

impl TaskCard {
    pub fn new(state: Entity<AppState>, cx: &mut Context<Self>) -> Self {
        let state_changes = cx.observe(&state, |card, _, cx| {
            card.sync(cx);
            cx.notify();
        });
        let agent = cx.new(|cx| Pickers::new_for_tasks(state.clone(), cx));
        Self {
            agent,
            state,
            open: None,
            project_select: widgets::SelectState::default(),
            starting: HashMap::new(),
            _state_changes: state_changes,
        }
    }

    pub fn task_id(&self) -> Option<&str> {
        self.open.as_ref().map(|open| open.task_id.as_str())
    }

    pub fn is_open(&self) -> bool {
        self.open.is_some()
    }

    /// `title` seeds the card of a task whose row has not arrived yet.
    pub fn open(
        &mut self,
        task_id: String,
        title: Option<String>,
        focus_notes: bool,
        window: Option<&mut Window>,
        cx: &mut Context<Self>,
    ) {
        if self.task_id() == Some(task_id.as_str()) {
            return;
        }
        self.flush(cx);
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
            let mut input = ComposerInput::new("Task title", cx)
                .with_single_line()
                .with_max_chars(zeron_doc::TASK_TITLE_MAX_CHARS);
            input.set_text(saved_title.clone(), cx);
            input
        });
        let notes_input = cx.new(|cx| {
            let mut input =
                ComposerInput::new("Add notes", cx).with_max_chars(zeron_doc::TASK_NOTES_MAX_CHARS);
            input.set_text(saved_notes.clone(), cx);
            input
        });
        let edits = |card: &mut Self,
                     _: Entity<ComposerInput>,
                     event: &ComposerInputEvent,
                     cx: &mut Context<Self>| match event {
            ComposerInputEvent::Edited => card.schedule_save(cx),
            ComposerInputEvent::Submitted => card.flush(cx),
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
        self.open = Some(Open {
            task_id,
            title: title_input,
            notes: notes_input,
            saved_title,
            saved_notes,
            refused: false,
            seen: row.is_some(),
            save: None,
            _subscriptions: subscriptions,
        });
        cx.notify();
    }

    pub fn close(&mut self, cx: &mut Context<Self>) {
        self.flush(cx);
        self.open = None;
        cx.notify();
    }

    fn sync(&mut self, cx: &mut Context<Self>) {
        let linked: Vec<String> = {
            let state = self.state.read(cx);
            self.starting
                .keys()
                .filter(|id| {
                    state
                        .tasks
                        .iter()
                        .any(|task| &task.id == *id && linked_chat(&state.chats, task).is_some())
                })
                .cloned()
                .collect()
        };
        for id in linked {
            self.starting.remove(&id);
        }
        self.starting.retain(|_, at| at.elapsed() < START_PENDING);
        let Some(open) = self.open.as_mut() else {
            return;
        };
        let present = self
            .state
            .read(cx)
            .tasks
            .iter()
            .any(|task| task.id == open.task_id && !task.deleted);
        if present {
            open.seen = true;
        } else if open.seen {
            self.open = None;
        }
    }

    fn schedule_save(&mut self, cx: &mut Context<Self>) {
        let Some(open) = self.open.as_mut() else {
            return;
        };
        open.save = Some(cx.spawn(async move |card, cx| {
            cx.background_executor().timer(SAVE_DEBOUNCE).await;
            card.update(cx, |card, cx| card.flush(cx)).ok();
        }));
    }

    /// Notes too large to sync stay unsaved, so a later flush retries them once shortened.
    fn take_patch(&mut self, cx: &mut Context<Self>) -> Option<(String, TaskPatch)> {
        let open = self.open.as_mut()?;
        open.save = None;
        let title = open.title.read(cx).text().trim().to_string();
        let notes = open.notes.read(cx).text().to_string();
        let mut patch = TaskPatch::default();
        if !title.is_empty() && title != open.saved_title {
            patch.title = Some(title);
        }
        let notes_dirty = notes != open.saved_notes;
        if notes_dirty {
            patch.notes = Some(notes);
        }
        if patch == TaskPatch::default() {
            return None;
        }
        let refusal = self
            .state
            .read(cx)
            .tasks
            .iter()
            .find(|task| task.id == open.task_id)
            .and_then(|row| zeron_doc::check_task_syncs(&patched(row, &patch)).err());
        if refusal.is_some() {
            patch.notes = None;
        }
        if let Some(error) = refusal.filter(|_| !open.refused) {
            cx.emit(TaskCardEvent::Refused(error.to_string()));
        }
        open.refused = notes_dirty && patch.notes.is_none();
        if let Some(title) = &patch.title {
            open.saved_title = title.clone();
        }
        if let Some(notes) = &patch.notes {
            open.saved_notes = notes.clone();
        }
        (patch != TaskPatch::default()).then(|| (open.task_id.clone(), patch))
    }

    pub fn save_failed(&mut self, task_id: &str, patch: &TaskPatch, cx: &mut Context<Self>) {
        let row = self
            .state
            .read(cx)
            .tasks
            .iter()
            .find(|task| task.id == task_id)
            .cloned();
        let Some(open) = self.open.as_mut().filter(|open| open.task_id == task_id) else {
            return;
        };
        if patch.title.as_ref() == Some(&open.saved_title) {
            open.saved_title = row.as_ref().map(|t| t.title.clone()).unwrap_or_default();
        }
        if patch.notes.as_ref() == Some(&open.saved_notes) {
            open.saved_notes = row.map(|t| t.notes).unwrap_or_default();
        }
    }

    pub fn start_failed(&mut self, task_id: &str, cx: &mut Context<Self>) {
        self.starting.remove(task_id);
        cx.notify();
    }

    fn flush(&mut self, cx: &mut Context<Self>) {
        if let Some((task_id, patch)) = self.take_patch(cx) {
            cx.emit(TaskCardEvent::Update { task_id, patch });
        }
    }

    fn start(&mut self, cx: &mut Context<Self>) {
        let Some(task_id) = self.task_id().map(str::to_owned) else {
            return;
        };
        let state = self.state.read(cx);
        let startable = state.tasks.iter().any(|task| {
            task.id == task_id
                && linked_chat(&state.chats, task).is_none()
                && task.completed_at.is_none()
        });
        if !startable || self.is_starting(&task_id) {
            return;
        }
        self.starting.insert(task_id.clone(), Instant::now());
        cx.spawn(async move |card, cx| {
            cx.background_executor().timer(START_PENDING).await;
            card.update(cx, |_, cx| cx.notify()).ok();
        })
        .detach();
        let patch = self.take_patch(cx).map(|(_, patch)| patch);
        let (harness, model) = self.agent.read(cx).task_harness_model();
        cx.emit(TaskCardEvent::Start {
            task_id,
            patch,
            harness,
            model,
        });
        cx.notify();
    }

    /// Covers the gap until the link lands; a failed start frees it on expiry.
    fn is_starting(&self, task_id: &str) -> bool {
        self.starting
            .get(task_id)
            .is_some_and(|at| at.elapsed() < START_PENDING)
    }

    fn update(&mut self, task_id: &str, patch: TaskPatch, cx: &mut Context<Self>) {
        cx.emit(TaskCardEvent::Update {
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

    fn today() -> chrono::NaiveDate {
        chrono::Local::now().date_naive()
    }

    fn render_card(
        &self,
        fields: Fields,
        row: Option<&Row>,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let today = Self::today();
        let Fields {
            task_id,
            title,
            notes,
        } = fields;
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
            .on_click(cx.listener(|card, _: &ClickEvent, _, cx| card.close(cx)));
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
        let project = widgets::select("task-card-project", "Project", theme, |card: &mut Self| {
            &mut card.project_select
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
        .on_select(move |card, ix, _, cx| {
            let space = ix.checked_sub(1).and_then(|i| space_ids.get(i).cloned());
            card.update(
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
        let weak = cx.weak_entity();
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
                weak.update(cx, |card, cx| {
                    card.update(
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
                            .child(super::TasksPage::linked_label(linked.indicator)),
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

        let starting = self.is_starting(&task_id);
        let chat_id = row
            .filter(|row| row.linked.is_some())
            .and_then(|row| row.task.chat_id.clone());
        let done = status == TaskStatus::Done;
        let primary = match chat_id.clone() {
            Some(chat) => widgets::text_action(theme, widgets::ActionTone::Solid, "Open thread")
                .id("task-card-open-thread")
                .debug_selector(|| "task-card-open-thread".into())
                .on_click(cx.listener(move |_, _: &ClickEvent, _, cx| {
                    cx.emit(TaskCardEvent::OpenChat(chat.clone()));
                }))
                .into_any_element(),
            None if !done => widgets::action_button(theme, widgets::ActionTone::Solid)
                .id("task-card-start")
                .debug_selector(|| "task-card-start".into())
                .when(row.is_none() || starting, |el| el.opacity(0.5))
                .child(if starting { "Starting…" } else { "Start" })
                .child(
                    div()
                        .text_size(ui_rems(11.0))
                        .opacity(0.7)
                        .child(crate::settings::badge_combo("mod-enter")),
                )
                .on_click(cx.listener(|card, _: &ClickEvent, _, cx| card.start(cx)))
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
        .on_click(cx.listener(move |card, _: &ClickEvent, _, cx| {
            card.set_completed(&done_task, !done, cx);
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
            .on_click(cx.listener(move |card, _: &ClickEvent, _, cx| {
                if let Some(open) = card.open.as_mut() {
                    open.save = None;
                }
                card.open = None;
                cx.emit(TaskCardEvent::Delete(delete_task.clone()));
                cx.notify();
            }));

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
            .on_key_down(cx.listener(|card, event: &KeyDownEvent, _, cx| {
                let modifiers = &event.keystroke.modifiers;
                if event.keystroke.key == "enter" && (modifiers.platform || modifiers.control) {
                    cx.stop_propagation();
                    card.start(cx);
                }
            }))
            .child(header)
            .child(
                div()
                    .text_size(ui_rems(15.0))
                    .font_weight(gpui::FontWeight::MEDIUM)
                    .child(title),
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
                    .capture_action(cx.listener(|card, _: &Submit, window, cx| {
                        let Some(open) = card.open.as_ref() else {
                            return;
                        };
                        if open.notes.focus_handle(cx).is_focused(window) {
                            cx.stop_propagation();
                            open.notes.update(cx, |input, cx| {
                                input.replace_text_in_range(None, "\n", window, cx)
                            });
                        }
                    }))
                    .child(crate::composer::growing_field(notes, 60.0, 220.0, cx)),
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
            .into_any_element()
    }
}

impl Render for TaskCard {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::of(cx).clone();
        let Some(open) = self.open.as_ref() else {
            return div().into_any_element();
        };
        let row = {
            let state = self.state.read(cx);
            let now = Utc::now();
            state
                .tasks
                .iter()
                .find(|task| task.id == open.task_id && !task.deleted)
                .map(|task| row_for(state, task, now))
        };
        let fields = Fields {
            task_id: open.task_id.clone(),
            title: open.title.clone(),
            notes: open.notes.clone(),
        };
        self.render_card(fields, row.as_ref(), &theme, cx)
    }
}
