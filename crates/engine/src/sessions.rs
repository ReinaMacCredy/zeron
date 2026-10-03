//! SessionsEngine — per-chat agent runs: dispatch, steering, interrupts, input bridging,
//! journal + broadcast fan-out, and 120ms coalesced doc streaming.
//!
//! Pragmatic port of zeron's `sessions.ts` (spec: feature-inventory §3.2):
//! - every `AgentEvent` is (a) appended to the on-disk run journal, (b) broadcast to
//!   in-process subscribers, (c) folded via `fold_event_into_parts` and diffed into the
//!   chat's `SessionDoc` through `SegmentWriter` on a coalesced `STREAM_COMMIT_MS` timer;
//! - the user message entry is pushed to the doc immediately on dispatch (id = the
//!   command's client-minted message id, so optimistic echoes never flicker);
//! - a `Steered` event splits the assistant entry at the exact boundary;
//! - recovery (interrupt or a stale journal at boot) stamps the streaming entry `aborted`.
//!
//! Scope notes: sessions are keyed by chat id (one live run per chat). Zeron's pulse
//! loop is ported as the 15s liveness heartbeat in `drive_run`; its stall watchdog is
//! deliberately NOT ported (rejected in review — agents may legitimately wait on
//! something for far longer than any timeout, and a live child IS the working signal).
//! Every dying path must instead carry its own visible error (child crash with stderr,
//! spawn failure, stream error, engine-restart recovery).

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, PoisonError};

use chrono::Utc;
use futures::StreamExt;
use tokio::sync::{broadcast, mpsc, oneshot, watch};

use zeron_doc::{
    DocError, MessagePart, MessageRole, MessageStatus, STREAM_COMMIT_MS, SegmentWriter, SessionDoc,
    SessionMessageEntry, fold_event_into_parts, sanitize_tool_call,
};
use zeron_harness::{CancellationToken, Harness, HarnessError, RunControls, SteerMessage};
use zeron_proto::{
    AgentEvent, AnchorRole, Chat, Compaction, DoneStatus, HarnessId, PermissionMode, RunRequest,
    Session, SessionBranch, SessionStatus, ToolDiff, UserInputAnswer, UserInputQuestion,
};

use crate::computer::runtime::ComputerRuntime;

use crate::doc_host::{ChatDocHandle, DocHost};
use crate::registry::{HarnessDescriptor, HarnessRegistry};
use crate::run_journal::RunJournal;
use crate::{EngineError, new_id, now_ms};

/// One journaled event: the durable seq plus the event, as broadcast to subscribers.
#[derive(Debug, Clone)]
pub struct JournaledEvent {
    pub seq: u64,
    pub event: AgentEvent,
}

/// Result of a manual context compaction — also the RPC result shape.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CompactOutcome {
    pub compacted: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pre_tokens: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub post_tokens: Option<u64>,
    /// What the harness said when it compacted nothing ("Not enough messages
    /// to compact."), for the caller to surface as a notice.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

/// Outcome of a steer attempt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SteerOutcome {
    /// Delivered into the live run's steering mailbox.
    Accepted,
    /// A live run owns the turn, but an update prevents accepting another prompt.
    DeferredByUpdate,
    /// No live steerable run — the caller should dispatch the prompt as a new turn.
    NotSteerable,
}

type PendingInputs = Arc<Mutex<HashMap<String, oneshot::Sender<Vec<UserInputAnswer>>>>>;

/// A harness-native session id plus the cwd it was created under. Harness
/// session stores are cwd-scoped (claude keys conversations by project
/// directory — zeron sessions.ts:563 "harness session stores are keyed by
/// cwd"), so resume is only injected for runs launched from the same cwd.
#[derive(Debug, Clone)]
struct HarnessSessionRef {
    session_id: String,
    cwd: String,
}

/// Configuration baked into a live harness runtime. The steering mailbox only
/// carries prompt text, so routing a request whose model/effort/sandbox changed
/// would silently run it with the old process configuration. Such requests
/// must replace the runtime and resume its harness-native session instead.
#[derive(Debug, Clone, PartialEq)]
struct RuntimeConfig {
    harness_id: HarnessId,
    model: Option<String>,
    reasoning: Option<zeron_proto::ReasoningLevel>,
    model_options: serde_json::Map<String, serde_json::Value>,
    cwd: String,
    sandbox: zeron_proto::SandboxLevel,
    auto_approve: bool,
    permission_mode: zeron_proto::PermissionMode,
    worktree: Option<zeron_proto::WorktreeSpec>,
    computer_control: bool,
}

impl RuntimeConfig {
    fn from_request(harness_id: HarnessId, request: &RunRequest) -> Self {
        Self {
            harness_id,
            model: request.model.clone(),
            reasoning: request.reasoning,
            model_options: request.model_options.clone(),
            cwd: request.cwd.clone(),
            sandbox: request.sandbox,
            auto_approve: request.auto_approve,
            permission_mode: request.permission_mode,
            worktree: request.worktree.clone(),
            computer_control: request.computer_control,
        }
    }

    /// Attachments ride the steer mailbox (inlined by drivers that can,
    /// path refs in the text otherwise), so they never force a new run.
    fn can_route(&self, harness_id: HarnessId, request: &RunRequest, live_reasoning: bool) -> bool {
        let mut wanted = Self::from_request(harness_id, request);
        if live_reasoning && self.reasoning.is_some() && wanted.reasoning.is_some() {
            wanted.reasoning = self.reasoning;
        }
        self == &wanted
    }
}

struct RunHandle {
    run_id: String,
    steerable: bool,
    runtime_config: RuntimeConfig,
    steer_tx: mpsc::Sender<SteerMessage>,
    /// Harness-level cancellation (protocol interrupt + child teardown).
    interrupt_token: CancellationToken,
    /// Engine-level cancel: arms the run task's grace deadline so a harness that
    /// ignores its token can never strand the run.
    cancel: watch::Sender<bool>,
    engine_tx: mpsc::UnboundedSender<AgentEvent>,
    pending_inputs: PendingInputs,
    /// Steers accepted into the mailbox but not yet confirmed by a `Steered`
    /// event — the at-least-once ledger. A run can die with accepted steers
    /// still in its mailbox (idle reaper vs. a routed send; a mid-turn error
    /// discarding queued boundary steers): the run task drains this at exit
    /// and re-dispatches each entry as a fresh turn, so an accepted message
    /// can never silently evaporate from a transcript that shows it as sent.
    routed_steers: Arc<Mutex<std::collections::VecDeque<RoutedSteer>>>,
    /// This runtime's provider session holds the fork's copied history (a
    /// side chat's bootstrap went out on its run or on one of its steers).
    fork_history_sent: Arc<std::sync::atomic::AtomicBool>,
    computer: Arc<RunComputer>,
    /// Stop one live background task of this run (see `RunControls::stop_tasks`).
    stop_tx: mpsc::UnboundedSender<String>,
    /// The MCP call never carries the harness tool id, so the part is matched by task text.
    delegate_binds: Arc<Mutex<Vec<(String, String)>>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ComputerRun {
    pub run_id: String,
    pub turn_id: Option<String>,
    pub permission_mode: PermissionMode,
    pub run_computer_control: bool,
    pub turn_computer_control: bool,
    pub turn_desktop_revoked: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ComputerTurnForeground {
    pub declined: bool,
    pub apps: Vec<String>,
}

#[derive(Debug, Clone)]
struct TurnMark {
    id: String,
    computer_control: bool,
    desktop_revoked: bool,
    foreground: ComputerTurnForeground,
}

struct RunComputer {
    secret: String,
    revoked: AtomicBool,
    turn: Mutex<Option<TurnMark>>,
    ended: Mutex<Option<TurnMark>>,
}

impl RunComputer {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            secret: format!(
                "{}{}",
                uuid::Uuid::new_v4().simple(),
                uuid::Uuid::new_v4().simple()
            ),
            revoked: AtomicBool::new(false),
            turn: Mutex::new(None),
            ended: Mutex::new(None),
        })
    }

    fn accepts(&self, secret: &str) -> bool {
        !self.revoked.load(Ordering::Acquire) && secrets_match(&self.secret, secret)
    }

    fn begin_turn(&self, id: &str, computer_control: bool) {
        let mut turn = lock(&self.turn);
        let resumed = turn
            .take()
            .or_else(|| lock(&self.ended).take())
            .filter(|previous| previous.id == id);
        *turn = Some(TurnMark {
            id: id.to_owned(),
            computer_control,
            desktop_revoked: resumed.as_ref().is_some_and(|mark| mark.desktop_revoked),
            foreground: resumed.map(|mark| mark.foreground).unwrap_or_default(),
        });
    }

    fn revoke_turn(&self) -> Option<String> {
        let mut turn = lock(&self.turn);
        let turn = turn.as_mut().filter(|turn| !turn.desktop_revoked)?;
        turn.desktop_revoked = true;
        Some(turn.id.clone())
    }

    fn answer_foreground(&self, turn_id: Option<&str>, app: Option<&str>, granted: bool) -> bool {
        let mut turn = lock(&self.turn);
        let Some(turn) = turn
            .as_mut()
            .filter(|turn| turn_id.is_none_or(|id| id == turn.id))
        else {
            return false;
        };
        let foreground = &mut turn.foreground;
        match app {
            _ if !granted => foreground.declined = true,
            Some(app) if !foreground.apps.iter().any(|granted| granted == app) => {
                foreground.apps.push(app.to_owned());
            }
            _ => {}
        }
        true
    }

    fn end_turn(&self) -> Option<String> {
        let ended = lock(&self.turn).take();
        let id = ended.as_ref().map(|turn| turn.id.clone());
        *lock(&self.ended) = ended;
        id
    }

    fn turn(&self) -> Option<TurnMark> {
        lock(&self.turn).clone()
    }
}

fn secrets_match(expected: &str, given: &str) -> bool {
    expected.len() == given.len()
        && expected
            .bytes()
            .zip(given.bytes())
            .fold(0u8, |diff, (a, b)| diff | (a ^ b))
            == 0
}

enum RunMode {
    Turn { steerable: bool },
    Compaction,
}

struct RunSetup {
    handle: RunHandle,
    controls: RunControls,
    engine_rx: mpsc::UnboundedReceiver<AgentEvent>,
    cancel_rx: watch::Receiver<bool>,
}

impl RunSetup {
    fn new(harness_id: HarnessId, request: &RunRequest, mode: RunMode) -> Self {
        let (steerable, accepts_input, capacity) = match mode {
            RunMode::Turn { steerable } => (steerable, true, 32),
            RunMode::Compaction => (false, false, 1),
        };
        let (steer_tx, steer_rx) = mpsc::channel(capacity);
        let (cancel_tx, cancel_rx) = watch::channel(false);
        let (engine_tx, engine_rx) = mpsc::unbounded_channel();
        let pending_inputs: PendingInputs = Arc::new(Mutex::new(HashMap::new()));
        let pending = pending_inputs.clone();
        let events = engine_tx.clone();
        let interrupt_token = CancellationToken::new();
        let (stop_tx, stop_rx) = mpsc::unbounded_channel();
        let controls = RunControls {
            execution_lease: None,
            request_input: Box::new(move |questions: Vec<UserInputQuestion>| {
                let (mut tx, rx) = oneshot::channel();
                if accepts_input {
                    let (answer_tx, answer_rx) = oneshot::channel();
                    let request_id = new_id();
                    lock(&pending).insert(request_id.clone(), answer_tx);
                    let _ = events.send(AgentEvent::InputRequested {
                        request_id: request_id.clone(),
                        questions,
                    });
                    let pending = pending.clone();
                    let events = events.clone();
                    tokio::spawn(async move {
                        tokio::select! {
                            // A dropped resolver stays an error for the harness, as before.
                            answer = answer_rx => if let Ok(answer) = answer { let _ = tx.send(answer); },
                            _ = tx.closed() => {
                                lock(&pending).remove(&request_id);
                                let _ = events.send(AgentEvent::InputResolved { request_id, answers: None });
                            }
                        }
                    });
                } else {
                    let _ = tx.send(Vec::new());
                }
                rx
            }),
            steering: steer_rx,
            interrupt: interrupt_token.clone(),
            tool_server: None,
            stop_tasks: Some(stop_rx),
        };
        Self {
            handle: RunHandle {
                run_id: new_id(),
                steerable,
                runtime_config: RuntimeConfig::from_request(harness_id, request),
                steer_tx,
                interrupt_token,
                cancel: cancel_tx,
                engine_tx,
                pending_inputs,
                routed_steers: Arc::new(Mutex::new(std::collections::VecDeque::new())),
                fork_history_sent: Arc::new(std::sync::atomic::AtomicBool::new(false)),
                computer: RunComputer::new(),
                stop_tx,
                delegate_binds: Arc::new(Mutex::new(Vec::new())),
            },
            controls,
            engine_rx,
            cancel_rx,
        }
    }
}

/// One accepted-but-unconfirmed steer: enough to re-dispatch it verbatim.
#[derive(Debug, Clone)]
struct RoutedSteer {
    prompt: String,
    message_id: String,
    attachments: Vec<String>,
    computer_control: bool,
    /// The delivered text carried the fork's copied history: its delivery
    /// is recorded only when the runtime confirms consuming it (`Steered`).
    /// An orphan re-dispatches the bare prompt, still owing the history.
    fork_history: bool,
}

struct Inner {
    device_id: String,
    journal: Arc<RunJournal>,
    registry: Arc<HarnessRegistry>,
    /// The IPC port this engine actually bound, once it has; `zeron mcp`
    /// subprocesses dial it. A headed engine that lost the port race leaves
    /// this unset and its runs get no thread tools, not a server that cannot
    /// connect.
    tool_server_port: Mutex<Option<u16>>,
    /// Set-once (first wins), cleared on runtime retirement: sessions and
    /// doc-host reference each other through Arcs, so this back-edge must be
    /// severable for a replaced engine graph to drop.
    doc_host: Mutex<Option<DocHost>>,
    /// chat_id → live run.
    runs: Mutex<HashMap<String, RunHandle>>,
    /// chat_id → broadcast hub (retained across runs so subscribers survive turns).
    hubs: Mutex<HashMap<String, broadcast::Sender<JournaledEvent>>>,
    statuses: Mutex<HashMap<String, Session>>,
    sessions_tx: watch::Sender<Vec<Session>>,
    /// Last dispatched request per chat — the steer→new-turn fallback re-derives its
    /// run config from this (chat config rows land with the workspace doc in M4).
    last_requests: Mutex<HashMap<String, RunRequest>>,
    /// Harness-native session ids per chat (resume continuity across turns) —
    /// the live-process cache over the durable copy on the workspace chat row
    /// (zeron kept the same pair on `chats.harness_session_id`). An empty
    /// session id is the "do not resume" tombstone after a rejected resume.
    harness_sessions: Mutex<HashMap<String, HarnessSessionRef>>,
    /// Chats whose next run must branch the resumed harness session (fork
    /// and rewind). The durable copy is a `BranchPending` journal record
    /// newer than the chat's last `SessionStarted`; the run that applies it
    /// retires both through its own `SessionStarted`.
    pending_branches: Mutex<HashMap<String, Option<SessionBranch>>>,
    forked_at: Mutex<HashMap<String, Option<String>>>,
    sidechat_contexts: Mutex<HashMap<String, SidechatContext>>,
    /// Chats with a compaction in flight — one per chat, and the reservation
    /// is taken before the run slot so two RPCs can never both pass the check.
    compacting: Mutex<std::collections::HashSet<String>>,
    /// Chats whose last compaction timed out. A retry is refused until the
    /// chat completes a real turn (t3code `timedOutNativeCompactions`): the
    /// harness may still be compacting behind the timeout.
    compaction_timed_out: Mutex<std::collections::HashSet<String>>,
    /// Auto-titler for untitled chats (wired at engine assembly; absent in bare tests).
    titles: OnceLock<crate::titles::TitleGenerator>,
    generated_images: OnceLock<(crate::uploads::Uploads, std::path::PathBuf)>,
    /// Fired with `(chat_id, cwd)` when a user prompt starts a turn (fresh
    /// dispatch or accepted steer) — the diff sync snapshots the checkout tree
    /// for the Changes pane's "Latest turn" scope. Absent in bare tests.
    turn_listener: OnceLock<TurnListener>,
    computer: Mutex<Option<Arc<ComputerRuntime>>>,
}

/// Turn-start hook: called with `(chat_id, cwd)`.
pub type TurnListener = Arc<dyn Fn(&str, &str) + Send + Sync>;

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

#[derive(Clone)]
pub struct SessionsEngine {
    inner: Arc<Inner>,
}

impl Inner {
    /// The thread-tools server for one chat's run: this binary as
    /// `zeron mcp --chat <id> --port <port>`. `None` until an IPC port is
    /// recorded, so a run never gets a server it cannot dial.
    fn tool_server_for(
        &self,
        chat_id: &str,
        secret: &str,
        computer_control: bool,
    ) -> Option<zeron_harness::ToolServer> {
        let port = (*lock(&self.tool_server_port))?;
        let command = std::env::current_exe().ok()?.to_string_lossy().into_owned();
        Some(zeron_harness::ToolServer {
            command,
            args: vec![
                "mcp".into(),
                "--chat".into(),
                chat_id.into(),
                "--port".into(),
                port.to_string(),
            ],
            env: vec![(zeron_rpc::COMPUTER_RUN_SECRET_ENV.into(), secret.into())],
            guidance: Some(match computer_control {
                true => format!(
                    "{}\n\n{}",
                    crate::computer::tools::guidance::computer_tool_instructions(),
                    crate::doc_host::DELEGATION_GUIDANCE
                ),
                false => crate::doc_host::DELEGATION_GUIDANCE.to_string(),
            }),
        })
    }

    fn computer(&self) -> Option<Arc<ComputerRuntime>> {
        lock(&self.computer).clone()
    }

    fn bind_delegates(&self, chat_id: &str, run_id: &str, parts: &mut [MessagePart]) {
        let Some(binds) = lock(&self.runs)
            .get(chat_id)
            .filter(|run| run.run_id == run_id)
            .map(|run| run.delegate_binds.clone())
        else {
            return;
        };
        lock(&binds).retain(|(task, child)| {
            let slot = parts.iter_mut().find_map(|part| match part {
                MessagePart::Tool {
                    call, subagent_ref, ..
                } if subagent_ref.is_none() && call.delegate_task_text() == Some(task.as_str()) => {
                    Some(subagent_ref)
                }
                _ => None,
            });
            match slot {
                Some(slot) => {
                    *slot = Some(child.clone());
                    false
                }
                None => true,
            }
        });
    }

    fn computer_turn_ended(&self, chat_id: &str, turn_id: Option<String>) {
        if let (Some(computer), Some(turn_id)) = (self.computer(), turn_id) {
            computer.turn_ended(chat_id, &turn_id);
        }
    }
}

impl SessionsEngine {
    pub fn new(
        device_id: String,
        journal: Arc<RunJournal>,
        registry: Arc<HarnessRegistry>,
    ) -> Self {
        let (sessions_tx, _) = watch::channel(Vec::new());
        Self {
            inner: Arc::new(Inner {
                device_id,
                journal,
                registry,
                tool_server_port: Mutex::new(None),
                doc_host: Mutex::new(None),
                runs: Mutex::new(HashMap::new()),
                hubs: Mutex::new(HashMap::new()),
                statuses: Mutex::new(HashMap::new()),
                sessions_tx,
                last_requests: Mutex::new(HashMap::new()),
                harness_sessions: Mutex::new(HashMap::new()),
                pending_branches: Mutex::new(HashMap::new()),
                forked_at: Mutex::new(HashMap::new()),
                sidechat_contexts: Mutex::new(HashMap::new()),
                compacting: Mutex::new(std::collections::HashSet::new()),
                compaction_timed_out: Mutex::new(std::collections::HashSet::new()),
                titles: OnceLock::new(),
                generated_images: OnceLock::new(),
                turn_listener: OnceLock::new(),
                computer: Mutex::new(None),
            }),
        }
    }

    /// Record the IPC port this engine serves, so every later turn hands its
    /// agent the thread tools (`zeron mcp --chat <id> --port <port>`).
    pub fn set_tool_server_port(&self, port: u16) {
        *lock(&self.inner.tool_server_port) = Some(port);
    }

    /// Wire the doc host (called once at engine assembly; the two services are mutually
    /// referential by design — sessions stream into docs, docs execute commands here).
    pub fn set_doc_host(&self, host: DocHost) {
        // First set wins (the OnceLock contract this slot replaced).
        let mut slot = lock(&self.inner.doc_host);
        if slot.is_none() {
            *slot = Some(host);
        }
    }

    /// Sever the doc-host back-edge (runtime retirement; the doc host's
    /// `shutdown_workers` clears its own sessions edge). Every access site
    /// already treats a missing doc host as "not wired".
    pub fn clear_doc_host(&self) {
        lock(&self.inner.doc_host).take();
        lock(&self.inner.computer).take();
    }

    pub fn set_computer(&self, computer: Arc<ComputerRuntime>) {
        let mut slot = lock(&self.inner.computer);
        if slot.is_none() {
            *slot = Some(computer);
        }
    }

    pub fn computer(&self) -> Option<Arc<ComputerRuntime>> {
        self.inner.computer()
    }

    pub(crate) fn workspace(&self) -> Option<crate::workspace_host::WorkspaceHost> {
        self.inner.workspace()
    }

    /// Bind generated-image intake to the same profile store used by attachment RPCs.
    pub fn set_generated_images(
        &self,
        uploads: crate::uploads::Uploads,
        allowed_root: std::path::PathBuf,
    ) {
        let _ = self.inner.generated_images.set((uploads, allowed_root));
    }

    /// Wire the chat auto-titler (called once at engine assembly). After each
    /// completed exchange the run task fires it for still-untitled chats.
    pub fn set_titles(&self, titles: crate::titles::TitleGenerator) {
        let _ = self.inner.titles.set(titles);
    }

    /// Wire the turn-start listener (called once at engine assembly).
    pub fn set_turn_listener(&self, listener: TurnListener) {
        let _ = self.inner.turn_listener.set(listener);
    }

    fn note_turn_start(&self, chat_id: &str, cwd: &str) {
        if let Some(listener) = self.inner.turn_listener.get() {
            listener(chat_id, cwd);
        }
    }

    fn doc_handle(&self, chat_id: &str) -> Result<Arc<ChatDocHandle>, EngineError> {
        let host = self
            .inner
            .doc_host()
            .ok_or_else(|| EngineError::Other("doc host not wired into sessions engine".into()))?;
        host.open(chat_id)
    }

    /// Status watch: the full session list, re-sent on every transition.
    pub fn watch_sessions(&self) -> watch::Receiver<Vec<Session>> {
        self.inner.sessions_tx.subscribe()
    }

    /// Ask the chat's live run to stop one background task (a shell's task
    /// id, or a subagent's spawn tool id). Harnesses without a stop ignore it.
    pub fn stop_background_task(&self, chat_id: &str, task_id: &str) -> Result<(), EngineError> {
        let runs = lock(&self.inner.runs);
        let handle = runs
            .get(chat_id)
            .ok_or_else(|| EngineError::Other(format!("no live run for chat {chat_id}")))?;
        handle
            .stop_tx
            .send(task_id.to_owned())
            .map_err(|_| EngineError::Other("the run has ended".into()))
    }

    pub fn bind_delegate(&self, chat_id: &str, task: &str, child_chat_id: &str) {
        if let Some(run) = lock(&self.inner.runs).get(chat_id) {
            lock(&run.delegate_binds).push((task.trim().to_string(), child_chat_id.to_string()));
        }
    }

    pub fn session_status(&self, chat_id: &str) -> Option<Session> {
        lock(&self.inner.statuses).get(chat_id).cloned()
    }

    /// Whether this harness takes a prompt *during* a turn, rather than only at
    /// the boundary between turns. The queue drain's central question: a
    /// mid-turn agent gets the message now, everything else holds it until the
    /// turn ends. Catalog lookup only — never spawns.
    pub fn steers_mid_turn(&self, harness: HarnessId) -> bool {
        self.inner
            .registry
            .descriptors()
            .iter()
            .find(|d| d.id == harness)
            .is_some_and(HarnessDescriptor::steers_mid_turn)
    }

