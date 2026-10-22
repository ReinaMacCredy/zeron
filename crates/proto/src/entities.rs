//! Synced entity rows (workspace doc) and local projections.
//!
//! In zeron these were synced Postgres rows; in zeron they live in the per-org
//! workspace Loro doc (see ARCHITECTURE.md §2.2) with the same field surface.

use std::collections::{BTreeMap, BTreeSet};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::{HarnessId, PermissionMode, ReasoningLevel, SandboxLevel};

/// Admission limit for new pins. Concurrent offline additions may exceed it;
/// existing pins remain visible, reorderable and removable without truncation.
pub const MAX_SIDEBAR_PINS: usize = 200;

/// Validate an optimistic projection without truncating concurrent overflow.
pub fn validate_sidebar_pin_update(
    current: &[String],
    next: &[String],
) -> Result<(), &'static str> {
    let mut seen = std::collections::HashSet::new();
    if next.iter().any(|id| id.is_empty() || !seen.insert(id)) {
        return Err("Sidebar pins must be non-empty and unique");
    }
    if next.len() > MAX_SIDEBAR_PINS && next.iter().any(|id| !current.contains(id)) {
        return Err("You can pin up to 200 sessions");
    }
    Ok(())
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SidebarPreferences {
    #[serde(default)]
    pub pinned_session_ids: Vec<String>,
    #[serde(default)]
    pub sections: Vec<SidebarSection>,
}

/// A user-named sidebar section. Archived sessions retain membership so restoring
/// them restores their section; deleting the section never deletes sessions.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct SidebarSection {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub session_ids: Vec<String>,
    #[serde(default)]
    pub collapsed: bool,
}

/// Watch payload for pins. `initialized` records known cached state, including
/// an empty list; `synced` records receipt of an authoritative registry state.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SidebarPreferencesState {
    /// Monotonic within one engine attachment, not a cross-device order key.
    /// Lets clients reject older watch frames after a mutation response.
    #[serde(default)]
    pub revision: u64,
    pub synced: bool,
    pub initialized: bool,
    #[serde(default)]
    pub pinned_session_ids: Vec<String>,
    #[serde(default)]
    pub sections: Vec<SidebarSection>,
}

impl SidebarPreferencesState {
    /// A cached initialized row remains editable offline. An unknown list does not.
    pub fn can_edit(&self) -> bool {
        self.synced || self.initialized
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Device {
    pub id: String,
    pub name: String,
    pub platform: String,
    pub last_seen_at: Option<DateTime<Utc>>,
    /// First registration time (zeron devices.created_at — the Devices page
    /// "Added …" fragment). Optional so pre-existing docs stay readable.
    #[serde(default)]
    pub created_at: Option<DateTime<Utc>>,
    /// App version the device's engine last booted with — fleet staleness at a
    /// glance (Devices page). Optional so pre-existing docs stay readable.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    /// Cursor SDK selected by the owning engine; absent on older engines.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cursor_sdk_version: Option<String>,
    /// Protocol/document features supported by the engine currently owning
    /// this device row. Missing on older builds.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub capabilities: Vec<String>,
}

impl Device {
    pub fn supports(&self, capability: &str) -> bool {
        self.capabilities.iter().any(|value| value == capability)
    }
}

/// A synced (device, folder) pair — the unit of organization in the sidebar.
/// Sessions belong to exactly one space; the space fixes their host device and
/// base cwd. Folders need not be git repos: `git_detected` is stamped by the
/// owning device (SpacesSync) and gates branch pickers / the diff sidebar on
/// every device without an RPC.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Space {
    pub id: String,
    /// Owning device — fixed at create, immutable.
    pub device_id: String,
    /// Absolute folder path on the owning device.
    pub path: String,
    /// User rename; absent ⇒ display = basename(path).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Owner-stamped: is `path` inside a git work tree?
    #[serde(default)]
    pub git_detected: bool,
    /// Owner-stamped freshness timestamp of the last git check.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub git_checked_at: Option<DateTime<Utc>>,
    /// Owner-stamped when git: canonical checkout identity of the space root
    /// (sha256(deviceId ‖ NUL ‖ git_dir)) — diff grouping key for root sessions.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checkout_id: Option<String>,
    /// Owner-stamped when git: identity shared by every clone and worktree of
    /// one repository — its trunk's root commit (`commit:<sha>`, which
    /// survives renames, transfers and remote spellings), else for a shallow
    /// or empty history the normalized origin remote (`host/owner/repo`),
    /// else `local:` + a device-scoped hash of the common git dir. Opaque to
    /// readers: projects with equal ids group together.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repository_id: Option<String>,
    pub created_at: DateTime<Utc>,
}

impl Space {
    /// Name override, else basename(path), else the path itself.
    /// Lives here (proto) so UI and engine agree on the derivation.
    pub fn display_name(&self) -> &str {
        if let Some(name) = self.name.as_deref()
            && !name.trim().is_empty()
        {
            return name;
        }
        let trimmed = self.path.trim_end_matches(['/', '\\']);
        trimmed
            .rsplit(['/', '\\'])
            .next()
            .filter(|s| !s.is_empty())
            .unwrap_or(&self.path)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatConfig {
    pub harness: HarnessId,
    pub model: Option<String>,
    pub reasoning: Option<ReasoningLevel>,
    #[serde(default)]
    pub model_options: serde_json::Map<String, serde_json::Value>,
    pub sandbox: SandboxLevel,
    /// See [`PermissionMode`]. Serde-defaulted (Full) so rows written before
    /// the field existed keep today's unattended behavior.
    #[serde(default)]
    pub permission_mode: PermissionMode,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub instructions: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile_id: Option<String>,
    /// None runs the harness's default instance.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instance_id: Option<String>,
    /// Tools this chat runs without, in Claude's spelling (see
    /// [`crate::tools::ZERON_TOOL_PREFIX`]).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub disabled_tools: Vec<String>,
}

/// Immutable-at-run-start repository context owned by one conversation.
///
/// This is deliberately separate from the live checkout snapshot: another
/// chat may change the branch at the same checkout without changing which
/// branch this conversation belongs to.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConversationSourceContext {
    pub checkout_id: String,
    pub repo_root: String,
    pub cwd: String,
    pub branch: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub head_sha: Option<String>,
    pub observed_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Chat {
    pub id: String,
    /// Owning (host) device.
    pub device_id: String,
    pub title: Option<String>,
    pub archived: bool,
    pub cwd: Option<String>,
    pub branch: Option<String>,
    /// Canonical id of the repo checkout/worktree this chat operates in.
    pub checkout_id: Option<String>,
    /// Repository identity captured for this conversation immediately before
    /// its harness run. Unlike `branch`, this is never inferred from another
    /// chat sharing the same checkout.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_context: Option<ConversationSourceContext>,
    pub config: Option<ChatConfig>,
    pub last_message_preview: Option<String>,
    pub last_message_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    /// Harness-native session id of the chat's latest run — engine-owned resume
    /// continuity across engine restarts (zeron's `chats.harness_session_id`).
    /// Empty string = explicit
    /// "do not resume" tombstone after a rejected resume.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub harness_session_id: Option<String>,
    /// Cwd the harness session was created under. Harness session stores are
    /// cwd-scoped (claude keys conversations by project directory), so resume
    /// is only injected when the next run launches from the same cwd.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub harness_session_cwd: Option<String>,
    /// The space this chat belongs to. Invariant: `Some` for every UI-created
    /// chat; rows with a missing/dangling space id are not rendered (the host
    /// device's repair sweep deletes its own danglers).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub space_id: Option<String>,
    /// Synced LWW seen marker — compared against `last_message_at` to derive
    /// the "completed (finished but unseen)" indicator. Reading a chat on any
    /// device clears the badge everywhere.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_seen_at: Option<DateTime<Utc>>,
    /// Which sync room generation serves this chat (docs/chat2-sync.md M2):
    /// `None`/1 = legacy s2 loro room, 2 = chat2 dumb relay. The HOST flips
    /// this in the same breath as seeding the chat2 checkpoint; every device
    /// dials the room the registry names. Per-chat and instantly revertible.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub room_gen: Option<u32>,
    /// The chat this one is a guarded side conversation of. Set once at
    /// creation; `None` for an ordinary chat. Nesting is refused, so this
    /// chat's own id never appears here on a third chat.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sidechat_source_chat_id: Option<String>,
    /// The chat whose agent opened this one through the thread tools. Sidebar
    /// grouping only: the creator holds no authority over this chat, and the
    /// link never expires. `None` for a chat a person opened.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_by_chat_id: Option<String>,
    /// A label like `created_by_chat_id`: the source holds no authority.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub forked_from_chat_id: Option<String>,
    /// Every harness this chat ran on before the current one, oldest first,
    /// each once (a provider switch appends the harness it left). The sidebar
    /// row and tab stack their marks ahead of the current one.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub previous_harnesses: Vec<HarnessId>,
    /// Unlike `created_by_chat_id`, the delegating chat owns this one (d67).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delegation: Option<Delegation>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub settle: Option<SidebarSettle>,
    /// When the chat last left Settled; anchors it in the Inbox.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unsettled_at: Option<DateTime<Utc>>,
}

pub const SETTLE_AFTER_DAYS_DEFAULT: u32 = 3;

/// Per-device; edited by hand in `harness-prefs.json`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct SidebarSettings {
    pub settle_after_days: u32,
}

impl Default for SidebarSettings {
    fn default() -> Self {
        Self {
            settle_after_days: SETTLE_AFTER_DAYS_DEFAULT,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RouteTarget {
    pub harness: HarnessId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning: Option<ReasoningLevel>,
}

/// `source` stays a plain string so a device that does not know a newer value still reads the row.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChildRoute {
    #[serde(default)]
    pub source: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "lenient_option"
    )]
    pub table: Option<RouteTarget>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resets_at: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wait_until: Option<i64>,
}

fn lenient_option<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: serde::de::DeserializeOwned,
{
    let value = Option::<serde_json::Value>::deserialize(deserializer)?;
    Ok(value.and_then(|value| serde_json::from_value(value).ok()))
}

impl ChildRoute {
    pub const ROUTED: &str = "routed";
    pub const CHOSEN_BY_AGENT: &str = "chosen_by_agent";
    pub const FELL_BACK: &str = "fell_back";
}

/// Holds until the chat's next message after `at`, as T3's `settledOverride` resets on activity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SidebarSettle {
    pub settled: bool,
    pub at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Delegation {
    pub delegated_by_chat_id: String,
    /// 1 for a child of a person-opened chat, 2 for a grandchild.
    pub depth: u32,
    /// The task message's id in this chat; the result is what the agent
    /// wrote after it.
    pub anchor: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_request_id: Option<String>,
    /// The owner's turn start when it delegated; an owner Stop disposes the
    /// wakes of exactly that turn's children.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner_turn_started_at: Option<DateTime<Utc>>,
    pub wake: DelegationWake,
    pub status: DelegationStatus,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub cancel_requested: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delivery: Option<DelegationDelivery>,
    /// The result, clipped for rows; `task_status` reads the full text.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    pub started_at: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub finding_outcomes: BTreeMap<String, FindingOutcome>,
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub findings_handed: BTreeSet<String>,
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub findings_announced: BTreeSet<String>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "lenient_option"
    )]
    pub route: Option<ChildRoute>,
}

