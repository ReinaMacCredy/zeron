use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use futures::stream::BoxStream;
use futures::{FutureExt, StreamExt};
use tokio::sync::{broadcast, watch};
use zeron_proto::computer::{
    ComputerAllowedApp, ComputerEvent, ComputerHostStreamItem, ComputerSettings,
    ComputerStillFrame, RespondComputerHostRequestParams,
};
use zeron_rpc::ComputerHost;

use super::activation::{TurnDispatch, resolve_turn_mode};
use super::approval_gate::ComputerApprovalGate;
use super::control_state::ComputerControlState;
use super::engine_seams::EngineComputerSeams;
use super::host_wire::{HostRegistration, HostSlot};
use super::manager::{ComputerManagerSeams, ComputerManagerSubscription, left_open_note};
use super::service::{ComputerService, ComputerServiceOptions};
use super::tool_calls::ComputerCalls;
use crate::sessions::SessionsEngine;

const SETTINGS_FILE: &str = "computer-settings.json";
const EVENT_BUFFER: usize = 256;
const SHUTDOWN_BUDGET: Duration = Duration::from_secs(5);

pub const COMPUTER_ESCAPE_STOP_NOTICE: &str = "Computer use stopped: Escape was pressed";
pub const COMPUTER_DESKTOP_REVOKED_NOTE: &str = "Desktop access revoked by you";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RevokeSource {
    Escape,
    Stop,
}

impl RevokeSource {
    pub fn hint(self) -> &'static str {
        match self {
            Self::Escape => "Esc",
            Self::Stop => "Stop",
        }
    }
}

pub struct ComputerRuntime {
    data_dir: PathBuf,
    control_state: Arc<ComputerControlState>,
    approval_gate: ComputerApprovalGate,
    engine_seams: Arc<EngineComputerSeams>,
    seams: Arc<dyn ComputerManagerSeams>,
    hosts: Arc<HostSlot>,
    service: watch::Sender<Option<Arc<ComputerService>>>,
    rebuilds: tokio::sync::Mutex<()>,
    shut_down: AtomicBool,
    settings: Mutex<ComputerSettings>,
    events: broadcast::Sender<ComputerEvent>,
    forwarding: Mutex<Option<ComputerManagerSubscription>>,
    chat_watch: Mutex<Option<tokio::task::JoinHandle<()>>>,
    calls: Arc<ComputerCalls>,
}

impl ComputerRuntime {
    pub fn start(data_dir: &Path) -> Arc<Self> {
        let settings = std::fs::read_to_string(data_dir.join(SETTINGS_FILE))
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default();
        let approval_gate = ComputerApprovalGate::default();
        approval_gate.set_always_allowed(always_allowed_ids(&settings));
        let engine_seams = Arc::new(EngineComputerSeams::new(approval_gate.clone()));
        let runtime = Arc::new(Self {
            data_dir: data_dir.to_path_buf(),
            control_state: Arc::default(),
            approval_gate,
            seams: engine_seams.clone(),
            engine_seams,
            hosts: Arc::default(),
            service: watch::channel(None).0,
            rebuilds: tokio::sync::Mutex::new(()),
            shut_down: AtomicBool::new(false),
            settings: Mutex::new(settings),
            events: broadcast::channel(EVENT_BUFFER).0,
            forwarding: Mutex::new(None),
            chat_watch: Mutex::new(None),
            calls: Arc::default(),
        });
        let weak = Arc::downgrade(&runtime);
        runtime
            .approval_gate
            .on_always_allowed(Arc::new(move |app| {
                if let Some(runtime) = weak.upgrade()
                    && let Err(error) = runtime.remember_always_allowed(app)
                {
                    tracing::warn!(%error, "computer always-allowed app not saved");
                }
            }));
        let weak = Arc::downgrade(&runtime);
        runtime
            .engine_seams
            .on_pending(Arc::new(move |chat_id, turn_id, input_id, request| {
                let Some(service) = weak.upgrade().and_then(|runtime| runtime.service()) else {
                    return;
                };
                match request {
                    Some(request) => service
                        .manager
                        .push_pending_request(chat_id, turn_id, request),
                    None => service.manager.clear_pending_request(chat_id, input_id),
                }
            }));
        tokio::spawn(runtime.clone().rebuild(None));
        runtime
    }

