use zeron_text::WhiteSpace;

use super::display::{ColorRole, Decoration, DisplayBuilder, LinkHit, WidgetKind};
use super::markdown::{Ctx, PText, Px, place_text, prepare_plain};
use super::rows::place_text_lines;
use super::style::{Family, Weight};

const T: f32 = 1.125;
const SIZE: f32 = 11.5 * T;
const MONO_SIZE: f32 = 10.0 * T;
const LH: f32 = 18.0;
const ICON: f32 = 13.0;
const GAP: f32 = 5.0;
const RULE_GAP: f32 = 10.0;
const RULE_MIN: f32 = 24.0;
const PAD_Y: f32 = 4.0;
const PILL_PAD_X: f32 = 7.0;
const KEY_W: f32 = 72.0;
const BOX_LINES: usize = 12;

fn d(px: Px, v: f32) -> f32 {
    px.v(v)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Align {
    Leading,
    Center,
    Trailing,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Rule {
    None,
    After,
    Around,
}

pub(crate) enum Item {
    Icon { name: String, color: ColorRole },
    Text(PText),
    Pill { label: PText, url: String },
    Hint { title: String, text: String },
    Chevron(bool),
}

pub(crate) enum Block {
    Caption(PText),
    Field(PText, PText),
    Mono { text: PText, full: String, title: String, accent: bool },
    Note(PText),
}

pub(crate) struct Strip {
    pub items: Vec<Item>,
    pub align: Align,
    pub rule: Rule,
    pub open: Option<bool>,
    pub card: Vec<Block>,
    pub after: Vec<Block>,
}

pub(crate) struct Faces {
    regular: super::style::Resolved,
    medium: super::style::Resolved,
    mono: super::style::Resolved,
    lh: f32,
}

impl Faces {
    pub fn new(ctx: &mut Ctx) -> Self {
        Self {
            regular: ctx.typo.style(Family::Sans, Weight::Regular, false, SIZE),
            medium: ctx.typo.style(Family::Sans, Weight::Medium, false, SIZE),
            mono: ctx.typo.style(Family::Mono, Weight::Regular, false, MONO_SIZE),
            lh: ctx.typo.px(LH),
        }
    }

    pub fn text(&self, ctx: &mut Ctx, s: &str, color: ColorRole) -> PText {
        prepare_plain(ctx, s, self.regular, self.lh, color, WhiteSpace::Normal)
    }

    pub fn medium(&self, ctx: &mut Ctx, s: &str, color: ColorRole) -> PText {
        prepare_plain(ctx, s, self.medium, self.lh, color, WhiteSpace::Normal)
    }

    pub fn mono(&self, ctx: &mut Ctx, s: &str, color: ColorRole) -> PText {
        prepare_plain(ctx, s, self.mono, self.lh, color, WhiteSpace::PreWrap)
    }
}

pub(crate) fn linked(mut t: PText, url: String) -> PText {
    t.links.push(url);
    for paint in &mut t.paints {
        paint.link = Some(0);
    }
    t
}

pub(crate) fn struck(mut t: PText) -> PText {
    for paint in &mut t.paints {
        paint.decoration = Decoration::Strikethrough;
    }
    t
}

pub(crate) fn chat_link(chat_id: &str) -> String {
    let mut out = String::from("zeron-chat://open?id=");
    for byte in chat_id.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            out.push(byte as char);
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

struct Slot {
    x: f32,
    y: f32,
    w: f32,
    h: f32,
}

struct Line {
    y: f32,
    h: f32,
    x0: f32,
    x1: f32,
}

fn item_size(item: &Item, px: Px, width: f32) -> (f32, f32) {
    let icon = d(px, ICON);
    match item {
        Item::Icon { .. } | Item::Hint { .. } | Item::Chevron(_) => (icon, icon),
        Item::Text(t) => {
            let natural = t.p.max_content_width().ceil();
            if natural <= width {
                (natural, t.lh)
            } else {
                (width, place_text(t, 0.0, 0.0, width, None))
            }
        }
        Item::Pill { label, .. } => (label.p.max_content_width().ceil() + d(px, PILL_PAD_X) * 2.0, label.lh + d(px, 2.0)),
    }
}

fn flow(items: &[Item], align: Align, px: Px, width: f32) -> (Vec<Slot>, Vec<Line>) {
    let gap = d(px, GAP);
    let mut slots: Vec<Slot> = Vec::with_capacity(items.len());
    let mut lines: Vec<Line> = Vec::new();
    let mut start = 0;
    let mut cursor = 0.0f32;
    let mut y = 0.0f32;
    let mut finish = |slots: &mut Vec<Slot>, start: usize, end: usize, y: &mut f32| {
        let line_h = slots[start..end].iter().map(|s| s.h).fold(0.0f32, f32::max);
        let line_w = slots[start..end].last().map_or(0.0, |s| s.x + s.w);
        let shift = match align {
            Align::Leading => 0.0,
            Align::Center => ((width - line_w) / 2.0).max(0.0).floor(),
            Align::Trailing => (width - line_w).max(0.0),
        };
        for slot in &mut slots[start..end] {
            slot.x += shift;
            slot.y = *y + (line_h - slot.h) / 2.0;
        }
        lines.push(Line {
            y: *y,
            h: line_h,
            x0: shift,
            x1: shift + line_w,
        });
        *y += line_h;
    };
    for item in items {
        let (w, h) = item_size(item, px, width);
        if slots.len() > start && cursor + gap + w > width {
            let end = slots.len();
            finish(&mut slots, start, end, &mut y);
            start = end;
            cursor = 0.0;
        }
        let x = if slots.len() > start { cursor + gap } else { 0.0 };
        slots.push(Slot { x, y: 0.0, w, h });
        cursor = x + w;
    }
    if slots.len() > start {
        let end = slots.len();
        finish(&mut slots, start, end, &mut y);
    }
    (slots, lines)
}

fn paint_item(item: &Item, slot: &Slot, x: f32, y: f32, px: Px, o: &mut DisplayBuilder) {
    let (sx, sy) = (x + slot.x, y + slot.y);
    match item {
        Item::Icon { name, color } => {
            o.widget(WidgetKind::Icon { name: name.clone(), color: *color }, (sx, sy, slot.w, slot.h), None);
        }
        Item::Chevron(open) => {
            o.widget(WidgetKind::Chevron { expanded: *open }, (sx, sy, slot.w, slot.h), None);
        }
        Item::Hint { title, text } => {
            o.widget(
                WidgetKind::Icon {
                    name: "info.circle".into(),
                    color: ColorRole::TextFaint,
                },
                (sx, sy, slot.w, slot.h),
                None,
            );
            let hit = d(px, 30.0);
            o.widget(
                WidgetKind::Detail { title: title.clone() },
                (sx + (slot.w - hit) / 2.0, sy + (slot.h - hit) / 2.0, hit, hit),
                Some(text.clone()),
            );
        }
        Item::Text(t) => {
            place_text(t, sx, sy, slot.w + 1.0, Some(o));
        }
        Item::Pill { label, url } => {
            o.fill(sx, sy, slot.w, slot.h, d(px, 6.0), ColorRole::ToolBadge);
            place_text(label, sx + d(px, PILL_PAD_X), sy + (slot.h - label.lh) / 2.0, slot.w, Some(o));
            o.links.push(LinkHit {
                x: sx,
                y: sy - d(px, 6.0),
                w: slot.w,
                h: slot.h + d(px, 12.0),
                url: url.clone(),
                scroller: o.scroller,
            });
        }
    }
}

pub(crate) fn place_strip(s: &Strip, px: Px, x: f32, y: f32, cw: f32, mut out: Option<&mut DisplayBuilder>) -> f32 {
    let rule_room = d(px, RULE_GAP + RULE_MIN);
    let flow_w = match s.rule {
        Rule::None => cw,
        Rule::After => cw - rule_room,
        Rule::Around => cw - rule_room * 2.0,
    }
    .max(d(px, 40.0));
    let fx = match s.rule {
        Rule::Around => x + rule_room,
        _ => x,
    };
    let fx = match s.align {
        Align::Trailing if s.rule == Rule::None => x + cw - flow_w,
        _ => fx,
    };
    let (slots, lines) = flow(&s.items, s.align, px, flow_w);
    let pad = d(px, PAD_Y);
    let head_h = lines.last().map_or(0.0, |l| l.y + l.h) + pad * 2.0;
    if let Some(o) = out.as_deref_mut() {
        let top = y + pad;
        for (item, slot) in s.items.iter().zip(&slots) {
            paint_item(item, slot, fx, top, px, o);
        }
        let gap = d(px, RULE_GAP);
        match (s.rule, lines.as_slice()) {
            (Rule::Around, [line]) => {
                let ry = (top + line.y + line.h / 2.0).floor();
                o.fill(x, ry, (fx + line.x0 - gap - x).max(0.0), 1.0, 0.0, ColorRole::Rule);
                let rx = fx + line.x1 + gap;
                o.fill(rx, ry, (x + cw - rx).max(0.0), 1.0, 0.0, ColorRole::Rule);
            }
            (Rule::After, [.., line]) => {
                let ry = (top + line.y + line.h / 2.0).floor();
                let rx = fx + line.x1 + gap;
                o.fill(rx, ry, (x + cw - rx).max(0.0), 1.0, 0.0, ColorRole::Rule);
            }
            _ => {}
        }
        if let Some(open) = s.open {
            o.widget(WidgetKind::Disclosure { expanded: open }, (x, y, cw, head_h), None);
        }
    }
    if s.open != Some(true) {
        return head_h;
    }
    let mut by = y + head_h + d(px, 6.0);
    if !s.card.is_empty() {
        let inset = d(px, 10.0);
        let body = place_blocks(&s.card, px, x + inset, by + inset, cw - inset * 2.0, None);
        let card_h = body + inset * 2.0;
        if let Some(o) = out.as_deref_mut() {
            o.fill(x, by, cw, card_h, d(px, 8.0), ColorRole::CardBackground);
            o.hairline(x, by, cw, card_h, d(px, 8.0), ColorRole::CardBorder);
            place_blocks(&s.card, px, x + inset, by + inset, cw - inset * 2.0, Some(o));
        }
        by += card_h + d(px, 10.0);
    }
    by += place_blocks(&s.after, px, x, by, cw, out);
    by - y + d(px, 4.0)
}

fn place_blocks(blocks: &[Block], px: Px, x: f32, y: f32, w: f32, mut out: Option<&mut DisplayBuilder>) -> f32 {
    let mut by = y;
    for (i, block) in blocks.iter().enumerate() {
        if i > 0 {
            by += match block {
                Block::Field(..) => d(px, 6.0),
                _ => d(px, 10.0),
            };
        }
        by += place_block(block, px, x, by, w, out.as_deref_mut());
    }
    by - y
}

fn place_block(block: &Block, px: Px, x: f32, y: f32, w: f32, out: Option<&mut DisplayBuilder>) -> f32 {
    match block {
        Block::Caption(t) | Block::Note(t) => place_text(t, x, y, w, out),
        Block::Field(key, value) => {
            let kw = d(px, KEY_W);
            let vw = (w - kw - d(px, 12.0)).max(1.0);
            match out {
                Some(o) => {
                    let kh = place_text(key, x, y, kw, Some(o));
                    let vh = place_text(value, x + kw + d(px, 12.0), y, vw, Some(o));
                    kh.max(vh)
                }
                None => place_text(key, x, y, kw, None).max(place_text(value, x, y, vw, None)),
            }
        }
        Block::Mono { text, full, title, accent } => {
            let inset = d(px, 10.0);
            let tw = (w - inset * 2.0).max(1.0);
            let th = place_text(text, 0.0, 0.0, tw, None).min(BOX_LINES as f32 * text.lh);
            let h = th + inset * 2.0;
            if let Some(o) = out {
                let border = if *accent { ColorRole::Accent } else { ColorRole::CardBorder };
                o.hairline(x, y, w, h, d(px, 8.0), border);
                place_text_lines(text, x + inset, y + inset, tw, BOX_LINES, px, o);
                o.widget(WidgetKind::Detail { title: title.clone() }, (x, y, w, h), Some(full.clone()));
            }
            h
        }
    }
}

pub(crate) fn heap_bytes(s: &Strip) -> usize {
    let block = |b: &Block| match b {
        Block::Caption(t) | Block::Note(t) => t.p.heap_bytes(),
        Block::Field(k, v) => k.p.heap_bytes() + v.p.heap_bytes(),
        Block::Mono { text, full, .. } => text.p.heap_bytes() + full.len(),
    };
    s.items
        .iter()
        .map(|item| match item {
            Item::Text(t) => t.p.heap_bytes(),
            Item::Pill { label, url } => label.p.heap_bytes() + url.len(),
            Item::Hint { text, .. } => text.len(),
            Item::Icon { .. } | Item::Chevron(_) => 0,
        })
        .sum::<usize>()
        + s.card.iter().map(block).sum::<usize>()
        + s.after.iter().map(block).sum::<usize>()
}
