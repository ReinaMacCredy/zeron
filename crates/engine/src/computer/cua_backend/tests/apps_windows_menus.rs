use serde_json::json;
use zeron_proto::computer::{
    ComputerAccessibilityTreeApp, ComputerAccessibilityTreeWindow, ComputerApp,
    ComputerDeliveryVerification, ComputerJpegMimeType, ComputerLaunchWindowReason,
    ComputerLaunchWindowStatus, ComputerRect, ComputerVerifyStateResult, ComputerVerifyStatus,
    CuaEffect, CuaRestoreStatus,
};

use super::*;
use crate::computer::backend::{
    ComputerAccessibilityTreeSnapshot, ComputerLaunchOptions, ComputerMenuBackendTarget,
};

fn strings(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| (*value).to_owned()).collect()
}

fn hidden(value: bool) -> ComputerLaunchOptions {
    ComputerLaunchOptions {
        hidden: Some(value),
    }
}

#[tokio::test]
async fn launches_by_name_or_bundle_id_without_a_delivery_mode() {
    let f = fixture();
    let launched = f
        .backend
        .launch_app("Calculator", &[], ComputerLaunchOptions::default())
        .await
        .unwrap();
    assert_eq!(launched.app, "Calculator");
    assert_eq!(launched.window, None);
    assert_eq!(f.first_args("launch_app"), json!({ "name": "Calculator" }));
    assert!(f.named("bring_to_front").is_empty());
    assert!(f.named("activate").is_empty());
    f.clear_calls();
    let launched = f
        .backend
        .launch_app(
            "com.apple.Calculator",
            &strings(&["--new-window"]),
            ComputerLaunchOptions::default(),
        )
        .await
        .unwrap();
    assert_eq!(launched.app, "com.apple.Calculator");
    assert_eq!(launched.window, None);
    assert_eq!(
        f.first_args("launch_app"),
        json!({ "bundle_id": "com.apple.Calculator", "additional_arguments": ["--new-window"] })
    );
    let error = f
        .backend
        .launch_app(
            "/Applications/Calculator.app",
            &[],
            ComputerLaunchOptions::default(),
        )
        .await
        .unwrap_err();
    assert_eq!(effect(&error), Some(CuaEffect::NotDispatched));
    assert_eq!(code(&error), Some("unsupported_operation"));
}

#[tokio::test]
async fn preserves_native_launch_readiness_failures_instead_of_polling_a_hidden_window_again() {
    let f = fixture();
    f.on_tool("launch_app", |_| {
        json!({ "structuredContent": {
            "pid": 321,
            "window_status": "no_usable_window",
            "window_reason": "hidden",
        } })
    });
    let launched = f
        .backend
        .launch_app("TextEdit", &[], ComputerLaunchOptions::default())
        .await
        .unwrap();
    assert_eq!(launched.pid, Some(321));
    assert_eq!(launched.window, None);
    assert_eq!(
        launched.window_status,
        Some(ComputerLaunchWindowStatus::NoUsableWindow)
    );
    assert_eq!(
        launched.window_reason,
        Some(ComputerLaunchWindowReason::Hidden)
    );
}

#[tokio::test]
async fn pauses_after_focus_restoration_failure_without_laundering_uncertain_delivery() {
    let f = fixture();
    f.on_tool("press_key", |_| {
        json!({
            "isError": true,
            "structuredContent": {
                "effect": "unverifiable",
                "code": "focus_restore_failed",
                "diagnostics": {
                    "focus_mutation": "without_raise",
                    "restore_status": "unobservable",
                    "error_code": "focus_restore_failed",
                },
            },
        })
    });
    let error = f
        .backend
        .press_key("enter", Some("cua:10:20"), None)
        .await
        .unwrap_err();
    assert_eq!(code(&error), Some("focus_restore_failed"));
    assert_eq!(effect(&error), Some(CuaEffect::DispatchedUnknown));
    let pause = error.input_pause().unwrap();
    assert_eq!(pause.window_id.as_deref(), Some("cua:10:20"));
    assert_eq!(pause.pid, Some(10));
    assert_eq!(
        error
            .cua()
            .unwrap()
            .diagnostics
            .as_ref()
            .unwrap()
            .restore_status,
        Some(CuaRestoreStatus::Unobservable)
    );
    assert_eq!(f.named("press_key").len(), 1);
}

