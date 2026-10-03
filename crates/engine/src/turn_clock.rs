use std::collections::HashSet;
use std::time::Instant;

use zeron_proto::{AgentEvent, LIVE_PLAN_TOOL_ID, SessionStats};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Phase {
    Llm,
    Tool,
    Waiting,
}

pub(crate) struct TurnClock {
    active: bool,
    started: Instant,
    awaiting_first_output: bool,
    phase_since: Instant,
    inflight: HashSet<String>,
    seen_tools: HashSet<String>,
    open_inputs: HashSet<String>,
    reported_usage: bool,
    stats: SessionStats,
}

impl TurnClock {
    pub(crate) fn new(now: Instant) -> Self {
        Self {
            active: false,
            started: now,
            awaiting_first_output: false,
            phase_since: now,
            inflight: HashSet::new(),
            seen_tools: HashSet::new(),
            open_inputs: HashSet::new(),
            reported_usage: false,
            stats: SessionStats::default(),
        }
    }

    pub(crate) fn is_active(&self) -> bool {
        self.active
    }

    pub(crate) fn streaming(&self) -> bool {
        self.active && self.phase() == Phase::Llm
    }

    /// `user_turn` marks a segment a person's prompt opened: it counts as a
    /// turn and its first output is a time-to-first-token sample.
    pub(crate) fn start(&mut self, user_turn: bool, now: Instant) {
        *self = Self::new(now);
        self.active = true;
        self.awaiting_first_output = user_turn;
        self.stats.turns = u64::from(user_turn);
    }

    pub(crate) fn observe(&mut self, event: &AgentEvent, now: Instant) {
        if !self.active {
            return;
        }
        let before = self.phase();
        match event {
            AgentEvent::TextDelta { text } | AgentEvent::ReasoningDelta { text }
                if !text.is_empty() =>
            {
                self.first_output(now);
            }
            AgentEvent::ToolCall { id, .. } => {
                self.first_output(now);
                if id != LIVE_PLAN_TOOL_ID && self.seen_tools.insert(id.clone()) {
                    self.stats.steps += 1;
                    self.inflight.insert(id.clone());
                }
            }
            AgentEvent::ToolResult { id, .. } => {
                self.inflight.remove(id);
            }
            AgentEvent::InputRequested { request_id, .. } => {
                self.open_inputs.insert(request_id.clone());
            }
            AgentEvent::InputResolved { request_id, .. } => {
                self.open_inputs.remove(request_id);
            }
            AgentEvent::Usage {
                input_tokens,
                output_tokens,
            } => {
                self.stats.input_tokens += input_tokens;
                self.stats.output_tokens += output_tokens;
                self.reported_usage = true;
            }
            AgentEvent::PromptCache {
                read_tokens,
                write_tokens,
                ttl_secs,
            } => {
                self.stats.cache_read_tokens += read_tokens;
                self.stats.cache_write_tokens += write_tokens;
                self.stats.cache_ttl_secs = ttl_secs.or(self.stats.cache_ttl_secs);
                self.stats.cache_touched_at = Some(crate::now_ms());
            }
            _ => {}
        }
        if self.phase() != before {
            self.close(before, now);
        }
    }

    pub(crate) fn settle(&mut self, now: Instant) -> Option<SessionStats> {
        if !self.active {
            return None;
        }
        self.close(self.phase(), now);
        if self.reported_usage {
            self.stats.usage_llm_ms = self.stats.llm_ms;
        }
        let stats = self.stats;
        *self = Self::new(now);
        Some(stats)
    }

    fn phase(&self) -> Phase {
        if !self.open_inputs.is_empty() {
            Phase::Waiting
        } else if !self.inflight.is_empty() {
            Phase::Tool
        } else {
            Phase::Llm
        }
    }

    fn close(&mut self, phase: Phase, now: Instant) {
        let ms = now.saturating_duration_since(self.phase_since).as_millis() as u64;
        match phase {
            Phase::Llm => self.stats.llm_ms += ms,
            Phase::Tool => self.stats.tool_ms += ms,
            Phase::Waiting => {}
        }
        self.phase_since = now;
    }