    /// Whether this chat has a turn in flight — streaming, or parked on a
    /// question it is still owed an answer to (see [`is_active`]). A persistent
    /// session parked BETWEEN turns is not: it holds a warm child with nothing
    /// outstanding, and takes the next prompt straight through its mailbox.
    pub fn turn_in_flight(&self, chat_id: &str) -> bool {
        lock(&self.inner.statuses)
            .get(chat_id)
            .is_some_and(is_active)
    }

    /// Any run currently working or blocked on input — the auto-updater's
    /// "don't restart from under a session" gate. Background subagents count:
    /// a restart would kill them with the child.
    pub fn any_active(&self) -> bool {
        lock(&self.inner.statuses).values().any(|s| {
            matches!(
                s.status,
                SessionStatus::Working | SessionStatus::AwaitingInput
            )
        })
    }

    /// A text prompt for `chat_id` would land in the mailbox of a live
    /// turn-boundary agent mid-turn. The agent reads it only after the turn,
    /// but a mailbox delivery writes the user message now — above the reply
    /// still streaming for the message before it. Such prompts belong in the
    /// visible queue. `request` = a Run that may differ from the live config
    /// (a different config restarts the runtime instead, which is no hold).
    pub fn defers_to_turn_end(
        &self,
        chat_id: &str,
        request: Option<(HarnessId, &RunRequest)>,
    ) -> bool {
        if !self.turn_in_flight(chat_id) {
            return false;
        }
        let live = lock(&self.inner.runs).get(chat_id).map(|h| {
            (
                h.runtime_config.harness_id,
                h.steerable
                    && request.is_none_or(|(id, r)| {
                        let live_reasoning = self
                            .inner
                            .registry
                            .resolve(id)
                            .is_ok_and(|harness| harness.live_reasoning());
                        h.runtime_config.can_route(id, r, live_reasoning)
                    }),
            )
        });
        live.is_some_and(|(harness, routable)| routable && !self.steers_mid_turn(harness))
    }

    /// Whether an accepted agent update gates the chat's live run, so a new
    /// prompt waits in the queue rather than joining that run's transcript.
    pub fn live_run_update_pending(&self, chat_id: &str) -> bool {
        let harness = lock(&self.inner.runs)
            .get(chat_id)
            .map(|h| h.runtime_config.harness_id);
        harness.is_some_and(|harness| self.inner.registry.update_pending(harness))
    }

    /// The chat's live run accepts steering into its mailbox.
    pub fn live_run_steerable(&self, chat_id: &str) -> bool {
        lock(&self.inner.runs)
            .get(chat_id)
            .is_some_and(|h| h.steerable)
    }

    /// The last request dispatched for a chat (steer→new-turn fallback).
    pub fn last_request(&self, chat_id: &str) -> Option<RunRequest> {
        lock(&self.inner.last_requests).get(chat_id).cloned()
    }

    /// Subscribe to a chat's live event stream: returns the journal replay after
    /// `after_seq` plus a live receiver. Subscribe-then-replay ordering means overlap
    /// (dedupe by seq) rather than gaps.
    pub fn subscribe(
        &self,
        chat_id: &str,
        after_seq: u64,
    ) -> Result<(Vec<JournaledEvent>, broadcast::Receiver<JournaledEvent>), EngineError> {
        let rx = {
            let mut hubs = lock(&self.inner.hubs);
            hubs.entry(chat_id.to_string())
                .or_insert_with(|| broadcast::channel(1024).0)
                .subscribe()
        };
        let replay = self
            .inner
            .journal
            .replay(chat_id, after_seq)?
            .into_iter()
            .map(|(seq, event)| JournaledEvent { seq, event })
            .collect();
        Ok((replay, rx))
    }

    /// The full diff a tool result carried, read back from this host's run
    /// journal — the doc keeps only its stats. Subagent results are wrapped
    /// in the parent's journal (a subagent doc id names its parent, see
    /// [`subagent_doc_id`]), so the scan looks through the tag. `None` when
    /// this device never ran the chat (a remote peer's journal).
    pub fn journaled_tool_diff(&self, chat_id: &str, part_id: &str) -> Option<ToolDiff> {
        let chat_id = chat_id
            .split_once("--sub--")
            .map_or(chat_id, |(parent, _)| parent);
        let events = match self.inner.journal.replay(chat_id, 0) {
            Ok(events) => events,
            Err(err) => {
                tracing::warn!(chat = %chat_id, error = %err, "journal scan for tool diff failed");
                return None;
            }
        };
        events.into_iter().rev().find_map(|(_, event)| {
            let event = match event {
                AgentEvent::Subagent { event, .. } => *event,
                other => other,
            };
            match event {
                AgentEvent::ToolResult {
                    id,
                    diff: Some(diff),
                    ..
                } if id == part_id => Some(diff),
                _ => None,
            }
        })
    }

    fn ensure_sidechat_active(&self, chat_id: &str) -> Result<(), EngineError> {
        if let Some(workspace) = self.inner.workspace()
            && let Some(chat) = workspace.chat(chat_id)?
            && chat.sidechat_expired(chrono::Utc::now())
        {
            return Err(EngineError::Other(
                "Sidechat expired after 24 hours. Create a new sidechat to continue.".into(),
            ));
        }
        Ok(())
    }

    /// Start (or route) a run for `chat_id`.
    ///
    /// - The user message entry is written to the doc immediately (id = `message_id`).
    /// - A live steerable run receives the prompt as its next turn via the mailbox
    ///   (zeron's persistent-session routing); otherwise any live run is interrupted
    ///   first — never two runtimes driving one chat.
    pub async fn dispatch(
        &self,
        chat_id: &str,
        harness_id: HarnessId,
        request: RunRequest,
        message_id: Option<String>,
    ) -> Result<String, EngineError> {
        self.dispatch_with(chat_id, harness_id, request, message_id, false)
            .await
    }

