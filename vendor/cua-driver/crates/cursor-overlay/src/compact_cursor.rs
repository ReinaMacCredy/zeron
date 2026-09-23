//! Compact pointer artwork: a crisp layered arrow drawn as a vector path at
//! the actual input hotspot.
//!
//! Three layers, painted back to front:
//! 1. a soft blurred drop shadow so the arrow reads on light backgrounds,
//! 2. a light rim so it reads on dark backgrounds,
//! 3. a near-black fill.
//!
//! The silhouette is a generic pointer with a short tail — original artwork,
//! no third-party asset. The paint call takes the spring channels (uniform
//! scale, stretch/squash, pivot, lean) so the rebuilt motion is what the user
//! sees.

use std::sync::OnceLock;
use tiny_skia::{Color, FillRule, Paint, Path, PathBuilder, Pixmap, Stroke, Transform};

/// Artwork-local width used to normalize the stretch pivot.
const ART_WIDTH: f64 = 18.4;
/// Artwork-local vertical centre used as the pivot's y coordinate.
const ART_PIVOT_Y: f64 = 14.6;
/// Drop-shadow offset in logical points, down-right of the silhouette.
const SHADOW_OFFSET_POINTS: (f64, f64) = (1.2, 1.8);
/// Blur radius of the drop shadow in logical points. The arrow itself keeps
/// its crisp rim; only the shadow is soft.
const SHADOW_BLUR_POINTS: f64 = 2.0;
/// Peak opacity of the fully-overlapped soft shadow. Low enough that the rim
/// and the fill always dominate the composite.
const SHADOW_PEAK_ALPHA: f64 = 0.42;
/// Separable box-blur passes. Three boxes approximate a Gaussian closely
/// enough that the shadow reads soft rather than banded.
const SHADOW_BLUR_PASSES: usize = 3;

fn arrow() -> &'static Path {
    static PATH: OnceLock<Path> = OnceLock::new();
    PATH.get_or_init(|| {
        let mut p = PathBuilder::new();
        // Tip at the hotspot, tail down-right; a classic pointer silhouette.
        p.move_to(0.0, 0.0);
        p.line_to(0.0, 24.8);
        p.line_to(6.1, 18.9);
        p.line_to(10.3, 28.2);
        p.line_to(14.5, 26.2);
        p.line_to(10.5, 16.8);
        p.line_to(18.4, 16.8);
        p.close();
        p.finish().expect("static compact cursor path")
    })
}

/// One frame of compact-cursor paint state.
#[derive(Debug, Clone, Copy)]
pub struct CompactPaint {
    /// Anchor (the hotspot/tip) in pixmap pixels.
    pub x: f64,
    pub y: f64,
    /// Pixmap pixels per logical point (the display backing scale).
    pub device_scale: f64,
    /// Visual uniform scale from the scale spring (1.0 = authored size).
    pub scale: f64,
    /// Along-travel stretch multiplier (≥ 1 stretches).
    pub stretch: f64,
    /// Perpendicular squash multiplier (≤ 1 flattens).
    pub squash: f64,
    /// Normalized pivot x of the stretch, 0..1 of the artwork width.
    pub pivot_x: f64,
    /// Lean in radians (the travel's tangent minus its chord).
    pub rotation: f64,
    /// Idle-fade alpha, 0..1.
    pub alpha: f64,
    pub pressed: bool,
    /// Arrow body color (stock near-black by default).
    pub fill: [u8; 3],
    /// Arrow body color while pressed.
    pub fill_pressed: [u8; 3],
    /// Outline color (stock light rim by default).
    pub rim: [u8; 3],
    /// Drop-shadow color (stock black by default).
    pub shadow: [u8; 3],
}

/// Build the artwork transform.
///
/// `M(P) = device * [ V * (P - pivot) + pivot ] + anchor`, with `V` the
/// visual rotation × stretch/squash. The authored tip (art-local `0,0`) sits
/// exactly on the anchor at rest; scale and lean pivot about the artwork
/// centre so the shape deforms symmetrically, while the device scale maps
/// logical points to pixmap pixels without moving the tip. The shadow is a
/// device-space offset under the whole thing.
fn affine(paint: &CompactPaint, shadow_offset: (f64, f64)) -> Transform {
    let sx = paint.scale * paint.stretch;
    let sy = paint.scale * paint.squash;
    let (sin, cos) = paint.rotation.sin_cos();
    let (v00, v01, v10, v11) = (cos * sx, -sin * sy, sin * sx, cos * sy);
    let pivot = (paint.pivot_x.clamp(0.0, 1.0) * ART_WIDTH, ART_PIVOT_Y);
    let device = paint.device_scale.max(0.0);
    let tx =
        paint.x + shadow_offset.0 * device + device * (pivot.0 - (v00 * pivot.0 + v01 * pivot.1));
    let ty =
        paint.y + shadow_offset.1 * device + device * (pivot.1 - (v10 * pivot.0 + v11 * pivot.1));
    // Column-major: from_row(sx, ky, kx, sy, tx, ty) with
    // x' = x*sx + y*kx + tx and y' = x*ky + y*sy + ty.
    Transform::from_row(
        (device * v00) as f32,
        (device * v10) as f32,
        (device * v01) as f32,
        (device * v11) as f32,
        tx as f32,
        ty as f32,
    )
}

