use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use chrono::{DateTime, SecondsFormat};
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use zeron_proto::computer::{
    COMPUTER_AUDIT_HISTORY_MAX_LIMIT, ComputerAuditEffect, ComputerAuditHistoryEntry,
    ComputerAuditHistoryStatus, ComputerGetAuditHistoryInput, ComputerGetAuditHistoryResult,
    is_computer_audit_app, is_computer_audit_identifier,
};

use super::audit_file::{
    ComputerAuditReadError, computer_audit_tail_lines, read_computer_audit_file_tail,
};

pub const COMPUTER_AUDIT_HISTORY_MAX_BYTES: u64 = 2 * 1024 * 1024;
pub const COMPUTER_AUDIT_HISTORY_MAX_ROWS: usize = 10_000;
const MAX_LINE_BYTES: usize = 64 * 1024;
const DEFAULT_LIMIT: u32 = 30;
const MAX_SAFE_INTEGER: u64 = (1 << 53) - 1;

static TIMESTAMP: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}:[0-9]{2}:[0-9]{2}(?:\.[0-9]{1,3})?Z$")
        .expect("audit timestamp pattern")
});
static TOOL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^computer_[a-z_]{1,80}$").expect("audit tool pattern"));
static CODE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^[a-z][a-z0-9_]{0,95}$").expect("audit code pattern"));

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ComputerAuditHistoryError {
    #[error("Computer activity history supports between 1 and 100 entries per page.")]
    InvalidLimit,
    #[error("Invalid Computer activity cursor. Refresh the history to continue.")]
    InvalidCursor,
    #[error(transparent)]
    Read(#[from] ComputerAuditReadError),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct AuditPosition {
    file: String,
    offset: u64,
    fingerprint: String,
}

fn digest(value: impl AsRef<[u8]>) -> String {
    Sha256::digest(value)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn is_fingerprint(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| matches!(b, b'a'..=b'f' | b'0'..=b'9'))
}

fn cursor_for(position: &AuditPosition) -> String {
    URL_SAFE_NO_PAD.encode(serde_json::to_vec(position).unwrap_or_default())
}

fn parse_cursor(cursor: Option<&str>) -> Result<Option<AuditPosition>, ComputerAuditHistoryError> {
    let Some(cursor) = cursor else {
        return Ok(None);
    };
    if cursor.len() > 512
        || cursor.is_empty()
        || !cursor
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
    {
        return Err(ComputerAuditHistoryError::InvalidCursor);
    }
    URL_SAFE_NO_PAD
        .decode(cursor)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<AuditPosition>(&bytes).ok())
        .filter(|position| {
            is_fingerprint(&position.file)
                && is_fingerprint(&position.fingerprint)
                && position.offset <= MAX_SAFE_INTEGER
        })
        .map(Some)
        .ok_or(ComputerAuditHistoryError::InvalidCursor)
}

fn safe_identifier(value: Option<&Value>) -> Option<String> {
    value
        .and_then(Value::as_str)
        .filter(|value| is_computer_audit_identifier(value))
        .map(str::to_owned)
}

fn project_entry(line: &[u8], position: &AuditPosition) -> Option<ComputerAuditHistoryEntry> {
    let record: Value = serde_json::from_str(&String::from_utf8_lossy(line)).ok()?;
    let record = record.as_object()?;
    let ts = record
        .get("ts")
        .and_then(Value::as_str)
        .filter(|ts| TIMESTAMP.is_match(ts))
        .and_then(|ts| DateTime::parse_from_rfc3339(ts).ok())?;
    let tool = record
        .get("tool")
        .and_then(Value::as_str)
        .filter(|tool| TOOL.is_match(tool))?;
    let effect: ComputerAuditEffect = record
        .get("effect")
        .filter(|effect| effect.is_string())
        .and_then(|effect| serde_json::from_value(effect.clone()).ok())?;
    Some(ComputerAuditHistoryEntry {
        id: digest(cursor_for(position)),
        ts: ts.to_utc().to_rfc3339_opts(SecondsFormat::Millis, true),
        tool: tool.to_owned(),
        effect,
        thread_id: safe_identifier(record.get("threadId")),
        turn_id: safe_identifier(record.get("turnId")),
        gateway_request_id: safe_identifier(record.get("gatewayRequestId")),
        code: record
            .get("code")
            .and_then(Value::as_str)
            .filter(|code| CODE.is_match(code))
            .map(str::to_owned),
        app: record
            .get("target")
            .and_then(|target| target.get("app"))
            .and_then(Value::as_str)
            .filter(|app| is_computer_audit_app(app))
            .map(str::to_owned),
    })
}

