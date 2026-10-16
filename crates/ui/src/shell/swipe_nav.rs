use super::*;
use crate::swipe::{self, Step, SwipeDir};

const SHIFT_MAX: f32 = 28.0;
const SHIFT_MAX_BLOCKED: f32 = 14.0;
const PILL_HIDDEN: f32 = -56.0;
const PILL_SHOWN: f32 = 14.0;
const PILL_SIZE: f32 = 40.0;
const RING_SIZE: f32 = 48.0;
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
    release: Option<(f32, std::time::Instant)>,
    enter: Option<(SwipeDir, std::time::Instant)>,
    watchdog: Option<Task<()>>,
}

pub(super) struct SwipeLayers {
    pub(super) shift: f32,
    pub(super) alpha: f32,
    pub(super) pill: Option<AnyElement>,
}

struct SwipePill {
    dir: SwipeDir,
    offset: f32,
    progress: f32,
    armed: bool,
    label: Option<SharedString>,
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
                    self.swipe.release = Some((travel, now));
                } else {
                    if !armed {
                        crate::haptics::selection_step();
                    }
                    self.step_history(dir, cx);
                }
            }
            Step::Cancel => {
                self.swipe.armed = false;
                if let Some(travel) = self.swipe.travel.take() {
                    self.swipe.release = Some((travel, now));
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
            pill: None,
        };
        if let Some(travel) = self.swipe.travel {
            let dir = SwipeDir::of(travel);
            let target = self.swipe_target(dir);
            frame.shift = Self::swipe_shift(travel, target.is_some());
            let (offset, progress) = Self::pill_at(travel, target.is_some());
            let label = match target {
                Some(entry) if self.swipe.armed => Some(self.nav_entry_title(entry, cx)),
                Some(_) => None,
                None if swipe::armed(travel) => Some(SharedString::from(match dir {
                    SwipeDir::Back => "No previous page",
                    SwipeDir::Forward => "No next page",
                })),
                None => None,
            };
            frame.pill = Some(self.render_swipe_pill(
                SwipePill {
                    dir,
                    offset,
                    progress,
                    armed: self.swipe.armed,
                    label,
                },
                right_inset,
                cx,
            ));
        } else if let Some((travel, started)) = self.swipe.release
            && !self.reduced_motion
            && let Some(t) = self.tween_t(started, SPRING_BACK)
        {
            let dir = SwipeDir::of(travel);
            let has_target = self.swipe_target(dir).is_some();
            let (offset, progress) = Self::pill_at(travel, has_target);
            frame.shift = motion::lerp(Self::swipe_shift(travel, has_target), 0.0, t);
            frame.pill = Some(self.render_swipe_pill(
                SwipePill {
                    dir,
                    offset: motion::lerp(offset, PILL_HIDDEN, t),
                    progress,
                    armed: false,
                    label: None,
                },
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

    fn swipe_shift(travel: f32, has_target: bool) -> f32 {
        let max = if has_target {
            SHIFT_MAX
        } else {
            SHIFT_MAX_BLOCKED
        };
        SwipeDir::of(travel).sign() * swipe::rubber(travel, max)
    }

    fn pill_at(travel: f32, has_target: bool) -> (f32, f32) {
        let progress = (travel.abs() / swipe::ARM_DISTANCE).min(1.0);
        let reach = if has_target {
            progress
        } else {
            progress * 0.75
        };
        let eased = 1.0 - (1.0 - reach).powi(3);
        let ring = if has_target { progress } else { 0.0 };
        (motion::lerp(PILL_HIDDEN, PILL_SHOWN, eased), ring)
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
                .map(|chat| transcript::single_line(&crate::home::chat_title(chat)).into())
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
        }
    }

    fn render_swipe_pill(&self, pill: SwipePill, right_inset: f32, cx: &App) -> AnyElement {
        let theme = Theme::of(cx).for_popup();
        let glass = theme.is_frost();
        let labeled = pill.label.is_some();
        let open = motion::state_t(
            "swipe-pill-open",
            labeled,
            motion::SNAP,
            self.reduced_motion,
        );
        let (bg, fg) = if pill.armed {
            (theme.accent, theme.on_accent)
        } else if labeled {
            (popover::surface_bg(&theme), theme.text_faint)
        } else if glass {
            (popover::surface_bg(&theme), theme.text)
        } else {
            (theme.surface_raised, theme.text)
        };
        let arrow = match pill.dir {
            SwipeDir::Back => icons::ARROW_LEFT,
            SwipeDir::Forward => icons::ARROW_RIGHT,
        };
        let label = div()
            .max_w(px(240.0 * open))
            .overflow_hidden()
            .whitespace_nowrap()
            .text_ellipsis()
            .text_size(crate::typography::ui_rems(13.0))
            .text_color(fg)
            .children(pill.label);
        let body = div()
            .h(px(PILL_SIZE))
            .min_w(px(PILL_SIZE))
            .px(px(12.0))
            .rounded_full()
            .border_1()
            .border_color(if pill.armed {
                gpui::transparent_black()
            } else {
                theme.border
            })
            .when(!glass, |el| el.shadow_md())
            .bg(bg)
            .flex()
            .items_center()
            .justify_center()
            .gap(px(8.0 * open))
            .when(pill.dir == SwipeDir::Forward, |el| el.flex_row_reverse())
            .child(icon(arrow).size(px(16.0)).text_color(fg))
            .child(label);
        let ring = (!pill.armed && !labeled).then(|| {
            div()
                .absolute()
                .top(px((PILL_SIZE - RING_SIZE) / 2.0))
                .left(px((PILL_SIZE - RING_SIZE) / 2.0))
                .child(crate::context_usage::progress_ring(
                    RING_SIZE,
                    RING_SIZE / 2.0 - 2.0,
                    2.0,
                    pill.progress,
                    theme.accent,
                    theme.text_faint.opacity(0.25),
                ))
        });
        div()
            .absolute()
            .top(gpui::relative(0.5))
            .mt(px(-PILL_SIZE / 2.0))
            .map(|el| match pill.dir {
                SwipeDir::Back => el.left(px(pill.offset)),
                SwipeDir::Forward => el.right(px(pill.offset + right_inset)),
            })
            .child(crate::frost::frosted(
                PILL_SIZE / 2.0,
                crate::frost::MENU_BLUR,
                div().relative().child(body).children(ring),
            ))
            .into_any_element()
    }
}
