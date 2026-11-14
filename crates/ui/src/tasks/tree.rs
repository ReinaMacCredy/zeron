use std::time::{Duration, Instant};

use gpui::{
    AnyElement, Bounds, Hsla, IntoElement, PathBuilder, canvas, div, point, prelude::*, px, size,
};
use zeron_proto::ChatIndicator;

use crate::icons::{self, icon};
use crate::motion;
use crate::theme::Theme;
use crate::typography::ui_rems;

const SPIN: Duration = Duration::from_millis(900);

pub(crate) use zeron_doc::tree::TreeStatus as Status;

pub(crate) trait StatusTone {
    fn tone(self, theme: &Theme) -> Hsla;
}

impl StatusTone for Status {
    fn tone(self, theme: &Theme) -> Hsla {
        let tone = |indicator| crate::shell::chat_status_tone(indicator, theme).2;
        match self {
            Self::Running => tone(ChatIndicator::Working),
            Self::Waiting => theme.warning,
            Self::Done => tone(ChatIndicator::Completed),
            Self::Failed => theme.danger,
            Self::Queued | Self::Stopped => theme.text_faint,
        }
    }
}

fn cap(window: &mut gpui::Window, at: gpui::Point<gpui::Pixels>, width: f32, color: Hsla) {
    window.paint_quad(
        gpui::fill(Bounds::centered_at(at, size(px(width), px(width))), color)
            .corner_radii(px(width / 2.0)),
    );
}

fn spin_epoch() -> Instant {
    static EPOCH: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
    *EPOCH.get_or_init(motion::now)
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
                let elapsed = motion::now().saturating_duration_since(spin_epoch());
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

pub(crate) fn kind_badge(theme: &Theme) -> AnyElement {
    let label = zeron_doc::tree::BADGE;
    let fg = theme.text_muted;
    div()
        .debug_selector(move || format!("kind-badge-{label}"))
        .flex_none()
        .h(px(18.0))
        .pl(px(5.0))
        .pr(px(6.0))
        .rounded(px(5.0))
        .bg(theme.wash(0.07))
        .flex()
        .items_center()
        .gap(px(4.0))
        .text_size(ui_rems(11.0))
        .font_weight(gpui::FontWeight::MEDIUM)
        .text_color(fg)
        .child(icon(icons::LUCIDE_LIST_TREE).size(px(11.0)).text_color(fg))
        .child(label)
        .into_any_element()
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
