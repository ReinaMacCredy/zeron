use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use chrono::Local;
use zeron_doc::WorkspaceState;
use zeron_proto::today::{self, Part};
use zeron_proto::{TodayRun, mission};

use super::{DeriveContext, DeviceView, HostRef, NeedsYouEntry, NeedsYouKind, SessionRow};

#[derive(Debug, Clone, Default, PartialEq)]
pub struct TodayPage {
    pub greeting: String,
    pub summary: String,
    pub tasks: Vec<TodayTask>,
    pub open_tasks: u32,
    pub prompts: u32,
    pub digest: Vec<Vec<TodayPart>>,
    pub slots: Vec<TodaySlot>,
    pub usage: Option<TodayUsage>,
    pub pull_requests: Option<TodayPullRequests>,
    pub runs_loaded: bool,
    pub updated: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TodayPullRequests {
    pub title: String,
    pub meta: String,
    pub count: u32,
    pub device_id: String,
    pub repository: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TodayTask {
    pub id: String,
    pub title: String,
    pub meta: Option<String>,
    pub overdue: bool,
    pub done: bool,
    pub host: Option<HostRef>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum TodayPart {
    Text(String),
    Strong(String),
    Bad(String),
}

#[derive(Debug, Clone, PartialEq)]
pub struct TodaySlot {
    pub label: String,
    pub now: bool,
    pub prompts: u32,
    pub hours: Vec<(u32, bool)>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TodayUsage {
    pub harness: String,
    pub harness_name: String,
    pub window: String,
    pub used_fraction: f32,
    pub resets_in: Option<String>,
    pub host: HostRef,
}

pub(crate) fn derive(
    state: &WorkspaceState,
    rows: &HashMap<String, Arc<SessionRow>>,
    devices: &[DeviceView],
    needs_you: &[NeedsYouEntry],
    cx: &DeriveContext<'_>,
) -> TodayPage {
    let now = cx.now.with_timezone(&Local);
    let day_start = today::work_day_start(now);
    let today_key = today::work_day(now).format("%Y-%m-%d").to_string();
    let host = |device_id: &str| {
        let device = devices.iter().find(|d| d.id == device_id);
        HostRef {
            device_id: device_id.to_owned(),
            name: device.map_or_else(|| "Unknown device".to_owned(), |d| d.name.clone()),
            online: device.is_some_and(|d| d.online),
        }
    };

    let tasks: Vec<TodayTask> = today::today_tasks(
        &state.tasks,
        &state.chats,
        today::work_day(now),
        day_start.with_timezone(&chrono::Utc),
    )
    .into_iter()
    .map(|task| {
        let done = task.completed_at.is_some();
        let overdue = today::is_overdue(task, &state.chats, &today_key);
        let linked = mission::linked_chat(&state.chats, task);
        let meta = if done {
            None
        } else if let Some(chat) = linked {
            let title = rows.get(&chat.id).map(|row| row.title.clone());
            Some(format!("In {}", title.unwrap_or_else(|| "a chat".to_owned())))
        } else if overdue {
            Some("Overdue".to_owned())
        } else {
            task.due_on.as_ref().map(|_| "Today".to_owned())
        };
        TodayTask {
            id: task.id.clone(),
            title: task.title.clone(),
            meta,
            overdue,
            done,
            host: super::tasks::task_device(state, task).map(|device| host(&device)),
        }
    })
    .collect();
    let open = tasks.iter().filter(|t| !t.done).count();
    let overdue = tasks.iter().filter(|t| t.overdue).count();

    let since = day_start.timestamp_millis();
    let mut runs: Vec<TodayRun> = cx
        .today_runs
        .values()
        .flat_map(|day| &day.runs)
        .filter(|run| run.started_at_ms >= since)
        .cloned()
        .collect();
    runs.sort_by_key(|run| run.started_at_ms);
    let slots = today::day_slots(&runs, now);
    let digest = today::digest(&runs, &slots, open, tasks.len() - open, overdue)
        .into_iter()
        .map(|parts| {
            parts
                .into_iter()
                .map(|part| match part {
                    Part::Text(s) => TodayPart::Text(s),
                    Part::Strong(s) => TodayPart::Strong(s),
                    Part::Bad(s) => TodayPart::Bad(s),
                })
                .collect()
        })
        .collect();

    let usage = cx
        .accounts
        .iter()
        .filter_map(|(device, snapshot)| {
            today::tightest_window(&snapshot.accounts).map(|(harness, window)| (device, harness, window))
        })
        .max_by(|a, b| a.2.used_fraction.total_cmp(&b.2.used_fraction))
        .map(|(device, harness, window)| TodayUsage {
            harness: serde_json::to_value(harness)
                .ok()
                .and_then(|v| v.as_str().map(str::to_owned))
                .unwrap_or_default(),
            harness_name: harness.display_name().to_owned(),
            window: window.label.clone(),
            used_fraction: window.used_fraction.clamp(0.0, 1.0),
            resets_in: window.resets_at.and_then(|at| zeron_proto::account_usage::resets_in(at, cx.now)),
            host: host(device),
        });

    let asking: HashSet<&str> = needs_you
        .iter()
        .filter_map(|entry| match &entry.kind {
            NeedsYouKind::Input { chat_id, .. } => Some(chat_id.as_str()),
            _ => None,
        })
        .collect();
    let mut hosts: Vec<_> = cx.today_prs.iter().collect();
    hosts.sort_by(|a, b| a.0.cmp(b.0));
    let flagged = today::attention_prs(hosts.iter().flat_map(|(_, items)| items.iter()));
    let pull_requests = flagged.first().map(|(first, _)| {
        let (title, meta) = today::attention_row(&flagged);
        TodayPullRequests {
            title,
            meta,
            count: flagged.len() as u32,
            device_id: hosts
                .iter()
                .find(|(_, items)| items.iter().any(|item| item.url == first.url))
                .map(|(device, _)| (*device).clone())
                .unwrap_or_default(),
            repository: first.repository.clone(),
        }
    });
    let line = today::header_line(now, asking.len() + flagged.len(), open);
    let greeting = today::greeting(now);
    TodayPage {
        greeting: greeting.to_owned(),
        summary: line
            .strip_prefix(greeting)
            .map_or(line.as_str(), |rest| rest.trim_start_matches(". "))
            .to_owned(),
        tasks,
        open_tasks: open as u32,
        prompts: runs.len() as u32,
        digest,
        slots: slots
            .into_iter()
            .map(|slot| TodaySlot {
                label: slot.label.to_owned(),
                now: slot.now,
                prompts: slot.prompts as u32,
                hours: slot
                    .hours
                    .into_iter()
                    .map(|(count, future)| (count as u32, future))
                    .collect(),
            })
            .collect(),
        usage,
        pull_requests,
        runs_loaded: !cx.today_runs.is_empty(),
        updated: today::updated_label(cx.today_refresh.updated, cx.today_refresh.failed),
    }
}
