use std::cmp::Ordering;

use serde::{Deserialize, Serialize};

use crate::{Chat, ChatIndicator};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum SidebarOrganization {
    ByProject,
    ByDevice,
    #[default]
    InOneList,
    Inbox,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum SidebarSort {
    #[default]
    LastUpdated,
    Created,
}

/// The Archived and Settled shelves show this many rows, then "Show more"
/// pages by [`SHELF_PAGE_ROWS`].
pub const SHELF_INITIAL_ROWS: usize = 10;
pub const SHELF_PAGE_ROWS: usize = 25;

pub fn shelf_visible(shown: usize, total: usize) -> usize {
    shown.max(SHELF_INITIAL_ROWS).min(total)
}

pub fn shelf_next(shown: usize) -> usize {
    shown.max(SHELF_INITIAL_ROWS) + SHELF_PAGE_ROWS
}

pub fn compare_sidebar_chats(sort: SidebarSort, left: &Chat, right: &Chat) -> Ordering {
    let primary = match sort {
        SidebarSort::Created => right.created_at.cmp(&left.created_at),
        SidebarSort::LastUpdated => right
            .last_message_at
            .unwrap_or(right.created_at)
            .cmp(&left.last_message_at.unwrap_or(left.created_at)),
    };
    primary.then_with(|| left.id.cmp(&right.id))
}

pub fn compare_sidebar_rows(
    sort: SidebarSort,
    active_first: bool,
    left: (ChatIndicator, &Chat),
    right: (ChatIndicator, &Chat),
) -> Ordering {
    let active = |status: ChatIndicator| active_first && status != ChatIndicator::Idle;
    active(right.0)
        .cmp(&active(left.0))
        .then_with(|| compare_sidebar_chats(sort, left.1, right.1))
}

/// Put one group first (this machine's device, or the projectless Chats)
/// without disturbing the recency-based order of the others. A targeted
/// promotion is more truthful than a full name sort: that context leads, then
/// the user's chosen chat sort wins.
pub fn promote_group<G>(
    groups: &mut Vec<G>,
    key: Option<&str>,
    key_of: impl Fn(&G) -> Option<&str>,
) {
    let Some(key) = key else {
        return;
    };
    let Some(index) = groups.iter().position(|group| key_of(group) == Some(key)) else {
        return;
    };
    if index > 0 {
        let first = groups.remove(index);
        groups.insert(0, first);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shelf_pages_from_ten_by_twenty_five() {
        assert_eq!(shelf_visible(0, 4), 4);
        assert_eq!(shelf_visible(0, 40), 10);
        assert_eq!(shelf_next(0), 35);
        assert_eq!(shelf_visible(shelf_next(0), 40), 35);
        assert_eq!(shelf_visible(shelf_next(shelf_next(0)), 40), 40);
    }
}
