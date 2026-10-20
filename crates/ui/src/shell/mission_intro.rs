use super::*;
use crate::tasks::{SampleRow, StripTree, TaskCenter, TreeStatus, tree_glyph};
use std::time::Instant;
use zeron_proto::TaskMode;

const PAGE_IN: motion::MotionSpec = motion::MotionSpec::new(250, motion::EASE_OUT);
const PAGE_SHIFT: f32 = 8.0;
const VISUAL_HEIGHT: f32 = 172.0;
const TREE_PAGE: usize = 2;
const TEXT_HEIGHT: f32 = 23.0 + 8.0 + 4.0 * 21.0;

struct Page {
    title: &'static str,
    body: &'static str,
}

const PAGES: [Page; 4] = [
    Page {
        title: "Start with a goal",
        body: "Press Shift+Tab in the composer, or pick Mission Control on a task\u{2019}s card, and describe what you want done. Add up to five chats for the lead to read before it plans.",
    },
    Page {
        title: "Approve the plan",
        body: "A lead agent turns your goal into steps you can edit, reorder and add images to. By default nothing runs until you click Approve and run.",
    },
    Page {
        title: "Watch every step",
        body: "Each step runs on its own thread, side by side when steps don\u{2019}t depend on each other. The tree on the lead\u{2019}s thread follows them live, and the Mission Control page adds each step\u{2019}s token count.",
    },
    Page {
        title: "A lead and a Supervisor watch",
        body: "The lead reviews each step\u{2019}s work when it finishes and sends it back if something is missing. The Supervisor gathers what needs you into one sheet per mission, and one Send answers it all.",
    },
];

#[derive(Debug, Clone, PartialEq)]
pub(super) enum MissionIntro {
    Pending,
    Open {
        page: usize,
        forward: bool,
        since: Option<Instant>,
        tree: Option<Entity<StripTree>>,
    },
    Closed,
}

impl MissionIntro {
    pub(super) fn at_boot(acknowledged: bool) -> Self {
        if std::env::var("ZERON_OPEN_DIALOG").as_deref() == Ok("mission-intro") {
            Self::open()
        } else if acknowledged {
            Self::Closed
        } else {
            Self::Pending
        }
    }

    fn open() -> Self {
        Self::Open {
            page: 0,
            forward: true,
            since: None,
            tree: None,
        }
    }

    pub(super) fn resolve(state: &AppState) -> Option<Self> {
        if !state.tasks_synced {
            return None;
        }
        let started = state
            .tasks
            .iter()
            .any(|task| task.mode == TaskMode::Orchestrated);
        Some(if started { Self::Closed } else { Self::open() })
    }
}

impl Shell {
    pub(super) fn resolve_mission_intro(&mut self, cx: &mut Context<Self>) {
        if self.mission_intro != MissionIntro::Pending {
            return;
        }
        match MissionIntro::resolve(self.state.read(cx)) {
            Some(MissionIntro::Closed) => self.close_mission_intro(cx),
            Some(intro) => self.mission_intro = intro,
            None => {}
        }
    }

    pub(super) fn mission_intro_visible(&self) -> bool {
        matches!(self.mission_intro, MissionIntro::Open { .. })
            && !self.appshot_welcome
            && self.whats_new_dialog.is_none()
            && !self.sync_flow.has_visible_overlay()
    }

    pub(super) fn reopen_mission_intro(&mut self, cx: &mut Context<Self>) {
        self.mission_intro = MissionIntro::open();
        cx.notify();
    }

    pub(super) fn close_mission_intro(&mut self, cx: &mut Context<Self>) {
        self.mission_intro = MissionIntro::Closed;
        if !self.settings.mission_intro_acknowledged {
            self.settings.mission_intro_acknowledged = true;
            self.schedule_save(cx);
        }
        cx.notify();
    }

    pub(super) fn page_mission_intro(&mut self, delta: isize, cx: &mut Context<Self>) {
        let reduced = motion::reduced_motion(cx);
        let MissionIntro::Open { page, .. } = self.mission_intro else {
            return;
        };
        let next = page.saturating_add_signed(delta).min(PAGES.len() - 1);
        if next == page {
            return;
        }
        self.mission_intro = MissionIntro::Open {
            page: next,
            forward: next > page,
            since: (!reduced).then(Instant::now),
            tree: None,
        };
        cx.notify();
    }

