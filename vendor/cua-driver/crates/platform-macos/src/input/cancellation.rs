//! Interruption and retirement of one embedded driver's native input.
//! Interruptions reopen only after every admitted action and release drains;
//! retirement remains irreversible until the host replaces the process.
//! Native release events stay on their originating thread and are prepared
//! before posting a down event, so cancellation never has to allocate them.

use std::collections::HashMap;
use std::sync::{Condvar, Mutex, MutexGuard, OnceLock};
use std::time::{Duration, Instant};

#[derive(Default)]
struct State {
    stopped: bool,
    interrupted: bool,
    epoch: u64,
    held: usize,
    operations: usize,
    unconfirmed_releases: usize,
    target: Option<super::target_guard::InputTarget>,
    concurrent_targets: HashMap<u64, super::target_guard::InputTarget>,
    next_target_id: u64,
    target_invalidated: bool,
}

#[derive(Default)]
pub struct InputGate {
    state: Mutex<State>,
    changed: Condvar,
}

pub struct InputInterruption {
    pub pending: usize,
    pub epoch: u64,
    pub resumed: bool,
}

pub fn gate() -> &'static InputGate {
    static GATE: OnceLock<InputGate> = OnceLock::new();
    GATE.get_or_init(InputGate::default)
}

fn concurrent_target_admission_open(
    state: &State,
    target: &super::target_guard::InputTarget,
) -> bool {
    let same_target_active = state
        .concurrent_targets
        .values()
        .any(|active| active.pid == target.pid && active.window_id == target.window_id);
    concurrent_target_counts_admissible(
        state.operations,
        state.target.is_some(),
        state.concurrent_targets.len(),
        same_target_active,
    )
}

fn concurrent_target_counts_admissible(
    operations: usize,
    has_exclusive_target: bool,
    concurrent_targets: usize,
    same_target_active: bool,
) -> bool {
    !has_exclusive_target && !same_target_active && operations == concurrent_targets
}