/// Device-space bounding box of the arrow after `ts` is applied.
fn transformed_arrow_bounds(ts: Transform) -> Option<(f64, f64, f64, f64)> {
    let bounds = arrow().bounds();
    let corners = [
        tiny_skia::Point::from_xy(bounds.left(), bounds.top()),
        tiny_skia::Point::from_xy(bounds.right(), bounds.top()),
        tiny_skia::Point::from_xy(bounds.left(), bounds.bottom()),
        tiny_skia::Point::from_xy(bounds.right(), bounds.bottom()),
    ];
    let (mut min_x, mut min_y, mut max_x, mut max_y) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
    for mut corner in corners {
        ts.map_point(&mut corner);
        if !corner.x.is_finite() || !corner.y.is_finite() {
            return None;
        }
        min_x = min_x.min(corner.x as f64);
        min_y = min_y.min(corner.y as f64);
        max_x = max_x.max(corner.x as f64);
        max_y = max_y.max(corner.y as f64);
    }
    Some((min_x, min_y, max_x, max_y))
}

/// Kernel support (device pixels) of the multi-pass box blur.
fn shadow_support_px(blur_radius: i64) -> i64 {
    blur_radius * SHADOW_BLUR_PASSES as i64 + 2
}

/// Paint the soft drop shadow: the arrow silhouette, offset down-right,
/// blurred and faint. Rendered into a small scratch pixmap and composited, so
/// the blur can never smear the rim, the fill, or the click-pulse ring —
/// everything else is painted after this layer.
fn paint_soft_shadow(pm: &mut Pixmap, paint: &CompactPaint, alpha: f64) {
    let device = paint.device_scale.max(0.0);
    if device <= 0.0 {
        return;
    }
    let ts = affine(paint, SHADOW_OFFSET_POINTS);
    let Some((min_x, min_y, max_x, max_y)) = transformed_arrow_bounds(ts) else {
        return;
    };
    let blur_radius = (SHADOW_BLUR_POINTS * device).round().max(1.0) as i64;
    let support = shadow_support_px(blur_radius);
    let x0 = (min_x.floor() as i64 - support).max(0);
    let y0 = (min_y.floor() as i64 - support).max(0);
    let x1 = (max_x.ceil() as i64 + support).min(pm.width() as i64 - 1);
    let y1 = (max_y.ceil() as i64 + support).min(pm.height() as i64 - 1);
    if x1 < x0 || y1 < y0 {
        return;
    }
    let (w, h) = ((x1 - x0 + 1) as u32, (y1 - y0 + 1) as u32);
    let Some(mut scratch) = Pixmap::new(w, h) else {
        return;
    };
    let [sr, sg, sb] = paint.shadow;
    let shadow = Paint {
        shader: tiny_skia::Shader::SolidColor(Color::from_rgba8(
            sr,
            sg,
            sb,
            (alpha * SHADOW_PEAK_ALPHA * 255.0) as u8,
        )),
        anti_alias: true,
        ..Default::default()
    };
    scratch.fill_path(
        arrow(),
        &shadow,
        FillRule::Winding,
        ts.post_translate(-(x0 as f32), -(y0 as f32)),
        None,
    );
    box_blur_rgba(scratch.data_mut(), w, h, blur_radius as u32);
    pm.draw_pixmap(
        x0 as i32,
        y0 as i32,
        scratch.as_ref(),
        &tiny_skia::PixmapPaint::default(),
        Transform::identity(),
        None,
    );
}

/// Separable three-pass box blur over premultiplied RGBA bytes. Box blurs of
/// premultiplied components preserve the color/coverage ratio of a solid
/// silhouette, which is all this shadow needs.
fn box_blur_rgba(data: &mut [u8], width: u32, height: u32, radius: u32) {
    if radius == 0 {
        return;
    }
    let (w, h) = (width as usize, height as usize);
    if w == 0 || h == 0 || data.len() < w * h * 4 {
        return;
    }
    let mut scratch = vec![0u8; w * h * 4];
    for _ in 0..SHADOW_BLUR_PASSES {
        box_blur_pass(data, &mut scratch, w, h, radius as usize);
    }
}

