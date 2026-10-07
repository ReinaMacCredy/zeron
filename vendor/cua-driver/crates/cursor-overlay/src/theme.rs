//! Canonical cursor-theme semantics and `cua.default` runtime parameters.
//!
//! The default artwork is authored as a checked-in dotLottie archive and
//! compiled into the bounded artifact embedded by [`crate::theme_artifact`].
//! This module owns transport-neutral state, session colors, and the shared
//! runtime float transform. It does not parse Lottie at runtime.

use serde::{Deserialize, Serialize};

pub use cua_driver_contract::{
    CursorAction, CursorDelivery as DeliveryModifier, CursorPlayback as PlaybackKind,
    CursorReducedMotion as ReducedMotion, CursorTarget as TargetModifier,
};

pub const DEFAULT_THEME_ID: &str = "cua.default";
pub const COMPACT_THEME_ID: &str = "synara.compact";
pub const CODEX_THEME_ID: &str = "zeron.codex";
pub const DEFAULT_THEME_VERSION: &str = "2.0.0";
pub const THEME_PROFILE: &str = "cua-driver-actions-v2";
pub const CANVAS_SIZE: f32 = 128.0;
pub const DISPLAY_SIZE: f32 = 42.0;
const FLOAT_DURATION_SECS: f32 = 4.0;

pub const DEFAULT_CURSOR_FILL: [u8; 4] = [94, 192, 232, 255];

const SESSION_CURSOR_FILLS: &[[u8; 4]] = &[
    [178, 132, 255, 255],
    [247, 132, 170, 255],
    [96, 218, 174, 255],
    [244, 178, 66, 255],
    [76, 204, 224, 255],
    [221, 113, 236, 255],
    [232, 82, 98, 255],
    [184, 220, 54, 255],
    [80, 126, 236, 255],
];

/// Return the stable fill color for one session-owned cursor.
///
/// The anonymous/default cursor keeps the original Cua blue. Named sessions
/// hash into the former multi-cursor palette so concurrent runs are visually
/// distinct without accepting an agent-controlled styling argument.
pub fn session_fill_rgba(session_id: &str) -> [u8; 4] {
    if session_id.is_empty() || session_id == "default" {
        return DEFAULT_CURSOR_FILL;
    }

    SESSION_CURSOR_FILLS[stable_session_index(session_id, SESSION_CURSOR_FILLS.len())]
}

pub fn session_fill_hex(session_id: &str) -> String {
    let [r, g, b, _] = session_fill_rgba(session_id);
    format!("#{r:02X}{g:02X}{b:02X}")
}

// ── Agent cursor style (stock monochrome + optional session overrides) ───────

/// Stock fill: the near-black arrow body of a normal pointer.
pub const STOCK_CURSOR_FILL: [u8; 3] = [18, 20, 26];
/// Stock fill while a button is held (a touch lighter so the press reads).
pub const STOCK_CURSOR_FILL_PRESSED: [u8; 3] = [44, 47, 56];
/// Stock rim: the light outline that keeps the arrow readable on dark art.
pub const STOCK_CURSOR_RIM: [u8; 3] = [255, 255, 255];
/// Stock shadow color; the paint applies its own alpha.
pub const STOCK_CURSOR_SHADOW: [u8; 3] = [0, 0, 0];

/// Per-session cursor color channels.
///
/// Every channel is optional: `None` means the stock monochrome default, so an
/// empty style always renders the stock pointer. The driver never derives
/// colors from the session hash any more; a session-scoped
/// `set_agent_cursor_style` call (an internal control tool, not part of the
/// model-facing tool list) is the only way to paint other colors.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct CursorStyle {
    /// Arrow body color, `#rrggbb`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fill: Option<[u8; 3]>,
    /// Outline color, `#rrggbb`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rim: Option<[u8; 3]>,
    /// Drop-shadow color, `#rrggbb`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shadow: Option<[u8; 3]>,
}

impl CursorStyle {
    /// Stock style: every channel falls back to the monochrome defaults.
    pub fn is_stock(&self) -> bool {
        self.fill.is_none() && self.rim.is_none() && self.shadow.is_none()
    }

    pub fn fill_rgb(&self) -> [u8; 3] {
        self.fill.unwrap_or(STOCK_CURSOR_FILL)
    }

    pub fn rim_rgb(&self) -> [u8; 3] {
        self.rim.unwrap_or(STOCK_CURSOR_RIM)
    }

