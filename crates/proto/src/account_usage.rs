//! Agent accounts: plan-usage meters (the binding window, Extra Usage, the
//! reserve/deficit pace), sign-in options and labels. Shared by the desktop
//! and the phone.
use chrono::{DateTime, Utc};

use crate::{AgentAccount, AgentAccountsSnapshot, AgentUsageWindow, HarnessId};

/// "2h 14m" until `resets_at`; `None` once it has passed.
pub fn resets_in(resets_at: DateTime<Utc>, now: DateTime<Utc>) -> Option<String> {
    let seconds = (resets_at - now).num_seconds();
    if seconds <= 0 {
        return None;
    }
    let minutes = (seconds + 59) / 60;
    let (hours, days) = (minutes / 60, minutes / (60 * 24));
    Some(match () {
        _ if minutes < 60 => format!("{minutes}m"),
        _ if hours < 24 => match minutes % 60 {
            0 => format!("{hours}h"),
            m => format!("{hours}h {m}m"),
        },
        _ => match hours % 24 {
            0 => format!("{days}d"),
            h => format!("{days}d {h}h"),
        },
    })
}

/// The binding limit: the most-used window of the account. Pure.
pub fn used_fraction(account: &AgentAccount) -> Option<f32> {
    account
        .usage_windows
        .iter()
        .map(|window| window.used_fraction.clamp(0.0, 1.0))
        .reduce(f32::max)
}

pub fn active_account(
    snapshot: &AgentAccountsSnapshot,
    harness: HarnessId,
) -> Option<&AgentAccount> {
    snapshot
        .accounts
        .iter()
        .find(|account| account.harness == harness && account.active)
}

#[derive(Debug, Clone, PartialEq)]
pub struct ExtraUsageView {
    pub used_fraction: f32,
    pub headline: String,
    pub amount: String,
    pub note: Option<String>,
}

fn money(amount: f64, currency: Option<&str>) -> String {
    match currency.unwrap_or("USD") {
        "USD" => format!("${amount:.2}"),
        "EUR" => format!("€{amount:.2}"),
        "GBP" => format!("£{amount:.2}"),
        other => format!("{amount:.2} {other}"),
    }
}

