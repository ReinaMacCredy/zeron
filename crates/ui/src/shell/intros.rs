use super::*;
use crate::tasks::{TreeStatus, tree_glyph};
use std::time::Instant;

mod release;

pub(super) const VISUAL_HEIGHT: f32 = 172.0;

const TASKS_HOME: &str = "tasks-home";
const MISSION_TABS: &str = "mission-tabs";
const SIDEBAR_MISSION: &str = "sidebar-mission";
const SUPERVISOR_CONVERSATION: &str = "supervisor-conversation";

fn since_row(
    theme: &Theme,
    lead: AnyElement,
    title: &'static str,
    meta: Option<&'static str>,
) -> gpui::Div {
    div()
        .h(px(28.0))
        .px(px(10.0))
        .flex()
        .items_center()
        .gap(px(8.0))
        .rounded(px(8.0))
        .text_size(crate::typography::ui_rems(12.0))
        .text_color(theme.text)
        .child(lead)
        .child(div().flex_1().min_w_0().truncate().child(title))
        .children(meta.map(|meta| {
            div()
                .flex_none()
                .text_size(crate::typography::ui_rems(11.0))
                .text_color(theme.text_faint)
                .child(meta)
        }))
}

fn ring(theme: &Theme, percent: u8) -> AnyElement {
    let (_, _, working) = chat_status_tone(zeron_proto::ChatIndicator::Working, theme);
    crate::loaders::upload_progress_arc(percent, 12.0, theme.text_faint.opacity(0.35), working)
}

fn running(theme: &Theme, reduced: bool) -> AnyElement {
    let (_, _, working) = chat_status_tone(zeron_proto::ChatIndicator::Working, theme);
    tree_glyph(TreeStatus::Running, 12.0, working, reduced)
}

fn ramp(t: f32, start: f32, ms: f32, curve: motion::CubicBezier) -> f32 {
    curve.eval(((t - start) * 1000.0 / ms).clamp(0.0, 1.0))
}

fn rise(el: gpui::Div, t: f32, start: f32) -> gpui::Div {
    let p = ramp(t, start, 220.0, motion::EASE_OUT);
    el.relative().top(px((1.0 - p) * 6.0)).opacity(p)
}

fn task_box(theme: &Theme) -> AnyElement {
    div()
        .flex_none()
        .size(px(12.0))
        .rounded(px(4.0))
        .border_1()
        .border_color(theme.border_strong)
        .into_any_element()
}

fn filter_chip(theme: &Theme, label: &'static str, count: &'static str, on: bool) -> gpui::Div {
    div()
        .flex_none()
        .h(px(22.0))
        .px(px(8.0))
        .flex()
        .items_center()
        .gap(px(5.0))
        .rounded(px(7.0))
        .border_1()
        .border_color(if on {
            theme.border_strong
        } else {
            theme.border
        })
        .when(on, |el| el.bg(theme.wash(0.06)))
        .text_size(crate::typography::ui_rems(11.0))
        .text_color(if on { theme.text } else { theme.text_muted })
        .child(label)
        .child(div().text_color(theme.text_faint).child(count))
}

fn tasks_visual(theme: &Theme, t: f32, reduced: bool) -> AnyElement {
    const CHIPS: [(&str, &str); 3] = [("All", "3"), ("Plans", "1"), ("Trees", "1")];
    const STARTS: [f32; 3] = [0.0, 2.0, 3.4];
    let filter = STARTS.iter().rposition(|start| t >= *start).unwrap_or(0);
    let start = STARTS[filter];
    let rows: &[usize] = match filter {
        0 => &[0, 1, 2],
        1 => &[0],
        _ => &[1],
    };
    let row = |ix: usize| match ix {
        0 => since_row(theme, ring(theme, 80), "Ship the shop", Some("4/5")),
        1 => since_row(
            theme,
            running(theme, reduced),
            "Fix tab strip jitter",
            Some("tree \u{b7} 3"),
        ),
        _ => since_row(
            theme,
            task_box(theme),
            "Write the release notes",
            Some("task"),
        ),
    };
    div()
        .size_full()
        .px(px(14.0))
        .pt(px(14.0))
        .flex()
        .flex_col()
        .gap(px(6.0))
        .child(
            div().flex().gap(px(6.0)).mb(px(2.0)).children(
                CHIPS
                    .iter()
                    .enumerate()
                    .map(|(ix, (label, count))| filter_chip(theme, label, count, ix == filter)),
            ),
        )
        .children(
            rows.iter()
                .enumerate()
                .map(|(n, &ix)| rise(row(ix), t, start + n as f32 * 0.07)),
        )
        .into_any_element()
}