    pub fn shadow_rgb(&self) -> [u8; 3] {
        self.shadow.unwrap_or(STOCK_CURSOR_SHADOW)
    }

    /// Arrow body color with the fill alpha the compact paint uses.
    pub fn fill_rgba(&self) -> [u8; 4] {
        let [r, g, b] = self.fill_rgb();
        [r, g, b, 255]
    }

    /// Outline color with full alpha for the theme palette replacement.
    pub fn rim_rgba(&self) -> [u8; 4] {
        let [r, g, b] = self.rim_rgb();
        [r, g, b, 255]
    }

    /// Paint-time arrow color for the current press state.
    ///
    /// Stock colors reproduce the authored artwork exactly (including the
    /// pressed shade); a custom fill lightens by the same delta.
    pub fn fill_rgb_pressed(&self) -> [u8; 3] {
        match self.fill {
            None => STOCK_CURSOR_FILL_PRESSED,
            Some(fill) => [
                fill[0].saturating_add(STOCK_CURSOR_FILL_PRESSED[0] - STOCK_CURSOR_FILL[0]),
                fill[1].saturating_add(STOCK_CURSOR_FILL_PRESSED[1] - STOCK_CURSOR_FILL[1]),
                fill[2].saturating_add(STOCK_CURSOR_FILL_PRESSED[2] - STOCK_CURSOR_FILL[2]),
            ],
        }
    }
}

/// Parse one `#rrggbb` style argument. Returns the channel name in the error
/// so a junk value is rejected honestly instead of silently stock.
pub fn parse_style_color(channel: &str, value: &str) -> Result<[u8; 3], String> {
    let Some(hex) = value.strip_prefix('#') else {
        return Err(format!(
            "cursor style `{channel}` must be \"#rrggbb\" (e.g. \"#122334\"), got {value:?}"
        ));
    };
    if hex.len() != 6 || !hex.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(format!(
            "cursor style `{channel}` must be \"#rrggbb\" (e.g. \"#122334\"), got {value:?}"
        ));
    }
    let byte = |index: usize| u8::from_str_radix(&hex[index..index + 2], 16).expect("hex digit");
    Ok([byte(0), byte(2), byte(4)])
}

fn stable_session_index(id: &str, count: usize) -> usize {
    let suffix = id
        .rfind(['-', '_', '.'])
        .map(|index| &id[index + 1..])
        .unwrap_or(id);
    if let Ok(number) = suffix.parse::<usize>() {
        if number > 0 {
            return (number - 1) % count;
        }
    }
    if suffix.len() == 1 {
        if let Some(character) = suffix.chars().next() {
            if character.is_ascii_alphabetic() {
                return (character.to_ascii_lowercase() as usize - b'a' as usize) % count;
            }
        }
    }

    let mut hash: u32 = 2_166_136_261;
    for character in id.chars() {
        hash ^= character as u32;
        hash = hash.wrapping_mul(16_777_619);
    }
    hash as usize % count
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CursorVisualState {
    pub requested_action: CursorAction,
    pub resolved_action: CursorAction,
    pub delivery: Option<DeliveryModifier>,
    pub target: Option<TargetModifier>,
    pub elapsed_secs: f64,
    /// Grace period remaining after a short tool call ends, so observe/key
    /// cues remain visible for at least one rendered frame.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ending_secs: Option<f64>,
    pub reduced_motion: ReducedMotion,
    pub preempted_count: u64,
}

impl Default for CursorVisualState {
    fn default() -> Self {
        Self {
            requested_action: CursorAction::Idle,
            resolved_action: CursorAction::Idle,
            delivery: None,
            target: None,
            elapsed_secs: 0.0,
            ending_secs: None,
            reduced_motion: ReducedMotion::Auto,
            preempted_count: 0,
        }
    }
}

impl CursorVisualState {
    pub fn begin(
        &mut self,
        action: CursorAction,
        delivery: Option<DeliveryModifier>,
        target: Option<TargetModifier>,
    ) {
        if self.resolved_action != CursorAction::Idle && self.resolved_action != action {
            self.preempted_count = self.preempted_count.saturating_add(1);
        }
        self.requested_action = action;
        self.resolved_action = action;
        self.delivery = delivery;
        self.target = target;
        self.elapsed_secs = 0.0;
        self.ending_secs = None;
    }

    pub fn end(&mut self, action: CursorAction) {
        if self.resolved_action == action
            && matches!(action.playback(), PlaybackKind::Held | PlaybackKind::Loop)
        {
            self.ending_secs = Some(0.4);
        }
    }

    pub fn to_idle(&mut self) {
        self.requested_action = CursorAction::Idle;
        self.resolved_action = CursorAction::Idle;
        self.delivery = None;
        self.target = None;
        self.elapsed_secs = 0.0;
        self.ending_secs = None;
    }

    pub fn tick(&mut self, dt: f64) {
        let dt = dt.max(0.0);
        self.elapsed_secs = (self.elapsed_secs + dt).min(86_400.0);
        if let Some(remaining) = self.ending_secs {
            if remaining <= dt {
                self.to_idle();
                return;
            }
            self.ending_secs = Some(remaining - dt);
        }
        let action = self.resolved_action;
        if action.playback() == PlaybackKind::OneShot && self.elapsed_secs >= action.duration_secs()
        {
            self.to_idle();
        }
    }

    pub fn phase(&self) -> &'static str {
        match self.resolved_action.playback() {
            PlaybackKind::Resting | PlaybackKind::Loop => "loop",
            PlaybackKind::Held => "sustain",
            PlaybackKind::OneShot => "one_shot",
        }
    }

    pub fn frame(&self) -> u32 {
        let duration = self.resolved_action.duration_secs().max(1.0 / 30.0);
        let elapsed = match self.resolved_action.playback() {
            PlaybackKind::Resting | PlaybackKind::Loop | PlaybackKind::Held => {
                self.elapsed_secs.rem_euclid(duration)
            }
            PlaybackKind::OneShot => self.elapsed_secs.min(duration),
        };
        (elapsed * 30.0).floor() as u32
    }
}

