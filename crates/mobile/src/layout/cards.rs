use std::sync::Arc;

use zeron_proto::computer::ComputerConsentApp;
use zeron_proto::computer::cards::{
    COMPUTER_DENIED_ACTION, COMPUTER_DENIED_HINT, COMPUTER_DENIED_TITLE, COMPUTER_FOREGROUND_ALLOW,
    COMPUTER_FOREGROUND_DECLINE, COMPUTER_FOREGROUND_DETAIL, ComputerNotice, ConsentChoice,
    consent_app, consent_card_title, consent_choice_key, consent_choices, consent_detail,
    consent_event_label, consent_outcome, consent_scope, elevated_risk, foreground_outcome_label,
    foreground_title, revoked_label,
};
use zeron_proto::{UserInputAnswer, UserInputKind, UserInputQuestion};
use zeron_text::WhiteSpace;

use super::display::{CardAction, ColorRole, DisplayBuilder, WidgetKind};
use super::markdown::{Ctx, PText, Px, place_text, prepare_plain};
use super::rows::{Content, RowCore, RowKind, chip_row, next_version, place_text_lines, row_key};
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
const RISK_PAD: f32 = 8.0;
const RISK_GAP: f32 = 2.0;
const RISK_TITLE: &str = "Elevated Risk";

pub(crate) struct CardButton {
    pub label: PText,
    pub title: String,
    pub primary: bool,
    pub action: CardAction,
}

pub(crate) struct Risk {
    pub title: PText,
    pub body: PText,
}

pub(crate) struct Card {
    pub icon: &'static str,
    pub risk: Option<Risk>,
    pub title: PText,
    pub lines: Vec<PText>,
    pub buttons: Vec<CardButton>,
}

struct Spec<'a> {
    icon: &'static str,
    risk: Option<&'a str>,
    title: &'a str,
    lines: Vec<&'a str>,
    buttons: Vec<(String, bool, CardAction)>,
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
    let risk = spec.risk.map(|body| Risk {
        title: prepare_plain(
            ctx,
            RISK_TITLE,
            medium,
            lh,
            ColorRole::Warning,
            WhiteSpace::Pre,
        ),
        body: prepare_plain(
            ctx,
            body,
            regular,
            lh,
            ColorRole::TextSecondary,
            WhiteSpace::Normal,
        ),
    });
    let mut copy = Vec::new();
    if let Some(body) = spec.risk {
        copy.extend([RISK_TITLE, body]);
    }
    copy.push(spec.title);
    copy.extend(spec.lines.iter().copied());
    RowCore {
        key: row_key(key),
        version: next_version(),
        kind: RowKind::Card,
        entry_id: Arc::from(entry_id),
        content: Content::Card(Card {
            icon: spec.icon,
            risk,
            title,
            lines,
            buttons,
        }),
        copy_text: copy.join("\n"),
    }
}

fn event_row(ctx: &mut Ctx, entry_id: &str, part_id: &str, warning: bool, text: &str) -> RowCore {
    let (icon, color) = if warning {
        ("stop.circle", ColorRole::Warning)
    } else {
        ("cursorarrow", ColorRole::TextSecondary)
    };
    chip_row(ctx, entry_id, part_id, icon, color, text, WhiteSpace::Normal)
}

pub(crate) fn notice_row(
    ctx: &mut Ctx,
    key: &str,
    entry_id: &str,
    notice: ComputerNotice,
    text: &str,
    hint: Option<&str>,
) -> Option<RowCore> {
    let part = key.rsplit('#').next().unwrap_or(key);
    let lines = match notice {
        ComputerNotice::Disclosure => return None,
        ComputerNotice::Revoked => {
            return Some(event_row(
                ctx,
                entry_id,
                part,
                true,
                &revoked_label(text, hint),
            ));
        }
        ComputerNotice::Paused | ComputerNotice::Foreground | ComputerNotice::LeftOpen => {
            return Some(event_row(ctx, entry_id, part, false, text));
        }
        ComputerNotice::ControlDenied => vec![COMPUTER_DENIED_HINT],
        ComputerNotice::SetupRequired | ComputerNotice::Unavailable => hint.into_iter().collect(),
    };
    let buttons = if notice == ComputerNotice::ControlDenied {
        vec![(
            COMPUTER_DENIED_ACTION.to_owned(),
            false,
            CardAction::EnableComputer,
        )]
    } else {
        Vec::new()
    };
    Some(card_row(
        ctx,
        key,
        entry_id,
        Spec {
            icon: "cursorarrow",
            risk: None,
            title: text,
            lines,
            buttons,
        },
    ))
}

pub(crate) fn denied_row(ctx: &mut Ctx, key: &str, entry_id: &str) -> RowCore {
    notice_row(
        ctx,
        key,
        entry_id,
        ComputerNotice::ControlDenied,
        COMPUTER_DENIED_TITLE,
        None,
    )
    .expect("a denied notice is a card")
}

