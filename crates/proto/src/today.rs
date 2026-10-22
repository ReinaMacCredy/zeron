use std::collections::HashSet;

use chrono::{DateTime, Local, NaiveDate, TimeZone, Timelike, Utc};

use crate::change_request_assessment::{AttentionReason, Facts};
use crate::{
    AgentAccount, AgentUsageWindow, ChangeRequestListItem, Chat, HarnessId, Task, TodayRun,
    TodayRunStatus,
};

pub const DAY_STARTS_AT_HOUR: u32 = 4;
pub const SLOTS: [(&str, u32, u32); 3] = [
    ("Morning", DAY_STARTS_AT_HOUR, 12),
    ("Afternoon", 12, 18),
    ("Evening", 18, 24 + DAY_STARTS_AT_HOUR),
];

pub fn work_day(now: DateTime<Local>) -> NaiveDate {
    (now - chrono::Duration::hours(i64::from(DAY_STARTS_AT_HOUR))).date_naive()
}

fn local_at(day: NaiveDate, hour: u32) -> Option<DateTime<Local>> {
    let date = day.checked_add_days(chrono::Days::new(u64::from(hour / 24)))?;
    let at = date.and_hms_opt(hour % 24, 0, 0)?;
    Local.from_local_datetime(&at).earliest()
}

pub fn work_day_start(now: DateTime<Local>) -> DateTime<Local> {
    local_at(work_day(now), DAY_STARTS_AT_HOUR).unwrap_or(now)
}

pub fn greeting(now: DateTime<Local>) -> &'static str {
    match now.hour() {
        h if (DAY_STARTS_AT_HOUR..12).contains(&h) => "Good morning",
        12..=17 => "Good afternoon",
        _ => "Good evening",
    }
}

pub fn plural(count: usize, one: &str, many: &str) -> String {
    format!("{count} {}", if count == 1 { one } else { many })
}

pub fn header_line(now: DateTime<Local>, needs: usize, open_tasks: usize) -> String {
    let date = work_day(now).format("%A, %B %-d");
    let tasks = match open_tasks {
        0 => String::new(),
        n => format!(" {} to do.", plural(n, "task", "tasks")),
    };
    let rest = match (needs, open_tasks) {
        (0, 0) => " You’re all caught up.".to_owned(),
        (0, _) => tasks,
        (1, _) => format!(" 1 thing needs you.{tasks}"),
        (n, _) => format!(" {n} things need you.{tasks}"),
    };
    format!("{}. {date}.{rest}", greeting(now))
}

pub fn attention_prs<'a>(
    items: impl IntoIterator<Item = &'a ChangeRequestListItem>,
) -> Vec<(&'a ChangeRequestListItem, Vec<AttentionReason>)> {
    let mut seen = HashSet::new();
    items
        .into_iter()
        .filter(|item| seen.insert(item.url.as_str()))
        .filter_map(|item| {
            let attention = Facts::from_item(item).assess().attention;
            (!attention.is_empty()).then_some((item, attention))
        })
        .collect()
}

fn attention_breakdown(prs: &[(&ChangeRequestListItem, Vec<AttentionReason>)]) -> String {
    [
        (AttentionReason::ReviewRequested, "to review"),
        (AttentionReason::FeedbackOnAuthored, "need changes"),
        (AttentionReason::FailingCi, "failing CI"),
    ]
    .into_iter()
    .filter_map(|(reason, label)| {
        let count = prs.iter().filter(|(_, why)| why.contains(&reason)).count();
        (count > 0).then(|| format!("{count} {label}"))
    })
    .collect::<Vec<_>>()
    .join(" · ")
}

pub fn attention_row(prs: &[(&ChangeRequestListItem, Vec<AttentionReason>)]) -> (String, String) {
    let reviews = prs
        .iter()
        .all(|(_, why)| why.contains(&AttentionReason::ReviewRequested));
    let count = plural(prs.len(), "pull request", "pull requests");
    let title = if reviews {
        format!("{count} to review")
    } else if prs.len() == 1 {
        format!("{count} needs you")
    } else {
        format!("{count} need you")
    };
    let meta = if reviews {
        "GitHub".to_owned()
    } else {
        format!("GitHub, {}", attention_breakdown(prs))
    };
    (title, meta)
}