    fn try_mission_control(&mut self, cx: &mut Context<Self>) {
        self.close_mission_intro(cx);
        self.new_mission(cx);
    }

    pub(super) fn new_mission(&mut self, cx: &mut Context<Self>) {
        self.open_new_session(None, cx);
        self.composer.update(cx, |composer, cx| {
            composer.mission_pending = true;
            composer.focus_pending = true;
            cx.notify();
        });
    }

    fn mission_intro_tree(&mut self, cx: &mut Context<Self>) -> Entity<StripTree> {
        if let MissionIntro::Open {
            tree: Some(tree), ..
        } = &self.mission_intro
        {
            return tree.clone();
        }
        let state = self.state.clone();
        let tree = cx.new(|cx| StripTree::sample(state, sample_rows(), cx));
        if let MissionIntro::Open { tree: slot, .. } = &mut self.mission_intro {
            *slot = Some(tree.clone());
        }
        tree
    }

    pub(super) fn render_mission_intro(
        &mut self,
        viewport: gpui::Size<Pixels>,
        theme: &Theme,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let MissionIntro::Open {
            page,
            forward,
            since,
            ..
        } = self.mission_intro
        else {
            return div().into_any_element();
        };
        let visual = match page {
            TREE_PAGE => div()
                .size_full()
                .pt(px(8.0))
                .child(self.mission_intro_tree(cx))
                .into_any_element(),
            _ => intro_visual(page, theme),
        };
        let progress = since.map_or(1.0, |since| {
            let total = PAGE_IN.total().as_secs_f32() * motion::speed_scale();
            let raw = since.elapsed().as_secs_f32() / total.max(f32::EPSILON);
            if raw < 1.0 {
                window.request_animation_frame();
            }
            PAGE_IN.progress(raw)
        });
        let shift = (1.0 - progress) * PAGE_SHIFT * if forward { 1.0 } else { -1.0 };
        let last = page + 1 == PAGES.len();
        let content = div()
            .relative()
            .left(px(shift))
            .opacity(progress)
            .child(
                div()
                    .mb(px(20.0))
                    .h(px(VISUAL_HEIGHT))
                    .overflow_hidden()
                    .rounded(px(14.0))
                    .border_1()
                    .border_color(theme.border)
                    .bg(crate::theme::ink(0.04))
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_size(crate::typography::ui_rems(12.5))
                    .text_color(theme.text_muted)
                    .child(visual),
            )
            .child(
                div()
                    .min_h(px(TEXT_HEIGHT))
                    .child(popover::welcome_title(theme, PAGES[page].title))
                    .child(popover::welcome_body(theme, PAGES[page].body)),
            );
        let dots = div()
            .id("mission-intro-pages")
            .role(gpui::Role::Group)
            .aria_label(SharedString::from(format!(
                "Page {} of {}",
                page + 1,
                PAGES.len()
            )))
            .flex_1()
            .flex()
            .items_center()
            .gap(px(5.0))
            .children((0..PAGES.len()).map(|dot| {
                div().size(px(6.0)).rounded_full().bg(if dot == page {
                    theme.text
                } else {
                    crate::theme::ink(0.18)
                })
            }));
        let next = if last {
            popover::welcome_action(theme, "mission-intro-try", "Try Mission Control", true)
                .on_click(cx.listener(|this, _, _, cx| this.try_mission_control(cx)))
        } else {
            popover::welcome_action(theme, "mission-intro-next", "Next", true)
                .on_click(cx.listener(|this, _, _, cx| this.page_mission_intro(1, cx)))
        };
        let card = popover::welcome_sheet(theme, "mission-intro-card", PAGES[page].title)
            .child(content)
            .child(
                div()
                    .mt(px(14.0))
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(8.0))
                    .child(dots)
                    .child(
                        popover::welcome_action(theme, "mission-intro-dismiss", "Not now", false)
                            .on_click(cx.listener(|this, _, _, cx| this.close_mission_intro(cx))),
                    )
                    .child(next),
            )
            .into_any_element();
        popover::modal_glass("mission-intro-sheet", viewport, card, 20.0)
    }
}

