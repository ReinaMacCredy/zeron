use chrono::{Datelike, NaiveDate};
use zeron_proto::{Task, TaskStatus, pin_order_key_between};

pub(crate) const COLUMNS: [TaskStatus; 4] = [
    TaskStatus::ToDo,
    TaskStatus::Working,
    TaskStatus::NeedsYou,
    TaskStatus::Done,
];

pub(crate) fn column_title(status: TaskStatus) -> &'static str {
    match status {
        TaskStatus::ToDo => "To do",
        TaskStatus::Working => "Working",
        TaskStatus::NeedsYou => "Needs you",
        TaskStatus::Done => "Done",
    }
}

pub(crate) fn split_tasks(tasks: &[Task]) -> (Vec<&Task>, Vec<&Task>) {
    let (mut open, mut done): (Vec<&Task>, Vec<&Task>) = tasks
        .iter()
        .filter(|task| !task.deleted)
        .partition(|task| task.completed_at.is_none());
    open.sort_by(|a, b| {
        (&a.order_key, a.created_at, &a.id).cmp(&(&b.order_key, b.created_at, &b.id))
    });
    done.sort_by(|a, b| b.completed_at.cmp(&a.completed_at).then(a.id.cmp(&b.id)));
    (open, done)
}

/// Only Done is a drop target that writes; the other columns follow the linked chat.
pub(crate) fn drop_completes(from: TaskStatus, to: TaskStatus) -> Option<bool> {
    match (from, to) {
        (TaskStatus::Done, TaskStatus::ToDo) => Some(false),
        (from, TaskStatus::Done) if from != TaskStatus::Done => Some(true),
        _ => None,
    }
}

pub(crate) fn reorder_key(
    open: &[&Task],
    moving: &str,
    before: Option<&str>,
    nonce: &str,
) -> Option<String> {
    if before == Some(moving) {
        return None;
    }
    let rest: Vec<&Task> = open.iter().copied().filter(|t| t.id != moving).collect();
    let at = match before {
        Some(id) => rest.iter().position(|t| t.id == id)?,
        None => rest.len(),
    };
    let current = open.iter().position(|t| t.id == moving)?;
    if current == at {
        return None;
    }
    let lower = at.checked_sub(1).map(|i| rest[i].order_key.as_str());
    let upper = rest.get(at).map(|t| t.order_key.as_str());
    pin_order_key_between(lower, upper, nonce).ok()
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
    fn only_done_and_back_to_to_do_are_writing_drops() {
        use TaskStatus::*;
        for from in [ToDo, Working, NeedsYou] {
            assert_eq!(drop_completes(from, Done), Some(true), "{from:?}");
            assert_eq!(drop_completes(from, ToDo), None, "{from:?}");
        }
        assert_eq!(drop_completes(Done, ToDo), Some(false));
        assert_eq!(drop_completes(Done, Done), None);
        for to in [Working, NeedsYou] {
            for from in COLUMNS {
                assert_eq!(drop_completes(from, to), None, "{from:?} -> {to:?}");
            }
        }
    }

    #[test]
    fn reorder_key_lands_strictly_between_the_new_neighbours() {
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

        let to_top = reorder_key(&open, "c", Some("a"), nonce).unwrap();
        assert!(valid_pin_order_key(&to_top) && to_top < sorted[0]);

        let middle = reorder_key(&open, "a", Some("c"), nonce).unwrap();
        assert!(sorted[1] < middle && middle < sorted[2]);

        let to_end = reorder_key(&open, "a", None, nonce).unwrap();
        assert!(to_end > sorted[2]);

        assert_eq!(reorder_key(&open, "b", Some("b"), nonce), None);
        assert_eq!(reorder_key(&open, "a", Some("b"), nonce), None);
        assert_eq!(reorder_key(&open, "c", None, nonce), None);
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
