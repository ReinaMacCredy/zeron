use std::collections::HashMap;
use std::time::Instant;

use chrono::Utc;
use gpui::{
    AnyElement, App, ClickEvent, Context, Entity, EntityInputHandler, EventEmitter, FocusHandle,
    Focusable, IntoElement, KeyDownEvent, PathPromptOptions, Render, SharedString, Subscription,
    Task, Window, div, prelude::*, px,
};
use zeron_proto::{ChatConfig, ChatIndicator, TaskPatch, TaskStatus};

use super::images::{self, ImageGallery};
use super::logic::{self, DuePick};
use super::{Row, SAVE_DEBOUNCE, START_PENDING, linked_chat, patched, row_for};
use crate::composer::task_completions::{TaskCompletions, TaskScope};
use crate::composer::{ComposerInput, ComposerInputEvent, Submit};
use crate::icons::{self, icon};
use crate::motion::{self, MotionSpec, Tween};
use crate::pickers::Pickers;
use crate::popover;
use crate::settings::widgets;
use crate::state::AppState;
use crate::theme::Theme;
use crate::typography::ui_rems;

const PILL_HEIGHT: f32 = 28.0;
const CARD_RADIUS: f32 = 24.0;
const PANEL_IN: MotionSpec = MotionSpec::new(300, motion::EASE_DRAWER);
const PANEL_OUT: MotionSpec = MotionSpec::new(220, motion::EASE_IN_SHARP);
const SLOT_SHIFT: MotionSpec = MotionSpec::new(320, motion::EASE_DRAWER);
const CONTENT_IN: MotionSpec = MotionSpec::new(200, motion::LINEAR).with_delay(80);
const SWAP_OUT: MotionSpec = MotionSpec::new(90, motion::LINEAR);
const SWAP_IN: MotionSpec = MotionSpec::new(180, motion::EASE_DRAWER).with_delay(90);
const PANEL_SHIFT_IN: f32 = 28.0;
const PANEL_SHIFT_OUT: f32 = 20.0;
const SWAP_OUT_RISE: f32 = 4.0;
const SWAP_IN_DROP: f32 = 6.0;

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
        space_id: Option<Option<String>>,
        config: Option<ChatConfig>,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HandOff {
    NewChat,
    Thread,
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
    title_menus: Entity<TaskCompletions>,
    notes_menus: Entity<TaskCompletions>,
    saved_title: String,
    saved_notes: String,
    stored: (String, String),
    images: Vec<String>,
    uploads: Vec<Task<()>>,
    refused: bool,
    seen: bool,
    save: Option<Task<()>>,
    run_in: Option<Option<String>>,
    folder: Option<String>,
    tab: HandOff,
    assign: Entity<ComposerInput>,
    assign_pick: Option<(String, String)>,
    assign_hidden: bool,
    _subscriptions: [Subscription; 5],
}

struct Fields {
    task_id: String,
    title: Entity<ComposerInput>,
    notes: Entity<ComposerInput>,
}

struct Leaving {
    open: Open,
    swap_out: Option<Tween>,
}

#[derive(Clone, Copy)]
struct Content {
    opacity: f32,
    rise: f32,
    inert: bool,
}

pub struct CardFrame {
    pub slot: f32,
    pub panel: f32,
    pub shift: f32,
}

