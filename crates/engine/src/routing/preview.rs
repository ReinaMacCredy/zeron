use serde::Deserialize;
use zeron_proto::{ChildRoute, HarnessId, RouteTarget};

use super::quota::{self, Gated};
use super::{QuotaState, Request, Resolved, Role, RoutingSettings, guidance, resolve};

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PreviewParams {
    #[serde(default)]
    pub base: Option<RouteTarget>,
    #[serde(default)]
    pub roles: Option<Vec<String>>,
    #[serde(default)]
    pub owner_chat_id: Option<String>,
    #[serde(default)]
    pub steps: bool,
}

pub use zeron_proto::routing::{Fallback, Preview, Row};

pub struct Inputs<'a> {
    pub settings: &'a RoutingSettings,
    pub base: RouteTarget,
    pub roles: Vec<Role>,
    pub offered: &'a [HarnessId],
    pub price: &'a dyn Fn(HarnessId, &str) -> Option<f64>,
    pub quota: &'a dyn Fn(HarnessId) -> QuotaState,
    pub step: bool,
    pub now: i64,
}

pub fn roles(names: Option<&[String]>) -> Vec<Role> {
    match names {
        None => Role::ALL.to_vec(),
        Some(names) => {
            let mut roles = Vec::new();
            for role in names.iter().filter_map(|name| Role::parse(name)) {
                if !roles.contains(&role) {
                    roles.push(role);
                }
            }
            roles
        }
    }
}

pub fn multiplier(
    price: &dyn Fn(HarnessId, &str) -> Option<f64>,
    target: &RouteTarget,
    base: &RouteTarget,
) -> Option<f64> {
    if target.harness == base.harness && target.model == base.model {
        return Some(1.0);
    }
    let of = |t: &RouteTarget| t.model.as_deref().and_then(|model| price(t.harness, model));
    let (target, base) = (of(target)?, of(base)?);
    (base > 0.0).then(|| target / base)
}

pub fn preview(inputs: &Inputs) -> Preview {
    let base = &inputs.base;
    let rows = inputs.roles.iter().map(|role| row(inputs, *role)).collect();
    Preview {
        base: Some(base.clone()),
        rows,
        guidance: guidance::guidance(
            inputs.settings,
            Some(base.model.as_deref().unwrap_or(base.harness.display_name())),
        ),
    }
}

