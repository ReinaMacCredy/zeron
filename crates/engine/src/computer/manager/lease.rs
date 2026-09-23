use std::future::Future;

use chrono::{SecondsFormat, TimeZone, Utc};
use futures::FutureExt;
use zeron_proto::computer::{ComputerDeliveryMode, ComputerLaunchAppResult};

use super::{
    BackgroundLease, BackgroundTarget, ComputerLeaseLifecycle, ComputerLeaseRecord,
    ComputerManager, DesktopLease, State, agent_thread_id, error_message, window_id_set,
};
use crate::computer::backend::{
    ComputerBackendError, ComputerBackendMethod, ComputerError, ComputerLeaseError,
};
use crate::computer::desktop_operation_queue::{
    assert_desktop_operation_active, desktop_delivery_mode,
};
use crate::computer::task_context::current_computer_task;

pub(super) type BeforeClaim<'a> = Option<&'a (dyn Fn() -> Result<(), ComputerError> + Send + Sync)>;

impl ComputerManager {
    pub(super) fn is_lease_stale(&self, state: &State, lease: &DesktopLease, now: u64) -> bool {
        if now.saturating_sub(lease.last_activity_ms) < self.lease_idle_ms {
            return false;
        }
        state
            .agent_calls_in_flight
            .get(&lease.thread_id)
            .copied()
            .unwrap_or(0)
            == 0
    }

    pub(super) fn can_use_background_target(&self) -> bool {
        self.backend.exact_target_background_input()
            && desktop_delivery_mode() != ComputerDeliveryMode::Foreground
    }

    pub(super) async fn with_desktop_control<A, F, Fut>(
        &self,
        thread_id: Option<&str>,
        action: F,
        before_claim: BeforeClaim<'_>,
    ) -> Result<A, ComputerError>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<A, ComputerError>>,
    {
        assert_desktop_operation_active()?;
        let owner = agent_thread_id(thread_id);
        let now = self.now_ms();
        {
            let mut state = self.state();
            if owner.is_none() {
                state.last_user_desktop_input_at = Some(now);
            }
            if let (Some(owner), Some(lease)) = (owner, state.lease.as_ref())
                && lease.thread_id != owner
                && !self.is_lease_stale(&state, lease, now)
            {
                return Err(ComputerLeaseError { target_only: false }.into());
            }
        }
        Box::pin(self.operations.run(
            || async move {
                self.assert_control_authority(owner)?;
                self.assert_input_not_paused(owner)?;
                if let Some(before_claim) = before_claim {
                    before_claim()?;
                }
                self.claim_desktop_control(thread_id).await?;
                assert_desktop_operation_active()?;
                let result = self.with_computer_call(action()).await;
                if let Err(error) = &result {
                    self.record_input_pause(owner, error);
                }
                result
            },
            None,
        ))
        .await
    }

    pub(super) async fn with_semantic_control<A, F, Fut>(
        &self,
        thread_id: Option<&str>,
        window_id: Option<&str>,
        action: F,
    ) -> Result<A, ComputerError>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<A, ComputerError>>,
    {
        match window_id {
            Some(window_id) if self.can_use_background_target() => {
                self.with_background_window_control(thread_id, window_id, action)
                    .await
            }
            _ => self.with_desktop_control(thread_id, action, None).await,
        }
    }

    pub(super) async fn with_background_process_control<A, F, Fut>(
        &self,
        thread_id: Option<&str>,
        window_id: Option<&str>,
        action: F,
    ) -> Result<A, ComputerError>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<A, ComputerError>>,
    {
        let Some(window_id) = window_id.filter(|_| self.can_use_background_target()) else {
            return self.with_desktop_control(thread_id, action, None).await;
        };
        self.with_background_resource_control(
            thread_id,
            || async move {
                let window = self.resolve_window_target(thread_id, window_id).await?;
                match window.pid.filter(|pid| *pid > 0) {
                    Some(pid) => Ok(BackgroundTarget {
                        key: format!("process:{pid}"),
                        pid: Some(pid),
                        window_id: None,
                    }),
                    None => Err(ComputerBackendError::new(
                        "Background input needs a verified application process for the exact window.",
                    )
                    .into()),
                }
            },
            |_| action(),
        )
        .await
    }

