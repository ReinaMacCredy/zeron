use crate::theme::Theme;
use gpui::{Context, IntoElement, SharedString, Task, Window, div, prelude::*, px};
use zeron_proto::SessionStats;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Cache {
    Refreshing { ttl_secs: u32 },
    Warm { left_ms: i64, ttl_secs: u32 },
    Cold { ago_ms: i64 },
}

impl Cache {
    fn of(stats: &SessionStats, working: bool, now_ms: i64) -> Option<Self> {
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

    fn running_low(self) -> bool {
        matches!(self, Cache::Warm { left_ms, ttl_secs } if left_ms * 5 < i64::from(ttl_secs) * 1000)
    }

    fn short(self) -> String {
        match self {
            Cache::Refreshing { ttl_secs } => tier(ttl_secs),
            Cache::Warm { left_ms, .. } if left_ms >= 60_000 => format!("{}m", left_ms / 60_000),
            Cache::Warm { left_ms, .. } => format!("{}s", left_ms / 1000),
            Cache::Cold { .. } => "cold".into(),
        }
    }

    fn long(self) -> String {
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

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

fn tick<T: 'static>(
    secs: u64,
    cx: &mut Context<T>,
    on_tick: impl Fn(&mut T, &mut Context<T>) + 'static,
) -> Task<()> {
    cx.spawn(async move |this, cx| {
        loop {
            cx.background_executor()
                .timer(std::time::Duration::from_secs(secs))
                .await;
            if this.update(cx, |this, cx| on_tick(this, cx)).is_err() {
                break;
            }
        }
    })
}

pub struct StatsChip {
    stats: Option<SessionStats>,
    working: bool,
    warm: bool,
    _tick: Task<()>,
}

impl StatsChip {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let tick = tick(10, cx, |chip, cx| {
            let warm = chip.warm_now();
            if warm || chip.warm {
                cx.notify();
            }
            chip.warm = warm;
        });
        Self {
            stats: None,
            working: false,
            warm: false,
            _tick: tick,
        }
    }

    fn warm_now(&self) -> bool {
        self.stats
            .and_then(|stats| Cache::of(&stats, self.working, now_ms()))
            .is_some_and(|cache| matches!(cache, Cache::Warm { .. }))
    }

    pub fn set(&mut self, stats: Option<SessionStats>, working: bool, cx: &mut Context<Self>) {
        if (self.stats, self.working) != (stats, working) {
            self.stats = stats;
            self.working = working;
            self.warm = self.warm_now();
            cx.notify();
        }
    }
}

impl Render for StatsChip {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::of(cx).clone();
        match self
            .stats
            .and_then(|stats| chip(stats, self.working, &theme))
        {
            Some(chip) => chip.into_any_element(),
            None => div().into_any_element(),
        }
    }
}

fn chip(stats: SessionStats, working: bool, theme: &Theme) -> Option<gpui::Stateful<gpui::Div>> {
    let label = chip_label(&stats)?;
    let cache = Cache::of(&stats, working, now_ms());
    let cache_color = match cache {
        Some(c) if c.running_low() => theme.warning,
        Some(Cache::Cold { .. }) => theme.text_faint,
        _ => theme.text_muted.opacity(0.6),
    };
    Some(
        div()
            .id("session-stats")
            .flex_none()
            .h(px(20.0))
            .flex()
            .flex_row()
            .items_center()
            .gap(px(6.0))
            .px(px(8.0))
            .rounded(px(6.0))
            .text_size(crate::typography::ui_rems(12.0))
            .font_weight(gpui::FontWeight::MEDIUM)
            .text_color(theme.text_muted.opacity(0.6))
            .hover(|s| s.bg(crate::theme::ink(0.05)))
            .child(
                crate::icons::icon(crate::icons::SPEEDOMETER)
                    .size(px(12.0))
                    .text_color(theme.text_muted.opacity(0.6)),
            )
            .child(SharedString::from(label))
            .when_some(cache, |el, cache| {
                el.child(
                    div()
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap(px(4.0))
                        .text_color(cache_color)
                        .child("·")
                        .child(
                            crate::icons::icon(crate::icons::CLOCK_CIRCLE)
                                .size(px(11.0))
                                .text_color(cache_color),
                        )
                        .child(SharedString::from(cache.short())),
                )
            })
            .tooltip(move |_, cx| cx.new(|cx| StatsCard::new(stats, working, cx)).into()),
    )
}