pub fn updated_label(updated: Option<DateTime<Local>>, failed: bool) -> Option<String> {
    match (updated, failed) {
        (Some(at), false) => Some(format!("Updated {}", at.format("%-I:%M %p"))),
        (Some(at), true) => Some(format!(
            "Updated {}, refresh failed",
            at.format("%-I:%M %p")
        )),
        (None, true) => Some("Runs did not load".to_owned()),
        (None, false) => None,
    }
}

pub fn is_overdue(task: &Task, chats: &[Chat], today: &str) -> bool {
    task.completed_at.is_none()
        && crate::mission::linked_chat(chats, task).is_none()
        && task.due_on.as_deref().is_some_and(|due| due < today)
}

pub fn resets_in(resets_at: DateTime<Utc>, now: DateTime<Utc>) -> Option<String> {
    let seconds = (resets_at - now).num_seconds();
    if seconds <= 0 {
        return None;
    }
    let minutes = (seconds + 59) / 60;
    let (hours, days) = (minutes / 60, minutes / (60 * 24));
    Some(match () {
        _ if minutes < 60 => format!("{minutes}m"),
        _ if hours < 24 => match minutes % 60 {
            0 => format!("{hours}h"),
            m => format!("{hours}h {m}m"),
        },
        _ => match hours % 24 {
            0 => format!("{days}d"),
            h => format!("{days}d {h}h"),
        },
    })
}

pub fn today_tasks<'a>(
    tasks: &'a [Task],
    chats: &[Chat],
    today: NaiveDate,
    day_start: DateTime<Utc>,
) -> Vec<&'a Task> {
    let today = today.format("%Y-%m-%d").to_string();
    let mut open: Vec<&Task> = tasks
        .iter()
        .filter(|task| !task.deleted && task.completed_at.is_none())
        .filter(|task| {
            crate::mission::linked_chat(chats, task).is_some()
                || task
                    .due_on
                    .as_deref()
                    .is_some_and(|due| due <= today.as_str())
        })
        .collect();
    open.sort_by(|a, b| a.order_key.cmp(&b.order_key));
    let mut done: Vec<&Task> = tasks
        .iter()
        .filter(|task| !task.deleted && task.completed_at.is_some_and(|at| at >= day_start))
        .collect();
    done.sort_by_key(|task| std::cmp::Reverse(task.completed_at));
    open.extend(done);
    open
}

#[derive(Debug, Clone, PartialEq)]
pub struct Slot {
    pub label: &'static str,
    pub now: bool,
    pub prompts: usize,
    pub work_ms: i64,
    pub hours: Vec<(usize, bool)>,
}

pub fn day_slots(runs: &[TodayRun], now: DateTime<Local>) -> Vec<Slot> {
    let day = work_day(now);
    let now_ms = now.timestamp_millis();
    SLOTS
        .iter()
        .filter_map(|&(label, from, to)| {
            let start = local_at(day, from)?.timestamp_millis();
            let end = local_at(day, to)?.timestamp_millis();
            if start > now_ms {
                return None;
            }
            let inside: Vec<&TodayRun> = runs
                .iter()
                .filter(|run| (start..end).contains(&run.started_at_ms))
                .collect();
            let hours = (from..to)
                .filter_map(|hour| {
                    let at = local_at(day, hour)?.timestamp_millis();
                    let count = inside
                        .iter()
                        .filter(|run| (at..at + 3_600_000).contains(&run.started_at_ms))
                        .count();
                    Some((count, at > now_ms))
                })
                .collect();
            let slot = Slot {
                label,
                now: now_ms < end,
                prompts: inside.len(),
                work_ms: inside.iter().map(|run| run_work_ms(run, now_ms)).sum(),
                hours,
            };
            (slot.now || slot.prompts > 0).then_some(slot)
        })
        .collect()
}

