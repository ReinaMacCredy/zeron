mod feed;

use std::cell::Cell;
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
    LeadQuestion, NeedsYouInput, NeedsYouRow, QuestionState, Raiser, ResourceState, SettledBy,
    SharedResource, SupervisorEventKind, SupervisorIntervention, SupervisorItem,
    SupervisorSettings, SupervisorSnapshot, SupervisorState, SupervisorWake, WakeDelivery,
    is_supervisor_chat,
};
use zeron_proto::{
    AgentProfile, Chat, ChatConfig, ChatIndicator, PermissionMode, PlanProgress, SandboxLevel,
    StepStatus, TaskStatus, UserInputAnswer, UserInputQuestion,
};
use zeron_rpc::methods;

use crate::composer::Composer;
use crate::home::MissionRow;
use crate::icons::{self, icon};
use crate::motion::{self, CubicBezier, MotionSpec, Tween};
use crate::pickers::Pickers;
use crate::popover::{self, Loadable, Popup};
use crate::settings::widgets;
use crate::state::AppState;
use crate::tasks::{Rollup, TaskCenter, chat_brand, linked_chat, rollup, step_chat};
use crate::theme::Theme;
use crate::transcript::Transcript;
use crate::typography::ui_rems;
use feed::{Entry, Feed, Filter, Kind, Memory, Row};

pub(crate) const PLACEHOLDER: &str = "Talk to the Supervisor about architecture and direction…";
const PROFILE_ID: &str = "slp-supervisor";
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
const EASE_PAGE: CubicBezier = CubicBezier::new(0.2, 0.9, 0.3, 1.0);
const ENTER: MotionSpec = MotionSpec::new(340, EASE_PAGE);
const ENTER_STAGGER_MS: u64 = 45;
const FILL: MotionSpec = MotionSpec::new(1000, EASE_PAGE);
const ROW_FOLD: MotionSpec = MotionSpec::new(260, EASE_PAGE);
const ARRIVE: MotionSpec = MotionSpec::new(500, EASE_PAGE);
const GLOW: MotionSpec = MotionSpec::new(1600, motion::EASE_OUT).with_delay(150);
const FOLD: MotionSpec = MotionSpec::new(300, EASE_PAGE);
const MISSION_FOLD: MotionSpec = MotionSpec::new(280, EASE_PAGE);
const CHEVRON_TURN: MotionSpec = MotionSpec::new(220, EASE_PAGE);
const DRAWER_SLIDE: MotionSpec = MotionSpec::new(300, EASE_PAGE);
const CHIP_FADE: MotionSpec = MotionSpec::new(300, motion::EASE);
const COUNT_TICK: MotionSpec = MotionSpec::new(350, EASE_PAGE);
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

fn next_effect_check(rows: &[SupervisorIntervention], now: DateTime<Utc>) -> Option<DateTime<Utc>> {
    rows.iter()
        .filter(|row| matches!(row.chips.as_slice(), [InterventionChip::Sent]))
        .map(|row| no_effect_since(row.at))
        .filter(|at| *at > now)
        .min()
}

pub enum SupervisorEvent {
    OpenTaskCenter(String),
    OpenChat(String),
    WatchDoc(Option<String>),
    Discuss(String),
}

