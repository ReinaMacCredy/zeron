//! Plan usage of this device's agent logins, shared with Settings → Accounts
//! through [`AccountsSnapshotCache`].
use std::time::{Duration, Instant};

use gpui::{App, Context, Entity, Task};
use zeron_proto::{AgentAccount, AgentAccountsSnapshot, HarnessId};
use zeron_rpc::methods;

use crate::settings::accounts::AccountsSnapshotCache;
use crate::state::AppState;

/// Forced probes hit the provider; the engine throttles them too, but the
/// rings re-probe on hover, so don't even ask more often than this.
const FORCE_MIN_INTERVAL: Duration = Duration::from_secs(30);
/// Background re-probe: usage moves as turns run.
const POLL_INTERVAL: Duration = Duration::from_secs(5 * 60);

/// The binding limit: the most-used window of the account. Pure.
pub fn used_fraction(account: &AgentAccount) -> Option<f32> {
    account
        .usage_windows
        .iter()
        .map(|window| window.used_fraction.clamp(0.0, 1.0))
        .reduce(f32::max)
}

/// The harness's live account. Pure.
pub fn active_account(
    snapshot: &AgentAccountsSnapshot,
    harness: HarnessId,
) -> Option<&AgentAccount> {
    snapshot
        .accounts
        .iter()
        .find(|account| account.harness == harness && account.active)
}

pub(crate) fn local_snapshot(cx: &App) -> Option<&AgentAccountsSnapshot> {
    cx.try_global::<AccountsSnapshotCache>()?.0.get(&None)
}

pub struct DeviceAccounts {
    state: Entity<AppState>,
    last_forced: Option<Instant>,
    load_task: Option<Task<()>>,
    _poll: Task<()>,
}

impl DeviceAccounts {
    pub fn new(state: Entity<AppState>, cx: &mut Context<Self>) -> Self {
        let poll = cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(POLL_INTERVAL).await;
                if this.update(cx, |accounts, cx| accounts.load(cx)).is_err() {
                    break;
                }
            }
        });
        Self {
            state,
            last_forced: None,
            load_task: None,
            _poll: poll,
        }
    }

    pub fn loading(&self) -> bool {
        self.load_task.is_some()
    }

    /// With nothing cached the plain list paints first: the engine serves its
    /// persisted usage at once.
    pub fn load(&mut self, cx: &mut Context<Self>) {
        if self
            .last_forced
            .is_some_and(|at| at.elapsed() < FORCE_MIN_INTERVAL)
        {
            return;
        }
        let Some(engine) = self.state.read(cx).engine().cloned() else {
            return;
        };
        self.last_forced = Some(Instant::now());
        let paint_first = local_snapshot(cx).is_none();
        self.load_task = Some(cx.spawn(async move |this, cx| {
            let fetch = |force_usage: bool| {
                let engine = engine.clone();
                async move {
                    engine
                        .client()
                        .call(
                            methods::LIST_AGENT_ACCOUNTS,
                            serde_json::json!({ "forceUsage": force_usage }),
                        )
                        .await
                        .ok()
                        .and_then(|value| {
                            serde_json::from_value::<AgentAccountsSnapshot>(value).ok()
                        })
                }
            };
            if paint_first && let Some(snapshot) = fetch(false).await {
                this.update(cx, |_, cx| {
                    cx.default_global::<AccountsSnapshotCache>()
                        .0
                        .insert(None, snapshot);
                })
                .ok();
            }
            let snapshot = fetch(true).await;
            this.update(cx, |accounts, cx| {
                accounts.load_task = None;
                if let Some(snapshot) = snapshot {
                    cx.default_global::<AccountsSnapshotCache>()
                        .0
                        .insert(None, snapshot);
                }
                cx.notify();
            })
            .ok();
        }));
        cx.notify();
    }
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
}
