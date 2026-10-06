use std::f64::consts::{FRAC_PI_2, PI};
use std::ops::Range;

use zeron_proto::device_hub::{DeviceOrientation, DeviceScreenConfig};

use super::scene3d::{self, Builder, Camera, IOS_PHONE, Quat, SCREEN_HEIGHT, Vec3, Vertex};

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
        let sane = |size: (u32, u32), fallback| {
            if size.0 > 0 && size.1 > 0 {
                size
            } else {
                fallback
            }
        };
        let (cover, inner) = (sane(cover, COVER_SIZE), sane(inner, INNER_SIZE));
        let half = SCREEN_HEIGHT * f64::from(inner.1) / f64::from(inner.0) / 2.0;
        let leaf_width = half + HINGE_INSET + BEZEL;
        let leaf_height = SCREEN_HEIGHT + 2.0 * BEZEL;
        let aspect = f64::from(cover.0) / f64::from(cover.1);
        let cover_height = SCREEN_HEIGHT.min((half + HINGE_INSET) / aspect);
        let cover_width = cover_height * aspect;
        let cover_x = -(HINGE_INSET + half / 2.0);
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
                let outline = display.corners().map(|corner| (corner.x, corner.y));
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
}
