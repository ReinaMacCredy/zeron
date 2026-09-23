use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use futures::StreamExt;
use futures::stream::BoxStream;
use tokio::sync::{oneshot, watch};
use zeron_proto::computer::{
    ComputerHostStreamItem, ComputerSettings, ComputerStillFrame, RespondComputerHostRequestParams,
};
use zeron_rpc::ComputerHost;

use super::approval_gate::ComputerApprovalGate;
use super::control_state::ComputerControlState;
use super::engine_seams::EngineComputerSeams;
use super::host_wire::{HostRegistration, HostSlot};
use super::manager::ComputerManagerSeams;
use super::service::{ComputerService, ComputerServiceOptions};

const SETTINGS_FILE: &str = "computer-settings.json";
const STILL_FRAME_WAIT: Duration = Duration::from_secs(2);

pub struct ComputerRuntime {
    data_dir: PathBuf,
    control_state: Arc<ComputerControlState>,
    approval_gate: ComputerApprovalGate,
    seams: Arc<dyn ComputerManagerSeams>,
    hosts: Arc<HostSlot>,
    service: watch::Sender<Option<Arc<ComputerService>>>,
    rebuilds: tokio::sync::Mutex<bool>,
    settings: Mutex<ComputerSettings>,
}

impl ComputerRuntime {
    pub fn start(data_dir: &Path) -> Arc<Self> {
        let settings = std::fs::read_to_string(data_dir.join(SETTINGS_FILE))
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default();
        let runtime = Arc::new(Self {
            data_dir: data_dir.to_path_buf(),
            control_state: Arc::default(),
            approval_gate: ComputerApprovalGate::default(),
            seams: Arc::new(EngineComputerSeams),
            hosts: Arc::default(),
            service: watch::channel(None).0,
            rebuilds: tokio::sync::Mutex::new(false),
            settings: Mutex::new(settings),
        });
        tokio::spawn(runtime.clone().rebuild(None));
        runtime
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
        self.service.send_replace(Some(Arc::new(service)));
    }

    pub async fn shutdown(&self) {
        let mut shut_down = self.rebuilds.lock().await;
        *shut_down = true;
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
}