pub struct TaskCard {
    state: Entity<AppState>,
    open: Option<Open>,
    leaving: Option<Leaving>,
    presence: Tween,
    slot: Tween,
    content: Tween,
    content_drop: f32,
    project_select: widgets::SelectState,
    due_select: widgets::SelectState,
    run_select: widgets::SelectState,
    starting: HashMap<String, Instant>,
    agent: Entity<Pickers>,
    focus: FocusHandle,
    start_focus: FocusHandle,
    folder_prompt: Option<Task<()>>,
    picking: Option<Task<()>>,
    gallery: Entity<ImageGallery>,
    menu: popover::Popup<()>,
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
        let gallery = cx.new(|cx| ImageGallery::new(state.clone(), cx));
        let now = motion::now();
        Self {
            gallery,
            picking: None,
            agent,
            state,
            open: None,
            leaving: None,
            presence: Tween::at(0.0, now),
            slot: Tween::at(0.0, now),
            content: Tween::at(1.0, now),
            content_drop: 0.0,
            project_select: widgets::SelectState::default(),
            due_select: widgets::SelectState::default(),
            run_select: widgets::SelectState::default(),
            starting: HashMap::new(),
            focus: cx.focus_handle(),
            start_focus: cx.focus_handle(),
            folder_prompt: None,
            menu: popover::Popup::default(),
            _state_changes: state_changes,
        }
    }

    pub fn task_id(&self) -> Option<&str> {
        self.open.as_ref().map(|open| open.task_id.as_str())
    }

    pub(crate) fn chat_config(&self, cx: &gpui::App) -> Option<ChatConfig> {
        self.agent.read(cx).resolved(cx).chat_config()
    }

    pub fn is_open(&self) -> bool {
        self.open.is_some()
    }

    pub fn frame(&self, now: Instant) -> Option<CardFrame> {
        let shown =
            self.open.is_some() || !self.presence.settled_at(now) || !self.slot.settled_at(now);
        shown.then(|| {
            let panel = self.presence.value_at(now);
            let shift = if self.presence.target() > 0.0 {
                PANEL_SHIFT_IN
            } else {
                PANEL_SHIFT_OUT
            };
            CardFrame {
                slot: self.slot.value_at(now),
                panel,
                shift: shift * (1.0 - panel),
            }
        })
    }

    fn animating(&self, now: Instant) -> bool {
        self.leaving.is_some()
            || !self.presence.settled_at(now)
            || !self.slot.settled_at(now)
            || !self.content.settled_at(now)
    }

    fn dismiss(&mut self, window: Option<&mut Window>, cx: &mut Context<Self>) {
        if let Some(window) = window
            && self.focus.contains_focused(window, cx)
        {
            window.blur();
        }
        let now = motion::now();
        let reduced = motion::reduced_motion(cx);
        if let Some(mut open) = self.open.take() {
            open.save = None;
            self.leaving = (!reduced).then_some(Leaving {
                open,
                swap_out: None,
            });
        }
        self.presence.retarget(0.0, PANEL_OUT, reduced, now);
        self.slot.retarget(0.0, SLOT_SHIFT, reduced, now);
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
            self.focus_field(focus, window, cx);
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
        let saved_images = row
            .as_ref()
            .map(|task| task.images.clone())
            .unwrap_or_default();
        self.menu = popover::Popup::default();
        let state = self.state.clone();
        let title_input = cx.new(|cx| {
            let mut input = ComposerInput::new("Task title", cx)
                .with_single_line()
                .with_max_chars(zeron_doc::TASK_TITLE_MAX_CHARS)
                .with_mentions(state.clone());
            input.set_text(saved_title.clone(), cx);
            input
        });
        let notes_input = cx.new(|cx| {
            let mut input = ComposerInput::new("Add a note", cx)
                .with_max_chars(zeron_doc::TASK_NOTES_MAX_CHARS)
                .with_mentions(state.clone());
            input.set_text(saved_notes.clone(), cx);
            input
        });
        let title_menus = self.completions(&title_input, cx);
        let notes_menus = self.completions(&notes_input, cx);
        let edits = |card: &mut Self,
                     _: Entity<ComposerInput>,
                     event: &ComposerInputEvent,
                     cx: &mut Context<Self>| match event {
            ComposerInputEvent::Edited => card.schedule_save(cx),
            ComposerInputEvent::Submitted => card.flush(cx),
            event => {
                if let Some(staged) = images::pasted(event, cx) {
                    card.add_images(staged, cx);
                }
            }
        };
        let thread_input = |placeholder: &'static str, cx: &mut Context<Self>| {
            cx.new(|cx| {
                ComposerInput::new(placeholder, cx)
                    .with_single_line()
                    .with_max_chars(200)
            })
        };
        let assign_input = thread_input("@ a chat to give this task to", cx);
        let subscriptions = [
            cx.observe(&title_menus, |_, _, cx| cx.notify()),
            cx.observe(&notes_menus, |_, _, cx| cx.notify()),
            cx.subscribe(&title_input, edits),
            cx.subscribe(&notes_input, edits),
            cx.subscribe(&assign_input, |card: &mut Self, _, event, cx| {
                match event {
                    ComposerInputEvent::Edited => {
                        if let Some(open) = card.open.as_mut() {
                            open.assign_hidden = false;
                        }
                    }
                    ComposerInputEvent::Submitted => {
                        if let Some(first) = card.assign_matches(cx).into_iter().next() {
                            card.pick_thread(first.chat_id, first.title, cx);
                        }
                    }
                    _ => {}
                }
                cx.notify();
            }),
        ];
        let now = motion::now();
        let reduced = motion::reduced_motion(cx);
        let previous = self.open.replace(Open {
            task_id,
            title: title_input,
            notes: notes_input,
            stored: (saved_title.clone(), saved_notes.clone()),
            saved_title,
            saved_notes,
            images: saved_images,
            uploads: Vec::new(),
            refused: false,
            seen: row.is_some(),
            save: None,
            run_in: None,
            folder: None,
            tab: HandOff::NewChat,
            title_menus,
            notes_menus,
            assign: assign_input,
            assign_pick: None,
            assign_hidden: false,
            _subscriptions: subscriptions,
        });
        self.focus_field(focus, window, cx);
        match previous {
            Some(mut open) if !reduced => {
                open.save = None;
                let mut fade = Tween::at(1.0, now);
                fade.retarget(0.0, SWAP_OUT, false, now);
                self.leaving = Some(Leaving {
                    open,
                    swap_out: Some(fade),
                });
                self.content = Tween::at(0.0, now);
                self.content.retarget(1.0, SWAP_IN, false, now);
                self.content_drop = SWAP_IN_DROP;
            }
            Some(_) => {}
            None => {
                self.leaving = None;
                if self.presence.value_at(now) == 0.0 {
                    self.content = Tween::at(0.0, now);
                    self.content.retarget(1.0, CONTENT_IN, reduced, now);
                    self.content_drop = 0.0;
                }
                self.presence.retarget(1.0, PANEL_IN, reduced, now);
                self.slot.retarget(1.0, SLOT_SHIFT, reduced, now);
            }
        }
        cx.notify();
    }

    fn focus_field(&self, focus: CardFocus, window: Option<&mut Window>, cx: &mut Context<Self>) {
        let (Some(window), Some(open)) = (window, self.open.as_ref()) else {
            return;
        };
        let target = match focus {
            CardFocus::Title => open.title.focus_handle(cx),
            CardFocus::Notes => open.notes.focus_handle(cx),
            CardFocus::Agent => self.start_focus.clone(),
        };
        window.focus(&target, cx);
    }

    pub fn close(&mut self, window: Option<&mut Window>, cx: &mut Context<Self>) {
        self.flush(cx);
        self.dismiss(window, cx);
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
        let state = self.state.read(cx);
        let row = state
            .tasks
            .iter()
            .find(|task| task.id == open.task_id && !task.deleted);
        let arrived = match row {
            Some(row) if !open.seen => {
                open.seen = true;
                let added: Vec<String> = std::mem::take(&mut open.images)
                    .into_iter()
                    .filter(|path| !row.images.contains(path))
                    .collect();
                open.images = row.images.clone();
                open.images.extend(added);
                open.images.truncate(zeron_doc::TASK_IMAGES_MAX);
                open.images != row.images
            }
            Some(_) => false,
            None => {
                if open.seen {
                    self.dismiss(None, cx);
                }
                return;
            }
        };
        if arrived {
            self.save_images(cx);
        }
        self.adopt_stored_text(cx);
    }

    fn adopt_stored_text(&mut self, cx: &mut Context<Self>) {
        let Some(open) = self.open.as_mut() else {
            return;
        };
        let state = self.state.read(cx);
        let Some(row) = state.tasks.iter().find(|task| task.id == open.task_id) else {
            return;
        };
        let (stored_title, stored_notes) = &open.stored;
        let title = (row.title != *stored_title
            && row.title != open.saved_title
            && open.title.read(cx).text().trim() == open.saved_title)
            .then(|| row.title.clone());
        let notes = (row.notes != *stored_notes
            && row.notes != open.saved_notes
            && open.notes.read(cx).text() == open.saved_notes)
            .then(|| row.notes.clone());
        open.stored = (row.title.clone(), row.notes.clone());
        if let Some(title) = title {
            open.saved_title = title.clone();
            open.title.update(cx, |input, cx| input.set_text(title, cx));
        }
        if let Some(notes) = notes {
            open.saved_notes = notes.clone();
            open.notes.update(cx, |input, cx| input.set_text(notes, cx));
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
            open.saved_notes = row.as_ref().map(|t| t.notes.clone()).unwrap_or_default();
        }
        if patch.images.as_ref() == Some(&open.images) {
            open.images = row.map(|t| t.images).unwrap_or_default();
        }
    }

    fn add_images(
        &mut self,
        staged: Vec<crate::attachments::StagedAttachment>,
        cx: &mut Context<Self>,
    ) {
        let Some(open) = self.open.as_mut() else {
            return;
        };
        if staged.is_empty() {
            return;
        }
        let task_id = open.task_id.clone();
        let upload = images::upload(&self.state, staged, cx);
        open.uploads.push(cx.spawn(async move |card, cx| {
            let paths = upload.await;
            card.update(cx, |card, cx| {
                let Some(open) = card.open.as_mut().filter(|open| open.task_id == task_id) else {
                    return;
                };
                if paths.is_empty() {
                    return;
                }
                images::append(&mut open.images, paths, cx);
                if open.seen {
                    card.save_images(cx);
                }
            })
            .ok();
        }));
    }

    fn pick_images(&mut self, cx: &mut Context<Self>) {
        let picked = images::pick(cx);
        self.picking = Some(cx.spawn(async move |card, cx| {
            let paths = picked.await;
            card.update(cx, |card, cx| {
                let staged = images::staged_files(&paths, cx);
                card.add_images(staged, cx);
            })
            .ok();
        }));
    }

    fn remove_image(&mut self, index: usize, cx: &mut Context<Self>) {
        if let Some(open) = self.open.as_mut()
            && index < open.images.len()
        {
            open.images.remove(index);
            self.save_images(cx);
        }
    }

    fn save_images(&mut self, cx: &mut Context<Self>) {
        let Some(open) = self.open.as_ref() else {
            return;
        };
        let (task_id, images) = (open.task_id.clone(), open.images.clone());
        self.update(
            &task_id,
            TaskPatch {
                images: Some(images),
                ..Default::default()
            },
            cx,
        );
        cx.notify();
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
        let Some(task_id) = self.open.as_ref().map(|open| open.task_id.clone()) else {
            return;
        };
        let state = self.state.read(cx);
        let startable = state.tasks.iter().any(|task| {
            task.id == task_id
                && linked_chat(&state.chats, task).is_none()
                && task.completed_at.is_none()
        });
        if !startable {
            return;
        }
        if self.is_starting(&task_id) || self.runs_elsewhere(cx).is_some() {
            return;
        }
        if let Some(open) = self.open.as_mut() {
            open.refused = false;
        }
        let taken = self.take_patch(cx);
        if self.open.as_ref().is_some_and(|open| open.refused) {
            if let Some((task_id, patch)) = taken {
                cx.emit(TaskCardEvent::Update { task_id, patch });
            }
            return;
        }
        self.starting.insert(task_id.clone(), Instant::now());
        cx.spawn(async move |card, cx| {
            cx.background_executor().timer(START_PENDING).await;
            card.update(cx, |_, cx| cx.notify()).ok();
        })
        .detach();
        let patch = taken.map(|(_, patch)| patch);
        let space_id = self.open.as_ref().and_then(|open| open.run_in.clone());
        let config = self.agent.read(cx).resolved(cx).chat_config();
        cx.emit(TaskCardEvent::Start {
            task_id,
            patch,
            space_id,
            config,
        });
        cx.notify();
    }

    fn completions(
        &self,
        input: &Entity<ComposerInput>,
        cx: &mut Context<Self>,
    ) -> Entity<TaskCompletions> {
        let card = cx.weak_entity();
        let (state, input) = (self.state.clone(), input.clone());
        cx.new(|cx| {
            TaskCompletions::new(
                state,
                input,
                move |cx| {
                    card.upgrade().map_or(
                        TaskScope {
                            space_id: None,
                            agent: None,
                        },
                        |card| card.read(cx).completion_scope(cx),
                    )
                },
                cx,
            )
        })
    }

    fn completion_scope(&self, cx: &App) -> TaskScope {
        let saved = self.open.as_ref().and_then(|open| {
            let tasks = &self.state.read(cx).tasks;
            let task = tasks.iter().find(|task| task.id == open.task_id)?;
            task.space_id.clone()
        });
        TaskScope {
            space_id: self
                .open
                .as_ref()
                .and_then(|open| open.run_in.clone())
                .unwrap_or(saved),
            agent: Some(self.agent.clone()),
        }
    }

    fn assign_matches(&self, cx: &Context<Self>) -> Vec<crate::composer::ThreadMatch> {
        match self.open.as_ref() {
            Some(open) if open.assign_pick.is_none() => {
                self.thread_offer(&open.assign, &open.task_id, &[], cx)
            }
            _ => Vec::new(),
        }
    }

    fn thread_offer(
        &self,
        input: &Entity<ComposerInput>,
        task_id: &str,
        taken: &[String],
        cx: &Context<Self>,
    ) -> Vec<crate::composer::ThreadMatch> {
        let text = input.read(cx).text().trim();
        if text.is_empty() {
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
            .find(|task| task.id == task_id)
            .and_then(|task| task.space_id.as_deref());
        crate::composer::thread_matches(
            state
                .visible_chats()
                .filter(|chat| !taken.contains(&chat.id)),
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
        open.assign_pick = Some((chat_id, title));
        open.assign.update(cx, |input, cx| input.set_text("", cx));
        cx.notify();
    }

    fn unpick_thread(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(open) = self.open.as_mut() else {
            return;
        };
        open.assign_pick = None;
        window.focus(&open.assign.focus_handle(cx), cx);
        cx.notify();
    }

    fn assign(&mut self, send: bool, cx: &mut Context<Self>) {
        let Some((task_id, (chat_id, _))) = self
            .open
            .as_ref()
            .and_then(|open| Some((open.task_id.clone(), open.assign_pick.clone()?)))
        else {
            return;
        };
        let patch = self.take_patch(cx).map(|(_, patch)| patch);
        if let Some(open) = self.open.as_mut() {
            open.assign_pick = None;
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
        let Some(task_id) = self.task_id().map(str::to_owned) else {
            return;
        };
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
                    card.run_in_folder(&task_id, path.to_string_lossy().into_owned(), cx);
                }
            })
            .ok();
        }));
    }

    fn run_in_folder(&mut self, task_id: &str, path: String, cx: &mut Context<Self>) {
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
        let Some(open) = self.open.as_mut().filter(|open| open.task_id == task_id) else {
            return;
        };
        let space_id = existing.unwrap_or_else(|| {
            let space_id = uuid::Uuid::new_v4().to_string();
            cx.emit(TaskCardEvent::CreateSpace {
                space_id: space_id.clone(),
                device_id,
                path: path.clone(),
            });
            space_id
        });
        open.run_in = Some(Some(space_id));
        open.folder = Some(path);
        cx.notify();
    }

    fn set_tab(&mut self, tab: HandOff, cx: &mut Context<Self>) {
        if let Some(open) = self.open.as_mut() {
            open.tab = tab;
        }
        cx.notify();
    }

    fn toggle_menu(&mut self, cx: &mut Context<Self>) {
        if self.menu.take_press_was_open() || self.menu.is_open() {
            self.close_menu(cx);
        } else {
            self.menu.open(());
            cx.notify();
        }
    }

    pub fn dismiss_popups(&mut self, cx: &mut Context<Self>) -> bool {
        if self.menu.is_open() {
            self.close_menu(cx);
            return true;
        }
        let Some(open) = self.open.as_mut() else {
            return false;
        };
        let shown = |hidden: bool, input: &Entity<ComposerInput>| {
            !hidden && !input.read(cx).text().trim().is_empty()
        };
        let assign = open.assign_pick.is_none() && shown(open.assign_hidden, &open.assign);
        if open.tab != HandOff::Thread || !assign {
            return false;
        }
        open.assign_hidden = true;
        cx.notify();
        true
    }

    fn close_menu(&mut self, cx: &mut Context<Self>) {
        if self.menu.begin_close() {
            popover::reap_popup(cx, |card: &mut Self| &mut card.menu);
            cx.notify();
        }
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

        let add_image = Self::pill(theme, "task-card-add-image")
            .aria_label("Add image")
            .tooltip(widgets::text_tooltip("Add image"))
            .child(
                icon(icons::GALLERY)
                    .size(px(14.0))
                    .text_color(theme.text_muted),
            )
            .on_click(cx.listener(|card, _: &ClickEvent, _, cx| card.pick_images(cx)));
        div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap(px(6.0))
            .child(due)
            .child(project)
            .child(add_image)
            .into_any_element()
    }

    fn render_run_line(
        &self,
        row: Option<&Row>,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let open = self.open.as_ref();
        let state = self.state.read(cx);
        let local = state.local_device_id.clone();
        let spaces: Vec<(String, String)> = state
            .spaces
            .iter()
            .filter(|space| local.as_deref() == Some(space.device_id.as_str()))
            .map(|space| (space.id.clone(), space.display_name().to_string()))
            .collect();
        let run_in = Self::run_in(open, row);
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
        let elsewhere = self.runs_elsewhere(cx);
        let selected = match (&pending_folder, elsewhere) {
            (Some(name), _) => {
                options.push(widgets::SelectOption::new(name.clone()).leading(folder));
                options.len() - 1
            }
            (None, Some((space, device))) => {
                options.push(
                    widgets::SelectOption::new(format!("{space} on {device}")).leading(folder),
                );
                options.len() - 1
            }
            (None, None) => run_in
                .as_deref()
                .and_then(|id| spaces.iter().position(|(sid, _)| sid == id))
                .map_or(0, |ix| ix + 1),
        };
        let space_ids: Vec<String> = spaces.iter().map(|(id, _)| id.clone()).collect();
        let hover = theme.wash(0.10);
        let text = theme.text;
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
        .max_w(px(170.0))
        .bg(gpui::transparent_black())
        .text_color(theme.text_muted)
        .hover(move |s| s.bg(hover).text_color(text));
        let folder_button = div()
            .id("task-card-folder")
            .debug_selector(|| "task-card-folder".into())
            .role(gpui::Role::Button)
            .aria_label("Folder…")
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
        div()
            .flex()
            .items_center()
            .gap(px(2.0))
            .child(
                div()
                    .flex_none()
                    .pl(px(2.0))
                    .text_size(ui_rems(12.5))
                    .text_color(theme.text_muted)
                    .child("in"),
            )
            .child(run_select)
            .child(folder_button)
            .into_any_element()
    }

    fn runs_elsewhere(&self, cx: &App) -> Option<(String, String)> {
        let open = self.open.as_ref()?;
        let state = self.state.read(cx);
        let saved = state
            .tasks
            .iter()
            .find(|task| task.id == open.task_id)
            .and_then(|task| task.space_id.clone());
        let space_id = open.run_in.clone().unwrap_or(saved)?;
        let space = state.spaces.iter().find(|space| space.id == space_id)?;
        let local = state.local_device_id.as_deref()?;
        (space.device_id != local).then(|| {
            let device = state
                .device_name(&space.device_id)
                .unwrap_or("another device");
            (space.display_name().to_string(), device.to_string())
        })
    }

    fn run_in(open: Option<&Open>, row: Option<&Row>) -> Option<String> {
        open.and_then(|open| open.run_in.clone())
            .unwrap_or_else(|| row.and_then(|row| row.task.space_id.clone()))
    }

    fn render_agent(&self) -> gpui::Div {
        div().ml(px(-6.0)).min_w_0().child(self.agent.clone())
    }

    fn solid_button(
        &self,
        id: &'static str,
        label: SharedString,
        enabled: bool,
        theme: &Theme,
    ) -> gpui::Stateful<gpui::Div> {
        widgets::action_button(theme, widgets::ActionTone::Solid)
            .id(id)
            .debug_selector(move || id.into())
            .role(gpui::Role::Button)
            .aria_label(label.clone())
            .aria_disabled(!enabled)
            .track_focus(&self.start_focus)
            .tab_index(0)
            .rounded_full()
            .min_h(px(30.0))
            .px(px(14.0))
            .when(!enabled, |el| el.opacity(0.4))
            .child(label)
            .child(div().text_size(ui_rems(11.0)).opacity(0.5).child("⌘↩"))
    }

    fn render_hand_off(
        &self,
        task_id: &str,
        row: Option<&Row>,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Some(open) = self.open.as_ref() else {
            return div().into_any_element();
        };
        let tab = open.tab;
        let weak = cx.entity().downgrade();
        let tabs = widgets::segmented_fill(
            theme,
            "task-card-hand-off-mode",
            &[(HandOff::NewChat, "New chat"), (HandOff::Thread, "Thread")],
            tab,
            move |tab, _, cx| {
                weak.update(cx, |card, cx| card.set_tab(tab, cx)).ok();
            },
        )
        .debug_selector(|| "task-card-hand-off-mode".into());
        let starting = self.is_starting(task_id);
        let elsewhere = self
            .runs_elsewhere(cx)
            .map(|(_, device)| format!("Runs on {device}"));
        let ready = row.is_some() && !starting && elsewhere.is_none();
        let (pane, footer) = match tab {
            HandOff::NewChat => {
                let start = self
                    .solid_button(
                        "task-card-start",
                        match &elsewhere {
                            Some(label) => label.clone().into(),
                            None if starting => "Starting…".into(),
                            None => "Start".into(),
                        },
                        ready,
                        theme,
                    )
                    .on_click(cx.listener(|card, _: &ClickEvent, _, cx| card.start(cx)));
                let pane = div().flex().flex_col().gap(px(8.0)).child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(2.0))
                        .child(self.render_agent())
                        .child(self.render_run_line(row, theme, cx)),
                );
                (pane.into_any_element(), vec![start.into_any_element()])
            }
            HandOff::Thread => self.render_thread_tab(theme, cx),
        };
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
                    .child("Hand it to"),
            )
            .child(tabs)
            .child(pane)
            .child(
                div()
                    .pt(px(6.0))
                    .flex()
                    .items_center()
                    .justify_end()
                    .gap(px(6.0))
                    .children(footer),
            )
            .into_any_element()
    }

    fn render_thread_tab(
        &self,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> (AnyElement, Vec<AnyElement>) {
        let Some(open) = self.open.as_ref() else {
            return (div().into_any_element(), Vec::new());
        };
        let picked = open.assign_pick.clone();
        let target = match &picked {
            Some((chat_id, title)) => {
                let state = self.state.read(cx);
                let chat = state.chats.iter().find(|chat| &chat.id == chat_id);
                let (mark, tint) = crate::pickers::agent_brand_icon(
                    chat.and_then(|chat| chat.config.as_ref().map(|config| config.harness)),
                    chat.and_then(|chat| chat.config.as_ref()?.model.as_deref()),
                );
                let indicator = chat.map(|chat| state.display_status_for(chat, Utc::now()));
                let tone = indicator
                    .map(|indicator| crate::shell::chat_status_tone(indicator, theme).2)
                    .unwrap_or(theme.text_muted);
                div()
                    .id("task-card-assign-picked")
                    .debug_selector(|| "task-card-assign-picked".into())
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .px(px(10.0))
                    .py(px(8.0))
                    .rounded(px(12.0))
                    .bg(theme.wash(0.06))
                    .text_size(ui_rems(12.5))
                    .child(
                        icon(mark)
                            .size(px(14.0))
                            .flex_none()
                            .text_color(tint.unwrap_or(theme.text_muted)),
                    )
                    .child(div().flex_1().min_w_0().truncate().child(title.clone()))
                    .children(indicator.map(|indicator| {
                        div()
                            .flex_none()
                            .text_color(tone)
                            .child(zeron_doc::tree::linked_label(indicator))
                    }))
                    .child(
                        div()
                            .id("task-card-assign-clear")
                            .debug_selector(|| "task-card-assign-clear".into())
                            .role(gpui::Role::Button)
                            .aria_label("Clear")
                            .flex_none()
                            .size(px(20.0))
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded_full()
                            .cursor_pointer()
                            .hover(|s| s.bg(theme.glass_hover()))
                            .child(
                                icon(icons::LUCIDE_X)
                                    .size(px(12.0))
                                    .text_color(theme.text_muted),
                            )
                            .on_click(cx.listener(|card, _: &ClickEvent, window, cx| {
                                card.unpick_thread(window, cx)
                            })),
                    )
                    .into_any_element()
            }
            None => {
                let matches = (!open.assign_hidden).then(|| self.assign_matches(cx));
                let popover = matches
                    .filter(|_| !open.assign.read(cx).text().trim().is_empty())
                    .map(|matches| {
                        let rows = self.thread_rows(
                            matches,
                            "task-card-assign-match",
                            |card: &mut Self, chat_id, title, cx| {
                                card.pick_thread(chat_id, title, cx)
                            },
                            theme,
                            cx,
                        );
                        self.match_popover(
                            "task-card-assign-matches",
                            rows,
                            "Enter picks the first match",
                            306.0,
                            |card: &mut Self| {
                                if let Some(open) = card.open.as_mut() {
                                    open.assign_hidden = true;
                                }
                            },
                            theme,
                            cx,
                        )
                    });
                div()
                    .id("task-card-assign")
                    .debug_selector(|| "task-card-assign".into())
                    .relative()
                    .px(px(10.0))
                    .py(px(6.0))
                    .rounded(px(10.0))
                    .bg(theme.wash(0.06))
                    .child(open.assign.clone())
                    .children(popover)
                    .into_any_element()
            }
        };
        let strong = gpui::HighlightStyle {
            font_weight: Some(gpui::FontWeight::MEDIUM),
            color: Some(theme.text_muted),
            ..Default::default()
        };
        let help_text = "Send task posts the title and notes to it, steering it if it is working. \
                         Link only sends nothing; the task just follows its status.";
        let link_at = help_text.find("Link only").unwrap_or_default();
        let help = div()
            .text_size(ui_rems(11.5))
            .text_color(theme.text_muted.opacity(0.75))
            .child(gpui::StyledText::new(help_text).with_highlights(vec![
                (0.."Send task".len(), strong),
                (link_at..link_at + "Link only".len(), strong),
            ]));
        let enabled = picked.is_some();
        let link = widgets::text_action(theme, widgets::ActionTone::Quiet, "Link only")
            .id("task-card-link-only")
            .debug_selector(|| "task-card-link-only".into())
            .role(gpui::Role::Button)
            .aria_label("Link only")
            .aria_disabled(!enabled)
            .tab_index(0)
            .rounded_full()
            .min_h(px(30.0))
            .px(px(14.0))
            .border_1()
            .border_color(theme.border)
            .when(!enabled, |el| el.opacity(0.4))
            .on_click(cx.listener(|card, _: &ClickEvent, _, cx| card.assign(false, cx)));
        let send = self
            .solid_button(
                "task-card-assign-button",
                "Send task".into(),
                enabled,
                theme,
            )
            .on_click(cx.listener(|card, _: &ClickEvent, _, cx| card.assign(true, cx)));
        (
            div()
                .flex()
                .flex_col()
                .gap(px(8.0))
                .child(target)
                .child(help)
                .into_any_element(),
            vec![link.into_any_element(), send.into_any_element()],
        )
    }

    fn thread_rows(
        &self,
        matches: Vec<crate::composer::ThreadMatch>,
        prefix: &'static str,
        pick: impl Fn(&mut Self, String, String, &mut Context<Self>) + Clone + 'static,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        let state = self.state.read(cx);
        let now = Utc::now();
        matches
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
                let pick = pick.clone();
                div()
                    .id((prefix, ix))
                    .debug_selector(move || format!("{prefix}-{ix}"))
                    .role(gpui::Role::Button)
                    .aria_label(found.title.clone())
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .px(px(8.0))
                    .py(px(6.0))
                    .rounded(px(8.0))
                    .cursor_pointer()
                    .text_size(ui_rems(12.5))
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
                                .max_w(px(80.0))
                                .truncate()
                                .text_color(theme.text_muted)
                                .child(found.project),
                        )
                    })
                    .children(label.map(|label| div().flex_none().text_color(tone).child(label)))
                    .on_click(cx.listener(move |card, _: &ClickEvent, _, cx| {
                        pick(card, chat_id.clone(), title.clone(), cx);
                    }))
                    .into_any_element()
            })
            .collect()
    }

    #[allow(clippy::too_many_arguments)]
    fn match_popover(
        &self,
        id: &'static str,
        rows: Vec<AnyElement>,
        hint: &'static str,
        width: f32,
        hide: impl Fn(&mut Self) + 'static,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let theme = theme.for_popup();
        let empty = rows.is_empty();
        let card = popover::popover_card(&theme)
            .id(id)
            .debug_selector(move || id.into())
            .w(px(width))
            .flex()
            .flex_col()
            .on_mouse_down_out(cx.listener(move |card, _, _, cx| {
                hide(card);
                cx.notify();
            }))
            .children(rows)
            .child(
                div()
                    .px(px(8.0))
                    .pt(px(4.0))
                    .pb(px(2.0))
                    .text_size(ui_rems(11.5))
                    .text_color(theme.text_muted)
                    .child(if empty { "No chat matches" } else { hint }),
            );
        popover::anchored_menu_below_gap(id, card.into_any_element(), None, 4.0)
    }

    fn render_menu(
        &self,
        task_id: &str,
        done: bool,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        self.menu.get()?;
        let theme = theme.for_popup();
        let done_task = task_id.to_owned();
        let delete_task = task_id.to_owned();
        let toggle_label = if done {
            "Mark as not done"
        } else {
            "Mark as done"
        };
        let card = popover::popover_card(&theme)
            .w(px(170.0))
            .flex()
            .flex_col()
            .on_mouse_down_out(cx.listener(|card, _, _, cx| card.close_menu(cx)))
            .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(
                popover::menu_row(&theme, false, "task-card-toggle-done")
                    .id("task-card-toggle-done")
                    .debug_selector(|| "task-card-toggle-done".into())
                    .role(gpui::Role::MenuItem)
                    .aria_label(toggle_label)
                    .child(
                        icon(icons::LUCIDE_CIRCLE_CHECK)
                            .size(px(15.0))
                            .text_color(theme.text_muted),
                    )
                    .child(toggle_label)
                    .on_click(cx.listener(move |card, _: &ClickEvent, _, cx| {
                        cx.stop_propagation();
                        card.close_menu(cx);
                        card.set_completed(&done_task, !done, cx);
                    })),
            )
            .child(
                popover::menu_row(&theme, false, "task-card-delete")
                    .id("task-card-delete")
                    .debug_selector(|| "task-card-delete".into())
                    .role(gpui::Role::MenuItem)
                    .aria_label("Delete task")
                    .text_color(theme.danger)
                    .child(
                        icon(icons::TRASH_BIN_MINIMALISTIC)
                            .size(px(15.0))
                            .text_color(theme.danger),
                    )
                    .child("Delete")
                    .on_click(cx.listener(move |card, _: &ClickEvent, window, cx| {
                        cx.stop_propagation();
                        card.close_menu(cx);
                        card.dismiss(Some(window), cx);
                        cx.emit(TaskCardEvent::Delete(delete_task.clone()));
                        cx.notify();
                    })),
            );
        Some(popover::anchored_menu_below_end(
            "task-card-menu-popover",
            card.into_any_element(),
            self.menu.closing_since(),
        ))
    }

    fn render_card(
        &self,
        fields: Fields,
        row: Option<&Row>,
        content: Content,
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
        let open = self.open.as_ref().filter(|open| open.task_id == task_id);
        let title_popup = open.and_then(|open| {
            open.title_menus
                .read(cx)
                .popup_below("task-card-title-completions", cx)
        });
        let notes_popup = open.and_then(|open| {
            open.notes_menus
                .read(cx)
                .popup_below("task-card-notes-completions", cx)
        });
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
            .on_click(cx.listener(|card, _: &ClickEvent, window, cx| card.close(Some(window), cx)));
        let menu_button = div()
            .id("task-card-menu")
            .debug_selector(|| "task-card-menu".into())
            .role(gpui::Role::Button)
            .aria_label("More options")
            .aria_expanded(self.menu.is_open())
            .relative()
            .flex_none()
            .size(px(24.0))
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(6.0))
            .cursor_pointer()
            .hover(|s| s.bg(theme.glass_hover()))
            .child(
                icon(icons::LUCIDE_ELLIPSIS)
                    .size(px(14.0))
                    .text_color(theme.text_muted),
            )
            .on_mouse_down(
                gpui::MouseButton::Left,
                cx.listener(|card, _, _, _| card.menu.note_trigger_press()),
            )
            .on_click(cx.listener(|card, _: &ClickEvent, _, cx| card.toggle_menu(cx)))
            .children(self.render_menu(&task_id, done, theme, cx));
        let header = div()
            .flex()
            .items_start()
            .gap(px(4.0))
            .child(
                div()
                    .relative()
                    .flex_1()
                    .min_w_0()
                    .text_size(ui_rems(15.0))
                    .font_weight(gpui::FontWeight::MEDIUM)
                    .child(title)
                    .children(title_popup),
            )
            .child(menu_button)
            .child(close);
        let note = div()
            .id("task-card-notes")
            .debug_selector(|| "task-card-notes".into())
            .relative()
            .text_size(ui_rems(13.0))
            .capture_action(cx.listener(|card, _: &Submit, window, cx| {
                let Some(open) = card.open.as_ref() else {
                    return;
                };
                if open.notes.focus_handle(cx).is_focused(window)
                    && !open.notes_menus.read(cx).has_selection(cx)
                {
                    cx.stop_propagation();
                    open.notes.update(cx, |input, cx| {
                        input.replace_text_in_range(None, "\n", window, cx)
                    });
                }
            }))
            .child(crate::composer::growing_field(notes, 20.0, 220.0, cx))
            .children(notes_popup);
        let saved = self
            .open
            .as_ref()
            .map(|open| open.images.clone())
            .unwrap_or_default();
        let thumbs = (!saved.is_empty()).then(|| {
            let open_paths = saved.clone();
            images::strip(
                "task-card",
                &saved,
                images::SPEC_THUMB,
                &self.state,
                theme,
                move |card: &mut Self, ix, _, cx| {
                    card.gallery
                        .update(cx, |gallery, cx| gallery.open(open_paths.clone(), ix, cx))
                },
                Some(
                    |card: &mut Self, ix, _: &mut Window, cx: &mut Context<Self>| {
                        card.remove_image(ix, cx)
                    },
                ),
                cx,
            )
            .pt(px(6.0))
        });
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
                                .child(zeron_doc::tree::linked_label(linked.indicator)),
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

        let reopen_task = task_id.clone();
        let actions = done.then(|| {
            div()
                .flex()
                .items_center()
                .justify_end()
                .gap(px(6.0))
                .children(open_chat("Open chat", cx))
                .child(
                    widgets::text_action(theme, widgets::ActionTone::Filled, "Mark as not done")
                        .id("task-card-reopen")
                        .debug_selector(|| "task-card-reopen".into())
                        .rounded_full()
                        .on_click(cx.listener(move |card, _: &ClickEvent, _, cx| {
                            card.set_completed(&reopen_task, false, cx);
                        })),
                )
        });

        div()
            .id("task-card")
            .debug_selector(|| "task-card".into())
            .track_focus(&self.focus)
            .w_full()
            .max_h_full()
            .overflow_y_scroll()
            .p(px(16.0))
            .rounded(px(CARD_RADIUS))
            .bg(theme.surface_raised)
            .border_1()
            .border_color(theme.border)
            .on_key_down(cx.listener(|card, event: &KeyDownEvent, _, cx| {
                let modifiers = &event.keystroke.modifiers;
                if event.keystroke.key == "enter" && (modifiers.platform || modifiers.control) {
                    cx.stop_propagation();
                    match card.open.as_ref().map(|open| open.tab) {
                        Some(HandOff::Thread) => card.assign(true, cx),
                        Some(_) => card.start(cx),
                        None => {}
                    }
                }
            }))
            .child(
                div()
                    .relative()
                    .top(px(content.rise))
                    .opacity(content.opacity)
                    .flex()
                    .flex_col()
                    .gap(px(14.0))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(4.0))
                            .child(header)
                            .child(note)
                            .children(thumbs),
                    )
                    .child(chips)
                    .children(body)
                    .children(actions),
            )
            .when(content.inert, |el| {
                el.child(div().absolute().inset_0().occlude())
            })
            .drag_over::<gpui::ExternalPaths>({
                let accent = theme.accent;
                move |style, _, _, _| style.border_color(accent)
            })
            .on_drop(cx.listener(|card, paths: &gpui::ExternalPaths, _, cx| {
                cx.stop_propagation();
                let staged = images::staged_files(paths.paths(), cx);
                card.add_images(staged, cx);
            }))
            .child(div().absolute().size_0().child(self.gallery.clone()))
            .into_any_element()
    }
}

