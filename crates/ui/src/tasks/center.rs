use std::cell::Cell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::time::Instant;

use chrono::{DateTime, Utc};
use gpui::{
    AnyElement, App, ClickEvent, Context, CursorStyle, DragMoveEvent, Edges, Entity,
    EntityInputHandler, EventEmitter, FocusHandle, Focusable, Hsla, IntoElement, KeyDownEvent,
    MouseButton, MouseDownEvent, Pixels, Point, Render, ScrollHandle, SharedString, Subscription,
    Window, div, prelude::*, px, relative,
};
use zeron_proto::{
    ChatIndicator, DelegationStatus, PlanProgress, StepOutcome, StepStatus, TaskPatch,
    TaskStepPatch,
};

use super::images::{self, ImageGallery};
use super::morph::{self, Key, Painted};
use super::{Rollup, StepRow, chat_brand, linked_chat, rollup, step_chat};
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

mod spine;

const EDIT_GROW: MotionSpec = MotionSpec::new(280, motion::EASE_DRAWER);
const EDIT_LEAVE: MotionSpec = MotionSpec::new(220, motion::EASE_DRAWER);
const EDIT_FADE: MotionSpec = MotionSpec::new(150, motion::LINEAR);

pub enum TaskCenterEvent {
    Update {
        task_id: String,
        patch: TaskPatch,
    },
    ApprovePlan(String),
    TakeBackApproval(String),
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
    Node(String),
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
    fade: Tween,
    uploads: Vec<gpui::Task<()>>,
    _subscriptions: Vec<Subscription>,
}

enum Ask {
    Loading { _fetch: gpui::Task<()> },
    Ready(Vec<String>),
}