fn run_work_ms(run: &TodayRun, now_ms: i64) -> i64 {
    match (run.status, run.duration_ms) {
        (_, Some(ms)) => ms.max(0),
        (TodayRunStatus::Running, None) => (now_ms - run.started_at_ms).max(0),
        _ => 0,
    }
}

fn clock_duration(ms: i64) -> String {
    let seconds = (ms / 1000).max(1);
    match seconds {
        s if s < 60 => format!("{s}s"),
        s if s < 3600 => format!("{}m", s / 60),
        s => match (s % 3600) / 60 {
            0 => format!("{}h", s / 3600),
            m => format!("{}h {m}m", s / 3600),
        },
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Part {
    Text(String),
    Strong(String),
    Bad(String),
}

pub fn digest(
    runs: &[TodayRun],
    slots: &[Slot],
    open: usize,
    done: usize,
    overdue: usize,
) -> Vec<Vec<Part>> {
    let text = |s: &str| Part::Text(s.to_owned());
    let mut sentences = Vec::new();
    if !runs.is_empty() {
        let chats: HashSet<&str> = runs.iter().map(|run| run.chat_id.as_str()).collect();
        sentences.push(vec![
            text("You sent "),
            Part::Strong(plural(runs.len(), "prompt", "prompts")),
            text(" across "),
            Part::Strong(plural(chats.len(), "chat", "chats")),
            text("."),
        ]);
    }
    let work: i64 = slots.iter().map(|slot| slot.work_ms).sum();
    if work >= 1_000 {
        let mut parts = vec![
            text("Your agents ran for "),
            Part::Strong(clock_duration(work)),
        ];
        if slots.len() > 1
            && let Some(busiest) = slots.iter().max_by_key(|slot| slot.work_ms)
        {
            parts.push(Part::Text(format!(
                ", mostly in the {}",
                busiest.label.to_lowercase()
            )));
        }
        parts.push(text("."));
        sentences.push(parts);
    }
    if open > 0 || done > 0 {
        let mut parts = if open == 0 {
            vec![
                text(if done == 1 {
                    "You finished your "
                } else {
                    "You finished all "
                }),
                Part::Strong(plural(done, "task", "tasks")),
                text(" for today"),
            ]
        } else if done > 0 {
            vec![
                text("You finished "),
                Part::Strong(plural(done, "task", "tasks")),
                text(" today, "),
                Part::Strong(format!("{open} to go")),
            ]
        } else {
            vec![
                text("You have "),
                Part::Strong(plural(open, "task", "tasks")),
                text(" to do today"),
            ]
        };
        if overdue > 0 && open > 0 {
            parts.push(text(", "));
            parts.push(Part::Bad(format!("{overdue} overdue")));
        }
        parts.push(text("."));
        sentences.push(parts);
    }
    sentences
}

pub fn tightest_window<'a>(
    accounts: impl IntoIterator<Item = &'a AgentAccount>,
) -> Option<(HarnessId, &'a AgentUsageWindow)> {
    accounts
        .into_iter()
        .filter(|account| account.active)
        .flat_map(|account| {
            account
                .usage_windows
                .iter()
                .map(move |window| (account.harness, window))
        })
        .max_by(|a, b| a.1.used_fraction.total_cmp(&b.1.used_fraction))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn local(y: i32, m: u32, d: u32, h: u32, min: u32) -> DateTime<Local> {
        Local.with_ymd_and_hms(y, m, d, h, min, 0).unwrap()
    }

    fn pr(url: &str, authored: bool, requested: bool, ci_failed: bool) -> ChangeRequestListItem {
        serde_json::from_value(serde_json::json!({
            "provider": "github",
            "repository": "reina/zeron",
            "headRefOid": "0123456789abcdef0123456789abcdef01234567",
            "ci": { "state": if ci_failed { "failed" } else { "passed" } },
            "viewerDidAuthor": authored,
            "viewerReviewRequested": requested,
            "number": 1,
            "title": url,
            "url": url,
            "state": "open",
            "isDraft": false,
            "reviewDecision": "reviewRequired",
            "additions": 1,
            "deletions": 1,
            "mergeability": "mergeable",
            "createdAt": "2026-10-17T00:00:00Z",
            "updatedAt": "2026-10-17T00:00:00Z",
        }))
        .unwrap()
    }

    #[test]
    fn attention_prs_dedupe_by_url_and_skip_quiet_ones() {
        let mine = [pr("u/1", true, false, true), pr("u/2", true, false, false)];
        let review = [pr("u/1", true, false, true), pr("u/3", false, true, false)];
        let flagged = attention_prs(mine.iter().chain(&review));
        let urls: Vec<&str> = flagged.iter().map(|(item, _)| item.url.as_str()).collect();
        assert_eq!(urls, ["u/1", "u/3"]);
        assert_eq!(
            attention_row(&flagged),
            (
                "2 pull requests need you".to_owned(),
                "GitHub, 1 to review · 1 failing CI".to_owned()
            )
        );
        assert_eq!(
            attention_row(&flagged[1..]),
            ("1 pull request to review".to_owned(), "GitHub".to_owned())
        );
    }

    #[test]
    fn updated_label_reads_like_the_mockup() {
        assert_eq!(
            updated_label(Some(local(2026, 10, 16, 9, 40)), false).as_deref(),
            Some("Updated 9:40 AM")
        );
        assert_eq!(
            updated_label(Some(local(2026, 10, 16, 21, 5)), true).as_deref(),
            Some("Updated 9:05 PM, refresh failed")
        );
        assert_eq!(
            updated_label(None, true).as_deref(),
            Some("Runs did not load")
        );
        assert_eq!(updated_label(None, false), None);
    }

    #[test]
    fn greeting_follows_a_day_that_starts_at_four() {
        assert_eq!(greeting(local(2026, 10, 16, 3, 59)), "Good evening");
        assert_eq!(greeting(local(2026, 10, 16, 4, 0)), "Good morning");
        assert_eq!(greeting(local(2026, 10, 16, 11, 59)), "Good morning");
        assert_eq!(greeting(local(2026, 10, 16, 12, 0)), "Good afternoon");
        assert_eq!(greeting(local(2026, 10, 16, 17, 59)), "Good afternoon");
        assert_eq!(greeting(local(2026, 10, 16, 18, 0)), "Good evening");
        assert_eq!(
            header_line(local(2026, 10, 17, 2, 30), 0, 0),
            "Good evening. Friday, October 16. You’re all caught up."
        );
        assert_eq!(
            header_line(local(2026, 10, 17, 9, 0), 1, 0),
            "Good morning. Saturday, October 17. 1 thing needs you."
        );
        assert_eq!(
            header_line(local(2026, 10, 17, 9, 0), 5, 1),
            "Good morning. Saturday, October 17. 5 things need you. 1 task to do."
        );
        assert_eq!(
            header_line(local(2026, 10, 17, 13, 0), 0, 3),
            "Good afternoon. Saturday, October 17. 3 tasks to do."
        );
    }

    fn run(chat: &str, at: DateTime<Local>, ms: Option<i64>) -> TodayRun {
        TodayRun {
            chat_id: chat.into(),
            title: None,
            harness: None,
            started_at_ms: at.timestamp_millis(),
            duration_ms: ms,
            status: TodayRunStatus::Complete,
        }
    }

    #[test]
    fn the_day_counts_prompts_per_slot_from_four_am() {
        let now = local(2026, 10, 17, 14, 30);
        let runs = vec![
            run("a", local(2026, 10, 17, 3, 59), Some(60_000)),
            run("a", local(2026, 10, 17, 9, 10), Some(120_000)),
            run("b", local(2026, 10, 17, 9, 40), Some(60_000)),
            run("b", local(2026, 10, 17, 13, 5), Some(30_000)),
        ];
        let slots = day_slots(&runs, now);
        let summary: Vec<(&str, bool, usize)> = slots
            .iter()
            .map(|slot| (slot.label, slot.now, slot.prompts))
            .collect();
        assert_eq!(summary, [("Morning", false, 2), ("Afternoon", true, 1)]);
        assert_eq!(slots[0].hours.len(), 8);
        assert_eq!(slots[0].hours[5], (2, false));
        assert_eq!(slots[1].hours[1], (1, false));
        assert_eq!(slots[1].hours[3], (0, true));
        let sentences = digest(&runs[1..], &slots, 1, 0, 0);
        assert_eq!(
            sentences.last().unwrap(),
            &vec![
                Part::Text("You have ".into()),
                Part::Strong("1 task".into()),
                Part::Text(" to do today".into()),
                Part::Text(".".into()),
            ]
        );
        assert_eq!(
            sentences[1],
            vec![
                Part::Text("Your agents ran for ".into()),
                Part::Strong("3m".into()),
                Part::Text(", mostly in the morning".into()),
                Part::Text(".".into()),
            ]
        );
    }

    #[test]
    fn resets_in_reads_minutes_hours_then_days() {
        let now = Utc.with_ymd_and_hms(2026, 10, 17, 12, 0, 0).unwrap();
        let later = |secs: i64| now + chrono::Duration::seconds(secs);
        assert_eq!(resets_in(now, now), None);
        assert_eq!(resets_in(later(-60), now), None);
        assert_eq!(resets_in(later(30), now).as_deref(), Some("1m"));
        assert_eq!(resets_in(later(59 * 60), now).as_deref(), Some("59m"));
        assert_eq!(resets_in(later(3600), now).as_deref(), Some("1h"));
        assert_eq!(
            resets_in(later(2 * 3600 + 14 * 60), now).as_deref(),
            Some("2h 14m")
        );
        assert_eq!(resets_in(later(24 * 3600), now).as_deref(), Some("1d"));
        assert_eq!(
            resets_in(later(3 * 86_400 + 4 * 3600), now).as_deref(),
            Some("3d 4h")
        );
    }

    fn task(
        id: &str,
        due: Option<&str>,
        chat: Option<&str>,
        completed: Option<i64>,
    ) -> Task {
        serde_json::from_value(serde_json::json!({
            "id": id,
            "title": id,
            "dueOn": due,
            "chatId": chat,
            "orderKey": id,
            "completedAt": completed.map(|s| Utc.timestamp_opt(s, 0).unwrap()),
        }))
        .unwrap()
    }

    #[test]
    fn today_tasks_keep_due_linked_and_done_today() {
        let today = NaiveDate::from_ymd_opt(2026, 10, 17).unwrap();
        let day_start = Utc.with_ymd_and_hms(2026, 10, 17, 0, 0, 0).unwrap();
        let mut deleted = task("deleted", Some("2026-10-17"), None, None);
        deleted.deleted = true;
        let start = day_start.timestamp();
        let tasks = vec![
            task("a-overdue", Some("2026-10-16"), None, None),
            task("b-today", Some("2026-10-17"), None, None),
            task("c-tomorrow", Some("2026-10-18"), None, None),
            task("d-backlog", None, None, None),
            task("e-linked", None, Some("chat"), None),
            task("e-linked-dead", None, Some("deleted-chat"), None),
            task(
                "f-done-yesterday",
                Some("2026-10-16"),
                None,
                Some(start - 60),
            ),
            task("g-done-early", None, None, Some(start + 60)),
            task("h-done-late", Some("2026-10-20"), None, Some(start + 600)),
            deleted,
        ];
        let chats: Vec<Chat> = vec![
            serde_json::from_value(serde_json::json!({
                "id": "chat", "deviceId": "local", "archived": false, "createdAt": day_start,
            }))
            .unwrap(),
        ];
        let ids: Vec<&str> = today_tasks(&tasks, &chats, today, day_start)
            .into_iter()
            .map(|task| task.id.as_str())
            .collect();
        assert_eq!(
            ids,
            [
                "a-overdue",
                "b-today",
                "e-linked",
                "h-done-late",
                "g-done-early"
            ]
        );
    }
}
