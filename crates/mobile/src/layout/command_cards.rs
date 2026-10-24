use std::sync::Arc;

use zeron_proto::claude_cards::{
    self as cards, LOCAL_FOOTNOTE, MCP_EMPTY, MCP_FOOTNOTE, MODEL_FOOTNOTE, Series, TerminalAction,
};
use zeron_proto::claude_config::CommandCard;
use zeron_proto::claude_panel as panel;
use zeron_proto::usage::format_tokens;
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
const STACK: f32 = 10.0;
const BAR_W: f32 = 64.0;
const LEAD_W: f32 = 46.0;
const DOT: f32 = 7.0;
const BUTTON_H: f32 = 32.0;
const BUTTON_PAD: f32 = 13.0;

pub(crate) enum Item {
    Meter {
        label: PText,
        value: PText,
        foot: Option<PText>,
        fill: f32,
        warning: bool,
    },
    Stack(Vec<(f32, ColorRole)>),
    Legend {
        color: ColorRole,
        name: PText,
        value: PText,
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
    Choices(Vec<CardButton>),
    Text(PText),
}

pub(crate) struct CommandCardView {
    pub icon: &'static str,
    pub title: PText,
    pub subtitle: Option<PText>,
    pub pill: Option<PText>,
    pub items: Vec<Item>,
    pub buttons: Vec<CardButton>,
    pub notice: bool,
}

struct Builder<'a, 'c> {
    ctx: &'a mut Ctx<'c>,
    copy: Vec<String>,
}

impl Builder<'_, '_> {
    fn styled(&mut self, text: &str, family: Family, weight: Weight, color: ColorRole) -> PText {
        let (size, lh) = TYPE.small;
        let lh = self.ctx.typo.px(lh);
        let style = self.ctx.typo.style(family, weight, false, size);
        self.copy.push(text.to_string());
        prepare_plain(self.ctx, text, style, lh, color, WhiteSpace::PreWrap)
    }

    fn text(&mut self, text: &str, weight: Weight, color: ColorRole) -> PText {
        self.styled(text, Family::Sans, weight, color)
    }

    fn note(&mut self, text: &str) -> Item {
        Item::Text(self.text(text, Weight::Regular, ColorRole::TextTertiary))
    }

    fn heading(&mut self, text: &str) -> Item {
        Item::Text(self.text(text, Weight::Medium, ColorRole::Text))
    }

    fn button(&mut self, title: &str, action: CardAction, on: bool) -> CardButton {
        let label = self.text(
            title,
            Weight::Medium,
            if on {
                ColorRole::Accent
            } else {
                ColorRole::TextSoft
            },
        );
        self.copy.pop();
        CardButton {
            label,
            title: title.to_string(),
            primary: on,
            action,
        }
    }

    fn copy_button(&mut self, raw: &str) -> CardButton {
        let mut button = self.button("Copy", CardAction::CopyText { text: raw.into() }, false);
        button.title = "Copy as Markdown".into();
        button
    }

