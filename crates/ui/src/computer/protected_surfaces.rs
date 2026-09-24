use std::collections::BTreeSet;
use std::sync::{Mutex, PoisonError};

use gpui::EntityId;

static SHOWING: Mutex<BTreeSet<u64>> = Mutex::new(BTreeSet::new());

pub fn publish(view: EntityId, showing: bool) {
    let mut views = SHOWING.lock().unwrap_or_else(PoisonError::into_inner);
    if showing {
        views.insert(view.as_u64());
    } else {
        views.remove(&view.as_u64());
    }
}

pub fn any_showing() -> bool {
    !SHOWING
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .is_empty()
}
