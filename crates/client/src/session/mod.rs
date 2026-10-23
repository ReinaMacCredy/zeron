//! Sessions: one chat's doc mirror, its incrementally maintained transcript,
//! the composer state, and every command a viewer can issue.
//!
//! Writer discipline (what a viewer writes into the session doc):
//! - `commands`: append-only own entries (run / steer / interrupt /
//!   respondInput) with client-minted message ids for optimistic echo;
//! - `queue`: enqueue + reorder. Edits and removals go through the host
//!   (lease / acknowledgement RPCs) because it serializes them with delivery.
//!
//! The host writes all transcript entries and command outcomes.

mod background;
mod compaction;
mod prediction;
mod review;
mod snapshot;
#[cfg(test)]
mod tests;
pub(crate) mod transcript;
mod host_watch;

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, RwLock, Weak};

use tokio::sync::watch;
use zeron_doc::{
    CommandBasedOn, MessagePart, MessageRole, MessageStatus, QueueDeliveryGate, QueuedMessage,
    SessionCommandEntry, SessionCommandPayload, SessionCommandStatus, SessionDoc,
    SessionMessageEntry,
};
use zeron_proto::{
    ChatIndicator, ContextUsage, RunRequest, SandboxLevel, UserInputAnswer, UserInputQuestion,
    WorktreeSpec, transcript::CompactionWait,
};

pub use background::BackgroundRoster;
pub use review::{NextReviewer, ReviewChip, ReviewSnapshot, model_levels};
pub use snapshot::{
    AppendHint, ComposerState, Entry, HostCapabilities, HostInfo, InputRequest, LiveStatus,
    LocalEcho, PendingKind, PendingSend, QueueGate, QueueItem, RoomState, SessionSnapshot,
    SnapshotDelta,
};
use transcript::{Dirty, Tracker};

use crate::attachments;
use crate::client::ClientInner;
use crate::connectivity::{SendState, send_state};
use crate::error::{ClientError, Result};
use crate::{lock, now_ms, read, write};

/// Coalescing window for remote-import republishes (one display frame).
const REFRESH_FRAME: std::time::Duration = std::time::Duration::from_millis(16);

impl Drop for SessionCore {
    fn drop(&mut self) {
        self.tasks.cancel();
    }
}

/// What to do with a message typed while the agent is working.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum BusyPolicy {
    /// Park it on the shared queue until the turn ends (the default).
    #[default]
    Queue,
    /// Deliver mid-turn: a steer command into the live turn when the
    /// harness steers mid-turn (the host holds it for turn-boundary agents
    /// and never interrupts to deliver it); otherwise it queues.
    Steer,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum QueueAction {
    SendNow,
    Steer,
    Remove,
}

/// Image bytes picked in the composer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutgoingAttachment {
    pub name: String,
    pub mime_type: String,
    pub data: Vec<u8>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct SendRequest {
    pub text: String,
    pub attachments: Vec<OutgoingAttachment>,
    /// Host-side isolated worktree for this run (new-session "New worktree").
    pub worktree: Option<WorktreeSpec>,
    pub busy: BusyPolicy,
}

