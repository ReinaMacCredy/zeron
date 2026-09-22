use async_trait::async_trait;
use cua_driver_core::{
    protocol::ToolResult,
    tool::{Tool, ToolDef},
};
use serde_json::Value;
use std::path::PathBuf;

pub struct LaunchAppTool;

static DEF: std::sync::OnceLock<ToolDef> = std::sync::OnceLock::new();

fn def() -> &'static ToolDef {
    DEF.get_or_init(|| ToolDef {
        name: "launch_app".into(),
        description:
            "Launch or reuse a macOS app without requesting foreground activation.\n\n\
             Provide `bundle_id` (preferred) or `name`. A plain request reuses a running \
             application and its windows without delivering another reopen event.\n\n\
             `hidden` means the application's real hidden state, not background operation. \
             Leave it false or omitted for usable background windows. Explicit hidden:true \
             may leave no usable window; unhide through a later non-hidden launch.\n\n\
             Optional `urls` are handed to the app as open targets. `additional_arguments` \
             apply to a new process; an argument-bearing request to a running app may start \
             another instance. Use `creates_new_application_instance` only when a separate \
             instance is needed. Browser DevTools setup belongs to `browser_prepare`.\n\n\
             Optional `webkit_inspector_port` sets WEBKIT_INSPECTOR_SERVER and \
             TAURI_WEBVIEW_AUTOMATION for an explicitly requested WebKit inspector.\n\n\
             Returns the actual pid, app identity and windows. `launch_state.window_ready` \
             requires a visible window on the active Space, not merely a running process. \
             If no usable window appears, inspect the reason; never kill or relaunch the \
             app automatically. The driver never restores a previous app's focus or \
             schedules activation after this request returns."
            .into(),
        input_schema: serde_json::json!({
            "type": "object",
            "properties": {
                "bundle_id": {
                    "type": "string",
                    "description": "App bundle identifier, e.g. com.apple.calculator. Preferred over name."
                },
                "name": {
                    "type": "string",
                    "description": "App display name. Used only when bundle_id is absent."
                },
                "urls": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Optional file paths or URLs to open with the app (e.g. a folder path for Finder)."
                },
                "webkit_inspector_port": {
                    "type": "integer",
                    "description": "Open a WebKit inspector server on this port (sets WEBKIT_INSPECTOR_SERVER env var)."
                },
                "creates_new_application_instance": {
                    "type": "boolean",
                    "description": "When true, explicitly request a new app instance instead of reusing a running one. Single-instance apps may still hand off to their existing process."
                },
                "hidden": {
                    "type": "boolean",
                    "description": "Explicit application hiding, equivalent to Cmd+H. This can prevent usable windows. Background operation uses activates=false with hidden:false or omitted; it does not require hiding the app."
                },
                "additional_arguments": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Extra arguments appended after --args when launching."
                }
            },
            "additionalProperties": false
        }),
        read_only: false,
        destructive: false,
        idempotent: true,
        open_world: true,
    })
}

#[async_trait]
impl Tool for LaunchAppTool {
    fn def(&self) -> &ToolDef {
        def()
    }

