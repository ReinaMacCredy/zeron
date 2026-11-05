use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{Map, Value, json};
use zeron_proto::ui_driver::{
    UiDriverDescriptor, ZGX_DRIVE_COMMANDS, ZGX_INSPECT_COMMANDS, run_dir,
};

use super::{exchange, reply_timeout};

const NATIVE_FIELDS: &[&str] = &[
    "ref", "select", "nth", "x", "y", "root", "do", "during", "rect",
];

const WINDOW_ROOT: &str = "@1";

pub(super) async fn request(
    app: &UiDriverDescriptor,
    request: Map<String, Value>,
) -> Result<Value, String> {
    let timeout = reply_timeout(&request);
    let as_json = request.get("json") == Some(&json!(true));
    let out = request
        .get("path")
        .and_then(Value::as_str)
        .map(str::to_owned);
    let wire = wire(request)?;
    let snapshot = wire["cmd"] == "snapshot";
    let reply = tokio::time::timeout(timeout, exchange(&app.endpoint, Value::Object(wire)))
        .await
        .map_err(|_| {
            format!(
                "{} (pid {}) did not answer within {} s",
                app.label,
                app.pid,
                timeout.as_secs()
            )
        })??;
    if reply["ok"] != true {
        return Err(reply["error"].as_str().map_or_else(
            || format!("reply is not ok: {reply}"),
            |error| error.replace("`target`", "`node`"),
        ));
    }
    if snapshot {
        return kept_snapshot(app.pid, reply["data"].clone(), out);
    }
    let text = reply["text"].as_str().unwrap_or_default();
    Ok(if as_json || text.is_empty() {
        reply["data"].clone()
    } else {
        Value::String(text.to_owned())
    })
}

fn wire(mut request: Map<String, Value>) -> Result<Map<String, Value>, String> {
    let action = request
        .remove("action")
        .and_then(|action| action.as_str().map(str::to_owned))
        .ok_or("`action` is required")?;
    if !ZGX_INSPECT_COMMANDS.contains(&action.as_str())
        && !ZGX_DRIVE_COMMANDS.contains(&action.as_str())
    {
        return Err(format!(
            "a zgx app has no `{action}`; it takes {}, {}",
            ZGX_INSPECT_COMMANDS.join(", "),
            ZGX_DRIVE_COMMANDS.join(", ")
        ));
    }
    let present = |field: &str| request.get(field).is_some_and(|value| !value.is_null());
    let native = NATIVE_FIELDS
        .iter()
        .copied()
        .chain((action != "search").then_some("query"))
        .find(|field| present(field));
    if let Some(field) = native {
        return Err(format!(
            "`{field}` is the gpui app's: on a zgx app `node` names a node (#id, \"Name\", role:button, @12, a >> b) and `find` searches"
        ));
    }
    request.remove("json");
    request.remove("path");
    if let Some(Value::Object(extra)) = request.remove("args") {
        request.extend(extra);
    }
    if let Some(node) = request.remove("node") {
        request.insert("target".into(), node);
    }
    if action == "snapshot" {
        request
            .entry("target")
            .or_insert_with(|| json!(WINDOW_ROOT));
    }
    if let Some(ms) = request.remove("timeoutMs") {
        request.insert("timeout_ms".into(), ms);
    }
    if let Some(to) = request.remove("to") {
        request.insert("to".into(), json!(place(&to)?));
    }
    if let Some(keys) = request.remove("keys") {
        request.insert("keys".into(), single_key(keys)?);
    }
    request.insert("cmd".into(), json!(action));
    Ok(request)
}

fn place(to: &Value) -> Result<String, String> {
    let point = |x: &Value, y: &Value| Some(format!("{},{}", x.as_f64()?, y.as_f64()?));
    let found = match to {
        Value::String(node) => Some(node.clone()),
        Value::Array(pair) if pair.len() == 2 => point(&pair[0], &pair[1]),
        Value::Object(end) => end
            .get("node")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .or_else(|| point(end.get("x")?, end.get("y")?)),
        _ => None,
    };
    found.ok_or_else(|| format!("`to` is a node or a point, not {to}"))
}

fn single_key(keys: Value) -> Result<Value, String> {
    match keys {
        Value::Array(mut keys) if keys.len() == 1 => Ok(keys.remove(0)),
        Value::Array(_) => Err("a zgx app takes one key per `key` call".into()),
        keys => Ok(keys),
    }
}

fn kept_snapshot(pid: u32, mut data: Value, out: Option<String>) -> Result<Value, String> {
    let from = data["path"]
        .as_str()
        .ok_or_else(|| format!("snapshot reply names no path: {data}"))?;
    let to = match out {
        Some(out) => std::path::PathBuf::from(out),
        None => {
            let shots = run_dir().join("shots");
            std::fs::create_dir_all(&shots)
                .map_err(|error| format!("{}: {error}", shots.display()))?;
            let at = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_or(0, |now| now.as_millis());
            shots.join(format!("zgx-{pid}-{at}.png"))
        }
    };
    std::fs::copy(from, &to).map_err(|error| format!("{from} to {}: {error}", to.display()))?;
    data["path"] = json!(to);
    Ok(data)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wire_of(request: Value) -> Result<Value, String> {
        let Value::Object(request) = request else {
            unreachable!()
        };
        wire(request).map(Value::Object)
    }

    #[test]
    fn our_fields_become_the_wire_s_and_extra_args_pass_through() {
        assert_eq!(
            wire_of(json!({
                "action": "wait", "node": "\"Copy\"", "timeoutMs": 8000, "window": 2,
                "json": true, "args": { "gone": true }
            })),
            Ok(json!({
                "cmd": "wait", "target": "\"Copy\"", "timeout_ms": 8000, "window": 2, "gone": true
            }))
        );
        assert_eq!(
            wire_of(json!({ "action": "drag", "node": "#a", "to": { "x": 1, "y": 2.5 } })),
            Ok(json!({ "cmd": "drag", "target": "#a", "to": "1,2.5" }))
        );
        assert_eq!(
            wire_of(json!({ "action": "drag", "node": "#a", "to": { "node": "#b" } })),
            Ok(json!({ "cmd": "drag", "target": "#a", "to": "#b" }))
        );
        assert_eq!(
            wire_of(json!({ "action": "key", "keys": ["cmd+k"] })),
            Ok(json!({ "cmd": "key", "keys": "cmd+k" }))
        );
        assert_eq!(
            wire_of(json!({ "action": "snapshot" })),
            Ok(json!({ "cmd": "snapshot", "target": WINDOW_ROOT }))
        );
    }

    #[test]
    fn what_a_zgx_app_would_not_understand_is_refused_rather_than_sent() {
        for (request, says) in [
            (json!({ "action": "press" }), "has no `press`"),
            (json!({ "action": "exec" }), "has no `exec`"),
            (json!({ "action": "click", "select": "Button" }), "`select`"),
            (json!({ "action": "tree", "query": "x" }), "`query`"),
            (json!({ "action": "drag", "to": true }), "`to` is a node"),
            (
                json!({ "action": "key", "keys": ["a", "b"] }),
                "one key per",
            ),
        ] {
            let error = wire_of(request).unwrap_err();
            assert!(error.contains(says), "{error}");
        }
        assert_eq!(
            wire_of(json!({ "action": "search", "query": "x" })),
            Ok(json!({ "cmd": "search", "query": "x" }))
        );
    }
}
