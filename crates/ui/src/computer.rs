//! Computer use: the desktop host that owns `cua-driver`.

#[cfg(target_os = "macos")]
pub mod driver;

/// Proof knob (`ZERON_CUA_PROBE=<file>`): spawn the bundled driver from this
/// process, handshake, run one grant-requiring read, and write the replies to
/// `<file>`.
#[cfg(target_os = "macos")]
pub fn run_probe_from_env(data_dir: &std::path::Path) {
    let Some(out) = std::env::var_os("ZERON_CUA_PROBE") else {
        return;
    };
    let runtime_dir = data_dir.join("computer");
    std::thread::spawn(move || {
        let report = probe(&runtime_dir);
        let _ = std::fs::write(&out, serde_json::to_vec_pretty(&report).unwrap_or_default());
    });
}

#[cfg(target_os = "macos")]
fn probe(runtime_dir: &std::path::Path) -> serde_json::Value {
    use serde_json::json;
    let Some(binary) = driver::bundled_binary() else {
        return json!({ "error": "no bundled cua-driver" });
    };
    let bundle_id = std::env::var("ZERON_BUNDLE_ID").unwrap_or_else(|_| "sh.zeron.app".into());
    let process = match driver::DriverProcess::spawn(&binary, runtime_dir, &bundle_id) {
        Ok(process) => process,
        Err(err) => return json!({ "error": format!("spawn: {err}") }),
    };
    let own_pid = std::process::id();
    let metadata = process.request(&json!({ "method": "metadata" }), driver::DEFAULT_TIMEOUT);
    let permissions = process.call("check_permissions", json!({}));
    let windows = process.call("list_windows", json!({ "pid": own_pid }));
    let window_id = windows.as_ref().ok().and_then(first_window_id);
    let state = window_id.as_ref().map(|id| {
        process.call(
            "get_window_state",
            json!({ "pid": own_pid, "window_id": id, "max_dimension": 256 }),
        )
    });
    let show = |r: &std::io::Result<serde_json::Value>| match r {
        Ok(v) => v.clone(),
        Err(e) => json!({ "io_error": e.to_string() }),
    };
    json!({
        "binary": binary,
        "host_pid": own_pid,
        "driver_pid": process.pid(),
        "expected_revision": driver::NATIVE_REVISION,
        "metadata": show(&metadata),
        "check_permissions": show(&permissions),
        "list_windows": show(&windows),
        "get_window_state": state.as_ref().map(show),
    })
}

#[cfg(target_os = "macos")]
fn first_window_id(value: &serde_json::Value) -> Option<serde_json::Value> {
    match value {
        serde_json::Value::Object(map) => map
            .get("window_id")
            .cloned()
            .or_else(|| map.values().find_map(first_window_id)),
        serde_json::Value::Array(items) => items.iter().find_map(first_window_id),
        _ => None,
    }
}
