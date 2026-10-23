use chrono::{DateTime, Utc};
use gpui::{AnyElement, Div, Hsla, SharedString, Stateful, div, prelude::*, px};
use zeron_proto::account_usage::UsageBlock;
use zeron_proto::{AgentAccount, AgentAccountsSnapshot, AgentUsageWindow, HarnessId};

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
const DOUBLE_RING_STROKE: f32 = 2.25;
const DOUBLE_RING_RADIUS: f32 = (BUTTON - DOUBLE_RING_STROKE) / 2.0;
const DOUBLE_RING_SPACING: f32 = 4.5;
const DOUBLE_RING_GLYPH: f32 = 12.0;
const MAX_RINGS: usize = 2;
const CARD_WIDTH: f32 = 400.0;
const HOVER_CARD_WIDTH: f32 = 272.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RailItem {
    Home,
    Inbox,
    Tasks,
    MissionControl,
    Supervisor,
    PullRequests,
    Usage,
    Settings,
}

pub(crate) struct RailEntry {
    pub item: RailItem,
    pub id: &'static str,
    pub label: &'static str,
    pub icon: &'static str,
}

pub(crate) const TOP: [RailEntry; 7] = [
    RailEntry {
        item: RailItem::Home,
        id: "rail-home",
        label: "Home",
        icon: icons::HOME,
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
        item: RailItem::MissionControl,
        id: "open-mission-control",
        label: "Mission Control",
        icon: icons::MISSION_CONTROL,
    },
    RailEntry {
        item: RailItem::Supervisor,
        id: "open-supervisor",
        label: "Supervisor",
        icon: icons::LUCIDE_SHIELD,
    },
    RailEntry {
        item: RailItem::PullRequests,
        id: "open-pull-requests",
        label: "Pull requests",
        icon: icons::LUCIDE_GIT_PULL_REQUEST,
    },
    RailEntry {
        item: RailItem::Usage,
        id: "open-usage",
        label: "Usage",
        icon: icons::LUCIDE_CHART_NO_AXES_COLUMN,
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

pub(crate) fn badge(count: usize, scale: f32, theme: &Theme) -> Div {
    let label = if count > 99 {
        "99+".to_owned()
    } else {
        count.to_string()
    };
    let grow = 7.5 * (scale - 1.0);
    div()
        .absolute()
        .top(px(1.0 - grow))
        .right(px(-grow))
        .min_w(px(15.0 * scale))
        .h(px(15.0 * scale))
        .px(px(4.0 * scale))
        .rounded_full()
        .bg(theme.accent)
        .text_color(theme.on_accent)
        .text_size(crate::typography::ui_rems(9.5 * scale))
        .font_weight(gpui::FontWeight::SEMIBOLD)
        .flex()
        .items_center()
        .justify_center()
        .child(SharedString::from(label))
}

pub(crate) fn ring_accounts(snapshot: &AgentAccountsSnapshot) -> Vec<&AgentAccount> {
    accounts::sidebar_accounts(snapshot)
        .into_iter()
        .filter(|account| accounts::reports_usage(account.harness))
        .take(MAX_RINGS)
        .collect()
}

pub(crate) fn ring_tooltip(account: Option<&AgentAccount>, loading: bool) -> SharedString {
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
                window.label,
                (window.used_fraction.clamp(0.0, 1.0) * 100.0).round() as u32
            )
        })
        .collect::<Vec<_>>();
    match (windows.is_empty(), account.usage_error.as_deref()) {
        (false, _) => format!("{name} · {}", windows.join(", ")).into(),
        (true, Some(error)) => format!("{name} · {error}").into(),
        (true, None) => format!("{name} · no usage reported").into(),
    }
}

/// Outermost first, like Synara: the week ring around the session ring.
fn ring_windows(account: &AgentAccount) -> Option<[&AgentUsageWindow; 2]> {
    let find = |label: &str| account.usage_windows.iter().find(|w| w.label == label);
    if let (Some(week), Some(session)) = (find("Week"), find("Session")) {
        return Some([week, session]);
    }
    match account.usage_windows.as_slice() {
        [first, second] if first.label == "Week" => Some([first, second]),
        [first, second] => Some([second, first]),
        _ => None,
    }
}

