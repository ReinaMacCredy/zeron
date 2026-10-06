use std::f64::consts::{FRAC_PI_2, PI};
use std::ops::{Add, Mul, Range, Sub};
use std::sync::atomic::{AtomicU64, Ordering};

use zeron_proto::device_hub::{DeviceOrientation, DevicePlatform, TouchPhase};

use super::geometry;

pub const SCREEN_HEIGHT: f64 = 2.2;
pub const SCREEN_Z: f64 = 0.043;
pub const FOV_Y: f64 = 32.0 * PI / 180.0;
pub const YAW_LIMIT: f64 = PI / 3.0;
pub const WHEEL_IDLE_MS: u64 = 1200;

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Vec3 {
    pub x: f64,
    pub y: f64,
    pub z: f64,
}

impl Vec3 {
    pub const ZERO: Vec3 = Vec3::new(0.0, 0.0, 0.0);

    pub const fn new(x: f64, y: f64, z: f64) -> Self {
        Self { x, y, z }
    }

    pub fn dot(self, other: Vec3) -> f64 {
        self.x * other.x + self.y * other.y + self.z * other.z
    }

    pub fn length(self) -> f64 {
        self.dot(self).sqrt()
    }

    pub fn normalize(self) -> Vec3 {
        let length = self.length();
        if length == 0.0 {
            self
        } else {
            self * (1.0 / length)
        }
    }

    fn clamp_length(self, max: f64) -> Vec3 {
        let length = self.length();
        if length > max {
            self * (max / length)
        } else {
            self
        }
    }

    fn lerp(self, to: Vec3, t: f64) -> Vec3 {
        self + (to - self) * t
    }

    fn min(self, o: Vec3) -> Vec3 {
        Vec3::new(self.x.min(o.x), self.y.min(o.y), self.z.min(o.z))
    }

    fn max(self, o: Vec3) -> Vec3 {
        Vec3::new(self.x.max(o.x), self.y.max(o.y), self.z.max(o.z))
    }
}

impl Add for Vec3 {
    type Output = Vec3;
    fn add(self, o: Vec3) -> Vec3 {
        Vec3::new(self.x + o.x, self.y + o.y, self.z + o.z)
    }
}

impl Sub for Vec3 {
    type Output = Vec3;
    fn sub(self, o: Vec3) -> Vec3 {
        Vec3::new(self.x - o.x, self.y - o.y, self.z - o.z)
    }
}

