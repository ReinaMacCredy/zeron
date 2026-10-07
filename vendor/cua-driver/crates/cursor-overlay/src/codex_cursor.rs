use std::f64::consts::TAU;
use std::sync::OnceLock;

use tiny_skia::{
    Color, FillRule, GradientStop, LineCap, LineJoin, Paint, Path, PathBuilder, Pixmap,
    PixmapPaint, Point, RadialGradient, Rect, SpreadMode, Stroke, Transform,
};

use crate::feel::CursorFeel;
use crate::spring::{SpringChannel, SpringSpec};

const ARROW: [(f32, f32); 4] = [(0.0, 0.0), (8.551, 4.181), (3.834, 5.808), (1.795, 9.723)];
const FILL_ROUND: f32 = 0.937;
const RIM_WIDTH: f32 = 0.987;
const FOG_CENTER: (f32, f32) = (1.22, 2.915);
const FOG_RADIUS: f32 = 40.5;
const FOG_PROFILE: [(f32, f32); 11] = [
    (0.0, 0.77),
    (4.5, 0.743),
    (8.5, 0.649),
    (12.5, 0.509),
    (16.5, 0.354),
    (20.5, 0.215),
    (24.5, 0.113),
    (28.5, 0.050),
    (32.5, 0.019),
    (36.5, 0.0052),
    (40.5, 0.0),
];
const TIP_AXIS: (f64, f64) = (-0.584, -0.812);
pub const FOG_FALLBACK: [u8; 3] = [101, 94, 90];
pub const WALLPAPER_SAMPLE_RADIUS: f64 = 25.0;
pub const WALLPAPER_SHADE: f64 = 0.61;
const FILL_SHADE: f32 = 0.81;
const RIM_ALPHA: f32 = 0.81;

pub const PRESS_SCALE: f64 = 0.865;
const PRESS_HOLD_SECS: f64 = 0.12;
pub const TILT_MAX_DEGREES: f64 = 18.0;
const SPIN_SECS_PER_TURN: f64 = 2.0;
const SPIN_RETURN: SpringSpec = SpringSpec::new(0.4, 0.86);
pub const REVEAL_SECS: f64 = 0.22;
const FADE_BLUR_POINTS: f64 = 8.0;

fn arrow() -> &'static Path {
    static PATH: OnceLock<Path> = OnceLock::new();
    PATH.get_or_init(|| {
        let mut p = PathBuilder::new();
        p.move_to(ARROW[0].0, ARROW[0].1);
        for &(x, y) in &ARROW[1..] {
            p.line_to(x, y);
        }
        p.close();
        p.finish().expect("static codex cursor path")
    })
}
#[derive(Debug, Clone, Copy)]
pub struct CodexInput {
    pub position: (f64, f64),
    pub travelling: bool,
    pub scoot: bool,
    pub shown: bool,
    pub pressed: bool,
    pub loading: bool,
    pub reduced_motion: bool,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CodexChannels {
    pub rotation: f64,
    pub scale: f64,
    pub stretch: f64,
    pub squash: f64,
    pub axis: f64,
    pub alpha: f64,
}
#[derive(Debug, Clone, PartialEq)]
pub struct CodexState {
    last: Option<(f64, f64)>,
    axis: f64,
    tilt: SpringChannel,
    tilt_target: f64,
    scale: SpringChannel,
    scale_target: f64,
    stretch: SpringChannel,
    squash: SpringChannel,
    stretch_target: (f64, f64),
    spin: SpringChannel,
    spin_target: f64,
    loading: bool,
    press_hold: f64,
    reveal: f64,
    shown: bool,
}

impl Default for CodexState {
    fn default() -> Self {
        Self {
            last: None,
            axis: 0.0,
            tilt: SpringChannel::new(0.0),
            tilt_target: 0.0,
            scale: SpringChannel::new(1.0),
            scale_target: 1.0,
            stretch: SpringChannel::new(1.0),
            squash: SpringChannel::new(1.0),
            stretch_target: (1.0, 1.0),
            spin: SpringChannel::new(0.0),
            spin_target: 0.0,
            loading: false,
            press_hold: 0.0,
            reveal: 0.0,
            shown: false,
        }
    }
}

impl CodexState {
    pub fn click(&mut self) {
        self.press_hold = PRESS_HOLD_SECS;
    }
    pub fn conceal(&mut self) {
        self.reveal = 0.0;
        self.last = None;
    }

