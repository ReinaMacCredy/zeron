use std::ops::Deref;
use std::rc::Rc;

use gpui::{AnyElement, App, SharedString, Window, div, prelude::*, px};
use zeron_proto::route_preview::{self as rp, Labels};
use zeron_proto::{Chat, RouteTarget, Task, TaskStep, ToolCall};

use crate::route_badge::{self, OnRouteLink, RouteInfo, RouteLink, target_text};
use crate::theme::Theme;

pub(crate) use zeron_proto::route_preview::{
    Preview, multiplier_text, plan_steps, preview_key, preview_roles,
};

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

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct PlanLine {
    pub line: rp::PlanLine,
    pub badge: Option<RouteInfo>,
}

impl Deref for PlanLine {
    type Target = rp::PlanLine;

    fn deref(&self) -> &rp::PlanLine {
        &self.line
    }
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
    let labels = Labels {
        runs_as: &|target, native| runs_as(target, native, cx),
        clock: &route_badge::clock,
    };
    rp::plan_lines(steps, chats, lead, preview, &labels)
        .into_iter()
        .zip(steps)
        .map(|(line, step)| {
            let bound = step.chat_id.as_deref().filter(|id| {
                chats
                    .iter()
                    .any(|chat| chat.id == *id && chat.config.is_some())
            });
            let badge = bound.and_then(|chat_id| {
                step.route
                    .clone()
                    .map(|route| RouteInfo::for_child(chats, chat_id, route))
            });
            PlanLine { line, badge }
        })
        .collect()
}

fn inner(lines: &[PlanLine]) -> Vec<rp::PlanLine> {
    lines.iter().map(|line| line.line.clone()).collect()
}

pub(crate) fn totals(lines: &[PlanLine]) -> rp::Totals {
    rp::totals(&inner(lines))
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
    rp::mix(&inner(lines))
        .into_iter()
        .map(|(target, native, count)| {
            let label = mix_label(&target, native, cx);
            (target, label, count)
        })
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
        zeron_proto::mission::START_MISSION
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
