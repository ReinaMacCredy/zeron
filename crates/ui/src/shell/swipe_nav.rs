use super::*;
use crate::swipe::{self, Step, SwipeDir};

const SHIFT_MAX_BLOCKED: f32 = 14.0;
const PEEK_MAX: f32 = 220.0;
const PEEK_INNER: f32 = 560.0;
const PEEK_PARALLAX: f32 = 0.3;
const ENTER_OFFSET: f32 = 32.0;
const SPRING_BACK: motion::MotionSpec = motion::MotionSpec::new(180, motion::EASE_OUT);
const ENTER: motion::MotionSpec = motion::MotionSpec::new(260, motion::EASE_OUT_QUINT);
const ENTER_REDUCED: motion::MotionSpec = motion::MotionSpec::new(150, motion::EASE);
const WATCHDOG_TICK: Duration = Duration::from_millis(250);

#[derive(Default)]
pub(super) struct SwipeNav {
    input: swipe::Swipe,
    travel: Option<f32>,
    armed: bool,
    release: Option<(f32, bool, std::time::Instant)>,
    enter: Option<(SwipeDir, std::time::Instant)>,
    expand: Option<(SwipeDir, f32, SharedString, std::time::Instant)>,
    watchdog: Option<Task<()>>,
}

pub(super) struct SwipeLayers {
    pub(super) shift: f32,
    pub(super) alpha: f32,
    pub(super) peek: Option<AnyElement>,
}

impl SwipeNav {
    pub(super) fn yield_to_slide(&mut self) {
        self.enter = None;
        self.expand = None;
    }
}

fn demo_wheel(
    cx: &mut gpui::AsyncWindowContext,
    at: gpui::Point<gpui::Pixels>,
    phase: gpui::TouchPhase,
    travel: f32,
) {
    let _ = cx.update(|window, cx| {
        window.dispatch_event(
            gpui::PlatformInput::ScrollWheel(gpui::ScrollWheelEvent {
                position: at,
                delta: gpui::ScrollDelta::Pixels(gpui::point(px(travel), px(0.0))),
                modifiers: gpui::Modifiers::default(),
                touch_phase: phase,
            }),
            cx,
        );
    });
}

impl Shell {
    pub(super) fn swipe_layer(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let shell = cx.entity().downgrade();
        swipe::wheel_layer(move |event, phase, blocked, _, cx| {
            let _ = shell.update(cx, |shell, cx| {
                let blocked = phase == gpui::DispatchPhase::Bubble
                    && (blocked || cx.has_active_drag() || !swipe::enabled());
                let now = shell.motion_time();
                let step = shell.swipe.input.wheel(event, phase, blocked, now, cx);
                shell.apply_swipe(step, now, cx);
            });
        })
    }

    pub(super) fn step_history(&mut self, dir: SwipeDir, cx: &mut Context<Self>) {
        let moves = match dir {
            SwipeDir::Back => self.nav.can_back(),
            SwipeDir::Forward => self.nav.can_forward(),
        };
        if moves {
            self.swipe.travel = None;
            self.swipe.armed = false;
            self.swipe.release = None;
            self.swipe.enter = Some((dir, self.motion_time()));
        }
        match dir {
            SwipeDir::Back => self.navigate_back(cx),
            SwipeDir::Forward => self.navigate_forward(cx),
        }
    }

