use super::*;
use crate::shell::update_glyph;
use gpui::{linear_color_stop, linear_gradient, relative};

const CLAUDE_CONFIG: &str = "claude-config";
const COMMAND_CARDS: &str = "claude-command-cards";
const COMPACTION_CARD: &str = "compaction-card";
const COMPACT_AHEAD: &str = "compact-ahead";
const WORKING_TREE: &str = "working-now-tree";
const PHONE_SLASH: &str = "phone-slash-popup";
const PHONE_SUBAGENTS: &str = "phone-subagents-pill";
const PHONE_BACKGROUND: &str = "phone-chat-background";
const RAIL_UPDATE: &str = "rail-update";

pub(super) const VISUALS: [(&str, f32, f32); 9] = [
    (CLAUDE_CONFIG, 7.0, 3.4),
    (COMMAND_CARDS, 6.6, 4.5),
    (COMPACTION_CARD, 7.6, 5.4),
    (COMPACT_AHEAD, 8.0, 2.2),
    (WORKING_TREE, 6.6, 2.0),
    (PHONE_SLASH, 7.0, 2.6),
    (PHONE_SUBAGENTS, 6.6, 2.6),
    (PHONE_BACKGROUND, 7.0, 4.4),
    (RAIL_UPDATE, 6.4, 5.2),
];

pub(super) fn visual(id: &str, theme: &Theme, t: f32, reduced: bool) -> AnyElement {
    match id {
        CLAUDE_CONFIG => config_visual(theme, t),
        COMMAND_CARDS => command_cards_visual(theme, t, reduced),
        COMPACTION_CARD => compaction_visual(theme, t, reduced),
        COMPACT_AHEAD => compact_ahead_visual(theme, t, reduced),
        WORKING_TREE => working_tree_visual(theme, t, reduced),
        PHONE_SLASH => phone_slash_visual(theme, t),
        PHONE_SUBAGENTS => phone_subagents_visual(theme, t, reduced),
        PHONE_BACKGROUND => phone_background_visual(theme, t),
        RAIL_UPDATE => rail_update_visual(theme, t, reduced),
        _ => div().into_any_element(),
    }
}

fn text(size: f32, color: gpui::Hsla, copy: impl Into<SharedString>) -> gpui::Div {
    div()
        .flex_none()
        .whitespace_nowrap()
        .text_size(crate::typography::ui_rems(size))
        .text_color(color)
        .child(copy.into())
}

fn mono(theme: &Theme, size: f32, color: gpui::Hsla, copy: impl Into<SharedString>) -> gpui::Div {
    text(size, color, copy).font_family(theme.font_mono.clone())
}

fn card(theme: &Theme) -> gpui::Div {
    div()
        .rounded(px(10.0))
        .border_1()
        .border_color(theme.border)
        .bg(crate::settings::widgets::block_fill(theme))
}

fn pill(label: &'static str, tone: gpui::Hsla) -> gpui::Div {
    div()
        .flex_none()
        .h(px(18.0))
        .px(px(7.0))
        .flex()
        .items_center()
        .rounded_full()
        .border_1()
        .border_color(tone.opacity(0.55))
        .text_size(crate::typography::ui_rems(10.5))
        .text_color(tone)
        .child(label)
}

fn glyph(path: &'static str, size: f32, color: gpui::Hsla) -> gpui::Svg {
    icon(path).size(px(size)).flex_none().text_color(color)
}

fn phone(theme: &Theme, width: f32, height: f32) -> gpui::Div {
    div()
        .flex_none()
        .w(px(width))
        .h(px(height))
        .relative()
        .overflow_hidden()
        .rounded_t(px(26.0))
        .border_t_1()
        .border_l_1()
        .border_r_1()
        .border_color(theme.border_strong)
        .bg(theme.surface)
}

fn caret(theme: &Theme, t: f32) -> gpui::Div {
    div()
        .flex_none()
        .w(px(1.5))
        .h(px(14.0))
        .when((t * 2.0).fract() < 0.5, |el| el.bg(theme.text))
}

