use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::{UserInputAnswer, UserInputQuestion};

pub const CHAT_PREFIX: &str = "supervisor-";
pub const CHAT_TITLE: &str = "Supervisor";

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
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SupervisorSnapshot {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chat_id: Option<String>,
    pub state: SupervisorState,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub waiting: usize,
    #[serde(default)]
    pub wakes: Vec<SupervisorWake>,
    #[serde(default)]
    pub unwatched: Vec<String>,
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
}

impl Default for SupervisorSettings {
    fn default() -> Self {
        Self {
            message_threads: true,
            rebrief: true,
            pause_steps: true,
            start_missions: false,
            heartbeat_minutes: 0,
        }
    }
}
