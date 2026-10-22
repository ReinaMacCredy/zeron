use crate::theme::Theme;
use gpui::{Context, IntoElement, SharedString, Task, Window, div, prelude::*, px};
use zeron_proto::SessionStats;
use zeron_proto::stats_view::{
    Cache, chip_label, cost_headline, cost_note, stats_rows, token_split,
};

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

pub(crate) fn tick<T: 'static>(
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
            .min_w_0()
            .overflow_hidden()
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
                    .flex_none()
                    .text_color(theme.text_muted.opacity(0.6)),
            )
            .child(div().min_w_0().truncate().child(SharedString::from(label)))
            .when_some(cache, |el, cache| {
                el.child(
                    div()
                        .flex_none()
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

pub(crate) use zeron_proto::usage::format_tokens as tokens;

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
            .children(stats_rows(&self.stats, cache).into_iter().map(|(label, value)| {
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

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CompactView {
    pub plan: Option<zeron_proto::CompactPlan>,
    pub compacting: bool,
    pub stats: Option<SessionStats>,
}

impl CompactView {
    pub fn shown(&self) -> bool {
        self.compacting || self.plan.is_some()
    }
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

fn compact_rows(view: &CompactView, now_ms: i64) -> Vec<(&'static str, String)> {
    let mut rows = Vec::new();
    if let Some(cache) = view
        .stats
        .and_then(|stats| Cache::of(&stats, false, now_ms))
    {
        rows.push(("Prompt cache", cache.long()));
    }
    match view.plan {
        Some(zeron_proto::CompactPlan::Idle { at }) => {
            rows.push(("Compacts at", clock(at)));
        }
        Some(zeron_proto::CompactPlan::Agent) => {
            rows.push(("Compacts", "when this turn ends".into()));
        }
        None => {}
    }
    rows
}

pub fn compact_section(view: &CompactView, theme: &Theme) -> gpui::Div {
    let (heading, note) = match view.plan {
        _ if view.compacting => ("Compacting", None),
        Some(zeron_proto::CompactPlan::Agent) => (
            "Agent compaction",
            Some("The agent asked to compact. It runs as soon as the turn ends."),
        ),
        _ => (
            "Idle compaction",
            Some("Sending a message first cancels it."),
        ),
    };
    div()
        .w(px(280.0))
        .flex()
        .flex_col()
        .child(crate::popover::menu_heading(theme, heading))
        .children(
            compact_rows(view, now_ms())
                .into_iter()
                .map(|(label, value)| {
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
                }),
        )
        .when_some(note, |el, note| {
            el.child(
                div()
                    .px(px(8.0))
                    .pt(px(4.0))
                    .pb(px(8.0))
                    .text_size(crate::typography::ui_rems(12.0))
                    .text_color(theme.text_faint)
                    .child(note),
            )
        })
}