impl InputGate {
    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|error| error.into_inner())
    }

    pub fn ensure_active(&self) -> anyhow::Result<()> {
        self.check_target()
    }

    /// Capture before asynchronous preflight, then present at lease admission.
    /// A request that predates an interrupt must never acquire a fresh lease.
    pub fn admission_epoch(&self) -> anyhow::Result<u64> {
        let state = self.lock();
        Self::validate_admission(&state, state.epoch)?;
        Ok(state.epoch)
    }

    /// Host-captured admission survives socket buffering and async dispatch.
    /// Never replace it with the current epoch after an interruption reopens.
    pub fn validate_epoch(&self, epoch: u64) -> anyhow::Result<u64> {
        Self::validate_admission(&self.lock(), epoch)?;
        Ok(epoch)
    }

    fn validate_admission(state: &State, epoch: u64) -> anyhow::Result<()> {
        if state.stopped || state.interrupted || state.epoch != epoch {
            anyhow::bail!("native_input_cancelled: action admission was interrupted");
        }
        Ok(())
    }

    fn validate_target(state: &mut State) -> anyhow::Result<()> {
        if state.stopped {
            anyhow::bail!("native_input_cancelled: driver input generation is retired");
        }
        if state.interrupted {
            anyhow::bail!(
                "native_input_cancelled: input was interrupted; do not replay this action"
            );
        }
        if state.target_invalidated {
            anyhow::bail!(
                "native_input_target_changed: the exact input target changed during input; do not replay this action"
            );
        }
        if let Some(target) = &state.target {
            if let Err(error) = target.verify() {
                state.target_invalidated = true;
                return Err(error.into());
            }
        }
        for target in state.concurrent_targets.values() {
            if let Err(error) = target.verify() {
                state.target_invalidated = true;
                return Err(error.into());
            }
        }
        Ok(())
    }

    pub fn check_target(&self) -> anyhow::Result<()> {
        Self::validate_target(&mut self.lock())
    }

    /// A Space change cancels focus-sensitive actions, never the reusable input
    /// generation. Exact semantic actions revalidate their unchanged window
    /// membership before every mutation and do not follow the user's Space.
    pub fn target_space_changed(&self) {
        let mut state = self.lock();
        let exclusive_requires_active_space = state
            .target
            .as_ref()
            .is_some_and(super::target_guard::InputTarget::requires_active_space);
        let concurrent_requires_active_space = state
            .concurrent_targets
            .values()
            .filter(|target| target.requires_active_space())
            .count();
        if active_space_change_requires_invalidation(
            exclusive_requires_active_space,
            concurrent_requires_active_space,
        ) {
            state.target_invalidated = true;
            self.changed.notify_all();
        }
    }

    fn modal_element_facts(
        &self,
        element: Option<usize>,
    ) -> Option<super::modal_guard::ElementFacts> {
        let element = element?;
        let armed = {
            let state = self.lock();
            state
                .target
                .as_ref()
                .is_some_and(|target| target.has_modal())
                || state
                    .concurrent_targets
                    .values()
                    .any(|target| target.has_modal())
        };
        // SAFETY: callers pass an element they keep retained for the call.
        armed.then(|| unsafe { super::modal_guard::ElementFacts::gather(element) })
    }

    fn validate_modal_target(
        state: &State,
        addressed_element: bool,
        facts: Option<&super::modal_guard::ElementFacts>,
    ) -> anyhow::Result<()> {
        if let Some(target) = &state.target {
            target.verify_modal(addressed_element, facts)?;
            return Ok(());
        }
        if !state
            .concurrent_targets
            .values()
            .any(|target| target.has_modal())
        {
            return Ok(());
        }
        if addressed_element {
            let window_id = facts.and_then(super::modal_guard::ElementFacts::window_id);
            let target = state
                .concurrent_targets
                .values()
                .find(|target| Some(target.window_id) == window_id)
                .ok_or_else(|| {
                    anyhow::anyhow!(
                        "modal_target_mismatch: the semantic element's window cannot be proven"
                    )
                })?;
            target.verify_modal(true, facts)?;
        } else {
            for target in state.concurrent_targets.values() {
                target.verify_modal(false, None)?;
            }
        }
        Ok(())
    }

    pub fn check_modal_target(&self, element: Option<usize>) -> anyhow::Result<()> {
        let facts = self.modal_element_facts(element);
        let mut state = self.lock();
        Self::validate_target(&mut state)?;
        Self::validate_modal_target(&state, element.is_some(), facts.as_ref())
    }

    /// Semantic dispatch may address an exact descendant of a normal modal.
    /// Raw events have no such ancestry proof and retain the stricter gate.
    pub fn check_ax_target(&self, element: usize) -> anyhow::Result<()> {
        self.check_modal_target(Some(element))
    }

    pub fn post_ax<T>(&self, element: usize, send: impl FnOnce() -> T) -> anyhow::Result<T> {
        let facts = self.modal_element_facts(Some(element));
        let mut state = self.lock();
        Self::validate_target(&mut state)?;
        Self::validate_modal_target(&state, true, facts.as_ref())?;
        Ok(send())
    }

    /// Serialize one non-held event with cancellation admission.
    pub fn post<T>(&self, send: impl FnOnce() -> T) -> anyhow::Result<T> {
        let mut state = self.lock();
        Self::validate_target(&mut state)?;
        Self::validate_modal_target(&state, false, None)?;
        Ok(send())
    }

    /// Covers the complete daemon action, including native focus restoration
    /// and verification after the last matching up event was submitted.
    pub fn lease(&self) -> anyhow::Result<InputLease<'_>> {
        self.lease_with_target(None, self.admission_epoch()?)
    }

    pub fn lease_with_target(
        &self,
        target: Option<super::target_guard::InputTarget>,
        epoch: u64,
    ) -> anyhow::Result<InputLease<'_>> {
        let mut state = self.lock();
        Self::validate_admission(&state, epoch)?;
        if !state.concurrent_targets.is_empty()
            || (state.operations != 0 && (state.target.is_some() || target.is_some()))
        {
            anyhow::bail!("native_input_busy: an exact-target action is already active");
        }
        if let Some(target) = &target {
            target.verify()?;
        }
        if target.is_some() {
            state.target = target;
            state.target_invalidated = false;
        }
        state.operations += 1;
        Ok(InputLease {
            gate: self,
            concurrent_target_id: None,
        })
    }

    /// Admit an exact semantic mutation alongside other exact semantic
    /// mutations. The shared gate still validates every active target before
    /// each actuator call and remains the generation cleanup barrier.
    pub fn lease_with_concurrent_target(
        &self,
        target: super::target_guard::InputTarget,
        epoch: u64,
    ) -> anyhow::Result<InputLease<'_>> {
        let mut state = self.lock();
        Self::validate_admission(&state, epoch)?;
        if !concurrent_target_admission_open(&state, &target) {
            anyhow::bail!(
                "native_input_busy: an exclusive or same-target input action is already active"
            );
        }
        Self::validate_target(&mut state)?;
        target.verify()?;
        let target_id = state.next_target_id;
        state.next_target_id = state.next_target_id.wrapping_add(1);
        state.concurrent_targets.insert(target_id, target);
        state.operations += 1;
        Ok(InputLease {
            gate: self,
            concurrent_target_id: Some(target_id),
        })
    }

    /// `release` must already own everything needed to post the matching up.
    /// The guard releases on ordinary return, early failure and Rust unwind.
    pub fn press<R: FnOnce()>(
        &self,
        down: impl FnOnce(),
        release: R,
    ) -> anyhow::Result<HeldInput<'_, R>> {
        self.press_ax(None, down, release)
    }

    pub fn press_ax<R: FnOnce()>(
        &self,
        element: Option<usize>,
        down: impl FnOnce(),
        release: R,
    ) -> anyhow::Result<HeldInput<'_, R>> {
        let facts = self.modal_element_facts(element);
        // Declared before the mutex guard so unwinding unlocks before release.
        let mut held = HeldInput {
            gate: self,
            release: None,
        };
        let mut state = self.lock();
        Self::validate_target(&mut state)?;
        Self::validate_modal_target(&state, element.is_some(), facts.as_ref())?;
        held.release = Some(release);
        state.held += 1;
        down();
        drop(state);
        Ok(held)
    }

    pub fn pause(&self, duration: Duration) -> anyhow::Result<()> {
        let (mut state, _) = self
            .changed
            .wait_timeout_while(self.lock(), duration, |state| {
                !state.stopped && !state.interrupted && !state.target_invalidated
            })
            .unwrap_or_else(|error| error.into_inner());
        Self::validate_target(&mut state)
    }

    /// Read-only observers stop waiting during either interruption or final
    /// retirement; this never admits or releases input itself.
    pub fn is_stopped(&self) -> bool {
        let state = self.lock();
        state.stopped || state.interrupted
    }

    /// A CDP release with no acknowledgement cannot be proven by an OS key-up.
    /// Preserve the unresolved release in cleanup accounting and close this
    /// generation; later interrupts must never manufacture a successful drain.
    pub fn has_unconfirmed_releases(&self) -> bool {
        self.lock().unconfirmed_releases != 0
    }

    pub fn mark_unconfirmed_release(&self) {
        let mut state = self.lock();
        state.unconfirmed_releases = state.unconfirmed_releases.saturating_add(1);
        state.interrupted = true;
        self.changed.notify_all();
    }

    /// Wake input loops, let their original owners post matching releases,
    /// and reopen only after the full native action contexts have drained.
    /// A timed-out drain stays closed and can be checked again by the host.
    pub fn interrupt_and_wait(&self, timeout: Duration) -> InputInterruption {
        let deadline = Instant::now() + timeout;
        let mut state = self.lock();
        state.interrupted = true;
        state.epoch = state.epoch.wrapping_add(1);
        let epoch = state.epoch;
        self.changed.notify_all();
        state = self.wait_for_drain(state, deadline);
        let pending = state.held + state.operations + state.unconfirmed_releases;
        let resumed = pending == 0 && !state.stopped && state.epoch == epoch;
        if resumed {
            state.interrupted = false;
        }
        InputInterruption {
            pending,
            epoch,
            resumed,
        }
    }

    /// Waits for registered releases and action contexts, within a bounded
    /// deadline. A nonzero result is NOT permission to kill or replace us.
    pub fn cancel_and_wait(&self, timeout: Duration) -> usize {
        let deadline = Instant::now() + timeout;
        let mut state = self.lock();
        state.stopped = true;
        self.changed.notify_all();
        state = self.wait_for_drain(state, deadline);
        state.held + state.operations + state.unconfirmed_releases
    }

    fn wait_for_drain<'a>(
        &self,
        mut state: MutexGuard<'a, State>,
        deadline: Instant,
    ) -> MutexGuard<'a, State> {
        while state.held != 0 || state.operations != 0 {
            let Some(remaining) = deadline.checked_duration_since(Instant::now()) else {
                break;
            };
            let (next, waited) = self
                .changed
                .wait_timeout(state, remaining)
                .unwrap_or_else(|error| error.into_inner());
            state = next;
            if waited.timed_out() {
                break;
            }
        }
        state
    }
}