    /// [`Self::dispatch`] with the startup-crash retry marker: the retry
    /// re-dispatches with `startup_retry = true`, which makes that attempt
    /// final (its own startup death surfaces instead of retrying again).
    /// Boxed future: `drive_run` re-enters this for that retry, and the
    /// erasure breaks the opaque-type cycle the recursion would otherwise form.
    fn dispatch_with<'a>(
        &'a self,
        chat_id: &'a str,
        harness_id: HarnessId,
        request: RunRequest,
        message_id: Option<String>,
        startup_retry: bool,
    ) -> futures::future::BoxFuture<'a, Result<String, EngineError>> {
        Box::pin(self.dispatch_inner(chat_id, harness_id, request, message_id, startup_retry))
    }

    async fn dispatch_inner(
        &self,
        chat_id: &str,
        harness_id: HarnessId,
        mut request: RunRequest,
        mut message_id: Option<String>,
        startup_retry: bool,
    ) -> Result<String, EngineError> {
        self.ensure_sidechat_active(chat_id)?;
        // Project-less chats store cwd `~` (the creating device can't know the
        // host's home); expand it here, on the host, where the run spawns.
        request.cwd = crate::repos::expand_home(&request.cwd)
            .map_err(|error| EngineError::Other(error.to_string()))?;
        // Native-only catalog entries have no portable file fallback. Reject
        // cross-harness delivery before recording or routing the user turn.
        zeron_proto::invocation::validate_harness_invocations(&request.prompt, harness_id)
            .map_err(EngineError::Other)?;
        self.label_computer_chat(chat_id);
        if request.computer_control
            && self
                .inner
                .registry
                .resolve(harness_id)
                .is_ok_and(|harness| !harness.computer_tools_reachable())
        {
            self.note_computer_unreachable(chat_id, harness_id);
            request.computer_control = false;
        }
        let mut turn_computer = request.computer_control;
        let live_computer = lock(&self.inner.runs)
            .get(chat_id)
            .map(|h| h.runtime_config.computer_control);
        if let Some(live) = live_computer
            && live != request.computer_control
            && self
                .inner
                .registry
                .resolve(harness_id)
                .is_ok_and(|harness| harness.resume_supported() == Some(false))
        {
            if request.computer_control {
                self.note_computer_unavailable(chat_id, harness_id);
            }
            turn_computer = false;
            request.computer_control = live;
        }
        let live_reasoning = self
            .inner
            .registry
            .resolve(harness_id)
            .is_ok_and(|harness| harness.live_reasoning());
        let routed = lock(&self.inner.runs).get(chat_id).map(|h| {
            (
                h.run_id.clone(),
                h.steerable,
                h.runtime_config
                    .can_route(harness_id, &request, live_reasoning),
                h.steer_tx.clone(),
                h.routed_steers.clone(),
                h.fork_history_sent.clone(),
            )
        });
        if let Some((run_id, steerable, same_runtime, steer_tx, ledger, history_sent)) = routed {
            let user_id = message_id.clone().unwrap_or_else(new_id);
            let mut bootstrap = None;
            let accepted = if steerable && same_runtime {
                bootstrap =
                    self.warm_fork_history(chat_id, harness_id, &request.prompt, &history_sent);
                let delivered = bootstrap.as_deref().unwrap_or(&request.prompt);
                // Warm dispatch uses the same mailbox as explicit steering.
                // Register acceptance before a fast boundary can retire it.
                let message = SteerMessage {
                    // The mailbox is the harness's ear: a sidechat's prompt is
                    // wrapped here, while the doc entry and the ledger (which a
                    // dying run re-dispatches) keep the typed text. OpenCode
                    // must see the canonical selection before it decodes the
                    // provider command.
                    prompt: {
                        let wrapped = self
                            .inner
                            .harness_prompt(chat_id, delivered, &user_id, turn_computer)
                            .map_or_else(|| delivered.to_owned(), |wrapped| wrapped.prompt);
                        if harness_id == HarnessId::Opencode {
                            wrapped
                        } else {
                            zeron_proto::invocation::harness_prompt(&wrapped, harness_id)
                        }
                    },
                    message_id: Some(user_id.clone()),
                    attachments: request.attachments.clone(),
                    reasoning: request.reasoning,
                };
                if let Ok(permit) = steer_tx.reserve().await {
                    // Commit the reserved slot atomically with the update
                    // marker. An accepted update releases the slot instead, and
                    // the prompt takes the fresh-run path behind the update.
                    self.inner
                        .registry
                        .while_update_clear(harness_id, || {
                            let mut pending = lock(&ledger);
                            pending.push_back(RoutedSteer {
                                prompt: request.prompt.clone(),
                                message_id: user_id.clone(),
                                attachments: request.attachments.clone(),
                                computer_control: turn_computer,
                                fork_history: bootstrap.is_some(),
                            });
                            permit.send(message);
                        })
                        .is_some()
                } else {
                    false
                }
            } else {
                false
            };
            if accepted {
                if let Some(live) = lock(&self.inner.runs)
                    .get_mut(chat_id)
                    .filter(|h| h.run_id == run_id)
                {
                    live.runtime_config.reasoning = request.reasoning;
                }
                let handle = self.doc_handle(chat_id)?;
                handle.write_user_message(&user_id, &request.prompt, now_ms())?;
                self.note_turn_start(chat_id, &request.cwd);
                if self.is_live(chat_id, &run_id) {
                    if bootstrap.is_some() {
                        history_sent.store(true, std::sync::atomic::Ordering::Release);
                    }
                    // Working BEFORE the lastMessageAt bump: both ride the
                    // workspace doc from this one peer, so causal order makes it
                    // impossible for an observer to hold [new message, old status]
                    // — that gap read as unseen-with-no-live-run = a phantom
                    // "completed" flash on every remote send (2026-07-31).
                    self.set_status(chat_id, SessionStatus::Working, false);
                    self.inner.note_message(chat_id, &request.prompt);
                    return Ok(run_id);
                }
                // The run died around the send. If its exit drain already
                // claimed the entry, that re-dispatch owns the message —
                // otherwise reclaim it and fall through to a fresh run.
                let reclaimed = {
                    let mut ledger = lock(&ledger);
                    let before = ledger.len();
                    ledger.retain(|s| s.message_id != user_id);
                    ledger.len() != before
                };
                if !reclaimed {
                    self.inner.note_message(chat_id, &request.prompt);
                    return Ok(run_id);
                }
                // Keep the already-written doc entry's id for the fresh run
                // below (write_user_message dedupes by id).
                message_id = Some(user_id);
            }
            if !same_runtime {
                tracing::debug!(
                    chat = %chat_id,
                    "restarting live harness to apply changed run configuration"
                );
            }
            // Mailbox closed (runtime mid-teardown / non-steering harness) or
            // the routed run died with the message reclaimed, or configuration
            // changed beyond what the text-only mailbox can carry: replace it.
            self.interrupt(chat_id).await?;
        }

        let harness = self.inner.registry.resolve(harness_id)?;
        let handle = self.doc_handle(chat_id)?;
        let user_id = message_id.unwrap_or_else(new_id);
        handle.write_user_message(&user_id, &request.prompt, now_ms())?;

        // Engine-owned resume (zeron sessions.ts:736 — every dispatch read the
        // chat's stored harness session): callers always send `resume: None`;
        // the engine threads the chat's prior harness session back in so a new
        // process (app restart) continues the same harness conversation. The
        // startup-crash retry injects too — a stale id is the harness's
        // problem now (`session/load` falls back to `session/new` internally),
        // and starting the retry fresh silently dropped a good conversation.
        let mut resume_injected = false;
        if request.resume.is_none() {
            request.resume = self.inner.resume_for(chat_id, &request.cwd);
            resume_injected = request.resume.is_some();
        }
        self.inner.inject_branch(chat_id, &mut request);
        lock(&self.inner.last_requests).insert(chat_id.to_string(), request.clone());

        let RunSetup {
            handle: run,
            mut controls,
            engine_rx,
            cancel_rx,
        } = RunSetup::new(
            harness_id,
            &request,
            RunMode::Turn {
                steerable: harness.supports_steering(),
            },
        );
        controls.tool_server =
            self.inner
                .tool_server_for(chat_id, &run.computer.secret, request.computer_control);
        let run_computer = run.computer.clone();
        let fork_history_sent = run.fork_history_sent.clone();
        let run_id = run.run_id.clone();
        lock(&self.inner.runs).insert(chat_id.to_string(), run);
        self.set_status(chat_id, SessionStatus::Working, true);
        // AFTER Working (same causal-order guarantee as the steer path): the
        // lastMessageAt bump must never be observable ahead of the live run.
        self.inner.note_message(chat_id, &request.prompt);

        // Name the chat NOW, off the first prompt — not after the first
        // exchange completes ("called New session for a long time for no
        // reason"; the titler only needs the prompt and skips titled chats;
        // the Done-time call below stays as the retry for a failed
        // generation).
        if let Some(titles) = self.inner.titles.get() {
            titles.maybe_generate(chat_id, harness_id, &request.prompt, &request.cwd);
        }

        tokio::spawn(drive_run(
            self.inner.clone(),
            chat_id.to_string(),
            run_id.clone(),
            harness,
            request,
            handle.writer(),
            controls,
            engine_rx,
            cancel_rx,
            RunResumeState {
                user_message_id: user_id,
                resume_injected,
                startup_retry,
                computer: run_computer,
                turn_computer,
                fork_history_sent,
            },
        ));
        Ok(run_id)
    }

    fn label_computer_chat(&self, chat_id: &str) {
        let Some(computer) = self.inner.computer() else {
            return;
        };
        if let Some(title) = self
            .inner
            .workspace()
            .and_then(|ws| ws.chat(chat_id).ok().flatten())
            .and_then(|chat| chat.title)
        {
            computer.set_chat_label(chat_id, Some(&title));
        }
    }

    fn harness_display_name(&self, harness_id: HarnessId) -> String {
        self.inner
            .registry
            .resolve(harness_id)
            .map(|harness| harness.display_name().to_owned())
            .unwrap_or_else(|_| "This agent".into())
    }

    fn note_computer_unreachable(&self, chat_id: &str, harness_id: HarnessId) {
        let name = self.harness_display_name(harness_id);
        let hint = format!(
            "{name} does not receive Zeron's Computer tools. Switch this chat to Claude, Codex or another agent that does to use Computer."
        );
        self.write_computer_note(
            chat_id,
            "Computer is unavailable for this turn",
            Some(&hint),
        );
    }

    fn note_computer_unavailable(&self, chat_id: &str, harness_id: HarnessId) {
        let name = self.harness_display_name(harness_id);
        let hint = format!(
            "{name} cannot reload its session, so Zeron cannot restart it with the Computer tools without losing the conversation. Start a new chat to use Computer with it."
        );
        self.write_computer_note(
            chat_id,
            "Computer is unavailable for this turn",
            Some(&hint),
        );
    }

    /// A warm send's prompt with the fork's copied history in front, when the
    /// live provider session never received it (the fork's first turns were
    /// native commands) and no earlier send in this runtime carries it
    /// (`sent`). `None` = send the prompt as is.
    fn warm_fork_history(
        &self,
        chat_id: &str,
        harness_id: HarnessId,
        prompt: &str,
        sent: &std::sync::atomic::AtomicBool,
    ) -> Option<String> {
        if sent.load(std::sync::atomic::Ordering::Acquire) {
            return None;
        }
        let session = lock(&self.inner.harness_sessions)
            .get(chat_id)
            .map(|known| known.session_id.clone())
            .filter(|id| !id.is_empty());
        let handle = self.doc_handle(chat_id).ok()?;
        self.inner.fork_history_prompt(
            chat_id,
            handle.doc(),
            harness_id,
            prompt,
            None,
            ProviderSession::Continued(session.as_deref()),
        )
    }

    /// Push a steer prompt into the live run's mailbox. `NotSteerable` when no live
    /// steerable run exists — the caller (command executor) dispatches a new turn.
    pub async fn steer(
        &self,
        chat_id: &str,
        prompt: &str,
        attachments: &[String],
        message_id: Option<String>,
    ) -> Result<SteerOutcome, EngineError> {
        self.steer_with(chat_id, prompt, attachments, message_id, None)
            .await
    }

    /// The text-only mailbox cannot change a run's tool catalog, so a Computer flag mismatch is NotSteerable.
    pub async fn steer_with(
        &self,
        chat_id: &str,
        prompt: &str,
        attachments: &[String],
        message_id: Option<String>,
        computer_control: Option<bool>,
    ) -> Result<SteerOutcome, EngineError> {
        self.steer_at(
            chat_id,
            prompt,
            attachments,
            message_id,
            computer_control,
            now_ms(),
        )
        .await
    }

    pub(crate) async fn steer_at(
        &self,
        chat_id: &str,
        prompt: &str,
        attachments: &[String],
        message_id: Option<String>,
        computer_control: Option<bool>,
        issued_at: i64,
    ) -> Result<SteerOutcome, EngineError> {
        self.ensure_sidechat_active(chat_id)?;
        let target = lock(&self.inner.runs)
            .get(chat_id)
            .filter(|h| h.steerable)
            .filter(|h| {
                computer_control.is_none_or(|flag| flag == h.runtime_config.computer_control)
            })
            .map(|h| {
                (
                    h.run_id.clone(),
                    h.runtime_config.harness_id,
                    h.steer_tx.clone(),
                    h.routed_steers.clone(),
                    h.runtime_config.computer_control,
                    h.fork_history_sent.clone(),
                )
            });
        let Some((run_id, harness_id, steer_tx, ledger, run_computer, history_sent)) = target
        else {
            return Ok(SteerOutcome::NotSteerable);
        };
        zeron_proto::invocation::validate_harness_invocations(prompt, harness_id)
            .map_err(EngineError::Other)?;
        let user_id = message_id.unwrap_or_else(new_id);
        let turn_computer = computer_control.unwrap_or(run_computer);
        let bootstrap = self.warm_fork_history(chat_id, harness_id, prompt, &history_sent);
        let delivered = bootstrap.as_deref().unwrap_or(prompt);
        let message = SteerMessage {
            // Held-queue release and queued sends reach the harness through
            // here without ever entering `dispatch`, so a sidechat's wrap has
            // to happen on this path too.
            prompt: {
                let wrapped = self
                    .inner
                    .harness_prompt(chat_id, delivered, &user_id, turn_computer)
                    .map_or_else(|| delivered.to_string(), |wrapped| wrapped.prompt);
                if harness_id == HarnessId::Opencode {
                    wrapped
                } else {
                    zeron_proto::invocation::harness_prompt(&wrapped, harness_id)
                }
            },
            message_id: Some(user_id.clone()),
            attachments: attachments.to_vec(),
            reasoning: None,
        };
        // Saturation is backpressure, not a dead runtime. Waiting for room
        // preserves the live process and every accepted message in a burst.
        let Ok(permit) = steer_tx.reserve().await else {
            return Ok(SteerOutcome::NotSteerable);
        };
        let accepted = self.inner.registry.while_update_clear(harness_id, || {
            // Serialize mailbox acceptance with confirmation and Done-time
            // inspection: a fast consumer must never outrun its ledger entry.
            // The update marker is checked in the same critical section, so an
            // accepted update releases the reserved slot instead.
            let mut pending = lock(&ledger);
            pending.push_back(RoutedSteer {
                prompt: prompt.to_string(),
                message_id: user_id.clone(),
                attachments: attachments.to_vec(),
                computer_control: turn_computer,
                fork_history: bootstrap.is_some(),
            });
            permit.send(message);
        });
        if accepted.is_none() {
            return Ok(SteerOutcome::DeferredByUpdate);
        }
        let handle = self.doc_handle(chat_id)?;
        handle.write_user_message(&user_id, prompt, issued_at.min(now_ms()))?;
        // A routed steer is a turn too. Fired here (not only on the confirmed
        // path) — a reclaim falls back to dispatch, which just re-snapshots.
        if let Some(request) = self.last_request(chat_id) {
            self.note_turn_start(chat_id, &request.cwd);
        }
        if self.is_live(chat_id, &run_id) {
            if bootstrap.is_some() {
                history_sent.store(true, std::sync::atomic::Ordering::Release);
            }
            self.set_status(chat_id, SessionStatus::Working, false);
            self.inner.note_message(chat_id, prompt);
            return Ok(SteerOutcome::Accepted);
        }
        // The run died around the send. Exit drain claimed the entry → its
        // re-dispatch owns the message; still ours → reclaim and report
        // NotSteerable so the executor falls back to a fresh dispatch
        // (same message id — the doc entry dedupes).
        let reclaimed = {
            let mut ledger = lock(&ledger);
            let before = ledger.len();
            ledger.retain(|s| s.message_id != user_id);
            ledger.len() != before
        };
        if reclaimed {
            return Ok(SteerOutcome::NotSteerable);
        }
        self.inner.note_message(chat_id, prompt);
        Ok(SteerOutcome::Accepted)
    }

    /// Interrupt the live run, if any. The run settles with a synthetic
    /// `Done{interrupted}` and its streaming entry stamped `aborted`; this waits
    /// (bounded) for that settlement so callers observe a consistent doc.
    pub async fn interrupt(&self, chat_id: &str) -> Result<bool, EngineError> {
        let target = lock(&self.inner.runs).get(chat_id).map(|h| {
            h.computer.revoked.store(true, Ordering::Release);
            (
                h.run_id.clone(),
                h.interrupt_token.clone(),
                h.cancel.clone(),
                h.pending_inputs.clone(),
            )
        });
        let Some((run_id, token, cancel, pending)) = target else {
            return Ok(false);
        };
        // Publish cancellation BEFORE waking the harness. Otherwise a fast
        // EOF after token.cancel() can beat this watch notification and be
        // classified as an error instead of an interrupted turn.
        let _ = cancel.send(true);
        // Unpark questions before harness teardown, which can await them.
        let parked: Vec<_> = lock(&pending).drain().map(|(_, tx)| tx).collect();
        for tx in parked {
            let _ = tx.send(Vec::new());
        }
        // Harness-level interrupt (protocol + child teardown) …
        token.cancel();
        // Bounded settle wait (the run task appends Done + stamps `aborted`).
        for _ in 0..500 {
            if !self.is_live(chat_id, &run_id) {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        Ok(true)
    }

    /// Compact the chat's harness-side context in place.
    ///
    /// Not a turn: no user entry, no assistant entry, nothing journaled. On
    /// success exactly one `system` entry is appended and the context meter is
    /// reset to the harness's post-compaction measurement. The compaction
    /// occupies the chat's run slot and reports the chat Working for its whole
    /// duration, so nothing dispatches underneath it and held queue rows stay
    /// held until the next real turn.
    pub async fn compact(
        &self,
        chat_id: &str,
        focus: Option<String>,
    ) -> Result<CompactOutcome, EngineError> {
        {
            let mut pending = lock(&self.inner.compacting);
            if pending.contains(chat_id) {
                return Err(EngineError::Other(
                    "Context compaction is already in progress.".into(),
                ));
            }
            if self.turn_in_flight(chat_id) {
                return Err(EngineError::Other("Chat is busy".into()));
            }
            if lock(&self.inner.compaction_timed_out).contains(chat_id) {
                return Err(EngineError::Other(
                    "Context compaction timed out — send a message before trying again.".into(),
                ));
            }
            pending.insert(chat_id.to_string());
        }
        let mut reservation = CompactionReservation {
            inner: self.inner.clone(),
            chat_id: chat_id.to_string(),
            run_id: None,
        };

        let (harness_id, mut request) = self.compaction_request(chat_id)?;
        let harness = self.inner.registry.resolve(harness_id)?;
        let compaction = harness.compaction();
        let focus = focus
            .map(|focus| focus.trim().to_string())
            .filter(|focus| !focus.is_empty());
        match &compaction {
            Compaction::None => {
                return Err(EngineError::Other(format!(
                    "{} does not support compaction",
                    harness.display_name()
                )));
            }
            Compaction::Native => request.prompt = focus.unwrap_or_default(),
            Compaction::SlashText(command) => {
                request.prompt = match &focus {
                    Some(focus) => format!("{command} {focus}"),
                    None => command.clone(),
                };
            }
        }
        // The same engine-owned resume the normal dispatch path injects: a
        // compaction that started a fresh session would compact nothing.
        request.resume = self.inner.resume_for(chat_id, &request.cwd);
        self.inner.inject_branch(chat_id, &mut request);

        // Retire a parked warm child so the compaction owns the run slot; a
        // turn actually in flight was refused above.
        self.interrupt(chat_id).await?;

        let handle = self.doc_handle(chat_id)?;
        let run_cwd = request.cwd.clone();
        let RunSetup {
            handle: run,
            controls,
            engine_rx: _engine_rx,
            mut cancel_rx,
        } = RunSetup::new(harness_id, &request, RunMode::Compaction);
        let interrupt_token = run.interrupt_token.clone();
        let _cancel_on_drop = interrupt_token.clone().drop_guard();
        {
            let mut runs = lock(&self.inner.runs);
            if runs.contains_key(chat_id) {
                return Err(EngineError::Other("Chat is busy".into()));
            }
            reservation.run_id = Some(run.run_id.clone());
            runs.insert(chat_id.to_string(), run);
        }
        self.set_status(chat_id, SessionStatus::Working, true);

        let started = match &compaction {
            Compaction::Native => harness.compact(request, controls).await,
            _ => harness.run(request, controls).await,
        };
        match started {
            Err(err) => Err(EngineError::Harness(err)),
            Ok(stream) => {
                match tokio::time::timeout(
                    COMPACT_TIMEOUT,
                    drain_compaction(&self.inner, chat_id, &run_cwd, stream, &mut cancel_rx),
                )
                .await
                {
                    Err(_elapsed) => {
                        interrupt_token.cancel();
                        lock(&self.inner.compaction_timed_out).insert(chat_id.to_string());
                        Err(EngineError::Other("Context compaction timed out.".into()))
                    }
                    Ok(drain) => self.settle_compaction(&handle, drain),
                }
            }
        }
    }

    /// Write the compaction's doc effects: one system entry, then the meter
    /// reset to the harness's post-compaction occupancy.
    fn settle_compaction(
        &self,
        handle: &ChatDocHandle,
        drain: CompactionDrain,
    ) -> Result<CompactOutcome, EngineError> {
        let Some((pre_tokens, post_tokens)) = drain.compacted else {
            if let Some(failure) = drain.failure {
                return Err(EngineError::Other(failure));
            }
            return Ok(CompactOutcome {
                compacted: false,
                pre_tokens: None,
                post_tokens: None,
                message: Some(
                    drain
                        .result
                        .map(|result| result.trim().to_string())
                        .filter(|result| !result.is_empty())
                        .unwrap_or_else(|| "Nothing to compact".into()),
                ),
            });
        };
        record_compaction(handle.doc(), &self.inner.device_id, pre_tokens, post_tokens)?;
        Ok(CompactOutcome {
            compacted: true,
            pre_tokens,
            post_tokens,
            message: None,
        })
    }

    /// The run configuration a compaction inherits: the chat's last dispatched
    /// request, else its durable row (the engine may have restarted since the
    /// last turn). Prompt and resume are the caller's to fill in.
    /// Fork `source_chat_id` after the assistant entry `entry_id` into the
    /// client-minted `new_chat_id`: a new row, a doc holding the source's
    /// entries through that reply, the source's harness session remembered
    /// on the fork, and a pending branch cutting it after the reply. The
    /// source is not written.
    pub fn fork_chat(
        &self,
        source_chat_id: &str,
        entry_id: &str,
        new_chat_id: &str,
    ) -> Result<Chat, EngineError> {
        let (host, ws) = self.branch_context()?;
        let harness = self.branching_harness(&host, source_chat_id)?;
        let entries = self.doc_handle(source_chat_id)?.doc().read_entries()?;
        let ix = entries
            .iter()
            .position(|e| e.id == entry_id && e.role == MessageRole::Assistant)
            .ok_or_else(|| EngineError::Other("That reply is no longer in the chat.".into()))?;
        if entries[ix].status == Some(MessageStatus::Streaming) {
            return Err(EngineError::Other("Wait for the reply to finish.".into()));
        }
        let end = entries[ix + 1..]
            .iter()
            .position(|e| e.role == MessageRole::User)
            .map(|rel| ix + 1 + rel)
            .unwrap_or(entries.len());
        let session = self.inner.stored_harness_session(source_chat_id);
        let anchors = self.inner.anchors(source_chat_id);
        if anchors.dead.contains(entry_id) {
            return Err(EngineError::Other(
                "That reply came from a previous provider: fork from a reply after the switch."
                    .into(),
            ));
        }
        let branch = match &session {
            None => None,
            Some(_) => Some(SessionBranch {
                keep: Some(anchors.live.get(entry_id).cloned().ok_or_else(|| {
                    EngineError::Other(format!(
                        "No checkpoint for this reply: it was recorded before {} checkpoints were tracked.",
                        harness.display_name()
                    ))
                })?),
                drop: entries.get(end).and_then(|e| anchors.live.get(&e.id).cloned()),
            }),
        };
        let chat = ws.create_fork_chat(source_chat_id, new_chat_id)?;
        let handle = self.doc_handle(new_chat_id)?;
        if handle.doc().read_entries()?.is_empty() {
            for entry in &entries[..end] {
                handle.doc().push_message(&SessionMessageEntry {
                    fork_import: true,
                    ..entry.clone()
                })?;
            }
        }
        if let Some(text) = entries[..end].iter().rev().find_map(|e| {
            let text = entry_text(e);
            (!text.is_empty()).then_some(text)
        }) {
            self.inner.note_message(new_chat_id, &text);
        }
        if let (Some((session, cwd)), Some(branch)) = (session, branch) {
            self.inner
                .remember_harness_session(new_chat_id, &session, &cwd);
            let forked_at = chat
                .created_at
                .to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
            self.inner
                .set_pending_branch(new_chat_id, branch, Some(forked_at));
        }
        Ok(chat)
    }

    /// Rewind `chat_id` to before the user entry `entry_id`: the doc loses
    /// that entry and everything after it, the context meter clears, and the
    /// next run branches the harness session after the previous reply. The
    /// removed prompt comes back for the composer. A warm child parked
    /// between turns is retired: the branch rides the next run's resume, and
    /// a live mailbox would take the next prompt on the unbranched session.
    pub async fn rewind_chat(
        &self,
        chat_id: &str,
        entry_id: &str,
    ) -> Result<RewindOutcome, EngineError> {
        let (host, _ws) = self.branch_context()?;
        let harness = self.branching_harness(&host, chat_id)?;
        if self.turn_in_flight(chat_id) {
            return Err(EngineError::Other(
                "Stop the running turn before rewinding.".into(),
            ));
        }
        let handle = self.doc_handle(chat_id)?;
        let entries = handle.doc().read_entries()?;
        let ix = entries
            .iter()
            .position(|e| e.id == entry_id && e.role == MessageRole::User)
            .ok_or_else(|| EngineError::Other("That prompt is no longer in the chat.".into()))?;
        let previous_reply = entries[..ix]
            .iter()
            .rev()
            .find(|e| e.role == MessageRole::Assistant);
        let anchors = self.inner.anchors(chat_id);
        // A reply from before a provider switch cannot be branched at; the
        // rewind lands before the switch instead and the next run starts
        // fresh, carrying what is left as text.
        let previous_reply = previous_reply.filter(|reply| !anchors.dead.contains(&reply.id));
        let keep = match previous_reply {
            None => None,
            Some(reply) => Some(anchors.live.get(&reply.id).cloned().ok_or_else(|| {
                EngineError::Other(format!(
                    "No checkpoint before this prompt: it was recorded before {} checkpoints were tracked.",
                    harness.display_name()
                ))
            })?),
        };
        // Before the branch is recorded: the retiring run's Done may still
        // remember a session id, which clears any pending branch.
        self.interrupt(chat_id).await?;
        if lock(&self.inner.runs).contains_key(chat_id) {
            return Err(EngineError::Other(
                "Stop the running turn before rewinding.".into(),
            ));
        }
        let prompt = entry_text(&entries[ix]);
        let removed = handle.doc().truncate_from(entry_id)?;
        let _ = handle.doc().clear_context_usage();
        if let Some(text) = entries[..ix].iter().rev().find_map(|e| {
            let text = entry_text(e);
            (!text.is_empty()).then_some(text)
        }) {
            self.inner.note_message(chat_id, &text);
        }
        match keep {
            Some(keep) => self.inner.set_pending_branch(
                chat_id,
                SessionBranch {
                    keep: Some(keep),
                    drop: anchors.live.get(entry_id).cloned(),
                },
                None,
            ),
            None => {
                let cwd = self
                    .inner
                    .stored_harness_session(chat_id)
                    .map(|(_, cwd)| cwd)
                    .unwrap_or_else(|| self.chat_cwd(chat_id));
                self.inner.tombstone_harness_session(chat_id, &cwd)
            }
        }
        Ok(RewindOutcome { prompt, removed })
    }

    /// The chat is moving to another harness: retire whatever child is
    /// parked or running (its Done would re-remember the old session id)
    /// and tombstone the session, so the next run starts fresh and hands
    /// the conversation over as text. The row config is the caller's write.
    pub async fn switch_harness(&self, chat_id: &str) -> Result<(), EngineError> {
        self.interrupt(chat_id).await?;
        let cwd = self
            .inner
            .stored_harness_session(chat_id)
            .map(|(_, cwd)| cwd)
            .unwrap_or_else(|| self.chat_cwd(chat_id));
        self.inner.tombstone_harness_session(chat_id, &cwd);
        Ok(())
    }

    /// The conversation so far as the packet the next harness receives —
    /// `None` when there is nothing to hand over. Written onto the switch's
    /// divider entry; [`Inner::harness_prompt`] sends that recorded text as
    /// long as no message followed the divider.
    pub fn provider_switch_packet(&self, chat_id: &str) -> Option<String> {
        let entries = self
            .inner
            .doc_host()?
            .open(chat_id)
            .ok()?
            .doc()
            .read_entries()
            .ok()?;
        let history = bounded_source_copy(&entries, SIDECHAT_CONTEXT_BUDGET_CHARS);
        (!history.is_empty()).then(|| provider_switch_context(&history))
    }

    fn branch_context(
        &self,
    ) -> Result<(DocHost, crate::workspace_host::WorkspaceHost), EngineError> {
        let host = self
            .inner
            .doc_host()
            .ok_or_else(|| EngineError::Other("doc host not wired into sessions engine".into()))?;
        let ws = self
            .inner
            .workspace()
            .ok_or_else(|| EngineError::Other("workspace not wired into sessions engine".into()))?;
        Ok((host, ws))
    }

    fn chat_cwd(&self, chat_id: &str) -> String {
        self.inner
            .workspace()
            .and_then(|ws| ws.chat(chat_id).ok().flatten())
            .and_then(|chat| chat.cwd)
            .map(|cwd| crate::repos::expand_home(&cwd).unwrap_or(cwd))
            .unwrap_or_else(|| crate::repos::expand_home("~").unwrap_or_else(|_| "~".into()))
    }

    fn branching_harness(
        &self,
        host: &DocHost,
        chat_id: &str,
    ) -> Result<Arc<dyn Harness>, EngineError> {
        let harness = self.inner.registry.resolve(host.harness_for(chat_id))?;
        if !harness.branching() {
            return Err(EngineError::Other(format!(
                "{} cannot fork or rewind a session.",
                harness.display_name()
            )));
        }
        Ok(harness)
    }

    fn compaction_request(&self, chat_id: &str) -> Result<(HarnessId, RunRequest), EngineError> {
        let host = self
            .inner
            .doc_host()
            .ok_or_else(|| EngineError::Other("doc host not wired into sessions engine".into()))?;
        let mut request = match self.last_request(chat_id) {
            Some(request) => request,
            None => {
                let workspace = self.inner.workspace();
                let config = workspace
                    .as_ref()
                    .and_then(|ws| ws.chat_config(chat_id))
                    .ok_or_else(|| {
                        EngineError::Other("This chat has no run configuration yet.".into())
                    })?;
                let cwd = workspace
                    .as_ref()
                    .and_then(|ws| ws.chat(chat_id).ok().flatten())
                    .and_then(|chat| chat.cwd)
                    .unwrap_or_else(|| "~".into());
                RunRequest {
                    prompt: String::new(),
                    harness: Some(config.harness),
                    model: config.model,
                    reasoning: config.reasoning,
                    model_options: config.model_options,
                    cwd: crate::repos::expand_home(&cwd)
                        .map_err(|error| EngineError::Other(error.to_string()))?,
                    sandbox: config.sandbox,
                    permission_mode: config.permission_mode,
                    auto_approve: false,
                    resume: None,
                    attachments: Vec::new(),
                    worktree: None,
                    mcp: None,
                    branch: None,
                    forked_at: None,
                    computer_control: false,
                }
            }
        };
        let harness_id = host.harness_for_request(chat_id, &request);
        request.attachments.clear();
        // The checkout the chat runs in already exists; cwd points at it.
        request.worktree = None;
        request.branch = None;
        Ok((harness_id, request))
    }

    /// Resolve a pending `request_input` question set. Returns `false` when no such
    /// request is pending (unknown id, or the run already settled).
    pub fn respond_input(
        &self,
        chat_id: &str,
        request_id: &str,
        answers: Vec<UserInputAnswer>,
    ) -> Result<bool, EngineError> {
        let target = lock(&self.inner.runs)
            .get(chat_id)
            .map(|h| (h.pending_inputs.clone(), h.engine_tx.clone()));
        let Some((pending, engine_tx)) = target else {
            return Ok(false);
        };
        let Some(resolver) = lock(&pending).remove(request_id) else {
            return Ok(false);
        };
        let _ = resolver.send(answers.clone());
        let _ = engine_tx.send(AgentEvent::InputResolved {
            request_id: request_id.to_string(),
            answers: Some(answers),
        });
        Ok(true)
    }

    pub fn raise_input(
        &self,
        chat_id: &str,
        request_id: &str,
        questions: Vec<UserInputQuestion>,
    ) -> Result<oneshot::Receiver<Vec<UserInputAnswer>>, String> {
        let runs = lock(&self.inner.runs);
        let run = runs
            .get(chat_id)
            .ok_or_else(|| "This chat has no live run to ask in.".to_owned())?;
        if run.computer.turn().is_none() {
            return Err("The turn that asked has already ended.".into());
        }
        let (tx, rx) = oneshot::channel();
        lock(&run.pending_inputs).insert(request_id.to_owned(), tx);
        run.engine_tx
            .send(AgentEvent::InputRequested {
                request_id: request_id.to_owned(),
                questions,
            })
            .map_err(|_| "The run is ending.".to_owned())?;
        Ok(rx)
    }

    pub async fn ask_user(
        &self,
        chat_id: &str,
        run_secret: &str,
        questions: Vec<UserInputQuestion>,
        wait: std::time::Duration,
    ) -> Result<Vec<UserInputAnswer>, String> {
        self.computer_run(chat_id, run_secret)
            .ok_or_else(|| "This chat has no live run to ask in.".to_owned())?;
        let request_id = new_id();
        let answers = self.raise_input(chat_id, &request_id, questions)?;
        match tokio::time::timeout(wait, answers).await {
            Ok(answers) => Ok(answers.unwrap_or_default()),
            Err(_) => {
                self.settle_input(chat_id, &request_id);
                Ok(Vec::new())
            }
        }
    }

    pub fn settle_input(&self, chat_id: &str, request_id: &str) -> bool {
        let runs = lock(&self.inner.runs);
        let Some(run) = runs.get(chat_id) else {
            return false;
        };
        if lock(&run.pending_inputs).remove(request_id).is_none() {
            return false;
        }
        let _ = run.engine_tx.send(AgentEvent::InputResolved {
            request_id: request_id.to_owned(),
            answers: None,
        });
        true
    }

    pub fn computer_run(&self, chat_id: &str, secret: &str) -> Option<ComputerRun> {
        let runs = lock(&self.inner.runs);
        let run = runs.get(chat_id).filter(|h| h.computer.accepts(secret))?;
        let turn = run.computer.turn();
        Some(ComputerRun {
            run_id: run.run_id.clone(),
            turn_id: turn.as_ref().map(|turn| turn.id.clone()),
            permission_mode: run.runtime_config.permission_mode,
            run_computer_control: run.runtime_config.computer_control,
            turn_computer_control: turn.as_ref().is_some_and(|turn| turn.computer_control),
            turn_desktop_revoked: turn.is_some_and(|turn| turn.desktop_revoked),
        })
    }

    pub fn revoke_computer_turn(&self, chat_id: &str) -> Option<String> {
        lock(&self.inner.runs)
            .get(chat_id)
            .and_then(|run| run.computer.revoke_turn())
    }

    pub fn computer_turn_foreground(
        &self,
        chat_id: &str,
        turn_id: Option<&str>,
    ) -> ComputerTurnForeground {
        lock(&self.inner.runs)
            .get(chat_id)
            .and_then(|run| run.computer.turn())
            .filter(|turn| turn_id.is_none_or(|id| id == turn.id))
            .map(|turn| turn.foreground)
            .unwrap_or_default()
    }

    pub fn answer_computer_foreground(
        &self,
        chat_id: &str,
        turn_id: Option<&str>,
        app: Option<&str>,
        granted: bool,
    ) -> bool {
        lock(&self.inner.runs)
            .get(chat_id)
            .is_some_and(|run| run.computer.answer_foreground(turn_id, app, granted))
    }

    pub fn computer_turn_live(&self, chat_id: &str) -> bool {
        lock(&self.inner.runs)
            .get(chat_id)
            .and_then(|run| run.computer.turn())
            .is_some_and(|turn| turn.computer_control)
    }

    pub fn computer_caller(
        &self,
        chat_id: &str,
        turn_id: Option<&str>,
    ) -> Result<Option<PermissionMode>, String> {
        if let Some(workspace) = self.inner.workspace() {
            match workspace.chat(chat_id) {
                Ok(Some(chat)) if !chat.archived => {}
                _ => return Ok(None),
            }
        }
        let runs = lock(&self.inner.runs);
        let run = runs
            .get(chat_id)
            .ok_or_else(|| "The turn that made this call has ended.".to_owned())?;
        match run.computer.turn() {
            Some(turn) if turn_id.is_none_or(|id| id == turn.id) => {
                Ok(Some(run.runtime_config.permission_mode))
            }
            _ => Err("The turn that made this call has ended.".into()),
        }
    }

    pub fn computer_visible_use_entries(
        &self,
        chat_id: &str,
    ) -> Result<Vec<crate::computer::visible_use::VisibleUseEntry>, String> {
        let entries = self
            .doc_handle(chat_id)
            .and_then(|handle| Ok(handle.doc().read_entries()?))
            .map_err(|err| err.to_string())?;
        Ok(crate::computer::visible_use::visible_use_entries(&entries))
    }

    pub fn computer_chat_label(&self, chat_id: &str) -> Option<String> {
        self.inner
            .workspace()?
            .chat(chat_id)
            .ok()
            .flatten()
            .and_then(|chat| chat.title)
            .filter(|title| !title.trim().is_empty())
    }

    pub fn write_computer_note(&self, chat_id: &str, text: &str, hint: Option<&str>) {
        let written = self
            .doc_handle(chat_id)
            .and_then(|handle| Ok(handle.write_system_note(&new_id(), text, hint, now_ms())?));
        if let Err(err) = written {
            tracing::warn!(chat = %chat_id, error = %err, "computer note write failed");
        }
    }

    pub fn record_computer_generation(&self, chat_id: &str, generation: u64) {
        let written = self
            .doc_handle(chat_id)
            .and_then(|handle| Ok(handle.doc().set_computer_generation(generation)?));
        if let Err(err) = written {
            tracing::warn!(chat = %chat_id, error = %err, "computer generation write failed");
        }
    }

    pub fn live_computer_control(&self, chat_id: &str) -> Option<bool> {
        lock(&self.inner.runs)
            .get(chat_id)
            .map(|h| h.runtime_config.computer_control)
    }

    /// Boot recovery: for every journal whose last event is not `Done` (a run died
    /// mid-stream), stamp this device's abandoned `streaming` doc entries `aborted`
    /// with a VISIBLE "Run interrupted by engine restart" error part, close the
    /// journal with a synthetic `Done{interrupted}` — and then PICK THE RUN BACK
    /// UP: a fresh crashed turn with revival budget left is re-dispatched against
    /// the remembered harness session (zeron: "not just eulogized";
    /// `MAX_AUTO_RESUME` = 3 consecutive revivals, fresh = crashed < 12h ago).
    pub fn recover_stale(&self) -> Result<usize, EngineError> {
        const MAX_AUTO_RESUME: u32 = 3;
        const RESUME_FRESH_MS: i64 = 12 * 60 * 60 * 1000;

        let stale = self.inner.journal.stale_sessions()?;
        let mut recovered = 0usize;
        for chat_id in stale {
            if lock(&self.inner.runs).contains_key(&chat_id) {
                continue; // a live run owns this journal
            }
            let handle = self.doc_handle(&chat_id)?;
            // Harness continuity first: the crashed run's session id may only
            // exist in the journal (the debounced workspace-row write may
            // never have landed) — remember it so the revived run resumes the
            // same harness conversation (zeron recoverDraft, sessions.ts:538).
            if let Some((session_id, cwd)) = self.inner.journal_harness_session(&chat_id) {
                self.inner
                    .cache_harness_session(&chat_id, &session_id, &cwd);
            }
            // The revival prompt: the last user message (idempotent re-dispatch
            // under the SAME id — `write_user_message` dedupes by id, so the
            // transcript never shows a duplicate).
            let prompt = handle.doc().read_entries().ok().and_then(|entries| {
                entries
                    .iter()
                    .rev()
                    .find(|e| e.role == MessageRole::User)
                    .and_then(|e| {
                        e.parts.iter().find_map(|p| match p {
                            MessagePart::Text { text, .. } => {
                                Some((e.id.clone(), text.clone(), e.from_chat_id.is_none()))
                            }
                            _ => None,
                        })
                    })
            });
            let attempts = self.inner.journal.resume_attempts(&chat_id);
            let fresh = handle
                .doc()
                .read_entries()
                .ok()
                .and_then(|entries| {
                    entries
                        .iter()
                        .rev()
                        .find(|e| e.status == Some(MessageStatus::Streaming))
                        .map(|e| now_ms() - e.created_at < RESUME_FRESH_MS)
                })
                .unwrap_or(false);
            let will_resume = fresh && prompt.is_some() && attempts < MAX_AUTO_RESUME;

            let note = if will_resume {
                "Run interrupted by engine restart — resuming"
            } else {
                "Run interrupted by engine restart"
            };
            let done = AgentEvent::Done {
                status: DoneStatus::Interrupted,
                result: None,
                error: Some(note.into()),
                session_id: None,
            };
            self.inner.publish(&chat_id, &done);
            let stamped = handle.mark_abandoned_streams(note)?.len();
            self.set_status(&chat_id, SessionStatus::Idle, false);
            tracing::info!(chat = %chat_id, stamped, will_resume, attempts, "recovered stale session journal");
            recovered += 1;

            if !will_resume {
                continue;
            }
            let attempt = self.inner.journal.note_resume_attempt(&chat_id);
            let (user_id, prompt_text, user_authored) = prompt.expect("gated by will_resume");
            let sessions = self.clone();
            tokio::spawn(async move {
                let Some(host) = sessions.inner.doc_host() else {
                    return;
                };
                let request = sessions
                    .last_request(&chat_id)
                    .or_else(|| host.request_from_chat_row(&chat_id, &prompt_text))
                    // Last resort: the journal's own cwd (zeron's draft config)
                    // — a crash can predate the debounced workspace-row write.
                    .or_else(|| {
                        let (_, cwd) = sessions.inner.journal_harness_session(&chat_id)?;
                        Some(RunRequest {
                            mcp: None,
                            prompt: String::new(),
                            harness: None,
                            model: None,
                            reasoning: None,
                            model_options: Default::default(),
                            cwd,
                            sandbox: zeron_proto::SandboxLevel::WorkspaceWrite,
                            permission_mode: zeron_proto::PermissionMode::Full,
                            auto_approve: false,
                            attachments: Vec::new(),
                            resume: None,
                            worktree: None,
                            branch: None,
                            forked_at: None,
                            computer_control: false,
                        })
                    });
                let Some(mut request) = request else {
                    tracing::warn!(chat = %chat_id, "auto-resume skipped: no run config");
                    return;
                };
                request.computer_control = false;
                if let (Some(computer), Ok(handle)) = (sessions.computer(), host.open(&chat_id)) {
                    request.computer_control = computer.resolve_computer_control(
                        &chat_id,
                        handle.doc().computer_generation(),
                        &prompt_text,
                        user_authored,
                        dispatched_generation(handle.doc(), &user_id),
                    );
                }
                request.prompt = prompt_text;
                request.resume = None; // dispatch re-injects the remembered session
                request.attachments = Vec::new();
                let harness_id = host.harness_for_request(&chat_id, &request);
                match host
                    .dispatch_with_source_context(
                        &sessions,
                        &chat_id,
                        harness_id,
                        request,
                        Some(user_id),
                    )
                    .await
                {
                    Ok(_) => {
                        tracing::info!(chat = %chat_id, attempt, "auto-resumed crashed run")
                    }
                    Err(err) => {
                        tracing::warn!(chat = %chat_id, error = %err, "auto-resume dispatch failed")
                    }
                }
            });
        }
        Ok(recovered)
    }

    /// Graceful shutdown: interrupt every live run so streaming entries settle.
    pub async fn shutdown(&self) {
        let chats: Vec<String> = lock(&self.inner.runs).keys().cloned().collect();
        for chat_id in chats {
            if let Err(err) = self.interrupt(&chat_id).await {
                tracing::warn!(chat = %chat_id, error = %err, "shutdown interrupt failed");
            }
        }
    }

    fn is_live(&self, chat_id: &str, run_id: &str) -> bool {
        lock(&self.inner.runs)
            .get(chat_id)
            .is_some_and(|h| h.run_id == run_id)
    }

    fn set_status(&self, chat_id: &str, status: SessionStatus, fresh_start: bool) {
        self.inner.set_status(chat_id, status, fresh_start);
    }
}

/// What a rewind hands back: the removed prompt, for the composer.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RewindOutcome {
    pub prompt: String,
    pub removed: usize,
}

