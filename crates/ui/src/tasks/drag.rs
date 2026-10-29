use std::collections::HashMap;
use std::time::Instant;

use crate::motion::{self, MotionSpec, Tween};

pub(super) const EDGE: f32 = 56.0;
const EDGE_SPEED: f32 = 14.0;
const MAX_SPEED: f32 = 18.0;
const OVERSHOOT: f32 = 6.0;
pub(super) const SHIFT: MotionSpec = MotionSpec::new(200, motion::EASE_OUT_QUINT);
pub(super) const SETTLE: MotionSpec = MotionSpec::new(220, motion::EASE_OUT_QUINT);
pub(super) const REDUCED_FADE: MotionSpec = MotionSpec::new(120, motion::EASE);

/// A list drag in content space (window y minus the scroll offset), so autoscroll
/// moves the pointer through the rows without re-measuring them.
pub(super) struct ListDrag {
    pub ids: Vec<String>,
    pub rows: Vec<(f32, f32)>,
    pub moving: Vec<usize>,
    pub primary: usize,
    pub start: f32,
    pub dy: f32,
    pub slot: usize,
    pub pointer: f32,
}

impl ListDrag {
    pub fn new(
        ids: Vec<String>,
        rows: Vec<(f32, f32)>,
        primary: usize,
        picked: &[String],
        start: f32,
    ) -> Option<Self> {
        if primary >= ids.len() || rows.len() != ids.len() {
            return None;
        }
        let mut moving: Vec<usize> = (0..ids.len())
            .filter(|ix| {
                *ix == primary || (picked.contains(&ids[primary]) && picked.contains(&ids[*ix]))
            })
            .collect();
        moving.dedup();
        let slot = (0..primary).filter(|ix| !moving.contains(ix)).count();
        Some(Self {
            ids,
            rows,
            moving,
            primary,
            start,
            dy: 0.0,
            slot,
            pointer: start,
        })
    }

    fn rest(&self) -> impl Iterator<Item = usize> + '_ {
        (0..self.ids.len()).filter(|ix| !self.moving.contains(ix))
    }

    fn base(&self) -> f32 {
        self.rows.first().map_or(0.0, |row| row.0)
    }

    fn gap(&self) -> f32 {
        self.rows[self.primary].1
    }

    pub fn count(&self) -> usize {
        self.moving.len()
    }

    pub fn is_moving(&self, id: &str) -> bool {
        self.moving.iter().any(|ix| self.ids[*ix] == id)
    }

    pub fn primary_id(&self) -> &str {
        &self.ids[self.primary]
    }

    /// Crossing the next row's midpoint moves the gap one slot.
    pub fn drag_to(&mut self, content_y: f32) {
        let (top, height) = self.rows[self.primary];
        let gap = self.gap();
        let base = self.base();
        let rest: f32 = self.rest().map(|ix| self.rows[ix].1).sum();
        let min = base - OVERSHOOT - top;
        let max = base + rest + gap - height + OVERSHOOT - top;
        self.dy = (content_y - self.start).clamp(min.min(max), max.max(min));
        let center = top + self.dy + height / 2.0;
        let mut at = base;
        let mut slot = 0;
        for ix in self.rest().collect::<Vec<_>>() {
            let row = self.rows[ix].1;
            if at + row / 2.0 + gap / 2.0 < center {
                slot += 1;
            }
            at += row;
        }
        self.slot = slot;
    }

    /// Where each row sits relative to its natural top while dragging.
    pub fn shifts(&self) -> HashMap<String, f32> {
        let gap = self.gap();
        let mut at = self.base();
        let mut shifts = HashMap::new();
        for (k, ix) in self.rest().enumerate() {
            if k == self.slot {
                at += gap;
            }
            shifts.insert(self.ids[ix].clone(), at - self.rows[ix].0);
            at += self.rows[ix].1;
        }
        shifts.insert(self.primary_id().to_string(), self.dy);
        shifts
    }

    pub fn noop(&self) -> bool {
        let rest: Vec<usize> = self.rest().collect();
        let before = rest.get(self.slot).copied().unwrap_or(self.ids.len());
        let after = self.slot.checked_sub(1).map(|k| rest[k]);
        self.moving.windows(2).all(|pair| pair[1] == pair[0] + 1)
            && self.moving.last().is_some_and(|last| {
                last + 1 == before || *last + 1 == self.ids.len() && before == self.ids.len()
            })
            && after.is_none_or(|after| after + 1 == self.moving[0])
    }

    /// The visible ids in their order after the drop.
    pub fn order(&self) -> Vec<String> {
        let mut order: Vec<String> = self.rest().map(|ix| self.ids[ix].clone()).collect();
        let moving: Vec<String> = self.moving.iter().map(|ix| self.ids[*ix].clone()).collect();
        let at = self.slot.min(order.len());
        order.splice(at..at, moving);
        order
    }

    /// The rest row the moved block lands before, or `None` for after the last one.
    pub fn before(&self) -> Option<String> {
        self.rest().nth(self.slot).map(|ix| self.ids[ix].clone())
    }

    pub fn last_rest(&self) -> Option<String> {
        self.rest().last().map(|ix| self.ids[ix].clone())
    }

    /// Each row's offset from its top in `order`, so the drop and the spring back start where the eye is.
    pub fn flip(
        &self,
        order: &[String],
        now_shifts: &HashMap<String, f32>,
    ) -> HashMap<String, f32> {
        let mut at = self.base();
        let mut offsets = HashMap::new();
        for id in order {
            let Some(ix) = self.ids.iter().position(|known| known == id) else {
                continue;
            };
            let (natural, height) = self.rows[ix];
            let shown = natural + now_shifts.get(id).copied().unwrap_or(0.0);
            offsets.insert(id.clone(), shown - at);
            at += height;
        }
        offsets
    }
}

