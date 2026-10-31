use chrono::{DateTime, Utc};
use gpui::{Div, Hsla, SharedString, Stateful, div, prelude::*, px};
use zeron_proto::account_usage::UsageBlock;
use zeron_proto::{AgentAccount, AgentAccountsSnapshot, AgentUsageWindow};

use crate::account_usage::used_fraction;
use crate::icons::{self, icon};
use crate::motion;
use crate::popover;
use crate::settings::accounts::{self, UsageLevel, usage_color, usage_level};
use crate::theme::Theme;

pub(crate) const WIDTH: f32 = 46.0;
pub(crate) const BUTTON: f32 = 34.0;
const GLYPH: f32 = 18.0;
pub(crate) const GAP: f32 = 6.0;
pub(crate) const PAD_Y: f32 = 8.0;
const RING_RADIUS: f32 = 13.0;
const RING_STROKE: f32 = 2.5;
const RING_GLYPH: f32 = 14.0;
const HOVER_CARD_WIDTH: f32 = 290.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RailItem {
    NewChat,
    Inbox,
    Tasks,
    PullRequests,
    Settings,
}

pub(crate) struct RailEntry {
    pub item: RailItem,
    pub id: &'static str,
    pub label: &'static str,
    pub icon: &'static str,
}

pub(crate) const TOP: [RailEntry; 4] = [
    RailEntry {
        item: RailItem::NewChat,
        id: "rail-new-chat",
        label: "New chat",
        icon: icons::PEN_NEW_SQUARE,
    },
    RailEntry {
        item: RailItem::Inbox,
        id: "rail-inbox",
        label: "Inbox",
        icon: icons::BELL,
    },
    RailEntry {
        item: RailItem::Tasks,
        id: "open-tasks",
        label: "Tasks",
        icon: icons::TASKS,
    },
    RailEntry {
        item: RailItem::PullRequests,
        id: "open-pull-requests",
        label: "Pull requests",
        icon: icons::LUCIDE_GIT_PULL_REQUEST,
    },
];

pub(crate) const BOTTOM: [RailEntry; 1] = [RailEntry {
    item: RailItem::Settings,
    id: "settings-trigger",
    label: "Settings",
    icon: icons::LUCIDE_SETTINGS,
}];

pub(crate) fn button(
    id: &'static str,
    label: &'static str,
    glyph: &'static str,
    active: bool,
    wash: Hsla,
    theme: &Theme,
) -> Stateful<Div> {
    div()
        .id(id)
        .debug_selector(move || id.into())
        .role(gpui::Role::Button)
        .aria_label(label)
        .tab_index(0)
        .focus_visible(|style| style.border_2().border_color(theme.accent))
        .size(px(BUTTON))
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(8.0))
        .cursor_pointer()
        .bg(if active {
            wash
        } else {
            motion::hover_blend(id, wash.opacity(0.0), wash)
        })
        .on_hover(motion::hover_listener(id))
        .child(icon(glyph).size(px(GLYPH)).text_color(if active {
            theme.text
        } else {
            motion::hover_blend(id, theme.text_muted, theme.text)
        }))
}

pub(crate) fn badge(count: usize, theme: &Theme) -> Div {
    let label = if count > 99 {
        "99+".to_owned()
    } else {
        count.to_string()
    };
    div()
        .absolute()
        .top(px(1.0))
        .right_0()
        .min_w(px(15.0))
        .h(px(15.0))
        .px(px(4.0))
        .rounded_full()
        .bg(theme.accent)
        .text_color(theme.on_accent)
        .text_size(crate::typography::ui_rems(9.5))
        .font_weight(gpui::FontWeight::SEMIBOLD)
        .flex()
        .items_center()
        .justify_center()
        .child(SharedString::from(label))
}

pub(crate) fn usage_accounts(snapshot: &AgentAccountsSnapshot) -> Vec<&AgentAccount> {
    accounts::sidebar_accounts(snapshot)
        .into_iter()
        .filter(|account| accounts::reports_usage(account.harness))
        .collect()
}

