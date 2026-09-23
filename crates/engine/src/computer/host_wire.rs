use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, Weak};

use async_trait::async_trait;
use tokio::sync::{mpsc, oneshot};
use zeron_proto::computer::{
    ComputerHostCall, ComputerHostRequest, ComputerHostStreamItem, CuaEffect, CuaReply,
    RespondComputerHostRequestParams,
};
use zeron_rpc::ComputerHost;

pub const HOST_BUSY_RETRY_MS: u64 = 2_000;

const HOST_LOST_MESSAGE: &str = "host_lost: the Zeron desktop host disconnected before answering. Input already dispatched may have taken effect; do not replay.";
const NO_HOST_MESSAGE: &str = "host_lost: no Zeron desktop host is registered with this engine.";
const STALE_TOKEN_MESSAGE: &str = "This desktop host registration is no longer live.";

fn host_lost(effect: CuaEffect) -> CuaReply {
    let message = match effect {
        CuaEffect::NotDispatched => NO_HOST_MESSAGE,
        CuaEffect::DispatchedUnknown | CuaEffect::Verified => HOST_LOST_MESSAGE,
    };
    CuaReply {
        ok: false,
        error: Some(message.to_owned()),
        effect: Some(effect),
        ..CuaReply::default()
    }
}

struct LiveHost {
    id: u64,
    token: String,
    bundle_id: String,
    items: mpsc::UnboundedSender<ComputerHostStreamItem>,
    pending: HashMap<String, oneshot::Sender<CuaReply>>,
}

#[derive(Default)]
pub struct HostSlot {
    live: Mutex<Option<LiveHost>>,
    next_id: AtomicU64,
}

pub struct HostRegistration {
    slot: Arc<HostSlot>,
    id: u64,
    token: String,
    on_lost: Option<Box<dyn FnOnce() + Send + Sync>>,
}

pub struct HostRefused {
    pub retry_after_ms: u64,
}

impl HostSlot {
    fn live(&self) -> MutexGuard<'_, Option<LiveHost>> {
        self.live.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub fn live_id(&self) -> Option<u64> {
        self.live().as_ref().map(|live| live.id)
    }

    pub fn live_bundle_id(&self) -> Option<String> {
        self.live().as_ref().map(|live| live.bundle_id.clone())
    }

    pub fn register(
        self: &Arc<Self>,
        bundle_id: String,
        on_lost: Box<dyn FnOnce() + Send + Sync>,
    ) -> Result<
        (
            HostRegistration,
            mpsc::UnboundedReceiver<ComputerHostStreamItem>,
        ),
        HostRefused,
    > {
        let mut live = self.live();
        if live.is_some() {
            return Err(HostRefused {
                retry_after_ms: HOST_BUSY_RETRY_MS,
            });
        }
        let id = self.next_id.fetch_add(1, Ordering::Relaxed) + 1;
        let token = uuid::Uuid::new_v4().to_string();
        let (items, received) = mpsc::unbounded_channel();
        *live = Some(LiveHost {
            id,
            token: token.clone(),
            bundle_id,
            items,
            pending: HashMap::new(),
        });
        Ok((
            HostRegistration {
                slot: self.clone(),
                id,
                token,
                on_lost: Some(on_lost),
            },
            received,
        ))
    }

    pub fn respond(&self, reply: RespondComputerHostRequestParams) -> Result<bool, String> {
        let sender = {
            let mut live = self.live();
            let Some(live) = live.as_mut().filter(|live| live.token == reply.host_token) else {
                return Err(STALE_TOKEN_MESSAGE.to_owned());
            };
            live.pending.remove(&reply.correlation_id)
        };
        Ok(sender.is_some_and(|sender| sender.send(reply.result).is_ok()))
    }
}

impl HostRegistration {
    pub fn id(&self) -> u64 {
        self.id
    }

    pub fn token(&self) -> &str {
        &self.token
    }

    pub fn host(&self) -> WireComputerHost {
        WireComputerHost {
            slot: Arc::downgrade(&self.slot),
            id: self.id,
        }
    }
}

impl Drop for HostRegistration {
    fn drop(&mut self) {
        let lost = {
            let mut live = self.slot.live();
            match live.as_ref() {
                Some(current) if current.id == self.id => live.take(),
                _ => None,
            }
        };
        if let Some(lost) = lost {
            for (_, pending) in lost.pending {
                let _ = pending.send(host_lost(CuaEffect::DispatchedUnknown));
            }
        }
        if let Some(on_lost) = self.on_lost.take() {
            on_lost();
        }
    }
}

#[derive(Clone)]
pub struct WireComputerHost {
    slot: Weak<HostSlot>,
    id: u64,
}

struct PendingEntry {
    slot: Weak<HostSlot>,
    id: u64,
    correlation_id: String,
}

