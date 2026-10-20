//! EngineRpc — the engine-side `RpcService`: sessions + docs + the workspace-doc
//! entity surface.
//!
//! Methods (feature-inventory §2):
//! - `ListHarnesses` → `[HarnessDescriptor]`
//! - `ListModels {harness}` → `[Model]`
//! - `QueueCommand {chatId, command}` → `{commandId}` (durable doc command)
//! - `WatchDocMessages {chatId}` → stream of joined `SessionMessageEntry[]`,
//!   re-emitted on every doc change
//! - `WatchChats` / `WatchDevices` → streams of the workspace doc's entity rows
//! - `WatchSessions` → stream of `Session[]`: this engine's live statuses merged with
//!   remote devices' workspace session rows
//! - `Mutate {op, …}` → `{ok}` — workspace entity mutations (createChat, renameChat,
//!   setChatArchived, deleteChat, renameDevice, markChatSeen)
//! - `EngineInfo` → `{deviceId, workspaceScope}` — this runtime's fixed identity
//!   and data boundary (never forwarded)
//! - `LocalDevice` → `{deviceId}` — legacy engine identity (never forwarded)
//! - AuthRpc (feature-inventory §2): `AuthStatus` (stream), `SignIn`/`SignInHeadless` →
//!   `{url}`, `CompleteSignIn {code}`, `SignOut`, `ListOrgs`, `CreateOrg {name}`,
//!   `SelectOrg {organizationId}`
//! - Repos (§3.5): `ListRepos`, `AddRepo {path}`, `CloneRepo {url}`,
//!   `CreateRepo {name}`, `ListBranches {repoPath}` (default branch first),
//!   `ListFolders {path?}`, `CreateWorktree {repoPath, branch}`, `DeleteWorktree
//!   {repoPath, worktreePath}`; `WatchCheckoutDiffs` → stream of `CheckoutDiff[]`
//! - Workspace files: lazy directory listing, recursive path search, bounded text
//!   reads, hash-guarded writes, and a checkout-scoped filesystem change stream.
//! - Terminals (§3.4): `OpenTerminal {chatId, cols, rows, cwd?}` → `TerminalSession`,
//!   `SubscribeTerminal {terminalId, afterSeq?}` → stream of `TerminalEvent`
//!   (replay then live tail), `WriteTerminal {terminalId, data}`, `ResizeTerminal`,
//!   `CloseTerminal`. M5 is single-user local: per-user owner checks land with
//!   real multi-account auth in M6.
//! - Agent accounts (§3.7): `ListAgentAccounts {forceUsage?}` →
//!   `AgentAccountsSnapshot`, `ActivateAgentAccount`/`ForgetAgentAccount`
//!   `{harness, accountId}` → snapshot, `StartAgentLogin {harness}` →
//!   `{loginId, url, mode}`, `CompleteAgentLogin {loginId, code}` → snapshot,
//!   `PollAgentLogin {loginId}`, `CancelAgentLogin {loginId}`.
//! - Uploads (§3.7): `UploadChunk {uploadId, data, seq?}`,
//!   `UploadCommit {uploadId, fileName}` → `{path}`,
//!   `ReadAttachmentChunk {path, offset}` → `{name, mimeType, data, nextOffset,
//!   done}` (path-jailed to the uploads dir + workspace-known chat cwds).
//!
//! ## Device-addressed routing (`targetDeviceId`, feature-inventory §2.1)
//!
//! ControlRpc methods are relay-forwardable: params may carry `targetDeviceId`. When it
//! names another device, the call is forwarded verbatim over that device's relay DO via
//! the [`LinkCache`] — the remote engine sees its own id and handles locally, so the
//! forward can never loop. Streaming methods are proxied by re-subscribing remotely and
//! piping items. To make another method device-addressable, nothing per-method is needed
//! beyond listing it in [`forwardable`] (and [`is_stream_method`] if it streams);
//! handlers stay transport-agnostic. This includes the workspace file surface,
//! whose checkout always lives on the routed target device.

mod supervisor;
mod tasks;
mod voice_chat;
mod voice_remote;
use async_trait::async_trait;
use base64::Engine as _;
use futures::StreamExt;
use futures::stream::BoxStream;
use serde::Deserialize;
use std::collections::HashSet;
use std::time::Duration;
use tokio::sync::watch;

use zeron_doc::{MessagePart, SessionCommandPayload};
use zeron_proto::{
    AgentEvent, AgentProfileDraft, ChatConfig, CreateWorktreeOutcome, EngineInfo, HarnessId,
    HarnessUpdatePolicy, ProjectActionDraft, Space, ToolCall, WorkspaceScope,
};
use zeron_rpc::{
    LinkCache, RpcError, RpcReply, RpcService, capability_errors, methods, parse_params,
};

use crate::agent_accounts::AgentAccounts;
use crate::agent_profiles::AgentProfilesStore;
use crate::auth::Auth;
use crate::change_requests::CheckoutChangeRequests;
use crate::computer::runtime::ComputerRuntime;
use crate::diff_sync::CheckoutDiffSync;
use crate::doc_host::DocHost;
use crate::project_actions::ProjectActionsStore;
use crate::provider_instances::ProviderInstancesStore;
use crate::registry::HarnessRegistry;
use crate::repos::{Repos, session_home_dir};
use crate::sessions::SessionsEngine;
use crate::source_control::{ChangeRequestError, GitHubCli};
use crate::terminals::Terminals;
use crate::uploads::Uploads;
use crate::workspace_host::WorkspaceHost;

const FILE_SEARCH_RPC_TIMEOUT: Duration = Duration::from_secs(6);
const FILE_SEARCH_FEATURED_PATHS: usize = 32;
/// Under the 30-minute MCP call ceiling the harnesses are configured with, so
/// the card settles here instead of outliving a call the harness abandoned.
const ASK_USER_WAIT: Duration = Duration::from_secs(25 * 60);

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ChatParams {
    chat_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ChatsParams {
    chat_ids: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ListModelsParams {
    harness: HarnessId,
    #[serde(default)]
    force: bool,
    #[serde(default)]
    instance_id: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct BackgroundTaskParams {
    chat_id: String,
    task_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CompactChatParams {
    chat_id: String,
    /// Passed to the harness's own compaction command as its focus argument.
    #[serde(default)]
    focus: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RequestCompactParams {
    chat_id: String,
    #[serde(default)]
    focus: Option<String>,
    #[serde(default)]
    continue_with: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CreateSidechatParams {
    source_chat_id: String,
    chat_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ContinueInNewThreadParams {
    source_chat_id: String,
    chat_id: String,
    #[serde(default)]
    harness: Option<HarnessId>,
    #[serde(default)]
    target_space_id: Option<String>,
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    reasoning: Option<zeron_proto::ReasoningLevel>,
    #[serde(flatten)]
    profile: crate::doc_host::ProfileParams,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CreateThreadParams {
    from_chat_id: String,
    prompt: String,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    harness: Option<HarnessId>,
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    reasoning: Option<zeron_proto::ReasoningLevel>,
    #[serde(flatten)]
    profile: crate::doc_host::ProfileParams,
    #[serde(default)]
    space_id: Option<String>,
    #[serde(default)]
    cwd: Option<String>,
    #[serde(default)]
    task_step_id: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct StartReviewParams {
    owner_chat_id: String,
    scope: String,
    #[serde(default)]
    commit_sha: Option<String>,
    #[serde(default)]
    reviewer: Option<crate::review::ReviewerChoice>,
    #[serde(default)]
    wake: Option<zeron_proto::DelegationWake>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct TaskParams {
    chat_id: String,
    task_id: String,
    #[serde(default)]
    acknowledge: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct HandFindingsParams {
    owner_chat_id: String,
    task_id: String,
    ids: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SendThreadMessageParams {
    chat_id: String,
    from_chat_id: String,
    text: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SetThreadWatchesParams {
    chat_id: String,
    #[serde(default)]
    add: Vec<String>,
    #[serde(default)]
    remove: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AskUserParams {
    chat_id: String,
    run_secret: String,
    questions: Vec<zeron_proto::UserInputQuestion>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ForkChatParams {
    source_chat_id: String,
    entry_id: String,
    chat_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RewindChatParams {
    chat_id: String,
    entry_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SetHarnessEnabledParams {
    harness: HarnessId,
    enabled: bool,
}

async fn update_harness_enabled(
    registry: &HarnessRegistry,
    harness: HarnessId,
    enabled: bool,
) -> Result<(), RpcError> {
    registry
        .set_enabled(harness, enabled)
        .map_err(RpcError::Failed)
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct HarnessUpdateParams {
    #[serde(default)]
    harness: Option<HarnessId>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DismissHarnessUpdateParams {
    harness: HarnessId,
    #[serde(default)]
    version: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct UpdateHarnessesParams {
    harnesses: Vec<HarnessId>,
    #[serde(default)]
    continue_threads: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PrepareRestartParams {
    #[serde(default)]
    continue_threads: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SetLimitResumeParams {
    chat_id: String,
    auto: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SetAutoResumeLimitsParams {
    enabled: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SetHarnessUpdatePolicyParams {
    harness: HarnessId,
    policy: HarnessUpdatePolicy,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct QueueCommandParams {
    chat_id: String,
    command: SessionCommandPayload,
    /// Queued attachments (bytes already committed locally as `pending://`
    /// refs) the engine delivers to a remote host AFTER the command is
    /// durably queued — never as a gate in front of it.
    #[serde(default)]
    transfers: Vec<crate::uploads::AttachmentTransfer>,
    #[serde(default)]
    from_chat_id: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RelayCommandParams {
    chat_id: String,
    /// The full command entry, client-minted id included — the exactly-once
    /// key the host claims in its processed ledger before executing.
    entry: zeron_doc::SessionCommandEntry,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct TakeProjectActionSetupParams {
    chat_id: String,
    command_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct QueueMessageParams {
    chat_id: String,
    text: String,
    #[serde(default)]
    attachments: Vec<String>,
    /// Keep this row visible during the current turn even when the harness
    /// supports mid-turn steering.
    #[serde(default)]
    hold_for_turn_end: bool,
    /// Steer the row into the live turn right away when the harness can take
    /// it; otherwise it stays queued exactly like a plain submit.
    #[serde(default)]
    steer_now: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct QueuedMessageParams {
    chat_id: String,
    id: String,
    /// Present for UpdateQueuedMessage only; empty text deletes the row.
    #[serde(default)]
    text: String,
    /// Present for MoveQueuedMessage only.
    #[serde(default)]
    to_index: usize,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct BeginQueuedMessageEditParams {
    chat_id: String,
    id: String,
    editor_device_id: String,
    editor_instance_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RenewQueuedMessageEditParams {
    chat_id: String,
    id: String,
    lease_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
enum FinishQueuedMessageEditAction {
    Commit,
    Cancel,
    Discard,
    ReleaseUnchanged,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct FinishQueuedMessageEditParams {
    chat_id: String,
    id: String,
    lease_id: String,
    action: FinishQueuedMessageEditAction,
    #[serde(default)]
    text: Option<String>,
    #[serde(default)]
    expected_text_hash: Option<String>,
    #[serde(default)]
    attachments: Option<Vec<String>>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RepoPathParams {
    /// `repoPath` per §3.5 (the §2.1 shorthand `repo` is accepted as an alias).
    #[serde(alias = "repo")]
    repo_path: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CheckoutChangeRequestParams {
    cwd: String,
    #[serde(default)]
    branch: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SwitchRefParams {
    /// The checkout to switch — a session's cwd (main folder or worktree).
    repo_path: String,
    ref_name: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CreateWorktreeParams {
    #[serde(alias = "repo")]
    repo_path: String,
    branch: String,
    #[serde(default)]
    space_id: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DeleteWorktreeParams {
    #[serde(alias = "repo")]
    repo_path: String,
    #[serde(alias = "path")]
    worktree_path: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DiscardWorkingTreeParams {
    chat_id: String,
    checkout_id: String,
    expected_checksum: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ListProjectActionsParams {
    space_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct UpsertProjectActionParams {
    space_id: String,
    #[serde(default)]
    action_id: Option<String>,
    action: ProjectActionDraft,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DeleteProjectActionParams {
    space_id: String,
    action_id: String,
}

#[derive(Debug, Deserialize)]
struct UsageSummaryParams {
    days: u32,
}

async fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> Result<T, crate::EngineError> + Send + 'static,
) -> Result<T, RpcError> {
    tokio::task::spawn_blocking(work)
        .await
        .map_err(|err| RpcError::Failed(err.to_string()))?
        .map_err(|err| RpcError::Failed(err.to_string()))
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct GetToolSettingsParams {
    chat_id: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct UpsertAgentProfileParams {
    #[serde(default)]
    profile_id: Option<String>,
    profile: AgentProfileDraft,
    #[serde(default)]
    by_agent: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DeleteAgentProfileParams {
    profile_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct UpsertProviderInstanceParams {
    instance: zeron_proto::ProviderInstance,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DeleteProviderInstanceParams {
    instance_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ListAcpAuthMethodsParams {
    instance_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SearchAcpRegistryParams {
    #[serde(default)]
    query: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PrepareAcpAgentParams {
    agent_id: String,
    #[serde(default)]
    distribution: Option<zeron_proto::AcpDistribution>,
    #[serde(default)]
    allow_unverified: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SetSystemPromptParams {
    prompt: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ReorderAgentProfilesParams {
    profile_ids: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RunProjectActionParams {
    space_id: String,
    chat_id: String,
    action_id: String,
    cols: u16,
    rows: u16,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ListFoldersParams {
    #[serde(default)]
    path: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct FileSearchParams {
    query: String,
    #[serde(default)]
    chat_id: Option<String>,
    #[serde(default)]
    space_id: Option<String>,
    /// Existing linked worktree selected for a new chat. The engine accepts it
    /// only after verifying it against the space repository's worktree list.
    #[serde(default)]
    path: Option<String>,
}

fn tool_file_path(call: &ToolCall) -> Option<&str> {
    match call {
        ToolCall::ReadFile { path }
        | ToolCall::WriteFile { path, .. }
        | ToolCall::EditFile { path, .. } => Some(path),
        ToolCall::ApplyPatch { path } | ToolCall::Search { path, .. } => path.as_deref(),
        ToolCall::Exec { .. }
        | ToolCall::Glob { .. }
        | ToolCall::WebFetch { .. }
        | ToolCall::WebSearch { .. }
        | ToolCall::Todo { .. }
        | ToolCall::Mcp { .. }
        | ToolCall::Unknown { .. } => None,
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct OpenTerminalParams {
    chat_id: String,
    cols: u16,
    rows: u16,
    /// Explicit working directory (new-chat canvas: the selected project
    /// folder, or `~`). When omitted, the chat row's cwd is used, then the
    /// space named by a `space-canvas:{spaceId}` chat id.
    #[serde(default)]
    cwd: Option<String>,
    #[serde(default)]
    command: Option<String>,
    #[serde(default)]
    agent: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ReadTerminalParams {
    terminal_id: String,
    chat_id: String,
    #[serde(default)]
    lines: Option<usize>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SendTerminalInputParams {
    terminal_id: String,
    chat_id: String,
    #[serde(default)]
    text: Option<String>,
    #[serde(default)]
    keys: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SetTerminalSelectionParams {
    terminal_id: String,
    #[serde(default)]
    text: Option<String>,
}

/// Matches the UI canvas panel key (`AppState::panel_session_key`).
const CANVAS_TERMINAL_PREFIX: &str = "space-canvas:";

/// A cwd the user (or a project-less chat) meant as "host home", not a folder.
fn meaningful_cwd(cwd: Option<String>) -> Option<String> {
    cwd.filter(|cwd| {
        let trimmed = cwd.trim();
        !trimmed.is_empty() && trimmed != "~"
    })
}

/// Space id encoded in a new-chat canvas terminal key, if any.
fn canvas_space_id(chat_id: &str) -> Option<&str> {
    chat_id
        .strip_prefix(CANVAS_TERMINAL_PREFIX)
        .filter(|id| !id.is_empty())
}

/// Resolve the PTY cwd: a real explicit path wins, then the chat row, then
/// the project folder named by `space-canvas:{spaceId}`, then `~`.
/// The portable `~` marker is a fallback, not an override — otherwise a
/// canvas OpenTerminal that still says `~` (spaces watch not landed in the
/// UI) would ignore the selected project encoded in `chatId`.
fn resolve_open_terminal_cwd(
    explicit: Option<String>,
    chat_cwd: Option<String>,
    space_cwd: Option<String>,
    scratch_root: &std::path::Path,
) -> Result<String, String> {
    let raw = meaningful_cwd(explicit)
        .or_else(|| meaningful_cwd(chat_cwd))
        .or_else(|| meaningful_cwd(space_cwd))
        .unwrap_or_else(|| "~".to_string());
    crate::repos::resolve_chat_cwd(&raw, scratch_root)
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct TerminalIdParams {
    terminal_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SubscribeTerminalParams {
    terminal_id: String,
    #[serde(default)]
    after_seq: Option<u64>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct WriteTerminalParams {
    terminal_id: String,
    /// Base64 input bytes (plain UTF-8 accepted leniently).
    data: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ResizeTerminalParams {
    terminal_id: String,
    cols: u16,
    rows: u16,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ListAgentAccountsParams {
    #[serde(default)]
    force_usage: Option<bool>,
    #[serde(default)]
    instance_id: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AgentAccountParams {
    harness: HarnessId,
    account_id: String,
    #[serde(default)]
    instance_id: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct StartAgentLoginParams {
    harness: HarnessId,
    /// Stamped by the requesting engine when it forwards the start: the
    /// device whose browser finishes the sign-in. The login's callback port
    /// is served over P2P to that device alone.
    #[serde(default)]
    requester_device_id: Option<String>,
    /// For agents that keep a login per model provider (OpenCode, Pi,
    /// Hermes): which provider to sign in to; `None` = the agent's default.
    #[serde(default)]
    provider: Option<String>,
    #[serde(default)]
    instance_id: Option<String>,
    /// An ACP instance's `authMethods` id; `None` = the agent's default.
    #[serde(default)]
    method_id: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct LoginIdParams {
    login_id: String,
    #[serde(default)]
    instance_id: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CompleteAgentLoginParams {
    login_id: String,
    code: String,
    #[serde(default)]
    instance_id: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct UploadChunkParams {
    upload_id: String,
    /// Base64 payload chunk.
    data: String,
    #[serde(default)]
    seq: Option<u64>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct UploadCommitParams {
    upload_id: String,
    file_name: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ReadAttachmentChunkParams {
    path: String,
    #[serde(default)]
    offset: u64,
}

const _: () = assert!(zeron_proto::VISUAL_MAX_BYTES == zeron_visual::MAX_SCRIPT_BYTES);
const _: () = assert!(zeron_proto::VISUAL_MAX_ASSETS == zeron_visual::assets::MAX_ASSETS);
const _: () = assert!(zeron_proto::VISUAL_MAX_ASSET_BYTES == zeron_visual::assets::MAX_ASSET_BYTES);
const _: () = assert!(
    zeron_proto::VISUAL_MAX_TOTAL_ASSET_BYTES == zeron_visual::assets::MAX_TOTAL_ASSET_BYTES
);

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PublishVisualParams {
    chat_id: String,
    title: String,
    script: String,
    /// Files the script uses, base64 by name.
    #[serde(default)]
    assets: std::collections::BTreeMap<String, String>,
}

fn no_turn_for_visual() -> RpcError {
    RpcError::Failed(
        "This chat has no running turn on this device; show_visual only works from inside a turn."
            .into(),
    )
}

/// Decode and bound a visual's assets.
fn decode_visual_assets(
    assets: &std::collections::BTreeMap<String, String>,
) -> Result<Vec<(String, Vec<u8>)>, RpcError> {
    use base64::Engine as _;
    use zeron_visual::assets::{MAX_ASSET_BYTES, MAX_ASSETS, MAX_TOTAL_ASSET_BYTES, valid_name};
    if assets.len() > MAX_ASSETS {
        return Err(RpcError::BadParams(format!(
            "a visual takes at most {MAX_ASSETS} assets"
        )));
    }
    let mut total = 0;
    let mut out = Vec::with_capacity(assets.len());
    let mut seen = std::collections::HashSet::new();
    for (name, data) in assets {
        if !valid_name(name) {
            return Err(RpcError::BadParams(format!(
                "asset name '{name}' must be a plain file name"
            )));
        }
        // Stored as files, and file systems often ignore case.
        if !seen.insert(name.to_ascii_lowercase()) {
            return Err(RpcError::BadParams(format!(
                "asset names must differ by more than case ('{name}')"
            )));
        }
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(data)
            .map_err(|e| RpcError::BadParams(format!("asset '{name}' is not base64: {e}")))?;
        total += bytes.len();
        if bytes.len() > MAX_ASSET_BYTES || total > MAX_TOTAL_ASSET_BYTES {
            return Err(RpcError::BadParams(format!(
                "asset '{name}' is too large ({} MB each, {} MB together)",
                MAX_ASSET_BYTES / (1024 * 1024),
                MAX_TOTAL_ASSET_BYTES / (1024 * 1024)
            )));
        }
        out.push((name.clone(), bytes));
    }
    Ok(out)
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct FetchToolBlobParams {
    /// Doc-resident sidecar ref (`{chatId}/{partId}` or `…​.diff`).
    blob_ref: String,
}

/// A host-written transcript boundary riding on a config mutation: the
/// divider label plus the hint shown behind its info glyph.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SystemNote {
    text: String,
    #[serde(default)]
    hint: Option<String>,
}

/// The Mutate surface (feature-inventory §2 DataRpc), tagged by `op`.
#[derive(Debug, Deserialize)]
#[serde(tag = "op", rename_all = "camelCase")]
enum MutateParams {
    #[serde(rename_all = "camelCase")]
    CreateChat {
        chat_id: String,
        /// The project the chat is created in — fixes host device + base cwd.
        /// `None` mints a project-less chat: `deviceId` picks the host and the
        /// cwd defaults to `~` (expanded on the host at run time).
        #[serde(default)]
        space_id: Option<String>,
        /// Host device for a project-less chat; ignored when `spaceId` is set.
        #[serde(default)]
        device_id: Option<String>,
        #[serde(default)]
        config: Option<ChatConfig>,
        /// The picked ref, named on the row from the first frame (the footer
        /// read "Select ref" until the diff reconciler stamped it).
        #[serde(default)]
        branch: Option<String>,
        /// Cwd override (isolated-worktree path); default = the space's folder.
        #[serde(default)]
        cwd: Option<String>,
        /// The chat whose agent is opening this one (thread tools); sidebar
        /// grouping only.
        #[serde(default)]
        created_by_chat_id: Option<String>,
    },
    /// Create a space (device + folder pair). Idempotent by id; a live
    /// duplicate `(deviceId, path)` no-ops. `gitDetected` is seeded from the
    /// picker's FolderEntry — the owning device's SpacesSync re-verifies.
    #[serde(rename_all = "camelCase")]
    CreateSpace {
        space_id: String,
        device_id: String,
        path: String,
        #[serde(default)]
        name: Option<String>,
        #[serde(default)]
        git_detected: bool,
    },
    /// LWW display-name set; `name: None` clears back to basename(path).
    #[serde(rename_all = "camelCase")]
    RenameSpace {
        space_id: String,
        #[serde(default)]
        name: Option<String>,
    },
    /// Hard delete: cascades to every chat (and session row) in the space.
    /// Live runs hosted here are interrupted best-effort.
    #[serde(rename_all = "camelCase")]
    DeleteSpace { space_id: String },
    #[serde(rename_all = "camelCase")]
    RenameChat { chat_id: String, title: String },
    /// Set the chat's checkout branch label — the sidebar's
    /// "project · branch" sub-line.
    #[serde(rename_all = "camelCase")]
    SetChatBranch { chat_id: String, branch: String },
    /// Retarget a chat onto another folder — mid-session switch to an
    /// EXISTING worktree (the picked ref's checkout). Next run starts a
    /// fresh harness conversation there (resume is cwd-scoped).
    #[serde(rename_all = "camelCase")]
    SetChatCwd { chat_id: String, cwd: String },
    /// Backdate a chat's activity timestamps (epoch ms) — the sidebar's
    /// relative-time column. Used by tooling/seeds; the doc fold sets these on
    /// real message traffic.
    #[serde(rename_all = "camelCase")]
    SetChatActivity {
        chat_id: String,
        #[serde(default)]
        last_message_at: Option<i64>,
        #[serde(default)]
        created_at: Option<i64>,
    },
    /// Re-home a chat to another device (tooling/seeds; device migration later).
    #[serde(rename_all = "camelCase")]
    SetChatHost { chat_id: String, device_id: String },
    #[serde(rename_all = "camelCase")]
    SetChatArchived { chat_id: String, archived: bool },
    #[serde(rename_all = "camelCase")]
    SetChatSettled { chat_id: String, settled: bool },
    /// Change one pin without replacing another device's edits.
    #[serde(rename_all = "camelCase")]
    ChangeSidebarPin {
        change: zeron_proto::SidebarPinChange,
    },
    /// Full-config replace on the chat row (zeron `SetChatConfig`): the
    /// composer's mid-session model / reasoning / options changes, LWW-synced
    /// so they survive restarts and reach every device. `note` marks the
    /// change in the transcript as a host-written `system` entry (the
    /// model-switch divider); only the chat's host device writes it — a
    /// viewer's copy of the doc is not the writer's lineage.
    #[serde(rename_all = "camelCase")]
    SetChatConfig {
        chat_id: String,
        config: ChatConfig,
        #[serde(default)]
        note: Option<SystemNote>,
    },
    /// Tombstone: removes the chats-map row; the session doc remains.
    #[serde(rename_all = "camelCase")]
    DeleteChat { chat_id: String },
    #[serde(rename_all = "camelCase")]
    RenameDevice { device_id: String, name: String },
    /// Synced seen marker (LWW + monotonic guard): clears the "completed"
    /// badge on every device. `at` is epoch ms; default = now.
    #[serde(rename_all = "camelCase")]
    MarkChatSeen {
        chat_id: String,
        #[serde(default)]
        at: Option<i64>,
    },
    #[serde(rename_all = "camelCase")]
    CreateTask {
        task_id: String,
        title: String,
        #[serde(default)]
        notes: String,
        #[serde(default)]
        space_id: Option<String>,
        #[serde(default)]
        due_on: Option<String>,
        #[serde(default)]
        order_key: Option<String>,
        #[serde(default)]
        images: Vec<String>,
        #[serde(default)]
        context: Vec<String>,
    },
    #[serde(rename_all = "camelCase")]
    UpdateTask {
        task_id: String,
        patch: zeron_proto::TaskPatch,
        #[serde(default)]
        from_chat_id: Option<String>,
    },
    #[serde(rename_all = "camelCase")]
    DeleteTask {
        task_id: String,
        #[serde(default)]
        from_chat_id: Option<String>,
    },
    #[serde(rename_all = "camelCase")]
    StartTask {
        task_id: String,
        chat_id: String,
        #[serde(default)]
        space_id: Option<String>,
        #[serde(default)]
        config: Option<ChatConfig>,
        #[serde(default)]
        orchestrate: bool,
        #[serde(default = "approve_plan_default")]
        approve_plan: bool,
        #[serde(default)]
        same_agents: bool,
        #[serde(default)]
        plan_doc: bool,
    },
    #[serde(rename_all = "camelCase")]
    AssignTask {
        task_id: String,
        chat_id: String,
        #[serde(default)]
        send: bool,
        #[serde(default)]
        from_chat_id: Option<String>,
        #[serde(default)]
        orchestrate: bool,
        #[serde(default = "approve_plan_default")]
        approve_plan: bool,
        #[serde(default)]
        same_agents: bool,
        #[serde(default)]
        plan_doc: bool,
        #[serde(default)]
        attach_plan: bool,
    },
    #[serde(rename_all = "camelCase")]
    PlanSteps {
        task_id: String,
        steps: Vec<PlannedStep>,
        #[serde(default)]
        overview: Option<String>,
        #[serde(default)]
        resources: Option<Vec<String>>,
        #[serde(default)]
        from_chat_id: Option<String>,
    },
    #[serde(rename_all = "camelCase")]
    CreateTaskStep {
        task_id: String,
        step_id: String,
        title: String,
        #[serde(default)]
        notes: String,
        #[serde(default)]
        order_key: Option<String>,
    },
    #[serde(rename_all = "camelCase")]
    UpdateTaskStep {
        step_id: String,
        patch: zeron_proto::TaskStepPatch,
        #[serde(default)]
        from_chat_id: Option<String>,
    },
    #[serde(rename_all = "camelCase")]
    DeleteTaskStep { step_id: String },
    #[serde(rename_all = "camelCase")]
    ApprovePlan { task_id: String },
    #[serde(rename_all = "camelCase")]
    TakeBackPlanApproval { task_id: String },
    #[serde(rename_all = "camelCase")]
    RetryTaskStep { step_id: String },
    #[serde(rename_all = "camelCase")]
    EditTaskPlan {
        task_id: String,
        #[serde(default)]
        overview: Option<String>,
        #[serde(default)]
        overview_images: Option<Vec<String>>,
        #[serde(default)]
        steps: Vec<tasks::PlanEdit>,
        #[serde(default)]
        remove_step_ids: Vec<String>,
        #[serde(default)]
        order: Vec<String>,
    },
    #[serde(rename_all = "camelCase")]
    SetMissionSupervised { task_id: String, supervised: bool },
    #[serde(rename_all = "camelCase")]
    SetTreeWatched { root_chat_id: String, watched: bool },
    #[serde(rename_all = "camelCase")]
    SetWatchEverything { on: bool },
    #[serde(rename_all = "camelCase")]
    EscalateTask {
        task_id: String,
        #[serde(default)]
        from_chat_id: Option<String>,
        title: String,
        why: zeron_proto::supervisor::ItemWhy,
        #[serde(default)]
        detail: String,
        options: Vec<String>,
    },
    #[serde(rename_all = "camelCase")]
    ResolveSupervisorItem { item_id: String, choice: String },
    #[serde(rename_all = "camelCase")]
    ResolveSupervisorItems { answers: Vec<ItemChoice> },
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ItemChoice {
    item_id: String,
    choice: String,
}

fn approve_plan_default() -> bool {
    true
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PlannedStep {
    title: String,
    #[serde(default)]
    notes: String,
    #[serde(default)]
    thread_id: Option<String>,
}

#[derive(Clone)]
pub struct EngineRpc {
    voice: crate::voice::VoiceManager,
    sessions: SessionsEngine,
    doc_host: DocHost,
    workspace: WorkspaceHost,
    registry: std::sync::Arc<HarnessRegistry>,
    repos: Repos,
    workspace_files: crate::WorkspaceFiles,
    terminals: Terminals,
    project_actions: ProjectActionsStore,
    agent_profiles: Option<AgentProfilesStore>,
    provider_instances: Option<ProviderInstancesStore>,
    previews: Option<zeron_preview::PreviewService>,
    change_requests: CheckoutChangeRequests,
    github: GitHubCli,
    diff_sync: CheckoutDiffSync,
    uploads: Uploads,
    agent_accounts: AgentAccounts,
    auth: Option<Auth>,
    links: Option<std::sync::Arc<LinkCache>>,
    updater: Option<zeron_update::Updater>,
    harness_updates: Option<crate::harness_updates::HarnessUpdateCoordinator>,
    resume_schedule: Option<crate::resume_schedule::ResumeSchedule>,
    local_import: Option<crate::local_import::LocalImporter>,
    computer: Option<std::sync::Arc<ComputerRuntime>>,
    device_hub: Option<std::sync::Arc<crate::device_hub::DeviceHub>>,
    html_preview: std::sync::Arc<crate::html_render::PreviewHost>,
    starting_tasks: std::sync::Arc<std::sync::Mutex<std::collections::HashSet<String>>>,
    engine_info: EngineInfo,
}

impl EngineRpc {
    #[allow(clippy::too_many_arguments)] // engine assembly seam, not a public API
    pub fn new(
        sessions: SessionsEngine,
        doc_host: DocHost,
        workspace: WorkspaceHost,
        registry: std::sync::Arc<HarnessRegistry>,
        repos: Repos,
        workspace_files: crate::WorkspaceFiles,
        terminals: Terminals,
        project_actions: ProjectActionsStore,
        change_requests: CheckoutChangeRequests,
        diff_sync: CheckoutDiffSync,
        uploads: Uploads,
        agent_accounts: AgentAccounts,
        workspace_scope: WorkspaceScope,
    ) -> Self {
        let engine_info = EngineInfo {
            device_id: doc_host.device_id().to_string(),
            workspace_scope,
            cursor_sdk_version: Some(zeron_harness::CursorHarness::sdk_version().into()),
            capabilities: zeron_proto::capabilities::current(),
        };
        let github = GitHubCli::new().with_store(repos.data_dir());
        Self {
            voice: sessions.voice_manager(),
            sessions,
            doc_host,
            workspace,
            registry,
            repos,
            workspace_files,
            terminals,
            project_actions,
            agent_profiles: None,
            provider_instances: None,
            previews: None,
            change_requests,
            github,
            diff_sync,
            uploads,
            agent_accounts,
            auth: None,
            links: None,
            updater: None,
            harness_updates: None,
            resume_schedule: None,
            local_import: None,
            computer: None,
            device_hub: None,
            html_preview: Default::default(),
            starting_tasks: Default::default(),
            engine_info,
        }
    }

    fn delete_chat(&self, chat_id: &str) -> Result<(), RpcError> {
        self.voice.retire_chat(chat_id);
        self.workspace
            .delete_chat(chat_id)
            .map_err(|e| RpcError::Failed(e.to_string()))?;
        self.doc_host.purge_chat(chat_id);
        crate::html_render::remove_chat_pages(&self.uploads, chat_id);
        Ok(())
    }

    pub fn with_agent_profiles(mut self, agent_profiles: AgentProfilesStore) -> Self {
        self.agent_profiles = Some(agent_profiles);
        self
    }

    pub fn with_provider_instances(mut self, provider_instances: ProviderInstancesStore) -> Self {
        self.provider_instances = Some(provider_instances);
        self
    }

    pub fn with_previews(mut self, previews: zeron_preview::PreviewService) -> Self {
        self.previews = Some(previews);
        self
    }

    /// Attach the auth service (AuthStatus + AuthRpc mutations).
    pub fn with_auth(mut self, auth: Auth) -> Self {
        self.auth = Some(auth);
        self
    }

    /// Attach the peer link cache — enables `targetDeviceId` relay forwarding.
    pub fn with_links(mut self, links: std::sync::Arc<LinkCache>) -> Self {
        self.links = Some(links);
        self
    }

    /// Attach the release checker (UpdateStatus stream + ApplyUpdate).
    pub fn with_updater(mut self, updater: zeron_update::Updater) -> Self {
        self.updater = Some(updater);
        self
    }

    pub fn with_harness_updates(
        mut self,
        coordinator: crate::harness_updates::HarnessUpdateCoordinator,
    ) -> Self {
        self.harness_updates = Some(coordinator);
        self
    }

    pub fn with_resume_schedule(
        mut self,
        schedule: crate::resume_schedule::ResumeSchedule,
    ) -> Self {
        self.resume_schedule = Some(schedule);
        self
    }

    /// Attach the local→synced profile importer (synced runtimes only).
    pub fn with_local_import(mut self, importer: crate::local_import::LocalImporter) -> Self {
        self.local_import = Some(importer);
        self
    }

    pub fn with_computer(mut self, computer: std::sync::Arc<ComputerRuntime>) -> Self {
        self.computer = Some(computer);
        self
    }

    pub fn with_device_hub(mut self, hub: std::sync::Arc<crate::device_hub::DeviceHub>) -> Self {
        self.device_hub = Some(hub);
        self
    }

    pub(crate) fn with_html_preview(
        mut self,
        host: std::sync::Arc<crate::html_render::PreviewHost>,
    ) -> Self {
        self.html_preview = host;
        self
    }

    fn tool_settings_state(&self, chat_id: Option<&str>) -> zeron_proto::tools::ToolSettingsState {
        let devices_on = self
            .device_hub
            .as_ref()
            .is_some_and(|hub| hub.agent_access_enabled());
        let chat_disabled = chat_id
            .and_then(|id| self.workspace.chat_config(id))
            .map(|config| config.disabled_tools)
            .unwrap_or_default();
        self.registry
            .tool_settings_state(devices_on, &chat_disabled)
    }

    fn device_hub(&self) -> Result<&std::sync::Arc<crate::device_hub::DeviceHub>, RpcError> {
        self.device_hub
            .as_ref()
            .ok_or_else(|| RpcError::Failed("device hub unavailable".into()))
    }

    fn computer(&self) -> Result<&std::sync::Arc<ComputerRuntime>, RpcError> {
        self.computer
            .as_ref()
            .ok_or_else(|| RpcError::Failed("computer use unavailable".into()))
    }

    #[cfg(test)]
    pub(crate) fn with_github(mut self, github: GitHubCli) -> Self {
        self.github = github;
        self
    }

    fn auth(&self) -> Result<&Auth, RpcError> {
        self.auth
            .as_ref()
            .ok_or_else(|| RpcError::Failed("auth unavailable".into()))
    }

    fn updater(&self) -> Result<&zeron_update::Updater, RpcError> {
        self.updater
            .as_ref()
            .ok_or_else(|| RpcError::Failed("updates unavailable".into()))
    }

    fn harness_updates(
        &self,
    ) -> Result<&crate::harness_updates::HarnessUpdateCoordinator, RpcError> {
        self.harness_updates
            .as_ref()
            .ok_or_else(|| RpcError::Failed("agent updates unavailable".into()))
    }

    fn resume_schedule(&self) -> Result<&crate::resume_schedule::ResumeSchedule, RpcError> {
        self.resume_schedule
            .as_ref()
            .ok_or_else(|| RpcError::Failed("resume schedule unavailable".into()))
    }

    async fn stop_for_update(&self, chats: &[String]) -> Vec<crate::resume_schedule::Continuation> {
        let mut stopped = Vec::new();
        for chat_id in chats {
            match self.sessions.interrupt(chat_id).await {
                Ok(_) => stopped.push(crate::resume_schedule::Continuation {
                    chat_id: chat_id.clone(),
                    after_user: self.doc_host.last_user_entry(chat_id).ok().flatten(),
                }),
                Err(error) => tracing::warn!(chat = %chat_id, %error, "update interrupt failed"),
            }
        }
        stopped
    }

    fn local_importer(&self) -> Result<&crate::local_import::LocalImporter, RpcError> {
        self.local_import
            .as_ref()
            .ok_or_else(|| RpcError::Failed("local import requires a synced workspace".into()))
    }

    fn detach_profile_from_chats(&self, profile_id: &str) -> Result<(), RpcError> {
        let local_device = self.doc_host.device_id();
        let chats = self.workspace.watch_chats().borrow().clone();
        for chat in chats.iter().filter(|chat| chat.device_id == local_device) {
            let Some(mut config) = chat
                .config
                .clone()
                .filter(|config| config.profile_id.as_deref() == Some(profile_id))
            else {
                continue;
            };
            config.profile_id = None;
            config.instructions.clear();
            self.workspace
                .set_chat_config(&chat.id, &config)
                .map_err(|e| RpcError::Failed(e.to_string()))?;
        }
        Ok(())
    }

    async fn retire_instance_sessions(&self, instance_id: &str) {
        let local_device = self.doc_host.device_id();
        let chats = self.workspace.watch_chats().borrow().clone();
        for chat in chats.iter().filter(|chat| {
            chat.device_id == local_device
                && chat
                    .config
                    .as_ref()
                    .and_then(|config| config.instance_id.as_deref())
                    == Some(instance_id)
        }) {
            if let Err(error) = self.sessions.switch_harness(&chat.id).await {
                tracing::warn!(chat_id = %chat.id, %error, "could not close a provider instance's session");
            }
        }
    }

    fn route_delegation(
        &self,
        mut p: crate::doc_host::DelegateTaskParams,
    ) -> Result<crate::doc_host::DelegateTaskParams, RpcError> {
        let Some(role) = p.role.as_deref().and_then(crate::routing::Role::parse) else {
            return Ok(p);
        };
        let owner = self
            .workspace
            .chat(&p.owner_chat_id)
            .map_err(|e| RpcError::Failed(e.to_string()))?;
        let offered: Vec<HarnessId> = self.registry.offered().iter().map(|d| d.id).collect();
        let resolved = crate::routing::resolve(
            &crate::routing::Request {
                role: Some(role),
                chosen: &p.chosen,
                harness: p.harness,
                model: p.model.as_deref(),
                reasoning: p.reasoning,
                profile: p.profile.profile_id.is_some(),
            },
            owner.as_ref().and_then(|chat| chat.config.as_ref()),
            &self.registry.routing_settings(),
            &offered,
        );
        p.harness = resolved.harness;
        p.model = resolved.model;
        p.reasoning = resolved.reasoning;
        p.route = resolved.route;
        Ok(p)
    }

    async fn start_review(&self, p: StartReviewParams) -> Result<serde_json::Value, RpcError> {
        use crate::review::{PromptInput, ReviewScope, ReviewerOptions};
        let failed = |message: &str| RpcError::Failed(message.to_string());
        let chat = self
            .workspace
            .chat(&p.owner_chat_id)
            .map_err(|e| RpcError::Failed(e.to_string()))?
            .ok_or_else(|| failed("chat not found"))?;
        if chat.device_id != self.doc_host.device_id() {
            return Err(failed("chat is not hosted by this device"));
        }
        let space = match chat.space_id.as_deref() {
            Some(id) => self
                .workspace
                .space(id)
                .map_err(|e| RpcError::Failed(e.to_string()))?,
            None => None,
        }
        .filter(|space| space.git_detected)
        .ok_or_else(|| failed("this chat's project is not a git repository"))?;
        let cwd = chat.cwd.clone().unwrap_or(space.path);
        let scope =
            ReviewScope::from_key(&p.scope).ok_or_else(|| failed("unknown review scope"))?;
        let commit = match scope {
            ReviewScope::Commit => Some(
                p.commit_sha
                    .clone()
                    .filter(|sha| !sha.trim().is_empty())
                    .ok_or_else(|| failed("commitSha required"))?,
            ),
            _ => None,
        };
        let value = |reply: RpcReply| match reply {
            RpcReply::Value(value) => Ok(value),
            RpcReply::Stream(_) => Err(failed("unexpected stream")),
        };
        let base = match scope {
            ReviewScope::Branch => {
                let branches = value(
                    self.handle(
                        methods::LIST_BRANCHES,
                        serde_json::json!({ "repoPath": cwd }),
                    )
                    .await?,
                )?;
                let branches: Vec<String> = serde_json::from_value(branches).unwrap_or_default();
                Some(
                    crate::review::default_base_ref(&branches, chat.branch.as_deref())
                        .ok_or_else(|| failed("No base branch to compare against"))?,
                )
            }
            _ => None,
        };
        let diff = value(
            self.handle(
                methods::GET_CHECKOUT_DIFF,
                serde_json::json!({
                    "cwd": cwd,
                    "mode": scope.mode(),
                    "chatId": chat.id,
                    "baseRef": base,
                    "commitSha": commit,
                }),
            )
            .await?,
        )?;
        let diff: zeron_proto::CheckoutDiff =
            serde_json::from_value(diff).map_err(|e| RpcError::Failed(e.to_string()))?;
        let files: Vec<String> = diff.files.iter().map(|file| file.path.clone()).collect();
        if files.is_empty() {
            return Err(failed("Nothing to review"));
        }
        let settings = self.registry.review_settings();
        let descriptors = self.registry.offered();
        let options = ReviewerOptions {
            offered: descriptors.iter().map(|d| d.id).collect(),
            levels: descriptors
                .into_iter()
                .map(|d| (d.id, d.reasoning_levels))
                .collect(),
            profiles: self
                .agent_profiles
                .as_ref()
                .map(|store| store.snapshot().profiles)
                .unwrap_or_default(),
        };
        let chat_harness = chat.config.as_ref().map(|config| config.harness);
        let owner_permission = chat.config.as_ref().map(|config| config.permission_mode);
        let choice = p.reviewer.clone().unwrap_or(settings.reviewer);
        let reviewer = crate::review::resolve_reviewer(&choice, chat_harness, &options);
        let profile = reviewer.profile.as_ref();
        let permission_mode = profile
            .and_then(|profile| profile.permission_mode)
            .filter(|mode| owner_permission.is_some_and(|owner| mode.rank() <= owner.rank()));
        let task = crate::review::prompt(PromptInput {
            scope,
            base: base.as_deref(),
            commit: commit.as_deref(),
            patch: &diff.patch,
            truncated: diff.truncated,
            files: &files,
            instructions: &settings.instructions,
        });
        let request =
            crate::review::request_id(scope, commit.as_deref(), &uuid::Uuid::new_v4().to_string());
        let view = self
            .doc_host
            .delegate_task(
                crate::doc_host::DelegateTaskParams {
                    owner_chat_id: chat.id.clone(),
                    task,
                    title: Some("Review changes".to_string()),
                    role: Some("validator".to_string()),
                    harness: reviewer.harness,
                    model: reviewer.model.clone(),
                    reasoning: reviewer.reasoning,
                    profile: crate::doc_host::ProfileParams {
                        permission_mode,
                        model_options: profile
                            .filter(|profile| !profile.model_options.is_empty())
                            .map(|profile| profile.model_options.clone()),
                        instructions: profile.map(|profile| profile.instructions.clone()),
                        profile_id: profile.map(|profile| profile.id.clone()),
                        disabled_tools: profile
                            .map(|profile| profile.disabled_tools.clone())
                            .unwrap_or_default(),
                    },
                    wait: false,
                    wake: Some(p.wake.unwrap_or(zeron_proto::DelegationWake::Never)),
                    client_request_id: Some(request),
                    chosen: Vec::new(),
                    route: None,
                },
                self.registry.delegation_settings(),
            )
            .await
            .map_err(|e| RpcError::Failed(e.to_string()))?;
        let task_id = serde_json::to_value(&view)
            .ok()
            .and_then(|view| view.get("taskId")?.as_str().map(str::to_string))
            .ok_or_else(|| failed("The reviewer did not start"))?;
        Ok(serde_json::json!({
            "taskId": task_id,
            "patch": diff.patch,
            "note": reviewer.note,
            "reviewer": reviewer.name(),
            "scope": scope.key(),
            "commitSha": commit,
        }))
    }

    fn agent_profiles_store(&self) -> Result<AgentProfilesStore, RpcError> {
        self.agent_profiles
            .clone()
            .ok_or_else(|| RpcError::Failed("agent profiles are not available".into()))
    }

    fn discovery_harness(
        &self,
        chat_id: Option<&str>,
        harness: HarnessId,
    ) -> Result<std::sync::Arc<dyn zeron_harness::Harness>, RpcError> {
        match chat_id {
            Some(chat_id) => self.sessions.harness_for_chat(chat_id, harness),
            None => self
                .registry
                .resolve(harness)
                .map_err(crate::EngineError::from),
        }
        .map_err(|e| RpcError::Failed(e.to_string()))
    }

    fn provider_instances_store(&self) -> Result<ProviderInstancesStore, RpcError> {
        self.provider_instances
            .clone()
            .ok_or_else(|| RpcError::Failed("provider instances are not available".into()))
    }

    /// ACP instances have no accounts of their own; their sign-ins are flows of the default ones.
    fn acp_instance(
        &self,
        instance_id: Option<&str>,
    ) -> Result<Option<zeron_proto::ProviderInstance>, RpcError> {
        let Some(id) = instance_id else {
            return Ok(None);
        };
        let instance = self
            .provider_instances_store()?
            .require(id)
            .map_err(|e| RpcError::Failed(e.to_string()))?;
        Ok(crate::provider_instances::runs_over_acp(instance.driver).then_some(instance))
    }

    fn login_accounts_for(&self, instance_id: Option<&str>) -> Result<AgentAccounts, RpcError> {
        if self.acp_instance(instance_id)?.is_some() {
            return Ok(self.agent_accounts.clone());
        }
        Ok(self.agent_accounts_for(instance_id, None)?.0)
    }

    fn agent_accounts_for(
        &self,
        instance_id: Option<&str>,
        harness: Option<HarnessId>,
    ) -> Result<(AgentAccounts, Option<HarnessId>), RpcError> {
        let Some(id) = instance_id else {
            return Ok((self.agent_accounts.clone(), None));
        };
        let instance = self
            .provider_instances_store()?
            .require(id)
            .map_err(|e| RpcError::Failed(e.to_string()))?;
        if let Some(harness) = harness.filter(|harness| *harness != instance.driver) {
            return Err(RpcError::Failed(format!(
                "Provider instance '{id}' runs on {}, not {}",
                instance.driver.display_name(),
                harness.display_name()
            )));
        }
        let accounts = self
            .agent_accounts
            .for_instance(&instance)
            .map_err(|e| RpcError::Failed(e.to_string()))?;
        Ok((accounts, Some(instance.driver)))
    }

    fn local_project_action_space(&self, space_id: &str) -> Result<Space, RpcError> {
        let space = self
            .workspace
            .space(space_id)
            .map_err(|err| RpcError::Failed(err.to_string()))?
            .ok_or_else(|| RpcError::Failed("Project space not found".into()))?;
        if space.device_id != self.doc_host.device_id() {
            return Err(RpcError::Failed(
                "Project space belongs to another device".into(),
            ));
        }
        Ok(space)
    }

    /// Resolve a mention-search root from synced workspace rows. A client may
    /// name an existing linked worktree for a new chat, but it is verified
    /// against the space repository before any filesystem walk begins.
    async fn file_search_root(&self, p: &FileSearchParams) -> Result<std::path::PathBuf, RpcError> {
        let target = zeron_proto::WorkspaceTarget {
            chat_id: p.chat_id.clone(),
            space_id: p.space_id.clone(),
            checkout_path: p.path.clone(),
        };
        self.workspace_files
            .resolve_target(&target)
            .await
            .map(|workspace| workspace.root)
            .map_err(Into::into)
    }

    /// Catalogs also serve projectless sessions, which have no workspace space.
    /// Keep workspace validation for project targets and use only a known local
    /// chat's persisted cwd (or home) for a projectless conversation.
    async fn catalog_root(&self, p: &FileSearchParams) -> Result<std::path::PathBuf, RpcError> {
        if p.space_id.is_none() && p.path.is_none() {
            let Some(chat_id) = &p.chat_id else {
                return session_home_dir().map_err(|error| RpcError::Failed(error.to_string()));
            };
            let chat = self
                .workspace
                .chat(chat_id)
                .map_err(|e| RpcError::Failed(e.to_string()))?
                .ok_or_else(|| RpcError::BadParams("chat not found".into()))?;
            if chat.device_id != self.doc_host.device_id() {
                return Err(RpcError::BadParams("chat belongs to another device".into()));
            }
            if chat.space_id.is_none() {
                let cwd = chat.cwd.as_deref().unwrap_or("~");
                return crate::repos::resolve_chat_cwd(cwd, &self.repos.scratch_root())
                    .map(std::path::PathBuf::from)
                    .map_err(RpcError::Failed);
            }
        }
        self.file_search_root(p).await
    }

    async fn source_control_checkout(
        &self,
        target: &zeron_proto::WorkspaceTarget,
    ) -> Result<crate::workspace_files::ResolvedWorkspace, RpcError> {
        if target.chat_id.is_some() || target.space_id.is_none() {
            return Err(RpcError::Failed(
                "source control needs a spaceId target".into(),
            ));
        }
        let workspace = self
            .workspace_files
            .resolve_target(target)
            .await
            .map_err(RpcError::from)?;
        if workspace.is_plain_folder() {
            return Err(RpcError::Failed(
                "this project is not a Git checkout".into(),
            ));
        }
        Ok(workspace)
    }

    async fn project_folder_in_checkout(
        &self,
        target: &zeron_proto::WorkspaceTarget,
        root: &std::path::Path,
    ) -> Option<String> {
        let space = self.workspace.space(target.space_id.as_deref()?).ok()??;
        let requested = target.checkout_path.clone().unwrap_or(space.path);
        let requested = tokio::fs::canonicalize(requested).await.ok()?;
        let root = tokio::fs::canonicalize(root).await.ok()?;
        let folder = requested.strip_prefix(&root).ok()?;
        (!folder.as_os_str().is_empty()).then(|| folder.to_string_lossy().replace('\\', "/"))
    }

    async fn refuse_if_agent_busy(
        &self,
        checkout_id: &str,
        allowed: bool,
        policy: zeron_proto::GitAgentBusyPolicy,
        safe_only: &str,
    ) -> Result<(), RpcError> {
        if allowed || !self.agent_active_on_checkout(checkout_id).await {
            return Ok(());
        }
        Err(RpcError::Failed(
            match policy {
                zeron_proto::GitAgentBusyPolicy::BlockAll => {
                    "An agent is running in this checkout, and Git is read-only while it runs (Settings > Source Control)."
                }
                _ => safe_only,
            }
            .into(),
        ))
    }

    async fn agent_active_on_checkout(&self, checkout_id: &str) -> bool {
        let local_device = self.doc_host.device_id();
        let local: Vec<(String, Option<String>, Option<String>)> = self
            .workspace
            .watch_chats()
            .borrow()
            .iter()
            .filter(|chat| chat.device_id == local_device)
            .map(|chat| (chat.id.clone(), chat.checkout_id.clone(), chat.cwd.clone()))
            .collect();
        for (id, candidate_checkout, candidate_cwd) in local {
            if !self.sessions.session_status(&id).is_some_and(|session| {
                matches!(
                    session.status,
                    zeron_proto::SessionStatus::Working | zeron_proto::SessionStatus::AwaitingInput
                )
            }) {
                continue;
            }
            let same_checkout = if candidate_checkout.as_deref() == Some(checkout_id) {
                true
            } else if let Some(candidate_cwd) = candidate_cwd.as_deref() {
                self.repos
                    .checkout_identity(std::path::Path::new(candidate_cwd))
                    .await
                    .is_ok_and(|candidate_identity| candidate_identity.id == checkout_id)
            } else {
                false
            };
            if same_checkout {
                return true;
            }
        }
        false
    }

    /// Accept only a checkout already named by a local chat or contained in a
    /// local space. Remote clients must not turn this RPC into an arbitrary path probe.
    async fn change_request_root(&self, cwd: &str) -> Result<std::path::PathBuf, RpcError> {
        let requested = std::path::PathBuf::from(cwd);
        let local_device = self.doc_host.device_id();
        let mut chats_rx = self.workspace.watch_chats();
        let mut spaces_rx = self.workspace.watch_spaces();
        let deadline = tokio::time::Instant::now() + Duration::from_millis(250);
        loop {
            let chats = chats_rx.borrow_and_update().clone();
            if chats.iter().any(|chat| {
                chat.device_id == local_device
                    && chat.cwd.as_deref().map(std::path::Path::new) == Some(requested.as_path())
            }) {
                return Ok(requested);
            }

            let spaces = spaces_rx.borrow_and_update().clone();
            for space in spaces
                .iter()
                .filter(|space| space.device_id == local_device)
            {
                if let Some(checkout) = self
                    .repos
                    .workspace_checkout(std::path::Path::new(&space.path), &requested)
                    .await
                {
                    return Ok(checkout);
                }
            }

            if tokio::time::Instant::now() >= deadline {
                break;
            }
            tokio::select! {
                _ = chats_rx.changed() => {}
                _ = spaces_rx.changed() => {}
                _ = tokio::time::sleep_until(deadline) => break,
            }
        }
        Err(RpcError::BadParams(
            "cwd is not a known checkout on this device".into(),
        ))
    }

    /// Most-recent-first paths the current chat actually touched, followed by
    /// files still changed in its checkout. The search worker validates and
    /// normalizes them against the resolved root before using them as ranking
    /// hints, so stale or out-of-workspace tool paths simply disappear.
    fn featured_file_paths(&self, chat_id: &str) -> Vec<String> {
        let mut paths = Vec::new();
        let mut seen = HashSet::new();
        if let Ok(handle) = self.doc_host.open(chat_id)
            && let Ok(entries) = handle.doc().read_entries()
        {
            for entry in entries.into_iter().rev() {
                for part in entry.parts.into_iter().rev() {
                    if let MessagePart::Tool { call, .. } = part
                        && let Some(path) = tool_file_path(&call)
                        && !path.trim().is_empty()
                        && seen.insert(path.to_string())
                    {
                        paths.push(path.to_string());
                        if paths.len() == FILE_SEARCH_FEATURED_PATHS {
                            break;
                        }
                    }
                }
                if paths.len() == FILE_SEARCH_FEATURED_PATHS {
                    break;
                }
            }
        }

        if let Ok(Some(chat)) = self.workspace.chat(chat_id) {
            let diffs = self.diff_sync.watch_diffs().borrow().clone();
            let diff = chat
                .checkout_id
                .as_deref()
                .and_then(|id| diffs.iter().find(|diff| diff.checkout_id == id))
                .or_else(|| {
                    chat.cwd
                        .as_deref()
                        .and_then(|cwd| diffs.iter().find(|diff| diff.cwd == cwd))
                });
            if let Some(diff) = diff {
                for file in &diff.files {
                    if paths.len() == FILE_SEARCH_FEATURED_PATHS {
                        break;
                    }
                    if seen.insert(file.path.clone()) {
                        paths.push(file.path.clone());
                    }
                }
            }
        }
        paths
    }

    /// An agent login runs on `target`, but the browser that finishes it runs
    /// HERE: while the login waits on a loopback callback, this device's same
    /// port forwards to it over P2P ([`zeron_preview::login`]). The forwarder
    /// opens when a reply first names the port and closes when the login
    /// finishes, fails, is cancelled or its time runs out; a port taken here
    /// fails the login with that reason instead of stranding the browser.
    async fn forward_agent_login(
        &self,
        target: &str,
        method: &str,
        mut params: serde_json::Value,
    ) -> Result<RpcReply, RpcError> {
        use zeron_proto::{AgentLoginPoll, AgentLoginStart, AgentLoginStatus};
        let login_id = params
            .get("loginId")
            .and_then(|v| v.as_str())
            .map(str::to_string);
        let instance_id = params.get("instanceId").cloned();
        if method == methods::START_AGENT_LOGIN
            && let Some(object) = params.as_object_mut()
        {
            object.insert(
                "requesterDeviceId".into(),
                serde_json::json!(self.doc_host.device_id()),
            );
        }
        let reply = self.forward(target, method, params).await;
        let Some(previews) = &self.previews else {
            return reply;
        };
        let value = match &reply {
            Ok(RpcReply::Value(value)) => Some(value.clone()),
            _ => None,
        };
        match method {
            methods::START_AGENT_LOGIN => {
                let Some(start) =
                    value.and_then(|v| serde_json::from_value::<AgentLoginStart>(v).ok())
                else {
                    return reply;
                };
                if let Some(port) = start.callback_port
                    && let Err(error) = if crate::agent_accounts::tunnel_port_allowed(
                        port,
                        Some(start.url.as_str()),
                    ) {
                        previews
                            .open_login_tunnel(&start.login_id, target, port, LOGIN_TUNNEL_TTL)
                            .await
                    } else {
                        Err(anyhow::anyhow!(
                            "The other device reported an unexpected sign-in port."
                        ))
                    }
                {
                    self.cancel_remote_login(target, &start.login_id, instance_id)
                        .await;
                    return Err(RpcError::Failed(error.to_string()));
                }
                reply
            }
            methods::POLL_AGENT_LOGIN => {
                let Some(login_id) = login_id else {
                    return reply;
                };
                let poll = value.and_then(|v| serde_json::from_value::<AgentLoginPoll>(v).ok());
                match poll {
                    Some(poll) if poll.status == AgentLoginStatus::Pending => {
                        if let Some(port) = poll.callback_port
                            && let Err(error) = if crate::agent_accounts::tunnel_port_allowed(
                                port,
                                poll.url.as_deref(),
                            ) {
                                previews
                                    .open_login_tunnel(&login_id, target, port, LOGIN_TUNNEL_TTL)
                                    .await
                            } else {
                                Err(anyhow::anyhow!(
                                    "The other device reported an unexpected sign-in port."
                                ))
                            }
                        {
                            self.cancel_remote_login(target, &login_id, instance_id)
                                .await;
                            return RpcReply::value(&AgentLoginPoll {
                                status: AgentLoginStatus::Error,
                                message: Some(error.to_string()),
                                url: None,
                                callback_port: None,
                            });
                        }
                        reply
                    }
                    // Done, failed, expired, or unreachable: the login is over.
                    _ => {
                        previews.close_login_tunnel(&login_id);
                        reply
                    }
                }
            }
            _ => {
                if let Some(login_id) = login_id {
                    previews.close_login_tunnel(&login_id);
                }
                reply
            }
        }
    }

    async fn cancel_remote_login(
        &self,
        target: &str,
        login_id: &str,
        instance_id: Option<serde_json::Value>,
    ) {
        let mut params = serde_json::json!({ "loginId": login_id, "targetDeviceId": target });
        if let Some(instance_id) = instance_id {
            params["instanceId"] = instance_id;
        }
        if let Err(error) = self
            .forward(target, methods::CANCEL_AGENT_LOGIN, params)
            .await
        {
            tracing::debug!(%error, "cancelling the remote login failed (best-effort)");
        }
    }

    /// Forward a device-addressed call over the target device's relay. On transport
    /// failure the cached link is invalidated so the next call re-dials.
    async fn forward(
        &self,
        target: &str,
        method: &str,
        params: serde_json::Value,
    ) -> Result<RpcReply, RpcError> {
        let Some(links) = &self.links else {
            return Err(RpcError::Failed(format!(
                "cannot reach device {target}: remote routing unavailable (offline)"
            )));
        };
        let client = links.client(target).await?;
        if is_stream_method(method) {
            // Streams are unbounded by design (a quiet WATCH_* is healthy);
            // only unary calls below get the reply deadline.
            if matches!(
                method,
                methods::OWN_VOICE_V2
                    | methods::WATCH_CHECKOUT_CHANGE_REQUEST
                    | methods::WATCH_WORKSPACE_GIT_STATUS
                    | methods::WATCH_HARNESS_UPDATES
                    | methods::WATCH_LIMIT_RESUMES
            ) {
                let rx = match client.subscribe_checked(method, params).await {
                    Ok(rx) => rx,
                    Err(err) => {
                        if should_invalidate_link(&err) {
                            links.invalidate(target);
                        }
                        return Err(err);
                    }
                };
                let stream = futures::stream::unfold((rx, client), |(mut rx, client)| async move {
                    rx.recv().await.map(|item| (item, (rx, client)))
                });
                return Ok(RpcReply::Stream(stream.boxed()));
            }
            let rx = match client.subscribe_scoped(method, params).await {
                Ok(rx) => rx,
                Err(err) => {
                    if should_invalidate_link(&err) {
                        links.invalidate(target);
                    }
                    return Err(err);
                }
            };
            // Pipe remote items; the held client keeps the link's RpcClient alive for
            // the stream's lifetime. A remote error just ends the stream (the relay
            // link-down path fails pending calls; stream receivers close).
            let stream = futures::stream::unfold((rx, client), |(mut rx, client)| async move {
                rx.recv().await.map(|item| (item, (rx, client)))
            });
            return Ok(RpcReply::Stream(stream.boxed()));
        }
        let deadline = forward_deadline(method);
        match tokio::time::timeout(deadline, client.call(method, params)).await {
            Ok(Ok(value)) => Ok(RpcReply::Value(value)),
            Ok(Err(err)) => {
                if should_invalidate_link(&err) {
                    links.invalidate(target);
                }
                Err(err)
            }
            Err(_) => {
                // No reply inside the deadline. The link may be a zombie — the
                // relay's auto-pong keeps a dead host socket looking alive
                // (ws3 auto-pong incident) — so drop it; the next call re-dials.
                // NOTE: the remote may still complete the forwarded work; the
                // caller sees a retryable failure instead of hanging forever
                // (the "Sending…" wedge, 2026-08-18).
                links.invalidate(target);
                Err(RpcError::Transport(format!(
                    "no reply from device {target} for {method} within {}s",
                    deadline.as_secs()
                )))
            }
        }
    }

    async fn prepare_voice_bridge(
        &self,
        chat: &str,
    ) -> Result<
        (
            zeron_harness::codex::realtime::RealtimeHandle,
            tokio::sync::broadcast::Receiver<zeron_proto::voice::VoiceEvent>,
            std::sync::Arc<crate::sessions::VoiceActivity>,
        ),
        zeron_proto::voice::VoiceRejection,
    > {
        use zeron_proto::voice::VoiceRejection;
        // Without Codex here a retry can never connect: ask for an install.
        if !self
            .registry
            .resolve(HarnessId::Codex)
            .is_ok_and(|codex| codex.installed())
        {
            return Err(VoiceRejection::NativeRuntimeUnavailable);
        }
        let mut request = self
            .doc_host
            .request_from_chat_row(chat, "")
            .ok_or(VoiceRejection::Unsupported)?;
        if request.cwd.is_empty() {
            request.cwd = "~".into();
        }
        self.sessions
            .start_idle(chat, request)
            .await
            .map_err(|_| VoiceRejection::Unsupported)?;
        self.sessions
            .take_voice_bridge(chat)
            .ok_or(VoiceRejection::Unsupported)
    }

    async fn mutate(&self, params: MutateParams) -> Result<(), RpcError> {
        let failed = |e: crate::EngineError| RpcError::Failed(e.to_string());
        match params {
            MutateParams::CreateChat {
                chat_id,
                space_id,
                device_id,
                config,
                branch,
                cwd,
                created_by_chat_id,
            } => {
                // A voice orchestrator is hidden, so the chats it creates are
                // the user's top-level sessions rather than its side chats.
                let created_by_chat_id = created_by_chat_id
                    .filter(|parent| !zeron_proto::voice::is_orchestrator_chat(parent));
                self.workspace
                    .create_chat_from(
                        &chat_id,
                        space_id.as_deref(),
                        device_id.as_deref(),
                        config,
                        cwd,
                        created_by_chat_id.as_deref(),
                    )
                    .map_err(failed)?;
                if let Some(branch) = branch.as_deref().filter(|b| !b.is_empty()) {
                    self.workspace
                        .set_chat_branch(&chat_id, branch)
                        .map_err(failed)?;
                }
                Ok(())
            }
            MutateParams::CreateSpace {
                space_id,
                device_id,
                path,
                name,
                git_detected,
            } => self
                .workspace
                .create_space(&space_id, &device_id, &path, name, git_detected)
                .map_err(failed),
            MutateParams::RenameSpace { space_id, name } => self
                .workspace
                .rename_space(&space_id, name.as_deref())
                .map_err(failed)
                .map(drop),
            MutateParams::DeleteSpace { space_id } => {
                let deleted = self.workspace.delete_space(&space_id).map_err(failed)?;
                // Best-effort teardown of live runs we host for the deleted chats
                // (the doc rows are already tombstoned; a straggler run would only
                // write into an orphaned session doc).
                let sessions = self.sessions.clone();
                let doc_host = self.doc_host.clone();
                let uploads = self.uploads.clone();
                let chat_ids = deleted.chat_ids;
                tokio::spawn(async move {
                    for chat_id in chat_ids {
                        if let Err(err) = sessions.terminate(&chat_id).await {
                            tracing::debug!(chat = %chat_id, error = %err, "deleteSpace teardown skipped");
                        }
                        doc_host.purge_chat(&chat_id);
                        crate::html_render::remove_chat_pages(&uploads, &chat_id);
                    }
                });
                Ok(())
            }
            MutateParams::RenameChat { chat_id, title } => self
                .workspace
                .rename_chat(&chat_id, &title)
                .map_err(failed)
                .map(drop),
            MutateParams::SetChatBranch { chat_id, branch } => self
                .workspace
                .set_chat_branch(&chat_id, &branch)
                .map_err(failed)
                .map(drop),
            MutateParams::SetChatCwd { chat_id, cwd } => {
                self.voice.retire_chat(&chat_id);
                self.workspace
                    .set_chat_cwd(&chat_id, &cwd)
                    .map_err(failed)
                    .map(drop)
            }
            MutateParams::SetChatActivity {
                chat_id,
                last_message_at,
                created_at,
            } => self
                .workspace
                .set_chat_activity(&chat_id, last_message_at, created_at)
                .map_err(failed)
                .map(drop),
            MutateParams::SetChatHost { chat_id, device_id } => {
                self.voice.retire_chat(&chat_id);
                self.workspace
                    .set_chat_host(&chat_id, &device_id)
                    .map_err(failed)
                    .map(drop)
            }
            MutateParams::SetChatArchived { chat_id, archived } => self
                .workspace
                .set_chat_archived(&chat_id, archived)
                .map_err(failed)
                .map(drop),
            MutateParams::SetChatSettled { chat_id, settled } => self
                .workspace
                .set_chat_settled(&chat_id, settled)
                .map_err(failed)
                .map(drop),
            MutateParams::ChangeSidebarPin { change } => {
                self.workspace.change_sidebar_pin(&change).map_err(failed)
            }
            MutateParams::SetChatConfig {
                chat_id,
                config,
                note,
            } => {
                self.voice.retire_chat(&chat_id);
                let left_config = self.workspace.chat_config(&chat_id);
                let left_instance = left_config.as_ref().and_then(|c| c.instance_id.clone());
                let left_model = left_config.and_then(|c| c.model);
                let left = self.doc_host.harness_for(&chat_id);
                // Another instance of the same driver is another home and login: its session cannot resume.
                let switching = left != config.harness || left_instance != config.instance_id;
                if switching {
                    self.sessions
                        .switch_harness(&chat_id)
                        .await
                        .map_err(failed)?;
                }
                let handle = (note.is_some() && self.doc_host.is_host(&chat_id))
                    .then(|| self.doc_host.open(&chat_id))
                    .transpose()
                    .map_err(failed)?;
                // A change on top of a handoff nobody has sent to yet folds
                // into that divider instead of stacking a second one: the chat
                // still left the harness the first divider named, and the one
                // in between never ran, so it leaves no mark either.
                let replaced = handle
                    .as_ref()
                    .and_then(|handle| unanswered_switch_note(handle));
                if replaced.is_some() && switching {
                    self.workspace
                        .set_chat_config_after_unsent_switch(&chat_id, &config)
                } else {
                    self.workspace.set_chat_config(&chat_id, &config)
                }
                .map_err(failed)?;
                let (Some(handle), Some(mut note)) = (handle, note) else {
                    return Ok(());
                };
                let handoff = match replaced {
                    Some(previous) => {
                        handle
                            .doc()
                            .truncate_from(&previous.id)
                            .map_err(|e| failed(e.into()))?;
                        match fold_switch_note(&previous, left, &config, &note) {
                            Some((folded, route)) => {
                                note = folded;
                                Some((previous.packet, route))
                            }
                            // Back where it started: the transcript shows no
                            // switch, and the closed session hands over on the
                            // next turn like any re-sent first turn.
                            None => return Ok(()),
                        }
                    }
                    // The packet is built now, at the switch, so the divider
                    // shows it at once; the first run sends this same text.
                    None => switching.then(|| {
                        let packet = self
                            .sessions
                            .provider_switch_packet(&chat_id)
                            .unwrap_or_default();
                        let route = zeron_doc::HandoffRoute {
                            from_harness: Some(left),
                            from_model: left_model,
                            to_model: config.model.clone(),
                            ..Default::default()
                        };
                        (packet, route)
                    }),
                };
                handle
                    .write_system_note_with(
                        &crate::new_id(),
                        &note.text,
                        note.hint.as_deref(),
                        handoff.as_ref().map(|(packet, route)| {
                            (config.harness, packet.as_str(), route.clone())
                        }),
                        crate::now_ms(),
                    )
                    .map_err(|e| failed(e.into()))?;
                Ok(())
            }
            MutateParams::DeleteChat { chat_id } => self.delete_chat(&chat_id),
            MutateParams::RenameDevice { device_id, name } => self
                .workspace
                .rename_device(&device_id, &name)
                .map_err(failed)
                .map(drop),
            MutateParams::MarkChatSeen { chat_id, at } => {
                let at = at
                    .and_then(chrono::DateTime::<chrono::Utc>::from_timestamp_millis)
                    .unwrap_or_else(chrono::Utc::now);
                self.workspace
                    .mark_chat_seen(&chat_id, at)
                    .map_err(failed)
                    .map(drop)
            }
            MutateParams::CreateTask {
                task_id,
                title,
                notes,
                space_id,
                due_on,
                order_key,
                images,
                context,
            } => self.create_task(
                task_id, title, notes, space_id, due_on, order_key, images, context,
            ),
            MutateParams::UpdateTask {
                task_id,
                patch,
                from_chat_id,
            } => {
                if patch.title.is_some()
                    || patch.notes.is_some()
                    || patch.images.is_some()
                    || patch.chat_id.is_some()
                {
                    let refusal = if self.registry.supervisor_settings().goal_and_spend {
                        supervisor::GOAL_ACTION
                    } else {
                        supervisor::GOAL_OR_SPEND
                    };
                    supervisor::refuse_supervisor(from_chat_id.as_deref(), refusal)?;
                }
                self.update_task(&task_id, &patch)
            }
            MutateParams::DeleteTask {
                task_id,
                from_chat_id,
            } => {
                supervisor::refuse_supervisor(from_chat_id.as_deref(), supervisor::GOAL_OR_SPEND)?;
                self.workspace
                    .delete_task(&task_id)
                    .map_err(failed)
                    .map(drop)
            }
            MutateParams::StartTask {
                task_id,
                chat_id,
                space_id,
                config,
                orchestrate,
                approve_plan,
                same_agents,
                plan_doc,
            } => {
                let lead = orchestrate.then_some(tasks::Lead {
                    approve_plan,
                    same_agents,
                    plan_doc,
                    attach: false,
                });
                self.start_task(&task_id, &chat_id, space_id, config, lead)
                    .await
                    .map(drop)
            }
            MutateParams::AssignTask {
                task_id,
                chat_id,
                send,
                from_chat_id,
                orchestrate,
                approve_plan,
                same_agents,
                plan_doc,
                attach_plan,
            } => {
                supervisor::refuse_supervisor(from_chat_id.as_deref(), supervisor::GOAL_OR_SPEND)?;
                let lead = orchestrate.then_some(tasks::Lead {
                    approve_plan,
                    same_agents,
                    plan_doc,
                    attach: attach_plan,
                });
                self.assign_task(&task_id, &chat_id, send, from_chat_id.as_deref(), lead)
                    .await
            }
            MutateParams::PlanSteps {
                task_id,
                steps,
                overview,
                resources,
                from_chat_id,
            } => {
                let steps = steps
                    .into_iter()
                    .map(|step| (step.title, step.notes, step.thread_id))
                    .collect();
                self.plan_steps(
                    &task_id,
                    steps,
                    overview,
                    resources,
                    from_chat_id.as_deref(),
                )
                .map(drop)
            }
            MutateParams::CreateTaskStep {
                task_id,
                step_id,
                title,
                notes,
                order_key,
            } => self.create_task_step(task_id, step_id, title, notes, order_key, Vec::new()),
            MutateParams::UpdateTaskStep {
                step_id,
                patch,
                from_chat_id,
            } => self.update_task_step(&step_id, &patch, from_chat_id.as_deref()),
            MutateParams::DeleteTaskStep { step_id } => self.delete_task_step(&step_id),
            MutateParams::ApprovePlan { task_id } => self.approve_plan(&task_id).await.map(drop),
            MutateParams::TakeBackPlanApproval { task_id } => {
                self.take_back_plan_approval(&task_id).await
            }
            MutateParams::RetryTaskStep { step_id } => self.retry_task_step(&step_id).await,
            MutateParams::EditTaskPlan {
                task_id,
                overview,
                overview_images,
                steps,
                remove_step_ids,
                order,
            } => self
                .edit_task_plan(
                    &task_id,
                    overview,
                    overview_images,
                    steps,
                    &remove_step_ids,
                    &order,
                    None,
                )
                .await
                .map(drop),
            MutateParams::SetMissionSupervised {
                task_id,
                supervised,
            } => self.set_mission_supervised(&task_id, supervised),
            MutateParams::SetTreeWatched {
                root_chat_id,
                watched,
            } => self.set_tree_watched(&root_chat_id, watched),
            MutateParams::SetWatchEverything { on } => self.set_watch_everything(on),
            MutateParams::EscalateTask {
                task_id,
                from_chat_id,
                title,
                why,
                detail,
                options,
            } => self
                .escalate_task(
                    &task_id,
                    from_chat_id.as_deref(),
                    title,
                    why,
                    detail,
                    options,
                )
                .map(drop),
            MutateParams::ResolveSupervisorItem { item_id, choice } => {
                self.resolve_supervisor_items(vec![ItemChoice { item_id, choice }])
                    .await
            }
            MutateParams::ResolveSupervisorItems { answers } => {
                self.resolve_supervisor_items(answers).await
            }
        }
    }
}

/// An RPC rejection is scoped to the requested capability. Only a broken
/// transport means the shared device link itself cannot carry other calls.
fn should_invalidate_link(error: &RpcError) -> bool {
    matches!(error, RpcError::Closed | RpcError::Transport(_))
}

fn change_request_rpc_error(error: ChangeRequestError) -> RpcError {
    let code = match error {
        ChangeRequestError::CliUnavailable => capability_errors::PULL_REQUESTS_CLI_UNAVAILABLE,
        ChangeRequestError::Authentication => capability_errors::PULL_REQUESTS_AUTHENTICATION,
        ChangeRequestError::RateLimited => capability_errors::PULL_REQUESTS_RATE_LIMITED,
        ChangeRequestError::Timeout => capability_errors::PULL_REQUESTS_TIMEOUT,
        ChangeRequestError::Decode => capability_errors::PULL_REQUESTS_DECODE,
        ChangeRequestError::RepositoryUnavailable
        | ChangeRequestError::UnsupportedRepository
        | ChangeRequestError::CommandFailed => capability_errors::PULL_REQUESTS_COMMAND_FAILED,
    };
    RpcError::Capability(code.into())
}

fn action_rpc_error(error: crate::source_control::ActionError) -> RpcError {
    match error {
        crate::source_control::ActionError::Refused(message) => RpcError::Failed(message),
        crate::source_control::ActionError::Host(error) => change_request_rpc_error(error),
    }
}

/// Reply deadline for a relay-forwarded unary call. The relay is WebSocket
/// frames through a DO: a dropped frame (host socket replaced mid-call, DO
/// restart) loses the reply SILENTLY — the DO's auto-pong keeps the client
/// socket looking healthy — and an unbounded await wedged callers forever
/// (the composer's permanent "Sending…", 2026-08-18). Network-bound git and
/// update methods get a long leash; worktree creation checks out a full tree;
/// everything else is interactive and must fail fast.
#[derive(Default)]
pub(crate) struct Installations(
    std::sync::Mutex<std::collections::HashMap<HarnessId, zeron_harness::CancellationToken>>,
);

struct Installing<'a> {
    installs: &'a Installations,
    harness: HarnessId,
    cancel: zeron_harness::CancellationToken,
}
impl Drop for Installing<'_> {
    fn drop(&mut self) {
        self.cancel.cancel();
        self.installs
            .0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&self.harness);
    }
}
impl Installations {
    fn begin(&self, harness: HarnessId) -> Result<Installing<'_>, RpcError> {
        let mut installs = self.0.lock().unwrap_or_else(|e| e.into_inner());
        if installs.contains_key(&harness) {
            return Err(RpcError::Failed("already installing".into()));
        }
        let cancel = zeron_harness::CancellationToken::new();
        installs.insert(harness, cancel.clone());
        Ok(Installing {
            installs: self,
            harness,
            cancel,
        })
    }
    fn cancel(&self, harness: HarnessId) {
        if let Some(cancel) = self
            .0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(&harness)
        {
            cancel.cancel();
        }
    }
}

async fn run_requested_install(
    harness: HarnessId,
    cancel: zeron_harness::CancellationToken,
) -> Result<(), zeron_harness::HarnessError> {
    #[cfg(test)]
    if let Ok(script) = std::env::var(format!("ZERON_INSTALLER_COMMAND_{harness:?}").to_uppercase())
    {
        return zeron_harness::install::install_with_command(harness, &script, cancel).await;
    }
    zeron_harness::install::install_harness(harness, cancel).await
}

async fn install_harness_with<F, Fut>(
    registry: &HarnessRegistry,
    harness: HarnessId,
    install: F,
) -> Result<Vec<crate::registry::HarnessDescriptor>, RpcError>
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = Result<(), zeron_harness::HarnessError>>,
{
    if !zeron_harness::install::can_install(harness) {
        return Err(RpcError::Failed(
            "No supported installer or required tools available on this device".into(),
        ));
    }
    install()
        .await
        .map_err(|error| RpcError::Failed(error.to_string()))?;
    Ok(registry.descriptors())
}

async fn read_tree_timed(
    workspace: &crate::workspace_files::ResolvedWorkspace,
) -> Result<zeron_proto::WorkingTree, RpcError> {
    tokio::time::timeout(
        crate::working_tree::READ_TIMEOUT,
        crate::working_tree::read(&workspace.root, &workspace.checkout_id),
    )
    .await
    .map_err(|_| RpcError::Failed("working tree read timed out".into()))?
    .map_err(|error| RpcError::Failed(error.to_string()))
}

fn forward_deadline(method: &str) -> std::time::Duration {
    if method == methods::PREPARE_VOICE_V2 {
        return Duration::from_secs(65);
    }
    if method == methods::NEGOTIATE_VOICE_V2 {
        return Duration::from_secs(95);
    }
    use std::time::Duration;
    match method {
        methods::CLONE_REPO | methods::FETCH_ALL | methods::APPLY_UPDATE | methods::SYNC_BRANCH => {
            Duration::from_secs(15 * 60)
        }
        methods::INSTALL_HARNESS | methods::PREPARE_ACP_AGENT => Duration::from_secs(15 * 60),
        // A full fleet of enabled providers is checked two at a time; each
        // provider may need both a CLI probe and a network request.
        methods::CHECK_HARNESS_UPDATES => Duration::from_secs(4 * 60),
        // Leave headroom beyond the provider's 15-minute mutation timeout for
        // queueing, verification, and the relayed response itself.
        methods::APPLY_HARNESS_UPDATE => Duration::from_secs(20 * 60),
        methods::CREATE_WORKTREE => Duration::from_secs(120),
        // Outlives the engine's own write timeout, which covers commit hooks.
        methods::RUN_WORKING_TREE_ACTION => Duration::from_secs(180),
        // Up to three sequential gh calls, each under the 20-second GitHub timeout.
        methods::MERGE_CHANGE_REQUEST
        | methods::SET_CHANGE_REQUEST_LABEL
        | methods::SET_CHANGE_REQUEST_REVIEWER
        | methods::SUBMIT_CHANGE_REQUEST_REVIEW
        | methods::REPLY_TO_CHANGE_REQUEST_THREAD
        | methods::SET_CHANGE_REQUEST_THREAD_RESOLVED => Duration::from_secs(90),
        // Outlives the engine's own compaction timeout, so a forwarded call
        // reports the harness's outcome rather than a transport timeout.
        methods::COMPACT_CHAT => Duration::from_secs(660),
        // Allow the adapter discovery budget plus relay and shutdown overhead.
        methods::LIST_MODELS
        | methods::LIST_COMMANDS
        | methods::LIST_HARNESS_TOOLS
        | methods::GENERATE_COMMIT_MESSAGE => Duration::from_secs(100),
        _ => Duration::from_secs(30),
    }
}

/// A handoff divider nothing followed: the chat's newest entry is a system
/// note carrying a packet whose route names the harness the chat left.
struct SwitchNote {
    id: String,
    text: String,
    hint: Option<String>,
    packet: String,
    route: zeron_doc::HandoffRoute,
}

fn unanswered_switch_note(handle: &crate::doc_host::ChatDocHandle) -> Option<SwitchNote> {
    let entries = handle.doc().read_entries().ok()?;
    let last = entries.last()?;
    if last.role != zeron_doc::MessageRole::System {
        return None;
    }
    let (packet, route) = last.parts.iter().find_map(|part| match part {
        MessagePart::Handoff { text, route, .. } if route.from_harness.is_some() => {
            Some((text.clone(), route.clone()))
        }
        _ => None,
    })?;
    let mut texts = last.parts.iter().filter_map(|part| match part {
        MessagePart::Text { text, .. } => Some(text.clone()),
        _ => None,
    });
    Some(SwitchNote {
        id: last.id.clone(),
        text: texts.next().unwrap_or_default(),
        hint: texts.next(),
        packet,
        route,
    })
}

/// The divider of a change folded onto an unanswered handoff: it keeps the
/// origin the first divider recorded and takes the new destination. `None`
/// when that is where the chat already was. A seeded thread's divider is its
/// only copy of the source conversation, so it is never dropped.
fn fold_switch_note(
    previous: &SwitchNote,
    between: HarnessId,
    config: &zeron_proto::ChatConfig,
    next: &SystemNote,
) -> Option<(SystemNote, zeron_doc::HandoffRoute)> {
    let origin = previous.route.from_harness?;
    let route = zeron_doc::HandoffRoute {
        to_model: config.model.clone(),
        ..previous.route.clone()
    };
    let seeded = route.source_chat_id.is_some();
    if !seeded && config.harness == origin && config.model == route.from_model {
        return None;
    }
    let note = if seeded || between == config.harness {
        SystemNote {
            text: previous.text.clone(),
            hint: previous.hint.clone(),
        }
    } else if config.harness == origin {
        SystemNote {
            text: format!(
                "Model changed from {} to {}.",
                route.from_model.as_deref().unwrap_or("default"),
                route.to_model.as_deref().unwrap_or("default"),
            ),
            hint: None,
        }
    } else {
        SystemNote {
            text: format!(
                "Provider changed from {} to {}.",
                origin.display_name(),
                config.harness.display_name()
            ),
            hint: next
                .hint
                .as_ref()
                .map(|hint| hint.replace(between.display_name(), origin.display_name())),
        }
    };
    Some((note, route))
}
/// How long this device forwards a remote login's callback at most — the
/// running engine reaps an abandoned login after the same 15 minutes.
const LOGIN_TUNNEL_TTL: Duration = Duration::from_secs(15 * 60);

/// ControlRpc methods that honor `targetDeviceId` (feature-inventory §2.1). Extend this
/// list (plus [`is_stream_method`] for streams) to make more of the surface
/// device-addressable — the handlers themselves need no changes.
fn forwardable(method: &str) -> bool {
    if voice_remote::handles(method) {
        return true;
    }
    matches!(
        method,
        methods::FORK_SIDE_CHAT
            | methods::CONTINUE_IN_NEW_THREAD
            | methods::LIST_HARNESSES
            | methods::INSTALL_HARNESS
            | methods::CANCEL_INSTALL
            | methods::GET_TITLE_SETTINGS
            | methods::SET_TITLE_SETTINGS
            | methods::GET_PREDICTION_SETTINGS
            | methods::SET_PREDICTION_SETTINGS
            | methods::GET_DELEGATION_SETTINGS
            | methods::SET_DELEGATION_SETTINGS
            | methods::GET_ROUTING_SETTINGS
            | methods::SET_ROUTING_SETTINGS
            | methods::GET_COMPACTION_SETTINGS
            | methods::SET_COMPACTION_SETTINGS
            | methods::GET_REVIEW_SETTINGS
            | methods::SET_REVIEW_SETTINGS
            | methods::GET_SIDEBAR_SETTINGS
            | methods::START_REVIEW
            | methods::GET_TOOL_SETTINGS
            | methods::SET_TOOL_SETTINGS
            | methods::SET_HARNESS_ENABLED
            | methods::LIST_MODELS
            | methods::LIST_SKILLS
            | methods::LIST_COMMANDS
            | methods::LIST_HARNESS_TOOLS
            | methods::HARNESS_COMPACTION
            | methods::COMPACT_CHAT
            | methods::REQUEST_COMPACT
            | methods::STOP_BACKGROUND_TASK
            | methods::WATCH_BACKGROUND_OUTPUT
            | methods::WATCH_TOKEN_RATE
            | methods::WATCH_COMPOSER_PREDICTION
            | methods::FORK_CHAT
            | methods::REWIND_CHAT
            | methods::QUEUE_COMMAND
            | methods::TAKE_PROJECT_ACTION_SETUP
            | methods::WATCH_DOC_MESSAGES
            // The queue lives on the chat doc, and only its host may send from
            // it — same addressing as the command ledger next door.
            | methods::WATCH_QUEUE
            | methods::QUEUE_MESSAGE
            | methods::UPDATE_QUEUED_MESSAGE
            | methods::BEGIN_QUEUED_MESSAGE_EDIT
            | methods::RENEW_QUEUED_MESSAGE_EDIT
            | methods::FINISH_QUEUED_MESSAGE_EDIT
            | methods::MOVE_QUEUED_MESSAGE
            | methods::REMOVE_QUEUED_MESSAGE
            | methods::SEND_QUEUED_MESSAGE_NOW
            | methods::STEER_QUEUED_MESSAGE_NOW
            // Repos/worktrees/folders are device-local filesystem state.
            | methods::LIST_REPOS
            | methods::ADD_REPO
            | methods::CLONE_REPO
            | methods::CREATE_REPO
            | methods::LIST_BRANCHES
            | methods::LIST_REFS
            | methods::LIST_GIT_HISTORY
            | methods::SEARCH_GIT_HISTORY
            | methods::RESOLVE_GIT_AVATARS
            | methods::FETCH_ALL
            | methods::SWITCH_REF
            | methods::LIST_FOLDERS
            | methods::GET_WORKTREE_SETTINGS
            | methods::SET_WORKTREE_SETTINGS
            | methods::LIST_DRIVES
            | methods::SEARCH_HOME_FOLDERS
            | methods::RELEASE_HOME_FOLDER_SEARCH
            | methods::SEARCH_FILES
            | methods::LIST_WORKSPACE_DIRECTORY
            | methods::SEARCH_WORKSPACE_FILES
            | methods::WARM_WORKSPACE_SEARCH
            | methods::SEARCH_WORKSPACE_CONTENT
            | methods::READ_WORKSPACE_IMAGE
            | methods::READ_WORKSPACE_FILE
            | methods::DELETE_WORKSPACE_ENTRY
            | methods::MOVE_WORKSPACE_ENTRY
            | methods::WRITE_WORKSPACE_FILE
            | methods::WATCH_WORKSPACE_FILES
            | methods::CREATE_WORKTREE
            | methods::DELETE_WORKTREE
            // Project Actions live in the owning engine's private profile store.
            | methods::LIST_PROJECT_ACTIONS
            | methods::UPSERT_PROJECT_ACTION
            | methods::DELETE_PROJECT_ACTION
            | methods::RUN_PROJECT_ACTION
            | methods::LIST_AGENT_PROFILES
            | methods::UPSERT_AGENT_PROFILE
            | methods::DELETE_AGENT_PROFILE
            | methods::REORDER_AGENT_PROFILES
            // Provider instances name paths on the device that runs them.
            | methods::LIST_PROVIDER_INSTANCES
            | methods::WATCH_PROVIDER_INSTANCES
            | methods::UPSERT_PROVIDER_INSTANCE
            | methods::DELETE_PROVIDER_INSTANCE
            | methods::LIST_ACP_AUTH_METHODS
            | methods::SEARCH_ACP_REGISTRY
            | methods::PREPARE_ACP_AGENT
            | methods::GET_SYSTEM_PROMPT
            | methods::SET_SYSTEM_PROMPT
            // Checkout diffs are produced on the device holding the checkout.
            | methods::WATCH_CHECKOUT_DIFFS
            | methods::WATCH_WORKSPACE_GIT_STATUS
            | methods::WATCH_CHECKOUT_CHANGE_REQUEST
            | methods::LIST_CHANGE_REQUEST_PAGE
            | methods::GET_CHANGE_REQUEST_REPOSITORY
            | methods::GET_CHANGE_REQUEST
            | methods::GET_CHANGE_REQUEST_DIFF
            | methods::POST_CHANGE_REQUEST_COMMENT
            | methods::GET_CHANGE_REQUEST_CONTROLS
            | methods::MERGE_CHANGE_REQUEST
            | methods::LIST_CHANGE_REQUEST_LABEL_CANDIDATES
            | methods::SET_CHANGE_REQUEST_LABEL
            | methods::LIST_CHANGE_REQUEST_REVIEWER_CANDIDATES
            | methods::SET_CHANGE_REQUEST_REVIEWER
            | methods::SUBMIT_CHANGE_REQUEST_REVIEW
            | methods::REPLY_TO_CHANGE_REQUEST_THREAD
            | methods::SET_CHANGE_REQUEST_THREAD_RESOLVED
            | methods::GET_CHECKOUT_DIFF
            | methods::DISCARD_WORKING_TREE
            | methods::GET_CHECKOUT_FILE_DIFF_TEXT
            | methods::GET_WORKING_TREE
            | methods::GET_WORKING_TREE_FILE_DIFF
            | methods::RUN_WORKING_TREE_ACTION
            | methods::SYNC_BRANCH
            | methods::GENERATE_COMMIT_MESSAGE
            // Terminals live on the chat's host device.
            | methods::OPEN_TERMINAL
            | methods::SUBSCRIBE_TERMINAL
            | methods::WRITE_TERMINAL
            | methods::RESIZE_TERMINAL
            | methods::CLOSE_TERMINAL
            | methods::LIST_TERMINALS
            | methods::READ_TERMINAL
            | methods::SEND_TERMINAL_INPUT
            | methods::SET_TERMINAL_SELECTION
            // Agent accounts are per-device CLI logins (the device switcher
            // retargets which device's logins are shown).
            | methods::LIST_AGENT_ACCOUNTS
            | methods::ACTIVATE_AGENT_ACCOUNT
            | methods::FORGET_AGENT_ACCOUNT
            | methods::START_AGENT_LOGIN
            | methods::COMPLETE_AGENT_LOGIN
            | methods::POLL_AGENT_LOGIN
            | methods::CANCEL_AGENT_LOGIN
            // Uploads/attachments target the chat's host device (the agent reads
            // the committed file from that device's disk).
            | methods::UPLOAD_CHUNK
            | methods::UPLOAD_COMMIT
            | methods::READ_ATTACHMENT_CHUNK
            // Updates report/apply on the device whose binary they concern.
            | methods::UPDATE_STATUS
            | methods::APPLY_UPDATE
            | methods::WATCH_HARNESS_UPDATES
            | methods::CHECK_HARNESS_UPDATES
            | methods::APPLY_HARNESS_UPDATE
            | methods::CANCEL_HARNESS_UPDATE
            | methods::DISMISS_HARNESS_UPDATE
            | methods::SET_HARNESS_UPDATE_POLICY
            | methods::UPDATE_HARNESSES
            | methods::PREPARE_RESTART
            | methods::WATCH_LIMIT_RESUMES
            | methods::SET_LIMIT_RESUME
            | methods::SET_AUTO_RESUME_LIMITS
    )
}

/// Forwardable methods whose reply is a stream (proxied item-by-item).
fn is_stream_method(method: &str) -> bool {
    if method == methods::OWN_VOICE_V2 {
        return true;
    }
    matches!(
        method,
        methods::WATCH_DOC_MESSAGES
            | methods::WATCH_QUEUE
            | methods::WATCH_BACKGROUND_OUTPUT
            | methods::WATCH_TOKEN_RATE
            | methods::WATCH_COMPOSER_PREDICTION
            | methods::SUBSCRIBE_TERMINAL
            | methods::WATCH_CHECKOUT_DIFFS
            | methods::WATCH_WORKSPACE_GIT_STATUS
            | methods::WATCH_CHECKOUT_CHANGE_REQUEST
            | methods::WATCH_WORKSPACE_FILES
            | methods::UPDATE_STATUS
            | methods::WATCH_HARNESS_UPDATES
            | methods::WATCH_LIMIT_RESUMES
            | methods::WATCH_PROVIDER_INSTANCES
    )
}

/// A watch receiver as a stream: current value first, then every change.
fn watch_stream<T>(rx: watch::Receiver<T>) -> BoxStream<'static, serde_json::Value>
where
    T: serde::Serialize + Clone + Send + Sync + 'static,
{
    futures::stream::unfold((rx, false), |(mut rx, emitted)| async move {
        if emitted {
            rx.changed().await.ok()?;
        }
        let value = {
            let borrowed = rx.borrow_and_update();
            serde_json::to_value(&*borrowed).ok()?
        };
        Some((value, (rx, true)))
    })
    .boxed()
}

/// The transcript watch as delta frames (`zeron_doc::transcript_delta`): a
/// full `reset` first, then only changed entries per commit — the whole-Vec
/// serialization here was the per-tick cost that scaled with transcript size.
fn doc_messages_stream(
    rx: watch::Receiver<crate::doc_host::TranscriptSnapshot>,
    doc: std::sync::Arc<zeron_doc::SessionDoc>,
) -> BoxStream<'static, serde_json::Value> {
    use zeron_doc::transcript_delta::{TranscriptFrame, diff_transcript};
    futures::stream::unfold(
        (
            rx,
            None::<crate::doc_host::TranscriptSnapshot>,
            doc,
            (None, None),
            zeron_doc::TranscriptBaseline::default(),
        ),
        |(mut rx, mut prev, doc, mut previous_usage, mut opening_baseline)| async move {
            loop {
                if prev.is_some() {
                    rx.changed().await.ok()?;
                }
                // Watchers retain the immutable published snapshot. Each
                // connection used to deep-copy the entire transcript here.
                let current = rx.borrow_and_update().clone();
                let frame = match prev.as_ref() {
                    None => TranscriptFrame::reset(&current.entries),
                    Some(prev) => diff_transcript(&prev.entries, &current.entries),
                };
                let replay_baseline = match prev.as_ref() {
                    None => {
                        opening_baseline = zeron_doc::TranscriptBaseline::capture(&current.entries);
                        Some(opening_baseline.clone())
                    }
                    Some(prev)
                        if !std::sync::Arc::ptr_eq(
                            &prev.replay_baseline,
                            &current.replay_baseline,
                        ) =>
                    {
                        // The tracker only observes changes after attach; its
                        // baseline omits unchanged cached parts. Preserve this
                        // subscription's opening cutoff without capturing live
                        // appends or sharing another viewer's later cutoff.
                        // Ordinary live updates never rebuild this metadata.
                        let mut baseline = (*current.replay_baseline).clone();
                        for (entry, parts) in &opening_baseline.entries {
                            let merged = baseline.entries.entry(entry.clone()).or_default();
                            for (part, &len) in parts {
                                let cutoff = merged.entry(part.clone()).or_default();
                                *cutoff = (*cutoff).max(len);
                            }
                        }
                        Some(baseline)
                    }
                    _ => None,
                };
                prev = Some(current);
                // No-op commits (a second watcher attaching, command-only
                // changes) produce empty deltas — skip the frame entirely.
                let usage = (doc.context_usage(), doc.session_stats());
                if frame.is_empty_delta() && usage == previous_usage && replay_baseline.is_none() {
                    continue;
                }
                previous_usage = usage;
                let value = serde_json::to_value(zeron_doc::TranscriptUpdate {
                    frame,
                    context_usage: usage.0,
                    session_stats: usage.1,
                    replay_baseline,
                })
                .ok()?;
                return Some((value, (rx, prev, doc, previous_usage, opening_baseline)));
            }
        },
    )
    .boxed()
}

/// First paint reads only the local tail; full history is deferred until the
/// next stream poll. No network dependency or persisted truncation.
async fn opening_doc_messages_stream(
    host: crate::doc_host::DocHost,
    chat_id: String,
) -> Result<BoxStream<'static, serde_json::Value>, RpcError> {
    let (handle, preview) = tokio::task::spawn_blocking(move || {
        let handle = host.open(&chat_id)?;
        let entries = handle.doc().read_opening_tail(128)?;
        let mut preview = serde_json::to_value(zeron_doc::TranscriptUpdate {
            frame: zeron_doc::TranscriptFrame::reset(&entries),
            context_usage: handle.doc().context_usage(),
            session_stats: handle.doc().session_stats(),
            replay_baseline: Some(zeron_doc::TranscriptBaseline::capture(&entries)),
        })
        .map_err(|e| crate::EngineError::Other(e.to_string()))?;
        preview["historyPending"] = serde_json::Value::Bool(true);
        Ok::<_, crate::EngineError>((handle, preview))
    })
    .await
    .map_err(|e| RpcError::Failed(e.to_string()))?
    .map_err(|e| RpcError::Failed(e.to_string()))?;
    // Do not build the full mirror before yielding the preview.
    // The next poll attaches normally and begins with a complete
    // authoritative reset; subsequent frames use normal deltas.
    let full = futures::stream::once(async move {
        match tokio::task::spawn_blocking(move || (handle.watch_messages(), handle.doc_arc())).await
        {
            Ok((rx, doc)) => doc_messages_stream(rx, doc),
            Err(error) => {
                tracing::warn!(%error, "transcript opening failed");
                futures::stream::empty().boxed()
            }
        }
    })
    .flatten();
    Ok(futures::stream::once(async move { preview })
        .chain(full)
        .boxed())
}

/// Authentication-only RPC surface used while the headed app is waiting for a
/// production WorkOS session. Keeping this independent from [`EngineRpc`] lets
/// the UI show its sign-in and organization gates before identity-scoped Loro
/// stores are opened.
#[derive(Clone)]
pub struct AuthRpc {
    auth: Auth,
}

impl AuthRpc {
    pub fn new(auth: Auth) -> Self {
        Self { auth }
    }

    pub fn handles(method: &str) -> bool {
        matches!(
            method,
            methods::AUTH_STATUS
                | methods::SIGN_IN
                | methods::SIGN_IN_HEADLESS
                | methods::COMPLETE_SIGN_IN
                | methods::SIGN_OUT
                | methods::LIST_ORGS
                | methods::CREATE_ORG
                | methods::SELECT_ORG
        )
    }
}

#[async_trait]
impl RpcService for AuthRpc {
    async fn handle(&self, method: &str, params: serde_json::Value) -> Result<RpcReply, RpcError> {
        match method {
            methods::AUTH_STATUS => Ok(RpcReply::Stream(watch_stream(self.auth.watch_state()))),
            methods::SIGN_IN => {
                let url = self
                    .auth
                    .start_sign_in()
                    .await
                    .map_err(|e| RpcError::Failed(e.to_string()))?;
                RpcReply::value(&serde_json::json!({ "url": url }))
            }
            methods::SIGN_IN_HEADLESS => {
                let url = self.auth.start_headless_sign_in();
                RpcReply::value(&serde_json::json!({ "url": url }))
            }
            methods::COMPLETE_SIGN_IN => {
                #[derive(Deserialize)]
                struct P {
                    code: String,
                }
                let p: P = parse_params(params)?;
                self.auth
                    .complete_sign_in(&p.code)
                    .await
                    .map_err(|e| RpcError::Failed(e.to_string()))?;
                RpcReply::value(&serde_json::json!({ "ok": true }))
            }
            methods::SIGN_OUT => {
                self.auth.sign_out();
                RpcReply::value(&serde_json::json!({ "ok": true }))
            }
            methods::LIST_ORGS => {
                let orgs = self
                    .auth
                    .list_orgs()
                    .await
                    .map_err(|e| RpcError::Failed(e.to_string()))?;
                RpcReply::value(&serde_json::json!({ "orgs": orgs }))
            }
            methods::CREATE_ORG => {
                #[derive(Deserialize)]
                struct P {
                    name: String,
                }
                let p: P = parse_params(params)?;
                self.auth
                    .create_org(&p.name)
                    .await
                    .map_err(|e| RpcError::Failed(e.to_string()))?;
                RpcReply::value(&serde_json::json!({ "ok": true }))
            }
            methods::SELECT_ORG => {
                #[derive(Deserialize)]
                #[serde(rename_all = "camelCase")]
                struct P {
                    organization_id: String,
                }
                let p: P = parse_params(params)?;
                self.auth
                    .select_org(&p.organization_id)
                    .await
                    .map_err(|e| RpcError::Failed(e.to_string()))?;
                RpcReply::value(&serde_json::json!({ "ok": true }))
            }
            _ => Err(RpcError::UnknownMethod(method.to_string())),
        }
    }
}

#[async_trait]
impl RpcService for EngineRpc {
    async fn handle(&self, method: &str, params: serde_json::Value) -> Result<RpcReply, RpcError> {
        // Device-addressed routing: forward calls that target another device over its
        // relay. The target compares the id to its own, so forwards cannot loop.
        if forwardable(method)
            && let Some(target) = params.get("targetDeviceId").and_then(|v| v.as_str())
            && target != self.doc_host.device_id()
        {
            let target = target.to_string();
            if matches!(
                method,
                methods::START_AGENT_LOGIN
                    | methods::POLL_AGENT_LOGIN
                    | methods::COMPLETE_AGENT_LOGIN
                    | methods::CANCEL_AGENT_LOGIN
            ) {
                return self.forward_agent_login(&target, method, params).await;
            }
            return self.forward(&target, method, params).await;
        }
        if matches!(
            method,
            methods::SIGN_OUT
                | methods::SELECT_ORG
                | methods::COMPLETE_AGENT_LOGIN
                | methods::ACTIVATE_AGENT_ACCOUNT
                | methods::FORGET_AGENT_ACCOUNT
        ) {
            self.voice.retire();
        }
        if voice_remote::handles(method) {
            return self.voice_remote(method, params).await;
        }
        if AuthRpc::handles(method) {
            return AuthRpc::new(self.auth()?.clone())
                .handle(method, params)
                .await;
        }
        match method {
            methods::ENGINE_INFO => RpcReply::value(&self.engine_info),
            methods::ENGINE_READY => RpcReply::value(&serde_json::json!({ "ready": true })),
            methods::LIST_HARNESSES => RpcReply::value(&self.registry.descriptors()),
            methods::INSTALL_HARNESS => {
                let p: ListModelsParams = parse_params(params)?;
                let installing = self.registry.installs.begin(p.harness)?;
                let descriptors = install_harness_with(&self.registry, p.harness, || {
                    run_requested_install(p.harness, installing.cancel.clone())
                })
                .await?;
                RpcReply::value(&descriptors)
            }
            methods::CANCEL_INSTALL => {
                let p: ListModelsParams = parse_params(params)?;
                self.registry.installs.cancel(p.harness);
                RpcReply::value(&serde_json::json!({}))
            }
            methods::GET_TITLE_SETTINGS => RpcReply::value(&self.registry.title_settings()),
            methods::SET_TITLE_SETTINGS => {
                let p: crate::registry::TitleSettings = parse_params(params)?;
                self.registry
                    .set_title_settings(p)
                    .map_err(RpcError::Failed)?;
                RpcReply::value(&self.registry.title_settings())
            }
            methods::GET_PREDICTION_SETTINGS => {
                RpcReply::value(&self.sessions.prediction_settings().await)
            }
            methods::SET_PREDICTION_SETTINGS => {
                let p: crate::predictions::PredictionSettingsUpdate = parse_params(params)?;
                let state = self
                    .sessions
                    .update_prediction_settings(p)
                    .await
                    .map_err(RpcError::Failed)?;
                RpcReply::value(&state)
            }
            methods::GET_DELEGATION_SETTINGS => {
                RpcReply::value(&self.registry.delegation_settings())
            }
            methods::GET_REVIEW_SETTINGS => RpcReply::value(&self.registry.review_settings()),
            methods::GET_ROUTING_SETTINGS => RpcReply::value(&self.registry.routing_settings()),
            methods::SET_ROUTING_SETTINGS => {
                let p: crate::routing::RoutingSettings = parse_params(params)?;
                self.registry
                    .set_routing_settings(p)
                    .map_err(RpcError::Failed)?;
                RpcReply::value(&self.registry.routing_settings())
            }
            methods::GET_SIDEBAR_SETTINGS => RpcReply::value(&self.registry.sidebar_settings()),
            methods::SET_REVIEW_SETTINGS => {
                let p: crate::review::ReviewSettings = parse_params(params)?;
                self.registry
                    .set_review_settings(p)
                    .map_err(RpcError::Failed)?;
                RpcReply::value(&self.registry.review_settings())
            }
            methods::START_REVIEW => {
                let p: StartReviewParams = parse_params(params)?;
                RpcReply::value(&Box::pin(self.start_review(p)).await?)
            }
            methods::SET_DELEGATION_SETTINGS => {
                let p: crate::registry::DelegationSettings = parse_params(params)?;
                self.registry
                    .set_delegation_settings(p)
                    .map_err(RpcError::Failed)?;
                RpcReply::value(&self.registry.delegation_settings())
            }
            methods::GET_COMPACTION_SETTINGS => {
                RpcReply::value(&crate::jev::settings_state(&self.registry).await)
            }
            methods::SET_COMPACTION_SETTINGS => {
                let p: crate::jev::CompactionSettingsUpdate = parse_params(params)?;
                let state = crate::jev::update_settings(&self.registry, p)
                    .await
                    .map_err(RpcError::Failed)?;
                RpcReply::value(&state)
            }
            methods::GET_TOOL_SETTINGS => {
                let p: GetToolSettingsParams = if params.is_null() {
                    GetToolSettingsParams::default()
                } else {
                    parse_params(params)?
                };
                RpcReply::value(&self.tool_settings_state(p.chat_id.as_deref()))
            }
            methods::SET_TOOL_SETTINGS => {
                let p: zeron_proto::tools::ToolSettings = parse_params(params)?;
                self.registry.set_tool_settings(p);
                RpcReply::value(&self.tool_settings_state(None))
            }
            methods::SET_HARNESS_ENABLED => {
                let p: SetHarnessEnabledParams = parse_params(params)?;
                update_harness_enabled(&self.registry, p.harness, p.enabled).await?;
                if let Some(coordinator) = &self.harness_updates {
                    coordinator.refresh_enabled();
                }
                // Fresh catalog in the reply: the page repaints from it in one
                // round trip, and a refused/raced toggle self-corrects.
                RpcReply::value(&self.registry.descriptors())
            }
            methods::LIST_MODELS => {
                let p: ListModelsParams = parse_params(params)?;
                let harness = match &p.instance_id {
                    Some(id) => self
                        .provider_instances_store()?
                        .resolve(&self.registry, p.harness, id)
                        .map_err(|e| RpcError::Failed(e.to_string()))?,
                    None => self
                        .registry
                        .resolve(p.harness)
                        .map_err(|e| RpcError::Failed(e.to_string()))?,
                };
                let lease = std::sync::Arc::new(self.registry.execution_lease(p.harness).await);
                let models = crate::model_catalogs::list_with_lease(
                    self.repos.data_dir(),
                    harness,
                    p.force,
                    Some(lease),
                )
                .await
                .map_err(|e| RpcError::Failed(e.to_string()))?;
                // The seen-models record is per driver; an instance's models must not mark them seen.
                if p.instance_id.is_some() {
                    return RpcReply::value(&zeron_proto::ModelCatalog {
                        models,
                        new_ids: Vec::new(),
                        announce_ids: Vec::new(),
                    });
                }
                RpcReply::value(
                    &self
                        .registry
                        .model_catalog(p.harness, models, crate::now_ms()),
                )
            }
            methods::LIST_SKILLS => {
                #[derive(Deserialize)]
                #[serde(rename_all = "camelCase")]
                struct Params {
                    harness: HarnessId,
                    #[serde(default)]
                    chat_id: Option<String>,
                    #[serde(default)]
                    space_id: Option<String>,
                    #[serde(default)]
                    path: Option<String>,
                }
                let p: Params = parse_params(params)?;
                let harness = self.discovery_harness(p.chat_id.as_deref(), p.harness)?;
                let root = self
                    .catalog_root(&FileSearchParams {
                        query: String::new(),
                        chat_id: p.chat_id,
                        space_id: p.space_id,
                        path: p.path,
                    })
                    .await?;
                let skills = self
                    .registry
                    .discover_skills(harness, &root)
                    .await
                    .map_err(|e| RpcError::Failed(e.to_string()))?;
                RpcReply::value(&skills)
            }
            methods::LIST_COMMANDS => {
                #[derive(Deserialize)]
                #[serde(rename_all = "camelCase")]
                struct Params {
                    harness: HarnessId,
                    #[serde(default)]
                    chat_id: Option<String>,
                    #[serde(default)]
                    space_id: Option<String>,
                    #[serde(default)]
                    path: Option<String>,
                }
                let p: Params = parse_params(params)?;
                let harness = self.discovery_harness(p.chat_id.as_deref(), p.harness)?;
                let root = self
                    .catalog_root(&FileSearchParams {
                        query: String::new(),
                        chat_id: p.chat_id,
                        space_id: p.space_id,
                        path: p.path,
                    })
                    .await?;
                let commands = self
                    .registry
                    .discover_commands(harness, &root)
                    .await
                    .map_err(|e| RpcError::Failed(e.to_string()))?;
                RpcReply::value(&commands)
            }
            methods::LIST_HARNESS_TOOLS => {
                #[derive(Deserialize)]
                #[serde(rename_all = "camelCase")]
                struct Params {
                    harness: HarnessId,
                    #[serde(default)]
                    space_id: Option<String>,
                }
                let p: Params = parse_params(params)?;
                let root = self
                    .catalog_root(&FileSearchParams {
                        query: String::new(),
                        chat_id: None,
                        space_id: p.space_id,
                        path: None,
                    })
                    .await?;
                let inventory = self
                    .registry
                    .discover_tool_inventory(p.harness, &root)
                    .await
                    .map_err(|e| RpcError::Failed(e.to_string()))?;
                RpcReply::value(&inventory)
            }
            methods::HARNESS_COMPACTION => {
                let p: zeron_proto::HarnessCompactionParams = parse_params(params)?;
                let harness = self
                    .registry
                    .resolve(p.harness)
                    .map_err(|e| RpcError::Failed(e.to_string()))?;
                RpcReply::value(&zeron_proto::HarnessCompactionResult {
                    compaction: harness.compaction().into(),
                    branching: harness.branching(),
                })
            }
            methods::STOP_BACKGROUND_TASK => {
                let p: BackgroundTaskParams = parse_params(params)?;
                self.sessions
                    .stop_background_task(&p.chat_id, &p.task_id)
                    .map_err(|e| RpcError::Failed(e.to_string()))?;
                RpcReply::value(&serde_json::json!({ "ok": true }))
            }
            methods::WATCH_BACKGROUND_OUTPUT => {
                let p: BackgroundTaskParams = parse_params(params)?;
                let stream =
                    crate::background_output::watch(self.sessions.clone(), p.chat_id, p.task_id)
                        .map_err(|e| RpcError::Failed(e.to_string()))?;
                Ok(RpcReply::Stream(stream))
            }
            methods::WATCH_TOKEN_RATE => {
                let p: ChatParams = parse_params(params)?;
                let rx = self.sessions.watch_token_rate(&p.chat_id);
                Ok(RpcReply::Stream(
                    watch_stream(rx)
                        .map(|rate| serde_json::json!({ "rate": rate }))
                        .boxed(),
                ))
            }
            methods::WATCH_COMPOSER_PREDICTION => {
                let p: ChatParams = parse_params(params)?;
                let rx = self.sessions.watch_prediction(&p.chat_id);
                Ok(RpcReply::Stream(
                    watch_stream(rx)
                        .map(|prediction| serde_json::json!({ "prediction": prediction }))
                        .boxed(),
                ))
            }
            methods::COMPACT_CHAT => {
                let p: CompactChatParams = parse_params(params)?;
                let outcome = self
                    .sessions
                    .compact(&p.chat_id, p.focus)
                    .await
                    .map_err(|e| RpcError::Failed(e.to_string()))?;
                RpcReply::value(&outcome)
            }
            methods::MID_TURN_NUDGE => {
                let p: ChatParams = parse_params(params)?;
                RpcReply::value(&serde_json::json!({ "text": self.sessions.mid_turn_nudge(&p.chat_id) }))
            }
            methods::REQUEST_COMPACT => {
                let p: RequestCompactParams = parse_params(params)?;
                let queued = self
                    .sessions
                    .request_compact(&p.chat_id, p.focus, p.continue_with)
                    .map_err(|e| RpcError::Failed(e.to_string()))?;
                RpcReply::value(&queued)
            }
            methods::FORK_CHAT => {
                let p: ForkChatParams = parse_params(params)?;
                let chat = self
                    .sessions
                    .fork_chat(&p.source_chat_id, &p.entry_id, &p.chat_id)
                    .map_err(|e| RpcError::Failed(e.to_string()))?;
                RpcReply::value(&chat)
            }
            methods::REWIND_CHAT => {
                let p: RewindChatParams = parse_params(params)?;
                let outcome = self
                    .sessions
                    .rewind_chat(&p.chat_id, &p.entry_id)
                    .await
                    .map_err(|e| RpcError::Failed(e.to_string()))?;
                RpcReply::value(&outcome)
            }
            methods::CREATE_SIDECHAT => {
                let p: CreateSidechatParams = parse_params(params)?;
                let chat = self
                    .workspace
                    .create_sidechat(&p.source_chat_id, &p.chat_id)
                    .map_err(|e| RpcError::Failed(e.to_string()))?;
                RpcReply::value(&chat)
            }
            methods::CONTINUE_IN_NEW_THREAD => {
                let p: ContinueInNewThreadParams = parse_params(params)?;
                let packet = self
                    .sessions
                    .provider_switch_packet(&p.source_chat_id)
                    .unwrap_or_default();
                let chat = self
                    .doc_host
                    .continue_in_new_thread(
                        &p.source_chat_id,
                        &p.chat_id,
                        p.harness,
                        p.model.as_deref(),
                        p.reasoning,
                        &p.profile,
                        p.target_space_id.as_deref(),
                        &packet,
                        |source, title, config| {
                            self.sessions.handoff_pointer(source, title, config)
                        },
                    )
                    .map_err(|e| RpcError::Failed(e.to_string()))?;
                RpcReply::value(&chat)
            }
            methods::CREATE_THREAD => {
                let p: CreateThreadParams = parse_params(params)?;
                self.supervisor_gate(Some(&p.from_chat_id), |s| s.start_missions, supervisor::START_OFF)?;
                let chat_id = self
                    .doc_host
                    .create_step_thread(
                        &p.from_chat_id,
                        &p.prompt,
                        p.title.as_deref(),
                        p.harness,
                        p.model.as_deref(),
                        p.reasoning,
                        &p.profile,
                        p.space_id.as_deref(),
                        p.cwd.as_deref(),
                        p.task_step_id
                            .as_deref()
                            .map(|id| (id, self.registry.delegation_settings())),
                    )
                    .await
                    .map_err(|e| RpcError::Failed(e.to_string()))?;
                RpcReply::value(&serde_json::json!({ "chatId": chat_id }))
            }
            methods::SEND_THREAD_MESSAGE => {
                let p: SendThreadMessageParams = parse_params(params)?;
                let basis = self
                    .supervisor_gate(Some(&p.from_chat_id), |s| s.message_threads, supervisor::MESSAGE_OFF)?
                    .then(|| {
                        self.doc_host.intervention_basis(
                            zeron_proto::supervisor::InterventionKind::Send,
                            &p.from_chat_id,
                            Some(&p.chat_id),
                            None,
                        )
                    });
                let id = self
                    .doc_host
                    .queue_thread_message(&p.chat_id, Some(&p.from_chat_id), &p.text)
                    .await
                    .map_err(|e| RpcError::Failed(e.to_string()))?;
                if let Some(basis) = basis {
                    self.doc_host
                        .record_intervention(basis, Some(id.clone()), None, &p.text);
                }
                RpcReply::value(&serde_json::json!({ "id": id }))
            }
            methods::SET_THREAD_WATCHES => {
                let p: SetThreadWatchesParams = parse_params(params)?;
                let watching = self
                    .doc_host
                    .set_thread_watches_for(&p.chat_id, &p.add, &p.remove)
                    .map_err(|e| RpcError::Failed(e.to_string()))?;
                RpcReply::value(&serde_json::json!({ "watching": watching }))
            }
            methods::DELEGATE_TASK => {
                let p: crate::doc_host::DelegateTaskParams = parse_params(params)?;
                if p
                    .client_request_id
                    .as_deref()
                    .is_some_and(|id| id.starts_with(crate::review::REQUEST_PREFIX))
                {
                    return Err(RpcError::Failed(format!(
                        "invalid_request: clientRequestId prefix {} is reserved for review_changes",
                        crate::review::REQUEST_PREFIX
                    )));
                }
                self.supervisor_gate(Some(&p.owner_chat_id), |s| s.start_missions, supervisor::START_OFF)?;
                let p = self.route_delegation(p)?;
                let view = self
                    .doc_host
                    .delegate_task(p, self.registry.delegation_settings())
                    .await
                    .map_err(|e| RpcError::Failed(e.to_string()))?;
                RpcReply::value(&view)
            }
            methods::TASK_STATUS => {
                let p: TaskParams = parse_params(params)?;
                let view = self
                    .doc_host
                    .task_status(&p.chat_id, &p.task_id, p.acknowledge)
                    .await
                    .map_err(|e| RpcError::Failed(e.to_string()))?;
                RpcReply::value(&view)
            }
            methods::TASK_CANCEL => {
                let p: TaskParams = parse_params(params)?;
                let status = self
                    .doc_host
                    .task_cancel(&p.chat_id, &p.task_id)
                    .await
                    .map_err(|e| RpcError::Failed(e.to_string()))?;
                RpcReply::value(&serde_json::json!({ "taskId": p.task_id, "status": status }))
            }
            methods::RESOLVE_REVIEW_FINDINGS => {
                let p: crate::doc_host::ResolveFindingsParams = parse_params(params)?;
                let resolved = self
                    .doc_host
                    .resolve_review_findings(p)
                    .map_err(|e| RpcError::Failed(e.to_string()))?;
                RpcReply::value(&resolved)
            }
            methods::HAND_REVIEW_FINDINGS => {
                let p: HandFindingsParams = parse_params(params)?;
                let handed = self
                    .doc_host
                    .hand_review_findings(&p.owner_chat_id, &p.task_id, p.ids)
                    .map_err(|e| RpcError::Failed(e.to_string()))?;
                RpcReply::value(&serde_json::json!({ "handed": handed }))
            }
            methods::TASK_WAKE_ALWAYS => {
                let p: TaskParams = parse_params(params)?;
                self.doc_host
                    .task_wake_always(&p.chat_id, &p.task_id)
                    .await
                    .map_err(|e| RpcError::Failed(e.to_string()))?;
                RpcReply::value(&serde_json::json!({}))
            }
            methods::GET_USAGE_SUMMARY => {
                let p: UsageSummaryParams = parse_params(params)?;
                let config = self.agent_accounts.config();
                let mut roots = crate::usage::UsageRoots::new(
                    &config.claude_config_dir,
                    &config.codex_home,
                    &config.grok_home,
                );
                if let Ok(store) = self.provider_instances_store() {
                    for instance in store.snapshot().instances {
                        if let zeron_proto::InstanceConfig::Native {
                            home_dir: Some(home),
                            ..
                        } = &instance.config
                            && let Ok(home) = crate::provider_instances::home_path(
                                &instance.id,
                                home,
                                store.data_dir(),
                            )
                        {
                            roots.add_home(instance.driver, &home);
                        }
                    }
                }
                let summary = tokio::task::spawn_blocking(move || {
                    crate::usage::summary(&roots, p.days, chrono::Local::now())
                })
                .await
                .map_err(|e| RpcError::Failed(e.to_string()))?;
                RpcReply::value(&summary)
            }
            methods::GET_THREAD_SUMMARY => {
                let p: ChatParams = parse_params(params)?;
                let host = self.doc_host.clone();
                let summary = tokio::task::spawn_blocking(move || {
                    let handle = host
                        .open(&p.chat_id)
                        .map_err(|e| RpcError::Failed(e.to_string()))?;
                    handle
                        .doc()
                        .thread_summary()
                        .map_err(|e| RpcError::Failed(e.to_string()))
                })
                .await
                .map_err(|e| RpcError::Failed(e.to_string()))??;
                RpcReply::value(&summary)
            }
            methods::GET_SESSION_STATS => {
                let p: ChatsParams = parse_params(params)?;
                let host = self.doc_host.clone();
                let stats = tokio::task::spawn_blocking(move || {
                    p.chat_ids
                        .into_iter()
                        .filter_map(|id| {
                            let stats = host.open_local(&id).ok()?.doc().session_stats()?;
                            Some((id, stats))
                        })
                        .collect::<std::collections::HashMap<_, _>>()
                })
                .await
                .map_err(|e| RpcError::Failed(e.to_string()))?;
                RpcReply::value(&stats)
            }
            methods::TAKE_THREAD_MESSAGES => {
                let p: ChatParams = parse_params(params)?;
                let messages: Vec<serde_json::Value> = self
                    .doc_host
                    .take_thread_messages(&p.chat_id)
                    .map_err(|e| RpcError::Failed(e.to_string()))?
                    .into_iter()
                    .map(|row| {
                        serde_json::json!({
                            "id": row.id,
                            "fromChatId": row.from_chat_id,
                            "text": row.text,
                        })
                    })
                    .collect();
                RpcReply::value(&serde_json::json!({ "messages": messages }))
            }
            methods::ASK_USER => {
                let mut p: AskUserParams = parse_params(params)?;
                for question in &mut p.questions {
                    question.kind = None;
                }
                let answers = self
                    .sessions
                    .ask_user(&p.chat_id, &p.run_secret, p.questions, ASK_USER_WAIT)
                    .await
                    .map_err(RpcError::Failed)?;
                RpcReply::value(&serde_json::json!({ "answers": answers }))
            }
            methods::QUEUE_COMMAND => {
                let p: QueueCommandParams = parse_params(params)?;
                let respond = match &p.command {
                    SessionCommandPayload::Interrupt { .. } => {
                        supervisor::refuse_supervisor(p.from_chat_id.as_deref(), supervisor::NO_INTERRUPT)?;
                        None
                    }
                    SessionCommandPayload::RespondInput {
                        request_id,
                        answers,
                    } => match p.from_chat_id.as_deref() {
                        Some(from)
                            if self.supervisor_gate(Some(from), |s| s.message_threads, supervisor::MESSAGE_OFF)? =>
                        {
                            self.supervisor_may_answer(from, &p.chat_id)?;
                            let text: Vec<String> = answers
                                .iter()
                                .map(|answer| format!("{}: {}", answer.question_id, answer.labels.join(", ")))
                                .collect();
                            let basis = self.doc_host.intervention_basis(
                                zeron_proto::supervisor::InterventionKind::Respond,
                                from,
                                Some(&p.chat_id),
                                None,
                            );
                            Some((basis, request_id.clone(), text.join("; ")))
                        }
                        _ => None,
                    },
                    _ => None,
                };
                let command_id = self
                    .doc_host
                    .queue_command_with_transfers(&p.chat_id, p.command, p.transfers)
                    .map_err(|e| RpcError::Failed(e.to_string()))?;
                if let Some((basis, request_id, text)) = respond {
                    self.doc_host.record_intervention(
                        basis,
                        Some(command_id.clone()),
                        Some(request_id),
                        &text,
                    );
                }
                RpcReply::value(&serde_json::json!({ "commandId": command_id }))
            }
            methods::TAKE_PROJECT_ACTION_SETUP => {
                let p: TakeProjectActionSetupParams = parse_params(params)?;
                let outcome = self
                    .project_actions
                    .take_setup_handoff(&p.command_id, &p.chat_id);
                match outcome {
                    Some(outcome) => RpcReply::value(&serde_json::json!({
                        "ready": true,
                        "setupAction": outcome.setup_action,
                        "setupError": outcome.setup_error,
                    })),
                    None => RpcReply::value(&serde_json::json!({ "ready": false })),
                }
            }
            methods::RETRY_DELIVERY => {
                let p: ChatParams = parse_params(params)?;
                self.doc_host
                    .retry_delivery(&p.chat_id)
                    .map_err(|e| RpcError::Failed(e.to_string()))?;
                RpcReply::value(&serde_json::json!({}))
            }
            methods::RELAY_COMMAND => {
                let p: RelayCommandParams = parse_params(params)?;
                let outcome = self
                    .doc_host
                    .ingest_relayed_command(&p.chat_id, p.entry)
                    .await
                    .map_err(|e| RpcError::Failed(e.to_string()))?;
                RpcReply::value(&serde_json::json!({ "outcome": outcome }))
            }
            methods::FORK_SIDE_CHAT => {
                #[derive(Deserialize)]
                #[serde(rename_all = "camelCase")]
                struct ForkParams {
                    chat_id: String,
                    source_chat_id: String,
                    /// Where the fork hangs in the tree. Defaults to the
                    /// source; a side chat's own fork button passes the
                    /// side chat's parent so the copy lists as a sibling.
                    #[serde(default)]
                    created_by_chat_id: Option<String>,
                }
                let p: ForkParams = parse_params(params)?;
                let created_by_chat_id = p
                    .created_by_chat_id
                    .clone()
                    .filter(|id| !id.trim().is_empty())
                    .unwrap_or_else(|| p.source_chat_id.clone());
                let failed = |e: crate::EngineError| RpcError::Failed(e.to_string());
                let source = self
                    .workspace
                    .chat(&p.source_chat_id)
                    .map_err(failed)?
                    .ok_or_else(|| RpcError::Failed("Source chat no longer exists".into()))?;
                if source.device_id != self.doc_host.device_id() {
                    return Err(RpcError::Failed(
                        "Fork must be created on the source device".into(),
                    ));
                }
                if let Some(existing) = self.workspace.chat(&p.chat_id).map_err(failed)? {
                    if existing.created_by_chat_id.as_deref() == Some(created_by_chat_id.as_str()) {
                        return RpcReply::value(&existing);
                    }
                    return Err(RpcError::Failed("Chat id already exists".into()));
                }
                let source_doc = self.doc_host.open(&p.source_chat_id).map_err(failed)?;
                let entries = source_doc
                    .doc()
                    .read_entries()
                    .map_err(|e| RpcError::Failed(e.to_string()))?;
                let boundary = entries
                    .iter()
                    .rposition(|entry| {
                        entry.role == zeron_doc::MessageRole::Assistant
                            && entry.status == Some(zeron_doc::MessageStatus::Complete)
                    })
                    .ok_or_else(|| {
                        RpcError::Failed(
                            "Wait for a completed response before starting a side chat".into(),
                        )
                    })?;
                let mut chat = source.clone();
                chat.id = p.chat_id;
                chat.created_by_chat_id = Some(created_by_chat_id);
                chat.forked_from_chat_id = Some(source.id.clone());
                chat.delegation = None;
                chat.title = None; // First side-chat turn receives its own generated title.
                chat.archived = false;
                chat.created_at = chrono::Utc::now();
                chat.last_message_at = None;
                chat.last_message_preview = None;
                chat.last_seen_at = None;
                chat.harness_session_id = None;
                chat.harness_session_cwd = None;
                chat.room_gen = Some(2);
                // Missing rows open on chat2. Persist history before publishing
                // the registry row so a crash cannot leave a discoverable empty fork.
                let target = self.doc_host.open(&chat.id).map_err(failed)?;
                let existing = target
                    .doc()
                    .read_entries()
                    .map_err(|e| RpcError::Failed(e.to_string()))?;
                for entry in entries[..=boundary]
                    .iter()
                    .filter(|entry| !existing.iter().any(|e| e.id == entry.id))
                {
                    let mut entry = entry.clone();
                    // Historical approvals belong to the source runtime; they
                    // must never block or send answers from the new composer.
                    for part in &mut entry.parts {
                        if let zeron_doc::MessagePart::Input { resolved, .. } = part {
                            *resolved = true;
                        }
                    }
                    crate::html_render::adopt_pages(&self.uploads, &chat.id, &mut entry);
                    target
                        .doc()
                        .push_message(&entry)
                        .map_err(|e| RpcError::Failed(e.to_string()))?;
                }
                // The seam: everything above came from the source. Its own
                // entry (system role, complete) so the copied history and the
                // fork's first turn never share a row.
                let marker_id = format!("fork:{}", chat.id);
                if !existing.iter().any(|e| e.id == marker_id) {
                    let source_title = source
                        .title
                        .clone()
                        .or_else(|| source.last_message_preview.clone())
                        .unwrap_or_else(|| "New session".into());
                    target
                        .doc()
                        .push_message(&zeron_doc::SessionMessageEntry {
                            duration_ms: None,
                            id: marker_id.clone(),
                            role: zeron_doc::MessageRole::System,
                            parts: vec![zeron_doc::MessagePart::Fork {
                                id: marker_id,
                                source_chat_id: source.id.clone(),
                                source_title,
                            }],
                            created_at: chrono::Utc::now().timestamp_millis(),
                            device_id: self.doc_host.device_id().to_owned(),
                            status: Some(zeron_doc::MessageStatus::Complete),
                            continuation_of: None,
                            from_chat_id: None,
                            fork_import: false,
                            answers_request_id: None,
                        })
                        .map_err(|e| RpcError::Failed(e.to_string()))?;
                }
                self.doc_host.persist_fork(&target).map_err(failed)?;
                self.workspace.import_chat_row(&chat).map_err(failed)?;
                RpcReply::value(&chat)
            }
            methods::FOCUS_CHAT => {
                let p: ChatParams = parse_params(params)?;
                self.doc_host
                    .focus_chat(&p.chat_id)
                    .map_err(|e| RpcError::Failed(e.to_string()))?;
                RpcReply::value(&serde_json::json!({}))
            }
            methods::WATCH_DOC_MESSAGES => {
                // Opt-in: older viewports retain the full-reset contract.
                let opening_tail = params
                    .get("openingTail")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                let p: ChatParams = parse_params(params)?;
                // An open transcript keeps its chat's checkout diff live (and
                // discardable) even when the chat is archived; the guard lives
                // exactly as long as the stream.
                let interest = self.diff_sync.retain_chat(&p.chat_id);
                let stream = if opening_tail {
                    opening_doc_messages_stream(self.doc_host.clone(), p.chat_id).await?
                } else {
                    let handle = self
                        .doc_host
                        .open(&p.chat_id)
                        .map_err(|e| RpcError::Failed(e.to_string()))?;
                    doc_messages_stream(handle.watch_messages(), handle.doc_arc())
                };
                Ok(RpcReply::Stream(
                    stream
                        .map(move |frame| {
                            let _held = &interest;
                            frame
                        })
                        .boxed(),
                ))
            }
            methods::WATCH_QUEUE => {
                let p: ChatParams = parse_params(params)?;
                let handle = self
                    .doc_host
                    .open(&p.chat_id)
                    .map_err(|e| RpcError::Failed(e.to_string()))?;
                let rx = handle.watch_queue();
                Ok(RpcReply::Stream(
                    futures::stream::unfold((rx, true), |(mut rx, first)| async move {
                        if !first {
                            rx.changed().await.ok()?;
                        }
                        let items = rx.borrow_and_update().clone();
                        let value = serde_json::json!({ "items": items });
                        Some((value, (rx, false)))
                    })
                    .boxed(),
                ))
            }
            methods::QUEUE_MESSAGE => {
                let p: QueueMessageParams = parse_params(params)?;
                let id = self
                    .doc_host
                    .queue_message_with_behavior(
                        &p.chat_id,
                        &p.text,
                        p.attachments,
                        p.hold_for_turn_end,
                    )
                    .map_err(|e| RpcError::Failed(e.to_string()))?;
                if p.steer_now {
                    self.doc_host.steer_queued_on_submit(&p.chat_id, &id).await;
                }
                RpcReply::value(&serde_json::json!({ "id": id }))
            }
            methods::UPDATE_QUEUED_MESSAGE => {
                let p: QueuedMessageParams = parse_params(params)?;
                let changed = self
                    .doc_host
                    .update_queued_message(&p.chat_id, &p.id, &p.text)
                    .map_err(|e| RpcError::Failed(e.to_string()))?;
                RpcReply::value(&serde_json::json!({ "changed": changed }))
            }
            methods::BEGIN_QUEUED_MESSAGE_EDIT => {
                let p: BeginQueuedMessageEditParams = parse_params(params)?;
                let outcome = self
                    .doc_host
                    .begin_queued_message_edit(
                        &p.chat_id,
                        &p.id,
                        &p.editor_device_id,
                        &p.editor_instance_id,
                    )
                    .await
                    .map_err(|e| RpcError::Failed(e.to_string()))?;
                RpcReply::value(
                    &serde_json::to_value(outcome).map_err(|e| RpcError::Failed(e.to_string()))?,
                )
            }
            methods::RENEW_QUEUED_MESSAGE_EDIT => {
                let p: RenewQueuedMessageEditParams = parse_params(params)?;
                let outcome = self
                    .doc_host
                    .renew_queued_message_edit(&p.chat_id, &p.id, &p.lease_id)
                    .await
                    .map_err(|e| RpcError::Failed(e.to_string()))?;
                RpcReply::value(
                    &serde_json::to_value(outcome).map_err(|e| RpcError::Failed(e.to_string()))?,
                )
            }
            methods::FINISH_QUEUED_MESSAGE_EDIT => {
                let p: FinishQueuedMessageEditParams = parse_params(params)?;
                let action = match p.action {
                    FinishQueuedMessageEditAction::Commit => {
                        crate::doc_host::FinishQueueEditAction::Commit
                    }
                    FinishQueuedMessageEditAction::Cancel => {
                        crate::doc_host::FinishQueueEditAction::Cancel
                    }
                    FinishQueuedMessageEditAction::Discard => {
                        crate::doc_host::FinishQueueEditAction::Discard
                    }
                    FinishQueuedMessageEditAction::ReleaseUnchanged => {
                        crate::doc_host::FinishQueueEditAction::ReleaseUnchanged
                    }
                };
                let outcome = self
                    .doc_host
                    .finish_queued_message_edit_with_attachments(
                        &p.chat_id,
                        &p.id,
                        &p.lease_id,
                        action,
                        p.text.as_deref(),
                        p.expected_text_hash.as_deref(),
                        p.attachments.as_deref(),
                    )
                    .await
                    .map_err(|e| RpcError::Failed(e.to_string()))?;
                RpcReply::value(
                    &serde_json::to_value(outcome).map_err(|e| RpcError::Failed(e.to_string()))?,
                )
            }
            methods::MOVE_QUEUED_MESSAGE => {
                let p: QueuedMessageParams = parse_params(params)?;
                let changed = self
                    .doc_host
                    .move_queued_message(&p.chat_id, &p.id, p.to_index)
                    .map_err(|e| RpcError::Failed(e.to_string()))?;
                RpcReply::value(&serde_json::json!({ "changed": changed }))
            }
            methods::REMOVE_QUEUED_MESSAGE => {
                let p: QueuedMessageParams = parse_params(params)?;
                let removed = self
                    .doc_host
                    .remove_queued_message(&p.chat_id, &p.id)
                    .await
                    .map_err(|e| RpcError::Failed(e.to_string()))?;
                RpcReply::value(&serde_json::json!({ "removed": removed }))
            }
            methods::SEND_QUEUED_MESSAGE_NOW => {
                let p: QueuedMessageParams = parse_params(params)?;
                let sent = self
                    .doc_host
                    .send_queued_now(&p.chat_id, &p.id)
                    .await
                    .map_err(|e| RpcError::Failed(e.to_string()))?;
                RpcReply::value(&serde_json::json!({ "sent": sent }))
            }
            methods::STEER_QUEUED_MESSAGE_NOW => {
                let p: QueuedMessageParams = parse_params(params)?;
                let sent = self
                    .doc_host
                    .steer_queued_now(&p.chat_id, &p.id)
                    .await
                    .map_err(|e| RpcError::Failed(e.to_string()))?;
                RpcReply::value(&serde_json::json!({ "sent": sent }))
            }
            methods::PROBE_SYNC => {
                // Focus probes remain cheap. An explicit Retry may also allow
                // one fresh, shared auth attempt before its cooldown expires.
                if params.get("retry").and_then(serde_json::Value::as_bool) == Some(true)
                    && let Some(auth) = &self.auth
                {
                    auth.retry_refresh();
                }
                self.workspace.probe();
                self.doc_host.probe_open_chats();
                self.doc_host.probe_edge_reachability();
                RpcReply::value(&serde_json::json!({}))
            }
            methods::SYNC_STATUS => {
                fn room_json(s: &zeron_sync::RoomStatsSnapshot) -> serde_json::Value {
                    serde_json::json!({
                        "connected": s.connected,
                        "synced": s.synced,
                        "lastPushedMs": s.last_pushed_ms,
                        "lastAckMs": s.last_ack_ms,
                        "rejoins": s.rejoins,
                        "probes": s.probes,
                        "fullResyncs": s.full_resyncs,
                        "disconnects": s.disconnects,
                        "rejected": s.rejected,
                    })
                }
                fn chat2_json(s: &zeron_sync::ChatStatsSnapshot) -> serde_json::Value {
                    serde_json::json!({
                        "connected": s.connected,
                        "cursor": s.cursor,
                        "headSeq": s.head_seq,
                        "seqFloor": s.seq_floor,
                        "checkpointSeq": s.checkpoint_seq,
                        "checkpointSize": s.checkpoint_size,
                        "rowCount": s.row_count,
                        "rowBytes": s.row_bytes,
                        "pendingPushes": s.pending_pushes,
                        "rejoins": s.rejoins,
                        "disconnects": s.disconnects,
                        "rejected": s.rejected,
                        "serverResets": s.server_resets,
                    })
                }
                let workspace = self.workspace.sync_status();
                let chats: Vec<serde_json::Value> = self
                    .doc_host
                    .sync_statuses()
                    .iter()
                    .map(|(chat_id, room)| {
                        serde_json::json!({
                            "chatId": chat_id,
                            "room": room.as_ref().map(chat2_json),
                            "state": self.doc_host.chat_sync_state(chat_id),
                        })
                    })
                    .collect();
                RpcReply::value(&serde_json::json!({
                    "deviceId": self.doc_host.device_id(),
                    "nowMs": crate::now_ms(),
                    "workspace": workspace.as_ref().map(room_json),
                    "chats": chats,
                    "resources": self.doc_host.sync_resources(),
                }))
            }
            methods::WATCH_CONNECTIVITY => Ok(RpcReply::Stream(watch_stream(
                self.doc_host.watch_connectivity(),
            ))),
            methods::WATCH_TRANSFERS => Ok(RpcReply::Stream(watch_stream(
                self.doc_host.watch_transfers(),
            ))),
            methods::WATCH_PREVIEWS => {
                let p: zeron_proto::WatchPreviewsParams = parse_params(params)?;
                if self
                    .workspace
                    .chat(&p.chat_id)
                    .map_err(|e| RpcError::Failed(e.to_string()))?
                    .is_none()
                {
                    return Err(RpcError::Failed("Project session not found".into()));
                }
                let previews = self
                    .previews
                    .as_ref()
                    .ok_or_else(|| RpcError::Failed("Preview discovery unavailable".into()))?;
                let catalog = previews.catalog().clone();
                let changes = catalog.subscribe();
                let chats = self.workspace.watch_chats();
                let workspace = self.workspace.clone();
                // This subscription stays on the viewing device. A remote chat
                // selects advertised services, but its URL uses our local proxy.
                let stream = futures::stream::unfold(
                    (changes, chats, true, workspace, catalog, p.chat_id),
                    |(mut changes, mut chats, first, workspace, catalog, chat_id)| async move {
                        if !first {
                            tokio::select! {
                                result = changes.changed() => { if result.is_err() { return None; } }
                                result = chats.changed() => { if result.is_err() { return None; } }
                            }
                        }
                        let mut snapshot = changes.borrow_and_update().clone();
                        chats.borrow_and_update();
                        let chat = workspace.chat(&chat_id).ok().flatten();
                        let device = chat
                            .as_ref()
                            .map(|c| c.device_id.clone())
                            .unwrap_or_default();
                        snapshot.remote = device != catalog.device_id();
                        let cwd = chat.and_then(|c| c.cwd);
                        let cwd = cwd.map(|cwd| {
                            if snapshot.remote {
                                std::path::PathBuf::from(cwd)
                            } else {
                                std::path::PathBuf::from(&cwd)
                                    .canonicalize()
                                    .unwrap_or_else(|_| cwd.into())
                            }
                        });
                        snapshot.project_name = cwd
                            .as_ref()
                            .and_then(|p| p.file_name())
                            .map(|s| s.to_string_lossy().into_owned());
                        snapshot.services.retain(|service| {
                            service.device_id == device
                                && cwd.as_ref().is_some_and(|cwd| {
                                    cwd == std::path::Path::new(&service.project_cwd)
                                })
                        });
                        let value = serde_json::to_value(snapshot).ok()?;
                        Some((value, (changes, chats, false, workspace, catalog, chat_id)))
                    },
                );
                Ok(RpcReply::Stream(Box::pin(stream)))
            }
            methods::WATCH_CHATS => {
                Ok(RpcReply::Stream(watch_stream(self.workspace.watch_chats())))
            }
            methods::WATCH_SIDEBAR_PREFERENCES => Ok(RpcReply::Stream(watch_stream(
                self.workspace.watch_sidebar_preferences(),
            ))),
            methods::WATCH_DEVICES => Ok(RpcReply::Stream(watch_stream(
                self.workspace.watch_devices(),
            ))),
            methods::WATCH_SPACES => Ok(RpcReply::Stream(watch_stream(
                self.workspace.watch_spaces(),
            ))),
            methods::ENSURE_SUPERVISOR
            | methods::WATCH_SUPERVISOR
            | methods::GET_SUPERVISOR_SETTINGS
            | methods::SET_SUPERVISOR_SETTINGS
            | methods::SUPERVISOR_ACT
            | methods::REQUEST_SUPERVISOR_CHECK => self.supervisor_rpc(method, params).await,
            methods::WATCH_TASKS => {
                Ok(RpcReply::Stream(watch_stream(self.workspace.watch_tasks())))
            }
            methods::WATCH_TASK_STEPS => Ok(RpcReply::Stream(watch_stream(
                self.workspace.watch_task_steps(),
            ))),
            methods::WATCH_SESSIONS => {
                // Local live statuses merged with remote devices' workspace rows.
                let merged = self
                    .workspace
                    .merged_sessions_watch(self.sessions.watch_sessions());
                Ok(RpcReply::Stream(watch_stream(merged)))
            }
            methods::LOCAL_DEVICE => {
                RpcReply::value(&serde_json::json!({ "deviceId": self.doc_host.device_id() }))
            }
            methods::LOCAL_IMPORT_STATUS => {
                let importer = self.local_importer()?.clone();
                let status = tokio::task::spawn_blocking(move || importer.status())
                    .await
                    .map_err(|e| RpcError::Failed(e.to_string()))?
                    .map_err(|e| RpcError::Failed(e.to_string()))?;
                RpcReply::value(&status)
            }
            methods::IMPORT_LOCAL_WORKSPACE => {
                let importer = self.local_importer()?.clone();
                // Progress rides an unbounded channel: the importer is
                // blocking (sqlite + fs) and must never wedge on a slow
                // viewer; items are tiny and bounded by the chat count.
                let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<serde_json::Value>();
                tokio::task::spawn_blocking(move || {
                    let emit = |event: crate::local_import::ImportEvent| {
                        if let Ok(item) = serde_json::to_value(&event) {
                            let _ = tx.send(item);
                        }
                    };
                    if let Err(err) = importer.run(emit) {
                        tracing::error!(error = %err, "local import failed");
                        let _ = tx.send(serde_json::json!({
                            "kind": "summary",
                            "importedChats": 0, "importedSpaces": 0,
                            "skippedChats": 0, "skippedSpaces": 0,
                            "journalsCopied": 0, "ledgerRowsMerged": 0,
                            "errors": [format!("{err}")],
                        }));
                    }
                    // tx drops here — the stream ends after the summary item.
                });
                Ok(RpcReply::Stream(Box::pin(futures::stream::poll_fn(
                    move |cx| rx.poll_recv(cx),
                ))))
            }
            methods::UPDATE_STATUS => Ok(RpcReply::Stream(watch_stream(self.updater()?.watch()))),
            methods::APPLY_UPDATE => {
                let version = self
                    .updater()?
                    .apply()
                    .await
                    .map_err(|e| RpcError::Failed(format!("{e:#}")))?;
                RpcReply::value(&serde_json::json!({ "ok": true, "version": version }))
            }
            methods::WATCH_HARNESS_UPDATES => Ok(RpcReply::Stream(watch_stream(
                self.harness_updates()?.watch(),
            ))),
            methods::CHECK_HARNESS_UPDATES => {
                let p: HarnessUpdateParams = parse_params(params)?;
                let coordinator = self.harness_updates()?.clone();
                // Provider checks are engine-owned once accepted. If a
                // Settings view closes or a relay drops, the request future
                // may disappear; detaching the work prevents a permanent
                // `Checking` status and still publishes the result by watch.
                let statuses = tokio::spawn(async move {
                    if let Some(harness) = p.harness {
                        coordinator.check_one(harness).await?;
                    } else {
                        coordinator.check_all().await;
                    }
                    Ok::<_, String>(coordinator.snapshot())
                })
                .await
                .map_err(|error| {
                    RpcError::Failed(format!("agent update check task failed: {error}"))
                })?
                .map_err(RpcError::Failed)?;
                RpcReply::value(&statuses)
            }
            methods::APPLY_HARNESS_UPDATE => {
                let p: HarnessUpdateParams = parse_params(params)?;
                let harness = p
                    .harness
                    .ok_or_else(|| RpcError::BadParams("harness is required".into()))?;
                let version = self
                    .harness_updates()?
                    .apply(harness)
                    .await
                    .map_err(RpcError::Failed)?;
                RpcReply::value(&serde_json::json!({ "ok": true, "version": version }))
            }
            methods::CANCEL_HARNESS_UPDATE => {
                let p: HarnessUpdateParams = parse_params(params)?;
                let harness = p
                    .harness
                    .ok_or_else(|| RpcError::BadParams("harness is required".into()))?;
                RpcReply::value(&serde_json::json!({
                    "cancelled": self.harness_updates()?.cancel(harness),
                }))
            }
            methods::DISMISS_HARNESS_UPDATE => {
                let p: DismissHarnessUpdateParams = parse_params(params)?;
                RpcReply::value(&self.harness_updates()?.dismiss(p.harness, p.version))
            }
            methods::SET_HARNESS_UPDATE_POLICY => {
                let p: SetHarnessUpdatePolicyParams = parse_params(params)?;
                RpcReply::value(&self.harness_updates()?.set_policy(p.harness, p.policy))
            }
            methods::UPDATE_HARNESSES => {
                let p: UpdateHarnessesParams = parse_params(params)?;
                let coordinator = self.harness_updates()?.clone();
                let chats = self.sessions.working_chats(Some(&p.harnesses));
                let stopped = self.stop_for_update(&chats).await;
                let host = self.doc_host.clone();
                tokio::spawn(async move {
                    for harness in p.harnesses {
                        if let Err(error) = coordinator.apply(harness).await {
                            tracing::warn!(?harness, %error, "update all: agent update failed");
                        }
                    }
                    if p.continue_threads {
                        for continuation in stopped {
                            if let Err(error) = host.queue_continuation(&continuation).await {
                                tracing::warn!(chat = %continuation.chat_id, %error, "update continuation failed");
                            }
                        }
                    }
                });
                RpcReply::value(&serde_json::json!({ "interrupted": chats.len() }))
            }
            methods::PREPARE_RESTART => {
                let p: PrepareRestartParams = parse_params(params)?;
                let schedule = self.resume_schedule()?.clone();
                let chats = self.sessions.working_chats(None);
                let stopped = self.stop_for_update(&chats).await;
                schedule.record_restart(
                    if p.continue_threads { stopped } else { Vec::new() },
                    chrono::Utc::now().timestamp_millis(),
                );
                let host = self.doc_host.clone();
                if let Err(error) = tokio::task::spawn_blocking(move || host.flush_all()).await {
                    tracing::warn!(%error, "restart flush failed");
                }
                RpcReply::value(&serde_json::json!({ "interrupted": chats.len() }))
            }
            methods::WATCH_LIMIT_RESUMES => Ok(RpcReply::Stream(watch_stream(
                self.resume_schedule()?.watch(),
            ))),
            methods::SET_LIMIT_RESUME => {
                let p: SetLimitResumeParams = parse_params(params)?;
                let changed = self.resume_schedule()?.set_auto(
                    &p.chat_id,
                    p.auto,
                    chrono::Utc::now().timestamp_millis(),
                );
                RpcReply::value(&serde_json::json!({ "changed": changed }))
            }
            methods::SET_AUTO_RESUME_LIMITS => {
                let p: SetAutoResumeLimitsParams = parse_params(params)?;
                let schedule = self.resume_schedule()?;
                schedule.set_auto_resume(p.enabled);
                RpcReply::value(&schedule.snapshot())
            }
            methods::MUTATE => {
                let p: MutateParams = parse_params(params)?;
                if let MutateParams::EscalateTask {
                    task_id,
                    from_chat_id,
                    title,
                    why,
                    detail,
                    options,
                } = p
                {
                    let item_id = self.escalate_task(
                        &task_id,
                        from_chat_id.as_deref(),
                        title,
                        why,
                        detail,
                        options,
                    )?;
                    return RpcReply::value(&serde_json::json!({ "ok": true, "itemId": item_id }));
                }
                if let MutateParams::PlanSteps {
                    task_id,
                    steps,
                    overview,
                    resources,
                    from_chat_id,
                } = p
                {
                    let steps = steps
                        .into_iter()
                        .map(|step| (step.title, step.notes, step.thread_id))
                        .collect();
                    let images_moved = self.plan_steps(
                        &task_id,
                        steps,
                        overview,
                        resources,
                        from_chat_id.as_deref(),
                    )?;
                    return RpcReply::value(&serde_json::json!({
                        "ok": true,
                        "task": self.workspace.task(&task_id),
                        "steps": self.workspace.task_steps_of(&task_id),
                        "imagesMoved": images_moved,
                    }));
                }
                let sidebar_pins = matches!(&p, MutateParams::ChangeSidebarPin { .. });
                self.mutate(p).await?;
                if sidebar_pins {
                    return RpcReply::value(&serde_json::json!({
                        "ok": true, "sidebarPreferences": self.workspace.sidebar_preferences_snapshot(),
                    }));
                }
                RpcReply::value(&serde_json::json!({ "ok": true }))
            }
            methods::WATCH_CHECKOUT_DIFFS => {
                Ok(RpcReply::Stream(watch_stream(self.diff_sync.watch_diffs())))
            }
            methods::WATCH_WORKSPACE_GIT_STATUS => {
                let request: zeron_proto::WatchWorkspaceFilesRequest = parse_params(params)?;
                let workspace = self.workspace_files.resolve_target(&request.target).await?;
                let rx = self.diff_sync.watch_git_statuses();
                // Only this authorized checkout crosses the connection. None means
                // unavailable, including plain folders and initial/restarting engines.
                let stream = futures::stream::unfold(
                    (rx, workspace.checkout_id, None, false),
                    |(mut rx, checkout_id, mut previous, mut emitted)| async move {
                        loop {
                            if emitted {
                                rx.changed().await.ok()?;
                            }
                            let next = rx
                                .borrow_and_update()
                                .iter()
                                .find(|s| s.checkout_id == checkout_id)
                                .cloned();
                            if !emitted || previous != next {
                                emitted = true;
                                previous = next.clone();
                                let value =
                                    serde_json::to_value(zeron_proto::WorkspaceGitStatusFrame {
                                        status: next,
                                    })
                                    .ok()?;
                                return Some((value, (rx, checkout_id, previous, emitted)));
                            }
                        }
                    },
                );
                Ok(RpcReply::Stream(stream.boxed()))
            }
            methods::WATCH_CHECKOUT_CHANGE_REQUEST => {
                let p: CheckoutChangeRequestParams = parse_params(params)?;
                let cwd = self.change_request_root(&p.cwd).await?;
                let stream = self
                    .change_requests
                    .watch_for_branch(&cwd, p.branch.as_deref())
                    .await
                    .map_err(|error| RpcError::Failed(error.to_string()))?
                    .filter_map(|status| async move { serde_json::to_value(status).ok() });
                Ok(RpcReply::Stream(stream.boxed()))
            }
            methods::GET_GIT_ACTIVITY => {
                let p: zeron_proto::GitActivityRequest = parse_params(params)?;
                let spaces = self.workspace.watch_spaces().borrow().clone();
                let groups =
                    crate::home_widgets::repo_groups(&spaces, self.doc_host.device_id());
                let activity = crate::home_widgets::git_activity(
                    groups,
                    p.space_id.as_deref(),
                    p.days.unwrap_or(14),
                    chrono::Local::now().date_naive(),
                )
                .await;
                RpcReply::value(&activity)
            }
            methods::LIST_LISTENING_SERVICES => {
                let p: zeron_proto::ListeningServicesRequest = parse_params(params)?;
                let spaces = self.workspace.watch_spaces().borrow().clone();
                let chats = self.workspace.read_chats().unwrap_or_default();
                let roots = crate::home_widgets::project_roots(
                    &spaces,
                    &chats,
                    self.doc_host.device_id(),
                );
                let services =
                    crate::home_widgets::listening_services(roots, p.all_ports).await;
                RpcReply::value(&zeron_proto::ListeningServices { services })
            }
            methods::GET_TODAY_RUNS => {
                let p: zeron_proto::TodayRunsRequest = parse_params(params)?;
                let chats = self
                    .workspace
                    .read_chats()
                    .map_err(|e| RpcError::Failed(e.to_string()))?;
                let host = self.doc_host.clone();
                let runs = tokio::task::spawn_blocking(move || {
                    let ids = p.chat_ids.as_deref();
                    crate::today_runs::today_runs(&chats, p.since_ms, ids, |chat| {
                        host.open_local(&chat.id)
                            .map(|handle| {
                                let doc = handle.doc().doc();
                                (
                                    crate::today_runs::turns_since(doc, p.since_ms),
                                    crate::today_runs::waits_since(doc, p.since_ms),
                                )
                            })
                            .unwrap_or_default()
                    })
                })
                .await
                .map_err(|e| RpcError::Failed(e.to_string()))?;
                RpcReply::value(&runs)
            }
            methods::GET_SYSTEM_STATS => {
                RpcReply::value(&crate::home_widgets::system_stats().await)
            }
            methods::GET_CHANGE_REQUEST_REPOSITORY => {
                #[derive(Deserialize)]
                struct P {
                    cwd: String,
                    #[serde(default)]
                    repository: Option<String>,
                }
                let p: P = parse_params(params)?;
                // Only a chat or project checkout on this device: a caller
                // never probes arbitrary directories for their remotes.
                let cwd = self.change_request_root(&p.cwd).await?;
                let cwd = cwd.as_path();
                let resolver = crate::source_control::ChangeRequestResolver::new();
                let repository = match p.repository {
                    Some(repository) => {
                        resolver
                            .matching_repository_for_checkout(cwd, &repository, &self.github)
                            .await
                    }
                    None => resolver.repository_for_checkout(cwd).await,
                };
                RpcReply::value(&repository)
            }
            methods::LIST_CHANGE_REQUEST_PAGE => {
                #[derive(Deserialize)]
                struct P {
                    repository: String,
                    #[serde(default)]
                    filter: zeron_proto::ChangeRequestFilter,
                    #[serde(default)]
                    after: Option<String>,
                    #[serde(default)]
                    refresh: bool,
                    /// Serve the first page's on-disk copy when there is one.
                    #[serde(default)]
                    cached: bool,
                }
                let p: P = parse_params(params)?;
                if !crate::source_control::valid_pr_repository(&p.repository) {
                    return Err(RpcError::BadParams("repository must be owner/repo".into()));
                }
                if p.after
                    .as_deref()
                    .is_some_and(|cursor| !crate::source_control::valid_page_cursor(cursor))
                {
                    return Err(RpcError::BadParams("invalid page cursor".into()));
                }
                let page = self
                    .github
                    .list_page(
                        &p.repository,
                        p.filter,
                        p.after.as_deref(),
                        p.refresh,
                        p.cached,
                    )
                    .await
                    .map_err(change_request_rpc_error)?;
                RpcReply::value(&page)
            }
            methods::POST_CHANGE_REQUEST_COMMENT => {
                #[derive(Deserialize)]
                struct P {
                    url: String,
                    body: String,
                }
                let p: P = parse_params(params)?;
                if p.body.trim().is_empty() || p.body.len() > 60_000 {
                    return Err(RpcError::BadParams(
                        "Comment must contain 1–60000 bytes".into(),
                    ));
                }
                let comment = self
                    .github
                    .post_comment(&p.url, &p.body)
                    .await
                    .map_err(change_request_rpc_error)?;
                RpcReply::value(&comment)
            }
            methods::GET_CHANGE_REQUEST_CONTROLS
            | methods::LIST_CHANGE_REQUEST_LABEL_CANDIDATES
            | methods::LIST_CHANGE_REQUEST_REVIEWER_CANDIDATES => {
                #[derive(Deserialize)]
                struct P {
                    url: String,
                }
                let p: P = parse_params(params)?;
                match method {
                    methods::GET_CHANGE_REQUEST_CONTROLS => RpcReply::value(
                        &self
                            .github
                            .change_request_controls(&p.url)
                            .await
                            .map_err(change_request_rpc_error)?,
                    ),
                    methods::LIST_CHANGE_REQUEST_LABEL_CANDIDATES => RpcReply::value(
                        &self
                            .github
                            .label_candidates(&p.url)
                            .await
                            .map_err(change_request_rpc_error)?,
                    ),
                    _ => RpcReply::value(
                        &self
                            .github
                            .reviewer_candidates(&p.url)
                            .await
                            .map_err(change_request_rpc_error)?,
                    ),
                }
            }
            methods::MERGE_CHANGE_REQUEST => {
                #[derive(Deserialize)]
                struct P {
                    url: String,
                    method: zeron_proto::ChangeRequestMergeMethod,
                }
                let p: P = parse_params(params)?;
                self.github
                    .merge(&p.url, p.method)
                    .await
                    .map_err(action_rpc_error)?;
                RpcReply::value(&serde_json::json!({}))
            }
            methods::SET_CHANGE_REQUEST_LABEL => {
                #[derive(Deserialize)]
                struct P {
                    url: String,
                    name: String,
                    add: bool,
                }
                let p: P = parse_params(params)?;
                self.github
                    .set_label(&p.url, &p.name, p.add)
                    .await
                    .map_err(action_rpc_error)?;
                RpcReply::value(&serde_json::json!({}))
            }
            methods::SET_CHANGE_REQUEST_REVIEWER => {
                #[derive(Deserialize)]
                struct P {
                    url: String,
                    login: String,
                    #[serde(default)]
                    team: bool,
                    add: bool,
                }
                let p: P = parse_params(params)?;
                self.github
                    .set_reviewer(&p.url, &p.login, p.team, p.add)
                    .await
                    .map_err(action_rpc_error)?;
                RpcReply::value(&serde_json::json!({}))
            }
            methods::SUBMIT_CHANGE_REQUEST_REVIEW => {
                #[derive(Deserialize)]
                struct P {
                    url: String,
                    event: zeron_proto::ChangeRequestReviewEvent,
                    #[serde(default)]
                    body: String,
                }
                let p: P = parse_params(params)?;
                self.github
                    .submit_review(&p.url, p.event, &p.body)
                    .await
                    .map_err(action_rpc_error)?;
                RpcReply::value(&serde_json::json!({}))
            }
            methods::REPLY_TO_CHANGE_REQUEST_THREAD => {
                #[derive(Deserialize)]
                #[serde(rename_all = "camelCase")]
                struct P {
                    url: String,
                    thread_id: String,
                    body: String,
                }
                let p: P = parse_params(params)?;
                self.github
                    .reply_to_thread(&p.url, &p.thread_id, &p.body)
                    .await
                    .map_err(action_rpc_error)?;
                RpcReply::value(&serde_json::json!({}))
            }
            methods::SET_CHANGE_REQUEST_THREAD_RESOLVED => {
                #[derive(Deserialize)]
                #[serde(rename_all = "camelCase")]
                struct P {
                    url: String,
                    thread_id: String,
                    resolved: bool,
                }
                let p: P = parse_params(params)?;
                self.github
                    .set_thread_resolved(&p.url, &p.thread_id, p.resolved)
                    .await
                    .map_err(action_rpc_error)?;
                RpcReply::value(&serde_json::json!({}))
            }
            methods::GET_CHANGE_REQUEST | methods::GET_CHANGE_REQUEST_DIFF => {
                #[derive(Deserialize)]
                #[serde(rename_all = "camelCase")]
                struct P {
                    url: String,
                    #[serde(default)]
                    refresh: bool,
                    #[serde(default)]
                    head_ref_oid: Option<String>,
                    #[serde(default)]
                    base_ref_oid: Option<String>,
                }
                let p: P = parse_params(params)?;
                let value = self
                    .github
                    .detail_request(
                        &p.url,
                        method == methods::GET_CHANGE_REQUEST_DIFF,
                        p.refresh,
                        p.head_ref_oid.as_deref(),
                        p.base_ref_oid.as_deref(),
                    )
                    .await
                    .map_err(change_request_rpc_error)?;
                RpcReply::value(&value)
            }
            // One-shot scoped capture for the Changes pane: `branch` diffs the
            // working tree against merge-base(baseRef, HEAD); `turn` diffs the
            // turn-start tree snapshot against the current tree; anything else
            // is the plain working-tree capture.
            methods::GET_CHECKOUT_DIFF => {
                #[derive(Deserialize)]
                #[serde(rename_all = "camelCase")]
                struct P {
                    cwd: String,
                    #[serde(default)]
                    mode: String,
                    base_ref: Option<String>,
                    chat_id: Option<String>,
                    commit_sha: Option<String>,
                }
                let p: P = parse_params(params)?;
                let repos = self.repos.clone();
                let device_id = self.doc_host.device_id().to_string();
                let turn = match (p.mode.as_str(), p.chat_id.as_deref()) {
                    ("turn", Some(chat_id)) => self.diff_sync.turn_snapshot(chat_id),
                    _ => None,
                };
                // A task of its own: under the dispatcher's frame a debug build overflows the 2 MB worker stack.
                let task = tokio_util::task::AbortOnDropHandle::new(tokio::spawn(async move {
                    let identity = repos
                        .checkout_identity(std::path::Path::new(&p.cwd))
                        .await
                        .map_err(|e| RpcError::Failed(e.to_string()))?;
                    let root = identity.root.as_path();
                    let snapshot = match p.mode.as_str() {
                        "branch" => {
                            let base_ref = p
                                .base_ref
                                .as_deref()
                                .ok_or_else(|| RpcError::Failed("baseRef required".into()))?;
                            let base = crate::diff_sync::merge_base(root, base_ref)
                                .await
                                .map_err(|e| RpcError::Failed(e.to_string()))?;
                            crate::diff_sync::capture_diff_against(&repos, root, Some(&base)).await
                        }
                        // One commit's own changes (History → per-commit tab):
                        // parent (or the empty tree) vs the commit itself.
                        "commit" => {
                            let sha = p
                                .commit_sha
                                .as_deref()
                                .ok_or_else(|| RpcError::Failed("commitSha required".into()))?;
                            crate::working_tree::resolve_commit(root, sha)
                                .await
                                .map_err(|e| RpcError::Failed(e.to_string()))?;
                            crate::diff_sync::capture_commit_diff(&repos, root, sha).await
                        }
                        "turn" => {
                            if p.chat_id.is_none() {
                                return Err(RpcError::Failed("chatId required".into()));
                            }
                            let snapshot = turn
                                .filter(|s| s.root == identity.root)
                                .ok_or_else(|| RpcError::Failed("no turn recorded".into()))?;
                            crate::diff_sync::capture_turn_diff(&repos, root, &snapshot.tree).await
                        }
                        _ => crate::diff_sync::capture_diff(&repos, root).await,
                    }
                    .map_err(|e| RpcError::Failed(e.to_string()))?;
                    RpcReply::value(&zeron_proto::CheckoutDiff {
                        checkout_id: identity.id,
                        device_id,
                        cwd: identity.root.to_string_lossy().to_string(),
                        patch: snapshot.patch,
                        files: snapshot.files,
                        additions: snapshot.additions,
                        deletions: snapshot.deletions,
                        truncated: snapshot.truncated,
                        checksum: snapshot.checksum,
                        updated_at: chrono::Utc::now(),
                    })
                }));
                task.await
                    .map_err(|e| RpcError::Failed(format!("checkout diff task failed: {e}")))?
            }
            methods::DISCARD_WORKING_TREE => {
                // This destructive branch performs several nested filesystem
                // futures. Box it so unrelated RPC calls do not inherit that
                // state in the already-large dispatcher stack frame.
                Box::pin(async move {
                    let p: DiscardWorkingTreeParams = parse_params(params)?;
                    let chat = self
                        .workspace
                        .chat(&p.chat_id)
                        .map_err(|e| RpcError::Failed(e.to_string()))?
                        .ok_or_else(|| RpcError::Failed("chat not found".into()))?;
                    if chat.device_id != self.doc_host.device_id() {
                        return Err(RpcError::Failed("chat is not hosted by this device".into()));
                    }
                    let cwd = chat
                        .cwd
                        .as_deref()
                        .ok_or_else(|| RpcError::Failed("chat has no checkout".into()))?;
                    let identity = self
                        .repos
                        .checkout_identity(std::path::Path::new(cwd))
                        .await
                        .map_err(|e| RpcError::Failed(e.to_string()))?;
                    if identity.id != p.checkout_id {
                        return Err(RpcError::Failed(
                            "chat checkout changed since the confirmation was opened".into(),
                        ));
                    }

                    // Refuse the mutation when any local chat on this exact
                    // checkout has a live run. We never interrupt an agent as a
                    // side effect of discarding files.
                    if self.agent_active_on_checkout(&identity.id).await {
                        return Err(RpcError::Failed(
                            "an agent is active in this working tree".into(),
                        ));
                    }

                    let snapshot = self
                        .diff_sync
                        .discard_working_tree(&identity, &p.expected_checksum)
                        .await
                        .map_err(|e| RpcError::Failed(e.to_string()))?;
                    RpcReply::value(&serde_json::json!({
                        "ok": true,
                        "checksum": snapshot.checksum,
                    }))
                })
                .await
            }
            methods::GET_CHECKOUT_FILE_DIFF_TEXT => {
                // This branch contains several large nested async futures. Keep it
                // behind an allocation so every unrelated RPC does not carry that
                // state in `EngineRpc::handle`'s stack frame.
                Box::pin(async move {
                    let p: zeron_proto::GetCheckoutFileDiffTextRequest = parse_params(params)?;
                    let identity =
                        Box::pin(self.repos.checkout_identity(std::path::Path::new(&p.cwd)))
                            .await
                            .map_err(|error| RpcError::Failed(error.to_string()))?;
                    if identity.id != p.checkout_id {
                        return Err(RpcError::Failed("checkoutId does not match cwd".into()));
                    }
                    let root = identity.root.as_path();
                    // A turn's base tree lives in the snapshot's own object
                    // store, so the snapshot rides along with its sha.
                    let mut turn_snapshot = None;
                    let (snapshot, base, target) = match p.mode.as_str() {
                        "branch" => {
                            let base_ref = p
                                .base_ref
                                .as_deref()
                                .ok_or_else(|| RpcError::Failed("baseRef required".into()))?;
                            let base = Box::pin(crate::diff_sync::merge_base(root, base_ref))
                                .await
                                .map_err(|error| RpcError::Failed(error.to_string()))?;
                            let snapshot = Box::pin(crate::diff_sync::capture_diff_against(
                                &self.repos,
                                root,
                                Some(&base),
                            ))
                            .await
                            .map_err(|error| RpcError::Failed(error.to_string()))?;
                            (snapshot, base, None)
                        }
                        "commit" => {
                            let sha = p
                                .commit_sha
                                .as_deref()
                                .ok_or_else(|| RpcError::Failed("commitSha required".into()))?;
                            Box::pin(crate::working_tree::resolve_commit(root, sha))
                                .await
                                .map_err(|error| RpcError::Failed(error.to_string()))?;
                            let base =
                                Box::pin(crate::diff_sync::commit_diff_base(root, sha)).await;
                            let snapshot = Box::pin(crate::diff_sync::capture_commit_diff(
                                &self.repos,
                                root,
                                sha,
                            ))
                            .await
                            .map_err(|error| RpcError::Failed(error.to_string()))?;
                            (snapshot, base, Some(sha.to_string()))
                        }
                        "turn" => {
                            let chat_id = p
                                .chat_id
                                .as_deref()
                                .ok_or_else(|| RpcError::Failed("chatId required".into()))?;
                            let turn = self
                                .diff_sync
                                .turn_snapshot(chat_id)
                                .filter(|snapshot| snapshot.root == identity.root)
                                .ok_or_else(|| RpcError::Failed("no turn recorded".into()))?;
                            let snapshot = Box::pin(crate::diff_sync::capture_turn_diff(
                                &self.repos,
                                root,
                                &turn.tree,
                            ))
                            .await
                            .map_err(|error| RpcError::Failed(error.to_string()))?;
                            let base = turn.tree.tree().to_string();
                            turn_snapshot = Some(turn.tree);
                            (snapshot, base, None)
                        }
                        _ => {
                            let base = Box::pin(crate::diff_sync::working_diff_base(root))
                                .await
                                .map_err(|error| RpcError::Failed(error.to_string()))?;
                            let snapshot =
                                Box::pin(crate::diff_sync::capture_diff(&self.repos, root))
                                    .await
                                    .map_err(|error| RpcError::Failed(error.to_string()))?;
                            (snapshot, base, None)
                        }
                    };
                    let stale = || zeron_proto::CheckoutFileDiffText {
                        diff_checksum: p.diff_checksum.clone(),
                        old_text: None,
                        new_text: None,
                        old_content_hash: None,
                        new_content_hash: None,
                        binary: false,
                        truncated: false,
                        stale: true,
                    };
                    if snapshot.checksum != p.diff_checksum {
                        return RpcReply::value(&stale());
                    }
                    let file = snapshot
                        .files
                        .iter()
                        .find(|file| file.path == p.path)
                        .ok_or_else(|| {
                            RpcError::Failed("path is not part of diff snapshot".into())
                        })?;
                    let pair = Box::pin(crate::diff_sync::read_diff_file_text_at(
                        root,
                        &base,
                        turn_snapshot.as_ref(),
                        target.as_deref(),
                        file,
                    ))
                    .await
                    .map_err(|error| RpcError::Failed(error.to_string()))?;
                    let current = match p.mode.as_str() {
                        "branch" => {
                            Box::pin(crate::diff_sync::capture_diff_against(
                                &self.repos,
                                root,
                                Some(&base),
                            ))
                            .await
                        }
                        "turn" => match turn_snapshot.as_ref() {
                            Some(turn) => {
                                Box::pin(crate::diff_sync::capture_turn_diff(
                                    &self.repos,
                                    root,
                                    turn,
                                ))
                                .await
                            }
                            None => Err(crate::EngineError::Other("no turn recorded".into())),
                        },
                        "commit" => {
                            let sha = p
                                .commit_sha
                                .as_deref()
                                .ok_or_else(|| RpcError::Failed("commitSha required".into()))?;
                            Box::pin(crate::diff_sync::capture_commit_diff(
                                &self.repos,
                                root,
                                sha,
                            ))
                            .await
                        }
                        _ => Box::pin(crate::diff_sync::capture_diff(&self.repos, root)).await,
                    }
                    .map_err(|error| RpcError::Failed(error.to_string()))?;
                    if current.checksum != p.diff_checksum {
                        return RpcReply::value(&stale());
                    }
                    RpcReply::value(&zeron_proto::CheckoutFileDiffText {
                        diff_checksum: p.diff_checksum,
                        old_text: pair.old_text,
                        new_text: pair.new_text,
                        old_content_hash: pair.old_content_hash,
                        new_content_hash: pair.new_content_hash,
                        binary: pair.binary,
                        truncated: pair.truncated,
                        stale: false,
                    })
                })
                .await
            }
            methods::LIST_REPOS => RpcReply::value(&self.repos.list().await),
            methods::ADD_REPO => {
                #[derive(Deserialize)]
                struct P {
                    path: String,
                }
                let p: P = parse_params(params)?;
                let repo = self
                    .repos
                    .add(&p.path)
                    .await
                    .map_err(|e| RpcError::Failed(e.to_string()))?;
                RpcReply::value(&repo)
            }
            methods::CLONE_REPO => {
                #[derive(Deserialize)]
                struct P {
                    url: String,
                }
                let p: P = parse_params(params)?;
                let repo = self
                    .repos
                    .clone_repo(&p.url)
                    .await
                    .map_err(|e| RpcError::Failed(e.to_string()))?;
                RpcReply::value(&repo)
            }
            methods::CREATE_REPO => {
                #[derive(Deserialize)]
                struct P {
                    name: String,
                }
                let p: P = parse_params(params)?;
                let repo = self
                    .repos
                    .create(&p.name)
                    .await
                    .map_err(|e| RpcError::Failed(e.to_string()))?;
                RpcReply::value(&repo)
            }
            methods::LIST_BRANCHES => {
                let p: RepoPathParams = parse_params(params)?;
                let branches = self
                    .repos
                    .branches(std::path::Path::new(&p.repo_path))
                    .await
                    .map_err(|e| RpcError::Failed(e.to_string()))?;
                RpcReply::value(&branches)
            }
            methods::LIST_REFS => {
                let p: RepoPathParams = parse_params(params)?;
                let refs = self
                    .repos
                    .refs(std::path::Path::new(&p.repo_path))
                    .await
                    .map_err(|e| RpcError::Failed(e.to_string()))?;
                RpcReply::value(&refs)
            }
            methods::LIST_GIT_HISTORY => {
                #[derive(Deserialize)]
                #[serde(rename_all = "camelCase")]
                struct P {
                    cwd: String,
                    #[serde(default)]
                    cursor: usize,
                    #[serde(default = "default_git_history_limit")]
                    limit: usize,
                }
                fn default_git_history_limit() -> usize {
                    crate::repos::GIT_HISTORY_DEFAULT_LIMIT
                }
                let p: P = parse_params(params)?;
                let history = self
                    .repos
                    .history(std::path::Path::new(&p.cwd), p.cursor, p.limit)
                    .await
                    .map_err(|e| RpcError::Failed(e.to_string()))?;
                RpcReply::value(&history)
            }
            methods::SEARCH_GIT_HISTORY => {
                #[derive(Deserialize)]
                #[serde(rename_all = "camelCase")]
                struct P {
                    cwd: String,
                    query: String,
                    #[serde(default)]
                    cursor: usize,
                    #[serde(default = "default_git_history_search_limit")]
                    limit: usize,
                }
                fn default_git_history_search_limit() -> usize {
                    crate::repos::GIT_HISTORY_DEFAULT_LIMIT
                }
                let p: P = parse_params(params)?;
                let history = self
                    .repos
                    .search_history(std::path::Path::new(&p.cwd), &p.query, p.cursor, p.limit)
                    .await
                    .map_err(|e| RpcError::Failed(e.to_string()))?;
                RpcReply::value(&history)
            }
            methods::RESOLVE_GIT_AVATARS => {
                #[derive(Deserialize)]
                #[serde(rename_all = "camelCase")]
                struct P {
                    cwd: String,
                    authors: Vec<GitAvatarAuthor>,
                    #[serde(default)]
                    cursor: usize,
                    #[serde(default = "default_git_avatar_limit")]
                    limit: usize,
                }
                #[derive(Deserialize)]
                struct GitAvatarAuthor {
                    sha: String,
                    email: String,
                }
                fn default_git_avatar_limit() -> usize {
                    crate::repos::GIT_HISTORY_DEFAULT_LIMIT
                }
                let p: P = parse_params(params)?;
                let authors: Vec<_> = p
                    .authors
                    .into_iter()
                    .take(crate::repos::GIT_HISTORY_MAX_LIMIT)
                    .filter(|author| author.sha.len() <= 64 && author.email.len() <= 512)
                    .map(|author| (author.sha, author.email))
                    .collect();
                let avatar_paths = self
                    .repos
                    .history_avatar_urls(std::path::Path::new(&p.cwd), &authors, p.cursor, p.limit)
                    .await;
                let mut avatars = std::collections::HashMap::new();
                for (email, path) in avatar_paths {
                    if let Ok(bytes) = tokio::fs::read(path).await {
                        avatars.insert(
                            email,
                            base64::engine::general_purpose::STANDARD.encode(bytes),
                        );
                    }
                }
                RpcReply::value(&avatars)
            }
            methods::FETCH_ALL => {
                let p: RepoPathParams = parse_params(params)?;
                self.repos
                    .fetch_all(std::path::Path::new(&p.repo_path))
                    .await
                    .map_err(|e| RpcError::Failed(e.to_string()))?;
                // Remote refs are repository state too. Force the checkout
                // watchers to publish a fresh snapshot instead of waiting for
                // the repair tick (some platforms do not report packed-refs).
                self.diff_sync.sync_all();
                RpcReply::value(&serde_json::json!({ "ok": true }))
            }
            methods::SWITCH_REF => {
                let p: SwitchRefParams = parse_params(params)?;
                let branch = self
                    .repos
                    .switch_ref(std::path::Path::new(&p.repo_path), &p.ref_name)
                    .await
                    .map_err(|e| RpcError::Failed(e.to_string()))?;
                RpcReply::value(&serde_json::json!({ "branch": branch }))
            }
            methods::GET_WORKTREE_SETTINGS => RpcReply::value(&self.repos.worktree_settings()),
            methods::SET_WORKTREE_SETTINGS => {
                let settings: zeron_proto::WorktreeSettings = parse_params(params)?;
                let status = self
                    .repos
                    .set_worktree_settings(settings)
                    .await
                    .map_err(|error| RpcError::Failed(error.to_string()))?;
                RpcReply::value(&status)
            }
            methods::LIST_FOLDERS => {
                let p: ListFoldersParams = parse_params(params)?;
                let listing = self
                    .repos
                    .list_folders(p.path)
                    .await
                    .map_err(|e| RpcError::Failed(e.to_string()))?;
                RpcReply::value(&listing)
            }
            methods::SEARCH_HOME_FOLDERS => {
                let request: zeron_proto::SearchHomeFoldersRequest = parse_params(params)?;
                if request.query.chars().count() > 256 {
                    return Err(RpcError::BadParams(
                        "SearchHomeFolders query must not exceed 256 characters".into(),
                    ));
                }
                let limit = request
                    .limit
                    .map_or(crate::repos::HOME_FOLDER_RESULTS, usize::from)
                    .min(crate::repos::HOME_FOLDER_MAX_RESULTS);
                let result = self
                    .repos
                    .search_home_folders(request.query, limit)
                    .await
                    .map_err(|e| RpcError::Failed(e.to_string()))?;
                RpcReply::value(&result)
            }
            methods::RELEASE_HOME_FOLDER_SEARCH => {
                self.repos.release_home_folder_search();
                RpcReply::value(&serde_json::json!({}))
            }
            methods::LIST_DRIVES => {
                let drives = self
                    .repos
                    .list_drives()
                    .await
                    .map_err(|e| RpcError::Failed(e.to_string()))?;
                RpcReply::value(&zeron_proto::DriveListing { drives })
            }
            methods::SEARCH_FILES => {
                let p: FileSearchParams = parse_params(params)?;
                if p.query.chars().count() > 256 {
                    return Err(RpcError::BadParams(
                        "SearchFiles query must not exceed 256 characters".into(),
                    ));
                }
                let matches = tokio::time::timeout(FILE_SEARCH_RPC_TIMEOUT, async {
                    let root = self.file_search_root(&p).await?;
                    let featured_paths = p
                        .chat_id
                        .as_deref()
                        .filter(|_| p.query.is_empty())
                        .map(|chat_id| self.featured_file_paths(chat_id))
                        .unwrap_or_default();
                    self.repos
                        .search_files(root, p.query, featured_paths)
                        .await
                        .map_err(|e| RpcError::Failed(e.to_string()))
                })
                .await
                .map_err(|_| RpcError::Failed("file search timed out".into()))??;
                RpcReply::value(&matches)
            }
            methods::GET_WORKING_TREE => {
                Box::pin(async move {
                    let request: zeron_proto::GetWorkingTreeRequest = parse_params(params)?;
                    let mut tree = tokio::time::timeout(crate::working_tree::READ_TIMEOUT, async {
                        let workspace = self.source_control_checkout(&request.target).await?;
                        crate::working_tree::read(&workspace.root, &workspace.checkout_id)
                            .await
                            .map_err(|error| RpcError::Failed(error.to_string()))
                    })
                    .await
                    .map_err(|_| RpcError::Failed("working tree read timed out".into()))??;
                    tree.agent_active = self.agent_active_on_checkout(&tree.checkout_id).await;
                    RpcReply::value(&tree)
                })
                .await
            }
            methods::GET_WORKING_TREE_FILE_DIFF => {
                Box::pin(async move {
                    let request: zeron_proto::GetWorkingTreeFileDiffRequest =
                        parse_params(params)?;
                    let diff = tokio::time::timeout(crate::working_tree::READ_TIMEOUT, async {
                        let workspace = self.source_control_checkout(&request.target).await?;
                        crate::working_tree::file_diff(
                            &workspace.root,
                            &workspace.checkout_id,
                            &request.path,
                            request.staged,
                        )
                        .await
                        .map_err(|error| RpcError::Failed(error.to_string()))
                    })
                    .await
                    .map_err(|_| RpcError::Failed("file diff read timed out".into()))??;
                    RpcReply::value(&diff)
                })
                .await
            }
            methods::RUN_WORKING_TREE_ACTION => {
                Box::pin(async move {
                    let request: zeron_proto::RunWorkingTreeActionRequest = parse_params(params)?;
                    let workspace = self.source_control_checkout(&request.target).await?;
                    if let Some(folder) = self
                        .project_folder_in_checkout(&request.target, &workspace.root)
                        .await
                    {
                        crate::working_tree::refuse_outside_folder(&request.action, &folder)
                            .map_err(|error| RpcError::Failed(error.to_string()))?;
                    }
                    let reply = tokio::time::timeout(crate::working_tree::WRITE_TIMEOUT, async {
                        let _guard =
                            crate::working_tree::lock_checkout_briefly(&workspace.checkout_id)
                                .await
                                .map_err(|error| RpcError::Failed(error.to_string()))?;
                        self.refuse_if_agent_busy(
                            &workspace.checkout_id,
                            request.busy_policy.allows(&request.action),
                            request.busy_policy,
                            "An agent is running in this checkout. Only stage, unstage and commit of staged files run until it stops (Settings > Source Control).",
                        )
                        .await?;
                        let tree =
                            crate::working_tree::read(&workspace.root, &workspace.checkout_id)
                                .await
                                .map_err(|error| RpcError::Failed(error.to_string()))?;
                        if tree.revision != request.expected_revision {
                            return Err(RpcError::Failed(
                                "The working tree changed since it was shown. Check the refreshed list and try again."
                                    .into(),
                            ));
                        }
                        let result =
                            crate::working_tree::apply(&workspace.root, &tree, &request.action)
                                .await
                                .map_err(|error| RpcError::Failed(error.to_string()));
                        self.diff_sync.sync_all();
                        result
                    })
                    .await
                    .map_err(|_| RpcError::Failed("the Git action timed out".into()))??;
                    RpcReply::value(&reply)
                })
                .await
            }
            methods::SYNC_BRANCH => {
                Box::pin(async move {
                    let request: zeron_proto::SyncBranchRequest = parse_params(params)?;
                    let workspace = self.source_control_checkout(&request.target).await?;
                    let _guard = crate::working_tree::lock_checkout_briefly(&workspace.checkout_id)
                        .await
                        .map_err(|error| RpcError::Failed(error.to_string()))?;
                    self.refuse_if_agent_busy(
                        &workspace.checkout_id,
                        request.busy_policy.allows_sync(request.direction),
                        request.busy_policy,
                        "An agent is running in this checkout. Pull waits until it stops, so it cannot rewrite files under the agent (Settings > Source Control).",
                    )
                    .await?;
                    let tree = read_tree_timed(&workspace).await?;
                    if tree.head_sha.as_deref() != Some(request.expected_head_sha.as_str()) {
                        return Err(RpcError::Failed(
                            "The branch moved since it was shown. Check the refreshed state and try again."
                                .into(),
                        ));
                    }
                    let result = crate::working_tree::sync(
                        &workspace.root,
                        &tree,
                        request.direction,
                        crate::working_tree::SYNC_TIMEOUT,
                    )
                    .await
                    .map_err(|error| RpcError::Failed(error.to_string()));
                    self.diff_sync.sync_all();
                    let commits = result?;
                    let after = read_tree_timed(&workspace).await?;
                    RpcReply::value(&zeron_proto::SyncBranchReply {
                        head_sha: after.head_sha.unwrap_or_default(),
                        upstream: after.upstream,
                        ahead: after.ahead,
                        behind: after.behind,
                        commits,
                    })
                })
                .await
            }
            methods::GENERATE_COMMIT_MESSAGE => {
                Box::pin(async move {
                    let request: zeron_proto::GenerateCommitMessageRequest = parse_params(params)?;
                    let workspace = self.source_control_checkout(&request.target).await?;
                    let generated = tokio::time::timeout(
                        crate::commit_messages::GENERATE_TIMEOUT,
                        crate::commit_messages::generate(&self.registry, &workspace.root, &request),
                    )
                    .await
                    .map_err(|_| {
                        RpcError::Failed(
                            "Writing the commit message took too long. Try again, or pick a faster model."
                                .into(),
                        )
                    })?
                    .map_err(|error| RpcError::Failed(error.to_string()))?;
                    RpcReply::value(&generated)
                })
                .await
            }
            methods::LIST_WORKSPACE_DIRECTORY => {
                let request: zeron_proto::ListWorkspaceDirectoryRequest = parse_params(params)?;
                let page = tokio::time::timeout(
                    crate::workspace_files::WORKSPACE_FILE_RPC_TIMEOUT,
                    self.workspace_files.list_directory(request),
                )
                .await
                .map_err(|_| RpcError::Failed("workspace directory listing timed out".into()))?
                .map_err(RpcError::from)?;
                RpcReply::value(&page)
            }
            methods::SEARCH_WORKSPACE_FILES => {
                let request: zeron_proto::SearchWorkspaceFilesRequest = parse_params(params)?;
                let matches = tokio::time::timeout(
                    crate::workspace_files::WORKSPACE_FILE_RPC_TIMEOUT,
                    self.workspace_files.search(request),
                )
                .await
                .map_err(|_| RpcError::Failed("workspace file search timed out".into()))?
                .map_err(RpcError::from)?;
                RpcReply::value(&matches)
            }
            methods::SEARCH_WORKSPACE_CONTENT => {
                let request: zeron_proto::SearchWorkspaceContentRequest = parse_params(params)?;
                let result = tokio::time::timeout(
                    crate::workspace_files::WORKSPACE_FILE_RPC_TIMEOUT,
                    self.workspace_files.search_content(request),
                )
                .await
                .map_err(|_| RpcError::Failed("workspace content search timed out".into()))?
                .map_err(RpcError::from)?;
                RpcReply::value(&result)
            }
            methods::WARM_WORKSPACE_SEARCH => {
                let request: zeron_proto::WarmWorkspaceSearchRequest = parse_params(params)?;
                let result = tokio::time::timeout(
                    crate::workspace_files::WORKSPACE_FILE_RPC_TIMEOUT,
                    self.workspace_files.warm_search(request),
                )
                .await
                .map_err(|_| RpcError::Failed("workspace search warm-up timed out".into()))?
                .map_err(RpcError::from)?;
                RpcReply::value(&result)
            }
            methods::READ_WORKSPACE_IMAGE => {
                let request: zeron_proto::ReadWorkspaceImageRequest = parse_params(params)?;
                let chunk = tokio::time::timeout(
                    crate::workspace_files::WORKSPACE_FILE_RPC_TIMEOUT,
                    self.workspace_files.read_image(request),
                )
                .await
                .map_err(|_| RpcError::Failed("Workspace image read timed out".into()))?
                .map_err(RpcError::from)?;
                RpcReply::value(&chunk)
            }
            methods::READ_WORKSPACE_FILE => {
                let request: zeron_proto::ReadWorkspaceFileRequest = parse_params(params)?;
                let file = tokio::time::timeout(
                    crate::workspace_files::WORKSPACE_FILE_RPC_TIMEOUT,
                    self.workspace_files.read_file(request),
                )
                .await
                .map_err(|_| RpcError::Failed("workspace file read timed out".into()))?
                .map_err(RpcError::from)?;
                RpcReply::value(&file)
            }
            methods::DELETE_WORKSPACE_ENTRY => {
                let request: zeron_proto::DeleteWorkspaceEntryRequest = parse_params(params)?;
                let outcome = self
                    .workspace_files
                    .delete_entry(request)
                    .await
                    .map_err(RpcError::from)?;
                RpcReply::value(&outcome)
            }
            methods::MOVE_WORKSPACE_ENTRY => {
                let request: zeron_proto::MoveWorkspaceEntryRequest = parse_params(params)?;
                let outcome = self
                    .workspace_files
                    .move_entry(request)
                    .await
                    .map_err(RpcError::from)?;
                RpcReply::value(&outcome)
            }
            methods::WRITE_WORKSPACE_FILE => {
                let request: zeron_proto::WriteWorkspaceFileRequest = parse_params(params)?;
                let outcome = tokio::time::timeout(
                    crate::workspace_files::WORKSPACE_FILE_RPC_TIMEOUT,
                    self.workspace_files.write_file(request),
                )
                .await
                .map_err(|_| RpcError::Failed("workspace file write timed out".into()))?
                .map_err(RpcError::from)?;
                RpcReply::value(&outcome)
            }
            methods::WATCH_WORKSPACE_FILES => {
                let request: zeron_proto::WatchWorkspaceFilesRequest = parse_params(params)?;
                let subscription = self
                    .workspace_files
                    .watch_files(request)
                    .await
                    .map_err(RpcError::from)?;
                let stream = futures::stream::unfold(subscription, |mut subscription| async move {
                    let changes = subscription.recv().await?;
                    let value = serde_json::to_value(changes).ok()?;
                    Some((value, subscription))
                });
                Ok(RpcReply::Stream(stream.boxed()))
            }
            methods::CREATE_WORKTREE => {
                let p: CreateWorktreeParams = parse_params(params)?;
                let setup_space = match p.space_id.as_deref() {
                    Some(space_id) => {
                        let space = self.local_project_action_space(space_id)?;
                        let space_root = std::fs::canonicalize(&space.path)
                            .map_err(|_| RpcError::Failed("Project root is unavailable".into()))?;
                        let repo_root = std::fs::canonicalize(&p.repo_path).map_err(|_| {
                            RpcError::Failed("Worktree repository is unavailable".into())
                        })?;
                        if space_root != repo_root {
                            return Err(RpcError::Failed(
                                "Worktree repository does not match project space".into(),
                            ));
                        }
                        Some((space, space_root))
                    }
                    None => None,
                };
                let worktree = self
                    .repos
                    .create_worktree(std::path::Path::new(&p.repo_path), &p.branch)
                    .await
                    .map_err(|e| RpcError::Failed(e.to_string()))?;
                let mut outcome = CreateWorktreeOutcome {
                    worktree,
                    setup_action: None,
                    setup_error: None,
                };
                if let Some((space, project_root)) = setup_space {
                    match self
                        .project_actions
                        .setup_action(&space.id, std::path::Path::new(&space.path))
                    {
                        Ok(Some(action)) => {
                            let worktree_root = std::fs::canonicalize(&outcome.worktree.path)
                                .unwrap_or_else(|_| outcome.worktree.path.clone().into());
                            match crate::project_actions::launch_project_setup_action(
                                &self.terminals,
                                &action,
                                &project_root,
                                &worktree_root,
                                80,
                                24,
                            ) {
                                Ok(run) => outcome.setup_action = Some(run),
                                Err(err) => {
                                    tracing::warn!(
                                        space_id = %space.id,
                                        worktree = %outcome.worktree.path,
                                        error = %err,
                                        "failed to start project setup Action"
                                    );
                                    outcome.setup_error = Some(err.to_string());
                                }
                            }
                        }
                        Ok(None) => {}
                        Err(err) => {
                            tracing::warn!(
                                space_id = %space.id,
                                error = %err,
                                "failed to resolve project setup Action"
                            );
                            outcome.setup_error = Some(err.to_string());
                        }
                    }
                }
                RpcReply::value(&outcome)
            }
            methods::DELETE_WORKTREE => {
                let p: DeleteWorktreeParams = parse_params(params)?;
                self.repos
                    .delete_worktree(
                        std::path::Path::new(&p.repo_path),
                        std::path::Path::new(&p.worktree_path),
                    )
                    .await
                    .map_err(|e| RpcError::Failed(e.to_string()))?;
                RpcReply::value(&serde_json::json!({ "ok": true }))
            }
            methods::GET_SYSTEM_PROMPT => {
                RpcReply::value(&serde_json::json!({ "prompt": self.registry.system_prompt() }))
            }
            methods::SET_SYSTEM_PROMPT => {
                let p: SetSystemPromptParams = parse_params(params)?;
                self.registry
                    .set_system_prompt(&p.prompt)
                    .map_err(RpcError::Failed)?;
                RpcReply::value(&serde_json::json!({ "prompt": self.registry.system_prompt() }))
            }
            methods::LIST_AGENT_PROFILES => {
                RpcReply::value(&self.agent_profiles_store()?.snapshot())
            }
            methods::UPSERT_AGENT_PROFILE => {
                let p: UpsertAgentProfileParams = parse_params(params)?;
                let store = self.agent_profiles_store()?;
                let snapshot = blocking(move || store.upsert_as(p.profile_id.as_deref(), p.profile, p.by_agent))
                    .await?;
                RpcReply::value(&snapshot)
            }
            methods::DELETE_AGENT_PROFILE => {
                let p: DeleteAgentProfileParams = parse_params(params)?;
                let store = self.agent_profiles_store()?;
                let profile_id = p.profile_id.clone();
                let snapshot = blocking(move || store.delete(&p.profile_id)).await?;
                self.detach_profile_from_chats(&profile_id)?;
                RpcReply::value(&snapshot)
            }
            methods::REORDER_AGENT_PROFILES => {
                let p: ReorderAgentProfilesParams = parse_params(params)?;
                let store = self.agent_profiles_store()?;
                let snapshot = blocking(move || store.reorder(&p.profile_ids)).await?;
                RpcReply::value(&snapshot)
            }
            methods::LIST_PROVIDER_INSTANCES => {
                RpcReply::value(&self.provider_instances_store()?.snapshot())
            }
            methods::WATCH_PROVIDER_INSTANCES => Ok(RpcReply::Stream(watch_stream(
                self.provider_instances_store()?.watch(),
            ))),
            methods::UPSERT_PROVIDER_INSTANCE => {
                let p: UpsertProviderInstanceParams = parse_params(params)?;
                let store = self.provider_instances_store()?;
                let before = store.instance(p.instance.id.trim());
                let writer = store.clone();
                let snapshot = blocking(move || writer.upsert(p.instance)).await?;
                if let Some(before) = before
                    && let Some(after) = store.instance(&before.id)
                    && !crate::provider_instances::keeps_sessions(&before, &after, store.data_dir())
                {
                    self.retire_instance_sessions(&before.id).await;
                }
                RpcReply::value(&snapshot)
            }
            methods::DELETE_PROVIDER_INSTANCE => {
                let p: DeleteProviderInstanceParams = parse_params(params)?;
                let store = self.provider_instances_store()?;
                let deleted = store.instance(&p.instance_id);
                let snapshot = blocking(move || store.delete(&p.instance_id)).await?;
                if let Some(instance) = deleted {
                    self.retire_instance_sessions(&instance.id).await;
                    self.agent_accounts.remove_instance(&instance).await;
                }
                RpcReply::value(&snapshot)
            }
            methods::LIST_ACP_AUTH_METHODS => {
                let p: ListAcpAuthMethodsParams = parse_params(params)?;
                let acp = self
                    .provider_instances_store()?
                    .acp_harness(&p.instance_id)
                    .map_err(|e| RpcError::Failed(e.to_string()))?;
                let auth_methods = acp
                    .auth_methods()
                    .await
                    .map_err(|e| RpcError::Failed(e.to_string()))?;
                RpcReply::value(&auth_methods)
            }
            methods::SEARCH_ACP_REGISTRY => {
                let p: SearchAcpRegistryParams = parse_params(params)?;
                let store = self.provider_instances_store()?;
                let agents = crate::acp_registry::search(store.data_dir(), p.query.as_deref())
                    .await
                    .map_err(|e| RpcError::Failed(e.to_string()))?;
                RpcReply::value(&agents)
            }
            methods::PREPARE_ACP_AGENT => {
                let p: PrepareAcpAgentParams = parse_params(params)?;
                let store = self.provider_instances_store()?;
                let prepared = crate::acp_registry::prepare(
                    store.data_dir(),
                    &p.agent_id,
                    p.distribution,
                    p.allow_unverified,
                )
                .await
                .map_err(|e| RpcError::Failed(e.to_string()))?;
                RpcReply::value(&prepared)
            }
            methods::LIST_PROJECT_ACTIONS => {
                let p: ListProjectActionsParams = parse_params(params)?;
                let space = self.local_project_action_space(&p.space_id)?;
                let actions = self.project_actions.clone();
                // Snapshots discover repository files; keep all filesystem work
                // (including mutation persistence below) off the async worker.
                let snapshot = tokio::task::spawn_blocking(move || {
                    actions.snapshot(&space.id, std::path::Path::new(&space.path))
                })
                .await
                .map_err(|err| RpcError::Failed(err.to_string()))?
                .map_err(|err| RpcError::Failed(err.to_string()))?;
                RpcReply::value(&snapshot)
            }
            methods::UPSERT_PROJECT_ACTION => {
                let p: UpsertProjectActionParams = parse_params(params)?;
                let space = self.local_project_action_space(&p.space_id)?;
                let actions = self.project_actions.clone();
                let snapshot = tokio::task::spawn_blocking(move || {
                    actions.upsert(
                        &space.id,
                        std::path::Path::new(&space.path),
                        p.action_id.as_deref(),
                        p.action,
                    )
                })
                .await
                .map_err(|err| RpcError::Failed(err.to_string()))?
                .map_err(|err| RpcError::Failed(err.to_string()))?;
                RpcReply::value(&snapshot)
            }
            methods::DELETE_PROJECT_ACTION => {
                let p: DeleteProjectActionParams = parse_params(params)?;
                let space = self.local_project_action_space(&p.space_id)?;
                let actions = self.project_actions.clone();
                let snapshot = tokio::task::spawn_blocking(move || {
                    actions.delete(&space.id, std::path::Path::new(&space.path), &p.action_id)
                })
                .await
                .map_err(|err| RpcError::Failed(err.to_string()))?
                .map_err(|err| RpcError::Failed(err.to_string()))?;
                RpcReply::value(&snapshot)
            }
            methods::RUN_PROJECT_ACTION => {
                let p: RunProjectActionParams = parse_params(params)?;
                let space = self.local_project_action_space(&p.space_id)?;
                let chat = self
                    .workspace
                    .chat(&p.chat_id)
                    .map_err(|err| RpcError::Failed(err.to_string()))?
                    .ok_or_else(|| RpcError::Failed("Project chat not found".into()))?;
                if chat.device_id != self.doc_host.device_id() {
                    return Err(RpcError::Failed(
                        "Project chat belongs to another device".into(),
                    ));
                }
                if chat.space_id.as_deref() != Some(space.id.as_str()) {
                    return Err(RpcError::Failed(
                        "Project chat belongs to another space".into(),
                    ));
                }
                let cwd = chat
                    .cwd
                    .map(std::path::PathBuf::from)
                    .ok_or_else(|| RpcError::Failed("Project chat has no checkout".into()))?;
                let checkout = self
                    .repos
                    .workspace_checkout(std::path::Path::new(&space.path), &cwd)
                    .await
                    .ok_or_else(|| {
                        RpcError::Failed("Project chat checkout is unavailable".into())
                    })?;
                let project_root = std::fs::canonicalize(&space.path)
                    .map_err(|_| RpcError::Failed("Project root is unavailable".into()))?;
                let action = self
                    .project_actions
                    .action(&space.id, std::path::Path::new(&space.path), &p.action_id)
                    .map_err(|err| RpcError::Failed(err.to_string()))?
                    .ok_or_else(|| RpcError::Failed("Project action not found".into()))?;
                let run = crate::project_actions::launch_project_action(
                    &self.terminals,
                    &action,
                    &project_root,
                    &checkout,
                    p.cols,
                    p.rows,
                )
                .map_err(|err| RpcError::Failed(err.to_string()))?;
                let _ = self.terminals.assign_chat(&run.terminal.id, &p.chat_id);
                RpcReply::value(&run)
            }
            methods::OPEN_TERMINAL => {
                let p: OpenTerminalParams = parse_params(params)?;
                // Prefer an explicit real path (new-chat canvas has no row
                // yet). `space-canvas:{spaceId}` names the selected project
                // so a missing/tilde cwd still lands in that folder.
                let chat_cwd = self
                    .workspace
                    .chat(&p.chat_id)
                    .ok()
                    .flatten()
                    .and_then(|chat| chat.cwd);
                let space_cwd = canvas_space_id(&p.chat_id).and_then(|space_id| {
                    self.workspace
                        .space(space_id)
                        .ok()
                        .flatten()
                        .map(|space| space.path)
                });
                let cwd = resolve_open_terminal_cwd(
                    p.cwd,
                    chat_cwd,
                    space_cwd,
                    &self.repos.scratch_root(),
                )
                .map_err(|error| RpcError::Failed(error.to_string()))?;
                let session = match &p.command {
                    Some(command) => self.terminals.open_with_command(
                        &cwd,
                        p.cols,
                        p.rows,
                        &std::collections::HashMap::new(),
                        command,
                    ),
                    None => self.terminals.open(&cwd, p.cols, p.rows),
                }
                .map_err(|e| RpcError::Failed(e.to_string()))?;
                self.terminals
                    .assign_chat(&session.id, &p.chat_id)
                    .map_err(|e| RpcError::Failed(e.to_string()))?;
                if p.agent {
                    self.terminals
                        .announce_agent_open(&p.chat_id, session.clone());
                }
                RpcReply::value(&session)
            }
            methods::LIST_TERMINALS => {
                let p: ChatParams = parse_params(params)?;
                RpcReply::value(&self.terminals.list_for_chat(&p.chat_id))
            }
            methods::READ_TERMINAL => {
                let p: ReadTerminalParams = parse_params(params)?;
                let screen = self
                    .terminals
                    .read(
                        &p.terminal_id,
                        &p.chat_id,
                        p.lines.unwrap_or(80).clamp(1, 2000),
                    )
                    .map_err(|e| RpcError::Failed(e.to_string()))?;
                RpcReply::value(&screen)
            }
            methods::SEND_TERMINAL_INPUT => {
                let p: SendTerminalInputParams = parse_params(params)?;
                self.terminals
                    .send_input(&p.terminal_id, &p.chat_id, p.text.as_deref(), &p.keys)
                    .map_err(|e| RpcError::Failed(e.to_string()))?;
                RpcReply::value(&serde_json::json!({ "ok": true }))
            }
            methods::SET_TERMINAL_SELECTION => {
                let p: SetTerminalSelectionParams = parse_params(params)?;
                self.terminals
                    .set_selection(&p.terminal_id, p.text)
                    .map_err(|e| RpcError::Failed(e.to_string()))?;
                RpcReply::value(&serde_json::json!({ "ok": true }))
            }
            methods::WATCH_AGENT_TERMINALS => {
                let opened = self.terminals.subscribe_agent_opened();
                Ok(RpcReply::Stream(
                    futures::stream::unfold(opened, |mut opened| async move {
                        loop {
                            match opened.recv().await {
                                Ok(event) => {
                                    if let Ok(value) = serde_json::to_value(&event) {
                                        return Some((value, opened));
                                    }
                                }
                                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                                Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                                    return None;
                                }
                            }
                        }
                    })
                    .boxed(),
                ))
            }
            methods::SUBSCRIBE_TERMINAL => {
                let p: SubscribeTerminalParams = parse_params(params)?;
                let rx = self
                    .terminals
                    .subscribe(&p.terminal_id, p.after_seq)
                    .map_err(|e| RpcError::Failed(e.to_string()))?;
                let stream = futures::stream::unfold(rx, |mut rx| async move {
                    let event = rx.recv().await?;
                    let value = serde_json::to_value(&event).ok()?;
                    Some((value, rx))
                });
                Ok(RpcReply::Stream(stream.boxed()))
            }
            methods::WRITE_TERMINAL => {
                let p: WriteTerminalParams = parse_params(params)?;
                self.terminals
                    .write(&p.terminal_id, &p.data)
                    .map_err(|e| RpcError::Failed(e.to_string()))?;
                RpcReply::value(&serde_json::json!({ "ok": true }))
            }
            methods::RESIZE_TERMINAL => {
                let p: ResizeTerminalParams = parse_params(params)?;
                self.terminals
                    .resize(&p.terminal_id, p.cols, p.rows)
                    .map_err(|e| RpcError::Failed(e.to_string()))?;
                RpcReply::value(&serde_json::json!({ "ok": true }))
            }
            methods::CLOSE_TERMINAL => {
                let p: TerminalIdParams = parse_params(params)?;
                self.terminals
                    .close(&p.terminal_id)
                    .map_err(|e| RpcError::Failed(e.to_string()))?;
                RpcReply::value(&serde_json::json!({ "ok": true }))
            }
            methods::LIST_AGENT_ACCOUNTS => {
                let p: ListAgentAccountsParams = parse_params(params)?;
                let (accounts, driver) = self.agent_accounts_for(p.instance_id.as_deref(), None)?;
                let mut snapshot = accounts
                    .list(p.force_usage.unwrap_or(false))
                    .await
                    .map_err(|e| RpcError::Failed(e.to_string()))?;
                if let Some(driver) = driver {
                    snapshot.accounts.retain(|account| account.harness == driver);
                    snapshot.warnings.retain(|warning| warning.harness == driver);
                }
                RpcReply::value(&snapshot)
            }
            methods::ACTIVATE_AGENT_ACCOUNT => {
                self.voice.retire();
                let p: AgentAccountParams = parse_params(params)?;
                let (accounts, _) =
                    self.agent_accounts_for(p.instance_id.as_deref(), Some(p.harness))?;
                let snapshot = accounts
                    .activate(p.harness, &p.account_id)
                    .await
                    .map_err(|e| RpcError::Failed(e.to_string()))?;
                RpcReply::value(&snapshot)
            }
            methods::FORGET_AGENT_ACCOUNT => {
                self.voice.retire();
                let p: AgentAccountParams = parse_params(params)?;
                let (accounts, _) =
                    self.agent_accounts_for(p.instance_id.as_deref(), Some(p.harness))?;
                let snapshot = accounts
                    .forget(p.harness, &p.account_id)
                    .await
                    .map_err(|e| RpcError::Failed(e.to_string()))?;
                RpcReply::value(&snapshot)
            }
            methods::START_AGENT_LOGIN => {
                let p: StartAgentLoginParams = parse_params(params)?;
                // A requester naming this device is no remote login at all:
                // publishing a callback route for ourselves would be a no-op
                // at best, so never register one.
                let own_id = self.doc_host.device_id();
                let requester = p
                    .requester_device_id
                    .as_deref()
                    .filter(|requester| !requester.is_empty() && *requester != own_id);
                if let Some(instance) = self.acp_instance(p.instance_id.as_deref())? {
                    if instance.driver != p.harness {
                        return Err(RpcError::Failed(format!(
                            "Provider instance '{}' runs on {}, not {}",
                            instance.id,
                            instance.driver.display_name(),
                            p.harness.display_name()
                        )));
                    }
                    let acp = self
                        .provider_instances_store()?
                        .acp_harness(&instance.id)
                        .map_err(|e| RpcError::Failed(e.to_string()))?;
                    let start = self.agent_accounts.start_acp_instance_login(
                        p.harness,
                        acp,
                        p.method_id,
                        requester,
                    );
                    return RpcReply::value(&start);
                }
                let (accounts, _) =
                    self.agent_accounts_for(p.instance_id.as_deref(), Some(p.harness))?;
                let start = accounts
                    .start_login_with(p.harness, p.provider.as_deref(), requester)
                    .await
                    .map_err(|e| RpcError::Failed(e.to_string()))?;
                RpcReply::value(&start)
            }
            methods::COMPLETE_AGENT_LOGIN => {
                let p: CompleteAgentLoginParams = parse_params(params)?;
                let (accounts, _) = self.agent_accounts_for(p.instance_id.as_deref(), None)?;
                let snapshot = accounts
                    .complete_login(&p.login_id, &p.code)
                    .await
                    .map_err(|e| RpcError::Failed(e.to_string()))?;
                RpcReply::value(&snapshot)
            }
            methods::POLL_AGENT_LOGIN => {
                let p: LoginIdParams = parse_params(params)?;
                let accounts = self.login_accounts_for(p.instance_id.as_deref())?;
                let poll = accounts
                    .poll_login(&p.login_id)
                    .await
                    .map_err(|e| RpcError::Failed(e.to_string()))?;
                RpcReply::value(&poll)
            }
            methods::CANCEL_AGENT_LOGIN => {
                let p: LoginIdParams = parse_params(params)?;
                let accounts = self.login_accounts_for(p.instance_id.as_deref())?;
                accounts.cancel_login(&p.login_id);
                RpcReply::value(&serde_json::json!({ "ok": true }))
            }
            methods::UPLOAD_CHUNK => {
                let p: UploadChunkParams = parse_params(params)?;
                self.uploads
                    .append(&p.upload_id, &p.data, p.seq)
                    .map_err(|e| RpcError::Failed(e.to_string()))?;
                RpcReply::value(&serde_json::json!({ "ok": true }))
            }
            methods::UPLOAD_COMMIT => {
                let p: UploadCommitParams = parse_params(params)?;
                let path = self
                    .uploads
                    .commit(&p.upload_id, &p.file_name)
                    .map_err(|e| RpcError::Failed(e.to_string()))?;
                // Bytes just landed on this device: any command deferred on
                // them (queued-attachment refs) is executable NOW.
                self.doc_host.kick_drains();
                RpcReply::value(&serde_json::json!({ "path": path }))
            }
            methods::READ_ATTACHMENT_CHUNK => {
                let p: ReadAttachmentChunkParams = parse_params(params)?;
                // Path jail: the uploads dir plus every workspace-known chat cwd.
                let roots: Vec<std::path::PathBuf> = self
                    .workspace
                    .read_chats()
                    .unwrap_or_default()
                    .into_iter()
                    .filter_map(|chat| chat.cwd)
                    .map(std::path::PathBuf::from)
                    .collect();
                let uploads = self.uploads.clone();
                let chunk = tokio::task::spawn_blocking(move || {
                    uploads.read_chunk(&p.path, p.offset, &roots)
                })
                .await
                .map_err(|e| RpcError::Failed(e.to_string()))?
                .map_err(|e| RpcError::Failed(e.to_string()))?;
                RpcReply::value(&chunk)
            }
            methods::PUBLISH_VISUAL => {
                let p: PublishVisualParams = parse_params(params)?;
                let title: String = p
                    .title
                    .trim()
                    .chars()
                    .take(zeron_proto::VISUAL_MAX_TITLE_CHARS)
                    .collect();
                if title.is_empty() {
                    return Err(RpcError::BadParams("title is empty".into()));
                }
                if p.script.trim().is_empty() || p.script.len() > zeron_proto::VISUAL_MAX_BYTES {
                    return Err(RpcError::BadParams(format!(
                        "script must be 1 to {} bytes",
                        zeron_proto::VISUAL_MAX_BYTES
                    )));
                }
                // Before decoding and storing anything (checked again below).
                if !self.sessions.has_run(&p.chat_id) {
                    return Err(no_turn_for_visual());
                }
                let id = format!("v:{}", crate::new_id());
                let assets = decode_visual_assets(&p.assets)?;
                // Run the script the way the desktop will (setup, render,
                // every draw, every control) before anyone sees it, so the
                // agent gets its errors with line numbers and can fix them.
                let uploads = self.uploads.clone();
                let (stored_id, script) = (id.clone(), p.script);
                let (path, report) = tokio::task::spawn_blocking(move || {
                    let files = zeron_visual::Assets::new(
                        assets
                            .iter()
                            .map(|(name, bytes)| (name.clone(), bytes.as_slice().into()))
                            .collect(),
                    );
                    let checked = zeron_visual::check_with_assets(
                        &script,
                        zeron_visual::seed_for(&stored_id),
                        files,
                    );
                    let report = checked
                        .map_err(|e| RpcError::BadParams(format!("visual script failed: {e}")))?;
                    // Assets first: a script is only ever found with its files.
                    uploads
                        .store_visual_assets(&stored_id, &assets)
                        .map_err(|e| RpcError::Failed(e.to_string()))?;
                    let path = uploads.store_visual(&stored_id, &script).map_err(|e| {
                        uploads.remove_visual(&stored_id);
                        RpcError::Failed(e.to_string())
                    })?;
                    Ok::<_, RpcError>((path, report))
                })
                .await
                .map_err(|e| RpcError::Failed(e.to_string()))??;
                let event = AgentEvent::Visual {
                    id: id.clone(),
                    path: path.clone(),
                    title: title.clone(),
                };
                if !self.sessions.inject_event(&p.chat_id, event) {
                    self.uploads.remove_visual(&id);
                    return Err(no_turn_for_visual());
                }
                RpcReply::value(&serde_json::json!({
                    "id": id,
                    "title": title,
                    "checkedInteractions": report.interactions,
                    "printed": report.logs,
                }))
            }
            methods::FETCH_TOOL_BLOB => {
                let p: FetchToolBlobParams = parse_params(params)?;
                // A diff this host produced is answered from its own run
                // journal; the edge sidecar (parked) is the remote fallback.
                if let Some((chat, part)) = p
                    .blob_ref
                    .strip_suffix(".diff")
                    .and_then(|r| r.split_once('/'))
                    && let Some(diff) = self.sessions.journaled_tool_diff(chat, part)
                {
                    let text = serde_json::to_string(&diff)
                        .map_err(|e| RpcError::Failed(e.to_string()))?;
                    return RpcReply::value(&serde_json::json!({ "text": text }));
                }
                let text = self
                    .doc_host
                    .fetch_tool_blob(&p.blob_ref)
                    .await
                    .map_err(|e| RpcError::Failed(e.to_string()))?;
                RpcReply::value(&serde_json::json!({ "text": text }))
            }
            methods::WATCH_COMPUTER_HOST_REQUESTS => {
                let p: zeron_proto::computer::WatchComputerHostRequestsParams =
                    parse_params(params)?;
                let items = self.computer()?.register_host(p.bundle_id);
                Ok(RpcReply::Stream(
                    items
                        .filter_map(|item| async move { serde_json::to_value(item).ok() })
                        .boxed(),
                ))
            }
            methods::RESPOND_COMPUTER_HOST_REQUEST => {
                let p: zeron_proto::computer::RespondComputerHostRequestParams =
                    parse_params(params)?;
                self.computer()?.respond(p).map_err(RpcError::Failed)?;
                RpcReply::value(&serde_json::json!({}))
            }
            methods::CANCEL_COMPUTER_ACTION => {
                let p: zeron_proto::computer::CancelComputerActionParams = parse_params(params)?;
                let cancelled = crate::computer::tool_calls::cancel_computer_action(
                    self.computer()?,
                    &self.sessions,
                    &p,
                )
                .map_err(RpcError::Failed)?;
                RpcReply::value(&serde_json::json!({ "cancelled": cancelled }))
            }
            methods::GET_COMPUTER_CATALOG => {
                let p: zeron_proto::computer::GetComputerCatalogParams = parse_params(params)?;
                let catalog = crate::computer::tool_calls::computer_catalog(
                    self.computer()?,
                    &self.sessions,
                    &p,
                )
                .map_err(RpcError::Failed)?;
                RpcReply::value(&catalog)
            }
            methods::COMPUTER_ACTION => {
                let p: zeron_proto::computer::ComputerActionParams = parse_params(params)?;
                let result =
                    crate::computer::tool_calls::computer_action(self.computer()?, &self.sessions, p)
                        .await
                        .map_err(RpcError::Failed)?;
                RpcReply::value(&result)
            }
            methods::WATCH_DEVICE_HUB => {
                let states = self.device_hub()?.watch();
                Ok(RpcReply::Stream(
                    futures::stream::unfold((states, true), |(mut states, first)| async move {
                        if !first {
                            states.changed().await.ok()?;
                        }
                        let value = serde_json::to_value(&*states.borrow_and_update()).ok()?;
                        Some((value, (states, false)))
                    })
                    .boxed(),
                ))
            }
            methods::CONFIGURE_DEVICE_HUB => {
                let p: zeron_proto::device_hub::ConfigureDeviceHubParams = parse_params(params)?;
                let state = self
                    .device_hub()?
                    .configure(p)
                    .await
                    .map_err(RpcError::Failed)?;
                RpcReply::value(&state)
            }
            methods::LIST_HUB_DEVICES => {
                let p: zeron_proto::device_hub::ListHubDevicesParams = parse_params(params)?;
                let state = self
                    .device_hub()?
                    .list(p.inspect_only)
                    .await
                    .map_err(RpcError::Failed)?;
                RpcReply::value(&state)
            }
            methods::OPEN_HUB_DEVICE => {
                let p: zeron_proto::device_hub::OpenHubDeviceParams = parse_params(params)?;
                let session = self.device_hub()?.open(p).await.map_err(RpcError::Failed)?;
                RpcReply::value(&session)
            }
            methods::CLOSE_HUB_DEVICE => {
                let p: zeron_proto::device_hub::CloseHubDeviceParams = parse_params(params)?;
                self.device_hub()?
                    .close(p)
                    .await
                    .map_err(RpcError::Failed)?;
                RpcReply::value(&serde_json::json!({}))
            }
            methods::SHUTDOWN_HUB_DEVICE => {
                let p: zeron_proto::device_hub::ShutdownHubDeviceParams = parse_params(params)?;
                self.device_hub()?
                    .power_off(p)
                    .await
                    .map_err(RpcError::Failed)?;
                RpcReply::value(&serde_json::json!({}))
            }
            methods::WATCH_HUB_DEVICE_FRAMES => {
                use base64::Engine as _;
                let p: zeron_proto::device_hub::WatchHubDeviceFramesParams = parse_params(params)?;
                if p.panel.is_some() && p.codec != zeron_proto::device_hub::DeviceVideoCodec::H264
                {
                    return Err(RpcError::Failed("A display feed needs H.264.".into()));
                }
                if p.codec == zeron_proto::device_hub::DeviceVideoCodec::H264 {
                    let subscription = self
                        .device_hub()?
                        .watch_avcc(p.host_id.as_deref(), &p.device_id, p.panel)
                        .map_err(RpcError::Failed)?;
                    let mut seq = 0;
                    return Ok(RpcReply::Stream(
                        crate::device_hub::avcc_batches(subscription)
                            .filter_map(move |bytes| {
                                seq += 1;
                                let frame = zeron_proto::device_hub::DeviceFrame {
                                    seq,
                                    avcc: base64::engine::general_purpose::STANDARD.encode(bytes),
                                    ..Default::default()
                                };
                                futures::future::ready(serde_json::to_value(frame).ok())
                            })
                            .boxed(),
                    ));
                }
                let subscription = self
                    .device_hub()?
                    .watch_frames(p.host_id.as_deref(), &p.device_id)
                    .map_err(RpcError::Failed)?;
                Ok(RpcReply::Stream(
                    futures::stream::unfold(subscription, |mut subscription| async move {
                        loop {
                            subscription.frames.changed().await.ok()?;
                            let Some(frame) = subscription.frames.borrow_and_update().clone()
                            else {
                                continue;
                            };
                            let value =
                                serde_json::to_value(zeron_proto::device_hub::DeviceFrame {
                                    seq: frame.seq,
                                    width: None,
                                    height: None,
                                    jpeg: base64::engine::general_purpose::STANDARD
                                        .encode(&frame.jpeg),
                                    ..Default::default()
                                })
                                .ok()?;
                            return Some((value, subscription));
                        }
                    })
                    .boxed(),
                ))
            }
            methods::SEND_HUB_DEVICE_INPUT => {
                let p: zeron_proto::device_hub::SendHubDeviceInputParams = parse_params(params)?;
                self.device_hub()?
                    .send_input(p.host_id.as_deref(), &p.device_id, &p.input)
                    .map_err(RpcError::Failed)?;
                RpcReply::value(&serde_json::json!({}))
            }
            methods::TEST_DEVICE_HOST => {
                let p: zeron_proto::device_hub::SshDeviceHostConfig = parse_params(params)?;
                RpcReply::value(&self.device_hub()?.test_host(p).await)
            }
            methods::SET_HUB_DEVICE_HINGE => {
                let p: zeron_proto::device_hub::SetHubDeviceHingeParams = parse_params(params)?;
                self.device_hub()?
                    .set_hinge(p)
                    .await
                    .map_err(RpcError::Failed)?;
                RpcReply::value(&serde_json::json!({}))
            }
            methods::HUB_DEVICE_FOLD => {
                let p: zeron_proto::device_hub::HubDeviceFoldParams = parse_params(params)?;
                let fold = self
                    .device_hub()?
                    .device_fold(p)
                    .await
                    .map_err(RpcError::Failed)?;
                RpcReply::value(&fold)
            }
            methods::GET_HUB_DEVICE_DETAIL => {
                let p: zeron_proto::device_hub::HubDeviceTargetParams = parse_params(params)?;
                let detail = self
                    .device_hub()?
                    .device_detail(p)
                    .await
                    .map_err(RpcError::Failed)?;
                RpcReply::value(&detail)
            }
            methods::RUN_HUB_DEVICE_ACTION => {
                let p: zeron_proto::device_hub::RunHubDeviceActionParams = parse_params(params)?;
                let detail = self
                    .device_hub()?
                    .run_action(p)
                    .await
                    .map_err(RpcError::Failed)?;
                RpcReply::value(&detail)
            }
            methods::HUB_DEVICE_SCREENSHOT => {
                let p: zeron_proto::device_hub::HubDeviceTargetParams = parse_params(params)?;
                let shot = self
                    .device_hub()?
                    .device_screenshot(p)
                    .await
                    .map_err(RpcError::Failed)?;
                RpcReply::value(&shot)
            }
            methods::GET_HUB_DEVICE_AX_TREE => {
                let p: zeron_proto::device_hub::HubDeviceTargetParams = parse_params(params)?;
                let tree = self
                    .device_hub()?
                    .device_ax_tree(p)
                    .await
                    .map_err(RpcError::Failed)?;
                RpcReply::value(&tree)
            }
            methods::WATCH_HUB_DEVICE_APP_STATE => {
                let p: zeron_proto::device_hub::HubDeviceTargetParams = parse_params(params)?;
                let feed = self
                    .device_hub()?
                    .app_state_feed(p.host_id.as_deref(), &p.device_id)
                    .await
                    .map_err(RpcError::Failed)?;
                Ok(RpcReply::Stream(
                    feed.filter_map(|item| async move { serde_json::to_value(item).ok() })
                        .boxed(),
                ))
            }
            methods::WATCH_HUB_DEVICE_EVENTS => {
                let p: zeron_proto::device_hub::HubDeviceTargetParams = parse_params(params)?;
                let feed = self
                    .device_hub()?
                    .event_log_feed(p.host_id.as_deref(), &p.device_id)
                    .await
                    .map_err(RpcError::Failed)?;
                Ok(RpcReply::Stream(
                    feed.filter_map(|item| async move { serde_json::to_value(item).ok() })
                        .boxed(),
                ))
            }
            methods::PUBLISH_HTML_RENDER => {
                let p: zeron_proto::html_render::PublishHtmlRenderParams = parse_params(params)?;
                if self.sessions.computer_run(&p.chat_id, &p.run_secret).is_none() {
                    return Err(RpcError::Failed(
                        "html_render needs this chat's live Zeron run".into(),
                    ));
                }
                let uploads = self.uploads.clone();
                let (chat_id, html, title, height) = (p.chat_id.clone(), p.html.clone(), p.title, p.height);
                let page = tokio::task::spawn_blocking(move || {
                    crate::html_render::publish(&uploads, &chat_id, &html, &title, height)
                })
                .await
                .map_err(|e| RpcError::Failed(e.to_string()))?
                .map_err(|e| RpcError::Failed(e.to_string()))?;
                self.sessions.bind_html_render(&p.chat_id, &p.html, page.clone());
                RpcReply::value(&serde_json::json!({
                    "htmlRender": page,
                    "message": zeron_proto::html_render::RENDERED_MESSAGE,
                }))
            }
            methods::HTML_PREVIEW => {
                let p: zeron_proto::html_render::HtmlPreviewParams = parse_params(params)?;
                if self.sessions.computer_run(&p.chat_id, &p.run_secret).is_none() {
                    return Err(RpcError::Failed(
                        "html_preview needs this chat's live Zeron run".into(),
                    ));
                }
                let preview =
                    crate::html_render::preview(&self.html_preview, p.html, p.width, p.appearance)
                        .await
                        .map_err(RpcError::Failed)?;
                RpcReply::value(&preview)
            }
            methods::WATCH_HTML_PREVIEW_HOST => Ok(RpcReply::Stream(
                self.html_preview
                    .register()
                    .map_err(RpcError::Failed)?
                    .filter_map(|item| async move { serde_json::to_value(item).ok() })
                    .boxed(),
            )),
            methods::RESPOND_HTML_PREVIEW => {
                let p: zeron_proto::html_render::RespondHtmlPreviewParams = parse_params(params)?;
                let reply = match (p.preview, p.error) {
                    (Some(preview), _) => Ok(preview),
                    (None, error) => Err(error.unwrap_or_else(|| "The preview failed.".into())),
                };
                self.html_preview.respond(&p.id, reply);
                RpcReply::value(&serde_json::json!({}))
            }
            methods::DEVICE_TOOL_LIST
            | methods::DEVICE_TOOL_OPEN
            | methods::DEVICE_TOOL_SCREENSHOT
            | methods::DEVICE_TOOL_CLOSE => {
                let p: zeron_proto::device_hub::DeviceToolParams = parse_params(params)?;
                if self.sessions.computer_run(&p.chat_id, &p.run_secret).is_none() {
                    return Err(RpcError::Failed(
                        crate::device_hub::DEVICE_CALLER_REFUSED.into(),
                    ));
                }
                let hub = self.device_hub()?;
                let result = match method {
                    methods::DEVICE_TOOL_LIST => hub.tool_list(p).await,
                    methods::DEVICE_TOOL_OPEN => hub.tool_open(p).await,
                    methods::DEVICE_TOOL_SCREENSHOT => hub.tool_screenshot(p).await,
                    _ => hub.tool_close(p).await.map(|()| serde_json::json!({})),
                };
                RpcReply::value(&result.map_err(RpcError::Failed)?)
            }
            methods::GET_COMPUTER_SETTINGS => RpcReply::value(&self.computer()?.settings()),
            methods::GET_COMPUTER_STATUS => {
                let runtime = self.computer()?;
                let service = runtime
                    .service()
                    .ok_or_else(|| RpcError::Failed("computer use is still starting".into()))?;
                RpcReply::value(&zeron_proto::computer::ComputerHostStatus {
                    host_bundle_id: runtime.host_bundle_id(),
                    driver_native_revision: service.manager.driver_native_revision(),
                    status: service.manager.get_status().await,
                })
            }
            methods::SET_COMPUTER_SETTINGS => {
                let p: zeron_proto::computer::ComputerSettings = parse_params(params)?;
                self.computer()?.set_settings(p).map_err(RpcError::Failed)?;
                RpcReply::value(&serde_json::json!({}))
            }
            methods::GET_COMPUTER_STILL_FRAME => {
                let p: zeron_proto::computer::GetComputerStillFrameParams = parse_params(params)?;
                let frame = self.computer()?.still_frame(&p.chat_id);
                RpcReply::value(&serde_json::json!({ "frame": frame }))
            }
            methods::GET_COMPUTER_AUDIT_HISTORY => {
                let p: zeron_proto::computer::ComputerGetAuditHistoryInput = parse_params(params)?;
                let service = self
                    .computer()?
                    .service()
                    .ok_or_else(|| RpcError::Failed("computer use is still starting".into()))?;
                let history = service
                    .manager
                    .get_audit_history(p)
                    .await
                    .map_err(|e| RpcError::Failed(e.to_string()))?;
                RpcReply::value(&history)
            }
            methods::COMPUTER_EMERGENCY_STOP => {
                let stopped = self.computer()?.emergency_stop().await;
                RpcReply::value(&serde_json::json!({ "stoppedChatIds": stopped }))
            }
            methods::COMPUTER_REVOKE_DESKTOP => {
                let p: zeron_proto::computer::ComputerRevokeDesktopParams = parse_params(params)?;
                let computer = self.computer()?;
                let revoked = match &p.app {
                    Some(app) => computer.stop_app(&p.chat_id, app).await,
                    None => {
                        computer
                            .revoke_desktop(&p.chat_id, crate::computer::runtime::RevokeSource::Stop)
                            .await
                    }
                };
                RpcReply::value(&serde_json::json!({ "revoked": revoked }))
            }
            methods::WATCH_COMPUTER_EVENTS => {
                let events = self.computer()?.subscribe_events();
                Ok(RpcReply::Stream(
                    futures::stream::unfold(events, |mut events| async move {
                        loop {
                            match events.recv().await {
                                Ok(event) => {
                                    if let Ok(value) = serde_json::to_value(&event) {
                                        return Some((value, events));
                                    }
                                }
                                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                                Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                                    return None;
                                }
                            }
                        }
                    })
                    .boxed(),
                ))
            }
            other => Err(RpcError::UnknownMethod(other.to_string())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commit_messages::tests::git;

    #[test]
    fn a_folded_switch_note_keeps_the_harness_the_chat_left() {
        let config = |harness: &str, model: &str| -> zeron_proto::ChatConfig {
            serde_json::from_value(serde_json::json!({
                "harness": harness, "model": model, "modelOptions": {},
                "sandbox": "workspace-write",
            }))
            .unwrap()
        };
        let previous = SwitchNote {
            id: "n1".into(),
            text: "Provider changed from Mock to Codex.".into(),
            hint: Some("Mock session closed. Handed to Codex.".into()),
            packet: "packet".into(),
            route: zeron_doc::HandoffRoute {
                from_harness: Some(HarnessId::Mock),
                from_model: Some("m1".into()),
                to_model: Some("gpt".into()),
                ..Default::default()
            },
        };
        let next = SystemNote {
            text: "Provider changed from Codex to Grok.".into(),
            hint: Some(
                "Codex session closed. Handed to Grok; Codex's files were not carried.".into(),
            ),
        };
        let (folded, route) =
            fold_switch_note(&previous, HarnessId::Codex, &config("grok", "g"), &next).unwrap();
        assert_eq!(folded.text, "Provider changed from Mock to Grok.");
        assert_eq!(
            folded.hint.as_deref(),
            Some("Mock session closed. Handed to Grok; Mock's files were not carried.")
        );
        assert_eq!(route.from_harness, Some(HarnessId::Mock));
        assert_eq!(route.from_model.as_deref(), Some("m1"));
        assert_eq!(route.to_model.as_deref(), Some("g"));

        // Back where it started, harness and model: no divider at all.
        assert!(
            fold_switch_note(&previous, HarnessId::Codex, &config("mock", "m1"), &next).is_none()
        );
        // The origin harness on another model is still a handoff, worded as the model change.
        let (back, _) =
            fold_switch_note(&previous, HarnessId::Codex, &config("mock", "m2"), &next).unwrap();
        assert_eq!(back.text, "Model changed from m1 to m2.");
        assert_eq!(back.hint, None);

        // A model change on the harness the handoff reached keeps its divider.
        let (same, route) = fold_switch_note(
            &previous,
            HarnessId::Codex,
            &config("codex", "gpt-2"),
            &SystemNote {
                text: "Model changed from gpt to gpt-2.".into(),
                hint: None,
            },
        )
        .unwrap();
        assert_eq!(same.text, previous.text);
        assert_eq!(same.hint, previous.hint);
        assert_eq!(route.to_model.as_deref(), Some("gpt-2"));

        // A seeded thread's divider survives a return to its source's agent.
        let seeded = SwitchNote {
            route: zeron_doc::HandoffRoute {
                source_chat_id: Some("source".into()),
                ..previous.route.clone()
            },
            ..previous
        };
        assert!(
            fold_switch_note(&seeded, HarnessId::Codex, &config("mock", "m1"), &next).is_some()
        );
    }

    #[test]
    fn create_thread_params_refuse_an_unknown_harness_or_reasoning_level() {
        let base = serde_json::json!({ "fromChatId": "a", "prompt": "go" });
        let mut bad_reasoning = base.clone();
        bad_reasoning["reasoning"] = "very-high".into();
        assert!(matches!(
            parse_params::<CreateThreadParams>(bad_reasoning),
            Err(RpcError::BadParams(_))
        ));
        let mut bad_harness = base.clone();
        bad_harness["harness"] = "gemini".into();
        assert!(matches!(
            parse_params::<CreateThreadParams>(bad_harness),
            Err(RpcError::BadParams(_))
        ));
        let p: CreateThreadParams = parse_params(serde_json::json!({
            "fromChatId": "a", "prompt": "go", "harness": "codex", "model": "gpt-5", "reasoning": "xhigh",
            "spaceId": "s2", "cwd": "/tmp"
        }))
        .unwrap();
        assert_eq!(p.harness, Some(HarnessId::Codex));
        assert_eq!(p.model.as_deref(), Some("gpt-5"));
        assert_eq!(p.reasoning, Some(zeron_proto::ReasoningLevel::XHigh));
        assert_eq!(p.space_id.as_deref(), Some("s2"));
        assert_eq!(p.cwd.as_deref(), Some("/tmp"));
    }

    #[tokio::test]
    async fn voice_owner_is_shared_across_services_and_shutdown_retires_it() {
        let temp = tempfile::tempdir().unwrap();
        let core = crate::EngineCore::assemble_with_profile(
            crate::EngineProfile::local(temp.path()).unwrap(),
            std::sync::Arc::new(HarnessRegistry::new()),
            HarnessId::Codex,
            None,
        )
        .unwrap();
        let first = core.rpc_service();
        let second = core.rpc_service();
        let lease = first.voice.reserve("chat").unwrap();
        assert!(second.voice.owns_chat("chat"));
        assert_eq!(
            second.voice.reserve("other").unwrap_err(),
            zeron_proto::voice::VoiceRejection::Busy
        );
        let owner = second.voice.own(lease).unwrap();
        core.sessions.shutdown().await;
        assert!(!first.voice.owns_chat("chat"));
        drop(owner);
    }

    #[tokio::test]
    async fn chats_created_by_a_voice_orchestrator_are_top_level() {
        let temp = tempfile::tempdir().unwrap();
        let core = crate::EngineCore::assemble_with_profile(
            crate::EngineProfile::local(temp.path()).unwrap(),
            std::sync::Arc::new(HarnessRegistry::new()),
            HarnessId::Codex,
            None,
        )
        .unwrap();
        core.workspace
            .create_chat("ordinary", None, Some(&core.device_id), None, None)
            .unwrap();
        let rpc = core.rpc_service();
        let orchestrator = format!("{}1", zeron_proto::voice::ORCHESTRATOR_CHAT_PREFIX);
        for (chat, parent, expected) in [
            ("worker", orchestrator.as_str(), None),
            ("side", "ordinary", Some("ordinary")),
        ] {
            rpc.handle(
                methods::MUTATE,
                serde_json::json!({"op":"createChat", "chatId":chat,
                    "createdByChatId":parent, "deviceId":core.device_id,
                    "config":{"harness":"codex","sandbox":"workspace-write"}}),
            )
            .await
            .unwrap();
            let row = core.workspace.chat(chat).unwrap().unwrap();
            assert_eq!(row.created_by_chat_id.as_deref(), expected);
        }
        core.sessions.shutdown().await;
    }

    #[tokio::test]
    async fn voice_origin_uses_normal_remote_routing() {
        let temp = tempfile::tempdir().unwrap();
        let core = crate::EngineCore::assemble_with_profile(
            crate::EngineProfile::local(temp.path()).unwrap(),
            std::sync::Arc::new(HarnessRegistry::new()),
            HarnessId::Codex,
            None,
        )
        .unwrap();
        let rpc = core.rpc_service();
        let lease = rpc.voice.reserve("voice-origin").unwrap();
        let owner = rpc.voice.own(lease).unwrap();
        for method in [methods::QUEUE_COMMAND, methods::QUEUE_MESSAGE] {
            let error = match rpc.handle(method, serde_json::json!({"originChatId":"voice-origin","targetDeviceId":"remote","chatId":"other"})).await {
                Err(error) => error,
                Ok(_) => panic!("offline remote routing unexpectedly succeeded"),
            };
            assert!(
                matches!(error, RpcError::Failed(ref message) if message == "cannot reach device remote: remote routing unavailable (offline)")
            );
        }
        // Voice origin follows the same forwarding boundary as ordinary MCP
        // calls; this fixture has no relay configured.
        rpc.handle(
            methods::MUTATE,
            serde_json::json!({"op":"createChat", "chatId":"remote-grok",
                "originChatId":"voice-origin", "deviceId":"remote",
                "config":{"harness":"grok","sandbox":"workspace-write"}}),
        )
        .await
        .unwrap();
        let chat = core.workspace.chat("remote-grok").unwrap().unwrap();
        assert_eq!(chat.device_id, "remote");
        assert_eq!(chat.config.unwrap().harness, HarnessId::Grok);
        drop(owner);
        core.sessions.shutdown().await;
    }

    // Each subprocess has private HOME/PATH/overrides, avoiding process-global test races.
    #[cfg(unix)]
    async fn installer_rpc_fixture(mode: &str) {
        use std::{os::unix::fs::PermissionsExt, sync::Arc};
        if std::env::var_os("ZERON_INSTALL_FIXTURE_CHILD").is_none() {
            let root = tempfile::tempdir().unwrap();
            let bin = root.path().join("bin");
            std::fs::create_dir(&bin).unwrap();
            let script = match mode {
                "success" => {
                    "test -z \"$ZERON_INSTALL_FIXTURE_CHILD\" && test -z \"$CLAUDECODE\" && printf '#!/bin/sh\\necho 99.0.0\\n' > \"$CODEX_EXECUTABLE\" && /bin/chmod +x \"$CODEX_EXECUTABLE\""
                }
                "failure" => "echo 'fixture failure api_key=private' >&2; exit 7",
                "missing" => "exit 0",
                "cancel" => "echo ready > \"$READY_FILE\"; sleep 60",
                "npm" => "npm install -g @openai/codex",
                _ => unreachable!(),
            };
            let test = format!("rpc::tests::installer_rpc_{mode}");
            let output = tokio::process::Command::new(std::env::current_exe().unwrap())
                .args(["--exact", &test, "--nocapture", "--include-ignored"])
                .env("ZERON_INSTALL_FIXTURE_CHILD", root.path())
                .env("ZERON_INSTALLER_COMMAND_CODEX", script)
                .env("ZERON_NO_LOGIN_SHELL", "1")
                .env("HOME", root.path())
                .env("XDG_CONFIG_HOME", root.path().join("config"))
                .env("CODEX_EXECUTABLE", bin.join("codex"))
                .env("CLAUDECODE", "nested-test")
                .env("READY_FILE", root.path().join("ready"))
                .env("npm_config_prefix", root.path())
                .env("npm_config_cache", root.path().join("npm-cache"))
                .env(
                    "PATH",
                    std::env::join_paths(
                        std::iter::once(bin)
                            .chain(std::env::split_paths(&std::env::var_os("PATH").unwrap())),
                    )
                    .unwrap(),
                )
                .output()
                .await
                .unwrap();
            assert!(
                output.status.success(),
                "{}\n{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            println!("{}", String::from_utf8_lossy(&output.stdout));
            return;
        }
        let root =
            std::path::PathBuf::from(std::env::var_os("ZERON_INSTALL_FIXTURE_CHILD").unwrap());
        let registry = Arc::new(HarnessRegistry::new());
        registry.register(Arc::new(zeron_harness::CodexHarness::new()));
        let core = crate::EngineCore::assemble(
            &root.join("engine"),
            registry.clone(),
            HarnessId::Codex,
            None,
        )
        .unwrap();
        let rpc = core.rpc_service();
        let params = serde_json::json!({"harness": "codex"});
        assert!(!registry.descriptors()[0].installed);
        assert_eq!(registry.descriptors()[0].enabled, Some(false));
        let result = if mode == "cancel" {
            let rpc = rpc.clone();
            let task = tokio::spawn(async move {
                rpc.handle(
                    methods::INSTALL_HARNESS,
                    serde_json::json!({"harness": "codex"}),
                )
                .await
            });
            tokio::time::timeout(std::time::Duration::from_secs(5), async {
                while !root.join("ready").exists() {
                    tokio::time::sleep(std::time::Duration::from_millis(10)).await;
                }
            })
            .await
            .unwrap();
            // A second service for the same device must share the in-flight guard.
            let other = core.rpc_service();
            assert!(
                matches!(other.handle(methods::INSTALL_HARNESS, params.clone()).await, Err(RpcError::Failed(e)) if e == "already installing")
            );
            other.handle(methods::CANCEL_INSTALL, params).await.unwrap();
            tokio::time::timeout(std::time::Duration::from_secs(5), task)
                .await
                .unwrap()
                .unwrap()
        } else {
            rpc.handle(methods::INSTALL_HARNESS, params).await
        };
        match mode {
            "success" | "npm" => {
                let RpcReply::Value(value) = result.unwrap() else {
                    panic!("expected descriptors");
                };
                let list: Vec<crate::registry::HarnessDescriptor> =
                    serde_json::from_value(value).unwrap();
                assert!(list[0].installed);
                assert_eq!(list[0].enabled, Some(true));
                assert!(list[0].can_install);
                let path = root.join("bin/codex");
                assert!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o111 != 0);
                println!(
                    "InstallHarness ({mode}): installed=false/enabled=false -> installed=true/enabled=true; {}",
                    path.display()
                );
            }
            "failure" => assert!(
                matches!(result, Err(RpcError::Failed(e)) if e.contains("fixture failure") && e.contains("[REDACTED]") && !e.contains("private"))
            ),
            "missing" => assert!(
                matches!(result, Err(RpcError::Failed(e)) if e.contains("installer finished but `codex` was not found on PATH"))
            ),
            "cancel" => {
                assert!(matches!(result, Err(RpcError::Failed(e)) if e.contains("cancelled")));
                assert!(registry.installs.begin(HarnessId::Codex).is_ok());
            }
            _ => unreachable!(),
        }
        assert!(forwardable(methods::CANCEL_INSTALL));
        core.shutdown().await;
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn installer_rpc_success() {
        installer_rpc_fixture("success").await;
    }
    #[cfg(unix)]
    #[tokio::test]
    async fn installer_rpc_failure() {
        installer_rpc_fixture("failure").await;
    }
    #[cfg(unix)]
    #[tokio::test]
    async fn installer_rpc_missing() {
        installer_rpc_fixture("missing").await;
    }
    #[cfg(unix)]
    #[tokio::test]
    async fn installer_rpc_cancel() {
        installer_rpc_fixture("cancel").await;
    }
    #[cfg(unix)]
    #[tokio::test]
    #[ignore = "downloads the official npm package into an isolated temporary prefix"]
    async fn installer_rpc_npm() {
        installer_rpc_fixture("npm").await;
    }

    #[tokio::test]
    async fn explicit_install_rpc_verifies_archive_and_refreshes_descriptors() {
        use sha2::{Digest, Sha512};
        use std::io::Write;
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        use zeron_harness::archive_install::{ArchivePin, ensure_installed, installed_entry};
        if std::env::var_os("ZERON_INSTALL_RPC_TEST").is_none() {
            let root = tempfile::tempdir().unwrap();
            let output = tokio::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "rpc::tests::explicit_install_rpc_verifies_archive_and_refreshes_descriptors",
                    "--nocapture",
                ])
                .env("ZERON_INSTALL_RPC_TEST", "1")
                .env("ZERON_ADAPTERS_DIR", root.path())
                .output()
                .await
                .unwrap();
            assert!(
                output.status.success(),
                "{} {}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            return;
        }
        if !zeron_harness::acp::can_install(HarnessId::Antigravity) {
            return;
        }
        let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        zip.start_file("server", zip::write::SimpleFileOptions::default())
            .unwrap();
        zip.write_all(b"#!/bin/sh\nexit 0\n").unwrap();
        let bytes = zip.finish().unwrap().into_inner();
        let digest = Box::leak(format!("{:x}", Sha512::digest(&bytes)).into_boxed_str());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = Box::leak(
            format!("http://{}/archive.zip", listener.local_addr().unwrap()).into_boxed_str(),
        );
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut headers = Vec::new();
            let mut buf = [0; 4096];
            while !headers.windows(4).any(|bytes| bytes == b"\r\n\r\n") {
                let count = socket.read(&mut buf).await.unwrap();
                assert!(count > 0, "archive request closed before its headers");
                headers.extend_from_slice(&buf[..count]);
            }
            socket
                .write_all(
                    format!(
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        bytes.len()
                    )
                    .as_bytes(),
                )
                .await
                .unwrap();
            socket.write_all(&bytes).await.unwrap();
        });
        let pin = ArchivePin {
            name: "explicit-install-test",
            version: "1",
            url,
            entry: "server",
            sha512: digest,
        };
        let registry = HarnessRegistry::new();
        let descriptor = serde_json::from_value(serde_json::json!({
            "id": "antigravity", "name": "Antigravity", "supportsSteering": true,
            "steeringMode": "turn-boundary", "reasoningLevels": [], "installed": false
        }))
        .unwrap();
        registry.register_lazy(
            descriptor,
            Box::new(move || installed_entry(&pin).is_some()),
            Box::new(|| panic!("installation must not spawn the harness")),
        );
        assert!(!registry.descriptors()[0].installed);
        let result = install_harness_with(&registry, HarnessId::Antigravity, || async {
            ensure_installed(pin, "Test adapter").await.map(|_| ())
        })
        .await
        .unwrap();
        assert!(result[0].installed);
        assert_eq!(result[0].enabled, Some(true));
        assert!(result[0].can_install);
        assert!(installed_entry(&pin).unwrap().is_file());
        server.await.unwrap();
        // The verified marker makes another explicit install idempotent, even
        // after the archive server has stopped.
        install_harness_with(&registry, HarnessId::Antigravity, || async {
            ensure_installed(pin, "Test adapter").await.map(|_| ())
        })
        .await
        .unwrap();
        assert!(
            install_harness_with(&registry, HarnessId::Mock, || async {
                panic!("unsupported harness must not invoke an installer")
            })
            .await
            .is_err()
        );
        assert!(forwardable(methods::INSTALL_HARNESS));
        assert_eq!(
            forward_deadline(methods::INSTALL_HARNESS),
            std::time::Duration::from_secs(15 * 60)
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn local_opening_tail_arrives_before_full_mirror_and_keeps_all_history() {
        use crate::doc_host::{DocHost, DocHostConfig};
        use std::sync::Arc;
        let dir = tempfile::tempdir().unwrap();
        let store = Arc::new(zeron_sync::DocsStore::open(dir.path()).unwrap());
        let host = DocHost::new(
            store,
            DocHostConfig {
                device_id: "viewer".into(),
                default_harness: zeron_proto::HarnessId::Mock,
                edge: None,
            },
        );
        let handle = host.open("whale").unwrap();
        handle
            .doc()
            .push_message(&zeron_doc::SessionMessageEntry {
                id: "turn".into(),
                role: zeron_doc::MessageRole::Assistant,
                parts: (0..500)
                    .map(|i| zeron_doc::MessagePart::Text {
                        id: format!("part-{i}"),
                        text: "local text".into(),
                    })
                    .collect(),
                created_at: 0,
                device_id: "host".into(),
                status: None,
                continuation_of: None,
                from_chat_id: None,
                fork_import: false,
                answers_request_id: None,
                duration_ms: None,
            })
            .unwrap();
        // Hold publication blocked: the opening must not await the full mirror.
        let held = handle.clone();
        let (locked_tx, locked_rx) = std::sync::mpsc::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        let blocker = std::thread::spawn(move || {
            held.import_transcript(|| {
                locked_tx.send(()).unwrap();
                release_rx.recv().unwrap();
            })
        });
        locked_rx.recv().unwrap();
        let opening = tokio::time::timeout(
            Duration::from_secs(2),
            opening_doc_messages_stream(host.clone(), "whale".into()),
        )
        .await;
        release_tx.send(()).unwrap();
        blocker.join().unwrap();
        let mut stream = opening
            .expect("first paint must not wait for publication")
            .unwrap();
        let preview = stream.next().await.unwrap();
        assert_eq!(preview["historyPending"], true);
        assert_eq!(preview["reset"][0]["parts"].as_array().unwrap().len(), 128);
        assert_eq!(preview["reset"][0]["parts"][0]["id"], "part-372");
        // Changes between preview and subscribe must appear in the full reset.
        handle
            .write_user_message("arrived", "new local message", 1)
            .unwrap();
        let full = stream.next().await.unwrap();
        assert!(full.get("historyPending").is_none());
        assert_eq!(full["reset"][0]["parts"].as_array().unwrap().len(), 500);
        assert_eq!(full["reset"][1]["id"], "arrived");
        handle
            .write_user_message("live", "after attach", 2)
            .unwrap();
        let live = tokio::time::timeout(Duration::from_secs(2), stream.next())
            .await
            .unwrap()
            .unwrap();
        let mut entries = Vec::new();
        for value in [full, live] {
            let update: zeron_doc::TranscriptUpdate = serde_json::from_value(value).unwrap();
            zeron_doc::apply_transcript_frame(&mut entries, update.frame).unwrap();
        }
        assert_eq!(entries.len(), 3);
        assert_eq!(entries.last().unwrap().id, "live");
        host.shutdown_workers().await;
    }

    #[tokio::test]
    async fn antigravity_disable_does_not_launch_the_server() {
        let registry = HarnessRegistry::new();
        let executable = std::env::current_exe().unwrap();
        registry.register(std::sync::Arc::new(
            zeron_harness::AcpHarness::grok().with_executable(executable.clone()),
        ));
        registry.register(std::sync::Arc::new(
            zeron_harness::AcpHarness::antigravity().with_executable(executable),
        ));
        registry.set_enabled(HarnessId::Antigravity, true).unwrap();

        update_harness_enabled(&registry, HarnessId::Antigravity, false)
            .await
            .unwrap();
        assert!(!registry.enabled_set().contains(&HarnessId::Antigravity));
    }

    /// The UI's Switch/Forget calls send `{id, accountId, harness}` (+ optional
    /// `targetDeviceId`); the extra fields must be tolerated, `accountId` wins.
    #[test]
    fn list_models_force_is_optional_and_backward_compatible() {
        let old: ListModelsParams =
            serde_json::from_value(serde_json::json!({"harness":"codex"})).unwrap();
        assert!(!old.force);
        let forced: ListModelsParams =
            serde_json::from_value(serde_json::json!({"harness":"codex","force":true})).unwrap();
        assert!(forced.force);
        assert_eq!(old.instance_id, None);
        let instance: ListModelsParams = serde_json::from_value(
            serde_json::json!({"harness":"codex","instanceId":"codex_work"}),
        )
        .unwrap();
        assert_eq!(instance.instance_id.as_deref(), Some("codex_work"));
    }

    #[test]
    fn agent_account_params_accept_ui_shape() {
        let p: AgentAccountParams = parse_params(serde_json::json!({
            "id": "acct-1",
            "accountId": "acct-1",
            "harness": "claude-code",
            "targetDeviceId": "dev-2",
        }))
        .expect("ui param shape");
        assert_eq!(p.account_id, "acct-1");
        assert_eq!(p.harness, HarnessId::ClaudeCode);
    }

    #[test]
    fn sidebar_preferences_mutation_accepts_desktop_wire_shape() {
        let p: MutateParams = parse_params(serde_json::json!({
            "op": "changeSidebarPin",
            "change": {"action":"move","sessionId":"chat-b","before":"chat-a","after":null},
        }))
        .expect("sidebar preferences params");
        assert!(matches!(
            p,
            MutateParams::ChangeSidebarPin { change: zeron_proto::SidebarPinChange::Move { session_id, before, .. } }
                if session_id == "chat-b" && before.as_deref() == Some("chat-a")
        ));
    }

    #[test]
    fn local_device_is_not_forwardable() {
        assert!(!forwardable(methods::LOCAL_DEVICE));
        assert!(!forwardable(methods::FOCUS_CHAT));
        assert!(!forwardable(methods::ENGINE_INFO));
        assert!(!forwardable(methods::ENGINE_READY));
        assert!(forwardable(methods::QUEUE_COMMAND));
        assert!(forwardable(methods::SEARCH_FILES));
        assert!(forwardable(methods::SEARCH_GIT_HISTORY));
        assert!(forwardable(methods::FETCH_ALL));
        assert!(forwardable(methods::RESOLVE_GIT_AVATARS));
        assert!(forwardable(methods::WATCH_CHECKOUT_CHANGE_REQUEST));
        assert!(is_stream_method(methods::WATCH_CHECKOUT_CHANGE_REQUEST));
        assert!(forwardable(methods::DISCARD_WORKING_TREE));
        for method in [
            methods::GET_WORKING_TREE,
            methods::GET_WORKING_TREE_FILE_DIFF,
            methods::RUN_WORKING_TREE_ACTION,
            methods::SYNC_BRANCH,
            methods::GENERATE_COMMIT_MESSAGE,
        ] {
            assert!(forwardable(method), "{method}");
            assert!(!is_stream_method(method), "{method}");
        }
        assert!(forwardable(methods::LIST_WORKSPACE_DIRECTORY));
        assert!(forwardable(methods::SEARCH_WORKSPACE_FILES));
        assert!(forwardable(methods::READ_WORKSPACE_FILE));
        assert!(forwardable(methods::READ_WORKSPACE_IMAGE));
        assert!(forwardable(methods::DELETE_WORKSPACE_ENTRY));
        assert!(forwardable(methods::MOVE_WORKSPACE_ENTRY));
        assert!(forwardable(methods::WRITE_WORKSPACE_FILE));
        assert!(forwardable(methods::WATCH_WORKSPACE_FILES));
        assert!(forwardable(methods::WATCH_WORKSPACE_GIT_STATUS));
        assert!(!is_stream_method(methods::LIST_WORKSPACE_DIRECTORY));
        assert!(!is_stream_method(methods::SEARCH_WORKSPACE_FILES));
        assert!(forwardable(methods::WARM_WORKSPACE_SEARCH));
        assert!(!is_stream_method(methods::WARM_WORKSPACE_SEARCH));
        assert!(forwardable(methods::SEARCH_WORKSPACE_CONTENT));
        assert!(!is_stream_method(methods::SEARCH_WORKSPACE_CONTENT));
        assert!(forwardable(methods::SEARCH_HOME_FOLDERS));
        assert!(forwardable(methods::RELEASE_HOME_FOLDER_SEARCH));
        assert!(!is_stream_method(methods::READ_WORKSPACE_FILE));
        assert!(!is_stream_method(methods::WRITE_WORKSPACE_FILE));
        assert!(is_stream_method(methods::WATCH_WORKSPACE_FILES));
        assert!(is_stream_method(methods::WATCH_WORKSPACE_GIT_STATUS));
        assert!(forwardable(methods::WATCH_HARNESS_UPDATES));
        assert!(is_stream_method(methods::WATCH_HARNESS_UPDATES));
        assert!(forwardable(methods::WATCH_PROVIDER_INSTANCES));
        assert!(is_stream_method(methods::WATCH_PROVIDER_INSTANCES));
        assert!(forwardable(methods::CHECK_HARNESS_UPDATES));
        assert!(forwardable(methods::APPLY_HARNESS_UPDATE));
        assert!(forwardable(methods::LIST_CHANGE_REQUEST_PAGE));
        assert!(!is_stream_method(methods::LIST_CHANGE_REQUEST_PAGE));
        assert!(forwardable(methods::GET_CHANGE_REQUEST_REPOSITORY));
        assert!(forwardable(methods::GET_CHANGE_REQUEST));
        assert!(forwardable(methods::GET_CHANGE_REQUEST_DIFF));
        assert!(forwardable(methods::POST_CHANGE_REQUEST_COMMENT));
        assert!(!is_stream_method(methods::POST_CHANGE_REQUEST_COMMENT));
        for method in [
            methods::GET_CHANGE_REQUEST_CONTROLS,
            methods::MERGE_CHANGE_REQUEST,
            methods::LIST_CHANGE_REQUEST_LABEL_CANDIDATES,
            methods::SET_CHANGE_REQUEST_LABEL,
            methods::LIST_CHANGE_REQUEST_REVIEWER_CANDIDATES,
            methods::SET_CHANGE_REQUEST_REVIEWER,
            methods::SUBMIT_CHANGE_REQUEST_REVIEW,
            methods::REPLY_TO_CHANGE_REQUEST_THREAD,
            methods::SET_CHANGE_REQUEST_THREAD_RESOLVED,
        ] {
            assert!(forwardable(method) && !is_stream_method(method));
        }
        assert!(!is_stream_method(methods::GET_CHANGE_REQUEST));
        assert!(!is_stream_method(methods::GET_CHANGE_REQUEST_DIFF));
    }

    #[tokio::test]
    async fn get_working_tree_reads_space_checkouts_only_and_survives_the_wire() {
        let dir = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(dir.path()).unwrap();
        let repo = root.join("repo");
        let linked = root.join("linked");
        let outside = root.join("outside");
        let plain = root.join("plain");
        let data = root.join("data");
        for path in [&repo, &outside, &plain, &data] {
            std::fs::create_dir(path).unwrap();
        }
        git(&repo, &["init", "-b", "main"]);
        std::fs::write(repo.join("a.txt"), "a\n").unwrap();
        git(&repo, &["add", "."]);
        git(&repo, &["commit", "-m", "first"]);
        git(
            &repo,
            &["worktree", "add", "-b", "side", linked.to_str().unwrap()],
        );
        std::fs::write(repo.join("new file.txt"), "x\n").unwrap();
        git(&outside, &["init", "-b", "main"]);

        let core = crate::EngineCore::assemble(
            &data,
            std::sync::Arc::new(HarnessRegistry::new()),
            HarnessId::Mock,
            None,
        )
        .unwrap();
        let rpc = core.rpc_service();
        for (space_id, path, git_detected) in
            [("space-repo", &repo, true), ("space-plain", &plain, false)]
        {
            rpc.handle(
                methods::MUTATE,
                serde_json::json!({
                    "op": "createSpace", "spaceId": space_id, "deviceId": core.device_id,
                    "path": path, "gitDetected": git_detected,
                }),
            )
            .await
            .unwrap();
        }

        let reply = rpc
            .handle(
                methods::GET_WORKING_TREE,
                serde_json::json!({ "spaceId": "space-repo" }),
            )
            .await
            .unwrap();
        let RpcReply::Value(value) = reply else {
            panic!("unary reply expected");
        };
        let frame = zeron_rpc::ServerFrame {
            id: 1,
            ok: Some(value),
            ..Default::default()
        };
        let received: zeron_rpc::ServerFrame =
            serde_json::from_str(&serde_json::to_string(&frame).unwrap()).unwrap();
        let tree: zeron_proto::WorkingTree =
            serde_json::from_value(received.ok.expect("a reply, never null")).unwrap();
        assert_eq!(tree.branch.as_deref(), Some("main"));
        assert_eq!(tree.root, repo.to_string_lossy());
        assert!(!tree.agent_active);
        assert_eq!(
            tree.files
                .iter()
                .map(|f| f.path.as_str())
                .collect::<Vec<_>>(),
            ["new file.txt"]
        );

        let RpcReply::Value(linked_tree) = rpc
            .handle(
                methods::GET_WORKING_TREE,
                serde_json::json!({ "spaceId": "space-repo", "checkoutPath": linked }),
            )
            .await
            .unwrap()
        else {
            panic!("unary reply expected");
        };
        assert_eq!(linked_tree["branch"], "side");
        assert_ne!(
            linked_tree["checkoutId"],
            serde_json::json!(tree.checkout_id)
        );

        for (params, expected) in [
            (
                serde_json::json!({ "spaceId": "space-repo", "checkoutPath": outside }),
                "not a workspace checkout",
            ),
            (
                serde_json::json!({ "spaceId": "space-repo", "checkoutPath": root }),
                "not a workspace checkout",
            ),
            (serde_json::json!({ "chatId": "chat-1" }), "spaceId"),
            (
                serde_json::json!({ "spaceId": "space-plain" }),
                "not a Git checkout",
            ),
            (serde_json::json!({ "spaceId": "missing" }), "not found"),
        ] {
            let error = match rpc.handle(methods::GET_WORKING_TREE, params.clone()).await {
                Err(error) => error.to_string(),
                Ok(_) => panic!("{params} must be refused"),
            };
            assert!(error.contains(expected), "{params}: {error}");
        }
    }

    fn through_the_wire(reply: RpcReply) -> serde_json::Value {
        let RpcReply::Value(value) = reply else {
            panic!("unary reply expected");
        };
        let frame = zeron_rpc::ServerFrame {
            id: 1,
            ok: Some(value),
            ..Default::default()
        };
        let received: zeron_rpc::ServerFrame =
            serde_json::from_str(&serde_json::to_string(&frame).unwrap()).unwrap();
        received.ok.expect("a reply, never null")
    }

    #[tokio::test]
    async fn engine_info_advertises_source_control_through_the_wire() {
        let dir = tempfile::tempdir().unwrap();
        let core = crate::EngineCore::assemble(
            dir.path(),
            std::sync::Arc::new(HarnessRegistry::new()),
            HarnessId::Mock,
            None,
        )
        .unwrap();
        let info: zeron_proto::EngineInfo = serde_json::from_value(through_the_wire(
            core.rpc_service()
                .handle(methods::ENGINE_INFO, serde_json::json!({}))
                .await
                .unwrap(),
        ))
        .unwrap();
        assert!(info.supports(zeron_proto::capabilities::SOURCE_CONTROL_V1));
    }

    #[tokio::test]
    async fn generate_commit_message_reads_the_staged_change_and_replies_through_the_wire() {
        use crate::commit_messages::tests::Scripted;
        let dir = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(dir.path()).unwrap();
        let repo = root.join("repo");
        let data = root.join("data");
        for path in [&repo, &data] {
            std::fs::create_dir(path).unwrap();
        }
        git(&repo, &["init", "-b", "main"]);
        std::fs::write(repo.join("a.txt"), "a\n").unwrap();
        git(&repo, &["add", "."]);
        git(&repo, &["commit", "-q", "-m", "first"]);
        std::fs::write(repo.join("a.txt"), "a\nb\n").unwrap();

        let registry = std::sync::Arc::new(HarnessRegistry::new());
        let scripted = std::sync::Arc::new(Scripted::new(
            HarnessId::ClaudeCode,
            &["claude-sonnet", "claude-haiku-4-5"],
            "A: add b\n\nWhy it matters.",
        ));
        registry.register(scripted.clone());
        let core = crate::EngineCore::assemble(&data, registry, HarnessId::Mock, None).unwrap();
        let rpc = core.rpc_service();
        rpc.handle(
            methods::MUTATE,
            serde_json::json!({
                "op": "createSpace", "spaceId": "space-repo", "deviceId": core.device_id,
                "path": repo, "gitDetected": true,
            }),
        )
        .await
        .unwrap();
        let params = serde_json::json!({
            "spaceId": "space-repo",
            "instruction": "Write a short commit message for this staged diff.",
        });

        let error = rpc
            .handle(methods::GENERATE_COMMIT_MESSAGE, params.clone())
            .await
            .err()
            .expect("nothing staged is refused");
        assert!(error.to_string().contains("Nothing is staged"), "{error}");
        let error = rpc
            .handle(
                methods::GENERATE_COMMIT_MESSAGE,
                serde_json::json!({ "chatId": "chat-1" }),
            )
            .await
            .err()
            .expect("a chat target is refused");
        assert!(error.to_string().contains("spaceId"), "{error}");
        assert!(scripted.calls.lock().unwrap().is_empty());

        git(&repo, &["add", "a.txt"]);
        let reply = through_the_wire(
            rpc.handle(methods::GENERATE_COMMIT_MESSAGE, params)
                .await
                .unwrap(),
        );
        assert_eq!(
            reply,
            serde_json::json!({
                "message": "A: add b\n\nWhy it matters.",
                "harness": serde_json::to_value(HarnessId::ClaudeCode).unwrap(),
                "model": "claude-haiku-4-5",
            })
        );
        assert_eq!(git(&repo, &["log", "--format=%s"]), "first\n");
        assert_eq!(git(&repo, &["diff", "--cached", "--name-only"]), "a.txt\n");
        core.shutdown().await;
    }

    #[tokio::test]
    async fn working_tree_actions_check_the_revision_the_target_and_the_busy_policy() {
        let dir = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(dir.path()).unwrap();
        let repo = root.join("repo");
        let outside = root.join("outside");
        let data = root.join("data");
        for path in [&repo, &outside, &data] {
            std::fs::create_dir(path).unwrap();
        }
        git(&repo, &["init", "-b", "main"]);
        std::fs::write(repo.join("a.txt"), "a\n").unwrap();
        git(&repo, &["add", "."]);
        git(&repo, &["commit", "-m", "first"]);
        std::fs::write(repo.join("a.txt"), "a\nb\n").unwrap();
        std::fs::write(repo.join("new file.txt"), "n\n").unwrap();
        git(&outside, &["init", "-b", "main"]);

        let core = crate::EngineCore::assemble(
            &data,
            std::sync::Arc::new(HarnessRegistry::new()),
            HarnessId::Mock,
            None,
        )
        .unwrap();
        let rpc = core.rpc_service();
        rpc.handle(
            methods::MUTATE,
            serde_json::json!({
                "op": "createSpace", "spaceId": "space-repo", "deviceId": core.device_id,
                "path": repo, "gitDetected": true,
            }),
        )
        .await
        .unwrap();
        async fn revision(rpc: &EngineRpc) -> String {
            let tree: zeron_proto::WorkingTree = serde_json::from_value(through_the_wire(
                rpc.handle(
                    methods::GET_WORKING_TREE,
                    serde_json::json!({ "spaceId": "space-repo" }),
                )
                .await
                .unwrap(),
            ))
            .unwrap();
            tree.revision
        }
        let action = |revision: &str, policy: &str, action: serde_json::Value| {
            serde_json::json!({
                "spaceId": "space-repo", "expectedRevision": revision,
                "busyPolicy": policy, "action": action,
            })
        };
        let stage_a = serde_json::json!({"kind": "stage", "paths": ["a.txt"]});

        let error = rpc
            .handle(
                methods::RUN_WORKING_TREE_ACTION,
                action("stale", "safeOnly", stage_a.clone()),
            )
            .await
            .err()
            .expect("a stale revision is refused");
        assert!(error.to_string().contains("changed since"), "{error}");
        assert_eq!(
            git(&repo, &["status", "--porcelain"]).replace('"', ""),
            " M a.txt\n?? new file.txt\n"
        );

        for (params, expected) in [
            (
                serde_json::json!({
                    "spaceId": "space-repo", "checkoutPath": outside,
                    "expectedRevision": "r", "action": stage_a,
                }),
                "not a workspace checkout",
            ),
            (
                serde_json::json!({
                    "chatId": "chat-1", "expectedRevision": "r", "action": stage_a,
                }),
                "spaceId",
            ),
        ] {
            let error = rpc
                .handle(methods::RUN_WORKING_TREE_ACTION, params.clone())
                .await
                .err()
                .unwrap_or_else(|| panic!("{params} must be refused"));
            assert!(error.to_string().contains(expected), "{params}: {error}");
        }

        let current = revision(&rpc).await;
        let reply = through_the_wire(
            rpc.handle(
                methods::RUN_WORKING_TREE_ACTION,
                action(&current, "safeOnly", stage_a.clone()),
            )
            .await
            .unwrap(),
        );
        assert_eq!(reply, serde_json::json!({}));
        assert_eq!(
            git(&repo, &["status", "--porcelain"]).replace('"', ""),
            "M  a.txt\n?? new file.txt\n"
        );

        let diff: zeron_proto::WorkingTreeFileDiff = serde_json::from_value(through_the_wire(
            rpc.handle(
                methods::GET_WORKING_TREE_FILE_DIFF,
                serde_json::json!({ "spaceId": "space-repo", "path": "a.txt", "staged": true }),
            )
            .await
            .unwrap(),
        ))
        .unwrap();
        assert!(diff.patch.contains("+b"), "{}", diff.patch);
        let error = rpc
            .handle(
                methods::GET_WORKING_TREE_FILE_DIFF,
                serde_json::json!({ "spaceId": "space-repo", "path": "../outside/x", "staged": false }),
            )
            .await
            .err()
            .expect("escape refused");
        assert!(
            error.to_string().contains("not a working tree path"),
            "{error}"
        );

        rpc.handle(
            methods::MUTATE,
            serde_json::json!({
                "op": "createChat", "chatId": "chat-busy", "deviceId": core.device_id,
                "spaceId": "space-repo", "cwd": repo,
            }),
        )
        .await
        .unwrap();
        core.sessions
            .set_status_for_test("chat-busy", zeron_proto::SessionStatus::Working);
        let tree: zeron_proto::WorkingTree = serde_json::from_value(through_the_wire(
            rpc.handle(
                methods::GET_WORKING_TREE,
                serde_json::json!({ "spaceId": "space-repo" }),
            )
            .await
            .unwrap(),
        ))
        .unwrap();
        assert!(tree.agent_active);

        let unstage_all = serde_json::json!({"kind": "unstageAll"});
        let discard = serde_json::json!({"kind": "discard", "paths": ["a.txt"]});
        let error = rpc
            .handle(
                methods::RUN_WORKING_TREE_ACTION,
                action(&tree.revision, "blockAll", unstage_all.clone()),
            )
            .await
            .err()
            .expect("BlockAll refuses every write while an agent runs");
        assert!(error.to_string().contains("read-only"), "{error}");
        let error = rpc
            .handle(
                methods::RUN_WORKING_TREE_ACTION,
                action(&tree.revision, "safeOnly", discard.clone()),
            )
            .await
            .err()
            .expect("SafeOnly refuses discard while an agent runs");
        assert!(error.to_string().contains("Only stage"), "{error}");
        let error = rpc
            .handle(
                methods::RUN_WORKING_TREE_ACTION,
                action(&tree.revision, "allowAll", discard),
            )
            .await
            .err()
            .expect("AllowAll passes the policy; a.txt has nothing unstaged to discard");
        assert!(error.to_string().contains("no unstaged changes"), "{error}");
        assert_eq!(
            git(&repo, &["status", "--porcelain"]).replace('"', ""),
            "M  a.txt\n?? new file.txt\n"
        );

        rpc.handle(
            methods::RUN_WORKING_TREE_ACTION,
            action(&tree.revision, "safeOnly", unstage_all),
        )
        .await
        .expect("SafeOnly allows unstage while an agent runs");
        assert_eq!(
            git(&repo, &["status", "--porcelain"]).replace('"', ""),
            " M a.txt\n?? new file.txt\n"
        );
        let current = revision(&rpc).await;
        rpc.handle(
            methods::RUN_WORKING_TREE_ACTION,
            action(
                &current,
                "allowAll",
                serde_json::json!({"kind": "stage", "paths": ["new file.txt"]}),
            ),
        )
        .await
        .expect("AllowAll allows stage while an agent runs");
        assert_eq!(
            git(&repo, &["status", "--porcelain"]).replace('"', ""),
            " M a.txt\nA  new file.txt\n"
        );

        for (key, value) in [
            ("user.name", "Lab"),
            ("user.email", "lab@example.com"),
            ("commit.gpgsign", "false"),
        ] {
            git(&repo, &["config", key, value]);
        }
        let first = git(&repo, &["rev-parse", "HEAD"]);
        let current = revision(&rpc).await;
        for (commit, reason) in [
            (
                serde_json::json!({"kind": "commit", "message": "All", "all": true}),
                "SafeOnly refuses commit all while an agent runs",
            ),
            (
                serde_json::json!({"kind": "commit", "message": "Amend", "amend": true}),
                "SafeOnly refuses amend while an agent runs",
            ),
        ] {
            let error = rpc
                .handle(
                    methods::RUN_WORKING_TREE_ACTION,
                    action(&current, "safeOnly", commit),
                )
                .await
                .err()
                .expect(reason);
            assert!(error.to_string().contains("Only stage"), "{error}");
        }
        let error = rpc
            .handle(
                methods::RUN_WORKING_TREE_ACTION,
                action(
                    &current,
                    "blockAll",
                    serde_json::json!({"kind": "commit", "message": "Staged"}),
                ),
            )
            .await
            .err()
            .expect("BlockAll refuses a commit while an agent runs");
        assert!(error.to_string().contains("read-only"), "{error}");
        assert_eq!(git(&repo, &["rev-parse", "HEAD"]), first);

        let reply = through_the_wire(
            rpc.handle(
                methods::RUN_WORKING_TREE_ACTION,
                action(
                    &current,
                    "safeOnly",
                    serde_json::json!({"kind": "commit", "message": "Add the new file\n\nWhy it exists."}),
                ),
            )
            .await
            .expect("SafeOnly commits staged files while an agent runs"),
        );
        let head = git(&repo, &["rev-parse", "HEAD"]);
        assert_eq!(
            reply,
            serde_json::json!({"commit": {
                "sha": head.trim(), "branch": "main", "subject": "Add the new file",
            }})
        );
        assert_eq!(
            git(&repo, &["status", "--porcelain"]).replace('"', ""),
            " M a.txt\n"
        );

        let current = revision(&rpc).await;
        let reply = through_the_wire(
            rpc.handle(
                methods::RUN_WORKING_TREE_ACTION,
                action(
                    &current,
                    "allowAll",
                    serde_json::json!({"kind": "commit", "message": "Everything", "all": true}),
                ),
            )
            .await
            .expect("AllowAll commits everything while an agent runs"),
        );
        assert_eq!(reply["commit"]["subject"], "Everything");
        assert_eq!(git(&repo, &["status", "--porcelain"]), "");
    }

    #[tokio::test]
    async fn discard_and_stash_run_by_revision_and_oid_behind_the_busy_policy() {
        let dir = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(dir.path()).unwrap();
        let repo = root.join("repo");
        let data = root.join("data");
        for path in [&repo, &data] {
            std::fs::create_dir(path).unwrap();
        }
        git(&repo, &["init", "-b", "main"]);
        std::fs::write(repo.join("a.txt"), "a\n").unwrap();
        git(&repo, &["add", "."]);
        git(&repo, &["commit", "-m", "first"]);
        std::fs::write(repo.join("a.txt"), "edited\n").unwrap();
        std::fs::write(repo.join("new file.txt"), "n\n").unwrap();

        let core = crate::EngineCore::assemble(
            &data,
            std::sync::Arc::new(HarnessRegistry::new()),
            HarnessId::Mock,
            None,
        )
        .unwrap();
        let rpc = core.rpc_service();
        rpc.handle(
            methods::MUTATE,
            serde_json::json!({
                "op": "createSpace", "spaceId": "space-repo", "deviceId": core.device_id,
                "path": repo, "gitDetected": true,
            }),
        )
        .await
        .unwrap();
        async fn read(rpc: &EngineRpc) -> zeron_proto::WorkingTree {
            serde_json::from_value(through_the_wire(
                rpc.handle(
                    methods::GET_WORKING_TREE,
                    serde_json::json!({ "spaceId": "space-repo" }),
                )
                .await
                .unwrap(),
            ))
            .unwrap()
        }
        let action = |revision: &str, policy: &str, action: serde_json::Value| {
            serde_json::json!({
                "spaceId": "space-repo", "expectedRevision": revision,
                "busyPolicy": policy, "action": action,
            })
        };
        let discard = serde_json::json!({"kind": "discard", "paths": ["a.txt", "new file.txt"]});

        let shown = read(&rpc).await.revision;
        std::thread::sleep(std::time::Duration::from_millis(20));
        std::fs::write(repo.join("a.txt"), "EDITED\n").unwrap();
        let error = rpc
            .handle(
                methods::RUN_WORKING_TREE_ACTION,
                action(&shown, "safeOnly", discard.clone()),
            )
            .await
            .err()
            .expect("a file rewritten after the confirm is refused, even with equal line counts");
        assert!(error.to_string().contains("changed since"), "{error}");
        assert_eq!(
            std::fs::read_to_string(repo.join("a.txt")).unwrap(),
            "EDITED\n"
        );
        assert!(repo.join("new file.txt").exists());

        let reply = through_the_wire(
            rpc.handle(
                methods::RUN_WORKING_TREE_ACTION,
                action(&read(&rpc).await.revision, "safeOnly", discard),
            )
            .await
            .unwrap(),
        );
        assert_eq!(reply, serde_json::json!({}));
        assert_eq!(git(&repo, &["status", "--porcelain"]), "");

        std::fs::write(repo.join("a.txt"), "parked\n").unwrap();
        rpc.handle(
            methods::RUN_WORKING_TREE_ACTION,
            action(
                &read(&rpc).await.revision,
                "safeOnly",
                serde_json::json!({"kind": "stashPush", "message": "Park the edit"}),
            ),
        )
        .await
        .unwrap();
        let tree = read(&rpc).await;
        assert!(tree.files.is_empty());
        assert_eq!(tree.stashes.len(), 1);
        assert!(
            tree.stashes[0].message.contains("Park the edit"),
            "{:?}",
            tree.stashes
        );
        let oid = tree.stashes[0].oid.clone();

        rpc.handle(
            methods::MUTATE,
            serde_json::json!({
                "op": "createChat", "chatId": "chat-busy", "deviceId": core.device_id,
                "spaceId": "space-repo", "cwd": repo,
            }),
        )
        .await
        .unwrap();
        core.sessions
            .set_status_for_test("chat-busy", zeron_proto::SessionStatus::Working);
        std::fs::write(repo.join("b.txt"), "b\n").unwrap();
        let tree = read(&rpc).await;
        assert!(tree.agent_active);
        for (policy, kind, expected) in [
            (
                "safeOnly",
                serde_json::json!({"kind": "stashPop", "oid": oid}),
                "Only stage",
            ),
            (
                "safeOnly",
                serde_json::json!({"kind": "stashDrop", "oid": oid}),
                "Only stage",
            ),
            (
                "safeOnly",
                serde_json::json!({"kind": "stashPush"}),
                "Only stage",
            ),
            (
                "safeOnly",
                serde_json::json!({"kind": "discard", "paths": ["b.txt"]}),
                "Only stage",
            ),
            (
                "blockAll",
                serde_json::json!({"kind": "stashDrop", "oid": oid}),
                "read-only",
            ),
        ] {
            let error = rpc
                .handle(
                    methods::RUN_WORKING_TREE_ACTION,
                    action(&tree.revision, policy, kind.clone()),
                )
                .await
                .err()
                .unwrap_or_else(|| panic!("{policy} {kind} must be refused"));
            assert!(error.to_string().contains(expected), "{error}");
        }
        assert_eq!(read(&rpc).await.stashes.len(), 1);
        assert!(repo.join("b.txt").exists());

        let reply = through_the_wire(
            rpc.handle(
                methods::RUN_WORKING_TREE_ACTION,
                action(
                    &tree.revision,
                    "allowAll",
                    serde_json::json!({"kind": "stashPop", "oid": oid}),
                ),
            )
            .await
            .expect("AllowAll pops while an agent runs"),
        );
        assert_eq!(reply, serde_json::json!({}));
        assert_eq!(
            std::fs::read_to_string(repo.join("a.txt")).unwrap(),
            "parked\n"
        );
        assert!(read(&rpc).await.stashes.is_empty());
    }

    #[tokio::test]
    async fn checkout_diffs_run_on_their_own_task_for_every_mode() {
        let dir = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(dir.path()).unwrap();
        let git = |args: &[&str]| git(&root, args).trim().to_owned();
        git(&["init", "-b", "main"]);
        std::fs::write(root.join("a.txt"), "a\n").unwrap();
        git(&["add", "."]);
        git(&["commit", "-m", "first"]);
        std::fs::write(root.join("b.txt"), "b\n").unwrap();
        git(&["add", "."]);
        git(&["commit", "-m", "second"]);
        let second = git(&["rev-parse", "HEAD"]);
        std::fs::write(root.join("a.txt"), "edited\n").unwrap();
        let data = tempfile::tempdir().unwrap();
        let core = crate::EngineCore::assemble(
            data.path(),
            std::sync::Arc::new(HarnessRegistry::new()),
            HarnessId::Mock,
            None,
        )
        .unwrap();
        let rpc = core.rpc_service();
        let diff = |params: serde_json::Value| {
            let rpc = rpc.clone();
            async move { rpc.handle(methods::GET_CHECKOUT_DIFF, params).await }
        };
        let paths = |reply: serde_json::Value| -> Vec<String> {
            reply["files"]
                .as_array()
                .unwrap()
                .iter()
                .map(|file| file["path"].as_str().unwrap().to_owned())
                .collect()
        };
        let commit = through_the_wire(
            diff(serde_json::json!({"cwd": root, "mode": "commit", "commitSha": second}))
                .await
                .unwrap(),
        );
        assert_eq!(paths(commit.clone()), ["b.txt"]);
        assert_eq!(commit["deviceId"], core.device_id.to_string());
        let working = through_the_wire(diff(serde_json::json!({"cwd": root})).await.unwrap());
        assert_eq!(paths(working), ["a.txt"]);
        for (params, expected) in [
            (
                serde_json::json!({"cwd": root, "mode": "commit"}),
                "commitSha required",
            ),
            (
                serde_json::json!({"cwd": root, "mode": "branch"}),
                "baseRef required",
            ),
            (
                serde_json::json!({"cwd": root, "mode": "turn"}),
                "chatId required",
            ),
            (
                serde_json::json!({"cwd": root, "mode": "turn", "chatId": "none"}),
                "no turn recorded",
            ),
        ] {
            let error = diff(params).await.err().expect(expected);
            assert!(error.to_string().contains(expected), "{error}");
        }
    }

    #[tokio::test]
    async fn dropping_a_spawned_rpc_capture_cancels_its_task() {
        let (alive, gone) = tokio::sync::oneshot::channel::<()>();
        let task = tokio_util::task::AbortOnDropHandle::new(tokio::spawn(async move {
            let _alive = alive;
            std::future::pending::<()>().await;
        }));
        drop(task);
        assert!(
            tokio::time::timeout(Duration::from_secs(5), gone)
                .await
                .expect("the task outlived its caller")
                .is_err()
        );
    }

    #[tokio::test]
    async fn a_commit_sha_that_is_a_git_option_never_reaches_git() {
        let dir = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(dir.path()).unwrap();
        let repo = root.join("repo");
        std::fs::create_dir(&repo).unwrap();
        let git = |args: &[&str]| git(&repo, args).trim().to_owned();
        git(&["init", "-b", "main"]);
        std::fs::write(repo.join("a.txt"), "a\n").unwrap();
        git(&["add", "."]);
        git(&["commit", "-m", "first"]);
        let head = git(&["rev-parse", "HEAD"]);
        let data = tempfile::tempdir().unwrap();
        let core = crate::EngineCore::assemble(
            data.path(),
            std::sync::Arc::new(HarnessRegistry::new()),
            HarnessId::Mock,
            None,
        )
        .unwrap();
        let rpc = core.rpc_service();
        let good = through_the_wire(
            rpc.handle(
                methods::GET_CHECKOUT_DIFF,
                serde_json::json!({"cwd": repo, "mode": "commit", "commitSha": head}),
            )
            .await
            .unwrap(),
        );
        let target = root.join("pwned");
        let option = format!("--output={}", target.display());
        for (method, params) in [
            (
                methods::GET_CHECKOUT_DIFF,
                serde_json::json!({"cwd": repo, "mode": "commit", "commitSha": option}),
            ),
            (
                methods::GET_CHECKOUT_FILE_DIFF_TEXT,
                serde_json::json!({
                    "cwd": repo, "checkoutId": good["checkoutId"], "mode": "commit",
                    "commitSha": option, "path": "a.txt", "diffChecksum": good["checksum"],
                }),
            ),
        ] {
            let error = rpc
                .handle(method, params)
                .await
                .err()
                .unwrap_or_else(|| panic!("{method} must refuse an option as a commit"));
            assert!(error.to_string().contains("not a commit id"), "{error}");
            assert!(!target.exists(), "{method} let git write {target:?}");
        }
        core.shutdown().await;
    }

    #[tokio::test]
    async fn commit_detail_writes_follow_the_revision_and_the_busy_policy() {
        let dir = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(dir.path()).unwrap();
        let git = |cwd: &std::path::Path, args: &[&str]| git(cwd, args).trim().to_owned();
        let repo = root.join("repo");
        let data = root.join("data");
        for path in [&repo, &data] {
            std::fs::create_dir(path).unwrap();
        }
        git(&repo, &["init", "-b", "main"]);
        for (key, value) in [
            ("user.name", "Lab"),
            ("user.email", "lab@example.com"),
            ("commit.gpgsign", "false"),
        ] {
            git(&repo, &["config", key, value]);
        }
        std::fs::write(repo.join("a.txt"), "a\n").unwrap();
        git(&repo, &["add", "."]);
        git(&repo, &["commit", "-m", "first"]);
        git(&repo, &["switch", "-c", "side"]);
        std::fs::write(repo.join("side.txt"), "s\n").unwrap();
        git(&repo, &["add", "."]);
        git(&repo, &["commit", "-m", "Add side"]);
        let clean = git(&repo, &["rev-parse", "HEAD"]);
        std::fs::write(repo.join("a.txt"), "side a\n").unwrap();
        git(&repo, &["commit", "-am", "Side a"]);
        let clashing = git(&repo, &["rev-parse", "HEAD"]);
        git(&repo, &["switch", "main"]);
        std::fs::write(repo.join("a.txt"), "main a\n").unwrap();
        git(&repo, &["commit", "-am", "Main a"]);
        let base = git(&repo, &["rev-parse", "HEAD"]);

        let core = crate::EngineCore::assemble(
            &data,
            std::sync::Arc::new(HarnessRegistry::new()),
            HarnessId::Mock,
            None,
        )
        .unwrap();
        let rpc = core.rpc_service();
        rpc.handle(
            methods::MUTATE,
            serde_json::json!({
                "op": "createSpace", "spaceId": "space-repo", "deviceId": core.device_id,
                "path": repo, "gitDetected": true,
            }),
        )
        .await
        .unwrap();
        async fn read(rpc: &EngineRpc) -> zeron_proto::WorkingTree {
            serde_json::from_value(through_the_wire(
                rpc.handle(
                    methods::GET_WORKING_TREE,
                    serde_json::json!({ "spaceId": "space-repo" }),
                )
                .await
                .unwrap(),
            ))
            .unwrap()
        }
        let action = |revision: &str, policy: &str, action: serde_json::Value| {
            serde_json::json!({
                "spaceId": "space-repo", "expectedRevision": revision,
                "busyPolicy": policy, "action": action,
            })
        };
        let checkout = serde_json::json!({"kind": "checkout", "targetRef": "side"});
        let detach = serde_json::json!({"kind": "checkout", "targetRef": clean, "detach": true});

        let shown = read(&rpc).await.revision;
        std::fs::write(repo.join("a.txt"), "edited\n").unwrap();
        let error = rpc
            .handle(
                methods::RUN_WORKING_TREE_ACTION,
                action(&shown, "safeOnly", checkout.clone()),
            )
            .await
            .err()
            .expect("a checkout against an older revision is refused");
        assert!(error.to_string().contains("changed since"), "{error}");
        let error = rpc
            .handle(
                methods::RUN_WORKING_TREE_ACTION,
                action(&read(&rpc).await.revision, "safeOnly", checkout.clone()),
            )
            .await
            .err()
            .expect("a dirty tree refuses a checkout");
        assert!(error.to_string().contains("uncommitted changes"), "{error}");
        git(&repo, &["checkout", "--", "a.txt"]);

        rpc.handle(
            methods::MUTATE,
            serde_json::json!({
                "op": "createChat", "chatId": "chat-busy", "deviceId": core.device_id,
                "spaceId": "space-repo", "cwd": repo,
            }),
        )
        .await
        .unwrap();
        core.sessions
            .set_status_for_test("chat-busy", zeron_proto::SessionStatus::Working);
        let tree = read(&rpc).await;
        assert!(tree.agent_active);
        let pick = |sha: &str| serde_json::json!({"kind": "cherryPick", "sha": sha});
        for (policy, kind, expected) in [
            ("safeOnly", checkout.clone(), "Only stage"),
            ("safeOnly", detach.clone(), "Only stage"),
            ("safeOnly", pick(&clean), "Only stage"),
            (
                "safeOnly",
                serde_json::json!({"kind": "createBranch", "name": "moved", "startPoint": clean, "checkout": true}),
                "Only stage",
            ),
            (
                "safeOnly",
                serde_json::json!({"kind": "abortCherryPick"}),
                "Only stage",
            ),
            (
                "blockAll",
                serde_json::json!({"kind": "createBranch", "name": "kept", "startPoint": clean}),
                "read-only",
            ),
        ] {
            let error = rpc
                .handle(
                    methods::RUN_WORKING_TREE_ACTION,
                    action(&tree.revision, policy, kind.clone()),
                )
                .await
                .err()
                .unwrap_or_else(|| panic!("{policy} {kind} must be refused"));
            assert!(error.to_string().contains(expected), "{error}");
        }
        assert_eq!(git(&repo, &["rev-parse", "HEAD"]), base);
        assert_eq!(git(&repo, &["branch", "--list", "moved", "kept"]), "");

        let reply = through_the_wire(
            rpc.handle(
                methods::RUN_WORKING_TREE_ACTION,
                action(
                    &tree.revision,
                    "safeOnly",
                    serde_json::json!({"kind": "createBranch", "name": "kept", "startPoint": clean}),
                ),
            )
            .await
            .expect("SafeOnly creates a branch without checking it out while an agent runs"),
        );
        assert_eq!(reply, serde_json::json!({}));
        assert_eq!(git(&repo, &["rev-parse", "refs/heads/kept"]), clean);
        assert_eq!(git(&repo, &["branch", "--show-current"]), "main");

        let reply = through_the_wire(
            rpc.handle(
                methods::RUN_WORKING_TREE_ACTION,
                action(&read(&rpc).await.revision, "allowAll", pick(&clean)),
            )
            .await
            .expect("AllowAll cherry-picks while an agent runs"),
        );
        let head = git(&repo, &["rev-parse", "HEAD"]);
        assert_eq!(
            reply,
            serde_json::json!({"commit": {"sha": head, "branch": "main", "subject": "Add side"}})
        );

        let reply = through_the_wire(
            rpc.handle(
                methods::RUN_WORKING_TREE_ACTION,
                action(&read(&rpc).await.revision, "allowAll", pick(&clashing)),
            )
            .await
            .unwrap(),
        );
        assert_eq!(reply, serde_json::json!({"conflicts": ["a.txt"]}));
        let tree = read(&rpc).await;
        assert_eq!(tree.cherry_pick_head.as_deref(), Some(clashing.as_str()));
        let reply = through_the_wire(
            rpc.handle(
                methods::RUN_WORKING_TREE_ACTION,
                action(
                    &tree.revision,
                    "allowAll",
                    serde_json::json!({"kind": "abortCherryPick"}),
                ),
            )
            .await
            .unwrap(),
        );
        assert_eq!(reply, serde_json::json!({}));
        assert_eq!(git(&repo, &["rev-parse", "HEAD"]), head);
        assert_eq!(git(&repo, &["status", "--porcelain"]), "");
        assert_eq!(read(&rpc).await.cherry_pick_head, None);

        let reply = through_the_wire(
            rpc.handle(
                methods::RUN_WORKING_TREE_ACTION,
                action(&read(&rpc).await.revision, "allowAll", detach),
            )
            .await
            .expect("AllowAll checks out while an agent runs"),
        );
        assert_eq!(reply, serde_json::json!({}));
        let tree = read(&rpc).await;
        assert_eq!(
            (tree.branch, tree.head_sha.as_deref()),
            (None, Some(clean.as_str()))
        );
    }

    #[tokio::test]
    async fn sync_branch_checks_the_head_and_the_busy_policy_and_replies_through_the_wire() {
        let dir = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(dir.path()).unwrap();
        let git = |cwd: &std::path::Path, args: &[&str]| git(cwd, args).trim().to_owned();
        let remote = root.join("remote.git");
        let repo = root.join("repo");
        let data = root.join("data");
        for path in [&repo, &data] {
            std::fs::create_dir(path).unwrap();
        }
        git(
            &root,
            &[
                "init",
                "-q",
                "--bare",
                "-b",
                "main",
                remote.to_str().unwrap(),
            ],
        );
        git(&repo, &["init", "-q", "-b", "main"]);
        git(
            &repo,
            &["remote", "add", "origin", remote.to_str().unwrap()],
        );
        std::fs::write(repo.join("a.txt"), "a\n").unwrap();
        git(&repo, &["add", "."]);
        git(&repo, &["commit", "-q", "-m", "first"]);
        git(&repo, &["push", "-q", "-u", "origin", "main"]);
        git(&repo, &["checkout", "-q", "-b", "feature/x"]);
        std::fs::write(repo.join("b.txt"), "b\n").unwrap();
        git(&repo, &["add", "."]);
        git(&repo, &["commit", "-q", "-m", "second"]);
        let head = git(&repo, &["rev-parse", "HEAD"]);

        let core = crate::EngineCore::assemble(
            &data,
            std::sync::Arc::new(HarnessRegistry::new()),
            HarnessId::Mock,
            None,
        )
        .unwrap();
        let rpc = core.rpc_service();
        rpc.handle(
            methods::MUTATE,
            serde_json::json!({
                "op": "createSpace", "spaceId": "space-repo", "deviceId": core.device_id,
                "path": repo, "gitDetected": true,
            }),
        )
        .await
        .unwrap();
        let sync = |head: &str, policy: &str, direction: serde_json::Value| {
            serde_json::json!({
                "spaceId": "space-repo", "expectedHeadSha": head,
                "busyPolicy": policy, "direction": direction,
            })
        };
        let push = serde_json::json!({"kind": "push", "setUpstream": true});
        let pull = serde_json::json!({"kind": "pull"});

        for (params, expected) in [
            (sync("0000", "safeOnly", push.clone()), "moved since"),
            (
                serde_json::json!({
                    "chatId": "chat-1", "expectedHeadSha": head, "direction": push,
                }),
                "spaceId",
            ),
        ] {
            let error = rpc
                .handle(methods::SYNC_BRANCH, params.clone())
                .await
                .err()
                .unwrap_or_else(|| panic!("{params} must be refused"));
            assert!(error.to_string().contains(expected), "{params}: {error}");
        }
        assert_eq!(
            git(&remote, &["for-each-ref", "--format=%(refname)"]),
            "refs/heads/main"
        );

        let reply = through_the_wire(
            rpc.handle(methods::SYNC_BRANCH, sync(&head, "safeOnly", push.clone()))
                .await
                .unwrap(),
        );
        assert_eq!(
            reply,
            serde_json::json!({
                "headSha": head, "upstream": "origin/feature/x",
                "ahead": 0, "behind": 0, "commits": 1,
            })
        );
        assert_eq!(git(&remote, &["rev-parse", "refs/heads/feature/x"]), head);

        rpc.handle(
            methods::MUTATE,
            serde_json::json!({
                "op": "createChat", "chatId": "chat-busy", "deviceId": core.device_id,
                "spaceId": "space-repo", "cwd": repo,
            }),
        )
        .await
        .unwrap();
        core.sessions
            .set_status_for_test("chat-busy", zeron_proto::SessionStatus::Working);
        std::fs::write(repo.join("c.txt"), "c\n").unwrap();
        git(&repo, &["add", "."]);
        git(&repo, &["commit", "-q", "-m", "third"]);
        let head = git(&repo, &["rev-parse", "HEAD"]);
        for (policy, direction, expected) in [
            ("safeOnly", pull.clone(), "Pull waits"),
            ("blockAll", push.clone(), "read-only"),
            ("blockAll", pull.clone(), "read-only"),
        ] {
            let error = rpc
                .handle(methods::SYNC_BRANCH, sync(&head, policy, direction.clone()))
                .await
                .err()
                .unwrap_or_else(|| panic!("{policy} {direction} must be refused"));
            assert!(error.to_string().contains(expected), "{error}");
        }
        assert_ne!(git(&remote, &["rev-parse", "refs/heads/feature/x"]), head);
        let reply = through_the_wire(
            rpc.handle(methods::SYNC_BRANCH, sync(&head, "safeOnly", push))
                .await
                .unwrap(),
        );
        assert_eq!(
            reply["commits"], 1,
            "SafeOnly still pushes under the banner"
        );
        assert_eq!(git(&remote, &["rev-parse", "refs/heads/feature/x"]), head);
        let reply = through_the_wire(
            rpc.handle(methods::SYNC_BRANCH, sync(&head, "allowAll", pull))
                .await
                .unwrap(),
        );
        assert_eq!(reply["commits"], 0);
        assert_eq!(reply["headSha"], head);
        core.shutdown().await;
    }

    async fn locked_checkout_lab() -> (
        tempfile::TempDir,
        std::path::PathBuf,
        crate::EngineCore,
        std::sync::Arc<EngineRpc>,
        zeron_proto::WorkingTree,
    ) {
        let dir = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(dir.path()).unwrap();
        let repo = root.join("repo");
        let data = root.join("data");
        for path in [&repo, &data] {
            std::fs::create_dir(path).unwrap();
        }
        for args in [
            &["init", "-q", "-b", "main"][..],
            &["config", "user.name", "Test"],
            &["config", "user.email", "test@example.com"],
            &["config", "commit.gpgsign", "false"],
        ] {
            let status = std::process::Command::new("git")
                .arg("-C")
                .arg(&repo)
                .args(args)
                .status()
                .unwrap();
            assert!(status.success());
        }
        std::fs::write(repo.join("a.txt"), "a\n").unwrap();
        for args in [&["add", "."][..], &["commit", "-q", "-m", "first"]] {
            let status = std::process::Command::new("git")
                .arg("-C")
                .arg(&repo)
                .args(args)
                .status()
                .unwrap();
            assert!(status.success());
        }
        std::fs::write(repo.join("a.txt"), "edited\n").unwrap();
        let core = crate::EngineCore::assemble(
            &data,
            std::sync::Arc::new(HarnessRegistry::new()),
            HarnessId::Mock,
            None,
        )
        .unwrap();
        let rpc = core.rpc_service();
        for params in [
            serde_json::json!({
                "op": "createSpace", "spaceId": "space-repo", "deviceId": core.device_id,
                "path": repo, "gitDetected": true,
            }),
            serde_json::json!({
                "op": "createChat", "chatId": "chat-later", "deviceId": core.device_id,
                "spaceId": "space-repo", "cwd": repo,
            }),
        ] {
            rpc.handle(methods::MUTATE, params).await.unwrap();
        }
        let tree = serde_json::from_value(through_the_wire(
            rpc.handle(
                methods::GET_WORKING_TREE,
                serde_json::json!({ "spaceId": "space-repo" }),
            )
            .await
            .unwrap(),
        ))
        .unwrap();
        (dir, repo, core, rpc, tree)
    }

    #[tokio::test]
    async fn a_git_action_behind_a_long_one_is_refused_instead_of_waiting_out_its_timeout() {
        let (_dir, _repo, core, rpc, tree) = locked_checkout_lab().await;
        let held = crate::working_tree::lock_checkout(&tree.checkout_id).await;
        let stage = rpc.handle(
            methods::RUN_WORKING_TREE_ACTION,
            serde_json::json!({
                "spaceId": "space-repo", "expectedRevision": tree.revision,
                "action": {"kind": "stage", "paths": ["a.txt"]},
            }),
        );
        let push = rpc.handle(
            methods::SYNC_BRANCH,
            serde_json::json!({
                "spaceId": "space-repo", "expectedHeadSha": tree.head_sha,
                "direction": {"kind": "push", "setUpstream": true},
            }),
        );
        let (stage, push) =
            tokio::time::timeout(Duration::from_secs(30), async { tokio::join!(stage, push) })
                .await
                .expect("a queued Git action must not wait for the lock holder");
        for result in [stage, push] {
            let error = result.err().expect("refused while the lock is held");
            assert!(
                error.to_string().contains("Another Git action is running"),
                "{error}"
            );
        }
        drop(held);
        rpc.handle(
            methods::RUN_WORKING_TREE_ACTION,
            serde_json::json!({
                "spaceId": "space-repo", "expectedRevision": tree.revision,
                "action": {"kind": "stage", "paths": ["a.txt"]},
            }),
        )
        .await
        .expect("runs once the lock is free");
        core.shutdown().await;
    }

    #[tokio::test]
    async fn a_project_in_a_subfolder_keeps_whole_repository_writes_out_of_the_rest() {
        let (_dir, repo, core, rpc, _) = locked_checkout_lab().await;
        for (path, text) in [("sub/mine.txt", "mine\n"), ("other/theirs.txt", "theirs\n")] {
            std::fs::create_dir_all(repo.join(path).parent().unwrap()).unwrap();
            std::fs::write(repo.join(path), text).unwrap();
        }
        for args in [&["add", "."][..], &["commit", "-q", "-m", "second"]] {
            let status = std::process::Command::new("git")
                .arg("-C")
                .arg(&repo)
                .args(args)
                .status()
                .unwrap();
            assert!(status.success());
        }
        for path in ["sub/mine.txt", "other/theirs.txt"] {
            std::fs::write(repo.join(path), "edited\n").unwrap();
        }
        std::fs::write(repo.join("other/untracked.txt"), "keep\n").unwrap();
        rpc.handle(
            methods::MUTATE,
            serde_json::json!({
                "op": "createSpace", "spaceId": "space-sub", "deviceId": core.device_id,
                "path": repo.join("sub"), "gitDetected": true,
            }),
        )
        .await
        .unwrap();
        let tree: zeron_proto::WorkingTree = serde_json::from_value(through_the_wire(
            rpc.handle(
                methods::GET_WORKING_TREE,
                serde_json::json!({ "spaceId": "space-sub" }),
            )
            .await
            .unwrap(),
        ))
        .unwrap();
        assert_eq!(
            std::path::Path::new(&tree.root),
            repo,
            "the enclosing repository"
        );
        assert!(
            tree.files
                .iter()
                .any(|file| file.path == "other/theirs.txt")
        );
        let action = |action: serde_json::Value| {
            serde_json::json!({
                "spaceId": "space-sub", "expectedRevision": tree.revision,
                "busyPolicy": "allowAll", "action": action,
            })
        };
        for (request, expected) in [
            (
                serde_json::json!({"kind": "stashPush"}),
                "Stash all would reach",
            ),
            (
                serde_json::json!({"kind": "commit", "message": "All", "all": true}),
                "Commit all would reach",
            ),
            (
                serde_json::json!({"kind": "discard", "paths": ["sub/mine.txt", "other/theirs.txt"]}),
                "other/theirs.txt is outside this project's folder, sub/",
            ),
        ] {
            let error = rpc
                .handle(methods::RUN_WORKING_TREE_ACTION, action(request.clone()))
                .await
                .err()
                .unwrap_or_else(|| panic!("{request} must be refused"));
            assert!(error.to_string().contains(expected), "{error}");
        }
        for path in ["sub/mine.txt", "other/theirs.txt"] {
            assert_eq!(
                std::fs::read_to_string(repo.join(path)).unwrap(),
                "edited\n"
            );
        }
        assert!(repo.join("other/untracked.txt").exists());
        rpc.handle(
            methods::RUN_WORKING_TREE_ACTION,
            action(serde_json::json!({"kind": "discard", "paths": ["sub/mine.txt"]})),
        )
        .await
        .expect("a file inside the project folder discards");
        assert_eq!(
            std::fs::read_to_string(repo.join("sub/mine.txt")).unwrap(),
            "mine\n"
        );
        core.shutdown().await;
    }

    #[tokio::test]
    async fn an_agent_that_starts_while_a_git_action_waits_for_the_lock_still_blocks_it() {
        let (_dir, repo, core, rpc, tree) = locked_checkout_lab().await;
        let requests = [
            (
                methods::RUN_WORKING_TREE_ACTION,
                serde_json::json!({
                    "spaceId": "space-repo", "expectedRevision": tree.revision,
                    "busyPolicy": "safeOnly", "action": {"kind": "discard", "paths": ["a.txt"]},
                }),
                "Only stage",
            ),
            (
                methods::SYNC_BRANCH,
                serde_json::json!({
                    "spaceId": "space-repo", "expectedHeadSha": tree.head_sha,
                    "busyPolicy": "safeOnly", "direction": {"kind": "pull"},
                }),
                "Pull waits",
            ),
        ];
        for (method, params, expected) in requests {
            core.sessions
                .set_status_for_test("chat-later", zeron_proto::SessionStatus::Idle);
            let held = crate::working_tree::lock_checkout(&tree.checkout_id).await;
            let (result, ()) = tokio::join!(rpc.handle(method, params), async {
                tokio::time::sleep(Duration::from_millis(300)).await;
                core.sessions
                    .set_status_for_test("chat-later", zeron_proto::SessionStatus::Working);
                drop(held);
            });
            let error = result
                .err()
                .unwrap_or_else(|| panic!("{method} ran under an agent that started first"));
            assert!(error.to_string().contains(expected), "{method}: {error}");
        }
        assert_eq!(
            std::fs::read_to_string(repo.join("a.txt")).unwrap(),
            "edited\n"
        );
        core.shutdown().await;
    }

    /// Every forwardable unary method gets a bounded reply deadline —
    /// interactive calls fail fast, network-bound git/update calls get the
    /// long leash, and nothing awaits forever (the "Sending…" wedge).
    #[test]
    fn forward_deadlines_are_tiered_and_bounded() {
        for method in [
            methods::LIST_MODELS,
            methods::LIST_COMMANDS,
            methods::GENERATE_COMMIT_MESSAGE,
        ] {
            assert_eq!(
                forward_deadline(method),
                std::time::Duration::from_secs(100)
            );
        }
        use std::time::Duration;
        assert_eq!(
            forward_deadline(methods::CREATE_WORKTREE),
            Duration::from_secs(120)
        );
        assert_eq!(
            forward_deadline(methods::RUN_WORKING_TREE_ACTION),
            Duration::from_secs(180)
        );
        assert!(
            forward_deadline(methods::RUN_WORKING_TREE_ACTION) > crate::working_tree::WRITE_TIMEOUT,
            "a forwarded write reports the engine's own outcome"
        );
        assert!(
            crate::working_tree::COMMIT_TIMEOUT + crate::working_tree::READ_TIMEOUT
                <= crate::working_tree::WRITE_TIMEOUT
        );
        assert!(
            crate::working_tree::WRITE_TIMEOUT
                + crate::working_tree::READ_TIMEOUT * 2
                + crate::working_tree::SYNC_TIMEOUT
                < forward_deadline(methods::SYNC_BRANCH),
            "a forwarded push waits out a held lock and still reports the engine's own outcome"
        );
        assert!(
            crate::commit_messages::ATTEMPT_TIMEOUT < crate::commit_messages::GENERATE_TIMEOUT
                && crate::commit_messages::GENERATE_TIMEOUT
                    < forward_deadline(methods::GENERATE_COMMIT_MESSAGE),
            "a forwarded generation reports the engine's own outcome"
        );
        for method in [
            methods::CLONE_REPO,
            methods::FETCH_ALL,
            methods::SYNC_BRANCH,
        ] {
            assert_eq!(forward_deadline(method), Duration::from_secs(15 * 60));
        }
        for method in [
            methods::GET_WORKING_TREE,
            methods::GET_WORKING_TREE_FILE_DIFF,
        ] {
            assert_eq!(forward_deadline(method), Duration::from_secs(30));
        }
        assert_eq!(
            forward_deadline(methods::APPLY_HARNESS_UPDATE),
            Duration::from_secs(20 * 60)
        );
        assert_eq!(
            forward_deadline(methods::CHECK_HARNESS_UPDATES),
            Duration::from_secs(4 * 60)
        );
        assert_eq!(
            forward_deadline(methods::LIST_BRANCHES),
            Duration::from_secs(30)
        );
        assert_eq!(
            forward_deadline(methods::QUEUE_COMMAND),
            Duration::from_secs(30)
        );
        assert_eq!(
            forward_deadline(methods::LIST_CHANGE_REQUEST_PAGE),
            Duration::from_secs(30)
        );
    }

    #[test]
    fn pull_request_errors_are_stable_and_sanitized() {
        for (error, expected) in [
            (
                ChangeRequestError::CliUnavailable,
                capability_errors::PULL_REQUESTS_CLI_UNAVAILABLE,
            ),
            (
                ChangeRequestError::Authentication,
                capability_errors::PULL_REQUESTS_AUTHENTICATION,
            ),
            (
                ChangeRequestError::RateLimited,
                capability_errors::PULL_REQUESTS_RATE_LIMITED,
            ),
            (
                ChangeRequestError::Timeout,
                capability_errors::PULL_REQUESTS_TIMEOUT,
            ),
            (
                ChangeRequestError::Decode,
                capability_errors::PULL_REQUESTS_DECODE,
            ),
            (
                ChangeRequestError::CommandFailed,
                capability_errors::PULL_REQUESTS_COMMAND_FAILED,
            ),
        ] {
            assert!(matches!(
                change_request_rpc_error(error),
                RpcError::Capability(code) if code == expected
            ));
        }
    }

    #[test]
    fn open_terminal_cwd_prefers_explicit_then_chat_then_space_then_home() {
        let home = crate::repos::session_home_dir()
            .unwrap()
            .to_string_lossy()
            .into_owned();
        assert_eq!(
            resolve_open_terminal_cwd(
                Some("/proj".into()),
                Some("/chat".into()),
                Some("/space".into()),
                std::path::Path::new("/scratch")
            )
            .unwrap(),
            "/proj"
        );
        assert_eq!(
            resolve_open_terminal_cwd(
                None,
                Some("/chat".into()),
                Some("/space".into()),
                std::path::Path::new("/scratch")
            )
            .unwrap(),
            "/chat"
        );
        assert_eq!(
            resolve_open_terminal_cwd(
                Some("~".into()),
                None,
                Some("/space".into()),
                std::path::Path::new("/scratch")
            )
            .unwrap(),
            "/space",
            "tilde is a fallback, not an override of the selected project"
        );
        assert_eq!(
            resolve_open_terminal_cwd(
                None,
                None,
                Some("/space".into()),
                std::path::Path::new("/scratch")
            )
            .unwrap(),
            "/space"
        );
        assert_eq!(
            resolve_open_terminal_cwd(None, None, None, std::path::Path::new("/scratch")).unwrap(),
            home
        );
        assert_eq!(
            resolve_open_terminal_cwd(
                Some("~".into()),
                Some("/chat".into()),
                None,
                std::path::Path::new("/scratch")
            )
            .unwrap(),
            "/chat"
        );
        assert_eq!(
            resolve_open_terminal_cwd(
                Some("  ".into()),
                Some("/chat".into()),
                None,
                std::path::Path::new("/scratch")
            )
            .unwrap(),
            "/chat"
        );
        assert_eq!(canvas_space_id("space-canvas:s1"), Some("s1"));
        assert_eq!(canvas_space_id("space-canvas:"), None);
        assert_eq!(canvas_space_id("chat-1"), None);
    }

    #[test]
    fn tool_file_paths_keep_workspace_activity_only() {
        assert_eq!(
            tool_file_path(&ToolCall::EditFile {
                path: "src/main.rs".into(),
                old_string: None,
                new_string: None,
            }),
            Some("src/main.rs")
        );
        assert_eq!(
            tool_file_path(&ToolCall::Exec {
                command: "cargo test".into(),
            }),
            None
        );
    }
}

#[cfg(test)]
mod context_usage_tests {
    use super::*;
    use futures::StreamExt;
    use std::sync::Arc;

    #[tokio::test]
    async fn replay_cutoff_travels_with_coalesced_backfill_and_live_content() {
        use crate::doc_host::{DocHost, DocHostConfig};
        use zeron_sync::chat_client::ChatDocSink;
        let dir = tempfile::tempdir().unwrap();
        let store = Arc::new(zeron_sync::DocsStore::open(dir.path()).unwrap());
        let host = DocHost::new(
            store.clone(),
            DocHostConfig {
                device_id: "viewer".into(),
                default_harness: zeron_proto::HarnessId::Mock,
                edge: None,
            },
        );
        let handle = host.open("replay-chat").unwrap();
        let sink = crate::chat2_host::EngineChatSink::new(&handle.doc_arc(), store, "replay-chat")
            .with_handle(Arc::downgrade(&handle));
        let mut stream = doc_messages_stream(handle.watch_messages(), handle.doc_arc());
        let first: zeron_doc::TranscriptUpdate =
            serde_json::from_value(stream.next().await.unwrap()).unwrap();
        assert!(first.replay_baseline.unwrap().entries.is_empty());

        let source = zeron_doc::SessionDoc::init("replay-chat").unwrap();
        let append = |id: &str| {
            source
                .push_message(&zeron_doc::SessionMessageEntry {
                    id: id.into(),
                    role: zeron_doc::MessageRole::Assistant,
                    parts: vec![zeron_doc::MessagePart::Text {
                        id: "text".into(),
                        text: id.into(),
                    }],
                    created_at: 0,
                    device_id: "writer".into(),
                    status: Some(zeron_doc::MessageStatus::Streaming),
                    continuation_of: None,
                    from_chat_id: None,
                    fork_import: false,
                    answers_request_id: None,
                    duration_ms: None,
                })
                .unwrap()
        };
        append("cached");
        sink.apply_checkpoint(&source.export_snapshot().unwrap(), 0)
            .unwrap();
        let checkpoint: zeron_doc::TranscriptUpdate = serde_json::from_value(
            tokio::time::timeout(std::time::Duration::from_secs(2), stream.next())
                .await
                .unwrap()
                .unwrap(),
        )
        .unwrap();
        assert!(
            checkpoint
                .replay_baseline
                .unwrap()
                .entries
                .contains_key("cached")
        );

        let version = source.doc().oplog_vv();
        append("away");
        sink.apply_replay_row(
            &source
                .doc()
                .export(loro::ExportMode::updates(&version))
                .unwrap(),
            1,
        );
        let version = source.doc().oplog_vv();
        append("live");
        sink.apply_row(
            &source
                .doc()
                .export(loro::ExportMode::updates(&version))
                .unwrap(),
            2,
        );
        // Neither the doc worker nor the RPC consumer ran between these
        // imports. They must not flatten their different presentation origins.
        let update: zeron_doc::TranscriptUpdate = serde_json::from_value(
            tokio::time::timeout(std::time::Duration::from_secs(2), stream.next())
                .await
                .unwrap()
                .unwrap(),
        )
        .unwrap();
        let cutoff = update.replay_baseline.unwrap();
        assert!(cutoff.entries.contains_key("away"));
        assert!(!cutoff.entries.contains_key("live"));
        let mut entries = vec![];
        zeron_doc::apply_transcript_frame(&mut entries, checkpoint.frame).unwrap();
        zeron_doc::apply_transcript_frame(&mut entries, update.frame).unwrap();
        assert_eq!(entries.len(), 3);

        let version = source.doc().oplog_vv();
        append("next-live");
        sink.apply_row(
            &source
                .doc()
                .export(loro::ExportMode::updates(&version))
                .unwrap(),
            3,
        );
        let update: zeron_doc::TranscriptUpdate = serde_json::from_value(
            tokio::time::timeout(std::time::Duration::from_secs(2), stream.next())
                .await
                .unwrap()
                .unwrap(),
        )
        .unwrap();
        assert!(
            update.replay_baseline.is_none(),
            "live updates must not resend the history watermark"
        );
        let mut reopened = doc_messages_stream(handle.watch_messages(), handle.doc_arc());
        let opening: zeron_doc::TranscriptUpdate =
            serde_json::from_value(reopened.next().await.unwrap()).unwrap();
        assert_eq!(
            opening.replay_baseline.unwrap().entries.len(),
            4,
            "reopening includes all existing content as history"
        );
        host.shutdown_workers().await;
    }

    #[tokio::test]
    async fn replay_metadata_and_backfill_leave_interleaved_local_content_live() {
        use crate::doc_host::{DocHost, DocHostConfig};
        use zeron_sync::chat_client::ChatDocSink;
        let dir = tempfile::tempdir().unwrap();
        let store = Arc::new(zeron_sync::DocsStore::open(dir.path()).unwrap());
        let host = DocHost::new(
            store.clone(),
            DocHostConfig {
                device_id: "host".into(),
                default_harness: zeron_proto::HarnessId::Mock,
                edge: None,
            },
        );
        let handle = host.open("interleaved").unwrap();
        let sink = crate::chat2_host::EngineChatSink::new(&handle.doc_arc(), store, "interleaved")
            .with_handle(Arc::downgrade(&handle));
        let mut stream = doc_messages_stream(handle.watch_messages(), handle.doc_arc());
        stream.next().await.unwrap();
        let source = zeron_doc::SessionDoc::init("interleaved").unwrap();
        sink.apply_checkpoint(&source.export_snapshot().unwrap(), 0)
            .unwrap();
        let entry = |id: &str| zeron_doc::SessionMessageEntry {
            id: id.into(),
            role: zeron_doc::MessageRole::Assistant,
            parts: vec![zeron_doc::MessagePart::Text {
                id: "text".into(),
                text: id.into(),
            }],
            created_at: 0,
            device_id: "host".into(),
            status: Some(zeron_doc::MessageStatus::Streaming),
            continuation_of: None,
            from_chat_id: None,
            fork_import: false,
            answers_request_id: None,
            duration_ms: None,
        };
        handle.doc().push_message(&entry("local-before")).unwrap();
        source.update_context_usage(Some(10), Some(100)).unwrap();
        sink.apply_replay_row(&source.export_snapshot().unwrap(), 1);
        let update: zeron_doc::TranscriptUpdate = serde_json::from_value(
            tokio::time::timeout(std::time::Duration::from_secs(2), stream.next())
                .await
                .unwrap()
                .unwrap(),
        )
        .unwrap();
        assert!(
            update.replay_baseline.is_none(),
            "metadata must not reset ongoing live animations"
        );
        let mut entries = vec![];
        zeron_doc::apply_transcript_frame(&mut entries, update.frame).unwrap();
        assert_eq!(entries[0].id, "local-before");
        let version = source.doc().oplog_vv();
        source.push_message(&entry("historical")).unwrap();
        handle.doc().push_message(&entry("local-between")).unwrap();
        sink.apply_replay_row(
            &source
                .doc()
                .export(loro::ExportMode::updates(&version))
                .unwrap(),
            2,
        );
        handle.doc().push_message(&entry("local-after")).unwrap();
        let update: zeron_doc::TranscriptUpdate = serde_json::from_value(
            tokio::time::timeout(std::time::Duration::from_secs(2), stream.next())
                .await
                .unwrap()
                .unwrap(),
        )
        .unwrap();
        let baseline = update.replay_baseline.unwrap();
        assert_eq!(baseline.entries.len(), 1);
        assert!(baseline.entries.contains_key("historical"));
        zeron_doc::apply_transcript_frame(&mut entries, update.frame).unwrap();
        assert_eq!(entries.len(), 4);
        host.shutdown_workers().await;
    }

    #[tokio::test]
    async fn replay_preserves_each_watchers_opening_cutoff_without_consuming_live_text() {
        use crate::doc_host::{DocHost, DocHostConfig};
        use zeron_sync::chat_client::ChatDocSink;
        let dir = tempfile::tempdir().unwrap();
        let store = Arc::new(zeron_sync::DocsStore::open(dir.path()).unwrap());
        let host = DocHost::new(
            store.clone(),
            DocHostConfig {
                device_id: "viewer".into(),
                default_harness: zeron_proto::HarnessId::Mock,
                edge: None,
            },
        );
        let handle = host.open("cached-replay").unwrap();
        let sink =
            crate::chat2_host::EngineChatSink::new(&handle.doc_arc(), store, "cached-replay")
                .with_handle(Arc::downgrade(&handle));
        let source = zeron_doc::SessionDoc::init("cached-replay").unwrap();
        let mut writer = zeron_doc::SegmentWriter::begin(&source, "reply", "host", 0).unwrap();
        let text = |id: &str, value: &str| zeron_doc::MessagePart::Text {
            id: id.into(),
            text: value.into(),
        };
        let cached = text("body", "café histórico");
        writer.sync(&[cached.clone()]).unwrap();
        sink.apply_checkpoint(&source.export_snapshot().unwrap(), 0)
            .unwrap();
        // Cached content exists before the first watcher and never enters
        // the changed-parts tracker. It may not have been painted yet.
        let mut first = doc_messages_stream(handle.watch_messages(), handle.doc_arc());
        let opening: zeron_doc::TranscriptUpdate =
            serde_json::from_value(first.next().await.unwrap()).unwrap();
        assert_eq!(
            opening.replay_baseline.unwrap().entries["reply"]["body"],
            "café histórico".len()
        );

        let live = text("body", "café histórico y nuevo");
        writer.sync(&[live.clone()]).unwrap();
        sink.apply_row(&source.export_snapshot().unwrap(), 1);
        let update: zeron_doc::TranscriptUpdate = serde_json::from_value(
            tokio::time::timeout(std::time::Duration::from_secs(2), first.next())
                .await
                .unwrap()
                .unwrap(),
        )
        .unwrap();
        assert!(update.replay_baseline.is_none());
        // A later subscriber sees a longer historical prefix, but must not
        // change the first subscriber's ongoing live animation.
        let mut second = doc_messages_stream(handle.watch_messages(), handle.doc_arc());
        let opening: zeron_doc::TranscriptUpdate =
            serde_json::from_value(second.next().await.unwrap()).unwrap();
        assert_eq!(
            opening.replay_baseline.unwrap().entries["reply"]["body"],
            "café histórico y nuevo".len()
        );

        let mut parts = vec![live];
        for ix in 0..2 {
            let id = format!("recovered-{ix}");
            parts.push(text(&id, "otro bloque histórico"));
            writer.sync(&parts).unwrap();
            sink.apply_replay_row(&source.export_snapshot().unwrap(), 2 + ix);
            for (stream, expected) in [
                (&mut first, "café histórico".len()),
                (&mut second, "café histórico y nuevo".len()),
            ] {
                let update: zeron_doc::TranscriptUpdate = serde_json::from_value(
                    tokio::time::timeout(std::time::Duration::from_secs(2), stream.next())
                        .await
                        .unwrap()
                        .unwrap(),
                )
                .unwrap();
                let baseline = update.replay_baseline.unwrap();
                assert_eq!(
                    baseline.entries["reply"].get("body"),
                    Some(&expected),
                    "replay must retain this watcher's opening cutoff, excluding later live bytes"
                );
                assert_eq!(
                    baseline.entries["reply"][&id],
                    "otro bloque histórico".len()
                );
                assert_eq!(baseline.entries["reply"].len(), 2 + ix as usize);
            }
        }
        host.shutdown_workers().await;
    }

    #[tokio::test]
    async fn reopening_rearms_history_for_previously_live_text() {
        use crate::doc_host::{DocHost, DocHostConfig};
        use zeron_sync::chat_client::ChatDocSink;
        let dir = tempfile::tempdir().unwrap();
        let store = Arc::new(zeron_sync::DocsStore::open(dir.path()).unwrap());
        let host = DocHost::new(
            store.clone(),
            DocHostConfig {
                device_id: "viewer".into(),
                default_harness: zeron_proto::HarnessId::Mock,
                edge: None,
            },
        );
        let handle = host.open("reopen").unwrap();
        let sink = crate::chat2_host::EngineChatSink::new(&handle.doc_arc(), store, "reopen")
            .with_handle(Arc::downgrade(&handle));
        let source = zeron_doc::SessionDoc::init("reopen").unwrap();
        let mut writer = zeron_doc::SegmentWriter::begin(&source, "reply", "host", 0).unwrap();
        let part = |text: &str| zeron_doc::MessagePart::Text {
            id: "body".into(),
            text: text.into(),
        };
        let mut stream = doc_messages_stream(handle.watch_messages(), handle.doc_arc());
        stream.next().await.unwrap();
        writer.sync(&[part("live")]).unwrap();
        sink.apply_row(&source.export_snapshot().unwrap(), 1);
        let _: serde_json::Value =
            tokio::time::timeout(std::time::Duration::from_secs(2), stream.next())
                .await
                .unwrap()
                .unwrap();
        drop(stream);
        // No unwatched commit clears provenance before the new attach.
        let mut stream = doc_messages_stream(handle.watch_messages(), handle.doc_arc());
        stream.next().await.unwrap();
        writer.sync(&[part("live plus recovered")]).unwrap();
        sink.apply_replay_row(&source.export_snapshot().unwrap(), 2);
        let update: zeron_doc::TranscriptUpdate = serde_json::from_value(
            tokio::time::timeout(std::time::Duration::from_secs(2), stream.next())
                .await
                .unwrap()
                .unwrap(),
        )
        .unwrap();
        assert_eq!(
            update.replay_baseline.unwrap().entries["reply"]["body"],
            "live plus recovered".len()
        );
        host.shutdown_workers().await;
    }

    #[tokio::test]
    async fn context_only_commits_reach_remote_watch_and_reconnect() {
        let host = zeron_doc::SessionDoc::init("context-chat").unwrap();
        host.update_context_usage(Some(42000), Some(200000))
            .unwrap();
        // The viewing engine reads a replicated document, with no harness process.
        let remote = Arc::new(zeron_doc::SessionDoc::from_doc(loro::LoroDoc::new()));
        remote
            .doc()
            .import(&host.export_snapshot().unwrap())
            .unwrap();
        let (tx, rx) = watch::channel(crate::doc_host::TranscriptSnapshot::default());
        let mut stream = doc_messages_stream(rx, remote.clone());
        let first = stream.next().await.unwrap();
        assert_eq!(first["contextUsage"]["tokens"], 42000);
        assert!(first.get("reset").is_some());
        let version = host.doc().oplog_vv();
        host.update_context_usage(Some(0), None).unwrap();
        remote
            .doc()
            .import(
                &host
                    .doc()
                    .export(loro::ExportMode::updates(&version))
                    .unwrap(),
            )
            .unwrap();
        tx.send_replace(crate::doc_host::TranscriptSnapshot::default());
        let update = tokio::time::timeout(std::time::Duration::from_secs(1), stream.next())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(update["contextUsage"]["tokens"], 0);
        assert_eq!(update["contextUsage"]["window"], 200000);
        let mut reconnect = doc_messages_stream(tx.subscribe(), remote.clone());
        assert_eq!(
            reconnect.next().await.unwrap()["contextUsage"],
            update["contextUsage"]
        );
        remote.clear_context_usage().unwrap();
        tx.send_replace(crate::doc_host::TranscriptSnapshot::default());
        assert!(stream.next().await.unwrap()["contextUsage"].is_null());
    }
}