    pub fn attach(self: &Arc<Self>, sessions: &SessionsEngine) {
        self.engine_seams.attach(sessions.clone());
        sessions.set_computer(self.clone());
        let Some(workspace) = sessions.workspace() else {
            return;
        };
        let mut chats = workspace.watch_chats();
        let weak = Arc::downgrade(self);
        let watcher = tokio::spawn(async move {
            let mut known: std::collections::HashMap<String, (bool, Option<String>)> = chats
                .borrow_and_update()
                .iter()
                .map(|chat| (chat.id.clone(), (chat.archived, chat.title.clone())))
                .collect();
            while chats.changed().await.is_ok() {
                let current: std::collections::HashMap<String, (bool, Option<String>)> = chats
                    .borrow_and_update()
                    .iter()
                    .map(|chat| (chat.id.clone(), (chat.archived, chat.title.clone())))
                    .collect();
                let Some(runtime) = weak.upgrade() else {
                    return;
                };
                for change in chat_changes(&known, &current) {
                    runtime.apply_chat_change(change).await;
                }
                known = current;
            }
        });
        *self
            .chat_watch
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Some(watcher);
    }

    async fn apply_chat_change(&self, change: ChatChange) {
        if let ChatChange::Removed(chat_id) = &change {
            self.calls.forget_chat(chat_id);
        }
        let Some(service) = self.service() else {
            return;
        };
        let manager = &service.manager;
        match change {
            ChatChange::Removed(chat_id) => manager.handle_thread_removed(&chat_id).await,
            ChatChange::Restored(chat_id) => manager.handle_thread_restored(&chat_id).await,
            ChatChange::Titled(chat_id, title) => {
                manager.set_thread_label(&chat_id, title.as_deref())
            }
        }
    }

    pub fn subscribe_events(&self) -> broadcast::Receiver<ComputerEvent> {
        self.events.subscribe()
    }

    pub fn set_chat_label(&self, chat_id: &str, title: Option<&str>) {
        if let Some(service) = self.service() {
            service.manager.set_thread_label(chat_id, title);
        }
    }

    pub fn chat_generation(&self, chat_id: &str, doc_generation: u64) -> u64 {
        self.control_state.seed(chat_id, doc_generation);
        self.control_state.generation(chat_id)
    }

    pub fn resolve_computer_control(
        &self,
        chat_id: &str,
        doc_generation: u64,
        text: &str,
        user_authored: bool,
        stamped_generation: Option<u64>,
    ) -> bool {
        let current = self.chat_generation(chat_id, doc_generation);
        resolve_turn_mode(
            TurnDispatch {
                settings_enabled: self.settings().computer_control_enabled,
                user_authored,
                text,
                stamped_generation: stamped_generation.unwrap_or(0),
            },
            current,
        )
        .enabled()
    }

    pub fn calls(&self) -> &Arc<ComputerCalls> {
        &self.calls
    }

    #[cfg(test)]
    pub(crate) async fn install_backend(&self, backend: Arc<dyn super::backend::ComputerBackend>) {
        let _rebuilding = self.rebuilds.lock().await;
        if let Some(previous) = self.service.send_replace(None) {
            previous.dispose().await;
        }
        let service = ComputerService::boot(ComputerServiceOptions {
            backend: Some(backend),
            control_state: self.control_state.clone(),
            approval_gate: self.approval_gate.clone(),
            seams: self.seams.clone(),
            env: super::manager::ComputerManagerEnv::default(),
            ..ComputerServiceOptions::default()
        })
        .await;
        self.service.send_replace(Some(Arc::new(service)));
    }

    pub fn stop_chat(&self, chat_id: &str, doc_generation: u64) {
        self.calls.cancel_chat(chat_id, None);
        self.control_state.seed(chat_id, doc_generation);
        let Some(service) = self.service() else {
            let generation = self.control_state.bump(chat_id);
            self.seams.generation_bumped(chat_id, generation);
            return;
        };
        let chat = chat_id.to_owned();
        let mut stop = Box::pin(async move {
            if let Err(error) = service.manager.stop_thread(&chat).await {
                tracing::warn!(chat = %chat, %error, "computer stop failed");
            }
        });
        // The first poll bumps, records and revokes; only the host teardown is left to the task.
        if (&mut stop).now_or_never().is_none() {
            tokio::spawn(stop);
        }
    }

