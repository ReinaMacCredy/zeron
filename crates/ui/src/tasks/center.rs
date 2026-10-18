use std::collections::HashMap;
use std::time::Instant;

use chrono::{DateTime, Utc};
use gpui::{
    AnyElement, App, ClickEvent, Context, Entity, EventEmitter, FocusHandle, Focusable,
    IntoElement, KeyDownEvent, Render, SharedString, Subscription, Window, div, prelude::*, px,
    relative,
};
use zeron_proto::{
    ChatIndicator, DelegationStatus, PlanProgress, StepOutcome, StepStatus, StepTurnEnd, TaskPatch,
    TaskStatus, TaskStepPatch,
};

use super::{Rollup, StepRow, chat_brand, column_tone, linked_chat, rollup, step_chat};
use crate::composer::{Composer, ComposerInput, ComposerInputEvent};
use crate::icons::{self, icon};
use crate::motion::{self, MotionSpec, Tween};
use crate::settings::widgets;
use crate::state::AppState;
use crate::theme::Theme;
use crate::transcript::Transcript;
use crate::typography::ui_rems;

const ICON_SLOT: f32 = 16.0;
const STEP_IN: MotionSpec = MotionSpec::new(350, motion::EASE_DRAWER);
const STEP_RISE: f32 = 6.0;
const PROGRESS_FILL: MotionSpec = MotionSpec::new(500, motion::EASE_DRAWER);

pub enum TaskCenterEvent {
    Update {
        task_id: String,
        patch: TaskPatch,
    },
    ApprovePlan(String),
    RetryStep(String),
    UpdateStep {
        step_id: String,
        patch: TaskStepPatch,
    },
    DeleteStep(String),
    CreateStep {
        task_id: String,
        step_id: String,
        title: String,
    },
    OpenChat(String),
    Selected,
    Back,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Selection {
    Lead,
    Step(String),
}

pub struct LiveThread {
    pub chat_id: String,
    pub transcript: Entity<Transcript>,
    pub composer: Entity<Composer>,
}

struct Rename {
    step_id: String,
    input: Entity<ComposerInput>,
    _subscription: Subscription,
}

struct Thread {
    title: String,
    delegation: Option<(DelegationStatus, Option<String>)>,
    preview: Option<String>,
    since: Option<DateTime<Utc>>,
}

enum StepState {
    Working(Option<DateTime<Utc>>),
    Asked,
    Failed(&'static str, Option<String>),
    NeedsYou,
    Done,
    Planned,
    Skipped,
}

pub struct TaskCenter {
    state: Entity<AppState>,
    task_id: String,
    selected: Selection,
    rename: Option<Rename>,
    add: Entity<ComposerInput>,
    live: Option<LiveThread>,
    fill: Option<Tween>,
    arrivals: Option<HashMap<String, Tween>>,
    focus: FocusHandle,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<TaskCenterEvent> for TaskCenter {}

impl Focusable for TaskCenter {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl TaskCenter {
    pub fn new(state: Entity<AppState>, task_id: String, cx: &mut Context<Self>) -> Self {
        let add = cx.new(|cx| {
            ComposerInput::new("Add a step", cx)
                .with_single_line()
                .with_max_chars(zeron_doc::TASK_TITLE_MAX_CHARS)
        });
        let add_events = cx.subscribe(&add, |center, _, event: &ComposerInputEvent, cx| {
            if matches!(event, ComposerInputEvent::Submitted) {
                center.add_step(cx);
            }
        });
        let state_changes = cx.observe(&state, |_, _, cx| cx.notify());
        Self {
            state,
            task_id,
            selected: Selection::Lead,
            rename: None,
            add,
            live: None,
            fill: None,
            arrivals: None,
            focus: cx.focus_handle(),
            _subscriptions: vec![add_events, state_changes],
        }
    }

    pub fn task_id(&self) -> &str {
        &self.task_id
    }

    pub fn select(&mut self, selection: Selection, cx: &mut Context<Self>) {
        if self.selected != selection {
            self.selected = selection;
            cx.emit(TaskCenterEvent::Selected);
            cx.notify();
        }
    }

    pub fn live_target(&self, cx: &App) -> Option<String> {
        let state = self.state.read(cx);
        let task = state
            .tasks
            .iter()
            .find(|task| task.id == self.task_id && !task.deleted)?;
        if let Selection::Step(id) = &self.selected
            && let Some(chat) = state
                .task_steps
                .iter()
                .find(|step| &step.id == id && !step.deleted)
                .and_then(|step| step_chat(&state.chats, step))
        {
            return Some(chat.id.clone());
        }
        linked_chat(&state.chats, task).map(|chat| chat.id.clone())
    }

