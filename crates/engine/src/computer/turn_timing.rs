use std::collections::VecDeque;
use std::sync::{Arc, Mutex, PoisonError};

use serde_json::{Map, Value, json};

const MAX_TRACKED_TURNS: usize = 256;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TurnTimingOrigin {
    ProviderTurnStart,
    FirstComputerCall,
}

impl TurnTimingOrigin {
    fn as_str(self) -> &'static str {
        match self {
            Self::ProviderTurnStart => "provider-turn-start",
            Self::FirstComputerCall => "first-computer-call",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TurnCallKind {
    Observation,
    Write,
}

struct TurnTiming {
    id: u64,
    thread_id: String,
    turn_id: String,
    started: f64,
    origin: TurnTimingOrigin,
    first_observation: Option<f64>,
    last_observation: Option<f64>,
    first_write: Option<f64>,
}

#[derive(Default)]
struct Turns {
    next_id: u64,
    turns: VecDeque<TurnTiming>,
}

impl Turns {
    fn find(&mut self, thread_id: &str, turn_id: &str) -> Option<&mut TurnTiming> {
        self.turns
            .iter_mut()
            .find(|turn| turn.thread_id == thread_id && turn.turn_id == turn_id)
    }
}

pub type TurnTimingClock = Arc<dyn Fn() -> f64 + Send + Sync>;
pub type TurnTimingEmit = Arc<dyn Fn(Map<String, Value>) + Send + Sync>;

#[derive(Clone)]
pub struct ComputerTurnTimings {
    turns: Arc<Mutex<Turns>>,
    now: TurnTimingClock,
    emit: TurnTimingEmit,
}

impl ComputerTurnTimings {
    pub fn new(now: TurnTimingClock, emit: TurnTimingEmit) -> Self {
        Self {
            turns: Arc::default(),
            now,
            emit,
        }
    }

    pub fn with_log(now: TurnTimingClock) -> Self {
        Self::new(
            now,
            Arc::new(|row| {
                let row = Value::Object(row);
                tracing::info!("[computer-turn-timing] {row}");
            }),
        )
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Turns> {
        self.turns.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub fn start(&self, thread_id: &str, turn_id: &str, origin: TurnTimingOrigin) {
        let mut turns = self.lock();
        if turns.find(thread_id, turn_id).is_some() {
            return;
        }
        while turns.turns.len() >= MAX_TRACKED_TURNS {
            turns.turns.pop_front();
        }
        let id = turns.next_id;
        turns.next_id += 1;
        turns.turns.push_back(TurnTiming {
            id,
            thread_id: thread_id.to_owned(),
            turn_id: turn_id.to_owned(),
            started: (self.now)(),
            origin,
            first_observation: None,
            last_observation: None,
            first_write: None,
        });
    }

    pub fn end(&self, thread_id: &str, turn_id: Option<&str>) {
        self.lock().turns.retain(|turn| {
            turn.thread_id != thread_id || turn_id.is_some_and(|turn_id| turn.turn_id != turn_id)
        });
    }

    pub fn begin(
        &self,
        thread_id: &str,
        turn_id: &str,
        kind: TurnCallKind,
    ) -> impl FnOnce(bool) + Send + 'static {
        self.start(thread_id, turn_id, TurnTimingOrigin::FirstComputerCall);
        let started = (self.now)();
        let (id, observation) = {
            let mut turns = self.lock();
            let state = turns.find(thread_id, turn_id).expect("turn just started");
            (state.id, state.last_observation)
        };
        let timings = self.clone();
        let thread_id = thread_id.to_owned();
        let turn_id = turn_id.to_owned();
        move |completed: bool| {
            if !completed {
                return;
            }
            let row = {
                let mut turns = timings.lock();
                let Some(state) = turns.turns.iter_mut().find(|turn| turn.id == id) else {
                    return;
                };
                let ended = (timings.now)();
                match kind {
                    TurnCallKind::Observation => {
                        state.first_observation.get_or_insert(ended);
                        state.last_observation = Some(ended);
                    }
                    TurnCallKind::Write => {
                        state.first_write.get_or_insert(started);
                    }
                }
                let mut row = Map::new();
                row.insert("threadId".into(), json!(thread_id));
                row.insert("turnId".into(), json!(turn_id));
                row.insert("origin".into(), json!(state.origin.as_str()));
                row.insert(
                    "kind".into(),
                    json!(match kind {
                        TurnCallKind::Observation => "observation",
                        TurnCallKind::Write => "write",
                    }),
                );
                row.insert(
                    "call_start_ms".into(),
                    json!((started - state.started).max(0.0)),
                );
                row.insert(
                    "call_end_ms".into(),
                    json!((ended - state.started).max(0.0)),
                );
                if let Some(first) = state.first_observation {
                    row.insert(
                        "time_to_first_observation_ms".into(),
                        json!((first - state.started).max(0.0)),
                    );
                }
                if let Some(first) = state.first_write {
                    row.insert(
                        "time_to_first_write_ms".into(),
                        json!((first - state.started).max(0.0)),
                    );
                }
                if let (TurnCallKind::Write, Some(observation)) = (kind, observation) {
                    row.insert(
                        "observe_to_write_start_ms".into(),
                        json!((started - observation).max(0.0)),
                    );
                    row.insert(
                        "observe_to_write_end_ms".into(),
                        json!((ended - observation).max(0.0)),
                    );
                }
                row
            };
            (timings.emit)(row);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU64, Ordering};

    use super::*;

    struct Fixture {
        now: Arc<AtomicU64>,
        rows: Arc<Mutex<Vec<Map<String, Value>>>>,
        timings: ComputerTurnTimings,
    }

    impl Fixture {
        fn new(start: u64) -> Self {
            let now = Arc::new(AtomicU64::new(start));
            let rows = Arc::new(Mutex::new(Vec::new()));
            let clock = now.clone();
            let sink = rows.clone();
            let timings = ComputerTurnTimings::new(
                Arc::new(move || clock.load(Ordering::SeqCst) as f64),
                Arc::new(move |row| sink.lock().unwrap().push(row)),
            );
            Self { now, rows, timings }
        }

        fn set(&self, now: u64) {
            self.now.store(now, Ordering::SeqCst);
        }

        fn rows(&self) -> Vec<Map<String, Value>> {
            self.rows.lock().unwrap().clone()
        }
    }

    #[test]
    fn measures_model_delay_and_observation_to_write_without_another_native_call() {
        let f = Fixture::new(0);
        f.timings
            .start("thread", "turn", TurnTimingOrigin::ProviderTurnStart);
        f.set(100);
        let observed = f.timings.begin("thread", "turn", TurnCallKind::Observation);
        f.set(150);
        observed(true);
        f.set(300);
        let wrote = f.timings.begin("thread", "turn", TurnCallKind::Write);
        f.set(320);
        wrote(true);
        let row = &f.rows()[1];
        assert_eq!(row["origin"], "provider-turn-start");
        assert_eq!(row["time_to_first_observation_ms"], 150.0);
        assert_eq!(row["time_to_first_write_ms"], 300.0);
        assert_eq!(row["observe_to_write_start_ms"], 150.0);
        assert_eq!(row["observe_to_write_end_ms"], 170.0);
    }

    #[test]
    fn does_not_record_refusals_or_late_completions_after_a_turn_ends() {
        let f = Fixture::new(10);
        f.timings.begin("thread", "old", TurnCallKind::Write)(false);
        let old = f.timings.begin("thread", "old", TurnCallKind::Observation);
        f.timings
            .start("thread", "new", TurnTimingOrigin::ProviderTurnStart);
        f.timings.end("thread", Some("old"));
        old(true);
        f.timings.begin("thread", "new", TurnCallKind::Write)(true);
        let rows = f.rows();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0]["turnId"], "new");
    }

    #[test]
    fn labels_missing_provider_start_honestly_and_isolates_overlapping_turns() {
        let f = Fixture::new(0);
        f.timings.begin("t", "a", TurnCallKind::Observation)(true);
        f.set(10);
        f.timings.begin("t", "b", TurnCallKind::Write)(true);
        let row = &f.rows()[1];
        assert_eq!(row["origin"], "first-computer-call");
        assert_eq!(row["time_to_first_write_ms"], 0.0);
        assert!(!row.contains_key("observe_to_write_start_ms"));
    }

    #[test]
    fn ending_a_thread_without_a_turn_drops_all_of_its_turns() {
        let f = Fixture::new(0);
        let a = f.timings.begin("t", "a", TurnCallKind::Write);
        let b = f.timings.begin("t", "b", TurnCallKind::Write);
        let other = f.timings.begin("u", "a", TurnCallKind::Write);
        f.timings.end("t", None);
        a(true);
        b(true);
        other(true);
        let rows = f.rows();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0]["threadId"], "u");
    }
}
