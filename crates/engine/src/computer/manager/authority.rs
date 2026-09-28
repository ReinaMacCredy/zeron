use std::future::Future;
use std::sync::{Arc, Mutex};

use futures::FutureExt;
use serde_json::{Map, Value};
use tokio_util::sync::CancellationToken;
use zeron_proto::PermissionMode;
use zeron_proto::computer::{
    COMPUTER_MESSAGE_MAX_LENGTH, ComputerAllowedApp, ComputerPendingRequest, CuaComputerTask,
};

use super::{
    Authority, COMPUTER_CONTROL_TEARDOWN_TIMEOUT_MS, COMPUTER_USER_PAUSE_NOTE_PREFIX,
    COMPUTER_USER_REVOKED, ComputerCaller, ComputerConsent, ComputerConsentCard,
    ComputerConsentScope, ComputerConsentTarget, ComputerManager, agent_thread_id,
    display_app_name, lock, with_teardown_timeout,
};
use crate::computer::approval_gate::{AppGrant, ApprovalPublish, ApprovalRequest};
use crate::computer::backend::{ComputerBackendError, ComputerError, js_slice};
use crate::computer::call_context::{
    create_computer_call_context, current_computer_call, with_computer_call_context,
};
use crate::computer::desktop_operation_queue::{
    DesktopOperationError, OperationSignal, assert_desktop_operation_active,
    with_desktop_operation_signal, without_desktop_cancellation,
};

const REVOKED_ADMISSION: &str =
    "Computer control was revoked for this conversation; no input was dispatched.";

fn revoked(message: &str) -> ComputerBackendError {
    ComputerBackendError {
        control_revoked: true,
        ..ComputerBackendError::new(message)
    }
}

enum Settled {
    Nothing,
    Publish(String),
    Release(String, Option<String>),
}

struct InFlightGuard<'a> {
    manager: &'a ComputerManager,
    thread_id: String,
    owner: Option<String>,
    controller: Authority,
    armed: bool,
}

impl Drop for InFlightGuard<'_> {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        let settled = self.manager.settle_agent_call(
            &self.thread_id,
            self.owner.as_deref(),
            &self.controller,
        );
        let Some(manager) = self.manager.strong() else {
            return;
        };
        if let Ok(runtime) = tokio::runtime::Handle::try_current() {
            runtime.spawn(async move { manager.finish_settled(settled).await });
        }
    }
}

impl ComputerManager {
    pub(super) async fn with_computer_call<A>(
        &self,
        run: impl Future<Output = Result<A, ComputerError>>,
    ) -> Result<A, ComputerError> {
        let run = Box::pin(run);
        if current_computer_call().is_some() {
            return run.await;
        }
        let now = self.now.clone();
        let Some(context) = create_computer_call_context(
            self.env.timing_log,
            self.env.conditional_settle,
            Arc::new(move || now() as f64),
        ) else {
            return run.await;
        };
        let timing = context.clone();
        with_computer_call_context(context, async move {
            let result = run.await;
            if let Some(timing) = &timing.timing {
                if result.is_err() {
                    timing.mark_failed();
                }
                timing.finish();
            }
            result
        })
        .await
    }

    pub(super) fn assert_control_authority(
        &self,
        owner: Option<&str>,
    ) -> Result<(), ComputerError> {
        let Some(owner) = owner else {
            return Ok(());
        };
        let state = self.state();
        if !state.suspended_threads.contains(owner) && !state.revoked_threads.contains(owner) {
            return Ok(());
        }
        Err(revoked(REVOKED_ADMISSION).into())
    }

    pub(super) fn assert_input_not_paused(&self, owner: Option<&str>) -> Result<(), ComputerError> {
        let pause = owner.and_then(|owner| {
            self.state()
                .thread(owner)
                .and_then(|thread| thread.input_pause.clone())
        });
        match pause {
            Some(pause) => Err(ComputerBackendError {
                input_pause: Some(pause.clone()),
                ..ComputerBackendError::new(pause.message)
            }
            .into()),
            None => Ok(()),
        }
    }