#[tokio::test]
async fn preserves_bounded_cooldown_hints_and_scopes_native_pause_recovery_to_the_app() {
    let f = fixture();
    f.on_tool("press_key", |_| {
        json!({
            "isError": true,
            "structuredContent": {
                "effect": "refused",
                "code": "computer_input_paused",
                "layer": "driver-host",
                "wait_seconds": 0.75,
            },
        })
    });
    let error = f
        .backend
        .press_key("enter", Some("cua:10:20"), None)
        .await
        .unwrap_err();
    assert_eq!(code(&error), Some("computer_input_paused"));
    assert_eq!(error.cua().unwrap().wait_seconds, Some(0.75));
    let pause = error.input_pause().unwrap();
    assert_eq!(pause.window_id.as_deref(), Some("cua:10:20"));
    assert_eq!(pause.pid, Some(10));
    assert_eq!(effect(&error), Some(CuaEffect::NotDispatched));
    assert!(!pause.user_takeover);
}

#[tokio::test]
async fn a_user_input_refusal_marks_the_pause_as_a_takeover() {
    let f = fixture();
    f.on_tool("press_key", |_| {
        json!({
            "isError": true,
            "structuredContent": {
                "effect": "refused",
                "code": "computer_input_paused",
                "cause": "user_input",
                "layer": "driver-host",
            },
        })
    });
    let error = f
        .backend
        .press_key("enter", Some("cua:10:20"), None)
        .await
        .unwrap_err();
    let pause = error.input_pause().unwrap();
    assert!(pause.user_takeover);
    assert_eq!(pause.window_id.as_deref(), Some("cua:10:20"));
}

#[tokio::test]
async fn preserves_the_launched_process_identity_without_claiming_window_readiness() {
    let f = fixture();
    f.on_tool("launch_app", |_| {
        json!({ "structuredContent": {
            "pid": 10,
            "launch_state": { "process_running": true, "window_ready": false },
        } })
    });
    let launched = f
        .backend
        .launch_app(
            "com.apple.Calculator",
            &[],
            ComputerLaunchOptions::default(),
        )
        .await
        .unwrap();
    assert_eq!(launched.pid, Some(10));
    assert_eq!(launched.window, None);
    assert_eq!(
        launched.window_status,
        Some(ComputerLaunchWindowStatus::NotChecked)
    );
}

fn linux_fixture() -> Fixture {
    fixture_with(FixtureOptions {
        host_platform: Some("linux"),
        native_revision: Some(Some(0)),
        ..Default::default()
    })
}

fn probes(f: &Fixture) -> usize {
    f.calls()
        .iter()
        .filter(|call| matches!(call.method, ComputerHostMethod::Probe))
        .count()
}

#[tokio::test]
async fn uses_the_linux_launch_schema_after_discovering_a_remote_hosts_platform() {
    let f = linux_fixture();
    f.backend
        .launch_app(
            "/usr/bin/gnome-calculator",
            &strings(&["--mode=basic"]),
            hidden(false),
        )
        .await
        .unwrap();
    assert!(matches!(f.calls()[0].method, ComputerHostMethod::Probe));
    let last = f.calls().last().unwrap().clone();
    assert_eq!(last.name.as_deref(), Some("launch_app"));
    assert_subset(
        &last.args,
        json!({ "launch_path": "/usr/bin/gnome-calculator", "additional_arguments": ["--mode=basic"] }),
    );
    f.backend
        .launch_app("org.gnome.Calculator.desktop", &[], hidden(false))
        .await
        .unwrap();
    let last = f.calls().last().unwrap().clone();
    assert_eq!(last.name.as_deref(), Some("launch_app"));
    assert_subset(
        &last.args,
        json!({ "name": "org.gnome.Calculator.desktop" }),
    );
    assert!(last.args.get("hidden").is_none());
    assert_eq!(probes(&f), 1);
}

