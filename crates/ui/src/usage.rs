use std::cell::Cell;
use std::rc::Rc;

use chrono::{Local, TimeZone};
use gpui::{
    AnyElement, Bounds, Context, Entity, Hsla, IntoElement, MouseMoveEvent, PathBuilder, Pixels,
    Render, SharedString, Task, Window, canvas, div, point, prelude::*, px, relative,
};
use zeron_proto::HarnessId;
use zeron_proto::usage::{
    ModelTotal, PeriodTotal, USAGE_WINDOWS as WINDOWS, UsageSummary, UsageView as View,
    format_tokens, format_usd, percent, plural,
};
use zeron_rpc::methods;

use crate::icons::{self, icon};
use crate::settings::widgets;
use crate::state::AppState;
use crate::theme::Theme;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Metric {
    Cost,
    Tokens,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Breakdown {
    Model,
    Period,
}

const CHART_HEIGHT: f32 = 224.0;
const PLOT_TOP: f32 = 8.0;
const AXIS_WIDTH: f32 = 56.0;
const SUMMARY_WIDTH: f32 = 288.0;

pub struct UsagePage {
    state: Entity<AppState>,
    days: u32,
    metric: Metric,
    breakdown: Breakdown,
    summary: Option<UsageSummary>,
    error: Option<SharedString>,
    task: Option<Task<()>>,
    hover: Option<usize>,
    plot: Rc<Cell<Option<Bounds<Pixels>>>>,
    details_open: std::collections::HashSet<String>,
    _accounts: gpui::Subscription,
}

impl UsagePage {
    pub fn new(state: Entity<AppState>, cx: &mut Context<Self>) -> Self {
        let mut page = Self {
            state,
            days: 30,
            metric: Metric::Cost,
            breakdown: Breakdown::Model,
            summary: None,
            error: None,
            task: None,
            hover: None,
            plot: Rc::default(),
            details_open: Default::default(),
            _accounts: cx.observe_global::<crate::settings::accounts::AccountsSnapshotCache>(
                |_, cx| cx.notify(),
            ),
        };
        page.load(cx);
        page
    }

    pub fn load(&mut self, cx: &mut Context<Self>) {
        let Some(engine) = self.state.read(cx).engine().cloned() else {
            self.error = Some("The engine is not connected.".into());
            cx.notify();
            return;
        };
        let params = serde_json::json!({ "days": self.days });
        self.task = Some(cx.spawn(async move |this, cx| {
            let result = engine
                .client()
                .call_as::<UsageSummary>(methods::GET_USAGE_SUMMARY, params)
                .await;
            this.update(cx, |page, cx| {
                page.task = None;
                match result {
                    Ok(summary) => {
                        page.summary = Some(summary);
                        page.error = None;
                    }
                    Err(err) => page.error = Some(err.to_string().into()),
                }
                page.hover = None;
                cx.notify();
            })
            .ok();
        }));
        cx.notify();
    }

    fn select_window(&mut self, days: u32, cx: &mut Context<Self>) {
        if self.days != days {
            self.days = days;
            self.load(cx);
        }
    }

    fn track_hover(&mut self, event: &MouseMoveEvent, cx: &mut Context<Self>) {
        let periods = self.summary.as_ref().map_or(0, |s| s.periods.len());
        let hover = self.plot.get().filter(|_| periods > 1).and_then(|bounds| {
            if !bounds.contains(&event.position) {
                return None;
            }
            let x = f32::from(event.position.x - bounds.origin.x) / f32::from(bounds.size.width);
            Some(((x * (periods - 1) as f32).round() as usize).min(periods - 1))
        });
        if hover != self.hover {
            self.hover = hover;
            cx.notify();
        }
    }
}

fn value(metric: Metric, cost: f64, tokens: u64) -> f64 {
    match metric {
        Metric::Cost => cost,
        Metric::Tokens => tokens as f64,
    }
}

fn format_value(metric: Metric, value: f64) -> String {
    match metric {
        Metric::Cost => format_usd(value),
        Metric::Tokens => format_tokens(value.round() as u64),
    }
}

fn nice_scale(peak: f64, target_ticks: usize) -> (f64, Vec<f64>) {
    if peak <= 0.0 || !peak.is_finite() {
        return (1.0, vec![0.0]);
    }
    let raw = peak / target_ticks as f64;
    let magnitude = 10f64.powf(raw.log10().floor());
    let step = [1.0, 2.0, 5.0, 10.0]
        .into_iter()
        .map(|m| m * magnitude)
        .find(|&step| step >= raw)
        .unwrap_or(10.0 * magnitude);
    let max = (peak / step).ceil() * step;
    let count = (max / step).round() as usize;
    (max, (0..=count).map(|i| i as f64 * step).collect())
}

fn monotone_tangents(ys: &[f32], dx: f32) -> Vec<f32> {
    let n = ys.len();
    let deltas: Vec<f32> = ys.windows(2).map(|w| (w[1] - w[0]) / dx).collect();
    let mut m = vec![0.0; n];
    m[0] = deltas[0];
    m[n - 1] = deltas[n - 2];
    for i in 1..n - 1 {
        m[i] = if deltas[i - 1] * deltas[i] <= 0.0 {
            0.0
        } else {
            (deltas[i - 1] + deltas[i]) / 2.0
        };
    }
    for i in 0..n - 1 {
        if deltas[i] == 0.0 {
            m[i] = 0.0;
            m[i + 1] = 0.0;
            continue;
        }
        let a = m[i] / deltas[i];
        let b = m[i + 1] / deltas[i];
        let s = a * a + b * b;
        if s > 9.0 {
            let t = 3.0 / s.sqrt();
            m[i] = t * a * deltas[i];
            m[i + 1] = t * b * deltas[i];
        }
    }
    m
}

fn provider_color(harness: HarnessId, theme: &Theme) -> Hsla {
    match harness {
        HarnessId::ClaudeCode => icons::claude_brand(),
        HarnessId::Grok => crate::motion::mix(theme.bg, theme.text, 0.72),
        _ => theme.text,
    }
}

fn provider_mark(harness: HarnessId, size: f32, theme: &Theme) -> impl IntoElement {
    let (path, tint) = crate::pickers::harness_brand_icon(harness);
    icon(path)
        .size(px(size))
        .flex_none()
        .text_color(tint.unwrap_or(theme.text))
}

fn period_label(start: i64, hourly: bool) -> String {
    let Some(time) = Local.timestamp_millis_opt(start).single() else {
        return String::new();
    };
    if hourly {
        time.format("%-I %p").to_string()
    } else {
        time.format("%b %-d").to_string()
    }
}

fn period_title(start: i64, hourly: bool) -> String {
    let Some(time) = Local.timestamp_millis_opt(start).single() else {
        return String::new();
    };
    if hourly {
        time.format("%b %-d, %-I %p").to_string()
    } else {
        time.format("%a, %b %-d").to_string()
    }
}

fn muted(theme: &Theme, size: f32, text: impl Into<SharedString>) -> gpui::Div {
    div()
        .text_size(crate::typography::ui_rems(size))
        .text_color(theme.text_muted)
        .child(text.into())
}

fn heading(theme: &Theme, text: impl Into<SharedString>) -> gpui::Div {
    div()
        .text_size(crate::typography::ui_rems(13.0))
        .font_weight(gpui::FontWeight::MEDIUM)
        .text_color(theme.text)
        .child(text.into())
}

impl UsagePage {
    fn segmented<T: Copy + PartialEq + 'static>(
        &self,
        id: &'static str,
        options: &[(T, &'static str)],
        selected: T,
        enabled: bool,
        theme: &Theme,
        window: &mut Window,
        cx: &mut Context<Self>,
        select: fn(&mut Self, T, &mut Context<Self>),
    ) -> AnyElement {
        let reduced_motion = crate::motion::reduced_motion(cx);
        let tabs = options.iter().map(|&(value, label)| {
            let on = value == selected;
            let key = format!("{id}-{label}");
            let t =
                widgets::tab_selection_t(window, format!("{key}-selection"), on, reduced_motion);
            widgets::section_tab(theme, on, t, SharedString::from(key.clone()), key)
                .min_h(px(26.0))
                .py(px(3.0))
                .text_size(crate::typography::ui_rems(12.0))
                .role(gpui::Role::Tab)
                .aria_label(label)
                .when(enabled, |tab| {
                    tab.cursor_pointer()
                        .on_click(cx.listener(move |this, _, _, cx| select(this, value, cx)))
                })
                .child(label)
        });
        div()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(2.0))
            .p(px(2.0))
            .rounded(px(10.0))
            .bg(crate::theme::wash(0.04))
            .when(!enabled, |el| el.opacity(0.5))
            .children(tabs)
            .into_any_element()
    }

    fn render_header(
        &self,
        theme: &Theme,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let loading = self.task.is_some();
        let windows: Vec<(u32, &'static str)> = WINDOWS.to_vec();
        div()
            .flex()
            .flex_row()
            .flex_wrap()
            .items_center()
            .justify_between()
            .gap(px(12.0))
            .child(heading(theme, "Token usage"))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(8.0))
                    .child(self.segmented(
                        "usage-metric",
                        &[(Metric::Cost, "Cost"), (Metric::Tokens, "Tokens")],
                        self.metric,
                        true,
                        theme,
                        window,
                        cx,
                        |this, metric, cx| {
                            this.metric = metric;
                            cx.notify();
                        },
                    ))
                    .child(self.segmented(
                        "usage-window",
                        &windows,
                        self.days,
                        true,
                        theme,
                        window,
                        cx,
                        |this, days, cx| this.select_window(days, cx),
                    ))
                    .child(
                        div()
                            .id("usage-refresh")
                            .role(gpui::Role::Button)
                            .aria_label("Refresh usage")
                            .size(px(28.0))
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded(px(8.0))
                            .text_color(theme.text_muted)
                            .hover(|s| s.bg(theme.glass_hover()))
                            .cursor_pointer()
                            .when(loading, |el| el.opacity(0.5))
                            .on_click(cx.listener(|this, _, _, cx| {
                                if this.task.is_none() {
                                    this.load(cx);
                                }
                            }))
                            .tooltip(widgets::text_tooltip("Refresh usage"))
                            .child(
                                icon(icons::REFRESH)
                                    .size(px(14.0))
                                    .text_color(theme.text_muted),
                            ),
                    ),
            )
            .into_any_element()
    }

    fn render_summary(&self, view: &View, active: &[usize], theme: &Theme) -> AnyElement {
        let metric = self.metric;
        let headline = match metric {
            Metric::Cost => format_usd(view.cost),
            Metric::Tokens => format_tokens(view.tokens.total()),
        };
        let sessions = plural(view.sessions, "session");
        let caption = match metric {
            Metric::Cost => format!("{sessions} · API estimate"),
            Metric::Tokens => sessions,
        };
        let note = (metric == Metric::Cost && view.unpriced > 0).then(|| {
            format!(
                "Excludes {} on models with no known price",
                plural(view.unpriced, "response")
            )
        });
        let rows = active.iter().map(|&i| {
            let provider = &view.providers[i];
            let (own, total) = match metric {
                Metric::Cost => (provider.cost, view.cost),
                Metric::Tokens => (provider.tokens as f64, view.tokens.total() as f64),
            };
            let detail = match metric {
                Metric::Cost => format!(
                    "{} of cost · {} tokens",
                    percent(own, total),
                    format_tokens(provider.tokens)
                ),
                Metric::Tokens => format!(
                    "{} of tokens · {}",
                    percent(own, total),
                    format_usd(provider.cost)
                ),
            };
            div()
                .flex()
                .flex_col()
                .gap(px(2.0))
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .items_center()
                        .justify_between()
                        .gap(px(12.0))
                        .child(
                            div()
                                .flex()
                                .flex_row()
                                .items_center()
                                .gap(px(8.0))
                                .min_w_0()
                                .child(
                                    div()
                                        .size(px(8.0))
                                        .flex_none()
                                        .rounded_full()
                                        .bg(provider_color(provider.harness, theme)),
                                )
                                .child(provider_mark(provider.harness, 16.0, theme))
                                .child(
                                    div()
                                        .text_size(crate::typography::ui_rems(13.0))
                                        .text_color(theme.text)
                                        .truncate()
                                        .child(provider.harness.display_name()),
                                )
                                .child(muted(theme, 11.0, plural(provider.sessions, "session"))),
                        )
                        .child(
                            div()
                                .flex_none()
                                .text_size(crate::typography::ui_rems(13.0))
                                .font_weight(gpui::FontWeight::MEDIUM)
                                .text_color(theme.text)
                                .child(format_value(metric, own)),
                        ),
                )
                .child(muted(theme, 12.0, detail))
        });
        div()
            .w(px(SUMMARY_WIDTH))
            .flex_none()
            .flex()
            .flex_col()
            .gap(px(20.0))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(4.0))
                    .child(
                        div()
                            .text_size(crate::typography::ui_rems(36.0))
                            .line_height(crate::typography::ui_rems(42.0))
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .text_color(theme.text)
                            .child(headline),
                    )
                    .child(muted(theme, 12.0, caption))
                    .children(note.map(|note| muted(theme, 12.0, note))),
            )
            .children(rows)
            .into_any_element()
    }

    fn render_chart(
        &self,
        view: &View,
        active: &[usize],
        hourly: bool,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let metric = self.metric;
        let series: Vec<(Hsla, Vec<f64>)> = active
            .iter()
            .map(|&i| {
                let values = view
                    .periods
                    .iter()
                    .map(|p| value(metric, p.by_provider[i].0, p.by_provider[i].1))
                    .collect();
                (provider_color(view.providers[i].harness, theme), values)
            })
            .collect();
        let peak = series
            .iter()
            .flat_map(|(_, values)| values.iter().copied())
            .fold(0.0, f64::max);
        let (axis_max, ticks) = nice_scale(peak, 4);
        let y_of = move |v: f64| CHART_HEIGHT - (v / axis_max) as f32 * (CHART_HEIGHT - PLOT_TOP);

        let labels = ticks.iter().map(|&tick| {
            let text = if tick == 0.0 {
                "0".to_string()
            } else {
                format_value(metric, tick)
            };
            div()
                .absolute()
                .right(px(10.0))
                .top(px(y_of(tick) - 7.0))
                .text_size(px(10.0))
                .line_height(px(14.0))
                .text_color(theme.text_faint)
                .child(text)
        });

        let mut ordered: Vec<(Hsla, Vec<f64>)> = series.clone();
        ordered.sort_by(|a, b| {
            let sum = |v: &Vec<f64>| v.iter().sum::<f64>();
            sum(&b.1).total_cmp(&sum(&a.1))
        });
        let grid_color = theme.border;
        let hover_color = theme.text_faint.opacity(0.6);
        let hover = self.hover;
        let plot_cell = self.plot.clone();
        let tick_ys: Vec<f32> = ticks.iter().map(|&t| y_of(t)).collect();
        let plot = canvas(
            move |bounds, _, _| plot_cell.set(Some(bounds)),
            move |bounds, _, window, _| {
                let width = f32::from(bounds.size.width);
                let height = f32::from(bounds.size.height);
                let origin = bounds.origin;
                let at = |x: f32, y: f32| {
                    point(origin.x + px(x), origin.y + px(y * height / CHART_HEIGHT))
                };
                for &y in &tick_ys {
                    let mut line = PathBuilder::stroke(px(1.0));
                    line.move_to(at(0.0, y));
                    line.line_to(at(width, y));
                    if let Ok(path) = line.build() {
                        window.paint_path(path, grid_color);
                    }
                }
                let n = ordered.first().map_or(0, |(_, v)| v.len());
                if n < 2 {
                    return;
                }
                let dx = width / (n - 1) as f32;
                let curve = |path: &mut PathBuilder, ys: &[f32], tangents: &[f32]| {
                    for i in 0..n - 1 {
                        let (x0, x1) = (i as f32 * dx, (i + 1) as f32 * dx);
                        path.cubic_bezier_to(
                            at(x1, ys[i + 1]),
                            at(x0 + dx / 3.0, ys[i] + tangents[i] * dx / 3.0),
                            at(x1 - dx / 3.0, ys[i + 1] - tangents[i + 1] * dx / 3.0),
                        );
                    }
                };
                let shapes: Vec<(Hsla, Vec<f32>, Vec<f32>)> = ordered
                    .iter()
                    .map(|(color, values)| {
                        let ys: Vec<f32> = values.iter().map(|&v| y_of(v)).collect();
                        let tangents = monotone_tangents(&ys, dx);
                        (*color, ys, tangents)
                    })
                    .collect();
                for (color, ys, tangents) in &shapes {
                    let mut area = PathBuilder::fill();
                    area.move_to(at(0.0, CHART_HEIGHT));
                    area.line_to(at(0.0, ys[0]));
                    curve(&mut area, ys, tangents);
                    area.line_to(at(width, CHART_HEIGHT));
                    area.close();
                    if let Ok(path) = area.build() {
                        window.paint_path(path, color.opacity(0.12));
                    }
                }
                for (color, ys, tangents) in &shapes {
                    let mut line = PathBuilder::stroke(px(2.0));
                    line.move_to(at(0.0, ys[0]));
                    curve(&mut line, ys, tangents);
                    if let Ok(path) = line.build() {
                        window.paint_path(path, *color);
                    }
                }
                if let Some(index) = hover {
                    let x = index as f32 * dx;
                    let mut line = PathBuilder::stroke(px(1.0));
                    line.move_to(at(x, PLOT_TOP));
                    line.line_to(at(x, CHART_HEIGHT));
                    if let Ok(path) = line.build() {
                        window.paint_path(path, hover_color);
                    }
                }
            },
        )
        .size_full();

        let tooltip = hover
            .zip(self.plot.get())
            .and_then(|(index, bounds)| Some((index, bounds, view.periods.get(index)?)))
            .map(|(index, bounds, period)| {
                let width = f32::from(bounds.size.width);
                let n = view.periods.len().max(2);
                let x = index as f32 * width / (n - 1) as f32;
                let rows = active.iter().map(|&i| {
                    let (cost, tokens) = period.by_provider[i];
                    let harness = view.providers[i].harness;
                    div()
                        .flex()
                        .flex_row()
                        .items_center()
                        .justify_between()
                        .gap(px(16.0))
                        .child(
                            div()
                                .flex()
                                .flex_row()
                                .items_center()
                                .gap(px(6.0))
                                .child(provider_mark(harness, 12.0, theme))
                                .child(harness.display_name()),
                        )
                        .child(format_value(metric, value(metric, cost, tokens)))
                });
                let card = div()
                    .min_w(px(180.0))
                    .px(px(10.0))
                    .py(px(8.0))
                    .rounded(px(8.0))
                    .border_1()
                    .border_color(theme.border)
                    .bg(crate::popover::surface_bg(theme))
                    .text_size(px(11.0))
                    .text_color(theme.text)
                    .flex()
                    .flex_col()
                    .gap(px(4.0))
                    .child(
                        div()
                            .text_color(theme.text_muted)
                            .child(period_title(period.start, hourly)),
                    )
                    .children(rows)
                    .child(div().h(px(1.0)).bg(theme.border))
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .justify_between()
                            .child("Total")
                            .child(format_value(
                                metric,
                                value(metric, period.cost, period.tokens),
                            )),
                    );
                let card = crate::frost::frosted(8.0, crate::frost::MENU_BLUR, card);
                let anchor = div().absolute().top(px(PLOT_TOP));
                if x > width * 0.6 {
                    anchor.right(px(width - x + 12.0)).child(card)
                } else {
                    anchor.left(px(x + 12.0)).child(card)
                }
            });

        let first = view.periods.first().map(|p| p.start);
        let middle = view.periods.get(view.periods.len() / 2).map(|p| p.start);
        let last = view.periods.last().map(|p| p.start);
        let axis_label = |start: Option<i64>| {
            div()
                .text_size(px(10.0))
                .text_color(theme.text_faint)
                .child(
                    start
                        .map(|s| period_label(s, hourly).to_uppercase())
                        .unwrap_or_default(),
                )
        };
        let title = match (hourly, metric) {
            (true, Metric::Cost) => "Hourly cost",
            (true, Metric::Tokens) => "Hourly processed tokens",
            (false, Metric::Cost) => "Daily cost",
            (false, Metric::Tokens) => "Daily processed tokens",
        };
        div()
            .flex_1()
            .min_w_0()
            .flex()
            .flex_col()
            .gap(px(12.0))
            .child(heading(theme, title))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .child(
                        div()
                            .w(px(AXIS_WIDTH))
                            .flex_none()
                            .h(px(CHART_HEIGHT))
                            .relative()
                            .children(labels),
                    )
                    .child(
                        div()
                            .id("usage-chart")
                            .flex_1()
                            .min_w_0()
                            .h(px(CHART_HEIGHT))
                            .relative()
                            .on_mouse_move(cx.listener(|this, event: &MouseMoveEvent, _, cx| {
                                this.track_hover(event, cx)
                            }))
                            .on_hover(cx.listener(|this, hovered: &bool, _, cx| {
                                if !hovered && this.hover.take().is_some() {
                                    cx.notify();
                                }
                            }))
                            .child(plot)
                            .children(tooltip),
                    ),
            )
            .child(
                div()
                    .ml(px(AXIS_WIDTH))
                    .flex()
                    .flex_row()
                    .justify_between()
                    .child(axis_label(first))
                    .child(axis_label(middle))
                    .child(axis_label(last)),
            )
            .into_any_element()
    }

    fn render_totals(&self, view: &View, theme: &Theme) -> AnyElement {
        let metric = |label: &'static str, value: String| {
            div()
                .min_w(px(120.0))
                .flex_1()
                .flex()
                .flex_col()
                .gap(px(2.0))
                .child(muted(theme, 12.0, label))
                .child(
                    div()
                        .text_size(crate::typography::ui_rems(15.0))
                        .font_weight(gpui::FontWeight::MEDIUM)
                        .text_color(theme.text)
                        .child(value),
                )
        };
        div()
            .flex()
            .flex_col()
            .gap(px(8.0))
            .child(heading(theme, "Totals"))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .flex_wrap()
                    .gap(px(16.0))
                    .py(px(4.0))
                    .child(metric(
                        "Processed tokens",
                        format_tokens(view.tokens.total()),
                    ))
                    .child(metric(
                        "Cached input",
                        format_tokens(view.tokens.cached_input),
                    ))
                    .child(metric(
                        "Uncached input",
                        format_tokens(view.tokens.uncached_input + view.tokens.cache_write),
                    ))
                    .child(metric("Output", format_tokens(view.tokens.output)))
                    .child(metric("Cache savings", format_usd(view.savings))),
            )
            .into_any_element()
    }

    fn render_breakdown(
        &self,
        view: &View,
        active: &[usize],
        hourly: bool,
        theme: &Theme,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let row = |first: bool| {
            div()
                .flex()
                .flex_row()
                .items_center()
                .py(px(8.0))
                .border_b_1()
                .border_color(if first {
                    theme.border
                } else {
                    theme.border.opacity(0.5)
                })
                .text_size(crate::typography::ui_rems(13.0))
        };
        let cell = |width: f32, right: bool| {
            div()
                .w(relative(width))
                .min_w_0()
                .truncate()
                .when(right, |el| el.flex().justify_end())
        };
        let head = |labels: Vec<(String, f32, bool)>| {
            labels
                .into_iter()
                .fold(row(true), |row, (label, width, right)| {
                    row.child(
                        cell(width, right)
                            .text_size(crate::typography::ui_rems(12.0))
                            .text_color(theme.text_muted)
                            .child(label),
                    )
                })
        };
        let empty = || {
            div().py(px(24.0)).flex().justify_center().child(muted(
                theme,
                13.0,
                "No activity in this window.",
            ))
        };
        let mut table = div().flex().flex_col();
        match self.breakdown {
            Breakdown::Model => {
                table = table.child(head(vec![
                    ("Model".into(), 0.4, false),
                    ("Cost".into(), 0.2, true),
                    ("Share".into(), 0.2, true),
                    ("Tokens".into(), 0.2, true),
                ]));
                let mut models: Vec<&ModelTotal> = view.models.iter().collect();
                if self.metric == Metric::Tokens {
                    models.sort_by(|a, b| b.tokens.cmp(&a.tokens).then(b.cost.total_cmp(&a.cost)));
                }
                if models.is_empty() {
                    table = table.child(empty());
                }
                for model in models {
                    table = table.child(
                        row(false)
                            .hover(|s| s.bg(theme.glass_hover()))
                            .child(
                                cell(0.4, false).child(
                                    div()
                                        .flex()
                                        .flex_row()
                                        .items_center()
                                        .gap(px(8.0))
                                        .min_w_0()
                                        .text_color(theme.text)
                                        .child(provider_mark(model.harness, 14.0, theme))
                                        .child(div().truncate().child(model.model.clone())),
                                ),
                            )
                            .child(
                                cell(0.2, true)
                                    .text_color(theme.text)
                                    .child(format_usd(model.cost)),
                            )
                            .child(
                                cell(0.2, true)
                                    .text_color(theme.text_muted)
                                    .child(percent(model.cost, view.cost)),
                            )
                            .child(
                                cell(0.2, true)
                                    .text_color(theme.text_muted)
                                    .child(format_tokens(model.tokens)),
                            ),
                    );
                }
            }
            Breakdown::Period => {
                let column = 0.6 / (active.len() + 2) as f32;
                let mut labels =
                    vec![(if hourly { "Hour" } else { "Day" }.to_string(), 0.4, false)];
                labels.extend(active.iter().map(|&i| {
                    (
                        view.providers[i].harness.display_name().to_string(),
                        column,
                        true,
                    )
                }));
                labels.push(("Total".into(), column, true));
                labels.push(("Tokens".into(), column, true));
                table = table.child(head(labels));
                let periods: Vec<&PeriodTotal> = view
                    .periods
                    .iter()
                    .rev()
                    .filter(|p| p.tokens > 0 || p.cost > 0.0)
                    .collect();
                if periods.is_empty() {
                    table = table.child(empty());
                }
                for period in periods {
                    let mut line = row(false).hover(|s| s.bg(theme.glass_hover())).child(
                        cell(0.4, false)
                            .text_color(theme.text)
                            .child(period_title(period.start, hourly)),
                    );
                    for &i in active {
                        line = line.child(
                            cell(column, true)
                                .text_color(theme.text_muted)
                                .child(format_usd(period.by_provider[i].0)),
                        );
                    }
                    table = table.child(
                        line.child(
                            cell(column, true)
                                .text_color(theme.text)
                                .child(format_usd(period.cost)),
                        )
                        .child(
                            cell(column, true)
                                .text_color(theme.text_muted)
                                .child(format_tokens(period.tokens)),
                        ),
                    );
                }
            }
        }
        let period_label = if hourly { "Hour" } else { "Day" };
        div()
            .flex()
            .flex_col()
            .gap(px(12.0))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .justify_between()
                    .child(heading(theme, "Breakdown"))
                    .child(self.segmented(
                        "usage-breakdown",
                        &[
                            (Breakdown::Model, "Model"),
                            (Breakdown::Period, period_label),
                        ],
                        self.breakdown,
                        true,
                        theme,
                        window,
                        cx,
                        |this, breakdown, cx| {
                            this.breakdown = breakdown;
                            cx.notify();
                        },
                    )),
            )
            .child(table)
            .into_any_element()
    }
}