    pub(super) fn record_input_pause(&self, owner: Option<&str>, error: &ComputerError) {
        let (Some(owner), Some(pause)) = (owner, error.input_pause()) else {
            return;
        };
        let episode = {
            let mut state = self.state();
            if state.disposed
                || state.suspended_threads.contains(owner)
                || state.revoked_threads.contains(owner)
            {
                return;
            }
            let held = state
                .thread_runtime(owner)
                .input_pause
                .replace(pause.clone());
            pause.user_takeover && !held.is_some_and(|held| held.user_takeover)
        };
        if episode {
            let app = pause
                .window_id
                .as_deref()
                .and_then(|window_id| display_app_name(self.window_app_name(window_id).as_deref()));
            self.seams.note(
                owner,
                &format!(
                    "{COMPUTER_USER_PAUSE_NOTE_PREFIX}{}",
                    app.as_deref().unwrap_or("the target window")
                ),
            );
        }
        self.publish_cached(owner);
    }

    pub fn can_activate_control(&self, thread_id: &str, generation: u64) -> bool {
        !self.state().suspended_threads.contains(thread_id)
            && self.control_state.allows(thread_id, generation)
    }

    fn thread_authority(&self, thread_id: &str) -> Authority {
        let mut state = self.state();
        let current = state
            .authority_revocations
            .get(thread_id)
            .filter(|authority| !authority.aborted())
            .cloned();
        match current {
            Some(authority) => authority,
            None => {
                let authority = Authority::new();
                state
                    .authority_revocations
                    .insert(thread_id.to_owned(), authority.clone());
                state.revoked_threads.remove(thread_id);
                authority
            }
        }
    }

    pub fn thread_authority_signal(&self, thread_id: &str) -> OperationSignal {
        self.thread_authority(thread_id).token.clone().into()
    }

    pub async fn with_agent_activity<A, F, Fut>(
        &self,
        thread_id: &str,
        action: F,
        signal: Option<OperationSignal>,
        turn_id: Option<&str>,
        operation_key: Option<&str>,
    ) -> Result<A, ComputerError>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<A, ComputerError>>,
    {
        assert_desktop_operation_active()?;
        let pending = self.state().pending_stops.get(thread_id).cloned();
        if let Some(pending) = pending {
            let drained = with_teardown_timeout(async {
                let _ = pending.await;
                Ok(())
            })
            .await;
            if drained.is_err() {
                return Err(revoked(REVOKED_ADMISSION).into());
            }
        }
        let authority = self.thread_authority(thread_id);
        let broadcast: OperationSignal = authority.token.clone().into();
        let admission = match signal {
            Some(signal) => signal.any(&broadcast),
            None => broadcast,
        };
        let controller: Arc<Mutex<Option<Authority>>> = Arc::default();
        let slot = controller.clone();
        let execute = move || self.execute_agent_activity(thread_id, turn_id, action, slot);
        let run = async {
            match operation_key {
                Some(key) => {
                    self.operations
                        .run_scoped(key, execute, Some(admission))
                        .await
                }
                None => self.operations.run(execute, Some(admission)).await,
            }
        };
        let result = self.with_computer_call(Box::pin(run)).await;
        result.map_err(|error| {
            if !matches!(
                error,
                ComputerError::Operation(DesktopOperationError::Aborted)
            ) {
                return error;
            }
            let live = lock(&controller).as_ref().and_then(Authority::reason);
            match live.or_else(|| authority.reason()) {
                Some(reason) => reason.into(),
                None => error,
            }
        })
    }