impl Delegation {
    pub fn is_running(&self) -> bool {
        self.status == DelegationStatus::Running
    }

    pub fn finding_handed(&self, id: &str) -> bool {
        self.wake == DelegationWake::Always || self.findings_handed.contains(id)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FindingOutcome {
    pub outcome: FindingOutcomeKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub commit: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FindingOutcomeKind {
    Fixed,
    Skipped,
    NoChange,
}

/// When a finished child wakes its owner. `SettledOnly` is a `mode: wait`
/// call: its blocking return carries the result while the owner's turn is
/// live, so only an owner that already settled is woken. `Never` is a child
/// the user started from the UI (a review), whose result the UI reads itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DelegationWake {
    Always,
    SettledOnly,
    Never,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DelegationStatus {
    Running,
    Completed,
    Failed,
    Cancelled,
    Interrupted,
}

/// Where the owner's wake for a finished child stands. `Claimed` = named by
/// a wake message not yet handed to the owner's agent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DelegationDelivery {
    Claimed,
    Delivered,
    Acknowledged,
    Disposed,
}

impl DelegationDelivery {
    pub fn is_final(self) -> bool {
        !matches!(self, Self::Claimed)
    }
}

impl Chat {
    pub fn sidechat_expired(&self, now: DateTime<Utc>) -> bool {
        self.sidechat_source_chat_id.is_some()
            && now.signed_duration_since(self.created_at)
                >= chrono::Duration::hours(crate::sidechat::SIDECHAT_LIFETIME_HOURS)
    }

    pub fn live_sidechat_source(&self, now: DateTime<Utc>) -> Option<&str> {
        self.sidechat_source_chat_id
            .as_deref()
            .filter(|_| !self.archived && !self.sidechat_expired(now))
    }

    /// A session the user sees in chat lists: neither a delegated child nor a
    /// hidden system chat.
    pub fn is_top_level(&self) -> bool {
        self.delegated_by().is_none() && !self.is_system()
    }

    pub fn is_system(&self) -> bool {
        crate::voice::is_orchestrator_chat(&self.id)
            || crate::supervisor::is_supervisor_chat(&self.id)
    }

    /// True when this chat syncs over the chat2 dumb relay.
    pub fn on_chat2(&self) -> bool {
        self.room_gen.unwrap_or(1) >= 2
    }

    pub fn delegated_by(&self) -> Option<&str> {
        self.delegation
            .as_ref()
            .map(|d| d.delegated_by_chat_id.as_str())
    }

    pub fn is_person_opened(&self) -> bool {
        self.created_by_chat_id.is_none() && self.delegation.is_none() && !self.is_system()
    }
}

impl Chat {
    /// Ignores the live status; Working and AwaitingInput chats never settle.
    /// A chat with no messages never settles on its own, as in T3.
    pub fn settled_at(&self, after_days: u32, now: DateTime<Utc>) -> Option<DateTime<Utc>> {
        let activity = self.last_message_at;
        match self
            .settle
            .filter(|settle| activity.is_none_or(|at| at <= settle.at))
        {
            Some(settle) => settle.settled.then_some(settle.at),
            None => activity.filter(|at| *at < now - chrono::Duration::days(i64::from(after_days))),
        }
    }

    /// True when the chat has activity the user hasn't seen on any device.
    pub fn unseen(&self) -> bool {
        match (self.last_message_at, self.last_seen_at) {
            (Some(msg), Some(seen)) => msg > seen,
            (Some(_), None) => true,
            (None, _) => false,
        }
    }
}

/// Display status for a chat row/tab: the four user-facing states plus a
/// distinct Errored. Derived — never stored.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ChatIndicator {
    Working,
    AwaitingInput,
    Errored,
    /// Finished running (or errored out) but not seen yet on any device.
    Completed,
    Idle,
}

/// Derive the display status. `live` must already be staleness-gated by the
/// caller (the UI's 45s window) — pass `None` for a stale/absent session row.
pub fn chat_indicator(chat: &Chat, live: Option<&Session>) -> ChatIndicator {
    match live.map(|s| s.status) {
        Some(SessionStatus::Working) => ChatIndicator::Working,
        Some(SessionStatus::AwaitingInput) => ChatIndicator::AwaitingInput,
        Some(SessionStatus::Errored) if chat.unseen() => ChatIndicator::Errored,
        _ if chat.unseen() => ChatIndicator::Completed,
        _ => ChatIndicator::Idle,
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Task {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub space_id: Option<String>,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub notes: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub due_on: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chat_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub order_key: String,
    #[serde(default)]
    pub created_at: DateTime<Utc>,
    #[serde(default)]
    pub updated_at: DateTime<Utc>,
    #[serde(default)]
    pub deleted: bool,
    #[serde(default, skip_serializing_if = "TaskMode::is_single")]
    pub mode: TaskMode,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub plan_pending: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub send_error: Option<String>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub plan: String,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub plan_doc: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub same_agents: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub resources: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub images: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub context: Vec<String>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TaskMode {
    #[default]
    Single,
    Orchestrated,
}

impl TaskMode {
    pub fn is_single(&self) -> bool {
        *self == Self::Single
    }
}

impl Task {
    pub fn has_plan(&self) -> bool {
        self.mode == TaskMode::Orchestrated && self.chat_id.is_some()
    }

    pub fn is_open_plan_of(&self, chat_id: &str) -> bool {
        !self.deleted
            && self.completed_at.is_none()
            && self.has_plan()
            && self.chat_id.as_deref() == Some(chat_id)
    }
}

/// Rows from before the one-open-plan rule resolve to the first match.
pub fn plan_of_chat<'a>(tasks: &'a [Task], chat_id: &str) -> Option<&'a Task> {
    tasks.iter().find(|task| task.is_open_plan_of(chat_id))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum LinkKind {
    Delegated,
    OpenedFrom,
    ForkedFrom,
    SidechatOf,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadLink {
    pub parent: String,
    pub kind: LinkKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub step_id: Option<String>,
}

/// Only `Delegated` is step-like; the other kinds are labels.
pub fn thread_link(chat: &Chat, tasks: &[Task], steps: &[TaskStep]) -> Option<ThreadLink> {
    if chat.is_system() {
        return None;
    }
    let link = |parent: &str, kind, step_id| {
        Some(ThreadLink {
            parent: parent.to_string(),
            kind,
            step_id,
        })
    };
    let step_under = |parent: &str| {
        steps
            .iter()
            .filter(|step| !step.deleted && step.chat_id.as_deref() == Some(chat.id.as_str()))
            .filter(|step| {
                tasks.iter().any(|task| {
                    task.id == step.task_id
                        && !task.deleted
                        && task.chat_id.as_deref() == Some(parent)
                })
            })
            .max_by_key(|step| step.outcome.is_none())
            .map(|step| step.id.clone())
    };
    if let Some(source) = &chat.forked_from_chat_id {
        return link(source, LinkKind::ForkedFrom, None);
    }
    if let Some(owner) = chat.delegated_by() {
        return link(owner, LinkKind::Delegated, step_under(owner));
    }
    if let Some(creator) = &chat.created_by_chat_id {
        return match step_under(creator) {
            Some(step) => link(creator, LinkKind::Delegated, Some(step)),
            None => link(creator, LinkKind::OpenedFrom, None),
        };
    }
    if let Some(source) = &chat.sidechat_source_chat_id {
        return link(source, LinkKind::SidechatOf, None);
    }
    None
}

/// A sidechat or a fork roots its own tree: neither link makes it a child.
pub fn tree_path<'a>(
    chat: &'a Chat,
    chats: &'a [Chat],
    tasks: &[Task],
    steps: &[TaskStep],
) -> Vec<&'a Chat> {
    let mut path = vec![chat];
    while let Some(parent) = path
        .last()
        .and_then(|node| thread_link(node, tasks, steps))
        .filter(|link| matches!(link.kind, LinkKind::Delegated | LinkKind::OpenedFrom))
        .and_then(|link| chats.iter().find(|chat| chat.id == link.parent))
    {
        if path.iter().any(|node| node.id == parent.id) {
            break;
        }
        path.push(parent);
    }
    path
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskStep {
    pub id: String,
    pub task_id: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub notes: String,
    #[serde(default)]
    pub order_key: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chat_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outcome: Option<StepOutcome>,
    #[serde(default)]
    pub created_at: DateTime<Utc>,
    #[serde(default)]
    pub updated_at: DateTime<Utc>,
    #[serde(default)]
    pub deleted: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turn: Option<StepTurn>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub images: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "lenient_option"
    )]
    pub route: Option<ChildRoute>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum StepOutcome {
    Done,
    Skipped,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StepTurn {
    pub ended: StepTurnEnd,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turn: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum StepTurnEnd {
    Completed,
    Failed,
    Stopped,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanEdit {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub overview: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub overview_images: Option<Vec<String>>,
    pub steps: Vec<PlanStepEdit>,
    #[serde(default)]
    pub remove_step_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub order: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanStepEdit {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub step_id: Option<String>,
    pub title: String,
    #[serde(default)]
    pub notes: String,
    #[serde(default)]
    pub images: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskStepPatch {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
    #[serde(
        default,
        deserialize_with = "present_nullable",
        skip_serializing_if = "Option::is_none"
    )]
    pub outcome: Option<Option<StepOutcome>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub order_key: Option<String>,
    #[serde(
        default,
        deserialize_with = "present_nullable",
        skip_serializing_if = "Option::is_none"
    )]
    pub chat_id: Option<Option<String>>,
    #[serde(
        default,
        deserialize_with = "present_nullable",
        skip_serializing_if = "Option::is_none"
    )]
    pub turn: Option<Option<StepTurn>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub images: Option<Vec<String>>,
    #[serde(
        default,
        deserialize_with = "present_nullable",
        skip_serializing_if = "Option::is_none"
    )]
    pub role: Option<Option<String>>,
    #[serde(
        default,
        deserialize_with = "present_nullable",
        skip_serializing_if = "Option::is_none"
    )]
    pub route: Option<Option<ChildRoute>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum StepStatus {
    Planned,
    Working,
    NeedsYou,
    Done,
    Skipped,
    Failed,
}