fn typed(copy: &'static str, t: f32, start: f32, per_char: f32) -> &'static str {
    let count = (((t - start) / per_char).max(0.0) as usize).min(copy.len());
    &copy[..count]
}

fn config_visual(theme: &Theme, t: f32) -> AnyElement {
    const FLIP_AT: f32 = 1.6;
    const SAVED_AT: f32 = 2.5;
    const CLOSE_AT: f32 = 4.0;
    let flipped = t >= FLIP_AT;
    let close = ramp(t, CLOSE_AT, 180.0, motion::EASE_IN);
    let summary = ramp(t, CLOSE_AT + 0.25, 240.0, motion::EASE_OUT);
    let selected = if t >= 1.0 { 0 } else { 1 };
    let row = |ix: usize, label: &'static str, id: &'static str, value: gpui::AnyElement| {
        let on = ix == selected;
        div()
            .h(px(22.0))
            .px(px(6.0))
            .flex()
            .items_center()
            .gap(px(6.0))
            .rounded(px(5.0))
            .when(on, |el| el.bg(theme.accent.opacity(0.16)))
            .child(text(11.0, theme.accent, if on { "\u{276F}" } else { " " }).w(px(8.0)))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .overflow_hidden()
                    .flex()
                    .items_baseline()
                    .gap(px(5.0))
                    .child(text(11.5, theme.text, label))
                    .child(mono(theme, 9.5, theme.text_faint, id)),
            )
            .child(value)
    };
    let saving = if t < FLIP_AT {
        "Claude Code defaults"
    } else if t < SAVED_AT {
        "Saving\u{2026}"
    } else {
        "1 change, saved"
    };
    let tabs = ["Status", "Config", "Usage", "Stats"].map(|tab| {
        let on = tab == "Config";
        div()
            .flex_none()
            .pb(px(3.0))
            .border_b_2()
            .border_color(if on {
                theme.accent
            } else {
                gpui::transparent_black()
            })
            .child(text(
                11.5,
                if on { theme.text } else { theme.text_muted },
                tab,
            ))
    });
    let chip = |label: &'static str, count: &'static str, on: bool| {
        div()
            .flex_none()
            .h(px(18.0))
            .px(px(7.0))
            .flex()
            .items_center()
            .gap(px(4.0))
            .rounded_full()
            .border_1()
            .border_color(if on { theme.accent } else { theme.border })
            .when(on, |el| el.bg(theme.accent.opacity(0.14)))
            .child(text(
                10.5,
                if on { theme.accent } else { theme.text_muted },
                label,
            ))
            .child(text(10.5, theme.text_faint, count))
    };
    let panel = card(theme)
        .w_full()
        .p(px(8.0))
        .flex()
        .flex_col()
        .gap(px(5.0))
        .relative()
        .top(px(4.0 * close))
        .opacity(1.0 - close)
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(12.0))
                .child(text(10.5, theme.text_faint, "Claude Code settings"))
                .children(tabs),
        )
        .child(
            div()
                .flex()
                .gap(px(5.0))
                .overflow_hidden()
                .child(chip("All", "49", true))
                .child(chip("Affects Zeron chats", "20", false))
                .child(chip("Terminal only", "29", false)),
        )
        .child(row(
            0,
            "Auto-compact",
            "autoCompact",
            div()
                .flex()
                .items_center()
                .gap(px(6.0))
                .child(crate::settings::widgets::toggle_switch(
                    theme,
                    !flipped,
                    "whats-new-config-auto-compact",
                ))
                .child(mono(
                    theme,
                    11.0,
                    theme.text,
                    if flipped { "false" } else { "true" },
                ))
                .when(flipped, |el| {
                    el.child(div().size(px(6.0)).rounded_full().bg(theme.accent))
                })
                .into_any_element(),
        ))
        .child(row(
            1,
            "Theme",
            "theme",
            mono(theme, 11.0, theme.text, "dark \u{203A}").into_any_element(),
        ))
        .child(row(
            2,
            "Show tips",
            "tips",
            div()
                .flex()
                .items_center()
                .gap(px(6.0))
                .child(crate::settings::widgets::toggle_switch(
                    theme,
                    true,
                    "whats-new-config-tips",
                ))
                .child(mono(theme, 11.0, theme.text, "true"))
                .child(
                    div()
                        .px(px(5.0))
                        .rounded(px(4.0))
                        .bg(crate::theme::ink(0.06))
                        .child(text(10.0, theme.text_muted, "Terminal")),
                )
                .into_any_element(),
        ))
        .child(
            div()
                .flex()
                .justify_end()
                .child(text(10.5, theme.text_faint, saving)),
        );
    let summary_row = div()
        .w_full()
        .h(px(32.0))
        .px(px(10.0))
        .flex()
        .items_center()
        .gap(px(7.0))
        .rounded(px(8.0))
        .bg(crate::theme::ink(0.03))
        .relative()
        .top(px((1.0 - summary) * 6.0))
        .opacity(summary)
        .child(glyph(icons::SETTINGS, 13.0, theme.text_muted))
        .child(text(12.0, theme.text_muted, "Claude Code settings \u{b7}"))
        .child(text(12.0, theme.text, "Auto-compact").font_weight(gpui::FontWeight::MEDIUM))
        .child(text(12.0, theme.text_muted, "\u{2192} false"))
        .overflow_hidden()
        .child(div().flex_1())
        .child(
            div()
                .h(px(22.0))
                .px(px(9.0))
                .flex()
                .items_center()
                .rounded(px(7.0))
                .border_1()
                .border_color(theme.border)
                .child(text(11.5, theme.text, "Reopen")),
        );
    div()
        .relative()
        .w_full()
        .max_w(px(390.0))
        .mx(px(10.0))
        .h(px(150.0))
        .child(div().absolute().top_0().left_0().right_0().child(panel))
        .when(summary > 0.0, |el| {
            el.child(
                div()
                    .absolute()
                    .top(px(56.0))
                    .left_0()
                    .right_0()
                    .child(summary_row),
            )
        })
        .into_any_element()
}

