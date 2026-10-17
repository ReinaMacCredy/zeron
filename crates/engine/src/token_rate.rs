use std::time::Instant;

use zeron_proto::AgentEvent;

const HALF_LIVES_MS: [f64; 3] = [5_000.0, 20_000.0, 80_000.0];
const MIN_TOKENS: f64 = 200.0;
const MIN_TIME_MS: f64 = 4_000.0;
const HIDDEN_DECAY: f64 = 0.8;
const HIDDEN_PRIOR_MS: f64 = 10_000.0;
const CHARS_PER_TOKEN: f64 = 4.0;

pub fn estimated_tokens(chars: usize) -> u64 {
    (chars as f64 / CHARS_PER_TOKEN).ceil() as u64
}

#[derive(Clone, Copy, Default)]
struct Sums {
    tokens: f64,
    time: f64,
}

struct Segment {
    local: f64,
    span_ms: f64,
    hidden_rate: f64,
    streaming_since: Option<Instant>,
}

/// Port of oh-my-pi's `TokenRateMeter` (packages/coding-agent/src/utils/token-rate.ts).
/// A segment runs to the next billed `Usage`, which corrects the character
/// estimate as a hidden-token rate; tool time never ages the sums.
pub(crate) struct TokenRate {
    history: [Sums; 3],
    inflight: [Sums; 3],
    segment: Option<Segment>,
    hidden_tokens: f64,
    hidden_span_ms: f64,
}

impl TokenRate {
    pub(crate) fn new() -> Self {
        Self {
            history: [Sums::default(); 3],
            inflight: [Sums::default(); 3],
            segment: None,
            hidden_tokens: 0.0,
            hidden_span_ms: 0.0,
        }
    }

    /// Wrapped subagent events are never counted: `Usage` bills the main thread only.
    pub(crate) fn observe(&mut self, event: &AgentEvent, streaming: bool, now: Instant) {
        match event {
            AgentEvent::Usage { output_tokens, .. } => self.end(Some(*output_tokens), now),
            AgentEvent::Done { .. } => self.end(None, now),
            _ => {
                if streaming {
                    self.stream(now);
                } else {
                    self.pause(now);
                }
                if let AgentEvent::TextDelta { text } | AgentEvent::ReasoningDelta { text } = event
                    && streaming
                {
                    self.push(text);
                }
            }
        }
    }

    pub(crate) fn stream(&mut self, now: Instant) {
        let hidden_rate = (self.hidden_tokens / (self.hidden_span_ms + HIDDEN_PRIOR_MS)).max(0.0);
        let segment = self.segment.get_or_insert(Segment {
            local: 0.0,
            span_ms: 0.0,
            hidden_rate,
            streaming_since: None,
        });
        if segment.streaming_since.is_none() {
            segment.streaming_since = Some(now);
        } else {
            self.advance(now);
        }
    }

    fn pause(&mut self, now: Instant) {
        self.advance(now);
        if let Some(segment) = &mut self.segment {
            segment.streaming_since = None;
        }
    }

    fn push(&mut self, text: &str) {
        let tokens = text.chars().count() as f64 / CHARS_PER_TOKEN;
        if let Some(segment) = &mut self.segment {
            segment.local += tokens;
        }
        for sums in &mut self.inflight {
            sums.tokens += tokens;
        }
    }

    fn end(&mut self, billed: Option<u64>, now: Instant) {
        self.advance(now);
        let Some(segment) = self.segment.take() else {
            return;
        };
        let mut extra = 0.0;
        if let Some(billed) = billed.filter(|n| *n > 0) {
            let hidden = billed as f64 - segment.local;
            self.hidden_tokens = self.hidden_tokens * HIDDEN_DECAY + hidden;
            self.hidden_span_ms = self.hidden_span_ms * HIDDEN_DECAY + segment.span_ms;
            extra = hidden - segment.hidden_rate * segment.span_ms;
        }
        for (history, live) in self.history.iter_mut().zip(&mut self.inflight) {
            let spread = if segment.span_ms > 0.0 {
                extra * live.time / segment.span_ms
            } else {
                0.0
            };
            history.tokens += (live.tokens + spread).max(0.0);
            history.time += live.time;
            *live = Sums::default();
        }
    }

    fn advance(&mut self, now: Instant) {
        let Some(segment) = &mut self.segment else {
            return;
        };
        let Some(since) = segment.streaming_since else {
            return;
        };
        let dt = now.saturating_duration_since(since).as_secs_f64() * 1000.0;
        if dt <= 0.0 {
            return;
        }
        segment.streaming_since = Some(now);
        segment.span_ms += dt;
        for ((history, live), half_life) in self
            .history
            .iter_mut()
            .zip(&mut self.inflight)
            .zip(HALF_LIVES_MS)
        {
            let f = (-dt / half_life).exp2();
            let integral = half_life / std::f64::consts::LN_2 * (1.0 - f);
            history.tokens *= f;
            history.time *= f;
            live.tokens = live.tokens * f + segment.hidden_rate * integral;
            live.time = live.time * f + integral;
        }
    }

