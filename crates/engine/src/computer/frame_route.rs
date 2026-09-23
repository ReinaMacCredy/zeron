use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use futures::future::BoxFuture;
use serde_json::Value;

use super::utf8_truncation::utf16_len;

pub const COMPUTER_FRAME_RESYNC_MESSAGE: &str = "computer.frame.resync";
const MAX_CLIENT_MESSAGE_BYTES: usize = 1_024;

pub fn decode_resync_request(message: &[u8]) -> bool {
    let text = String::from_utf8_lossy(message);
    if utf16_len(&text) > MAX_CLIENT_MESSAGE_BYTES {
        return false;
    }
    serde_json::from_str::<Value>(&text).is_ok_and(|parsed| {
        parsed.is_object()
            && parsed.get("type").and_then(Value::as_str) == Some(COMPUTER_FRAME_RESYNC_MESSAGE)
    })
}

type SendFn = dyn Fn(Vec<u8>) -> BoxFuture<'static, Result<(), ()>> + Send + Sync;
type Listener = Arc<dyn Fn() + Send + Sync>;

#[derive(Default)]
struct SinkState {
    in_flight: AtomicUsize,
    failed: AtomicBool,
    next_listener: AtomicU64,
    listeners: Mutex<Vec<(u64, Listener)>>,
}

impl SinkState {
    fn settle(&self, bytes: usize, failed: bool) {
        if failed {
            self.failed.store(true, Ordering::SeqCst);
        }
        let _ = self
            .in_flight
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |current| {
                Some(current.saturating_sub(bytes))
            });
        let listeners: Vec<Listener> = self
            .listeners
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .map(|(_, listener)| listener.clone())
            .collect();
        for listener in listeners {
            listener();
        }
    }
}

#[derive(Clone)]
pub struct ComputerFrameSink {
    send: Arc<SendFn>,
    is_open: Arc<dyn Fn() -> bool + Send + Sync>,
    state: Arc<SinkState>,
}

pub struct WritableSubscription {
    id: u64,
    state: Arc<SinkState>,
}

impl Drop for WritableSubscription {
    fn drop(&mut self) {
        self.state
            .listeners
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .retain(|(id, _)| *id != self.id);
    }
}

impl ComputerFrameSink {
    pub fn new(
        send: impl Fn(Vec<u8>) -> BoxFuture<'static, Result<(), ()>> + Send + Sync + 'static,
        is_open: impl Fn() -> bool + Send + Sync + 'static,
    ) -> Self {
        Self {
            send: Arc::new(send),
            is_open: Arc::new(is_open),
            state: Arc::default(),
        }
    }

    pub fn send(&self, bytes: Vec<u8>) {
        let length = bytes.len();
        self.state.in_flight.fetch_add(length, Ordering::SeqCst);
        let write = (self.send)(bytes);
        let state = self.state.clone();
        tokio::spawn(async move {
            let result = write.await;
            state.settle(length, result.is_err());
        });
    }

    pub fn buffered_amount(&self) -> usize {
        self.state.in_flight.load(Ordering::SeqCst)
    }

    pub fn is_open(&self) -> bool {
        !self.state.failed.load(Ordering::SeqCst) && (self.is_open)()
    }

    pub fn on_writable(&self, listener: impl Fn() + Send + Sync + 'static) -> WritableSubscription {
        let id = self.state.next_listener.fetch_add(1, Ordering::SeqCst);
        self.state
            .listeners
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((id, Arc::new(listener)));
        WritableSubscription {
            id,
            state: self.state.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use futures::FutureExt;
    use serde_json::json;
    use tokio::sync::oneshot;

    use super::*;

    async fn settle_until(sink: &ComputerFrameSink, expected: usize) {
        for _ in 0..100 {
            if sink.buffered_amount() == expected {
                return;
            }
            tokio::task::yield_now().await;
        }
        panic!("buffered amount stayed at {}", sink.buffered_amount());
    }

    #[test]
    fn recognizes_text_and_binary_resync_messages() {
        let message = json!({ "type": COMPUTER_FRAME_RESYNC_MESSAGE }).to_string();
        assert!(decode_resync_request(message.as_bytes()));
    }

    #[test]
    fn ignores_malformed_unrelated_and_oversized_messages() {
        assert!(!decode_resync_request(b"not json"));
        assert!(!decode_resync_request(
            json!({ "type": "other" }).to_string().as_bytes()
        ));
        assert!(!decode_resync_request(
            json!(["resync"]).to_string().as_bytes()
        ));
        assert!(!decode_resync_request(b"null"));
        let oversized =
            json!({ "type": COMPUTER_FRAME_RESYNC_MESSAGE, "x": "x".repeat(2_000) }).to_string();
        assert!(!decode_resync_request(oversized.as_bytes()));
    }

    #[tokio::test]
    async fn accounts_for_bytes_until_a_write_settles() {
        let (settle, settled) = oneshot::channel::<()>();
        let settled = Mutex::new(Some(settled));
        let sink = ComputerFrameSink::new(
            move |_| {
                let wait = settled.lock().unwrap().take().unwrap();
                async move {
                    let _ = wait.await;
                    Ok(())
                }
                .boxed()
            },
            || true,
        );
        let notified = Arc::new(AtomicUsize::new(0));
        let counter = notified.clone();
        let _subscription = sink.on_writable(move || {
            counter.fetch_add(1, Ordering::SeqCst);
        });
        sink.send(vec![0; 128]);
        assert_eq!(sink.buffered_amount(), 128);
        settle.send(()).unwrap();
        settle_until(&sink, 0).await;
        assert_eq!(notified.load(Ordering::SeqCst), 1);
        assert!(sink.is_open());
    }

    #[tokio::test]
    async fn clears_the_backlog_when_a_write_fails() {
        let sink = ComputerFrameSink::new(|_| async { Err(()) }.boxed(), || true);
        sink.send(vec![0; 64]);
        settle_until(&sink, 0).await;
        assert!(!sink.is_open());
    }

    #[test]
    fn reports_a_closed_connection_to_the_shared_transport() {
        let open = Arc::new(AtomicBool::new(true));
        let reader = open.clone();
        let sink = ComputerFrameSink::new(
            |_| async { Ok(()) }.boxed(),
            move || reader.load(Ordering::SeqCst),
        );
        assert!(sink.is_open());
        open.store(false, Ordering::SeqCst);
        assert!(!sink.is_open());
    }
}
