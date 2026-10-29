use std::f32::consts::{FRAC_PI_2, TAU};
use std::time::{Duration, Instant};

use gpui::{Bounds, Hsla, PathBuilder, StyleRefinement, Transformation, canvas, point, size};

use super::*;
use crate::app_update::Flow;

pub(super) const CIRCLE: f32 = 28.0;
pub(super) const ICON: f32 = 16.0;
pub(super) const DROP_PERIOD: Duration = Duration::from_millis(1100);
pub(super) const ARC_PERIOD: Duration = Duration::from_millis(1200);
pub(super) const ARC_SPAN: f32 = 0.26;
const ARC_RADIUS: f32 = 15.8;
const ARC_STROKE: f32 = 2.0;
const POP: f32 = 0.45;
pub(super) const CHECK_UNTIL: f32 = 0.9;
pub(super) const TURN: f32 = 0.5;
pub(super) const HALOS: [(f32, f32, f32); 2] =
    [(CIRCLE + 6.0, 0.0, 0.32), (CIRCLE + 14.0, -0.08, 0.16)];

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum GlyphPhase {
    Available,
    Downloading,
    Ready,
    Failed,
}

impl GlyphPhase {
    pub(super) fn of(flow: &Flow) -> Self {
        match flow {
            Flow::Idle => Self::Available,
            Flow::Downloading { .. } => Self::Downloading,
            Flow::Ready { .. } | Flow::Installed => Self::Ready,
            Flow::Failed { .. } => Self::Failed,
        }
    }
}

pub(super) fn update_glyph(phase: GlyphPhase, fill: Hsla, ink: Hsla) -> UpdateGlyph {
    UpdateGlyph { phase, fill, ink }
}

#[derive(IntoElement)]
pub(super) struct UpdateGlyph {
    phase: GlyphPhase,
    fill: Hsla,
    ink: Hsla,
}

impl RenderOnce for UpdateGlyph {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let Self { phase, fill, ink } = self;
        let view = window.with_global_id("update-rail-glyph".into(), |id, window| {
            window.with_element_state(id, |previous: Option<Entity<UpdateGlyphView>>, _| {
                let view = previous.unwrap_or_else(|| {
                    cx.new(|_| UpdateGlyphView {
                        phase,
                        since: None,
                        fill,
                        ink,
                    })
                });
                view.update(cx, |view, cx| {
                    if view.phase != phase {
                        view.phase = phase;
                        view.since = Some(Instant::now());
                        cx.notify();
                    }
                    if view.fill != fill || view.ink != ink {
                        view.fill = fill;
                        view.ink = ink;
                        cx.notify();
                    }
                });
                (view.clone(), view)
            })
        });
        view.cached(StyleRefinement::default().size(px(app_rail::BUTTON)))
    }
}

struct UpdateGlyphView {
    phase: GlyphPhase,
    since: Option<Instant>,
    fill: Hsla,
    ink: Hsla,
}

pub(super) struct Layer {
    pub(super) path: &'static str,
    pub(super) opacity: f32,
    pub(super) transform: Transformation,
}

impl Layer {
    fn rest(path: &'static str) -> Self {
        Self {
            path,
            opacity: 1.0,
            transform: Transformation::default(),
        }
    }
}

impl Render for UpdateGlyphView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let (fill, ink) = (self.fill, self.ink);
        let reduced = cx.reduce_motion();
        let elapsed = self
            .since
            .map(|since| since.elapsed().as_secs_f32() / motion::speed_scale().max(0.01));
        let mut diameter = CIRCLE;
        let mut arc = None;
        let mut halo = false;
        let layers = match self.phase {
            GlyphPhase::Available | GlyphPhase::Failed => vec![Layer::rest(icons::LUCIDE_DOWNLOAD)],
            GlyphPhase::Downloading if reduced => vec![Layer::rest(icons::LUCIDE_DOWNLOAD)],
            GlyphPhase::Downloading => {
                let id = cx.entity_id();
                let drop = motion::spin_phase(id, DROP_PERIOD, cx).unwrap_or(0.0);
                arc = motion::spin_phase(id, ARC_PERIOD, cx);
                vec![
                    Layer::rest(icons::LUCIDE_DOWNLOAD_TRAY),
                    falling_arrow(drop),
                ]
            }
            GlyphPhase::Ready => {
                let entrance = elapsed.filter(|&t| !reduced && t < CHECK_UNTIL + TURN);
                halo = !reduced;
                match entrance {
                    None => vec![Layer::rest(icons::LUCIDE_ROTATE_CW)],
                    Some(t) => {
                        motion::pulse_lease(cx.entity_id(), cx);
                        diameter = CIRCLE * pop_scale(t);
                        if t < CHECK_UNTIL {
                            vec![check_in(t)]
                        } else {
                            vec![restart_in((t - CHECK_UNTIL) / TURN)]
                        }
                    }
                }
            }
        };
        let arc_tint = Hsla {
            l: (fill.l + 0.16).min(0.92),
            ..fill
        };
        let halo_tint = fill;
        div()
            .size(px(app_rail::BUTTON))
            .relative()
            .flex()
            .items_center()
            .justify_center()
            .child(
                canvas(
                    |_, _, _| (),
                    move |bounds, _, window, cx| {
                        if halo {
                            paint_halo(bounds, halo_tint, window, cx);
                        }
                        if let Some(turn) = arc {
                            paint_arc(bounds, turn, arc_tint, window);
                        }
                    },
                )
                .absolute()
                .inset_0(),
            )
            .child(
                div()
                    .size(px(diameter))
                    .flex_none()
                    .relative()
                    .rounded_full()
                    .bg(fill)
                    .children(layers.into_iter().map(|layer| {
                        div()
                            .absolute()
                            .inset_0()
                            .flex()
                            .items_center()
                            .justify_center()
                            .child(
                                icon(layer.path)
                                    .size(px(ICON))
                                    .text_color(ink.opacity(layer.opacity))
                                    .with_transformation(layer.transform),
                            )
                    })),
            )
    }
}

