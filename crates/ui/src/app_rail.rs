//! The app rail: a narrow column at the window's left edge, below the
//! titlebar, holding the top-level destinations and the plan-usage rings.
use chrono::{DateTime, Utc};
use gpui::{AnyElement, Div, SharedString, Stateful, div, prelude::*, px};
use zeron_proto::{AgentAccount, AgentAccountsSnapshot};

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
const CARD_WIDTH: f32 = 280.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RailItem {
    Home,
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

pub(crate) const TOP: [RailEntry; 3] = [
    RailEntry {
        item: RailItem::Home,
        id: "rail-home",
        label: "Home",
        icon: icons::HOME,
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

/// A rail button: hover wash fading in, held while `active`.
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

/// The accounts the rail rings show: the active account of each of the first
/// two usage-reporting harnesses with a landed probe. Pure.
pub(crate) fn ring_accounts(snapshot: &AgentAccountsSnapshot) -> Vec<&AgentAccount> {
    accounts::sidebar_accounts(snapshot)
        .into_iter()
        .filter(|account| accounts::reports_usage(account.harness))
        .take(MAX_RINGS)
        .collect()
}

/// One ring's tooltip: the provider with its window readings, or why there
/// is nothing to read.
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

/// The ring glyph: a used-fraction arc around the harness mark. Without an
/// account (nothing probed yet) only the track shows.
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

/// The ring's popover: the account, one bar per usage window with its
/// reset, and `footer` (the "Open Usage" row) underneath.
pub(crate) fn ring_card(
    account: &AgentAccount,
    footer: AnyElement,
    theme: &Theme,
    now: DateTime<Utc>,
) -> Div {
    let (mark, tint) = crate::pickers::harness_brand_icon(account.harness);
    let email = account
        .email
        .clone()
        .or_else(|| account.display_name.clone());
    popover::popover_card(theme)
        .w(px(CARD_WIDTH))
        .flex()
        .flex_col()
        .gap(px(2.0))
        .child(
            div()
                .px(px(8.0))
                .pt(px(6.0))
                .flex()
                .flex_col()
                .gap(px(5.0))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(6.0))
                        .text_size(crate::typography::ui_rems(12.0))
                        .font_weight(gpui::FontWeight::MEDIUM)
                        .text_color(theme.text)
                        .child(
                            icon(mark)
                                .size(px(12.0))
                                .text_color(tint.unwrap_or(theme.text_muted)),
                        )
                        .child(SharedString::from(accounts::provider_name(account.harness)))
                        .when_some(account.plan_label.clone(), |row, plan| {
                            row.child(crate::settings::widgets::badge(theme, plan).ml_auto())
                        }),
                )
                .when_some(email, |col, email| {
                    col.child(
                        div()
                            .mt(px(-2.0))
                            .text_size(crate::typography::ui_rems(10.5))
                            .text_color(theme.text_muted.opacity(0.5))
                            .truncate()
                            .child(SharedString::from(email)),
                    )
                })
                .children(
                    account
                        .usage_windows
                        .iter()
                        .map(|window| accounts::compact_usage_meter(window, theme, now)),
                )
                .when_some(account.usage_error.clone(), |col, error| {
                    col.child(
                        div()
                            .text_size(crate::typography::ui_rems(11.0))
                            .text_color(theme.text_muted)
                            .child(SharedString::from(error)),
                    )
                })
                .pb(px(4.0)),
        )
        .child(popover::menu_separator())
        .child(footer)
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
