use std::collections::HashSet;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use serde_json::json;
use zeron_proto::computer::ComputerHostStreamItem;
use zeron_rpc::{ComputerHost, RpcError, RpcSubscription, methods};

use crate::state::EngineHandle;

const RETRY_MIN: Duration = Duration::from_millis(500);
const RETRY_MAX: Duration = Duration::from_secs(30);

enum Served {
    Refused(Duration),
    Lost,
    Ended,
}

pub async fn run_host_registration(
    engine: EngineHandle,
    host: Arc<dyn ComputerHost>,
    bundle_id: String,
) {
    run_host_registration_with_reset(engine, host, bundle_id, None).await;
}

struct RegistrationReset(Option<Arc<dyn Fn() + Send + Sync>>);
impl Drop for RegistrationReset {
    fn drop(&mut self) {
        if let Some(reset) = &self.0 {
            reset();
        }
    }
}

async fn run_host_registration_with_reset(
    engine: EngineHandle,
    host: Arc<dyn ComputerHost>,
    bundle_id: String,
    reset: Option<Arc<dyn Fn() + Send + Sync>>,
) {
    let reset = RegistrationReset(reset);
    let mut backoff = RETRY_MIN;
    loop {
        let subscribed = engine
            .client()
            .subscribe_checked(
                methods::WATCH_COMPUTER_HOST_REQUESTS,
                json!({ "bundleId": bundle_id }),
            )
            .await;
        let (wait, served) = match subscribed {
            Ok(mut items) => match serve(&engine, &host, &mut items).await {
                Served::Refused(wait) => (wait, false),
                Served::Lost => {
                    backoff = RETRY_MIN;
                    (RETRY_MIN, true)
                }
                Served::Ended => (backoff, false),
            },
            Err(RpcError::UnknownMethod(_) | RpcError::Closed) => return,
            Err(error) => {
                tracing::warn!(target: "desktop_cua", %error, "computer host registration failed");
                (backoff, false)
            }
        };
        if served && let Some(reset) = &reset.0 {
            reset();
        }
        tokio::time::sleep(wait).await;
        backoff = (backoff * 2).min(RETRY_MAX);
    }
}

async fn serve(
    engine: &EngineHandle,
    host: &Arc<dyn ComputerHost>,
    items: &mut RpcSubscription,
) -> Served {
    let first = items
        .recv()
        .await
        .and_then(|value| serde_json::from_value(value).ok());
    match first {
        Some(ComputerHostStreamItem::Registered { .. }) => {}
        Some(ComputerHostStreamItem::Refused { retry_after_ms }) => {
            return Served::Refused(Duration::from_millis(retry_after_ms));
        }
        _ => return Served::Ended,
    }
    let in_flight = Arc::new(Mutex::new(HashSet::new()));
    while let Some(value) = items.recv().await {
        match serde_json::from_value(value) {
            Ok(ComputerHostStreamItem::Request(request)) => {
                let engine = engine.clone();
                let host = host.clone();
                let in_flight = in_flight.clone();
                let correlation_id = request.call.correlation_id.clone();
                in_flight
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .insert(correlation_id.clone());
                tokio::spawn(async move {
                    let result = host.request(request.call).await;
                    in_flight
                        .lock()
                        .unwrap_or_else(PoisonError::into_inner)
                        .remove(&correlation_id);
                    let answered = engine
                        .client()
                        .call(
                            methods::RESPOND_COMPUTER_HOST_REQUEST,
                            json!({
                                "hostToken": request.host_token,
                                "correlationId": correlation_id,
                                "result": result,
                            }),
                        )
                        .await;
                    if let Err(error) = answered {
                        tracing::debug!(target: "desktop_cua", %error, "computer host reply dropped");
                    }
                });
            }
            Ok(ComputerHostStreamItem::Cancel { correlation_id, .. }) => {
                host.cancel(&correlation_id);
            }
            Ok(_) => {}
            Err(error) => {
                tracing::warn!(target: "desktop_cua", %error, "unreadable computer host item");
            }
        }
    }
    let orphaned: Vec<String> = in_flight
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .drain()
        .collect();
    for correlation_id in orphaned {
        host.cancel(&correlation_id);
    }
    Served::Lost
}

#[cfg(target_os = "macos")]
pub const DESKTOP_HOST_RETIRE_BUDGET: Duration = Duration::from_secs(5);

#[cfg(target_os = "macos")]
pub struct DesktopHost {
    browser_admission: Arc<crate::browser::control_admission::Admission>,
    browser_sender: tokio::sync::mpsc::UnboundedSender<crate::browser::control_host::Message>,
    host: Arc<dyn ComputerHost>,
    driver: super::host::CuaDriverHost,
    bundle_id: String,
    engine: Arc<Mutex<Option<EngineHandle>>>,
    cursor_style: Arc<Mutex<Option<super::host::CuaCursorStyle>>>,
    _lifecycle: Option<super::native::desktop_events::DesktopLifecycleWatch>,
    _link: super::native::NativeHostLink,
}

#[cfg(target_os = "macos")]
impl gpui::Global for DesktopHost {}