struct Leave {
    step: String,
    from: f32,
    grow: Tween,
    fade: Tween,
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
    boxes: Vec<(f32, f32)>,
    origin: (f32, f32),
    start: f32,
    dy: f32,
    over: usize,
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

#[derive(Clone)]
enum MenuFor {
    Step(StepActions),
    Mission { lead: Option<String> },
}

struct StepMenu {
    position: Point<Pixels>,
    of: MenuFor,
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
    OpenLead,
    FoldSteps,
}

struct Thread {
    agent: Option<String>,
    delegation: Option<(DelegationStatus, Option<String>)>,
    preview: Option<String>,
    since: Option<DateTime<Utc>>,
}

pub struct TaskCenter {
    state: Entity<AppState>,
    task_id: String,
    selected: Selection,
    draft: Draft,
    sent: Option<Draft>,
    expanded: HashMap<String, spine::Spec>,
    editor: Option<Editor>,
    leaving: Option<Leave>,
    add: Entity<ComposerInput>,
    live: Option<LiveThread>,
    tree: super::tree::TreeMotion,
    stats: super::tree::StatsCache,
    drawer: Tween,
    drawer_open: bool,
    overview_open: bool,
    overview_turn: Tween,
    folded: bool,
    fold_turn: Tween,
    pop: Option<spine::Pop>,
    painted: Painted,
    scroll: ScrollHandle,
    menu: popover::Popup<StepMenu>,
    drag: Option<StepDrag>,
    press: f32,
    gallery: Entity<ImageGallery>,
    picking: Option<gpui::Task<()>>,
    clock: Option<(Instant, gpui::Task<()>)>,
    asks: HashMap<String, Ask>,
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
            clock: None,
            asks: HashMap::new(),
            state,
            task_id,
            selected: Selection::Lead,
            draft: Draft::default(),
            sent: None,
            expanded: HashMap::new(),
            editor: None,
            leaving: None,
            add,
            live: None,
            tree: super::tree::TreeMotion::solid(),
            stats: Default::default(),
            drawer: Tween::at(0.0, Instant::now()),
            drawer_open: false,
            overview_open: false,
            overview_turn: Tween::at(1.0, Instant::now()),
            folded: false,
            fold_turn: Tween::at(0.0, Instant::now()),
            pop: None,
            painted: Painted::default(),
            scroll: ScrollHandle::new(),
            menu: popover::Popup::default(),
            drag: None,
            press: 0.0,
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
                    .with_wrapping()
                    .with_text_metrics(13.0, 18.0)
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
            if matches!(target, EditTarget::Step(_)) {
                input = input.with_text_metrics(12.5, 18.0);
            }
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
        let now = Instant::now();
        let reduced = motion::reduced_motion(cx);
        let from = match &target {
            EditTarget::Overview => {
                let mut painted = self.painted.borrow_mut();
                painted.remove(&Key::Editor);
                painted
                    .get(&Key::Overview)
                    .map_or(0.0, |rect| f32::from(rect.size.height))
            }
            EditTarget::Step(id) => {
                let from = self.last_region(id);
                self.leaving = self.leaving.take().filter(|leave| &leave.step != id);
                self.open_spec(id, true, reduced, now);
                if let Some(spec) = self.expanded.get_mut(id) {
                    spec.fold.tween = Tween::at(1.0, now);
                    spec.edit.set(0.0);
                }
                from
            }
        };
        let run = |spec: MotionSpec| {
            let mut tween = Tween::at(0.0, now);
            tween.retarget(1.0, spec, reduced, now);
            tween
        };
        self.editor = Some(Editor {
            target,
            title,
            body,
            images: text.images,
            started,
            from,
            grow: run(EDIT_GROW),
            fade: run(EDIT_FADE),
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
                    center.open_spec(&step_id, true, motion::reduced_motion(cx), Instant::now());
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

    fn end_edit(&mut self, cx: &mut Context<Self>) -> Option<Editor> {
        let editor = self.editor.take()?;
        if let EditTarget::Step(id) = &editor.target {
            let now = Instant::now();
            let reduced = motion::reduced_motion(cx);
            let run = |spec: MotionSpec| {
                let mut tween = Tween::at(0.0, now);
                tween.retarget(1.0, spec, reduced, now);
                tween
            };
            self.leaving = Some(Leave {
                step: id.clone(),
                from: self.last_region(id),
                grow: run(EDIT_LEAVE),
                fade: run(EDIT_FADE),
            });
        }
        Some(editor)
    }

    fn commit_edit(&mut self, cx: &mut Context<Self>) {
        let Some(editor) = self.end_edit(cx) else {
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
        self.end_edit(cx);
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
        if self
            .editor
            .as_ref()
            .is_some_and(|editor| editor.target == EditTarget::Step(step_id.to_string()))
        {
            return;
        }
        let open = !self.spec_open(step_id);
        self.open_spec(step_id, open, motion::reduced_motion(cx), Instant::now());
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
        let rects = shown
            .iter()
            .map(|(row, ..)| painted.get(&Key::Task(row.step.id.clone())).copied())
            .collect::<Option<Vec<_>>>()?;
        let steps = painted.get(&Key::Steps)?;
        Some(StepDrag {
            id: step_id.to_string(),
            ids: shown.iter().map(|(row, ..)| row.step.id.clone()).collect(),
            from,
            range,
            rows: rects
                .iter()
                .map(|rect| (f32::from(rect.top()), f32::from(rect.size.height)))
                .collect(),
            boxes: rects
                .iter()
                .map(|rect| (f32::from(rect.left()), f32::from(rect.size.width)))
                .collect(),
            origin: (f32::from(steps.left()), f32::from(steps.top())),
            start: self.press,
            dy: 0.0,
            over: from,
        })
    }

    fn drag_to(&mut self, step_id: &str, y: f32, cx: &mut Context<Self>) {
        if self.drag.as_ref().is_none_or(|drag| drag.id != step_id) {
            self.drag = self.begin_drag(step_id, cx);
        }
        let Some(drag) = &mut self.drag else {
            return;
        };
        let (lo, hi) = drag.range;
        let (top, height) = drag.rows[drag.from];
        let first = drag.rows[lo.min(drag.from)];
        let last = drag.rows[hi.max(drag.from)];
        drag.dy = (y - drag.start).clamp(first.0 - top - 6.0, last.0 + last.1 - top - height + 6.0);
        drag.over = drop_index(&drag.rows, drag.from, drag.range, drag.dy);
        cx.notify();
    }

    fn drop_step(&mut self, cx: &mut Context<Self>) {
        let Some(drag) = self.drag.take() else {
            return;
        };
        if drag.over != drag.from {
            let place = Place::Beside {
                anchor: drag.ids[drag.over].clone(),
                after: drag.over > drag.from,
            };
            self.move_step(&drag.id, place, cx);
        }
        cx.notify();
    }

    fn open_menu(
        &mut self,
        of: MenuFor,
        position: Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        window.focus(&self.focus, cx);
        self.menu.open(StepMenu { position, of });
        cx.notify();
    }

    fn close_menu(&mut self, cx: &mut Context<Self>) {
        if self.menu.begin_close() {
            popover::reap_popup(cx, |center: &mut Self| &mut center.menu);
            cx.notify();
        }
    }

    fn menu_action(&mut self, action: MenuAction, window: &mut Window, cx: &mut Context<Self>) {
        let Some(of) = self.menu.as_open().map(|menu| menu.of.clone()) else {
            return;
        };
        self.close_menu(cx);
        let actions = match of {
            MenuFor::Step(actions) => actions,
            MenuFor::Mission { lead } => {
                match action {
                    MenuAction::OpenLead => {
                        if let Some(chat) = lead {
                            cx.emit(TaskCenterEvent::OpenChat(chat));
                        }
                    }
                    MenuAction::FoldSteps => self.toggle_steps(cx),
                    _ => {}
                }
                return;
            }
        };
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
            MenuAction::OpenLead | MenuAction::FoldSteps => {}
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
                    agent: agent_of(chat, cx),
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

    fn question(&self, row: &StepRow) -> Option<(&str, usize)> {
        let chat = row.step.chat_id.as_deref()?;
        match self.asks.get(chat)? {
            Ask::Ready(questions) => questions
                .first()
                .map(|first| (first.as_str(), questions.len() - 1)),
            Ask::Loading { .. } => None,
        }
    }

    fn ask_count(&self, row: &StepRow) -> Option<usize> {
        match row
            .step
            .chat_id
            .as_deref()
            .and_then(|chat| self.asks.get(chat))
        {
            Some(Ask::Ready(questions)) => Some(questions.len().max(1)),
            Some(Ask::Loading { .. }) => None,
            None => Some(1),
        }
    }

    fn step_note(&self, row: &StepRow, thread: Option<&Thread>) -> Option<String> {
        let delegation = thread.and_then(|t| t.delegation.as_ref());
        let (status, _) = super::tree::Status::of_step(row, delegation.map(|d| d.0));
        match status {
            super::tree::Status::Failed => delegation
                .filter(|(status, _)| retryable(*status))
                .and_then(|(_, summary)| summary.clone())
                .or_else(|| thread.and_then(|t| t.preview.clone())),
            super::tree::Status::Waiting => self.question(row).map(|(question, more)| match more {
                0 => question.to_string(),
                more => format!("{question} +{more}"),
            }),
            _ => None,
        }
    }

    fn sync_asks(
        &mut self,
        shown: &[(StepRow, Option<Tag>, Option<Thread>)],
        cx: &mut Context<Self>,
    ) {
        let awaiting: Vec<String> = shown
            .iter()
            .filter(|(row, _, thread)| {
                thread.is_some() && row.indicator == Some(ChatIndicator::AwaitingInput)
            })
            .filter_map(|(row, ..)| row.step.chat_id.clone())
            .collect();
        self.asks.retain(|chat, _| awaiting.contains(chat));
        let Some(engine) = self.state.read(cx).engine().cloned() else {
            return;
        };
        for chat in awaiting {
            if self.asks.contains_key(&chat) {
                continue;
            }
            let id = chat.clone();
            let engine = engine.clone();
            let fetch = cx.spawn(async move |center, cx| {
                let questions =
                    crate::composer::pending_questions(&engine, &id, cx.background_executor())
                        .await
                        .unwrap_or_default()
                        .into_iter()
                        .map(|question| {
                            if question.question.trim().is_empty() {
                                question.header
                            } else {
                                question.question
                            }
                        })
                        .collect();
                center
                    .update(cx, |center, cx| {
                        if let Some(ask) = center.asks.get_mut(&id) {
                            *ask = Ask::Ready(questions);
                            cx.notify();
                        }
                    })
                    .ok();
            });
            self.asks.insert(chat, Ask::Loading { _fetch: fetch });
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
                    .aria_label("Back to Tasks")
                    .rounded_full()
                    .on_click(
                        cx.listener(|_, _: &ClickEvent, _, cx| cx.emit(TaskCenterEvent::Back)),
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
                                .aria_label("Dismiss")
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
        let working = plan.lead == Some(ChatIndicator::Working);
        if task.completed_at.is_some() {
            "Done".into()
        } else if plan.lead.is_none() {
            "Not started".into()
        } else if working && plan.steps.is_empty() {
            "Planning".into()
        } else if plan.lead == Some(ChatIndicator::Errored) {
            "Failed".into()
        } else if let Some(waits) = Self::lead_waits(task, plan).filter(|_| !working) {
            waits
        } else if running > 0 {
            format!("Watching {running} {}", plural(running, "step", "steps"))
        } else if working {
            "Working".into()
        } else {
            "Idle".into()
        }
    }

    pub(super) fn lead_waits(task: &zeron_proto::Task, plan: &Rollup) -> Option<String> {
        let mut waits = Vec::new();
        if plan.phase(task) == super::PlanPhase::PlanReady {
            waits.push("Plan review".to_owned());
        }
        let questions = plan
            .questions
            .max(usize::from(plan.lead == Some(ChatIndicator::AwaitingInput)));
        if questions > 0 {
            waits.push(format!(
                "{questions} {}",
                plural(questions, "question", "questions")
            ));
        }
        (!waits.is_empty()).then(|| waits.join(", "))
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

    fn shimmer(&self, working: bool, cx: &mut Context<Self>) -> Option<f32> {
        (working && !motion::reduced_motion(cx)).then(|| {
            motion::pulse_lease(cx.entity_id(), cx);
            super::tree::title_phase(self.shimmer_since, Instant::now())
        })
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

    fn edit_notes(
        &self,
        editor: &Editor,
        min: f32,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let body = editor.body.clone();
        div()
            .id("task-center-edit-body")
            .debug_selector(|| "task-center-edit-body".into())
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
                min,
                260.0,
                cx,
            ))
            .into_any_element()
    }

    fn edit_tray(&self, editor: &Editor, theme: &Theme, cx: &mut Context<Self>) -> gpui::Div {
        let paths = editor.images.clone();
        let open_paths = paths.clone();
        div()
            .debug_selector(|| "task-center-edit-tray".into())
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
            })
    }

    fn edit_footer(&self, theme: &Theme, cx: &mut Context<Self>) -> gpui::Div {
        let hint = if cfg!(target_os = "macos") {
            "⌘↵ save · Esc cancel"
        } else {
            "Ctrl+↵ save · Esc cancel"
        };
        div()
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
                    .aria_label("Cancel")
                    .min_h(px(24.0))
                    .rounded_full()
                    .on_click(cx.listener(|center, _: &ClickEvent, _, cx| center.cancel_edit(cx))),
            )
            .child(
                widgets::text_action(theme, widgets::ActionTone::Solid, "Save")
                    .id("task-center-edit-save")
                    .debug_selector(|| "task-center-edit-save".into())
                    .role(gpui::Role::Button)
                    .aria_label("Save")
                    .min_h(px(24.0))
                    .rounded_full()
                    .on_click(cx.listener(|center, _: &ClickEvent, _, cx| center.commit_edit(cx))),
            )
    }

    fn edit_events<E: InteractiveElement>(
        &self,
        el: E,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> E {
        let accent = theme.accent;
        el.drag_over::<gpui::ExternalPaths>(move |style, _, _, _| style.border_color(accent))
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
    }

    fn edit_card(
        &self,
        editor: &Editor,
        head: AnyElement,
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
        let content = div()
            .relative()
            .flex_none()
            .flex()
            .flex_col()
            .pl(px(12.0))
            .pr(px(12.0))
            .py(px(10.0))
            .child(morph::measure(&self.painted, Key::Editor, Edges::default()))
            .child(head)
            .child(
                div()
                    .pt(px(8.0))
                    .child(self.edit_notes(editor, 18.0, theme, cx)),
            )
            .child(div().pt(px(10.0)).child(self.edit_tray(editor, theme, cx)))
            .child(self.edit_footer(theme, cx).mt(px(12.0)).pt(px(9.0)));
        let card = div()
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
            .when_some(height, |el, height| el.h(px(height)));
        self.edit_events(card, theme, cx)
            .child(content)
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
                        .aria_label("Undo")
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
                        .aria_label("Send to lead")
                        .min_h(px(26.0))
                        .rounded_full()
                        .on_click(cx.listener(|center, _: &ClickEvent, _, cx| {
                            center.send_edits(false, cx)
                        })),
                )
                .into_any_element(),
        )
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
            Selection::Lead | Selection::Node(_) => None,
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
                .aria_label("Open thread")
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
            .pl(px(16.0))
            .pr(px(48.0))
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

pub(super) fn agent_of(chat: &zeron_proto::Chat, cx: &App) -> Option<String> {
    let config = chat.config.as_ref()?;
    let harness = config.harness.display_name();
    Some(match config.model.as_deref() {
        Some(model) => format!(
            "{harness} · {}",
            crate::pickers::model_display_label(config.harness, model, cx)
        ),
        None => harness.to_string(),
    })
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
        let card = popover::popover_card(&theme)
            .w(px(180.0))
            .flex()
            .flex_col()
            .on_mouse_down_out(cx.listener(|center, _, _, cx| center.close_menu(cx)))
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation());
        let card = match &menu.of {
            MenuFor::Mission { lead } => card
                .children(lead.is_some().then(|| {
                    self.menu_item(
                        &theme,
                        "task-center-menu-open-lead",
                        "Open lead thread",
                        true,
                        false,
                        MenuAction::OpenLead,
                        cx,
                    )
                }))
                .child(self.menu_item(
                    &theme,
                    "task-center-menu-fold",
                    if self.folded {
                        "Show the steps"
                    } else {
                        "Fold the steps"
                    },
                    true,
                    false,
                    MenuAction::FoldSteps,
                    cx,
                )),
            MenuFor::Step(actions) => {
                let outcome = match actions.outcome {
                    Outcome::Reopen => "Reopen",
                    Outcome::Skip | Outcome::Locked => "Skip",
                };
                card.child(self.menu_item(
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
                ))
            }
        };
        Some(popover::menu_at(
            "task-center-step-menu",
            menu.position,
            card.into_any_element(),
            self.menu.closing_since(),
        ))
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
        let reduced = motion::reduced_motion(cx);
        let now = Instant::now();
        if self.drag.is_some() && !cx.has_active_drag() {
            self.drag = None;
        }
        let facts = self.facts(&task, &plan, &shown, cx);
        if task.completed_at.is_none() && self.clock.as_ref().is_none_or(|(due, _)| now >= *due) {
            let tick = std::time::Duration::from_secs(1);
            self.clock = Some((
                now + tick,
                cx.spawn(async move |center, cx| {
                    cx.background_executor().timer(tick).await;
                    center.update(cx, |_, cx| cx.notify()).ok();
                }),
            ));
        }
        let canvas = self.render_spine(
            &task, &plan, &shown, &facts, editable, reduced, now, &theme, window, cx,
        );
        if self.spine_animating(now) {
            window.request_animation_frame();
        }
        let list = div()
            .id("task-center-plan")
            .debug_selector(|| "task-center-plan".into())
            .absolute()
            .inset_0()
            .overflow_y_scroll()
            .track_scroll(&self.scroll)
            .pt(px(14.0))
            .pb(px(96.0))
            .pl(px(20.0))
            .pr(px(self.canvas_pad()))
            .child(canvas);
        let live = self.render_live(&task, &plan, &theme, cx);
        let drawer = self.render_drawer(live, now, &theme, cx);
        let body = div()
            .flex_1()
            .min_h_0()
            .relative()
            .flex()
            .flex_col()
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .relative()
                    .child(morph::measure(&self.painted, Key::Canvas, Edges::default()))
                    .child(list),
            )
            .children(
                editable
                    .then(|| self.render_edit_bar(&theme, cx))
                    .flatten()
                    .map(|bar| {
                        div()
                            .flex_none()
                            .pr(px(self.canvas_pad() - spine::CANVAS_PAD))
                            .child(bar)
                    }),
            )
            .children(drawer);
        div()
            .id("task-center")
            .debug_selector(|| "task-center".into())
            .track_focus(&self.focus)
            .on_key_down(cx.listener(|center, event: &KeyDownEvent, _, cx| {
                if event.keystroke.key != "escape" {
                    return;
                }
                if center.menu.is_open() {
                    cx.stop_propagation();
                    center.close_menu(cx);
                } else if center.close_pop(cx) {
                    cx.stop_propagation();
                } else if center.drawer_open && center.editor.is_none() {
                    cx.stop_propagation();
                    center.close_drawer(cx);
                }
            }))
            .relative()
            .size_full()
            .pt(px(Theme::TITLEBAR_HEIGHT))
            .flex()
            .flex_col()
            .child(self.render_header(&task, &plan, &facts, reduced, now, &theme, cx))
            .children(self.render_send_error(&task, &theme, cx))
            .child(body)
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