    async fn execute_agent_activity<A, F, Fut>(
        &self,
        thread_id: &str,
        turn_id: Option<&str>,
        action: F,
        slot: Arc<Mutex<Option<Authority>>>,
    ) -> Result<A, ComputerError>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<A, ComputerError>>,
    {
        self.assert_control_authority(Some(thread_id))?;
        let controller = Authority::new();
        *lock(&slot) = Some(controller.clone());
        let owner = agent_thread_id(Some(thread_id)).map(str::to_owned);
        let first = {
            let mut state = self.state();
            state
                .active_authorities
                .entry(thread_id.to_owned())
                .or_default()
                .push(controller.clone());
            match &owner {
                Some(owner) => {
                    if let Some(turn_id) = turn_id {
                        state
                            .authority_turns
                            .insert(owner.clone(), turn_id.to_owned());
                        super::left_open::begin_turn(&mut state, owner, turn_id);
                    }
                    let depth = state
                        .agent_calls_in_flight
                        .entry(owner.clone())
                        .or_insert(0);
                    *depth += 1;
                    *depth == 1
                }
                None => false,
            }
        };
        self.sync_still_stream();
        if first && let Some(owner) = &owner {
            self.publish_cached(owner);
        }
        let mut guard = InFlightGuard {
            manager: self,
            thread_id: thread_id.to_owned(),
            owner,
            controller: controller.clone(),
            armed: true,
        };
        let result = Box::pin(with_desktop_operation_signal(
            controller.token.clone().into(),
            action(),
        ))
        .await;
        guard.armed = false;
        let settled = self.settle_agent_call(thread_id, guard.owner.as_deref(), &controller);
        self.finish_settled(settled).await;
        result
    }

    fn settle_agent_call(
        &self,
        thread_id: &str,
        owner: Option<&str>,
        controller: &Authority,
    ) -> Settled {
        {
            let mut state = self.state();
            if let Some(live) = state.active_authorities.get_mut(thread_id) {
                live.retain(|candidate| !candidate.same(controller));
                if live.is_empty() {
                    state.active_authorities.remove(thread_id);
                }
            }
        }
        self.sync_still_stream();
        let Some(owner) = owner else {
            return Settled::Nothing;
        };
        {
            let mut state = self.state();
            let remaining = state
                .agent_calls_in_flight
                .get(owner)
                .copied()
                .unwrap_or(1)
                .saturating_sub(1);
            if remaining > 0 {
                state
                    .agent_calls_in_flight
                    .insert(owner.to_owned(), remaining);
                return Settled::Nothing;
            }
            state.agent_calls_in_flight.remove(owner);
        }
        self.release_background_control(owner, None, true);
        let mut state = self.state();
        let lease_owner = state
            .lease
            .as_ref()
            .is_some_and(|lease| lease.thread_id == owner);
        if !lease_owner && !state.owns_background_lease(owner) {
            state.authority_turns.remove(owner);
        }
        match state.lease.as_mut() {
            Some(lease) if lease.thread_id == owner && lease.release_requested => {
                let requested = lease.release_requested_turn_id.clone();
                let still_matches = match &requested {
                    None => lease.turn_id.is_none(),
                    Some(turn) => lease.turn_id.as_ref() == Some(turn),
                };
                if still_matches {
                    Settled::Release(owner.to_owned(), requested)
                } else {
                    lease.release_requested = false;
                    lease.release_requested_turn_id = None;
                    Settled::Publish(owner.to_owned())
                }
            }
            _ => Settled::Publish(owner.to_owned()),
        }
    }

    async fn finish_settled(&self, settled: Settled) {
        match settled {
            Settled::Nothing => {}
            Settled::Publish(owner) => {
                self.publish_cached(&owner);
            }
            Settled::Release(owner, turn_id) => {
                let _ = without_desktop_cancellation(
                    self.release_desktop_control(&owner, turn_id.as_deref()),
                )
                .await;
            }
        }
    }

