use super::*;
use crate::tasks::{
    Rollup, TreeLink, TreeStatus, kind_badge, link_color, thread_time, tree_glyph, tree_links,
};
use zeron_proto::{ChatIndicator, PlanProgress, StepStatus};

const CARD_INSET: f32 = 2.0;
const CARD_RADIUS: f32 = 9.0;
const HEAD_PAD: f32 = 5.0;
const HEAD_ROW: f32 = 18.0;
const TITLE_ROW: f32 = 20.0;
const BAR_ROW: f32 = 10.0;
const STEP_ROW: f32 = 22.0;
const TREE_X: f32 = 14.0;
const STEP_LEFT: f32 = 24.0;
const GLYPH: f32 = 12.0;

fn step_live(status: StepStatus) -> bool {
    matches!(status, StepStatus::Working | StepStatus::NeedsYou)
}

pub(super) fn live_plan(
    state: &AppState,
    lead: &zeron_proto::Chat,
    now: DateTime<Utc>,
) -> Option<Rollup> {
    let task = zeron_proto::plan_of_chat(&state.tasks, &lead.id)?;
    let plan = crate::tasks::rollup(state, task, now);
    let live = plan.lead == Some(ChatIndicator::Working)
        || plan.steps.iter().any(|row| step_live(row.status));
    live.then_some(plan)
}

struct LiveStep {
    chat_id: String,
    title: SharedString,
    status: TreeStatus,
    word: &'static str,
    since: Option<DateTime<Utc>>,
}

pub(super) struct MissionCard {
    lead_id: String,
    title: SharedString,
    progress: PlanProgress,
    working: usize,
    needs_you: usize,
    planned: usize,
    rest: usize,
    steps: Vec<LiveStep>,
}

impl MissionCard {
    pub(super) fn of(
        state: &AppState,
        lead: &zeron_proto::Chat,
        now: DateTime<Utc>,
    ) -> Option<Self> {
        let plan = live_plan(state, lead, now)?;
        let live: std::collections::HashSet<&str> = plan
            .steps
            .iter()
            .filter(|row| step_live(row.status))
            .map(|row| row.step.id.as_str())
            .collect();
        let steps: Vec<LiveStep> = crate::tasks::tree_rows(state, &lead.id, false, now)
            .into_iter()
            .filter(|node| node.step_id.as_deref().is_some_and(|id| live.contains(id)))
            .filter_map(|node| {
                Some(LiveStep {
                    chat_id: node.chat_id?,
                    title: node.title.into(),
                    status: node.status,
                    word: node.word,
                    since: node.since,
                })
            })
            .collect();
        Some(Self {
            lead_id: lead.id.clone(),
            title: transcript::single_line(&crate::home::chat_title(lead)).into(),
            progress: plan.progress,
            working: plan.count(StepStatus::Working),
            needs_you: plan.count(StepStatus::NeedsYou),
            planned: plan.count(StepStatus::Planned),
            rest: plan.steps.len() - steps.len(),
            steps,
        })
    }

    pub(super) fn top_height() -> f32 {
        2.0 + 2.0 * CARD_INSET + HEAD_PAD + HEAD_ROW + TITLE_ROW + BAR_ROW
    }

    fn lines(&self) -> usize {
        self.steps.len() + usize::from(self.rest > 0)
    }

    pub(super) fn body_height(&self) -> f32 {
        self.lines() as f32 * STEP_ROW
    }

    fn rest_line(&self) -> String {
        let PlanProgress { done, total } = self.progress;
        match self.planned {
            0 => format!("{done}/{total} done"),
            planned => format!("+{planned} planned \u{b7} {done}/{total} done"),
        }
    }

    fn live_line(&self) -> Option<String> {
        (self.working > 0).then(|| format!("{} working", self.working))
    }

    fn needs_line(&self) -> Option<String> {
        (self.needs_you > 0).then(|| format!("{} needs you", self.needs_you))
    }
}