#[cfg(target_os = "macos")]
pub fn install_desktop_host(cx: &mut gpui::App) {
    let Some(binary_path) = super::driver::bundled_binary() else {
        return;
    };
    let Some(bundle_id) = super::native::permissions::running_app_bundle_identifier() else {
        return;
    };
    let runtime = gpui_tokio::Tokio::handle(cx);
    let engine: Arc<Mutex<Option<EngineHandle>>> = Arc::default();
    let cursor_style: Arc<Mutex<Option<super::host::CuaCursorStyle>>> = Arc::default();
    let browser_sender = crate::browser::control_host::install(cx);
    let browser_admission = Arc::new(crate::browser::control_admission::Admission::default());
    let config = super::native::NativeHostConfig {
        binary_path,
        bundle_id: bundle_id.clone(),
        preview: super::preview::native_sink(cx),
        browser_admission: browser_admission.clone(),
        cursor_style: Some(Box::new({
            let cursor_style = cursor_style.clone();
            move || {
                cursor_style
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .clone()
            }
        })),
        on_takeover: super::preview::takeover_sink(cx),
    };
    match super::native::start_native_host(config) {
        Ok((driver, link)) => cx.set_global(DesktopHost {
            browser_admission: browser_admission.clone(),
            browser_sender: browser_sender.clone(),
            host: Arc::new(crate::browser::control_host::Host {
                driver: Arc::new(driver.clone()),
                admission: browser_admission,
                monitor: link.monitor().clone(),
                sender: browser_sender,
                engine: engine.clone(),
                pending: Mutex::default(),
                cancelled: Mutex::default(),
            }),
            driver,
            bundle_id,
            engine,
            cursor_style,
            _lifecycle: link.watch_desktop_lifecycle(runtime),
            _link: link,
        }),
        Err(error) => {
            tracing::warn!(target: "desktop_cua", %error, "computer desktop host failed to start");
        }
    }
}

#[cfg(target_os = "macos")]
pub fn spawn_host_registration(engine: EngineHandle, cx: &gpui::App) -> Option<gpui::Task<()>> {
    let desktop = cx.try_global::<DesktopHost>()?;
    *desktop
        .engine
        .lock()
        .unwrap_or_else(PoisonError::into_inner) = Some(engine.clone());
    let admission = desktop.browser_admission.clone();
    let sender = desktop.browser_sender.clone();
    let reset = Arc::new(move || {
        admission.stop(None);
        let _ = sender.send(crate::browser::control_host::Message::Revoke(None));
    });
    let registration = gpui_tokio::Tokio::spawn(
        cx,
        run_host_registration_with_reset(
            engine,
            desktop.host.clone(),
            desktop.bundle_id.clone(),
            Some(reset),
        ),
    );
    Some(cx.background_executor().spawn(async move {
        let _ = registration.await;
    }))
}

#[cfg(target_os = "macos")]
pub fn apply_cursor_colors(
    colors: Option<zeron_proto::computer::AgentCursorColors>,
    cx: &gpui::App,
) {
    let Some(desktop) = cx.try_global::<DesktopHost>() else {
        return;
    };
    let style = colors.map(|colors| super::host::CuaCursorStyle {
        fill: colors.fill,
        rim: colors.rim,
        shadow: None,
    });
    *desktop
        .cursor_style
        .lock()
        .unwrap_or_else(PoisonError::into_inner) = style.clone();
    drop(desktop.driver.set_cursor_style(style));
}

#[cfg(target_os = "macos")]
pub fn retire_desktop_host(cx: &mut gpui::App) -> Option<gpui::Task<()>> {
    cx.try_global::<DesktopHost>()?;
    super::native::target_ring::release_all();
    super::native::status_item::remove();
    let desktop = cx.remove_global::<DesktopHost>();
    let disposing = desktop.driver.dispose();
    let budget = cx.background_executor().timer(DESKTOP_HOST_RETIRE_BUDGET);
    Some(cx.background_executor().spawn(async move {
        match futures::future::select(disposing, Box::pin(budget)).await {
            futures::future::Either::Left((Err(error), _)) => {
                tracing::warn!(target: "desktop_cua", %error, "computer desktop host retirement failed");
            }
            futures::future::Either::Right(_) => {
                tracing::warn!(target: "desktop_cua", "computer desktop host retirement timed out");
            }
            futures::future::Either::Left((Ok(()), _)) => {}
        }
        drop(desktop);
    }))
}

#[cfg(test)]
mod tests {
    use async_trait::async_trait;
    use futures::StreamExt;
    use zeron_engine::{EngineCore, HarnessRegistry};
    use zeron_proto::HarnessId;
    use zeron_proto::computer::{
        ComputerHostCall, ComputerHostMethod, ComputerHostRequest, CuaReply,
    };
    use zeron_rpc::{RpcReply, RpcService};

    use super::*;

    #[derive(Default)]
    struct RecordingHost {
        requests: Mutex<Vec<ComputerHostMethod>>,
        cancels: Mutex<Vec<String>>,
    }

    #[async_trait]
    impl ComputerHost for RecordingHost {
        async fn request(&self, call: ComputerHostCall) -> CuaReply {
            self.requests.lock().unwrap().push(call.method);
            CuaReply {
                ok: true,
                ..CuaReply::default()
            }
        }

