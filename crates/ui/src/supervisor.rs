use std::collections::{HashMap, HashSet};
use std::time::Duration;

use chrono::{DateTime, Local, Utc};
use gpui::{
    AnyElement, App, ClickEvent, Context, Entity, EventEmitter, Hsla, IntoElement, Render,
    SharedString, Subscription, Task, Window, div, prelude::*, px, relative,
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
    SessionStatus, TaskStatus, UserInputAnswer, UserInputQuestion,
};
use zeron_rpc::methods;

use crate::composer::Composer;
use crate::home::MissionRow;
use crate::icons::{self, icon};
use crate::pickers::Pickers;
use crate::popover::Loadable;
use crate::settings::widgets;
use crate::state::AppState;
use crate::tasks::{Rollup, TaskCenter, chat_brand, linked_chat, rollup, step_chat};
use crate::theme::Theme;
use crate::transcript::Transcript;
use crate::typography::ui_rems;

pub(crate) const PLACEHOLDER: &str = "Talk to the Supervisor about architecture and direction…";
const PROFILE_ID: &str = "slp-supervisor";
const ROLE: &str = "Talks direction with you, watches every mission for drift and cross-scope \
                    problems, and steps in through each Lead within the authority you set.";
const HEARTBEATS: [(u32, &str); 3] = [(0, "Off"), (30, "30 min"), (60, "60 min")];
const DRAWER_WIDTH: f32 = 380.0;
const MESSAGE_CHARS: usize = 600;
const TITLE_CHARS: usize = 120;
const RECENTLY_ACTIVE: Duration = Duration::from_secs(180);
const NO_EFFECT_AFTER_MINUTES: i64 = 10;

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

fn state_line(snapshot: Option<&SupervisorSnapshot>) -> String {
    let Some(snapshot) = snapshot.filter(|s| s.chat_id.is_some()) else {
        return "Off".into();
    };
    let word = match snapshot.state {
        SupervisorState::Off => return "Off".into(),
        SupervisorState::Stopped => {
            return match snapshot.waiting {
                0 => "Stopped · send a message to resume".into(),
                n => format!(
                    "Stopped · {n} {} waiting · send a message to resume",
                    plural(n, "wake", "wakes")
                ),
            };
        }
        SupervisorState::Asleep => "Asleep",
        SupervisorState::Awake => "Awake",
    };
    match snapshot.wakes.iter().find(|wake| wake.woke()) {
        Some(wake) => format!("{word} · woke {} for {}", clock(wake.at), wake_reason(wake)),
        None => format!("{word} · no wakes yet"),
    }
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
}

