use zeron_proto::routing::{
    MidRun, OutOfQuota, Preview, Role, RoleRoute, RoutingSettings, short_name, target_label,
};
use zeron_proto::{HarnessId, ReasoningLevel, RouteTarget};
use zeron_rpc::methods;

use super::{Page, Row, RowAction, RowKind, Section};
use crate::catalog::{HarnessInfo, ModelInfo, reasoning_label};
use crate::client::Client;
use crate::error::{ClientError, Result};

const INTRO: &str = "Which harness and model a delegated child runs on when the agent passes a \
                     role and no model. An explicit model in delegate_task always wins.";

async fn settings(client: &Client, device_id: &str) -> Result<RoutingSettings> {
    client
        .host_call(
            device_id,
            methods::GET_ROUTING_SETTINGS,
            serde_json::json!({}),
        )
        .await
}

async fn save(client: &Client, device_id: &str, settings: &RoutingSettings) -> Result<()> {
    client
        .inner
        .host_rpc(
            device_id,
            methods::SET_ROUTING_SETTINGS,
            serde_json::to_value(settings).unwrap_or_default(),
        )
        .await?;
    Ok(())
}

fn harness_of(id: &str) -> Option<HarnessId> {
    serde_json::from_value(serde_json::Value::String(id.to_owned())).ok()
}

fn wire<T: serde::Serialize>(value: T) -> String {
    serde_json::to_value(value)
        .ok()
        .and_then(|v| v.as_str().map(str::to_owned))
        .unwrap_or_default()
}

fn offered(harnesses: Vec<HarnessInfo>) -> Vec<(HarnessId, String)> {
    harnesses
        .into_iter()
        .filter(|h| h.installed && h.enabled != Some(false))
        .filter_map(|h| Some((harness_of(&h.id)?, h.label)))
        .filter(|(harness, _)| *harness != HarnessId::Acp)
        .collect()
}

pub(super) fn route_label(route: &RoleRoute) -> String {
    match route {
        RoleRoute::Off => "Off (agent decides)".into(),
        RoleRoute::SameAsParent => "Same as parent · Parent's model".into(),
        RoleRoute::Target(target) => {
            let mut parts = vec![target.harness.display_name().to_owned()];
            parts.extend(target.model.clone());
            parts.extend(target.reasoning.map(|r| reasoning_label(&wire(r))));
            parts.join(" · ")
        }
    }
}

pub(super) fn summary(settings: &RoutingSettings) -> String {
    let routed: Vec<String> = Role::ALL
        .iter()
        .filter_map(|role| match settings.route(*role) {
            RoleRoute::Target(target) => Some(format!(
                "{} → {}",
                role.name(),
                target.harness.display_name()
            )),
            _ => None,
        })
        .collect();
    if routed.is_empty() {
        "Agents pick".into()
    } else {
        routed.join(" · ")
    }
}

pub(super) async fn page(client: &Client, device_id: &str, page: &str) -> Result<Page> {
    let settings = settings(client, device_id).await?;
    match page {
        "routing" => {
            let preview: Option<Preview> = client
                .host_call(device_id, methods::PREVIEW_ROUTES, serde_json::json!({}))
                .await
                .ok();
            Ok(build(&settings, preview.as_ref()))
        }
        "routing:policy" => Ok(policy_page(&settings)),
        "routing:mid-run" => Ok(mid_run_page(&settings)),
        "routing:fallback-add" => {
            let harnesses = offered(client.list_harnesses(device_id).await);
            Ok(fallback_add_page(&settings, &harnesses))
        }
        _ => {
            let role = page
                .strip_prefix("routing:role:")
                .and_then(Role::parse)
                .ok_or_else(|| ClientError::NotFound(page.to_owned()))?;
            let harnesses = offered(client.list_harnesses(device_id).await);
            let models = match settings.route(role) {
                RoleRoute::Target(target) => {
                    client.list_models(device_id, &wire(target.harness)).await
                }
                _ => Vec::new(),
            };
            Ok(role_page(&settings, role, &harnesses, &models))
        }
    }
}

pub(super) async fn act(client: &Client, device_id: &str, page: &str, action: &str) -> Result<()> {
    let mut settings = settings(client, device_id).await?;
    let models = match page.strip_prefix("routing:role:").and_then(Role::parse) {
        Some(role) => match (settings.route(role), action.strip_prefix("harness:")) {
            (_, Some(harness)) => client.list_models(device_id, harness).await,
            (RoleRoute::Target(target), None) => {
                client.list_models(device_id, &wire(target.harness)).await
            }
            _ => Vec::new(),
        },
        None => Vec::new(),
    };
    if !edit(&mut settings, page, action, &models) {
        return Err(ClientError::InvalidArgument(action.to_owned()));
    }
    save(client, device_id, &settings).await
}

