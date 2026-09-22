//! Compact-cursor feel: per-channel spring motion along a planned arc.
//!
//! One `CompactMotion` owns a travel that the logical cursor position does
//! not wait on. `pos` is already the hotspot when the motion starts; the
//! channels here only steer what gets painted, so input latency is
//! unaffected and a retarget can chase (or interrupt) the picture at any
//! time.
//!
//! Channels:
//! - **progress** — scalar spring 0 → 1 along the planned path. Its response
//!   is distance-scaled and clamped (`spring.rs::travel_response`).
//! - **lean** — spring toward the tangent-vs-chord angle, capped by the
//!   configured maximum rotation.
//! - **scale** — uniform spring used for the press reaction and the loading
//!   breath.
//! - **stretch / squash** — springs driven by instantaneous speed; active
//!   only for travels past the configured scoot distance.
//!
//! All reference numbers and the chosen adaptations are documented in the
//! L4 notebook and in `CursorFeel::default`.

use crate::arc::{ArcConfig, ArcPath};
use crate::spring::{travel_response, SpringChannel, SpringSpec};

/// Early-ack: the motion is "committed" once the progress spring passes this
/// fraction of the path or the painted point is within this many points of
/// the target, whichever happens first (reference `CloseEnoughConfiguration`).
pub const EARLY_ACK_PROGRESS: f64 = 0.995;
pub const EARLY_ACK_DISTANCE: f64 = 3.157;
/// Reference peak speed used to translate a distance into a travel response.
pub const DEFAULT_PEAK_SPEED: f64 = 900.0;

/// What the pointer is doing right now. Paused freezes every channel and
/// holds the current frame; Loading adds a slow breathing pulse once the
/// pointer is settled.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompactActivity {
    Idle,
    Loading,
    Paused,
}

/// Runtime feel constants for the rebuilt compact cursor.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CursorFeel {
    /// Points per second used to scale distance into a travel response.
    pub peak_speed: f64,
    pub travel_response_scaler: f64,
    pub travel_response_min: f64,
    pub travel_response_max: f64,
    pub travel_damping: f64,

    /// Lean spring (`scootRotationResponse` / `scootRotationDampingFraction`).
    pub rotation_response: f64,
    pub rotation_damping: f64,
    /// Reference bound on the total turn of one travel (`scootRotationMax`);
    /// the path builder keeps every scored candidate inside it.
    pub rotation_max_degrees: f64,
    /// Cap on the *painted* lean, in degrees. The reference's 76° is a
    /// path-level bound; a fixed pointer silhouette that tilts that far reads
    /// as a spin, so the rendered lean is a deliberate, much smaller
    /// adaptation.
    pub painted_lean_max_degrees: f64,
    /// Path fraction where the lean blends back to the rest orientation
    /// (`terminalTangentBlendStart`), so the arrow lands level.
    pub terminal_tangent_blend_start: f64,

    /// Travels longer than this get the stretch/squash treatment.
    pub scoot_distance_threshold: f64,
    pub stretch_response: f64,
    pub stretch_damping: f64,
    pub stretch_x_amount: f64,
    pub squash_y_amount: f64,
    pub stretch_pivot_x: f64,
    /// Reserved reference knob (`scootStretchMin`); 0.0 = no compression.
    pub stretch_min: f64,

    /// Press reaction. The reference only proves `isPressed` drives a press
    /// animation; the numbers are an adaptation.
    pub press_scale: f64,
    pub press_response: f64,
    pub press_damping: f64,

    /// Loading breath (adapted; the reference states no timings).
    pub loading_breath_amplitude: f64,
    pub loading_breath_hz: f64,

    /// Channel at-rest windows used to expire a motion.
    pub settle_position_eps: f64,
    pub settle_velocity_eps: f64,

    pub arc: ArcConfig,
}

