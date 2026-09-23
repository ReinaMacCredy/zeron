use std::sync::{Arc, Mutex, MutexGuard, PoisonError, Weak};
use std::time::Duration;

use futures::FutureExt;
use futures::future::BoxFuture;
use tokio::task::JoinHandle;

use super::backend::{ComputerError, ComputerFrameListener, ComputerStreamFrame};
use super::still_frame_dedupe::StillFrameDedupe;

const MAX_FORCE_RETRIES: u32 = 1;

pub const DEFAULT_STILL_INTERVAL_MS: u64 = 500;
pub const MIN_STILL_INTERVAL_MS: u64 = 100;

pub fn resolve_still_interval_ms(interval_ms: Option<u64>) -> u64 {
    interval_ms
        .unwrap_or(DEFAULT_STILL_INTERVAL_MS)
        .max(MIN_STILL_INTERVAL_MS)
}

pub type StillCapture =
    Arc<dyn Fn(bool) -> BoxFuture<'static, Result<Option<Vec<u8>>, ComputerError>> + Send + Sync>;
pub type StillPrepare = Arc<dyn Fn() -> BoxFuture<'static, ()> + Send + Sync>;

#[derive(Clone)]
pub struct StillFramePublisherOptions {
    pub capture: StillCapture,
    pub is_capture_available: Arc<dyn Fn() -> bool + Send + Sync>,
    pub prepare: Option<StillPrepare>,
    pub emit: Arc<dyn Fn(&ComputerStreamFrame) + Send + Sync>,
    pub now: Arc<dyn Fn() -> f64 + Send + Sync>,
    pub interval_ms: u64,
}

struct PublisherState {
    dedupe: StillFrameDedupe,
    listener: Option<ComputerFrameListener>,
    attachment_generation: u64,
    timer: Option<JoinHandle<()>>,
    in_flight: bool,
    next_sequence: u64,
    force_retries: u32,
}

struct PublisherInner {
    options: StillFramePublisherOptions,
    state: Mutex<PublisherState>,
}

#[derive(Clone)]
pub struct StillFramePublisher {
    inner: Arc<PublisherInner>,
}

impl StillFramePublisher {
    pub fn new(options: StillFramePublisherOptions) -> Self {
        Self {
            inner: Arc::new(PublisherInner {
                options,
                state: Mutex::new(PublisherState {
                    dedupe: StillFrameDedupe::default(),
                    listener: None,
                    attachment_generation: 0,
                    timer: None,
                    in_flight: false,
                    next_sequence: 1,
                    force_retries: 0,
                }),
            }),
        }
    }