    fn first_output(&mut self, now: Instant) {
        if std::mem::take(&mut self.awaiting_first_output) {
            self.stats.ttft_ms += now.saturating_duration_since(self.started).as_millis() as u64;
            self.stats.ttft_count += 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn at(base: Instant, ms: u64) -> Instant {
        base + Duration::from_millis(ms)
    }

    fn call(id: &str) -> AgentEvent {
        AgentEvent::ToolCall {
            id: id.into(),
            call: zeron_proto::ToolCall::Unknown {
                name: "Bash".into(),
                input: None,
            },
        }
    }

    fn result(id: &str) -> AgentEvent {
        AgentEvent::ToolResult {
            id: id.into(),
            is_error: false,
            output: None,
            diff: None,
        }
    }

    fn text(t: &str) -> AgentEvent {
        AgentEvent::TextDelta { text: t.into() }
    }

    #[test]
    fn overlapping_tools_count_wall_time_once() {
        let t0 = Instant::now();
        let mut clock = TurnClock::new(t0);
        clock.start(true, t0);
        clock.observe(&text("hi"), at(t0, 1_000));
        clock.observe(&call("a"), at(t0, 2_000));
        clock.observe(&call("b"), at(t0, 2_500));
        clock.observe(&result("a"), at(t0, 4_000));
        clock.observe(&result("b"), at(t0, 6_000));
        clock.observe(&text("done"), at(t0, 7_000));
        let stats = clock.settle(at(t0, 8_000)).unwrap();
        assert_eq!(stats.turns, 1);
        assert_eq!(stats.steps, 2);
        assert_eq!(stats.tool_ms, 4_000);
        assert_eq!(stats.llm_ms, 4_000);
        assert_eq!((stats.ttft_ms, stats.ttft_count), (1_000, 1));
        assert!(!clock.is_active());
    }

    #[test]
    fn open_question_and_plan_chip_are_not_billed() {
        let t0 = Instant::now();
        let mut clock = TurnClock::new(t0);
        clock.start(true, t0);
        clock.observe(&call(LIVE_PLAN_TOOL_ID), at(t0, 500));
        clock.observe(
            &AgentEvent::InputRequested {
                request_id: "q".into(),
                questions: Vec::new(),
            },
            at(t0, 1_000),
        );
        clock.observe(
            &AgentEvent::InputResolved {
                request_id: "q".into(),
                answers: None,
            },
            at(t0, 61_000),
        );
        let stats = clock.settle(at(t0, 62_000)).unwrap();
        assert_eq!(stats.steps, 0);
        assert_eq!(stats.tool_ms, 0);
        assert_eq!(stats.llm_ms, 2_000);
        assert_eq!((stats.ttft_ms, stats.ttft_count), (500, 1));
    }

    #[test]
    fn continuation_segments_add_time_without_a_turn_or_ttft() {
        let t0 = Instant::now();
        let mut clock = TurnClock::new(t0);
        assert_eq!(clock.settle(t0), None);
        clock.start(false, t0);
        clock.observe(&text("x"), at(t0, 3_000));
        clock.observe(
            &AgentEvent::Usage {
                input_tokens: 10,
                output_tokens: 300,
            },
            at(t0, 3_000),
        );
        let stats = clock.settle(at(t0, 3_000)).unwrap();
        assert_eq!((stats.turns, stats.ttft_count), (0, 0));
        assert_eq!(stats.output_tokens, 300);
        assert_eq!(stats.usage_llm_ms, 3_000);
        assert_eq!(stats.tokens_per_second(), Some(100.0));
    }

    #[test]
    fn prompt_cache_sums_tokens_and_keeps_the_last_written_tier() {
        let t0 = Instant::now();
        let mut clock = TurnClock::new(t0);
        clock.start(true, t0);
        for (read, write, ttl) in [(40_000, 3_000, Some(3600)), (45_000, 0, None)] {
            clock.observe(
                &AgentEvent::PromptCache {
                    read_tokens: read,
                    write_tokens: write,
                    ttl_secs: ttl,
                },
                t0,
            );
        }
        clock.observe(
            &AgentEvent::Usage {
                input_tokens: 2_000,
                output_tokens: 10,
            },
            t0,
        );
        let stats = clock.settle(t0).unwrap();
        assert_eq!(
            (stats.cache_read_tokens, stats.cache_write_tokens),
            (85_000, 3_000)
        );
        assert_eq!(stats.cache_ttl_secs, Some(3600));
        assert!(stats.cache_touched_at.is_some());
        assert_eq!(stats.cache_hit_rate(), Some(85_000.0 / 90_000.0));
    }
}
