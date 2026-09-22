//! Deterministic bow-out travel paths for the compact agent cursor.
//!
//! Straight line hops read as a teleport; the rebuilt cursor crosses the
//! screen along a flat cubic bezier that bows out of the direct chord,
//! leans into the curve, and is guaranteed to stay inside the screen frame.
//!
//! The builder follows the reference `CursorMotionPath` shape: a configurable
//! number of candidate arcs (handle lengths + arc size + flow), each scored
//! first by staying in bounds and then by turn energy, with a straight-path
//! distance threshold below which no bow is attempted. The original control
//! points are not statically recoverable, so the candidate set and the turn
//! score here are an original, documented adaptation (see `L4-notebook.md`).

use std::f64::consts::PI;

/// Shape parameters for bezier travel paths.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ArcConfig {
    /// Control-point offset from the start, as a fraction of the chord.
    pub start_handle: f64,
    /// Control-point offset from the end, as a fraction of the chord.
    pub end_handle: f64,
    /// Perpendicular deflection magnitude as a fraction of the chord.
    pub arc_size: f64,
    /// Deflection asymmetry: positive pushes the apex toward the destination.
    pub arc_flow: f64,
    /// Number of scored candidate arcs (alternating sides, varying size).
    pub candidate_count: usize,
    /// Points closer than this to the frame edge are considered unsafe.
    pub bounds_margin: f64,
    /// Chords shorter than this stay straight (no candidate is scored).
    pub straight_path_distance_threshold: f64,
}

impl Default for ArcConfig {
    fn default() -> Self {
        // Reference `MotionConfiguration.live` values (cursor-spec.md §3)
        // for every field the compact cursor actually consumes.
        Self {
            start_handle: 0.419_602_950_315_766_33,
            end_handle: 0.15,
            arc_size: 0.276_552_318_806_427_7,
            arc_flow: 0.578_355_532_786_877_9,
            candidate_count: 20,
            bounds_margin: 20.0,
            straight_path_distance_threshold: 10.0,
        }
    }
}

/// Whether a planned path bows or stays on the direct chord.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArcKind {
    Straight,
    Arc,
}

/// A planned travel path. `sample`/`tangent`/`lean` are pure functions of the
/// path, so identical inputs always produce identical frames.
#[derive(Debug, Clone, Copy)]
pub struct ArcPath {
    kind: ArcKind,
    from: (f64, f64),
    to: (f64, f64),
    control1: (f64, f64),
    control2: (f64, f64),
    /// Straight-line start→end distance (the chord).
    pub chord: f64,
    /// Chord direction in radians.
    pub chord_angle: f64,
    /// Approximate arc length, sampled at a fixed resolution.
    pub length: f64,
    /// Approximate total tangent turn along the path, radians.
    pub total_turn: f64,
}

impl ArcPath {
    /// Plan the best-bounded/lowest-turn candidate arc, or a straight chord
    /// when the distance is below the threshold or every candidate leaves
    /// the frame. `bounds` is the screen size `(width, height)` in points;
    /// `None` means no frame is known and the preferred candidate is used.
    pub fn plan(
        from: (f64, f64),
        to: (f64, f64),
        cfg: &ArcConfig,
        bounds: Option<(f64, f64)>,
    ) -> Self {
        let dx = to.0 - from.0;
        let dy = to.1 - from.1;
        let chord = dx.hypot(dy);
        let chord_angle = dy.atan2(dx);
        if chord <= cfg.straight_path_distance_threshold
            || cfg.candidate_count < 2
            || cfg.arc_size <= 0.0
        {
            return Self::straight(from, to, chord, chord_angle);
        }

        let candidates = cfg.candidate_count;
        let mut best: Option<(f64, f64, ArcPath)> = None; // (oob, score, path)
        for index in 0..candidates {
            let side = if index % 2 == 0 { 1.0 } else { -1.0 };
            let scale = candidate_scale(index, candidates);
            let path = Self::candidate(from, to, cfg, chord, chord_angle, side, scale);
            let (oob, turn_score) = match bounds {
                Some((width, height)) => {
                    match path.out_of_bounds_samples(width, height, cfg.bounds_margin) {
                        0 => (0.0, (path.total_turn - cfg.arc_size * PI / 2.0).abs()),
                        count => (count as f64, f64::INFINITY),
                    }
                }
                // Without a known frame the preferred candidate always wins;
                // turn energy only breaks ties between identical scores.
                None => (0.0, index as f64 * 1.0e-9),
            };
            if turn_score.is_finite()
                && best.as_ref().is_none_or(|(oob_best, score, _)| {
                    oob < *oob_best || (oob == *oob_best && turn_score < *score)
                })
            {
                best = Some((oob, turn_score, path));
            }
        }
        match best {
            Some((oob, _, path)) if oob == 0.0 => path,
            // Every candidate bowed off-frame: the direct chord between two
            // on-screen endpoints can never leave a convex frame.
            _ => Self::straight(from, to, chord, chord_angle),
        }
    }