/// One separable box-blur pass (horizontal into `scratch`, vertical back).
/// Samples clamp at the edges, so the border keeps its value instead of
/// darkening.
fn box_blur_pass(data: &mut [u8], scratch: &mut [u8], w: usize, h: usize, radius: usize) {
    let window = radius * 2 + 1;
    let half = window as u32 / 2;
    let clamp = |value: i64, max: i64| value.clamp(0, max) as usize;

    for y in 0..h {
        let row = y * w * 4;
        for c in 0..4 {
            let mut sum = 0u32;
            for k in 0..window {
                sum += u32::from(data[row + clamp(k as i64 - radius as i64, w as i64 - 1) * 4 + c]);
            }
            for x in 0..w {
                scratch[row + x * 4 + c] = ((sum + half) / window as u32) as u8;
                let drop = clamp(x as i64 - radius as i64, w as i64 - 1);
                let add = clamp(x as i64 + radius as i64 + 1, w as i64 - 1);
                sum =
                    sum - u32::from(data[row + drop * 4 + c]) + u32::from(data[row + add * 4 + c]);
            }
        }
    }

    for x in 0..w {
        for c in 0..4 {
            let mut sum = 0u32;
            for k in 0..window {
                sum += u32::from(
                    scratch[(clamp(k as i64 - radius as i64, h as i64 - 1) * w + x) * 4 + c],
                );
            }
            for y in 0..h {
                data[(y * w + x) * 4 + c] = ((sum + half) / window as u32) as u8;
                let drop = clamp(y as i64 - radius as i64, h as i64 - 1);
                let add = clamp(y as i64 + radius as i64 + 1, h as i64 - 1);
                sum = sum - u32::from(scratch[(drop * w + x) * 4 + c])
                    + u32::from(scratch[(add * w + x) * 4 + c]);
            }
        }
    }
}

pub fn paint(pm: &mut Pixmap, paint: &CompactPaint) {
    if paint.alpha <= 0.0 || paint.scale <= 0.0 || !paint.scale.is_finite() {
        return;
    }
    let alpha = paint.alpha.clamp(0.0, 1.0);

    // Soft shadow: the same silhouette, offset down-right, blurred, and faint.
    // The stock shadow color is black; a session style can repaint it.
    paint_soft_shadow(pm, paint, alpha);

    // Light rim: keeps the arrow visible on dark backgrounds.
    let [rr, rg, rb] = paint.rim;
    let rim = Paint {
        shader: tiny_skia::Shader::SolidColor(Color::from_rgba8(
            rr,
            rg,
            rb,
            (alpha * 0.95 * 255.0) as u8,
        )),
        anti_alias: true,
        ..Default::default()
    };
    pm.stroke_path(
        arrow(),
        &rim,
        &Stroke {
            width: 2.3,
            line_join: tiny_skia::LineJoin::Round,
            line_cap: tiny_skia::LineCap::Round,
            ..Default::default()
        },
        affine(paint, (0.0, 0.0)),
        None,
    );

    // Arrow body; the press lightens it a touch so the shrink reads.
    let (r, g, b) = if paint.pressed {
        let [r, g, b] = paint.fill_pressed;
        (r, g, b)
    } else {
        let [r, g, b] = paint.fill;
        (r, g, b)
    };
    let fill = Paint {
        shader: tiny_skia::Shader::SolidColor(Color::from_rgba8(
            r,
            g,
            b,
            (alpha * 0.97 * 255.0) as u8,
        )),
        anti_alias: true,
        ..Default::default()
    };
    pm.fill_path(
        arrow(),
        &fill,
        FillRule::Winding,
        affine(paint, (0.0, 0.0)),
        None,
    );
}