    pub fn live_chat_id(&self) -> Option<&str> {
        self.live.as_ref().map(|live| live.chat_id.as_str())
    }

    pub fn set_live(&mut self, live: Option<LiveThread>, cx: &mut Context<Self>) {
        self.live = live;
        cx.notify();
    }

    fn add_step(&mut self, cx: &mut Context<Self>) {
        let title = self.add.read(cx).text().trim().to_string();
        if title.is_empty() {
            return;
        }
        self.add.update(cx, |input, cx| input.set_text("", cx));
        cx.emit(TaskCenterEvent::CreateStep {
            task_id: self.task_id.clone(),
            step_id: uuid::Uuid::new_v4().to_string(),
            title,
        });
    }

    fn start_rename(
        &mut self,
        step_id: String,
        title: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let input = cx.new(|cx| {
            let mut input = ComposerInput::new("Step title", cx)
                .with_single_line()
                .with_max_chars(zeron_doc::TASK_TITLE_MAX_CHARS);
            input.set_text(title, cx);
            input
        });
        let subscription = cx.subscribe(&input, |center, _, event: &ComposerInputEvent, cx| {
            if matches!(event, ComposerInputEvent::Submitted) {
                center.finish_rename(cx);
            }
        });
        window.focus(&input.focus_handle(cx), cx);
        self.rename = Some(Rename {
            step_id,
            input,
            _subscription: subscription,
        });
        cx.notify();
    }

    fn finish_rename(&mut self, cx: &mut Context<Self>) {
        let Some(rename) = self.rename.take() else {
            return;
        };
        let title = rename.input.read(cx).text().trim().to_string();
        let unchanged = self
            .state
            .read(cx)
            .task_steps
            .iter()
            .any(|step| step.id == rename.step_id && step.title == title);
        if !title.is_empty() && !unchanged {
            cx.emit(TaskCenterEvent::UpdateStep {
                step_id: rename.step_id,
                patch: TaskStepPatch {
                    title: Some(title),
                    ..Default::default()
                },
            });
        }
        cx.notify();
    }

    fn set_outcome(&mut self, step_id: &str, outcome: Option<StepOutcome>, cx: &mut Context<Self>) {
        cx.emit(TaskCenterEvent::UpdateStep {
            step_id: step_id.to_string(),
            patch: TaskStepPatch {
                outcome: Some(outcome),
                ..Default::default()
            },
        });
    }

    fn threads(&self, plan: &Rollup, cx: &App) -> Vec<Option<Thread>> {
        let state = self.state.read(cx);
        plan.steps
            .iter()
            .map(|row| {
                let chat = step_chat(&state.chats, &row.step)?;
                let delegation = chat.delegation.as_ref();
                Some(Thread {
                    title: crate::transcript::single_line(&crate::home::chat_title(chat)),
                    delegation: delegation.map(|d| (d.status, d.summary.clone())),
                    preview: chat.last_message_preview.clone(),
                    since: state
                        .session_for(&chat.id)
                        .and_then(|session| session.started_at)
                        .or(delegation.map(|d| d.started_at)),
                })
            })
            .collect()
    }

    fn step_state(row: &StepRow, thread: Option<&Thread>) -> StepState {
        match row.status {
            StepStatus::Done => StepState::Done,
            StepStatus::Skipped => StepState::Skipped,
            StepStatus::Planned => StepState::Planned,
            StepStatus::Working => StepState::Working(thread.and_then(|t| t.since)),
            StepStatus::Failed => {
                StepState::Failed("Failed", thread.and_then(|t| t.preview.clone()))
            }
            StepStatus::NeedsYou => {
                let delegation = thread.and_then(|t| t.delegation.as_ref());
                let stopped = delegation.is_none()
                    && row.step.turn.as_ref().map(|turn| turn.ended) == Some(StepTurnEnd::Stopped);
                if row.indicator == Some(ChatIndicator::AwaitingInput) {
                    StepState::Asked
                } else if let Some((status, summary)) =
                    delegation.filter(|(status, _)| retryable(*status))
                {
                    let label = match status {
                        DelegationStatus::Cancelled => "Stopped",
                        DelegationStatus::Interrupted => "Interrupted",
                        _ => "Failed",
                    };
                    StepState::Failed(label, summary.clone())
                } else if row.indicator == Some(ChatIndicator::Errored) || stopped {
                    let label = if stopped { "Stopped" } else { "Failed" };
                    StepState::Failed(label, thread.and_then(|t| t.preview.clone()))
                } else {
                    StepState::NeedsYou
                }
            }
        }
    }

