use std::collections::HashMap;
use std::time::Instant;

use chrono::Utc;
use gpui::{
    AnyElement, App, ClickEvent, Context, Entity, EntityInputHandler, EventEmitter, FocusHandle,
    Focusable, IntoElement, KeyDownEvent, PathPromptOptions, Render, SharedString, Subscription,
    Task, Window, div, prelude::*, px,
};
use zeron_proto::tools::{ToolSettings, ToolSettingsState};
use zeron_proto::{ChatConfig, ChatIndicator, TaskPatch, TaskStatus};

use super::images::{self, ImageGallery};
use super::logic::{self, DuePick};
use super::{Row, SAVE_DEBOUNCE, START_PENDING, linked_chat, patched, row_for};
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
const MISSION_LABEL_WIDTH: f32 = 62.0;
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
        orchestrate: Option<Orchestrate>,
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
pub struct Orchestrate {
    pub approve_plan: bool,
    pub same_agents: bool,
    pub plan_doc: bool,
}

impl Default for Orchestrate {
    fn default() -> Self {
        Self {
            approve_plan: true,
            same_agents: false,
            plan_doc: true,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HandOff {
    NewChat,
    Thread,
    Mission,
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
    mission: Orchestrate,
    reads: Entity<ComposerInput>,
    reads_hidden: bool,
    context: Vec<String>,
    _subscriptions: [Subscription; 4],
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
    tool_settings: Option<ToolSettings>,
    tools_fetch: Option<Task<()>>,
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
        let now = Instant::now();
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
            tool_settings: None,
            tools_fetch: None,
            _state_changes: state_changes,
        }
    }

    pub fn task_id(&self) -> Option<&str> {
        self.open.as_ref().map(|open| open.task_id.as_str())
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
        let now = Instant::now();
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
        let context = row
            .as_ref()
            .map(|task| task.context.clone())
            .unwrap_or_default();
        self.menu = popover::Popup::default();
        self.fetch_tool_settings(cx);
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
        let reads_input = thread_input("@ chats the lead reads first", cx);
        let subscriptions = [
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
            cx.subscribe(&reads_input, |card: &mut Self, _, event, cx| {
                match event {
                    ComposerInputEvent::Edited => {
                        if let Some(open) = card.open.as_mut() {
                            open.reads_hidden = false;
                        }
                    }
                    ComposerInputEvent::Submitted => {
                        if let Some(first) = card.reads_matches(cx).into_iter().next() {
                            card.add_context(first.chat_id, cx);
                        }
                    }
                    _ => {}
                }
                cx.notify();
            }),
        ];
        let now = Instant::now();
        let reduced = motion::reduced_motion(cx);
        let previous = self.open.replace(Open {
            task_id,
            title: title_input,
            notes: notes_input,
            saved_title,
            saved_notes,
            images: saved_images,
            uploads: Vec::new(),
            refused: false,
            seen: row.is_some(),
            save: None,
            run_in: None,
            folder: None,
            tab: if context.is_empty() {
                HandOff::NewChat
            } else {
                HandOff::Mission
            },
            assign: assign_input,
            assign_pick: None,
            assign_hidden: false,
            mission: Orchestrate::default(),
            reads: reads_input,
            reads_hidden: false,
            context,
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
        let present = self
            .state
            .read(cx)
            .tasks
            .iter()
            .any(|task| task.id == open.task_id && !task.deleted);
        if present {
            open.seen = true;
        } else if open.seen {
            self.dismiss(None, cx);
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
        if patch.context.as_ref() == Some(&open.context) {
            open.context = row.as_ref().map(|t| t.context.clone()).unwrap_or_default();
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
                let room = zeron_doc::TASK_IMAGES_MAX.saturating_sub(open.images.len());
                if paths.len() > room {
                    crate::toast::error(cx, "A task holds up to 12 images.");
                }
                open.images.extend(paths.into_iter().take(room));
                card.save_images(cx);
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
        let Some((task_id, orchestrate, context)) = self.open.as_ref().map(|open| {
            let mission = (open.tab == HandOff::Mission).then_some(open.mission);
            (open.task_id.clone(), mission, open.context.clone())
        }) else {
            return;
        };
        if orchestrate.is_some() && !context.is_empty() && self.reads_off(cx) {
            return;
        }
        let state = self.state.read(cx);
        let row = state.tasks.iter().find(|task| {
            task.id == task_id
                && linked_chat(&state.chats, task).is_none()
                && task.completed_at.is_none()
        });
        let Some(saved_context) = row.map(|task| task.context.clone()) else {
            return;
        };
        if self.is_starting(&task_id) {
            return;
        }
        self.starting.insert(task_id.clone(), Instant::now());
        cx.spawn(async move |card, cx| {
            cx.background_executor().timer(START_PENDING).await;
            card.update(cx, |_, cx| cx.notify()).ok();
        })
        .detach();
        let mut patch = self.take_patch(cx).map(|(_, patch)| patch);
        if orchestrate.is_some() && context != saved_context {
            patch.get_or_insert_default().context = Some(context);
        }
        let space_id = self.open.as_ref().and_then(|open| open.run_in.clone());
        let config = self.agent.read(cx).resolved(cx).chat_config();
        cx.emit(TaskCardEvent::Start {
            task_id,
            patch,
            space_id,
            config,
            orchestrate,
        });
        cx.notify();
    }

    fn assign_matches(&self, cx: &Context<Self>) -> Vec<crate::composer::ThreadMatch> {
        match self.open.as_ref() {
            Some(open) if open.assign_pick.is_none() => {
                self.thread_offer(&open.assign, &open.task_id, &[], cx)
            }
            _ => Vec::new(),
        }
    }

    fn reads_matches(&self, cx: &Context<Self>) -> Vec<crate::composer::ThreadMatch> {
        match self.open.as_ref() {
            Some(open) if open.context.len() < zeron_doc::TASK_CONTEXT_MAX => {
                self.thread_offer(&open.reads, &open.task_id, &open.context, cx)
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

    fn add_context(&mut self, chat_id: String, cx: &mut Context<Self>) {
        let Some(open) = self.open.as_mut() else {
            return;
        };
        if open.context.len() >= zeron_doc::TASK_CONTEXT_MAX || open.context.contains(&chat_id) {
            return;
        }
        open.context.push(chat_id);
        open.reads.update(cx, |input, cx| input.set_text("", cx));
        self.save_context(cx);
    }

    fn remove_context(&mut self, chat_id: &str, cx: &mut Context<Self>) {
        if let Some(open) = self.open.as_mut() {
            open.context.retain(|id| id != chat_id);
            self.save_context(cx);
        }
    }

    fn save_context(&mut self, cx: &mut Context<Self>) {
        let Some(open) = self.open.as_ref() else {
            return;
        };
        let (task_id, context) = (open.task_id.clone(), open.context.clone());
        self.update(
            &task_id,
            TaskPatch {
                context: Some(context),
                ..Default::default()
            },
            cx,
        );
        cx.notify();
    }

    fn fetch_tool_settings(&mut self, cx: &mut Context<Self>) {
        let Some(engine) = self.state.read(cx).engine().cloned() else {
            return;
        };
        self.tools_fetch = Some(cx.spawn(async move |card, cx| {
            let reply = engine
                .client()
                .call_as::<ToolSettingsState>(
                    zeron_rpc::methods::GET_TOOL_SETTINGS,
                    serde_json::json!({}),
                )
                .await;
            card.update(cx, |card, cx| {
                if let Ok(state) = reply {
                    card.tool_settings = Some(state.settings);
                    cx.notify();
                }
            })
            .ok();
        }));
    }

    /// The engine's `reads_threads` test, so the lead gets no Reads block exactly when this is true.
    fn reads_off(&self, cx: &App) -> bool {
        let Some(settings) = &self.tool_settings else {
            return false;
        };
        let disabled = self
            .agent
            .read(cx)
            .resolved(cx)
            .chat_config()
            .map(|config| config.disabled_tools)
            .unwrap_or_default();
        settings.chat_tool_off(&disabled, "read_thread")
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

    fn set_mission(&mut self, mission: Orchestrate, cx: &mut Context<Self>) {
        if let Some(open) = self.open.as_mut() {
            open.mission = mission;
        }
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
        let reads = shown(open.reads_hidden, &open.reads);
        match open.tab {
            HandOff::Thread if assign => open.assign_hidden = true,
            HandOff::Mission if reads => open.reads_hidden = true,
            _ => return false,
        }
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
        label: &'static str,
        enabled: bool,
        theme: &Theme,
    ) -> gpui::Stateful<gpui::Div> {
        widgets::action_button(theme, widgets::ActionTone::Solid)
            .id(id)
            .debug_selector(move || id.into())
            .role(gpui::Role::Button)
            .aria_label(label)
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
            &[
                (HandOff::NewChat, "New chat"),
                (HandOff::Thread, "Thread"),
                (HandOff::Mission, "Mission Control"),
            ],
            tab,
            move |tab, _, cx| {
                weak.update(cx, |card, cx| card.set_tab(tab, cx)).ok();
            },
        )
        .debug_selector(|| "task-card-hand-off-mode".into());
        let starting = self.is_starting(task_id);
        let ready = row.is_some() && !starting;
        let (pane, footer) = match tab {
            HandOff::NewChat => {
                let start = self
                    .solid_button(
                        "task-card-start",
                        if starting { "Starting…" } else { "Start" },
                        ready,
                        theme,
                    )
                    .on_click(cx.listener(|card, _: &ClickEvent, _, cx| card.start(cx)));
                let pane = div()
                    .flex()
                    .flex_col()
                    .gap(px(2.0))
                    .child(self.render_agent())
                    .child(self.render_run_line(row, theme, cx));
                (pane.into_any_element(), vec![start.into_any_element()])
            }
            HandOff::Thread => self.render_thread_tab(theme, cx),
            HandOff::Mission => {
                let blocked = !open.context.is_empty() && self.reads_off(cx);
                let start = self
                    .solid_button(
                        "task-card-orchestrate",
                        if starting {
                            "Starting…"
                        } else {
                            "Start mission"
                        },
                        ready && !blocked,
                        theme,
                    )
                    .on_click(cx.listener(|card, _: &ClickEvent, _, cx| card.start(cx)));
                (
                    self.render_mission_tab(open.mission, row, theme, cx),
                    vec![start.into_any_element()],
                )
            }
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
                            .child(super::TasksPage::linked_label(indicator))
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
            .tab_index(0)
            .rounded_full()
            .min_h(px(30.0))
            .px(px(14.0))
            .border_1()
            .border_color(theme.border)
            .when(!enabled, |el| el.opacity(0.4))
            .on_click(cx.listener(|card, _: &ClickEvent, _, cx| card.assign(false, cx)));
        let send = self
            .solid_button("task-card-assign-button", "Send task", enabled, theme)
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

    fn render_mission_tab(
        &self,
        choice: Orchestrate,
        row: Option<&Row>,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let weak = cx.entity().downgrade();
        let step_agents = widgets::segmented(
            theme,
            "task-card-step-agents",
            &[(false, "Lead picks"), (true, "Same as lead")],
            choice.same_agents,
            move |same_agents, _, cx| {
                weak.update(cx, |card, cx| {
                    card.set_mission(
                        Orchestrate {
                            same_agents,
                            ..choice
                        },
                        cx,
                    )
                })
                .ok();
            },
        )
        .debug_selector(|| "task-card-step-agents".into());
        let weak = cx.entity().downgrade();
        let plan_mode = widgets::segmented(
            theme,
            "task-card-plan-mode",
            &[(true, "plan.md"), (false, "Steps only")],
            choice.plan_doc,
            move |plan_doc, _, cx| {
                weak.update(cx, |card, cx| {
                    card.set_mission(Orchestrate { plan_doc, ..choice }, cx)
                })
                .ok();
            },
        )
        .debug_selector(|| "task-card-plan-mode".into());
        let (description, review) = if choice.plan_doc {
            (
                "A lead agent writes a plan you can read and edit, runs each step on its \
                 own thread, checks the results and reports back here.",
                "Let me review the plan before it runs",
            )
        } else {
            (
                "A lead agent plans the work into steps, runs each step on its own \
                 thread, checks the results and reports back here.",
                "Show me the plan before it runs",
            )
        };
        let on = choice.approve_plan;
        let approve = div()
            .flex()
            .items_center()
            .justify_between()
            .gap(px(8.0))
            .text_size(ui_rems(12.5))
            .text_color(theme.text)
            .child(review)
            .child(
                widgets::toggle_switch(theme, on, "task-card-approve-plan")
                    .id("task-card-approve-plan")
                    .debug_selector(|| "task-card-approve-plan".into())
                    .cursor_pointer()
                    .tab_index(0)
                    .role(gpui::Role::Switch)
                    .aria_label(review)
                    .aria_toggled(if on {
                        gpui::Toggled::True
                    } else {
                        gpui::Toggled::False
                    })
                    .on_click(cx.listener(move |card, _: &ClickEvent, _, cx| {
                        card.set_mission(
                            Orchestrate {
                                approve_plan: !on,
                                ..choice
                            },
                            cx,
                        )
                    })),
            );
        let field = |label: &'static str, top: bool, control: AnyElement| {
            div()
                .flex()
                .when(top, |el| el.items_start())
                .when(!top, |el| el.items_center())
                .gap(px(10.0))
                .child(
                    div()
                        .flex_none()
                        .w(px(MISSION_LABEL_WIDTH))
                        .when(top, |el| el.pt(px(7.0)))
                        .text_size(ui_rems(12.5))
                        .text_color(theme.text_muted)
                        .child(label),
                )
                .child(div().flex_1().min_w_0().child(control))
        };
        let lead = div()
            .flex()
            .flex_col()
            .gap(px(2.0))
            .child(self.render_agent())
            .child(self.render_run_line(row, theme, cx))
            .into_any_element();
        div()
            .debug_selector(|| "task-card-orchestrate-pane".into())
            .flex()
            .flex_col()
            .gap(px(8.0))
            .child(
                div()
                    .debug_selector(|| "task-card-mission-description".into())
                    .text_size(ui_rems(12.0))
                    .text_color(theme.text_muted)
                    .child(description),
            )
            .child(field(
                "Plan",
                false,
                div().flex().child(plan_mode).into_any_element(),
            ))
            .child(field("Lead", true, lead))
            .child(field("Reads", true, self.render_reads(row, theme, cx)))
            .child(field(
                "Steps",
                false,
                div().flex().child(step_agents).into_any_element(),
            ))
            .child(approve)
            .into_any_element()
    }

    fn render_reads(&self, row: Option<&Row>, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let Some(open) = self.open.as_ref() else {
            return div().into_any_element();
        };
        let off = self.reads_off(cx);
        let home = Self::run_in(Some(open), row);
        let now = Utc::now();
        let chips: Vec<AnyElement> = {
            let state = self.state.read(cx);
            open.context
                .iter()
                .filter_map(|chat_id| {
                    let chip = context_chip(state, chat_id, home.as_deref(), theme, now)?;
                    let remove = chat_id.clone();
                    Some(
                        chip.pr(px(4.0))
                            .child(
                                div()
                                    .id(SharedString::from(format!(
                                        "task-card-reads-remove-{chat_id}"
                                    )))
                                    .debug_selector({
                                        let id = format!("task-card-reads-remove-{chat_id}");
                                        move || id.clone()
                                    })
                                    .role(gpui::Role::Button)
                                    .aria_label("Remove")
                                    .flex_none()
                                    .size(px(18.0))
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .rounded_full()
                                    .cursor_pointer()
                                    .hover(|s| s.bg(theme.glass_hover()))
                                    .child(
                                        icon(icons::LUCIDE_X)
                                            .size(px(11.0))
                                            .text_color(theme.text_muted),
                                    )
                                    .on_click(cx.listener(move |card, _: &ClickEvent, _, cx| {
                                        card.remove_context(&remove, cx)
                                    })),
                            )
                            .into_any_element(),
                    )
                })
                .collect()
        };
        let room = open.context.len() < zeron_doc::TASK_CONTEXT_MAX;
        let popover = (!off && room && !open.reads_hidden)
            .then(|| self.reads_matches(cx))
            .filter(|_| !open.reads.read(cx).text().trim().is_empty())
            .map(|matches| {
                let rows = self.thread_rows(
                    matches,
                    "task-card-reads-match",
                    |card: &mut Self, chat_id, _, cx| card.add_context(chat_id, cx),
                    theme,
                    cx,
                );
                self.match_popover(
                    "task-card-reads-matches",
                    rows,
                    "Enter adds the first match. Pick up to 5.",
                    260.0,
                    |card: &mut Self| {
                        if let Some(open) = card.open.as_mut() {
                            open.reads_hidden = true;
                        }
                    },
                    theme,
                    cx,
                )
            });
        let count = open.context.len();
        let field = div()
            .id("task-card-reads")
            .debug_selector(|| "task-card-reads".into())
            .relative()
            .min_h(px(32.0))
            .px(px(6.0))
            .py(px(4.0))
            .rounded(px(10.0))
            .bg(theme.wash(0.06))
            .flex()
            .flex_wrap()
            .items_center()
            .gap(px(5.0))
            .children(chips)
            .when(!off && room, |el| {
                el.child(
                    div()
                        .flex_1()
                        .min_w(px(90.0))
                        .px(px(2.0))
                        .text_size(ui_rems(12.5))
                        .child(open.reads.clone()),
                )
            })
            .on_key_down(cx.listener(|card, event: &KeyDownEvent, _, cx| {
                if event.keystroke.key != "backspace" {
                    return;
                }
                let last = card.open.as_ref().and_then(|open| {
                    open.reads
                        .read(cx)
                        .text()
                        .is_empty()
                        .then(|| open.context.last().cloned())
                        .flatten()
                });
                if let Some(last) = last {
                    cx.stop_propagation();
                    card.remove_context(&last, cx);
                }
            }))
            .children(popover);
        let note = if off {
            let fix = if count > 0 {
                " Remove them to start, turn read_thread on for this lead, or pick another lead."
            } else {
                " Turn it on in the lead's profile or Settings > Tools, or pick another lead."
            };
            widgets::warning_strip(
                theme,
                format!(
                    "This lead's tools leave out read_thread, so it cannot read context chats.{fix}"
                ),
            )
            .debug_selector(|| "task-card-reads-off".into())
            .mt_0()
            .into_any_element()
        } else {
            div()
                .text_size(ui_rems(11.5))
                .text_color(theme.text_muted.opacity(0.75))
                .child(if count == 0 {
                    "Optional. The lead reads them before it plans. They are not assigned."
                        .to_string()
                } else {
                    format!(
                        "{count} of {} · read before it plans; nothing is sent to them",
                        zeron_doc::TASK_CONTEXT_MAX
                    )
                })
                .into_any_element()
        };
        div()
            .flex()
            .flex_col()
            .gap(px(5.0))
            .child(field)
            .child(note)
            .into_any_element()
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
                    .flex_1()
                    .min_w_0()
                    .text_size(ui_rems(15.0))
                    .font_weight(gpui::FontWeight::MEDIUM)
                    .child(title),
            )
            .child(menu_button)
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

pub(super) fn context_chip(
    state: &AppState,
    chat_id: &str,
    home: Option<&str>,
    theme: &Theme,
    now: chrono::DateTime<Utc>,
) -> Option<gpui::Stateful<gpui::Div>> {
    let chat = state.chats.iter().find(|chat| chat.id == chat_id)?;
    let (mark, tint) = crate::pickers::agent_brand_icon(
        chat.config.as_ref().map(|config| config.harness),
        chat.config
            .as_ref()
            .and_then(|config| config.model.as_deref()),
    );
    let indicator = state.display_status_for(chat, now);
    let (_, _, tone) = crate::shell::chat_status_tone(indicator, theme);
    let project = chat
        .space_id
        .as_deref()
        .filter(|space| Some(*space) != home)
        .and_then(|_| state.space_for_chat(chat))
        .map(|space| space.display_name().to_string());
    let mut tip = match indicator {
        ChatIndicator::Working => "Still working. The lead reads what is there when it starts, \
                                   then reads it again before it sends the plan."
            .to_string(),
        ChatIndicator::AwaitingInput => {
            "Waiting on you. The lead sees the question but leaves it to you.".to_string()
        }
        ChatIndicator::Errored => "Its last turn failed. The lead reads it like any other; \
                                   the failure is part of the context."
            .to_string(),
        _ => "Context only. The lead reads it with read_thread before it plans. Nothing is \
              sent to it, and the task does not follow it."
            .to_string(),
    };
    if let Some(project) = &project {
        tip.push_str(&format!(
            " It is in {project}, another project on this device."
        ));
    }
    let title = crate::transcript::single_line(chat.title.as_deref().unwrap_or("New session"));
    let selector = format!("task-context-{chat_id}");
    Some(
        div()
            .id(SharedString::from(selector.clone()))
            .debug_selector(move || selector.clone())
            .flex_none()
            .max_w_full()
            .h(px(24.0))
            .pl(px(7.0))
            .pr(px(8.0))
            .flex()
            .items_center()
            .gap(px(5.0))
            .rounded_full()
            .bg(theme.wash(0.08))
            .text_size(ui_rems(12.0))
            .text_color(theme.text)
            .tooltip(widgets::text_tooltip(tip))
            .child(
                icon(mark)
                    .size(px(12.0))
                    .flex_none()
                    .text_color(tint.unwrap_or(theme.text_muted)),
            )
            .child(div().flex_none().size(px(7.0)).rounded_full().bg(tone))
            .child(div().min_w_0().max_w(px(150.0)).truncate().child(title))
            .children(project.map(|project| {
                div()
                    .flex_none()
                    .max_w(px(80.0))
                    .truncate()
                    .px(px(4.0))
                    .rounded(px(5.0))
                    .border_1()
                    .border_color(theme.border)
                    .text_size(ui_rems(10.5))
                    .text_color(theme.text_muted)
                    .child(project)
            })),
    )
}

impl Render for TaskCard {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let now = Instant::now();
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

    fn failed_turn() -> Option<zeron_proto::StepTurn> {
        Some(zeron_proto::StepTurn {
            ended: zeron_proto::StepTurnEnd::Failed,
            turn: None,
        })
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
        state.chats = vec![chat("lead"), chat("child")];
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

        state.task_steps[1].turn = failed_turn();
        let plan = super::super::rollup(&state, &state.tasks[0], now);
        assert_eq!(plan.steps[1].status, Failed);
        assert_eq!(plan.status, TaskStatus::NeedsYou);
        assert_eq!(super::super::needs_you_count(&state), 1);
    }

    #[gpui::test]
    fn an_orchestrated_row_opens_the_center_instead_of_the_card(cx: &mut gpui::TestAppContext) {
        let _dir = init(cx);
        let state = cx.new(|_| AppState::new());
        state.update(cx, |state, _| {
            state.tasks = vec![orchestrated(false), task("plain", false)];
            state.chats = vec![chat("lead")];
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

        let at = cx.debug_bounds("task-row-orch").unwrap().center();
        cx.simulate_mouse_down(at, gpui::MouseButton::Right, gpui::Modifiers::none());
        cx.simulate_mouse_up(at, gpui::MouseButton::Right, gpui::Modifiers::none());
        cx.run_until_parked();
        let edit = cx.debug_bounds("task-row-menu-edit").unwrap().center();
        cx.simulate_click(edit, gpui::Modifiers::none());
        cx.run_until_parked();
        assert_eq!(*opened.borrow(), ["orch"]);
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
    fn the_center_shows_the_selected_steps_thread_or_else_the_lead(cx: &mut gpui::TestAppContext) {
        let _dir = init(cx);
        let state = cx.new(|_| AppState::new());
        state.update(cx, |state, _| {
            state.tasks = vec![orchestrated(true)];
            state.chats = vec![chat("lead"), chat("child")];
            state.task_steps = vec![
                step("s1", "1", Some("child"), None),
                step("s2", "2", None, None),
            ];
            state.task_steps[0].turn = failed_turn();
        });
        let (center, cx) =
            cx.add_window_view(|_, cx| super::super::TaskCenter::new(state, "orch".into(), cx));
        cx.update(|_, cx| crate::motion::set_reduced_motion(cx, true));
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
                        EditPlan { edit, approve, .. } => sink.borrow_mut().push(format!(
                            "edit {:?} remove {:?} approve {approve}",
                            edit.steps
                                .iter()
                                .map(|step| (step.step_id.as_deref(), step.title.as_str()))
                                .collect::<Vec<_>>(),
                            edit.remove_step_ids,
                        )),
                        Update { .. } | UpdateStep { .. } => sink.borrow_mut().push("write".into()),
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
        let click = |cx: &mut gpui::VisualTestContext, selector: &'static str| {
            let at = cx.debug_bounds(selector).unwrap().center();
            cx.simulate_click(at, gpui::Modifiers::none());
            cx.run_until_parked();
        };
        let rename_s2 = |cx: &mut gpui::VisualTestContext| {
            click(cx, "task-center-edit-s2");
            cx.simulate_input(" now");
            click(cx, "task-center-edit-save");
        };
        assert!(cx.debug_bounds("task-center-edit-s2").is_none());
        click(cx, "task-center-spec-s2");
        rename_s2(cx);
        assert!(cx.debug_bounds("task-center-edit-bar").is_some());
        assert!(
            events.borrow().is_empty(),
            "an unsent edit never leaves the center"
        );
        click(cx, "task-center-undo");
        assert!(cx.debug_bounds("task-center-edit-bar").is_none());
        for selector in ["task-center-retry-s1", "task-center-approve"] {
            click(cx, selector);
        }
        assert_eq!(*events.borrow(), ["retry s1", "approve orch"]);

        rename_s2(cx);
        click(cx, "task-center-send");
        assert!(cx.debug_bounds("task-center-edit-bar").is_none());
        click(cx, "task-center-remove-s2");
        assert!(cx.debug_bounds("task-center-restore-s2").is_some());
        click(cx, "task-center-approve");
        assert_eq!(
            events.borrow()[2..],
            [
                r#"edit [(Some("s2"), "s2 now")] remove [] approve false"#,
                r#"edit [] remove ["s2"] approve true"#,
            ]
        );
    }

    #[gpui::test]
    fn a_step_moved_from_its_menu_sends_the_new_order(cx: &mut gpui::TestAppContext) {
        let _dir = init(cx);
        let state = cx.new(|_| AppState::new());
        state.update(cx, |state, _| {
            state.tasks = vec![orchestrated(false)];
            state.chats = vec![chat("lead"), chat("child")];
            state.task_steps = vec![
                step("s1", "1", Some("child"), None),
                step("s2", "2", None, None),
                step("s3", "3", None, None),
            ];
        });
        let (center, cx) =
            cx.add_window_view(|_, cx| super::super::TaskCenter::new(state, "orch".into(), cx));
        let sent = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        let sink = sent.clone();
        cx.update(|_, cx| {
            cx.subscribe(
                &center,
                move |_, event: &super::super::TaskCenterEvent, _| {
                    if let super::super::TaskCenterEvent::EditPlan { edit, .. } = event {
                        sink.borrow_mut().push(edit.order.clone());
                    }
                },
            )
            .detach();
        });
        cx.run_until_parked();
        let menu = |cx: &mut gpui::VisualTestContext, step: &str, item: &'static str| {
            let at = cx
                .debug_bounds(Box::leak(
                    format!("task-center-step-{step}").into_boxed_str(),
                ))
                .unwrap()
                .center();
            cx.simulate_mouse_down(at, gpui::MouseButton::Right, gpui::Modifiers::none());
            cx.simulate_mouse_up(at, gpui::MouseButton::Right, gpui::Modifiers::none());
            cx.run_until_parked();
            let at = cx.debug_bounds(item).unwrap().center();
            cx.simulate_click(at, gpui::Modifiers::none());
            cx.run_until_parked();
            let away = cx.debug_bounds("task-center-header").unwrap().center();
            cx.simulate_click(away, gpui::Modifiers::none());
            std::thread::sleep(crate::motion::MENU_OUT.total());
            cx.executor().advance_clock(std::time::Duration::from_secs(1));
            cx.run_until_parked();
        };
        menu(cx, "s3", "task-center-menu-up");
        menu(cx, "s3", "task-center-menu-up");
        assert!(cx.debug_bounds("task-center-edit-bar").is_some());
        let at = cx.debug_bounds("task-center-send").unwrap().center();
        cx.simulate_click(at, gpui::Modifiers::none());
        cx.run_until_parked();
        assert_eq!(
            *sent.borrow(),
            [vec!["s1".to_string(), "s3".into(), "s2".into()]],
            "a started step keeps its place above the moved one"
        );
    }

    struct Uploads;

    #[async_trait::async_trait]
    impl zeron_rpc::RpcService for Uploads {
        async fn handle(
            &self,
            method: &str,
            params: serde_json::Value,
        ) -> Result<zeron_rpc::RpcReply, zeron_rpc::RpcError> {
            let reply = match method {
                zeron_rpc::methods::UPLOAD_COMMIT => serde_json::json!({
                    "path": format!("/uploads/{}", params["fileName"].as_str().unwrap_or_default()),
                }),
                _ => serde_json::json!({}),
            };
            zeron_rpc::RpcReply::value(&reply)
        }
    }

    #[gpui::test]
    fn a_pasted_step_image_goes_with_the_plan_edit_only_once_saved(cx: &mut gpui::TestAppContext) {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let _guard = runtime.enter();
        let _dir = init(cx);
        cx.executor().allow_parking();
        let state = cx.new(|_| AppState::new());
        state.update(cx, |state, _| {
            state.set_test_engine(crate::state::EngineHandle::from_test_client(
                zeron_rpc::memory_client(std::sync::Arc::new(Uploads)),
            ));
            state.tasks = vec![orchestrated(false)];
            state.chats = vec![chat("lead")];
            state.task_steps = vec![step("s1", "1", None, None)];
        });
        let (center, cx) =
            cx.add_window_view(|_, cx| super::super::TaskCenter::new(state, "orch".into(), cx));
        cx.update(|_, cx| crate::motion::set_reduced_motion(cx, true));
        let sent = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        let sink = sent.clone();
        cx.update(|_, cx| {
            cx.subscribe(
                &center,
                move |_, event: &super::super::TaskCenterEvent, _| {
                    if let super::super::TaskCenterEvent::EditPlan { edit, .. } = event {
                        sink.borrow_mut().push(edit.steps.clone());
                    }
                },
            )
            .detach();
        });
        let mut png = std::io::Cursor::new(Vec::new());
        image::RgbaImage::new(2, 2)
            .write_to(&mut png, image::ImageFormat::Png)
            .unwrap();
        let shot = gpui::Image::from_bytes(gpui::ImageFormat::Png, png.into_inner());
        let settle = |cx: &mut gpui::VisualTestContext,
                      done: &dyn Fn(&mut gpui::VisualTestContext) -> bool| {
            for _ in 0..200 {
                cx.run_until_parked();
                if done(cx) {
                    return;
                }
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            panic!("condition never settled");
        };
        let edit_and_paste = |cx: &mut gpui::VisualTestContext| {
            let at = cx.debug_bounds("task-center-step-s1").unwrap().center();
            cx.simulate_mouse_down(at, gpui::MouseButton::Right, gpui::Modifiers::none());
            cx.simulate_mouse_up(at, gpui::MouseButton::Right, gpui::Modifiers::none());
            cx.run_until_parked();
            let at = cx.debug_bounds("task-center-menu-edit").unwrap().center();
            cx.simulate_click(at, gpui::Modifiers::none());
            std::thread::sleep(crate::motion::MENU_OUT.total());
            cx.executor()
                .advance_clock(std::time::Duration::from_secs(1));
            cx.run_until_parked();
            cx.write_to_clipboard(gpui::ClipboardItem::new_image(&shot));
            cx.dispatch_action(crate::composer::Paste);
            settle(cx, &|cx| {
                cx.debug_bounds("task-center-edit-image-0").is_some()
            });
        };

        edit_and_paste(cx);
        let at = cx.debug_bounds("task-center-edit-cancel").unwrap().center();
        cx.simulate_click(at, gpui::Modifiers::none());
        cx.run_until_parked();
        assert!(cx.debug_bounds("task-center-editor").is_none());
        assert!(
            cx.debug_bounds("task-center-edit-bar").is_none(),
            "Cancel drops the new image"
        );

        edit_and_paste(cx);
        let at = cx.debug_bounds("task-center-edit-save").unwrap().center();
        cx.simulate_click(at, gpui::Modifiers::none());
        cx.run_until_parked();
        assert!(cx.debug_bounds("task-center-editor").is_none());
        assert!(
            cx.debug_bounds("task-center-spec-s1-image-0").is_some(),
            "the saved image shows under the spec"
        );
        let at = cx.debug_bounds("task-center-send").unwrap().center();
        cx.simulate_click(at, gpui::Modifiers::none());
        cx.run_until_parked();
        let sent = sent.borrow();
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0][0].step_id.as_deref(), Some("s1"));
        assert_eq!(sent[0][0].images, ["/uploads/image.png"]);
    }

    #[gpui::test]
    fn orchestrate_starts_the_task_with_the_chosen_options(cx: &mut gpui::TestAppContext) {
        let _dir = init(cx);
        let state = cx.new(|_| AppState::new());
        state.update(cx, |state, _| state.tasks = vec![task("plain", false)]);
        let (card, cx) = cx.add_window_view(|_, cx| TaskCard::new(state, cx));
        let starts = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        let sink = starts.clone();
        cx.update(|_, cx| {
            cx.subscribe(&card, move |_, event: &TaskCardEvent, _| {
                if let TaskCardEvent::Start { orchestrate, .. } = event {
                    sink.borrow_mut().push(*orchestrate);
                }
            })
            .detach();
        });
        card.update(cx, |card, cx| {
            card.open("plain".into(), None, CardFocus::Title, None, cx)
        });
        cx.run_until_parked();
        let right_half = |cx: &mut gpui::VisualTestContext, selector: &'static str| {
            let bounds = cx.debug_bounds(selector).unwrap();
            gpui::point(
                bounds.origin.x + bounds.size.width * 0.75,
                bounds.center().y,
            )
        };
        assert!(cx.debug_bounds("task-card-start").is_some());
        assert!(cx.debug_bounds("task-card-assign").is_none());
        let at = cx
            .debug_bounds("task-card-hand-off-mode-2")
            .unwrap()
            .center();
        cx.simulate_click(at, gpui::Modifiers::none());
        cx.run_until_parked();
        assert!(cx.debug_bounds("task-card-start").is_none());
        assert!(cx.debug_bounds("task-card-reads").is_some());
        let at = cx.debug_bounds("task-card-approve-plan").unwrap().center();
        cx.simulate_click(at, gpui::Modifiers::none());
        cx.run_until_parked();
        let at = right_half(cx, "task-card-step-agents");
        cx.simulate_click(at, gpui::Modifiers::none());
        cx.run_until_parked();
        let at = right_half(cx, "task-card-plan-mode");
        cx.simulate_click(at, gpui::Modifiers::none());
        cx.run_until_parked();
        let at = cx.debug_bounds("task-card-orchestrate").unwrap().center();
        cx.simulate_click(at, gpui::Modifiers::none());
        cx.run_until_parked();
        assert_eq!(
            *starts.borrow(),
            [Some(Orchestrate {
                approve_plan: false,
                same_agents: true,
                plan_doc: false,
            })]
        );
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

    #[gpui::test]
    fn the_reads_field_saves_its_chats_and_holds_the_start_without_read_thread(
        cx: &mut gpui::TestAppContext,
    ) {
        let _dir = init(cx);
        let state = cx.new(|_| AppState::new());
        state.update(cx, |state, _| {
            state.local_device_id = Some("mac".into());
            state.tasks = vec![task("plain", false)];
            state.chats = vec![chat("review"), chat("redirect")];
        });
        let (card, cx) = cx.add_window_view(|_, cx| TaskCard::new(state, cx));
        let events = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        let sink = events.clone();
        cx.update(|_, cx| {
            cx.subscribe(&card, move |_, event: &TaskCardEvent, _| match event {
                TaskCardEvent::Update { patch, .. } => {
                    if let Some(context) = &patch.context {
                        sink.borrow_mut().push(format!("save {context:?}"));
                    }
                }
                TaskCardEvent::Start { .. } => sink.borrow_mut().push("start".into()),
                _ => {}
            })
            .detach();
        });
        card.update(cx, |card, cx| {
            card.open("plain".into(), None, CardFocus::Title, None, cx)
        });
        cx.run_until_parked();
        let click = |cx: &mut gpui::VisualTestContext, selector: &'static str| {
            let at = cx.debug_bounds(selector).unwrap().center();
            cx.simulate_click(at, gpui::Modifiers::none());
            cx.run_until_parked();
        };
        click(cx, "task-card-hand-off-mode-2");
        for query in ["@rev", "@red"] {
            card.update(cx, |card, cx| {
                let input = card.open.as_ref().unwrap().reads.clone();
                input.update(cx, |input, cx| input.set_text(query, cx));
            });
            cx.run_until_parked();
            click(cx, "task-card-reads-match-0");
        }
        assert!(cx.debug_bounds("task-context-review").is_some());
        assert!(cx.debug_bounds("task-context-redirect").is_some());
        card.update_in(cx, |card, window, cx| {
            let reads = card.open.as_ref().unwrap().reads.focus_handle(cx);
            window.focus(&reads, cx);
        });
        cx.simulate_keystrokes("backspace");
        cx.run_until_parked();
        assert!(cx.debug_bounds("task-context-redirect").is_none());
        assert_eq!(
            *events.borrow(),
            [
                r#"save ["review"]"#,
                r#"save ["review", "redirect"]"#,
                r#"save ["review"]"#
            ]
        );

        card.update(cx, |card, cx| {
            let mut settings = ToolSettings::default();
            settings.set_tool("read_thread", false);
            card.tool_settings = Some(settings);
            cx.notify();
        });
        cx.run_until_parked();
        assert!(cx.debug_bounds("task-card-reads-off").is_some());
        click(cx, "task-card-orchestrate");
        assert_eq!(
            events.borrow().len(),
            3,
            "a lead that cannot read them does not start"
        );
        click(cx, "task-card-reads-remove-review");
        click(cx, "task-card-orchestrate");
        assert_eq!(events.borrow()[3..], ["save []", "start"]);
    }
}
