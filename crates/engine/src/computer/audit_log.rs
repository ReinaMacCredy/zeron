use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock, mpsc};

use chrono::{DateTime, SecondsFormat, Utc};
use regex::Regex;
use serde::Serialize;
use serde_json::{Map, Value, json};
use tokio::sync::oneshot;
use zeron_proto::computer::{
    ComputerAuditEffect, ComputerGetAuditHistoryInput, ComputerGetAuditHistoryResult,
    CuaActionDiagnostics, parse_cua_action_diagnostics,
};

use super::audit_file::{
    ComputerAuditReadError, append_private, computer_audit_tail_lines, create_private_dir,
    read_computer_audit_file_tail, replace_private,
};
use super::audit_history::{ComputerAuditHistoryError, read_computer_audit_history};
use super::utf8_truncation::utf16_len;

pub const COMPUTER_AUDIT_FILE_NAME: &str = "computer-audit.jsonl";
pub const COMPUTER_AUDIT_MAX_ENTRIES: usize = 10_000;
pub const COMPUTER_AUDIT_MAX_BYTES: u64 = 2 * 1024 * 1024;
const COMPUTER_AUDIT_COMPACT_TO: usize = COMPUTER_AUDIT_MAX_ENTRIES / 2;
const COMPUTER_AUDIT_COMPACT_BYTES: usize = (COMPUTER_AUDIT_MAX_BYTES / 2) as usize;
const COMPUTER_AUDIT_MAX_STRING: usize = 256;
const COMPUTER_AUDIT_MAX_ITEMS: usize = 64;

const COMPUTER_AUDIT_SENSITIVE_ARGS: &[&str] = &[
    "text",
    "value",
    "arguments",
    "files",
    "clipboard",
    "contents",
    "data",
    "payload",
];

