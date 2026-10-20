use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

use gpui::{
    AnyElement, Bounds, Context, Hsla, IntoElement, PathBuilder, SharedString, canvas, div, point,
    prelude::*, px, size,
};
use zeron_proto::{ChatIndicator, DelegationStatus, SessionStats, SessionStatus, StepStatus};

use super::StepRow;
use crate::icons::{self, icon};
use crate::motion::{self, CubicBezier, MotionSpec, Tween};
use crate::state::AppState;
use crate::theme::{Appearance, Theme};

pub(crate) const EASE: CubicBezier = CubicBezier::new(0.32, 0.72, 0.24, 1.0);
pub(crate) const RISE: MotionSpec = MotionSpec::new(360, EASE);
pub(crate) const RISE_Y: f32 = 6.0;
pub(crate) const DRAW: MotionSpec = MotionSpec::new(420, EASE);
pub(crate) const DRAW_LEAD_MS: u64 = 160;
pub(crate) const FIRST_ENTER_MS: u64 = 200;
pub(crate) const LATER_ENTER_MS: u64 = 160;
pub(crate) const STAGGER_MS: u64 = 60;
pub(crate) const GLIDE: MotionSpec = MotionSpec::new(420, EASE);
pub(crate) const LEAVE: MotionSpec = MotionSpec::new(280, EASE);
pub(crate) const LEAVE_Y: f32 = 4.0;
pub(crate) const STATUS: MotionSpec = MotionSpec::new(240, motion::EASE);
pub(crate) const FADE: MotionSpec = MotionSpec::new(280, motion::EASE);
pub(crate) const FOLD: MotionSpec = MotionSpec::new(280, EASE);
pub(crate) const POP: MotionSpec = MotionSpec::new(200, EASE);
pub(crate) const CHEVRON: MotionSpec = MotionSpec::new(220, EASE);
const SPIN: Duration = Duration::from_millis(900);
const REMOVE_AFTER: Duration = Duration::from_millis(30);
const SHINE: Duration = Duration::from_millis(1100);
const MUTED_OPACITY: f32 = 0.6;
const CORNER: f32 = 8.0;
pub(crate) const LINK_WIDTH: f32 = 1.5;
pub(crate) const DOT_RADIUS: f32 = 3.25;
const STATS_POLL: Duration = Duration::from_secs(10);
const STATS_SETTLE: Duration = Duration::from_secs(2);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Status {
    Queued,
    Running,
    Waiting,
    Done,
    Failed,
    Stopped,
}

impl Status {
    pub fn of_step(row: &StepRow, delegation: Option<DelegationStatus>) -> (Self, &'static str) {
        match row.status {
            StepStatus::Done => (Self::Done, "Done"),
            StepStatus::Skipped => (Self::Stopped, "Skipped"),
            StepStatus::Planned => (Self::Queued, "Planned"),
            StepStatus::Working => (Self::Running, "Working"),
            StepStatus::Failed => (Self::Failed, "Failed"),
            StepStatus::NeedsYou => {
                let stopped = delegation.is_none()
                    && row.step.turn.as_ref().map(|turn| turn.ended)
                        == Some(zeron_proto::StepTurnEnd::Stopped);
                match delegation {
                    _ if row.indicator == Some(ChatIndicator::AwaitingInput) => {
                        (Self::Waiting, "Needs you")
                    }
                    Some(DelegationStatus::Cancelled) => (Self::Failed, "Stopped"),
                    Some(DelegationStatus::Interrupted) => (Self::Failed, "Interrupted"),
                    Some(DelegationStatus::Failed) => (Self::Failed, "Failed"),
                    _ if stopped => (Self::Failed, "Stopped"),
                    _ if row.indicator == Some(ChatIndicator::Errored) => (Self::Failed, "Failed"),
                    _ => (Self::Waiting, "Needs you"),
                }
            }
        }
    }

    pub fn muted(self) -> bool {
        matches!(self, Self::Queued | Self::Stopped)
    }

    pub fn live(self) -> bool {
        matches!(self, Self::Running | Self::Waiting)
    }