fn intro_chip(theme: &Theme, label: &'static str, icon: Option<&'static str>) -> gpui::Div {
    div()
        .h(px(22.0))
        .px(px(8.0))
        .flex()
        .items_center()
        .gap(px(5.0))
        .rounded_full()
        .border_1()
        .border_color(theme.accent.opacity(0.5))
        .bg(theme.accent.opacity(0.16))
        .text_size(crate::typography::ui_rems(11.5))
        .text_color(theme.text)
        .when_some(icon, |el, path| {
            el.child(icons::icon(path).size(px(12.0)).text_color(theme.text))
        })
        .child(label)
}

fn brand(model: &str) -> Option<(&'static str, Option<gpui::Hsla>)> {
    Some(crate::pickers::agent_brand_icon(None, Some(model)))
}

fn sample_rows() -> Vec<SampleRow> {
    let row = |title, status, word, brand, secs| SampleRow {
        title,
        status,
        word,
        brand,
        secs,
    };
    vec![
        row(
            "Watching 1 step",
            TreeStatus::Running,
            "Working",
            brand("claude-opus"),
            Some(95),
        ),
        row(
            "Scaffold the app",
            TreeStatus::Done,
            "Done",
            brand("gpt-5"),
            None,
        ),
        row(
            "Product grid with filters",
            TreeStatus::Running,
            "Working",
            brand("claude-sonnet"),
            Some(42),
        ),
        row(
            "Cart with persistence",
            TreeStatus::Waiting,
            "Needs you",
            brand("gpt-5"),
            Some(18),
        ),
        row("Final check", TreeStatus::Queued, "Planned", None, None),
    ]
}

fn read_chip(theme: &Theme, model: &str, title: &'static str) -> gpui::Div {
    let (_, _, done) = chat_status_tone(zeron_proto::ChatIndicator::Completed, theme);
    div()
        .h(px(24.0))
        .px(px(8.0))
        .flex()
        .items_center()
        .gap(px(5.0))
        .rounded_full()
        .bg(theme.wash(0.07))
        .text_size(crate::typography::ui_rems(11.5))
        .text_color(theme.text)
        .children(brand(model).map(|(mark, tint)| {
            icons::icon(mark)
                .size(px(12.0))
                .text_color(tint.unwrap_or(theme.text_muted))
        }))
        .child(div().size(px(6.0)).rounded_full().bg(done))
        .child(title)
}

fn mini_solid(theme: &Theme, label: &'static str) -> gpui::Div {
    div()
        .flex_none()
        .h(px(22.0))
        .px(px(9.0))
        .flex()
        .items_center()
        .rounded_full()
        .bg(theme.solid)
        .text_size(crate::typography::ui_rems(11.0))
        .font_weight(gpui::FontWeight::MEDIUM)
        .text_color(theme.on_solid)
        .child(label)
}

fn plan_card(theme: &Theme, number: usize, title: &'static str) -> gpui::Div {
    div()
        .flex_none()
        .flex()
        .flex_col()
        .pt(px(9.0))
        .px(px(10.0))
        .pb(px(7.0))
        .rounded(px(10.0))
        .border_1()
        .border_color(theme.border)
        .bg(theme.surface_card)
        .child(
            div()
                .flex()
                .items_start()
                .gap(px(8.0))
                .child(TaskCenter::avatar(None, theme).mt(px(1.0)))
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
                                .gap(px(4.0))
                                .text_size(crate::typography::ui_rems(11.0))
                                .text_color(theme.text_muted)
                                .child(
                                    div()
                                        .font_weight(gpui::FontWeight::MEDIUM)
                                        .text_color(theme.text)
                                        .child(format!("Step {number}")),
                                )
                                .child("\u{b7} Not started"),
                        )
                        .child(
                            div()
                                .truncate()
                                .text_size(crate::typography::ui_rems(13.0))
                                .font_weight(gpui::FontWeight::MEDIUM)
                                .text_color(theme.text)
                                .child(title),
                        ),
                )
                .child(TaskCenter::chip(TreeStatus::Queued, "Planned", true, theme)),
        )
        .child(
            div().mt(px(5.0)).ml(px(30.0)).flex().justify_end().child(
                div()
                    .h(px(20.0))
                    .pl(px(8.0))
                    .pr(px(6.0))
                    .flex()
                    .items_center()
                    .gap(px(4.0))
                    .text_size(crate::typography::ui_rems(11.5))
                    .text_color(theme.text_muted)
                    .child("Spec")
                    .child(
                        icons::icon(icons::LUCIDE_CHEVRON_RIGHT)
                            .size(px(12.0))
                            .text_color(theme.text_muted),
                    ),
            ),
        )
}

