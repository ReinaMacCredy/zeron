use std::rc::Rc;

use gpui::{AnyElement, App, SharedString, Window, div, prelude::*, px};
use zeron_engine::routing::Role;
use zeron_proto::{Chat, ChildRoute, RouteTarget, Task, TaskStep, ToolCall};

use crate::route_badge::{self, OnRouteLink, RouteInfo, RouteLink, same_model, target_text};
use crate::theme::Theme;

pub(crate) use zeron_engine::routing::preview::Preview;

pub(crate) fn plan_task_id(call: &ToolCall) -> Option<String> {
    let input = match call {
        ToolCall::Mcp {
            server,
            tool,
            input,
        } if server == "zeron" && tool == "tasks" => input.as_ref(),
        ToolCall::Unknown { name, input } if name.ends_with("zeron__tasks") => input.as_ref(),
        _ => None,
    }?;
    (input.get("action")?.as_str()? == "plan")
        .then(|| input.get("taskId")?.as_str().map(str::to_owned))
        .flatten()
        .filter(|id| !id.is_empty())
}

pub(crate) fn plan_steps<'a>(task_id: &str, steps: &'a [TaskStep]) -> Vec<&'a TaskStep> {
    let mut steps: Vec<&TaskStep> = steps
        .iter()
        .filter(|step| step.task_id == task_id && !step.deleted)
        .collect();
    steps.sort_by(|a, b| a.order_key.cmp(&b.order_key));
    steps
}

pub(crate) fn role_of(step: &TaskStep) -> Option<&'static str> {
    step.role.as_deref().and_then(Role::parse).map(Role::name)
}

/// Changes whenever the step set or a role changes, so the preview refetches.
pub(crate) fn preview_key(steps: &[&TaskStep]) -> String {
    steps
        .iter()
        .map(|step| format!("{}={}", step.id, role_of(step).unwrap_or("")))
        .collect::<Vec<_>>()
        .join(",")
}

pub(crate) fn preview_roles(steps: &[&TaskStep]) -> Vec<&'static str> {
    let mut roles: Vec<&'static str> = Vec::new();
    for role in steps.iter().filter_map(|step| role_of(step)) {
        if !roles.contains(&role) {
            roles.push(role);
        }
    }
    roles
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct PlanLine {
    pub title: String,
    pub role: Option<&'static str>,
    pub target: Option<RouteTarget>,
    pub agent_picks: bool,
    pub native: bool,
    pub multiplier: Option<f64>,
    pub planned: Option<f64>,
    pub fallback: Option<String>,
    pub badge: Option<RouteInfo>,
}

fn until(resets_at: Option<i64>) -> String {
    resets_at
        .and_then(route_badge::clock)
        .map(|at| format!(" until {at}"))
        .unwrap_or_default()
}

fn runs_as(target: &RouteTarget, native: bool, cx: &App) -> String {
    if native {
        format!("{} native", target.harness.display_name())
    } else {
        target_text(target, cx)
    }
}

pub(crate) fn plan_lines(
    steps: &[&TaskStep],
    chats: &[Chat],
    lead: Option<&RouteTarget>,
    preview: Option<&Preview>,
    cx: &App,
) -> Vec<PlanLine> {
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
                badge: None,
            };
            let bound = step.chat_id.as_deref().and_then(|id| {
                let chat = chats.iter().find(|chat| chat.id == id)?;
                Some((id, chat.config.as_ref().map(route_badge::target_of)?))
            });
            if let Some((chat_id, ran)) = bound {
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
                line.badge = step
                    .route
                    .clone()
                    .map(|route| RouteInfo::for_child(chats, chat_id, route));
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
                            runs_as(target, fallback.native, cx)
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
                    row.target
                        .as_ref()
                        .map_or_else(|| "this chat's model".to_string(), |t| target_text(t, cx))
                ));
            }
            line
        })
        .collect()
}

pub(crate) fn multiplier_text(multiplier: f64) -> String {
    let text = format!("{multiplier:.2}");
    let text = text.trim_end_matches('0').trim_end_matches('.');
    format!("{text}x")
}

