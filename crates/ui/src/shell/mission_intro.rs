use super::*;
use std::time::Instant;
use zeron_proto::TaskMode;

const PAGE_IN: motion::MotionSpec = motion::MotionSpec::new(250, motion::EASE_OUT);
const PAGE_SHIFT: f32 = 8.0;
const VISUAL_HEIGHT: f32 = 96.0;
const TEXT_HEIGHT: f32 = 23.0 + 8.0 + 4.0 * 21.0;

struct Page {
    title: &'static str,
    body: &'static str,
}

const PAGES: [Page; 3] = [
    Page {
        title: "Start with a goal",
        body: "Switch the composer to Mission Control and describe what you want done. A lead agent turns it into a plan you can edit, and by default nothing runs until you approve it.",
    },
    Page {
        title: "Every step gets its own thread",
        body: "Each step runs on its own thread, and steps that don\u{2019}t depend on each other can run at the same time. The strip at the top of each thread takes you back to the whole mission.",
    },
    Page {
        title: "A lead and a Supervisor watch",
        body: "The lead reviews each step\u{2019}s work when it finishes and sends it back if something is missing. The Supervisor tells you when a mission needs you.",
    },
];

#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) enum MissionIntro {
    Pending,
    Open {
        page: usize,
        forward: bool,
        since: Option<Instant>,
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
        };
        cx.notify();
    }

    fn try_mission_control(&mut self, cx: &mut Context<Self>) {
        self.close_mission_intro(cx);
        self.open_new_session(None, cx);
        self.composer.update(cx, |composer, cx| {
            composer.mission_pending = true;
            composer.focus_pending = true;
            cx.notify();
        });
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
        } = self.mission_intro
        else {
            return div().into_any_element();
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
                    .rounded(px(14.0))
                    .border_1()
                    .border_color(theme.border)
                    .bg(crate::theme::ink(0.04))
                    .flex()
                    .items_center()
                    .justify_center()
                    .gap(px(10.0))
                    .text_size(crate::typography::ui_rems(12.5))
                    .text_color(theme.text_muted)
                    .children(intro_visual(page, theme)),
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

fn intro_visual(page: usize, theme: &Theme) -> Vec<AnyElement> {
    match page {
        0 => vec![
            intro_chip(theme, "Mission Control", Some(icons::MISSION_CONTROL)).into_any_element(),
            div()
                .text_size(crate::typography::ui_rems(10.5))
                .text_color(theme.text_faint)
                .child("⇧Tab")
                .into_any_element(),
        ],
        1 => {
            let (_, _, complete) = chat_status_tone(zeron_proto::ChatIndicator::Completed, theme);
            vec![
                div()
                    .w(px(340.0))
                    .h(px(30.0))
                    .px(px(10.0))
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .rounded(px(10.0))
                    .bg(theme.wash(0.04))
                    .border_1()
                    .border_color(theme.border)
                    .text_size(crate::typography::ui_rems(12.0))
                    .text_color(theme.text_muted)
                    .child(crate::loaders::upload_progress_arc(
                        25,
                        13.0,
                        theme.text_faint.opacity(0.6),
                        complete,
                    ))
                    .child(
                        div()
                            .font_weight(gpui::FontWeight::MEDIUM)
                            .text_color(theme.text)
                            .child("Ship the shop"),
                    )
                    .child(div().whitespace_nowrap().child("Step 2 of 4"))
                    .child(
                        div()
                            .flex_1()
                            .h(px(3.0))
                            .rounded(px(2.0))
                            .bg(theme.wash(0.08))
                            .child(div().h_full().w(gpui::relative(0.25)).bg(complete)),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(2.0))
                            .whitespace_nowrap()
                            .text_color(theme.text)
                            .child("Mission Control")
                            .child(
                                icons::icon(icons::LUCIDE_CHEVRON_RIGHT)
                                    .size(px(12.0))
                                    .text_color(theme.text_muted),
                            ),
                    )
                    .into_any_element(),
            ]
        }
        _ => vec![
            intro_chip(theme, "Lead", None).into_any_element(),
            div().child("→ reviews each step →").into_any_element(),
            intro_chip(theme, "Supervisor", None).into_any_element(),
        ],
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
