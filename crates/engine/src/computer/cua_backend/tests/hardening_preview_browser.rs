use serde_json::json;
use zeron_proto::computer::{
    ComputerAvailability, ComputerBuildSignature, ComputerHealthStatus, ComputerInputModifier,
    ComputerPermission, ComputerState, CuaComputerTask, CuaToolResult,
};

use super::*;
use crate::computer::backend::{ComputerAgentDialect, ComputerBrowserCall};
use crate::computer::task_context::with_computer_task;

fn task(thread_id: &str, turn_id: Option<&str>) -> CuaComputerTask {
    CuaComputerTask {
        thread_id: thread_id.to_owned(),
        turn_id: turn_id.map(str::to_owned),
        label: None,
    }
}

async fn browser_call(
    f: &Fixture,
    name: &str,
    args: Value,
    task: CuaComputerTask,
    mutation: bool,
) -> Result<CuaToolResult, ComputerError> {
    f.backend
        .browser()
        .unwrap()
        .call(ComputerBrowserCall {
            name: name.to_owned(),
            args: args.as_object().cloned().unwrap(),
            task,
            mutation,
            signal: None,
        })
        .await
}

fn with_pixels(window_id: Option<&str>, include_tree: bool) -> ComputerStateOptions {
    ComputerStateOptions {
        window_id: window_id.map(str::to_owned),
        include_tree,
        include_screenshot: true,
        ..ComputerStateOptions::default()
    }
}

fn last_call(f: &Fixture) -> ComputerHostCall {
    f.calls().last().cloned().unwrap()
}

#[tokio::test(start_paused = true)]
async fn reports_a_stable_build_signature_on_the_backend_and_its_availability() {
    let f = fixture();
    assert_eq!(
        f.backend.build_signature(),
        Some(ComputerBuildSignature::Unknown)
    );
    assert_eq!(f.backend.build_signature(), f.backend.build_signature());
    f.deny_permissions();
    let availability = f.backend.availability(false).await.unwrap();
    let ComputerAvailability::PermissionRequired {
        build_signature, ..
    } = availability
    else {
        panic!("expected permission-required, got {availability:?}");
    };
    assert_eq!(build_signature, ComputerBuildSignature::Unknown);
}

#[tokio::test]
async fn pauses_input_while_an_auth_sheet_holds_focus_keeping_observation_available() {
    let f = fixture();
    f.action_result(json!({
        "effect": "refused",
        "code": "auth_sheet_focused",
        "message": "An authentication sheet has focus.",
    }));
    let error = f
        .backend
        .type_text("abc", Some("cua:10:20"), None)
        .await
        .unwrap_err();
    assert_eq!(effect(&error), Some(CuaEffect::NotDispatched));
    assert_eq!(code(&error), Some("auth_sheet_focused"));
    assert_eq!(
        error
            .input_pause()
            .and_then(|pause| pause.window_id.as_deref()),
        Some("cua:10:20")
    );
    let state = f
        .backend
        .get_state(ComputerStateOptions {
            window_id: Some("cua:10:20".into()),
            ..ComputerStateOptions::default()
        })
        .await
        .unwrap();
    assert_eq!(state.computer_id, "desktop");
}

#[tokio::test]
async fn flips_health_on_capture_failure_without_blocking_input_and_heals_on_refresh() {
    let f = fixture();
    f.capture_window(21, 10);
    let error = f
        .backend
        .capture_screenshot(window_capture("cua:10:20"))
        .await
        .unwrap_err();
    assert_eq!(effect(&error), Some(CuaEffect::NotDispatched));
    let health = f.backend.health();
    assert_eq!(health.status, ComputerHealthStatus::Unavailable);
    assert!(!health.capture_available);
    assert!(health.consecutive_failures > 0);
    f.capture_window(20, 10);
    f.backend
        .type_text("abc", Some("cua:10:20"), None)
        .await
        .unwrap();
    assert!(matches!(
        f.backend.availability(false).await.unwrap(),
        ComputerAvailability::Available { .. }
    ));
    let health = f.backend.health();
    assert_eq!(health.status, ComputerHealthStatus::Connected);
    assert!(health.capture_available);
}

