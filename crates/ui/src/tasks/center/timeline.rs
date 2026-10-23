use gpui::{Bounds, canvas, point, size};
use zeron_proto::{TodayRunStatus, TodayRuns, TodayRunsRequest};

use super::*;
use crate::tasks::thread_tree::{NodeKind, TreeNode};
use crate::tasks::tree::{Status, StatusTone};

const AXIS: f32 = 30.0;
const BAR: f32 = 8.0;
const PLANNED_WIDTH: f32 = 56.0;
const MIN_BAR: f32 = 14.0;
const TICKS_S: [i64; 13] = [
    5, 10, 15, 30, 60, 120, 300, 600, 900, 1800, 3600, 7200, 14400,
];

#[derive(Default)]
pub(super) struct RunsCache {
    key: String,
    runs: TodayRuns,
    task: Option<gpui::Task<()>>,
    chart: std::rc::Rc<std::cell::Cell<Option<Bounds<Pixels>>>>,
}

#[derive(Clone, Copy)]
struct Palette {
    working: Hsla,
    done: Hsla,
    wait_fill: Hsla,
    wait_edge: Hsla,
    link: Hsla,
}

impl Palette {
    fn of(theme: &Theme) -> Self {
        let dark = theme.appearance == crate::theme::Appearance::Dark;
        let working = Status::Running.tone(theme);
        let done = Status::Done.tone(theme);
        Self {
            working,
            done: if dark {
                done.opacity(0.6)
            } else {
                motion::mix(done, theme.text, 0.22)
            },
            wait_fill: theme.warning.opacity(if dark { 0.5 } else { 0.32 }),
            wait_edge: theme.warning,
            link: working.opacity(if dark { 0.65 } else { 0.8 }),
        }
    }
}

struct Lane {
    y: f32,
    height: f32,
    node: usize,
}

fn tick_seconds(span_ms: i64) -> i64 {
    let span_s = span_ms / 1_000;
    TICKS_S
        .into_iter()
        .find(|step| span_s / step <= 8)
        .unwrap_or(28_800)
}

fn dashed(window: &mut Window, from: (f32, f32), to: (f32, f32), dash: f32, color: Hsla) {
    let half = 0.75;
    let length = (to.0 - from.0).hypot(to.1 - from.1);
    if length <= 0.0 {
        return;
    }
    let (dx, dy) = ((to.0 - from.0) / length, (to.1 - from.1) / length);
    let mut at = 0.0;
    while at < length {
        let end = (at + dash).min(length);
        let (x0, y0) = (from.0 + dx * at, from.1 + dy * at);
        let (x1, y1) = (from.0 + dx * end, from.1 + dy * end);
        let bounds = Bounds::from_corners(
            point(px(x0.min(x1) - half), px(y0.min(y1) - half)),
            point(px(x0.max(x1) + half), px(y0.max(y1) + half)),
        );
        window.paint_quad(gpui::fill(bounds, color));
        at += dash * 2.0;
    }
}

impl TaskCenter {
    fn refresh_runs(&mut self, nodes: &[TreeNode], since: DateTime<Utc>, cx: &mut Context<Self>) {
        let ids: Vec<String> = nodes
            .iter()
            .filter_map(|node| node.chat_id.clone())
            .collect();
        let key = {
            let state = self.state.read(cx);
            let mut key = format!("{}|", since.timestamp_millis());
            for id in &ids {
                let session = state.session_for(id);
                key.push_str(&format!(
                    "{id}:{:?}:{:?};",
                    session.map(|s| s.status),
                    session.map(|s| s.updated_at)
                ));
            }
            key
        };
        if key == self.runs.key || ids.is_empty() {
            return;
        }
        let Some(engine) = self.state.read(cx).engine().cloned() else {
            return;
        };
        self.runs.key = key;
        let params = serde_json::to_value(TodayRunsRequest {
            since_ms: since.timestamp_millis(),
            chat_ids: Some(ids),
        })
        .unwrap_or_default();
        self.runs.task = Some(cx.spawn(async move |center, cx| {
            let Ok(runs) = engine
                .client()
                .call_as::<TodayRuns>(zeron_rpc::methods::GET_TODAY_RUNS, params)
                .await
            else {
                return;
            };
            center
                .update(cx, |center, cx| {
                    if center.runs.runs != runs {
                        center.runs.runs = runs;
                        cx.notify();
                    }
                })
                .ok();
        }));
    }

