use std::collections::{HashMap, HashSet};

use chrono::{DateTime, Local, Utc};
use zeron_proto::Chat;

use super::task_led_by;
use super::thread_tree::{NodeKind, TreeNode, tree_rows};
use super::tree::Status;
use crate::state::AppState;

#[derive(Debug, Clone)]
pub(crate) struct IndexRow {
    pub root: String,
    pub title: String,
    pub status: Status,
    pub line: String,
    pub asks: bool,
    pub segments: Vec<Status>,
    pub started: DateTime<Utc>,
    pub ended: Option<DateTime<Utc>>,
}

#[derive(Debug, Default)]
pub(crate) struct IndexRows {
    pub running: Vec<IndexRow>,
    pub done_today: Vec<IndexRow>,
}

fn is_today(at: DateTime<Utc>, now: DateTime<Utc>) -> bool {
    at.with_timezone(&Local).date_naive() == now.with_timezone(&Local).date_naive()
}

fn roots_a_tree(
    state: &AppState,
    chat: &Chat,
    children: &HashMap<&str, Vec<&Chat>>,
    nested: &HashSet<&str>,
) -> bool {
    if chat.archived || chat.is_system() || state.is_hidden_chat(&chat.id) {
        return false;
    }
    !nested.contains(chat.id.as_str())
        && children.contains_key(chat.id.as_str())
        && task_led_by(&state.tasks, &chat.id).is_none()
}

pub(crate) fn tree_index(state: &AppState, now: DateTime<Utc>) -> IndexRows {
    let children = super::thread_tree::children_by_parent(state);
    let nested: HashSet<&str> = children
        .iter()
        .flat_map(|(parent, kids)| {
            kids.iter()
                .filter(move |kid| kid.id != *parent)
                .map(|kid| kid.id.as_str())
        })
        .collect();
    let mut rows = IndexRows::default();
    for chat in &state.chats {
        if !roots_a_tree(state, chat, &children, &nested) {
            continue;
        }
        let nodes = tree_rows(state, &chat.id, false, now);
        if nodes.is_empty() {
            continue;
        }
        let live = nodes.iter().any(|node| node.status.live());
        let last = nodes
            .iter()
            .filter_map(|node| node.chat_id.as_deref())
            .filter_map(|id| state.chats.iter().find(|chat| chat.id == id))
            .filter_map(|chat| chat.last_message_at)
            .max();
        let settled_today = chat
            .settle
            .filter(|settle| {
                settle.settled && chat.last_message_at.is_none_or(|at| at <= settle.at)
            })
            .map(|settle| settle.at);
        let done = match settled_today {
            Some(at) if is_today(at, now) => Some(at),
            Some(_) => continue,
            None if live => None,
            None => match last {
                Some(at) if is_today(at, now) => Some(at),
                _ => continue,
            },
        };
        let row = index_row(state, chat, &nodes, done);
        if done.is_some() {
            rows.done_today.push(row);
        } else {
            rows.running.push(row);
        }
    }
    let latest = |row: &IndexRow| std::cmp::Reverse(row.ended.unwrap_or(row.started));
    rows.running.sort_by_key(latest);
    rows.done_today.sort_by_key(latest);
    rows
}

