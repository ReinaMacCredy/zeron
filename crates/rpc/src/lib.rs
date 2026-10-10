//! zeron-rpc — the typed control plane (UiRpc / ControlRpc) over WebSocket + in-memory
//! transports, plus the device-room relay transport ({s,k,to,from} frames — [`device_room`]).
//!
//! Framing: ndjson envelopes, one JSON object per WebSocket text message (or per line on
//! byte transports), matching the shape of zeron's Effect RPC without the Effect runtime:
//!
//! - client → server: `{id, method, params}` to invoke, `{id, cancel: true}` to stop a stream;
//! - server → client: `{id, ok}` / `{id, err}` for unary calls,
//!   `{id, item}`* then `{id, done: true}` (or `{id, err}`) for streams.
//!
//! The server dispatches into an [`RpcService`]; the [`RpcClient`] offers `call` and
//! `subscribe`. Both ends run over any pair of string channels, so the in-memory transport
//! ([`memory_client`]) exercises the exact same code path as the WebSocket one.

use std::sync::Arc;

use async_trait::async_trait;
use futures::stream::BoxStream;
use serde::{Deserialize, Serialize};

mod client;
mod computer_host;
pub mod device_room;
mod server;

pub use client::{RpcClient, RpcSubscription, connect_ws};
pub use computer_host::ComputerHost;
pub use device_room::{
    DeviceFrameHeader, DeviceLink, HostRelay, HostRelayConfig, LinkCache, LinkCacheConfig,
    NudgeHandler, PeerLiveness, PeerLivenessProbe, StaticToken, TokenError, TokenSource,
    decode_device_frame, device_room_ws_url, encode_device_frame,
};
pub use server::{serve_connection, serve_ws_listener};

pub const COMPUTER_RUN_SECRET_ENV: &str = "ZERON_RUN_SECRET";

