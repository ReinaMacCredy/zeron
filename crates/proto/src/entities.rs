//! Synced entity rows (workspace doc) and local projections.
//!
//! In zeron these were synced Postgres rows; in zeron they live in the per-org
//! workspace Loro doc (see ARCHITECTURE.md §2.2) with the same field surface.

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
    /// Every harness this chat ran on before the current one, oldest first,
    /// each once (a provider switch appends the harness it left). The sidebar
    /// row and tab stack their marks ahead of the current one.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub previous_harnesses: Vec<HarnessId>,
    /// Unlike `created_by_chat_id`, the delegating chat owns this one (d67).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delegation: Option<Delegation>,
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
}

impl Delegation {
    pub fn is_running(&self) -> bool {
        self.status == DelegationStatus::Running
    }
}

/// When a finished child wakes its owner. `SettledOnly` is a `mode: wait`
/// call: its blocking return carries the result while the owner's turn is
/// live, so only an owner that already settled is woken.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DelegationWake {
    Always,
    SettledOnly,
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
            && now.signed_duration_since(self.created_at) >= chrono::Duration::hours(24)
    }

    /// A session the user sees in chat lists: neither a delegated child nor a
    /// hidden voice orchestrator.
    pub fn is_top_level(&self) -> bool {
        self.delegated_by().is_none() && !crate::voice::is_orchestrator_chat(&self.id)
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
        self.created_by_chat_id.is_none()
            && self.delegation.is_none()
            && !crate::voice::is_orchestrator_chat(&self.id)
    }
}

