//! Trusted-host interruption checkpoints for one browser mutation.
//!
//! The host owns admission and the operation's lifetime. Core only checks its
//! scoped callback before each CDP send and finishes owned input releases.
//! Ordinary browser reads do not install a scope and remain available.

use std::future::Future;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use serde_json::Value;

use super::cdp_ws::CdpConnection;

type Checkpoint = Arc<dyn Fn() -> anyhow::Result<()> + Send + Sync>;
type UncertainRelease = Arc<dyn Fn() + Send + Sync>;

struct InputControl {
    checkpoint: Checkpoint,
    mark_uncertain_release: UncertainRelease,
    uncertain_release: AtomicBool,
}

impl InputControl {
    fn unresolved_release(&self) {
        if !self.uncertain_release.swap(true, Ordering::SeqCst) {
            (self.mark_uncertain_release)();
        }
    }
}

tokio::task_local! {
    static INPUT_CONTROL: Arc<InputControl>;
}

/// The caller must retain its admitted operation lease until this scope ends.
/// A failed release callback must keep its cleanup barrier closed; an OS-level
/// keyboard release is not evidence that a CDP key or mouse button was released.
pub async fn scope<F: Future>(
    checkpoint: Checkpoint,
    mark_uncertain_release: UncertainRelease,
    future: F,
) -> F::Output {
    INPUT_CONTROL
        .scope(
            Arc::new(InputControl {
                checkpoint,
                mark_uncertain_release,
                uncertain_release: AtomicBool::new(false),
            }),
            future,
        )
        .await
}

#[derive(Debug)]
struct Interrupted(String);

impl std::fmt::Display for Interrupted {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "browser input interrupted before dispatch: {}",
            self.0
        )
    }
}

impl std::error::Error for Interrupted {}

pub(crate) fn interrupted_before_send(error: &anyhow::Error) -> bool {
    error.is::<Interrupted>()
}

/// Check immediately before a non-CDP actuator, after any async preflight.
pub(crate) fn checkpoint() -> anyhow::Result<()> {
    INPUT_CONTROL
        .try_with(|control| {
            (control.checkpoint)().map_err(|error| Interrupted(error.to_string()).into())
        })
        .unwrap_or(Ok(()))
}

/// Matching releases and focus cleanup remain legal after admission closes.
/// Called again after acquiring the writer so queued commands cannot skip Stop.
pub(crate) fn before_send(method: &str, params: &Value) -> anyhow::Result<()> {
    let cleanup = match method {
        "Input.dispatchKeyEvent" => params["type"] == "keyUp",
        "Input.dispatchMouseEvent" => params["type"] == "mouseReleased",
        "Emulation.setFocusEmulationEnabled" => params["enabled"] == false,
        "Browser.setDownloadBehavior" => {
            params["behavior"] == "default" && params["eventsEnabled"] == false
        }
        _ => false,
    };
    if cleanup {
        return Ok(());
    }
    checkpoint()
}

/// Dropping a future after down may leave browser input held. Preserve that
/// uncertainty even on task cancellation or panic instead of acknowledging a
/// clean drain when the enclosing operation lease drops.
struct PendingRelease {
    control: Option<Arc<InputControl>>,
}

impl PendingRelease {
    fn new() -> Self {
        Self {
            control: INPUT_CONTROL.try_with(Arc::clone).ok(),
        }
    }

    fn resolved(&mut self) {
        self.control = None;
    }
}

impl Drop for PendingRelease {
    fn drop(&mut self) {
        if let Some(control) = self.control.take() {
            control.unresolved_release();
        }
    }
}