#[tokio::test]
async fn keeps_capture_health_when_a_tree_only_observation_fails() {
    let f = fixture();
    f.set_elements(vec![json!({
        "role": "AXButton",
        "label": "Equals",
        "frame": { "x": -290, "y": 30, "width": 20, "height": 20 },
        "element_token": "fresh-token",
    })]);
    f.backend
        .get_state(window_state("cua:10:20"))
        .await
        .unwrap();
    let health = f.backend.health();
    assert_eq!(health.status, ComputerHealthStatus::Connected);
    assert!(health.capture_available);
    f.capture_window(999, 10);
    let error = f
        .backend
        .get_state(window_state("cua:10:20"))
        .await
        .unwrap_err();
    assert!(error.to_string().contains("belongs to a different window"));
    let health = f.backend.health();
    assert_eq!(health.status, ComputerHealthStatus::Connected);
    assert!(health.capture_available);
    assert_eq!(health.consecutive_failures, 0);
    let error = f
        .backend
        .get_state(with_pixels(Some("cua:10:20"), true))
        .await
        .unwrap_err();
    assert!(error.to_string().contains("belongs to a different window"));
    let health = f.backend.health();
    assert_eq!(health.status, ComputerHealthStatus::Unavailable);
    assert!(!health.capture_available);
}

#[tokio::test]
async fn returns_a_preview_note_instead_of_failing_the_observation_on_preview_only_failure() {
    let f = fixture();
    f.set_elements(vec![json!({
        "role": "AXButton",
        "label": "Equals",
        "frame": { "x": -290, "y": 30, "width": 20, "height": 20 },
        "element_token": "fresh-token",
        "actions": ["AXPress"],
    })]);
    f.invalidate_capture();
    let state = f
        .backend
        .get_state(with_pixels(Some("cua:10:20"), true))
        .await
        .unwrap();
    assert!(state.screenshot.is_none());
    let note = state.preview_note.clone().unwrap();
    assert!(note.contains("Reselect the window to resume"));
    assert_eq!(state.root.as_ref().unwrap().children.len(), 1);
    let decoded: ComputerState =
        serde_json::from_value(serde_json::to_value(&state).unwrap()).unwrap();
    assert_eq!(decoded.preview_note.as_deref(), Some(note.as_str()));
    f.backend
        .type_text("abc", Some("cua:10:20"), None)
        .await
        .unwrap();
    f.backend.dispose().await;
}

#[tokio::test]
async fn pauses_off_space_pixels_instead_of_presenting_freshness_unverified_capture_as_live() {
    let f = fixture();
    f.mark_off_space_capture_unverified();
    let state = f
        .backend
        .get_state(with_pixels(Some("cua:10:20"), true))
        .await
        .unwrap();
    assert!(state.screenshot.is_none());
    assert!(
        state
            .preview_note
            .as_deref()
            .unwrap()
            .contains("another macOS Space")
    );
    let health = f.backend.health();
    assert_eq!(health.status, ComputerHealthStatus::Connected);
    assert!(health.capture_available);
    let error = f
        .backend
        .capture_screenshot(window_capture("cua:10:20"))
        .await
        .unwrap_err();
    assert_eq!(code(&error), Some("off_space_capture_unverified"));
    assert_eq!(effect(&error), Some(CuaEffect::NotDispatched));
    let health = f.backend.health();
    assert_eq!(health.status, ComputerHealthStatus::Connected);
    assert!(health.capture_available);
}

#[tokio::test]
async fn pauses_retained_pixels_of_a_minimized_or_hidden_window() {
    let f = fixture();
    f.mark_hidden_capture_unverified();
    let state = f
        .backend
        .get_state(with_pixels(Some("cua:10:20"), true))
        .await
        .unwrap();
    assert!(state.screenshot.is_none());
    assert!(state.preview_note.as_deref().unwrap().contains("minimized"));
    let error = f
        .backend
        .capture_screenshot(window_capture("cua:10:20"))
        .await
        .unwrap_err();
    assert_eq!(code(&error), Some("hidden_capture_unverified"));
    assert_eq!(effect(&error), Some(CuaEffect::NotDispatched));
    let health = f.backend.health();
    assert_eq!(health.status, ComputerHealthStatus::Connected);
    assert!(health.capture_available);
}

#[tokio::test]
async fn clears_window_grounding_when_the_owning_task_ends() {
    let f = fixture();
    let owner = task("thread", Some("turn"));
    with_computer_task(
        owner.clone(),
        f.backend.capture_screenshot(window_capture("cua:10:20")),
    )
    .await
    .unwrap();
    with_computer_task(
        owner.clone(),
        f.backend.click(point(-275.0, 30.0), Some("cua:10:20"), &[]),
    )
    .await
    .unwrap();
    f.backend.end_task("thread", Some("turn")).await.unwrap();
    let error = f
        .backend
        .click(point(-275.0, 30.0), Some("cua:10:20"), &[])
        .await
        .unwrap_err();
    assert_eq!(code(&error), Some("stale_geometry"));
    f.backend.dispose().await;
}

