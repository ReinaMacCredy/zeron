use std::collections::{HashMap, HashSet};
use std::time::Instant;

use chrono::{DateTime, Utc};
use gpui::{
    AnyElement, App, ClickEvent, Context, CursorStyle, DragMoveEvent, Edges, Entity,
    EntityInputHandler, EventEmitter, FocusHandle, Focusable, Hsla, IntoElement, KeyDownEvent,
    MouseButton, MouseDownEvent, Pixels, Point, Render, ScrollHandle, SharedString, Subscription,
    Window, div, prelude::*, px, relative,
};
use zeron_proto::{
    ChatIndicator, DelegationStatus, PlanProgress, StepOutcome, StepStatus, StepTurnEnd, TaskPatch,
    TaskStatus, TaskStepPatch,
};

use super::images::{self, ImageGallery};
use super::morph::{self, Key, Painted};
use super::{Rollup, StepRow, chat_brand, column_tone, linked_chat, rollup, step_chat};
use crate::attachments;
use crate::composer::{Composer, ComposerInput, ComposerInputEvent, Submit};
use crate::icons::{self, icon};
use crate::motion::{self, MotionSpec, Tween};
use crate::popover;
use crate::settings::widgets;
use crate::state::AppState;
use crate::theme::Theme;
use crate::transcript::Transcript;
use crate::typography::ui_rems;

const ICON_SLOT: f32 = 16.0;
const STEP_IN: MotionSpec = MotionSpec::new(350, motion::EASE_DRAWER);
const STEP_RISE: f32 = 6.0;
const PROGRESS_FILL: MotionSpec = MotionSpec::new(500, motion::EASE_DRAWER);
const ROW_PAD_LEFT: f32 = 16.0;
const ROW_SHIFT: MotionSpec = MotionSpec::new(200, motion::EASE_DRAWER);
const EDIT_GROW: MotionSpec = MotionSpec::new(280, motion::EASE_DRAWER);