    pub fn revoke_control(
        &self,
        thread_id: &str,
    ) -> impl Future<Output = Result<(), ComputerError>> + use<> {
        self.approval_gate.cancel_thread(thread_id, None);
        let reason = revoked(COMPUTER_USER_REVOKED);
        {
            let mut state = self.state();
            state.revoked_threads.insert(thread_id.to_owned());
            state
                .authority_revocations
                .entry(thread_id.to_owned())
                .or_insert_with(Authority::new)
                .abort(&reason);
            for controller in state
                .active_authorities
                .get(thread_id)
                .into_iter()
                .flatten()
            {
                controller.abort(&reason);
            }
            match state.pending_stops.get(thread_id) {
                Some(pending) => pending.clone(),
                None => {
                    let manager = self.weak.clone();
                    let thread = thread_id.to_owned();
                    let stop = async move {
                        let Some(manager) = manager.upgrade() else {
                            return Ok(());
                        };
                        let result = manager.stop_and_release(&thread).await;
                        manager.state().pending_stops.remove(&thread);
                        result
                    }
                    .boxed()
                    .shared();
                    state
                        .pending_stops
                        .insert(thread_id.to_owned(), stop.clone());
                    stop
                }
            }
        }
    }

    async fn stop_and_release(&self, thread_id: &str) -> Result<(), ComputerError> {
        let task = {
            let state = self.state();
            let driving = state
                .lease
                .as_ref()
                .is_some_and(|lease| lease.thread_id == thread_id)
                || state.owns_background_lease(thread_id)
                || state
                    .active_authorities
                    .get(thread_id)
                    .is_some_and(|live| !live.is_empty());
            driving.then(|| CuaComputerTask {
                thread_id: thread_id.to_owned(),
                turn_id: state.authority_turns.get(thread_id).cloned(),
                label: None,
            })
        };
        if let Some(task) = task {
            self.backend.stop_input(Some(&task)).await?;
        }
        self.release_desktop_control(thread_id, None).await
    }

    pub async fn stop_thread(&self, thread_id: &str) -> Result<u64, ComputerError> {
        let generation = self.control_state.bump(thread_id);
        self.seams.generation_bumped(thread_id, generation);
        if let Some(runtime) = self.state().thread_mut(thread_id) {
            runtime.pane_surfaced = false;
        }
        let outcome = with_teardown_timeout(self.revoke_control(thread_id)).await;
        let suspended = self.state().suspended_threads.contains(thread_id);
        if !suspended {
            self.state().thread_runtime(thread_id);
            self.publish_cached(thread_id);
        }
        outcome.map(|()| generation)
    }

    pub async fn emergency_stop_input(&self) -> Result<(), ComputerError> {
        self.emergency_stop().await.1
    }

    pub fn desktop_holders(&self) -> Vec<String> {
        let state = self.state();
        if state.disposed {
            return Vec::new();
        }
        let mut threads: Vec<String> = state
            .active_authorities
            .iter()
            .filter(|(_, live)| !live.is_empty())
            .map(|(thread, _)| thread.clone())
            .collect();
        threads.extend(state.lease.iter().map(|lease| lease.thread_id.clone()));
        threads.extend(
            state
                .background_leases
                .iter()
                .map(|(_, lease)| lease.lease.thread_id.clone()),
        );
        threads.sort();
        threads.dedup();
        threads
            .iter()
            .filter_map(|thread| agent_thread_id(Some(thread)))
            .map(str::to_owned)
            .collect()
    }

    pub async fn emergency_stop(&self) -> (Vec<String>, Result<(), ComputerError>) {
        if self.state().disposed {
            return (Vec::new(), Ok(()));
        }
        let stopped = self.desktop_holders();
        let reason = revoked(COMPUTER_USER_REVOKED);
        {
            let state = self.state();
            for authority in state.authority_revocations.values() {
                authority.abort(&reason);
            }
            for live in state.active_authorities.values() {
                for controller in live {
                    controller.abort(&reason);
                }
            }
        }
        for thread_id in &stopped {
            let generation = self.control_state.bump(thread_id);
            self.seams.generation_bumped(thread_id, generation);
        }
        let result = self.backend.stop_input(None).await;
        (stopped, result)
    }

