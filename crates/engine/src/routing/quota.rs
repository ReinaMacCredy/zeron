use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock, PoisonError};

use serde::Deserialize;
use zeron_proto::{AgentUsageWindow, ChildRoute, HarnessId, RouteTarget};

use super::{NativePolicy, OutOfQuota, QuotaState, Resolved, RoutingSettings};
use crate::agent_accounts::AgentAccounts;

const STALE_MS: i64 = 15 * 60 * 1000;
const UNKNOWN_RESET_MS: i64 = 60 * 60 * 1000;
pub const START_SLACK_MS: i64 = 15_000;

#[derive(Debug, Clone, PartialEq)]
pub struct CachedQuota {
    pub used_fraction: f64,
    pub resets_at: Option<i64>,
    pub fetched_at: i64,
}

impl CachedQuota {
    pub fn from_windows(windows: &[AgentUsageWindow], fetched_at: i64) -> Self {
        let used_fraction = windows
            .iter()
            .map(|window| f64::from(window.used_fraction))
            .fold(0.0, f64::max);
        let exhausted = windows.iter().filter(|window| window.used_fraction >= 1.0);
        let resets_at = exhausted
            .filter_map(|window| window.resets_at.map(|at| at.timestamp_millis()))
            .max()
            .or_else(|| {
                windows
                    .iter()
                    .filter_map(|window| window.resets_at.map(|at| at.timestamp_millis()))
                    .min()
            });
        CachedQuota {
            used_fraction,
            resets_at,
            fetched_at,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Memo {
    pub until: i64,
    pub noted_at: i64,
}

pub fn merge(cache: Option<CachedQuota>, memo: Option<Memo>, now: i64) -> QuotaState {
    let cache = cache.filter(|cache| now - cache.fetched_at <= STALE_MS);
    if let Some(memo) = memo.filter(|memo| memo.until > now)
        && cache
            .as_ref()
            .is_none_or(|cache| cache.fetched_at <= memo.noted_at)
    {
        return QuotaState {
            known: true,
            used_fraction: Some(1.0),
            resets_at: Some(memo.until),
            exhausted: true,
        };
    }
    match cache {
        Some(cache) => {
            let exhausted = cache.used_fraction >= 1.0 && cache.resets_at.is_none_or(|at| at > now);
            QuotaState {
                known: true,
                used_fraction: Some(cache.used_fraction),
                resets_at: cache.resets_at,
                exhausted,
            }
        }
        None => QuotaState::default(),
    }
}

/// `ZERON_FAKE_QUOTA=<harness>:<fraction>:<secs>` stands in for the accounts cache in Dev proofs.
fn fake(harness: HarnessId, now: i64) -> Option<CachedQuota> {
    if !cfg!(debug_assertions) {
        return None;
    }
    let spec = std::env::var("ZERON_FAKE_QUOTA").ok()?;
    spec.split(',').find_map(|entry| {
        let mut parts = entry.trim().split(':');
        let named: HarnessId = serde_json::from_value(serde_json::json!(parts.next()?)).ok()?;
        let fraction: f64 = parts.next()?.parse().ok()?;
        let secs: i64 = parts.next().unwrap_or("3600").parse().ok()?;
        (named == harness).then_some(CachedQuota {
            used_fraction: fraction,
            resets_at: Some(now + secs * 1000),
            fetched_at: now,
        })
    })
}

#[derive(Default)]
pub struct QuotaBook {
    accounts: OnceLock<AgentAccounts>,
    limited: Mutex<HashMap<HarnessId, Memo>>,
}

impl QuotaBook {
    pub fn new() -> Arc<Self> {
        Arc::default()
    }

    pub fn set_accounts(&self, accounts: AgentAccounts) {
        let _ = self.accounts.set(accounts);
    }

    pub fn note_limit(&self, harness: HarnessId, resets_at: Option<i64>, now: i64) {
        let until = resets_at
            .filter(|at| *at > now)
            .unwrap_or(now + UNKNOWN_RESET_MS);
        self.limited
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(
                harness,
                Memo {
                    until,
                    noted_at: now,
                },
            );
    }

    pub fn clear(&self, harness: HarnessId) {
        self.limited
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(&harness);
    }

    pub fn state(&self, harness: HarnessId, now: i64) -> QuotaState {
        let limited = self
            .limited
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(&harness)
            .copied();
        let cache = fake(harness, now).or_else(|| {
            self.accounts
                .get()
                .and_then(|accounts| accounts.cached_quota(harness))
        });
        merge(cache, limited, now)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum QuotaChoice {
    Native,
    Fallback,
    Wait,
}

pub struct Gate<'a> {
    pub settings: &'a RoutingSettings,
    pub owner: Option<RouteTarget>,
    pub step: bool,
    pub offered: &'a [HarnessId],
    pub choice: Option<QuotaChoice>,
    pub quota: &'a dyn Fn(HarnessId) -> QuotaState,
    pub now: i64,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Gated {
    Run(Resolved),
    UseNative { route: ChildRoute, message: String },
    Exhausted(String),
}

pub fn clock(at: i64) -> String {
    chrono::DateTime::from_timestamp_millis(at)
        .map(|at| at.with_timezone(&chrono::Local).format("%H:%M").to_string())
        .unwrap_or_else(|| "its reset".to_string())
}

pub fn gate(resolved: Resolved, gate: &Gate) -> Gated {
    let Some(route) = resolved.route.clone() else {
        return Gated::Run(resolved);
    };
    let (Some(harness), Some(_)) = (resolved.harness, route.table.as_ref()) else {
        return Gated::Run(resolved);
    };
    if route.source != ChildRoute::ROUTED {
        return Gated::Run(resolved);
    }
    let quota = (gate.quota)(harness);
    if !quota.exhausted {
        return Gated::Run(resolved);
    }
    let role = route.role.clone().unwrap_or_default();
    let resets_at = quota.resets_at;
    let until = resets_at.map_or_else(|| "its reset".to_string(), clock);
    let out = format!("{} out of quota", harness.display_name());
    let fell_back = |reason: String| ChildRoute {
        source: ChildRoute::FELL_BACK.to_string(),
        reason: Some(reason),
        resets_at,
        ..route.clone()
    };
    let parent = gate.owner.as_ref();
    let on_parent = || {
        Gated::Run(Resolved {
            harness: parent.map(|config| config.harness),
            model: parent.and_then(|config| config.model.clone()),
            reasoning: parent.and_then(|config| config.reasoning),
            route: Some(fell_back(out.clone())),
        })
    };
    let wait = || {
        Gated::Run(Resolved {
            route: Some(ChildRoute {
                wait_until: Some(resets_at.unwrap_or(gate.now + UNKNOWN_RESET_MS) + START_SLACK_MS),
                resets_at,
                reason: Some(out.clone()),
                ..route.clone()
            }),
            ..resolved.clone()
        })
    };
    let next_in_list = || {
        gate.settings
            .fallback
            .iter()
            .find(|target| {
                gate.offered.contains(&target.harness) && !(gate.quota)(target.harness).exhausted
            })
            .map_or_else(wait, |target: &RouteTarget| {
                Gated::Run(Resolved {
                    harness: Some(target.harness),
                    model: target.model.clone(),
                    reasoning: target.reasoning,
                    route: Some(fell_back(out.clone())),
                })
            })
    };
    let policy = match gate.choice {
        Some(QuotaChoice::Native) => OutOfQuota::Native,
        Some(QuotaChoice::Fallback) => OutOfQuota::NextInList,
        Some(QuotaChoice::Wait) => OutOfQuota::Wait,
        None => gate.settings.out_of_quota,
    };
    match policy {
        OutOfQuota::Native => {
            let parent_harness = parent.map(|config| config.harness);
            if parent_harness.is_some_and(|harness| (gate.quota)(harness).exhausted) {
                return next_in_list();
            }
            if !gate.step
                && parent_harness == Some(HarnessId::ClaudeCode)
                && gate.settings.native != NativePolicy::Off
            {
                return Gated::UseNative {
                    route: fell_back(out.clone()),
                    message: format!(
                        "No child was created: {} is out of quota until {until}, and Settings > Routing falls back to your native sub-agent. Run this task with your own sub-agent tool (Agent) on this chat's model now; after {until}, {role} routes to {} again.",
                        harness.display_name(),
                        harness.display_name()
                    ),
                };
            }
            on_parent()
        }
        OutOfQuota::NextInList => next_in_list(),
        OutOfQuota::Wait => wait(),
        OutOfQuota::Ask => Gated::Exhausted(format!(
            "quota_exhausted: {} is out of quota until {until}, so the {role} route cannot start. Ask the person with ask_user whether to run it as your native sub-agent, on the next harness in the fallback list, or wait for the reset, then call again with quotaChoice native, fallback or wait.",
            harness.display_name()
        )),
    }
}

pub fn offer(
    settings: &RoutingSettings,
    child: HarnessId,
    owner: Option<RouteTarget>,
    offered: &[HarnessId],
    quota: &dyn Fn(HarnessId) -> QuotaState,
    now: i64,
) -> zeron_proto::LimitOffer {
    let usable = |target: &RouteTarget| {
        target.harness != child
            && offered.contains(&target.harness)
            && !quota(target.harness).exhausted
    };
    let listed = || {
        settings
            .fallback
            .iter()
            .find(|target| usable(target))
            .cloned()
    };
    let to = match settings.out_of_quota {
        OutOfQuota::NextInList => listed(),
        _ => owner.filter(|owner| usable(owner)).or_else(listed),
    };
    let preselect = match (&to, settings.out_of_quota) {
        (None, _) | (_, OutOfQuota::Wait) => zeron_proto::OfferChoice::Wait,
        _ => zeron_proto::OfferChoice::Switch,
    };
    let act_at = match settings.out_of_quota {
        OutOfQuota::Ask => None,
        _ => settings.mid_run.delay_ms().map(|delay| now + delay),
    };
    zeron_proto::LimitOffer {
        to,
        preselect,
        act_at,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::routing::tests::owner;
    use crate::routing::{Request, Role, RoleRoute, resolve};
    use zeron_proto::ChatConfig;

    const NOW: i64 = 1_000_000_000;

    fn codex() -> RouteTarget {
        RouteTarget {
            harness: HarnessId::Codex,
            model: Some("gpt-6-luna".into()),
            reasoning: None,
        }
    }

    fn claude(model: &str) -> RouteTarget {
        RouteTarget {
            harness: HarnessId::ClaudeCode,
            model: Some(model.into()),
            reasoning: None,
        }
    }

    fn settings(policy: OutOfQuota) -> RoutingSettings {
        let mut settings = RoutingSettings {
            out_of_quota: policy,
            ..RoutingSettings::default()
        };
        settings
            .roles
            .insert(Role::Worker, RoleRoute::Target(codex()));
        settings
    }

    fn parent(harness: HarnessId) -> ChatConfig {
        ChatConfig {
            harness,
            model: Some(match harness {
                HarnessId::ClaudeCode => "claude-opus-5-5".into(),
                _ => "gpt-5.6-sol".into(),
            }),
            ..owner()
        }
    }

    const OFFERED: [HarnessId; 3] = [HarnessId::ClaudeCode, HarnessId::Codex, HarnessId::Grok];

    fn run(
        settings: &RoutingSettings,
        owner: &ChatConfig,
        exhausted: &[HarnessId],
        step: bool,
        choice: Option<QuotaChoice>,
    ) -> Gated {
        let request = Request {
            role: Some(Role::Worker),
            ..Request::default()
        };
        let resolved = resolve(
            &request,
            Some(&crate::routing::target_of(owner)),
            settings,
            &OFFERED,
        );
        let quota = |harness| QuotaState {
            known: true,
            used_fraction: Some(if exhausted.contains(&harness) {
                1.0
            } else {
                0.3
            }),
            resets_at: Some(NOW + 3_600_000),
            exhausted: exhausted.contains(&harness),
        };
        gate(
            resolved,
            &Gate {
                settings,
                owner: Some(crate::routing::target_of(owner)),
                step,
                offered: &OFFERED,
                choice,
                quota: &quota,
                now: NOW,
            },
        )
    }

    fn ran(gated: Gated) -> Resolved {
        match gated {
            Gated::Run(resolved) => resolved,
            other => panic!("expected a child, got {other:?}"),
        }
    }

    #[test]
    fn a_route_with_quota_runs_as_routed() {
        let resolved = ran(run(
            &settings(OutOfQuota::Native),
            &parent(HarnessId::ClaudeCode),
            &[],
            false,
            None,
        ));
        assert_eq!(resolved.harness, Some(HarnessId::Codex));
        assert_eq!(resolved.route.unwrap().source, "routed");
    }

    #[test]
    fn native_policy_hands_a_claude_parent_its_own_sub_agent() {
        let claude_parent = parent(HarnessId::ClaudeCode);
        match run(
            &settings(OutOfQuota::Native),
            &claude_parent,
            &[HarnessId::Codex],
            false,
            None,
        ) {
            Gated::UseNative { route, message } => {
                assert_eq!(route.source, "fell_back");
                assert_eq!(route.resets_at, Some(NOW + 3_600_000));
                assert!(message.contains("native sub-agent"), "{message}");
            }
            other => panic!("{other:?}"),
        }
        let step = ran(run(
            &settings(OutOfQuota::Native),
            &claude_parent,
            &[HarnessId::Codex],
            true,
            None,
        ));
        assert_eq!(step.model.as_deref(), Some("claude-opus-5-5"));
        assert_eq!(step.route.unwrap().source, "fell_back");

        let mut off = settings(OutOfQuota::Native);
        off.native = NativePolicy::Off;
        let child = ran(run(&off, &claude_parent, &[HarnessId::Codex], false, None));
        assert_eq!(child.harness, Some(HarnessId::ClaudeCode));
        assert_eq!(child.route.unwrap().source, "fell_back");

        let other = ran(run(
            &settings(OutOfQuota::Native),
            &parent(HarnessId::Grok),
            &[HarnessId::Codex],
            false,
            None,
        ));
        assert_eq!(other.harness, Some(HarnessId::Grok));
    }

    #[test]
    fn a_parent_on_the_exhausted_harness_takes_the_fallback_list() {
        let mut settings = settings(OutOfQuota::Native);
        settings.fallback = vec![codex(), claude("claude-sonnet-5-5")];
        let child = ran(run(
            &settings,
            &parent(HarnessId::Codex),
            &[HarnessId::Codex],
            false,
            None,
        ));
        assert_eq!(child.model.as_deref(), Some("claude-sonnet-5-5"));
        let route = child.route.unwrap();
        assert_eq!(route.source, "fell_back");
        assert_eq!(route.reason.as_deref(), Some("Codex out of quota"));
    }

    #[test]
    fn next_in_list_skips_exhausted_entries_and_waits_when_none_has_quota() {
        let mut settings = settings(OutOfQuota::NextInList);
        settings.fallback = vec![
            claude("claude-opus-5-5"),
            RouteTarget {
                harness: HarnessId::Grok,
                model: Some("grok-4.7".into()),
                reasoning: None,
            },
        ];
        let owner = parent(HarnessId::ClaudeCode);
        let child = ran(run(
            &settings,
            &owner,
            &[HarnessId::Codex, HarnessId::ClaudeCode],
            false,
            None,
        ));
        assert_eq!(child.harness, Some(HarnessId::Grok));
        let waiting = ran(run(
            &settings,
            &owner,
            &[HarnessId::Codex, HarnessId::ClaudeCode, HarnessId::Grok],
            false,
            None,
        ));
        assert_eq!(waiting.harness, Some(HarnessId::Codex));
        assert_eq!(
            waiting.route.unwrap().wait_until,
            Some(NOW + 3_600_000 + START_SLACK_MS)
        );
    }

    #[test]
    fn ask_refuses_until_a_quota_choice_is_given() {
        let settings = settings(OutOfQuota::Ask);
        let owner = parent(HarnessId::ClaudeCode);
        match run(&settings, &owner, &[HarnessId::Codex], false, None) {
            Gated::Exhausted(message) => assert!(message.starts_with("quota_exhausted:")),
            other => panic!("{other:?}"),
        }
        let waited = ran(run(
            &settings,
            &owner,
            &[HarnessId::Codex],
            false,
            Some(QuotaChoice::Wait),
        ));
        assert!(waited.route.unwrap().wait_until.is_some());
        assert!(matches!(
            run(
                &settings,
                &owner,
                &[HarnessId::Codex],
                false,
                Some(QuotaChoice::Native)
            ),
            Gated::UseNative { .. }
        ));
    }

    #[test]
    fn stale_cache_routes_normally_and_a_limit_memo_wins() {
        let full = CachedQuota {
            used_fraction: 1.0,
            resets_at: Some(NOW + 60_000),
            fetched_at: NOW - STALE_MS - 1,
        };
        assert!(!merge(Some(full.clone()), None, NOW).known);
        let fresh = CachedQuota {
            fetched_at: NOW - 1_000,
            ..full
        };
        assert!(merge(Some(fresh.clone()), None, NOW).exhausted);
        let noted = |until, noted_at| Some(Memo { until, noted_at });
        let memo = merge(None, noted(NOW + 5_000, NOW - 2_000), NOW);
        assert!(memo.exhausted && memo.resets_at == Some(NOW + 5_000));
        assert!(!merge(None, noted(NOW - 1, NOW - 2_000), NOW).known);
        let probed_after = CachedQuota {
            used_fraction: 0.1,
            ..fresh
        };
        assert!(
            !merge(
                Some(probed_after.clone()),
                noted(NOW + 5_000, NOW - 2_000),
                NOW
            )
            .exhausted,
            "a usage probe fetched after the limit wins"
        );
        assert!(merge(Some(probed_after), noted(NOW + 5_000, NOW), NOW).exhausted);
    }

    #[test]
    fn the_quota_book_reads_memory_only() {
        let book = QuotaBook::new();
        let dir = tempfile::tempdir().unwrap();
        book.set_accounts(AgentAccounts::new(
            crate::agent_accounts::AgentAccountsConfig::detect(dir.path()),
        ));
        assert!(!book.state(HarnessId::Codex, NOW).known);
        book.note_limit(HarnessId::Grok, None, NOW);
        let grok = book.state(HarnessId::Grok, NOW);
        assert!(grok.exhausted);
        assert_eq!(grok.resets_at, Some(NOW + UNKNOWN_RESET_MS));
        book.clear(HarnessId::Grok);
        assert!(!book.state(HarnessId::Grok, NOW).known);
    }

    #[test]
    fn a_mid_run_offer_switches_to_the_owner_or_the_list_and_counts_down() {
        use zeron_proto::OfferChoice;
        let quota = |harness| QuotaState {
            exhausted: harness == HarnessId::Codex,
            ..QuotaState::default()
        };
        let mut settings = settings(OutOfQuota::Native);
        let owner = Some(claude("claude-opus-5-5"));
        let native = offer(
            &settings,
            HarnessId::Codex,
            owner.clone(),
            &OFFERED,
            &quota,
            NOW,
        );
        assert_eq!(native.to, owner);
        assert_eq!(native.preselect, OfferChoice::Switch);
        assert_eq!(native.act_at, Some(NOW + 30_000));

        settings.out_of_quota = OutOfQuota::NextInList;
        settings.fallback = vec![codex(), claude("claude-sonnet-5-5")];
        settings.mid_run = crate::routing::MidRun::Ask;
        let listed = offer(
            &settings,
            HarnessId::Codex,
            owner.clone(),
            &OFFERED,
            &quota,
            NOW,
        );
        assert_eq!(listed.to, Some(claude("claude-sonnet-5-5")));
        assert_eq!(listed.act_at, None);

        settings.fallback = vec![codex()];
        let none = offer(&settings, HarnessId::Codex, owner, &OFFERED, &quota, NOW);
        assert_eq!((none.to, none.preselect), (None, OfferChoice::Wait));
    }
}