#[tokio::test(start_paused = true)]
async fn degrades_blind_on_a_mid_task_screen_recording_revoke_without_replaying_input() {
    let f = fixture();
    f.backend
        .capture_screenshot(window_capture("cua:10:20"))
        .await
        .unwrap();
    f.backend
        .click(point(-275.0, 30.0), Some("cua:10:20"), &[])
        .await
        .unwrap();
    let clicks = f.named("click").len();
    f.deny_screen_recording();
    let availability = f.backend.availability(false).await.unwrap();
    let ComputerAvailability::PermissionRequired { missing, .. } = &availability else {
        panic!("expected permission-required, got {availability:?}");
    };
    assert_eq!(missing, &vec![ComputerPermission::ScreenRecording]);
    let blind = f.backend.get_state(with_pixels(None, false)).await.unwrap();
    assert!(blind.screenshot.is_none());
    let state = f
        .backend
        .get_state(window_state("cua:10:20"))
        .await
        .unwrap();
    assert_eq!(state.computer_id, "desktop");
    assert!(!f.backend.health().capture_available);
    f.backend
        .type_text("abc", Some("cua:10:20"), None)
        .await
        .unwrap();
    assert_eq!(f.named("click").len(), clicks);
    assert_eq!(f.named("type_text").len(), 1);
    f.backend.dispose().await;
}

#[tokio::test]
async fn requires_a_fresh_granted_observation_to_recover_from_a_failed_capture() {
    let f = fixture();
    f.fail_overview();
    f.backend
        .get_state(with_pixels(None, false))
        .await
        .unwrap_err();
    let health = f.backend.health();
    assert_eq!(health.status, ComputerHealthStatus::Unavailable);
    assert!(!health.capture_available);
    let overviews = f.named("get_desktop_state").len();
    assert!(f.named("click").is_empty() && f.named("type_text").is_empty());
    f.backend
        .type_text("abc", Some("cua:10:20"), None)
        .await
        .unwrap();
    f.grant_permissions();
    f.backend.provision().await.unwrap();
    let health = f.backend.health();
    assert_eq!(health.status, ComputerHealthStatus::Connected);
    assert!(health.capture_available);
    f.backend
        .get_state(with_pixels(None, false))
        .await
        .unwrap_err();
    let health = f.backend.health();
    assert_eq!(health.status, ComputerHealthStatus::Unavailable);
    assert!(!health.capture_available);
    assert_eq!(f.named("get_desktop_state").len(), overviews + 1);
    f.backend.dispose().await;
}

#[tokio::test]
async fn drops_grounding_after_uncertain_delivery_but_keeps_it_after_a_clean_refusal() {
    let f = fixture();
    f.backend
        .capture_screenshot(window_capture("cua:10:20"))
        .await
        .unwrap();
    f.fail(json!({ "ok": false, "error": "timeout", "effect": "dispatched-unknown" }));
    let error = f
        .backend
        .type_text("abc", Some("cua:10:20"), None)
        .await
        .unwrap_err();
    assert_eq!(effect(&error), Some(CuaEffect::DispatchedUnknown));
    let error = f
        .backend
        .click(point(-275.0, 30.0), Some("cua:10:20"), &[])
        .await
        .unwrap_err();
    assert_eq!(code(&error), Some("stale_geometry"));
    f.backend
        .capture_screenshot(window_capture("cua:10:20"))
        .await
        .unwrap();
    f.refuse();
    f.unfail();
    let error = f
        .backend
        .type_text("abc", Some("cua:10:20"), None)
        .await
        .unwrap_err();
    assert_eq!(effect(&error), Some(CuaEffect::NotDispatched));
    f.backend
        .click(point(-275.0, 30.0), Some("cua:10:20"), &[])
        .await
        .unwrap();
    f.backend.dispose().await;
}

#[tokio::test]
async fn does_no_host_work_when_an_ordinary_turn_ends() {
    let f = fixture();
    f.backend.end_task("ordinary", Some("turn")).await.unwrap();
    assert_eq!(f.call_count(), 0);
}

#[tokio::test]
async fn attributes_observations_and_ends_only_the_matching_turn() {
    let f = fixture();
    let owner = task("thread", Some("turn"));
    with_computer_task(
        owner.clone(),
        observing(f.backend.get_state(with_pixels(Some("cua:10:20"), false))),
    )
    .await
    .unwrap();
    assert!(f.named("get_window_state").iter().any(|call| {
        call.task.as_ref() == Some(&owner) && call.model_observation == Some(true)
    }));
    let before = f.call_count();
    f.backend.end_task("thread", Some("old")).await.unwrap();
    assert_eq!(f.call_count(), before);
    f.backend.end_task("thread", Some("turn")).await.unwrap();
    let last = last_call(&f);
    assert_eq!(last.method, ComputerHostMethod::EndTask);
    assert_eq!(last.task.as_ref(), Some(&owner));
    f.backend.end_task("thread", Some("turn")).await.unwrap();
    assert_eq!(f.call_count(), before + 1);
}

