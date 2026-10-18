use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use serde::Serialize;
use serde_json::{Map, Value, json};
use zeron_proto::ui_driver::{UiDriverDescriptor, run_dir};

use crate::ui_driver;

const ASSERT_TIMEOUT_MS: u64 = 3000;
const ENGINE_TIMEOUT: Duration = Duration::from_secs(30);
const RESULT_CHARS: usize = 160;

#[derive(Debug, PartialEq)]
struct Scenario {
    name: String,
    target: Option<String>,
    steps: Vec<Step>,
}

#[derive(Debug, PartialEq)]
struct Step {
    name: Option<String>,
    continue_on_fail: bool,
    save: Option<String>,
    kind: Kind,
}

#[derive(Debug, PartialEq)]
enum Kind {
    Engine {
        method: String,
        params: Value,
        first: bool,
    },
    Ui {
        action: String,
        args: Map<String, Value>,
    },
    Assert(Assert),
    Snapshot(String),
    Sleep(u64),
}

#[derive(Debug, PartialEq)]
enum Assert {
    Select {
        select: String,
        nth: Option<u64>,
        exists: bool,
        timeout_ms: u64,
    },
    Text {
        text: String,
        exists: bool,
        timeout_ms: u64,
    },
    Saved {
        path: String,
        expect: Expect,
    },
}