fn answer(request_id: &str, question_id: &str, label: &str, identifier: String) -> CardAction {
    CardAction::Consent {
        request_id: request_id.to_owned(),
        question_id: question_id.to_owned(),
        answer: label.to_owned(),
        identifier,
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn consent_row(
    ctx: &mut Ctx,
    key: &str,
    entry_id: &str,
    part_id: &str,
    request_id: &str,
    question: &UserInputQuestion,
    resolved: bool,
    answers: Option<&[UserInputAnswer]>,
) -> RowCore {
    let scope = consent_scope(question);
    let app = consent_app(question);
    if let Some(outcome) = consent_outcome(resolved, answers) {
        let label = consent_event_label(outcome, scope, app.as_ref());
        return event_row(ctx, entry_id, part_id, false, &label);
    }
    let detail = consent_detail(question);
    let title = consent_card_title(scope, app.as_ref());
    let buttons = consent_choices(scope, app.as_ref())
        .into_iter()
        .rev()
        .map(|(choice, label)| {
            let identifier = format!("computer-consent-{}", consent_choice_key(&choice));
            let action = answer(request_id, &question.id, choice.answer_label(), identifier);
            (label, choice == ConsentChoice::Allow, action)
        })
        .collect();
    card_row(
        ctx,
        key,
        entry_id,
        Spec {
            icon: "cursorarrow",
            risk: app.as_ref().and_then(elevated_risk),
            title: &title,
            lines: (!detail.is_empty())
                .then_some(detail.as_str())
                .into_iter()
                .collect(),
            buttons,
        },
    )
}

pub(crate) fn foreground_question(
    questions: &[UserInputQuestion],
) -> Option<(&UserInputQuestion, Option<&ComputerConsentApp>)> {
    questions.iter().find_map(|question| match &question.kind {
        Some(UserInputKind::Foreground { app }) => Some((question, app.as_ref())),
        _ => None,
    })
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn foreground_row(
    ctx: &mut Ctx,
    key: &str,
    entry_id: &str,
    part_id: &str,
    request_id: &str,
    questions: &[UserInputQuestion],
    resolved: bool,
    answers: Option<&[UserInputAnswer]>,
) -> Option<RowCore> {
    if let Some(label) = foreground_outcome_label(questions, answers) {
        return Some(event_row(ctx, entry_id, part_id, false, &label));
    }
    if resolved {
        return None;
    }
    let (question, app) = foreground_question(questions)?;
    let title = foreground_title(app.map(|app| app.name.as_str()));
    let buttons = [
        (COMPUTER_FOREGROUND_DECLINE, false, "decline"),
        (COMPUTER_FOREGROUND_ALLOW, true, "allow"),
    ]
    .into_iter()
    .map(|(label, primary, key)| {
        let identifier = format!("computer-foreground-{key}");
        (
            label.to_owned(),
            primary,
            answer(request_id, &question.id, label, identifier),
        )
    })
    .collect();
    Some(card_row(
        ctx,
        key,
        entry_id,
        Spec {
            icon: "cursorarrow",
            risk: None,
            title: &title,
            lines: vec![COMPUTER_FOREGROUND_DETAIL],
            buttons,
        },
    ))
}

fn footer(card: &Card, px: Px, width: f32) -> (bool, f32) {
    let bh = px.v(BUTTON_H);
    let gap = px.v(BUTTON_GAP);
    let widths: Vec<f32> = card.buttons.iter().map(|b| button_width(b, px)).collect();
    if widths.is_empty() {
        return (true, 0.0);
    }
    let row_w: f32 = widths.iter().sum::<f32>() + gap * (widths.len() - 1) as f32;
    if row_w <= width || widths.len() == 1 {
        return (true, bh);
    }
    (
        false,
        widths.len() as f32 * bh + (widths.len() - 1) as f32 * gap,
    )
}

fn risk_height(risk: &Risk, px: Px, width: f32) -> f32 {
    let pad = px.v(RISK_PAD);
    let inner = (width - pad * 2.0).max(1.0);
    pad * 2.0
        + place_text(&risk.title, 0.0, 0.0, inner, None)
        + px.v(RISK_GAP)
        + place_text(&risk.body, 0.0, 0.0, inner, None)
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
    let band_w = cw - pad * 2.0;
    let band_h = card
        .risk
        .as_ref()
        .map_or(0.0, |risk| risk_height(risk, px, band_w));
    let body_top = if band_h > 0.0 {
        top + pad + band_h
    } else {
        top
    };
    let mut y = body_top + pad;
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
    let h = (y - top + pad).max(body_top - top + pad * 2.0 + tile);
    let Some(o) = out else { return h };
    o.fill(x, top, cw, h, px.v(12.0), ColorRole::CardBackground);
    o.hairline(x, top, cw, h, px.v(12.0), ColorRole::CardBorder);
    if let Some(risk) = &card.risk {
        let rp = px.v(RISK_PAD);
        let (bx, by) = (x + pad, top + pad);
        o.fill(bx, by, band_w, band_h, px.v(8.0), ColorRole::WarningTile);
        let inner = (band_w - rp * 2.0).max(1.0);
        let ty = by + rp;
        let title_h = place_text(&risk.title, bx + rp, ty, inner, Some(&mut *o));
        place_text(
            &risk.body,
            bx + rp,
            ty + title_h + px.v(RISK_GAP),
            inner,
            Some(&mut *o),
        );
    }
    o.fill(
        x + pad,
        body_top + pad,
        tile,
        tile,
        px.v(8.0),
        ColorRole::WarningTile,
    );
    let is = px.v(ICON);
    o.widget(
        WidgetKind::Icon {
            name: card.icon.into(),
            color: ColorRole::Warning,
        },
        (
            x + pad + (tile - is) / 2.0,
            body_top + pad + (tile - is) / 2.0,
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
        + card
            .risk
            .as_ref()
            .map_or(0, |r| r.title.p.heap_bytes() + r.body.p.heap_bytes())
}
