use serde::{Deserialize, Serialize};

use crate::{Chat, ChatConfig, ChildRoute, RouteTarget, TaskStep};

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

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct QuotaState {
    pub known: bool,
    pub used_fraction: Option<f64>,
    pub resets_at: Option<i64>,
    pub exhausted: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Fallback {
    pub target: Option<RouteTarget>,
    pub native: bool,
    pub reason: String,
    pub resets_at: Option<i64>,
    pub multiplier: Option<f64>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Row {
    pub role: String,
    pub source: String,
    pub target: Option<RouteTarget>,
    pub multiplier: Option<f64>,
    pub quota: QuotaState,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fallback: Option<Fallback>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Preview {
    pub base: Option<RouteTarget>,
    pub rows: Vec<Row>,
    pub guidance: String,
}

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

pub fn plan_steps<'a>(task_id: &str, steps: &'a [TaskStep]) -> Vec<&'a TaskStep> {
    let mut steps: Vec<&TaskStep> = steps
        .iter()
        .filter(|step| step.task_id == task_id && !step.deleted)
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

#[derive(Debug, Clone, PartialEq)]
pub struct PlanLine {
    pub title: String,
    pub role: Option<&'static str>,
    pub target: Option<RouteTarget>,
    pub agent_picks: bool,
    pub native: bool,
    pub multiplier: Option<f64>,
    pub planned: Option<f64>,
    pub fallback: Option<String>,
}

pub struct Labels<'a> {
    pub runs_as: &'a dyn Fn(&RouteTarget, bool) -> String,
    pub clock: &'a dyn Fn(i64) -> Option<String>,
}

pub fn plan_lines(
    steps: &[&TaskStep],
    chats: &[Chat],
    lead: Option<&RouteTarget>,
    preview: Option<&Preview>,
    labels: &Labels,
) -> Vec<PlanLine> {
    let until = |resets_at: Option<i64>| {
        resets_at
            .and_then(labels.clock)
            .map(|at| format!(" until {at}"))
            .unwrap_or_default()
    };
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
            let mut line = PlanLine {
                title: step.title.clone(),
                role,
                target: None,
                agent_picks: false,
                native: false,
                multiplier: None,
                planned: None,
                fallback: None,
            };
            let ran = step.chat_id.as_deref().and_then(|id| {
                let chat = chats.iter().find(|chat| chat.id == id)?;
                chat.config.as_ref().map(target_of)
            });
            if let Some(ran) = ran {
                line.multiplier = row
                    .and_then(|row| {
                        if row.target.as_ref().is_some_and(|t| same_model(t, &ran)) {
                            return row.multiplier;
                        }
                        let fallback = row.fallback.as_ref()?;
                        fallback
                            .target
                            .as_ref()
                            .is_some_and(|t| same_model(t, &ran))
                            .then_some(fallback.multiplier)
                            .flatten()
                    })
                    .or_else(|| {
                        lead.is_some_and(|lead| same_model(lead, &ran))
                            .then_some(1.0)
                    });
                line.planned = line.multiplier;
                line.target = Some(ran);
                return line;
            }
            let Some(row) = row else {
                if role.is_none() || preview.is_some() {
                    line.agent_picks = true;
                    line.target = lead.cloned();
                    line.multiplier = Some(1.0);
                    line.planned = Some(1.0);
                }
                return line;
            };
            let role = role.unwrap_or("general");
            line.target = row.target.clone();
            line.multiplier = row.multiplier;
            line.planned = row.multiplier;
            if let Some(fallback) = &row.fallback {
                match &fallback.target {
                    Some(target) => {
                        line.fallback = Some(format!(
                            "{role}: {}{}, runs as {}",
                            fallback.reason,
                            until(fallback.resets_at),
                            (labels.runs_as)(target, fallback.native)
                        ));
                        line.target = Some(target.clone());
                        line.native = fallback.native;
                        line.multiplier = fallback.multiplier;
                    }
                    None => {
                        let harness = row
                            .target
                            .as_ref()
                            .map_or("The route", |target| target.harness.display_name());
                        line.fallback = Some(format!(
                            "{role}: {harness} out of quota{}, {}",
                            until(fallback.resets_at),
                            fallback.reason
                        ));
                    }
                }
            } else if row.source == ChildRoute::FELL_BACK {
                line.fallback = Some(format!(
                    "{role}: {}, runs as {}",
                    row.reason.as_deref().unwrap_or("route unavailable"),
                    row.target.as_ref().map_or_else(
                        || "this chat's model".to_string(),
                        |t| (labels.runs_as)(t, false)
                    )
                ));
            }
            line
        })
        .collect()
}

pub fn multiplier_text(multiplier: f64) -> String {
    let text = format!("{multiplier:.2}");
    let text = text.trim_end_matches('0').trim_end_matches('.');
    format!("{text}x")
}

pub struct Totals {
    pub total: f64,
    pub planned: f64,
    pub raised: usize,
}

pub fn totals(lines: &[PlanLine]) -> Totals {
    let cost = |m: Option<f64>| m.unwrap_or(1.0);
    Totals {
        total: lines.iter().map(|line| cost(line.multiplier)).sum(),
        planned: lines
            .iter()
            .map(|line| cost(line.planned.or(line.multiplier)))
            .sum(),
        raised: lines
            .iter()
            .filter(|line| {
                line.fallback.is_some() && cost(line.multiplier) > cost(line.planned) + 1e-9
            })
            .count(),
    }
}

pub fn mix(lines: &[PlanLine]) -> Vec<(RouteTarget, bool, usize)> {
    let mut groups: Vec<(RouteTarget, bool, usize)> = Vec::new();
    for line in lines {
        let Some(target) = &line.target else { continue };
        match groups
            .iter_mut()
            .find(|(t, native, _)| same_model(t, target) && *native == line.native)
        {
            Some(group) => group.2 += 1,
            None => groups.push((target.clone(), line.native, 1)),
        }
    }
    groups
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(multiplier: f64, planned: f64, fallback: bool) -> PlanLine {
        PlanLine {
            title: String::new(),
            role: Some("worker"),
            target: None,
            agent_picks: false,
            native: false,
            multiplier: Some(multiplier),
            planned: Some(planned),
            fallback: fallback.then(String::new),
        }
    }

    #[test]
    fn totals_count_only_fallbacks_that_raise_the_cost() {
        let t = totals(&[
            line(1.0, 0.04, true),
            line(0.2, 0.2, false),
            line(1.0, 1.0, true),
        ]);
        assert!((t.total - 2.2).abs() < 1e-9);
        assert!((t.planned - 1.24).abs() < 1e-9);
        assert_eq!(t.raised, 1);
    }
}
