use std::sync::Arc;
use std::time::Duration;

use futures::FutureExt;
use futures::future::{BoxFuture, Shared};
use zeron_proto::computer::{
    ComputerActionResult, ComputerAvailability, ComputerEvent, ComputerScreenSize,
    ThreadComputerState,
};

use super::{
    COMPUTER_ERROR_REPUBLISH_DEBOUNCE_MS, ComputerManager, PhysicalState, State,
    THREAD_RECORD_LIMIT, ThreadRuntime, agent_thread_id, corrected_availability, error_message,
};
use crate::computer::backend::clamp_computer_message;

impl State {
    fn bump_version(&mut self) -> u64 {
        let version = self.next_state_version;
        self.next_state_version += 1;
        version
    }

    pub(super) fn new_thread_runtime(&mut self) -> ThreadRuntime {
        let record = self.next_record;
        self.next_record += 1;
        ThreadRuntime {
            record,
            version: self.bump_version(),
            last_error: None,
            reported_error: None,
            input_pause: None,
            windows: Vec::new(),
            screen_size: ComputerScreenSize {
                width: 1,
                height: 1,
                scale: None,
            },
            availability: ComputerAvailability::BackendUnavailable {
                message: "Computer state has not been queried yet".into(),
            },
            cursor: None,
            pane_surfaced: false,
        }
    }

    pub(super) fn thread(&self, thread_id: &str) -> Option<&ThreadRuntime> {
        self.threads
            .iter()
            .find(|(id, _)| id == thread_id)
            .map(|(_, runtime)| runtime)
    }

    pub(super) fn thread_mut(&mut self, thread_id: &str) -> Option<&mut ThreadRuntime> {
        self.threads
            .iter_mut()
            .find(|(id, _)| id == thread_id)
            .map(|(_, runtime)| runtime)
    }

    pub(super) fn remove_thread(&mut self, thread_id: &str) {
        self.threads.retain(|(id, _)| id != thread_id);
    }

    pub(super) fn thread_runtime(&mut self, thread_id: &str) -> &mut ThreadRuntime {
        match self.threads.iter().position(|(id, _)| id == thread_id) {
            Some(index) => {
                let entry = self.threads.remove(index);
                self.threads.push(entry);
            }
            None => {
                let runtime = self.new_thread_runtime();
                self.threads.push((thread_id.to_owned(), runtime));
            }
        }
        let mut index = 0;
        while self.threads.len() > THREAD_RECORD_LIMIT && index < self.threads.len() {
            let (id, candidate) = &self.threads[index];
            let keep = id == thread_id
                || self
                    .lease
                    .as_ref()
                    .is_some_and(|lease| &lease.thread_id == id)
                || self.agent_calls_in_flight.contains_key(id)
                || self.publish_locks.contains_key(id)
                || candidate.pane_surfaced
                || candidate.input_pause.is_some();
            if keep {
                index += 1;
                continue;
            }
            let id = id.clone();
            self.threads.remove(index);
            self.thread_labels.retain(|(label_id, _)| *label_id != id);
        }
        self.thread_mut(thread_id).expect("inserted above")
    }

    pub(super) fn owns_background_lease(&self, owner: &str) -> bool {
        self.background_leases
            .iter()
            .any(|(_, lease)| lease.lease.thread_id == owner)
    }

    pub(super) fn thread_label(&self, thread_id: &str) -> Option<&str> {
        self.thread_labels
            .iter()
            .find(|(id, _)| id == thread_id)
            .map(|(_, label)| label.as_str())
    }
}