impl Default for CursorFeel {
    fn default() -> Self {
        Self {
            peak_speed: DEFAULT_PEAK_SPEED,
            travel_response_scaler: 0.9,
            travel_response_min: 0.12,
            travel_response_max: 2.2,
            travel_damping: 0.9,

            rotation_response: 0.09,
            rotation_damping: 0.86,
            rotation_max_degrees: 76.0,
            painted_lean_max_degrees: 22.0,
            terminal_tangent_blend_start: 0.99,

            scoot_distance_threshold: 196.0,
            stretch_response: 0.095,
            stretch_damping: 0.72,
            stretch_x_amount: 0.38,
            squash_y_amount: 0.18,
            stretch_pivot_x: 0.5,
            stretch_min: 0.0,

            press_scale: 0.86,
            press_response: 0.08,
            press_damping: 0.9,

            loading_breath_amplitude: 0.022,
            loading_breath_hz: 1.4,

            settle_position_eps: 1.0e-3,
            settle_velocity_eps: 1.0e-2,

            arc: ArcConfig::default(),
        }
    }
}

impl CursorFeel {
    /// Response used for a travel of `distance` points.
    pub fn travel_spec(&self, distance: f64) -> SpringSpec {
        SpringSpec::new(
            travel_response(
                distance,
                self.travel_response_scaler,
                self.travel_response_min,
                self.travel_response_max,
                self.peak_speed,
            ),
            self.travel_damping,
        )
    }

    pub fn rotation_spec(&self) -> SpringSpec {
        SpringSpec::new(self.rotation_response, self.rotation_damping)
    }

    pub fn stretch_spec(&self) -> SpringSpec {
        SpringSpec::new(self.stretch_response, self.stretch_damping)
    }

    pub fn press_spec(&self) -> SpringSpec {
        SpringSpec::new(self.press_response, self.press_damping)
    }
}

/// The painted channel values for one frame.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PaintChannels {
    /// Painted position in screen points (already includes the settle).
    pub x: f64,
    pub y: f64,
    /// Uniform scale (1.0 = the authored size).
    pub scale: f64,
    /// Multiplier along the arrow's local x axis (≥ 1 stretches).
    pub stretch: f64,
    /// Multiplier along the arrow's local y axis (≤ 1 squashes).
    pub squash: f64,
    /// Normalized pivot x of the stretch (0..1 of the artwork width).
    pub pivot_x: f64,
    /// Lean in radians, positive = turning left of the chord.
    pub rotation: f64,
}

/// One in-flight compact travel.
#[derive(Debug, Clone)]
pub struct CompactMotion {
    path: ArcPath,
    /// Painted start, kept for tests and retarget chaining.
    pub from: (f64, f64),
    pub target: (f64, f64),
    distance: f64,
    progress: SpringChannel,
    lean: SpringChannel,
    scale: SpringChannel,
    stretch: SpringChannel,
    squash: SpringChannel,
    elapsed: f64,
    previous: (f64, f64),
    travel_spec: SpringSpec,
}

impl CompactMotion {
    /// Plan a travel from `from` to `target`. `bounds` is the screen size in
    /// points when known, so the chosen arc stays inside the frame.
    pub fn new(
        from: (f64, f64),
        target: (f64, f64),
        feel: &CursorFeel,
        bounds: Option<(f64, f64)>,
    ) -> Self {
        let path = ArcPath::plan(from, target, &feel.arc, bounds);
        let distance = path.chord;
        Self {
            distance,
            travel_spec: feel.travel_spec(distance),
            path,
            from,
            target,
            progress: SpringChannel::new(0.0),
            lean: SpringChannel::new(0.0),
            scale: SpringChannel::new(1.0),
            stretch: SpringChannel::new(1.0),
            squash: SpringChannel::new(1.0),
            elapsed: 0.0,
            previous: from,
        }
    }