    pub fn tick(&mut self, dt: f64, input: &CodexInput, feel: &CursorFeel) {
        let dt = if dt.is_finite() { dt.max(0.0) } else { 0.0 };
        self.shown = input.shown;
        self.loading = input.loading && !input.reduced_motion;
        self.press_hold = (self.press_hold - dt).max(0.0);
        self.scale_target = if input.pressed || self.press_hold > 0.0 {
            PRESS_SCALE
        } else {
            1.0
        };

        if input.reduced_motion {
            self.last = Some(input.position);
            self.reveal = if input.shown { 1.0 } else { 0.0 };
            self.tilt_target = 0.0;
            self.tilt.snap(0.0);
            self.stretch_target = (1.0, 1.0);
            self.stretch.snap(1.0);
            self.squash.snap(1.0);
            self.spin_target = 0.0;
            self.spin.snap(0.0);
            self.scale.snap(self.scale_target);
            return;
        }

        let velocity = match self.last {
            Some(prev) if input.travelling && dt > 0.0 => (
                (input.position.0 - prev.0) / dt,
                (input.position.1 - prev.1) / dt,
            ),
            _ => (0.0, 0.0),
        };
        self.last = Some(input.position);
        let peak = feel.peak_speed.max(1.0);
        let speed = velocity.0.hypot(velocity.1);
        if speed > 1.0 {
            self.axis = velocity.1.atan2(velocity.0);
        }

        let sideways = TIP_AXIS.0 * velocity.1 - TIP_AXIS.1 * velocity.0;
        self.tilt_target = TILT_MAX_DEGREES.to_radians() * (sideways / peak).clamp(-1.0, 1.0);
        self.tilt.step(self.tilt_target, &feel.rotation_spec(), dt);

        let ratio = (speed / peak).clamp(0.0, 1.0);
        self.stretch_target = if input.scoot {
            (
                1.0 + feel.stretch_x_amount * ratio,
                1.0 - feel.squash_y_amount * ratio,
            )
        } else {
            (1.0, 1.0)
        };
        self.stretch
            .step(self.stretch_target.0, &feel.stretch_spec(), dt);
        self.squash
            .step(self.stretch_target.1, &feel.stretch_spec(), dt);

        self.scale.step(self.scale_target, &feel.press_spec(), dt);

        if self.loading {
            let rate = TAU / SPIN_SECS_PER_TURN;
            self.spin.value += rate * dt;
            self.spin.velocity = rate;
            self.spin_target = self.spin.value;
        } else {
            self.spin_target = (self.spin.value / TAU).round() * TAU;
            self.spin.step(self.spin_target, &SPIN_RETURN, dt);
            if self.spin.settled(self.spin_target, 1.0e-3, 1.0e-2) {
                self.spin.snap(0.0);
                self.spin_target = 0.0;
            }
        }

        self.reveal = if input.shown {
            (self.reveal + dt / REVEAL_SECS).min(1.0)
        } else {
            (self.reveal - dt / REVEAL_SECS).max(0.0)
        };
    }
    pub fn animating(&self) -> bool {
        self.loading
            || self.press_hold > 0.0
            || (self.shown && self.reveal < 1.0)
            || (!self.shown && self.reveal > 0.0)
            || !self.tilt.settled(self.tilt_target, 1.0e-3, 1.0e-2)
            || !self.scale.settled(self.scale_target, 1.0e-3, 1.0e-2)
            || !self.stretch.settled(self.stretch_target.0, 1.0e-3, 1.0e-2)
            || !self.squash.settled(self.stretch_target.1, 1.0e-3, 1.0e-2)
            || !self.spin.settled(self.spin_target, 1.0e-3, 1.0e-2)
    }

    pub fn reveal(&self) -> f64 {
        self.reveal
    }

