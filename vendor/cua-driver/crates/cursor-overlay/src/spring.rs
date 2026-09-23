//! Spring integrator for the compact agent cursor.
//!
//! Every animated channel of the rebuilt cursor (travel progress, lean,
//! uniform scale, stretch, squash) is a scalar spring. A spring is stated the
//! way Apple's animation API states it: a **response** (the period of one
//! oscillation if there were no damping, in seconds) and a **damping
//! fraction** (1.0 = critically damped).
//!
//! The integrator is a semi-implicit Euler step with substeps derived from
//! the response, so a large frame gap (a wake from an idle block, a paused
//! process) can never blow the channel up or teleport it. Every step is pure
//! arithmetic — no clocks, no allocation, deterministic for a given
//! `(value, velocity, target, spec, dt)` tuple.

use std::f64::consts::TAU;

/// A response/damping pair. Response is clamped by the caller into the
/// configured travel window; the integrator itself only guards against a
/// zero/negative response.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SpringSpec {
    /// Seconds per oscillation at zero damping.
    pub response: f64,
    /// 1.0 = critical damping, < 1.0 underdamped (overshoots).
    pub damping_fraction: f64,
}

impl SpringSpec {
    pub const fn new(response: f64, damping_fraction: f64) -> Self {
        Self {
            response,
            damping_fraction,
        }
    }

    /// Angular frequency (rad/s) for unit mass.
    pub fn omega(&self) -> f64 {
        TAU / self.response.max(0.001)
    }

    /// Damping coefficient (1/s) for unit mass.
    pub fn damping(&self) -> f64 {
        2.0 * self.damping_fraction.max(0.0) * self.omega()
    }
}

impl Default for SpringSpec {
    fn default() -> Self {
        Self::new(0.3, 0.9)
    }
}

/// One scalar spring channel: current value and velocity.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct SpringChannel {
    pub value: f64,
    pub velocity: f64,
}

impl SpringChannel {
    pub fn new(value: f64) -> Self {
        Self {
            value,
            velocity: 0.0,
        }
    }

    /// Advance toward `target` by `dt` seconds.
    ///
    /// Substepping keeps the explicit integrator stable for every configured
    /// response (the shortest channel response is 0.055 s) and for frame gaps
    /// up to ~0.25 s. Non-finite inputs can only come from a corrupted state;
    /// they snap to the target instead of poisoning the whole render.
    pub fn step(&mut self, target: f64, spec: &SpringSpec, dt: f64) {
        if !dt.is_finite() || dt <= 0.0 {
            return;
        }
        if !self.value.is_finite() || !self.velocity.is_finite() || !target.is_finite() {
            self.snap(target);
            return;
        }
        let h = spec.response.max(0.001) / 8.0;
        let substeps = ((dt / h).ceil() as usize).clamp(1, 64);
        let sdt = dt / substeps as f64;
        let omega = spec.omega();
        let damping = spec.damping();
        for _ in 0..substeps {
            self.velocity +=
                (-omega * omega * (self.value - target) - damping * self.velocity) * sdt;
            self.value += self.velocity * sdt;
        }
    }

    /// True when the channel is close enough to `target` to be considered
    /// at rest (used to expire a motion and stop repainting).
    pub fn settled(&self, target: f64, position_eps: f64, velocity_eps: f64) -> bool {
        (self.value - target).abs() <= position_eps && self.velocity.abs() <= velocity_eps
    }

    /// Jump to `target` and stop (first placement, reduced motion, expiry).
    pub fn snap(&mut self, target: f64) {
        self.value = target;
        self.velocity = 0.0;
    }
}