fn index_row(
    state: &AppState,
    chat: &Chat,
    nodes: &[TreeNode],
    done: Option<DateTime<Utc>>,
) -> IndexRow {
    let asker = nodes
        .iter()
        .find(|node| node.status == Status::Waiting && node.kind != NodeKind::Root);
    let root_waits = nodes[0].status == Status::Waiting;
    let agents_running = nodes
        .iter()
        .filter(|node| {
            matches!(node.kind, NodeKind::Agent | NodeKind::Native)
                && node.status == Status::Running
        })
        .count();
    let line = if let Some(asker) = asker {
        format!("Needs you \u{b7} {} asks", asker.title)
    } else if root_waits {
        format!("Needs you \u{b7} {}", nodes[0].title)
    } else if done.is_some() {
        "Done".into()
    } else if agents_running > 0 {
        format!(
            "Watching {agents_running} {}",
            if agents_running == 1 {
                "agent"
            } else {
                "agents"
            }
        )
    } else if nodes[0].status == Status::Running {
        "Working".into()
    } else {
        "Idle".into()
    };
    let status = if done.is_some() {
        Status::Done
    } else if asker.is_some() || root_waits {
        Status::Waiting
    } else if nodes[0].status == Status::Done && nodes.iter().any(|node| node.status.live()) {
        Status::Running
    } else {
        nodes[0].status
    };
    let started = nodes[1..]
        .iter()
        .filter_map(|node| node.chat_id.as_deref())
        .filter_map(|id| state.chats.iter().find(|chat| chat.id == id))
        .map(|chat| chat.created_at)
        .min()
        .unwrap_or(chat.created_at);
    IndexRow {
        root: chat.id.clone(),
        title: crate::transcript::single_line(&crate::home::chat_title(chat)),
        status,
        line,
        asks: asker.is_some() || root_waits,
        segments: nodes[1..].iter().map(|node| node.status).collect(),
        started,
        ended: done,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chat(id: &str, at: DateTime<Utc>) -> Chat {
        serde_json::from_value(serde_json::json!({
            "id": id, "title": id, "deviceId": "mac", "archived": false,
            "createdAt": at, "lastMessageAt": at,
        }))
        .unwrap()
    }

    #[test]
    fn only_zeron_children_or_a_plan_root_a_row_and_a_settled_tree_is_done_today() {
        let now = Utc::now();
        let mut state = AppState::new();
        state.local_device_id = Some("mac".into());
        let mut native_only = chat("native-only", now);
        native_only.last_message_at = Some(now);
        let mut sidechat = chat("side", now);
        sidechat.sidechat_source_chat_id = Some("side-source".into());
        let mut fork = chat("fork", now);
        fork.forked_from_chat_id = Some("fork-source".into());
        fork.created_by_chat_id = Some("fork-source".into());
        let mut child = chat("child", now);
        child.created_by_chat_id = Some("parent".into());
        let mut grandchild = chat("grandchild", now);
        grandchild.created_by_chat_id = Some("child".into());
        let mut settled = chat("settled", now - chrono::Duration::minutes(5));
        settled.settle = Some(zeron_proto::SidebarSettle {
            settled: true,
            at: now,
        });
        let mut settled_child = chat("settled-child", now - chrono::Duration::minutes(5));
        settled_child.created_by_chat_id = Some("settled".into());
        state.chats = vec![
            native_only,
            chat("side-source", now),
            sidechat,
            chat("fork-source", now),
            fork,
            chat("parent", now),
            child,
            grandchild,
            settled,
            settled_child,
        ];
        state.sessions = vec![
            serde_json::from_value(serde_json::json!({
                "chatId": "native-only", "deviceId": "mac", "status": "idle",
                "runningSubagents": 2, "updatedAt": now,
            }))
            .unwrap(),
        ];
        let rows = tree_index(&state, now);
        let ids = |rows: &[IndexRow]| -> Vec<String> {
            rows.iter().map(|row| row.root.clone()).collect()
        };
        let all: Vec<String> = ids(&rows.running)
            .into_iter()
            .chain(ids(&rows.done_today))
            .collect();
        assert!(
            !all.contains(&"native-only".to_string()),
            "native sub-agents alone never root a row"
        );
        assert!(
            !all.contains(&"side-source".to_string()),
            "a sidechat is no child"
        );
        assert!(
            !all.contains(&"fork-source".to_string()),
            "a fork is no child"
        );
        assert!(
            !all.contains(&"child".to_string()),
            "a child sits in its parent's tree, not its own row"
        );
        assert!(all.contains(&"parent".to_string()));
        let mut done = ids(&rows.done_today);
        done.sort();
        assert_eq!(done, ["parent", "settled"].map(String::from));
        let parent = rows
            .done_today
            .iter()
            .find(|row| row.root == "parent")
            .unwrap();
        assert_eq!(parent.segments.len(), 2);
    }

    #[test]
    fn only_a_local_live_main_chat_without_an_open_plan_can_add_one() {
        let now = Utc::now();
        let mut state = AppState::new();
        state.local_device_id = Some("mac".into());
        let mut remote = chat("remote", now);
        remote.device_id = "pc".into();
        let mut archived = chat("archived", now);
        archived.archived = true;
        let mut side = chat("side", now);
        side.sidechat_source_chat_id = Some("plain".into());
        state.chats = vec![
            chat("plain", now),
            remote,
            archived,
            side,
            chat("planned", now),
            chat("was-planned", now),
        ];
        let task = |id: &str, lead: &str, done: Option<DateTime<Utc>>| -> zeron_proto::Task {
            serde_json::from_value(serde_json::json!({
                "id": id, "title": id, "orderKey": id, "chatId": lead,
                "mode": "orchestrated", "completedAt": done,
            }))
            .unwrap()
        };
        state.tasks = vec![
            task("open", "planned", None),
            task("done", "was-planned", Some(now)),
        ];
        let can: Vec<&str> = state
            .chats
            .iter()
            .filter(|chat| crate::tasks::can_add_plan(&state, chat))
            .map(|chat| chat.id.as_str())
            .collect();
        assert_eq!(can, ["plain", "was-planned"]);
    }
}
