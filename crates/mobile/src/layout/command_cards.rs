use std::sync::Arc;

use zeron_proto::claude_panel::{
    self as panel, CommandReply, ContextReport, McpReport, ModelReport, TerminalEquivalent,
    UsageReport,
};
use zeron_text::WhiteSpace;

use super::cards::CardButton;
use super::display::{CardAction, ColorRole, DisplayBuilder, WidgetKind};
use super::markdown::{Ctx, PText, Px, place_text, prepare_plain};
use super::rows::{Content, RowCore, RowKind, next_version, place_text_lines, row_key};
use super::style::{Family, TYPE, Weight};

const PAD: f32 = 14.0;
const ICON: f32 = 16.0;
const GAP: f32 = 8.0;
const ITEM_GAP: f32 = 10.0;
const TRACK: f32 = 6.0;
const BAR_W: f32 = 64.0;
const LEAD_W: f32 = 46.0;
const DOT: f32 = 7.0;
const PILL_H: f32 = 30.0;
const PILL_PAD: f32 = 12.0;
const BUTTON_H: f32 = 34.0;
const BUTTON_PAD: f32 = 14.0;
const LOCAL_NOTE: &str = "Ran locally in Claude Code · no tokens used";

pub(crate) enum Item {
    Meter {
        label: PText,
        value: PText,
        foot: Option<PText>,
        fill: f32,
        warning: bool,
    },
    Bar {
        lead: PText,
        fill: f32,
        text: PText,
    },
    Status {
        name: PText,
        state: PText,
        ok: bool,
    },
    Pills(Vec<(PText, bool)>),
    Heading(PText),
    Text(PText),
}

pub(crate) struct CommandCard {
    pub icon: &'static str,
    pub title: PText,
    pub subtitle: Option<PText>,
    pub items: Vec<Item>,
    pub buttons: Vec<CardButton>,
}

struct Builder<'a, 'c> {
    ctx: &'a mut Ctx<'c>,
    copy: Vec<String>,
}

impl Builder<'_, '_> {
    fn text(&mut self, text: &str, weight: Weight, color: ColorRole) -> PText {
        let (size, lh) = TYPE.small;
        let lh = self.ctx.typo.px(lh);
        let style = self.ctx.typo.style(Family::Sans, weight, false, size);
        self.copy.push(text.to_string());
        prepare_plain(self.ctx, text, style, lh, color, WhiteSpace::Normal)
    }

    fn button(&mut self, title: &str, action: CardAction) -> CardButton {
        CardButton {
            label: self.text(title, Weight::Medium, ColorRole::TextSoft),
            title: title.to_string(),
            primary: false,
            action,
        }
    }

    fn bar_line(&mut self, line: &str) -> Item {
        let lead = line.split_whitespace().next().unwrap_or_default();
        match lead.strip_suffix('%').and_then(|p| p.parse::<f32>().ok()) {
            Some(percent) => Item::Bar {
                lead: self.text(lead, Weight::Regular, ColorRole::TextSecondary),
                fill: (percent / 100.0).clamp(0.0, 1.0),
                text: self.text(line[lead.len()..].trim(), Weight::Regular, ColorRole::Text),
            },
            None => Item::Text(self.text(line, Weight::Regular, ColorRole::TextTertiary)),
        }
    }
}

fn usage_items(b: &mut Builder<'_, '_>, report: &UsageReport) -> Vec<Item> {
    let mut items: Vec<Item> = report
        .meters
        .iter()
        .map(|m| Item::Meter {
            label: b.text(&m.label, Weight::Regular, ColorRole::Text),
            value: b.text(
                &format!("{}%", m.percent),
                Weight::Regular,
                if m.warning() {
                    ColorRole::Warning
                } else {
                    ColorRole::TextSecondary
                },
            ),
            foot: m.resets.as_ref().map(|r| {
                b.text(
                    &format!("Resets {r}"),
                    Weight::Regular,
                    ColorRole::TextTertiary,
                )
            }),
            fill: m.percent.min(100) as f32 / 100.0,
            warning: m.warning(),
        })
        .collect();
    for section in &report.sections {
        items.push(Item::Heading(b.text(
            &section.title,
            Weight::Medium,
            ColorRole::Text,
        )));
        for line in &section.lines {
            items.push(b.bar_line(line));
        }
    }
    items
}