fn command_cards_visual(theme: &Theme, t: f32, reduced: bool) -> AnyElement {
    const SENT_AT: f32 = 1.1;
    const ANSWER_AT: f32 = 2.3;
    const SEGMENTS: [(&str, f32, &str); 4] = [
        ("System prompt", 0.10, "6.1k"),
        ("MCP tools", 0.16, "9.8k"),
        ("Skills", 0.06, "3.4k"),
        ("Messages", 0.28, "16.9k"),
    ];
    let tone = |ix: usize| match ix {
        0 => theme.accent,
        1 => theme.syntax.function,
        2 => theme.success,
        _ => theme.danger,
    };
    let working = div()
        .h(px(22.0))
        .flex()
        .items_center()
        .gap(px(7.0))
        .child(running(theme, reduced))
        .child(text(
            12.0,
            theme.text_muted,
            "Writing the checkout tests\u{2026}",
        ));
    let body: gpui::AnyElement = if t < SENT_AT {
        div()
            .h(px(30.0))
            .px(px(10.0))
            .flex()
            .items_center()
            .gap(px(1.0))
            .rounded(px(10.0))
            .border_1()
            .border_color(theme.border_strong)
            .child(mono(
                theme,
                12.0,
                theme.text,
                typed("/context", t, 0.3, 0.08),
            ))
            .child(caret(theme, t))
            .into_any_element()
    } else if t < ANSWER_AT {
        rise(
            div()
                .h(px(30.0))
                .px(px(10.0))
                .flex()
                .items_center()
                .gap(px(7.0))
                .rounded(px(10.0))
                .border_1()
                .border_color(theme.border)
                .child(running(theme, reduced))
                .child(mono(theme, 12.0, theme.text, "/context"))
                .child(text(
                    12.0,
                    theme.text_muted,
                    "Running in Claude Code\u{2026}",
                ))
                .child(div().flex_1())
                .child(text(
                    11.0,
                    theme.text_faint,
                    format!("{}s", (t - SENT_AT).floor() as u32 + 1),
                )),
            t,
            SENT_AT,
        )
        .into_any_element()
    } else {
        let grow = ramp(t, ANSWER_AT + 0.1, 600.0, motion::EASE_OUT);
        rise(
            card(theme)
                .p(px(9.0))
                .flex()
                .flex_col()
                .gap(px(7.0))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(6.0))
                        .child(glyph(icons::SPEEDOMETER, 14.0, theme.text_muted))
                        .child(
                            text(12.5, theme.text, "Context")
                                .font_weight(gpui::FontWeight::SEMIBOLD),
                        )
                        .child(text(11.5, theme.text_muted, "Claude Code \u{b7} Opus"))
                        .child(div().flex_1())
                        .child(pill("36.2k / 200k \u{b7} 18.1%", theme.text_muted)),
                )
                .child(
                    div()
                        .h(px(10.0))
                        .flex()
                        .gap(px(1.0))
                        .rounded(px(5.0))
                        .overflow_hidden()
                        .children(SEGMENTS.iter().enumerate().map(|(ix, (_, share, _))| {
                            div().h_full().w(px(370.0 * share * grow)).bg(tone(ix))
                        })),
                )
                .child(
                    div()
                        .flex()
                        .flex_wrap()
                        .gap_x(px(14.0))
                        .gap_y(px(3.0))
                        .children(SEGMENTS.iter().enumerate().map(|(ix, (name, _, tokens))| {
                            div()
                                .flex()
                                .items_center()
                                .gap(px(5.0))
                                .child(div().size(px(8.0)).rounded(px(2.0)).bg(tone(ix)))
                                .child(text(11.0, theme.text_muted, *name))
                                .child(text(11.0, theme.text_faint, *tokens))
                        })),
                ),
            t,
            ANSWER_AT,
        )
        .into_any_element()
    };
    div()
        .w(px(400.0))
        .flex()
        .flex_col()
        .gap(px(8.0))
        .child(working)
        .child(body)
        .child(text(
            11.0,
            theme.text_faint,
            "Answered beside the reply: no turn, no tokens",
        ))
        .into_any_element()
}

fn compaction_visual(theme: &Theme, t: f32, reduced: bool) -> AnyElement {
    const COMPACT_AT: f32 = 1.5;
    const DONE_AT: f32 = 4.0;
    const SLIM_AT: f32 = 4.9;
    let slim = ramp(t, SLIM_AT, 200.0, motion::EASE_OUT);
    let meta = |copy: String| text(11.5, theme.text_faint, copy);
    let head = div().flex().items_center().gap(px(7.0));
    let head = if t < COMPACT_AT {
        head.child(running(theme, reduced))
            .child(
                text(12.5, theme.text, "Checking whether to compact")
                    .font_weight(gpui::FontWeight::SEMIBOLD),
            )
            .child(
                div()
                    .h(px(18.0))
                    .px(px(6.0))
                    .flex()
                    .items_center()
                    .gap(px(4.0))
                    .rounded(px(5.0))
                    .bg(crate::theme::ink(0.06))
                    .child(glyph(icons::BOT, 11.0, theme.text_muted))
                    .child(text(10.5, theme.text_muted, "haiku")),
            )
            .child(meta(format!("\u{b7} {:.1}s", (t - 0.2).max(0.0))))
    } else if t < DONE_AT {
        head.child(running(theme, reduced))
            .child(text(12.5, theme.text, "Compacting").font_weight(gpui::FontWeight::SEMIBOLD))
            .child(pill("Carry over", theme.accent))
            .child(meta(format!(
                "\u{b7} 120.0k \u{b7} {}s, usually ~9s",
                (t - COMPACT_AT).floor() as u32 + 1
            )))
    } else {
        let post = 120.0 - 79.8 * ramp(t, DONE_AT, 600.0, motion::EASE_OUT);
        head.child(glyph(icons::LUCIDE_CHECK, 14.0, theme.success))
            .child(
                text(12.5, theme.text, "Compacted").font_weight(if slim < 1.0 {
                    gpui::FontWeight::SEMIBOLD
                } else {
                    gpui::FontWeight::NORMAL
                }),
            )
            .child(pill("Carry over", theme.accent))
            .child(meta(format!("\u{b7} 120.0k \u{2192} {post:.1}k \u{b7} 9s")))
            .child(div().flex_1())
            .when(slim > 0.0, |el| {
                el.child(glyph(
                    icons::LUCIDE_CHEVRON_RIGHT,
                    12.0,
                    theme.text_faint.opacity(slim),
                ))
            })
    };
    let progress = 0.92 * ramp(t, COMPACT_AT, 2400.0, motion::EASE_OUT);
    let detail = (COMPACT_AT..DONE_AT).contains(&t).then(|| {
        div()
            .pl(px(21.0))
            .flex()
            .flex_col()
            .gap(px(8.0))
            .child(text(
                12.0,
                theme.text_muted,
                "Same task: the checkout flow is still being built.",
            ))
            .child(
                div()
                    .h(px(2.0))
                    .w_full()
                    .rounded_full()
                    .bg(crate::theme::ink(0.06))
                    .child(
                        div()
                            .h_full()
                            .w(relative(progress))
                            .rounded_full()
                            .bg(theme.accent),
                    ),
            )
    });
    let pad = 9.0 - 5.0 * slim;
    div()
        .w(px(400.0))
        .flex()
        .flex_col()
        .gap(px(10.0))
        .child(
            div()
                .p(px(pad))
                .px(px(10.0))
                .flex()
                .flex_col()
                .gap(px(8.0))
                .rounded(px(10.0))
                .border_1()
                .border_color(theme.border.opacity(1.0 - slim))
                .bg(theme.surface_card.opacity(0.85 * (1.0 - slim)))
                .child(head)
                .children(detail),
        )
        .child(text(
            11.0,
            theme.text_faint,
            match t {
                t if t < COMPACT_AT => "A small model checks first",
                t if t < DONE_AT => "Its verdict and reason, then the compaction",
                _ => "Open it for the summary and what was kept",
            },
        ))
        .into_any_element()
}