/// RPC method names — single source of truth for both ends.
/// Full surface: docs/research/feature-inventory.md §2.
pub mod methods {
    pub const VOICE_CAPABILITIES_V2: &str = "VoiceCapabilitiesV2";
    pub const PREPARE_VOICE_V2: &str = "PrepareVoiceV2";
    pub const OWN_VOICE_V2: &str = "OwnVoiceV2";
    pub const NEGOTIATE_VOICE_V2: &str = "NegotiateVoiceV2";
    pub const CONFIRM_VOICE_MEDIA_V2: &str = "ConfirmVoiceMediaV2";
    pub const REPORT_VOICE_MEDIA_V2: &str = "ReportVoiceMediaV2";
    pub const STOP_VOICE_V2: &str = "StopVoiceV2";
    pub const CANCEL_VOICE_ATTEMPT_V2: &str = "CancelVoiceAttemptV2";
    pub const WATCH_PREVIEWS: &str = "WatchPreviews";
    pub const LIST_HARNESSES: &str = "ListHarnesses";
    pub const CANCEL_INSTALL: &str = "CancelInstall";
    pub const INSTALL_HARNESS: &str = "InstallHarness";
    /// Flip a harness's enablement on the target device (Settings → Providers);
    /// replies with the device's fresh `ListHarnesses` catalog.
    pub const GET_TITLE_SETTINGS: &str = "GetTitleSettings";
    pub const SET_TITLE_SETTINGS: &str = "SetTitleSettings";
    /// `delegate_task` limits (d69). No params / `DelegationSettings` → `{}`.
    pub const GET_DELEGATION_SETTINGS: &str = "GetDelegationSettings";
    pub const SET_DELEGATION_SETTINGS: &str = "SetDelegationSettings";
    /// Settings > Tools opt-outs. No params / `ToolSettings` → `ToolSettingsState`.
    pub const GET_TOOL_SETTINGS: &str = "GetToolSettings";
    pub const SET_TOOL_SETTINGS: &str = "SetToolSettings";
    pub const SET_HARNESS_ENABLED: &str = "SetHarnessEnabled";
    pub const LIST_MODELS: &str = "ListModels";
    pub const LIST_SKILLS: &str = "ListSkills";
    pub const LIST_COMMANDS: &str = "ListCommands";
    /// Built-in tool groups and MCP servers a chat on `harness` can turn off.
    pub const LIST_HARNESS_TOOLS: &str = "ListHarnessTools";
    /// How a harness compacts its own context: `native`, `slash` or `none`.
    pub const HARNESS_COMPACTION: &str = "HarnessCompaction";
    /// Compact a chat's context in place. Returns once the harness is done —
    /// there is no live transcript to watch it through.
    pub const COMPACT_CHAT: &str = "CompactChat";
    /// Stop one live background task (a shell, or a subagent by its spawn
    /// tool id) of `chatId`'s running harness: `{chatId, taskId}`.
    pub const STOP_BACKGROUND_TASK: &str = "StopBackgroundTask";
    /// Tail of a live background shell's output file: `{chatId, taskId}` →
    /// stream of `{lines, running}`, ending after the task leaves the set.
    pub const WATCH_BACKGROUND_OUTPUT: &str = "WatchBackgroundOutput";
    /// Live output tokens/s of `chatId`'s running harness: `{chatId}` →
    /// stream of `{rate}` (`null` until enough output to measure).
    pub const WATCH_TOKEN_RATE: &str = "WatchTokenRate";
    /// Create a sidechat of `sourceChatId` under the client-minted `chatId`,
    /// replying with the created `Chat` row. The first prompt is the caller's
    /// to send afterwards, as an ordinary `QueueCommand` Run on `chatId`.
    pub const CREATE_SIDECHAT: &str = "CreateSidechat";
    // Thread tools (peer chats, d25). IPC-only; the caller identity comes from
    // the engine-spawned `zeron mcp --chat` process, never from the agent.
    /// Open a peer chat in `fromChatId`'s project, stamped `createdByChatId`,
    /// and hand it `prompt` as its first turn. Omitted run settings copy the
    /// creator's. `{fromChatId, prompt, title?, harness?, model?, reasoning?}`
    /// → `{chatId}`.
    pub const CREATE_THREAD: &str = "CreateThread";
    /// Deliver `text` from one thread to another: a turn opens on an idle
    /// target, a working target finishes its own turn first. Self-send and
    /// cross-project sends are refused. `{chatId, fromChatId, text}` → `{id}`.
    pub const SEND_THREAD_MESSAGE: &str = "SendThreadMessage";
    /// Change the threads `chatId`'s agent is woken for when they block on
    /// input. `{chatId, add?, remove?}` → `{watching: [chatId]}`.
    pub const SET_THREAD_WATCHES: &str = "SetThreadWatches";
    // Delegated child tasks (d67-d70), IPC-only like the thread tools.
    /// `DelegateTaskParams` → `TaskView`. Failures read `"<code>: <message>"`.
    pub const DELEGATE_TASK: &str = "DelegateTask";
    /// `{chatId, taskId, acknowledge?}` → `TaskView`; acknowledging a finished
    /// task settles its wake.
    pub const TASK_STATUS: &str = "TaskStatus";
    /// `{chatId, taskId}` → `{status}`: `cancel_requested` or the final status.
    pub const TASK_CANCEL: &str = "TaskCancel";
    /// `{chatId, taskId}` → `{}`: a timed-out `mode: wait` hands the result
    /// back to the wake.
    pub const TASK_WAKE_ALWAYS: &str = "TaskWakeAlways";
    /// Compact joined transcript count, last role and bounded prose preview. `{chatId}`.
    pub const GET_THREAD_SUMMARY: &str = "GetThreadSummary";
    /// Move the inter-thread messages waiting on `chatId`'s queue into its
    /// transcript, for an agent that asked for them mid-turn.
    /// `{chatId}` → `{messages: [{id, fromChatId, text}]}`.
    pub const TAKE_THREAD_MESSAGES: &str = "TakeThreadMessages";
    /// Ask the person in `chatId`'s own transcript and wait for the answer,
    /// authenticated by the run secret. `{chatId, runSecret, questions:
    /// [UserInputQuestion]}` → `{answers: [UserInputAnswer]}`; empty when the
    /// turn ended or nobody answered in time.
    pub const ASK_USER: &str = "AskUser";
    /// Fork `sourceChatId` after the assistant entry `entryId` into the
    /// client-minted `chatId`, replying with the created `Chat` row.
    pub const FORK_CHAT: &str = "ForkChat";
    /// Rewind `chatId` to before the user entry `entryId`, replying with
    /// `{prompt, removed}`.
    pub const REWIND_CHAT: &str = "RewindChat";
    pub const QUEUE_COMMAND: &str = "QueueCommand";
    pub const TAKE_PROJECT_ACTION_SETUP: &str = "TakeProjectActionSetup";
    /// Peer-to-peer delivery fallback: the SENDER's engine forwards a queued
    /// command entry (client-minted id and all) straight over the device-room
    /// link when its chat2 rows can't reach the edge but the host's peer link
    /// is alive. The host claims the id in its processed ledger before
    /// executing, so the doc row arriving later dedupes to a no-op —
    /// exactly-once by construction. Params `{chatId, entry}`.
    pub const RELAY_COMMAND: &str = "RelayCommand";
    /// User-driven delivery retry for a chat with unadopted queued sends:
    /// fresh chat2 socket, host nudge, drain pass, and a new delivery escort
    /// per pending command. Params `{chatId}`; IPC-only.
    pub const RETRY_DELIVERY: &str = "RetryDelivery";
    pub const FORK_SIDE_CHAT: &str = "ForkSideChat";
    pub const WATCH_DOC_MESSAGES: &str = "WatchDocMessages";
    /// Explicit user navigation, `{chatId}`. Prioritizes this device's sync
    /// connection; automatic subscriptions and retries must not call it.
    pub const FOCUS_CHAT: &str = "FocusChat";
    /// Messages typed while the agent was busy, held on the chat doc so every
    /// device sees the same queue. `{ chatId }` → `{ items: QueuedMessage[] }`.
    pub const WATCH_QUEUE: &str = "WatchQueue";
    /// Append to the queue. `{ chatId, text, attachments?, holdForTurnEnd? }` → `{ id }`.
    pub const QUEUE_MESSAGE: &str = "QueueMessage";
    /// Retype a queued message; empty text deletes it.
    /// `{ chatId, id, text }` → `{ changed }`.
    pub const UPDATE_QUEUED_MESSAGE: &str = "UpdateQueuedMessage";
    /// Acquire a host-authoritative edit lease for one queued row.
    pub const BEGIN_QUEUED_MESSAGE_EDIT: &str = "BeginQueuedMessageEdit";
    /// Renew an acquired queue edit lease.
    pub const RENEW_QUEUED_MESSAGE_EDIT: &str = "RenewQueuedMessageEdit";
    /// Commit, cancel, discard, or explicitly release an acquired edit.
    pub const FINISH_QUEUED_MESSAGE_EDIT: &str = "FinishQueuedMessageEdit";
    /// Reorder. `{ chatId, id, toIndex }` → `{ changed }`.
    pub const MOVE_QUEUED_MESSAGE: &str = "MoveQueuedMessage";
    pub const REMOVE_QUEUED_MESSAGE: &str = "RemoveQueuedMessage";
    /// Interrupt whatever is running and send this one. `{ chatId, id }` → `{ sent }`.
    pub const SEND_QUEUED_MESSAGE_NOW: &str = "SendQueuedMessageNow";
    /// Steer this row into the live turn without interrupting it.
    /// `{ chatId, id }` → `{ sent }`.
    pub const STEER_QUEUED_MESSAGE_NOW: &str = "SteerQueuedMessageNow";
    /// Nudge every open room client to verify liveness NOW (window focus,
    /// app foregrounded). No params; IPC-only. Each room ignores the hint
    /// unless it has been broadcast-quiet ≥30s, so this is cheap to spam.
    pub const PROBE_SYNC: &str = "ProbeSync";
    /// Live sync introspection (`zeron sync` / debug surfaces): per-room
    /// connection state, last pushed-frame/ack ages, rejoin/probe/resync
    /// counters for the workspace room and every open chat doc. No params;
    /// IPC-only.
    pub const SYNC_STATUS: &str = "SyncStatus";
    /// Pushed edge-connectivity posture (`zeron_proto::Connectivity`):
    /// current value first, then every change — the connection pill /
    /// composer-honesty / queued-badge feed. No params; IPC-only.
    pub const WATCH_CONNECTIVITY: &str = "WatchConnectivity";
    /// In-flight queued-attachment transfers (`zeron_proto::TransferProgress`
    /// list): current set first, then a fresh snapshot per landed chunk —
    /// the sending thumbnail's percent-ring feed. No params; IPC-only.
    pub const WATCH_TRANSFERS: &str = "WatchTransfers";
    pub const WATCH_CHATS: &str = "WatchChats";
    pub const WATCH_SIDEBAR_PREFERENCES: &str = "WatchSidebarPreferences";
    pub const WATCH_DEVICES: &str = "WatchDevices";
    pub const WATCH_SESSIONS: &str = "WatchSessions";
    /// Spaces registry (device+folder pairs) from the workspace doc.
    pub const WATCH_SPACES: &str = "WatchSpaces";
    /// Entity mutations against the workspace doc (feature-inventory §2 DataRpc).
    /// Params are tagged `{op: createChat|createSpace|renameSpace|deleteSpace|
    /// renameChat|setChatArchived|deleteChat|renameDevice|markChatSeen, …}`.
    pub const MUTATE: &str = "Mutate";
    /// This engine's identity → `{deviceId}` (IPC-only; never relay-forwarded —
    /// the answer is about whichever engine you are directly connected to).
    pub const LOCAL_DEVICE: &str = "LocalDevice";
    /// This engine runtime's fixed device and workspace identity.
    pub const ENGINE_INFO: &str = "EngineInfo";
    /// Readiness barrier for the engine runtime. The call completes once stores
    /// and journals are assembled, or fails with the assembly error.
    pub const ENGINE_READY: &str = "EngineReady";
    /// Ask a headless IPC owner to drain its runtime and exit successfully.
    /// Headed IPC owners do not implement this method: closing another app's
    /// engine behind its windows would leave that process unusable.
    pub const STOP_ENGINE: &str = "StopEngine";
    pub const AUTH_STATUS: &str = "AuthStatus";
    // AuthRpc mutations (feature-inventory §2 AuthRpc; IPC-only).
    pub const SIGN_IN: &str = "SignIn";
    pub const SIGN_IN_HEADLESS: &str = "SignInHeadless";
    pub const COMPLETE_SIGN_IN: &str = "CompleteSignIn";
    pub const SIGN_OUT: &str = "SignOut";
    pub const LIST_ORGS: &str = "ListOrgs";
    pub const CREATE_ORG: &str = "CreateOrg";
    pub const SELECT_ORG: &str = "SelectOrg";
    /// One-time local→synced profile import: what's importable (unary).
    pub const LOCAL_IMPORT_STATUS: &str = "LocalImportStatus";
    /// One-time local→synced profile import: run it (stream of progress items).
    pub const IMPORT_LOCAL_WORKSPACE: &str = "ImportLocalWorkspace";
    // Repos / worktrees / folders (ControlRpc, relay-forwardable).
    pub const LIST_REPOS: &str = "ListRepos";
    pub const ADD_REPO: &str = "AddRepo";
    pub const CLONE_REPO: &str = "CloneRepo";
    pub const CREATE_REPO: &str = "CreateRepo";
    pub const LIST_BRANCHES: &str = "ListBranches";
    pub const LIST_REFS: &str = "ListRefs";
    pub const LIST_GIT_HISTORY: &str = "ListGitHistory";
    /// Fuzzy commit-subject / SHA search over the complete public history.
    pub const SEARCH_GIT_HISTORY: &str = "SearchGitHistory";
    /// Resolve hosted profile images for a page of Git commit authors.
    pub const RESOLVE_GIT_AVATARS: &str = "ResolveGitAvatars";
    /// Update remote-tracking refs without changing HEAD, the index, or files.
    pub const FETCH_ALL: &str = "FetchAll";
    pub const SWITCH_REF: &str = "SwitchRef";
    pub const LIST_FOLDERS: &str = "ListFolders";
    /// The device's browse roots: home plus mounted drives/volumes.
    pub const LIST_DRIVES: &str = "ListDrives";
    /// Fuzzy relative-path search rooted in a known chat or space checkout.
    pub const SEARCH_FILES: &str = "SearchFiles";
    // Device-local workspace filesystem operations. All are relay-forwardable;
    // WatchWorkspaceFiles is the only streaming method in this group.
    pub const LIST_WORKSPACE_DIRECTORY: &str = "ListWorkspaceDirectory";
    pub const SEARCH_WORKSPACE_FILES: &str = "SearchWorkspaceFiles";
    pub const READ_WORKSPACE_IMAGE: &str = "ReadWorkspaceImage";
    pub const READ_WORKSPACE_FILE: &str = "ReadWorkspaceFile";
    pub const MOVE_WORKSPACE_ENTRY: &str = "MoveWorkspaceEntry";
    pub const DELETE_WORKSPACE_ENTRY: &str = "DeleteWorkspaceEntry";
    pub const WRITE_WORKSPACE_FILE: &str = "WriteWorkspaceFile";
    pub const WATCH_WORKSPACE_FILES: &str = "WatchWorkspaceFiles";
    pub const CREATE_WORKTREE: &str = "CreateWorktree";
    pub const DELETE_WORKTREE: &str = "DeleteWorktree";
    // Project Actions are private state on the device that owns the project.
    pub const LIST_PROJECT_ACTIONS: &str = "ListProjectActions";
    pub const UPSERT_PROJECT_ACTION: &str = "UpsertProjectAction";
    pub const DELETE_PROJECT_ACTION: &str = "DeleteProjectAction";
    pub const RUN_PROJECT_ACTION: &str = "RunProjectAction";
    pub const LIST_AGENT_PROFILES: &str = "ListAgentProfiles";
    pub const UPSERT_AGENT_PROFILE: &str = "UpsertAgentProfile";
    pub const DELETE_AGENT_PROFILE: &str = "DeleteAgentProfile";
    pub const REORDER_AGENT_PROFILES: &str = "ReorderAgentProfiles";
    pub const GET_SYSTEM_PROMPT: &str = "GetSystemPrompt";
    pub const SET_SYSTEM_PROMPT: &str = "SetSystemPrompt";
    // Terminals (ControlRpc, relay-forwardable; SubscribeTerminal streams).
    pub const OPEN_TERMINAL: &str = "OpenTerminal";
    pub const SUBSCRIBE_TERMINAL: &str = "SubscribeTerminal";
    pub const WRITE_TERMINAL: &str = "WriteTerminal";
    pub const RESIZE_TERMINAL: &str = "ResizeTerminal";
    pub const CLOSE_TERMINAL: &str = "CloseTerminal";
    pub const LIST_TERMINALS: &str = "ListTerminals";
    pub const READ_TERMINAL: &str = "ReadTerminal";
    pub const SEND_TERMINAL_INPUT: &str = "SendTerminalInput";
    pub const SET_TERMINAL_SELECTION: &str = "SetTerminalSelection";
    pub const WATCH_AGENT_TERMINALS: &str = "WatchAgentTerminals";
    // Device hub: refused to relay clients; no reply may carry the hub's origin.
    pub const WATCH_DEVICE_HUB: &str = "WatchDeviceHub";
    pub const CONFIGURE_DEVICE_HUB: &str = "ConfigureDeviceHub";
    pub const LIST_HUB_DEVICES: &str = "ListHubDevices";
    pub const OPEN_HUB_DEVICE: &str = "OpenHubDevice";
    pub const CLOSE_HUB_DEVICE: &str = "CloseHubDevice";
    pub const SHUTDOWN_HUB_DEVICE: &str = "ShutdownHubDevice";
    pub const WATCH_HUB_DEVICE_FRAMES: &str = "WatchHubDeviceFrames";
    pub const SEND_HUB_DEVICE_INPUT: &str = "SendHubDeviceInput";
    pub const SET_HUB_DEVICE_HINGE: &str = "SetHubDeviceHinge";
    pub const HUB_DEVICE_FOLD: &str = "HubDeviceFold";
    pub const GET_HUB_DEVICE_DETAIL: &str = "GetHubDeviceDetail";
    pub const RUN_HUB_DEVICE_ACTION: &str = "RunHubDeviceAction";
    pub const HUB_DEVICE_SCREENSHOT: &str = "HubDeviceScreenshot";
    pub const GET_HUB_DEVICE_AX_TREE: &str = "GetHubDeviceAxTree";
    pub const WATCH_HUB_DEVICE_APP_STATE: &str = "WatchHubDeviceAppState";
    pub const WATCH_HUB_DEVICE_EVENTS: &str = "WatchHubDeviceEvents";
    pub const TEST_DEVICE_HOST: &str = "TestDeviceHost";
    pub const DEVICE_TOOL_LIST: &str = "DeviceToolList";
    pub const DEVICE_TOOL_OPEN: &str = "DeviceToolOpen";
    pub const DEVICE_TOOL_SCREENSHOT: &str = "DeviceToolScreenshot";
    pub const DEVICE_TOOL_CLOSE: &str = "DeviceToolClose";
    /// `html_render`: store an agent's page in its chat (run-secret gated).
    pub const PUBLISH_HTML_RENDER: &str = "PublishHtmlRender";
    /// `html_preview`: lay a page out in the desktop app and return a
    /// screenshot (run-secret gated, never relay-forwarded).
    pub const HTML_PREVIEW: &str = "HtmlPreview";
    /// The desktop app's stream of pages to preview, and its answers.
    pub const WATCH_HTML_PREVIEW_HOST: &str = "WatchHtmlPreviewHost";
    pub const RESPOND_HTML_PREVIEW: &str = "RespondHtmlPreview";
    /// Checkout-diff stream for the target device's chats (DataRpc,
    /// relay-forwardable — diffs are produced where the checkout lives).
    pub const WATCH_CHECKOUT_DIFFS: &str = "WatchCheckoutDiffs";
    pub const WATCH_WORKSPACE_GIT_STATUS: &str = "WatchWorkspaceGitStatus";
    /// Current pull request for one checkout, resolved on the checkout's host device.
    pub const WATCH_CHECKOUT_CHANGE_REQUEST: &str = "WatchCheckoutChangeRequest";
    pub const LIST_CHANGE_REQUESTS: &str = "ListChangeRequests";
    pub const GET_CHANGE_REQUEST: &str = "GetChangeRequest";
    pub const LIST_CHANGE_REQUEST_STATS: &str = "ListChangeRequestStats";
    pub const GET_GIT_ACTIVITY: &str = "GetGitActivity";
    pub const LIST_LISTENING_SERVICES: &str = "ListListeningServices";
    pub const GET_SYSTEM_STATS: &str = "GetSystemStats";
    pub const GET_CHANGE_REQUEST_DIFF: &str = "GetChangeRequestDiff";
    pub const GET_CHANGE_REQUEST_ACTIVITY: &str = "GetChangeRequestActivity";
    pub const POST_CHANGE_REQUEST_COMMENT: &str = "PostChangeRequestComment";
    pub const SET_CHANGE_REQUEST_REACTION: &str = "SetChangeRequestReaction";
    pub const GET_CHANGE_REQUEST_PERMISSIONS: &str = "GetChangeRequestPermissions";
    pub const RUN_CHANGE_REQUEST_ACTION: &str = "RunChangeRequestAction";
    pub const UPDATE_CHANGE_REQUEST: &str = "UpdateChangeRequest";
    pub const LIST_CHANGE_REQUEST_LABEL_CANDIDATES: &str = "ListChangeRequestLabelCandidates";
    pub const SET_CHANGE_REQUEST_LABEL: &str = "SetChangeRequestLabel";
    pub const LIST_CHANGE_REQUEST_REVIEWER_CANDIDATES: &str = "ListChangeRequestReviewerCandidates";
    pub const SET_CHANGE_REQUEST_REVIEWER: &str = "SetChangeRequestReviewer";
    pub const SUBMIT_CHANGE_REQUEST_REVIEW: &str = "SubmitChangeRequestReview";
    pub const REPLY_TO_CHANGE_REQUEST_THREAD: &str = "ReplyToChangeRequestThread";
    pub const SET_CHANGE_REQUEST_THREAD_RESOLVED: &str = "SetChangeRequestThreadResolved";
    pub const PREPARE_CHANGE_REQUEST_CHAT: &str = "PrepareChangeRequestChat";
    pub const INVALIDATE_CHANGE_REQUESTS: &str = "InvalidateChangeRequests";
    pub const GET_CHECKOUT_DIFF: &str = "GetCheckoutDiff";
    /// Permanently restore one chat-owned checkout to its current HEAD and
    /// remove only its untracked, non-ignored paths.
    pub const DISCARD_WORKING_TREE: &str = "DiscardWorkingTree";
    pub const GET_CHECKOUT_FILE_DIFF_TEXT: &str = "GetCheckoutFileDiffText";
    // Source control for one space checkout, addressed by `WorkspaceTarget`.
    pub const GET_WORKING_TREE: &str = "GetWorkingTree";
    pub const GET_WORKING_TREE_FILE_DIFF: &str = "GetWorkingTreeFileDiff";
    pub const RUN_WORKING_TREE_ACTION: &str = "RunWorkingTreeAction";
    pub const SYNC_BRANCH: &str = "SyncBranch";
    pub const GENERATE_COMMIT_MESSAGE: &str = "GenerateCommitMessage";
    pub const CREATE_CHANGE_REQUEST: &str = "CreateChangeRequest";
    // Agent accounts (ControlRpc, relay-forwardable — CLI logins are per-device).
    pub const LIST_AGENT_ACCOUNTS: &str = "ListAgentAccounts";
    pub const ACTIVATE_AGENT_ACCOUNT: &str = "ActivateAgentAccount";
    pub const FORGET_AGENT_ACCOUNT: &str = "ForgetAgentAccount";
    pub const START_AGENT_LOGIN: &str = "StartAgentLogin";
    pub const COMPLETE_AGENT_LOGIN: &str = "CompleteAgentLogin";
    pub const POLL_AGENT_LOGIN: &str = "PollAgentLogin";
    pub const CANCEL_AGENT_LOGIN: &str = "CancelAgentLogin";
    // Uploads / attachments (ControlRpc, relay-forwardable — target the chat's host device).
    pub const UPLOAD_CHUNK: &str = "UploadChunk";
    pub const UPLOAD_COMMIT: &str = "UploadCommit";
    pub const READ_ATTACHMENT_CHUNK: &str = "ReadAttachmentChunk";
    /// Lazy full-tool-output fetch from the R2 sidecar by doc-resident ref
    /// (chat2-sync A3). Edge-direct from any device — never relay-forwarded.
    pub const FETCH_TOOL_BLOB: &str = "FetchToolBlob";
    // Updates (ControlRpc, relay-forwardable — a device reports/applies its own
    // binary's update). Stream: current UpdateStatus, then every change.
    pub const UPDATE_STATUS: &str = "UpdateStatus";
    /// Download + apply the newest release on the target device (symlink-managed
    /// installs; the service restart is scheduled after the reply flushes).
    pub const APPLY_UPDATE: &str = "ApplyUpdate";
    // Computer use (w88). Never relay-forwarded; relay clients are refused every
    // method here except the settings, status, still-frame and audit-history reads.
    /// Register as the desktop host that owns `cua-driver`. First stream wins; a
    /// second is refused. `{bundleId}` → stream of `ComputerHostStreamItem`: first
    /// `registered {hostToken}` then `request`/`cancel` items, or a lone
    /// `refused {retryAfterMs}` before the stream ends.
    pub const WATCH_COMPUTER_HOST_REQUESTS: &str = "WatchComputerHostRequests";
    /// The host's answer to one streamed request; unknown or cancelled ids are
    /// a no-op. `{hostToken, correlationId, result: CuaReply}` → `{}`.
    pub const RESPOND_COMPUTER_HOST_REQUEST: &str = "RespondComputerHostRequest";
    /// `zeron mcp` relays `notifications/cancelled` for one in-flight action.
    /// `{chatId, runSecret, requestId}` → `{cancelled}`.
    pub const CANCEL_COMPUTER_ACTION: &str = "CancelComputerAction";
    /// Written only by a Settings page on the engine's Mac. `ComputerSettings` → `{}`.
    pub const SET_COMPUTER_SETTINGS: &str = "SetComputerSettings";
    /// No params → `ComputerSettings`.
    pub const GET_COMPUTER_SETTINGS: &str = "GetComputerSettings";
    /// No params → `ComputerHostStatus`; never engages the driver.
    pub const GET_COMPUTER_STATUS: &str = "GetComputerStatus";
    /// The run's `computer_*` tool list for `tools/list`. `{chatId, runSecret}` →
    /// `ComputerCatalog`.
    pub const GET_COMPUTER_CATALOG: &str = "GetComputerCatalog";
    /// One `computer_*` tool call from `zeron mcp`, authenticated by the run
    /// secret. `{chatId, runSecret, tool, arguments, requestId}` → `McpToolResult`.
    pub const COMPUTER_ACTION: &str = "ComputerAction";
    /// Recent Computer mutations, newest first. `ComputerGetAuditHistoryInput`
    /// → `ComputerGetAuditHistoryResult`.
    pub const GET_COMPUTER_AUDIT_HISTORY: &str = "GetComputerAuditHistory";
    /// Latest still frame of a chat's Computer task, for remote viewers. `{chatId}`
    /// → `{frame: ComputerStillFrame | null}`.
    pub const GET_COMPUTER_STILL_FRAME: &str = "GetComputerStillFrame";
    /// Physical Escape from the desktop host. No params → `{stoppedChatIds}`.
    pub const COMPUTER_EMERGENCY_STOP: &str = "ComputerEmergencyStop";
    pub const COMPUTER_REVOKE_DESKTOP: &str = "ComputerRevokeDesktop";
    /// No params → stream of `ComputerEvent`, across desktop host changes.
    pub const WATCH_COMPUTER_EVENTS: &str = "WatchComputerEvents";
    /// Device-local agent CLI update lifecycle. The stream emits the complete
    /// ordered status list initially and after every transition.
    pub const WATCH_HARNESS_UPDATES: &str = "WatchHarnessUpdates";
    pub const CHECK_HARNESS_UPDATES: &str = "CheckHarnessUpdates";
    pub const APPLY_HARNESS_UPDATE: &str = "ApplyHarnessUpdate";
    pub const CANCEL_HARNESS_UPDATE: &str = "CancelHarnessUpdate";
    pub const DISMISS_HARNESS_UPDATE: &str = "DismissHarnessUpdate";
    pub const SET_HARNESS_UPDATE_POLICY: &str = "SetHarnessUpdatePolicy";
}