    pub(super) async fn with_background_app_control<F, Fut>(
        &self,
        thread_id: Option<&str>,
        app: &str,
        action: F,
    ) -> Result<ComputerLaunchAppResult, ComputerError>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<ComputerLaunchAppResult, ComputerError>>,
    {
        if !self.can_use_background_target() {
            return self.with_desktop_control(thread_id, action, None).await;
        }
        self.with_background_resource_control(
            thread_id,
            || async move {
                self.assert_driven_app_allowed(app)?;
                let apps = if self.backend.supports(ComputerBackendMethod::ListApps) {
                    self.backend.list_apps().await?
                } else {
                    Vec::new()
                };
                let spelling = app.trim().to_lowercase();
                let matches: Vec<_> = apps
                    .iter()
                    .filter(|candidate| {
                        [
                            Some(candidate.name.as_str()),
                            candidate.bundle_id.as_deref(),
                            candidate.launch_path.as_deref(),
                        ]
                        .into_iter()
                        .flatten()
                        .any(|name| name.to_lowercase() == spelling)
                    })
                    .collect();
                let running: Vec<_> = matches
                    .iter()
                    .filter(|candidate| candidate.running && candidate.pid > 0)
                    .collect();
                if running.len() > 1 {
                    return Err(ComputerBackendError::new(
                        "Several running applications match this launch. Use an exact existing window instead.",
                    )
                    .into());
                }
                let target = running.first().copied().or(matches.first());
                Ok(match target {
                    Some(target) if target.running => BackgroundTarget {
                        key: format!("process:{}", target.pid),
                        pid: Some(target.pid),
                        window_id: None,
                    },
                    _ => {
                        let name = target
                            .and_then(|target| {
                                target
                                    .bundle_id
                                    .clone()
                                    .or_else(|| target.launch_path.clone())
                                    .or_else(|| Some(target.name.clone()))
                            })
                            .unwrap_or_else(|| spelling.clone());
                        BackgroundTarget {
                            key: format!("application:{}", name.to_lowercase()),
                            pid: None,
                            window_id: None,
                        }
                    }
                })
            },
            |target| async move {
                let result = action().await?;
                let pid = result
                    .pid
                    .or_else(|| result.window.as_ref().and_then(|window| window.pid));
                let owner = agent_thread_id(thread_id);
                if let Some(pid) = pid.filter(|pid| *pid > 0) {
                    let mut state = self.state();
                    if let Some((_, held)) = state
                        .background_leases
                        .iter_mut()
                        .find(|(key, _)| *key == target.key)
                        && Some(held.lease.thread_id.as_str()) == owner
                    {
                        held.target = BackgroundTarget {
                            pid: Some(pid),
                            ..target.clone()
                        };
                    }
                }
                Ok(result)
            },
        )
        .await
    }

    async fn with_background_resource_control<A, R, RFut, F, Fut>(
        &self,
        thread_id: Option<&str>,
        resolve: R,
        action: F,
    ) -> Result<A, ComputerError>
    where
        R: FnOnce() -> RFut,
        RFut: Future<Output = Result<BackgroundTarget, ComputerError>>,
        F: FnOnce(BackgroundTarget) -> Fut,
        Fut: Future<Output = Result<A, ComputerError>>,
    {
        assert_desktop_operation_active()?;
        let owner = agent_thread_id(thread_id);
        if owner.is_none() {
            self.state().last_user_desktop_input_at = Some(self.now_ms());
        }
        Box::pin(self.operations.run(
            || async move {
                self.assert_control_authority(owner)?;
                self.assert_input_not_paused(owner)?;
                let target = resolve().await?;
                assert_desktop_operation_active()?;
                self.claim_background_control(owner, &target)?;
                self.engage_backend();
                let result = self.with_computer_call(action(target)).await;
                if let Err(error) = &result {
                    self.record_input_pause(owner, error);
                }
                result
            },
            None,
        ))
        .await
    }

