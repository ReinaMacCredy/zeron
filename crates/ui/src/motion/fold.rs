use std::cell::Cell;
use std::rc::Rc;
use std::time::Instant;

use gpui::{AnyElement, IntoElement, SharedString, canvas, div, prelude::*, px};

use super::{MotionSpec, Tween};

pub(crate) struct Fold {
    pub(crate) tween: Tween,
    pub(crate) height: Rc<Cell<f32>>,
}

impl Fold {
    pub(crate) fn at(open: bool, now: Instant) -> Self {
        Self {
            tween: Tween::at(if open { 1.0 } else { 0.0 }, now),
            height: Rc::new(Cell::new(0.0)),
        }
    }

    pub(crate) fn set(&mut self, open: bool, spec: MotionSpec, reduced: bool, now: Instant) {
        let to = if open { 1.0 } else { 0.0 };
        if self.tween.target() != to {
            self.tween.retarget(to, spec, reduced, now);
        }
    }

    pub(crate) fn value(&self, now: Instant) -> f32 {
        self.tween.value_at(now)
    }

    pub(crate) fn moving(&self, now: Instant) -> bool {
        !self.tween.settled_at(now)
    }

    pub(crate) fn gone(&self, now: Instant) -> bool {
        !self.moving(now) && self.tween.target() == 0.0
    }
}

pub(crate) fn folded(id: String, fold: &Fold, now: Instant, body: impl IntoElement) -> AnyElement {
    let probe = fold.height.clone();
    let inner = div().relative().flex_none().w_full().child(body).child(
        canvas(
            move |bounds, _, _| {
                let height = f32::from(bounds.size.height);
                if (probe.get() - height).abs() > 0.5 {
                    probe.set(height);
                }
            },
            |_, _, _, _| {},
        )
        .absolute()
        .inset_0(),
    );
    if !fold.moving(now) {
        return inner.into_any_element();
    }
    let value = fold.value(now);
    div()
        .id(SharedString::from(id))
        .relative()
        .flex_none()
        .w_full()
        .overflow_hidden()
        .h(px(fold.height.get() * value))
        .opacity(value)
        .child(inner)
        .into_any_element()
}
