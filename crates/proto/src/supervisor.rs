use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::{UserInputAnswer, UserInputQuestion};

pub const CHAT_PREFIX: &str = "supervisor-";
pub const CHAT_TITLE: &str = "Supervisor";
pub const WAKES_PER_HOUR: usize = 12;

pub fn is_supervisor_chat(chat_id: &str) -> bool {
    chat_id.starts_with(CHAT_PREFIX)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SupervisorEventKind {
    StepFailed,
    NeedsYou,
    MissionDone,
    MissionSilent,
    LeadSilent,
    Escalation,
    PlanEdited,
    Resource,
    Heartbeat,
    NodeFailed,
    NodeFinished,
    QuestionWaiting,
    Dirty,
    Unmerged,
    LimitIdle,
    LostWake,
    MergeBlocked,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SupervisorEventSummary {
    pub kind: SupervisorEventKind,
    pub key: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub step_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub root_chat_id: Option<String>,
    pub label: String,
    pub at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum WakeDelivery {
    Started,
    Steered,
    HeldForTurnEnd,
    HeldByStop,
    Failed { error: String },
    Dropped,
}

/// Wakes are queued with no sender, like the person's prompts: tell them apart by this id, the transcript entry's.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SupervisorWake {
    pub id: String,
    pub at: DateTime<Utc>,
    #[serde(default)]
    pub events: Vec<SupervisorEventSummary>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub dropped: Vec<SupervisorEventSummary>,
    pub delivery: WakeDelivery,
}

impl SupervisorEventKind {
    pub fn word(self) -> &'static str {
        match self {
            Self::StepFailed => "step failed",
            Self::NeedsYou => "needs you",
            Self::MissionDone => "mission done",
            Self::MissionSilent => "mission silent",
            Self::LeadSilent => "lead silent",
            Self::Escalation => "lead escalated",
            Self::PlanEdited => "plan edited",
            Self::Resource => "resource contention",
            Self::Heartbeat => "check-in",
            Self::NodeFailed => "thread failed",
            Self::NodeFinished => "delegated task ended",
            Self::QuestionWaiting => "question waiting",
            Self::Dirty => "uncommitted files",
            Self::Unmerged => "unmerged branch",
            Self::LimitIdle => "idle after limit reset",
            Self::LostWake => "lost wake",
            Self::MergeBlocked => "merge blocked",
        }
    }
}

impl WakeDelivery {
    pub fn label(&self) -> String {
        match self {
            Self::Started => "Started a turn".into(),
            Self::Steered => "Steered its turn".into(),
            Self::HeldForTurnEnd => "Held for turn end".into(),
            Self::HeldByStop => "Held by your Stop".into(),
            Self::Failed { error } => format!("Failed: {error}"),
            Self::Dropped => "Dropped".into(),
        }
    }
}

pub fn wakes_this_hour(wakes: &[SupervisorWake], now: DateTime<Utc>) -> usize {
    wakes
        .iter()
        .filter(|wake| wake.woke() && now - wake.at < chrono::Duration::hours(1))
        .count()
}

impl SupervisorWake {
    pub fn woke(&self) -> bool {
        !matches!(
            self.delivery,
            WakeDelivery::Failed { .. } | WakeDelivery::Dropped
        )
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SupervisorState {
    #[default]
    Off,
    Asleep,
    Awake,
    Stopped,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ItemWhy {
    Goal,
    Spend,
    Decision,
}

impl ItemWhy {
    pub fn label(self) -> &'static str {
        match self {
            Self::Goal => "Goal",
            Self::Spend => "Spend",
            Self::Decision => "Decision",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Raiser {
    Lead,
    Supervisor,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ItemAnswer {
    pub at: DateTime<Utc>,
    pub choice: String,
    /// Set when the Supervisor decided; a `choice` that differs is the person's correction.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub supervisor: Option<SupervisorChoice>,
}

impl ItemAnswer {
    pub fn corrected(&self) -> bool {
        self.supervisor
            .as_ref()
            .is_some_and(|decided| decided.choice != self.choice)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SupervisorChoice {
    pub at: DateTime<Utc>,
    pub choice: String,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SupervisorItem {
    pub id: String,
    pub at: DateTime<Utc>,
    pub raised_by: Raiser,
    pub raiser_chat_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_id: Option<String>,
    pub title: String,
    pub why: ItemWhy,
    #[serde(default)]
    pub detail: String,
    #[serde(default)]
    pub options: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub answer: Option<ItemAnswer>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum InputAsker {
    Lead,
    Step,
    Supervisor,
    Thread,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NeedsYouInput {
    pub chat_id: String,
    pub request_id: String,
    pub asker: InputAsker,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub step_id: Option<String>,
    pub at: DateTime<Utc>,
    pub questions: Vec<UserInputQuestion>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum NeedsYouRow {
    Item(SupervisorItem),
    Input(NeedsYouInput),
}

const TITLE_CHARS: usize = 120;

impl NeedsYouInput {
    pub fn key(&self) -> String {
        format!("input:{}:{}", self.chat_id, self.request_id)
    }

    pub fn title(&self) -> String {
        let Some(first) = self.questions.first() else {
            return "A question".into();
        };
        let text = if first.header.trim().is_empty() {
            &first.question
        } else {
            &first.header
        };
        clip(&crate::view::single_line(text), TITLE_CHARS)
    }
}

fn clip(text: &str, chars: usize) -> String {
    match text.char_indices().nth(chars) {
        Some((at, _)) => format!("{}…", &text[..at]),
        None => text.to_owned(),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SendDest {
    Lead,
    Step,
    Supervisor,
    Thread,
}

impl SendDest {
    pub fn word(self) -> &'static str {
        match self {
            Self::Lead => "the Lead",
            Self::Step => "the step",
            Self::Thread => "the thread",
            Self::Supervisor => "the Supervisor",
        }
    }
}

impl NeedsYouRow {
    pub fn dest(&self) -> SendDest {
        match self {
            Self::Item(item) if item.task_id.is_some() => SendDest::Lead,
            Self::Item(_) => SendDest::Supervisor,
            Self::Input(input) => match input.asker {
                InputAsker::Lead => SendDest::Lead,
                InputAsker::Step => SendDest::Step,
                InputAsker::Thread => SendDest::Thread,
                InputAsker::Supervisor => SendDest::Supervisor,
            },
        }
    }

    pub fn title(&self) -> String {
        match self {
            Self::Item(item) => clip(&item.title, TITLE_CHARS),
            Self::Input(input) => input.title(),
        }
    }

    pub fn key(&self) -> String {
        match self {
            Self::Item(item) => format!("item:{}", item.id),
            Self::Input(input) => input.key(),
        }
    }

    pub fn origin(&self) -> (Option<&str>, &str) {
        match self {
            Self::Item(item) => (item.task_id.as_deref(), item.raiser_chat_id.as_str()),
            Self::Input(input) => (input.task_id.as_deref(), input.chat_id.as_str()),
        }
    }

    pub fn task_id(&self) -> Option<&str> {
        self.origin().0
    }

    pub fn at(&self) -> DateTime<Utc> {
        match self {
            Self::Item(item) => item.at,
            Self::Input(input) => input.at,
        }
    }

    /// A watched plain tree has no task, so its threads' asks group under the tree's root chat.
    pub fn group(&self, root_of: &dyn Fn(&str) -> Option<String>) -> Option<String> {
        match self {
            Self::Input(input)
                if input.task_id.is_none() && input.asker != InputAsker::Supervisor =>
            {
                root_of(&input.chat_id)
            }
            row => row.task_id().map(str::to_owned),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SettledBy {
    Lead,
    Supervisor,
    Outside,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum QuestionState {
    Open,
    Settled {
        by: SettledBy,
        answers: Vec<UserInputAnswer>,
    },
    Dropped,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LeadQuestion {
    pub task_id: String,
    pub step_id: String,
    pub chat_id: String,
    pub request_id: String,
    pub at: DateTime<Utc>,
    pub questions: Vec<UserInputQuestion>,
    pub state: QuestionState,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum InterventionKind {
    Send,
    Respond,
    Rebrief,
    Start,
    Pause,
    Resume,
    Decide,
    Approve,
    Goal,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum InterventionTrigger {
    #[serde(rename_all = "camelCase")]
    Wake {
        wake_id: String,
    },
    Person,
}

/// A Supervisor plan approval reaches the lead at `starts_at`, so the person can take it back first.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "state",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum PlanApproval {
    Held { starts_at: DateTime<Utc> },
    Sent,
    TakenBack { at: DateTime<Utc> },
}

/// Step numbers are 1-based, in the plan's order.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum InterventionChip {
    Sent,
    LeadRanTurn,
    PlanEdited { steps: Vec<usize> },
    StepCompletedTurn { steps: Vec<usize> },
    StepPaused,
    ResourceFreed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SupervisorIntervention {
    pub id: String,
    pub at: DateTime<Utc>,
    pub kind: InterventionKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_id: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub step_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_id: Option<String>,
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trigger: Option<InterventionTrigger>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub chips: Vec<InterventionChip>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub item: Option<SupervisorItem>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub approval: Option<PlanApproval>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum NextCheckKind {
    MissionSilent,
    LeadSilent,
    Heartbeat,
    HeldWake,
    Resource,
    Tree,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NextCheck {
    pub at: DateTime<Utc>,
    pub kind: NextCheckKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_id: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SupervisorSnapshot {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chat_id: Option<String>,
    pub state: SupervisorState,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub waiting: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_check: Option<NextCheck>,
    /// Why the Supervisor's agent cannot run on this Mac; wakes wait while it is set.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blocked: Option<String>,
    #[serde(default)]
    pub wakes: Vec<SupervisorWake>,
    #[serde(default)]
    pub unwatched: Vec<String>,
    #[serde(default)]
    pub watched_roots: Vec<String>,
    #[serde(default)]
    pub watch_everything: bool,
    #[serde(default)]
    pub needs_you: Vec<NeedsYouRow>,
    #[serde(default)]
    pub questions: Vec<LeadQuestion>,
    #[serde(default)]
    pub interventions: Vec<SupervisorIntervention>,
    #[serde(default)]
    pub resources: Vec<SharedResource>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ResourceKey {
    Port(u16),
    Slot(String),
    Path(String),
}

impl ResourceKey {
    pub fn parse(key: &str) -> Option<Self> {
        let (kind, value) = key.trim().split_once(':')?;
        let value = value.trim();
        match kind.trim() {
            "port" => value.parse().ok().filter(|port| *port > 0).map(Self::Port),
            "slot" => (!value.is_empty()
                && value.len() <= 64
                && value
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.')))
            .then(|| Self::Slot(value.to_string())),
            "path" => {
                if value.starts_with('/') || value.starts_with('~') {
                    return None;
                }
                let parts: Vec<&str> = value
                    .split('/')
                    .filter(|part| !part.is_empty() && *part != ".")
                    .collect();
                (!parts.is_empty() && !parts.contains(&"..")).then(|| Self::Path(parts.join("/")))
            }
            _ => None,
        }
    }

    pub fn overlaps(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Path(a), Self::Path(b)) => {
                let inside = |inner: &str, outer: &str| {
                    inner
                        .strip_prefix(outer)
                        .is_some_and(|rest| rest.is_empty() || rest.starts_with('/'))
                };
                inside(a, b) || inside(b, a)
            }
            _ => self == other,
        }
    }
}

impl std::fmt::Display for ResourceKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Port(port) => write!(f, "port:{port}"),
            Self::Slot(name) => write!(f, "slot:{name}"),
            Self::Path(path) => write!(f, "path:{path}"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ResourceState {
    Free,
    Held,
    Checking,
    Clear,
    Contended,
    CheckFailed,
    NotVerified,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CheckTool {
    Lsof,
    Ps,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResourceUse {
    pub pid: u32,
    pub command: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cpu: Option<f32>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResourceCheck {
    pub at: DateTime<Utc>,
    pub tool: CheckTool,
    pub contended: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub uses: Vec<ResourceUse>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SharedResource {
    pub key: String,
    #[serde(default)]
    pub declared_by: Vec<String>,
    #[serde(default)]
    pub holders: Vec<String>,
    pub state: ResourceState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub check: Option<ResourceCheck>,
}

impl SupervisorSnapshot {
    pub fn needs_you_count(&self) -> usize {
        self.needs_you.len()
    }
}

fn is_zero(n: &usize) -> bool {
    *n == 0
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct SupervisorSettings {
    pub message_threads: bool,
    pub rebrief: bool,
    pub pause_steps: bool,
    pub start_missions: bool,
    pub heartbeat_minutes: u32,
    pub decide_for_me: bool,
    pub goal_and_spend: bool,
}

impl Default for SupervisorSettings {
    fn default() -> Self {
        Self {
            message_threads: true,
            rebrief: true,
            pause_steps: true,
            start_missions: false,
            heartbeat_minutes: 0,
            decide_for_me: false,
            goal_and_spend: false,
        }
    }
}

impl SupervisorSettings {
    pub fn decides(&self, why: ItemWhy) -> bool {
        match why {
            ItemWhy::Decision => self.decide_for_me,
            ItemWhy::Goal | ItemWhy::Spend => self.goal_and_spend,
        }
    }

    pub fn full_authority(&self) -> bool {
        self.decide_for_me || self.goal_and_spend
    }
}

/// Only the changed fields, so a concurrent change to another field survives.
pub fn settings_patch(
    before: Option<SupervisorSettings>,
    after: SupervisorSettings,
) -> serde_json::Value {
    let after = serde_json::to_value(after).unwrap_or_default();
    match (before.map(serde_json::to_value), after) {
        (Some(Ok(serde_json::Value::Object(before))), serde_json::Value::Object(after)) => {
            serde_json::Value::Object(
                after
                    .into_iter()
                    .filter(|(key, value)| before.get(key) != Some(value))
                    .collect(),
            )
        }
        (_, after) => after,
    }
}