    pub(super) async fn with_background_window_control<A, F, Fut>(
        &self,
        thread_id: Option<&str>,
        window_id: &str,
        action: F,
    ) -> Result<A, ComputerError>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<A, ComputerError>>,
    {
        if desktop_delivery_mode() == ComputerDeliveryMode::Foreground {
            return self.with_desktop_control(thread_id, action, None).await;
        }
        assert_desktop_operation_active()?;
        let owner = agent_thread_id(thread_id);
        if owner.is_none() {
            self.state().last_user_desktop_input_at = Some(self.now_ms());
        }
        Box::pin(self.operations.run_scoped(
            window_id,
            || async move {
                self.assert_control_authority(owner)?;
                self.assert_input_not_paused(owner)?;
                let target = self.resolve_window_target(thread_id, window_id).await?;
                assert_desktop_operation_active()?;
                self.claim_background_control(
                    owner,
                    &BackgroundTarget {
                        key: format!("window:{window_id}"),
                        pid: target.pid,
                        window_id: Some(window_id.to_owned()),
                    },
                )?;
                self.engage_backend();
                let result = self.with_computer_call(action()).await;
                if let Err(error) = &result {
                    self.record_input_pause(owner, error);
                }
                result
            },
            None,
        ))
        .await
    }

    fn claim_background_control(
        &self,
        owner: Option<&str>,
        target: &BackgroundTarget,
    ) -> Result<(), ComputerError> {
        let Some(owner) = owner else {
            return Ok(());
        };
        let now = self.now_ms();
        let mut evicted = Vec::new();
        let outcome: Result<bool, ComputerError> = {
            let mut state = self.state();
            (|| {
                if let Some(lease) = state.lease.as_ref() {
                    let stale = self.is_lease_stale(&state, lease, now);
                    if lease.thread_id != owner && !stale {
                        return Err(ComputerLeaseError { target_only: false }.into());
                    }
                    if stale {
                        let thread = lease.thread_id.clone();
                        state.authority_turns.remove(&thread);
                        state.lease = None;
                    }
                }
                let mut index = 0;
                while index < state.background_leases.len() {
                    let (key, lease) = &state.background_leases[index];
                    if self.is_lease_stale(&state, &lease.lease, now) {
                        let removed = state.background_leases.remove(index);
                        evicted.push(removed.1.lease.thread_id);
                        continue;
                    }
                    let same_process = target.pid.is_some() && target.pid == lease.target.pid;
                    let conflict = target.key == *key
                        || (same_process
                            && (target.window_id.is_none() || lease.target.window_id.is_none()));
                    if conflict && lease.lease.thread_id != owner {
                        return Err(ComputerLeaseError { target_only: true }.into());
                    }
                    index += 1;
                }
                let claiming = current_computer_task();
                let turn_id = claiming
                    .filter(|task| task.thread_id == owner)
                    .and_then(|task| task.turn_id)
                    .or_else(|| state.authority_turns.get(owner).cloned());
                let held = state
                    .background_leases
                    .iter()
                    .position(|(key, _)| *key == target.key);
                let carry = held.and_then(|index| {
                    let held = &state.background_leases[index].1.lease;
                    (held.thread_id == owner && held.turn_id == turn_id && held.release_requested)
                        .then(|| held.release_requested_turn_id.clone())
                });
                let lease = BackgroundLease {
                    lease: DesktopLease {
                        thread_id: owner.to_owned(),
                        turn_id,
                        last_activity_ms: now,
                        release_requested: carry.is_some(),
                        release_requested_turn_id: carry.flatten(),
                    },
                    target: target.clone(),
                };
                let changed_owner = match held {
                    Some(index) => {
                        let changed = state.background_leases[index].1.lease.thread_id != owner;
                        state.background_leases[index].1 = lease;
                        changed
                    }
                    None => {
                        state.background_leases.push((target.key.clone(), lease));
                        true
                    }
                };
                Ok(changed_owner)
            })()
        };
        for owner in evicted {
            self.clear_evicted_background_owner(&owner);
        }
        self.sync_still_stream();
        if outcome? {
            self.publish_ownership_cached();
        }
        Ok(())
    }

