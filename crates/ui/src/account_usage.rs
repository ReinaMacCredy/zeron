//! Plan usage of this device's agent logins, shared with Settings → Accounts
//! through [`AccountsSnapshotCache`].
use std::time::{Duration, Instant};

use gpui::{App, Context, Entity, SharedString, Subscription, Task};
pub use zeron_proto::account_usage::{active_account, extra_usage_view, used_fraction};
use zeron_proto::{AgentAccount, AgentAccountsSnapshot};
use zeron_rpc::methods;

use crate::settings::accounts::{self, AccountsSnapshotCache};
use crate::state::AppState;

/// Forced probes hit the provider; the engine throttles them too, but the
/// rings re-probe on hover, so don't even ask more often than this.
const FORCE_MIN_INTERVAL: Duration = Duration::from_secs(30);
/// Background re-probe: usage moves as turns run.
const POLL_INTERVAL: Duration = Duration::from_secs(5 * 60);
/// A finished turn re-probes once its usage has posted, at most this often:
/// parallel agents finish turns constantly and Anthropic's usage endpoint 429s.
const TURN_REFRESH_MIN: Duration = Duration::from_secs(60);
const TURN_SETTLE: Duration = Duration::from_secs(5);
const RESET_SETTLE: Duration = Duration::from_secs(3);

pub(crate) fn local_snapshot(cx: &App) -> Option<&AgentAccountsSnapshot> {
    cx.try_global::<AccountsSnapshotCache>()?.0.get(&None)
}