    pub fn tone(self, theme: &Theme) -> Hsla {
        let tone = |indicator| crate::shell::chat_status_tone(indicator, theme).2;
        match self {
            Self::Running => tone(ChatIndicator::Working),
            Self::Waiting => theme.warning,
            Self::Done => tone(ChatIndicator::Completed),
            Self::Failed => theme.danger,
            Self::Queued | Self::Stopped => theme.text_faint,
        }
    }

    fn opacity(self) -> f32 {
        if self.muted() { MUTED_OPACITY } else { 1.0 }
    }
}

pub(crate) fn lead_status(
    task: &zeron_proto::Task,
    plan: &super::Rollup,
) -> (Status, &'static str) {
    if task.completed_at.is_some() {
        return (Status::Done, "Done");
    }
    match plan.lead {
        None => (Status::Queued, "Not started"),
        _ if plan.phase(task) == super::PlanPhase::Planning => (Status::Running, "Planning"),
        Some(ChatIndicator::Working) => (Status::Running, "Working"),
        Some(ChatIndicator::Errored) => (Status::Failed, "Failed"),
        _ if super::TaskCenter::lead_waits(task, plan).is_some() => (Status::Waiting, "Needs you"),
        _ if plan.count(StepStatus::Working) > 0 => (Status::Running, "Working"),
        _ => (Status::Queued, "Idle"),
    }
}

pub(crate) fn link_color(muted: bool, theme: &Theme) -> Hsla {
    let dark = theme.appearance == Appearance::Dark;
    match (muted, dark) {
        (false, true) => gpui::rgb(0x4f8fdb).into(),
        (false, false) => gpui::rgb(0x82bcff).into(),
        (true, true) => gpui::white().opacity(0.14),
        (true, false) => gpui::black().opacity(0.12),
    }
}

pub(crate) struct NodeIn<'a> {
    pub id: &'a str,
    pub root: bool,
    pub visible: bool,
    pub status: Status,
}

struct Node {
    rise: Tween,
    draw: Tween,
    leave: Option<Tween>,
    fade: Option<Tween>,
    at: Option<(f32, f32)>,
    extra: (f32, f32),
    glide: Tween,
    glide_from: (f32, f32),
    from: Status,
    status: Status,
    tone: Tween,
}

impl Node {
    fn entering(delay_ms: u64, status: Status, reduced: bool, now: Instant) -> Self {
        let rise = |spec: MotionSpec| {
            let mut tween = Tween::at(0.0, now);
            tween.retarget(1.0, spec, reduced, now);
            tween
        };
        Self {
            rise: rise(RISE.with_delay(delay_ms)),
            draw: rise(DRAW.with_delay(delay_ms.saturating_sub(DRAW_LEAD_MS))),
            leave: None,
            fade: None,
            at: None,
            glide: Tween::at(0.0, now),
            glide_from: (0.0, 0.0),
            extra: (0.0, 0.0),
            from: status,
            status,
            tone: Tween::at(1.0, now),
        }
    }

    fn tweens(&self) -> impl Iterator<Item = &Tween> {
        [&self.rise, &self.draw, &self.glide, &self.tone]
            .into_iter()
            .chain(self.leave.as_ref())
            .chain(self.fade.as_ref())
    }
}

pub(crate) struct Frame {
    pub opacity: f32,
    pub y: f32,
    pub drawn: f32,
    pub leaving: bool,
    pub fade: f32,
}

#[derive(Default)]
pub(crate) struct TreeMotion {
    built: bool,
    nodes: HashMap<String, Node>,
    solid: bool,
}

impl TreeMotion {
    pub fn solid() -> Self {
        Self {
            solid: true,
            ..Self::default()
        }
    }

    fn opacity(&self, status: Status) -> f32 {
        if self.solid && status == Status::Queued {
            1.0
        } else {
            status.opacity()
        }
    }