    pub async fn emergency_stop(&self) -> Vec<String> {
        let Some(service) = self.service() else {
            return Vec::new();
        };
        let sessions = self.engine_seams.sessions();
        let latched: Vec<String> = service
            .manager
            .desktop_holders()
            .into_iter()
            .filter(|chat_id| {
                sessions
                    .as_ref()
                    .is_some_and(|sessions| sessions.revoke_computer_turn(chat_id).is_some())
            })
            .collect();
        let (mut stopped, result) = service.manager.emergency_stop().await;
        if let Err(error) = result {
            tracing::warn!(%error, "computer emergency stop relay failed");
        }
        if let Some(sessions) = &sessions {
            for chat_id in &latched {
                self.finish_revoke(sessions, chat_id, RevokeSource::Escape)
                    .await;
            }
        }
        for chat_id in stopped.iter().filter(|chat_id| !latched.contains(chat_id)) {
            self.revoke_desktop(chat_id, RevokeSource::Escape).await;
        }
        stopped.extend(latched);
        stopped.sort();
        stopped.dedup();
        stopped
    }

    pub async fn revoke_desktop(&self, chat_id: &str, source: RevokeSource) -> bool {
        let Some(sessions) = self.engine_seams.sessions() else {
            return false;
        };
        if sessions.revoke_computer_turn(chat_id).is_none() {
            return false;
        }
        self.finish_revoke(&sessions, chat_id, source).await;
        true
    }

    async fn finish_revoke(&self, sessions: &SessionsEngine, chat_id: &str, source: RevokeSource) {
        match self.service() {
            Some(service) => {
                for pending in service.manager.drain_pending_requests(chat_id, None) {
                    sessions.settle_input(chat_id, &pending.input_id);
                }
                if let Err(error) = service.manager.revoke_desktop(chat_id).await {
                    tracing::warn!(chat = %chat_id, %error, "computer desktop revoke failed");
                }
            }
            None => tracing::warn!(
                chat = %chat_id,
                "computer desktop revoke latched the turn with no computer service to drain"
            ),
        }
        sessions.write_computer_note(chat_id, COMPUTER_DESKTOP_REVOKED_NOTE, Some(source.hint()));
    }

    pub fn turn_ended(&self, chat_id: &str, turn_id: &str) {
        self.calls.cancel_chat(chat_id, Some(turn_id));
        self.approval_gate.cancel_thread(chat_id, Some(turn_id));
        let Some(service) = self.service() else {
            return;
        };
        service.manager.clear_desktop_revoked(chat_id);
        let sessions = self.engine_seams.sessions();
        for pending in service
            .manager
            .drain_pending_requests(chat_id, Some(turn_id))
        {
            if let Some(sessions) = &sessions {
                sessions.settle_input(chat_id, &pending.input_id);
            }
        }
        let (chat_id, turn_id) = (chat_id.to_owned(), turn_id.to_owned());
        tokio::spawn(async move {
            if let Err(error) = service
                .manager
                .release_desktop_control(&chat_id, Some(&turn_id))
                .await
            {
                tracing::warn!(chat = %chat_id, %error, "computer turn-end release failed");
            }
            let left = service.manager.left_open_windows(&chat_id, &turn_id).await;
            if let Some(sessions) = &sessions {
                for left in &left {
                    sessions.write_computer_note(&chat_id, &left_open_note(left), None);
                }
            }
        });
    }

    pub fn approval_gate(&self) -> &ComputerApprovalGate {
        &self.approval_gate
    }

    pub fn control_state(&self) -> &Arc<ComputerControlState> {
        &self.control_state
    }

    pub fn service(&self) -> Option<Arc<ComputerService>> {
        self.service.borrow().clone()
    }

    pub fn watch_service(&self) -> watch::Receiver<Option<Arc<ComputerService>>> {
        self.service.subscribe()
    }