#[derive(Debug, thiserror::Error)]
pub enum RpcError {
    #[error("unknown method: {0}")]
    UnknownMethod(String),
    #[error("bad params: {0}")]
    BadParams(String),
    #[error("{0}")]
    Failed(String),
    #[error("transport: {0}")]
    Transport(String),
    #[error("connection closed")]
    Closed,
}

/// A client-originated frame.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClientFrame {
    pub id: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub method: Option<String>,
    #[serde(default, skip_serializing_if = "serde_json::Value::is_null")]
    pub params: serde_json::Value,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub cancel: bool,
}

/// A server-originated frame. Exactly one of `ok` / `err` / `item` / `done` is meaningful.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ServerFrame {
    pub id: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ok: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub err: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub item: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub done: bool,
}

/// What a service returns for one invocation.
pub enum RpcReply {
    /// Unary response — sent as `{id, ok}`.
    Value(serde_json::Value),
    /// Stream — each item sent as `{id, item}`, then `{id, done: true}` when it ends.
    Stream(BoxStream<'static, serde_json::Value>),
}

impl RpcReply {
    /// Serialize a value into a unary reply.
    pub fn value<T: Serialize>(value: &T) -> Result<Self, RpcError> {
        serde_json::to_value(value)
            .map(RpcReply::Value)
            .map_err(|e| RpcError::Failed(format!("serialize response: {e}")))
    }
}

/// Server-side dispatch: one implementation serves every transport.
#[async_trait]
pub trait RpcService: Send + Sync + 'static {
    async fn handle(&self, method: &str, params: serde_json::Value) -> Result<RpcReply, RpcError>;
}