#[tokio::test]
async fn refuses_unsupported_linux_hidden_launches_and_ambiguous_executable_paths_before_dispatch()
{
    let f = linux_fixture();
    for options in [ComputerLaunchOptions::default(), hidden(true)] {
        let error = f
            .backend
            .launch_app("org.gnome.Calculator", &[], options)
            .await
            .unwrap_err();
        assert_eq!(effect(&error), Some(CuaEffect::NotDispatched));
        assert_eq!(code(&error), Some("unsupported_operation"));
    }
    let error = f
        .backend
        .launch_app("/opt/My App/bin/calculator", &[], hidden(false))
        .await
        .unwrap_err();
    assert_eq!(effect(&error), Some(CuaEffect::NotDispatched));
    assert_eq!(code(&error), Some("unsupported_operation"));
    assert_eq!(f.call_count(), 1);
    assert!(matches!(f.calls()[0].method, ComputerHostMethod::Probe));
}

#[tokio::test]
async fn sends_the_hidden_launch_flag_only_when_the_caller_asks_for_it() {
    let f = fixture();
    f.backend
        .launch_app("TextEdit", &[], hidden(true))
        .await
        .unwrap();
    assert_eq!(
        f.first_args("launch_app"),
        json!({ "name": "TextEdit", "hidden": true })
    );
    f.clear_calls();
    f.backend
        .launch_app("TextEdit", &[], hidden(false))
        .await
        .unwrap();
    assert_eq!(f.first_args("launch_app"), json!({ "name": "TextEdit" }));
    f.clear_calls();
    f.backend
        .launch_app("TextEdit", &[], ComputerLaunchOptions::default())
        .await
        .unwrap();
    assert_eq!(f.first_args("launch_app"), json!({ "name": "TextEdit" }));
}

#[tokio::test]
async fn lists_apps_with_pid_name_bundle_id_running_and_active_state() {
    let f = fixture();
    f.on_tool("list_apps", |_| {
        json!({ "structuredContent": { "apps": [
            {
                "pid": 42,
                "name": "TextEdit",
                "bundle_id": "com.apple.TextEdit",
                "active": true,
                "running": true,
                "launch_path": "/System/Applications/TextEdit.app",
                "windows": [{}, {}],
                "last_used": "2026-09-17T00:00:00Z",
            },
            { "pid": 43, "name": "NoBundle", "active": false, "running": true },
            {
                "pid": 0,
                "name": "Chess",
                "bundle_id": "com.apple.Chess",
                "running": false,
                "active": false,
                "launch_path": "/System/Applications/Chess.app",
            },
            { "pid": -1, "name": "bogus" },
            { "pid": 44 },
            {
                "pid": DEV_BUNDLE_PID,
                "name": "Zeron Dev",
                "bundle_id": "sh.zeron.app",
                "running": true,
                "launch_path": "/Applications/Zeron.app",
            },
        ] } })
    });
    let app = |pid: u32, name: &str, running: bool, active: bool| ComputerApp {
        pid,
        name: name.to_owned(),
        bundle_id: None,
        team_id: None,
        running,
        active,
        launch_path: None,
        window_count: None,
        last_used: None,
    };
    assert_eq!(
        f.backend.list_apps().await.unwrap(),
        vec![
            ComputerApp {
                bundle_id: Some("com.apple.TextEdit".into()),
                launch_path: Some("/System/Applications/TextEdit.app".into()),
                window_count: Some(2),
                last_used: Some("2026-09-17T00:00:00Z".into()),
                ..app(42, "TextEdit", true, true)
            },
            app(43, "NoBundle", true, false),
            ComputerApp {
                bundle_id: Some("com.apple.Chess".into()),
                launch_path: Some("/System/Applications/Chess.app".into()),
                ..app(0, "Chess", false, false)
            },
            ComputerApp {
                bundle_id: Some("sh.zeron.app".into()),
                launch_path: Some("/Users/dev/zeron/target/macos-dev7/Zeron Dev 7.app".into()),
                ..app(DEV_BUNDLE_PID, "Zeron Dev", true, false)
            },
        ]
    );
}