    fn straight(from: (f64, f64), to: (f64, f64), chord: f64, chord_angle: f64) -> Self {
        Self {
            kind: ArcKind::Straight,
            from,
            to,
            control1: from,
            control2: to,
            chord,
            chord_angle,
            length: chord,
            total_turn: 0.0,
        }
    }

    fn candidate(
        from: (f64, f64),
        to: (f64, f64),
        cfg: &ArcConfig,
        chord: f64,
        chord_angle: f64,
        side: f64,
        scale: f64,
    ) -> Self {
        let flow = cfg.arc_flow.clamp(-1.0, 1.0);
        let amount = chord * cfg.arc_size * scale;
        // Unit normal on `side` of the chord (side = +1 bows left of travel).
        let normal_angle = chord_angle + side * PI / 2.0;
        let (nx, ny) = (normal_angle.cos(), normal_angle.sin());
        let chord_dir = (chord_angle.cos(), chord_angle.sin());
        let control1 = (
            from.0 + chord_dir.0 * chord * cfg.start_handle + nx * amount * (1.0 - flow),
            from.1 + chord_dir.1 * chord * cfg.start_handle + ny * amount * (1.0 - flow),
        );
        let control2 = (
            to.0 - chord_dir.0 * chord * cfg.end_handle + nx * amount * flow,
            to.1 - chord_dir.1 * chord * cfg.end_handle + ny * amount * flow,
        );
        let mut path = Self {
            kind: ArcKind::Arc,
            from,
            to,
            control1,
            control2,
            chord,
            chord_angle,
            length: 0.0,
            total_turn: 0.0,
        };
        path.measure();
        path
    }

    /// Sample the path at `t`; values outside `[0, 1]` linearly extend along
    /// the start/end chord so a spring overshoot does not jump.
    pub fn sample(&self, t: f64) -> (f64, f64) {
        match self.kind {
            ArcKind::Straight => (
                self.from.0 + (self.to.0 - self.from.0) * t,
                self.from.1 + (self.to.1 - self.from.1) * t,
            ),
            ArcKind::Arc => {
                if t < 0.0 {
                    let reach = self.chord * t;
                    (
                        self.from.0 + self.chord_angle.cos() * reach,
                        self.from.1 + self.chord_angle.sin() * reach,
                    )
                } else if t > 1.0 {
                    let reach = self.chord * (t - 1.0);
                    (
                        self.to.0 + self.chord_angle.cos() * reach,
                        self.to.1 + self.chord_angle.sin() * reach,
                    )
                } else {
                    let u = 1.0 - t;
                    let (u2, u3, t2, t3) = (u * u, u * u * u, t * t, t * t * t);
                    (
                        3.0 * u2 * t * self.control1.0
                            + 3.0 * u * t2 * self.control2.0
                            + t3 * self.to.0
                            + u3 * self.from.0,
                        3.0 * u2 * t * self.control1.1
                            + 3.0 * u * t2 * self.control2.1
                            + t3 * self.to.1
                            + u3 * self.from.1,
                    )
                }
            }
        }
    }

    /// Tangent direction at `t`, clamped to the path domain.
    pub fn tangent_angle(&self, t: f64) -> f64 {
        match self.kind {
            ArcKind::Straight => self.chord_angle,
            ArcKind::Arc => {
                let t = t.clamp(0.0, 1.0);
                let u = 1.0 - t;
                let dx = 3.0 * u * u * (self.control1.0 - self.from.0)
                    + 6.0 * u * t * (self.control2.0 - self.control1.0)
                    + 3.0 * t * t * (self.to.0 - self.control2.0);
                let dy = 3.0 * u * u * (self.control1.1 - self.from.1)
                    + 6.0 * u * t * (self.control2.1 - self.control1.1)
                    + 3.0 * t * t * (self.to.1 - self.control2.1);
                if dx.hypot(dy) < 1.0e-9 {
                    self.chord_angle
                } else {
                    dy.atan2(dx)
                }
            }
        }
    }

