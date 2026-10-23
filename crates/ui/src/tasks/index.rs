use chrono::{DateTime, Utc};

use super::thread_tree::Source;
use crate::state::AppState;

pub(crate) use zeron_doc::tree::{IndexRow, IndexRows};

pub(crate) fn tree_index(state: &AppState, now: DateTime<Utc>) -> IndexRows {
    zeron_doc::tree::tree_index(&Source { state, now })
}

#[cfg(test)]
mod tests {
    use super::*;
    use zeron_proto::Chat;

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