fn chip_label(stats: &SessionStats) -> Option<String> {
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

fn duration(ms: u64) -> String {
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

fn rows(stats: &SessionStats, cache: Option<Cache>) -> Vec<(&'static str, String)> {
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

fn tokens(n: u64) -> String {
    let trim = |s: String| {
        if s.contains('.') {
            s.trim_end_matches('0').trim_end_matches('.').to_string()
        } else {
            s
        }
    };
    let n = n as f64;
    if n >= 1e7 {
        format!("{}M", trim(format!("{:.1}", n / 1e6)))
    } else if n >= 999_500.0 {
        format!("{}M", trim(format!("{:.2}", n / 1e6)))
    } else if n >= 1e5 {
        format!("{:.0}K", n / 1e3)
    } else if n >= 1e3 {
        format!("{}K", trim(format!("{:.1}", n / 1e3)))
    } else {
        format!("{n}")
    }
}

fn usd(x: f64) -> String {
    if x < 1.0 {
        format!("${x:.3}")
    } else {
        format!("${x:.2}")
    }
}

fn cost_headline(stats: &SessionStats) -> Option<String> {
    match stats.cost_usd() {
        Some(cost) if stats.unpriced_turns > 0 => Some(format!("≥ {}", usd(cost))),
        Some(cost) if stats.cost_estimated => Some(format!("≈ {}", usd(cost))),
        Some(cost) => Some(usd(cost)),
        None if stats.total_tokens() > 0 => Some("No known price".into()),
        None => None,
    }
}

fn cost_note(stats: &SessionStats) -> Option<String> {
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

fn token_split(stats: &SessionStats) -> Vec<(&'static str, u64)> {
    let mut split = vec![("Input", stats.input_tokens)];
    if stats.cache_read_tokens + stats.cache_write_tokens > 0 {
        split.push(("Cache read", stats.cache_read_tokens));
        split.push(("Cache write", stats.cache_write_tokens));
    }
    split.push(("Output", stats.output_tokens));
    split
}

fn cost_section(stats: &SessionStats, theme: &Theme) -> Vec<gpui::AnyElement> {
    let Some(headline) = cost_headline(stats) else {
        return Vec::new();
    };
    let priced = stats.cost_usd().is_some();
    let total = stats.total_tokens();
    let split: Vec<_> = token_split(stats)
        .into_iter()
        .map(|(label, n)| {
            let color = match label {
                "Input" => theme.accent,
                "Cache read" => theme.success,
                "Cache write" => theme.warning,
                _ => theme.text_muted,
            };
            (label, n, color)
        })
        .collect();
    let hero = div()
        .px(px(8.0))
        .pb(px(6.0))
        .flex()
        .flex_row()
        .items_center()
        .justify_between()
        .child(
            div()
                .text_size(crate::typography::ui_rems(if priced { 20.0 } else { 15.0 }))
                .font_weight(gpui::FontWeight::MEDIUM)
                .text_color(if priced { theme.text } else { theme.text_faint })
                .child(SharedString::from(headline)),
        )
        .child(
            div()
                .text_size(crate::typography::ui_rems(11.0))
                .text_color(theme.text_muted)
                .child(SharedString::from(format!("{} tokens", tokens(total)))),
        );
    let bar = div()
        .mx(px(8.0))
        .h(px(6.0))
        .rounded(px(3.0))
        .overflow_hidden()
        .flex()
        .flex_row()
        .bg(crate::theme::ink(0.08))
        .children(
            split
                .iter()
                .filter(|(_, n, _)| *n > 0)
                .map(|(_, n, color)| {
                    div()
                        .h_full()
                        .min_w(px(2.0))
                        .w(gpui::relative(*n as f32 / total.max(1) as f32))
                        .bg(*color)
                }),
        );
    let legend = split.chunks(2).map(|pair| {
        div()
            .px(px(8.0))
            .flex()
            .flex_row()
            .gap(px(12.0))
            .children(pair.iter().map(|(label, n, color)| {
                div()
                    .flex_1()
                    .h(px(20.0))
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(6.0))
                    .text_size(crate::typography::ui_rems(11.5))
                    .text_color(theme.text_muted)
                    .child(div().size(px(7.0)).rounded(px(2.0)).bg(*color))
                    .child(*label)
                    .child(div().flex_1())
                    .child(
                        div()
                            .text_color(theme.text)
                            .child(SharedString::from(tokens(*n))),
                    )
            }))
            .into_any_element()
    });
    let mut section = vec![
        crate::popover::menu_heading(theme, "Session cost").into_any_element(),
        hero.into_any_element(),
    ];
    if total > 0 {
        section.push(bar.into_any_element());
        section.push(div().h(px(4.0)).into_any_element());
        section.extend(legend);
    }
    if let Some(note) = cost_note(stats) {
        section.push(
            div()
                .px(px(8.0))
                .pt(px(4.0))
                .text_size(crate::typography::ui_rems(11.0))
                .text_color(theme.text_faint)
                .child(SharedString::from(note))
                .into_any_element(),
        );
    }
    section.push(crate::popover::menu_separator().into_any_element());
    section
}

struct StatsCard {
    stats: SessionStats,
    working: bool,
    _tick: Task<()>,
}

impl StatsCard {
    fn new(stats: SessionStats, working: bool, cx: &mut Context<Self>) -> Self {
        let tick = tick(1, cx, |_, cx| cx.notify());
        Self {
            stats,
            working,
            _tick: tick,
        }
    }
}

impl Render for StatsCard {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::of(cx).for_popup();
        let cache = Cache::of(&self.stats, self.working, now_ms());
        let card = crate::popover::popover_card(&theme)
            .w(px(300.0))
            .flex()
            .flex_col()
            .children(cost_section(&self.stats, &theme))
            .child(crate::popover::menu_heading(&theme, "Session statistics"))
            .children(rows(&self.stats, cache).into_iter().map(|(label, value)| {
                div()
                    .px(px(8.0))
                    .h(px(24.0))
                    .flex()
                    .flex_row()
                    .items_center()
                    .justify_between()
                    .gap(px(12.0))
                    .text_size(crate::typography::ui_rems(12.0))
                    .text_color(theme.text_muted)
                    .child(label)
                    .child(
                        div()
                            .text_color(theme.text)
                            .child(SharedString::from(value)),
                    )
            }))
            .child(div().h(px(4.0)));
        crate::frost::frosted(crate::popover::CARD_RADIUS, crate::frost::MENU_BLUR, card)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        let rows = rows(&stats, Some(warm));
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
    }

    #[test]
    fn unreported_figures_read_as_a_dash() {
        let values: Vec<String> = rows(&SessionStats::default(), None)
            .into_iter()
            .map(|(_, v)| v)
            .collect();
        assert_eq!(values, ["0.0s", "0.0s", "—", "—"]);
    }
}
