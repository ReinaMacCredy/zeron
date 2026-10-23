use std::time::{Duration, Instant};

use gpui::{App, Background, Bounds, Hsla, PathBuilder, Pixels, Point, Window, canvas, point, px};

use crate::theme::Theme;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Lit {
    Off,
    Partial,
    On,
}

impl Lit {
    fn strength(self) -> f32 {
        match self {
            Lit::Off => 0.0,
            Lit::Partial => 0.45,
            Lit::On => 1.0,
        }
    }
}

#[derive(Clone, Copy)]
pub struct Wire {
    lit: Lit,
    from: Lit,
    since: Option<Instant>,
}

impl Default for Wire {
    fn default() -> Self {
        Self {
            lit: Lit::Off,
            from: Lit::Off,
            since: None,
        }
    }
}

impl Wire {
    pub fn set(&mut self, lit: Lit, animate: bool) {
        if lit != self.lit {
            self.from = self.lit;
            self.lit = lit;
            self.since = animate.then(crate::motion::now);
        }
    }

    fn progress(&self) -> f32 {
        self.since.map_or(1.0, |since| {
            (crate::motion::since(since).as_secs_f32() / DRAW.as_secs_f32()).min(1.0)
        })
    }
}

const DRAW: Duration = Duration::from_millis(750);
const WIDTH: f32 = 640.0;
const HEIGHT: f32 = 210.0;
const COS: f32 = 0.866;
const SIN: f32 = 0.5;
const HUB: (f32, f32) = (320.0, 162.0);
const HUB_SIZE: f32 = 42.0;
const HUB_ROUND: f32 = 9.0;
const PORT_Z: [f32; 3] = [87.0, 59.0, 37.0];
const NODES: [(f32, f32); 3] = [(96.0, 40.0), (70.0, 108.0), (112.0, 172.0)];
const NODE_SIZE: f32 = 13.0;
const NODE_ROUND: f32 = 4.0;
const FLOW_SPAN: f32 = 15.0;
const FLOW_STEP: f32 = 1.5;
const WIRE_COLORS: [(u32, u32); 6] = [
    (0x4fe0a8, 0x5aa8ff),
    (0x4dd6ff, 0x5a8cff),
    (0x8be36a, 0x45c8c0),
    (0xc66bff, 0xff5fa2),
    (0xff6fae, 0xff8a4c),
    (0xffb347, 0xff6a6a),
];
const DOT_COLORS: [u32; 6] = [0x5f8cff, 0x6f80ff, 0x8070ff, 0x9466ff, 0xa85fff, 0xb65cff];

pub fn backdrop(theme: &Theme) -> Hsla {
    Hsla { a: 1.0, ..theme.bg }
}

pub fn hero(theme: &Theme, wires: [Wire; 6], glow: f32) -> gpui::Canvas<()> {
    let palette = Palette {
        fill: backdrop(theme),
        edge: theme.text.opacity(0.72),
        faint: theme.text.opacity(0.28),
        warning: theme.warning,
    };
    canvas(
        |_, _, _| (),
        move |bounds, _, window, cx| paint(bounds, wires, glow, palette, window, cx),
    )
}

#[derive(Clone, Copy)]
struct Palette {
    fill: Hsla,
    edge: Hsla,
    faint: Hsla,
    warning: Hsla,
}

type Pt = (f32, f32);

struct Frame {
    origin: Point<Pixels>,
    scale: f32,
}

impl Frame {
    fn at(&self, (x, y): Pt) -> Point<Pixels> {
        self.origin + point(px(x * self.scale), px(y * self.scale))
    }
}

fn rgb(hex: u32) -> Hsla {
    gpui::rgb(hex).into()
}

fn iso(center: Pt, x: f32, y: f32, z: f32) -> Pt {
    (center.0 + (x - y) * COS, center.1 + (x + y) * SIN - z)
}

fn rounded(a: f32, r: f32) -> Vec<Pt> {
    let corners = [
        (a - r, a - r),
        (r - a, a - r),
        (r - a, r - a),
        (a - r, r - a),
    ];
    let mut points = Vec::with_capacity(24);
    for (k, (cx, cy)) in corners.into_iter().enumerate() {
        for i in 0..=5 {
            let t = (90.0 * k as f32 + 18.0 * i as f32).to_radians();
            points.push((cx + r * t.cos(), cy + r * t.sin()));
        }
    }
    points
}