fn compact_ahead_visual(theme: &Theme, t: f32, reduced: bool) -> AnyElement {
    const QUEUE_AT: f32 = 1.0;
    const DONE_AT: f32 = 3.0;
    const CODEX_AT: f32 = 4.2;
    const SWAP_AT: f32 = 6.4;
    let meta = |copy: String| text(11.5, theme.text_faint, copy);
    let strong =
        |copy: &'static str| text(12.5, theme.text, copy).font_weight(gpui::FontWeight::SEMIBOLD);
    let reply = |copy: &'static str| text(12.0, theme.text, copy);
    let bubble = |copy: &'static str| {
        div().flex().justify_end().child(
            div()
                .px(px(10.0))
                .py(px(5.0))
                .rounded(px(10.0))
                .bg(crate::theme::ink(0.06))
                .child(text(12.0, theme.text, copy)),
        )
    };
    let frame = |head: gpui::Div, bar: Option<f32>| {
        div()
            .p(px(9.0))
            .px(px(10.0))
            .flex()
            .flex_col()
            .gap(px(8.0))
            .rounded(px(10.0))
            .border_1()
            .border_color(theme.border)
            .bg(theme.surface_card.opacity(0.85))
            .child(head)
            .children(bar.map(|fill| {
                div()
                    .ml(px(21.0))
                    .h(px(2.0))
                    .rounded_full()
                    .bg(crate::theme::ink(0.06))
                    .child(
                        div()
                            .h_full()
                            .w(relative(fill))
                            .rounded_full()
                            .bg(theme.accent),
                    )
            }))
    };
    let head = || div().flex().flex_wrap().items_center().gap(px(7.0));
    let claude = t < CODEX_AT;
    let mut chat = div().flex().flex_col().gap(px(10.0));
    if claude {
        chat = chat.child(reply("Done: the receipt matches the order page."));
        let card = if t < DONE_AT {
            let secs = (t * 10.0).floor() as u32 + 1;
            frame(
                head()
                    .child(running(theme, reduced))
                    .child(strong("Compacting"))
                    .child(pill("Carry over", theme.accent))
                    .child(meta(format!("\u{b7} 212.0k \u{b7} {secs}s, usually ~40s"))),
                Some(0.92 * ramp(t, 0.0, 2800.0, motion::EASE_OUT)),
            )
        } else {
            let post = 212.0 - 143.0 * ramp(t, DONE_AT, 600.0, motion::EASE_OUT);
            frame(
                head()
                    .child(glyph(icons::LUCIDE_CHECK, 14.0, theme.success))
                    .child(strong("Compacted"))
                    .child(pill("Carry over", theme.accent))
                    .child(meta(format!(
                        "\u{b7} 212.0k \u{2192} {post:.1}k \u{b7} 40s"
                    ))),
                None,
            )
        };
        chat = chat.child(card);
        if t >= DONE_AT + 0.4 {
            chat = chat.child(rise(bubble("Add a refund button."), t, DONE_AT + 0.4));
        }
    } else {
        chat = chat
            .child(bubble("Add a refund button."))
            .child(reply("Refund button added."));
        if t >= SWAP_AT {
            let post = 212.0 - 143.0 * ramp(t, SWAP_AT, 600.0, motion::EASE_OUT);
            chat = chat.child(rise(
                frame(
                    head()
                        .child(glyph(icons::LUCIDE_CHECK, 14.0, theme.success))
                        .child(strong("Compacted in the background"))
                        .child(pill("Swapped in", theme.warning))
                        .child(meta(format!(
                            "\u{b7} 212k \u{2192} {post:.0}k \u{b7} 1 turn as text"
                        ))),
                    None,
                ),
                t,
                SWAP_AT,
            ));
        }
    }
    let queued = (claude && (QUEUE_AT..DONE_AT + 0.4).contains(&t)).then(|| {
        let left = (DONE_AT - t).max(0.0) * 10.0;
        rise(
            div()
                .h(px(30.0))
                .px(px(10.0))
                .flex()
                .items_center()
                .gap(px(8.0))
                .rounded(px(8.0))
                .border_1()
                .border_color(theme.border)
                .child(text(12.0, theme.text, "Add a refund button."))
                .child(div().flex_1())
                .child(meta(format!(
                    "Sends after compaction \u{b7} 0:{:02}",
                    left.ceil() as u32
                ))),
            t,
            QUEUE_AT,
        )
    });
    let chip = (!claude && t < SWAP_AT).then(|| {
        let ready = t >= SWAP_AT - 0.6;
        div()
            .flex_none()
            .h(px(20.0))
            .px(px(7.0))
            .flex()
            .items_center()
            .gap(px(5.0))
            .rounded(px(6.0))
            .bg(theme.warning.opacity(0.12))
            .child(glyph(
                if ready {
                    icons::LUCIDE_CHECK
                } else {
                    icons::LUCIDE_CIRCLE_DASHED
                },
                12.0,
                theme.warning,
            ))
            .child(text(
                11.0,
                theme.warning,
                if ready {
                    "Copy ready \u{b7} swaps now".to_string()
                } else {
                    format!(
                        "Compacting a copy \u{b7} 0:{:02}",
                        ((t - CODEX_AT) * 10.0) as u32
                    )
                },
            ))
    });
    div()
        .w(px(380.0))
        .flex()
        .flex_col()
        .gap(px(10.0))
        .child(chat)
        .children(queued)
        .children(chip.map(|chip| div().flex().child(chip)))
        .child(text(
            11.0,
            theme.text_faint,
            if claude {
                "Claude compacts in place as the turn ends; a message waits for it"
            } else {
                "Codex compacts a copy while it works, then swaps it in"
            },
        ))
        .into_any_element()
}