        fn cancel(&self, correlation_id: &str) {
            self.cancels.lock().unwrap().push(correlation_id.to_owned());
        }
    }

    async fn eventually(what: &str, mut done: impl FnMut() -> bool) {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
        while !done() {
            assert!(tokio::time::Instant::now() < deadline, "{what}");
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }

    fn supported(core: &EngineCore) -> bool {
        core.computer.service().is_some_and(|s| s.supported)
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn registers_with_the_engine_serves_it_and_the_next_ui_takes_over() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("device-id"), "device-mac").unwrap();
        let core = EngineCore::assemble(
            dir.path(),
            Arc::new(HarnessRegistry::new()),
            HarnessId::Mock,
            None,
        )
        .unwrap();
        let engine = EngineHandle::from_test_client(zeron_rpc::memory_client(core.rpc_service()));

        let first_host = Arc::new(RecordingHost::default());
        let first = tokio::spawn(run_host_registration(
            engine.clone(),
            first_host.clone(),
            "sh.zeron.test".into(),
        ));
        eventually("the engine never booted with the UI's host", || {
            supported(&core)
        })
        .await;
        assert_eq!(
            *first_host.requests.lock().unwrap(),
            vec![ComputerHostMethod::Probe]
        );

        let second_host = Arc::new(RecordingHost::default());
        let second = tokio::spawn(run_host_registration(
            engine.clone(),
            second_host.clone(),
            "sh.zeron.test".into(),
        ));
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert!(second_host.requests.lock().unwrap().is_empty());

        first.abort();
        eventually("the refused UI never re-registered", || {
            !second_host.requests.lock().unwrap().is_empty()
        })
        .await;
        eventually("the engine never rebuilt on the new host", || {
            supported(&core)
        })
        .await;
        second.abort();
        core.shutdown().await;
    }

    struct ScriptedEngine {
        watches: Mutex<usize>,
        replies: Mutex<Vec<serde_json::Value>>,
    }

    #[async_trait]
    impl RpcService for ScriptedEngine {
        async fn handle(
            &self,
            method: &str,
            params: serde_json::Value,
        ) -> Result<RpcReply, RpcError> {
            match method {
                methods::WATCH_COMPUTER_HOST_REQUESTS => {
                    let watch = {
                        let mut watches = self.watches.lock().unwrap();
                        *watches += 1;
                        *watches
                    };
                    let items = match watch {
                        1 => vec![ComputerHostStreamItem::Refused { retry_after_ms: 10 }],
                        2 => vec![
                            ComputerHostStreamItem::Registered {
                                host_token: "tok".into(),
                            },
                            ComputerHostStreamItem::Request(ComputerHostRequest {
                                host_token: "tok".into(),
                                call: ComputerHostCall {
                                    correlation_id: "c1".into(),
                                    method: ComputerHostMethod::Call,
                                    name: Some("click".into()),
                                    args: serde_json::Value::Null,
                                    task: None,
                                    delivery_mode: None,
                                    model_observation: None,
                                },
                            }),
                            ComputerHostStreamItem::Cancel {
                                host_token: "tok".into(),
                                correlation_id: "c9".into(),
                            },
                        ],
                        _ => {
                            return Ok(RpcReply::Stream(futures::stream::pending().boxed()));
                        }
                    };
                    let items: Vec<_> = items
                        .into_iter()
                        .map(|item| serde_json::to_value(item).unwrap())
                        .collect();
                    Ok(RpcReply::Stream(futures::stream::iter(items).boxed()))
                }
                methods::RESPOND_COMPUTER_HOST_REQUEST => {
                    self.replies.lock().unwrap().push(params);
                    RpcReply::value(&json!({}))
                }
                other => Err(RpcError::UnknownMethod(other.into())),
            }
        }
    }

    #[tokio::test]
    async fn honours_a_refusal_answers_requests_and_forwards_cancels() {
        let engine = Arc::new(ScriptedEngine {
            watches: Mutex::new(0),
            replies: Mutex::new(Vec::new()),
        });
        let host = Arc::new(RecordingHost::default());
        let registration = tokio::spawn(run_host_registration(
            EngineHandle::from_test_client(zeron_rpc::memory_client(engine.clone())),
            host.clone(),
            "sh.zeron.test".into(),
        ));
        eventually(
            "the registration never re-subscribed after its stream ended",
            || *engine.watches.lock().unwrap() >= 3,
        )
        .await;
        eventually("the host reply never reached the engine", || {
            !engine.replies.lock().unwrap().is_empty()
        })
        .await;
        let reply = engine.replies.lock().unwrap()[0].clone();
        assert_eq!(reply["hostToken"], "tok");
        assert_eq!(reply["correlationId"], "c1");
        assert_eq!(reply["result"]["ok"], true);
        assert_eq!(
            *host.requests.lock().unwrap(),
            vec![ComputerHostMethod::Call]
        );
        assert!(host.cancels.lock().unwrap().contains(&"c9".to_string()));
        registration.abort();
    }
}