fn edit(settings: &mut RoutingSettings, page: &str, action: &str, models: &[ModelInfo]) -> bool {
    if let Some(policy) = action.strip_prefix("policy:") {
        let Some(policy) = OutOfQuota::ALL.into_iter().find(|p| wire(*p) == policy) else {
            return false;
        };
        settings.out_of_quota = policy;
        return true;
    }
    if let Some(mid_run) = action.strip_prefix("mid-run:") {
        let Some((mid_run, _)) = MidRun::ALL.into_iter().find(|(m, _)| wire(*m) == mid_run) else {
            return false;
        };
        settings.mid_run = mid_run;
        return true;
    }
    if let Some(ix) = action.strip_prefix("fallback-remove:") {
        let Some(ix) = ix
            .parse::<usize>()
            .ok()
            .filter(|ix| *ix < settings.fallback.len())
        else {
            return false;
        };
        settings.fallback.remove(ix);
        return true;
    }
    if let Some(harness) = action.strip_prefix("fallback-add:") {
        let Some(harness) = harness_of(harness) else {
            return false;
        };
        settings.fallback.push(RouteTarget {
            harness,
            model: None,
            reasoning: None,
        });
        return true;
    }
    let Some(role) = page.strip_prefix("routing:role:").and_then(Role::parse) else {
        return false;
    };
    let route = match action {
        "route:off" => RoleRoute::Off,
        "route:same" => RoleRoute::SameAsParent,
        _ => {
            let mut target = match (settings.route(role), action.strip_prefix("harness:")) {
                (_, Some(harness)) => match harness_of(harness) {
                    Some(harness) => RouteTarget {
                        harness,
                        model: models.first().map(|m| m.id.clone()),
                        reasoning: None,
                    },
                    None => return false,
                },
                (RoleRoute::Target(target), None) => target.clone(),
                _ => return false,
            };
            if let Some(model) = action.strip_prefix("model:") {
                target.model = Some(model.to_owned());
                let levels = models
                    .iter()
                    .find(|m| m.id == model)
                    .map(|m| m.reasoning_levels.clone())
                    .unwrap_or_default();
                if target.reasoning.is_some_and(|r| !levels.contains(&wire(r))) {
                    target.reasoning = None;
                }
            } else if let Some(level) = action.strip_prefix("reasoning:") {
                target.reasoning = if level.is_empty() {
                    None
                } else {
                    match serde_json::from_value::<ReasoningLevel>(level.into()) {
                        Ok(level) => Some(level),
                        Err(_) => return false,
                    }
                };
            } else if !action.starts_with("harness:") {
                return false;
            }
            RoleRoute::Target(target)
        }
    };
    settings.roles.insert(role, route);
    true
}

fn role_subtitle(settings: &RoutingSettings, role: Role, preview: Option<&Preview>) -> String {
    let row = preview.and_then(|p| p.rows.iter().find(|row| row.role == role.name()));
    if let Some(line) = row.and_then(|row| row.fallback_line()) {
        return line;
    }
    match (settings.route(role), row) {
        (RoleRoute::Target(target), Some(row)) => match row.quota.used_fraction {
            Some(used) if row.quota.known => format!(
                "{} {}%",
                short_name(target.harness),
                (used * 100.0).round() as u32
            ),
            _ => "No quota data, falls back after the first limit error".into(),
        },
        _ => role.summary().into(),
    }
}