fn working_tree_visual(theme: &Theme, t: f32, reduced: bool) -> AnyElement {
    const DONE_AT: f32 = 2.4;
    let turn_done = t >= DONE_AT;
    let grow = |start: f32| ramp(t, start, 180.0, motion::EASE_OUT);
    let line = |path: &'static str,
                label: gpui::Div,
                model: Option<&'static str>,
                elapsed: String,
                open: f32| {
        div()
            .h(px(22.0 * open))
            .opacity(open)
            .overflow_hidden()
            .child(
                div()
                    .h(px(22.0))
                    .flex()
                    .items_center()
                    .gap(px(6.0))
                    .child(glyph(path, 12.0, theme.text_muted))
                    .child(label)
                    .children(model.map(|model| text(11.0, theme.text_faint, model)))
                    .child(div().flex_1())
                    .child(mono(theme, 10.5, theme.text_faint, elapsed)),
            )
    };
    let secs = |base: f32| {
        let total = (base + t).floor() as u32;
        if total >= 60 {
            format!("{}m {:02}s", total / 60, total % 60)
        } else {
            format!("{total}s")
        }
    };
    let lead = if turn_done {
        div()
            .flex_none()
            .size(px(12.0))
            .flex()
            .items_center()
            .justify_center()
            .child(
                div()
                    .size(px(7.0))
                    .rounded_full()
                    .border_1()
                    .border_color(theme.accent),
            )
            .into_any_element()
    } else {
        running(theme, reduced)
    };
    let totals = "2 subagents \u{b7} 1 shell";
    card(theme)
        .w(px(330.0))
        .p(px(10.0))
        .flex()
        .flex_col()
        .gap(px(4.0))
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(6.0))
                .child(glyph(icons::SPEEDOMETER, 13.0, theme.text_muted))
                .child(text(12.0, theme.text_muted, "Working now"))
                .child(text(12.0, theme.text_faint, "1"))
                .child(div().flex_1())
                .child(text(11.0, theme.text_faint, totals)),
        )
        .child(
            div()
                .mt(px(2.0))
                .h(px(24.0))
                .flex()
                .items_center()
                .gap(px(7.0))
                .child(lead)
                .child(text(12.5, theme.text, "Ship the shop"))
                .child(text(11.5, theme.text_muted, "Opus High"))
                .child(div().flex_1())
                .child(mono(theme, 10.5, theme.text_faint, secs(128.0))),
        )
        .child(
            div()
                .h(px(16.0 * ramp(t, DONE_AT, 180.0, motion::EASE_OUT)))
                .overflow_hidden()
                .pl(px(19.0))
                .child(text(
                    11.0,
                    theme.text_faint,
                    "Turn done \u{b7} background still running",
                )),
        )
        .child(
            div()
                .ml(px(5.0))
                .pl(px(13.0))
                .border_l_1()
                .border_color(theme.border)
                .flex()
                .flex_col()
                .child(line(
                    icons::BOT,
                    text(12.0, theme.text_muted, "Research payment providers"),
                    None,
                    secs(64.0),
                    grow(0.3),
                ))
                .child(line(
                    icons::BOT,
                    text(12.0, theme.text_muted, "Product grid with filters"),
                    Some("Sonnet"),
                    secs(20.0),
                    grow(0.45),
                ))
                .child(line(
                    icons::TERMINAL,
                    mono(theme, 11.0, theme.text_muted, "npm run dev"),
                    None,
                    secs(121.0),
                    grow(0.6),
                )),
        )
        .into_any_element()
}

