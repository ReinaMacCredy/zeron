mod feed;
mod inbox;
mod rail;

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::time::{Duration, Instant};

use chrono::{DateTime, Local, Utc};
use gpui::{
    AnyElement, App, ClickEvent, Context, Entity, EventEmitter, FocusHandle, Hsla, IntoElement,
    Render, SharedString, Subscription, Task, Window, canvas, div, prelude::*, px, relative,
};
use zeron_doc::{MessagePart, MessageRole, SessionMessageEntry};
use zeron_proto::supervisor::{
    CheckTool, InputAsker, InterventionChip, InterventionKind, InterventionTrigger, ItemWhy,
    LeadQuestion, NeedsYouInput, NeedsYouRow, NextCheckKind, PlanApproval, QuestionState, Raiser,
    ResourceState, SettledBy, SharedResource, SupervisorEventKind, SupervisorIntervention,
    SupervisorItem, SupervisorSettings, SupervisorSnapshot, SupervisorState, SupervisorWake,
    WAKES_PER_HOUR, WakeDelivery, is_supervisor_chat,
};
use zeron_proto::{
    AgentProfile, Chat, ChatConfig, ChatIndicator, PermissionMode, PlanProgress, SandboxLevel,
    StepStatus, TaskStatus, UserInputAnswer, UserInputQuestion,
};
use zeron_rpc::methods;

use crate::composer::Composer;
use crate::icons::{self, icon};
use crate::motion::{self, CubicBezier, Fold, MotionSpec, Tween, folded};
use crate::pickers::Pickers;
use crate::popover::{self, Loadable, Popup};
use crate::settings::widgets;
use crate::state::AppState;
use crate::tasks::{Rollup, TaskCenter, linked_chat, rollup, step_chat};
use crate::theme::Theme;
use crate::transcript::Transcript;
use crate::typography::ui_rems;
use feed::{Entry, Feed, Filter, Kind, Memory, Row};
use rail::{RailMode, RailMotion, mission_tone};

pub(crate) const PLACEHOLDER: &str = "Talk to the Supervisor about architecture and direction…";
const PROFILE_ID: &str = "slp-supervisor";
const NOT_CONNECTED: &str = "Not connected to the engine";
const WATCH_EVERYTHING: &str = "everything";
const NO_AGENT: &str =
    "no agent is installed and turned on. Turn one on in Settings > Providers, then try again.";
const HEARTBEATS: [(u32, &str); 3] = [(0, "Off"), (30, "30 min"), (60, "60 min")];
const DRAWER_WIDTH: f32 = 380.0;
const MESSAGE_CHARS: usize = 600;
const TITLE_CHARS: usize = 120;
const RECENTLY_ACTIVE: Duration = Duration::from_secs(180);
const NO_EFFECT_AFTER_MINUTES: i64 = 10;
const EARLIER_ROWS: usize = 50;
const CONVERSATION_HEIGHT: f32 = 320.0;
const PAGE_WIDTH: f32 = 760.0;
const EASE_PAGE: CubicBezier = CubicBezier::new(0.2, 0.9, 0.3, 1.0);
const ENTER: MotionSpec = MotionSpec::new(340, EASE_PAGE);
const ENTER_STAGGER_MS: u64 = 45;
const FILL: MotionSpec = MotionSpec::new(1000, EASE_PAGE);
const ROW_FOLD: MotionSpec = MotionSpec::new(220, EASE_PAGE);
const ARRIVE: MotionSpec = MotionSpec::new(220, EASE_PAGE);
const FOLD: MotionSpec = MotionSpec::new(300, EASE_PAGE);
const MISSION_FOLD: MotionSpec = MotionSpec::new(220, EASE_PAGE);
const CHEVRON_TURN: MotionSpec = MotionSpec::new(220, EASE_PAGE);
const DRAWER_SLIDE: MotionSpec = MotionSpec::new(300, EASE_PAGE);
const CHIP_FADE: MotionSpec = MotionSpec::new(300, motion::EASE);
const COUNT_TICK: MotionSpec = MotionSpec::new(220, EASE_PAGE);
const TAB_SLIDE: MotionSpec = MotionSpec::new(200, EASE_PAGE);
const LIST_OUT: MotionSpec = MotionSpec::new(100, motion::EASE_OUT);
const LIST_IN: MotionSpec = MotionSpec::new(160, motion::EASE_OUT);
const CONFIRM: MotionSpec = motion::FADE_QUICK;
const HOLD: Duration = Duration::from_millis(450);
const UNDO_WINDOW: Duration = Duration::from_secs(2);
const RAIL_OUT: MotionSpec = MotionSpec::new(100, motion::EASE_OUT);
const RAIL_WIDTH: MotionSpec = MotionSpec::new(220, EASE_PAGE);
const RAIL_IN: MotionSpec = MotionSpec::new(160, motion::EASE_OUT);
const STATUS_GROW: MotionSpec = MotionSpec::new(220, EASE_PAGE);
const STATUS_FADE: MotionSpec = MotionSpec::new(160, motion::EASE_OUT);
const DOT_FLASH: MotionSpec = MotionSpec::new(900, motion::EASE_OUT);
pub(crate) const BADGE_POP: MotionSpec = MotionSpec::new(400, motion::LINEAR);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum NeedsYouRoute {
    Quiet,
    InApp,
    Banner,
}

pub(crate) fn needs_you_route(
    looking: bool,
    focused: bool,
    since_interaction: Option<Duration>,
) -> NeedsYouRoute {
    if looking && focused {
        return NeedsYouRoute::Quiet;
    }
    match since_interaction {
        Some(since) if since <= RECENTLY_ACTIVE => NeedsYouRoute::InApp,
        _ => NeedsYouRoute::Banner,
    }
}

pub(crate) fn row_key(row: &NeedsYouRow) -> String {
    match row {
        NeedsYouRow::Item(item) => format!("item:{}", item.id),
        NeedsYouRow::Input(input) => format!("input:{}:{}", input.chat_id, input.request_id),
    }
}

pub(crate) fn row_origin(row: &NeedsYouRow) -> (Option<&str>, &str) {
    match row {
        NeedsYouRow::Item(item) => (item.task_id.as_deref(), item.raiser_chat_id.as_str()),
        NeedsYouRow::Input(input) => (input.task_id.as_deref(), input.chat_id.as_str()),
    }
}

pub(crate) fn row_title(row: &NeedsYouRow) -> String {
    match row {
        NeedsYouRow::Item(item) => clip_to(&item.title, TITLE_CHARS),
        NeedsYouRow::Input(input) => input_title(&input.questions),
    }
}

fn input_title(questions: &[UserInputQuestion]) -> String {
    let Some(first) = questions.first() else {
        return "A question".into();
    };
    let text = if first.header.trim().is_empty() {
        &first.question
    } else {
        &first.header
    };
    clip_to(&crate::transcript::single_line(text), TITLE_CHARS)
}

pub(crate) fn fresh_needs_you(
    seen: &mut Option<HashSet<String>>,
    rows: &[NeedsYouRow],
) -> Vec<NeedsYouRow> {
    let fresh = match seen.as_ref() {
        Some(seen) => rows
            .iter()
            .filter(|row| !seen.contains(&row_key(row)))
            .cloned()
            .collect(),
        None => Vec::new(),
    };
    *seen = Some(rows.iter().map(row_key).collect());
    fresh
}

pub(crate) fn badge_scale(t: f32) -> f32 {
    let rise = 0.4;
    let peak = if t < rise {
        motion::EASE_OUT.eval(t / rise)
    } else {
        1.0 - motion::EASE_OUT.eval((t - rise) / (1.0 - rise))
    };
    1.0 + 0.35 * peak
}

fn no_effect_since(at: DateTime<Utc>) -> DateTime<Utc> {
    at + chrono::Duration::minutes(NO_EFFECT_AFTER_MINUTES)
}