#[derive(Debug, PartialEq)]
enum Expect {
    Equals(Value),
    Contains(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
enum Status {
    Pass,
    Fail,
    Skipped,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
struct StepReport {
    index: usize,
    kind: &'static str,
    name: String,
    status: Status,
    ms: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    warning: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    shot: Option<String>,
}

trait Io {
    async fn ui(&mut self, request: Map<String, Value>) -> Result<Value, String>;
    async fn engine(&mut self, method: &str, params: Value, first: bool) -> Result<Value, String>;
}

struct Live {
    app: UiDriverDescriptor,
    rpc: Option<zeron_rpc::RpcClient>,
}

impl Io for Live {
    async fn ui(&mut self, request: Map<String, Value>) -> Result<Value, String> {
        ui_driver::request(&self.app, request).await
    }

    async fn engine(&mut self, method: &str, params: Value, first: bool) -> Result<Value, String> {
        if self.rpc.is_none() {
            let url = format!("ws://127.0.0.1:{}", self.app.ipc_port);
            let client = zeron_rpc::connect_ws(&url)
                .await
                .map_err(|error| format!("no engine at {url}: {error}"))?;
            self.rpc = Some(client);
        }
        let rpc = self.rpc.as_ref().expect("connected above");
        let reply = async {
            if !first {
                return rpc
                    .call(method, params)
                    .await
                    .map_err(|error| error.to_string());
            }
            let mut stream = rpc
                .subscribe_scoped(method, params)
                .await
                .map_err(|error| error.to_string())?;
            stream
                .recv()
                .await
                .ok_or_else(|| String::from("the stream ended before its first item"))
        };
        tokio::time::timeout(ENGINE_TIMEOUT, reply)
            .await
            .map_err(|_| {
                format!(
                    "{method} did not answer within {} s",
                    ENGINE_TIMEOUT.as_secs()
                )
            })?
            .map_err(|error| format!("{method}: {error}"))
    }
}

pub async fn run(file: &Path, target: Option<&str>, out: Option<PathBuf>) -> Result<bool, String> {
    let raw =
        std::fs::read_to_string(file).map_err(|error| format!("{}: {error}", file.display()))?;
    let value: Value =
        serde_json::from_str(&raw).map_err(|error| format!("{}: {error}", file.display()))?;
    let scenario = parse_scenario(&value)?;
    let live = ui_driver::list();
    let app = ui_driver::select(&live, target.or(scenario.target.as_deref()), None)?.clone();
    let out = match out {
        Some(out) => out,
        None => run_dir().join("scenarios").join(format!(
            "{}-{}",
            file_label(&scenario.name),
            chrono::Local::now().format("%Y%m%d-%H%M%S")
        )),
    };
    let shots = out.join("shots");
    std::fs::create_dir_all(&shots).map_err(|error| format!("{}: {error}", shots.display()))?;
    let out = out
        .canonicalize()
        .map_err(|error| format!("{}: {error}", out.display()))?;
    println!("report: {}", out.display());
    let started_at = chrono::Utc::now();
    let mut saved = Map::new();
    saved.insert(
        "target".into(),
        serde_json::to_value(&app).map_err(|error| error.to_string())?,
    );
    saved.insert(
        "run".into(),
        json!({ "out": out, "shots": out.join("shots") }),
    );
    let target = json!({ "pid": app.pid, "label": app.label, "ipcPort": app.ipc_port });
    let mut io = Live { app, rpc: None };
    let reports = play(&scenario.steps, &mut io, &out, &mut saved, |report| {
        println!(
            "{:>3} {:<7} {:>6}ms {}{}",
            report.index,
            status_word(report.status),
            report.ms,
            report.name,
            report
                .error
                .as_deref()
                .map(|error| format!(": {}", one_line(error)))
                .unwrap_or_default()
        )
    })
    .await;
    let summary = summary(&reports);
    let report = json!({
        "name": scenario.name,
        "file": file,
        "target": target,
        "startedAt": started_at,
        "summary": summary,
        "passed": reports.iter().filter(|r| r.status == Status::Pass).count(),
        "total": reports.len(),
        "steps": reports,
    });
    let write = |name: &str, body: String| {
        let path = out.join(name);
        std::fs::write(&path, body).map_err(|error| format!("{}: {error}", path.display()))
    };
    write(
        "report.json",
        serde_json::to_string_pretty(&report).unwrap_or_default(),
    )?;
    write("report.md", markdown(&scenario.name, &target, &reports))?;
    println!("{summary}");
    Ok(reports.iter().all(|r| r.status == Status::Pass))
}

fn parse_scenario(value: &Value) -> Result<Scenario, String> {
    let name = value["name"]
        .as_str()
        .filter(|name| !name.trim().is_empty())
        .ok_or("a scenario needs a non-empty \"name\"")?
        .to_owned();
    let target = match &value["target"] {
        Value::Null => None,
        Value::String(target) => Some(target.clone()),
        Value::Number(pid) => Some(pid.to_string()),
        other => return Err(format!("\"target\" must be a pid or a label, not {other}")),
    };
    let steps = value["steps"]
        .as_array()
        .ok_or("a scenario needs a \"steps\" array")?
        .iter()
        .enumerate()
        .map(|(i, step)| parse_step(step).map_err(|error| format!("step {}: {error}", i + 1)))
        .collect::<Result<_, _>>()?;
    Ok(Scenario {
        name,
        target,
        steps,
    })
}

fn parse_step(value: &Value) -> Result<Step, String> {
    let mut fields = value
        .as_object()
        .ok_or_else(|| format!("a step is a JSON object, not {value}"))?
        .clone();
    let name = match fields.remove("name") {
        None => None,
        Some(Value::String(name)) => Some(name),
        Some(other) => return Err(format!("\"name\" must be a string, not {other}")),
    };
    let continue_on_fail = match fields.remove("continueOnFail") {
        None => false,
        Some(Value::Bool(flag)) => flag,
        Some(other) => return Err(format!("\"continueOnFail\" must be a bool, not {other}")),
    };
    let kinds: Vec<&str> = ["engine", "ui", "assert", "snapshot", "sleep"]
        .into_iter()
        .filter(|kind| fields.contains_key(*kind))
        .collect();
    let [kind] = kinds.as_slice() else {
        return Err(format!(
            "a step needs exactly one of engine, ui, assert, snapshot, sleep; got {}",
            if kinds.is_empty() {
                "none".to_owned()
            } else {
                kinds.join(", ")
            }
        ));
    };
    let save = match fields.remove("save") {
        None => None,
        Some(Value::String(save)) if *kind == "engine" || *kind == "ui" => Some(save),
        Some(_) if *kind != "engine" && *kind != "ui" => {
            return Err(format!(
                "\"save\" applies to engine and ui steps, not {kind}"
            ));
        }
        Some(other) => return Err(format!("\"save\" must be a name, not {other}")),
    };
    let head = fields.remove(*kind).expect("present above");
    let kind = match *kind {
        "engine" => {
            let method = head
                .as_str()
                .ok_or("\"engine\" names an RPC method")?
                .to_owned();
            let params = fields.remove("params").unwrap_or_else(|| json!({}));
            let first = match fields.remove("first") {
                None => false,
                Some(Value::Bool(first)) => first,
                Some(other) => return Err(format!("\"first\" must be a bool, not {other}")),
            };
            if let Some(extra) = fields.keys().next() {
                return Err(format!("unknown engine step field \"{extra}\""));
            }
            Kind::Engine {
                method,
                params,
                first,
            }
        }
        "ui" => Kind::Ui {
            action: head
                .as_str()
                .ok_or("\"ui\" names a driver action")?
                .to_owned(),
            args: fields,
        },
        other => {
            if let Some(extra) = fields.keys().next() {
                return Err(format!("unknown {other} step field \"{extra}\""));
            }
            match other {
                "assert" => Kind::Assert(parse_assert(&head)?),
                "snapshot" => Kind::Snapshot(
                    head.as_str()
                        .filter(|label| !label.trim().is_empty())
                        .ok_or("\"snapshot\" takes a label")?
                        .to_owned(),
                ),
                _ => Kind::Sleep(head.as_u64().ok_or("\"sleep\" takes milliseconds")?),
            }
        }
    };
    Ok(Step {
        name,
        continue_on_fail,
        save,
        kind,
    })
}

fn parse_assert(value: &Value) -> Result<Assert, String> {
    let exists = match &value["exists"] {
        Value::Null => true,
        Value::Bool(exists) => *exists,
        other => return Err(format!("\"exists\" must be a bool, not {other}")),
    };
    let timeout_ms = match &value["timeoutMs"] {
        Value::Null => ASSERT_TIMEOUT_MS,
        other => other.as_u64().ok_or("\"timeoutMs\" takes milliseconds")?,
    };
    if let Some(select) = value["select"].as_str() {
        let nth = match &value["nth"] {
            Value::Null => None,
            other => Some(other.as_u64().ok_or("\"nth\" is a 0-based index")?),
        };
        return Ok(Assert::Select {
            select: select.to_owned(),
            nth,
            exists,
            timeout_ms,
        });
    }
    if let Some(text) = value["text"].as_str() {
        return Ok(Assert::Text {
            text: text.to_owned(),
            exists,
            timeout_ms,
        });
    }
    if let Some(path) = value["saved"].as_str() {
        let expect = match (value.get("equals"), &value["contains"]) {
            (Some(equals), Value::Null) => Expect::Equals(equals.clone()),
            (None, Value::String(part)) => Expect::Contains(part.clone()),
            _ => {
                return Err(
                    "a saved assert needs \"equals\" (JSON) or \"contains\" (a string)".into(),
                );
            }
        };
        return Ok(Assert::Saved {
            path: path.to_owned(),
            expect,
        });
    }
    Err(format!(
        "an assert takes \"select\", \"text\" or \"saved\", not {value}"
    ))
}

async fn play(
    steps: &[Step],
    io: &mut impl Io,
    out: &Path,
    saved: &mut Map<String, Value>,
    mut progress: impl FnMut(&StepReport),
) -> Vec<StepReport> {
    let mut stopped = false;
    let mut reports = Vec::with_capacity(steps.len());
    for (i, step) in steps.iter().enumerate() {
        let index = i + 1;
        let mut report = StepReport {
            index,
            kind: kind_word(&step.kind),
            name: step.name.clone().unwrap_or_else(|| describe(&step.kind)),
            status: Status::Skipped,
            ms: 0,
            result: None,
            error: None,
            warning: None,
            shot: None,
        };
        if !stopped {
            let started = Instant::now();
            let outcome = perform(index, &step.kind, io, out, saved).await;
            report.ms = started.elapsed().as_millis() as u64;
            match outcome {
                Ok((result, shot)) => {
                    report.status = Status::Pass;
                    if let (Some(save), Some(result)) = (&step.save, &result) {
                        saved.insert(save.clone(), result.clone());
                    }
                    if let (Kind::Ui { .. }, Some(result)) = (&step.kind, &result) {
                        report.warning = reply_warning(result);
                    }
                    report.result = result.map(|value| short(&value));
                    report.shot = shot;
                }
                Err(error) => {
                    report.status = Status::Fail;
                    report.error = Some(error);
                    stopped = !step.continue_on_fail;
                }
            }
        }
        progress(&report);
        reports.push(report);
    }
    reports
}

async fn perform(
    index: usize,
    kind: &Kind,
    io: &mut impl Io,
    out: &Path,
    saved: &mut Map<String, Value>,
) -> Result<(Option<Value>, Option<String>), String> {
    match kind {
        Kind::Engine {
            method,
            params,
            first,
        } => {
            let params = substitute(params, saved)?;
            let result = io.engine(method, params, *first).await?;
            Ok((Some(result), None))
        }
        Kind::Ui { action, args } => {
            let Value::Object(mut request) = substitute(&Value::Object(args.clone()), saved)?
            else {
                unreachable!("an object substitutes to an object")
            };
            request.insert("action".into(), json!(action));
            let result = io.ui(request).await?;
            Ok((Some(result), None))
        }
        Kind::Assert(assert) => check(assert, io, saved).await.map(|found| (found, None)),
        Kind::Snapshot(label) => {
            let shot = format!("shots/{index:02}-{}.png", file_label(label));
            let path = out.join(&shot);
            let mut request = Map::new();
            request.insert("action".into(), json!("snapshot"));
            request.insert("path".into(), json!(path));
            let result = io.ui(request).await?;
            Ok((Some(result), Some(shot)))
        }
        Kind::Sleep(ms) => {
            tokio::time::sleep(Duration::from_millis(*ms)).await;
            Ok((None, None))
        }
    }
}

// Waits, not one tree read, for the positive asserts: a drive reply lands after
// one frame, so a tree read right after a click can race the render.
async fn check(
    assert: &Assert,
    io: &mut impl Io,
    saved: &Map<String, Value>,
) -> Result<Option<Value>, String> {
    let wait = |until: &str, fields: Value, timeout_ms: u64| {
        let mut request = Map::new();
        request.insert("action".into(), json!("wait"));
        request.insert("for".into(), json!(until));
        request.insert("timeoutMs".into(), json!(timeout_ms));
        if let Value::Object(fields) = fields {
            request.extend(fields);
        }
        request
    };
    match assert {
        Assert::Select {
            select,
            nth,
            exists,
            timeout_ms,
        } => {
            let select = substitute_str(select, saved)?;
            let mut fields = json!({ "select": select });
            if let Some(nth) = nth {
                fields["nth"] = json!(nth);
            }
            let until = if *exists { "select" } else { "gone" };
            io.ui(wait(until, fields, *timeout_ms)).await.map(Some)
        }
        Assert::Text {
            text,
            exists,
            timeout_ms,
        } => {
            let text = substitute_str(text, saved)?;
            let until = if *exists { "text" } else { "gone" };
            io.ui(wait(until, json!({ "text": text }), *timeout_ms))
                .await
                .map(Some)
        }
        Assert::Saved { path, expect } => {
            let expect = match expect {
                Expect::Equals(equals) => Expect::Equals(substitute(equals, saved)?),
                Expect::Contains(part) => Expect::Contains(substitute_str(part, saved)?),
            };
            saved_matches(saved, path, &expect).map(Some)
        }
    }
}

fn reply_warning(reply: &Value) -> Option<String> {
    let mut warnings = Vec::new();
    if reply["unsettled"] == true {
        warnings.push(String::from(
            "unsettled: the element never held still across two frames",
        ));
    }
    if let Some(handled) = reply["handled"].as_array() {
        let missed: Vec<String> = handled
            .iter()
            .enumerate()
            .filter(|(_, handled)| **handled == false)
            .map(|(i, _)| (i + 1).to_string())
            .collect();
        if !missed.is_empty() {
            warnings.push(format!(
                "keystroke {} of {} not handled",
                missed.join(", "),
                handled.len()
            ));
        }
    }
    (!warnings.is_empty()).then(|| warnings.join("; "))
}

fn saved_matches(saved: &Map<String, Value>, path: &str, expect: &Expect) -> Result<Value, String> {
    let actual = lookup(saved, path)?;
    match expect {
        Expect::Equals(equals) if &actual == equals => Ok(actual),
        Expect::Equals(equals) => Err(format!("`{path}` is {actual}, expected {equals}")),
        Expect::Contains(part) => {
            let text = match &actual {
                Value::String(text) => text.clone(),
                other => other.to_string(),
            };
            if text.contains(part.as_str()) {
                Ok(json!(part))
            } else {
                Err(format!(
                    "`{path}` does not contain {part:?}: {}",
                    short(&actual)
                ))
            }
        }
    }
}

fn lookup(saved: &Map<String, Value>, path: &str) -> Result<Value, String> {
    let unresolved = |why: String| format!("unresolved `${{{path}}}`: {why}");
    let (head, rest) = path.split_once('.').unwrap_or((path, ""));
    if head == "env" {
        return std::env::var(rest)
            .map(Value::String)
            .map_err(|_| unresolved(format!("environment variable {rest} is not set")));
    }
    let mut value = saved.get(head).ok_or_else(|| {
        unresolved(format!(
            "nothing saved as `{head}`; saved: {}",
            saved.keys().cloned().collect::<Vec<_>>().join(", ")
        ))
    })?;
    for segment in rest.split('.').filter(|segment| !segment.is_empty()) {
        let next = match value {
            Value::Array(items) => segment.parse::<usize>().ok().and_then(|i| items.get(i)),
            Value::Object(fields) => fields.get(segment),
            _ => None,
        };
        value = next.ok_or_else(|| unresolved(format!("no `{segment}` in {value}")))?;
    }
    Ok(value.clone())
}

fn substitute(value: &Value, saved: &Map<String, Value>) -> Result<Value, String> {
    Ok(match value {
        Value::String(text) => match whole_reference(text) {
            Some(path) => lookup(saved, path)?,
            None => Value::String(substitute_str(text, saved)?),
        },
        Value::Array(items) => Value::Array(
            items
                .iter()
                .map(|item| substitute(item, saved))
                .collect::<Result<_, _>>()?,
        ),
        Value::Object(fields) => Value::Object(
            fields
                .iter()
                .map(|(key, item)| Ok((key.clone(), substitute(item, saved)?)))
                .collect::<Result<_, String>>()?,
        ),
        other => other.clone(),
    })
}

fn whole_reference(text: &str) -> Option<&str> {
    let path = text.strip_prefix("${")?.strip_suffix('}')?;
    (!path.contains('}')).then_some(path)
}

fn substitute_str(text: &str, saved: &Map<String, Value>) -> Result<String, String> {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find("${") {
        out.push_str(&rest[..start]);
        let after = &rest[start + 2..];
        let end = after
            .find('}')
            .ok_or_else(|| format!("unclosed `${{` in {text:?}"))?;
        match lookup(saved, &after[..end])? {
            Value::String(value) => out.push_str(&value),
            value => out.push_str(&value.to_string()),
        }
        rest = &after[end + 1..];
    }
    out.push_str(rest);
    Ok(out)
}

fn kind_word(kind: &Kind) -> &'static str {
    match kind {
        Kind::Engine { .. } => "engine",
        Kind::Ui { .. } => "ui",
        Kind::Assert(_) => "assert",
        Kind::Snapshot(_) => "snapshot",
        Kind::Sleep(_) => "sleep",
    }
}

fn describe(kind: &Kind) -> String {
    match kind {
        Kind::Engine { method, params, .. } => match params["op"].as_str() {
            Some(op) => format!("engine {method} {op}"),
            None => format!("engine {method}"),
        },
        Kind::Ui { action, args, .. } => {
            let detail = ["select", "ref", "keys", "text", "for", "name"]
                .into_iter()
                .find_map(|key| args.get(key)?.as_str().map(|value| format!(" {value}")));
            format!("ui {action}{}", detail.unwrap_or_default())
        }
        Kind::Assert(Assert::Select { select, exists, .. }) => {
            format!("assert {}select {select}", if *exists { "" } else { "no " })
        }
        Kind::Assert(Assert::Text { text, exists, .. }) => {
            format!("assert {}text {text:?}", if *exists { "" } else { "no " })
        }
        Kind::Assert(Assert::Saved {
            path,
            expect: Expect::Equals(equals),
        }) => format!("assert {path} == {equals}"),
        Kind::Assert(Assert::Saved {
            path,
            expect: Expect::Contains(part),
        }) => format!("assert {path} contains {part:?}"),
        Kind::Snapshot(label) => format!("snapshot {label}"),
        Kind::Sleep(ms) => format!("sleep {ms}"),
    }
}

fn status_word(status: Status) -> &'static str {
    match status {
        Status::Pass => "pass",
        Status::Fail => "FAIL",
        Status::Skipped => "skipped",
    }
}

fn file_label(label: &str) -> String {
    label
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '-'
            }
        })
        .collect()
}

