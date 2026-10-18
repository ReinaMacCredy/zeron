use std::collections::HashMap;
use std::time::Instant;

use chrono::Utc;
use gpui::{
    AnyElement, ClickEvent, Context, Entity, EntityInputHandler, EventEmitter, FocusHandle,
    Focusable, IntoElement, KeyDownEvent, PathPromptOptions, Render, Subscription, Task, Window,
    div, prelude::*, px,
};
use zeron_proto::{ChatIndicator, HarnessId, TaskPatch, TaskStatus};

use super::logic::{self, DuePick};
use super::{Row, SAVE_DEBOUNCE, START_PENDING, linked_chat, patched, row_for};
use crate::composer::{ComposerInput, ComposerInputEvent, Submit};
use crate::icons::{self, icon};
use crate::pickers::Pickers;
use crate::settings::widgets;
use crate::state::AppState;
use crate::theme::Theme;
use crate::typography::ui_rems;

const PILL_HEIGHT: f32 = 28.0;
const CARD_RADIUS: f32 = 24.0;

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
        space_id: Option<String>,
        harness: Option<HarnessId>,
        model: Option<String>,
    },
    CreateSpace {
        space_id: String,
        device_id: String,
        path: String,
    },
    Assign {
        task_id: String,
        patch: Option<TaskPatch>,
        chat_id: String,
        send: bool,
    },
    OpenChat(String),
    Refused(String),
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum CardFocus {
    Title,
    Notes,
    Agent,
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
    run_in: Option<Option<String>>,
    folder: Option<String>,
    more: bool,
    assign: Entity<ComposerInput>,
    assign_pick: Option<(String, String)>,
    assign_send: bool,
    _subscriptions: [Subscription; 3],
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
    due_select: widgets::SelectState,
    run_select: widgets::SelectState,
    starting: HashMap<String, Instant>,
    agent: Entity<Pickers>,
    start_focus: FocusHandle,
    folder_prompt: Option<Task<()>>,
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
            due_select: widgets::SelectState::default(),
            run_select: widgets::SelectState::default(),
            starting: HashMap::new(),
            start_focus: cx.focus_handle(),
            folder_prompt: None,
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
        focus: CardFocus,
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
            let mut input = ComposerInput::new("Add a note", cx)
                .with_max_chars(zeron_doc::TASK_NOTES_MAX_CHARS);
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
        let assign_input = cx.new(|cx| {
            ComposerInput::new("@ a thread", cx)
                .with_single_line()
                .with_max_chars(200)
        });
        let subscriptions = [
            cx.subscribe(&title_input, edits),
            cx.subscribe(&notes_input, edits),
            cx.subscribe(&assign_input, |card: &mut Self, input, event, cx| {
                match event {
                    ComposerInputEvent::Edited => {
                        let text = input.read(cx).text().to_string();
                        if let Some(open) = card.open.as_mut()
                            && open
                                .assign_pick
                                .as_ref()
                                .is_some_and(|(_, title)| text != format!("@{title}"))
                        {
                            open.assign_pick = None;
                        }
                    }
                    ComposerInputEvent::Submitted => {
                        if card
                            .open
                            .as_ref()
                            .is_some_and(|open| open.assign_pick.is_some())
                        {
                            card.assign(cx);
                        } else if let Some(first) = card.assign_matches(cx).into_iter().next() {
                            card.pick_thread(first.chat_id, first.title, cx);
                        }
                    }
                    _ => {}
                }
                cx.notify();
            }),
        ];
        if let Some(window) = window {
            let target = match focus {
                CardFocus::Title => title_input.focus_handle(cx),
                CardFocus::Notes => notes_input.focus_handle(cx),
                CardFocus::Agent => self.start_focus.clone(),
            };
            window.focus(&target, cx);
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
            run_in: None,
            folder: None,
            more: false,
            assign: assign_input,
            assign_pick: None,
            assign_send: true,
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
        let space_id = self
            .open
            .as_ref()
            .and_then(|open| open.run_in.clone())
            .flatten();
        let (harness, model) = self.agent.read(cx).task_harness_model();
        cx.emit(TaskCardEvent::Start {
            task_id,
            patch,
            space_id,
            harness,
            model,
        });
        cx.notify();
    }

    fn assign_matches(&self, cx: &Context<Self>) -> Vec<crate::composer::ThreadMatch> {
        let Some(open) = self.open.as_ref() else {
            return Vec::new();
        };
        let text = open.assign.read(cx).text().trim();
        if text.is_empty() || open.assign_pick.is_some() {
            return Vec::new();
        }
        let query = text.strip_prefix('@').unwrap_or(text);
        let state = self.state.read(cx);
        let Some(device) = state.local_device_id.as_deref() else {
            return Vec::new();
        };
        let space = state
            .tasks
            .iter()
            .find(|task| task.id == open.task_id)
            .and_then(|task| task.space_id.as_deref());
        crate::composer::thread_matches(
            state.visible_chats(),
            query,
            crate::composer::MentionScope {
                device_id: device,
                space_id: space,
                exclude: None,
            },
            |id| state.is_draft(id),
            |chat| {
                state
                    .space_for_chat(chat)
                    .map(|space| space.display_name().to_string())
                    .unwrap_or_default()
            },
            |_| Vec::new(),
        )
    }

    fn pick_thread(&mut self, chat_id: String, title: String, cx: &mut Context<Self>) {
        let Some(open) = self.open.as_mut() else {
            return;
        };
        let label = format!("@{title}");
        open.assign_pick = Some((chat_id, title));
        open.assign
            .update(cx, |input, cx| input.set_text(label, cx));
        cx.notify();
    }

    fn assign(&mut self, cx: &mut Context<Self>) {
        let Some((task_id, (chat_id, _), send)) = self.open.as_ref().and_then(|open| {
            Some((
                open.task_id.clone(),
                open.assign_pick.clone()?,
                open.assign_send,
            ))
        }) else {
            return;
        };
        let patch = self.take_patch(cx).map(|(_, patch)| patch);
        if let Some(open) = self.open.as_mut() {
            open.assign_pick = None;
            open.assign.update(cx, |input, cx| input.set_text("", cx));
        }
        cx.emit(TaskCardEvent::Assign {
            task_id,
            patch,
            chat_id,
            send,
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

    fn pick_folder(&mut self, cx: &mut Context<Self>) {
        let rx = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("Run here".into()),
        });
        self.folder_prompt = Some(cx.spawn(async move |card, cx| {
            let picked = rx.await;
            card.update(cx, |card, cx| {
                card.folder_prompt = None;
                if let Ok(Ok(Some(paths))) = picked
                    && let Some(path) = paths.into_iter().next()
                {
                    card.run_in_folder(path.to_string_lossy().into_owned(), cx);
                }
            })
            .ok();
        }));
    }

    fn run_in_folder(&mut self, path: String, cx: &mut Context<Self>) {
        let state = self.state.read(cx);
        let Some(device_id) = state.local_device_id.clone() else {
            return;
        };
        let existing = state
            .spaces
            .iter()
            .find(|space| {
                space.device_id == device_id && crate::pickers::same_folder_path(&space.path, &path)
            })
            .map(|space| space.id.clone());
        let space_id = existing.unwrap_or_else(|| {
            let space_id = uuid::Uuid::new_v4().to_string();
            cx.emit(TaskCardEvent::CreateSpace {
                space_id: space_id.clone(),
                device_id,
                path: path.clone(),
            });
            space_id
        });
        if let Some(open) = self.open.as_mut() {
            open.run_in = Some(Some(space_id));
            open.folder = Some(path);
        }
        cx.notify();
    }

    fn toggle_more(&mut self, cx: &mut Context<Self>) {
        if let Some(open) = self.open.as_mut() {
            open.more = !open.more;
        }
        cx.notify();
    }

    fn pill(theme: &Theme, id: &'static str) -> gpui::Stateful<gpui::Div> {
        let hover = theme.wash(0.10);
        let text = theme.text;
        div()
            .id(id)
            .debug_selector(move || id.to_string())
            .role(gpui::Role::Button)
            .flex_none()
            .h(px(PILL_HEIGHT))
            .min_w(px(PILL_HEIGHT))
            .px(px(9.0))
            .flex()
            .items_center()
            .justify_center()
            .gap(px(6.0))
            .rounded_full()
            .bg(theme.wash(0.06))
            .cursor_pointer()
            .text_size(ui_rems(12.5))
            .text_color(theme.text_muted)
            .hover(move |s| s.bg(hover).text_color(text))
    }

    fn render_chips(
        &self,
        task_id: &str,
        row: Option<&Row>,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let today = Self::today();
        let due_on = row.and_then(|row| row.task.due_on.clone());
        let pick = logic::due_pick(due_on.as_deref(), today);
        let muted = theme.text_muted;
        let calendar = move || {
            icon(icons::CALENDAR)
                .size(px(14.0))
                .flex_none()
                .text_color(muted)
                .into_any_element()
        };
        let mut due_options = vec![
            widgets::SelectOption::new("No date").leading(calendar),
            widgets::SelectOption::new("Today").leading(calendar),
            widgets::SelectOption::new("Tomorrow").leading(calendar),
        ];
        let other = (pick == DuePick::Other)
            .then(|| due_on.as_deref().and_then(|d| logic::due_label(d, today)))
            .flatten();
        if let Some(label) = &other {
            let label = label.strip_prefix("due ").unwrap_or(label).to_owned();
            due_options.push(widgets::SelectOption::new(label).leading(calendar));
        }
        let due_selected = match pick {
            DuePick::None => 0,
            DuePick::Today => 1,
            DuePick::Tomorrow => 2,
            DuePick::Other => 3,
        };
        let due_task = task_id.to_owned();
        let due = widgets::select("task-card-due", "Due", theme, |card: &mut Self| {
            &mut card.due_select
        })
        .options(due_options, due_selected)
        .menu_width(180.0)
        .open_rightward()
        .on_select(move |card, ix, _, cx| {
            let pick = match ix {
                0 => DuePick::None,
                1 => DuePick::Today,
                2 => DuePick::Tomorrow,
                _ => return,
            };
            card.update(
                &due_task,
                TaskPatch {
                    due_on: Some(logic::due_value(pick, Self::today())),
                    ..Default::default()
                },
                cx,
            );
        })
        .render(&self.due_select, cx)
        .h(px(PILL_HEIGHT))
        .rounded_full();

        let spaces: Vec<(String, String)> = self
            .state
            .read(cx)
            .spaces
            .iter()
            .map(|space| (space.id.clone(), space.display_name().to_string()))
            .collect();
        let muted = theme.text_muted;
        let folder = move || {
            icon(icons::LUCIDE_FOLDER)
                .size(px(14.0))
                .flex_none()
                .text_color(muted)
                .into_any_element()
        };
        let selected_space = row
            .and_then(|row| row.task.space_id.as_deref())
            .and_then(|id| spaces.iter().position(|(sid, _)| sid == id))
            .map_or(0, |ix| ix + 1);
        let space_ids: Vec<String> = spaces.iter().map(|(id, _)| id.clone()).collect();
        let project_task = task_id.to_owned();
        let project = widgets::select("task-card-project", "Project", theme, |card: &mut Self| {
            &mut card.project_select
        })
        .options(
            std::iter::once(widgets::SelectOption::new("No project").leading(folder)).chain(
                spaces
                    .iter()
                    .map(|(_, name)| widgets::SelectOption::new(name.clone()).leading(folder)),
            ),
            selected_space,
        )
        .menu_width(220.0)
        .open_rightward()
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
        .render(&self.project_select, cx)
        .h(px(PILL_HEIGHT))
        .rounded_full();

        let more = Self::pill(theme, "task-card-more")
            .aria_label("More options")
            .child(
                icon(icons::LUCIDE_ELLIPSIS)
                    .size(px(14.0))
                    .text_color(theme.text_muted),
            )
            .on_click(cx.listener(|card, _: &ClickEvent, _, cx| card.toggle_more(cx)));
        div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap(px(6.0))
            .child(due)
            .child(project)
            .child(more)
            .into_any_element()
    }

    fn render_hand_off(
        &self,
        task_id: &str,
        row: Option<&Row>,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let assign = self.render_assign(theme, cx);
        let open = self.open.as_ref();
        let picked = open.is_some_and(|open| open.assign_pick.is_some());
        let state = self.state.read(cx);
        let local = state.local_device_id.clone();
        let spaces: Vec<(String, String)> = state
            .spaces
            .iter()
            .filter(|space| local.as_deref() == Some(space.device_id.as_str()))
            .map(|space| (space.id.clone(), space.display_name().to_string()))
            .collect();
        let run_in = open
            .and_then(|open| open.run_in.clone())
            .unwrap_or_else(|| row.and_then(|row| row.task.space_id.clone()));
        let pending_folder = open
            .and_then(|open| open.folder.clone())
            .filter(|_| {
                run_in
                    .as_deref()
                    .is_some_and(|id| !spaces.iter().any(|(sid, _)| sid == id))
            })
            .map(|path| {
                std::path::Path::new(&path)
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or(path)
            });
        let muted = theme.text_muted;
        let folder = move || {
            icon(icons::LUCIDE_FOLDER)
                .size(px(14.0))
                .flex_none()
                .text_color(muted)
                .into_any_element()
        };
        let mut options: Vec<widgets::SelectOption> =
            std::iter::once(widgets::SelectOption::new("No project").leading(folder))
                .chain(
                    spaces
                        .iter()
                        .map(|(_, name)| widgets::SelectOption::new(name.clone()).leading(folder)),
                )
                .collect();
        let selected = match &pending_folder {
            Some(name) => {
                options.push(widgets::SelectOption::new(name.clone()).leading(folder));
                options.len() - 1
            }
            None => run_in
                .as_deref()
                .and_then(|id| spaces.iter().position(|(sid, _)| sid == id))
                .map_or(0, |ix| ix + 1),
        };
        let space_ids: Vec<String> = spaces.iter().map(|(id, _)| id.clone()).collect();
        let run_select = widgets::select("task-card-run-in", "Run in", theme, |card: &mut Self| {
            &mut card.run_select
        })
        .options(options, selected)
        .menu_width(220.0)
        .open_rightward()
        .on_select(move |card, ix, _, cx| {
            let space = match ix.checked_sub(1) {
                None => None,
                Some(i) => match space_ids.get(i) {
                    Some(id) => Some(id.clone()),
                    None => return,
                },
            };
            if let Some(open) = card.open.as_mut() {
                open.run_in = Some(space);
                open.folder = None;
            }
            cx.notify();
        })
        .render(&self.run_select, cx)
        .max_w(px(150.0));
        let folder_button = div()
            .id("task-card-folder")
            .debug_selector(|| "task-card-folder".into())
            .role(gpui::Role::Button)
            .flex_none()
            .px(px(8.0))
            .py(px(4.0))
            .rounded(px(8.0))
            .cursor_pointer()
            .text_size(ui_rems(12.5))
            .text_color(theme.text_muted)
            .hover(|s| s.bg(theme.glass_hover()).text_color(theme.text))
            .child("Folder…")
            .on_click(cx.listener(|card, _: &ClickEvent, _, cx| card.pick_folder(cx)));

        let starting = self.is_starting(task_id);
        let start = widgets::action_button(theme, widgets::ActionTone::Solid)
            .id("task-card-start")
            .debug_selector(|| "task-card-start".into())
            .role(gpui::Role::Button)
            .track_focus(&self.start_focus)
            .tab_index(0)
            .rounded_full()
            .min_h(px(30.0))
            .px(px(14.0))
            .when(row.is_none() || starting, |el| el.opacity(0.5))
            .child(if starting { "Starting…" } else { "Start" })
            .on_click(cx.listener(|card, _: &ClickEvent, _, cx| card.start(cx)));
        let assign_button = widgets::action_button(theme, widgets::ActionTone::Solid)
            .id("task-card-assign-button")
            .debug_selector(|| "task-card-assign-button".into())
            .role(gpui::Role::Button)
            .tab_index(0)
            .rounded_full()
            .min_h(px(30.0))
            .px(px(14.0))
            .child("Assign")
            .on_click(cx.listener(|card, _: &ClickEvent, _, cx| card.assign(cx)));
        let more_open = open.is_some_and(|open| open.more);
        let more = div()
            .id("task-card-more-options")
            .debug_selector(|| "task-card-more-options".into())
            .role(gpui::Role::Button)
            .aria_expanded(more_open)
            .cursor_pointer()
            .text_size(ui_rems(12.5))
            .text_color(theme.text_muted)
            .hover(|s| s.text_color(theme.text))
            .child(if more_open {
                "Fewer options"
            } else {
                "More options"
            })
            .on_click(cx.listener(|card, _: &ClickEvent, _, cx| card.toggle_more(cx)));

        div()
            .id("task-card-hand-off")
            .debug_selector(|| "task-card-hand-off".into())
            .flex()
            .flex_col()
            .gap(px(8.0))
            .child(
                div()
                    .text_size(ui_rems(12.5))
                    .text_color(theme.text_muted)
                    .child("Hand it to an agent"),
            )
            .child(
                div()
                    .ml(px(-6.0))
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap(px(4.0))
                    .child(div().min_w_0().child(self.agent.clone()))
                    .child(run_select)
                    .child(folder_button),
            )
            .child(assign)
            .child(
                div()
                    .pt(px(2.0))
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(more)
                    .map(|el| {
                        if picked {
                            el.child(assign_button)
                        } else {
                            el.child(start)
                        }
                    }),
            )
            .into_any_element()
    }

    fn render_assign(&self, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let Some(open) = self.open.as_ref() else {
            return div().into_any_element();
        };
        let picked = open.assign_pick.is_some();
        let send = open.assign_send;
        let matches = self.assign_matches(cx);
        let state = self.state.read(cx);
        let now = Utc::now();
        let rows: Vec<AnyElement> = matches
            .into_iter()
            .enumerate()
            .map(|(ix, found)| {
                let chat = state.chats.iter().find(|chat| chat.id == found.chat_id);
                let (mark, tint) = crate::pickers::agent_brand_icon(
                    chat.and_then(|chat| chat.config.as_ref().map(|config| config.harness)),
                    chat.and_then(|chat| chat.config.as_ref()?.model.as_deref()),
                );
                let (label, _, tone) = chat
                    .map(|chat| {
                        crate::shell::chat_status_tone(state.display_status_for(chat, now), theme)
                    })
                    .unwrap_or((None, "", theme.text_muted));
                let (chat_id, title) = (found.chat_id.clone(), found.title.clone());
                div()
                    .id(("task-card-assign-match", ix))
                    .debug_selector(move || format!("task-card-assign-match-{ix}"))
                    .role(gpui::Role::Button)
                    .aria_label(found.title.clone())
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .px(px(8.0))
                    .py(px(6.0))
                    .rounded(px(8.0))
                    .cursor_pointer()
                    .when(ix == 0, |el| el.bg(theme.wash(0.06)))
                    .hover(|s| s.bg(theme.glass_hover()))
                    .child(
                        icon(mark)
                            .size(px(14.0))
                            .flex_none()
                            .text_color(tint.unwrap_or(theme.text_muted)),
                    )
                    .child(div().flex_1().min_w_0().truncate().child(found.title))
                    .when(!found.project.is_empty(), |el| {
                        el.child(
                            div()
                                .flex_none()
                                .max_w(px(90.0))
                                .truncate()
                                .text_color(theme.text_muted)
                                .child(found.project),
                        )
                    })
                    .children(label.map(|label| div().flex_none().text_color(tone).child(label)))
                    .on_click(cx.listener(move |card, _: &ClickEvent, _, cx| {
                        card.pick_thread(chat_id.clone(), title.clone(), cx);
                    }))
                    .into_any_element()
            })
            .collect();
        let weak = cx.entity().downgrade();
        let mode = widgets::segmented(
            theme,
            "task-card-assign-mode",
            &[(true, "Send the task"), (false, "Link only")],
            send,
            move |send, _, cx| {
                weak.update(cx, |card, cx| {
                    if let Some(open) = card.open.as_mut() {
                        open.assign_send = send;
                    }
                    cx.notify();
                })
                .ok();
            },
        );
        div()
            .flex()
            .flex_col()
            .gap(px(6.0))
            .text_size(ui_rems(12.5))
            .child(
                div()
                    .text_color(theme.text_muted)
                    .child("or assign to a thread"),
            )
            .child(
                div()
                    .id("task-card-assign")
                    .debug_selector(|| "task-card-assign".into())
                    .px(px(10.0))
                    .py(px(6.0))
                    .rounded(px(10.0))
                    .bg(theme.wash(0.06))
                    .child(open.assign.clone()),
            )
            .when(!rows.is_empty(), |el| {
                el.child(
                    div()
                        .id("task-card-assign-matches")
                        .debug_selector(|| "task-card-assign-matches".into())
                        .p(px(4.0))
                        .rounded(px(10.0))
                        .border_1()
                        .border_color(theme.border)
                        .flex()
                        .flex_col()
                        .children(rows),
                )
            })
            .when(picked, |el| el.child(mode))
            .into_any_element()
    }

    fn render_card(
        &self,
        fields: Fields,
        row: Option<&Row>,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Fields {
            task_id,
            title,
            notes,
        } = fields;
        let status = row.map_or(TaskStatus::ToDo, |row| row.status);
        let done = status == TaskStatus::Done;
        let close = div()
            .id("task-card-close")
            .debug_selector(|| "task-card-close".into())
            .role(gpui::Role::Button)
            .aria_label("Close")
            .flex_none()
            .size(px(24.0))
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(6.0))
            .cursor_pointer()
            .text_color(theme.text_muted)
            .hover(|s| s.bg(theme.glass_hover()).text_color(theme.text))
            .child(
                icon(icons::LUCIDE_X)
                    .size(px(14.0))
                    .text_color(theme.text_muted),
            )
            .on_click(cx.listener(|card, _: &ClickEvent, _, cx| card.close(cx)));
        let header = div()
            .flex()
            .items_start()
            .gap(px(8.0))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .text_size(ui_rems(15.0))
                    .font_weight(gpui::FontWeight::MEDIUM)
                    .child(title),
            )
            .child(close);
        let note = div()
            .id("task-card-notes")
            .debug_selector(|| "task-card-notes".into())
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
            .child(crate::composer::growing_field(notes, 20.0, 220.0, cx));
        let chips = self.render_chips(&task_id, row, theme, cx);

        let linked_chat_id = row
            .filter(|row| row.linked.is_some())
            .and_then(|row| row.task.chat_id.clone());
        let open_chat = |label: &'static str, cx: &mut Context<Self>| {
            linked_chat_id.clone().map(|chat| {
                widgets::text_action(theme, widgets::ActionTone::Solid, label)
                    .id("task-card-open-thread")
                    .debug_selector(|| "task-card-open-thread".into())
                    .rounded_full()
                    .on_click(cx.listener(move |_, _: &ClickEvent, _, cx| {
                        cx.emit(TaskCardEvent::OpenChat(chat.clone()));
                    }))
            })
        };
        let unassign_task = task_id.clone();
        let linked_title = linked_chat_id
            .as_deref()
            .and_then(|id| self.state.read(cx).chats.iter().find(|chat| chat.id == id))
            .map(|chat| {
                crate::transcript::single_line(chat.title.as_deref().unwrap_or("New session"))
            })
            .unwrap_or_default();
        let body = if done {
            None
        } else if linked_chat_id.is_some() {
            let linked_block = row.and_then(|row| {
                let linked = row.linked.as_ref()?;
                let (mark, tint) = linked.brand;
                let (_, _, tone) = crate::shell::chat_status_tone(linked.indicator, theme);
                Some(
                    div()
                        .px(px(12.0))
                        .py(px(10.0))
                        .rounded(px(14.0))
                        .bg(theme.wash(0.06))
                        .flex()
                        .items_center()
                        .gap(px(8.0))
                        .text_size(ui_rems(12.5))
                        .child(
                            icon(mark)
                                .size(px(14.0))
                                .text_color(tint.unwrap_or(theme.text_muted)),
                        )
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .truncate()
                                .text_color(theme.text)
                                .child(linked_title.clone()),
                        )
                        .child(
                            div()
                                .flex_none()
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
            let not_sent = row
                .and_then(|row| row.task.send_error.clone())
                .zip(linked_chat_id.clone())
                .map(|(error, chat_id)| {
                    let retry_task = task_id.clone();
                    widgets::warning_strip(theme, format!("Not sent: {error}"))
                        .debug_selector(|| "task-card-not-sent".into())
                        .mt_0()
                        .child(div().flex_1())
                        .child(
                            widgets::text_action(theme, widgets::ActionTone::Filled, "Retry")
                                .id("task-card-retry-send")
                                .debug_selector(|| "task-card-retry-send".into())
                                .role(gpui::Role::Button)
                                .flex_none()
                                .min_h(px(24.0))
                                .rounded_full()
                                .on_click(cx.listener(move |_, _: &ClickEvent, _, cx| {
                                    cx.emit(TaskCardEvent::Assign {
                                        task_id: retry_task.clone(),
                                        patch: None,
                                        chat_id: chat_id.clone(),
                                        send: true,
                                    });
                                })),
                        )
                });
            let unassign = widgets::text_action(theme, widgets::ActionTone::Quiet, "Unassign")
                .id("task-card-unassign")
                .debug_selector(|| "task-card-unassign".into())
                .role(gpui::Role::Button)
                .rounded_full()
                .on_click(cx.listener(move |card, _: &ClickEvent, _, cx| {
                    card.update(
                        &unassign_task,
                        TaskPatch {
                            chat_id: Some(None),
                            ..Default::default()
                        },
                        cx,
                    );
                }));
            Some(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(8.0))
                    .children(linked_block)
                    .children(not_sent)
                    .child(
                        div()
                            .flex()
                            .justify_end()
                            .gap(px(6.0))
                            .child(unassign)
                            .children(open_chat("Open thread", cx)),
                    )
                    .into_any_element(),
            )
        } else {
            Some(self.render_hand_off(&task_id, row, theme, cx))
        };

        let more_open = self.open.as_ref().is_some_and(|open| open.more);
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
        .debug_selector(|| "task-card-toggle-done".into())
        .rounded_full()
        .on_click(cx.listener(move |card, _: &ClickEvent, _, cx| {
            card.set_completed(&done_task, !done, cx);
        }));
        let delete_task = task_id.clone();
        let delete = widgets::action_button(theme, widgets::ActionTone::Quiet)
            .id("task-card-delete")
            .debug_selector(|| "task-card-delete".into())
            .role(gpui::Role::Button)
            .aria_label("Delete task")
            .rounded_full()
            .child(
                icon(icons::TRASH_BIN_MINIMALISTIC)
                    .size(px(14.0))
                    .text_color(theme.text_muted),
            )
            .child("Delete")
            .on_click(cx.listener(move |card, _: &ClickEvent, _, cx| {
                if let Some(open) = card.open.as_mut() {
                    open.save = None;
                }
                card.open = None;
                cx.emit(TaskCardEvent::Delete(delete_task.clone()));
                cx.notify();
            }));
        let actions = (done || more_open).then(|| {
            div()
                .flex()
                .items_center()
                .justify_end()
                .gap(px(6.0))
                .when(done, |el| el.children(open_chat("Open chat", cx)))
                .child(toggle_done)
                .child(delete)
        });

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
            .rounded(px(CARD_RADIUS))
            .bg(theme.surface_raised)
            .border_1()
            .border_color(theme.border)
            .on_key_down(cx.listener(|card, event: &KeyDownEvent, _, cx| {
                let modifiers = &event.keystroke.modifiers;
                if event.keystroke.key == "enter" && (modifiers.platform || modifiers.control) {
                    cx.stop_propagation();
                    if card
                        .open
                        .as_ref()
                        .is_some_and(|open| open.assign_pick.is_some())
                    {
                        card.assign(cx);
                    } else {
                        card.start(cx);
                    }
                }
            }))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(4.0))
                    .child(header)
                    .child(note),
            )
            .child(chips)
            .children(body)
            .children(actions)
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