fn context_items(b: &mut Builder<'_, '_>, report: &ContextReport) -> Vec<Item> {
    let mut items = Vec::new();
    if let Some(tokens) = &report.tokens {
        let percent = tokens
            .rsplit_once('(')
            .and_then(|(_, p)| p.trim_end_matches([')', '%']).parse::<f32>().ok())
            .unwrap_or(0.0);
        items.push(Item::Meter {
            label: b.text("Tokens", Weight::Regular, ColorRole::Text),
            value: b.text(tokens, Weight::Regular, ColorRole::TextSecondary),
            foot: None,
            fill: (percent / 100.0).clamp(0.0, 1.0),
            warning: percent >= 80.0,
        });
    }
    let max = report
        .categories
        .iter()
        .filter(|c| c.name != "Free space")
        .map(|c| c.percent)
        .fold(0.0f32, f32::max)
        .max(0.1);
    if !report.categories.is_empty() {
        items.push(Item::Heading(b.text(
            "Estimated usage by category",
            Weight::Medium,
            ColorRole::Text,
        )));
    }
    for category in &report.categories {
        items.push(Item::Bar {
            lead: b.text(
                &format!("{}%", category.percent),
                Weight::Regular,
                ColorRole::TextSecondary,
            ),
            fill: (category.percent / max).clamp(0.0, 1.0),
            text: b.text(
                &format!("{} · {}", category.name, category.tokens),
                Weight::Regular,
                ColorRole::Text,
            ),
        });
    }
    items
}

fn model_items(b: &mut Builder<'_, '_>, report: &ModelReport) -> Vec<Item> {
    let current = report.current.to_lowercase();
    let pills = report
        .available
        .iter()
        .map(|alias| {
            let on = current.split_whitespace().next() == Some(alias.as_str());
            (
                b.text(
                    alias,
                    Weight::Regular,
                    if on {
                        ColorRole::Accent
                    } else {
                        ColorRole::Text
                    },
                ),
                on,
            )
        })
        .collect();
    vec![
        Item::Pills(pills),
        Item::Text(b.text(
            "Zeron runs this chat on its model chip; change it there.",
            Weight::Regular,
            ColorRole::TextTertiary,
        )),
    ]
}

fn mcp_items(b: &mut Builder<'_, '_>, report: &McpReport) -> Vec<Item> {
    [
        ("Connected", report.connected, true),
        ("Not connected", report.not_connected, false),
        ("Disabled", report.disabled, false),
    ]
    .into_iter()
    .map(|(name, n, ok)| Item::Status {
        name: b.text(name, Weight::Regular, ColorRole::Text),
        state: b.text(&n.to_string(), Weight::Regular, ColorRole::TextSecondary),
        ok: ok && n > 0,
    })
    .collect()
}

