use std::sync::Arc;

use zeron_client::computer::{self, ComputerNotice, ConsentOutcome};
use zeron_proto::{UserInputAnswer, UserInputQuestion};
use zeron_text::WhiteSpace;

use super::display::{CardAction, ColorRole, DisplayBuilder, WidgetKind};
use super::markdown::{Ctx, PText, Px, place_text, prepare_plain};
use super::rows::{Content, RowCore, RowKind, next_version, place_text_lines, row_key};
use super::style::{Family, TYPE, Weight};

const PAD: f32 = 12.0;
const TILE: f32 = 28.0;
const ICON: f32 = 14.0;
const GAP: f32 = 10.0;
const LINE_GAP: f32 = 3.0;
const FOOTER_GAP: f32 = 8.0;
const BUTTON_H: f32 = 36.0;
const BUTTON_PAD: f32 = 14.0;
const BUTTON_GAP: f32 = 8.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Tone {
    Info,
    Success,
    Warning,
}

impl Tone {
    fn accent(self) -> ColorRole {
        match self {
            Tone::Info => ColorRole::TextSecondary,
            Tone::Success => ColorRole::Success,
            Tone::Warning => ColorRole::Warning,
        }
    }

    fn tile(self) -> ColorRole {
        match self {
            Tone::Info => ColorRole::CardTile,
            Tone::Success => ColorRole::SuccessTile,
            Tone::Warning => ColorRole::WarningTile,
        }
    }
}

pub(crate) struct CardButton {
    pub label: PText,
    pub title: String,
    pub primary: bool,
    pub action: CardAction,
}

pub(crate) struct Card {
    pub tone: Tone,
    pub icon: &'static str,
    pub title: PText,
    pub lines: Vec<PText>,
    pub buttons: Vec<CardButton>,
    pub status: Option<PText>,
}

struct Spec<'a> {
    tone: Tone,
    icon: &'static str,
    title: &'a str,
    lines: Vec<&'a str>,
    buttons: Vec<(String, bool, CardAction)>,
    status: Option<&'static str>,
}

fn card_row(ctx: &mut Ctx, key: &str, entry_id: &str, spec: Spec) -> RowCore {
    let (size, lh) = TYPE.small;
    let lh = ctx.typo.px(lh);
    let regular = ctx.typo.style(Family::Sans, Weight::Regular, false, size);
    let medium = ctx.typo.style(Family::Sans, Weight::Medium, false, size);
    let title = prepare_plain(
        ctx,
        spec.title,
        medium,
        lh,
        ColorRole::Text,
        WhiteSpace::Normal,
    );
    let lines = spec
        .lines
        .iter()
        .map(|line| {
            prepare_plain(
                ctx,
                line,
                regular,
                lh,
                ColorRole::TextSecondary,
                WhiteSpace::PreWrap,
            )
        })
        .collect();
    let buttons = spec
        .buttons
        .into_iter()
        .map(|(title, primary, action)| CardButton {
            label: prepare_plain(
                ctx,
                &title,
                medium,
                lh,
                if primary {
                    ColorRole::OnSolid
                } else {
                    ColorRole::TextSoft
                },
                WhiteSpace::Pre,
            ),
            title,
            primary,
            action,
        })
        .collect();
    let status = spec
        .status
        .map(|status| prepare_plain(ctx, status, medium, lh, spec.tone.accent(), WhiteSpace::Pre));
    let mut copy = vec![spec.title];
    copy.extend(spec.lines.iter().copied());
    copy.extend(spec.status);
    RowCore {
        key: row_key(key),
        version: next_version(),
        kind: RowKind::Card,
        entry_id: Arc::from(entry_id),
        content: Content::Card(Card {
            tone: spec.tone,
            icon: spec.icon,
            title,
            lines,
            buttons,
            status,
        }),
        copy_text: copy.join("\n"),
    }
}

pub(crate) fn notice_row(
    ctx: &mut Ctx,
    key: &str,
    entry_id: &str,
    notice: ComputerNotice,
    text: &str,
    hint: Option<&str>,
) -> RowCore {
    let (tone, icon) = match notice {
        ComputerNotice::Disclosure => (Tone::Info, "cursorarrow"),
        ComputerNotice::EscapeStop => (Tone::Warning, "stop.circle"),
        ComputerNotice::SetupRequired
        | ComputerNotice::Unavailable
        | ComputerNotice::ControlDenied => (Tone::Warning, "desktopcomputer"),
    };
    let lines = match notice {
        ComputerNotice::Disclosure | ComputerNotice::EscapeStop => Vec::new(),
        _ => hint.into_iter().collect(),
    };
    let buttons = if notice == ComputerNotice::ControlDenied {
        vec![(
            computer::DENIED_ENABLE.to_owned(),
            false,
            CardAction::EnableComputer,
        )]
    } else {
        Vec::new()
    };
    card_row(
        ctx,
        key,
        entry_id,
        Spec {
            tone,
            icon,
            title: text,
            lines,
            buttons,
            status: None,
        },
    )
}

pub(crate) fn denied_row(ctx: &mut Ctx, key: &str, entry_id: &str) -> RowCore {
    notice_row(
        ctx,
        key,
        entry_id,
        ComputerNotice::ControlDenied,
        computer::DENIED_TITLE,
        Some(computer::DENIED_HINT),
    )
}