    /// Lean of the path relative to the chord at `t`, shortest angle in
    /// radians (positive = turning left of the chord).
    pub fn lean(&self, t: f64) -> f64 {
        shortest_angle(self.tangent_angle(t) - self.chord_angle)
    }

    pub fn kind(&self) -> ArcKind {
        self.kind
    }

    pub fn from(&self) -> (f64, f64) {
        self.from
    }

    pub fn to(&self) -> (f64, f64) {
        self.to
    }

    fn measure(&mut self) {
        const SAMPLES: usize = 32;
        let mut previous = self.sample(0.0);
        let mut length = 0.0;
        let mut previous_angle = self.tangent_angle(0.0);
        let mut turn = 0.0;
        for index in 1..=SAMPLES {
            let t = index as f64 / SAMPLES as f64;
            let point = self.sample(t);
            length += (point.0 - previous.0).hypot(point.1 - previous.1);
            let angle = self.tangent_angle(t);
            turn += shortest_angle(angle - previous_angle).abs();
            previous = point;
            previous_angle = angle;
        }
        self.length = length;
        self.total_turn = turn;
    }

    /// Count of sampled points (controls + curve) that sit outside the frame
    /// minus its margin.
    fn out_of_bounds_samples(&self, width: f64, height: f64, margin: f64) -> usize {
        let lo_x = margin;
        let lo_y = margin;
        let hi_x = (width - margin).max(lo_x);
        let hi_y = (height - margin).max(lo_y);
        let mut outside = 0;
        for point in [self.control1, self.control2] {
            if point.0 < lo_x || point.0 > hi_x || point.1 < lo_y || point.1 > hi_y {
                outside += 1;
            }
        }
        for index in 0..=16 {
            let point = self.sample(index as f64 / 16.0);
            if point.0 < lo_x || point.0 > hi_x || point.1 < lo_y || point.1 > hi_y {
                outside += 1;
            }
        }
        outside
    }
}

/// Candidate size multipliers in preference order: the configured arc size
/// first, then alternating smaller/larger bows (0.4×–1.6×) as fallbacks.
fn candidate_scale(index: usize, count: usize) -> f64 {
    if index == 0 {
        return 1.0;
    }
    let pairs = (count / 2).max(1) as f64;
    let step = ((index + 1) / 2) as f64 * (0.6 / pairs);
    if index % 2 == 1 {
        (1.0 - step).max(0.4)
    } else {
        (1.0 + step).min(1.6)
    }
}