    // `ZERON_OPEN_DIALOG=swipe` capture knob: trackpad input never reaches a Dev bundle.
    pub(super) fn run_swipe_demo(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        cx.spawn_in(window, async move |this, cx| {
            let mut sessions = Vec::new();
            while sessions.len() < 2 {
                cx.background_executor().timer(Duration::from_secs(1)).await;
                let Ok(order) = this.update(cx, |shell, cx| {
                    let mut order = shell.sidebar_visible_order(cx);
                    order.truncate(2);
                    order
                }) else {
                    return;
                };
                sessions = order;
            }
            let mut at = gpui::point(px(0.0), px(0.0));
            for id in sessions {
                let Ok(point) = this.update(cx, |shell, cx| {
                    shell.navigate_to(Some(id), true, cx);
                    gpui::point(
                        px(shell.sidebar_edge() + 160.0),
                        px(shell.viewport_height / 2.0),
                    )
                }) else {
                    return;
                };
                at = point;
                cx.background_executor()
                    .timer(Duration::from_millis(1500))
                    .await;
            }
            for (travel, steps, every, hold) in [
                (60.0, 6, 50, 4000),
                (150.0, 10, 50, 4000),
                (-60.0, 3, 16, 0),
                (-150.0, 10, 50, 4000),
            ] {
                cx.background_executor().timer(Duration::from_secs(2)).await;
                demo_wheel(cx, at, gpui::TouchPhase::Started, 0.0);
                for _ in 0..steps {
                    cx.background_executor()
                        .timer(Duration::from_millis(every))
                        .await;
                    demo_wheel(cx, at, gpui::TouchPhase::Moved, travel / steps as f32);
                }
                cx.background_executor()
                    .timer(Duration::from_millis(hold))
                    .await;
                demo_wheel(cx, at, gpui::TouchPhase::Ended, 0.0);
            }
        })
        .detach();
    }

    fn swipe_target(&self, dir: SwipeDir) -> Option<&NavEntry> {
        match dir {
            SwipeDir::Back => self.nav.previous(),
            SwipeDir::Forward => self.nav.next(),
        }
    }

    fn apply_swipe(&mut self, step: Step, now: std::time::Instant, cx: &mut Context<Self>) {
        match step {
            Step::Pass => return,
            Step::Track(travel) => {
                let armed =
                    swipe::armed(travel) && self.swipe_target(SwipeDir::of(travel)).is_some();
                if armed && !self.swipe.armed {
                    crate::haptics::selection_step();
                }
                if self.swipe.travel.is_none() {
                    self.swipe.expand = None;
                    self.start_swipe_watchdog(cx);
                }
                self.swipe.armed = armed;
                self.swipe.travel = Some(travel);
                self.swipe.release = None;
            }
            Step::Commit(dir) => {
                let travel = self.swipe.travel.take().unwrap_or_default();
                let armed = std::mem::take(&mut self.swipe.armed);
                if self.swipe_target(dir).is_none() {
                    self.swipe.release = Some((travel, false, now));
                } else {
                    if !armed {
                        crate::haptics::selection_step();
                    }
                    let title = self
                        .swipe_target(dir)
                        .map(|entry| self.nav_entry_title(entry, cx));
                    self.step_history(dir, cx);
                    if self.swipe.enter.is_none() {
                        self.swipe.release = Some((travel, true, now));
                    } else if !self.reduced_motion {
                        self.swipe.expand =
                            title.map(|title| (dir, Self::peek_width(travel), title, now));
                    }
                }
            }
            Step::Cancel => {
                self.swipe.armed = false;
                if let Some(travel) = self.swipe.travel.take() {
                    let has_target = self.swipe_target(SwipeDir::of(travel)).is_some();
                    self.swipe.release = Some((travel, has_target, now));
                }
            }
        }
        cx.notify();
    }