fn one_line(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn short(value: &Value) -> String {
    let line = one_line(&ui_driver::text(value));
    match line.char_indices().nth(RESULT_CHARS) {
        Some((cut, _)) => format!("{}…", &line[..cut]),
        None => line,
    }
}

fn summary(reports: &[StepReport]) -> String {
    let passed = reports.iter().filter(|r| r.status == Status::Pass).count();
    let total = reports.len();
    match reports.iter().find(|r| r.status == Status::Fail) {
        None => format!("PASS {passed}/{total}"),
        Some(failed) => format!(
            "FAIL {passed}/{total}: step {} {}: {}",
            failed.index,
            failed.name,
            one_line(failed.error.as_deref().unwrap_or_default())
        ),
    }
}

fn markdown(name: &str, target: &Value, reports: &[StepReport]) -> String {
    let cell = |text: &str| one_line(text).replace('|', "\\|");
    let mut md = format!(
        "# Scenario {name}\n\n**{}**\n\nTarget: {} (pid {}, engine port {})\n\n| # | Step | Kind | Status | ms | Result or error |\n|---|---|---|---|---|---|\n",
        summary(reports),
        target["label"].as_str().unwrap_or_default(),
        target["pid"],
        target["ipcPort"],
    );
    for report in reports {
        let detail = match (&report.error, &report.shot, &report.result) {
            (Some(error), _, _) => cell(error),
            (None, Some(shot), _) => format!("[{shot}]({shot})"),
            (None, None, Some(result)) => cell(result),
            (None, None, None) => String::new(),
        };
        let detail = match &report.warning {
            Some(warning) => format!("warning: {}; {detail}", cell(warning)),
            None => detail,
        };
        md.push_str(&format!(
            "| {} | {} | {} | {} | {} | {detail} |\n",
            report.index,
            cell(&report.name),
            report.kind,
            status_word(report.status),
            report.ms,
        ));
    }
    let shots: Vec<&String> = reports.iter().filter_map(|r| r.shot.as_ref()).collect();
    if !shots.is_empty() {
        md.push_str("\n## Snapshots\n");
        for shot in shots {
            md.push_str(&format!("\n### {shot}\n\n![{shot}]({shot})\n"));
        }
    }
    md
}

#[cfg(test)]
mod tests {
    use super::*;

    fn saved() -> Map<String, Value> {
        let Value::Object(saved) = json!({
            "dev": { "deviceId": "d-1", "ports": [27, 49880] },
            "chat": { "id": "c-9", "count": 3 },
        }) else {
            unreachable!()
        };
        saved
    }

    #[test]
    fn a_whole_reference_keeps_its_type_and_an_embedded_one_becomes_text() {
        let saved = saved();
        let params = json!({
            "deviceId": "${dev.deviceId}",
            "port": "${dev.ports.1}",
            "chat": "${chat}",
            "path": "/tmp/${chat.id}/n${chat.count}",
            "list": ["${dev.ports.0}", 7, null],
        });
        assert_eq!(
            substitute(&params, &saved).unwrap(),
            json!({
                "deviceId": "d-1",
                "port": 49880,
                "chat": { "id": "c-9", "count": 3 },
                "path": "/tmp/c-9/n3",
                "list": [27, 7, null],
            })
        );
    }

    #[test]
    fn an_env_reference_reads_the_environment_and_a_missing_one_fails() {
        let saved = saved();
        let home = std::env::var("HOME").unwrap();
        assert_eq!(
            substitute_str("${env.HOME}/x", &saved).unwrap(),
            format!("{home}/x")
        );
        let error = substitute_str("${env.ZERON_SCENARIO_SURELY_UNSET_9}", &saved).unwrap_err();
        assert!(
            error.contains("ZERON_SCENARIO_SURELY_UNSET_9 is not set"),
            "{error}"
        );
    }

    #[test]
    fn unresolved_references_name_what_is_missing() {
        let saved = saved();
        let error = substitute(&json!({ "a": ["${nope.x}"] }), &saved).unwrap_err();
        assert!(error.starts_with("unresolved `${nope.x}`"), "{error}");
        assert!(error.contains("saved: chat, dev"), "{error}");
        let error = substitute_str("id ${chat.missing}", &saved).unwrap_err();
        assert!(error.contains("no `missing`"), "{error}");
        let error = substitute_str("${dev.ports.5}", &saved).unwrap_err();
        assert!(error.contains("no `5`"), "{error}");
        assert!(
            substitute_str("${chat.id", &saved)
                .unwrap_err()
                .contains("unclosed")
        );
    }

    #[test]
    fn steps_parse_by_their_one_kind_key() {
        let step = parse_step(&json!({ "ui": "click", "select": "Button \"Go\"", "nth": 1, "save": "c", "name": "go" })).unwrap();
        let Kind::Ui { action, args } = &step.kind else {
            panic!("{step:?}")
        };
        assert_eq!(action, "click");
        assert_eq!(
            Value::Object(args.clone()),
            json!({ "select": "Button \"Go\"", "nth": 1 })
        );
        assert_eq!(step.save.as_deref(), Some("c"));
        assert_eq!(step.name.as_deref(), Some("go"));
        assert_eq!(
            parse_step(&json!({ "assert": { "text": "hi", "exists": false } }))
                .unwrap()
                .kind,
            Kind::Assert(Assert::Text {
                text: "hi".into(),
                exists: false,
                timeout_ms: ASSERT_TIMEOUT_MS
            })
        );
        let engine =
            parse_step(&json!({ "engine": "LocalDevice", "continueOnFail": true })).unwrap();
        assert!(engine.continue_on_fail);
        assert_eq!(
            engine.kind,
            Kind::Engine {
                method: "LocalDevice".into(),
                params: json!({}),
                first: false,
            }
        );
        assert_eq!(engine.save, None);
        for (bad, why) in [
            (json!({ "ui": "click", "sleep": 3 }), "exactly one"),
            (json!({ "name": "x" }), "got none"),
            (
                json!({ "engine": "M", "parms": {} }),
                "unknown engine step field \"parms\"",
            ),
            (json!({ "snapshot": "a", "save": "s" }), "\"save\" applies"),
            (json!({ "assert": { "saved": "a.b" } }), "needs \"equals\""),
            (
                json!({ "assert": { "saved": "a.b", "equals": 1, "contains": "x" } }),
                "needs \"equals\"",
            ),
            (
                json!({ "engine": "M", "first": "yes" }),
                "\"first\" must be a bool",
            ),
            (json!({ "sleep": "1s" }), "milliseconds"),
        ] {
            let error = parse_step(&bad).unwrap_err();
            assert!(error.contains(why), "{bad}: {error}");
        }
    }

    #[test]
    fn a_saved_assert_compares_or_searches_the_value_at_the_path() {
        let saved = saved();
        let equals = |path, value| saved_matches(&saved, path, &Expect::Equals(value));
        let contains =
            |path, part: &str| saved_matches(&saved, path, &Expect::Contains(part.into()));
        assert_eq!(equals("chat.count", json!(3)), Ok(json!(3)));
        let error = equals("chat.id", json!("c-8")).unwrap_err();
        assert_eq!(error, "`chat.id` is \"c-9\", expected \"c-8\"");
        assert!(equals("chat.count", json!("3")).is_err());
        assert!(contains("chat.id", "-9").is_ok());
        assert!(contains("dev", "\"ports\":[27,49880]").is_ok());
        let error = contains("dev", "Picked").unwrap_err();
        assert!(
            error.starts_with("`dev` does not contain \"Picked\""),
            "{error}"
        );
    }

    struct Fake {
        requests: Vec<Value>,
        fail_on: &'static str,
    }

    impl Io for Fake {
        async fn ui(&mut self, request: Map<String, Value>) -> Result<Value, String> {
            let request = Value::Object(request);
            self.requests.push(request.clone());
            if request["action"] == self.fail_on {
                return Err(format!("{} broke", self.fail_on));
            }
            if let Some(reply) = request.get("reply") {
                return Ok(reply.clone());
            }
            Ok(json!({ "echo": request }))
        }

        async fn engine(
            &mut self,
            method: &str,
            params: Value,
            first: bool,
        ) -> Result<Value, String> {
            self.requests
                .push(json!({ "engine": method, "params": params, "first": first }));
            Ok(json!({ "deviceId": "dev-7" }))
        }
    }

    fn steps(value: Value) -> Vec<Step> {
        parse_scenario(&json!({ "name": "t", "steps": value }))
            .unwrap()
            .steps
    }

    #[tokio::test]
    async fn a_failure_skips_the_rest_unless_the_step_continues_on_fail() {
        let steps = steps(json!([
            { "engine": "WatchDocMessages", "params": { "chatId": "c" }, "first": true, "save": "dev" },
            { "ui": "key", "keys": "${dev.deviceId}", "continueOnFail": true, "name": "keys" },
            { "assert": { "saved": "dev.deviceId", "equals": "dev-7" } },
            { "snapshot": "after key" },
            { "ui": "click", "x": 1, "y": 2 },
            { "assert": { "select": "Button \"Go\"", "exists": false } },
        ]));
        let mut fake = Fake {
            requests: Vec::new(),
            fail_on: "key",
        };
        let mut saved = Map::new();
        let reports = play(&steps, &mut fake, Path::new("/out"), &mut saved, |_| {}).await;
        let statuses: Vec<Status> = reports.iter().map(|r| r.status).collect();
        assert_eq!(
            statuses,
            [
                Status::Pass,
                Status::Fail,
                Status::Pass,
                Status::Pass,
                Status::Pass,
                Status::Pass
            ]
        );
        assert_eq!(
            fake.requests[0],
            json!({ "engine": "WatchDocMessages", "params": { "chatId": "c" }, "first": true })
        );
        assert_eq!(
            fake.requests[1],
            json!({ "action": "key", "keys": "dev-7" })
        );
        assert_eq!(
            fake.requests[2],
            json!({ "action": "snapshot", "path": "/out/shots/04-after-key.png" })
        );
        assert_eq!(reports[3].shot.as_deref(), Some("shots/04-after-key.png"));
        assert_eq!(
            fake.requests[4],
            json!({ "action": "wait", "for": "gone", "select": "Button \"Go\"", "timeoutMs": ASSERT_TIMEOUT_MS })
        );
        assert_eq!(summary(&reports), "FAIL 5/6: step 2 keys: key broke");

        let mut fake = Fake {
            requests: Vec::new(),
            fail_on: "snapshot",
        };
        let reports = play(
            &steps,
            &mut fake,
            Path::new("/out"),
            &mut Map::new(),
            |_| {},
        )
        .await;
        let statuses: Vec<Status> = reports.iter().map(|r| r.status).collect();
        assert_eq!(
            statuses,
            [
                Status::Pass,
                Status::Pass,
                Status::Pass,
                Status::Fail,
                Status::Skipped,
                Status::Skipped
            ]
        );
        assert_eq!(fake.requests.len(), 3);
        assert_eq!(
            summary(&reports),
            "FAIL 3/6: step 4 snapshot after key: snapshot broke"
        );
    }

    #[tokio::test]
    async fn an_unsettled_or_unhandled_ui_reply_passes_with_a_warning() {
        let steps = steps(json!([
            { "ui": "click", "x": 1, "y": 2, "reply": { "frame": 9, "unsettled": true } },
            { "ui": "key", "keys": "a b c", "reply": { "frame": 9, "handled": [true, false, false] } },
            { "ui": "key", "keys": "a", "reply": { "frame": 9, "handled": [true] } },
            { "ui": "state", "reply": { "unsettled": false } },
        ]));
        let mut fake = Fake {
            requests: Vec::new(),
            fail_on: "",
        };
        let reports = play(
            &steps,
            &mut fake,
            Path::new("/out"),
            &mut Map::new(),
            |_| {},
        )
        .await;
        assert!(reports.iter().all(|r| r.status == Status::Pass));
        let warnings: Vec<Option<&str>> = reports.iter().map(|r| r.warning.as_deref()).collect();
        assert_eq!(
            warnings,
            [
                Some("unsettled: the element never held still across two frames"),
                Some("keystroke 2, 3 of 3 not handled"),
                None,
                None
            ]
        );
        assert_eq!(summary(&reports), "PASS 4/4");
        assert_eq!(
            serde_json::to_value(&reports[1]).unwrap()["warning"],
            "keystroke 2, 3 of 3 not handled"
        );
        assert!(
            serde_json::to_value(&reports[2])
                .unwrap()
                .get("warning")
                .is_none()
        );
        let md = markdown("w", &json!({}), &reports);
        assert!(
            md.contains(" | warning: keystroke 2, 3 of 3 not handled; {"),
            "{md}"
        );
    }

    #[test]
    fn the_committed_example_scenarios_parse() {
        for (file, raw) in [
            (
                "mock-question.json",
                include_str!("../../../scripts/scenarios/mock-question.json"),
            ),
            (
                "journal-replay.json",
                include_str!("../../../scripts/scenarios/journal-replay.json"),
            ),
        ] {
            let value: Value = serde_json::from_str(raw).unwrap_or_else(|e| panic!("{file}: {e}"));
            let scenario = parse_scenario(&value).unwrap_or_else(|e| panic!("{file}: {e}"));
            assert!(!scenario.steps.is_empty(), "{file}");
        }
    }

    #[test]
    fn the_report_tables_every_step_and_links_each_snapshot() {
        let report = |index, status, shot: Option<&str>, error: Option<&str>| StepReport {
            index,
            kind: "ui",
            name: format!("step | {index}"),
            status,
            ms: 5,
            result: Some("ok".into()),
            error: error.map(str::to_owned),
            warning: None,
            shot: shot.map(str::to_owned),
        };
        let reports = [
            report(1, Status::Pass, Some("shots/01-before.png"), None),
            report(2, Status::Fail, None, Some("timed out\nlast seen: x")),
            report(3, Status::Skipped, None, None),
        ];
        let md = markdown(
            "demo",
            &json!({ "label": "Zeron Scenario", "pid": 42, "ipcPort": 49880 }),
            &reports,
        );
        assert!(
            md.starts_with(
                "# Scenario demo\n\n**FAIL 1/3: step 2 step | 2: timed out last seen: x**"
            ),
            "{md}"
        );
        assert!(
            md.contains("Target: Zeron Scenario (pid 42, engine port 49880)"),
            "{md}"
        );
        assert!(
            md.contains(
                "| 1 | step \\| 1 | ui | pass | 5 | [shots/01-before.png](shots/01-before.png) |"
            ),
            "{md}"
        );
        assert!(
            md.contains("| 2 | step \\| 2 | ui | FAIL | 5 | timed out last seen: x |"),
            "{md}"
        );
        assert!(
            md.contains("| 3 | step \\| 3 | ui | skipped | 5 | ok |"),
            "{md}"
        );
        assert!(
            md.contains("![shots/01-before.png](shots/01-before.png)"),
            "{md}"
        );
        assert_eq!(summary(&reports[..1]), "PASS 1/1");
    }
}
