use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use serde::Serialize;
use serde_json::{Map, Value, json};
use zeron_proto::ui_driver::{UiDriverBuild, UiDriverDescriptor, run_dir};

use zeron_mcp::ui_driver;

mod parse;
mod report;

use parse::{Assert, Expect, Kind, Step, parse_scenario};
use report::{file_label, markdown, one_line, short, status_word, summary};

const ENGINE_TIMEOUT: Duration = Duration::from_secs(30);

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
            let client = zeron_rpc::connect_ipc(self.app.ipc_port, Some(&self.app.data_dir))
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
    if app.build != UiDriverBuild::Debug
        && scenario
            .steps
            .iter()
            .any(|step| matches!(step.kind, Kind::Engine { .. }))
    {
        return Err(format!(
            "{} (pid {}) is a release build: a scenario with engine steps runs only against a Zeron Dev bundle, so its RPCs never reach your real workspace",
            app.label, app.pid
        ));
    }
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
        let mut names: Vec<&str> = saved.keys().map(String::as_str).collect();
        names.sort_unstable();
        unresolved(format!(
            "nothing saved as `{head}`; saved: {}",
            names.join(", ")
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

#[cfg(test)]
mod tests {
    use super::*;
    use parse::ASSERT_TIMEOUT_MS;

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
        let path = std::env::var("PATH").unwrap();
        assert_eq!(
            substitute_str("${env.PATH}/x", &saved).unwrap(),
            format!("{path}/x")
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
            { "assert": { "text": "Picked", "exists": false } },
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
            json!({ "action": "snapshot", "path": Path::new("/out").join("shots/04-after-key.png") })
        );
        assert_eq!(reports[3].shot.as_deref(), Some("shots/04-after-key.png"));
        assert_eq!(
            fake.requests[4],
            json!({ "action": "wait", "for": "gone", "select": "Button \"Go\"", "timeoutMs": ASSERT_TIMEOUT_MS })
        );
        assert_eq!(
            fake.requests[5],
            json!({ "action": "wait", "for": "gone", "text": "Picked", "timeoutMs": ASSERT_TIMEOUT_MS })
        );
        assert_eq!(summary(&reports), "FAIL 6/7: step 2 keys: key broke");

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
                Status::Skipped,
                Status::Skipped
            ]
        );
        assert_eq!(fake.requests.len(), 3);
        assert_eq!(
            summary(&reports),
            "FAIL 3/7: step 4 snapshot after key: snapshot broke"
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
}