    async fn invoke(&self, args: Value) -> ToolResult {
        use cua_driver_core::tool_args::ArgsExt;
        let bundle_id = args.opt_str("bundle_id");
        let name = args.opt_str("name");
        let mut response_bundle_id = bundle_id.clone();
        let response_requested_name = name.clone();
        let urls: Vec<String> = args
            .str_array("urls")
            .into_iter()
            .map(normalize_launch_url)
            .collect();
        if args.get("cdp_debugging_port").is_some() {
            return ToolResult::error(
                "cdp_debugging_port moved to browser_prepare so DevTools is never enabled on an unproven user profile",
            );
        }
        let webkit_inspector_port = args.opt_u64("webkit_inspector_port").map(|v| v as u16);
        let creates_new_instance = args.bool_or("creates_new_application_instance", false);
        let hidden = args.bool_or("hidden", false);
        let additional_arguments: Vec<String> = args.str_array("additional_arguments");
        if additional_arguments
            .iter()
            .any(|argument| argument == super::check_permissions::PERMISSIONS_HOST_REQUEST_ARG)
        {
            return protected_host_launch_refusal();
        }
        if additional_arguments
            .iter()
            .any(|argument| contains_remote_debugging_flag(argument))
        {
            return ToolResult::error(
                "Chromium remote-debugging flags moved to browser_prepare so DevTools is never enabled on an unproven user profile",
            );
        }

        if bundle_id.is_none() && name.is_none() {
            return ToolResult::error(
                "Provide either bundle_id or name to identify the app to launch.",
            );
        }
        if bundle_id.as_deref().is_some_and(is_cua_driver_bundle_id) {
            return protected_host_launch_refusal();
        }
        if let Some(ref bid) = bundle_id {
            if crate::apps::resolve_bundle_id_to_locator(bid).is_none() {
                return structured_launch_error(
                    "APP_NOT_INSTALLED",
                    format!("No installed macOS app found for bundle_id '{bid}'."),
                    serde_json::json!({ "bundle_id": bid }),
                );
            }
        } else if let Some(ref n) = name {
            let Some(locator) = crate::apps::locate_by_name(n) else {
                return structured_launch_error(
                    "APP_NOT_INSTALLED",
                    format!("No installed macOS app found for name '{n}'."),
                    serde_json::json!({ "name": n }),
                );
            };
            let (_, resolved_bundle_id) = locator.app_ref_and_bundle_id();
            response_bundle_id = resolved_bundle_id.clone();
            if resolved_bundle_id
                .as_deref()
                .is_some_and(is_cua_driver_bundle_id)
            {
                return protected_host_launch_refusal();
            }
        }
        if let Some(err) = preflight_file_urls(&urls) {
            return err;
        }

        // Build env dict for webkit inspector.
        let mut env: std::collections::HashMap<String, String> = std::collections::HashMap::new();
        if let Some(port) = webkit_inspector_port {
            env.insert(
                "WEBKIT_INSPECTOR_SERVER".to_string(),
                format!("127.0.0.1:{port}"),
            );
            env.insert("TAURI_WEBVIEW_AUTOMATION".to_string(), "1".to_string());
        }

        let port_summary = {
            let mut s = String::new();
            if let Some(port) = webkit_inspector_port {
                s.push_str(&format!("\nWebKit inspector available on port {port}."));
            }
            s
        };

        // LaunchServices is asked not to activate. Never compensate by activating
        // a saved prior app: that would overwrite a user's later choice, including
        // after this RPC has already returned.
        let prior_frontmost = crate::apps::frontmost_pid();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        let launch_result = tokio::task::spawn_blocking(move || {
            crate::input::cancellation::gate().ensure_active()?;
            let pid = if let Some(ref bid) = bundle_id {
                if urls.is_empty()
                    && additional_arguments.is_empty()
                    && env.is_empty()
                    && !creates_new_instance
                {
                    crate::apps::launch_app_with_options(bid, hidden)?
                } else {
                    crate::apps::launch_with_urls_by_bundle(
                        bid,
                        &urls,
                        &additional_arguments,
                        &env,
                        creates_new_instance,
                        hidden,
                    )?
                }
            } else {
                let n = name.as_deref().unwrap();
                if urls.is_empty()
                    && additional_arguments.is_empty()
                    && env.is_empty()
                    && !creates_new_instance
                {
                    crate::apps::launch_app_by_name_with_options(n, hidden)?
                } else {
                    crate::apps::launch_with_urls_by_name(
                        n,
                        &urls,
                        &additional_arguments,
                        &env,
                        creates_new_instance,
                        hidden,
                    )?
                }
            };

            // Use the remainder of the launch budget for WindowServer readiness.
            // Explicitly hidden apps cannot become ready through more polling.
            let windows = resolve_windows_for_pid(pid, hidden, deadline);

            let app_info: Option<crate::apps::AppInfo> = {
                let apps = crate::apps::list_running_apps();
                apps.into_iter().find(|a| a.pid == pid)
            };

            Ok::<_, anyhow::Error>((pid, app_info, windows))
        })
        .await;

        match launch_result {
            Ok(Ok((pid, app_info, windows))) => {
                let (app_name, bid) = response_identity(
                    app_info.as_ref(),
                    response_bundle_id.as_deref(),
                    response_requested_name.as_deref(),
                );

                let actual_hidden = crate::apps::app_is_hidden(pid);
                let visibility_change_confirmed = actual_hidden == hidden;
                let window_reason = launch_window_reason(&windows, actual_hidden)
                    .or((!visibility_change_confirmed).then_some("input_unavailable"));
                let window_ready = window_reason.is_none();
                let mut summary = format!(
                    "{app_name} is running (pid {pid}); foreground activation was not requested."
                );
                if let Some(reason) = window_reason {
                    summary.push_str(&format!(
                        " No usable window is available ({reason}). Do not kill or relaunch the app automatically."
                    ));
                }
                summary.push_str(&port_summary);

                if !windows.is_empty() {
                    summary.push_str("\n\nWindows:");
                    for w in &windows {
                        let title = if w.title.is_empty() {
                            "(no title)".to_owned()
                        } else {
                            format!("\"{}\"", w.title)
                        };
                        summary.push_str(&format!("\n- {title} [window_id: {}]", w.window_id));
                    }
                    summary.push_str(&format!(
                        "\n→ Call get_window_state(pid: {pid}, window_id) to inspect."
                    ));
                }

                let windows_json: Vec<Value> = windows
                    .iter()
                    .map(super::list_windows::window_record_json)
                    .collect();

                let mut structured = serde_json::json!({
                    "pid": pid,
                    "bundle_id": bid,
                    "name": app_name,
                    "windows": windows_json,
                    "hidden": actual_hidden,
                    "visibility_change_confirmed": visibility_change_confirmed,
                    "launch_state": launch_state(true, true, window_ready),
                    "window_status": if window_ready { "ready" } else { "no_usable_window" },
                });
                if let Some(reason) = window_reason {
                    structured["window_reason"] = Value::String(reason.to_owned());
                }
                if let Some(prior) = prior_frontmost {
                    // Observed outcome, not a promise that the launched app cannot
                    // activate itself later. Never perform a focus correction.
                    structured["focus_changed_during_launch"] =
                        Value::Bool(crate::apps::frontmost_pid() != Some(prior));
                }
                ToolResult::text(summary).with_structured(structured)
            }
            Ok(Err(e)) => structured_launch_failure(&e),
            Err(e) => ToolResult::error(format!("Task error: {e}")),
        }
    }
}