pub(super) fn build(settings: &RoutingSettings, preview: Option<&Preview>) -> Page {
    let roles = Role::ALL
        .iter()
        .map(|role| {
            let fell_back = preview
                .and_then(|p| p.rows.iter().find(|row| row.role == role.name()))
                .is_some_and(|row| row.fell_back());
            Row::new(
                format!("role:{}", role.name()),
                if fell_back {
                    format!("{} · fell back", role.name())
                } else {
                    role.name().to_owned()
                },
                RowKind::Link {
                    page: format!("routing:role:{}", role.name()),
                },
            )
            .subtitle(Some(format!(
                "{}\n{}",
                route_label(settings.route(*role)),
                role_subtitle(settings, *role, preview)
            )))
        })
        .collect();
    let policy = settings.out_of_quota;
    let policy_value = if policy == OutOfQuota::Native {
        format!("{} (Recommended)", policy.label())
    } else {
        policy.label().to_owned()
    };
    let mid_run = MidRun::ALL
        .iter()
        .find(|(m, _)| *m == settings.mid_run)
        .map_or("", |(_, label)| label);
    let mut quota_rows = vec![
        Row::new(
            "policy",
            "When the routed harness is out of quota",
            RowKind::Link {
                page: "routing:policy".into(),
            },
        )
        .subtitle(Some(policy_value)),
    ];
    quota_rows.extend(settings.fallback.iter().enumerate().map(|(ix, target)| {
        let mut row = Row::new(
            format!("fallback:{ix}"),
            format!("{}. {}", ix + 1, target_label(target)),
            RowKind::Info,
        );
        row.actions.push(RowAction {
            id: format!("fallback-remove:{ix}"),
            title: "Remove".into(),
            destructive: true,
            confirm: None,
        });
        row
    }));
    quota_rows.push(Row::new(
        "fallback-add",
        "Add to fallback list",
        RowKind::Link {
            page: "routing:fallback-add".into(),
        },
    ));
    quota_rows.push(
        Row::new(
            "mid-run",
            "Mid-run limit",
            RowKind::Link {
                page: "routing:mid-run".into(),
            },
        )
        .value(Some(mid_run.to_owned())),
    );
    Page {
        title: "Routing".into(),
        sections: vec![
            Section {
                title: Some("Roles".into()),
                footer: Some(INTRO.into()),
                rows: roles,
            },
            Section {
                title: Some("When out of quota".into()),
                footer: Some(
                    "The fallback list is used by \"Next in fallback list\". Harnesses without \
                     quota are skipped."
                        .into(),
                ),
                rows: quota_rows,
            },
        ],
    }
}

fn policy_page(settings: &RoutingSettings) -> Page {
    let rows = OutOfQuota::ALL
        .iter()
        .map(|policy| {
            Row::new(
                format!("policy:{}", wire(*policy)),
                if *policy == OutOfQuota::Native {
                    format!("{} (Recommended)", policy.label())
                } else {
                    policy.label().to_owned()
                },
                RowKind::Check {
                    on: settings.out_of_quota == *policy,
                },
            )
            .subtitle(Some(policy.detail().to_owned()))
        })
        .collect();
    Page {
        title: "When out of quota".into(),
        sections: vec![Section {
            title: None,
            footer: Some(
                "Checked before start for harnesses that report usage. The others report no \
                 quota, so this applies after their first limit error."
                    .into(),
            ),
            rows,
        }],
    }
}

fn mid_run_page(settings: &RoutingSettings) -> Page {
    let rows = MidRun::ALL
        .iter()
        .map(|(mid_run, label)| {
            Row::new(
                format!("mid-run:{}", wire(*mid_run)),
                *label,
                RowKind::Check {
                    on: settings.mid_run == *mid_run,
                },
            )
        })
        .collect();
    Page {
        title: "Mid-run limit".into(),
        sections: vec![Section {
            title: None,
            footer: Some(
                "When a running child hits its limit, act on the choice above after a \
                 countdown, or always ask."
                    .into(),
            ),
            rows,
        }],
    }
}

fn fallback_add_page(settings: &RoutingSettings, harnesses: &[(HarnessId, String)]) -> Page {
    let rows: Vec<Row> = harnesses
        .iter()
        .filter(|(harness, _)| !settings.fallback.iter().any(|t| t.harness == *harness))
        .map(|(harness, label)| {
            Row::new(
                format!("fallback-add:{}", wire(*harness)),
                label.clone(),
                RowKind::Check { on: false },
            )
        })
        .collect();
    Page {
        title: "Add to fallback list".into(),
        sections: vec![Section {
            rows: if rows.is_empty() {
                vec![Row::new(
                    "empty",
                    "Every harness is in the list",
                    RowKind::Info,
                )]
            } else {
                rows
            },
            ..Section::default()
        }],
    }
}