    pub fn sync(&mut self, nodes: &[NodeIn], reduced: bool, now: Instant) {
        let mut fresh = 0;
        let mut leaving = Vec::new();
        for node in nodes {
            let current = self.nodes.get_mut(node.id);
            match current {
                Some(current) if node.visible && current.leave.is_none() => {
                    if current.status != node.status {
                        current.from = current.status;
                        current.status = node.status;
                        current.tone = Tween::at(0.0, now);
                        current.tone.retarget(1.0, STATUS, reduced, now);
                    }
                }
                Some(current) if !node.visible => {
                    if current.leave.is_none() {
                        leaving.push(node.id);
                    }
                }
                None if !node.visible => {}
                _ => {
                    let delay = if node.root {
                        0
                    } else {
                        let base = if self.built {
                            LATER_ENTER_MS
                        } else {
                            FIRST_ENTER_MS
                        };
                        fresh += 1;
                        base + (fresh - 1) * STAGGER_MS
                    };
                    let at = self.nodes.get(node.id).and_then(|old| old.at);
                    let mut entering = Node::entering(delay, node.status, reduced, now);
                    entering.at = at;
                    self.nodes.insert(node.id.to_string(), entering);
                }
            }
        }
        for (i, id) in leaving.iter().rev().enumerate() {
            if let Some(node) = self.nodes.get_mut(*id) {
                let delay = i as u64 * STAGGER_MS;
                let out = |spec: MotionSpec| {
                    let mut tween = Tween::at(1.0, now);
                    tween.retarget(0.0, spec.with_delay(delay), reduced, now);
                    tween
                };
                node.leave = Some(out(LEAVE));
                node.fade = Some(out(FADE));
            }
        }
        let listed: HashSet<&str> = nodes.iter().map(|node| node.id).collect();
        let linger = REMOVE_AFTER.mul_f32(motion::speed_scale());
        let gone = now.checked_sub(linger).is_some_and(|then| {
            self.nodes
                .values()
                .filter_map(|node| node.leave)
                .all(|leave| leave.settled_at(then))
        }) || reduced;
        self.nodes
            .retain(|id, node| listed.contains(id.as_str()) && (node.leave.is_none() || !gone));
        self.built = true;
    }

    pub fn shown(&self, id: &str) -> bool {
        self.nodes.contains_key(id)
    }

    pub fn frame(&self, id: &str, now: Instant) -> Frame {
        let Some(node) = self.nodes.get(id) else {
            return Frame {
                opacity: 1.0,
                y: 0.0,
                drawn: 1.0,
                leaving: false,
                fade: 1.0,
            };
        };
        let mute = motion::lerp(
            self.opacity(node.from),
            self.opacity(node.status),
            node.tone.value_at(now),
        );
        match node.leave {
            Some(leave) => {
                let left = leave.value_at(now);
                Frame {
                    opacity: left * mute,
                    y: LEAVE_Y * (1.0 - left),
                    drawn: left,
                    leaving: true,
                    fade: node.fade.map_or(left, |fade| fade.value_at(now)),
                }
            }
            None => {
                let risen = node.rise.value_at(now);
                Frame {
                    opacity: risen * mute,
                    y: RISE_Y * (1.0 - risen),
                    drawn: node.draw.value_at(now),
                    leaving: false,
                    fade: 1.0,
                }
            }
        }
    }

    // FLIP: a moved node, or one whose drag offset was let go, glides from where it was drawn.
    pub fn place(
        &mut self,
        id: &str,
        at: (f32, f32),
        extra: (f32, f32),
        reduced: bool,
        now: Instant,
    ) -> (f32, f32) {
        let Some(node) = self.nodes.get_mut(id) else {
            return (0.0, 0.0);
        };
        let glide = |node: &Node| {
            let t = node.glide.value_at(now);
            (node.glide_from.0 * t, node.glide_from.1 * t)
        };
        let moved = node.at.is_some_and(|previous| {
            (previous.0 - at.0).abs() > 0.5 || (previous.1 - at.1).abs() > 0.5
        });
        let dropped = node.extra != (0.0, 0.0) && extra == (0.0, 0.0);
        if let Some(previous) = node.at
            && (moved || dropped)
            && !reduced
        {
            let (gx, gy) = glide(node);
            node.glide_from = (
                previous.0 + gx + node.extra.0 - at.0 - extra.0,
                previous.1 + gy + node.extra.1 - at.1 - extra.1,
            );
            node.glide = Tween::at(1.0, now);
            node.glide.retarget(0.0, GLIDE, false, now);
        }
        node.at = Some(at);
        node.extra = extra;
        glide(node)
    }