pub(crate) fn shared_float_motion(visual: &CursorVisualState) -> (f32, f32, f32) {
    if visual.reduced_motion == ReducedMotion::On {
        return (0.0, 0.0, 0.0);
    }

    let progress =
        (visual.elapsed_secs as f32).rem_euclid(FLOAT_DURATION_SECS) / FLOAT_DURATION_SECS;
    let angle = progress * std::f32::consts::TAU;
    (
        angle.sin() * 5.0,
        6.0 * angle.cos() - 5.0,
        2.5_f32.to_radians() * angle.cos(),
    )
}

/// Paint one frame of the embedded actions-v2 theme.
///
/// `anchor_x/y` is the existing overlay's cursor centre. The Lottie canvas is
/// centred there so the established click offset and path physics remain
/// unchanged. `heading` rotates the artwork around the canvas centre.
pub fn paint_default_theme(
    pm: &mut tiny_skia::Pixmap,
    visual: &CursorVisualState,
    anchor_x: f32,
    anchor_y: f32,
    heading: f32,
    backing_scale: f32,
    alpha: f32,
) {
    paint_default_theme_with_fill(
        pm,
        visual,
        anchor_x,
        anchor_y,
        heading,
        backing_scale,
        alpha,
        DEFAULT_CURSOR_FILL,
    );
}