pub(crate) fn consent_row(
    ctx: &mut Ctx,
    key: &str,
    entry_id: &str,
    request_id: &str,
    question: &UserInputQuestion,
    resolved: bool,
    answers: Option<&[UserInputAnswer]>,
) -> RowCore {
    let scope = computer::consent_scope(question);
    let app = computer::consent_app(question);
    let outcome = computer::consent_outcome(resolved, answers);
    let detail = computer::consent_detail(scope, app, &question.question);
    let title = computer::consent_card_title(scope, app);
    let description = computer::consent_allow_description(scope, app);
    let mut lines: Vec<&str> = Vec::new();
    if !detail.is_empty() {
        lines.push(&detail);
    }
    let buttons = match outcome {
        Some(_) => Vec::new(),
        None => {
            lines.push(&description);
            computer::consent_choices(scope, app)
                .into_iter()
                .map(|choice| {
                    let action = CardAction::Consent {
                        request_id: request_id.to_owned(),
                        question_id: question.id.clone(),
                        answer: choice.answer,
                    };
                    (choice.title, choice.primary, action)
                })
                .collect()
        }
    };
    let tone = match outcome {
        None => Tone::Warning,
        Some(ConsentOutcome::Allowed | ConsentOutcome::AlwaysAllowed) => Tone::Success,
        Some(_) => Tone::Info,
    };
    card_row(
        ctx,
        key,
        entry_id,
        Spec {
            tone,
            icon: "desktopcomputer",
            title: &title,
            lines,
            buttons,
            status: outcome.map(computer::consent_outcome_label),
        },
    )
}

fn footer(card: &Card, px: Px, width: f32) -> (bool, f32) {
    let bh = px.v(BUTTON_H);
    let gap = px.v(BUTTON_GAP);
    let widths: Vec<f32> = card.buttons.iter().map(|b| button_width(b, px)).collect();
    let status_w = card.status.as_ref().map(|s| s.p.max_content_width());
    let items = widths.len() + status_w.is_some() as usize;
    if items == 0 {
        return (true, 0.0);
    }
    let row_w: f32 =
        widths.iter().sum::<f32>() + status_w.unwrap_or(0.0) + gap * (items - 1) as f32;
    let row_h = if widths.is_empty() {
        card.status.as_ref().map_or(0.0, |s| s.lh)
    } else {
        bh
    };
    if row_w <= width || items == 1 {
        return (true, row_h);
    }
    let status_h = card.status.as_ref().map_or(0.0, |s| s.lh + gap);
    (
        false,
        widths.len() as f32 * bh + (widths.len().saturating_sub(1)) as f32 * gap + status_h,
    )
}

fn button_width(b: &CardButton, px: Px) -> f32 {
    b.label.p.max_content_width().ceil() + px.v(BUTTON_PAD) * 2.0
}

pub(crate) fn place_card(
    card: &Card,
    px: Px,
    x: f32,
    top: f32,
    cw: f32,
    mut out: Option<&mut DisplayBuilder>,
) -> f32 {
    let pad = px.v(PAD);
    let tile = px.v(TILE);
    let tx = x + pad + tile + px.v(GAP);
    let tw = (x + cw - pad - tx).max(1.0);
    let mut y = top + pad;
    y += place_text(&card.title, tx, y, tw, out.as_deref_mut());
    for line in &card.lines {
        y += px.v(LINE_GAP);
        y += place_text(line, tx, y, tw, out.as_deref_mut());
    }
    let (inline, footer_h) = footer(card, px, tw);
    let footer_y = y + px.v(FOOTER_GAP);
    if footer_h > 0.0 {
        y = footer_y + footer_h;
    }
    let h = (y - top + pad).max(pad * 2.0 + tile);
    let Some(o) = out else { return h };
    o.fill(x, top, cw, h, px.v(12.0), ColorRole::CardBackground);
    o.hairline(x, top, cw, h, px.v(12.0), ColorRole::CardBorder);
    o.fill(x + pad, top + pad, tile, tile, px.v(8.0), card.tone.tile());
    let is = px.v(ICON);
    o.widget(
        WidgetKind::Icon {
            name: card.icon.into(),
            color: card.tone.accent(),
        },
        (
            x + pad + (tile - is) / 2.0,
            top + pad + (tile - is) / 2.0,
            is,
            is,
        ),
        None,
    );
    let bh = px.v(BUTTON_H);
    let gap = px.v(BUTTON_GAP);
    let (mut bx, mut by) = (tx, footer_y);
    for b in &card.buttons {
        let bw = button_width(b, px).min(tw);
        if b.primary {
            o.fill(bx, by, bw, bh, bh / 2.0, ColorRole::Text);
        } else {
            o.hairline(bx, by, bw, bh, bh / 2.0, ColorRole::ButtonBorder);
        }
        let pad_x = px.v(BUTTON_PAD);
        place_text_lines(
            &b.label,
            bx + pad_x,
            by + (bh - b.label.lh) / 2.0,
            (bw - pad_x * 2.0).max(1.0),
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
        if inline {
            bx += bw + gap;
        } else {
            by += bh + gap;
        }
    }
    if let Some(status) = &card.status {
        let sy = if card.buttons.is_empty() {
            by
        } else if inline {
            by + (bh - status.lh) / 2.0
        } else {
            by
        };
        place_text(status, bx, sy, status.p.max_content_width() + 1.0, Some(o));
    }
    h
}

pub(crate) fn heap_bytes(card: &Card) -> usize {
    card.title.p.heap_bytes()
        + card.lines.iter().map(|l| l.p.heap_bytes()).sum::<usize>()
        + card
            .buttons
            .iter()
            .map(|b| b.label.p.heap_bytes())
            .sum::<usize>()
        + card.status.as_ref().map_or(0, |s| s.p.heap_bytes())
}