    /// Advance every channel by `dt`. Returns true once the whole motion is
    /// at rest (the caller then drops it). A Paused activity freezes all
    /// channels and holds the current frame.
    pub fn tick(
        &mut self,
        dt: f64,
        feel: &CursorFeel,
        pressed: bool,
        activity: CompactActivity,
    ) -> bool {
        if activity == CompactActivity::Paused {
            return false;
        }
        self.elapsed += dt.max(0.0);
        self.progress.step(1.0, &self.travel_spec, dt);

        let position = self.path.sample(self.progress.value.clamp(-0.05, 1.06));
        let speed = if dt > 0.0 {
            (position.0 - self.previous.0).hypot(position.1 - self.previous.1) / dt
        } else {
            0.0
        };
        self.previous = position;

        // The arrow leans into the curve, then blends back to level over the
        // last fraction of the path so it lands at its rest orientation.
        let travel_t = self.progress.value.clamp(0.0, 1.0);
        let blend_start = feel.terminal_tangent_blend_start.clamp(0.0, 1.0);
        let blend = if travel_t <= blend_start {
            1.0
        } else {
            (1.0 - (travel_t - blend_start) / (1.0 - blend_start).max(1.0e-9)).clamp(0.0, 1.0)
        };
        let lean_cap = feel
            .painted_lean_max_degrees
            .min(feel.rotation_max_degrees)
            .to_radians();
        let lean_target = (self.path.lean(travel_t) * blend).clamp(-lean_cap, lean_cap);
        self.lean.step(lean_target, &feel.rotation_spec(), dt);

        let scoot = self.distance > feel.scoot_distance_threshold && self.progress.value < 1.0;
        let speed_ratio = (speed / feel.peak_speed.max(1.0)).clamp(0.0, 1.0);
        let (stretch_target, squash_target) = if scoot {
            (
                (1.0 + feel.stretch_x_amount * speed_ratio).max(1.0 - feel.stretch_min),
                (1.0 - feel.squash_y_amount * speed_ratio).max(1.0 - feel.squash_y_amount),
            )
        } else {
            (1.0, 1.0)
        };
        self.stretch.step(stretch_target, &feel.stretch_spec(), dt);
        self.squash.step(squash_target, &feel.stretch_spec(), dt);

        let scale_target = if pressed { feel.press_scale } else { 1.0 };
        self.scale.step(scale_target, &feel.press_spec(), dt);

        if self.progress.value >= 1.0 - feel.settle_position_eps
            && self.progress.velocity.abs() <= feel.settle_velocity_eps
        {
            self.progress.snap(1.0);
        }
        self.settled(feel)
    }

    /// True when the motion has come far enough on the path, or close enough
    /// to the target, that a caller may treat it as committed (early-ack).
    pub fn committed(&self) -> bool {
        if self.progress.value >= EARLY_ACK_PROGRESS {
            return true;
        }
        let (x, y) = self.display_pos();
        (x - self.target.0).hypot(y - self.target.1) <= EARLY_ACK_DISTANCE
    }

    pub fn display_pos(&self) -> (f64, f64) {
        if self.progress.value >= 1.0 {
            return self.target;
        }
        self.path.sample(self.progress.value.clamp(0.0, 1.0))
    }

    /// Painted channels for the current frame. `activity` adds the loading
    /// breath to the uniform scale while the pointer is settled.
    pub fn channels(&self, feel: &CursorFeel, activity: CompactActivity) -> PaintChannels {
        let (x, y) = self.display_pos();
        let breath = if activity == CompactActivity::Loading {
            1.0 + feel.loading_breath_amplitude
                * (self.elapsed * feel.loading_breath_hz * std::f64::consts::TAU).sin()
        } else {
            1.0
        };
        PaintChannels {
            x,
            y,
            scale: self.scale.value * breath,
            stretch: self.stretch.value,
            squash: self.squash.value,
            pivot_x: feel.stretch_pivot_x,
            rotation: self.lean.value,
        }
    }

    pub fn path_kind(&self) -> crate::arc::ArcKind {
        self.path.kind()
    }

    pub fn progress_value(&self) -> f64 {
        self.progress.value
    }

