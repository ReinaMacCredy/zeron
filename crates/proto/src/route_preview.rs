use crate::routing::{Preview, Role};
use crate::{ChatConfig, RouteTarget, TaskStep};

pub fn target_of(config: &ChatConfig) -> RouteTarget {
    RouteTarget {
        harness: config.harness,
        model: config.model.clone(),
        reasoning: config.reasoning,
    }
}

pub fn same_model(a: &RouteTarget, b: &RouteTarget) -> bool {
    a.harness == b.harness && a.model == b.model
}

pub fn plan_steps<'a>(chat_id: &str, steps: &'a [TaskStep]) -> Vec<&'a TaskStep> {
    let mut steps: Vec<&TaskStep> = steps
        .iter()
        .filter(|step| step.owner_chat_id == chat_id && !step.deleted)
        .collect();
    steps.sort_by(|a, b| a.order_key.cmp(&b.order_key));
    steps
}

pub fn role_of(step: &TaskStep) -> Option<&'static str> {
    step.role.as_deref().and_then(Role::parse).map(Role::name)
}

/// Changes whenever the step set or a role changes, so the preview refetches.
pub fn preview_key(steps: &[&TaskStep]) -> String {
    steps
        .iter()
        .map(|step| format!("{}={}", step.id, role_of(step).unwrap_or("")))
        .collect::<Vec<_>>()
        .join(",")
}

pub fn preview_roles(steps: &[&TaskStep]) -> Vec<&'static str> {
    let mut roles: Vec<&'static str> = Vec::new();
    for role in steps.iter().filter_map(|step| role_of(step)) {
        if !roles.contains(&role) {
            roles.push(role);
        }
    }
    roles
}

pub fn step_targets(
    steps: &[&TaskStep],
    lead: Option<&RouteTarget>,
    preview: Option<&Preview>,
) -> Vec<Option<RouteTarget>> {
    steps
        .iter()
        .map(|step| {
            let role = role_of(step);
            let row = role.and_then(|role| {
                preview?
                    .rows
                    .iter()
                    .find(|row| row.role == role && row.source != "off")
            });
            match row {
                Some(row) => row
                    .fallback
                    .as_ref()
                    .and_then(|fallback| fallback.target.clone())
                    .or_else(|| row.target.clone()),
                None if role.is_none() || preview.is_some() => lead.cloned(),
                None => None,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_unstarted_step_runs_where_its_role_routes_else_on_its_lead() {
        let target = |harness: &str| -> RouteTarget {
            serde_json::from_value(serde_json::json!({ "harness": harness })).unwrap()
        };
        let step = |role: Option<&str>| -> TaskStep {
            serde_json::from_value(serde_json::json!({ "id": "s", "role": role })).unwrap()
        };
        let preview: Preview = serde_json::from_value(serde_json::json!({ "rows": [
            { "role": "worker", "source": "routed", "target": { "harness": "codex" } },
            { "role": "design", "source": "fell_back", "target": { "harness": "codex" },
              "fallback": { "target": { "harness": "claude-code" }, "reason": "out of quota" } },
            { "role": "research", "source": "off", "target": { "harness": "codex" } },
        ] }))
        .unwrap();
        let steps = [
            step(None),
            step(Some("worker")),
            step(Some("design")),
            step(Some("research")),
        ];
        let steps: Vec<&TaskStep> = steps.iter().collect();
        let lead = target("mock");
        assert_eq!(
            step_targets(&steps, Some(&lead), Some(&preview)),
            [
                Some(lead.clone()),
                Some(target("codex")),
                Some(target("claude-code")),
                Some(lead.clone())
            ]
        );
        assert_eq!(
            step_targets(&steps, Some(&lead), None),
            [Some(lead.clone()), None, None, None],
            "a role waits for its route"
        );
    }
}