impl Render for TaskCard {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if let Some(open) = &self.open {
            for menus in [&open.title_menus, &open.notes_menus] {
                menus.update(cx, |menus, cx| menus.close_if_blurred(window, cx));
            }
        }
        let now = motion::now();
        let expired = self.leaving.as_ref().is_some_and(|leaving| {
            leaving
                .swap_out
                .map_or(self.presence.settled_at(now), |fade| fade.settled_at(now))
        });
        if expired {
            self.leaving = None;
        }
        if self.animating(now) {
            window.request_animation_frame();
        }
        let swap_out = self
            .leaving
            .as_ref()
            .and_then(|leaving| leaving.swap_out)
            .map(|fade| fade.value_at(now));
        let content = match (swap_out, &self.open, &self.leaving) {
            (Some(fade), _, _) => Content {
                opacity: fade,
                rise: -SWAP_OUT_RISE * (1.0 - fade),
                inert: true,
            },
            (None, Some(_), _) => {
                let shown = self.content.value_at(now);
                Content {
                    opacity: shown,
                    rise: self.content_drop * (1.0 - shown),
                    inert: false,
                }
            }
            (None, None, Some(_)) => Content {
                opacity: 1.0,
                rise: 0.0,
                inert: true,
            },
            (None, None, None) => return div().into_any_element(),
        };
        if !content.inert {
            return self.render_open(content, cx);
        }
        let Some(mut leaving) = self.leaving.take() else {
            return div().into_any_element();
        };
        let current = std::mem::replace(&mut self.open, Some(leaving.open));
        let element = self.render_open(content, cx);
        if let Some(open) = std::mem::replace(&mut self.open, current) {
            leaving.open = open;
            self.leaving = Some(leaving);
        }
        element
    }
}

