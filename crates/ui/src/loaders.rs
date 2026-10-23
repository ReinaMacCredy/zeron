//! Loaders: the zeron pulse loader, the gradient matrix spinner, and the boot
//! splash content. All motion routes through `crate::motion` pure helpers, so
//! the math is unit-tested and these elements are testable-by-compile.
//!
//! Rendering pattern: cells share a self-parking pulse clock; per-cell offsets
//! come from [`motion::staggered_phase`], so all cells stay phase-locked.
//! Cells animate inside fixed-size slots — opacity and inner size
//! are paint-local and never move surrounding layout. Reduced motion snaps every
//! cell to its rest state automatically (gpui `reduce_motion`). Activity grids
//! retain a gentle brightness pulse with system reduced motion; explicit On
//! and background pause keep them still.

use gpui::{
    AnyElement, App, AppContext, Context, Entity, EntityId, IntoElement, ParentElement,
    PathBuilder, Render, RenderOnce, SharedString, Styled, Window, canvas, div, point, px,
};

use crate::motion::{self, GRADIENT_SPIN, PULSE_STAGGER, SPLASH_OUT, ZERON_PULSE};
use crate::theme::{GlyphPalette, Theme};

// Shared with the terminal viewport (`zeron_proto::motion`) so both animate the
// same loaders from the same numbers.
pub use zeron_proto::motion::{
    MARK_CELLS, MARK_SPREAD, MATRIX_SIDE, ZERON_CELLS, mark_cell_stagger,
};

/// The animated zeron mark (zeron-loader.tsx `ZeronLoader`): the full logo
/// pixel grid with a light wave sweeping tail→head. Each cell rests dim
/// (opacity 0.08, scale 0.9) and flares to full as the crest passes; per-cell
/// stagger follows the flight axis. `height_px` sets the mark's height (width
/// follows the 820:940 canvas).
pub fn zeron_mark_loader(
    _id: &'static str,
    theme: &Theme,
    height_px: f32,
    view: EntityId,
    cx: &mut App,
) -> impl IntoElement {
    let color = theme.text;
    let scale = height_px / 940.0;
    let cell = 100.0 * scale;
    let delta = motion::pulse_delta(&ZERON_PULSE, view, cx);
    div()
        .relative()
        .w(px(820.0 * scale))
        .h(px(height_px))
        .children(MARK_CELLS.iter().map(move |&(x, y)| {
            let stagger = mark_cell_stagger(x, y);
            // Fixed slot; the animated cell breathes inside it (paint-local).
            div()
                .absolute()
                .left(px(x * scale))
                .top(px(y * scale))
                .size(px(cell))
                .flex()
                .items_center()
                .justify_center()
                .child({
                    // Negative CSS delay ⇒ the cell starts mid-cycle:
                    // the stagger ADDS phase (zeron-loader.tsx delayFor).
                    let phase = (delta + stagger).rem_euclid(1.0);
                    div()
                        .rounded(px(16.0 * scale))
                        .bg(color)
                        .opacity(motion::pulse_opacity(phase))
                        .size(px(cell * motion::pulse_scale(phase)))
                })
        }))
}

/// The zeron wave loader: a row of cells pulsing opacity 0.08→1 / scale 0.9→1
/// over 2.4s with a 0.15s stagger per cell.
///
/// `id` scopes the per-cell animation state — give each loader instance a
/// distinct id.
pub fn zeron_loader(
    _id: &'static str,
    theme: &Theme,
    cell_px: f32,
    view: EntityId,
    cx: &mut App,
) -> impl IntoElement {
    let color = theme.text;
    let slot = cell_px;
    let delta = motion::pulse_delta(&ZERON_PULSE, view, cx);
    div()
        .flex()
        .flex_row()
        .items_center()
        .gap(px(slot / 2.0))
        .children((0..ZERON_CELLS).map(move |i| {
            // Fixed slot; the animated cell breathes inside it.
            div()
                .size(px(slot))
                .flex()
                .items_center()
                .justify_center()
                .child({
                    let phase = motion::staggered_phase(delta, i, PULSE_STAGGER);
                    div()
                        .rounded(px(slot / 4.0))
                        .bg(color)
                        .opacity(motion::pulse_opacity(phase))
                        .size(px(slot * motion::pulse_scale(phase)))
                })
        }))
}

pub use zeron_proto::motion::{GSPIN_DIM, GSPIN_ROW_TINTS};