fn contains_remote_debugging_flag(value: &str) -> bool {
    let lower = value.to_ascii_lowercase();
    lower.contains("--remote-debugging-port") || lower.contains("--remote-debugging-pipe")
}

fn is_cua_driver_bundle_id(bundle_id: &str) -> bool {
    matches!(bundle_id, "com.trycua.driver" | "com.trycua.driver.local")
}

fn protected_host_launch_refusal() -> ToolResult {
    structured_launch_error(
        "PROTECTED_HOST_ENTRYPOINT",
        "launch_app cannot launch Cua Driver's protected host; operating-system permission UI must originate outside the agent tool stream".to_owned(),
        serde_json::json!({}),
    )
}

// ── Blocking helpers ──────────────────────────────────────────────────────────

/// Read only; never switch Spaces, raise a window, or reopen the app to make
/// readiness pass. Window creation shares the five-second launch budget.
fn resolve_windows_for_pid(
    pid: i32,
    requested_hidden: bool,
    deadline: std::time::Instant,
) -> Vec<crate::windows::WindowInfo> {
    loop {
        let windows: Vec<_> = crate::windows::all_windows()
            .into_iter()
            .filter(|window| window.pid == pid && window.layer == 0)
            .filter(|window| window.bounds.width > 1.0 && window.bounds.height > 1.0)
            .collect();
        if requested_hidden
            || launch_window_reason(&windows, crate::apps::app_is_hidden(pid)).is_none()
            || (!windows.is_empty()
                && windows
                    .iter()
                    .all(|window| window.on_current_space == Some(false)))
            || std::time::Instant::now() >= deadline
            || crate::input::cancellation::gate().ensure_active().is_err()
        {
            return windows;
        }
        std::thread::sleep(
            std::time::Duration::from_millis(100)
                .min(deadline.saturating_duration_since(std::time::Instant::now())),
        );
    }
}

fn launch_window_reason(
    windows: &[crate::windows::WindowInfo],
    hidden: bool,
) -> Option<&'static str> {
    if hidden {
        return Some("hidden");
    }
    if windows
        .iter()
        .any(|window| window.is_on_screen && window.on_current_space != Some(false))
    {
        return None;
    }
    if windows.is_empty() {
        Some("no_window")
    } else if windows
        .iter()
        .all(|window| window.on_current_space == Some(false))
    {
        Some("off_space")
    } else {
        Some("input_unavailable")
    }
}