pub struct DeviceAccounts {
    state: Entity<AppState>,
    last_forced: Option<Instant>,
    load_task: Option<Task<()>>,
    action_task: Option<Task<()>>,
    error: Option<SharedString>,
    turns: std::collections::HashMap<String, Option<String>>,
    scheduled: Option<(Instant, Task<()>)>,
    _poll: Task<()>,
    _turns: Subscription,
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
        let turns = cx.observe(&state, |accounts: &mut Self, state, cx| {
            if accounts.note_turns(&state, cx) {
                let at = Instant::now() + TURN_SETTLE;
                let at = accounts
                    .last_forced
                    .map_or(at, |forced| at.max(forced + TURN_REFRESH_MIN));
                accounts.schedule(at, cx);
            }
        });
        Self {
            state,
            last_forced: None,
            load_task: None,
            action_task: None,
            error: None,
            turns: Default::default(),
            scheduled: None,
            _poll: poll,
            _turns: turns,
        }
    }

    /// A session first seen is only recorded: launch and reattach are not turn ends.
    fn note_turns(&mut self, state: &Entity<AppState>, cx: &App) -> bool {
        let state = state.read(cx);
        let local = state.local_device_id.as_deref();
        let mut ended = false;
        let mut seen = 0;
        for session in state
            .sessions
            .iter()
            .filter(|session| local.is_none_or(|id| id == session.device_id))
        {
            seen += 1;
            match self.turns.get_mut(&session.chat_id) {
                Some(turn) if *turn != session.last_completed_turn => {
                    ended |= session.last_completed_turn.is_some();
                    turn.clone_from(&session.last_completed_turn);
                }
                Some(_) => {}
                None => {
                    self.turns
                        .insert(session.chat_id.clone(), session.last_completed_turn.clone());
                }
            }
        }
        if self.turns.len() > seen {
            self.turns.retain(|chat, _| {
                state
                    .sessions
                    .iter()
                    .any(|session| session.chat_id == *chat)
            });
        }
        ended
    }

    fn schedule(&mut self, at: Instant, cx: &mut Context<Self>) {
        if self
            .scheduled
            .as_ref()
            .is_some_and(|(pending, _)| *pending <= at)
        {
            return;
        }
        let task = cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(at.saturating_duration_since(Instant::now()))
                .await;
            this.update(cx, |accounts, cx| {
                accounts.scheduled = None;
                // A click probed moments ago, maybe before this turn's usage posted.
                match accounts
                    .last_forced
                    .map(|forced| forced + FORCE_MIN_INTERVAL)
                    .filter(|ready| *ready > Instant::now())
                {
                    Some(ready) => accounts.schedule(ready, cx),
                    None => accounts.load(cx),
                }
            })
            .ok();
        });
        self.scheduled = Some((at, task));
    }

    fn schedule_next_reset(&mut self, cx: &mut Context<Self>) {
        let now = chrono::Utc::now();
        let Some(next) = local_snapshot(cx)
            .into_iter()
            .flat_map(|snapshot| &snapshot.accounts)
            .filter(|account| account.active)
            .flat_map(|account| &account.usage_windows)
            .filter_map(|window| (window.resets_at? - now).to_std().ok())
            .min()
        else {
            return;
        };
        let at = Instant::now() + next + RESET_SETTLE;
        let at = self
            .last_forced
            .map_or(at, |forced| at.max(forced + FORCE_MIN_INTERVAL));
        self.schedule(at, cx);
    }

    pub fn loading(&self) -> bool {
        self.load_task.is_some()
    }

    pub fn error(&self) -> Option<SharedString> {
        self.error.clone()
    }

    /// Switch optimistically, like Settings → Accounts: the rows flip at once
    /// and the engine's reply replaces them; a refusal restores the list.
    pub fn switch(&mut self, account: &AgentAccount, cx: &mut Context<Self>) {
        let Some(engine) = self.state.read(cx).engine().cloned() else {
            return;
        };
        let cache = &mut cx.default_global::<AccountsSnapshotCache>().0;
        let previous = cache.get(&None).cloned();
        if let Some(snapshot) = cache.get_mut(&None) {
            // Per-provider agents keep one live login per provider: only the
            // switched-to row's group changes.
            accounts::mark_switched(snapshot, account);
        }
        self.error = None;
        // A probe started before the switch lands is stale and would undo it.
        self.load_task = None;
        let params = serde_json::json!({
            "id": account.id,
            "accountId": account.id,
            "harness": account.harness,
        });
        self.action_task = Some(cx.spawn(async move |this, cx| {
            let result = engine
                .client()
                .call(methods::ACTIVATE_AGENT_ACCOUNT, params)
                .await;
            this.update(cx, |accounts, cx| {
                accounts.action_task = None;
                match result.map(serde_json::from_value::<AgentAccountsSnapshot>) {
                    Ok(Ok(snapshot)) => {
                        accounts.load_task = None;
                        cx.default_global::<AccountsSnapshotCache>()
                            .0
                            .insert(None, snapshot);
                    }
                    Ok(Err(_)) => {
                        accounts.last_forced = None;
                        accounts.load(cx);
                    }
                    Err(err) => {
                        if let Some(previous) = previous {
                            cx.default_global::<AccountsSnapshotCache>()
                                .0
                                .insert(None, previous);
                        }
                        accounts.error = Some(err.to_string().into());
                    }
                }
                cx.notify();
            })
            .ok();
        }));
        cx.notify();
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
                accounts.schedule_next_reset(cx);
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
                    label: "5h".into(),
                    used_fraction: *fraction,
                    resets_at: None,
                })
                .collect::<Vec<_>>(),
        }))
        .unwrap()
    }

    #[gpui::test]
    fn a_finished_turn_schedules_a_probe_but_a_first_sighting_does_not(
        cx: &mut gpui::TestAppContext,
    ) {
        use crate::pull_request_test_support as fixture;
        use gpui::AppContext;
        fixture::init(cx);
        let state = cx.update(|cx| fixture::state(cx, None));
        let accounts = cx.new(|cx| DeviceAccounts::new(state.clone(), cx));
        let session = |chat: &str, turn: Option<&str>| zeron_proto::Session {
            last_completed_turn: turn.map(str::to_owned),
            chat_id: chat.into(),
            device_id: "dev".into(),
            status: zeron_proto::SessionStatus::Idle,
            held_by_subagents: false,
            compacting: false,
            background_tasks: Vec::new(),
            started_at: None,
            updated_at: chrono::Utc::now(),
            running_subagents: 0,
            compact_plan: None,
            classifying: None,
            last_turn_outcome: Default::default(),
            terminal_activity: Default::default(),
        };
        let set = |sessions: Vec<zeron_proto::Session>, cx: &mut gpui::TestAppContext| {
            state.update(cx, |state, cx| {
                state.apply_sessions(sessions);
                cx.notify();
            });
            cx.run_until_parked();
            accounts.read_with(cx, |accounts, _| accounts.scheduled.is_some())
        };
        assert!(!set(vec![session("a", Some("t0")), session("b", None)], cx));
        assert!(!set(vec![session("a", Some("t0")), session("b", None)], cx));
        assert!(set(
            vec![session("a", Some("t0")), session("b", Some("t1"))],
            cx
        ));
    }

    #[gpui::test]
    fn a_switch_outlives_a_usage_probe_that_began_before_it(cx: &mut gpui::TestAppContext) {
        use crate::pull_request_test_support::{self as fixture, ScriptedRpc};
        use gpui::AppContext;
        use std::sync::Arc;
        use std::sync::atomic::{AtomicBool, Ordering};
        let runtime = fixture::runtime();
        let _guard = runtime.enter();
        fixture::init(cx);
        let before = AgentAccountsSnapshot {
            accounts: vec![
                account(HarnessId::Codex, true, &[0.1]),
                account(HarnessId::Codex, false, &[0.2]),
            ],
            warnings: Vec::new(),
        };
        let target = before.accounts[1].clone();
        let mut after = before.clone();
        accounts::mark_switched(&mut after, &target);
        let probing = Arc::new(AtomicBool::new(false));
        let release = Arc::new(tokio::sync::Notify::new());
        let rpc = ScriptedRpc::new({
            let (before, probing, release) = (before.clone(), probing.clone(), release.clone());
            move |method, _| {
                let (before, after) = (before.clone(), after.clone());
                let (probing, release) = (probing.clone(), release.clone());
                async move {
                    match method.as_str() {
                        methods::LIST_AGENT_ACCOUNTS => {
                            probing.store(true, Ordering::SeqCst);
                            release.notified().await;
                            zeron_rpc::RpcReply::value(&before)
                        }
                        methods::ACTIVATE_AGENT_ACCOUNT => zeron_rpc::RpcReply::value(&after),
                        other => Err(zeron_rpc::RpcError::UnknownMethod(other.into())),
                    }
                }
            }
        });
        let state = cx.update(|cx| fixture::state(cx, Some(rpc.client())));
        cx.update(|cx| {
            cx.default_global::<AccountsSnapshotCache>()
                .0
                .insert(None, before.clone());
        });
        let accounts = cx.new(|cx| DeviceAccounts::new(state, cx));
        accounts.update(cx, |accounts, cx| accounts.load(cx));
        rpc.settle(cx, &runtime, |_| probing.load(Ordering::SeqCst));
        accounts.update(cx, |accounts, cx| accounts.switch(&target, cx));
        rpc.settle(cx, &runtime, |cx| {
            accounts.read_with(cx, |accounts, _| accounts.action_task.is_none())
        });
        release.notify_one();
        rpc.settle(cx, &runtime, |_| rpc.completed() >= 2);
        cx.run_until_parked();
        let active = cx.update(|cx| {
            local_snapshot(cx)
                .and_then(|snapshot| active_account(snapshot, HarnessId::Codex))
                .map(|account| account.id.clone())
        });
        assert_eq!(active.as_deref(), Some(target.id.as_str()));
    }
}
