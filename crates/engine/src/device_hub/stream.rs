use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use futures::StreamExt;
use tokio::sync::watch;

use super::input::InputSocket;
use super::mjpeg::MjpegParser;

const RECONNECT_DELAY: Duration = Duration::from_secs(1);

pub(crate) struct Frame {
    pub seq: u64,
    pub jpeg: Vec<u8>,
}

pub(crate) type FrameSlot = watch::Sender<Option<Arc<Frame>>>;

struct Entry {
    generation: u64,
    frames: FrameSlot,
    subscribers: usize,
    reader: tokio::task::JoinHandle<()>,
    input: Option<InputSocket>,
}

impl Drop for Entry {
    fn drop(&mut self) {
        self.reader.abort();
    }
}

#[derive(Default)]
struct Entries {
    next_generation: u64,
    by_device: HashMap<String, Entry>,
}

#[derive(Default)]
pub(crate) struct DeviceStreams {
    entries: Mutex<Entries>,
}

pub(crate) struct FrameSubscription {
    pub frames: watch::Receiver<Option<Arc<Frame>>>,
    _release: Release,
}

struct Release {
    streams: Arc<DeviceStreams>,
    device_id: String,
    generation: u64,
}

impl Drop for Release {
    fn drop(&mut self) {
        let removed = {
            let mut entries = self.streams.lock();
            match entries.by_device.get_mut(&self.device_id) {
                Some(entry) if entry.generation != self.generation => None,
                Some(entry) if entry.subscribers > 1 => {
                    entry.subscribers -= 1;
                    None
                }
                Some(_) => entries.by_device.remove(&self.device_id),
                None => None,
            }
        };
        drop(removed);
    }
}

impl DeviceStreams {
    fn lock(&self) -> std::sync::MutexGuard<'_, Entries> {
        self.entries.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub(crate) fn subscribe(
        self: &Arc<Self>,
        device_id: &str,
        start: impl FnOnce(FrameSlot) -> tokio::task::JoinHandle<()>,
    ) -> FrameSubscription {
        let mut entries = self.lock();
        let Entries {
            next_generation,
            by_device,
        } = &mut *entries;
        let entry = by_device.entry(device_id.to_string()).or_insert_with(|| {
            let (frames, _) = watch::channel(None);
            let reader = start(frames.clone());
            *next_generation += 1;
            Entry {
                generation: *next_generation,
                frames,
                subscribers: 0,
                reader,
                input: None,
            }
        });
        entry.subscribers += 1;
        let mut frames = entry.frames.subscribe();
        frames.mark_changed();
        FrameSubscription {
            frames,
            _release: Release {
                streams: self.clone(),
                device_id: device_id.to_string(),
                generation: entry.generation,
            },
        }
    }

    pub(crate) fn has_input(&self, device_id: &str) -> bool {
        self.lock()
            .by_device
            .get(device_id)
            .is_some_and(|entry| entry.input.is_some())
    }

    pub(crate) fn set_input(&self, device_id: &str, input: InputSocket) {
        if let Some(entry) = self.lock().by_device.get_mut(device_id)
            && entry.input.is_none()
        {
            entry.input = Some(input);
        }
    }

    pub(crate) fn send_input(
        &self,
        device_id: &str,
        input: &zeron_proto::device_hub::DeviceInput,
    ) -> Result<(), String> {
        match self
            .lock()
            .by_device
            .get(device_id)
            .and_then(|entry| entry.input.as_ref())
        {
            Some(socket) => socket.send(input),
            None => Err("Input is not connected; open the device's video first".into()),
        }
    }

    pub(crate) fn clear(&self) {
        let entries = std::mem::take(&mut self.lock().by_device);
        drop(entries);
    }

    #[cfg(test)]
    pub(crate) fn subscribers(&self, device_id: &str) -> usize {
        self.lock()
            .by_device
            .get(device_id)
            .map_or(0, |entry| entry.subscribers)
    }
}