static GATEWAY_REQUEST_ID: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^[A-Za-z0-9_.:-]{1,128}$").expect("gateway id pattern"));

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ComputerAuditLayer {
    DriverHost,
    NativeDriver,
    ServerManager,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ComputerAuditTarget {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub window_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pid: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub app: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bundle_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ComputerAuditEntry {
    pub tool: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thread_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub turn_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gateway_request_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target: Option<ComputerAuditTarget>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub args: Option<Map<String, Value>>,
    pub effect: ComputerAuditEffect,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
    #[serde(skip)]
    pub diagnostics: Option<CuaActionDiagnostics>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub layer: Option<ComputerAuditLayer>,
}

impl ComputerAuditEntry {
    pub fn new(tool: impl Into<String>, effect: ComputerAuditEffect) -> Self {
        Self {
            tool: tool.into(),
            thread_id: None,
            turn_id: None,
            gateway_request_id: None,
            target: None,
            args: None,
            effect,
            code: None,
            diagnostics: None,
            layer: None,
        }
    }
}

#[derive(Serialize)]
struct AuditLine<'a> {
    ts: String,
    #[serde(flatten)]
    entry: &'a ComputerAuditEntry,
    #[serde(skip_serializing_if = "Option::is_none")]
    diagnostics: Option<CuaActionDiagnostics>,
}

pub fn computer_audit_gateway_request_id(value: &Value) -> Option<String> {
    let id = match value {
        Value::String(id) => id.clone(),
        Value::Number(number) if number.as_f64().is_some_and(f64::is_finite) => number.to_string(),
        _ => return None,
    };
    GATEWAY_REQUEST_ID.is_match(&id).then_some(id)
}

fn is_sensitive(key: &str) -> bool {
    COMPUTER_AUDIT_SENSITIVE_ARGS.contains(&key)
}

fn js_slice(value: &str, max_units: usize) -> &str {
    let mut units = 0;
    let end = value
        .char_indices()
        .find(|(_, character)| {
            units += character.len_utf16();
            units > max_units
        })
        .map_or(value.len(), |(index, _)| index);
    &value[..end]
}

fn chars(value: &str) -> Value {
    json!({ "chars": utf16_len(value) })
}

pub fn summarize_computer_audit_args(args: &Map<String, Value>) -> Map<String, Value> {
    args.iter()
        .filter_map(|(key, value)| Some((key.clone(), summarize_value(key, value)?)))
        .collect()
}

fn step_type(step: &Value) -> String {
    match step.get("type") {
        Some(Value::String(kind)) => kind.clone(),
        Some(Value::Null) | None => "unknown".to_owned(),
        Some(Value::Bool(flag)) => flag.to_string(),
        Some(Value::Number(number)) => number.to_string(),
        Some(_) => "[object]".to_owned(),
    }
}

fn summarize_value(key: &str, value: &Value) -> Option<Value> {
    Some(match value {
        Value::Null => return None,
        Value::Bool(_) | Value::Number(_) => value.clone(),
        Value::String(text) if is_sensitive(key) => chars(text),
        Value::String(text) if utf16_len(text) > COMPUTER_AUDIT_MAX_STRING => {
            Value::String(format!("{}…", js_slice(text, COMPUTER_AUDIT_MAX_STRING)))
        }
        Value::String(_) => value.clone(),
        Value::Array(items) if key == "steps" => json!({
            "count": items.len(),
            "types": items
                .iter()
                .take(COMPUTER_AUDIT_MAX_ITEMS)
                .map(|step| if step.is_object() { step_type(step) } else { "unknown".to_owned() })
                .collect::<Vec<_>>(),
        }),
        Value::Array(items) if is_sensitive(key) => json!({ "items": items.len() }),
        Value::Array(items) => Value::Array(
            items
                .iter()
                .take(COMPUTER_AUDIT_MAX_ITEMS)
                .map(|item| match item {
                    Value::Bool(_) | Value::Number(_) => item.clone(),
                    Value::String(text) => {
                        Value::String(js_slice(text, COMPUTER_AUDIT_MAX_STRING).to_owned())
                    }
                    _ => Value::String("[object]".to_owned()),
                })
                .collect(),
        ),
        Value::Object(fields) => Value::Object(
            fields
                .iter()
                .map(|(inner_key, inner)| {
                    let summary = match inner {
                        Value::Bool(_) | Value::Number(_) | Value::Null => inner.clone(),
                        Value::String(text) if is_sensitive(inner_key) => chars(text),
                        Value::String(text) => {
                            Value::String(js_slice(text, COMPUTER_AUDIT_MAX_STRING).to_owned())
                        }
                        _ => Value::String("[object]".to_owned()),
                    };
                    (inner_key.clone(), summary)
                })
                .collect(),
        ),
    })
}

enum Operation {
    Append(String),
    Flush(oneshot::Sender<()>),
    Read(
        ComputerGetAuditHistoryInput,
        oneshot::Sender<Result<ComputerGetAuditHistoryResult, ComputerAuditHistoryError>>,
    ),
}

type Clock = Arc<dyn Fn() -> DateTime<Utc> + Send + Sync>;

pub struct ComputerAuditLog {
    file_path: Option<PathBuf>,
    now: Clock,
    writer: Option<mpsc::Sender<Operation>>,
}

impl ComputerAuditLog {
    pub fn new(dir: Option<&Path>) -> Self {
        Self::with_clock(dir, Arc::new(Utc::now))
    }

    pub fn with_clock(dir: Option<&Path>, now: Clock) -> Self {
        let file_path = dir.map(|dir| dir.join(COMPUTER_AUDIT_FILE_NAME));
        let writer = file_path.clone().and_then(|path| {
            let (sender, receiver) = mpsc::channel();
            std::thread::Builder::new()
                .name("computer-audit".into())
                .spawn(move || AuditWriter::new(path).run(receiver))
                .ok()
                .map(|_| sender)
        });
        Self {
            file_path,
            now,
            writer,
        }
    }

    pub fn file_path(&self) -> Option<&Path> {
        self.file_path.as_deref()
    }

    pub fn record(&self, entry: &ComputerAuditEntry) {
        let Some(writer) = &self.writer else {
            return;
        };
        let diagnostics = entry
            .diagnostics
            .as_ref()
            .and_then(|supplied| parse_cua_action_diagnostics(&json!({ "diagnostics": supplied })));
        let line = AuditLine {
            ts: (self.now)().to_rfc3339_opts(SecondsFormat::Millis, true),
            entry,
            diagnostics,
        };
        let Ok(mut line) = serde_json::to_string(&line) else {
            return;
        };
        line.push('\n');
        if line.len() as u64 > COMPUTER_AUDIT_MAX_BYTES {
            return;
        }
        let _ = writer.send(Operation::Append(line));
    }

    pub async fn flush(&self) {
        let Some(writer) = &self.writer else {
            return;
        };
        let (done, wait) = oneshot::channel();
        if writer.send(Operation::Flush(done)).is_ok() {
            let _ = wait.await;
        }
    }

    pub async fn read_history(
        &self,
        input: ComputerGetAuditHistoryInput,
    ) -> Result<ComputerGetAuditHistoryResult, ComputerAuditHistoryError> {
        let Some(writer) = &self.writer else {
            return read_computer_audit_history(None, &input);
        };
        let (reply, wait) = oneshot::channel();
        writer
            .send(Operation::Read(input, reply))
            .map_err(|_| ComputerAuditHistoryError::Read(ComputerAuditReadError))?;
        wait.await
            .map_err(|_| ComputerAuditHistoryError::Read(ComputerAuditReadError))?
    }
}

struct AuditWriter {
    path: PathBuf,
    entries: usize,
    bytes: u64,
    loaded: bool,
    needs_separator: bool,
}

impl AuditWriter {
    fn new(path: PathBuf) -> Self {
        Self {
            path,
            entries: 0,
            bytes: 0,
            loaded: false,
            needs_separator: false,
        }
    }

    fn run(mut self, operations: mpsc::Receiver<Operation>) {
        for operation in operations {
            match operation {
                Operation::Append(line) => {
                    let _ = self.append(&line);
                }
                Operation::Flush(done) => {
                    let _ = done.send(());
                }
                Operation::Read(input, reply) => {
                    let _ = reply.send(read_computer_audit_history(Some(&self.path), &input));
                }
            }
        }
    }

    fn append(&mut self, line: &str) -> Result<(), Box<dyn std::error::Error>> {
        if !self.loaded {
            if let Some(existing) =
                read_computer_audit_file_tail(&self.path, COMPUTER_AUDIT_MAX_BYTES)?
            {
                self.bytes = existing.total_bytes;
                self.needs_separator = existing.contents.last().is_some_and(|byte| *byte != b'\n');
                self.entries = usize::from(self.needs_separator);
                let mut lines = computer_audit_tail_lines(&existing);
                while self.entries <= COMPUTER_AUDIT_MAX_ENTRIES && lines.next().is_some() {
                    self.entries += 1;
                }
            }
            self.loaded = true;
        }
        if let Some(parent) = self.path.parent() {
            create_private_dir(parent)?;
        }
        let content = if self.needs_separator {
            format!("\n{line}")
        } else {
            line.to_owned()
        };
        append_private(&self.path, content.as_bytes())?;
        self.needs_separator = false;
        self.entries += 1;
        self.bytes += content.len() as u64;
        if self.entries > COMPUTER_AUDIT_MAX_ENTRIES || self.bytes > COMPUTER_AUDIT_MAX_BYTES {
            self.compact()?;
        }
        Ok(())
    }

    fn compact(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        let Some(existing) = read_computer_audit_file_tail(&self.path, COMPUTER_AUDIT_MAX_BYTES)?
        else {
            return Ok(());
        };
        let mut kept: Vec<&[u8]> = Vec::new();
        let mut kept_bytes = 0;
        for (line, _) in computer_audit_tail_lines(&existing).take(COMPUTER_AUDIT_MAX_ENTRIES) {
            if line.len() <= 1 {
                continue;
            }
            if kept.len() >= COMPUTER_AUDIT_COMPACT_TO
                || (!kept.is_empty() && kept_bytes + line.len() > COMPUTER_AUDIT_COMPACT_BYTES)
            {
                break;
            }
            kept.push(line);
            kept_bytes += line.len();
        }
        let content: Vec<u8> = kept
            .iter()
            .rev()
            .flat_map(|line| line.iter().copied())
            .collect();
        replace_private(&self.path, &content)?;
        self.entries = kept.len();
        self.bytes = content.len() as u64;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::io::{Seek, SeekFrom, Write};

    use zeron_proto::computer::{CuaActuator, CuaDeliveryPath};

    use super::*;

    fn object(value: Value) -> Map<String, Value> {
        value.as_object().unwrap().clone()
    }

    fn read_lines(path: &Path) -> Vec<Value> {
        std::fs::read_to_string(path)
            .unwrap()
            .trim()
            .split('\n')
            .map(|line| serde_json::from_str(line).unwrap())
            .collect()
    }

    fn click(effect: ComputerAuditEffect) -> ComputerAuditEntry {
        ComputerAuditEntry::new("computer_click", effect)
    }

    #[test]
    fn records_typed_text_as_a_character_count_never_the_payload() {
        let summary = summarize_computer_audit_args(&object(json!({
            "text": "hunter2 -- a typed secret",
            "x": 10,
            "y": 20,
        })));
        assert_eq!(
            Value::Object(summary.clone()),
            json!({ "text": { "chars": 25 }, "x": 10, "y": 20 })
        );
        assert!(!Value::Object(summary).to_string().contains("hunter2"));
    }

    #[test]
    fn records_clipboard_and_file_payloads_as_counts_only() {
        let summary = summarize_computer_audit_args(&object(json!({
            "contents": "clipboard payload with a token",
            "value": "set_value payload",
            "arguments": ["--password=s3cret"],
            "files": ["/tmp/a", "/tmp/b"],
            "label": "OK",
        })));
        let text = Value::Object(summary.clone()).to_string();
        assert!(!text.contains("s3cret"));
        assert!(!text.contains("token"));
        assert!(!text.contains("/tmp/a"));
        assert_eq!(summary["contents"], json!({ "chars": 30 }));
        assert_eq!(summary["value"], json!({ "chars": 17 }));
        assert_eq!(summary["arguments"], json!({ "items": 1 }));
        assert_eq!(summary["files"], json!({ "items": 2 }));
        assert_eq!(summary["label"], json!("OK"));
    }

    #[test]
    fn keeps_a_computer_runs_step_shape_without_the_step_payloads() {
        let summary = summarize_computer_audit_args(&object(json!({
            "steps": [
                { "type": "click", "x": 1, "y": 2 },
                { "type": "type", "text": "password" },
            ],
        })));
        assert_eq!(
            summary["steps"],
            json!({ "count": 2, "types": ["click", "type"] })
        );
        assert!(!Value::Object(summary).to_string().contains("password"));
    }

    #[test]
    fn sanitizes_sensitive_keys_nested_inside_a_target_object() {
        let summary = summarize_computer_audit_args(&object(json!({
            "target": { "windowId": "w1", "value": "field payload", "x": 5 },
        })));
        assert_eq!(summary["target"]["value"], json!({ "chars": 13 }));
        assert_eq!(summary["target"]["windowId"], json!("w1"));
        assert!(!Value::Object(summary).to_string().contains("field payload"));
    }

    #[test]
    fn cuts_long_labels_and_drops_null_arguments() {
        let summary = summarize_computer_audit_args(&object(json!({
            "label": "l".repeat(300),
            "missing": null,
        })));
        assert_eq!(summary["label"], json!(format!("{}…", "l".repeat(256))));
        assert!(!summary.contains_key("missing"));
    }

    #[tokio::test]
    async fn persists_reviewed_native_diagnostics_but_strips_arbitrary_native_payloads() {
        let dir = tempfile::tempdir().unwrap();
        let log = ComputerAuditLog::new(Some(dir.path()));
        let driver_payload = json!({
            "diagnostics": {
                "delivery_path": "ax",
                "actuator": "ax_press",
                "ax_error": -25202,
                "window_title": "private window title",
                "message": "private field value",
                "text": "private typed content",
            }
        });
        let mut entry = click(ComputerAuditEffect::DispatchedUnknown);
        entry.code = Some("cua_action_failed".into());
        entry.diagnostics = parse_cua_action_diagnostics(&driver_payload);
        log.record(&entry);
        log.flush().await;
        let path = log.file_path().unwrap();
        let saved = std::fs::read_to_string(path).unwrap();
        assert_eq!(
            read_lines(path)[0]["diagnostics"],
            json!({ "delivery_path": "ax", "actuator": "ax_press", "ax_error": -25202 })
        );
        assert!(!saved.contains("private"));
        assert_eq!(
            entry.diagnostics.unwrap().delivery_path,
            Some(CuaDeliveryPath::Ax)
        );
        assert_eq!(
            parse_cua_action_diagnostics(&driver_payload)
                .unwrap()
                .actuator,
            Some(CuaActuator::AxPress)
        );
    }

    #[tokio::test]
    async fn appends_one_json_object_per_line_with_timestamp_target_and_effect() {
        let dir = tempfile::tempdir().unwrap();
        let log = ComputerAuditLog::new(Some(dir.path()));
        let mut first = click(ComputerAuditEffect::Verified);
        first.thread_id = Some("thread-1".into());
        first.turn_id = Some("turn-1".into());
        first.target = Some(ComputerAuditTarget {
            window_id: Some("w1".into()),
            pid: Some(42),
            app: Some("Finder".into()),
            bundle_id: None,
        });
        first.args = Some(object(json!({ "x": 10, "y": 20 })));
        log.record(&first);
        let mut second =
            ComputerAuditEntry::new("computer_type_text", ComputerAuditEffect::Refused);
        second.thread_id = Some("thread-1".into());
        second.target = Some(ComputerAuditTarget {
            window_id: Some("w1".into()),
            ..Default::default()
        });
        second.args = Some(object(json!({ "text": { "chars": 12 } })));
        second.code = Some("computer_denylist_refused".into());
        log.record(&second);
        log.flush().await;
        let path = log.file_path().unwrap();
        let lines = read_lines(path);
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0]["tool"], "computer_click");
        assert_eq!(lines[0]["threadId"], "thread-1");
        assert_eq!(lines[0]["turnId"], "turn-1");
        assert_eq!(
            lines[0]["target"],
            json!({ "windowId": "w1", "pid": 42, "app": "Finder" })
        );
        assert_eq!(lines[0]["effect"], "verified");
        assert!(
            Regex::new(r"^\d{4}-\d{2}-\d{2}T")
                .unwrap()
                .is_match(lines[0]["ts"].as_str().unwrap())
        );
        assert!(
            std::fs::read_to_string(path)
                .unwrap()
                .starts_with("{\"ts\":")
        );
        assert_eq!(lines[1]["effect"], "refused");
        assert_eq!(lines[1]["code"], "computer_denylist_refused");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(path).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600);
        }
    }

    #[tokio::test]
    async fn serializes_concurrent_appends_into_intact_ordered_lines() {
        let dir = tempfile::tempdir().unwrap();
        let log = ComputerAuditLog::new(Some(dir.path()));
        for index in 0..100 {
            let mut entry = click(ComputerAuditEffect::DispatchedUnknown);
            entry.thread_id = Some("thread".into());
            entry.args = Some(object(json!({ "index": index })));
            log.record(&entry);
        }
        log.flush().await;
        let lines = read_lines(log.file_path().unwrap());
        assert_eq!(lines.len(), 100);
        for (index, line) in lines.iter().enumerate() {
            assert_eq!(line["args"], json!({ "index": index }));
        }
    }

    #[tokio::test]
    async fn counts_a_pre_existing_log_toward_the_entry_cap_and_compacts_to_a_bounded_tail() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(COMPUTER_AUDIT_FILE_NAME);
        let seeded: String = (0..COMPUTER_AUDIT_MAX_ENTRIES)
            .map(|index| {
                format!(
                    "{}\n",
                    json!({ "ts": "old", "tool": "computer_click", "args": { "index": index } })
                )
            })
            .collect();
        std::fs::write(&path, seeded).unwrap();
        let log = ComputerAuditLog::new(Some(dir.path()));
        let mut entry = click(ComputerAuditEffect::Verified);
        entry.args = Some(object(json!({ "index": -1 })));
        log.record(&entry);
        log.flush().await;
        let lines = read_lines(&path);
        assert!(lines.len() < COMPUTER_AUDIT_MAX_ENTRIES);
        assert!(!lines.is_empty());
        assert_eq!(lines.last().unwrap()["args"], json!({ "index": -1 }));
    }

    #[tokio::test]
    async fn swallows_write_failures_instead_of_failing_the_recorded_action() {
        let dir = tempfile::tempdir().unwrap();
        let blocker = dir.path().join("blocker");
        std::fs::write(&blocker, "x").unwrap();
        let log = ComputerAuditLog::new(Some(&blocker));
        log.record(&click(ComputerAuditEffect::Verified));
        log.flush().await;
        assert!(!blocker.join(COMPUTER_AUDIT_FILE_NAME).exists());
    }

    #[tokio::test]
    async fn retains_a_bounded_utf8_tail_of_a_huge_legacy_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(COMPUTER_AUDIT_FILE_NAME);
        let prefix_bytes = 16 * COMPUTER_AUDIT_MAX_BYTES;
        let label = "界🙂".repeat(100);
        let tail: String = (0..2_000)
            .map(|index| {
                json!({
                    "ts": "old",
                    "tool": "computer_click",
                    "args": { "index": index, "label": label },
                })
                .to_string()
            })
            .collect::<Vec<_>>()
            .join("\n")
            + "\n";
        {
            let mut legacy = std::fs::File::create(&path).unwrap();
            legacy.set_len(prefix_bytes).unwrap();
            legacy.seek(SeekFrom::Start(prefix_bytes)).unwrap();
            legacy.write_all(tail.as_bytes()).unwrap();
        }
        let log = ComputerAuditLog::new(Some(dir.path()));
        for index in [-1, -2] {
            let mut entry = click(ComputerAuditEffect::Verified);
            entry.args = Some(object(json!({ "index": index })));
            log.record(&entry);
        }
        log.flush().await;
        let retained = std::fs::read_to_string(&path).unwrap();
        let entries = read_lines(&path);
        assert!(retained.len() as u64 <= COMPUTER_AUDIT_MAX_BYTES);
        assert!(entries.len() <= COMPUTER_AUDIT_MAX_ENTRIES);
        let indexes: Vec<i64> = entries
            .iter()
            .map(|entry| entry["args"]["index"].as_i64().unwrap())
            .collect();
        assert_eq!(indexes[indexes.len() - 2..], [-1, -2]);
        let historical = &entries[..entries.len() - 2];
        assert!(!historical.is_empty());
        assert_eq!(historical.last().unwrap()["args"]["index"], 1_999);
        assert!(
            historical
                .iter()
                .all(|entry| entry["args"]["label"] == label.as_str())
        );
        assert!(historical.windows(2).all(|pair| {
            pair[1]["args"]["index"].as_i64() == pair[0]["args"]["index"].as_i64().map(|i| i + 1)
        }));
    }

    #[tokio::test]
    async fn omits_oversized_evidence_without_poisoning_subsequent_appends() {
        let dir = tempfile::tempdir().unwrap();
        let log = ComputerAuditLog::new(Some(dir.path()));
        let mut oversized = click(ComputerAuditEffect::Verified);
        oversized.args = Some(object(
            json!({ "label": "界".repeat(COMPUTER_AUDIT_MAX_BYTES as usize) }),
        ));
        log.record(&oversized);
        let mut next = click(ComputerAuditEffect::Verified);
        next.gateway_request_id = Some("next".into());
        log.record(&next);
        log.flush().await;
        let lines = read_lines(log.file_path().unwrap());
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0]["gatewayRequestId"], "next");
    }

    #[tokio::test]
    async fn separates_an_interrupted_legacy_tail_from_the_next_complete_action() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join(COMPUTER_AUDIT_FILE_NAME),
            "{\"tool\":\"computer_click\"",
        )
        .unwrap();
        let log = ComputerAuditLog::new(Some(dir.path()));
        let mut next = click(ComputerAuditEffect::Verified);
        next.gateway_request_id = Some("next".into());
        log.record(&next);
        let history = log
            .read_history(ComputerGetAuditHistoryInput::default())
            .await
            .unwrap();
        assert_eq!(history.entries.len(), 1);
        assert_eq!(
            history.entries[0].gateway_request_id.as_deref(),
            Some("next")
        );
        assert!(history.truncated);
    }

    #[tokio::test]
    async fn waits_for_queued_evidence_before_reading_through_the_log_owner() {
        let dir = tempfile::tempdir().unwrap();
        let log = ComputerAuditLog::new(Some(dir.path()));
        let mut entry = click(ComputerAuditEffect::Verified);
        entry.gateway_request_id = Some("request-1".into());
        log.record(&entry);
        let history = log
            .read_history(ComputerGetAuditHistoryInput::default())
            .await
            .unwrap();
        assert_eq!(history.entries[0].tool, "computer_click");
        assert_eq!(history.entries[0].effect, ComputerAuditEffect::Verified);
        assert_eq!(
            history.entries[0].gateway_request_id.as_deref(),
            Some("request-1")
        );
    }

    #[tokio::test]
    async fn a_log_without_a_directory_records_nothing_and_reports_disabled() {
        let log = ComputerAuditLog::new(None);
        log.record(&click(ComputerAuditEffect::Verified));
        log.flush().await;
        let history = log
            .read_history(ComputerGetAuditHistoryInput::default())
            .await
            .unwrap();
        assert_eq!(
            history.status,
            zeron_proto::computer::ComputerAuditHistoryStatus::Disabled
        );
    }

    #[test]
    fn keeps_actual_bounded_transport_identities_and_omits_payload_like_values() {
        assert_eq!(
            computer_audit_gateway_request_id(&json!(42)),
            Some("42".into())
        );
        assert_eq!(
            computer_audit_gateway_request_id(&json!("rpc-request-1")),
            Some("rpc-request-1".into())
        );
        for invalid in [
            Value::Null,
            json!("/private/file"),
            json!("token\nvalue"),
            json!("x".repeat(129)),
        ] {
            assert_eq!(
                computer_audit_gateway_request_id(&invalid),
                None,
                "{invalid}"
            );
        }
    }
}
