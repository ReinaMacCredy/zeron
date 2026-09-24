use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use futures::FutureExt;
use futures::future::{BoxFuture, Shared};
use tokio::sync::oneshot;
use tokio_util::sync::CancellationToken;
use zeron_proto::PermissionMode;
use zeron_proto::computer::ComputerAllowedApp;

pub const COMPUTER_APPROVAL_QUEUE_FULL_CODE: &str = "approval_queue_full";
pub const COMPUTER_APPROVAL_QUEUE_GLOBAL_LIMIT: usize = 128;
pub const COMPUTER_APPROVAL_QUEUE_THREAD_LIMIT: usize = 8;
pub const COMPUTER_APPROVAL_TIMEOUT: Duration = Duration::from_secs(5 * 60);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApprovalDecision {
    Accept,
    Decline,
    Cancel,
    AcceptForSession,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ComputerApprovalError {
    #[error(
        "Too many computer approvals are waiting for this chat; try again once an earlier prompt settles."
    )]
    ThreadQueueFull,
    #[error("Too many computer approvals are waiting.")]
    GlobalQueueFull,
    #[error("The computer approval was cancelled.")]
    Aborted,
    #[error("The computer approval prompt could not be shown: {0}")]
    Publish(String),
}

impl ComputerApprovalError {
    pub fn code(&self) -> Option<&'static str> {
        match self {
            Self::ThreadQueueFull | Self::GlobalQueueFull => {
                Some(COMPUTER_APPROVAL_QUEUE_FULL_CODE)
            }
            Self::Aborted | Self::Publish(_) => None,
        }
    }

    pub fn retryable(&self) -> bool {
        self.code().is_some()
    }
}

pub fn computer_consent_card_required(mode: PermissionMode) -> bool {
    mode == PermissionMode::Ask
}

pub type ApprovalPublish = Arc<
    dyn Fn(String, Option<ApprovalDecision>) -> BoxFuture<'static, Result<(), String>>
        + Send
        + Sync,
>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppGrant {
    pub key: String,
    pub always: Option<ComputerAllowedApp>,
}

pub type AlwaysAllowHook = Arc<dyn Fn(ComputerAllowedApp) + Send + Sync>;

#[derive(Clone)]
pub struct ApprovalRequest {
    pub chat_id: String,
    pub turn_id: Option<String>,
    pub mode: PermissionMode,
    pub signal: CancellationToken,
    pub publish: ApprovalPublish,
    pub app: Option<AppGrant>,
}

type SharedApproval = Shared<BoxFuture<'static, Result<bool, ComputerApprovalError>>>;

struct PendingApproval {
    chat_id: String,
    turn_id: Option<String>,
    always: Option<ComputerAllowedApp>,
    settle: oneshot::Sender<ApprovalDecision>,
}

struct TaskApproval {
    id: u64,
    granted: Option<bool>,
    pending: Option<SharedApproval>,
}

struct TurnApprovals {
    turn_id: Option<String>,
    grants: HashMap<Option<String>, TaskApproval>,
}

#[derive(Default)]
struct GateState {
    pending: HashMap<String, PendingApproval>,
    tasks: HashMap<String, TurnApprovals>,
    next_task: u64,
    always: HashSet<String>,
    on_always: Option<AlwaysAllowHook>,
}

impl GateState {
    fn cancel_thread(&mut self, chat_id: &str, turn_id: Option<&str>) {
        if self
            .tasks
            .get(chat_id)
            .is_some_and(|task| turn_id.is_none() || task.turn_id.as_deref() == turn_id)
        {
            self.tasks.remove(chat_id);
        }
        let cancelled: Vec<String> = self
            .pending
            .iter()
            .filter(|(_, pending)| {
                pending.chat_id == chat_id
                    && (turn_id.is_none() || pending.turn_id.as_deref() == turn_id)
            })
            .map(|(id, _)| id.clone())
            .collect();
        for id in cancelled {
            if let Some(pending) = self.pending.remove(&id) {
                let _ = pending.settle.send(ApprovalDecision::Cancel);
            }
        }
    }

    fn task_is(&self, chat_id: &str, key: &Option<String>, id: u64) -> bool {
        self.tasks
            .get(chat_id)
            .and_then(|turn| turn.grants.get(key))
            .is_some_and(|task| task.id == id)
    }

    fn task(&mut self, chat_id: &str, key: &Option<String>) -> Option<&mut TaskApproval> {
        self.tasks
            .get_mut(chat_id)
            .and_then(|turn| turn.grants.get_mut(key))
    }
}

struct GateInner {
    state: Mutex<GateState>,
    timeout: Duration,
}

#[derive(Clone)]
pub struct ComputerApprovalGate {
    inner: Arc<GateInner>,
}

impl Default for ComputerApprovalGate {
    fn default() -> Self {
        Self::new(COMPUTER_APPROVAL_TIMEOUT)
    }
}

static NEXT_REQUEST: AtomicU64 = AtomicU64::new(1);

