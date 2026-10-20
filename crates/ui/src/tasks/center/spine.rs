use super::*;
use crate::motion::Fold;
use crate::tasks::thread_tree::{NodeKind, TreeNode};
use crate::tasks::tree::{self, Dot, Link, NodeIn, Status};

pub(super) struct SpineMission<'a> {
    pub task: &'a zeron_proto::Task,
    pub plan: &'a Rollup,
    pub shown: &'a [(StepRow, Option<Tag>, Option<Thread>)],
    pub facts: &'a Facts,
    pub editable: bool,
}

const SPINE: f32 = 64.0;
const ROW: f32 = 52.0;
const ROW_GAP: f32 = 12.0;
const COLUMN: f32 = 340.0;
const LEAD_WIDTH: f32 = 420.0;
const BODY_GAP: f32 = 28.0;
const CARD_RADIUS: f32 = 10.0;
const ANCHOR_Y: f32 = 21.0;
pub(super) const CANVAS_PAD: f32 = 20.0;
pub(super) const DRAWER_WIDTH: f32 = 440.0;
const DRAWER: MotionSpec = MotionSpec::new(420, tree::EASE);
const SPEC_CLOSE: MotionSpec = MotionSpec::new(240, tree::EASE);
const SWEEP: std::time::Duration = std::time::Duration::from_millis(1400);
const LEAD: &str = "lead";
const GHOST: &str = "add";
const MARK: &str = "agents-marker";
const MARK_WIDTH: f32 = 300.0;
const MARK_HEIGHT: f32 = 26.0;
const AGENT_HEIGHT: f32 = 84.0;
const CARD_HEIGHT: f32 = 96.0;
const LEAD_HEIGHT: f32 = 150.0;

struct BarRow {
    id: String,
    status: Status,
    word: &'static str,
    name: String,
    title: String,
    text: Option<String>,
    asks: Option<usize>,
}

pub(super) struct Facts {
    rows: Vec<BarRow>,
    tokens: Vec<Option<u64>>,
    branch: Option<u64>,
    live: bool,
    lead_time: Option<String>,
    lead_chat: Option<String>,
    lead_agent: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Body {
    None,
    Read,
    Editor,
}

pub(super) struct Spec {
    pub fold: Fold,
    pub edit: Rc<Cell<f32>>,
    turn: Tween,
    drawn: Cell<(Body, Option<f32>)>,
}

impl Spec {
    fn closed(now: Instant) -> Self {
        Self {
            fold: Fold::at(false, now),
            edit: Rc::new(Cell::new(0.0)),
            turn: Tween::at(0.0, now),
            drawn: Cell::new((Body::None, Some(0.0))),
        }
    }
}

struct Region {
    body: Body,
    natural: f32,
    clip: Option<(f32, f32)>,
}

impl Region {
    fn height(&self) -> f32 {
        self.clip.map_or(self.natural, |(height, _)| height)
    }
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

fn spine_tops(heights: &[f32]) -> (Vec<(Side, f32)>, f32) {
    let pitch = ROW + ROW_GAP;
    let (mut top, mut bottom) = (0.0_f32, 0.0_f32);
    let tops = heights
        .iter()
        .enumerate()
        .map(|(i, height)| {
            let side = if i % 2 == 0 { Side::Left } else { Side::Right };
            let at = (side, top);
            bottom = bottom.max(top + height);
            top += pitch.max(height + ROW_GAP - pitch);
            at
        })
        .collect();
    let end = if heights.is_empty() {
        0.0
    } else {
        bottom + ROW_GAP + 16.0
    };
    (tops, end)
}

struct Placed {
    id: String,
    x: f32,
    y: f32,
    width: f32,
    height: f32,
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

