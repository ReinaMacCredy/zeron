use serde_json::{Map, Value, json};
use zeron_proto::ui_driver::MAX_WAIT_MS;

pub(super) const ASSERT_TIMEOUT_MS: u64 = 3000;
const RESERVED_NAMES: [&str; 3] = ["env", "target", "run"];
const ASSERT_FIELDS: [&str; 8] = [
    "select",
    "nth",
    "text",
    "saved",
    "equals",
    "contains",
    "exists",
    "timeoutMs",
];

#[derive(Debug, PartialEq)]
pub(super) struct Scenario {
    pub(super) name: String,
    pub(super) target: Option<String>,
    pub(super) steps: Vec<Step>,
}

#[derive(Debug, PartialEq)]
pub(super) struct Step {
    pub(super) name: Option<String>,
    pub(super) continue_on_fail: bool,
    pub(super) save: Option<String>,
    pub(super) kind: Kind,
}

#[derive(Debug, PartialEq)]
pub(super) enum Kind {
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
pub(super) enum Assert {
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
pub(super) enum Expect {
    Equals(Value),
    Contains(String),
}

pub(super) fn parse_scenario(value: &Value) -> Result<Scenario, String> {
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
        Some(Value::String(save))
            if save.is_empty() || save.contains('.') || RESERVED_NAMES.contains(&save.as_str()) =>
        {
            return Err(format!(
                "\"save\" must be a name without dots other than {}, not {save:?}",
                RESERVED_NAMES.join(", ")
            ));
        }
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
                _ => Kind::Sleep(
                    head.as_u64()
                        .filter(|&ms| ms <= MAX_WAIT_MS)
                        .ok_or(format!("\"sleep\" takes up to {MAX_WAIT_MS} milliseconds"))?,
                ),
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
    if let Some(fields) = value.as_object() {
        if let Some(extra) = fields
            .keys()
            .find(|key| !ASSERT_FIELDS.contains(&key.as_str()))
        {
            return Err(format!("unknown assert field \"{extra}\""));
        }
        let kinds = ["select", "text", "saved"]
            .into_iter()
            .filter(|kind| fields.contains_key(*kind))
            .count();
        if kinds > 1 {
            return Err("an assert takes one of \"select\", \"text\" or \"saved\"".into());
        }
    }
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

#[cfg(test)]
mod tests {
    use super::*;

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
            (json!({ "sleep": 60_001 }), "up to 60000 milliseconds"),
            (json!({ "ui": "state", "save": "a.b" }), "without dots"),
            (json!({ "ui": "state", "save": "target" }), "without dots"),
            (
                json!({ "assert": { "select": "Button", "text": "Go" } }),
                "one of \"select\"",
            ),
            (
                json!({ "assert": { "text": "Go", "exist": false } }),
                "unknown assert field \"exist\"",
            ),
        ] {
            let error = parse_step(&bad).unwrap_err();
            assert!(error.contains(why), "{bad}: {error}");
        }
    }

    #[test]
    fn the_committed_example_scenarios_parse() {
        for (file, raw) in [
            (
                "mock-question.json",
                include_str!("../../../../scripts/scenarios/mock-question.json"),
            ),
            (
                "journal-replay.json",
                include_str!("../../../../scripts/scenarios/journal-replay.json"),
            ),
        ] {
            let value: Value = serde_json::from_str(raw).unwrap_or_else(|e| panic!("{file}: {e}"));
            let scenario = parse_scenario(&value).unwrap_or_else(|e| panic!("{file}: {e}"));
            assert!(!scenario.steps.is_empty(), "{file}");
        }
    }
}