pub(crate) fn command_card(
    ctx: &mut Ctx,
    key: &str,
    entry_id: &str,
    command: &str,
    reply: &str,
) -> Option<RowCore> {
    let parsed = panel::command_reply(command, reply)?;
    let mut b = Builder {
        ctx,
        copy: Vec::new(),
    };
    let (icon, title, subtitle, mut items, buttons) = match &parsed {
        CommandReply::Usage(report) => (
            "bolt",
            "Usage",
            Some("Claude Code · subscription".to_string()),
            usage_items(&mut b, report),
            vec![b.button("Open Usage page", CardAction::OpenUsage)],
        ),
        CommandReply::Context(report) => (
            "chart.pie",
            "Context",
            Some(
                report
                    .model
                    .as_ref()
                    .map_or("Claude Code".to_string(), |m| format!("Claude Code · {m}")),
            ),
            context_items(&mut b, report),
            Vec::new(),
        ),
        CommandReply::Model(report) => (
            "bolt",
            "Model",
            Some(format!("Claude Code · {}", report.current)),
            model_items(&mut b, report),
            vec![b.button("Change model", CardAction::OpenChip { id: "model".into() })],
        ),
        CommandReply::Mcp(report) => (
            "powerplug",
            "MCP servers",
            Some(format!(
                "{} connected · {} not connected",
                report.connected, report.not_connected
            )),
            mcp_items(&mut b, report),
            vec![b.button("Open Claude Code settings", CardAction::OpenClaudeConfig)],
        ),
        CommandReply::TerminalOnly(name) => {
            let buttons = panel::terminal_equivalent(name)
                .map(|(label, equivalent)| {
                    let action = match equivalent {
                        TerminalEquivalent::SessionInfo => CardAction::OpenSessionInfo,
                        TerminalEquivalent::PermissionChip => CardAction::OpenChip {
                            id: "permission".into(),
                        },
                        TerminalEquivalent::ClaudeConfig => CardAction::OpenClaudeConfig,
                        TerminalEquivalent::CopyTranscript => CardAction::CopyTranscript,
                    };
                    b.button(label, action)
                })
                .into_iter()
                .collect();
            ("apple.terminal", "", None, Vec::new(), buttons)
        }
    };
    let title = match &parsed {
        CommandReply::TerminalOnly(name) => panel::terminal_only_notice(name),
        _ => title.to_string(),
    };
    if !matches!(parsed, CommandReply::TerminalOnly(_)) {
        items.push(Item::Text(b.text(
            LOCAL_NOTE,
            Weight::Regular,
            ColorRole::TextTertiary,
        )));
    }
    Some(finish(
        b,
        key,
        entry_id,
        icon,
        &title,
        subtitle.as_deref(),
        items,
        buttons,
    ))
}

pub(crate) fn summary_card(
    ctx: &mut Ctx,
    key: &str,
    entry_id: &str,
    text: &str,
) -> Option<RowCore> {
    let parts = panel::summary_parts(text)?;
    let mut b = Builder {
        ctx,
        copy: Vec::new(),
    };
    let body = parts
        .iter()
        .map(|(label, value)| format!("{label} → {value}"))
        .collect::<Vec<_>>()
        .join(", ");
    let items = vec![Item::Text(b.text(
        &body,
        Weight::Regular,
        ColorRole::TextSecondary,
    ))];
    let buttons = vec![b.button("Reopen", CardAction::OpenClaudeConfig)];
    let title = panel::SUMMARY_PREFIX.trim_end_matches([' ', '·']);
    Some(finish(
        b,
        key,
        entry_id,
        "gearshape",
        title,
        None,
        items,
        buttons,
    ))
}

#[allow(clippy::too_many_arguments)]
fn finish(
    mut b: Builder<'_, '_>,
    key: &str,
    entry_id: &str,
    icon: &'static str,
    title: &str,
    subtitle: Option<&str>,
    items: Vec<Item>,
    buttons: Vec<CardButton>,
) -> RowCore {
    let title = b.text(title, Weight::Medium, ColorRole::Text);
    let subtitle = subtitle.map(|s| b.text(s, Weight::Regular, ColorRole::TextSecondary));
    RowCore {
        key: row_key(key),
        version: next_version(),
        kind: RowKind::Card,
        entry_id: Arc::from(entry_id),
        copy_text: b.copy.join("\n"),
        content: Content::Command(CommandCard {
            icon,
            title,
            subtitle,
            items,
            buttons,
        }),
    }
}

fn width(t: &PText) -> f32 {
    t.p.max_content_width().ceil()
}

pub(crate) fn place_command_card(
    card: &CommandCard,
    px: Px,
    x: f32,
    top: f32,
    cw: f32,
    out: Option<&mut DisplayBuilder>,
) -> f32 {
    let h = layout(card, px, x, top, cw, None);
    if let Some(o) = out {
        o.fill(x, top, cw, h, px.v(14.0), ColorRole::CardBackground);
        o.hairline(x, top, cw, h, px.v(14.0), ColorRole::CardBorder);
        layout(card, px, x, top, cw, Some(o));
    }
    h
}