fn hull(mut points: Vec<Pt>) -> Vec<Pt> {
    points.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.total_cmp(&b.1)));
    let cross = |o: Pt, a: Pt, b: Pt| (a.0 - o.0) * (b.1 - o.1) - (a.1 - o.1) * (b.0 - o.0);
    let chain = |iter: &mut dyn Iterator<Item = Pt>| {
        let mut out: Vec<Pt> = Vec::new();
        for p in iter {
            while out.len() > 1 && cross(out[out.len() - 2], out[out.len() - 1], p) <= 0.0 {
                out.pop();
            }
            out.push(p);
        }
        out.pop();
        out
    };
    let mut lower = chain(&mut points.iter().copied());
    lower.extend(chain(&mut points.iter().rev().copied()));
    lower
}

fn polygon(
    frame: &Frame,
    points: &[Pt],
    fill: impl Into<Background>,
    stroke: Hsla,
    window: &mut Window,
) {
    let mut shape = PathBuilder::fill();
    let mut edge = PathBuilder::stroke(px(1.0));
    for (i, &p) in points.iter().enumerate() {
        if i == 0 {
            shape.move_to(frame.at(p));
            edge.move_to(frame.at(p));
        } else {
            shape.line_to(frame.at(p));
            edge.line_to(frame.at(p));
        }
    }
    shape.close();
    edge.close();
    if let Ok(path) = shape.build() {
        window.paint_path(path, fill);
    }
    if let Ok(path) = edge.build() {
        window.paint_path(path, stroke);
    }
}

fn slab(
    frame: &Frame,
    center: Pt,
    (a, r): (f32, f32),
    (z0, z1): (f32, f32),
    side: impl Into<Background>,
    p: &Palette,
    window: &mut Window,
) {
    let outline = rounded(a, r);
    let top: Vec<Pt> = outline
        .iter()
        .map(|&(x, y)| iso(center, x, y, z1))
        .collect();
    let mut both = top.clone();
    both.extend(outline.iter().map(|&(x, y)| iso(center, x, y, z0)));
    polygon(frame, &hull(both), side, p.edge, window);
    polygon(frame, &top, p.fill, p.edge, window);
}

fn disc(frame: &Frame, at: Pt, radius: f32, color: Hsla) -> gpui::PaintQuad {
    let r = radius * frame.scale;
    gpui::fill(
        Bounds::new(
            frame.at(at) - point(px(r), px(r)),
            gpui::size(px(2.0 * r), px(2.0 * r)),
        ),
        color,
    )
    .corner_radii(px(r))
}

fn line(frame: &Frame, from: Pt, to: Pt, width: f32, color: Hsla, window: &mut Window) {
    let mut path = PathBuilder::stroke(px(width));
    path.move_to(frame.at(from));
    path.line_to(frame.at(to));
    if let Ok(path) = path.build() {
        window.paint_path(path, color);
    }
}

fn corner_extent(a: f32, r: f32) -> f32 {
    a - r + r * std::f32::consts::FRAC_1_SQRT_2
}

fn route(from: Pt, to: Pt) -> Vec<Pt> {
    let sign = (to.0 - from.0).signum();
    let dx = (to.0 - from.0).abs();
    let dy = to.1 - from.1;
    let mut rise = (dx / COS - 2.0 * dy) / 2.0;
    let mut up = 1.0;
    if rise < 0.0 || dx / COS - rise < 0.0 {
        up = -1.0;
        rise = (dx / COS + 2.0 * dy) / 2.0;
    }
    let bend = (from.0 + sign * COS * rise, from.1 - up * SIN * rise);
    let radius = (rise * 0.5).min(12.0);
    let toward = |a: Pt, b: Pt, d: f32| {
        let len = dist(a, b).max(f32::EPSILON);
        (a.0 + (b.0 - a.0) / len * d, a.1 + (b.1 - a.1) / len * d)
    };
    let enter = toward(bend, from, radius);
    let leave = toward(bend, to, radius);
    let mut points = vec![from, enter];
    for i in 1..8 {
        let t = i as f32 / 8.0;
        let u = 1.0 - t;
        points.push((
            u * u * enter.0 + 2.0 * u * t * bend.0 + t * t * leave.0,
            u * u * enter.1 + 2.0 * u * t * bend.1 + t * t * leave.1,
        ));
    }
    points.extend([leave, to]);
    points
}

fn dist(a: Pt, b: Pt) -> f32 {
    ((b.0 - a.0).powi(2) + (b.1 - a.1).powi(2)).sqrt()
}