    fn clear_evicted_background_owner(&self, owner: &str) {
        {
            let mut state = self.state();
            if state
                .lease
                .as_ref()
                .is_some_and(|lease| lease.thread_id == owner)
                || state.owns_background_lease(owner)
            {
                return;
            }
            state.authority_turns.remove(owner);
            if let Some(runtime) = state.thread_mut(owner) {
                runtime.pane_surfaced = false;
            }
        }
        self.publish_ownership_cached();
    }

    pub(super) fn release_background_control(
        &self,
        owner: &str,
        turn_id: Option<&str>,
        only_requested: bool,
    ) {
        let changed = {
            let mut state = self.state();
            let in_flight = state.agent_calls_in_flight.get(owner).copied().unwrap_or(0) > 0;
            let mut changed = false;
            let mut index = 0;
            while index < state.background_leases.len() {
                let lease = &mut state.background_leases[index].1.lease;
                let skip = lease.thread_id != owner
                    || matches!((turn_id, &lease.turn_id), (Some(turn), Some(held)) if held != turn)
                    || (only_requested
                        && (!lease.release_requested
                            || lease.release_requested_turn_id != lease.turn_id));
                if skip {
                    index += 1;
                    continue;
                }
                if in_flight {
                    lease.release_requested = true;
                    lease.release_requested_turn_id =
                        turn_id.map(str::to_owned).or_else(|| lease.turn_id.clone());
                    index += 1;
                } else {
                    state.background_leases.remove(index);
                    changed = true;
                }
            }
            let lease_owner = state
                .lease
                .as_ref()
                .is_some_and(|lease| lease.thread_id == owner);
            if !lease_owner
                && !state.owns_background_lease(owner)
                && let Some(runtime) = state.thread_mut(owner)
            {
                runtime.pane_surfaced = false;
            }
            changed
        };
        self.sync_still_stream();
        if changed {
            self.publish_ownership_cached();
        }
    }

    pub(super) async fn claim_desktop_control(
        &self,
        thread_id: Option<&str>,
    ) -> Result<(), ComputerError> {
        self.engage_backend();
        let Some(owner) = agent_thread_id(thread_id) else {
            return Ok(());
        };
        let known = self.state().last_known_window_ids.clone();
        let baseline = match known {
            Some(known) => Some(known),
            None => self
                .read_windows()
                .await
                .ok()
                .map(|windows| window_id_set(&windows)),
        };
        self.state().pre_action_window_ids = baseline;
        let now = self.now_ms();
        let mut evicted = Vec::new();
        let claimed = {
            let mut state = self.state();
            (|| {
                let mut index = 0;
                while index < state.background_leases.len() {
                    let lease = &state.background_leases[index].1.lease;
                    if self.is_lease_stale(&state, lease, now) {
                        let removed = state.background_leases.remove(index);
                        evicted.push(removed.1.lease.thread_id);
                        continue;
                    }
                    if lease.thread_id != owner {
                        return Err(ComputerError::from(ComputerLeaseError {
                            target_only: false,
                        }));
                    }
                    index += 1;
                }
                let held = state.lease.clone();
                let held_stale = held
                    .as_ref()
                    .is_some_and(|held| self.is_lease_stale(&state, held, now));
                if let Some(held) = &held
                    && held.thread_id != owner
                    && !held_stale
                {
                    return Err(ComputerLeaseError { target_only: false }.into());
                }
                if held_stale && let Some(held) = &held {
                    state.authority_turns.remove(&held.thread_id);
                    if let Some(runtime) = state.thread_mut(&held.thread_id) {
                        runtime.pane_surfaced = false;
                    }
                }
                let changed = held.as_ref().is_none_or(|held| held.thread_id != owner);
                Ok((held, held_stale, changed))
            })()
        };
        for evicted in evicted {
            self.clear_evicted_background_owner(&evicted);
        }
        self.sync_still_stream();
        let (held, held_stale, changed) = claimed?;
        assert_desktop_operation_active()?;
        if changed {
            self.backend.clear_focus_window().await?;
            assert_desktop_operation_active()?;
        }
        let lease = {
            let mut state = self.state();
            let stamped = current_computer_task()
                .filter(|task| agent_thread_id(Some(&task.thread_id)) == Some(owner))
                .and_then(|task| task.turn_id)
                .or_else(|| state.authority_turns.get(owner).cloned());
            let carry = held
                .as_ref()
                .filter(|held| !changed && held.release_requested);
            let lease = DesktopLease {
                thread_id: owner.to_owned(),
                turn_id: stamped,
                last_activity_ms: now,
                release_requested: carry.is_some(),
                release_requested_turn_id: carry
                    .and_then(|held| held.release_requested_turn_id.clone()),
            };
            state.lease = Some(lease.clone());
            lease
        };
        self.sync_still_stream();
        if held_stale && let Some(held) = &held {
            self.record_lease_lifecycle(
                ComputerLeaseLifecycle::StaleReclaimed,
                held,
                Some((now.saturating_sub(held.last_activity_ms), owner)),
            );
        }
        if changed || held_stale || held.as_ref().map(|held| &held.turn_id) != Some(&lease.turn_id)
        {
            self.record_lease_lifecycle(ComputerLeaseLifecycle::Acquired, &lease, None);
        }
        if changed {
            self.announce_driving_agent(Some(owner)).await;
            self.publish_all_threads().await;
        }
        Ok(())
    }