/// The gradient matrix spinner (WorkingIndicator), ported from zeron's
/// gradient-spin.tsx: a 3×3 grid of round cells tinted per row from the
/// sunrise gradient. Each cell pulses opacity once per 750ms period; the
/// per-cell phase follows the "arrow-up" pattern (the pulse enters at the
/// bottom edge and converges toward the top-center cell), so the wave reads
/// as travelling upward.
pub fn gradient_spinner(
    _id: &'static str,
    _theme: &Theme,
    cell_px: f32,
    _view: EntityId,
    _cx: &mut App,
) -> impl IntoElement {
    let center = (MATRIX_SIDE as f32 - 1.0) / 2.0;
    let max = MATRIX_SIDE as f32 - 1.0 + center;
    let phases = (0..MATRIX_SIDE)
        .flat_map(|row| {
            (0..MATRIX_SIDE).map(move |col| {
                // Distance of this cell from the wave origin, normalized
                // into a phase offset (gradient-spin's `--gspin-phase`).
                let d = MATRIX_SIDE as f32 - 1.0 - row as f32 + (col as f32 - center).abs();
                if max == 0.0 { 0.0 } else { d / (max + 1.0) }
            })
        })
        .collect();
    let tints = (0..MATRIX_SIDE)
        .map(|row| gpui::rgb(GSPIN_ROW_TINTS[row]).into())
        .collect();
    activity_grid(MATRIX_SIDE, cell_px, tints, phases, 2)
}

/// A 2×3 activity glyph sized for compact status slots. Its color is an
/// explicit accent-preset role supplied by the caller, while brightness snakes
/// around the grid's perimeter as a tiny radial chase.
pub fn mini_glyph_spinner(
    key: impl Into<SharedString>,
    cell_px: f32,
    palette: GlyphPalette,
    view: EntityId,
    cx: &mut App,
) -> impl IntoElement {
    mini_spinner_tinted(key, cell_px, palette.rows(), view, cx)
}

/// Grayscale variant for surfaces where an accent would pull focus (the
/// sidebar connection line): same grid, snake, and timing, color left to the
/// caller.
pub fn mini_mono_spinner(
    key: impl Into<SharedString>,
    cell_px: f32,
    tint: gpui::Hsla,
    view: EntityId,
    cx: &mut App,
) -> impl IntoElement {
    mini_spinner_tinted(key, cell_px, [tint; 3], view, cx)
}

fn mini_spinner_tinted(
    _key: impl Into<SharedString>,
    cell_px: f32,
    row_tints: [gpui::Hsla; 3],
    _view: EntityId,
    _cx: &mut App,
) -> impl IntoElement {
    /// Clockwise ring position of each `(row, col)` cell, top-left first:
    /// (0,0) → (0,1) → (1,1) → (2,1) → (2,0) → (1,0).
    const RING: [[usize; 2]; 3] = [[0, 1], [5, 2], [4, 3]];
    let phases = RING
        .iter()
        .flatten()
        .map(|&position| position as f32 / 6.0)
        .collect();
    activity_grid(2, cell_px, row_tints.to_vec(), phases, 1)
}

fn activity_grid(
    cols: usize,
    cell_px: f32,
    tints: Vec<gpui::Hsla>,
    phases: Vec<f32>,
    stride: u32,
) -> impl IntoElement {
    let rows = tints.len();
    let gap = cell_px / 2.0;
    let extent = |n: usize| n as f32 * cell_px + n.saturating_sub(1) as f32 * gap;
    canvas(
        |_, _, _| (),
        move |bounds, _, window, cx| {
            for (index, phase) in phases.into_iter().enumerate() {
                let (row, col) = (index / cols, index % cols);
                let cell = gpui::Bounds::new(
                    bounds.origin
                        + point(
                            px(col as f32 * (cell_px + gap)),
                            px(row as f32 * (cell_px + gap)),
                        ),
                    gpui::size(px(cell_px), px(cell_px)),
                );
                let fill = gpui::fill(cell, tints[row]).corner_radii(px(cell_px / 2.0));
                match motion::activity_cell(phase, stride, cx) {
                    motion::ActivityCell::Still(opacity) => window.paint_quad(
                        gpui::fill(cell, tints[row].opacity(opacity))
                            .corner_radii(px(cell_px / 2.0)),
                    ),
                    motion::ActivityCell::Animated(animation) => {
                        window.paint_animated_quad(fill, animation)
                    }
                }
            }
        },
    )
    .flex_none()
    .w(px(extent(cols)))
    .h(px(extent(rows)))
}