pub enum TaskCenterEvent {
    Update {
        task_id: String,
        patch: TaskPatch,
    },
    ApprovePlan(String),
    RetryStep(String),
    UpdateStep {
        step_id: String,
        patch: TaskStepPatch,
    },
    EditPlan {
        task_id: String,
        edit: PlanEdit,
        approve: bool,
    },
    OpenChat(String),
    Selected,
    Back,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanEdit {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub overview: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub overview_images: Option<Vec<String>>,
    pub steps: Vec<PlanStepEdit>,
    pub remove_step_ids: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub order: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanStepEdit {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub step_id: Option<String>,
    pub title: String,
    pub notes: String,
    pub images: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Selection {
    Lead,
    Step(String),
}

pub struct LiveThread {
    pub chat_id: String,
    pub transcript: Entity<Transcript>,
    pub composer: Entity<Composer>,
}

#[derive(Debug, Clone, PartialEq)]
struct StepText {
    title: String,
    notes: String,
    images: Vec<String>,
}

#[derive(Default)]
struct Draft {
    overview: Option<StepText>,
    edited: HashMap<String, StepText>,
    added: Vec<(String, StepText)>,
    removed: Vec<String>,
    order: Option<Vec<String>>,
    moved: HashSet<String>,
}

impl Draft {
    fn count(&self) -> usize {
        usize::from(self.overview.is_some())
            + self.edited.len()
            + self.added.len()
            + self.removed.len()
            + self.moved.len()
    }

    fn retain_live(&mut self, plan: &Rollup) {
        let row = |id: &str| plan.steps.iter().find(|row| row.step.id == id);
        self.edited.retain(|id, _| row(id).is_some());
        self.removed
            .retain(|id| row(id).is_some_and(|row| row.status == StepStatus::Planned));
        self.moved.retain(|id| row(id).is_some());
        if let Some(order) = &mut self.order {
            order.retain(|id| row(id).is_some());
            for row in &plan.steps {
                if !order.contains(&row.step.id) {
                    order.push(row.step.id.clone());
                }
            }
        }
        let started = self
            .moved
            .iter()
            .any(|id| row(id).is_some_and(|row| row.status != StepStatus::Planned));
        let unchanged = self
            .order
            .as_ref()
            .is_none_or(|order| order.iter().eq(plan.steps.iter().map(|row| &row.step.id)));
        if started || unchanged || self.moved.is_empty() {
            self.order = None;
            self.moved.clear();
        }
    }

    fn edit(&self, plan: &Rollup) -> PlanEdit {
        let mut steps: Vec<PlanStepEdit> = plan
            .steps
            .iter()
            .filter_map(|row| {
                let text = self.edited.get(&row.step.id)?;
                Some(PlanStepEdit {
                    step_id: Some(row.step.id.clone()),
                    title: text.title.clone(),
                    notes: text.notes.clone(),
                    images: text.images.clone(),
                })
            })
            .collect();
        steps.extend(self.added.iter().map(|(_, text)| PlanStepEdit {
            step_id: None,
            title: text.title.clone(),
            notes: text.notes.clone(),
            images: text.images.clone(),
        }));
        PlanEdit {
            overview: self.overview.as_ref().map(|text| text.notes.clone()),
            overview_images: self.overview.as_ref().map(|text| text.images.clone()),
            steps,
            remove_step_ids: plan
                .steps
                .iter()
                .filter(|row| self.removed.contains(&row.step.id))
                .map(|row| row.step.id.clone())
                .collect(),
            order: self
                .order
                .iter()
                .flatten()
                .filter(|id| !self.removed.contains(id))
                .cloned()
                .collect(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Slot {
    Started,
    Planned,
    Removed,
    Added,
}

fn arrange(ids: &[&str], order: Option<&[String]>) -> Vec<usize> {
    let mut out: Vec<usize> = order
        .unwrap_or_default()
        .iter()
        .filter_map(|id| ids.iter().position(|row| row == id))
        .collect();
    let rest: Vec<usize> = (0..ids.len()).filter(|ix| !out.contains(ix)).collect();
    out.extend(rest);
    out
}

fn move_range(slots: &[Slot], from: usize) -> Option<(usize, usize)> {
    match slots.get(from)? {
        Slot::Planned => {
            let floor = slots
                .iter()
                .rposition(|slot| *slot == Slot::Started)
                .map_or(0, |ix| ix + 1);
            let lo = if from < floor { floor - 1 } else { floor };
            let hi = slots.iter().rposition(|slot| *slot != Slot::Added)?;
            Some((lo, hi))
        }
        Slot::Added => Some((
            slots.iter().position(|slot| *slot == Slot::Added)?,
            slots.len() - 1,
        )),
        Slot::Started | Slot::Removed => None,
    }
}

enum Place {
    Up,
    Down,
    Beside { anchor: String, after: bool },
}

fn landing(ids: &[&str], from: usize, place: &Place) -> Option<usize> {
    match place {
        Place::Up => from.checked_sub(1),
        Place::Down => Some(from + 1).filter(|to| *to < ids.len()),
        Place::Beside { anchor, after } => {
            let at = ids
                .iter()
                .enumerate()
                .filter(|(ix, _)| *ix != from)
                .position(|(_, id)| id == anchor)?;
            Some(at + usize::from(*after))
        }
    }
}

fn drop_index(rows: &[(f32, f32)], from: usize, (lo, hi): (usize, usize), dy: f32) -> usize {
    let (top, height) = rows[from];
    let mid = top + height / 2.0 + dy;
    let center = |k: usize| rows[k].0 + rows[k].1 / 2.0;
    if let Some(k) = (lo..from).find(|&k| mid < center(k)) {
        return k;
    }
    (from + 1..=hi)
        .filter(|&k| mid > center(k))
        .last()
        .filter(|&k| k >= lo)
        .unwrap_or(from)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tag {
    Edited,
    New,
    Removed,
    Moved,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum EditTarget {
    Overview,
    Step(String),
}

struct Editor {
    target: EditTarget,
    title: Option<Entity<ComposerInput>>,
    body: Entity<ComposerInput>,
    images: Vec<String>,
    started: bool,
    from: f32,
    grow: Tween,
    uploads: Vec<gpui::Task<()>>,
    _subscriptions: Vec<Subscription>,
}

pub struct StepDragPayload {
    task_id: String,
    step_id: String,
}

struct StepDragGhost;

impl Render for StepDragGhost {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        gpui::Empty
    }
}

struct StepDrag {
    id: String,
    ids: Vec<String>,
    from: usize,
    range: (usize, usize),
    rows: Vec<(f32, f32)>,
    origin: f32,
    start: f32,
    dy: f32,
    over: usize,
    shifts: HashMap<usize, Tween>,
}

#[derive(Clone)]
enum Outcome {
    Skip,
    Reopen,
    Locked,
}

#[derive(Clone)]
struct StepActions {
    step_id: String,
    edit: bool,
    thread: Option<String>,
    up: bool,
    down: bool,
    outcome: Outcome,
    remove: bool,
}

struct StepMenu {
    position: Point<Pixels>,
    actions: StepActions,
}

#[derive(Clone, Copy)]
enum MenuAction {
    Edit,
    Open,
    AddImage,
    Up,
    Down,
    Outcome,
    Remove,
}

struct Thread {
    title: String,
    delegation: Option<(DelegationStatus, Option<String>)>,
    preview: Option<String>,
    since: Option<DateTime<Utc>>,
}

enum StepState {
    Working(Option<DateTime<Utc>>),
    Asked,
    Failed(&'static str, Option<String>),
    NeedsYou,
    Done,
    Planned,
    Skipped,
}

pub struct TaskCenter {
    state: Entity<AppState>,
    task_id: String,
    selected: Selection,
    draft: Draft,
    sent: Option<Draft>,
    expanded: HashSet<String>,
    editor: Option<Editor>,
    add: Entity<ComposerInput>,
    live: Option<LiveThread>,
    fill: Option<Tween>,
    arrivals: Option<HashMap<String, Tween>>,
    painted: Painted,
    scroll: ScrollHandle,
    menu: popover::Popup<StepMenu>,
    drag: Option<StepDrag>,
    press: f32,
    settle: Option<(String, f32, Tween)>,
    gallery: Entity<ImageGallery>,
    picking: Option<gpui::Task<()>>,
    focus: FocusHandle,
    shimmer_since: Instant,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<TaskCenterEvent> for TaskCenter {}

impl Focusable for TaskCenter {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl TaskCenter {
    pub fn new(state: Entity<AppState>, task_id: String, cx: &mut Context<Self>) -> Self {
        let add = cx.new(|cx| {
            ComposerInput::new("Add a step", cx)
                .with_single_line()
                .with_max_chars(zeron_doc::TASK_TITLE_MAX_CHARS)
        });
        let add_events = cx.subscribe(&add, |center, _, event: &ComposerInputEvent, cx| {
            if matches!(event, ComposerInputEvent::Submitted) {
                center.add_step(cx);
            }
        });
        let state_changes = cx.observe(&state, |_, _, cx| cx.notify());
        let gallery = cx.new(|cx| ImageGallery::new(state.clone(), cx));
        Self {
            gallery,
            picking: None,
            state,
            task_id,
            selected: Selection::Lead,
            draft: Draft::default(),
            sent: None,
            expanded: HashSet::new(),
            editor: None,
            add,
            live: None,
            fill: None,
            arrivals: None,
            painted: Painted::default(),
            scroll: ScrollHandle::new(),
            menu: popover::Popup::default(),
            drag: None,
            press: 0.0,
            settle: None,
            focus: cx.focus_handle(),
            shimmer_since: Instant::now(),
            _subscriptions: vec![add_events, state_changes],
        }
    }

    pub fn task_id(&self) -> &str {
        &self.task_id
    }

    pub fn select(&mut self, selection: Selection, cx: &mut Context<Self>) {
        if self.selected != selection {
            self.selected = selection;
            cx.emit(TaskCenterEvent::Selected);
            cx.notify();
        }
    }

    pub fn live_target(&self, cx: &App) -> Option<String> {
        let state = self.state.read(cx);
        let task = state
            .tasks
            .iter()
            .find(|task| task.id == self.task_id && !task.deleted)?;
        if let Selection::Step(id) = &self.selected
            && let Some(chat) = state
                .task_steps
                .iter()
                .find(|step| &step.id == id && !step.deleted)
                .and_then(|step| step_chat(&state.chats, step))
        {
            return Some(chat.id.clone());
        }
        linked_chat(&state.chats, task).map(|chat| chat.id.clone())
    }

    pub fn live_chat_id(&self) -> Option<&str> {
        self.live.as_ref().map(|live| live.chat_id.as_str())
    }

    pub fn set_live(&mut self, live: Option<LiveThread>, cx: &mut Context<Self>) {
        self.live = live;
        cx.notify();
    }

    fn add_step(&mut self, cx: &mut Context<Self>) {
        let title = self.add.read(cx).text().trim().to_string();
        if title.is_empty() {
            return;
        }
        self.add.update(cx, |input, cx| input.set_text("", cx));
        self.draft.added.push((
            uuid::Uuid::new_v4().to_string(),
            StepText {
                title,
                notes: String::new(),
                images: Vec::new(),
            },
        ));
        cx.notify();
    }

    fn task_and_plan(&self, cx: &App) -> Option<(zeron_proto::Task, Rollup)> {
        let state = self.state.read(cx);
        state
            .tasks
            .iter()
            .find(|task| task.id == self.task_id && !task.deleted)
            .map(|task| (task.clone(), rollup(state, task, Utc::now())))
    }

    fn shown_step(&self, step: &zeron_proto::TaskStep) -> StepText {
        self.draft
            .edited
            .get(&step.id)
            .cloned()
            .unwrap_or_else(|| StepText {
                title: step.title.clone(),
                notes: step.notes.clone(),
                images: step.images.clone(),
            })
    }

    fn shown_text(&self, target: &EditTarget, cx: &App) -> Option<StepText> {
        let (task, plan) = self.task_and_plan(cx)?;
        match target {
            EditTarget::Overview => Some(self.draft.overview.clone().unwrap_or(StepText {
                title: String::new(),
                notes: task.plan,
                images: task.images,
            })),
            EditTarget::Step(id) => self
                .draft
                .added
                .iter()
                .find(|(added, _)| added == id)
                .map(|(_, text)| text.clone())
                .or_else(|| {
                    plan.steps
                        .iter()
                        .find(|row| &row.step.id == id)
                        .map(|row| self.shown_step(&row.step))
                }),
        }
    }

    fn start_edit(&mut self, target: EditTarget, window: &mut Window, cx: &mut Context<Self>) {
        self.commit_edit(cx);
        let Some(text) = self.shown_text(&target, cx) else {
            return;
        };
        let title = matches!(target, EditTarget::Step(_)).then(|| {
            cx.new(|cx| {
                let mut input = ComposerInput::new("Step title", cx)
                    .with_single_line()
                    .with_max_chars(zeron_doc::TASK_TITLE_MAX_CHARS);
                input.set_text(text.title.clone(), cx);
                input
            })
        });
        let placeholder = match target {
            EditTarget::Overview => "The goal, the approach, and how the result will be checked",
            EditTarget::Step(_) => "What this step builds, where, and how to tell it is done",
        };
        let body = cx.new(|cx| {
            let mut input =
                ComposerInput::new(placeholder, cx).with_max_chars(zeron_doc::TASK_NOTES_MAX_CHARS);
            input.set_text(text.notes, cx);
            input
        });
        let mut subscriptions: Vec<Subscription> = title
            .iter()
            .map(|title| {
                cx.subscribe(title, |center, _, event: &ComposerInputEvent, cx| {
                    if matches!(event, ComposerInputEvent::Submitted) {
                        center.commit_edit(cx);
                    } else if let Some(staged) = images::pasted(event, cx) {
                        center.add_images(staged, cx);
                    }
                })
            })
            .collect();
        subscriptions.push(
            cx.subscribe(&body, |center, _, event: &ComposerInputEvent, cx| {
                if let Some(staged) = images::pasted(event, cx) {
                    center.add_images(staged, cx);
                }
            }),
        );
        let started = match &target {
            EditTarget::Overview => false,
            EditTarget::Step(id) => self.task_and_plan(cx).is_some_and(|(_, plan)| {
                plan.steps
                    .iter()
                    .any(|row| &row.step.id == id && row.step.chat_id.is_some())
            }),
        };
        let focus = title.as_ref().unwrap_or(&body).focus_handle(cx);
        window.focus(&focus, cx);
        if let EditTarget::Step(id) = &target {
            self.expanded.insert(id.clone());
        }
        let key = match &target {
            EditTarget::Overview => Key::Overview,
            EditTarget::Step(id) => Key::Task(id.clone()),
        };
        let from = {
            let mut painted = self.painted.borrow_mut();
            painted.remove(&Key::Editor);
            painted
                .get(&key)
                .map_or(0.0, |rect| f32::from(rect.size.height))
        };
        let now = Instant::now();
        let mut grow = Tween::at(0.0, now);
        grow.retarget(1.0, EDIT_GROW, motion::reduced_motion(cx), now);
        self.editor = Some(Editor {
            target,
            title,
            body,
            images: text.images,
            started,
            from,
            grow,
            uploads: Vec::new(),
            _subscriptions: subscriptions,
        });
        cx.notify();
    }

    fn add_images(&mut self, staged: Vec<attachments::StagedAttachment>, cx: &mut Context<Self>) {
        let Some(editor) = self.editor.as_mut() else {
            return;
        };
        if staged.is_empty() {
            return;
        }
        let target = editor.target.clone();
        let upload = images::upload(&self.state, staged, cx);
        editor.uploads.push(cx.spawn(async move |center, cx| {
            let paths = upload.await;
            center
                .update(cx, |center, cx| {
                    let Some(editor) = center
                        .editor
                        .as_mut()
                        .filter(|editor| editor.target == target)
                    else {
                        return;
                    };
                    let room = zeron_doc::TASK_IMAGES_MAX.saturating_sub(editor.images.len());
                    if paths.len() > room {
                        crate::toast::error(cx, "A step or overview holds up to 12 images.");
                    }
                    editor.images.extend(paths.into_iter().take(room));
                    cx.notify();
                })
                .ok();
        }));
    }

    fn pick_images(&mut self, step_id: Option<String>, cx: &mut Context<Self>) {
        let picked = images::pick(cx);
        self.picking = Some(cx.spawn(async move |center, cx| {
            let paths = picked.await;
            center
                .update(cx, |center, cx| {
                    let staged = images::staged_files(&paths, cx);
                    match step_id {
                        None => center.add_images(staged, cx),
                        Some(id) => center.add_to_spec(id, staged, cx),
                    }
                })
                .ok();
        }));
    }

    fn add_to_spec(
        &mut self,
        step_id: String,
        staged: Vec<attachments::StagedAttachment>,
        cx: &mut Context<Self>,
    ) {
        if staged.is_empty() {
            return;
        }
        let upload = images::upload(&self.state, staged, cx);
        self.picking = Some(cx.spawn(async move |center, cx| {
            let paths = upload.await;
            center
                .update(cx, |center, cx| {
                    if paths.is_empty() {
                        return;
                    }
                    let target = EditTarget::Step(step_id.clone());
                    let Some(mut text) = center.shown_text(&target, cx) else {
                        return;
                    };
                    let room = zeron_doc::TASK_IMAGES_MAX.saturating_sub(text.images.len());
                    if paths.len() > room {
                        crate::toast::error(cx, "A step holds up to 12 images.");
                    }
                    text.images.extend(paths.into_iter().take(room));
                    if let Some((_, added)) =
                        center.draft.added.iter_mut().find(|(id, _)| *id == step_id)
                    {
                        *added = text;
                    } else {
                        center.draft.edited.insert(step_id.clone(), text);
                    }
                    center.expanded.insert(step_id);
                    cx.notify();
                })
                .ok();
        }));
    }

    fn remove_image(&mut self, index: usize, cx: &mut Context<Self>) {
        if let Some(editor) = self.editor.as_mut()
            && index < editor.images.len()
        {
            editor.images.remove(index);
            cx.notify();
        }
    }

    fn open_images(&mut self, paths: Vec<String>, index: usize, cx: &mut Context<Self>) {
        self.gallery
            .update(cx, |gallery, cx| gallery.open(paths, index, cx));
    }

    fn commit_edit(&mut self, cx: &mut Context<Self>) {
        let Some(editor) = self.editor.take() else {
            return;
        };
        let notes = editor.body.read(cx).text().to_string();
        let title = editor
            .title
            .map(|input| input.read(cx).text().trim().to_string())
            .filter(|title| !title.is_empty());
        let images = editor.images;
        let saved = self.task_and_plan(cx);
        match editor.target {
            EditTarget::Overview => {
                let unchanged =
                    saved.is_some_and(|(task, _)| task.plan == notes && task.images == images);
                self.draft.overview = (!unchanged).then_some(StepText {
                    title: String::new(),
                    notes,
                    images,
                });
            }
            EditTarget::Step(id) => {
                if let Some((_, text)) = self.draft.added.iter_mut().find(|(added, _)| *added == id)
                {
                    text.title = title.unwrap_or(std::mem::take(&mut text.title));
                    text.notes = notes;
                    text.images = images;
                } else if let Some(step) = saved.and_then(|(_, plan)| {
                    plan.steps
                        .into_iter()
                        .find(|row| row.step.id == id)
                        .map(|row| row.step)
                }) {
                    let text = StepText {
                        title: title.unwrap_or_else(|| self.shown_step(&step).title),
                        notes,
                        images,
                    };
                    if text.title == step.title
                        && text.notes == step.notes
                        && text.images == step.images
                    {
                        self.draft.edited.remove(&id);
                    } else {
                        self.draft.edited.insert(id, text);
                    }
                }
            }
        }
        cx.notify();
    }

    fn cancel_edit(&mut self, cx: &mut Context<Self>) {
        self.editor = None;
        cx.notify();
    }

    fn toggle_remove(&mut self, step_id: &str, cx: &mut Context<Self>) {
        if self
            .editor
            .as_ref()
            .is_some_and(|editor| editor.target == EditTarget::Step(step_id.to_string()))
        {
            self.editor = None;
        }
        let added = self.draft.added.len();
        self.draft.added.retain(|(id, _)| id != step_id);
        if self.draft.added.len() == added {
            if let Some(ix) = self.draft.removed.iter().position(|id| id == step_id) {
                self.draft.removed.remove(ix);
            } else {
                self.draft.edited.remove(step_id);
                self.draft.removed.push(step_id.to_string());
            }
        }
        cx.notify();
    }

    fn toggle_spec(&mut self, step_id: &str, cx: &mut Context<Self>) {
        if !self.expanded.remove(step_id) {
            self.expanded.insert(step_id.to_string());
        }
        cx.notify();
    }

    fn undo_edits(&mut self, cx: &mut Context<Self>) {
        self.draft = Draft::default();
        self.editor = None;
        cx.notify();
    }

    fn send_edits(&mut self, approve: bool, cx: &mut Context<Self>) {
        self.commit_edit(cx);
        let Some((_, plan)) = self.task_and_plan(cx) else {
            return;
        };
        self.draft.retain_live(&plan);
        if self.draft.count() == 0 {
            if approve {
                cx.emit(TaskCenterEvent::ApprovePlan(self.task_id.clone()));
            }
            return;
        }
        let edit = self.draft.edit(&plan);
        self.sent = Some(std::mem::take(&mut self.draft));
        cx.emit(TaskCenterEvent::EditPlan {
            task_id: self.task_id.clone(),
            edit,
            approve,
        });
        cx.notify();
    }

    /// The edits come back as unsent, so a refused send loses nothing the person wrote.
    pub fn edit_failed(&mut self, cx: &mut Context<Self>) {
        if let Some(sent) = self.sent.take()
            && self.draft.count() == 0
        {
            self.draft = sent;
        }
        cx.notify();
    }

    pub fn edit_sent(&mut self) {
        self.sent = None;
    }

    fn set_outcome(&mut self, step_id: &str, outcome: Option<StepOutcome>, cx: &mut Context<Self>) {
        cx.emit(TaskCenterEvent::UpdateStep {
            step_id: step_id.to_string(),
            patch: TaskStepPatch {
                outcome: Some(outcome),
                ..Default::default()
            },
        });
    }

    fn shown(
        &self,
        task: &zeron_proto::Task,
        plan: &Rollup,
        cx: &App,
    ) -> Vec<(StepRow, Option<Tag>, Option<Thread>)> {
        let mut threads = self.threads(plan, cx);
        let ids: Vec<&str> = plan.steps.iter().map(|row| row.step.id.as_str()).collect();
        let mut shown: Vec<(StepRow, Option<Tag>, Option<Thread>)> =
            arrange(&ids, self.draft.order.as_deref())
                .into_iter()
                .map(|ix| {
                    let row = &plan.steps[ix];
                    let text = self.shown_step(&row.step);
                    let tag = if self.draft.removed.contains(&row.step.id) {
                        Some(Tag::Removed)
                    } else {
                        self.draft
                            .edited
                            .contains_key(&row.step.id)
                            .then_some(Tag::Edited)
                    };
                    let mut step = row.step.clone();
                    step.title = text.title;
                    step.notes = text.notes;
                    step.images = text.images;
                    (
                        StepRow {
                            step,
                            status: row.status,
                            indicator: row.indicator,
                            brand: row.brand,
                        },
                        tag,
                        threads[ix].take(),
                    )
                })
                .collect();
        for (id, text) in &self.draft.added {
            let step = zeron_proto::TaskStep {
                id: id.clone(),
                task_id: task.id.clone(),
                title: text.title.clone(),
                notes: text.notes.clone(),
                order_key: String::new(),
                chat_id: None,
                outcome: None,
                created_at: Default::default(),
                updated_at: Default::default(),
                deleted: false,
                turn: None,
                images: text.images.clone(),
            };
            shown.push((
                StepRow {
                    step,
                    status: StepStatus::Planned,
                    indicator: None,
                    brand: None,
                },
                Some(Tag::New),
                None,
            ));
        }
        shown
    }

    fn slots(shown: &[(StepRow, Option<Tag>, Option<Thread>)]) -> Vec<Slot> {
        shown
            .iter()
            .map(|(row, tag, _)| match tag {
                Some(Tag::New) => Slot::Added,
                Some(Tag::Removed) => Slot::Removed,
                _ if row.status == StepStatus::Planned => Slot::Planned,
                _ => Slot::Started,
            })
            .collect()
    }

    fn move_step(&mut self, step_id: &str, place: Place, cx: &mut Context<Self>) {
        let Some((task, plan)) = self.task_and_plan(cx) else {
            return;
        };
        let shown = self.shown(&task, &plan, cx);
        let ids: Vec<&str> = shown.iter().map(|(row, ..)| row.step.id.as_str()).collect();
        let slots = Self::slots(&shown);
        let Some(from) = ids.iter().position(|id| *id == step_id) else {
            return;
        };
        let Some(to) = landing(&ids, from, &place) else {
            return;
        };
        let Some((lo, hi)) = move_range(&slots, from) else {
            return;
        };
        if to == from || to < lo || to > hi {
            return;
        }
        let persisted = slots.iter().filter(|slot| **slot != Slot::Added).count();
        if from >= persisted {
            let row = self.draft.added.remove(from - persisted);
            self.draft.added.insert(to - persisted, row);
        } else {
            let mut order: Vec<String> = shown[..persisted]
                .iter()
                .map(|(row, ..)| row.step.id.clone())
                .collect();
            let id = order.remove(from);
            order.insert(to, id.clone());
            self.draft.moved.insert(id);
            self.draft.order = Some(order);
            self.draft.retain_live(&plan);
        }
        cx.notify();
    }

    fn begin_drag(&self, step_id: &str, cx: &App) -> Option<StepDrag> {
        let (task, plan) = self.task_and_plan(cx)?;
        let shown = self.shown(&task, &plan, cx);
        let from = shown.iter().position(|(row, ..)| row.step.id == step_id)?;
        let range = move_range(&Self::slots(&shown), from)?;
        let painted = self.painted.borrow();
        let rows = shown
            .iter()
            .map(|(row, ..)| {
                let rect = painted.get(&Key::Task(row.step.id.clone()))?;
                Some((f32::from(rect.top()), f32::from(rect.size.height)))
            })
            .collect::<Option<Vec<_>>>()?;
        Some(StepDrag {
            id: step_id.to_string(),
            ids: shown.iter().map(|(row, ..)| row.step.id.clone()).collect(),
            from,
            range,
            rows,
            origin: f32::from(painted.get(&Key::Steps)?.top()),
            start: self.press,
            dy: 0.0,
            over: from,
            shifts: HashMap::new(),
        })
    }

    fn drag_to(&mut self, step_id: &str, y: f32, cx: &mut Context<Self>) {
        if self.drag.as_ref().is_none_or(|drag| drag.id != step_id) {
            self.drag = self.begin_drag(step_id, cx);
        }
        let reduced = motion::reduced_motion(cx);
        let Some(drag) = &mut self.drag else {
            return;
        };
        let now = Instant::now();
        let (lo, hi) = drag.range;
        let (top, height) = drag.rows[drag.from];
        let first = drag.rows[lo.min(drag.from)];
        let last = drag.rows[hi.max(drag.from)];
        drag.dy = (y - drag.start).clamp(first.0 - top - 6.0, last.0 + last.1 - top - height + 6.0);
        let over = drop_index(&drag.rows, drag.from, drag.range, drag.dy);
        if over != drag.over {
            drag.over = over;
            for k in (0..drag.rows.len()).filter(|k| *k != drag.from) {
                let target = if drag.from < k && k <= over {
                    -1.0
                } else if over <= k && k < drag.from {
                    1.0
                } else {
                    0.0
                };
                let tween = drag.shifts.entry(k).or_insert_with(|| Tween::at(0.0, now));
                if tween.target() != target {
                    tween.retarget(target, ROW_SHIFT, reduced, now);
                }
            }
        }
        cx.notify();
    }

    fn drop_step(&mut self, cx: &mut Context<Self>) {
        let Some(drag) = self.drag.take() else {
            return;
        };
        let (top, height) = drag.rows[drag.from];
        let (over_top, over_height) = drag.rows[drag.over];
        let slot = match drag.over.cmp(&drag.from) {
            std::cmp::Ordering::Less => over_top,
            std::cmp::Ordering::Greater => over_top + over_height - height,
            std::cmp::Ordering::Equal => top,
        };
        if drag.over != drag.from {
            let place = Place::Beside {
                anchor: drag.ids[drag.over].clone(),
                after: drag.over > drag.from,
            };
            self.move_step(&drag.id, place, cx);
        }
        self.settle(drag.id, top + drag.dy - slot, cx);
    }

    fn settle(&mut self, step_id: String, offset: f32, cx: &mut Context<Self>) {
        let now = Instant::now();
        let mut tween = Tween::at(1.0, now);
        tween.retarget(0.0, ROW_SHIFT, motion::reduced_motion(cx), now);
        self.settle = Some((step_id, offset, tween));
        cx.notify();
    }

    fn open_menu(
        &mut self,
        actions: StepActions,
        position: Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        window.focus(&self.focus, cx);
        self.menu.open(StepMenu { position, actions });
        cx.notify();
    }

    fn close_menu(&mut self, cx: &mut Context<Self>) {
        if self.menu.begin_close() {
            popover::reap_popup(cx, |center: &mut Self| &mut center.menu);
            cx.notify();
        }
    }

    fn menu_action(&mut self, action: MenuAction, window: &mut Window, cx: &mut Context<Self>) {
        let Some(actions) = self.menu.as_open().map(|menu| menu.actions.clone()) else {
            return;
        };
        self.close_menu(cx);
        let id = actions.step_id;
        match action {
            MenuAction::Edit => self.start_edit(EditTarget::Step(id), window, cx),
            MenuAction::AddImage => self.pick_images(Some(id), cx),
            MenuAction::Open => {
                if let Some(chat) = actions.thread {
                    cx.emit(TaskCenterEvent::OpenChat(chat));
                }
            }
            MenuAction::Up => self.move_step(&id, Place::Up, cx),
            MenuAction::Down => self.move_step(&id, Place::Down, cx),
            MenuAction::Outcome => match actions.outcome {
                Outcome::Skip => self.set_outcome(&id, Some(StepOutcome::Skipped), cx),
                Outcome::Reopen => self.set_outcome(&id, None, cx),
                Outcome::Locked => {}
            },
            MenuAction::Remove => self.toggle_remove(&id, cx),
        }
    }

    fn threads(&self, plan: &Rollup, cx: &App) -> Vec<Option<Thread>> {
        let state = self.state.read(cx);
        plan.steps
            .iter()
            .map(|row| {
                let chat = step_chat(&state.chats, &row.step)?;
                let delegation = chat.delegation.as_ref();
                Some(Thread {
                    title: crate::transcript::single_line(&crate::home::chat_title(chat)),
                    delegation: delegation.map(|d| (d.status, d.summary.clone())),
                    preview: chat.last_message_preview.clone(),
                    since: state
                        .session_for(&chat.id)
                        .and_then(|session| session.started_at)
                        .or(delegation.map(|d| d.started_at)),
                })
            })
            .collect()
    }

    fn step_state(row: &StepRow, thread: Option<&Thread>) -> StepState {
        match row.status {
            StepStatus::Done => StepState::Done,
            StepStatus::Skipped => StepState::Skipped,
            StepStatus::Planned => StepState::Planned,
            StepStatus::Working => StepState::Working(thread.and_then(|t| t.since)),
            StepStatus::Failed => {
                StepState::Failed("Failed", thread.and_then(|t| t.preview.clone()))
            }
            StepStatus::NeedsYou => {
                let delegation = thread.and_then(|t| t.delegation.as_ref());
                let stopped = delegation.is_none()
                    && row.step.turn.as_ref().map(|turn| turn.ended) == Some(StepTurnEnd::Stopped);
                if row.indicator == Some(ChatIndicator::AwaitingInput) {
                    StepState::Asked
                } else if let Some((status, summary)) =
                    delegation.filter(|(status, _)| retryable(*status))
                {
                    let label = match status {
                        DelegationStatus::Cancelled => "Stopped",
                        DelegationStatus::Interrupted => "Interrupted",
                        _ => "Failed",
                    };
                    StepState::Failed(label, summary.clone())
                } else if row.indicator == Some(ChatIndicator::Errored) || stopped {
                    let label = if stopped { "Stopped" } else { "Failed" };
                    StepState::Failed(label, thread.and_then(|t| t.preview.clone()))
                } else {
                    StepState::NeedsYou
                }
            }
        }
    }

    fn render_missing(&self, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
        div()
            .id("task-center-missing")
            .debug_selector(|| "task-center-missing".into())
            .size_full()
            .pt(px(Theme::TITLEBAR_HEIGHT))
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap(px(12.0))
            .text_size(ui_rems(13.0))
            .text_color(theme.text_muted)
            .child("This task is gone. It was deleted, or it has not synced yet.")
            .child(
                widgets::text_action(theme, widgets::ActionTone::Filled, "Back to Tasks")
                    .id("task-center-missing-back")
                    .debug_selector(|| "task-center-missing-back".into())
                    .role(gpui::Role::Button)
                    .rounded_full()
                    .on_click(
                        cx.listener(|_, _: &ClickEvent, _, cx| cx.emit(TaskCenterEvent::Back)),
                    ),
            )
            .into_any_element()
    }

    fn render_header(
        &self,
        task: &zeron_proto::Task,
        plan: &Rollup,
        fill: f32,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let (label, tone) = plan.label(task);
        let (_, _, tone) = column_tone(tone, theme);
        let (_, _, complete) = column_tone(TaskStatus::Done, theme);
        let done = task.completed_at.is_some();
        let PlanProgress {
            done: finished,
            total,
        } = plan.progress;
        let task_id = task.id.clone();
        let toggle = widgets::text_action(
            theme,
            widgets::ActionTone::Filled,
            if done { "Reopen" } else { "Mark as done" },
        )
        .id("task-center-toggle-done")
        .debug_selector(|| "task-center-toggle-done".into())
        .role(gpui::Role::Button)
        .rounded_full()
        .on_click(cx.listener(move |_, _: &ClickEvent, _, cx| {
            cx.emit(TaskCenterEvent::Update {
                task_id: task_id.clone(),
                patch: TaskPatch {
                    completed: Some(!done),
                    ..Default::default()
                },
            });
        }));
        div()
            .id("task-center-header")
            .debug_selector(|| "task-center-header".into())
            .flex_none()
            .px(px(20.0))
            .pt(px(8.0))
            .pb(px(12.0))
            .border_b_1()
            .border_color(theme.border)
            .flex()
            .flex_col()
            .gap(px(10.0))
            .child(
                div()
                    .debug_selector(|| "task-center-kicker".into())
                    .mb(px(-6.0))
                    .flex()
                    .items_center()
                    .gap(px(5.0))
                    .text_size(ui_rems(11.0))
                    .font_weight(gpui::FontWeight::MEDIUM)
                    .text_color(theme.text_muted)
                    .child(
                        icon(icons::MISSION_CONTROL)
                            .size(px(11.0))
                            .text_color(theme.text_muted),
                    )
                    .child("MISSION CONTROL"),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(10.0))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_size(ui_rems(18.0))
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .text_color(theme.text)
                            .child(SharedString::from(task.title.clone())),
                    )
                    .child(
                        div()
                            .debug_selector(|| "task-center-status".into())
                            .flex_none()
                            .flex()
                            .items_center()
                            .gap(px(6.0))
                            .text_size(ui_rems(12.0))
                            .text_color(theme.text_muted)
                            .child(div().size(px(7.0)).rounded_full().bg(tone))
                            .child(label),
                    )
                    .child(toggle),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(10.0))
                    .child(
                        div()
                            .debug_selector(|| "task-center-progress".into())
                            .flex_1()
                            .h(px(4.0))
                            .rounded(px(2.0))
                            .bg(theme.wash(0.08))
                            .overflow_hidden()
                            .child(div().h_full().w(relative(fill)).bg(complete)),
                    )
                    .child(
                        div()
                            .flex_none()
                            .text_size(ui_rems(12.0))
                            .text_color(theme.text_muted)
                            .child(format!("{finished}/{total}")),
                    ),
            )
            .into_any_element()
    }

    fn render_send_error(
        &self,
        task: &zeron_proto::Task,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let error = task.send_error.clone()?;
        let task_id = task.id.clone();
        Some(
            div()
                .flex_none()
                .px(px(20.0))
                .child(
                    widgets::warning_strip(theme, format!("Not sent to the lead: {error}"))
                        .debug_selector(|| "task-center-send-error".into())
                        .child(div().flex_1())
                        .child(
                            widgets::text_action(theme, widgets::ActionTone::Quiet, "Dismiss")
                                .id("task-center-dismiss-send-error")
                                .debug_selector(|| "task-center-dismiss-send-error".into())
                                .role(gpui::Role::Button)
                                .flex_none()
                                .min_h(px(24.0))
                                .rounded_full()
                                .on_click(cx.listener(move |_, _: &ClickEvent, _, cx| {
                                    cx.emit(TaskCenterEvent::Update {
                                        task_id: task_id.clone(),
                                        patch: TaskPatch {
                                            send_error: Some(None),
                                            ..Default::default()
                                        },
                                    });
                                })),
                        ),
                )
                .into_any_element(),
        )
    }

    pub(super) fn lead_line(task: &zeron_proto::Task, plan: &Rollup) -> String {
        let running = plan.count(StepStatus::Working);
        if task.completed_at.is_some() {
            "Done".into()
        } else if plan.lead.is_none() {
            "Not started".into()
        } else if plan.lead == Some(ChatIndicator::Working) && plan.steps.is_empty() {
            "Planning".into()
        } else if running > 0 {
            format!("Watching {running} {}", plural(running, "step", "steps"))
        } else if plan.lead == Some(ChatIndicator::Working) {
            "Working".into()
        } else {
            "Waiting on you".into()
        }
    }

    fn render_lead(
        &self,
        task: &zeron_proto::Task,
        plan: &Rollup,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let (brand, model) = {
            let state = self.state.read(cx);
            let chat = linked_chat(&state.chats, task);
            let model = chat
                .and_then(|chat| chat.config.as_ref())
                .and_then(|config| {
                    let model = config.model.as_deref()?;
                    Some(crate::pickers::model_display_label(
                        config.harness,
                        model,
                        cx,
                    ))
                });
            (chat.map(chat_brand), model)
        };
        let (mark, tint) = brand.unwrap_or((icons::BOT, None));
        let selected = self.selected == Selection::Lead;
        let lead_shimmer = self.shimmer(
            task.completed_at.is_none() && plan.lead == Some(ChatIndicator::Working),
            cx,
        );
        div()
            .id("task-center-lead")
            .debug_selector(|| "task-center-lead".into())
            .role(gpui::Role::Button)
            .aria_selected(selected)
            .flex()
            .items_center()
            .gap(px(10.0))
            .px(px(10.0))
            .py(px(9.0))
            .rounded(px(12.0))
            .cursor_pointer()
            .bg(theme.wash(if selected { 0.10 } else { 0.05 }))
            .hover(|s| s.bg(theme.wash(0.10)))
            .child(
                icon(mark)
                    .size(px(14.0))
                    .flex_none()
                    .text_color(tint.unwrap_or(theme.text_muted)),
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
                            .text_size(ui_rems(13.0))
                            .text_color(theme.text)
                            .child(match model {
                                Some(model) => format!("Lead · {model}"),
                                None => "Lead".into(),
                            }),
                    )
                    .child(
                        div()
                            .text_size(ui_rems(12.0))
                            .text_color(theme.text_muted)
                            .child(super::tree::shimmer_text(
                                Self::lead_line(task, plan),
                                lead_shimmer,
                                ui_rems(12.0),
                                theme.text_muted,
                                theme,
                            )),
                    ),
            )
            .on_click(
                cx.listener(|center, _: &ClickEvent, _, cx| center.select(Selection::Lead, cx)),
            )
            .into_any_element()
    }

    fn render_reads(
        &self,
        task: &zeron_proto::Task,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        if task.context.is_empty() {
            return None;
        }
        let state = self.state.read(cx);
        let home = linked_chat(&state.chats, task)
            .and_then(|chat| chat.space_id.as_deref())
            .or(task.space_id.as_deref());
        let now = Utc::now();
        let chips: Vec<_> = task
            .context
            .iter()
            .filter_map(|chat_id| super::card::context_chip(state, chat_id, home, theme, now))
            .collect();
        Some(
            div()
                .id("task-center-reads")
                .debug_selector(|| "task-center-reads".into())
                .flex()
                .items_start()
                .gap(px(10.0))
                .px(px(10.0))
                .child(
                    div()
                        .flex_none()
                        .pt(px(4.0))
                        .text_size(ui_rems(12.0))
                        .text_color(theme.text_muted)
                        .child("Reads"),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .flex_wrap()
                        .gap(px(5.0))
                        .children(chips),
                )
                .into_any_element(),
        )
    }

    fn render_banner(
        &self,
        task: &zeron_proto::Task,
        plan: &Rollup,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        if !task.plan_pending || task.completed_at.is_some() {
            return None;
        }
        let planning = plan.steps.is_empty() || plan.lead == Some(ChatIndicator::Working);
        let banner = div()
            .id("task-center-plan-banner")
            .debug_selector(|| "task-center-plan-banner".into())
            .flex()
            .items_center()
            .gap(px(10.0))
            .px(px(12.0))
            .py(px(10.0))
            .rounded(px(12.0))
            .bg(theme.wash(0.06))
            .text_size(ui_rems(13.0))
            .text_color(theme.text);
        if planning {
            return Some(
                banner
                    .child(
                        crate::loaders::mini_glyph_spinner(
                            "task-center-planning",
                            2.0,
                            theme.glyph,
                            cx.entity_id(),
                            cx,
                        )
                        .into_any_element(),
                    )
                    .child("The lead is planning")
                    .into_any_element(),
            );
        }
        let total = plan.progress.total;
        Some(
            banner
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .child("Plan ready")
                        .child(
                            div()
                                .text_size(ui_rems(12.0))
                                .text_color(theme.text_muted)
                                .child(format!(
                                    "{total} {} · the lead waits for you",
                                    plural(total, "step", "steps")
                                )),
                        ),
                )
                .child(
                    widgets::text_action(theme, widgets::ActionTone::Solid, "Approve and run")
                        .id("task-center-approve")
                        .debug_selector(|| "task-center-approve".into())
                        .role(gpui::Role::Button)
                        .flex_none()
                        .rounded_full()
                        .on_click(
                            cx.listener(|center, _: &ClickEvent, _, cx| {
                                center.send_edits(true, cx)
                            }),
                        ),
                )
                .into_any_element(),
        )
    }

    fn step_icon(status: StepStatus, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let slot = div()
            .flex_none()
            .size(px(ICON_SLOT))
            .flex()
            .items_center()
            .justify_center();
        match status {
            StepStatus::Working => slot
                .child(crate::loaders::mini_glyph_spinner(
                    "task-center-step-working",
                    2.0,
                    theme.glyph,
                    cx.entity_id(),
                    cx,
                ))
                .into_any_element(),
            StepStatus::NeedsYou => slot
                .child(
                    icon(icons::LUCIDE_CIRCLE_ALERT)
                        .size(px(15.0))
                        .text_color(theme.warning),
                )
                .into_any_element(),
            StepStatus::Failed => slot
                .child(
                    icon(icons::LUCIDE_CIRCLE_ALERT)
                        .size(px(15.0))
                        .text_color(theme.danger),
                )
                .into_any_element(),
            StepStatus::Done => slot
                .child(
                    div()
                        .size(px(15.0))
                        .rounded_full()
                        .bg(column_tone(TaskStatus::Done, theme).2)
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(icon(icons::CHECK).size(px(10.0)).text_color(gpui::white())),
                )
                .into_any_element(),
            StepStatus::Planned | StepStatus::Skipped => {
                let ring = if status == StepStatus::Skipped {
                    theme.text_faint.opacity(0.4)
                } else {
                    theme.text_faint
                };
                slot.child(
                    div()
                        .size(px(12.0))
                        .rounded_full()
                        .border_1()
                        .border_color(ring),
                )
                .into_any_element()
            }
        }
    }

    fn shimmer(&self, working: bool, cx: &mut Context<Self>) -> Option<f32> {
        (working && !motion::reduced_motion(cx)).then(|| {
            motion::pulse_lease(cx.entity_id(), cx);
            super::tree::title_phase(self.shimmer_since, Instant::now())
        })
    }

    fn step_subline(
        row: &StepRow,
        thread: Option<&Thread>,
        shimmer: Option<f32>,
        theme: &Theme,
    ) -> Vec<AnyElement> {
        let mut parts = Vec::new();
        if let (Some((mark, tint)), Some(thread)) = (row.brand, thread) {
            parts.push(
                icon(mark)
                    .size(px(12.0))
                    .flex_none()
                    .text_color(tint.unwrap_or(theme.text_muted))
                    .into_any_element(),
            );
            if thread.title != crate::transcript::single_line(&row.step.title) {
                parts.push(
                    div()
                        .min_w_0()
                        .truncate()
                        .child(thread.title.clone())
                        .into_any_element(),
                );
                parts.push(div().flex_none().child("·").into_any_element());
            }
        }
        let (_, _, working) = column_tone(TaskStatus::Working, theme);
        let (_, _, asked) = column_tone(TaskStatus::NeedsYou, theme);
        let (_, _, complete) = column_tone(TaskStatus::Done, theme);
        let state = match Self::step_state(row, thread) {
            StepState::Working(since) => div()
                .flex()
                .items_center()
                .gap(px(4.0))
                .text_color(working)
                .child(super::tree::shimmer_text(
                    "Working",
                    shimmer,
                    ui_rems(11.5),
                    working,
                    theme,
                ))
                .children(since.map(|since| {
                    crate::loaders::elapsed_label(
                        format!("task-center-step-elapsed-{}", row.step.id),
                        since,
                        ui_rems(11.5),
                        working,
                    )
                })),
            StepState::Asked => div().text_color(asked).child("Asked you a question"),
            StepState::Failed(label, summary) => div()
                .flex_1()
                .min_w_0()
                .truncate()
                .text_color(if row.status == StepStatus::Failed {
                    theme.danger
                } else {
                    theme.warning
                })
                .child(match summary {
                    Some(summary) => {
                        format!("{label} · {}", crate::transcript::single_line(&summary))
                    }
                    None => label.into(),
                }),
            StepState::NeedsYou => div().text_color(theme.warning).child("Needs you"),
            StepState::Done => div().text_color(complete).child("Done"),
            StepState::Planned => div().child("Planned"),
            StepState::Skipped => div().child("Skipped"),
        };
        parts.push(state.into_any_element());
        parts
    }

    fn hover_action(
        theme: &Theme,
        id: String,
        label: &'static str,
        step: usize,
        group: SharedString,
    ) -> gpui::Stateful<gpui::Div> {
        let selector = id.clone();
        div()
            .id(SharedString::from(id))
            .debug_selector(move || selector.clone())
            .role(gpui::Role::Button)
            .aria_label(format!("{label} step {step}"))
            .flex_none()
            .px(px(8.0))
            .py(px(2.0))
            .rounded_full()
            .cursor_pointer()
            .text_size(ui_rems(11.5))
            .text_color(theme.text_muted)
            .invisible()
            .group_hover(group, |s| s.visible())
            .hover(|s| s.bg(theme.wash(0.10)).text_color(theme.text))
            .child(label)
    }

    fn tag_pill(tag: Tag, theme: &Theme) -> AnyElement {
        let (label, tone): (&str, Hsla) = match tag {
            Tag::Edited => ("Edited", theme.warning),
            Tag::New => ("New", theme.accent),
            Tag::Removed => ("Removed", theme.danger),
            Tag::Moved => ("New order", theme.accent),
        };
        div()
            .flex_none()
            .px(px(6.0))
            .rounded_full()
            .bg(tone.opacity(0.15))
            .text_size(ui_rems(11.0))
            .text_color(tone)
            .child(label)
            .into_any_element()
    }

    pub(crate) fn markdown(key: String, text: &str, theme: &Theme, window: &Window) -> AnyElement {
        let tree = crate::markdown::parse_full(text);
        let options = crate::markdown::render::RenderOptions {
            tasks: None,
            media: None,
            row_key: key.into(),
            veil: None,
            cache: None,
            now: Instant::now(),
            copy: None,
            link: None,
            workspace_root: None,
            code: None,
            tables: None,
            math: Default::default(),
        };
        crate::markdown::render::render_tree(&tree, &options, theme, window, &|_| None)
    }

    #[allow(clippy::too_many_arguments)]
    fn edit_card(
        &self,
        editor: &Editor,
        head: AnyElement,
        indent: f32,
        pad: (f32, f32),
        theme: &Theme,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let now = Instant::now();
        let growing = !editor.grow.settled_at(now);
        if growing {
            window.request_animation_frame();
        }
        let natural = self
            .painted
            .borrow()
            .get(&Key::Editor)
            .map(|rect| f32::from(rect.size.height));
        let height = growing.then(|| {
            motion::lerp(
                editor.from,
                natural.unwrap_or(editor.from),
                editor.grow.value_at(now),
            )
        });
        let body = editor.body.clone();
        let notes = div()
            .id("task-center-edit-body")
            .debug_selector(|| "task-center-edit-body".into())
            .pt(px(8.0))
            .text_size(ui_rems(12.5))
            .text_color(theme.text)
            .capture_action(cx.listener(move |_, _: &Submit, window, cx| {
                if body.focus_handle(cx).is_focused(window) {
                    cx.stop_propagation();
                    body.update(cx, |input, cx| {
                        input.replace_text_in_range(None, "\n", window, cx)
                    });
                }
            }))
            .child(crate::composer::growing_field(
                editor.body.clone(),
                18.0,
                260.0,
                cx,
            ));
        let paths = editor.images.clone();
        let open_paths = paths.clone();
        let tray = div()
            .debug_selector(|| "task-center-edit-tray".into())
            .pt(px(10.0))
            .flex()
            .flex_col()
            .gap(px(6.0))
            .child(
                images::strip(
                    "task-center-edit",
                    &paths,
                    images::SPEC_THUMB,
                    &self.state,
                    theme,
                    move |center: &mut Self, ix, _, cx| {
                        center.open_images(open_paths.clone(), ix, cx)
                    },
                    Some(
                        |center: &mut Self, ix, _: &mut Window, cx: &mut Context<Self>| {
                            center.remove_image(ix, cx)
                        },
                    ),
                    cx,
                )
                .child(
                    images::add_tile("task-center-edit-add-image", theme).on_click(
                        cx.listener(|center, _: &ClickEvent, _, cx| center.pick_images(None, cx)),
                    ),
                ),
            )
            .when(editor.started, |el| {
                el.child(
                    div()
                        .debug_selector(|| "task-center-edit-started".into())
                        .text_size(ui_rems(11.5))
                        .text_color(theme.text_faint)
                        .child("This step's thread has started, so new images will not reach it."),
                )
            });
        let hint = if cfg!(target_os = "macos") {
            "⌘↵ save · Esc cancel"
        } else {
            "Ctrl+↵ save · Esc cancel"
        };
        let footer = div()
            .mt(px(12.0))
            .pt(px(9.0))
            .border_t_1()
            .border_color(theme.border)
            .flex()
            .items_center()
            .gap(px(4.0))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_size(ui_rems(11.5))
                    .text_color(theme.text_faint)
                    .child(hint),
            )
            .child(
                widgets::text_action(theme, widgets::ActionTone::Quiet, "Cancel")
                    .id("task-center-edit-cancel")
                    .debug_selector(|| "task-center-edit-cancel".into())
                    .role(gpui::Role::Button)
                    .min_h(px(24.0))
                    .rounded_full()
                    .on_click(cx.listener(|center, _: &ClickEvent, _, cx| center.cancel_edit(cx))),
            )
            .child(
                widgets::text_action(theme, widgets::ActionTone::Solid, "Save")
                    .id("task-center-edit-save")
                    .debug_selector(|| "task-center-edit-save".into())
                    .role(gpui::Role::Button)
                    .min_h(px(24.0))
                    .rounded_full()
                    .on_click(cx.listener(|center, _: &ClickEvent, _, cx| center.commit_edit(cx))),
            );
        let content = div()
            .relative()
            .flex_none()
            .flex()
            .flex_col()
            .pl(px(pad.0))
            .pr(px(12.0))
            .py(px(pad.1))
            .child(morph::measure(&self.painted, Key::Editor, Edges::default()))
            .child(head)
            .child(
                div()
                    .pl(px(indent))
                    .flex()
                    .flex_col()
                    .child(notes)
                    .child(tray)
                    .child(footer),
            );
        div()
            .debug_selector(|| "task-center-editor".into())
            .flex()
            .flex_col()
            .overflow_hidden()
            .rounded(px(12.0))
            .border_1()
            .border_color(theme.border)
            .bg(theme.surface_card)
            .shadow(vec![gpui::BoxShadow {
                color: gpui::black().opacity(0.3),
                offset: gpui::point(px(0.0), px(10.0)),
                blur_radius: px(28.0),
                spread_radius: px(0.0),
                inset: false,
            }])
            .when_some(height, |el, height| el.h(px(height)))
            .drag_over::<gpui::ExternalPaths>({
                let accent = theme.accent;
                move |style, _, _, _| style.border_color(accent)
            })
            .on_drop(cx.listener(|center, paths: &gpui::ExternalPaths, _, cx| {
                cx.stop_propagation();
                let staged = images::staged_files(paths.paths(), cx);
                center.add_images(staged, cx);
            }))
            .on_key_down(cx.listener(|center, event: &KeyDownEvent, _, cx| {
                let modifiers = &event.keystroke.modifiers;
                match event.keystroke.key.as_str() {
                    "escape" => {
                        cx.stop_propagation();
                        center.cancel_edit(cx);
                    }
                    "enter" if modifiers.platform || modifiers.control => {
                        cx.stop_propagation();
                        center.commit_edit(cx);
                    }
                    _ => {}
                }
            }))
            .child(content)
            .into_any_element()
    }

    fn render_overview(
        &self,
        task: &zeron_proto::Task,
        editable: bool,
        theme: &Theme,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        if !task.plan_doc {
            return None;
        }
        let editor = self
            .editor
            .as_ref()
            .filter(|editor| editor.target == EditTarget::Overview);
        let (text, saved_images) = match &self.draft.overview {
            Some(draft) => (draft.notes.clone(), draft.images.clone()),
            None => (task.plan.clone(), task.images.clone()),
        };
        let edit = (editable && editor.is_none()).then(|| {
            widgets::text_action(theme, widgets::ActionTone::Quiet, "Edit")
                .id("task-center-edit-overview")
                .debug_selector(|| "task-center-edit-overview".into())
                .role(gpui::Role::Button)
                .aria_label("Edit the overview")
                .min_h(px(22.0))
                .rounded_full()
                .on_click(cx.listener(|center, _: &ClickEvent, window, cx| {
                    center.start_edit(EditTarget::Overview, window, cx)
                }))
        });
        let head = div()
            .flex()
            .items_center()
            .gap(px(6.0))
            .child(
                icon(icons::FILE_MARKDOWN)
                    .size(px(13.0))
                    .text_color(theme.text_muted),
            )
            .child(
                div()
                    .text_size(ui_rems(12.0))
                    .font_weight(gpui::FontWeight::MEDIUM)
                    .text_color(theme.text_muted)
                    .child("Overview"),
            )
            .children(
                self.draft
                    .overview
                    .is_some()
                    .then(|| Self::tag_pill(Tag::Edited, theme)),
            )
            .child(div().flex_1())
            .children(edit);
        if let Some(editor) = editor {
            return Some(
                div()
                    .id("task-center-overview")
                    .debug_selector(|| "task-center-overview".into())
                    .child(self.edit_card(
                        editor,
                        head.into_any_element(),
                        0.0,
                        (12.0, 10.0),
                        theme,
                        window,
                        cx,
                    ))
                    .into_any_element(),
            );
        }
        let body = if text.trim().is_empty() {
            div()
                .text_size(ui_rems(12.5))
                .text_color(theme.text_muted)
                .child("The lead has not written the overview yet.")
                .into_any_element()
        } else {
            Self::markdown(
                format!("task-center-overview-{}", task.id),
                &text,
                theme,
                window,
            )
        };
        let thumbs = (!saved_images.is_empty()).then(|| {
            let paths = saved_images.clone();
            images::strip(
                "task-center-overview",
                &saved_images,
                images::SPEC_THUMB,
                &self.state,
                theme,
                move |center: &mut Self, ix, _, cx| center.open_images(paths.clone(), ix, cx),
                None::<fn(&mut Self, usize, &mut Window, &mut Context<Self>)>,
                cx,
            )
            .mt(px(4.0))
        });
        Some(
            div()
                .id("task-center-overview")
                .debug_selector(|| "task-center-overview".into())
                .relative()
                .flex()
                .flex_col()
                .gap(px(6.0))
                .px(px(12.0))
                .py(px(10.0))
                .rounded(px(12.0))
                .border_1()
                .border_color(theme.border)
                .child(morph::measure(
                    &self.painted,
                    Key::Overview,
                    Edges::default(),
                ))
                .child(head)
                .child(body)
                .children(thumbs)
                .into_any_element(),
        )
    }

    fn grip(
        theme: &Theme,
        step_id: &str,
        lock: Option<&'static str>,
        group: SharedString,
    ) -> gpui::Stateful<gpui::Div> {
        let dot = || div().size(px(2.5)).rounded_full().bg(theme.text_muted);
        let column = || {
            div()
                .flex()
                .flex_col()
                .gap(px(2.5))
                .child(dot())
                .child(dot())
                .child(dot())
        };
        let selector = format!("task-center-grip-{step_id}");
        div()
            .id(SharedString::from(selector.clone()))
            .debug_selector(move || selector.clone())
            .absolute()
            .left(px(2.0))
            .top(px(8.0))
            .w(px(12.0))
            .h(px(ICON_SLOT))
            .flex()
            .items_center()
            .justify_center()
            .gap(px(2.0))
            .invisible()
            .group_hover(group, |s| s.visible())
            .opacity(if lock.is_some() { 0.35 } else { 0.85 })
            .cursor(if lock.is_some() {
                CursorStyle::OperationNotAllowed
            } else {
                CursorStyle::OpenHand
            })
            .child(column())
            .child(column())
            .when_some(lock, |el, lock| el.tooltip(widgets::text_tooltip(lock)))
    }

    #[allow(clippy::too_many_arguments)]
    fn render_step_card(
        &self,
        index: usize,
        row: &StepRow,
        thread: Option<&Thread>,
        editor: &Editor,
        theme: &Theme,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let step_id = row.step.id.clone();
        let head = div()
            .flex()
            .items_start()
            .gap(px(10.0))
            .child(Self::step_icon(row.status, theme, cx))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap(px(2.0))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(4.0))
                            .text_size(ui_rems(13.0))
                            .text_color(theme.text)
                            .child(div().flex_none().child(format!("{}.", index + 1)))
                            .children(editor.title.clone().map(|title| {
                                div()
                                    .id("task-center-edit-title")
                                    .debug_selector(|| "task-center-edit-title".into())
                                    .flex_1()
                                    .min_w_0()
                                    .child(title)
                            })),
                    )
                    .child(
                        div()
                            .min_w_0()
                            .flex()
                            .items_center()
                            .gap(px(5.0))
                            .text_size(ui_rems(11.5))
                            .text_color(theme.text_muted)
                            .children(Self::step_subline(
                                row,
                                thread,
                                self.shimmer(row.status == StepStatus::Working, cx),
                                theme,
                            )),
                    ),
            );
        div()
            .id(SharedString::from(format!("task-center-step-{step_id}")))
            .debug_selector(move || format!("task-center-step-{step_id}"))
            .relative()
            .child(morph::measure(
                &self.painted,
                Key::Task(row.step.id.clone()),
                Edges::default(),
            ))
            .child(self.edit_card(
                editor,
                head.into_any_element(),
                ICON_SLOT + 10.0,
                (ROW_PAD_LEFT, 8.0),
                theme,
                window,
                cx,
            ))
            .into_any_element()
    }

    #[allow(clippy::too_many_arguments)]
    fn render_step(
        &self,
        index: usize,
        last: bool,
        row: &StepRow,
        thread: Option<&Thread>,
        tag: Option<Tag>,
        range: Option<(usize, usize)>,
        editable: bool,
        theme: &Theme,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let step_id = row.step.id.clone();
        let group = SharedString::from(format!("task-center-step-{step_id}"));
        let selected = self.selected == Selection::Step(step_id.clone());
        if let Some(editor) = self
            .editor
            .as_ref()
            .filter(|editor| editor.target == EditTarget::Step(step_id.clone()))
        {
            return self.render_step_card(index, row, thread, editor, theme, window, cx);
        }
        let expanded = self.expanded.contains(&step_id);
        let moved = self.draft.moved.contains(&step_id);
        let registered = tag != Some(Tag::New) && tag != Some(Tag::Removed);
        let retry = registered
            && row.step.outcome.is_none()
            && matches!(Self::step_state(row, thread), StepState::Failed(..));
        let skip = registered
            && row.step.outcome.is_none()
            && matches!(
                row.status,
                StepStatus::Planned | StepStatus::NeedsYou | StepStatus::Failed
            );
        let reopen = registered && row.step.outcome.is_some();
        let removable = editable
            && (tag == Some(Tag::New)
                || (tag != Some(Tag::Removed)
                    && row.status == StepStatus::Planned
                    && thread.is_none()));
        let struck = row.status == StepStatus::Skipped || tag == Some(Tag::Removed);
        let title_color = if struck { theme.text_muted } else { theme.text };
        let title = div()
            .flex()
            .items_center()
            .gap(px(6.0))
            .child(
                div()
                    .min_w_0()
                    .truncate()
                    .text_size(ui_rems(13.0))
                    .text_color(title_color)
                    .when(struck, |el| el.line_through())
                    .child(format!("{}. {}", index + 1, row.step.title)),
            )
            .children(tag.map(|tag| Self::tag_pill(tag, theme)))
            .children(moved.then(|| Self::tag_pill(Tag::Moved, theme)));
        let mut actions_row: Vec<AnyElement> = Vec::new();
        if skip {
            let id = step_id.clone();
            actions_row.push(
                Self::hover_action(
                    theme,
                    format!("task-center-skip-{step_id}"),
                    "Skip",
                    index + 1,
                    group.clone(),
                )
                .on_click(cx.listener(move |center, _: &ClickEvent, _, cx| {
                    cx.stop_propagation();
                    center.set_outcome(&id, Some(StepOutcome::Skipped), cx);
                }))
                .into_any_element(),
            );
        }
        if reopen {
            let id = step_id.clone();
            actions_row.push(
                Self::hover_action(
                    theme,
                    format!("task-center-reopen-{step_id}"),
                    "Reopen",
                    index + 1,
                    group.clone(),
                )
                .on_click(cx.listener(move |center, _: &ClickEvent, _, cx| {
                    cx.stop_propagation();
                    center.set_outcome(&id, None, cx);
                }))
                .into_any_element(),
            );
        }
        let side_button = |label: &'static str, selector: String, tone: widgets::ActionTone| {
            widgets::text_action(theme, tone, label)
                .id(SharedString::from(selector.clone()))
                .debug_selector(move || selector.clone())
                .role(gpui::Role::Button)
                .aria_label(format!("{label} step {}", index + 1))
                .flex_none()
                .min_h(px(24.0))
                .rounded_full()
        };
        let retry_button = retry.then(|| {
            let id = step_id.clone();
            side_button(
                "Retry",
                format!("task-center-retry-{step_id}"),
                widgets::ActionTone::Filled,
            )
            .on_click(cx.listener(move |_, _: &ClickEvent, _, cx| {
                cx.stop_propagation();
                cx.emit(TaskCenterEvent::RetryStep(id.clone()));
            }))
        });
        let restore_button = (tag == Some(Tag::Removed)).then(|| {
            let id = step_id.clone();
            side_button(
                "Restore",
                format!("task-center-restore-{step_id}"),
                widgets::ActionTone::Quiet,
            )
            .on_click(cx.listener(move |center, _: &ClickEvent, _, cx| {
                cx.stop_propagation();
                center.toggle_remove(&id, cx);
            }))
        });
        let toggle_id = step_id.clone();
        let spec_toggle = div()
            .id(SharedString::from(format!("task-center-spec-{step_id}")))
            .debug_selector({
                let id = step_id.clone();
                move || format!("task-center-spec-{id}")
            })
            .role(gpui::Role::Button)
            .aria_expanded(expanded)
            .flex_none()
            .flex()
            .items_center()
            .gap(px(2.0))
            .cursor_pointer()
            .hover(|s| s.text_color(theme.text))
            .child("spec")
            .child(
                icon(if expanded {
                    icons::LUCIDE_CHEVRON_DOWN
                } else {
                    icons::LUCIDE_CHEVRON_RIGHT
                })
                .size(px(11.0))
                .text_color(theme.text_muted),
            )
            .on_click(cx.listener(move |center, _: &ClickEvent, _, cx| {
                cx.stop_propagation();
                center.toggle_spec(&toggle_id, cx);
            }));
        let shimmer = self.shimmer(row.status == StepStatus::Working, cx);
        let mut subline = Self::step_subline(row, thread, shimmer, theme);
        subline.push(div().flex_none().child("·").into_any_element());
        subline.push(spec_toggle.into_any_element());
        let panel = expanded.then(|| {
            let spec = if row.step.notes.trim().is_empty() {
                div()
                    .text_size(ui_rems(12.5))
                    .text_color(theme.text_muted)
                    .child("No spec yet.")
                    .into_any_element()
            } else {
                Self::markdown(
                    format!("task-center-spec-body-{step_id}"),
                    &row.step.notes,
                    theme,
                    window,
                )
            };
            let thumbs = (!row.step.images.is_empty()).then(|| {
                let paths = row.step.images.clone();
                images::strip(
                    &format!("task-center-spec-{step_id}"),
                    &row.step.images,
                    images::SPEC_THUMB,
                    &self.state,
                    theme,
                    move |center: &mut Self, ix, _, cx| center.open_images(paths.clone(), ix, cx),
                    None::<fn(&mut Self, usize, &mut Window, &mut Context<Self>)>,
                    cx,
                )
            });
            let edit = (editable && tag != Some(Tag::Removed)).then(|| {
                let id = step_id.clone();
                side_button(
                    "Edit",
                    format!("task-center-edit-{step_id}"),
                    widgets::ActionTone::Quiet,
                )
                .on_click(cx.listener(
                    move |center, _: &ClickEvent, window, cx| {
                        center.start_edit(EditTarget::Step(id.clone()), window, cx)
                    },
                ))
            });
            let remove = removable.then(|| {
                let id = step_id.clone();
                side_button(
                    "Remove",
                    format!("task-center-remove-{step_id}"),
                    widgets::ActionTone::Quiet,
                )
                .on_click(
                    cx.listener(move |center, _: &ClickEvent, _, cx| center.toggle_remove(&id, cx)),
                )
            });
            div()
                .debug_selector({
                    let id = step_id.clone();
                    move || format!("task-center-spec-panel-{id}")
                })
                .ml(px(ROW_PAD_LEFT + ICON_SLOT + 10.0))
                .mr(px(8.0))
                .mb(px(6.0))
                .pl(px(10.0))
                .border_l_2()
                .border_color(theme.border)
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(6.0))
                        .child(spec)
                        .children(thumbs)
                        .child(
                            div()
                                .flex()
                                .gap(px(4.0))
                                .ml(px(-8.0))
                                .children(edit)
                                .children(remove),
                        ),
                )
        });
        let connector = (!last).then(|| {
            div()
                .absolute()
                .left(px(ROW_PAD_LEFT + ICON_SLOT / 2.0))
                .top(px(8.0 + ICON_SLOT + 4.0))
                .bottom(px(-8.0))
                .w(px(1.0))
                .bg(theme.border)
        });
        let grip = editable && tag != Some(Tag::Removed);
        let lock = match row.status {
            _ if range.is_some() => None,
            StepStatus::Done | StepStatus::Skipped => Some("Finished steps keep their place"),
            _ => Some("Running steps keep their place"),
        };
        let grip = grip.then(|| {
            let el = Self::grip(theme, &step_id, lock, group.clone());
            if lock.is_some() {
                return el;
            }
            let payload = StepDragPayload {
                task_id: self.task_id.clone(),
                step_id: step_id.clone(),
            };
            el.on_mouse_down(
                MouseButton::Left,
                cx.listener(|center, event: &MouseDownEvent, _, _| {
                    center.press = f32::from(event.position.y);
                }),
            )
            .on_click(|_, _, cx| cx.stop_propagation())
            .on_drag(payload, |_, _, _, cx| cx.new(|_| StepDragGhost))
        });
        let actions = StepActions {
            step_id: step_id.clone(),
            edit: editable && tag != Some(Tag::Removed),
            thread: thread.and(row.step.chat_id.clone()),
            up: editable
                && range.is_some_and(|(lo, hi)| index >= 1 && (lo..=hi).contains(&(index - 1))),
            down: editable && range.is_some_and(|(lo, hi)| (lo..=hi).contains(&(index + 1))),
            outcome: if reopen {
                Outcome::Reopen
            } else if skip {
                Outcome::Skip
            } else {
                Outcome::Locked
            },
            remove: removable,
        };
        let select_id = step_id.clone();
        let aria = format!("Step {}: {}", index + 1, row.step.title);
        div()
            .relative()
            .flex()
            .flex_col()
            .child(morph::measure(
                &self.painted,
                Key::Task(step_id.clone()),
                Edges::default(),
            ))
            .children(connector)
            .child(
                div()
                    .id(SharedString::from(format!("task-center-step-{step_id}")))
                    .debug_selector(move || format!("task-center-step-{step_id}"))
                    .group(group)
                    .role(gpui::Role::ListItem)
                    .aria_label(aria)
                    .aria_selected(selected)
                    .relative()
                    .flex()
                    .items_start()
                    .gap(px(10.0))
                    .pl(px(ROW_PAD_LEFT))
                    .pr(px(8.0))
                    .py(px(8.0))
                    .rounded(px(10.0))
                    .cursor_pointer()
                    .when(selected, |el| el.bg(theme.wash(0.08)))
                    .when(!selected, |el| el.hover(|s| s.bg(theme.wash(0.05))))
                    .children(grip)
                    .child(Self::step_icon(row.status, theme, cx))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .gap(px(2.0))
                            .child(title)
                            .child(
                                div()
                                    .min_w_0()
                                    .flex()
                                    .items_center()
                                    .gap(px(5.0))
                                    .text_size(ui_rems(11.5))
                                    .text_color(theme.text_muted)
                                    .children(subline),
                            ),
                    )
                    .child(
                        div()
                            .flex_none()
                            .flex()
                            .items_center()
                            .gap(px(2.0))
                            .children(actions_row)
                            .children(retry_button)
                            .children(restore_button),
                    )
                    .on_mouse_down(
                        MouseButton::Right,
                        cx.listener(move |center, event: &MouseDownEvent, window, cx| {
                            cx.stop_propagation();
                            window.prevent_default();
                            center.open_menu(actions.clone(), event.position, window, cx);
                        }),
                    )
                    .on_click(cx.listener(move |center, _: &ClickEvent, _, cx| {
                        center.select(Selection::Step(select_id.clone()), cx)
                    })),
            )
            .children(panel)
            .into_any_element()
    }

    fn render_edit_bar(&self, theme: &Theme, cx: &mut Context<Self>) -> Option<AnyElement> {
        let count = self.draft.count();
        if count == 0 {
            return None;
        }
        Some(
            div()
                .id("task-center-edit-bar")
                .debug_selector(|| "task-center-edit-bar".into())
                .flex_none()
                .mx(px(14.0))
                .mb(px(12.0))
                .px(px(12.0))
                .py(px(8.0))
                .rounded(px(12.0))
                .bg(theme.wash(0.08))
                .border_1()
                .border_color(theme.border)
                .flex()
                .items_center()
                .gap(px(8.0))
                .text_size(ui_rems(12.5))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .text_color(theme.text)
                        .child(format!(
                            "{count} unsent {}",
                            plural(count, "change", "changes")
                        )),
                )
                .child(
                    widgets::text_action(theme, widgets::ActionTone::Quiet, "Undo")
                        .id("task-center-undo")
                        .debug_selector(|| "task-center-undo".into())
                        .role(gpui::Role::Button)
                        .min_h(px(26.0))
                        .rounded_full()
                        .on_click(
                            cx.listener(|center, _: &ClickEvent, _, cx| center.undo_edits(cx)),
                        ),
                )
                .child(
                    widgets::text_action(theme, widgets::ActionTone::Solid, "Send to lead")
                        .id("task-center-send")
                        .debug_selector(|| "task-center-send".into())
                        .role(gpui::Role::Button)
                        .min_h(px(26.0))
                        .rounded_full()
                        .on_click(cx.listener(|center, _: &ClickEvent, _, cx| {
                            center.send_edits(false, cx)
                        })),
                )
                .into_any_element(),
        )
    }

    fn render_add_step(&self, theme: &Theme) -> AnyElement {
        div()
            .id("task-center-add-step")
            .debug_selector(|| "task-center-add-step".into())
            .flex()
            .items_center()
            .gap(px(10.0))
            .pl(px(ROW_PAD_LEFT))
            .pr(px(8.0))
            .py(px(8.0))
            .rounded(px(10.0))
            .child(
                div()
                    .flex_none()
                    .size(px(ICON_SLOT))
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(
                        icon(icons::PLUS)
                            .size(px(13.0))
                            .text_color(theme.text_faint),
                    ),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .text_size(ui_rems(13.0))
                    .child(self.add.clone()),
            )
            .into_any_element()
    }

    fn render_live(
        &self,
        task: &zeron_proto::Task,
        plan: &Rollup,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let step = match &self.selected {
            Selection::Step(id) => plan
                .steps
                .iter()
                .find(|row| &row.step.id == id)
                .zip(plan.step_label(id)),
            Selection::Lead => None,
        };
        let live_chat = self.live.as_ref().and_then(|live| {
            let state = self.state.read(cx);
            let chat = state.chats.iter().find(|chat| chat.id == live.chat_id)?;
            Some((
                chat.id.clone(),
                crate::transcript::single_line(&crate::home::chat_title(chat)),
                chat_brand(chat),
            ))
        });
        let step_has_thread = step.as_ref().is_some_and(|(row, _)| {
            live_chat
                .as_ref()
                .is_some_and(|(id, _, _)| row.step.chat_id.as_deref() == Some(id.as_str()))
        });
        let (title, sub) = match step {
            Some((row, label)) if !step_has_thread => (
                row.step.title.clone(),
                format!("{label} has not started · messages go to the lead"),
            ),
            Some((_, label)) => (
                live_chat
                    .as_ref()
                    .map(|(_, title, _)| title.clone())
                    .unwrap_or_default(),
                label,
            ),
            None => (
                live_chat
                    .as_ref()
                    .map(|(_, title, _)| title.clone())
                    .unwrap_or_else(|| task.title.clone()),
                "Lead".into(),
            ),
        };
        let (mark, tint) = live_chat
            .as_ref()
            .map(|(_, _, brand)| *brand)
            .unwrap_or((icons::BOT, None));
        let open = live_chat.as_ref().map(|(id, _, _)| {
            let id = id.clone();
            widgets::text_action(theme, widgets::ActionTone::Filled, "Open thread")
                .id("task-center-open-thread")
                .debug_selector(|| "task-center-open-thread".into())
                .role(gpui::Role::Button)
                .flex_none()
                .rounded_full()
                .on_click(cx.listener(move |_, _: &ClickEvent, _, cx| {
                    cx.emit(TaskCenterEvent::OpenChat(id.clone()))
                }))
        });
        let header = div()
            .id("task-center-live-header")
            .debug_selector(|| "task-center-live-header".into())
            .flex_none()
            .px(px(16.0))
            .py(px(10.0))
            .border_b_1()
            .border_color(theme.border)
            .flex()
            .items_center()
            .gap(px(8.0))
            .child(
                icon(mark)
                    .size(px(14.0))
                    .flex_none()
                    .text_color(tint.unwrap_or(theme.text_muted)),
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
                            .text_size(ui_rems(13.0))
                            .text_color(theme.text)
                            .child(title),
                    )
                    .child(
                        div()
                            .truncate()
                            .text_size(ui_rems(11.5))
                            .text_color(theme.text_muted)
                            .child(sub),
                    ),
            )
            .children(open);
        let body = match &self.live {
            Some(live) => div()
                .flex_1()
                .min_h_0()
                .flex()
                .flex_col()
                .child(div().flex_1().min_h_0().child(live.transcript.clone()))
                .child(div().flex_none().child(live.composer.clone()))
                .into_any_element(),
            None => div()
                .flex_1()
                .p(px(16.0))
                .text_size(ui_rems(12.5))
                .text_color(theme.text_muted)
                .child("The lead has not started yet.")
                .into_any_element(),
        };
        div()
            .id("task-center-live")
            .debug_selector(|| "task-center-live".into())
            .flex_1()
            .min_w_0()
            .h_full()
            .flex()
            .flex_col()
            .border_l_1()
            .border_color(theme.border)
            .child(header)
            .child(body)
            .into_any_element()
    }
}

