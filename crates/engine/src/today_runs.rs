use std::collections::HashMap;

use loro::{Container, LoroDoc, LoroValue, ToJson, ValueOrContainer};
use serde::Deserialize;
use zeron_doc::{MessageRole, MessageStatus};
use zeron_proto::{Chat, TodayRun, TodayRunStatus, TodayRuns, TodayWait};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Turn {
    pub started_at_ms: i64,
    pub duration_ms: Option<i64>,
    pub status: TodayRunStatus,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct EntryHead {
    id: String,
    role: MessageRole,
    #[serde(default)]
    created_at: i64,
    #[serde(default)]
    status: Option<MessageStatus>,
    #[serde(default)]
    continuation_of: Option<String>,
    #[serde(default)]
    duration_ms: Option<i64>,
}

pub(crate) fn turns_since(doc: &LoroDoc, since_ms: i64) -> Vec<Turn> {
    let messages = doc.get_list("messages");
    let mut roots = HashMap::<String, usize>::new();
    let mut turns: Vec<Turn> = Vec::new();
    for index in 0..messages.len() {
        let Some(ValueOrContainer::Container(Container::Map(map))) = messages.get(index) else {
            continue;
        };
        let Ok(head) = serde_json::from_value::<EntryHead>(map.get_value().to_json_value()) else {
            continue;
        };
        if head.role != MessageRole::Assistant {
            continue;
        }
        let status = match head.status {
            Some(MessageStatus::Streaming) => TodayRunStatus::Running,
            Some(MessageStatus::Aborted) => TodayRunStatus::Aborted,
            _ => TodayRunStatus::Complete,
        };
        if let Some(&at) = head.continuation_of.as_ref().and_then(|id| roots.get(id)) {
            let turn = &mut turns[at];
            turn.status = status;
            turn.duration_ms = head
                .duration_ms
                .map(|ms| (head.created_at + ms - turn.started_at_ms).max(0));
            continue;
        }
        roots.insert(head.id, turns.len());
        turns.push(Turn {
            started_at_ms: head.created_at,
            duration_ms: head.duration_ms,
            status,
        });
    }
    turns.retain(|turn| turn.started_at_ms >= since_ms);
    turns
}

/// Questions stamped with when they were asked, as `(asked, answered)`; older parts carry no stamp.
pub(crate) fn waits_since(doc: &LoroDoc, since_ms: i64) -> Vec<(i64, Option<i64>)> {
    let messages = doc.get_list("messages");
    let mut waits = Vec::new();
    for index in 0..messages.len() {
        let Some(ValueOrContainer::Container(Container::Map(map))) = messages.get(index) else {
            continue;
        };
        let Some(ValueOrContainer::Container(Container::List(parts))) = map.get("parts") else {
            continue;
        };
        for part in 0..parts.len() {
            let Some(ValueOrContainer::Container(Container::Map(part))) = parts.get(part) else {
                continue;
            };
            let field = |key: &str| match part.get(key) {
                Some(ValueOrContainer::Value(value)) => Some(value),
                _ => None,
            };
            let stamp = |key: &str| match field(key) {
                Some(LoroValue::I64(ms)) => Some(ms),
                _ => None,
            };
            let input =
                matches!(field("kind"), Some(LoroValue::String(kind)) if kind.as_str() == "input");
            if input
                && let Some(asked) = stamp("askedAtMs")
                && stamp("answeredAtMs").is_none_or(|ended| ended >= since_ms)
            {
                waits.push((asked, stamp("answeredAtMs")));
            }
        }
    }
    waits
}

pub(crate) fn today_runs(
    chats: &[Chat],
    since_ms: i64,
    chat_ids: Option<&[String]>,
    mut read: impl FnMut(&Chat) -> (Vec<Turn>, Vec<(i64, Option<i64>)>),
) -> TodayRuns {
    let mut runs = Vec::new();
    let mut waits = Vec::new();
    let picked = chats.iter().filter(|chat| match chat_ids {
        Some(ids) => ids.iter().any(|id| *id == chat.id),
        None => {
            !chat.archived
                && chat.is_top_level()
                && chat
                    .last_message_at
                    .is_some_and(|at| at.timestamp_millis() >= since_ms)
        }
    });
    for chat in picked {
        let (turns, chat_waits) = read(chat);
        runs.extend(turns.into_iter().map(|turn| TodayRun {
            chat_id: chat.id.clone(),
            title: chat.title.clone(),
            harness: chat.config.as_ref().map(|config| config.harness),
            started_at_ms: turn.started_at_ms,
            duration_ms: turn.duration_ms,
            status: turn.status,
        }));
        waits.extend(chat_waits.into_iter().map(|(asked, ended)| TodayWait {
            chat_id: chat.id.clone(),
            started_at_ms: asked,
            ended_at_ms: ended,
        }));
    }
    runs.sort_by_key(|run| std::cmp::Reverse(run.started_at_ms));
    TodayRuns { runs, waits }
}

#[cfg(test)]
mod tests {
    use super::*;
    use zeron_doc::{MessagePart, SessionDoc, SessionMessageEntry};

    fn entry(
        id: &str,
        role: MessageRole,
        created_at: i64,
        status: Option<MessageStatus>,
        continuation_of: Option<&str>,
        duration_ms: Option<i64>,
    ) -> SessionMessageEntry {
        SessionMessageEntry {
            id: id.into(),
            role,
            parts: vec![MessagePart::Text {
                id: format!("{id}-t"),
                text: "hi".into(),
            }],
            created_at,
            device_id: "d".into(),
            status,
            continuation_of: continuation_of.map(Into::into),
            from_chat_id: None,
            fork_import: false,
            answers_request_id: None,
            duration_ms,
        }
    }

    #[test]
    fn turns_join_continuations_and_skip_turns_before_since() {
        use MessageRole::{Assistant, User};
        use MessageStatus::{Aborted, Complete, Streaming};
        let doc = SessionDoc::init("c").unwrap();
        for e in [
            entry("u0", User, 500, None, None, None),
            entry("a0", Assistant, 600, Some(Complete), None, Some(100)),
            entry("u1", User, 1_000, None, None, None),
            entry("a1", Assistant, 1_100, Some(Complete), None, Some(200)),
            entry(
                "a1b",
                Assistant,
                1_400,
                Some(Complete),
                Some("a1"),
                Some(300),
            ),
            entry("a2", Assistant, 2_000, Some(Aborted), None, Some(50)),
            entry("a3", Assistant, 3_000, Some(Streaming), None, None),
        ] {
            doc.push_message(&e).unwrap();
        }
        assert_eq!(
            turns_since(doc.doc(), 1_000),
            vec![
                Turn {
                    started_at_ms: 1_100,
                    duration_ms: Some(600),
                    status: TodayRunStatus::Complete,
                },
                Turn {
                    started_at_ms: 2_000,
                    duration_ms: Some(50),
                    status: TodayRunStatus::Aborted,
                },
                Turn {
                    started_at_ms: 3_000,
                    duration_ms: None,
                    status: TodayRunStatus::Running,
                },
            ]
        );
    }

    fn chat(id: &str, last_ms: Option<i64>) -> Chat {
        serde_json::from_value(serde_json::json!({
            "id": id,
            "deviceId": "d",
            "archived": false,
            "createdAt": "2026-10-17T00:00:00Z",
            "lastMessageAt": last_ms.map(|ms| chrono::DateTime::from_timestamp_millis(ms).unwrap()),
        }))
        .unwrap()
    }

    #[test]
    fn rows_come_from_chats_active_since_newest_first() {
        let mut archived = chat("archived", Some(5_000));
        archived.archived = true;
        let chats = vec![
            chat("stale", Some(500)),
            chat("idle", None),
            chat("a", Some(2_500)),
            chat("b", Some(4_000)),
            archived,
        ];
        let mut opened = Vec::new();
        let runs = today_runs(&chats, 1_000, None, |chat| {
            opened.push(chat.id.clone());
            let start = if chat.id == "a" { 2_000 } else { 3_000 };
            let turn = Turn {
                started_at_ms: start,
                duration_ms: Some(10),
                status: TodayRunStatus::Complete,
            };
            (vec![turn], Vec::new())
        });
        assert_eq!(opened, ["a", "b"]);
        let order: Vec<(&str, i64)> = runs
            .runs
            .iter()
            .map(|run| (run.chat_id.as_str(), run.started_at_ms))
            .collect();
        assert_eq!(order, [("b", 3_000), ("a", 2_000)]);
    }

    #[test]
    fn explicit_chats_skip_the_inbox_filters_and_bring_their_stamped_waits() {
        let doc = SessionDoc::init("c").unwrap();
        let question = |id: &str, asked: Option<i64>, answered: Option<i64>| MessagePart::Input {
            id: format!("in-{id}"),
            request_id: id.into(),
            questions: Vec::new(),
            resolved: answered.is_some(),
            answers: None,
            asked_at_ms: asked,
            answered_at_ms: answered,
        };
        let mut asks = entry("a0", MessageRole::Assistant, 900, None, None, None);
        asks.parts = vec![
            question("old", None, None),
            question("done", Some(1_200), Some(1_500)),
            question("open", Some(2_000), None),
            question("before", Some(100), Some(400)),
        ];
        doc.push_message(&asks).unwrap();
        assert_eq!(
            waits_since(doc.doc(), 1_000),
            vec![(1_200, Some(1_500)), (2_000, None)],
            "an unstamped question and a wait over before `since` are left out"
        );

        let mut child = chat("child", None);
        child.created_by_chat_id = Some("parent".into());
        child.delegation = serde_json::from_value(serde_json::json!({
            "delegatedByChatId": "parent", "depth": 1, "anchor": "m", "wake": "always", "status": "running",
            "startedAt": "2026-10-17T00:00:00Z",
        }))
        .unwrap();
        assert!(!child.is_top_level());
        let chats = vec![
            chat("parent", Some(5_000)),
            child,
            chat("other", Some(5_000)),
        ];
        let ids = ["child".to_string()];
        let runs = today_runs(&chats, 1_000, Some(&ids), |_| {
            (
                vec![Turn {
                    started_at_ms: 1_100,
                    duration_ms: Some(10),
                    status: TodayRunStatus::Complete,
                }],
                vec![(1_200, None)],
            )
        });
        assert_eq!(runs.runs.len(), 1);
        assert_eq!(runs.runs[0].chat_id, "child");
        assert_eq!(
            runs.waits,
            vec![TodayWait {
                chat_id: "child".into(),
                started_at_ms: 1_200,
                ended_at_ms: None,
            }]
        );
    }
}