pub(super) fn falling_arrow(phase: f32) -> Layer {
    let unit = ICON / 24.0;
    let eased = |t: f32| t * t;
    let (y, opacity) = if phase < 0.7 {
        let t = phase / 0.7;
        (motion::lerp(-9.0, 1.0, eased(t)), (phase / 0.25).min(1.0))
    } else {
        let t = (phase - 0.7) / 0.3;
        (motion::lerp(1.0, 7.0, eased(t)), 1.0 - t)
    };
    Layer {
        path: icons::LUCIDE_DOWNLOAD_ARROW,
        opacity,
        transform: Transformation::translate(point(px(0.0), px(y * unit))),
    }
}

pub(super) fn pop_scale(t: f32) -> f32 {
    let u = (t / POP).min(1.0);
    let bump = if u < 0.4 {
        motion::EASE_OUT.eval(u / 0.4)
    } else {
        1.0 - motion::EASE_IN_OUT.eval((u - 0.4) / 0.6)
    };
    1.0 + 0.18 * bump
}

pub(super) fn check_in(t: f32) -> Layer {
    let grow = motion::EASE_OUT.eval((t / 0.2).min(1.0));
    let fade = ((CHECK_UNTIL - t) / 0.1).clamp(0.0, 1.0);
    let scale = motion::lerp(0.6, 1.0, grow);
    Layer {
        path: icons::LUCIDE_CHECK,
        opacity: grow.min(fade),
        transform: Transformation::scale(size(scale, scale)),
    }
}

pub(super) fn restart_in(u: f32) -> Layer {
    let e = motion::EASE_OUT.eval(u.clamp(0.0, 1.0));
    let scale = motion::lerp(0.4, 1.0, e);
    Layer {
        path: icons::LUCIDE_ROTATE_CW,
        opacity: e,
        transform: Transformation::rotate(gpui::radians(motion::lerp(
            -200f32.to_radians(),
            0.0,
            e,
        )))
        .with_scaling(size(scale, scale)),
    }
}

fn paint_halo(bounds: Bounds<Pixels>, tint: Hsla, window: &mut Window, cx: &mut App) {
    let center = bounds.center();
    for (diameter, offset, peak) in HALOS.into_iter().rev() {
        let Some(animation) = motion::update_halo(offset, peak, cx) else {
            return;
        };
        let disc = Bounds::new(
            point(center.x - px(diameter / 2.0), center.y - px(diameter / 2.0)),
            size(px(diameter), px(diameter)),
        );
        window.paint_animated_quad(
            gpui::fill(disc, tint).corner_radii(px(diameter / 2.0)),
            animation,
        );
    }
}

pub(super) fn paint_arc(bounds: Bounds<Pixels>, turn: f32, tint: Hsla, window: &mut Window) {
    let center = bounds.center();
    let start = -FRAC_PI_2 + TAU * turn;
    let sweep = TAU * ARC_SPAN;
    let at = |theta: f32| {
        point(
            center.x + px(ARC_RADIUS * theta.cos()),
            center.y + px(ARC_RADIUS * theta.sin()),
        )
    };
    let mut builder = PathBuilder::stroke(px(ARC_STROKE));
    builder.move_to(at(start));
    for step in 1..=12 {
        builder.line_to(at(start + sweep * step as f32 / 12.0));
    }
    if let Ok(path) = builder.build() {
        window.paint_path(path, tint);
    }
}