    fn render_missing(&self, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
        div()
            .id("task-center-missing")
            .debug_selector(|| "task-center-missing".into())
            .size_full()
            .pt(px(Theme::TITLEBAR_HEIGHT))
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap(px(12.0))
            .text_size(ui_rems(13.0))
            .text_color(theme.text_muted)
            .child("This task is gone. It was deleted, or it has not synced yet.")
            .child(
                widgets::text_action(theme, widgets::ActionTone::Filled, "Back to Tasks")
                    .id("task-center-missing-back")
                    .debug_selector(|| "task-center-missing-back".into())
                    .role(gpui::Role::Button)
                    .rounded_full()
                    .on_click(
                        cx.listener(|_, _: &ClickEvent, _, cx| cx.emit(TaskCenterEvent::Back)),
                    ),
            )
            .into_any_element()
    }

    fn render_header(
        &self,
        task: &zeron_proto::Task,
        plan: &Rollup,
        fill: f32,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let (label, tone) = plan.label(task);
        let (_, _, tone) = column_tone(tone, theme);
        let (_, _, complete) = column_tone(TaskStatus::Done, theme);
        let done = task.completed_at.is_some();
        let PlanProgress {
            done: finished,
            total,
        } = plan.progress;
        let task_id = task.id.clone();
        let toggle = widgets::text_action(
            theme,
            widgets::ActionTone::Filled,
            if done { "Reopen" } else { "Mark as done" },
        )
        .id("task-center-toggle-done")
        .debug_selector(|| "task-center-toggle-done".into())
        .role(gpui::Role::Button)
        .rounded_full()
        .on_click(cx.listener(move |_, _: &ClickEvent, _, cx| {
            cx.emit(TaskCenterEvent::Update {
                task_id: task_id.clone(),
                patch: TaskPatch {
                    completed: Some(!done),
                    ..Default::default()
                },
            });
        }));
        div()
            .id("task-center-header")
            .debug_selector(|| "task-center-header".into())
            .flex_none()
            .px(px(20.0))
            .pt(px(8.0))
            .pb(px(12.0))
            .border_b_1()
            .border_color(theme.border)
            .flex()
            .flex_col()
            .gap(px(10.0))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(10.0))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_size(ui_rems(18.0))
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .text_color(theme.text)
                            .child(SharedString::from(task.title.clone())),
                    )
                    .child(
                        div()
                            .debug_selector(|| "task-center-status".into())
                            .flex_none()
                            .flex()
                            .items_center()
                            .gap(px(6.0))
                            .text_size(ui_rems(12.0))
                            .text_color(theme.text_muted)
                            .child(div().size(px(7.0)).rounded_full().bg(tone))
                            .child(label),
                    )
                    .child(toggle),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(10.0))
                    .child(
                        div()
                            .debug_selector(|| "task-center-progress".into())
                            .flex_1()
                            .h(px(4.0))
                            .rounded(px(2.0))
                            .bg(theme.wash(0.08))
                            .overflow_hidden()
                            .child(div().h_full().w(relative(fill)).bg(complete)),
                    )
                    .child(
                        div()
                            .flex_none()
                            .text_size(ui_rems(12.0))
                            .text_color(theme.text_muted)
                            .child(format!("{finished}/{total}")),
                    ),
            )
            .into_any_element()
    }

    fn render_send_error(
        &self,
        task: &zeron_proto::Task,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let error = task.send_error.clone()?;
        let task_id = task.id.clone();
        Some(
            div()
                .flex_none()
                .px(px(20.0))
                .child(
                    widgets::warning_strip(theme, format!("Not sent to the lead: {error}"))
                        .debug_selector(|| "task-center-send-error".into())
                        .child(div().flex_1())
                        .child(
                            widgets::text_action(theme, widgets::ActionTone::Quiet, "Dismiss")
                                .id("task-center-dismiss-send-error")
                                .debug_selector(|| "task-center-dismiss-send-error".into())
                                .role(gpui::Role::Button)
                                .flex_none()
                                .min_h(px(24.0))
                                .rounded_full()
                                .on_click(cx.listener(move |_, _: &ClickEvent, _, cx| {
                                    cx.emit(TaskCenterEvent::Update {
                                        task_id: task_id.clone(),
                                        patch: TaskPatch {
                                            send_error: Some(None),
                                            ..Default::default()
                                        },
                                    });
                                })),
                        ),
                )
                .into_any_element(),
        )
    }

    fn lead_line(task: &zeron_proto::Task, plan: &Rollup) -> String {
        let running = plan.count(StepStatus::Working);
        if task.completed_at.is_some() {
            "Done".into()
        } else if plan.lead.is_none() {
            "Not started".into()
        } else if plan.lead == Some(ChatIndicator::Working) && plan.steps.is_empty() {
            "Planning".into()
        } else if running > 0 {
            format!("Watching {running} {}", plural(running, "step", "steps"))
        } else if plan.lead == Some(ChatIndicator::Working) {
            "Working".into()
        } else {
            "Waiting on you".into()
        }
    }