    pub fn channels(&self) -> CodexChannels {
        let t = self.reveal.clamp(0.0, 1.0);
        CodexChannels {
            rotation: self.tilt.value + self.spin.value,
            scale: self.scale.value,
            stretch: self.stretch.value,
            squash: self.squash.value,
            axis: self.axis,
            alpha: t * t * (3.0 - 2.0 * t),
        }
    }
}
#[derive(Debug, Clone, Copy)]
pub struct CodexPaint {
    pub x: f64,
    pub y: f64,
    pub device_scale: f64,
    pub channels: CodexChannels,
    pub fog: [u8; 3],
    pub fill: Option<[u8; 3]>,
    pub rim: [u8; 3],
}

fn shade(rgb: [u8; 3], factor: f32) -> [u8; 3] {
    rgb.map(|c| (f32::from(c) * factor).round().clamp(0.0, 255.0) as u8)
}

fn mix(top: [u8; 3], bottom: [u8; 3], alpha: f32) -> [u8; 3] {
    let mut out = [0u8; 3];
    for i in 0..3 {
        out[i] = (f32::from(top[i]) * alpha + f32::from(bottom[i]) * (1.0 - alpha))
            .round()
            .clamp(0.0, 255.0) as u8;
    }
    out
}
pub fn fog_from_wallpaper(rgb: [u8; 3]) -> [u8; 3] {
    shade(rgb, WALLPAPER_SHADE as f32)
}

fn solid(rgb: [u8; 3]) -> Paint<'static> {
    Paint {
        shader: tiny_skia::Shader::SolidColor(Color::from_rgba8(rgb[0], rgb[1], rgb[2], 255)),
        anti_alias: true,
        ..Default::default()
    }
}