/// Dashes around a [`ring_spinner`].
const SPIN_DASHES: usize = 12;
/// One full turn of a [`ring_spinner`].
const SPIN_PERIOD: std::time::Duration = std::time::Duration::from_millis(900);

/// The loading spinner for everything that isn't a session at work (those
/// keep [`mini_glyph_spinner`] and [`gradient_spinner`]): a ring of short
/// dashes, brightest at the head and fading round to a gap at the tail,
/// turning clockwise. Only the dashes paint, so it sits on glass, washes and
/// images alike. It repaints from its own cached view, so the chrome around
/// it stays still; with reduced motion it rests.
pub fn ring_spinner(
    key: impl Into<SharedString>,
    diameter: f32,
    tint: gpui::Hsla,
) -> impl IntoElement {
    RingSpinner {
        key: key.into(),
        diameter,
        tint,
    }
}

#[derive(IntoElement)]
struct RingSpinner {
    key: SharedString,
    diameter: f32,
    tint: gpui::Hsla,
}

impl RenderOnce for RingSpinner {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let (diameter, tint) = (self.diameter, self.tint);
        let view = window.with_global_id(self.key.into(), |id, window| {
            window.with_element_state(id, |previous: Option<Entity<RingSpinnerView>>, _| {
                let view =
                    previous.unwrap_or_else(|| cx.new(|_| RingSpinnerView { diameter, tint }));
                view.update(cx, |view, cx| {
                    if view.diameter != diameter || view.tint != tint {
                        view.diameter = diameter;
                        view.tint = tint;
                        cx.notify();
                    }
                });
                (view.clone(), view)
            })
        });
        view.cached(gpui::StyleRefinement::default().size(px(diameter)))
    }
}

struct RingSpinnerView {
    diameter: f32,
    tint: gpui::Hsla,
}

impl Render for RingSpinnerView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let phase = motion::spin_phase(cx.entity_id(), SPIN_PERIOD, cx).unwrap_or(0.0);
        let (diameter, tint) = (self.diameter, self.tint);
        div().size(px(diameter)).child(
            canvas(
                |_, _, _| (),
                move |bounds, _, window, _| {
                    let stroke = (diameter * 0.11).max(1.5);
                    let center = bounds.center();
                    let radius = diameter / 2.0 - stroke;
                    let slot = std::f32::consts::TAU / SPIN_DASHES as f32;
                    // The head leads at 12 o'clock plus the turn; each dash
                    // behind it is dimmer, the last few nearly gone.
                    let head = -std::f32::consts::FRAC_PI_2 + std::f32::consts::TAU * phase;
                    for age in 0..SPIN_DASHES {
                        let opacity = (1.0 - age as f32 / SPIN_DASHES as f32).powf(1.6);
                        if opacity < 0.04 {
                            continue;
                        }
                        let start = head - slot * age as f32 - slot * 0.62;
                        let end = head - slot * age as f32;
                        let at = |theta: f32| {
                            point(
                                center.x + px(radius * theta.cos()),
                                center.y + px(radius * theta.sin()),
                            )
                        };
                        let mut builder = PathBuilder::stroke(px(stroke));
                        builder.move_to(at(start));
                        for step in 1..=4 {
                            builder.line_to(at(start + (end - start) * step as f32 / 4.0));
                        }
                        if let Ok(path) = builder.build() {
                            window.paint_path(path, tint.opacity(opacity));
                        }
                    }
                },
            )
            .size_full(),
        )
    }
}

/// Stroke width of [`upload_progress_ring`].
const RING_STROKE: f32 = 2.5;
/// Polyline segments for a full circle — plenty for a ≤40px ring.
const RING_SEGMENTS: f32 = 64.0;