fn plan_visual(theme: &Theme) -> AnyElement {
    let waiting = TreeStatus::Waiting.tone(theme);
    let pill = div()
        .flex_none()
        .h(px(22.0))
        .pl(px(7.0))
        .pr(px(9.0))
        .flex()
        .items_center()
        .gap(px(4.0))
        .rounded_full()
        .bg(waiting.opacity(0.15))
        .text_size(crate::typography::ui_rems(11.0))
        .font_weight(gpui::FontWeight::MEDIUM)
        .text_color(waiting)
        .child(tree_glyph(TreeStatus::Waiting, 11.0, waiting, true))
        .child("Needs you \u{b7} plan review");
    let track = div()
        .flex()
        .items_center()
        .gap(px(8.0))
        .child(div().flex_1().flex().gap(px(3.0)).children((0..3).map(|_| {
            div()
                .flex_1()
                .h(px(5.0))
                .rounded(px(2.5))
                .bg(theme.wash(0.10))
        })))
        .child(
            div()
                .flex()
                .gap(px(3.0))
                .text_size(crate::typography::ui_rems(11.0))
                .child(
                    div()
                        .font_weight(gpui::FontWeight::MEDIUM)
                        .text_color(theme.text)
                        .child("0/3"),
                )
                .child("done"),
        );
    div()
        .size_full()
        .px(px(12.0))
        .pt(px(14.0))
        .flex()
        .flex_col()
        .gap(px(9.0))
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(6.0))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .text_size(crate::typography::ui_rems(12.5))
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .text_color(theme.text)
                        .child("Ship the shop"),
                )
                .child(pill)
                .child(mini_solid(theme, "Approve and run")),
        )
        .child(track)
        .child(plan_card(theme, 1, "Scaffold the app"))
        .child(plan_card(theme, 2, "Product grid with filters"))
        .into_any_element()
}

fn sheet_row(
    theme: &Theme,
    question: &'static str,
    options: [&'static str; 2],
    picked: Option<usize>,
) -> gpui::Div {
    let segments = div()
        .flex_none()
        .flex()
        .items_center()
        .h(px(22.0))
        .p(px(2.0))
        .rounded(px(7.0))
        .bg(theme.wash(0.06))
        .border_1()
        .border_color(theme.border)
        .children(options.into_iter().enumerate().map(|(ix, option)| {
            let chosen = picked == Some(ix);
            div()
                .h(px(16.0))
                .px(px(8.0))
                .flex()
                .items_center()
                .rounded(px(5.0))
                .text_size(crate::typography::ui_rems(11.5))
                .when(chosen, |el| {
                    el.bg(theme.solid)
                        .text_color(theme.on_solid)
                        .font_weight(gpui::FontWeight::MEDIUM)
                })
                .when(!chosen, |el| el.text_color(theme.text))
                .child(option)
        }));
    div()
        .h(px(34.0))
        .pl(px(10.0))
        .pr(px(8.0))
        .flex()
        .items_center()
        .gap(px(8.0))
        .border_t_1()
        .border_color(crate::settings::widgets::row_divider(theme))
        .child(div().size(px(6.0)).rounded_full().bg(theme.warning))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .truncate()
                .text_size(crate::typography::ui_rems(12.0))
                .font_weight(gpui::FontWeight::MEDIUM)
                .text_color(theme.text)
                .child(question),
        )
        .child(segments)
}

