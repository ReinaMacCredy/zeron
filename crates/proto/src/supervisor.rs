use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

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
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SupervisorSnapshot {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chat_id: Option<String>,
    pub state: SupervisorState,
    #[serde(default)]
    pub wakes: Vec<SupervisorWake>,
    #[serde(default)]
    pub unwatched: Vec<String>,
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