fn structured_launch_error(code: &str, message: String, details: serde_json::Value) -> ToolResult {
    let mut payload = serde_json::json!({
        "error": code,
    });

    match details {
        serde_json::Value::Object(details) => {
            if let serde_json::Value::Object(payload) = &mut payload {
                payload.extend(details);
            }
        }
        details => {
            if let serde_json::Value::Object(payload) = &mut payload {
                payload.insert("details".to_string(), details);
            }
        }
    }

    ToolResult::error(message).with_structured(payload)
}

fn launch_state(requested: bool, process_running: bool, window_ready: bool) -> serde_json::Value {
    serde_json::json!({
        "requested": requested,
        "process_running": process_running,
        "window_ready": window_ready,
    })
}

fn response_identity(
    app_info: Option<&crate::apps::AppInfo>,
    requested_bundle_id: Option<&str>,
    requested_name: Option<&str>,
) -> (String, String) {
    let bundle_id = app_info
        .and_then(|app| app.bundle_id.as_deref())
        .filter(|value| !value.is_empty())
        .or(requested_bundle_id)
        .unwrap_or("?")
        .to_owned();

    let name = app_info
        .map(|app| app.name.as_str())
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .unwrap_or_else(|| requested_app_name(requested_name, requested_bundle_id));

    (name, bundle_id)
}

fn requested_app_name(requested_name: Option<&str>, requested_bundle_id: Option<&str>) -> String {
    if let Some(name) = requested_name.filter(|name| Some(*name) != requested_bundle_id) {
        let file_name = std::path::Path::new(name)
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or(name);
        return file_name
            .strip_suffix(".app")
            .unwrap_or(file_name)
            .to_owned();
    }

    requested_bundle_id
        .and_then(|bundle_id| bundle_id.rsplit('.').next())
        .filter(|name| !name.is_empty())
        .unwrap_or("?")
        .to_owned()
}

fn structured_launch_failure(error: &anyhow::Error) -> ToolResult {
    use crate::apps::nsworkspace::LaunchError;

    let (code, requested) = if let Some(launch_error) = error.downcast_ref::<LaunchError>() {
        match launch_error {
            LaunchError::Cocoa(_) => ("NSWORKSPACE_LAUNCH_FAILED", true),
            LaunchError::NoApp => ("LAUNCH_RESULT_MISSING", true),
            LaunchError::Timeout => ("LAUNCH_CALLBACK_TIMEOUT", true),
            LaunchError::BadUrl(_) => ("APP_URL_INVALID", false),
            LaunchError::Cancelled => ("LAUNCH_CANCELLED", false),
        }
    } else {
        ("LAUNCH_FAILED", false)
    };

    structured_launch_error(
        code,
        format!("Launch failed: {error:#}"),
        serde_json::json!({
            "launch_state": launch_state(requested, false, false),
        }),
    )
}

fn preflight_file_urls(urls: &[String]) -> Option<ToolResult> {
    for raw in urls {
        let Some(path) = local_file_target(raw) else {
            continue;
        };
        if !path.exists() {
            return Some(structured_launch_error(
                "FILE_NOT_FOUND",
                format!(
                    "Local launch_app url target does not exist: {}",
                    path.display()
                ),
                serde_json::json!({
                    "url": raw,
                    "path": path.display().to_string(),
                }),
            ));
        }
    }
    None
}

fn normalize_launch_url(raw: String) -> String {
    local_file_target(&raw)
        .map(|path| path.to_string_lossy().into_owned())
        .unwrap_or(raw)
}

fn local_file_target(raw: &str) -> Option<PathBuf> {
    if raw.is_empty() {
        return Some(PathBuf::from(raw));
    }
    if let Some(rest) = raw.strip_prefix("file://") {
        let path = rest.strip_prefix("localhost").unwrap_or(rest);
        let decoded = percent_decode_path(path);
        return Some(expand_tilde(&decoded));
    }
    let looks_like_url = raw.contains(':') && !raw.starts_with('/') && !raw.starts_with('~');
    if looks_like_url {
        return None;
    }
    Some(expand_tilde(raw))
}

