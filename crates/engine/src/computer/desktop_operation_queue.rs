use std::collections::HashMap;
use std::future::Future;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use futures::FutureExt;
use futures::future::{BoxFuture, Shared, join_all, select_all};
use tokio::sync::oneshot;
use tokio_util::sync::CancellationToken;
use zeron_proto::computer::ComputerDeliveryMode;

pub const DESKTOP_OPERATION_QUEUE_LIMIT: usize = 64;

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum DesktopOperationError {
    #[error("Computer manager is closed.")]
    Closed,
    #[error("Too many computer operations are queued; try again later.")]
    QueueFull,
    #[error("A scoped computer operation cannot switch targets before it finishes.")]
    TargetSwitch,
    #[error("The computer operation was cancelled.")]
    Aborted,
    #[error("The computer operation has ended; no new input may be dispatched.")]
    Ended,
}

impl DesktopOperationError {
    pub fn retryable(self) -> bool {
        self == Self::QueueFull
    }
}

#[derive(Debug, Clone)]
pub struct OperationSignal(Arc<[CancellationToken]>);

impl OperationSignal {
    pub fn any(&self, other: &OperationSignal) -> OperationSignal {
        OperationSignal(self.0.iter().chain(other.0.iter()).cloned().collect())
    }

    pub fn is_aborted(&self) -> bool {
        self.0.iter().any(CancellationToken::is_cancelled)
    }

    pub fn check(&self) -> Result<(), DesktopOperationError> {
        if self.is_aborted() {
            Err(DesktopOperationError::Aborted)
        } else {
            Ok(())
        }
    }

    pub async fn aborted(&self) {
        select_all(self.0.iter().map(|token| Box::pin(token.cancelled()))).await;
    }