    fn started_at(&self, node: &TreeNode, cx: &App) -> Option<DateTime<Utc>> {
        let state = self.state.read(cx);
        let chat = node
            .chat_id
            .as_deref()
            .and_then(|id| state.chats.iter().find(|chat| chat.id == id));
        match chat {
            Some(chat) => Some(
                chat.delegation
                    .as_ref()
                    .map_or(chat.created_at, |delegation| delegation.started_at),
            ),
            None => node.since,
        }
    }

    fn legend(theme: &Theme) -> AnyElement {
        let palette = Palette::of(theme);
        let swatch = |color: Hsla, label: &'static str| {
            div()
                .flex()
                .items_center()
                .gap(px(6.0))
                .child(div().w(px(18.0)).h(px(6.0)).rounded(px(3.0)).bg(color))
                .child(label)
        };
        div()
            .debug_selector(|| "task-center-timeline-legend".into())
            .flex_none()
            .mx(px(22.0))
            .mt(px(4.0))
            .py(px(10.0))
            .border_t_1()
            .border_color(theme.border)
            .flex()
            .items_center()
            .gap(px(18.0))
            .text_size(ui_rems(11.5))
            .text_color(theme.text_muted)
            .child(swatch(palette.working, "Working"))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.0))
                    .child(
                        div()
                            .w(px(18.0))
                            .h(px(6.0))
                            .rounded(px(3.0))
                            .bg(palette.wait_fill)
                            .border_1()
                            .border_color(palette.wait_edge),
                    )
                    .child("Needs you, waiting on an answer"),
            )
            .child(swatch(palette.done, "Done"))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.0))
                    .child(
                        div()
                            .w(px(18.0))
                            .h(px(6.0))
                            .rounded(px(3.0))
                            .border_1()
                            .border_dashed()
                            .border_color(theme.text_faint),
                    )
                    .child("Planned"),
            )
            .child(div().flex_1())
            .child(
                div()
                    .text_color(theme.text_faint)
                    .child("Dotted line: who started whom, and when"),
            )
            .into_any_element()
    }

    pub(super) fn render_timeline(
        &mut self,
        root: &str,
        nodes: &[TreeNode],
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let reduced = motion::reduced_motion(cx);
        let now = Utc::now();
        let starts: Vec<Option<DateTime<Utc>>> =
            nodes.iter().map(|node| self.started_at(node, cx)).collect();
        let since = starts.iter().flatten().min().copied().unwrap_or(now);
        self.refresh_runs(nodes, since, cx);
        let first = starts[1..].iter().flatten().min().copied().unwrap_or(since);
        let ix = self.picked(nodes);
        let outline = self.render_outline_column(root, &nodes[ix].id, theme, cx);
        let rects = self
            .outline
            .as_ref()
            .map(|(_, tree, _)| tree.read(cx).rects())
            .unwrap_or_default();
        let runs = self.runs.runs.clone();
        let live_now = nodes.iter().any(|node| node.status.live());
        if live_now && !reduced {
            motion::pulse_lease(cx.entity_id(), cx);
        }
        let last_end = runs
            .runs
            .iter()
            .map(|run| run.started_at_ms + run.duration_ms.unwrap_or(0))
            .max()
            .unwrap_or(first.timestamp_millis());
        let has_planned = nodes
            .iter()
            .any(|node| matches!(node.kind, NodeKind::Step(_)) && node.chat_id.is_none());
        let now_ms = now.timestamp_millis();
        let last_wait = runs
            .waits
            .iter()
            .filter_map(|wait| wait.ended_at_ms)
            .max()
            .unwrap_or(0);
        let first_ms = first.timestamp_millis();
        let raw_end = if live_now || has_planned {
            now_ms.max(last_end)
        } else {
            last_end.max(last_wait).max(first_ms)
        };
        let span = (raw_end - first_ms).max(10_000);
        let pad = ((span as f32 * 0.06) as i64).max(1_000);
        let start_ms = first_ms - pad;
        let end_ms = raw_end + if has_planned { span / 4 } else { pad };
        let step_ms = tick_seconds(end_ms - start_ms) * 1_000;
        let lanes_src: Vec<(String, usize)> = nodes
            .iter()
            .enumerate()
            .map(|(ix, node)| (node.id.clone(), ix))
            .collect();
        let palette = Palette::of(theme);
        let tones = (theme.text_faint, theme.accent, theme.border);
        let statuses: Vec<(Status, Option<String>, Option<usize>, NodeKind)> = nodes
            .iter()
            .map(|node| (node.status, node.chat_id.clone(), node.parent, node.kind))
            .collect();
        let child_starts: Vec<Option<i64>> = starts
            .iter()
            .map(|at| at.map(|at| at.timestamp_millis()))
            .collect();
        let pulse = if reduced {
            1.0
        } else {
            let t = motion::now().saturating_duration_since(self.shimmer_since);
            0.55 + 0.45 * (t.as_secs_f32() * std::f32::consts::TAU / 1.6).sin().abs()
        };
        let chart_box = self.runs.chart.clone();
        let label_rects = rects.clone();
        let bars = canvas(
            move |bounds, _, _| chart_box.set(Some(bounds)),
            move |bounds, _, window, _| {
                let (faint, accent, grid) = tones;
                let Palette {
                    working,
                    done,
                    wait_fill,
                    wait_edge,
                    link,
                } = palette;
                let left = f32::from(bounds.left()) + 12.0;
                let width = (f32::from(bounds.size.width) - 24.0).max(1.0);
                let x_of = |ms: i64| {
                    left + width * ((ms - start_ms) as f32 / (end_ms - start_ms).max(1) as f32)
                };
                let top = f32::from(bounds.top());
                let bottom = f32::from(bounds.bottom());
                let rects = rects.borrow();
                let lanes: Vec<Lane> = lanes_src
                    .iter()
                    .filter_map(|(id, ix)| {
                        let rect = rects.get(id)?;
                        Some(Lane {
                            y: f32::from(rect.top()),
                            height: f32::from(rect.size.height),
                            node: *ix,
                        })
                    })
                    .collect();
                let lane_of = |node: usize| lanes.iter().find(|lane| lane.node == node);
                let first_tick = (start_ms / step_ms + 1) * step_ms;
                let mut tick = first_tick;
                while tick < end_ms {
                    let x = x_of(tick);
                    window.paint_quad(gpui::fill(
                        Bounds::new(
                            point(px(x), px(top + AXIS)),
                            size(px(1.0), px(bottom - top - AXIS)),
                        ),
                        grid.opacity(0.6),
                    ));
                    tick += step_ms;
                }
                let bar = |window: &mut Window, y: f32, from: i64, to: i64, color: Hsla| {
                    let (x0, x1) = (x_of(from), x_of(to).max(x_of(from) + MIN_BAR));
                    window.paint_quad(
                        gpui::fill(
                            Bounds::new(
                                point(px(x0), px(y - BAR / 2.0)),
                                size(px(x1 - x0), px(BAR)),
                            ),
                            color,
                        )
                        .corner_radii(px(BAR / 2.0)),
                    );
                };
                for (ix, (status, chat, parent, kind)) in statuses.iter().enumerate() {
                    let Some(lane) = lane_of(ix) else {
                        continue;
                    };
                    let mid = lane.y + lane.height / 2.0;
                    if let (Some(parent), Some(at)) = (parent, child_starts[ix])
                        && let Some(up) = lane_of(*parent)
                    {
                        let x = x_of(at);
                        dashed(window, (x, up.y + up.height / 2.0), (x, mid), 3.0, link);
                    }
                    match (chat, kind) {
                        (Some(chat), _) => {
                            for run in runs.runs.iter().filter(|run| &run.chat_id == chat) {
                                let running = run.status == TodayRunStatus::Running;
                                let to = if running {
                                    now_ms
                                } else {
                                    run.started_at_ms + run.duration_ms.unwrap_or(0)
                                };
                                bar(
                                    window,
                                    mid,
                                    run.started_at_ms,
                                    to,
                                    if running { working } else { done },
                                );
                                if running {
                                    let end = point(px(x_of(to)), px(mid));
                                    window.paint_quad(
                                        gpui::fill(
                                            Bounds::centered_at(end, size(px(8.0), px(8.0))),
                                            gpui::white().opacity(pulse),
                                        )
                                        .corner_radii(px(4.0)),
                                    );
                                }
                            }
                            for wait in runs.waits.iter().filter(|wait| &wait.chat_id == chat) {
                                let to = wait.ended_at_ms.unwrap_or(now_ms);
                                let (x0, x1) = (
                                    x_of(wait.started_at_ms),
                                    x_of(to).max(x_of(wait.started_at_ms) + MIN_BAR),
                                );
                                window.paint_quad(
                                    gpui::fill(
                                        Bounds::new(
                                            point(px(x0), px(mid - BAR / 2.0)),
                                            size(px(x1 - x0), px(BAR)),
                                        ),
                                        wait_fill,
                                    )
                                    .corner_radii(px(BAR / 2.0))
                                    .border_widths(px(1.0))
                                    .border_color(wait_edge),
                                );
                            }
                        }
                        (None, NodeKind::Native) => {
                            if let Some(at) = child_starts[ix] {
                                let x = x_of(at);
                                window.paint_quad(gpui::fill(
                                    Bounds::new(
                                        point(px(x - 1.0), px(mid - 6.0)),
                                        size(px(2.0), px(12.0)),
                                    ),
                                    if *status == Status::Running {
                                        working
                                    } else {
                                        done
                                    },
                                ));
                            }
                        }
                        (None, NodeKind::Step(_)) => {
                            let x0 = x_of(now_ms) + 10.0;
                            let (x1, y0, y1) =
                                (x0 + PLANNED_WIDTH, mid - BAR / 2.0, mid + BAR / 2.0);
                            dashed(window, (x0, y0), (x1, y0), 3.0, faint);
                            dashed(window, (x0, y1), (x1, y1), 3.0, faint);
                            dashed(window, (x0, y0), (x0, y1), 3.0, faint);
                            dashed(window, (x1, y0), (x1, y1), 3.0, faint);
                        }
                        _ => {}
                    }
                }
                if now_ms > end_ms {
                    return;
                }
                let x = x_of(now_ms);
                window.paint_quad(gpui::fill(
                    Bounds::new(
                        point(px(x), px(top + AXIS - 4.0)),
                        size(px(1.0), px(bottom - top - AXIS + 4.0)),
                    ),
                    accent.opacity(0.8),
                ));
            },
        )
        .absolute()
        .inset_0();
        let local = |ms: i64| {
            DateTime::from_timestamp_millis(ms)
                .map(|at| at.with_timezone(&chrono::Local).format("%H:%M").to_string())
                .unwrap_or_default()
        };
        let tick_format = if step_ms < 60_000 {
            "%H:%M:%S"
        } else {
            "%H:%M"
        };
        let mut labels: Vec<(f32, String)> = Vec::new();
        let mut tick = (start_ms / step_ms + 1) * step_ms;
        while tick < end_ms - (end_ms - start_ms) / 20 {
            labels.push((
                (tick - start_ms) as f32 / (end_ms - start_ms).max(1) as f32,
                DateTime::from_timestamp_millis(tick)
                    .map(|at| {
                        at.with_timezone(&chrono::Local)
                            .format(tick_format)
                            .to_string()
                    })
                    .unwrap_or_default(),
            ));
            tick += step_ms;
        }
        let now_at = (now_ms - start_ms) as f32 / (end_ms - start_ms).max(1) as f32;
        let axis = div()
            .absolute()
            .top_0()
            .left(px(12.0))
            .right(px(12.0))
            .h(px(AXIS))
            .children(labels.into_iter().map(|(at, label)| {
                div()
                    .absolute()
                    .top(px(6.0))
                    .left(gpui::relative(at))
                    .ml(px(4.0))
                    .text_size(ui_rems(11.0))
                    .text_color(theme.text_faint)
                    .child(label)
            }))
            .children((now_ms <= end_ms).then(|| {
                div()
                    .debug_selector(|| "task-center-timeline-now".into())
                    .absolute()
                    .top(px(4.0))
                    .left(gpui::relative(now_at))
                    .ml(px(-20.0))
                    .w(px(40.0))
                    .h(px(18.0))
                    .rounded_full()
                    .bg(theme.accent)
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_size(ui_rems(11.0))
                    .font_weight(gpui::FontWeight::MEDIUM)
                    .text_color(gpui::white())
                    .child(local(now_ms))
            }));
        let wait_labels: Vec<AnyElement> = match self.runs.chart.get() {
            Some(chart) => {
                let rects = label_rects.borrow();
                self.runs
                    .runs
                    .waits
                    .iter()
                    .enumerate()
                    .filter_map(|(k, wait)| {
                        let node = nodes
                            .iter()
                            .find(|node| node.chat_id.as_deref() == Some(wait.chat_id.as_str()))?;
                        let rect = rects.get(&node.id)?;
                        let y = f32::from(rect.top()) + f32::from(rect.size.height) / 2.0
                            - BAR / 2.0
                            - 18.0
                            - f32::from(chart.top());
                        let at = (wait.started_at_ms - start_ms) as f32
                            / (end_ms - start_ms).max(1) as f32;
                        let (label, tip) = match wait.ended_at_ms {
                            None => (
                                "Needs you".to_string(),
                                format!("Waiting on an answer since {}", local(wait.started_at_ms)),
                            ),
                            Some(ended) => (
                                format!(
                                    "Waited {}",
                                    crate::tasks::tree::fmt_time(
                                        (ended - wait.started_at_ms).max(0) as u64
                                    )
                                ),
                                format!(
                                    "Waited on an answer from {} to {}",
                                    local(wait.started_at_ms),
                                    local(ended)
                                ),
                            ),
                        };
                        let selector = format!("task-center-timeline-wait-{k}");
                        Some(
                            div()
                                .id(SharedString::from(selector.clone()))
                                .debug_selector(move || selector.clone())
                                .absolute()
                                .top(px(y))
                                .left(gpui::relative(at))
                                .h(px(14.0))
                                .flex()
                                .items_center()
                                .text_size(ui_rems(10.5))
                                .font_weight(gpui::FontWeight::MEDIUM)
                                .text_color(theme.warning)
                                .whitespace_nowrap()
                                .tooltip(widgets::text_tooltip(tip))
                                .child(label)
                                .into_any_element(),
                        )
                    })
                    .collect()
            }
            None => Vec::new(),
        };
        let slid = self.slide_from(crate::settings::TreeView::Outline);
        let chart = div()
            .id("task-center-timeline")
            .debug_selector(|| "task-center-timeline".into())
            .relative()
            .overflow_hidden()
            .left(px(super::view_morph::SLIDE * (1.0 - slid)))
            .opacity(slid)
            .flex_1()
            .min_w_0()
            .h_full()
            .border_l_1()
            .border_color(theme.border)
            .child(bars)
            .child(axis)
            .child(
                div()
                    .absolute()
                    .top_0()
                    .bottom_0()
                    .left(px(12.0))
                    .right(px(12.0))
                    .children(wait_labels),
            );
        let detail = (!matches!(self.selected, Selection::Lead) || self.drawer_open).then(|| {
            let ix = self.picked(nodes);
            self.render_detail(nodes, ix, reduced, theme, cx)
        });
        let drawer = detail.and_then(|detail| self.render_drawer(detail, motion::now(), theme, cx));
        div()
            .flex_1()
            .min_h_0()
            .relative()
            .flex()
            .flex_col()
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .child(div().pt(px(AXIS)).h_full().child(outline))
                    .child(chart),
            )
            .child(Self::legend(theme))
            .children(drawer)
            .into_any_element()
    }
}