fn phone_composer(theme: &Theme, content: gpui::AnyElement) -> gpui::Div {
    div()
        .mx(px(8.0))
        .mb(px(10.0))
        .h(px(34.0))
        .px(px(12.0))
        .flex()
        .items_center()
        .gap(px(1.0))
        .rounded(px(17.0))
        .border_1()
        .border_color(theme.border_strong)
        .bg(theme.surface_raised)
        .child(content)
        .child(div().flex_1())
        .child(
            div()
                .size(px(22.0))
                .rounded_full()
                .flex()
                .items_center()
                .justify_center()
                .bg(theme.accent)
                .child(glyph(icons::ARROW_UP, 12.0, theme.on_accent)),
        )
}

fn phone_slash_visual(theme: &Theme, t: f32) -> AnyElement {
    const SKILLS_AT: f32 = 3.6;
    let skills = t >= SKILLS_AT;
    let query = if skills {
        typed("$", t, SKILLS_AT + 0.2, 0.1)
    } else {
        typed("/", t, 0.3, 0.1)
    };
    let open_at = if skills { SKILLS_AT + 0.3 } else { 0.4 };
    let open = if query.is_empty() {
        0.0
    } else {
        ramp(t, open_at, 200.0, motion::EASE_OUT)
    };
    let row = |path: &'static str, title: &'static str, detail: &'static str, terminal: bool| {
        div()
            .h(px(25.0))
            .px(px(8.0))
            .flex()
            .items_center()
            .gap(px(6.0))
            .when(terminal, |el| el.opacity(0.55))
            .child(glyph(path, 12.0, theme.text_muted))
            .child(text(11.5, theme.text, title).font_weight(gpui::FontWeight::MEDIUM))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_size(crate::typography::ui_rems(10.5))
                    .text_color(theme.text_muted)
                    .child(detail),
            )
            .when(terminal, |el| {
                el.child(
                    div()
                        .px(px(4.0))
                        .rounded(px(4.0))
                        .bg(crate::theme::ink(0.08))
                        .child(text(9.5, theme.text_muted, "Terminal")),
                )
            })
    };
    let rows: Vec<gpui::Div> = if skills {
        vec![
            row(icons::SPARK, "$review", "Review the current diff", false),
            row(
                icons::SPARK,
                "$release-notes",
                "Draft the release notes",
                false,
            ),
            row(icons::SPARK, "$tests", "Write tests for a change", false),
        ]
    } else {
        vec![
            row(icons::COMMAND, "/model", "Switch the model", false),
            row(
                icons::COMMAND,
                "/new",
                "Zeron: start a new conversation",
                false,
            ),
            row(icons::COMMAND, "/compact", "Compact the context", false),
            row(icons::COMMAND, "/status", "Runs only in the terminal", true),
        ]
    };
    let popup = div()
        .mx(px(8.0))
        .mb(px(6.0))
        .p(px(3.0))
        .flex()
        .flex_col()
        .rounded(px(14.0))
        .border_1()
        .border_color(theme.border)
        .bg(theme.surface_raised)
        .relative()
        .top(px((1.0 - open) * 8.0))
        .opacity(open)
        .children(rows);
    div()
        .flex()
        .items_end()
        .gap(px(18.0))
        .child(
            phone(theme, 230.0, 168.0)
                .flex()
                .flex_col()
                .justify_end()
                .child(popup)
                .child(phone_composer(
                    theme,
                    div()
                        .flex()
                        .items_center()
                        .gap(px(1.0))
                        .child(text(13.0, theme.text, query))
                        .child(caret(theme, t))
                        .into_any_element(),
                )),
        )
        .child(
            div()
                .w(px(130.0))
                .pb(px(20.0))
                .text_size(crate::typography::ui_rems(11.5))
                .text_color(theme.text_faint)
                .child(if skills {
                    "$ lists the project\u{2019}s skills"
                } else {
                    "/ lists every agent\u{2019}s commands, Zeron\u{2019}s too"
                }),
        )
        .into_any_element()
}