fn retryable(status: DelegationStatus) -> bool {
    matches!(
        status,
        DelegationStatus::Failed | DelegationStatus::Interrupted | DelegationStatus::Cancelled
    )
}

fn plural(n: usize, one: &'static str, many: &'static str) -> &'static str {
    if n == 1 { one } else { many }
}

impl TaskCenter {
    fn fill_at(&mut self, progress: PlanProgress, reduced: bool, now: Instant) -> (f32, bool) {
        let PlanProgress { done, total } = progress;
        let fraction = if total == 0 {
            0.0
        } else {
            done as f32 / total as f32
        };
        let fill = self.fill.get_or_insert_with(|| Tween::at(fraction, now));
        if fill.target() != fraction {
            fill.retarget(fraction, PROGRESS_FILL, reduced, now);
        }
        (fill.value_at(now), !fill.settled_at(now))
    }

    /// Steps on screen when the page first draws are already there; only later ones rise in.
    fn arrivals_at(&mut self, ids: &[&str], reduced: bool, now: Instant) -> Vec<Tween> {
        let first = self.arrivals.is_none();
        let previous = self.arrivals.take().unwrap_or_default();
        let arrivals: HashMap<String, Tween> = ids
            .iter()
            .map(|id| {
                let tween = previous.get(*id).copied().unwrap_or_else(|| {
                    let mut tween = Tween::at(if first { 1.0 } else { 0.0 }, now);
                    tween.retarget(1.0, STEP_IN, reduced, now);
                    tween
                });
                (id.to_string(), tween)
            })
            .collect();
        let tweens = ids.iter().map(|id| arrivals[*id]).collect();
        self.arrivals = Some(arrivals);
        tweens
    }
}

