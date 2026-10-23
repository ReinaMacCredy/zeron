pub mod guidance;
pub mod preview;
pub mod quota;

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use zeron_proto::{ChildRoute, HarnessId, ReasoningLevel, RouteTarget};

pub use zeron_proto::route_preview::{QuotaState, Role, target_of};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum RoleRoute {
    #[default]
    Off,
    SameAsParent,
    Target(RouteTarget),
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum OutOfQuota {
    #[default]
    Native,
    NextInList,
    Wait,
    Ask,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MidRun {
    Ask,
    After10s,
    #[default]
    After30s,
}

impl MidRun {
    pub fn delay_ms(self) -> Option<i64> {
        match self {
            MidRun::Ask => None,
            MidRun::After10s => Some(10_000),
            MidRun::After30s => Some(30_000),
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum NativePolicy {
    #[default]
    SameModelOnly,
    Always,
    Off,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct RoutingSettings {
    pub roles: BTreeMap<Role, RoleRoute>,
    pub out_of_quota: OutOfQuota,
    pub fallback: Vec<RouteTarget>,
    pub mid_run: MidRun,
    pub native: NativePolicy,
}

impl RoutingSettings {
    pub fn route(&self, role: Role) -> &RoleRoute {
        static OFF: RoleRoute = RoleRoute::Off;
        self.roles.get(&role).unwrap_or(&OFF)
    }

    pub fn targets(&self) -> impl Iterator<Item = &RouteTarget> {
        self.roles
            .values()
            .filter_map(|route| match route {
                RoleRoute::Target(target) => Some(target),
                _ => None,
            })
            .chain(&self.fallback)
    }
}

pub fn check_target(target: &RouteTarget, known: impl Fn(HarnessId) -> bool) -> Result<(), String> {
    if target.harness == HarnessId::Acp || !known(target.harness) {
        return Err(format!(
            "{} cannot be a route target",
            target.harness.display_name()
        ));
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Chosen {
    Profile,
    Harness,
    Model,
    Reasoning,
}

impl Chosen {
    fn name(self) -> &'static str {
        match self {
            Chosen::Profile => "profile",
            Chosen::Harness => "harness",
            Chosen::Model => "model",
            Chosen::Reasoning => "reasoning",
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct Request<'a> {
    pub role: Option<Role>,
    pub chosen: &'a [Chosen],
    pub harness: Option<HarnessId>,
    pub model: Option<&'a str>,
    pub reasoning: Option<ReasoningLevel>,
    pub profile: bool,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Resolved {
    pub harness: Option<HarnessId>,
    pub model: Option<String>,
    pub reasoning: Option<ReasoningLevel>,
    pub route: Option<ChildRoute>,
}

impl Request<'_> {
    fn picked(&self) -> Vec<&'static str> {
        let mut picked: Vec<&'static str> = self
            .chosen
            .iter()
            .filter(|chosen| **chosen != Chosen::Reasoning)
            .map(|chosen| chosen.name())
            .collect();
        if picked.is_empty() {
            let present = [
                (self.profile, Chosen::Profile),
                (self.harness.is_some(), Chosen::Harness),
                (self.model.is_some(), Chosen::Model),
            ];
            picked = present
                .into_iter()
                .filter(|(set, _)| *set)
                .map(|(_, chosen)| chosen.name())
                .collect();
        }
        picked
    }

    fn explicit(&self) -> Resolved {
        Resolved {
            harness: self.harness,
            model: self.model.map(str::to_string),
            reasoning: self.reasoning,
            route: None,
        }
    }
}

pub fn resolve(
    request: &Request,
    owner: Option<&RouteTarget>,
    settings: &RoutingSettings,
    offered: &[HarnessId],
) -> Resolved {
    let explicit = request.explicit();
    let Some(role) = request.role else {
        return explicit;
    };
    let route = settings.route(role);
    if *route == RoleRoute::Off {
        return explicit;
    }
    let table = match route {
        RoleRoute::Target(target) => Some(target.clone()),
        _ => None,
    };
    let record = |source: &str, reason: Option<String>| {
        Some(ChildRoute {
            source: source.to_string(),
            role: Some(role.name().to_string()),
            table: table.clone(),
            reason,
            ..ChildRoute::default()
        })
    };
    let picked = request.picked();
    if !picked.is_empty() {
        return Resolved {
            route: record(
                ChildRoute::CHOSEN_BY_AGENT,
                Some(format!("The lead passed {}", picked.join(" and "))),
            ),
            ..explicit
        };
    }
    let parent = |route| Resolved {
        harness: owner.map(|parent| parent.harness),
        model: owner.and_then(|parent| parent.model.clone()),
        reasoning: request
            .reasoning
            .or(owner.and_then(|parent| parent.reasoning)),
        route,
    };
    match route {
        RoleRoute::Off => explicit,
        RoleRoute::SameAsParent => parent(record(ChildRoute::ROUTED, None)),
        RoleRoute::Target(target) if offered.contains(&target.harness) => Resolved {
            harness: Some(target.harness),
            model: target.model.clone(),
            reasoning: request.reasoning.or(target.reasoning),
            route: record(ChildRoute::ROUTED, None),
        },
        RoleRoute::Target(target) => parent(record(
            ChildRoute::FELL_BACK,
            Some(format!(
                "{} is not available",
                target.harness.display_name()
            )),
        )),
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use zeron_proto::ChatConfig;

    #[test]
    fn roles_parse_their_names_and_the_old_aliases() {
        for role in Role::ALL {
            assert_eq!(Role::parse(role.name()), Some(role));
        }
        assert_eq!(Role::parse("implementation"), Some(Role::Worker));
        assert_eq!(Role::parse("review"), Some(Role::Validator));
        assert_eq!(Role::parse(" Test "), Some(Role::Validator));
        assert_eq!(Role::parse("planner"), None);
        assert_eq!(Role::parse(""), None);
    }

    #[test]
    fn settings_default_and_round_trip() {
        let empty: RoutingSettings = serde_json::from_str("{}").unwrap();
        assert_eq!(empty, RoutingSettings::default());
        assert_eq!(empty.out_of_quota, OutOfQuota::Native);
        assert_eq!(empty.mid_run, MidRun::After30s);
        assert_eq!(empty.native, NativePolicy::SameModelOnly);
        assert_eq!(empty.route(Role::Worker), &RoleRoute::Off);

        let mut settings = RoutingSettings::default();
        settings.roles.insert(
            Role::Worker,
            RoleRoute::Target(RouteTarget {
                harness: HarnessId::Codex,
                model: Some("gpt-6-luna".into()),
                reasoning: Some(ReasoningLevel::Medium),
            }),
        );
        settings
            .roles
            .insert(Role::General, RoleRoute::SameAsParent);
        settings.out_of_quota = OutOfQuota::NextInList;
        settings.fallback.push(RouteTarget {
            harness: HarnessId::ClaudeCode,
            model: Some("claude-opus-5-5".into()),
            reasoning: None,
        });
        settings.mid_run = MidRun::After10s;
        settings.native = NativePolicy::Off;
        let json = serde_json::to_value(&settings).unwrap();
        assert_eq!(
            json["roles"]["worker"],
            serde_json::json!({"kind": "target", "harness": "codex", "model": "gpt-6-luna", "reasoning": "medium"})
        );
        assert_eq!(
            json["roles"]["general"],
            serde_json::json!({"kind": "sameAsParent"})
        );
        assert_eq!(json["outOfQuota"], "nextInList");
        assert_eq!(json["midRun"], "after10s");
        assert_eq!(json["native"], "off");
        let back: RoutingSettings = serde_json::from_value(json).unwrap();
        assert_eq!(back, settings);
    }

    #[test]
    fn acp_and_unknown_harnesses_are_refused() {
        let target = |harness| RouteTarget {
            harness,
            model: None,
            reasoning: None,
        };
        assert!(check_target(&target(HarnessId::Codex), |_| true).is_ok());
        assert!(check_target(&target(HarnessId::Acp), |_| true).is_err());
        assert!(check_target(&target(HarnessId::Grok), |_| false).is_err());
    }

    fn haiku() -> RouteTarget {
        RouteTarget {
            harness: HarnessId::ClaudeCode,
            model: Some("claude-haiku-5-5".into()),
            reasoning: Some(ReasoningLevel::Low),
        }
    }

    pub(crate) fn owner() -> ChatConfig {
        ChatConfig {
            harness: HarnessId::Codex,
            model: Some("gpt-5.6-sol".into()),
            reasoning: Some(ReasoningLevel::High),
            model_options: Default::default(),
            sandbox: zeron_proto::SandboxLevel::WorkspaceWrite,
            permission_mode: zeron_proto::PermissionMode::Ask,
            instructions: String::new(),
            profile_id: None,
            instance_id: None,
            disabled_tools: Vec::new(),
        }
    }

    fn routed_worker() -> RoutingSettings {
        let mut settings = RoutingSettings::default();
        settings
            .roles
            .insert(Role::Worker, RoleRoute::Target(haiku()));
        settings
            .roles
            .insert(Role::Validator, RoleRoute::SameAsParent);
        settings
    }

    const OFFERED: [HarnessId; 2] = [HarnessId::ClaudeCode, HarnessId::Codex];

    #[test]
    fn a_routed_role_runs_on_its_target() {
        let request = Request {
            role: Role::parse("implementation"),
            ..Request::default()
        };
        let resolved = resolve(
            &request,
            Some(&target_of(&owner())),
            &routed_worker(),
            &OFFERED,
        );
        assert_eq!(resolved.harness, Some(HarnessId::ClaudeCode));
        assert_eq!(resolved.model.as_deref(), Some("claude-haiku-5-5"));
        assert_eq!(resolved.reasoning, Some(ReasoningLevel::Low));
        let route = resolved.route.unwrap();
        assert_eq!(route.source, ChildRoute::ROUTED);
        assert_eq!(route.role.as_deref(), Some("worker"));
        assert_eq!(route.table, Some(haiku()));
    }

    #[test]
    fn an_agent_choice_wins_over_the_route() {
        for (chosen, model, profile, passed) in [
            (
                vec![Chosen::Model],
                Some("claude-sonnet-5-5"),
                false,
                "model",
            ),
            (vec![Chosen::Profile], None, true, "profile"),
            (vec![], Some("claude-sonnet-5-5"), false, "model"),
        ] {
            let request = Request {
                role: Some(Role::Worker),
                chosen: &chosen,
                harness: Some(HarnessId::ClaudeCode),
                model,
                profile,
                ..Request::default()
            };
            let resolved = resolve(
                &request,
                Some(&target_of(&owner())),
                &routed_worker(),
                &OFFERED,
            );
            assert_eq!(resolved.model.as_deref(), model);
            let route = resolved.route.unwrap();
            assert_eq!(route.source, ChildRoute::CHOSEN_BY_AGENT);
            assert_eq!(route.table, Some(haiku()));
            assert!(route.reason.unwrap().contains(passed));
        }
    }

    #[test]
    fn reasoning_alone_keeps_the_route() {
        let chosen = [Chosen::Reasoning];
        let request = Request {
            role: Some(Role::Worker),
            chosen: &chosen,
            reasoning: Some(ReasoningLevel::High),
            ..Request::default()
        };
        let resolved = resolve(
            &request,
            Some(&target_of(&owner())),
            &routed_worker(),
            &OFFERED,
        );
        assert_eq!(resolved.model.as_deref(), Some("claude-haiku-5-5"));
        assert_eq!(resolved.reasoning, Some(ReasoningLevel::High));
        assert_eq!(resolved.route.unwrap().source, ChildRoute::ROUTED);
    }

    #[test]
    fn off_and_missing_roles_resolve_as_today() {
        for role in [None, Some(Role::Research)] {
            let request = Request {
                role,
                model: Some("gpt-5.6-luna"),
                ..Request::default()
            };
            let resolved = resolve(
                &request,
                Some(&target_of(&owner())),
                &routed_worker(),
                &OFFERED,
            );
            assert_eq!(
                resolved,
                Resolved {
                    model: Some("gpt-5.6-luna".into()),
                    ..Resolved::default()
                }
            );
        }
    }

    #[test]
    fn same_as_parent_and_an_unavailable_target_take_the_parent() {
        let request = Request {
            role: Some(Role::Validator),
            ..Request::default()
        };
        let resolved = resolve(
            &request,
            Some(&target_of(&owner())),
            &routed_worker(),
            &OFFERED,
        );
        assert_eq!(resolved.harness, Some(HarnessId::Codex));
        assert_eq!(resolved.model.as_deref(), Some("gpt-5.6-sol"));
        assert_eq!(resolved.route.unwrap().source, ChildRoute::ROUTED);

        let request = Request {
            role: Some(Role::Worker),
            ..Request::default()
        };
        let resolved = resolve(
            &request,
            Some(&target_of(&owner())),
            &routed_worker(),
            &[HarnessId::Codex],
        );
        assert_eq!(resolved.harness, Some(HarnessId::Codex));
        let route = resolved.route.unwrap();
        assert_eq!(route.source, ChildRoute::FELL_BACK);
        assert_eq!(
            route.reason.as_deref(),
            Some("Claude Code is not available")
        );
    }
}