fn row(inputs: &Inputs, role: Role) -> Row {
    let base = &inputs.base;
    let resolved = resolve(
        &Request {
            role: Some(role),
            ..Request::default()
        },
        Some(base),
        inputs.settings,
        inputs.offered,
    );
    let Some(route) = resolved.route.clone() else {
        return Row {
            role: role.name().to_string(),
            source: "off".to_string(),
            target: None,
            multiplier: Some(1.0),
            quota: (inputs.quota)(base.harness),
            reason: None,
            fallback: None,
        };
    };
    let actual = |resolved: &Resolved| RouteTarget {
        harness: resolved.harness.unwrap_or(base.harness),
        model: resolved.model.clone(),
        reasoning: resolved.reasoning,
    };
    let planned = actual(&resolved);
    let row = |target: RouteTarget, source: &'static str, reason, fallback| Row {
        role: role.name().to_string(),
        source: source.to_string(),
        multiplier: multiplier(inputs.price, &target, base),
        quota: (inputs.quota)(target.harness),
        target: Some(target),
        reason,
        fallback,
    };
    if route.source != ChildRoute::ROUTED {
        return row(planned, ChildRoute::FELL_BACK, route.reason, None);
    }
    let planned_quota = (inputs.quota)(planned.harness);
    let fallback = |target: Option<RouteTarget>, native: bool, reason: Option<String>| {
        Some(Fallback {
            multiplier: target
                .as_ref()
                .and_then(|target| multiplier(inputs.price, target, base)),
            target,
            native,
            reason: reason.unwrap_or_default(),
            resets_at: planned_quota.resets_at,
        })
    };
    let gated = quota::gate(
        resolved,
        &quota::Gate {
            settings: inputs.settings,
            owner: Some(base.clone()),
            step: inputs.step,
            offered: inputs.offered,
            choice: None,
            quota: inputs.quota,
            now: inputs.now,
        },
    );
    match gated {
        Gated::Run(ran) => {
            let route = ran.route.clone().unwrap_or_default();
            let fallback = match (route.source.as_str(), route.wait_until) {
                (ChildRoute::FELL_BACK, _) => fallback(Some(actual(&ran)), false, route.reason),
                (_, Some(_)) => fallback(None, false, Some("waits for the reset".into())),
                _ => None,
            };
            row(planned, ChildRoute::ROUTED, None, fallback)
        }
        Gated::UseNative { route, .. } => row(
            planned,
            ChildRoute::ROUTED,
            None,
            fallback(Some(base.clone()), true, route.reason),
        ),
        Gated::Exhausted(_) => row(
            planned,
            ChildRoute::ROUTED,
            None,
            fallback(None, false, Some("asks you first".into())),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::routing::RoleRoute;

    fn api() -> serde_json::Value {
        serde_json::json!({
            "anthropic": {"models": {
                "claude-opus-5-5": {"cost": {"input": 5, "output": 25}},
                "claude-haiku-5-5": {"cost": {"input": 1, "output": 5}}
            }}
        })
    }

    fn claude(model: &str) -> RouteTarget {
        RouteTarget {
            harness: HarnessId::ClaudeCode,
            model: Some(model.into()),
            reasoning: None,
        }
    }

    #[test]
    fn preview_prices_each_role_against_the_base() {
        let api = api();
        let price = |harness, model: &str| crate::model_prices::blended_in(&api, harness, model);
        let mut settings = RoutingSettings::default();
        settings
            .roles
            .insert(Role::Worker, RoleRoute::Target(claude("claude-haiku-5-5")));
        settings
            .roles
            .insert(Role::Validator, RoleRoute::SameAsParent);
        settings.roles.insert(
            Role::Research,
            RoleRoute::Target(claude("claude-mystery-1")),
        );
        settings.roles.insert(
            Role::Design,
            RoleRoute::Target(RouteTarget {
                harness: HarnessId::Grok,
                model: Some("grok-4.7".into()),
                reasoning: None,
            }),
        );
        let offered = [HarnessId::ClaudeCode];
        let preview = preview(&Inputs {
            settings: &settings,
            base: claude("claude-opus-5-5"),
            roles: roles(None),
            offered: &offered,
            price: &price,
            quota: &|_| QuotaState::default(),
            step: false,
            now: 0,
        });
        let row = |role| preview.rows.iter().find(|row| row.role == role).unwrap();
        assert_eq!(row("worker").multiplier, Some(0.2));
        assert_eq!(row("worker").source, "routed");
        assert_eq!(row("validator").multiplier, Some(1.0));
        assert_eq!(row("research").multiplier, None);
        assert_eq!(row("design").source, "fell_back");
        assert_eq!(row("design").target, Some(claude("claude-opus-5-5")));
        assert_eq!(row("general").source, "off");
        assert_eq!(row("general").target, None);
        assert_eq!(row("general").multiplier, Some(1.0));
        assert!(
            preview
                .guidance
                .contains("worker on Claude Code claude-haiku-5-5")
        );
        assert!(!row("worker").quota.known);
    }

    #[test]
    fn an_exhausted_route_previews_its_fallback() {
        let api = api();
        let price = |harness, model: &str| crate::model_prices::blended_in(&api, harness, model);
        let mut settings = RoutingSettings::default();
        settings.roles.insert(
            Role::Worker,
            RoleRoute::Target(RouteTarget {
                harness: HarnessId::Codex,
                model: Some("gpt-6-luna".into()),
                reasoning: None,
            }),
        );
        let offered = [HarnessId::ClaudeCode, HarnessId::Codex];
        let quota = |harness| QuotaState {
            known: true,
            used_fraction: Some(1.0),
            resets_at: Some(5_000),
            exhausted: harness == HarnessId::Codex,
        };
        let worker = |step| {
            preview(&Inputs {
                settings: &settings,
                base: claude("claude-opus-5-5"),
                roles: vec![Role::Worker],
                offered: &offered,
                price: &price,
                quota: &quota,
                step,
                now: 0,
            })
            .rows
            .remove(0)
        };
        let chat = worker(false);
        assert_eq!(chat.target.unwrap().harness, HarnessId::Codex);
        let fallback = chat.fallback.expect("fallback");
        assert!(fallback.native);
        assert_eq!(fallback.target, Some(claude("claude-opus-5-5")));
        assert_eq!(fallback.multiplier, Some(1.0));
        assert_eq!(fallback.resets_at, Some(5_000));
        let step = worker(true).fallback.expect("fallback");
        assert!(!step.native);
        assert_eq!(step.reason, "Codex out of quota");
    }

    #[test]
    fn roles_accept_aliases_and_skip_unknown_names() {
        let names = [
            "implementation".to_string(),
            "worker".into(),
            "planner".into(),
            "test".into(),
        ];
        assert_eq!(roles(Some(&names)), [Role::Worker, Role::Validator]);
    }
}