#[tokio::test]
async fn publishes_nothing_when_no_window_or_tab_is_the_target() {
    let f = fixture();
    assert_eq!(f.backend.capture_still().await.unwrap(), None);
    assert!(f.named("get_desktop_state").is_empty());
    assert!(f.named("get_window_state").is_empty());
    f.backend.dispose().await;
}

#[tokio::test]
async fn captures_the_exact_window_the_task_aims_at_never_the_desktop() {
    let f = fixture();
    f.backend.focus_window("cua:10:20").await.unwrap();
    f.clear_calls();
    assert!(f.backend.capture_still().await.unwrap().is_some());
    assert_subset(
        &f.first_args("get_window_state"),
        json!({
            "pid": 10,
            "window_id": 20,
            "include_screenshot": true,
            "include_accessibility_tree": false,
        }),
    );
    assert!(f.named("get_desktop_state").is_empty());
    f.backend.dispose().await;
}

#[tokio::test]
async fn captures_a_bound_browser_tab_through_the_drivers_screenshot_route() {
    use base64::Engine as _;
    let f = fixture();
    let owner = task("thread", Some("turn"));
    let image = base64::engine::general_purpose::STANDARD.encode([1u8, 2, 3]);
    f.on_tool("get_browser_state", move |args| {
        if args.contains_key("pid") {
            json!({
                "structuredContent": {
                    "status": "ok",
                    "target_id": "bt-1",
                    "tabs": [{ "tab_id": "tab-1", "active": true }],
                },
            })
        } else {
            json!({
                "structuredContent": { "status": "ok" },
                "content": [{ "type": "image", "mimeType": "image/png", "data": image }],
            })
        }
    });
    browser_call(
        &f,
        "get_browser_state",
        json!({ "pid": 42 }),
        owner.clone(),
        false,
    )
    .await
    .unwrap();
    f.clear_calls();
    assert_eq!(
        f.backend.capture_still().await.unwrap(),
        Some(vec![1, 2, 3])
    );
    let last = last_call(&f);
    assert_eq!(last.method, ComputerHostMethod::Call);
    assert_eq!(last.name.as_deref(), Some("get_browser_state"));
    assert_subset(
        &last.args,
        json!({ "target_id": "bt-1", "tab_id": "tab-1", "include_screenshot": true }),
    );
    assert_eq!(last.task.as_ref(), Some(&owner));
    assert!(f.named("get_desktop_state").is_empty());
    f.backend.dispose().await;
}

#[tokio::test]
async fn drops_the_browser_still_target_when_its_task_ends() {
    let f = fixture();
    f.on_tool("get_browser_state", |_| {
        json!({
            "structuredContent": { "status": "ok", "target_id": "bt-1", "tabs": [{ "tab_id": "tab-1" }] },
        })
    });
    browser_call(
        &f,
        "get_browser_state",
        json!({ "pid": 42 }),
        task("thread", Some("turn")),
        false,
    )
    .await
    .unwrap();
    f.backend.end_task("thread", Some("turn")).await.unwrap();
    f.clear_calls();
    assert_eq!(f.backend.capture_still().await.unwrap(), None);
    assert!(f.named("get_browser_state").is_empty());
    f.backend.dispose().await;
}

#[tokio::test]
async fn registers_a_browser_task_so_end_task_reaches_the_host() {
    let f = fixture();
    let owner = task("thread", Some("turn"));
    browser_call(
        &f,
        "get_browser_state",
        json!({ "pid": 10, "window_id": 20 }),
        owner.clone(),
        false,
    )
    .await
    .unwrap();
    f.backend.end_task("thread", Some("turn")).await.unwrap();
    let last = last_call(&f);
    assert_eq!(last.method, ComputerHostMethod::EndTask);
    assert_eq!(last.task.as_ref(), Some(&owner));
    f.backend.dispose().await;
}

#[tokio::test]
async fn a_refused_browser_call_still_ends_its_task_cleanly() {
    let f = fixture();
    let owner = task("thread", Some("turn"));
    f.on_tool("get_browser_state", |_| {
        json!({
            "structuredContent": {
                "status": "refused",
                "refusal": { "code": "browser_requires_setup", "message": "Prepare a browser first." },
            },
            "content": [{ "type": "text", "text": "refused (browser_requires_setup)" }],
        })
    });
    browser_call(
        &f,
        "get_browser_state",
        json!({ "pid": 10, "window_id": 20 }),
        owner.clone(),
        false,
    )
    .await
    .unwrap();
    f.backend.end_task("thread", Some("turn")).await.unwrap();
    let last = last_call(&f);
    assert_eq!(last.method, ComputerHostMethod::EndTask);
    assert_eq!(last.task.as_ref(), Some(&owner));
    f.backend.dispose().await;
}

