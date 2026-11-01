use super::*;
use crate::swipe::{self, Step, SwipeDir};

const DOT: f32 = 56.0;
const DOT_HIDDEN: f32 = -30.0;
const DOT_REACH: f32 = 44.0;
const DOT_REACH_BLOCKED: f32 = 6.0;
const DOT_OVERSHOOT: f32 = 10.0;
const DOT_ARMED_GROWTH: f32 = 0.1;
const DOT_POP_SCALE: f32 = 0.12;
const ENTER_OFFSET: f32 = 32.0;
const ARM_BOUNCE: motion::MotionSpec = motion::MotionSpec::new(200, motion::EASE_BOUNCE);
const POP: motion::MotionSpec = motion::MotionSpec::new(200, motion::EASE_TAILWIND);
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
    pop: Option<(SwipeDir, f32, std::time::Instant)>,
    watchdog: Option<Task<()>>,
}

pub(super) struct SwipeLayers {
    pub(super) shift: f32,
    pub(super) alpha: f32,
    pub(super) dot: Option<AnyElement>,
}

impl SwipeNav {
    pub(super) fn yield_to_slide(&mut self) {
        self.enter = None;
        self.pop = None;
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
                    self.swipe.pop = None;
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
                    self.step_history(dir, cx);
                    if self.swipe.enter.is_none() {
                        self.swipe.release = Some((travel, true, now));
                    } else if !self.reduced_motion {
                        self.swipe.pop = Some((dir, Self::dot_center(travel, true), now));
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
            dot: None,
        };
        if let Some(travel) = self.swipe.travel {
            let has_target = self.swipe_target(SwipeDir::of(travel)).is_some();
            frame.dot = Some(self.render_swipe_dot(
                SwipeDir::of(travel),
                Self::dot_center(travel, has_target),
                1.0,
                self.swipe.armed,
                right_inset,
                cx,
            ));
        } else if let Some((travel, has_target, started)) = self.swipe.release
            && !self.reduced_motion
            && let Some(t) = self.tween_t(started, SPRING_BACK)
        {
            frame.dot = Some(self.render_swipe_dot(
                SwipeDir::of(travel),
                motion::lerp(Self::dot_center(travel, has_target), DOT_HIDDEN, t),
                1.0,
                false,
                right_inset,
                cx,
            ));
        }
        if let Some((dir, center, started)) = self.swipe.pop
            && let Some(t) = self.tween_t(started, POP)
        {
            frame.dot = Some(self.render_swipe_dot(
                dir,
                center,
                motion::lerp(1.0, DOT_POP_SCALE, t),
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
                    frame.shift = -dir.sign() * ENTER_OFFSET * (1.0 - t);
                }
            }
        }
        frame
    }

    fn dot_center(travel: f32, has_target: bool) -> f32 {
        let progress = (travel.abs() / swipe::ARM_DISTANCE).min(1.0);
        let eased = 1.0 - (1.0 - progress).powi(3);
        if !has_target {
            return motion::lerp(DOT_HIDDEN, DOT_REACH_BLOCKED, eased);
        }
        let past = travel.abs() - swipe::ARM_DISTANCE;
        let overshoot = if past > 0.0 {
            swipe::rubber(past, DOT_OVERSHOOT)
        } else {
            0.0
        };
        motion::lerp(DOT_HIDDEN, DOT_REACH, eased) + overshoot
    }

    fn render_swipe_dot(
        &self,
        dir: SwipeDir,
        center: f32,
        shrink: f32,
        armed: bool,
        right_inset: f32,
        cx: &App,
    ) -> AnyElement {
        let theme = Theme::of(cx).for_popup();
        let glass = theme.is_frost();
        let lit = motion::state_t("swipe-dot-armed", armed, ARM_BOUNCE, self.reduced_motion);
        let size = DOT * (1.0 + DOT_ARMED_GROWTH * lit) * shrink;
        let alpha = 1.0 - ((1.0 - shrink) / (1.0 - DOT_POP_SCALE) - 0.55).clamp(0.0, 0.45) / 0.45;
        let chevron = match dir {
            SwipeDir::Back => icons::LUCIDE_CHEVRON_LEFT,
            SwipeDir::Forward => icons::LUCIDE_CHEVRON_RIGHT,
        };
        let body = div()
            .size(px(size))
            .rounded_full()
            .border_1()
            .border_color(theme.border)
            .when(!glass, |el| el.shadow_md())
            .bg(if glass {
                popover::surface_bg(&theme)
            } else {
                theme.surface_raised
            })
            .relative()
            .flex()
            .items_center()
            .justify_center()
            .child(
                div()
                    .absolute()
                    .inset_0()
                    .rounded_full()
                    .bg(theme.wash(0.08)),
            )
            .child(
                icon(chevron)
                    .size(px((20.0 + 3.0 * lit) * shrink))
                    .text_color(theme.text.opacity(0.45 + 0.55 * lit.min(1.0))),
            );
        let edge = center - size / 2.0;
        div()
            .absolute()
            .top(gpui::relative(0.5))
            .mt(px(-size / 2.0))
            .map(|el| match dir {
                SwipeDir::Back => el.left(px(edge)),
                SwipeDir::Forward => el.right(px(edge + right_inset)),
            })
            .opacity(alpha)
            .child(crate::frost::frosted(
                size / 2.0,
                crate::frost::MENU_BLUR * alpha,
                body,
            ))
            .into_any_element()
    }
}