pub fn step_status(
    step: &TaskStep,
    thread: Option<&Chat>,
    indicator: Option<ChatIndicator>,
) -> StepStatus {
    match step.outcome {
        Some(StepOutcome::Done) => return StepStatus::Done,
        Some(StepOutcome::Skipped) => return StepStatus::Skipped,
        None => {}
    }
    let Some(thread) = thread.filter(|_| step.chat_id.is_some()) else {
        return StepStatus::Planned;
    };
    match indicator {
        Some(ChatIndicator::Working) => return StepStatus::Working,
        Some(ChatIndicator::AwaitingInput) => return StepStatus::NeedsYou,
        _ => {}
    }
    // Steps bound to a delegate_task child before steps ran on threads.
    if let Some(delegation) = &thread.delegation {
        return match delegation.status {
            DelegationStatus::Running => StepStatus::Working,
            DelegationStatus::Completed => StepStatus::Done,
            _ => StepStatus::NeedsYou,
        };
    }
    match step.turn.as_ref().map(|turn| turn.ended) {
        Some(StepTurnEnd::Completed) => StepStatus::Done,
        Some(StepTurnEnd::Failed) => StepStatus::Failed,
        Some(StepTurnEnd::Stopped) => StepStatus::NeedsYou,
        None => StepStatus::Working,
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlanProgress {
    pub done: usize,
    pub total: usize,
}

pub fn plan_progress(steps: &[StepStatus]) -> PlanProgress {
    PlanProgress {
        done: steps.iter().filter(|s| **s == StepStatus::Done).count(),
        total: steps.iter().filter(|s| **s != StepStatus::Skipped).count(),
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskPatch {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
    #[serde(
        default,
        deserialize_with = "present_nullable",
        skip_serializing_if = "Option::is_none"
    )]
    pub due_on: Option<Option<String>>,
    #[serde(
        default,
        deserialize_with = "present_nullable",
        skip_serializing_if = "Option::is_none"
    )]
    pub space_id: Option<Option<String>>,
    #[serde(
        default,
        deserialize_with = "present_nullable",
        skip_serializing_if = "Option::is_none"
    )]
    pub chat_id: Option<Option<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completed: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub order_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<TaskMode>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan_pending: Option<bool>,
    #[serde(
        default,
        deserialize_with = "present_nullable",
        skip_serializing_if = "Option::is_none"
    )]
    pub send_error: Option<Option<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan_doc: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub same_agents: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resources: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub images: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context: Option<Vec<String>>,
}

pub fn present_nullable<'de, D, T>(deserializer: D) -> Result<Option<Option<T>>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::<T>::deserialize(deserializer).map(Some)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TaskStatus {
    ToDo,
    Working,
    NeedsYou,
    Done,
}