/// Radial upload-progress ring with the percent centered — overlaid on a
/// sending echo's attachment thumbnail while its bytes cross the relay
/// (2026-08-18 "Sending… forever" report; the thumbnail is where the wait
/// visibly belongs). A faint full track plus a bright arc growing clockwise
/// from 12 o'clock; gpui paths have no arc primitive, so both are stroked
/// polylines. Fixed white-on-wash palette: the caller dims the image behind
/// it, which reads in both themes.
pub fn upload_progress_ring(percent: u8, diameter: f32) -> AnyElement {
    let ring = progress_arc(
        percent,
        diameter,
        gpui::hsla(0.0, 0.0, 1.0, 0.22),
        gpui::hsla(0.0, 0.0, 1.0, 0.95),
    );
    div()
        .relative()
        .size(px(diameter))
        .flex()
        .items_center()
        .justify_center()
        .child(ring)
        .child(
            div()
                .text_size(px(9.0))
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .text_color(gpui::hsla(0.0, 0.0, 1.0, 0.95))
                .child(SharedString::from(format!("{percent}%"))),
        )
        .into_any_element()
}

/// [`upload_progress_ring`] without the percent, in the caller's colors: for
/// a spot too small to label, such as a sent chip's icon well.
pub fn upload_progress_arc(
    percent: u8,
    diameter: f32,
    track: gpui::Hsla,
    arc: gpui::Hsla,
) -> AnyElement {
    div()
        .relative()
        .size(px(diameter))
        .child(progress_arc(percent, diameter, track, arc))
        .into_any_element()
}

/// A faint full track plus an arc growing clockwise from 12 o'clock, filling
/// its parent.
fn progress_arc(
    percent: u8,
    diameter: f32,
    track: gpui::Hsla,
    arc: gpui::Hsla,
) -> gpui::Canvas<()> {
    let frac = f32::from(percent.min(100)) / 100.0;
    canvas(
        |_, _, _| (),
        move |bounds, _, window, _| {
            paint_arc(bounds, diameter, 0.0, 1.0, track, window);
            paint_arc(bounds, diameter, 0.0, frac, arc, window);
        },
    )
    .absolute()
    .inset_0()
}

pub fn turning_arc(diameter: f32, start: f32, sweep: f32, color: gpui::Hsla) -> gpui::Canvas<()> {
    canvas(
        |_, _, _| (),
        move |bounds, _, window, _| paint_arc(bounds, diameter, start, sweep, color, window),
    )
    .absolute()
    .inset_0()
}

fn paint_arc(
    bounds: gpui::Bounds<gpui::Pixels>,
    diameter: f32,
    start: f32,
    sweep: f32,
    color: gpui::Hsla,
    window: &mut Window,
) {
    if sweep <= 0.0 {
        return;
    }
    let center = bounds.center();
    let radius = diameter / 2.0 - RING_STROKE;
    let steps = ((RING_SEGMENTS * sweep).ceil() as usize).max(2);
    let at = |i: usize| {
        let theta = -std::f32::consts::FRAC_PI_2
            + std::f32::consts::TAU * (start + sweep * (i as f32 / steps as f32));
        point(
            center.x + px(radius * theta.cos()),
            center.y + px(radius * theta.sin()),
        )
    };
    let mut builder = PathBuilder::stroke(px(RING_STROKE));
    builder.move_to(at(0));
    for i in 1..=steps {
        builder.line_to(at(i));
    }
    if let Ok(path) = builder.build() {
        window.paint_path(path, color);
    }
}

/// Full-window boot splash: the app's dot loader (the same [`gradient_spinner`]
/// the session list and the reconnecting line pulse — user request, replacing
/// the hero ascii) over the app background with a quiet status line. While
/// `fading` it plays `splash-out` (150ms hold, then 0.5s fade + 6px lift); the
/// shell removes it once [`SPLASH_OUT`] has run its course.
pub fn splash_overlay(theme: &Theme, fading: bool, view: EntityId, cx: &mut App) -> AnyElement {
    let content = div()
        .absolute()
        .inset_0()
        // Frosted glass, not the opaque page tone (user request): the boot
        // overlay reads like the rest of the chrome — the frost tint over
        // the blurred window background (opaque platforms get the surface
        // tone, since `glass()` collapses to it there).
        .bg(theme.glass())
        .flex()
        .flex_col()
        .items_center()
        .justify_center()
        .gap(px(12.0))
        // Cell 2.5 — the size every other surface runs this spinner at (the
        // "Sending…" strip, the transcript working trailer).
        .child(gradient_spinner(
            "boot-splash-spinner",
            theme,
            2.5,
            view,
            cx,
        ))
        .child(
            div()
                .text_size(crate::typography::ui_rems(12.0))
                .text_color(theme.text_muted.opacity(0.7))
                .child(SharedString::from("Setting up Zeron environment")),
        );
    if fading {
        motion::splash_out("boot-splash-out", content).into_any_element()
    } else {
        content.into_any_element()
    }
}

