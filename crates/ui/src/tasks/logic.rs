use chrono::{Datelike, NaiveDate};
use zeron_proto::{Task, pin_order_key_between};

pub(crate) use zeron_proto::mission::{COLUMNS, drop_completes};

/// New order keys that put `moving` (in that order) right before `before`, or
/// at the end; empty when they already sit there.
pub(crate) fn reorder_keys(
    open: &[&Task],
    moving: &[String],
    before: Option<&str>,
    nonce: &str,
) -> Vec<(String, String)> {
    if before.is_some_and(|id| moving.iter().any(|m| m == id)) {
        return Vec::new();
    }
    let rest: Vec<&Task> = open
        .iter()
        .copied()
        .filter(|t| !moving.contains(&t.id))
        .collect();
    let Some(at) = (match before {
        Some(id) => rest.iter().position(|t| t.id == id),
        None => Some(rest.len()),
    }) else {
        return Vec::new();
    };
    let mut current: Vec<&str> = rest.iter().map(|t| t.id.as_str()).collect();
    current.splice(at..at, moving.iter().map(String::as_str));
    let now: Vec<&str> = open.iter().map(|t| t.id.as_str()).collect();
    if current == now {
        return Vec::new();
    }
    let upper = rest.get(at).map(|t| t.order_key.as_str());
    let mut lower = at.checked_sub(1).map(|i| rest[i].order_key.clone());
    let mut keys = Vec::new();
    for (ix, id) in moving.iter().enumerate() {
        let Ok(key) = pin_order_key_between(lower.as_deref(), upper, &format!("{nonce}{ix}"))
        else {
            return Vec::new();
        };
        lower = Some(key.clone());
        keys.push((id.clone(), key));
    }
    keys
}