    fn settled(&self, feel: &CursorFeel) -> bool {
        self.progress
            .settled(1.0, feel.settle_position_eps, feel.settle_velocity_eps)
            && self.lean.settled(0.0, 1.0e-3, 1.0e-2)
            && self.scale.settled(1.0, 1.0e-3, 1.0e-2)
            && self.stretch.settled(1.0, 1.0e-3, 1.0e-2)
            && self.squash.settled(1.0, 1.0e-3, 1.0e-2)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::arc::ArcKind;

    fn feel() -> CursorFeel {
        CursorFeel::default()
    }

    fn motion(from: (f64, f64), to: (f64, f64)) -> CompactMotion {
        CompactMotion::new(from, to, &feel(), Some((1280.0, 800.0)))
    }

    #[test]
    fn travel_settles_on_target_without_a_teleport() {
        let mut m = motion((100.0, 100.0), (900.0, 620.0));
        assert_eq!(m.path_kind(), ArcKind::Arc);
        let mut previous = m.display_pos();
        let mut settled = false;
        for _ in 0..1200 {
            settled = m.tick(1.0 / 120.0, &feel(), false, CompactActivity::Idle);
            let position = m.display_pos();
            assert!(
                (position.0 - previous.0).hypot(position.1 - previous.1) < 40.0,
                "a 120 Hz frame must never teleport: {previous:?} -> {position:?}"
            );
            previous = position;
            if settled {
                break;
            }
        }
        assert!(settled, "a long travel must settle and expire");
        assert_eq!(m.display_pos(), m.target);
    }

    #[test]
    fn short_travels_are_straight_and_near_hops_floor_at_the_minimum_response() {
        let tiny = motion((100.0, 100.0), (105.0, 103.0));
        assert_eq!(tiny.path_kind(), ArcKind::Straight);

        let hop = motion((100.0, 100.0), (200.0, 100.0));
        assert_eq!(hop.path_kind(), ArcKind::Arc);
        assert!(
            (hop.travel_spec.response - 0.12).abs() < 1.0e-9,
            "a 100 pt hop floors at the minimum response, got {}",
            hop.travel_spec.response
        );
    }

    #[test]
    fn early_ack_reports_committed_before_the_settle_finishes() {
        let mut m = motion((0.0, 0.0), (1200.0, 700.0));
        assert!(!m.committed(), "a fresh travel is not committed yet");
        let mut committed_frame = None;
        let mut settled_frame = None;
        for frame in 0..2000 {
            let settled = m.tick(1.0 / 120.0, &feel(), false, CompactActivity::Idle);
            if committed_frame.is_none() && m.committed() {
                committed_frame = Some(frame);
            }
            if settled {
                settled_frame = Some(frame);
                break;
            }
        }
        let committed = committed_frame.expect("early-ack must trigger");
        let settled = settled_frame.expect("travel must settle");
        assert!(
            committed < settled,
            "early-ack frame {committed} must precede settle frame {settled}"
        );
    }

    #[test]
    fn press_shrinks_and_release_restores_the_scale() {
        let mut m = motion((200.0, 200.0), (600.0, 400.0));
        for _ in 0..60 {
            m.tick(1.0 / 120.0, &feel(), true, CompactActivity::Idle);
        }
        let pressed = m.channels(&feel(), CompactActivity::Idle).scale;
        assert!(pressed < 0.92, "pressed scale must shrink, got {pressed}");
        for _ in 0..600 {
            m.tick(1.0 / 120.0, &feel(), false, CompactActivity::Idle);
        }
        let released = m.channels(&feel(), CompactActivity::Idle).scale;
        assert!(
            (released - 1.0).abs() < 1.0e-2,
            "release must restore scale, got {released}"
        );
    }

    #[test]
    fn long_fast_travels_stretch_and_squash_but_short_ones_do_not() {
        let mut long = motion((0.0, 0.0), (1200.0, 0.0));
        let mut long_stretched = false;
        for _ in 0..400 {
            long.tick(1.0 / 120.0, &feel(), false, CompactActivity::Idle);
            let channels = long.channels(&feel(), CompactActivity::Idle);
            if channels.stretch > 1.01 && channels.squash < 0.99 {
                long_stretched = true;
            }
        }
        assert!(
            long_stretched,
            "a >scoot-distance travel must stretch then relax"
        );

        let mut short = motion((0.0, 0.0), (180.0, 0.0));
        let mut short_stretched = false;
        for _ in 0..400 {
            short.tick(1.0 / 120.0, &feel(), false, CompactActivity::Idle);
            let channels = short.channels(&feel(), CompactActivity::Idle);
            if channels.stretch > 1.01 {
                short_stretched = true;
            }
        }
        assert!(!short_stretched, "a sub-threshold travel must not stretch");
    }

    #[test]
    fn lean_follows_the_arc_under_the_painted_cap_and_lands_level() {
        let mut m = motion((100.0, 100.0), (1100.0, 300.0));
        let cap = feel().painted_lean_max_degrees.to_radians();
        let mut peak = 0.0_f64;
        let mut settled = false;
        for _ in 0..1200 {
            settled = m.tick(1.0 / 120.0, &feel(), false, CompactActivity::Idle);
            let rotation = m.channels(&feel(), CompactActivity::Idle).rotation;
            assert!(
                rotation.abs() <= cap + 1.0e-9,
                "painted lean {rotation} over cap {cap}"
            );
            if rotation.abs() > peak.abs() {
                peak = rotation;
            }
        }
        assert!(
            peak.abs() > 0.02 && peak.abs() <= cap + 1.0e-9,
            "an arc must lean the arrow within the painted cap, peaked at {peak}"
        );
        assert!(settled, "the travel must settle");
        let landed = m.channels(&feel(), CompactActivity::Idle).rotation;
        assert!(
            landed.abs() < 0.01,
            "the lean must blend back to level on arrival, got {landed}"
        );
    }

    #[test]
    fn retarget_chains_from_the_currently_painted_point() {
        let mut m = motion((0.0, 0.0), (600.0, 0.0));
        for _ in 0..20 {
            m.tick(1.0 / 120.0, &feel(), false, CompactActivity::Idle);
        }
        let painted = m.display_pos();
        assert!(painted.0 > 0.0 && painted.0 < 600.0);
        let next = CompactMotion::new(painted, (600.0, 500.0), &feel(), Some((1280.0, 800.0)));
        assert_eq!(next.from, painted, "the retarget must chain from paint");
    }

    #[test]
    fn paused_freezes_every_channel_and_holds_the_frame() {
        let mut m = motion((0.0, 0.0), (1200.0, 0.0));
        for _ in 0..10 {
            m.tick(1.0 / 120.0, &feel(), false, CompactActivity::Idle);
        }
        let frozen = m.channels(&feel(), CompactActivity::Idle);
        for _ in 0..120 {
            let settled = m.tick(1.0 / 120.0, &feel(), false, CompactActivity::Paused);
            assert!(!settled);
        }
        let held = m.channels(&feel(), CompactActivity::Idle);
        assert_eq!(held, frozen, "a paused motion must hold its exact frame");
    }

    #[test]
    fn loading_breath_modulates_scale_but_idle_does_not() {
        let mut m = motion((0.0, 0.0), (0.0, 0.0));
        let idle = m.channels(&feel(), CompactActivity::Idle);
        let loading = m.channels(&feel(), CompactActivity::Loading);
        assert_eq!(idle, loading);
        m.tick(1.0 / 120.0, &feel(), false, CompactActivity::Loading);
        let loading = m.channels(&feel(), CompactActivity::Loading);
        assert!(
            (loading.scale - 1.0).abs() > 0.0 && (loading.scale - 1.0).abs() <= 0.022 + 1.0e-9,
            "loading must breathe within its amplitude, got {}",
            loading.scale
        );
    }
}