// Compile-time proof the specs referenced here stay wired to the catalog.
const _: () = {
    assert!(SPLASH_OUT.delay_ms == 150);
    assert!(ZERON_PULSE.duration_ms == 2400);
    assert!(GRADIENT_SPIN.duration_ms == 750);
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mark_stagger_follows_flight_axis() {
        // Tail tip (720, 0) leads: near-maximal stagger (starts deepest into
        // the cycle); head (0, 840) trails with stagger 0.
        let tail = mark_cell_stagger(720.0, 0.0);
        let head = mark_cell_stagger(0.0, 840.0);
        assert!(tail > head, "tail {tail} should lead head {head}");
        assert!((head - 0.0).abs() < 1e-6, "head stagger ≈ 0, got {head}");
        assert!(tail <= MARK_SPREAD + 1e-6, "stagger capped at SPREAD");
        // Every logo cell stays inside [0, SPREAD].
        for &(x, y) in &MARK_CELLS {
            let s = mark_cell_stagger(x, y);
            assert!(
                (0.0..=MARK_SPREAD + 1e-6).contains(&s),
                "cell ({x},{y}) stagger {s}"
            );
        }
    }
}

/// Repaints from its own cached view so the sidebar around it stays put.
pub fn elapsed_label(
    key: impl Into<SharedString>,
    since: chrono::DateTime<chrono::Utc>,
    size: gpui::Rems,
    color: gpui::Hsla,
) -> impl IntoElement {
    ElapsedLabel {
        key: key.into(),
        style: ElapsedStyle { since, size, color },
    }
}

pub fn format_elapsed(seconds: i64) -> String {
    let seconds = seconds.max(0);
    if seconds < 60 {
        format!("{seconds}s")
    } else if seconds < 3600 {
        format!("{}m", seconds / 60)
    } else {
        format!("{}h {}m", seconds / 3600, (seconds % 3600) / 60)
    }
}

#[derive(Clone, Copy, PartialEq)]
struct ElapsedStyle {
    since: chrono::DateTime<chrono::Utc>,
    size: gpui::Rems,
    color: gpui::Hsla,
}

#[derive(IntoElement)]
struct ElapsedLabel {
    key: SharedString,
    style: ElapsedStyle,
}

impl RenderOnce for ElapsedLabel {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let view = window.with_global_id(self.key.into(), |id, window| {
            window.with_element_state(id, |previous: Option<Entity<ElapsedView>>, _| {
                let view = previous.unwrap_or_else(|| {
                    cx.new(|_| ElapsedView {
                        style: self.style,
                        ticking: false,
                    })
                });
                view.update(cx, |view, cx| {
                    if view.style != self.style {
                        view.style = self.style;
                        cx.notify();
                    }
                });
                (view.clone(), view)
            })
        });
        let seconds = (chrono::Utc::now() - self.style.since).num_seconds();
        let widest = if seconds < 60 {
            "00s"
        } else if seconds < 3600 {
            "00m"
        } else {
            "00h 00m"
        };
        let run = gpui::TextRun {
            len: widest.len(),
            font: window.text_style().font(),
            color: self.style.color,
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        let width = window
            .text_system()
            .shape_line(
                widest.into(),
                self.style.size.to_pixels(window.rem_size()),
                &[run],
                None,
            )
            .width()
            .ceil();
        view.cached(
            gpui::StyleRefinement::default()
                .flex_none()
                .w(width)
                .h(px(16.0)),
        )
    }
}

struct ElapsedView {
    style: ElapsedStyle,
    ticking: bool,
}

impl Render for ElapsedView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let seconds = (chrono::Utc::now() - self.style.since).num_seconds().max(0);
        if !self.ticking {
            self.ticking = true;
            let wait = if seconds < 60 { 1 } else { 60 - seconds % 60 };
            cx.spawn(async move |this, cx| {
                cx.background_executor()
                    .timer(std::time::Duration::from_secs(wait as u64))
                    .await;
                this.update(cx, |view, cx| {
                    view.ticking = false;
                    cx.notify();
                })
                .ok();
            })
            .detach();
        }
        div()
            .size_full()
            .flex()
            .justify_end()
            .whitespace_nowrap()
            .text_size(self.style.size)
            .line_height(px(16.0))
            .text_color(self.style.color)
            .child(format_elapsed(seconds))
    }
}