    fn render_lead(
        &self,
        task: &zeron_proto::Task,
        plan: &Rollup,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let (brand, model) = {
            let state = self.state.read(cx);
            let chat = linked_chat(&state.chats, task);
            let model = chat
                .and_then(|chat| chat.config.as_ref())
                .and_then(|config| {
                    let model = config.model.as_deref()?;
                    Some(crate::pickers::model_display_label(
                        config.harness,
                        model,
                        cx,
                    ))
                });
            (chat.map(chat_brand), model)
        };
        let (mark, tint) = brand.unwrap_or((icons::BOT, None));
        let selected = self.selected == Selection::Lead;
        div()
            .id("task-center-lead")
            .debug_selector(|| "task-center-lead".into())
            .role(gpui::Role::Button)
            .aria_selected(selected)
            .flex()
            .items_center()
            .gap(px(10.0))
            .px(px(10.0))
            .py(px(9.0))
            .rounded(px(12.0))
            .cursor_pointer()
            .bg(theme.wash(if selected { 0.10 } else { 0.05 }))
            .hover(|s| s.bg(theme.wash(0.10)))
            .child(
                icon(mark)
                    .size(px(14.0))
                    .flex_none()
                    .text_color(tint.unwrap_or(theme.text_muted)),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .child(
                        div()
                            .truncate()
                            .text_size(ui_rems(13.0))
                            .text_color(theme.text)
                            .child(match model {
                                Some(model) => format!("Lead · {model}"),
                                None => "Lead".into(),
                            }),
                    )
                    .child(
                        div()
                            .text_size(ui_rems(12.0))
                            .text_color(theme.text_muted)
                            .child(Self::lead_line(task, plan)),
                    ),
            )
            .on_click(
                cx.listener(|center, _: &ClickEvent, _, cx| center.select(Selection::Lead, cx)),
            )
            .into_any_element()
    }