    pub fn same_as(&self, other: &OperationSignal) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl From<CancellationToken> for OperationSignal {
    fn from(token: CancellationToken) -> Self {
        OperationSignal(Arc::new([token]))
    }
}

fn combine(a: Option<OperationSignal>, b: Option<OperationSignal>) -> Option<OperationSignal> {
    match (a, b) {
        (Some(a), Some(b)) => Some(a.any(&b)),
        (a, b) => a.or(b),
    }
}

#[derive(Clone)]
struct ExecutionScope {
    signal: Option<OperationSignal>,
    active: Arc<AtomicBool>,
}

impl ExecutionScope {
    fn new(signal: Option<OperationSignal>) -> Self {
        Self {
            signal,
            active: Arc::new(AtomicBool::new(true)),
        }
    }
}

struct EndScope(Arc<AtomicBool>);

impl Drop for EndScope {
    fn drop(&mut self) {
        self.0.store(false, Ordering::SeqCst);
    }
}

async fn run_in_scope<F: Future>(scope: ExecutionScope, action: F) -> F::Output {
    let _end = EndScope(scope.active.clone());
    EXECUTION.scope(scope, action).await
}

#[derive(Clone)]
pub struct DesktopOperationScope {
    execution: Option<ExecutionScope>,
    frames: Arc<Vec<QueueFrame>>,
    delivery: Option<ComputerDeliveryMode>,
}

pub fn current_desktop_operation() -> DesktopOperationScope {
    DesktopOperationScope {
        execution: EXECUTION.try_with(Clone::clone).ok(),
        frames: current_frames(),
        delivery: DELIVERY.try_with(|mode| *mode).ok(),
    }
}

impl DesktopOperationScope {
    pub async fn run<F: Future>(self, action: F) -> F::Output {
        let framed = QUEUE_FRAMES.scope(self.frames, action);
        match (self.execution, self.delivery) {
            (Some(execution), Some(mode)) => {
                EXECUTION
                    .scope(execution, DELIVERY.scope(mode, framed))
                    .await
            }
            (Some(execution), None) => EXECUTION.scope(execution, framed).await,
            (None, Some(mode)) => DELIVERY.scope(mode, framed).await,
            (None, None) => framed.await,
        }
    }
}

#[derive(Clone)]
struct QueueFrame {
    queue: u64,
    key: Option<String>,
}

tokio::task_local! {
    static EXECUTION: ExecutionScope;
    static QUEUE_FRAMES: Arc<Vec<QueueFrame>>;
    static DELIVERY: ComputerDeliveryMode;
}

pub fn desktop_operation_signal() -> Option<OperationSignal> {
    EXECUTION
        .try_with(|scope| {
            scope
                .active
                .load(Ordering::SeqCst)
                .then(|| scope.signal.clone())
                .flatten()
        })
        .ok()
        .flatten()
}

pub async fn with_desktop_operation_signal<F: Future>(
    signal: OperationSignal,
    action: F,
) -> F::Output {
    let signal = match desktop_operation_signal() {
        Some(parent) => parent.any(&signal),
        None => signal,
    };
    run_in_scope(ExecutionScope::new(Some(signal)), action).await
}

pub fn assert_desktop_operation_active() -> Result<(), DesktopOperationError> {
    let ended = EXECUTION
        .try_with(|scope| !scope.active.load(Ordering::SeqCst))
        .unwrap_or(false);
    if ended {
        return Err(DesktopOperationError::Ended);
    }
    desktop_operation_signal().map_or(Ok(()), |signal| signal.check())
}

pub async fn without_desktop_cancellation<F: Future>(action: F) -> F::Output {
    EXECUTION.scope(ExecutionScope::new(None), action).await
}

pub fn desktop_delivery_mode() -> ComputerDeliveryMode {
    DELIVERY
        .try_with(|mode| *mode)
        .unwrap_or(ComputerDeliveryMode::Background)
}

pub async fn with_desktop_delivery_mode<F: Future>(
    mode: ComputerDeliveryMode,
    action: F,
) -> F::Output {
    DELIVERY.scope(mode, action).await
}

fn current_frames() -> Arc<Vec<QueueFrame>> {
    QUEUE_FRAMES.try_with(Arc::clone).unwrap_or_default()
}

type Gate = Shared<BoxFuture<'static, ()>>;

fn ready_gate() -> Gate {
    futures::future::ready(()).boxed().shared()
}

fn gate(receiver: oneshot::Receiver<()>) -> Gate {
    receiver.map(|_| ()).boxed().shared()
}

struct QueueState {
    tail: Gate,
    scoped_tails: HashMap<String, (u64, Gate)>,
    active_scoped: HashMap<u64, Gate>,
    scoped_controllers: HashMap<u64, CancellationToken>,
    active_controller: Option<(u64, CancellationToken)>,
    pending: usize,
    closed: bool,
    next_operation: u64,
}

static NEXT_QUEUE: AtomicU64 = AtomicU64::new(1);

#[derive(Clone)]
pub struct DesktopOperationQueue {
    id: u64,
    state: Arc<Mutex<QueueState>>,
}

impl Default for DesktopOperationQueue {
    fn default() -> Self {
        Self::new()
    }
}

struct PendingSlot(Arc<Mutex<QueueState>>);

impl Drop for PendingSlot {
    fn drop(&mut self) {
        lock(&self.0).pending -= 1;
    }
}

struct ScopedSlot {
    state: Arc<Mutex<QueueState>>,
    key: String,
    operation: u64,
    _settled: oneshot::Sender<()>,
}

impl Drop for ScopedSlot {
    fn drop(&mut self) {
        let mut state = lock(&self.state);
        state.pending -= 1;
        if state
            .scoped_tails
            .get(&self.key)
            .is_some_and(|(operation, _)| *operation == self.operation)
        {
            state.scoped_tails.remove(&self.key);
        }
    }
}

struct ActiveScoped {
    state: Arc<Mutex<QueueState>>,
    operation: u64,
    _finish: oneshot::Sender<()>,
}

impl Drop for ActiveScoped {
    fn drop(&mut self) {
        let mut state = lock(&self.state);
        state.active_scoped.remove(&self.operation);
        state.scoped_controllers.remove(&self.operation);
    }
}

struct ActiveExclusive {
    state: Arc<Mutex<QueueState>>,
    operation: u64,
}

impl Drop for ActiveExclusive {
    fn drop(&mut self) {
        let mut state = lock(&self.state);
        if state
            .active_controller
            .as_ref()
            .is_some_and(|(operation, _)| *operation == self.operation)
        {
            state.active_controller = None;
        }
    }
}

fn lock(state: &Mutex<QueueState>) -> MutexGuard<'_, QueueState> {
    state.lock().unwrap_or_else(PoisonError::into_inner)
}