    pub fn tint(&self, id: &str, now: Instant, color: impl Fn(Status) -> Hsla) -> Hsla {
        match self.nodes.get(id) {
            Some(node) => motion::mix(
                color(node.from),
                color(node.status),
                node.tone.value_at(now),
            ),
            None => color(Status::Queued),
        }
    }

    pub fn animating(&self, now: Instant) -> bool {
        self.nodes
            .values()
            .flat_map(Node::tweens)
            .any(|tween| !tween.settled_at(now))
    }
}

pub(crate) struct Link {
    pub from: (f32, f32),
    pub to: (f32, f32),
    pub drawn: f32,
    pub color: Hsla,
}

pub(crate) struct Dot {
    pub at: (f32, f32),
    pub stroke: Hsla,
    pub fill: Hsla,
    pub opacity: f32,
}

fn link_points((ax, top): (f32, f32), (cx, cy): (f32, f32)) -> Vec<(f32, f32)> {
    let dx = cx - ax;
    if dx.abs() < 2.0 {
        return vec![(ax, top), (ax, cy)];
    }
    let r = CORNER.min(dx.abs() / 2.0).min((cy - top) / 2.0).max(0.0);
    let turn = ax + dx.signum() * r;
    let mut points = vec![(ax, top), (ax, cy - r)];
    for i in 1..=6 {
        let t = i as f32 / 6.0;
        let u = 1.0 - t;
        points.push((
            u * u * ax + 2.0 * u * t * ax + t * t * turn,
            u * u * (cy - r) + 2.0 * u * t * cy + t * t * cy,
        ));
    }
    points.push((cx, cy));
    points
}

// gpui dashes take no offset, so a connector draws in by growing.
fn drawn(points: &[(f32, f32)], fraction: f32) -> Vec<(f32, f32)> {
    let length = |a: (f32, f32), b: (f32, f32)| (b.0 - a.0).hypot(b.1 - a.1);
    let total: f32 = points.windows(2).map(|pair| length(pair[0], pair[1])).sum();
    let mut left = total * fraction.clamp(0.0, 1.0);
    let mut out = vec![points[0]];
    for pair in points.windows(2) {
        let step = length(pair[0], pair[1]);
        if step >= left {
            let t = if step > 0.0 { left / step } else { 0.0 };
            out.push((
                pair[0].0 + (pair[1].0 - pair[0].0) * t,
                pair[0].1 + (pair[1].1 - pair[0].1) * t,
            ));
            return out;
        }
        left -= step;
        out.push(pair[1]);
    }
    out
}

pub(crate) fn links(links: Vec<Link>, dots: Vec<Dot>, width: f32, radius: f32) -> impl IntoElement {
    canvas(
        |_, _, _| {},
        move |bounds, _, window, _| {
            let at = |(x, y): (f32, f32)| point(bounds.origin.x + px(x), bounds.origin.y + px(y));
            for link in &links {
                if link.drawn <= 0.001 {
                    continue;
                }
                let points = drawn(&link_points(link.from, link.to), link.drawn);
                let mut path = PathBuilder::stroke(px(width));
                path.move_to(at(points[0]));
                for point in &points[1..] {
                    path.line_to(at(*point));
                }
                if let Ok(path) = path.build() {
                    window.paint_path(path, link.color);
                }
                for end in [points[0], points[points.len() - 1]] {
                    cap(window, at(end), width, link.color);
                }
            }
            for dot in &dots {
                if dot.opacity <= 0.001 {
                    continue;
                }
                let center = at(dot.at);
                let bounds = Bounds::centered_at(center, size(px(radius * 2.0), px(radius * 2.0)));
                window.paint_quad(
                    gpui::fill(bounds, dot.fill.opacity(dot.opacity))
                        .corner_radii(px(radius))
                        .border_widths(px(width))
                        .border_color(dot.stroke.opacity(dot.opacity)),
                );
            }
        },
    )
    .absolute()
    .inset_0()
}