fn missions(state: &AppState, now: DateTime<Utc>) -> Vec<Mission> {
    let unwatched = state
        .supervisor
        .as_ref()
        .map_or(&[][..], |snapshot| snapshot.unwatched.as_slice());
    crate::home::mission_rows(state, now)
        .into_iter()
        .map(|row| {
            let host = state
                .tasks
                .iter()
                .find(|task| task.id == row.task_id)
                .and_then(|task| linked_chat(&state.chats, task))
                .map(|chat| chat.device_id.as_str());
            let watch = watch_of(
                &row.task_id,
                host,
                state.local_device_id.as_deref(),
                unwatched,
            );
            Mission { row, watch }
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
    wakes_open: bool,
    pane_open: bool,
    pane: Option<Pane>,
    replies: HashMap<String, Reply>,
    picks: HashMap<String, HashMap<String, Vec<String>>>,
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
            wakes_open: false,
            pane_open: true,
            pane: None,
            replies: HashMap::new(),
            picks: HashMap::new(),
            profiles_task: None,
            settings_task: None,
            ensure_task: None,
            effect_task: None,
            _subscriptions: subscriptions,
        };
        page.load_profiles(cx);
        page.load_settings(cx);
        page
    }

    pub fn entered(&mut self, cx: &mut Context<Self>) {
        self.ensured = None;
        if self.ensure != Ensure::Running {
            self.ensure = Ensure::Wanted;
        }
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
        let open: HashSet<String> = self
            .state
            .read(cx)
            .supervisor
            .as_ref()
            .map(|snapshot| snapshot.needs_you.iter().map(row_key).collect())
            .unwrap_or_default();
        self.replies.retain(|key, _| open.contains(key));
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
            .and_then(|snapshot| next_effect_check(&snapshot.interventions, now));
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
        cx.emit(SupervisorEvent::WatchDoc(thread));
        cx.notify();
    }

    fn close_drawer(&mut self, cx: &mut Context<Self>) {
        self.drawer = None;
        cx.emit(SupervisorEvent::WatchDoc(None));
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

    fn render_dot(&self, live: bool, theme: &Theme) -> AnyElement {
        let tone = if live {
            crate::shell::chat_status_tone(ChatIndicator::Completed, theme).2
        } else {
            theme.text_faint
        };
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
                gpui::canvas(
                    |_, _, _| {},
                    move |bounds, _, window, cx| {
                        let quad = gpui::fill(bounds, tone).corner_radii(bounds.size.width / 2.0);
                        match live
                            .then(|| crate::motion::breathing(0.35, 1.0, cx))
                            .flatten()
                        {
                            Some(animation) => window.paint_animated_quad(quad, animation),
                            None => window.paint_quad(quad),
                        }
                    },
                )
                .size_full(),
            )
            .into_any_element()
    }

    fn render_header(&self, watched: usize, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let (model, working, line) = {
            let state = self.state.read(cx);
            let chat = self
                .chat_id(cx)
                .and_then(|id| state.chats.iter().find(|chat| chat.id == id));
            let working = chat
                .and_then(|chat| state.session_for(&chat.id))
                .is_some_and(|session| session.status == SessionStatus::Working);
            (
                chat.and_then(|chat| model_label(chat, cx)),
                working,
                state_line(state.supervisor.as_ref()),
            )
        };
        let watching = match model {
            Some(model) => format!(
                "Watching {watched} {} · {model}",
                plural(watched, "mission", "missions")
            ),
            None => format!(
                "Watching {watched} {}",
                plural(watched, "mission", "missions")
            ),
        };
        let toggle = widgets::text_action(
            theme,
            widgets::ActionTone::Quiet,
            if self.pane_open {
                "Hide conversation"
            } else {
                "Show conversation"
            },
        )
        .id("supervisor-pane-toggle")
        .debug_selector(|| "supervisor-pane-toggle".into())
        .role(gpui::Role::Button)
        .aria_label(if self.pane_open {
            "Hide conversation"
        } else {
            "Show conversation"
        })
        .flex_none()
        .rounded_full()
        .on_click(cx.listener(|page, _: &ClickEvent, _, cx| {
            page.pane_open = !page.pane_open;
            cx.notify();
        }));
        div()
            .id("supervisor-header")
            .debug_selector(|| "supervisor-header".into())
            .px(px(8.0))
            .flex()
            .flex_col()
            .gap(px(6.0))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .child(
                        icon(icons::LUCIDE_SHIELD)
                            .size(px(18.0))
                            .text_color(theme.text),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_size(ui_rems(20.0))
                            .font_weight(gpui::FontWeight::MEDIUM)
                            .text_color(theme.text)
                            .child("Supervisor"),
                    )
                    .child(toggle),
            )
            .child(
                div()
                    .text_size(ui_rems(13.0))
                    .line_height(ui_rems(19.0))
                    .text_color(theme.text_muted)
                    .child(ROLE),
            )
            .child(
                div()
                    .debug_selector(|| "supervisor-live".into())
                    .flex()
                    .items_center()
                    .flex_wrap()
                    .gap(px(8.0))
                    .text_size(ui_rems(12.0))
                    .text_color(theme.text_muted)
                    .child(self.render_dot(working, theme))
                    .child(
                        div()
                            .debug_selector(|| "supervisor-watching".into())
                            .text_color(theme.text)
                            .child(watching),
                    )
                    .child(div().text_color(theme.text_faint).child("·"))
                    .child(
                        div()
                            .debug_selector(|| "supervisor-state".into())
                            .child(line),
                    ),
            )
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

    fn render_owner(
        &self,
        task_id: &str,
        step_id: Option<String>,
        role: String,
        scope: String,
        chat: Option<&Chat>,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let (mark, tint) = chat.map(chat_brand).unwrap_or((icons::BOT, None));
        let model = chat.and_then(|chat| model_label(chat, cx));
        let profile = chat.and_then(|chat| self.profile_name(chat));
        let key = format!("{task_id}-{}", step_id.as_deref().unwrap_or("lead"));
        let owner = Owner {
            task_id: task_id.to_owned(),
            step_id,
        };
        let brief_key = format!("supervisor-brief-{key}");
        let brief_label = format!("Brief for {role}");
        let chip = |label: String| {
            div()
                .flex_none()
                .px(px(7.0))
                .rounded_full()
                .border_1()
                .border_color(theme.border)
                .text_size(ui_rems(11.0))
                .text_color(theme.text_muted)
                .child(label)
        };
        div()
            .debug_selector({
                let key = key.clone();
                move || format!("supervisor-owner-{key}")
            })
            .flex()
            .items_center()
            .gap(px(8.0))
            .pl(px(34.0))
            .pr(px(12.0))
            .py(px(5.0))
            .text_size(ui_rems(12.5))
            .child(
                div()
                    .flex_none()
                    .w(px(96.0))
                    .truncate()
                    .text_color(theme.text_muted)
                    .child(role),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_color(theme.text)
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
                            .text_size(ui_rems(11.5))
                            .text_color(theme.text_muted)
                            .child(match (chat, model) {
                                (_, Some(model)) => model,
                                (Some(_), None) => "Default model".into(),
                                (None, None) => "No thread yet".into(),
                            }),
                    ),
            )
            .children(profile.map(chip))
            .child(
                div()
                    .id(SharedString::from(brief_key.clone()))
                    .debug_selector(move || brief_key.clone())
                    .role(gpui::Role::Button)
                    .aria_label(brief_label)
                    .flex_none()
                    .cursor_pointer()
                    .text_size(ui_rems(12.0))
                    .text_color(theme.accent)
                    .hover(|s| s.opacity(0.8))
                    .child("Brief")
                    .on_click(cx.listener(move |page, _: &ClickEvent, _, cx| {
                        cx.stop_propagation();
                        page.open_drawer(owner.clone(), cx);
                    })),
            )
            .into_any_element()
    }

    fn render_owners(&self, task_id: &str, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
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
                .map(|row| {
                    (
                        row.step.id.clone(),
                        plan.step_label(&row.step.id).unwrap_or_default(),
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
        let mut rows = vec![self.render_owner(
            task_id,
            None,
            "Lead".into(),
            lead_scope,
            lead.as_ref(),
            theme,
            cx,
        )];
        for (step_id, label, title, chat) in &steps {
            rows.push(self.render_owner(
                task_id,
                Some(step_id.clone()),
                label.clone(),
                title.clone(),
                chat.as_ref(),
                theme,
                cx,
            ));
        }
        let open_id = task_id.to_owned();
        let open_key = format!("supervisor-open-center-{task_id}");
        div()
            .border_t_1()
            .border_color(widgets::row_divider(theme))
            .py(px(4.0))
            .flex()
            .flex_col()
            .children(rows)
            .child(
                div().pl(px(34.0)).py(px(5.0)).child(
                    div()
                        .id(SharedString::from(open_key.clone()))
                        .debug_selector(move || open_key.clone())
                        .role(gpui::Role::Button)
                        .aria_label("Open Mission Control")
                        .cursor_pointer()
                        .text_size(ui_rems(12.0))
                        .text_color(theme.text_muted)
                        .hover(|s| s.text_color(theme.text))
                        .child("Open Mission Control ›")
                        .on_click(cx.listener(move |_, _: &ClickEvent, _, cx| {
                            cx.emit(SupervisorEvent::OpenTaskCenter(open_id.clone()))
                        })),
                ),
            )
            .into_any_element()
    }

    fn render_mission(
        &self,
        mission: Mission,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let row = mission.row;
        let tone = |indicator| crate::shell::chat_status_tone(indicator, theme).2;
        let (arc, text) = match row.status {
            TaskStatus::NeedsYou => {
                let needs = tone(ChatIndicator::AwaitingInput);
                (needs, needs)
            }
            TaskStatus::Working => (tone(ChatIndicator::Completed), tone(ChatIndicator::Working)),
            _ => (tone(ChatIndicator::Completed), theme.text_faint),
        };
        let PlanProgress { done, total } = row.progress;
        let percent = (done * 100).checked_div(total).unwrap_or(0).min(100) as u8;
        let expanded = self.expanded.contains(&row.task_id);
        let task_id = row.task_id.clone();
        let watch = match &mission.watch {
            Watch::Elsewhere(device) => {
                let name = self
                    .state
                    .read(cx)
                    .device_name(device)
                    .unwrap_or("another device")
                    .to_owned();
                div()
                    .flex_none()
                    .text_size(ui_rems(11.5))
                    .text_color(theme.text_faint)
                    .child(format!("on {name}, not watched"))
                    .into_any_element()
            }
            watch => {
                let on = *watch == Watch::Watched;
                let id = task_id.clone();
                div()
                    .flex_none()
                    .flex()
                    .items_center()
                    .gap(px(6.0))
                    .text_size(ui_rems(11.5))
                    .text_color(theme.text_muted)
                    .child(if on { "Watched" } else { "Not watched" })
                    .child(
                        Self::render_switch(
                            theme,
                            format!("supervisor-watch-{task_id}"),
                            format!("Watch {}", row.title).into(),
                            on,
                        )
                        .on_click(cx.listener(
                            move |page, _: &ClickEvent, _, cx| {
                                cx.stop_propagation();
                                page.set_supervised(id.clone(), !on, cx);
                            },
                        )),
                    )
                    .into_any_element()
            }
        };
        let header_key = format!("supervisor-mission-{task_id}");
        let toggle_id = task_id.clone();
        div()
            .rounded(px(12.0))
            .bg(widgets::block_fill(theme))
            .overflow_hidden()
            .flex()
            .flex_col()
            .child(
                div()
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
                    .hover(|s| s.bg(theme.wash(0.04)))
                    .child(div().flex_none().child(crate::loaders::upload_progress_arc(
                        percent,
                        14.0,
                        theme.text_faint.opacity(0.6),
                        arc,
                    )))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_size(ui_rems(13.0))
                            .text_color(theme.text)
                            .child(row.title.clone()),
                    )
                    .child(
                        div()
                            .flex_none()
                            .text_size(ui_rems(12.0))
                            .text_color(text)
                            .child(row.label.clone()),
                    )
                    .child(
                        div()
                            .flex_none()
                            .text_size(ui_rems(12.0))
                            .text_color(theme.text_muted)
                            .child(format!("{done}/{total}")),
                    )
                    .child(watch)
                    .child(
                        icon(if expanded {
                            icons::LUCIDE_CHEVRON_DOWN
                        } else {
                            icons::LUCIDE_CHEVRON_RIGHT
                        })
                        .size(px(13.0))
                        .text_color(theme.text_muted),
                    )
                    .on_click(cx.listener(move |page, _: &ClickEvent, _, cx| {
                        if !page.expanded.remove(&toggle_id) {
                            page.expanded.insert(toggle_id.clone());
                        }
                        cx.notify();
                    })),
            )
            .when(expanded, |el| {
                el.child(self.render_owners(&row.task_id, theme, cx))
            })
            .into_any_element()
    }

    fn render_missions(
        &self,
        missions: Vec<Mission>,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let count = missions.len();
        let body: Vec<AnyElement> = if missions.is_empty() {
            vec![
                div()
                    .px(px(8.0))
                    .text_size(ui_rems(12.5))
                    .text_color(theme.text_muted)
                    .child("No missions running. Start one from Tasks or the composer.")
                    .into_any_element(),
            ]
        } else {
            missions
                .into_iter()
                .map(|mission| self.render_mission(mission, theme, cx))
                .collect()
        };
        div()
            .id("supervisor-missions")
            .debug_selector(|| "supervisor-missions".into())
            .flex()
            .flex_col()
            .gap(px(8.0))
            .child(widgets::section_label(theme, format!("Missions · {count}")))
            .children(body)
            .into_any_element()
    }

    fn render_authority(&self, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let label = widgets::section_label(theme, "Authority");
        let settings = match &self.settings {
            Loadable::Ready(settings) => *settings,
            Loadable::Error(error) => {
                return div()
                    .id("supervisor-authority")
                    .debug_selector(|| "supervisor-authority".into())
                    .flex()
                    .flex_col()
                    .child(label)
                    .child(widgets::error_strip(
                        theme,
                        format!("Could not read the Supervisor's authority: {error}"),
                    ))
                    .into_any_element();
            }
            _ => {
                return div()
                    .id("supervisor-authority")
                    .flex()
                    .flex_col()
                    .gap(px(8.0))
                    .child(label)
                    .child(
                        div()
                            .px(px(8.0))
                            .text_size(ui_rems(12.5))
                            .text_color(theme.text_muted)
                            .child("Loading…"),
                    )
                    .into_any_element();
            }
        };
        let row = |first: bool| {
            div()
                .mx(px(14.0))
                .py(px(10.0))
                .when(!first, |el| {
                    el.border_t_1().border_color(widgets::row_divider(theme))
                })
                .flex()
                .items_center()
                .justify_between()
                .gap(px(12.0))
                .text_size(ui_rems(12.5))
                .text_color(theme.text)
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
        let mut card = div()
            .rounded(px(12.0))
            .bg(widgets::block_fill(theme))
            .flex()
            .flex_col();
        for (ix, (key, text, on, set)) in toggles.into_iter().enumerate() {
            card = card.child(row(ix == 0).child(text).child(
                Self::render_switch(theme, key.into(), text.into(), on).on_click(cx.listener(
                    move |page, _: &ClickEvent, _, cx| page.save_settings(set(settings, !on), cx),
                )),
            ));
        }
        let weak = cx.entity().downgrade();
        card = card
            .child(
                row(false)
                    .debug_selector(|| "supervisor-auth-goal".into())
                    .child("Change a mission's goal or spend")
                    .child(
                        div()
                            .flex_none()
                            .flex()
                            .items_center()
                            .gap(px(5.0))
                            .text_size(ui_rems(11.5))
                            .text_color(theme.text_muted)
                            .child(
                                icon(icons::LUCIDE_SHIELD)
                                    .size(px(11.0))
                                    .text_color(theme.text_faint),
                            )
                            .child("always asks you"),
                    ),
            )
            .child(
                row(false).child("Check in on its own").child(
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
            );
        div()
            .id("supervisor-authority")
            .debug_selector(|| "supervisor-authority".into())
            .flex()
            .flex_col()
            .gap(px(8.0))
            .child(label)
            .child(card)
            .children(
                self.settings_error
                    .clone()
                    .map(|error| widgets::error_strip(theme, error)),
            )
            .into_any_element()
    }

    fn render_reply(&self, key: &str, buttons: gpui::Div, theme: &Theme) -> AnyElement {
        let note = |text: String| {
            div()
                .text_size(ui_rems(12.0))
                .text_color(theme.text_muted)
                .child(text)
                .into_any_element()
        };
        match self.replies.get(key) {
            Some(Reply::Sent(choice)) if choice.is_empty() => note("Answered".into()),
            Some(Reply::Sent(choice)) => note(format!("Answered: {choice}")),
            Some(Reply::Sending) => note("Sending…".into()),
            Some(Reply::Failed(error)) => div()
                .flex()
                .flex_col()
                .child(buttons)
                .child(widgets::error_strip(
                    theme,
                    format!("Could not send the answer: {error}"),
                ))
                .into_any_element(),
            None => buttons.into_any_element(),
        }
    }

    fn render_item_card(
        &self,
        item: &SupervisorItem,
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
        needs_card(theme, sel)
            .child(
                div()
                    .text_size(ui_rems(13.5))
                    .font_weight(gpui::FontWeight::MEDIUM)
                    .text_color(theme.text)
                    .child(item.title.clone()),
            )
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap(px(6.0))
                    .text_size(ui_rems(12.0))
                    .text_color(theme.text_muted)
                    .child(meta)
                    .child(tone_chip(
                        theme,
                        why_label(item.why),
                        Some(needs_tone(theme)),
                    )),
            )
            .when(!item.detail.trim().is_empty(), |el| {
                el.child(
                    div()
                        .mt(px(2.0))
                        .pl(px(10.0))
                        .border_l_2()
                        .border_color(theme.border)
                        .text_size(ui_rems(12.5))
                        .line_height(ui_rems(18.0))
                        .text_color(theme.text_muted)
                        .child(clip(item.detail.trim())),
                )
            })
            .child(
                div()
                    .mt(px(4.0))
                    .child(self.render_reply(&key, buttons, theme)),
            )
            .into_any_element()
    }

    fn render_input_card(
        &self,
        input: &NeedsYouInput,
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
                        |el| {
                            el.child(
                                div()
                                    .pl(px(10.0))
                                    .border_l_2()
                                    .border_color(theme.border)
                                    .text_size(ui_rems(12.5))
                                    .line_height(ui_rems(18.0))
                                    .text_color(theme.text_muted)
                                    .child(clip(question.question.trim())),
                            )
                        },
                    )
                    .when(
                        self.replies
                            .get(&key)
                            .is_none_or(|reply| matches!(reply, Reply::Failed(_))),
                        |el| el.child(options),
                    ),
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
        needs_card(theme, sel)
            .child(
                div()
                    .text_size(ui_rems(13.5))
                    .font_weight(gpui::FontWeight::MEDIUM)
                    .text_color(theme.text)
                    .child(row_title(&row)),
            )
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap(px(6.0))
                    .text_size(ui_rems(12.0))
                    .text_color(theme.text_muted)
                    .child(format!("Asked by {who}"))
                    .child(tone_chip(
                        theme,
                        if approval { "Approval" } else { "Question" },
                        Some(needs_tone(theme)),
                    )),
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
            )
            .into_any_element()
    }

    fn render_needs_you(
        &self,
        rows: &[NeedsYouRow],
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let body: Vec<AnyElement> = if rows.is_empty() {
            vec![empty_line(theme, "Nothing waits on you.")]
        } else {
            rows.iter()
                .map(|row| match row {
                    NeedsYouRow::Item(item) => self.render_item_card(item, theme, cx),
                    NeedsYouRow::Input(input) => self.render_input_card(input, theme, cx),
                })
                .collect()
        };
        div()
            .id("supervisor-needs-you")
            .debug_selector(|| "supervisor-needs-you".into())
            .flex()
            .flex_col()
            .gap(px(8.0))
            .child(widgets::section_label(
                theme,
                format!("Needs you · {}", rows.len()),
            ))
            .children(body)
            .into_any_element()
    }

    fn render_questions(
        &self,
        questions: &[LeadQuestion],
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let now = Utc::now();
        let body = if questions.is_empty() {
            empty_line(theme, "No step has asked its lead anything yet.")
        } else {
            let state = self.state.read(cx);
            let rows: Vec<(LeadQuestion, String)> = questions
                .iter()
                .map(|question| {
                    let step = step_name(state, &question.task_id, &question.step_id, now)
                        .unwrap_or_else(|| "A step".into());
                    let place = match mission_title(state, Some(&question.task_id)) {
                        Some(mission) => format!("{step} · {mission}"),
                        None => step,
                    };
                    (question.clone(), place)
                })
                .collect();
            div()
                .rounded(px(12.0))
                .bg(widgets::block_fill(theme))
                .flex()
                .flex_col()
                .children(rows.into_iter().enumerate().map(|(ix, (question, place))| {
                    let (label, detail) = question_state(&question.state);
                    let tone = match question.state {
                        QuestionState::Open => Some(needs_tone(theme)),
                        QuestionState::Settled { .. } => Some(ok_tone(theme)),
                        QuestionState::Dropped => None,
                    };
                    let sel = format!(
                        "supervisor-question-{}",
                        question.request_id.replace(':', "-")
                    );
                    let text = question_text(&question.questions);
                    let chat_id = question.chat_id.clone();
                    div()
                        .id(SharedString::from(sel.clone()))
                        .debug_selector(move || sel.clone())
                        .role(gpui::Role::Button)
                        .aria_label(format!("{label}: {text}"))
                        .cursor_pointer()
                        .hover(|s| s.bg(theme.wash(0.04)))
                        .mx(px(6.0))
                        .px(px(8.0))
                        .py(px(9.0))
                        .when(ix > 0, |el| {
                            el.border_t_1().border_color(widgets::row_divider(theme))
                        })
                        .flex()
                        .items_start()
                        .gap(px(10.0))
                        .child(div().mt(px(1.0)).child(tone_chip(theme, label, tone)))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .flex()
                                .flex_col()
                                .gap(px(3.0))
                                .child(
                                    div()
                                        .text_size(ui_rems(12.5))
                                        .line_height(ui_rems(18.0))
                                        .text_color(theme.text)
                                        .child(clip(&text)),
                                )
                                .child(
                                    div()
                                        .text_size(ui_rems(11.5))
                                        .text_color(theme.text_muted)
                                        .child(match detail {
                                            Some(detail) => format!("{place} · {detail}"),
                                            None => place,
                                        }),
                                ),
                        )
                        .on_click(cx.listener(move |_, _: &ClickEvent, _, cx| {
                            cx.emit(SupervisorEvent::OpenChat(chat_id.clone()))
                        }))
                }))
                .into_any_element()
        };
        div()
            .id("supervisor-questions")
            .debug_selector(|| "supervisor-questions".into())
            .flex()
            .flex_col()
            .gap(px(8.0))
            .child(widgets::section_label(
                theme,
                format!("Open questions to the Lead · {}", questions.len()),
            ))
            .child(body)
            .into_any_element()
    }

    fn render_interventions(
        &self,
        rows: &[SupervisorIntervention],
        wakes: &[SupervisorWake],
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let now = Utc::now();
        let body = if rows.is_empty() {
            empty_line(theme, "The Supervisor has not acted yet.")
        } else {
            let state = self.state.read(cx);
            div()
                .rounded(px(12.0))
                .bg(widgets::block_fill(theme))
                .flex()
                .flex_col()
                .children(rows.iter().enumerate().map(|(ix, row)| {
                    let target = row
                        .target
                        .as_deref()
                        .map(|target| thread_name(state, target, now))
                        .or_else(|| mission_title(state, row.task_id.as_deref()));
                    let mut chips: Vec<gpui::Div> = row
                        .chips
                        .iter()
                        .map(|chip| {
                            let tone = (*chip != InterventionChip::Sent).then(|| ok_tone(theme));
                            tone_chip(theme, chip_label(chip), tone)
                        })
                        .collect();
                    if no_effect_yet(row, now) {
                        chips.push(tone_chip(theme, "No effect yet", Some(needs_tone(theme))));
                    }
                    let id = row.id.clone();
                    div()
                        .debug_selector(move || format!("supervisor-intervention-{id}"))
                        .mx(px(14.0))
                        .py(px(10.0))
                        .when(ix > 0, |el| {
                            el.border_t_1().border_color(widgets::row_divider(theme))
                        })
                        .flex()
                        .flex_col()
                        .gap(px(5.0))
                        .child(
                            div()
                                .flex()
                                .flex_wrap()
                                .items_center()
                                .gap(px(8.0))
                                .text_size(ui_rems(12.5))
                                .child(
                                    div()
                                        .font_family(theme.font_mono.clone())
                                        .text_size(ui_rems(11.5))
                                        .text_color(theme.text_muted)
                                        .child(clock(row.at)),
                                )
                                .child(
                                    div()
                                        .text_color(theme.text)
                                        .child(intervention_kind_label(row.kind)),
                                )
                                .children(target.map(|target| {
                                    div()
                                        .text_color(theme.text_muted)
                                        .child(format!("→ {target}"))
                                })),
                        )
                        .when(!row.text.trim().is_empty(), |el| {
                            el.child(
                                div()
                                    .pl(px(10.0))
                                    .border_l_2()
                                    .border_color(theme.border)
                                    .text_size(ui_rems(12.5))
                                    .line_height(ui_rems(18.0))
                                    .text_color(theme.text)
                                    .child(clip(row.text.trim())),
                            )
                        })
                        .children(row.trigger.as_ref().map(|trigger| {
                            div()
                                .text_size(ui_rems(11.5))
                                .text_color(theme.text_muted)
                                .child(trigger_label(trigger, wakes))
                        }))
                        .when(!chips.is_empty(), |el| {
                            el.child(
                                div()
                                    .flex()
                                    .flex_wrap()
                                    .items_center()
                                    .gap(px(6.0))
                                    .children(chips),
                            )
                        })
                }))
                .into_any_element()
        };
        div()
            .id("supervisor-interventions")
            .debug_selector(|| "supervisor-interventions".into())
            .flex()
            .flex_col()
            .gap(px(8.0))
            .child(widgets::section_label(
                theme,
                format!("Interventions · {}", rows.len()),
            ))
            .child(body)
            .into_any_element()
    }

    fn render_resources(&self, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let state = self.state.read(cx);
        let resources: Vec<SharedResource> = state
            .supervisor
            .as_ref()
            .map(|snapshot| snapshot.resources.clone())
            .unwrap_or_default();
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
        let body = if resources.is_empty() {
            div()
                .px(px(8.0))
                .text_size(ui_rems(12.5))
                .text_color(theme.text_muted)
                .child("No mission declares a port, CPU slot or path. A lead declares them with its plan.")
                .into_any_element()
        } else {
            div()
                .rounded(px(12.0))
                .bg(widgets::block_fill(theme))
                .flex()
                .flex_col()
                .children(resources.iter().enumerate().map(|(ix, resource)| {
                    let tone = match resource.state {
                        ResourceState::Contended => theme.danger,
                        ResourceState::Clear => {
                            crate::shell::chat_status_tone(ChatIndicator::Completed, theme).2
                        }
                        _ => theme.text_muted,
                    };
                    let held = match resource.holders.as_slice() {
                        [] => "held by none".to_string(),
                        holders => format!("held by {}", names(holders)),
                    };
                    let key = resource.key.clone();
                    div()
                        .debug_selector(move || format!("supervisor-resource-{key}"))
                        .mx(px(14.0))
                        .py(px(9.0))
                        .when(ix > 0, |el| {
                            el.border_t_1().border_color(widgets::row_divider(theme))
                        })
                        .flex()
                        .flex_col()
                        .gap(px(3.0))
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap(px(8.0))
                                .text_size(ui_rems(12.5))
                                .child(
                                    div()
                                        .font_family(theme.font_mono.clone())
                                        .text_color(theme.text)
                                        .child(resource.key.clone()),
                                )
                                .child(
                                    div().text_color(tone).child(resource_label(resource.state)),
                                ),
                        )
                        .child(
                            div()
                                .text_size(ui_rems(12.0))
                                .text_color(theme.text_muted)
                                .child(format!(
                                    "Declared by {} · {held}",
                                    names(&resource.declared_by)
                                )),
                        )
                        .children(verified_line(resource, &title).map(|line| {
                            div()
                                .text_size(ui_rems(12.0))
                                .text_color(theme.text_muted)
                                .child(line)
                        }))
                }))
                .into_any_element()
        };
        div()
            .id("supervisor-resources")
            .debug_selector(|| "supervisor-resources".into())
            .flex()
            .flex_col()
            .gap(px(8.0))
            .child(widgets::section_label(
                theme,
                format!("Shared resources · {}", resources.len()),
            ))
            .child(body)
            .into_any_element()
    }

    fn render_wake_log(&self, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let wakes: Vec<SupervisorWake> = self
            .state
            .read(cx)
            .supervisor
            .as_ref()
            .map(|snapshot| snapshot.wakes.clone())
            .unwrap_or_default();
        let open = self.wakes_open;
        let header = div()
            .id("supervisor-wake-log-toggle")
            .debug_selector(|| "supervisor-wake-log-toggle".into())
            .role(gpui::Role::Button)
            .aria_label("Wake log")
            .aria_expanded(open)
            .flex()
            .items_center()
            .gap(px(4.0))
            .cursor_pointer()
            .child(widgets::section_label(
                theme,
                format!("Wake log · {}", wakes.len()),
            ))
            .child(
                icon(if open {
                    icons::LUCIDE_CHEVRON_DOWN
                } else {
                    icons::LUCIDE_CHEVRON_RIGHT
                })
                .size(px(12.0))
                .text_color(theme.text_muted),
            )
            .on_click(cx.listener(|page, _: &ClickEvent, _, cx| {
                page.wakes_open = !page.wakes_open;
                cx.notify();
            }));
        let body = open.then(|| {
            if wakes.is_empty() {
                return div()
                    .px(px(8.0))
                    .text_size(ui_rems(12.5))
                    .text_color(theme.text_muted)
                    .child("No wakes yet. Events from watched missions wake it here.")
                    .into_any_element();
            }
            div()
                .rounded(px(12.0))
                .bg(widgets::block_fill(theme))
                .flex()
                .flex_col()
                .children(wakes.iter().enumerate().map(|(ix, wake)| {
                    let failed = matches!(
                        wake.delivery,
                        WakeDelivery::Failed { .. } | WakeDelivery::Dropped
                    );
                    let events = wake
                        .events
                        .iter()
                        .map(|event| (event.label.clone(), false))
                        .chain(
                            wake.dropped
                                .iter()
                                .map(|event| (format!("Dropped: {}", event.label), true)),
                        );
                    div()
                        .debug_selector({
                            let id = wake.id.clone();
                            move || format!("supervisor-wake-{id}")
                        })
                        .mx(px(14.0))
                        .py(px(9.0))
                        .when(ix > 0, |el| {
                            el.border_t_1().border_color(widgets::row_divider(theme))
                        })
                        .flex()
                        .flex_col()
                        .gap(px(3.0))
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap(px(8.0))
                                .text_size(ui_rems(12.5))
                                .child(
                                    div()
                                        .font_family(theme.font_mono.clone())
                                        .text_size(ui_rems(11.5))
                                        .text_color(theme.text_muted)
                                        .child(clock(wake.at)),
                                )
                                .child(div().text_color(theme.text).child(format!(
                                    "{} {}",
                                    wake.events.len(),
                                    plural(wake.events.len(), "event", "events")
                                )))
                                .child(
                                    div()
                                        .text_color(if failed {
                                            theme.danger
                                        } else {
                                            theme.text_muted
                                        })
                                        .child(delivery_label(&wake.delivery)),
                                ),
                        )
                        .children(events.map(|(label, dropped)| {
                            div()
                                .pl(px(10.0))
                                .border_l_2()
                                .border_color(theme.border)
                                .text_size(ui_rems(12.0))
                                .text_color(if dropped {
                                    theme.text_faint
                                } else {
                                    theme.text_muted
                                })
                                .child(label)
                        }))
                }))
                .into_any_element()
        });
        div()
            .id("supervisor-wake-log")
            .debug_selector(|| "supervisor-wake-log".into())
            .flex()
            .flex_col()
            .gap(px(8.0))
            .child(header)
            .children(body)
            .into_any_element()
    }

    fn render_pane_status(&self, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let body = div()
            .p(px(16.0))
            .flex()
            .flex_col()
            .gap(px(10.0))
            .text_size(ui_rems(12.5))
            .text_color(theme.text_muted);
        match &self.ensure {
            Ensure::Failed(error) => body
                .child(format!("Could not start the Supervisor: {error}"))
                .child(
                    widgets::text_action(theme, widgets::ActionTone::Filled, "Try again")
                        .id("supervisor-retry")
                        .debug_selector(|| "supervisor-retry".into())
                        .role(gpui::Role::Button)
                        .aria_label("Try again")
                        .self_start()
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

    fn render_pane(&self, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let body = match &self.pane {
            Some(pane) => div()
                .flex_1()
                .min_h_0()
                .flex()
                .flex_col()
                .child(div().flex_1().min_h_0().child(pane.transcript.clone()))
                .child(div().flex_none().child(pane.composer.clone()))
                .into_any_element(),
            None => self.render_pane_status(theme, cx),
        };
        div()
            .id("supervisor-pane")
            .debug_selector(|| "supervisor-pane".into())
            .flex_none()
            .w(relative(0.42))
            .min_w(px(340.0))
            .max_w(px(620.0))
            .h_full()
            .pt(px(Theme::TITLEBAR_HEIGHT))
            .flex()
            .flex_col()
            .border_l_1()
            .border_color(theme.border)
            .child(body)
            .into_any_element()
    }

    fn render_drawer(
        &self,
        owner: &Owner,
        theme: &Theme,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let now = Utc::now();
        let found = {
            let state = self.state.read(cx);
            state
                .tasks
                .iter()
                .find(|task| task.id == owner.task_id && !task.deleted)
                .map(|task| (task.clone(), rollup(state, task, now)))
        };
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
            .right_0()
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
    crate::shell::chat_status_tone(ChatIndicator::AwaitingInput, theme).2
}

fn ok_tone(theme: &Theme) -> Hsla {
    crate::shell::chat_status_tone(ChatIndicator::Completed, theme).2
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

fn needs_card(theme: &Theme, selector: String) -> gpui::Div {
    let tone = needs_tone(theme);
    div()
        .debug_selector(move || selector.clone())
        .rounded(px(12.0))
        .border_1()
        .border_color(tone.opacity(0.45))
        .bg(tone.opacity(0.06))
        .px(px(14.0))
        .py(px(12.0))
        .flex()
        .flex_col()
        .gap(px(6.0))
}

fn empty_line(theme: &Theme, text: &'static str) -> AnyElement {
    div()
        .px(px(8.0))
        .text_size(ui_rems(12.5))
        .text_color(theme.text_muted)
        .child(text)
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
        let missions = missions(self.state.read(cx), Utc::now());
        let watches: Vec<Watch> = missions.iter().map(|m| m.watch.clone()).collect();
        let snapshot = self.state.read(cx).supervisor.clone().unwrap_or_default();
        let column = widgets::page_column()
            .max_w(px(820.0))
            .gap(px(26.0))
            .child(self.render_header(watched_count(&watches), &theme, cx))
            .child(self.render_needs_you(&snapshot.needs_you, &theme, cx))
            .child(self.render_missions(missions, &theme, cx))
            .child(self.render_questions(&snapshot.questions, &theme, cx))
            .child(self.render_interventions(&snapshot.interventions, &snapshot.wakes, &theme, cx))
            .when(!cfg!(windows), |el| {
                el.child(self.render_resources(&theme, cx))
            })
            .child(self.render_authority(&theme, cx))
            .child(self.render_wake_log(&theme, cx));
        let scroll = widgets::scroll_faded(
            "supervisor-page-scroll",
            div()
                .id("supervisor-page-scroll")
                .size_full()
                .overflow_y_scroll()
                .child(column),
        );
        let docked = (!self.pane_open)
            .then(|| self.pane.as_ref().map(|pane| pane.composer.clone()))
            .flatten();
        let left = div()
            .flex_1()
            .min_w(px(360.0))
            .h_full()
            .flex()
            .flex_col()
            .child(div().flex_1().min_h_0().child(scroll))
            .children(docked.map(|composer| {
                div()
                    .flex_none()
                    .w_full()
                    .max_w(px(820.0))
                    .mx_auto()
                    .px(px(24.0))
                    .child(composer)
            }));
        let drawer = self
            .drawer
            .clone()
            .map(|owner| self.render_drawer(&owner, &theme, window, cx));
        div()
            .id("supervisor-page")
            .debug_selector(|| "supervisor-page".into())
            .relative()
            .size_full()
            .flex()
            .flex_row()
            .child(left)
            .when(self.pane_open, |el| el.child(self.render_pane(&theme, cx)))
            .children(drawer)
    }
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