    fn render_banner(
        &self,
        task: &zeron_proto::Task,
        plan: &Rollup,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        if !task.plan_pending || task.completed_at.is_some() {
            return None;
        }
        let planning = plan.steps.is_empty() || plan.lead == Some(ChatIndicator::Working);
        let banner = div()
            .id("task-center-plan-banner")
            .debug_selector(|| "task-center-plan-banner".into())
            .flex()
            .items_center()
            .gap(px(10.0))
            .px(px(12.0))
            .py(px(10.0))
            .rounded(px(12.0))
            .bg(theme.wash(0.06))
            .text_size(ui_rems(13.0))
            .text_color(theme.text);
        if planning {
            return Some(
                banner
                    .child(
                        crate::loaders::mini_glyph_spinner(
                            "task-center-planning",
                            2.0,
                            theme.glyph,
                            cx.entity_id(),
                            cx,
                        )
                        .into_any_element(),
                    )
                    .child("The lead is planning")
                    .into_any_element(),
            );
        }
        let total = plan.progress.total;
        let task_id = task.id.clone();
        Some(
            banner
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .child("Plan ready")
                        .child(
                            div()
                                .text_size(ui_rems(12.0))
                                .text_color(theme.text_muted)
                                .child(format!(
                                    "{total} {} · the lead waits for you",
                                    plural(total, "step", "steps")
                                )),
                        ),
                )
                .child(
                    widgets::text_action(theme, widgets::ActionTone::Solid, "Approve and run")
                        .id("task-center-approve")
                        .debug_selector(|| "task-center-approve".into())
                        .role(gpui::Role::Button)
                        .flex_none()
                        .rounded_full()
                        .on_click(cx.listener(move |_, _: &ClickEvent, _, cx| {
                            cx.emit(TaskCenterEvent::ApprovePlan(task_id.clone()))
                        })),
                )
                .into_any_element(),
        )
    }

    fn step_icon(status: StepStatus, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let slot = div()
            .flex_none()
            .size(px(ICON_SLOT))
            .flex()
            .items_center()
            .justify_center();
        match status {
            StepStatus::Working => slot
                .child(crate::loaders::mini_glyph_spinner(
                    "task-center-step-working",
                    2.0,
                    theme.glyph,
                    cx.entity_id(),
                    cx,
                ))
                .into_any_element(),
            StepStatus::NeedsYou => slot
                .child(
                    icon(icons::LUCIDE_CIRCLE_ALERT)
                        .size(px(15.0))
                        .text_color(theme.warning),
                )
                .into_any_element(),
            StepStatus::Failed => slot
                .child(
                    icon(icons::LUCIDE_CIRCLE_ALERT)
                        .size(px(15.0))
                        .text_color(theme.danger),
                )
                .into_any_element(),
            StepStatus::Done => slot
                .child(
                    div()
                        .size(px(15.0))
                        .rounded_full()
                        .bg(column_tone(TaskStatus::Done, theme).2)
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(icon(icons::CHECK).size(px(10.0)).text_color(gpui::white())),
                )
                .into_any_element(),
            StepStatus::Planned | StepStatus::Skipped => {
                let ring = if status == StepStatus::Skipped {
                    theme.text_faint.opacity(0.4)
                } else {
                    theme.text_faint
                };
                slot.child(
                    div()
                        .size(px(12.0))
                        .rounded_full()
                        .border_1()
                        .border_color(ring),
                )
                .into_any_element()
            }
        }
    }

    fn step_subline(row: &StepRow, thread: Option<&Thread>, theme: &Theme) -> Vec<AnyElement> {
        let mut parts = Vec::new();
        if let (Some((mark, tint)), Some(thread)) = (row.brand, thread) {
            parts.push(
                icon(mark)
                    .size(px(12.0))
                    .flex_none()
                    .text_color(tint.unwrap_or(theme.text_muted))
                    .into_any_element(),
            );
            if thread.title != crate::transcript::single_line(&row.step.title) {
                parts.push(
                    div()
                        .min_w_0()
                        .truncate()
                        .child(thread.title.clone())
                        .into_any_element(),
                );
                parts.push(div().flex_none().child("·").into_any_element());
            }
        }
        let (_, _, working) = column_tone(TaskStatus::Working, theme);
        let (_, _, asked) = column_tone(TaskStatus::NeedsYou, theme);
        let (_, _, complete) = column_tone(TaskStatus::Done, theme);
        let state = match Self::step_state(row, thread) {
            StepState::Working(since) => div()
                .flex()
                .items_center()
                .gap(px(4.0))
                .text_color(working)
                .child("Working")
                .children(since.map(|since| {
                    crate::loaders::elapsed_label(
                        format!("task-center-step-elapsed-{}", row.step.id),
                        since,
                        ui_rems(11.5),
                        working,
                    )
                })),
            StepState::Asked => div().text_color(asked).child("Asked you a question"),
            StepState::Failed(label, summary) => div()
                .flex_1()
                .min_w_0()
                .truncate()
                .text_color(if row.status == StepStatus::Failed {
                    theme.danger
                } else {
                    theme.warning
                })
                .child(match summary {
                    Some(summary) => {
                        format!("{label} · {}", crate::transcript::single_line(&summary))
                    }
                    None => label.into(),
                }),
            StepState::NeedsYou => div().text_color(theme.warning).child("Needs you"),
            StepState::Done => div().text_color(complete).child("Done"),
            StepState::Planned => div().child("Planned"),
            StepState::Skipped => div().child("Skipped"),
        };
        parts.push(state.into_any_element());
        parts
    }

    fn hover_action(
        theme: &Theme,
        id: String,
        label: &'static str,
        step: usize,
        group: SharedString,
    ) -> gpui::Stateful<gpui::Div> {
        let selector = id.clone();
        div()
            .id(SharedString::from(id))
            .debug_selector(move || selector.clone())
            .role(gpui::Role::Button)
            .aria_label(format!("{label} step {step}"))
            .flex_none()
            .px(px(8.0))
            .py(px(2.0))
            .rounded_full()
            .cursor_pointer()
            .text_size(ui_rems(11.5))
            .text_color(theme.text_muted)
            .invisible()
            .group_hover(group, |s| s.visible())
            .hover(|s| s.bg(theme.wash(0.10)).text_color(theme.text))
            .child(label)
    }

    #[allow(clippy::too_many_arguments)]
    fn render_step(
        &self,
        index: usize,
        last: bool,
        row: &StepRow,
        thread: Option<&Thread>,
        editable: bool,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let step_id = row.step.id.clone();
        let group = SharedString::from(format!("task-center-step-{step_id}"));
        let selected = self.selected == Selection::Step(step_id.clone());
        let renaming = self
            .rename
            .as_ref()
            .filter(|rename| rename.step_id == step_id)
            .map(|rename| rename.input.clone());
        let retry = row.step.outcome.is_none()
            && matches!(Self::step_state(row, thread), StepState::Failed(..));
        let skip = row.step.outcome.is_none()
            && matches!(
                row.status,
                StepStatus::Planned | StepStatus::NeedsYou | StepStatus::Failed
            );
        let reopen = row.step.outcome.is_some();
        let remove = editable && thread.is_none();
        let title_color = if row.status == StepStatus::Skipped {
            theme.text_muted
        } else {
            theme.text
        };
        let title: AnyElement = match renaming {
            Some(input) => div()
                .debug_selector(|| "task-center-rename".into())
                .px(px(6.0))
                .py(px(2.0))
                .rounded(px(6.0))
                .bg(theme.wash(0.08))
                .text_size(ui_rems(13.0))
                .on_key_down(cx.listener(|center, event: &KeyDownEvent, _, cx| {
                    if event.keystroke.key == "escape" {
                        cx.stop_propagation();
                        center.rename = None;
                        cx.notify();
                    }
                }))
                .child(input)
                .into_any_element(),
            None => div()
                .truncate()
                .text_size(ui_rems(13.0))
                .text_color(title_color)
                .when(row.status == StepStatus::Skipped, |el| el.line_through())
                .child(format!("{}. {}", index + 1, row.step.title))
                .into_any_element(),
        };
        let mut actions: Vec<AnyElement> = Vec::new();
        if editable {
            let (id, title) = (step_id.clone(), row.step.title.clone());
            actions.push(
                Self::hover_action(
                    theme,
                    format!("task-center-rename-{step_id}"),
                    "Rename",
                    index + 1,
                    group.clone(),
                )
                .on_click(cx.listener(move |center, _: &ClickEvent, window, cx| {
                    cx.stop_propagation();
                    center.start_rename(id.clone(), title.clone(), window, cx);
                }))
                .into_any_element(),
            );
        }
        if remove {
            let id = step_id.clone();
            actions.push(
                Self::hover_action(
                    theme,
                    format!("task-center-remove-{step_id}"),
                    "Remove",
                    index + 1,
                    group.clone(),
                )
                .on_click(cx.listener(move |_, _: &ClickEvent, _, cx| {
                    cx.stop_propagation();
                    cx.emit(TaskCenterEvent::DeleteStep(id.clone()));
                }))
                .into_any_element(),
            );
        }
        if skip {
            let id = step_id.clone();
            actions.push(
                Self::hover_action(
                    theme,
                    format!("task-center-skip-{step_id}"),
                    "Skip",
                    index + 1,
                    group.clone(),
                )
                .on_click(cx.listener(move |center, _: &ClickEvent, _, cx| {
                    cx.stop_propagation();
                    center.set_outcome(&id, Some(StepOutcome::Skipped), cx);
                }))
                .into_any_element(),
            );
        }
        if reopen {
            let id = step_id.clone();
            actions.push(
                Self::hover_action(
                    theme,
                    format!("task-center-reopen-{step_id}"),
                    "Reopen",
                    index + 1,
                    group.clone(),
                )
                .on_click(cx.listener(move |center, _: &ClickEvent, _, cx| {
                    cx.stop_propagation();
                    center.set_outcome(&id, None, cx);
                }))
                .into_any_element(),
            );
        }
        let retry_button = retry.then(|| {
            let id = step_id.clone();
            let selector = format!("task-center-retry-{step_id}");
            widgets::text_action(theme, widgets::ActionTone::Filled, "Retry")
                .id(SharedString::from(selector.clone()))
                .debug_selector(move || selector.clone())
                .role(gpui::Role::Button)
                .aria_label(format!("Retry step {}", index + 1))
                .flex_none()
                .min_h(px(24.0))
                .rounded_full()
                .on_click(cx.listener(move |_, _: &ClickEvent, _, cx| {
                    cx.stop_propagation();
                    cx.emit(TaskCenterEvent::RetryStep(id.clone()));
                }))
        });
        let connector = (!last).then(|| {
            div()
                .absolute()
                .left(px(8.0 + ICON_SLOT / 2.0))
                .top(px(8.0 + ICON_SLOT + 4.0))
                .bottom(px(-8.0))
                .w(px(1.0))
                .bg(theme.border)
        });
        let select_id = step_id.clone();
        div()
            .id(SharedString::from(format!("task-center-step-{step_id}")))
            .debug_selector(move || format!("task-center-step-{step_id}"))
            .group(group)
            .role(gpui::Role::ListItem)
            .aria_label(format!("Step {}: {}", index + 1, row.step.title))
            .aria_selected(selected)
            .relative()
            .flex()
            .items_start()
            .gap(px(10.0))
            .px(px(8.0))
            .py(px(8.0))
            .rounded(px(10.0))
            .cursor_pointer()
            .when(selected, |el| el.bg(theme.wash(0.08)))
            .when(!selected, |el| el.hover(|s| s.bg(theme.wash(0.05))))
            .children(connector)
            .child(Self::step_icon(row.status, theme, cx))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap(px(2.0))
                    .child(title)
                    .child(
                        div()
                            .min_w_0()
                            .flex()
                            .items_center()
                            .gap(px(5.0))
                            .text_size(ui_rems(11.5))
                            .text_color(theme.text_muted)
                            .children(Self::step_subline(row, thread, theme)),
                    ),
            )
            .child(
                div()
                    .flex_none()
                    .flex()
                    .items_center()
                    .gap(px(2.0))
                    .children(actions)
                    .children(retry_button),
            )
            .on_click(cx.listener(move |center, _: &ClickEvent, _, cx| {
                center.select(Selection::Step(select_id.clone()), cx)
            }))
            .into_any_element()
    }

    fn render_add_step(&self, theme: &Theme) -> AnyElement {
        div()
            .id("task-center-add-step")
            .debug_selector(|| "task-center-add-step".into())
            .flex()
            .items_center()
            .gap(px(10.0))
            .px(px(8.0))
            .py(px(8.0))
            .rounded(px(10.0))
            .child(
                div()
                    .flex_none()
                    .size(px(ICON_SLOT))
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(
                        icon(icons::PLUS)
                            .size(px(13.0))
                            .text_color(theme.text_faint),
                    ),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .text_size(ui_rems(13.0))
                    .child(self.add.clone()),
            )
            .into_any_element()
    }

    fn render_live(
        &self,
        task: &zeron_proto::Task,
        plan: &Rollup,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let step = match &self.selected {
            Selection::Step(id) => plan
                .steps
                .iter()
                .position(|row| &row.step.id == id)
                .map(|ix| (ix, &plan.steps[ix])),
            Selection::Lead => None,
        };
        let total = plan.steps.len();
        let live_chat = self.live.as_ref().and_then(|live| {
            let state = self.state.read(cx);
            let chat = state.chats.iter().find(|chat| chat.id == live.chat_id)?;
            Some((
                chat.id.clone(),
                crate::transcript::single_line(&crate::home::chat_title(chat)),
                chat_brand(chat),
            ))
        });
        let step_has_thread = step.is_some_and(|(_, row)| {
            live_chat
                .as_ref()
                .is_some_and(|(id, _, _)| row.step.chat_id.as_deref() == Some(id.as_str()))
        });
        let (title, sub) = match step {
            Some((ix, row)) if !step_has_thread => (
                row.step.title.clone(),
                format!(
                    "Step {} of {total} has not started · messages go to the lead",
                    ix + 1
                ),
            ),
            Some((ix, _)) => (
                live_chat
                    .as_ref()
                    .map(|(_, title, _)| title.clone())
                    .unwrap_or_default(),
                format!("Step {} of {total}", ix + 1),
            ),
            None => (
                live_chat
                    .as_ref()
                    .map(|(_, title, _)| title.clone())
                    .unwrap_or_else(|| task.title.clone()),
                "Lead".into(),
            ),
        };
        let (mark, tint) = live_chat
            .as_ref()
            .map(|(_, _, brand)| *brand)
            .unwrap_or((icons::BOT, None));
        let open = live_chat.as_ref().map(|(id, _, _)| {
            let id = id.clone();
            widgets::text_action(theme, widgets::ActionTone::Filled, "Open thread")
                .id("task-center-open-thread")
                .debug_selector(|| "task-center-open-thread".into())
                .role(gpui::Role::Button)
                .flex_none()
                .rounded_full()
                .on_click(cx.listener(move |_, _: &ClickEvent, _, cx| {
                    cx.emit(TaskCenterEvent::OpenChat(id.clone()))
                }))
        });
        let header = div()
            .id("task-center-live-header")
            .debug_selector(|| "task-center-live-header".into())
            .flex_none()
            .px(px(16.0))
            .py(px(10.0))
            .border_b_1()
            .border_color(theme.border)
            .flex()
            .items_center()
            .gap(px(8.0))
            .child(
                icon(mark)
                    .size(px(14.0))
                    .flex_none()
                    .text_color(tint.unwrap_or(theme.text_muted)),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .child(
                        div()
                            .truncate()
                            .text_size(ui_rems(13.0))
                            .text_color(theme.text)
                            .child(title),
                    )
                    .child(
                        div()
                            .truncate()
                            .text_size(ui_rems(11.5))
                            .text_color(theme.text_muted)
                            .child(sub),
                    ),
            )
            .children(open);
        let body = match &self.live {
            Some(live) => div()
                .flex_1()
                .min_h_0()
                .flex()
                .flex_col()
                .child(div().flex_1().min_h_0().child(live.transcript.clone()))
                .child(div().flex_none().child(live.composer.clone()))
                .into_any_element(),
            None => div()
                .flex_1()
                .p(px(16.0))
                .text_size(ui_rems(12.5))
                .text_color(theme.text_muted)
                .child("The lead has not started yet.")
                .into_any_element(),
        };
        div()
            .id("task-center-live")
            .debug_selector(|| "task-center-live".into())
            .flex_1()
            .min_w_0()
            .h_full()
            .flex()
            .flex_col()
            .border_l_1()
            .border_color(theme.border)
            .child(header)
            .child(body)
            .into_any_element()
    }
}

