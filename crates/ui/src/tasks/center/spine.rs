use super::*;
use crate::tasks::tree::{self, Dot, Link, NodeIn, Status};

const SPINE: f32 = 64.0;
const ROW: f32 = 52.0;
const ROW_GAP: f32 = 12.0;
const COLUMN: f32 = 340.0;
const LEAD_WIDTH: f32 = 420.0;
const BODY_GAP: f32 = 28.0;
const GHOST_WIDTH: f32 = 260.0;
const CARD_RADIUS: f32 = 10.0;
const ANCHOR_Y: f32 = 21.0;
pub(super) const CANVAS_PAD: f32 = 20.0;
pub(super) const DRAWER_WIDTH: f32 = 440.0;
const DRAWER: MotionSpec = MotionSpec::new(420, tree::EASE);
const LEAD: &str = "lead";
const GHOST: &str = "add";
const CARD_HEIGHT: f32 = 96.0;
const LEAD_HEIGHT: f32 = 150.0;

struct BarRow {
    id: String,
    status: Status,
    name: String,
    title: String,
    text: Option<String>,
}

pub(super) struct Pop {
    pub kind: Status,
    pub tween: Tween,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Side {
    Left,
    Right,
}

fn span(height: f32) -> usize {
    ((height + ROW_GAP) / (ROW + ROW_GAP)).ceil().max(2.0) as usize
}

fn spine_rows(spans: &[usize]) -> (Vec<(Side, usize)>, usize) {
    let (mut row, mut end) = (0, 0);
    let rows = spans
        .iter()
        .enumerate()
        .map(|(i, span)| {
            let side = if i % 2 == 0 { Side::Left } else { Side::Right };
            let at = (side, row);
            end = end.max(row + span);
            row += span - 1;
            at
        })
        .collect();
    (rows, end)
}

struct Placed {
    id: String,
    x: f32,
    y: f32,
    width: f32,
    side: Option<Side>,
    frame: tree::Frame,
}

impl TaskCenter {
    pub(super) fn sync_drawer(&mut self, open: bool, cx: &App) {
        if self.drawer_open != open {
            self.drawer_open = open;
            let now = Instant::now();
            self.drawer.retarget(
                f32::from(u8::from(open)),
                DRAWER,
                motion::reduced_motion(cx),
                now,
            );
        }
    }

    pub(super) fn open_drawer(&mut self, cx: &mut Context<Self>) {
        self.sync_drawer(true, cx);
        cx.notify();
    }

    pub(super) fn close_drawer(&mut self, cx: &mut Context<Self>) {
        self.sync_drawer(false, cx);
        cx.notify();
    }

    fn toggle_overview(&mut self, cx: &mut Context<Self>) {
        self.overview_open = !self.overview_open;
        let now = Instant::now();
        self.overview_turn.retarget(
            f32::from(u8::from(!self.overview_open)),
            tree::CHEVRON,
            motion::reduced_motion(cx),
            now,
        );
        cx.notify();
    }

    fn toggle_steps(&mut self, cx: &mut Context<Self>) {
        self.folded = !self.folded;
        let now = Instant::now();
        self.fold_turn.retarget(
            f32::from(u8::from(self.folded)),
            tree::CHEVRON,
            motion::reduced_motion(cx),
            now,
        );
        cx.notify();
    }

    fn toggle_pop(&mut self, kind: Status, cx: &mut Context<Self>) {
        let now = Instant::now();
        self.pop = match &self.pop {
            Some(pop) if pop.kind == kind => None,
            _ => {
                let mut tween = Tween::at(0.0, now);
                tween.retarget(1.0, tree::POP, motion::reduced_motion(cx), now);
                Some(Pop { kind, tween })
            }
        };
        cx.notify();
    }

    pub(super) fn close_pop(&mut self, cx: &mut Context<Self>) -> bool {
        if self.pop.take().is_some() {
            cx.notify();
            return true;
        }
        false
    }

    fn pick(&mut self, selection: Selection, cx: &mut Context<Self>) {
        self.select(selection, cx);
        self.open_drawer(cx);
    }

    pub(super) fn spine_animating(&self, now: Instant) -> bool {
        self.tree.animating(now)
            || !self.drawer.settled_at(now)
            || !self.overview_turn.settled_at(now)
            || !self.fold_turn.settled_at(now)
            || self
                .pop
                .as_ref()
                .is_some_and(|pop| !pop.tween.settled_at(now))
    }