impl Mul<f64> for Vec3 {
    type Output = Vec3;
    fn mul(self, s: f64) -> Vec3 {
        Vec3::new(self.x * s, self.y * s, self.z * s)
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Quat {
    pub x: f64,
    pub y: f64,
    pub z: f64,
    pub w: f64,
}

impl Quat {
    pub const IDENTITY: Quat = Quat {
        x: 0.0,
        y: 0.0,
        z: 0.0,
        w: 1.0,
    };

    pub fn axis_angle(axis: Vec3, angle: f64) -> Quat {
        let axis = axis.normalize();
        let s = (angle / 2.0).sin();
        Quat {
            x: axis.x * s,
            y: axis.y * s,
            z: axis.z * s,
            w: (angle / 2.0).cos(),
        }
    }

    pub fn mul(self, b: Quat) -> Quat {
        let a = self;
        Quat {
            x: a.x * b.w + a.w * b.x + a.y * b.z - a.z * b.y,
            y: a.y * b.w + a.w * b.y + a.z * b.x - a.x * b.z,
            z: a.z * b.w + a.w * b.z + a.x * b.y - a.y * b.x,
            w: a.w * b.w - a.x * b.x - a.y * b.y - a.z * b.z,
        }
    }

    pub fn inverse(self) -> Quat {
        Quat {
            x: -self.x,
            y: -self.y,
            z: -self.z,
            w: self.w,
        }
    }

    pub fn normalize(self) -> Quat {
        let length = (self.x * self.x + self.y * self.y + self.z * self.z + self.w * self.w).sqrt();
        if length == 0.0 {
            return Quat::IDENTITY;
        }
        Quat {
            x: self.x / length,
            y: self.y / length,
            z: self.z / length,
            w: self.w / length,
        }
    }

    pub fn angle_to(self, o: Quat) -> f64 {
        let dot = self.x * o.x + self.y * o.y + self.z * o.z + self.w * o.w;
        2.0 * dot.abs().clamp(-1.0, 1.0).acos()
    }

    pub fn rotate(self, v: Vec3) -> Vec3 {
        let q = Vec3::new(self.x, self.y, self.z);
        let t = cross(q, v) * 2.0;
        v + t * self.w + cross(q, t)
    }

    fn slerp(self, to: Quat, t: f64) -> Quat {
        if t == 0.0 {
            return self;
        }
        if t == 1.0 {
            return to;
        }
        let mut b = to;
        let mut cos = self.w * b.w + self.x * b.x + self.y * b.y + self.z * b.z;
        if cos < 0.0 {
            b = Quat {
                x: -b.x,
                y: -b.y,
                z: -b.z,
                w: -b.w,
            };
            cos = -cos;
        }
        if cos >= 1.0 {
            return self;
        }
        let sin_sq = 1.0 - cos * cos;
        let (wa, wb) = if sin_sq <= f64::EPSILON {
            (1.0 - t, t)
        } else {
            let sin = sin_sq.sqrt();
            let half = sin.atan2(cos);
            (((1.0 - t) * half).sin() / sin, (t * half).sin() / sin)
        };
        Quat {
            x: self.x * wa + b.x * wb,
            y: self.y * wa + b.y * wb,
            z: self.z * wa + b.z * wb,
            w: self.w * wa + b.w * wb,
        }
        .normalize()
    }

    fn matrix(self) -> [[f32; 4]; 4] {
        let Quat { x, y, z, w } = self;
        [
            [
                1.0 - 2.0 * (y * y + z * z),
                2.0 * (x * y + w * z),
                2.0 * (x * z - w * y),
                0.0,
            ],
            [
                2.0 * (x * y - w * z),
                1.0 - 2.0 * (x * x + z * z),
                2.0 * (y * z + w * x),
                0.0,
            ],
            [
                2.0 * (x * z + w * y),
                2.0 * (y * z - w * x),
                1.0 - 2.0 * (x * x + y * y),
                0.0,
            ],
            [0.0, 0.0, 0.0, 1.0],
        ]
        .map(|column| column.map(|value| value as f32))
    }
}

fn cross(a: Vec3, b: Vec3) -> Vec3 {
    Vec3::new(
        a.y * b.z - a.z * b.y,
        a.z * b.x - a.x * b.z,
        a.x * b.y - a.y * b.x,
    )
}

pub fn rotation_vector(rotation: Quat) -> Vec3 {
    let mut q = rotation.normalize();
    if q.w < 0.0 {
        q = Quat {
            x: -q.x,
            y: -q.y,
            z: -q.z,
            w: -q.w,
        };
    }
    let v = Vec3::new(q.x, q.y, q.z);
    let length = v.length();
    if length < 1e-8 {
        Vec3::ZERO
    } else {
        v * (2.0 * length.atan2(q.w) / length)
    }
}

fn from_vector(vector: Vec3) -> Quat {
    let angle = vector.length();
    if angle < 1e-8 {
        Quat::IDENTITY
    } else {
        Quat::axis_angle(vector, angle)
    }
}

pub fn nearest_view(rotation: Quat, snaps: &[(Quat, f64)]) -> Option<Quat> {
    let mut closest = None;
    let mut distance = f64::INFINITY;
    for &(snap, yaw_limit) in snaps {
        let relative = rotation.mul(snap.inverse());
        let turn = 2.0 * relative.y.atan2(relative.w);
        let yaw = turn.sin().atan2(turn.cos()).clamp(-yaw_limit, yaw_limit);
        let candidate = Quat::axis_angle(Vec3::new(0.0, 1.0, 0.0), yaw).mul(snap);
        let next = candidate.angle_to(rotation);
        if next < distance - 1e-8 {
            distance = next;
            closest = Some(candidate);
        }
    }
    closest
}

pub fn screen_facing(rotation: Quat) -> Quat {
    nearest_view(rotation, &[(Quat::IDENTITY, YAW_LIMIT)]).unwrap_or(Quat::IDENTITY)
}

const GAIN: f64 = 0.006;
const RESPONSE: f64 = 0.5;
const DAMPING: f64 = 0.78;
const FREQUENCY: f64 = 2.0 * PI / RESPONSE;
const DECAY: f64 = FREQUENCY * DAMPING;
const STEP_SECONDS: f64 = 1.0 / 120.0;
const FLICK_THRESHOLD: f64 = 4.5;
const FLICK_LIMIT: f64 = 22.0;
const SPIN_DECAY: f64 = 1.1;
const SETTLE_FREQUENCY: f64 = 2.0;

struct Spring {
    at: f64,
    rotation: Quat,
    velocity: Vec3,
    steps: u32,
}

struct Spin {
    at: f64,
    rotation: Quat,
    velocity: Vec3,
    correction: Vec3,
}

struct Drag {
    start: Quat,
    rest: Quat,
    x: f64,
    y: f64,
    error: Vec3,
    at: f64,
}

// Ported from t3code (deviceMotion, phoneScene, phoneInteraction); motion constants per t3code from Bitrig 0.25.
pub struct Motion {
    pub rotation: Quat,
    target: Quat,
    velocity: Vec3,
    gesture_velocity: Vec3,
    spring: Option<Spring>,
    spin: Option<Spin>,
    drag: Option<Drag>,
    held: bool,
    interrupted_drag: bool,
    last_input: f64,
    choose: Box<dyn FnMut(Quat) -> Quat>,
}

impl Motion {
    pub fn new(choose: impl FnMut(Quat) -> Quat + 'static) -> Self {
        Self {
            rotation: Quat::IDENTITY,
            target: Quat::IDENTITY,
            velocity: Vec3::ZERO,
            gesture_velocity: Vec3::ZERO,
            spring: None,
            spin: None,
            drag: None,
            held: false,
            interrupted_drag: false,
            last_input: f64::NEG_INFINITY,
            choose: Box::new(choose),
        }
    }

    fn begin_spring(&mut self, next: Quat, now: f64) {
        self.target = next.normalize();
        self.spring = Some(Spring {
            at: now,
            rotation: self.rotation,
            velocity: self.velocity,
            steps: 0,
        });
    }

    fn release(&mut self, now: f64) {
        let Some(drag) = self.drag.take() else {
            return;
        };
        if now - self.last_input > 100.0 {
            self.gesture_velocity = Vec3::ZERO;
        }
        let prediction = from_vector(self.gesture_velocity * 0.085).mul(self.target);
        self.velocity =
            (self.velocity * 0.2 + self.gesture_velocity * 0.8).clamp_length(FLICK_LIMIT);
        let moved = drag.x != 0.0 || drag.y != 0.0;
        if moved && self.velocity.length() >= FLICK_THRESHOLD {
            let projected = from_vector(self.velocity * (1.0 / SPIN_DECAY)).mul(self.rotation);
            self.target = (self.choose)(projected).normalize();
            self.spin = Some(Spin {
                at: now,
                rotation: self.rotation,
                velocity: self.velocity,
                correction: rotation_vector(self.target.mul(projected.inverse())),
            });
            return;
        }
        let next = if moved {
            (self.choose)(prediction)
        } else {
            drag.rest
        };
        self.begin_spring(next, now);
    }

    pub fn advance(&mut self, now: f64, reduced: bool) -> bool {
        if self.held || !now.is_finite() {
            return false;
        }
        let target = self.target;
        if let Some(spin) = &self.spin {
            let seconds = ((now - spin.at) / 1000.0).max(0.0);
            let coast = (-SPIN_DECAY * seconds).exp();
            let settle = (-SETTLE_FREQUENCY * seconds).exp();
            let correction =
                from_vector(spin.correction * (1.0 - (1.0 + SETTLE_FREQUENCY * seconds) * settle));
            self.rotation = correction
                .mul(from_vector(spin.velocity * ((1.0 - coast) / SPIN_DECAY)))
                .mul(spin.rotation)
                .normalize();
            self.velocity = correction.rotate(spin.velocity * coast)
                + spin.correction * (SETTLE_FREQUENCY.powi(2) * seconds * settle);
            if reduced || (self.rotation.angle_to(target) < 0.01 && self.velocity.length() < 0.08) {
                self.spin = None;
                self.rotation = target;
                self.velocity = Vec3::ZERO;
            }
            return true;
        }
        if let Some(drag) = &mut self.drag {
            let seconds = ((now - drag.at) / 1000.0).clamp(0.0, 0.05);
            drag.at = now;
            let steps = (seconds * 120.0).ceil() as u32;
            let dt = if steps > 0 {
                seconds / f64::from(steps)
            } else {
                0.0
            };
            for _ in 0..steps {
                let mut error = rotation_vector(target.mul(self.rotation.inverse()));
                if error.dot(error) > 1e-10 {
                    let axis = error.normalize();
                    let turns = ((drag.error.dot(axis) - error.length()) / (2.0 * PI)).round();
                    error = error + axis * (turns * 2.0 * PI);
                }
                drag.error = error;
                let omega = 2.0 * PI / 0.68;
                let acceleration = error.clamp_length(1.2) * (omega * omega)
                    + self.velocity * (-2.0 * 1.12 * omega);
                self.velocity = (self.velocity + acceleration * dt).clamp_length(9.0);
                self.rotation = from_vector(self.velocity * dt)
                    .mul(self.rotation)
                    .normalize();
            }
            if reduced {
                self.rotation = target;
                self.velocity = Vec3::ZERO;
            }
            return steps > 0 || reduced;
        }
        let Some(spring) = &mut self.spring else {
            return false;
        };
        let seconds = ((now - spring.at) / 1000.0).max(0.0);
        let steps = ((seconds / STEP_SECONDS).floor() as u32).min(240);
        let integrate = |q: &mut Quat, speed: &mut Vec3, dt: f64| {
            let error = rotation_vector(target.mul(q.inverse()));
            let acceleration = error * (FREQUENCY * FREQUENCY) + *speed * (-2.0 * DECAY);
            *speed = (*speed + acceleration * dt).clamp_length(9.0);
            *q = from_vector(*speed * dt).mul(*q).normalize();
        };
        while spring.steps < steps {
            integrate(&mut spring.rotation, &mut spring.velocity, STEP_SECONDS);
            spring.steps += 1;
        }
        let (mut next_rotation, mut next_velocity) = (spring.rotation, spring.velocity);
        integrate(&mut next_rotation, &mut next_velocity, STEP_SECONDS);
        let fraction = ((seconds - f64::from(steps) * STEP_SECONDS) / STEP_SECONDS).min(1.0);
        self.rotation = spring.rotation.slerp(next_rotation, fraction);
        self.velocity = spring.velocity.lerp(next_velocity, fraction);
        if reduced
            || steps == 240
            || (self.rotation.angle_to(target) < 0.0005 && self.velocity.length() < 0.005)
        {
            self.rotation = target;
            self.velocity = Vec3::ZERO;
            self.spring = None;
        }
        true
    }

    fn begin_drag(&mut self, now: f64) {
        if self.held || self.drag.is_some() {
            return;
        }
        self.advance(now, false);
        self.drag = Some(Drag {
            start: self.rotation,
            rest: self.target,
            x: 0.0,
            y: 0.0,
            error: Vec3::ZERO,
            at: now,
        });
        self.target = self.rotation;
        self.gesture_velocity = Vec3::ZERO;
        self.last_input = f64::NEG_INFINITY;
        self.spring = None;
        self.spin = None;
    }

    pub fn set_pose(&mut self, next: Quat, now: f64, immediate: bool) {
        self.advance(now, false);
        self.drag = None;
        self.spin = None;
        self.begin_spring(next, now);
        if immediate {
            self.rotation = self.target;
            self.velocity = Vec3::ZERO;
            self.spring = None;
        }
    }

    pub fn drag_active(&mut self, active: bool, now: f64) {
        if active {
            if self.drag.is_some() || self.spin.is_some() {
                self.advance(now, false);
                self.drag = None;
                self.spin = None;
            }
            self.begin_drag(now);
        } else {
            self.advance(now, false);
            self.release(now);
        }
    }

    pub fn orbit(&mut self, x: f64, y: f64, now: f64) {
        if self.held || ![x, y, now].iter().all(|v| v.is_finite()) || (x == 0.0 && y == 0.0) {
            return;
        }
        self.advance(now, false);
        self.begin_drag(now);
        let Some(drag) = &mut self.drag else {
            return;
        };
        let previous = self.target;
        drag.x += x;
        drag.y += y;
        self.target = from_vector(Vec3::new(drag.y * GAIN, drag.x * GAIN, 0.0))
            .mul(drag.start)
            .normalize();
        let seconds = (now - self.last_input) / 1000.0;
        let velocity = rotation_vector(self.target.mul(previous.inverse()));
        self.gesture_velocity = if seconds > 0.0 && seconds < 0.1 {
            (velocity * (1.0 / seconds)).clamp_length(FLICK_LIMIT)
        } else {
            Vec3::ZERO
        };
        self.last_input = now;
    }

    pub fn hold(&mut self, active: bool, now: f64) {
        if self.held == active {
            return;
        }
        if active {
            self.interrupted_drag = self.drag.is_some() || self.spin.is_some();
            self.drag = None;
            self.spin = None;
            self.spring = None;
            self.velocity = Vec3::ZERO;
        }
        self.held = active;
        if !active {
            if self.interrupted_drag {
                let next = (self.choose)(self.rotation);
                self.begin_spring(next, now);
            } else if self.rotation.angle_to(self.target) > 0.0005 {
                self.begin_spring(self.target, now);
            }
            self.interrupted_drag = false;
        }
    }

    pub fn needs_frame(&self) -> bool {
        !self.held
            && (self.spring.is_some()
                || self.spin.is_some()
                || (self.drag.is_some()
                    && (self.rotation.angle_to(self.target) > 0.0005
                        || self.velocity.length() > 0.005)))
    }

    pub fn reset(&mut self, next: Quat, now: f64) {
        self.advance(now, false);
        self.drag = None;
        self.spin = None;
        self.begin_spring(next, now);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Edge {
    Left,
    Right,
    Top,
}

#[derive(Debug, PartialEq)]
pub struct CameraPlate {
    pub width: f64,
    pub height: f64,
    pub inset_x: f64,
    pub inset_y: f64,
    pub lens_radius: f64,
    pub lenses: &'static [(f64, f64)],
    pub flash: Option<(f64, f64)>,
}

// t3code shapeProfile.ts: original family silhouettes, not reproductions of particular hardware.
#[derive(Debug, PartialEq)]
pub struct ShapeProfile {
    pub id: &'static str,
    pub bezel: f64,
    pub body_radius: f64,
    pub screen_radius: f64,
    pub depth: f64,
    pub back_color: u32,
    pub buttons: &'static [(Edge, f64, f64)],
    pub camera: CameraPlate,
}

pub const IOS_PHONE: ShapeProfile = ShapeProfile {
    id: "ios-phone",
    bezel: 0.055,
    body_radius: 0.15,
    screen_radius: 0.105,
    depth: 0.085,
    back_color: 0x424b5d,
    buttons: &[
        (Edge::Right, 0.35, 0.3),
        (Edge::Left, 0.48, 0.18),
        (Edge::Left, 0.22, 0.18),
    ],
    camera: CameraPlate {
        width: 0.39,
        height: 0.44,
        inset_x: 0.25,
        inset_y: 0.29,
        lens_radius: 0.068,
        lenses: &[(-0.08, 0.095), (0.08, -0.095)],
        flash: Some((0.085, 0.11)),
    },
};

pub const IOS_TABLET: ShapeProfile = ShapeProfile {
    id: "ios-tablet",
    bezel: 0.065,
    body_radius: 0.105,
    screen_radius: 0.045,
    depth: 0.055,
    back_color: 0x9ca5af,
    buttons: &[
        (Edge::Top, 0.5, 0.15),
        (Edge::Right, 0.78, 0.13),
        (Edge::Right, 0.58, 0.13),
    ],
    camera: CameraPlate {
        width: 0.19,
        height: 0.19,
        inset_x: 0.15,
        inset_y: 0.15,
        lens_radius: 0.045,
        lenses: &[(0.0, 0.0)],
        flash: None,
    },
};

pub const ANDROID_PHONE: ShapeProfile = ShapeProfile {
    id: "android-phone",
    bezel: 0.035,
    body_radius: 0.115,
    screen_radius: 0.08,
    depth: 0.085,
    back_color: 0x344449,
    buttons: &[(Edge::Right, 0.2, 0.24), (Edge::Right, 0.65, 0.32)],
    camera: CameraPlate {
        width: 0.24,
        height: 0.47,
        inset_x: 0.18,
        inset_y: 0.29,
        lens_radius: 0.056,
        lenses: &[(0.0, 0.11), (0.0, -0.11)],
        flash: Some((0.1, 0.0)),
    },
};

pub const ANDROID_TABLET: ShapeProfile = ShapeProfile {
    id: "android-tablet",
    back_color: 0x697b80,
    ..IOS_TABLET
};

pub fn resolve_shape(
    platform: DevicePlatform,
    name: &str,
    portrait_aspect: f64,
) -> &'static ShapeProfile {
    let words: Vec<String> = name
        .split(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
        .map(str::to_ascii_lowercase)
        .collect();
    let named = |options: [&str; 2]| words.iter().any(|word| options.contains(&word.as_str()));
    let tablet =
        named(["ipad", "tablet"]) || (!named(["iphone", "phone"]) && portrait_aspect >= 0.6);
    match (platform, tablet) {
        (DevicePlatform::Ios, true) => &IOS_TABLET,
        (DevicePlatform::Ios, false) => &IOS_PHONE,
        (_, true) => &ANDROID_TABLET,
        (_, false) => &ANDROID_PHONE,
    }
}

pub fn orientation_angle(orientation: DeviceOrientation) -> f64 {
    match orientation {
        DeviceOrientation::Portrait => 0.0,
        DeviceOrientation::LandscapeLeft => -FRAC_PI_2,
        DeviceOrientation::LandscapeRight => FRAC_PI_2,
        DeviceOrientation::PortraitUpsideDown => PI,
    }
}

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Vertex {
    pub position: [f32; 3],
    pub normal: [f32; 3],
    pub color: [f32; 3],
    pub uv: [f32; 2],
    pub screen: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Part {
    Body,
    Face,
    Back,
    Screen,
    Button,
    CameraPlate,
    CameraRing,
    CameraLens,
    Flash,
}

fn rgb(hex: u32) -> [f32; 3] {
    [16, 8, 0].map(|shift| ((hex >> shift) & 0xff) as f32 / 255.0)
}

const METAL: u32 = 0xb5bcc7;
const GLASS: u32 = 0x141820;
const LENS: u32 = 0x071326;
const FLASH: u32 = 0xf2ead6;

fn rounded(width: f64, height: f64, radius: f64, segments: usize) -> Vec<(f64, f64)> {
    let (x, y) = (-width / 2.0, -height / 2.0);
    let r = radius.min(width / 2.0).min(height / 2.0);
    let corners = [
        ((x + width - r, y), (x + width, y), (x + width, y + r)),
        (
            (x + width, y + height - r),
            (x + width, y + height),
            (x + width - r, y + height),
        ),
        ((x + r, y + height), (x, y + height), (x, y + height - r)),
        ((x, y + r), (x, y), (x + r, y)),
    ];
    let mut points = Vec::with_capacity(4 * (segments + 1));
    for (a, c, b) in corners {
        for i in 0..=segments {
            let t = i as f64 / segments as f64;
            let s = 1.0 - t;
            points.push((
                s * s * a.0 + 2.0 * s * t * c.0 + t * t * b.0,
                s * s * a.1 + 2.0 * s * t * c.1 + t * t * b.1,
            ));
        }
    }
    points
}

fn circle(radius: f64, segments: usize) -> Vec<(f64, f64)> {
    (0..segments)
        .map(|i| {
            let angle = i as f64 / segments as f64 * 2.0 * PI;
            (radius * angle.cos(), radius * angle.sin())
        })
        .collect()
}

fn outline_normals(points: &[(f64, f64)]) -> Vec<(f64, f64)> {
    let n = points.len();
    (0..n)
        .map(|i| {
            let (prev, here, next) = (points[(i + n - 1) % n], points[i], points[(i + 1) % n]);
            let a = Vec3::new(here.1 - prev.1, prev.0 - here.0, 0.0).normalize();
            let b = Vec3::new(next.1 - here.1, here.0 - next.0, 0.0).normalize();
            let sum = (a + b).normalize();
            (sum.x, sum.y)
        })
        .collect()
}

fn inside(points: &[(f64, f64)], x: f64, y: f64) -> bool {
    let n = points.len();
    (0..n).all(|i| {
        let (a, b) = (points[i], points[(i + 1) % n]);
        (b.0 - a.0) * (y - a.1) - (b.1 - a.1) * (x - a.0) >= 0.0
    })
}

#[derive(Clone, Copy)]
struct Place {
    origin: Vec3,
    flip: bool,
}

impl Place {
    const MODEL: Place = Place {
        origin: Vec3::ZERO,
        flip: false,
    };

    fn turn(self, v: Vec3) -> Vec3 {
        if self.flip {
            Vec3::new(-v.x, v.y, -v.z)
        } else {
            v
        }
    }
}

struct Builder {
    vertices: Vec<Vertex>,
    parts: Vec<(Part, Range<usize>)>,
    place: Place,
}

impl Builder {
    fn vertex(&mut self, position: Vec3, normal: Vec3, color: [f32; 3], uv: Option<(f32, f32)>) {
        let f = |v: Vec3| [v.x as f32, v.y as f32, v.z as f32];
        self.vertices.push(Vertex {
            position: f(self.place.origin + self.place.turn(position)),
            normal: f(self.place.turn(normal).normalize()),
            color,
            uv: uv.map_or([0.0; 2], |(u, v)| [u, v]),
            screen: if uv.is_some() { 1.0 } else { 0.0 },
        });
    }

    fn part(&mut self, part: Part, build: impl FnOnce(&mut Self)) {
        let start = self.vertices.len();
        build(self);
        self.parts.push((part, start..self.vertices.len()));
    }

    fn cap(
        &mut self,
        center: (f64, f64),
        outline: &[(f64, f64)],
        z: f64,
        facing: f64,
        color: [f32; 3],
        uv: Option<&dyn Fn(f64, f64) -> (f32, f32)>,
    ) {
        let normal = Vec3::new(0.0, 0.0, facing);
        let n = outline.len();
        for i in 0..n {
            for (x, y) in [center, outline[i], outline[(i + 1) % n]] {
                self.vertex(Vec3::new(x, y, z), normal, color, uv.map(|uv| uv(x, y)));
            }
        }
    }

    fn walls(
        &mut self,
        rings: &[(Vec<(f64, f64)>, f64, f64, f64)],
        normals: &[(f64, f64)],
        color: [f32; 3],
    ) {
        for pair in rings.windows(2) {
            let ((a, za, sa, na), (b, zb, sb, nb)) = (&pair[0], &pair[1]);
            let n = a.len();
            for i in 0..n {
                let j = (i + 1) % n;
                let at = |ring: &[(f64, f64)], k: usize, z: f64| Vec3::new(ring[k].0, ring[k].1, z);
                let normal = |k: usize, side: f64, nz: f64| {
                    Vec3::new(normals[k].0 * side, normals[k].1 * side, nz)
                };
                let corners = [
                    (at(a, i, *za), normal(i, *sa, *na)),
                    (at(a, j, *za), normal(j, *sa, *na)),
                    (at(b, j, *zb), normal(j, *sb, *nb)),
                    (at(b, i, *zb), normal(i, *sb, *nb)),
                ];
                for k in [0, 1, 2, 0, 2, 3] {
                    self.vertex(corners[k].0, corners[k].1, color, None);
                }
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn extrude(
        &mut self,
        (width, height, radius): (f64, f64, f64),
        z0: f64,
        depth: f64,
        size: f64,
        thickness: f64,
        bevel_segments: usize,
        color: [f32; 3],
    ) {
        let base = rounded(width, height, radius, 12);
        let normals = outline_normals(&base);
        let ring = |k: usize, front: bool| {
            let angle = k as f64 / bevel_segments as f64 * FRAC_PI_2;
            let grow = size * angle.sin();
            let lift = thickness * angle.cos();
            let (z, nz) = if front {
                (z0 + depth + lift, angle.cos())
            } else {
                (z0 - lift, -angle.cos())
            };
            (
                rounded(width + 2.0 * grow, height + 2.0 * grow, radius + grow, 12),
                z,
                angle.sin(),
                nz,
            )
        };
        let rings: Vec<_> = (0..=bevel_segments)
            .map(|k| ring(k, true))
            .chain((0..=bevel_segments).rev().map(|k| ring(k, false)))
            .collect();
        self.walls(&rings, &normals, color);
        self.cap((0.0, 0.0), &base, z0 + depth + thickness, 1.0, color, None);
        self.cap((0.0, 0.0), &base, z0 - thickness, -1.0, color, None);
    }

    fn cylinder(
        &mut self,
        (cx, cy): (f64, f64),
        radius: f64,
        height: f64,
        z: f64,
        color: [f32; 3],
    ) {
        let outline: Vec<_> = circle(radius, 32)
            .into_iter()
            .map(|(x, y)| (cx + x, cy + y))
            .collect();
        let normals: Vec<_> = circle(1.0, 32);
        let rings = [
            (outline.clone(), z + height / 2.0, 1.0, 0.0),
            (outline.clone(), z - height / 2.0, 1.0, 0.0),
        ];
        self.walls(&rings, &normals, color);
        self.cap((cx, cy), &outline, z + height / 2.0, 1.0, color, None);
        self.cap((cx, cy), &outline, z - height / 2.0, -1.0, color, None);
    }

    fn disc(&mut self, (cx, cy): (f64, f64), radius: f64, z: f64, color: [f32; 3]) {
        let outline: Vec<_> = circle(radius, 32)
            .into_iter()
            .map(|(x, y)| (cx + x, cy + y))
            .collect();
        self.cap((cx, cy), &outline, z, 1.0, color, None);
    }

    fn cuboid(&mut self, center: Vec3, size: Vec3, color: [f32; 3]) {
        let h = size * 0.5;
        for axis in 0..3 {
            for sign in [-1.0, 1.0] {
                let pick = |v: [f64; 3]| Vec3::new(v[axis], v[(axis + 1) % 3], v[(axis + 2) % 3]);
                let unpick = |v: Vec3| {
                    let mut out = [0.0; 3];
                    out[axis] = v.x;
                    out[(axis + 1) % 3] = v.y;
                    out[(axis + 2) % 3] = v.z;
                    Vec3::new(out[0], out[1], out[2])
                };
                let half = pick([h.x, h.y, h.z]);
                let normal = unpick(Vec3::new(sign, 0.0, 0.0));
                let corners = [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)].map(|(a, b)| {
                    center + unpick(Vec3::new(sign * half.x, a * half.y, b * half.z))
                });
                for k in [0, 1, 2, 0, 2, 3] {
                    self.vertex(corners[k], normal, color, None);
                }
            }
        }
    }
}

static MODEL_IDS: AtomicU64 = AtomicU64::new(1);

pub struct Model {
    pub id: u64,
    pub profile: &'static ShapeProfile,
    pub aspect: f64,
    pub orientation: DeviceOrientation,
    pub width: f64,
    pub height: f64,
    screen_width: f64,
    screen: Vec<(f64, f64)>,
    pub vertices: Vec<Vertex>,
    pub parts: Vec<(Part, Range<usize>)>,
}

impl Model {
    pub fn new(
        profile: &'static ShapeProfile,
        aspect: f64,
        orientation: DeviceOrientation,
    ) -> Self {
        let aspect = if aspect.is_finite() && aspect > 0.0 {
            aspect.min(1.0 / aspect)
        } else {
            geometry::IOS_PLACEHOLDER_ASPECT as f64
        };
        let screen_width = SCREEN_HEIGHT * aspect;
        let width = screen_width + profile.bezel * 2.0;
        let height = SCREEN_HEIGHT + profile.bezel * 2.0;
        let back_z = 0.01 - profile.depth;
        let (metal, back) = (rgb(METAL), rgb(profile.back_color));
        let screen = rounded(screen_width, SCREEN_HEIGHT, profile.screen_radius, 20);
        let mut mesh = Builder {
            vertices: Vec::new(),
            parts: Vec::new(),
            place: Place::MODEL,
        };
        mesh.part(Part::Body, |mesh| {
            let radius = profile.body_radius;
            mesh.extrude(
                (width, height, radius),
                0.025 - profile.depth,
                profile.depth,
                0.012,
                0.012,
                3,
                metal,
            )
        });
        mesh.part(Part::Face, |mesh| {
            let face = rounded(
                width - 0.014,
                height - 0.014,
                profile.body_radius - 0.01,
                16,
            );
            mesh.cap((0.0, 0.0), &face, 0.04, 1.0, rgb(GLASS), None)
        });
        mesh.part(Part::Back, |mesh| {
            let plate = rounded(
                width - 0.012,
                height - 0.012,
                profile.body_radius - 0.01,
                16,
            );
            mesh.cap((0.0, 0.0), &plate, back_z, -1.0, back, None)
        });
        let uv = |x: f64, y: f64| {
            let u = ((x + screen_width / 2.0) / screen_width) as f32;
            let v = (1.0 - (y + SCREEN_HEIGHT / 2.0) / SCREEN_HEIGHT) as f32;
            geometry::display_point(orientation, u, v)
        };
        mesh.part(Part::Screen, |mesh| {
            mesh.cap((0.0, 0.0), &screen, SCREEN_Z, 1.0, [0.0; 3], Some(&uv))
        });
        mesh.part(Part::Button, |mesh| {
            for &(edge, offset, length) in profile.buttons {
                let top = edge == Edge::Top;
                let side = if edge == Edge::Left { -1.0 } else { 1.0 };
                mesh.cuboid(
                    Vec3::new(
                        if top {
                            offset
                        } else {
                            side * (width / 2.0 + 0.015)
                        },
                        if top { height / 2.0 + 0.015 } else { offset },
                        (0.025 + back_z) / 2.0,
                    ),
                    Vec3::new(
                        if top { length } else { 0.026 },
                        if top { 0.026 } else { length },
                        profile.depth * 0.65,
                    ),
                    metal,
                );
            }
        });
        let camera = &profile.camera;
        mesh.place = Place {
            origin: Vec3::new(
                width / 2.0 - camera.inset_x,
                height / 2.0 - camera.inset_y,
                back_z + 0.001,
            ),
            flip: true,
        };
        let (plate_front, ring_depth) = (0.025 + 0.007, 0.025);
        mesh.part(Part::CameraPlate, |mesh| {
            let radius = camera.width.min(camera.height) / 4.0;
            mesh.extrude(
                (camera.width, camera.height, radius),
                0.0,
                0.025,
                0.009,
                0.007,
                2,
                back,
            )
        });
        let ring_z = plate_front + ring_depth / 2.0 - 0.003;
        mesh.part(Part::CameraRing, |mesh| {
            for &lens in camera.lenses {
                mesh.cylinder(lens, camera.lens_radius + 0.014, ring_depth, ring_z, metal);
            }
        });
        mesh.part(Part::CameraLens, |mesh| {
            for &lens in camera.lenses {
                mesh.disc(
                    lens,
                    camera.lens_radius,
                    ring_z + ring_depth / 2.0 + 0.0005,
                    rgb(LENS),
                );
            }
        });
        mesh.part(Part::Flash, |mesh| {
            if let Some(flash) = camera.flash {
                mesh.disc(flash, 0.022, plate_front + 0.0005, rgb(FLASH));
            }
        });
        Self {
            id: MODEL_IDS.fetch_add(1, Ordering::Relaxed),
            profile,
            aspect,
            orientation,
            width,
            height,
            screen_width,
            screen,
            vertices: mesh.vertices,
            parts: mesh.parts,
        }
    }

    pub fn bounds(&self, part: Option<Part>) -> (Vec3, Vec3) {
        let mut low = Vec3::new(f64::INFINITY, f64::INFINITY, f64::INFINITY);
        let mut high = low * -1.0;
        for (_, range) in self
            .parts
            .iter()
            .filter(|(p, _)| part.is_none_or(|part| *p == part))
        {
            for vertex in &self.vertices[range.clone()] {
                let [x, y, z] = vertex.position.map(f64::from);
                low = low.min(Vec3::new(x, y, z));
                high = high.max(Vec3::new(x, y, z));
            }
        }
        (low, high)
    }

    pub fn rotation(&self, pose: Quat) -> Quat {
        pose.mul(Quat::axis_angle(
            Vec3::new(0.0, 0.0, 1.0),
            orientation_angle(self.orientation),
        ))
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Camera {
    pub center: (f64, f64),
    pub distance: f64,
    pub aspect: f64,
}

impl Camera {
    pub fn fit(bounds: (Vec3, Vec3), rotation: Quat, aspect: f64) -> Camera {
        let (lo, hi) = bounds;
        let mut low = Vec3::new(f64::INFINITY, f64::INFINITY, f64::INFINITY);
        let mut high = low * -1.0;
        for i in 0..8 {
            let corner = Vec3::new(
                if i & 1 == 0 { lo.x } else { hi.x },
                if i & 2 == 0 { lo.y } else { hi.y },
                if i & 4 == 0 { lo.z } else { hi.z },
            );
            let turned = rotation.rotate(corner);
            low = low.min(turned);
            high = high.max(turned);
        }
        let tan_y = (FOV_Y / 2.0).tan();
        let tan_x = tan_y * aspect;
        let size = high - low;
        Camera {
            center: ((low.x + high.x) / 2.0, (low.y + high.y) / 2.0),
            distance: ((size.x / tan_x).max(size.y / tan_y) * 0.565 + high.z).max(1.0),
            aspect,
        }
    }

    fn eye(&self) -> Vec3 {
        Vec3::new(self.center.0, self.center.1, self.distance)
    }

    pub fn ray(&self, x: f64, y: f64) -> (Vec3, Vec3) {
        let tan = (FOV_Y / 2.0).tan();
        let (nx, ny) = (x * 2.0 - 1.0, 1.0 - y * 2.0);
        (
            self.eye(),
            Vec3::new(nx * tan * self.aspect, ny * tan, -1.0),
        )
    }

    pub fn project(&self, rotation: Quat, local: Vec3) -> (f64, f64) {
        let view = rotation.rotate(local) - self.eye();
        let tan = (FOV_Y / 2.0).tan();
        let nx = view.x / (-view.z * tan * self.aspect);
        let ny = view.y / (-view.z * tan);
        ((nx + 1.0) / 2.0, (1.0 - ny) / 2.0)
    }

    pub fn matrices(&self, rotation: Quat) -> ([[f32; 4]; 4], [[f32; 4]; 4]) {
        let f = (1.0 / (FOV_Y / 2.0).tan()) as f32;
        let (near, far) = (0.1f32, 30.0f32);
        let mut projection = [[0.0f32; 4]; 4];
        projection[0][0] = f / self.aspect as f32;
        projection[1][1] = f;
        projection[2][2] = far / (near - far);
        projection[2][3] = -1.0;
        projection[3][2] = near * far / (near - far);
        let mut view = Quat::IDENTITY.matrix();
        view[3] = [
            -self.center.0 as f32,
            -self.center.1 as f32,
            -self.distance as f32,
            1.0,
        ];
        let model = rotation.matrix();
        (multiply(&projection, &multiply(&view, &model)), model)
    }
}

fn multiply(a: &[[f32; 4]; 4], b: &[[f32; 4]; 4]) -> [[f32; 4]; 4] {
    let mut out = [[0.0; 4]; 4];
    for column in 0..4 {
        for row in 0..4 {
            out[column][row] = (0..4).map(|k| a[k][row] * b[column][k]).sum();
        }
    }
    out
}

impl Model {
    pub fn screen_point(
        &self,
        camera: &Camera,
        rotation: Quat,
        x: f64,
        y: f64,
        captured: bool,
    ) -> Option<(f32, f32)> {
        let (origin, direction) = camera.ray(x, y);
        let inverse = rotation.inverse();
        let (origin, direction) = (inverse.rotate(origin), inverse.rotate(direction));
        if direction.z == 0.0 || (!captured && direction.z > 0.0) {
            return None;
        }
        let distance = (SCREEN_Z - origin.z) / direction.z;
        if distance < 0.0 {
            return None;
        }
        let hit = origin + direction * distance;
        if !captured && !inside(&self.screen, hit.x, hit.y) {
            return None;
        }
        let u = ((hit.x + self.screen_width / 2.0) / self.screen_width).clamp(0.0, 1.0);
        let v = (1.0 - (hit.y + SCREEN_HEIGHT / 2.0) / SCREEN_HEIGHT).clamp(0.0, 1.0);
        Some(geometry::display_point(
            self.orientation,
            u as f32,
            v as f32,
        ))
    }

    pub fn screen_local(&self, display: (f32, f32)) -> Vec3 {
        let (u, v) = geometry::raw_point(self.orientation, display.0, display.1);
        Vec3::new(
            (f64::from(u) - 0.5) * self.screen_width,
            (0.5 - f64::from(v)) * SCREEN_HEIGHT,
            SCREEN_Z,
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Touch,
    Orbit,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Action {
    Touch(TouchPhase, (f32, f32)),
    Orbit(f64, f64),
    Active(bool, Mode),
}

struct Active {
    mode: Mode,
    last: (f64, f64),
    screen: Option<(f32, f32)>,
}

#[derive(Default)]
pub struct Interaction {
    active: Option<Active>,
}

impl Interaction {
    pub fn mode(&self) -> Option<Mode> {
        self.active.as_ref().map(|active| active.mode)
    }

    pub fn navigate(&self, x: f64, y: f64) -> Option<Action> {
        self.active.is_none().then_some(Action::Orbit(x, y))
    }

    pub fn end_wheel(&self) -> Option<Action> {
        self.active
            .is_none()
            .then_some(Action::Active(false, Mode::Orbit))
    }

    pub fn begin(&mut self, point: (f64, f64), screen: Option<(f32, f32)>) -> Vec<Action> {
        if self.active.is_some() {
            return Vec::new();
        }
        let mode = if screen.is_some() {
            Mode::Touch
        } else {
            Mode::Orbit
        };
        self.active = Some(Active {
            mode,
            last: point,
            screen,
        });
        let mut actions = vec![Action::Active(true, mode)];
        actions.extend(screen.map(|screen| Action::Touch(TouchPhase::Begin, screen)));
        actions
    }

    pub fn move_to(&mut self, point: (f64, f64), screen: Option<(f32, f32)>) -> Vec<Action> {
        let Some(active) = &mut self.active else {
            return Vec::new();
        };
        let action = match active.mode {
            Mode::Touch => screen.map(|screen| {
                active.screen = Some(screen);
                Action::Touch(TouchPhase::Move, screen)
            }),
            Mode::Orbit => Some(Action::Orbit(
                point.0 - active.last.0,
                point.1 - active.last.1,
            )),
        };
        active.last = point;
        action.into_iter().collect()
    }

    pub fn end(&mut self) -> Vec<Action> {
        let Some(active) = self.active.take() else {
            return Vec::new();
        };
        let mut actions = Vec::new();
        if let (Mode::Touch, Some(screen)) = (active.mode, active.screen) {
            actions.push(Action::Touch(TouchPhase::End, screen));
        }
        actions.push(Action::Active(false, active.mode));
        actions
    }
}

pub fn wheel_orbit(dx: f64, dy: f64, lines: bool, width: f64, height: f64) -> Option<(f64, f64)> {
    if ![dx, dy, width, height].iter().all(|v| v.is_finite()) || width <= 0.0 || height <= 0.0 {
        return None;
    }
    let unit = if lines { 16.0 } else { 1.0 };
    let clamp = |value: f64, extent: f64| value.clamp(-extent / 4.0, extent / 4.0);
    Some((clamp(dx * unit, width), clamp(dy * unit, height)))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WheelStep {
    Orbit,
    End,
    Ignore,
}

// gpui reports momentum scrolls as Moved after Ended; a trackpad orbit ends with the fingers.
#[derive(Default)]
pub struct Wheel {
    coasting: bool,
}

impl Wheel {
    pub fn step(&mut self, phase: gpui::TouchPhase, lines: bool) -> WheelStep {
        match phase {
            gpui::TouchPhase::Started => {
                self.coasting = false;
                WheelStep::Orbit
            }
            gpui::TouchPhase::Ended | gpui::TouchPhase::Cancelled => {
                self.coasting = true;
                WheelStep::End
            }
            gpui::TouchPhase::Moved if self.coasting && !lines => WheelStep::Ignore,
            gpui::TouchPhase::Moved => WheelStep::Orbit,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RenderKey {
    pub source: usize,
    pub model: u64,
    pub mvp: [[f32; 4]; 4],
    pub size: (usize, usize),
    pub clear: [f32; 3],
}

#[derive(Default)]
pub struct Schedule {
    last: Option<RenderKey>,
}

impl Schedule {
    pub fn due(&mut self, key: RenderKey) -> bool {
        if self.last == Some(key) {
            return false;
        }
        self.last = Some(key);
        true
    }

    pub fn forget(&mut self) {
        self.last = None;
    }
}

pub struct Viewer {
    pub motion: Motion,
    pub interaction: Interaction,
    model: Option<std::rc::Rc<Model>>,
    camera: Option<Camera>,
    viewport: Option<(f64, f64, f64, f64)>,
}

impl Default for Viewer {
    fn default() -> Self {
        let mut motion = Motion::new(screen_facing);
        motion.set_pose(Quat::IDENTITY, 0.0, true);
        Self {
            motion,
            interaction: Interaction::default(),
            model: None,
            camera: None,
            viewport: None,
        }
    }
}

impl Viewer {
    pub fn set_model(
        &mut self,
        profile: &'static ShapeProfile,
        aspect: f64,
        orientation: DeviceOrientation,
    ) -> std::rc::Rc<Model> {
        let portrait = aspect.min(1.0 / aspect);
        match &self.model {
            Some(model)
                if std::ptr::eq(model.profile, profile)
                    && model.orientation == orientation
                    && (model.aspect - portrait).abs() < 1e-9 =>
            {
                model.clone()
            }
            _ => {
                let model = std::rc::Rc::new(Model::new(profile, aspect, orientation));
                self.model = Some(model.clone());
                model
            }
        }
    }

    pub fn rotation(&self) -> Option<Quat> {
        Some(self.model.as_ref()?.rotation(self.motion.rotation))
    }

    pub fn model(&self) -> Option<std::rc::Rc<Model>> {
        self.model.clone()
    }

    pub fn viewport(&self) -> Option<(f64, f64, f64, f64)> {
        self.viewport
    }

    pub fn fit(&mut self, viewport: (f64, f64, f64, f64)) -> Option<Camera> {
        let model = self.model.as_ref()?;
        let camera = Camera::fit(
            model.bounds(None),
            model.rotation(self.motion.rotation),
            viewport.2 / viewport.3,
        );
        self.viewport = Some(viewport);
        self.camera = Some(camera);
        Some(camera)
    }

    fn normalized(&self, x: f64, y: f64) -> Option<(f64, f64)> {
        let (left, top, width, height) = self.viewport?;
        (width > 0.0 && height > 0.0).then(|| ((x - left) / width, (y - top) / height))
    }

    pub fn screen_point(&self, x: f64, y: f64, captured: bool) -> Option<(f32, f32)> {
        let (model, camera) = (self.model.as_ref()?, self.camera.as_ref()?);
        let (x, y) = self.normalized(x, y)?;
        model.screen_point(camera, model.rotation(self.motion.rotation), x, y, captured)
    }

    pub fn position_of(&self, display: (f32, f32)) -> Option<(f64, f64)> {
        let (model, camera) = (self.model.as_ref()?, self.camera.as_ref()?);
        let (left, top, width, height) = self.viewport?;
        let (x, y) = camera.project(
            model.rotation(self.motion.rotation),
            model.screen_local(display),
        );
        Some((left + x * width, top + y * height))
    }

    fn apply(&mut self, actions: Vec<Action>, now: f64) -> Vec<(TouchPhase, (f32, f32))> {
        let mut touches = Vec::new();
        for action in actions {
            match action {
                Action::Touch(phase, point) => touches.push((phase, point)),
                Action::Orbit(x, y) => self.motion.orbit(x, y, now),
                Action::Active(active, Mode::Touch) => self.motion.hold(active, now),
                Action::Active(active, Mode::Orbit) => self.motion.drag_active(active, now),
            }
        }
        touches
    }

    pub fn press(
        &mut self,
        x: f64,
        y: f64,
        orbit: bool,
        now: f64,
    ) -> Vec<(TouchPhase, (f32, f32))> {
        if self.interaction.mode().is_some() {
            return Vec::new();
        }
        let screen = if orbit {
            None
        } else {
            self.screen_point(x, y, false)
        };
        let actions = self.interaction.begin((x, y), screen);
        self.apply(actions, now)
    }

    pub fn drag(&mut self, x: f64, y: f64, now: f64) -> Vec<(TouchPhase, (f32, f32))> {
        let screen = match self.interaction.mode() {
            Some(Mode::Touch) => self.screen_point(x, y, true),
            _ => None,
        };
        let actions = self.interaction.move_to((x, y), screen);
        self.apply(actions, now)
    }

    pub fn lift(&mut self, now: f64) -> Vec<(TouchPhase, (f32, f32))> {
        let actions = self.interaction.end();
        self.apply(actions, now)
    }

    pub fn wheel(&mut self, dx: f64, dy: f64, now: f64) -> bool {
        match self.interaction.navigate(dx, dy) {
            Some(action) => {
                self.apply(vec![action], now);
                true
            }
            None => false,
        }
    }

    pub fn end_wheel(&mut self, now: f64) {
        let actions = self.interaction.end_wheel().into_iter().collect();
        self.apply(actions, now);
    }

    pub fn restore(&mut self, now: f64) {
        self.motion.reset(Quat::IDENTITY, now);
    }

    pub fn advance(&mut self, now: f64, reduced: bool) -> bool {
        self.motion.advance(now, reduced)
    }

    pub fn needs_frame(&self) -> bool {
        self.motion.needs_frame()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;
    use std::rc::Rc;
    use zeron_proto::device_hub::DeviceScreenConfig;

    const ORIENTATIONS: [DeviceOrientation; 4] = [
        DeviceOrientation::Portrait,
        DeviceOrientation::LandscapeLeft,
        DeviceOrientation::LandscapeRight,
        DeviceOrientation::PortraitUpsideDown,
    ];

    fn rotation(x: f64, y: f64, z: f64) -> Quat {
        let v = Vec3::new(x, y, z);
        Quat::axis_angle(v, v.length())
    }

    fn yxz(x: f64, y: f64) -> Quat {
        Quat::axis_angle(Vec3::new(0.0, 1.0, 0.0), y)
            .mul(Quat::axis_angle(Vec3::new(1.0, 0.0, 0.0), x))
    }

    fn snaps() -> Vec<(Quat, f64)> {
        (0..4)
            .map(|i| {
                (
                    Quat::axis_angle(Vec3::new(0.0, 0.0, 1.0), -f64::from(i) * FRAC_PI_2),
                    PI / 3.0,
                )
            })
            .collect()
    }

    fn counted(choose: impl Fn(Quat) -> Quat + 'static) -> (Motion, Rc<Cell<u32>>) {
        let calls = Rc::new(Cell::new(0));
        let seen = calls.clone();
        let motion = Motion::new(move |q| {
            seen.set(seen.get() + 1);
            choose(q)
        });
        (motion, calls)
    }

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-3
    }

    #[test]
    fn chooses_tablet_and_phone_families_without_mistaking_display_rotation_for_device_shape() {
        assert_eq!(
            resolve_shape(DevicePlatform::Ios, "iPad Pro 11-inch (M5)", 0.75).id,
            "ios-tablet"
        );
        assert_eq!(
            resolve_shape(DevicePlatform::Ios, "iPhone 18 Pro", 0.46).id,
            "ios-phone"
        );
        assert_eq!(
            resolve_shape(DevicePlatform::Android, "Pixel 9", 0.45).id,
            "android-phone"
        );
        assert_eq!(
            resolve_shape(DevicePlatform::Android, "Pixel Tablet", 0.625).id,
            "android-tablet"
        );
    }

    #[test]
    fn uses_the_screen_shape_for_renamed_devices_and_a_phone_fallback_before_metadata() {
        assert_eq!(
            resolve_shape(DevicePlatform::Ios, "Julius", 0.75).id,
            "ios-tablet"
        );
        assert_eq!(
            resolve_shape(DevicePlatform::Android, "", 0.45).id,
            "android-phone"
        );
        assert_eq!(
            resolve_shape(DevicePlatform::Ios, "", f64::NAN).id,
            "ios-phone"
        );
    }

    #[test]
    fn each_profile_builds_a_body_around_a_screen_two_point_two_units_tall() {
        for profile in [&IOS_PHONE, &IOS_TABLET, &ANDROID_PHONE, &ANDROID_TABLET] {
            for aspect in [0.46, 0.75, 1950.0 / 900.0] {
                let model = Model::new(profile, aspect, DeviceOrientation::Portrait);
                let portrait = aspect.min(1.0 / aspect);
                let (lo, hi) = model.bounds(Some(Part::Screen));
                let screen_width = SCREEN_HEIGHT * portrait;
                assert!((hi.x - lo.x - screen_width).abs() < 1e-5, "{}", profile.id);
                assert!((hi.y - lo.y - SCREEN_HEIGHT).abs() < 1e-5, "{}", profile.id);
                assert!((lo.z - SCREEN_Z).abs() < 1e-6 && (hi.z - SCREEN_Z).abs() < 1e-6);
                let (lo, hi) = model.bounds(Some(Part::Body));
                let width = screen_width + 2.0 * profile.bezel;
                assert!((model.width - width).abs() < 1e-9);
                assert!(
                    (hi.x - lo.x - (width + 0.024)).abs() < 1e-5,
                    "{}",
                    profile.id
                );
                assert!((hi.y - lo.y - (SCREEN_HEIGHT + 2.0 * profile.bezel + 0.024)).abs() < 1e-5);
                assert!(
                    (hi.z - 0.037).abs() < 1e-6 && (lo.z - (0.013 - profile.depth)).abs() < 1e-6
                );
                let (lo, _) = model.bounds(Some(Part::Back));
                assert!((lo.z - (0.01 - profile.depth)).abs() < 1e-6);
                let (blo, bhi) = model.bounds(Some(Part::Button));
                assert!(
                    bhi.x > hi.x || bhi.y > hi.y || blo.x < lo.x,
                    "{}: buttons stand proud",
                    profile.id
                );
                assert!(
                    model
                        .vertices
                        .iter()
                        .all(|v| v.position.iter().all(|c| c.is_finite()))
                );
            }
        }
    }

    #[test]
    fn attaches_the_rear_camera_plate_to_the_back_and_seats_its_lenses_on_the_housing() {
        for profile in [&IOS_PHONE, &IOS_TABLET, &ANDROID_PHONE] {
            let model = Model::new(profile, 900.0 / 1950.0, DeviceOrientation::Portrait);
            let (back, _) = model.bounds(Some(Part::Back));
            let (plate_lo, plate_hi) = model.bounds(Some(Part::CameraPlate));
            assert!(plate_lo.z < back.z, "{}", profile.id);
            assert!(plate_hi.z >= back.z, "{}", profile.id);
            let (ring_lo, ring_hi) = model.bounds(Some(Part::CameraRing));
            assert!(ring_hi.z >= plate_lo.z && ring_lo.z <= plate_hi.z);
            let (_, lens_hi) = model.bounds(Some(Part::CameraLens));
            assert!((lens_hi.z - ring_lo.z).abs() < 0.001, "{}", profile.id);
            assert!(
                plate_hi.x <= model.width / 2.0 && plate_lo.y > 0.0,
                "{}: top right of the back",
                profile.id
            );
        }
    }

    #[test]
    fn selects_the_nearest_quarter_turn_retains_a_nearby_yaw_and_treats_quaternion_signs_identically()
     {
        let snaps = snaps();
        let side = Quat::axis_angle(Vec3::new(0.0, 0.0, 1.0), -FRAC_PI_2);
        let released = rotation(0.0, 0.4, 0.0).mul(side);
        let nearest = nearest_view(released, &snaps).unwrap();
        assert!(nearest.angle_to(snaps[1].0) < PI / 3.0 + 1e-6);
        assert!(nearest.angle_to(released) < 1e-6);
        let negative = Quat {
            x: -released.x,
            y: -released.y,
            z: -released.z,
            w: -released.w,
        };
        assert!(nearest_view(negative, &snaps).unwrap().angle_to(nearest) < 1e-6);
        assert!(
            nearest_view(rotation(0.0, 3.0, 0.0), &snaps)
                .unwrap()
                .angle_to(Quat::IDENTITY)
                <= PI / 3.0 + 1e-6
        );
    }

    #[test]
    fn springs_toward_a_cumulative_drag_without_teleporting_and_keeps_the_same_target_across_event_partitions()
     {
        let run = |parts: u32| {
            let (mut motion, calls) = counted(|q| q);
            motion.drag_active(true, 0.0);
            for _ in 0..parts {
                motion.orbit(160.0 / f64::from(parts), 80.0 / f64::from(parts), 0.0);
            }
            assert!(motion.rotation.angle_to(Quat::IDENTITY) < 1e-6);
            for time in (8..=400).step_by(8) {
                motion.advance(f64::from(time), false);
            }
            assert!(motion.rotation.angle_to(Quat::IDENTITY) > 0.5);
            motion.drag_active(false, 400.0);
            for time in (408..=2400).step_by(8) {
                motion.advance(f64::from(time), false);
            }
            assert_eq!(calls.get(), 1);
            assert!(!motion.needs_frame());
            motion.rotation
        };
        assert!(run(1).angle_to(run(20)) < 1e-6);
    }

    #[test]
    fn selects_the_predicted_nearest_view_once_and_preserves_velocity_when_interrupted() {
        let snaps = snaps();
        let (mut motion, calls) = counted(move |q| nearest_view(q, &snaps).unwrap());
        motion.drag_active(true, 0.0);
        motion.orbit(0.0, 240.0, 0.0);
        motion.orbit(0.0, 20.0, 20.0);
        motion.advance(40.0, false);
        motion.drag_active(false, 40.0);
        let before = motion.rotation;
        motion.advance(80.0, false);
        assert!(motion.rotation.angle_to(before) > 0.01);
        let interrupted = motion.rotation;
        motion.drag_active(true, 80.0);
        assert!(motion.rotation.angle_to(interrupted) < 1e-6);
        motion.orbit(30.0, -30.0, 80.0);
        motion.advance(96.0, false);
        motion.drag_active(false, 100.0);
        for time in (116..=2500).step_by(16) {
            motion.advance(f64::from(time), false);
        }
        assert_eq!(calls.get(), 2);
        assert!(!motion.needs_frame());
    }

    #[test]
    fn lets_a_hard_flick_coast_through_multiple_turns_before_settling_on_a_visible_view() {
        let (mut motion, calls) = counted(|_| Quat::IDENTITY);
        motion.drag_active(true, 0.0);
        motion.orbit(0.0, 80.0, 16.0);
        motion.advance(16.0, false);
        motion.orbit(0.0, 80.0, 32.0);
        motion.advance(32.0, false);
        motion.drag_active(false, 32.0);
        let (mut previous, mut travel, mut late_speed) = (motion.rotation, 0.0, 0.0f64);
        for time in (40..=8500).step_by(8) {
            motion.advance(f64::from(time), false);
            let distance = motion.rotation.angle_to(previous);
            travel += distance;
            if time >= 2500 {
                late_speed = late_speed.max(distance / 0.008);
            }
            previous = motion.rotation;
        }
        assert!(travel > 4.0 * PI, "{travel}");
        assert!(late_speed < 2.5);
        assert_eq!(calls.get(), 1);
        assert!(motion.rotation.angle_to(Quat::IDENTITY) < 1e-6);
        assert!(!motion.needs_frame());
    }

    #[test]
    fn release_uses_elapsed_time_equally_at_different_frame_rates_and_a_captured_contact_freezes_projection()
     {
        let run = |step: usize| {
            let mut motion = Motion::new(|_| Quat::IDENTITY);
            motion.drag_active(true, 0.0);
            motion.orbit(160.0, 80.0, 0.0);
            motion.advance(50.0, false);
            motion.drag_active(false, 50.0);
            for time in (50..450).step_by(step) {
                motion.advance(time as f64, false);
            }
            motion.advance(450.0, false);
            motion
        };
        let (mut fast, slow, throttled) = (run(8), run(33), run(1000));
        assert!(fast.rotation.angle_to(slow.rotation) < 1e-6);
        assert!(fast.rotation.angle_to(throttled.rotation) < 1e-6);
        fast.hold(true, 450.0);
        let contact = fast.rotation;
        fast.orbit(100.0, 100.0, 500.0);
        fast.advance(10000.0, false);
        assert!(fast.rotation.angle_to(contact) < 1e-6);
        fast.hold(false, 10000.0);
        fast.advance(12000.0, false);
        assert!(!fast.needs_frame());
    }

    #[test]
    fn holds_a_trackpad_orbit_through_a_pause_and_snaps_only_when_the_gesture_ends() {
        let (mut motion, calls) = counted(|_| Quat::IDENTITY);
        motion.orbit(f64::NAN, 0.0, 0.0);
        assert!(!motion.needs_frame());
        motion.orbit(200.0, 100.0, 0.0);
        motion.advance(100.0, false);
        assert_eq!(calls.get(), 0);
        assert!(motion.rotation.angle_to(Quat::IDENTITY) > 0.0);
        motion.advance(140.0, true);
        let held = motion.rotation;
        assert!(held.angle_to(Quat::IDENTITY) > 0.5);
        assert!(!motion.needs_frame());
        motion.orbit(80.0, 0.0, 200.0);
        motion.advance(340.0, true);
        assert!(motion.rotation.angle_to(held) > 0.1);
        assert_eq!(calls.get(), 0);
        assert!(!motion.needs_frame());
        motion.drag_active(false, 340.0);
        motion.advance(2500.0, false);
        assert_eq!(calls.get(), 1);
        assert!(motion.rotation.angle_to(Quat::IDENTITY) < 1e-6);
        assert!(!motion.needs_frame());
        assert!((rotation_vector(rotation(0.0, PI, 0.0)).length() - PI).abs() < 1e-6);
    }

    #[test]
    fn keeps_release_angular_speed_bounded_even_across_a_back_facing_half_turn() {
        let mut motion = Motion::new(|_| rotation(0.0, PI, 0.0));
        motion.set_pose(rotation(0.0, PI, 0.0), 0.0, false);
        let mut previous = motion.rotation;
        for time in (8..=1600).step_by(8) {
            motion.advance(f64::from(time), false);
            assert!(motion.rotation.angle_to(previous) <= 9.0 * 0.008 + 1e-5);
            previous = motion.rotation;
        }
        assert!(motion.rotation.angle_to(rotation(0.0, PI, 0.0)) < 1e-6);
        assert!(!motion.needs_frame());
    }

    #[test]
    fn a_click_without_dragging_resumes_the_interrupted_resting_view_instead_of_stranding_the_device()
     {
        let (mut motion, calls) = counted(|_| Quat::IDENTITY);
        let rest = rotation(0.0, 0.8, 0.0);
        motion.set_pose(rest, 0.0, false);
        motion.advance(50.0, false);
        motion.drag_active(true, 50.0);
        motion.advance(60.0, false);
        motion.drag_active(false, 60.0);
        motion.advance(2100.0, false);
        assert!(motion.rotation.angle_to(rest) < 1e-6);
        assert_eq!(calls.get(), 0);
        assert!(!motion.needs_frame());
    }

    fn viewer(
        profile: &'static ShapeProfile,
        aspect: f64,
        orientation: DeviceOrientation,
    ) -> Viewer {
        let mut viewer = Viewer::default();
        viewer.set_model(profile, aspect, orientation);
        viewer.fit((100.0, 50.0, 600.0, 800.0));
        viewer
    }

    fn settle(viewer: &mut Viewer, from: u32) -> u32 {
        let mut time = from;
        while viewer.needs_frame() && time < 10_000 {
            time += 16;
            viewer.advance(f64::from(time), false);
            viewer.fit((100.0, 50.0, 600.0, 800.0));
        }
        time
    }

    #[test]
    fn resets_the_device_to_a_square_front_view_after_orbiting() {
        let mut viewer = viewer(&IOS_PHONE, 0.46, DeviceOrientation::Portrait);
        assert!(viewer.motion.rotation.angle_to(Quat::IDENTITY) < 1e-6);
        assert!(viewer.wheel(480.0, 320.0, 0.0));
        viewer.advance(80.0, false);
        assert!(viewer.motion.rotation.angle_to(Quat::IDENTITY) > 0.05);
        viewer.restore(96.0);
        settle(&mut viewer, 96);
        assert!(!viewer.needs_frame());
        assert!(viewer.motion.rotation.angle_to(Quat::IDENTITY) < 1e-6);
    }

    #[test]
    fn springs_an_ordinary_device_back_toward_its_screen_freezes_captured_input_and_stops_rendering_at_rest()
     {
        let mut viewer = viewer(&IOS_PHONE, 0.46, DeviceOrientation::Portrait);
        assert!(
            viewer.press(110.0, 60.0, false, 0.0).is_empty(),
            "the corner misses the screen and orbits"
        );
        viewer.drag(590.0, 300.0, 16.0);
        viewer.advance(80.0, false);
        let dragged = viewer.motion.rotation;
        assert!(dragged.angle_to(Quat::IDENTITY) > 0.05);
        assert!(viewer.lift(80.0).is_empty());
        let time = settle(&mut viewer, 80);
        let rest = viewer.motion.rotation;
        let yaw = 2.0 * rest.y.atan2(rest.w);
        assert!(
            yaw.abs() <= PI / 3.0 + 1e-6 && rest.angle_to(Quat::IDENTITY) > 0.05,
            "{yaw}"
        );
        let camera = viewer.fit((100.0, 50.0, 600.0, 800.0));
        let center = viewer.position_of((0.5, 0.5)).unwrap();
        let touches = viewer.press(center.0, center.1, false, f64::from(time));
        assert_eq!(touches.len(), 1);
        assert!(
            !viewer.wheel(100.0, 100.0, f64::from(time)),
            "no orbit under a held touch"
        );
        viewer.advance(f64::from(time) + 1000.0, false);
        assert_eq!(viewer.motion.rotation, rest);
        assert_eq!(viewer.fit((100.0, 50.0, 600.0, 800.0)), camera);
        assert!(!viewer.needs_frame());
        viewer.lift(f64::from(time) + 1000.0);
        settle(&mut viewer, time + 1000);
        assert!(!viewer.needs_frame());
    }

    #[test]
    fn preserves_input_projection_for_every_silhouette_in_every_orientation() {
        for profile in [&IOS_TABLET, &IOS_PHONE, &ANDROID_PHONE] {
            for orientation in ORIENTATIONS {
                let aspect = if profile.id == "ios-tablet" {
                    0.75
                } else {
                    0.45
                };
                let model = Model::new(profile, aspect, orientation);
                let camera = Camera {
                    center: (0.0, 0.0),
                    distance: 6.0,
                    aspect: 1.0,
                };
                let turned = model.rotation(yxz(0.2, -0.3));
                let point = Vec3::new(-2.2 * aspect / 4.0, -2.2 / 4.0, SCREEN_Z);
                let (x, y) = camera.project(turned, point);
                let hit = model.screen_point(&camera, turned, x, y, false).unwrap();
                let expected = match orientation {
                    DeviceOrientation::LandscapeLeft => (0.25, 0.25),
                    DeviceOrientation::LandscapeRight => (0.75, 0.75),
                    DeviceOrientation::PortraitUpsideDown => (0.75, 0.25),
                    DeviceOrientation::Portrait => (0.25, 0.75),
                };
                assert!(
                    close(hit.0, expected.0) && close(hit.1, expected.1),
                    "{} {orientation:?}: {hit:?}",
                    profile.id
                );
            }
        }
    }

    #[test]
    fn keeps_device_coordinates_accurate_at_any_camera_distance() {
        for zoom in [0.6, 2.2] {
            let model = Model::new(&IOS_PHONE, 900.0 / 1950.0, DeviceOrientation::Portrait);
            let camera = Camera {
                center: (0.0, 0.0),
                distance: 6.0 / zoom,
                aspect: 0.65,
            };
            let turned = model.rotation(yxz(0.2, -0.3));
            let point = Vec3::new(-2.2 * model.aspect / 4.0, -2.2 / 4.0, SCREEN_Z);
            let (x, y) = camera.project(turned, point);
            let hit = model.screen_point(&camera, turned, x, y, false).unwrap();
            assert!(close(hit.0, 0.25) && close(hit.1, 0.75), "{zoom}: {hit:?}");
        }
    }

    #[test]
    fn rejects_new_gestures_outside_the_display_but_clamps_captured_drags_at_its_edge() {
        let model = Model::new(&IOS_PHONE, 900.0 / 1950.0, DeviceOrientation::Portrait);
        let camera = Camera {
            center: (0.0, 0.0),
            distance: 5.0,
            aspect: 1.0,
        };
        let front = model.rotation(Quat::IDENTITY);
        assert_eq!(model.screen_point(&camera, front, 1.0, 0.5, false), None);
        assert_eq!(
            model.screen_point(&camera, front, 1.0, 0.5, true),
            Some((1.0, 0.5))
        );
        let behind = model.rotation(Quat::axis_angle(Vec3::new(0.0, 1.0, 0.0), PI));
        assert_eq!(model.screen_point(&camera, behind, 0.5, 0.5, false), None);
        let corner = model.screen_local((0.0, 0.0));
        let (x, y) = camera.project(front, corner);
        assert_eq!(
            model.screen_point(&camera, front, x, y, false),
            None,
            "rounded corners miss"
        );
    }

    #[test]
    fn unrotates_an_already_landscape_framebuffer() {
        for orientation in [
            DeviceOrientation::LandscapeLeft,
            DeviceOrientation::LandscapeRight,
        ] {
            let model = Model::new(&IOS_PHONE, 1950.0 / 900.0, orientation);
            let camera = Camera {
                center: (0.0, 0.0),
                distance: 5.0,
                aspect: 1.0,
            };
            let hit = model.screen_point(&camera, model.rotation(Quat::IDENTITY), 0.5, 0.5, false);
            assert_eq!(hit, Some((0.5, 0.5)));
        }
    }

    #[test]
    fn a_tap_on_the_turned_screen_reaches_the_raw_point_the_flat_view_sends_and_the_texel_drawn_there()
     {
        for orientation in ORIENTATIONS {
            for (width, height) in [(588u32, 1280u32), (1280, 588)] {
                let screen = DeviceScreenConfig {
                    width,
                    height,
                    orientation,
                    ..Default::default()
                };
                let rotation = geometry::display_rotation(Some(&screen));
                let shown_aspect = match rotation {
                    DeviceOrientation::LandscapeLeft | DeviceOrientation::LandscapeRight => {
                        f64::from(height) / f64::from(width)
                    }
                    _ => f64::from(width) / f64::from(height),
                };
                let mut viewer = viewer(&IOS_PHONE, shown_aspect, orientation);
                viewer.motion.set_pose(yxz(0.3, -0.45), 0.0, true);
                viewer.fit((100.0, 50.0, 600.0, 800.0));
                for display in [(0.2f32, 0.3f32), (0.5, 0.5), (0.8, 0.9)] {
                    let at = viewer.position_of(display).unwrap();
                    let touches = viewer.press(at.0, at.1, false, 0.0);
                    viewer.lift(0.0);
                    let [(TouchPhase::Begin, hit)] = touches[..] else {
                        panic!("{orientation:?}: {touches:?}");
                    };
                    let raw = geometry::raw_point(rotation, hit.0, hit.1);
                    let flat = geometry::raw_point(rotation, display.0, display.1);
                    assert!(
                        close(raw.0, flat.0) && close(raw.1, flat.1),
                        "{orientation:?} {width}x{height}: {raw:?} vs {flat:?}"
                    );
                }
                let model = viewer.model.clone().unwrap();
                let (_, range) = model
                    .parts
                    .iter()
                    .find(|(part, _)| *part == Part::Screen)
                    .unwrap();
                for vertex in model.vertices[range.clone()].iter().step_by(7) {
                    let [x, y, z] = vertex.position.map(f64::from);
                    let camera = viewer.camera.unwrap();
                    let turned = viewer.rotation().unwrap();
                    let (sx, sy) = camera.project(turned, Vec3::new(x * 0.98, y * 0.98, z));
                    let hit = model.screen_point(&camera, turned, sx, sy, true).unwrap();
                    let (u, v) = (vertex.uv[0], vertex.uv[1]);
                    let tolerance = 0.012;
                    assert!(
                        (hit.0 - (0.5 + (u - 0.5) * 0.98)).abs() < tolerance
                            && (hit.1 - (0.5 + (v - 0.5) * 0.98)).abs() < tolerance,
                        "{orientation:?}: uv {u},{v} vs hit {hit:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn the_displayed_frame_stands_upright_on_the_body_in_every_orientation() {
        let camera = Camera {
            center: (0.0, 0.0),
            distance: 6.0,
            aspect: 1.0,
        };
        for orientation in ORIENTATIONS {
            let model = Model::new(&IOS_PHONE, 0.46, orientation);
            let front = model.rotation(Quat::IDENTITY);
            let at = |x, y| camera.project(front, model.screen_local((x, y)));
            let (left, right, top, bottom) =
                (at(0.1, 0.5), at(0.9, 0.5), at(0.5, 0.1), at(0.5, 0.9));
            assert!(
                left.0 < right.0 && (left.1 - right.1).abs() < 1e-9,
                "{orientation:?}"
            );
            assert!(
                top.1 < bottom.1 && (top.0 - bottom.0).abs() < 1e-9,
                "{orientation:?}"
            );
            let (lo, hi) = model.bounds(Some(Part::Screen));
            let span = |p: Vec3| front.rotate(p);
            let (a, b) = (span(lo), span(hi));
            let landscape = matches!(
                orientation,
                DeviceOrientation::LandscapeLeft | DeviceOrientation::LandscapeRight
            );
            assert_eq!(
                (a.x - b.x).abs() > (a.y - b.y).abs(),
                landscape,
                "{orientation:?}"
            );
            let camera_corner = front.rotate(Vec3::new(1.0, 1.0, 0.0));
            let expected = match orientation {
                DeviceOrientation::Portrait => (1.0, 1.0),
                DeviceOrientation::LandscapeLeft => (1.0, -1.0),
                DeviceOrientation::LandscapeRight => (-1.0, 1.0),
                DeviceOrientation::PortraitUpsideDown => (-1.0, -1.0),
            };
            assert!(
                (camera_corner.x - expected.0).abs() < 1e-9
                    && (camera_corner.y - expected.1).abs() < 1e-9,
                "{orientation:?}: the device top turns to the right for LandscapeLeft, as the Rotator does"
            );
        }
    }

    #[test]
    fn a_press_on_the_screen_touches_and_a_press_on_the_body_or_with_alt_orbits() {
        let mut viewer = viewer(&ANDROID_PHONE, 0.45, DeviceOrientation::Portrait);
        let at = viewer.position_of((0.25, 0.75)).unwrap();
        let begin = viewer.press(at.0, at.1, false, 0.0);
        assert!(
            matches!(begin[..], [(TouchPhase::Begin, (x, y))] if close(x, 0.25) && close(y, 0.75))
        );
        let moved = viewer.drag(10_000.0, at.1, 16.0);
        assert!(
            matches!(moved[..], [(TouchPhase::Move, (x, _))] if x == 1.0),
            "{moved:?}"
        );
        assert!(matches!(viewer.lift(32.0)[..], [(TouchPhase::End, (x, _))] if x == 1.0));
        assert!(!viewer.needs_frame(), "a touch never turns the model");
        assert!(
            viewer.press(at.0, at.1, true, 40.0).is_empty(),
            "alt forces an orbit"
        );
        assert!(viewer.drag(at.0 + 120.0, at.1, 56.0).is_empty());
        viewer.advance(120.0, false);
        assert!(viewer.motion.rotation.angle_to(Quat::IDENTITY) > 0.05);
        assert!(viewer.lift(120.0).is_empty());
    }

    #[test]
    fn keeps_a_captured_screen_gesture_separate_from_orbit_and_ends_it_once() {
        let mut input = Interaction::default();
        let begin = input.begin((0.4, 0.3), Some((0.2, 0.3)));
        assert_eq!(
            begin,
            [
                Action::Active(true, Mode::Touch),
                Action::Touch(TouchPhase::Begin, (0.2, 0.3))
            ]
        );
        assert!(input.begin((0.2, 0.2), Some((0.1, 0.1))).is_empty());
        assert_eq!(
            input.move_to((1.3, 0.6), Some((1.0, 0.6))),
            [Action::Touch(TouchPhase::Move, (1.0, 0.6))]
        );
        assert!(
            input.move_to((1.4, 0.6), None).is_empty(),
            "a lost projection keeps the last point"
        );
        assert_eq!(
            input.end(),
            [
                Action::Touch(TouchPhase::End, (1.0, 0.6)),
                Action::Active(false, Mode::Touch)
            ]
        );
        assert!(input.end().is_empty());
    }

    #[test]
    fn orbits_without_sending_touches_when_dragged_outside_the_screen() {
        let mut input = Interaction::default();
        assert_eq!(
            input.begin((10.0, 20.0), None),
            [Action::Active(true, Mode::Orbit)]
        );
        assert_eq!(
            input.move_to((40.0, 50.0), None),
            [Action::Orbit(30.0, 30.0)]
        );
        assert_eq!(input.end(), [Action::Active(false, Mode::Orbit)]);
    }

    #[test]
    fn blocks_navigation_during_either_captured_gesture_then_resumes() {
        let mut input = Interaction::default();
        input.begin((0.5, 0.5), Some((0.5, 0.5)));
        assert_eq!(input.navigate(0.1, 0.2), None);
        input.end();
        input.begin((0.5, 0.5), None);
        assert_eq!(input.navigate(0.1, 0.2), None);
        input.end();
        assert_eq!(input.navigate(0.1, 0.2), Some(Action::Orbit(0.1, 0.2)));
    }

    #[test]
    fn ends_wheel_orbit_without_ending_a_captured_screen_touch() {
        let mut input = Interaction::default();
        assert_eq!(input.end_wheel(), Some(Action::Active(false, Mode::Orbit)));
        input.begin((0.5, 0.5), Some((0.5, 0.5)));
        assert_eq!(input.end_wheel(), None);
        assert_eq!(
            input.end().last(),
            Some(&Action::Active(false, Mode::Touch))
        );
    }

    #[test]
    fn normalizes_wheel_units_and_bounds_jumps_from_coarse_scroll_wheels() {
        assert_eq!(
            wheel_orbit(-16.0, -32.0, false, 400.0, 800.0),
            Some((-16.0, -32.0))
        );
        assert_eq!(
            wheel_orbit(-1.0, -2.0, true, 400.0, 800.0),
            Some((-16.0, -32.0))
        );
        assert_eq!(
            wheel_orbit(-1000.0, 1000.0, false, 400.0, 800.0),
            Some((-100.0, 200.0))
        );
        assert_eq!(wheel_orbit(1.0, 1.0, false, 0.0, 800.0), None);
        assert_eq!(wheel_orbit(1.0, f64::NAN, false, 400.0, 800.0), None);
    }

    #[test]
    fn trackpad_momentum_after_the_fingers_lift_is_ignored_until_the_next_gesture() {
        use gpui::TouchPhase as Phase;
        let mut wheel = Wheel::default();
        assert_eq!(
            wheel.step(Phase::Moved, true),
            WheelStep::Orbit,
            "a mouse wheel has no phases"
        );
        assert_eq!(wheel.step(Phase::Started, false), WheelStep::Orbit);
        assert_eq!(wheel.step(Phase::Moved, false), WheelStep::Orbit);
        assert_eq!(wheel.step(Phase::Ended, false), WheelStep::End);
        assert_eq!(wheel.step(Phase::Moved, false), WheelStep::Ignore);
        assert_eq!(wheel.step(Phase::Moved, true), WheelStep::Orbit);
        assert_eq!(wheel.step(Phase::Started, false), WheelStep::Orbit);
        assert_eq!(wheel.step(Phase::Moved, false), WheelStep::Orbit);
    }

    #[test]
    fn renders_only_when_a_frame_arrives_the_pose_moves_or_the_view_resizes() {
        let mut viewer = viewer(&IOS_PHONE, 0.46, DeviceOrientation::Portrait);
        let mut schedule = Schedule::default();
        let renders = Cell::new(0);
        let mut frame = |viewer: &mut Viewer, source: usize, size: (f64, f64), now: f64| {
            viewer.advance(now, false);
            let camera = viewer.fit((0.0, 0.0, size.0, size.1)).unwrap();
            let model = viewer.model.clone().unwrap();
            let (mvp, _) = camera.matrices(viewer.rotation().unwrap());
            let key = RenderKey {
                source,
                model: model.id,
                mvp,
                size: (size.0 as usize, size.1 as usize),
                clear: [0.1, 0.1, 0.1],
            };
            if schedule.due(key) {
                renders.set(renders.get() + 1);
            }
            viewer.needs_frame()
        };
        assert!(!frame(&mut viewer, 1, (600.0, 800.0), 0.0));
        for time in 1..30 {
            assert!(!frame(
                &mut viewer,
                1,
                (600.0, 800.0),
                f64::from(time) * 16.0
            ));
        }
        assert_eq!(renders.get(), 1, "idle frames reuse the last render");
        frame(&mut viewer, 2, (600.0, 800.0), 500.0);
        frame(&mut viewer, 2, (640.0, 800.0), 516.0);
        assert_eq!(
            renders.get(),
            3,
            "a new frame and a resize each render once"
        );
        viewer.wheel(200.0, 0.0, 520.0);
        viewer.end_wheel(520.0);
        let mut time = 520.0;
        let mut animating = 0;
        while frame(&mut viewer, 2, (640.0, 800.0), time) {
            time += 16.0;
            animating += 1;
            assert!(animating < 400);
        }
        assert!(animating > 5);
        let settled = renders.get();
        frame(&mut viewer, 2, (640.0, 800.0), time + 16.0);
        assert_eq!(
            renders.get(),
            settled,
            "nothing renders once the spring settles"
        );
    }
}