fn retryable(status: DelegationStatus) -> bool {
    matches!(
        status,
        DelegationStatus::Failed | DelegationStatus::Interrupted | DelegationStatus::Cancelled
    )
}

fn plural(n: usize, one: &'static str, many: &'static str) -> &'static str {
    if n == 1 { one } else { many }
}

impl TaskCenter {
    fn fill_at(&mut self, progress: PlanProgress, reduced: bool, now: Instant) -> (f32, bool) {
        let PlanProgress { done, total } = progress;
        let fraction = if total == 0 {
            0.0
        } else {
            done as f32 / total as f32
        };
        let fill = self.fill.get_or_insert_with(|| Tween::at(fraction, now));
        if fill.target() != fraction {
            fill.retarget(fraction, PROGRESS_FILL, reduced, now);
        }
        (fill.value_at(now), !fill.settled_at(now))
    }

    /// Steps on screen when the page first draws are already there; only later ones rise in.
    fn arrivals_at(&mut self, steps: &[StepRow], reduced: bool, now: Instant) -> Vec<Tween> {
        let first = self.arrivals.is_none();
        let previous = self.arrivals.take().unwrap_or_default();
        let arrivals: HashMap<String, Tween> = steps
            .iter()
            .map(|row| {
                let tween = previous.get(&row.step.id).copied().unwrap_or_else(|| {
                    let mut tween = Tween::at(if first { 1.0 } else { 0.0 }, now);
                    tween.retarget(1.0, STEP_IN, reduced, now);
                    tween
                });
                (row.step.id.clone(), tween)
            })
            .collect();
        let tweens = steps.iter().map(|row| arrivals[&row.step.id]).collect();
        self.arrivals = Some(arrivals);
        tweens
    }
}