#[allow(clippy::too_many_arguments)]
pub fn paint_default_theme_with_fill(
    pm: &mut tiny_skia::Pixmap,
    visual: &CursorVisualState,
    anchor_x: f32,
    anchor_y: f32,
    heading: f32,
    backing_scale: f32,
    alpha: f32,
    fill_rgba: [u8; 4],
) {
    let theme = crate::theme_artifact::embedded_default_theme();
    crate::theme_artifact::paint_compiled_theme_with_tint(
        pm,
        &theme,
        visual,
        anchor_x,
        anchor_y,
        heading,
        backing_scale,
        alpha,
        Some(fill_rgba),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn full_profile_has_twelve_unique_actions() {
        let mut names = std::collections::BTreeSet::new();
        for action in CursorAction::ALL {
            assert!(names.insert(action.as_str()));
        }
        assert_eq!(names.len(), 12);
    }

    #[test]
    fn default_theme_uses_compact_42_point_footprint() {
        assert_eq!(DISPLAY_SIZE, 42.0);
    }

    #[test]
    fn default_cursor_keeps_original_blue_fill() {
        assert_eq!(session_fill_rgba("default"), DEFAULT_CURSOR_FILL);
        assert_eq!(session_fill_hex("default"), "#5EC0E8");
    }

    #[test]
    fn named_session_colors_are_stable_and_distinct() {
        assert_eq!(session_fill_rgba("agent-1"), session_fill_rgba("agent-1"));
        assert_ne!(session_fill_rgba("agent-1"), session_fill_rgba("agent-2"));
        assert_ne!(session_fill_rgba("agent-2"), DEFAULT_CURSOR_FILL);
    }

    #[test]
    fn empty_style_is_stock_monochrome() {
        let style = CursorStyle::default();
        assert!(style.is_stock());
        assert_eq!(style.fill_rgb(), STOCK_CURSOR_FILL);
        assert_eq!(style.fill_rgb_pressed(), STOCK_CURSOR_FILL_PRESSED);
        assert_eq!(style.rim_rgb(), STOCK_CURSOR_RIM);
        assert_eq!(style.shadow_rgb(), STOCK_CURSOR_SHADOW);
        // The stock fill is nothing like any hash-palette session color.
        assert_ne!(style.fill_rgba()[0..3], DEFAULT_CURSOR_FILL[0..3]);
    }

    #[test]
    fn style_hex_parsing_accepts_rrggbb_and_rejects_junk() {
        assert_eq!(parse_style_color("fill", "#122334"), Ok([0x12, 0x23, 0x34]));
        assert_eq!(parse_style_color("rim", "#AABBCC"), Ok([0xAA, 0xBB, 0xCC]));
        for junk in [
            "122334",
            "#12345",
            "#1234567",
            "#12g334",
            "",
            "#",
            "rgb(1,2,3)",
            "#1122334",
            " #112233",
        ] {
            assert!(
                parse_style_color("shadow", junk).is_err(),
                "{junk:?} must be rejected"
            );
        }
    }

    #[test]
    fn custom_style_channels_override_independently() {
        let style = CursorStyle {
            fill: Some([0xE8, 0x6C, 0x00]),
            rim: None,
            shadow: None,
        };
        assert_eq!(style.fill_rgb(), [0xE8, 0x6C, 0x00]);
        assert_eq!(style.rim_rgb(), STOCK_CURSOR_RIM);
        assert_eq!(style.shadow_rgb(), STOCK_CURSOR_SHADOW);
        assert!(!style.is_stock());
        // The pressed shade keeps the same lightening delta.
        let pressed = style.fill_rgb_pressed();
        assert_eq!(pressed[0], 0xE8u8.saturating_add(26));
        assert_eq!(pressed[1], 0x6Cu8.saturating_add(27));
        assert_eq!(pressed[2], 0x00u8.saturating_add(30));
    }

    #[test]
    fn default_cursor_uses_parameterized_fill_white_ink_and_matching_glow() {
        let mut pixmap = tiny_skia::Pixmap::new(256, 256).unwrap();
        paint_default_theme_with_fill(
            &mut pixmap,
            &CursorVisualState::default(),
            128.0,
            128.0,
            std::f32::consts::FRAC_PI_4,
            2.0,
            1.0,
            [12, 34, 56, 255],
        );

        let pixels = pixmap.data().chunks_exact(4).collect::<Vec<_>>();
        assert!(pixels.iter().any(|pixel| *pixel == [12, 34, 56, 255]));
        assert!(pixels.iter().any(|pixel| *pixel == [255, 255, 255, 255]));
        assert!(pixels.iter().any(|pixel| pixel[2] > pixel[1]
            && pixel[1] > pixel[0]
            && pixel[0] > 0
            && pixel[3] > 16
            && pixel[3] < 200));
    }

    #[test]
    fn matching_glow_surrounds_the_full_pointer_silhouette() {
        let mut pixmap = tiny_skia::Pixmap::new(256, 256).unwrap();
        let visual = CursorVisualState {
            reduced_motion: ReducedMotion::On,
            ..CursorVisualState::default()
        };
        paint_default_theme_with_fill(
            &mut pixmap,
            &visual,
            128.0,
            128.0,
            std::f32::consts::FRAC_PI_4,
            2.0,
            1.0,
            [12, 34, 56, 255],
        );

        let bounds = |predicate: &dyn Fn(&[u8]) -> bool| {
            let mut result = (u32::MAX, u32::MAX, 0, 0);
            for (index, pixel) in pixmap.data().chunks_exact(4).enumerate() {
                if predicate(pixel) {
                    let x = index as u32 % pixmap.width();
                    let y = index as u32 / pixmap.width();
                    result.0 = result.0.min(x);
                    result.1 = result.1.min(y);
                    result.2 = result.2.max(x);
                    result.3 = result.3.max(y);
                }
            }
            result
        };
        let body = bounds(&|pixel| pixel == [12, 34, 56, 255]);
        let glow = bounds(&|pixel| {
            pixel[2] > pixel[1]
                && pixel[1] > pixel[0]
                && pixel[0] > 0
                && pixel[3] > 8
                && pixel[3] < 220
        });

        assert!(
            glow.0 + 4 <= body.0,
            "glow should cover the pointer's left edge: body={body:?}, glow={glow:?}"
        );
        assert!(
            glow.1 + 4 <= body.1,
            "glow should cover the pointer's top edge: body={body:?}, glow={glow:?}"
        );
        assert!(
            glow.2 >= body.2 + 4,
            "glow should cover the pointer's right edge: body={body:?}, glow={glow:?}"
        );
        assert!(
            glow.3 >= body.3 + 4,
            "glow should cover the pointer's bottom edge: body={body:?}, glow={glow:?}"
        );
    }

    #[test]
    fn action_theme_ignores_host_owned_modifiers() {
        let mut base = tiny_skia::Pixmap::new(128, 128).unwrap();
        let mut pixmap = tiny_skia::Pixmap::new(128, 128).unwrap();
        let base_visual = CursorVisualState {
            reduced_motion: ReducedMotion::On,
            ..CursorVisualState::default()
        };
        let visual = CursorVisualState {
            delivery: Some(DeliveryModifier::Foreground),
            target: Some(TargetModifier::Pixel),
            reduced_motion: ReducedMotion::On,
            ..CursorVisualState::default()
        };
        paint_default_theme_with_fill(
            &mut base,
            &base_visual,
            64.0,
            64.0,
            std::f32::consts::FRAC_PI_4,
            1.0,
            1.0,
            [12, 34, 56, 255],
        );
        paint_default_theme_with_fill(
            &mut pixmap,
            &visual,
            64.0,
            64.0,
            std::f32::consts::FRAC_PI_4,
            1.0,
            1.0,
            [12, 34, 56, 255],
        );

        assert_eq!(pixmap.data(), base.data());
    }

    #[test]
    fn every_action_inherits_the_same_floating_base_motion() {
        let idle = CursorVisualState {
            elapsed_secs: 0.75,
            ..CursorVisualState::default()
        };
        let expected = shared_float_motion(&idle);
        assert_ne!(expected, (0.0, 0.0, 0.0));

        for action in CursorAction::ALL {
            let mut visual = CursorVisualState::default();
            visual.begin(action, None, None);
            visual.elapsed_secs = 0.75;
            assert_eq!(
                shared_float_motion(&visual),
                expected,
                "{} did not inherit the shared floating motion",
                action.as_str()
            );
        }
    }

    #[test]
    fn reduced_motion_disables_shared_floating_motion() {
        let visual = CursorVisualState {
            elapsed_secs: 0.75,
            reduced_motion: ReducedMotion::On,
            ..CursorVisualState::default()
        };
        assert_eq!(shared_float_motion(&visual), (0.0, 0.0, 0.0));
    }

    #[test]
    fn one_shot_returns_to_idle() {
        let mut state = CursorVisualState::default();
        state.begin(CursorAction::Click, None, Some(TargetModifier::Pixel));
        state.tick(CursorAction::Click.duration_secs() + 0.01);
        assert_eq!(state.resolved_action, CursorAction::Idle);
        assert_eq!(state.target, None);
    }

    #[test]
    fn held_action_waits_for_matching_end() {
        let mut state = CursorVisualState::default();
        state.begin(CursorAction::Text, Some(DeliveryModifier::Foreground), None);
        state.tick(30.0);
        assert_eq!(state.resolved_action, CursorAction::Text);
        state.end(CursorAction::Click);
        assert_eq!(state.resolved_action, CursorAction::Text);
        state.end(CursorAction::Text);
        assert_eq!(state.resolved_action, CursorAction::Text);
        state.tick(0.41);
        assert_eq!(state.resolved_action, CursorAction::Idle);
    }

    #[test]
    fn every_action_renders_pixels_at_one_and_two_x() {
        for action in CursorAction::ALL {
            for scale in [1.0, 2.0] {
                let mut pm = tiny_skia::Pixmap::new(256, 256).unwrap();
                let mut visual = CursorVisualState::default();
                visual.begin(
                    action,
                    Some(DeliveryModifier::Background),
                    Some(TargetModifier::Ax),
                );
                visual.elapsed_secs = action.duration_secs() * 0.4;
                paint_default_theme(&mut pm, &visual, 128.0, 128.0, 0.0, scale, 1.0);
                assert!(
                    pm.data().chunks_exact(4).any(|pixel| pixel[3] != 0),
                    "{} did not render at {scale}x",
                    action.as_str()
                );
            }
        }
    }
}