pub(crate) fn ring(account: Option<&AgentAccount>, theme: &Theme) -> Div {
    let fraction = account.and_then(used_fraction);
    let (mark, tint) = account.map_or((icons::USAGE_CHART, None), |account| {
        crate::pickers::harness_brand_icon(account.harness)
    });
    let track = theme.text_faint.opacity(0.25);
    let arc = |radius: f32, stroke: f32, fraction: f32| {
        div()
            .absolute()
            .inset_0()
            .child(crate::context_usage::progress_ring(
                BUTTON,
                radius,
                stroke,
                fraction,
                usage_color(usage_level(fraction), theme),
                track,
            ))
    };
    let pair = account.and_then(ring_windows);
    let rings: Vec<Div> = match pair {
        Some(windows) => windows
            .iter()
            .enumerate()
            .map(|(ix, window)| {
                arc(
                    DOUBLE_RING_RADIUS - ix as f32 * DOUBLE_RING_SPACING,
                    DOUBLE_RING_STROKE,
                    window.used_fraction.clamp(0.0, 1.0),
                )
            })
            .collect(),
        None => vec![arc(RING_RADIUS, RING_STROKE, fraction.unwrap_or(0.0))],
    };
    div()
        .size(px(BUTTON))
        .relative()
        .flex()
        .items_center()
        .justify_center()
        .children(rings)
        .child(
            icon(mark)
                .size(px(if pair.is_some() {
                    DOUBLE_RING_GLYPH
                } else {
                    RING_GLYPH
                }))
                .text_color(tint.unwrap_or(theme.text))
                .when(fraction.is_none(), |glyph| glyph.opacity(0.5)),
        )
}

/// The card stays while the ring or the card holds the pointer, so Details is reachable.
#[derive(Default)]
pub(crate) struct RingHover {
    pub harness: Option<HarnessId>,
    pub trigger: bool,
    pub card: bool,
    pub details: bool,
}