pub(crate) fn tightest(accounts: &[AgentAccount]) -> Option<&AgentAccount> {
    accounts
        .iter()
        .filter_map(|account| used_fraction(account).map(|used| (account, used)))
        .max_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(account, _)| account)
}

pub(crate) fn ring_label(account: Option<&AgentAccount>, hot: bool, loading: bool) -> SharedString {
    if hot {
        return "Usage: limit reached".into();
    }
    let Some(account) = account else {
        return if loading {
            "Loading usage…".into()
        } else {
            "No plan usage reported".into()
        };
    };
    let name = accounts::provider_name(account.harness);
    let windows = account
        .usage_windows
        .iter()
        .map(|window| {
            format!(
                "{} {}%",
                zeron_proto::account_usage::window_label(&window.label),
                (window.used_fraction.clamp(0.0, 1.0) * 100.0).round() as u32
            )
        })
        .collect::<Vec<_>>();
    match (windows.is_empty(), account.usage_error.as_deref()) {
        (false, _) => format!("Usage: {name} · {}", windows.join(", ")).into(),
        (true, Some(error)) => format!("Usage: {name} · {error}").into(),
        (true, None) => format!("Usage: {name} · no usage reported").into(),
    }
}

pub(crate) fn ring(account: Option<&AgentAccount>, hot: bool, theme: &Theme) -> Div {
    let fraction = account.and_then(used_fraction);
    let (mark, tint) = account.map_or((icons::USAGE_CHART, None), |account| {
        crate::pickers::harness_brand_icon(account.harness)
    });
    let shown = if hot { 1.0 } else { fraction.unwrap_or(0.0) };
    let color = if hot {
        theme.danger
    } else {
        usage_color(usage_level(shown), theme)
    };
    div()
        .size(px(BUTTON))
        .relative()
        .flex()
        .items_center()
        .justify_center()
        .child(
            div()
                .absolute()
                .inset_0()
                .child(crate::context_usage::progress_ring(
                    BUTTON,
                    RING_RADIUS,
                    RING_STROKE,
                    shown,
                    color,
                    theme.text_faint.opacity(0.25),
                )),
        )
        .child(
            icon(mark)
                .size(px(RING_GLYPH))
                .text_color(tint.unwrap_or(theme.text))
                .when(fraction.is_none() && !hot, |glyph| glyph.opacity(0.5)),
        )
        .when(hot, |el| {
            el.child(
                div()
                    .absolute()
                    .top(px(1.0))
                    .right_0()
                    .size(px(12.0))
                    .rounded_full()
                    .bg(theme.danger)
                    .text_color(theme.on_accent)
                    .text_size(crate::typography::ui_rems(9.0))
                    .font_weight(gpui::FontWeight::BOLD)
                    .flex()
                    .items_center()
                    .justify_center()
                    .child("!"),
            )
        })
}

/// The card stays while the ring or the card holds the pointer.
#[derive(Default)]
pub(crate) struct RingHover {
    pub trigger: bool,
    pub card: bool,
}

impl RingHover {
    pub fn shown(&self) -> bool {
        self.trigger || self.card
    }
}

fn pace_bar(remaining: f32, marker: Option<(f32, UsageLevel)>, fill: Hsla, theme: &Theme) -> Div {
    let segment = |left: f32, width: f32| {
        div()
            .absolute()
            .top_0()
            .bottom_0()
            .left(gpui::relative(left))
            .w(gpui::relative(width))
    };
    let fill_bar = || div().size_full().bg(fill);
    let track = div()
        .relative()
        .w_full()
        .h(px(6.0))
        .rounded_full()
        .overflow_hidden()
        .bg(theme.wash(0.08));
    let Some((marker, marker_level)) = marker else {
        return track.child(segment(0.0, remaining).child(fill_bar()));
    };
    let marker_color = usage_color(marker_level, theme);
    track
        .child(
            segment(0.0, remaining.min(marker))
                .when(remaining > marker, |el| el.pr(px(3.0)))
                .child(fill_bar()),
        )
        .when(remaining > marker, |el| {
            el.child(
                segment(marker, remaining - marker)
                    .pl(px(3.0))
                    .child(fill_bar()),
            )
        })
        .child(
            segment(marker, 0.0)
                .ml(px(-1.0))
                .w(px(2.0))
                .rounded_full()
                .bg(marker_color),
        )
}