/// Deserialize typed params out of the envelope's `params` value.
pub fn parse_params<T: serde::de::DeserializeOwned>(
    params: serde_json::Value,
) -> Result<T, RpcError> {
    serde_json::from_value(params).map_err(|e| RpcError::BadParams(e.to_string()))
}

/// Spawn an in-memory server for `service` and return a connected client.
/// Same envelopes, same dispatch loop as the WebSocket path — the in-process UI
/// transport (ARCHITECTURE §1 "zero serialization shortcuts").
pub fn memory_client(service: Arc<dyn RpcService>) -> RpcClient {
    let (client_out, server_in) = tokio::sync::mpsc::channel::<String>(256);
    let (server_out, client_in) = tokio::sync::mpsc::channel::<String>(256);
    tokio::spawn(serve_connection(service, server_out, server_in));
    RpcClient::new(client_out, client_in)
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::StreamExt;
    use std::sync::Mutex;

    struct TestService;

    struct CancelAwareService {
        dropped: Mutex<Option<tokio::sync::oneshot::Sender<()>>>,
    }

    struct DropSignal(Option<tokio::sync::oneshot::Sender<()>>);

    impl Drop for DropSignal {
        fn drop(&mut self) {
            if let Some(dropped) = self.0.take() {
                let _ = dropped.send(());
            }
        }
    }

    #[async_trait]
    impl RpcService for CancelAwareService {
        async fn handle(
            &self,
            method: &str,
            _params: serde_json::Value,
        ) -> Result<RpcReply, RpcError> {
            if !matches!(
                method,
                methods::WATCH_CHECKOUT_CHANGE_REQUEST | methods::WATCH_HARNESS_UPDATES | "Silent"
            ) {
                return Err(RpcError::UnknownMethod(method.into()));
            }
            let guard = DropSignal(self.dropped.lock().unwrap().take());
            let stream = futures::stream::unfold(guard, |guard| async move {
                let item = std::future::pending::<Option<(serde_json::Value, DropSignal)>>().await;
                drop(guard);
                item
            });
            if method == methods::WATCH_HARNESS_UPDATES {
                // Update watches send an initial status snapshot, then may stay
                // quiet indefinitely. Legacy peers do not send a readiness frame.
                return Ok(RpcReply::Stream(
                    futures::stream::once(async { serde_json::json!([]) })
                        .chain(stream)
                        .boxed(),
                ));
            }
            Ok(RpcReply::Stream(stream.boxed()))
        }
    }

    #[async_trait]
    impl RpcService for TestService {
        async fn handle(
            &self,
            method: &str,
            params: serde_json::Value,
        ) -> Result<RpcReply, RpcError> {
            match method {
                "Echo" => Ok(RpcReply::Value(params)),
                "Count" => {
                    let n = params.get("n").and_then(|v| v.as_u64()).unwrap_or(0);
                    Ok(RpcReply::Stream(
                        futures::stream::iter((0..n).map(|i| serde_json::json!(i))).boxed(),
                    ))
                }
                "Never" => Ok(RpcReply::Stream(futures::stream::pending().boxed())),
                "Boom" => Err(RpcError::Failed("boom".into())),
                other => Err(RpcError::UnknownMethod(other.into())),
            }
        }
    }

    #[tokio::test]
    async fn memory_call_stream_and_error() {
        let client = memory_client(Arc::new(TestService));

        let echoed = client
            .call("Echo", serde_json::json!({"x": 1}))
            .await
            .unwrap();
        assert_eq!(echoed, serde_json::json!({"x": 1}));

        let mut items = client
            .subscribe("Count", serde_json::json!({"n": 3}))
            .await
            .unwrap();
        let mut seen = Vec::new();
        while let Some(v) = items.recv().await {
            seen.push(v);
        }
        assert_eq!(
            seen,
            vec![
                serde_json::json!(0),
                serde_json::json!(1),
                serde_json::json!(2)
            ]
        );

        let err = client
            .call("Boom", serde_json::Value::Null)
            .await
            .unwrap_err();
        assert!(matches!(err, RpcError::Failed(m) if m == "boom"));
    }

    #[tokio::test]
    async fn checked_stream_acknowledges_support_and_preserves_unknown_method() {
        let client = memory_client(Arc::new(TestService));

        let mut items = client
            .subscribe_checked("Count", serde_json::json!({"n": 1}))
            .await
            .unwrap();
        assert_eq!(items.recv().await, Some(serde_json::json!(0)));
        assert_eq!(items.recv().await, None);

        let error = match client
            .subscribe_checked("FutureStream", serde_json::Value::Null)
            .await
        {
            Ok(_) => panic!("old service must reject unknown stream"),
            Err(error) => error,
        };
        assert!(matches!(error, RpcError::UnknownMethod(method) if method == "FutureStream"));
    }

    #[tokio::test]
    async fn dropping_checked_subscription_cancels_pending_server_stream() {
        let (dropped_tx, dropped_rx) = tokio::sync::oneshot::channel();
        let client = memory_client(Arc::new(CancelAwareService {
            dropped: Mutex::new(Some(dropped_tx)),
        }));
        let stream = client
            .subscribe_checked(
                methods::WATCH_CHECKOUT_CHANGE_REQUEST,
                serde_json::Value::Null,
            )
            .await
            .unwrap();

        drop(stream);

        tokio::time::timeout(std::time::Duration::from_secs(1), dropped_rx)
            .await
            .expect("server stream cancelled")
            .expect("drop signal");
    }

    #[tokio::test]
    async fn scoped_subscription_returns_before_first_item_and_cancels_silence() {
        let (dropped_tx, dropped_rx) = tokio::sync::oneshot::channel();
        let service = Arc::new(CancelAwareService {
            dropped: Mutex::new(Some(dropped_tx)),
        });
        let client = memory_client(service.clone());
        let stream = tokio::time::timeout(
            std::time::Duration::from_secs(1),
            client.subscribe_scoped("Silent", serde_json::Value::Null),
        )
        .await
        .unwrap()
        .unwrap();
        while service.dropped.lock().unwrap().is_some() {
            tokio::task::yield_now().await;
        }
        drop(stream);
        tokio::time::timeout(std::time::Duration::from_secs(1), dropped_rx)
            .await
            .unwrap()
            .unwrap();
    }

    #[tokio::test]
    async fn dropping_one_device_update_watch_preserves_the_other() {
        let (first_tx, first_rx) = tokio::sync::oneshot::channel();
        let (second_tx, mut second_rx) = tokio::sync::oneshot::channel();
        let first = memory_client(Arc::new(CancelAwareService {
            dropped: Mutex::new(Some(first_tx)),
        }));
        let second = memory_client(Arc::new(CancelAwareService {
            dropped: Mutex::new(Some(second_tx)),
        }));
        let first_watch = first
            .subscribe_checked(methods::WATCH_HARNESS_UPDATES, serde_json::Value::Null)
            .await
            .unwrap();
        let second_watch = second
            .subscribe_checked(methods::WATCH_HARNESS_UPDATES, serde_json::Value::Null)
            .await
            .unwrap();

        drop(first_watch);
        tokio::time::timeout(std::time::Duration::from_secs(1), first_rx)
            .await
            .expect("first device's quiet stream cancelled")
            .expect("first drop signal");
        assert!(matches!(
            second_rx.try_recv(),
            Err(tokio::sync::oneshot::error::TryRecvError::Empty)
        ));

        drop(second_watch);
        tokio::time::timeout(std::time::Duration::from_secs(1), second_rx)
            .await
            .expect("second device's quiet stream cancelled")
            .expect("second drop signal");
    }

    #[tokio::test]
    async fn websocket_round_trip() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(serve_ws_listener(listener, Arc::new(TestService)));

        let client = connect_ws(&format!("ws://127.0.0.1:{port}")).await.unwrap();
        let echoed = client
            .call("Echo", serde_json::json!("hello"))
            .await
            .unwrap();
        assert_eq!(echoed, serde_json::json!("hello"));

        let mut items = client
            .subscribe("Count", serde_json::json!({"n": 2}))
            .await
            .unwrap();
        assert_eq!(items.recv().await, Some(serde_json::json!(0)));
        assert_eq!(items.recv().await, Some(serde_json::json!(1)));
        assert_eq!(items.recv().await, None);
    }

    #[tokio::test]
    async fn handshake_with_origin_header_is_rejected() {
        use tokio_tungstenite::tungstenite::client::IntoClientRequest;

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(serve_ws_listener(listener, Arc::new(TestService)));

        // A browser page opening ws://127.0.0.1:{port} always sends Origin;
        // the server must refuse the handshake before serving any RPC.
        let mut req = format!("ws://127.0.0.1:{port}")
            .into_client_request()
            .unwrap();
        req.headers_mut()
            .insert("origin", "https://evil.example".parse().unwrap());
        let result = tokio_tungstenite::connect_async(req).await;
        assert!(
            result.is_err(),
            "handshake carrying an Origin header must be rejected"
        );

        // A native viewport (no Origin) still connects and can call RPC — the
        // reject must not be a blanket denial.
        let client = connect_ws(&format!("ws://127.0.0.1:{port}")).await.unwrap();
        let echoed = client.call("Echo", serde_json::json!("ok")).await.unwrap();
        assert_eq!(echoed, serde_json::json!("ok"));
    }

    #[tokio::test]
    async fn dropping_stream_receiver_cancels_server_side() {
        let client = memory_client(Arc::new(TestService));
        let items = client
            .subscribe("Never", serde_json::Value::Null)
            .await
            .unwrap();
        drop(items);
        // The next unary call still works — the dead stream didn't wedge the connection.
        let echoed = client.call("Echo", serde_json::json!(2)).await.unwrap();
        assert_eq!(echoed, serde_json::json!(2));
    }
}