#[tokio::test]
async fn sends_explicit_include_screenshot_false_on_a_tree_only_get_state() {
    let f = fixture();
    let state = f
        .backend
        .get_state(window_state("cua:10:20"))
        .await
        .unwrap();
    assert_subset(
        &f.first_args("get_window_state"),
        json!({
            "include_screenshot": false,
            "max_dimension": 1536,
            "include_accessibility_tree": true,
        }),
    );
    assert!(state.screenshot.is_none());
    f.backend.dispose().await;
}

#[tokio::test]
async fn keeps_the_capture_arguments_a_pixel_read_needs() {
    let f = fixture();
    f.backend
        .get_state(with_pixels(Some("cua:10:20"), true))
        .await
        .unwrap();
    assert_subset(
        &f.first_args("get_window_state"),
        json!({
            "include_screenshot": true,
            "max_dimension": 1536,
            "include_accessibility_tree": true,
        }),
    );
    f.backend.dispose().await;
}

#[tokio::test]
async fn exposes_a_browser_route_whenever_the_backend_exists() {
    let f = fixture();
    assert!(f.backend.browser().is_some());
}

#[tokio::test]
async fn forwards_the_drivers_reply_verbatim_a_deliberate_refusal_is_a_result_not_an_error() {
    let f = fixture();
    let owner = task("thread", Some("turn"));
    f.on_tool("get_browser_state", |_| {
        json!({
            "content": [{ "type": "text", "text": "refused (browser_requires_setup)" }],
            "structuredContent": {
                "status": "refused",
                "refusal": { "code": "browser_requires_setup", "message": "Prepare a browser first." },
            },
        })
    });
    let result = browser_call(
        &f,
        "get_browser_state",
        json!({ "target_id": "bt-1", "tab_id": "tab-1" }),
        owner.clone(),
        false,
    )
    .await
    .unwrap();
    assert_subset(
        &Value::Object(result.structured_content.unwrap()),
        json!({ "status": "refused", "refusal": { "code": "browser_requires_setup" } }),
    );
    let last = last_call(&f);
    assert_eq!(last.method, ComputerHostMethod::Call);
    assert_eq!(last.name.as_deref(), Some("get_browser_state"));
    assert_subset(
        &last.args,
        json!({ "target_id": "bt-1", "tab_id": "tab-1" }),
    );
    assert_eq!(last.task.as_ref(), Some(&owner));
}

#[tokio::test]
async fn fails_closed_without_a_gui_host_instead_of_fabricating_a_route() {
    let f = fixture();
    f.backend.dispose().await;
    let error = browser_call(&f, "browser_click", json!({}), task("thread", None), true)
        .await
        .unwrap_err();
    assert_eq!(code(&error), Some("gui_host_required"));
}

#[tokio::test]
async fn ends_the_threads_driver_browser_session_through_the_host() {
    let f = fixture();
    f.backend
        .browser()
        .unwrap()
        .end_thread("thread-1")
        .await
        .unwrap();
    let last = last_call(&f);
    assert_eq!(last.method, ComputerHostMethod::EndBrowserThread);
    assert_eq!(last.task, Some(task("thread-1", None)));
}

#[tokio::test]
async fn adopts_the_host_platform_and_narrows_patched_only_capabilities_for_an_unpatched_driver() {
    let f = fixture_with(FixtureOptions {
        host_platform: Some("win32"),
        native_revision: Some(Some(0)),
        ..Default::default()
    });
    assert_eq!(f.backend.agent_dialect(), ComputerAgentDialect::Macos);
    assert!(f.backend.focus_neutral_semantic_text());
    assert!(f.backend.capabilities().ghost_cursor);
    f.backend.probe_availability().await.unwrap();
    assert_eq!(f.backend.agent_dialect(), ComputerAgentDialect::Linux);
    assert!(!f.backend.focus_neutral_semantic_text());
    assert!(!f.backend.capabilities().ghost_cursor);
}

#[tokio::test]
async fn keeps_the_patched_surface_for_a_driver_that_reports_its_revision() {
    let f = fixture_with(FixtureOptions {
        host_platform: Some("darwin"),
        native_revision: Some(Some(20)),
        ..Default::default()
    });
    f.backend.probe_availability().await.unwrap();
    assert_eq!(f.backend.agent_dialect(), ComputerAgentDialect::Macos);
    assert!(f.backend.focus_neutral_semantic_text());
    assert!(f.backend.capabilities().ghost_cursor);
}

#[tokio::test]
async fn does_not_advertise_macos_input_guarantees_for_a_patched_linux_browser_driver() {
    let f = fixture_with(FixtureOptions {
        host_platform: Some("linux"),
        native_revision: Some(Some(32)),
        ..Default::default()
    });
    f.backend.probe_availability().await.unwrap();
    assert!(!f.backend.focus_neutral_semantic_text());
    let capabilities = f.backend.capabilities();
    assert!(!capabilities.input);
    assert!(!capabilities.focus);
    assert!(!capabilities.raise);
    assert!(!capabilities.ghost_cursor);
}