fn cap(window: &mut gpui::Window, at: gpui::Point<gpui::Pixels>, width: f32, color: Hsla) {
    window.paint_quad(
        gpui::fill(Bounds::centered_at(at, size(px(width), px(width))), color)
            .corner_radii(px(width / 2.0)),
    );
}

fn spin_epoch() -> Instant {
    static EPOCH: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
    *EPOCH.get_or_init(Instant::now)
}

fn arc(
    center: gpui::Point<gpui::Pixels>,
    radius: f32,
    from: f32,
    sweep: f32,
) -> Vec<gpui::Point<gpui::Pixels>> {
    let steps = (sweep.abs() / std::f32::consts::TAU * 48.0).ceil().max(2.0) as usize;
    (0..=steps)
        .map(|i| {
            let angle = from + sweep * i as f32 / steps as f32;
            point(
                center.x + px(radius * angle.cos()),
                center.y + px(radius * angle.sin()),
            )
        })
        .collect()
}

fn stroke(
    window: &mut gpui::Window,
    points: &[gpui::Point<gpui::Pixels>],
    width: f32,
    color: Hsla,
) {
    let mut path = PathBuilder::stroke(px(width));
    path.move_to(points[0]);
    for point in &points[1..] {
        path.line_to(*point);
    }
    if let Ok(path) = path.build() {
        window.paint_path(path, color);
    }
}

// The owning view repaints while a node runs (its title sweeps), which turns this.
pub(crate) fn spinner(diameter: f32, color: Hsla, reduced: bool) -> impl IntoElement {
    canvas(
        |_, _, _| {},
        move |bounds, _, window, _| {
            let scale = diameter / 24.0;
            let (radius, width) = (8.5 * scale, 2.5 * scale);
            let turn = if reduced {
                0.0
            } else {
                let period = SPIN.mul_f32(motion::speed_scale()).as_secs_f32();
                let elapsed = Instant::now().saturating_duration_since(spin_epoch());
                (elapsed.as_secs_f32() / period).fract()
            };
            let center = bounds.center();
            let tau = std::f32::consts::TAU;
            stroke(
                window,
                &arc(center, radius, 0.0, tau),
                width,
                color.opacity(0.25),
            );
            let points = arc(
                center,
                radius,
                -std::f32::consts::FRAC_PI_2 + turn * tau,
                tau / 4.0,
            );
            stroke(window, &points, width, color);
            for end in [points[0], points[points.len() - 1]] {
                cap(window, end, width, color);
            }
        },
    )
    .flex_none()
    .size(px(diameter))
}

pub(crate) fn glyph(status: Status, diameter: f32, color: Hsla, reduced: bool) -> AnyElement {
    let mark = |name: &'static str| {
        icon(name)
            .size(px(diameter))
            .flex_none()
            .text_color(color)
            .into_any_element()
    };
    match status {
        Status::Running => spinner(diameter, color, reduced).into_any_element(),
        Status::Done => mark(icons::LUCIDE_CHECK),
        Status::Failed => mark(icons::DANGER_TRIANGLE),
        Status::Waiting => mark(icons::CLOCK_CIRCLE),
        Status::Stopped => mark(icons::LUCIDE_SQUARE),
        Status::Queued => div()
            .size(px(diameter))
            .flex_none()
            .flex()
            .items_center()
            .justify_center()
            .child(
                div()
                    .size(px(diameter * 7.0 / 24.0))
                    .rounded_full()
                    .bg(color),
            )
            .into_any_element(),
    }
}

pub(crate) fn title_phase(since: Instant, now: Instant) -> f32 {
    crate::transcript::tool_title_shimmer_phase(since, now)
}

pub(crate) fn shimmer_text(
    text: impl Into<SharedString>,
    shimmer: Option<f32>,
    size: gpui::Rems,
    base: Hsla,
    theme: &Theme,
) -> AnyElement {
    let text = text.into();
    match shimmer {
        Some(phase) => crate::transcript::shimmer_label(
            text,
            phase,
            theme.font_sans.clone(),
            size.into(),
            None,
            base,
            theme.text,
        )
        .flex_none()
        .into_any_element(),
        None => text.into_any_element(),
    }
}

pub(crate) mod amount {
    use super::*;