#[derive(Debug, Clone, PartialEq)]
enum Reply {
    Sending,
    Sent(String),
    Failed(String),
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

fn trigger_label(trigger: &InterventionTrigger, wakes: &[SupervisorWake]) -> String {
    match trigger {
        InterventionTrigger::Person => "after your message".into(),
        InterventionTrigger::Wake { wake_id } => match wakes.iter().find(|w| &w.id == wake_id) {
            Some(wake) => format!("after the {} wake", clock(wake.at)),
            None => "after a wake".into(),
        },
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
    if let Some(task) = state
        .tasks
        .iter()
        .find(|task| task.chat_id.as_deref() == Some(chat_id))
    {
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

fn status_line(snapshot: Option<&SupervisorSnapshot>, watched: usize) -> String {
    let watching = format!(
        "watching {watched} {}",
        plural(watched, "mission", "missions")
    );
    match (live_state(snapshot), snapshot.map_or(0, |s| s.waiting)) {
        (SupervisorState::Off, _) => "Off".into(),
        (SupervisorState::Stopped, 0) => "Stopped · send a message to resume".into(),
        (SupervisorState::Stopped, n) => format!(
            "Stopped · {n} {} waiting · send a message to resume",
            plural(n, "wake", "wakes")
        ),
        (SupervisorState::Asleep, _) => format!("Asleep · {watching}"),
        (SupervisorState::Awake, _) => format!("Awake · {watching}"),
    }
}

fn last_wake_line(snapshot: Option<&SupervisorSnapshot>) -> String {
    match snapshot.and_then(|snapshot| snapshot.wakes.iter().find(|wake| wake.woke())) {
        Some(wake) => format!("Last woke {} for {}", clock(wake.at), wake_reason(wake)),
        None => "No wakes yet".into(),
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
        let who = match entry.role {
            MessageRole::User => "You",
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
    row: MissionRow,
    watch: Watch,
    current: String,
}

impl Mission {
    fn fraction(&self) -> f32 {
        let PlanProgress { done, total } = self.row.progress;
        if total == 0 {
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
    crate::home::mission_rows(state, now)
        .into_iter()
        .map(|row| {
            let task = state.tasks.iter().find(|task| task.id == row.task_id);
            let host = task
                .and_then(|task| linked_chat(&state.chats, task))
                .map(|chat| chat.device_id.as_str());
            let watch = watch_of(
                &row.task_id,
                host,
                state.local_device_id.as_deref(),
                unwatched,
            );
            let current = task
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
                .unwrap_or_else(|| row.label.clone());
            Mission {
                row,
                watch,
                current,
            }
        })
        .collect()
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

struct Fold {
    tween: Tween,
    height: Rc<Cell<f32>>,
}

impl Fold {
    fn at(open: bool, now: Instant) -> Self {
        Self {
            tween: Tween::at(if open { 1.0 } else { 0.0 }, now),
            height: Rc::new(Cell::new(0.0)),
        }
    }

    fn set(&mut self, open: bool, spec: MotionSpec, reduced: bool, now: Instant) {
        let to = if open { 1.0 } else { 0.0 };
        if self.tween.target() != to {
            self.tween.retarget(to, spec, reduced, now);
        }
    }

    fn value(&self, now: Instant) -> f32 {
        self.tween.value_at(now)
    }

    fn moving(&self, now: Instant) -> bool {
        !self.tween.settled_at(now)
    }

    fn gone(&self, now: Instant) -> bool {
        !self.moving(now) && self.tween.target() == 0.0
    }
}

fn run(from: f32, to: f32, spec: MotionSpec, reduced: bool, now: Instant) -> Tween {
    let mut tween = Tween::at(from, now);
    tween.retarget(to, spec, reduced, now);
    tween
}

struct Arrival {
    slide: Tween,
    glow: Tween,
}

struct RowMotion {
    fold: Fold,
    group: u8,
    row: Row,
    arrival: Option<Arrival>,
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
    ensure: Ensure,
    ensured: Option<String>,
    expanded: HashSet<String>,
    drawer: Option<Owner>,
    drawer_open: bool,
    drawer_slide: Tween,
    pane: Option<Pane>,
    replies: HashMap<String, Reply>,
    picks: HashMap<String, HashMap<String, Vec<String>>>,
    filter: Filter,
    memory: Memory,
    rows: HashMap<String, RowMotion>,
    arrivals: HashSet<String>,
    calm: Fold,
    ticks: HashMap<Kind, Tick>,
    chips: HashMap<String, ChipFade>,
    missions: HashMap<String, MissionMotion>,
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
                page.load_profiles(cx)
            }),
        ];
        let now = Instant::now();
        let mut page = Self {
            state,
            pickers,
            profiles: Loadable::Idle,
            settings: Loadable::Idle,
            settings_error: None,
            ensure: Ensure::Idle,
            ensured: None,
            expanded: HashSet::new(),
            drawer: None,
            drawer_open: false,
            drawer_slide: Tween::at(0.0, now),
            pane: None,
            replies: HashMap::new(),
            picks: HashMap::new(),
            filter: Filter::All,
            memory: Memory::default(),
            rows: HashMap::new(),
            arrivals: HashSet::new(),
            calm: Fold::at(false, now),
            ticks: HashMap::new(),
            chips: HashMap::new(),
            missions: HashMap::new(),
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
            _subscriptions: subscriptions,
        };
        page.load_profiles(cx);
        page.load_settings(cx);
        page.on_state(cx);
        page
    }

    pub fn entered(&mut self, cx: &mut Context<Self>) {
        self.ensured = None;
        if self.ensure != Ensure::Running {
            self.ensure = Ensure::Wanted;
        }
        self.entered_at = (!motion::reduced_motion(cx)).then(Instant::now);
        self.fresh = true;
        self.rows.clear();
        self.arrivals.clear();
        self.ticks.clear();
        self.chips.clear();
        self.missions.clear();
        self.load_settings(cx);
        self.try_ensure(cx);
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
            self.load_profiles(cx);
        }
        let known = self
            .state
            .read(cx)
            .supervisor
            .as_ref()
            .is_some_and(|snapshot| snapshot.chat_id.is_some());
        if known {
            self.ensured = None;
        }
        let choices: HashMap<String, String> = self
            .replies
            .iter()
            .filter_map(|(key, reply)| match reply {
                Reply::Sent(choice) if !choice.is_empty() => Some((key.clone(), choice.clone())),
                _ => None,
            })
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
            self.filter = Filter::All;
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
        self.picks
            .retain(|key, _| open.contains(key) || rows.contains_key(key));
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
                [
                    next_effect_check(&snapshot.interventions, now),
                    feed::next_expiry(&snapshot.interventions, now),
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

    fn resolve_item(&mut self, item_id: String, choice: String, cx: &mut Context<Self>) {
        let Some(engine) = self.state.read(cx).engine().cloned() else {
            return;
        };
        let key = format!("item:{item_id}");
        self.replies.insert(key.clone(), Reply::Sending);
        let params = serde_json::json!({
            "op": "resolveSupervisorItem", "itemId": item_id, "choice": choice,
        });
        cx.spawn(async move |page, cx| {
            let result = engine.client().call(methods::MUTATE, params).await;
            page.update(cx, |page, cx| {
                let reply = match result {
                    Ok(_) => Reply::Sent(choice),
                    Err(err) => Reply::Failed(err.to_string()),
                };
                if page.replies.contains_key(&key) {
                    page.replies.insert(key, reply);
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    fn answer_input(
        &mut self,
        input: &NeedsYouInput,
        answers: Vec<UserInputAnswer>,
        cx: &mut Context<Self>,
    ) {
        let Some(engine) = self.state.read(cx).engine().cloned() else {
            return;
        };
        let key = row_key(&NeedsYouRow::Input(input.clone()));
        let summary = answers_text(&answers);
        self.replies.insert(key.clone(), Reply::Sending);
        let (chat_id, request_id) = (input.chat_id.clone(), input.request_id.clone());
        cx.spawn(async move |page, cx| {
            let result =
                crate::composer::respond_input(&engine, &chat_id, &request_id, answers).await;
            page.update(cx, |page, cx| {
                let reply = match result {
                    Ok(()) => Reply::Sent(summary),
                    Err(err) => Reply::Failed(err),
                };
                if page.replies.contains_key(&key) {
                    page.replies.insert(key, reply);
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    fn pick_option(
        &mut self,
        input: &NeedsYouInput,
        question: &UserInputQuestion,
        option: String,
        cx: &mut Context<Self>,
    ) {
        if input.questions.len() == 1 && !question.multi_select {
            let answer = UserInputAnswer {
                question_id: question.id.clone(),
                labels: vec![option],
            };
            self.answer_input(input, vec![answer], cx);
            return;
        }
        let key = row_key(&NeedsYouRow::Input(input.clone()));
        let picked = self
            .picks
            .entry(key)
            .or_default()
            .entry(question.id.clone())
            .or_default();
        if !question.multi_select {
            picked.clear();
            picked.push(option);
        } else if let Some(at) = picked.iter().position(|label| *label == option) {
            picked.remove(at);
        } else {
            picked.push(option);
        }
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

    fn load_profiles(&mut self, cx: &mut Context<Self>) {
        let Some(engine) = self.state.read(cx).engine().cloned() else {
            return;
        };
        if matches!(self.profiles, Loadable::Idle) {
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

    fn set_supervised(&mut self, task_id: String, supervised: bool, cx: &mut Context<Self>) {
        let Some(engine) = self.state.read(cx).engine().cloned() else {
            return;
        };
        let params = serde_json::json!({
            "op": "setMissionSupervised", "taskId": task_id, "supervised": supervised,
        });
        cx.spawn(async move |page, cx| {
            if let Err(err) = engine.client().call(methods::MUTATE, params).await {
                page.update(cx, |page, cx| {
                    page.settings_error = Some(format!("Could not change the mission: {err}"));
                    cx.notify();
                })
                .ok();
            }
        })
        .detach();
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
        self.load_profiles(cx);
        self.pickers
            .update(cx, |pickers, cx| pickers.ensure_harnesses(true, cx));
        self.try_ensure(cx);
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
        self.filter = self.filter.pick(chip);
        cx.notify();
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

    fn toggle_mission(&mut self, task_id: &str, cx: &mut Context<Self>) {
        if !self.expanded.remove(task_id) {
            self.expanded.insert(task_id.to_owned());
        }
        cx.notify();
    }

    fn profile_name(&self, chat: &Chat) -> Option<String> {
        let id = chat.config.as_ref()?.profile_id.as_deref()?;
        self.profiles
            .ready()?
            .iter()
            .find(|profile| profile.id == id)
            .map(|profile| profile.name.clone())
    }

    fn sync_motion(&mut self, feed: &Feed, missions: &[Mission], now: Instant, reduced: bool) {
        let fresh = std::mem::take(&mut self.fresh);
        let mut seen: HashSet<String> = HashSet::new();
        let mut any_shown = false;
        for (group, row, shown) in feed.rows(self.filter) {
            let shown = shown && !matches!(self.replies.get(&row.key), Some(Reply::Sent(_)));
            any_shown |= shown;
            seen.insert(row.key.clone());
            if let Some(motion) = self.rows.get_mut(&row.key) {
                motion.group = group;
                motion.row = row.clone();
                motion.fold.set(shown, ROW_FOLD, reduced, now);
                if motion
                    .arrival
                    .as_ref()
                    .is_some_and(|a| a.slide.settled_at(now) && a.glow.settled_at(now))
                {
                    motion.arrival = None;
                }
                continue;
            }
            let arriving = !fresh && !reduced && self.arrivals.contains(&row.key);
            let mut fold = Fold::at(fresh && shown, now);
            fold.set(shown, ROW_FOLD, reduced, now);
            self.rows.insert(
                row.key.clone(),
                RowMotion {
                    fold,
                    group,
                    row: row.clone(),
                    arrival: arriving.then(|| Arrival {
                        slide: run(0.0, 1.0, ARRIVE, reduced, now),
                        glow: run(0.0, 1.0, GLOW, reduced, now),
                    }),
                },
            );
        }
        self.arrivals.clear();
        self.rows.retain(|key, motion| {
            if seen.contains(key) {
                return true;
            }
            motion.fold.set(false, ROW_FOLD, reduced, now);
            !motion.fold.gone(now)
        });
        if fresh {
            self.calm.tween = Tween::at(if any_shown { 0.0 } else { 1.0 }, now);
        } else {
            self.calm.set(!any_shown, FOLD, reduced, now);
        }
        for kind in Kind::chips() {
            let count = feed.count(kind);
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
            let id = &mission.row.task_id;
            let (open, fill) = (self.expanded.contains(id), mission.fraction());
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
            .retain(|id, _| missions.iter().any(|mission| &mission.row.task_id == id));
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
        self.rows.values().any(|motion| {
            motion.fold.moving(now)
                || motion
                    .arrival
                    .as_ref()
                    .is_some_and(|a| !settled(&a.slide) || !settled(&a.glow))
        }) || self.calm.moving(now)
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

    fn render_dot(&self, state: SupervisorState, theme: &Theme, now: Instant) -> AnyElement {
        let amber = needs_tone(theme);
        let base = match state {
            SupervisorState::Awake => ok_tone(theme),
            SupervisorState::Stopped => amber,
            _ => theme.text_faint,
        };
        let flash = (!self.flash.settled_at(now)).then(|| self.flash.value_at(now));
        let live = state == SupervisorState::Awake;
        div()
            .debug_selector(move || {
                if live {
                    "supervisor-dot-live".into()
                } else {
                    "supervisor-dot".into()
                }
            })
            .flex_none()
            .size(px(7.0))
            .child(
                canvas(
                    |_, _, _| {},
                    move |bounds, _, window, cx| {
                        let round = |bounds: gpui::Bounds<gpui::Pixels>| bounds.size.width / 2.0;
                        let Some(t) = flash else {
                            let quad = gpui::fill(bounds, base).corner_radii(round(bounds));
                            match live.then(|| motion::breathing(0.35, 1.0, cx)).flatten() {
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
                            gpui::fill(bounds, motion::mix(amber, base, t))
                                .corner_radii(round(bounds)),
                        );
                    },
                )
                .size_full(),
            )
            .into_any_element()
    }

    fn render_header(
        &self,
        feed: &Feed,
        watched: usize,
        now: Instant,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let (state, line, last) = {
            let snapshot = self.state.read(cx).supervisor.as_ref();
            (
                live_state(snapshot),
                status_line(snapshot, watched),
                last_wake_line(snapshot),
            )
        };
        let stopped = state == SupervisorState::Stopped;
        let amber = needs_tone(theme);
        let pill = div()
            .id("supervisor-status")
            .debug_selector(|| "supervisor-status".into())
            .role(gpui::Role::Status)
            .aria_label(line.clone())
            .tooltip(widgets::text_tooltip(last))
            .ml_auto()
            .flex_none()
            .flex()
            .items_center()
            .gap(px(7.0))
            .pl(px(8.0))
            .pr(px(10.0))
            .py(px(3.0))
            .rounded_full()
            .border_1()
            .border_color(if stopped {
                amber.opacity(0.45)
            } else {
                theme.border
            })
            .text_size(ui_rems(11.5))
            .text_color(if stopped { amber } else { theme.text_muted })
            .child(self.render_dot(state, theme, now))
            .child(
                div()
                    .debug_selector(|| "supervisor-state".into())
                    .child(line),
            );
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
        let mut chips = vec![self.render_count_chip(None, 0, now, theme, cx)];
        for kind in Kind::chips() {
            chips.push(self.render_count_chip(Some(kind), feed.count(kind), now, theme, cx));
        }
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
                    .child(pill)
                    .child(gear),
            )
            .child(
                div()
                    .id("supervisor-chips")
                    .mt(px(14.0))
                    .flex()
                    .flex_wrap()
                    .gap(px(6.0))
                    .children(chips),
            )
            .into_any_element()
    }

    fn render_count_chip(
        &self,
        kind: Option<Kind>,
        count: usize,
        now: Instant,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let selected = match kind {
            None => self.filter == Filter::All,
            Some(kind) => self.filter == Filter::Only(kind),
        };
        let hot = kind == Some(Kind::Needs) && count > 0;
        let amber = needs_tone(theme);
        let label = kind.map_or("All", Kind::chip);
        let key = format!("supervisor-chip-{}", label.to_lowercase().replace(' ', "-"));
        let selector = key.clone();
        let tick = kind
            .and_then(|kind| self.ticks.get(&kind))
            .map_or(1.0, |tick| tick.tween.value_at(now));
        div()
            .id(SharedString::from(key))
            .debug_selector(move || selector.clone())
            .role(gpui::Role::Button)
            .aria_label(match kind {
                None => "All".to_owned(),
                Some(_) => format!("{label}, {count}"),
            })
            .aria_selected(selected)
            .flex()
            .items_center()
            .gap(px(7.0))
            .px(px(10.0))
            .py(px(5.0))
            .rounded(px(9.0))
            .border_1()
            .border_color(match (hot, selected) {
                (true, _) => amber.opacity(0.55),
                (false, true) => theme.border_strong,
                (false, false) => theme.border,
            })
            .when(selected, |el| {
                el.bg(if hot {
                    amber.opacity(0.12)
                } else {
                    theme.wash(0.08)
                })
            })
            .cursor_pointer()
            .text_size(ui_rems(11.5))
            .text_color(if selected {
                theme.text
            } else {
                theme.text_muted
            })
            .hover(|s| s.text_color(theme.text))
            .child(label)
            .when(kind.is_some(), |el| {
                el.child(
                    div()
                        .h(ui_rems(14.0))
                        .line_height(ui_rems(14.0))
                        .overflow_hidden()
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .text_color(theme.text)
                        .child(
                            div()
                                .relative()
                                .top(ui_rems(14.0 * (1.0 - tick)))
                                .child(count.to_string()),
                        ),
                )
            })
            .on_click(cx.listener(move |page, _: &ClickEvent, _, cx| page.pick_filter(kind, cx)))
            .into_any_element()
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

    fn render_switch(
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
        let mut list = div()
            .id("supervisor-authority")
            .debug_selector(|| "supervisor-authority".into())
            .flex()
            .flex_col();
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
                .debug_selector(|| "supervisor-auth-goal".into())
                .child("Change a mission's goal or spend")
                .child(
                    div()
                        .flex_none()
                        .text_size(ui_rems(11.0))
                        .text_color(theme.text_faint)
                        .child("always asks you"),
                ),
        )
        .child(
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

    fn answering(&self, key: &str) -> bool {
        matches!(self.replies.get(key), Some(Reply::Sending | Reply::Sent(_)))
    }

    fn render_reply(&self, key: &str, buttons: gpui::Div, theme: &Theme) -> AnyElement {
        match self.replies.get(key) {
            Some(Reply::Failed(error)) => div()
                .flex()
                .flex_col()
                .child(buttons)
                .child(widgets::error_strip(
                    theme,
                    format!("Could not send the answer: {error}"),
                ))
                .into_any_element(),
            _ => settle_while(self.answering(key), buttons),
        }
    }

    fn render_item_card(
        &self,
        item: &SupervisorItem,
        arrival: Option<&Arrival>,
        now: Instant,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let key = format!("item:{}", item.id);
        let sel = format!("supervisor-needs-item-{}", item.id);
        let mission = mission_title(self.state.read(cx), item.task_id.as_deref());
        let raised = match item.raised_by {
            Raiser::Lead => "Raised by Lead",
            Raiser::Supervisor => "Raised by Supervisor",
        };
        let meta = match &mission {
            Some(mission) => format!("{raised} · {mission}"),
            None => raised.to_owned(),
        };
        let mut quote = vec![
            format!("Needs you: {}", item.title),
            format!("{meta} · {}", why_label(item.why)),
        ];
        if !item.detail.trim().is_empty() {
            quote.push(item.detail.trim().to_owned());
        }
        if !item.options.is_empty() {
            quote.push(format!("Options: {}", item.options.join(" | ")));
        }
        let mut buttons = div().flex().flex_wrap().items_center().gap(px(6.0));
        for (ix, option) in item.options.iter().enumerate() {
            let (id, choice) = (item.id.clone(), option.clone());
            buttons = buttons.child(
                pill_button(
                    theme,
                    format!("{sel}-option-{ix}"),
                    option.clone(),
                    if ix == 0 {
                        widgets::ActionTone::Solid
                    } else {
                        widgets::ActionTone::Outlined
                    },
                )
                .on_click(cx.listener(move |page, _: &ClickEvent, _, cx| {
                    page.resolve_item(id.clone(), choice.clone(), cx)
                })),
            );
        }
        let quote = quote.join("\n");
        buttons = buttons.child(
            pill_button(
                theme,
                format!("{sel}-discuss"),
                "Discuss".into(),
                widgets::ActionTone::Outlined,
            )
            .on_click(cx.listener(move |_, _: &ClickEvent, _, cx| {
                cx.emit(SupervisorEvent::Discuss(quote.clone()))
            })),
        );
        let body = card_body()
            .child(card_title(theme, item.title.clone(), true))
            .child(
                card_meta(theme)
                    .child(meta)
                    .child(tone_chip(
                        theme,
                        why_label(item.why),
                        Some(needs_tone(theme)),
                    ))
                    .child(clock(item.at)),
            )
            .when(!item.detail.trim().is_empty(), |el| {
                el.child(evidence(theme, clip(item.detail.trim())))
            })
            .child(
                div()
                    .mt(px(4.0))
                    .child(self.render_reply(&key, buttons, theme)),
            );
        arrive(
            card(
                theme,
                &key,
                sel,
                item.title.clone(),
                gpui::Role::Group,
                true,
            )
            .child(kind_tag(theme, Some(Kind::Needs)))
            .child(body),
            arrival,
            theme,
            now,
        )
    }

    fn render_input_card(
        &self,
        input: &NeedsYouInput,
        arrival: Option<&Arrival>,
        now: Instant,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let row = NeedsYouRow::Input(input.clone());
        let key = row_key(&row);
        let sel = format!(
            "supervisor-needs-input-{}",
            input.request_id.replace(':', "-")
        );
        let who = asker_name(input, self.state.read(cx), Utc::now());
        let approval = input.questions.iter().any(|question| question.approval);
        let picks = self.picks.get(&key);
        let submit = input.questions.len() > 1 || input.questions.iter().any(|q| q.multi_select);
        let answerable = input.questions.iter().all(|q| !q.options.is_empty());
        let mut quote = vec![
            format!("Needs you: {}", row_title(&row)),
            format!("Asked by {who}"),
        ];
        let mut questions = div().flex().flex_col().gap(px(10.0));
        for (qx, question) in input.questions.iter().enumerate() {
            quote.push(match question.options.as_slice() {
                [] => crate::transcript::single_line(&question.question),
                options => format!(
                    "{} Options: {}",
                    crate::transcript::single_line(&question.question),
                    options.join(" | ")
                ),
            });
            let picked = picks.and_then(|picks| picks.get(&question.id));
            let mut options = div().flex().flex_wrap().items_center().gap(px(6.0));
            for (ix, option) in question.options.iter().enumerate() {
                let chosen = picked.is_some_and(|labels| labels.contains(option));
                let tone = match (submit, chosen, ix) {
                    (true, true, _) => widgets::ActionTone::Solid,
                    (true, false, _) => widgets::ActionTone::Outlined,
                    (false, _, 0) => widgets::ActionTone::Solid,
                    (false, _, _) => widgets::ActionTone::Outlined,
                };
                let (row, question, option) = (input.clone(), question.clone(), option.clone());
                options = options.child(
                    pill_button(
                        theme,
                        format!("{sel}-q{qx}-option-{ix}"),
                        option.clone(),
                        tone,
                    )
                    .when(submit, |el| el.aria_selected(chosen))
                    .on_click(cx.listener(
                        move |page, _: &ClickEvent, _, cx| {
                            page.pick_option(&row, &question, option.clone(), cx)
                        },
                    )),
                );
            }
            questions = questions.child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(6.0))
                    .when(
                        input.questions.len() > 1 && !question.header.trim().is_empty(),
                        |el| {
                            el.child(
                                div()
                                    .text_size(ui_rems(12.0))
                                    .font_weight(gpui::FontWeight::MEDIUM)
                                    .text_color(theme.text)
                                    .child(question.header.clone()),
                            )
                        },
                    )
                    .when(
                        input.questions.len() > 1 || !question.header.trim().is_empty(),
                        |el| el.child(evidence(theme, clip(question.question.trim()))),
                    )
                    .child(settle_while(self.answering(&key), options)),
            );
        }
        let mut buttons = div().flex().flex_wrap().items_center().gap(px(6.0));
        if submit && answerable {
            let ready = self.picked_answers(input);
            let row = input.clone();
            buttons = buttons.child(
                pill_button(
                    theme,
                    format!("{sel}-send"),
                    "Send answers".into(),
                    widgets::ActionTone::Solid,
                )
                .when(ready.is_none(), |el| el.opacity(0.5).cursor_default())
                .on_click(cx.listener(move |page, _: &ClickEvent, _, cx| {
                    if let Some(answers) = page.picked_answers(&row) {
                        page.answer_input(&row, answers, cx);
                    }
                })),
            );
        }
        if input.asker != InputAsker::Supervisor {
            let chat_id = input.chat_id.clone();
            buttons = buttons.child(
                pill_button(
                    theme,
                    format!("{sel}-open"),
                    "Open thread".into(),
                    widgets::ActionTone::Outlined,
                )
                .on_click(cx.listener(move |_, _: &ClickEvent, _, cx| {
                    cx.emit(SupervisorEvent::OpenChat(chat_id.clone()))
                })),
            );
        }
        let quote = quote.join("\n");
        buttons = buttons.child(
            pill_button(
                theme,
                format!("{sel}-discuss"),
                "Discuss".into(),
                widgets::ActionTone::Outlined,
            )
            .on_click(cx.listener(move |_, _: &ClickEvent, _, cx| {
                cx.emit(SupervisorEvent::Discuss(quote.clone()))
            })),
        );
        let title = row_title(&row);
        let body = card_body()
            .child(card_title(theme, title.clone(), true))
            .child(
                card_meta(theme)
                    .child(format!("Asked by {who}"))
                    .child(tone_chip(
                        theme,
                        if approval { "Approval" } else { "Question" },
                        Some(needs_tone(theme)),
                    ))
                    .child(clock(input.at)),
            )
            .child(questions)
            .when(!answerable, |el| {
                el.child(
                    div()
                        .text_size(ui_rems(12.0))
                        .text_color(theme.text_faint)
                        .child(if input.asker == InputAsker::Supervisor {
                            "This question takes a typed answer: answer it in the Supervisor's conversation."
                        } else {
                            "This question takes a typed answer: answer it in its thread."
                        }),
                )
            })
            .child(
                div()
                    .mt(px(4.0))
                    .child(self.render_reply(&key, buttons, theme)),
            );
        arrive(
            card(theme, &key, sel, title, gpui::Role::Group, true)
                .child(kind_tag(theme, Some(Kind::Needs)))
                .child(body),
            arrival,
            theme,
            now,
        )
    }

    fn render_question_card(
        &self,
        key: &str,
        question: &LeadQuestion,
        done: bool,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let place = {
            let state = self.state.read(cx);
            let step = step_name(state, &question.task_id, &question.step_id, Utc::now())
                .unwrap_or_else(|| "A step".into());
            match mission_title(state, Some(&question.task_id)) {
                Some(mission) => format!("{step} → Lead · {mission}"),
                None => format!("{step} → Lead"),
            }
        };
        let (label, detail) = question_state(&question.state);
        let text = question_text(&question.questions);
        let meta = match (done, detail) {
            (false, _) | (true, None) => format!("{place} · {}", clock(question.at)),
            (true, Some(detail)) => {
                let join = if matches!(question.state, QuestionState::Dropped) {
                    ": "
                } else {
                    " "
                };
                format!("{place} · {label}{join}{detail} · {}", clock(question.at))
            }
        };
        let sel = format!(
            "{}supervisor-question-{}",
            if done { "earlier-" } else { "" },
            question.request_id.replace(':', "-")
        );
        let chat_id = question.chat_id.clone();
        card(
            theme,
            &done_key(key, done),
            sel,
            format!("{label}: {text}"),
            gpui::Role::Button,
            false,
        )
        .cursor_pointer()
        .hover(|s| s.border_color(theme.border_strong))
        .when(done, |el| el.opacity(0.55))
        .child(kind_tag(theme, (!done).then_some(Kind::Question)))
        .child(
            card_body()
                .child(card_title(theme, clip(&text), false))
                .child(card_meta(theme).child(meta)),
        )
        .on_click(cx.listener(move |_, _: &ClickEvent, _, cx| {
            cx.emit(SupervisorEvent::OpenChat(chat_id.clone()))
        }))
        .into_any_element()
    }

    fn render_intervention_card(
        &self,
        key: &str,
        row: &SupervisorIntervention,
        done: bool,
        now: Instant,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let utc = Utc::now();
        let (target, trigger) = {
            let state = self.state.read(cx);
            let wakes = state
                .supervisor
                .as_ref()
                .map_or(&[][..], |snapshot| snapshot.wakes.as_slice());
            (
                row.target
                    .as_deref()
                    .map(|target| thread_name(state, target, utc))
                    .or_else(|| mission_title(state, row.task_id.as_deref())),
                row.trigger
                    .as_ref()
                    .map(|trigger| trigger_label(trigger, wakes)),
            )
        };
        let kind = intervention_kind_label(row.kind);
        let title = if row.text.trim().is_empty() {
            kind.to_owned()
        } else {
            clip(row.text.trim())
        };
        let mut meta = vec![match target {
            Some(target) => format!("{kind} → {target}"),
            None => kind.to_owned(),
        }];
        meta.extend(trigger);
        if done {
            meta.extend(row.chips.iter().map(chip_label));
        }
        meta.push(clock(row.at));
        let face = (!done).then(|| chip_face(row, utc)).flatten();
        let label = match &face {
            Some(face) => format!("{title} · {}", face.label),
            None => title.clone(),
        };
        let id = row.id.clone();
        card(
            theme,
            &done_key(key, done),
            format!(
                "{}supervisor-intervention-{id}",
                if done { "earlier-" } else { "" }
            ),
            label,
            gpui::Role::Group,
            false,
        )
        .when(done, |el| el.opacity(0.55))
        .child(kind_tag(theme, (!done).then_some(Kind::Intervention)))
        .child(
            card_body()
                .child(card_title(theme, title, false))
                .child(card_meta(theme).child(meta.join(" · "))),
        )
        .children(face.map(|face| self.render_chip(key, &face, now, theme)))
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
        let border = |face: &Face| tone(face).map_or(theme.border, |tone| tone.opacity(0.45));
        let (text_color, border_color) = match from {
            Some(old) => (
                motion::mix(text(old), text(face), t),
                motion::mix(border(old), border(face), t),
            ),
            None => (text(face), border(face)),
        };
        let selector = format!("supervisor-chip-face-{key}");
        div()
            .debug_selector(move || selector.clone())
            .relative()
            .flex_none()
            .mt(px(1.0))
            .px(px(7.0))
            .rounded_full()
            .border_1()
            .border_color(border_color)
            .text_size(ui_rems(10.5))
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
                    .left(px(7.0))
                    .opacity(1.0 - t)
                    .child(old.label.clone())
            }))
            .into_any_element()
    }

    fn render_resource_card(
        &self,
        key: &str,
        resource: &SharedResource,
        done: bool,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let (declared, held, verified, listed) = {
            let state = self.state.read(cx);
            let listed = state.supervisor.as_ref().is_some_and(|snapshot| {
                snapshot
                    .resources
                    .iter()
                    .any(|declared| declared.key == resource.key)
            });
            let title = |id: &str| {
                state
                    .tasks
                    .iter()
                    .find(|task| task.id == id)
                    .map_or_else(|| format!("task {id}"), |task| task.title.clone())
            };
            let names = |ids: &[String]| {
                ids.iter()
                    .map(|id| title(id))
                    .collect::<Vec<_>>()
                    .join(", ")
            };
            (
                names(&resource.declared_by),
                match resource.holders.as_slice() {
                    [] => "held by none".to_string(),
                    holders => format!("held by {}", names(holders)),
                },
                verified_line(resource, &title),
                listed,
            )
        };
        let tone = match resource.state {
            _ if !listed => theme.text_muted,
            ResourceState::Contended => theme.danger,
            ResourceState::Clear => ok_tone(theme),
            _ if feed::resource_open(resource.state) => needs_tone(theme),
            _ => theme.text_muted,
        };
        let state = if listed {
            resource_label(resource.state)
        } else {
            "No longer shared by watched missions"
        };
        let name = resource.key.clone();
        card(
            theme,
            &done_key(key, done),
            format!(
                "{}supervisor-resource-{name}",
                if done { "earlier-" } else { "" }
            ),
            format!("{name} · {state}"),
            gpui::Role::Group,
            false,
        )
        .when(done, |el| el.opacity(0.55))
        .child(kind_tag(theme, (!done).then_some(Kind::Resource)))
        .child(
            card_body()
                .child(
                    div()
                        .flex()
                        .flex_wrap()
                        .gap(px(6.0))
                        .text_size(ui_rems(13.0))
                        .line_height(ui_rems(19.0))
                        .text_color(theme.text)
                        .child(
                            div()
                                .font_family(theme.font_mono.clone())
                                .child(resource.key.clone()),
                        )
                        .child(div().text_color(theme.text_faint).child("·"))
                        .child(div().text_color(tone).child(state)),
                )
                .child(card_meta(theme).child(format!("Declared by {declared} · {held}")))
                .children(verified.map(|line| card_meta(theme).child(line))),
        )
        .into_any_element()
    }

    fn render_answered_card(
        &self,
        answered: &feed::Answered,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let key = row_key(&answered.row);
        let (task_id, _) = row_origin(&answered.row);
        let mission = mission_title(self.state.read(cx), task_id);
        let mut meta = vec![match &answered.choice {
            Some(choice) => format!("Answered: {choice}"),
            None => "Answered".to_owned(),
        }];
        meta.extend(mission);
        meta.push(clock(answered.at));
        let title = row_title(&answered.row);
        card(
            theme,
            &done_key(&key, true),
            format!("earlier-supervisor-needs-{}", key.replace(':', "-")),
            title.clone(),
            gpui::Role::Group,
            false,
        )
        .opacity(0.55)
        .child(kind_tag(theme, None))
        .child(
            card_body()
                .child(card_title(theme, title, false))
                .child(card_meta(theme).child(meta.join(" · "))),
        )
        .into_any_element()
    }

    fn render_row(
        &self,
        row: &Row,
        done: bool,
        arrival: Option<&Arrival>,
        now: Instant,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        match &row.entry {
            Entry::Needs(NeedsYouRow::Item(item)) => {
                self.render_item_card(item, arrival, now, theme, cx)
            }
            Entry::Needs(NeedsYouRow::Input(input)) => {
                self.render_input_card(input, arrival, now, theme, cx)
            }
            Entry::Question(question) => {
                self.render_question_card(&row.key, question, done, theme, cx)
            }
            Entry::Intervention(intervention) => {
                self.render_intervention_card(&row.key, intervention, done, now, theme, cx)
            }
            Entry::Resource(resource) => {
                self.render_resource_card(&row.key, resource, done, theme, cx)
            }
            Entry::Answered(answered) => self.render_answered_card(answered, theme, cx),
        }
    }

    fn render_feed(
        &self,
        feed: &Feed,
        calm: String,
        now: Instant,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let mut rows: Vec<&RowMotion> = self
            .rows
            .values()
            .filter(|motion| !motion.fold.gone(now))
            .collect();
        rows.sort_by(|a, b| {
            (a.group, std::cmp::Reverse(a.row.at), &a.row.key).cmp(&(
                b.group,
                std::cmp::Reverse(b.row.at),
                &b.row.key,
            ))
        });
        let cards: Vec<AnyElement> = rows
            .into_iter()
            .map(|motion| {
                let card =
                    self.render_row(&motion.row, false, motion.arrival.as_ref(), now, theme, cx);
                folded(
                    format!("supervisor-fold-{}", motion.row.key),
                    &motion.fold,
                    now,
                    div().pb(px(6.0)).child(card),
                )
            })
            .collect();
        let calm = (!self.calm.gone(now)).then(|| {
            folded(
                "supervisor-calm-fold".into(),
                &self.calm,
                now,
                render_calm(theme, calm),
            )
        });
        div()
            .id("supervisor-feed")
            .debug_selector(|| "supervisor-feed".into())
            .mt(px(12.0))
            .flex()
            .flex_col()
            .children(cards)
            .children(calm)
            .children(self.render_earlier(&feed.earlier, now, theme, cx))
            .into_any_element()
    }

    fn render_earlier(
        &self,
        rows: &[Row],
        now: Instant,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        if rows.is_empty() {
            return None;
        }
        let header = div()
            .id("supervisor-earlier-toggle")
            .debug_selector(|| "supervisor-earlier-toggle".into())
            .role(gpui::Role::Button)
            .aria_label(format!("Earlier, {}", rows.len()))
            .aria_expanded(self.earlier_open)
            .flex()
            .items_center()
            .gap(px(6.0))
            .mt(px(4.0))
            .mx(px(2.0))
            .py(px(2.0))
            .cursor_pointer()
            .text_size(ui_rems(11.5))
            .text_color(theme.text_faint)
            .hover(|s| s.text_color(theme.text_muted))
            .child(chevron(theme, self.earlier_chevron.value_at(now) * 90.0))
            .child(format!("Earlier · {}", rows.len()))
            .on_click(cx.listener(|page, _: &ClickEvent, _, cx| page.toggle_earlier(cx)));
        let body = (!self.earlier.gone(now)).then(|| {
            let hidden = rows.len().saturating_sub(EARLIER_ROWS);
            let list = div()
                .id("supervisor-earlier")
                .debug_selector(|| "supervisor-earlier".into())
                .pt(px(6.0))
                .flex()
                .flex_col()
                .children(rows.iter().take(EARLIER_ROWS).map(|row| {
                    div()
                        .pb(px(6.0))
                        .child(self.render_row(row, true, None, now, theme, cx))
                }))
                .when(hidden > 0, |el| {
                    el.child(
                        div()
                            .px(px(2.0))
                            .pb(px(6.0))
                            .text_size(ui_rems(11.5))
                            .text_color(theme.text_faint)
                            .child(format!(
                                "{hidden} older {} not shown",
                                plural(hidden, "row", "rows")
                            )),
                    )
                });
            folded("supervisor-earlier-fold".into(), &self.earlier, now, list)
        });
        Some(
            div()
                .flex()
                .flex_col()
                .child(header)
                .children(body)
                .into_any_element(),
        )
    }

    fn render_owner(
        &self,
        owner: Owner,
        role: String,
        scope: String,
        chat: Option<&Chat>,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let (mark, tint) = chat.map(chat_brand).unwrap_or((icons::BOT, None));
        let model = chat.and_then(|chat| model_label(chat, cx));
        let profile = chat.and_then(|chat| self.profile_name(chat));
        let key = format!(
            "{}-{}",
            owner.task_id,
            owner.step_id.as_deref().unwrap_or("lead")
        );
        let group = SharedString::from(format!("supervisor-owner-{key}"));
        let brief_key = format!("supervisor-brief-{key}");
        let brief_label = format!("Brief for {role}");
        div()
            .debug_selector({
                let group = group.clone();
                move || group.to_string()
            })
            .group(group.clone())
            .flex()
            .items_center()
            .gap(px(8.0))
            .pl(px(37.0))
            .pr(px(12.0))
            .py(px(5.0))
            .text_size(ui_rems(12.0))
            .child(
                div()
                    .flex_none()
                    .w(px(62.0))
                    .truncate()
                    .text_color(theme.text_muted)
                    .child(role),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_color(if chat.is_some() {
                        theme.text
                    } else {
                        theme.text_muted
                    })
                    .child(scope),
            )
            .child(
                div()
                    .flex_none()
                    .flex()
                    .items_center()
                    .gap(px(5.0))
                    .when(chat.is_some(), |el| {
                        el.child(
                            icon(mark)
                                .size(px(12.0))
                                .text_color(tint.unwrap_or(theme.text_muted)),
                        )
                    })
                    .child(
                        div()
                            .text_size(ui_rems(11.0))
                            .text_color(theme.text_muted)
                            .child(match (chat, model) {
                                (_, Some(model)) => model,
                                (Some(_), None) => "Default model".into(),
                                (None, None) => "not started".into(),
                            }),
                    ),
            )
            .children(profile.map(|name| tone_chip(theme, name, None)))
            .child(
                div()
                    .id(SharedString::from(brief_key.clone()))
                    .debug_selector(move || brief_key.clone())
                    .role(gpui::Role::Button)
                    .aria_label(brief_label)
                    .flex_none()
                    .w(px(46.0))
                    .text_right()
                    .cursor_pointer()
                    .text_size(ui_rems(11.0))
                    .text_color(theme.accent)
                    .opacity(0.0)
                    .group_hover(group, |s| s.opacity(1.0))
                    .child("Brief ›")
                    .on_click(cx.listener(move |page, _: &ClickEvent, _, cx| {
                        cx.stop_propagation();
                        page.open_drawer(owner.clone(), cx);
                    })),
            )
            .into_any_element()
    }

    fn render_owners(
        &self,
        mission: &Mission,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let task_id = mission.row.task_id.as_str();
        let (lead, steps) = {
            let state = self.state.read(cx);
            let Some(task) = state.tasks.iter().find(|task| task.id == task_id) else {
                return div().into_any_element();
            };
            let plan = rollup(state, task, Utc::now());
            let lead = linked_chat(&state.chats, task).cloned();
            let steps: Vec<(String, String, String, Option<Chat>)> = plan
                .steps
                .iter()
                .enumerate()
                .map(|(ix, row)| {
                    (
                        row.step.id.clone(),
                        format!("Step {}", ix + 1),
                        row.step.title.clone(),
                        step_chat(&state.chats, &row.step).cloned(),
                    )
                })
                .collect();
            (lead, steps)
        };
        let lead_scope = lead
            .as_ref()
            .map(|chat| crate::transcript::single_line(&crate::home::chat_title(chat)))
            .unwrap_or_else(|| "Not started".into());
        let owner = |step_id: Option<String>| Owner {
            task_id: task_id.to_owned(),
            step_id,
        };
        let mut rows = vec![self.render_owner(
            owner(None),
            "Lead".into(),
            lead_scope,
            lead.as_ref(),
            theme,
            cx,
        )];
        for (step_id, label, title, chat) in &steps {
            rows.push(self.render_owner(
                owner(Some(step_id.clone())),
                label.clone(),
                title.clone(),
                chat.as_ref(),
                theme,
                cx,
            ));
        }
        let open_id = task_id.to_owned();
        let open_key = format!("supervisor-open-center-{task_id}");
        let footer = |el: gpui::Div| {
            el.flex()
                .items_center()
                .justify_between()
                .gap(px(10.0))
                .pl(px(37.0))
                .pr(px(12.0))
                .py(px(7.0))
                .border_t_1()
                .border_color(widgets::row_divider(theme))
                .text_size(ui_rems(11.5))
                .text_color(theme.text_muted)
        };
        let watch = match &mission.watch {
            Watch::Elsewhere(device) => {
                let name = self
                    .state
                    .read(cx)
                    .device_name(device)
                    .unwrap_or("another device")
                    .to_owned();
                footer(div()).child(format!("on {name}, not watched"))
            }
            watch => {
                let on = *watch == Watch::Watched;
                let id = task_id.to_owned();
                footer(div())
                    .child(if on {
                        "Watched by the Supervisor"
                    } else {
                        "Not watched"
                    })
                    .child(
                        Self::render_switch(
                            theme,
                            format!("supervisor-watch-{task_id}"),
                            format!("Watch {}", mission.row.title).into(),
                            on,
                        )
                        .on_click(cx.listener(
                            move |page, _: &ClickEvent, _, cx| {
                                cx.stop_propagation();
                                page.set_supervised(id.clone(), !on, cx);
                            },
                        )),
                    )
            }
        };
        div()
            .border_t_1()
            .border_color(widgets::row_divider(theme))
            .pt(px(3.0))
            .flex()
            .flex_col()
            .children(rows)
            .child(
                div().pl(px(37.0)).pt(px(3.0)).pb(px(6.0)).child(
                    div()
                        .id(SharedString::from(open_key.clone()))
                        .debug_selector(move || open_key.clone())
                        .role(gpui::Role::Button)
                        .aria_label("Open Mission Control")
                        .cursor_pointer()
                        .text_size(ui_rems(11.5))
                        .text_color(theme.text_muted)
                        .hover(|s| s.text_color(theme.text))
                        .child("Open Mission Control ›")
                        .on_click(cx.listener(move |_, _: &ClickEvent, _, cx| {
                            cx.emit(SupervisorEvent::OpenTaskCenter(open_id.clone()))
                        })),
                ),
            )
            .child(watch)
            .into_any_element()
    }

    fn render_mission(
        &self,
        mission: &Mission,
        now: Instant,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let row = &mission.row;
        let tone = |indicator| crate::shell::chat_status_tone(indicator, theme).2;
        let arc = match row.status {
            TaskStatus::NeedsYou => tone(ChatIndicator::AwaitingInput),
            TaskStatus::Working => tone(ChatIndicator::Working),
            _ => tone(ChatIndicator::Completed),
        };
        let motion = self.missions.get(&row.task_id);
        let fill = motion.map_or(mission.fraction(), |motion| motion.fill.value_at(now));
        let turn = motion.map_or(0.0, |motion| motion.chevron.value_at(now));
        let expanded = self.expanded.contains(&row.task_id);
        let unwatched = match &mission.watch {
            Watch::Watched => None,
            Watch::Unwatched => Some("Not watched".to_owned()),
            Watch::Elsewhere(device) => Some(format!(
                "on {}, not watched",
                self.state
                    .read(cx)
                    .device_name(device)
                    .unwrap_or("another device")
            )),
        };
        let header_key = format!("supervisor-mission-{}", row.task_id);
        let toggle_id = row.task_id.clone();
        let header = div()
            .id(SharedString::from(header_key.clone()))
            .debug_selector(move || header_key.clone())
            .role(gpui::Role::Button)
            .aria_label(row.title.clone())
            .aria_expanded(expanded)
            .flex()
            .items_center()
            .gap(px(9.0))
            .px(px(12.0))
            .py(px(9.0))
            .cursor_pointer()
            .hover(|s| s.bg(theme.wash(0.03)))
            .child(div().flex_none().child(crate::loaders::upload_progress_arc(
                (fill * 100.0).round() as u8,
                16.0,
                theme.text_faint.opacity(0.35),
                arc,
            )))
            .child(
                div()
                    .flex_none()
                    .max_w(px(260.0))
                    .truncate()
                    .text_size(ui_rems(13.0))
                    .text_color(theme.text)
                    .child(row.title.clone()),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_size(ui_rems(11.5))
                    .text_color(theme.text_muted)
                    .child(mission.current.clone()),
            )
            .children(unwatched.map(|label| {
                div()
                    .flex_none()
                    .text_size(ui_rems(11.0))
                    .text_color(theme.text_faint)
                    .child(label)
            }))
            .child(
                div()
                    .flex_none()
                    .w(px(64.0))
                    .h(px(3.0))
                    .rounded(px(2.0))
                    .bg(theme.wash(0.1))
                    .overflow_hidden()
                    .child(div().h_full().w(relative(fill)).bg(arc)),
            )
            .child(chevron(theme, turn * 90.0))
            .on_click(
                cx.listener(move |page, _: &ClickEvent, _, cx| page.toggle_mission(&toggle_id, cx)),
            );
        let body = motion
            .filter(|motion| !motion.fold.gone(now))
            .map(|motion| {
                folded(
                    format!("supervisor-mission-fold-{}", row.task_id),
                    &motion.fold,
                    now,
                    div()
                        .opacity(motion.fold.value(now))
                        .child(self.render_owners(mission, theme, cx)),
                )
            });
        div()
            .rounded(px(11.0))
            .border_1()
            .border_color(theme.border)
            .bg(widgets::block_fill(theme))
            .overflow_hidden()
            .flex()
            .flex_col()
            .child(header)
            .children(body)
            .into_any_element()
    }

    fn render_missions(
        &self,
        missions: &[Mission],
        now: Instant,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        if missions.is_empty() {
            return None;
        }
        Some(
            div()
                .id("supervisor-missions")
                .debug_selector(|| "supervisor-missions".into())
                .mt(px(18.0))
                .flex()
                .flex_col()
                .gap(px(7.0))
                .child(caps_label(theme, format!("Missions · {}", missions.len())))
                .children(
                    missions
                        .iter()
                        .map(|mission| self.render_mission(mission, now, theme, cx)),
                )
                .into_any_element(),
        )
    }

    fn render_pane_status(&self, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let body = div()
            .id("supervisor-pane-status")
            .debug_selector(|| "supervisor-pane-status".into())
            .flex()
            .items_center()
            .gap(px(10.0))
            .py(px(14.0))
            .text_size(ui_rems(12.5))
            .text_color(theme.text_muted);
        match &self.ensure {
            Ensure::Failed(error) => body
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .child(format!("Could not start the Supervisor: {error}")),
                )
                .child(
                    widgets::text_action(theme, widgets::ActionTone::Filled, "Try again")
                        .id("supervisor-retry")
                        .debug_selector(|| "supervisor-retry".into())
                        .role(gpui::Role::Button)
                        .aria_label("Try again")
                        .flex_none()
                        .rounded_full()
                        .on_click(cx.listener(|page, _: &ClickEvent, _, cx| page.retry_ensure(cx))),
                )
                .into_any_element(),
            _ if self.state.read(cx).supervisor.is_none() => body
                .child("Waiting for the engine to report the Supervisor.")
                .into_any_element(),
            _ => body.child("Starting the Supervisor…").into_any_element(),
        }
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
            .max_w(px(820.0))
            .mx_auto()
            .px(px(24.0))
            .flex()
            .flex_col();
        let Some(pane) = &self.pane else {
            return column
                .child(self.render_pane_status(theme, cx))
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
                .unwrap_or_else(|| "No messages yet.".into())
        };
        let height =
            (f32::from(window.viewport_size().height) * 0.4).clamp(160.0, CONVERSATION_HEIGHT);
        let bar = div()
            .id("supervisor-conversation-toggle")
            .debug_selector(|| "supervisor-conversation-toggle".into())
            .role(gpui::Role::Button)
            .aria_label("Conversation")
            .aria_expanded(self.conversation_open)
            .flex()
            .items_center()
            .gap(px(7.0))
            .pt(px(6.0))
            .pb(px(7.0))
            .cursor_pointer()
            .text_size(ui_rems(11.5))
            .text_color(theme.text_muted)
            .hover(|s| s.text_color(theme.text))
            .child(chevron(theme, -90.0 + 180.0 * open))
            .child(div().flex_none().child("Conversation"))
            .child(
                div()
                    .debug_selector(|| "supervisor-conversation-preview".into())
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_color(theme.text_muted)
                    .opacity(1.0 - open)
                    .child(preview),
            )
            .on_click(cx.listener(|page, _: &ClickEvent, _, cx| page.toggle_conversation(cx)));
        column
            .child(bar)
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
            .child(div().flex_none().child(pane.composer.clone()))
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
                    &sha[..sha.len().min(7)]
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

fn needs_tone(theme: &Theme) -> Hsla {
    theme.warning
}

fn ok_tone(theme: &Theme) -> Hsla {
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

fn tone_chip(theme: &Theme, label: impl Into<SharedString>, tone: Option<Hsla>) -> gpui::Div {
    div()
        .flex_none()
        .px(px(7.0))
        .rounded_full()
        .border_1()
        .border_color(tone.map_or(theme.border, |tone| tone.opacity(0.45)))
        .text_size(ui_rems(11.0))
        .text_color(tone.unwrap_or(theme.text_muted))
        .child(label.into())
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

// An answered card keeps its controls, inert, so it folds away without first changing height.
fn settle_while(answering: bool, controls: gpui::Div) -> AnyElement {
    if !answering {
        return controls.into_any_element();
    }
    div()
        .relative()
        .opacity(0.5)
        .child(controls)
        .child(div().absolute().inset_0().occlude())
        .into_any_element()
}

fn done_key(key: &str, done: bool) -> String {
    if done {
        format!("earlier-{key}")
    } else {
        key.to_owned()
    }
}

fn card(
    theme: &Theme,
    key: &str,
    selector: String,
    label: String,
    role: gpui::Role,
    needs: bool,
) -> gpui::Stateful<gpui::Div> {
    let tone = needs_tone(theme);
    div()
        .id(SharedString::from(format!("supervisor-card-{key}")))
        .debug_selector(move || selector.clone())
        .role(role)
        .aria_label(label)
        .relative()
        .flex()
        .items_start()
        .gap(px(10.0))
        .px(px(12.0))
        .py(px(10.0))
        .rounded(px(11.0))
        .border_1()
        .border_color(if needs {
            tone.opacity(0.4)
        } else {
            theme.border
        })
        .bg(if needs {
            tone.opacity(0.05)
        } else {
            widgets::block_fill(theme)
        })
}

fn kind_tag(theme: &Theme, kind: Option<Kind>) -> gpui::Div {
    let tone = kind.and_then(|kind| kind_tone(theme, kind));
    div()
        .flex_none()
        .mt(px(1.0))
        .px(px(7.0))
        .rounded_full()
        .text_size(ui_rems(10.5))
        .line_height(ui_rems(17.0))
        .bg(tone.map_or(theme.wash(0.08), |tone| tone.opacity(0.18)))
        .text_color(match (tone, kind) {
            (Some(tone), _) => tone,
            (None, Some(_)) => theme.text_muted,
            (None, None) => theme.text_faint,
        })
        .child(kind.map_or("Done", Kind::tag))
}

fn card_body() -> gpui::Div {
    div().flex_1().min_w_0().flex().flex_col().gap(px(3.0))
}

fn card_title(theme: &Theme, text: String, strong: bool) -> gpui::Div {
    div()
        .text_size(ui_rems(13.0))
        .line_height(ui_rems(19.0))
        .when(strong, |el| el.font_weight(gpui::FontWeight::MEDIUM))
        .text_color(theme.text)
        .child(text)
}

fn card_meta(theme: &Theme) -> gpui::Div {
    div()
        .flex()
        .flex_wrap()
        .items_center()
        .gap(px(6.0))
        .text_size(ui_rems(11.5))
        .text_color(theme.text_muted)
}

fn evidence(theme: &Theme, text: String) -> gpui::Div {
    div()
        .mt(px(3.0))
        .pl(px(9.0))
        .border_l_2()
        .border_color(theme.border)
        .text_size(ui_rems(11.5))
        .line_height(ui_rems(17.0))
        .text_color(theme.text_muted)
        .child(text)
}

fn arrive(
    card: gpui::Stateful<gpui::Div>,
    arrival: Option<&Arrival>,
    theme: &Theme,
    now: Instant,
) -> AnyElement {
    let Some(arrival) = arrival else {
        return card.into_any_element();
    };
    let slide = arrival.slide.value_at(now);
    let glow = arrival.glow.value_at(now);
    let tone = needs_tone(theme).opacity(0.5 * (1.0 - glow));
    card.top(px(-10.0 * (1.0 - slide)))
        .opacity(slide)
        .when(glow < 1.0, |el| {
            el.child(
                canvas(
                    |_, _, _| {},
                    move |bounds, _, window, _| {
                        let spread = px(8.0 * glow);
                        let ring = gpui::Bounds::new(
                            gpui::point(bounds.origin.x - spread, bounds.origin.y - spread),
                            gpui::size(
                                bounds.size.width + spread * 2.0,
                                bounds.size.height + spread * 2.0,
                            ),
                        );
                        window.paint_quad(gpui::quad(
                            ring,
                            px(11.0) + spread,
                            gpui::transparent_black(),
                            spread,
                            tone,
                            gpui::BorderStyle::default(),
                        ));
                    },
                )
                .absolute()
                .inset_0(),
            )
        })
        .into_any_element()
}

fn folded(id: String, fold: &Fold, now: Instant, body: impl IntoElement) -> AnyElement {
    let probe = fold.height.clone();
    let inner = div().relative().flex_none().w_full().child(body).child(
        canvas(
            move |bounds, _, _| {
                let height = f32::from(bounds.size.height);
                if (probe.get() - height).abs() > 0.5 {
                    probe.set(height);
                }
            },
            |_, _, _, _| {},
        )
        .absolute()
        .inset_0(),
    );
    if !fold.moving(now) {
        return inner.into_any_element();
    }
    let value = fold.value(now);
    div()
        .id(SharedString::from(id))
        .relative()
        .flex_none()
        .w_full()
        .overflow_hidden()
        .h(px(fold.height.get() * value))
        .opacity(value)
        .child(inner)
        .into_any_element()
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

fn breathing_dot(size: f32, tone: Hsla) -> AnyElement {
    div()
        .flex_none()
        .size(px(size))
        .child(
            canvas(
                |_, _, _| {},
                move |bounds, _, window, cx| {
                    let quad = gpui::fill(bounds, tone).corner_radii(bounds.size.width / 2.0);
                    match motion::breathing(0.35, 1.0, cx) {
                        Some(animation) => window.paint_animated_quad(quad, animation),
                        None => window.paint_quad(quad),
                    }
                },
            )
            .size_full(),
        )
        .into_any_element()
}

fn render_calm(theme: &Theme, line: String) -> AnyElement {
    div()
        .id("supervisor-calm")
        .debug_selector(|| "supervisor-calm".into())
        .role(gpui::Role::Group)
        .aria_label(format!("All quiet. {line}"))
        .flex()
        .flex_col()
        .items_center()
        .gap(px(7.0))
        .pt(px(26.0))
        .pb(px(18.0))
        .text_size(ui_rems(12.5))
        .text_color(theme.text_muted)
        .child(
            div()
                .size(px(32.0))
                .rounded_full()
                .border_1()
                .border_color(theme.border)
                .flex()
                .items_center()
                .justify_center()
                .child(breathing_dot(9.0, ok_tone(theme))),
        )
        .child(div().text_color(theme.text).child("All quiet"))
        .child(line)
        .into_any_element()
}

fn calm_line(
    snapshot: Option<&SupervisorSnapshot>,
    feed: &Feed,
    filter: Filter,
    watched: usize,
    missions: usize,
) -> String {
    match filter {
        Filter::Only(kind) if !feed.open.is_empty() => {
            return format!("Nothing open under {}.", kind.chip());
        }
        _ => {}
    }
    if snapshot.is_none() {
        return "Waiting for the engine to report the Supervisor.".into();
    }
    if live_state(snapshot) == SupervisorState::Off {
        return "The Supervisor is off. Anything that needs you lands here once it starts.".into();
    }
    match (missions, watched) {
        (0, _) => "No missions running. Start one from Tasks or the composer.".into(),
        (_, 0) => "No mission is watched. Turn Watched on for one below.".into(),
        (_, n) => format!(
            "Watching {n} {}. Anything that needs you lands here.",
            plural(n, "mission", "missions")
        ),
    }
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
        let watches: Vec<Watch> = missions.iter().map(|m| m.watch.clone()).collect();
        let watched = watched_count(&watches);
        let (feed, calm) = {
            let snapshot = self.state.read(cx).supervisor.as_ref();
            let feed = snapshot
                .map(|snapshot| feed::feed(snapshot, &self.memory, utc))
                .unwrap_or_default();
            let calm = calm_line(snapshot, &feed, self.filter, watched, missions.len());
            (feed, calm)
        };
        self.sync_motion(&feed, &missions, now, reduced);
        let header = enter_block(
            self.enter_progress(0, now),
            self.render_header(&feed, watched, now, &theme, cx),
        );
        let body = enter_block(
            self.enter_progress(1, now),
            self.render_feed(&feed, calm, now, &theme, cx),
        );
        let mission_block = self
            .render_missions(&missions, now, &theme, cx)
            .map(|block| enter_block(self.enter_progress(2, now), block));
        let column = widgets::page_column()
            .max_w(px(820.0))
            .child(header)
            .child(body)
            .children(mission_block);
        let scroll = widgets::scroll_faded(
            "supervisor-page-scroll",
            div()
                .id("supervisor-page-scroll")
                .size_full()
                .overflow_y_scroll()
                .child(column),
        );
        let bottom = self.render_bottom(now, &theme, window, cx);
        let drawer = self
            .drawer
            .clone()
            .map(|owner| self.render_drawer(&owner, now, &theme, window, cx));
        if self.animating(now) {
            window.request_animation_frame();
        }
        div()
            .id("supervisor-page")
            .debug_selector(|| "supervisor-page".into())
            .relative()
            .size_full()
            .flex()
            .flex_col()
            .child(div().flex_1().min_h_0().child(scroll))
            .child(bottom)
            .children(drawer)
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
            text: "Use the mock.".into(),
            trigger: Some(InterventionTrigger::Person),
            chips,
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
            } else {
                assert!(configs.is_empty());
                let ensure = page.read_with(cx, |page, _| page.ensure.clone());
                assert!(matches!(ensure, Ensure::Failed(_)), "{ensure:?}");
            }
        }
    }
}