#[cfg(test)]
mod tests {
    use super::*;

    fn task(id: &str, completed: bool) -> zeron_proto::Task {
        serde_json::from_value(serde_json::json!({
            "id": id,
            "title": id,
            "orderKey": id,
            "completedAt": completed.then(|| "2026-10-17T00:00:00Z"),
        }))
        .unwrap()
    }

    fn chat(id: &str, delegation: Option<&str>) -> zeron_proto::Chat {
        serde_json::from_value(serde_json::json!({
            "id": id, "title": id, "deviceId": "mac", "archived": false,
            "createdAt": "2026-10-18T00:00:00Z",
            "delegation": delegation.map(|status| serde_json::json!({
                "delegatedByChatId": "lead", "depth": 1, "anchor": "m",
                "wake": "always", "status": status, "startedAt": "2026-10-18T00:00:00Z",
            })),
        }))
        .unwrap()
    }

    fn step(
        id: &str,
        key: &str,
        chat: Option<&str>,
        outcome: Option<&str>,
    ) -> zeron_proto::TaskStep {
        serde_json::from_value(serde_json::json!({
            "id": id, "taskId": "orch", "title": id, "orderKey": key,
            "chatId": chat, "outcome": outcome,
        }))
        .unwrap()
    }

    fn orchestrated(plan_pending: bool) -> zeron_proto::Task {
        serde_json::from_value(serde_json::json!({
            "id": "orch", "title": "Build it", "orderKey": "a", "chatId": "lead",
            "mode": "orchestrated", "planPending": plan_pending,
        }))
        .unwrap()
    }