struct ExclusiveTicket {
    previous: Gate,
    done: oneshot::Sender<()>,
    caller_signal: Option<OperationSignal>,
    slot: PendingSlot,
}

struct ScopedTicket {
    key: String,
    operation: u64,
    previous_tail: Gate,
    admitted: oneshot::Sender<()>,
    predecessor: Gate,
    finish: oneshot::Sender<()>,
    active: Gate,
    caller_signal: Option<OperationSignal>,
    slot: ScopedSlot,
}

enum Plan<T> {
    Reject(DesktopOperationError),
    Reentrant(Option<OperationSignal>),
    Queued(T),
}

async fn run_reentrant<T, E, Fut>(
    signal: Option<OperationSignal>,
    action: impl FnOnce() -> Fut,
) -> Result<T, E>
where
    Fut: Future<Output = Result<T, E>>,
    E: From<DesktopOperationError>,
{
    match signal {
        Some(signal) => {
            with_desktop_operation_signal(signal, async {
                assert_desktop_operation_active()?;
                action().await
            })
            .await
        }
        None => action().await,
    }
}

impl DesktopOperationQueue {
    pub fn new() -> Self {
        Self {
            id: NEXT_QUEUE.fetch_add(1, Ordering::Relaxed),
            state: Arc::new(Mutex::new(QueueState {
                tail: ready_gate(),
                scoped_tails: HashMap::new(),
                active_scoped: HashMap::new(),
                scoped_controllers: HashMap::new(),
                active_controller: None,
                pending: 0,
                closed: false,
                next_operation: 0,
            })),
        }
    }

    fn reentrant<T>(signal: Option<OperationSignal>) -> Plan<T> {
        match assert_desktop_operation_active() {
            Ok(()) => Plan::Reentrant(signal),
            Err(error) => Plan::Reject(error),
        }
    }

    fn plan_exclusive(&self, signal: Option<OperationSignal>) -> Plan<ExclusiveTicket> {
        let frames = current_frames();
        let mut state = lock(&self.state);
        if state.closed {
            return Plan::Reject(DesktopOperationError::Closed);
        }
        if frames.iter().any(|frame| frame.queue == self.id) {
            return Self::reentrant(signal);
        }
        if state.pending >= DESKTOP_OPERATION_QUEUE_LIMIT {
            return Plan::Reject(DesktopOperationError::QueueFull);
        }
        let caller_signal = combine(signal, desktop_operation_signal());
        state.pending += 1;
        let (done, released) = oneshot::channel();
        let previous = std::mem::replace(&mut state.tail, gate(released));
        Plan::Queued(ExclusiveTicket {
            previous,
            done,
            caller_signal,
            slot: PendingSlot(self.state.clone()),
        })
    }

    pub fn run<T, E, F, Fut>(
        &self,
        action: F,
        signal: Option<OperationSignal>,
    ) -> impl Future<Output = Result<T, E>> + use<T, E, F, Fut>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<T, E>>,
        E: From<DesktopOperationError>,
    {
        let plan = self.plan_exclusive(signal);
        let queue = self.clone();
        async move {
            let ticket = match plan {
                Plan::Reject(error) => return Err(error.into()),
                Plan::Reentrant(signal) => return run_reentrant(signal, action).await,
                Plan::Queued(ticket) => ticket,
            };
            let ExclusiveTicket {
                previous,
                done: _done,
                caller_signal,
                slot: _slot,
            } = ticket;
            previous.await;
            queue.admit(caller_signal.as_ref())?;
            let scoped: Vec<Gate> = lock(&queue.state).active_scoped.values().cloned().collect();
            join_all(scoped).await;
            queue.admit(caller_signal.as_ref())?;
            let controller = CancellationToken::new();
            let _active = {
                let mut state = lock(&queue.state);
                let operation = state.next_operation;
                state.next_operation += 1;
                state.active_controller = Some((operation, controller.clone()));
                ActiveExclusive {
                    state: queue.state.clone(),
                    operation,
                }
            };
            let signal = combine(caller_signal, Some(controller.into()));
            queue
                .enter(signal, None, async move { action().await })
                .await
        }
    }