impl ComputerManager {
    pub(super) fn thread_snapshot(
        &self,
        state: &State,
        thread_id: &str,
        runtime: &ThreadRuntime,
    ) -> ThreadComputerState {
        let background_owners: std::collections::HashSet<&str> = state
            .background_leases
            .iter()
            .map(|(_, lease)| lease.lease.thread_id.as_str())
            .collect();
        let control_owner = state
            .lease
            .as_ref()
            .map(|lease| lease.thread_id.as_str())
            .or_else(|| background_owners.contains(thread_id).then_some(thread_id));
        let lease_owner = state.lease.as_ref().map(|lease| lease.thread_id.as_str());
        ThreadComputerState {
            control_generation: Some(self.control_state.generation(thread_id)),
            thread_id: thread_id.to_owned(),
            version: runtime.version,
            computer_id: self.computer_id.clone(),
            windows: runtime.windows.clone(),
            screen_size: runtime.screen_size,
            cursor: runtime.cursor,
            agent_active: state
                .agent_calls_in_flight
                .get(thread_id)
                .is_some_and(|count| *count > 0),
            activity: state
                .activity
                .clone()
                .filter(|_| lease_owner == Some(thread_id)),
            input_pause: runtime.input_pause.clone(),
            control_owner_thread_id: control_owner.map(str::to_owned),
            control_owner_label: control_owner.map(|owner| {
                let label = state.thread_label(owner).unwrap_or("Agent");
                crate::computer::backend::js_slice(label, 512).to_owned()
            }),
            controlled_by_other_thread: lease_owner.is_some_and(|owner| owner != thread_id),
            shared_preview_unavailable: (background_owners.len() > 1).then_some(true),
            availability: corrected_availability(
                state.backend_engaged,
                &state.backend_health,
                runtime.availability.clone(),
            ),
            health: state.backend_health.clone(),
            capabilities: self.backend.capabilities(),
            input_stopped: None,
            last_error: runtime
                .reported_error
                .clone()
                .or_else(|| runtime.last_error.clone()),
        }
    }

    pub(super) fn publish_cached(&self, thread_id: &str) -> Option<ThreadComputerState> {
        let snapshot = {
            let mut state = self.state();
            if state.disposed || state.thread(thread_id).is_none() {
                return None;
            }
            let version = state.bump_version();
            let runtime = state.thread_mut(thread_id).expect("checked above");
            runtime.version = version;
            let runtime = state.thread(thread_id).expect("checked above");
            let snapshot = self.thread_snapshot(&state, thread_id, runtime);
            state
                .thread_mut(thread_id)
                .expect("checked above")
                .reported_error = None;
            snapshot
        };
        self.emit(ComputerEvent::ThreadState {
            state: Box::new(snapshot.clone()),
        });
        Some(snapshot)
    }

    pub(super) fn publish_ownership_cached(&self) {
        let threads: Vec<String> = self
            .state()
            .threads
            .iter()
            .map(|(id, _)| id.clone())
            .collect();
        for thread_id in threads {
            self.publish_cached(&thread_id);
        }
    }

    pub(super) async fn publish(&self, thread_id: &str) -> Option<ThreadComputerState> {
        let gate = {
            let mut state = self.state();
            let entry = state
                .publish_locks
                .entry(thread_id.to_owned())
                .or_insert_with(|| (Arc::new(tokio::sync::Mutex::new(())), 0));
            entry.1 += 1;
            entry.0.clone()
        };
        let snapshot = {
            let _turn = gate.lock().await;
            Box::pin(self.publish_now(thread_id)).await
        };
        let mut state = self.state();
        if let Some(entry) = state.publish_locks.get_mut(thread_id) {
            entry.1 -= 1;
            if entry.1 == 0 {
                state.publish_locks.remove(thread_id);
            }
        }
        snapshot
    }