    pub async fn revoke_desktop(&self, thread_id: &str) -> Result<(), ComputerError> {
        self.set_desktop_revoked(thread_id, true);
        with_teardown_timeout(self.revoke_control(thread_id)).await
    }

    pub fn clear_desktop_revoked(&self, thread_id: &str) {
        self.set_desktop_revoked(thread_id, false);
    }

    fn set_desktop_revoked(&self, thread_id: &str, revoked: bool) {
        {
            let mut state = self.state();
            if state.disposed || state.suspended_threads.contains(thread_id) {
                return;
            }
            let runtime = if revoked {
                Some(state.thread_runtime(thread_id))
            } else {
                state.thread_mut(thread_id)
            };
            match runtime {
                Some(runtime) if runtime.desktop_revoked != revoked => {
                    runtime.desktop_revoked = revoked;
                }
                _ => return,
            }
        }
        self.publish_cached(thread_id);
    }

    pub fn pending_request(&self, thread_id: &str) -> Option<ComputerPendingRequest> {
        self.state()
            .thread(thread_id)
            .and_then(|runtime| runtime.pending_requests.front())
            .map(|(_, request)| request.clone())
    }

    pub fn pending_requests(&self, thread_id: &str) -> Vec<ComputerPendingRequest> {
        self.state()
            .thread(thread_id)
            .map_or_else(Vec::new, |runtime| {
                runtime
                    .pending_requests
                    .iter()
                    .map(|(_, request)| request.clone())
                    .collect()
            })
    }

    pub fn push_pending_request(
        &self,
        thread_id: &str,
        turn_id: Option<&str>,
        request: ComputerPendingRequest,
    ) {
        {
            let mut state = self.state();
            if state.disposed {
                return;
            }
            let queue = &mut state.thread_runtime(thread_id).pending_requests;
            if queue
                .iter()
                .any(|(_, queued)| queued.input_id == request.input_id)
            {
                return;
            }
            queue.push_back((turn_id.map(str::to_owned), request));
            if queue.len() > 1 {
                return;
            }
        }
        self.publish_cached(thread_id);
    }

    pub fn clear_pending_request(&self, thread_id: &str, input_id: &str) {
        self.take_pending_requests(thread_id, |_, request| request.input_id == input_id);
    }

    pub fn drain_pending_requests(
        &self,
        thread_id: &str,
        turn_id: Option<&str>,
    ) -> Vec<ComputerPendingRequest> {
        self.take_pending_requests(thread_id, |turn, _| {
            turn_id.is_none_or(|ended| turn.is_none_or(|turn| turn == ended))
        })
    }

    fn take_pending_requests(
        &self,
        thread_id: &str,
        matches: impl Fn(Option<&str>, &ComputerPendingRequest) -> bool,
    ) -> Vec<ComputerPendingRequest> {
        let (taken, head_changed) = {
            let mut state = self.state();
            let Some(runtime) = state.thread_mut(thread_id) else {
                return Vec::new();
            };
            let head = runtime
                .pending_requests
                .front()
                .map(|(_, request)| request.input_id.clone());
            let (taken, kept): (Vec<_>, Vec<_>) = std::mem::take(&mut runtime.pending_requests)
                .into_iter()
                .partition(|(turn, request)| matches(turn.as_deref(), request));
            runtime.pending_requests = kept.into();
            let next = runtime
                .pending_requests
                .front()
                .map(|(_, request)| request.input_id.clone());
            (taken, head != next)
        };
        if head_changed {
            self.publish_cached(thread_id);
        }
        taken.into_iter().map(|(_, request)| request).collect()
    }