    pub fn host_bundle_id(&self) -> Option<String> {
        self.hosts.live_bundle_id()
    }

    pub fn settings(&self) -> ComputerSettings {
        self.settings
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub fn set_settings(&self, settings: ComputerSettings) -> Result<(), String> {
        let mut current = self.settings.lock().unwrap_or_else(PoisonError::into_inner);
        self.write_settings(&mut current, settings)
    }

    fn remember_always_allowed(&self, app: ComputerAllowedApp) -> Result<(), String> {
        let mut current = self.settings.lock().unwrap_or_else(PoisonError::into_inner);
        if current
            .always_allowed_apps
            .iter()
            .any(|saved| saved.bundle_id == app.bundle_id)
        {
            return Ok(());
        }
        let mut next = current.clone();
        next.always_allowed_apps.push(app);
        self.write_settings(&mut current, next)
    }

    fn write_settings(
        &self,
        current: &mut ComputerSettings,
        settings: ComputerSettings,
    ) -> Result<(), String> {
        let json = serde_json::to_string_pretty(&settings).map_err(|e| e.to_string())?;
        let path = self.data_dir.join(SETTINGS_FILE);
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, json)
            .and_then(|()| std::fs::rename(&tmp, &path))
            .map_err(|e| format!("computer settings save failed: {e}"))?;
        self.approval_gate
            .set_always_allowed(always_allowed_ids(&settings));
        *current = settings;
        Ok(())
    }

    pub fn register_host(
        self: &Arc<Self>,
        bundle_id: String,
    ) -> BoxStream<'static, ComputerHostStreamItem> {
        let weak = Arc::downgrade(self);
        let on_lost = Box::new(move || {
            let Some(runtime) = weak.upgrade() else {
                return;
            };
            runtime.approval_gate.revoke_task_grants();
            if let Ok(handle) = tokio::runtime::Handle::try_current() {
                handle.spawn(runtime.rebuild(None));
            }
        });
        let (registration, items) = match self.hosts.register(bundle_id, on_lost) {
            Ok(accepted) => accepted,
            Err(refused) => {
                return futures::stream::once(async move {
                    ComputerHostStreamItem::Refused {
                        retry_after_ms: refused.retry_after_ms,
                    }
                })
                .boxed();
            }
        };
        let host: Arc<dyn ComputerHost> = Arc::new(registration.host());
        tokio::spawn(self.clone().rebuild(Some((registration.id(), host))));
        let registered = ComputerHostStreamItem::Registered {
            host_token: registration.token().to_owned(),
        };
        futures::stream::once(async move { registered })
            .chain(futures::stream::unfold(
                (items, registration),
                |(mut items, registration): (_, HostRegistration)| async move {
                    let item = items.recv().await?;
                    Some((item, (items, registration)))
                },
            ))
            .boxed()
    }

    pub fn respond(&self, reply: RespondComputerHostRequestParams) -> Result<bool, String> {
        self.hosts.respond(reply)
    }

    async fn rebuild(self: Arc<Self>, host: Option<(u64, Arc<dyn ComputerHost>)>) {
        let _rebuilding = self.rebuilds.lock().await;
        if self.shut_down.load(Ordering::SeqCst)
            || self.hosts.live_id() != host.as_ref().map(|(id, _)| *id)
        {
            return;
        }
        if let Some(previous) = self.service.send_replace(None) {
            previous.dispose().await;
        }
        self.approval_gate.revoke_task_grants();
        let service = ComputerService::boot(ComputerServiceOptions {
            host: host.map(|(_, host)| host),
            data_dir: Some(self.data_dir.clone()),
            control_state: self.control_state.clone(),
            approval_gate: self.approval_gate.clone(),
            seams: self.seams.clone(),
            ..ComputerServiceOptions::default()
        })
        .await;
        let events = self.events.clone();
        let forwarding = service.manager.on_event(Arc::new(move |event| {
            let _ = events.send(event.clone());
        }));
        *self
            .forwarding
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Some(forwarding);
        self.service.send_replace(Some(Arc::new(service)));
        // Pairs with shutdown's flag-then-take: one side always sees the other.
        if self.shut_down.load(Ordering::SeqCst) {
            self.forwarding
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .take();
            if let Some(service) = self.service.send_replace(None) {
                service.dispose().await;
            }
        }
    }

    pub async fn shutdown(&self) {
        self.shut_down.store(true, Ordering::SeqCst);
        if let Some(watcher) = self
            .chat_watch
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take()
        {
            watcher.abort();
        }
        self.engine_seams.detach();
        self.forwarding
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        if let Some(service) = self.service.send_replace(None)
            && tokio::time::timeout(SHUTDOWN_BUDGET, service.dispose())
                .await
                .is_err()
        {
            tracing::warn!("computer service dispose exceeded the shutdown budget");
        }
    }

    pub fn still_frame(&self, chat_id: &str) -> Option<ComputerStillFrame> {
        let task_live = self
            .engine_seams
            .sessions()
            .is_some_and(|sessions| sessions.computer_turn_live(chat_id));
        if !task_live && !self.settings().computer_control_enabled {
            return None;
        }
        let service = self.service()?;
        if !service.manager.holds_desktop(chat_id) {
            return None;
        }
        let frame = service.manager.latest_frame()?;
        Some(ComputerStillFrame {
            sequence: frame.sequence,
            timestamp_ms: frame.timestamp_ms,
            keyframe: frame.keyframe,
            data: BASE64.encode(&frame.data),
        })
    }
}