/// Distance-scaled travel response, adapted from the reference
/// `springResponseScaler/min/max`: a farther target gets a longer, more
/// composed travel, with both a floor and a ceiling so micro-hops stay brisk
/// and screen-wide jumps never crawl.
pub fn travel_response(distance: f64, scaler: f64, min: f64, max: f64, peak_speed: f64) -> f64 {
    let seconds = distance.abs() / peak_speed.max(1.0) * scaler.max(0.0);
    seconds.clamp(min.min(max), max.max(min))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run_channel(
        start: f64,
        target: f64,
        spec: SpringSpec,
        steps: usize,
        dt: f64,
    ) -> SpringChannel {
        let mut channel = SpringChannel::new(start);
        for _ in 0..steps {
            channel.step(target, &spec, dt);
        }
        channel
    }

    #[test]
    fn channel_converges_to_its_target_at_sixty_fps() {
        let spec = SpringSpec::new(0.24, 0.84);
        let channel = run_channel(0.0, 1.0, spec, 240, 1.0 / 60.0);
        assert!(
            (channel.value - 1.0).abs() < 1.0e-4,
            "channel must settle on target, got {}",
            channel.value
        );
        assert!(channel.velocity.abs() < 1.0e-3, "velocity must decay");
        assert!(
            channel.settled(1.0, 1.0e-3, 1.0e-2),
            "settle predicate must agree with the integrated state"
        );
    }

    #[test]
    fn underdamped_channel_overshoots_and_critically_damped_does_not() {
        let mut soft = SpringChannel::new(0.0);
        let spec = SpringSpec::new(0.3, 0.35);
        let mut peak: f64 = 0.0;
        for _ in 0..120 {
            soft.step(1.0, &spec, 1.0 / 60.0);
            peak = peak.max(soft.value);
        }
        assert!(peak > 1.0, "damping 0.35 must overshoot, peaked at {peak}");
        let critical = run_channel(0.0, 1.0, SpringSpec::new(0.3, 1.0), 200, 1.0 / 60.0);
        assert!(
            critical.value <= 1.0 + 1.0e-6,
            "critical damping must not overshoot, got {}",
            critical.value
        );
    }

    #[test]
    fn a_huge_frame_gap_stays_finite_and_near_target() {
        // A cursor that wakes from a blocked channel can see a long dt. The
        // integrator must never explode or teleport past the target.
        let spec = SpringSpec::new(0.055, 0.9);
        let mut channel = SpringChannel::new(0.0);
        channel.velocity = 40.0;
        channel.step(1.0, &spec, 0.25);
        assert!(channel.value.is_finite() && channel.velocity.is_finite());
        assert!(
            channel.value > 0.9 && channel.value < 1.1,
            "a long gap must converge, got {}",
            channel.value
        );
    }

    #[test]
    fn non_finite_state_snaps_instead_of_poisoning_the_channel() {
        let mut channel = SpringChannel::new(f64::NAN);
        channel.step(0.5, &SpringSpec::default(), 1.0 / 60.0);
        assert_eq!(channel.value, 0.5);
        assert_eq!(channel.velocity, 0.0);
    }

    #[test]
    fn travel_response_is_distance_scaled_and_clamped() {
        let spec = (0.9, 0.12, 2.2, 900.0);
        let near = travel_response(10.0, spec.0, spec.1, spec.2, spec.3);
        let mid = travel_response(500.0, spec.0, spec.1, spec.2, spec.3);
        let far = travel_response(10_000.0, spec.0, spec.1, spec.2, spec.3);
        assert_eq!(near, 0.12, "a micro-hop floors at the minimum response");
        assert!(
            (mid - 0.5).abs() < 1.0e-9,
            "500 px at 900 px/s scaled by 0.9 must be 0.5 s, got {mid}"
        );
        assert_eq!(far, 2.2, "a screen-wide jump caps at the maximum response");
    }

    #[test]
    fn snap_clears_velocity_and_value() {
        let mut channel = SpringChannel::new(0.0);
        channel.velocity = 12.0;
        channel.snap(3.0);
        assert_eq!(channel.value, 3.0);
        assert_eq!(channel.velocity, 0.0);
    }
}