fn phone_subagents_visual(theme: &Theme, t: f32, reduced: bool) -> AnyElement {
    const TAP_AT: f32 = 1.2;
    const DONE_AT: f32 = 3.4;
    let open = ramp(t, TAP_AT + 0.1, 220.0, motion::EASE_OUT);
    let done = t >= DONE_AT;
    let pressed = (TAP_AT..TAP_AT + 0.2).contains(&t);
    let heading = if done {
        "1 of 3 working"
    } else {
        "2 of 3 working"
    };
    let (_, _, ok) = chat_status_tone(zeron_proto::ChatIndicator::Completed, theme);
    let finished = || tree_glyph(TreeStatus::Done, 12.0, ok, true);
    let row = |lead: gpui::AnyElement, title: &'static str, line: String, stop: bool| {
        div()
            .h(px(32.0))
            .px(px(8.0))
            .flex()
            .items_center()
            .gap(px(7.0))
            .child(lead)
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .child(
                        div()
                            .truncate()
                            .text_size(crate::typography::ui_rems(11.5))
                            .text_color(theme.text)
                            .child(title),
                    )
                    .child(
                        div()
                            .truncate()
                            .text_size(crate::typography::ui_rems(10.0))
                            .text_color(theme.text_muted)
                            .child(line),
                    ),
            )
            .when(stop, |el| {
                el.child(
                    div()
                        .h(px(18.0))
                        .px(px(7.0))
                        .flex()
                        .items_center()
                        .rounded_full()
                        .bg(crate::theme::ink(0.08))
                        .child(text(10.0, theme.text, "Stop")),
                )
            })
    };
    let secs = |base: f32| format!("{}s", (base + t).floor() as u32);
    let roster = div()
        .mx(px(8.0))
        .mb(px(6.0))
        .p(px(4.0))
        .flex()
        .flex_col()
        .rounded(px(16.0))
        .border_1()
        .border_color(theme.border)
        .bg(theme.surface_raised)
        .relative()
        .top(px((1.0 - open) * 10.0))
        .opacity(open)
        .child(
            div()
                .h(px(22.0))
                .px(px(8.0))
                .flex()
                .items_center()
                .child(text(11.5, theme.text, heading).font_weight(gpui::FontWeight::MEDIUM))
                .child(div().flex_1())
                .child(glyph(icons::CLOSE, 9.0, theme.text_muted)),
        )
        .child(row(
            running(theme, reduced),
            "Research payment providers",
            format!("Working \u{b7} {} \u{b7} Sonnet", secs(41.0)),
            true,
        ))
        .child(row(
            if done {
                finished()
            } else {
                running(theme, reduced)
            },
            "Write the cart tests",
            if done {
                "Completed \u{b7} 38s".into()
            } else {
                format!("Working \u{b7} {}", secs(34.6))
            },
            !done,
        ));
    let pill = div()
        .ml(px(8.0))
        .mb(px(6.0))
        .h(px(24.0))
        .px(px(10.0))
        .flex()
        .items_center()
        .gap(px(5.0))
        .rounded_full()
        .border_1()
        .border_color(theme.border)
        .bg(if pressed || open > 0.0 {
            theme.accent.opacity(0.16)
        } else {
            theme.surface_raised
        })
        .child(running(theme, reduced))
        .child(text(
            11.0,
            if open > 0.0 {
                theme.text
            } else {
                theme.text_muted
            },
            heading,
        ));
    div()
        .flex()
        .items_end()
        .gap(px(18.0))
        .child(
            phone(theme, 250.0, 168.0)
                .flex()
                .flex_col()
                .justify_end()
                .when(open > 0.0, |el| el.child(roster))
                .child(div().flex().child(pill))
                .child(phone_composer(
                    theme,
                    text(12.0, theme.text_faint, "Message").into_any_element(),
                )),
        )
        .child(
            div()
                .w(px(120.0))
                .pb(px(20.0))
                .text_size(crate::typography::ui_rems(11.5))
                .text_color(theme.text_faint)
                .child("Tap the pill for the roster: stop a run or open its output"),
        )
        .into_any_element()
}

fn phone_background_visual(theme: &Theme, t: f32) -> AnyElement {
    const SYNC_AT: f32 = 1.0;
    const CHANGE_AT: f32 = 3.0;
    let artwork = |hue: f32| {
        linear_gradient(
            160.0,
            linear_color_stop(gpui::hsla(hue, 0.55, 0.55, 0.85), 0.0),
            linear_color_stop(gpui::hsla((hue + 0.12) % 1.0, 0.5, 0.35, 0.0), 1.0),
        )
    };
    let mac_shift = ramp(t, CHANGE_AT, 500.0, motion::EASE_IN_OUT);
    let phone_shift = ramp(t, CHANGE_AT + 0.45, 500.0, motion::EASE_IN_OUT);
    let hue = |shift: f32| 0.58 + 0.32 * shift;
    let synced = ramp(t, SYNC_AT + 0.3, 400.0, motion::EASE_OUT);
    let bubbles = |width: f32| {
        div()
            .absolute()
            .left(px(10.0))
            .right(px(10.0))
            .top(px(30.0))
            .flex()
            .flex_col()
            .gap(px(6.0))
            .child(
                div()
                    .h(px(8.0))
                    .w(px(width * 0.5))
                    .rounded_full()
                    .bg(theme.text.opacity(0.25)),
            )
            .child(
                div()
                    .h(px(8.0))
                    .w(px(width * 0.7))
                    .rounded_full()
                    .bg(theme.text.opacity(0.18)),
            )
            .child(
                div().flex().justify_end().child(
                    div()
                        .h(px(14.0))
                        .w(px(width * 0.35))
                        .rounded(px(7.0))
                        .bg(theme.accent.opacity(0.5)),
                ),
            )
    };
    let mac = div()
        .flex_none()
        .w(px(180.0))
        .h(px(104.0))
        .relative()
        .overflow_hidden()
        .rounded(px(10.0))
        .border_1()
        .border_color(theme.border_strong)
        .bg(theme.surface)
        .child(
            div()
                .absolute()
                .inset_0()
                .h(px(72.0))
                .bg(artwork(hue(mac_shift))),
        )
        .child(
            div()
                .absolute()
                .top(px(6.0))
                .left(px(8.0))
                .flex()
                .gap(px(4.0))
                .children(
                    [0.6, 0.45, 0.3]
                        .map(|a| div().size(px(6.0)).rounded_full().bg(theme.text.opacity(a))),
                ),
        )
        .child(bubbles(160.0));
    let phone_screen = phone(theme, 84.0, 128.0)
        .child(
            div()
                .absolute()
                .inset_0()
                .h(px(86.0))
                .opacity(synced)
                .bg(artwork(hue(phone_shift))),
        )
        .child(bubbles(72.0));
    let row = div()
        .w(px(300.0))
        .h(px(26.0))
        .px(px(10.0))
        .flex()
        .items_center()
        .gap(px(7.0))
        .rounded(px(8.0))
        .bg(crate::theme::ink(0.04))
        .child(glyph(icons::LAPTOP, 13.0, theme.text_muted))
        .child(text(12.0, theme.text, "Sync with Mac"))
        .child(text(
            11.0,
            theme.text_faint,
            if t >= SYNC_AT {
                "Follows MacBook Pro"
            } else {
                "Use MacBook Pro\u{2019}s background"
            },
        ))
        .child(div().flex_1())
        .child(crate::settings::widgets::toggle_switch(
            theme,
            t >= SYNC_AT,
            "whats-new-sync-with-mac",
        ));
    div()
        .flex()
        .flex_col()
        .items_center()
        .gap(px(8.0))
        .child(
            div()
                .flex()
                .items_end()
                .gap(px(16.0))
                .child(mac)
                .child(phone_screen),
        )
        .child(row)
        .into_any_element()
}