/// Shortest signed difference between two angles, in `(-π, π]`.
pub fn shortest_angle(angle: f64) -> f64 {
    let mut value = angle % (2.0 * PI);
    if value > PI {
        value -= 2.0 * PI;
    } else if value <= -PI {
        value += 2.0 * PI;
    }
    value
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config() -> ArcConfig {
        ArcConfig::default()
    }

    #[test]
    fn short_chords_stay_straight_and_long_chords_bow() {
        let short = ArcPath::plan((100.0, 100.0), (104.0, 103.0), &config(), None);
        assert_eq!(short.kind(), ArcKind::Straight);
        assert_eq!(short.total_turn, 0.0);
        assert!(short.sample(0.5).0 > 100.0 && short.sample(0.5).0 < 104.0);

        let long = ArcPath::plan((100.0, 100.0), (700.0, 400.0), &config(), None);
        assert_eq!(long.kind(), ArcKind::Arc);
        assert!(long.total_turn > 0.0, "an arc must turn along its length");
        // The bow leaves the chord: the midpoint is off the direct line.
        let mid = long.sample(0.5);
        let chord_mid = (400.0, 250.0);
        assert!(
            (mid.0 - chord_mid.0).hypot(mid.1 - chord_mid.1) > 20.0,
            "midpoint {mid:?} must bow away from the chord midpoint {chord_mid:?}"
        );
    }

    #[test]
    fn endpoints_are_exact_and_the_path_is_bounded_to_the_frame() {
        let cfg = config();
        let frame = (1280.0, 800.0);
        for (from, to) in [
            ((30.0, 30.0), (1250.0, 770.0)),
            ((1250.0, 30.0), (30.0, 770.0)),
            ((20.0, 400.0), (1260.0, 405.0)),
            ((640.0, 10.0), (645.0, 790.0)),
        ] {
            let path = ArcPath::plan(from, to, &cfg, Some(frame));
            assert_eq!(path.sample(0.0), from);
            assert_eq!(path.sample(1.0), to);
            for index in 0..=32 {
                let point = path.sample(index as f64 / 32.0);
                assert!(
                    point.0 >= 1.0 && point.0 <= frame.0 - 1.0,
                    "x out of frame at {index}: {point:?}"
                );
                assert!(
                    point.1 >= 1.0 && point.1 <= frame.1 - 1.0,
                    "y out of frame at {index}: {point:?}"
                );
            }
        }
    }

    #[test]
    fn out_of_frame_candidates_fall_back_to_the_straight_chord() {
        let cfg = ArcConfig {
            // A bow so large that no candidate can fit in a tiny frame.
            arc_size: 0.9,
            ..config()
        };
        let path = ArcPath::plan((8.0, 8.0), (40.0, 40.0), &cfg, Some((48.0, 48.0)));
        assert_eq!(path.kind(), ArcKind::Straight);
        let mid = path.sample(0.5);
        assert!((mid.0 - 24.0).abs() < 1.0e-9 && (mid.1 - 24.0).abs() < 1.0e-9);
    }

    #[test]
    fn planning_is_deterministic_for_identical_inputs() {
        let cfg = config();
        let a = ArcPath::plan((10.0, 20.0), (900.0, 640.0), &cfg, Some((1280.0, 800.0)));
        let b = ArcPath::plan((10.0, 20.0), (900.0, 640.0), &cfg, Some((1280.0, 800.0)));
        for index in 0..=64 {
            let t = index as f64 / 64.0;
            assert_eq!(a.sample(t), b.sample(t), "sample {t} must be identical");
            assert_eq!(a.tangent_angle(t), b.tangent_angle(t));
        }
        assert_eq!(a.length, b.length);
        assert_eq!(a.total_turn, b.total_turn);
    }

    #[test]
    fn prefer_bows_to_the_left_of_travel_on_both_directions() {
        // The preferred candidate always bows to its own left, so a mirrored
        // chord bows to the opposite side of the screen.
        let cfg = config();
        let forward = ArcPath::plan((0.0, 0.0), (600.0, 0.0), &cfg, None);
        let backward = ArcPath::plan((600.0, 0.0), (0.0, 0.0), &cfg, None);
        let f = forward.sample(0.5);
        let b = backward.sample(0.5);
        assert!(f.1 > 1.0, "forward travel must bow below the chord: {f:?}");
        assert!(
            b.1 < -1.0,
            "backward travel must bow above the chord: {b:?}"
        );
        assert_eq!(forward.sample(0.0), (0.0, 0.0));
        assert_eq!(backward.sample(1.0), (0.0, 0.0));
    }

    #[test]
    fn lean_is_zero_on_a_straight_path_and_bounded_on_an_arc() {
        let straight = ArcPath::plan((0.0, 0.0), (5.0, 0.0), &config(), None);
        assert_eq!(straight.lean(0.5), 0.0);
        let arc = ArcPath::plan((0.0, 0.0), (800.0, 200.0), &config(), None);
        for index in 0..=16 {
            let lean = arc.lean(index as f64 / 16.0);
            assert!(lean.abs() <= PI / 2.0, "lean must stay a lean: {lean}");
        }
        assert!(arc.lean(0.5).abs() > 0.01, "an arc midpoint must lean");
    }

    #[test]
    fn overshoot_extends_along_the_end_chord_instead_of_jumping() {
        let path = ArcPath::plan((0.0, 0.0), (500.0, 0.0), &config(), None);
        let end = path.sample(1.0);
        let past = path.sample(1.05);
        assert!(
            (past.0 - (end.0 + 0.05 * path.chord)).abs() < 1.0e-6,
            "overshoot must extend ~5% of the chord along the end direction, got {past:?}"
        );
        let before = path.sample(-0.03);
        assert!(before.0 < 0.0, "undershoot must extend behind the start");
    }
}