fn linux() -> Fixture {
    fixture_with(FixtureOptions {
        host_platform: Some("linux"),
        ..Default::default()
    })
}

#[tokio::test]
async fn uses_the_strict_upstream_pixel_and_keyboard_schemas_only_in_authorized_foreground_mode() {
    let f = linux();
    f.backend
        .capture_screenshot(window_capture("cua:10:20"))
        .await
        .unwrap();
    foreground(async {
        f.backend
            .click(point(-275.0, 30.0), Some("cua:10:20"), &[])
            .await
            .unwrap();
        f.backend
            .type_text("visible input", Some("cua:10:20"), None)
            .await
            .unwrap();
        f.backend
            .drag(
                point(-275.0, 30.0),
                point(-225.0, 50.0),
                500.0,
                Some("cua:10:20"),
            )
            .await
            .unwrap();
    })
    .await;
    let click = f.named("click")[0].clone();
    assert_eq!(click.delivery_mode, Some(ComputerDeliveryMode::Foreground));
    assert_subset(
        &click.args,
        json!({ "pid": 10, "window_id": 20, "delivery_mode": "foreground", "count": 1, "x": 25, "y": 10 }),
    );
    assert_eq!(
        f.first_args("type_text"),
        json!({ "pid": 10, "window_id": 20, "delivery_mode": "foreground", "text": "visible input" })
    );
    assert_eq!(
        f.first_args("drag"),
        json!({
            "pid": 10,
            "window_id": 20,
            "delivery_mode": "foreground",
            "from_x": 25,
            "from_y": 10,
            "to_x": 75,
            "to_y": 30,
            "duration_ms": 500,
        })
    );
    for call in f
        .calls()
        .into_iter()
        .filter(|call| matches!(call.name.as_deref(), Some("click" | "type_text" | "drag")))
    {
        assert!(call.args.get("force_synthetic").is_none());
        assert!(call.args.get("coordinate_space").is_none());
        assert!(call.args.get("expected_window_bounds").is_none());
    }
}

#[tokio::test]
async fn refuses_default_background_input_without_dispatching_a_relaxed_linux_request() {
    let f = linux();
    let error = f
        .backend
        .type_text("must not type", Some("cua:10:20"), None)
        .await
        .unwrap_err();
    assert_eq!(effect(&error), Some(CuaEffect::NotDispatched));
    assert_eq!(code(&error), Some("linux_background_unavailable"));
    assert!(f.named("type_text").is_empty());
    assert!(
        f.calls()
            .iter()
            .all(|call| call.delivery_mode == Some(ComputerDeliveryMode::Background))
    );
}

#[tokio::test]
async fn translates_an_unmodified_single_axis_scroll_without_dropping_unsupported_gesture_semantics()
 {
    let f = linux();
    f.backend
        .capture_screenshot(window_capture("cua:10:20"))
        .await
        .unwrap();
    foreground(f.backend.scroll(
        Some(point(-275.0, 30.0)),
        0.0,
        240.0,
        Some("cua:10:20"),
        &[],
        None,
        None,
    ))
    .await
    .unwrap();
    assert_eq!(
        f.first_args("scroll"),
        json!({
            "pid": 10,
            "window_id": 20,
            "delivery_mode": "foreground",
            "direction": "down",
            "amount": 2,
            "by": "line",
            "x": 25,
            "y": 10,
        })
    );
    let error = foreground(f.backend.scroll(
        Some(point(-275.0, 30.0)),
        120.0,
        240.0,
        Some("cua:10:20"),
        &[],
        None,
        None,
    ))
    .await
    .unwrap_err();
    assert_eq!(effect(&error), Some(CuaEffect::NotDispatched));
    assert_eq!(code(&error), Some("unsupported_linux_operation"));
    let error = foreground(f.backend.scroll(
        Some(point(-275.0, 30.0)),
        0.0,
        240.0,
        Some("cua:10:20"),
        &[ComputerInputModifier::Ctrl],
        None,
        None,
    ))
    .await
    .unwrap_err();
    assert_eq!(effect(&error), Some(CuaEffect::NotDispatched));
    assert_eq!(code(&error), Some("unsupported_linux_operation"));
    assert_eq!(f.named("scroll").len(), 1);
}