impl TaskCenter {
    #[allow(clippy::too_many_arguments)]
    fn menu_item(
        &self,
        theme: &Theme,
        id: &'static str,
        label: &'static str,
        enabled: bool,
        danger: bool,
        action: MenuAction,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let row = popover::menu_row(theme, false, id)
            .id(id)
            .debug_selector(move || id.into())
            .role(gpui::Role::MenuItem)
            .aria_label(label)
            .child(label);
        if !enabled {
            return row
                .bg(gpui::transparent_black())
                .text_color(theme.text_faint)
                .cursor_default()
                .into_any_element();
        }
        row.when(danger, |el| el.text_color(theme.danger))
            .on_click(cx.listener(move |center, _: &ClickEvent, window, cx| {
                center.menu_action(action, window, cx)
            }))
            .into_any_element()
    }

    fn render_menu(&self, theme: &Theme, cx: &mut Context<Self>) -> Option<AnyElement> {
        let menu = self.menu.get()?;
        let theme = theme.for_popup();
        let actions = &menu.actions;
        let outcome = match actions.outcome {
            Outcome::Reopen => "Reopen",
            Outcome::Skip | Outcome::Locked => "Skip",
        };
        let card = popover::popover_card(&theme)
            .w(px(180.0))
            .flex()
            .flex_col()
            .on_mouse_down_out(cx.listener(|center, _, _, cx| center.close_menu(cx)))
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(self.menu_item(
                &theme,
                "task-center-menu-edit",
                "Edit spec",
                actions.edit,
                false,
                MenuAction::Edit,
                cx,
            ))
            .children(actions.thread.is_some().then(|| {
                self.menu_item(
                    &theme,
                    "task-center-menu-open",
                    "Open thread",
                    true,
                    false,
                    MenuAction::Open,
                    cx,
                )
            }))
            .child(self.menu_item(
                &theme,
                "task-center-menu-add-image",
                "Add image",
                actions.edit,
                false,
                MenuAction::AddImage,
                cx,
            ))
            .child(self.menu_item(
                &theme,
                "task-center-menu-up",
                "Move up",
                actions.up,
                false,
                MenuAction::Up,
                cx,
            ))
            .child(self.menu_item(
                &theme,
                "task-center-menu-down",
                "Move down",
                actions.down,
                false,
                MenuAction::Down,
                cx,
            ))
            .child(popover::menu_separator())
            .child(self.menu_item(
                &theme,
                "task-center-menu-outcome",
                outcome,
                !matches!(actions.outcome, Outcome::Locked),
                false,
                MenuAction::Outcome,
                cx,
            ))
            .child(self.menu_item(
                &theme,
                "task-center-menu-remove",
                "Remove",
                actions.remove,
                true,
                MenuAction::Remove,
                cx,
            ));
        Some(popover::menu_at(
            "task-center-step-menu",
            menu.position,
            card.into_any_element(),
            self.menu.closing_since(),
        ))
    }