impl Chat {
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
    /// The run's live background tasks; independent of `status`, which
    /// they never hold Working.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub background_tasks: Vec<BackgroundTask>,
    pub started_at: Option<DateTime<Utc>>,
    pub updated_at: DateTime<Utc>,
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

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ChangeRequestListState {
    #[default]
    Open,
    Closed,
    Merged,
    All,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ChangeRequestInvolvement {
    #[default]
    All,
    Authored,
    Reviewing,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ChangeRequestMergeability {
    Mergeable,
    Conflicting,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ChangeRequestReviewDecision {
    Approved,
    ChangesRequested,
    ReviewRequired,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ChangeRequestChecksState {
    Passing,
    Failing,
    Pending,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChangeRequestActor {
    pub login: String,
    #[serde(default)]
    pub avatar_url: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChangeRequestLabel {
    pub name: String,
    #[serde(default)]
    pub color: Option<String>,
}

/// Kept apart from [`ChangeRequestSummary`], whose wire shape the checkout badge pins.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChangeRequestListItem {
    pub provider: String,
    pub host: String,
    pub repository: String,
    pub number: u64,
    pub title: String,
    pub url: String,
    pub author: Option<ChangeRequestActor>,
    pub head_ref: String,
    pub base_ref: String,
    pub state: ChangeRequestState,
    pub is_draft: bool,
    pub mergeability: ChangeRequestMergeability,
    pub additions: u64,
    pub deletions: u64,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub viewer_review_requested: bool,
    pub labels: Vec<ChangeRequestLabel>,
    #[serde(default)]
    pub review_decision: Option<ChangeRequestReviewDecision>,
    #[serde(default)]
    pub checks_state: Option<ChangeRequestChecksState>,
    /// Stamped by the reader when the row came from another device's engine.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device_id: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ChangeRequestFailure {
    RepositoryUnavailable,
    UnsupportedRepository,
    CliUnavailable,
    Authentication,
    RateLimited,
    Timeout,
    Decode,
    CommandFailed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChangeRequestListError {
    pub host: Option<String>,
    pub repository: Option<String>,
    pub path: String,
    pub failure: ChangeRequestFailure,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ChangeRequestDraftFilter {
    Only,
    Hide,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ChangeRequestReviewFilter {
    Approved,
    ChangesRequested,
    ReviewRequired,
    None,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChangeRequestListFilters {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub draft: Option<ChangeRequestDraftFilter>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub review: Option<ChangeRequestReviewFilter>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checks: Option<ChangeRequestChecksState>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub labels: Vec<Vec<String>>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub excluded_labels: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repository: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListChangeRequestsRequest {
    #[serde(default)]
    pub state: ChangeRequestListState,
    #[serde(default)]
    pub involvement: ChangeRequestInvolvement,
    #[serde(default)]
    pub limit: Option<u32>,
    #[serde(default)]
    pub query: Option<String>,
    #[serde(default)]
    pub filters: ChangeRequestListFilters,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ChangeRequestCheckStatus {
    Pending,
    ActionRequired,
    Success,
    Failure,
    Cancelled,
    Skipped,
    Neutral,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChangeRequestCheck {
    pub name: String,
    pub status: ChangeRequestCheckStatus,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub workflow_name: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ChangeRequestVerdict {
    Approved,
    ChangesRequested,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChangeRequestReviewer {
    pub login: String,
    #[serde(default)]
    pub avatar_url: Option<String>,
    #[serde(default)]
    pub verdict: Option<ChangeRequestVerdict>,
    #[serde(default)]
    pub stale: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ChangeRequestCommentKind {
    Comment,
    Review,
    ReviewComment,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChangeRequestComment {
    pub id: String,
    pub kind: ChangeRequestCommentKind,
    pub author: Option<ChangeRequestActor>,
    pub body: String,
    pub created_at: DateTime<Utc>,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub review_state: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChangeRequestDetail {
    pub item: ChangeRequestListItem,
    pub body: String,
    pub changed_files: u64,
    #[serde(default)]
    pub merged_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub closed_at: Option<DateTime<Utc>>,
    pub is_cross_repository: bool,
    pub reviewers: Vec<ChangeRequestReviewer>,
    pub checks: Vec<ChangeRequestCheck>,
    pub comments: Vec<ChangeRequestComment>,
    pub comment_count: u64,
    pub commit_count: u64,
    #[serde(default)]
    pub commits: Vec<ChangeRequestCommit>,
    #[serde(default)]
    pub auto_merge: Option<ChangeRequestAutoMerge>,
    #[serde(default)]
    pub head_owner: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ChangeRequestMergeMethod {
    Merge,
    Squash,
    Rebase,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChangeRequestAutoMerge {
    #[serde(default)]
    pub method: Option<ChangeRequestMergeMethod>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChangeRequestPermissions {
    pub merge_methods: Vec<ChangeRequestMergeMethod>,
    pub can_write: bool,
    pub can_triage: bool,
    pub can_update: bool,
    pub did_author: bool,
    pub can_update_branch: bool,
    #[serde(default)]
    pub behind_by: Option<u64>,
    #[serde(default)]
    pub default_branch: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum ChangeRequestAction {
    Merge { method: ChangeRequestMergeMethod },
    EnableAutoMerge { method: ChangeRequestMergeMethod },
    DisableAutoMerge,
    Ready,
    Draft,
    Close,
    Reopen,
    UpdateBranch { rebase: bool },
    Revert,
    ApproveWorkflows,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunChangeRequestActionRequest {
    pub host: String,
    pub repository: String,
    pub number: u64,
    pub action: ChangeRequestAction,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub comment: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ChangeRequestReviewEvent {
    Comment,
    Approve,
    RequestChanges,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChangeRequestReviewComment {
    pub path: String,
    pub line: u64,
    pub left_side: bool,
    pub body: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubmitChangeRequestReviewRequest {
    pub host: String,
    pub repository: String,
    pub number: u64,
    pub event: ChangeRequestReviewEvent,
    #[serde(default)]
    pub body: String,
    #[serde(default)]
    pub comments: Vec<ChangeRequestReviewComment>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReplyToChangeRequestThreadRequest {
    pub host: String,
    pub repository: String,
    pub number: u64,
    pub thread_id: String,
    pub body: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetChangeRequestThreadResolvedRequest {
    pub host: String,
    pub repository: String,
    pub number: u64,
    pub thread_id: String,
    pub resolved: bool,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ChangeRequestCheckoutMode {
    #[default]
    None,
    Local,
    Worktree,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PrepareChangeRequestChatRequest {
    pub host: String,
    pub repository: String,
    pub number: u64,
    #[serde(default)]
    pub checkout: ChangeRequestCheckoutMode,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChangeRequestChat {
    #[serde(default)]
    pub space_id: Option<String>,
    #[serde(default)]
    pub root: String,
    pub cwd: String,
    #[serde(default)]
    pub branch: Option<String>,
    pub worktree: bool,
    pub on_head: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateChangeRequestRequest {
    pub host: String,
    pub repository: String,
    pub number: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChangeRequestLabelCandidate {
    pub name: String,
    #[serde(default)]
    pub color: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    pub applied: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChangeRequestLabelCandidates {
    pub labels: Vec<ChangeRequestLabelCandidate>,
    pub truncated: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetChangeRequestLabelRequest {
    pub host: String,
    pub repository: String,
    pub number: u64,
    pub name: String,
    pub add: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChangeRequestReviewerCandidate {
    pub login: String,
    #[serde(default)]
    pub name: Option<String>,
    pub team: bool,
    pub requested: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChangeRequestReviewerCandidates {
    pub reviewers: Vec<ChangeRequestReviewerCandidate>,
    pub truncated: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetChangeRequestReviewerRequest {
    pub host: String,
    pub repository: String,
    pub number: u64,
    pub login: String,
    pub team: bool,
    pub add: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChangeRequestCommit {
    pub oid: String,
    pub headline: String,
    #[serde(default)]
    pub committed_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ChangeRequestReactionContent {
    ThumbsUp,
    ThumbsDown,
    Laugh,
    Hooray,
    Confused,
    Heart,
    Rocket,
    Eyes,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChangeRequestReaction {
    pub content: ChangeRequestReactionContent,
    pub count: u64,
    pub viewer_has_reacted: bool,
    #[serde(default)]
    pub reactors: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChangeRequestActivityComment {
    pub id: String,
    pub kind: ChangeRequestCommentKind,
    pub author: Option<ChangeRequestActor>,
    pub body: String,
    pub created_at: DateTime<Utc>,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub review_state: Option<String>,
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default)]
    pub line: Option<u64>,
    #[serde(default)]
    pub thread_id: Option<String>,
    #[serde(default)]
    pub thread_resolved: bool,
    #[serde(default)]
    pub outdated: bool,
    #[serde(default)]
    pub left_side: bool,
    #[serde(default)]
    pub reactions: Vec<ChangeRequestReaction>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChangeRequestActivityCommit {
    pub oid: String,
    pub headline: String,
    #[serde(default)]
    pub committed_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub additions: Option<u64>,
    #[serde(default)]
    pub deletions: Option<u64>,
    #[serde(default)]
    pub author: Option<ChangeRequestActor>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChangeRequestActivity {
    pub viewer: String,
    pub subject_id: String,
    #[serde(default)]
    pub reactions: Vec<ChangeRequestReaction>,
    pub comments: Vec<ChangeRequestActivityComment>,
    pub comment_count: u64,
    pub truncated: bool,
    pub commits: Vec<ChangeRequestActivityCommit>,
    #[serde(default)]
    pub viewer_can_update: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PostChangeRequestCommentRequest {
    pub host: String,
    pub repository: String,
    pub number: u64,
    pub body: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetChangeRequestReactionRequest {
    pub host: String,
    pub repository: String,
    pub number: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subject_id: Option<String>,
    pub content: ChangeRequestReactionContent,
    pub add: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GetChangeRequestDiffRequest {
    pub host: String,
    pub repository: String,
    pub number: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cursor: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub commit: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChangeRequestFileStat {
    pub path: String,
    pub additions: u64,
    pub deletions: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChangeRequestDiffSlice {
    pub patch: String,
    pub truncated: bool,
    #[serde(default)]
    pub next_cursor: Option<String>,
    #[serde(default)]
    pub omitted_file_stats: Vec<ChangeRequestFileStat>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChangeRequestRef {
    pub host: String,
    pub repository: String,
    pub number: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListChangeRequestStatsRequest {
    pub refs: Vec<ChangeRequestRef>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChangeRequestStat {
    #[serde(flatten)]
    pub reference: ChangeRequestRef,
    pub additions: u64,
    pub deletions: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChangeRequestStats {
    pub stats: Vec<ChangeRequestStat>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GetChangeRequestRequest {
    pub host: String,
    pub repository: String,
    pub number: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChangeRequestListing {
    pub viewers: std::collections::BTreeMap<String, String>,
    pub items: Vec<ChangeRequestListItem>,
    pub errors: Vec<ChangeRequestListError>,
    pub truncated: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub repositories: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub projects: Vec<ChangeRequestProject>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChangeRequestProject {
    pub repository: String,
    pub host: String,
    pub space_id: String,
    pub path: String,
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
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentProfilesSnapshot {
    pub profiles: Vec<AgentProfile>,
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
