use crate::model_options::compact_tokens;
use crate::usage::format_usd as usd;
use crate::{CompactPlan, Session, SessionStats, SessionStatus};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cache {
    Refreshing { ttl_secs: u32 },
    Warm { left_ms: i64, ttl_secs: u32 },
    Cold { ago_ms: i64 },
}

impl Cache {
    pub fn of(stats: &SessionStats, working: bool, now_ms: i64) -> Option<Self> {
        let ttl_secs = stats.cache_ttl_secs?;
        let left_ms = stats.cache_remaining_ms(now_ms)?;
        Some(if working {
            Cache::Refreshing { ttl_secs }
        } else if left_ms > 0 {
            Cache::Warm { left_ms, ttl_secs }
        } else {
            Cache::Cold { ago_ms: -left_ms }
        })
    }

    pub fn running_low(self) -> bool {
        matches!(self, Cache::Warm { left_ms, ttl_secs } if left_ms * 5 < i64::from(ttl_secs) * 1000)
    }

    pub fn short(self) -> String {
        match self {
            Cache::Refreshing { ttl_secs } => tier(ttl_secs),
            Cache::Warm { left_ms, .. } if left_ms >= 60_000 => format!("{}m", left_ms / 60_000),
            Cache::Warm { left_ms, .. } => format!("{}s", left_ms / 1000),
            Cache::Cold { .. } => "cold".into(),
        }
    }

    pub fn long(self) -> String {
        match self {
            Cache::Refreshing { ttl_secs } => format!("Refreshing · {}", tier(ttl_secs)),
            Cache::Warm { left_ms, ttl_secs } => {
                format!("{} left · {}", duration(left_ms as u64), tier(ttl_secs))
            }
            Cache::Cold { ago_ms } => format!("Expired {} ago", duration(ago_ms as u64)),
        }
    }
}

fn tier(ttl_secs: u32) -> String {
    if ttl_secs % 3600 == 0 {
        format!("{}h", ttl_secs / 3600)
    } else {
        format!("{}m", ttl_secs / 60)
    }
}

pub fn chip_label(stats: &SessionStats) -> Option<String> {
    if stats.turns == 0 && stats.steps == 0 {
        return None;
    }
    let plural = |n: u64, one: &str| {
        if n == 1 {
            format!("1 {one}")
        } else {
            format!("{n} {one}s")
        }
    };
    let mut label = format!(
        "{} {}",
        plural(stats.turns, "turn"),
        plural(stats.steps, "step")
    );
    if let Some(tps) = stats.tokens_per_second() {
        label.push_str(&format!(" · {tps:.0} tok/s"));
    }
    Some(label)
}

pub fn duration(ms: u64) -> String {
    const UNITS: [(u64, &str); 6] = [
        (365 * 86_400, "y"),
        (30 * 86_400, "mo"),
        (86_400, "d"),
        (3_600, "h"),
        (60, "m"),
        (1, "s"),
    ];
    if ms < 60_000 {
        return format!("{:.1}s", ms as f64 / 1000.0);
    }
    let secs = ms / 1000;
    let (i, &(size, unit)) = UNITS
        .iter()
        .enumerate()
        .find(|(_, (size, _))| secs >= *size)
        .expect("secs >= 60");
    let (next_size, next_unit) = UNITS[i + 1];
    format!(
        "{}{unit}{:02}{next_unit}",
        secs / size,
        secs % size / next_size
    )
}

pub fn stats_rows(stats: &SessionStats, cache: Option<Cache>) -> Vec<(&'static str, String)> {
    let missing = || "—".to_string();
    let mut rows = vec![
        ("LLM time", duration(stats.llm_ms)),
        ("Tool time", duration(stats.tool_ms)),
        (
            "Avg time to first token (TTFT)",
            stats.avg_ttft_ms().map(duration).unwrap_or_else(missing),
        ),
        (
            "Tokens per second (TPS)",
            stats
                .tokens_per_second()
                .map(|tps| format!("{tps:.0} tok/s"))
                .unwrap_or_else(missing),
        ),
    ];
    if let Some(cache) = cache {
        rows.push(("Cache TTL", cache.long()));
    }
    if let Some(rate) = stats.cache_hit_rate() {
        rows.push(("Cache hit rate", format!("{:.0}%", rate * 100.0)));
    }
    rows
}