/// A stopped linked thread waits on the person until they tick the task.
pub fn task_status(task: &Task, linked: Option<ChatIndicator>, steps: &[StepStatus]) -> TaskStatus {
    if task.completed_at.is_some() {
        return TaskStatus::Done;
    }
    let lead_working = linked == Some(ChatIndicator::Working);
    if task.send_error.is_some()
        || matches!(
            linked,
            Some(ChatIndicator::AwaitingInput | ChatIndicator::Errored)
        )
        || steps.contains(&StepStatus::NeedsYou)
        || (steps.contains(&StepStatus::Failed) && !lead_working)
        || (task.plan_pending && !steps.is_empty() && !lead_working)
    {
        return TaskStatus::NeedsYou;
    }
    if lead_working || steps.contains(&StepStatus::Working) {
        return TaskStatus::Working;
    }
    match linked {
        Some(_) => TaskStatus::NeedsYou,
        None => TaskStatus::ToDo,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SessionStatus {
    Idle,
    Working,
    AwaitingInput,
    Errored,
}

/// Work a harness runs beside its turns — a background shell today. Lives
/// only as long as the harness process that owns it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BackgroundTask {
    pub id: String,
    pub kind: BackgroundTaskKind,
    pub label: String,
    /// Stamped by the engine the first time it sees the task; the harness
    /// wire carries no start time.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub started_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_path: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum BackgroundTaskKind {
    Shell,
    Agent,
    #[serde(other)]
    Other,
}

/// Live run status for a chat — drives the Working indicator and sidebar status dots.
/// Staleness-checked client-side against `updated_at` so a crashed backend never shows
/// an eternal "Working".
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Session {
    /// Last successfully completed assistant turn. Retained while the next turn
    /// runs so coalesced status watches do not lose normal queue completions.
    /// Interrupts, failures and liveness expiry never advance this marker.
    #[serde(default)]
    pub last_completed_turn: Option<String>,
    pub chat_id: String,
    pub device_id: String,
    pub status: SessionStatus,
    /// `Working` held only by background subagents: the turn itself has
    /// parked, and the next prompt goes straight through (a send, not a
    /// queue). The dot still reads Working.
    #[serde(default)]
    pub held_by_subagents: bool,
    #[serde(default)]
    pub compacting: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub classifying: Option<Classifying>,
    /// The run's live background tasks; independent of `status`, which
    /// they never hold Working.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub background_tasks: Vec<BackgroundTask>,
    pub started_at: Option<DateTime<Utc>>,
    pub updated_at: DateTime<Utc>,
    /// Subagents of this chat streaming right now. Rides the session row (and
    /// its staleness window) so every device's sidebar can badge a chat it
    /// has not opened; read it through `view::running_subagents`.
    #[serde(default)]
    pub running_subagents: u32,
    /// Host-local, never mirrored into the workspace doc.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub compact_plan: Option<CompactPlan>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Classifying {
    pub harness: crate::HarnessId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum CompactPlan {
    Idle { at: i64 },
    Agent,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Repo {
    pub path: String,
    pub name: String,
    pub default_branch: Option<String>,
}

/// One row of `ListRefs`: a branch plus its checkout state — whether it is
/// the repo's current (main-checkout) branch and whether it is materialized
/// as a linked worktree. Drives the composer's ref picker (`current` /
/// `worktree` tags) and the checkout-kind selector.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RepoRef {
    pub name: String,
    /// Checked out in the repo's MAIN folder right now.
    #[serde(default)]
    pub current: bool,
    /// Path of the linked worktree this branch is checked out in, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub worktree_path: Option<String>,
}

/// Public Git reference attached to a commit in the history graph.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum GitHistoryRefKind {
    Branch,
    Remote,
    Tag,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GitHistoryRef {
    pub kind: GitHistoryRefKind,
    pub label: String,
}

/// One topologically ordered row in the repository history graph.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GitHistoryCommit {
    pub sha: String,
    pub parent_shas: Vec<String>,
    pub subject: String,
    pub author_name: String,
    pub author_email: String,
    pub authored_at: String,
    #[serde(default)]
    pub refs: Vec<GitHistoryRef>,
}

/// Divergence between the checked-out branch and the repository's integration
/// branch. Counts are computed only from locally available refs; callers must
/// fetch explicitly when they want newer remote state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GitHistoryComparison {
    /// The local remote-tracking ref used as the comparison base, e.g.
    /// `upstream/main`.
    pub base: String,
    /// Commits reachable from HEAD but not from [`Self::base`].
    pub ahead: usize,
    /// Commits reachable from [`Self::base`] but not from HEAD.
    pub behind: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GitHistoryPage {
    pub commits: Vec<GitHistoryCommit>,
    /// Deduplicated tips of every public local/remote branch. Populated with
    /// the first page so clients can switch to the compact overview without
    /// another round trip or loading the complete history.
    #[serde(default)]
    pub branch_tips: Vec<GitHistoryCommit>,
    pub head_sha: Option<String>,
    pub next_cursor: Option<usize>,
    pub total_count: Option<usize>,
    /// Number of commits reachable from the active checkout's HEAD.
    #[serde(default)]
    pub head_commit_count: Option<usize>,
    /// Current branch divergence from the preferred integration branch.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub comparison: Option<GitHistoryComparison>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Worktree {
    pub repo_path: String,
    pub path: String,
    pub branch: String,
    /// Generated worktree folder name (`zeron/<name>` is its branch).
    #[serde(default)]
    pub name: String,
    /// Canonical checkout identity (device-scoped hash of the git dir).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checkout_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FolderEntry {
    pub name: String,
    pub is_dir: bool,
    pub is_repo: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FolderListing {
    pub path: String,
    pub entries: Vec<FolderEntry>,
    /// True when the listing hit the entry cap.
    #[serde(default)]
    pub truncated: bool,
}

/// Device-local preferences, used only when creating a new worktree.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct WorktreeSettings {
    pub use_custom_directory: bool,
    /// Remembered even while the custom location is disabled.
    pub custom_directory: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorktreeSettingsStatus {
    pub settings: WorktreeSettings,
    pub default_directory: String,
    pub effective_directory: String,
    /// An explicit host environment override takes precedence over preferences.
    pub environment_override: Option<String>,
}

/// `SearchHomeFolders`: folders anywhere under the host's home whose names
/// match `query`, for the new-project picker. An empty query only starts the
/// host's home search index.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchHomeFoldersRequest {
    pub query: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<u16>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HomeFolderMatch {
    /// Absolute path on the host.
    pub path: String,
    /// Home-relative, `/`-separated (what the picker shows).
    pub relative: String,
    pub is_repo: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HomeFolderSearchResult {
    pub matches: Vec<HomeFolderMatch>,
    /// The host's home index was still scanning; results may be incomplete.
    pub indexing: bool,
}

/// A browse root beyond home: a mounted drive/volume (or the system root).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DriveEntry {
    /// Display name (volume label / mount folder name; "System" for `/`).
    pub name: String,
    /// Absolute mount point.
    pub path: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DriveListing {
    pub drives: Vec<DriveEntry>,
}

/// A workspace-relative file or directory returned by `SearchFiles`.
/// Contents deliberately never cross this boundary: mentioning a path leaves
/// the harness to read it through its normal workspace tools when needed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileSearchMatch {
    pub path: String,
    pub is_dir: bool,
}

/// Identifies the local checkout used by workspace file operations.
/// Exactly one of `chat_id` and `space_id` must be present.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceTarget {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chat_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub space_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checkout_path: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListWorkspaceDirectoryRequest {
    #[serde(flatten)]
    pub target: WorkspaceTarget,
    #[serde(default)]
    pub directory: String,
    #[serde(default)]
    pub include_ignored: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cursor: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceDirectoryPage {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checkout_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mutation_capabilities: Option<WorkspaceMutationCapabilities>,
    pub directory: String,
    pub entries: Vec<WorkspaceEntry>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
    pub truncated: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceEntry {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mutation_revision: Option<String>,
    pub path: String,
    pub name: String,
    pub kind: WorkspaceEntryKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub modified_at: Option<DateTime<Utc>>,
    pub ignored: bool,
    pub read_only: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum WorkspaceEntryKind {
    File,
    Directory,
    Symlink,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchWorkspaceFilesRequest {
    #[serde(flatten)]
    pub target: WorkspaceTarget,
    pub query: String,
    #[serde(default)]
    pub include_ignored: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<u16>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceFileSearchMatch {
    pub path: String,
    pub name: String,
    pub kind: WorkspaceEntryKind,
    pub score: i64,
}

/// Plain-text search over a workspace's file contents (smart case). Binary
/// and oversized files are skipped.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchWorkspaceContentRequest {
    #[serde(flatten)]
    pub target: WorkspaceTarget,
    pub query: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub per_file_limit: Option<u16>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceContentMatch {
    /// Workspace-relative, `/`-separated.
    pub path: String,
    /// 1-based.
    pub line: u64,
    /// 0-based byte column of the first match in the original line.
    pub column: u64,
    /// The matched line, leading whitespace trimmed, at most 512 bytes.
    pub preview: String,
    /// `[start, end)` byte ranges of the matches within `preview`.
    pub ranges: Vec<(u32, u32)>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceContentSearchResult {
    pub matches: Vec<WorkspaceContentMatch>,
    /// More matches exist, or the host's time budget ran out.
    pub truncated: bool,
    /// The host's index was still scanning; results may be incomplete.
    pub indexing: bool,
}

/// Start (or keep alive) the host's search index for a workspace, so the
/// chat's first search finds it built.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WarmWorkspaceSearchRequest {
    #[serde(flatten)]
    pub target: WorkspaceTarget,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum WorkspaceSearchIndexState {
    /// The initial scan is still running; searches may be incomplete.
    Building,
    Ready,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WarmWorkspaceSearchResult {
    pub state: WorkspaceSearchIndexState,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReadWorkspaceFileRequest {
    #[serde(flatten)]
    pub target: WorkspaceTarget,
    pub path: String,
}

/// Workspace images travel in bounded relay frames, independently of text reads.
pub const MAX_WORKSPACE_IMAGE_BYTES: usize = 8 * 1024 * 1024;
pub const WORKSPACE_IMAGE_CHUNK_BYTES: usize = 384 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReadWorkspaceImageRequest {
    #[serde(flatten)]
    pub target: WorkspaceTarget,
    pub path: String,
    pub expected_checkout_id: String,
    pub offset: usize,
    pub expected_content_hash: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceImageChunk {
    pub checkout_id: String,
    pub content_hash: String,
    pub mime_type: String,
    pub data: String,
    pub next_offset: usize,
    pub size: usize,
    pub done: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceFileText {
    /// Identity of the checkout this snapshot was read from.
    pub checkout_id: String,
    pub path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_hash: Option<String>,
    pub size: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub modified_at: Option<DateTime<Utc>>,
    pub encoding: WorkspaceTextEncoding,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line_ending: Option<WorkspaceLineEnding>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub read_only_reason: Option<WorkspaceReadOnlyReason>,
    pub truncated: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum WorkspaceTextEncoding {
    Utf8,
    Utf8Bom,
    Binary,
    Unsupported,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum WorkspaceLineEnding {
    Lf,
    Crlf,
    Mixed,
    None,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum WorkspaceReadOnlyReason {
    Binary,
    UnsupportedEncoding,
    MixedLineEndings,
    Symlink,
    TooLarge,
    PermissionDenied,
    NotRegularFile,
    /// A file read by absolute path beyond the chat's workspace root: reads
    /// are allowed, writes never are. Older peers never see this variant —
    /// only a new UI asks for an absolute path — so the wire stays additive.
    OutsideWorkspace,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WriteWorkspaceFileRequest {
    /// Must match the read snapshot, even if the chat has since changed cwd.
    pub expected_checkout_id: String,
    #[serde(flatten)]
    pub target: WorkspaceTarget,
    pub path: String,
    pub text: String,
    pub expected_content_hash: String,
    pub encoding: WorkspaceWritableEncoding,
    pub line_ending: WorkspaceWritableLineEnding,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum WorkspaceWritableEncoding {
    Utf8,
    Utf8Bom,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum WorkspaceWritableLineEnding {
    Lf,
    Crlf,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "camelCase")]
pub enum WriteWorkspaceFileOutcome {
    #[serde(rename_all = "camelCase")]
    Written { file: WorkspaceFileWriteResult },
    #[serde(rename_all = "camelCase")]
    Conflict {
        reason: WorkspaceFileConflictReason,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        current_content_hash: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        current_modified_at: Option<DateTime<Utc>>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceFileWriteResult {
    pub path: String,
    pub content_hash: String,
    pub size: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub modified_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum WorkspaceFileConflictReason {
    Changed,
    Deleted,
    Replaced,
    NotRegularFile,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WatchWorkspaceFilesRequest {
    #[serde(flatten)]
    pub target: WorkspaceTarget,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceFileChanges {
    pub sequence: u64,
    pub resync_required: bool,
    pub changes: Vec<WorkspaceFileChange>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceFileChange {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub operation_id: Option<String>,
    pub kind: WorkspaceFileChangeKind,
    pub path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub old_path: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum WorkspaceFileChangeKind {
    Created,
    Modified,
    Removed,
    Renamed,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiffFileSummary {
    pub path: String,
    /// Previous path for renames/copies.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub old_path: Option<String>,
    pub status: String,
    pub additions: u32,
    pub deletions: u32,
    #[serde(default)]
    pub binary: bool,
}

/// Git porcelain states, independent of patch size and line counts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum GitFileState {
    Unchanged,
    Added,
    Modified,
    Deleted,
    Renamed,
    Copied,
    Unmerged,
    Untracked,
    TypeChanged,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GitFileStatus {
    pub path: String,
    pub old_path: Option<String>,
    pub index: GitFileState,
    pub worktree: GitFileState,
}

/// Latest status only: never contains file content or a patch. `complete = false`
/// means unavailable/partial, not clean. Revision covers only these statuses.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CheckoutGitStatus {
    pub checkout_id: String,
    pub device_id: String,
    pub revision: String,
    pub complete: bool,
    pub files: Vec<GitFileStatus>,
}

/// Keep unavailable updates inside an object: the RPC envelope uses JSON null
/// for a missing item, so a bare optional snapshot cannot signal invalidation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceGitStatusFrame {
    pub status: Option<CheckoutGitStatus>,
}

/// Working-tree diff for a checkout — latest-only sidecar, 3MiB patch cap.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CheckoutDiff {
    pub checkout_id: String,
    pub device_id: String,
    pub cwd: String,
    pub patch: String,
    pub files: Vec<DiffFileSummary>,
    pub additions: u32,
    pub deletions: u32,
    /// True when the patch was truncated at the byte cap ("Partial snapshot").
    pub truncated: bool,
    pub checksum: String,
    pub updated_at: DateTime<Utc>,
}

/// Provider-neutral lifecycle state for a code change request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ChangeRequestState {
    Open,
    Closed,
    Merged,
}

/// Whether a change request can be merged without resolving conflicts first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ChangeRequestMergeability {
    Mergeable,
    Conflicting,
    Unknown,
}

/// Aggregate review decision reported for a change request.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ChangeRequestReviewDecision {
    Approved,
    ChangesRequested,
    ReviewRequired,
    #[default]
    Unknown,
}

/// Compact provider-neutral change request metadata for checkout surfaces.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChangeRequestSummary {
    pub provider: String,
    pub number: u64,
    pub title: String,
    pub url: String,
    pub state: ChangeRequestState,
    pub base_ref: String,
    pub head_ref: String,
}

/// Relationship filter for one repository; never widens repository scope.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ChangeRequestFilter {
    #[default]
    Authored,
    All,
    Reviewing,
}

/// Provider-neutral change request metadata for global listing surfaces.
///
/// Unlike [`ChangeRequestSummary`], this contract is not tied to a checkout
/// and therefore identifies the repository instead of requiring branch refs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChangeRequestListItem {
    pub provider: String,
    #[serde(default)]
    pub author: ChangeRequestActor,
    pub repository: String,
    /// Current source commit, captured with the CI rollup in one provider request.
    #[serde(default)]
    pub head_ref_oid: String,
    #[serde(default)]
    pub ci: crate::change_request_assessment::ChangeRequestCi,
    /// None means an older host or unavailable viewer relationship metadata.
    #[serde(default)]
    pub viewer_did_author: Option<bool>,
    #[serde(default)]
    pub viewer_review_requested: Option<bool>,
    pub number: u64,
    pub title: String,
    pub url: String,
    pub state: ChangeRequestState,
    pub is_draft: bool,
    /// `Unknown` when the provider reports no decision.
    #[serde(default)]
    pub review_decision: ChangeRequestReviewDecision,
    pub additions: u64,
    pub deletions: u64,
    pub mergeability: ChangeRequestMergeability,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// One page of a repository-scoped listing, newest updates first.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ChangeRequestPage {
    pub items: Vec<ChangeRequestListItem>,
    /// Opaque provider cursor for the following page; `None` on the last one.
    pub next_cursor: Option<String>,
    /// Every match for the query, when the provider reports it.
    pub total_count: Option<u64>,
    /// When the engine fetched this page from the provider. A page served
    /// from the engine's on-disk copy carries its original time, so clients
    /// can show it at once and refresh it when it has aged. Absent from
    /// engines that predate the copy.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fetched_at: Option<DateTime<Utc>>,
}

/// Read-only GitHub detail response. Optional collections tolerate absent provider data.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ChangeRequestDetail {
    pub title: String,
    pub body: String,
    pub url: String,
    pub number: u64,
    pub author: ChangeRequestActor,
    pub base_ref_name: String,
    pub base_ref_oid: String,
    pub head_ref_name: String,
    pub head_ref_oid: String,
    pub ci: crate::change_request_assessment::ChangeRequestCi,
    pub viewer_did_author: Option<bool>,
    pub viewer_review_requested: Option<bool>,
    pub state: String,
    pub is_draft: bool,
    pub review_decision: String,
    pub mergeable: String,
    pub additions: u64,
    pub deletions: u64,
    pub comments: Vec<ChangeRequestComment>,
    pub reviews: Vec<ChangeRequestComment>,
    /// None means unavailable (including an older engine), not no feedback.
    pub review_threads: Option<Vec<ChangeRequestReviewThread>>,
    pub files: Vec<ChangeRequestFile>,
    pub status_check_rollup: Vec<ChangeRequestCheck>,
    /// The signed-in GitHub account, so clients can tell which comments and
    /// reviews are the viewer's. Absent from older engines.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub viewer_login: Option<String>,
}

impl ChangeRequestDetail {
    pub fn activity_comments(&self) -> impl Iterator<Item = &ChangeRequestComment> {
        self.comments.iter().chain(&self.reviews).chain(
            self.review_threads
                .iter()
                .flatten()
                .flat_map(|thread| &thread.comments),
        )
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ChangeRequestReviewThread {
    pub id: String,
    pub is_resolved: bool,
    pub is_outdated: bool,
    pub path: String,
    pub line: Option<u64>,
    pub start_line: Option<u64>,
    pub original_line: Option<u64>,
    pub original_start_line: Option<u64>,
    pub diff_side: String,
    pub start_diff_side: Option<String>,
    pub comments: Vec<ChangeRequestComment>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ChangeRequestActor {
    pub login: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ChangeRequestComment {
    pub id: String,
    pub url: String,
    pub reply_to: Option<ChangeRequestCommentReference>,
    /// Provided by GitHub for the authenticated viewer, independent of PR authorship.
    pub viewer_did_author: bool,
    pub author: ChangeRequestActor,
    pub body: String,
    pub state: String,
    pub created_at: String,
    pub submitted_at: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ChangeRequestCommentReference {
    pub id: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ChangeRequestFile {
    pub path: String,
    pub additions: u64,
    pub deletions: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ChangeRequestCheck {
    pub name: String,
    pub context: String,
    pub status: String,
    pub conclusion: String,
    pub state: String,
    pub details_url: String,
    pub target_url: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ChangeRequestMergeMethod {
    Merge,
    Squash,
    Rebase,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ChangeRequestReviewEvent {
    Comment,
    Approve,
    RequestChanges,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ChangeRequestControls {
    pub merge_methods: Vec<ChangeRequestMergeMethod>,
    pub can_write: bool,
    pub can_triage: bool,
    pub did_author: bool,
    pub labels: Vec<ChangeRequestLabelCandidate>,
    pub reviewers: Vec<ChangeRequestReviewerCandidate>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ChangeRequestLabelCandidate {
    pub name: String,
    pub color: Option<String>,
    pub description: Option<String>,
    pub applied: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ChangeRequestLabelCandidates {
    pub labels: Vec<ChangeRequestLabelCandidate>,
    pub truncated: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ChangeRequestReviewerCandidate {
    pub login: String,
    pub name: Option<String>,
    pub team: bool,
    pub requested: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ChangeRequestReviewerCandidates {
    pub reviewers: Vec<ChangeRequestReviewerCandidate>,
    pub truncated: bool,
}

/// Latest successful change request resolution for one checkout and branch.
///
/// `change_request: None` is an authoritative successful lookup with no match;
/// resolution failures must retain the previous successful snapshot instead.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CheckoutChangeRequestStatus {
    pub checkout_id: String,
    pub device_id: String,
    pub cwd: String,
    pub branch: String,
    pub change_request: Option<ChangeRequestSummary>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GetCheckoutFileDiffTextRequest {
    pub checkout_id: String,
    pub cwd: String,
    pub path: String,
    #[serde(default)]
    pub mode: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_ref: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chat_id: Option<String>,
    /// Pinned commit for History's per-commit diff scope. When present, the
    /// source pair is read from the commit parent and this commit, never from
    /// the live working tree.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub commit_sha: Option<String>,
    pub diff_checksum: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CheckoutFileDiffText {
    pub diff_checksum: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub old_text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub new_text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub old_content_hash: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub new_content_hash: Option<String>,
    pub binary: bool,
    pub truncated: bool,
    #[serde(default)]
    pub stale: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UserProfile {
    pub id: String,
    pub email: String,
    pub name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "camelCase")]
pub enum AuthState {
    SignedOut,
    NeedsOrganization {
        user: UserProfile,
    },
    #[serde(rename_all = "camelCase")]
    SignedIn {
        user: UserProfile,
        org_id: Option<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentAccount {
    pub id: String,
    pub harness: HarnessId,
    pub email: Option<String>,
    pub plan_label: Option<String>,
    pub active: bool,
    #[serde(default)]
    pub usage_windows: Vec<AgentUsageWindow>,
    /// Epoch millis the `usage_windows` were fetched. The engine serves the
    /// last good probe (persisted across restarts) while a refresh runs, so
    /// windows may be minutes old; `None` = never fetched.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage_fetched_at: Option<i64>,
    /// Why the last usage probe failed ("Rate limited — retrying in 2m",
    /// "Sign in again", …), shown instead of a bare "Usage unavailable" — or
    /// beside stale windows. `None` when the last probe succeeded or none ran.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage_error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub organization: Option<String>,
    /// How the CLI is signed in (`oauth` account vs raw `api-key`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auth_kind: Option<AgentAuthKind>,
    /// False for a live login whose credentials we could not read (e.g. macOS
    /// Keychain denied) or whose account couldn't be identified — shown, but
    /// not re-activatable. Always false for Hermes: Hermes owns its
    /// credential pool (it picks and rotates entries itself), so zeron lists
    /// it read-only — no switch, no remove; accounts are added through
    /// `hermes auth add`.
    #[serde(default)]
    pub switchable: bool,
    /// Epoch millis of the slot's last snapshot.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub saved_at: Option<i64>,
    /// The upstream login this row belongs to inside an agent that keeps one
    /// login PER model provider (OpenCode's `openai`, Pi's `anthropic`,
    /// Hermes' `nous`). Rows sharing it form one single-choice group — at
    /// most one of them is in use. `None` for single-login agents.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    /// Kept apart from `usage_windows` so it never becomes the binding limit.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extra_usage: Option<AgentExtraUsage>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AgentAuthKind {
    Oauth,
    ApiKey,
}

/// Everything the Accounts settings page renders, rebuilt after every mutation.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentAccountsSnapshot {
    pub accounts: Vec<AgentAccount>,
    pub warnings: Vec<AgentAccountWarning>,
}

/// A per-harness detection warning (e.g. Keychain denied reading the live login).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentAccountWarning {
    pub harness: HarnessId,
    pub message: String,
}

/// `StartAgentLogin` reply: open `url`, then either paste the code back
/// (`CompleteAgentLogin`) or poll until the browser flow lands (`PollAgentLogin`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentLoginStart {
    pub login_id: String,
    /// Empty when the sign-in page is only known later (a poll carries it).
    pub url: String,
    pub mode: AgentLoginMode,
    /// The loopback port the login's OAuth redirect lands on, on the device
    /// running the login. A requester on ANOTHER device forwards that same
    /// port on its own loopback to it, so its browser finishes the redirect.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub callback_port: Option<u16>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AgentLoginMode {
    /// Claude's fallback when no loopback port could be bound: the user
    /// pastes the OAuth code back into the app.
    PasteCode,
    /// A loopback callback completes the sign-in in the browser (every
    /// provider's default); poll until done.
    Browser,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentLoginPoll {
    pub status: AgentLoginStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    /// a sign-in page that only became known after the start reply (the
    /// agent had to install first); the app opens it once.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    /// [`AgentLoginStart::callback_port`] for a page that arrived with this
    /// poll.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub callback_port: Option<u16>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AgentLoginStatus {
    Pending,
    Done,
    Error,
}

/// CLI plan rate-limit window (accounts settings meters) — NOT app token accounting.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentUsageWindow {
    pub label: String,
    /// 0.0..=1.0
    pub used_fraction: f32,
    pub resets_at: Option<DateTime<Utc>>,
}

/// Claude's usage credits, spent past the plan limits; amounts in `currency` units.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentExtraUsage {
    pub enabled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub used: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub currency: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ProjectActionIcon {
    Play,
    Test,
    Lint,
    Configure,
    Build,
    Debug,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectAction {
    pub id: String,
    pub name: String,
    pub command: String,
    pub icon: ProjectActionIcon,
    pub run_on_worktree_create: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectActionDraft {
    pub name: String,
    pub command: String,
    pub icon: ProjectActionIcon,
    #[serde(default)]
    pub run_on_worktree_create: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentProfile {
    pub id: String,
    pub name: String,
    pub harness: HarnessId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning: Option<ReasoningLevel>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub permission_mode: Option<PermissionMode>,
    #[serde(default, skip_serializing_if = "serde_json::Map::is_empty")]
    pub model_options: serde_json::Map<String, serde_json::Value>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub notes: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub instructions: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub disabled_tools: Vec<String>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub by_agent: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentProfileDraft {
    pub name: String,
    pub harness: HarnessId,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub reasoning: Option<ReasoningLevel>,
    #[serde(default)]
    pub permission_mode: Option<PermissionMode>,
    #[serde(default)]
    pub model_options: serde_json::Map<String, serde_json::Value>,
    #[serde(default)]
    pub notes: String,
    #[serde(default)]
    pub instructions: String,
    #[serde(default)]
    pub disabled_tools: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentProfilesSnapshot {
    pub profiles: Vec<AgentProfile>,
}

/// "Claude Code · Opus 5.5 · Auto · High · 2 tools off" under a profile's name.
pub fn profile_line(
    harness_name: &str,
    model: Option<&str>,
    mode: Option<PermissionMode>,
    reasoning: Option<ReasoningLevel>,
    tools_off: usize,
) -> String {
    let mut parts = vec![harness_name.to_string()];
    if let Some(model) = model {
        parts.push(model.to_string());
    }
    if let Some(mode) = mode {
        parts.push(mode.short_label().to_string());
    }
    if let Some(level) = reasoning {
        parts.push(level.label().to_string());
    }
    if tools_off > 0 {
        parts.push(format!("{tools_off} tools off"));
    }
    parts.join(" · ")
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectActionsSnapshot {
    pub space_id: String,
    pub actions: Vec<ProjectAction>,
    pub importable_actions: Vec<ProjectActionDraft>,
    pub project_file_issue: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectActionRun {
    pub action_id: String,
    pub action_name: String,
    pub terminal: TerminalSession,
}

/// Result of creating a worktree. The worktree remains flattened so this is
/// wire-compatible with both legacy callers and legacy engine replies.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateWorktreeOutcome {
    #[serde(flatten)]
    pub worktree: Worktree,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub setup_action: Option<ProjectActionRun>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub setup_error: Option<String>,
}

/// An open PTY session on the owning device (`OpenTerminal` reply).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TerminalSession {
    pub id: String,
    pub cwd: String,
    /// Shell basename (`zsh`, `bash`, …) for the tab label.
    pub shell: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TerminalInfo {
    pub id: String,
    pub cwd: String,
    pub shell: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selection: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TerminalScreen {
    pub terminal: TerminalInfo,
    pub cols: u16,
    pub rows: u16,
    pub alt_screen: bool,
    pub cursor_row: usize,
    pub cursor_col: usize,
    pub lines: Vec<String>,
    pub truncated: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentTerminalOpened {
    pub chat_id: String,
    pub terminal: TerminalSession,
}

/// One `SubscribeTerminal` stream item. `seq` is a per-terminal monotonic counter
/// used for replay resumption (`afterSeq`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum TerminalEvent {
    /// Output chunk; `data` is base64 (PTY output is raw bytes, not valid UTF-8).
    Data { seq: u64, data: String },
    #[serde(rename_all = "camelCase")]
    Exit {
        seq: u64,
        exit_code: i32,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        signal: Option<String>,
    },
}

/// One in-flight queued-attachment transfer (the `WatchTransfers` stream):
/// raw-byte progress of the engine-side relay leg pushing staged bytes to a
/// remote host. An entry appears when a file's chunks start moving, updates
/// per landed chunk, and disappears when the host commits it (or the attempt
/// fails — the retry re-adds it). Keyed by the send-minted uploadId, so the
/// sender's thumbnails can resolve their `pending://{uploadId}/…` refs to a
/// real percent instead of an indeterminate spinner.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TransferProgress {
    pub upload_id: String,
    pub file_name: String,
    /// Raw bytes the host has acknowledged so far.
    pub done: u64,
    /// Total raw bytes of the staged file.
    pub total: u64,
}

/// Live edge-connectivity posture (the `WatchConnectivity` stream): the truth
/// the connection pill, composer honesty, and queued-send badges render.
/// Derived engine-side from the registry room's reconnect state, the OS
/// network-path monitor, and each open chat room's stats.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Connectivity {
    pub state: ConnectivityState,
    /// Epoch ms of the next scheduled registry dial while reconnecting
    /// (0 = none pending / dialing right now). The countdown renders
    /// client-side from this.
    #[serde(default)]
    pub retry_at_ms: i64,
    /// The failure that started the current outage — sticky through the next
    /// attempt (no flicker back to a bare "connecting…"), cleared on rejoin.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_failure: Option<String>,
    /// Per-OPEN-chat room state; a chat absent here is unknown (consumers
    /// fall back to the global state).
    #[serde(default)]
    pub chats: Vec<ChatConnectivity>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ConnectivityState {
    /// No edge transports on this profile (local scope) — hide the pill.
    #[default]
    Disabled,
    /// The OS reports no network path.
    Offline,
    /// Edge expected but the registry room is down (dialing/backing off).
    Reconnecting,
    Connected,
}

/// Additive per-chat admission status; legacy peers omit it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ChatSyncState {
    Local,
    Waiting,
    Connecting,
    Synced,
    Offline,
    StorageError,
    #[default]
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatConnectivity {
    pub chat_id: String,
    #[serde(default)]
    pub sync_state: ChatSyncState,
    /// Grace-filtered per-chat health; true does not prove live delivery.
    pub connected: bool,
    /// This chat can currently deliver over its room or HTTP fallback.
    /// Older engines omit it, so consumers conservatively assume false.
    #[serde(default)]
    pub delivery_live: bool,
    /// Local update batches not yet acked by the chat's edge room.
    #[serde(default)]
    pub pending_pushes: u64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn task_status_derives_from_completion_then_the_linked_thread() {
        let mut task: Task = serde_json::from_value(serde_json::json!({ "id": "t" })).unwrap();
        assert_eq!(task_status(&task, None, &[]), TaskStatus::ToDo);
        assert_eq!(
            task_status(&task, Some(ChatIndicator::Working), &[]),
            TaskStatus::Working
        );
        for stopped in [
            ChatIndicator::AwaitingInput,
            ChatIndicator::Errored,
            ChatIndicator::Completed,
            ChatIndicator::Idle,
        ] {
            assert_eq!(task_status(&task, Some(stopped), &[]), TaskStatus::NeedsYou);
        }
        task.send_error = Some("offline".into());
        assert_eq!(
            task_status(&task, Some(ChatIndicator::Working), &[]),
            TaskStatus::NeedsYou,
            "a failed send waits on the person even while the thread runs"
        );
        task.send_error = None;
        task.completed_at = Some(Utc::now());
        assert_eq!(
            task_status(&task, Some(ChatIndicator::Working), &[]),
            TaskStatus::Done
        );
    }

    #[test]
    fn an_orchestrated_task_rolls_its_steps_up_under_the_lead() {
        use StepStatus::*;
        let mut task: Task = serde_json::from_value(serde_json::json!({ "id": "t" })).unwrap();
        let lead = Some(ChatIndicator::Idle);
        assert_eq!(
            task_status(&task, lead, &[Done, Working]),
            TaskStatus::Working
        );
        assert_eq!(
            task_status(&task, Some(ChatIndicator::Working), &[Planned]),
            TaskStatus::Working
        );
        assert_eq!(
            task_status(&task, Some(ChatIndicator::Working), &[Working, NeedsYou]),
            TaskStatus::NeedsYou,
            "a step that needs the person outranks a working lead"
        );
        assert_eq!(
            task_status(&task, Some(ChatIndicator::Errored), &[Working]),
            TaskStatus::NeedsYou
        );
        assert_eq!(
            task_status(&task, lead, &[Done, Skipped]),
            TaskStatus::NeedsYou
        );
        assert_eq!(task_status(&task, None, &[Working]), TaskStatus::Working);
        assert_eq!(
            task_status(&task, lead, &[Failed, Working]),
            TaskStatus::NeedsYou,
            "a failed step waits on the person while the lead is idle"
        );
        assert_eq!(
            task_status(&task, None, &[Done, Failed]),
            TaskStatus::NeedsYou
        );
        assert_eq!(
            task_status(&task, Some(ChatIndicator::Working), &[Failed]),
            TaskStatus::Working,
            "the lead is handling the failed step"
        );
        task.plan_pending = true;
        assert_eq!(
            task_status(&task, Some(ChatIndicator::Working), &[Planned]),
            TaskStatus::Working,
            "the lead is still writing the plan"
        );
        assert_eq!(task_status(&task, lead, &[Planned]), TaskStatus::NeedsYou);
        assert_eq!(
            task_status(&task, Some(ChatIndicator::Working), &[]),
            TaskStatus::Working
        );
        assert_eq!(
            task_status(&task, None, &[]),
            TaskStatus::ToDo,
            "no plan yet means nothing to approve"
        );
    }

    fn step(chat_id: Option<&str>, outcome: Option<StepOutcome>) -> TaskStep {
        let mut step: TaskStep =
            serde_json::from_value(serde_json::json!({ "id": "s", "taskId": "t" })).unwrap();
        step.chat_id = chat_id.map(str::to_string);
        step.outcome = outcome;
        step
    }

    fn child(status: Option<DelegationStatus>) -> Chat {
        let mut chat: Chat = serde_json::from_value(serde_json::json!({
            "id": "c", "deviceId": "dev", "archived": false, "createdAt": "2026-10-18T00:00:00Z"
        }))
        .unwrap();
        chat.delegation = status.map(|status| Delegation {
            delegated_by_chat_id: "lead".into(),
            depth: 1,
            anchor: "a".into(),
            client_request_id: None,
            owner_turn_started_at: None,
            wake: DelegationWake::Always,
            status,
            cancel_requested: false,
            delivery: None,
            summary: None,
            started_at: chat.created_at,
            completed_at: None,
            finding_outcomes: Default::default(),
            findings_handed: Default::default(),
            findings_announced: Default::default(),
            route: None,
        });
        chat
    }

    #[test]
    fn a_step_reads_its_outcome_then_its_thread() {
        let running = child(Some(DelegationStatus::Running));
        let working = Some(ChatIndicator::Working);
        for (outcome, status) in [
            (StepOutcome::Done, StepStatus::Done),
            (StepOutcome::Skipped, StepStatus::Skipped),
        ] {
            assert_eq!(
                step_status(&step(Some("c"), Some(outcome)), Some(&running), working),
                status,
                "an outcome wins over a running thread"
            );
        }
        assert_eq!(
            step_status(&step(None, None), None, None),
            StepStatus::Planned
        );
        assert_eq!(
            step_status(&step(Some("c"), None), None, None),
            StepStatus::Planned,
            "a deleted thread reads unstarted"
        );
        let done = child(Some(DelegationStatus::Completed));
        assert_eq!(
            step_status(&step(Some("c"), None), Some(&done), working),
            StepStatus::Working,
            "a finished child taking another turn works"
        );
        assert_eq!(
            step_status(
                &step(Some("c"), None),
                Some(&running),
                Some(ChatIndicator::AwaitingInput)
            ),
            StepStatus::NeedsYou
        );
        let cases = [
            (DelegationStatus::Running, StepStatus::Working),
            (DelegationStatus::Completed, StepStatus::Done),
            (DelegationStatus::Failed, StepStatus::NeedsYou),
            (DelegationStatus::Cancelled, StepStatus::NeedsYou),
            (DelegationStatus::Interrupted, StepStatus::NeedsYou),
        ];
        for (delegation, status) in cases {
            let mut bound = step(Some("c"), None);
            bound.turn = Some(StepTurn {
                ended: StepTurnEnd::Failed,
                turn: None,
            });
            assert_eq!(
                step_status(
                    &bound,
                    Some(&child(Some(delegation))),
                    Some(ChatIndicator::Idle)
                ),
                status,
                "a delegated child reads its delegation: {delegation:?}"
            );
        }
        let thread = child(None);
        let ended = [
            (None, StepStatus::Working),
            (Some(StepTurnEnd::Completed), StepStatus::Done),
            (Some(StepTurnEnd::Failed), StepStatus::Failed),
            (Some(StepTurnEnd::Stopped), StepStatus::NeedsYou),
        ];
        for (end, status) in ended {
            let mut bound = step(Some("c"), None);
            bound.turn = end.map(|ended| StepTurn {
                ended,
                turn: Some("turn-1".into()),
            });
            assert_eq!(
                step_status(&bound, Some(&thread), Some(ChatIndicator::Idle)),
                status,
                "{end:?}"
            );
            assert_eq!(
                step_status(&bound, Some(&thread), Some(ChatIndicator::Working)),
                StepStatus::Working,
                "a thread taking another turn works: {end:?}"
            );
        }
    }

    #[test]
    fn thread_link_owns_only_delegated_and_step_bound_children() {
        let task = |id: &str, lead: Option<&str>, mode: TaskMode| {
            let mut task: Task = serde_json::from_value(serde_json::json!({ "id": id })).unwrap();
            task.chat_id = lead.map(str::to_string);
            task.mode = mode;
            task
        };
        let plan = task("t", Some("lead"), TaskMode::Orchestrated);
        let other = task("o", Some("elsewhere"), TaskMode::Orchestrated);
        let mut bound = step(Some("c"), None);
        bound.id = "s1".into();
        let mut foreign = step(Some("c"), None);
        foreign.task_id = "o".into();
        let tasks = [plan.clone(), other];
        let link = |chat: &Chat, steps: &[TaskStep]| thread_link(chat, &tasks, steps);
        let delegated = child(Some(DelegationStatus::Running));
        assert_eq!(
            link(&delegated, &[]),
            Some(ThreadLink {
                parent: "lead".into(),
                kind: LinkKind::Delegated,
                step_id: None
            })
        );
        let mut opened = child(None);
        opened.created_by_chat_id = Some("lead".into());
        assert_eq!(
            link(&opened, &[foreign.clone()]).map(|l| (l.parent, l.kind)),
            Some(("lead".into(), LinkKind::OpenedFrom)),
            "a step of a plan another chat leads does not make the creator own it"
        );
        assert_eq!(
            link(&opened, std::slice::from_ref(&bound)),
            Some(ThreadLink {
                parent: "lead".into(),
                kind: LinkKind::Delegated,
                step_id: Some("s1".into())
            }),
            "a step thread is owned through its step"
        );
        let mut done = bound.clone();
        done.id = "s0".into();
        done.outcome = Some(StepOutcome::Done);
        assert_eq!(
            link(&opened, &[done, bound.clone()]).and_then(|l| l.step_id),
            Some("s1".into()),
            "a thread bound again after a done step fills the open one"
        );
        let mut gone = bound.clone();
        gone.deleted = true;
        assert_eq!(
            link(&opened, &[gone]).map(|l| l.kind),
            Some(LinkKind::OpenedFrom)
        );
        let mut fork = opened.clone();
        fork.forked_from_chat_id = Some("source".into());
        assert_eq!(
            link(&fork, std::slice::from_ref(&bound)),
            Some(ThreadLink {
                parent: "source".into(),
                kind: LinkKind::ForkedFrom,
                step_id: None
            }),
            "a fork is a label even where its row says created_by"
        );
        let mut side = child(None);
        side.sidechat_source_chat_id = Some("lead".into());
        assert_eq!(link(&side, &[]).map(|l| l.kind), Some(LinkKind::SidechatOf));
        assert_eq!(link(&child(None), &[]), None);
        let mut supervisor = opened.clone();
        supervisor.id = format!("{}dev", crate::supervisor::CHAT_PREFIX);
        assert_eq!(link(&supervisor, &[]), None);

        let single = task("h", Some("lead"), TaskMode::Single);
        assert!(plan.has_plan() && !single.has_plan());
        assert!(!task("u", None, TaskMode::Orchestrated).has_plan());
        let mut done = plan.clone();
        done.completed_at = Some(Utc::now());
        assert_eq!(plan_of_chat(&[single.clone(), done], "lead"), None);
        assert_eq!(plan_of_chat(&[single, plan.clone()], "lead"), Some(&plan));
    }

    #[test]
    fn plan_progress_leaves_skipped_steps_out_of_the_total() {
        use StepStatus::*;
        assert_eq!(
            plan_progress(&[Done, Skipped, Working, Planned, NeedsYou, Done, Failed]),
            PlanProgress { done: 2, total: 6 }
        );
        assert_eq!(plan_progress(&[]), PlanProgress::default());
    }

    #[test]
    fn task_patch_tells_a_cleared_field_from_an_absent_one() {
        let patch: TaskPatch =
            serde_json::from_value(serde_json::json!({ "dueOn": null, "chatId": "c" })).unwrap();
        assert_eq!(patch.due_on, Some(None));
        assert_eq!(patch.chat_id, Some(Some("c".into())));
        assert_eq!(patch.space_id, None);
        assert_eq!(
            serde_json::to_value(&patch).unwrap(),
            serde_json::json!({ "dueOn": null, "chatId": "c" })
        );
        let patch: TaskPatch = serde_json::from_value(
            serde_json::json!({ "sendError": null, "mode": "orchestrated" }),
        )
        .unwrap();
        assert_eq!(patch.send_error, Some(None));
        assert_eq!(patch.mode, Some(TaskMode::Orchestrated));
        let patch: TaskStepPatch =
            serde_json::from_value(serde_json::json!({ "outcome": null, "chatId": "c" })).unwrap();
        assert_eq!(patch.outcome, Some(None));
        assert_eq!(patch.chat_id, Some(Some("c".into())));
        assert_eq!(patch.title, None);
        assert_eq!(patch.turn, None);
        let patch: TaskStepPatch = serde_json::from_value(
            serde_json::json!({ "turn": { "ended": "completed", "turn": "m1" } }),
        )
        .unwrap();
        assert_eq!(
            patch.turn,
            Some(Some(StepTurn {
                ended: StepTurnEnd::Completed,
                turn: Some("m1".into())
            }))
        );
        let patch: TaskStepPatch =
            serde_json::from_value(serde_json::json!({ "turn": null })).unwrap();
        assert_eq!(patch.turn, Some(None));
        assert_eq!(
            serde_json::to_value(&patch).unwrap(),
            serde_json::json!({ "turn": null })
        );
    }

    #[test]
    fn sidechat_expires_at_24_hours_from_creation_not_last_activity() {
        let mut chat: Chat = serde_json::from_value(serde_json::json!({
            "id": "side", "deviceId": "dev", "archived": false,
            "createdAt": "2026-09-18T00:00:00Z", "sidechatSourceChatId": "parent"
        }))
        .unwrap();
        let deadline = chat.created_at + chrono::Duration::hours(24);
        assert!(!chat.sidechat_expired(chat.created_at - chrono::Duration::seconds(1)));
        assert!(!chat.sidechat_expired(deadline - chrono::Duration::milliseconds(1)));
        assert!(chat.sidechat_expired(deadline));
        chat.last_message_at = Some(deadline + chrono::Duration::minutes(1));
        assert!(chat.sidechat_expired(deadline + chrono::Duration::minutes(1)));
        chat.sidechat_source_chat_id = None;
        assert!(!chat.sidechat_expired(deadline + chrono::Duration::days(100)));
    }

    #[test]
    fn outside_workspace_reads_round_trip_without_breaking_older_reasons() {
        let reason = WorkspaceReadOnlyReason::OutsideWorkspace;
        assert_eq!(serde_json::to_value(reason).unwrap(), "outsideWorkspace");
        assert_eq!(
            serde_json::from_value::<WorkspaceReadOnlyReason>(serde_json::json!("symlink"))
                .unwrap(),
            WorkspaceReadOnlyReason::Symlink,
            "older peers keep parsing every reason they knew"
        );
    }

    #[test]
    fn legacy_chat_connectivity_has_no_live_delivery_proof() {
        let chat: ChatConnectivity = serde_json::from_value(serde_json::json!({
            "chatId": "remote",
            "connected": true,
            "syncState": "synced"
        }))
        .unwrap();
        assert!(!chat.delivery_live);
        let mut live = chat;
        live.delivery_live = true;
        assert_eq!(serde_json::to_value(live).unwrap()["deliveryLive"], true);
    }

    #[test]
    fn checkout_change_request_status_round_trips_all_states_as_camel_case() {
        for (state, encoded_state) in [
            (ChangeRequestState::Open, "open"),
            (ChangeRequestState::Closed, "closed"),
            (ChangeRequestState::Merged, "merged"),
        ] {
            let status = CheckoutChangeRequestStatus {
                checkout_id: "checkout-1".into(),
                device_id: "device-1".into(),
                cwd: "/repo".into(),
                branch: "feature/change".into(),
                change_request: Some(ChangeRequestSummary {
                    provider: "github".into(),
                    number: 90,
                    title: "Model checkout change request status".into(),
                    url: "https://github.com/acme/zeron/pull/90".into(),
                    state,
                    base_ref: "main".into(),
                    head_ref: "feature/change".into(),
                }),
                updated_at: Utc.with_ymd_and_hms(2026, 8, 15, 12, 30, 0).unwrap(),
            };

            let value = serde_json::to_value(&status).unwrap();
            assert_eq!(value["checkoutId"], "checkout-1");
            assert_eq!(value["deviceId"], "device-1");
            assert_eq!(value["changeRequest"]["state"], encoded_state);
            assert_eq!(value["changeRequest"]["baseRef"], "main");
            assert_eq!(value["changeRequest"]["headRef"], "feature/change");
            assert_eq!(
                serde_json::from_value::<CheckoutChangeRequestStatus>(value).unwrap(),
                status
            );
        }
    }

    #[test]
    fn change_request_list_item_round_trips_all_states_as_camel_case() {
        for (state, encoded_state) in [
            (ChangeRequestState::Open, "open"),
            (ChangeRequestState::Closed, "closed"),
            (ChangeRequestState::Merged, "merged"),
        ] {
            let item = ChangeRequestListItem {
                provider: "github".into(),
                author: Default::default(),
                head_ref_oid: String::new(),
                ci: Default::default(),
                viewer_did_author: None,
                viewer_review_requested: None,
                repository: "private-owner/private-repo".into(),
                number: 123,
                title: "Add pull request dashboard".into(),
                url: "https://github.com/private-owner/private-repo/pull/123".into(),
                state,
                is_draft: true,
                review_decision: ChangeRequestReviewDecision::ChangesRequested,
                additions: 42,
                deletions: 7,
                mergeability: ChangeRequestMergeability::Conflicting,
                created_at: Utc.with_ymd_and_hms(2026, 8, 10, 9, 30, 0).unwrap(),
                updated_at: Utc.with_ymd_and_hms(2026, 8, 19, 12, 0, 0).unwrap(),
            };

            let value = serde_json::to_value(&item).unwrap();
            assert_eq!(value["repository"], "private-owner/private-repo");
            assert_eq!(value["state"], encoded_state);
            assert_eq!(value["isDraft"], true);
            assert_eq!(value["reviewDecision"], "changesRequested");
            assert_eq!(value["additions"], 42);
            assert_eq!(value["deletions"], 7);
            assert_eq!(value["mergeability"], "conflicting");
            assert_eq!(value["createdAt"], "2026-08-10T09:30:00Z");
            assert_eq!(value["updatedAt"], "2026-08-19T12:00:00Z");
            assert!(value.get("baseRef").is_none());
            assert!(value.get("headRef").is_none());
            assert_eq!(
                serde_json::from_value::<ChangeRequestListItem>(value).unwrap(),
                item
            );
        }
    }

    #[test]
    fn change_request_mergeability_round_trips_all_states() {
        for (state, encoded) in [
            (ChangeRequestMergeability::Mergeable, "mergeable"),
            (ChangeRequestMergeability::Conflicting, "conflicting"),
            (ChangeRequestMergeability::Unknown, "unknown"),
        ] {
            let value = serde_json::to_value(state).unwrap();
            assert_eq!(value, encoded);
            assert_eq!(
                serde_json::from_value::<ChangeRequestMergeability>(value).unwrap(),
                state
            );
        }
    }

    #[test]
    fn change_request_review_decision_round_trips_and_defaults_when_absent() {
        for (decision, encoded) in [
            (ChangeRequestReviewDecision::Approved, "approved"),
            (
                ChangeRequestReviewDecision::ChangesRequested,
                "changesRequested",
            ),
            (
                ChangeRequestReviewDecision::ReviewRequired,
                "reviewRequired",
            ),
            (ChangeRequestReviewDecision::Unknown, "unknown"),
        ] {
            let value = serde_json::to_value(decision).unwrap();
            assert_eq!(value, encoded);
            assert_eq!(
                serde_json::from_value::<ChangeRequestReviewDecision>(value).unwrap(),
                decision
            );
        }

        let mut value = serde_json::to_value(ChangeRequestListItem {
            provider: "github".into(),
            author: Default::default(),
            head_ref_oid: String::new(),
            ci: Default::default(),
            viewer_did_author: None,
            viewer_review_requested: None,
            repository: "acme/zeron".into(),
            number: 1,
            title: "No decision".into(),
            url: "https://github.com/acme/zeron/pull/1".into(),
            state: ChangeRequestState::Open,
            is_draft: false,
            review_decision: ChangeRequestReviewDecision::Approved,
            additions: 1,
            deletions: 0,
            mergeability: ChangeRequestMergeability::Mergeable,
            created_at: Utc.with_ymd_and_hms(2026, 8, 10, 9, 30, 0).unwrap(),
            updated_at: Utc.with_ymd_and_hms(2026, 8, 19, 12, 0, 0).unwrap(),
        })
        .unwrap();
        value.as_object_mut().unwrap().remove("reviewDecision");
        assert_eq!(
            serde_json::from_value::<ChangeRequestListItem>(value)
                .unwrap()
                .review_decision,
            ChangeRequestReviewDecision::Unknown
        );
    }

    #[test]
    fn checkout_file_diff_text_contract_is_camel_case() {
        let request = GetCheckoutFileDiffTextRequest {
            checkout_id: "checkout".into(),
            cwd: "/repo".into(),
            path: "src/lib.rs".into(),
            mode: "branch".into(),
            base_ref: Some("main".into()),
            chat_id: None,
            commit_sha: Some("deadbeef".into()),
            diff_checksum: "abc".into(),
        };
        let value = serde_json::to_value(&request).unwrap();
        assert_eq!(value["checkoutId"], "checkout");
        assert_eq!(value["diffChecksum"], "abc");
        assert_eq!(value["commitSha"], "deadbeef");
        assert_eq!(
            serde_json::from_value::<GetCheckoutFileDiffTextRequest>(value).unwrap(),
            request
        );
    }

    #[test]
    fn create_worktree_outcome_accepts_legacy_reply_and_stays_flattened() {
        let legacy = serde_json::json!({
            "repoPath": "/repo",
            "path": "/worktree",
            "branch": "zeron/branch",
            "name": "branch",
            "checkoutId": "checkout",
        });
        let outcome: CreateWorktreeOutcome = serde_json::from_value(legacy.clone()).unwrap();
        assert_eq!(outcome.worktree.path, "/worktree");
        assert!(outcome.setup_action.is_none());
        assert!(outcome.setup_error.is_none());

        let encoded = serde_json::to_value(outcome).unwrap();
        assert_eq!(encoded["path"], legacy["path"]);
        assert!(encoded.get("worktree").is_none());
        assert!(encoded.get("setupAction").is_none());
        assert!(encoded.get("setupError").is_none());
        assert!(serde_json::from_value::<Worktree>(encoded).is_ok());
    }

    #[test]
    fn workspace_file_requests_flatten_target_and_omit_options() {
        let request = ListWorkspaceDirectoryRequest {
            target: WorkspaceTarget {
                chat_id: Some("chat-1".into()),
                space_id: None,
                checkout_path: None,
            },
            directory: "src/日本語".into(),
            include_ignored: false,
            cursor: None,
        };
        let value = serde_json::to_value(&request).unwrap();
        assert_eq!(
            value,
            serde_json::json!({
                "chatId": "chat-1",
                "directory": "src/日本語",
                "includeIgnored": false,
            })
        );
        assert_eq!(
            serde_json::from_value::<ListWorkspaceDirectoryRequest>(value).unwrap(),
            request
        );

        let requests = [
            serde_json::to_value(SearchWorkspaceFilesRequest {
                target: request.target.clone(),
                query: "main".into(),
                include_ignored: false,
                limit: None,
            })
            .unwrap(),
            serde_json::to_value(ReadWorkspaceFileRequest {
                target: request.target.clone(),
                path: "src/main.rs".into(),
            })
            .unwrap(),
            serde_json::to_value(WatchWorkspaceFilesRequest {
                target: request.target,
            })
            .unwrap(),
        ];
        assert!(requests.iter().all(|value| value["chatId"] == "chat-1"));
    }

    #[test]
    fn warm_workspace_search_flattens_its_target() {
        // Early clients also sent a `pin` flag; it is ignored now.
        let request: WarmWorkspaceSearchRequest =
            serde_json::from_value(serde_json::json!({ "chatId": "chat-1", "pin": true })).unwrap();
        assert_eq!(request.target.chat_id.as_deref(), Some("chat-1"));
        assert_eq!(
            serde_json::to_value(WarmWorkspaceSearchResult {
                state: WorkspaceSearchIndexState::Building,
            })
            .unwrap(),
            serde_json::json!({ "state": "building" })
        );
    }

    #[test]
    fn workspace_write_outcomes_have_stable_tags() {
        let written = WriteWorkspaceFileOutcome::Written {
            file: WorkspaceFileWriteResult {
                path: "src/emoji-🛰️.rs".into(),
                content_hash: "hash-2".into(),
                size: 12,
                modified_at: None,
            },
        };
        assert_eq!(
            serde_json::to_value(&written).unwrap(),
            serde_json::json!({
                "status": "written",
                "file": {
                    "path": "src/emoji-🛰️.rs",
                    "contentHash": "hash-2",
                    "size": 12,
                }
            })
        );
        assert_eq!(
            serde_json::from_value::<WriteWorkspaceFileOutcome>(
                serde_json::to_value(&written).unwrap()
            )
            .unwrap(),
            written
        );

        let conflict = WriteWorkspaceFileOutcome::Conflict {
            reason: WorkspaceFileConflictReason::Changed,
            current_content_hash: Some("hash-3".into()),
            current_modified_at: None,
        };
        assert_eq!(
            serde_json::to_value(&conflict).unwrap(),
            serde_json::json!({
                "status": "conflict",
                "reason": "changed",
                "currentContentHash": "hash-3",
            })
        );
    }

    #[test]
    fn workspace_file_change_contract_is_camel_case() {
        let changes = WorkspaceFileChanges {
            sequence: 4,
            resync_required: false,
            changes: vec![WorkspaceFileChange {
                operation_id: None,
                kind: WorkspaceFileChangeKind::Renamed,
                path: "src/new.rs".into(),
                old_path: Some("src/old.rs".into()),
            }],
        };
        let value = serde_json::to_value(&changes).unwrap();
        assert_eq!(value["resyncRequired"], false);
        assert_eq!(value["changes"][0]["kind"], "renamed");
        assert_eq!(value["changes"][0]["oldPath"], "src/old.rs");
        assert_eq!(
            serde_json::from_value::<WorkspaceFileChanges>(value).unwrap(),
            changes
        );
    }
}

/// Host capabilities; absent fields keep older peers read-only for mutations.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceMutationCapabilities {
    #[serde(default)]
    pub move_entry: bool,
    #[serde(default)]
    pub delete_entry: bool,
}

/// Rename and move share the same no-replacement operation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MoveWorkspaceEntryRequest {
    #[serde(flatten)]
    pub target: WorkspaceTarget,
    pub operation_id: String,
    pub expected_checkout_id: String,
    pub source_path: String,
    pub destination_path: String,
    pub expected_source_revision: String,
    pub expected_kind: WorkspaceEntryKind,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeleteWorkspaceEntryRequest {
    #[serde(flatten)]
    pub target: WorkspaceTarget,
    pub operation_id: String,
    pub expected_checkout_id: String,
    pub path: String,
    pub expected_source_revision: String,
    pub expected_kind: WorkspaceEntryKind,
    /// Explicit consent to delete the directory's current contents.
    pub recursive: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "camelCase")]
pub enum WorkspaceMutationOutcome {
    #[serde(rename_all = "camelCase")]
    Applied {
        operation_id: String,
        checkout_id: String,
        change: WorkspaceFileChange,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        entry: Option<WorkspaceEntry>,
    },
    #[serde(rename_all = "camelCase")]
    Rejected {
        operation_id: String,
        reason: WorkspaceMutationRejection,
        message: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum WorkspaceMutationRejection {
    InvalidPath,
    WorkspaceChanged,
    SourceMissing,
    SourceChanged,
    DestinationExists,
    InvalidDestination,
    PermissionDenied,
    Unsupported,
    Busy,
    PartialFailure,
}

#[cfg(test)]
mod mutation_contract_tests {
    use super::*;
    #[test]
    fn old_peers_remain_readable_without_mutation_support() {
        let page: WorkspaceDirectoryPage = serde_json::from_value(serde_json::json!({
            "directory":"", "entries":[{"path":"a", "name":"a", "kind":"file", "ignored":false, "readOnly":false}],
            "truncated":false
        })).unwrap();
        assert_eq!(page.checkout_id, None);
        assert_eq!(page.mutation_capabilities, None);
        assert_eq!(page.entries[0].mutation_revision, None);
        let change: WorkspaceFileChange =
            serde_json::from_value(serde_json::json!({"kind":"removed", "path":"a"})).unwrap();
        assert_eq!(change.operation_id, None);
    }
    #[test]
    fn mutation_requests_and_outcomes_round_trip() {
        let request = serde_json::json!({"chatId":"chat", "operationId":"op", "expectedCheckoutId":"checkout", "sourcePath":"a", "destinationPath":"b", "expectedSourceRevision":"rev", "expectedKind":"file"});
        let decoded: MoveWorkspaceEntryRequest = serde_json::from_value(request.clone()).unwrap();
        let encoded = serde_json::to_value(decoded).unwrap();
        for (key, value) in request.as_object().unwrap() {
            assert_eq!(&encoded[key], value);
        }
        let delete: DeleteWorkspaceEntryRequest = serde_json::from_value(serde_json::json!({"chatId":"chat", "operationId":"op", "expectedCheckoutId":"checkout", "path":"a", "expectedSourceRevision":"rev", "expectedKind":"directory", "recursive":true})).unwrap();
        assert_eq!(
            serde_json::from_value::<DeleteWorkspaceEntryRequest>(
                serde_json::to_value(&delete).unwrap()
            )
            .unwrap(),
            delete
        );
        let outcome = WorkspaceMutationOutcome::Rejected {
            operation_id: "op".into(),
            reason: WorkspaceMutationRejection::DestinationExists,
            message: "Destination exists".into(),
        };
        assert_eq!(
            serde_json::from_value::<WorkspaceMutationOutcome>(
                serde_json::to_value(&outcome).unwrap()
            )
            .unwrap(),
            outcome
        );
        assert!(
            serde_json::from_value::<WorkspaceMutationRejection>(serde_json::json!("overwrite"))
                .is_err()
        );
    }
}
