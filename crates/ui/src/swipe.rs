use std::cell::Cell;
use std::collections::VecDeque;
use std::time::{Duration, Instant};

use gpui::{
    App, DispatchPhase, HitboxBehavior, IntoElement, Point, ScrollDelta, ScrollHandle,
    ScrollWheelEvent, Styled, TouchPhase, Window, canvas, point, px,
};

use crate::overscroll::FLING_GAP;

pub(crate) const ARM_DISTANCE: f32 = 110.0;
const FLICK_SPEED: f32 = 500.0;
const FLICK_MIN_DISTANCE: f32 = 24.0;
const AXIS_SLOP: f32 = 8.0;
const AXIS_RATIO: f32 = 1.5;
const VELOCITY_WINDOW: Duration = Duration::from_millis(100);
// Events are timed when handled, so a batch handled after a stall must not read as a flick.
const SPEED_SPAN_FLOOR: Duration = Duration::from_millis(50);
// The first momentum event may queue behind the destination's first render.
const MOMENTUM_START: Duration = Duration::from_secs(1);
const STALL_CANCEL: Duration = Duration::from_millis(500);
const IDLE_CANCEL: Duration = Duration::from_secs(8);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SwipeDir {
    Back,
    Forward,
}

impl SwipeDir {
    pub(crate) fn of(travel: f32) -> Self {
        if travel >= 0.0 {
            Self::Back
        } else {
            Self::Forward
        }
    }