    pub fn run_scoped<T, E, F, Fut>(
        &self,
        key: &str,
        action: F,
        signal: Option<OperationSignal>,
    ) -> impl Future<Output = Result<T, E>> + use<T, E, F, Fut>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<T, E>>,
        E: From<DesktopOperationError>,
    {
        let plan = self.plan_scoped(key, signal);
        let queue = self.clone();
        async move {
            let ticket = match plan {
                Plan::Reject(error) => return Err(error.into()),
                Plan::Reentrant(signal) => return run_reentrant(signal, action).await,
                Plan::Queued(ticket) => ticket,
            };
            let ScopedTicket {
                key,
                operation,
                previous_tail,
                admitted,
                predecessor,
                finish,
                active,
                caller_signal,
                slot: _slot,
            } = ticket;
            let _active = ActiveScoped {
                state: queue.state.clone(),
                operation,
                _finish: finish,
            };
            previous_tail.await;
            {
                let mut state = lock(&queue.state);
                if state.closed {
                    return Err(DesktopOperationError::Closed.into());
                }
                if let Some(signal) = &caller_signal {
                    signal.check()?;
                }
                state.active_scoped.insert(operation, active);
            }
            drop(admitted);
            predecessor.await;
            queue.admit(caller_signal.as_ref())?;
            let controller = CancellationToken::new();
            lock(&queue.state)
                .scoped_controllers
                .insert(operation, controller.clone());
            let signal = combine(caller_signal, Some(controller.into()));
            queue
                .enter(signal, Some(key), async move { action().await })
                .await
        }
    }

    fn plan_scoped(&self, key: &str, signal: Option<OperationSignal>) -> Plan<ScopedTicket> {
        let frames = current_frames();
        let mut state = lock(&self.state);
        if state.closed {
            return Plan::Reject(DesktopOperationError::Closed);
        }
        let own = frames.iter().rev().find(|frame| frame.queue == self.id);
        match own.map(|frame| frame.key.as_deref()) {
            Some(Some(active)) if active != key => {
                return Plan::Reject(DesktopOperationError::TargetSwitch);
            }
            Some(_) => return Self::reentrant(signal),
            None => {}
        }
        if state.pending >= DESKTOP_OPERATION_QUEUE_LIMIT {
            return Plan::Reject(DesktopOperationError::QueueFull);
        }
        let caller_signal = combine(signal, desktop_operation_signal());
        let predecessor = state
            .scoped_tails
            .get(key)
            .map_or_else(ready_gate, |(_, tail)| tail.clone());
        state.pending += 1;
        let operation = state.next_operation;
        state.next_operation += 1;
        let (finish, finished) = oneshot::channel();
        let (admitted, admission) = oneshot::channel();
        let previous_tail = std::mem::replace(&mut state.tail, gate(admission));
        let (settled, settlement) = oneshot::channel();
        state
            .scoped_tails
            .insert(key.to_owned(), (operation, gate(settlement)));
        Plan::Queued(ScopedTicket {
            key: key.to_owned(),
            operation,
            previous_tail,
            admitted,
            predecessor,
            finish,
            active: gate(finished),
            caller_signal,
            slot: ScopedSlot {
                state: self.state.clone(),
                key: key.to_owned(),
                operation,
                _settled: settled,
            },
        })
    }

    fn admit(&self, caller_signal: Option<&OperationSignal>) -> Result<(), DesktopOperationError> {
        if lock(&self.state).closed {
            return Err(DesktopOperationError::Closed);
        }
        caller_signal.map_or(Ok(()), OperationSignal::check)
    }

    async fn enter<F: Future>(
        &self,
        signal: Option<OperationSignal>,
        key: Option<String>,
        action: F,
    ) -> F::Output {
        let mut frames = (*current_frames()).clone();
        frames.push(QueueFrame {
            queue: self.id,
            key,
        });
        run_in_scope(
            ExecutionScope::new(signal),
            QUEUE_FRAMES.scope(Arc::new(frames), action),
        )
        .await
    }