/// Always await the matching up, including after an uncertain down reply or a
/// cancelled intermediate event. A definite pre-send interruption owns no down.
pub(crate) async fn with_release<F, R>(
    connection: &CdpConnection,
    session: &str,
    method: &str,
    down: Value,
    release: R,
    body: F,
) -> anyhow::Result<()>
where
    F: Future<Output = anyhow::Result<()>>,
    R: FnOnce() -> Value,
{
    let mut pending = PendingRelease::new();
    let down = connection.call(Some(session), method, down).await;
    if down.as_ref().is_err_and(interrupted_before_send) {
        pending.resolved();
        return down.map(|_| ());
    }
    let delivery = match down {
        Ok(_) => body.await,
        Err(error) => Err(error),
    };
    let released = connection.call(Some(session), method, release()).await;
    if released.is_ok() {
        pending.resolved();
    }
    delivery.and(released.map(|_| ()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::browser::mock_cdp::{MockCdpServer, MockReply};
    use serde_json::json;
    use std::sync::atomic::AtomicUsize;
    use std::sync::Mutex;

    #[tokio::test]
    async fn interrupted_download_can_restore_routing_but_cannot_enable_a_new_download() {
        let events = Arc::new(Mutex::new(Vec::new()));
        let server_events = events.clone();
        let server = MockCdpServer::start(Arc::new(move |call| {
            server_events.lock().unwrap().push(call.params.clone());
            MockReply::ok(json!({}))
        }))
        .await;
        let connection = CdpConnection::connect(&server.ws_url()).await.unwrap();
        let (check, uncertain) = callbacks(
            Arc::new(AtomicBool::new(true)),
            Arc::new(AtomicUsize::new(0)),
        );
        scope(check, uncertain, async {
            assert!(connection
                .call(
                    None,
                    "Browser.setDownloadBehavior",
                    json!({"behavior":"allowAndName","eventsEnabled":true})
                )
                .await
                .is_err());
            connection
                .call(
                    None,
                    "Browser.setDownloadBehavior",
                    json!({"behavior":"default","eventsEnabled":false}),
                )
                .await
                .unwrap();
        })
        .await;
        assert_eq!(
            *events.lock().unwrap(),
            vec![json!({"behavior":"default","eventsEnabled":false})]
        );
    }

    fn callbacks(
        stopped: Arc<AtomicBool>,
        unresolved: Arc<AtomicUsize>,
    ) -> (Checkpoint, UncertainRelease) {
        (
            Arc::new(move || {
                if stopped.load(Ordering::SeqCst) {
                    anyhow::bail!("test interruption");
                }
                Ok(())
            }),
            Arc::new(move || {
                unresolved.fetch_add(1, Ordering::SeqCst);
            }),
        )
    }

    #[tokio::test]
    async fn interruption_stops_characters_but_releases_the_key_and_preserves_reads() {
        let stopped = Arc::new(AtomicBool::new(false));
        let unresolved = Arc::new(AtomicUsize::new(0));
        let events = Arc::new(Mutex::new(Vec::new()));
        let server_events = events.clone();
        let server_stopped = stopped.clone();
        let server = MockCdpServer::start(Arc::new(move |call| {
            server_events
                .lock()
                .unwrap()
                .push((call.method.clone(), call.params["type"].clone()));
            if call.params["type"] == "keyDown" {
                server_stopped.store(true, Ordering::SeqCst);
            }
            MockReply::ok(json!({}))
        }))
        .await;
        let connection = CdpConnection::connect(&server.ws_url()).await.unwrap();
        let (check, uncertain) = callbacks(stopped.clone(), unresolved.clone());
        let result = scope(
            check,
            uncertain,
            with_release(
                &connection,
                "tab",
                "Input.dispatchKeyEvent",
                json!({"type":"keyDown","key":"a"}),
                || json!({"type":"keyUp","key":"a"}),
                async {
                    connection
                        .call(
                            Some("tab"),
                            "Input.dispatchKeyEvent",
                            json!({"type":"char","text":"a"}),
                        )
                        .await?;
                    Ok(())
                },
            ),
        )
        .await;
        assert!(interrupted_before_send(&result.unwrap_err()));
        assert_eq!(
            *events.lock().unwrap(),
            vec![
                ("Input.dispatchKeyEvent".to_owned(), json!("keyDown")),
                ("Input.dispatchKeyEvent".to_owned(), json!("keyUp")),
            ]
        );
        assert_eq!(unresolved.load(Ordering::SeqCst), 0);
        connection
            .call(Some("tab"), "Page.getFrameTree", json!({}))
            .await
            .unwrap();
        stopped.store(false, Ordering::SeqCst);
        let (check, uncertain) = callbacks(stopped, unresolved);
        scope(
            check,
            uncertain,
            connection.call(Some("tab"), "Input.insertText", json!({"text":"next"})),
        )
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn a_failed_matching_release_reports_uncertain_cleanup() {
        let unresolved = Arc::new(AtomicUsize::new(0));
        let server = MockCdpServer::start(Arc::new(|call| {
            if call.params["type"] == "keyUp" {
                MockReply::err(-1, "release unconfirmed")
            } else {
                MockReply::ok(json!({}))
            }
        }))
        .await;
        let connection = CdpConnection::connect(&server.ws_url()).await.unwrap();
        let (check, uncertain) = callbacks(Arc::new(AtomicBool::new(false)), unresolved.clone());
        let result = scope(
            check,
            uncertain,
            with_release(
                &connection,
                "tab",
                "Input.dispatchKeyEvent",
                json!({"type":"keyDown","key":"a"}),
                || json!({"type":"keyUp","key":"a"}),
                async { Ok(()) },
            ),
        )
        .await;
        assert!(result.is_err());
        assert_eq!(unresolved.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn interrupted_before_down_neither_releases_nor_poison_cleanup() {
        let sent = Arc::new(AtomicUsize::new(0));
        let server_sent = sent.clone();
        let server = MockCdpServer::start(Arc::new(move |_| {
            server_sent.fetch_add(1, Ordering::SeqCst);
            MockReply::ok(json!({}))
        }))
        .await;
        let connection = CdpConnection::connect(&server.ws_url()).await.unwrap();
        let unresolved = Arc::new(AtomicUsize::new(0));
        let (check, uncertain) = callbacks(Arc::new(AtomicBool::new(true)), unresolved.clone());
        let result = scope(
            check,
            uncertain,
            with_release(
                &connection,
                "tab",
                "Input.dispatchKeyEvent",
                json!({"type":"keyDown","key":"a"}),
                || json!({"type":"keyUp","key":"a"}),
                async { Ok(()) },
            ),
        )
        .await;
        assert!(interrupted_before_send(&result.unwrap_err()));
        assert_eq!(sent.load(Ordering::SeqCst), 0);
        assert_eq!(unresolved.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn dropping_a_dispatched_input_future_preserves_uncertain_cleanup() {
        let server = MockCdpServer::start(Arc::new(|_| MockReply::ok(json!({})))).await;
        let connection = CdpConnection::connect(&server.ws_url()).await.unwrap();
        let unresolved = Arc::new(AtomicUsize::new(0));
        let (check, uncertain) = callbacks(Arc::new(AtomicBool::new(false)), unresolved.clone());
        let (down, received) = tokio::sync::oneshot::channel();
        let task = tokio::spawn(async move {
            scope(
                check,
                uncertain,
                with_release(
                    &connection,
                    "tab",
                    "Input.dispatchKeyEvent",
                    json!({"type":"keyDown","key":"a"}),
                    || json!({"type":"keyUp","key":"a"}),
                    async {
                        down.send(()).unwrap();
                        std::future::pending::<anyhow::Result<()>>().await
                    },
                ),
            )
            .await
        });
        received.await.unwrap();
        task.abort();
        assert!(task.await.unwrap_err().is_cancelled());
        assert_eq!(unresolved.load(Ordering::SeqCst), 1);
    }
}