fn tabs_visual(theme: &Theme, t: f32, reduced: bool) -> AnyElement {
    let tone = motion::mix(theme.accent, theme.text, 0.2);
    let fold =
        ramp(t, 1.6, 240.0, motion::EASE_OUT) * (1.0 - ramp(t, 4.0, 240.0, motion::EASE_OUT));
    let hover = ramp(t, 2.4, 150.0, motion::EASE) * (1.0 - ramp(t, 3.6, 120.0, motion::EASE));
    let pressed = (1.45..1.65).contains(&t) || (3.85..4.05).contains(&t);
    let tab = |label: &'static str, selected: bool, width: f32, open: f32| {
        div()
            .flex_none()
            .w(px(width * open))
            .ml(px(2.0 * open))
            .opacity(open)
            .when(open < 1.0, |el| el.overflow_hidden())
            .child(
                div()
                    .w(px(width))
                    .h(px(26.0))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(8.0))
                    .text_size(px(12.0))
                    .whitespace_nowrap()
                    .text_color(if selected {
                        theme.text
                    } else {
                        theme.text_muted
                    })
                    .when(selected, |el| {
                        el.bg(crate::theme::glass_selected_bg())
                            .shadow(crate::theme::glass_selected_shadows())
                    })
                    .child(label),
            )
    };
    let title = div()
        .flex_none()
        .h(px(26.0))
        .px(px(10.0))
        .flex()
        .items_center()
        .gap(px(6.0))
        .rounded(px(8.0))
        .when(pressed, |el| el.bg(theme.wash(0.1)))
        .when(!pressed && hover > 0.0, |el| {
            el.bg(theme.wash(0.06 * hover))
        })
        .text_size(px(12.0))
        .text_color(tone)
        .child(
            icons::icon(icons::LUCIDE_FOLDER)
                .size(px(12.0))
                .text_color(tone),
        )
        .child("Ship the shop");
    let menu = div()
        .w(px(170.0))
        .p(px(4.0))
        .flex()
        .flex_col()
        .gap(px(1.0))
        .rounded(px(9.0))
        .border_1()
        .border_color(theme.border)
        .bg(crate::settings::widgets::block_fill(theme))
        .relative()
        .top(px((1.0 - hover) * -4.0))
        .opacity(hover)
        .children(
            ["Step 2", "Step 3"]
                .map(|label| since_row(theme, running(theme, reduced), label, None).h(px(24.0))),
        );
    let caption = if (2.3..3.8).contains(&t) {
        "Hover the title to list the rest"
    } else if (3.8..4.4).contains(&t) {
        "Click it again to unfold"
    } else {
        "Click the title to fold the group to the Lead"
    };
    div()
        .flex()
        .flex_col()
        .items_center()
        .gap(px(10.0))
        .child(
            div().w(px(300.0)).flex().child(rise(
                div()
                    .flex()
                    .items_center()
                    .pr(px(3.0))
                    .rounded(px(9.0))
                    .border_1()
                    .border_color(tone.opacity(0.45))
                    .child(title)
                    .child(tab("Lead", true, 46.0, 1.0))
                    .child(tab("Step 2", false, 58.0, 1.0 - fold))
                    .child(tab("Step 3", false, 58.0, 1.0 - fold)),
                t,
                0.0,
            )),
        )
        .child(div().w(px(300.0)).h(px(60.0)).child(menu))
        .child(
            div()
                .text_size(crate::typography::ui_rems(11.5))
                .text_color(theme.text_faint)
                .child(caption),
        )
        .into_any_element()
}

fn sidebar_visual(theme: &Theme, t: f32, reduced: bool) -> AnyElement {
    const DONE_AT: f32 = 2.2;
    const NEXT_AT: f32 = 2.9;
    let nested = |row: gpui::Div| row.ml(px(18.0));
    let finished = t >= DONE_AT;
    let next = t >= NEXT_AT;
    let leave = ramp(t, 2.8, 260.0, motion::EASE_OUT);
    let percent = (40.0 + 20.0 * ramp(t, DONE_AT, 500.0, motion::EASE_OUT)).round() as u8;
    let (_, _, ok) = chat_status_tone(zeron_proto::ChatIndicator::Completed, theme);
    let first = if finished {
        tree_glyph(TreeStatus::Done, 12.0, ok, true)
    } else {
        running(theme, reduced)
    };
    div()
        .w(px(250.0))
        .p(px(6.0))
        .flex()
        .flex_col()
        .gap(px(2.0))
        .rounded(px(10.0))
        .border_1()
        .border_color(theme.border)
        .bg(crate::settings::widgets::block_fill(theme))
        .child(rise(
            since_row(
                theme,
                ring(theme, percent),
                "Ship the shop",
                Some(if finished { "3/5" } else { "2/5" }),
            )
            .bg(theme.wash(0.06)),
            t,
            0.0,
        ))
        .child(
            div()
                .h(px(28.0 * (1.0 - leave)))
                .overflow_hidden()
                .opacity(1.0 - leave)
                .child(rise(
                    nested(since_row(theme, first, "Product grid with filters", None)),
                    t,
                    0.07,
                )),
        )
        .child(rise(
            nested(since_row(
                theme,
                running(theme, reduced),
                "Cart with persistence",
                None,
            )),
            t,
            0.14,
        ))
        .when(next, |el| {
            el.child(rise(
                nested(since_row(
                    theme,
                    running(theme, reduced),
                    "Checkout flow",
                    None,
                )),
                t,
                NEXT_AT,
            ))
        })
        .child(rise(
            nested(
                div()
                    .h(px(24.0))
                    .px(px(10.0))
                    .flex()
                    .items_center()
                    .text_size(crate::typography::ui_rems(12.0))
                    .text_color(theme.text_muted.opacity(0.6))
                    .child(if next { "1 more step" } else { "2 more steps" }),
            ),
            t,
            0.21,
        ))
        .into_any_element()
}

