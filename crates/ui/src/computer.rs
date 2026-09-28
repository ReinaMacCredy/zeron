//! Computer use: the desktop host that owns `cua-driver`.

pub mod cards;
pub mod cursor_preference;
pub mod desktop_lifecycle;
#[cfg(target_os = "macos")]
pub mod driver;
pub mod emergency_stop_notice;
#[cfg(target_os = "macos")]
pub mod host;
#[cfg(target_os = "macos")]
pub mod host_socket;
#[cfg(target_os = "macos")]
pub mod native;
pub mod presence;
pub mod presentation;
pub mod preview;
pub mod preview_card;
pub mod protected_surfaces;
pub mod registration;
#[cfg(target_os = "macos")]
pub mod runtime_ownership;
#[cfg(target_os = "macos")]
pub mod stop_native_helper;
pub mod strip;

/// Proof knob (`ZERON_CUA_PROBE=<file>`): spawn the bundled driver through the
/// host, run one grant-requiring read, retire it, and write the replies to
/// `<file>`.
#[cfg(target_os = "macos")]
pub fn run_probe_from_env(_data_dir: &std::path::Path) {
    let Some(out) = std::env::var_os("ZERON_CUA_PROBE") else {
        return;
    };
    std::thread::spawn(move || {
        let report = match tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        {
            Ok(runtime) => runtime.block_on(probe()),
            Err(err) => serde_json::json!({ "error": format!("runtime: {err}") }),
        };
        let _ = std::fs::write(&out, serde_json::to_vec_pretty(&report).unwrap_or_default());
    });
}

#[cfg(target_os = "macos")]
async fn probe() -> serde_json::Value {
    use serde_json::json;
    use zeron_proto::computer::{ComputerHostCall, ComputerHostMethod};
    use zeron_rpc::ComputerHost;

    let Some(binary) = driver::bundled_binary() else {
        return json!({ "error": "no bundled cua-driver" });
    };
    let bundle_id = std::env::var("ZERON_BUNDLE_ID").unwrap_or_else(|_| "sh.zeron.app".into());
    let options = host::CuaDriverHostOptions::new(
        binary.clone(),
        bundle_id,
        Box::new(|| Box::pin(async { Ok(()) })),
    );
    let host = match host::CuaDriverHost::new(options) {
        Ok(host) => host,
        Err(err) => return json!({ "error": format!("host: {err}") }),
    };
    let own_pid = std::process::id();
    let call = |id: &str, name: &str, args: serde_json::Value| ComputerHostCall {
        correlation_id: id.into(),
        method: ComputerHostMethod::Call,
        name: Some(name.into()),
        args,
        task: None,
        delivery_mode: None,
        model_observation: None,
    };
    let permissions = host
        .request(call("probe-1", "check_permissions", json!({})))
        .await;
    let windows = host
        .request(call("probe-2", "list_windows", json!({ "pid": own_pid })))
        .await;
    let window_id = serde_json::to_value(&windows)
        .ok()
        .as_ref()
        .and_then(first_window_id);
    let state = match window_id {
        Some(id) => Some(
            host.request(call(
                "probe-3",
                "get_window_state",
                json!({ "pid": own_pid, "window_id": id, "max_dimension": 256 }),
            ))
            .await,
        ),
        None => None,
    };
    let retired = host.dispose().await;
    json!({
        "binary": binary,
        "host_pid": own_pid,
        "expected_revision": zeron_proto::computer::CUA_NATIVE_REVISION,
        "driver_native_revision": permissions.driver_native_revision,
        "check_permissions": permissions,
        "list_windows": windows,
        "get_window_state": state,
        "retired": retired,
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