    pub async fn handle_thread_removed(&self, thread_id: &str) {
        self.space_broker.release(thread_id, None);
        self.approval_gate.cancel_thread(thread_id, None);
        self.state().suspended_threads.insert(thread_id.to_owned());
        let _ = with_teardown_timeout(self.revoke_control(thread_id)).await;
        {
            let mut state = self.state();
            state.remove_thread(thread_id);
            state.thread_labels.retain(|(id, _)| id != thread_id);
            state.authority_turns.remove(thread_id);
            state.turn_windows.remove(thread_id);
            state.authority_revocations.remove(thread_id);
            state.active_authorities.remove(thread_id);
        }
        self.sync_still_stream();
        let _ = with_teardown_timeout(self.release_desktop_control(thread_id, None)).await;
        if let Some(browser) = self.backend.browser() {
            let _ = with_teardown_timeout(browser.end_thread(thread_id)).await;
        }
    }

    pub async fn handle_thread_restored(&self, thread_id: &str) {
        let pending = self.state().pending_stops.get(thread_id).cloned();
        if let Some(pending) = pending {
            let _ = pending.await;
        }
        let mut state = self.state();
        state.suspended_threads.remove(thread_id);
        state.revoked_threads.remove(thread_id);
        state.authority_revocations.remove(thread_id);
    }

    pub async fn authorize_action(
        &self,
        tool_name: &str,
        args: &Map<String, Value>,
        caller: &ComputerCaller,
        signal: CancellationToken,
    ) -> Result<bool, ComputerError> {
        let consent = self
            .authorize_consent(tool_name, args, caller, signal, None)
            .await?;
        Ok(consent == ComputerConsent::Granted)
    }

    pub async fn authorize_consent(
        &self,
        tool_name: &str,
        args: &Map<String, Value>,
        caller: &ComputerCaller,
        signal: CancellationToken,
        target: Option<&ComputerConsentTarget>,
    ) -> Result<ComputerConsent, ComputerError> {
        let Some(mode) = self
            .seams
            .caller(&caller.chat_id, caller.turn_id.as_deref())
            .await?
        else {
            return Ok(ComputerConsent::Denied(None));
        };
        if mode != PermissionMode::Ask {
            return Ok(ComputerConsent::Granted);
        }
        if target == Some(&ComputerConsentTarget::Unscoped) {
            return Ok(ComputerConsent::Unscoped);
        }
        let task_consent = tool_name != "computer_read_clipboard" && caller.turn_id.is_some();
        let resolved = match target {
            Some(target) => Some(self.resolve_consent_app(target).await),
            None => None,
        };
        let app = resolved.as_ref().map(|resolved| resolved.app.clone());
        let params_display = consent_params(args);
        let seams = self.seams.clone();
        let chat_id = caller.chat_id.clone();
        let turn_id = caller.turn_id.clone();
        let tool = tool_name.to_owned();
        let scope = if task_consent {
            ComputerConsentScope::Task
        } else {
            ComputerConsentScope::Call
        };
        let card_app = app.clone();
        let publish: ApprovalPublish = Arc::new(move |request_id, decision| {
            let seams = seams.clone();
            let card = ComputerConsentCard {
                request_id,
                chat_id: chat_id.clone(),
                turn_id: turn_id.clone(),
                tool_name: tool.clone(),
                params_display: params_display.clone(),
                scope,
                app: card_app.clone(),
            };
            async move { seams.publish_consent(card, decision).await }.boxed()
        });
        let grant = resolved.map(|resolved| AppGrant {
            always: resolved
                .app
                .bundle_id
                .clone()
                .filter(|_| task_consent)
                .map(|bundle_id| ComputerAllowedApp {
                    bundle_id,
                    name: Some(resolved.app.name.clone()),
                }),
            key: resolved.key,
        });
        let request = ApprovalRequest {
            chat_id: caller.chat_id.clone(),
            turn_id: caller.turn_id.clone(),
            mode,
            signal,
            publish,
            app: grant,
        };
        let approved = if task_consent {
            self.approval_gate.request_task(request).await?
        } else {
            self.approval_gate.request(request).await?
        };
        if !approved {
            return Ok(ComputerConsent::Denied(app));
        }
        Ok(ComputerConsent::Granted)
    }