pub(crate) fn no_effect_yet(row: &SupervisorIntervention, now: DateTime<Utc>) -> bool {
    matches!(row.chips.as_slice(), [InterventionChip::Sent]) && now > no_effect_since(row.at)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ApprovalState {
    Held { secs: i64 },
    Sent,
    Started,
    TakenBack,
}

impl ApprovalState {
    pub(crate) fn line(self) -> String {
        match self {
            Self::Held { secs } => format!(
                "Approved by the Supervisor · starts in {}:{:02}",
                secs / 60,
                secs % 60
            ),
            Self::Sent => "Approved by the Supervisor · no step has started yet".into(),
            Self::Started => "Approved by the Supervisor · steps already started. To change course, edit the plan in Mission Control or message the lead.".into(),
            Self::TakenBack => "You took this approval back · the plan went back to review".into(),
        }
    }

    pub(crate) fn can_take_back(self) -> bool {
        matches!(self, Self::Held { .. } | Self::Sent)
    }
}

pub(crate) fn approval_state(
    row: &SupervisorIntervention,
    state: &AppState,
    now: DateTime<Utc>,
) -> Option<ApprovalState> {
    let task_id = row.task_id.as_deref()?;
    let started = || {
        state.task_steps.iter().any(|step| {
            step.task_id == task_id
                && !step.deleted
                && (step.chat_id.is_some() || step.outcome.is_some())
        })
    };
    Some(match row.approval.as_ref()? {
        PlanApproval::TakenBack { .. } => ApprovalState::TakenBack,
        _ if started() => ApprovalState::Started,
        PlanApproval::Held { starts_at } => ApprovalState::Held {
            secs: ((*starts_at - now).num_milliseconds().max(0) + 999) / 1000,
        },
        PlanApproval::Sent => ApprovalState::Sent,
    })
}

/// The Supervisor's newest approval of the mission's plan, as the person can still act on it.
pub(crate) fn plan_approval(
    state: &AppState,
    task_id: &str,
    now: DateTime<Utc>,
) -> Option<ApprovalState> {
    let row = state
        .supervisor
        .as_ref()?
        .interventions
        .iter()
        .find(|row| {
            row.kind == InterventionKind::Approve && row.task_id.as_deref() == Some(task_id)
        })?;
    approval_state(row, state, now)
}

fn take_back_key(task_id: &str) -> String {
    format!("take-back:{task_id}")
}

fn next_effect_check(rows: &[SupervisorIntervention], now: DateTime<Utc>) -> Option<DateTime<Utc>> {
    rows.iter()
        .filter(|row| matches!(row.chips.as_slice(), [InterventionChip::Sent]))
        .map(|row| no_effect_since(row.at))
        .filter(|at| *at > now)
        .min()
}

pub enum SupervisorEvent {
    OpenTaskCenter(String),
    OpenMission(String, crate::tasks::Selection),
    OpenChat(String),
    WatchDoc(Option<String>),
    Discuss(String),
}

#[derive(Debug, Clone, PartialEq)]
enum Stage {
    Held,
    Sending,
    Sent(Instant),
    Failed(String),
}

#[derive(Debug, Clone, PartialEq)]
struct Reply {
    choice: String,
    flipped: Instant,
    stage: Stage,
}

impl Reply {
    fn failed(&self) -> bool {
        matches!(self.stage, Stage::Failed(_))
    }

    fn holding(&self, now: Instant, reduced: bool) -> bool {
        match self.stage {
            Stage::Sending => true,
            Stage::Sent(at) => !reduced && now < at + HOLD.mul_f32(motion::speed_scale()),
            _ => false,
        }
    }

    fn collapsing(&self, now: Instant, reduced: bool) -> bool {
        matches!(self.stage, Stage::Sent(_)) && !self.holding(now, reduced)
    }
}

pub struct Pane {
    pub transcript: Entity<Transcript>,
    pub composer: Entity<Composer>,
}

#[derive(Debug, Clone, PartialEq)]
enum Ensure {
    Idle,
    Wanted,
    Running,
    Failed(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Owner {
    task_id: String,
    step_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Watch {
    Watched,
    Unwatched,
    Elsewhere(String),
}

pub(crate) fn watch_of(
    task_id: &str,
    host: Option<&str>,
    local: Option<&str>,
    unwatched: &[String],
) -> Watch {
    match (host, local) {
        (Some(host), Some(local)) if host != local => Watch::Elsewhere(host.to_owned()),
        _ if unwatched.iter().any(|id| id == task_id) => Watch::Unwatched,
        _ => Watch::Watched,
    }
}

pub(crate) fn tree_watch(state: &AppState, root: &str) -> Watch {
    let host = state
        .chats
        .iter()
        .find(|chat| chat.id == root)
        .map(|chat| chat.device_id.as_str());
    let local = state.local_device_id.as_deref();
    let snapshot = state.supervisor.as_ref();
    if let Some(task) = crate::tasks::plan_led_by(&state.tasks, root) {
        let unwatched = snapshot.map_or(&[][..], |snapshot| snapshot.unwatched.as_slice());
        return watch_of(&task.id, host, local, unwatched);
    }
    match (host, local) {
        (Some(host), Some(local)) if host != local => Watch::Elsewhere(host.to_owned()),
        _ if snapshot.is_some_and(|snapshot| {
            snapshot.watch_everything || snapshot.watched_roots.iter().any(|id| id == root)
        }) =>
        {
            Watch::Watched
        }
        _ => Watch::Unwatched,
    }
}

pub(crate) fn watched_count(watches: &[Watch]) -> usize {
    watches
        .iter()
        .filter(|watch| **watch == Watch::Watched)
        .count()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Sender {
    You,
    Lead,
    Supervisor,
    Step(String),
    Itself,
    Thread,
}

impl Sender {
    fn tag(&self) -> SharedString {
        match self {
            Self::You => "From you".into(),
            Self::Lead => "From Lead".into(),
            Self::Supervisor => "From Supervisor".into(),
            Self::Step(label) => format!("From {label}").into(),
            Self::Itself => "Auto-continue".into(),
            Self::Thread => "From another thread".into(),
        }
    }
}

pub(crate) fn sender(
    from: Option<&str>,
    own: &str,
    lead: Option<&str>,
    step_label: &dyn Fn(&str) -> Option<String>,
) -> Sender {
    let Some(from) = from else {
        return Sender::You;
    };
    if is_supervisor_chat(from) {
        Sender::Supervisor
    } else if from == own {
        Sender::Itself
    } else if Some(from) == lead {
        Sender::Lead
    } else {
        step_label(from).map_or(Sender::Thread, Sender::Step)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Received {
    pub sender: Sender,
    pub text: String,
    pub at: Option<DateTime<Utc>>,
}

pub(crate) fn messages_it_got(
    entries: &[SessionMessageEntry],
    own: &str,
    lead: Option<&str>,
    step_label: &dyn Fn(&str) -> Option<String>,
) -> Vec<Received> {
    entries
        .iter()
        .filter(|entry| entry.role == MessageRole::User)
        .map(|entry| Received {
            sender: sender(entry.from_chat_id.as_deref(), own, lead, step_label),
            text: entry
                .parts
                .iter()
                .filter_map(|part| match part {
                    MessagePart::Text { text, .. } => Some(text.trim()),
                    _ => None,
                })
                .filter(|text| !text.is_empty())
                .collect::<Vec<_>>()
                .join("\n\n"),
            at: DateTime::from_timestamp_millis(entry.created_at),
        })
        .collect()
}

fn kind_word(kind: SupervisorEventKind) -> &'static str {
    match kind {
        SupervisorEventKind::StepFailed => "step failed",
        SupervisorEventKind::NeedsYou => "needs you",
        SupervisorEventKind::MissionDone => "mission done",
        SupervisorEventKind::MissionSilent => "mission silent",
        SupervisorEventKind::LeadSilent => "lead silent",
        SupervisorEventKind::Escalation => "lead escalated",
        SupervisorEventKind::PlanEdited => "plan edited",
        SupervisorEventKind::Resource => "resource contention",
        SupervisorEventKind::Heartbeat => "check-in",
        SupervisorEventKind::NodeFailed => "thread failed",
        SupervisorEventKind::NodeFinished => "delegated task ended",
        SupervisorEventKind::QuestionWaiting => "question waiting",
        SupervisorEventKind::Dirty => "uncommitted files",
        SupervisorEventKind::Unmerged => "unmerged branch",
        SupervisorEventKind::LimitIdle => "idle after limit reset",
        SupervisorEventKind::LostWake => "lost wake",
        SupervisorEventKind::MergeBlocked => "merge blocked",
    }
}

fn delivery_label(delivery: &WakeDelivery) -> String {
    match delivery {
        WakeDelivery::Started => "Started a turn".into(),
        WakeDelivery::Steered => "Steered its turn".into(),
        WakeDelivery::HeldForTurnEnd => "Held for turn end".into(),
        WakeDelivery::HeldByStop => "Held by your Stop".into(),
        WakeDelivery::Failed { error } => format!("Failed: {error}"),
        WakeDelivery::Dropped => "Dropped".into(),
    }
}

fn why_label(why: ItemWhy) -> &'static str {
    match why {
        ItemWhy::Goal => "Goal",
        ItemWhy::Spend => "Spend",
        ItemWhy::Decision => "Decision",
    }
}

fn intervention_kind_label(kind: InterventionKind) -> &'static str {
    match kind {
        InterventionKind::Send => "Message",
        InterventionKind::Respond => "Answer",
        InterventionKind::Rebrief => "Re-brief",
        InterventionKind::Start => "Start a mission",
        InterventionKind::Pause => "Pause a step",
        InterventionKind::Resume => "Resume a step",
        InterventionKind::Decide => "Decision",
        InterventionKind::Approve => "Approve a plan",
        InterventionKind::Goal => "Change the goal",
    }
}

fn step_numbers(steps: &[usize]) -> String {
    steps
        .iter()
        .map(usize::to_string)
        .collect::<Vec<_>>()
        .join(", ")
}

fn chip_label(chip: &InterventionChip) -> String {
    match chip {
        InterventionChip::Sent => "Sent".into(),
        InterventionChip::LeadRanTurn => "Lead ran a turn".into(),
        InterventionChip::PlanEdited { steps } => format!(
            "Plan edited on {} {}",
            plural(steps.len(), "step", "steps"),
            step_numbers(steps)
        ),
        InterventionChip::StepCompletedTurn { steps } => match steps.as_slice() {
            [one] => format!("Step {one} thread completed a turn"),
            _ => format!("Step {} threads completed a turn", step_numbers(steps)),
        },
        InterventionChip::StepPaused => "Step paused".into(),
        InterventionChip::ResourceFreed => "Resource freed".into(),
    }
}

fn answers_text(answers: &[UserInputAnswer]) -> String {
    answers
        .iter()
        .map(|answer| answer.labels.join(", "))
        .filter(|text| !text.trim().is_empty())
        .collect::<Vec<_>>()
        .join(" · ")
}

fn question_text(questions: &[UserInputQuestion]) -> String {
    questions
        .iter()
        .map(|question| crate::transcript::single_line(&question.question))
        .collect::<Vec<_>>()
        .join(" · ")
}

fn question_state(state: &QuestionState) -> (&'static str, Option<String>) {
    let settled = |by: &str, answers: &[UserInputAnswer]| match answers_text(answers) {
        text if text.is_empty() => by.to_owned(),
        text => format!("{by}: {text}"),
    };
    match state {
        QuestionState::Open => ("Open", None),
        QuestionState::Settled { by, answers } => (
            "Settled",
            Some(settled(
                match by {
                    SettledBy::Lead => "by the Lead",
                    SettledBy::Supervisor => "by the Supervisor",
                    SettledBy::Outside => "answered outside the Lead",
                },
                answers,
            )),
        ),
        QuestionState::Dropped => ("Dropped", Some("the step's turn ended unanswered".into())),
    }
}

fn step_name(state: &AppState, task_id: &str, step_id: &str, now: DateTime<Utc>) -> Option<String> {
    let task = state.tasks.iter().find(|task| task.id == task_id)?;
    let plan = rollup(state, task, now);
    let row = plan.steps.iter().find(|row| row.step.id == step_id)?;
    Some(format!(
        "{} · {}",
        plan.step_label(step_id)?,
        row.step.title
    ))
}

fn mission_title(state: &AppState, task_id: Option<&str>) -> Option<String> {
    let task_id = task_id?;
    state
        .tasks
        .iter()
        .find(|task| task.id == task_id)
        .map(|task| task.title.clone())
}

fn thread_name(state: &AppState, chat_id: &str, now: DateTime<Utc>) -> String {
    if is_supervisor_chat(chat_id) {
        return "Supervisor".into();
    }
    if let Some(task) = crate::tasks::plan_led_by(&state.tasks, chat_id) {
        return format!("Lead · {}", task.title);
    }
    if let Some(step) = state
        .task_steps
        .iter()
        .find(|step| step.chat_id.as_deref() == Some(chat_id) && !step.deleted)
        && let Some(name) = step_name(state, &step.task_id, &step.id, now)
    {
        let mission = mission_title(state, Some(&step.task_id));
        return match mission {
            Some(mission) => format!("{name} · {mission}"),
            None => name,
        };
    }
    state
        .chats
        .iter()
        .find(|chat| chat.id == chat_id)
        .map(|chat| crate::transcript::single_line(&crate::home::chat_title(chat)))
        .unwrap_or_else(|| "a thread".into())
}

fn asker_name(input: &NeedsYouInput, state: &AppState, now: DateTime<Utc>) -> String {
    let mission = mission_title(state, input.task_id.as_deref());
    let who = match (
        input.asker,
        input.task_id.as_deref(),
        input.step_id.as_deref(),
    ) {
        (InputAsker::Supervisor, _, _) => return "Supervisor".into(),
        (InputAsker::Thread, _, _) => {
            let name = thread_name(state, &input.chat_id, now);
            let root = state
                .chats
                .iter()
                .find(|chat| chat.id == input.chat_id)
                .and_then(|chat| {
                    zeron_proto::tree_path(chat, &state.chats, &state.tasks, &state.task_steps)
                        .last()
                        .filter(|root| root.id != chat.id)
                        .map(|root| crate::transcript::single_line(&crate::home::chat_title(root)))
                });
            match root {
                Some(root) => format!("{name} · {root}"),
                None => name,
            }
        }
        (InputAsker::Step, Some(task_id), Some(step_id)) => {
            step_name(state, task_id, step_id, now).unwrap_or_else(|| "A step".into())
        }
        (InputAsker::Step, _, _) => "A step".into(),
        (InputAsker::Lead, _, _) => "Lead".into(),
    };
    match mission {
        Some(mission) => format!("{who} · {mission}"),
        None => who,
    }
}

fn clock(at: DateTime<Utc>) -> String {
    at.with_timezone(&Local).format("%H:%M").to_string()
}

fn wake_reason(wake: &SupervisorWake) -> String {
    match wake.events.as_slice() {
        [] => "nothing new".into(),
        [one] => kind_word(one.kind).into(),
        [first, rest @ ..] => format!("{} and {} more", kind_word(first.kind), rest.len()),
    }
}

fn live_state(snapshot: Option<&SupervisorSnapshot>) -> SupervisorState {
    snapshot
        .filter(|snapshot| snapshot.chat_id.is_some())
        .map_or(SupervisorState::Off, |snapshot| snapshot.state)
}

#[derive(Debug, Clone, PartialEq)]
enum Mode {
    Watching,
    Working,
    Stopped,
    Off,
    Failed(String),
}

fn mode(snapshot: Option<&SupervisorSnapshot>, ensure: &Ensure) -> Mode {
    match (live_state(snapshot), ensure) {
        (SupervisorState::Off, Ensure::Failed(error)) => Mode::Failed(error.clone()),
        (SupervisorState::Off, _) => Mode::Off,
        (SupervisorState::Asleep, _) => Mode::Watching,
        (SupervisorState::Awake, _) => Mode::Working,
        (SupervisorState::Stopped, _) => Mode::Stopped,
    }
}

#[derive(Debug, Clone, PartialEq)]
enum Part {
    Text(String),
    Strong(String),
    Chip(Face),
}

fn text(value: impl Into<String>) -> Part {
    Part::Text(value.into())
}

fn wakes_this_hour(wakes: &[SupervisorWake], now: DateTime<Utc>) -> usize {
    wakes
        .iter()
        .filter(|wake| wake.woke() && now - wake.at < chrono::Duration::hours(1))
        .count()
}

fn target_name(state: &AppState, chat_id: &str, now: DateTime<Utc>) -> String {
    if crate::tasks::plan_led_by(&state.tasks, chat_id).is_some() {
        return "the Lead".into();
    }
    let step = state
        .task_steps
        .iter()
        .find(|step| step.chat_id.as_deref() == Some(chat_id) && !step.deleted);
    let number = step.and_then(|step| {
        let task = state.tasks.iter().find(|task| task.id == step.task_id)?;
        let plan = rollup(state, task, now);
        plan.steps.iter().position(|row| row.step.id == step.id)
    });
    match number {
        Some(ix) => format!("step {}", ix + 1),
        None => thread_name(state, chat_id, now),
    }
}

fn action_text(row: &SupervisorIntervention, state: &AppState, now: DateTime<Utc>) -> String {
    let who = row
        .target
        .as_deref()
        .map_or_else(|| "a thread".into(), |id| target_name(state, id, now));
    match row.kind {
        InterventionKind::Send => format!("messaged {who}"),
        InterventionKind::Respond => format!("answered {who}"),
        InterventionKind::Rebrief => format!("re-briefed {who}"),
        InterventionKind::Start => match mission_title(state, row.task_id.as_deref()) {
            Some(title) => format!("started {title}"),
            None => "started a mission".into(),
        },
        InterventionKind::Pause => format!("paused {who}"),
        InterventionKind::Resume => format!("resumed {who}"),
        InterventionKind::Decide => format!("decided for {who}"),
        InterventionKind::Approve => format!("approved the plan of {who}"),
        InterventionKind::Goal => format!("changed the goal of {who}"),
    }
}

fn last_parts(snapshot: &SupervisorSnapshot, state: &AppState, now: DateTime<Utc>) -> Vec<Part> {
    let Some(wake) = snapshot.wakes.iter().find(|wake| wake.woke()) else {
        return vec![text("No wakes yet")];
    };
    let acted = snapshot.interventions.iter().find(|row| {
        matches!(&row.trigger, Some(InterventionTrigger::Wake { wake_id }) if *wake_id == wake.id)
    });
    let head = format!("{} · {} →", clock(wake.at), wake_reason(wake));
    match acted {
        Some(row) => {
            let mut parts = vec![text(head), text(action_text(row, state, now))];
            if let Some(face) = chip_face(row, now) {
                parts.push(text("·"));
                parts.push(Part::Chip(face));
            }
            parts
        }
        None => vec![text(head), text("no action")],
    }
}

fn next_parts(snapshot: &SupervisorSnapshot, state: &AppState) -> Vec<Part> {
    let Some(check) = &snapshot.next_check else {
        return vec![text("the next event from a watched mission")];
    };
    let at = clock(check.at);
    let on = |what: &str, title: Option<String>, tail: String| match title {
        Some(title) => vec![text(format!("{what} on")), Part::Strong(title), text(tail)],
        None => vec![text(format!("{what} {tail}"))],
    };
    let title = mission_title(state, check.task_id.as_deref());
    match check.kind {
        NextCheckKind::MissionSilent => {
            on("silence check", title, format!("at {at} if it stays quiet"))
        }
        NextCheckKind::LeadSilent => on(
            "Lead check",
            title,
            format!("at {at} if the Lead stays silent"),
        ),
        NextCheckKind::Heartbeat => vec![text(format!("check-in at {at}"))],
        NextCheckKind::HeldWake => vec![text(format!("held events wake it at {at}"))],
        NextCheckKind::Resource => vec![text(format!("resource check at {at}"))],
    }
}

fn current_wake<'a>(
    snapshot: &'a SupervisorSnapshot,
    entries: &[SessionMessageEntry],
) -> Option<&'a SupervisorWake> {
    let asked = entries
        .iter()
        .rev()
        .find(|entry| entry.role == MessageRole::User)?;
    snapshot.wakes.iter().find(|wake| wake.id == asked.id)
}

fn woke_parts(wake: &SupervisorWake, state: &AppState) -> Vec<Part> {
    let head = format!("{} for {}", clock(wake.at), wake_reason(wake));
    let title = mission_title(
        state,
        wake.events
            .first()
            .and_then(|event| event.task_id.as_deref()),
    );
    match title {
        Some(title) => vec![text(head), text("on"), Part::Strong(title)],
        None => vec![text(head)],
    }
}

fn held_wakes(snapshot: &SupervisorSnapshot) -> Vec<&SupervisorWake> {
    let mut held: Vec<&SupervisorWake> = snapshot
        .wakes
        .iter()
        .filter(|wake| wake.delivery == WakeDelivery::HeldByStop)
        .take(snapshot.waiting)
        .collect();
    held.reverse();
    held
}

fn held_line(held: &[&SupervisorWake]) -> String {
    let Some(first) = held.first() else {
        return "Nothing held yet. New wakes wait here until you resume.".into();
    };
    let mut reasons: Vec<&str> = Vec::new();
    for event in held.iter().flat_map(|wake| &wake.events) {
        let word = kind_word(event.kind);
        if !reasons.contains(&word) {
            reasons.push(word);
        }
    }
    match reasons.as_slice() {
        [] => format!("since {}", clock(first.at)),
        reasons => format!("{}, since {}", reasons.join(" · "), clock(first.at)),
    }
}

fn last_message(entries: &[SessionMessageEntry], wakes: &[SupervisorWake]) -> Option<String> {
    entries.iter().rev().find_map(|entry| {
        if let Some(wake) = wakes.iter().find(|wake| wake.id == entry.id) {
            let labels: Vec<&str> = wake.events.iter().map(|e| e.label.as_str()).collect();
            return Some(match labels.as_slice() {
                [] => "Woke".to_owned(),
                labels => format!("Woke: {}", labels.join("; ")),
            });
        }
        let text = entry
            .parts
            .iter()
            .filter_map(|part| match part {
                MessagePart::Text { text, .. } => Some(text.trim()),
                _ => None,
            })
            .find(|text| !text.is_empty())?;
        let who = match (&entry.role, entry.from_chat_id.as_deref()) {
            (MessageRole::User, None) => "You",
            (MessageRole::User, Some(from)) if !is_supervisor_chat(from) => "A thread",
            _ => "Supervisor",
        };
        Some(format!(
            "{who}: {}",
            clip_to(&crate::transcript::single_line(text), TITLE_CHARS)
        ))
    })
}

fn resource_label(state: ResourceState) -> &'static str {
    match state {
        ResourceState::Free => "Free",
        ResourceState::Held => "Held",
        ResourceState::Checking => "Checking…",
        ResourceState::Clear => "Clear",
        ResourceState::Contended => "Contended",
        ResourceState::CheckFailed => "Not verified",
        ResourceState::NotVerified => "Declared by both, not verified",
    }
}

fn verified_line(resource: &SharedResource, title: &dyn Fn(&str) -> String) -> Option<String> {
    let check = resource.check.as_ref()?;
    let at = clock(check.at);
    if let Some(error) = &check.error {
        return Some(format!("Could not verify at {at}: {error}"));
    }
    let place = |task_id: Option<&str>, cwd: Option<&str>| match task_id {
        Some(id) => title(id),
        None => format!("a process in {}", cwd.unwrap_or("an unknown folder")),
    };
    let (tool, found) = match check.tool {
        CheckTool::Lsof if check.uses.is_empty() => ("lsof", "nothing listens on it".to_string()),
        CheckTool::Lsof => (
            "lsof",
            check
                .uses
                .iter()
                .map(|used| {
                    format!(
                        "{} (pid {}) listens · {}",
                        used.command,
                        used.pid,
                        place(used.task_id.as_deref(), used.cwd.as_deref())
                    )
                })
                .collect::<Vec<_>>()
                .join("; "),
        ),
        CheckTool::Ps if check.uses.is_empty() => (
            "ps",
            "no other mission used more than half a core".to_string(),
        ),
        CheckTool::Ps => {
            let mut loads: Vec<(String, f32, String)> = Vec::new();
            for used in &check.uses {
                let who = place(used.task_id.as_deref(), used.cwd.as_deref());
                match loads.iter_mut().find(|(at, _, _)| *at == who) {
                    Some((_, cpu, _)) => *cpu += used.cpu.unwrap_or_default(),
                    None => loads.push((
                        who,
                        used.cpu.unwrap_or_default(),
                        format!("{}, pid {}", used.command, used.pid),
                    )),
                }
            }
            (
                "ps",
                loads
                    .iter()
                    .map(|(who, cpu, top)| format!("{who} used {cpu:.0}% CPU ({top})"))
                    .collect::<Vec<_>>()
                    .join("; "),
            )
        }
    };
    Some(format!("Verified by {tool} {at} · {found}"))
}

fn plural(n: usize, one: &'static str, many: &'static str) -> &'static str {
    if n == 1 { one } else { many }
}

fn clip(text: &str) -> String {
    clip_to(text, MESSAGE_CHARS)
}

fn clip_to(text: &str, chars: usize) -> String {
    match text.char_indices().nth(chars) {
        Some((at, _)) => format!("{}…", &text[..at]),
        None => text.to_owned(),
    }
}

fn profile_config(profile: &AgentProfile) -> ChatConfig {
    ChatConfig {
        harness: profile.harness,
        model: profile.model.clone(),
        reasoning: profile.reasoning,
        model_options: profile.model_options.clone(),
        sandbox: SandboxLevel::WorkspaceWrite,
        permission_mode: profile.permission_mode.unwrap_or(PermissionMode::Full),
        instructions: profile.instructions.clone(),
        profile_id: Some(profile.id.clone()),
        instance_id: None,
        disabled_tools: profile.disabled_tools.clone(),
    }
}

struct Mission {
    key: String,
    root: Option<String>,
    plan: bool,
    title: String,
    progress: PlanProgress,
    status: TaskStatus,
    watch: Watch,
    current: String,
    done_at: Option<DateTime<Utc>>,
    starts_open: bool,
}

const RAIL_OPEN_TREES: usize = 4;

impl Mission {
    fn fraction(&self) -> f32 {
        let PlanProgress { done, total } = self.progress;
        if self.done_at.is_some() {
            1.0
        } else if total == 0 {
            0.0
        } else {
            (done as f32 / total as f32).min(1.0)
        }
    }
}

fn missions(state: &AppState, now: DateTime<Utc>) -> Vec<Mission> {
    let unwatched = state
        .supervisor
        .as_ref()
        .map_or(&[][..], |snapshot| snapshot.unwatched.as_slice());
    let active = crate::home::mission_rows(state, now)
        .into_iter()
        .map(|row| (row, false));
    let done = crate::home::done_today_rows(state, now)
        .into_iter()
        .map(|row| (row, true));
    let plans = active.chain(done).map(|(row, done)| {
        let task = state.tasks.iter().find(|task| task.id == row.task_id);
        let done_at = task.and_then(|task| task.completed_at).filter(|_| done);
        let lead = task.and_then(|task| linked_chat(&state.chats, task));
        let watch = watch_of(
            &row.task_id,
            lead.map(|chat| chat.device_id.as_str()),
            state.local_device_id.as_deref(),
            unwatched,
        );
        let current = match done_at {
            Some(at) => format!(
                "Done · {} of {} · {}",
                row.progress.done,
                row.progress.total,
                clock(at)
            ),
            None => task
                .map(|task| rollup(state, task, now))
                .and_then(|plan| {
                    let step = plan
                        .steps
                        .iter()
                        .find(|step| !matches!(step.status, StepStatus::Done | StepStatus::Skipped))
                        .or(plan.steps.last())?;
                    Some(format!(
                        "{} · {}",
                        plan.step_label(&step.step.id)?,
                        step.step.title
                    ))
                })
                .unwrap_or_else(|| row.label.clone()),
        };
        Mission {
            root: task.and_then(|task| task.chat_id.clone()),
            plan: true,
            title: row.title,
            progress: row.progress,
            status: row.status,
            key: row.task_id,
            watch,
            current,
            done_at,
            starts_open: false,
        }
    });
    let index = crate::tasks::index_rows(state, now);
    let trees = index
        .running
        .into_iter()
        .chain(index.done_today)
        .filter(|row| row.kind == crate::tasks::IndexKind::Tree)
        .filter(|row| tree_watch(state, &row.root) == Watch::Watched)
        .map(|row| {
            let done = row
                .segments
                .iter()
                .filter(|status| **status == crate::tasks::TreeStatus::Done)
                .count();
            Mission {
                root: Some(row.root.clone()),
                plan: false,
                title: row.title,
                progress: PlanProgress {
                    done,
                    total: row.segments.len(),
                },
                status: if row.asks {
                    TaskStatus::NeedsYou
                } else if row.ended.is_some() {
                    TaskStatus::Done
                } else if row.status.live() {
                    TaskStatus::Working
                } else {
                    TaskStatus::ToDo
                },
                key: row.root,
                watch: Watch::Watched,
                current: row.line,
                done_at: row.ended,
                starts_open: false,
            }
        });
    let mut all: Vec<Mission> = plans.chain(trees).collect();
    let running = all
        .iter()
        .filter(|mission| mission.done_at.is_none())
        .count();
    for mission in &mut all {
        mission.starts_open = mission.done_at.is_none() && running <= RAIL_OPEN_TREES;
    }
    all
}