struct BlockFoot {
    left: Option<String>,
    right: Option<String>,
    left_color: Hsla,
    right_color: Hsla,
}

fn usage_block(
    label: SharedString,
    headline: Option<String>,
    right: Option<String>,
    bar: Div,
    foot: Option<BlockFoot>,
    theme: &Theme,
) -> Div {
    div()
        .flex()
        .flex_col()
        .gap(px(4.0))
        .child(
            div()
                .flex()
                .items_baseline()
                .justify_between()
                .gap(px(12.0))
                .child(
                    div()
                        .min_w_0()
                        .flex()
                        .items_baseline()
                        .gap(px(6.0))
                        .child(
                            div()
                                .flex_none()
                                .text_size(crate::typography::ui_rems(12.5))
                                .font_weight(gpui::FontWeight::MEDIUM)
                                .text_color(theme.text)
                                .child(label),
                        )
                        .children(
                            headline
                                .map(|text| div().truncate().text_color(theme.text).child(text)),
                        ),
                )
                .children(
                    right.map(|text| div().flex_none().text_color(theme.text_muted).child(text)),
                ),
        )
        .child(bar)
        .when_some(foot, |el, foot| {
            el.child(
                div()
                    .flex()
                    .items_baseline()
                    .justify_between()
                    .gap(px(12.0))
                    .child(
                        div()
                            .min_w_0()
                            .truncate()
                            .text_color(foot.left_color)
                            .children(foot.left),
                    )
                    .children(
                        foot.right
                            .map(|text| div().flex_none().text_color(foot.right_color).child(text)),
                    ),
            )
        })
}

fn block(block: UsageBlock, theme: &Theme) -> Div {
    let foot = (block.foot_left.is_some() || block.foot_right.is_some()).then(|| BlockFoot {
        left: block.foot_left,
        right: block.foot_right,
        left_color: if block.foot_left_alert {
            theme.danger
        } else {
            theme.text_muted
        },
        right_color: if block.foot_right_alert {
            theme.danger
        } else {
            theme.text_faint
        },
    });
    usage_block(
        SharedString::from(block.label),
        block.headline,
        block.resets,
        pace_bar(
            block.fill,
            block.marker,
            usage_color(block.level, theme),
            theme,
        ),
        foot,
        theme,
    )
}

/// One block per plan window, then Claude's Extra Usage: the Usage page.
pub(crate) fn usage_blocks(account: &AgentAccount, now: DateTime<Utc>, theme: &Theme) -> Vec<Div> {
    zeron_proto::account_usage::usage_blocks(account, now)
        .into_iter()
        .map(|b| block(b, theme))
        .collect()
}