fn paint_silhouette(pm: &mut Pixmap, rgb: [u8; 3], outset: f32, ts: Transform) {
    let paint = solid(rgb);
    pm.fill_path(arrow(), &paint, FillRule::Winding, ts, None);
    pm.stroke_path(
        arrow(),
        &paint,
        &Stroke {
            width: outset * 2.0,
            line_join: LineJoin::Round,
            line_cap: LineCap::Round,
            ..Default::default()
        },
        ts,
        None,
    );
}
pub fn paint(pm: &mut Pixmap, paint: &CodexPaint) {
    let ch = paint.channels;
    let device = paint.device_scale.max(0.0);
    if ch.alpha <= 0.004 || ch.scale <= 0.0 || !ch.scale.is_finite() || device <= 0.0 {
        return;
    }
    let alpha = ch.alpha.clamp(0.0, 1.0);
    let blur = ((1.0 - alpha) * FADE_BLUR_POINTS * device).round().max(0.0) as i64;
    let center = (
        paint.x + device * f64::from(FOG_CENTER.0),
        paint.y + device * f64::from(FOG_CENTER.1),
    );
    let reach = device * f64::from(FOG_RADIUS) * ch.scale.max(1.0) * ch.stretch.max(1.0)
        + (blur * 3 + 2) as f64;
    let x0 = ((center.0 - reach).floor() as i64).max(0);
    let y0 = ((center.1 - reach).floor() as i64).max(0);
    let x1 = ((center.0 + reach).ceil() as i64).min(pm.width() as i64);
    let y1 = ((center.1 + reach).ceil() as i64).min(pm.height() as i64);
    if x1 <= x0 || y1 <= y0 {
        return;
    }
    let (w, h) = ((x1 - x0) as u32, (y1 - y0) as u32);
    let Some(mut layer) = Pixmap::new(w, h) else {
        return;
    };

    let (fx, fy) = FOG_CENTER;
    let d = device as f32;
    let base =
        Transform::from_translate((paint.x - x0 as f64) as f32, (paint.y - y0 as f64) as f32)
            .pre_scale(d, d)
            .pre_translate(fx, fy);
    let scale = ch.scale as f32;

    let fog_ts = base.pre_scale(scale, scale);
    let [r, g, b] = paint.fog;
    let stops = FOG_PROFILE
        .iter()
        .map(|&(radius, a)| {
            GradientStop::new(
                radius / FOG_RADIUS,
                Color::from_rgba(
                    f32::from(r) / 255.0,
                    f32::from(g) / 255.0,
                    f32::from(b) / 255.0,
                    a,
                )
                .unwrap_or(Color::TRANSPARENT),
            )
        })
        .collect();
    if let Some(shader) = RadialGradient::new(
        Point::from_xy(0.0, 0.0),
        Point::from_xy(0.0, 0.0),
        FOG_RADIUS,
        stops,
        SpreadMode::Pad,
        Transform::identity(),
    ) {
        let fog = Paint {
            shader,
            anti_alias: true,
            ..Default::default()
        };
        if let Some(rect) =
            Rect::from_xywh(-FOG_RADIUS, -FOG_RADIUS, FOG_RADIUS * 2.0, FOG_RADIUS * 2.0)
        {
            layer.fill_rect(rect, &fog, fog_ts, None);
        }
    }

    let axis = ch.axis.to_degrees() as f32;
    let arrow_ts = base
        .pre_rotate(axis)
        .pre_scale(ch.stretch as f32, ch.squash as f32)
        .pre_rotate(-axis)
        .pre_rotate(ch.rotation.to_degrees() as f32)
        .pre_scale(scale, scale)
        .pre_translate(-fx, -fy);
    let body = paint.fill.unwrap_or_else(|| shade(paint.fog, FILL_SHADE));
    paint_silhouette(
        &mut layer,
        mix(paint.rim, body, RIM_ALPHA),
        FILL_ROUND + RIM_WIDTH,
        arrow_ts,
    );
    paint_silhouette(&mut layer, body, FILL_ROUND, arrow_ts);

    if blur > 0 {
        crate::compact_cursor::box_blur_rgba(layer.data_mut(), w, h, blur as u32);
    }
    pm.draw_pixmap(
        x0 as i32,
        y0 as i32,
        layer.as_ref(),
        &PixmapPaint {
            opacity: alpha as f32,
            ..Default::default()
        },
        Transform::identity(),
        None,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rest() -> CodexChannels {
        CodexChannels {
            rotation: 0.0,
            scale: 1.0,
            stretch: 1.0,
            squash: 1.0,
            axis: 0.0,
            alpha: 1.0,
        }
    }

    fn frame(channels: CodexChannels, fog: [u8; 3]) -> Pixmap {
        let mut pm = Pixmap::new(252, 252).unwrap();
        paint(
            &mut pm,
            &CodexPaint {
                x: 126.0 - 2.0 * f64::from(FOG_CENTER.0),
                y: 126.0 - 2.0 * f64::from(FOG_CENTER.1),
                device_scale: 2.0,
                channels,
                fog,
                fill: None,
                rim: [255, 255, 255],
            },
        );
        pm
    }

    fn opaque_bounds(pm: &Pixmap) -> (u32, u32, u32, u32) {
        let mut b = (u32::MAX, u32::MAX, 0, 0);
        for (i, p) in pm.pixels().iter().enumerate() {
            if p.alpha() == 255 {
                let (x, y) = (i as u32 % pm.width(), i as u32 / pm.width());
                b = (b.0.min(x), b.1.min(y), b.2.max(x), b.3.max(y));
            }
        }
        b
    }

    fn input(position: (f64, f64)) -> CodexInput {
        CodexInput {
            position,
            travelling: false,
            scoot: false,
            shown: true,
            pressed: false,
            loading: false,
            reduced_motion: false,
        }
    }

    #[test]
    fn fog_follows_the_measured_profile_and_tint() {
        let fog = [138, 114, 97];
        let pm = frame(rest(), fog);
        let at = |dx: i32| pm.pixel((126 - dx) as u32, 126).unwrap().demultiply();
        for (radius_pt, expected) in [(8.5_f32, 0.649), (12.5, 0.509), (20.5, 0.215), (28.5, 0.05)]
        {
            let px = at((radius_pt * 2.0) as i32);
            let measured = f32::from(px.alpha()) / 255.0;
            assert!(
                (measured - expected).abs() < 0.03,
                "r={radius_pt}: {measured} vs {expected}"
            );
            let tolerance = if expected > 0.1 { 2 } else { 12 };
            assert!(
                (i16::from(px.red()) - 138).abs() <= tolerance
                    && (i16::from(px.blue()) - 97).abs() <= tolerance,
                "r={radius_pt}: {px:?}"
            );
        }
        assert_eq!(pm.pixel(126 - 84, 126).unwrap().alpha(), 0);
    }

    #[test]
    fn arrow_body_and_rim_are_opaque_and_derived_from_the_fog() {
        let fog = [138, 114, 97];
        let pm = frame(rest(), fog);
        let body = shade(fog, FILL_SHADE);
        let rim = mix([255, 255, 255], body, RIM_ALPHA);
        let has = |rgb: [u8; 3]| {
            pm.pixels().iter().any(|p| {
                p.alpha() == 255 && p.red() == rgb[0] && p.green() == rgb[1] && p.blue() == rgb[2]
            })
        };
        assert!(has(body), "body {body:?}");
        assert!(has(rim), "rim {rim:?}");
        let (x0, y0, x1, y1) = opaque_bounds(&pm);
        let (w, h) = (x1 - x0 + 1, y1 - y0 + 1);
        assert!(
            (24..=27).contains(&w) && (26..=29).contains(&h),
            "arrow {w}x{h} px"
        );
    }

    #[test]
    fn press_shrinks_the_cursor_and_springs_back() {
        let feel = CursorFeel::default();
        let mut s = CodexState::default();
        s.click();
        let mut min = 1.0_f64;
        for _ in 0..240 {
            s.tick(1.0 / 120.0, &input((100.0, 100.0)), &feel);
            min = min.min(s.channels().scale);
        }
        assert!(
            min < 0.9 && min > PRESS_SCALE - 0.03,
            "press dipped to {min}"
        );
        assert!((s.channels().scale - 1.0).abs() < 1.0e-2);
        assert!(!s.animating());
    }

    #[test]
    fn loading_spins_and_returns_to_rest() {
        let feel = CursorFeel::default();
        let mut s = CodexState::default();
        let mut busy = input((0.0, 0.0));
        busy.loading = true;
        for _ in 0..60 {
            s.tick(1.0 / 60.0, &busy, &feel);
        }
        let spun = s.channels().rotation;
        assert!(
            (spun - TAU / SPIN_SECS_PER_TURN).abs() < 0.05,
            "spun {spun}"
        );
        assert!(s.animating());
        for _ in 0..600 {
            s.tick(1.0 / 60.0, &input((0.0, 0.0)), &feel);
        }
        assert_eq!(s.channels().rotation, 0.0);
        assert!(!s.animating());
    }

    #[test]
    fn sideways_travel_tilts_within_the_cap_and_settles_level() {
        let feel = CursorFeel::default();
        let mut s = CodexState::default();
        let mut peak = 0.0_f64;
        for i in 0..60 {
            let mut step = input((i as f64 * 20.0, 100.0));
            step.travelling = true;
            s.tick(1.0 / 60.0, &step, &feel);
            peak = peak.max(s.channels().rotation.abs());
        }
        assert!(peak > 10f64.to_radians() && peak <= TILT_MAX_DEGREES.to_radians() + 1.0e-3);
        for _ in 0..240 {
            s.tick(1.0 / 60.0, &input((1200.0, 100.0)), &feel);
        }
        assert!(s.channels().rotation.abs() < 1.0e-2);
    }

    #[test]
    fn reveal_fades_in_and_out_with_blur() {
        let feel = CursorFeel::default();
        let mut s = CodexState::default();
        assert_eq!(s.channels().alpha, 0.0);
        s.tick(REVEAL_SECS / 2.0, &input((0.0, 0.0)), &feel);
        let half = s.channels();
        assert!(half.alpha > 0.2 && half.alpha < 0.8);
        let sharp = frame(rest(), [138, 114, 97]);
        let soft = frame(
            CodexChannels {
                alpha: half.alpha,
                ..rest()
            },
            [138, 114, 97],
        );
        assert!(soft.pixels().iter().all(|p| p.alpha() < 255));
        assert!(opaque_bounds(&sharp).0 != u32::MAX);
        s.tick(REVEAL_SECS, &input((0.0, 0.0)), &feel);
        assert_eq!(s.channels().alpha, 1.0);
        let mut hidden = input((0.0, 0.0));
        hidden.shown = false;
        s.tick(REVEAL_SECS * 2.0, &hidden, &feel);
        assert_eq!(s.channels().alpha, 0.0);
        assert!(!s.animating());
    }

    #[test]
    fn reduced_motion_snaps_every_channel() {
        let feel = CursorFeel::default();
        let mut s = CodexState::default();
        let mut step = input((0.0, 0.0));
        step.reduced_motion = true;
        step.loading = true;
        s.tick(0.0, &step, &feel);
        assert_eq!(s.channels().alpha, 1.0);
        assert_eq!(s.channels().rotation, 0.0);
        assert!(!s.animating());
    }
}
