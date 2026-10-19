use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::time::Instant;

use gpui::{Bounds, Edges, Hsla, IntoElement, Pixels, Styled, px};

use crate::motion::{self, CubicBezier};

const ZOOM_IN: f32 = 440.0;
const ZOOM_OUT: f32 = 360.0;
const ZOOM_HEAD_DELAY: f32 = 120.0;
const ZOOM_HEAD: f32 = 300.0;
const ZOOM_HEAD_OUT: f32 = 160.0;
const ZOOM_ROW_DELAY: f32 = 160.0;
const ZOOM_ROW_STAGGER: f32 = 35.0;
const ZOOM_ROW_CAP: usize = 6;
const ZOOM_ROW: f32 = 280.0;
const ZOOM_ROW_OUT: f32 = 180.0;
const ZOOM_BAR_DELAY: f32 = 160.0;
const ZOOM_LIVE_HEAD_DELAY: f32 = 220.0;
const ZOOM_FADE: f32 = 200.0;
const ZOOM_FADE_OUT: f32 = 120.0;
pub(super) const ZOOM_DROP: f32 = 10.0;
pub(super) const ZOOM_SLIDE: f32 = 14.0;
const DURATION: f32 = 320.0;
const STAGGER: f32 = 22.0;
const FADE: f32 = DURATION * 0.55;
const INCOMING_AT: f32 = DURATION * 0.3;
const COLUMN_DELAY: f32 = 40.0;
const COLUMN_STAGGER: f32 = 30.0;
const HEAD_DELAY: f32 = 120.0;
const HEAD_DURATION: f32 = DURATION * 0.8;
const EXTRA_DELAY: f32 = DURATION * 0.4;
const EXTRA_DURATION: f32 = DURATION * 0.6;
pub(super) const COLUMN_RISE: f32 = 8.0;
pub(super) const HEAD_RISE: f32 = 6.0;

#[derive(Clone, PartialEq, Eq, Hash)]
pub(super) enum Key {
    Task(String),
    Header,
    Add,
    Done,
    Column(usize),
    ColumnHead(usize),
    Center,
    Live,
}

pub(super) type Rects = HashMap<Key, Bounds<Pixels>>;
pub(super) type Painted = Rc<RefCell<Rects>>;

/// Taffy lays an absolute child out inside the parent's border, so the insets pin it to the border box.
pub(super) fn measure(painted: &Painted, key: Key, border: Edges<f32>) -> impl IntoElement {
    let painted = painted.clone();
    gpui::canvas(
        move |bounds, _, _| {
            painted.borrow_mut().insert(key, bounds);
        },
        |_, _, _, _| {},
    )
    .absolute()
    .top(px(-border.top))
    .right(px(-border.right))
    .bottom(px(-border.bottom))
    .left(px(-border.left))
}

pub(super) fn visible(rects: &Rects, key: &Key, body: Bounds<Pixels>) -> Option<Bounds<Pixels>> {
    rects
        .get(key)
        .copied()
        .filter(|rect| rect.intersects(&body))
}

pub(super) struct Morph {
    pub from: Rects,
    pub started: Option<Instant>,
}

impl Morph {
    pub fn elapsed_ms(&self, now: Instant) -> f32 {
        self.started.map_or(0.0, |started| {
            now.saturating_duration_since(started).as_secs_f32() * 1000.0 / motion::speed_scale()
        })
    }
}

fn eased(elapsed: f32, delay: f32, duration: f32, curve: CubicBezier) -> f32 {
    curve.eval(((elapsed - delay) / duration).clamp(0.0, 1.0))
}

pub(super) struct TaskFrame {
    pub rect: f32,
    pub outgoing: f32,
    pub incoming: f32,
}

pub(super) fn task_frame(elapsed: f32, index: usize) -> TaskFrame {
    let delay = index as f32 * STAGGER;
    TaskFrame {
        rect: eased(elapsed, delay, DURATION, motion::EASE_DRAWER),
        outgoing: 1.0 - eased(elapsed, delay, FADE, motion::EASE_OUT),
        incoming: eased(elapsed, delay + INCOMING_AT, FADE, motion::EASE_OUT),
    }
}

pub(super) fn header_t(elapsed: f32) -> f32 {
    eased(elapsed, 0.0, DURATION, motion::EASE_DRAWER)
}

pub(super) fn column_t(elapsed: f32, column: usize, arriving: bool) -> f32 {
    let stagger = column as f32 * COLUMN_STAGGER;
    if arriving {
        eased(
            elapsed,
            COLUMN_DELAY + stagger,
            DURATION,
            motion::EASE_DRAWER,
        )
    } else {
        1.0 - eased(elapsed, stagger, DURATION, motion::EASE_DRAWER)
    }
}