pub(crate) fn details_fold(
    id: gpui::ElementId,
    selector: String,
    account: &AgentAccount,
    open: bool,
    on_details: impl Fn(&gpui::ClickEvent, &mut gpui::Window, &mut gpui::App) + 'static,
    now: DateTime<Utc>,
    theme: &Theme,
) -> Div {
    let identity = account
        .email
        .clone()
        .or_else(|| account.display_name.clone());
    let fetched = account
        .usage_fetched_at
        .and_then(DateTime::<Utc>::from_timestamp_millis)
        .map(|at| format!("Updated {}", crate::pull_requests::relative_time(at, now)));
    div()
        .flex()
        .flex_col()
        .gap(px(4.0))
        .text_color(theme.text_muted)
        .child(
            div()
                .id(id)
                .debug_selector(move || selector.clone())
                .role(gpui::Role::Button)
                .aria_label("Details")
                .aria_expanded(open)
                .flex()
                .items_center()
                .gap(px(4.0))
                .cursor_pointer()
                .hover(|style| style.text_color(theme.text))
                .on_click(on_details)
                .child(
                    icon(if open {
                        icons::LUCIDE_CHEVRON_DOWN
                    } else {
                        icons::LUCIDE_CHEVRON_RIGHT
                    })
                    .size(px(12.0))
                    .text_color(theme.text_muted),
                )
                .child("Details"),
        )
        .when(open, |el| {
            el.children(identity.into_iter().chain(fetched).map(|line| {
                div()
                    .pl(px(16.0))
                    .truncate()
                    .child(SharedString::from(line))
            }))
        })
}

pub(crate) fn ring_hover_card(accounts: &[AgentAccount], now: DateTime<Utc>, theme: &Theme) -> Div {
    let updated = accounts
        .iter()
        .filter_map(|account| account.usage_fetched_at)
        .max()
        .and_then(DateTime::<Utc>::from_timestamp_millis)
        .map(|at| {
            format!(
                "used · updated {}",
                crate::pull_requests::relative_time(at, now)
            )
        });
    let binding = tightest(accounts).and_then(|account| {
        account
            .usage_windows
            .iter()
            .max_by(|a, b| a.used_fraction.total_cmp(&b.used_fraction))
    });
    let window_row = |window: &AgentUsageWindow| {
        let used = window.used_fraction.clamp(0.0, 1.0);
        let hot = used >= 1.0;
        let sub = binding
            .is_some_and(|binding| std::ptr::eq(binding, window))
            .then(|| match (hot, window.resets_at) {
                (true, at) => Some(match accounts::format_reset(at, now) {
                    Some(reset) => format!("Limit reached · {reset}"),
                    None => "Limit reached".to_owned(),
                }),
                (false, Some(at)) => zeron_proto::account_usage::resets_in(at, now)
                    .map(|left| format!("Resets in {left}")),
                (false, None) => None,
            })
            .flatten();
        let tone = if hot { theme.danger } else { theme.text_muted };
        div()
            .flex()
            .flex_col()
            .gap(px(2.0))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .child(
                        div()
                            .w(px(62.0))
                            .flex_none()
                            .truncate()
                            .text_color(tone)
                            .child(SharedString::from(
                                zeron_proto::account_usage::window_label(&window.label).to_owned(),
                            )),
                    )
                    .child(
                        div()
                            .flex_1()
                            .h(px(4.0))
                            .rounded_full()
                            .overflow_hidden()
                            .bg(theme.wash(0.08))
                            .child(
                                div()
                                    .h_full()
                                    .w(gpui::relative(used))
                                    .rounded_full()
                                    .bg(usage_color(usage_level(used), theme)),
                            ),
                    )
                    .child(
                        div()
                            .w(px(44.0))
                            .flex_none()
                            .flex()
                            .justify_end()
                            .text_color(if hot { theme.danger } else { theme.text })
                            .child(SharedString::from(format!(
                                "{}%",
                                (used * 100.0).round() as u32
                            ))),
                    ),
            )
            .children(sub.map(|line| {
                div()
                    .pl(px(70.0))
                    .text_color(tone)
                    .child(SharedString::from(line))
            }))
    };
    let account_section = |account: &AgentAccount| {
        let (mark, tint) = crate::pickers::harness_brand_icon(account.harness);
        div()
            .flex()
            .flex_col()
            .gap(px(8.0))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.0))
                    .child(
                        icon(mark)
                            .size(px(13.0))
                            .text_color(tint.unwrap_or(theme.text)),
                    )
                    .child(
                        div()
                            .flex_none()
                            .text_size(crate::typography::ui_rems(13.0))
                            .font_weight(gpui::FontWeight::MEDIUM)
                            .child(accounts::provider_name(account.harness)),
                    )
                    .children(account.plan_label.clone().map(|plan| {
                        div()
                            .flex_none()
                            .text_color(theme.text_muted)
                            .child(SharedString::from(plan))
                    }))
                    .children(
                        account
                            .email
                            .clone()
                            .or_else(|| account.display_name.clone())
                            .map(|who| {
                                div()
                                    .ml_auto()
                                    .min_w_0()
                                    .truncate()
                                    .text_color(theme.text_muted)
                                    .child(SharedString::from(who))
                            }),
                    ),
            )
            .children(account.usage_error.clone().map(|error| {
                div()
                    .text_color(theme.warning)
                    .child(SharedString::from(error))
            }))
            .children(account.usage_windows.iter().map(window_row))
    };
    div()
        .debug_selector(|| "rail-usage-hover-card".into())
        .w(px(HOVER_CARD_WIDTH))
        .rounded(px(8.0))
        .border_1()
        .border_color(theme.text.opacity(0.1))
        .bg(popover::surface_bg(theme))
        .flex()
        .flex_col()
        .text_size(crate::typography::ui_rems(11.5))
        .line_height(px(16.0))
        .text_color(theme.text)
        .child(
            div()
                .p(px(10.0))
                .flex()
                .flex_col()
                .gap(px(12.0))
                .child(
                    div()
                        .flex()
                        .items_baseline()
                        .justify_between()
                        .gap(px(8.0))
                        .child(
                            div()
                                .text_size(crate::typography::ui_rems(13.0))
                                .font_weight(gpui::FontWeight::MEDIUM)
                                .child("Usage"),
                        )
                        .children(updated.map(|line| {
                            div()
                                .text_color(theme.text_muted)
                                .child(SharedString::from(line))
                        })),
                )
                .children(accounts.iter().map(account_section)),
        )
        .child(popover::menu_separator())
        .child(
            div()
                .px(px(10.0))
                .py(px(8.0))
                .flex()
                .items_center()
                .justify_between()
                .text_color(theme.text_muted)
                .child("Click the ring for the Usage page")
                .child(
                    icon(icons::LUCIDE_CHEVRON_RIGHT)
                        .size(px(12.0))
                        .text_color(theme.text_muted),
                ),
        )
}