fn rail_update_visual(theme: &Theme, t: f32, reduced: bool) -> AnyElement {
    const DOWNLOAD_AT: f32 = 0.9;
    const READY_AT: f32 = 3.6;
    let fill = theme.accent;
    let ink = theme.on_accent;
    let ready = t >= READY_AT;
    let downloading = (DOWNLOAD_AT..READY_AT).contains(&t);
    let mut diameter = update_glyph::CIRCLE;
    let layers = if downloading && !reduced {
        let drop = ((t - DOWNLOAD_AT) / update_glyph::DROP_PERIOD.as_secs_f32()).fract();
        vec![
            update_glyph::Layer {
                path: icons::LUCIDE_DOWNLOAD_TRAY,
                opacity: 1.0,
                transform: gpui::Transformation::default(),
            },
            update_glyph::falling_arrow(drop),
        ]
    } else if ready && !reduced && t - READY_AT < update_glyph::CHECK_UNTIL + update_glyph::TURN {
        let since = t - READY_AT;
        diameter *= update_glyph::pop_scale(since);
        if since < update_glyph::CHECK_UNTIL {
            vec![update_glyph::check_in(since)]
        } else {
            vec![update_glyph::restart_in(
                (since - update_glyph::CHECK_UNTIL) / update_glyph::TURN,
            )]
        }
    } else {
        vec![update_glyph::Layer {
            path: if ready {
                icons::LUCIDE_ROTATE_CW
            } else {
                icons::LUCIDE_DOWNLOAD
            },
            opacity: 1.0,
            transform: gpui::Transformation::default(),
        }]
    };
    let arc = (downloading && !reduced)
        .then(|| ((t - DOWNLOAD_AT) / update_glyph::ARC_PERIOD.as_secs_f32()).fract());
    let arc_tint = gpui::Hsla {
        l: (fill.l + 0.16).min(0.92),
        ..fill
    };
    let halo_phase = (t - READY_AT) / motion::UPDATE_HALO_PERIOD.as_secs_f32();
    let halos = ready.then(|| {
        update_glyph::HALOS.map(|(size, offset, peak)| {
            let alpha = if reduced {
                peak * 0.5
            } else {
                peak * motion::update_halo_ripple(halo_phase + offset)
            };
            div()
                .absolute()
                .size(px(size))
                .left(px((app_rail::BUTTON - size) / 2.0))
                .top(px((app_rail::BUTTON - size) / 2.0))
                .rounded_full()
                .bg(fill.opacity(alpha))
        })
    });
    let button = div()
        .size(px(app_rail::BUTTON))
        .relative()
        .flex()
        .items_center()
        .justify_center()
        .children(halos.into_iter().flatten())
        .when_some(arc, |el, turn| {
            el.child(
                gpui::canvas(
                    |_, _, _| (),
                    move |bounds, _, window, _| {
                        update_glyph::paint_arc(bounds, turn, arc_tint, window);
                    },
                )
                .absolute()
                .inset_0(),
            )
        })
        .child(
            div()
                .size(px(diameter))
                .flex_none()
                .relative()
                .rounded_full()
                .bg(fill)
                .children(layers.into_iter().map(|layer| {
                    div()
                        .absolute()
                        .inset_0()
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(
                            icon(layer.path)
                                .size(px(update_glyph::ICON))
                                .text_color(ink.opacity(layer.opacity))
                                .with_transformation(layer.transform),
                        )
                })),
        );
    let rail_icon = |glyph: &'static str| {
        div()
            .size(px(app_rail::BUTTON))
            .flex()
            .items_center()
            .justify_center()
            .child(icon(glyph).size(px(16.0)).text_color(theme.text_faint))
    };
    let caption = if ready {
        "Update ready \u{2014} restart to apply"
    } else if downloading {
        "Downloading v0.2.129\u{2026}"
    } else {
        "Update available \u{2014} v0.2.129"
    };
    div()
        .flex()
        .flex_row()
        .items_center()
        .gap(px(16.0))
        .child(
            div()
                .py(px(8.0))
                .px(px(4.0))
                .flex()
                .flex_col()
                .items_center()
                .gap(px(6.0))
                .rounded(px(12.0))
                .border_1()
                .border_color(theme.border)
                .bg(crate::settings::widgets::block_fill(theme))
                .child(rail_icon(icons::LUCIDE_CIRCLE_HELP))
                .child(button),
        )
        .child(
            div()
                .w(px(210.0))
                .text_size(crate::typography::ui_rems(12.0))
                .text_color(theme.text_muted)
                .child(caption),
        )
        .into_any_element()
}