    fn start_swipe_watchdog(&mut self, cx: &mut Context<Self>) {
        self.swipe.watchdog = Some(cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(WATCHDOG_TICK).await;
                let live = this
                    .update(cx, |shell, cx| {
                        let now = shell.motion_time();
                        if shell.swipe.input.tracker.expire(now) {
                            shell.apply_swipe(Step::Cancel, now, cx);
                        }
                        shell.swipe.input.tracker.swiping()
                    })
                    .unwrap_or(false);
                if !live {
                    break;
                }
            }
        }));
    }

    fn tween_t(&self, started: std::time::Instant, spec: motion::MotionSpec) -> Option<f32> {
        let total = spec.total().mul_f32(motion::speed_scale());
        let raw = self.tween_elapsed(started).as_secs_f32() / total.as_secs_f32();
        if raw >= 1.0 {
            return None;
        }
        self.motion_active.set(true);
        Some(spec.curve.eval(raw))
    }

    pub(super) fn swipe_layers(&self, right_inset: f32, cx: &App) -> SwipeLayers {
        let mut frame = SwipeLayers {
            shift: 0.0,
            alpha: 1.0,
            peek: None,
        };
        if let Some(travel) = self.swipe.travel {
            let dir = SwipeDir::of(travel);
            match self.swipe_target(dir) {
                Some(entry) => {
                    let width = Self::peek_width(travel);
                    frame.shift = dir.sign() * width * PEEK_PARALLAX;
                    frame.peek = Some(self.render_swipe_peek(
                        dir,
                        width,
                        1.0,
                        self.nav_entry_title(entry, cx),
                        self.swipe.armed,
                        right_inset,
                        cx,
                    ));
                }
                None => {
                    frame.shift = dir.sign() * swipe::rubber(travel, SHIFT_MAX_BLOCKED);
                    frame.peek =
                        Some(self.render_swipe_blocked(dir, swipe::armed(travel), right_inset, cx));
                }
            }
        } else if let Some((travel, has_target, started)) = self.swipe.release
            && !self.reduced_motion
            && let Some(t) = self.tween_t(started, SPRING_BACK)
        {
            let dir = SwipeDir::of(travel);
            match self.swipe_target(dir).filter(|_| has_target) {
                Some(entry) => {
                    let width = Self::peek_width(travel);
                    frame.shift = motion::lerp(dir.sign() * width * PEEK_PARALLAX, 0.0, t);
                    frame.peek = Some(self.render_swipe_peek(
                        dir,
                        motion::lerp(width, 0.0, t),
                        1.0,
                        self.nav_entry_title(entry, cx),
                        false,
                        right_inset,
                        cx,
                    ));
                }
                None => {
                    frame.shift = motion::lerp(
                        dir.sign() * swipe::rubber(travel, SHIFT_MAX_BLOCKED),
                        0.0,
                        t,
                    );
                }
            }
        }
        if let Some((dir, from, title, started)) = &self.swipe.expand
            && let Some(t) = self.tween_t(*started, ENTER)
        {
            let full = (self.viewport_width - self.sidebar_edge() - right_inset).max(*from);
            frame.peek = Some(self.render_swipe_peek(
                *dir,
                motion::lerp(*from, full, t),
                1.0 - t,
                title.clone(),
                true,
                right_inset,
                cx,
            ));
        }
        if let Some((dir, started)) = self.swipe.enter {
            let spec = if self.reduced_motion {
                ENTER_REDUCED
            } else {
                ENTER
            };
            if let Some(t) = self.tween_t(started, spec) {
                frame.alpha = t;
                if !self.reduced_motion {
                    frame.shift -= dir.sign() * ENTER_OFFSET * (1.0 - t);
                }
            }
        }
        frame
    }

    fn peek_width(travel: f32) -> f32 {
        let progress = (travel.abs() / swipe::ARM_DISTANCE).min(1.0);
        PEEK_MAX * (1.0 - (1.0 - progress).powi(3))
    }

    fn nav_entry_title(&self, entry: &NavEntry, cx: &App) -> SharedString {
        match entry {
            NavEntry::Chat(id) if id.is_empty() => "New session".into(),
            NavEntry::Chat(id) => self
                .state
                .read(cx)
                .chats
                .iter()
                .find(|chat| &chat.id == id)
                .map(|chat| transcript::single_line(&chat.display_title()).into())
                .unwrap_or_else(|| "New session".into()),
            NavEntry::Draft(id) => self
                .state
                .read(cx)
                .is_draft(id)
                .then(|| crate::draft_store::entry_prompt(id, cx))
                .flatten()
                .map(|prompt| transcript::single_line(&prompt).into())
                .unwrap_or_else(|| "New session".into()),
            NavEntry::PullRequests => "Pull requests".into(),
            NavEntry::PullRequest { url, .. } => url
                .rsplit('/')
                .next()
                .filter(|number| number.parse::<u64>().is_ok())
                .map(|number| format!("Pull request #{number}").into())
                .unwrap_or_else(|| "Pull request".into()),
            NavEntry::Settings(_) => "Settings".into(),
            NavEntry::Usage => "Usage".into(),
            NavEntry::Inbox => "Inbox".into(),
            NavEntry::Tasks => "Tasks".into(),
            NavEntry::TaskCenter(id) => self
                .state
                .read(cx)
                .tasks
                .iter()
                .find(|task| &task.id == id)
                .map(|task| transcript::single_line(&task.title).into())
                .unwrap_or_else(|| "Task".into()),
            NavEntry::Supervisor => "Supervisor".into(),
            NavEntry::Docs(article) => article
                .and_then(crate::docs::article)
                .map_or_else(|| "Docs".into(), |article| article.title.clone().into()),
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn render_swipe_peek(
        &self,
        dir: SwipeDir,
        width: f32,
        alpha: f32,
        title: SharedString,
        armed: bool,
        right_inset: f32,
        cx: &App,
    ) -> AnyElement {
        let theme = Theme::of(cx);
        let lit = motion::state_t("swipe-peek-armed", armed, motion::SNAP, self.reduced_motion);
        let line = |fraction: f32| {
            div()
                .h(px(10.0))
                .w(gpui::relative(fraction))
                .rounded_full()
                .bg(theme.wash(0.07))
        };
        let page = div()
            .absolute()
            .top_0()
            .bottom_0()
            .left_0()
            .w(px(PEEK_INNER))
            .px(px(40.0))
            .pt(px(Theme::TITLEBAR_HEIGHT + 16.0))
            .flex()
            .flex_col()
            .gap(px(10.0))
            .opacity(0.55 + 0.45 * lit)
            .child(
                div()
                    .mb(px(8.0))
                    .truncate()
                    .text_size(crate::typography::ui_rems(17.0))
                    .font_weight(gpui::FontWeight::MEDIUM)
                    .text_color(theme.text)
                    .child(title),
            )
            .child(line(0.9))
            .child(line(0.75))
            .child(line(0.82));
        let edge = div()
            .absolute()
            .top_0()
            .bottom_0()
            .w(px(2.0))
            .bg(theme.accent)
            .opacity(lit)
            .map(|el| match dir {
                SwipeDir::Back => el.right_0(),
                SwipeDir::Forward => el.left_0(),
            });
        let panel = div()
            .absolute()
            .top_0()
            .bottom_0()
            .w(px(width))
            .overflow_hidden()
            .opacity(alpha)
            .bg(if theme.is_frost() {
                popover::surface_bg(theme)
            } else {
                theme.bg
            })
            .border_color(theme.border)
            .shadow_lg()
            .map(|el| match dir {
                SwipeDir::Back => el.left_0().border_r_1(),
                SwipeDir::Forward => el.right(px(right_inset)).border_l_1(),
            })
            .child(page)
            .child(edge);
        crate::frost::frosted(0.0, crate::frost::MENU_BLUR * alpha, panel).into_any_element()
    }

    fn render_swipe_blocked(
        &self,
        dir: SwipeDir,
        armed: bool,
        right_inset: f32,
        cx: &App,
    ) -> AnyElement {
        let theme = Theme::of(cx);
        let shown = motion::state_t("swipe-blocked", armed, motion::SNAP, self.reduced_motion);
        div()
            .absolute()
            .top(gpui::relative(0.5))
            .mt(px(-9.0))
            .map(|el| match dir {
                SwipeDir::Back => el.left(px(16.0)),
                SwipeDir::Forward => el.right(px(16.0 + right_inset)),
            })
            .opacity(shown)
            .text_size(crate::typography::ui_rems(13.0))
            .text_color(theme.text_faint)
            .child(match dir {
                SwipeDir::Back => "No previous page",
                SwipeDir::Forward => "No next page",
            })
            .into_any_element()
    }
}