impl RingHover {
    pub fn shown(&self) -> Option<HarnessId> {
        self.harness.filter(|_| self.trigger || self.card)
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

/// One block per plan window, then Claude's Extra Usage: the hover card and the Usage page.
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

pub(crate) fn ring_hover_card(
    account: &AgentAccount,
    details_open: bool,
    on_details: impl Fn(&gpui::ClickEvent, &mut gpui::Window, &mut gpui::App) + 'static,
    now: DateTime<Utc>,
    theme: &Theme,
) -> Div {
    let legend = ring_windows(account).map(|windows| {
        div()
            .flex()
            .items_center()
            .gap(px(12.0))
            .text_color(theme.text_muted)
            .children(
                windows
                    .iter()
                    .zip(["outer", "inner"])
                    .map(|(window, ring)| {
                        div()
                            .flex()
                            .items_center()
                            .gap(px(6.0))
                            .child(div().size(px(6.0)).rounded_full().bg(usage_color(
                                usage_level(window.used_fraction.clamp(0.0, 1.0)),
                                theme,
                            )))
                            .child(format!(
                                "{} · {ring}",
                                zeron_proto::account_usage::window_label(&window.label)
                            ))
                    }),
            )
    });
    let details = details_fold(
        "rail-usage-hover-details".into(),
        "rail-usage-hover-details".into(),
        account,
        details_open,
        on_details,
        now,
        theme,
    );
    div()
        .debug_selector(|| "rail-usage-hover-card".into())
        .w(px(HOVER_CARD_WIDTH))
        .rounded(px(8.0))
        .border_1()
        .border_color(theme.text.opacity(0.1))
        .bg(popover::surface_bg(theme))
        .p(px(10.0))
        .flex()
        .flex_col()
        .gap(px(8.0))
        .text_size(crate::typography::ui_rems(11.5))
        .line_height(px(16.0))
        .text_color(theme.text)
        .child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .gap(px(8.0))
                .child(
                    div()
                        .min_w_0()
                        .truncate()
                        .text_size(crate::typography::ui_rems(13.0))
                        .font_weight(gpui::FontWeight::MEDIUM)
                        .child(accounts::provider_name(account.harness)),
                )
                .children(account.plan_label.clone().map(|plan| {
                    div()
                        .flex_none()
                        .text_size(crate::typography::ui_rems(12.0))
                        .text_color(theme.text_muted)
                        .child(SharedString::from(plan))
                })),
        )
        .children(legend)
        .children(account.usage_error.clone().map(|error| {
            div()
                .text_color(theme.warning)
                .child(SharedString::from(error))
        }))
        .children(usage_blocks(account, now, theme))
        .child(details)
}

pub(crate) fn ring_card(
    harness: HarnessId,
    rows: Vec<AnyElement>,
    error: Option<SharedString>,
    footer: AnyElement,
    theme: &Theme,
) -> Div {
    let title = format!("{} accounts", accounts::provider_name(harness));
    popover::popover_card(theme)
        .w(px(CARD_WIDTH))
        .flex()
        .flex_col()
        .child(popover::menu_heading(theme, &title))
        .children(rows)
        .children(error.map(|error| {
            div()
                .px(px(8.0))
                .py(px(4.0))
                .text_size(px(12.0))
                .text_color(theme.danger)
                .child(error)
        }))
        .child(popover::menu_separator())
        .child(footer)
}

pub(crate) fn account_row(
    account: &AgentAccount,
    ix: usize,
    theme: &Theme,
    now: DateTime<Utc>,
) -> Stateful<Div> {
    let email: SharedString = account
        .email
        .clone()
        .or_else(|| account.display_name.clone())
        .unwrap_or_else(|| "Unknown account".into())
        .into();
    let mut meta: Vec<AnyElement> = account
        .plan_label
        .iter()
        .map(|plan| {
            div()
                .child(SharedString::from(plan.clone()))
                .into_any_element()
        })
        .collect();
    if account.active {
        meta.push(
            div()
                .text_color(theme.accent)
                .child(SharedString::from("In use"))
                .into_any_element(),
        );
    } else if let Some(reason) = account
        .usage_error
        .clone()
        .filter(|_| account.usage_windows.is_empty())
    {
        meta.push(div().child(SharedString::from(reason)).into_any_element());
    }
    popover::menu_row(theme, account.active, format!("rail-account-row-{ix}"))
        .id(("rail-account-row", ix))
        .debug_selector(move || format!("rail-account-row-{ix}"))
        .when(account.active || !account.switchable, |row| {
            row.cursor_default()
        })
        .child(
            div()
                .flex_1()
                .min_w_0()
                .child(
                    div()
                        .truncate()
                        .text_size(px(12.5))
                        .font_weight(gpui::FontWeight::MEDIUM)
                        .child(email),
                )
                .when(!meta.is_empty(), |el| {
                    el.child(crate::settings::widgets::meta_line(theme, meta))
                }),
        )
        .child(
            div()
                .id(("rail-account-meters", ix))
                .flex_none()
                .flex()
                .flex_col()
                .gap(px(2.0))
                .when(!account.usage_windows.is_empty(), |meters| {
                    meters.tooltip(crate::settings::widgets::text_tooltip(
                        accounts::usage_resets(&account.usage_windows, now),
                    ))
                })
                .children(
                    account
                        .usage_windows
                        .iter()
                        .take(2)
                        .map(|window| accounts::render_usage_meter(window, theme)),
                ),
        )
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn two_windows_draw_the_week_ring_outside_the_session_ring() {
        let mut claude = account(HarnessId::ClaudeCode, true, &[0.3, 0.8]);
        claude.usage_windows[0].label = "Session".into();
        claude.usage_windows[1].label = "Week".into();
        let labels = ring_windows(&claude).map(|rings| rings.map(|window| window.label.clone()));
        assert_eq!(labels, Some(["Week".to_string(), "Session".to_string()]));
        claude.usage_windows.push(AgentUsageWindow {
            label: "Fable".into(),
            used_fraction: 0.1,
            resets_at: None,
        });
        claude.usage_windows.swap(0, 1);
        let labels = ring_windows(&claude).map(|rings| rings.map(|window| window.label.clone()));
        assert_eq!(labels, Some(["Week".to_string(), "Session".to_string()]));
        assert!(ring_windows(&account(HarnessId::Cursor, true, &[0.4])).is_none());
    }

    #[test]
    fn rings_take_the_first_two_harnesses_with_landed_usage() {
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
        let harnesses: Vec<_> = ring_accounts(&snapshot)
            .into_iter()
            .map(|account| account.harness)
            .collect();
        assert_eq!(harnesses, vec![HarnessId::Codex, HarnessId::Cursor]);
    }
}