pub(super) fn column_head_t(elapsed: f32, column: usize, arriving: bool) -> f32 {
    let stagger = column as f32 * COLUMN_STAGGER;
    if arriving {
        eased(
            elapsed,
            HEAD_DELAY + stagger,
            HEAD_DURATION,
            motion::EASE_DRAWER,
        )
    } else {
        1.0 - eased(elapsed, stagger, HEAD_DURATION, motion::EASE_DRAWER)
    }
}

pub(super) fn extra_t(elapsed: f32, arriving: bool) -> f32 {
    if arriving {
        eased(elapsed, EXTRA_DELAY, EXTRA_DURATION, motion::LINEAR)
    } else {
        1.0 - eased(elapsed, 0.0, EXTRA_DURATION, motion::LINEAR)
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct ZoomFrame {
    pub entering: bool,
    pub elapsed: f32,
    pub backdrop: Option<Hsla>,
}

impl ZoomFrame {
    fn ramp(&self, delay: f32, duration: f32, curve: CubicBezier, out: f32) -> f32 {
        if self.entering {
            eased(self.elapsed, delay, duration, curve)
        } else {
            1.0 - eased(self.elapsed, 0.0, out, motion::LINEAR)
        }
    }

    pub fn rect(&self) -> f32 {
        if self.entering {
            eased(self.elapsed, 0.0, ZOOM_IN, motion::EASE_DRAWER)
        } else {
            1.0 - eased(self.elapsed, 0.0, ZOOM_OUT, motion::EASE_DRAWER)
        }
    }

    pub fn header(&self) -> f32 {
        self.ramp(
            ZOOM_HEAD_DELAY,
            ZOOM_HEAD,
            motion::EASE_DRAWER,
            ZOOM_HEAD_OUT,
        )
    }

    pub fn row(&self, index: usize) -> f32 {
        let delay = ZOOM_ROW_DELAY + ZOOM_ROW_STAGGER * index.min(ZOOM_ROW_CAP) as f32;
        self.ramp(delay, ZOOM_ROW, motion::EASE_DRAWER, ZOOM_ROW_OUT)
    }

    pub fn live_header(&self) -> f32 {
        self.ramp(
            ZOOM_LIVE_HEAD_DELAY,
            ZOOM_FADE,
            motion::LINEAR,
            ZOOM_FADE_OUT,
        )
    }

    pub fn titlebar(&self) -> f32 {
        self.ramp(ZOOM_BAR_DELAY, ZOOM_FADE, motion::LINEAR, ZOOM_FADE_OUT)
    }

    pub fn settled(&self) -> bool {
        self.elapsed
            >= if self.entering {
                ZOOM_ROW_DELAY + ZOOM_ROW_STAGGER * ZOOM_ROW_CAP as f32 + ZOOM_ROW
            } else {
                ZOOM_OUT
            }
    }
}

pub(super) fn end_ms(tasks: usize, columns: usize) -> f32 {
    let last_task = tasks.saturating_sub(1) as f32 * STAGGER;
    let last_column = columns.saturating_sub(1) as f32 * COLUMN_STAGGER;
    (last_task + INCOMING_AT + FADE)
        .max(last_task + DURATION)
        .max(COLUMN_DELAY + last_column + DURATION)
        .max(HEAD_DELAY + last_column + HEAD_DURATION)
        .max(EXTRA_DELAY + EXTRA_DURATION)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_part_settles_by_the_end_and_not_before() {
        let (tasks, columns) = (9, 4);
        let end = end_ms(tasks, columns);
        let settled = |at: f32| {
            (0..tasks).all(|ix| {
                let frame = task_frame(at, ix);
                frame.rect == 1.0 && frame.outgoing == 0.0 && frame.incoming == 1.0
            }) && (0..columns).all(|column| {
                [true, false].iter().all(|&arriving| {
                    column_t(at, column, arriving) == f32::from(arriving)
                        && column_head_t(at, column, arriving) == f32::from(arriving)
                })
            }) && extra_t(at, true) == 1.0
                && extra_t(at, false) == 0.0
        };
        assert!(settled(end));
        assert!(!settled(end - 20.0));
        let first = task_frame(DURATION * 0.5, 0);
        let last = task_frame(DURATION * 0.5, tasks - 1);
        assert!(first.rect > last.rect);
    }
}