fn watch_params(mission: &Mission, on: bool) -> serde_json::Value {
    match (&mission.root, mission.plan) {
        (Some(root), false) => serde_json::json!({
            "op": "setTreeWatched", "rootChatId": root, "watched": on,
        }),
        _ => serde_json::json!({
            "op": "setMissionSupervised", "taskId": mission.key, "supervised": on,
        }),
    }
}

fn model_label(chat: &Chat, cx: &App) -> Option<String> {
    let config = chat.config.as_ref()?;
    let model = config.model.as_deref()?;
    Some(crate::pickers::model_display_label(
        config.harness,
        model,
        cx,
    ))
}

fn run(from: f32, to: f32, spec: MotionSpec, reduced: bool, now: Instant) -> Tween {
    let mut tween = Tween::at(from, now);
    tween.retarget(to, spec, reduced, now);
    tween
}

struct Arrival {
    slide: Tween,
}

struct RowMotion {
    fold: Fold,
    row: Row,
    arrival: Option<Arrival>,
}

struct GroupMotion {
    fold: Fold,
    tick: Tick,
}

#[derive(Default)]
struct TabProbe {
    origin: f32,
    rects: Vec<(f32, f32)>,
}

struct StatusMotion {
    key: &'static str,
    fade: Tween,
    grow: Tween,
    from: f32,
    height: Rc<Cell<f32>>,
}

struct Tick {
    value: usize,
    tween: Tween,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Tone {
    Plain,
    Ok,
    Warn,
}

#[derive(Debug, Clone, PartialEq)]
struct Face {
    label: String,
    tone: Tone,
}

struct ChipFade {
    face: Face,
    from: Option<Face>,
    tween: Tween,
}

struct MissionMotion {
    fold: Fold,
    chevron: Tween,
    fill: Tween,
}

pub struct SupervisorPage {
    state: Entity<AppState>,
    pickers: Entity<Pickers>,
    profiles: Loadable<Vec<AgentProfile>>,
    settings: Loadable<SupervisorSettings>,
    settings_error: Option<String>,
    watch_error: Option<(String, String)>,
    ensure: Ensure,
    ensured: Option<String>,
    opened: HashMap<String, bool>,
    drawer: Option<Owner>,
    drawer_open: bool,
    drawer_slide: Tween,
    pane: Option<Pane>,
    replies: HashMap<String, Reply>,
    picks: HashMap<String, HashMap<String, Vec<String>>>,
    sends: HashMap<String, Option<String>>,
    filter: Filter,
    list_filter: Filter,
    list_switch: Option<Instant>,
    list_in: Tween,
    tabs: Rc<RefCell<TabProbe>>,
    underline: Option<(usize, Tween, Tween)>,
    memory: Memory,
    rows: HashMap<String, RowMotion>,
    groups: HashMap<String, GroupMotion>,
    arrivals: HashSet<String>,
    glows: HashMap<String, Tween>,
    hovered: Option<String>,
    calm: Fold,
    ticks: HashMap<Option<Kind>, Tick>,
    chips: HashMap<String, ChipFade>,
    missions: HashMap<String, MissionMotion>,
    trees: HashMap<String, (Entity<crate::tasks::StripTree>, Subscription)>,
    earlier_open: bool,
    earlier: Fold,
    earlier_chevron: Tween,
    conversation_open: bool,
    conversation: Tween,
    gear: Popup<()>,
    gear_focus: FocusHandle,
    entered_at: Option<Instant>,
    fresh: bool,
    flash: Tween,
    last_wake: Option<Option<String>>,
    profiles_task: Option<Task<()>>,
    settings_task: Option<Task<()>>,
    ensure_task: Option<Task<()>>,
    effect_task: Option<Task<()>>,
    resuming: bool,
    shown: bool,
    batches: HashMap<String, Task<()>>,
    last_sheet: Option<String>,
    focus: FocusHandle,
    width: Rc<Cell<f32>>,
    rail_pref: Option<bool>,
    rail: Option<RailMotion>,
    status: Option<StatusMotion>,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<SupervisorEvent> for SupervisorPage {}

impl SupervisorPage {
    pub fn new(state: Entity<AppState>, cx: &mut Context<Self>) -> Self {
        let pickers = cx.new(|cx| Pickers::new_for_tasks(state.clone(), cx));
        let subscriptions = vec![
            cx.observe(&state, |page, _, cx| page.on_state(cx)),
            cx.observe(&pickers, |page, _, cx| page.try_ensure(cx)),
            cx.observe_global::<crate::agent_profiles::AgentProfilesChanged>(|page, cx| {
                page.load_profiles(false, cx)
            }),
        ];
        let now = Instant::now();
        let mut page = Self {
            state,
            pickers,
            profiles: Loadable::Idle,
            settings: Loadable::Idle,
            settings_error: None,
            watch_error: None,
            ensure: Ensure::Idle,
            ensured: None,
            opened: HashMap::new(),
            drawer: None,
            drawer_open: false,
            drawer_slide: Tween::at(0.0, now),
            pane: None,
            replies: HashMap::new(),
            picks: HashMap::new(),
            sends: HashMap::new(),
            filter: Filter::All,
            list_filter: Filter::All,
            list_switch: None,
            list_in: Tween::at(1.0, now),
            tabs: Rc::default(),
            underline: None,
            memory: Memory::default(),
            rows: HashMap::new(),
            groups: HashMap::new(),
            arrivals: HashSet::new(),
            glows: HashMap::new(),
            hovered: None,
            calm: Fold::at(false, now),
            ticks: HashMap::new(),
            chips: HashMap::new(),
            missions: HashMap::new(),
            trees: HashMap::new(),
            earlier_open: false,
            earlier: Fold::at(false, now),
            earlier_chevron: Tween::at(0.0, now),
            conversation_open: false,
            conversation: Tween::at(0.0, now),
            gear: Popup::default(),
            gear_focus: cx.focus_handle(),
            entered_at: None,
            fresh: true,
            flash: Tween::at(1.0, now),
            last_wake: None,
            profiles_task: None,
            settings_task: None,
            ensure_task: None,
            effect_task: None,
            resuming: false,
            shown: false,
            batches: HashMap::new(),
            last_sheet: None,
            focus: cx.focus_handle(),
            width: Rc::default(),
            rail_pref: None,
            rail: None,
            status: None,
            _subscriptions: subscriptions,
        };
        page.load_profiles(false, cx);
        page.load_settings(cx);
        page.on_state(cx);
        page
    }

    pub fn entered(&mut self, cx: &mut Context<Self>) {
        self.shown = true;
        self.ensured = None;
        if self.ensure != Ensure::Running {
            self.ensure = Ensure::Wanted;
        }
        self.entered_at = (!motion::reduced_motion(cx)).then(Instant::now);
        self.fresh = true;
        self.rows.clear();
        self.groups.clear();
        self.arrivals.clear();
        self.glows.clear();
        self.ticks.clear();
        self.chips.clear();
        self.missions.clear();
        self.list_filter = self.filter;
        self.list_switch = None;
        self.hovered = None;
        self.load_settings(cx);
        self.load_profiles(true, cx);
        self.request_check(cx);
    }

    fn request_check(&mut self, cx: &mut Context<Self>) {
        if cfg!(windows) {
            return;
        }
        let Some(engine) = self.state.read(cx).engine().cloned() else {
            return;
        };
        cx.background_spawn(async move {
            engine
                .client()
                .call(methods::REQUEST_SUPERVISOR_CHECK, serde_json::json!({}))
                .await
                .ok();
        })
        .detach();
    }

    pub fn left(&mut self, cx: &mut Context<Self>) {
        self.shown = false;
        if self.ensure == Ensure::Wanted {
            self.ensure = Ensure::Idle;
        }
        self.drawer = None;
        self.drawer_open = false;
        self.drawer_slide = Tween::at(0.0, Instant::now());
        self.gear = Popup::default();
        self.pane = None;
        cx.notify();
    }

    pub fn chat_id(&self, cx: &App) -> Option<String> {
        self.state
            .read(cx)
            .supervisor
            .as_ref()
            .and_then(|snapshot| snapshot.chat_id.clone())
            .or_else(|| self.ensured.clone())
    }

    pub fn set_pane(&mut self, pane: Option<Pane>, cx: &mut Context<Self>) {
        self.pane = pane;
        cx.notify();
    }

    fn on_state(&mut self, cx: &mut Context<Self>) {
        if matches!(self.settings, Loadable::Idle) {
            self.load_settings(cx);
        }
        if matches!(self.profiles, Loadable::Idle) {
            self.load_profiles(false, cx);
        }
        let known = self
            .state
            .read(cx)
            .supervisor
            .as_ref()
            .is_some_and(|snapshot| snapshot.chat_id.is_some());
        if known {
            self.ensured = None;
        } else if self.shown && self.ensured.is_none() && self.ensure == Ensure::Idle {
            self.ensure = Ensure::Wanted;
        }
        let choices: HashMap<String, String> = self
            .replies
            .iter()
            .filter(|(_, reply)| matches!(reply.stage, Stage::Sending | Stage::Sent(_)))
            .map(|(key, reply)| (key.clone(), reply.choice.clone()))
            .collect();
        let (arrived, wake) = {
            let snapshot = self.state.read(cx).supervisor.as_ref();
            (
                self.memory.observe(snapshot, &choices, Utc::now()),
                snapshot.map(|snapshot| snapshot.wakes.first().map(|wake| wake.id.clone())),
            )
        };
        let woke = matches!((&self.last_wake, &wake), (Some(seen), Some(now)) if seen != now && now.is_some());
        if wake.is_some() {
            self.last_wake = wake;
        }
        if woke || !arrived.is_empty() {
            let reduced = motion::reduced_motion(cx);
            self.flash = run(0.0, 1.0, DOT_FLASH, reduced, Instant::now());
        }
        if !arrived.is_empty() {
            let (reduced, now) = (motion::reduced_motion(cx), Instant::now());
            let tasks: Vec<String> = self
                .state
                .read(cx)
                .supervisor
                .iter()
                .flat_map(|snapshot| &snapshot.needs_you)
                .filter(|row| arrived.contains(&row_key(row)))
                .filter_map(|row| row_origin(row).0.map(str::to_owned))
                .collect();
            for task in tasks {
                self.glows
                    .insert(task, run(0.0, 1.0, DOT_FLASH, reduced, now));
            }
            self.pick_filter_to(Filter::All, cx);
            self.arrivals.extend(arrived);
        }
        let open: HashSet<String> = self
            .state
            .read(cx)
            .supervisor
            .as_ref()
            .map(|snapshot| snapshot.needs_you.iter().map(row_key).collect())
            .unwrap_or_default();
        let rows = &self.rows;
        self.replies
            .retain(|key, _| open.contains(key) || rows.contains_key(key));
        self.picks.retain(|key, _| open.contains(key));
        self.schedule_effect_check(cx);
        self.try_ensure(cx);
        cx.notify();
    }

    fn schedule_effect_check(&mut self, cx: &mut Context<Self>) {
        let now = Utc::now();
        let next = self
            .state
            .read(cx)
            .supervisor
            .as_ref()
            .and_then(|snapshot| {
                let held = snapshot
                    .interventions
                    .iter()
                    .any(|row| matches!(row.approval, Some(PlanApproval::Held { .. })));
                [
                    next_effect_check(&snapshot.interventions, now),
                    feed::next_expiry(&snapshot.interventions, now),
                    held.then_some(now),
                ]
                .into_iter()
                .flatten()
                .min()
            });
        self.effect_task = next.map(|at| {
            let wait = (at - now).to_std().unwrap_or_default() + Duration::from_secs(1);
            cx.spawn(async move |page, cx| {
                cx.background_executor().timer(wait).await;
                page.update(cx, |page, cx| {
                    page.schedule_effect_check(cx);
                    cx.notify();
                })
                .ok();
            })
        });
    }

    fn staged(&self, key: &str, question: &str) -> Vec<String> {
        self.picks
            .get(key)
            .and_then(|picks| picks.get(question))
            .cloned()
            .unwrap_or_default()
    }

    #[allow(clippy::too_many_arguments)]
    fn stage(
        &mut self,
        key: String,
        group: String,
        question: String,
        option: String,
        multi: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        window.focus(&self.focus, cx);
        if self.replies.get(&key).is_some_and(|reply| !reply.failed()) {
            return;
        }
        self.replies.remove(&key);
        let picked = self
            .picks
            .entry(key)
            .or_default()
            .entry(question)
            .or_default();
        match picked.iter().position(|label| *label == option) {
            Some(at) => {
                picked.remove(at);
            }
            None if multi => picked.push(option),
            None => *picked = vec![option],
        }
        self.last_sheet = Some(group);
        cx.notify();
    }

    fn ready(&self, row: &NeedsYouRow) -> Option<(String, Option<Vec<UserInputAnswer>>)> {
        match row {
            NeedsYouRow::Item(_) => {
                let choice = self.staged(&row_key(row), "").into_iter().next()?;
                Some((choice, None))
            }
            NeedsYouRow::Input(input) => {
                let answers = self.picked_answers(input)?;
                Some((answers_text(&answers), Some(answers)))
            }
        }
    }

    fn send_sheet(&mut self, group: &str, cx: &mut Context<Self>) {
        let now = Instant::now();
        let ready: Vec<(String, String)> = self
            .rows
            .values()
            .filter(|motion| feed::task_of(&motion.row.entry).unwrap_or_default() == group)
            .filter_map(|motion| inbox::sheet_row(&motion.row.entry))
            .filter(|row| {
                self.replies
                    .get(&row_key(row))
                    .is_none_or(|reply| reply.failed())
            })
            .filter_map(|row| Some((row_key(row), self.ready(row)?.0)))
            .collect();
        if ready.is_empty() {
            return;
        }
        for (key, choice) in ready {
            self.replies.insert(
                key,
                Reply {
                    choice,
                    flipped: now,
                    stage: Stage::Held,
                },
            );
        }
        let key = group.to_owned();
        self.batches.insert(
            key.clone(),
            cx.spawn(async move |page, cx| {
                cx.background_executor().timer(UNDO_WINDOW).await;
                page.update(cx, |page, cx| page.flush(&key, cx)).ok();
            }),
        );
        cx.notify();
    }

    fn undo(&mut self, key: &str, cx: &mut Context<Self>) {
        if self
            .replies
            .get(key)
            .is_some_and(|reply| reply.stage == Stage::Held)
        {
            self.replies.remove(key);
            cx.notify();
        }
    }