impl UsagePage {
    fn render_plan_limits(&self, theme: &Theme, cx: &mut Context<Self>) -> Option<AnyElement> {
        let snapshot = crate::account_usage::local_snapshot(cx)?;
        let accounts: Vec<zeron_proto::AgentAccount> =
            crate::settings::accounts::sidebar_accounts(snapshot)
                .into_iter()
                .filter(|account| crate::settings::accounts::reports_usage(account.harness))
                .cloned()
                .collect();
        if accounts.is_empty() {
            return None;
        }
        let now = chrono::Utc::now();
        let updated = accounts
            .iter()
            .filter_map(|account| account.usage_fetched_at)
            .max()
            .and_then(chrono::DateTime::<chrono::Utc>::from_timestamp_millis)
            .map(|at| crate::pull_requests::relative_time(at, now));
        let note = match updated {
            Some(when) => format!("Updated {when} · refreshes after each turn"),
            None => "Refreshes after each turn".to_owned(),
        };
        let mut cards: Vec<AnyElement> = accounts
            .iter()
            .enumerate()
            .map(|(ix, account)| self.plan_card(ix, account, now, theme, cx))
            .collect();
        if cards.len() % 2 == 1 {
            cards.push(div().flex_1().min_w_0().into_any_element());
        }
        let mut rows = Vec::new();
        let mut cards = cards.into_iter();
        while let (Some(left), Some(right)) = (cards.next(), cards.next()) {
            rows.push(
                div()
                    .flex()
                    .flex_row()
                    .items_start()
                    .gap(px(16.0))
                    .child(left)
                    .child(right),
            );
        }
        Some(
            div()
                .flex()
                .flex_col()
                .gap(px(12.0))
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .items_baseline()
                        .justify_between()
                        .gap(px(12.0))
                        .child(heading(theme, "Plan limits"))
                        .child(muted(theme, 12.0, note)),
                )
                .children(rows)
                .into_any_element(),
        )
    }

    fn plan_card(
        &self,
        ix: usize,
        account: &zeron_proto::AgentAccount,
        now: chrono::DateTime<chrono::Utc>,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let id = account.id.clone();
        let open = self.details_open.contains(&id);
        let toggle = cx.listener(move |this, _, _, cx| {
            if !this.details_open.remove(&id) {
                this.details_open.insert(id.clone());
            }
            cx.notify();
        });
        div()
            .flex_1()
            .min_w_0()
            .p(px(14.0))
            .rounded(px(12.0))
            .border_1()
            .border_color(theme.text.opacity(0.08))
            .bg(crate::theme::wash(0.03))
            .flex()
            .flex_col()
            .gap(px(12.0))
            .text_size(crate::typography::ui_rems(11.5))
            .line_height(px(16.0))
            .text_color(theme.text)
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(8.0))
                    .child(provider_mark(account.harness, 14.0, theme))
                    .child(
                        div()
                            .flex_none()
                            .text_size(crate::typography::ui_rems(13.0))
                            .font_weight(gpui::FontWeight::MEDIUM)
                            .child(crate::settings::accounts::provider_name(account.harness)),
                    )
                    .children(account.plan_label.clone().map(|plan| {
                        div()
                            .flex_none()
                            .text_size(crate::typography::ui_rems(12.0))
                            .text_color(theme.text_muted)
                            .child(SharedString::from(plan))
                    }))
                    .children(account.email.clone().map(|email| {
                        div()
                            .ml_auto()
                            .min_w_0()
                            .truncate()
                            .text_color(theme.text_faint)
                            .child(SharedString::from(email))
                    })),
            )
            .children(account.usage_error.clone().map(|error| {
                div()
                    .text_color(theme.warning)
                    .child(SharedString::from(error))
            }))
            .children(crate::app_rail::usage_blocks(account, now, theme))
            .child(crate::app_rail::details_fold(
                ("usage-plan-details", ix).into(),
                format!("usage-plan-details-{ix}"),
                account,
                open,
                toggle,
                now,
                theme,
            ))
            .into_any_element()
    }
}

