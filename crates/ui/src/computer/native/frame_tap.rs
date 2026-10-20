use std::collections::VecDeque;
use std::sync::{Arc, Mutex, MutexGuard, Weak};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use zeron_proto::computer::{CuaComputerTask, CuaPreviewTarget};

use crate::computer::host::ComputerFrameTapHost;

const MAX_DEAD_TARGETS: usize = 256;
// The engine ends the task only when the whole turn ends, which can be long after the last action.
const IDLE_STOP: Duration = Duration::from_secs(30);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ComputerPreviewFrame {
    pub thread_id: String,
    pub window_id: u32,
    pub seq: u64,
    pub jpeg: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ComputerPreviewEvent {
    Frame(ComputerPreviewFrame),
    Failed {
        thread_id: String,
        window_id: u32,
        code: String,
        message: String,
    },
}

pub type PreviewSink = Arc<dyn Fn(ComputerPreviewEvent) + Send + Sync>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StreamFailure {
    pub code: &'static str,
    pub message: String,
}

impl StreamFailure {
    pub fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

pub enum StreamSignal {
    Frame(Vec<u8>),
    Failed(StreamFailure),
}

pub type StreamEvents = Arc<dyn Fn(StreamSignal) + Send + Sync>;

pub trait FrameSource: Send + Sync + 'static {
    type Stream: Send + 'static;
    fn start(
        &self,
        pid: u32,
        window_id: u32,
        events: StreamEvents,
    ) -> Result<Self::Stream, StreamFailure>;
    fn stop(&self, stream: Self::Stream);
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct TargetKey {
    thread_id: String,
    turn_id: Option<String>,
    pid: u32,
    window_id: u32,
}

impl TargetKey {
    fn of(target: &CuaPreviewTarget) -> Self {
        Self {
            thread_id: target.task.thread_id.clone(),
            turn_id: target.task.turn_id.clone(),
            pid: target.pid,
            window_id: target.window_id,
        }
    }

    fn ended_by(&self, task: &CuaComputerTask) -> bool {
        self.thread_id == task.thread_id && (task.turn_id.is_none() || self.turn_id == task.turn_id)
    }
}

struct Active<T> {
    key: TargetKey,
    generation: u64,
    stream: Option<T>,
    touched: Instant,
}

struct TapState<T> {
    active: Option<Active<T>>,
    dead: VecDeque<TargetKey>,
    generation: u64,
    sequence: u64,
}

impl<T> TapState<T> {
    fn remember_dead(&mut self, key: TargetKey) {
        if !self.dead.contains(&key) {
            self.dead.push_back(key);
        }
        while self.dead.len() > MAX_DEAD_TARGETS {
            self.dead.pop_front();
        }
    }
}

struct Inner<S: FrameSource> {
    source: S,
    sink: PreviewSink,
    state: Mutex<TapState<S::Stream>>,
    idle_stop: Duration,
}

impl<S: FrameSource> Inner<S> {
    fn lock(&self) -> MutexGuard<'_, TapState<S::Stream>> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn signal(&self, generation: u64, signal: StreamSignal) {
        match signal {
            StreamSignal::Frame(jpeg) => {
                let frame = {
                    let mut state = self.lock();
                    let Some(active) = state
                        .active
                        .as_ref()
                        .filter(|active| active.generation == generation)
                    else {
                        return;
                    };
                    if active.touched.elapsed() >= self.idle_stop {
                        let stream = state.active.take().and_then(|active| active.stream);
                        drop(state);
                        if let Some(stream) = stream {
                            self.source.stop(stream);
                        }
                        tracing::info!(target: "desktop_cua", "computer frame tap idle, stopped");
                        return;
                    }
                    let (thread_id, window_id) =
                        (active.key.thread_id.clone(), active.key.window_id);
                    state.sequence += 1;
                    ComputerPreviewFrame {
                        thread_id,
                        window_id,
                        seq: state.sequence,
                        jpeg,
                    }
                };
                (self.sink)(ComputerPreviewEvent::Frame(frame));
            }
            StreamSignal::Failed(failure) => {
                let (thread_id, window_id, stream) = {
                    let mut state = self.lock();
                    if !state
                        .active
                        .as_ref()
                        .is_some_and(|active| active.generation == generation)
                    {
                        return;
                    }
                    let Some(active) = state.active.take() else {
                        return;
                    };
                    let window_id = active.key.window_id;
                    let thread_id = active.key.thread_id.clone();
                    state.remember_dead(active.key);
                    (thread_id, window_id, active.stream)
                };
                if let Some(stream) = stream {
                    self.source.stop(stream);
                }
                tracing::info!(target: "desktop_cua", "computer frame tap ended: {}", failure.code);
                (self.sink)(ComputerPreviewEvent::Failed {
                    thread_id,
                    window_id,
                    code: failure.code.into(),
                    message: failure.message,
                });
            }
        }
    }
}

pub struct ComputerFrameTap<S: FrameSource> {
    inner: Arc<Inner<S>>,
}

impl<S: FrameSource> ComputerFrameTap<S> {
    pub fn new(source: S, sink: PreviewSink) -> Self {
        Self {
            inner: Arc::new(Inner {
                source,
                sink,
                state: Mutex::new(TapState {
                    active: None,
                    dead: VecDeque::new(),
                    generation: 0,
                    sequence: 0,
                }),
                idle_stop: IDLE_STOP,
            }),
        }
    }

    fn start(&self, target: &CuaPreviewTarget) -> Result<(), String> {
        let key = TargetKey::of(target);
        let (previous, generation) = {
            let mut state = self.inner.lock();
            if state.dead.contains(&key) {
                return Ok(());
            }
            if let Some(active) = state.active.as_mut().filter(|active| active.key == key) {
                active.touched = Instant::now();
                return Ok(());
            }
            state.generation += 1;
            let generation = state.generation;
            let previous = state.active.replace(Active {
                key: key.clone(),
                generation,
                stream: None,
                touched: Instant::now(),
            });
            (previous.and_then(|active| active.stream), generation)
        };
        if let Some(stream) = previous {
            self.inner.source.stop(stream);
        }
        let weak: Weak<Inner<S>> = Arc::downgrade(&self.inner);
        let events: StreamEvents = Arc::new(move |signal| {
            if let Some(inner) = weak.upgrade() {
                inner.signal(generation, signal);
            }
        });
        match self.inner.source.start(key.pid, key.window_id, events) {
            Ok(stream) => {
                let orphan = {
                    let mut state = self.inner.lock();
                    match state.active.as_mut() {
                        Some(active) if active.generation == generation => {
                            active.stream = Some(stream);
                            None
                        }
                        _ => Some(stream),
                    }
                };
                if let Some(stream) = orphan {
                    self.inner.source.stop(stream);
                }
                Ok(())
            }
            Err(failure) => {
                let message = failure.message.clone();
                self.inner.signal(generation, StreamSignal::Failed(failure));
                Err(message)
            }
        }
    }

    fn stop_active(&self) {
        let stream = self
            .inner
            .lock()
            .active
            .take()
            .and_then(|active| active.stream);
        if let Some(stream) = stream {
            self.inner.source.stop(stream);
        }
    }
}

#[async_trait(?Send)]
impl<S: FrameSource> ComputerFrameTapHost for ComputerFrameTap<S> {
    fn update(&self, target: CuaPreviewTarget) -> Result<(), String> {
        self.start(&target)
    }

    fn touch(&self, task: &CuaComputerTask) {
        if let Some(active) = self
            .inner
            .lock()
            .active
            .as_mut()
            .filter(|active| active.key.ended_by(task))
        {
            active.touched = Instant::now();
        }
    }

    async fn end_task(&self, task: CuaComputerTask) -> Result<(), String> {
        {
            let mut state = self.inner.lock();
            if state
                .active
                .as_ref()
                .is_some_and(|active| !active.key.ended_by(&task))
            {
                return Ok(());
            }
            state.dead.retain(|key| !key.ended_by(&task));
        }
        self.stop_active();
        Ok(())
    }

    async fn stop(&self) -> Result<(), String> {
        self.stop_active();
        Ok(())
    }

    async fn dispose(&self) -> Result<(), String> {
        self.stop_active();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct Source {
        log: Mutex<Vec<String>>,
        events: Mutex<Vec<StreamEvents>>,
        fail_start: Mutex<Option<StreamFailure>>,
        next: Mutex<u32>,
    }

    impl Source {
        fn log(&self) -> Vec<String> {
            self.log.lock().unwrap().clone()
        }

        fn emit(&self, stream: usize, signal: StreamSignal) {
            let events = self.events.lock().unwrap()[stream].clone();
            events(signal);
        }
    }

    impl FrameSource for Arc<Source> {
        type Stream = u32;

        fn start(
            &self,
            pid: u32,
            window_id: u32,
            events: StreamEvents,
        ) -> Result<u32, StreamFailure> {
            if let Some(failure) = self.fail_start.lock().unwrap().take() {
                return Err(failure);
            }
            let mut next = self.next.lock().unwrap();
            *next += 1;
            self.events.lock().unwrap().push(events);
            self.log
                .lock()
                .unwrap()
                .push(format!("start {} {pid}:{window_id}", *next));
            Ok(*next)
        }

        fn stop(&self, stream: u32) {
            self.log.lock().unwrap().push(format!("stop {stream}"));
        }
    }

    fn target(thread: &str, turn: Option<&str>, window_id: u32) -> CuaPreviewTarget {
        CuaPreviewTarget {
            task: CuaComputerTask {
                thread_id: thread.into(),
                turn_id: turn.map(Into::into),
                label: None,
            },
            pid: 42,
            window_id,
            cursor: None,
        }
    }

    fn task(thread: &str, turn: Option<&str>) -> CuaComputerTask {
        target(thread, turn, 0).task
    }

    fn tap() -> (
        Arc<Source>,
        ComputerFrameTap<Arc<Source>>,
        Arc<Mutex<Vec<ComputerPreviewEvent>>>,
    ) {
        let source = Arc::new(Source::default());
        let events = Arc::new(Mutex::new(Vec::new()));
        let sink_events = events.clone();
        let tap = ComputerFrameTap::new(
            source.clone(),
            Arc::new(move |event| sink_events.lock().unwrap().push(event)),
        );
        (source, tap, events)
    }

    #[test]
    fn one_target_one_stream_and_frames_carry_a_running_sequence() {
        let (source, tap, events) = tap();
        tap.update(target("a", Some("1"), 7)).unwrap();
        tap.update(target("a", Some("1"), 7)).unwrap();
        assert_eq!(source.log(), vec!["start 1 42:7"]);
        source.emit(0, StreamSignal::Frame(vec![1]));
        tap.update(target("a", Some("1"), 8)).unwrap();
        assert_eq!(source.log(), vec!["start 1 42:7", "stop 1", "start 2 42:8"]);
        source.emit(0, StreamSignal::Frame(vec![9]));
        source.emit(1, StreamSignal::Frame(vec![2]));
        assert_eq!(
            *events.lock().unwrap(),
            vec![
                ComputerPreviewEvent::Frame(ComputerPreviewFrame {
                    thread_id: "a".into(),
                    window_id: 7,
                    seq: 1,
                    jpeg: vec![1],
                }),
                ComputerPreviewEvent::Frame(ComputerPreviewFrame {
                    thread_id: "a".into(),
                    window_id: 8,
                    seq: 2,
                    jpeg: vec![2],
                }),
            ]
        );
    }

    #[test]
    fn a_failed_target_stays_dead_and_reports_instead_of_widening() {
        let (source, tap, events) = tap();
        tap.update(target("a", Some("1"), 7)).unwrap();
        source.emit(
            0,
            StreamSignal::Failed(StreamFailure::new("target_unavailable", "closed")),
        );
        assert_eq!(source.log(), vec!["start 1 42:7", "stop 1"]);
        tap.update(target("a", Some("1"), 7)).unwrap();
        assert_eq!(source.log().len(), 2, "a dead target never respawns");
        assert_eq!(
            events.lock().unwrap().last(),
            Some(&ComputerPreviewEvent::Failed {
                thread_id: "a".into(),
                window_id: 7,
                code: "target_unavailable".into(),
                message: "closed".into(),
            })
        );
        source.emit(0, StreamSignal::Frame(vec![1]));
        assert_eq!(
            events.lock().unwrap().len(),
            1,
            "a dead stream delivers nothing"
        );
        tap.update(target("a", Some("2"), 7)).unwrap();
        assert_eq!(source.log().last().unwrap(), "start 2 42:7");
    }

    #[test]
    fn a_start_failure_is_reported_and_remembered() {
        let (source, tap, events) = tap();
        *source.fail_start.lock().unwrap() = Some(StreamFailure::new(
            "screen_recording_required",
            "Screen Recording is required.",
        ));
        assert_eq!(
            tap.update(target("a", None, 7)).unwrap_err(),
            "Screen Recording is required."
        );
        assert!(matches!(
            events.lock().unwrap().as_slice(),
            [ComputerPreviewEvent::Failed { code, .. }] if code == "screen_recording_required"
        ));
        tap.update(target("a", None, 7)).unwrap();
        assert!(source.log().is_empty());
    }

    #[tokio::test]
    async fn end_task_forgets_failures_and_leaves_other_tasks_running() {
        let (source, tap, _events) = tap();
        tap.update(target("a", Some("1"), 7)).unwrap();
        source.emit(
            0,
            StreamSignal::Failed(StreamFailure::new("capture_stopped", "stopped")),
        );
        tap.update(target("b", Some("1"), 9)).unwrap();
        tap.end_task(task("a", Some("1"))).await.unwrap();
        assert_eq!(
            source.log().last().unwrap(),
            "start 2 42:9",
            "b keeps streaming"
        );
        tap.end_task(task("b", None)).await.unwrap();
        assert_eq!(source.log().last().unwrap(), "stop 2");
        tap.update(target("a", Some("1"), 7)).unwrap();
        assert_eq!(source.log().last().unwrap(), "stop 2", "a1 is still dead");
        tap.end_task(task("a", Some("1"))).await.unwrap();
        tap.update(target("a", Some("1"), 7)).unwrap();
        assert_eq!(
            source.log().last().unwrap(),
            "start 3 42:7",
            "the ended task's failure memory is gone"
        );
        tap.end_task(task("a", Some("2"))).await.unwrap();
        assert_eq!(source.log().last().unwrap(), "start 3 42:7");
        tap.end_task(task("a", None)).await.unwrap();
        assert_eq!(source.log().last().unwrap(), "stop 3");
    }

    #[test]
    fn an_idle_stream_stops_on_its_next_frame_and_the_next_action_restarts_it() {
        let (source, mut tap, events) = tap();
        Arc::get_mut(&mut tap.inner).unwrap().idle_stop = Duration::ZERO;
        tap.update(target("a", Some("1"), 7)).unwrap();
        source.emit(0, StreamSignal::Frame(vec![1]));
        assert_eq!(source.log(), vec!["start 1 42:7", "stop 1"]);
        assert!(
            events.lock().unwrap().is_empty(),
            "an idle frame is dropped"
        );
        tap.update(target("a", Some("1"), 7)).unwrap();
        assert_eq!(source.log().last().unwrap(), "start 2 42:7");
    }

    #[test]
    fn any_call_from_the_owning_task_keeps_the_stream_alive() {
        let (source, mut tap, events) = tap();
        Arc::get_mut(&mut tap.inner).unwrap().idle_stop = Duration::from_millis(100);
        tap.update(target("a", Some("1"), 7)).unwrap();
        std::thread::sleep(Duration::from_millis(150));
        tap.touch(&task("a", Some("1")));
        source.emit(0, StreamSignal::Frame(vec![1]));
        assert_eq!(events.lock().unwrap().len(), 1);
        std::thread::sleep(Duration::from_millis(150));
        tap.touch(&task("b", Some("1")));
        source.emit(0, StreamSignal::Frame(vec![2]));
        assert_eq!(source.log(), vec!["start 1 42:7", "stop 1"]);
    }

    #[tokio::test]
    async fn stop_and_dispose_end_the_stream() {
        let (source, tap, _events) = tap();
        tap.update(target("a", None, 7)).unwrap();
        tap.stop().await.unwrap();
        tap.update(target("a", None, 7)).unwrap();
        tap.dispose().await.unwrap();
        assert_eq!(
            source.log(),
            vec!["start 1 42:7", "stop 1", "start 2 42:7", "stop 2"]
        );
    }
}