    fn drop_line(&self, theme: &Theme) -> Option<AnyElement> {
        let drag = self.drag.as_ref().filter(|drag| drag.over != drag.from)?;
        let (top, height) = drag.rows[drag.over];
        let y = if drag.over < drag.from {
            top - 3.0
        } else {
            top + height + 1.0
        };
        let line = div()
            .debug_selector(|| "task-center-drop-line".into())
            .absolute()
            .left(px(6.0))
            .right(px(6.0))
            .top(px(y - drag.origin))
            .h(px(2.0))
            .rounded(px(1.0))
            .bg(theme.accent)
            .shadow(vec![gpui::BoxShadow {
                color: theme.accent.opacity(0.25),
                offset: gpui::point(px(0.0), px(0.0)),
                blur_radius: px(0.0),
                spread_radius: px(3.0),
                inset: false,
            }]);
        Some(gpui::deferred(line).priority(1).into_any_element())
    }

    fn row_offset(&self, ix: usize, step_id: &str, now: Instant) -> (f32, bool) {
        if let Some(drag) = &self.drag {
            if step_id == drag.id {
                return (drag.dy, true);
            }
            let shift = drag
                .shifts
                .get(&ix)
                .map_or(0.0, |tween| tween.value_at(now));
            return (shift * drag.rows[drag.from].1, false);
        }
        match &self.settle {
            Some((id, offset, tween)) if id == step_id => (offset * tween.value_at(now), false),
            _ => (0.0, false),
        }
    }