    pub async fn dispose(&self) {
        {
            let mut state = self.state();
            if state.disposed {
                return;
            }
            state.disposed = true;
        }
        self.space_broker.dispose();
        self.cursor_activity.dispose();
        let _ = with_teardown_timeout(self.backend.stop_input(None)).await;
        let close = self.operations.close();
        let _ = tokio::time::timeout(
            std::time::Duration::from_millis(COMPUTER_CONTROL_TEARDOWN_TIMEOUT_MS),
            close,
        )
        .await;
        {
            let mut state = self.state();
            if let Some(timer) = state.windows_publish_timer.take() {
                timer.abort();
            }
            state.windows_publish_pending = false;
            if let Some(timer) = state.error_republish_timer.take() {
                timer.abort();
            }
            state.stream.desired = false;
            state.stream.epoch += 1;
        }
        let _ = self.stream_teardown().await;
        lock(&self.backend_subscription).take();
        self.backend.dispose().await;
        lock(&self.listeners).clear();
        self.audit_log.flush().await;
    }
}

const CONSENT_PARAM_TEXT_MAX: usize = 200;

pub(crate) fn consent_params(args: &Map<String, Value>) -> String {
    let projected = Value::Object(consent_projection(args)).to_string();
    if projected.encode_utf16().count() <= COMPUTER_MESSAGE_MAX_LENGTH {
        projected
    } else {
        "{}".to_owned()
    }
}

fn consent_projection(args: &Map<String, Value>) -> Map<String, Value> {
    let text = |value: &Value| {
        value
            .as_str()
            .map(|text| Value::String(js_slice(text, CONSENT_PARAM_TEXT_MAX).to_owned()))
    };
    args.iter()
        .filter_map(|(key, value)| {
            let kept = match key.as_str() {
                "window_id" | "label" | "role" | "key" | "app" | "app_name" | "application"
                | "name" | "bundle_id" | "action" | "operation" | "topic" | "tool"
                | "delivery_mode" => text(value),
                "x" | "y" | "delta_x" | "delta_y" | "width" | "height" | "start" | "length"
                | "space_id" | "duration_ms" | "pid" => value.is_number().then(|| value.clone()),
                "minimized" | "hidden" | "allow_launch" | "windowed" | "replace" => {
                    value.is_boolean().then(|| value.clone())
                }
                "keys" | "path" => value
                    .as_array()
                    .map(|items| Value::Array(items.iter().filter_map(text).collect())),
                "from" | "to" | "arguments" => value
                    .as_object()
                    .map(|inner| Value::Object(consent_projection(inner))),
                "steps" => value.as_array().map(|steps| {
                    Value::Array(
                        steps
                            .iter()
                            .map(|step| {
                                Value::Object(
                                    step.get("type")
                                        .and_then(text)
                                        .map(|kind| Map::from_iter([("type".to_owned(), kind)]))
                                        .unwrap_or_default(),
                                )
                            })
                            .collect(),
                    )
                }),
                "url" => value.as_str().and_then(url_origin).map(Value::String),
                "files" => value
                    .as_array()
                    .map(|files| Value::Array(vec![Value::String(String::new()); files.len()])),
                _ => None,
            };
            kept.map(|kept| (key.clone(), kept))
        })
        .collect()
}

fn url_origin(url: &str) -> Option<String> {
    let (scheme, rest) = url.trim().split_once("://")?;
    let scheme = scheme.to_ascii_lowercase();
    if scheme != "http" && scheme != "https" {
        return None;
    }
    let authority = rest.split(['/', '?', '#', '\\']).next()?;
    let host = authority.rsplit('@').next()?;
    (!host.is_empty()).then(|| format!("{scheme}://{}", js_slice(host, CONSENT_PARAM_TEXT_MAX)))
}