    fn avatar(brand: Option<(&'static str, Option<Hsla>)>, theme: &Theme) -> gpui::Div {
        let slot = div()
            .flex_none()
            .size(px(22.0))
            .rounded(px(6.0))
            .border_1()
            .border_color(theme.border)
            .flex()
            .items_center()
            .justify_center();
        match brand {
            Some((mark, tint)) => slot.child(
                icon(mark)
                    .size(px(13.0))
                    .text_color(tint.unwrap_or(theme.text_muted)),
            ),
            None => slot.child(super::super::dashed_circle(theme)),
        }
    }

    fn chip(status: Status, word: &'static str, reduced: bool, theme: &Theme) -> AnyElement {
        let tone = status.tone(theme);
        let (bg, fg) = if status.muted() {
            (theme.wash(0.08), theme.text_muted)
        } else {
            (tone.opacity(0.15), tone)
        };
        div()
            .flex_none()
            .h(px(18.0))
            .pl(px(4.0))
            .pr(px(6.0))
            .rounded(px(4.0))
            .bg(bg)
            .flex()
            .items_center()
            .gap(px(4.0))
            .text_size(ui_rems(11.0))
            .font_weight(gpui::FontWeight::MEDIUM)
            .text_color(fg)
            .child(tree::glyph(status, 12.0, fg, reduced))
            .child(word)
            .into_any_element()
    }

    fn who(name: String, model: String, theme: &Theme) -> gpui::Div {
        div()
            .flex_1()
            .min_w_0()
            .flex()
            .flex_col()
            .child(
                div()
                    .truncate()
                    .text_size(ui_rems(13.0))
                    .font_weight(gpui::FontWeight::MEDIUM)
                    .text_color(theme.text)
                    .child(name),
            )
            .child(
                div()
                    .truncate()
                    .text_size(ui_rems(11.0))
                    .text_color(theme.text_muted)
                    .child(model),
            )
    }

    fn amount(
        &self,
        tokens: Option<u64>,
        live: bool,
        time: Option<String>,
        reduced: bool,
        theme: &Theme,
    ) -> gpui::Div {
        let phase =
            (live && !reduced).then(|| tree::amount::phase(self.shimmer_since, Instant::now()));
        div()
            .flex_1()
            .min_w_0()
            .flex()
            .items_center()
            .gap(px(4.0))
            .text_size(ui_rems(11.0))
            .font_weight(gpui::FontWeight::MEDIUM)
            .children(tree::amount::label(tokens, phase, ui_rems(11.0), theme))
            .children(time.map(|time| {
                let text = if tokens.is_some() {
                    format!("· {time}")
                } else {
                    time
                };
                div()
                    .flex_none()
                    .font_weight(gpui::FontWeight::NORMAL)
                    .text_color(theme.text_muted)
                    .child(text)
            }))
    }

    fn card(&self, selected: bool, theme: &Theme) -> gpui::Div {
        div()
            .relative()
            .flex()
            .flex_col()
            .gap(px(6.0))
            .pt(px(10.0))
            .px(px(10.0))
            .pb(px(8.0))
            .rounded(px(CARD_RADIUS))
            .border_1()
            .border_color(if selected {
                theme.border_strong
            } else {
                theme.border
            })
            .bg(if selected {
                motion::mix(theme.surface_card, theme.text, 0.06)
            } else {
                theme.surface_card
            })
            .cursor_pointer()
    }

    fn fold_button(
        &self,
        steps: usize,
        now: Instant,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let turn = self.fold_turn.value_at(now);
        div()
            .id("task-center-fold")
            .debug_selector(|| "task-center-fold".into())
            .role(gpui::Role::Button)
            .aria_expanded(!self.folded)
            .aria_label(if self.folded {
                format!("Show {steps} steps")
            } else {
                "Fold the steps".into()
            })
            .flex_none()
            .flex()
            .items_center()
            .gap(px(2.0))
            .h(px(20.0))
            .pl(px(6.0))
            .pr(px(4.0))
            .rounded(px(5.0))
            .text_size(ui_rems(11.0))
            .font_weight(gpui::FontWeight::MEDIUM)
            .text_color(theme.text_muted)
            .hover(|s| s.bg(theme.wash(0.08)).text_color(theme.text))
            .child(if self.folded {
                format!("+{steps}")
            } else {
                steps.to_string()
            })
            .child(
                icon(icons::LUCIDE_CHEVRON_DOWN)
                    .size(px(12.0))
                    .text_color(theme.text_muted)
                    .with_transformation(gpui::Transformation::rotate(gpui::radians(
                        -std::f32::consts::FRAC_PI_2 * turn,
                    ))),
            )
            .on_click(cx.listener(|center, _: &ClickEvent, _, cx| {
                cx.stop_propagation();
                center.toggle_steps(cx);
            }))
            .into_any_element()
    }

    #[allow(clippy::too_many_arguments)]
    fn render_lead_card(
        &self,
        task: &zeron_proto::Task,
        plan: &Rollup,
        editable: bool,
        tokens: Option<u64>,
        time: Option<String>,
        reduced: bool,
        theme: &Theme,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let now = Instant::now();
        let (brand, agent) = {
            let state = self.state.read(cx);
            let chat = linked_chat(&state.chats, task);
            let agent = chat.and_then(|chat| chat.config.as_ref()).map(|config| {
                let harness = config.harness.display_name();
                match config.model.as_deref() {
                    Some(model) => format!(
                        "{harness} · {}",
                        crate::pickers::model_display_label(config.harness, model, cx)
                    ),
                    None => harness.to_string(),
                }
            });
            (chat.map(chat_brand), agent)
        };
        let (status, word) = tree::lead_status(task, plan);
        let planning = plan.phase(task) == super::super::PlanPhase::Planning;
        let ready = plan.phase(task) == super::super::PlanPhase::PlanReady;
        let line = if planning {
            "The lead is planning".to_string()
        } else {
            Self::lead_line(task, plan)
        };
        let shimmer = self.shimmer(
            task.completed_at.is_none() && plan.lead == Some(ChatIndicator::Working),
            cx,
        );
        let selected = self.selected == Selection::Lead;
        let top = div()
            .flex()
            .items_center()
            .gap(px(8.0))
            .child(Self::avatar(brand, theme))
            .child(Self::who(
                "Lead".into(),
                agent.unwrap_or_else(|| "Not started".into()),
                theme,
            ))
            .child(Self::chip(status, word, reduced, theme));
        let task_line = div()
            .min_w_0()
            .text_size(ui_rems(12.5))
            .text_color(theme.text_muted)
            .child(super::super::tree::shimmer_text(
                line,
                shimmer,
                ui_rems(12.5),
                theme.text_muted,
                theme,
            ));
        let overview = self.render_overview_section(task, editable, now, theme, window, cx);
        let reads = self.render_reads(task, theme, cx);
        let total = plan.progress.total;
        let approve = (ready && task.completed_at.is_none()).then(|| {
            div()
                .flex()
                .items_center()
                .justify_between()
                .gap(px(8.0))
                .child(
                    div()
                        .min_w_0()
                        .truncate()
                        .text_size(ui_rems(11.5))
                        .text_color(theme.text_muted)
                        .child(format!(
                            "Plan ready · {total} {} · the lead waits for you",
                            plural(total, "step", "steps")
                        )),
                )
                .child(
                    widgets::text_action(theme, widgets::ActionTone::Solid, "Approve and run")
                        .id("task-center-approve")
                        .debug_selector(|| "task-center-approve".into())
                        .role(gpui::Role::Button)
                        .flex_none()
                        .min_h(px(24.0))
                        .rounded_full()
                        .on_click(cx.listener(|center, _: &ClickEvent, _, cx| {
                            cx.stop_propagation();
                            center.send_edits(true, cx)
                        })),
                )
        });
        let steps = plan.steps.len() + self.draft.added.len();
        let live = task.completed_at.is_none()
            && (plan.lead == Some(ChatIndicator::Working) || plan.count(StepStatus::Working) > 0);
        let meta = div()
            .flex()
            .items_center()
            .gap(px(6.0))
            .min_h(px(20.0))
            .child(self.amount(tokens, live, time, reduced, theme))
            .children((steps > 0).then(|| self.fold_button(steps, now, theme, cx)));
        self.card(selected, theme)
            .id("task-center-lead")
            .debug_selector(|| "task-center-lead".into())
            .role(gpui::Role::Button)
            .aria_selected(selected)
            .child(morph::measure(&self.painted, Key::Lead, Edges::default()))
            .child(top)
            .child(task_line)
            .children(overview)
            .children(reads)
            .children(approve)
            .child(meta)
            .on_click(cx.listener(|center, _: &ClickEvent, _, cx| center.pick(Selection::Lead, cx)))
            .into_any_element()
    }

    fn render_overview_section(
        &self,
        task: &zeron_proto::Task,
        editable: bool,
        now: Instant,
        theme: &Theme,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        if !task.plan_doc {
            return None;
        }
        let editor = self
            .editor
            .as_ref()
            .filter(|editor| editor.target == EditTarget::Overview);
        let (text, saved_images) = match &self.draft.overview {
            Some(draft) => (draft.notes.clone(), draft.images.clone()),
            None => (task.plan.clone(), task.images.clone()),
        };
        let open = self.overview_open;
        let turn = self.overview_turn.value_at(now);
        let edit = (editable && editor.is_none()).then(|| {
            widgets::text_action(theme, widgets::ActionTone::Quiet, "Edit")
                .id("task-center-edit-overview")
                .debug_selector(|| "task-center-edit-overview".into())
                .role(gpui::Role::Button)
                .aria_label("Edit the overview")
                .min_h(px(22.0))
                .rounded_full()
                .on_click(cx.listener(|center, _: &ClickEvent, window, cx| {
                    cx.stop_propagation();
                    center.start_edit(EditTarget::Overview, window, cx)
                }))
        });
        let head = div()
            .flex()
            .items_center()
            .gap(px(6.0))
            .child(
                div()
                    .id("task-center-overview-toggle")
                    .debug_selector(|| "task-center-overview-toggle".into())
                    .role(gpui::Role::Button)
                    .aria_label("Overview")
                    .aria_expanded(open)
                    .flex()
                    .items_center()
                    .gap(px(6.0))
                    .cursor_pointer()
                    .text_size(ui_rems(12.0))
                    .font_weight(gpui::FontWeight::MEDIUM)
                    .text_color(theme.text_muted)
                    .hover(|s| s.text_color(theme.text))
                    .child(
                        icon(icons::FILE_MARKDOWN)
                            .size(px(13.0))
                            .text_color(theme.text_muted),
                    )
                    .child("Overview")
                    .child(
                        icon(icons::LUCIDE_CHEVRON_DOWN)
                            .size(px(12.0))
                            .text_color(theme.text_muted)
                            .with_transformation(gpui::Transformation::rotate(gpui::radians(
                                -std::f32::consts::FRAC_PI_2 * turn,
                            ))),
                    )
                    .on_click(cx.listener(|center, _: &ClickEvent, _, cx| {
                        cx.stop_propagation();
                        center.toggle_overview(cx);
                    })),
            )
            .children(
                self.draft
                    .overview
                    .is_some()
                    .then(|| Self::tag_pill(Tag::Edited, theme)),
            )
            .child(div().flex_1())
            .children(edit);
        let section = div()
            .id("task-center-overview")
            .debug_selector(|| "task-center-overview".into())
            .mx(px(-10.0))
            .px(px(10.0))
            .pt(px(8.0))
            .pb(px(2.0))
            .border_t_1()
            .border_color(theme.border)
            .flex()
            .flex_col()
            .gap(px(4.0))
            .on_click(|_, _, cx| cx.stop_propagation());
        if let Some(editor) = editor {
            return Some(
                section
                    .child(self.edit_card(
                        editor,
                        head.into_any_element(),
                        0.0,
                        (12.0, 10.0),
                        theme,
                        window,
                        cx,
                    ))
                    .into_any_element(),
            );
        }
        let body = if text.trim().is_empty() {
            div()
                .text_size(ui_rems(12.5))
                .text_color(theme.text_muted)
                .child("The lead has not written the overview yet.")
                .into_any_element()
        } else if open {
            Self::markdown(
                format!("task-center-overview-{}", task.id),
                &text,
                theme,
                window,
            )
        } else {
            div()
                .truncate()
                .text_size(ui_rems(12.5))
                .text_color(theme.text_muted)
                .child(crate::transcript::single_line(&text))
                .into_any_element()
        };
        let thumbs = (open && !saved_images.is_empty()).then(|| {
            let paths = saved_images.clone();
            images::strip(
                "task-center-overview",
                &saved_images,
                images::SPEC_THUMB,
                &self.state,
                theme,
                move |center: &mut Self, ix, _, cx| center.open_images(paths.clone(), ix, cx),
                None::<fn(&mut Self, usize, &mut Window, &mut Context<Self>)>,
                cx,
            )
            .mt(px(4.0))
        });
        Some(
            section
                .child(morph::measure(
                    &self.painted,
                    Key::Overview,
                    Edges::default(),
                ))
                .child(head)
                .child(body)
                .children(thumbs)
                .into_any_element(),
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn render_spine_step(
        &self,
        index: usize,
        row: &StepRow,
        thread: Option<&Thread>,
        tag: Option<Tag>,
        range: Option<(usize, usize)>,
        editable: bool,
        tokens: Option<u64>,
        reduced: bool,
        theme: &Theme,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let step_id = row.step.id.clone();
        let group = SharedString::from(format!("task-center-step-{step_id}"));
        let selected = self.selected == Selection::Step(step_id.clone());
        let (status, word) =
            Status::of_step(row, thread.and_then(|t| t.delegation.as_ref().map(|d| d.0)));
        let name = format!("Step {}", index + 1);
        let agent = thread
            .and_then(|thread| thread.agent.clone())
            .unwrap_or_else(|| "Not started".into());
        let avatar = Self::avatar(thread.and(row.brand), theme);
        if let Some(editor) = self
            .editor
            .as_ref()
            .filter(|editor| editor.target == EditTarget::Step(step_id.clone()))
        {
            let head = div()
                .flex()
                .items_center()
                .gap(px(8.0))
                .child(avatar)
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .child(
                            div()
                                .text_size(ui_rems(11.0))
                                .text_color(theme.text_muted)
                                .child(name),
                        )
                        .children(editor.title.clone().map(|title| {
                            div()
                                .id("task-center-edit-title")
                                .debug_selector(|| "task-center-edit-title".into())
                                .min_w_0()
                                .text_size(ui_rems(13.0))
                                .text_color(theme.text)
                                .child(title)
                        })),
                );
            return div()
                .id(SharedString::from(format!("task-center-step-{step_id}")))
                .debug_selector(move || format!("task-center-step-{step_id}"))
                .relative()
                .child(morph::measure(
                    &self.painted,
                    Key::Task(row.step.id.clone()),
                    Edges::default(),
                ))
                .child(self.edit_card(
                    editor,
                    head.into_any_element(),
                    30.0,
                    (10.0, 10.0),
                    theme,
                    window,
                    cx,
                ))
                .into_any_element();
        }
        let expanded = self.expanded.contains(&step_id);
        let moved = self.draft.moved.contains(&step_id);
        let registered = tag != Some(Tag::New) && tag != Some(Tag::Removed);
        let retry = registered && row.step.outcome.is_none() && status == Status::Failed;
        let skip = registered
            && row.step.outcome.is_none()
            && matches!(
                row.status,
                StepStatus::Planned | StepStatus::NeedsYou | StepStatus::Failed
            );
        let reopen = registered && row.step.outcome.is_some();
        let removable = editable
            && (tag == Some(Tag::New)
                || (tag != Some(Tag::Removed)
                    && row.status == StepStatus::Planned
                    && thread.is_none()));
        let struck = row.status == StepStatus::Skipped || tag == Some(Tag::Removed);
        let running = status == Status::Running;
        let title_text = SharedString::from(row.step.title.clone());
        let title = if running && !reduced {
            let phase = self.shimmer(true, cx).unwrap_or_default();
            crate::transcript::shimmer_label(
                title_text,
                phase,
                theme.font_sans.clone(),
                ui_rems(12.5).into(),
                None,
                theme.text_muted,
                theme.text,
            )
            .whitespace_nowrap()
            .text_color(theme.text_muted)
            .into_any_element()
        } else {
            div()
                .min_w_0()
                .line_clamp(2)
                .text_color(if running || status.muted() || struck {
                    theme.text_muted
                } else {
                    theme.text
                })
                .when(struck, |el| el.line_through())
                .child(title_text)
                .into_any_element()
        };
        let failure = (status == Status::Failed)
            .then(|| Self::step_note(row, thread))
            .flatten()
            .map(|summary| {
                div()
                    .truncate()
                    .text_size(ui_rems(11.5))
                    .text_color(theme.danger)
                    .child(crate::transcript::single_line(&summary))
            });
        let mut actions: Vec<AnyElement> = Vec::new();
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
        let side_button = |label: &'static str, selector: String, tone: widgets::ActionTone| {
            widgets::text_action(theme, tone, label)
                .id(SharedString::from(selector.clone()))
                .debug_selector(move || selector.clone())
                .role(gpui::Role::Button)
                .aria_label(format!("{label} step {}", index + 1))
                .flex_none()
                .min_h(px(22.0))
                .rounded_full()
        };
        if retry {
            let id = step_id.clone();
            actions.push(
                side_button(
                    "Retry",
                    format!("task-center-retry-{step_id}"),
                    widgets::ActionTone::Filled,
                )
                .on_click(cx.listener(move |_, _: &ClickEvent, _, cx| {
                    cx.stop_propagation();
                    cx.emit(TaskCenterEvent::RetryStep(id.clone()));
                }))
                .into_any_element(),
            );
        }
        if tag == Some(Tag::Removed) {
            let id = step_id.clone();
            actions.push(
                side_button(
                    "Restore",
                    format!("task-center-restore-{step_id}"),
                    widgets::ActionTone::Quiet,
                )
                .on_click(cx.listener(move |center, _: &ClickEvent, _, cx| {
                    cx.stop_propagation();
                    center.toggle_remove(&id, cx);
                }))
                .into_any_element(),
            );
        }
        let toggle_id = step_id.clone();
        let spec_toggle = div()
            .id(SharedString::from(format!("task-center-spec-{step_id}")))
            .debug_selector({
                let id = step_id.clone();
                move || format!("task-center-spec-{id}")
            })
            .role(gpui::Role::Button)
            .aria_label(format!("Spec of step {}", index + 1))
            .aria_expanded(expanded)
            .flex_none()
            .flex()
            .items_center()
            .gap(px(2.0))
            .cursor_pointer()
            .text_size(ui_rems(11.0))
            .text_color(theme.text_muted)
            .hover(|s| s.text_color(theme.text))
            .child("spec")
            .child(
                icon(if expanded {
                    icons::LUCIDE_CHEVRON_DOWN
                } else {
                    icons::LUCIDE_CHEVRON_RIGHT
                })
                .size(px(11.0))
                .text_color(theme.text_muted),
            )
            .on_click(cx.listener(move |center, _: &ClickEvent, _, cx| {
                cx.stop_propagation();
                center.toggle_spec(&toggle_id, cx);
            }));
        let time = tree::thread_time(status, thread.and_then(|t| t.since), None)
            .filter(|_| status.live())
            .or_else(|| {
                let state = self.state.read(cx);
                let stats = row
                    .step
                    .chat_id
                    .as_deref()
                    .filter(|_| thread.is_some())
                    .and_then(|id| self.stats.get(state, id));
                tree::thread_time(status, None, stats.as_ref())
            });
        let meta = div()
            .flex()
            .items_center()
            .gap(px(6.0))
            .min_h(px(20.0))
            .child(self.amount(tokens, running, time, reduced, theme))
            .children(tag.map(|tag| Self::tag_pill(tag, theme)))
            .children(moved.then(|| Self::tag_pill(Tag::Moved, theme)))
            .children(actions)
            .child(spec_toggle);
        let panel = expanded.then(|| {
            let spec = if row.step.notes.trim().is_empty() {
                div()
                    .text_size(ui_rems(12.5))
                    .text_color(theme.text_muted)
                    .child("No spec yet.")
                    .into_any_element()
            } else {
                Self::markdown(
                    format!("task-center-spec-body-{step_id}"),
                    &row.step.notes,
                    theme,
                    window,
                )
            };
            let thumbs = (!row.step.images.is_empty()).then(|| {
                let paths = row.step.images.clone();
                images::strip(
                    &format!("task-center-spec-{step_id}"),
                    &row.step.images,
                    images::SPEC_THUMB,
                    &self.state,
                    theme,
                    move |center: &mut Self, ix, _, cx| center.open_images(paths.clone(), ix, cx),
                    None::<fn(&mut Self, usize, &mut Window, &mut Context<Self>)>,
                    cx,
                )
            });
            let edit = (editable && tag != Some(Tag::Removed)).then(|| {
                let id = step_id.clone();
                side_button(
                    "Edit",
                    format!("task-center-edit-{step_id}"),
                    widgets::ActionTone::Quiet,
                )
                .on_click(cx.listener(
                    move |center, _: &ClickEvent, window, cx| {
                        cx.stop_propagation();
                        center.start_edit(EditTarget::Step(id.clone()), window, cx)
                    },
                ))
            });
            let remove = removable.then(|| {
                let id = step_id.clone();
                side_button(
                    "Remove",
                    format!("task-center-remove-{step_id}"),
                    widgets::ActionTone::Quiet,
                )
                .on_click(cx.listener(move |center, _: &ClickEvent, _, cx| {
                    cx.stop_propagation();
                    center.toggle_remove(&id, cx)
                }))
            });
            div()
                .debug_selector({
                    let id = step_id.clone();
                    move || format!("task-center-spec-panel-{id}")
                })
                .pl(px(10.0))
                .border_l_2()
                .border_color(theme.border)
                .flex()
                .flex_col()
                .gap(px(6.0))
                .child(spec)
                .children(thumbs)
                .child(
                    div()
                        .flex()
                        .gap(px(4.0))
                        .ml(px(-8.0))
                        .children(edit)
                        .children(remove),
                )
        });
        let movable = editable && tag != Some(Tag::Removed) && range.is_some();
        let grip = movable.then(|| {
            let payload = StepDragPayload {
                task_id: self.task_id.clone(),
                step_id: step_id.clone(),
            };
            let selector = format!("task-center-grip-{step_id}");
            let dot = || div().size(px(2.5)).rounded_full().bg(theme.text_muted);
            let column = || {
                div()
                    .flex()
                    .flex_col()
                    .gap(px(2.5))
                    .child(dot())
                    .child(dot())
                    .child(dot())
            };
            div()
                .id(SharedString::from(selector.clone()))
                .debug_selector(move || selector.clone())
                .absolute()
                .inset_0()
                .rounded(px(6.0))
                .bg(theme.surface_card)
                .border_1()
                .border_color(theme.border_strong)
                .flex()
                .items_center()
                .justify_center()
                .gap(px(3.0))
                .invisible()
                .group_hover(group.clone(), |s| s.visible())
                .cursor(CursorStyle::OpenHand)
                .child(column())
                .child(column())
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|center, event: &MouseDownEvent, _, _| {
                        center.press = f32::from(event.position.y);
                    }),
                )
                .on_click(|_, _, cx| cx.stop_propagation())
                .on_drag(payload, |_, _, _, cx| cx.new(|_| StepDragGhost))
        });
        let actions = StepActions {
            step_id: step_id.clone(),
            edit: editable && tag != Some(Tag::Removed),
            thread: thread.and(row.step.chat_id.clone()),
            up: editable
                && range.is_some_and(|(lo, hi)| index >= 1 && (lo..=hi).contains(&(index - 1))),
            down: editable && range.is_some_and(|(lo, hi)| (lo..=hi).contains(&(index + 1))),
            outcome: if reopen {
                Outcome::Reopen
            } else if skip {
                Outcome::Skip
            } else {
                Outcome::Locked
            },
            remove: removable,
        };
        let select_id = step_id.clone();
        let aria = format!("Step {}: {}", index + 1, row.step.title);
        self.card(selected, theme)
            .id(SharedString::from(format!("task-center-step-{step_id}")))
            .debug_selector(move || format!("task-center-step-{step_id}"))
            .group(group)
            .role(gpui::Role::ListItem)
            .aria_label(aria)
            .aria_selected(selected)
            .child(morph::measure(
                &self.painted,
                Key::Task(row.step.id.clone()),
                Edges::default(),
            ))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .child(avatar.relative().children(grip))
                    .child(Self::who(name, agent, theme))
                    .child(Self::chip(status, word, reduced, theme)),
            )
            .child(div().text_size(ui_rems(12.5)).child(title))
            .children(failure)
            .child(meta)
            .children(panel)
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |center, event: &MouseDownEvent, window, cx| {
                    cx.stop_propagation();
                    window.prevent_default();
                    center.open_menu(actions.clone(), event.position, window, cx);
                }),
            )
            .on_click(cx.listener(move |center, _: &ClickEvent, _, cx| {
                center.pick(Selection::Step(select_id.clone()), cx)
            }))
            .into_any_element()
    }

    fn render_ghost(&self, theme: &Theme) -> AnyElement {
        div()
            .id("task-center-add-step")
            .debug_selector(|| "task-center-add-step".into())
            .relative()
            .flex()
            .items_center()
            .gap(px(8.0))
            .min_h(px(44.0))
            .px(px(12.0))
            .rounded(px(CARD_RADIUS))
            .border_1()
            .border_dashed()
            .border_color(theme.border_strong)
            .child(morph::measure(&self.painted, Key::Add, Edges::default()))
            .child(
                icon(icons::PLUS)
                    .size(px(13.0))
                    .flex_none()
                    .text_color(theme.text_muted),
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

    #[allow(clippy::too_many_arguments)]
    fn render_steps_bar(
        &self,
        rows: &[BarRow],
        total: Option<u64>,
        live: bool,
        reduced: bool,
        now: Instant,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let steps = rows.iter().filter(|row| row.id != LEAD).count();
        let running = rows
            .iter()
            .filter(|row| row.id != LEAD && row.status == Status::Running)
            .count();
        let count = |kind: Status| rows.iter().filter(|row| row.status == kind).count();
        let (waiting, failed) = (count(Status::Waiting), count(Status::Failed));
        let button = |kind: Status, n: usize, label: &str, cx: &mut Context<Self>| {
            let tone = kind.tone(theme);
            let open = self.pop.as_ref().is_some_and(|pop| pop.kind == kind);
            let selector = format!(
                "task-center-pop-{}",
                if kind == Status::Waiting {
                    "waiting"
                } else {
                    "failed"
                }
            );
            div()
                .id(SharedString::from(selector.clone()))
                .debug_selector(move || selector.clone())
                .role(gpui::Role::Button)
                .aria_label(format!("{n} {label}"))
                .aria_expanded(open)
                .flex_none()
                .flex()
                .items_center()
                .gap(px(2.0))
                .h(px(22.0))
                .pl(px(7.0))
                .pr(px(4.0))
                .rounded(px(5.0))
                .bg(tone.opacity(0.15))
                .text_color(tone)
                .font_weight(gpui::FontWeight::MEDIUM)
                .cursor_pointer()
                .child(format!("{n} {label}"))
                .child(
                    icon(icons::LUCIDE_CHEVRON_DOWN)
                        .size(px(12.0))
                        .text_color(tone)
                        .when(open, |el| {
                            el.with_transformation(gpui::Transformation::rotate(gpui::radians(
                                std::f32::consts::PI,
                            )))
                        }),
                )
                .on_click(cx.listener(move |center, _: &ClickEvent, _, cx| {
                    cx.stop_propagation();
                    center.toggle_pop(kind, cx)
                }))
        };
        let phase = (live && !reduced).then(|| tree::amount::phase(self.shimmer_since, now));
        let mut summary = div()
            .flex_1()
            .min_w_0()
            .flex()
            .flex_wrap()
            .justify_end()
            .items_center()
            .gap(px(8.0))
            .text_size(ui_rems(12.0))
            .text_color(theme.text_muted)
            .child(format!("{steps} {}", plural(steps, "step", "steps")));
        if running > 0 {
            summary = summary.child(
                div()
                    .text_color(Status::Running.tone(theme))
                    .child(format!("{running} working")),
            );
        }
        if waiting > 0 {
            summary = summary.child(button(Status::Waiting, waiting, "needs you", cx));
        }
        if failed > 0 {
            summary = summary.child(button(Status::Failed, failed, "failed", cx));
        }
        summary = summary.children(tree::amount::label(total, phase, ui_rems(12.0), theme));
        div()
            .id("task-center-steps-bar")
            .debug_selector(|| "task-center-steps-bar".into())
            .relative()
            .flex()
            .items_center()
            .gap(px(8.0))
            .mb(px(16.0))
            .child(
                icon(icons::MISSION_CONTROL)
                    .size(px(13.0))
                    .text_color(theme.text_muted),
            )
            .child(
                div()
                    .flex_none()
                    .text_size(ui_rems(12.0))
                    .font_weight(gpui::FontWeight::MEDIUM)
                    .text_color(theme.text_muted)
                    .child("Steps"),
            )
            .child(summary)
            .children(self.render_pop(rows, now, theme, cx))
            .into_any_element()
    }

    fn render_pop(
        &self,
        rows: &[BarRow],
        now: Instant,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let pop = self.pop.as_ref()?;
        let items: Vec<&BarRow> = rows.iter().filter(|row| row.status == pop.kind).collect();
        if items.is_empty() {
            return None;
        }
        let shown = pop.tween.value_at(now);
        let popup = theme.for_popup();
        let entries = items.into_iter().map(|row| {
            let BarRow {
                id,
                status,
                name,
                title,
                text,
            } = row;
            let lead = id == LEAD;
            let action = |label: &'static str, primary: bool, key: String| {
                widgets::text_action(
                    &popup,
                    if primary {
                        widgets::ActionTone::Solid
                    } else {
                        widgets::ActionTone::Filled
                    },
                    label,
                )
                .id(SharedString::from(key.clone()))
                .debug_selector(move || key.clone())
                .role(gpui::Role::Button)
                .min_h(px(26.0))
                .rounded_full()
            };
            let selection = if lead {
                Selection::Lead
            } else {
                Selection::Step(id.clone())
            };
            let (first, second): (AnyElement, AnyElement) = if pop.kind == Status::Waiting {
                let open = selection.clone();
                let answer = selection.clone();
                let primary = if lead && text.is_some() {
                    action(
                        "Approve and run",
                        true,
                        format!("task-center-pop-approve-{id}"),
                    )
                    .on_click(cx.listener(|center, _: &ClickEvent, _, cx| {
                        center.pop = None;
                        center.send_edits(true, cx)
                    }))
                    .into_any_element()
                } else {
                    action("Answer", true, format!("task-center-pop-answer-{id}"))
                        .on_click(cx.listener(move |center, _: &ClickEvent, _, cx| {
                            center.pop = None;
                            center.pick(answer.clone(), cx)
                        }))
                        .into_any_element()
                };
                (
                    action("Open thread", false, format!("task-center-pop-open-{id}"))
                        .on_click(cx.listener(move |center, _: &ClickEvent, _, cx| {
                            center.pop = None;
                            center.pick(open.clone(), cx)
                        }))
                        .into_any_element(),
                    primary,
                )
            } else {
                let skip = id.clone();
                let retry = id.clone();
                (
                    action("Skip", false, format!("task-center-pop-skip-{id}"))
                        .on_click(cx.listener(move |center, _: &ClickEvent, _, cx| {
                            center.pop = None;
                            center.set_outcome(&skip, Some(StepOutcome::Skipped), cx)
                        }))
                        .into_any_element(),
                    action("Retry", true, format!("task-center-pop-retry-{id}"))
                        .on_click(cx.listener(move |center, _: &ClickEvent, _, cx| {
                            center.pop = None;
                            cx.emit(TaskCenterEvent::RetryStep(retry.clone()))
                        }))
                        .into_any_element(),
                )
            };
            div()
                .flex()
                .flex_col()
                .gap(px(4.0))
                .p(px(8.0))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(8.0))
                        .child(tree::glyph(*status, 12.0, status.tone(&popup), true))
                        .child(
                            div()
                                .flex_none()
                                .text_size(ui_rems(13.0))
                                .font_weight(gpui::FontWeight::MEDIUM)
                                .text_color(popup.text)
                                .child(name.clone()),
                        )
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .truncate()
                                .text_size(ui_rems(12.0))
                                .text_color(popup.text_muted)
                                .child(title.clone()),
                        ),
                )
                .children(text.clone().map(|text| {
                    div()
                        .pl(px(20.0))
                        .text_size(ui_rems(12.0))
                        .text_color(popup.text_muted)
                        .child(text)
                }))
                .child(
                    div()
                        .flex()
                        .justify_end()
                        .gap(px(6.0))
                        .mt(px(4.0))
                        .child(first)
                        .child(second),
                )
        });
        let card = popover::popover_card(&popup)
            .id("task-center-pop")
            .debug_selector(|| "task-center-pop".into())
            .absolute()
            .top(px(30.0 - 4.0 * (1.0 - shown)))
            .right_0()
            .w(px(300.0))
            .opacity(shown)
            .flex()
            .flex_col()
            .on_mouse_down_out(cx.listener(|center, _, _, cx| {
                center.close_pop(cx);
            }))
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .children(entries);
        Some(
            gpui::deferred(crate::frost::frosted(10.0, crate::frost::MENU_BLUR, card))
                .priority(2)
                .into_any_element(),
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn render_spine(
        &mut self,
        task: &zeron_proto::Task,
        plan: &Rollup,
        shown: &[(StepRow, Option<Tag>, Option<Thread>)],
        editable: bool,
        reduced: bool,
        now: Instant,
        theme: &Theme,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let slots = Self::slots(shown);
        let state = self.state.clone();
        let lead_chat = {
            let state = state.read(cx);
            linked_chat(&state.chats, task).map(|chat| chat.id.clone())
        };
        let mut chat_ids: Vec<String> = shown
            .iter()
            .filter(|(_, _, thread)| thread.is_some())
            .filter_map(|(row, ..)| row.step.chat_id.clone())
            .collect();
        chat_ids.extend(lead_chat.clone());
        self.stats
            .refresh(&state, chat_ids, |center: &mut Self| &mut center.stats, cx);
        let (lead_status, _) = tree::lead_status(task, plan);
        let mut rows = vec![BarRow {
            id: LEAD.to_string(),
            status: lead_status,
            name: "Lead".into(),
            title: task.title.clone(),
            text: (plan.phase(task) == super::super::PlanPhase::PlanReady)
                .then(|| "Plan ready: approve to start running the steps.".to_string()),
        }];
        rows.extend(shown.iter().enumerate().map(|(ix, (row, _, thread))| {
            let (status, _) = Status::of_step(
                row,
                thread
                    .as_ref()
                    .and_then(|t| t.delegation.as_ref().map(|d| d.0)),
            );
            let text = Self::step_note(row, thread.as_ref());
            BarRow {
                id: row.step.id.clone(),
                status,
                name: format!("Step {}", ix + 1),
                title: row.step.title.clone(),
                text: text.map(|text| crate::transcript::single_line(&text)),
            }
        }));
        let (tokens, lead_tokens): (Vec<Option<u64>>, Option<u64>) = {
            let state = state.read(cx);
            let of = |id: Option<&str>| {
                id.and_then(|id| tree::amount::of(self.stats.get(state, id).as_ref()))
            };
            (
                shown
                    .iter()
                    .map(|(row, _, thread)| {
                        of(row.step.chat_id.as_deref().filter(|_| thread.is_some()))
                    })
                    .collect(),
                of(lead_chat.as_deref()),
            )
        };
        let branch = tree::amount::branch(lead_tokens, tokens.iter().copied());
        let any_running = plan.count(StepStatus::Working) > 0
            || (plan.lead == Some(ChatIndicator::Working) && task.completed_at.is_none());
        let lead_time = {
            let state = state.read(cx);
            let session = lead_chat.as_deref().and_then(|id| state.session_for(id));
            let stats = lead_chat
                .as_deref()
                .and_then(|id| self.stats.get(state, id));
            tree::thread_time(
                if plan.lead == Some(ChatIndicator::Working) {
                    Status::Running
                } else {
                    Status::Done
                },
                session.and_then(|session| session.started_at),
                stats.as_ref(),
            )
        };

        let mut nodes = vec![NodeIn {
            id: LEAD,
            root: true,
            visible: true,
            status: lead_status,
        }];
        for (ix, (row, ..)) in shown.iter().enumerate() {
            nodes.push(NodeIn {
                id: &row.step.id,
                root: false,
                visible: !self.folded,
                status: rows[ix + 1].status,
            });
        }
        nodes.push(NodeIn {
            id: GHOST,
            root: false,
            visible: editable && !self.folded,
            // Not dimmed like a planned step: the ghost is the place to type.
            status: Status::Done,
        });
        self.tree.sync(&nodes, reduced, now);

        let width = self
            .painted
            .borrow()
            .get(&Key::Canvas)
            .map(|rect| f32::from(rect.size.width) - CANVAS_PAD - self.canvas_pad());
        let known = width.is_some();
        let width = width.unwrap_or(LEAD_WIDTH + 2.0 * COLUMN);
        let center = width / 2.0;
        let column = COLUMN.min(((width - SPINE) / 2.0).max(0.0));
        let height_of = |key: &Key, fallback: f32| {
            self.painted
                .borrow()
                .get(key)
                .map_or(fallback, |rect| f32::from(rect.size.height))
        };
        let lead_width = LEAD_WIDTH.min(width);
        let lead_height = height_of(&Key::Lead, LEAD_HEIGHT);
        let body = lead_height + BODY_GAP;
        let visible: Vec<usize> = (0..shown.len())
            .filter(|ix| self.tree.shown(&shown[*ix].0.step.id))
            .collect();
        let spans: Vec<usize> = visible
            .iter()
            .map(|ix| {
                span(height_of(
                    &Key::Task(shown[*ix].0.step.id.clone()),
                    CARD_HEIGHT,
                ))
            })
            .collect();
        let (places, end) = spine_rows(&spans);
        let pitch = ROW + ROW_GAP;
        let lifted = self.drag.as_ref().map(|drag| (drag.id.clone(), drag.dy));
        let mut placed: Vec<Placed> = Vec::new();
        let mut place = |tree: &mut tree::TreeMotion,
                         id: &str,
                         x: f32,
                         y: f32,
                         width: f32,
                         side: Option<Side>| {
            let extra = lifted
                .as_ref()
                .filter(|(lifted, _)| lifted == id)
                .map_or((0.0, 0.0), |(_, dy)| (0.0, *dy));
            let (gx, gy) = tree.place(id, (x, y), extra, reduced || !known, now);
            let frame = tree.frame(id, now);
            placed.push(Placed {
                id: id.to_string(),
                x: x + gx + extra.0,
                y: y + gy + extra.1 + frame.y,
                width,
                side,
                frame,
            });
        };
        place(
            &mut self.tree,
            LEAD,
            center - lead_width / 2.0,
            0.0,
            lead_width,
            None,
        );
        for (k, ix) in visible.iter().enumerate() {
            let (side, row) = places[k];
            let x = match side {
                Side::Left => center - SPINE / 2.0 - column,
                Side::Right => center + SPINE / 2.0,
            };
            place(
                &mut self.tree,
                &shown[*ix].0.step.id,
                x,
                body + row as f32 * pitch,
                column,
                Some(side),
            );
        }
        let ghost_width = GHOST_WIDTH.min(width);
        if self.tree.shown(GHOST) {
            place(
                &mut self.tree,
                GHOST,
                center - ghost_width / 2.0,
                body + end as f32 * pitch,
                ghost_width,
                None,
            );
        }
        let ghost_height = height_of(&Key::Add, 44.0);
        let bottom = placed
            .iter()
            .map(|placed| {
                let height = if placed.id == LEAD {
                    lead_height
                } else if placed.id == GHOST {
                    ghost_height
                } else {
                    height_of(&Key::Task(placed.id.clone()), CARD_HEIGHT)
                };
                placed.y + height
            })
            .fold(0.0, f32::max);

        let lead = placed.iter().find(|placed| placed.id == LEAD);
        let anchor = lead.map(|lead| (lead.x + lead.width / 2.0, lead.y + lead_height));
        let mut links: Vec<(bool, Link)> = Vec::new();
        let mut leaving = Vec::new();
        if let Some(anchor) = anchor {
            for node in placed.iter().filter(|placed| placed.id != LEAD) {
                let status = rows
                    .iter()
                    .find(|row| row.id == node.id)
                    .map_or(Status::Queued, |row| row.status);
                let to = match node.side {
                    Some(Side::Left) => (node.x + node.width, node.y + ANCHOR_Y),
                    Some(Side::Right) => (node.x, node.y + ANCHOR_Y),
                    None => (anchor.0, node.y),
                };
                let muted = node.id == GHOST || status.muted();
                links.push((
                    muted,
                    Link {
                        from: anchor,
                        to,
                        drawn: node.frame.drawn,
                        color: if node.id == GHOST {
                            tree::link_color(true, theme)
                        } else {
                            self.tree.tint(&node.id, now, |status| {
                                tree::link_color(status.muted(), theme)
                            })
                        },
                    },
                ));
                leaving.push((node.frame.leaving, node.frame.fade, muted));
            }
        }
        links.sort_by_key(|(muted, _)| !*muted);
        let dots: Vec<Dot> = anchor
            .filter(|_| !leaving.is_empty())
            .map(|at| Dot {
                at,
                stroke: tree::link_color(leaving.iter().all(|(_, _, muted)| *muted), theme),
                fill: theme.bg,
                opacity: if leaving.iter().all(|(leaving, ..)| *leaving) {
                    leaving.iter().map(|(_, fade, _)| *fade).fold(0.0, f32::max)
                } else {
                    1.0
                },
            })
            .into_iter()
            .collect();

        let mut cards: Vec<AnyElement> = Vec::new();
        for node in &placed {
            let element = if node.id == LEAD {
                self.render_lead_card(
                    task,
                    plan,
                    editable,
                    branch,
                    lead_time.clone(),
                    reduced,
                    theme,
                    window,
                    cx,
                )
            } else if node.id == GHOST {
                self.render_ghost(theme)
            } else {
                let Some(ix) = shown.iter().position(|(row, ..)| row.step.id == node.id) else {
                    continue;
                };
                let (row, tag, thread) = &shown[ix];
                self.render_spine_step(
                    ix,
                    row,
                    thread.as_ref(),
                    *tag,
                    move_range(&slots, ix),
                    editable,
                    tokens[ix],
                    reduced,
                    theme,
                    window,
                    cx,
                )
            };
            let is_lifted = lifted.as_ref().is_some_and(|(id, _)| *id == node.id);
            let wrapper = div()
                .absolute()
                .left(px(node.x))
                .top(px(node.y))
                .w(px(node.width))
                .opacity(node.frame.opacity)
                .when(is_lifted, |el| {
                    el.rounded(px(CARD_RADIUS)).shadow(vec![gpui::BoxShadow {
                        color: gpui::black().opacity(0.45),
                        offset: gpui::point(px(0.0), px(12.0)),
                        blur_radius: px(30.0),
                        spread_radius: px(0.0),
                        inset: false,
                    }])
                })
                .child(element);
            cards.push(if is_lifted {
                gpui::deferred(wrapper).into_any_element()
            } else {
                wrapper.into_any_element()
            });
        }
        if self
            .pop
            .as_ref()
            .is_some_and(|pop| !rows.iter().any(|row| row.status == pop.kind))
        {
            self.pop = None;
        }
        let bar = self.render_steps_bar(&rows, branch, any_running, reduced, now, theme, cx);
        div()
            .flex()
            .flex_col()
            .child(bar)
            .child(
                div()
                    .id("task-center-steps")
                    .relative()
                    .w_full()
                    .h(px(bottom))
                    .child({
                        let painted = self.painted.clone();
                        gpui::canvas(
                            move |bounds, window, _| {
                                painted.borrow_mut().insert(Key::Steps, bounds);
                                if !known || (f32::from(bounds.size.width) - width).abs() > 0.5 {
                                    window.request_animation_frame();
                                }
                            },
                            |_, _, _, _| {},
                        )
                        .absolute()
                        .inset_0()
                    })
                    .child(tree::links(
                        links.into_iter().map(|(_, link)| link).collect(),
                        Vec::new(),
                    ))
                    .children(cards)
                    .child(tree::links(Vec::new(), dots))
                    .children(self.drop_bar(theme))
                    .on_drag_move::<StepDragPayload>(cx.listener(
                        |center, event: &DragMoveEvent<StepDragPayload>, _, cx| {
                            let drag = event.drag(cx);
                            if drag.task_id != center.task_id {
                                return;
                            }
                            let step_id = drag.step_id.clone();
                            center.drag_to(&step_id, f32::from(event.event.position.y), cx);
                        },
                    ))
                    .on_drop::<StepDragPayload>(
                        cx.listener(|center, _: &StepDragPayload, _, cx| center.drop_step(cx)),
                    )
                    .on_mouse_up_out(
                        MouseButton::Left,
                        cx.listener(|center, _, _, cx| center.drop_step(cx)),
                    ),
            )
            .into_any_element()
    }

    fn drop_bar(&self, theme: &Theme) -> Option<AnyElement> {
        let drag = self.drag.as_ref().filter(|drag| drag.over != drag.from)?;
        let (top, height) = drag.rows[drag.over];
        let (left, width) = drag.boxes[drag.over];
        let y = if drag.over < drag.from {
            top - 7.0
        } else {
            top + height + 5.0
        };
        Some(
            div()
                .debug_selector(|| "task-center-drop-line".into())
                .absolute()
                .left(px(left - drag.origin.0))
                .w(px(width))
                .top(px(y - drag.origin.1))
                .h(px(2.0))
                .rounded(px(1.0))
                .bg(theme.accent)
                .into_any_element(),
        )
    }

    pub(super) fn render_drawer(
        &self,
        live: AnyElement,
        now: Instant,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let shown = self.drawer.value_at(now);
        if shown <= 0.001 && !self.drawer_open {
            return None;
        }
        Some(
            div()
                .id("task-center-drawer")
                .debug_selector(|| "task-center-drawer".into())
                .absolute()
                .top_0()
                .bottom_0()
                .right(px(-(DRAWER_WIDTH + 24.0) * (1.0 - shown)))
                .w(px(DRAWER_WIDTH))
                .bg(theme.bg)
                .border_l_1()
                .border_color(theme.border)
                .shadow(vec![gpui::BoxShadow {
                    color: gpui::black().opacity(0.5),
                    offset: gpui::point(px(-22.0), px(0.0)),
                    blur_radius: px(40.0),
                    spread_radius: px(-28.0),
                    inset: false,
                }])
                .flex()
                .flex_col()
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .child(live)
                .child(
                    div()
                        .id("task-center-drawer-close")
                        .debug_selector(|| "task-center-drawer-close".into())
                        .role(gpui::Role::Button)
                        .aria_label("Close the thread")
                        .absolute()
                        .top(px(12.0))
                        .right(px(10.0))
                        .size(px(28.0))
                        .rounded(px(8.0))
                        .flex()
                        .items_center()
                        .justify_center()
                        .cursor_pointer()
                        .hover(|s| s.bg(theme.wash(0.08)))
                        .child(
                            icon(icons::LUCIDE_X)
                                .size(px(14.0))
                                .text_color(theme.text_muted),
                        )
                        .on_click(
                            cx.listener(|center, _: &ClickEvent, _, cx| center.close_drawer(cx)),
                        ),
                )
                .into_any_element(),
        )
    }

    pub(super) fn canvas_pad(&self) -> f32 {
        if self.drawer_open {
            DRAWER_WIDTH + CANVAS_PAD
        } else {
            CANVAS_PAD
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cards_alternate_sides_one_row_apart_and_a_tall_card_pushes_the_rest() {
        let (rows, end) = spine_rows(&[2, 2, 5, 2]);
        assert_eq!(
            rows,
            [
                (Side::Left, 0),
                (Side::Right, 1),
                (Side::Left, 2),
                (Side::Right, 6)
            ]
        );
        assert_eq!(end, 8, "the ghost sits under the lowest card");
        assert_eq!(span(96.0), 2);
        assert_eq!(span(200.0), 4);
    }
}