#[test]
fn a_running_apps_bundle_is_the_innermost_app_around_its_executable() {
    assert_eq!(
        super::super::app_bundle_of_executable(
            "/Applications/Google Chrome.app/Contents/Frameworks/Helper.app/Contents/MacOS/Helper"
        )
        .as_deref(),
        Some("/Applications/Google Chrome.app/Contents/Frameworks/Helper.app")
    );
    assert_eq!(
        super::super::app_bundle_of_executable("/Users/dev/zeron/target/debug/zeron"),
        None
    );
}

fn frame() -> ComputerRect {
    ComputerRect {
        x: 0.0,
        y: 0.0,
        width: 640.0,
        height: 480.0,
    }
}

#[tokio::test]
async fn verifies_a_moved_window_through_an_independent_list_windows_readback() {
    let f = fixture();
    let knobs = f.knobs.clone();
    f.on_tool("set_window_frame", move |args| {
        knobs.lock().unwrap().bounds = json!({
            "x": args["x"],
            "y": args["y"],
            "width": args["width"],
            "height": args["height"],
        });
        json!({ "structuredContent": {
            "effect": "confirmed",
            "route": "ax_window_frame",
            "delivery": { "mode": "background" },
            "evidence": [{ "kind": "value_readback" }],
        } })
    });
    let result = f
        .backend
        .set_window_frame("cua:10:20", frame())
        .await
        .unwrap();
    assert_eq!(result.window_id.as_deref(), Some("cua:10:20"));
    assert_eq!(
        result.verified,
        Some(ComputerDeliveryVerification::Confirmed)
    );
    assert_eq!(result.effect, Some(CuaEffect::Verified));
    assert_eq!(
        f.first_args("set_window_frame"),
        json!({ "pid": 10, "window_id": 20, "x": 0, "y": 0, "width": 640, "height": 480 })
    );
    f.set_bounds(json!({ "x": -300, "y": 20, "width": 200, "height": 100 }));
    f.on_tool("set_window_frame", |_| {
        json!({ "structuredContent": {
            "effect": "confirmed",
            "route": "ax_window_frame",
            "evidence": [{ "kind": "value_readback" }],
        } })
    });
    let result = f
        .backend
        .set_window_frame("cua:10:20", frame())
        .await
        .unwrap();
    assert_eq!(
        result.verified,
        Some(ComputerDeliveryVerification::Unconfirmed)
    );
    assert_eq!(result.effect, Some(CuaEffect::DispatchedUnknown));
}

fn window_menu() -> ComputerMenuBackendTarget {
    ComputerMenuBackendTarget::Window {
        window_id: "cua:10:20".into(),
    }
}

#[tokio::test]
async fn invokes_a_menu_path_and_preserves_the_status_coded_refusal_dialect() {
    let f = fixture();
    f.on_tool("invoke_menu", |_| {
        json!({ "structuredContent": {
            "effect": "unverifiable",
            "route": "ax_action",
            "delivery": { "mode": "foreground" },
        } })
    });
    let result = f
        .backend
        .invoke_menu(&window_menu(), &strings(&["File", "Save"]))
        .await
        .unwrap();
    assert_eq!(result.window_id.as_deref(), Some("cua:10:20"));
    assert_eq!(
        result.verified,
        Some(ComputerDeliveryVerification::Unverifiable)
    );
    assert_eq!(result.effect, Some(CuaEffect::DispatchedUnknown));
    assert_eq!(
        f.first_args("invoke_menu"),
        json!({ "pid": 10, "window_id": 20, "path": ["File", "Save"] })
    );
    f.on_tool("invoke_menu", |_| {
        json!({
            "isError": true,
            "structuredContent": {
                "status": "refused",
                "refusal": {
                    "code": "menu_path_unavailable",
                    "message": "The menu item is disabled.",
                },
            },
            "content": [{ "type": "text", "text": "The menu item is disabled." }],
        })
    });
    let error = f
        .backend
        .invoke_menu(&window_menu(), &strings(&["Edit", "Undo"]))
        .await
        .unwrap_err();
    assert_eq!(effect(&error), Some(CuaEffect::NotDispatched));
    assert_eq!(code(&error), Some("menu_path_unavailable"));
}