    fn bar_line(&mut self, line: &str) -> Item {
        let lead = line.split_whitespace().next().unwrap_or_default();
        match lead.strip_suffix('%').and_then(|p| p.parse::<f32>().ok()) {
            Some(percent) => Item::Bar {
                lead: self.text(lead, Weight::Regular, ColorRole::TextSecondary),
                fill: (percent / 100.0).clamp(0.0, 1.0),
                text: self.text(line[lead.len()..].trim(), Weight::Regular, ColorRole::Text),
            },
            None => self.note(line),
        }
    }
}

struct Spec {
    icon: &'static str,
    title: String,
    subtitle: Option<String>,
    pill: Option<String>,
    items: Vec<Item>,
    buttons: Vec<CardButton>,
    notice: bool,
}

fn series_color(name: &str) -> ColorRole {
    match cards::series(name) {
        Series::System => ColorRole::Accent,
        Series::Mcp => ColorRole::SyntaxFunction,
        Series::Agents => ColorRole::SyntaxType,
        Series::Memory => ColorRole::Warning,
        Series::Skills => ColorRole::Success,
        Series::Messages => ColorRole::Danger,
        Series::Other => ColorRole::TextTertiary,
    }
}

fn context(b: &mut Builder<'_, '_>, card: &CommandCard) -> Option<Spec> {
    let report = cards::parse_context(&card.text)?;
    let mut stack: Vec<(f32, ColorRole)> = report
        .categories
        .iter()
        .filter(|(_, n)| *n > 0)
        .map(|(name, n)| (report.bar_fraction(*n), series_color(name)))
        .collect();
    if report.buffer > 0 {
        stack.push((report.bar_fraction(report.buffer), ColorRole::Rule));
    }
    let mut items = vec![Item::Stack(stack), b.note(&report.bar_caption())];
    let total = report.total;
    let legend = |b: &mut Builder<'_, '_>, color: ColorRole, name: &str, n: u64| Item::Legend {
        color,
        name: b.text(name, Weight::Regular, ColorRole::Text),
        value: b.text(
            &format!("{} · {}", format_tokens(n), cards::percent(n, total)),
            Weight::Regular,
            ColorRole::TextSecondary,
        ),
    };
    for (name, n) in &report.categories {
        items.push(legend(b, series_color(name), name, *n));
    }
    if report.buffer > 0 {
        items.push(legend(
            b,
            ColorRole::Rule,
            "Autocompact buffer",
            report.buffer,
        ));
    }
    items.push(legend(b, ColorRole::ChipBackground, "Free", report.free));
    if let Some(tip) = report.tip() {
        items.push(b.note(&tip));
    }
    for group in &report.groups {
        items.push(b.heading(&format!("{} · {}", group.title, format_tokens(group.total))));
        for (name, n) in &group.rows {
            items.push(Item::Legend {
                color: ColorRole::ChipBackground,
                name: b.text(name, Weight::Regular, ColorRole::TextSecondary),
                value: b.text(
                    &format_tokens(*n),
                    Weight::Regular,
                    ColorRole::TextSecondary,
                ),
            });
        }
    }
    if let Some(note) = &card.note {
        items.push(b.note(note));
    }
    let buttons = vec![
        b.button("Compact now", CardAction::Compact, true),
        b.button("Manage tools", CardAction::OpenSettings, false),
        b.button("Context settings", CardAction::OpenSettings, false),
        b.copy_button(&card.text),
    ];
    Some(Spec {
        icon: "chart.pie",
        title: "Context".into(),
        subtitle: Some(format!("Claude Code · {}", report.model)),
        pill: Some(report.pill()),
        items,
        buttons,
        notice: false,
    })
}

fn usage(b: &mut Builder<'_, '_>, card: &CommandCard) -> Option<Spec> {
    let report = panel::parse_usage(&card.text);
    if report.meters.is_empty() {
        return None;
    }
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
    if !report.sections.is_empty() {
        items.push(b.heading("What uses your limits"));
    }
    for section in &report.sections {
        items.push(b.note(&section.title));
        for line in &section.lines {
            items.push(b.bar_line(line));
        }
    }
    let buttons = vec![
        b.button("Open Usage page", CardAction::OpenUsage, false),
        b.copy_button(&card.text),
    ];
    Some(Spec {
        icon: "bolt",
        title: "Usage".into(),
        subtitle: Some(cards::usage_subtitle(report.note.as_deref())),
        pill: None,
        items,
        buttons,
        notice: false,
    })
}

fn model(b: &mut Builder<'_, '_>, card: &CommandCard, live: Option<&str>) -> Spec {
    let current = live.map(str::to_owned).or_else(|| card.model.clone());
    let selected = cards::selected_model(&card.models, current.as_deref());
    let label = selected
        .map(|ix| card.models[ix].1.clone())
        .or(current)
        .unwrap_or_else(|| "default".into());
    let choices = card
        .models
        .iter()
        .enumerate()
        .map(|(ix, (id, name))| {
            b.button(
                name,
                CardAction::PickModel {
                    id: id.clone(),
                    label: name.clone(),
                },
                selected == Some(ix),
            )
        })
        .collect();
    let note = card.note.clone().unwrap_or_else(|| MODEL_FOOTNOTE.into());
    Spec {
        icon: "bolt",
        title: "Model".into(),
        subtitle: Some("Claude Code".into()),
        pill: Some(label),
        items: vec![Item::Choices(choices), b.note(&note)],
        buttons: Vec::new(),
        notice: false,
    }
}

fn mcp(b: &mut Builder<'_, '_>, card: &CommandCard) -> Option<Spec> {
    let servers = cards::mcp_servers(card)?;
    let connected = servers.iter().filter(|(_, s)| s == "connected").count();
    let mut items: Vec<Item> = servers
        .iter()
        .map(|(name, state)| Item::Status {
            name: b.text(name, Weight::Regular, ColorRole::Text),
            state: b.text(state, Weight::Regular, ColorRole::TextSecondary),
            ok: state == "connected",
        })
        .collect();
    if servers.is_empty() {
        items.push(b.note(MCP_EMPTY));
    }
    items.push(b.note(MCP_FOOTNOTE));
    let buttons = vec![
        b.button("Manage in Settings", CardAction::OpenSettings, false),
        b.copy_button(&card.text),
    ];
    Some(Spec {
        icon: "powerplug",
        title: "MCP servers".into(),
        subtitle: Some(format!(
            "{connected} connected · {} not connected",
            servers.len() - connected
        )),
        pill: None,
        items,
        buttons,
        notice: false,
    })
}

fn notice(b: &mut Builder<'_, '_>, name: &str) -> Spec {
    let buttons = cards::terminal_action(name)
        .and_then(|(label, action)| {
            let action = match action {
                TerminalAction::Status => CardAction::OpenClaudeStatus,
                TerminalAction::Permissions => CardAction::OpenClaudePermissions,
                TerminalAction::Help => CardAction::OpenDocs,
                TerminalAction::Hooks | TerminalAction::Export => return None,
            };
            Some(b.button(label, action, false))
        })
        .into_iter()
        .collect();
    Spec {
        icon: "apple.terminal",
        title: format!("/{name} only runs in Claude Code's terminal."),
        subtitle: None,
        pill: None,
        items: Vec::new(),
        buttons,
        notice: true,
    }
}

fn generic(b: &mut Builder<'_, '_>, card: &CommandCard) -> Spec {
    let failed = card.error.is_some();
    let text = card.error.clone().unwrap_or_else(|| card.text.clone());
    let body = b.styled(
        text.trim(),
        Family::Mono,
        Weight::Regular,
        if failed {
            ColorRole::Warning
        } else {
            ColorRole::Text
        },
    );
    let items = vec![Item::Text(body), b.note(LOCAL_FOOTNOTE)];
    let buttons = vec![b.copy_button(&text)];
    Spec {
        icon: "apple.terminal",
        title: card.command.clone(),
        subtitle: Some("Claude Code · command output".into()),
        pill: None,
        items,
        buttons,
        notice: false,
    }
}

pub(crate) fn command_card(
    ctx: &mut Ctx,
    key: &str,
    entry_id: &str,
    card: &CommandCard,
    live_model: Option<&str>,
) -> RowCore {
    let mut b = Builder {
        ctx,
        copy: Vec::new(),
    };
    let spec = if card.pending {
        Spec {
            icon: "apple.terminal",
            title: card.command.clone(),
            subtitle: Some("Asking Claude Code…".into()),
            pill: None,
            items: Vec::new(),
            buttons: Vec::new(),
            notice: false,
        }
    } else if let Some(name) = cards::terminal_only(card) {
        notice(&mut b, name)
    } else {
        let spec = match (card.error.as_deref(), card.command.as_str()) {
            (Some(_), _) => None,
            (None, "/context") => context(&mut b, card),
            (None, "/usage") => usage(&mut b, card),
            (None, "/model") => Some(model(&mut b, card, live_model)),
            (None, "/mcp") => mcp(&mut b, card),
            _ => None,
        };
        spec.unwrap_or_else(|| generic(&mut b, card))
    };
    finish(b, key, entry_id, spec)
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
    let buttons = vec![b.button("Reopen", CardAction::OpenClaudeConfig, false)];
    let spec = Spec {
        icon: "gearshape",
        title: panel::SUMMARY_PREFIX.trim_end_matches([' ', '·']).into(),
        subtitle: None,
        pill: None,
        items,
        buttons,
        notice: false,
    };
    Some(finish(b, key, entry_id, spec))
}

fn finish(mut b: Builder<'_, '_>, key: &str, entry_id: &str, spec: Spec) -> RowCore {
    let title = b.text(&spec.title, Weight::Medium, ColorRole::Text);
    let subtitle = spec
        .subtitle
        .as_deref()
        .map(|s| b.text(s, Weight::Regular, ColorRole::TextSecondary));
    let pill = spec
        .pill
        .as_deref()
        .map(|p| b.text(p, Weight::Regular, ColorRole::TextSecondary));
    RowCore {
        key: row_key(key),
        version: next_version(),
        kind: RowKind::Card,
        entry_id: Arc::from(entry_id),
        copy_text: b.copy.join("\n"),
        content: Content::Command(CommandCardView {
            icon: spec.icon,
            title,
            subtitle,
            pill,
            items: spec.items,
            buttons: spec.buttons,
            notice: spec.notice,
        }),
    }
}

fn width(t: &PText) -> f32 {
    t.p.max_content_width().ceil()
}

pub(crate) fn place_command_card(
    card: &CommandCardView,
    px: Px,
    x: f32,
    top: f32,
    cw: f32,
    out: Option<&mut DisplayBuilder>,
) -> f32 {
    let h = layout(card, px, x, top, cw, None);
    if let Some(o) = out {
        let fill = if card.notice {
            ColorRole::ChipBackground
        } else {
            ColorRole::CardBackground
        };
        o.fill(x, top, cw, h, px.v(14.0), fill);
        o.hairline(x, top, cw, h, px.v(14.0), ColorRole::CardBorder);
        layout(card, px, x, top, cw, Some(o));
    }
    h
}

fn place_buttons(
    buttons: &[CardButton],
    px: Px,
    x: f32,
    y: f32,
    w: f32,
    out: &mut Option<&mut DisplayBuilder>,
) -> f32 {
    if buttons.is_empty() {
        return 0.0;
    }
    let bh = px.v(BUTTON_H);
    let gap = px.v(GAP);
    let (mut bx, mut by) = (x, y);
    for b in buttons {
        let bw = (width(&b.label) + px.v(BUTTON_PAD) * 2.0).min(w);
        if bx > x && bx + bw > x + w {
            bx = x;
            by += bh + gap;
        }
        if let Some(o) = out.as_deref_mut() {
            if b.primary {
                o.fill(bx, by, bw, bh, bh / 2.0, ColorRole::AccentWash);
            }
            o.hairline(
                bx,
                by,
                bw,
                bh,
                bh / 2.0,
                if b.primary {
                    ColorRole::Accent
                } else {
                    ColorRole::ButtonBorder
                },
            );
            place_text_lines(
                &b.label,
                bx + px.v(BUTTON_PAD),
                by + (bh - b.label.lh) / 2.0,
                (bw - px.v(BUTTON_PAD) * 2.0).max(1.0),
                1,
                px,
                o,
            );
            o.widget(
                WidgetKind::CardButton {
                    action: b.action.clone(),
                },
                (bx, by, bw, bh),
                Some(b.title.clone()),
            );
        }
        bx += bw + gap;
    }
    by + bh - y
}

fn layout(
    card: &CommandCardView,
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
    let pill_w = card
        .pill
        .as_ref()
        .map_or(0.0, |p| (width(p) + px.v(14.0)).min(inner_w / 2.0));
    let trailing = if pill_w > 0.0 {
        pill_w + px.v(GAP)
    } else {
        0.0
    };
    let title_w = (x + cw - pad - title_x - trailing).max(1.0);
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
        if let Some(pill) = &card.pill {
            let ph = pill.lh + px.v(4.0);
            let left = x + cw - pad - pill_w;
            o.hairline(
                left,
                y + (card.title.lh - ph) / 2.0,
                pill_w,
                ph,
                ph / 2.0,
                ColorRole::ButtonBorder,
            );
            place_text_lines(
                pill,
                left + px.v(7.0),
                y + (card.title.lh - pill.lh) / 2.0,
                pill_w - px.v(14.0),
                1,
                px,
                o,
            );
        }
    }
    y += place_text(&card.title, title_x, y, title_w, out.as_deref_mut());
    if let Some(sub) = &card.subtitle {
        y += place_text(sub, title_x, y, title_w, out.as_deref_mut());
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
        y += place_buttons(&card.buttons, px, inner_x, y, inner_w, &mut out);
    }
    y + pad - top
}

