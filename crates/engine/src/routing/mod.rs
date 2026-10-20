use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use zeron_proto::{HarnessId, RouteTarget};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    Worker,
    Validator,
    Research,
    Design,
    General,
}

impl Role {
    pub const ALL: [Role; 5] = [
        Role::Worker,
        Role::Validator,
        Role::Research,
        Role::Design,
        Role::General,
    ];

    pub fn parse(name: &str) -> Option<Role> {
        Some(match name.trim().to_ascii_lowercase().as_str() {
            "worker" | "implementation" => Role::Worker,
            "validator" | "review" | "test" => Role::Validator,
            "research" => Role::Research,
            "design" => Role::Design,
            "general" => Role::General,
            _ => return None,
        })
    }

    pub fn name(self) -> &'static str {
        match self {
            Role::Worker => "worker",
            Role::Validator => "validator",
            Role::Research => "research",
            Role::Design => "design",
            Role::General => "general",
        }
    }

    pub fn names() -> String {
        Role::ALL.map(Role::name).join(", ")
    }
}

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

#[cfg(test)]
mod tests {
    use super::*;
    use zeron_proto::ReasoningLevel;

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
}