    async fn publish_now(&self, thread_id: &str) -> Option<ThreadComputerState> {
        let record = self.state().thread(thread_id)?.record;
        let needs_read = {
            let state = self.state();
            state.physical_state.is_none() && state.physical_failure.is_none()
        };
        if needs_read {
            self.refresh_physical_state().await;
        }
        let snapshot = {
            let mut state = self.state();
            let failure = state.physical_failure.clone();
            let physical = state.physical_state.as_ref().map(|physical| {
                (
                    physical.availability.clone(),
                    physical.windows.clone(),
                    physical.screen_size,
                )
            });
            if state.disposed || state.thread(thread_id).is_none_or(|t| t.record != record) {
                return None;
            }
            let version = state.bump_version();
            let runtime = state.thread_mut(thread_id).expect("checked above");
            match failure {
                Some(failure) => {
                    runtime.last_error = Some(clamp_computer_message(
                        &failure,
                        "The computer backend reported an error without a message.",
                    ));
                }
                None => {
                    if let Some((availability, windows, screen_size)) = physical {
                        runtime.availability = availability;
                        if let Some(windows) = windows {
                            runtime.windows = windows;
                        }
                        if let Some(screen_size) = screen_size {
                            runtime.screen_size = screen_size;
                        }
                    }
                    runtime.last_error = None;
                }
            }
            runtime.version = version;
            let runtime = state.thread(thread_id).expect("checked above");
            let snapshot = self.thread_snapshot(&state, thread_id, runtime);
            state
                .thread_mut(thread_id)
                .expect("checked above")
                .reported_error = None;
            snapshot
        };
        self.emit(ComputerEvent::ThreadState {
            state: Box::new(snapshot.clone()),
        });
        Some(snapshot)
    }