impl Drop for PendingEntry {
    fn drop(&mut self) {
        if let Some(slot) = self.slot.upgrade()
            && let Some(live) = slot.live().as_mut().filter(|live| live.id == self.id)
        {
            live.pending.remove(&self.correlation_id);
        }
    }
}

#[async_trait]
impl ComputerHost for WireComputerHost {
    async fn request(&self, call: ComputerHostCall) -> CuaReply {
        let correlation_id = call.correlation_id.clone();
        let (reply, answer) = oneshot::channel();
        {
            let Some(slot) = self.slot.upgrade() else {
                return host_lost(CuaEffect::NotDispatched);
            };
            let mut live = slot.live();
            let Some(live) = live.as_mut().filter(|live| live.id == self.id) else {
                return host_lost(CuaEffect::NotDispatched);
            };
            let item = ComputerHostStreamItem::Request(ComputerHostRequest {
                host_token: live.token.clone(),
                call,
            });
            if live.items.send(item).is_err() {
                return host_lost(CuaEffect::NotDispatched);
            }
            live.pending.insert(correlation_id.clone(), reply);
        }
        let _entry = PendingEntry {
            slot: self.slot.clone(),
            id: self.id,
            correlation_id,
        };
        answer
            .await
            .unwrap_or_else(|_| host_lost(CuaEffect::DispatchedUnknown))
    }