    fn record_lease_lifecycle(
        &self,
        event: ComputerLeaseLifecycle,
        lease: &DesktopLease,
        detail: Option<(u64, &str)>,
    ) {
        let now = self.now_ms();
        let ts = Utc
            .timestamp_millis_opt(now as i64)
            .single()
            .map(|ts| ts.to_rfc3339_opts(SecondsFormat::Millis, true))
            .unwrap_or_default();
        self.seams.lease_lifecycle(&ComputerLeaseRecord {
            ts,
            event,
            thread_id: lease.thread_id.clone(),
            turn_id: lease.turn_id.clone(),
            idle_ms: detail.map(|(idle, _)| idle),
            next_thread_id: detail.map(|(_, next)| next.to_owned()),
        });
    }

    async fn announce_driving_agent(&self, thread_id: Option<&str>) {
        self.cursor_activity.set_owner(thread_id);
        let label =
            thread_id.and_then(|thread| self.state().thread_label(thread).map(str::to_owned));
        let _ = self.backend.set_driving_agent(label.as_deref()).await;
    }

    pub fn set_thread_label(&self, thread_id: &str, label: Option<&str>) {
        let Some(owner) = agent_thread_id(Some(thread_id)) else {
            return;
        };
        let announce = {
            let mut state = self.state();
            match label.map(str::trim).filter(|label| !label.is_empty()) {
                Some(trimmed) => {
                    if state.thread_label(owner) == Some(trimmed) {
                        return;
                    }
                    state.thread_labels.retain(|(id, _)| id != owner);
                    state
                        .thread_labels
                        .push((owner.to_owned(), trimmed.to_owned()));
                    let mut index = 0;
                    while state.thread_labels.len() > 256 && index < state.thread_labels.len() {
                        let id = &state.thread_labels[index].0;
                        if id == owner
                            || state
                                .lease
                                .as_ref()
                                .is_some_and(|lease| &lease.thread_id == id)
                            || state.agent_calls_in_flight.contains_key(id)
                        {
                            index += 1;
                            continue;
                        }
                        state.thread_labels.remove(index);
                    }
                }
                None => {
                    let before = state.thread_labels.len();
                    state.thread_labels.retain(|(id, _)| id != owner);
                    if state.thread_labels.len() == before {
                        return;
                    }
                }
            }
            state
                .lease
                .as_ref()
                .is_some_and(|lease| lease.thread_id == owner)
        };
        if announce && let Some(manager) = self.strong() {
            let owner = owner.to_owned();
            let mut announcing = async move {
                manager.announce_driving_agent(Some(&owner)).await;
            }
            .boxed();
            let waker = futures::task::noop_waker();
            let mut context = std::task::Context::from_waker(&waker);
            if announcing.as_mut().poll(&mut context).is_pending() {
                tokio::spawn(announcing);
            }
        }
    }