    pub(crate) fn sign(self) -> f32 {
        match self {
            Self::Back => 1.0,
            Self::Forward => -1.0,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Step {
    Pass,
    Track(f32),
    Commit(SwipeDir),
    Cancel,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
enum Phase {
    #[default]
    Idle,
    Touch {
        travel: Point<f32>,
    },
    Content,
    Swipe {
        travel: f32,
        stalled: bool,
    },
    Coast {
        swiped: bool,
        tail: bool,
    },
}

#[derive(Debug, Default)]
pub(crate) struct Tracker {
    phase: Phase,
    samples: VecDeque<(Instant, f32)>,
    last_event: Option<Instant>,
}

impl Tracker {
    pub(crate) fn swiping(&self) -> bool {
        matches!(self.phase, Phase::Swipe { .. })
    }

    fn owns(&self, phase: TouchPhase, now: Instant) -> bool {
        match self.phase {
            Phase::Swipe { .. } => true,
            Phase::Coast { swiped, tail } => {
                swiped && phase == TouchPhase::Moved && self.within(now, tail)
            }
            _ => false,
        }
    }

    fn within(&self, now: Instant, tail: bool) -> bool {
        let gap = if tail { FLING_GAP } else { MOMENTUM_START };
        self.last_event
            .is_some_and(|last| now.saturating_duration_since(last) <= gap)
    }

    fn yield_touch(&mut self) {
        if let Phase::Touch { .. } = self.phase {
            self.phase = Phase::Content;
        }
    }

    pub(crate) fn feed(
        &mut self,
        phase: TouchPhase,
        delta: Point<f32>,
        claimed: bool,
        now: Instant,
    ) -> Step {
        let coasting = match self.phase {
            Phase::Coast { tail, .. } => self.within(now, tail),
            _ => false,
        };
        self.last_event = Some(now);
        let still = delta.x == 0.0 && delta.y == 0.0;
        match (phase, self.phase) {
            (TouchPhase::Started, Phase::Swipe { .. }) => {
                self.phase = Phase::Touch {
                    travel: Point::default(),
                };
                Step::Cancel
            }
            (TouchPhase::Started, Phase::Touch { .. }) => self.touch(delta, claimed, now),
            (TouchPhase::Started, _) => {
                self.phase = Phase::Touch {
                    travel: Point::default(),
                };
                self.touch(delta, claimed, now)
            }
            (TouchPhase::Moved, Phase::Touch { travel }) if still && travel == Point::default() => {
                self.phase = Phase::Idle;
                Step::Pass
            }
            (TouchPhase::Moved, Phase::Touch { .. }) => self.touch(delta, claimed, now),
            // NSEventPhaseCancelled arrives as a still `Moved`; silence after it confirms.
            (TouchPhase::Moved, Phase::Swipe { travel, .. }) if still => {
                self.phase = Phase::Swipe {
                    travel,
                    stalled: true,
                };
                Step::Track(travel)
            }
            (TouchPhase::Moved, Phase::Swipe { travel, .. }) => {
                let travel = travel + delta.x;
                self.phase = Phase::Swipe {
                    travel,
                    stalled: false,
                };
                self.sample(now, travel);
                Step::Track(travel)
            }
            (TouchPhase::Moved, Phase::Coast { swiped, .. }) if coasting => {
                self.phase = Phase::Coast { swiped, tail: true };
                Step::Pass
            }
            (TouchPhase::Moved, Phase::Coast { .. }) => {
                self.phase = Phase::Idle;
                Step::Pass
            }
            (TouchPhase::Moved, _) => Step::Pass,
            (TouchPhase::Ended, Phase::Swipe { travel, .. }) => {
                let travel = travel + delta.x;
                self.sample(now, travel);
                let speed = self.speed(now);
                self.phase = Phase::Coast {
                    swiped: true,
                    tail: false,
                };
                if commits(travel, speed) {
                    Step::Commit(SwipeDir::of(travel))
                } else {
                    Step::Cancel
                }
            }
            (TouchPhase::Ended, _) => {
                self.phase = Phase::Coast {
                    swiped: false,
                    tail: false,
                };
                Step::Pass
            }
            (TouchPhase::Cancelled, Phase::Swipe { .. }) => {
                self.phase = Phase::Idle;
                Step::Cancel
            }
            (TouchPhase::Cancelled, _) => {
                self.phase = Phase::Idle;
                Step::Pass
            }
        }
    }

    pub(crate) fn expire(&mut self, now: Instant) -> bool {
        let Phase::Swipe { stalled, .. } = self.phase else {
            return false;
        };
        let limit = if stalled { STALL_CANCEL } else { IDLE_CANCEL };
        let idle = self
            .last_event
            .is_none_or(|last| now.saturating_duration_since(last) >= limit);
        if idle {
            self.phase = Phase::Idle;
        }
        idle
    }

    fn touch(&mut self, delta: Point<f32>, claimed: bool, now: Instant) -> Step {
        let Phase::Touch { mut travel } = self.phase else {
            return Step::Pass;
        };
        travel.x += delta.x;
        travel.y += delta.y;
        if claimed {
            self.phase = Phase::Content;
            return Step::Pass;
        }
        if travel.x.abs() + travel.y.abs() < AXIS_SLOP {
            self.phase = Phase::Touch { travel };
            return Step::Pass;
        }
        if travel.x.abs() <= AXIS_RATIO * travel.y.abs() {
            self.phase = Phase::Content;
            return Step::Pass;
        }
        self.phase = Phase::Swipe {
            travel: travel.x,
            stalled: false,
        };
        self.samples.clear();
        self.sample(now, travel.x);
        Step::Track(travel.x)
    }

    fn sample(&mut self, now: Instant, travel: f32) {
        self.samples.push_back((now, travel));
        while self
            .samples
            .front()
            .is_some_and(|(at, _)| now.saturating_duration_since(*at) > VELOCITY_WINDOW)
        {
            self.samples.pop_front();
        }
    }

    fn speed(&self, now: Instant) -> f32 {
        let (Some((first_at, first)), Some((_, last))) =
            (self.samples.front(), self.samples.back())
        else {
            return 0.0;
        };
        let span = now
            .saturating_duration_since(*first_at)
            .max(SPEED_SPAN_FLOOR);
        (last - first) / span.as_secs_f32()
    }
}

fn commits(travel: f32, speed: f32) -> bool {
    armed(travel)
        || (travel.abs() >= FLICK_MIN_DISTANCE
            && speed.abs() >= FLICK_SPEED
            && speed.signum() == travel.signum())
}

pub(crate) fn armed(travel: f32) -> bool {
    travel.abs() >= ARM_DISTANCE
}

pub(crate) fn enabled() -> bool {
    #[cfg(all(target_os = "macos", not(test)))]
    return objc2_app_kit::NSEvent::isSwipeTrackingFromScrollEventsEnabled();
    #[cfg(test)]
    return true;
    #[cfg(all(not(target_os = "macos"), not(test)))]
    return false;
}

// A live swipe takes its events in capture, before anything under the pointer
// scrolls; any other event is decided in bubble, after nested scrollers could claim it.
// An event that never reaches bubble was consumed underneath, so its touch is content.
#[derive(Debug, Default)]
pub(crate) struct Swipe {
    pub(crate) tracker: Tracker,
    handled: bool,
    unseen: bool,
}

impl Swipe {
    pub(crate) fn wheel(
        &mut self,
        event: &ScrollWheelEvent,
        phase: DispatchPhase,
        blocked: bool,
        now: Instant,
        cx: &mut App,
    ) -> Step {
        let ScrollDelta::Pixels(delta) = event.delta else {
            return Step::Pass;
        };
        let delta = point(f32::from(delta.x), f32::from(delta.y));
        match phase {
            DispatchPhase::Capture => {
                if std::mem::take(&mut self.unseen) {
                    self.tracker.yield_touch();
                }
                self.handled = self.tracker.owns(event.touch_phase, now);
                if !self.handled {
                    self.unseen = true;
                    return Step::Pass;
                }
                if event.touch_phase == TouchPhase::Moved {
                    cx.stop_propagation();
                }
                self.tracker.feed(event.touch_phase, delta, false, now)
            }
            DispatchPhase::Bubble => {
                self.unseen = false;
                if std::mem::take(&mut self.handled) {
                    return Step::Pass;
                }
                self.tracker.feed(event.touch_phase, delta, blocked, now)
            }
        }
    }
}

thread_local! { static CLAIMED: Cell<bool> = const { Cell::new(false) }; }

pub(crate) fn claim_any(_: &ScrollWheelEvent, _: &mut Window, _: &mut App) {
    CLAIMED.with(|claimed| claimed.set(true));
}

pub(crate) fn claim_scrollable(
    handle: &ScrollHandle,
) -> impl Fn(&ScrollWheelEvent, &mut Window, &mut App) + 'static {
    let handle = handle.clone();
    move |event, window, _| {
        let delta = event.delta.pixel_delta(window.line_height());
        if delta.x.abs() <= delta.y.abs() {
            return;
        }
        // GPUI's own listener already added this delta; it clamps only at prepaint.
        let before = handle.offset().x - delta.x;
        let room = if delta.x > px(0.0) {
            before < px(0.0)
        } else {
            before > -handle.max_offset().x
        };
        if room {
            CLAIMED.with(|claimed| claimed.set(true));
        }
    }
}

// Must paint before the content it covers: its listener then runs first in
// capture and last in bubble.
pub(crate) fn wheel_layer(
    listener: impl Fn(&ScrollWheelEvent, DispatchPhase, bool, &mut Window, &mut App) + 'static,
) -> impl IntoElement {
    canvas(
        |bounds, window, _| window.insert_hitbox(bounds, HitboxBehavior::Normal),
        move |_, hitbox, window, _| {
            window.on_mouse_event(move |event: &ScrollWheelEvent, phase, window, cx| {
                let claimed = CLAIMED.with(|claimed| claimed.replace(false))
                    && phase == DispatchPhase::Bubble;
                listener(
                    event,
                    phase,
                    claimed || !hitbox.should_handle_scroll(window),
                    window,
                    cx,
                );
            });
        },
    )
    .absolute()
    .top_0()
    .left_0()
    .size_full()
}

pub(crate) fn rubber(travel: f32, max: f32) -> f32 {
    max * (1.0 - (-travel.abs() / (max * 3.0)).exp())
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{
        Context, InteractiveElement, Modifiers, ParentElement, Render, StatefulInteractiveElement,
        TestAppContext, div,
    };
    use std::cell::RefCell;
    use std::rc::Rc;

    fn dx(x: f32) -> Point<f32> {
        point(x, 0.0)
    }

    fn drag(tracker: &mut Tracker, steps: &[f32], every: Duration, start: Instant) -> Vec<Step> {
        let mut out = vec![tracker.feed(TouchPhase::Started, dx(0.0), false, start)];
        let mut at = start;
        for step in steps {
            at += every;
            out.push(tracker.feed(TouchPhase::Moved, dx(*step), false, at));
        }
        out
    }

    #[test]
    fn a_slow_drag_past_the_threshold_commits_and_a_short_one_springs_back() {
        let now = Instant::now();
        let mut tracker = Tracker::default();
        drag(&mut tracker, &[20.0; 6], Duration::from_millis(60), now);
        let end = now + Duration::from_millis(600);
        assert_eq!(
            tracker.feed(TouchPhase::Ended, dx(0.0), false, end),
            Step::Commit(SwipeDir::Back)
        );

        let mut tracker = Tracker::default();
        drag(&mut tracker, &[-20.0; 3], Duration::from_millis(80), now);
        let end = now + Duration::from_millis(500);
        assert_eq!(
            tracker.feed(TouchPhase::Ended, dx(0.0), false, end),
            Step::Cancel
        );
    }

    #[test]
    fn a_short_flick_commits_unless_it_is_too_short() {
        let now = Instant::now();
        let mut tracker = Tracker::default();
        let steps = drag(&mut tracker, &[-20.0; 3], Duration::from_millis(16), now);
        assert_eq!(steps.last(), Some(&Step::Track(-60.0)));
        let end = now + Duration::from_millis(56);
        assert_eq!(
            tracker.feed(TouchPhase::Ended, dx(0.0), false, end),
            Step::Commit(SwipeDir::Forward)
        );

        let mut tracker = Tracker::default();
        drag(&mut tracker, &[9.0, 9.0], Duration::from_millis(8), now);
        let end = now + Duration::from_millis(20);
        assert_eq!(
            tracker.feed(TouchPhase::Ended, dx(0.0), false, end),
            Step::Cancel
        );
    }

    #[test]
    fn a_vertical_or_claimed_touch_never_becomes_a_swipe() {
        let now = Instant::now();
        let mut tracker = Tracker::default();
        tracker.feed(TouchPhase::Started, dx(0.0), false, now);
        assert_eq!(
            tracker.feed(TouchPhase::Moved, point(6.0, 10.0), false, now),
            Step::Pass
        );
        assert_eq!(
            tracker.feed(TouchPhase::Moved, dx(200.0), false, now),
            Step::Pass
        );
        assert_eq!(
            tracker.feed(TouchPhase::Ended, dx(0.0), false, now),
            Step::Pass
        );

        let mut tracker = Tracker::default();
        tracker.feed(TouchPhase::Started, dx(0.0), false, now);
        tracker.feed(TouchPhase::Moved, dx(4.0), true, now);
        assert_eq!(
            tracker.feed(TouchPhase::Moved, dx(200.0), false, now),
            Step::Pass
        );
    }

    #[test]
    fn momentum_after_a_swipe_stays_owned_until_the_fling_pauses() {
        let now = Instant::now();
        let mut tracker = Tracker::default();
        drag(&mut tracker, &[40.0; 4], Duration::from_millis(16), now);
        let end = now + Duration::from_millis(70);
        tracker.feed(TouchPhase::Ended, dx(0.0), false, end);
        let tail = end + Duration::from_millis(600);
        assert!(
            tracker.owns(TouchPhase::Moved, tail),
            "momentum queued behind a slow render"
        );
        assert_eq!(
            tracker.feed(TouchPhase::Moved, dx(30.0), false, tail),
            Step::Pass
        );
        assert!(tracker.owns(TouchPhase::Moved, tail + FLING_GAP));
        assert!(!tracker.owns(TouchPhase::Moved, tail + FLING_GAP * 2));
    }

    #[test]
    fn a_slow_drag_handled_in_one_batch_is_not_a_flick() {
        let now = Instant::now();
        let mut tracker = Tracker::default();
        tracker.feed(TouchPhase::Started, dx(0.0), false, now);
        tracker.feed(TouchPhase::Moved, dx(9.0), false, now);
        tracker.feed(TouchPhase::Moved, dx(15.0), false, now);
        assert_eq!(
            tracker.feed(TouchPhase::Ended, dx(0.0), false, now),
            Step::Cancel
        );
    }

    #[test]
    fn resting_fingers_that_lift_and_a_repeated_start_do_not_swipe() {
        let now = Instant::now();
        let mut tracker = Tracker::default();
        tracker.feed(TouchPhase::Started, dx(0.0), false, now);
        assert_eq!(
            tracker.feed(TouchPhase::Moved, dx(0.0), false, now),
            Step::Pass
        );
        assert_eq!(
            tracker.feed(TouchPhase::Moved, dx(200.0), false, now),
            Step::Pass,
            "a cancelled touch is over"
        );

        let mut tracker = Tracker::default();
        tracker.feed(TouchPhase::Started, dx(0.0), false, now);
        tracker.feed(TouchPhase::Started, dx(3.0), false, now);
        assert_eq!(
            tracker.feed(TouchPhase::Moved, dx(9.0), false, now),
            Step::Track(12.0)
        );
        assert_eq!(
            tracker.feed(TouchPhase::Moved, dx(0.0), false, now),
            Step::Track(12.0)
        );
        assert_eq!(
            tracker.feed(TouchPhase::Moved, dx(5.0), false, now),
            Step::Track(17.0),
            "a still event mid-swipe is not a cancel by itself"
        );
        assert!(!tracker.expire(now + STALL_CANCEL));
        tracker.feed(TouchPhase::Moved, dx(0.0), false, now);
        assert!(tracker.expire(now + STALL_CANCEL), "silence after it is");
    }

    #[gpui::test]
    fn a_scroller_with_room_keeps_the_gesture_and_a_swipe_stops_it_scrolling(
        cx: &mut TestAppContext,
    ) {
        struct Fixture {
            scroll: ScrollHandle,
            swipe: Rc<RefCell<Swipe>>,
            steps: Rc<RefCell<Vec<Step>>>,
            pan: Rc<Cell<u32>>,
        }
        impl Render for Fixture {
            fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
                let (swipe, steps, pan) =
                    (self.swipe.clone(), self.steps.clone(), self.pan.clone());
                div()
                    .size(px(400.0))
                    .relative()
                    .child(wheel_layer(move |event, phase, blocked, _, cx| {
                        let step =
                            swipe
                                .borrow_mut()
                                .wheel(event, phase, blocked, Instant::now(), cx);
                        if step != Step::Pass {
                            steps.borrow_mut().push(step);
                        }
                    }))
                    .child(
                        div()
                            .id("code")
                            .w(px(100.0))
                            .h(px(40.0))
                            .overflow_x_scroll()
                            .track_scroll(&self.scroll)
                            .on_scroll_wheel(claim_scrollable(&self.scroll))
                            .child(div().w(px(400.0)).h(px(40.0))),
                    )
                    .child(
                        div()
                            .id("dialog")
                            .absolute()
                            .left(px(200.0))
                            .top_0()
                            .size(px(100.0))
                            .occlude(),
                    )
                    .child(
                        div()
                            .id("image")
                            .absolute()
                            .left(px(300.0))
                            .top(px(200.0))
                            .size(px(100.0))
                            .on_scroll_wheel(move |event, _, cx| {
                                if event.touch_phase == TouchPhase::Moved && pan.get() > 0 {
                                    pan.set(pan.get() - 1);
                                    cx.stop_propagation();
                                }
                            }),
                    )
            }
        }
        let scroll = ScrollHandle::new();
        let swipe = Rc::new(RefCell::new(Swipe::default()));
        let steps = Rc::new(RefCell::new(Vec::new()));
        let pan = Rc::new(Cell::new(2));
        let (_, cx) = cx.add_window_view(|_, _| Fixture {
            scroll: scroll.clone(),
            swipe: swipe.clone(),
            steps: steps.clone(),
            pan,
        });
        cx.update(|window, cx| window.draw(cx).clear());
        let mut wheel = |x: f32, y: f32, delta: f32, phase| {
            cx.simulate_event(ScrollWheelEvent {
                position: point(px(x), px(y)),
                delta: ScrollDelta::Pixels(point(px(delta), px(0.0))),
                modifiers: Modifiers::default(),
                touch_phase: phase,
            });
            cx.update(|window, cx| window.draw(cx).clear());
        };

        wheel(50.0, 20.0, 0.0, TouchPhase::Started);
        wheel(50.0, 20.0, -30.0, TouchPhase::Moved);
        wheel(50.0, 20.0, -30.0, TouchPhase::Moved);
        wheel(50.0, 20.0, 0.0, TouchPhase::Ended);
        assert_eq!(scroll.offset().x, px(-60.0));
        assert!(steps.borrow().is_empty(), "the code block kept the gesture");

        wheel(250.0, 50.0, 0.0, TouchPhase::Started);
        wheel(250.0, 50.0, 60.0, TouchPhase::Moved);
        wheel(250.0, 50.0, 0.0, TouchPhase::Ended);
        assert!(steps.borrow().is_empty(), "the dialog blocks the swipe");

        wheel(350.0, 250.0, 0.0, TouchPhase::Started);
        wheel(350.0, 250.0, 30.0, TouchPhase::Moved);
        wheel(350.0, 250.0, 30.0, TouchPhase::Moved);
        wheel(350.0, 250.0, 60.0, TouchPhase::Moved);
        wheel(350.0, 250.0, 60.0, TouchPhase::Moved);
        wheel(350.0, 250.0, 0.0, TouchPhase::Ended);
        assert!(
            steps.borrow().is_empty(),
            "a pan that consumed the start keeps the gesture past its edge"
        );

        wheel(150.0, 300.0, 0.0, TouchPhase::Started);
        wheel(150.0, 300.0, 60.0, TouchPhase::Moved);
        wheel(50.0, 20.0, 60.0, TouchPhase::Moved);
        assert_eq!(scroll.offset().x, px(-60.0), "a live swipe scrolls nothing");
        wheel(50.0, 20.0, 0.0, TouchPhase::Ended);
        assert_eq!(
            *steps.borrow(),
            vec![
                Step::Track(60.0),
                Step::Track(120.0),
                Step::Commit(SwipeDir::Back)
            ]
        );
    }
}
