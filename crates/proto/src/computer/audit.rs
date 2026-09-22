use serde::{Deserialize, Deserializer, Serialize};

use super::bounds::{self, chars_in};

pub const COMPUTER_AUDIT_HISTORY_MAX_LIMIT: u32 = 100;

pub fn is_computer_audit_identifier(value: &str) -> bool {
    chars_in(value, 1, 128, |c| {
        c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | ':' | '-')
    })
}

pub fn is_computer_audit_cursor(value: &str) -> bool {
    chars_in(value, 1, 512, |c| {
        c.is_ascii_alphanumeric() || matches!(c, '_' | '-')
    })
}

fn is_entry_id(value: &str) -> bool {
    chars_in(value, 64, 64, |c| matches!(c, 'a'..='f' | '0'..='9'))
}

fn is_tool(value: &str) -> bool {
    value
        .strip_prefix("computer_")
        .is_some_and(|rest| chars_in(rest, 1, 80, |c| c.is_ascii_lowercase() || c == '_'))
}

fn is_code(value: &str) -> bool {
    value.starts_with(|c: char| c.is_ascii_lowercase())
        && chars_in(value, 1, 96, |c| {
            c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_'
        })
}

fn entry_id<'de, D: Deserializer<'de>>(d: D) -> Result<String, D::Error> {
    bounds::matching(d, is_entry_id)
}

fn tool<'de, D: Deserializer<'de>>(d: D) -> Result<String, D::Error> {
    bounds::matching(d, is_tool)
}

fn identifier<'de, D: Deserializer<'de>>(d: D) -> Result<Option<String>, D::Error> {
    bounds::opt_matching(d, is_computer_audit_identifier)
}

fn code<'de, D: Deserializer<'de>>(d: D) -> Result<Option<String>, D::Error> {
    bounds::opt_matching(d, is_code)
}

fn cursor<'de, D: Deserializer<'de>>(d: D) -> Result<Option<String>, D::Error> {
    bounds::opt_matching(d, is_computer_audit_cursor)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ComputerAuditEffect {
    Verified,
    DispatchedUnknown,
    NotDispatched,
    Refused,
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ComputerAuditHistoryEntry {
    #[serde(deserialize_with = "entry_id")]
    pub id: String,
    pub ts: String,
    #[serde(deserialize_with = "tool")]
    pub tool: String,
    pub effect: ComputerAuditEffect,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "identifier"
    )]
    pub thread_id: Option<String>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "identifier"
    )]
    pub turn_id: Option<String>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "identifier"
    )]
    pub gateway_request_id: Option<String>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "code"
    )]
    pub code: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ComputerGetAuditHistoryInput {
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "bounds::opt_int::<_, u32, 1, { COMPUTER_AUDIT_HISTORY_MAX_LIMIT as i64 }>"
    )]
    pub limit: Option<u32>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "cursor"
    )]
    pub before: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ComputerAuditHistoryStatus {
    Available,
    Disabled,
    Missing,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ComputerGetAuditHistoryResult {
    #[serde(
        deserialize_with = "bounds::items::<_, _, 0, { COMPUTER_AUDIT_HISTORY_MAX_LIMIT as usize }>"
    )]
    pub entries: Vec<ComputerAuditHistoryEntry>,
    #[serde(deserialize_with = "cursor")]
    pub next_cursor: Option<String>,
    pub truncated: bool,
    pub status: ComputerAuditHistoryStatus,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};

    #[test]
    fn accepts_bounded_history_pages_and_rejects_oversized_or_path_like_cursors() {
        let input: ComputerGetAuditHistoryInput =
            serde_json::from_value(json!({ "limit": 100, "before": "YWJj" })).unwrap();
        assert_eq!(
            input,
            ComputerGetAuditHistoryInput {
                limit: Some(100),
                before: Some("YWJj".into()),
            }
        );
        for bad in [
            json!({ "limit": 0 }),
            json!({ "limit": 101 }),
            json!({ "limit": 1.5 }),
            json!({ "before": "../private" }),
            json!({ "before": "x".repeat(513) }),
        ] {
            assert!(
                serde_json::from_value::<ComputerGetAuditHistoryInput>(bad.clone()).is_err(),
                "{bad}"
            );
        }
    }

    fn entry() -> Value {
        json!({
            "id": "a".repeat(64),
            "ts": "2026-09-20T12:00:00.000Z",
            "tool": "computer_click",
            "effect": "dispatched-unknown",
        })
    }

    #[test]
    fn rejects_unknown_outcomes_and_unbounded_result_pages() {
        let decode = |value: Value| serde_json::from_value::<ComputerGetAuditHistoryResult>(value);
        let result = json!({ "entries": [entry()], "status": "available", "nextCursor": null, "truncated": false });
        assert_eq!(
            serde_json::to_value(decode(result.clone()).unwrap()).unwrap(),
            result
        );
        let mut unknown = entry();
        unknown["effect"] = json!("success");
        let mut bad = result.clone();
        bad["entries"] = json!([unknown]);
        assert!(decode(bad).is_err());
        let mut oversized = result;
        oversized["entries"] = Value::Array(vec![entry(); 101]);
        assert!(decode(oversized).is_err());
    }

    #[test]
    fn entry_fields_keep_their_patterns() {
        let decode = |patch: Value| {
            let mut value = entry();
            for (key, field) in patch.as_object().unwrap() {
                value[key] = field.clone();
            }
            serde_json::from_value::<ComputerAuditHistoryEntry>(value)
        };
        assert!(decode(json!({ "threadId": "chat-1", "code": "approval_queue_full" })).is_ok());
        for bad in [
            json!({ "id": "A".repeat(64) }),
            json!({ "id": "a".repeat(63) }),
            json!({ "tool": "click" }),
            json!({ "tool": "computer_Click" }),
            json!({ "threadId": "has space" }),
            json!({ "code": "1bad" }),
            json!({ "code": format!("a{}", "b".repeat(96)) }),
        ] {
            assert!(decode(bad.clone()).is_err(), "{bad}");
        }
    }
}