pub(crate) fn spawn_reader(
    http: reqwest::Client,
    url: String,
    frames: FrameSlot,
    on_first_frame: impl Fn() + Send + 'static,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut seq = 0u64;
        loop {
            if let Ok(response) = http
                .get(&url)
                .send()
                .await
                .and_then(|r| r.error_for_status())
            {
                let mut body = response.bytes_stream();
                let mut parser = MjpegParser::default();
                let mut first = true;
                while let Some(Ok(chunk)) = body.next().await {
                    for jpeg in parser.push(&chunk) {
                        seq += 1;
                        frames.send_replace(Some(Arc::new(Frame { seq, jpeg })));
                        if first {
                            first = false;
                            on_first_frame();
                        }
                    }
                }
            }
            tokio::time::sleep(RECONNECT_DELAY).await;
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    struct FakeHub {
        url: String,
        accepted: Arc<AtomicUsize>,
        open: Arc<AtomicUsize>,
    }

    async fn fake_hub(frames: usize, interval: Duration) -> FakeHub {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let accepted = Arc::new(AtomicUsize::new(0));
        let open = Arc::new(AtomicUsize::new(0));
        let (accepted_count, open_count) = (accepted.clone(), open.clone());
        tokio::spawn(async move {
            loop {
                let (mut socket, _) = listener.accept().await.unwrap();
                let (accepted_count, open_count) = (accepted_count.clone(), open_count.clone());
                tokio::spawn(async move {
                    let mut request = [0u8; 1024];
                    let n = socket.read(&mut request).await.unwrap_or(0);
                    // A running Zeron's preview discovery probes test listeners with `HEAD /`.
                    if !request[..n].starts_with(b"GET /vendor/serve-sim/helper/") {
                        return;
                    }
                    accepted_count.fetch_add(1, Ordering::SeqCst);
                    open_count.fetch_add(1, Ordering::SeqCst);
                    let head = "HTTP/1.1 200 OK\r\nContent-Type: multipart/x-mixed-replace; boundary=frame\r\nConnection: close\r\n\r\n";
                    let mut alive = socket.write_all(head.as_bytes()).await.is_ok();
                    for n in 0..frames {
                        if !alive {
                            break;
                        }
                        let body = format!("frame-{n}");
                        alive = socket
                            .write_all(&super::super::mjpeg::part(body.as_bytes()))
                            .await
                            .is_ok();
                        tokio::time::sleep(interval).await;
                    }
                    while alive {
                        tokio::time::sleep(Duration::from_millis(20)).await;
                        alive = socket.write_all(b"\r\n").await.is_ok();
                    }
                    open_count.fetch_sub(1, Ordering::SeqCst);
                });
            }
        });
        FakeHub {
            url: format!("http://127.0.0.1:{port}/vendor/serve-sim/helper/UDID/stream.mjpeg"),
            accepted,
            open,
        }
    }

    async fn eventually(mut done: impl FnMut() -> bool) {
        for _ in 0..300 {
            if done() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        panic!("condition never held");
    }

    #[tokio::test]
    async fn two_subscribers_share_one_hub_read_and_dropping_both_stops_it() {
        let hub = fake_hub(1, Duration::from_millis(1)).await;
        let streams = Arc::new(DeviceStreams::default());
        let http = reqwest::Client::new();
        let start = |frames: FrameSlot| spawn_reader(http.clone(), hub.url.clone(), frames, || {});
        let mut first = streams.subscribe("UDID", start);
        let second = streams.subscribe("UDID", |_| unreachable!("one reader per device"));
        assert_eq!(streams.subscribers("UDID"), 2);
        tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                first.frames.changed().await.unwrap();
                if first.frames.borrow().is_some() {
                    break;
                }
            }
        })
        .await
        .unwrap();
        assert_eq!(hub.accepted.load(Ordering::SeqCst), 1);
        drop(first);
        assert_eq!(streams.subscribers("UDID"), 1);
        assert_eq!(hub.open.load(Ordering::SeqCst), 1);
        drop(second);
        assert_eq!(streams.subscribers("UDID"), 0);
        eventually(|| hub.open.load(Ordering::SeqCst) == 0).await;
        tokio::time::sleep(Duration::from_millis(1200)).await;
        assert_eq!(hub.accepted.load(Ordering::SeqCst), 1, "no reconnect");
    }

    #[tokio::test]
    async fn a_release_from_before_a_clear_leaves_the_replacement_entry_alone() {
        let streams = Arc::new(DeviceStreams::default());
        let idle = |_: FrameSlot| tokio::spawn(std::future::pending::<()>());
        let stale = streams.subscribe("UDID", idle);
        streams.clear();
        let current = streams.subscribe("UDID", idle);
        drop(stale);
        assert_eq!(streams.subscribers("UDID"), 1);
        drop(current);
        assert_eq!(streams.subscribers("UDID"), 0);
    }

    #[tokio::test]
    async fn a_slow_subscriber_sees_only_the_newest_frame() {
        let hub = fake_hub(5, Duration::from_millis(5)).await;
        let streams = Arc::new(DeviceStreams::default());
        let http = reqwest::Client::new();
        let mut slow = streams.subscribe("UDID", |frames| {
            spawn_reader(http.clone(), hub.url.clone(), frames, || {})
        });
        tokio::time::sleep(Duration::from_millis(500)).await;
        slow.frames.changed().await.unwrap();
        let newest = slow.frames.borrow_and_update().clone().unwrap();
        assert_eq!(newest.seq, 5);
        assert_eq!(newest.jpeg, b"frame-4");
        assert!(!slow.frames.has_changed().unwrap());
    }
}
