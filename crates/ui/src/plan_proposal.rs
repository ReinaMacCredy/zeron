use std::rc::Rc;
use std::sync::Arc;

use gpui::{AnyElement, App, SharedString, Window, div, prelude::*, px};
use zeron_doc::MessagePart;
use zeron_proto::{
    APPROVAL_ALLOW, APPROVAL_DENY, PLAN_APPROVAL_HEADER, ToolCall, UserInputAnswer,
    UserInputQuestion,
};

use crate::popover;
use crate::theme::Theme;

const MAX_STEPS: usize = 8;

pub(crate) fn plan_question(questions: &[UserInputQuestion]) -> Option<&UserInputQuestion> {
    questions
        .iter()
        .find(|question| question.approval && question.header == PLAN_APPROVAL_HEADER)
}

pub(crate) fn is_plan_question(questions: &[UserInputQuestion]) -> bool {
    plan_question(questions).is_some()
}

pub(crate) fn plan_answers(question_id: &str, allow: bool) -> Vec<UserInputAnswer> {
    vec![UserInputAnswer {
        question_id: question_id.to_owned(),
        labels: vec![if allow { APPROVAL_ALLOW } else { APPROVAL_DENY }.to_owned()],
    }]
}

pub(crate) fn proposed_plan(call: &ToolCall) -> Option<&str> {
    match call {
        ToolCall::Unknown { name, input } if name == "ExitPlanMode" => {
            input.as_ref()?.get("plan")?.as_str()
        }
        _ => None,
    }
}

pub(crate) fn entry_steps(parts: &[MessagePart]) -> Vec<String> {
    parts
        .iter()
        .rev()
        .find_map(|part| match part {
            MessagePart::Tool { call, .. } => proposed_plan(call),
            _ => None,
        })
        .map(proposal_steps)
        .unwrap_or_default()
}

fn list_item(line: &str) -> Option<&str> {
    if line.starts_with([' ', '\t']) {
        return None;
    }
    let rest = line
        .strip_prefix("- ")
        .or_else(|| line.strip_prefix("* "))
        .or_else(|| line.strip_prefix("+ "));
    if rest.is_some() {
        return rest;
    }
    let digits = line.len() - line.trim_start_matches(|c: char| c.is_ascii_digit()).len();
    (digits > 0)
        .then(|| {
            line[digits..]
                .strip_prefix(". ")
                .or_else(|| line[digits..].strip_prefix(") "))
        })
        .flatten()
}

fn clean(line: &str) -> String {
    line.replace("**", "").replace('`', "").trim().to_owned()
}