fn expand_tilde(path: &str) -> PathBuf {
    if path == "~" {
        if let Ok(home) = std::env::var("HOME") {
            return PathBuf::from(home);
        }
    } else if let Some(rest) = path.strip_prefix("~/") {
        if let Ok(home) = std::env::var("HOME") {
            return PathBuf::from(home).join(rest);
        }
    }
    PathBuf::from(path)
}

fn percent_decode_path(path: &str) -> String {
    let bytes = path.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut i = 0;

    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let (Some(high), Some(low)) = (hex_value(bytes[i + 1]), hex_value(bytes[i + 2])) {
                decoded.push((high << 4) | low);
                i += 3;
                continue;
            }
        }

        decoded.push(bytes[i]);
        i += 1;
    }

    String::from_utf8_lossy(&decoded).into_owned()
}

fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::{
        contains_remote_debugging_flag, is_cua_driver_bundle_id, launch_window_reason,
        local_file_target, normalize_launch_url, preflight_file_urls, response_identity,
        structured_launch_failure, LaunchAppTool,
    };
    use cua_driver_core::tool::Tool;
    use serde_json::json;
    use std::path::PathBuf;

    fn launch_window(
        is_on_screen: bool,
        on_current_space: Option<bool>,
    ) -> crate::windows::WindowInfo {
        crate::windows::WindowInfo {
            window_id: 7,
            pid: 42,
            app_name: "Fixture".to_owned(),
            title: "Window".to_owned(),
            bounds: crate::windows::WindowBounds {
                x: 0.0,
                y: 0.0,
                width: 400.0,
                height: 300.0,
            },
            layer: 0,
            z_index: 0,
            is_on_screen,
            on_screen_evidence: Some(is_on_screen),
            current_space_id: Some(1),
            on_current_space,
            space_ids: Some(vec![if on_current_space == Some(false) {
                2
            } else {
                1
            }]),
        }
    }

    #[test]
    fn hidden_and_off_space_windows_are_not_launch_readiness() {
        assert_eq!(launch_window_reason(&[], false), Some("no_window"));
        assert_eq!(
            launch_window_reason(&[launch_window(true, Some(true))], true),
            Some("hidden")
        );
        assert_eq!(
            launch_window_reason(&[launch_window(false, Some(false))], false),
            Some("off_space")
        );
        assert_eq!(
            launch_window_reason(&[launch_window(false, Some(true))], false),
            Some("input_unavailable")
        );
        assert_eq!(
            launch_window_reason(
                &[
                    launch_window(false, Some(false)),
                    launch_window(true, Some(true)),
                ],
                false
            ),
            None
        );
    }

    #[test]
    fn local_file_target_treats_plain_paths_as_files() {
        assert_eq!(
            local_file_target("/tmp/does-not-exist.md"),
            Some(PathBuf::from("/tmp/does-not-exist.md"))
        );
        assert_eq!(
            local_file_target("relative/path.md"),
            Some(PathBuf::from("relative/path.md"))
        );
    }

    #[test]
    fn launch_url_normalization_expands_home_relative_paths() {
        let home = PathBuf::from(std::env::var_os("HOME").expect("HOME must be set for macOS"));

        assert_eq!(
            PathBuf::from(normalize_launch_url("~/Desktop/BenchInbox".to_owned())),
            home.join("Desktop/BenchInbox")
        );
        assert_eq!(
            normalize_launch_url("https://example.com".to_owned()),
            "https://example.com"
        );
    }

    #[test]
    fn local_file_target_skips_remote_and_custom_schemes() {
        assert_eq!(local_file_target("https://example.com"), None);
        assert_eq!(local_file_target("about:blank"), None);
        assert_eq!(local_file_target("myapp://open/item"), None);
    }

    #[test]
    fn preflight_file_urls_returns_structured_file_not_found() {
        let missing = "/tmp/cua-driver-definitely-missing-file-for-test.md".to_string();
        let result = preflight_file_urls(&[missing]).expect("missing file should error");
        assert_eq!(result.is_error, Some(true));
        let structured = result.structured_content.expect("structured error");
        assert_eq!(structured["error"], "FILE_NOT_FOUND");
        assert_eq!(
            structured["path"],
            "/tmp/cua-driver-definitely-missing-file-for-test.md"
        );
        assert!(structured.get("details").is_none());
    }

    #[test]
    fn local_file_target_percent_decodes_file_urls_before_path_checks() {
        assert_eq!(
            local_file_target("file:///tmp/My%20Doc.txt"),
            Some(PathBuf::from("/tmp/My Doc.txt"))
        );
        assert_eq!(
            local_file_target("file://localhost/tmp/%E2%9C%93.txt"),
            Some(PathBuf::from("/tmp/✓.txt"))
        );
    }

    #[test]
    fn rejects_all_chromium_remote_debugging_spellings() {
        assert!(contains_remote_debugging_flag("--remote-debugging-port=0"));
        assert!(contains_remote_debugging_flag("--REMOTE-DEBUGGING-PIPE"));
        assert!(!contains_remote_debugging_flag(
            "--user-data-dir=/tmp/profile"
        ));
    }

    #[test]
    fn recognizes_release_and_local_protected_host_bundle_ids() {
        assert!(is_cua_driver_bundle_id("com.trycua.driver"));
        assert!(is_cua_driver_bundle_id("com.trycua.driver.local"));
        assert!(!is_cua_driver_bundle_id("com.trycua.harness.tauri"));
    }

    #[test]
    fn launch_timeout_reports_requested_without_process_or_window() {
        let error = anyhow::Error::new(crate::apps::nsworkspace::LaunchError::Timeout)
            .context("Failed to launch com.example.App");
        let result = structured_launch_failure(&error);
        let structured = result.structured_content.expect("structured error");

        assert_eq!(result.is_error, Some(true));
        assert_eq!(structured["error"], "LAUNCH_CALLBACK_TIMEOUT");
        assert_eq!(structured["launch_state"]["requested"], true);
        assert_eq!(structured["launch_state"]["process_running"], false);
        assert_eq!(structured["launch_state"]["window_ready"], false);
    }

    #[test]
    fn cancellation_before_dispatch_does_not_claim_a_launch_request() {
        let error = anyhow::Error::new(crate::apps::nsworkspace::LaunchError::Cancelled);
        let result = structured_launch_failure(&error);
        let structured = result.structured_content.expect("structured error");
        assert_eq!(structured["error"], "LAUNCH_CANCELLED");
        assert_eq!(structured["launch_state"]["requested"], false);
    }

    #[test]
    fn invalid_url_reports_request_was_not_sent() {
        let error = anyhow::Error::new(crate::apps::nsworkspace::LaunchError::BadUrl(
            "bad url".to_owned(),
        ))
        .context("Failed to launch com.example.App");
        let result = structured_launch_failure(&error);
        let structured = result.structured_content.expect("structured error");

        assert_eq!(structured["error"], "APP_URL_INVALID");
        assert_eq!(structured["launch_state"]["requested"], false);
        assert_eq!(structured["launch_state"]["process_running"], false);
        assert_eq!(structured["launch_state"]["window_ready"], false);
    }

    #[test]
    fn process_only_response_falls_back_to_requested_identity() {
        assert_eq!(
            response_identity(None, Some("com.apple.Safari"), None),
            ("Safari".to_owned(), "com.apple.Safari".to_owned())
        );
        assert_eq!(
            response_identity(
                None,
                Some("com.example.Editor"),
                Some("/Applications/Example Editor.app"),
            ),
            ("Example Editor".to_owned(), "com.example.Editor".to_owned())
        );
    }

    #[tokio::test]
    async fn launch_app_cannot_reach_private_permission_host_entrypoint() {
        let result = LaunchAppTool
            .invoke(json!({
                "bundle_id": "com.example.not-installed",
                "additional_arguments": [
                    "__permissions-host-request",
                    "--result-file",
                    "/tmp/cua-driver-permissions-forged.json"
                ]
            }))
            .await;
        assert_eq!(result.is_error, Some(true));
        assert_eq!(
            result.structured_content.unwrap()["error"],
            "PROTECTED_HOST_ENTRYPOINT"
        );

        let result = LaunchAppTool
            .invoke(json!({ "bundle_id": "com.trycua.driver" }))
            .await;
        assert_eq!(result.is_error, Some(true));
        assert_eq!(
            result.structured_content.unwrap()["error"],
            "PROTECTED_HOST_ENTRYPOINT"
        );
    }
}