#[cfg(test)]
mod tests {
    use super::*;
    use zeron_proto::{AgentUsageWindow, HarnessId};

    fn account(harness: HarnessId, active: bool, used: &[f32]) -> AgentAccount {
        serde_json::from_value(serde_json::json!({
            "id": format!("{harness:?}-{active}"),
            "harness": harness,
            "email": null,
            "planLabel": null,
            "active": active,
            "switchable": true,
            "usageWindows": used
                .iter()
                .map(|fraction| AgentUsageWindow {
                    label: "Week".into(),
                    used_fraction: *fraction,
                    resets_at: None,
                })
                .collect::<Vec<_>>(),
        }))
        .unwrap()
    }

    #[test]
    fn the_ring_follows_the_tightest_window_across_reporting_accounts() {
        let snapshot = AgentAccountsSnapshot {
            accounts: vec![
                account(HarnessId::Cursor, true, &[0.4]),
                account(HarnessId::Antigravity, true, &[0.9]),
                account(HarnessId::Codex, true, &[0.2]),
                account(HarnessId::ClaudeCode, false, &[0.7]),
                account(HarnessId::ClaudeCode, true, &[]),
            ],
            warnings: Vec::new(),
        };
        let shown: Vec<AgentAccount> = usage_accounts(&snapshot).into_iter().cloned().collect();
        let harnesses: Vec<_> = shown.iter().map(|account| account.harness).collect();
        assert_eq!(harnesses, vec![HarnessId::Codex, HarnessId::Cursor]);
        assert_eq!(
            tightest(&shown).map(|account| account.harness),
            Some(HarnessId::Cursor)
        );
    }
}