pub(crate) fn proposal_steps(plan: &str) -> Vec<String> {
    let items: Vec<String> = plan
        .lines()
        .filter_map(list_item)
        .map(clean)
        .filter(|step| !step.is_empty())
        .collect();
    let steps = if items.is_empty() {
        plan.lines()
            .map(str::trim)
            .filter(|line| !line.is_empty() && !line.starts_with('#'))
            .map(clean)
            .filter(|step| !step.is_empty())
            .collect()
    } else {
        items
    };
    steps.into_iter().take(MAX_STEPS).collect()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanAnswer {
    pub request_id: String,
    pub question_id: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PlanOutcome {
    Waiting,
    Approved,
    Declined,
}

pub(crate) fn plan_outcome(
    question_id: &str,
    resolved: bool,
    answers: Option<&[UserInputAnswer]>,
    live: bool,
) -> PlanOutcome {
    let allowed = answers.into_iter().flatten().any(|answer| {
        answer.question_id == question_id
            && answer
                .labels
                .iter()
                .any(|label| label.eq_ignore_ascii_case(APPROVAL_ALLOW))
    });
    match (allowed, resolved || answers.is_some() || !live) {
        (true, _) => PlanOutcome::Approved,
        (false, true) => PlanOutcome::Declined,
        (false, false) => PlanOutcome::Waiting,
    }
}

pub(crate) enum CardView {
    Waiting { busy: bool },
    Here,
    NotApproved,
}

pub(crate) struct ProposalCard {
    pub id: SharedString,
    pub steps: Arc<Vec<SharedString>>,
    pub view: CardView,
    pub on_approve: Rc<dyn Fn(&mut Window, &mut App)>,
    pub on_edit: Rc<dyn Fn(&mut Window, &mut App)>,
}

fn pill(text: impl Into<SharedString>, accent: bool, theme: &Theme) -> gpui::Div {
    div()
        .flex_none()
        .px(px(8.0))
        .py(px(2.0))
        .rounded(px(10.0))
        .text_size(crate::typography::ui_rems(12.0))
        .bg(if accent {
            theme.accent_wash
        } else {
            crate::theme::ink(0.06)
        })
        .text_color(if accent {
            theme.accent
        } else {
            theme.text_muted
        })
        .child(text.into())
}

fn step_row(ix: usize, title: SharedString, theme: &Theme) -> AnyElement {
    let badge = div()
        .size(px(18.0))
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .rounded_full()
        .border_1()
        .text_size(crate::typography::ui_rems(10.5))
        .border_color(theme.border_strong)
        .text_color(theme.text_muted)
        .child(SharedString::from((ix + 1).to_string()));
    div()
        .min_h(px(28.0))
        .flex()
        .flex_row()
        .items_center()
        .gap(px(10.0))
        .child(badge)
        .child(
            div()
                .min_w_0()
                .truncate()
                .text_color(theme.text)
                .child(title),
        )
        .into_any_element()
}

pub(crate) fn proposal_card(card: ProposalCard, theme: &Theme) -> AnyElement {
    let ProposalCard {
        id,
        steps,
        view: card_view,
        on_approve,
        on_edit,
    } = card;
    let (header, why, actions) = match card_view {
        CardView::Waiting { busy } => {
            let approve = popover::btn_primary(theme, "Approve")
                .id(SharedString::from(format!("{id}-approve")))
                .debug_selector(|| "plan-approve".into())
                .role(gpui::Role::Button)
                .aria_label("Approve")
                .aria_disabled(busy)
                .when(busy, |el| el.opacity(0.45))
                .when(!busy, |el| {
                    el.on_click(move |_, window, cx| {
                        cx.stop_propagation();
                        on_approve(window, cx);
                    })
                });
            let edit = popover::btn_ghost(theme, "Edit plan", format!("{id}-edit"))
                .id(SharedString::from(format!("{id}-edit")))
                .debug_selector(|| "plan-edit".into())
                .role(gpui::Role::Button)
                .aria_label("Edit plan")
                .aria_disabled(busy)
                .when(busy, |el| el.opacity(0.45))
                .when(!busy, |el| {
                    el.on_click(move |_, window, cx| {
                        cx.stop_propagation();
                        on_edit(window, cx);
                    })
                });
            (
                div()
                    .child(div().font_weight(gpui::FontWeight::SEMIBOLD).child("Plan"))
                    .child(pill(
                        if busy {
                            "Approving…"
                        } else {
                            "Waiting for your approval"
                        },
                        true,
                        theme,
                    )),
                "Approve runs it in this chat.".to_owned(),
                Some(
                    div()
                        .pt(px(4.0))
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap(px(8.0))
                        .child(approve)
                        .child(edit),
                ),
            )
        }
        CardView::Here => (
            div()
                .child(div().font_weight(gpui::FontWeight::SEMIBOLD).child("Plan"))
                .child(pill("Running here", false, theme)),
            "Runs in this chat.".to_owned(),
            None,
        ),
        CardView::NotApproved => (
            div()
                .child(div().font_weight(gpui::FontWeight::SEMIBOLD).child("Plan"))
                .child(pill("Not approved", false, theme)),
            "The agent was told not to run this plan.".to_owned(),
            None,
        ),
    };
    let rows: Vec<AnyElement> = steps
        .iter()
        .enumerate()
        .map(|(ix, title)| step_row(ix, title.clone(), theme))
        .collect();
    div()
        .py(px(4.0))
        .w_full()
        .child(
            div()
                .id(id.clone())
                .debug_selector(|| "plan-proposal".into())
                .role(gpui::Role::Group)
                .aria_label("Plan")
                .w_full()
                .px(px(14.0))
                .py(px(12.0))
                .flex()
                .flex_col()
                .gap(px(6.0))
                .rounded(px(10.0))
                .border_1()
                .border_color(crate::theme::hairline(0.08))
                .bg(theme.surface)
                .text_size(crate::typography::ui_rems(13.0))
                .child(
                    header
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap(px(8.0))
                        .text_color(theme.text),
                )
                .child(div().flex().flex_col().children(rows))
                .children(actions)
                .child(
                    div()
                        .text_size(crate::typography::ui_rems(12.0))
                        .text_color(theme.text_muted)
                        .child(SharedString::from(why)),
                ),
        )
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn steps_come_from_top_level_list_items() {
        let plan = "## Plan\n\n1. **Audit** the checkout flow\n   - read `cart.ts`\n2. Add Apple Pay to the payment step\n- Release notes and rollout\n";
        assert_eq!(
            proposal_steps(plan),
            [
                "Audit the checkout flow",
                "Add Apple Pay to the payment step",
                "Release notes and rollout"
            ]
        );
        assert_eq!(
            proposal_steps("# Plan\nRefactor totals\n\nShip it"),
            ["Refactor totals", "Ship it"]
        );
    }

    #[test]
    fn an_unanswered_plan_waits_only_while_the_run_lives() {
        let yes = plan_answers("q", true);
        let no = plan_answers("q", false);
        assert_eq!(plan_outcome("q", false, None, true), PlanOutcome::Waiting);
        assert_eq!(plan_outcome("q", false, None, false), PlanOutcome::Declined);
        assert_eq!(
            plan_outcome("q", true, Some(&yes), true),
            PlanOutcome::Approved
        );
        assert_eq!(
            plan_outcome("q", true, Some(&no), true),
            PlanOutcome::Declined
        );
    }
}