    pub(crate) fn rate(&self, now: Instant) -> Option<f64> {
        let (dt, hidden_rate) = match &self.segment {
            Some(Segment {
                streaming_since: Some(since),
                hidden_rate,
                ..
            }) => (
                now.saturating_duration_since(*since).as_secs_f64() * 1000.0,
                *hidden_rate,
            ),
            _ => (0.0, 0.0),
        };
        let (mut tokens, mut time) = (0.0, 0.0);
        let (mut evidence_tokens, mut evidence_time) = (0.0, 0.0);
        for ((history, live), half_life) in
            self.history.iter().zip(&self.inflight).zip(HALF_LIVES_MS)
        {
            let f = (-dt / half_life).exp2();
            let integral = half_life / std::f64::consts::LN_2 * (1.0 - f);
            evidence_tokens = (history.tokens + live.tokens) * f + hidden_rate * integral;
            evidence_time = (history.time + live.time) * f + integral;
            tokens += evidence_tokens;
            time += evidence_time;
        }
        (evidence_tokens >= MIN_TOKENS && evidence_time >= MIN_TIME_MS)
            .then(|| tokens * 1000.0 / time)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn at(base: Instant, ms: u64) -> Instant {
        base + Duration::from_millis(ms)
    }

    fn text(n: usize) -> AgentEvent {
        AgentEvent::TextDelta {
            text: "x".repeat(n),
        }
    }

    fn stream_for(meter: &mut TokenRate, t0: Instant, from_ms: u64, to_ms: u64, chars: usize) {
        let mut t = from_ms;
        while t < to_ms {
            t += 100;
            meter.observe(&text(chars), true, at(t0, t));
        }
    }

    #[test]
    fn gate_hides_a_thin_start_then_reads_the_streamed_rate() {
        let t0 = Instant::now();
        let mut meter = TokenRate::new();
        meter.stream(t0);
        stream_for(&mut meter, t0, 0, 2_000, 40);
        assert_eq!(meter.rate(at(t0, 2_000)), None);
        stream_for(&mut meter, t0, 2_000, 20_000, 40);
        let rate = meter.rate(at(t0, 20_000)).unwrap();
        assert!((rate - 100.0).abs() < 2.0, "{rate}");
    }

    #[test]
    fn tool_time_holds_the_reading() {
        let t0 = Instant::now();
        let mut meter = TokenRate::new();
        meter.stream(t0);
        stream_for(&mut meter, t0, 0, 10_000, 40);
        let before = meter.rate(at(t0, 10_000)).unwrap();
        meter.observe(
            &AgentEvent::ToolCall {
                id: "t".into(),
                call: zeron_proto::ToolCall::Unknown {
                    name: "Bash".into(),
                    input: None,
                },
            },
            false,
            at(t0, 10_000),
        );
        assert_eq!(meter.rate(at(t0, 70_000)), Some(before));
        meter.observe(
            &AgentEvent::ToolResult {
                id: "t".into(),
                is_error: false,
                output: None,
                diff: None,
            },
            true,
            at(t0, 70_000),
        );
        stream_for(&mut meter, t0, 70_000, 75_000, 40);
        let after = meter.rate(at(t0, 75_000)).unwrap();
        assert!((after - 100.0).abs() < 2.0, "{after}");
    }

    #[test]
    fn billed_usage_teaches_the_hidden_rate_for_the_next_segment() {
        let t0 = Instant::now();
        let mut meter = TokenRate::new();
        meter.stream(t0);
        stream_for(&mut meter, t0, 0, 10_000, 40);
        meter.observe(
            &AgentEvent::Usage {
                input_tokens: 0,
                output_tokens: 2_000,
            },
            true,
            at(t0, 10_000),
        );
        let settled = meter.rate(at(t0, 10_000)).unwrap();
        assert!((settled - 200.0).abs() < 2.0, "{settled}");
        meter.stream(at(t0, 10_000));
        meter.observe(
            &AgentEvent::ReasoningDelta {
                text: String::new(),
            },
            true,
            at(t0, 20_000),
        );
        let silent = meter.rate(at(t0, 20_000)).unwrap();
        assert!(silent > 20.0, "{silent}");
    }

    #[test]
    fn subagent_text_is_not_counted() {
        let t0 = Instant::now();
        let mut meter = TokenRate::new();
        meter.stream(t0);
        let mut t = 0;
        while t < 10_000 {
            t += 100;
            meter.observe(
                &AgentEvent::Subagent {
                    parent_tool_use_id: "p".into(),
                    event: Box::new(text(400)),
                },
                true,
                at(t0, t),
            );
        }
        assert_eq!(meter.rate(at(t0, 10_000)), None);
    }
}