#[tokio::test]
async fn a_menu_action_that_errors_after_dispatch_is_not_reported_as_unsent() {
    let f = fixture();
    f.on_tool("invoke_menu", |_| {
        json!({
            "isError": true,
            "structuredContent": {
                "verified": false,
                "effect": "unverifiable",
                "code": "ax_action_uncertain",
            },
            "content": [{ "type": "text", "text": "invoke_menu: AX actuator returned -25204 after dispatch" }],
        })
    });
    let error = f
        .backend
        .invoke_menu(&window_menu(), &strings(&["File", "Open…"]))
        .await
        .unwrap_err();
    assert_eq!(effect(&error), Some(CuaEffect::DispatchedUnknown));
}

#[tokio::test]
async fn invokes_the_application_level_menu_on_a_windowless_app_with_no_window_id() {
    let f = fixture();
    f.on_tool("invoke_menu", |_| {
        json!({ "structuredContent": {
            "effect": "confirmed",
            "route": "ax_menu_bar",
            "delivery": { "mode": "background" },
        } })
    });
    let app = ComputerMenuBackendTarget::Pid { pid: 44 };
    let path = strings(&["File", "New Window"]);
    let result = f.backend.invoke_menu(&app, &path).await.unwrap();
    assert_eq!(
        result.verified,
        Some(ComputerDeliveryVerification::Confirmed)
    );
    assert_eq!(result.effect, Some(CuaEffect::Verified));
    assert_eq!(
        f.first_args("invoke_menu"),
        json!({ "pid": 44, "path": ["File", "New Window"] })
    );
    let result = f.backend.invoke_menu(&app, &path).await.unwrap();
    assert_eq!(result.window_id, None);
    let error = f
        .backend
        .invoke_menu(
            &ComputerMenuBackendTarget::Pid { pid: 0 },
            &strings(&["File"]),
        )
        .await
        .unwrap_err();
    assert_eq!(code(&error), Some("invalid_arguments"));
}

#[tokio::test]
async fn verifies_window_state_from_the_drivers_per_predicate_outcome() {
    let f = fixture();
    let predicates = json!([{
        "index": 0,
        "status": "satisfied",
        "unknown_reason": null,
        "observed_json": "{}",
    }]);
    let reply = predicates.clone();
    f.on_tool("verify_state", move |_| {
        json!({ "structuredContent": {
            "status": "satisfied",
            "stable": true,
            "samples": 2,
            "predicates": reply,
        } })
    });
    let expect = vec![json!({ "element": { "selector": { "role": "AXButton" }, "exists": true } })];
    assert_eq!(
        f.backend.verify_state("cua:10:20", &expect).await.unwrap(),
        ComputerVerifyStateResult {
            status: ComputerVerifyStatus::Satisfied,
            stable: true,
            samples: 2,
            elapsed_ms: 0,
            predicates: predicates.as_array().unwrap().clone(),
        }
    );
    assert_eq!(
        f.first_args("verify_state"),
        json!({
            "pid": 10,
            "window_id": 20,
            "expect": [{ "element": { "selector": { "role": "AXButton" }, "exists": true } }],
        })
    );
    f.on_tool("verify_state", |_| {
        json!({ "structuredContent": {
            "status": "unsatisfied",
            "stable": true,
            "samples": 1,
            "predicates": [{
                "index": 0,
                "status": "unsatisfied",
                "unknown_reason": null,
                "observed_json": "{}",
            }],
        } })
    });
    let bounds = vec![json!({ "window": { "bounds": { "x": 0 } } })];
    let result = f.backend.verify_state("cua:10:20", &bounds).await.unwrap();
    assert_eq!(result.status, ComputerVerifyStatus::Unsatisfied);
    assert!(result.stable);
    f.on_tool(
        "verify_state",
        |_| json!({ "structuredContent": { "status": "weird" } }),
    );
    let result = f.backend.verify_state("cua:10:20", &bounds).await.unwrap();
    assert_eq!(result.status, ComputerVerifyStatus::Unknown);
}

