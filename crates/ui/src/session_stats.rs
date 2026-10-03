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

pub struct StatsChip {
    stats: Option<SessionStats>,
    working: bool,
    _tick: Task<()>,
}

impl StatsChip {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let tick = cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(std::time::Duration::from_secs(10))
                    .await;
                if this
                    .update(cx, |chip, cx| {
                        if chip.stats.is_some() {
                            cx.notify();
                        }
                    })
                    .is_err()
                {
                    break;
                }
            }
        });
        Self {
            stats: None,
            working: false,
            _tick: tick,
        }
    }

    pub fn set(&mut self, stats: Option<SessionStats>, working: bool, cx: &mut Context<Self>) {
        if (self.stats, self.working) != (stats, working) {
            self.stats = stats;
            self.working = working;
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
    if ms < 60_000 {
        format!("{:.1}s", ms as f64 / 1000.0)
    } else {
        let secs = ms / 1000;
        format!("{}m{:02}s", secs / 60, secs % 60)
    }
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

struct StatsCard {
    stats: SessionStats,
    working: bool,
    _tick: Task<()>,
}

impl StatsCard {
    fn new(stats: SessionStats, working: bool, cx: &mut Context<Self>) -> Self {
        let tick = cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(std::time::Duration::from_secs(1))
                    .await;
                if this.update(cx, |_, cx| cx.notify()).is_err() {
                    break;
                }
            }
        });
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
            .w(px(280.0))
            .flex()
            .flex_col()
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
    fn durations_switch_to_minutes_past_one() {
        assert_eq!(duration(13_400), "13.4s");
        assert_eq!(duration(558 * 60_000 + 41_000), "558m41s");
        assert_eq!(duration(60_000), "1m00s");
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