    pub fn of(stats: Option<&SessionStats>) -> Option<u64> {
        stats
            .map(SessionStats::total_tokens)
            .filter(|tokens| *tokens > 0)
    }

    pub fn branch(
        own: Option<u64>,
        children: impl IntoIterator<Item = Option<u64>>,
    ) -> Option<u64> {
        std::iter::once(own)
            .chain(children)
            .flatten()
            .reduce(|a, b| a + b)
    }

    pub fn text(tokens: u64) -> String {
        format!("{} tokens", crate::session_stats::tokens(tokens))
    }

    pub fn shine(theme: &Theme) -> Hsla {
        match theme.appearance {
            Appearance::Dark => gpui::white().opacity(0.35),
            _ => gpui::black().opacity(0.22),
        }
    }

    // One sweep per 1100 ms: the highlight crosses from -0.5 to 1.5 widths, as the reference's 300% gradient does.
    pub fn phase(since: Instant, now: Instant) -> f32 {
        let period = SHINE.mul_f32(motion::speed_scale()).as_secs_f32();
        let elapsed = now.saturating_duration_since(since).as_secs_f32();
        (1.0 + (elapsed / period).fract()) / 3.0
    }

    pub fn label(
        tokens: Option<u64>,
        live: Option<f32>,
        size: gpui::Rems,
        theme: &Theme,
    ) -> Option<AnyElement> {
        let text = SharedString::from(self::text(tokens?));
        Some(match live {
            Some(phase) => crate::transcript::shimmer_label(
                text,
                phase,
                theme.font_sans.clone(),
                size.into(),
                None,
                theme.text,
                shine(theme),
            )
            .flex_none()
            .text_color(theme.text)
            .into_any_element(),
            None => div()
                .flex_none()
                .text_color(theme.text)
                .child(text)
                .into_any_element(),
        })
    }
}

pub(crate) fn fmt_time(ms: u64) -> String {
    let secs = ms / 1000;
    match secs {
        0..60 => format!("{secs}s"),
        60..3600 => format!("{}m {:02}s", secs / 60, secs % 60),
        _ => format!("{}h {:02}m", secs / 3600, secs % 3600 / 60),
    }
}

pub(crate) fn thread_time(
    status: Status,
    since: Option<chrono::DateTime<chrono::Utc>>,
    stats: Option<&SessionStats>,
) -> Option<String> {
    if status.live()
        && let Some(since) = since
    {
        let ms = (chrono::Utc::now() - since).num_milliseconds().max(0) as u64;
        return Some(fmt_time(ms));
    }
    let stats = stats?;
    let ms = stats.llm_ms + stats.tool_ms;
    (ms > 0).then(|| fmt_time(ms))
}

#[derive(Default)]
pub(crate) struct StatsCache {
    stats: HashMap<String, SessionStats>,
    key: String,
    fetched: Option<Instant>,
    task: Option<gpui::Task<()>>,
}

impl StatsCache {
    pub fn get(&self, state: &AppState, chat_id: &str) -> Option<SessionStats> {
        state
            .session_stats_for(Some(chat_id))
            .or_else(|| self.stats.get(chat_id).copied())
    }