    fn cancel(&self, correlation_id: &str) {
        let Some(slot) = self.slot.upgrade() else {
            return;
        };
        if let Some(live) = slot.live().as_ref().filter(|live| live.id == self.id) {
            let _ = live.items.send(ComputerHostStreamItem::Cancel {
                host_token: live.token.clone(),
                correlation_id: correlation_id.to_owned(),
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use futures::FutureExt;
    use zeron_proto::computer::{ComputerAvailability, ComputerHostMethod};

    use super::*;
    use crate::computer::manager::ComputerManagerEnv;
    use crate::computer::service::{ComputerService, ComputerServiceOptions};

    fn call(id: &str, method: ComputerHostMethod) -> ComputerHostCall {
        ComputerHostCall {
            correlation_id: id.into(),
            method,
            name: None,
            args: serde_json::Value::Null,
            task: None,
            delivery_mode: None,
            model_observation: None,
        }
    }

    fn register(
        slot: &Arc<HostSlot>,
    ) -> (
        HostRegistration,
        mpsc::UnboundedReceiver<ComputerHostStreamItem>,
        Arc<AtomicU64>,
    ) {
        let lost = Arc::new(AtomicU64::new(0));
        let counter = lost.clone();
        let (registration, items) = slot
            .register(
                "sh.zeron.test".into(),
                Box::new(move || {
                    counter.fetch_add(1, Ordering::SeqCst);
                }),
            )
            .unwrap_or_else(|_| panic!("the slot is free"));
        (registration, items, lost)
    }

    fn requested(item: ComputerHostStreamItem) -> ComputerHostRequest {
        match item {
            ComputerHostStreamItem::Request(request) => request,
            other => panic!("expected a request item, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn one_live_registration_is_the_host_and_a_second_is_refused() {
        let slot = Arc::new(HostSlot::default());
        let (first, mut items, _) = register(&slot);
        assert_eq!(slot.live_id(), Some(first.id()));
        assert_eq!(slot.live_bundle_id().as_deref(), Some("sh.zeron.test"));
        let refused = match slot.register("sh.zeron.other".into(), Box::new(|| {})) {
            Ok(_) => panic!("a second host must be refused while the first lives"),
            Err(refused) => refused,
        };
        assert_eq!(refused.retry_after_ms, HOST_BUSY_RETRY_MS);

        let host = first.host();
        let pending =
            tokio::spawn(async move { host.request(call("c1", ComputerHostMethod::Probe)).await });
        let request = requested(items.recv().await.expect("request item"));
        assert_eq!(request.host_token, first.token());
        assert_eq!(request.call.correlation_id, "c1");
        let answered = slot.respond(RespondComputerHostRequestParams {
            host_token: first.token().to_owned(),
            correlation_id: "c1".into(),
            result: CuaReply {
                ok: true,
                ..CuaReply::default()
            },
        });
        assert_eq!(answered, Ok(true));
        assert!(pending.await.unwrap().ok);

        drop(first);
        assert_eq!(slot.live_id(), None);
        let (second, _items, _) = register(&slot);
        assert!(slot.live_id() == Some(second.id()));
    }

    #[tokio::test]
    async fn dropping_the_stream_fails_every_pending_call_within_one_tick() {
        let slot = Arc::new(HostSlot::default());
        let (registration, mut items, lost) = register(&slot);
        let host = registration.host();
        let mut calls: Vec<_> = (0..3)
            .map(|n| {
                let host = host.clone();
                async move {
                    host.request(call(&format!("c{n}"), ComputerHostMethod::Call))
                        .await
                }
                .boxed()
            })
            .collect();
        for call in &mut calls {
            assert!(call.now_or_never().is_none());
        }
        for _ in 0..3 {
            requested(items.recv().await.expect("request item"));
        }

        drop(registration);
        assert_eq!(slot.live_id(), None);
        assert_eq!(lost.load(Ordering::SeqCst), 1);
        for call in calls {
            let reply = call.now_or_never().expect("failed in the same tick");
            assert!(!reply.ok);
            assert_eq!(reply.effect, Some(CuaEffect::DispatchedUnknown));
            assert!(reply.error.unwrap().starts_with("host_lost"));
        }
        let after = host.request(call("late", ComputerHostMethod::Call)).await;
        assert_eq!(after.effect, Some(CuaEffect::NotDispatched));
    }

    #[tokio::test]
    async fn late_and_unknown_replies_are_no_ops_and_a_stale_token_is_refused() {
        let slot = Arc::new(HostSlot::default());
        let (registration, mut items, _) = register(&slot);
        let host = registration.host();
        let reply = |token: &str, id: &str| RespondComputerHostRequestParams {
            host_token: token.to_owned(),
            correlation_id: id.to_owned(),
            result: CuaReply::default(),
        };
        assert_eq!(
            slot.respond(reply(registration.token(), "never-sent")),
            Ok(false)
        );
        assert!(slot.respond(reply("forged", "never-sent")).is_err());

        let abandoned = host.request(call("c1", ComputerHostMethod::Call));
        let mut abandoned = Box::pin(abandoned);
        assert!((&mut abandoned).now_or_never().is_none());
        requested(items.recv().await.expect("request item"));
        drop(abandoned);
        assert_eq!(slot.respond(reply(registration.token(), "c1")), Ok(false));

        let token = registration.token().to_owned();
        drop(registration);
        assert!(slot.respond(reply(&token, "c1")).is_err());
    }

    #[tokio::test]
    async fn a_cancel_reaches_the_host_while_another_call_is_outstanding() {
        let slot = Arc::new(HostSlot::default());
        let (registration, mut items, _) = register(&slot);
        let host = registration.host();
        let outstanding = tokio::spawn({
            let host = host.clone();
            async move { host.request(call("keep", ComputerHostMethod::Call)).await }
        });
        let cancelled = tokio::spawn({
            let host = host.clone();
            async move { host.request(call("drop", ComputerHostMethod::Call)).await }
        });
        requested(items.recv().await.unwrap());
        requested(items.recv().await.unwrap());
        cancelled.abort();
        host.cancel("drop");
        assert_eq!(
            items.recv().await.unwrap(),
            ComputerHostStreamItem::Cancel {
                host_token: registration.token().to_owned(),
                correlation_id: "drop".into(),
            }
        );
        assert_eq!(
            slot.respond(RespondComputerHostRequestParams {
                host_token: registration.token().to_owned(),
                correlation_id: "keep".into(),
                result: CuaReply {
                    ok: true,
                    ..CuaReply::default()
                },
            }),
            Ok(true)
        );
        assert!(outstanding.await.unwrap().ok);
    }

    #[tokio::test(start_paused = true)]
    async fn a_hung_host_fails_the_call_at_its_deadline_and_is_told_to_cancel() {
        let slot = Arc::new(HostSlot::default());
        let (registration, mut items, _) = register(&slot);
        let started = tokio::time::Instant::now();
        let service = ComputerService::boot(ComputerServiceOptions {
            host: Some(Arc::new(registration.host())),
            env: ComputerManagerEnv::default(),
            ..ComputerServiceOptions::default()
        })
        .await;
        let waited = started.elapsed();
        assert!(
            waited >= Duration::from_millis(crate::computer::cua_backend::CUA_HOST_CALL_TIMEOUT_MS),
            "{waited:?}"
        );
        let ComputerAvailability::BackendUnavailable { message } = &service.availability else {
            panic!(
                "a hung host must not read as available: {:?}",
                service.availability
            );
        };
        assert!(message.contains("timed out"), "{message}");
        let probe = requested(items.recv().await.expect("probe item"));
        assert_eq!(probe.call.method, ComputerHostMethod::Probe);
        assert_eq!(
            items.recv().await.expect("cancel item"),
            ComputerHostStreamItem::Cancel {
                host_token: registration.token().to_owned(),
                correlation_id: probe.call.correlation_id.clone(),
            }
        );
        assert!(
            slot.live()
                .as_ref()
                .is_some_and(|live| live.pending.is_empty())
        );
        service.dispose().await;
    }
}