/// An entry's prose, text parts only.
fn entry_text(entry: &SessionMessageEntry) -> String {
    entry
        .parts
        .iter()
        .filter_map(|part| match part {
            MessagePart::Text { text, .. } => Some(text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n\n")
        .trim()
        .to_string()
}

const COMPUTER_USE_EMPTY_TASK: &str = "Use Zeron Computer for this task.";

/// Ported verbatim from astra `sidechatProviderInput.ts:3-4`.
const SIDECHAT_BOUNDARY_INSTRUCTION: &str = "You are in a sidechat. Treat all prior conversation as reference-only context. Do not continue any prior task automatically. Do not mutate files, git, or the workspace and do not run workspace-changing commands unless the latest user message explicitly asks you to do so after this boundary. Use this sidechat for focused explanation, safety checks, summaries, and alternatives.";

/// How much of the source transcript a sidechat's first turn carries.
const SIDECHAT_CONTEXT_BUDGET_CHARS: usize = 48_000;

fn sidechat_prompt(typed: &str, source_copy: &str) -> String {
    let mut prompt =
        format!("<sidechat_boundary>\n{SIDECHAT_BOUNDARY_INSTRUCTION}\n</sidechat_boundary>\n\n");
    if !source_copy.is_empty() {
        prompt.push_str(&format!(
            "<sidechat_context>\n{source_copy}\n</sidechat_context>\n\n"
        ));
    }
    prompt.push_str(&format!(
        "<latest_user_message>\n{typed}\n</latest_user_message>"
    ));
    prompt
}

/// The source chat's prose — user and assistant text only — walked newest-first
/// until `budget` characters are spent, then joined oldest-first. Tool,
/// reasoning and error parts are not conversation, and a streaming entry is
/// half-written; the oldest kept message is truncated from its start so the
/// text nearest the sidechat's question always survives.
fn bounded_source_copy(entries: &[SessionMessageEntry], budget: usize) -> String {
    let mut selected: Vec<String> = Vec::new();
    let mut remaining = budget;
    for entry in entries.iter().rev() {
        if remaining == 0 {
            break;
        }
        if entry.status == Some(MessageStatus::Streaming) {
            continue;
        }
        let role = match entry.role {
            MessageRole::User => "User",
            MessageRole::Assistant => "Assistant",
            MessageRole::System => continue,
        };
        let text: String = entry
            .parts
            .iter()
            .filter_map(|part| match part {
                MessagePart::Text { text, .. } => Some(text.as_str()),
                _ => None,
            })
            .collect();
        let text = text.trim();
        if text.is_empty() {
            continue;
        }
        let separator = if selected.is_empty() { 0 } else { 2 };
        let retained = last_chars(&format!("{role}: {text}"), remaining);
        remaining = remaining.saturating_sub(retained.chars().count() + separator);
        selected.push(retained);
    }
    selected.reverse();
    selected.join("\n\n")
}

/// The last `max` characters of `text` — whole scalar values, because a byte
/// slice would panic mid-codepoint.
fn last_chars(text: &str, max: usize) -> String {
    let count = text.chars().count();
    if count <= max {
        return text.to_string();
    }
    text.chars().skip(count - max).collect()
}

/// The chat's harness anchors: `live` ones a branch can cut at, `dead` ones
/// recorded for a session that was tombstoned since (a provider switch).
#[derive(Default)]
struct Anchors {
    live: HashMap<String, String>,
    dead: std::collections::HashSet<String>,
}

/// What a harness receives ahead of the typed message on the first turn after
/// a provider switch: the conversation so far as text. Recorded on the user
/// entry verbatim, so the transcript shows exactly what crossed.
fn provider_switch_context(history: &str) -> String {
    format!(
        "<provider_switch>\nThis chat was moved to you from another agent. Its conversation so far is below as plain text; that agent's tool results and file state were not carried over. Continue from where it left off.\n</provider_switch>\n\n<previous_conversation>\n{history}\n</previous_conversation>"
    )
}

/// What crosses into the harness in place of the typed text, and the
/// provider-switch packet inside it when this is that first turn.
struct HarnessPrompt {
    prompt: String,
    handoff: Option<String>,
}

#[derive(Default)]
struct SidechatContext {
    completed_assistant: Option<usize>,
    answer_version: Option<loro::VersionVector>,
    source: Option<(String, loro::VersionVector, String)>,
}

impl Inner {
    /// What crosses into the harness in place of the typed text: a sidechat's
    /// wrap, else — on a chat whose session was tombstoned under a
    /// conversation — that conversation as text (the harness that produced it
    /// is gone, and the new one has never seen it). `None` leaves the typed
    /// text as is. `user_entry_id` is the entry this prompt writes; it is not
    /// part of the history.
    fn harness_prompt(
        &self,
        chat_id: &str,
        typed: &str,
        user_entry_id: &str,
        computer: bool,
    ) -> Option<HarnessPrompt> {
        let invoked = computer
            .then(|| crate::computer::activation::parse_computer_invocation(typed))
            .flatten()
            .map(|task| {
                if task.is_empty() {
                    COMPUTER_USE_EMPTY_TASK.to_owned()
                } else {
                    task
                }
            });
        let stripped = invoked.clone().map(|prompt| HarnessPrompt {
            prompt,
            handoff: None,
        });
        let typed = invoked.as_deref().unwrap_or(typed);
        if let Some(prompt) = self.sidechat_harness_prompt(chat_id, typed) {
            return Some(HarnessPrompt {
                prompt,
                handoff: None,
            });
        }
        if !self.session_tombstoned(chat_id) {
            return stripped;
        }
        self.provider_switch_prompt(chat_id, typed, user_entry_id)
            .or(stripped)
    }

    fn provider_switch_prompt(
        &self,
        chat_id: &str,
        typed: &str,
        user_entry_id: &str,
    ) -> Option<HarnessPrompt> {
        let entries = self
            .doc_host()?
            .open(chat_id)
            .ok()?
            .doc()
            .read_entries()
            .ok()?;
        let history: Vec<SessionMessageEntry> = entries
            .into_iter()
            .filter(|entry| entry.id != user_entry_id)
            .collect();
        // The switch's divider already recorded the packet; send exactly
        // that while it is still the newest thing said. Anything after it
        // (a first turn that died and was re-sent) needs a fresh copy, which
        // then lands on the user entry instead.
        let recorded = history
            .last()
            .filter(|entry| entry.role == MessageRole::System)
            .and_then(|entry| {
                entry.parts.iter().find_map(|part| match part {
                    MessagePart::Handoff { text, .. } => Some(text.clone()),
                    _ => None,
                })
            });
        let (context, handoff) = match recorded {
            Some(context) => (context, None),
            None => {
                let history = bounded_source_copy(&history, SIDECHAT_CONTEXT_BUDGET_CHARS);
                if history.is_empty() {
                    return None;
                }
                let context = provider_switch_context(&history);
                (context.clone(), Some(context))
            }
        };
        Some(HarnessPrompt {
            prompt: format!("{context}\n\n<latest_user_message>\n{typed}\n</latest_user_message>"),
            handoff,
        })
    }

    /// What a sidechat's harness receives in place of the typed text: the
    /// boundary instruction, the source transcript on the first turn, and the
    /// typed message. `None` for an ordinary chat, which is every chat whose
    /// row names no source.
    fn sidechat_harness_prompt(&self, chat_id: &str, typed: &str) -> Option<String> {
        let source_id = self
            .workspace()?
            .chat(chat_id)
            .ok()
            .flatten()?
            .sidechat_source_chat_id?;
        let host = self.doc_host()?;
        let handle = host.open(chat_id).ok()?;
        let mut contexts = lock(&self.sidechat_contexts);
        let context = contexts.entry(chat_id.to_string()).or_default();
        let version = handle.doc().doc().oplog_vv();
        if context.answer_version.as_ref() != Some(&version) {
            context.completed_assistant = handle
                .doc()
                .completed_assistant_index(context.completed_assistant);
            context.answer_version = Some(version);
        }
        if context.completed_assistant.is_some() {
            context.source = None;
            return Some(sidechat_prompt(typed, ""));
        }
        // An interrupted first turn still carries source context. Refresh the
        // bounded copy only when its source changes; never cache an open error.
        let Ok(source) = host.open(&source_id) else {
            return Some(sidechat_prompt(typed, ""));
        };
        let source_version = source.doc().doc().oplog_vv();
        if context
            .source
            .as_ref()
            .is_none_or(|(id, version, _)| id != &source_id || version != &source_version)
        {
            let copy = source
                .doc()
                .read_entries()
                .ok()
                .map(|entries| bounded_source_copy(&entries, SIDECHAT_CONTEXT_BUDGET_CHARS));
            let Some(copy) = copy else {
                return Some(sidechat_prompt(typed, ""));
            };
            context.source = Some((source_id, source_version, copy));
        }
        Some(sidechat_prompt(typed, &context.source.as_ref()?.2))
    }

    /// Journal + broadcast one event (the two unconditional legs of the pipeline).
    /// Sanitize before ANY journal/publish path, including nested subagent events.
    async fn prepare_generated_image(
        &self,
        chat_id: &str,
        event: AgentEvent,
        seen: &mut std::collections::HashSet<String>,
    ) -> Vec<AgentEvent> {
        let mut parents = Vec::new();
        let mut leaf = event;
        while let AgentEvent::Subagent {
            parent_tool_use_id,
            event,
        } = leaf
        {
            parents.push(parent_tool_use_id);
            leaf = *event;
        }
        let events = if let AgentEvent::GeneratedImage { id, path, .. } = leaf {
            let key = std::iter::once(chat_id)
                .chain(parents.iter().map(String::as_str))
                .chain(std::iter::once(id.as_str()))
                .collect::<Vec<_>>()
                .join("\0");
            if seen.contains(&key) {
                return vec![];
            }
            let result = if let Some((uploads, allowed_root)) = self.generated_images.get() {
                let uploads = uploads.clone();
                let root = allowed_root.clone();
                let stable_key = key.clone();
                tokio::task::spawn_blocking(move || {
                    uploads.import_generated_image(std::path::Path::new(&path), &root, &stable_key)
                })
                .await
                .unwrap_or_else(|err| Err(EngineError::Other(err.to_string())))
            } else {
                Err(EngineError::Other(
                    "Generated image intake is not configured".into(),
                ))
            };
            match result {
                Ok(image) => {
                    seen.insert(key);
                    vec![AgentEvent::GeneratedImage {
                        id,
                        path: image.path,
                        name: image.name,
                        mime_type: image.mime_type,
                    }]
                }
                Err(err) => {
                    tracing::warn!(chat = %chat_id, error = %err, "generated image import failed");
                    vec![
                        AgentEvent::ToolResult {
                            id: id.strip_suffix(":image").unwrap_or(&id).into(),
                            is_error: true,
                            output: None,
                            diff: None,
                        },
                        AgentEvent::Error {
                            message: "Generated image unavailable".into(),
                        },
                    ]
                }
            }
        } else {
            vec![leaf]
        };
        events
            .into_iter()
            .map(|mut event| {
                for parent in parents.iter().rev() {
                    event = AgentEvent::Subagent {
                        parent_tool_use_id: parent.clone(),
                        event: Box::new(event),
                    };
                }
                event
            })
            .collect()
    }

    fn publish(&self, chat_id: &str, event: &AgentEvent) -> u64 {
        let seq = match self.journal.append(chat_id, event) {
            Ok(seq) => seq,
            Err(err) => {
                tracing::error!(chat = %chat_id, error = %err, "journal append failed");
                0
            }
        };
        if let Some(hub) = lock(&self.hubs).get(chat_id) {
            let _ = hub.send(JournaledEvent {
                seq,
                event: event.clone(),
            });
        }
        seq
    }

    /// Bump the session's freshness on stream activity WITHOUT a status
    /// transition. Long silent-LOOKING stretches (thinking heartbeats, a big
    /// tool input being generated) still carry events — the UI's 45s
    /// staleness gate must not flip "Working" off mid-run. Throttled: a
    /// workspace-doc mirror per delta would be far too chatty.
    fn touch_session(&self, chat_id: &str) {
        const TOUCH_THROTTLE_MS: i64 = 10_000;
        let now = Utc::now();
        let session = {
            let mut statuses = lock(&self.statuses);
            let Some(entry) = statuses.get_mut(chat_id) else {
                return;
            };
            let age = now
                .signed_duration_since(entry.updated_at)
                .num_milliseconds();
            if age < TOUCH_THROTTLE_MS {
                return;
            }
            entry.updated_at = now;
            let session = entry.clone();
            let mut list: Vec<Session> = statuses.values().cloned().collect();
            list.sort_by(|a, b| a.chat_id.cmp(&b.chat_id));
            self.sessions_tx.send_replace(list);
            session
        };
        if let Some(ws) = self.workspace() {
            ws.record_session(&session);
        }
    }

    fn set_status(&self, chat_id: &str, status: SessionStatus, fresh_start: bool) {
        self.set_status_with_completion(chat_id, status, fresh_start, None, false);
    }

    fn has_pending_steers(&self, chat_id: &str, run_id: &str) -> bool {
        lock(&self.runs)
            .get(chat_id)
            .filter(|h| h.run_id == run_id)
            .is_some_and(|h| !lock(&h.routed_steers).is_empty())
    }

    /// Publish the completion marker atomically with the settled status. Keep
    /// it on subsequent Working/heartbeat rows for coalesced and remote watches.
    /// `held_by_subagents`: the status is a parked turn's, Working only because
    /// background subagents are still streaming (see [`parked_status`]).
    fn set_status_with_completion(
        &self,
        chat_id: &str,
        status: SessionStatus,
        fresh_start: bool,
        completed_turn: Option<String>,
        held_by_subagents: bool,
    ) {
        let now = Utc::now();
        let session = {
            let mut statuses = lock(&self.statuses);
            let entry = statuses
                .entry(chat_id.to_string())
                .or_insert_with(|| Session {
                    last_completed_turn: None,
                    chat_id: chat_id.to_string(),
                    device_id: self.device_id.clone(),
                    status,
                    held_by_subagents,
                    compacting: false,
                    background_tasks: Vec::new(),
                    started_at: None,
                    updated_at: now,
                });
            // `started_at` is the elapsed-timer base and must only ever mean
            // "this turn". Entering Working from a settled state always
            // restamps (a steer into a parked session is a NEW turn — reusing
            // the old base showed the previous turn's 30min on send), and a
            // settled session drops its base entirely so no later reader can
            // resurrect a stale elapsed.
            let was_active = is_active(entry);
            if let Some(turn) = completed_turn {
                entry.last_completed_turn = Some(turn);
                // A completed turn is the retry gate a timed-out compaction waits on.
                lock(&self.compaction_timed_out).remove(chat_id);
            }
            entry.status = status;
            entry.held_by_subagents = held_by_subagents;
            entry.updated_at = now;
            match status {
                SessionStatus::Working if fresh_start || !was_active => {
                    entry.started_at = Some(now);
                }
                SessionStatus::Working | SessionStatus::AwaitingInput => {}
                SessionStatus::Idle | SessionStatus::Errored => {
                    entry.started_at = None;
                    entry.compacting = false;
                }
            }
            let session = entry.clone();
            let mut list: Vec<Session> = statuses.values().cloned().collect();
            list.sort_by(|a, b| a.chat_id.cmp(&b.chat_id));
            // send_replace: keep the current value fresh even with no receivers,
            // so late WatchSessions subscribers see the last transition.
            self.sessions_tx.send_replace(list);
            session
        };
        // Mirror the transition into the workspace doc's session-status row so
        // remote devices' sidebars show this run (staleness-checked client-side).
        if let Some(ws) = self.workspace() {
            ws.record_session(&session);
        }
    }

    fn set_compacting(&self, chat_id: &str, compacting: bool) {
        let session = {
            let mut statuses = lock(&self.statuses);
            let Some(entry) = statuses.get_mut(chat_id) else {
                return;
            };
            if entry.compacting == compacting {
                return;
            }
            entry.compacting = compacting;
            entry.updated_at = Utc::now();
            let session = entry.clone();
            let mut list: Vec<Session> = statuses.values().cloned().collect();
            list.sort_by(|a, b| a.chat_id.cmp(&b.chat_id));
            self.sessions_tx.send_replace(list);
            session
        };
        if let Some(ws) = self.workspace() {
            ws.record_session(&session);
        }
    }

    fn set_background_tasks(&self, chat_id: &str, tasks: Vec<zeron_proto::BackgroundTask>) {
        let session = {
            let mut statuses = lock(&self.statuses);
            let Some(entry) = statuses.get_mut(chat_id) else {
                return;
            };
            let now = Utc::now();
            let tasks: Vec<_> = tasks
                .into_iter()
                .map(|mut task| {
                    task.started_at = entry
                        .background_tasks
                        .iter()
                        .find(|known| known.id == task.id)
                        .and_then(|known| known.started_at)
                        .or(Some(now));
                    task
                })
                .collect();
            if entry.background_tasks == tasks {
                return;
            }
            entry.background_tasks = tasks;
            entry.updated_at = now;
            let session = entry.clone();
            let mut list: Vec<Session> = statuses.values().cloned().collect();
            list.sort_by(|a, b| a.chat_id.cmp(&b.chat_id));
            self.sessions_tx.send_replace(list);
            session
        };
        if let Some(ws) = self.workspace() {
            ws.record_session(&session);
        }
    }

    /// The doc host, once wired. `None` before assembly or after retirement.
    fn doc_host(&self) -> Option<DocHost> {
        lock(&self.doc_host).clone()
    }

    fn workspace(&self) -> Option<crate::workspace_host::WorkspaceHost> {
        self.doc_host().and_then(|host| host.workspace().cloned())
    }

    /// Sidebar freshness: push a message-persist preview into the chat's workspace row.
    fn note_message(&self, chat_id: &str, text: &str) {
        if text.is_empty() {
            return;
        }
        if let Some(ws) = self.workspace() {
            ws.note_message(chat_id, text);
        }
    }

    /// Record the chat's harness-native session id (and its cwd): live-process
    /// cache plus the durable workspace chat row — the row is what survives an
    /// engine restart (zeron sessions.ts:1039).
    fn remember_harness_session(&self, chat_id: &str, session_id: &str, cwd: &str) {
        if session_id.is_empty() {
            return;
        }
        lock(&self.pending_branches).insert(chat_id.to_string(), None);
        self.cache_harness_session(chat_id, session_id, cwd);
    }

    /// Recovering an older session does not consume a later pending branch.
    fn cache_harness_session(&self, chat_id: &str, session_id: &str, cwd: &str) {
        lock(&self.harness_sessions).insert(
            chat_id.to_string(),
            HarnessSessionRef {
                session_id: session_id.to_string(),
                cwd: cwd.to_string(),
            },
        );
        if let Some(ws) = self.workspace() {
            ws.set_chat_harness_session(chat_id, session_id, cwd);
        }
    }

    // NB: there is deliberately no `forget_harness_session` anymore. The old
    // tombstone fired on "run died before SessionStarted", which — since the
    // ACP conversion made stale ids a harness-internal fallback — only ever
    // meant a child STARTUP failure, and permanently severed good
    // conversations (user incident 2026-08-13). A truly stale id simply
    // yields a fresh session whose SessionStarted overwrites the row.

    /// The session id to resume for a run in `chat_id` launching from `cwd`
    /// (zeron sessions.ts:736, looked up on every dispatch):
    /// live-process cache → workspace chat row → journal scan (the crash path
    /// where the debounced row write never landed — SessionStarted/Done events
    /// are journaled per event, flushed immediately). Cwd-gated throughout:
    /// harness session stores are keyed by cwd, so a session created elsewhere
    /// never rides `--resume`. An empty stored id is the explicit tombstone —
    /// no resume, no falling through to staler sources.
    fn resume_for(&self, chat_id: &str, cwd: &str) -> Option<String> {
        let cwd_ok = |session_cwd: &str| session_cwd.is_empty() || session_cwd == cwd;
        if let Some(known) = lock(&self.harness_sessions).get(chat_id).cloned() {
            return (!known.session_id.is_empty() && cwd_ok(&known.cwd))
                .then_some(known.session_id);
        }
        if let Some(ws) = self.workspace()
            && let Some((session_id, session_cwd)) = ws.chat_harness_session(chat_id)
        {
            return (!session_id.is_empty() && cwd_ok(session_cwd.as_deref().unwrap_or("")))
                .then_some(session_id);
        }
        let (session_id, session_cwd) = self.journal_harness_session(chat_id)?;
        // Cache the journal hit (memory + row) so later dispatches skip the scan.
        self.cache_harness_session(chat_id, &session_id, &session_cwd);
        cwd_ok(&session_cwd).then_some(session_id)
    }

    /// The chat's stored harness session and the cwd it was created under,
    /// ungated: a branch resumes the session where it lives, whatever the
    /// row's cwd says today. `None` for a tombstone or a chat that never ran.
    fn stored_harness_session(&self, chat_id: &str) -> Option<(String, String)> {
        if let Some(known) = lock(&self.harness_sessions).get(chat_id).cloned() {
            return (!known.session_id.is_empty()).then_some((known.session_id, known.cwd));
        }
        if let Some(ws) = self.workspace()
            && let Some((session_id, cwd)) = ws.chat_harness_session(chat_id)
        {
            return (!session_id.is_empty()).then_some((session_id, cwd.unwrap_or_default()));
        }
        self.journal_harness_session(chat_id)
    }

    /// Copy the chat's pending branch onto a request that resumes; a request
    /// starting fresh has nothing to cut.
    fn inject_branch(&self, chat_id: &str, request: &mut RunRequest) {
        if request.resume.is_none() {
            return;
        }
        if request.branch.is_none() {
            request.branch = self.pending_branch(chat_id);
        }
        request.forked_at = self.forked_at(chat_id);
    }

    fn forked_at(&self, chat_id: &str) -> Option<String> {
        let mut forked = lock(&self.forked_at);
        if let Some(at) = forked.get(chat_id) {
            return at.clone();
        }
        let at = self
            .journal
            .replay(chat_id, 0)
            .ok()?
            .into_iter()
            .filter_map(|(_, event)| match event {
                AgentEvent::BranchPending { forked_at, .. } => forked_at,
                _ => None,
            })
            .last();
        forked.insert(chat_id.to_string(), at.clone());
        at
    }

    /// The branch the chat's next run must apply: the live map, else the
    /// journal (a `BranchPending` newer than the last `SessionStarted`).
    fn pending_branch(&self, chat_id: &str) -> Option<SessionBranch> {
        let mut branches = lock(&self.pending_branches);
        if let Some(branch) = branches.get(chat_id) {
            return branch.clone();
        }
        let events = self.journal.replay(chat_id, 0).ok()?;
        let mut pending = None;
        for (_, event) in events {
            match event {
                AgentEvent::BranchPending { keep, drop, .. } => {
                    pending = Some(SessionBranch { keep, drop })
                }
                AgentEvent::SessionStarted { .. } => pending = None,
                _ => {}
            }
        }
        branches.insert(chat_id.to_string(), pending.clone());
        pending
    }

    fn set_pending_branch(&self, chat_id: &str, branch: SessionBranch, forked_at: Option<String>) {
        lock(&self.pending_branches).insert(chat_id.to_string(), Some(branch.clone()));
        if forked_at.is_some() {
            lock(&self.forked_at).insert(chat_id.to_string(), forked_at.clone());
        }
        self.publish(
            chat_id,
            &AgentEvent::BranchPending {
                keep: branch.keep,
                drop: branch.drop,
                forked_at,
            },
        );
    }

    /// Explicit "do not resume": the next run starts a fresh harness session.
    fn tombstone_harness_session(&self, chat_id: &str, cwd: &str) {
        lock(&self.pending_branches).insert(chat_id.to_string(), None);
        lock(&self.harness_sessions).insert(
            chat_id.to_string(),
            HarnessSessionRef {
                session_id: String::new(),
                cwd: cwd.to_string(),
            },
        );
        if let Some(ws) = self.workspace() {
            ws.set_chat_harness_session(chat_id, "", cwd);
        }
        self.publish(chat_id, &AgentEvent::SessionTombstoned {});
    }

    /// Whether the chat's stored session is the explicit tombstone: the next
    /// run starts fresh even though the doc may hold a conversation.
    fn session_tombstoned(&self, chat_id: &str) -> bool {
        if let Some(known) = lock(&self.harness_sessions).get(chat_id) {
            return known.session_id.is_empty();
        }
        self.workspace()
            .and_then(|ws| ws.chat_harness_session(chat_id))
            .is_some_and(|(session_id, _)| session_id.is_empty())
    }

    /// Every doc entry's harness anchor from the journal, latest per entry.
    fn anchors(&self, chat_id: &str) -> Anchors {
        let mut anchors = Anchors::default();
        if let Ok(events) = self.journal.replay(chat_id, 0) {
            for (_, event) in events {
                match event {
                    AgentEvent::Anchor {
                        id,
                        entry_id: Some(entry_id),
                        ..
                    } => {
                        anchors.dead.remove(&entry_id);
                        anchors.live.insert(entry_id, id);
                    }
                    AgentEvent::SessionTombstoned {} => {
                        anchors
                            .dead
                            .extend(anchors.live.drain().map(|(entry_id, _)| entry_id));
                    }
                    _ => {}
                }
            }
        }
        anchors
    }

    /// The last harness session id named anywhere in the chat's journal, with
    /// the cwd of the `SessionStarted` that governs it. `Done.session_id`
    /// inherits the cwd of the most recent `SessionStarted` (same run).
    fn journal_harness_session(&self, chat_id: &str) -> Option<(String, String)> {
        let events = match self.journal.replay(chat_id, 0) {
            Ok(events) => events,
            Err(err) => {
                tracing::warn!(chat = %chat_id, error = %err, "journal scan for harness session failed");
                return None;
            }
        };
        let mut current_cwd = String::new();
        let mut found: Option<(String, String)> = None;
        for (_, event) in events {
            match event {
                AgentEvent::SessionStarted {
                    session_id, cwd, ..
                } => {
                    current_cwd = cwd;
                    if !session_id.is_empty() {
                        found = Some((session_id, current_cwd.clone()));
                    }
                }
                AgentEvent::Done {
                    session_id: Some(session_id),
                    ..
                } if !session_id.is_empty() => {
                    found = Some((session_id, current_cwd.clone()));
                }
                _ => {}
            }
        }
        found
    }

    fn remove_run(&self, chat_id: &str, run_id: &str) -> bool {
        let removed = {
            let mut runs = lock(&self.runs);
            if runs.get(chat_id).is_some_and(|h| h.run_id == run_id) {
                runs.remove(chat_id);
                true
            } else {
                false
            }
        };
        if removed {
            // The harness process owned them; they died with it.
            self.set_background_tasks(chat_id, Vec::new());
        }
        removed
    }
}

// ── compaction ──────────────────────────────────────────────────────────────

/// A routine compaction runs tens of seconds (37s measured live on claude
/// 2.1.273), but a long 1M-window chat can take several minutes; past this
/// the chat is released and a retry is refused until its next completed turn.
const COMPACT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(600);

/// Releases the chat's one-compaction-at-a-time reservation on every exit path.
struct CompactionReservation {
    inner: Arc<Inner>,
    chat_id: String,
    run_id: Option<String>,
}

impl Drop for CompactionReservation {
    fn drop(&mut self) {
        if let Some(run_id) = &self.run_id
            && self.inner.remove_run(&self.chat_id, run_id)
        {
            self.inner
                .set_status(&self.chat_id, SessionStatus::Idle, false);
        }
        lock(&self.inner.compacting).remove(&self.chat_id);
    }
}

#[derive(Default)]
struct CompactionDrain {
    /// `(pre_tokens, post_tokens)` from the harness's compaction boundary.
    compacted: Option<(Option<u64>, Option<u64>)>,
    result: Option<String>,
    failure: Option<String>,
}

/// Consume a compaction run's stream. Deliberately NOT the transcript fold:
/// nothing here is journaled, broadcast or written as a message part.
async fn drain_compaction(
    inner: &Arc<Inner>,
    chat_id: &str,
    run_cwd: &str,
    mut stream: futures::stream::BoxStream<'static, Result<AgentEvent, HarnessError>>,
    cancel: &mut watch::Receiver<bool>,
) -> CompactionDrain {
    let mut drain = CompactionDrain::default();
    let mut heartbeat = tokio::time::interval(std::time::Duration::from_secs(15));
    heartbeat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        let event = tokio::select! {
            // A compaction is one long silence, and the UI's staleness gate
            // would read an untouched Working row as no longer busy.
            _ = heartbeat.tick() => {
                inner.touch_session(chat_id);
                continue;
            }
            // Both ends below leave no boundary and no result: without an
            // explicit failure they would settle as the benign
            // "Nothing to compact" notice.
            _ = cancel.changed() => {
                drain.failure = Some("Context compaction was interrupted.".into());
                break;
            }
            next = stream.next() => match next {
                Some(Ok(event)) => event,
                Some(Err(err)) => {
                    drain.failure = Some(err.to_string());
                    break;
                }
                None => {
                    drain.failure = Some("harness stream ended without Done".into());
                    break;
                }
            },
        };
        match event {
            AgentEvent::Compacted {
                pre_tokens,
                post_tokens,
            } => drain.compacted = Some((pre_tokens, post_tokens)),
            AgentEvent::SessionStarted {
                session_id, cwd, ..
            } => inner.remember_harness_session(chat_id, &session_id, &cwd),
            AgentEvent::Error { message } => {
                drain.failure = Some(message);
                break;
            }
            AgentEvent::Done {
                session_id,
                result,
                error,
                ..
            } => {
                if let Some(session_id) = &session_id {
                    inner.remember_harness_session(chat_id, session_id, run_cwd);
                }
                drain.failure = error;
                drain.result = result;
                break;
            }
            _ => {}
        }
    }
    drain
}

fn record_compaction(
    doc: &SessionDoc,
    device_id: &str,
    pre_tokens: Option<u64>,
    post_tokens: Option<u64>,
) -> Result<(), DocError> {
    // Read the window BEFORE clearing: the meter stores tokens and window
    // as one atomic value, so the clear takes the window with it.
    let window = doc.context_usage().and_then(|usage| usage.window);
    doc.push_message(&SessionMessageEntry {
        id: new_id(),
        role: MessageRole::System,
        parts: vec![MessagePart::Text {
            id: "t0".into(),
            text: compaction_note(pre_tokens, post_tokens),
        }],
        created_at: now_ms(),
        device_id: device_id.to_string(),
        status: Some(MessageStatus::Complete),
        continuation_of: None,
        from_chat_id: None,
        fork_import: false,
        answers_request_id: None,
        duration_ms: None,
    })?;
    doc.clear_context_usage()?;
    if let Some(post_tokens) = post_tokens {
        doc.update_context_usage(Some(post_tokens), window)?;
    }
    Ok(())
}

/// The system entry's text: the boundary, plus the harness's own before/after
/// when it reported both.
fn compaction_note(pre_tokens: Option<u64>, post_tokens: Option<u64>) -> String {
    match (pre_tokens, post_tokens) {
        (Some(pre), Some(post)) => format!(
            "Context compacted · {} → {} tokens",
            format_tokens(pre),
            format_tokens(post)
        ),
        _ => "Context compacted".into(),
    }
}

fn format_tokens(tokens: u64) -> String {
    if tokens >= 1000 {
        format!("{:.1}k", tokens as f64 / 1000.0)
    } else {
        tokens.to_string()
    }
}

// ── run task ────────────────────────────────────────────────────────────────

/// Which provider session a side chat's prompt goes to.
#[derive(Clone, Copy)]
enum ProviderSession<'a> {
    /// A new provider session: it holds none of the transcript.
    Fresh,
    /// An existing one (id when known): it holds the fork's own turns.
    Continued(Option<&'a str>),
}

/// A run's provider session now holds the fork history its own prompt
/// carried (steers record theirs when confirmed, not here).
fn note_fork_history_session(doc: &SessionDoc, carried: bool, session_id: &str) {
    if carried && !session_id.is_empty() {
        let _ = doc.set_fork_history_session(session_id);
    }
}

impl Inner {
    /// `prompt` with the conversation a side chat's provider session lacks in
    /// front of it, or `None` when it lacks nothing. A fresh session gets the
    /// whole transcript (`current` excluded); a continued one only the fork's
    /// copied history before the seam, until that went out to that session.
    /// Native commands are never wrapped: providers route a leading `/name`.
    fn fork_history_prompt(
        &self,
        chat_id: &str,
        doc: &SessionDoc,
        harness_id: HarnessId,
        prompt: &str,
        current: Option<&str>,
        provider: ProviderSession<'_>,
    ) -> Option<String> {
        if native_command(prompt, harness_id)
            || !self
                .workspace()
                .and_then(|ws| ws.chat(chat_id).ok().flatten())
                .is_some_and(|chat| chat.created_by_chat_id.is_some())
        {
            return None;
        }
        let entries: Vec<_> = doc
            .read_entries()
            .unwrap_or_default()
            .into_iter()
            .filter(|entry| Some(entry.id.as_str()) != current)
            .collect();
        // created_by_chat_id also marks threads an agent opened, which copy
        // no history; only a fork carries the seam.
        let seam = entries.iter().position(|entry| {
            entry
                .parts
                .iter()
                .any(|part| matches!(part, zeron_doc::MessagePart::Fork { .. }))
        })?;
        let end = match provider {
            ProviderSession::Fresh => entries.len(),
            ProviderSession::Continued(session) => {
                let delivered = doc.fork_history_session();
                if session.is_some() && delivered.as_deref() == session {
                    return None;
                }
                seam
            }
        };
        let history: Vec<_> = entries[..end]
            .iter()
            .map(|entry| {
                let text = entry
                    .parts
                    .iter()
                    .filter_map(|part| match part {
                        zeron_doc::MessagePart::Text { text, .. } => Some(text.clone()),
                        zeron_doc::MessagePart::Tool { call, output, .. } => Some(format!(
                            "Tool: {}\n{}",
                            serde_json::to_string(call).unwrap_or_default(),
                            output.clone().unwrap_or_default()
                        )),
                        _ => None,
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
                (entry.role, text)
            })
            .filter(|(_, text)| !text.is_empty())
            .map(|(role, text)| serde_json::json!({ "role": role, "text": text }))
            .collect();
        (!history.is_empty()).then(|| {
            format!(
                "Continue this side conversation using the following prior conversation as context.\n<conversation>\n{}\n</conversation>\n\n{}",
                serde_json::to_string(&history).unwrap_or_default(),
                prompt
            )
        })
    }
}

/// Whether the provider routes this prompt as a native command: its delivered
/// text leads with `/name` (Codex `command_request`, OpenCode commands, Claude
/// slash commands). Anything put in front of it would make it model input.
fn native_command(prompt: &str, harness: HarnessId) -> bool {
    let delivered = if harness == HarnessId::Codex {
        zeron_proto::invocation::invocation_prompt(prompt)
    } else {
        zeron_proto::invocation::harness_prompt(prompt, harness)
    };
    zeron_proto::invocation::leading_command(&delivered).is_some()
}

/// A turn is in flight: streaming, or parked on a question it is still owed an
/// answer to. Notably NOT a persistent session that has parked between turns —
/// that holds a warm child with nothing outstanding — even while background
/// subagents keep its dot Working.
fn is_active(session: &Session) -> bool {
    matches!(
        session.status,
        SessionStatus::Working | SessionStatus::AwaitingInput
    ) && !session.held_by_subagents
}

// ── subagent docs ───────────────────────────────────────────────────────────

/// The per-subagent doc id: `{chatId}--sub--{suffix}`. Constrained by the
/// edge's `ID_RE` (`^[A-Za-z0-9_-]{1,128}$` — the same id names the ChatRoom
/// `chat2/{id}/ws` and the frozen blob `blob/{chatId}/{id}`): a clean, short
/// tool-use id rides verbatim; anything unclean or over budget hashes.
pub(crate) fn subagent_doc_id(chat_id: &str, tool_use_id: &str) -> String {
    let clean = tool_use_id
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-');
    let budget = 128usize.saturating_sub(chat_id.len() + "--sub--".len());
    if clean && !tool_use_id.is_empty() && tool_use_id.len() <= budget {
        return format!("{chat_id}--sub--{tool_use_id}");
    }
    use sha2::Digest as _;
    let digest = sha2::Sha256::digest(tool_use_id.as_bytes());
    let mut hex = String::with_capacity(16);
    for b in &digest[..8] {
        use std::fmt::Write as _;
        let _ = write!(hex, "{b:02x}");
    }
    format!("{chat_id}--sub--{hex}")
}

/// Test-only instrumentation: how often `drive_run`'s coalesced commit branch
/// fires. The sink-dirty regression test proves the branch ticks once per
/// commit window instead of spinning on a deadline left in the past.
/// Thread-local so parallel tests' runs can't bleed into the count: the
/// test's current-thread runtime runs its spawned `drive_run` on its own
/// thread.
#[cfg(test)]
thread_local! {
    static FLUSH_TICKS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// A live subagent transcript sink: its own doc (opened by id — the room
/// `chat2/{docId}/ws` dials automatically, so viewers sync it like a chat),
/// one streaming assistant entry folded from the tagged events. The held
/// doc Arc pins the doc warm for the LRU while the subagent runs.
struct SubagentSink {
    doc_id: String,
    doc: crate::doc_host::DocWriter,
    entry_id: String,
    started_at: i64,
    entry_index: Option<usize>,
    written: Vec<MessagePart>,
    folded: Vec<MessagePart>,
    dirty: bool,
}

impl SubagentSink {
    fn flush(&mut self, device_id: &str) {
        if !self.dirty {
            return;
        }
        // An empty fold still counts as flushed: leaving dirty set would hold
        // the run loop's coalesced-commit guard open on a deadline already in
        // the past, spinning it until the subagent emits real content.
        self.dirty = false;
        if self.folded.is_empty() {
            return;
        }
        let rendered = render_parts(&self.folded);
        let result = match self.entry_index {
            Some(ix) => {
                let mut w = SegmentWriter::resume(&self.doc, ix, std::mem::take(&mut self.written));
                let r = w.sync(&rendered);
                let (ix, written) = w.into_state();
                self.entry_index = Some(ix);
                self.written = written;
                r
            }
            None => {
                match SegmentWriter::begin(&self.doc, &self.entry_id, device_id, self.started_at) {
                    Ok(mut w) => {
                        let r = w.sync(&rendered);
                        let (ix, written) = w.into_state();
                        self.entry_index = Some(ix);
                        self.written = written;
                        r
                    }
                    Err(e) => Err(e),
                }
            }
        };
        if let Err(err) = result {
            // Fail soft: a broken subagent doc degrades to chip-only, never
            // errors the chat.
            tracing::warn!(doc = %self.doc_id, error = %err, "subagent sink flush failed");
        }
    }

    /// A parent→subagent steer ([`AgentEvent::UserMessage`], tagged): close
    /// the open assistant segment (it reads Complete above the steer), write
    /// the steer as its own USER entry, and reset so the next tagged delta
    /// opens a fresh assistant entry below it — the subagent transcript then
    /// reads like any steered chat.
    fn push_user(&mut self, device_id: &str, text: &str) {
        let rendered = render_parts(&self.folded);
        let closed = match self.entry_index.take() {
            Some(ix) => SegmentWriter::resume(&self.doc, ix, std::mem::take(&mut self.written))
                .finish(&rendered, MessageStatus::Complete),
            None if !self.folded.is_empty() => {
                match SegmentWriter::begin(&self.doc, &self.entry_id, device_id, self.started_at) {
                    Ok(w) => w.finish(&rendered, MessageStatus::Complete),
                    Err(e) => Err(e),
                }
            }
            None => Ok(()),
        };
        if let Err(err) = closed {
            tracing::warn!(doc = %self.doc_id, error = %err, "subagent segment close failed");
        }
        let entry = SessionMessageEntry {
            id: new_id(),
            role: MessageRole::User,
            parts: vec![MessagePart::Text {
                id: "t0".into(),
                text: text.to_owned(),
            }],
            created_at: now_ms(),
            device_id: device_id.to_owned(),
            status: Some(MessageStatus::Complete),
            continuation_of: None,
            from_chat_id: None,
            fork_import: false,
            answers_request_id: None,
            duration_ms: None,
        };
        if let Err(err) = self.doc.push_message(&entry) {
            tracing::warn!(doc = %self.doc_id, error = %err, "subagent steer write failed");
        }
        self.entry_id = new_id();
        self.started_at = now_ms();
        self.written = Vec::new();
        self.folded = Vec::new();
        self.dirty = false;
    }

    /// Final flush + entry finalize; returns the transcript JSON for the
    /// frozen blob (entries as the client renders them).
    fn finish(mut self, device_id: &str, status: MessageStatus) -> Option<String> {
        let rendered = render_parts(&self.folded);
        let finished = match self.entry_index {
            Some(ix) => SegmentWriter::resume(&self.doc, ix, std::mem::take(&mut self.written))
                .finish(&rendered, status),
            None if !self.folded.is_empty() => {
                match SegmentWriter::begin(&self.doc, &self.entry_id, device_id, self.started_at) {
                    Ok(w) => w.finish(&rendered, status),
                    Err(e) => Err(e),
                }
            }
            None => Ok(()),
        };
        if let Err(err) = finished {
            tracing::warn!(doc = %self.doc_id, error = %err, "subagent sink finish failed");
        }
        let entries = zeron_doc::join_continuation_entries(self.doc.read_entries().ok()?);
        serde_json::to_string(&entries).ok()
    }
}

/// What a parked turn settles to: Working while any subagent is still
/// live (a background agent is still running for this chat), else Idle.
fn parked_status(subagents: &std::collections::HashSet<String>) -> SessionStatus {
    if subagents.is_empty() {
        SessionStatus::Idle
    } else {
        SessionStatus::Working
    }
}

/// The parent-chip refresh a tagged event implies, for the in-place (parked)
/// path — LIFECYCLE ONLY, mirroring the fold (live tails were rejected:
/// per-delta chip rewrites grew the parent doc for the whole run).
fn subagent_chip_update(event: &AgentEvent) -> Option<&'static str> {
    match event {
        AgentEvent::Done { status, .. } => Some(match status {
            DoneStatus::Errored => "failed",
            _ => "done",
        }),
        _ => Some("running"),
    }
}

/// Apply the render-parts privacy policy: strip heavy/sensitive tool inputs before doc
/// entry. Full inputs live only in the local run journal.
fn render_parts(parts: &[MessagePart]) -> Vec<MessagePart> {
    parts
        .iter()
        .map(|part| match part {
            MessagePart::Tool {
                id,
                call,
                is_error,
                resolved,
                output,
                diff,
                output_ref,
                output_bytes,
                diff_ref,
                diff_stats,
                subagent_ref,
                subagent_status,
                subagent_tail,
            } => MessagePart::Tool {
                id: id.clone(),
                call: sanitize_tool_call(call),
                is_error: *is_error,
                resolved: *resolved,
                // Output summaries, diff stats, and sidecar refs are
                // deliberately kept: unlike raw tool inputs they are the
                // transcript's record of what happened, and the strip already
                // bounded them (docs/chat2-sync.md A1).
                output: output.clone(),
                diff: diff.clone(),
                output_ref: output_ref.clone(),
                output_bytes: *output_bytes,
                diff_ref: diff_ref.clone(),
                diff_stats: diff_stats.clone(),
                subagent_ref: subagent_ref.clone(),
                subagent_status: *subagent_status,
                subagent_tail: subagent_tail.clone(),
            },
            other => other.clone(),
        })
        .collect()
}

/// The persisted assistant text of a folded segment (workspace preview source).
fn folded_text(parts: &[MessagePart]) -> String {
    parts
        .iter()
        .filter_map(|part| match part {
            MessagePart::Text { text, .. } => Some(text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn sync_segment<'a>(
    doc: &'a SessionDoc,
    writer: &mut Option<SegmentWriter<'a>>,
    entry_id: &str,
    device_id: &str,
    started_at: i64,
    folded: &[MessagePart],
) -> Result<(), DocError> {
    if folded.is_empty() {
        return Ok(());
    }
    let rendered = render_parts(folded);
    if writer.is_none() {
        *writer = Some(SegmentWriter::begin(doc, entry_id, device_id, started_at)?);
    }
    if let Some(w) = writer.as_mut() {
        w.sync(&rendered)?;
    }
    Ok(())
}

fn finish_segment<'a>(
    doc: &'a SessionDoc,
    writer: Option<SegmentWriter<'a>>,
    entry_id: &str,
    device_id: &str,
    started_at: i64,
    folded: &[MessagePart],
    status: MessageStatus,
) -> Result<(), DocError> {
    let rendered = render_parts(folded);
    match writer {
        Some(w) => w.finish(&rendered, status),
        None if !folded.is_empty() => {
            SegmentWriter::begin(doc, entry_id, device_id, started_at)?.finish(&rendered, status)
        }
        None => Ok(()),
    }
}

/// Resume bookkeeping for one run task: which user entry the run answers (so
/// the startup-crash retry re-dispatches idempotently against the same doc
/// entry), whether `dispatch` injected the resume id itself (only
/// engine-injected resumes retry — a caller-specified resume fails loudly),
/// and whether this run already IS the retry (one attempt only).
struct RunResumeState {
    user_message_id: String,
    resume_injected: bool,
    startup_retry: bool,
    computer: Arc<RunComputer>,
    turn_computer: bool,
    fork_history_sent: Arc<std::sync::atomic::AtomicBool>,
}

fn cursor_unstarted_history(
    doc: &SessionDoc,
    current_id: &str,
    prompt: &str,
    has_session: bool,
) -> Result<String, DocError> {
    // Convert each message before JSON encoding. Rewriting canonical chips in
    // the encoded envelope can introduce unescaped quotes or newlines and can
    // cause Cursor's current message to be converted twice.
    let prompt = zeron_proto::invocation::harness_prompt(prompt, HarnessId::Cursor);
    let entries = doc.read_entries()?;
    let preceding: Vec<_> = entries
        .iter()
        .take_while(|entry| entry.id != current_id)
        .collect();
    // With an existing session, only bridge the tail whose assistant never
    // produced content. A process can die before writing its SDK-side receipt,
    // even though an older session ID still exists.
    let after = if has_session {
        preceding
            .iter()
            .rposition(|entry| {
                entry.role == MessageRole::Assistant
                    && entry.parts.iter().any(|part| match part {
                        MessagePart::Text { text, .. } | MessagePart::Reasoning { text, .. } => {
                            !text.is_empty()
                        }
                        MessagePart::Tool { .. } | MessagePart::Input { .. } => true,
                        _ => false,
                    })
            })
            .map_or(0, |i| i + 1)
    } else {
        0
    };
    let previous: Vec<String> = preceding
        .iter()
        .skip(after)
        .filter(|entry| entry.role == MessageRole::User)
        .map(|entry| {
            entry
                .parts
                .iter()
                .filter_map(|part| match part {
                    MessagePart::Text { text, .. } => Some(text.as_str()),
                    _ => None,
                })
                .collect::<Vec<_>>()
                .join("\n")
        })
        .filter(|text| !text.is_empty())
        .map(|text| zeron_proto::invocation::harness_prompt(&text, HarnessId::Cursor))
        .collect();
    if previous.is_empty() {
        return Ok(prompt);
    }
    Ok(format!(
        "The preceding user messages may not have reached a Cursor checkpoint before startup stopped. Retain this JSON as conversation history; do not rerun prior tools or side effects. Respond to the current message.\n{}",
        serde_json::json!({"previousUserMessages": previous, "currentUserMessage": prompt})
    ))
}

#[allow(clippy::too_many_arguments)]
async fn drive_run(
    inner: Arc<Inner>,
    chat_id: String,
    run_id: String,
    harness: Arc<dyn Harness>,
    mut request: RunRequest,
    doc: crate::doc_host::DocWriter,
    mut controls: RunControls,
    mut engine_rx: mpsc::UnboundedReceiver<AgentEvent>,
    mut cancel_rx: watch::Receiver<bool>,
    resume_state: RunResumeState,
) {
    let device_id = inner.device_id.clone();
    // Captured for post-run auto-titling (the request moves into the harness).
    let harness_id = harness.id();
    let user_prompt = request.prompt.clone();
    let run_cwd = request.cwd.clone();
    let mut last_turn_computer = resume_state.turn_computer;
    let run_computer = resume_state.computer.clone();
    run_computer.begin_turn(&resume_state.user_message_id, resume_state.turn_computer);
    if request.resume.is_none() {
        let _ = doc.clear_context_usage();
    }
    // Drivers without their own tool-server path (OpenCode, Cursor, pi) read
    // the thread tools from here; a client-sent value is never trusted.
    request.mcp = controls
        .tool_server
        .as_ref()
        .map(|server| zeron_proto::McpServer {
            name: "zeron".into(),
            command: server.command.clone(),
            args: server.args.clone(),
            env: server.env.iter().cloned().collect(),
        });
    // Kept whole for the startup-crash retry (same user entry; dispatch
    // re-injects the stored resume id). Option so the retry branch (inside
    // the event loop) can take ownership.
    let mut retry_request = Some(RunRequest {
        resume: None,
        ..request.clone()
    });
    // A side chat owns a fresh provider session. Bootstrap it from the frozen
    // conversation, never resume (and mutate) the parent's provider session.
    let mut carried_history = false;
    let provider = match request.resume.as_deref() {
        None => ProviderSession::Fresh,
        resumed => ProviderSession::Continued(resumed),
    };
    if let Some(prompt) = inner.fork_history_prompt(
        &chat_id,
        &doc,
        harness_id,
        &request.prompt,
        Some(&resume_state.user_message_id),
        provider,
    ) {
        request.prompt = prompt;
        carried_history = true;
        resume_state
            .fork_history_sent
            .store(true, std::sync::atomic::Ordering::Release);
    }
    // After the retry snapshot and the titler's capture, both of which want the
    // typed text: only what crosses into the harness carries the sidechat wrap.
    let mut request = match inner.harness_prompt(
        &chat_id,
        &request.prompt,
        &resume_state.user_message_id,
        resume_state.turn_computer,
    ) {
        Some(HarnessPrompt { prompt, handoff }) => {
            if let Some(text) = handoff {
                // Idempotent by part id: the startup-crash retry runs this
                // again against the same entry.
                let part = MessagePart::Handoff {
                    id: format!("{}-handoff", resume_state.user_message_id),
                    harness: harness_id,
                    text,
                };
                if let Err(err) = doc.append_part(&resume_state.user_message_id, &part) {
                    tracing::warn!(chat = %chat_id, error = %err, "provider-switch packet not recorded");
                }
            }
            RunRequest { prompt, ..request }
        }
        None => request,
    };
    // Startup can stop before the SDK saves user text, with no new session
    // ID or receipt. Bridge that unacknowledged tail from our transcript;
    // a fresh session needs all prior user text, not just the latest tail.
    let prepared = if harness_id == HarnessId::Cursor {
        cursor_unstarted_history(
            &doc,
            &resume_state.user_message_id,
            &request.prompt,
            request.resume.is_some(),
        )
        .map(|prompt| request.prompt = prompt)
        .map_err(|e| zeron_harness::HarnessError::Protocol(e.to_string()))
    } else {
        Ok(())
    };
    // Waiting here keeps dispatch and the shared queue-flush watcher responsive.
    // The pending-update marker still orders new subprocesses after installation.
    // Share the lease with the adapter so child cleanup outlives this event loop.
    let mut _execution_lease = None;
    let started = match prepared {
        Ok(()) => {
            let lease = tokio::select! {
                biased;
                _ = controls.interrupt.cancelled() => None,
                lease = inner.registry.execution_lease(harness_id) => Some(Arc::new(lease)),
            };
            if let Some(lease) = lease {
                _execution_lease = Some(lease.clone());
                controls.execution_lease = Some(lease);
                if let Some(listener) = inner.turn_listener.get() {
                    listener(&chat_id, &request.cwd);
                }
                let mut wire_request = request;
                if !matches!(harness_id, HarnessId::Cursor | HarnessId::Opencode) {
                    wire_request.prompt =
                        zeron_proto::invocation::harness_prompt(&wire_request.prompt, harness_id);
                }
                harness.run(wire_request, controls).await
            } else {
                Ok(futures::stream::once(async {
                    Ok(AgentEvent::Done {
                        status: DoneStatus::Interrupted,
                        result: None,
                        error: None,
                        session_id: None,
                    })
                })
                .boxed())
            }
        }
        Err(error) => Err(error),
    };
    let mut stream = match started {
        Ok(stream) => stream,
        Err(err) => {
            let message = err.to_string();
            tracing::warn!(chat = %chat_id, harness = ?harness_id, error = %message, "run failed to start");
            // The journal alone is live-only: without an entry the transcript
            // shows "Run failed" with no reason (an OpenCode server that never
            // booted looked exactly like that).
            let parts = [MessagePart::Error {
                id: "e0".into(),
                message: message.clone(),
            }];
            if let Err(err) = finish_segment(
                &doc,
                None,
                &new_id(),
                &device_id,
                now_ms(),
                &parts,
                MessageStatus::Complete,
            ) {
                tracing::warn!(chat = %chat_id, error = %err, "start failure entry failed");
            }
            inner.publish(
                &chat_id,
                &AgentEvent::Error {
                    message: message.clone(),
                },
            );
            inner.publish(
                &chat_id,
                &AgentEvent::Done {
                    status: DoneStatus::Errored,
                    result: None,
                    error: Some(message),
                    session_id: None,
                },
            );
            inner.remove_run(&chat_id, &run_id);
            inner.set_status(&chat_id, SessionStatus::Errored, false);
            inner.computer_turn_ended(&chat_id, run_computer.end_turn());
            return;
        }
    };

    let doc_ref: &SessionDoc = &doc;
    let mut folded: Vec<MessagePart> = Vec::new();
    // Every tool id this run has folded, across segment resets. Adapters
    // re-emit shape-bearing `tool_call_update`s (title/rawInput refreshes,
    // long-running completions) as full ToolCall events; once the fold has
    // reset at a steer/park boundary those ids are gone from `folded`, and
    // folding the echo would mint an orphan chip mid-text in the NEXT
    // segment — the mid-word transcript splits.
    let mut seen_tools: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut seen_images = std::collections::HashSet::new();
    for entry in doc_ref.read_entries().unwrap_or_default() {
        for part in entry.parts {
            if let MessagePart::Image { id, .. } = part {
                seen_images.insert(format!("{chat_id}\0{id}"));
                if let Some(tool_id) = id.strip_suffix(":image") {
                    seen_tools.insert(tool_id.to_owned());
                }
            }
        }
    }
    let mut prepared_events = std::collections::VecDeque::new();
    let mut entry_id = new_id();
    // The user entry the harness is currently answering: the dispatch's
    // prompt, then each confirmed steer in turn. Anchors file under it.
    let mut current_user_id = resume_state.user_message_id.clone();
    let mut segment_started = now_ms();
    let mut writer: Option<SegmentWriter<'_>> = None;
    let mut dirty = false;
    let mut flush_at = tokio::time::Instant::now();
    // Set when the engine interrupts the run: the harness gets this long to end its own
    // stream (its token was cancelled); past it, a terminal Done is synthesized.
    let mut interrupt_deadline: Option<tokio::time::Instant> = None;
    let mut interrupted = false;
    let mut saw_session_started = false;
    // Liveness heartbeat: this loop RUNNING is proof the harness stream is
    // open, so freshness must not depend on events arriving. Silent stretches
    // are normal and UNBOUNDED — a long tool call, redacted thinking, an
    // agent waiting on an external process, a question parked for an hour —
    // and each starved the UI's 45s staleness gate in turn (working strip /
    // AwaitingInput dot vanishing mid-run, both user-reported). No stall
    // timeout here by design (a first port was rejected — agents may
    // legitimately be quiet for >10min): a live child means Working, dying
    // paths each carry their own error, and engine death stops these ticks
    // so the gate still catches real crashes. touch_session throttles at 10s.
    let mut live_heartbeat = tokio::time::interval(std::time::Duration::from_secs(15));
    live_heartbeat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    // PERSISTENT SESSION (zeron runsBySession): a completed turn on a
    // steerable harness parks here instead of ending the run — the child and
    // its steering mailbox stay warm, and the next user message (dispatch
    // routes into a live run) starts the next turn with zero respawn/resume
    // latency. `Some(when)` = idle since then; the 30-min reaper below ends
    // a session nobody comes back to (zeron SESSION_IDLE_MS).
    // `ZERON_SESSION_IDLE_MS` overrides the window (tests).
    let session_idle = std::env::var("ZERON_SESSION_IDLE_MS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .map(std::time::Duration::from_millis)
        .unwrap_or(std::time::Duration::from_secs(30 * 60));
    // A live subagent stretches the window, but never unbounded: every
    // driver's subagent terminal is a best-effort vendor signal (claude's
    // untagged task_notification, grok's subagent_finished, codex thread
    // closure), and one lost signal would otherwise pin the child — and this
    // loop's registry heartbeat — until the chat is archived. A subagent
    // silent this long (4h by default) is presumed lost; the reap stamps its
    // chip failed.
    let subagent_silence = session_idle * 8;
    let mut idle_since: Option<tokio::time::Instant> = None;
    let steerable = harness.supports_steering();
    // TURN-QUIESCE WATCHDOG (2026-08-12 stuck-Working incident): a harness
    // that loses a turn's Done — the adapter never settles `session/prompt`
    // even though the agent finished — strands Working forever: the live
    // heartbeat above keeps the row fresh, and there is no per-turn timeout
    // by design. This is NOT that stall timeout: it never ends the run or
    // errors anything. When the stream has been silent past the window AND
    // the fold shows completed output with nothing in flight (no unresolved
    // tool, no open question), the turn parks exactly like a Done would —
    // segment finalized Complete, status Idle, child and mailbox warm. A
    // false trip (the agent was quietly waiting on something invisible)
    // costs a status dip: the parked-resume path below re-arms Working the
    // moment output flows again, and nothing is lost. `ZERON_TURN_QUIESCE_MS`
    // overrides the window; 0 disables.
    // RETIRED for native drivers: a harness whose every turn shape ends with
    // a deterministic wire Done (claude/codex/cursor native) needs no
    // quiesce backstop — arming one only risks false parks on long silent
    // work. The env knob can configure a diagnostic window, but a prompt
    // with an authoritative completion signal must still await that signal.
    // ACP retains the watchdog only for unowned self-continued activity.
    let deterministic_turn_end = harness.deterministic_turn_end();
    let authoritative_prompt_end = harness.authoritative_prompt_end();
    let quiesce_after: Option<std::time::Duration> = match std::env::var("ZERON_TURN_QUIESCE_MS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
    {
        Some(0) => None,
        Some(ms) => Some(std::time::Duration::from_millis(ms)),
        None if deterministic_turn_end => None,
        None => Some(std::time::Duration::from_secs(120)),
    };
    let mut last_stream_activity = tokio::time::Instant::now();
    // SELF-CONTINUED turns get a much SHORTER quiesce window. A turn the
    // agent starts on its own (background-task wake) can never receive a
    // harness Done: the adapter has no `session/prompt` outstanding to
    // settle — verified in claude-agent-acp's autonomous-result lane, which
    // consumes the SDK's turn-end without emitting anything; codex shows
    // the same shape. The watchdog is that turn shape's ONLY settle path,
    // so the default 120s window read as 2min of stuck-Working after every
    // background notification (user report 2026-08-13). The in-flight
    // fold gate below still protects running tools; reasoning heartbeats
    // push the window during real thinking. `ZERON_SELF_TURN_QUIESCE_MS`
    // overrides; 0 falls back to the normal window. An explicit
    // `ZERON_TURN_QUIESCE_MS=0` still disables the watchdog entirely.
    let self_quiesce_after: Option<std::time::Duration> =
        match std::env::var("ZERON_SELF_TURN_QUIESCE_MS")
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
        {
            Some(0) => None,
            Some(ms) => Some(std::time::Duration::from_millis(ms)),
            None => Some(std::time::Duration::from_secs(20)),
        };
    let mut self_continued_turn = false;
    // Spawn chips that have SETTLED (tagged Done seen). Content events for a
    // settled chip with no live sink are dropped — a straggler frame after
    // the freeze must not mint a new doc entry or wedge the transcript back
    // into Streaming. Only a steer (UserMessage) legitimately REOPENS a
    // settled subagent: it announces more work is coming.
    let mut settled_subagents: std::collections::HashSet<String> = std::collections::HashSet::new();
    // The harness has background shells running: the process that owns them
    // must outlive the idle window, or they die unfinished.
    let mut background_busy = false;
    // Live subagent sinks, parent tool-use id → transcript doc state.
    let mut subagents: std::collections::HashMap<String, SubagentSink> =
        std::collections::HashMap::new();
    // Spawns that have streamed and not yet settled, independent of whether
    // a sink could open (headless or open failure). They hold the parked
    // session: the idle reaper would kill the child they run in.
    let mut live_subagents: std::collections::HashSet<String> = std::collections::HashSet::new();
    // Last tagged subagent event. A background subagent outlives the turn that
    // spawned it, and its traffic never un-parks the chat, so the idle reaper
    // measures from here as well as from the park.
    let mut last_subagent_activity: Option<tokio::time::Instant> = None;

    let mut final_completed_turn = None;
    let final_status = loop {
        let event: AgentEvent = if let Some(event) = prepared_events.pop_front() {
            event
        } else {
            let raw_event = tokio::select! {
                biased;
                changed = cancel_rx.changed(), if !interrupted => {
                    let _ = changed;
                    interrupted = true;
                    interrupt_deadline = Some(
                        tokio::time::Instant::now() + std::time::Duration::from_secs(3),
                    );
                    continue;
                }
                _ = tokio::time::sleep_until(
                    interrupt_deadline.unwrap_or_else(tokio::time::Instant::now)
                ), if interrupt_deadline.is_some() => AgentEvent::Done {
                    status: DoneStatus::Interrupted,
                    result: None,
                    error: None,
                    session_id: None,
                },
                _ = live_heartbeat.tick() => {
                    inner.touch_session(&chat_id);
                    continue;
                }
                // An accepted update must not wait behind a warm between-turn
                // child for the full idle-reaper window. The completed turn is
                // already durable, so retire the parked process cleanly and let
                // the queued exclusive lease proceed.
                _ = tokio::time::sleep_until(tokio::time::Instant::now()),
                    if idle_since.is_some() && inner.registry.update_pending(harness_id) =>
                {
                    if let Some(token) = lock(&inner.runs)
                        .get(&chat_id)
                        .filter(|h| h.run_id == run_id)
                        .map(|h| h.interrupt_token.clone())
                    {
                        token.cancel();
                    }
                    break SessionStatus::Idle;
                }
                // Idle reaper (zeron SESSION_IDLE_MS): a parked persistent session
                // nobody returned to in 30 minutes releases its child. The turn
                // was finalized at Done, so this end is clean — no aborted stamp.
                // A live background subagent is somebody still using the child:
                // the window counts from its last activity, and stretches to
                // `subagent_silence` while its sink is open.
                _ = tokio::time::sleep_until(
                    idle_since
                        .map(|at| {
                            at.max(last_subagent_activity.unwrap_or(at))
                                + if live_subagents.is_empty() { session_idle } else { subagent_silence }
                        })
                        .unwrap_or_else(tokio::time::Instant::now)
                ), if idle_since.is_some() && !background_busy => {
                    tracing::info!(
                        chat = %chat_id,
                        live_subagents = live_subagents.len(),
                        "reaping idle persistent session"
                    );
                    if let Some(token) = lock(&inner.runs)
                        .get(&chat_id)
                        .filter(|h| h.run_id == run_id)
                        .map(|h| h.interrupt_token.clone())
                    {
                        token.cancel();
                    }
                    break SessionStatus::Idle;
                }
                Some(event) = engine_rx.recv() => event,
                next = stream.next() => match next {
                    Some(Ok(event)) => event,
                    // A stream error while PARKED is a post-turn child death —
                    // the turn was already finalized, so the run ends clean
                    // instead of stamping a completed session Errored.
                    Some(Err(err)) if idle_since.is_some() => {
                        tracing::warn!(chat = %chat_id, error = %err, "parked session child died; ending clean");
                        break SessionStatus::Idle;
                    }
                    Some(Err(err)) => AgentEvent::Done {
                        status: DoneStatus::Errored,
                        result: None,
                        error: Some(err.to_string()),
                        session_id: None,
                    },
                    None if interrupted => AgentEvent::Done {
                        status: DoneStatus::Interrupted,
                        result: None,
                        error: None,
                        session_id: None,
                    },
                    // Stream end while PARKED idle: a per-turn adapter closing
                    // after its final Done — a clean end, not a crash (the turn
                    // was already finalized). Persistent adapters keep the
                    // stream open and never hit this.
                    None if idle_since.is_some() => break SessionStatus::Idle,
                    None => AgentEvent::Done {
                        status: DoneStatus::Errored,
                        result: None,
                        error: Some("harness stream ended without Done".into()),
                        session_id: None,
                    },
                },
                _ = tokio::time::sleep_until(flush_at), if dirty || subagents.values().any(|s| s.dirty) => {
                    #[cfg(test)]
                    FLUSH_TICKS.with(|t| t.set(t.get() + 1));
                    // Coalesced STREAM_COMMIT_MS tick: one doc commit per window
                    // (parent + any dirty subagent docs).
                    if dirty {
                        if let Err(err) = sync_segment(
                            doc_ref, &mut writer, &entry_id, &device_id, segment_started, &folded,
                        ) {
                            tracing::warn!(chat = %chat_id, error = %err, "segment sync failed");
                        }
                        dirty = false;
                    }
                    for sink in subagents.values_mut() {
                        sink.flush(&device_id);
                    }
                    continue;
                }
                // Turn-quiesce watchdog (see the knob above). Armed only when the
                // fold says nothing is in flight: an unresolved tool part is a
                // command still running (legitimately silent for minutes — the
                // rejected stall-timeout case), an unresolved input part is a
                // question awaiting the user. The live-plan chip is exempt from
                // the tool check: it is a singleton that never resolves. An EMPTY
                // fold still arms — a Steered boundary that no output ever
                // follows is one of the wedge shapes — it just parks without
                // writing a segment (an empty finalize would leave a stub entry).
                _ = tokio::time::sleep_until({
                    let mut window = quiesce_after.unwrap_or_default();
                    if self_continued_turn && let Some(short) = self_quiesce_after {
                        window = window.min(short);
                    }
                    last_stream_activity + window
                }), if quiesce_after.is_some()
                    && (self_continued_turn || !authoritative_prompt_end)
                    && idle_since.is_none()
                    && !interrupted
                    && steerable
                    && !folded.iter().any(|p| match p {
                        MessagePart::Tool { id, resolved: false, .. } => {
                            id != zeron_proto::LIVE_PLAN_TOOL_ID
                        }
                        MessagePart::Input { resolved: false, .. } => true,
                        _ => false,
                    }) =>
                {
                    tracing::warn!(
                        chat = %chat_id,
                        quiet_ms = quiesce_after.unwrap_or_default().as_millis() as u64,
                        "turn quiesced: stream silent after completed output with no \
                         turn-end; parking (suspected missing harness Done)"
                    );
                    // Some adapters close a completed response through this
                    // engine watchdog instead of a native Done. Preserve that
                    // completion notice, but never notify for an empty boundary
                    // or while an accepted steer still awaits delivery.
                    let completed_turn = ((!folded.is_empty() || writer.is_some())
                        && !inner.has_pending_steers(&chat_id, &run_id))
                        .then(|| entry_id.clone());
                    if !folded.is_empty() || writer.is_some() {
                        if let Err(err) = finish_segment(
                            doc_ref,
                            writer.take(),
                            &entry_id,
                            &device_id,
                            segment_started,
                            &folded,
                            MessageStatus::Complete,
                        ) {
                            tracing::warn!(chat = %chat_id, error = %err, "quiesce segment finish failed");
                        }
                        inner.note_message(&chat_id, &folded_text(&folded));
                    }
                    folded.clear();
                    dirty = false;
                    entry_id = new_id();
                    segment_started = now_ms();
                    idle_since = Some(tokio::time::Instant::now());
                    self_continued_turn = false;
                    inner.set_status_with_completion(
                        &chat_id,
                        parked_status(&live_subagents),
                        false,
                        completed_turn,
                        !live_subagents.is_empty(),
                    );
                    inner.computer_turn_ended(&chat_id, run_computer.end_turn());
                    continue;
                }
            };

            prepared_events.extend(
                inner
                    .prepare_generated_image(&chat_id, raw_event, &mut seen_images)
                    .await,
            );
            let Some(event) = prepared_events.pop_front() else {
                continue;
            };
            event
        };

        // ── subagent routing ───────────────────────────────────────────
        // Tagged events NEVER fold into the parent transcript: they stream
        // into the subagent's own doc, and the parent keeps only the spawn
        // chip (ref + status + one-line tail) — refreshed through the live
        // fold while its segment still streams, else stamped in place (the
        // eager-done norm: the chip's entry finished before the background
        // subagent spoke). Handled BEFORE the parked gate so a parked
        // session's background traffic still reaches the subagent doc
        // without un-parking the chat.
        if let AgentEvent::Subagent {
            parent_tool_use_id,
            event: sub_event,
        } = &event
        {
            inner.publish(&chat_id, &event);
            last_subagent_activity = Some(tokio::time::Instant::now());
            let is_steer = matches!(
                sub_event.as_ref(),
                AgentEvent::UserMessage { .. } | AgentEvent::Steered { .. }
            );
            if is_steer {
                settled_subagents.remove(parent_tool_use_id);
            } else if settled_subagents.contains(parent_tool_use_id)
                && !subagents.contains_key(parent_tool_use_id)
            {
                // Straggler after the freeze: chip-only silence (the old
                // pre-viz behavior), never a reopened doc.
                continue;
            }
            let sub_id = subagent_doc_id(&chat_id, parent_tool_use_id);
            let chip_streaming = folded
                .iter()
                .any(|p| matches!(p, MessagePart::Tool { id, .. } if id == parent_tool_use_id));
            let sink_known = subagents.contains_key(parent_tool_use_id);
            // A Done with NO sink (a subagent that never streamed — codex
            // turn ends can beat registration) is chip-only: minting a doc
            // just to freeze it empty helps no one, and stamping the ref
            // would link the chip to that never-created doc (an empty tab
            // on click). A bare usage report is the same: the fold ignores
            // it, so a sink minted for it would be a phantom that also held
            // the session Working (below) with no Done ever coming.
            let done_only = !sink_known
                && matches!(
                    sub_event.as_ref(),
                    AgentEvent::Done { .. } | AgentEvent::ContextUsage { .. }
                );
            if chip_streaming {
                if !sink_known && !done_only {
                    for p in folded.iter_mut() {
                        if let MessagePart::Tool {
                            id,
                            call,
                            subagent_ref,
                            ..
                        } = p
                            && id == parent_tool_use_id
                            // Genus gate: a subagent ref may only ever bind
                            // to a SPAWN call — mis-keyed tagged traffic
                            // must not turn an ordinary chip into a spawn
                            // link (empty tab on click).
                            && call.is_subagent_spawn()
                        {
                            *subagent_ref = Some(sub_id.clone());
                        }
                    }
                }
                zeron_doc::fold_event_into_parts(&mut folded, &event);
                if !dirty {
                    dirty = true;
                    flush_at = tokio::time::Instant::now()
                        + std::time::Duration::from_millis(STREAM_COMMIT_MS);
                }
            }
            // Open the sink lazily; an open failure degrades to chip-only.
            if !sink_known && !done_only {
                let opened = inner.doc_host().and_then(|host| match host.open(&sub_id) {
                    Ok(handle) => Some(handle.writer()),
                    Err(err) => {
                        tracing::warn!(doc = %sub_id, error = %err, "subagent doc open failed (chip-only)");
                        None
                    }
                });
                if let Some(sub_doc) = opened {
                    subagents.insert(
                        parent_tool_use_id.clone(),
                        SubagentSink {
                            doc_id: sub_id.clone(),
                            doc: sub_doc,
                            entry_id: new_id(),
                            started_at: now_ms(),
                            entry_index: None,
                            written: Vec::new(),
                            folded: Vec::new(),
                            dirty: false,
                        },
                    );
                    if !chip_streaming {
                        let _ = doc_ref.update_subagent_chip(
                            parent_tool_use_id,
                            Some(&sub_id),
                            Some("running"),
                            None,
                        );
                    }
                }
            }
            let done = matches!(sub_event.as_ref(), AgentEvent::Done { .. });
            if done {
                settled_subagents.insert(parent_tool_use_id.clone());
                // The idle clock restarts at the last settle, not the parent's
                // Done hours ago, so the wake turn that reports it can stream.
                if live_subagents.remove(parent_tool_use_id)
                    && live_subagents.is_empty()
                    && idle_since.is_some()
                {
                    idle_since = Some(tokio::time::Instant::now());
                }
                if !chip_streaming {
                    // A resumed run can finish an older chip without ever
                    // opening a sink. The chip's lifecycle is independent of
                    // whether this run received transcript content.
                    let _ = doc_ref.update_subagent_chip(
                        parent_tool_use_id,
                        None,
                        subagent_chip_update(sub_event),
                        None,
                    );
                }
            } else if !matches!(
                sub_event.as_ref(),
                // The seeded spawn prompt (a UserMessage) arrives even for a
                // spawn that is refused and never settles; only the agent's
                // own output proves it is running.
                AgentEvent::ContextUsage { .. } | AgentEvent::UserMessage { .. }
            ) {
                live_subagents.insert(parent_tool_use_id.clone());
            }
            if let Some(sink) = subagents.get_mut(parent_tool_use_id) {
                if let AgentEvent::UserMessage { text } = sub_event.as_ref() {
                    // A steer splits ENTRIES, not parts — handled at the
                    // sink level (the fold ignores UserMessage).
                    sink.push_user(&device_id, text);
                    continue;
                }
                if let AgentEvent::ContextUsage { tokens, window } = sub_event.as_ref() {
                    if let Err(err) = sink.doc.update_context_usage(*tokens, *window) {
                        tracing::warn!(doc = %sink.doc_id, error = %err, "subagent usage write failed");
                    }
                    continue;
                }
                let was_clean = !sink.dirty;
                zeron_doc::fold_event_into_parts(&mut sink.folded, sub_event);
                sink.dirty = true;
                // A sink waking on its own must arm the same commit window
                // the parent's dirty flag does — `flush_at` is otherwise only
                // rescheduled on the parent's flip, so a past deadline would
                // fire the commit branch on every event instead of once per
                // STREAM_COMMIT_MS window.
                if was_clean && !dirty && flush_at <= tokio::time::Instant::now() {
                    flush_at = tokio::time::Instant::now()
                        + std::time::Duration::from_millis(STREAM_COMMIT_MS);
                }
                if done {
                    let status = match sub_event.as_ref() {
                        AgentEvent::Done {
                            status: DoneStatus::Errored,
                            ..
                        } => MessageStatus::Complete,
                        AgentEvent::Done {
                            status: DoneStatus::Interrupted,
                            ..
                        } => MessageStatus::Aborted,
                        _ => MessageStatus::Complete,
                    };
                    let sink = subagents.remove(parent_tool_use_id).expect("checked");
                    let doc_id = sink.doc_id.clone();
                    // FREEZE: the finished transcript uploads as a static R2
                    // blob (`blob/{chatId}/{subDocId}`) so viewers of
                    // finished subagents never wake the doc's room; dropping
                    // the sink unpins the doc for the LRU and the room
                    // idles. The live doc remains the fallback.
                    if let Some(json) = sink.finish(&device_id, status)
                        && let Some(host) = inner.doc_host()
                    {
                        host.upload_tool_sidecar(
                            &chat_id,
                            zeron_doc::SidecarPayload {
                                part_id: doc_id,
                                output: Some(json),
                                diff: None,
                            },
                        );
                    }
                }
            }
            // A parked session with a live subagent is still working: the
            // sidebar dot follows the sinks, not the parent turn. Held until
            // the last tagged Done (or the run ending).
            if idle_since.is_some() {
                let want = parked_status(&live_subagents);
                if lock(&inner.statuses).get(&chat_id).map(|s| s.status) != Some(want) {
                    inner.set_status_with_completion(
                        &chat_id,
                        want,
                        false,
                        None,
                        !live_subagents.is_empty(),
                    );
                }
            }
            continue;
        }

        // Any stream activity proves the run is alive — keep the session's
        // freshness inside the UI's 45s staleness window (throttled), and
        // push the quiesce watchdog's window out.
        inner.touch_session(&chat_id);
        last_stream_activity = tokio::time::Instant::now();
        if let AgentEvent::BackgroundTasks { tasks } = &event {
            let was_busy = std::mem::replace(&mut background_busy, !tasks.is_empty());
            if was_busy && !background_busy && idle_since.is_some() {
                idle_since = Some(tokio::time::Instant::now());
            }
            inner.set_background_tasks(&chat_id, tasks.clone());
            continue;
        }
        if let AgentEvent::Compacting { active } = &event {
            // A compaction can begin after a stray Done parked the session
            // (2026-09-22); the gate below would hold it Idle throughout.
            if *active && idle_since.take().is_some() {
                entry_id = new_id();
                segment_started = now_ms();
                inner.set_status(&chat_id, SessionStatus::Working, true);
            }
            inner.set_compacting(&chat_id, *active);
            continue;
        }
        if let AgentEvent::Compacted {
            pre_tokens,
            post_tokens,
        } = &event
        {
            inner.set_compacting(&chat_id, false);
            inner.publish(&chat_id, &event);
            if !folded.is_empty() || writer.is_some() {
                if let Err(err) = finish_segment(
                    doc_ref,
                    writer.take(),
                    &entry_id,
                    &device_id,
                    segment_started,
                    &folded,
                    MessageStatus::Complete,
                ) {
                    tracing::warn!(chat = %chat_id, error = %err, "segment finish failed");
                }
                inner.note_message(&chat_id, &folded_text(&folded));
                folded.clear();
                dirty = false;
                entry_id = new_id();
                segment_started = now_ms();
            }
            if let Err(err) = record_compaction(doc_ref, &device_id, *pre_tokens, *post_tokens) {
                tracing::warn!(chat = %chat_id, error = %err, "compaction note write failed");
            }
            continue;
        }
        // The engine's input bridge is the sole authority on input requests:
        // it mints the id and parks the resolver BEFORE emitting the event,
        // so a legitimate id is always pending here. A harness emitting its
        // own copy (a different id no resolver knows) would fold an
        // unanswerable twin chip into the doc — and answering the twin would
        // never resume the run. Dropped BEFORE the parked gate below: letting
        // it un-park the session and then dropping it would disarm the idle
        // reaper with no turn running (a leaked child no reap ever ends).
        if let AgentEvent::InputRequested { request_id, .. } = &event {
            let pending = lock(&inner.runs)
                .get(&chat_id)
                .map(|h| h.pending_inputs.clone());
            let known = pending.is_some_and(|p| lock(&p).contains_key(request_id));
            if !known {
                tracing::warn!(
                    chat = %chat_id,
                    request = %request_id,
                    "dropping harness-emitted InputRequested (unknown id; \
                     the engine input bridge owns this lifecycle)"
                );
                continue;
            }
        }
        // Capacity/occupancy can settle after Done; updating it must not reopen a turn.
        if let AgentEvent::ContextUsage { tokens, window } = &event {
            if let Err(err) = doc_ref.update_context_usage(*tokens, *window) {
                tracing::warn!(%chat_id, error = %err, "context usage write failed");
            }
            continue;
        }
        // PARKED: a steer boundary, a terminal Done, or SELF-CONTINUED OUTPUT
        // re-opens the session; everything else stays gated. The ACP child
        // keeps forwarding `session/update` frames after a turn completes,
        // and they split two ways:
        //
        // - Post-turn NOISE — late tool_call_updates for commands folded in a
        //   prior segment, command refreshes, reasoning heartbeats. Treating
        //   those as "the next turn" re-armed Working with no Done ever
        //   coming (the eternally-running session bug) and folded orphan
        //   parts into a phantom segment. Still dropped.
        // - SELF-CONTINUED WORK — Claude Code re-invokes itself when a
        //   background task finishes (turns no prompt started) and streams
        //   real output for them. Dropping those LOST transcript content
        //   (2026-08-12: "Build finished successfully…" streamed by the
        //   agent, absent from the doc). Fresh text or a genuinely new tool
        //   call resumes the session: new segment, Working, and the turn
        //   settles again via Done — or via the quiesce watchdog, which is
        //   what makes this resume safe where the naive version was not.
        //
        // The RESUME_GATE separates the two by arrival time: a finished
        // turn's tail flush lands within milliseconds of its Done, while a
        // self-continued turn starts a whole new agent round trip (seconds
        // at minimum, minutes in the incident). Inside the gate everything
        // non-boundary stays inert, exactly as before.
        let turn_was_active = idle_since.is_none();
        const RESUME_GATE: std::time::Duration = std::time::Duration::from_secs(1);
        if idle_since.is_some() {
            let self_continued = idle_since
                .is_some_and(|parked_at| parked_at.elapsed() >= RESUME_GATE)
                && (matches!(
                    &event,
                    AgentEvent::TextDelta { text } if !text.is_empty()
                ) || matches!(
                    &event,
                    AgentEvent::ToolCall { id, .. }
                        if id == zeron_proto::LIVE_PLAN_TOOL_ID || !seen_tools.contains(id)
                ));
            if self_continued {
                tracing::info!(
                    chat = %chat_id,
                    "parked session resumed by self-continued agent output"
                );
                idle_since = None;
                self_continued_turn = true;
                // The park cleared the fold; rotate to a fresh entry and
                // fall through — this event is the new segment's first part.
                entry_id = new_id();
                segment_started = now_ms();
                run_computer.begin_turn(&current_user_id, last_turn_computer);
                inner.set_status(&chat_id, SessionStatus::Working, true);
            } else {
                match &event {
                    AgentEvent::Steered { .. } => {
                        idle_since = None;
                        inner.set_status(&chat_id, SessionStatus::Working, true);
                    }
                    AgentEvent::Done { .. } => {
                        idle_since = None;
                    }
                    // A question with NO turn behind it (post-turn permission
                    // noise): answer it empty right here. Un-parking into
                    // AwaitingInput would disarm the idle reaper with no Done
                    // ever coming — stranded status, leaked warm child.
                    AgentEvent::InputRequested { request_id, .. } => {
                        let resolver = lock(&inner.runs)
                            .get(&chat_id)
                            .and_then(|h| lock(&h.pending_inputs).remove(request_id));
                        if let Some(tx) = resolver {
                            let _ = tx.send(Vec::new());
                        }
                        tracing::debug!(chat = %chat_id, "parked session: post-turn input request auto-declined");
                        continue;
                    }
                    // A stale answer settling after its turn already closed:
                    // nothing is running — stay parked.
                    AgentEvent::InputResolved { .. } => continue,
                    _ => continue,
                }
            }
        }
        // Empty reasoning deltas are PURE heartbeats: redacted thinking and
        // tool-input-generation windows stream them with no text. They fold
        // to nothing, so journaling/publishing them is only noise (hundreds
        // per long turn observed) — the touch above already did their job.
        if matches!(&event, AgentEvent::ReasoningDelta { text } if text.is_empty()) {
            continue;
        }
        let event = match event {
            AgentEvent::Anchor {
                role,
                id,
                entry_id: None,
            } => AgentEvent::Anchor {
                role,
                id,
                entry_id: Some(match role {
                    AnchorRole::User => current_user_id.clone(),
                    AnchorRole::Assistant => entry_id.clone(),
                }),
            },
            event => event,
        };

        // Stale tool echoes: a ToolCall/ToolResult naming an id folded in a
        // PRIOR segment (the fold reset at a steer or park since) belongs to
        // a chip that already rendered in its own entry. Folding the echo
        // would splice a phantom chip into the middle of the current
        // segment's streaming text (the mid-word transcript splits). Dropped;
        // same-segment refreshes still land in place.
        let in_segment = |folded: &[MessagePart], id: &str| {
            folded
                .iter()
                .any(|p| matches!(p, MessagePart::Tool { id: pid, .. } if pid == id))
        };
        match &event {
            // The live plan chip is a deliberate singleton (every update
            // reuses `LIVE_PLAN_TOOL_ID` so the fold refreshes in place):
            // treating its reappearance after a park/steer reset as a stale
            // echo dropped the todo list for the rest of the run — from the
            // first boundary on, plans never rendered again.
            AgentEvent::ToolCall { id, .. } if id == zeron_proto::LIVE_PLAN_TOOL_ID => {}
            AgentEvent::ToolResult { id, .. } if id == zeron_proto::LIVE_PLAN_TOOL_ID => {}
            AgentEvent::ToolCall { id, .. } => {
                if !in_segment(&folded, id) && seen_tools.contains(id) {
                    continue;
                }
                seen_tools.insert(id.clone());
            }
            AgentEvent::ToolResult { id, .. }
                if !in_segment(&folded, id) && seen_tools.contains(id) =>
            {
                continue;
            }
            _ => {}
        }

        // Startup-crash retry: a run that dies before ever starting (errored
        // Done, no SessionStarted, nothing streamed) means the AGENT CHILD
        // failed to come up — not that the injected resume id was bad. Since
        // the ACP conversion (2026-08-08) a stale id is handled inside the
        // harness (`session/load` falls back to `session/new`), so the old
        // guess here — tombstone the id, retry fresh — fired only on child
        // startup failures and permanently severed GOOD conversations (user
        // incident 2026-08-13). The id stays; retry ONCE against the same
        // user entry, resume and all, in case the crash was transient. A
        // helper that is down hard fails the retry too and surfaces its
        // crash text (the harness now appends exit status + stderr).
        if resume_state.resume_injected
            && !resume_state.startup_retry
            && !saw_session_started
            && folded.is_empty()
            && !interrupted
            && matches!(
                &event,
                AgentEvent::Done {
                    status: DoneStatus::Errored,
                    ..
                }
            )
            && let Some(retry) = retry_request.take()
        {
            tracing::warn!(
                chat = %chat_id,
                "run died before session start; retrying once (resume kept)"
            );
            inner.remove_run(&chat_id, &run_id);
            let engine = SessionsEngine {
                inner: inner.clone(),
            };
            let chat = chat_id.clone();
            let message_id = resume_state.user_message_id.clone();
            tokio::spawn(async move {
                // The user entry write inside dispatch is idempotent by
                // message id; `startup_retry` makes this attempt final.
                if let Err(err) = engine
                    .dispatch_with(&chat, harness_id, retry, Some(message_id), true)
                    .await
                {
                    tracing::error!(chat = %chat, error = %err, "startup-crash retry dispatch failed");
                    // No run is coming: leaving the row Working would spin
                    // the session forever with nothing behind it.
                    engine
                        .inner
                        .set_status(&chat, SessionStatus::Errored, false);
                }
            });
            return;
        }

        // A steer boundary splits the assistant entry exactly where the fold resets.
        if let AgentEvent::Steered {
            next_assistant_message_id,
            ..
        } = &event
        {
            inner.publish(&chat_id, &event);
            // A steer boundary means a real prompt owns the turn again — its
            // Done will come; the short self-continued window stands down.
            self_continued_turn = false;
            if let Err(err) = finish_segment(
                doc_ref,
                writer.take(),
                &entry_id,
                &device_id,
                segment_started,
                &folded,
                MessageStatus::Complete,
            ) {
                tracing::warn!(chat = %chat_id, error = %err, "segment finish failed");
            }
            inner.note_message(&chat_id, &folded_text(&folded));
            folded.clear();
            dirty = false;
            entry_id = next_assistant_message_id.clone().unwrap_or_else(new_id);
            segment_started = now_ms();
            // The elapsed timer is per user message, not per child process: a
            // steer boundary restarts it (matches the parked-resume path and
            // the composer's optimistic overlay, which already reads 0:00).
            inner.set_status(&chat_id, SessionStatus::Working, true);
            // The boundary confirms delivery of the oldest accepted steer —
            // retire its at-least-once ledger entry.
            let steer = lock(&inner.runs)
                .get(&chat_id)
                .filter(|h| h.run_id == run_id)
                .and_then(|h| lock(&h.routed_steers).pop_front());
            if let Some(steer) = steer {
                // Consumed: a steer carrying the fork history delivered it to
                // this runtime's provider session.
                if steer.fork_history
                    && let Some(session) = lock(&inner.harness_sessions)
                        .get(&chat_id)
                        .map(|known| known.session_id.clone())
                        .filter(|id| !id.is_empty())
                {
                    let _ = doc.set_fork_history_session(&session);
                }
                let previous = run_computer.turn().map(|turn| turn.id);
                run_computer.begin_turn(&steer.message_id, steer.computer_control);
                last_turn_computer = steer.computer_control;
                if previous.as_deref() != Some(steer.message_id.as_str()) {
                    inner.computer_turn_ended(&chat_id, previous);
                }
                current_user_id = steer.message_id;
            }
            continue;
        }

        match &event {
            AgentEvent::SessionStarted {
                session_id, cwd, ..
            } => {
                saw_session_started = true;
                // The event's own cwd (where the harness actually created the
                // session) scopes the stored id, not the request's.
                inner.remember_harness_session(&chat_id, session_id, cwd);
                note_fork_history_session(&doc, carried_history, session_id);
            }
            AgentEvent::Done {
                session_id: Some(session_id),
                ..
            } => {
                inner.remember_harness_session(&chat_id, session_id, &run_cwd);
                note_fork_history_session(&doc, carried_history, session_id);
            }
            AgentEvent::InputRequested { questions, .. } => {
                // Known-id guaranteed: the unknown-id twin was dropped above,
                // before the parked gate.
                if !questions.iter().any(UserInputQuestion::is_computer) {
                    inner.set_status(&chat_id, SessionStatus::AwaitingInput, false);
                }
            }
            AgentEvent::InputResolved { request_id, .. } => {
                let pending = lock(&inner.runs)
                    .get(&chat_id)
                    .filter(|h| h.run_id == run_id)
                    .map(|h| h.pending_inputs.clone());
                // A closed resolver means the harness stopped waiting: the agent withdrew it.
                let awaited = |rid: &str| {
                    pending
                        .as_ref()
                        .is_some_and(|p| lock(p).get(rid).is_some_and(|tx| !tx.is_closed()))
                };
                let resolved_consent = folded.iter().any(|part| {
                    matches!(part, MessagePart::Input { request_id: rid, questions, .. }
                        if rid == request_id && questions.iter().any(UserInputQuestion::is_computer))
                });
                let question_open = resolved_consent && folded.iter().any(|part| {
                    matches!(part, MessagePart::Input { request_id: rid, questions, resolved: false, .. }
                        if rid != request_id
                            && !questions.iter().any(UserInputQuestion::is_computer)
                            && awaited(rid))
                });
                let status = if question_open {
                    SessionStatus::AwaitingInput
                } else {
                    SessionStatus::Working
                };
                inner.set_status(&chat_id, status, false);
            }
            _ => {}
        }

        inner.publish(&chat_id, &event);

        // Defensive rule from zeron: a mid-run SessionStarted re-emission (Claude SDK
        // background re-invocations) must not wipe the segment being written.
        let skip_fold = matches!(&event, AgentEvent::SessionStarted { .. }) && !folded.is_empty();
        if !skip_fold {
            fold_event_into_parts(&mut folded, &event);
            if matches!(
                event,
                AgentEvent::ToolCall { .. } | AgentEvent::ToolResult { .. }
            ) {
                inner.bind_delegates(&chat_id, &run_id, &mut folded);
            }
            // R2 sidecar PARKED (2026-08-10, product call): the fold's
            // summary/stats ARE the doc's whole record — no refs stamped, no
            // uploads. Full outputs survive only in the host's local run
            // journal. To reintroduce: `zeron_doc::sidecar_payload(&event)`
            // → `apply_sidecar_refs` → `doc_host.upload_tool_sidecar`, all
            // still in place and tested.
        }

        if let AgentEvent::Done { status, .. } = &event {
            inner.set_compacting(&chat_id, false);
            // A question still pending at turn end can never be legitimately
            // answered (its turn is over): drain the resolvers NOW, or a late
            // `respond_input` finds one, emits InputResolved, and un-parks
            // the session into Working with no turn behind it — stranded
            // Working, timer forever, reaper disarmed. Empty answers unblock
            // the harness-side bridge like an interrupt does.
            let pending = lock(&inner.runs)
                .get(&chat_id)
                .filter(|h| h.run_id == run_id)
                .map(|h| h.pending_inputs.clone());
            if let Some(pending) = pending {
                for (_, tx) in lock(&pending).drain() {
                    let _ = tx.send(Vec::new());
                }
            }
            let message_status = match status {
                DoneStatus::Interrupted => MessageStatus::Aborted,
                DoneStatus::Completed | DoneStatus::Errored => MessageStatus::Complete,
            };
            // No dangling chips: a run that ends for ANY reason (completed,
            // errored, interrupted) terminally resolves its input parts — an
            // unresolved question must not outlive the run that asked it
            // (its resolver died with the run; an answer could never land).
            for part in folded.iter_mut() {
                if let MessagePart::Input { resolved, .. } = part {
                    *resolved = true;
                }
            }
            // A Done landing on a PARKED session with nothing streamed (the
            // idle reaper's or an interrupt's own teardown) has no entry to
            // finalize — writing one would leave an empty aborted stub.
            let nothing_streamed = writer.is_none() && folded.is_empty();
            if !nothing_streamed {
                if let Err(err) = finish_segment(
                    doc_ref,
                    writer.take(),
                    &entry_id,
                    &device_id,
                    segment_started,
                    &folded,
                    message_status,
                ) {
                    tracing::warn!(chat = %chat_id, error = %err, "final segment finish failed");
                }
                inner.note_message(&chat_id, &folded_text(&folded));
            }
            if *status == DoneStatus::Completed {
                // A cleanly completed turn resets the auto-resume revival
                // budget: only consecutive crash-revive-crash cycles spend it.
                inner.journal.clear_resume_attempts(&chat_id);
            }
            // Exchange completed on an untitled chat → name it (fire-and-forget;
            // interrupted/errored turns never trigger naming).
            if *status == DoneStatus::Completed
                && let Some(titles) = inner.titles.get()
            {
                titles.maybe_generate(&chat_id, harness_id, &user_prompt, &run_cwd);
            }
            // An accepted steer awaiting its boundary owns the continuation:
            // the previous Done is an internal handoff, not a completion ping.
            // Ordinary queued rows are not in this ledger and still notify.
            let pending_steer = inner.has_pending_steers(&chat_id, &run_id);
            let completed_turn = (*status == DoneStatus::Completed
                && !interrupted
                && turn_was_active
                && !pending_steer)
                .then(|| entry_id.clone());
            // PERSISTENT SESSION: a cleanly completed turn on a steerable
            // harness PARKS instead of ending — child + mailbox stay warm for
            // the next routed dispatch; per-turn state resets for it.
            if *status == DoneStatus::Completed
                && steerable
                && !interrupted
                && !inner.registry.update_pending(harness_id)
            {
                folded.clear();
                dirty = false;
                entry_id = new_id();
                segment_started = now_ms();
                // Resume-retry is strictly a first-turn concern.
                saw_session_started = true;
                idle_since = Some(tokio::time::Instant::now());
                self_continued_turn = false;
                // Accepted steers still own this runtime. Publishing Idle
                // here lets the ordinary queue overtake that continuation.
                inner.set_status_with_completion(
                    &chat_id,
                    if pending_steer {
                        SessionStatus::Working
                    } else {
                        parked_status(&live_subagents)
                    },
                    false,
                    completed_turn,
                    !live_subagents.is_empty(),
                );
                inner.computer_turn_ended(&chat_id, run_computer.end_turn());
                continue;
            }
            final_completed_turn = completed_turn;
            break match status {
                DoneStatus::Errored => SessionStatus::Errored,
                _ => SessionStatus::Idle,
            };
        }

        if !folded.is_empty() && !dirty {
            dirty = true;
            flush_at =
                tokio::time::Instant::now() + std::time::Duration::from_millis(STREAM_COMMIT_MS);
        }
    };

    // Any subagent still streaming when the run ends freezes as-is: the
    // parent process is gone, so nothing more can arrive on this stream.
    for (parent_id, mut sink) in subagents.drain() {
        zeron_doc::fold_event_into_parts(
            &mut sink.folded,
            &AgentEvent::Error {
                message: "Stopped: the chat's agent process ended before this subagent finished."
                    .into(),
            },
        );
        let doc_id = sink.doc_id.clone();
        let _ = doc_ref.update_subagent_chip(&parent_id, None, Some("failed"), None);
        if let Some(json) = sink.finish(&device_id, MessageStatus::Aborted)
            && let Some(host) = inner.doc_host()
        {
            host.upload_tool_sidecar(
                &chat_id,
                zeron_doc::SidecarPayload {
                    part_id: doc_id,
                    output: Some(json),
                    diff: None,
                },
            );
        }
    }

    // Claim any accepted-but-unconfirmed steers BEFORE the handle goes away:
    // a routed send that raced this exit either finds its entry gone (we own
    // it — re-dispatched below) or reclaims it and starts a fresh run itself.
    let orphans: Vec<RoutedSteer> = lock(&inner.runs)
        .get(&chat_id)
        .filter(|h| h.run_id == run_id)
        .map(|h| std::mem::take(&mut *lock(&h.routed_steers)).into())
        .unwrap_or_default();
    inner.remove_run(&chat_id, &run_id);
    inner.set_status_with_completion(&chat_id, final_status, false, final_completed_turn, false);
    inner.computer_turn_ended(&chat_id, run_computer.end_turn());
    if !interrupted && !orphans.is_empty() {
        // The dying run accepted these into its mailbox but never confirmed a
        // Steered boundary (idle-reaper race, a mid-turn error discarding
        // queued boundary steers, a parked child death). Their user entries
        // are already in the transcript — a message that shows as sent must
        // never silently not run. Re-dispatch each as a fresh turn
        // (write_user_message dedupes by id; resume is engine-injected).
        let engine = SessionsEngine {
            inner: inner.clone(),
        };
        let chat = chat_id.clone();
        tokio::spawn(async move {
            for steer in orphans {
                let Some(mut request) = engine.last_request(&chat) else {
                    tracing::warn!(chat = %chat, "orphaned steer lost: no run config to re-dispatch");
                    break;
                };
                request.prompt = steer.prompt.clone();
                request.resume = None;
                request.attachments = steer.attachments.clone();
                request.computer_control = steer.computer_control;
                tracing::info!(chat = %chat, "re-dispatching steer orphaned by a dying run");
                let Some(host) = engine.inner.doc_host() else {
                    tracing::warn!(chat = %chat, "orphaned steer lost: doc host unavailable");
                    break;
                };
                if let Err(err) = host
                    .dispatch_with_source_context(
                        &engine,
                        &chat,
                        harness_id,
                        request,
                        Some(steer.message_id.clone()),
                    )
                    .await
                {
                    tracing::warn!(chat = %chat, error = %err, "orphaned steer re-dispatch failed");
                    engine
                        .inner
                        .set_status(&chat, SessionStatus::Errored, false);
                    break;
                }
            }
        });
    }
}

fn dispatched_generation(doc: &SessionDoc, message_id: &str) -> Option<u64> {
    doc.read_commands()
        .ok()?
        .into_iter()
        .rev()
        .find(|command| match &command.payload {
            zeron_doc::SessionCommandPayload::Run { message_id: id, .. } => id == message_id,
            zeron_doc::SessionCommandPayload::Steer {
                message_id: Some(id),
                ..
            } => id == message_id,
            _ => false,
        })?
        .computer_generation
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_self_continued_turn_keeps_its_revoke_and_foreground_but_a_new_turn_resets_them() {
        let computer = super::RunComputer::new();
        computer.begin_turn("t1", true);
        assert!(computer.answer_foreground(Some("t1"), Some("bundle:a"), true));
        assert!(computer.answer_foreground(Some("t1"), Some("bundle:b"), false));
        assert_eq!(computer.revoke_turn().as_deref(), Some("t1"));
        assert_eq!(computer.end_turn().as_deref(), Some("t1"));
        computer.begin_turn("t1", true);
        let resumed = computer.turn().unwrap();
        assert!(resumed.desktop_revoked, "a parked resume stays revoked");
        assert_eq!(
            resumed.foreground,
            super::ComputerTurnForeground {
                declined: true,
                apps: vec!["bundle:a".into()],
            }
        );
        computer.begin_turn("t1", true);
        assert!(computer.turn().unwrap().desktop_revoked);
        computer.begin_turn("t2", true);
        let next = computer.turn().unwrap();
        assert!(!next.desktop_revoked);
        assert_eq!(next.foreground, super::ComputerTurnForeground::default());
        computer.end_turn();
        computer.begin_turn("t1", true);
        assert!(!computer.turn().unwrap().desktop_revoked);
    }

    #[test]
    fn cursor_recovery_converts_rich_messages_before_json_encoding() {
        let doc = zeron_doc::SessionDoc::init("cursor-rich-recovery").unwrap();
        let skill = zeron_proto::invocation::Invocation::Skill {
            name: "review \"quoted\"".into(),
            path: "/repo/quoted \"path\"/SKILL.md".into(),
            command: None,
        }
        .link();
        let previous = format!("Previous {skill}\nSecond line with \\ and \"quotes\"");
        doc.push_message(&zeron_doc::SessionMessageEntry {
            id: "u1".into(),
            role: zeron_doc::MessageRole::User,
            parts: vec![zeron_doc::MessagePart::Text {
                id: "u1-text".into(),
                text: previous.clone(),
            }],
            created_at: 0,
            device_id: "test".into(),
            status: None,
            continuation_of: None,
            from_chat_id: None,
            fork_import: false,
            answers_request_id: None,
            duration_ms: None,
        })
        .unwrap();
        let current = format!("Current {skill}\nKeep **Markdown**");
        let delivered = super::cursor_unstarted_history(&doc, "u2", &current, false).unwrap();
        let (_, json) = delivered.split_once('\n').unwrap();
        let parsed: serde_json::Value = serde_json::from_str(json).unwrap();
        assert_eq!(
            parsed["currentUserMessage"],
            zeron_proto::invocation::harness_prompt(&current, zeron_proto::HarnessId::Cursor)
        );
        assert_eq!(
            parsed["previousUserMessages"][0],
            zeron_proto::invocation::harness_prompt(&previous, zeron_proto::HarnessId::Cursor)
        );
    }

    #[test]
    fn cursor_without_a_session_id_retains_only_preceding_user_messages() {
        let doc = zeron_doc::SessionDoc::init("cursor-unstarted").unwrap();
        for (id, role, text) in [
            (
                "u1",
                zeron_doc::MessageRole::User,
                "first interrupted request",
            ),
            ("a1", zeron_doc::MessageRole::Assistant, "partial output"),
            ("u2", zeron_doc::MessageRole::User, "current request"),
            ("u3", zeron_doc::MessageRole::User, "future pending request"),
        ] {
            doc.push_message(&zeron_doc::SessionMessageEntry {
                id: id.into(),
                role,
                parts: vec![zeron_doc::MessagePart::Text {
                    id: format!("{id}-text"),
                    text: text.into(),
                }],
                created_at: 0,
                device_id: "test".into(),
                status: None,
                continuation_of: None,
                from_chat_id: None,
                fork_import: false,
                answers_request_id: None,
                duration_ms: None,
            })
            .unwrap();
        }
        assert_eq!(
            super::cursor_unstarted_history(&doc, "u1", "first interrupted request", false)
                .unwrap(),
            "first interrupted request"
        );
        let prompt = super::cursor_unstarted_history(&doc, "u2", "current request", false).unwrap();
        assert!(prompt.contains("first interrupted request"));
        assert!(prompt.contains("current request"));
        assert!(!prompt.contains("partial output"));
        assert!(!prompt.contains("future pending request"));
        assert!(prompt.contains("do not rerun prior tools or side effects"));
        assert_eq!(
            super::cursor_unstarted_history(&doc, "u2", "current request", true).unwrap(),
            "current request",
            "content from the preceding turn is already in the native session"
        );
        assert!(
            super::cursor_unstarted_history(&doc, "u3", "future pending request", true)
                .unwrap()
                .contains("current request"),
            "an unacknowledged prompt after an older checkpoint must survive"
        );
        assert_eq!(doc.read_entries().unwrap().len(), 4);
    }

    use zeron_proto::{HarnessId, RunRequest, SandboxLevel};

    #[test]
    fn pending_branch_caches_misses_and_tracks_lifecycle_updates() {
        use super::*;
        let dir = tempfile::tempdir().unwrap();
        let sessions = SessionsEngine::new(
            "host".into(),
            Arc::new(RunJournal::open(dir.path().join("journals")).unwrap()),
            Arc::new(HarnessRegistry::new()),
        );
        assert!(sessions.inner.pending_branch("chat").is_none());
        assert!(matches!(
            lock(&sessions.inner.pending_branches).get("chat"),
            Some(None)
        ));
        let branch = SessionBranch {
            keep: Some("reply-1".into()),
            drop: None,
        };
        sessions
            .inner
            .set_pending_branch("chat", branch.clone(), None);
        assert_eq!(sessions.inner.pending_branch("chat"), Some(branch));
        sessions
            .inner
            .remember_harness_session("chat", "session", "/tmp");
        assert!(sessions.inner.pending_branch("chat").is_none());
        assert!(matches!(
            lock(&sessions.inner.pending_branches).get("chat"),
            Some(None)
        ));
    }

    #[test]
    fn a_fork_instant_outlives_its_branch_and_later_rewinds() {
        use super::*;
        let dir = tempfile::tempdir().unwrap();
        let journal = Arc::new(RunJournal::open(dir.path().join("journals")).unwrap());
        let sessions = SessionsEngine::new(
            "host".into(),
            journal.clone(),
            Arc::new(HarnessRegistry::new()),
        );
        let at = "2026-09-22T17:30:00.000Z".to_string();
        let branch = SessionBranch {
            keep: Some("reply-1".into()),
            drop: None,
        };
        sessions
            .inner
            .set_pending_branch("fork", branch.clone(), Some(at.clone()));
        let mut request = request();
        request.resume = Some("source".into());
        sessions.inner.inject_branch("fork", &mut request);
        assert_eq!(request.branch, Some(branch.clone()));
        assert_eq!(request.forked_at.as_deref(), Some(at.as_str()));

        journal
            .append(
                "fork",
                &AgentEvent::SessionStarted {
                    harness: HarnessId::ClaudeCode,
                    model: "m".into(),
                    tools: Vec::new(),
                    cwd: "/tmp".into(),
                    session_id: "forked".into(),
                    assistant_message_id: "a".into(),
                },
            )
            .unwrap();
        sessions
            .inner
            .remember_harness_session("fork", "forked", "/tmp");
        sessions.inner.set_pending_branch("fork", branch, None);

        let restarted =
            SessionsEngine::new("host".into(), journal, Arc::new(HarnessRegistry::new()));
        let mut request = request_with_resume("forked");
        restarted.inner.inject_branch("fork", &mut request);
        assert_eq!(request.forked_at.as_deref(), Some(at.as_str()));

        let mut fresh = request_with_resume("forked");
        fresh.resume = None;
        restarted.inner.inject_branch("fork", &mut fresh);
        assert_eq!(fresh.forked_at, None);

        let mut plain = request_with_resume("other");
        restarted.inner.inject_branch("plain", &mut plain);
        assert_eq!(plain.forked_at, None);
        assert_eq!(plain.branch, None);
    }

    fn request_with_resume(session: &str) -> RunRequest {
        RunRequest {
            resume: Some(session.into()),
            ..request()
        }
    }

    #[test]
    fn a_tool_diff_is_read_back_from_the_journal_through_the_subagent_tag() {
        use super::*;
        let dir = tempfile::tempdir().unwrap();
        let journal = Arc::new(RunJournal::open(dir.path().join("journals")).unwrap());
        let sessions = SessionsEngine::new(
            "host".into(),
            journal.clone(),
            Arc::new(HarnessRegistry::new()),
        );
        let diff = |new_text: &str| ToolDiff {
            path: "/w/a.rs".into(),
            old_text: None,
            new_text: new_text.into(),
            line_offset: None,
        };
        let result = |id: &str, diff: Option<ToolDiff>| AgentEvent::ToolResult {
            id: id.into(),
            is_error: false,
            output: None,
            diff,
        };
        journal
            .append("chat", &result("t1", Some(diff("one"))))
            .unwrap();
        journal.append("chat", &result("t2", None)).unwrap();
        journal
            .append(
                "chat",
                &AgentEvent::Subagent {
                    parent_tool_use_id: "spawn".into(),
                    event: Box::new(result("t3", Some(diff("child")))),
                },
            )
            .unwrap();
        assert_eq!(
            sessions.journaled_tool_diff("chat", "t1"),
            Some(diff("one"))
        );
        assert_eq!(
            sessions.journaled_tool_diff("chat", "t3"),
            Some(diff("child"))
        );
        // The subagent tab asks by its own doc id.
        assert_eq!(
            sessions.journaled_tool_diff(&subagent_doc_id("chat", "spawn"), "t3"),
            Some(diff("child"))
        );
        assert_eq!(sessions.journaled_tool_diff("chat", "t2"), None);
        assert_eq!(sessions.journaled_tool_diff("other", "t1"), None);
    }

    #[tokio::test]
    async fn generated_image_failure_is_sanitized_even_inside_subagents() {
        use super::*;
        let dir = tempfile::tempdir().unwrap();
        let sessions = SessionsEngine::new(
            "host".into(),
            Arc::new(RunJournal::open(dir.path().join("journals")).unwrap()),
            Arc::new(HarnessRegistry::new()),
        );
        let source = "/outside/private/source.png";
        let event = AgentEvent::Subagent {
            parent_tool_use_id: "child".into(),
            event: Box::new(AgentEvent::GeneratedImage {
                id: "i:image".into(),
                path: source.into(),
                name: "secret".into(),
                mime_type: "bad".into(),
            }),
        };
        let events = sessions
            .inner
            .prepare_generated_image("chat", event, &mut std::collections::HashSet::new())
            .await;
        assert_eq!(events.len(), 2);
        let mut parts = vec![];
        fold_event_into_parts(
            &mut parts,
            &AgentEvent::ToolCall {
                id: "i".into(),
                call: zeron_proto::ToolCall::Unknown {
                    name: "Generate image".into(),
                    input: None,
                },
            },
        );
        for event in &events {
            sessions.inner.publish("chat", event);
            let AgentEvent::Subagent {
                parent_tool_use_id,
                event,
            } = event
            else {
                panic!("lost owner")
            };
            assert_eq!(parent_tool_use_id, "child");
            fold_event_into_parts(&mut parts, event);
        }
        assert!(matches!(
            parts[0],
            MessagePart::Tool {
                is_error: true,
                resolved: true,
                ..
            }
        ));
        assert!(matches!(parts[1], MessagePart::Error { .. }));
        let journal =
            serde_json::to_string(&sessions.inner.journal.replay("chat", 0).unwrap()).unwrap();
        assert!(!journal.contains(source));
        assert!(!journal.contains("secret"));
    }

    fn request() -> RunRequest {
        RunRequest {
            mcp: None,
            prompt: "first".into(),
            harness: None,
            model: Some("grok-4.6".into()),
            reasoning: Some(zeron_proto::ReasoningLevel::High),
            model_options: serde_json::Map::new(),
            cwd: "/tmp".into(),
            sandbox: SandboxLevel::WorkspaceWrite,
            permission_mode: zeron_proto::PermissionMode::Full,
            auto_approve: true,
            resume: None,
            attachments: Vec::new(),
            worktree: None,
            branch: None,
            forked_at: None,
            computer_control: false,
        }
    }

    #[test]
    fn live_routing_requires_the_same_runtime_configuration() {
        let initial = request();
        let config = RuntimeConfig::from_request(HarnessId::Grok, &initial);

        let mut follow_up = initial.clone();
        follow_up.prompt = "second".into();
        follow_up.resume = Some("session-1".into());
        assert!(config.can_route(HarnessId::Grok, &follow_up, false));

        follow_up.model = Some("grok-4.5".into());
        assert!(!config.can_route(HarnessId::Grok, &follow_up, true));
        follow_up.model = initial.model.clone();

        follow_up.reasoning = Some(zeron_proto::ReasoningLevel::Medium);
        assert!(!config.can_route(HarnessId::Grok, &follow_up, false));
        assert!(
            config.can_route(HarnessId::Grok, &follow_up, true),
            "a harness that changes effort in place keeps its run"
        );
        follow_up.reasoning = None;
        assert!(!config.can_route(HarnessId::Grok, &follow_up, true));
        follow_up.reasoning = initial.reasoning;

        follow_up.attachments.push("/tmp/image.png".into());
        assert!(config.can_route(HarnessId::Grok, &follow_up, false));
    }

    #[test]
    fn clean_tool_ids_ride_verbatim_and_fit_id_re() {
        let id = subagent_doc_id("chat-abc123", "toolu_01EjFLnNhCiMR2PBNKcVvkT");
        assert_eq!(id, "chat-abc123--sub--toolu_01EjFLnNhCiMR2PBNKcVvkT");
        assert!(id.len() <= 128);
        assert!(
            id.chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
        );
    }

    #[test]
    fn unclean_or_oversized_ids_hash_deterministically() {
        let dirty = subagent_doc_id("chat", "call/with:odd chars");
        assert!(
            dirty
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-'),
            "{dirty}"
        );
        assert_eq!(dirty, subagent_doc_id("chat", "call/with:odd chars"));
        let long_chat = "c".repeat(100);
        let long = subagent_doc_id(&long_chat, &"t".repeat(64));
        assert!(long.len() <= 128, "{}", long.len());
        // Distinct inputs stay distinct.
        assert_ne!(
            subagent_doc_id("chat", "a:b"),
            subagent_doc_id("chat", "a:c")
        );
    }

    #[tokio::test]
    async fn subagent_sink_flush_clears_dirty_when_nothing_folded() {
        use super::*;
        let dir = tempfile::tempdir().unwrap();
        let store = Arc::new(zeron_sync::DocsStore::open(dir.path()).unwrap());
        let host = DocHost::new(
            store,
            crate::doc_host::DocHostConfig {
                device_id: "dev-test".into(),
                default_harness: HarnessId::Mock,
                edge: None,
            },
        );
        let handle = host.open("chat-sub-flush--sub--t1").unwrap();
        let mut sink = SubagentSink {
            doc_id: "chat-sub-flush--sub--t1".into(),
            doc: handle.writer(),
            entry_id: "entry-1".into(),
            started_at: 0,
            entry_index: None,
            written: Vec::new(),
            folded: Vec::new(),
            dirty: true,
        };
        // Nothing to write is still a completed flush — left set, the flag
        // holds the run loop's commit guard open on a past deadline.
        sink.flush("dev-test");
        assert!(!sink.dirty);
    }

    /// Feed-by-hand harness: the test pushes wire events through a channel
    /// and decides when the stream ends.
    struct FeedHarness {
        feed: Mutex<Option<mpsc::UnboundedReceiver<AgentEvent>>>,
    }

    #[async_trait::async_trait]
    impl Harness for FeedHarness {
        fn id(&self) -> HarnessId {
            HarnessId::Mock
        }
        fn display_name(&self) -> &str {
            "Feed"
        }
        fn supports_steering(&self) -> bool {
            true
        }
        fn steering_mode(&self) -> zeron_proto::SteeringMode {
            zeron_proto::SteeringMode::StepBoundary
        }
        fn reasoning_levels(&self) -> &[zeron_proto::ReasoningLevel] {
            &[zeron_proto::ReasoningLevel::Medium]
        }
        async fn models(&self) -> Result<Vec<zeron_proto::Model>, zeron_harness::HarnessError> {
            Ok(vec![])
        }
        async fn run(
            &self,
            _request: RunRequest,
            _controls: RunControls,
        ) -> Result<
            futures::stream::BoxStream<'static, Result<AgentEvent, zeron_harness::HarnessError>>,
            zeron_harness::HarnessError,
        > {
            let mut feed = self
                .feed
                .lock()
                .unwrap()
                .take()
                .expect("FeedHarness serves one run per test");
            Ok(futures::stream::poll_fn(move |cx| feed.poll_recv(cx).map(|e| e.map(Ok))).boxed())
        }
    }

    // A tagged subagent event that folds to NO parts used to leave its sink
    // dirty forever: the commit branch's guard stayed true on a deadline in
    // the past and the run loop burned a core until real content arrived.
    #[tokio::test]
    async fn empty_subagent_events_commit_once_per_window_not_per_tick() {
        use super::*;
        FLUSH_TICKS.with(|t| t.set(0));
        let (feed, rx) = mpsc::unbounded_channel();
        let registry = HarnessRegistry::new();
        registry.register(Arc::new(FeedHarness {
            feed: Mutex::new(Some(rx)),
        }));
        let dir = tempfile::tempdir().unwrap();
        let core =
            crate::EngineCore::assemble(dir.path(), Arc::new(registry), HarnessId::Mock, None)
                .unwrap();
        let chat = "chat-sink-spin";
        core.sessions
            .dispatch(chat, HarnessId::Mock, request(), None)
            .await
            .unwrap();
        feed.send(AgentEvent::SessionStarted {
            harness: HarnessId::Mock,
            model: "mock-1".into(),
            tools: vec![],
            cwd: "/tmp".into(),
            session_id: "hs-1".into(),
            assistant_message_id: "a1".into(),
        })
        .unwrap();
        feed.send(AgentEvent::Subagent {
            parent_tool_use_id: "tool-1".into(),
            event: Box::new(AgentEvent::ReasoningDelta {
                text: String::new(),
            }),
        })
        .unwrap();
        // Well past several commit windows with the stream still open: the
        // loop must idle between commits, not spin on a past deadline.
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        let ticks = FLUSH_TICKS.with(|t| t.get());
        assert!(
            (1..=8).contains(&ticks),
            "commit branch fired {ticks} times in 1s for one empty subagent event"
        );
        assert_eq!(
            core.sessions.session_status(chat).map(|s| s.status),
            Some(SessionStatus::Working),
            "stream still open, turn still running"
        );
        feed.send(AgentEvent::Done {
            status: DoneStatus::Completed,
            result: None,
            error: None,
            session_id: None,
        })
        .unwrap();
        core.sessions.shutdown().await;
    }
}