    fn state(&self) -> MutexGuard<'_, PublisherState> {
        self.inner
            .state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }

    pub fn interval_ms(&self) -> u64 {
        self.inner.options.interval_ms
    }

    pub fn timer_armed(&self) -> bool {
        self.state().timer.is_some()
    }

    fn clear_timer(state: &mut PublisherState) {
        if let Some(timer) = state.timer.take() {
            timer.abort();
        }
    }

    pub async fn attach(&self, listener: ComputerFrameListener) {
        let generation = {
            let mut state = self.state();
            state.attachment_generation += 1;
            state.listener = None;
            Self::clear_timer(&mut state);
            state.dedupe.reset();
            state.attachment_generation
        };
        if let Some(prepare) = &self.inner.options.prepare {
            prepare().await;
        }
        {
            let mut state = self.state();
            if state.attachment_generation != generation {
                return;
            }
            state.listener = Some(listener);
        }
        self.publish(true).await;
        let mut state = self.state();
        if state.attachment_generation != generation {
            return;
        }
        let weak = Arc::downgrade(&self.inner);
        let interval = Duration::from_millis(self.inner.options.interval_ms);
        state.timer = Some(tokio::spawn(tick(weak, interval)));
    }

    pub async fn detach(&self) {
        let mut state = self.state();
        state.attachment_generation += 1;
        state.listener = None;
        Self::clear_timer(&mut state);
        state.dedupe.reset();
    }

    pub async fn request_keyframe(&self) {
        if self.state().listener.is_none() {
            return;
        }
        self.publish(true).await;
    }

    pub fn publish(&self, force: bool) -> BoxFuture<'static, ()> {
        let publisher = self.clone();
        async move { publisher.publish_once(force).await }.boxed()
    }

    async fn publish_once(&self, explicit: bool) {
        let (listener, generation, force) = {
            let mut state = self.state();
            let Some(listener) = state.listener.clone() else {
                return;
            };
            if !(self.inner.options.is_capture_available)() {
                return;
            }
            if state.in_flight {
                if explicit {
                    state.force_retries = 0;
                    state.dedupe.defer_force();
                }
                return;
            }
            state.in_flight = true;
            if explicit {
                state.force_retries = 0;
            }
            let force = state.dedupe.take_force(explicit);
            (listener, state.attachment_generation, force)
        };
        let captured = (self.inner.options.capture)(force).await;
        let frame = {
            let mut state = self.state();
            let frame = match captured {
                Ok(Some(bytes))
                    if state.attachment_generation == generation
                        && state.dedupe.should_publish(&bytes, force) =>
                {
                    state.force_retries = 0;
                    let sequence = state.next_sequence;
                    state.next_sequence += 1;
                    Some(ComputerStreamFrame {
                        sequence,
                        timestamp_ms: (self.inner.options.now)(),
                        keyframe: true,
                        codec_config: false,
                        data: bytes,
                    })
                }
                Ok(_) => None,
                Err(_) => {
                    if state.attachment_generation == generation
                        && force
                        && state.force_retries < MAX_FORCE_RETRIES
                    {
                        state.force_retries += 1;
                        state.dedupe.defer_force();
                    }
                    None
                }
            };
            if frame.is_none() {
                state.in_flight = false;
            }
            frame
        };
        if let Some(frame) = frame {
            listener(frame.clone());
            (self.inner.options.emit)(&frame);
            self.state().in_flight = false;
        }
        let again = {
            let state = self.state();
            state.dedupe.force_pending() && state.listener.is_some()
        };
        if again {
            self.publish(false).await;
        }
    }
}