#[tokio::test]
async fn does_not_downgrade_an_exact_semantic_text_target_to_focused_linux_typing() {
    let f = linux();
    f.set_elements(vec![json!({
        "role": "AXTextField",
        "label": "Fixture input",
        "element_token": "snapshot-token",
        "frame": { "x": -290, "y": 30, "width": 20, "height": 20 },
    })]);
    let state = f
        .backend
        .get_state(window_state("cua:10:20"))
        .await
        .unwrap();
    let node = state.root.unwrap().children[0].clone();
    let target = resolved("Fixture input", &node);
    let error = foreground(f.backend.type_text(
        "must preserve identity",
        Some("cua:10:20"),
        Some(&target),
    ))
    .await
    .unwrap_err();
    assert_eq!(effect(&error), Some(CuaEffect::NotDispatched));
    assert_eq!(code(&error), Some("linux_semantic_target_unproven"));
    assert!(f.named("type_text").is_empty() && f.named("set_value").is_empty());
}

#[tokio::test]
async fn keeps_native_arguments_separate_from_the_servers_delivery_authorization_envelope() {
    let f = linux();
    browser_call(
        &f,
        "browser_click",
        json!({ "target_id": "fixture", "delivery_mode": "foreground", "deliveryMode": "foreground" }),
        task("linux-envelope-test", None),
        true,
    )
    .await
    .unwrap();
    let last = last_call(&f);
    assert_eq!(last.delivery_mode, Some(ComputerDeliveryMode::Background));
    assert_subset(
        &last.args,
        json!({ "delivery_mode": "foreground", "deliveryMode": "foreground" }),
    );
}

#[tokio::test]
async fn marks_only_active_model_browser_observations_as_eligible_for_interruption_recovery() {
    let f = fixture();
    let observe = || {
        browser_call(
            &f,
            "get_browser_state",
            json!({ "target_id": "fixture", "tab_id": "tab" }),
            task("observation-test", None),
            false,
        )
    };
    observe().await.unwrap();
    assert_eq!(last_call(&f).model_observation, Some(false));
    observing(observe()).await.unwrap();
    assert_eq!(last_call(&f).model_observation, Some(true));
    observe().await.unwrap();
    assert_eq!(last_call(&f).model_observation, Some(false));
}

mod foreground_guard {
    use futures::FutureExt;
    use serde_json::json;
    use tokio_util::sync::CancellationToken;
    use zeron_proto::PermissionMode;

    use super::super::*;
    use crate::computer::approval_gate::{ApprovalDecision, ApprovalRequest, ComputerApprovalGate};

    #[derive(Default)]
    struct GuardHost {
        calls: Mutex<Vec<ComputerHostCall>>,
        readiness_refusal: Mutex<Option<Value>>,
        input_refusal: Mutex<Option<String>>,
    }

    impl GuardHost {
        fn respond(&self, call: &ComputerHostCall) -> Value {
            let bounds = json!({ "x": 0, "y": 0, "width": 200, "height": 100 });
            if matches!(
                call.method,
                ComputerHostMethod::Probe | ComputerHostMethod::Stop
            ) {
                return json!({ "ok": true });
            }
            let structured = match call.name.as_deref() {
                Some("check_permissions") => {
                    json!({ "accessibility": true, "screen_recording": true })
                }
                Some("list_windows") => json!({
                    "windows": [{
                        "pid": 10,
                        "window_id": 20,
                        "title": "Owned fixture",
                        "bounds": bounds,
                        "is_on_screen": true,
                        "on_current_space": true,
                        "z_index": 1,
                    }],
                }),
                Some("get_screen_size") => {
                    json!({ "width": 1000, "height": 800, "scale_factor": 2 })
                }
                Some("check_input_ready") => {
                    if let Some(refusal) = self.readiness_refusal.lock().unwrap().clone() {
                        return json!({
                            "ok": true,
                            "result": { "isError": true, "structuredContent": refusal },
                        });
                    }
                    json!({ "ready": true, "pid": 10, "window_id": 20 })
                }
                Some("get_window_state") => {
                    return json!({
                        "ok": true,
                        "result": {
                            "structuredContent": {
                                "pid": 10,
                                "window_id": 20,
                                "window_bounds": bounds,
                                "screenshot_frame_valid": true,
                                "elements": [],
                            },
                            "content": [{ "type": "image", "mimeType": "image/png", "data": png_header_base64() }],
                        },
                    });
                }
                Some("type_text") => {
                    if let Some(code) = self.input_refusal.lock().unwrap().clone() {
                        return json!({
                            "ok": true,
                            "result": {
                                "isError": true,
                                "structuredContent": {
                                    "effect": "refused",
                                    "code": code,
                                    "message": "The native operation could not complete.",
                                },
                                "content": [{ "type": "text", "text": "No actuator ran." }],
                            },
                        });
                    }
                    json!({})
                }
                _ => json!({}),
            };
            json!({ "ok": true, "result": { "structuredContent": structured } })
        }