    fn flush(&mut self, group: &str, cx: &mut Context<Self>) {
        self.batches.remove(group);
        let engine = self.state.read(cx).engine().cloned();
        let held: Vec<NeedsYouRow> = self
            .rows
            .values()
            .filter(|motion| feed::task_of(&motion.row.entry).unwrap_or_default() == group)
            .filter_map(|motion| inbox::sheet_row(&motion.row.entry).cloned())
            .filter(|row| {
                self.replies
                    .get(&row_key(row))
                    .is_some_and(|reply| reply.stage == Stage::Held)
            })
            .collect();
        let mut items = Vec::new();
        let mut inputs = Vec::new();
        for row in &held {
            let key = row_key(row);
            let Some((choice, answers)) = self.ready(row) else {
                self.replies.remove(&key);
                continue;
            };
            match (row, answers) {
                (NeedsYouRow::Item(item), _) => items.push((
                    key.clone(),
                    serde_json::json!({ "itemId": item.id, "choice": choice }),
                )),
                (NeedsYouRow::Input(input), Some(answers)) => {
                    inputs.push((key.clone(), input.clone(), answers))
                }
                (NeedsYouRow::Input(_), None) => continue,
            }
            if let Some(reply) = self.replies.get_mut(&key) {
                reply.stage = Stage::Sending;
            }
        }
        if items.is_empty() && inputs.is_empty() {
            cx.notify();
            return;
        }
        let Some(engine) = engine else {
            for key in items
                .iter()
                .map(|(key, _)| key)
                .chain(inputs.iter().map(|(key, ..)| key))
            {
                if let Some(reply) = self.replies.get_mut(key) {
                    reply.stage = Stage::Failed(NOT_CONNECTED.into());
                }
            }
            cx.notify();
            return;
        };
        cx.spawn(async move |page, cx| {
            let mut results: Vec<(String, Result<(), String>)> = Vec::new();
            if !items.is_empty() {
                let answers: Vec<serde_json::Value> =
                    items.iter().map(|(_, answer)| answer.clone()).collect();
                let sent = engine
                    .client()
                    .call(
                        methods::MUTATE,
                        serde_json::json!({ "op": "resolveSupervisorItems", "answers": answers }),
                    )
                    .await
                    .map(|_| ())
                    .map_err(|err| err.to_string());
                results.extend(items.into_iter().map(|(key, _)| (key, sent.clone())));
            }
            for (key, input, answers) in inputs {
                let sent = crate::composer::respond_input(
                    &engine,
                    &input.chat_id,
                    &input.request_id,
                    answers,
                )
                .await;
                results.push((key, sent));
            }
            page.update(cx, |page, cx| {
                let now = Instant::now();
                for (key, result) in results {
                    if let Some(reply) = page.replies.get_mut(&key) {
                        reply.stage = match result {
                            Ok(()) => Stage::Sent(now),
                            Err(error) => Stage::Failed(error),
                        };
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    fn correct(&mut self, item_id: String, choice: String, cx: &mut Context<Self>) {
        let params = serde_json::json!({
            "op": "resolveSupervisorItems",
            "answers": [{ "itemId": item_id, "choice": choice }],
        });
        self.send_mutation(item_id, params, cx);
    }

    fn take_back(&mut self, task_id: String, cx: &mut Context<Self>) {
        let params = serde_json::json!({ "op": "takeBackPlanApproval", "taskId": task_id });
        self.send_mutation(take_back_key(&task_id), params, cx);
    }

    fn send_mutation(&mut self, key: String, params: serde_json::Value, cx: &mut Context<Self>) {
        if matches!(self.sends.get(&key), Some(None)) {
            return;
        }
        let Some(engine) = self.state.read(cx).engine().cloned() else {
            self.sends.insert(key, Some(NOT_CONNECTED.into()));
            cx.notify();
            return;
        };
        self.sends.insert(key.clone(), None);
        cx.spawn(async move |page, cx| {
            let sent = engine.client().call(methods::MUTATE, params).await;
            page.update(cx, |page, cx| {
                match sent {
                    Ok(_) => page.sends.remove(&key),
                    Err(error) => page.sends.insert(key, Some(error.to_string())),
                };
                cx.notify();
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    fn picked_answers(&self, input: &NeedsYouInput) -> Option<Vec<UserInputAnswer>> {
        let picks = self
            .picks
            .get(&row_key(&NeedsYouRow::Input(input.clone())))?;
        input
            .questions
            .iter()
            .map(|question| {
                let labels = picks
                    .get(&question.id)
                    .filter(|labels| !labels.is_empty())?;
                Some(UserInputAnswer {
                    question_id: question.id.clone(),
                    labels: labels.clone(),
                })
            })
            .collect()
    }

    /// A fresh load holds back ensuring: the person may have changed the Supervisor's profile since the last.
    fn load_profiles(&mut self, fresh: bool, cx: &mut Context<Self>) {
        let Some(engine) = self.state.read(cx).engine().cloned() else {
            return;
        };
        if fresh || matches!(self.profiles, Loadable::Idle) {
            self.profiles = Loadable::Loading;
        }
        self.profiles_task = Some(cx.spawn(async move |page, cx| {
            let result = crate::agent_profiles::list(&engine, None).await;
            page.update(cx, |page, cx| {
                page.profiles = match result {
                    Ok(profiles) => Loadable::Ready(profiles),
                    Err(error) => Loadable::Error(error),
                };
                page.try_ensure(cx);
                cx.notify();
            })
            .ok();
        }));
    }

    fn load_settings(&mut self, cx: &mut Context<Self>) {
        let Some(engine) = self.state.read(cx).engine().cloned() else {
            return;
        };
        if matches!(self.settings, Loadable::Idle) {
            self.settings = Loadable::Loading;
        }
        self.settings_task = Some(cx.spawn(async move |page, cx| {
            let result = engine
                .client()
                .call_as::<SupervisorSettings>(
                    methods::GET_SUPERVISOR_SETTINGS,
                    serde_json::json!({}),
                )
                .await
                .map_err(|err| err.to_string());
            page.update(cx, |page, cx| {
                match result {
                    Ok(settings) => {
                        page.settings = Loadable::Ready(settings);
                        page.settings_error = None;
                    }
                    Err(error) if page.settings.ready().is_some() => {
                        page.settings_error = Some(error)
                    }
                    Err(error) => page.settings = Loadable::Error(error),
                }
                cx.notify();
            })
            .ok();
        }));
    }

    fn save_settings(&mut self, settings: SupervisorSettings, cx: &mut Context<Self>) {
        let Some(engine) = self.state.read(cx).engine().cloned() else {
            return;
        };
        let previous = self.settings.ready().copied();
        self.settings = Loadable::Ready(settings);
        self.settings_error = None;
        self.settings_task = Some(cx.spawn(async move |page, cx| {
            let saved = engine
                .client()
                .call(
                    methods::SET_SUPERVISOR_SETTINGS,
                    serde_json::to_value(settings).unwrap_or_default(),
                )
                .await;
            if let Err(err) = saved {
                page.update(cx, |page, cx| {
                    if let Some(previous) = previous {
                        page.settings = Loadable::Ready(previous);
                    }
                    page.settings_error = Some(err.to_string());
                    cx.notify();
                })
                .ok();
            }
        }));
        cx.notify();
    }

    fn set_watch_everything(&mut self, on: bool, cx: &mut Context<Self>) {
        self.send_watch(
            WATCH_EVERYTHING.into(),
            serde_json::json!({ "op": "setWatchEverything", "on": on }),
            cx,
        );
    }

    fn send_watch(&mut self, key: String, params: serde_json::Value, cx: &mut Context<Self>) {
        if self.watch_error.as_ref().is_some_and(|(at, _)| *at == key) {
            self.watch_error = None;
        }
        let Some(engine) = self.state.read(cx).engine().cloned() else {
            self.watch_error = Some((key, NOT_CONNECTED.into()));
            cx.notify();
            return;
        };
        cx.spawn(async move |page, cx| {
            if let Err(err) = engine.client().call(methods::MUTATE, params).await {
                page.update(cx, |page, cx| {
                    page.watch_error =
                        Some((key, format!("Could not change what it watches: {err}")));
                    cx.notify();
                })
                .ok();
            }
        })
        .detach();
        cx.notify();
    }

    fn watch_error_line(&self, key: &str, theme: &Theme) -> Option<gpui::Div> {
        let (_, error) = self.watch_error.as_ref().filter(|(at, _)| at == key)?;
        Some(
            div()
                .debug_selector(|| "supervisor-watch-error".into())
                .text_size(ui_rems(11.5))
                .text_color(theme.danger)
                .child(error.clone()),
        )
    }

    fn start_config(&self, cx: &App) -> Option<ChatConfig> {
        let profile = self
            .profiles
            .ready()
            .and_then(|profiles| profiles.iter().find(|profile| profile.id == PROFILE_ID));
        if let Some(profile) = profile {
            return Some(profile_config(profile));
        }
        let mut config = self.pickers.read(cx).resolved(cx).chat_config()?;
        config.permission_mode = PermissionMode::Full;
        Some(config)
    }

    fn try_ensure(&mut self, cx: &mut Context<Self>) {
        if self.ensure != Ensure::Wanted {
            return;
        }
        let (known, engine) = {
            let state = self.state.read(cx);
            (
                state
                    .supervisor
                    .as_ref()
                    .map(|snapshot| snapshot.chat_id.is_some()),
                state.engine().cloned(),
            )
        };
        let Some(known) = known else {
            return;
        };
        if known || self.ensured.is_some() {
            self.ensure = Ensure::Idle;
            return;
        }
        let Some(engine) = engine else {
            return;
        };
        if matches!(self.profiles, Loadable::Idle | Loadable::Loading) {
            return;
        }
        let Some(config) = self.start_config(cx) else {
            let pickers = self.pickers.read(cx);
            let failure = if pickers.no_agents_available() {
                Some(NO_AGENT.to_owned())
            } else {
                pickers.harnesses_error().map(str::to_owned)
            };
            match failure {
                Some(reason) => {
                    self.ensure = Ensure::Failed(reason);
                    cx.notify();
                }
                None => self
                    .pickers
                    .update(cx, |pickers, cx| pickers.ensure_harnesses(false, cx)),
            }
            return;
        };
        self.ensure = Ensure::Running;
        self.ensure_task = Some(cx.spawn(async move |page, cx| {
            let result = engine
                .client()
                .call(
                    methods::ENSURE_SUPERVISOR,
                    serde_json::json!({ "config": config }),
                )
                .await;
            page.update(cx, |page, cx| {
                match result {
                    Ok(value) => {
                        page.ensure = Ensure::Idle;
                        page.ensured = value
                            .get("chatId")
                            .and_then(|id| id.as_str())
                            .or(value.as_str())
                            .map(str::to_owned);
                    }
                    Err(err) => page.ensure = Ensure::Failed(err.to_string()),
                }
                cx.notify();
            })
            .ok();
        }));
        cx.notify();
    }

    fn retry_ensure(&mut self, cx: &mut Context<Self>) {
        self.ensure = Ensure::Wanted;
        self.load_profiles(true, cx);
        self.pickers
            .update(cx, |pickers, cx| pickers.ensure_harnesses(true, cx));
        cx.notify();
    }

    fn owner_thread(&self, owner: &Owner, cx: &App) -> Option<String> {
        let state = self.state.read(cx);
        match &owner.step_id {
            Some(step_id) => state
                .task_steps
                .iter()
                .find(|step| &step.id == step_id && !step.deleted)
                .and_then(|step| step_chat(&state.chats, step))
                .map(|chat| chat.id.clone()),
            None => state
                .tasks
                .iter()
                .find(|task| task.id == owner.task_id)
                .and_then(|task| linked_chat(&state.chats, task))
                .map(|chat| chat.id.clone()),
        }
    }

    fn open_drawer(&mut self, owner: Owner, cx: &mut Context<Self>) {
        let thread = self.owner_thread(&owner, cx);
        self.drawer = Some(owner);
        self.drawer_open = true;
        self.drawer_slide.retarget(
            1.0,
            DRAWER_SLIDE,
            motion::reduced_motion(cx),
            Instant::now(),
        );
        cx.emit(SupervisorEvent::WatchDoc(thread));
        cx.notify();
    }

    fn close_drawer(&mut self, cx: &mut Context<Self>) {
        self.drawer_open = false;
        let reduced = motion::reduced_motion(cx);
        self.drawer_slide
            .retarget(0.0, DRAWER_SLIDE, reduced, Instant::now());
        let wait = if reduced {
            Duration::ZERO
        } else {
            DRAWER_SLIDE.total().mul_f32(motion::speed_scale())
        };
        cx.spawn(async move |page, cx| {
            cx.background_executor().timer(wait).await;
            page.update(cx, |page, cx| {
                if !page.drawer_open && page.drawer.take().is_some() {
                    cx.emit(SupervisorEvent::WatchDoc(None));
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    fn open_gear(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.gear.open(());
        window.focus(&self.gear_focus, cx);
        self.load_settings(cx);
        cx.notify();
    }

    fn close_gear(&mut self, cx: &mut Context<Self>) {
        if self.gear.begin_close() {
            popover::reap_popup(cx, |page: &mut Self| &mut page.gear);
            cx.notify();
        }
    }

    fn pick_filter(&mut self, chip: Option<Kind>, cx: &mut Context<Self>) {
        self.pick_filter_to(self.filter.pick(chip), cx);
    }

    fn pick_filter_to(&mut self, filter: Filter, cx: &mut Context<Self>) {
        if self.filter == filter {
            return;
        }
        self.filter = filter;
        if self.list_switch.is_none() {
            self.list_switch = Some(Instant::now());
        }
        cx.notify();
    }

    fn hover_task(&mut self, task: Option<String>, hovered: bool, cx: &mut Context<Self>) {
        let next = if hovered {
            task
        } else if self.hovered == task {
            None
        } else {
            return;
        };
        if self.hovered != next {
            self.hovered = next;
            cx.notify();
        }
    }

    fn toggle_rail(&mut self, cx: &mut Context<Self>) {
        let showing = self.rail.as_ref().map_or(RailMode::Full, |rail| rail.to);
        self.rail_pref = Some(showing == RailMode::Strip);
        cx.notify();
    }

    fn mission_open(&self, mission: &Mission) -> bool {
        self.opened
            .get(&mission.key)
            .copied()
            .unwrap_or(mission.starts_open)
    }

    fn watched_by_everything(&self, mission: &Mission, cx: &App) -> bool {
        let snapshot = self.state.read(cx).supervisor.as_ref();
        mission.root.as_ref().is_some_and(|root| {
            snapshot.is_some_and(|snapshot| {
                snapshot.watch_everything && !snapshot.watched_roots.contains(root)
            })
        })
    }

    fn toggle_earlier(&mut self, cx: &mut Context<Self>) {
        self.earlier_open = !self.earlier_open;
        let (reduced, now) = (motion::reduced_motion(cx), Instant::now());
        self.earlier.set(self.earlier_open, FOLD, reduced, now);
        self.earlier_chevron.retarget(
            if self.earlier_open { 1.0 } else { 0.0 },
            CHEVRON_TURN,
            reduced,
            now,
        );
        cx.notify();
    }

    fn toggle_conversation(&mut self, cx: &mut Context<Self>) {
        self.conversation_open = !self.conversation_open;
        self.conversation.retarget(
            if self.conversation_open { 1.0 } else { 0.0 },
            FOLD,
            motion::reduced_motion(cx),
            Instant::now(),
        );
        cx.notify();
    }

    fn open_conversation(&mut self, cx: &mut Context<Self>) {
        if !self.conversation_open {
            self.toggle_conversation(cx);
        }
    }

    fn resume(&mut self, held: Vec<String>, cx: &mut Context<Self>) {
        let (Some(chat_id), Some(engine)) =
            (self.chat_id(cx), self.state.read(cx).engine().cloned())
        else {
            return;
        };
        if self.resuming {
            return;
        }
        self.resuming = true;
        cx.notify();
        cx.spawn(async move |page, cx| {
            for id in held {
                let sent = engine
                    .client()
                    .call(
                        methods::STEER_QUEUED_MESSAGE_NOW,
                        serde_json::json!({ "chatId": chat_id, "id": id }),
                    )
                    .await;
                match sent {
                    Ok(value) if value.get("sent").and_then(|v| v.as_bool()) == Some(true) => {
                        break;
                    }
                    Ok(_) => {}
                    Err(err) => {
                        tracing::warn!(error = %err, "resuming the Supervisor failed");
                        break;
                    }
                }
            }
            page.update(cx, |page, cx| {
                page.resuming = false;
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn set_mission_open(&mut self, key: &str, open: bool, cx: &mut Context<Self>) {
        self.opened.insert(key.to_owned(), open);
        cx.notify();
    }

    fn sync_motion(&mut self, feed: &Feed, missions: &[Mission], now: Instant, reduced: bool) {
        let fresh = std::mem::take(&mut self.fresh);
        let mut snap = reduced;
        if self.filter == self.list_filter {
            self.list_switch = None;
        } else {
            let out = LIST_OUT.total().mul_f32(motion::speed_scale());
            if reduced || self.list_switch.is_none_or(|at| now >= at + out) {
                self.list_filter = self.filter;
                self.list_switch = None;
                self.list_in = run(0.0, 1.0, LIST_IN, reduced, now);
                snap = true;
            }
        }
        let mut seen: HashSet<String> = HashSet::new();
        let mut any_shown = false;
        for (_, row, shown) in feed.rows(self.list_filter) {
            let shown = shown
                && !self
                    .replies
                    .get(&row.key)
                    .is_some_and(|reply| reply.collapsing(now, reduced));
            any_shown |= shown;
            seen.insert(row.key.clone());
            if let Some(motion) = self.rows.get_mut(&row.key) {
                motion.row = row.clone();
                motion.fold.set(shown, ROW_FOLD, snap, now);
                if motion
                    .arrival
                    .as_ref()
                    .is_some_and(|arrival| arrival.slide.settled_at(now))
                {
                    motion.arrival = None;
                }
                continue;
            }
            let arriving = !fresh && !snap && self.arrivals.contains(&row.key);
            let mut fold = Fold::at(fresh && shown, now);
            fold.set(shown, if arriving { ARRIVE } else { ROW_FOLD }, snap, now);
            self.rows.insert(
                row.key.clone(),
                RowMotion {
                    fold,
                    row: row.clone(),
                    arrival: arriving.then(|| Arrival {
                        slide: run(0.0, 1.0, ARRIVE, reduced, now),
                    }),
                },
            );
        }
        self.arrivals.clear();
        let replies = &self.replies;
        self.rows.retain(|key, motion| {
            if seen.contains(key) {
                return true;
            }
            let holding = replies
                .get(key)
                .is_some_and(|reply| reply.holding(now, reduced));
            any_shown |= holding;
            motion.fold.set(holding, ROW_FOLD, reduced, now);
            !motion.fold.gone(now)
        });
        let mut open_groups: HashMap<String, bool> = HashMap::new();
        for motion in self.rows.values() {
            let key = feed::task_of(&motion.row.entry)
                .unwrap_or_default()
                .to_owned();
            *open_groups.entry(key).or_default() |= motion.fold.tween.target() > 0.0;
        }
        if self
            .hovered
            .as_ref()
            .is_some_and(|key| open_groups.get(key) != Some(&true))
        {
            self.hovered = None;
        }
        for (key, open) in &open_groups {
            let count = feed
                .open
                .iter()
                .filter(|row| {
                    row.kind == Kind::Needs && feed::task_of(&row.entry).unwrap_or_default() == key
                })
                .count();
            match self.groups.get_mut(key) {
                Some(group) => {
                    group.fold.set(*open, ROW_FOLD, snap, now);
                    if group.tick.value != count {
                        group.tick.value = count;
                        group.tick.tween = run(0.0, 1.0, COUNT_TICK, reduced, now);
                    }
                }
                None => {
                    let mut fold = Fold::at(fresh && *open, now);
                    fold.set(*open, ARRIVE, snap, now);
                    self.groups.insert(
                        key.clone(),
                        GroupMotion {
                            fold,
                            tick: Tick {
                                value: count,
                                tween: Tween::at(1.0, now),
                            },
                        },
                    );
                }
            }
        }
        self.groups.retain(|key, group| {
            if !open_groups.contains_key(key) {
                group.fold.set(false, ROW_FOLD, reduced, now);
            }
            !group.fold.gone(now) || open_groups.contains_key(key)
        });
        if fresh {
            self.calm.tween = Tween::at(if any_shown { 0.0 } else { 1.0 }, now);
        } else {
            self.calm.set(!any_shown, FOLD, snap, now);
        }
        let mut counts = vec![(None, feed.open.len())];
        counts.extend(
            Kind::chips()
                .into_iter()
                .map(|kind| (Some(kind), feed.count(kind))),
        );
        for (kind, count) in counts {
            match self.ticks.get_mut(&kind) {
                Some(tick) if tick.value != count => {
                    tick.value = count;
                    tick.tween = run(0.0, 1.0, COUNT_TICK, reduced, now);
                }
                Some(_) => {}
                None => {
                    self.ticks.insert(
                        kind,
                        Tick {
                            value: count,
                            tween: Tween::at(1.0, now),
                        },
                    );
                }
            }
        }
        self.glows.retain(|_, tween| !tween.settled_at(now));
        let utc = Utc::now();
        for row in &feed.open {
            let Entry::Intervention(intervention) = &row.entry else {
                continue;
            };
            let Some(face) = chip_face(intervention, utc) else {
                self.chips.remove(&row.key);
                continue;
            };
            match self.chips.get_mut(&row.key) {
                Some(fade) if fade.face != face => {
                    fade.from = Some(std::mem::replace(&mut fade.face, face));
                    fade.tween = run(0.0, 1.0, CHIP_FADE, reduced, now);
                }
                Some(_) => {}
                None => {
                    self.chips.insert(
                        row.key.clone(),
                        ChipFade {
                            face,
                            from: None,
                            tween: Tween::at(1.0, now),
                        },
                    );
                }
            }
        }
        let rows = &self.rows;
        self.chips.retain(|key, _| rows.contains_key(key));
        for mission in missions {
            let id = &mission.key;
            let (open, fill) = (self.mission_open(mission), mission.fraction());
            let turn = if open { 1.0 } else { 0.0 };
            match self.missions.get_mut(id) {
                Some(motion) => {
                    if motion.fill.target() != fill {
                        motion.fill.retarget(fill, FILL, reduced, now);
                    }
                    motion.fold.set(open, MISSION_FOLD, reduced, now);
                    if motion.chevron.target() != turn {
                        motion.chevron.retarget(turn, CHEVRON_TURN, reduced, now);
                    }
                }
                None => {
                    self.missions.insert(
                        id.clone(),
                        MissionMotion {
                            fold: Fold::at(open, now),
                            chevron: Tween::at(turn, now),
                            fill: run(0.0, fill, FILL, reduced, now),
                        },
                    );
                }
            }
        }
        self.missions
            .retain(|id, _| missions.iter().any(|mission| &mission.key == id));
        self.opened
            .retain(|id, _| missions.iter().any(|mission| &mission.key == id));
    }

    fn sync_rail(&mut self, now: Instant, reduced: bool) -> rail::RailFrame {
        let mode = RailMode::auto(self.width.get(), self.rail_pref);
        if mode == RailMode::Full {
            self.rail_pref = None;
        }
        match &mut self.rail {
            Some(rail) if rail.to != mode => {
                let current = rail.frame(now).shown;
                *rail = RailMotion {
                    from: current,
                    to: mode,
                    at: (!reduced).then_some(now),
                };
            }
            Some(_) => {}
            None => {
                self.rail = Some(RailMotion {
                    from: mode,
                    to: mode,
                    at: None,
                })
            }
        }
        let rail = self.rail.as_mut().expect("set above");
        let frame = rail.frame(now);
        if !frame.moving {
            rail.at = None;
        }
        frame
    }

    fn sync_status(&mut self, key: &'static str, now: Instant, reduced: bool) {
        match &mut self.status {
            Some(status) if status.key != key => {
                status.key = key;
                status.from = status.height.get();
                status.fade = run(0.0, 1.0, STATUS_FADE, reduced, now);
                status.grow = run(0.0, 1.0, STATUS_GROW, reduced, now);
            }
            Some(_) => {}
            None => {
                self.status = Some(StatusMotion {
                    key,
                    fade: Tween::at(1.0, now),
                    grow: Tween::at(1.0, now),
                    from: 0.0,
                    height: Rc::default(),
                })
            }
        }
    }

    fn sync_underline(&mut self, now: Instant, reduced: bool) {
        let ix = match self.filter {
            Filter::All => 0,
            Filter::Only(kind) => {
                1 + Kind::chips()
                    .iter()
                    .position(|chip| *chip == kind)
                    .unwrap_or_default()
            }
        };
        let probe = self.tabs.borrow();
        let Some(&(x, w)) = probe.rects.get(ix).filter(|rect| rect.1 > 0.0) else {
            return;
        };
        let x = x - probe.origin;
        match &mut self.underline {
            Some((at, left, width)) if *at != ix => {
                *at = ix;
                left.retarget(x, TAB_SLIDE, reduced, now);
                width.retarget(w, TAB_SLIDE, reduced, now);
            }
            Some((_, left, width)) => {
                if left.settled_at(now) && (left.target() != x || width.target() != w) {
                    *left = Tween::at(x, now);
                    *width = Tween::at(w, now);
                }
            }
            None => self.underline = Some((ix, Tween::at(x, now), Tween::at(w, now))),
        }
    }

    fn underline_rect(&self, now: Instant) -> Option<(f32, f32)> {
        self.underline
            .as_ref()
            .map(|(_, left, width)| (left.value_at(now), width.value_at(now)))
    }

    fn enter_progress(&self, block: u64, now: Instant) -> f32 {
        let Some(at) = self.entered_at else {
            return 1.0;
        };
        let spec = ENTER.with_delay(block * ENTER_STAGGER_MS);
        let total = spec.total().mul_f32(motion::speed_scale()).as_secs_f32();
        if total <= 0.0 {
            return 1.0;
        }
        spec.progress(now.saturating_duration_since(at).as_secs_f32() / total)
    }

    fn animating(&self, now: Instant) -> bool {
        let settled = |tween: &Tween| tween.settled_at(now);
        let scale = motion::speed_scale();
        let held = |reply: &Reply| match reply.stage {
            Stage::Held | Stage::Sending => now < reply.flipped + CONFIRM.total().mul_f32(scale),
            Stage::Sent(at) => now <= at + HOLD.mul_f32(scale),
            Stage::Failed(_) => false,
        };
        self.rows.values().any(|motion| {
            motion.fold.moving(now)
                || motion
                    .arrival
                    .as_ref()
                    .is_some_and(|arrival| !settled(&arrival.slide))
        }) || self.replies.values().any(held)
            || self
                .groups
                .values()
                .any(|group| group.fold.moving(now) || !settled(&group.tick.tween))
            || self.glows.values().any(|tween| !settled(tween))
            || self.list_switch.is_some()
            || !settled(&self.list_in)
            || self
                .underline
                .as_ref()
                .is_some_and(|(_, left, width)| !settled(left) || !settled(width))
            || self.rail.as_ref().is_some_and(|rail| rail.at.is_some())
            || self
                .status
                .as_ref()
                .is_some_and(|status| !settled(&status.fade) || !settled(&status.grow))
            || self.calm.moving(now)
            || self.ticks.values().any(|tick| !settled(&tick.tween))
            || self.chips.values().any(|fade| !settled(&fade.tween))
            || self.missions.values().any(|motion| {
                motion.fold.moving(now) || !settled(&motion.chevron) || !settled(&motion.fill)
            })
            || self.earlier.moving(now)
            || !settled(&self.earlier_chevron)
            || !settled(&self.conversation)
            || !settled(&self.drawer_slide)
            || !settled(&self.flash)
            || (0..3).any(|block| self.enter_progress(block, now) < 1.0)
    }

    fn render_dot(
        &self,
        mode: &Mode,
        tone: Hsla,
        theme: &Theme,
        now: Instant,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let indicator = |el: gpui::Div, selector: &'static str| {
            el.debug_selector(move || selector.into())
                .flex_none()
                .size(px(18.0))
                .flex()
                .items_center()
                .justify_center()
        };
        let mark = |name: &'static str| icon(name).size(px(13.0)).text_color(tone);
        let live = match mode {
            Mode::Working => {
                let reduced = motion::reduced_motion(cx);
                if !reduced {
                    motion::pulse_lease(cx.entity_id(), cx);
                }
                return indicator(div(), "supervisor-dot-working")
                    .child(crate::tasks::spinner(13.0, tone, reduced))
                    .into_any_element();
            }
            Mode::Stopped => {
                return indicator(div(), "supervisor-dot-stopped")
                    .child(mark(icons::LUCIDE_SQUARE))
                    .into_any_element();
            }
            Mode::Failed(_) => {
                return indicator(div(), "supervisor-dot-failed")
                    .child(mark(icons::DANGER_TRIANGLE))
                    .into_any_element();
            }
            Mode::Off => {
                return indicator(div(), "supervisor-dot")
                    .child(
                        div()
                            .size(px(9.0))
                            .rounded_full()
                            .border_1()
                            .border_color(theme.text_faint),
                    )
                    .into_any_element();
            }
            Mode::Watching => tone,
        };
        let amber = needs_tone(theme);
        let flash = (!self.flash.settled_at(now)).then(|| self.flash.value_at(now));
        indicator(div(), "supervisor-dot-live")
            .child(
                div().size(px(9.0)).child(
                    canvas(
                        |_, _, _| {},
                        move |bounds, _, window, cx| {
                            let round =
                                |bounds: gpui::Bounds<gpui::Pixels>| bounds.size.width / 2.0;
                            let Some(t) = flash else {
                                let quad = gpui::fill(bounds, live).corner_radii(round(bounds));
                                match motion::breathing(0.35, 1.0, cx) {
                                    Some(animation) => window.paint_animated_quad(quad, animation),
                                    None => window.paint_quad(quad),
                                }
                                return;
                            };
                            let spread = px(12.0 * t);
                            let ring = gpui::Bounds::new(
                                gpui::point(bounds.origin.x - spread, bounds.origin.y - spread),
                                gpui::size(
                                    bounds.size.width + spread * 2.0,
                                    bounds.size.height + spread * 2.0,
                                ),
                            );
                            window.paint_quad(
                                gpui::fill(ring, amber.opacity(0.9 * (1.0 - t)))
                                    .corner_radii(round(ring)),
                            );
                            window.paint_quad(
                                gpui::fill(bounds, motion::mix(amber, live, t))
                                    .corner_radii(round(bounds)),
                            );
                        },
                    )
                    .size_full(),
                ),
            )
            .into_any_element()
    }

    fn render_status(
        &self,
        watched: usize,
        narrow: bool,
        now: Instant,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let utc = Utc::now();
        let (mode, head, right, lines, turn_since, held) = {
            let state = self.state.read(cx);
            let snapshot = state.supervisor.as_ref();
            let mode = mode(snapshot, &self.ensure);
            let empty = SupervisorSnapshot::default();
            let snap = snapshot.unwrap_or(&empty);
            let held: Vec<String> = held_wakes(snap).iter().map(|w| w.id.clone()).collect();
            let turn_since = self
                .chat_id(cx)
                .and_then(|id| state.session_for(&id))
                .and_then(|session| session.started_at)
                .filter(|_| mode == Mode::Working);
            let (head, right, lines): (_, Option<String>, Vec<(&str, Vec<Part>)>) = match &mode {
                Mode::Watching => (
                    (
                        "Watching",
                        format!("{watched} {}", plural(watched, "tree", "trees")),
                    ),
                    Some(format!(
                        "{} of {WAKES_PER_HOUR} wakes this hour",
                        wakes_this_hour(&snap.wakes, utc)
                    )),
                    vec![
                        ("Last", last_parts(snap, state, utc)),
                        ("Next", next_parts(snap, state)),
                    ],
                ),
                Mode::Working => {
                    let wake = self
                        .chat_id(cx)
                        .and_then(|id| current_wake(snap, state.sub_transcript(&id)));
                    (
                        (
                            "Working",
                            if wake.is_some() {
                                "on a wake"
                            } else {
                                "on a message"
                            }
                            .to_owned(),
                        ),
                        None,
                        wake.map(|wake| vec![("Woke", woke_parts(wake, state))])
                            .unwrap_or_default(),
                    )
                }
                Mode::Stopped => (
                    ("Stopped", "by you".to_owned()),
                    (snap.waiting > 0).then(|| {
                        format!(
                            "{} {} waiting",
                            snap.waiting,
                            plural(snap.waiting, "wake", "wakes")
                        )
                    }),
                    vec![("Held", vec![text(held_line(&held_wakes(snap)))])],
                ),
                Mode::Off => (
                    ("Off", String::new()),
                    None,
                    vec![(
                        "",
                        vec![text(match (&self.ensure, snapshot) {
                            (_, None) => "Waiting for the engine to report the Supervisor.",
                            (Ensure::Running, _) => "Starting the Supervisor…",
                            _ => "No Supervisor thread. Missions and trees run unwatched.",
                        })],
                    )],
                ),
                Mode::Failed(error) => (
                    ("Couldn't start", "the Supervisor".to_owned()),
                    None,
                    vec![("", vec![text(error.clone())])],
                ),
            };
            (mode, head, right, lines, turn_since, held)
        };
        let tone = status_tone(&mode, theme);
        let action = matches!(mode, Mode::Stopped | Mode::Off | Mode::Failed(_));
        let tint = |el: gpui::Div| el.when(action, |el| el.bg(tone.opacity(0.05)));
        let line = |key: &str, parts: Vec<Part>| {
            let row = div()
                .pl(px(28.0))
                .text_size(ui_rems(12.0))
                .line_height(ui_rems(18.0))
                .text_color(theme.text_muted);
            if key.is_empty() {
                return row.children(parts.into_iter().map(|part| match part {
                    Part::Text(value) | Part::Strong(value) => value,
                    Part::Chip(face) => face.label,
                }));
            }
            row.flex()
                .items_center()
                .gap(px(5.0))
                .min_w_0()
                .overflow_hidden()
                .whitespace_nowrap()
                .child(
                    div()
                        .flex_none()
                        .w(px(34.0))
                        .text_color(theme.text_faint)
                        .child(key.to_owned()),
                )
                .children(parts.into_iter().map(|part| {
                    match part {
                        Part::Text(value) => div().min_w_0().truncate().child(value),
                        Part::Strong(value) => div()
                            .min_w_0()
                            .truncate()
                            .font_weight(gpui::FontWeight::MEDIUM)
                            .text_color(theme.text)
                            .child(value),
                        Part::Chip(face) => div()
                            .flex_none()
                            .text_color(match face.tone {
                                Tone::Plain => theme.text_muted,
                                Tone::Ok => ok_tone(theme),
                                Tone::Warn => needs_tone(theme),
                            })
                            .child(face.label),
                    }
                }))
        };
        let label = format!("{} {}", head.0, head.1);
        let mut card = div()
            .id("supervisor-status")
            .debug_selector(|| "supervisor-status".into())
            .role(gpui::Role::Status)
            .aria_label(label)
            .rounded(px(11.0))
            .border_1()
            .border_color(if action {
                tone.opacity(0.35)
            } else {
                theme.border
            })
            .bg(widgets::block_fill(theme))
            .overflow_hidden()
            .child(
                tint(div())
                    .px(px(11.0))
                    .py(px(9.0))
                    .flex()
                    .flex_col()
                    .gap(px(3.0))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(10.0))
                            .child(self.render_dot(&mode, tone, theme, now, cx))
                            .child(
                                div()
                                    .debug_selector(|| "supervisor-state".into())
                                    .flex_1()
                                    .min_w_0()
                                    .truncate()
                                    .flex()
                                    .gap(px(5.0))
                                    .text_size(ui_rems(13.0))
                                    .font_weight(gpui::FontWeight::MEDIUM)
                                    .text_color(theme.text)
                                    .child(div().flex_none().text_color(tone).child(head.0))
                                    .when(!head.1.is_empty(), |el| el.child(head.1)),
                            )
                            .children(right.filter(|_| !narrow).map(|right| {
                                div()
                                    .flex_none()
                                    .text_size(ui_rems(11.5))
                                    .text_color(theme.text_faint)
                                    .child(right)
                            }))
                            .children(turn_since.map(|since| {
                                div()
                                    .flex_none()
                                    .flex()
                                    .items_center()
                                    .gap(px(4.0))
                                    .text_size(ui_rems(11.5))
                                    .line_height(px(16.0))
                                    .text_color(theme.text_faint)
                                    .child("turn")
                                    .child(crate::loaders::elapsed_label(
                                        "supervisor-turn",
                                        since,
                                        ui_rems(11.5),
                                        theme.text_faint,
                                    ))
                            })),
                    )
                    .children(lines.into_iter().map(|(key, parts)| line(key, parts))),
            );
        let actions = div().pl(px(28.0)).pt(px(5.0)).flex().gap(px(6.0));
        let conversation = || {
            pill_button(
                theme,
                "supervisor-open-conversation".into(),
                "Open the conversation".into(),
                widgets::ActionTone::Quiet,
            )
            .on_click(cx.listener(|page, _: &ClickEvent, _, cx| page.open_conversation(cx)))
        };
        let actions = match &mode {
            Mode::Working => Some(actions.child(conversation())),
            Mode::Stopped => Some(
                actions
                    .when(!held.is_empty(), |el| {
                        el.child(
                            pill_button(
                                theme,
                                "supervisor-resume".into(),
                                if self.resuming {
                                    "Resuming…"
                                } else {
                                    "Resume"
                                }
                                .into(),
                                widgets::ActionTone::Filled,
                            )
                            .on_click(cx.listener(
                                move |page, _: &ClickEvent, _, cx| page.resume(held.clone(), cx),
                            )),
                        )
                    })
                    .child(conversation()),
            ),
            Mode::Off if !matches!(self.ensure, Ensure::Running) => Some(
                actions.child(
                    pill_button(
                        theme,
                        "supervisor-start".into(),
                        "Start the Supervisor".into(),
                        widgets::ActionTone::Filled,
                    )
                    .on_click(cx.listener(|page, _: &ClickEvent, _, cx| page.retry_ensure(cx))),
                ),
            ),
            Mode::Failed(_) => Some(
                actions.child(
                    pill_button(
                        theme,
                        "supervisor-retry".into(),
                        "Try again".into(),
                        widgets::ActionTone::Filled,
                    )
                    .on_click(cx.listener(|page, _: &ClickEvent, _, cx| page.retry_ensure(cx))),
                ),
            ),
            _ => None,
        };
        if let Some(actions) = actions {
            card = card.child(tint(div()).px(px(11.0)).pb(px(9.0)).child(actions));
        }
        let Some(motion) = &self.status else {
            return card.into_any_element();
        };
        let probe = motion.height.clone();
        let inner = div()
            .relative()
            .w_full()
            .opacity(motion.fade.value_at(now))
            .child(card)
            .child(
                canvas(
                    move |bounds, _, _| probe.set(f32::from(bounds.size.height)),
                    |_, _, _, _| {},
                )
                .absolute()
                .inset_0(),
            );
        if motion.grow.settled_at(now) || motion.from <= 0.0 {
            return inner.into_any_element();
        }
        let t = motion.grow.value_at(now);
        let height = motion.from + (motion.height.get() - motion.from) * t;
        div()
            .id("supervisor-status-grow")
            .w_full()
            .h(px(height))
            .overflow_hidden()
            .child(inner)
            .into_any_element()
    }

    fn render_header(
        &self,
        feed: &Feed,
        status: Option<AnyElement>,
        now: Instant,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let open = self.gear.get().is_some();
        let gear = div()
            .id("supervisor-gear")
            .debug_selector(|| "supervisor-gear".into())
            .role(gpui::Role::Button)
            .aria_label("Authority and wake log")
            .aria_expanded(self.gear.is_open())
            .tooltip(widgets::text_tooltip("Authority and wake log"))
            .relative()
            .flex_none()
            .size(px(26.0))
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(7.0))
            .cursor_pointer()
            .when(open, |el| el.bg(theme.wash(0.08)))
            .hover(|s| s.bg(theme.wash(0.08)))
            .child(
                icon(icons::LUCIDE_SETTINGS)
                    .size(px(15.0))
                    .text_color(if open { theme.text } else { theme.text_muted }),
            )
            .on_mouse_down(
                gpui::MouseButton::Left,
                cx.listener(|page, _, _, _| page.gear.note_trigger_press()),
            )
            .on_click(cx.listener(|page, event: &ClickEvent, window, cx| {
                cx.stop_propagation();
                let was_open = if matches!(event, ClickEvent::Keyboard(_)) {
                    page.gear.is_open()
                } else {
                    page.gear.take_press_was_open()
                };
                if was_open {
                    page.close_gear(cx);
                } else {
                    page.open_gear(window, cx);
                }
            }))
            .when(open, |el| {
                el.child(popover::anchored_menu_below_end(
                    "supervisor-gear-menu",
                    self.render_gear_menu(theme, cx),
                    self.gear.closing_since(),
                ))
            });
        let tabs = self.render_tabs(feed, now, cx);
        div()
            .id("supervisor-header")
            .debug_selector(|| "supervisor-header".into())
            .flex()
            .flex_col()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(10.0))
                    .child(
                        icon(icons::LUCIDE_SHIELD)
                            .size(px(16.0))
                            .text_color(theme.text),
                    )
                    .child(
                        div()
                            .flex_none()
                            .text_size(ui_rems(16.0))
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .text_color(theme.text)
                            .child("Supervisor"),
                    )
                    .children(self.full_authority().then(|| {
                        div()
                            .id("supervisor-full-authority")
                            .debug_selector(|| "supervisor-full-authority".into())
                            .flex_none()
                            .px(px(6.0))
                            .py(px(1.0))
                            .rounded(px(5.0))
                            .border_1()
                            .border_color(theme.border)
                            .text_size(ui_rems(11.0))
                            .text_color(theme.text_muted)
                            .tooltip(widgets::text_tooltip(
                                "The Supervisor decides for you; change it under Authority",
                            ))
                            .child("Full authority")
                    }))
                    .child(div().flex_1())
                    .child(gear),
            )
            .children(status.map(|status| div().mt(px(14.0)).child(status)))
            .child(tabs)
            .into_any_element()
    }

    fn full_authority(&self) -> bool {
        matches!(&self.settings, Loadable::Ready(settings) if settings.full_authority())
    }

    fn render_gear_menu(&self, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let popup = theme.for_popup();
        let wakes: Vec<SupervisorWake> = self
            .state
            .read(cx)
            .supervisor
            .as_ref()
            .map(|snapshot| snapshot.wakes.clone())
            .unwrap_or_default();
        popover::popover_card(&popup)
            .id("supervisor-gear-card")
            .debug_selector(|| "supervisor-gear-menu".into())
            .track_focus(&self.gear_focus)
            .role(gpui::Role::Dialog)
            .aria_label("Authority and wake log")
            .w(px(340.0))
            .flex()
            .flex_col()
            .on_mouse_down_out(cx.listener(|page, _, _, cx| page.close_gear(cx)))
            .on_key_down(cx.listener(|page, event: &gpui::KeyDownEvent, _, cx| {
                if event.keystroke.key == "escape" {
                    cx.stop_propagation();
                    page.close_gear(cx);
                }
            }))
            .child(popover::menu_heading(&popup, "Authority"))
            .child(self.render_authority(&popup, cx))
            .child(popover::menu_separator())
            .child(popover::menu_heading(
                &popup,
                &format!("Wake log · {}", wakes.len()),
            ))
            .child(render_wake_log(&wakes, &popup))
            .into_any_element()
    }

    pub(crate) fn render_switch(
        theme: &Theme,
        key: String,
        label: SharedString,
        on: bool,
    ) -> gpui::Stateful<gpui::Div> {
        let selector = key.clone();
        widgets::toggle_switch(theme, on, key.clone())
            .id(SharedString::from(key))
            .debug_selector(move || selector.clone())
            .cursor_pointer()
            .tab_index(0)
            .role(gpui::Role::Switch)
            .aria_label(label)
            .aria_toggled(if on {
                gpui::Toggled::True
            } else {
                gpui::Toggled::False
            })
    }

    fn render_authority(&self, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let row = || {
            div()
                .flex()
                .items_center()
                .justify_between()
                .gap(px(10.0))
                .px(px(8.0))
                .py(px(5.0))
                .text_size(ui_rems(12.0))
                .text_color(theme.text)
        };
        let settings = match &self.settings {
            Loadable::Ready(settings) => *settings,
            Loadable::Error(error) => {
                return div()
                    .id("supervisor-authority")
                    .debug_selector(|| "supervisor-authority".into())
                    .px(px(4.0))
                    .child(widgets::error_strip(
                        theme,
                        format!("Could not read the Supervisor's authority: {error}"),
                    ))
                    .into_any_element();
            }
            _ => {
                return row()
                    .id("supervisor-authority")
                    .text_color(theme.text_muted)
                    .child("Loading…")
                    .into_any_element();
            }
        };
        type Set = fn(SupervisorSettings, bool) -> SupervisorSettings;
        let mut toggles: Vec<(&'static str, &'static str, bool, Set)> = vec![
            (
                "supervisor-auth-message",
                "Message a Lead or Peer directly",
                settings.message_threads,
                |s, on| SupervisorSettings {
                    message_threads: on,
                    ..s
                },
            ),
            (
                "supervisor-auth-rebrief",
                "Re-brief a step, through its Lead",
                settings.rebrief,
                |s, on| SupervisorSettings { rebrief: on, ..s },
            ),
        ];
        if !cfg!(windows) {
            toggles.push((
                "supervisor-auth-pause",
                "Pause a step that blocks a shared resource",
                settings.pause_steps,
                |s, on| SupervisorSettings {
                    pause_steps: on,
                    ..s
                },
            ));
        }
        toggles.push((
            "supervisor-auth-start",
            "Start a new mission",
            settings.start_missions,
            |s, on| SupervisorSettings {
                start_missions: on,
                ..s
            },
        ));
        toggles.push((
            "supervisor-auth-decide",
            "Decide for me",
            settings.decide_for_me,
            |s, on| SupervisorSettings {
                decide_for_me: on,
                ..s
            },
        ));
        toggles.push((
            "supervisor-auth-goal",
            "Change a mission's goal or spend",
            settings.goal_and_spend,
            |s, on| SupervisorSettings {
                goal_and_spend: on,
                ..s
            },
        ));
        let everything = self
            .state
            .read(cx)
            .supervisor
            .as_ref()
            .is_some_and(|snapshot| snapshot.watch_everything);
        let mut list = div()
            .id("supervisor-authority")
            .debug_selector(|| "supervisor-authority".into())
            .flex()
            .flex_col()
            .child(
                row().child("Watch everything").child(
                    Self::render_switch(
                        theme,
                        "supervisor-auth-everything".into(),
                        "Watch everything".into(),
                        everything,
                    )
                    .on_click(cx.listener(
                        move |page, _: &ClickEvent, _, cx| {
                            page.set_watch_everything(!everything, cx)
                        },
                    )),
                ),
            )
            .children(
                self.watch_error_line(WATCH_EVERYTHING, theme)
                    .map(|line| line.px(px(8.0)).pb(px(4.0))),
            );
        for (key, text, on, set) in toggles {
            list = list.child(row().child(text).child(
                Self::render_switch(theme, key.into(), text.into(), on).on_click(cx.listener(
                    move |page, _: &ClickEvent, _, cx| page.save_settings(set(settings, !on), cx),
                )),
            ));
        }
        let weak = cx.entity().downgrade();
        list.child(
            row()
                .flex_col()
                .items_start()
                .gap(px(6.0))
                .child("Check in on its own")
                .child(
                    widgets::segmented(
                        theme,
                        "supervisor-auth-heartbeat",
                        &HEARTBEATS,
                        settings.heartbeat_minutes,
                        move |minutes, _, cx| {
                            weak.update(cx, |page, cx| {
                                page.save_settings(
                                    SupervisorSettings {
                                        heartbeat_minutes: minutes,
                                        ..settings
                                    },
                                    cx,
                                )
                            })
                            .ok();
                        },
                    )
                    .debug_selector(|| "supervisor-auth-heartbeat".into()),
                ),
        )
        .children(
            self.settings_error
                .clone()
                .map(|error| div().px(px(4.0)).child(widgets::error_strip(theme, error))),
        )
        .into_any_element()
    }

    fn render_chip(&self, key: &str, face: &Face, now: Instant, theme: &Theme) -> AnyElement {
        let fade = self.chips.get(key);
        let t = fade.map_or(1.0, |fade| fade.tween.value_at(now));
        let from = fade.and_then(|fade| fade.from.as_ref()).filter(|_| t < 1.0);
        let tone = |face: &Face| match face.tone {
            Tone::Plain => None,
            Tone::Ok => Some(ok_tone(theme)),
            Tone::Warn => Some(needs_tone(theme)),
        };
        let text = |face: &Face| tone(face).unwrap_or(theme.text_muted);
        let text_color = match from {
            Some(old) => motion::mix(text(old), text(face), t),
            None => text(face),
        };
        let selector = format!("supervisor-chip-face-{key}");
        div()
            .debug_selector(move || selector.clone())
            .relative()
            .flex_none()
            .text_size(ui_rems(11.5))
            .text_color(text_color)
            .whitespace_nowrap()
            .child(
                div()
                    .opacity(if from.is_some() { t } else { 1.0 })
                    .child(face.label.clone()),
            )
            .children(from.map(|old| {
                div()
                    .absolute()
                    .top_0()
                    .left_0()
                    .opacity(1.0 - t)
                    .child(old.label.clone())
            }))
            .into_any_element()
    }

    fn sync_trees(&mut self, missions: &[Mission], now: Instant, cx: &mut Context<Self>) {
        let open: Vec<(String, String, bool)> = missions
            .iter()
            .filter(|mission| {
                self.missions
                    .get(&mission.key)
                    .is_some_and(|motion| !motion.fold.gone(now))
            })
            .filter_map(|mission| Some((mission.key.clone(), mission.root.clone()?, mission.plan)))
            .collect();
        self.trees
            .retain(|id, _| open.iter().any(|(key, _, _)| key == id));
        for (key, root, plan) in open {
            if self.trees.contains_key(&key) {
                continue;
            }
            let state = self.state.clone();
            let tree = cx.new(|cx| {
                crate::tasks::StripTree::new(
                    state,
                    root,
                    true,
                    true,
                    crate::tasks::TreeHost::Supervisor,
                    cx,
                )
            });
            let id = key.clone();
            let events = cx.subscribe(
                &tree,
                move |page, _, event: &crate::tasks::StripTreeEvent, cx| match event {
                    crate::tasks::StripTreeEvent::Open {
                        chat_id: Some(chat_id),
                        ..
                    } => cx.emit(SupervisorEvent::OpenChat(chat_id.clone())),
                    crate::tasks::StripTreeEvent::Open {
                        selection,
                        chat_id: None,
                    } => cx.emit(SupervisorEvent::OpenMission(id.clone(), selection.clone())),
                    crate::tasks::StripTreeEvent::OpenNative { parent_chat_id, .. } => {
                        cx.emit(SupervisorEvent::OpenChat(parent_chat_id.clone()))
                    }
                    crate::tasks::StripTreeEvent::Brief(_) if !plan => {}
                    crate::tasks::StripTreeEvent::Brief(selection) => {
                        let step_id = match selection {
                            crate::tasks::Selection::Step(step) => Some(step.clone()),
                            _ => None,
                        };
                        page.open_drawer(
                            Owner {
                                task_id: id.clone(),
                                step_id,
                            },
                            cx,
                        );
                    }
                },
            );
            self.trees.insert(key, (tree, events));
        }
    }

    fn render_pane_status(&self, theme: &Theme, cx: &mut Context<Self>) -> Option<AnyElement> {
        let line = match &self.ensure {
            Ensure::Failed(_) => return None,
            _ if self.state.read(cx).supervisor.is_none() => {
                "Waiting for the engine to report the Supervisor."
            }
            _ => "Starting the Supervisor…",
        };
        Some(
            div()
                .id("supervisor-pane-status")
                .debug_selector(|| "supervisor-pane-status".into())
                .flex()
                .items_center()
                .gap(px(10.0))
                .py(px(14.0))
                .text_size(ui_rems(12.5))
                .text_color(theme.text_muted)
                .child(line)
                .into_any_element(),
        )
    }

    fn render_bottom(
        &self,
        now: Instant,
        theme: &Theme,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let column = div()
            .flex_none()
            .w_full()
            .max_w(px(PAGE_WIDTH))
            .mx_auto()
            .px(px(24.0))
            .flex()
            .flex_col();
        let Some(pane) = &self.pane else {
            return column
                .children(self.render_pane_status(theme, cx))
                .into_any_element();
        };
        let open = self.conversation.value_at(now);
        let preview = {
            let state = self.state.read(cx);
            let wakes = state
                .supervisor
                .as_ref()
                .map_or(&[][..], |snapshot| snapshot.wakes.as_slice());
            self.chat_id(cx)
                .and_then(|id| last_message(state.sub_transcript(&id), wakes))
        };
        let height =
            (f32::from(window.viewport_size().height) * 0.4).clamp(160.0, CONVERSATION_HEIGHT);
        let tab = preview.map(|preview| {
            crate::queue::queue_panel_surface(theme, false)
                .id("supervisor-conversation-toggle")
                .debug_selector(|| "supervisor-conversation-toggle".into())
                .role(gpui::Role::Button)
                .aria_label("Conversation")
                .aria_expanded(self.conversation_open)
                .absolute()
                .top_0()
                .left(px(16.0))
                .max_w(relative(0.55))
                .rounded_t(px(10.0))
                .px(px(8.0))
                .pb(px(crate::composer::QUEUE_COMPOSER_OVERLAP))
                .cursor_pointer()
                .hover(|s| s.opacity(0.85))
                .child(
                    div()
                        .h(px(24.0))
                        .min_w_0()
                        .flex()
                        .items_center()
                        .gap(px(6.0))
                        .text_size(px(11.0))
                        .text_color(theme.text_muted)
                        .child(chevron(theme, -90.0 * open))
                        .child(
                            div()
                                .flex_none()
                                .font_weight(gpui::FontWeight::MEDIUM)
                                .text_color(theme.text)
                                .child("Conversation"),
                        )
                        .child(
                            div()
                                .debug_selector(|| "supervisor-conversation-preview".into())
                                .min_w_0()
                                .truncate()
                                .opacity(1.0 - open)
                                .child(preview),
                        ),
                )
                .on_click(cx.listener(|page, _: &ClickEvent, _, cx| page.toggle_conversation(cx)))
        });
        column
            .when(open > 0.0, |el| {
                el.child(
                    div()
                        .id("supervisor-conversation")
                        .debug_selector(|| "supervisor-conversation".into())
                        .flex_none()
                        .w_full()
                        .h(px(height * open))
                        .overflow_hidden()
                        .child(
                            div()
                                .w_full()
                                .h(px(height))
                                .flex()
                                .flex_col()
                                .child(pane.transcript.clone()),
                        ),
                )
            })
            .child(
                div()
                    .relative()
                    .flex_none()
                    .children(tab)
                    .child(pane.composer.clone()),
            )
            .into_any_element()
    }

    fn render_drawer(
        &self,
        owner: &Owner,
        now: Instant,
        theme: &Theme,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let found = {
            let state = self.state.read(cx);
            state
                .tasks
                .iter()
                .find(|task| task.id == owner.task_id && !task.deleted)
                .map(|task| (task.clone(), rollup(state, task, Utc::now())))
        };
        let hidden = 1.0 - self.drawer_slide.value_at(now);
        let close = div()
            .id("supervisor-brief-close")
            .debug_selector(|| "supervisor-brief-close".into())
            .role(gpui::Role::Button)
            .aria_label("Close brief")
            .flex_none()
            .cursor_pointer()
            .p(px(4.0))
            .rounded(px(6.0))
            .hover(|s| s.bg(theme.glass_hover()))
            .child(
                icon(icons::CLOSE)
                    .size(px(14.0))
                    .text_color(theme.text_muted),
            )
            .on_click(cx.listener(|page, _: &ClickEvent, _, cx| page.close_drawer(cx)));
        let shell = div()
            .id("supervisor-brief")
            .debug_selector(|| "supervisor-brief".into())
            .absolute()
            .top(px(Theme::TITLEBAR_HEIGHT))
            .right(px(-(DRAWER_WIDTH + 40.0) * hidden))
            .bottom_0()
            .w(px(DRAWER_WIDTH))
            .bg(theme.bg)
            .border_l_1()
            .border_color(theme.border)
            .shadow(vec![gpui::BoxShadow {
                color: gpui::black().opacity(0.35),
                offset: gpui::point(px(-16.0), px(0.0)),
                blur_radius: px(36.0),
                spread_radius: px(0.0),
                inset: false,
            }])
            .occlude()
            .overflow_y_scroll()
            .px(px(16.0))
            .py(px(14.0))
            .flex()
            .flex_col()
            .gap(px(14.0));
        let Some((task, plan)) = found else {
            return shell
                .child(
                    div()
                        .flex()
                        .justify_between()
                        .text_color(theme.text_muted)
                        .child("This mission is gone.")
                        .child(close),
                )
                .into_any_element();
        };
        let step = owner
            .step_id
            .as_deref()
            .and_then(|id| plan.steps.iter().find(|row| row.step.id == id));
        let thread = self.owner_thread(owner, cx);
        let (chat, entries) = {
            let state = self.state.read(cx);
            let chat = thread
                .as_deref()
                .and_then(|id| state.chats.iter().find(|chat| chat.id == id))
                .cloned();
            let entries = thread
                .as_deref()
                .map(|id| state.sub_transcript(id).to_vec())
                .unwrap_or_default();
            (chat, entries)
        };
        let title = match step {
            Some(row) => format!(
                "Brief · {} · {}",
                plan.step_label(&row.step.id).unwrap_or_default(),
                row.step.title
            ),
            None => "Brief · Lead".into(),
        };
        let mut meta = vec![task.title.clone()];
        if let Some(model) = chat.as_ref().and_then(|chat| model_label(chat, cx)) {
            meta.push(model);
        }
        let started = chat.as_ref().and_then(|chat| match &chat.source_context {
            Some(source) => Some(match source.head_sha.as_deref() {
                Some(sha) => format!(
                    "started on {} at {}",
                    source.branch,
                    crate::review::short_sha(sha)
                ),
                None => format!("started on {}", source.branch),
            }),
            None => chat
                .branch
                .as_deref()
                .filter(|branch| !branch.is_empty())
                .map(|branch| format!("started on {branch}")),
        });
        let block = |label: &'static str| {
            div().flex().flex_col().gap(px(6.0)).child(
                div()
                    .text_size(ui_rems(10.5))
                    .font_weight(gpui::FontWeight::MEDIUM)
                    .text_color(theme.text_muted)
                    .child(label),
            )
        };
        let muted = |text: &'static str| {
            div()
                .text_size(ui_rems(12.5))
                .text_color(theme.text_muted)
                .child(text)
        };
        let sublabel = |text: &'static str| {
            div()
                .text_size(ui_rems(11.5))
                .text_color(theme.text_faint)
                .child(text)
        };
        let task_block = block("TASK")
            .debug_selector(|| "supervisor-brief-task".into())
            .child(
                div()
                    .text_size(ui_rems(13.0))
                    .font_weight(gpui::FontWeight::MEDIUM)
                    .text_color(theme.text)
                    .child(task.title.clone()),
            )
            .when(!task.notes.trim().is_empty(), |el| {
                el.child(TaskCenter::markdown(
                    format!("supervisor-brief-notes-{}", task.id),
                    &task.notes,
                    theme,
                    window,
                    cx,
                ))
            });
        let mut spec_block = block("SPEC").debug_selector(|| "supervisor-brief-spec".into());
        spec_block = if task.plan.trim().is_empty() {
            spec_block.child(muted("The lead wrote no overview."))
        } else {
            spec_block
                .child(sublabel("Overview"))
                .child(TaskCenter::markdown(
                    format!("supervisor-brief-overview-{}", task.id),
                    &task.plan,
                    theme,
                    window,
                    cx,
                ))
        };
        if let Some(row) = step {
            spec_block = spec_block.child(sublabel("Step notes"));
            spec_block = if row.step.notes.trim().is_empty() {
                spec_block.child(muted("No spec yet."))
            } else {
                spec_block.child(TaskCenter::markdown(
                    format!("supervisor-brief-step-{}", row.step.id),
                    &row.step.notes,
                    theme,
                    window,
                    cx,
                ))
            };
        }
        let received = thread.as_deref().map(|own| {
            let labels = step_labels(&plan);
            messages_it_got(&entries, own, task.chat_id.as_deref(), &|chat_id| {
                labels
                    .iter()
                    .find(|(id, _)| id == chat_id)
                    .map(|(_, label)| label.clone())
            })
        });
        let mut messages =
            block("MESSAGES IT GOT").debug_selector(|| "supervisor-brief-messages".into());
        messages = match received {
            None => messages.child(muted("This step has no thread yet.")),
            Some(received) if received.is_empty() => messages.child(muted("No messages yet.")),
            Some(received) => {
                messages.children(received.into_iter().enumerate().map(|(ix, message)| {
                    let you = message.sender == Sender::You;
                    div()
                        .debug_selector(move || format!("supervisor-brief-message-{ix}"))
                        .flex()
                        .flex_col()
                        .gap(px(4.0))
                        .py(px(4.0))
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap(px(6.0))
                                .child(
                                    div()
                                        .flex_none()
                                        .px(px(6.0))
                                        .rounded_full()
                                        .text_size(ui_rems(10.5))
                                        .bg(if you {
                                            theme.accent.opacity(0.18)
                                        } else {
                                            theme.wash(0.08)
                                        })
                                        .text_color(if you {
                                            theme.accent
                                        } else {
                                            theme.text_muted
                                        })
                                        .child(message.sender.tag()),
                                )
                                .children(message.at.map(|at| {
                                    div()
                                        .text_size(ui_rems(10.5))
                                        .text_color(theme.text_faint)
                                        .child(clock(at))
                                })),
                        )
                        .child(
                            div()
                                .text_size(ui_rems(12.5))
                                .line_height(ui_rems(18.0))
                                .text_color(theme.text)
                                .child(if message.text.is_empty() {
                                    "(attachment)".to_owned()
                                } else {
                                    clip(&message.text)
                                }),
                        )
                }))
            }
        };
        shell
            .child(
                div()
                    .flex()
                    .items_start()
                    .gap(px(8.0))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .gap(px(3.0))
                            .child(
                                div()
                                    .debug_selector(|| "supervisor-brief-title".into())
                                    .text_size(ui_rems(14.0))
                                    .font_weight(gpui::FontWeight::SEMIBOLD)
                                    .text_color(theme.text)
                                    .child(title),
                            )
                            .child(
                                div()
                                    .text_size(ui_rems(12.0))
                                    .text_color(theme.text_muted)
                                    .child(meta.join(" · ")),
                            )
                            .child(
                                div()
                                    .debug_selector(|| "supervisor-brief-started".into())
                                    .text_size(ui_rems(12.0))
                                    .text_color(theme.text_muted)
                                    .child(started.unwrap_or_else(|| "Not started".into())),
                            ),
                    )
                    .child(close),
            )
            .child(task_block)
            .child(spec_block)
            .child(messages)
            .into_any_element()
    }
}

fn status_tone(mode: &Mode, theme: &Theme) -> Hsla {
    match mode {
        Mode::Watching => ok_tone(theme),
        Mode::Working => crate::shell::chat_status_tone(ChatIndicator::Working, theme).2,
        Mode::Stopped => needs_tone(theme),
        Mode::Off => theme.text_faint,
        Mode::Failed(_) => theme.danger,
    }
}

fn needs_tone(theme: &Theme) -> Hsla {
    theme.warning
}

pub(crate) fn ok_tone(theme: &Theme) -> Hsla {
    crate::shell::chat_status_tone(ChatIndicator::Completed, theme).2
}

fn kind_tone(theme: &Theme, kind: Kind) -> Option<Hsla> {
    let dark = theme.appearance == crate::theme::Appearance::Dark;
    match kind {
        Kind::Needs => Some(needs_tone(theme)),
        Kind::Question => Some(crate::shell::chat_status_tone(ChatIndicator::Working, theme).2),
        Kind::Intervention => Some(gpui::rgb(if dark { 0xc084fc } else { 0x9333ea }).into()),
        Kind::Resource => None,
    }
}

fn pill_button(
    theme: &Theme,
    key: String,
    label: String,
    tone: widgets::ActionTone,
) -> gpui::Stateful<gpui::Div> {
    let selector = key.clone();
    widgets::text_action(theme, tone, label.clone())
        .id(SharedString::from(key))
        .debug_selector(move || selector.clone())
        .role(gpui::Role::Button)
        .aria_label(label)
        .flex_none()
        .min_h(px(28.0))
        .rounded_full()
}

fn done_key(key: &str, done: bool) -> String {
    if done {
        format!("earlier-{key}")
    } else {
        key.to_owned()
    }
}

fn chevron(theme: &Theme, degrees: f32) -> gpui::Svg {
    icon(icons::LUCIDE_CHEVRON_RIGHT)
        .size(px(12.0))
        .text_color(theme.text_faint)
        .with_transformation(gpui::Transformation::rotate(gpui::radians(
            degrees.to_radians(),
        )))
}

fn caps_label(theme: &Theme, text: String) -> gpui::Div {
    div()
        .px(px(2.0))
        .text_size(ui_rems(10.5))
        .text_color(theme.text_faint)
        .child(text.to_uppercase())
}

fn calm_line(
    snapshot: Option<&SupervisorSnapshot>,
    feed: &Feed,
    filter: Filter,
    watched: usize,
    missions: usize,
) -> (String, String) {
    let head = "Nothing needs you".to_owned();
    match filter {
        Filter::Only(kind) if !feed.open.is_empty() => {
            let head = match kind {
                Kind::Needs => head,
                kind => format!("No {}", kind.chip().to_lowercase()),
            };
            return (head, format!("{} open under All", feed.open.len()));
        }
        _ => {}
    }
    if snapshot.is_none() {
        return (
            head,
            "Waiting for the engine to report the Supervisor.".into(),
        );
    }
    if live_state(snapshot) == SupervisorState::Off {
        return (
            head,
            "The Supervisor is off. Anything that needs you lands here once it starts.".into(),
        );
    }
    let line = match (missions, watched) {
        (0, _) => {
            "Nothing watched is running. Start a mission, or Watch a tree in Mission Control."
                .into()
        }
        (_, 0) => "No tree is watched. Turn Watched on for one below.".into(),
        (_, n) => format!(
            "Watching {n} {}. A mission's open decisions arrive here as one sheet.",
            plural(n, "tree", "trees")
        ),
    };
    (head, line)
}

fn chip_face(row: &SupervisorIntervention, now: DateTime<Utc>) -> Option<Face> {
    if no_effect_yet(row, now) {
        return Some(Face {
            label: "No effect yet".into(),
            tone: Tone::Warn,
        });
    }
    row.chips.last().map(|chip| Face {
        label: chip_label(chip),
        tone: if *chip == InterventionChip::Sent {
            Tone::Plain
        } else {
            Tone::Ok
        },
    })
}

fn render_wake_log(wakes: &[SupervisorWake], theme: &Theme) -> AnyElement {
    if wakes.is_empty() {
        return div()
            .id("supervisor-wake-log")
            .debug_selector(|| "supervisor-wake-log".into())
            .px(px(8.0))
            .pb(px(8.0))
            .text_size(ui_rems(11.5))
            .text_color(theme.text_faint)
            .child("No wakes yet. Events from watched missions wake it here.")
            .into_any_element();
    }
    div()
        .id("supervisor-wake-log")
        .debug_selector(|| "supervisor-wake-log".into())
        .max_h(px(220.0))
        .overflow_y_scroll()
        .pb(px(6.0))
        .flex()
        .flex_col()
        .children(wakes.iter().map(|wake| {
            let failed = !wake.woke();
            let events: Vec<String> = wake
                .events
                .iter()
                .map(|event| event.label.clone())
                .chain(
                    wake.dropped
                        .iter()
                        .map(|event| format!("Dropped: {}", event.label)),
                )
                .collect();
            let id = wake.id.clone();
            div()
                .debug_selector(move || format!("supervisor-wake-{id}"))
                .flex()
                .items_start()
                .gap(px(8.0))
                .px(px(8.0))
                .py(px(3.0))
                .text_size(ui_rems(11.5))
                .child(
                    div()
                        .flex_none()
                        .w(px(36.0))
                        .font_family(theme.font_mono.clone())
                        .text_color(theme.text_faint)
                        .child(clock(wake.at)),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .child(
                            div()
                                .text_color(theme.text_muted)
                                .child(if events.is_empty() {
                                    "No events".to_owned()
                                } else {
                                    clip_to(&events.join("; "), TITLE_CHARS)
                                }),
                        )
                        .child(
                            div()
                                .text_color(if failed {
                                    theme.danger
                                } else {
                                    theme.text_faint
                                })
                                .child(format!(
                                    "{} {} · {}",
                                    wake.events.len(),
                                    plural(wake.events.len(), "event", "events"),
                                    delivery_label(&wake.delivery)
                                )),
                        ),
                )
        }))
        .into_any_element()
}

fn step_labels(plan: &Rollup) -> Vec<(String, String)> {
    plan.steps
        .iter()
        .filter_map(|row| Some((row.step.chat_id.clone()?, plan.step_label(&row.step.id)?)))
        .collect()
}

impl Render for SupervisorPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::of(cx).clone();
        let (now, utc) = (Instant::now(), Utc::now());
        let reduced = motion::reduced_motion(cx);
        let missions = missions(self.state.read(cx), utc);
        let watches: Vec<Watch> = missions
            .iter()
            .filter(|m| m.done_at.is_none())
            .map(|m| m.watch.clone())
            .collect();
        let watched = watched_count(&watches);
        let (feed, calm) = {
            let snapshot = self.state.read(cx).supervisor.as_ref();
            let feed = snapshot
                .map(|snapshot| feed::feed(snapshot, &self.memory, utc))
                .unwrap_or_default();
            let calm = calm_line(snapshot, &feed, self.list_filter, watched, watches.len());
            (feed, calm)
        };
        self.sync_motion(&feed, &missions, now, reduced);
        self.sync_trees(&missions, now, cx);
        self.sync_underline(now, reduced);
        let frame = self.sync_rail(now, reduced);
        let mode = mode(self.state.read(cx).supervisor.as_ref(), &self.ensure);
        self.sync_status(
            match mode {
                Mode::Watching => "watching",
                Mode::Working => "working",
                Mode::Stopped => "stopped",
                Mode::Off => "off",
                Mode::Failed(_) => "failed",
            },
            now,
            reduced,
        );
        let compact = frame.shown != RailMode::Full;
        for (id, (tree, _)) in &self.trees {
            let lit = self.hovered.as_deref() == Some(id.as_str());
            tree.update(cx, |tree, cx| {
                tree.set_compact(compact, cx);
                tree.set_lit(lit, cx);
            });
        }
        let inline_status = (frame.shown == RailMode::Strip
            && matches!(mode, Mode::Stopped | Mode::Off | Mode::Failed(_)))
        .then(|| self.render_status(watched, false, now, &theme, cx));
        let header = enter_block(
            self.enter_progress(0, now),
            self.render_header(&feed, inline_status, now, &theme, cx),
        );
        let body = enter_block(
            self.enter_progress(1, now),
            self.render_inbox(&feed, &missions, calm, now, cx),
        );
        let column = widgets::page_column()
            .max_w(px(PAGE_WIDTH))
            .child(header)
            .child(body);
        let scroll = widgets::scroll_faded(
            "supervisor-page-scroll",
            div()
                .id("supervisor-page-scroll")
                .size_full()
                .overflow_y_scroll()
                .child(column),
        );
        let bottom = self.render_bottom(now, &theme, window, cx);
        let rail = enter_block(
            self.enter_progress(2, now),
            self.render_rail(&frame, &feed, &missions, watched, now, &theme, cx),
        )
        .h_full();
        let drawer = self
            .drawer
            .clone()
            .map(|owner| self.render_drawer(&owner, now, &theme, window, cx));
        if self.animating(now) {
            window.request_animation_frame();
        }
        let probe = self.width.clone();
        div()
            .id("supervisor-page")
            .debug_selector(|| "supervisor-page".into())
            .track_focus(&self.focus)
            .on_key_down(cx.listener(|page, event: &gpui::KeyDownEvent, window, cx| {
                let keys = &event.keystroke;
                if keys.key == "enter"
                    && keys.modifiers.platform
                    && page.focus.is_focused(window)
                    && let Some(group) = page.last_sheet.clone()
                {
                    cx.stop_propagation();
                    page.send_sheet(&group, cx);
                }
            }))
            .relative()
            .size_full()
            .flex()
            .flex_row()
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .flex()
                    .flex_col()
                    .child(div().flex_1().min_h_0().child(scroll))
                    .child(bottom),
            )
            .child(rail)
            .children(drawer)
            .child(
                canvas(
                    move |bounds, window, _| {
                        let width = f32::from(bounds.size.width);
                        if (probe.get() - width).abs() > 0.5 {
                            probe.set(width);
                            window.request_animation_frame();
                        }
                    },
                    |_, _, _, _| {},
                )
                .absolute()
                .inset_0(),
            )
    }
}

fn enter_block(progress: f32, element: impl IntoElement) -> gpui::Div {
    div()
        .relative()
        .top(px((1.0 - progress) * 6.0))
        .opacity(progress)
        .child(element)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(id: &str, role: MessageRole, from: Option<&str>, text: &str) -> SessionMessageEntry {
        SessionMessageEntry {
            id: id.into(),
            role,
            parts: vec![MessagePart::Text {
                id: format!("{id}-text"),
                text: text.into(),
            }],
            created_at: 1_760_000_000_000,
            device_id: "mac".into(),
            status: None,
            continuation_of: None,
            from_chat_id: from.map(str::to_owned),
            fork_import: false,
            answers_request_id: None,
            duration_ms: None,
        }
    }

    #[test]
    fn brief_tags_each_message_by_who_sent_it() {
        let entries = vec![
            entry("1", MessageRole::User, Some("lead"), "Build the checkout."),
            entry("2", MessageRole::Assistant, None, "On it."),
            entry("3", MessageRole::User, None, "Use Tailwind."),
            entry(
                "4",
                MessageRole::User,
                Some("supervisor-9f"),
                "Re-briefed: mock payments.",
            ),
            entry(
                "5",
                MessageRole::User,
                Some("step-2-thread"),
                "Cart store is ready.",
            ),
            entry("6", MessageRole::User, Some("step-3-thread"), "Continue."),
            entry(
                "7",
                MessageRole::User,
                Some("stranger"),
                "Port 5173 is mine.",
            ),
        ];
        let labels = |chat: &str| (chat == "step-2-thread").then(|| "Step 2 of 4".to_owned());
        let got: Vec<(Sender, String)> =
            messages_it_got(&entries, "step-3-thread", Some("lead"), &labels)
                .into_iter()
                .map(|message| (message.sender, message.text))
                .collect();
        assert_eq!(
            got,
            vec![
                (Sender::Lead, "Build the checkout.".into()),
                (Sender::You, "Use Tailwind.".into()),
                (Sender::Supervisor, "Re-briefed: mock payments.".into()),
                (
                    Sender::Step("Step 2 of 4".into()),
                    "Cart store is ready.".into()
                ),
                (Sender::Itself, "Continue.".into()),
                (Sender::Thread, "Port 5173 is mine.".into()),
            ]
        );
        assert_eq!(Sender::Step("Step 2 of 4".into()).tag(), "From Step 2 of 4");
    }

    #[test]
    fn watching_counts_local_missions_left_on() {
        let unwatched = vec!["off".to_owned()];
        let watches = vec![
            watch_of("on", Some("mac"), Some("mac"), &unwatched),
            watch_of("off", Some("mac"), Some("mac"), &unwatched),
            watch_of("remote", Some("pc"), Some("mac"), &unwatched),
            watch_of("no-lead", None, Some("mac"), &unwatched),
            watch_of("unknown-host", Some("pc"), None, &unwatched),
        ];
        assert_eq!(
            watches,
            vec![
                Watch::Watched,
                Watch::Unwatched,
                Watch::Elsewhere("pc".into()),
                Watch::Watched,
                Watch::Watched,
            ]
        );
        assert_eq!(watched_count(&watches), 3);
    }

    #[test]
    fn a_plain_tree_is_watched_only_when_picked_or_everything_is_on() {
        let chat = |id: &str, device: &str| -> Chat {
            serde_json::from_value(serde_json::json!({
                "id": id, "title": id, "deviceId": device, "archived": false,
                "createdAt": Utc::now(),
            }))
            .unwrap()
        };
        let mut state = AppState::new();
        state.local_device_id = Some("mac".into());
        state.chats = vec![
            chat("picked", "mac"),
            chat("other", "mac"),
            chat("lead", "mac"),
        ];
        state.chats.push(chat("remote", "pc"));
        state.tasks = vec![
            serde_json::from_value(serde_json::json!({
                "id": "t", "title": "t", "mode": "orchestrated", "chatId": "lead",
                "createdAt": Utc::now(), "updatedAt": Utc::now(),
            }))
            .unwrap(),
        ];
        assert_eq!(
            tree_watch(&state, "picked"),
            Watch::Unwatched,
            "no Supervisor yet"
        );
        state.supervisor = Some(SupervisorSnapshot {
            watched_roots: vec!["picked".into(), "remote".into()],
            unwatched: vec!["t".into()],
            ..Default::default()
        });
        assert_eq!(tree_watch(&state, "picked"), Watch::Watched);
        assert_eq!(tree_watch(&state, "other"), Watch::Unwatched);
        assert_eq!(tree_watch(&state, "remote"), Watch::Elsewhere("pc".into()));
        assert_eq!(tree_watch(&state, "lead"), Watch::Unwatched);
        state.supervisor.as_mut().unwrap().watch_everything = true;
        assert_eq!(tree_watch(&state, "other"), Watch::Watched);
        assert_eq!(
            tree_watch(&state, "lead"),
            Watch::Unwatched,
            "a mission left off stays off under Watch everything"
        );
    }

    #[test]
    fn needs_you_route_by_presence() {
        let secs = |n: u64| Some(Duration::from_secs(n));
        let cases = [
            (true, true, secs(5), NeedsYouRoute::Quiet),
            (true, true, None, NeedsYouRoute::Quiet),
            (true, false, secs(5), NeedsYouRoute::InApp),
            (false, true, secs(179), NeedsYouRoute::InApp),
            (false, true, secs(180), NeedsYouRoute::InApp),
            (false, true, secs(181), NeedsYouRoute::Banner),
            (true, false, secs(600), NeedsYouRoute::Banner),
            (false, false, None, NeedsYouRoute::Banner),
        ];
        for (looking, focused, since, want) in cases {
            assert_eq!(
                needs_you_route(looking, focused, since),
                want,
                "looking {looking}, focused {focused}, since {since:?}"
            );
        }
    }

    fn item(id: &str) -> NeedsYouRow {
        NeedsYouRow::Item(SupervisorItem {
            id: id.into(),
            at: Utc::now(),
            raised_by: Raiser::Lead,
            raiser_chat_id: "lead".into(),
            task_id: Some("task".into()),
            title: format!("Decide {id}"),
            why: ItemWhy::Decision,
            detail: String::new(),
            options: vec!["Yes".into(), "No".into()],
            answer: None,
        })
    }

    #[test]
    fn the_first_snapshot_seeds_silently_and_later_ones_report_new_rows() {
        let mut seen = None;
        let keys = |rows: Vec<NeedsYouRow>| rows.iter().map(row_key).collect::<Vec<_>>();
        assert!(fresh_needs_you(&mut seen, &[item("a")]).is_empty());
        assert_eq!(
            keys(fresh_needs_you(&mut seen, &[item("a"), item("b")])),
            vec!["item:b"]
        );
        assert!(fresh_needs_you(&mut seen, &[item("b")]).is_empty());
        assert_eq!(
            keys(fresh_needs_you(&mut seen, &[item("b"), item("a")])),
            vec!["item:a"]
        );
    }

    #[test]
    fn no_effect_yet_needs_sent_alone_for_ten_minutes() {
        let now = Utc::now();
        let row = |minutes: i64, chips: Vec<InterventionChip>| SupervisorIntervention {
            id: format!("i{minutes}"),
            at: now - chrono::Duration::minutes(minutes),
            kind: InterventionKind::Send,
            task_id: None,
            step_ids: Vec::new(),
            target: Some("lead".into()),
            message_id: Some("m".into()),
            request_id: None,
            item: None,
            text: "Use the mock.".into(),
            trigger: Some(InterventionTrigger::Person),
            chips,
            approval: None,
        };
        let stale = row(11, vec![InterventionChip::Sent]);
        let fresh = row(9, vec![InterventionChip::Sent]);
        let answered = row(
            11,
            vec![InterventionChip::Sent, InterventionChip::LeadRanTurn],
        );
        let unsent = row(11, Vec::new());
        assert!(no_effect_yet(&stale, now));
        assert!(!no_effect_yet(&fresh, now));
        assert!(!no_effect_yet(&answered, now));
        assert!(!no_effect_yet(&unsent, now));
        assert_eq!(
            next_effect_check(&[stale, fresh.clone(), answered, unsent], now),
            Some(fresh.at + chrono::Duration::minutes(10))
        );
    }

    #[gpui::test]
    fn a_sent_sheet_answers_its_staged_decisions_in_one_call_and_undo_keeps_one_back(
        cx: &mut gpui::TestAppContext,
    ) {
        use crate::pull_request_test_support::{self as fixture, ScriptedRpc};
        use std::sync::{Arc, Mutex};

        let runtime = fixture::runtime();
        let _guard = runtime.enter();
        let _settings = fixture::settings(cx, crate::settings::UiSettings::default());
        let calls = Arc::new(Mutex::new(Vec::<serde_json::Value>::new()));
        let rpc = ScriptedRpc::new({
            let calls = calls.clone();
            move |method, params| {
                let calls = calls.clone();
                async move {
                    match method.as_str() {
                        methods::MUTATE => {
                            calls.lock().unwrap().push(params);
                            zeron_rpc::RpcReply::value(&serde_json::json!({ "ok": true }))
                        }
                        other => Err(zeron_rpc::RpcError::UnknownMethod(other.into())),
                    }
                }
            }
        });
        let page = cx.update(|cx| {
            let state = fixture::state(cx, Some(rpc.client()));
            state.update(cx, |state, _| {
                state.supervisor = Some(SupervisorSnapshot {
                    needs_you: vec![item("a"), item("b"), item("c")],
                    ..Default::default()
                })
            });
            cx.new(|cx| SupervisorPage::new(state, cx))
        });
        page.update(cx, |page, cx| {
            let feed = feed::feed(
                page.state.read(cx).supervisor.as_ref().unwrap(),
                &page.memory,
                Utc::now(),
            );
            page.sync_motion(&feed, &[], Instant::now(), true);
            for id in ["a", "b", "c"] {
                page.picks
                    .entry(format!("item:{id}"))
                    .or_default()
                    .insert(String::new(), vec!["Yes".into()]);
            }
            page.send_sheet("task", cx);
            page.undo("item:b", cx);
        });
        cx.executor().advance_clock(UNDO_WINDOW);
        let resolved = |calls: &[serde_json::Value]| -> Vec<serde_json::Value> {
            calls
                .iter()
                .filter(|call| {
                    call["op"]
                        .as_str()
                        .is_some_and(|op| op.starts_with("resolveSupervisorItem"))
                })
                .cloned()
                .collect()
        };
        rpc.settle(cx, &runtime, |_| {
            !resolved(&calls.lock().unwrap()).is_empty()
        });
        let sent = resolved(&calls.lock().unwrap());
        assert_eq!(sent.len(), 1, "{sent:?}");
        assert_eq!(sent[0]["op"], "resolveSupervisorItems");
        let mut answers = sent[0]["answers"].as_array().cloned().unwrap_or_default();
        answers.sort_by_key(|answer| answer["itemId"].to_string());
        assert_eq!(
            answers,
            vec![
                serde_json::json!({ "itemId": "a", "choice": "Yes" }),
                serde_json::json!({ "itemId": "c", "choice": "Yes" }),
            ]
        );
        page.read_with(cx, |page, _| {
            assert!(page.replies.get("item:b").is_none());
            assert_eq!(page.staged("item:b", ""), vec!["Yes".to_owned()]);
        });
    }

    #[gpui::test]
    fn a_filter_picked_and_unpicked_inside_the_fade_leaves_the_list_shown(
        cx: &mut gpui::TestAppContext,
    ) {
        use crate::pull_request_test_support as fixture;

        let runtime = fixture::runtime();
        let _guard = runtime.enter();
        let _settings = fixture::settings(cx, crate::settings::UiSettings::default());
        let page = cx.update(|cx| {
            let state = fixture::state(cx, None);
            cx.new(|cx| SupervisorPage::new(state, cx))
        });
        page.update(cx, |page, cx| {
            page.pick_filter(Some(Kind::Needs), cx);
            page.pick_filter(Some(Kind::Needs), cx);
            let later = Instant::now() + Duration::from_secs(1);
            page.sync_motion(&Feed::default(), &[], later, false);
            assert!(page.list_switch.is_none());
            assert!(!page.animating(later));
        });
    }

    #[test]
    fn take_back_is_offered_only_before_a_step_starts() {
        let now = Utc::now();
        let mut state = AppState::new();
        let approval = |approval: PlanApproval| SupervisorIntervention {
            id: "a".into(),
            at: now,
            kind: InterventionKind::Approve,
            task_id: Some("t".into()),
            step_ids: Vec::new(),
            target: Some("lead".into()),
            message_id: None,
            request_id: None,
            item: None,
            text: String::new(),
            trigger: None,
            chips: Vec::new(),
            approval: Some(approval),
        };
        let held = approval(PlanApproval::Held {
            starts_at: now + chrono::Duration::seconds(25),
        });
        let sent = approval(PlanApproval::Sent);
        let taken = approval(PlanApproval::TakenBack { at: now });
        assert_eq!(
            approval_state(&held, &state, now),
            Some(ApprovalState::Held { secs: 25 })
        );
        assert_eq!(
            approval_state(&sent, &state, now),
            Some(ApprovalState::Sent)
        );
        assert_eq!(
            approval_state(&taken, &state, now),
            Some(ApprovalState::TakenBack)
        );
        state.task_steps = vec![
            serde_json::from_value(serde_json::json!({
                "id": "s", "taskId": "t", "title": "s", "orderKey": "a",
                "chatId": "step-chat", "createdAt": now, "updatedAt": now,
            }))
            .unwrap(),
        ];
        for row in [&held, &sent] {
            let started = approval_state(row, &state, now);
            assert_eq!(started, Some(ApprovalState::Started));
            assert!(!started.unwrap().can_take_back());
        }
        assert_eq!(
            approval_state(&taken, &state, now),
            Some(ApprovalState::TakenBack)
        );
        assert!(ApprovalState::Held { secs: 1 }.can_take_back());
        assert!(ApprovalState::Sent.can_take_back());
    }

    #[gpui::test]
    fn a_fresh_install_starts_the_supervisor_from_the_harness_catalog_or_says_why_not(
        cx: &mut gpui::TestAppContext,
    ) {
        use crate::pull_request_test_support::{self as fixture, ScriptedRpc};
        use std::sync::atomic::{AtomicBool, Ordering};
        use std::sync::{Arc, Mutex};
        use zeron_engine::registry::HarnessDescriptor;
        use zeron_proto::HarnessId;

        let runtime = fixture::runtime();
        let _guard = runtime.enter();
        let _settings = fixture::settings(cx, crate::settings::UiSettings::default());
        for (installed, listed) in [(true, true), (false, true), (true, false)] {
            let listed = Arc::new(AtomicBool::new(listed));
            let ensured = Arc::new(Mutex::new(Vec::new()));
            let rpc = ScriptedRpc::new({
                let listed = listed.clone();
                let ensured = ensured.clone();
                move |method, params| {
                    let listed = listed.load(Ordering::SeqCst);
                    let ensured = ensured.clone();
                    async move {
                        match method.as_str() {
                            methods::LIST_AGENT_PROFILES => {
                                zeron_rpc::RpcReply::value(&serde_json::json!({ "profiles": [] }))
                            }
                            methods::LIST_HARNESSES if listed => {
                                zeron_rpc::RpcReply::value(&[HarnessDescriptor {
                                    id: HarnessId::ClaudeCode,
                                    name: "Claude Code".into(),
                                    supports_steering: false,
                                    steering_mode: zeron_proto::SteeringMode::StepBoundary,
                                    reasoning_levels: Vec::new(),
                                    installed,
                                    can_install: false,
                                    enabled: Some(true),
                                }])
                            }
                            methods::ENSURE_SUPERVISOR => {
                                ensured.lock().unwrap().push(params["config"].clone());
                                zeron_rpc::RpcReply::value(
                                    &serde_json::json!({ "chatId": "supervisor-1" }),
                                )
                            }
                            other => Err(zeron_rpc::RpcError::UnknownMethod(other.into())),
                        }
                    }
                }
            });
            let page = cx.update(|cx| {
                let state = fixture::state(cx, Some(rpc.client()));
                state.update(cx, |state, _| {
                    state.supervisor = Some(SupervisorSnapshot::default())
                });
                cx.new(|cx| SupervisorPage::new(state, cx))
            });
            let settled = |cx: &mut gpui::TestAppContext| {
                page.read_with(cx, |page, _| {
                    page.ensured.is_some() || matches!(page.ensure, Ensure::Failed(_))
                })
            };
            page.update(cx, |page, cx| page.entered(cx));
            rpc.settle(cx, &runtime, settled);
            if !listed.load(Ordering::SeqCst) {
                assert!(page.read_with(cx, |page, _| matches!(page.ensure, Ensure::Failed(_))));
                listed.store(true, Ordering::SeqCst);
                page.update(cx, |page, cx| page.retry_ensure(cx));
                rpc.settle(cx, &runtime, settled);
            }
            let configs = ensured.lock().unwrap().clone();
            if installed {
                assert_eq!(
                    page.read_with(cx, |page, _| page.ensured.clone())
                        .as_deref(),
                    Some("supervisor-1")
                );
                let config: ChatConfig = serde_json::from_value(configs[0].clone()).unwrap();
                assert_eq!(config.harness, HarnessId::ClaudeCode);
                assert_eq!(config.permission_mode, PermissionMode::Full);
                let state = page.read_with(cx, |page, _| page.state.clone());
                for chat_id in [Some("supervisor-1"), None] {
                    state.update(cx, |state, cx| {
                        state.supervisor = Some(SupervisorSnapshot {
                            chat_id: chat_id.map(str::to_owned),
                            ..Default::default()
                        });
                        cx.notify();
                    });
                }
                rpc.settle(cx, &runtime, settled);
                assert_eq!(ensured.lock().unwrap().len(), 2);
            } else {
                assert!(configs.is_empty());
                let ensure = page.read_with(cx, |page, _| page.ensure.clone());
                assert!(matches!(ensure, Ensure::Failed(_)), "{ensure:?}");
            }
        }
    }

    #[gpui::test]
    fn try_again_reads_the_supervisor_profile_again_before_it_starts_the_supervisor(
        cx: &mut gpui::TestAppContext,
    ) {
        use crate::pull_request_test_support::{self as fixture, ScriptedRpc};
        use std::sync::{Arc, Mutex};

        const OFF: &str = "Codex is turned off in Settings > Providers. Turn it on, or give the slp-supervisor profile another agent.";
        let runtime = fixture::runtime();
        let _guard = runtime.enter();
        let _settings = fixture::settings(cx, crate::settings::UiSettings::default());
        let harness = Arc::new(Mutex::new("codex"));
        let ensured = Arc::new(Mutex::new(Vec::new()));
        let rpc = ScriptedRpc::new({
            let harness = harness.clone();
            let ensured = ensured.clone();
            move |method, params| {
                let harness = *harness.lock().unwrap();
                let ensured = ensured.clone();
                async move {
                    match method.as_str() {
                        methods::LIST_AGENT_PROFILES => {
                            zeron_rpc::RpcReply::value(&serde_json::json!({ "profiles": [{
                                "id": PROFILE_ID, "name": PROFILE_ID, "harness": harness, "model": "haiku",
                            }] }))
                        }
                        methods::ENSURE_SUPERVISOR if params["config"]["harness"] == "codex" => {
                            Err(zeron_rpc::RpcError::Failed(OFF.into()))
                        }
                        methods::ENSURE_SUPERVISOR => {
                            ensured.lock().unwrap().push(params["config"].clone());
                            zeron_rpc::RpcReply::value(
                                &serde_json::json!({ "chatId": "supervisor-1" }),
                            )
                        }
                        other => Err(zeron_rpc::RpcError::UnknownMethod(other.into())),
                    }
                }
            }
        });
        let page = cx.update(|cx| {
            let state = fixture::state(cx, Some(rpc.client()));
            state.update(cx, |state, _| {
                state.supervisor = Some(SupervisorSnapshot::default())
            });
            cx.new(|cx| SupervisorPage::new(state, cx))
        });
        let settled = |cx: &mut gpui::TestAppContext| {
            page.read_with(cx, |page, _| {
                page.ensured.is_some() || matches!(page.ensure, Ensure::Failed(_))
            })
        };
        page.update(cx, |page, cx| page.entered(cx));
        rpc.settle(cx, &runtime, settled);
        assert_eq!(
            page.read_with(cx, |page, _| page.ensure.clone()),
            Ensure::Failed(OFF.into())
        );
        *harness.lock().unwrap() = "claude-code";
        page.update(cx, |page, cx| page.retry_ensure(cx));
        rpc.settle(cx, &runtime, settled);
        assert_eq!(
            page.read_with(cx, |page, _| page.ensured.clone())
                .as_deref(),
            Some("supervisor-1"),
            "Try again starts the Supervisor from the profile as it is now"
        );
        let config: ChatConfig =
            serde_json::from_value(ensured.lock().unwrap()[0].clone()).unwrap();
        assert_eq!(config.harness, zeron_proto::HarnessId::ClaudeCode);
        assert_eq!(config.profile_id.as_deref(), Some(PROFILE_ID));
    }
}