fn active_space_change_requires_invalidation(
    exclusive_requires_active_space: bool,
    concurrent_requires_active_space: usize,
) -> bool {
    exclusive_requires_active_space || concurrent_requires_active_space > 0
}

pub struct InputLease<'a> {
    gate: &'a InputGate,
    concurrent_target_id: Option<u64>,
}

impl Drop for InputLease<'_> {
    fn drop(&mut self) {
        let mut state = self.gate.lock();
        state.operations -= 1;
        if let Some(target_id) = self.concurrent_target_id {
            state.concurrent_targets.remove(&target_id);
        }
        if state.operations == 0 {
            state.target = None;
            state.concurrent_targets.clear();
            state.target_invalidated = false;
        }
        self.gate.changed.notify_all();
    }
}

pub struct HeldInput<'a, R: FnOnce()> {
    gate: &'a InputGate,
    release: Option<R>,
}

impl<R: FnOnce()> Drop for HeldInput<'_, R> {
    fn drop(&mut self) {
        if let Some(release) = self.release.take() {
            let mut state = self.gate.lock();
            release();
            state.held -= 1;
            self.gate.changed.notify_all();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{mpsc, Arc};

    #[test]
    fn interruption_drains_held_input_prevents_later_events_and_preserves_next_action() {
        let gate = Arc::new(InputGate::default());
        let old_epoch = gate.admission_epoch().unwrap();
        let worker_gate = Arc::clone(&gate);
        let (events, received) = mpsc::channel();
        let worker = std::thread::spawn(move || {
            let _action = worker_gate.lease_with_target(None, old_epoch).unwrap();
            let held = worker_gate
                .press(
                    || events.send("down").unwrap(),
                    || events.send("up").unwrap(),
                )
                .unwrap();
            assert!(worker_gate.pause(Duration::from_secs(60)).is_err());
            assert!(worker_gate
                .post(|| events.send("late-down").unwrap())
                .is_err());
            drop(held);
        });
        assert_eq!(
            received.recv_timeout(Duration::from_secs(1)).unwrap(),
            "down"
        );
        let stopped = gate.interrupt_and_wait(Duration::from_secs(1));
        assert_eq!(stopped.pending, 0);
        assert!(stopped.resumed);
        assert!(stopped.epoch > old_epoch);
        worker.join().unwrap();
        assert_eq!(received.try_iter().collect::<Vec<_>>(), ["up"]);
        // A preflight captured before Escape cannot join the reopened epoch.
        assert!(gate.lease_with_target(None, old_epoch).is_err());
        let _next = gate.lease().unwrap();
        assert!(gate.post(|| ()).is_ok());
    }

    #[test]
    fn a_buffered_host_epoch_cannot_be_resampled_after_interruption() {
        let gate = InputGate::default();
        // The host creates the envelope before sending, but the daemon may
        // not parse it until after an independently accepted interrupt RPC.
        let buffered_host_epoch = gate.admission_epoch().unwrap();
        let interrupted = gate.interrupt_and_wait(Duration::ZERO);
        assert!(interrupted.resumed);
        assert!(gate.validate_epoch(buffered_host_epoch).is_err());
        assert!(gate.lease_with_target(None, buffered_host_epoch).is_err());
        assert_eq!(
            gate.validate_epoch(interrupted.epoch).unwrap(),
            interrupted.epoch
        );
        assert!(gate.lease_with_target(None, interrupted.epoch).is_ok());
    }

    #[test]
    fn interruption_timeout_stays_closed_until_a_later_confirmed_drain() {
        let gate = InputGate::default();
        let old_epoch = gate.admission_epoch().unwrap();
        let action = gate.lease().unwrap();
        let held = gate.press(|| (), || ()).unwrap();
        let stopped = gate.interrupt_and_wait(Duration::ZERO);
        assert_eq!(stopped.pending, 2);
        assert!(!stopped.resumed);
        assert!(gate.lease().is_err());
        assert!(gate
            .post(|| panic!("input escaped the cleanup barrier"))
            .is_err());
        drop(held);
        drop(action);
        assert!(gate.lease().is_err());
        let completed = gate.interrupt_and_wait(Duration::ZERO);
        assert_eq!(completed.pending, 0);
        assert!(completed.resumed);
        assert!(gate.lease_with_target(None, old_epoch).is_err());
        assert!(gate.lease().is_ok());
    }

    #[test]
    fn uncertain_browser_release_cannot_be_cleared_by_later_interrupts() {
        let gate = InputGate::default();
        let action = gate.lease().unwrap();
        gate.mark_unconfirmed_release();
        drop(action);
        let interrupted = gate.interrupt_and_wait(Duration::ZERO);
        assert_eq!(interrupted.pending, 1);
        assert!(!interrupted.resumed);
        assert!(gate.lease().is_err());
        assert_eq!(gate.interrupt_and_wait(Duration::ZERO).pending, 1);
        assert_eq!(gate.cancel_and_wait(Duration::ZERO), 1);
    }

    #[test]
    fn interruption_never_reopens_a_retired_generation() {
        let gate = InputGate::default();
        assert_eq!(gate.cancel_and_wait(Duration::ZERO), 0);
        let interrupted = gate.interrupt_and_wait(Duration::ZERO);
        assert_eq!(interrupted.pending, 0);
        assert!(!interrupted.resumed);
        assert!(gate.lease().is_err());
        assert!(gate.post(|| panic!("retired input reopened")).is_err());
    }

    #[test]
    fn active_space_change_only_invalidates_space_bound_targets() {
        assert!(!active_space_change_requires_invalidation(false, 0));
        assert!(active_space_change_requires_invalidation(true, 0));
        assert!(active_space_change_requires_invalidation(false, 1));
    }

    #[test]
    fn target_invalidation_stops_new_events_but_releases_and_next_action_survive() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let gate = InputGate::default();
        let downs = AtomicUsize::new(0);
        let ups = AtomicUsize::new(0);
        let action = gate.lease().unwrap();
        let held = gate
            .press(
                || {
                    downs.fetch_add(1, Ordering::SeqCst);
                },
                || {
                    ups.fetch_add(1, Ordering::SeqCst);
                },
            )
            .unwrap();
        // The observer sets exactly this flag on an admitted target action.
        gate.lock().target_invalidated = true;
        assert!(gate
            .post(|| {
                downs.fetch_add(1, Ordering::SeqCst);
            })
            .is_err());
        assert!(gate.pause(Duration::ZERO).is_err());
        assert_eq!(ups.load(Ordering::SeqCst), 0);
        drop(held);
        assert_eq!(ups.load(Ordering::SeqCst), 1);
        assert_eq!(downs.load(Ordering::SeqCst), 1);
        drop(action);
        let next = gate.lease().unwrap();
        assert!(gate.post(|| ()).is_ok());
        drop(next);
        assert_eq!(gate.cancel_and_wait(Duration::ZERO), 0);
        assert!(gate.lease().is_err());
    }

    #[test]
    fn cancel_wakes_a_held_input_and_waits_for_its_release() {
        let gate = Arc::new(InputGate::default());
        let worker_gate = Arc::clone(&gate);
        let (events, received) = mpsc::channel();
        let worker = std::thread::spawn(move || {
            let _held = worker_gate
                .press(
                    || events.send("down").unwrap(),
                    || events.send("up").unwrap(),
                )
                .unwrap();
            assert!(worker_gate.pause(Duration::from_secs(60)).is_err());
        });
        assert_eq!(
            received.recv_timeout(Duration::from_secs(1)).unwrap(),
            "down"
        );
        assert_eq!(gate.cancel_and_wait(Duration::from_secs(1)), 0);
        assert_eq!(received.try_recv().unwrap(), "up");
        worker.join().unwrap();
        assert!(gate.post(|| panic!("cancelled event posted")).is_err());
        assert!(gate
            .press(
                || panic!("cancelled down posted"),
                || panic!("unowned release")
            )
            .is_err());
    }

    #[test]
    fn incomplete_release_is_reported_until_the_owner_drains() {
        let gate = InputGate::default();
        let mut released = false;
        let held = gate.press(|| {}, || released = true).unwrap();
        assert_eq!(gate.cancel_and_wait(Duration::ZERO), 1);
        drop(held);
        assert!(released);
        assert_eq!(gate.cancel_and_wait(Duration::ZERO), 0);
        assert!(gate.ensure_active().is_err());
    }

    #[test]
    fn unwind_submits_the_release_before_cleanup_can_succeed() {
        let gate = InputGate::default();
        let mut released = false;
        let failed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _held = gate.press(|| {}, || released = true).unwrap();
            panic!("action failed after down");
        }));
        assert!(failed.is_err());
        assert!(released);
        assert_eq!(gate.cancel_and_wait(Duration::ZERO), 0);
    }

    #[test]
    fn cleanup_waits_for_the_action_to_restore_its_native_context() {
        let gate = InputGate::default();
        let action = gate.lease().unwrap();
        let held = gate.press(|| {}, || {}).unwrap();
        drop(held);
        assert_eq!(gate.cancel_and_wait(Duration::ZERO), 1);
        assert!(gate.lease().is_err());
        drop(action);
        assert_eq!(gate.cancel_and_wait(Duration::ZERO), 0);
    }

    #[test]
    fn concurrent_target_admission_rejects_every_exclusive_operation() {
        assert!(concurrent_target_counts_admissible(0, false, 0, false));
        assert!(concurrent_target_counts_admissible(3, false, 3, false));
        assert!(!concurrent_target_counts_admissible(1, false, 0, false));
        assert!(!concurrent_target_counts_admissible(3, true, 3, false));
        assert!(!concurrent_target_counts_admissible(3, false, 3, true));
    }
}