fn place_line(t: &PText, x: f32, y: f32, w: f32, px: Px, out: &mut Option<&mut DisplayBuilder>) {
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
        Item::Text(t) => place_text(t, x, y, w, out.as_deref_mut()),
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
            place_line(value, x + w - vw, y, vw, px, out);
            let track_y = y + lh + px.v(4.0);
            let track = px.v(TRACK);
            if let Some(o) = out.as_deref_mut() {
                o.fill(x, track_y, w, track, track / 2.0, ColorRole::ChipBackground);
                if *fill > 0.0 {
                    let color = if *warning {
                        ColorRole::Warning
                    } else {
                        ColorRole::Accent
                    };
                    o.fill(x, track_y, (w * fill).max(track), track, track / 2.0, color);
                }
            }
            let mut h = lh + px.v(4.0) + track;
            if let Some(foot) = foot {
                h += px.v(4.0);
                h += place_text(foot, x, y + h, w, out.as_deref_mut());
            }
            h
        }
        Item::Stack(segments) => {
            let h = px.v(STACK);
            if let Some(o) = out.as_deref_mut() {
                o.fill(x, y, w, h, h / 2.0, ColorRole::ChipBackground);
                let mut sx = x;
                for (fraction, color) in segments {
                    let sw = (w * fraction).min(x + w - sx).max(0.0);
                    if sw > 0.0 {
                        o.fill(sx, y, sw, h, 0.0, *color);
                    }
                    sx += sw + px.v(1.0);
                }
            }
            h
        }
        Item::Legend { color, name, value } => {
            let dot = px.v(DOT);
            let vw = width(value).min(w / 2.0);
            let nx = x + dot + px.v(GAP);
            let nw = (w - vw - dot - px.v(GAP) * 2.0).max(1.0);
            let nh = place_text(name, nx, y, nw, out.as_deref_mut());
            place_line(value, x + w - vw, y, vw, px, out);
            if let Some(o) = out.as_deref_mut() {
                o.fill(x, y + (name.lh - dot) / 2.0, dot, dot, px.v(2.0), *color);
            }
            nh
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
            place_line(lead, x, y, lead_w, px, out);
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
                    let bw = (bar_w * fill).max(track);
                    o.fill(bx, track_y, bw, track, track / 2.0, ColorRole::TextTertiary);
                }
            }
            th.max(lead.lh)
        }
        Item::Status { name, state, ok } => {
            let dot = px.v(DOT);
            let sw = width(state).min(w / 2.0);
            let nx = x + dot + px.v(GAP);
            let nw = (w - sw - dot - px.v(GAP) * 2.0).max(1.0);
            let nh = place_text(name, nx, y, nw, out.as_deref_mut());
            place_line(state, x + w - sw, y, sw, px, out);
            if let Some(o) = out.as_deref_mut() {
                let color = if *ok {
                    ColorRole::Success
                } else {
                    ColorRole::TextTertiary
                };
                o.fill(x, y + (name.lh - dot) / 2.0, dot, dot, dot / 2.0, color);
            }
            nh
        }
        Item::Choices(buttons) => place_buttons(buttons, px, x, y, w, out),
    }
}

pub(crate) fn heap_bytes(card: &CommandCardView) -> usize {
    let buttons = |b: &[CardButton]| b.iter().map(|b| b.label.p.heap_bytes()).sum::<usize>();
    let item = |item: &Item| match item {
        Item::Meter {
            label, value, foot, ..
        } => {
            label.p.heap_bytes()
                + value.p.heap_bytes()
                + foot.as_ref().map_or(0, |f| f.p.heap_bytes())
        }
        Item::Stack(_) => 0,
        Item::Legend { name, value, .. } => name.p.heap_bytes() + value.p.heap_bytes(),
        Item::Bar { lead, text, .. } => lead.p.heap_bytes() + text.p.heap_bytes(),
        Item::Status { name, state, .. } => name.p.heap_bytes() + state.p.heap_bytes(),
        Item::Choices(b) => buttons(b),
        Item::Text(t) => t.p.heap_bytes(),
    };
    card.title.p.heap_bytes()
        + card.subtitle.as_ref().map_or(0, |s| s.p.heap_bytes())
        + card.pill.as_ref().map_or(0, |s| s.p.heap_bytes())
        + card.items.iter().map(item).sum::<usize>()
        + buttons(&card.buttons)
}
