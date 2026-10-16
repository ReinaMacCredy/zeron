use chrono::{DateTime, Utc};
use gpui::{AnyElement, Div, SharedString, Stateful, div, prelude::*, px};
use zeron_proto::{AgentAccount, AgentAccountsSnapshot, HarnessId};

use crate::account_usage::used_fraction;
use crate::icons::{self, icon};
use crate::motion;
use crate::popover;
use crate::settings::accounts::{self, usage_color, usage_level};
use crate::theme::Theme;

pub(crate) const WIDTH: f32 = 46.0;
const BUTTON: f32 = 34.0;
const GLYPH: f32 = 18.0;
pub(crate) const GAP: f32 = 6.0;
pub(crate) const PAD_Y: f32 = 8.0;
const RING_RADIUS: f32 = 13.0;
const RING_STROKE: f32 = 2.5;
const RING_GLYPH: f32 = 14.0;
const MAX_RINGS: usize = 2;
const CARD_WIDTH: f32 = 400.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RailItem {
    Home,
    Tasks,
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

pub(crate) const TOP: [RailEntry; 4] = [
    RailEntry {
        item: RailItem::Home,
        id: "rail-home",
        label: "Home",
        icon: icons::HOME,
    },
    RailEntry {
        item: RailItem::Tasks,
        id: "open-tasks",
        label: "Tasks",
        icon: icons::LUCIDE_LIST_CHECKS,
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

pub(crate) fn button(entry: &RailEntry, active: bool, theme: &Theme) -> Stateful<Div> {
    let id = entry.id;
    div()
        .id(id)
        .debug_selector(move || id.into())
        .role(gpui::Role::Button)
        .aria_label(entry.label)
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
            theme.glass_hover()
        } else {
            motion::hover_blend(id, theme.glass_hover().opacity(0.0), theme.glass_hover())
        })
        .on_hover(motion::hover_listener(id))
        .child(icon(entry.icon).size(px(GLYPH)).text_color(if active {
            theme.text
        } else {
            motion::hover_blend(id, theme.text_muted, theme.text)
        }))
}

pub(crate) fn count_badge(count: usize, theme: &Theme) -> Div {
    let label = if count > 99 { "99+".to_string() } else { count.to_string() };
    div()
        .absolute()
        .top(px(-2.0))
        .right(px(-3.0))
        .min_w(px(16.0))
        .h(px(16.0))
        .px(px(4.0))
        .flex()
        .items_center()
        .justify_center()
        .rounded_full()
        .bg(theme.accent)
        .text_size(px(10.0))
        .font_weight(gpui::FontWeight::SEMIBOLD)
        .text_color(gpui::white())
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

pub(crate) fn ring(account: Option<&AgentAccount>, theme: &Theme) -> Div {
    let fraction = account.and_then(used_fraction);
    let color = usage_color(usage_level(fraction.unwrap_or(0.0)), theme);
    let (mark, tint) = account.map_or((icons::USAGE_CHART, None), |account| {
        crate::pickers::harness_brand_icon(account.harness)
    });
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
                    fraction.unwrap_or(0.0),
                    color,
                    theme.text_faint.opacity(0.25),
                )),
        )
        .child(
            icon(mark)
                .size(px(RING_GLYPH))
                .text_color(tint.unwrap_or(theme.text))
                .when(fraction.is_none(), |glyph| glyph.opacity(0.5)),
        )
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
    use zeron_proto::AgentUsageWindow;

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