        fn named(&self, name: &str) -> usize {
            self.calls
                .lock()
                .unwrap()
                .iter()
                .filter(|call| call.name.as_deref() == Some(name))
                .count()
        }
    }

    #[async_trait]
    impl ComputerHost for GuardHost {
        async fn request(&self, call: ComputerHostCall) -> CuaReply {
            self.calls.lock().unwrap().push(call.clone());
            let mut body = self.respond(&call);
            body["hostPlatform"] = json!("darwin");
            serde_json::from_value(body).expect("guard reply decodes")
        }

        fn cancel(&self, _correlation_id: &str) {}
    }

    fn guard_fixture() -> (CuaComputerBackend, Arc<GuardHost>) {
        let host = Arc::new(GuardHost::default());
        let backend = CuaComputerBackend::new(host.clone(), CuaComputerBackendOptions::default());
        (backend, host)
    }

    #[tokio::test]
    async fn background_never_activates_raise_is_refused_before_any_native_call() {
        let (backend, host) = guard_fixture();
        let error = backend.raise_window("cua:10:20").await.unwrap_err();
        assert_eq!(effect(&error), Some(CuaEffect::NotDispatched));
        assert_eq!(code(&error), Some("foreground_required"));
        assert_eq!(host.named("bring_to_front"), 0);
        assert!(host.calls.lock().unwrap().is_empty());
        backend.dispose().await;
    }

    #[tokio::test]
    async fn auth_sheets_never_approve_input_and_readiness_refuse_without_settling_approvals() {
        let (backend, host) = guard_fixture();
        let gate = ComputerApprovalGate::default();
        let thread_id = "foreground-guard-thread";
        let prompt_id = Arc::new(Mutex::new(String::new()));
        let publish = {
            let prompt_id = prompt_id.clone();
            Arc::new(move |id: String, decision: Option<ApprovalDecision>| {
                if decision.is_none() {
                    *prompt_id.lock().unwrap() = id;
                }
                async { Ok(()) }.boxed()
            })
        };
        let prompt = tokio::spawn({
            let gate = gate.clone();
            async move {
                gate.request(ApprovalRequest {
                    chat_id: thread_id.to_owned(),
                    turn_id: None,
                    mode: PermissionMode::Ask,
                    signal: CancellationToken::new(),
                    publish,
                    app: None,
                })
                .await
            }
        });
        *host.input_refusal.lock().unwrap() = Some("auth_sheet_focused".into());
        let error = backend
            .type_text("abc", Some("cua:10:20"), None)
            .await
            .unwrap_err();
        assert_eq!(effect(&error), Some(CuaEffect::NotDispatched));
        assert_eq!(code(&error), Some("auth_sheet_focused"));
        assert_eq!(
            error
                .input_pause()
                .and_then(|pause| pause.window_id.as_deref()),
            Some("cua:10:20")
        );
        let state = backend
            .get_state(ComputerStateOptions {
                window_id: Some("cua:10:20".into()),
                ..ComputerStateOptions::default()
            })
            .await
            .unwrap();
        assert_eq!(state.computer_id, "desktop");
        *host.readiness_refusal.lock().unwrap() = Some(json!({
            "effect": "refused",
            "code": "auth_sheet_focused",
            "message": "An authentication sheet has focus.",
            "pid": 10,
            "window_id": 20,
        }));
        let error = backend.check_input_ready("cua:10:20").await.unwrap_err();
        assert_eq!(effect(&error), Some(CuaEffect::NotDispatched));
        assert_eq!(code(&error), Some("auth_sheet_focused"));
        for _ in 0..32 {
            if !prompt_id.lock().unwrap().is_empty() {
                break;
            }
            tokio::task::yield_now().await;
        }
        let id = prompt_id.lock().unwrap().clone();
        assert!(!id.is_empty());
        assert!(gate.respond(thread_id, &id, ApprovalDecision::Accept));
        assert_eq!(prompt.await.unwrap(), Ok(true));
        assert_eq!(host.named("bring_to_front"), 0);
        backend.dispose().await;
    }

    #[tokio::test]
    async fn secure_input_refuses_keyboard_delivery_as_not_dispatched_without_an_input_pause() {
        let (backend, host) = guard_fixture();
        *host.input_refusal.lock().unwrap() = Some("secure_input_active".into());
        let error = backend
            .type_text("abc", Some("cua:10:20"), None)
            .await
            .unwrap_err();
        assert_eq!(effect(&error), Some(CuaEffect::NotDispatched));
        assert_eq!(code(&error), Some("secure_input_active"));
        let error = backend
            .type_text("abc", Some("cua:10:20"), None)
            .await
            .unwrap_err();
        assert!(error.input_pause().is_none());
        assert_eq!(host.named("click"), 0);
        backend.dispose().await;
    }
}