/// Scroll speed per 16 ms tick for a pointer near the list's top or bottom edge.
pub(super) fn edge_speed(pointer: f32, top: f32, bottom: f32) -> f32 {
    let v = if pointer < top + EDGE {
        -((top + EDGE - pointer) / EDGE * EDGE_SPEED).ceil()
    } else if pointer > bottom - EDGE {
        ((pointer - (bottom - EDGE)) / EDGE * EDGE_SPEED).ceil()
    } else {
        0.0
    };
    v.clamp(-MAX_SPEED, MAX_SPEED)
}

/// Per-row vertical offsets, eased toward their targets.
#[derive(Default)]
pub(super) struct Shifts {
    tweens: HashMap<String, Tween>,
    fades: HashMap<String, Tween>,
}

impl Shifts {
    pub fn aim(
        &mut self,
        targets: &HashMap<String, f32>,
        spec: MotionSpec,
        reduced: bool,
        now: Instant,
    ) {
        for (id, target) in targets {
            let tween = self
                .tweens
                .entry(id.clone())
                .or_insert_with(|| Tween::at(0.0, now));
            if tween.target() != *target {
                tween.retarget(*target, spec, reduced, now);
            }
        }
    }

    pub fn snap(&mut self, id: &str, value: f32, now: Instant) {
        self.tweens.insert(id.to_string(), Tween::at(value, now));
    }

    /// Start every row at `from` and ease it home.
    pub fn settle(&mut self, from: &HashMap<String, f32>, reduced: bool, now: Instant) {
        self.tweens.clear();
        if reduced {
            return;
        }
        for (id, offset) in from {
            if offset.abs() > 0.5 {
                let mut tween = Tween::at(*offset, now);
                tween.retarget(0.0, SETTLE, false, now);
                self.tweens.insert(id.clone(), tween);
            }
        }
    }

    pub fn fade_in(&mut self, ids: impl IntoIterator<Item = String>, reduced: bool, now: Instant) {
        let spec = if reduced { REDUCED_FADE } else { SETTLE };
        for id in ids {
            let mut tween = Tween::at(0.0, now);
            tween.retarget(1.0, spec, false, now);
            self.fades.insert(id, tween);
        }
    }

    pub fn offset(&self, id: &str, now: Instant) -> f32 {
        self.tweens.get(id).map_or(0.0, |tween| tween.value_at(now))
    }