fn supervisor_conversation_visual(theme: &Theme, t: f32) -> AnyElement {
    const PREFIX: &str = "Keep an eye on ";
    const MENTION: &str = "@Fix tab strip jitter";
    const WATCH_AT: f32 = 2.4;
    let tone = crate::supervisor::ok_tone(theme);
    let total = PREFIX.len() + MENTION.len();
    let typed = (((t - 0.3) / 0.045).max(0.0) as usize).min(total);
    let mention = &MENTION[..typed.saturating_sub(PREFIX.len())];
    let caret = t < WATCH_AT && (typed < total || (t * 2.0).fract() < 0.5);
    let bubble = div()
        .h(px(34.0))
        .max_w(px(300.0))
        .px(px(12.0))
        .rounded(px(14.0))
        .bg(theme.wash(0.08))
        .text_size(crate::typography::ui_rems(12.5))
        .text_color(theme.text)
        .flex()
        .items_center()
        .gap(px(4.0))
        .child(PREFIX[..typed.min(PREFIX.len())].trim_end())
        .when(!mention.is_empty(), |el| {
            el.child(
                div()
                    .font_weight(gpui::FontWeight::MEDIUM)
                    .text_color(theme.accent)
                    .child(mention),
            )
        })
        .when(caret, |el| {
            el.child(div().ml(px(-3.0)).w(px(1.5)).h(px(14.0)).bg(theme.text))
        });
    let pop = ramp(t, WATCH_AT, 220.0, motion::EASE_SNAP);
    let watched = div()
        .relative()
        .top(px((1.0 - pop) * 8.0))
        .opacity(ramp(t, WATCH_AT, 150.0, motion::EASE))
        .h(px(26.0))
        .pl(px(9.0))
        .pr(px(12.0))
        .flex()
        .items_center()
        .gap(px(6.0))
        .rounded_full()
        .border_1()
        .border_color(tone.opacity(0.55))
        .bg(tone.opacity(0.08))
        .text_size(crate::typography::ui_rems(12.0))
        .font_weight(gpui::FontWeight::MEDIUM)
        .text_color(theme.text)
        .child(
            icons::icon(icons::LUCIDE_EYE)
                .size(px(13.0))
                .text_color(tone),
        )
        .child("Watched");
    div()
        .flex()
        .flex_col()
        .items_center()
        .gap(px(12.0))
        .child(bubble)
        .child(watched)
        .into_any_element()
}

pub(super) fn loop_clock(shown: Instant, cx: &App) -> Option<f32> {
    (!motion::reduced_motion(cx))
        .then(|| motion::since(shown).as_secs_f32() / motion::speed_scale().max(f32::EPSILON))
}

const SINCE_VISUALS: [(&str, f32, f32); 4] = [
    (TASKS_HOME, 6.6, 1.9),
    (MISSION_TABS, 6.0, 1.5),
    (SIDEBAR_MISSION, 6.4, 4.5),
    (SUPERVISOR_CONVERSATION, 6.0, 4.5),
];

fn timing(id: &str) -> Option<(f32, f32)> {
    SINCE_VISUALS
        .iter()
        .chain(release::VISUALS.iter())
        .find(|(candidate, ..)| *candidate == id)
        .map(|&(_, period, rest)| (period, rest))
}

pub(super) fn is_since_visual(id: &str) -> bool {
    timing(id).is_some()
}

pub(super) fn since_visual(id: &str, theme: &Theme, clock: Option<f32>) -> AnyElement {
    let Some((period, rest)) = timing(id) else {
        return div().into_any_element();
    };
    let reduced = clock.is_none();
    let t = clock.map_or(rest, |clock| clock % period);
    let visual = match id {
        TASKS_HOME => tasks_visual(theme, t, reduced),
        MISSION_TABS => tabs_visual(theme, t, reduced),
        SIDEBAR_MISSION => sidebar_visual(theme, t, reduced),
        SUPERVISOR_CONVERSATION => supervisor_conversation_visual(theme, t),
        _ => release::visual(id, theme, t, reduced),
    };
    div()
        .size_full()
        .flex()
        .items_center()
        .justify_center()
        .opacity(1.0 - ramp(t, period - 0.35, 300.0, motion::EASE_IN))
        .child(visual)
        .into_any_element()
}