pub(crate) struct Totals {
    pub total: f64,
    pub planned: f64,
    pub raised: usize,
}

pub(crate) fn totals(lines: &[PlanLine]) -> Totals {
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

fn mix_label(target: &RouteTarget, native: bool, cx: &App) -> String {
    let harness = target.harness.display_name();
    let harness = if native {
        format!("{harness} native")
    } else {
        harness.to_string()
    };
    match &target.model {
        Some(model) => format!(
            "{harness} · {}",
            crate::pickers::model_display_label(target.harness, model, cx)
        ),
        None => harness,
    }
}

pub(crate) fn mix(lines: &[PlanLine], cx: &App) -> Vec<(RouteTarget, String, usize)> {
    let mut groups: Vec<(RouteTarget, bool, String, usize)> = Vec::new();
    for line in lines {
        let Some(target) = &line.target else { continue };
        match groups
            .iter_mut()
            .find(|(t, native, _, _)| same_model(t, target) && *native == line.native)
        {
            Some(group) => group.3 += 1,
            None => groups.push((
                target.clone(),
                line.native,
                mix_label(target, line.native, cx),
                1,
            )),
        }
    }
    groups
        .into_iter()
        .map(|(target, _, label, count)| (target, label, count))
        .collect()
}

pub(crate) struct PlanCard<'a> {
    pub id: SharedString,
    pub task: &'a Task,
    pub lines: Vec<PlanLine>,
    pub lead: Option<RouteTarget>,
    pub on_link: OnRouteLink,
    pub on_start: Rc<dyn Fn(&mut Window, &mut App)>,
}

fn small(text: impl Into<SharedString>, color: gpui::Hsla) -> gpui::Div {
    div()
        .text_size(crate::typography::ui_rems(11.5))
        .text_color(color)
        .child(text.into())
}