async fn tick(publisher: Weak<PublisherInner>, interval: Duration) {
    let mut ticks = tokio::time::interval_at(tokio::time::Instant::now() + interval, interval);
    ticks.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        ticks.tick().await;
        let Some(inner) = publisher.upgrade() else {
            return;
        };
        tokio::spawn(StillFramePublisher { inner }.publish(false));
    }
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use tokio::sync::oneshot;

    use super::*;
    use crate::computer::backend::ComputerBackendError;

    const FRAME_A: [u8; 3] = [1, 2, 3];
    const FRAME_B: [u8; 4] = [4, 5, 6, 7];

    type Frames = Arc<Mutex<Vec<ComputerStreamFrame>>>;

    struct Harness {
        publisher: StillFramePublisher,
        observed: Frames,
        captures: Arc<AtomicUsize>,
    }

    type Capture = Arc<
        dyn Fn(bool) -> BoxFuture<'static, Result<Option<Vec<u8>>, ComputerError>> + Send + Sync,
    >;

    fn make(
        capture: Capture,
        available: Option<Arc<dyn Fn() -> bool + Send + Sync>>,
        interval_ms: u64,
        prepare: Option<StillPrepare>,
    ) -> Harness {
        let observed: Frames = Arc::default();
        let captures = Arc::new(AtomicUsize::new(0));
        let counted = captures.clone();
        let sink = observed.clone();
        let publisher = StillFramePublisher::new(StillFramePublisherOptions {
            capture: Arc::new(move |force| {
                counted.fetch_add(1, Ordering::SeqCst);
                capture(force)
            }),
            is_capture_available: available.unwrap_or_else(|| Arc::new(|| true)),
            prepare,
            emit: Arc::new(move |frame| sink.lock().unwrap().push(frame.clone())),
            now: Arc::new(|| 0.0),
            interval_ms,
        });
        Harness {
            publisher,
            observed,
            captures,
        }
    }

    fn fixed(bytes: &'static [u8]) -> Capture {
        Arc::new(move |_| async move { Ok(Some(bytes.to_vec())) }.boxed())
    }

    fn failing() -> Capture {
        Arc::new(|_| async { Err(ComputerBackendError::new("capture failed").into()) }.boxed())
    }

    fn recorder() -> (ComputerFrameListener, Frames) {
        let frames: Frames = Arc::default();
        let sink = frames.clone();
        (
            Arc::new(move |frame| sink.lock().unwrap().push(frame)),
            frames,
        )
    }

    impl Harness {
        fn captures(&self) -> usize {
            self.captures.load(Ordering::SeqCst)
        }
    }

    async fn settle() {
        for _ in 0..16 {
            tokio::task::yield_now().await;
        }
    }

    #[tokio::test(start_paused = true)]
    async fn does_not_start_capturing_when_detached_during_preparation() {
        let (ready, preparation) = oneshot::channel::<()>();
        let preparation = preparation.shared();
        let harness = make(
            fixed(&FRAME_A),
            None,
            100,
            Some(Arc::new(move || {
                let preparation = preparation.clone();
                async move {
                    let _ = preparation.await;
                }
                .boxed()
            })),
        );
        let (listener, frames) = recorder();
        let publisher = harness.publisher.clone();
        let attaching = tokio::spawn(async move { publisher.attach(listener).await });
        settle().await;
        harness.publisher.detach().await;
        ready.send(()).unwrap();
        attaching.await.unwrap();
        tokio::time::sleep(Duration::from_millis(500)).await;
        assert_eq!(harness.captures(), 0);
        assert!(harness.observed.lock().unwrap().is_empty());
        assert!(frames.lock().unwrap().is_empty());
        assert!(!harness.publisher.timer_armed());
    }

    #[tokio::test(start_paused = true)]
    async fn keeps_the_newest_listener_when_preparations_finish_out_of_order() {
        let (first_ready, first) = oneshot::channel::<()>();
        let (second_ready, second) = oneshot::channel::<()>();
        let preparations = Arc::new(Mutex::new(VecDeque::from([first, second])));
        let harness = make(
            fixed(&FRAME_A),
            None,
            100,
            Some(Arc::new(move || {
                let next = preparations.lock().unwrap().pop_front();
                async move {
                    if let Some(next) = next {
                        let _ = next.await;
                    }
                }
                .boxed()
            })),
        );
        let (first_listener, first_frames) = recorder();
        let (second_listener, second_frames) = recorder();
        let publisher = harness.publisher.clone();
        let first_attach = tokio::spawn(async move { publisher.attach(first_listener).await });
        settle().await;
        let publisher = harness.publisher.clone();
        let second_attach = tokio::spawn(async move { publisher.attach(second_listener).await });
        settle().await;
        second_ready.send(()).unwrap();
        second_attach.await.unwrap();
        first_ready.send(()).unwrap();
        first_attach.await.unwrap();
        harness.publisher.request_keyframe().await;
        assert!(first_frames.lock().unwrap().is_empty());
        assert_eq!(second_frames.lock().unwrap().len(), 2);
        assert_eq!(harness.captures(), 2);
        assert!(harness.publisher.timer_armed());
        harness.publisher.detach().await;
    }

    #[tokio::test(start_paused = true)]
    async fn keeps_one_timer_when_overlapping_attaches_reuse_the_same_listener() {
        let harness = make(fixed(&FRAME_A), None, 100, None);
        let (listener, _frames) = recorder();
        futures::join!(
            harness.publisher.attach(listener.clone()),
            harness.publisher.attach(listener)
        );
        assert!(harness.publisher.timer_armed());
        let ticks_before = harness.captures();
        tokio::time::sleep(Duration::from_millis(100)).await;
        settle().await;
        assert_eq!(harness.captures(), ticks_before + 1);
        harness.publisher.detach().await;
        assert!(!harness.publisher.timer_armed());
    }

    #[tokio::test(start_paused = true)]
    async fn immediately_serves_a_replacement_listener_after_an_old_capture_settles() {
        for fails in [false, true] {
            let (release, held) = oneshot::channel::<Result<Vec<u8>, ()>>();
            let held = Arc::new(Mutex::new(Some(held)));
            let harness = make(
                Arc::new(move |_| {
                    let first = held.lock().unwrap().take();
                    async move {
                        match first {
                            Some(first) => match first.await.unwrap() {
                                Ok(bytes) => Ok(Some(bytes)),
                                Err(()) => {
                                    Err(ComputerBackendError::new("old capture failed").into())
                                }
                            },
                            None => Ok(Some(FRAME_B.to_vec())),
                        }
                    }
                    .boxed()
                }),
                None,
                100,
                None,
            );
            let (listener, frames) = recorder();
            let publisher = harness.publisher.clone();
            let reused = listener.clone();
            let first_attach = tokio::spawn(async move { publisher.attach(reused).await });
            settle().await;
            assert_eq!(harness.captures(), 1);
            harness.publisher.detach().await;
            harness.publisher.attach(listener).await;
            release
                .send(if fails { Err(()) } else { Ok(FRAME_A.to_vec()) })
                .unwrap();
            first_attach.await.unwrap();
            settle().await;
            assert_eq!(harness.captures(), 2, "fails={fails}");
            let received = frames.lock().unwrap().clone();
            assert_eq!(received.len(), 1);
            assert_eq!(received[0].data, FRAME_B);
            let observed: Vec<Vec<u8>> = harness
                .observed
                .lock()
                .unwrap()
                .iter()
                .map(|frame| frame.data.clone())
                .collect();
            assert_eq!(observed, [FRAME_B.to_vec()]);
            assert!(harness.publisher.timer_armed());
            harness.publisher.detach().await;
        }
    }

    #[tokio::test]
    async fn publishes_the_first_frame_dedupes_identical_ones_and_republishes_on_a_keyframe() {
        let bytes = Arc::new(Mutex::new(FRAME_A.to_vec()));
        let current = bytes.clone();
        let harness = make(
            Arc::new(move |_| {
                let bytes = current.lock().unwrap().clone();
                async move { Ok(Some(bytes)) }.boxed()
            }),
            None,
            100,
            None,
        );
        let (listener, frames) = recorder();
        harness.publisher.attach(listener).await;
        assert_eq!(frames.lock().unwrap().len(), 1);
        assert_eq!(*harness.observed.lock().unwrap(), *frames.lock().unwrap());
        harness.publisher.publish(false).await;
        assert_eq!(frames.lock().unwrap().len(), 1);
        harness.publisher.request_keyframe().await;
        assert_eq!(frames.lock().unwrap().len(), 2);
        *bytes.lock().unwrap() = FRAME_B.to_vec();
        harness.publisher.publish(false).await;
        assert_eq!(frames.lock().unwrap().len(), 3);
        harness.publisher.detach().await;
    }

    #[tokio::test]
    async fn passes_keyframe_force_to_native_deduplication_after_idle_ticks_and_reattach() {
        let forces = Arc::new(Mutex::new(Vec::new()));
        let seen = forces.clone();
        let harness = make(
            Arc::new(move |force| {
                seen.lock().unwrap().push(force);
                async move { Ok(force.then(|| FRAME_A.to_vec())) }.boxed()
            }),
            None,
            100,
            None,
        );
        let (listener, frames) = recorder();
        harness.publisher.attach(listener.clone()).await;
        harness.publisher.publish(false).await;
        harness.publisher.publish(false).await;
        assert_eq!(frames.lock().unwrap().len(), 1);
        harness.publisher.request_keyframe().await;
        assert_eq!(frames.lock().unwrap().len(), 2);
        harness.publisher.detach().await;
        harness.publisher.attach(listener).await;
        assert_eq!(frames.lock().unwrap().len(), 3);
        assert_eq!(*forces.lock().unwrap(), [true, false, false, true, true]);
        harness.publisher.detach().await;
    }

    #[tokio::test]
    async fn bounds_the_retries_a_failing_forced_capture_buys() {
        let harness = make(failing(), None, 100, None);
        let (listener, frames) = recorder();
        harness.publisher.attach(listener).await;
        assert!(frames.lock().unwrap().is_empty());
        assert_eq!(harness.captures(), 2);
        harness.publisher.publish(false).await;
        assert_eq!(harness.captures(), 3);
        harness.publisher.detach().await;
    }

    #[tokio::test]
    async fn gives_a_fresh_keyframe_request_its_own_retry_budget() {
        let harness = make(failing(), None, 100, None);
        let (listener, _frames) = recorder();
        harness.publisher.attach(listener).await;
        assert_eq!(harness.captures(), 2);
        harness.publisher.request_keyframe().await;
        assert_eq!(harness.captures(), 4);
        harness.publisher.detach().await;
    }

    #[tokio::test]
    async fn serves_a_keyframe_requested_while_a_capture_was_already_in_flight() {
        let gate: Arc<Mutex<Option<oneshot::Receiver<()>>>> = Arc::default();
        let armed = gate.clone();
        let harness = make(
            Arc::new(move |_| {
                let held = armed.lock().unwrap().take();
                async move {
                    if let Some(held) = held {
                        let _ = held.await;
                    }
                    Ok(Some(FRAME_A.to_vec()))
                }
                .boxed()
            }),
            None,
            100,
            None,
        );
        let (listener, frames) = recorder();
        harness.publisher.attach(listener).await;
        assert_eq!(frames.lock().unwrap().len(), 1);
        let (release, held) = oneshot::channel();
        *gate.lock().unwrap() = Some(held);
        let publisher = harness.publisher.clone();
        let in_flight = tokio::spawn(publisher.publish(false));
        settle().await;
        let keyframe = harness.publisher.request_keyframe();
        keyframe.await;
        release.send(()).unwrap();
        in_flight.await.unwrap();
        assert_eq!(frames.lock().unwrap().len(), 2);
        harness.publisher.detach().await;
    }

    #[tokio::test]
    async fn publishes_nothing_for_a_tick_the_backend_has_no_target_for() {
        let harness = make(Arc::new(|_| async { Ok(None) }.boxed()), None, 100, None);
        let (listener, frames) = recorder();
        harness.publisher.attach(listener).await;
        assert!(frames.lock().unwrap().is_empty());
        assert_eq!(harness.captures(), 1);
        harness.publisher.detach().await;
    }

    #[tokio::test]
    async fn never_captures_while_the_backend_says_capture_is_unavailable() {
        let harness = make(fixed(&FRAME_A), Some(Arc::new(|| false)), 100, None);
        let (listener, frames) = recorder();
        harness.publisher.attach(listener).await;
        harness.publisher.publish(false).await;
        harness.publisher.request_keyframe().await;
        assert_eq!(harness.captures(), 0);
        assert!(frames.lock().unwrap().is_empty());
        harness.publisher.detach().await;
    }

    #[tokio::test(start_paused = true)]
    async fn stops_the_timer_loop_on_detach() {
        let bytes = Arc::new(Mutex::new(FRAME_A.to_vec()));
        let current = bytes.clone();
        let harness = make(
            Arc::new(move |_| {
                let bytes = current.lock().unwrap().clone();
                async move { Ok(Some(bytes)) }.boxed()
            }),
            None,
            100,
            None,
        );
        let (listener, frames) = recorder();
        harness.publisher.attach(listener).await;
        tokio::time::sleep(Duration::from_millis(250)).await;
        settle().await;
        let while_attached = harness.captures();
        assert!(while_attached > 1);
        harness.publisher.detach().await;
        *bytes.lock().unwrap() = FRAME_B.to_vec();
        tokio::time::sleep(Duration::from_millis(500)).await;
        settle().await;
        assert_eq!(harness.captures(), while_attached);
        assert_eq!(frames.lock().unwrap().len(), 1);
    }

    #[tokio::test(start_paused = true)]
    async fn keeps_exactly_the_newest_attach_timer_when_two_attaches_overlap() {
        let harness = make(fixed(&FRAME_A), None, 100, None);
        let (first, _first_frames) = recorder();
        let (second, _second_frames) = recorder();
        futures::join!(
            harness.publisher.attach(first),
            harness.publisher.attach(second)
        );
        let settled = harness.captures();
        tokio::time::sleep(Duration::from_millis(100)).await;
        settle().await;
        assert_eq!(harness.captures(), settled + 1);
        harness.publisher.detach().await;
    }

    #[test]
    fn clamps_the_interval_to_the_floor_and_defaults_it() {
        assert_eq!(resolve_still_interval_ms(None), DEFAULT_STILL_INTERVAL_MS);
        assert_eq!(resolve_still_interval_ms(Some(5)), MIN_STILL_INTERVAL_MS);
        assert_eq!(resolve_still_interval_ms(Some(750)), 750);
    }
}
