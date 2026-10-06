use std::cell::RefCell;
use std::f64::consts::{FRAC_PI_2, PI};
use std::ops::Range;
use std::rc::Rc;

use zeron_proto::device_hub::{
    DeviceFacing, DeviceOrientation, DeviceScreenConfig, HingeCommand, HingePose, TouchPhase,
};

use super::scene3d::{
    self, Action, Builder, Camera, IOS_PHONE, Interaction, Mode, Motion, Quat, SCREEN_HEIGHT, Vec3,
    Vertex,
};

pub const COVER_SIZE: (u32, u32) = (1398, 2034);
pub const INNER_SIZE: (u32, u32) = (2006, 2852);

const BEZEL: f64 = 0.055;
const HINGE_INSET: f64 = 0.012;
const LEAF_DEPTH: f64 = 0.045;
const LEAF_RADIUS: f64 = 0.15;
const SCREEN_LIFT: f64 = 0.003;
const PIVOT_GAP: f64 = 0.003;
const BEVEL: f64 = 0.008;
const PIVOT: Vec3 = Vec3::new(0.0, 0.0, SCREEN_LIFT + PIVOT_GAP);
const Y: Vec3 = Vec3::new(0.0, 1.0, 0.0);

const ORIENTATIONS: [DeviceOrientation; 4] = [
    DeviceOrientation::Portrait,
    DeviceOrientation::LandscapeLeft,
    DeviceOrientation::PortraitUpsideDown,
    DeviceOrientation::LandscapeRight,
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DuoPanel {
    Cover = 1,
    Inner = 3,
}

impl DuoPanel {
    pub fn of(screen_id: Option<u32>) -> Self {
        if screen_id == Some(1) {
            DuoPanel::Cover
        } else {
            DuoPanel::Inner
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Leaf {
    Left,
    Right,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DuoFace {
    Cover,
    Inside,
    Left,
    Right,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DuoRestFrame {
    pub face: DuoFace,
    pub normal: Vec3,
    pub up: Vec3,
    pub center: Vec3,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DuoViewSnap {
    pub rotation: Quat,
    pub face: DuoFace,
    pub orientation: DeviceOrientation,
    pub center: Vec3,
    pub yaw_limit: f64,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rigid {
    pub rotation: Quat,
    pub translation: Vec3,
}

impl Rigid {
    pub fn apply(self, point: Vec3) -> Vec3 {
        self.rotation.rotate(point) + self.translation
    }

    pub fn turned(self, root: Quat) -> Rigid {
        Rigid {
            rotation: root.mul(self.rotation),
            translation: root.rotate(self.translation),
        }
    }

    fn inverse(self) -> Rigid {
        let rotation = self.rotation.inverse();
        Rigid {
            rotation,
            translation: rotation.rotate(self.translation) * -1.0,
        }
    }

    pub fn matrix(self) -> [[f32; 4]; 4] {
        let mut matrix = self.rotation.matrix();
        let t = self.translation;
        matrix[3] = [t.x as f32, t.y as f32, t.z as f32, 1.0];
        matrix
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DisplayQuad {
    pub panel: DuoPanel,
    pub leaf: Leaf,
    pub z: f64,
    pub facing: f64,
    pub x: (f64, f64),
    pub y: (f64, f64),
    u: (f64, f64),
}

impl DisplayQuad {
    fn contains(&self, x: f64, y: f64) -> bool {
        (self.x.0..=self.x.1).contains(&x) && (self.y.0..=self.y.1).contains(&y)
    }

    fn panel_uv(&self, x: f64, y: f64) -> (f64, f64) {
        let t = (x - self.x.0) / (self.x.1 - self.x.0);
        (
            self.u.0 + t * (self.u.1 - self.u.0),
            (self.y.1 - y) / (self.y.1 - self.y.0),
        )
    }

    fn local(&self, u: f64, v: f64) -> Vec3 {
        let t = (u - self.u.0) / (self.u.1 - self.u.0);
        Vec3::new(
            self.x.0 + t * (self.x.1 - self.x.0),
            self.y.1 - v * (self.y.1 - self.y.0),
            self.z,
        )
    }

    fn outline(&self, radius: f64) -> Vec<(f64, f64)> {
        let (x0, x1, y0, y1) = (self.x.0, self.x.1, self.y.0, self.y.1);
        let round = |outer: bool| if outer { radius } else { 0.0 };
        let (left, right) = match (self.panel, self.leaf) {
            (DuoPanel::Cover, _) => (true, true),
            (DuoPanel::Inner, Leaf::Left) => (true, false),
            (DuoPanel::Inner, Leaf::Right) => (false, true),
        };
        let corners = [
            ((x1, y0), (-1.0, 0.0), (0.0, 1.0), round(right)),
            ((x1, y1), (0.0, -1.0), (-1.0, 0.0), round(right)),
            ((x0, y1), (1.0, 0.0), (0.0, -1.0), round(left)),
            ((x0, y0), (0.0, 1.0), (1.0, 0.0), round(left)),
        ];
        let mut points = Vec::new();
        for ((cx, cy), (ax, ay), (bx, by), r) in corners {
            if r <= 0.0 {
                points.push((cx, cy));
                continue;
            }
            let (a, b) = ((cx + ax * r, cy + ay * r), (cx + bx * r, cy + by * r));
            for i in 0..=16 {
                let t = f64::from(i) / 16.0;
                let s = 1.0 - t;
                points.push((
                    s * s * a.0 + 2.0 * s * t * cx + t * t * b.0,
                    s * s * a.1 + 2.0 * s * t * cy + t * t * b.1,
                ));
            }
        }
        points
    }

    fn corners(&self) -> [Vec3; 4] {
        [
            (self.x.0, self.y.0),
            (self.x.1, self.y.0),
            (self.x.1, self.y.1),
            (self.x.0, self.y.1),
        ]
        .map(|(x, y)| Vec3::new(x, y, self.z))
    }
}

pub fn duo_raw_point(panel: DuoPanel, u: f64, v: f64) -> (f32, f32) {
    match panel {
        DuoPanel::Cover => (u as f32, v as f32),
        DuoPanel::Inner => (v as f32, (1.0 - u) as f32),
    }
}

fn panel_point(panel: DuoPanel, (x, y): (f32, f32)) -> (f64, f64) {
    match panel {
        DuoPanel::Cover => (f64::from(x), f64::from(y)),
        DuoPanel::Inner => (1.0 - f64::from(y), f64::from(x)),
    }
}

pub fn duo_frame_matches(frame: (u32, u32), screen: (u32, u32)) -> bool {
    let [fw, fh, sw, sh] = [frame.0, frame.1, screen.0, screen.1].map(f64::from);
    fw > 0.0 && fh > 0.0 && sh > 0.0 && (fw / fh - sw / sh).abs() <= 1.0 / fh + 1.0 / sh
}

pub fn duo_raw_size((width, height): (u32, u32), rotation: DeviceOrientation) -> (u32, u32) {
    match rotation {
        DeviceOrientation::LandscapeLeft | DeviceOrientation::LandscapeRight => (height, width),
        _ => (width, height),
    }
}

pub fn quarter_turns(rotation: DeviceOrientation) -> u8 {
    match rotation {
        DeviceOrientation::Portrait => 0,
        DeviceOrientation::LandscapeLeft => 1,
        DeviceOrientation::PortraitUpsideDown => 2,
        DeviceOrientation::LandscapeRight => 3,
    }
}

pub type DisplayKey = (Option<u32>, u32, u32, DeviceOrientation);

pub fn duo_display_key(screen: Option<&DeviceScreenConfig>) -> Option<DisplayKey> {
    screen.map(|screen| {
        (
            screen.screen_id,
            screen.width,
            screen.height,
            screen.orientation,
        )
    })
}

pub fn leaf_rotation(leaf: Leaf, angle: f64) -> Quat {
    let half = (180.0 - angle.clamp(0.0, 180.0)) * PI / 360.0;
    Quat::axis_angle(Y, if leaf == Leaf::Left { half } else { -half })
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Fold {
    pub leaves: [Rigid; 2],
    pub bounds: (Vec3, Vec3),
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DuoHit {
    pub panel: DuoPanel,
    pub display: usize,
    pub uv: (f64, f64),
}

fn grow((low, high): (Vec3, Vec3), point: Vec3) -> (Vec3, Vec3) {
    (
        Vec3::new(low.x.min(point.x), low.y.min(point.y), low.z.min(point.z)),
        Vec3::new(
            high.x.max(point.x),
            high.y.max(point.y),
            high.z.max(point.z),
        ),
    )
}

const EMPTY: (Vec3, Vec3) = (
    Vec3::new(f64::INFINITY, f64::INFINITY, f64::INFINITY),
    Vec3::new(f64::NEG_INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY),
);

fn box_corners((lo, hi): (Vec3, Vec3)) -> [Vec3; 8] {
    std::array::from_fn(|i| {
        Vec3::new(
            if i & 1 == 0 { lo.x } else { hi.x },
            if i & 2 == 0 { lo.y } else { hi.y },
            if i & 4 == 0 { lo.z } else { hi.z },
        )
    })
}

fn slab(origin: Vec3, direction: Vec3, (lo, hi): (Vec3, Vec3)) -> Option<f64> {
    let (mut near, mut far) = (0.0f64, f64::INFINITY);
    for (o, d, l, h) in [
        (origin.x, direction.x, lo.x, hi.x),
        (origin.y, direction.y, lo.y, hi.y),
        (origin.z, direction.z, lo.z, hi.z),
    ] {
        if d.abs() < 1e-12 {
            if o < l || o > h {
                return None;
            }
        } else {
            let (a, b) = ((l - o) / d, (h - o) / d);
            near = near.max(a.min(b));
            far = far.min(a.max(b));
        }
    }
    (near <= far).then_some(near)
}

// The rig follows the hub-reported display aspects; the hinge, bezel and depth values are generic.
pub struct DuoRig {
    pub id: u64,
    pub sizes: [(u32, u32); 2],
    pub vertices: Vec<Vertex>,
    pub left: Range<usize>,
    pub right: Range<usize>,
    pub displays: [DisplayQuad; 3],
    pub leaf_boxes: [(Vec3, Vec3); 2],
}

impl DuoRig {
    pub fn new(cover: (u32, u32), inner: (u32, u32)) -> Self {
        let (cover, inner) = (sane(cover, COVER_SIZE), sane(inner, INNER_SIZE));
        let half = SCREEN_HEIGHT * f64::from(inner.1) / f64::from(inner.0) / 2.0;
        let leaf_width = half + HINGE_INSET + BEZEL;
        let leaf_height = SCREEN_HEIGHT + 2.0 * BEZEL;
        let aspect = f64::from(cover.0) / f64::from(cover.1);
        let cover_height = SCREEN_HEIGHT.min((leaf_width - 2.0 * BEZEL) / aspect);
        let cover_width = cover_height * aspect;
        let cover_x = -leaf_width / 2.0;
        let screen_y = (-SCREEN_HEIGHT / 2.0, SCREEN_HEIGHT / 2.0);
        let displays = [
            DisplayQuad {
                panel: DuoPanel::Cover,
                leaf: Leaf::Left,
                z: -LEAF_DEPTH - SCREEN_LIFT,
                facing: -1.0,
                x: (cover_x - cover_width / 2.0, cover_x + cover_width / 2.0),
                y: (-cover_height / 2.0, cover_height / 2.0),
                u: (1.0, 0.0),
            },
            DisplayQuad {
                panel: DuoPanel::Inner,
                leaf: Leaf::Left,
                z: SCREEN_LIFT,
                facing: 1.0,
                x: (-(half + HINGE_INSET), -HINGE_INSET),
                y: screen_y,
                u: (0.0, 0.5),
            },
            DisplayQuad {
                panel: DuoPanel::Inner,
                leaf: Leaf::Right,
                z: SCREEN_LIFT,
                facing: 1.0,
                x: (HINGE_INSET, half + HINGE_INSET),
                y: screen_y,
                u: (0.5, 1.0),
            },
        ];
        let (metal, glass, back) = (
            scene3d::rgb(scene3d::METAL),
            scene3d::rgb(scene3d::GLASS),
            scene3d::rgb(IOS_PHONE.back_color),
        );
        let mut vertices = Vec::new();
        let mut ranges = Vec::new();
        for (leaf, side) in [(Leaf::Left, -1.0), (Leaf::Right, 1.0)] {
            let mut mesh = Builder::new();
            mesh.extrude(
                (
                    leaf_width - 2.0 * BEVEL,
                    leaf_height - 2.0 * BEVEL,
                    LEAF_RADIUS,
                ),
                BEVEL - LEAF_DEPTH,
                LEAF_DEPTH - 2.0 * BEVEL,
                BEVEL,
                BEVEL,
                3,
                metal,
            );
            let plate = scene3d::rounded(
                leaf_width - 0.014,
                leaf_height - 0.014,
                LEAF_RADIUS - 0.01,
                16,
            );
            mesh.cap((0.0, 0.0), &plate, SCREEN_LIFT / 2.0, 1.0, glass, None);
            mesh.cap(
                (0.0, 0.0),
                &plate,
                -LEAF_DEPTH - SCREEN_LIFT / 2.0,
                -1.0,
                back,
                None,
            );
            for vertex in &mut mesh.vertices {
                vertex.position[0] += (side * leaf_width / 2.0) as f32;
            }
            for display in displays.iter().filter(|display| display.leaf == leaf) {
                mesh.screen_tag = display.panel as u32 as f32;
                let outline = display.outline(IOS_PHONE.screen_radius);
                let uv = |x: f64, y: f64| {
                    let (u, v) = display.panel_uv(x, y);
                    duo_raw_point(display.panel, u, v)
                };
                let center = (
                    (display.x.0 + display.x.1) / 2.0,
                    (display.y.0 + display.y.1) / 2.0,
                );
                mesh.cap(
                    center,
                    &outline,
                    display.z,
                    display.facing,
                    [0.0; 3],
                    Some(&uv),
                );
            }
            let start = vertices.len();
            vertices.extend(mesh.vertices);
            ranges.push(start..vertices.len());
        }
        let y = leaf_height / 2.0;
        Self {
            id: scene3d::next_model_id(),
            sizes: [cover, inner],
            vertices,
            right: ranges.pop().unwrap_or_default(),
            left: ranges.pop().unwrap_or_default(),
            displays,
            leaf_boxes: [
                (
                    Vec3::new(-leaf_width, -y, -LEAF_DEPTH),
                    Vec3::new(0.0, y, 0.0),
                ),
                (
                    Vec3::new(0.0, -y, -LEAF_DEPTH),
                    Vec3::new(leaf_width, y, 0.0),
                ),
            ],
        }
    }

    pub fn fold(&self, angle: f64) -> Fold {
        let mut leaves = [Leaf::Left, Leaf::Right].map(|leaf| {
            let rotation = leaf_rotation(leaf, angle);
            Rigid {
                rotation,
                translation: PIVOT - rotation.rotate(PIVOT),
            }
        });
        let mut bounds = EMPTY;
        for (leaf, body) in leaves.iter().zip(self.leaf_boxes) {
            for corner in box_corners(body) {
                bounds = grow(bounds, leaf.apply(corner));
            }
        }
        let offset = (bounds.0 + bounds.1) * -0.5;
        for leaf in &mut leaves {
            leaf.translation = leaf.translation + offset;
        }
        Fold {
            leaves,
            bounds: (bounds.0 + offset, bounds.1 + offset),
        }
    }

    pub fn rest_frames(&self, angle: f64, panel: DuoPanel) -> Vec<DuoRestFrame> {
        let fold = self.fold(angle);
        let frame = |indices: &[usize], face| {
            let (mut bounds, mut normal, mut up) = (EMPTY, Vec3::ZERO, Vec3::ZERO);
            for &index in indices {
                let display = self.displays[index];
                let leaf = fold.leaves[display.leaf as usize];
                for corner in display.corners() {
                    bounds = grow(bounds, leaf.apply(corner));
                }
                normal = normal + leaf.rotation.rotate(Vec3::new(0.0, 0.0, display.facing));
                up = up + leaf.rotation.rotate(Y);
            }
            DuoRestFrame {
                face,
                normal: normal.normalize(),
                up: up.normalize(),
                center: (bounds.0 + bounds.1) * 0.5,
            }
        };
        match panel {
            DuoPanel::Cover => vec![frame(&[0], DuoFace::Cover)],
            DuoPanel::Inner => {
                let inside = frame(&[1, 2], DuoFace::Inside);
                if angle > 20.0 && angle < 165.0 {
                    vec![
                        inside,
                        frame(&[1], DuoFace::Left),
                        frame(&[2], DuoFace::Right),
                    ]
                } else {
                    vec![inside]
                }
            }
        }
    }

    fn cast(
        &self,
        fold: &Fold,
        root: Quat,
        (origin, direction): (Vec3, Vec3),
    ) -> Option<(Leaf, Option<(usize, Vec3)>)> {
        let mut best: Option<(f64, Leaf, Option<(usize, Vec3)>)> = None;
        let mut consider = |t: f64, leaf, display| {
            if best.is_none_or(|(nearest, ..)| t < nearest) {
                best = Some((t, leaf, display));
            }
        };
        for leaf in [Leaf::Left, Leaf::Right] {
            let inverse = fold.leaves[leaf as usize].turned(root).inverse();
            let (o, d) = (inverse.apply(origin), inverse.rotation.rotate(direction));
            if let Some(t) = slab(o, d, self.leaf_boxes[leaf as usize]) {
                consider(t, leaf, None);
            }
            for (index, display) in self.displays.iter().enumerate() {
                if display.leaf != leaf || d.z * display.facing >= 0.0 {
                    continue;
                }
                let t = (display.z - o.z) / d.z;
                let at = o + d * t;
                if t >= 0.0 && display.contains(at.x, at.y) {
                    consider(t, leaf, Some((index, at)));
                }
            }
        }
        best.map(|(_, leaf, display)| (leaf, display))
    }

    pub fn hit(&self, fold: &Fold, camera: &Camera, root: Quat, x: f64, y: f64) -> Option<DuoHit> {
        let (index, at) = self.cast(fold, root, camera.ray(x, y))?.1?;
        let display = self.displays[index];
        Some(DuoHit {
            panel: display.panel,
            display: index,
            uv: display.panel_uv(at.x, at.y),
        })
    }

    pub fn leaf_at(
        &self,
        fold: &Fold,
        camera: &Camera,
        root: Quat,
        x: f64,
        y: f64,
    ) -> Option<Leaf> {
        if !(x.is_finite() && y.is_finite()) {
            return None;
        }
        Some(self.cast(fold, root, camera.ray(x, y))?.0)
    }

    pub fn plane_uv(
        &self,
        fold: &Fold,
        camera: &Camera,
        root: Quat,
        display: usize,
        x: f64,
        y: f64,
    ) -> Option<(f64, f64)> {
        let quad = self.displays.get(display)?;
        let inverse = fold.leaves[quad.leaf as usize].turned(root).inverse();
        let (origin, direction) = camera.ray(x, y);
        let (o, d) = (inverse.apply(origin), inverse.rotation.rotate(direction));
        if d.z == 0.0 {
            return None;
        }
        let t = (quad.z - o.z) / d.z;
        if t < 0.0 {
            return None;
        }
        let at = o + d * t;
        let (u, v) = quad.panel_uv(at.x, at.y);
        Some((u.clamp(0.0, 1.0), v.clamp(0.0, 1.0)))
    }

    pub fn position_of(&self, fold: &Fold, root: Quat, panel: DuoPanel, raw: (f32, f32)) -> Vec3 {
        let (u, v) = panel_point(panel, raw);
        let index = match panel {
            DuoPanel::Cover => 0,
            DuoPanel::Inner if u < 0.5 => 1,
            DuoPanel::Inner => 2,
        };
        let quad = self.displays[index];
        fold.leaves[quad.leaf as usize]
            .turned(root)
            .apply(quad.local(u, v))
    }
}

fn is_leaf(face: DuoFace) -> bool {
    matches!(face, DuoFace::Left | DuoFace::Right)
}

// Ported from t3code (duoSnap): views come from the hinged display planes, not per-model Euler offsets.
pub fn duo_view_snaps(frames: &[DuoRestFrame], panel: DuoPanel) -> Vec<DuoViewSnap> {
    let z = Vec3::new(0.0, 0.0, 1.0);
    let leaves = || frames.iter().filter(|frame| is_leaf(frame.face));
    let mut snaps = Vec::new();
    for frame in frames {
        let normal = frame.normal.normalize();
        if normal.dot(normal) < 0.5 {
            continue;
        }
        let right = scene3d::cross(frame.up, normal).normalize();
        let up = scene3d::cross(normal, right).normalize();
        let face_rotation = Quat::from_basis(right, up, normal).inverse();
        for (index, orientation) in ORIENTATIONS.into_iter().enumerate() {
            if (frame.face == DuoFace::Right && index != 0)
                || (frame.face == DuoFace::Left && index != 2)
            {
                continue;
            }
            let base = if panel == DuoPanel::Inner {
                FRAC_PI_2
            } else {
                0.0
            };
            let roll = base - index as f64 * FRAC_PI_2;
            let mut rotation = Quat::axis_angle(z, roll).mul(face_rotation);
            if is_leaf(frame.face) {
                rotation = Quat::axis_angle(Vec3::new(1.0, 0.0, 0.0), PI / 12.0).mul(rotation);
            }
            if panel == DuoPanel::Inner
                && leaves().any(|leaf| rotation.rotate(leaf.normal).z < 0.04)
            {
                continue;
            }
            let mut yaw_limit = if frame.face == DuoFace::Cover {
                PI / 9.0
            } else {
                PI / 3.0
            };
            if panel == DuoPanel::Inner {
                for sign in [-1.0, 1.0] {
                    for step in 1..=60 {
                        let turn = Quat::axis_angle(Y, sign * f64::from(step) * PI / 180.0);
                        if leaves().any(|leaf| turn.rotate(rotation.rotate(leaf.normal)).z < 0.04) {
                            yaw_limit = yaw_limit.min(f64::from(step - 1) * PI / 180.0);
                            break;
                        }
                    }
                }
            }
            snaps.push(DuoViewSnap {
                rotation,
                face: frame.face,
                orientation,
                center: frame.center,
                yaw_limit,
            });
        }
    }
    snaps
}

pub fn nearest_duo_view(rotation: Quat, snaps: &[DuoViewSnap]) -> Option<DuoViewSnap> {
    let families: Vec<(Quat, f64)> = snaps
        .iter()
        .map(|snap| (snap.rotation, snap.yaw_limit))
        .collect();
    scene3d::nearest_index(rotation, &families).map(|(index, rotation)| DuoViewSnap {
        rotation,
        ..snaps[index]
    })
}

const HANDOFF_MS: f64 = 5000.0;

pub fn default_angle(screen: Option<&DeviceScreenConfig>) -> f64 {
    let screen_id = screen.and_then(|screen| screen.screen_id);
    match screen.and_then(|screen| screen.hinge_angle) {
        Some(angle) => f64::from(angle),
        None if screen_id == Some(1) => 0.0,
        None => 180.0,
    }
}

pub fn pinch_log_scale(delta: f32) -> Option<f64> {
    (delta.is_finite() && delta > -1.0).then(|| f64::from(1.0 + delta).ln())
}

// Ported from t3code (duoControl createDuoPinch): the accumulator ignores readback while active.
#[derive(Debug, Default, PartialEq)]
pub struct DuoPinch {
    angle: Option<f64>,
}

impl DuoPinch {
    pub fn begin(&mut self, contains: bool, angle: f64) -> bool {
        if !contains {
            return false;
        }
        self.angle = Some(angle.clamp(0.0, 180.0));
        true
    }

    pub fn moved(&mut self, log_scale: f64) -> Option<f64> {
        let angle = self.angle?;
        if !log_scale.is_finite() {
            return None;
        }
        let next = (angle + log_scale * 120.0).clamp(0.0, 180.0);
        if next == angle {
            return None;
        }
        self.angle = Some(next);
        Some(next)
    }

    pub fn end(&mut self) -> bool {
        self.angle.take().is_some()
    }

    pub fn active(&self) -> bool {
        self.angle.is_some()
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct Handoff {
    panel: DuoPanel,
    deadline: f64,
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct Captured {
    display: usize,
    key: DisplayKey,
}

#[derive(Default)]
struct SnapState {
    candidates: Vec<DuoViewSnap>,
    chosen: Option<DuoViewSnap>,
}

fn euler_yxz(x: f64, y: f64, z: f64) -> Quat {
    Quat::axis_angle(Y, y)
        .mul(Quat::axis_angle(Vec3::new(1.0, 0.0, 0.0), x))
        .mul(Quat::axis_angle(Vec3::new(0.0, 0.0, 1.0), z))
}

// Ported from t3code (duoViewer): presentation, snaps, display handoff, hinge preview and touch.
pub struct DuoViewer {
    pub motion: Motion,
    pub interaction: Interaction,
    snaps: Rc<RefCell<SnapState>>,
    rig: Option<Rc<DuoRig>>,
    viewport: Option<(f64, f64, f64, f64)>,
    screen: Option<DeviceScreenConfig>,
    angle: f64,
    target_angle: f64,
    applied_angle: f64,
    preview: Option<f64>,
    queued: Option<f64>,
    held_leaf: Option<Leaf>,
    touch_active: bool,
    rest_face: DuoFace,
    requested_orientation: Option<DeviceOrientation>,
    view_orientation: Option<DeviceOrientation>,
    handoff: Option<Handoff>,
    first_pose: bool,
    physical_pose: HingePose,
    presentation_angle: f64,
    target_presentation: Quat,
    last_time: f64,
    ready_key: Option<DisplayKey>,
    captured: Option<Captured>,
    pinch: DuoPinch,
    requests: Vec<HingeCommand>,
}

impl Default for DuoViewer {
    fn default() -> Self {
        let snaps = Rc::new(RefCell::new(SnapState::default()));
        let shared = snaps.clone();
        let motion = Motion::new(move |rotation| {
            let mut state = shared.borrow_mut();
            match nearest_duo_view(rotation, &state.candidates) {
                Some(snap) => {
                    state.chosen = Some(snap);
                    snap.rotation
                }
                None => rotation,
            }
        });
        Self {
            motion,
            interaction: Interaction::default(),
            snaps,
            rig: None,
            viewport: None,
            screen: None,
            angle: 180.0,
            target_angle: 180.0,
            applied_angle: f64::NAN,
            preview: None,
            queued: None,
            held_leaf: None,
            touch_active: false,
            rest_face: DuoFace::Inside,
            requested_orientation: None,
            view_orientation: None,
            handoff: None,
            first_pose: true,
            physical_pose: HingePose::Open,
            presentation_angle: 180.0,
            target_presentation: Quat::IDENTITY,
            last_time: 0.0,
            ready_key: None,
            captured: None,
            pinch: DuoPinch::default(),
            requests: Vec::new(),
        }
    }
}

impl DuoViewer {
    pub fn set_rig(&mut self, cover: (u32, u32), inner: (u32, u32)) -> Rc<DuoRig> {
        if let Some(rig) = &self.rig
            && rig.sizes == [sane(cover, COVER_SIZE), sane(inner, INNER_SIZE)]
        {
            return rig.clone();
        }
        let rig = Rc::new(DuoRig::new(cover, inner));
        self.rig = Some(rig.clone());
        self.applied_angle = f64::NAN;
        rig
    }

    pub fn rig(&self) -> Option<Rc<DuoRig>> {
        self.rig.clone()
    }

    pub fn screen(&self) -> Option<&DeviceScreenConfig> {
        self.screen.as_ref()
    }

    pub fn angle(&self) -> f64 {
        self.angle
    }

    pub fn viewport(&self) -> Option<(f64, f64, f64, f64)> {
        self.viewport
    }

    fn panel(&self) -> DuoPanel {
        DuoPanel::of(self.screen.as_ref().and_then(|screen| screen.screen_id))
    }

    fn snaps_for(&self, panel: DuoPanel, angle: f64) -> Vec<DuoViewSnap> {
        self.rig.as_ref().map_or_else(Vec::new, |rig| {
            duo_view_snaps(&rig.rest_frames(angle, panel), panel)
        })
    }

    fn active_snaps(&self) -> Vec<DuoViewSnap> {
        self.snaps_for(self.panel(), self.angle)
    }

    fn all_snaps(&self) -> Vec<DuoViewSnap> {
        let physical = self
            .screen
            .as_ref()
            .is_some_and(|screen| screen.supports_physical_orientation == Some(true));
        if !physical || !(self.angle > 20.0 && self.angle < 180.0) {
            return self.active_snaps();
        }
        let mut snaps = self.snaps_for(DuoPanel::Inner, self.angle);
        snaps.extend(self.snaps_for(DuoPanel::Cover, self.angle));
        snaps
    }

    fn moving(&self) -> bool {
        (self.angle - self.target_angle).abs() > 0.01
    }

    fn hold_rule(&self) -> Leaf {
        if self.panel() == DuoPanel::Cover && self.angle > 20.0 {
            Leaf::Left
        } else {
            Leaf::Right
        }
    }

    fn release<R>(&mut self, now: f64, act: impl FnOnce(&mut Motion) -> R) -> R {
        self.snaps.borrow_mut().candidates = self.all_snaps();
        let result = act(&mut self.motion);
        let chosen = self.snaps.borrow_mut().chosen.take();
        if let Some(snap) = chosen {
            self.chose(snap, now);
        }
        result
    }

    fn chose(&mut self, snap: DuoViewSnap, now: f64) {
        self.rest_face = snap.face;
        let Some(screen) = &self.screen else {
            return;
        };
        let panel = if snap.face == DuoFace::Cover {
            DuoPanel::Cover
        } else {
            DuoPanel::Inner
        };
        let current = self
            .handoff
            .map(|handoff| Some(handoff.panel as u32))
            .unwrap_or(screen.screen_id);
        if Some(panel as u32) != current {
            self.handoff = Some(Handoff {
                panel,
                deadline: now + HANDOFF_MS,
            });
            self.captured = None;
            self.requests.push(HingeCommand::Physical(match panel {
                DuoPanel::Cover => DeviceFacing::FaceDown,
                DuoPanel::Inner => DeviceFacing::FaceUp,
            }));
        } else if self.handoff.is_none() && Some(snap.orientation) != self.view_orientation {
            self.view_orientation = Some(snap.orientation);
            self.requested_orientation = Some(snap.orientation);
            self.requests
                .push(HingeCommand::Orientation(snap.orientation));
        }
    }

    fn hold_motion(&mut self, now: f64) {
        let held = self.touch_active || self.preview.is_some();
        self.release(now, |motion| motion.hold(held, now));
    }

    fn apply_pose(&mut self, now: f64) {
        if let Some(leaf) = self.held_leaf
            && self.applied_angle.is_finite()
            && self.applied_angle != self.angle
        {
            let correction = leaf_rotation(leaf, self.applied_angle)
                .mul(leaf_rotation(leaf, self.angle).inverse());
            if correction.angle_to(Quat::IDENTITY) > 1e-8 {
                let rotation = self.motion.rotation.mul(correction);
                self.motion.set_pose(rotation, now, true);
            }
        }
        self.applied_angle = self.angle;
    }

    fn presentation(&self, screen: Option<&DeviceScreenConfig>) -> Quat {
        let fold = (180.0 - self.presentation_angle) * PI / 360.0;
        let mut roll = if self.physical_pose == HingePose::Closed {
            0.0
        } else {
            FRAC_PI_2
        };
        if let Some(screen) = screen {
            if screen.width < screen.height {
                match screen.orientation {
                    DeviceOrientation::LandscapeLeft => roll -= FRAC_PI_2,
                    DeviceOrientation::LandscapeRight => roll += FRAC_PI_2,
                    _ => {}
                }
            }
            if screen.orientation == DeviceOrientation::PortraitUpsideDown {
                roll -= PI;
            }
        }
        match self.physical_pose {
            HingePose::Laptop => euler_yxz(-fold + PI / 9.0, -PI / 9.0, FRAC_PI_2),
            HingePose::Tent => euler_yxz(FRAC_PI_2 + PI / 18.0, -PI / 9.0, -FRAC_PI_2),
            _ => Quat::axis_angle(Vec3::new(0.0, 0.0, 1.0), roll).mul(euler_yxz(
                0.0,
                FRAC_PI_2 * (1.0 - self.presentation_angle / 180.0).powi(3),
                0.0,
            )),
        }
    }

    pub fn set_screen(&mut self, next: Option<DeviceScreenConfig>, now: f64) {
        let was_moving = self.moving() || self.motion.needs_frame();
        if duo_display_key(self.screen.as_ref()) != duo_display_key(next.as_ref()) {
            self.ready_key = None;
            self.captured = None;
        }
        let previous = self.screen.take();
        let id = |screen: &Option<DeviceScreenConfig>| screen.as_ref().and_then(|s| s.screen_id);
        let pose = |screen: &Option<DeviceScreenConfig>| screen.as_ref().and_then(|s| s.hinge_pose);
        let hinge =
            |screen: &Option<DeviceScreenConfig>| screen.as_ref().and_then(|s| s.hinge_angle);
        let turned = |screen: &Option<DeviceScreenConfig>| screen.as_ref().map(|s| s.orientation);
        let owned_handoff = self.handoff.is_some();
        if next.is_none()
            || self
                .handoff
                .is_some_and(|handoff| Some(handoff.panel as u32) == id(&next))
        {
            self.handoff = None;
        }
        let owned_rotation = self.requested_orientation.is_some() && id(&next) == id(&previous);
        let changed_display = id(&next) != id(&previous);
        self.requested_orientation = None;
        let changed_pose = pose(&next).is_some() && pose(&next) != pose(&previous);
        let folding = self.held_leaf.is_some() && !changed_pose;
        let rotated = id(&next) == id(&previous)
            && hinge(&next) == hinge(&previous)
            && turned(&next) != turned(&previous);
        let unowned = rotated && !owned_rotation && !owned_handoff && !folding;
        let cover = id(&next) == Some(1);
        if self.first_pose
            || changed_pose
            || (unowned && pose(&next).is_none() && self.preview.is_none())
        {
            self.physical_pose = pose(&next).unwrap_or(if cover {
                HingePose::Closed
            } else {
                HingePose::Open
            });
            self.presentation_angle =
                hinge(&next).map_or(if cover { 0.0 } else { 180.0 }, f64::from);
        }
        self.target_angle = self.preview.unwrap_or_else(|| default_angle(next.as_ref()));
        self.screen = next;
        if changed_display {
            self.view_orientation = turned(&self.screen);
        }
        if self.first_pose || changed_pose || unowned {
            self.held_leaf = None;
            self.view_orientation = turned(&self.screen);
            if self.first_pose {
                self.angle = self.target_angle;
            }
            let mut target = self.presentation(self.screen.as_ref());
            if cover
                && let Some(snap) =
                    nearest_duo_view(target, &self.snaps_for(DuoPanel::Cover, self.target_angle))
            {
                target = snap.rotation;
            }
            self.target_presentation = target;
            self.rest_face = if cover {
                DuoFace::Cover
            } else if self.physical_pose == HingePose::Laptop {
                DuoFace::Right
            } else {
                DuoFace::Inside
            };
            self.motion.set_pose(target, now, self.first_pose);
            self.first_pose = false;
        } else if self.screen.is_some() && changed_display && !owned_handoff && !folding {
            if let Some(snap) = nearest_duo_view(self.motion.rotation, &self.active_snaps()) {
                self.rest_face = snap.face;
                self.motion.set_pose(snap.rotation, now, false);
            }
        }
        if !was_moving {
            self.last_time = now;
        }
        self.apply_pose(now);
    }

    fn set_hinge_preview(&mut self, next: Option<f64>, now: f64) {
        if next.is_some_and(|angle| !(angle.is_finite() && (0.0..=180.0).contains(&angle))) {
            return;
        }
        if !self.moving() {
            self.last_time = now;
        }
        self.preview = next;
        if next.is_some() && self.held_leaf.is_none() && self.rig.is_some() {
            self.handoff = None;
            self.requested_orientation = None;
            self.held_leaf = Some(self.hold_rule());
            self.motion.set_pose(self.motion.rotation, now, true);
        }
        self.hold_motion(now);
        self.target_angle = next.unwrap_or_else(|| default_angle(self.screen.as_ref()));
        if let Some(angle) = next {
            self.angle = angle;
            self.apply_pose(now);
        }
        self.captured = None;
    }

    pub fn set_control(&mut self, queued: Option<f64>, failed: bool, now: f64) {
        self.queued = queued;
        if failed {
            self.pinch.end();
            self.set_hinge_preview(queued, now);
            self.reject_orientation(now);
        } else if !self.pinch.active() {
            self.set_hinge_preview(queued, now);
        }
    }

    pub fn orientation_accepted(&mut self, orientation: DeviceOrientation) {
        if self.requested_orientation == Some(orientation)
            && self
                .screen
                .as_ref()
                .is_some_and(|screen| screen.orientation == orientation)
        {
            self.requested_orientation = None;
        }
    }

    pub fn reject_orientation(&mut self, now: f64) {
        if self.requested_orientation.is_none() && self.handoff.is_none() {
            return;
        }
        self.handoff = None;
        self.requested_orientation = None;
        let orientation = self.screen.as_ref().map(|screen| screen.orientation);
        self.view_orientation = orientation;
        let confirmed: Vec<_> = self
            .active_snaps()
            .into_iter()
            .filter(|snap| Some(snap.orientation) == orientation)
            .collect();
        if let Some(snap) = nearest_duo_view(self.motion.rotation, &confirmed) {
            self.rest_face = snap.face;
            self.motion.set_pose(snap.rotation, now, false);
        }
    }

    pub fn fit(&mut self, viewport: (f64, f64, f64, f64)) -> Option<Camera> {
        self.viewport = Some(viewport);
        self.camera().map(|(camera, _)| camera)
    }

    fn camera(&self) -> Option<(Camera, Fold)> {
        let (rig, (_, _, width, height)) = (self.rig.as_ref()?, self.viewport?);
        if width <= 0.0 || height <= 0.0 {
            return None;
        }
        let fold = rig.fold(self.angle);
        Some((
            Camera::fit(fold.bounds, self.motion.rotation, width / height),
            fold,
        ))
    }

    fn normalized(&self, x: f64, y: f64) -> Option<(f64, f64)> {
        let (left, top, width, height) = self.viewport?;
        (width > 0.0 && height > 0.0).then(|| ((x - left) / width, (y - top) / height))
    }

    pub fn matrices(&self) -> Option<[(Range<usize>, [[f32; 4]; 4], [[f32; 4]; 4]); 2]> {
        let (camera, fold) = self.camera()?;
        let rig = self.rig.as_ref()?;
        Some(
            [
                (Leaf::Left, rig.left.clone()),
                (Leaf::Right, rig.right.clone()),
            ]
            .map(|(leaf, range)| {
                let rigid = fold.leaves[leaf as usize].turned(self.motion.rotation);
                let (mvp, rotation) = camera.matrices_for(rigid.matrix(), rigid.rotation.matrix());
                (range, mvp, rotation)
            }),
        )
    }

    pub fn position_of(&self, panel: DuoPanel, raw: (f32, f32)) -> Option<(f64, f64)> {
        let (camera, fold) = self.camera()?;
        let (left, top, width, height) = self.viewport?;
        let world = self
            .rig
            .as_ref()?
            .position_of(&fold, self.motion.rotation, panel, raw);
        let (x, y) = camera.project(Quat::IDENTITY, world);
        Some((left + x * width, top + y * height))
    }

    pub fn active_panel(&self) -> DuoPanel {
        self.panel()
    }

    pub fn accepts_touch(&self) -> bool {
        !self.moving()
            && self.preview.is_none()
            && self.requested_orientation.is_none()
            && self.handoff.is_none()
            && self.ready_key.is_some()
            && self.ready_key == duo_display_key(self.screen.as_ref())
    }

    pub fn frame_ready(&mut self, key: Option<DisplayKey>) {
        if key.is_some() && key == duo_display_key(self.screen.as_ref()) {
            self.ready_key = key;
        }
    }

    fn screen_point(&mut self, x: f64, y: f64, captured: bool, now: f64) -> Option<(f32, f32)> {
        if self.moving()
            || self.preview.is_some()
            || self.requested_orientation.is_some()
            || self.handoff.is_some()
        {
            return None;
        }
        self.apply_pose(now);
        let key = duo_display_key(self.screen.as_ref());
        if key.is_none() || self.ready_key != key {
            self.captured = None;
            return None;
        }
        let key = key?;
        let (camera, fold) = self.camera()?;
        let (x, y) = self.normalized(x, y)?;
        let rig = self.rig.clone()?;
        let rotation = self.motion.rotation;
        if captured && let Some(held) = self.captured.filter(|held| held.key == key) {
            let (u, v) = rig.plane_uv(&fold, &camera, rotation, held.display, x, y)?;
            return Some(duo_raw_point(rig.displays[held.display].panel, u, v));
        }
        let hit = rig.hit(&fold, &camera, rotation, x, y)?;
        if Some(hit.panel as u32) != key.0 {
            return None;
        }
        self.captured = Some(Captured {
            display: hit.display,
            key,
        });
        Some(duo_raw_point(hit.panel, hit.uv.0, hit.uv.1))
    }

    fn apply(&mut self, actions: Vec<Action>, now: f64) -> Vec<(TouchPhase, (f32, f32))> {
        let mut touches = Vec::new();
        for action in actions {
            match action {
                Action::Touch(phase, point) => touches.push((phase, point)),
                Action::Orbit(x, y) => {
                    self.held_leaf = None;
                    self.motion.orbit(x, y, now);
                }
                Action::Active(active, Mode::Touch) => {
                    self.touch_active = active;
                    self.hold_motion(now);
                }
                Action::Active(active, Mode::Orbit) => {
                    if active {
                        self.held_leaf = None;
                    }
                    self.release(now, |motion| motion.drag_active(active, now));
                }
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
            self.screen_point(x, y, false, now)
        };
        let actions = self.interaction.begin((x, y), screen);
        self.apply(actions, now)
    }

    pub fn drag(&mut self, x: f64, y: f64, now: f64) -> Vec<(TouchPhase, (f32, f32))> {
        let screen = match self.interaction.mode() {
            Some(Mode::Touch) => self.screen_point(x, y, true, now),
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

    fn begin_hinge(&mut self, x: f64, y: f64, now: f64) -> bool {
        if self.screen.is_none() {
            return false;
        }
        self.apply_pose(now);
        let (Some((camera, fold)), Some((x, y)), Some(rig)) =
            (self.camera(), self.normalized(x, y), self.rig.clone())
        else {
            return false;
        };
        if rig
            .leaf_at(&fold, &camera, self.motion.rotation, x, y)
            .is_none()
        {
            return false;
        }
        self.handoff = None;
        self.requested_orientation = None;
        self.held_leaf = Some(self.hold_rule());
        self.motion.set_pose(self.motion.rotation, now, true);
        self.captured = None;
        true
    }

    pub fn pinch_begin(&mut self, x: f64, y: f64, now: f64) -> bool {
        if self.pinch.active() {
            return true;
        }
        let contains = self.begin_hinge(x, y, now);
        let angle = self
            .queued
            .unwrap_or_else(|| default_angle(self.screen.as_ref()));
        self.pinch.begin(contains, angle)
    }

    pub fn pinch_move(&mut self, log_scale: f64, now: f64) {
        if let Some(angle) = self.pinch.moved(log_scale) {
            self.set_hinge_preview(Some(angle), now);
            self.requests.push(HingeCommand::Angle(angle));
        }
    }

    pub fn pinch_end(&mut self, now: f64) {
        if self.pinch.end() {
            self.set_hinge_preview(self.queued, now);
        }
    }

    pub fn pinching(&self) -> bool {
        self.pinch.active()
    }

    pub fn restore(&mut self, now: f64) {
        self.held_leaf = None;
        self.requested_orientation = None;
        self.handoff = None;
        let orientation = self.screen.as_ref().map(|screen| screen.orientation);
        let snap = self
            .active_snaps()
            .into_iter()
            .find(|snap| Some(snap.orientation) == orientation);
        if let Some(snap) = snap
            && Some(snap.orientation) != self.view_orientation
        {
            self.view_orientation = Some(snap.orientation);
            self.requested_orientation = Some(snap.orientation);
            self.requests
                .push(HingeCommand::Orientation(snap.orientation));
        }
        let target = snap.map_or(self.target_presentation, |snap| snap.rotation);
        self.motion.reset(target, now);
        self.apply_pose(now);
    }

    pub fn advance(&mut self, now: f64, reduced: bool) -> bool {
        let mut changed = false;
        if let Some(handoff) = self.handoff
            && now >= handoff.deadline
        {
            self.handoff = None;
            if let Some(snap) = nearest_duo_view(self.motion.rotation, &self.active_snaps()) {
                self.rest_face = snap.face;
                self.motion.set_pose(snap.rotation, now, false);
            }
            changed = true;
        }
        let elapsed = (now - self.last_time) / 1000.0;
        let elapsed = if elapsed == 0.0 || elapsed.is_nan() {
            0.016
        } else {
            elapsed
        };
        let amount = if reduced {
            1.0
        } else {
            1.0 - (-14.0 * elapsed.max(0.0)).exp()
        };
        self.last_time = now;
        let folding = self.moving();
        let orbiting = self.motion.advance(now, reduced);
        if folding {
            self.angle += (self.target_angle - self.angle) * amount;
            if (self.angle - self.target_angle).abs() <= 0.01 {
                self.angle = self.target_angle;
            }
            self.apply_pose(now);
        }
        if orbiting {
            self.apply_pose(now);
        }
        changed || folding || orbiting
    }

    pub fn needs_frame(&self) -> bool {
        self.moving() || self.motion.needs_frame()
    }

    pub fn handoff_deadline(&self) -> Option<f64> {
        self.handoff.map(|handoff| handoff.deadline)
    }

    pub fn take_requests(&mut self) -> Vec<HingeCommand> {
        std::mem::take(&mut self.requests)
    }
}

fn sane(size: (u32, u32), fallback: (u32, u32)) -> (u32, u32) {
    if size.0 > 0 && size.1 > 0 {
        size
    } else {
        fallback
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::device_panel::geometry;

    fn turn(x: f64, y: f64, z: f64) -> Quat {
        let v = Vec3::new(x, y, z);
        Quat::axis_angle(v, v.length())
    }

    fn yaw(angle: f64) -> Quat {
        Quat::axis_angle(Y, angle)
    }

    fn rig() -> DuoRig {
        DuoRig::new(COVER_SIZE, INNER_SIZE)
    }

    fn camera(fold: &Fold, root: Quat) -> Camera {
        Camera::fit(fold.bounds, root, 1.0)
    }

    fn project(camera: &Camera, world: Vec3) -> (f64, f64) {
        camera.project(Quat::IDENTITY, world)
    }

    fn inner_u(rig: &DuoRig, range: Range<usize>) -> Vec<f64> {
        rig.vertices[range]
            .iter()
            .filter(|vertex| vertex.screen == 3.0)
            .map(|vertex| 1.0 - f64::from(vertex.uv[1]))
            .collect()
    }

    #[test]
    fn keeps_one_continuous_inner_uv_map_across_the_hinge_and_folds_opposite_leaves() {
        assert!(leaf_rotation(Leaf::Left, 90.0).angle_to(yaw(PI / 4.0)) < 1e-9);
        assert!(leaf_rotation(Leaf::Right, 90.0).angle_to(yaw(-PI / 4.0)) < 1e-9);
        let rig = rig();
        let (left, right) = (
            inner_u(&rig, rig.left.clone()),
            inner_u(&rig, rig.right.clone()),
        );
        let max = |values: &[f64]| values.iter().copied().fold(f64::NEG_INFINITY, f64::max);
        let min = |values: &[f64]| values.iter().copied().fold(f64::INFINITY, f64::min);
        assert!((min(&left) - 0.0).abs() < 1e-6 && (max(&left) - 0.5).abs() < 1e-6);
        assert!((min(&right) - 0.5).abs() < 1e-6 && (max(&right) - 1.0).abs() < 1e-6);
        assert!(
            rig.vertices[rig.right.clone()]
                .iter()
                .all(|vertex| vertex.screen != 1.0),
            "the cover lives on the left leaf"
        );
    }

    #[test]
    fn derives_rest_frames_from_the_hinged_display_planes_independently_of_the_orbit() {
        let rig = rig();
        let frames = rig.rest_frames(90.0, DuoPanel::Inner);
        assert_eq!(
            frames.iter().map(|frame| frame.face).collect::<Vec<_>>(),
            [DuoFace::Inside, DuoFace::Left, DuoFace::Right]
        );
        assert!(frames[1].normal.x > 0.5);
        assert!(frames[2].normal.x < -0.5);
        assert!(frames[0].normal.z > 0.99);
        assert_eq!(
            rig.rest_frames(180.0, DuoPanel::Inner)
                .iter()
                .map(|frame| frame.face)
                .collect::<Vec<_>>(),
            [DuoFace::Inside]
        );
        assert_eq!(
            rig.rest_frames(180.0, DuoPanel::Cover)
                .iter()
                .map(|frame| frame.face)
                .collect::<Vec<_>>(),
            [DuoFace::Cover]
        );
    }

    #[test]
    fn maps_active_display_input_through_hardware_mounting_and_lets_the_chassis_hide_rear_displays()
    {
        let rig = rig();
        let fold = rig.fold(180.0);
        let camera = camera(&fold, Quat::IDENTITY);
        let local = rig.displays[1].local(0.25, 0.75);
        let (x, y) = project(&camera, fold.leaves[0].apply(local));
        let hit = rig.hit(&fold, &camera, Quat::IDENTITY, x, y).unwrap();
        assert_eq!(hit.panel, DuoPanel::Inner);
        let raw = duo_raw_point(hit.panel, hit.uv.0, hit.uv.1);
        assert!(
            (raw.0 - 0.75).abs() < 1e-4 && (raw.1 - 0.75).abs() < 1e-4,
            "{raw:?}"
        );
        assert_eq!(duo_raw_point(DuoPanel::Cover, 0.2, 0.7), (0.2, 0.7));

        let behind = yaw(PI);
        let camera = Camera::fit(fold.bounds, behind, 1.0);
        let (x, y) = project(&camera, fold.leaves[0].turned(behind).apply(local));
        assert!(
            rig.hit(&fold, &camera, behind, x, y)
                .is_none_or(|hit| hit.panel == DuoPanel::Cover),
            "the inner display cannot be touched through the back"
        );
        assert_eq!(rig.hit(&fold, &camera, behind, 0.01, 0.01), None);
    }

    #[test]
    fn chooses_a_leaf_focused_seated_view_when_it_is_closer_than_the_middle_of_the_fold() {
        let inside = DuoRestFrame {
            face: DuoFace::Inside,
            normal: Vec3::new(0.0, 0.0, 1.0),
            up: Y,
            center: Vec3::ZERO,
        };
        let leaf = DuoRestFrame {
            face: DuoFace::Right,
            normal: Vec3::new(0.6, 0.0, 0.8),
            up: Y,
            center: Vec3::new(1.0, 0.0, 0.0),
        };
        let candidates = duo_view_snaps(&[inside, leaf], DuoPanel::Inner);
        let seat = candidates
            .iter()
            .find(|candidate| {
                candidate.face == DuoFace::Right
                    && candidate.orientation == DeviceOrientation::Portrait
            })
            .unwrap();
        let released = turn(0.04, 0.05, 0.0).mul(seat.rotation);
        let chosen = nearest_duo_view(released, &candidates).unwrap();
        assert_eq!(chosen.face, DuoFace::Right);
        assert_eq!(chosen.orientation, DeviceOrientation::Portrait);
        assert_eq!(chosen.center.x, 1.0);
    }

    #[test]
    fn a_seated_view_exposes_the_base_and_upright_yaw_keeps_both_inner_displays_facing_the_camera()
    {
        let inside = DuoRestFrame {
            face: DuoFace::Inside,
            normal: Vec3::new(0.0, 0.0, 1.0),
            up: Y,
            center: Vec3::ZERO,
        };
        let s = std::f64::consts::FRAC_1_SQRT_2;
        let folded = [
            inside,
            DuoRestFrame {
                face: DuoFace::Left,
                normal: Vec3::new(s, 0.0, s),
                up: Y,
                center: Vec3::new(-1.0, 0.0, 0.0),
            },
            DuoRestFrame {
                face: DuoFace::Right,
                normal: Vec3::new(-s, 0.0, s),
                up: Y,
                center: Vec3::new(1.0, 0.0, 0.0),
            },
        ];
        let candidates = duo_view_snaps(&folded, DuoPanel::Inner);
        assert_eq!(
            candidates
                .iter()
                .filter(|candidate| candidate.face == DuoFace::Right)
                .count(),
            1
        );
        for candidate in &candidates {
            for frame in &folded[1..] {
                assert!(candidate.rotation.rotate(frame.normal).z > 0.2);
            }
            let side = turn(0.0, 1.5, 0.0).mul(candidate.rotation);
            let chosen = nearest_duo_view(side, std::slice::from_ref(candidate)).unwrap();
            for frame in &folded[1..] {
                assert!(chosen.rotation.rotate(frame.normal).z > 0.02);
            }
        }
    }

    #[test]
    fn the_cover_sits_centered_on_its_leaf_and_screens_round_only_their_outer_corners() {
        let rig = rig();
        let [cover, left, right] = rig.displays;
        let (body_lo, body_hi) = rig.leaf_boxes[0];
        let sides = (cover.x.0 - body_lo.x, body_hi.x - cover.x.1);
        let ends = (cover.y.0 - body_lo.y, body_hi.y - cover.y.1);
        assert!((sides.0 - sides.1).abs() < 1e-9, "{sides:?}");
        assert!((ends.0 - ends.1).abs() < 1e-9, "{ends:?}");
        for margin in [sides.0, ends.0] {
            assert!(margin >= BEZEL - 1e-9 && margin < BEZEL + 0.01, "{margin}");
        }
        let has = |range: &Range<usize>, tag: DuoPanel, x: f64, y: f64| {
            rig.vertices[range.clone()].iter().any(|vertex| {
                vertex.screen == tag as u32 as f32
                    && (f64::from(vertex.position[0]) - x).abs() < 1e-6
                    && (f64::from(vertex.position[1]) - y).abs() < 1e-6
            })
        };
        for (x, y) in [
            (cover.x.0, cover.y.0),
            (cover.x.1, cover.y.0),
            (cover.x.0, cover.y.1),
            (cover.x.1, cover.y.1),
        ] {
            assert!(
                !has(&rig.left, DuoPanel::Cover, x, y),
                "cover corner rounded"
            );
        }
        for y in [left.y.0, left.y.1] {
            assert!(
                !has(&rig.left, DuoPanel::Inner, left.x.0, y),
                "outer corner rounded"
            );
            assert!(
                has(&rig.left, DuoPanel::Inner, left.x.1, y),
                "hinge corner square"
            );
            assert!(
                !has(&rig.right, DuoPanel::Inner, right.x.1, y),
                "outer corner rounded"
            );
            assert!(
                has(&rig.right, DuoPanel::Inner, right.x.0, y),
                "hinge corner square"
            );
        }
    }

    #[test]
    fn builds_the_rig_from_hub_reported_aspects_and_closes_into_two_leaf_thicknesses() {
        let rig = rig();
        let [cover, left, right] = rig.displays;
        let width = right.x.1 - left.x.0 - 2.0 * HINGE_INSET;
        let height = left.y.1 - left.y.0;
        assert!((height - SCREEN_HEIGHT).abs() < 1e-12);
        assert!((width / height - 2852.0 / 2006.0).abs() < 1e-9);
        let cover_aspect = (cover.x.1 - cover.x.0) / (cover.y.1 - cover.y.0);
        assert!((cover_aspect - 1398.0 / 2034.0).abs() < 1e-9);
        assert!(cover.x.0 > rig.leaf_boxes[0].0.x && cover.x.1 < 0.0);
        let closed = rig.fold(0.0).bounds;
        let thickness = closed.1.x - closed.0.x;
        let expected = 2.0 * LEAF_DEPTH + 2.0 * (SCREEN_LIFT + PIVOT_GAP);
        assert!(
            (thickness - expected).abs() < 1e-9,
            "{thickness} vs {expected}"
        );
        let open = rig.fold(180.0).bounds;
        assert!(
            ((open.0 + open.1) * 0.5).length() < 1e-9,
            "the orbit pivots on the body"
        );
        assert!(
            rig.vertices
                .iter()
                .all(|v| v.position.iter().all(|c| c.is_finite()))
        );
        let other = DuoRig::new((0, 0), INNER_SIZE);
        assert_eq!(
            other.sizes, rig.sizes,
            "unknown sizes fall back to the hub fixtures"
        );
        assert_ne!(other.id, rig.id);
    }

    #[test]
    fn the_texel_drawn_at_a_panel_point_is_the_one_the_flat_view_shows_and_stands_upright() {
        let rig = rig();
        let cover_snaps = duo_view_snaps(&rig.rest_frames(0.0, DuoPanel::Cover), DuoPanel::Cover);
        let cover_front = cover_snaps
            .iter()
            .find(|snap| snap.orientation == DeviceOrientation::Portrait)
            .unwrap()
            .rotation;
        for (panel, angle, rotation, root) in [
            (
                DuoPanel::Inner,
                180.0,
                DeviceOrientation::LandscapeLeft,
                Quat::IDENTITY,
            ),
            (
                DuoPanel::Inner,
                180.0,
                DeviceOrientation::Portrait,
                Quat::axis_angle(Vec3::new(0.0, 0.0, 1.0), FRAC_PI_2),
            ),
            (
                DuoPanel::Cover,
                0.0,
                DeviceOrientation::Portrait,
                cover_front,
            ),
        ] {
            let fold = rig.fold(angle);
            let camera = camera(&fold, root);
            let tag = panel as u32 as f32;
            let mut checked = 0;
            for (index, quad) in rig.displays.iter().enumerate() {
                if quad.panel != panel {
                    continue;
                }
                let range = match quad.leaf {
                    Leaf::Left => rig.left.clone(),
                    Leaf::Right => rig.right.clone(),
                };
                let middle = Vec3::new(
                    (quad.x.0 + quad.x.1) / 2.0,
                    (quad.y.0 + quad.y.1) / 2.0,
                    quad.z,
                );
                for vertex in rig.vertices[range]
                    .iter()
                    .filter(|vertex| vertex.screen == tag)
                {
                    let [x, y, z] = vertex.position.map(f64::from);
                    let at = Vec3::new(x, y, z);
                    let baked = quad.panel_uv(at.x, at.y);
                    let baked = duo_raw_point(panel, baked.0, baked.1);
                    assert!(
                        (baked.0 - vertex.uv[0]).abs() < 1e-6
                            && (baked.1 - vertex.uv[1]).abs() < 1e-6
                    );
                    let inset = middle + (at - middle) * 0.98;
                    let expected = quad.panel_uv(inset.x, inset.y);
                    let expected = duo_raw_point(panel, expected.0, expected.1);
                    let world = fold.leaves[quad.leaf as usize].turned(root).apply(inset);
                    let (sx, sy) = project(&camera, world);
                    let hit = rig.hit(&fold, &camera, root, sx, sy).unwrap();
                    assert_eq!((hit.panel, hit.display), (panel, index));
                    let touched = duo_raw_point(hit.panel, hit.uv.0, hit.uv.1);
                    assert!(
                        (touched.0 - expected.0).abs() < 1e-4
                            && (touched.1 - expected.1).abs() < 1e-4,
                        "{panel:?} {rotation:?}: {touched:?} vs {expected:?}"
                    );
                    let texel = geometry::display_point(rotation, touched.0, touched.1);
                    let flat = geometry::raw_point(rotation, texel.0, texel.1);
                    assert!((flat.0 - touched.0).abs() < 1e-6 && (flat.1 - touched.1).abs() < 1e-6);
                    checked += 1;
                }
            }
            assert!(checked >= 4, "{panel:?}");
            let shown = |texel: (f32, f32)| {
                let raw = geometry::raw_point(rotation, texel.0, texel.1);
                project(&camera, rig.position_of(&fold, root, panel, raw))
            };
            let (left, right) = (shown((0.3, 0.5)), shown((0.7, 0.5)));
            let (top, bottom) = (shown((0.5, 0.3)), shown((0.5, 0.7)));
            assert!(
                left.0 < right.0 && (left.1 - right.1).abs() < 1e-3,
                "{panel:?} {rotation:?}: {left:?} {right:?}"
            );
            assert!(
                top.1 < bottom.1 && (top.0 - bottom.0).abs() < 1e-3,
                "{panel:?} {rotation:?}: {top:?} {bottom:?}"
            );
        }
    }

    #[test]
    fn a_projected_raw_point_ray_casts_back_to_itself_on_each_leaf_and_the_cover() {
        let rig = rig();
        for angle in [45.0, 90.0, 135.0] {
            let fold = rig.fold(angle);
            for (panel, raw, facing) in [
                (
                    DuoPanel::Inner,
                    (0.4f32, 0.8f32),
                    leaf_rotation(Leaf::Left, angle).inverse(),
                ),
                (
                    DuoPanel::Inner,
                    (0.6, 0.2),
                    leaf_rotation(Leaf::Right, angle).inverse(),
                ),
                (
                    DuoPanel::Cover,
                    (0.3, 0.6),
                    yaw(PI).mul(leaf_rotation(Leaf::Left, angle).inverse()),
                ),
            ] {
                let mut landed = 0;
                for side in [-1.0, 1.0] {
                    let root = yaw(side * PI / 6.0).mul(facing);
                    let camera = camera(&fold, root);
                    let (x, y) = project(&camera, rig.position_of(&fold, root, panel, raw));
                    let Some(hit) = rig.hit(&fold, &camera, root, x, y) else {
                        continue;
                    };
                    if hit.panel != panel {
                        continue;
                    }
                    let back = duo_raw_point(hit.panel, hit.uv.0, hit.uv.1);
                    assert!(
                        (back.0 - raw.0).abs() < 1e-4 && (back.1 - raw.1).abs() < 1e-4,
                        "{angle} {panel:?} {side}: {back:?} vs {raw:?}"
                    );
                    landed += 1;
                }
                assert!(landed >= 1, "{angle} {panel:?} {raw:?} was never reachable");
            }
        }
    }

    #[test]
    fn a_frame_matches_a_display_by_its_raw_shape_only() {
        let landscape = duo_raw_size((2852, 2006), DeviceOrientation::LandscapeLeft);
        assert_eq!(landscape, (2006, 2852));
        assert!(duo_frame_matches(landscape, INNER_SIZE));
        assert!(duo_frame_matches(
            duo_raw_size((2006, 2852), DeviceOrientation::Portrait),
            (2007, 2853)
        ));
        assert!(!duo_frame_matches((2852, 2006), INNER_SIZE));
        assert!(!duo_frame_matches(COVER_SIZE, INNER_SIZE));
        assert!(duo_frame_matches(COVER_SIZE, COVER_SIZE));
        assert!(!duo_frame_matches((0, 10), COVER_SIZE));
    }

    const VIEWPORT: (f64, f64, f64, f64) = (100.0, 50.0, 500.0, 700.0);

    fn inner_screen(angle: u8, pose: Option<HingePose>) -> DeviceScreenConfig {
        DeviceScreenConfig {
            width: 2007,
            height: 2853,
            orientation: DeviceOrientation::Portrait,
            screen_id: Some(3),
            supports_hinge_angle: Some(true),
            supports_physical_orientation: Some(true),
            hinge_angle: Some(angle),
            hinge_pose: pose,
        }
    }

    fn cover_screen(angle: u8, pose: Option<HingePose>) -> DeviceScreenConfig {
        DeviceScreenConfig {
            width: 1398,
            height: 2034,
            screen_id: Some(1),
            ..inner_screen(angle, pose)
        }
    }

    fn viewer(screen: DeviceScreenConfig) -> DuoViewer {
        let mut viewer = DuoViewer::default();
        viewer.set_rig(COVER_SIZE, INNER_SIZE);
        viewer.fit(VIEWPORT);
        viewer.set_screen(Some(screen), 0.0);
        viewer.advance(0.0, true);
        viewer
    }

    fn ready(viewer: &mut DuoViewer) {
        let key = duo_display_key(viewer.screen());
        viewer.frame_ready(key);
    }

    fn settle(viewer: &mut DuoViewer, from: f64) -> f64 {
        let mut now = from;
        while viewer.needs_frame() && now < from + 10_000.0 {
            now += 16.0;
            viewer.advance(now, false);
        }
        now
    }

    #[test]
    fn pinches_only_a_hit_device_accumulates_independently_of_readback_clamps_and_cancels() {
        let mut pinch = DuoPinch::default();
        assert!(!pinch.begin(false, 90.0));
        assert_eq!(pinch.moved(1.0), None);
        assert!(pinch.begin(true, 90.0));
        assert_eq!(pinch.moved(0.25), Some(120.0));
        assert_eq!(pinch.moved(0.25), Some(150.0));
        assert_eq!(pinch.moved(2.0), Some(180.0));
        assert_eq!(pinch.moved(2.0), None, "already at the stop");
        assert_eq!(pinch.moved(-3.0), Some(0.0));
        assert_eq!(pinch.moved(f64::NAN), None);
        assert!(pinch.end());
        assert_eq!(pinch.moved(1.0), None);
        assert!(!pinch.end());
        assert!(!pinch.active());
        let step = pinch_log_scale(0.1).unwrap();
        assert!((step - 1.1f64.ln()).abs() < 1e-6, "{step}");
        assert_eq!(pinch_log_scale(-1.0), None);
        assert_eq!(pinch_log_scale(f32::NAN), None);
    }

    #[test]
    fn the_rig_is_rebuilt_only_when_a_display_size_changes() {
        let mut viewer = DuoViewer::default();
        let first = viewer.set_rig(COVER_SIZE, INNER_SIZE).id;
        assert_eq!(viewer.set_rig(COVER_SIZE, INNER_SIZE).id, first);
        assert_eq!(viewer.set_rig((0, 0), INNER_SIZE).id, first);
        assert_ne!(viewer.set_rig(COVER_SIZE, (2007, 2853)).id, first);
    }

    #[test]
    fn touches_only_the_active_display_and_only_once_its_frame_was_painted() {
        let inner = inner_screen(180, Some(HingePose::Open));
        let mut viewer = viewer(inner.clone());
        let (x, y) = viewer.position_of(DuoPanel::Inner, (0.75, 0.75)).unwrap();
        assert!(
            viewer.press(x, y, false, 0.0).is_empty(),
            "no painted frame yet"
        );
        viewer.lift(0.0);
        ready(&mut viewer);
        let touches = viewer.press(x, y, false, 0.0);
        assert!(
            matches!(touches[..], [(TouchPhase::Begin, (u, v))] if (u - 0.75).abs() < 1e-3 && (v - 0.75).abs() < 1e-3),
            "{touches:?}"
        );
        let moved = viewer.drag(x + 10_000.0, y, 16.0);
        assert!(
            matches!(moved[..], [(TouchPhase::Move, (u, v))] if (0.0..=1.0).contains(&u) && (0.0..=1.0).contains(&v)),
            "{moved:?}"
        );
        assert!(matches!(viewer.lift(32.0)[..], [(TouchPhase::End, _)]));

        let stale = duo_display_key(Some(&inner_screen(90, None)));
        viewer.set_screen(Some(cover_screen(180, Some(HingePose::Open))), 40.0);
        viewer.frame_ready(stale);
        assert!(
            !viewer.accepts_touch(),
            "a stale frame key never opens touch"
        );
        ready(&mut viewer);
        assert!(
            viewer.press(x, y, false, 40.0).is_empty(),
            "the inner display is not the elected one"
        );
        viewer.lift(40.0);
    }

    #[test]
    fn snaps_onto_the_opposite_screen_requests_the_handoff_once_and_keeps_the_view_on_readback() {
        let inner = inner_screen(90, None);
        let mut viewer = viewer(inner.clone());
        ready(&mut viewer);
        assert!(viewer.accepts_touch());
        let half_turn = PI / 3.0 * VIEWPORT.2;
        assert!(viewer.wheel(half_turn, 0.0, 10.0));
        viewer.end_wheel(10.0);
        viewer.advance(20.0, true);
        assert_eq!(
            viewer.take_requests(),
            [HingeCommand::Physical(DeviceFacing::FaceDown)]
        );
        assert!(
            !viewer.accepts_touch(),
            "no touch until the hub elects the cover"
        );
        let chosen = viewer.motion.rotation;
        viewer.set_screen(Some(inner.clone()), 30.0);
        viewer.advance(40.0, true);
        assert!(viewer.motion.rotation.angle_to(chosen) < 1e-6);
        viewer.set_screen(Some(cover_screen(90, None)), 50.0);
        viewer.advance(60.0, true);
        assert!(viewer.motion.rotation.angle_to(chosen) < 1e-6);
        assert_eq!(viewer.handoff_deadline(), None, "the cover confirmed");
        viewer.advance(6000.0, true);
        assert!(viewer.take_requests().is_empty());
        assert!(viewer.motion.rotation.angle_to(chosen) < 1e-6);

        assert!(viewer.wheel(-half_turn, 0.0, 6010.0));
        viewer.end_wheel(6010.0);
        viewer.advance(6020.0, true);
        assert_eq!(
            viewer.take_requests(),
            [HingeCommand::Physical(DeviceFacing::FaceUp)]
        );
        let reverse = viewer.motion.rotation;
        viewer.set_screen(Some(inner), 6030.0);
        viewer.advance(6040.0, true);
        assert!(viewer.motion.rotation.angle_to(reverse) < 1e-6);
        viewer.advance(12_000.0, true);
        assert!(viewer.take_requests().is_empty());
        assert!(!viewer.needs_frame());
    }

    #[test]
    fn restore_after_a_handoff_asks_for_no_orientation_the_new_display_already_has() {
        let mut viewer = viewer(inner_screen(90, None));
        ready(&mut viewer);
        assert!(viewer.wheel(PI / 3.0 * VIEWPORT.2, 0.0, 10.0));
        viewer.end_wheel(10.0);
        viewer.advance(20.0, true);
        assert_eq!(
            viewer.take_requests(),
            [HingeCommand::Physical(DeviceFacing::FaceDown)]
        );
        let cover = DeviceScreenConfig {
            orientation: DeviceOrientation::PortraitUpsideDown,
            ..cover_screen(90, None)
        };
        viewer.set_screen(Some(cover), 30.0);
        ready(&mut viewer);
        viewer.restore(40.0);
        settle(&mut viewer, 40.0);
        assert_eq!(viewer.take_requests(), Vec::<HingeCommand>::new());
        assert!(viewer.accepts_touch());
    }

    #[test]
    fn rolls_an_unconfirmed_opposite_screen_snap_back_to_the_active_screen_after_the_timeout() {
        let mut viewer = viewer(inner_screen(90, None));
        assert!(viewer.wheel(PI / 3.0 * VIEWPORT.2, 0.0, 10.0));
        viewer.end_wheel(10.0);
        viewer.advance(20.0, true);
        let unconfirmed = viewer.motion.rotation;
        assert_eq!(
            viewer.take_requests(),
            [HingeCommand::Physical(DeviceFacing::FaceDown)]
        );
        assert_eq!(viewer.handoff_deadline(), Some(10.0 + HANDOFF_MS));
        viewer.advance(4000.0, true);
        assert!(viewer.motion.rotation.angle_to(unconfirmed) < 1e-6);
        viewer.advance(5010.0, true);
        assert!(viewer.motion.rotation.angle_to(unconfirmed) > 0.5);
        assert!(viewer.take_requests().is_empty(), "no second request");
        assert!(!viewer.needs_frame());
    }

    #[test]
    fn a_failed_or_timed_out_command_drops_the_preview_and_snaps_back() {
        let mut viewer = viewer(inner_screen(90, None));
        assert!(viewer.wheel(PI / 3.0 * VIEWPORT.2, 0.0, 10.0));
        viewer.end_wheel(10.0);
        viewer.advance(20.0, true);
        let unconfirmed = viewer.motion.rotation;
        viewer.take_requests();
        viewer.set_control(None, true, 30.0);
        viewer.advance(40.0, true);
        assert_eq!(viewer.handoff_deadline(), None);
        assert!(viewer.motion.rotation.angle_to(unconfirmed) > 0.5);

        let (x, y) = viewer.position_of(DuoPanel::Inner, (0.5, 0.25)).unwrap();
        assert!(viewer.pinch_begin(x, y, 50.0));
        viewer.pinch_move(-0.25, 50.0);
        assert_eq!(viewer.take_requests(), [HingeCommand::Angle(60.0)]);
        viewer.set_control(Some(60.0), false, 60.0);
        assert!(
            viewer.pinching(),
            "pushes while pinching keep the accumulator"
        );
        viewer.set_control(None, true, 70.0);
        assert!(!viewer.pinching());
        viewer.advance(80.0, true);
        assert_eq!(viewer.angle(), 90.0, "back to the hub angle");
        viewer.set_control(None, false, 90.0);
        viewer.advance(100.0, true);
        assert_eq!(viewer.angle(), 90.0);
        assert!(viewer.take_requests().is_empty());
    }

    #[test]
    fn previewing_the_hinge_keeps_the_held_leaf_still_in_camera_space_and_blocks_unconfirmed_touch()
    {
        for start in [(0.5f32, 0.75f32), (0.5, 0.25)] {
            let mut viewer = viewer(inner_screen(180, Some(HingePose::Open)));
            ready(&mut viewer);
            assert!(!viewer.begin_hinge(-1000.0, -1000.0, 0.0));
            let (x, y) = viewer.position_of(DuoPanel::Inner, start).unwrap();
            assert!(viewer.begin_hinge(x, y, 0.0));
            let held = |viewer: &DuoViewer, angle| {
                viewer
                    .motion
                    .rotation
                    .mul(leaf_rotation(Leaf::Right, angle))
            };
            let partner = |viewer: &DuoViewer, angle| {
                viewer.motion.rotation.mul(leaf_rotation(Leaf::Left, angle))
            };
            let orientation = held(&viewer, 180.0);
            let partner_start = partner(&viewer, 180.0);
            let mut now = 0.0;
            for value in [150u8, 90, 30, 0, 30, 90, 180] {
                now += 100.0;
                let angle = f64::from(value);
                viewer.set_hinge_preview(Some(angle), now);
                viewer.advance(now, true);
                assert!(
                    held(&viewer, angle).angle_to(orientation) < 1e-6,
                    "{start:?} {value}"
                );
                assert!(!viewer.accepts_touch());
                if value == 90 {
                    assert!(partner(&viewer, angle).angle_to(partner_start) > 1.0);
                }
                let next = if value == 0 {
                    cover_screen(0, None)
                } else {
                    inner_screen(value, None)
                };
                let changed = duo_display_key(Some(&next)) != duo_display_key(viewer.screen());
                viewer.set_screen(Some(next.clone()), now);
                viewer.set_hinge_preview(None, now);
                viewer.advance(now + 8.0, true);
                viewer.set_screen(Some(next), now + 8.0);
                viewer.advance(now + 16.0, true);
                assert!(
                    held(&viewer, angle).angle_to(orientation) < 1e-6,
                    "{start:?} {value} after readback"
                );
                assert_eq!(
                    viewer.accepts_touch(),
                    !changed,
                    "{start:?} {value}: a new display waits for its frame"
                );
                ready(&mut viewer);
                assert!(viewer.accepts_touch(), "{start:?} {value}");
                if value == 0 {
                    let (x, y) = viewer.position_of(DuoPanel::Cover, (0.5, 0.5)).unwrap();
                    assert!(viewer.begin_hinge(x, y, now + 16.0));
                }
            }
            assert!(viewer.take_requests().is_empty());
            assert!(!viewer.needs_frame());
        }
    }

    #[test]
    fn restore_faces_the_active_display_and_asks_for_an_orientation_only_when_the_view_turned() {
        let cover = cover_screen(0, Some(HingePose::Closed));
        let mut viewer = viewer(cover.clone());
        let normal = |viewer: &DuoViewer| {
            viewer
                .motion
                .rotation
                .rotate(leaf_rotation(Leaf::Left, viewer.angle()).rotate(Vec3::new(0.0, 0.0, -1.0)))
        };
        assert!(normal(&viewer).z > 0.99, "{:?}", normal(&viewer));
        assert!(viewer.wheel(200.0, 0.0, 10.0));
        viewer.end_wheel(10.0);
        let now = settle(&mut viewer, 10.0);
        assert!(normal(&viewer).z < 0.99, "the snap keeps a little yaw");
        assert!(viewer.take_requests().is_empty());
        viewer.restore(now);
        let now = settle(&mut viewer, now);
        assert!(normal(&viewer).z > 0.99);
        assert!(viewer.take_requests().is_empty(), "already upright");

        assert!(viewer.wheel(0.0, PI / 0.006, now));
        viewer.end_wheel(now);
        let now = settle(&mut viewer, now);
        assert_eq!(
            viewer.take_requests(),
            [HingeCommand::Orientation(
                DeviceOrientation::PortraitUpsideDown
            )]
        );
        viewer.set_screen(Some(cover), now);
        viewer.restore(now);
        let now = settle(&mut viewer, now);
        assert_eq!(
            viewer.take_requests(),
            [HingeCommand::Orientation(DeviceOrientation::Portrait)]
        );
        assert!(normal(&viewer).z > 0.99);
        assert!(!viewer.needs_frame(), "{now}");
    }
}