#[tokio::test]
async fn captures_a_zoom_region_in_window_local_points_scaled_to_pixels() {
    let f = fixture();
    f.on_tool("zoom", |_| {
        json!({
            "structuredContent": { "width": 168, "height": 140, "mime_type": "image/jpeg" },
            "content": [{ "type": "image", "mimeType": "image/jpeg", "data": "/9j/4AAQ" }],
        })
    });
    let zoom = f
        .backend
        .zoom_window(
            "cua:10:20",
            ComputerRect {
                x: 10.0,
                y: 20.0,
                width: 50.0,
                height: 50.0,
            },
        )
        .await
        .unwrap();
    assert_eq!(zoom.mime_type, ComputerJpegMimeType::Jpeg);
    assert_eq!((zoom.width, zoom.height), (168, 140));
    assert_eq!(zoom.window_id.as_deref(), Some("cua:10:20"));
    assert_eq!(zoom.bytes_base64, "/9j/4AAQ");
    assert_eq!(
        f.first_args("zoom"),
        json!({ "pid": 10, "window_id": 20, "x1": 20, "y1": 40, "x2": 120, "y2": 140 })
    );
    let error = f
        .backend
        .zoom_window(
            "cua:10:20",
            ComputerRect {
                x: 150.0,
                y: 0.0,
                width: 100.0,
                height: 50.0,
            },
        )
        .await
        .unwrap_err();
    assert_eq!(effect(&error), Some(CuaEffect::NotDispatched));
    assert_eq!(code(&error), Some("invalid_geometry"));
}

#[tokio::test(start_paused = true)]
async fn reports_a_killed_app_verified_only_once_it_leaves_the_app_list() {
    let f = fixture();
    let apps = Arc::new(Mutex::new(json!([{
        "pid": 10,
        "name": "TextEdit",
        "bundle_id": "com.apple.TextEdit",
        "active": true,
    }])));
    let listed = apps.clone();
    f.on_tool(
        "list_apps",
        move |_| json!({ "structuredContent": { "apps": listed.lock().unwrap().clone() } }),
    );
    let killed = apps.clone();
    f.on_tool("kill_app", move |_| {
        *killed.lock().unwrap() = json!([]);
        json!({ "content": [{ "type": "text", "text": "Sent SIGKILL to pid 10." }] })
    });
    let result = f.backend.kill_app(10).await.unwrap();
    assert_eq!(
        result.verified,
        Some(ComputerDeliveryVerification::Confirmed)
    );
    assert_eq!(result.effect, Some(CuaEffect::Verified));
    assert_eq!(f.first_args("kill_app"), json!({ "pid": 10 }));
    f.on_tool(
        "kill_app",
        |_| json!({ "content": [{ "type": "text", "text": "Sent SIGKILL to pid 10." }] }),
    );
    *apps.lock().unwrap() = json!([{ "pid": 10, "name": "TextEdit", "active": true }]);
    let result = f.backend.kill_app(10).await.unwrap();
    assert_eq!(
        result.verified,
        Some(ComputerDeliveryVerification::Unconfirmed)
    );
    assert_eq!(result.effect, Some(CuaEffect::DispatchedUnknown));
}

#[tokio::test]
async fn minimizes_the_exact_window_and_trusts_only_the_drivers_readback_evidence() {
    let f = fixture();
    f.on_tool("set_window_minimized", |_| {
        json!({ "structuredContent": {
            "effect": "confirmed",
            "route": "ax_window_minimized",
            "delivery": { "mode": "background" },
            "evidence": [{ "kind": "value_readback" }],
        } })
    });
    let result = f
        .backend
        .set_window_minimized("cua:10:20", true)
        .await
        .unwrap();
    assert_eq!(result.window_id.as_deref(), Some("cua:10:20"));
    assert_eq!(
        result.verified,
        Some(ComputerDeliveryVerification::Confirmed)
    );
    assert_eq!(result.effect, Some(CuaEffect::Verified));
    assert_eq!(
        f.first_args("set_window_minimized"),
        json!({ "pid": 10, "window_id": 20, "minimized": true })
    );
    f.on_tool("set_window_minimized", |_| {
        json!({ "structuredContent": { "effect": "confirmed", "route": "ax_window_minimized" } })
    });
    let result = f
        .backend
        .set_window_minimized("cua:10:20", true)
        .await
        .unwrap();
    assert_eq!(
        result.verified,
        Some(ComputerDeliveryVerification::Unverifiable)
    );
    assert_eq!(result.effect, Some(CuaEffect::DispatchedUnknown));
    f.on_tool(
        "set_window_minimized",
        |_| json!({ "structuredContent": { "effect": "unconfirmed" } }),
    );
    let result = f
        .backend
        .set_window_minimized("cua:10:20", false)
        .await
        .unwrap();
    assert_eq!(
        result.verified,
        Some(ComputerDeliveryVerification::Unconfirmed)
    );
    assert_eq!(result.effect, Some(CuaEffect::DispatchedUnknown));
}