    fn init(cx: &mut gpui::TestAppContext) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        cx.update(|cx| {
            gpui_base::init(cx);
            cx.set_global(Theme::dark());
            crate::settings::init(crate::settings::UiSettings::default(), dir.path(), cx);
        });
        dir
    }

    #[test]
    fn an_orchestrated_task_rolls_its_steps_into_its_status() {
        let mut state = AppState::new();
        state.tasks = vec![orchestrated(false)];
        state.chats = vec![chat("lead", None), chat("child", Some("running"))];
        state.task_steps = vec![
            step("s1", "1", None, Some("done")),
            step("s2", "2", Some("child"), None),
            step("s3", "3", None, None),
            step("s4", "4", None, Some("skipped")),
        ];
        let now = Utc::now();
        let plan = super::super::rollup(&state, &state.tasks[0], now);
        let statuses: Vec<_> = plan.steps.iter().map(|row| row.status).collect();
        use zeron_proto::StepStatus::*;
        assert_eq!(statuses, [Done, Working, Planned, Skipped]);
        assert_eq!(plan.status, TaskStatus::Working);
        assert_eq!((plan.progress.done, plan.progress.total), (1, 3));
        assert_eq!(super::super::needs_you_count(&state), 0);

        state.chats[1] = chat("child", Some("failed"));
        let plan = super::super::rollup(&state, &state.tasks[0], now);
        assert_eq!(plan.status, TaskStatus::NeedsYou);
        assert_eq!(super::super::needs_you_count(&state), 1);
    }

    #[gpui::test]
    fn an_orchestrated_row_opens_the_center_instead_of_the_card(cx: &mut gpui::TestAppContext) {
        let _dir = init(cx);
        let state = cx.new(|_| AppState::new());
        state.update(cx, |state, _| {
            state.tasks = vec![orchestrated(false), task("plain", false)];
            state.chats = vec![chat("lead", None)];
        });
        let (page, cx) = cx.add_window_view(|_, cx| super::super::TasksPage::new(state, cx));
        let opened = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        let sink = opened.clone();
        cx.update(|_, cx| {
            cx.subscribe(&page, move |_, event: &super::super::TasksEvent, _| {
                if let super::super::TasksEvent::OpenCenter(id) = event {
                    sink.borrow_mut().push(id.clone());
                }
            })
            .detach();
        });
        cx.run_until_parked();
        assert!(cx.debug_bounds("task-ring-orch").is_some());
        let at = cx.debug_bounds("task-row-orch").unwrap().center();
        cx.simulate_click(at, gpui::Modifiers::none());
        cx.run_until_parked();
        assert_eq!(*opened.borrow(), ["orch"]);
        assert!(!page.read_with(cx, |page, cx| page.card().read(cx).is_open()));
        let at = cx.debug_bounds("task-row-plain").unwrap().center();
        cx.simulate_click(at, gpui::Modifiers::none());
        cx.run_until_parked();
        assert_eq!(*opened.borrow(), ["orch"]);
        let open = page.read_with(cx, |page, cx| {
            page.card().read(cx).task_id().map(str::to_owned)
        });
        assert_eq!(open.as_deref(), Some("plain"));
    }

    #[gpui::test]
    fn the_center_shows_the_selected_steps_thread_or_else_the_lead(cx: &mut gpui::TestAppContext) {
        let _dir = init(cx);
        let state = cx.new(|_| AppState::new());
        state.update(cx, |state, _| {
            state.tasks = vec![orchestrated(true)];
            state.chats = vec![chat("lead", None), chat("child", Some("failed"))];
            state.task_steps = vec![
                step("s1", "1", Some("child"), None),
                step("s2", "2", None, None),
            ];
        });
        let (center, cx) =
            cx.add_window_view(|_, cx| super::super::TaskCenter::new(state, "orch".into(), cx));
        let events = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        let sink = events.clone();
        cx.update(|_, cx| {
            cx.subscribe(
                &center,
                move |_, event: &super::super::TaskCenterEvent, _| {
                    use super::super::TaskCenterEvent::*;
                    match event {
                        ApprovePlan(id) => sink.borrow_mut().push(format!("approve {id}")),
                        RetryStep(id) => sink.borrow_mut().push(format!("retry {id}")),
                        _ => {}
                    }
                },
            )
            .detach();
        });
        cx.run_until_parked();
        let target = |cx: &mut gpui::VisualTestContext| {
            center.read_with(cx, |center, cx| center.live_target(cx))
        };
        assert_eq!(target(cx).as_deref(), Some("lead"));
        for (selector, expected) in [
            ("task-center-step-s1", "child"),
            ("task-center-step-s2", "lead"),
            ("task-center-lead", "lead"),
        ] {
            let at = cx.debug_bounds(selector).unwrap().center();
            cx.simulate_click(at, gpui::Modifiers::none());
            cx.run_until_parked();
            assert_eq!(target(cx).as_deref(), Some(expected), "{selector}");
        }
        assert!(cx.debug_bounds("task-center-add-step").is_some());
        assert!(cx.debug_bounds("task-center-retry-s2").is_none());
        for selector in ["task-center-retry-s1", "task-center-approve"] {
            let at = cx.debug_bounds(selector).unwrap().center();
            cx.simulate_click(at, gpui::Modifiers::none());
            cx.run_until_parked();
        }
        assert_eq!(*events.borrow(), ["retry s1", "approve orch"]);
    }

    #[gpui::test]
    fn every_card_state_renders_its_controls(cx: &mut gpui::TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        cx.update(|cx| {
            gpui_base::init(cx);
            cx.set_global(Theme::dark());
            crate::settings::init(crate::settings::UiSettings::default(), dir.path(), cx);
        });
        let state = cx.new(|_| AppState::new());
        state.update(cx, |state, _| {
            state.tasks = vec![task("plain", false), task("finished", true)];
        });
        let (card, cx) = cx.add_window_view(|_, cx| TaskCard::new(state, cx));
        card.update(cx, |card, cx| {
            card.open("plain".into(), None, CardFocus::Title, None, cx);
            card.toggle_more(cx);
        });
        cx.run_until_parked();
        for selector in [
            "task-card-hand-off",
            "task-card-start",
            "task-card-folder",
            "task-card-due",
            "task-card-project",
            "task-card-delete",
        ] {
            assert!(cx.debug_bounds(selector).is_some(), "{selector}");
        }
        card.update(cx, |card, cx| {
            card.open("finished".into(), None, CardFocus::Title, None, cx)
        });
        cx.run_until_parked();
        assert!(cx.debug_bounds("task-card-hand-off").is_none());
        assert!(cx.debug_bounds("task-card-toggle-done").is_some());
    }

    #[gpui::test]
    fn typing_a_thread_name_picks_it_and_assign_sends_the_task(cx: &mut gpui::TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        cx.update(|cx| {
            gpui_base::init(cx);
            cx.set_global(Theme::dark());
            crate::settings::init(crate::settings::UiSettings::default(), dir.path(), cx);
        });
        let chat = |id: &str, title: &str, device: &str| -> zeron_proto::Chat {
            serde_json::from_value(serde_json::json!({
                "id": id, "title": title, "deviceId": device, "archived": false,
                "createdAt": "2026-10-18T00:00:00Z",
            }))
            .unwrap()
        };
        let state = cx.new(|_| AppState::new());
        state.update(cx, |state, _| {
            state.local_device_id = Some("mac".into());
            state.tasks = vec![task("plain", false)];
            state.chats = vec![
                chat("fix", "Fix login bug", "mac"),
                chat("far", "Fix login on the PC", "pc"),
                chat("docs", "Write docs", "mac"),
            ];
        });
        let (card, cx) = cx.add_window_view(|_, cx| TaskCard::new(state, cx));
        let events = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        let sink = events.clone();
        cx.update(|_, cx| {
            cx.subscribe(&card, move |_, event: &TaskCardEvent, _| {
                if let TaskCardEvent::Assign { chat_id, send, .. } = event {
                    sink.borrow_mut().push((chat_id.clone(), *send));
                }
            })
            .detach();
        });
        card.update(cx, |card, cx| {
            card.open("plain".into(), None, CardFocus::Title, None, cx);
            let input = card.open.as_ref().unwrap().assign.clone();
            input.update(cx, |input, cx| input.set_text("@fix log", cx));
        });
        cx.run_until_parked();
        assert!(cx.debug_bounds("task-card-assign-match-0").is_some());
        assert!(cx.debug_bounds("task-card-assign-match-1").is_none());
        assert!(cx.debug_bounds("task-card-assign-button").is_none());
        let at = cx
            .debug_bounds("task-card-assign-match-0")
            .unwrap()
            .center();
        cx.simulate_click(at, gpui::Modifiers::none());
        cx.run_until_parked();
        assert!(cx.debug_bounds("task-card-assign-matches").is_none());
        assert!(cx.debug_bounds("task-card-start").is_none());
        let at = cx.debug_bounds("task-card-assign-button").unwrap().center();
        cx.simulate_click(at, gpui::Modifiers::none());
        cx.run_until_parked();
        assert_eq!(*events.borrow(), vec![("fix".to_string(), true)]);
    }
}