fn layout(
    card: &CommandCard,
    px: Px,
    x: f32,
    top: f32,
    cw: f32,
    mut out: Option<&mut DisplayBuilder>,
) -> f32 {
    let pad = px.v(PAD);
    let inner_x = x + pad;
    let inner_w = (cw - pad * 2.0).max(1.0);
    let icon = px.v(ICON);
    let title_x = inner_x + icon + px.v(GAP);
    let title_w = (x + cw - pad - title_x).max(1.0);
    let mut y = top + pad;
    if let Some(o) = out.as_deref_mut() {
        o.widget(
            WidgetKind::Icon {
                name: card.icon.into(),
                color: ColorRole::TextSecondary,
            },
            (inner_x, y + (card.title.lh - icon) / 2.0, icon, icon),
            None,
        );
    }
    let th = place_text(&card.title, title_x, y, title_w, out.as_deref_mut());
    if let Some(sub) = &card.subtitle {
        let tw = width(&card.title);
        let gap = px.v(GAP);
        if th <= card.title.lh + 0.5 && tw + gap + width(sub) <= title_w {
            place_text_lines_opt(sub, title_x + tw + gap, y, title_w - tw - gap, px, &mut out);
            y += th;
        } else {
            y += th;
            y += place_text(sub, title_x, y, title_w, out.as_deref_mut());
        }
    } else {
        y += th;
    }
    if !card.items.is_empty() {
        y += pad;
        if let Some(o) = out.as_deref_mut() {
            o.fill(x, y - px.v(1.0), cw, px.v(1.0), 0.0, ColorRole::Rule);
        }
        y += px.v(ITEM_GAP) - px.v(1.0);
        for (ix, item) in card.items.iter().enumerate() {
            if ix > 0 {
                y += px.v(ITEM_GAP);
            }
            y += place_item(item, px, inner_x, y, inner_w, &mut out);
        }
    }
    if !card.buttons.is_empty() {
        y += pad;
        let bh = px.v(BUTTON_H);
        let mut bx = inner_x;
        for b in &card.buttons {
            let bw = (width(&b.label) + px.v(BUTTON_PAD) * 2.0).min(inner_w);
            if bx > inner_x && bx + bw > inner_x + inner_w {
                bx = inner_x;
                y += bh + px.v(GAP);
            }
            if let Some(o) = out.as_deref_mut() {
                o.hairline(bx, y, bw, bh, bh / 2.0, ColorRole::ButtonBorder);
                place_text_lines(
                    &b.label,
                    bx + px.v(BUTTON_PAD),
                    y + (bh - b.label.lh) / 2.0,
                    (bw - px.v(BUTTON_PAD) * 2.0).max(1.0),
                    1,
                    px,
                    o,
                );
                o.widget(
                    WidgetKind::CardButton {
                        action: b.action.clone(),
                    },
                    (bx, y, bw, bh),
                    Some(b.title.clone()),
                );
            }
            bx += bw + px.v(GAP);
        }
        y += bh;
    }
    y + pad - top
}

fn place_text_lines_opt(
    t: &PText,
    x: f32,
    y: f32,
    w: f32,
    px: Px,
    out: &mut Option<&mut DisplayBuilder>,
) {
    if let Some(o) = out.as_deref_mut() {
        place_text_lines(t, x, y, w, 1, px, o);
    }
}

