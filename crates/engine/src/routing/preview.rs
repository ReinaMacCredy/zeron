use serde::{Deserialize, Serialize};
use zeron_proto::{ChildRoute, HarnessId, RouteTarget};

use super::{QuotaState, Role, RoleRoute, RoutingSettings, guidance};

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PreviewParams {
    #[serde(default)]
    pub base: Option<RouteTarget>,
    #[serde(default)]
    pub roles: Option<Vec<String>>,
    #[serde(default)]
    pub owner_chat_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Fallback {
    pub target: Option<RouteTarget>,
    pub native: bool,
    pub reason: String,
    pub resets_at: Option<i64>,
    pub multiplier: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Row {
    pub role: &'static str,
    pub source: &'static str,
    pub target: Option<RouteTarget>,
    pub multiplier: Option<f64>,
    pub quota: QuotaState,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fallback: Option<Fallback>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Preview {
    pub base: RouteTarget,
    pub rows: Vec<Row>,
    pub guidance: String,
}

pub struct Inputs<'a> {
    pub settings: &'a RoutingSettings,
    pub base: RouteTarget,
    pub roles: Vec<Role>,
    pub offered: &'a [HarnessId],
    pub price: &'a dyn Fn(HarnessId, &str) -> Option<f64>,
    pub quota: &'a dyn Fn(HarnessId) -> QuotaState,
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
        base: base.clone(),
        rows,
        guidance: guidance::guidance(
            inputs.settings,
            Some(base.model.as_deref().unwrap_or(base.harness.display_name())),
        ),
    }
}

fn row(inputs: &Inputs, role: Role) -> Row {
    let base = &inputs.base;
    let at = |target: RouteTarget, source: &'static str, reason: Option<String>| Row {
        role: role.name(),
        source,
        multiplier: multiplier(inputs.price, &target, base),
        quota: (inputs.quota)(target.harness),
        target: Some(target),
        reason,
        fallback: None,
    };
    match inputs.settings.route(role) {
        RoleRoute::Off => Row {
            role: role.name(),
            source: "off",
            target: None,
            multiplier: Some(1.0),
            quota: (inputs.quota)(base.harness),
            reason: None,
            fallback: None,
        },
        RoleRoute::SameAsParent => at(base.clone(), ChildRoute::ROUTED, None),
        RoleRoute::Target(target) if inputs.offered.contains(&target.harness) => {
            at(target.clone(), ChildRoute::ROUTED, None)
        }
        RoleRoute::Target(target) => at(
            base.clone(),
            ChildRoute::FELL_BACK,
            Some(format!(
                "{} is not available",
                target.harness.display_name()
            )),
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