    pub(super) fn refresh_physical_state(&self) -> Shared<BoxFuture<'static, ()>> {
        let mut state = self.state();
        if let Some(read) = &state.physical_read {
            return read.clone();
        }
        let manager = self.weak.clone();
        let read = async move {
            let Some(manager) = manager.upgrade() else {
                return;
            };
            let engaged = {
                let mut state = manager.state();
                state.physical_failure = None;
                state.backend_engaged
            };
            let outcome = if engaged {
                let (availability, windows, screen_size) = futures::join!(
                    manager.backend.availability(false),
                    manager.read_windows(),
                    manager.backend.get_screen_size()
                );
                match (availability, windows, screen_size) {
                    (Ok(availability), Ok(windows), Ok(screen_size)) => Ok(PhysicalState {
                        availability,
                        windows: Some(windows),
                        screen_size: Some(screen_size),
                    }),
                    (Err(error), _, _) | (_, Err(error), _) | (_, _, Err(error)) => Err(error),
                }
            } else {
                manager
                    .backend
                    .probe_availability()
                    .await
                    .map(|availability| PhysicalState {
                        availability,
                        windows: None,
                        screen_size: None,
                    })
            };
            let mut state = manager.state();
            match outcome {
                Ok(physical) => state.physical_state = Some(physical),
                Err(error) => {
                    state.physical_failure = Some(clamp_computer_message(
                        &error_message(&error),
                        "Computer state is unavailable.",
                    ));
                }
            }
            state.physical_read = None;
        }
        .boxed()
        .shared();
        state.physical_read = Some(read.clone());
        read
    }

    pub(super) async fn publish_all_threads(&self) {
        self.refresh_physical_state().await;
        let threads: Vec<String> = {
            let mut state = self.state();
            state.publish_all_depth += 1;
            state.threads.iter().map(|(id, _)| id.clone()).collect()
        };
        for thread_id in threads {
            self.publish(&thread_id).await;
        }
        let reschedule = {
            let mut state = self.state();
            state.publish_all_depth -= 1;
            let reschedule = state.publish_all_depth == 0 && state.windows_publish_pending;
            if reschedule {
                state.windows_publish_pending = false;
            }
            reschedule
        };
        if reschedule {
            self.schedule_windows_publish();
        }
    }

    pub(super) fn schedule_windows_publish(&self) {
        let mut state = self.state();
        if state.disposed {
            return;
        }
        if state.publish_all_depth > 0 {
            state.windows_publish_pending = true;
            return;
        }
        if state.windows_publish_timer.is_some() {
            return;
        }
        let manager = self.weak.clone();
        let debounce = Duration::from_millis(self.windows_publish_debounce_ms);
        state.windows_publish_timer = Some(tokio::spawn(async move {
            tokio::time::sleep(debounce).await;
            let Some(manager) = manager.upgrade() else {
                return;
            };
            {
                let mut state = manager.state();
                state.windows_publish_timer = None;
                if state.disposed {
                    return;
                }
            }
            manager.publish_all_threads().await;
        }));
    }

    pub(super) fn republish_all_threads(&self) {
        let snapshots = {
            let mut state = self.state();
            if state.disposed {
                return;
            }
            let ids: Vec<String> = state.threads.iter().map(|(id, _)| id.clone()).collect();
            let mut snapshots = Vec::with_capacity(ids.len());
            for thread_id in ids {
                let version = state.bump_version();
                state.thread_mut(&thread_id).expect("listed").version = version;
                let runtime = state.thread(&thread_id).expect("listed");
                snapshots.push(self.thread_snapshot(&state, &thread_id, runtime));
            }
            snapshots
        };
        for snapshot in snapshots {
            self.emit(ComputerEvent::ThreadState {
                state: Box::new(snapshot),
            });
        }
    }

    pub(super) fn record_error(&self, message: &str) {
        let message = clamp_computer_message(
            message,
            "The computer backend reported an error without a message.",
        );
        let mut state = self.state();
        for (_, thread) in &mut state.threads {
            thread.reported_error = Some(message.clone());
        }
        if state.threads.is_empty() || state.error_republish_timer.is_some() {
            return;
        }
        let manager = self.weak.clone();
        state.error_republish_timer = Some(tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(COMPUTER_ERROR_REPUBLISH_DEBOUNCE_MS)).await;
            let Some(manager) = manager.upgrade() else {
                return;
            };
            let threads: Vec<String> = {
                let mut state = manager.state();
                state.error_republish_timer = None;
                state.threads.iter().map(|(id, _)| id.clone()).collect()
            };
            for thread_id in threads {
                let manager = manager.clone();
                tokio::spawn(async move {
                    manager.publish(&thread_id).await;
                });
            }
        }));
    }

    pub async fn record_thread_error(&self, thread_id: &str, message: &str) {
        {
            let mut state = self.state();
            let Some(runtime) = state.thread_mut(thread_id) else {
                return;
            };
            runtime.reported_error = Some(clamp_computer_message(
                message,
                "The computer backend reported an error without a message.",
            ));
        }
        self.publish(thread_id).await;
    }

    pub async fn get_thread_state(&self, thread_id: &str) -> ThreadComputerState {
        let detached = {
            let mut state = self.state();
            if state.suspended_threads.contains(thread_id) {
                if state.thread(thread_id).is_some() {
                    None
                } else {
                    Some(state.new_thread_runtime())
                }
            } else {
                state.thread_runtime(thread_id);
                None
            }
        };
        self.refresh_physical_state().await;
        if let Some(snapshot) = self.publish(thread_id).await {
            return snapshot;
        }
        let state = self.state();
        match &detached {
            Some(runtime) => self.thread_snapshot(&state, thread_id, runtime),
            None => match state.thread(thread_id) {
                Some(runtime) => self.thread_snapshot(&state, thread_id, runtime),
                None => {
                    drop(state);
                    let mut state = self.state();
                    let runtime = state.new_thread_runtime();
                    self.thread_snapshot(&state, thread_id, &runtime)
                }
            },
        }
    }

    pub(super) fn surface_pane_for_agent(&self, thread_id: &str) {
        {
            let mut state = self.state();
            if state.suspended_threads.contains(thread_id) {
                return;
            }
            let runtime = state.thread_runtime(thread_id);
            if runtime.pane_surfaced {
                return;
            }
            runtime.pane_surfaced = true;
        }
        self.emit(ComputerEvent::OpenPaneRequested {
            thread_id: thread_id.to_owned(),
        });
    }

    pub(super) fn emit_action(
        &self,
        thread_id: Option<&str>,
        action: &str,
        result: Option<&ComputerActionResult>,
    ) {
        let attributed = agent_thread_id(thread_id);
        if let Some(attributed) = attributed {
            self.surface_pane_for_agent(attributed);
        }
        self.emit(ComputerEvent::Action {
            window_id: result.and_then(|result| result.window_id.clone()),
            delivery: result.and_then(|result| result.delivery.clone()),
            action: action.to_owned(),
            ok: true,
            message: None,
            thread_id: attributed.map(str::to_owned),
        });
    }
}