struct PendingSlot {
    gate: Arc<GateInner>,
    request_id: String,
    publish: ApprovalPublish,
    decision: ApprovalDecision,
}

impl Drop for PendingSlot {
    fn drop(&mut self) {
        lock(&self.gate.state).pending.remove(&self.request_id);
        let dismissal = (self.publish)(self.request_id.clone(), Some(self.decision));
        if let Ok(runtime) = tokio::runtime::Handle::try_current() {
            runtime.spawn(dismissal);
        }
    }
}

fn lock(state: &Mutex<GateState>) -> MutexGuard<'_, GateState> {
    state.lock().unwrap_or_else(PoisonError::into_inner)
}

impl ComputerApprovalGate {
    pub fn new(timeout: Duration) -> Self {
        Self {
            inner: Arc::new(GateInner {
                state: Mutex::default(),
                timeout,
            }),
        }
    }

    fn state(&self) -> MutexGuard<'_, GateState> {
        lock(&self.inner.state)
    }

    pub fn cancel_thread(&self, chat_id: &str, turn_id: Option<&str>) {
        self.state().cancel_thread(chat_id, turn_id);
    }

    pub fn revoke_task_grants(&self) {
        for task in self
            .state()
            .tasks
            .values_mut()
            .flat_map(|turn| turn.grants.values_mut())
        {
            if task.granted == Some(true) {
                task.granted = None;
            }
        }
    }

    pub fn app_granted(
        &self,
        chat_id: &str,
        turn_id: Option<&str>,
        key: &str,
        bundle_id: Option<&str>,
    ) -> bool {
        let state = self.state();
        bundle_id.is_some_and(|bundle_id| state.always.contains(bundle_id))
            || state
                .tasks
                .get(chat_id)
                .filter(|turn| turn.turn_id.as_deref() == turn_id)
                .and_then(|turn| turn.grants.get(&Some(key.to_owned())))
                .is_some_and(|task| task.granted == Some(true))
    }

    pub fn set_always_allowed<'a>(&self, bundle_ids: impl IntoIterator<Item = &'a str>) {
        self.state().always = bundle_ids.into_iter().map(str::to_owned).collect();
    }

    pub fn on_always_allowed(&self, hook: AlwaysAllowHook) {
        self.state().on_always = Some(hook);
    }

    pub fn respond(&self, chat_id: &str, request_id: &str, decision: ApprovalDecision) -> bool {
        let mut state = self.state();
        if state
            .pending
            .get(request_id)
            .is_none_or(|pending| pending.chat_id != chat_id)
        {
            return false;
        }
        let pending = state.pending.remove(request_id).expect("checked above");
        let (effective, remembered) = match (decision, pending.always) {
            (ApprovalDecision::AcceptForSession, Some(app)) => {
                state.always.insert(app.bundle_id.clone());
                (ApprovalDecision::Accept, Some(app))
            }
            (ApprovalDecision::AcceptForSession, None) => (ApprovalDecision::Decline, None),
            (decision, _) => (decision, None),
        };
        let hook = remembered.and_then(|app| Some((state.on_always.clone()?, app)));
        drop(state);
        if let Some((hook, app)) = hook {
            hook(app);
        }
        let _ = pending.settle.send(effective);
        true
    }

    pub async fn request_task(
        &self,
        request: ApprovalRequest,
    ) -> Result<bool, ComputerApprovalError> {
        if request.signal.is_cancelled() {
            return Err(ComputerApprovalError::Aborted);
        }
        if !computer_consent_card_required(request.mode) {
            return Ok(true);
        }
        let chat_id = request.chat_id.clone();
        let signal = request.signal.clone();
        let key = request.app.as_ref().map(|app| app.key.clone());
        let (task_id, pending) = {
            let mut state = self.state();
            if state
                .tasks
                .get(&chat_id)
                .is_none_or(|turn| turn.turn_id != request.turn_id)
            {
                state.cancel_thread(&chat_id, None);
                state.tasks.insert(
                    chat_id.clone(),
                    TurnApprovals {
                        turn_id: request.turn_id.clone(),
                        grants: HashMap::new(),
                    },
                );
            }
            let always = request
                .app
                .as_ref()
                .and_then(|app| app.always.as_ref())
                .is_some_and(|app| state.always.contains(&app.bundle_id));
            let id = state.next_task;
            state.next_task += 1;
            let turn = state.tasks.get_mut(&chat_id).expect("inserted above");
            if key.is_none() && turn.grants.values().any(|task| task.granted == Some(true)) {
                return Ok(true);
            }
            let task = turn.grants.entry(key.clone()).or_insert(TaskApproval {
                id,
                granted: None,
                pending: None,
            });
            if always {
                task.granted = Some(true);
            }
            if let Some(granted) = task.granted {
                return Ok(granted);
            }
            let id = task.id;
            let pending = match &task.pending {
                Some(pending) => pending.clone(),
                None => {
                    let pending = self.spawn_task_request(id, key.clone(), request);
                    task.pending = Some(pending.clone());
                    pending
                }
            };
            (id, pending)
        };
        let accepted = tokio::select! {
            biased;
            () = signal.cancelled() => return Err(ComputerApprovalError::Aborted),
            accepted = pending => accepted?,
        };
        if signal.is_cancelled() {
            return Err(ComputerApprovalError::Aborted);
        }
        Ok(accepted && self.state().task_is(&chat_id, &key, task_id))
    }

    // Spawned so the shared prompt keeps running while only a follower polls it.
    fn spawn_task_request(
        &self,
        task_id: u64,
        key: Option<String>,
        request: ApprovalRequest,
    ) -> SharedApproval {
        let gate = self.clone();
        let chat_id = request.chat_id.clone();
        let signal = request.signal.clone();
        let handle = tokio::spawn(async move {
            let result = gate.request(request).await;
            let mut state = gate.state();
            let current = state.task(&chat_id, &key).filter(|task| task.id == task_id);
            let outcome = match result {
                Ok(accepted) => match current {
                    Some(task) if !signal.is_cancelled() => {
                        task.granted = Some(accepted);
                        Ok(accepted)
                    }
                    _ => Ok(false),
                },
                Err(error) => Err(error),
            };
            if let Some(task) = state.task(&chat_id, &key).filter(|task| task.id == task_id) {
                task.pending = None;
            }
            outcome
        });
        handle
            .map(|joined| joined.unwrap_or(Err(ComputerApprovalError::Aborted)))
            .boxed()
            .shared()
    }

    pub async fn request(&self, request: ApprovalRequest) -> Result<bool, ComputerApprovalError> {
        if request.signal.is_cancelled() {
            return Err(ComputerApprovalError::Aborted);
        }
        if !computer_consent_card_required(request.mode) {
            return Ok(true);
        }
        let request_id = format!("computer:{}", NEXT_REQUEST.fetch_add(1, Ordering::Relaxed));
        let (settle, mut answer) = oneshot::channel();
        {
            let mut state = self.state();
            let thread_pending = state
                .pending
                .values()
                .filter(|pending| pending.chat_id == request.chat_id)
                .count();
            if thread_pending >= COMPUTER_APPROVAL_QUEUE_THREAD_LIMIT {
                return Err(ComputerApprovalError::ThreadQueueFull);
            }
            if state.pending.len() >= COMPUTER_APPROVAL_QUEUE_GLOBAL_LIMIT {
                return Err(ComputerApprovalError::GlobalQueueFull);
            }
            state.pending.insert(
                request_id.clone(),
                PendingApproval {
                    chat_id: request.chat_id.clone(),
                    turn_id: request.turn_id.clone(),
                    always: request.app.as_ref().and_then(|app| app.always.clone()),
                    settle,
                },
            );
        }
        let mut slot = PendingSlot {
            gate: self.inner.clone(),
            request_id: request_id.clone(),
            publish: request.publish.clone(),
            decision: ApprovalDecision::Cancel,
        };
        let signal = request.signal;
        let timeout = tokio::time::sleep(self.inner.timeout);
        tokio::pin!(timeout);
        let mut published = (request.publish)(request_id, None);
        let decision = tokio::select! {
            biased;
            decision = &mut answer => decision.unwrap_or(ApprovalDecision::Cancel),
            () = signal.cancelled() => ApprovalDecision::Cancel,
            () = &mut timeout => ApprovalDecision::Cancel,
            outcome = &mut published => match outcome {
                Ok(()) => tokio::select! {
                    biased;
                    decision = &mut answer => decision.unwrap_or(ApprovalDecision::Cancel),
                    () = signal.cancelled() => ApprovalDecision::Cancel,
                    () = &mut timeout => ApprovalDecision::Cancel,
                },
                Err(error) => return Err(ComputerApprovalError::Publish(error)),
            },
        };
        slot.decision = decision;
        if signal.is_cancelled() {
            return Err(ComputerApprovalError::Aborted);
        }
        Ok(decision == ApprovalDecision::Accept)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    type Log = Arc<Mutex<Vec<(String, Option<ApprovalDecision>)>>>;

    async fn settle() {
        for _ in 0..32 {
            tokio::task::yield_now().await;
        }
    }

    fn recording(log: &Log) -> ApprovalPublish {
        let log = log.clone();
        Arc::new(move |id, decision| {
            log.lock().unwrap().push((id, decision));
            async { Ok(()) }.boxed()
        })
    }

    fn opened(log: &Log) -> Vec<String> {
        log.lock()
            .unwrap()
            .iter()
            .filter(|(_, decision)| decision.is_none())
            .map(|(id, _)| id.clone())
            .collect()
    }

    fn ask(chat_id: &str, turn_id: Option<&str>, publish: ApprovalPublish) -> ApprovalRequest {
        ApprovalRequest {
            chat_id: chat_id.to_owned(),
            turn_id: turn_id.map(str::to_owned),
            mode: PermissionMode::Ask,
            signal: CancellationToken::new(),
            publish,
            app: None,
        }
    }

    fn auto_answer(gate: &ComputerApprovalGate, chat_id: &str, log: &Log) -> ApprovalPublish {
        let gate = gate.clone();
        let chat_id = chat_id.to_owned();
        let log = log.clone();
        Arc::new(move |id, decision| {
            if decision.is_none() {
                log.lock().unwrap().push((id.clone(), None));
                gate.respond(&chat_id, &id, ApprovalDecision::Accept);
            }
            async { Ok(()) }.boxed()
        })
    }

    #[tokio::test]
    async fn cancels_a_concurrent_waiter_without_approving_or_cancelling_another_call() {
        let gate = ComputerApprovalGate::default();
        let log = Log::default();
        let input = ask("a", Some("turn"), recording(&log));
        let first = tokio::spawn({
            let gate = gate.clone();
            let input = input.clone();
            async move { gate.request_task(input).await }
        });
        settle().await;
        let follower_signal = CancellationToken::new();
        let follower = tokio::spawn({
            let gate = gate.clone();
            let input = ApprovalRequest {
                signal: follower_signal.clone(),
                ..input.clone()
            };
            async move { gate.request_task(input).await }
        });
        settle().await;
        follower_signal.cancel();
        assert_eq!(follower.await.unwrap(), Err(ComputerApprovalError::Aborted));
        let ids = opened(&log);
        assert_eq!(ids.len(), 1);
        assert!(gate.respond("a", &ids[0], ApprovalDecision::Accept));
        assert_eq!(first.await.unwrap(), Ok(true));
    }

    #[tokio::test]
    async fn does_not_reuse_task_consent_for_a_separate_clipboard_approval() {
        let gate = ComputerApprovalGate::default();
        let log = Log::default();
        let input = ask("a", Some("turn"), auto_answer(&gate, "a", &log));
        assert_eq!(gate.request_task(input.clone()).await, Ok(true));
        assert_eq!(gate.request_task(input.clone()).await, Ok(true));
        assert_eq!(gate.request(input.clone()).await, Ok(true));
        assert_eq!(gate.request(input).await, Ok(true));
        assert_eq!(opened(&log).len(), 3);
    }

    #[tokio::test]
    async fn shares_one_consent_across_concurrent_and_later_routine_actions_in_the_same_turn() {
        let gate = ComputerApprovalGate::default();
        let log = Log::default();
        let input = ask("a", Some("turn-1"), recording(&log));
        let spawn = |input: ApprovalRequest| {
            let gate = gate.clone();
            tokio::spawn(async move { gate.request_task(input).await })
        };
        let first = spawn(input.clone());
        let concurrent = spawn(input.clone());
        settle().await;
        assert_eq!(opened(&log).len(), 1);
        gate.respond("a", &opened(&log)[0], ApprovalDecision::Accept);
        assert_eq!(first.await.unwrap(), Ok(true));
        assert_eq!(concurrent.await.unwrap(), Ok(true));
        assert_eq!(gate.request_task(input.clone()).await, Ok(true));
        assert_eq!(opened(&log).len(), 1);
        gate.cancel_thread("a", Some("old-turn"));
        assert_eq!(gate.request_task(input.clone()).await, Ok(true));
        gate.cancel_thread("a", Some("turn-1"));
        let turn_two = ApprovalRequest {
            turn_id: Some("turn-2".to_owned()),
            ..input
        };
        let next = spawn(turn_two.clone());
        settle().await;
        assert_eq!(opened(&log).len(), 2);
        gate.respond("a", &opened(&log)[1], ApprovalDecision::Decline);
        assert_eq!(next.await.unwrap(), Ok(false));
        assert_eq!(gate.request_task(turn_two).await, Ok(false));
        assert_eq!(opened(&log).len(), 2);
    }

    #[tokio::test]
    async fn re_prompts_a_declined_task_next_turn_without_leaking_into_clipboard() {
        let gate = ComputerApprovalGate::default();
        let tasks = Log::default();
        let clipboard = Log::default();
        let task = |turn: &str| ask("a", Some(turn), recording(&tasks));
        let clip = || ask("a", None, recording(&clipboard));
        let spawn_task = |input: ApprovalRequest| {
            let gate = gate.clone();
            tokio::spawn(async move { gate.request_task(input).await })
        };
        let spawn_request = |input: ApprovalRequest| {
            let gate = gate.clone();
            tokio::spawn(async move { gate.request(input).await })
        };
        let first = spawn_task(task("turn-1"));
        settle().await;
        gate.respond("a", &opened(&tasks)[0], ApprovalDecision::Decline);
        assert_eq!(first.await.unwrap(), Ok(false));
        let clipboard_first = spawn_request(clip());
        settle().await;
        assert_eq!(opened(&clipboard).len(), 1);
        gate.respond("a", &opened(&clipboard)[0], ApprovalDecision::Decline);
        assert_eq!(clipboard_first.await.unwrap(), Ok(false));
        let second = spawn_task(task("turn-2"));
        settle().await;
        assert_eq!(opened(&tasks).len(), 2);
        gate.respond("a", &opened(&tasks)[1], ApprovalDecision::Accept);
        assert_eq!(second.await.unwrap(), Ok(true));
        assert_eq!(gate.request_task(task("turn-2")).await, Ok(true));
        assert_eq!(opened(&tasks).len(), 2);
        let clipboard_second = spawn_request(clip());
        settle().await;
        assert_eq!(opened(&clipboard).len(), 2);
        gate.respond("a", &opened(&clipboard)[1], ApprovalDecision::Accept);
        assert_eq!(clipboard_second.await.unwrap(), Ok(true));
    }

    #[tokio::test]
    async fn cannot_retain_consent_when_stop_races_an_accepted_response() {
        let gate = ComputerApprovalGate::default();
        let racing = gate.clone();
        let publish: ApprovalPublish = Arc::new(move |id, decision| {
            if decision.is_none() {
                racing.respond("a", &id, ApprovalDecision::Accept);
                racing.cancel_thread("a", None);
            }
            async { Ok(()) }.boxed()
        });
        assert_eq!(
            gate.request_task(ask("a", Some("turn"), publish)).await,
            Ok(false)
        );
    }

    #[tokio::test]
    async fn settles_only_the_disabled_conversations_live_prompt() {
        let gate = ComputerApprovalGate::default();
        let log = Log::default();
        let spawn = |chat: &str| {
            let gate = gate.clone();
            let input = ask(chat, None, recording(&log));
            tokio::spawn(async move { gate.request(input).await })
        };
        let a = spawn("a");
        settle().await;
        let b = spawn("b");
        settle().await;
        let ids = opened(&log);
        gate.cancel_thread("a", None);
        assert_eq!(a.await.unwrap(), Ok(false));
        assert!(!gate.respond("a", &ids[0], ApprovalDecision::Accept));
        assert!(gate.respond("b", &ids[1], ApprovalDecision::Accept));
        assert_eq!(b.await.unwrap(), Ok(true));
    }

    #[tokio::test]
    async fn binds_each_decision_to_the_requesting_conversation_and_one_call() {
        for decision in [
            ApprovalDecision::Accept,
            ApprovalDecision::Decline,
            ApprovalDecision::Cancel,
            ApprovalDecision::AcceptForSession,
        ] {
            let gate = ComputerApprovalGate::default();
            let events: Log = Log::default();
            let answering = gate.clone();
            let sink = events.clone();
            let publish: ApprovalPublish = Arc::new(move |id, resolved| {
                sink.lock().unwrap().push((id.clone(), resolved));
                if resolved.is_none() {
                    assert!(!answering.respond("b", &id, ApprovalDecision::Accept));
                    assert!(answering.respond("a", &id, decision));
                }
                async { Ok(()) }.boxed()
            });
            let result = gate.request(ask("a", None, publish)).await;
            assert_eq!(result, Ok(decision == ApprovalDecision::Accept));
            settle().await;
            let events = events.lock().unwrap().clone();
            assert!(!gate.respond("a", &events[0].0, ApprovalDecision::Accept));
            let expected = match decision {
                ApprovalDecision::AcceptForSession => ApprovalDecision::Decline,
                decision => decision,
            };
            assert_eq!(
                events.iter().map(|(_, d)| *d).collect::<Vec<_>>(),
                [None, Some(expected)]
            );
        }
    }

    #[tokio::test]
    async fn cancels_a_pending_prompt_and_rejects_late_decisions() {
        let gate = ComputerApprovalGate::default();
        let controller = CancellationToken::new();
        let events: Log = Log::default();
        let sink = events.clone();
        let abort = controller.clone();
        let publish: ApprovalPublish = Arc::new(move |id, decision| {
            sink.lock().unwrap().push((id, decision));
            if decision.is_none() {
                abort.cancel();
            }
            async { Ok(()) }.boxed()
        });
        let result = gate
            .request(ApprovalRequest {
                signal: controller,
                ..ask("a", None, publish)
            })
            .await;
        assert_eq!(result, Err(ComputerApprovalError::Aborted));
        settle().await;
        let events = events.lock().unwrap().clone();
        assert_eq!(
            events.iter().map(|(_, d)| *d).collect::<Vec<_>>(),
            [None, Some(ApprovalDecision::Cancel)]
        );
        assert!(!gate.respond("a", &events[0].0, ApprovalDecision::Accept));
    }

    #[tokio::test]
    async fn refuses_a_full_per_thread_queue_retryably_while_other_chats_still_prompt() {
        let gate = ComputerApprovalGate::default();
        let quiet: ApprovalPublish = Arc::new(|_, _| async { Ok(()) }.boxed());
        let mut busy = Vec::new();
        for _ in 0..COMPUTER_APPROVAL_QUEUE_THREAD_LIMIT {
            let gate = gate.clone();
            let input = ask("busy", None, quiet.clone());
            busy.push(tokio::spawn(async move { gate.request(input).await }));
        }
        settle().await;
        let full = gate.request(ask("busy", None, quiet.clone())).await;
        assert_eq!(full, Err(ComputerApprovalError::ThreadQueueFull));
        let error = full.unwrap_err();
        assert_eq!(error.code(), Some("approval_queue_full"));
        assert!(error.retryable());
        let log = Log::default();
        let other = tokio::spawn({
            let gate = gate.clone();
            let input = ask("other", None, recording(&log));
            async move { gate.request(input).await }
        });
        settle().await;
        gate.respond("other", &opened(&log)[0], ApprovalDecision::Accept);
        assert_eq!(other.await.unwrap(), Ok(true));
        gate.cancel_thread("busy", None);
        for waiter in busy {
            assert_eq!(waiter.await.unwrap(), Ok(false));
        }
    }

    #[tokio::test]
    async fn refuses_past_the_shared_queue_cap_with_the_same_retryable_code() {
        let gate = ComputerApprovalGate::default();
        let quiet: ApprovalPublish = Arc::new(|_, _| async { Ok(()) }.boxed());
        let threads: Vec<String> = (0..16).map(|i| format!("thread-{i}")).collect();
        let mut waiting = Vec::new();
        for chat in &threads {
            for _ in 0..COMPUTER_APPROVAL_QUEUE_THREAD_LIMIT {
                let gate = gate.clone();
                let input = ask(chat, None, quiet.clone());
                waiting.push(tokio::spawn(async move { gate.request(input).await }));
            }
        }
        settle().await;
        let overflow = gate.request(ask("overflow", None, quiet)).await;
        assert_eq!(overflow, Err(ComputerApprovalError::GlobalQueueFull));
        assert_eq!(
            overflow.unwrap_err().code(),
            Some(COMPUTER_APPROVAL_QUEUE_FULL_CODE)
        );
        for chat in &threads {
            gate.cancel_thread(chat, None);
        }
        for waiter in waiting {
            assert_eq!(waiter.await.unwrap(), Ok(false));
        }
    }

    #[tokio::test]
    async fn an_abort_releases_a_consent_whose_publish_never_resolves() {
        let gate = ComputerApprovalGate::default();
        let abort = CancellationToken::new();
        let stuck: ApprovalPublish = Arc::new(|_, decision| {
            if decision.is_none() {
                futures::future::pending().boxed()
            } else {
                async { Ok(()) }.boxed()
            }
        });
        let request = tokio::spawn({
            let gate = gate.clone();
            let input = ApprovalRequest {
                signal: abort.clone(),
                ..ask("stuck", None, stuck)
            };
            async move { gate.request(input).await }
        });
        settle().await;
        abort.cancel();
        assert_eq!(request.await.unwrap(), Err(ComputerApprovalError::Aborted));
        let next = tokio::spawn({
            let gate = gate.clone();
            let input = ask("stuck", None, Arc::new(|_, _| async { Ok(()) }.boxed()));
            async move { gate.request(input).await }
        });
        settle().await;
        gate.cancel_thread("stuck", None);
        assert_eq!(next.await.unwrap(), Ok(false));
    }

    #[tokio::test]
    async fn a_dismissal_publish_failure_cannot_convert_an_accepted_consent_into_a_rejection() {
        let gate = ComputerApprovalGate::default();
        let log = Log::default();
        let sink = log.clone();
        let publish: ApprovalPublish = Arc::new(move |id, decision| {
            if decision.is_none() {
                sink.lock().unwrap().push((id, None));
                async { Ok(()) }.boxed()
            } else {
                async { Err("socket gone".to_owned()) }.boxed()
            }
        });
        let decision = tokio::spawn({
            let gate = gate.clone();
            let input = ask("dismiss", None, publish);
            async move { gate.request(input).await }
        });
        settle().await;
        gate.respond("dismiss", &opened(&log)[0], ApprovalDecision::Accept);
        assert_eq!(decision.await.unwrap(), Ok(true));
    }

    #[tokio::test]
    async fn a_desktop_interruption_revokes_grants_but_keeps_declines_and_live_prompts() {
        let gate = ComputerApprovalGate::default();
        let logs: HashMap<&str, Log> = ["granted", "declined", "pending"]
            .into_iter()
            .map(|chat| (chat, Log::default()))
            .collect();
        let spawn = |chat: &'static str| {
            let gate = gate.clone();
            let input = ask(chat, Some("turn"), recording(&logs[chat]));
            tokio::spawn(async move { gate.request_task(input).await })
        };
        let granted = spawn("granted");
        settle().await;
        gate.respond(
            "granted",
            &opened(&logs["granted"])[0],
            ApprovalDecision::Accept,
        );
        assert_eq!(granted.await.unwrap(), Ok(true));
        let declined = spawn("declined");
        settle().await;
        gate.respond(
            "declined",
            &opened(&logs["declined"])[0],
            ApprovalDecision::Decline,
        );
        assert_eq!(declined.await.unwrap(), Ok(false));
        let pending = spawn("pending");
        settle().await;
        assert_eq!(opened(&logs["pending"]).len(), 1);
        gate.revoke_task_grants();
        let reprompted = spawn("granted");
        settle().await;
        assert_eq!(opened(&logs["granted"]).len(), 2);
        gate.respond(
            "granted",
            &opened(&logs["granted"])[1],
            ApprovalDecision::Accept,
        );
        assert_eq!(reprompted.await.unwrap(), Ok(true));
        assert_eq!(spawn("declined").await.unwrap(), Ok(false));
        assert_eq!(opened(&logs["declined"]).len(), 1);
        gate.respond(
            "pending",
            &opened(&logs["pending"])[0],
            ApprovalDecision::Accept,
        );
        assert_eq!(pending.await.unwrap(), Ok(true));
        assert_eq!(spawn("pending").await.unwrap(), Ok(true));
        assert_eq!(opened(&logs["pending"]).len(), 1);
    }

    #[tokio::test(start_paused = true)]
    async fn an_unanswered_prompt_auto_declines_at_the_timeout() {
        let gate = ComputerApprovalGate::default();
        let log = Log::default();
        let result = gate.request(ask("a", None, recording(&log))).await;
        assert_eq!(result, Ok(false));
        settle().await;
        assert_eq!(
            log.lock().unwrap().last().map(|(_, d)| *d),
            Some(Some(ApprovalDecision::Cancel))
        );
    }

    #[tokio::test]
    async fn auto_and_full_raise_no_task_or_clipboard_card() {
        let gate = ComputerApprovalGate::default();
        for mode in [PermissionMode::Auto, PermissionMode::Full] {
            let log = Log::default();
            let input = ApprovalRequest {
                mode,
                ..ask("a", Some("turn"), recording(&log))
            };
            assert_eq!(gate.request_task(input.clone()).await, Ok(true));
            assert_eq!(gate.request(input).await, Ok(true));
            assert!(log.lock().unwrap().is_empty(), "{mode:?}");
        }
    }

    #[tokio::test]
    async fn ask_raises_one_task_card_and_a_separate_clipboard_card_per_call() {
        let gate = ComputerApprovalGate::default();
        let log = Log::default();
        let input = ask("a", Some("turn"), auto_answer(&gate, "a", &log));
        assert_eq!(gate.request_task(input.clone()).await, Ok(true));
        assert_eq!(gate.request_task(input.clone()).await, Ok(true));
        assert_eq!(opened(&log).len(), 1);
        assert_eq!(gate.request(input.clone()).await, Ok(true));
        assert_eq!(gate.request(input).await, Ok(true));
        assert_eq!(opened(&log).len(), 3);
        assert!(computer_consent_card_required(PermissionMode::Ask));
        assert!(!computer_consent_card_required(PermissionMode::Auto));
        assert!(!computer_consent_card_required(PermissionMode::Full));
    }

    fn for_app(
        turn: &str,
        key: &str,
        always: Option<&str>,
        publish: ApprovalPublish,
    ) -> ApprovalRequest {
        ApprovalRequest {
            app: Some(AppGrant {
                key: key.to_owned(),
                always: always.map(|bundle_id| ComputerAllowedApp {
                    bundle_id: bundle_id.to_owned(),
                    name: None,
                }),
            }),
            ..ask("a", Some(turn), publish)
        }
    }

    fn answering(
        gate: &ComputerApprovalGate,
        log: &Log,
        decision: ApprovalDecision,
    ) -> ApprovalPublish {
        let gate = gate.clone();
        let log = log.clone();
        Arc::new(move |id, resolved| {
            if resolved.is_none() {
                log.lock().unwrap().push((id.clone(), None));
                gate.respond("a", &id, decision);
            }
            async { Ok(()) }.boxed()
        })
    }

    #[tokio::test]
    async fn an_app_grant_covers_only_that_app_and_satisfies_the_turn_grant() {
        let gate = ComputerApprovalGate::default();
        let log = Log::default();
        let accept = answering(&gate, &log, ApprovalDecision::Accept);
        let safari = for_app(
            "turn",
            "bundle:com.apple.Safari",
            Some("com.apple.Safari"),
            accept.clone(),
        );
        let notes = for_app(
            "turn",
            "bundle:com.apple.Notes",
            Some("com.apple.Notes"),
            accept.clone(),
        );
        assert_eq!(gate.request_task(safari.clone()).await, Ok(true));
        assert_eq!(opened(&log).len(), 1);
        assert_eq!(gate.request_task(notes.clone()).await, Ok(true));
        assert_eq!(opened(&log).len(), 2, "a second app raises its own card");
        assert_eq!(gate.request_task(safari).await, Ok(true));
        assert_eq!(
            gate.request_task(ask("a", Some("turn"), accept.clone()))
                .await,
            Ok(true)
        );
        assert_eq!(opened(&log).len(), 2, "the app grant covers app-less calls");
        let declined = Log::default();
        let decline = answering(&gate, &declined, ApprovalDecision::Decline);
        let calc = for_app("turn", "pid:77", None, decline);
        assert_eq!(gate.request_task(calc.clone()).await, Ok(false));
        assert_eq!(gate.request_task(calc).await, Ok(false));
        assert_eq!(opened(&declined).len(), 1, "a denial sticks for the turn");
        let next = for_app(
            "turn-2",
            "bundle:com.apple.Safari",
            Some("com.apple.Safari"),
            accept,
        );
        assert_eq!(gate.request_task(next).await, Ok(true));
        assert_eq!(opened(&log).len(), 3, "a new turn asks again");
    }

    #[tokio::test]
    async fn an_app_less_decline_is_lifted_by_a_later_app_grant_in_the_turn() {
        let gate = ComputerApprovalGate::default();
        let declined = Log::default();
        let decline = answering(&gate, &declined, ApprovalDecision::Decline);
        assert_eq!(
            gate.request_task(ask("a", Some("turn"), decline)).await,
            Ok(false)
        );
        let log = Log::default();
        let accept = answering(&gate, &log, ApprovalDecision::Accept);
        assert_eq!(
            gate.request_task(for_app("turn", "pid:9", None, accept.clone()))
                .await,
            Ok(true)
        );
        assert_eq!(
            gate.request_task(ask("a", Some("turn"), accept)).await,
            Ok(true)
        );
        assert_eq!(opened(&log).len(), 1);
    }

    #[tokio::test]
    async fn always_allow_persists_a_verified_app_until_the_list_drops_it() {
        let gate = ComputerApprovalGate::default();
        let saved: Arc<Mutex<Vec<ComputerAllowedApp>>> = Arc::default();
        let sink = saved.clone();
        gate.on_always_allowed(Arc::new(move |app| sink.lock().unwrap().push(app)));
        let log = Log::default();
        let always = answering(&gate, &log, ApprovalDecision::AcceptForSession);
        let safari = |turn: &str, publish| {
            for_app(
                turn,
                "bundle:com.apple.Safari",
                Some("com.apple.Safari"),
                publish,
            )
        };
        assert_eq!(
            gate.request_task(safari("turn-1", always.clone())).await,
            Ok(true)
        );
        assert_eq!(
            saved
                .lock()
                .unwrap()
                .iter()
                .map(|app| app.bundle_id.as_str())
                .collect::<Vec<_>>(),
            ["com.apple.Safari"]
        );
        assert_eq!(
            gate.request_task(safari("turn-2", always.clone())).await,
            Ok(true)
        );
        assert_eq!(
            gate.request_task(ask("a", Some("turn-2"), always.clone()))
                .await,
            Ok(true)
        );
        assert_eq!(
            opened(&log).len(),
            1,
            "an always-allowed app raises no card"
        );
        gate.revoke_task_grants();
        gate.cancel_thread("a", None);
        assert_eq!(
            gate.request_task(safari("turn-3", always.clone())).await,
            Ok(true)
        );
        assert_eq!(
            opened(&log).len(),
            1,
            "stop and host loss keep the always list"
        );
        gate.set_always_allowed([]);
        assert_eq!(gate.request_task(safari("turn-4", always)).await, Ok(true));
        assert_eq!(
            opened(&log).len(),
            2,
            "removing the app brings the card back"
        );
    }

    #[tokio::test]
    async fn always_allow_for_an_unverified_app_is_refused_and_not_saved() {
        let gate = ComputerApprovalGate::default();
        let saved: Arc<Mutex<Vec<ComputerAllowedApp>>> = Arc::default();
        let sink = saved.clone();
        gate.on_always_allowed(Arc::new(move |app| sink.lock().unwrap().push(app)));
        let log = Log::default();
        let always = answering(&gate, &log, ApprovalDecision::AcceptForSession);
        assert_eq!(
            gate.request_task(for_app("turn", "pid:4242", None, always))
                .await,
            Ok(false)
        );
        assert!(saved.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn the_ninth_pending_card_in_one_chat_fails_with_approval_queue_full() {
        let gate = ComputerApprovalGate::default();
        let quiet: ApprovalPublish = Arc::new(|_, _| async { Ok(()) }.boxed());
        let mut waiting = Vec::new();
        for _ in 0..8 {
            let gate = gate.clone();
            let input = ask("chat", None, quiet.clone());
            waiting.push(tokio::spawn(async move { gate.request(input).await }));
        }
        settle().await;
        assert_eq!(
            gate.request(ask("chat", None, quiet))
                .await
                .unwrap_err()
                .code(),
            Some("approval_queue_full")
        );
        gate.cancel_thread("chat", None);
        for waiter in waiting {
            waiter.await.unwrap().unwrap();
        }
    }
}