/// The click landmark: a soft violet ring that expands from the press point
/// and fades out (`radius`/`alpha` arrive pre-computed from the pulse phase).
/// Drawn under the arrow — it marks where input landed, never where it is.
pub fn paint_pulse(pm: &mut Pixmap, x: f32, y: f32, radius: f32, alpha: f32, scale: f32) {
    if alpha <= 0.0 || radius <= 0.0 {
        return;
    }
    let mut p = PathBuilder::new();
    p.push_circle(0.0, 0.0, radius / scale.max(0.001));
    let Some(path) = p.finish() else { return };
    // violet-300, matching the in-chat preview halo's accent family.
    let ring = Paint {
        shader: tiny_skia::Shader::SolidColor(Color::from_rgba8(
            0xC4,
            0xB5,
            0xFD,
            (alpha.clamp(0.0, 1.0) * 255.0) as u8,
        )),
        anti_alias: true,
        ..Default::default()
    };
    pm.stroke_path(
        &path,
        &ring,
        &Stroke {
            width: 1.6,
            ..Default::default()
        },
        Transform::from_row(scale, 0.0, 0.0, scale, x, y),
        None,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{CursorAction, CursorConfig, OverlayCommand, RenderStateCore};

    fn compact() -> RenderStateCore {
        RenderStateCore::new(CursorConfig {
            compact: true,
            ..CursorConfig::default()
        })
    }

    fn stationary_paint() -> CompactPaint {
        CompactPaint {
            x: 32.0,
            y: 32.0,
            device_scale: 1.0,
            scale: 1.0,
            stretch: 1.0,
            squash: 1.0,
            pivot_x: 0.5,
            rotation: 0.0,
            alpha: 1.0,
            pressed: false,
            fill: crate::STOCK_CURSOR_FILL,
            fill_pressed: crate::STOCK_CURSOR_FILL_PRESSED,
            rim: crate::STOCK_CURSOR_RIM,
            shadow: crate::STOCK_CURSOR_SHADOW,
        }
    }

    #[test]
    fn compact_moves_and_clicks_use_exact_hotspot_without_a_classic_path_or_spring() {
        let mut c = compact();
        for (x, y) in [(20.0, 30.0), (1900.0, 100.0), (3.0, 4.0)] {
            c.apply_command_base(
                OverlayCommand::MoveTo {
                    x,
                    y,
                    end_heading_radians: 2.0,
                },
                true,
                true,
            );
            assert_eq!(c.pos, (x, y));
            assert!(c.path.is_none());
            assert!(c.spring.is_none());
            c.apply_command_base(OverlayCommand::ClickPulse { x, y }, true, true);
            assert_eq!(c.pos, (x, y));
            assert!(c.click_t.is_none());
            assert!(c.spring.is_none());
        }
    }

    #[test]
    fn compact_hops_travel_the_painted_arrow_only() {
        let mut c = compact();
        c.pos = (100.0, 100.0);
        c.apply_command_base(
            OverlayCommand::SnapTo {
                x: 600.0,
                y: 400.0,
                heading_radians: None,
            },
            true,
            true,
        );
        // The logical position is already the hotspot — nothing waits.
        assert_eq!(c.pos, (600.0, 400.0));
        assert!(
            c.compact_motion.is_some(),
            "a 500px hop should record a travel"
        );
        assert_eq!(c.compact_display_pos(), (100.0, 100.0));
        assert!(c.compact_animating());
        c.tick_compact(0.05);
        let (mx, my) = c.compact_display_pos();
        assert!(
            (mx - 100.0).hypot(my - 100.0) > 1.0,
            "the arrow must have left the origin"
        );
        assert!(
            (mx - 600.0).hypot(my - 400.0) > 1.0,
            "the arrow must not have arrived yet"
        );
        for _ in 0..600 {
            c.tick_compact(1.0 / 120.0);
            if c.compact_motion.is_none() {
                break;
            }
        }
        assert!(c.compact_motion.is_none(), "the travel must settle");
        assert_eq!(c.compact_display_pos(), (600.0, 400.0));
        assert!(!c.compact_animating());
    }

    #[test]
    fn compact_small_hops_offscreen_moves_and_drags_snap_silently() {
        let mut c = compact();
        // Never placed: first placement must appear instantly, not fly in.
        c.apply_command_base(
            OverlayCommand::SnapTo {
                x: 500.0,
                y: 500.0,
                heading_radians: None,
            },
            true,
            true,
        );
        assert!(c.compact_motion.is_none());
        // A sub-threshold hop snaps — micro-corrections must not smear.
        c.apply_command_base(
            OverlayCommand::SnapTo {
                x: 520.0,
                y: 510.0,
                heading_radians: None,
            },
            true,
            true,
        );
        assert!(c.compact_motion.is_none());
        // A held button keeps tracking tight to the real drag path.
        c.apply_command_base(OverlayCommand::SetPressed(true), true, true);
        c.apply_command_base(
            OverlayCommand::SnapTo {
                x: 900.0,
                y: 700.0,
                heading_radians: None,
            },
            true,
            true,
        );
        assert!(c.compact_motion.is_none());
        assert_eq!(c.compact_display_pos(), (900.0, 700.0));
    }

    #[test]
    fn compact_chained_travel_continues_from_the_displayed_spot() {
        let mut c = compact();
        c.pos = (100.0, 100.0);
        c.apply_command_base(
            OverlayCommand::SnapTo {
                x: 700.0,
                y: 100.0,
                heading_radians: None,
            },
            true,
            true,
        );
        let first = c.compact_motion.as_ref().unwrap();
        assert_eq!(first.from, (100.0, 100.0));
        c.tick_compact(0.1);
        let mid = c.compact_display_pos();
        assert!(mid.0 > 100.0 && mid.0 < 700.0);
        c.apply_command_base(
            OverlayCommand::SnapTo {
                x: 700.0,
                y: 500.0,
                heading_radians: None,
            },
            true,
            true,
        );
        let second = c
            .compact_motion
            .as_ref()
            .expect("the retarget should record a new travel");
        assert!(
            (second.from.0 - mid.0).abs() < 1.0e-9 && (second.from.1 - mid.1).abs() < 1.0e-9,
            "the retarget must chain from the painted spot: {mid:?} -> {:?}",
            second.from
        );
    }

    #[test]
    fn compact_click_leaves_a_ring_that_expands_and_expires() {
        let mut c = compact();
        c.pos = (200.0, 200.0);
        c.apply_command_base(
            OverlayCommand::ClickPulse { x: 260.0, y: 240.0 },
            true,
            true,
        );
        let pulse = c.compact_pulse.expect("a click should leave a ring");
        assert_eq!((pulse.x, pulse.y), (260.0, 240.0));
        let (_, _, r0, a0) = c.compact_pulse_ring().unwrap();
        assert!(r0 > 3.0 && a0 > 0.3);
        c.tick_compact(crate::COMPACT_PULSE_SECS * 0.5);
        let (_, _, r1, a1) = c.compact_pulse_ring().unwrap();
        assert!(r1 > r0 && a1 < a0);
        c.tick_compact(crate::COMPACT_PULSE_SECS);
        assert!(c.compact_pulse_ring().is_none());
        assert!(c.compact_pulse.is_none());
    }

    #[test]
    fn compact_reduced_motion_never_travels_or_rings() {
        let mut c = RenderStateCore::new(CursorConfig {
            compact: true,
            reduced_motion: crate::ReducedMotion::On,
            ..CursorConfig::default()
        });
        c.pos = (100.0, 100.0);
        c.apply_command_base(
            OverlayCommand::SnapTo {
                x: 900.0,
                y: 700.0,
                heading_radians: None,
            },
            true,
            true,
        );
        c.apply_command_base(
            OverlayCommand::ClickPulse { x: 920.0, y: 720.0 },
            true,
            true,
        );
        assert!(c.compact_motion.is_none());
        assert!(c.compact_pulse.is_none());
        assert!(!c.compact_animating());
        assert_eq!(c.compact_display_pos(), (920.0, 720.0));
    }

    #[test]
    fn compact_loading_breath_expires_with_the_action() {
        let mut c = compact();
        c.pos = (100.0, 100.0);
        // Native semantic actions announce begin/end through the cursor event
        // sink; a held action is the cursor's loading state.
        c.apply_command_base(
            OverlayCommand::BeginAction {
                action: CursorAction::Drag,
                delivery: None,
                target: None,
            },
            true,
            true,
        );
        assert_eq!(c.compact_activity(), crate::CompactActivity::Loading);
        assert!(c.compact_animating(), "the loading breath needs frames");
        c.apply_command_base(OverlayCommand::EndAction(CursorAction::Drag), true, true);
        for _ in 0..1200 {
            c.tick_compact(1.0 / 120.0);
        }
        assert_eq!(c.compact_activity(), crate::CompactActivity::Idle);
        c.idle_secs = 0.0;
        assert!(
            !c.compact_animating(),
            "a settled compact cursor must not keep the 60 fps loop alive"
        );
    }

    #[test]
    fn compact_persistent_cursor_parks_visibly_without_animation() {
        let mut c = compact();
        c.pos = (100.0, 100.0);
        c.motion.idle_hide_ms = 0.0;
        c.apply_command_base(
            OverlayCommand::ClickPulse { x: 100.0, y: 100.0 },
            true,
            true,
        );
        for _ in 0..1200 {
            c.tick_compact(1.0 / 120.0);
        }
        c.idle_secs = 300.0;
        c.tick_compact(1.0);
        assert_eq!(c.idle_alpha, 1.0);
        assert!(!c.compact_animating());
        c.apply_command_base(OverlayCommand::SetEnabled(false), true, true);
        assert!(!c.visible);
        assert!(!c.compact_animating());
    }

    #[test]
    fn compact_long_idle_backstop_preserves_visible_model_turns_then_fades() {
        let mut c = compact();
        c.pos = (100.0, 100.0);
        c.motion.idle_hide_ms = 60_000.0;
        c.idle_secs = 10.0;
        c.tick_compact(0.05);
        assert_eq!(c.idle_alpha, 1.0);
        assert!(!c.compact_animating());
        c.idle_secs = 60.05;
        c.tick_compact(0.05);
        assert!(c.idle_alpha < 1.0 && c.idle_alpha > 0.0);
        assert!(c.compact_animating());
        c.idle_secs = 60.2;
        c.tick_compact(0.05);
        assert_eq!(c.idle_alpha, 0.0);
        assert!(!c.compact_animating());
    }

    #[test]
    fn compact_idle_hide_fades_out_instead_of_popping() {
        let mut c = compact();
        c.pos = (100.0, 100.0);
        c.motion.idle_hide_ms = 900.0;
        c.idle_secs = 0.95;
        assert!(c.compact_animating(), "the fade needs frame ticks");
        c.tick_compact(0.05);
        assert!(c.idle_alpha < 1.0 && c.idle_alpha > 0.0);
        c.idle_secs = 1.2;
        c.tick_compact(0.05);
        assert_eq!(c.idle_alpha, 0.0);
        assert!(!c.compact_animating());
    }

    #[test]
    fn compact_travel_and_ring_paint_pixels() {
        let mut c = compact();
        c.pos = (40.0, 40.0);
        c.apply_command_base(
            OverlayCommand::SnapTo {
                x: 160.0,
                y: 160.0,
                heading_radians: None,
            },
            true,
            true,
        );
        c.apply_command_base(
            OverlayCommand::ClickPulse { x: 165.0, y: 165.0 },
            true,
            true,
        );
        c.tick_compact(0.1);
        let mut frame = Pixmap::new(256, 256).unwrap();
        crate::paint_cursor(&mut frame, &c, 0.0, 0.0, None, 1.0);
        assert!(frame.pixels().iter().any(|p| p.alpha() != 0));
        // Mid-travel the arrow sits between the two points, not on either.
        let (dx, dy) = c.compact_display_pos();
        assert!(dx > 40.0 && dx < 165.0 && dy > 40.0 && dy < 165.0);
    }

    #[test]
    fn compact_artwork_is_stable_across_actions_heading_and_time() {
        let mut c = compact();
        c.pos = (16.0, 16.0);
        let mut baseline = Pixmap::new(128, 128).unwrap();
        crate::paint_cursor(&mut baseline, &c, 0.0, 0.0, None, 2.0);
        assert!(baseline.pixels().iter().any(|p| p.alpha() != 0));
        for action in [
            CursorAction::Click,
            CursorAction::Navigate,
            CursorAction::Text,
            CursorAction::Scroll,
        ] {
            c.visual.begin(action, None, None);
            c.visual.tick(0.15);
            c.heading = 2.8;
            let mut frame = Pixmap::new(128, 128).unwrap();
            crate::paint_cursor(&mut frame, &c, 0.0, 0.0, None, 2.0);
            assert_eq!(baseline.data(), frame.data());
        }
        if let Ok(path) = std::env::var("SYNARA_CURSOR_PREVIEW_PATH") {
            image::save_buffer(path, baseline.data(), 128, 128, image::ColorType::Rgba8).unwrap();
        }
    }

    #[test]
    fn compact_hidden_pointer_has_no_pixels_or_hover_poll() {
        let mut c = compact();
        c.pos = (16.0, 16.0);
        c.apply_command_base(OverlayCommand::SetSessionLabel("Synara".into()), true, true);
        assert!(!c.session_badge_needs_hover_poll());
        c.apply_command_base(OverlayCommand::SetEnabled(false), true, true);
        let mut frame = Pixmap::new(128, 128).unwrap();
        crate::paint_cursor(&mut frame, &c, 0.0, 0.0, None, 2.0);
        assert!(frame.pixels().iter().all(|p| p.alpha() == 0));
    }

    fn ink_bounds(pm: &Pixmap) -> Option<(u32, u32, u32, u32)> {
        let mut min_x = u32::MAX;
        let mut min_y = u32::MAX;
        let mut max_x = 0;
        let mut max_y = 0;
        for (index, pixel) in pm.pixels().iter().enumerate() {
            if pixel.alpha() == 0 {
                continue;
            }
            let x = index as u32 % pm.width();
            let y = index as u32 / pm.width();
            min_x = min_x.min(x);
            min_y = min_y.min(y);
            max_x = max_x.max(x);
            max_y = max_y.max(y);
        }
        (min_x != u32::MAX).then_some((min_x, min_y, max_x, max_y))
    }

    #[test]
    fn paint_layers_a_shadow_rim_and_fill_at_the_anchor() {
        let mut frame = Pixmap::new(64, 64).unwrap();
        paint(&mut frame, &stationary_paint());
        // The tip occupies the anchor; the rim/shadow cover more pixels below
        // and to the right of it.
        let opaque = frame
            .pixels()
            .iter()
            .filter(|pixel| pixel.alpha() > 200)
            .count();
        assert!(opaque > 40, "the arrow must be solidly painted: {opaque}");
        assert!(frame.pixel(32, 32).unwrap().alpha() >= 200);
        assert_eq!(frame.pixel(56, 8).unwrap().alpha(), 0);
    }

    /// Every pixel at or above `threshold` alpha, as `(x, y)` pairs.
    fn ink_with_alpha(pm: &Pixmap, threshold: u8) -> Vec<(u32, u32)> {
        pm.pixels()
            .iter()
            .enumerate()
            .filter(|(_, pixel)| pixel.alpha() >= threshold)
            .map(|(index, _)| (index as u32 % pm.width(), index as u32 / pm.width()))
            .collect()
    }

    #[test]
    fn paint_anchors_the_tip_on_every_device_scale() {
        for device in [1.0_f64, 2.0, 3.0] {
            let size = (64.0 * device) as u32;
            let anchor = (32.0 * device).round() as u32;
            let mut frame = Pixmap::new(size, size).unwrap();
            paint(
                &mut frame,
                &CompactPaint {
                    x: anchor as f64,
                    y: anchor as f64,
                    device_scale: device,
                    ..stationary_paint()
                },
            );

            // The crisp rim + fill stay anchored: no opaque ink starts
            // farther than the round-join margin above or left of the tip.
            let rim_margin = (1.6 * device).ceil() as i64;
            let opaque = ink_with_alpha(&frame, 200);
            assert!(!opaque.is_empty(), "device {device}: no opaque ink painted");
            let opaque_min_x = opaque.iter().map(|(x, _)| *x).min().unwrap();
            let opaque_min_y = opaque.iter().map(|(_, y)| *y).min().unwrap();
            assert!(
                i64::from(opaque_min_x) >= i64::from(anchor) - rim_margin,
                "device {device}: opaque ink starts left of the tip at {opaque_min_x}"
            );
            assert!(
                i64::from(opaque_min_y) >= i64::from(anchor) - rim_margin,
                "device {device}: opaque ink starts above the tip at {opaque_min_y}"
            );

            // The faint blurred shadow trails the offset and the blur support,
            // and the whole footprint stays inside that bounded envelope.
            let blur = (SHADOW_BLUR_POINTS * device).round().max(1.0) as i64;
            let support = shadow_support_px(blur);
            let offset_x = (SHADOW_OFFSET_POINTS.0 * device).ceil() as i64;
            let offset_y = (SHADOW_OFFSET_POINTS.1 * device).ceil() as i64;
            let (min_x, min_y, max_x, max_y) = ink_bounds(&frame).expect("ink");
            assert!(
                i64::from(min_x) >= i64::from(anchor) + offset_x - support - 2,
                "device {device}: shadow leaks left of the tip: {min_x}"
            );
            assert!(
                i64::from(min_y) >= i64::from(anchor) + offset_y - support - 2,
                "device {device}: shadow leaks above the tip: {min_y}"
            );
            let art = arrow().bounds();
            assert!(
                i64::from(max_x)
                    <= i64::from(anchor)
                        + (f64::from(art.right() + 1.2) * device) as i64
                        + offset_x
                        + support
                        + 2,
                "device {device}: ink leaks right: {max_x}"
            );
            assert!(
                i64::from(max_y)
                    <= i64::from(anchor)
                        + (f64::from(art.bottom() + 1.2) * device) as i64
                        + offset_y
                        + support
                        + 2,
                "device {device}: ink leaks down: {max_y}"
            );

            // The silhouette extends down-right and nothing is painted past
            // its bounded footprint in the bottom-right corner.
            assert!(
                max_x > anchor + (10.0 * device) as u32,
                "art must extend right"
            );
            assert!(
                max_y > anchor + (20.0 * device) as u32,
                "art must extend down"
            );
            let corner = (size - 1, size - 1);
            assert_eq!(
                frame.pixel(corner.0, corner.1).unwrap().alpha(),
                0,
                "device {device}: paint must not leak past the artwork"
            );
        }
    }

    #[test]
    fn box_blur_turns_a_hard_edge_into_a_ramp() {
        // One full-alpha row on an empty mask; three 2-px box passes must
        // leave a multi-pixel falloff above and below instead of a step.
        let mut data = vec![0u8; 21 * 21 * 4];
        for x in 0..21 {
            data[(10 * 21 + x) * 4 + 3] = 255;
        }
        box_blur_rgba(&mut data, 21, 21, 2);
        let column: Vec<u8> = (0..21).map(|y| data[(y * 21 + 10) * 4 + 3]).collect();
        let lit = column.iter().filter(|alpha| **alpha > 0).count();
        assert!(lit >= 8, "the kernel must spread the row: {column:?}");
        assert_eq!(column[0], 0, "the kernel support is bounded: {column:?}");
        assert_eq!(column[20], 0, "the kernel support is bounded: {column:?}");
        let ramp = column
            .windows(2)
            .filter(|pair| pair[0] > pair[1] && pair[0] - pair[1] < 60)
            .count();
        assert!(ramp >= 3, "the falloff must be gradual: {column:?}");
    }

    #[test]
    fn soft_shadow_is_blurred_faint_and_offset_below_right() {
        let anchor = 40.0_f64;
        let mut frame = Pixmap::new(128, 128).unwrap();
        paint(
            &mut frame,
            &CompactPaint {
                x: anchor,
                y: anchor,
                ..stationary_paint()
            },
        );

        let art = arrow().bounds();
        let blur = (SHADOW_BLUR_POINTS).round().max(1.0) as i64;
        let support = shadow_support_px(blur);
        // The blur tail below the silhouette: from the arrow's bottom edge to
        // the full kernel reach of the offset shadow.
        let top = (anchor + f64::from(art.bottom())) as u32 + 1;
        let bottom = top + support as u32;
        let mut alphas = Vec::new();
        for y in top..=bottom {
            for x in 24..=72 {
                let alpha = frame.pixel(x, y).unwrap().alpha();
                if alpha > 0 {
                    alphas.push(alpha);
                }
            }
        }
        assert!(
            alphas.len() >= 40,
            "the drop shadow must paint a visible tail: {alphas:?}"
        );

        let peak = *alphas.iter().max().unwrap();
        assert!(peak > 10, "the drop shadow must be visible: {peak}");
        assert!(
            peak < 140,
            "the drop shadow must stay faint (fill/rim dominate): {peak}"
        );

        // The blur produces a gradual alpha ramp, not a hard silhouette edge.
        let mut levels = alphas.clone();
        levels.sort_unstable();
        levels.dedup();
        assert!(
            levels.len() >= 5,
            "the shadow edge must be soft, found levels {levels:?}"
        );

        // The shadow sits down-right: at the same distance up-left of the
        // tip the sheet stays empty.
        let up_left = frame
            .pixel(
                (anchor as u32).saturating_sub(support as u32 + 2),
                (anchor as u32).saturating_sub(support as u32 + 2),
            )
            .unwrap()
            .alpha();
        assert_eq!(up_left, 0, "the stock shadow is down-right only");
    }

    #[test]
    fn paint_rotation_stretch_and_squash_change_the_pixels() {
        let mut baseline = Pixmap::new(96, 96).unwrap();
        paint(&mut baseline, &stationary_paint());

        let mut rotated = Pixmap::new(96, 96).unwrap();
        paint(
            &mut rotated,
            &CompactPaint {
                rotation: 0.6,
                ..stationary_paint()
            },
        );
        assert_ne!(baseline.data(), rotated.data(), "lean must rotate the art");

        let mut stretched = Pixmap::new(96, 96).unwrap();
        let mut squash = stationary_paint();
        squash.stretch = 1.35;
        squash.squash = 0.85;
        paint(&mut stretched, &squash);
        assert_ne!(
            baseline.data(),
            stretched.data(),
            "stretch/squash must deform the art"
        );

        let mut pressed = Pixmap::new(96, 96).unwrap();
        paint(
            &mut pressed,
            &CompactPaint {
                pressed: true,
                ..stationary_paint()
            },
        );
        assert_ne!(baseline.data(), pressed.data());
    }

    fn has_color(pm: &Pixmap, target: [u8; 3], tolerance: i16, min_alpha: u8) -> bool {
        pm.pixels().iter().any(|pixel| {
            pixel.alpha() >= min_alpha && {
                let color = pixel.demultiply();
                let close = |a: u8, b: u8| (i16::from(a) - i16::from(b)).abs() <= tolerance;
                close(color.red(), target[0])
                    && close(color.green(), target[1])
                    && close(color.blue(), target[2])
            }
        })
    }

    #[test]
    fn stock_paint_is_monochrome_and_matches_the_authored_ink() {
        // The stock default must stay byte-identical to the authored
        // near-black/white artwork: a session style with every channel
        // omitted paints the same pixels as the hardcoded defaults.
        let mut stock = Pixmap::new(64, 64).unwrap();
        paint(&mut stock, &stationary_paint());

        let mut explicit = Pixmap::new(64, 64).unwrap();
        paint(
            &mut explicit,
            &CompactPaint {
                fill: crate::STOCK_CURSOR_FILL,
                fill_pressed: crate::STOCK_CURSOR_FILL_PRESSED,
                rim: crate::STOCK_CURSOR_RIM,
                shadow: crate::STOCK_CURSOR_SHADOW,
                ..stationary_paint()
            },
        );
        assert_eq!(stock.data(), explicit.data());

        // Near-black body ink and near-white rim ink are both present in the
        // stock pointer (premultiplied pixels are demultiplied first).
        assert!(has_color(&stock, crate::STOCK_CURSOR_FILL, 3, 200));
        assert!(has_color(&stock, crate::STOCK_CURSOR_RIM, 3, 100));
    }

    #[test]
    fn session_style_repaints_the_fill_rim_and_shadow() {
        let mut baseline = Pixmap::new(64, 64).unwrap();
        paint(&mut baseline, &stationary_paint());

        let mut styled = Pixmap::new(64, 64).unwrap();
        paint(
            &mut styled,
            &CompactPaint {
                fill: [0xE8, 0x6C, 0x00],
                fill_pressed: [0xE8, 0x6C, 0x00],
                rim: [0x10, 0x20, 0x30],
                shadow: [0x40, 0x00, 0x80],
                ..stationary_paint()
            },
        );
        assert_ne!(baseline.data(), styled.data(), "the style must repaint");
        assert!(has_color(&styled, [0xE8, 0x6C, 0x00], 3, 200));
        assert!(has_color(&styled, [0x10, 0x20, 0x30], 3, 100));
        // The shadow is the faintest layer; it is offset below-right of the
        // tip where nothing else paints.
        assert!(has_color(&styled, [0x40, 0x00, 0x80], 6, 20));
        assert!(
            !has_color(&styled, crate::STOCK_CURSOR_FILL, 2, 200),
            "the stock near-black body ink must be fully replaced"
        );
    }
}