fn length(points: &[Pt]) -> f32 {
    points.windows(2).map(|w| dist(w[0], w[1])).sum()
}

fn along(points: &[Pt], mut at: f32) -> Pt {
    for w in points.windows(2) {
        let d = dist(w[0], w[1]);
        if at <= d {
            let k = at / d.max(f32::EPSILON);
            return (
                w[0].0 + (w[1].0 - w[0].0) * k,
                w[0].1 + (w[1].1 - w[0].1) * k,
            );
        }
        at -= d;
    }
    points[points.len() - 1]
}

fn mix(a: Hsla, b: Hsla, t: f32) -> Hsla {
    let (a, b) = (gpui::Rgba::from(a), gpui::Rgba::from(b));
    gpui::Rgba {
        r: a.r + (b.r - a.r) * t,
        g: a.g + (b.g - a.g) * t,
        b: a.b + (b.b - a.b) * t,
        a: a.a + (b.a - a.a) * t,
    }
    .into()
}

fn ease(t: f32) -> f32 {
    1.0 - (1.0 - t).powi(3)
}

fn paint(
    bounds: Bounds<Pixels>,
    wires: [Wire; 6],
    glow: f32,
    p: Palette,
    window: &mut Window,
    cx: &mut App,
) {
    let (w, h) = (f32::from(bounds.size.width), f32::from(bounds.size.height));
    let scale = (w / WIDTH).min(h / HEIGHT);
    let frame = Frame {
        origin: bounds.origin
            + point(
                px((w - WIDTH * scale) / 2.0),
                px((h - HEIGHT * scale) / 2.0),
            ),
        scale,
    };
    let still = cx.reduce_motion();
    let hub_corner = corner_extent(HUB_SIZE, HUB_ROUND);
    let node_corner = corner_extent(NODE_SIZE, NODE_ROUND);
    let mut animating = false;
    let mut nodes = Vec::new();
    let mut side_lit = [false; 2];
    for (i, wire) in wires.iter().enumerate() {
        let right = i >= 3;
        let (nx, ny) = NODES[i % 3];
        let node = if right { (WIDTH - nx, ny) } else { (nx, ny) };
        let flip = if right { -1.0 } else { 1.0 };
        let port = iso(node, flip * node_corner, -flip * node_corner, 3.0);
        let socket = iso(HUB, -flip * hub_corner, flip * hub_corner, PORT_Z[i % 3]);
        let points = route(port, socket);
        let total = length(&points);
        let (start, end) = (rgb(WIRE_COLORS[i].0), rgb(WIRE_COLORS[i].1));

        let mut dashed =
            PathBuilder::stroke(px(1.0)).dash_array(&[px(1.5 * scale), px(4.0 * scale)]);
        dashed.move_to(frame.at(points[0]));
        for &q in &points[1..] {
            dashed.line_to(frame.at(q));
        }
        if let Ok(path) = dashed.build() {
            window.paint_path(path, p.faint);
        }

        let progress = if still { 1.0 } else { wire.progress() };
        animating |= progress < 1.0;
        let (shown, strength) = if wire.lit == Lit::Off {
            (1.0 - ease(progress), wire.from.strength())
        } else if wire.from == Lit::Off {
            (ease(progress), wire.lit.strength())
        } else {
            (1.0, wire.lit.strength())
        };
        if shown > 0.0 && strength > 0.0 {
            side_lit[usize::from(right)] = true;
            let reach = total * shown;
            let steps = (reach / 3.0).ceil().max(1.0) as usize;
            for k in 0..steps {
                let (a, b) = (
                    k as f32 * reach / steps as f32,
                    (k + 1) as f32 * reach / steps as f32,
                );
                line(
                    &frame,
                    along(&points, a),
                    along(&points, b),
                    1.6,
                    mix(start, end, a / total).opacity(strength),
                    window,
                );
            }
        }
        if wire.lit != Lit::Off && progress >= 1.0 && !still {
            let count = (total / FLOW_STEP) as usize;
            for k in 0..count {
                let at = k as f32 * FLOW_STEP;
                if let Some(mut animation) = crate::motion::flow_dot(-at / FLOW_SPAN, cx) {
                    animation.to = if wire.lit == Lit::On { 0.95 } else { 0.5 };
                    window.paint_animated_quad(
                        disc(&frame, along(&points, at), 1.3, gpui::white()),
                        animation,
                    );
                }
            }
        }
        let tone = match wire.lit {
            Lit::On => Some((start, end, start)),
            Lit::Partial => Some((start, end, p.warning)),
            Lit::Off => None,
        };
        nodes.push((node, port, socket, tone));
    }
    paint_hub(&frame, &p, glow, window);
    for (k, lit) in side_lit.into_iter().enumerate() {
        if lit {
            let flip = if k == 1 { -1.0 } else { 1.0 };
            let top = iso(HUB, -flip * hub_corner, flip * hub_corner, PORT_Z[0]);
            let bottom = iso(HUB, -flip * hub_corner, flip * hub_corner, PORT_Z[2]);
            let color = rgb(if k == 0 { 0x5aa8ff } else { 0xc66bff });
            line(&frame, top, bottom, 1.6, color.opacity(0.9), window);
        }
    }
    for (node, port, socket, tone) in nodes {
        slab(
            &frame,
            node,
            (NODE_SIZE, NODE_ROUND),
            (0.0, 6.0),
            p.fill,
            &p,
            window,
        );
        let light = tone.map_or(p.faint, |(_, _, light)| light);
        window.paint_quad(disc(&frame, iso(node, 0.0, 0.0, 6.0), 2.6, light));
        for (at, color) in [(port, tone.map(|t| t.0)), (socket, tone.map(|t| t.1))] {
            window.paint_quad(
                disc(&frame, at, 3.5, p.fill)
                    .border_widths(px(1.3))
                    .border_color(color.unwrap_or(p.faint)),
            );
        }
    }
    if animating {
        window.request_animation_frame();
    }
}