    pub async fn release_desktop_control(
        &self,
        thread_id: &str,
        turn_id: Option<&str>,
    ) -> Result<(), ComputerError> {
        let Some(owner) = agent_thread_id(Some(thread_id)) else {
            return Ok(());
        };
        self.space_broker.release(owner, turn_id);
        self.release_background_control(owner, turn_id, false);
        let cleanup_turn = {
            let state = self.state();
            turn_id.map(str::to_owned).or_else(|| {
                let authorized = !state.revoked_threads.contains(owner)
                    && !state.suspended_threads.contains(owner);
                match state.lease.as_ref() {
                    Some(lease) if authorized && lease.thread_id == owner => lease.turn_id.clone(),
                    _ => state.authority_turns.get(owner).cloned(),
                }
            })
        };
        let backend = self.backend.clone();
        let manager = self.weak.clone();
        let cleanup_owner = owner.to_owned();
        tokio::spawn(async move {
            let Err(error) = backend
                .end_task(&cleanup_owner, cleanup_turn.as_deref())
                .await
            else {
                return;
            };
            if let Some(manager) = manager.upgrade() {
                manager
                    .record_thread_error(
                        &cleanup_owner,
                        &format!("Preview cleanup failed: {}", error_message(&error)),
                    )
                    .await;
            }
        });
        let released_turn = {
            let mut state = self.state();
            let in_flight = state.agent_calls_in_flight.get(owner).copied().unwrap_or(0);
            let owns_lease = state
                .lease
                .as_ref()
                .is_some_and(|lease| lease.thread_id == owner);
            if !owns_lease {
                if in_flight == 0
                    && !state.owns_background_lease(owner)
                    && (turn_id.is_none()
                        || state.authority_turns.get(owner).map(String::as_str) == turn_id)
                {
                    state.authority_turns.remove(owner);
                }
                drop(state);
                self.publish_cached(owner);
                return Ok(());
            }
            let lease = state.lease.clone().expect("owned above");
            if let (Some(turn), Some(held)) = (turn_id, &lease.turn_id)
                && held != turn
            {
                return Ok(());
            }
            if in_flight > 0 {
                let first = !lease.release_requested;
                let held = state.lease.as_mut().expect("owned above");
                held.release_requested = true;
                held.release_requested_turn_id =
                    turn_id.map(str::to_owned).or_else(|| lease.turn_id.clone());
                drop(state);
                if first {
                    self.record_lease_lifecycle(
                        ComputerLeaseLifecycle::ReleaseRequested,
                        &lease,
                        None,
                    );
                }
                return Ok(());
            }
            lease.turn_id
        };
        Box::pin(self.operations.run(
            || async move {
                {
                    let state = self.state();
                    match state.lease.as_ref() {
                        Some(lease)
                            if lease.thread_id == owner && lease.turn_id == released_turn => {}
                        _ => return Ok(()),
                    }
                }
                self.backend.clear_focus_window().await?;
                let lease = {
                    let mut state = self.state();
                    let lease = state.lease.take();
                    state.authority_turns.remove(owner);
                    if let Some(runtime) = state.thread_mut(owner) {
                        runtime.pane_surfaced = false;
                    }
                    lease
                };
                self.sync_still_stream();
                if let Some(lease) = &lease {
                    self.record_lease_lifecycle(ComputerLeaseLifecycle::Released, lease, None);
                }
                self.announce_driving_agent(None).await;
                Ok::<(), ComputerError>(())
            },
            None,
        ))
        .await?;
        self.publish_all_threads().await;
        Ok(())
    }
}
