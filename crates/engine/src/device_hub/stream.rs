use std::collections::{BTreeMap, HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use futures::{SinkExt, StreamExt};
use tokio::sync::{broadcast, watch};
use tokio_tungstenite::tungstenite::{self, Message};
use zeron_proto::device_hub::{
    AVCC_DELTA, AVCC_DESCRIPTION, AVCC_KEYFRAME, AVCC_SEED, AvccDemuxer, AvccEnvelope,
    DevicePlatform, DeviceStreamStatus, DeviceVideoCodec, StreamLinkStatus, annexb_to_avcc,
    semu_payload,
};

use super::input::{InputSocket, RESET_VIDEO};
use super::mjpeg::MjpegParser;

const RECONNECT_DELAY: Duration = Duration::from_secs(1);
const AVCC_QUEUE: usize = 256;
// A 300-frame GOP at 40 fps and the encoder's 6 Mbps is 5.6 MB; the cap doubles that.
const GOP_CACHE_BYTES: usize = 12 << 20;
// Base64 turns 3 MiB into 4 MiB per RPC item.
const AVCC_ITEM_BYTES: usize = 3 << 20;
// Base64 of 11 MiB stays under tungstenite's 16 MiB frame limit.
const AVCC_SENDABLE_BYTES: usize = 11 << 20;

pub(crate) struct Frame {
    pub seq: u64,
    pub jpeg: Vec<u8>,
}

pub(crate) type FrameSlot = watch::Sender<Option<Arc<Frame>>>;

#[derive(Default)]
struct AvccCache {
    description: Option<Arc<AvccEnvelope>>,
    seed: Option<Arc<AvccEnvelope>>,
    gop: Vec<Arc<AvccEnvelope>>,
    gop_bytes: usize,
    awaiting_keyframe: bool,
}

pub(crate) struct AvccFeed {
    cache: Mutex<AvccCache>,
    sender: broadcast::Sender<Arc<AvccEnvelope>>,
    keyframe_wanted: tokio::sync::Notify,
    pub(crate) platform: DevicePlatform,
}

impl AvccFeed {
    fn new(description: Option<Arc<AvccEnvelope>>, platform: DevicePlatform) -> Arc<Self> {
        Arc::new(Self {
            cache: Mutex::new(AvccCache {
                description,
                awaiting_keyframe: true,
                ..Default::default()
            }),
            sender: broadcast::channel(AVCC_QUEUE).0,
            keyframe_wanted: tokio::sync::Notify::new(),
            platform,
        })
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, AvccCache> {
        self.cache.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub(crate) fn connected(&self) {
        self.lock().awaiting_keyframe = true;
    }

    fn reset(&self) {
        *self.lock() = AvccCache {
            awaiting_keyframe: true,
            ..Default::default()
        };
    }

    pub(crate) fn push(&self, envelope: AvccEnvelope) {
        let mut cache = self.lock();
        let envelope = Arc::new(envelope);
        match envelope.tag {
            AVCC_DESCRIPTION => {
                cache.description = Some(envelope.clone());
                cache.gop.clear();
                cache.gop_bytes = 0;
                cache.awaiting_keyframe = true;
            }
            AVCC_SEED => cache.seed = Some(envelope.clone()),
            AVCC_KEYFRAME => {
                cache.awaiting_keyframe = false;
                cache.seed = None;
                cache.gop_bytes = envelope.payload.len();
                cache.gop = vec![envelope.clone()];
            }
            AVCC_DELTA if cache.awaiting_keyframe => return,
            AVCC_DELTA if !cache.gop.is_empty() => {
                cache.gop_bytes += envelope.payload.len();
                if cache.gop_bytes > GOP_CACHE_BYTES {
                    cache.gop.clear();
                } else {
                    cache.gop.push(envelope.clone());
                }
            }
            AVCC_DELTA => {}
            _ => return,
        }
        let _ = self.sender.send(envelope);
    }

    pub(crate) fn join(
        &self,
    ) -> (
        Vec<Arc<AvccEnvelope>>,
        broadcast::Receiver<Arc<AvccEnvelope>>,
    ) {
        let cache = self.lock();
        let mut replay: Vec<_> = cache.description.iter().cloned().collect();
        if cache.gop.is_empty() {
            replay.extend(cache.seed.clone());
        } else {
            replay.extend(cache.gop.iter().cloned());
        }
        (replay, self.sender.subscribe())
    }

    fn description(&self) -> Option<Arc<AvccEnvelope>> {
        self.lock().description.clone()
    }
}

struct Feed<S> {
    sink: S,
    subscribers: usize,
    reader: tokio::task::JoinHandle<()>,
}

impl<S> Drop for Feed<S> {
    fn drop(&mut self) {
        self.reader.abort();
    }
}

impl<S> Feed<S> {
    fn relink(
        &mut self,
        reader: Option<tokio::task::JoinHandle<()>>,
    ) -> Option<tokio::task::JoinHandle<()>> {
        self.reader.abort();
        reader.map(|reader| std::mem::replace(&mut self.reader, reader))
    }
}

struct Entry {
    generation: u64,
    link: u64,
    mjpeg: Option<Feed<FrameSlot>>,
    avcc: BTreeMap<Option<u32>, Feed<Arc<AvccFeed>>>,
    input: Option<InputSocket>,
    video_status: StreamLinkStatus,
    avcc_status: BTreeMap<Option<u32>, StreamLinkStatus>,
    input_status: StreamLinkStatus,
}

impl Entry {
    fn subscribers(&self) -> usize {
        self.mjpeg.as_ref().map_or(0, |feed| feed.subscribers)
            + self
                .avcc
                .values()
                .map(|feed| feed.subscribers)
                .sum::<usize>()
    }
}

#[derive(Default)]
struct Entries {
    next_generation: u64,
    by_device: HashMap<String, Entry>,
    descriptions: HashMap<(String, Option<u32>), Arc<AvccEnvelope>>,
}

impl Entries {
    fn entry(&mut self, device_id: &str) -> &mut Entry {
        let Entries {
            next_generation,
            by_device,
            ..
        } = self;
        by_device.entry(device_id.to_string()).or_insert_with(|| {
            *next_generation += 1;
            Entry {
                generation: *next_generation,
                link: *next_generation,
                mjpeg: None,
                avcc: BTreeMap::new(),
                input: None,
                video_status: StreamLinkStatus::Idle,
                avcc_status: BTreeMap::new(),
                input_status: StreamLinkStatus::Idle,
            }
        })
    }
}

type StatusSink = Box<dyn Fn(BTreeMap<String, DeviceStreamStatus>) + Send + Sync>;

#[derive(Default)]
pub(crate) struct DeviceStreams {
    entries: Mutex<Entries>,
    sink: Option<StatusSink>,
}

pub(crate) struct FrameSubscription {
    pub frames: watch::Receiver<Option<Arc<Frame>>>,
    _release: Release,
}

pub(crate) struct AvccSubscription {
    pub feed: Arc<AvccFeed>,
    _release: Release,
}

struct Release {
    streams: Arc<DeviceStreams>,
    device_id: String,
    generation: u64,
    codec: DeviceVideoCodec,
    panel: Option<u32>,
}

impl Drop for Release {
    fn drop(&mut self) {
        let removed = {
            let mut entries = self.streams.lock();
            let Entries {
                by_device,
                descriptions,
                ..
            } = &mut *entries;
            let mut removed = (None, None, None);
            if let Some(entry) = by_device
                .get_mut(&self.device_id)
                .filter(|entry| entry.generation == self.generation)
            {
                match self.codec {
                    DeviceVideoCodec::Mjpeg => {
                        if let Some(feed) = &mut entry.mjpeg {
                            feed.subscribers -= 1;
                        }
                        if entry
                            .mjpeg
                            .as_ref()
                            .is_some_and(|feed| feed.subscribers == 0)
                        {
                            removed.0 = entry.mjpeg.take();
                            entry.video_status = StreamLinkStatus::Idle;
                        }
                    }
                    DeviceVideoCodec::H264 => {
                        if let Some(feed) = entry.avcc.get_mut(&self.panel) {
                            feed.subscribers -= 1;
                        }
                        if entry
                            .avcc
                            .get(&self.panel)
                            .is_some_and(|feed| feed.subscribers == 0)
                        {
                            let feed = entry.avcc.remove(&self.panel);
                            if let Some(description) =
                                feed.as_ref().and_then(|feed| feed.sink.description())
                            {
                                descriptions
                                    .insert((self.device_id.clone(), self.panel), description);
                            }
                            removed.1 = feed;
                            entry.avcc_status.remove(&self.panel);
                        }
                    }
                }
                if entry.subscribers() == 0 {
                    removed.2 = by_device.remove(&self.device_id);
                }
            }
            self.streams.publish(&entries);
            removed
        };
        drop(removed);
    }
}

impl DeviceStreams {
    pub(crate) fn with_sink(
        sink: impl Fn(BTreeMap<String, DeviceStreamStatus>) + Send + Sync + 'static,
    ) -> Self {
        Self {
            entries: Mutex::default(),
            sink: Some(Box::new(sink)),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Entries> {
        self.entries.lock().unwrap_or_else(PoisonError::into_inner)
    }

    // Called with the entries lock held so concurrent changes publish in order.
    fn publish(&self, entries: &Entries) {
        if let Some(sink) = &self.sink {
            sink(
                entries
                    .by_device
                    .iter()
                    .map(|(id, entry)| {
                        let status = DeviceStreamStatus {
                            video: entry.video_status.clone(),
                            avcc: entry.avcc_status.get(&None).cloned().unwrap_or_default(),
                            input: entry.input_status.clone(),
                            subscribers: entry.subscribers() as u32,
                            panels: entry
                                .avcc_status
                                .iter()
                                .filter_map(|(panel, status)| Some(((*panel)?, status.clone())))
                                .collect(),
                        };
                        (id.clone(), status)
                    })
                    .collect(),
            );
        }
    }

    pub(crate) fn subscribe(
        self: &Arc<Self>,
        device_id: &str,
        start: impl FnOnce(FrameSlot, u64) -> tokio::task::JoinHandle<()>,
    ) -> FrameSubscription {
        let mut entries = self.lock();
        let entry = entries.entry(device_id);
        let link = entry.link;
        if entry.mjpeg.is_none() {
            entry.video_status = StreamLinkStatus::Connecting;
        }
        let feed = entry.mjpeg.get_or_insert_with(|| {
            let (frames, _) = watch::channel(None);
            Feed {
                reader: start(frames.clone(), link),
                sink: frames,
                subscribers: 0,
            }
        });
        feed.subscribers += 1;
        let mut frames = feed.sink.subscribe();
        frames.mark_changed();
        let generation = entry.generation;
        self.publish(&entries);
        FrameSubscription {
            frames,
            _release: self.release(device_id, generation, DeviceVideoCodec::Mjpeg, None),
        }
    }

    pub(crate) fn subscribe_avcc(
        self: &Arc<Self>,
        device_id: &str,
        panel: Option<u32>,
        platform: DevicePlatform,
        start: impl FnOnce(Arc<AvccFeed>, u64) -> tokio::task::JoinHandle<()>,
    ) -> AvccSubscription {
        let mut entries = self.lock();
        let cached = entries
            .descriptions
            .get(&(device_id.to_string(), panel))
            .cloned();
        let entry = entries.entry(device_id);
        let link = entry.link;
        if !entry.avcc.contains_key(&panel) {
            entry
                .avcc_status
                .insert(panel, StreamLinkStatus::Connecting);
        }
        let feed = entry.avcc.entry(panel).or_insert_with(|| {
            let sink = AvccFeed::new(cached, platform);
            Feed {
                reader: start(sink.clone(), link),
                sink,
                subscribers: 0,
            }
        });
        feed.subscribers += 1;
        if feed.subscribers > 1 && feed.sink.lock().gop.is_empty() {
            feed.sink.keyframe_wanted.notify_one();
        }
        let feed = feed.sink.clone();
        let generation = entry.generation;
        self.publish(&entries);
        AvccSubscription {
            feed,
            _release: self.release(device_id, generation, DeviceVideoCodec::H264, panel),
        }
    }

    fn release(
        self: &Arc<Self>,
        device_id: &str,
        generation: u64,
        codec: DeviceVideoCodec,
        panel: Option<u32>,
    ) -> Release {
        Release {
            streams: self.clone(),
            device_id: device_id.to_string(),
            generation,
            codec,
            panel,
        }
    }

    pub(crate) fn set_status(
        &self,
        device_id: &str,
        link: u64,
        video: Option<(DeviceVideoCodec, StreamLinkStatus)>,
        input: Option<StreamLinkStatus>,
    ) {
        let mut entries = self.lock();
        let Some(entry) = entries
            .by_device
            .get_mut(device_id)
            .filter(|entry| entry.link == link)
        else {
            return;
        };
        let snapshot = |entry: &Entry| {
            (
                entry.video_status.clone(),
                entry.avcc_status.clone(),
                entry.input_status.clone(),
            )
        };
        let before = snapshot(entry);
        match video {
            Some((DeviceVideoCodec::Mjpeg, status)) => entry.video_status = status,
            Some((DeviceVideoCodec::H264, status)) => {
                entry.avcc_status.insert(None, status);
            }
            None => {}
        }
        if let Some(input) = input {
            entry.input_status = input;
        }
        if before != snapshot(entry) {
            self.publish(&entries);
        }
    }

    pub(crate) fn set_panel_status(
        &self,
        device_id: &str,
        link: u64,
        panel: u32,
        status: StreamLinkStatus,
    ) {
        let mut entries = self.lock();
        let Some(entry) = entries
            .by_device
            .get_mut(device_id)
            .filter(|entry| entry.link == link && entry.avcc.contains_key(&Some(panel)))
        else {
            return;
        };
        if entry.avcc_status.get(&Some(panel)) != Some(&status) {
            entry.avcc_status.insert(Some(panel), status);
            self.publish(&entries);
        }
    }

    pub(crate) fn has_input(&self, device_id: &str) -> bool {
        self.lock()
            .by_device
            .get(device_id)
            .is_some_and(|entry| entry.input.is_some())
    }

    pub(crate) fn set_input(&self, device_id: &str, link: u64, input: InputSocket) {
        if let Some(entry) = self.lock().by_device.get_mut(device_id)
            && entry.link == link
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

    pub(crate) fn hinge(
        &self,
        device_id: &str,
        command: zeron_proto::device_hub::HingeCommand,
    ) -> Result<tokio::sync::oneshot::Receiver<Result<(), String>>, String> {
        match self
            .lock()
            .by_device
            .get(device_id)
            .and_then(|entry| entry.input.as_ref())
        {
            Some(socket) => socket.hinge(command),
            None => Err("Input is not connected; open the device's video first".into()),
        }
    }

    pub(crate) fn device_ids(&self, scope: &dyn Fn(&str) -> bool) -> Vec<String> {
        self.lock()
            .by_device
            .keys()
            .filter(|key| scope(key))
            .cloned()
            .collect()
    }

    pub(crate) fn pause(&self, scope: &dyn Fn(&str) -> bool) {
        self.relink(scope, &|_, _, _| None, &|_, _, _, _| None);
    }

    pub(crate) fn resume(
        &self,
        scope: &dyn Fn(&str) -> bool,
        mjpeg: impl Fn(&str, FrameSlot, u64) -> tokio::task::JoinHandle<()>,
        avcc: impl Fn(&str, Option<u32>, Arc<AvccFeed>, u64) -> tokio::task::JoinHandle<()>,
    ) {
        self.relink(
            scope,
            &|device_id, frames, link| Some(mjpeg(device_id, frames, link)),
            &|device_id, panel, feed, link| Some(avcc(device_id, panel, feed, link)),
        );
    }

    fn relink(
        &self,
        scope: &dyn Fn(&str) -> bool,
        mjpeg: &dyn Fn(&str, FrameSlot, u64) -> Option<tokio::task::JoinHandle<()>>,
        avcc: &dyn Fn(&str, Option<u32>, Arc<AvccFeed>, u64) -> Option<tokio::task::JoinHandle<()>>,
    ) {
        let (retired, inputs) = {
            let mut entries = self.lock();
            let Entries {
                next_generation,
                by_device,
                descriptions,
            } = &mut *entries;
            descriptions.retain(|(key, _), _| !scope(key));
            let mut retired = Vec::new();
            let mut inputs = Vec::new();
            for (device_id, entry) in by_device.iter_mut().filter(|(key, _)| scope(key)) {
                *next_generation += 1;
                let link = *next_generation;
                entry.link = link;
                if let Some(feed) = &mut entry.mjpeg {
                    retired.extend(feed.relink(mjpeg(device_id, feed.sink.clone(), link)));
                    entry.video_status = StreamLinkStatus::Connecting;
                }
                for (panel, feed) in &mut entry.avcc {
                    feed.sink.reset();
                    retired.extend(feed.relink(avcc(device_id, *panel, feed.sink.clone(), link)));
                    entry
                        .avcc_status
                        .insert(*panel, StreamLinkStatus::Connecting);
                }
                inputs.extend(entry.input.take());
                entry.input_status = StreamLinkStatus::Idle;
            }
            self.publish(&entries);
            (retired, inputs)
        };
        drop((retired, inputs));
    }

    pub(crate) fn clear(&self, scope: &dyn Fn(&str) -> bool) {
        let cleared = {
            let mut entries = self.lock();
            entries.descriptions.retain(|(key, _), _| !scope(key));
            let (cleared, kept) = std::mem::take(&mut entries.by_device)
                .into_iter()
                .partition(|(key, _)| scope(key));
            entries.by_device = kept;
            self.publish(&entries);
            cleared
        };
        drop(cleared);
    }

    #[cfg(test)]
    pub(crate) fn subscribers(&self, device_id: &str) -> usize {
        self.lock()
            .by_device
            .get(device_id)
            .map_or(0, Entry::subscribers)
    }
}

pub(crate) fn avcc_batches(
    subscription: AvccSubscription,
) -> impl futures::Stream<Item = Vec<u8>> + Send + 'static {
    let (replay, receiver) = subscription.feed.join();
    futures::stream::unfold(
        (subscription, receiver, VecDeque::from(replay)),
        |(subscription, mut receiver, mut batch)| async move {
            use broadcast::error::{RecvError, TryRecvError};
            loop {
                while batch.is_empty() {
                    match receiver.recv().await {
                        Ok(envelope) => batch.push_back(envelope),
                        Err(RecvError::Lagged(_)) => {
                            let (replay, live) = subscription.feed.join();
                            (batch, receiver) = (replay.into(), live);
                        }
                        Err(RecvError::Closed) => return None,
                    }
                    loop {
                        match receiver.try_recv() {
                            Ok(envelope) => batch.push_back(envelope),
                            Err(TryRecvError::Lagged(_)) => {
                                let (replay, live) = subscription.feed.join();
                                (batch, receiver) = (replay.into(), live);
                            }
                            Err(_) => break,
                        }
                    }
                }
                let bytes = pack(&mut batch, &subscription.feed);
                if !bytes.is_empty() {
                    return Some((bytes, (subscription, receiver, batch)));
                }
            }
        },
    )
}

fn pack(batch: &mut VecDeque<Arc<AvccEnvelope>>, feed: &AvccFeed) -> Vec<u8> {
    let sent = |envelope: &AvccEnvelope| envelope.payload.len() + 5;
    let mut bytes = Vec::new();
    while let Some(envelope) = batch.front() {
        if !bytes.is_empty() && bytes.len() + sent(envelope) > AVCC_ITEM_BYTES {
            break;
        }
        let envelope = batch.pop_front().expect("front exists");
        if sent(&envelope) <= AVCC_SENDABLE_BYTES {
            envelope.encode_into(&mut bytes);
        } else if let Some(description) = feed
            .description()
            .filter(|description| sent(description) <= AVCC_SENDABLE_BYTES)
        {
            // The description re-arms the view's keyframe gate, so it skips to the next GOP.
            description.encode_into(&mut bytes);
        }
    }
    bytes
}

pub(crate) fn spawn_reader(
    http: reqwest::Client,
    url: String,
    frames: FrameSlot,
    on_status: impl Fn(StreamLinkStatus) + Send + 'static,
    on_first_frame: impl Fn() + Send + 'static,
) -> tokio::task::JoinHandle<()> {
    let mut seq = frames.borrow().as_ref().map_or(0, |frame| frame.seq);
    spawn_link(
        http,
        url,
        on_status,
        on_first_frame,
        MjpegParser::default,
        move |parser, chunk| {
            let jpegs = parser.push(chunk);
            let produced = !jpegs.is_empty();
            for jpeg in jpegs {
                seq += 1;
                frames.send_replace(Some(Arc::new(Frame { seq, jpeg })));
            }
            Some(produced)
        },
    )
}

pub(crate) fn spawn_avcc_reader(
    http: reqwest::Client,
    url: String,
    feed: Arc<AvccFeed>,
    on_status: impl Fn(StreamLinkStatus) + Send + 'static,
    on_first_frame: impl Fn() + Send + 'static,
) -> tokio::task::JoinHandle<()> {
    let connected = feed.clone();
    spawn_link(
        http,
        url,
        on_status,
        on_first_frame,
        move || {
            connected.connected();
            AvccDemuxer::default()
        },
        move |demuxer, chunk| {
            let envelopes = demuxer.push(chunk)?;
            let produced = !envelopes.is_empty();
            for envelope in envelopes {
                feed.push(envelope);
            }
            Some(produced)
        },
    )
}

fn video_session(text: &str) -> bool {
    serde_json::from_str::<serde_json::Value>(text)
        .is_ok_and(|value| value["type"] == "video-session")
}

pub(crate) fn spawn_semu_reader(
    url: String,
    feed: Arc<AvccFeed>,
    mut controls: tokio::sync::mpsc::UnboundedReceiver<Message>,
    live: Arc<AtomicBool>,
    on_status: impl Fn(StreamLinkStatus) + Send + 'static,
    on_first_frame: impl Fn() + Send + 'static,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let reset = || Message::Text(RESET_VIDEO.into());
        loop {
            on_status(StreamLinkStatus::Connecting);
            let detail = match tokio_tungstenite::connect_async(url.as_str()).await {
                Err(tungstenite::Error::Http(response)) => {
                    format!("HTTP {}", response.status().as_u16())
                }
                Err(_) => "could not connect".to_string(),
                Ok((socket, _)) => {
                    let (mut sink, mut stream) = socket.split();
                    feed.connected();
                    while controls.try_recv().is_ok() {}
                    let mut first = true;
                    let detail = if sink.send(reset()).await.is_err() {
                        "the video stream ended"
                    } else {
                        live.store(true, Ordering::SeqCst);
                        loop {
                            let outgoing = tokio::select! {
                                control = controls.recv() => match control {
                                    Some(control) => control,
                                    None => return,
                                },
                                _ = feed.keyframe_wanted.notified() => reset(),
                                incoming = stream.next() => match incoming {
                                    Some(Ok(Message::Binary(bytes))) => {
                                        let Some((key, payload)) = semu_payload(&bytes) else {
                                            break "the video stream sent an unknown frame header";
                                        };
                                        let envelopes = annexb_to_avcc(payload, key);
                                        let produced = !envelopes.is_empty();
                                        for envelope in envelopes {
                                            feed.push(envelope);
                                        }
                                        if produced && first {
                                            first = false;
                                            on_status(StreamLinkStatus::Live);
                                            on_first_frame();
                                        }
                                        continue;
                                    }
                                    Some(Ok(Message::Text(text))) if video_session(&text) => {
                                        feed.connected();
                                        reset()
                                    }
                                    Some(Ok(Message::Close(_))) | Some(Err(_)) | None => {
                                        break "the video stream ended";
                                    }
                                    Some(Ok(_)) => continue,
                                },
                            };
                            if sink.send(outgoing).await.is_err() {
                                break "the video stream ended";
                            }
                        }
                    };
                    live.store(false, Ordering::SeqCst);
                    detail.to_string()
                }
            };
            on_status(StreamLinkStatus::Failed { detail });
            tokio::time::sleep(RECONNECT_DELAY).await;
        }
    })
}

fn spawn_link<P>(
    http: reqwest::Client,
    url: String,
    on_status: impl Fn(StreamLinkStatus) + Send + 'static,
    on_first_frame: impl Fn() + Send + 'static,
    mut connect: impl FnMut() -> P + Send + 'static,
    mut consume: impl FnMut(&mut P, &[u8]) -> Option<bool> + Send + 'static,
) -> tokio::task::JoinHandle<()>
where
    P: Send + 'static,
{
    tokio::spawn(async move {
        loop {
            on_status(StreamLinkStatus::Connecting);
            let detail = match http
                .get(&url)
                .send()
                .await
                .and_then(|r| r.error_for_status())
            {
                Err(error) => match error.status() {
                    Some(status) => format!("HTTP {}", status.as_u16()),
                    None => "could not connect".into(),
                },
                Ok(response) => {
                    let mut body = response.bytes_stream();
                    let mut parser = connect();
                    let mut first = true;
                    loop {
                        let Some(Ok(chunk)) = body.next().await else {
                            break "the video stream ended".into();
                        };
                        match consume(&mut parser, &chunk) {
                            None => break "the video stream is malformed".into(),
                            Some(true) if first => {
                                first = false;
                                on_status(StreamLinkStatus::Live);
                                on_first_frame();
                            }
                            Some(_) => {}
                        }
                    }
                }
            };
            on_status(StreamLinkStatus::Failed { detail });
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
        let start = |frames: FrameSlot, _| {
            spawn_reader(http.clone(), hub.url.clone(), frames, |_| {}, || {})
        };
        let mut first = streams.subscribe("UDID", start);
        let second = streams.subscribe("UDID", |_, _| unreachable!("one reader per device"));
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
        let idle = |_: FrameSlot, _| tokio::spawn(std::future::pending::<()>());
        let stale = streams.subscribe("UDID", idle);
        streams.clear(&|_| true);
        let current = streams.subscribe("UDID", idle);
        drop(stale);
        assert_eq!(streams.subscribers("UDID"), 1);
        drop(current);
        assert_eq!(streams.subscribers("UDID"), 0);
    }

    #[tokio::test]
    async fn link_status_follows_subscribers_and_ignores_stale_generations() {
        let published: Arc<Mutex<Vec<BTreeMap<String, DeviceStreamStatus>>>> = Arc::default();
        let sink = published.clone();
        let streams = Arc::new(DeviceStreams::with_sink(move |map| {
            sink.lock().unwrap().push(map)
        }));
        let last = || published.lock().unwrap().last().cloned().unwrap();
        let idle = |_: FrameSlot, _| tokio::spawn(std::future::pending::<()>());
        let first = streams.subscribe("UDID", idle);
        let second = streams.subscribe("UDID", idle);
        assert_eq!(
            last()["UDID"],
            DeviceStreamStatus {
                video: StreamLinkStatus::Connecting,
                avcc: StreamLinkStatus::Idle,
                input: StreamLinkStatus::Idle,
                subscribers: 2,
                panels: BTreeMap::new(),
            }
        );
        let live = |codec| Some((codec, StreamLinkStatus::Live));
        streams.set_status("UDID", 1, live(DeviceVideoCodec::Mjpeg), None);
        streams.set_status(
            "UDID",
            1,
            None,
            Some(StreamLinkStatus::Failed {
                detail: "socket closed".into(),
            }),
        );
        let count = published.lock().unwrap().len();
        streams.set_status(
            "UDID",
            99,
            Some((DeviceVideoCodec::Mjpeg, StreamLinkStatus::Idle)),
            None,
        );
        streams.set_status("UDID", 1, live(DeviceVideoCodec::Mjpeg), None);
        assert_eq!(published.lock().unwrap().len(), count, "no-op changes");
        assert_eq!(last()["UDID"].video, StreamLinkStatus::Live);
        let avcc = streams.subscribe_avcc("UDID", None, DevicePlatform::Ios, |_, _| {
            tokio::spawn(std::future::pending::<()>())
        });
        assert_eq!(last()["UDID"].avcc, StreamLinkStatus::Connecting);
        let failed = StreamLinkStatus::Failed {
            detail: "HTTP 404".into(),
        };
        streams.set_status(
            "UDID",
            1,
            Some((DeviceVideoCodec::H264, failed.clone())),
            None,
        );
        assert_eq!(
            (last()["UDID"].video.clone(), last()["UDID"].avcc.clone()),
            (StreamLinkStatus::Live, failed),
            "each reader reports its own status"
        );
        drop(avcc);
        assert_eq!(last()["UDID"].avcc, StreamLinkStatus::Idle);
        assert_eq!(
            last()["UDID"].input,
            StreamLinkStatus::Failed {
                detail: "socket closed".into()
            }
        );
        drop(first);
        assert_eq!(last()["UDID"].subscribers, 1);
        drop(second);
        assert!(last().is_empty());
        let _third = streams.subscribe("UDID", idle);
        streams.clear(&|_| true);
        assert!(last().is_empty());
    }

    #[tokio::test]
    async fn a_slow_subscriber_sees_only_the_newest_frame() {
        let hub = fake_hub(5, Duration::from_millis(5)).await;
        let streams = Arc::new(DeviceStreams::default());
        let http = reqwest::Client::new();
        let mut slow = streams.subscribe("UDID", |frames, _| {
            spawn_reader(http.clone(), hub.url.clone(), frames, |_| {}, || {})
        });
        tokio::time::sleep(Duration::from_millis(500)).await;
        slow.frames.changed().await.unwrap();
        let newest = slow.frames.borrow_and_update().clone().unwrap();
        assert_eq!(newest.seq, 5);
        assert_eq!(newest.jpeg, b"frame-4");
        assert!(!slow.frames.has_changed().unwrap());
    }

    fn envelope(tag: u8, payload: &[u8]) -> AvccEnvelope {
        AvccEnvelope {
            tag,
            payload: payload.to_vec(),
        }
    }

    fn tags(envelopes: &[Arc<AvccEnvelope>]) -> Vec<u8> {
        envelopes.iter().map(|envelope| envelope.tag).collect()
    }

    #[test]
    fn a_new_connection_drops_deltas_until_its_keyframe_and_joiners_get_the_gop() {
        let feed = AvccFeed::new(None, DevicePlatform::Ios);
        let (_, mut live) = feed.join();
        feed.connected();
        feed.push(envelope(AVCC_SEED, b"jpeg"));
        feed.push(envelope(AVCC_DELTA, b"orphan"));
        feed.push(envelope(AVCC_DESCRIPTION, b"avcC"));
        feed.push(envelope(AVCC_KEYFRAME, b"idr"));
        feed.push(envelope(AVCC_DELTA, b"p1"));
        feed.push(envelope(9, b"unknown"));
        let mut forwarded = Vec::new();
        while let Ok(envelope) = live.try_recv() {
            forwarded.push(envelope.tag);
        }
        assert_eq!(
            forwarded,
            [AVCC_SEED, AVCC_DESCRIPTION, AVCC_KEYFRAME, AVCC_DELTA]
        );
        assert_eq!(
            tags(&feed.join().0),
            [AVCC_DESCRIPTION, AVCC_KEYFRAME, AVCC_DELTA]
        );

        feed.connected();
        feed.push(envelope(AVCC_DELTA, b"stale"));
        assert_eq!(
            feed.join().0.len(),
            3,
            "a delta before the new keyframe is dropped"
        );
        feed.push(envelope(AVCC_KEYFRAME, b"idr2"));
        feed.push(envelope(AVCC_DELTA, &vec![0; GOP_CACHE_BYTES]));
        assert_eq!(
            tags(&feed.join().0),
            [AVCC_DESCRIPTION],
            "an oversized GOP is not replayed, and the seed it replaced is not either"
        );
        feed.connected();
        feed.push(envelope(AVCC_SEED, b"fresh"));
        assert_eq!(tags(&feed.join().0), [AVCC_DESCRIPTION, AVCC_SEED]);
        feed.push(envelope(AVCC_DESCRIPTION, b"rotated"));
        assert_eq!(feed.join().0[0].payload, b"rotated");
    }

    #[tokio::test]
    async fn a_lagging_avcc_batch_stream_resyncs_from_the_cached_gop() {
        let streams = Arc::new(DeviceStreams::default());
        let idle = |_: Arc<AvccFeed>, _| tokio::spawn(std::future::pending::<()>());
        let subscription = streams.subscribe_avcc("UDID", None, DevicePlatform::Ios, idle);
        let feed = subscription.feed.clone();
        feed.push(envelope(AVCC_DESCRIPTION, b"avcC"));
        feed.push(envelope(AVCC_KEYFRAME, b"idr"));
        let mut batches = Box::pin(avcc_batches(subscription));
        let mut demuxer = AvccDemuxer::default();
        let mut next = async || demuxer.push(&batches.next().await.unwrap()).unwrap();
        let first = next().await;
        assert_eq!(
            first.iter().map(|e| e.tag).collect::<Vec<_>>(),
            [AVCC_DESCRIPTION, AVCC_KEYFRAME]
        );
        feed.push(envelope(AVCC_DELTA, b"p1"));
        assert_eq!(next().await, [envelope(AVCC_DELTA, b"p1")]);
        for n in 0..AVCC_QUEUE + 10 {
            feed.push(envelope(AVCC_DELTA, &n.to_be_bytes()));
        }
        let resync = next().await;
        assert_eq!(
            resync[..2],
            [
                envelope(AVCC_DESCRIPTION, b"avcC"),
                envelope(AVCC_KEYFRAME, b"idr")
            ]
        );
        assert_eq!(
            resync.len(),
            2 + 1 + AVCC_QUEUE + 10,
            "the whole GOP, in order"
        );
    }

    #[tokio::test]
    async fn avcc_items_stay_under_the_rpc_frame_limit() {
        let streams = Arc::new(DeviceStreams::default());
        let idle = |_: Arc<AvccFeed>, _| tokio::spawn(std::future::pending::<()>());
        let subscription = streams.subscribe_avcc("UDID", None, DevicePlatform::Ios, idle);
        let feed = subscription.feed.clone();
        feed.push(envelope(AVCC_DESCRIPTION, b"avcC"));
        feed.push(envelope(AVCC_KEYFRAME, &vec![2; AVCC_ITEM_BYTES / 2]));
        for _ in 0..2 {
            feed.push(envelope(AVCC_DELTA, &vec![3; AVCC_ITEM_BYTES / 2]));
        }
        feed.push(envelope(AVCC_DELTA, &vec![4; AVCC_ITEM_BYTES * 2]));
        let mut batches = Box::pin(avcc_batches(subscription));
        let mut demuxer = AvccDemuxer::default();
        let mut items = Vec::new();
        while items.iter().map(Vec::len).sum::<usize>() < 5 {
            let bytes = batches.next().await.unwrap();
            items.push(demuxer.push(&bytes).unwrap());
            assert!(
                bytes.len() <= AVCC_ITEM_BYTES || items.last().unwrap().len() == 1,
                "{} bytes",
                bytes.len()
            );
        }
        let sizes: Vec<Vec<(u8, usize)>> = items
            .iter()
            .map(|item| item.iter().map(|e| (e.tag, e.payload.len())).collect())
            .collect();
        let half = AVCC_ITEM_BYTES / 2;
        assert_eq!(
            sizes,
            [
                vec![(AVCC_DESCRIPTION, 4), (AVCC_KEYFRAME, half)],
                vec![(AVCC_DELTA, half)],
                vec![(AVCC_DELTA, half)],
                vec![(AVCC_DELTA, AVCC_ITEM_BYTES * 2)],
            ][..],
            "the replay is carried across items in order; a big envelope rides alone"
        );

        feed.push(envelope(AVCC_DELTA, &vec![5; AVCC_SENDABLE_BYTES]));
        feed.push(envelope(AVCC_DELTA, b"p"));
        let resync = demuxer.push(&batches.next().await.unwrap()).unwrap();
        assert_eq!(
            resync,
            [
                envelope(AVCC_DESCRIPTION, b"avcC"),
                envelope(AVCC_DELTA, b"p")
            ],
            "an envelope over the frame limit becomes the description, re-arming the keyframe gate"
        );
    }

    #[tokio::test]
    async fn a_hub_pause_forgets_every_cached_description() {
        let streams = Arc::new(DeviceStreams::default());
        let idle = |_: Arc<AvccFeed>, _| tokio::spawn(std::future::pending::<()>());
        let parked = streams.subscribe_avcc("PARKED", None, DevicePlatform::Ios, idle);
        parked.feed.push(envelope(AVCC_DESCRIPTION, b"old"));
        drop(parked);
        let live = streams.subscribe_avcc("LIVE", None, DevicePlatform::Ios, idle);
        live.feed.push(envelope(AVCC_DESCRIPTION, b"old"));
        live.feed.push(envelope(AVCC_KEYFRAME, b"idr"));
        streams.pause(&|_| true);
        assert!(live.feed.join().0.is_empty(), "the live feed starts over");
        let parked = streams.subscribe_avcc("PARKED", None, DevicePlatform::Ios, idle);
        assert!(
            parked.feed.join().0.is_empty(),
            "the parked description is gone"
        );
    }

    #[tokio::test]
    async fn the_last_description_outlives_its_feed_until_the_streams_clear() {
        let streams = Arc::new(DeviceStreams::default());
        let idle = |_: Arc<AvccFeed>, _| tokio::spawn(std::future::pending::<()>());
        let first = streams.subscribe_avcc("UDID", None, DevicePlatform::Ios, idle);
        first.feed.push(envelope(AVCC_DESCRIPTION, b"avcC"));
        let mjpeg = streams.subscribe("UDID", |_, _| tokio::spawn(std::future::pending::<()>()));
        assert_eq!(streams.subscribers("UDID"), 2);
        drop(first);
        assert_eq!(
            streams.subscribers("UDID"),
            1,
            "the MJPEG watcher keeps the entry"
        );
        let second = streams.subscribe_avcc("UDID", None, DevicePlatform::Ios, idle);
        assert_eq!(tags(&second.feed.join().0), [AVCC_DESCRIPTION]);
        drop((second, mjpeg));
        assert_eq!(streams.subscribers("UDID"), 0);
        streams.clear(&|_| true);
        let third = streams.subscribe_avcc("UDID", None, DevicePlatform::Ios, idle);
        assert!(third.feed.join().0.is_empty());
    }

    #[tokio::test]
    async fn a_scoped_pause_and_clear_leave_other_hosts_streams_alone() {
        let streams = Arc::new(DeviceStreams::default());
        let idle = |_: Arc<AvccFeed>, _| tokio::spawn(std::future::pending::<()>());
        let local = streams.subscribe_avcc("emulator-5554", None, DevicePlatform::Android, idle);
        let remote =
            streams.subscribe_avcc("mini/emulator-5554", None, DevicePlatform::Android, idle);
        local.feed.push(envelope(AVCC_DESCRIPTION, b"local"));
        remote.feed.push(envelope(AVCC_DESCRIPTION, b"remote"));
        let is_local = |key: &str| !key.contains('/');
        streams.pause(&is_local);
        assert!(local.feed.join().0.is_empty());
        assert_eq!(tags(&remote.feed.join().0), [AVCC_DESCRIPTION]);
        assert_eq!(
            streams.device_ids(&|key| !is_local(key)),
            ["mini/emulator-5554"]
        );
        streams.clear(&|key| key.starts_with("mini/"));
        assert_eq!(streams.subscribers("mini/emulator-5554"), 0);
        assert_eq!(streams.subscribers("emulator-5554"), 1);
    }
}