fn role_page(
    settings: &RoutingSettings,
    role: Role,
    harnesses: &[(HarnessId, String)],
    models: &[ModelInfo],
) -> Page {
    let route = settings.route(role);
    let target = match route {
        RoleRoute::Target(target) => Some(target),
        _ => None,
    };
    let mut harness_rows = vec![
        Row::new(
            "route:off",
            "Off (agent decides)",
            RowKind::Check {
                on: *route == RoleRoute::Off,
            },
        )
        .subtitle(Some(
            "Agent picks harness and model for this role".to_owned(),
        )),
        Row::new(
            "route:same",
            "Same as parent",
            RowKind::Check {
                on: *route == RoleRoute::SameAsParent,
            },
        )
        .subtitle(Some("Parent's model and reasoning".to_owned())),
    ];
    harness_rows.extend(harnesses.iter().map(|(harness, label)| {
        Row::new(
            format!("harness:{}", wire(*harness)),
            label.clone(),
            RowKind::Check {
                on: target.is_some_and(|t| t.harness == *harness),
            },
        )
    }));
    let mut sections = vec![Section {
        title: Some("Harness".into()),
        footer: Some(role.summary().into()),
        rows: harness_rows,
    }];
    if let Some(target) = target {
        sections.push(Section {
            title: Some("Model".into()),
            footer: None,
            rows: models
                .iter()
                .map(|model| {
                    Row::new(
                        format!("model:{}", model.id),
                        model.label.clone(),
                        RowKind::Check {
                            on: target.model.as_deref() == Some(model.id.as_str()),
                        },
                    )
                })
                .collect(),
        });
        let levels = models
            .iter()
            .find(|m| target.model.as_deref() == Some(m.id.as_str()))
            .map(|m| m.reasoning_levels.clone())
            .unwrap_or_default();
        if !levels.is_empty() {
            let mut rows = vec![Row::new(
                "reasoning:",
                "Default",
                RowKind::Check {
                    on: target.reasoning.is_none(),
                },
            )];
            rows.extend(levels.iter().map(|level| {
                Row::new(
                    format!("reasoning:{level}"),
                    reasoning_label(level),
                    RowKind::Check {
                        on: target.reasoning.map(wire).as_deref() == Some(level.as_str()),
                    },
                )
            }));
            sections.push(Section {
                title: Some("Reasoning".into()),
                footer: None,
                rows,
            });
        }
    }
    Page {
        title: role.name().into(),
        sections,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn model(id: &str, levels: &[&str]) -> ModelInfo {
        serde_json::from_value(serde_json::json!({
            "id": id, "label": id, "reasoningLevels": levels,
        }))
        .unwrap()
    }

    #[test]
    fn picking_a_harness_then_a_model_drops_a_level_the_model_lacks() {
        let mut settings = RoutingSettings::default();
        let codex = [
            model("gpt-5.5", &["low", "high"]),
            model("gpt-5.5-mini", &["low"]),
        ];
        assert!(edit(
            &mut settings,
            "routing:role:worker",
            "harness:codex",
            &codex
        ));
        assert!(edit(
            &mut settings,
            "routing:role:worker",
            "reasoning:high",
            &codex
        ));
        assert_eq!(
            route_label(settings.route(Role::Worker)),
            "Codex · gpt-5.5 · High"
        );
        assert!(edit(
            &mut settings,
            "routing:role:worker",
            "model:gpt-5.5-mini",
            &codex
        ));
        let RoleRoute::Target(target) = settings.route(Role::Worker) else {
            panic!("worker routed");
        };
        assert_eq!(target.model.as_deref(), Some("gpt-5.5-mini"));
        assert_eq!(target.reasoning, None);
        assert_eq!(summary(&settings), "worker → Codex");
        assert!(edit(&mut settings, "routing:role:worker", "route:off", &[]));
        assert_eq!(*settings.route(Role::Worker), RoleRoute::Off);
    }

    #[test]
    fn policy_mid_run_and_fallback_edits_round_trip_their_wire_ids() {
        let mut settings = RoutingSettings::default();
        assert!(edit(
            &mut settings,
            "routing:policy",
            "policy:nextInList",
            &[]
        ));
        assert_eq!(settings.out_of_quota, OutOfQuota::NextInList);
        assert!(edit(
            &mut settings,
            "routing:mid-run",
            "mid-run:after10s",
            &[]
        ));
        assert_eq!(settings.mid_run, MidRun::After10s);
        assert!(edit(
            &mut settings,
            "routing:fallback-add",
            "fallback-add:claude-code",
            &[]
        ));
        assert_eq!(settings.fallback[0].harness, HarnessId::ClaudeCode);
        assert!(edit(&mut settings, "routing", "fallback-remove:0", &[]));
        assert!(settings.fallback.is_empty());
        assert!(!edit(&mut settings, "routing", "fallback-remove:0", &[]));
        let page = build(&settings, None);
        assert_eq!(page.sections[0].rows.len(), Role::ALL.len());
    }
}