    pub(super) fn toggle_steps(&mut self, cx: &mut Context<Self>) {
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

    pub(super) fn pick(&mut self, selection: Selection, cx: &mut Context<Self>) {
        self.select(selection, cx);
        self.open_drawer(cx);
    }

    pub(super) fn spec_open(&self, id: &str) -> bool {
        self.expanded
            .get(id)
            .is_some_and(|spec| spec.fold.tween.target() == 1.0)
    }

    pub(super) fn open_spec(&mut self, id: &str, open: bool, reduced: bool, now: Instant) {
        let spec = self
            .expanded
            .entry(id.to_string())
            .or_insert_with(|| Spec::closed(now));
        spec.fold.set(
            open,
            if open { tree::FOLD } else { SPEC_CLOSE },
            reduced,
            now,
        );
        let to = f32::from(u8::from(open));
        if spec.turn.target() != to {
            spec.turn.retarget(to, tree::CHEVRON, reduced, now);
        }
    }

    pub(super) fn last_region(&self, id: &str) -> f32 {
        let Some(spec) = self.expanded.get(id) else {
            return 0.0;
        };
        match spec.drawn.get() {
            (_, Some(height)) => height,
            (Body::Read, None) => spec.fold.height.get(),
            (Body::Editor, None) => spec.edit.get(),
            (Body::None, None) => 0.0,
        }
    }

    fn region(&self, id: &str, now: Instant) -> Region {
        let hidden = Region {
            body: Body::None,
            natural: 0.0,
            clip: Some((0.0, 1.0)),
        };
        let Some(spec) = self.expanded.get(id) else {
            return hidden;
        };
        if let Some(editor) = self
            .editor
            .as_ref()
            .filter(|editor| matches!(&editor.target, EditTarget::Step(step) if step == id))
        {
            let natural = spec.edit.get();
            let to = if natural > 0.0 { natural } else { editor.from };
            return Region {
                body: Body::Editor,
                natural,
                clip: (!editor.grow.settled_at(now)).then(|| {
                    (
                        motion::lerp(editor.from, to, editor.grow.value_at(now)),
                        editor.fade.value_at(now),
                    )
                }),
            };
        }
        let natural = spec.fold.height.get();
        if let Some(leave) = self
            .leaving
            .as_ref()
            .filter(|leave| leave.step == id && !leave.grow.settled_at(now))
        {
            return Region {
                body: Body::Read,
                natural,
                clip: Some((
                    motion::lerp(leave.from, natural, leave.grow.value_at(now)),
                    leave.fade.value_at(now),
                )),
            };
        }
        if spec.fold.gone(now) {
            return hidden;
        }
        Region {
            body: Body::Read,
            natural,
            clip: spec.fold.moving(now).then(|| {
                let value = spec.fold.value(now);
                (natural * value, value)
            }),
        }
    }

    fn sliding(&self, now: Instant) -> bool {
        self.expanded.values().any(|spec| spec.fold.moving(now))
            || self.editor.as_ref().is_some_and(|editor| {
                matches!(editor.target, EditTarget::Step(_)) && !editor.grow.settled_at(now)
            })
            || self
                .leaving
                .as_ref()
                .is_some_and(|leave| !leave.grow.settled_at(now))
    }

    pub(super) fn spine_animating(&self, now: Instant) -> bool {
        self.tree.animating(now)
            || self.sliding(now)
            || self
                .expanded
                .values()
                .any(|spec| !spec.turn.settled_at(now))
            || self
                .editor
                .as_ref()
                .is_some_and(|editor| !editor.grow.settled_at(now) || !editor.fade.settled_at(now))
            || self
                .leaving
                .as_ref()
                .is_some_and(|leave| !leave.fade.settled_at(now))
            || !self.drawer.settled_at(now)
            || !self.overview_turn.settled_at(now)
            || !self.fold_turn.settled_at(now)
            || self
                .pop
                .as_ref()
                .is_some_and(|pop| !pop.tween.settled_at(now))
    }

    pub(crate) fn avatar(brand: Option<(&'static str, Option<Hsla>)>, theme: &Theme) -> gpui::Div {
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

    pub(crate) fn chip(
        status: Status,
        word: &'static str,
        reduced: bool,
        theme: &Theme,
    ) -> AnyElement {
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

    pub(super) fn amount(
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
            (
                chat.map(chat_brand),
                chat.and_then(|chat| agent_of(chat, cx)),
            )
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
                        .aria_label("Approve and run")
                        .flex_none()
                        .min_h(px(24.0))
                        .rounded_full()
                        .on_click(cx.listener(|center, _: &ClickEvent, _, cx| {
                            cx.stop_propagation();
                            center.send_edits(true, cx)
                        })),
                )
        });
        let supervisor = (!ready && task.completed_at.is_none())
            .then(|| crate::supervisor::plan_approval(self.state.read(cx), &task.id, Utc::now()))
            .flatten()
            .filter(|approval| *approval != crate::supervisor::ApprovalState::TakenBack)
            .map(|approval| {
                let task_id = task.id.clone();
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap(px(8.0))
                    .child(
                        div()
                            .min_w_0()
                            .text_size(ui_rems(11.5))
                            .text_color(theme.text_muted)
                            .child(approval.line()),
                    )
                    .children(approval.can_take_back().then(|| {
                        widgets::text_action(theme, widgets::ActionTone::Outlined, "Take back")
                            .id("task-center-take-back")
                            .debug_selector(|| "task-center-take-back".into())
                            .role(gpui::Role::Button)
                            .aria_label("Take back")
                            .flex_none()
                            .min_h(px(24.0))
                            .rounded_full()
                            .on_click(cx.listener(move |_, _: &ClickEvent, _, cx| {
                                cx.stop_propagation();
                                cx.emit(TaskCenterEvent::TakeBackApproval(task_id.clone()))
                            }))
                    }))
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
            .gap(px(6.0))
            .id("task-center-lead")
            .debug_selector(|| "task-center-lead".into())
            .role(gpui::Role::Button)
            .aria_label("Lead")
            .aria_selected(selected)
            .child(morph::measure(&self.painted, Key::Lead, Edges::default()))
            .child(top)
            .child(task_line)
            .children(overview)
            .children(reads)
            .children(approve)
            .children(supervisor)
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
                    .child(self.edit_card(editor, head.into_any_element(), theme, window, cx))
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

    fn tool(
        theme: &Theme,
        id: String,
        mark: Option<&'static str>,
        label: &'static str,
        step: usize,
    ) -> gpui::Stateful<gpui::Div> {
        let selector = id.clone();
        div()
            .id(SharedString::from(id))
            .debug_selector(move || selector.clone())
            .role(gpui::Role::Button)
            .aria_label(format!("{label} step {step}"))
            .flex_none()
            .flex()
            .items_center()
            .gap(px(5.0))
            .h(px(22.0))
            .px(px(8.0))
            .rounded_full()
            .cursor_pointer()
            .text_size(ui_rems(11.5))
            .text_color(theme.text_muted)
            .hover(|s| s.bg(theme.wash(0.10)).text_color(theme.text))
            .children(mark.map(|mark| icon(mark).size(px(12.0)).text_color(theme.text_muted)))
            .child(label)
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
        inset: Option<AnyElement>,
        reduced: bool,
        now: Instant,
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
        let editor = self
            .editor
            .as_ref()
            .filter(|editor| editor.target == EditTarget::Step(step_id.clone()));
        let spec = self.expanded.get(&step_id);
        let region = self.region(&step_id, now);
        if let Some(spec) = spec {
            spec.drawn
                .set((region.body, region.clip.map(|(height, _)| height)));
        }
        let open = editor.is_some() || self.spec_open(&step_id);
        let turn = spec.map_or(0.0, |spec| spec.turn.value_at(now));
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
        let edits = editable && tag != Some(Tag::Removed);
        let struck = row.status == StepStatus::Skipped || tag == Some(Tag::Removed);
        let running = status == Status::Running;
        let title_text = SharedString::from(row.step.title.clone());
        let title = match editor.and_then(|editor| editor.title.clone()) {
            Some(input) => {
                let focused = input.focus_handle(cx).is_focused(window);
                div()
                    .id("task-center-edit-title")
                    .debug_selector(|| "task-center-edit-title".into())
                    .mt(px(-1.0))
                    .mx(px(-6.0))
                    .px(px(6.0))
                    .py(px(3.0))
                    .rounded(px(6.0))
                    .bg(theme.input_bg)
                    .border_1()
                    .border_color(if focused {
                        theme.accent.opacity(0.7)
                    } else {
                        theme.border_strong
                    })
                    .text_size(ui_rems(13.0))
                    .font_weight(gpui::FontWeight::MEDIUM)
                    .text_color(theme.text)
                    .child(crate::composer::growing_field(input, 18.0, 36.0, cx))
                    .into_any_element()
            }
            None if running && !reduced => {
                let phase = self.shimmer(true, cx).unwrap_or_default();
                crate::transcript::shimmer_label(
                    title_text,
                    phase,
                    theme.font_sans.clone(),
                    ui_rems(13.0).into(),
                    None,
                    theme.text_muted,
                    theme.text,
                )
                .whitespace_nowrap()
                .font_weight(gpui::FontWeight::MEDIUM)
                .text_color(theme.text_muted)
                .into_any_element()
            }
            None => div()
                .min_w_0()
                .line_clamp(2)
                .text_size(ui_rems(13.0))
                .font_weight(gpui::FontWeight::MEDIUM)
                .text_color(if struck { theme.text_muted } else { theme.text })
                .when(struck, |el| el.line_through())
                .child(title_text)
                .into_any_element(),
        };
        let kicker = div()
            .flex()
            .items_center()
            .gap(px(4.0))
            .min_w_0()
            .text_size(ui_rems(11.0))
            .text_color(theme.text_muted)
            .child(
                div()
                    .flex_none()
                    .font_weight(gpui::FontWeight::MEDIUM)
                    .text_color(theme.text)
                    .child(name),
            )
            .child(div().min_w_0().truncate().child(format!("· {agent}")));
        let movable = editable && tag != Some(Tag::Removed) && range.is_some();
        let grip = movable.then(|| {
            let payload = StepDragPayload {
                task_id: row.step.task_id.clone(),
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
        let head = div()
            .flex()
            .items_start()
            .gap(px(8.0))
            .child(
                Self::avatar(thread.and(row.brand), theme)
                    .mt(px(1.0))
                    .relative()
                    .children(grip),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap(px(2.0))
                    .child(kicker)
                    .child(title),
            )
            .child(Self::chip(status, word, reduced, theme));
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
        let line = matches!(status, Status::Waiting | Status::Failed).then(|| {
            let tone = status.tone(theme);
            let waiting = status == Status::Waiting;
            let (text, more) = if waiting {
                self.question(row).map_or_else(
                    || ("This step is waiting for you.".to_string(), 0),
                    |(question, more)| (crate::transcript::single_line(question), more),
                )
            } else {
                (
                    self.step_note(row, thread)
                        .map(|text| crate::transcript::single_line(&text))
                        .unwrap_or_else(|| "This step stopped before it finished.".into()),
                    0,
                )
            };
            let action = if waiting {
                let id = step_id.clone();
                Some(
                    side_button(
                        "Answer",
                        format!("task-center-answer-{step_id}"),
                        widgets::ActionTone::Solid,
                    )
                    .on_click(cx.listener(
                        move |center, _: &ClickEvent, _, cx| {
                            cx.stop_propagation();
                            center.pick(Selection::Step(id.clone()), cx);
                        },
                    )),
                )
            } else if retry {
                let id = step_id.clone();
                Some(
                    side_button(
                        "Retry",
                        format!("task-center-retry-{step_id}"),
                        widgets::ActionTone::Filled,
                    )
                    .on_click(cx.listener(move |_, _: &ClickEvent, _, cx| {
                        cx.stop_propagation();
                        cx.emit(TaskCenterEvent::RetryStep(id.clone()));
                    })),
                )
            } else {
                None
            };
            div()
                .debug_selector({
                    let id = step_id.clone();
                    move || format!("task-center-step-note-{id}")
                })
                .mt(px(7.0))
                .ml(px(30.0))
                .flex()
                .items_center()
                .gap(px(8.0))
                .pl(px(9.0))
                .pr(px(6.0))
                .py(px(6.0))
                .rounded(px(8.0))
                .bg(tone.opacity(0.10))
                .text_size(ui_rems(12.0))
                .text_color(theme.text)
                .child(tree::glyph(status, 13.0, tone, reduced))
                .child(div().flex_1().min_w_0().line_clamp(2).child(text))
                .children((more > 0).then(|| {
                    div()
                        .flex_none()
                        .text_color(theme.text_muted)
                        .child(format!("+{more}"))
                }))
                .children(action)
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
        let images = row.step.images.len();
        let empty = row.step.notes.trim().is_empty() && images == 0;
        let toggle_id = step_id.clone();
        let spec_button = div()
            .id(SharedString::from(format!("task-center-spec-{step_id}")))
            .debug_selector({
                let id = step_id.clone();
                move || format!("task-center-spec-{id}")
            })
            .role(gpui::Role::Button)
            .aria_label(format!("Spec of step {}", index + 1))
            .aria_expanded(open)
            .flex_none()
            .flex()
            .items_center()
            .gap(px(4.0))
            .h(px(22.0))
            .pl(px(8.0))
            .pr(px(6.0))
            .rounded(px(6.0))
            .cursor_pointer()
            .text_size(ui_rems(11.5))
            .text_color(if open { theme.text } else { theme.text_muted })
            .when(open, |el| el.bg(theme.wash(0.08)))
            .hover(|s| s.bg(theme.wash(0.08)).text_color(theme.text))
            .child(if empty && edits { "Add spec" } else { "Spec" })
            .children((images > 0).then(|| {
                div()
                    .flex()
                    .items_center()
                    .gap(px(2.0))
                    .text_color(theme.text_faint)
                    .child(
                        icon(icons::GALLERY)
                            .size(px(11.0))
                            .text_color(theme.text_faint),
                    )
                    .child(images.to_string())
            }))
            .child(
                icon(icons::LUCIDE_CHEVRON_RIGHT)
                    .size(px(12.0))
                    .text_color(theme.text_muted)
                    .with_transformation(gpui::Transformation::rotate(gpui::radians(
                        std::f32::consts::FRAC_PI_2 * turn,
                    ))),
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
        let footer = div()
            .mt(px(7.0))
            .ml(px(30.0))
            .flex()
            .items_center()
            .gap(px(6.0))
            .min_h(px(22.0))
            .child(self.amount(tokens, running, time, reduced, theme))
            .children(tag.map(|tag| Self::tag_pill(tag, theme)))
            .children(moved.then(|| Self::tag_pill(Tag::Moved, theme)))
            .children(actions)
            .child(spec_button);
        let thread_chat = thread.and(row.step.chat_id.clone());
        let body = match (region.body, editor) {
            (Body::Editor, Some(editor)) => Some(
                div()
                    .debug_selector(|| "task-center-editor".into())
                    .ml(px(30.0))
                    .mb(px(2.0))
                    .pt(px(10.0))
                    .px(px(12.0))
                    .pb(px(8.0))
                    .rounded(px(8.0))
                    .bg(theme.input_bg)
                    .border_1()
                    .border_color(theme.accent.opacity(0.55))
                    .shadow(vec![gpui::BoxShadow {
                        color: theme.accent.opacity(0.14),
                        offset: gpui::point(px(0.0), px(0.0)),
                        blur_radius: px(0.0),
                        spread_radius: px(3.0),
                        inset: false,
                    }])
                    .flex()
                    .flex_col()
                    .gap(px(8.0))
                    .child(self.edit_notes(editor, 54.0, theme, cx))
                    .child(self.edit_tray(editor, theme, cx))
                    .child(self.edit_footer(theme, cx).mr(px(-4.0)).pt(px(7.0)))
                    .into_any_element(),
            ),
            (Body::Read, _) => {
                let spec = if row.step.notes.trim().is_empty() {
                    div()
                        .text_size(ui_rems(12.5))
                        .text_color(theme.text_muted)
                        .child("No spec yet. The lead writes one when it plans the step.")
                        .into_any_element()
                } else {
                    Self::markdown(
                        format!("task-center-spec-body-{step_id}"),
                        &row.step.notes,
                        theme,
                        window,
                    )
                };
                let thumbs = (images > 0).then(|| {
                    let paths = row.step.images.clone();
                    images::strip(
                        &format!("task-center-spec-{step_id}"),
                        &row.step.images,
                        images::SPEC_THUMB,
                        &self.state,
                        theme,
                        move |center: &mut Self, ix, _, cx| {
                            center.open_images(paths.clone(), ix, cx)
                        },
                        None::<fn(&mut Self, usize, &mut Window, &mut Context<Self>)>,
                        cx,
                    )
                });
                let mut tools: Vec<AnyElement> = Vec::new();
                if edits {
                    let id = step_id.clone();
                    tools.push(
                        Self::tool(
                            theme,
                            format!("task-center-edit-{step_id}"),
                            Some(icons::LUCIDE_PENCIL),
                            "Edit",
                            index + 1,
                        )
                        .on_click(cx.listener(move |center, _: &ClickEvent, window, cx| {
                            cx.stop_propagation();
                            center.start_edit(EditTarget::Step(id.clone()), window, cx)
                        }))
                        .into_any_element(),
                    );
                    let id = step_id.clone();
                    tools.push(
                        Self::tool(
                            theme,
                            format!("task-center-add-image-{step_id}"),
                            Some(icons::GALLERY),
                            "Add image",
                            index + 1,
                        )
                        .on_click(cx.listener(move |center, _: &ClickEvent, _, cx| {
                            cx.stop_propagation();
                            center.pick_images(Some(id.clone()), cx)
                        }))
                        .into_any_element(),
                    );
                }
                if removable {
                    let id = step_id.clone();
                    tools.push(
                        Self::tool(
                            theme,
                            format!("task-center-remove-{step_id}"),
                            None,
                            "Remove",
                            index + 1,
                        )
                        .on_click(cx.listener(move |center, _: &ClickEvent, _, cx| {
                            cx.stop_propagation();
                            center.toggle_remove(&id, cx)
                        }))
                        .into_any_element(),
                    );
                }
                let open_thread = thread_chat.clone().map(|chat| {
                    Self::tool(
                        theme,
                        format!("task-center-spec-open-{step_id}"),
                        None,
                        "Open thread",
                        index + 1,
                    )
                    .child(
                        icon(icons::LUCIDE_ARROW_UP_RIGHT)
                            .size(px(12.0))
                            .text_color(theme.text_muted),
                    )
                    .on_click(cx.listener(move |_, _: &ClickEvent, _, cx| {
                        cx.stop_propagation();
                        cx.emit(TaskCenterEvent::OpenChat(chat.clone()))
                    }))
                });
                let toolbar = (!tools.is_empty() || open_thread.is_some()).then(|| {
                    div()
                        .flex()
                        .items_center()
                        .gap(px(2.0))
                        .ml(px(-8.0))
                        .mr(px(-6.0))
                        .pt(px(5.0))
                        .border_t_1()
                        .border_color(theme.border)
                        .children(tools)
                        .child(div().flex_1())
                        .children(open_thread)
                });
                Some(
                    div()
                        .debug_selector({
                            let id = step_id.clone();
                            move || format!("task-center-spec-panel-{id}")
                        })
                        .ml(px(30.0))
                        .mb(px(2.0))
                        .pt(px(10.0))
                        .px(px(12.0))
                        .pb(px(6.0))
                        .rounded(px(8.0))
                        .bg(theme.wash(0.05))
                        .border_1()
                        .border_color(theme.border)
                        .flex()
                        .flex_col()
                        .gap(px(8.0))
                        .child(spec)
                        .children(thumbs)
                        .children(toolbar)
                        .into_any_element(),
                )
            }
            _ => None,
        };
        let well = body.zip(spec).map(|(body, spec)| {
            let probe = if region.body == Body::Editor {
                spec.edit.clone()
            } else {
                spec.fold.height.clone()
            };
            motion::clipped(
                format!("task-center-spec-clip-{step_id}"),
                probe,
                region.clip,
                div().pt(px(7.0)).child(body),
            )
        });
        let actions = StepActions {
            step_id: step_id.clone(),
            edit: edits,
            thread: thread_chat,
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
        let editing = editor.is_some();
        let card = self
            .card(selected, theme)
            .id(SharedString::from(format!("task-center-step-{step_id}")))
            .debug_selector(move || format!("task-center-step-{step_id}"))
            .group(group)
            .role(gpui::Role::ListItem)
            .aria_label(aria)
            .aria_selected(selected)
            .when(editing, |el| {
                el.border_color(motion::mix(theme.border, theme.accent, 0.35))
                    .cursor_default()
            })
            .child(morph::measure(
                &self.painted,
                Key::Task(row.step.id.clone()),
                Edges::default(),
            ))
            .child(head)
            .children(line)
            .children(inset.map(|inset| div().mt(px(7.0)).child(inset)))
            .child(footer)
            .children(well)
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |center, event: &MouseDownEvent, window, cx| {
                    cx.stop_propagation();
                    window.prevent_default();
                    center.open_menu(MenuFor::Step(actions.clone()), event.position, window, cx);
                }),
            );
        if editing {
            self.edit_events(card, theme, cx).into_any_element()
        } else {
            card.on_click(cx.listener(move |center, _: &ClickEvent, _, cx| {
                center.pick(Selection::Step(select_id.clone()), cx)
            }))
            .into_any_element()
        }
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
            .bg(theme.bg)
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
    fn status_pill(
        &self,
        facts: &Facts,
        status: Status,
        label: String,
        pop: Option<Status>,
        reduced: bool,
        now: Instant,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let tone = status.tone(theme);
        let (bg, fg) = if status.muted() {
            (theme.wash(0.08), theme.text_muted)
        } else {
            (tone.opacity(0.15), tone)
        };
        let open = pop.is_some_and(|kind| self.pop.as_ref().is_some_and(|pop| pop.kind == kind));
        let selector = match pop {
            Some(Status::Waiting) => "task-center-pop-waiting",
            Some(_) => "task-center-pop-failed",
            None => "task-center-status",
        };
        let pill = div()
            .id(selector)
            .debug_selector(move || selector.into())
            .flex_none()
            .flex()
            .items_center()
            .gap(px(6.0))
            .h(px(28.0))
            .pl(px(10.0))
            .pr(px(if pop.is_some() { 8.0 } else { 12.0 }))
            .rounded_full()
            .bg(bg)
            .text_size(ui_rems(12.5))
            .font_weight(gpui::FontWeight::MEDIUM)
            .text_color(fg)
            .child(tree::glyph(status, 13.0, fg, reduced))
            .child(label.clone());
        let pill = match pop {
            Some(kind) => pill
                .role(gpui::Role::Button)
                .aria_label(label)
                .aria_expanded(open)
                .cursor_pointer()
                .child(
                    icon(icons::LUCIDE_CHEVRON_DOWN)
                        .size(px(12.0))
                        .text_color(fg)
                        .when(open, |el| {
                            el.with_transformation(gpui::Transformation::rotate(gpui::radians(
                                std::f32::consts::PI,
                            )))
                        }),
                )
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |center, _, _, _| {
                        let open = center.pop.as_ref().map(|pop| pop.kind);
                        center.pop_pressed = open.or(center.pop_dismissed) == Some(kind);
                    }),
                )
                .on_click(cx.listener(move |center, _: &ClickEvent, _, cx| {
                    cx.stop_propagation();
                    if !std::mem::take(&mut center.pop_pressed) {
                        center.toggle_pop(kind, cx)
                    }
                })),
            None => pill,
        };
        div()
            .relative()
            .flex_none()
            .child(pill)
            .children(
                open.then(|| self.render_pop(&facts.rows, now, theme, cx))
                    .flatten(),
            )
            .into_any_element()
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn render_header(
        &self,
        task: &zeron_proto::Task,
        plan: &Rollup,
        facts: &Facts,
        reduced: bool,
        now: Instant,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let done = task.completed_at.is_some();
        let steps: Vec<&BarRow> = facts.rows.iter().filter(|row| row.id != LEAD).collect();
        let count = |kind: Status| facts.rows.iter().filter(|row| row.status == kind).count();
        let (waiting, failed) = (count(Status::Waiting), count(Status::Failed));
        let running = steps
            .iter()
            .filter(|row| row.status == Status::Running)
            .count();
        let PlanProgress {
            done: finished,
            total,
        } = plan.progress;
        let review = facts
            .rows
            .iter()
            .any(|row| row.id == LEAD && row.status == Status::Waiting && row.text.is_some());
        let mut pills = Vec::new();
        if done {
            pills.push(self.status_pill(
                facts,
                Status::Done,
                "Done".into(),
                None,
                reduced,
                now,
                theme,
                cx,
            ));
        } else {
            if waiting > 0 {
                let asking: Vec<&BarRow> = facts
                    .rows
                    .iter()
                    .filter(|row| {
                        row.status == Status::Waiting && !(row.id == LEAD && row.text.is_some())
                    })
                    .collect();
                let mut parts = Vec::new();
                if review {
                    parts.push("plan review".to_string());
                }
                if asking.iter().any(|row| row.asks.is_none()) {
                    let steps = asking.len();
                    parts.push(format!("{steps} {}", plural(steps, "step", "steps")));
                } else {
                    let questions: usize = asking.iter().filter_map(|row| row.asks).sum();
                    if questions > 0 {
                        parts.push(format!(
                            "{questions} {}",
                            plural(questions, "question", "questions")
                        ));
                    }
                }
                pills.push(self.status_pill(
                    facts,
                    Status::Waiting,
                    format!("Needs you · {}", parts.join(", ")),
                    Some(Status::Waiting),
                    reduced,
                    now,
                    theme,
                    cx,
                ));
            }
            if failed > 0 {
                pills.push(self.status_pill(
                    facts,
                    Status::Failed,
                    format!("{failed} {} failed", plural(failed, "step", "steps")),
                    Some(Status::Failed),
                    reduced,
                    now,
                    theme,
                    cx,
                ));
            }
            if pills.is_empty() {
                let (status, word) = tree::lead_status(task, plan);
                pills.push(self.status_pill(
                    facts,
                    status,
                    word.into(),
                    None,
                    reduced,
                    now,
                    theme,
                    cx,
                ));
            }
        }
        let button = |label: SharedString, selector: &'static str, tone: widgets::ActionTone| {
            widgets::text_action(theme, tone, label.clone())
                .id(selector)
                .debug_selector(move || selector.into())
                .role(gpui::Role::Button)
                .aria_label(label)
                .flex_none()
                .min_h(px(28.0))
                .rounded_full()
        };
        let primary = if done {
            None
        } else if let Some(row) = facts.rows.iter().find(|row| row.status == Status::Waiting) {
            if row.id == LEAD && row.text.is_some() {
                Some(
                    button(
                        "Approve and run".into(),
                        "task-center-header-approve",
                        widgets::ActionTone::Solid,
                    )
                    .on_click(cx.listener(|center, _: &ClickEvent, _, cx| {
                        cx.stop_propagation();
                        center.send_edits(true, cx)
                    })),
                )
            } else {
                let selection = if row.id == LEAD {
                    Selection::Lead
                } else {
                    Selection::Step(row.id.clone())
                };
                Some(
                    button(
                        "Answer".into(),
                        "task-center-header-answer",
                        widgets::ActionTone::Solid,
                    )
                    .on_click(cx.listener(
                        move |center, _: &ClickEvent, _, cx| {
                            cx.stop_propagation();
                            center.pick(selection.clone(), cx)
                        },
                    )),
                )
            }
        } else {
            steps
                .iter()
                .find(|row| row.status == Status::Failed)
                .map(|row| {
                    let id = row.id.clone();
                    button(
                        format!("Retry {}", row.name.to_lowercase()).into(),
                        "task-center-header-retry",
                        widgets::ActionTone::Solid,
                    )
                    .on_click(cx.listener(move |_, _: &ClickEvent, _, cx| {
                        cx.stop_propagation();
                        cx.emit(TaskCenterEvent::RetryStep(id.clone()))
                    }))
                })
        };
        let watch = facts.lead_chat.as_deref().and_then(|root| {
            self.render_watch_button(
                super::watch_sheet::WatchTarget {
                    root,
                    task: Some(task),
                    title: &task.title,
                },
                theme,
                cx,
            )
        });
        let task_id = task.id.clone();
        let toggle_label = if done { "Reopen" } else { "Mark as done" };
        let toggle = button(
            toggle_label.into(),
            "task-center-toggle-done",
            widgets::ActionTone::Filled,
        )
        .on_click(cx.listener(move |_, _: &ClickEvent, _, cx| {
            cx.emit(TaskCenterEvent::Update {
                task_id: task_id.clone(),
                patch: TaskPatch {
                    completed: Some(!done),
                    ..Default::default()
                },
            });
        }));
        let lead = facts.lead_chat.clone();
        let more = div()
            .id("task-center-more")
            .debug_selector(|| "task-center-more".into())
            .role(gpui::Role::Button)
            .aria_label("More")
            .flex_none()
            .size(px(28.0))
            .rounded_full()
            .flex()
            .items_center()
            .justify_center()
            .cursor_pointer()
            .hover(|s| s.bg(theme.wash(0.08)))
            .child(
                icon(icons::LUCIDE_ELLIPSIS)
                    .size(px(14.0))
                    .text_color(theme.text_muted),
            )
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_click(cx.listener(move |center, event: &ClickEvent, window, cx| {
                cx.stop_propagation();
                center.open_menu(
                    MenuFor::Mission { lead: lead.clone() },
                    event.position(),
                    window,
                    cx,
                );
            }));
        let local = |at: chrono::DateTime<Utc>| {
            let at = at.with_timezone(&chrono::Local);
            if at.date_naive() == chrono::Local::now().date_naive() {
                at.format("%H:%M").to_string()
            } else {
                at.format("%b %-d %H:%M").to_string()
            }
        };
        let mut sub = vec!["Lead".to_string()];
        sub.extend(facts.lead_agent.clone());
        sub.push(match task.completed_at {
            Some(at) => format!("finished {}", local(at)),
            None => format!("started {}", local(task.created_at)),
        });
        let elapsed = tree::fmt_time(
            (task.completed_at.unwrap_or_else(Utc::now) - task.created_at)
                .num_milliseconds()
                .max(0) as u64,
        );
        let sweep = (running > 0 && !reduced).then(|| {
            self.shimmer(true, cx);
            let period = SWEEP.mul_f32(motion::speed_scale()).as_secs_f32();
            (now.saturating_duration_since(self.shimmer_since)
                .as_secs_f32()
                / period)
                .fract()
        });
        let segments = steps.iter().map(|row| {
            let id = row.id.clone();
            let selector = format!("task-center-segment-{id}");
            let paint = |status: Status| match status {
                Status::Queued => theme.wash(0.10),
                Status::Stopped => theme.text_faint.opacity(0.35),
                Status::Running => status.tone(theme).opacity(0.45),
                _ => status.tone(theme),
            };
            let color = if self.tree.shown(&row.id) {
                self.tree.tint(&row.id, now, paint)
            } else {
                paint(row.status)
            };
            let lit = motion::mix(color, theme.text, 0.2);
            let tone = row.status.tone(theme);
            let shine = sweep
                .filter(|_| row.status == Status::Running)
                .map(|phase| {
                    let band = |from: Hsla, to: Hsla| {
                        div().h_full().flex_1().bg(gpui::linear_gradient(
                            90.0,
                            gpui::linear_color_stop(from, 0.0),
                            gpui::linear_color_stop(to, 1.0),
                        ))
                    };
                    div()
                        .absolute()
                        .top_0()
                        .h_full()
                        .w_full()
                        .left(relative(-1.0 + 2.0 * phase))
                        .flex()
                        .child(band(tone.opacity(0.0), tone))
                        .child(band(tone, tone.opacity(0.0)))
                });
            div()
                .id(SharedString::from(selector.clone()))
                .debug_selector(move || selector.clone())
                .group(SharedString::from(format!("task-center-segment-{id}")))
                .role(gpui::Role::Button)
                .aria_label(format!("{}: {}", row.name, row.title))
                .flex_1()
                .min_w_0()
                .py(px(6.0))
                .cursor_pointer()
                .tooltip(widgets::text_tooltip(format!(
                    "{} · {} · {}",
                    row.name, row.title, row.word
                )))
                .on_click(cx.listener(move |center, _: &ClickEvent, _, cx| {
                    center.pick(Selection::Step(id.clone()), cx)
                }))
                .child(
                    div()
                        .relative()
                        .h(px(6.0))
                        .rounded(px(3.0))
                        .overflow_hidden()
                        .bg(color)
                        .group_hover(
                            SharedString::from(format!("task-center-segment-{}", row.id)),
                            |s| s.bg(lit),
                        )
                        .children(shine),
                )
        });
        let live = facts.live && !done;
        let phase = (live && !reduced).then(|| tree::amount::phase(self.shimmer_since, now));
        let mut totals = div()
            .flex_none()
            .flex()
            .items_center()
            .gap(px(4.0))
            .text_size(ui_rems(12.0))
            .text_color(theme.text_muted)
            .child(
                div()
                    .font_weight(gpui::FontWeight::MEDIUM)
                    .text_color(theme.text)
                    .child(format!("{finished}/{total}")),
            );
        if !done {
            totals = totals.child("done");
        }
        if running > 0 && !done {
            totals = totals.child("·").child(
                div()
                    .text_color(Status::Running.tone(theme))
                    .child(format!("{running} working")),
            );
        }
        if let Some(label) = tree::amount::label(facts.branch, phase, ui_rems(12.0), theme) {
            totals = totals.child("·").child(label);
        }
        totals = totals.child("·").child(if done {
            format!("took {elapsed}")
        } else {
            elapsed
        });
        div()
            .id("task-center-header")
            .debug_selector(|| "task-center-header".into())
            .flex_none()
            .px(px(20.0))
            .py(px(12.0))
            .border_b_1()
            .border_color(theme.border)
            .flex()
            .flex_col()
            .gap(px(6.0))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(10.0))
                    .child(
                        div()
                            .flex_none()
                            .size(px(28.0))
                            .rounded(px(8.0))
                            .border_1()
                            .border_color(theme.border)
                            .flex()
                            .items_center()
                            .justify_center()
                            .child(
                                icon(icons::MISSION_CONTROL)
                                    .size(px(14.0))
                                    .text_color(theme.text_muted),
                            ),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap(px(8.0))
                                    .min_w_0()
                                    .child(
                                        div()
                                            .min_w_0()
                                            .truncate()
                                            .text_size(ui_rems(18.0))
                                            .font_weight(gpui::FontWeight::SEMIBOLD)
                                            .text_color(theme.text)
                                            .child(SharedString::from(task.title.clone())),
                                    )
                                    .child(self.kind_badge(theme, now)),
                            )
                            .child(
                                div()
                                    .truncate()
                                    .text_size(ui_rems(12.0))
                                    .text_color(theme.text_muted)
                                    .child(sub.join(" · ")),
                            ),
                    )
                    .children(watch)
                    .children(pills)
                    .children(primary)
                    .child(toggle)
                    .child(more),
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
                            .min_w_0()
                            .flex()
                            .gap(px(3.0))
                            .children(segments),
                    )
                    .child(totals),
            )
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
                ..
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
                .aria_label(format!("{label} {name}"))
                .min_h(px(26.0))
                .rounded_full()
            };
            let selection = if lead {
                Selection::Lead
            } else {
                Selection::Step(id.clone())
            };
            let open_thread = |selection: Selection, cx: &Context<Self>| {
                action("Open thread", false, format!("task-center-pop-open-{id}"))
                    .on_click(cx.listener(move |center, _: &ClickEvent, _, cx| {
                        center.pop = None;
                        center.pick(selection.clone(), cx)
                    }))
                    .into_any_element()
            };
            let (first, second): (AnyElement, Option<AnyElement>) = if pop.kind == Status::Waiting {
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
                (open_thread(selection, cx), Some(primary))
            } else if lead {
                (open_thread(selection, cx), None)
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
                    Some(
                        action("Retry", true, format!("task-center-pop-retry-{id}"))
                            .on_click(cx.listener(move |center, _: &ClickEvent, _, cx| {
                                center.pop = None;
                                cx.emit(TaskCenterEvent::RetryStep(retry.clone()))
                            }))
                            .into_any_element(),
                    ),
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
                        .children(second),
                )
        });
        let card = popover::popover_card(&popup)
            .id("task-center-pop")
            .debug_selector(|| "task-center-pop".into())
            .absolute()
            .top(px(34.0 - 4.0 * (1.0 - shown)))
            .right_0()
            .w(px(300.0))
            .opacity(shown)
            .flex()
            .flex_col()
            .on_mouse_down_out(cx.listener(|center, _, _, cx| {
                center.pop_dismissed = center.pop.as_ref().map(|pop| pop.kind);
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

    pub(super) fn facts(
        &mut self,
        task: &zeron_proto::Task,
        plan: &Rollup,
        shown: &[(StepRow, Option<Tag>, Option<Thread>)],
        cx: &mut Context<Self>,
    ) -> Facts {
        self.sync_asks(shown, cx);
        let state = self.state.clone();
        let (lead_chat, lead_agent) = {
            let state = state.read(cx);
            let chat = linked_chat(&state.chats, task);
            (
                chat.map(|chat| chat.id.clone()),
                chat.and_then(|chat| agent_of(chat, cx)),
            )
        };
        let mut chat_ids: Vec<String> = shown
            .iter()
            .filter(|(_, _, thread)| thread.is_some())
            .filter_map(|(row, ..)| row.step.chat_id.clone())
            .collect();
        chat_ids.extend(lead_chat.clone());
        self.stats
            .refresh(&state, chat_ids, |center: &mut Self| &mut center.stats, cx);
        let (lead_status, lead_word) = tree::lead_status(task, plan);
        let mut rows = vec![BarRow {
            id: LEAD.to_string(),
            status: lead_status,
            word: lead_word,
            name: "Lead".into(),
            title: task.title.clone(),
            text: (plan.phase(task) == super::super::PlanPhase::PlanReady)
                .then(|| "Plan ready: approve to start running the steps.".to_string()),
            asks: Some(1),
        }];
        rows.extend(shown.iter().enumerate().map(|(ix, (row, _, thread))| {
            let (status, word) = Status::of_step(
                row,
                thread
                    .as_ref()
                    .and_then(|t| t.delegation.as_ref().map(|d| d.0)),
            );
            let text = self.step_note(row, thread.as_ref());
            BarRow {
                id: row.step.id.clone(),
                status,
                word,
                name: format!("Step {}", ix + 1),
                title: row.step.title.clone(),
                text: text.map(|text| crate::transcript::single_line(&text)),
                asks: self.ask_count(row),
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
        let live = plan.count(StepStatus::Working) > 0
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
        if self
            .pop
            .as_ref()
            .is_some_and(|pop| !rows.iter().any(|row| row.status == pop.kind))
        {
            self.pop = None;
        }
        if self.leaving.as_ref().is_some_and(|leave| {
            leave.grow.settled_at(Instant::now()) && leave.fade.settled_at(Instant::now())
        }) {
            self.leaving = None;
        }
        Facts {
            rows,
            tokens,
            branch,
            live,
            lead_time,
            lead_chat,
            lead_agent,
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn render_spine(
        &mut self,
        mission: Option<SpineMission<'_>>,
        tree_nodes: &[TreeNode],
        reduced: bool,
        now: Instant,
        theme: &Theme,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let editable = mission.as_ref().is_some_and(|m| m.editable);
        let shown: &[(StepRow, Option<Tag>, Option<Thread>)] =
            mission.as_ref().map_or(&[], |m| m.shown);
        let slots = Self::slots(shown);
        let lead_status = match &mission {
            Some(m) => m.facts.rows[0].status,
            None => tree_nodes
                .first()
                .map_or(Status::Queued, |node| node.status),
        };
        let agents: Vec<usize> = (1..tree_nodes.len())
            .filter(|ix| {
                tree_nodes[*ix].parent == Some(0)
                    && matches!(tree_nodes[*ix].kind, NodeKind::Agent | NodeKind::Native)
            })
            .collect();
        let marked = mission.is_some() && !agents.is_empty();

        let mut nodes = vec![NodeIn {
            id: LEAD,
            root: true,
            visible: true,
            status: lead_status,
        }];
        if let Some(m) = &mission {
            for (ix, (row, ..)) in shown.iter().enumerate() {
                nodes.push(NodeIn {
                    id: &row.step.id,
                    root: false,
                    visible: !self.folded,
                    status: m.facts.rows[ix + 1].status,
                });
            }
        }
        nodes.push(NodeIn {
            id: GHOST,
            root: false,
            visible: editable && !self.folded,
            // Not dimmed like a planned step: the ghost is the place to type.
            status: Status::Done,
        });
        nodes.push(NodeIn {
            id: MARK,
            root: false,
            visible: marked && !self.folded,
            status: Status::Done,
        });
        for ix in &agents {
            nodes.push(NodeIn {
                id: &tree_nodes[*ix].id,
                root: false,
                visible: !self.folded,
                status: tree_nodes[*ix].status,
            });
        }
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
        let heights: Vec<f32> = visible
            .iter()
            .map(|ix| {
                let id = &shown[*ix].0.step.id;
                let painted = self
                    .painted
                    .borrow()
                    .get(&Key::Task(id.clone()))
                    .map(|rect| f32::from(rect.size.height));
                match painted {
                    Some(painted) => {
                        (painted - self.last_region(id) + self.region(id, now).height()).max(0.0)
                    }
                    None => CARD_HEIGHT,
                }
            })
            .collect();
        let ghost = self.tree.shown(GHOST);
        let ghost_height = height_of(&Key::Add, 44.0);
        let mut slots_h = heights.clone();
        if ghost {
            slots_h.push(ghost_height);
        }
        let (tops, end) = spine_tops(&slots_h);
        let shown_agents: Vec<usize> = agents
            .iter()
            .copied()
            .filter(|ix| self.tree.shown(&tree_nodes[*ix].id))
            .collect();
        let agent_heights: Vec<f32> = shown_agents
            .iter()
            .map(|ix| height_of(&Key::Task(tree_nodes[*ix].id.clone()), AGENT_HEIGHT))
            .collect();
        let (agent_tops, _) = spine_tops(&agent_heights);
        let snap = reduced || !known || self.sliding(now);
        let lifted = self.drag.as_ref().map(|drag| (drag.id.clone(), drag.dy));
        let mut placed: Vec<Placed> = Vec::new();
        let mut place = |tree: &mut tree::TreeMotion,
                         id: &str,
                         x: f32,
                         y: f32,
                         width: f32,
                         height: f32,
                         side: Option<Side>| {
            let extra = lifted
                .as_ref()
                .filter(|(lifted, _)| lifted == id)
                .map_or((0.0, 0.0), |(_, dy)| (0.0, *dy));
            let (gx, gy) = tree.place(id, (x, y), extra, snap, now);
            let frame = tree.frame(id, now);
            placed.push(Placed {
                id: id.to_string(),
                x: x + gx + extra.0,
                y: y + gy + extra.1 + frame.y,
                width,
                height,
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
            lead_height,
            None,
        );
        for (k, ix) in visible.iter().enumerate() {
            let (side, top) = tops[k];
            let x = match side {
                Side::Left => center - SPINE / 2.0 - column,
                Side::Right => center + SPINE / 2.0,
            };
            place(
                &mut self.tree,
                &shown[*ix].0.step.id,
                x,
                body + top,
                column,
                heights[k],
                Some(side),
            );
        }
        let mut next = body + end;
        if ghost {
            let (side, top) = tops[visible.len()];
            let x = match side {
                Side::Left => center - SPINE / 2.0 - column,
                Side::Right => center + SPINE / 2.0,
            };
            place(
                &mut self.tree,
                GHOST,
                x,
                body + top,
                column,
                ghost_height,
                Some(side),
            );
        }
        if self.tree.shown(MARK) {
            let mark_width = MARK_WIDTH.min(width);
            place(
                &mut self.tree,
                MARK,
                center - mark_width / 2.0,
                next,
                mark_width,
                MARK_HEIGHT,
                None,
            );
            next += MARK_HEIGHT + ROW_GAP + 8.0;
        }
        if mission.is_none() {
            next = body;
        }
        for (k, ix) in shown_agents.iter().enumerate() {
            let (side, top) = agent_tops[k];
            let x = match side {
                Side::Left => center - SPINE / 2.0 - column,
                Side::Right => center + SPINE / 2.0,
            };
            place(
                &mut self.tree,
                &tree_nodes[*ix].id,
                x,
                next + top,
                column,
                agent_heights[k],
                Some(side),
            );
        }
        let bottom = placed
            .iter()
            .map(|placed| placed.y + placed.height)
            .fold(0.0, f32::max);

        let status_of = |id: &str| {
            mission
                .as_ref()
                .and_then(|m| m.facts.rows.iter().find(|row| row.id == id))
                .map(|row| row.status)
                .or_else(|| {
                    tree_nodes
                        .iter()
                        .find(|node| node.id == id)
                        .map(|n| n.status)
                })
                .unwrap_or(Status::Queued)
        };
        let lead = placed.iter().find(|placed| placed.id == LEAD);
        let anchor = lead.map(|lead| (lead.x + lead.width / 2.0, lead.y + lead_height));
        let mut links: Vec<(bool, Link)> = Vec::new();
        let mut leaving = Vec::new();
        if let Some(anchor) = anchor {
            for node in placed.iter().filter(|placed| placed.id != LEAD) {
                let to = match node.side {
                    Some(Side::Left) => (node.x + node.width, node.y + ANCHOR_Y),
                    Some(Side::Right) => (node.x, node.y + ANCHOR_Y),
                    None => (anchor.0, node.y),
                };
                let fixed = node.id == GHOST || node.id == MARK;
                let muted = fixed || status_of(&node.id).muted();
                links.push((
                    muted,
                    Link {
                        from: anchor,
                        to,
                        drawn: node.frame.drawn,
                        color: if fixed {
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
                match &mission {
                    Some(m) => self.render_lead_card(
                        m.task,
                        m.plan,
                        editable,
                        m.facts.branch,
                        m.facts.lead_time.clone(),
                        reduced,
                        theme,
                        window,
                        cx,
                    ),
                    None => self.render_tree_lead_card(tree_nodes, reduced, theme, cx),
                }
            } else if node.id == GHOST {
                self.render_ghost(theme)
            } else if node.id == MARK {
                Self::render_marker(agents.len(), theme)
            } else if let Some(ix) = shown.iter().position(|(row, ..)| row.step.id == node.id) {
                let (row, tag, thread) = &shown[ix];
                let inset = tree_nodes
                    .iter()
                    .position(|tree_node| tree_node.id == node.id)
                    .and_then(|at| self.render_inset(tree_nodes, at, reduced, theme, cx));
                let tokens = mission.as_ref().and_then(|m| m.facts.tokens[ix]);
                self.render_spine_step(
                    ix,
                    row,
                    thread.as_ref(),
                    *tag,
                    move_range(&slots, ix),
                    editable,
                    tokens,
                    inset,
                    reduced,
                    now,
                    theme,
                    window,
                    cx,
                )
            } else if let Some(ix) = tree_nodes.iter().position(|n| n.id == node.id) {
                self.render_agent_card(tree_nodes, ix, reduced, theme, cx)
            } else {
                continue;
            };
            let is_lifted = lifted.as_ref().is_some_and(|(id, _)| *id == node.id);
            let morph_id = match node.id.as_str() {
                LEAD => tree_nodes.first().map(|root| root.id.clone()),
                GHOST | MARK => None,
                id => Some(id.to_string()),
            };
            let wrapper =
                div()
                    .absolute()
                    .left(px(node.x))
                    .top(px(node.y))
                    .w(px(node.width))
                    .opacity(node.frame.opacity)
                    .children(morph_id.map(|id| {
                        morph::measure(&self.views.cards, Key::Node(id), Edges::default())
                    }))
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
        div()
            .flex()
            .flex_col()
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
                        tree::LINK_WIDTH,
                        tree::DOT_RADIUS,
                    ))
                    .children(cards)
                    .child(tree::links(
                        Vec::new(),
                        dots,
                        tree::LINK_WIDTH,
                        tree::DOT_RADIUS,
                    ))
                    .children(self.drop_bar(theme))
                    .on_drag_move::<StepDragPayload>(cx.listener(
                        |center, event: &DragMoveEvent<StepDragPayload>, _, cx| {
                            let drag = event.drag(cx);
                            let (task_id, step_id) = (drag.task_id.clone(), drag.step_id.clone());
                            if center.plan_id(cx).as_deref() != Some(task_id.as_str()) {
                                return;
                            }
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
        let line = div()
            .debug_selector(|| "task-center-drop-line".into())
            .absolute()
            .left(px(left - drag.origin.0))
            .w(px(width))
            .top(px(y - drag.origin.1))
            .h(px(2.0))
            .rounded(px(1.0))
            .bg(theme.accent);
        Some(gpui::deferred(line).priority(1).into_any_element())
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

impl TaskCenter {
    fn render_marker(count: usize, theme: &Theme) -> AnyElement {
        div()
            .debug_selector(|| "task-center-agents-marker".into())
            .h(px(MARK_HEIGHT))
            .flex()
            .justify_center()
            .child(
                div()
                    .h_full()
                    .flex()
                    .items_center()
                    .gap(px(6.0))
                    .px(px(10.0))
                    .rounded_full()
                    .border_1()
                    .border_color(theme.border)
                    .bg(theme.bg)
                    .text_size(ui_rems(11.5))
                    .text_color(theme.text_muted)
                    .child(
                        div()
                            .font_weight(gpui::FontWeight::MEDIUM)
                            .text_color(theme.text)
                            .child("Agents"),
                    )
                    .child(format!(
                        "{count} \u{b7} started by the Lead, not in the plan"
                    )),
            )
            .into_any_element()
    }

    fn node_selection(node: &TreeNode) -> Selection {
        match (node.kind, &node.step_id) {
            (NodeKind::Root, _) => Selection::Lead,
            (NodeKind::Step(_), Some(step)) => Selection::Step(step.clone()),
            _ => Selection::Node(node.id.clone()),
        }
    }

    pub(super) fn render_inset(
        &self,
        nodes: &[TreeNode],
        ix: usize,
        reduced: bool,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let base = nodes[ix].depth;
        let mut below = Vec::new();
        for (k, node) in nodes.iter().enumerate().skip(ix + 1) {
            if node.depth <= base {
                break;
            }
            below.push(k);
        }
        let direct = below
            .iter()
            .filter(|k| nodes[**k].parent == Some(ix))
            .count();
        if direct == 0 {
            return None;
        }
        let rows = below.into_iter().map(|k| {
            let node = &nodes[k];
            let selection = Self::node_selection(node);
            let selector = format!("task-center-inset-{}", node.id);
            let time = {
                let state = self.state.read(cx);
                let stats = node
                    .chat_id
                    .as_deref()
                    .and_then(|id| self.stats.get(state, id));
                tree::thread_time(node.status, node.since, stats.as_ref())
            };
            div()
                .id(SharedString::from(selector.clone()))
                .debug_selector(move || selector.clone())
                .role(gpui::Role::Button)
                .aria_label(format!("{}: {}", node.title, node.word))
                .flex()
                .items_center()
                .gap(px(7.0))
                .h(px(30.0))
                .pl(px(6.0 + 16.0 * (node.depth - base - 1) as f32))
                .pr(px(8.0))
                .rounded(px(6.0))
                .cursor_pointer()
                .hover(|s| s.bg(theme.wash(0.07)))
                .text_size(ui_rems(12.5))
                .text_color(theme.text_muted)
                .child(tree::glyph(
                    node.status,
                    14.0,
                    node.status.tone(theme),
                    reduced,
                ))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .text_size(ui_rems(14.0))
                        .font_weight(gpui::FontWeight::MEDIUM)
                        .text_color(theme.text)
                        .child(node.title.clone()),
                )
                .children(node.brand.map(|(mark, tint)| {
                    icon(mark)
                        .size(px(13.0))
                        .flex_none()
                        .text_color(tint.unwrap_or(theme.text_muted))
                }))
                .child(
                    div()
                        .flex_none()
                        .text_color(node.status.tone(theme))
                        .child(node.word),
                )
                .children(time.map(|time| div().flex_none().child(time)))
                .on_click(cx.listener(move |center, _: &ClickEvent, _, cx| {
                    cx.stop_propagation();
                    center.pick(selection.clone(), cx)
                }))
        });
        Some(
            div()
                .ml(px(30.0))
                .mb(px(2.0))
                .pt(px(4.0))
                .pl(px(4.0))
                .pr(px(2.0))
                .rounded(px(8.0))
                .border_1()
                .border_color(theme.border)
                .bg(theme.wash(0.03))
                .flex()
                .flex_col()
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(6.0))
                        .px(px(6.0))
                        .py(px(2.0))
                        .text_size(ui_rems(11.0))
                        .text_color(theme.text_faint)
                        .child(
                            icon(icons::LUCIDE_LIST_TREE)
                                .size(px(11.0))
                                .text_color(theme.text_faint),
                        )
                        .child(
                            div()
                                .font_weight(gpui::FontWeight::MEDIUM)
                                .text_color(theme.text_muted)
                                .child(format!(
                                    "Started {direct} {}",
                                    plural(direct, "agent", "agents")
                                )),
                        ),
                )
                .children(rows)
                .into_any_element(),
        )
    }

    pub(super) fn render_agent_card(
        &self,
        nodes: &[TreeNode],
        ix: usize,
        reduced: bool,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let node = &nodes[ix];
        let selection = Self::node_selection(node);
        let selected = self.selected == selection;
        let agent = {
            let state = self.state.read(cx);
            node.chat_id
                .as_deref()
                .and_then(|id| state.chats.iter().find(|chat| chat.id == id))
                .and_then(|chat| agent_of(chat, cx))
        };
        let kind = if node.kind == NodeKind::Native {
            "Sub-agent"
        } else {
            "Agent"
        };
        let head = div()
            .flex()
            .items_start()
            .gap(px(8.0))
            .child(Self::avatar(node.brand, theme).mt(px(1.0)))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap(px(2.0))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(4.0))
                            .min_w_0()
                            .text_size(ui_rems(11.0))
                            .text_color(theme.text_muted)
                            .child(
                                div()
                                    .flex_none()
                                    .font_weight(gpui::FontWeight::MEDIUM)
                                    .text_color(theme.text)
                                    .child(kind),
                            )
                            .children(agent.map(|agent| {
                                div().min_w_0().truncate().child(format!("\u{b7} {agent}"))
                            })),
                    )
                    .child(
                        div()
                            .min_w_0()
                            .line_clamp(2)
                            .text_size(ui_rems(13.0))
                            .font_weight(gpui::FontWeight::MEDIUM)
                            .text_color(theme.text)
                            .child(node.title.clone()),
                    ),
            )
            .child(Self::chip(node.status, node.word, reduced, theme));
        let (tokens, time) = {
            let state = self.state.read(cx);
            let stats = node
                .chat_id
                .as_deref()
                .and_then(|id| self.stats.get(state, id));
            (
                tree::amount::of(stats.as_ref()),
                tree::thread_time(node.status, node.since, stats.as_ref()),
            )
        };
        let open = node.chat_id.clone().map(|chat| {
            let selector = format!("task-center-agent-open-{}", node.id);
            div()
                .id(SharedString::from(selector.clone()))
                .debug_selector(move || selector.clone())
                .role(gpui::Role::Button)
                .aria_label("Open")
                .flex_none()
                .flex()
                .items_center()
                .gap(px(4.0))
                .h(px(22.0))
                .pl(px(8.0))
                .pr(px(6.0))
                .rounded(px(6.0))
                .cursor_pointer()
                .text_size(ui_rems(11.5))
                .text_color(theme.text_muted)
                .hover(|s| s.bg(theme.wash(0.08)).text_color(theme.text))
                .child("Open")
                .child(
                    icon(icons::LUCIDE_ARROW_UP_RIGHT)
                        .size(px(12.0))
                        .text_color(theme.text_muted),
                )
                .on_click(cx.listener(move |_, _: &ClickEvent, _, cx| {
                    cx.stop_propagation();
                    cx.emit(TaskCenterEvent::OpenChat(chat.clone()))
                }))
        });
        let footer = div()
            .mt(px(7.0))
            .ml(px(30.0))
            .flex()
            .items_center()
            .gap(px(6.0))
            .min_h(px(22.0))
            .child(self.amount(tokens, node.status == Status::Running, time, reduced, theme))
            .children(open);
        let inset = self.render_inset(nodes, ix, reduced, theme, cx);
        let selector = format!("task-center-agent-{}", node.id);
        self.card(selected, theme)
            .id(SharedString::from(selector.clone()))
            .debug_selector(move || selector.clone())
            .role(gpui::Role::ListItem)
            .aria_label(format!("{kind}: {}", node.title))
            .aria_selected(selected)
            .child(morph::measure(
                &self.painted,
                Key::Task(node.id.clone()),
                Edges::default(),
            ))
            .child(head)
            .children(inset.map(|inset| div().mt(px(7.0)).child(inset)))
            .child(footer)
            .on_click(
                cx.listener(move |center, _: &ClickEvent, _, cx| {
                    center.pick(selection.clone(), cx)
                }),
            )
            .into_any_element()
    }

    pub(super) fn render_tree_lead_card(
        &self,
        nodes: &[TreeNode],
        reduced: bool,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let root = &nodes[0];
        let (agent, can_plan, root_id) = {
            let state = self.state.read(cx);
            let chat = state.chats.iter().find(|chat| chat.id == root.id);
            (
                chat.and_then(|chat| agent_of(chat, cx)),
                chat.is_some_and(|chat| crate::tasks::can_add_plan(state, chat)),
                root.id.clone(),
            )
        };
        let selected = self.selected == Selection::Lead;
        let live = nodes.iter().any(|node| node.status.live());
        let shimmer = self.shimmer(root.status == Status::Running, cx);
        let top = div()
            .flex()
            .items_start()
            .gap(px(8.0))
            .child(Self::avatar(root.brand, theme).mt(px(1.0)))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap(px(2.0))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(4.0))
                            .text_size(ui_rems(11.0))
                            .text_color(theme.text_muted)
                            .child(
                                div()
                                    .font_weight(gpui::FontWeight::MEDIUM)
                                    .text_color(theme.text)
                                    .child("Lead"),
                            )
                            .children(agent.map(|agent| {
                                div().min_w_0().truncate().child(format!("\u{b7} {agent}"))
                            })),
                    )
                    .child(
                        div()
                            .min_w_0()
                            .text_size(ui_rems(13.0))
                            .font_weight(gpui::FontWeight::MEDIUM)
                            .text_color(theme.text)
                            .child(tree::shimmer_text(
                                root.title.clone(),
                                shimmer,
                                ui_rems(13.0),
                                theme.text_muted,
                                theme,
                            )),
                    ),
            )
            .child(Self::chip(root.status, root.word, reduced, theme));
        let add = can_plan.then(|| {
            widgets::action_button(theme, widgets::ActionTone::Filled)
                .id("task-center-lead-add-plan")
                .debug_selector(|| "task-center-lead-add-plan".into())
                .role(gpui::Role::Button)
                .aria_label("Add plan")
                .flex_none()
                .min_h(px(24.0))
                .rounded_full()
                .gap(px(5.0))
                .child(icon(icons::PLUS).size(px(12.0)).text_color(theme.text))
                .child("Add plan")
                .on_click(cx.listener(move |_, _: &ClickEvent, _, cx| {
                    cx.stop_propagation();
                    cx.emit(TaskCenterEvent::AddPlan(root_id.clone()))
                }))
        });
        let well = div()
            .debug_selector(|| "task-center-no-plan".into())
            .ml(px(30.0))
            .px(px(11.0))
            .py(px(9.0))
            .rounded(px(8.0))
            .bg(theme.wash(0.05))
            .border_1()
            .border_color(theme.border)
            .flex()
            .flex_col()
            .gap(px(6.0))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.0))
                    .child(
                        div()
                            .flex_1()
                            .text_size(ui_rems(12.5))
                            .font_weight(gpui::FontWeight::MEDIUM)
                            .text_color(theme.text)
                            .child("No plan"),
                    )
                    .children(add),
            )
            .child(
                div()
                    .text_size(ui_rems(12.0))
                    .text_color(theme.text_muted)
                    .child(
                        "Agents start from the chat as it goes. A plan turns this tree into a mission in place.",
                    ),
            );
        let (tokens, time) = {
            let state = self.state.read(cx);
            let stats = self.stats.get(state, &root.id);
            (
                tree::amount::branch(
                    None,
                    nodes
                        .iter()
                        .filter_map(|node| node.chat_id.as_deref())
                        .map(|id| tree::amount::of(self.stats.get(state, id).as_ref())),
                ),
                tree::thread_time(root.status, root.since, stats.as_ref()),
            )
        };
        let kids = nodes[1..]
            .iter()
            .filter(|node| node.parent == Some(0))
            .count();
        let meta = div()
            .ml(px(30.0))
            .flex()
            .items_center()
            .gap(px(6.0))
            .min_h(px(20.0))
            .child(self.amount(tokens, live, time, reduced, theme))
            .children((kids > 0).then(|| self.fold_button(kids, Instant::now(), theme, cx)));
        self.card(selected, theme)
            .gap(px(7.0))
            .id("task-center-lead")
            .debug_selector(|| "task-center-lead".into())
            .role(gpui::Role::Button)
            .aria_label("Lead")
            .aria_selected(selected)
            .child(morph::measure(&self.painted, Key::Lead, Edges::default()))
            .child(top)
            .child(well)
            .child(meta)
            .on_click(cx.listener(|center, _: &ClickEvent, _, cx| center.pick(Selection::Lead, cx)))
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cards_alternate_sides_one_row_apart_and_a_tall_card_pushes_the_rest() {
        let heights = [96.0, 96.0, 300.0, 96.0, 150.5];
        let (tops, end) = spine_tops(&heights);
        assert_eq!(
            tops,
            [
                (Side::Left, 0.0),
                (Side::Right, 64.0),
                (Side::Left, 128.0),
                (Side::Right, 376.0),
                (Side::Left, 440.0),
            ]
        );
        for i in 0..heights.len() - 2 {
            assert!(
                tops[i + 2].1 >= tops[i].1 + heights[i] + ROW_GAP,
                "card {i} ends above the next card on its side"
            );
        }
        assert_eq!(
            end,
            440.0 + 150.5 + ROW_GAP + 16.0,
            "the ghost sits under the lowest card"
        );
        let (_, grown) = spine_tops(&[96.0, 96.0, 96.4, 96.0]);
        let (_, more) = spine_tops(&[96.0, 96.0, 140.0, 96.0]);
        assert!(
            more > grown,
            "a growing card moves what follows without row steps"
        );
    }
}