impl TaskCard {
    fn render_open(&mut self, content: Content, cx: &mut Context<Self>) -> AnyElement {
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
        self.render_card(fields, row.as_ref(), content, &theme, cx)
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

    fn chat(id: &str) -> zeron_proto::Chat {
        serde_json::from_value(serde_json::json!({
            "id": id, "title": id, "deviceId": "mac", "archived": false,
            "createdAt": "2026-10-18T00:00:00Z",
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

    #[gpui::test]
    fn a_row_dragged_by_its_grip_moves_one_slot_and_escape_springs_it_back(
        cx: &mut gpui::TestAppContext,
    ) {
        let _dir = init(cx);
        let state = cx.new(|_| AppState::new());
        state.update(cx, |state, _| {
            state.tasks = vec![task("a", false), task("b", false), task("c", false)];
        });
        let (page, cx) = cx.add_window_view(|_, cx| super::super::TasksPage::new(state, cx));
        let moved = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        let sink = moved.clone();
        cx.update(|_, cx| {
            cx.subscribe(&page, move |_, event: &super::super::TasksEvent, _| {
                if let super::super::TasksEvent::Update { task_id, patch } = event
                    && let Some(key) = &patch.order_key
                {
                    sink.borrow_mut().push((task_id.clone(), key.clone()));
                }
            })
            .detach();
        });
        cx.run_until_parked();
        let pitch = cx.debug_bounds("task-row-c").unwrap().top()
            - cx.debug_bounds("task-row-b").unwrap().top();
        let grip = cx.debug_bounds("task-grip-b").unwrap().center();
        let drag_by = |cx: &mut gpui::VisualTestContext, dy: gpui::Pixels| {
            cx.simulate_mouse_down(grip, gpui::MouseButton::Left, gpui::Modifiers::none());
            for step in 1..=6 {
                let at = gpui::point(grip.x, grip.y + dy * (step as f32 / 6.0));
                cx.simulate_mouse_move(at, gpui::MouseButton::Left, gpui::Modifiers::none());
            }
            cx.run_until_parked();
        };
        let release = |cx: &mut gpui::VisualTestContext, dy: gpui::Pixels| {
            let at = gpui::point(grip.x, grip.y + dy);
            cx.simulate_mouse_up(at, gpui::MouseButton::Left, gpui::Modifiers::none());
            cx.run_until_parked();
            std::thread::sleep(std::time::Duration::from_millis(300));
            cx.update(|window, _| window.refresh());
            cx.run_until_parked();
        };

        drag_by(cx, pitch * 1.6);
        assert!(page.read_with(cx, |page, _| page.drag.is_some()));
        cx.simulate_keystrokes("escape");
        cx.run_until_parked();
        assert!(page.read_with(cx, |page, _| page.drag.is_none()));
        release(cx, pitch * 1.6);
        assert!(moved.borrow().is_empty(), "escape drops nothing");

        drag_by(cx, pitch * 0.4);
        release(cx, pitch * 0.4);
        assert!(moved.borrow().is_empty(), "short of c's midpoint stays put");

        drag_by(cx, pitch * 0.6);
        release(cx, pitch * 0.6);
        let moved = moved.borrow();
        assert_eq!(moved.len(), 1);
        assert_eq!(moved[0].0, "b");
        assert!(moved[0].1.as_str() > "c", "b lands after c");
    }

    #[gpui::test]
    fn a_row_with_a_thread_opens_it_instead_of_the_card(cx: &mut gpui::TestAppContext) {
        let _dir = init(cx);
        let state = cx.new(|_| AppState::new());
        state.update(cx, |state, _| {
            let linked = serde_json::from_value(serde_json::json!({
                "id": "orch", "title": "Build it", "orderKey": "a", "chatId": "lead",
            }))
            .unwrap();
            state.tasks = vec![linked, task("plain", false)];
            state.chats = vec![chat("lead")];
        });
        let (page, cx) = cx.add_window_view(|_, cx| super::super::TasksPage::new(state, cx));
        let opened = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        let sink = opened.clone();
        cx.update(|_, cx| {
            cx.subscribe(&page, move |_, event: &super::super::TasksEvent, _| {
                if let super::super::TasksEvent::OpenChat(id) = event {
                    sink.borrow_mut().push(id.clone());
                }
            })
            .detach();
        });
        cx.run_until_parked();
        let at = cx.debug_bounds("task-row-orch").unwrap().center();
        cx.simulate_click(at, gpui::Modifiers::none());
        cx.run_until_parked();
        assert_eq!(*opened.borrow(), ["lead"]);
        assert!(!page.read_with(cx, |page, cx| page.card().read(cx).is_open()));
        let at = cx.debug_bounds("task-row-plain").unwrap().center();
        cx.simulate_click(at, gpui::Modifiers::none());
        cx.run_until_parked();
        assert_eq!(*opened.borrow(), ["lead"]);
        let open = page.read_with(cx, |page, cx| {
            page.card().read(cx).task_id().map(str::to_owned)
        });
        assert_eq!(open.as_deref(), Some("plain"));

        let at = cx.debug_bounds("task-row-orch").unwrap().center();
        cx.simulate_mouse_down(at, gpui::MouseButton::Right, gpui::Modifiers::none());
        cx.simulate_mouse_up(at, gpui::MouseButton::Right, gpui::Modifiers::none());
        cx.run_until_parked();
        let edit = cx.debug_bounds("task-row-menu-edit").unwrap().center();
        cx.simulate_click(edit, gpui::Modifiers::none());
        cx.run_until_parked();
        assert_eq!(*opened.borrow(), ["lead"]);
        let open = page.read_with(cx, |page, cx| {
            page.card().read(cx).task_id().map(str::to_owned)
        });
        assert_eq!(open.as_deref(), Some("orch"));

        std::thread::sleep(crate::motion::MENU_OUT.total());
        cx.executor()
            .advance_clock(std::time::Duration::from_secs(1));
        cx.run_until_parked();
        let at = cx.debug_bounds("task-row-orch").unwrap().center();
        cx.simulate_mouse_down(at, gpui::MouseButton::Right, gpui::Modifiers::none());
        cx.simulate_mouse_up(at, gpui::MouseButton::Right, gpui::Modifiers::none());
        cx.run_until_parked();
        let edit = cx.debug_bounds("task-row-menu-edit").unwrap().center();
        cx.simulate_click(edit, gpui::Modifiers::none());
        cx.run_until_parked();
        let title = page.read_with(cx, |page, cx| {
            page.card()
                .read(cx)
                .open
                .as_ref()
                .unwrap()
                .title
                .focus_handle(cx)
        });
        assert_eq!(
            cx.update(|window, cx| window.focused(cx)),
            Some(title),
            "Edit on the task the card already shows puts the caret back in its title"
        );
    }

    #[gpui::test]
    fn an_open_card_keeps_its_save_in_flight_and_takes_a_save_from_elsewhere(
        cx: &mut gpui::TestAppContext,
    ) {
        let _dir = init(cx);
        let state = cx.new(|_| AppState::new());
        state.update(cx, |state, _| state.tasks = vec![task("plain", false)]);
        let (card, cx) = cx.add_window_view(|_, cx| TaskCard::new(state.clone(), cx));
        card.update(cx, |card, cx| {
            card.open("plain".into(), None, CardFocus::Title, None, cx)
        });
        cx.run_until_parked();
        let notes = |card: &Entity<TaskCard>, cx: &mut gpui::VisualTestContext| {
            card.read_with(cx, |card, cx| {
                card.open
                    .as_ref()
                    .unwrap()
                    .notes
                    .read(cx)
                    .text()
                    .to_string()
            })
        };
        card.update(cx, |card, cx| {
            let input = card.open.as_ref().unwrap().notes.clone();
            input.update(cx, |input, cx| input.set_text("typed", cx));
            card.flush(cx);
        });
        state.update(cx, |_, cx| cx.notify());
        cx.run_until_parked();
        assert_eq!(notes(&card, cx), "typed");
        state.update(cx, |state, cx| {
            state.tasks[0].notes = "from the inbox".into();
            cx.notify();
        });
        cx.run_until_parked();
        assert_eq!(notes(&card, cx), "from the inbox");
    }

    fn skill_row() -> zeron_proto::command_catalog::InvocationCandidate {
        zeron_proto::command_catalog::InvocationCandidate {
            name: "ship".into(),
            description: String::new(),
            input_hint: None,
            invocation: zeron_proto::invocation::Invocation::Skill {
                name: "ship".into(),
                path: "/repo/.claude/skills/ship/SKILL.md".into(),
                command: None,
            },
            workspace_command: None,
        }
    }

    #[gpui::test]
    fn enter_in_the_notes_picks_a_highlighted_row_and_only_then_breaks_the_line(
        cx: &mut gpui::TestAppContext,
    ) {
        let _dir = init(cx);
        cx.update(|cx| crate::composer::init(cx, Default::default()));
        let state = cx.new(|_| AppState::new());
        state.update(cx, |state, _| state.tasks = vec![task("plain", false)]);
        let (card, cx) = cx.add_window_view(|_, cx| TaskCard::new(state, cx));
        card.update_in(cx, |card, window, cx| {
            card.open("plain".into(), None, CardFocus::Notes, Some(window), cx);
            let input = card.open.as_ref().unwrap().notes.clone();
            input.update(cx, |input, cx| input.set_text("Do $sh", cx));
        });
        cx.run_until_parked();
        card.update(cx, |card, cx| {
            let menus = card.open.as_ref().unwrap().notes_menus.clone();
            menus.update(cx, |menus, cx| menus.seed_slash_rows(vec![skill_row()], cx));
        });
        cx.run_until_parked();
        let notes = |card: &Entity<TaskCard>, cx: &mut gpui::VisualTestContext| {
            card.read_with(cx, |card, cx| {
                let open = card.open.as_ref().unwrap();
                open.notes.read(cx).text().to_string()
            })
        };
        cx.simulate_keystrokes("enter");
        let link = skill_row().invocation.link();
        assert_eq!(notes(&card, cx), format!("Do {link} "));
        cx.simulate_keystrokes("enter");
        assert_eq!(notes(&card, cx), format!("Do {link} \n"));
    }

    #[gpui::test]
    fn a_reference_that_would_overflow_the_title_is_refused_whole(cx: &mut gpui::TestAppContext) {
        let _dir = init(cx);
        cx.update(|cx| crate::composer::init(cx, Default::default()));
        let state = cx.new(|_| AppState::new());
        state.update(cx, |state, _| state.tasks = vec![task("plain", false)]);
        let (card, cx) = cx.add_window_view(|_, cx| TaskCard::new(state, cx));
        let typed = format!("{} $sh", "x".repeat(zeron_doc::TASK_TITLE_MAX_CHARS - 10));
        card.update_in(cx, |card, window, cx| {
            card.open("plain".into(), None, CardFocus::Title, Some(window), cx);
            let input = card.open.as_ref().unwrap().title.clone();
            input.update(cx, |input, cx| input.set_text(typed.clone(), cx));
        });
        cx.run_until_parked();
        card.update(cx, |card, cx| {
            let menus = card.open.as_ref().unwrap().title_menus.clone();
            menus.update(cx, |menus, cx| menus.seed_slash_rows(vec![skill_row()], cx));
        });
        cx.run_until_parked();
        cx.simulate_keystrokes("enter");
        card.read_with(cx, |card, cx| {
            let open = card.open.as_ref().unwrap();
            assert_eq!(open.title.read(cx).text(), typed);
            assert!(!open.title_menus.read(cx).is_open(cx));
        });
    }

    #[gpui::test]
    fn tab_in_the_add_row_picks_a_highlighted_row_and_only_then_opens_the_task(
        cx: &mut gpui::TestAppContext,
    ) {
        let _dir = init(cx);
        cx.update(|cx| crate::composer::init(cx, Default::default()));
        let state = cx.new(|_| AppState::new());
        let (page, cx) = cx.add_window_view(|_, cx| super::super::TasksPage::new(state, cx));
        cx.run_until_parked();
        page.update_in(cx, |page, window, cx| {
            window.focus(&page.add.focus_handle(cx), cx);
            page.add
                .update(cx, |input, cx| input.set_text("Do $sh", cx));
        });
        cx.run_until_parked();
        page.update(cx, |page, cx| {
            page.add_menus
                .update(cx, |menus, cx| menus.seed_slash_rows(vec![skill_row()], cx));
        });
        cx.run_until_parked();
        cx.simulate_keystrokes("tab");
        let link = skill_row().invocation.link();
        let text = page.read_with(cx, |page, cx| page.add.read(cx).text().to_string());
        assert_eq!(text, format!("Do {link} "));
        cx.simulate_keystrokes("tab");
        let text = page.read_with(cx, |page, cx| page.add.read(cx).text().to_string());
        assert_eq!(text, "");
        cx.run_until_parked();
        page.update_in(cx, |page, window, cx| {
            let notes = page.card.read(cx).open.as_ref().unwrap().notes.clone();
            assert!(notes.focus_handle(cx).is_focused(window));
        });
    }

    #[gpui::test]
    fn start_tells_no_project_apart_from_the_tasks_own(cx: &mut gpui::TestAppContext) {
        let _dir = init(cx);
        let state = cx.new(|_| AppState::new());
        state.update(cx, |state, _| {
            let mut plain = task("plain", false);
            plain.space_id = Some("shop".into());
            state.tasks = vec![plain];
        });
        let (card, cx) = cx.add_window_view(|_, cx| TaskCard::new(state, cx));
        let starts = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        let sink = starts.clone();
        cx.update(|_, cx| {
            cx.subscribe(&card, move |_, event: &TaskCardEvent, _| {
                if let TaskCardEvent::Start { space_id, .. } = event {
                    sink.borrow_mut().push(space_id.clone());
                }
            })
            .detach();
        });
        for run_in in [None, Some(None)] {
            card.update(cx, |card, cx| {
                card.starting.clear();
                card.open("plain".into(), None, CardFocus::Title, None, cx);
                card.open.as_mut().unwrap().run_in = run_in;
            });
            cx.run_until_parked();
            let at = cx.debug_bounds("task-card-start").unwrap().center();
            cx.simulate_click(at, gpui::Modifiers::none());
            cx.run_until_parked();
        }
        assert_eq!(*starts.borrow(), [None, Some(None)]);
    }

    #[gpui::test]
    fn a_closing_card_lets_go_of_focus(cx: &mut gpui::TestAppContext) {
        let _dir = init(cx);
        let state = cx.new(|_| AppState::new());
        state.update(cx, |state, _| state.tasks = vec![task("plain", false)]);
        let (card, cx) = cx.add_window_view(|_, cx| TaskCard::new(state, cx));
        card.update_in(cx, |card, window, cx| {
            card.open("plain".into(), None, CardFocus::Title, Some(window), cx)
        });
        cx.run_until_parked();
        let focused = |cx: &mut gpui::VisualTestContext| cx.update(|window, cx| window.focused(cx));
        let title = card.read_with(cx, |card, cx| {
            card.open.as_ref().unwrap().title.focus_handle(cx)
        });
        assert_eq!(focused(cx), Some(title));
        let at = cx.debug_bounds("task-card-close").unwrap().center();
        cx.simulate_click(at, gpui::Modifiers::none());
        assert!(card.read_with(cx, |card, _| card.leaving.is_some()));
        assert_eq!(
            focused(cx),
            None,
            "keys typed while it slides out reach nothing"
        );
    }

    #[gpui::test]
    fn every_card_state_renders_its_controls(cx: &mut gpui::TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        cx.update(|cx| {
            gpui_base::init(cx);
            cx.set_global(Theme::dark());
            crate::settings::init(crate::settings::UiSettings::default(), dir.path(), cx);
            crate::motion::set_reduced_motion(cx, true);
        });
        let state = cx.new(|_| AppState::new());
        state.update(cx, |state, _| {
            state.tasks = vec![task("plain", false), task("finished", true)];
        });
        let (card, cx) = cx.add_window_view(|_, cx| TaskCard::new(state, cx));
        card.update(cx, |card, cx| {
            card.open("plain".into(), None, CardFocus::Title, None, cx);
        });
        cx.run_until_parked();
        assert!(cx.debug_bounds("task-card-delete").is_none());
        let at = cx.debug_bounds("task-card-menu").unwrap().center();
        cx.simulate_click(at, gpui::Modifiers::none());
        cx.run_until_parked();
        for selector in [
            "task-card-hand-off",
            "task-card-start",
            "task-card-folder",
            "task-card-due",
            "task-card-project",
            "task-card-toggle-done",
            "task-card-delete",
        ] {
            assert!(cx.debug_bounds(selector).is_some(), "{selector}");
        }
        card.update(cx, |card, cx| {
            card.open("finished".into(), None, CardFocus::Title, None, cx)
        });
        cx.run_until_parked();
        assert!(cx.debug_bounds("task-card-hand-off").is_none());
        assert!(cx.debug_bounds("task-card-reopen").is_some());
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
        });
        cx.run_until_parked();
        let click = |cx: &mut gpui::VisualTestContext, selector: &'static str| {
            let at = cx.debug_bounds(selector).unwrap().center();
            cx.simulate_click(at, gpui::Modifiers::none());
            cx.run_until_parked();
        };
        click(cx, "task-card-hand-off-mode-1");
        assert!(cx.debug_bounds("task-card-start").is_none());
        click(cx, "task-card-assign-button");
        assert!(events.borrow().is_empty(), "Send task waits for a pick");
        for (button, send) in [
            ("task-card-assign-button", true),
            ("task-card-link-only", false),
        ] {
            card.update(cx, |card, cx| {
                let input = card.open.as_ref().unwrap().assign.clone();
                input.update(cx, |input, cx| input.set_text("@fix log", cx));
            });
            cx.run_until_parked();
            assert!(cx.debug_bounds("task-card-assign-match-0").is_some());
            assert!(cx.debug_bounds("task-card-assign-match-1").is_none());
            click(cx, "task-card-assign-match-0");
            assert!(cx.debug_bounds("task-card-assign-matches").is_none());
            assert!(cx.debug_bounds("task-card-assign-picked").is_some());
            click(cx, button);
            assert_eq!(events.borrow().last(), Some(&("fix".to_string(), send)));
        }
    }
}