    pub fn refresh<T: 'static>(
        &mut self,
        state: &gpui::Entity<AppState>,
        chat_ids: Vec<String>,
        slot: fn(&mut T) -> &mut StatsCache,
        cx: &mut Context<T>,
    ) {
        let state = state.read(cx);
        let mut key = String::new();
        let mut working = false;
        for id in &chat_ids {
            let session = state.session_for(id);
            working |= session.is_some_and(|s| s.status == SessionStatus::Working);
            key.push_str(&format!(
                "{id}:{:?}:{:?};",
                session.map(|s| s.status),
                session.map(|s| s.updated_at)
            ));
        }
        let changed = key != self.key;
        let due = self
            .fetched
            .is_some_and(|at| working && at.elapsed() >= STATS_POLL);
        if chat_ids.is_empty() || !(changed || due) {
            return;
        }
        let Some(engine) = state.engine().cloned() else {
            return;
        };
        self.key = key;
        self.fetched = Some(Instant::now());
        let params = serde_json::json!({ "chatIds": chat_ids });
        self.task = Some(cx.spawn(async move |this, cx| {
            for pass in 0..1 + usize::from(changed) {
                if pass > 0 {
                    cx.background_executor().timer(STATS_SETTLE).await;
                }
                let Ok(stats) = engine
                    .client()
                    .call_as::<HashMap<String, SessionStats>>(
                        zeron_rpc::methods::GET_SESSION_STATS,
                        params.clone(),
                    )
                    .await
                else {
                    continue;
                };
                let alive = this.update(cx, |owner, cx| {
                    let cache = slot(owner);
                    if cache.stats != stats {
                        cache.stats = stats;
                        cx.notify();
                    }
                });
                if alive.is_err() {
                    return;
                }
            }
        }));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_first_build_staggers_from_200_and_a_later_one_from_160() {
        let now = Instant::now();
        let mut tree = TreeMotion::default();
        let node = |id, root| NodeIn {
            id,
            root,
            visible: true,
            status: Status::Queued,
        };
        tree.sync(
            &[node("lead", true), node("a", false), node("b", false)],
            false,
            now,
        );
        let at = |tree: &TreeMotion, id: &str, ms: u64| {
            tree.frame(id, now + Duration::from_millis(ms)).opacity
        };
        assert!(at(&tree, "lead", 1) > 0.0);
        assert_eq!(at(&tree, "a", 200), 0.0);
        assert!(at(&tree, "a", 201) > 0.0);
        assert_eq!(at(&tree, "b", 260), 0.0);
        assert!(tree.frame("b", now + Duration::from_millis(101)).drawn > 0.0);
        tree.sync(
            &[
                node("lead", true),
                node("a", false),
                node("b", false),
                node("c", false),
            ],
            false,
            now,
        );
        assert_eq!(at(&tree, "c", 160), 0.0);
        assert!(at(&tree, "c", 161) > 0.0);
    }

    #[test]
    fn a_fold_leaves_bottom_first_and_drops_the_nodes() {
        let now = Instant::now();
        let mut tree = TreeMotion::default();
        let node = |id, visible| NodeIn {
            id,
            root: id == "lead",
            visible,
            status: Status::Done,
        };
        tree.sync(
            &[node("lead", true), node("a", true), node("b", true)],
            false,
            now,
        );
        tree.sync(
            &[node("lead", true), node("a", false), node("b", false)],
            false,
            now,
        );
        let mid = now + Duration::from_millis(30);
        assert!(tree.frame("b", mid).opacity < 1.0);
        assert_eq!(
            tree.frame("a", mid).opacity,
            1.0,
            "the upper node waits 60 ms"
        );
        let first_gone = now + Duration::from_millis(300);
        tree.sync(
            &[node("lead", true), node("a", false), node("b", false)],
            false,
            first_gone,
        );
        assert!(
            tree.shown("b"),
            "a leaver stays until the last one has gone"
        );
        let done = now + Duration::from_millis(400);
        tree.sync(
            &[node("lead", true), node("a", false), node("b", false)],
            false,
            done,
        );
        assert!(!tree.shown("a") && !tree.shown("b"));
    }

    #[test]
    fn a_connector_draws_to_a_fraction_of_its_length() {
        let points = link_points((13.0, 26.0), (22.0, 43.0));
        assert_eq!(points.first(), Some(&(13.0, 26.0)));
        assert_eq!(points.last(), Some(&(22.0, 43.0)));
        let length = |points: &[(f32, f32)]| -> f32 {
            points
                .windows(2)
                .map(|pair| (pair[1].0 - pair[0].0).hypot(pair[1].1 - pair[0].1))
                .sum()
        };
        let left = link_points((100.0, 0.0), (60.0, 40.0));
        assert_eq!(left.last(), Some(&(60.0, 40.0)));
        assert!(
            left.iter().all(|(x, _)| *x <= 100.0),
            "a branch to the left turns left"
        );
        let half = drawn(&points, 0.5);
        assert!((length(&half) - length(&points) / 2.0).abs() < 0.01);
        assert_eq!(drawn(&points, 1.0).last(), Some(&(22.0, 43.0)));
    }
}