    pub fn close(&self) -> impl Future<Output = ()> + use<> {
        let (tail, scoped) = {
            let mut state = lock(&self.state);
            state.closed = true;
            if let Some((_, controller)) = &state.active_controller {
                controller.cancel();
            }
            for controller in state.scoped_controllers.values() {
                controller.cancel();
            }
            let scoped: Vec<Gate> = state
                .scoped_tails
                .values()
                .map(|(_, tail)| tail.clone())
                .collect();
            (state.tail.clone(), scoped)
        };
        async move {
            tail.await;
            join_all(scoped).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::AtomicUsize;

    use anyhow::{Result, anyhow};

    use super::*;

    fn deferred() -> (oneshot::Sender<()>, Gate) {
        let (release, held) = oneshot::channel();
        (release, gate(held))
    }

    fn events() -> Arc<Mutex<Vec<&'static str>>> {
        Arc::default()
    }

    fn push(events: &Arc<Mutex<Vec<&'static str>>>, event: &'static str) {
        events.lock().unwrap().push(event);
    }

    fn seen(events: &Arc<Mutex<Vec<&'static str>>>) -> Vec<&'static str> {
        events.lock().unwrap().clone()
    }

    async fn settle() {
        for _ in 0..32 {
            tokio::task::yield_now().await;
        }
    }

    fn queue_error(error: &anyhow::Error) -> Option<DesktopOperationError> {
        error.downcast_ref::<DesktopOperationError>().copied()
    }

    #[tokio::test]
    async fn holds_the_desktop_until_input_and_observation_finish_then_recovers_after_a_failure() {
        let queue = DesktopOperationQueue::new();
        let (release, held) = deferred();
        let (entered, entered_gate) = deferred();
        let log = events();
        let inner_queue = queue.clone();
        let first_log = log.clone();
        let first = tokio::spawn(queue.run(
            move || async move {
                inner_queue
                    .run(
                        || async {
                            push(&first_log, "input");
                            Ok::<_, anyhow::Error>(())
                        },
                        None,
                    )
                    .await?;
                let _ = entered.send(());
                held.await;
                push(&first_log, "observation");
                Err::<(), _>(anyhow!("capture failed"))
            },
            None,
        ));
        entered_gate.await;
        let second_log = log.clone();
        let second = tokio::spawn(queue.run(
            move || async move {
                push(&second_log, "next input");
                Ok::<_, anyhow::Error>(())
            },
            None,
        ));
        settle().await;
        assert_eq!(seen(&log), ["input"]);
        release.send(()).unwrap();
        assert_eq!(
            first.await.unwrap().unwrap_err().to_string(),
            "capture failed"
        );
        second.await.unwrap().unwrap();
        assert_eq!(seen(&log), ["input", "observation", "next input"]);
    }

    #[tokio::test]
    async fn skips_an_aborted_operation_before_it_can_send_input() {
        let queue = DesktopOperationQueue::new();
        let (release, held) = deferred();
        let first = tokio::spawn(queue.run(
            move || async move {
                held.await;
                Ok::<_, anyhow::Error>(())
            },
            None,
        ));
        let controller = CancellationToken::new();
        let ran = Arc::new(AtomicUsize::new(0));
        let flag = ran.clone();
        let second = tokio::spawn(queue.run(
            move || async move {
                flag.fetch_add(1, Ordering::SeqCst);
                Ok::<_, anyhow::Error>(())
            },
            Some(controller.clone().into()),
        ));
        controller.cancel();
        release.send(()).unwrap();
        first.await.unwrap().unwrap();
        let error = second.await.unwrap().unwrap_err();
        assert_eq!(queue_error(&error), Some(DesktopOperationError::Aborted));
        assert_eq!(ran.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn preserves_inherited_and_explicit_cancellation_together() {
        for cancel_inherited in [true, false] {
            let queue = DesktopOperationQueue::new();
            let (release, held) = deferred();
            let first = tokio::spawn(queue.run(
                move || async move {
                    held.await;
                    Ok::<_, anyhow::Error>(())
                },
                None,
            ));
            let inherited = CancellationToken::new();
            let explicit = CancellationToken::new();
            let ran = Arc::new(AtomicUsize::new(0));
            let flag = ran.clone();
            let inner_queue = queue.clone();
            let explicit_signal: OperationSignal = explicit.clone().into();
            let second = tokio::spawn(with_desktop_operation_signal(
                inherited.clone().into(),
                async move {
                    inner_queue
                        .run(
                            move || async move {
                                flag.fetch_add(1, Ordering::SeqCst);
                                Ok::<_, anyhow::Error>(())
                            },
                            Some(explicit_signal),
                        )
                        .await
                },
            ));
            settle().await;
            if cancel_inherited {
                inherited.cancel();
            } else {
                explicit.cancel();
            }
            release.send(()).unwrap();
            first.await.unwrap().unwrap();
            let error = second.await.unwrap().unwrap_err();
            assert_eq!(queue_error(&error), Some(DesktopOperationError::Aborted));
            assert_eq!(ran.load(Ordering::SeqCst), 0);
            queue.close().await;
        }
    }

    #[tokio::test]
    async fn composes_reentrant_cancellation_without_losing_the_outer_transaction() {
        let queue = DesktopOperationQueue::new();
        let inner = CancellationToken::new();
        let inner_queue = queue.clone();
        queue
            .run(
                move || async move {
                    let outer = desktop_operation_signal().unwrap();
                    let cancel = inner.clone();
                    inner_queue
                        .run(
                            move || async move {
                                let nested = desktop_operation_signal().unwrap();
                                assert!(!nested.is_aborted());
                                cancel.cancel();
                                assert!(nested.is_aborted());
                                Ok::<_, anyhow::Error>(())
                            },
                            Some(inner.into()),
                        )
                        .await?;
                    let restored = desktop_operation_signal().unwrap();
                    assert!(restored.same_as(&outer));
                    assert!(!outer.is_aborted());
                    Ok::<_, anyhow::Error>(())
                },
                None,
            )
            .await
            .unwrap();
        queue.close().await;
    }

    #[tokio::test]
    async fn runs_independent_scoped_targets_concurrently_and_orders_the_same_target() {
        let queue = DesktopOperationQueue::new();
        let (release_a, held_a) = deferred();
        let (release_b, held_b) = deferred();
        let (entered_a, entered_a_gate) = deferred();
        let (entered_b, entered_b_gate) = deferred();
        let log = events();
        let (l1, l2, l3) = (log.clone(), log.clone(), log.clone());
        let first_a = tokio::spawn(queue.run_scoped(
            "window-a",
            move || async move {
                push(&l1, "a1");
                let _ = entered_a.send(());
                held_a.await;
                Ok::<_, anyhow::Error>(())
            },
            None,
        ));
        let second_a = tokio::spawn(queue.run_scoped(
            "window-a",
            move || async move {
                push(&l2, "a2");
                Ok::<_, anyhow::Error>(())
            },
            None,
        ));
        let first_b = tokio::spawn(queue.run_scoped(
            "window-b",
            move || async move {
                push(&l3, "b1");
                let _ = entered_b.send(());
                held_b.await;
                Ok::<_, anyhow::Error>(())
            },
            None,
        ));
        entered_a_gate.await;
        entered_b_gate.await;
        assert_eq!(seen(&log), ["a1", "b1"]);
        release_a.send(()).unwrap();
        first_a.await.unwrap().unwrap();
        second_a.await.unwrap().unwrap();
        assert_eq!(seen(&log), ["a1", "b1", "a2"]);
        release_b.send(()).unwrap();
        first_b.await.unwrap().unwrap();
        queue.close().await;
    }

    #[tokio::test]
    async fn keeps_exclusive_work_ahead_of_later_scoped_admissions() {
        let queue = DesktopOperationQueue::new();
        let (release, held) = deferred();
        let (entered, entered_gate) = deferred();
        let log = events();
        let (l1, l2, l3) = (log.clone(), log.clone(), log.clone());
        let scoped = tokio::spawn(queue.run_scoped(
            "window-a",
            move || async move {
                push(&l1, "scoped");
                let _ = entered.send(());
                held.await;
                Ok::<_, anyhow::Error>(())
            },
            None,
        ));
        entered_gate.await;
        let exclusive = tokio::spawn(queue.run(
            move || async move {
                push(&l2, "exclusive");
                Ok::<_, anyhow::Error>(())
            },
            None,
        ));
        let later = tokio::spawn(queue.run_scoped(
            "window-b",
            move || async move {
                push(&l3, "later scoped");
                Ok::<_, anyhow::Error>(())
            },
            None,
        ));
        settle().await;
        assert_eq!(seen(&log), ["scoped"]);
        release.send(()).unwrap();
        scoped.await.unwrap().unwrap();
        exclusive.await.unwrap().unwrap();
        later.await.unwrap().unwrap();
        assert_eq!(seen(&log), ["scoped", "exclusive", "later scoped"]);
        queue.close().await;
    }

    #[tokio::test]
    async fn does_not_let_exclusive_work_overtake_an_earlier_same_target_operation() {
        let queue = DesktopOperationQueue::new();
        let (release, held) = deferred();
        let (entered, entered_gate) = deferred();
        let log = events();
        let (l1, l2, l3) = (log.clone(), log.clone(), log.clone());
        let first = tokio::spawn(queue.run_scoped(
            "window-a",
            move || async move {
                push(&l1, "first");
                let _ = entered.send(());
                held.await;
                Ok::<_, anyhow::Error>(())
            },
            None,
        ));
        entered_gate.await;
        let second = tokio::spawn(queue.run_scoped(
            "window-a",
            move || async move {
                push(&l2, "second");
                Ok::<_, anyhow::Error>(())
            },
            None,
        ));
        let exclusive = tokio::spawn(queue.run(
            move || async move {
                push(&l3, "exclusive");
                Ok::<_, anyhow::Error>(())
            },
            None,
        ));
        release.send(()).unwrap();
        first.await.unwrap().unwrap();
        second.await.unwrap().unwrap();
        exclusive.await.unwrap().unwrap();
        assert_eq!(seen(&log), ["first", "second", "exclusive"]);
        queue.close().await;
    }

    #[tokio::test]
    async fn refuses_a_scoped_operation_that_switches_targets_mid_flight() {
        let queue = DesktopOperationQueue::new();
        let inner_queue = queue.clone();
        let error = queue
            .run_scoped(
                "window-a",
                move || async move {
                    inner_queue
                        .run_scoped("window-a", || async { Ok::<_, anyhow::Error>(()) }, None)
                        .await?;
                    inner_queue
                        .run_scoped("window-b", || async { Ok::<_, anyhow::Error>(()) }, None)
                        .await
                },
                None,
            )
            .await
            .unwrap_err();
        assert_eq!(
            queue_error(&error),
            Some(DesktopOperationError::TargetSwitch)
        );
        queue.close().await;
    }

    #[tokio::test]
    async fn bounds_the_backlog_and_drains_active_input_before_closing() {
        let queue = DesktopOperationQueue::new();
        let (release, held) = deferred();
        let (entered, entered_gate) = deferred();
        let first = tokio::spawn(queue.run(
            move || async move {
                let _ = entered.send(());
                held.await;
                Ok::<_, anyhow::Error>(())
            },
            None,
        ));
        entered_gate.await;
        let queued_runs = Arc::new(AtomicUsize::new(0));
        let waiting: Vec<_> = (0..DESKTOP_OPERATION_QUEUE_LIMIT - 1)
            .map(|_| {
                let runs = queued_runs.clone();
                tokio::spawn(queue.run(
                    move || async move {
                        runs.fetch_add(1, Ordering::SeqCst);
                        Ok::<_, anyhow::Error>(())
                    },
                    None,
                ))
            })
            .collect();
        let full = queue
            .run(|| async { Ok::<_, anyhow::Error>(()) }, None)
            .await
            .unwrap_err();
        assert_eq!(queue_error(&full), Some(DesktopOperationError::QueueFull));
        assert!(DesktopOperationError::QueueFull.retryable());
        let closed = tokio::spawn(queue.close());
        release.send(()).unwrap();
        first.await.unwrap().unwrap();
        closed.await.unwrap();
        for waiter in waiting {
            let error = waiter.await.unwrap().unwrap_err();
            assert_eq!(queue_error(&error), Some(DesktopOperationError::Closed));
        }
        assert_eq!(queued_runs.load(Ordering::SeqCst), 0);
        let after = queue
            .run(|| async { Ok::<_, anyhow::Error>(()) }, None)
            .await
            .unwrap_err();
        assert_eq!(queue_error(&after), Some(DesktopOperationError::Closed));
    }

    #[tokio::test]
    async fn cancels_running_native_work_before_completing_shutdown() {
        let queue = DesktopOperationQueue::new();
        let (entered, entered_gate) = deferred();
        let running = tokio::spawn(queue.run(
            move || async move {
                let signal = desktop_operation_signal().unwrap();
                let _ = entered.send(());
                signal.aborted().await;
                assert!(signal.is_aborted());
                Ok::<_, anyhow::Error>(())
            },
            None,
        ));
        entered_gate.await;
        queue.close().await;
        running.await.unwrap().unwrap();
    }

    #[tokio::test]
    async fn does_not_retain_a_cancelled_turns_signal_in_detached_background_work() {
        let queue = DesktopOperationQueue::new();
        let controller = CancellationToken::new();
        let (release, held) = deferred();
        let detached = queue
            .run(
                move || async move {
                    assert!(!desktop_operation_signal().unwrap().is_aborted());
                    Ok::<_, anyhow::Error>(tokio::spawn(async move {
                        held.await;
                        desktop_operation_signal()
                    }))
                },
                Some(controller.clone().into()),
            )
            .await
            .unwrap();
        controller.cancel();
        release.send(()).unwrap();
        assert!(detached.await.unwrap().is_none());
        queue.close().await;
    }

    #[tokio::test]
    async fn expires_a_composed_operation_signal_before_detached_cosmetic_work_resumes() {
        let controller = CancellationToken::new();
        let signal: OperationSignal = controller.clone().into();
        let (release, held) = deferred();
        let (inside, detached) = with_desktop_operation_signal(signal.clone(), async move {
            let inside = desktop_operation_signal().unwrap().same_as(&signal);
            let detached = tokio::spawn(async move {
                held.await;
                desktop_operation_signal()
            });
            (inside, detached)
        })
        .await;
        assert!(inside);
        controller.cancel();
        release.send(()).unwrap();
        assert!(detached.await.unwrap().is_none());
    }

    #[tokio::test]
    async fn delivery_mode_defaults_to_background_and_scopes_foreground() {
        assert_eq!(desktop_delivery_mode(), ComputerDeliveryMode::Background);
        let inside = with_desktop_delivery_mode(ComputerDeliveryMode::Foreground, async {
            desktop_delivery_mode()
        })
        .await;
        assert_eq!(inside, ComputerDeliveryMode::Foreground);
        assert_eq!(desktop_delivery_mode(), ComputerDeliveryMode::Background);
    }

    #[tokio::test]
    async fn without_cancellation_hides_the_callers_signal() {
        let controller = CancellationToken::new();
        controller.cancel();
        let result: Result<()> = with_desktop_operation_signal(controller.into(), async {
            assert!(assert_desktop_operation_active().is_err());
            without_desktop_cancellation(async {
                assert_desktop_operation_active()?;
                Ok(())
            })
            .await
        })
        .await;
        result.unwrap();
    }

    #[tokio::test]
    async fn a_captured_continuation_is_refused_once_its_operation_has_ended() {
        let queue = DesktopOperationQueue::new();
        let (release, held) = oneshot::channel::<()>();
        let detached = queue
            .run(
                || async move {
                    let scope = current_desktop_operation();
                    assert!(desktop_operation_signal().is_some());
                    Ok::<_, DesktopOperationError>(tokio::spawn(scope.run(async move {
                        let _ = held.await;
                        (
                            assert_desktop_operation_active(),
                            desktop_operation_signal().is_none(),
                        )
                    })))
                },
                None,
            )
            .await
            .unwrap();
        release.send(()).unwrap();
        assert_eq!(
            detached.await.unwrap(),
            (Err(DesktopOperationError::Ended), true)
        );
        assert_eq!(assert_desktop_operation_active(), Ok(()));
        let fresh = queue
            .run(|| async { assert_desktop_operation_active() }, None)
            .await;
        assert_eq!(fresh, Ok(()));
    }

    #[tokio::test]
    async fn a_captured_continuation_still_runs_while_its_operation_is_live() {
        let live = with_desktop_operation_signal(CancellationToken::new().into(), async {
            let scope = current_desktop_operation();
            tokio::spawn(scope.run(async { assert_desktop_operation_active() }))
                .await
                .unwrap()
        })
        .await;
        assert_eq!(live, Ok(()));
    }
}