impl Render for TaskCenter {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::of(cx).clone();
        let found = {
            let state = self.state.read(cx);
            state
                .tasks
                .iter()
                .find(|task| task.id == self.task_id && !task.deleted)
                .map(|task| (task.clone(), rollup(state, task, Utc::now())))
        };
        let Some((task, plan)) = found else {
            return self.render_missing(&theme, cx);
        };
        let threads = self.threads(&plan, cx);
        let reduced = motion::reduced_motion(cx);
        let now = Instant::now();
        let (fill, filling) = self.fill_at(plan.progress, reduced, now);
        let arrivals = self.arrivals_at(&plan.steps, reduced, now);
        if filling || arrivals.iter().any(|tween| !tween.settled_at(now)) {
            window.request_animation_frame();
        }
        let editable = task.plan_pending && task.completed_at.is_none();
        let last = plan.steps.len().saturating_sub(1);
        let steps: Vec<AnyElement> = plan
            .steps
            .iter()
            .zip(&threads)
            .zip(&arrivals)
            .enumerate()
            .map(|(ix, ((row, thread), arrival))| {
                let step = self.render_step(
                    ix,
                    ix == last && !editable,
                    row,
                    thread.as_ref(),
                    editable,
                    &theme,
                    cx,
                );
                let shown = arrival.value_at(now);
                div()
                    .relative()
                    .top(px(STEP_RISE * (1.0 - shown)))
                    .opacity(shown)
                    .child(step)
                    .into_any_element()
            })
            .collect();
        let left = div()
            .id("task-center-plan")
            .debug_selector(|| "task-center-plan".into())
            .flex_1()
            .min_w(px(280.0))
            .h_full()
            .overflow_y_scroll()
            .px(px(14.0))
            .py(px(12.0))
            .flex()
            .flex_col()
            .gap(px(10.0))
            .child(self.render_lead(&task, &plan, &theme, cx))
            .children(self.render_banner(&task, &plan, &theme, cx))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .children(steps)
                    .when(editable, |el| el.child(self.render_add_step(&theme))),
            );
        div()
            .id("task-center")
            .debug_selector(|| "task-center".into())
            .track_focus(&self.focus)
            .size_full()
            .pt(px(Theme::TITLEBAR_HEIGHT))
            .flex()
            .flex_col()
            .child(self.render_header(&task, &plan, fill, &theme, cx))
            .children(self.render_send_error(&task, &theme, cx))
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .flex_row()
                    .child(left)
                    .child(self.render_live(&task, &plan, &theme, cx)),
            )
            .into_any_element()
    }
}