#[tokio::test]
async fn hides_and_unhides_an_app_by_pid_on_the_drivers_own_readback() {
    let f = fixture();
    f.on_tool("set_app_visibility", |_| {
        json!({ "structuredContent": {
            "effect": "confirmed",
            "route": "ax_app_visibility",
            "delivery": { "mode": "background" },
            "evidence": [{ "kind": "value_readback" }],
        } })
    });
    let result = f.backend.set_app_visibility(10, true).await.unwrap();
    assert_eq!(
        result.verified,
        Some(ComputerDeliveryVerification::Confirmed)
    );
    assert_eq!(result.effect, Some(CuaEffect::Verified));
    assert_eq!(
        f.first_args("set_app_visibility"),
        json!({ "pid": 10, "hidden": true })
    );
    f.on_tool(
        "set_app_visibility",
        |_| json!({ "structuredContent": { "effect": "suspected_noop" } }),
    );
    let result = f.backend.set_app_visibility(10, false).await.unwrap();
    assert_eq!(
        result.verified,
        Some(ComputerDeliveryVerification::Unconfirmed)
    );
    assert_eq!(result.effect, Some(CuaEffect::DispatchedUnknown));
    let error = f.backend.set_app_visibility(0, true).await.unwrap_err();
    assert_eq!(effect(&error), Some(CuaEffect::NotDispatched));
    assert_eq!(code(&error), Some("invalid_arguments"));
    assert_eq!(f.named("set_app_visibility").len(), 2);
}

fn tree_app(pid: u32, name: &str, bundle_id: Option<&str>) -> ComputerAccessibilityTreeApp {
    ComputerAccessibilityTreeApp {
        pid,
        name: name.into(),
        bundle_id: bundle_id.map(str::to_owned),
    }
}

fn text_edit_windows() -> Vec<ComputerAccessibilityTreeWindow> {
    vec![
        ComputerAccessibilityTreeWindow {
            id: "cua:10:20".into(),
            pid: 10,
            app_name: Some("TextEdit".into()),
            title: "Untitled".into(),
            bounds: Some(ComputerRect {
                x: -300.0,
                y: 20.0,
                width: 200.0,
                height: 100.0,
            }),
            on_screen: Some(true),
            z_index: Some(0),
        },
        ComputerAccessibilityTreeWindow {
            id: "cua:10:21".into(),
            pid: 10,
            app_name: Some("TextEdit".into()),
            title: "Second".into(),
            bounds: None,
            on_screen: Some(false),
            z_index: None,
        },
    ]
}

#[tokio::test]
async fn reads_the_desktop_inventory_and_scopes_it_to_the_scoped_windows_app() {
    let f = fixture();
    f.on_tool("get_accessibility_tree", |_| {
        json!({ "structuredContent": {
            "apps": [
                { "pid": 10, "name": "TextEdit", "bundle_id": "com.apple.TextEdit" },
                { "pid": 20, "name": "Finder" },
                { "pid": -3, "name": "bogus" },
                { "pid": 30 },
            ],
            "windows": [
                {
                    "window_id": 20,
                    "pid": 10,
                    "app_name": "TextEdit",
                    "title": "Untitled",
                    "bounds": { "x": -300, "y": 20, "width": 200, "height": 100 },
                    "is_on_screen": true,
                    "z_index": 0,
                },
                {
                    "window_id": 21,
                    "pid": 10,
                    "app_name": "TextEdit",
                    "title": "Second",
                    "is_on_screen": false,
                },
                { "window_id": 40, "pid": 20, "app_name": "Finder", "title": "" },
                { "window_id": 0, "pid": 10, "title": "bogus" },
            ],
        } })
    });
    let mut windows = text_edit_windows();
    windows.push(ComputerAccessibilityTreeWindow {
        id: "cua:20:40".into(),
        pid: 20,
        app_name: Some("Finder".into()),
        title: String::new(),
        bounds: None,
        on_screen: None,
        z_index: None,
    });
    assert_eq!(
        f.backend.get_accessibility_tree(None).await.unwrap(),
        ComputerAccessibilityTreeSnapshot {
            apps: vec![
                tree_app(10, "TextEdit", Some("com.apple.TextEdit")),
                tree_app(20, "Finder", None),
            ],
            windows,
            truncated: false,
        }
    );
    assert_eq!(
        f.backend
            .get_accessibility_tree(Some("cua:10:20"))
            .await
            .unwrap(),
        ComputerAccessibilityTreeSnapshot {
            apps: vec![tree_app(10, "TextEdit", Some("com.apple.TextEdit"))],
            windows: text_edit_windows(),
            truncated: false,
        }
    );
    assert_eq!(f.first_args("get_accessibility_tree"), json!({}));
}

