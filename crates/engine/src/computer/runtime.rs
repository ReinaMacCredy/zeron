use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use futures::StreamExt;
use futures::stream::BoxStream;
use tokio::sync::{broadcast, oneshot, watch};
use zeron_proto::computer::{
    ComputerEvent, ComputerHostStreamItem, ComputerSettings, ComputerStillFrame,
    RespondComputerHostRequestParams,
};
use zeron_rpc::ComputerHost;

use super::activation::{TurnDispatch, resolve_turn_mode};
use super::approval_gate::ComputerApprovalGate;
use super::control_state::ComputerControlState;
use super::engine_seams::EngineComputerSeams;
use super::host_wire::{HostRegistration, HostSlot};
use super::manager::{ComputerManagerSeams, ComputerManagerSubscription};
use super::service::{ComputerService, ComputerServiceOptions};
use crate::sessions::SessionsEngine;

const SETTINGS_FILE: &str = "computer-settings.json";
const STILL_FRAME_WAIT: Duration = Duration::from_secs(2);
const EVENT_BUFFER: usize = 256;

pub const COMPUTER_ESCAPE_STOP_NOTICE: &str = "Computer use stopped: Escape was pressed";

pub struct ComputerRuntime {
    data_dir: PathBuf,
    control_state: Arc<ComputerControlState>,
    approval_gate: ComputerApprovalGate,
    engine_seams: Arc<EngineComputerSeams>,
    seams: Arc<dyn ComputerManagerSeams>,
    hosts: Arc<HostSlot>,
    service: watch::Sender<Option<Arc<ComputerService>>>,
    rebuilds: tokio::sync::Mutex<bool>,
    settings: Mutex<ComputerSettings>,
    events: broadcast::Sender<ComputerEvent>,
    forwarding: Mutex<Option<ComputerManagerSubscription>>,
    chat_watch: Mutex<Option<tokio::task::JoinHandle<()>>>,
}

impl ComputerRuntime {
    pub fn start(data_dir: &Path) -> Arc<Self> {
        let settings = std::fs::read_to_string(data_dir.join(SETTINGS_FILE))
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default();
        let approval_gate = ComputerApprovalGate::default();
        let engine_seams = Arc::new(EngineComputerSeams::new(approval_gate.clone()));
        let runtime = Arc::new(Self {
            data_dir: data_dir.to_path_buf(),
            control_state: Arc::default(),
            approval_gate,
            seams: engine_seams.clone(),
            engine_seams,
            hosts: Arc::default(),
            service: watch::channel(None).0,
            rebuilds: tokio::sync::Mutex::new(false),
            settings: Mutex::new(settings),
            events: broadcast::channel(EVENT_BUFFER).0,
            forwarding: Mutex::new(None),
            chat_watch: Mutex::new(None),
        });
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

    pub async fn stop_chat(&self, chat_id: &str, doc_generation: u64) {
        self.control_state.seed(chat_id, doc_generation);
        match self.service() {
            Some(service) => {
                if let Err(error) = service.manager.stop_thread(chat_id).await {
                    tracing::warn!(chat = %chat_id, %error, "computer stop failed");
                }
            }
            None => {
                let generation = self.control_state.bump(chat_id);
                self.seams.generation_bumped(chat_id, generation);
            }
        }
    }

    pub async fn emergency_stop(&self) -> Vec<String> {
        let Some(service) = self.service() else {
            return Vec::new();
        };
        let (stopped, result) = service.manager.emergency_stop().await;
        if let Err(error) = result {
            tracing::warn!(%error, "computer emergency stop relay failed");
        }
        stopped
    }

    pub fn turn_ended(&self, chat_id: &str, turn_id: &str) {
        self.approval_gate.cancel_thread(chat_id, Some(turn_id));
        let Some(service) = self.service() else {
            return;
        };
        let (chat_id, turn_id) = (chat_id.to_owned(), turn_id.to_owned());
        tokio::spawn(async move {
            if let Err(error) = service
                .manager
                .release_desktop_control(&chat_id, Some(&turn_id))
                .await
            {
                tracing::warn!(chat = %chat_id, %error, "computer turn-end release failed");
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
        let json = serde_json::to_string_pretty(&settings).map_err(|e| e.to_string())?;
        let path = self.data_dir.join(SETTINGS_FILE);
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, json)
            .and_then(|()| std::fs::rename(&tmp, &path))
            .map_err(|e| format!("computer settings save failed: {e}"))?;
        *self.settings.lock().unwrap_or_else(PoisonError::into_inner) = settings;
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
        let shut_down = self.rebuilds.lock().await;
        if *shut_down || self.hosts.live_id() != host.as_ref().map(|(id, _)| *id) {
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
    }

    pub async fn shutdown(&self) {
        let mut shut_down = self.rebuilds.lock().await;
        *shut_down = true;
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
        if let Some(service) = self.service.send_replace(None) {
            service.dispose().await;
        }
    }

    pub async fn still_frame(&self) -> Option<ComputerStillFrame> {
        let service = self.service()?;
        let manager = &service.manager;
        let frame = match manager.latest_frame() {
            Some(frame) => frame,
            None => {
                let (sender, received) = oneshot::channel();
                let sender = Mutex::new(Some(sender));
                let _subscription = manager.subscribe_frames(Arc::new(move |frame| {
                    if let Some(sender) =
                        sender.lock().unwrap_or_else(PoisonError::into_inner).take()
                    {
                        let _ = sender.send(frame.clone());
                    }
                }));
                tokio::time::timeout(STILL_FRAME_WAIT, received)
                    .await
                    .ok()?
                    .ok()?
            }
        };
        Some(ComputerStillFrame {
            sequence: frame.sequence,
            timestamp_ms: frame.timestamp_ms,
            keyframe: frame.keyframe,
            data: BASE64.encode(&frame.data),
        })
    }
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
    use tokio_util::sync::CancellationToken;
    use zeron_proto::PermissionMode;
    use zeron_proto::computer::{ComputerHostMethod, CuaReply};

    use super::*;
    use crate::computer::approval_gate::{ApprovalDecision, ApprovalRequest};

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

    #[tokio::test]
    async fn settings_default_off_round_trip_and_persist() {
        let dir = tempfile::tempdir().unwrap();
        let runtime = ComputerRuntime::start(dir.path());
        assert!(!runtime.settings().computer_control_enabled);
        let on = ComputerSettings {
            computer_control_enabled: true,
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