impl Shell {
    pub(super) fn render_sidebar_mission_card(
        &self,
        card: MissionCard,
        open: bool,
        body_height: f32,
        selected: Option<&str>,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let lead_id = card.lead_id.clone();
        let motion_key = format!("lead:{lead_id}");
        let row_id = format!("chat-{lead_id}");
        let lead_selected = selected == Some(lead_id.as_str());
        let live_line = card.live_line();
        let needs_line = card.needs_line();
        let status: SharedString = [live_line.clone(), needs_line.clone()]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join(" \u{b7} ")
            .into();
        let counts = div()
            .min_w_0()
            .overflow_hidden()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(4.0))
            .whitespace_nowrap()
            .text_size(crate::typography::ui_rems(11.0))
            .text_color(theme.text_muted)
            .children(live_line)
            .when(card.working > 0 && card.needs_you > 0, |el| {
                el.child("\u{b7}")
            })
            .children(needs_line.map(|line| div().text_color(theme.warning).child(line)));
        let chevron = (card.lines() > 0).then(|| {
            let lead = lead_id.clone();
            let label = if open {
                "Collapse step threads"
            } else {
                "Expand step threads"
            };
            div()
                .id(SharedString::from(format!("{row_id}-steps-toggle")))
                .debug_selector({
                    let selector = format!("{row_id}-steps-toggle");
                    move || selector.clone()
                })
                .flex_none()
                .size(px(16.0))
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(4.0))
                .cursor_pointer()
                .role(gpui::Role::Button)
                .aria_label(label)
                .aria_expanded(open)
                .hover(|el| el.bg(theme.glass_hover()))
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .on_click(cx.listener(move |this, _, _, cx| {
                    cx.stop_propagation();
                    this.toggle_sidebar_lead(&lead, cx);
                }))
                .tooltip(crate::settings::widgets::text_tooltip_above(label))
                .child(self.turned_disclosure_chevron(&motion_key, open, -0.25, theme))
        });
        let header = div()
            .h(px(HEAD_ROW))
            .flex()
            .flex_row()
            .items_center()
            .gap(px(6.0))
            .justify_between()
            .child(kind_badge(true, theme))
            .child(counts);
        let rename = self.rename_input_for(&lead_id, ChatRenameSurface::Sidebar);
        let title_color = if lead_selected {
            theme.text
        } else {
            theme.text.opacity(0.8)
        };
        let title = div()
            .h(px(TITLE_ROW))
            .flex()
            .flex_row()
            .items_center()
            .gap(px(6.0))
            .child(
                div().flex_1().min_w_0().child(match rename.clone() {
                    Some(input) => chat_title_editor(
                        format!("chat-title-editor-{lead_id}").into(),
                        input,
                        theme,
                    ),
                    None => sidebar_faded_label(
                        format!("chat-title-{lead_id}").into(),
                        true,
                        div()
                            .text_size(crate::typography::ui_rems(13.0))
                            .line_height(px(TITLE_ROW))
                            .font_weight(gpui::FontWeight::MEDIUM)
                            .text_color(title_color)
                            .child(card.title.clone()),
                    )
                    .into_any_element(),
                }),
            )
            .children(chevron);
        let PlanProgress { done, total } = card.progress;
        let fraction = (done as f32 / total.max(1) as f32).min(1.0);
        let (_, _, complete) = chat_status_tone(ChatIndicator::Completed, theme);
        let bar = div().h(px(BAR_ROW)).flex().items_center().child(
            div()
                .flex_1()
                .h(px(3.0))
                .rounded(px(2.0))
                .bg(theme.wash(0.08))
                .overflow_hidden()
                .child(div().h_full().w(gpui::relative(fraction)).bg(complete)),
        );
        let hovered = self.chat_status_hover.as_deref() == Some(row_id.as_str());
        let head = self
            .chat_row_frame(
                &row_id,
                &lead_id,
                &card.title,
                lead_selected,
                Some(status),
                &[],
                None,
                false,
                true,
                rename.is_some(),
                hovered,
                &format!("{row_id}-hover"),
                cx,
            )
            .flex()
            .flex_col()
            .px(px(6.0))
            .pt(px(HEAD_PAD))
            .rounded(px(CARD_RADIUS - 2.0))
            .when(lead_selected, |el| el.bg(crate::theme::glass_selected_bg()))
            .when(!lead_selected, |el| {
                el.hover(|el| el.bg(theme.glass_hover()))
            })
            .child(header)
            .child(title)
            .child(bar);
        let rest_line = (card.rest > 0).then(|| card.rest_line());
        let mut links: Vec<TreeLink> = Vec::with_capacity(card.lines());
        let mut lines: Vec<AnyElement> = Vec::with_capacity(card.lines());
        for (ix, step) in card.steps.into_iter().enumerate() {
            links.push(TreeLink {
                from: (TREE_X, 0.0),
                to: (STEP_LEFT - 3.0, ix as f32 * STEP_ROW + STEP_ROW / 2.0),
                drawn: 1.0,
                color: link_color(false, theme),
            });
            lines.push(self.render_mission_step_line(step, selected, theme, cx));
        }
        if let Some(rest_line) = rest_line {
            let ix = lines.len();
            links.push(TreeLink {
                from: (TREE_X, 0.0),
                to: (STEP_LEFT - 3.0, ix as f32 * STEP_ROW + STEP_ROW / 2.0),
                drawn: 1.0,
                color: link_color(true, theme),
            });
            lines.push(
                div()
                    .h(px(STEP_ROW))
                    .ml(px(STEP_LEFT))
                    .flex()
                    .items_center()
                    .whitespace_nowrap()
                    .text_size(crate::typography::ui_rems(11.0))
                    .text_color(theme.text_muted.opacity(0.7))
                    .child(rest_line)
                    .into_any_element(),
            );
        }
        let body = div()
            .relative()
            .h(px(body_height))
            .child(tree_links(links, Vec::new(), 1.5, 3.0))
            .child(div().flex().flex_col().children(lines))
            .into_any_element();
        let body = self.render_sidebar_disclosure_body(&motion_key, open, body_height, body);
        div()
            .id(SharedString::from(format!("sidebar-mission-{lead_id}")))
            .debug_selector({
                let lead_id = lead_id.clone();
                move || format!("sidebar-mission-{lead_id}")
            })
            .w_full()
            .flex()
            .flex_col()
            .p(px(CARD_INSET))
            .rounded(px(CARD_RADIUS))
            .border_1()
            .border_color(theme.accent.opacity(0.32))
            .bg(theme.accent.opacity(0.06))
            .child(head)
            .child(body)
            .into_any_element()
    }

    fn render_mission_step_line(
        &self,
        step: LiveStep,
        selected: Option<&str>,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let row_id = format!("chat-{}", step.chat_id);
        let is_selected = selected == Some(step.chat_id.as_str());
        let glyph = if step.status == TreeStatus::Running {
            loaders::mini_glyph_spinner(
                format!("{row_id}-working"),
                2.0,
                theme.glyph,
                self.sidebar_pane.entity_id(),
                cx,
            )
            .into_any_element()
        } else {
            tree_glyph(
                step.status,
                GLYPH,
                step.status.tone(theme),
                self.reduced_motion,
            )
        };
        let rename = self.rename_input_for(&step.chat_id, ChatRenameSurface::Sidebar);
        let title = match rename.clone() {
            Some(input) => chat_title_editor(
                format!("chat-title-editor-{}", step.chat_id).into(),
                input,
                theme,
            ),
            None => sidebar_faded_label(
                format!("chat-title-{}", step.chat_id).into(),
                true,
                div()
                    .text_size(crate::typography::ui_rems(12.0))
                    .text_color(if is_selected {
                        theme.text
                    } else {
                        theme.text.opacity(0.75)
                    })
                    .child(step.title.clone()),
            )
            .into_any_element(),
        };
        let time = thread_time(step.status, step.since, None);
        let hovered = self.chat_status_hover.as_deref() == Some(row_id.as_str());
        self.chat_row_frame(
            &row_id,
            &step.chat_id,
            &step.title,
            is_selected,
            Some(step.word.into()),
            &[],
            None,
            false,
            true,
            rename.is_some(),
            hovered,
            &format!("{row_id}-hover"),
            cx,
        )
        .h(px(STEP_ROW))
        .ml(px(STEP_LEFT - 4.0))
        .pl(px(4.0))
        .pr(px(6.0))
        .flex()
        .flex_row()
        .items_center()
        .gap(px(6.0))
        .rounded(px(6.0))
        .when(is_selected, |el| el.bg(crate::theme::glass_selected_bg()))
        .when(!is_selected, |el| el.hover(|el| el.bg(theme.glass_hover())))
        .child(
            div()
                .flex_none()
                .size(px(GLYPH))
                .flex()
                .items_center()
                .justify_center()
                .child(glyph),
        )
        .child(title)
        .children(time.map(|time| {
            div()
                .flex_none()
                .whitespace_nowrap()
                .text_size(crate::typography::ui_rems(11.0))
                .text_color(theme.text_muted.opacity(0.7))
                .child(time)
        }))
        .into_any_element()
    }
}