fn paint_hub(frame: &Frame, p: &Palette, glow: f32, window: &mut Window) {
    let (a, r) = (HUB_SIZE, HUB_ROUND);
    let glass = gpui::linear_gradient(
        180.0,
        gpui::linear_color_stop(rgb(0x7b5cff).opacity(0.35), 0.0),
        gpui::linear_color_stop(rgb(0x3b6cff).opacity(0.12), 1.0),
    );
    slab(frame, HUB, (a, r), (0.0, 22.0), glass, p, window);
    line(
        frame,
        iso(HUB, a * 0.15, a, 10.0),
        iso(HUB, a * 0.5, a, 10.0),
        2.6,
        p.edge,
        window,
    );
    let dot_alpha = 0.25 + 0.75 * glow;
    for u in [-0.7, -0.45, -0.2] {
        window.paint_quad(disc(
            frame,
            iso(HUB, u * a, a, 10.0),
            1.9,
            rgb(0x7c7cff).opacity(dot_alpha),
        ));
    }
    slab(frame, HUB, (a, r), (32.0, 42.0), p.fill, p, window);
    for i in 0..6 {
        for j in 0..6 {
            if i < 4 && j < 4 {
                continue;
            }
            let at = iso(
                HUB,
                -a + 8.0 + i as f32 * 13.2,
                -a + 8.0 + j as f32 * 13.2,
                42.0,
            );
            let color = rgb(DOT_COLORS[(i + j) % 6]).opacity(dot_alpha);
            window.paint_quad(disc(frame, at, 2.2, color));
        }
    }
    slab(frame, HUB, (a - 2.0, r), (54.0, 64.0), p.fill, p, window);
    for i in 0..5 {
        for j in 0..5 {
            if i == 4 || j == 4 {
                let key = iso(
                    HUB,
                    -a + 10.0 + i as f32 * 15.5,
                    -a + 10.0 + j as f32 * 15.5,
                    64.0,
                );
                slab(frame, key, (5.5, 2.0), (0.0, 4.0), p.fill, p, window);
            }
        }
    }
    slab(frame, HUB, (a, r), (78.0, 96.0), p.fill, p, window);
    slab(
        frame,
        iso(HUB, 0.0, 0.0, 96.0),
        (19.0, 4.0),
        (0.0, 6.0),
        p.fill,
        p,
        window,
    );
    let q = |x, y| iso(HUB, x, y, 102.0);
    polygon(
        frame,
        &[
            q(-10.0, -13.0),
            q(-3.0, -13.0),
            q(-3.0, -6.0),
            q(-10.0, -6.0),
        ],
        p.fill,
        p.edge,
        window,
    );
    for (k, y) in [-12.0, -7.0, -2.0, 3.0].into_iter().enumerate() {
        let end = if k % 2 == 0 { 12.0 } else { 8.0 };
        line(frame, q(0.0, y), q(end, y), 1.0, p.edge, window);
    }
}