#[tokio::test]
async fn caps_the_inventory_rows_and_refuses_a_malformed_snapshot() {
    let f = fixture();
    let many_windows: Vec<Value> = (0..600)
        .map(|i| {
            json!({ "window_id": i + 1, "pid": 10, "app_name": "TextEdit", "title": format!("w{i}") })
        })
        .collect();
    f.on_tool("get_accessibility_tree", move |_| {
        json!({ "structuredContent": {
            "apps": [{ "pid": 10, "name": "TextEdit" }],
            "windows": many_windows,
        } })
    });
    let capped = f.backend.get_accessibility_tree(None).await.unwrap();
    assert_eq!(capped.windows.len(), 512);
    assert!(capped.truncated);
    f.on_tool(
        "get_accessibility_tree",
        |_| json!({ "structuredContent": { "windows": [] } }),
    );
    let error = f.backend.get_accessibility_tree(None).await.unwrap_err();
    assert_eq!(effect(&error), Some(CuaEffect::NotDispatched));
    assert_eq!(code(&error), Some("invalid_response"));
    f.on_tool(
        "get_accessibility_tree",
        |_| json!({ "structuredContent": { "apps": [], "windows": [] } }),
    );
    let error = f
        .backend
        .get_accessibility_tree(Some("cua:999:1"))
        .await
        .unwrap_err();
    assert_eq!(effect(&error), Some(CuaEffect::NotDispatched));
    assert_eq!(code(&error), Some("stale_target"));
}

#[tokio::test]
async fn reads_the_cursor_position_in_desktop_points_and_reports_window_containment() {
    let f = fixture();
    f.on_tool(
        "get_cursor_position",
        |_| json!({ "structuredContent": { "x": -200, "y": 60 } }),
    );
    let reading = f.backend.get_cursor_position(None).await.unwrap();
    assert_eq!((reading.x, reading.y), (-200.0, 60.0));
    assert!(!reading.captured_at.is_empty());
    assert_eq!(reading.window_id, None);
    assert_eq!(reading.inside_window, None);
    let reading = f
        .backend
        .get_cursor_position(Some("cua:10:20"))
        .await
        .unwrap();
    assert_eq!((reading.x, reading.y), (-200.0, 60.0));
    assert_eq!(reading.window_id.as_deref(), Some("cua:10:20"));
    assert_eq!(reading.inside_window, Some(true));
    f.on_tool(
        "get_cursor_position",
        |_| json!({ "structuredContent": { "x": 50, "y": 60 } }),
    );
    let reading = f
        .backend
        .get_cursor_position(Some("cua:10:20"))
        .await
        .unwrap();
    assert_eq!(reading.inside_window, Some(false));
    f.on_tool(
        "get_cursor_position",
        |_| json!({ "structuredContent": { "x": "left", "y": 60 } }),
    );
    let error = f.backend.get_cursor_position(None).await.unwrap_err();
    assert_eq!(effect(&error), Some(CuaEffect::NotDispatched));
    assert_eq!(code(&error), Some("invalid_response"));
    assert_eq!(f.first_args("get_cursor_position"), json!({}));
}