pub fn extra_usage_view(extra: &crate::AgentExtraUsage) -> ExtraUsageView {
    let currency = extra.currency.as_deref();
    match (extra.enabled, extra.used) {
        (true, Some(used)) => {
            let fraction = extra
                .limit
                .map_or(0.0, |limit| (used / limit).clamp(0.0, 1.0) as f32);
            ExtraUsageView {
                used_fraction: fraction,
                headline: match extra.limit {
                    Some(_) => format!("{}% used", (fraction * 100.0).round() as u32),
                    None => "No cap".to_owned(),
                },
                amount: match extra.limit {
                    Some(limit) => {
                        format!("{} of {}", money(used, currency), money(limit, currency))
                    }
                    None => format!("{} spent", money(used, currency)),
                },
                note: None,
            }
        }
        (enabled, _) => ExtraUsageView {
            used_fraction: 0.0,
            headline: "No data".to_owned(),
            amount: "—".to_owned(),
            note: (!enabled).then(|| "Off for this account".to_owned()),
        },
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PaceStatus {
    Ahead,
    OnTrack,
    Behind,
}

#[derive(Debug, Clone, PartialEq)]
pub struct UsagePace {
    pub status: PaceStatus,
    pub expected_remaining: f64,
    pub amount: Option<String>,
    pub eta: Option<String>,
}

const MIN_PROJECTION_ELAPSED: f64 = 0.05;

fn window_minutes(label: &str) -> Option<f64> {
    match label {
        "Session" => Some(300.0),
        "Week" => Some(10_080.0),
        "Day" => Some(1_440.0),
        "Month" => Some(43_200.0),
        _ => None,
    }
}

/// Ported from Synara's `deriveUsagePace` (apps/web/src/lib/usagePace.ts).
pub fn usage_pace(window: &AgentUsageWindow, now: DateTime<Utc>) -> Option<UsagePace> {
    let resets_at = window.resets_at?;
    let duration_ms = window_minutes(&window.label)? * 60_000.0;
    let remaining_ms = (resets_at - now).num_milliseconds() as f64;
    let elapsed_ms = duration_ms - remaining_ms;
    if elapsed_ms <= 0.0 || remaining_ms <= 0.0 {
        return None;
    }
    let used = (f64::from(window.used_fraction) * 100.0).clamp(0.0, 100.0);
    let elapsed = (elapsed_ms / duration_ms).max(MIN_PROJECTION_ELAPSED);
    let expected_used = (elapsed * 100.0).clamp(0.0, 100.0);
    let projected = if used == 0.0 { 0.0 } else { used / elapsed };
    let status = if used >= 100.0 {
        PaceStatus::Behind
    } else if used == 0.0 || projected <= 80.0 {
        PaceStatus::Ahead
    } else if projected <= 100.0 {
        PaceStatus::OnTrack
    } else {
        PaceStatus::Behind
    };
    let delta = used - expected_used;
    let amount = match delta.abs().round() as u32 {
        0 => None,
        n if delta > 0.0 => Some(format!("{n}% in deficit")),
        n => Some(format!("{n}% in reserve")),
    };
    let eta = match status {
        PaceStatus::Behind if used >= 100.0 => Some("Limit reached".to_owned()),
        PaceStatus::Behind => {
            let eta_ms = (100.0 - used) * duration_ms / projected;
            (eta_ms > 0.0 && eta_ms < remaining_ms)
                .then(|| resets_in(now + chrono::Duration::milliseconds(eta_ms as i64), now))
                .flatten()
                .map(|left| format!("Runs out in {left}"))
        }
        _ => Some("Lasts until reset".to_owned()),
    };
    Some(UsagePace {
        status,
        expected_remaining: 100.0 - expected_used,
        amount,
        eta,
    })
}

pub const USAGE_WARN_FRACTION: f32 = 0.80;
pub const USAGE_CRITICAL_FRACTION: f32 = 0.95;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UsageLevel {
    /// < 80% — indigo.
    Normal,
    /// ≥ 80% — amber.
    Warn,
    /// ≥ 95% — red.
    Critical,
}

/// Threshold classification of a usage fraction. Pure.
pub fn usage_level(fraction: f32) -> UsageLevel {
    if fraction >= USAGE_CRITICAL_FRACTION {
        UsageLevel::Critical
    } else if fraction >= USAGE_WARN_FRACTION {
        UsageLevel::Warn
    } else {
        UsageLevel::Normal
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct UsageBlock {
    pub label: String,
    pub headline: Option<String>,
    pub resets: Option<String>,
    pub fill: f32,
    pub level: UsageLevel,
    pub marker: Option<(f32, UsageLevel)>,
    pub foot_left: Option<String>,
    pub foot_right: Option<String>,
    pub foot_left_alert: bool,
    pub foot_right_alert: bool,
}

pub fn window_label(label: &str) -> &str {
    match label {
        "Session" => "5h",
        "Week" => "Weekly",
        "Day" => "Daily",
        other => other,
    }
}

fn window_block(window: &AgentUsageWindow, now: DateTime<Utc>) -> UsageBlock {
    let used = window.used_fraction.clamp(0.0, 1.0);
    let remaining = 1.0 - used;
    let pace = usage_pace(window, now);
    let marker = pace
        .as_ref()
        .map(|pace| (pace.expected_remaining as f32 / 100.0).clamp(0.0, 1.0))
        .filter(|_| remaining > 0.0 && remaining < 1.0)
        .map(|at| {
            let level = match pace.as_ref().map(|pace| pace.status) {
                Some(PaceStatus::Behind) => UsageLevel::Critical,
                Some(PaceStatus::OnTrack) => UsageLevel::Warn,
                _ => UsageLevel::Normal,
            };
            (at, level)
        });
    let foot = pace.filter(|pace| pace.amount.is_some() || pace.eta.is_some());
    UsageBlock {
        label: window_label(&window.label).to_owned(),
        headline: Some(format!("{}% left", (remaining * 100.0).round() as u32)),
        resets: window
            .resets_at
            .and_then(|at| resets_in(at, now))
            .map(|left| format!("Resets in {left}")),
        fill: remaining,
        level: usage_level(used),
        marker,
        foot_left_alert: foot
            .as_ref()
            .is_some_and(|pace| f64::from(remaining) * 100.0 < pace.expected_remaining),
        foot_right_alert: foot
            .as_ref()
            .is_some_and(|pace| pace.status == PaceStatus::Behind),
        foot_left: foot.as_ref().and_then(|pace| pace.amount.clone()),
        foot_right: foot.and_then(|pace| pace.eta),
    }
}

fn extra_usage_block(extra: &crate::AgentExtraUsage) -> UsageBlock {
    let view = extra_usage_view(extra);
    UsageBlock {
        label: "Extra Usage".into(),
        headline: None,
        resets: Some(view.headline),
        fill: view.used_fraction,
        level: usage_level(view.used_fraction),
        marker: None,
        foot_left: Some(view.amount),
        foot_right: view.note,
        foot_left_alert: false,
        foot_right_alert: false,
    }
}

/// One block per plan window, then Claude's Extra Usage.
pub fn usage_blocks(account: &AgentAccount, now: DateTime<Utc>) -> Vec<UsageBlock> {
    account
        .usage_windows
        .iter()
        .map(|window| window_block(window, now))
        .chain(account.extra_usage.as_ref().map(extra_usage_block))
        .collect()
}

/// Whether a login URL may be opened. Remote logins relay their URL from
/// another device (or from a CLI's output there), so only web sign-in pages
/// and loopback callbacks pass — never `file://` or custom schemes. Pure.
pub fn is_openable_login_url(url: &str) -> bool {
    let Some((scheme, rest)) = url.split_once("://") else {
        return false;
    };
    match scheme.to_ascii_lowercase().as_str() {
        "https" => true,
        "http" => {
            let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
            let authority = authority
                .rsplit_once('@')
                .map_or(authority, |(_, host)| host);
            let host = match authority.strip_prefix('[') {
                Some(v6) => v6.split(']').next().unwrap_or(""),
                None => authority.split(':').next().unwrap_or(""),
            };
            matches!(host, "localhost" | "127.0.0.1" | "::1")
        }
        _ => false,
    }
}

pub use crate::providers::{
    LoginOption, PROVIDERS, add_account_label, add_option_label, keeps_one_login, login_options,
    provider_name, provider_note, reports_usage, signs_in, switches_accounts,
};

/// What the sign-in dialog says while the browser finishes — one sentence
/// per provider, same shape. Pure.
pub fn login_copy(harness: HarnessId, provider: Option<&str>) -> &'static str {
    match (harness, provider) {
        (HarnessId::ClaudeCode, _) => {
            "Finish signing in to Claude in your browser. The new login is saved next to \
             your current one — nothing changes until you switch."
        }
        (HarnessId::Codex, _) => {
            "Finish signing in to ChatGPT in your browser. The new login is saved next to \
             your current one — nothing changes until you switch."
        }
        (HarnessId::Cursor, _) => {
            "Finish signing in to Cursor in your browser. This mints a zeron-named API key \
             you can revoke any time from Cursor's dashboard."
        }
        (HarnessId::Antigravity, _) => {
            "Finish signing in to Google in your browser. Antigravity keeps one login on \
             this device; if it is already signed in, this just confirms it."
        }
        (HarnessId::Grok, _) => {
            "Finish signing in to Grok in your browser — approve the code shown below. The \
             new login is saved next to your current one — nothing changes until you switch."
        }
        (HarnessId::Devin, _) => {
            "Finish signing in to Devin in your browser. The new login is saved next to your \
             current one — nothing changes until you switch."
        }
        (HarnessId::Opencode, Some("github-copilot")) => {
            "Finish signing in to GitHub in your browser — enter the code shown below. The \
             new login is saved next to your current one — nothing changes until you switch."
        }
        (HarnessId::Opencode | HarnessId::Pi, _) => {
            "Finish signing in to ChatGPT in your browser. The agent gets its own login, saved \
             next to any current one — nothing changes until you switch."
        }
        (HarnessId::Hermes, _) => {
            "Finish signing in in your browser — enter the code shown below. Hermes adds the \
             login to its own credential pool and rotates through it itself."
        }
        _ => "Finish signing in in your browser.",
    }
}

/// Accounts of one provider, in the engine's order (slot creation). No
/// active-first re-sort: switching accounts must not move the switched-to
/// card — the Active badge already says which one is live, and a list that
/// reshuffles under the click reads as broken. Pure.
pub fn provider_accounts(
    snapshot: &AgentAccountsSnapshot,
    harness: HarnessId,
) -> Vec<&AgentAccount> {
    snapshot
        .accounts
        .iter()
        .filter(|a| a.harness == harness)
        .collect()
}

/// The accounts the sidebar user row summarizes (docs/research/usage-mini):
/// per provider in [`PROVIDERS`] order, the active account, and only when a
/// usage probe has landed — a cold engine cache draws nothing rather than a
/// row of "0%". Pure.
pub fn sidebar_accounts(snapshot: &AgentAccountsSnapshot) -> Vec<&AgentAccount> {
    PROVIDERS
        .iter()
        .filter_map(|(harness, _, _)| {
            snapshot
                .accounts
                .iter()
                .find(|a| a.harness == *harness && a.active && !a.usage_windows.is_empty())
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

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
                    label: "5h".into(),
                    used_fraction: *fraction,
                    resets_at: None,
                })
                .collect::<Vec<_>>(),
        }))
        .unwrap()
    }

    #[test]
    fn ring_shows_the_most_used_window() {
        assert_eq!(used_fraction(&account(HarnessId::Codex, true, &[])), None);
        assert_eq!(
            used_fraction(&account(HarnessId::Codex, true, &[0.12, 0.64])),
            Some(0.64)
        );
        assert_eq!(
            used_fraction(&account(HarnessId::Codex, true, &[1.4])),
            Some(1.0)
        );
    }

    fn pace(label: &str, used: f32, elapsed: f64) -> Option<UsagePace> {
        use chrono::TimeZone;
        let resets_at = Utc.with_ymd_and_hms(2026, 6, 15, 18, 0, 0).unwrap();
        let minutes = window_minutes(label).unwrap_or(300.0);
        let left = chrono::Duration::milliseconds((minutes * 60_000.0 * (1.0 - elapsed)) as i64);
        let window = AgentUsageWindow {
            label: label.into(),
            used_fraction: used,
            resets_at: Some(resets_at),
        };
        usage_pace(&window, resets_at - left)
    }

    #[test]
    fn pace_reads_reserve_deficit_and_run_out_like_synara() {
        let ahead = pace("Session", 0.15, 0.88).unwrap();
        assert_eq!(ahead.status, PaceStatus::Ahead);
        assert!((ahead.expected_remaining - 12.0).abs() < 0.01);
        assert_eq!(ahead.amount.as_deref(), Some("73% in reserve"));
        assert_eq!(ahead.eta.as_deref(), Some("Lasts until reset"));

        let early = pace("Session", 0.09, 0.02).unwrap();
        assert_eq!(early.status, PaceStatus::Behind);
        assert!((early.expected_remaining - 95.0).abs() < 0.01);
        assert_eq!(early.amount.as_deref(), Some("4% in deficit"));

        let behind = pace("Week", 0.82, 0.78).unwrap();
        assert_eq!(behind.status, PaceStatus::Behind);
        assert_eq!(behind.amount.as_deref(), Some("4% in deficit"));
        assert_eq!(behind.eta.as_deref(), Some("Runs out in 1d 4h"));

        assert_eq!(
            pace("Week", 1.0, 0.5).unwrap().eta.as_deref(),
            Some("Limit reached")
        );
        assert_eq!(pace("Session", 0.496, 0.5).unwrap().amount, None);
        assert_eq!(
            pace("Month", 0.38, 0.33).unwrap().amount.as_deref(),
            Some("5% in deficit")
        );
        assert_eq!(pace("Premium", 0.5, 0.5), None);
        assert_eq!(pace("Session", 0.5, 1.0), None);
    }

    #[test]
    fn extra_usage_reads_like_synara() {
        let extra = |enabled, used, limit, currency: Option<&str>| crate::AgentExtraUsage {
            enabled,
            used,
            limit,
            currency: currency.map(str::to_owned),
        };
        let on = extra_usage_view(&extra(true, Some(5.0), Some(100.0), None));
        assert_eq!(on.amount, "$5.00 of $100.00");
        assert_eq!(on.headline, "5% used");
        assert!((on.used_fraction - 0.05).abs() < 1e-6);
        let off = extra_usage_view(&extra(false, Some(0.0), Some(40.0), Some("EUR")));
        assert_eq!(
            (
                off.headline.as_str(),
                off.amount.as_str(),
                off.note.as_deref()
            ),
            ("No data", "—", Some("Off for this account"))
        );
        assert_eq!(
            extra_usage_view(&extra(true, Some(12.5), None, Some("EUR"))).amount,
            "€12.50 spent"
        );
    }

    #[test]
    fn active_account_is_scoped_to_the_harness() {
        let snapshot = AgentAccountsSnapshot {
            accounts: vec![
                account(HarnessId::ClaudeCode, true, &[0.3]),
                account(HarnessId::Codex, false, &[0.1]),
                account(HarnessId::Codex, true, &[0.2]),
            ],
            warnings: Vec::new(),
        };
        assert_eq!(
            active_account(&snapshot, HarnessId::Codex).map(|a| a.id.as_str()),
            Some("Codex-true")
        );
        assert!(active_account(&snapshot, HarnessId::Cursor).is_none());
    }

    #[test]
    fn resets_in_reads_minutes_hours_then_days() {
        let now = Utc.with_ymd_and_hms(2026, 10, 17, 12, 0, 0).unwrap();
        let later = |secs: i64| now + chrono::Duration::seconds(secs);
        assert_eq!(resets_in(now, now), None);
        assert_eq!(resets_in(later(-60), now), None);
        assert_eq!(resets_in(later(30), now).as_deref(), Some("1m"));
        assert_eq!(resets_in(later(59 * 60), now).as_deref(), Some("59m"));
        assert_eq!(resets_in(later(3600), now).as_deref(), Some("1h"));
        assert_eq!(
            resets_in(later(2 * 3600 + 14 * 60), now).as_deref(),
            Some("2h 14m")
        );
        assert_eq!(resets_in(later(24 * 3600), now).as_deref(), Some("1d"));
        assert_eq!(
            resets_in(later(3 * 86_400 + 4 * 3600), now).as_deref(),
            Some("3d 4h")
        );
    }
}