    pub fn opacity(&self, id: &str, now: Instant) -> f32 {
        self.fades.get(id).map_or(1.0, |tween| tween.value_at(now))
    }

    pub fn current(&self, now: Instant) -> HashMap<String, f32> {
        self.tweens
            .iter()
            .map(|(id, tween)| (id.clone(), tween.value_at(now)))
            .collect()
    }

    /// Drops settled entries; true while something still moves.
    pub fn tick(&mut self, now: Instant) -> bool {
        self.tweens
            .retain(|_, tween| !(tween.settled_at(now) && tween.target() == 0.0));
        self.fades.retain(|_, tween| !tween.settled_at(now));
        !self.tweens.values().all(|tween| tween.settled_at(now)) || !self.fades.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn drag(primary: usize, picked: &[&str]) -> ListDrag {
        let ids: Vec<String> = ["a", "b", "c", "d"].map(String::from).to_vec();
        let rows = vec![(0.0, 44.0), (46.0, 44.0), (92.0, 60.0), (154.0, 44.0)];
        let picked: Vec<String> = picked.iter().map(|id| id.to_string()).collect();
        ListDrag::new(ids, rows, primary, &picked, 20.0).unwrap()
    }

    #[test]
    fn crossing_the_next_rows_midpoint_moves_one_slot() {
        let mut d = drag(1, &[]);
        assert!(d.noop());
        d.drag_to(20.0 + 20.0);
        assert_eq!(d.slot, 1, "short of c's midpoint stays put");
        d.drag_to(20.0 + 40.0);
        assert_eq!(d.slot, 2, "past c's midpoint opens the gap after c");
        assert_eq!(d.order(), ["a", "c", "b", "d"]);
        assert_eq!(d.before().as_deref(), Some("d"));
        assert!(!d.noop());
        d.drag_to(-500.0);
        assert_eq!((d.slot, d.dy), (0, -52.0), "clamped just past the top");
        assert_eq!(d.order(), ["b", "a", "c", "d"]);
        d.drag_to(5000.0);
        assert_eq!(d.slot, 3);
        assert_eq!(d.before(), None);
        assert_eq!(d.last_rest().as_deref(), Some("d"));
    }

    #[test]
    fn neighbours_shift_to_open_the_gap_where_the_row_lands() {
        let mut d = drag(0, &[]);
        d.drag_to(20.0 + 60.0);
        assert_eq!(d.slot, 1);
        let shifts = d.shifts();
        assert_eq!(shifts["b"], -46.0, "b slides up into a's place");
        assert_eq!(shifts["c"], 0.0 - 92.0 + 44.0 + 44.0);
        assert_eq!(shifts["a"], 60.0);
    }

    #[test]
    fn dragging_a_picked_row_carries_the_whole_selection() {
        let mut d = drag(0, &["a", "c"]);
        assert_eq!(d.count(), 2);
        assert!(d.is_moving("c") && !d.is_moving("b"));
        assert!(
            !d.noop(),
            "a and c are apart, so landing in place still gathers them"
        );
        d.drag_to(5000.0);
        assert_eq!(d.order(), ["b", "d", "a", "c"]);
        let unpicked = drag(1, &["a", "c"]);
        assert_eq!(unpicked.count(), 1, "an unpicked row drags alone");
    }

    #[test]
    fn the_drop_starts_each_row_where_it_was_shown() {
        let mut d = drag(1, &[]);
        d.drag_to(20.0 + 40.0);
        let order = d.order();
        let flip = d.flip(&order, &d.shifts());
        assert_eq!(flip["c"], 0.0, "c already sits in its new slot");
        assert_eq!(flip["b"], 46.0 + 40.0 - (44.0 + 60.0));
    }

    #[test]
    fn autoscroll_speeds_up_toward_the_edge() {
        assert_eq!(edge_speed(300.0, 0.0, 600.0), 0.0);
        assert!(edge_speed(10.0, 0.0, 600.0) < 0.0);
        assert_eq!(edge_speed(600.0, 0.0, 600.0), 14.0);
        assert_eq!(edge_speed(900.0, 0.0, 600.0), 18.0);
    }
}