pub(crate) fn plan_card(card: PlanCard, theme: &Theme, cx: &App) -> AnyElement {
    let PlanCard {
        id,
        task,
        lines,
        lead,
        on_link,
        on_start,
    } = card;
    let cyan = route_badge::fell_back_tint(theme);
    let count = lines.len();
    let totals = totals(&lines);
    let has_fallback = lines.iter().any(|line| line.fallback.is_some());
    let lead_label = lead
        .as_ref()
        .and_then(|lead| {
            lead.model
                .as_deref()
                .map(|model| crate::pickers::model_display_label(lead.harness, model, cx))
        })
        .unwrap_or_else(|| "model".to_string());
    let header = div()
        .h(px(36.0))
        .px(px(12.0))
        .flex()
        .flex_row()
        .items_center()
        .gap(px(8.0))
        .border_b_1()
        .border_color(crate::theme::hairline(0.07))
        .child(
            crate::icons::icon(crate::icons::CHECKLIST)
                .size(px(14.0))
                .flex_none()
                .text_color(theme.text_muted),
        )
        .child(
            div()
                .flex_1()
                .font_weight(gpui::FontWeight::MEDIUM)
                .text_color(theme.text)
                .child(if task.plan_pending {
                    "Plan ready"
                } else {
                    "Plan approved"
                }),
        )
        .child(small(
            format!("{count} {}", if count == 1 { "child" } else { "children" }),
            theme.text_faint,
        ));
    let rows = lines.iter().enumerate().map(|(ix, line)| {
        let target_label = match (&line.target, line.agent_picks) {
            (Some(target), true) => format!(
                "agent picks · {}",
                target
                    .model
                    .as_deref()
                    .map(|model| crate::pickers::model_display_label(target.harness, model, cx))
                    .unwrap_or_else(|| target.harness.display_name().to_string())
            ),
            (None, true) => "agent picks".to_string(),
            (Some(target), false) => runs_as(target, line.native, cx),
            (None, false) => "…".to_string(),
        };
        let row = div()
            .min_h(px(28.0))
            .flex()
            .flex_row()
            .items_center()
            .gap(px(10.0))
            .child(
                div()
                    .w(px(14.0))
                    .flex_none()
                    .text_color(theme.text_faint)
                    .child(SharedString::from((ix + 1).to_string())),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_color(theme.text.opacity(0.9))
                    .child(SharedString::from(line.title.clone())),
            )
            .when_some(line.role, |row, role| {
                row.child(small(role, theme.text_faint).flex_none())
            })
            .child(
                div()
                    .flex_none()
                    .max_w(px(260.0))
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(5.0))
                    .when_some(line.target.as_ref(), |el, target| {
                        el.child(route_badge::brand_mark(target, 12.0, theme))
                    })
                    .child(
                        div()
                            .min_w_0()
                            .truncate()
                            .text_color(theme.text_muted)
                            .child(SharedString::from(target_label)),
                    ),
            )
            .when_some(line.badge.clone(), |row, info| {
                row.children(route_badge::route_badge(
                    SharedString::from(format!("{id}-badge-{ix}")),
                    info,
                    on_link.clone(),
                    theme,
                ))
            })
            .child(
                div()
                    .w(px(40.0))
                    .flex_none()
                    .flex()
                    .justify_end()
                    .text_color(theme.text_faint)
                    .child(SharedString::from(
                        line.multiplier.map(multiplier_text).unwrap_or_default(),
                    )),
            );
        div()
            .flex()
            .flex_col()
            .child(row)
            .when_some(line.fallback.clone(), |el, fallback| {
                el.child(
                    div()
                        .pl(px(24.0))
                        .pb(px(4.0))
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap(px(6.0))
                        .text_size(crate::typography::ui_rems(11.5))
                        .text_color(cyan)
                        .child(
                            crate::icons::icon(crate::icons::FORK)
                                .size(px(11.0))
                                .flex_none()
                                .text_color(cyan),
                        )
                        .child(div().min_w_0().child(SharedString::from(fallback))),
                )
            })
    });
    let mix = mix(&lines, cx);
    let mix_line = div()
        .flex()
        .flex_row()
        .flex_wrap()
        .items_center()
        .gap(px(10.0))
        .child(
            div()
                .font_weight(gpui::FontWeight::MEDIUM)
                .text_color(theme.text)
                .child(SharedString::from(format!(
                    "{count} {}:",
                    if count == 1 { "child" } else { "children" }
                ))),
        )
        .children(mix.into_iter().map(|(target, label, n)| {
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(px(4.0))
                .text_color(theme.text_muted)
                .child(route_badge::brand_mark(&target, 12.0, theme))
                .child(SharedString::from(format!("{n} {label}")))
        }));
    let scale = totals.total.max(count as f64).max(f64::EPSILON);
    let bar = div()
        .flex_1()
        .h(px(4.0))
        .rounded(px(2.0))
        .overflow_hidden()
        .bg(crate::theme::ink(0.08))
        .flex()
        .flex_row()
        .gap(px(1.0))
        .children(lines.iter().map(|line| {
            let color = if line.fallback.is_some() {
                theme.warning
            } else {
                theme.text_muted
            };
            div()
                .h_full()
                .w(gpui::relative(
                    (line.multiplier.unwrap_or(1.0) / scale) as f32,
                ))
                .bg(color)
        }));
    let cost_line = div()
        .flex()
        .flex_row()
        .items_center()
        .gap(px(10.0))
        .child(small(
            format!("Relative cost {}", multiplier_text(totals.total)),
            theme.text_muted,
        ))
        .child(bar)
        .child(small(
            format!("vs {count}.0x if all ran on the lead's {lead_label}"),
            theme.text_faint,
        ));
    let jump = (totals.raised > 0 && totals.total > totals.planned + 1e-9).then(|| {
        let to = lines
            .iter()
            .filter(|line| line.fallback.is_some())
            .filter_map(|line| line.target.as_ref())
            .filter_map(|target| {
                target
                    .model
                    .as_deref()
                    .map(|model| crate::pickers::model_display_label(target.harness, model, cx))
            })
            .next()
            .unwrap_or_else(|| "the chat's model".to_string());
        div()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(6.0))
            .text_size(crate::typography::ui_rems(11.5))
            .text_color(theme.warning)
            .child(
                crate::icons::icon(crate::icons::LUCIDE_TRIANGLE_ALERT)
                    .size(px(12.0))
                    .flex_none()
                    .text_color(theme.warning),
            )
            .child(SharedString::from(format!(
                "Relative cost {} \u{2192} {} because {} {} fell back to {to}",
                multiplier_text(totals.planned),
                multiplier_text(totals.total),
                totals.raised,
                if totals.raised == 1 {
                    "child"
                } else {
                    "children"
                }
            )))
    });
    let edit = {
        let on_link = on_link.clone();
        div()
            .id(SharedString::from(format!("{id}-edit-routing")))
            .debug_selector(|| "plan-edit-routing".into())
            .role(gpui::Role::Button)
            .aria_label("Edit routing")
            .h(px(26.0))
            .px(px(10.0))
            .flex()
            .items_center()
            .rounded(px(8.0))
            .cursor_pointer()
            .text_color(theme.text.opacity(0.85))
            .hover(|s| s.bg(crate::theme::ink(0.06)))
            .on_click(move |_, window, cx| {
                cx.stop_propagation();
                on_link(RouteLink::Routing, window, cx);
            })
            .child("Edit routing")
    };
    let start_label = if has_fallback {
        "Start with fallback"
    } else {
        "Start mission"
    };
    let start = task.plan_pending.then(|| {
        div()
            .id(SharedString::from(format!("{id}-start")))
            .debug_selector(|| "plan-start".into())
            .role(gpui::Role::Button)
            .aria_label(start_label)
            .h(px(26.0))
            .px(px(12.0))
            .flex()
            .items_center()
            .rounded(px(13.0))
            .font_weight(gpui::FontWeight::MEDIUM)
            .cursor_pointer()
            .bg(theme.text)
            .text_color(theme.on_solid)
            .hover(|s| s.opacity(0.9))
            .on_click(move |_, window, cx| {
                cx.stop_propagation();
                on_start(window, cx);
            })
            .child(start_label)
    });
    let footer = div()
        .px(px(12.0))
        .py(px(10.0))
        .flex()
        .flex_col()
        .gap(px(8.0))
        .border_t_1()
        .border_color(crate::theme::hairline(0.07))
        .child(mix_line)
        .children(jump)
        .child(cost_line)
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(px(8.0))
                .child(
                    small(
                        "Per-model multipliers from models.dev prices. A floor: retries and \
                         reviews add runs.",
                        theme.text_faint,
                    )
                    .flex_1()
                    .min_w_0(),
                )
                .child(edit)
                .children(start),
        );
    div()
        .py(px(4.0))
        .w_full()
        .child(
            div()
                .id(id.clone())
                .debug_selector(|| "plan-estimate".into())
                .role(gpui::Role::Group)
                .aria_label("Plan estimate")
                .w_full()
                .flex()
                .flex_col()
                .overflow_hidden()
                .rounded(px(10.0))
                .border_1()
                .border_color(crate::theme::hairline(0.08))
                .bg(crate::theme::ink(0.03))
                .text_size(crate::typography::ui_rems(12.5))
                .child(header)
                .child(
                    div()
                        .px(px(12.0))
                        .py(px(6.0))
                        .flex()
                        .flex_col()
                        .children(rows),
                )
                .child(footer),
        )
        .into_any_element()
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
            badge: None,
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

    #[test]
    fn only_zeron_plan_calls_with_a_task_name_a_card() {
        let call = |server: &str, input: serde_json::Value| ToolCall::Mcp {
            server: server.into(),
            tool: "tasks".into(),
            input: Some(input),
        };
        assert_eq!(
            plan_task_id(&call(
                "zeron",
                serde_json::json!({"action": "plan", "taskId": "t1"})
            )),
            Some("t1".into())
        );
        assert_eq!(
            plan_task_id(&call(
                "zeron",
                serde_json::json!({"action": "list", "taskId": "t1"})
            )),
            None
        );
        assert_eq!(
            plan_task_id(&call(
                "other",
                serde_json::json!({"action": "plan", "taskId": "t1"})
            )),
            None
        );
    }
}