impl Render for UsagePage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::of(cx).clone();
        let header = self.render_header(&theme, window, cx);
        let plan_limits = self.render_plan_limits(&theme, cx);
        let mut column = widgets::page_column()
            .max_w(px(1080.0))
            .gap(px(28.0))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .child(widgets::page_header(&theme, "Usage", None))
                    .child(widgets::page_subtitle(
                        &theme,
                        "Plan limits from your agent accounts, then token spend from chats on this device.",
                    )),
            )
            .children(plan_limits)
            .child(header);
        match (&self.summary, &self.error) {
            (_, Some(error)) => {
                column = column.child(widgets::error_strip(&theme, error.clone()));
            }
            (None, None) => {
                column = column.child(muted(&theme, 13.0, "Reading agent transcripts…"));
            }
            _ => {}
        }
        if let Some(summary) = self.summary.clone() {
            let view = View::new(&summary);
            let active = view.active();
            let hourly = summary.hourly;
            column = column
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .flex_wrap()
                        .gap(px(24.0))
                        .child(self.render_summary(&view, &active, &theme))
                        .child(self.render_chart(&view, &active, hourly, &theme, cx)),
                )
                .child(self.render_totals(&view, &theme))
                .child(self.render_breakdown(&view, &active, hourly, &theme, window, cx));
        }
        widgets::scroll_faded(
            "usage-page-scroll",
            div()
                .id("usage-page")
                .debug_selector(|| "usage-page".into())
                .size_full()
                .overflow_y_scroll()
                .child(column),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nice_scale_rounds_the_peak_up_to_a_readable_step() {
        assert_eq!(nice_scale(87.0, 4), (100.0, vec![0.0, 50.0, 100.0]));
        assert_eq!(nice_scale(3.2, 4).0, 4.0);
        assert_eq!(nice_scale(0.0, 4), (1.0, vec![0.0]));
    }
}