pub fn cost_headline(stats: &SessionStats) -> Option<String> {
    match stats.cost_usd() {
        Some(cost) if stats.unpriced_turns > 0 => Some(format!("≥ {}", usd(cost))),
        Some(cost) if stats.cost_estimated => Some(format!("≈ {}", usd(cost))),
        Some(cost) => Some(usd(cost)),
        None if stats.total_tokens() > 0 => Some("No known price".into()),
        None => None,
    }
}

pub fn cost_note(stats: &SessionStats) -> Option<String> {
    stats.cost_usd()?;
    let n = stats.unpriced_turns;
    if n > 0 {
        let turns = if n == 1 {
            "1 turn".into()
        } else {
            format!("{n} turns")
        };
        Some(format!("Excludes {turns} on a model with no known price"))
    } else {
        stats
            .cost_estimated
            .then(|| "Estimated from public API prices".into())
    }
}

pub fn token_split(stats: &SessionStats) -> Vec<(&'static str, u64)> {
    let mut split = vec![("Input", stats.input_tokens)];
    if stats.cache_read_tokens + stats.cache_write_tokens > 0 {
        split.push(("Cache read", stats.cache_read_tokens));
        split.push(("Cache write", stats.cache_write_tokens));
    }
    split.push(("Output", stats.output_tokens));
    split
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NextIcon {
    Spin,
    Agent,
    Clock,
    Warn,
    Off,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NextCompaction {
    pub icon: NextIcon,
    pub title: String,
    pub detail: Option<String>,
    pub chip: Option<String>,
}

const CANCELS: &str = "A message from you first cancels it";

impl NextCompaction {
    pub fn of(
        session: Option<&Session>,
        compacting: bool,
        tokens: Option<u64>,
        auto: Option<u64>,
        harness: &str,
    ) -> Self {
        let next = |icon, title: String, detail: Option<&str>, chip: Option<String>| Self {
            icon,
            title,
            detail: detail.map(Into::into),
            chip,
        };
        if compacting {
            return next(NextIcon::Spin, "Compacting now".into(), None, None);
        }
        match session.and_then(|session| session.compact_plan) {
            Some(CompactPlan::Agent) => {
                return next(
                    NextIcon::Agent,
                    "Agent asked · when this turn ends".into(),
                    Some(CANCELS),
                    Some("compacts after this turn".into()),
                );
            }
            Some(CompactPlan::Idle { at }) => {
                return next(
                    NextIcon::Clock,
                    format!("Idle · {}", clock(at)),
                    Some(CANCELS),
                    Some(format!("compacts {}", clock(at))),
                );
            }
            _ => {}
        }
        if let (Some(tokens), Some(auto)) = (tokens, auto)
            && tokens * 100 >= auto * 85
        {
            let at = compact_tokens(auto);
            return next(
                NextIcon::Warn,
                format!("{harness} · at {at}"),
                Some("The turn carries on after it"),
                Some(format!("auto-compacts at {at}")),
            );
        }
        let detail = match session {
            Some(session) if session.status == SessionStatus::Working => {
                Some("The agent is working".into())
            }
            Some(session) => session.compact_hold.and_then(|hold| hold.detail()),
            None => None,
        };
        Self {
            detail,
            ..next(NextIcon::Off, "Not scheduled".into(), None, None)
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ContextMarks {
    pub floor: Option<u64>,
    pub warn: Option<u64>,
    pub auto: Option<u64>,
}

impl ContextMarks {
    /// `ask` is where Zeron asks a running turn whether to carry the history further
    /// (`model_options::chat_ask_threshold`).
    pub fn of(
        window: Option<u64>,
        threshold: Option<u64>,
        floor: Option<u64>,
        ask: Option<u64>,
    ) -> Self {
        let window = window.filter(|window| *window > 0);
        let auto = threshold.filter(|threshold| window.is_some_and(|window| *threshold < window));
        let warn = ask.filter(|ask| auto.or(window).is_some_and(|limit| *ask < limit));
        Self {
            floor: floor.filter(|floor| {
                warn.or(auto).or(window).is_some_and(|limit| *floor < limit)
            }),
            warn,
            auto,
        }
    }
}

pub fn reminder_floor(nudge: u64, baseline: u64, window: Option<u64>) -> Option<u64> {
    if nudge == 0 {
        return None;
    }
    // A window smaller than the floor would never see the reminder.
    let floor = window.filter(|window| *window > 0).map_or(nudge, |window| {
        nudge.min(window.saturating_sub(baseline) * 7 / 10)
    });
    Some(baseline + floor)
}

fn clock(ms: i64) -> String {
    chrono::DateTime::from_timestamp_millis(ms)
        .map(|utc| {
            utc.with_timezone(&chrono::Local)
                .format("%-I:%M %p")
                .to_string()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::CompactHold;
    use crate::usage::format_tokens as tokens;

    #[test]
    fn the_next_compaction_names_the_first_reason_that_holds() {
        let mut session: Session = serde_json::from_value(serde_json::json!({
            "chatId": "c",
            "deviceId": "d",
            "status": "idle",
            "startedAt": null,
            "updatedAt": "2026-01-01T00:00:00Z",
        }))
        .unwrap();
        let of = |session: &Session, compacting, tokens| {
            NextCompaction::of(
                Some(session),
                compacting,
                tokens,
                Some(167_000),
                "Claude Code",
            )
        };
        let unheld = of(&session, false, None);
        assert_eq!(
            (unheld.icon, unheld.title.as_str(), unheld.detail),
            (NextIcon::Off, "Not scheduled", None)
        );
        assert_eq!(of(&session, true, None).icon, NextIcon::Spin);
        let warn = of(&session, false, Some(141_950));
        assert_eq!(warn.icon, NextIcon::Warn);
        assert_eq!(warn.title, "Claude Code · at 167K");
        assert_eq!(warn.chip.as_deref(), Some("auto-compacts at 167K"));
        assert_eq!(of(&session, false, Some(141_949)).icon, NextIcon::Off);
        session.compact_hold = Some(CompactHold::Small { min: 100_000 });
        let held = of(&session, false, None);
        assert_eq!(
            (held.icon, held.title.as_str(), held.detail.as_deref()),
            (
                NextIcon::Off,
                "Not scheduled",
                Some("Context is under 100K")
            )
        );
        session.status = SessionStatus::Working;
        assert_eq!(
            of(&session, false, None).detail.as_deref(),
            Some("The agent is working")
        );
        let at = 1_700_000_000_000;
        session.compact_plan = Some(CompactPlan::Idle { at });
        let idle = of(&session, false, Some(160_000));
        assert_eq!(idle.icon, NextIcon::Clock);
        assert_eq!(idle.title, format!("Idle · {}", clock(at)));
        assert_eq!(idle.detail.as_deref(), Some(CANCELS));
        session.compact_plan = Some(CompactPlan::Agent);
        let agent = of(&session, false, None);
        assert_eq!(agent.chip.as_deref(), Some("compacts after this turn"));
        session.compact_plan = Some(CompactPlan::Other);
        session.compact_hold = Some(CompactHold::Other);
        session.status = SessionStatus::Idle;
        let unknown = of(&session, false, None);
        assert_eq!((unknown.icon, unknown.detail), (NextIcon::Off, None));
    }

    #[test]
    fn the_meter_marks_sit_under_a_known_auto_compaction() {
        let floor = reminder_floor(150_000, 20_000, Some(200_000));
        assert_eq!(floor, Some(20_000 + 126_000));
        assert_eq!(reminder_floor(0, 0, Some(200_000)), None);
        assert_eq!(
            ContextMarks::of(Some(200_000), Some(167_000), floor, None),
            ContextMarks {
                floor: Some(146_000),
                warn: None,
                auto: Some(167_000),
            }
        );
        let large = reminder_floor(150_000, 20_000, Some(1_000_000));
        assert_eq!(
            ContextMarks::of(Some(1_000_000), Some(767_000), large, Some(567_000)),
            ContextMarks {
                floor: Some(170_000),
                warn: Some(567_000),
                auto: Some(767_000),
            }
        );
        assert_eq!(
            ContextMarks::of(Some(1_000_000), Some(767_000), Some(600_000), Some(567_000)).floor,
            None,
            "a floor past the ask point is not drawn"
        );
        let codex = ContextMarks::of(
            Some(272_000),
            Some(272_000),
            reminder_floor(150_000, 0, Some(272_000)),
            None,
        );
        assert_eq!(
            codex,
            ContextMarks {
                floor: Some(150_000),
                warn: None,
                auto: None,
            }
        );
    }

    #[test]
    fn chip_reads_like_the_reference_and_hides_on_a_fresh_chat() {
        assert_eq!(chip_label(&SessionStats::default()), None);
        let stats = SessionStats {
            turns: 86,
            steps: 1903,
            output_tokens: 137_000,
            usage_llm_ms: 1_000_000,
            ..Default::default()
        };
        assert_eq!(
            chip_label(&stats).unwrap(),
            "86 turns 1903 steps · 137 tok/s"
        );
        let one = SessionStats {
            turns: 1,
            steps: 1,
            ..Default::default()
        };
        assert_eq!(chip_label(&one).unwrap(), "1 turn 1 step");
    }

    #[test]
    fn cache_reads_warm_low_cold_and_refreshing() {
        let stats = SessionStats {
            cache_ttl_secs: Some(3600),
            cache_touched_at: Some(1_000_000),
            cache_read_tokens: 90,
            cache_write_tokens: 5,
            input_tokens: 5,
            ..Default::default()
        };
        let warm = Cache::of(&stats, false, 1_000_000 + 8 * 60_000).unwrap();
        assert_eq!(warm.short(), "52m");
        assert_eq!(warm.long(), "52m00s left · 1h");
        assert!(!warm.running_low());
        let low = Cache::of(&stats, false, 1_000_000 + 3_590_000).unwrap();
        assert_eq!(low.short(), "10s");
        assert!(low.running_low());
        let cold = Cache::of(&stats, false, 1_000_000 + 3_600_000 + 720_000).unwrap();
        assert_eq!(cold.long(), "Expired 12m00s ago");
        let live = Cache::of(&stats, true, 1_000_000 + 9_000_000).unwrap();
        assert_eq!(live.long(), "Refreshing · 1h");
        let rows = stats_rows(&stats, Some(warm));
        assert_eq!(rows.last().unwrap(), &("Cache hit rate", "90%".to_string()));
        assert_eq!(Cache::of(&SessionStats::default(), false, 0), None);
    }

    #[test]
    fn durations_roll_up_into_the_largest_unit() {
        assert_eq!(duration(13_400), "13.4s");
        assert_eq!(duration(60_000), "1m00s");
        assert_eq!(duration(59 * 60_000 + 59_000), "59m59s");
        assert_eq!(duration(3_600_000), "1h00m");
        assert_eq!(duration(172 * 60_000 + 14_000), "2h52m");
        assert_eq!(duration(24 * 3_600_000), "1d00h");
        assert_eq!(duration(50 * 3_600_000), "2d02h");
        assert_eq!(duration(30 * 86_400_000), "1mo00d");
        assert_eq!(duration(400 * 86_400_000), "1y01mo");
    }

    #[test]
    fn token_counts_shorten_like_the_demo() {
        assert_eq!(tokens(940), "940");
        assert_eq!(tokens(3_140), "3.1K");
        assert_eq!(tokens(3_000), "3K");
        assert_eq!(tokens(418_600), "419K");
        assert_eq!(tokens(999_999), "1M");
        assert_eq!(tokens(9_812_000), "9.81M");
        assert_eq!(tokens(38_620_000), "38.6M");
        assert_eq!(tokens(3_271_436_831), "3.27B");
        assert_eq!(tokens(16_703_622_261), "16.7B");
    }

    #[test]
    fn cost_headline_marks_partial_and_unreported_sessions() {
        assert_eq!(cost_headline(&SessionStats::default()), None);
        let mut claude = SessionStats {
            input_tokens: 10,
            cost_micros: 12_840_000,
            ..Default::default()
        };
        claude.note_cost("s", 12.84);
        assert_eq!(cost_headline(&claude).unwrap(), "$12.84");
        assert_eq!(cost_note(&claude), None);
        let mixed = SessionStats {
            unpriced_turns: 4,
            ..claude
        };
        assert_eq!(cost_headline(&mixed).unwrap(), "≥ $12.84");
        assert_eq!(
            cost_note(&mixed).unwrap(),
            "Excludes 4 turns on a model with no known price"
        );
        let estimated = SessionStats {
            cost_estimated: true,
            ..claude
        };
        assert_eq!(cost_headline(&estimated).unwrap(), "≈ $12.84");
        assert_eq!(
            cost_note(&estimated).unwrap(),
            "Estimated from public API prices"
        );
        let codex = SessionStats {
            input_tokens: 9_812_000,
            output_tokens: 211_400,
            unpriced_turns: 5,
            ..Default::default()
        };
        assert_eq!(cost_headline(&codex).unwrap(), "No known price");
        assert_eq!(cost_note(&codex), None);
        assert_eq!(
            token_split(&codex),
            [("Input", 9_812_000), ("Output", 211_400)]
        );
        assert_eq!(usd(0.0811149), "$0.081");
        assert_eq!(usd(9352.924), "$9,352.92");
    }

    #[test]
    fn unreported_figures_read_as_a_dash() {
        let values: Vec<String> = stats_rows(&SessionStats::default(), None)
            .into_iter()
            .map(|(_, v)| v)
            .collect();
        assert_eq!(values, ["0.0s", "0.0s", "—", "—"]);
    }
}