fn with_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut path = path.as_os_str().to_owned();
    path.push(suffix);
    path.into()
}

// Blocking file reads: async callers go through `ComputerAuditLog::read_history`.
pub fn read_computer_audit_history(
    file_path: Option<&Path>,
    input: &ComputerGetAuditHistoryInput,
) -> Result<ComputerGetAuditHistoryResult, ComputerAuditHistoryError> {
    let limit = input.limit.unwrap_or(DEFAULT_LIMIT);
    if !(1..=COMPUTER_AUDIT_HISTORY_MAX_LIMIT).contains(&limit) {
        return Err(ComputerAuditHistoryError::InvalidLimit);
    }
    let before = parse_cursor(input.before.as_deref())?;
    let Some(file_path) = file_path else {
        return Ok(ComputerGetAuditHistoryResult {
            entries: Vec::new(),
            next_cursor: None,
            truncated: false,
            status: ComputerAuditHistoryStatus::Disabled,
        });
    };

    let mut records: Vec<(AuditPosition, ComputerAuditHistoryEntry)> = Vec::new();
    let mut seen_files = HashSet::new();
    let mut remaining_bytes = COMPUTER_AUDIT_HISTORY_MAX_BYTES;
    let mut scanned_rows = 0;
    let mut found_file = false;
    let mut truncated = false;
    for path in [
        file_path.to_path_buf(),
        with_suffix(file_path, ".1"),
        with_suffix(file_path, ".2"),
    ] {
        let budget = if scanned_rows < COMPUTER_AUDIT_HISTORY_MAX_ROWS {
            remaining_bytes
        } else {
            0
        };
        let Some(tail) = read_computer_audit_file_tail(&path, budget)? else {
            continue;
        };
        remaining_bytes -= tail.requested_bytes;
        found_file = true;
        let file_id = digest(format!("{}:{}", tail.device, tail.inode));
        if !seen_files.insert(file_id.clone()) {
            continue;
        }
        truncated |= tail.start > 0
            || tail.contents.len() as u64 != tail.requested_bytes
            || tail.contents.last().is_some_and(|byte| *byte != b'\n');
        for (terminated, offset) in computer_audit_tail_lines(&tail) {
            if scanned_rows >= COMPUTER_AUDIT_HISTORY_MAX_ROWS {
                truncated = true;
                break;
            }
            scanned_rows += 1;
            let line = &terminated[..terminated.len() - 1];
            if line.is_empty() {
                continue;
            }
            if line.len() > MAX_LINE_BYTES {
                truncated = true;
                continue;
            }
            let position = AuditPosition {
                file: file_id.clone(),
                offset,
                fingerprint: digest(line),
            };
            match project_entry(line, &position) {
                Some(entry) => records.push((position, entry)),
                None => truncated = true,
            }
        }
    }

    let status = if found_file {
        ComputerAuditHistoryStatus::Available
    } else {
        ComputerAuditHistoryStatus::Missing
    };
    let mut first = 0;
    if let Some(before) = before {
        let exact = records.iter().position(|(position, _)| *position == before);
        let index = exact.or_else(|| {
            let mut matches = records
                .iter()
                .enumerate()
                .filter(|(_, (position, _))| position.fingerprint == before.fingerprint)
                .map(|(index, _)| index);
            match (matches.next(), matches.next()) {
                (Some(index), None) => Some(index),
                _ => None,
            }
        });
        let Some(index) = index else {
            return Ok(ComputerGetAuditHistoryResult {
                entries: Vec::new(),
                next_cursor: None,
                truncated: true,
                status,
            });
        };
        first = index + 1;
    }
    let page = &records[first.min(records.len())..(first + limit as usize).min(records.len())];
    let next_cursor = page
        .last()
        .filter(|_| first + page.len() < records.len())
        .map(|(position, _)| cursor_for(position));
    Ok(ComputerGetAuditHistoryResult {
        entries: page.iter().map(|(_, entry)| entry.clone()).collect(),
        next_cursor,
        truncated,
        status,
    })
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn fixture() -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("computer-audit.jsonl");
        (dir, path)
    }

    fn line(index: i64, extra: Value) -> String {
        let mut value = json!({
            "ts": "2026-09-20T12:00:00.000Z",
            "tool": "computer_click",
            "effect": "dispatched-unknown",
            "gatewayRequestId": index.to_string(),
        });
        for (key, field) in extra.as_object().unwrap() {
            value[key] = field.clone();
        }
        format!("{value}\n")
    }

    fn plain(index: i64) -> String {
        line(index, json!({}))
    }

    fn lines(indexes: &[i64]) -> String {
        indexes.iter().map(|index| plain(*index)).collect()
    }

    fn read(
        path: &Path,
        limit: Option<u32>,
        before: Option<&str>,
    ) -> ComputerGetAuditHistoryResult {
        read_computer_audit_history(
            Some(path),
            &ComputerGetAuditHistoryInput {
                limit,
                before: before.map(str::to_owned),
            },
        )
        .unwrap()
    }

    fn ids(result: &ComputerGetAuditHistoryResult) -> Vec<&str> {
        result
            .entries
            .iter()
            .map(|entry| entry.gateway_request_id.as_deref().unwrap())
            .collect()
    }

    fn append(path: &Path, content: &str) {
        use std::io::Write;
        std::fs::OpenOptions::new()
            .append(true)
            .open(path)
            .unwrap()
            .write_all(content.as_bytes())
            .unwrap();
    }

    fn replace(path: &Path, content: &str) {
        let temporary = with_suffix(path, ".tmp");
        std::fs::write(&temporary, content).unwrap();
        std::fs::rename(&temporary, path).unwrap();
    }

    fn empty_truncated(status: ComputerAuditHistoryStatus) -> ComputerGetAuditHistoryResult {
        ComputerGetAuditHistoryResult {
            entries: Vec::new(),
            next_cursor: None,
            truncated: true,
            status,
        }
    }

    #[test]
    fn distinguishes_absent_logging_no_history_and_an_empty_retained_log() {
        let (_dir, path) = fixture();
        let disabled =
            read_computer_audit_history(None, &ComputerGetAuditHistoryInput::default()).unwrap();
        assert_eq!(disabled.status, ComputerAuditHistoryStatus::Disabled);
        assert!(disabled.entries.is_empty());
        assert_eq!(
            read(&path, None, None).status,
            ComputerAuditHistoryStatus::Missing
        );
        std::fs::write(&path, "").unwrap();
        assert_eq!(
            read(&path, None, None),
            ComputerGetAuditHistoryResult {
                status: ComputerAuditHistoryStatus::Available,
                entries: Vec::new(),
                next_cursor: None,
                truncated: false,
            }
        );
    }

    #[test]
    fn projects_only_bounded_identifiers_and_typed_outcomes_never_payloads_or_paths() {
        let (_dir, path) = fixture();
        let content = line(
            1,
            json!({
                "threadId": "thread-1",
                "turnId": "turn-1",
                "effect": "refused",
                "code": "computer_control_revoked",
                "args": { "text": "secret", "url": "https://private.test/?token=secret" },
                "target": { "app": "Finder", "windowId": "secret", "path": "/private/secret" },
                "result": "secret",
                "password": "secret",
            }),
        ) + &line(
            2,
            json!({
                "threadId": "/private/secret",
                "code": "secret\ncredential",
                "turnId": "x".repeat(129),
            }),
        );
        std::fs::write(&path, content).unwrap();
        let result = read(&path, None, None);
        let wire = serde_json::to_value(&result).unwrap();
        assert_eq!(
            serde_json::from_value::<ComputerGetAuditHistoryResult>(wire.clone()).unwrap(),
            result
        );
        assert!(!wire.to_string().contains("secret"));
        let older = &result.entries[1];
        assert_eq!(older.thread_id.as_deref(), Some("thread-1"));
        assert_eq!(older.turn_id.as_deref(), Some("turn-1"));
        assert_eq!(older.gateway_request_id.as_deref(), Some("1"));
        assert_eq!(older.code.as_deref(), Some("computer_control_revoked"));
        assert_eq!(older.effect, ComputerAuditEffect::Refused);
        assert_eq!(older.app.as_deref(), Some("Finder"));
        let newest = serde_json::to_string(&result.entries[0]).unwrap();
        let keys: Vec<&str> = Regex::new(r#""([A-Za-z]+)":"#)
            .unwrap()
            .captures_iter(&newest)
            .map(|capture| capture.get(1).unwrap().as_str())
            .collect();
        assert_eq!(keys, ["id", "ts", "tool", "effect", "gatewayRequestId"]);
    }

    #[test]
    fn projects_the_target_app_only_as_bounded_single_line_text() {
        let (_dir, path) = fixture();
        let content = line(1, json!({ "target": { "app": "Calculator", "pid": 42 } }))
            + &line(2, json!({ "target": { "app": "Calc\nulator" } }))
            + &line(3, json!({ "target": { "app": "x".repeat(129) } }))
            + &line(4, json!({ "target": { "app": 7 } }));
        std::fs::write(&path, content).unwrap();
        let apps: Vec<_> = read(&path, None, None)
            .entries
            .into_iter()
            .map(|entry| entry.app)
            .collect();
        assert_eq!(apps, [None, None, None, Some("Calculator".into())]);
    }

    #[test]
    fn keeps_append_order_pagination_stable_when_newer_actions_arrive() {
        let (_dir, path) = fixture();
        std::fs::write(&path, lines(&[1, 2, 3, 4])).unwrap();
        let first = read(&path, Some(2), None);
        assert_eq!(ids(&first), ["4", "3"]);
        append(&path, &plain(5));
        let second = read(&path, Some(2), first.next_cursor.as_deref());
        assert_eq!(ids(&second), ["2", "1"]);
        assert_eq!(second.next_cursor, None);
        assert!(!second.truncated);
    }

    #[cfg_attr(
        windows,
        ignore = "audit_file::identity is a path hash on Windows (stable std has no file index), so a rotated or replaced file keeps its identity"
    )]
    #[test]
    fn reads_known_rotations_and_preserves_distinct_identical_records() {
        let (_dir, path) = fixture();
        std::fs::write(&path, lines(&[1, 1, 2])).unwrap();
        let first = read(&path, Some(2), None);
        std::fs::rename(&path, with_suffix(&path, ".1")).unwrap();
        std::fs::write(&path, plain(3)).unwrap();
        std::fs::write(with_suffix(&path, ".2"), plain(0)).unwrap();
        let second = read(&path, None, first.next_cursor.as_deref());
        assert_eq!(ids(&second), ["1", "0"]);
        assert_ne!(second.entries[0].id, first.entries[1].id);
        assert!(!second.truncated);
    }

    #[test]
    fn recovers_a_uniquely_retained_cursor_after_compaction_and_marks_expired_cursors() {
        let (_dir, path) = fixture();
        std::fs::write(&path, lines(&[1, 2, 3, 4])).unwrap();
        let first = read(&path, Some(2), None);
        replace(&path, &lines(&[2, 3, 4, 5]));
        let retained = read(&path, None, first.next_cursor.as_deref());
        assert_eq!(ids(&retained), ["2"]);
        replace(&path, &plain(5));
        assert_eq!(
            read(&path, None, first.next_cursor.as_deref()),
            empty_truncated(ComputerAuditHistoryStatus::Available)
        );
    }

    #[cfg_attr(
        windows,
        ignore = "audit_file::identity is a path hash on Windows (stable std has no file index), so a rotated or replaced file keeps its identity"
    )]
    #[test]
    fn does_not_guess_an_ambiguous_duplicate_cursor_after_compaction() {
        let (_dir, path) = fixture();
        std::fs::write(&path, lines(&[1, 1])).unwrap();
        let first = read(&path, Some(1), None);
        replace(&path, &lines(&[1, 1, 2]));
        assert_eq!(
            read(&path, None, first.next_cursor.as_deref()),
            empty_truncated(ComputerAuditHistoryStatus::Available)
        );
    }

    #[test]
    fn bounds_bytes_and_skips_oversized_malformed_and_incomplete_records() {
        let (_dir, path) = fixture();
        std::fs::write(
            &path,
            "x".repeat(COMPUTER_AUDIT_HISTORY_MAX_BYTES as usize + 1)
                + "\n{invalid}\n"
                + &plain(1)
                + "{\"secret\":",
        )
        .unwrap();
        let result = read(&path, Some(100), None);
        assert_eq!(ids(&result), ["1"]);
        assert!(result.truncated);
    }

    #[test]
    fn bounds_scanned_rows_and_results_including_an_old_cursor_outside_the_tail() {
        let (_dir, path) = fixture();
        std::fs::write(&path, lines(&[1, 2])).unwrap();
        let initial = read(&path, Some(1), None);
        append(&path, &plain(3).repeat(COMPUTER_AUDIT_HISTORY_MAX_ROWS + 1));
        let result = read(&path, Some(100), None);
        assert_eq!(result.entries.len(), 100);
        assert!(result.truncated);
        let expired = read(&path, None, initial.next_cursor.as_deref());
        assert!(expired.entries.is_empty());
        assert!(expired.truncated);
    }

    #[cfg(unix)]
    #[test]
    fn refuses_symbolic_links_instead_of_following_arbitrary_paths() {
        let (_dir, path) = fixture();
        let private = with_suffix(&path, ".private");
        std::fs::write(&private, plain(1)).unwrap();
        std::os::unix::fs::symlink(&private, &path).unwrap();
        let error =
            read_computer_audit_history(Some(&path), &ComputerGetAuditHistoryInput::default())
                .unwrap_err();
        assert_eq!(
            error.to_string(),
            "Could not read Computer activity history."
        );
    }

    #[test]
    fn rejects_invalid_limits_and_cursor_formats_without_treating_them_as_paths() {
        let (_dir, path) = fixture();
        let attempt = |limit: Option<u32>, before: &str| {
            read_computer_audit_history(
                Some(&path),
                &ComputerGetAuditHistoryInput {
                    limit,
                    before: (!before.is_empty()).then(|| before.to_owned()),
                },
            )
            .unwrap_err()
            .to_string()
        };
        assert!(attempt(Some(101), "").contains("between 1 and 100"));
        assert!(attempt(None, "../../private").contains("Invalid Computer activity cursor"));
        assert!(attempt(None, "e30").contains("Invalid Computer activity cursor"));
    }

    #[test]
    fn normalizes_timestamps_to_millisecond_utc() {
        let (_dir, path) = fixture();
        std::fs::write(&path, line(1, json!({ "ts": "2026-09-20T12:00:00Z" }))).unwrap();
        assert_eq!(
            read(&path, None, None).entries[0].ts,
            "2026-09-20T12:00:00.000Z"
        );
    }
}