    fn rows_moving(&self, now: Instant) -> bool {
        self.drag
            .as_ref()
            .is_some_and(|drag| drag.shifts.values().any(|tween| !tween.settled_at(now)))
            || self
                .settle
                .as_ref()
                .is_some_and(|(_, _, tween)| !tween.settled_at(now))
    }
}

impl Render for TaskCenter {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::of(cx).clone();
        let found = {
            let state = self.state.read(cx);
            state
                .tasks
                .iter()
                .find(|task| task.id == self.task_id && !task.deleted)
                .map(|task| (task.clone(), rollup(state, task, Utc::now())))
        };
        let Some((task, plan)) = found else {
            return self.render_missing(&theme, cx);
        };
        self.draft.retain_live(&plan);
        let editable = task.completed_at.is_none() && plan.lead.is_some();
        let shown = self.shown(&task, &plan, cx);
        let slots = Self::slots(&shown);
        let reduced = motion::reduced_motion(cx);
        let now = Instant::now();
        if self.drag.is_some() && !cx.has_active_drag() {
            self.drag = None;
        }
        if self
            .settle
            .as_ref()
            .is_some_and(|(_, _, tween)| tween.settled_at(now))
        {
            self.settle = None;
        }
        let (fill, filling) = self.fill_at(plan.progress, reduced, now);
        let ids: Vec<&str> = shown.iter().map(|(row, ..)| row.step.id.as_str()).collect();
        let arrivals = self.arrivals_at(&ids, reduced, now);
        if filling || arrivals.iter().any(|tween| !tween.settled_at(now)) || self.rows_moving(now) {
            window.request_animation_frame();
        }
        let lead = self.render_lead(&task, &plan, &theme, cx);
        let reads = self.render_reads(&task, &theme, cx);
        let banner = self.render_banner(&task, &plan, &theme, cx);
        let overview = self.render_overview(&task, editable, &theme, window, cx);
        let last = shown.len().saturating_sub(1);
        let lift_bg = theme.surface_card;
        let steps: Vec<AnyElement> = shown
            .iter()
            .zip(&arrivals)
            .enumerate()
            .map(|(ix, ((row, tag, thread), arrival))| {
                let step = self.render_step(
                    ix,
                    ix == last && !editable,
                    row,
                    thread.as_ref(),
                    *tag,
                    move_range(&slots, ix),
                    editable,
                    &theme,
                    window,
                    cx,
                );
                let shown = arrival.value_at(now);
                let (offset, lifted) = self.row_offset(ix, &row.step.id, now);
                let el = div()
                    .relative()
                    .top(px(STEP_RISE * (1.0 - shown) + offset))
                    .opacity(shown)
                    .when(lifted, |el| {
                        el.rounded(px(10.0))
                            .bg(lift_bg)
                            .shadow(vec![gpui::BoxShadow {
                                color: gpui::black().opacity(0.45),
                                offset: gpui::point(px(0.0), px(12.0)),
                                blur_radius: px(30.0),
                                spread_radius: px(0.0),
                                inset: false,
                            }])
                    })
                    .child(step);
                if lifted {
                    gpui::deferred(el).into_any_element()
                } else {
                    el.into_any_element()
                }
            })
            .collect();
        let list = div()
            .id("task-center-plan")
            .debug_selector(|| "task-center-plan".into())
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .track_scroll(&self.scroll)
            .px(px(14.0))
            .py(px(12.0))
            .flex()
            .flex_col()
            .gap(px(10.0))
            .child(lead)
            .children(reads)
            .children(banner)
            .children(overview)
            .child(
                div()
                    .id("task-center-steps")
                    .relative()
                    .flex()
                    .flex_col()
                    .child(morph::measure(&self.painted, Key::Steps, Edges::default()))
                    .children(steps)
                    .children(self.drop_line(&theme))
                    .when(editable, |el| el.child(self.render_add_step(&theme)))
                    .on_drag_move::<StepDragPayload>(cx.listener(
                        |center, event: &DragMoveEvent<StepDragPayload>, _, cx| {
                            let drag = event.drag(cx);
                            if drag.task_id != center.task_id {
                                return;
                            }
                            let step_id = drag.step_id.clone();
                            center.drag_to(&step_id, f32::from(event.event.position.y), cx);
                        },
                    ))
                    .on_drop::<StepDragPayload>(
                        cx.listener(|center, _: &StepDragPayload, _, cx| center.drop_step(cx)),
                    )
                    .on_mouse_up_out(
                        MouseButton::Left,
                        cx.listener(|center, _, _, cx| center.drop_step(cx)),
                    ),
            );
        let left = div()
            .flex_1()
            .min_w(px(280.0))
            .h_full()
            .flex()
            .flex_col()
            .child(list)
            .children(editable.then(|| self.render_edit_bar(&theme, cx)).flatten());
        let live = self.render_live(&task, &plan, &theme, cx);
        div()
            .id("task-center")
            .debug_selector(|| "task-center".into())
            .track_focus(&self.focus)
            .on_key_down(cx.listener(|center, event: &KeyDownEvent, _, cx| {
                if event.keystroke.key == "escape" && center.menu.is_open() {
                    cx.stop_propagation();
                    center.close_menu(cx);
                }
            }))
            .relative()
            .size_full()
            .pt(px(Theme::TITLEBAR_HEIGHT))
            .flex()
            .flex_col()
            .child(self.render_header(&task, &plan, fill, &theme, cx))
            .children(self.render_send_error(&task, &theme, cx))
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .flex_row()
                    .child(left)
                    .child(live),
            )
            .children(self.render_menu(&theme, cx))
            .child(div().absolute().size_0().child(self.gallery.clone()))
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_planned_step_drops_only_below_the_last_started_one() {
        use Slot::*;
        let slots = [
            Started, Planned, Started, Planned, Removed, Planned, Added, Added,
        ];
        assert_eq!(move_range(&slots, 3), Some((3, 5)));
        assert_eq!(
            move_range(&slots, 1),
            Some((2, 5)),
            "it may drop below the running one"
        );
        assert_eq!(
            move_range(&slots, 6),
            Some((6, 7)),
            "a drafted step stays with the drafted"
        );
        assert_eq!(move_range(&slots, 0), None);
        assert_eq!(move_range(&slots, 4), None);
    }

    #[test]
    fn a_drag_lands_where_its_middle_crosses_a_row() {
        let rows = [(0.0, 40.0), (40.0, 40.0), (80.0, 100.0), (180.0, 40.0)];
        assert_eq!(drop_index(&rows, 3, (1, 3), -80.0), 2);
        assert_eq!(drop_index(&rows, 3, (1, 3), -150.0), 1);
        assert_eq!(
            drop_index(&rows, 3, (1, 3), -300.0),
            1,
            "never above the floor"
        );
        assert_eq!(drop_index(&rows, 1, (1, 3), 90.0), 2);
        assert_eq!(drop_index(&rows, 1, (1, 3), 10.0), 1);
        assert_eq!(drop_index(&rows, 0, (1, 3), 50.0), 1);
        assert_eq!(
            drop_index(&rows, 0, (2, 3), 50.0),
            0,
            "a short drag stays above the floor"
        );
    }

    #[test]
    fn a_move_lands_by_id_when_the_list_changed_under_it() {
        let place = Place::Beside {
            anchor: "b".into(),
            after: false,
        };
        assert_eq!(landing(&["a", "b", "d", "c"], 3, &place), Some(1));
        assert_eq!(
            landing(&["a", "b", "c", "d"], 2, &place),
            Some(1),
            "a draft order dropped mid-drag still moves c before b"
        );
        let gone = Place::Beside {
            anchor: "x".into(),
            after: true,
        };
        assert_eq!(landing(&["a", "b"], 1, &gone), None);
        assert_eq!(landing(&["a", "b"], 0, &Place::Up), None);
        assert_eq!(landing(&["a", "b"], 1, &Place::Down), None);
    }

    #[test]
    fn the_draft_order_comes_first_and_new_rows_follow() {
        assert_eq!(arrange(&["a", "b", "c"], None), [0, 1, 2]);
        let order = ["c".to_string(), "a".into(), "gone".into()];
        assert_eq!(arrange(&["a", "b", "c"], Some(&order)), [2, 0, 1]);
    }
}