pub(crate) fn quick_add_title(text: &str) -> Option<String> {
    let title = text.trim();
    (!title.is_empty()).then(|| title.to_string())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DuePick {
    None,
    Today,
    Tomorrow,
    Other,
}

pub(crate) fn due_pick(due_on: Option<&str>, today: NaiveDate) -> DuePick {
    match due_on.and_then(parse_day) {
        None => DuePick::None,
        Some(day) if day == today => DuePick::Today,
        Some(day) if Some(day) == today.succ_opt() => DuePick::Tomorrow,
        Some(_) => DuePick::Other,
    }
}

pub(crate) fn due_value(pick: DuePick, today: NaiveDate) -> Option<String> {
    let day = match pick {
        DuePick::Today => today,
        DuePick::Tomorrow => today.succ_opt()?,
        DuePick::None | DuePick::Other => return None,
    };
    Some(day.format("%Y-%m-%d").to_string())
}

pub(crate) fn due_label(due_on: &str, today: NaiveDate) -> Option<String> {
    let day = parse_day(due_on)?;
    let label = match (day - today).num_days() {
        0 => "due today".to_string(),
        1 => "due tomorrow".to_string(),
        -1 => "due yesterday".to_string(),
        _ if day.year() == today.year() => format!("due {}", day.format("%b %-d")),
        _ => format!("due {}", day.format("%b %-d, %Y")),
    };
    Some(label)
}

pub(crate) fn overdue(due_on: Option<&str>, today: NaiveDate) -> bool {
    due_on.and_then(parse_day).is_some_and(|day| day < today)
}

fn parse_day(value: &str) -> Option<NaiveDate> {
    NaiveDate::parse_from_str(value, "%Y-%m-%d").ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{TimeZone, Utc};
    use zeron_proto::mission::split_tasks;
    use zeron_proto::valid_pin_order_key;

    fn task(id: &str, key: &str, completed: Option<i64>) -> Task {
        Task {
            id: id.into(),
            title: id.into(),
            order_key: key.into(),
            created_at: Utc.timestamp_millis_opt(1_000).unwrap(),
            completed_at: completed.map(|ms| Utc.timestamp_millis_opt(ms).unwrap()),
            space_id: None,
            notes: String::new(),
            due_on: None,
            chat_id: None,
            updated_at: Utc.timestamp_millis_opt(1_000).unwrap(),
            deleted: false,
            mode: Default::default(),
            plan_pending: false,
            send_error: None,
            plan: String::new(),
            plan_doc: false,
            same_agents: false,
            resources: Vec::new(),
            images: Vec::new(),
            context: Vec::new(),
        }
    }

    fn ids(tasks: &[&Task]) -> Vec<String> {
        tasks.iter().map(|t| t.id.clone()).collect()
    }

    #[test]
    fn open_tasks_follow_order_key_and_done_ones_newest_first() {
        let mut deleted = task("gone", "1", None);
        deleted.deleted = true;
        let tasks = vec![
            task("c", "9", None),
            task("old-done", "2", Some(10)),
            task("a", "3", None),
            deleted,
            task("new-done", "8", Some(20)),
            task("b", "5", None),
        ];
        let (open, done) = split_tasks(&tasks);
        assert_eq!(ids(&open), ["a", "b", "c"]);
        assert_eq!(ids(&done), ["new-done", "old-done"]);
    }

    #[test]
    fn reorder_keys_land_strictly_between_the_new_neighbours() {
        let keys: Vec<String> = ["a", "b", "c"]
            .iter()
            .enumerate()
            .map(|(i, _)| pin_order_key_between(None, None, &format!("n{i}")).unwrap())
            .collect();
        let mut sorted = keys.clone();
        sorted.sort();
        let tasks: Vec<Task> = ["a", "b", "c"]
            .iter()
            .zip(&sorted)
            .map(|(id, key)| task(id, key, None))
            .collect();
        let open: Vec<&Task> = tasks.iter().collect();
        let nonce = "6f0c1e2a-0000-4000-8000-000000000000";

        let key = |moving: &[&str], before: Option<&str>| {
            let moving: Vec<String> = moving.iter().map(|id| id.to_string()).collect();
            reorder_keys(&open, &moving, before, nonce)
        };

        let to_top = key(&["c"], Some("a"));
        assert!(valid_pin_order_key(&to_top[0].1) && to_top[0].1 < sorted[0]);

        let middle = key(&["a"], Some("c"));
        assert!(sorted[1] < middle[0].1 && middle[0].1 < sorted[2]);

        let to_end = key(&["a"], None);
        assert!(to_end[0].1 > sorted[2]);

        assert!(key(&["b"], Some("b")).is_empty());
        assert!(key(&["a"], Some("b")).is_empty());
        assert!(key(&["c"], None).is_empty());

        let pair = key(&["a", "c"], None);
        assert_eq!(pair.len(), 2);
        assert!(
            sorted[1] < pair[0].1 && pair[0].1 < pair[1].1,
            "a then c after b"
        );
        assert!(key(&["b", "c"], None).is_empty());
        assert_eq!(key(&["c", "a"], Some("b")).len(), 2);
    }

    #[test]
    fn quick_add_needs_a_title() {
        assert_eq!(quick_add_title("  Ship it \n"), Some("Ship it".into()));
        assert_eq!(quick_add_title("   "), None);
    }

    #[test]
    fn due_days_read_relative_to_today() {
        let today = NaiveDate::from_ymd_opt(2026, 10, 17).unwrap();
        assert_eq!(due_label("2026-10-17", today).unwrap(), "due today");
        assert_eq!(due_label("2026-10-18", today).unwrap(), "due tomorrow");
        assert_eq!(due_label("2026-10-16", today).unwrap(), "due yesterday");
        assert_eq!(due_label("2026-11-03", today).unwrap(), "due Nov 3");
        assert_eq!(due_label("2027-01-02", today).unwrap(), "due Jan 2, 2027");
        assert_eq!(due_label("soon", today), None);
        assert_eq!(due_pick(Some("2026-10-18"), today), DuePick::Tomorrow);
        assert_eq!(due_pick(Some("2026-12-01"), today), DuePick::Other);
        assert_eq!(due_pick(None, today), DuePick::None);
        assert_eq!(
            due_value(DuePick::Tomorrow, today).as_deref(),
            Some("2026-10-18")
        );
        assert_eq!(due_value(DuePick::None, today), None);
        assert!(overdue(Some("2026-10-16"), today));
        assert!(!overdue(Some("2026-10-17"), today));
    }
}