fn always_allowed_ids(settings: &ComputerSettings) -> impl Iterator<Item = &str> {
    settings
        .always_allowed_apps
        .iter()
        .map(|app| app.bundle_id.as_str())
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ChatChange {
    Removed(String),
    Restored(String),
    Titled(String, Option<String>),
}

fn chat_changes(
    known: &std::collections::HashMap<String, (bool, Option<String>)>,
    current: &std::collections::HashMap<String, (bool, Option<String>)>,
) -> Vec<ChatChange> {
    let mut changes = Vec::new();
    for (chat_id, (archived, _)) in known {
        if !*archived && current.get(chat_id).is_none_or(|(now, _)| *now) {
            changes.push(ChatChange::Removed(chat_id.clone()));
        }
    }
    for (chat_id, (archived, title)) in current {
        match known.get(chat_id) {
            Some((true, _)) if !*archived => changes.push(ChatChange::Restored(chat_id.clone())),
            Some((_, before)) if before != title => {
                changes.push(ChatChange::Titled(chat_id.clone(), title.clone()))
            }
            _ => {}
        }
    }
    changes
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use futures::FutureExt;
    use tokio::sync::oneshot;
    use tokio_util::sync::CancellationToken;
    use zeron_proto::PermissionMode;
    use zeron_proto::computer::{ComputerHostMethod, CuaReply};

    use super::*;
    use crate::computer::approval_gate::{AppGrant, ApprovalDecision, ApprovalRequest};
    use crate::computer::fake_backend::{FakeComputerBackend, FakeComputerBackendOptions};

    async fn next_service(
        watch: &mut watch::Receiver<Option<Arc<ComputerService>>>,
        supported: bool,
    ) -> Arc<ComputerService> {
        let found = tokio::time::timeout(
            Duration::from_secs(5),
            watch.wait_for(|service| service.as_ref().is_some_and(|s| s.supported == supported)),
        )
        .await
        .expect("service rebuilt in time")
        .expect("runtime alive");
        found.clone().expect("checked above")
    }

    async fn serve_probe(
        items: &mut BoxStream<'static, ComputerHostStreamItem>,
        runtime: &ComputerRuntime,
        token: &str,
    ) {
        let ComputerHostStreamItem::Request(request) = items.next().await.expect("probe item")
        else {
            panic!("expected the boot probe");
        };
        assert_eq!(request.call.method, ComputerHostMethod::Probe);
        let answered = runtime.respond(RespondComputerHostRequestParams {
            host_token: token.to_owned(),
            correlation_id: request.call.correlation_id,
            result: CuaReply {
                ok: true,
                ..CuaReply::default()
            },
        });
        assert_eq!(answered, Ok(true));
    }

    fn consent(gate: &ComputerApprovalGate, published: &Arc<AtomicUsize>) -> ApprovalRequest {
        let gate = gate.clone();
        let published = published.clone();
        ApprovalRequest {
            chat_id: "chat".into(),
            turn_id: Some("turn".into()),
            mode: PermissionMode::Ask,
            signal: CancellationToken::new(),
            publish: Arc::new(move |request_id, decision| {
                if decision.is_none() {
                    published.fetch_add(1, Ordering::SeqCst);
                    gate.respond("chat", &request_id, ApprovalDecision::Accept);
                }
                async { Ok(()) }.boxed()
            }),
            app: None,
        }
    }

    #[tokio::test]
    async fn a_host_registration_rebuilds_the_service_and_its_loss_revokes_consent() {
        let dir = tempfile::tempdir().unwrap();
        let runtime = ComputerRuntime::start(dir.path());
        let mut services = runtime.watch_service();
        let unhosted = next_service(&mut services, false).await;
        runtime.control_state().bump("chat");

        let mut first = runtime.register_host("sh.zeron.test".into());
        let Some(ComputerHostStreamItem::Registered { host_token }) = first.next().await else {
            panic!("the first item carries the host token");
        };
        assert_eq!(runtime.host_bundle_id().as_deref(), Some("sh.zeron.test"));
        serve_probe(&mut first, &runtime, &host_token).await;
        let hosted = next_service(&mut services, true).await;
        assert!(!Arc::ptr_eq(&hosted, &unhosted));
        assert_eq!(
            hosted.manager.control_state().generation("chat"),
            1,
            "the rebuilt manager shares the control state"
        );

        let mut second = runtime.register_host("sh.zeron.other".into());
        assert_eq!(
            second.next().await,
            Some(ComputerHostStreamItem::Refused {
                retry_after_ms: super::super::host_wire::HOST_BUSY_RETRY_MS
            })
        );
        assert_eq!(second.next().await, None);

        let published = Arc::new(AtomicUsize::new(0));
        let gate = runtime.approval_gate().clone();
        assert_eq!(
            gate.request_task(consent(&gate, &published)).await,
            Ok(true)
        );
        assert_eq!(
            gate.request_task(consent(&gate, &published)).await,
            Ok(true)
        );
        assert_eq!(published.load(Ordering::SeqCst), 1);

        drop(first);
        assert_eq!(runtime.host_bundle_id(), None);
        assert_eq!(
            gate.request_task(consent(&gate, &published)).await,
            Ok(true)
        );
        assert_eq!(
            published.load(Ordering::SeqCst),
            2,
            "host loss revoked the task grant"
        );
        next_service(&mut services, false).await;

        let mut again = runtime.register_host("sh.zeron.other".into());
        assert!(matches!(
            again.next().await,
            Some(ComputerHostStreamItem::Registered { .. })
        ));
        runtime.shutdown().await;
    }

    fn safari_consent(
        gate: &ComputerApprovalGate,
        published: &Arc<AtomicUsize>,
        turn: &str,
        decision: ApprovalDecision,
    ) -> ApprovalRequest {
        let answering = gate.clone();
        let counter = published.clone();
        ApprovalRequest {
            turn_id: Some(turn.into()),
            publish: Arc::new(move |request_id, resolved| {
                if resolved.is_none() {
                    counter.fetch_add(1, Ordering::SeqCst);
                    answering.respond("chat", &request_id, decision);
                }
                async { Ok(()) }.boxed()
            }),
            app: Some(AppGrant {
                key: "bundle:com.apple.Safari".into(),
                always: Some(ComputerAllowedApp {
                    bundle_id: "com.apple.Safari".into(),
                    name: Some("Safari".into()),
                }),
            }),
            ..consent(gate, published)
        }
    }

    #[tokio::test]
    async fn always_allow_is_saved_in_settings_and_removing_it_asks_again() {
        let dir = tempfile::tempdir().unwrap();
        let runtime = ComputerRuntime::start(dir.path());
        let gate = runtime.approval_gate().clone();
        let published = Arc::new(AtomicUsize::new(0));
        let always = ApprovalDecision::AcceptForSession;
        assert_eq!(
            gate.request_task(safari_consent(&gate, &published, "turn-1", always))
                .await,
            Ok(true)
        );
        let saved = vec![ComputerAllowedApp {
            bundle_id: "com.apple.Safari".into(),
            name: Some("Safari".into()),
        }];
        assert_eq!(runtime.settings().always_allowed_apps, saved);
        runtime.shutdown().await;

        let reopened = ComputerRuntime::start(dir.path());
        assert_eq!(reopened.settings().always_allowed_apps, saved);
        let gate = reopened.approval_gate().clone();
        let accept = ApprovalDecision::Accept;
        assert_eq!(
            gate.request_task(safari_consent(&gate, &published, "turn-2", accept))
                .await,
            Ok(true)
        );
        assert_eq!(
            published.load(Ordering::SeqCst),
            1,
            "no card for a saved app"
        );
        reopened
            .set_settings(ComputerSettings {
                always_allowed_apps: Vec::new(),
                ..reopened.settings()
            })
            .unwrap();
        assert_eq!(
            gate.request_task(safari_consent(&gate, &published, "turn-3", accept))
                .await,
            Ok(true)
        );
        assert_eq!(
            published.load(Ordering::SeqCst),
            2,
            "removal brings the card back"
        );
        reopened.shutdown().await;
    }

    #[tokio::test]
    async fn settings_default_off_round_trip_and_persist() {
        let dir = tempfile::tempdir().unwrap();
        let runtime = ComputerRuntime::start(dir.path());
        assert!(!runtime.settings().computer_control_enabled);
        assert!(!runtime.settings().browser_developer_access);
        let on = ComputerSettings {
            computer_control_enabled: true,
            browser_developer_access: true,
            agent_cursor_fill_color: "#a1b2c3".into(),
            ..ComputerSettings::default()
        };
        runtime.set_settings(on.clone()).unwrap();
        assert_eq!(runtime.settings(), on);
        runtime.shutdown().await;

        let reopened = ComputerRuntime::start(dir.path());
        assert_eq!(reopened.settings(), on);
        reopened.shutdown().await;
    }

    async fn faked(dir: &Path) -> (Arc<ComputerRuntime>, Arc<FakeComputerBackend>) {
        let runtime = ComputerRuntime::start(dir);
        next_service(&mut runtime.watch_service(), false).await;
        let fake = Arc::new(FakeComputerBackend::new(
            FakeComputerBackendOptions::default(),
        ));
        runtime.install_backend(fake.clone()).await;
        runtime
            .set_settings(ComputerSettings {
                computer_control_enabled: true,
                ..ComputerSettings::default()
            })
            .unwrap();
        (runtime, fake)
    }

    #[tokio::test]
    async fn a_still_frame_read_never_starts_a_capture() {
        let dir = tempfile::tempdir().unwrap();
        let (runtime, fake) = faked(dir.path()).await;
        assert!(runtime.still_frame("chat-a").is_none());
        assert!(fake.calls_for("attachStream").is_empty());
        runtime.shutdown().await;
    }

    #[tokio::test]
    async fn a_chat_on_the_desktop_gets_stills_until_it_releases_it() {
        let dir = tempfile::tempdir().unwrap();
        let (runtime, fake) = faked(dir.path()).await;
        let manager = runtime.service().unwrap().manager.clone();
        manager.flush_stream_transitions().await;
        assert!(
            fake.calls_for("attachStream").is_empty(),
            "nothing captures while no chat holds the desktop"
        );
        assert!(runtime.still_frame("chat-a").is_none());

        let (release, released) = oneshot::channel::<()>();
        let (entered_tx, entered) = oneshot::channel::<()>();
        let activity = {
            let manager = manager.clone();
            tokio::spawn(async move {
                manager
                    .with_agent_activity(
                        "chat-a",
                        move || async move {
                            let _ = entered_tx.send(());
                            let _ = released.await;
                            Ok(())
                        },
                        None,
                        Some("turn"),
                        None,
                    )
                    .await
            })
        };
        entered.await.unwrap();
        tokio::time::timeout(Duration::from_secs(5), async {
            while runtime.still_frame("chat-a").is_none() {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("the chat holding the desktop reads a still with no viewer attached");
        assert_eq!(fake.calls_for("attachStream").len(), 1);
        assert!(
            runtime.still_frame("chat-b").is_none(),
            "another chat never sees the desktop"
        );
        runtime.set_settings(ComputerSettings::default()).unwrap();
        assert!(
            runtime.still_frame("chat-a").is_none(),
            "Settings off with no live Computer turn"
        );
        runtime
            .set_settings(ComputerSettings {
                computer_control_enabled: true,
                ..ComputerSettings::default()
            })
            .unwrap();

        let _ = release.send(());
        activity.await.unwrap().unwrap();
        manager.flush_stream_transitions().await;
        assert!(
            runtime.still_frame("chat-a").is_none(),
            "a released desktop serves nothing"
        );
        assert!(manager.latest_frame().is_none());
        assert_eq!(
            fake.calls_for("detachStream").len(),
            1,
            "releasing the desktop stops the capture"
        );
        assert!(runtime.still_frame("chat-b").is_none());
        runtime.shutdown().await;
    }

    #[tokio::test]
    async fn shutdown_does_not_wait_out_a_hung_boot_and_nothing_publishes_after_it() {
        let dir = tempfile::tempdir().unwrap();
        let runtime = ComputerRuntime::start(dir.path());
        next_service(&mut runtime.watch_service(), false).await;
        let mut host = runtime.register_host("sh.zeron.test".into());
        let Some(ComputerHostStreamItem::Registered { host_token }) = host.next().await else {
            panic!("the first item carries the host token");
        };
        let Some(ComputerHostStreamItem::Request(probe)) = host.next().await else {
            panic!("the rebuild probes the host");
        };
        tokio::time::timeout(Duration::from_secs(5), runtime.shutdown())
            .await
            .expect("shutdown is bounded while a boot waits on the host");
        let _ = runtime.respond(RespondComputerHostRequestParams {
            host_token,
            correlation_id: probe.call.correlation_id,
            result: CuaReply {
                ok: true,
                ..CuaReply::default()
            },
        });
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert!(
            runtime.service().is_none(),
            "a boot finished after shutdown"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn concurrent_settings_writes_leave_memory_and_disk_in_agreement() {
        let dir = tempfile::tempdir().unwrap();
        let runtime = ComputerRuntime::start(dir.path());
        for round in 0..200 {
            let writers: Vec<_> = (0..4)
                .map(|writer| {
                    let runtime = runtime.clone();
                    std::thread::spawn(move || {
                        runtime.set_settings(ComputerSettings {
                            agent_cursor_fill_color: format!("#{round:02x}{writer:02x}00"),
                            ..ComputerSettings::default()
                        })
                    })
                })
                .collect();
            for writer in writers {
                writer.join().unwrap().expect("each write lands");
            }
            let disk: ComputerSettings = serde_json::from_str(
                &std::fs::read_to_string(dir.path().join(SETTINGS_FILE)).unwrap(),
            )
            .unwrap();
            assert_eq!(disk, runtime.settings(), "round {round}");
        }
        runtime.shutdown().await;
    }

    #[test]
    fn chat_changes_report_removal_restore_and_retitle_only() {
        let rows = |rows: &[(&str, bool, Option<&str>)]| {
            rows.iter()
                .map(|(id, archived, title)| {
                    (id.to_string(), (*archived, title.map(str::to_owned)))
                })
                .collect::<std::collections::HashMap<_, _>>()
        };
        let known = rows(&[
            ("archived-now", false, None),
            ("deleted", false, None),
            ("restored", true, None),
            ("retitled", false, Some("old")),
            ("gone-while-archived", true, None),
            ("unchanged", false, Some("same")),
        ]);
        let current = rows(&[
            ("archived-now", true, None),
            ("restored", false, None),
            ("retitled", false, Some("new")),
            ("unchanged", false, Some("same")),
            ("created", false, Some("fresh")),
        ]);
        let mut changes = chat_changes(&known, &current);
        changes.sort_by_key(|change| format!("{change:?}"));
        assert_eq!(
            changes,
            vec![
                ChatChange::Removed("archived-now".into()),
                ChatChange::Removed("deleted".into()),
                ChatChange::Restored("restored".into()),
                ChatChange::Titled("retitled".into(), Some("new".into())),
            ]
        );
    }
}