fn place_item(
    item: &Item,
    px: Px,
    x: f32,
    y: f32,
    w: f32,
    out: &mut Option<&mut DisplayBuilder>,
) -> f32 {
    match item {
        Item::Heading(t) | Item::Text(t) => place_text(t, x, y, w, out.as_deref_mut()),
        Item::Meter {
            label,
            value,
            foot,
            fill,
            warning,
        } => {
            let vw = width(value).min(w / 2.0);
            let lh = place_text(
                label,
                x,
                y,
                (w - vw - px.v(GAP)).max(1.0),
                out.as_deref_mut(),
            );
            place_text_lines_opt(value, x + w - vw, y, vw, px, out);
            let track_y = y + lh + px.v(4.0);
            let track = px.v(TRACK);
            if let Some(o) = out.as_deref_mut() {
                o.fill(x, track_y, w, track, track / 2.0, ColorRole::ChipBackground);
                if *fill > 0.0 {
                    o.fill(
                        x,
                        track_y,
                        (w * fill).max(track),
                        track,
                        track / 2.0,
                        if *warning {
                            ColorRole::Warning
                        } else {
                            ColorRole::Accent
                        },
                    );
                }
            }
            let mut h = lh + px.v(4.0) + track;
            if let Some(foot) = foot {
                h += px.v(4.0);
                h += place_text(foot, x, y + h, w, out.as_deref_mut());
            }
            h
        }
        Item::Bar { lead, fill, text } => {
            let lead_w = px.v(LEAD_W);
            let bar_w = px.v(BAR_W);
            let text_x = x + lead_w + bar_w + px.v(GAP) * 2.0;
            let th = place_text(
                text,
                text_x,
                y,
                (x + w - text_x).max(1.0),
                out.as_deref_mut(),
            );
            place_text_lines_opt(lead, x, y, lead_w, px, out);
            let track = px.v(TRACK);
            let track_y = y + (lead.lh - track) / 2.0;
            if let Some(o) = out.as_deref_mut() {
                let bx = x + lead_w + px.v(GAP);
                o.fill(
                    bx,
                    track_y,
                    bar_w,
                    track,
                    track / 2.0,
                    ColorRole::ChipBackground,
                );
                if *fill > 0.0 {
                    o.fill(
                        bx,
                        track_y,
                        (bar_w * fill).max(track),
                        track,
                        track / 2.0,
                        ColorRole::TextTertiary,
                    );
                }
            }
            th.max(lead.lh)
        }
        Item::Status { name, state, ok } => {
            let dot = px.v(DOT);
            let sw = width(state).min(w / 2.0);
            let name_x = x + dot + px.v(GAP);
            let nh = place_text(
                name,
                name_x,
                y,
                (w - sw - dot - px.v(GAP) * 2.0).max(1.0),
                out.as_deref_mut(),
            );
            place_text_lines_opt(state, x + w - sw, y, sw, px, out);
            if let Some(o) = out.as_deref_mut() {
                o.fill(
                    x,
                    y + (name.lh - dot) / 2.0,
                    dot,
                    dot,
                    dot / 2.0,
                    if *ok {
                        ColorRole::Success
                    } else {
                        ColorRole::TextTertiary
                    },
                );
            }
            nh
        }
        Item::Pills(pills) => {
            let ph = px.v(PILL_H);
            let gap = px.v(GAP);
            let (mut px_, mut py) = (x, y);
            for (label, on) in pills {
                let pw = width(label) + px.v(PILL_PAD) * 2.0;
                if px_ > x && px_ + pw > x + w {
                    px_ = x;
                    py += ph + gap;
                }
                if let Some(o) = out.as_deref_mut() {
                    if *on {
                        o.fill(px_, py, pw, ph, ph / 2.0, ColorRole::AccentWash);
                    }
                    o.hairline(
                        px_,
                        py,
                        pw,
                        ph,
                        ph / 2.0,
                        if *on {
                            ColorRole::Accent
                        } else {
                            ColorRole::ButtonBorder
                        },
                    );
                    place_text_lines(
                        label,
                        px_ + px.v(PILL_PAD),
                        py + (ph - label.lh) / 2.0,
                        pw,
                        1,
                        px,
                        o,
                    );
                }
                px_ += pw + gap;
            }
            py + ph - y
        }
    }
}

pub(crate) fn heap_bytes(card: &CommandCard) -> usize {
    let item = |item: &Item| match item {
        Item::Meter {
            label, value, foot, ..
        } => {
            label.p.heap_bytes()
                + value.p.heap_bytes()
                + foot.as_ref().map_or(0, |f| f.p.heap_bytes())
        }
        Item::Bar { lead, text, .. } => lead.p.heap_bytes() + text.p.heap_bytes(),
        Item::Status { name, state, .. } => name.p.heap_bytes() + state.p.heap_bytes(),
        Item::Pills(pills) => pills.iter().map(|(p, _)| p.p.heap_bytes()).sum(),
        Item::Heading(t) | Item::Text(t) => t.p.heap_bytes(),
    };
    card.title.p.heap_bytes()
        + card.subtitle.as_ref().map_or(0, |s| s.p.heap_bytes())
        + card.items.iter().map(item).sum::<usize>()
        + card
            .buttons
            .iter()
            .map(|b| b.label.p.heap_bytes())
            .sum::<usize>()
}