impl SendRequest {
    pub fn text(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            ..Default::default()
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SendOutcome {
    /// A new turn was requested (`run`); echo id = `message_id`.
    Started { message_id: String },
    /// Delivered into the live turn (`steer`).
    Steered { message_id: String },
    /// Parked on the shared queue.
    Queued { queue_id: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkillFile {
    pub path: String,
    pub text: String,
    pub truncated: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueueEditLease {
    pub row_id: String,
    pub lease_id: String,
    /// Row text at lease time (edit starts from this).
    pub text: String,
    pub base_text_hash: String,
    pub expires_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QueueEditStart {
    Acquired(QueueEditLease),
    /// Another editor holds the row.
    Locked,
    /// The row left the queue.
    Missing,
    /// Host unreachable / capability missing.
    Unavailable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QueueEditAction {
    /// Save the new text.
    Commit,
    /// Keep the old text, release the row.
    Cancel,
    /// Delete the row.
    Discard,
    /// Release without changing anything.
    Release,
}

impl QueueEditAction {
    fn wire(self) -> &'static str {
        match self {
            QueueEditAction::Commit => "commit",
            QueueEditAction::Cancel => "cancel",
            QueueEditAction::Discard => "discard",
            QueueEditAction::Release => "releaseUnchanged",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QueueEditFinish {
    Finished,
    /// The row changed under the lease (hash mismatch).
    Conflict,
    Missing,
    /// The lease was superseded or expired.
    Lost,
    Unavailable,
}

/// Mutable per-session state behind one lock.
struct CoreState {
    tracker: Tracker,
    queue: Vec<QueuedMessage>,
    commands: Vec<SessionCommandEntry>,
    context_usage: Option<ContextUsage>,
    /// Stable echo entries by message id.
    echoes: HashMap<String, Arc<Entry>>,
    pending: Vec<PendingSend>,
    /// Delivery-grace clock per message id (retry restarts it).
    grace_started: HashMap<String, i64>,
    queue_actions_pending: HashSet<String>,
    queue_error: Option<String>,
    last_submitted: Option<String>,
    transfer_progress: Option<f64>,
    hydrated: bool,
    transcript_revision: u64,
    composer_revision: u64,
    token_rate: Option<u32>,
    compaction: Option<(CompactionWait, i64)>,
    /// This device's own `CompactChat` is in flight.
    compacting: bool,
    kept_full_history: Option<i64>,
    prediction: Option<zeron_proto::prediction::ComposerPrediction>,
    session: Option<zeron_proto::Session>,
    shell_output: Option<(String, Vec<String>)>,
    review: review::ReviewState,
}

pub(crate) struct SessionCore {
    pub(crate) chat_id: String,
    client: Weak<ClientInner>,
    /// Live chat2 room (declared before `doc`: it drops — and flushes — first).
    room: Mutex<Option<Arc<crate::live::room::Room>>>,
    /// Verified cursor restored with the local snapshot (room join cursor).
    initial_cursor: u64,
    refresh_wake: Arc<tokio::sync::Notify>,
    tasks: tokio_util::sync::CancellationToken,
    doc: Arc<SessionDoc>,
    write_gate: Mutex<()>,
    dirty: Arc<Mutex<Dirty>>,
    _subscription: loro::Subscription,
    state: Mutex<CoreState>,
    transcript_tx: watch::Sender<Arc<SessionSnapshot>>,
    composer: RwLock<Arc<ComposerState>>,
    /// Serializes composer recomputes: each reads its inputs and publishes
    /// under it, so an older read can't land after a newer one.
    recompute_gate: Mutex<()>,
    view_attached: AtomicBool,
    rate_watch: Mutex<Option<tokio_util::sync::CancellationToken>>,
    prediction_watch: Mutex<Option<tokio_util::sync::CancellationToken>>,
    session_watch: Mutex<Option<tokio_util::sync::CancellationToken>>,
    output_watch: Mutex<Option<tokio_util::sync::CancellationToken>>,
    /// Last open/attach/detach (warm-set eviction order).
    touched_ms: std::sync::atomic::AtomicI64,
}

impl SessionCore {
    pub(crate) fn new(
        chat_id: &str,
        client: &Arc<ClientInner>,
        doc: SessionDoc,
        cursor: u64,
    ) -> Arc<Self> {
        let doc = Arc::new(doc);
        let dirty = Arc::new(Mutex::new(Dirty::all()));
        let sink = dirty.clone();
        let subscription = doc.doc().subscribe_root(Arc::new(move |event| {
            lock(&sink).observe(&event);
        }));
        let (transcript_tx, _) = watch::channel(Arc::new(SessionSnapshot {
            chat_id: chat_id.to_owned(),
            ..Default::default()
        }));
        let composer = Arc::new(empty_composer(chat_id));
        let refresh_wake = Arc::new(tokio::sync::Notify::new());
        let tasks = client.cancel.child_token();
        let core = Arc::new(Self {
            chat_id: chat_id.to_owned(),
            client: Arc::downgrade(client),
            room: Mutex::new(None),
            initial_cursor: cursor,
            refresh_wake: refresh_wake.clone(),
            tasks: tasks.clone(),
            doc,
            write_gate: Mutex::new(()),
            dirty,
            _subscription: subscription,
            state: Mutex::new(CoreState {
                tracker: Tracker::new(),
                queue: Vec::new(),
                commands: Vec::new(),
                context_usage: None,
                echoes: HashMap::new(),
                pending: Vec::new(),
                grace_started: HashMap::new(),
                queue_actions_pending: HashSet::new(),
                queue_error: None,
                last_submitted: None,
                transfer_progress: None,
                hydrated: false,
                transcript_revision: 0,
                composer_revision: 0,
                token_rate: None,
                compaction: None,
                compacting: false,
                kept_full_history: None,
                prediction: None,
                session: None,
                shell_output: None,
                review: review::ReviewState::default(),
            }),
            transcript_tx,
            composer: RwLock::new(composer),
            recompute_gate: Mutex::new(()),
            view_attached: AtomicBool::new(false),
            rate_watch: Mutex::new(None),
            prediction_watch: Mutex::new(None),
            session_watch: Mutex::new(None),
            output_watch: Mutex::new(None),
            touched_ms: std::sync::atomic::AtomicI64::new(now_ms()),
        });
        // Coalesced republish for remote imports (a backfill of N rows costs
        // ~one refresh per frame, not N).
        let weak = Arc::downgrade(&core);
        crate::runtime::shared().spawn(async move {
            loop {
                tokio::select! {
                    _ = tasks.cancelled() => return,
                    _ = refresh_wake.notified() => {}
                }
                let Some(core) = weak.upgrade() else { return };
                core.refresh();
                drop(core);
                tokio::time::sleep(REFRESH_FRAME).await;
            }
        });
        core
    }

    /// Remote change landed: refresh on the next frame.
    pub(crate) fn schedule_refresh(&self) {
        self.refresh_wake.notify_one();
    }

    pub(crate) fn room(&self) -> Option<Arc<crate::live::room::Room>> {
        lock(&self.room).clone()
    }

    /// Drop the room (deleted chat): its tasks stop and it flushes once.
    pub(crate) fn close_room(&self) {
        let room = lock(&self.room).take();
        drop(room);
    }

    /// Live: join the chat's chat2 room once its row says it's dialable
    /// (roomGen ≥ 2). Idempotent.
    pub(crate) fn ensure_room(self: &Arc<Self>, client: &Arc<ClientInner>) {
        let Some(live) = client.live() else { return };
        if lock(&self.room).is_some() {
            return;
        }
        let Some(chat) = client.workspace.chat(&self.chat_id) else {
            return;
        };
        if chat.room_gen.unwrap_or(1) < 2 {
            // A retired legacy (s2) room: nothing to dial. Show what's local
            // instead of an endless loader.
            let newly = !std::mem::replace(&mut lock(&self.state).hydrated, true);
            if newly {
                self.schedule_refresh();
            }
            return;
        }
        let core = Arc::downgrade(self);
        let on_applied: Arc<dyn Fn() + Send + Sync> = Arc::new(move || {
            if let Some(core) = core.upgrade() {
                if core
                    .room()
                    .is_some_and(|r| r.status.caught_up.load(Ordering::Acquire))
                {
                    let newly = {
                        let mut st = lock(&core.state);
                        !std::mem::replace(&mut st.hydrated, true)
                    };
                    if newly {
                        tracing::debug!(chat = %core.chat_id, "session hydrated from the room");
                    }
                }
                core.schedule_refresh();
            }
        });
        let weak_client = Arc::downgrade(client);
        let status_core = Arc::downgrade(self);
        let on_status: Arc<dyn Fn() + Send + Sync> = Arc::new(move || {
            if let Some(client) = weak_client.upgrade() {
                client.recompute_connectivity();
                if let Some(core) = status_core.upgrade() {
                    core.recompute_composer(&client);
                }
            }
        });
        let room = crate::live::room::Room::start(
            &self.doc,
            &self.chat_id,
            self.initial_cursor,
            crate::live::room::RoomDeps {
                bearer: crate::live::LiveBackend::bearer(client),
                edge: live.edge.clone(),
                device_id: client.config.device_id.clone(),
                host_device_id: chat.device_id.clone(),
                store: live.store.clone(),
                on_applied,
                on_status,
                cancel: client.cancel.clone(),
            },
        );
        let mut slot = lock(&self.room);
        if slot.is_none() {
            *slot = Some(room);
        }
    }

    /// Persist the doc now (backgrounding).
    pub(crate) fn flush(&self) {
        if let Some(room) = self.room() {
            room.flush();
        }
    }

    pub(crate) fn doc(&self) -> &Arc<SessionDoc> {
        &self.doc
    }

    fn client(&self) -> Result<Arc<ClientInner>> {
        self.client.upgrade().ok_or(ClientError::Closed)
    }

    pub(crate) fn set_hydrated(&self) {
        lock(&self.state).hydrated = true;
    }

    /// Serialized doc write, then an incremental refresh.
    pub(crate) fn write<R>(
        &self,
        f: impl FnOnce(&SessionDoc) -> std::result::Result<R, zeron_doc::DocError>,
    ) -> Result<R> {
        let result = {
            let _gate = lock(&self.write_gate);
            let client = self.client()?;
            if let Some(live) = client.live() {
                // A new chat's registry row must survive alongside its outbox:
                // recovery needs its host and room generation before any UI opens it.
                live.persist_registry(&client)?;
            }
            let result = f(&self.doc)?;
            if let Some(live) = client.live() {
                live.store
                    .schedule_sync_job(&self.chat_id, crate::live::delivery::JOB)
                    .map_err(|err| ClientError::Storage(err.to_string()))?;
                live.delivery_wake.notify_one();
            }
            result
        };
        self.refresh();
        Ok(result)
    }

    pub(crate) fn snapshot(&self) -> Arc<SessionSnapshot> {
        self.transcript_tx.borrow().clone()
    }

    pub(crate) fn subscribe(&self) -> watch::Receiver<Arc<SessionSnapshot>> {
        self.transcript_tx.subscribe()
    }

    pub(crate) fn composer(&self) -> Arc<ComposerState> {
        read(&self.composer).clone()
    }

    /// Oldest unadopted send (feeds the workspace row badge).
    pub(crate) fn send_state(&self) -> Option<SendState> {
        oldest_state(&lock(&self.state).pending)
    }

    pub(crate) fn ask(&self) -> Option<crate::workspace::Ask> {
        let request = self.composer().open_input.clone()?;
        Some(
            if self.snapshot().input_answers.contains_key(&request.request_id) {
                crate::workspace::Ask::Answered(request.request_id)
            } else {
                crate::workspace::Ask::Open(request)
            },
        )
    }

    pub(crate) fn has_pending_sends(&self) -> bool {
        !lock(&self.state).pending.is_empty()
            || self.room().is_some_and(|room| room.has_pending_pushes())
    }

    /// Apply queued doc events + time-driven state; publish what changed.
    pub(crate) fn refresh(&self) {
        let Ok(client) = self.client() else { return };
        let dirty = std::mem::take(&mut *lock(&self.dirty));
        let now = now_ms();
        let degraded = client.chat_delivery_degraded(&self.chat_id);
        let (indicator_working, working_since_ms, compacting, classifying) = client
            .workspace
            .snapshot()
            .session(&self.chat_id)
            .map_or((false, None, false, None), |row| {
                (
                    row.host_indicator == ChatIndicator::Working && !row.held_by_subagents,
                    row.working_since_ms,
                    row.compacting,
                    row.classifying.clone(),
                )
            });
        let mut transcript_event = None;
        let ask_before = self.ask();
        let send_before;
        let send_after;
        {
            let mut st = lock(&self.state);
            send_before = oldest_state(&st.pending);
            let change = st.tracker.refresh(self.doc.doc(), &dirty);
            if dirty.queue {
                st.queue = self.doc.read_queue().unwrap_or_default();
            }
            if dirty.commands {
                st.commands = self.doc.read_commands().unwrap_or_default();
            }
            if dirty.meta {
                st.context_usage = self.doc.context_usage();
            }
            let echoes_changed = derive_pending(&mut st, &client.config.device_id, degraded, now);
            send_after = oldest_state(&st.pending);
            let previous = self.snapshot();
            let streaming = st
                .tracker
                .entries()
                .last()
                .is_some_and(|e| e.is_streaming());
            let working = indicator_working || streaming;
            let wait = if compacting {
                Some(CompactionWait::Compacting(
                    zeron_doc::schema::boundary_compaction(
                        st.tracker.entries().iter().map(|e| &*e.message),
                    ),
                ))
            } else {
                classifying.map(|run| CompactionWait::Classifying {
                    harness: run.harness,
                    model: run.model,
                    agent_note: false,
                })
            };
            st.compaction = match (wait, st.compaction.take()) {
                (Some(wait), Some((last, since))) if wait == last => Some((wait, since)),
                (wait, _) => wait.map(|wait| (wait, now)),
            };
            let live = LiveFlags {
                working,
                working_since_ms,
                token_rate: st.token_rate.filter(|_| working),
                limits_rev: client.workspace.limits_rev(),
                compaction: st.compaction.clone(),
            };
            let live_changed = previous.working != live.working
                || previous.compaction != live.compaction
                || previous.working_since_ms != live.working_since_ms
                || previous.token_rate != live.token_rate
                || previous.limits_rev != live.limits_rev
                || previous.hydrated != st.hydrated;
            let answers = sent_answers(&st.commands, now);
            let answers_changed = *previous.input_answers != answers;
            if change.is_some()
                || echoes_changed
                || dirty.meta
                || live_changed
                || answers_changed
                || st.transcript_revision == 0
            {
                st.transcript_revision += 1;
                let answers = if answers_changed {
                    Arc::new(answers)
                } else {
                    previous.input_answers.clone()
                };
                let snapshot = build_snapshot(&self.chat_id, &st, change, live, answers, &previous);
                transcript_event = Some(snapshot.revision);
                self.transcript_tx.send_replace(Arc::new(snapshot));
            }
        }
        if let Some(revision) = transcript_event {
            client.events.session(&self.chat_id, revision);
        }
        self.recompute_composer(&client);
        if send_before != send_after || ask_before != self.ask() {
            client.recompute_workspace();
        }
        if self.view_attached() {
            self.sync_review(&client);
        }
    }

    /// Rebuild the composer view (also after workspace/connectivity changes).
    pub(crate) fn recompute_composer(&self, client: &Arc<ClientInner>) {
        let _gate = lock(&self.recompute_gate);
        let workspace = client.workspace.snapshot();
        let row = workspace.session(&self.chat_id).cloned();
        let snapshot = self.snapshot();
        let degraded = client.chat_delivery_degraded(&self.chat_id);
        let room = client.room_state(&self.chat_id);
        let mut st = lock(&self.state);
        let host_id = row
            .as_ref()
            .map(|r| r.device_id.clone())
            .unwrap_or_default();
        let device = workspace.device(&host_id);
        let capabilities =
            client.host_capabilities(&host_id, row.as_ref().and_then(|r| r.harness.as_deref()));
        let indicator = row.as_ref().map_or(ChatIndicator::Idle, |r| r.indicator);
        let host_indicator = row
            .as_ref()
            .map_or(ChatIndicator::Idle, |r| r.host_indicator);
        let busy = matches!(
            host_indicator,
            ChatIndicator::Working | ChatIndicator::AwaitingInput
        );
        let parked = row.as_ref().is_some_and(|r| r.held_by_subagents);
        let turn_running =
            (host_indicator == ChatIndicator::Working && !parked) || snapshot.streaming;
        let compacting = st.compacting || row.as_ref().is_some_and(|r| r.compacting);
        let compact_blocked = compaction::compact_blocked(compacting, busy || snapshot.streaming);
        let resume_compaction = compact_blocked
            .is_none()
            .then(|| {
                compaction::resume_offer(
                    row.as_deref(),
                    st.context_usage.and_then(|u| u.tokens),
                    st.kept_full_history,
                )
            })
            .flatten();
        let mut next = ComposerState {
            chat_id: self.chat_id.clone(),
            revision: 0,
            title: row
                .as_ref()
                .map_or_else(|| "New session".into(), |r| r.title.clone()),
            host: HostInfo {
                device_id: host_id.clone(),
                name: device.map(|d| d.name.clone()),
                online: device.is_some_and(|d| d.online),
                capabilities,
            },
            live: LiveStatus {
                indicator,
                turn_running,
                working_since_ms: row.as_ref().and_then(|r| r.working_since_ms),
                streaming: snapshot.streaming,
                can_interrupt: busy || snapshot.streaming,
                resumable: !busy
                    && !matches!(
                        indicator,
                        ChatIndicator::Working | ChatIndicator::AwaitingInput
                    )
                    && snapshot
                        .transcript()
                        .last()
                        .is_some_and(|entry| zeron_doc::schema::turn_stopped(&entry.message)),
            },
            queue: st
                .queue
                .iter()
                .map(|item| {
                    let mut queued =
                        queue_item(item, &client.config.device_id, &st.queue_actions_pending);
                    queued.from_label = zeron_proto::transcript::queue_from_label(
                        &self.chat_id,
                        item.from_chat_id.as_deref(),
                        &item.text,
                        |id| client.workspace.chat(id).and_then(|chat| chat.title),
                    );
                    queued
                })
                .collect(),
            pending_sends: st.pending.clone(),
            send_state: oldest_state(&st.pending),
            delivery_degraded: degraded,
            room,
            transfer_progress: st.transfer_progress,
            open_input: open_input(st.tracker.entries()),
            queue_error: st.queue_error.clone(),
            last_submitted_message_id: st.last_submitted.clone(),
            context_usage: st.context_usage,
            limit: client.workspace.limit(&self.chat_id),
            compacting,
            compact_blocked,
            resume_compaction,
            prediction: st.prediction.as_ref().map(|p| p.text.clone()),
            review: review::chip(&st, review::target(client, &self.chat_id).as_ref()),
            staged_comments: review::staged(&st),
            review_revision: review::revision(&st),
        };
        let current = self.composer();
        next.revision = current.revision;
        if *current == next {
            return;
        }
        st.composer_revision += 1;
        next.revision = st.composer_revision;
        drop(st);
        let revision = next.revision;
        *write(&self.composer) = Arc::new(next);
        client.events.composer(&self.chat_id, revision);
    }

    fn update_state(&self, f: impl FnOnce(&mut CoreState)) {
        f(&mut lock(&self.state));
        if let Ok(client) = self.client() {
            self.recompute_composer(&client);
        }
    }

    #[allow(dead_code)] // attachment escort progress (live uploads)
    pub(crate) fn set_transfer_progress(&self, progress: Option<f64>) {
        self.update_state(|st| st.transfer_progress = progress);
    }

    fn last_entry_id(&self) -> Option<String> {
        self.snapshot()
            .entries
            .get(..self.snapshot().transcript_len)
            .and_then(|e| e.last())
            .map(|e| e.id.clone())
    }

    fn queue_command(&self, device_id: &str, payload: SessionCommandPayload) -> Result<String> {
        let now = now_ms();
        let message = matches!(
            payload,
            SessionCommandPayload::Run { .. } | SessionCommandPayload::Steer { .. }
        );
        let entry = SessionCommandEntry {
            id: crate::new_id(),
            payload,
            issued_by: device_id.to_owned(),
            issued_at: now,
            based_on: self.last_entry_id().map(|turn_id| CommandBasedOn {
                turn_id: Some(turn_id),
                frontier: None,
            }),
            expires_at: Some(now + zeron_doc::COMMAND_DEFAULT_TTL_MS),
            status: SessionCommandStatus::Pending,
            resolution: None,
            computer_generation: message.then(|| self.doc.computer_generation()),
        };
        let id = entry.id.clone();
        self.write(|doc| doc.queue_command(&entry))?;
        Ok(id)
    }

    pub(crate) fn view_attached(&self) -> bool {
        self.view_attached.load(Ordering::Acquire)
    }

    pub(crate) fn touch(&self) {
        self.touched_ms.store(now_ms(), Ordering::Release);
    }

    pub(crate) fn touched_ms(&self) -> i64 {
        self.touched_ms.load(Ordering::Acquire)
    }

    pub(crate) fn set_token_rate(&self, rate: Option<u32>) {
        let changed = std::mem::replace(&mut lock(&self.state).token_rate, rate) != rate;
        if changed {
            self.schedule_refresh();
        }
    }

    fn follow_token_rate(self: &Arc<Self>, on: bool) {
        let mut slot = lock(&self.rate_watch);
        if let Some(cancel) = slot.take() {
            cancel.cancel();
        }
        if !on {
            drop(slot);
            self.set_token_rate(None);
            return;
        }
        let Ok(client) = self.client() else { return };
        let Some(live) = client.live() else { return };
        let Some(chat) = client.workspace.chat(&self.chat_id) else {
            return;
        };
        let cancel = self.tasks.child_token();
        *slot = Some(cancel.clone());
        drop(slot);
        let weak = Arc::downgrade(self);
        live.relay.follow(
            chat.device_id.clone(),
            zeron_rpc::methods::WATCH_TOKEN_RATE,
            serde_json::json!({ "chatId": self.chat_id }),
            cancel,
            move |item| {
                let Some(core) = weak.upgrade() else {
                    return false;
                };
                let rate = item["rate"].as_u64().map(|rate| rate as u32);
                core.set_token_rate(rate);
                true
            },
            || {},
        );
    }
}

fn host_reason(err: ClientError) -> ClientError {
    match err {
        ClientError::HostError(message) => ClientError::HostError(
            message
                .split_once(": ")
                .map_or(message.as_str(), |(_, reason)| reason)
                .to_owned(),
        ),
        other => other,
    }
}

fn empty_composer(chat_id: &str) -> ComposerState {
    ComposerState {
        chat_id: chat_id.to_owned(),
        revision: 0,
        title: "New session".into(),
        host: HostInfo {
            device_id: String::new(),
            name: None,
            online: false,
            capabilities: HostCapabilities::default(),
        },
        live: LiveStatus {
            indicator: ChatIndicator::Idle,
            turn_running: false,
            working_since_ms: None,
            streaming: false,
            can_interrupt: false,
            resumable: false,
        },
        queue: Vec::new(),
        pending_sends: Vec::new(),
        send_state: None,
        delivery_degraded: false,
        room: RoomState::default(),
        transfer_progress: None,
        open_input: None,
        queue_error: None,
        last_submitted_message_id: None,
        context_usage: None,
        limit: None,
        compacting: false,
        compact_blocked: None,
        resume_compaction: None,
        prediction: None,
        review: None,
        staged_comments: Vec::new(),
        review_revision: 0,
    }
}

fn oldest_state(pending: &[PendingSend]) -> Option<SendState> {
    pending.iter().min_by_key(|p| p.sent_at_ms).map(|oldest| {
        if pending.iter().any(|p| p.state == SendState::Failed) {
            SendState::Failed
        } else {
            oldest.state
        }
    })
}

fn queue_item(item: &QueuedMessage, device_id: &str, pending: &HashSet<String>) -> QueueItem {
    QueueItem {
        id: item.id.clone(),
        text: item.text.clone(),
        visible_text: attachments::queue_visible_text(&item.text, &item.attachments),
        attachments: item.attachments.clone(),
        hold_for_turn_end: item.hold_for_turn_end,
        issued_by: item.issued_by.clone(),
        from_this_device: item.issued_by == device_id,
        issued_at_ms: item.issued_at,
        edited_at_ms: item.edited_at,
        gate: item.delivery_gate.as_ref().map(|gate| match gate {
            QueueDeliveryGate::Editing {
                owner_device_id,
                expires_at_ms,
                ..
            } => QueueGate::Editing {
                owner_device_id: owner_device_id.clone(),
                expires_at_ms: *expires_at_ms,
                mine: owner_device_id == device_id,
            },
            QueueDeliveryGate::ReviewRequired {
                owner_device_id, ..
            } => QueueGate::ReviewRequired {
                owner_device_id: owner_device_id.clone(),
            },
        }),
        action_pending: pending.contains(&item.id),
        from_label: None,
    }
}

/// The newest unresolved question with at least one question to answer.
fn open_input(entries: &[Arc<Entry>]) -> Option<InputRequest> {
    for entry in entries.iter().rev() {
        for part in entry.message.parts.iter().rev() {
            if let MessagePart::Input {
                request_id,
                questions,
                resolved: false,
                ..
            } = part
                && !questions.is_empty()
                && !questions.iter().any(UserInputQuestion::is_consent)
            {
                return Some(InputRequest {
                    entry_id: entry.id.clone(),
                    request_id: request_id.clone(),
                    questions: questions.clone(),
                });
            }
        }
    }
    None
}

/// Own run/steer commands whose message hasn't landed = pending echoes.
/// Returns whether the echo set/states changed.
fn derive_pending(st: &mut CoreState, device_id: &str, degraded: bool, now: i64) -> bool {
    struct Attempt {
        text: String,
        kind: PendingKind,
        first_issued: i64,
        live: bool,
        dead: bool,
    }
    let mut order: Vec<String> = Vec::new();
    let mut attempts: HashMap<String, Attempt> = HashMap::new();
    for command in &st.commands {
        if command.issued_by != device_id {
            continue;
        }
        let (message_id, text, kind) = match &command.payload {
            SessionCommandPayload::Run {
                request,
                message_id,
            } => (message_id, &request.prompt, PendingKind::Run),
            SessionCommandPayload::Steer {
                prompt,
                message_id: Some(message_id),
            } => (message_id, prompt, PendingKind::Steer),
            _ => continue,
        };
        if st.tracker.contains_id(message_id) {
            continue;
        }
        let expired = now >= command.effective_expiry();
        let (live, dead) = match command.status {
            SessionCommandStatus::Pending => (!expired, expired),
            SessionCommandStatus::Applied => (true, false),
            SessionCommandStatus::Rejected
            | SessionCommandStatus::Expired
            | SessionCommandStatus::Superseded => (false, true),
            SessionCommandStatus::Cancelled => continue,
        };
        let attempt = attempts.entry(message_id.clone()).or_insert_with(|| {
            order.push(message_id.clone());
            Attempt {
                text: text.clone(),
                kind,
                first_issued: command.issued_at,
                live: false,
                dead: false,
            }
        });
        attempt.live |= live;
        attempt.dead |= dead;
    }
    let mut next = Vec::with_capacity(order.len());
    for message_id in order {
        let attempt = &attempts[&message_id];
        let started = st
            .grace_started
            .get(&message_id)
            .copied()
            .unwrap_or(attempt.first_issued);
        let parsed = attachments::parse_user_message(&attempt.text);
        next.push(PendingSend {
            state: send_state(started, degraded, attempt.dead && !attempt.live, now),
            message_id,
            text: attempt.text.clone(),
            visible_text: parsed.text,
            images: parsed.images.into_iter().map(|i| i.path).collect(),
            kind: attempt.kind,
            sent_at_ms: attempt.first_issued,
        });
    }
    let live_ids: HashSet<&str> = next.iter().map(|p| p.message_id.as_str()).collect();
    st.grace_started
        .retain(|id, _| live_ids.contains(id.as_str()));
    let changed = next != st.pending;
    if changed {
        // Rebuild echo entries, reusing unchanged Arcs.
        let mut echoes = HashMap::with_capacity(next.len());
        for pending in &next {
            let reuse = st.echoes.get(&pending.message_id).filter(|e| {
                e.echo.as_ref().map(|x| x.state) == Some(pending.state)
                    && matches!(e.message.parts.first(), Some(MessagePart::Text { text, .. }) if *text == pending.text)
            });
            let entry = match reuse {
                Some(entry) => entry.clone(),
                None => {
                    let rev = st.tracker.next_rev();
                    Arc::new(Entry {
                        id: pending.message_id.clone(),
                        rev,
                        message: Arc::new(SessionMessageEntry {
                            id: pending.message_id.clone(),
                            role: MessageRole::User,
                            parts: vec![MessagePart::Text {
                                id: "t0".into(),
                                text: pending.text.clone(),
                            }],
                            created_at: pending.sent_at_ms,
                            device_id: device_id.to_owned(),
                            status: Some(MessageStatus::Complete),
                            continuation_of: None,
                            duration_ms: None,
                            from_chat_id: None,
                            fork_import: false,
                            answers_request_id: None,
                        }),
                        echo: Some(LocalEcho {
                            state: pending.state,
                            sent_at_ms: pending.sent_at_ms,
                        }),
                        append: None,
                    })
                }
            };
            echoes.insert(pending.message_id.clone(), entry);
        }
        st.echoes = echoes;
        st.pending = next;
    }
    changed
}

#[derive(Clone)]
struct LiveFlags {
    working: bool,
    working_since_ms: Option<i64>,
    token_rate: Option<u32>,
    limits_rev: u64,
    compaction: Option<(CompactionWait, i64)>,
}

fn sent_answers(
    commands: &[SessionCommandEntry],
    now: i64,
) -> HashMap<String, Vec<UserInputAnswer>> {
    commands
        .iter()
        .filter(|command| match command.status {
            SessionCommandStatus::Pending => now < command.effective_expiry(),
            SessionCommandStatus::Applied => true,
            _ => false,
        })
        .filter_map(|command| match &command.payload {
            SessionCommandPayload::RespondInput {
                request_id,
                answers,
            } => Some((request_id.clone(), answers.clone())),
            _ => None,
        })
        .collect()
}

fn build_snapshot(
    chat_id: &str,
    st: &CoreState,
    change: Option<transcript::TranscriptChange>,
    live: LiveFlags,
    input_answers: Arc<HashMap<String, Vec<UserInputAnswer>>>,
    previous: &SessionSnapshot,
) -> SessionSnapshot {
    let transcript = st.tracker.entries();
    let mut entries: Vec<Arc<Entry>> = Vec::with_capacity(transcript.len() + st.pending.len());
    entries.extend(transcript.iter().cloned());
    let transcript_len = entries.len();
    for pending in &st.pending {
        if let Some(echo) = st.echoes.get(&pending.message_id) {
            entries.push(echo.clone());
        }
    }
    let index: HashMap<String, usize> = entries
        .iter()
        .enumerate()
        .map(|(i, e)| (e.id.clone(), i))
        .collect();
    let streaming = transcript.last().is_some_and(|e| e.is_streaming());
    let mut snapshot = SessionSnapshot {
        chat_id: chat_id.to_owned(),
        revision: st.transcript_revision,
        entries,
        transcript_len,
        streaming,
        working: live.working,
        working_since_ms: live.working_since_ms,
        token_rate: live.token_rate,
        limits_rev: live.limits_rev,
        compaction: live.compaction,
        pending: st.pending.clone(),
        context_usage: st.context_usage,
        hydrated: st.hydrated,
        delta: SnapshotDelta::default(),
        index: Arc::new(index),
        input_answers,
    };
    snapshot.delta = match change {
        Some(change) if change.reset => SnapshotDelta {
            reset: true,
            changed: snapshot.entries.iter().map(|e| e.id.clone()).collect(),
            removed: Vec::new(),
        },
        _ => snapshot.changes_since(previous),
    };
    snapshot
}

/// Stops a [`SessionHandle::watch`] when dropped.
#[derive(Debug)]
pub struct SnapshotWatch {
    cancel: tokio_util::sync::CancellationToken,
}

impl Drop for SnapshotWatch {
    fn drop(&mut self) {
        self.cancel.cancel();
    }
}

/// One open chat. Cheap to clone; all clones share the session.
#[derive(Clone)]
pub struct SessionHandle {
    pub(crate) core: Arc<SessionCore>,
}

impl std::fmt::Debug for SessionHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SessionHandle")
            .field("chat_id", &self.core.chat_id)
            .finish()
    }
}

impl SessionHandle {
    pub fn chat_id(&self) -> &str {
        &self.core.chat_id
    }

    /// The current transcript (O(1): an `Arc` clone).
    pub fn snapshot(&self) -> Arc<SessionSnapshot> {
        self.core.snapshot()
    }

    /// Transcript subscription for Rust consumers (the layout engine). The
    /// watch coalesces: after `changed()`, diff with
    /// [`SessionSnapshot::changes_since`] against the last snapshot you
    /// processed rather than trusting `delta` alone.
    pub fn subscribe(&self) -> watch::Receiver<Arc<SessionSnapshot>> {
        self.core.subscribe()
    }

    /// Callback-style subscription: `f` gets the current snapshot at once,
    /// then every newer one (coalesced — diff with
    /// [`SessionSnapshot::changes_since`]). Runs on a client runtime thread;
    /// dropping the returned guard stops it.
    pub fn watch(&self, f: impl Fn(Arc<SessionSnapshot>) + Send + Sync + 'static) -> SnapshotWatch {
        let mut rx = self.subscribe();
        let cancel = tokio_util::sync::CancellationToken::new();
        let token = cancel.clone();
        crate::runtime::shared().spawn(async move {
            let first = rx.borrow_and_update().clone();
            f(first);
            loop {
                tokio::select! {
                    _ = token.cancelled() => return,
                    changed = rx.changed() => if changed.is_err() { return },
                }
                let next = rx.borrow_and_update().clone();
                f(next);
            }
        });
        SnapshotWatch { cancel }
    }

    pub fn composer(&self) -> Arc<ComposerState> {
        self.core.composer()
    }

    pub fn session_stats(&self) -> Option<zeron_proto::SessionStats> {
        self.core.doc().session_stats()
    }

    /// Tell the client whether a transcript view is on screen (marks seen on
    /// attach; prioritizes this room's sync in live mode).
    pub fn set_view_attached(&self, attached: bool) {
        let was = self.core.view_attached.swap(attached, Ordering::AcqRel);
        self.core.touch();
        if attached
            && !was
            && let Some(room) = self.core.room()
        {
            // On screen: verify the room is live now rather than trusting a
            // quiet socket (the 15-min quiet probe is far too slow for a
            // chat someone is looking at).
            room.kick();
        }
        if attached && !was {
            self.mark_seen();
            if let Ok(client) = self.core.client() {
                self.core.sync_review(&client);
            }
            *lock(&self.core.prediction_watch) = prediction::start(&self.core);
            *lock(&self.core.session_watch) = background::start(&self.core);
        }
        if !attached && was {
            for watch in [
                &self.core.prediction_watch,
                &self.core.session_watch,
                &self.core.output_watch,
            ] {
                if let Some(watch) = lock(watch).take() {
                    watch.cancel();
                }
            }
            self.core.update_state(|st| {
                st.prediction = None;
                st.session = None;
                st.shell_output = None;
            });
        }
        if attached != was {
            self.core.follow_token_rate(attached);
        }
    }

    pub fn mark_seen(&self) {
        if let Ok(client) = self.core.client() {
            client.mark_seen(&self.core.chat_id);
        }
    }

    /// Send a message: a new turn when idle; while the agent works, park it
    /// on the shared queue (or steer, per `busy`). Attachments ride
    /// `pending://` refs; their bytes are escorted to the host.
    pub fn send(&self, request: SendRequest) -> Result<SendOutcome> {
        let client = self.core.client()?;
        let chat = client
            .workspace
            .chat(&self.core.chat_id)
            .ok_or_else(|| ClientError::NotFound(self.core.chat_id.clone()))?;
        let composer = self.composer();
        let staged = composer.staged_comments.clone();
        if request.text.trim().is_empty() && request.attachments.is_empty() && staged.is_empty() {
            return Err(ClientError::InvalidArgument("empty message".into()));
        }
        // A turn is running, or a Run of ours is about to start one on a
        // reachable host (don't double-start in that ~RTT window).
        let starting = composer
            .pending_sends
            .iter()
            .any(|p| p.kind == PendingKind::Run && p.state == SendState::Sending);
        let busy = composer.live.turn_running || starting;
        let caps = &composer.host.capabilities;
        if !request.attachments.is_empty()
            && !composer.host.capabilities.queued_attachments
            && !client.is_demo()
        {
            return Err(ClientError::Unsupported(
                "the host is too old for image attachments — update Zeron on it".into(),
            ));
        }
        let refs =
            client.stage_attachments(&self.core.chat_id, &chat.device_id, &request.attachments)?;
        let prompt_text = zeron_proto::comments::with_comments(request.text.trim_end(), &staged);
        let device_id = client.config.device_id.clone();

        // Steering is a Steer command, never a queue row: the host no longer
        // steers rows on its own, and the row's only other exit on mobile was
        // an interrupting Send now, which killed the live turn (and a Codex
        // app-server's subagents with it).
        let steer = busy
            && request.busy == BusyPolicy::Steer
            && caps.mid_turn_steering == Some(true)
            && refs.is_empty();
        let outcome = if !steer
            && busy
            && caps.message_queue
            && (refs.is_empty() || caps.queue_attachments)
        {
            let text = if refs.is_empty() || caps.clean_attachment_text {
                if prompt_text.is_empty() {
                    attachments::attachment_only_text(&refs).to_owned()
                } else {
                    prompt_text.clone()
                }
            } else {
                attachments::with_attachments(&prompt_text, &refs)
            };
            let id = self.enqueue_inner(
                &device_id,
                &text,
                refs.clone(),
                request.busy == BusyPolicy::Queue,
            )?;
            SendOutcome::Queued { queue_id: id }
        } else {
            let content = attachments::with_attachments(&prompt_text, &refs);
            let message_id = crate::new_id();
            if busy {
                self.core.queue_command(
                    &device_id,
                    SessionCommandPayload::Steer {
                        prompt: content,
                        message_id: Some(message_id.clone()),
                    },
                )?;
                SendOutcome::Steered { message_id }
            } else {
                let config = chat.config.as_ref();
                let request = RunRequest {
                    prompt: content,
                    harness: config.map(|c| c.harness),
                    model: config.and_then(|c| c.model.clone()),
                    reasoning: config.and_then(|c| c.reasoning),
                    model_options: config.map(|c| c.model_options.clone()).unwrap_or_default(),
                    cwd: chat.cwd.clone().unwrap_or_else(|| "~".into()),
                    sandbox: config.map_or(SandboxLevel::WorkspaceWrite, |c| c.sandbox),
                    auto_approve: false,
                    resume: None,
                    attachments: refs.clone(),
                    worktree: request.worktree.clone(),
                    mcp: None,
                    branch: None,
                    forked_at: None,
                    computer_control: false,
                    permission_mode: config.map(|c| c.permission_mode).unwrap_or_default(),
                    disabled_tools: config.map(|c| c.disabled_tools.clone()).unwrap_or_default(),
                };
                self.core.queue_command(
                    &device_id,
                    SessionCommandPayload::Run {
                        request,
                        message_id: message_id.clone(),
                    },
                )?;
                SendOutcome::Started { message_id }
            }
        };
        if let SendOutcome::Started { message_id } | SendOutcome::Steered { message_id } = &outcome
        {
            let id = message_id.clone();
            self.core.update_state(|st| st.last_submitted = Some(id));
        }
        self.core.take_staged_comments(&staged);
        self.core.update_state(|st| st.prediction = None);
        client.after_command(&self.core, !refs.is_empty());
        Ok(outcome)
    }

    /// Stop the live turn.
    pub fn interrupt(&self) -> Result<()> {
        let client = self.core.client()?;
        self.core.queue_command(
            &client.config.device_id,
            SessionCommandPayload::Interrupt { teardown: false },
        )?;
        client.after_command(&self.core, false);
        Ok(())
    }

    /// Answer the open question panel.
    pub fn respond_input(&self, request_id: &str, answers: Vec<UserInputAnswer>) -> Result<()> {
        let client = self.core.client()?;
        self.core.queue_command(
            &client.config.device_id,
            SessionCommandPayload::RespondInput {
                request_id: request_id.to_owned(),
                answers,
            },
        )?;
        client.after_command(&self.core, false);
        Ok(())
    }

    pub fn answer_consent(&self, request_id: &str, question_id: &str, label: &str) -> Result<bool> {
        if self.snapshot().input_answers.contains_key(request_id) {
            return Ok(false);
        }
        self.respond_input(
            request_id,
            vec![UserInputAnswer {
                question_id: question_id.to_owned(),
                labels: vec![label.to_owned()],
            }],
        )?;
        Ok(true)
    }

    pub async fn computer_still_frame(&self) -> Result<Option<crate::computer::ComputerStill>> {
        let host = self.composer().host.clone();
        if host.device_id.is_empty() || !host.online {
            return Ok(None);
        }
        let client = self.core.client()?;
        let reply = client
            .host_rpc(
                &host.device_id,
                zeron_rpc::methods::GET_COMPUTER_STILL_FRAME,
                serde_json::json!({ "chatId": self.core.chat_id }),
            )
            .await?;
        Ok(crate::computer::parse_still_reply(&reply))
    }

    async fn chat_host_rpc<T: serde::de::DeserializeOwned>(
        &self,
        method: &'static str,
        mut params: serde_json::Value,
    ) -> Result<T> {
        let host = self.composer().host.clone();
        if host.device_id.is_empty() || !host.online {
            return Err(ClientError::HostUnavailable(host.device_id));
        }
        params["chatId"] = self.core.chat_id.clone().into();
        let reply = self
            .core
            .client()?
            .host_rpc(&host.device_id, method, params)
            .await?;
        serde_json::from_value(reply).map_err(|e| ClientError::Internal(e.to_string()))
    }

    /// Per chat, as the host's revoke is: other chats keep their access.
    pub async fn computer_stop(&self) -> Result<bool> {
        let reply: serde_json::Value = self
            .chat_host_rpc(
                zeron_rpc::methods::COMPUTER_REVOKE_DESKTOP,
                serde_json::json!({}),
            )
            .await?;
        Ok(reply["revoked"].as_bool().unwrap_or(false))
    }

    pub async fn list_terminals(&self) -> Result<Vec<zeron_proto::TerminalInfo>> {
        self.chat_host_rpc(zeron_rpc::methods::LIST_TERMINALS, serde_json::json!({}))
            .await
    }

    pub async fn read_terminal(&self, terminal_id: &str) -> Result<zeron_proto::TerminalScreen> {
        self.chat_host_rpc(
            zeron_rpc::methods::READ_TERMINAL,
            serde_json::json!({ "terminalId": terminal_id, "lines": 400, "styled": true }),
        )
        .await
    }

    fn space_id(&self) -> Result<Option<String>> {
        let client = self.core.client()?;
        Ok(client
            .workspace
            .chat(&self.core.chat_id)
            .and_then(|chat| chat.space_id))
    }

    pub async fn list_project_actions(
        &self,
    ) -> Result<Option<zeron_proto::ProjectActionsSnapshot>> {
        let Some(space_id) = self.space_id()? else {
            return Ok(None);
        };
        self.chat_host_rpc(
            zeron_rpc::methods::LIST_PROJECT_ACTIONS,
            serde_json::json!({ "spaceId": space_id }),
        )
        .await
        .map(Some)
    }

    pub async fn run_project_action(
        &self,
        action_id: &str,
        cols: u16,
        rows: u16,
    ) -> Result<zeron_proto::ProjectActionRun> {
        let space_id = self
            .space_id()?
            .ok_or_else(|| ClientError::InvalidArgument("this chat has no project".into()))?;
        self.chat_host_rpc(
            zeron_rpc::methods::RUN_PROJECT_ACTION,
            serde_json::json!({
                "spaceId": space_id,
                "actionId": action_id,
                "cols": cols,
                "rows": rows,
            }),
        )
        .await
    }

    fn enqueue_inner(
        &self,
        device_id: &str,
        text: &str,
        attachments: Vec<String>,
        hold_for_turn_end: bool,
    ) -> Result<String> {
        let mut item = QueuedMessage::new(crate::new_id(), text, device_id);
        item.attachments = attachments;
        item.hold_for_turn_end = hold_for_turn_end;
        item.issued_at = now_ms();
        item.computer_generation = Some(self.core.doc.computer_generation());
        let id = item.id.clone();
        self.core.write(|doc| doc.push_queued(&item))?;
        Ok(id)
    }

    /// Park a message on the shared queue directly.
    pub fn enqueue(
        &self,
        text: &str,
        attachments: Vec<String>,
        hold_for_turn_end: bool,
    ) -> Result<String> {
        let client = self.core.client()?;
        if text.trim().is_empty() {
            return Err(ClientError::InvalidArgument("empty message".into()));
        }
        let id = self.enqueue_inner(
            &client.config.device_id,
            text,
            attachments,
            hold_for_turn_end,
        )?;
        client.after_command(&self.core, false);
        Ok(id)
    }

    /// Move a queue row to `to` (clamped). Rows under a delivery gate or with
    /// an action in flight don't move.
    pub fn move_queued(&self, id: &str, to: usize) -> Result<bool> {
        {
            let st = lock(&self.core.state);
            let row = st.queue.iter().find(|q| q.id == id);
            if row.is_none_or(|r| r.delivery_gate.is_some())
                || st.queue_actions_pending.contains(id)
            {
                return Ok(false);
            }
        }
        let moved = self.core.write(|doc| doc.move_queued(id, to))?;
        if moved && let Ok(client) = self.core.client() {
            client.after_command(&self.core, false);
        }
        Ok(moved)
    }

    /// Nudge a row one slot up (`-1`) or down (`+1`).
    pub fn move_queued_by(&self, id: &str, delta: i32) -> Result<bool> {
        let (from, count) = {
            let st = lock(&self.core.state);
            (st.queue.iter().position(|q| q.id == id), st.queue.len())
        };
        let Some(from) = from else { return Ok(false) };
        let to = from as i64 + delta as i64;
        if to < 0 || to >= count as i64 {
            return Ok(false);
        }
        self.move_queued(id, to as usize)
    }

    async fn queue_rpc(
        &self,
        method: &'static str,
        params: serde_json::Value,
    ) -> Result<serde_json::Value> {
        let client = self.core.client()?;
        let host = self.composer().host.device_id.clone();
        if host.is_empty() {
            return Err(ClientError::HostUnavailable("no host".into()));
        }
        client.host_rpc(&host, method, params).await
    }

    pub async fn begin_queued_edit(&self, id: &str, instance_id: &str) -> QueueEditStart {
        let composer = self.composer();
        if !composer.host.capabilities.queue_edit_lease
            || !composer
                .queue
                .iter()
                .any(|q| q.id == id && !q.action_pending)
        {
            return QueueEditStart::Unavailable;
        }
        let Ok(client) = self.core.client() else {
            return QueueEditStart::Unavailable;
        };
        let reply = self
            .queue_rpc(
                zeron_rpc::methods::BEGIN_QUEUED_MESSAGE_EDIT,
                serde_json::json!({
                    "chatId": self.core.chat_id,
                    "id": id,
                    "editorDeviceId": client.config.device_id,
                    "editorInstanceId": instance_id,
                }),
            )
            .await;
        match reply {
            Ok(value) => match value.get("outcome").and_then(|o| o.as_str()) {
                Some("acquired") => {
                    let field = |k: &str| value.get(k).and_then(|v| v.as_str()).map(str::to_owned);
                    match (
                        field("leaseId"),
                        field("baseTextHash"),
                        value.get("expiresAtMs").and_then(|v| v.as_i64()),
                    ) {
                        (Some(lease_id), Some(base_text_hash), Some(expires_at_ms)) => {
                            QueueEditStart::Acquired(QueueEditLease {
                                row_id: id.to_owned(),
                                lease_id,
                                text: field("text").unwrap_or_default(),
                                base_text_hash,
                                expires_at_ms,
                            })
                        }
                        _ => QueueEditStart::Unavailable,
                    }
                }
                Some("locked") => QueueEditStart::Locked,
                _ => QueueEditStart::Missing,
            },
            Err(_) => QueueEditStart::Unavailable,
        }
    }

    pub async fn renew_queued_edit(&self, lease: &QueueEditLease) -> bool {
        self.queue_rpc(
            zeron_rpc::methods::RENEW_QUEUED_MESSAGE_EDIT,
            serde_json::json!({
                "chatId": self.core.chat_id,
                "id": lease.row_id,
                "leaseId": lease.lease_id,
            }),
        )
        .await
        .is_ok_and(|v| v.get("outcome").and_then(|o| o.as_str()) == Some("renewed"))
    }

    pub async fn finish_queued_edit(
        &self,
        lease: &QueueEditLease,
        action: QueueEditAction,
        text: Option<String>,
    ) -> QueueEditFinish {
        let mut params = serde_json::json!({
            "chatId": self.core.chat_id,
            "id": lease.row_id,
            "leaseId": lease.lease_id,
            "action": action.wire(),
            "expectedTextHash": lease.base_text_hash,
        });
        if let Some(text) = text {
            params["text"] = serde_json::Value::String(text);
        }
        match self
            .queue_rpc(zeron_rpc::methods::FINISH_QUEUED_MESSAGE_EDIT, params)
            .await
        {
            Ok(value) => match value.get("outcome").and_then(|o| o.as_str()) {
                Some("committed" | "cancelled" | "discarded" | "released") => {
                    QueueEditFinish::Finished
                }
                Some("conflict") => QueueEditFinish::Conflict,
                Some("missing") => QueueEditFinish::Missing,
                _ => QueueEditFinish::Lost,
            },
            Err(_) => QueueEditFinish::Unavailable,
        }
    }

    async fn queue_action(&self, id: &str, action: QueueAction) -> Result<bool> {
        let send_now = action != QueueAction::Remove;
        {
            let mut st = lock(&self.core.state);
            let Some(row) = st.queue.iter().find(|q| q.id == id) else {
                return Ok(false);
            };
            if st.queue_actions_pending.contains(id) || (send_now && row.delivery_gate.is_some()) {
                return Ok(false);
            }
            st.queue_actions_pending.insert(id.to_owned());
            st.queue_error = None;
        }
        if let Ok(client) = self.core.client() {
            self.core.recompute_composer(&client);
        }
        let method = match action {
            QueueAction::SendNow => zeron_rpc::methods::SEND_QUEUED_MESSAGE_NOW,
            QueueAction::Steer => zeron_rpc::methods::STEER_QUEUED_MESSAGE_NOW,
            QueueAction::Remove => zeron_rpc::methods::REMOVE_QUEUED_MESSAGE,
        };
        let result = self
            .queue_rpc(
                method,
                serde_json::json!({ "chatId": self.core.chat_id, "id": id }),
            )
            .await;
        let label = if send_now { "send now" } else { "remove" };
        let key = if send_now { "sent" } else { "removed" };
        let outcome = match result {
            Ok(value) if value.get(key).and_then(|v| v.as_bool()) == Some(true) => {
                if !send_now {
                    // Apply only a confirmed removal; sends leave it to sync.
                    let _ = self.core.write(|doc| doc.remove_queued(id));
                }
                Ok(true)
            }
            Ok(_) => Err(
                "The host did not confirm the action. The message may have already left the queue."
                    .to_owned(),
            ),
            Err(err) => Err(format!(
                "Couldn't complete {label}. Check the connection to the chat host and the queue before retrying. ({err})"
            )),
        };
        let id = id.to_owned();
        match outcome {
            Ok(done) => {
                self.core.update_state(|st| {
                    st.queue_actions_pending.remove(&id);
                });
                Ok(done)
            }
            Err(message) => {
                self.core.update_state(|st| {
                    st.queue_actions_pending.remove(&id);
                    st.queue_error = Some(message.clone());
                });
                Err(ClientError::HostUnavailable(message))
            }
        }
    }

    /// Deliver a queued row now (interrupting the live turn if needed).
    pub async fn send_queued_now(&self, id: &str) -> Result<bool> {
        self.queue_action(id, QueueAction::SendNow).await
    }

    /// Deliver a queued row without interrupting: steered into a live turn
    /// that reads mid-turn, else sent next at the turn boundary.
    pub async fn steer_queued_now(&self, id: &str) -> Result<bool> {
        self.queue_action(id, QueueAction::Steer).await
    }

    /// A row's primary action, as the desktop resolves it: text never
    /// interrupts the turn (steer / send next); only attachments, which need
    /// a fresh request, send now.
    pub async fn deliver_queued_now(&self, id: &str) -> Result<bool> {
        let has_attachments = lock(&self.core.state)
            .queue
            .iter()
            .any(|q| q.id == id && !q.attachments.is_empty());
        if has_attachments {
            self.send_queued_now(id).await
        } else {
            self.steer_queued_now(id).await
        }
    }

    /// Remove a queued row (applied locally only after the host acks).
    pub async fn remove_queued(&self, id: &str) -> Result<bool> {
        self.queue_action(id, QueueAction::Remove).await
    }

    pub fn clear_queue_error(&self) {
        self.core.update_state(|st| st.queue_error = None);
    }

    pub async fn fork_from(&self, entry_id: &str) -> Result<String> {
        let chat_id = crate::new_id();
        let reply = self
            .queue_rpc(
                zeron_rpc::methods::FORK_CHAT,
                serde_json::json!({
                    "sourceChatId": self.core.chat_id,
                    "entryId": entry_id,
                    "chatId": chat_id,
                }),
            )
            .await
            .map_err(host_reason)?;
        Ok(reply["id"].as_str().map_or(chat_id, str::to_owned))
    }

    pub async fn rewind_to(&self, entry_id: &str) -> Result<String> {
        let reply = self
            .queue_rpc(
                zeron_rpc::methods::REWIND_CHAT,
                serde_json::json!({ "chatId": self.core.chat_id, "entryId": entry_id }),
            )
            .await
            .map_err(host_reason)?;
        Ok(reply["prompt"].as_str().unwrap_or_default().to_owned())
    }

    pub async fn skill_file(&self, name: &str, path: Option<&str>) -> Result<SkillFile> {
        let path = match path {
            Some(path) => path.to_owned(),
            None => {
                let harness = self
                    .core
                    .client()?
                    .workspace
                    .chat(&self.core.chat_id)
                    .and_then(|chat| chat.config)
                    .map(|config| config.harness)
                    .ok_or_else(|| {
                        ClientError::NotFound(format!("Can't find the file for {name} here"))
                    })?;
                let skills = self
                    .queue_rpc(
                        zeron_rpc::methods::LIST_SKILLS,
                        serde_json::json!({ "harness": harness, "chatId": self.core.chat_id }),
                    )
                    .await?;
                serde_json::from_value::<Option<Vec<zeron_proto::invocation::Skill>>>(skills)
                    .ok()
                    .flatten()
                    .unwrap_or_default()
                    .into_iter()
                    .find(|skill| {
                        skill.name == name
                            && !zeron_proto::invocation::native_skill_identity(&skill.path)
                    })
                    .map(|skill| skill.path)
                    .ok_or_else(|| ClientError::NotFound(format!("No SKILL.md found for {name}")))?
            }
        };
        // An absolute path outside the workspace reads only with the chat named.
        let file = self
            .queue_rpc(
                zeron_rpc::methods::READ_WORKSPACE_FILE,
                serde_json::json!({ "chatId": self.core.chat_id, "path": path }),
            )
            .await
            .map_err(host_reason)?;
        let file: zeron_proto::WorkspaceFileText =
            serde_json::from_value(file).map_err(|err| ClientError::Internal(err.to_string()))?;
        Ok(SkillFile {
            path: file.path,
            text: file.text.unwrap_or_default(),
            truncated: file.truncated,
        })
    }

    // An unanswered harness probe allows it; the host still refuses.
    pub async fn branch_blocked(&self, rewind: bool) -> Option<String> {
        if rewind
            && matches!(
                self.composer().live.indicator,
                ChatIndicator::Working | ChatIndicator::AwaitingInput
            )
        {
            return Some("Stop the running turn before rewinding".into());
        }
        let harness = self
            .core
            .client()
            .ok()?
            .workspace
            .chat(&self.core.chat_id)?
            .config?
            .harness;
        let reply = self
            .queue_rpc(
                zeron_rpc::methods::HARNESS_COMPACTION,
                serde_json::json!({ "harness": harness }),
            )
            .await
            .ok()?;
        let probe: zeron_proto::HarnessCompactionResult = serde_json::from_value(reply).ok()?;
        (!probe.branching)
            .then(|| format!("{} does not support fork or rewind", harness.display_name()))
    }

    /// "Not delivered — tap to retry": restart grace clocks, re-issue dead
    /// attempts (fresh command id, same message id — the host dedupes the
    /// user entry by message id), kick the room, nudge the host.
    pub fn retry_delivery(&self) -> Result<()> {
        let client = self.core.client()?;
        let now = now_ms();
        let device_id = client.config.device_id.clone();
        let (commands, pending_ids) = {
            let mut st = lock(&self.core.state);
            let ids: Vec<String> = st.pending.iter().map(|p| p.message_id.clone()).collect();
            for id in &ids {
                st.grace_started.insert(id.clone(), now);
            }
            (st.commands.clone(), ids)
        };
        let landed = |id: &str| !pending_ids.iter().any(|p| p == id);
        let mut latest_dead: HashMap<String, SessionCommandEntry> = HashMap::new();
        let mut live: HashSet<String> = HashSet::new();
        for command in &commands {
            if command.issued_by != device_id {
                continue;
            }
            let message_id = match &command.payload {
                SessionCommandPayload::Run { message_id, .. } => message_id.clone(),
                SessionCommandPayload::Steer {
                    message_id: Some(message_id),
                    ..
                } => message_id.clone(),
                _ => continue,
            };
            if landed(&message_id) {
                continue;
            }
            let expired = now >= command.effective_expiry();
            match command.status {
                SessionCommandStatus::Pending if !expired => {
                    live.insert(message_id);
                }
                SessionCommandStatus::Applied => {
                    live.insert(message_id);
                }
                SessionCommandStatus::Rejected
                | SessionCommandStatus::Expired
                | SessionCommandStatus::Pending => {
                    let newer = latest_dead
                        .get(&message_id)
                        .is_none_or(|existing| existing.issued_at < command.issued_at);
                    if newer {
                        latest_dead.insert(message_id, command.clone());
                    }
                }
                _ => {}
            }
        }
        for (message_id, attempt) in latest_dead {
            if live.contains(&message_id) {
                continue;
            }
            tracing::info!(chat = %self.core.chat_id, old = %attempt.id, "retry re-issues a dead send attempt");
            self.core
                .queue_command(&device_id, attempt.payload.clone())?;
        }
        self.core.refresh();
        client.after_command(&self.core, true);
        client.kick_room(&self.core.chat_id);
        Ok(())
    }
}