fn supervisor_visual(theme: &Theme) -> AnyElement {
    let (_, _, watching) = chat_status_tone(zeron_proto::ChatIndicator::Completed, theme);
    let (_, _, working) = chat_status_tone(zeron_proto::ChatIndicator::Working, theme);
    let status = div()
        .flex()
        .items_center()
        .gap(px(6.0))
        .text_size(crate::typography::ui_rems(11.5))
        .text_color(theme.text_muted)
        .child(div().size(px(7.0)).rounded_full().bg(watching))
        .child(
            div()
                .font_weight(gpui::FontWeight::MEDIUM)
                .text_color(watching)
                .child("Watching"),
        )
        .child("1 mission \u{b7} 2 of 12 wakes this hour");
    let header = div()
        .h(px(30.0))
        .pl(px(10.0))
        .pr(px(10.0))
        .flex()
        .items_center()
        .gap(px(7.0))
        .text_size(crate::typography::ui_rems(11.5))
        .child(crate::loaders::upload_progress_arc(
            50,
            12.0,
            theme.text_faint.opacity(0.35),
            working,
        ))
        .child(
            div()
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .text_color(theme.text)
                .child("Ship the shop"),
        )
        .child(div().text_color(theme.warning).child("2 to answer"));
    let footer = div()
        .h(px(34.0))
        .pl(px(10.0))
        .pr(px(6.0))
        .flex()
        .items_center()
        .gap(px(8.0))
        .border_t_1()
        .border_color(crate::settings::widgets::row_divider(theme))
        .text_size(crate::typography::ui_rems(11.5))
        .text_color(theme.text_faint)
        .child(div().flex_1().child("1 of 2 picked"))
        .child(
            div()
                .h(px(22.0))
                .px(px(9.0))
                .flex()
                .items_center()
                .rounded(px(7.0))
                .bg(theme.solid)
                .text_color(theme.on_solid)
                .font_weight(gpui::FontWeight::MEDIUM)
                .child("Send 1 to the Lead"),
        );
    div()
        .size_full()
        .px(px(14.0))
        .pt(px(12.0))
        .flex()
        .flex_col()
        .gap(px(8.0))
        .child(status)
        .child(
            div()
                .rounded(px(10.0))
                .border_1()
                .border_color(theme.border)
                .bg(crate::settings::widgets::block_fill(theme))
                .overflow_hidden()
                .flex()
                .flex_col()
                .child(header)
                .child(sheet_row(
                    theme,
                    "Use a Stripe test account?",
                    ["Approve", "Use the mock"],
                    Some(0),
                ))
                .child(sheet_row(
                    theme,
                    "Ship on Friday?",
                    ["Yes", "Next week"],
                    None,
                ))
                .child(footer),
        )
        .into_any_element()
}

fn intro_visual(page: usize, theme: &Theme) -> AnyElement {
    match page {
        0 => div()
            .flex()
            .flex_col()
            .items_center()
            .gap(px(16.0))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(10.0))
                    .child(intro_chip(
                        theme,
                        "Mission Control",
                        Some(icons::MISSION_CONTROL),
                    ))
                    .child(
                        div()
                            .text_size(crate::typography::ui_rems(10.5))
                            .text_color(theme.text_faint)
                            .child("\u{21e7}Tab"),
                    ),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.0))
                    .child(div().mr(px(4.0)).child("Reads"))
                    .child(read_chip(theme, "claude-opus", "Checkout notes"))
                    .child(read_chip(theme, "gpt-5", "Pricing research")),
            )
            .into_any_element(),
        1 => plan_visual(theme),
        _ => supervisor_visual(theme),
    }
}

#[cfg(test)]
mod tests {
    use super::MissionIntro;
    use crate::state::AppState;
    use zeron_proto::TaskMode;

    fn task(mode: TaskMode) -> zeron_proto::Task {
        serde_json::from_value(serde_json::json!({
            "id": "t1",
            "title": "Ship the shop",
            "mode": mode,
        }))
        .unwrap()
    }

    #[test]
    fn the_intro_waits_for_tasks_and_skips_once_a_mission_exists() {
        let mut state = AppState::default();
        assert_eq!(MissionIntro::resolve(&state), None);

        state.apply_tasks(Vec::new());
        assert!(matches!(
            MissionIntro::resolve(&state),
            Some(MissionIntro::Open { page: 0, .. })
        ));

        state.apply_tasks(vec![task(TaskMode::Single)]);
        assert!(matches!(
            MissionIntro::resolve(&state),
            Some(MissionIntro::Open { .. })
        ));

        state.apply_tasks(vec![task(TaskMode::Orchestrated)]);
        assert_eq!(MissionIntro::resolve(&state), Some(MissionIntro::Closed));
    }
}
