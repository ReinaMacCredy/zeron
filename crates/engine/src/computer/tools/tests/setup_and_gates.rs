use super::*;

use futures::FutureExt;
use tokio::sync::Notify;
use zeron_proto::computer::{
    ComputerAvailability, ComputerBuildSignature, ComputerDeliveryMode, ComputerPermission,
    McpContent,
};

use crate::computer::backend::{
    ComputerBackend, ComputerBackendActionResult, ComputerBackendError,
};
use crate::computer::desktop_operation_queue::desktop_delivery_mode;
use crate::computer::fake_backend::FakeComputerOptIn;
use crate::computer::manager::tests::{deferred, settle};
use crate::computer::tools::guidance::computer_tool_instructions;
use crate::computer::ui_tree_targeting::{ComputerTargetError, ComputerTargetErrorCode};
use crate::computer::visible_use::COMPUTER_FOREGROUND_NOT_AUTHORIZED;

fn subset(actual: &Value, expected: &Value) -> bool {
    match (actual, expected) {
        (Value::Object(actual), Value::Object(expected)) => expected
            .iter()
            .all(|(key, value)| actual.get(key).is_some_and(|found| subset(found, value))),
        (Value::Array(actual), Value::Array(expected)) => {
            actual.len() == expected.len()
                && actual
                    .iter()
                    .zip(expected)
                    .all(|(found, value)| subset(found, value))
        }
        (Value::Number(actual), Value::Number(expected)) => actual.as_f64() == expected.as_f64(),
        _ => actual == expected,
    }
}

#[track_caller]
fn assert_subset(actual: &Value, expected: Value) {
    assert!(
        subset(actual, &expected),
        "expected {expected}\n  within {actual}"
    );
}

fn text(result: &McpToolResult) -> String {
    result.first_text().unwrap_or_default().to_owned()
}

fn kinds(result: &McpToolResult) -> Vec<&'static str> {
    result
        .content
        .iter()
        .map(|part| match part {
            McpContent::Text { .. } => "text",
            McpContent::Image { .. } => "image",
        })
        .collect()
}

fn with_backend(backend: Arc<Scripted>) -> Harness {
    harness_with(HarnessOptions {
        backend,
        ..HarnessOptions::default()
    })
}

fn prompts(h: &Harness) -> Vec<(String, ComputerSetupSignal)> {
    h.seams.setup.lock().unwrap().clone()
}

fn prompt_names(h: &Harness) -> Vec<String> {
    prompts(h).into_iter().map(|(name, _)| name).collect()
}

fn signal(
    missing: Vec<ComputerPermission>,
    blocking: bool,
    build_signature: Option<ComputerBuildSignature>,
) -> ComputerSetupSignal {
    ComputerSetupSignal {
        missing,
        blocking,
        build_signature,
        bundle_id: None,
    }
}

fn cards(h: &Harness) -> Vec<ComputerConsentCard> {
    h.approvals.cards.lock().unwrap().clone()
}

fn inactive_context(h: &Harness) -> ComputerToolContext {
    let seams = ToolSeams::default();
    seams.turn_inactive.store(true, Ordering::SeqCst);
    ComputerToolContext {
        seams: Arc::new(seams),
        ..h.context()
    }
}

fn list_windows_failing_with(error: ComputerError) -> Harness {
    let backend = Scripted::default_fake();
    let error = Arc::new(error);
    backend.hooks().list_windows = Some(Arc::new(move |_, ()| {
        let error = error.clone();
        async move { Err((*error).clone()) }.boxed()
    }));
    with_backend(backend)
}

fn setup_error(message: &str) -> ComputerError {
    ComputerBackendError {
        setup_required: true,
        ..ComputerBackendError::new(message)
    }
    .into()
}

fn permission_required(message: &str) -> ComputerAvailability {
    ComputerAvailability::PermissionRequired {
        missing: vec![ComputerPermission::Accessibility],
        message: message.to_owned(),
        build_signature: ComputerBuildSignature::Signed,
        bundle_id: None,
    }
}

fn spawn_call(
    h: &Harness,
    name: &'static str,
    arguments: Value,
    context: ComputerToolContext,
) -> tokio::task::JoinHandle<McpToolResult> {
    let tools = h.tools.clone();
    tokio::spawn(async move { tools.call(name, args(arguments), &context).await })
}

fn hold_press_key(
    backend: &Scripted,
) -> (Arc<crate::computer::manager::tests::Deferred>, Arc<Notify>) {
    let held = deferred();
    let entered = Arc::new(Notify::new());
    let (hold, enter) = (held.clone(), entered.clone());
    backend.hooks().press_key = Some(Arc::new(move |fake, (key, window, target)| {
        let (hold, enter) = (hold.clone(), enter.clone());
        async move {
            let result = fake
                .press_key(&key, window.as_deref(), target.as_ref())
                .await;
            enter.notify_one();
            hold.wait().await;
            result
        }
        .boxed()
    }));
    (held, entered)
}

#[tokio::test]
async fn prompts_for_setup_when_the_desktop_withheld_an_os_permission() {
    let h = list_windows_failing_with(setup_error("Screen Recording is not granted."));
    let result = h.call("computer_list_windows", json!({})).await;
    assert!(result.is_error, "{result:?}");
    assert_eq!(prompt_names(&h), ["computer_list_windows"]);
    h.dispose().await;
}

#[tokio::test]
async fn does_not_prompt_for_setup_after_a_missing_target_a_backend_fault_or_an_unrelated_failure()
{
    for error in [
        ComputerTargetError::new(
            ComputerTargetErrorCode::NotFound,
            "No control matches that label.",
            Vec::new(),
        )
        .into(),
        ComputerBackendError::new("The click was not delivered.").into(),
        ComputerError::Other("boom".into()),
    ] {
        let h = list_windows_failing_with(error);
        let result = h.call("computer_list_windows", json!({})).await;
        assert!(result.is_error, "{result:?}");
        assert!(prompts(&h).is_empty());
        h.dispose().await;
    }
}

#[tokio::test]
async fn prompts_for_setup_when_a_successful_result_reports_a_permission_state() {
    let h = harness();
    h.backend.fake.set_availability(permission_required(
        "Zeron needs Accessibility to control this Mac. Turn Zeron on in…",
    ));
    h.backend
        .fake
        .set_missing_permissions(vec![ComputerPermission::Accessibility]);
    let result = h.call("computer_list_windows", json!({})).await;
    assert!(!result.is_error, "{result:?}");
    assert_eq!(
        prompts(&h),
        [(
            "computer_list_windows".to_owned(),
            signal(
                vec![ComputerPermission::Accessibility],
                true,
                Some(ComputerBuildSignature::Signed)
            )
        )]
    );
    let text = text(&result);
    assert!(text.contains("Zeron needs Accessibility and has shown the user a setup card"));
    assert!(text.contains("waiting for the user to grant it"));
    assert!(!text.contains("Turn Zeron on in"));
    h.dispose().await;
}

#[tokio::test]
async fn prompts_for_setup_for_a_grant_that_only_blinds_the_desktop() {
    let h = harness();
    h.backend
        .fake
        .set_missing_permissions(vec![ComputerPermission::ScreenRecording]);
    let result = h.call("computer_list_windows", json!({})).await;
    assert!(!result.is_error, "{result:?}");
    assert_eq!(
        prompts(&h),
        [(
            "computer_list_windows".to_owned(),
            signal(vec![ComputerPermission::ScreenRecording], false, None)
        )]
    );
    h.dispose().await;
}

#[tokio::test]
async fn reads_the_missing_grants_fresh_on_every_call_never_from_the_previous_answer() {
    let h = harness();
    h.backend
        .fake
        .set_missing_permissions(vec![ComputerPermission::ScreenRecording]);
    h.call("computer_list_windows", json!({})).await;
    assert_eq!(
        prompts(&h),
        [(
            "computer_list_windows".to_owned(),
            signal(vec![ComputerPermission::ScreenRecording], false, None)
        )]
    );
    h.backend.fake.set_missing_permissions(Vec::new());
    let second = h.call("computer_list_windows", json!({})).await;
    assert!(!second.is_error, "{second:?}");
    assert_eq!(prompts(&h).len(), 1);
    assert!(!text(&second).contains("setup card"));
    h.dispose().await;
}

#[tokio::test]
async fn carries_an_ad_hoc_build_signature_to_the_card_so_it_can_explain_a_stale_grant() {
    let h = harness();
    h.backend
        .fake
        .set_missing_permissions(vec![ComputerPermission::ScreenRecording]);
    h.backend
        .fake
        .set_build_signature(Some(ComputerBuildSignature::Adhoc));
    h.call("computer_list_windows", json!({})).await;
    assert_eq!(
        prompts(&h),
        [(
            "computer_list_windows".to_owned(),
            signal(
                vec![ComputerPermission::ScreenRecording],
                false,
                Some(ComputerBuildSignature::Adhoc)
            )
        )]
    );
    h.dispose().await;
}

#[tokio::test]
async fn carries_the_named_grants_through_a_thrown_refusal() {
    let h = list_windows_failing_with(setup_error("The helper refused: -32000."));
    h.backend
        .fake
        .set_missing_permissions(vec![ComputerPermission::ScreenRecording]);
    h.call("computer_list_windows", json!({})).await;
    let prompts = prompts(&h);
    assert_eq!(prompt_names(&h), ["computer_list_windows"]);
    assert_eq!(prompts[0].1.missing, [ComputerPermission::ScreenRecording]);
    h.dispose().await;
}

#[tokio::test]
async fn says_nothing_about_setup_when_every_grant_is_in_place() {
    let h = harness();
    let result = h.call("computer_list_windows", json!({})).await;
    assert!(prompts(&h).is_empty());
    assert!(!text(&result).contains("setup card"));
    assert!(!text(&result).contains("macOS is asking"));
    h.dispose().await;
}

#[tokio::test]
async fn tells_the_model_to_stop_for_a_blocking_grant_and_to_carry_on_for_a_degrading_one() {
    let degrading = harness();
    degrading
        .backend
        .fake
        .set_missing_permissions(vec![ComputerPermission::ScreenRecording]);
    let text_of = text(&degrading.call("computer_list_windows", json!({})).await);
    assert!(text_of.contains("does not block desktop control"));
    assert!(text_of.contains("Do not stop"));
    assert!(!text_of.contains("Stop desktop automation"));
    degrading.dispose().await;

    let blocking = harness();
    blocking
        .backend
        .fake
        .set_missing_permissions(vec![ComputerPermission::Accessibility]);
    let text_of = text(&blocking.call("computer_list_windows", json!({})).await);
    assert!(text_of.contains("Nothing on the desktop can be driven without it"));
    assert!(text_of.contains("Stop desktop automation"));
    blocking.dispose().await;
}

#[tokio::test]
async fn puts_the_setup_note_on_the_error_path_and_on_a_screenshot_bearing_result() {
    let h = harness();
    h.backend
        .fake
        .set_missing_permissions(vec![ComputerPermission::Accessibility]);
    let state = h
        .call("computer_get_state", json!({ "include_screenshot": true }))
        .await;
    assert_eq!(kinds(&state), ["text", "image"]);
    assert!(
        result_json(&state)["setupRequired"]
            .as_str()
            .unwrap()
            .contains("Zeron needs Accessibility and has shown the user a setup card")
    );
    h.backend.fake.fail_next("captureScreenshot");
    let failed = h
        .call(
            "computer_screenshot",
            json!({ "window_id": "fake-terminal" }),
        )
        .await;
    assert!(failed.is_error, "{failed:?}");
    assert!(text(&failed).contains("setup card"));
    h.dispose().await;
}

#[tokio::test]
async fn raises_the_card_from_a_state_read_that_reports_a_blocking_permission() {
    let h = harness();
    h.backend.fake.set_availability(permission_required(
        "Zeron needs Accessibility to control this Mac.",
    ));
    let result = h.call("computer_get_state", json!({})).await;
    assert_eq!(prompt_names(&h), ["computer_get_state"]);
    assert_subset(
        &result_json(&result),
        json!({ "availability": { "kind": "permission-required", "missing": ["accessibility"] } }),
    );
    h.dispose().await;
}

#[tokio::test]
async fn refreshes_screenshot_coordinates_when_an_unchanged_window_moves() {
    let h = harness();
    h.call(
        "computer_press_key",
        json!({ "key": "enter", "window_id": "fake-calculator" }),
    )
    .await;
    let windows: Vec<_> = h
        .backend
        .fake
        .list_windows()
        .await
        .unwrap()
        .into_iter()
        .map(|mut window| {
            if window.id == "fake-calculator" {
                window.bounds.as_mut().unwrap().x = 600.0;
            }
            window
        })
        .collect();
    h.backend.fake.emit_windows_changed(windows);
    h.call(
        "computer_press_key",
        json!({ "key": "enter", "window_id": "fake-calculator" }),
    )
    .await;
    let click = h
        .call(
            "computer_click",
            json!({ "x": 5, "y": 5, "include_screenshot": false }),
        )
        .await;
    assert!(!click.is_error, "{click:?}");
    assert_subset(
        &h.backend.calls_for("click").last().unwrap().args[0],
        json!({ "x": 605, "y": 125 }),
    );
    h.dispose().await;
}

#[tokio::test]
async fn returns_the_action_window_after_an_intervening_workspace_screenshot() {
    let h = harness();
    h.call(
        "computer_press_key",
        json!({ "key": "enter", "window_id": "fake-calculator" }),
    )
    .await;
    h.see().await;
    let repeat = h
        .call(
            "computer_press_key",
            json!({ "key": "enter", "window_id": "fake-calculator" }),
        )
        .await;
    assert!(repeat.has_image(), "{repeat:?}");
    h.dispose().await;
}

#[tokio::test]
async fn keeps_pane_input_after_the_action_observation_and_refuses_a_queued_call_from_an_ended_turn()
 {
    let h = harness();
    let (held, entered) = hold_press_key(&h.backend);
    let events: Arc<Mutex<Vec<&'static str>>> = Arc::default();
    let log = events.clone();
    h.backend.hooks().capture_screenshot = Some(Arc::new(move |fake, request| {
        log.lock().unwrap().push("capture");
        async move { fake.capture_screenshot(request).await }.boxed()
    }));
    let log = events.clone();
    h.backend.hooks().type_text = Some(Arc::new(move |fake, (text, window, target)| {
        log.lock().unwrap().push("pane input");
        async move {
            fake.type_text(&text, window.as_deref(), target.as_ref())
                .await
        }
        .boxed()
    }));
    let first = spawn_call(
        &h,
        "computer_press_key",
        json!({ "key": "enter" }),
        h.context(),
    );
    entered.notified().await;
    let manager = h.manager.clone();
    let pane_input = tokio::spawn(async move { manager.type_text(None, "human", None).await });
    let next_seams = Arc::new(ToolSeams::default());
    let next = spawn_call(
        &h,
        "computer_press_key",
        json!({ "key": "escape" }),
        ComputerToolContext {
            seams: next_seams.clone(),
            ..h.context()
        },
    );
    next_seams.turn_inactive.store(true, Ordering::SeqCst);
    settle().await;
    assert!(events.lock().unwrap().is_empty());
    held.resolve();
    assert!(!first.await.unwrap().is_error);
    pane_input.await.unwrap().unwrap();
    assert!(next.await.unwrap().is_error);
    assert_eq!(*events.lock().unwrap(), ["capture", "pane input"]);
    assert_eq!(h.backend.calls_for("pressKey").len(), 1);
    h.dispose().await;
}

#[tokio::test]
async fn refuses_a_queued_mutation_flipped_off_mid_queue_without_new_backend_calls() {
    let h = harness();
    let (held, entered) = hold_press_key(&h.backend);
    let first = spawn_call(
        &h,
        "computer_press_key",
        json!({ "key": "enter" }),
        h.context(),
    );
    entered.notified().await;
    let queued = spawn_call(
        &h,
        "computer_press_key",
        json!({ "key": "escape" }),
        h.context(),
    );
    tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    let manager = h.manager.clone();
    let disabling = tokio::spawn(async move { manager.stop_thread(THREAD).await });
    held.resolve();
    let _ = disabling.await.unwrap();
    first.await.unwrap();
    let queued = queued.await.unwrap();
    assert!(queued.is_error, "{queued:?}");
    assert_eq!(h.backend.calls_for("pressKey").len(), 1);
    h.dispose().await;
}

#[tokio::test]
async fn waits_for_a_live_label_and_returns_its_window_screenshot_in_the_same_call() {
    let h = harness();
    let result = h
        .call(
            "computer_wait",
            json!({ "duration_ms": 5_000, "label": "Display", "window_id": "fake-calculator" }),
        )
        .await;
    assert!(!result.is_error, "{result:?}");
    assert_subset(
        &result_json(&result),
        json!({ "status": "ready", "screenshot": { "windowId": "fake-calculator" } }),
    );
    assert_eq!(h.backend.calls_for("getState").len(), 1);
    assert!(h.backend.calls_for("click").is_empty());
    assert!(h.backend.calls_for("raiseWindow").is_empty());
    let invalid = h
        .call(
            "computer_wait",
            json!({ "duration_ms": 0, "label": "Display" }),
        )
        .await;
    assert!(invalid.is_error, "{invalid:?}");
    h.dispose().await;
}

fn settle_observer() -> Harness {
    with_backend(fake_with(FakeComputerBackendOptions {
        wait_for_settle: FakeComputerOptIn::BuiltIn,
        ..FakeComputerBackendOptions::default()
    }))
}

#[tokio::test]
async fn waits_for_a_windows_surface_to_go_quiet_when_settle_is_requested() {
    let h = settle_observer();
    let result = h
        .call(
            "computer_wait",
            json!({ "duration_ms": 5_000, "window_id": "fake-terminal", "settle": true }),
        )
        .await;
    assert!(!result.is_error, "{result:?}");
    assert_subset(
        &result_json(&result),
        json!({ "settled": true, "mode": "observer" }),
    );
    let settles = h.backend.calls_for("waitForSettle");
    assert_eq!(settles.len(), 1);
    assert_subset(&settles[0].args[0], json!({ "windowId": "fake-terminal" }));
    assert!(h.backend.calls_for("click").is_empty());
    assert!(h.backend.calls_for("raiseWindow").is_empty());
    assert!(h.backend.calls_for("captureScreenshot").is_empty());
    h.dispose().await;
}

#[tokio::test]
async fn reports_the_fixed_fallback_when_the_backend_cannot_observe_a_settle() {
    let h = harness();
    let result = h
        .call(
            "computer_wait",
            json!({ "duration_ms": 5_000, "window_id": "fake-terminal", "settle": true }),
        )
        .await;
    assert!(!result.is_error, "{result:?}");
    assert_subset(
        &result_json(&result),
        json!({ "settled": true, "mode": "fixed", "waitedMs": 0 }),
    );
    assert!(h.backend.calls_for("waitForSettle").is_empty());
    h.dispose().await;
}

#[tokio::test]
async fn requires_a_window_for_a_settle_wait_and_refuses_an_unknown_one() {
    let h = settle_observer();
    for arguments in [
        json!({ "duration_ms": 5_000, "settle": true }),
        json!({ "duration_ms": 5_000, "window_id": "no-such-window", "settle": true }),
        json!({ "duration_ms": 5_000, "window_id": "fake-terminal", "label": "OK", "settle": true }),
    ] {
        let result = h.call("computer_wait", arguments.clone()).await;
        assert!(result.is_error, "{arguments}: {result:?}");
    }
    h.dispose().await;
}

#[tokio::test]
async fn can_wait_for_the_next_label_on_an_action_without_replaying_input() {
    let h = harness();
    let result = h
        .call(
            "computer_click",
            json!({ "label": "Display", "window_id": "fake-calculator", "wait_for_label": "Display" }),
        )
        .await;
    assert!(!result.is_error, "{result:?}");
    assert_subset(
        &result_json(&result),
        json!({ "readiness": { "status": "ready" } }),
    );
    assert_eq!(h.backend.calls_for("click").len(), 1);
    let invalid = h
        .call(
            "computer_click",
            json!({ "label": "Display", "wait_for_label": "" }),
        )
        .await;
    assert!(invalid.is_error, "{invalid:?}");
    assert_eq!(h.backend.calls_for("click").len(), 1);
    h.dispose().await;
}

#[tokio::test]
async fn does_not_substitute_a_new_window_from_another_process_for_the_target_screenshot() {
    let backend = Scripted::default_fake();
    let before: Vec<_> = backend
        .fake
        .list_windows()
        .await
        .unwrap()
        .into_iter()
        .map(|mut window| {
            window.pid = Some(10);
            window
        })
        .collect();
    backend.fake.emit_windows_changed(before.clone());
    let h = with_backend(backend);
    h.backend.hooks().press_key = Some(Arc::new(move |fake, _| {
        let mut popup = before[0].clone();
        popup.id = "unrelated-popup".into();
        popup.pid = Some(20);
        popup.stacking_index = Some(0);
        let mut windows = before.clone();
        windows.push(popup);
        fake.emit_windows_changed(windows);
        async { Ok(ComputerBackendActionResult::default()) }.boxed()
    }));
    let result = h
        .call(
            "computer_press_key",
            json!({ "key": "enter", "window_id": "fake-terminal" }),
        )
        .await;
    assert_subset(
        &result_json(&result),
        json!({ "screenshot": { "windowId": "fake-terminal" } }),
    );
    h.dispose().await;
}

#[tokio::test]
async fn limits_large_scrolls_to_overlapping_views_across_screenshot_scale_changes() {
    let h = harness();
    let first = result_json(
        &h.call(
            "computer_screenshot",
            json!({ "window_id": "fake-terminal" }),
        )
        .await,
    );
    let region_height = first["screenshot"]["region"]["height"].as_f64().unwrap();
    for distance in [1500.0_f64, 700.0, -1400.0] {
        let result = result_json(
            &h.call(
                "computer_scroll",
                json!({ "window_id": "fake-terminal", "delta_x": 0, "delta_y": distance }),
            )
            .await,
        );
        let limited = result["scroll"]["limitedTo"]["deltaY"].as_f64().unwrap();
        let requested = result["scroll"]["requested"]["deltaY"].as_f64().unwrap();
        assert_eq!(limited, distance.signum() * region_height / 2.0, "{result}");
        assert!(requested.abs() > limited.abs(), "{result}");
        let sent = h.backend.calls_for("scroll").last().unwrap().args[2]
            .as_f64()
            .unwrap();
        assert!(sent.abs() <= region_height / 2.0);
    }
    h.dispose().await;
}

#[tokio::test]
async fn keeps_a_completed_input_successful_when_conditional_observation_fails() {
    let h = harness();
    h.backend.hooks().click = Some(Arc::new(|fake, (point, window)| {
        async move {
            let result = fake.click(point, window.as_deref(), &[]).await;
            fake.fail_next("getState");
            result
        }
        .boxed()
    }));
    let result = h
        .call(
            "computer_click",
            json!({ "label": "Calculate", "window_id": "fake-calculator", "wait_for_label": "Display" }),
        )
        .await;
    assert!(!result.is_error, "{result:?}");
    assert_subset(
        &result_json(&result),
        json!({ "action": "computer_click", "readiness": { "status": "unavailable" } }),
    );
    assert_eq!(h.backend.calls_for("click").len(), 1);
    h.dispose().await;
}

#[tokio::test]
async fn inherits_an_omitted_scroll_target_from_the_screenshot_rather_than_the_old_keyboard_target()
{
    let h = harness();
    h.manager
        .type_text(None, "human", Some("fake-terminal"))
        .await
        .unwrap();
    let shot = h
        .call(
            "computer_screenshot",
            json!({ "window_id": "fake-calculator" }),
        )
        .await;
    assert!(!shot.is_error, "{shot:?}");
    let result = h
        .call(
            "computer_scroll",
            json!({ "delta_x": 0, "delta_y": 30, "include_screenshot": false }),
        )
        .await;
    assert!(!result.is_error, "{result:?}");
    assert_subset(
        &result_json(&result),
        json!({ "windowId": "fake-calculator" }),
    );
    assert_eq!(
        h.backend.calls_for("focusWindow").last().unwrap().args,
        [json!("fake-calculator")]
    );
    h.dispose().await;
}

#[tokio::test]
async fn allows_human_input_between_conditional_wait_observations_and_stops_polling_when_control_is_revoked()
 {
    let h = harness();
    let reading = Arc::new(Notify::new());
    let read = reading.clone();
    h.backend.hooks().get_state = Some(Arc::new(move |fake, options| {
        let read = read.clone();
        async move {
            let state = fake.get_state(options).await;
            read.notify_one();
            state
        }
        .boxed()
    }));
    let waiting = spawn_call(
        &h,
        "computer_wait",
        json!({
            "duration_ms": 1000,
            "label": "Never exists",
            "window_id": "fake-calculator",
            "include_screenshot": false,
        }),
        h.context(),
    );
    reading.notified().await;
    h.manager
        .type_text(None, "human input", None)
        .await
        .unwrap();
    assert_eq!(h.backend.calls_for("typeText").len(), 1);
    h.manager.stop_thread(THREAD).await.unwrap();
    let reads = h.backend.calls_for("getState").len();
    let waited = waiting.await.unwrap();
    assert!(waited.is_error, "{waited:?}");
    assert_eq!(h.backend.calls_for("getState").len(), reads);
    assert!(h.backend.calls_for("click").is_empty());
    h.dispose().await;
}

#[tokio::test]
async fn an_escape_stop_between_conditional_wait_observations_ends_the_wait() {
    let h = harness();
    let reading = Arc::new(Notify::new());
    let read = reading.clone();
    h.backend.hooks().get_state = Some(Arc::new(move |fake, options| {
        let read = read.clone();
        async move {
            let state = fake.get_state(options).await;
            read.notify_one();
            state
        }
        .boxed()
    }));
    let waiting = spawn_call(
        &h,
        "computer_wait",
        json!({
            "duration_ms": 1000,
            "label": "Never exists",
            "window_id": "fake-calculator",
            "include_screenshot": false,
        }),
        h.context(),
    );
    reading.notified().await;
    h.manager.emergency_stop_input().await.unwrap();
    let reads = h.backend.calls_for("getState").len();
    let waited = waiting.await.unwrap();
    assert!(waited.is_error, "{waited:?}");
    assert_eq!(h.backend.calls_for("getState").len(), reads);
    h.dispose().await;
}

fn refusing_visible_use() -> Harness {
    let h = harness();
    h.decide(ApprovalDecision::Accept);
    h.authorize_visible_use(false);
    h
}

#[tokio::test]
async fn gates_exact_window_and_app_menu_calls_and_batched_menu_steps_as_foreground() {
    let h = refusing_visible_use();
    for target in [
        json!({ "window_id": "fake-calculator" }),
        json!({ "pid": 1_002 }),
    ] {
        let mut arguments = target.clone();
        arguments["path"] = json!(["File"]);
        let result = h.call("computer_invoke_menu", arguments).await;
        assert_subset(
            &result_json(&result),
            json!({ "error": "foreground_not_requested", "effect": "not-dispatched" }),
        );
    }
    let cards = cards(&h);
    assert!(!cards.is_empty());
    assert_eq!(cards[0].tool_name, "computer_invoke_menu");
    assert!(
        cards[0]
            .params_display
            .contains("\"delivery_mode\":\"foreground\""),
        "{}",
        cards[0].params_display
    );
    let batch = h
        .call(
            "computer_run",
            json!({ "steps": [{ "type": "invoke_menu", "app": "Calculator", "path": ["File"] }] }),
        )
        .await;
    assert_subset(
        &result_json(&batch),
        json!({
            "completed": 0,
            "stopped": true,
            "steps": [
                { "ok": false, "error": { "code": "foreground_not_requested", "effect": "not-dispatched" } },
            ],
        }),
    );
    assert!(h.backend.calls_for("invokeMenu").is_empty());
    assert!(h.backend.calls_for("raiseWindow").is_empty());
    assert!(h.backend.calls_for("focusWindow").is_empty());
    h.dispose().await;
}

#[tokio::test]
async fn refuses_activate_without_the_users_task_text_authorization_and_raises_nothing() {
    let h = refusing_visible_use();
    let refused = h
        .call(
            "computer_activate_window",
            json!({ "window_id": "fake-calculator" }),
        )
        .await;
    assert!(refused.is_error, "{refused:?}");
    let payload = result_json(&refused);
    assert_eq!(payload["error"], json!("foreground_not_requested"));
    assert_eq!(payload["effect"], json!("not-dispatched"));
    assert!(
        payload["message"]
            .as_str()
            .unwrap()
            .contains("has not allowed"),
        "{payload}"
    );
    assert!(h.backend.calls_for("raiseWindow").is_empty());
    assert!(h.backend.calls_for("focusWindow").is_empty());
    h.dispose().await;
}

#[tokio::test]
async fn refuses_an_absent_authorization_too_never_raise_is_the_default() {
    assert!(!COMPUTER_FOREGROUND_NOT_AUTHORIZED.user_requested_visible_use);
    let h = harness();
    *h.seams.authorization.lock().unwrap() = COMPUTER_FOREGROUND_NOT_AUTHORIZED;
    let refused = h
        .call(
            "computer_activate_window",
            json!({ "window_id": "fake-calculator" }),
        )
        .await;
    assert!(refused.is_error, "{refused:?}");
    assert!(
        result_json(&refused)
            .to_string()
            .contains("foreground_not_requested")
    );
    assert!(h.backend.calls_for("raiseWindow").is_empty());
    h.dispose().await;
}

#[tokio::test]
async fn refuses_foreground_delivery_without_authorization_and_dispatches_nothing() {
    let h = refusing_visible_use();
    let refused = h
        .call(
            "computer_press_key",
            json!({ "key": "enter", "window_id": "fake-calculator", "delivery_mode": "foreground" }),
        )
        .await;
    assert!(refused.is_error, "{refused:?}");
    assert!(
        result_json(&refused)
            .to_string()
            .contains("foreground_not_requested")
    );
    assert!(h.backend.calls_for("pressKey").is_empty());
    h.dispose().await;
}

#[tokio::test]
async fn refuses_a_runs_activate_step_without_authorization_and_the_run_reports_it() {
    let h = refusing_visible_use();
    let result = h
        .call(
            "computer_run",
            json!({ "steps": [{ "type": "activate_window", "window_id": "fake-calculator" }] }),
        )
        .await;
    let payload = result_json(&result);
    assert_eq!(payload["stopped"], json!(true), "{payload}");
    assert_subset(
        &payload["steps"][0],
        json!({ "ok": false, "error": { "code": "foreground_not_requested", "effect": "not-dispatched" } }),
    );
    assert!(h.backend.calls_for("raiseWindow").is_empty());
    h.dispose().await;
}

#[tokio::test]
async fn launches_macos_apps_in_the_background_without_hiding_or_raising_them() {
    let h = harness_with(HarnessOptions {
        dialect: Some(ComputerAgentDialect::Macos),
        ..HarnessOptions::default()
    });
    h.decide(ApprovalDecision::Accept);
    h.authorize_visible_use(false);
    let result = h
        .call(
            "computer_launch_app",
            json!({ "app": "TextEdit", "hidden": false, "wait_for_window": false }),
        )
        .await;
    assert!(!result.is_error, "{result:?}");
    let batched = h
        .call(
            "computer_run",
            json!({ "steps": [{ "type": "launch_app", "app": "TextEdit", "wait_for_window": false }] }),
        )
        .await;
    assert_subset(&result_json(&batched), json!({ "steps": [{ "ok": true }] }));
    let launches: Vec<Vec<Value>> = h
        .backend
        .calls_for("launchApp")
        .into_iter()
        .map(|call| call.args)
        .collect();
    assert_eq!(
        launches,
        [
            vec![json!("TextEdit"), json!([]), json!({ "hidden": false })],
            vec![json!("TextEdit"), json!([])],
        ]
    );
    assert!(h.backend.calls_for("raiseWindow").is_empty());
    h.dispose().await;
}

#[tokio::test]
async fn allows_the_authorized_raise_and_the_authorization_is_read_per_call() {
    let h = refusing_visible_use();
    let refused = h
        .call(
            "computer_activate_window",
            json!({ "window_id": "fake-calculator" }),
        )
        .await;
    assert!(refused.is_error, "{refused:?}");
    assert!(h.backend.calls_for("raiseWindow").is_empty());
    let reads = h.seams.authorization_reads.load(Ordering::SeqCst);
    assert!(reads >= 1);
    h.authorize_visible_use(true);
    let allowed = h
        .call(
            "computer_activate_window",
            json!({ "window_id": "fake-calculator" }),
        )
        .await;
    assert!(!allowed.is_error, "{allowed:?}");
    assert!(h.seams.authorization_reads.load(Ordering::SeqCst) > reads);
    let raised: Vec<Value> = h
        .backend
        .calls_for("raiseWindow")
        .into_iter()
        .map(|call| call.args[0].clone())
        .collect();
    assert_eq!(raised, [json!("fake-calculator"), json!("fake-terminal")]);
    h.dispose().await;
}

#[tokio::test]
async fn keeps_the_refusal_map_guidance_and_the_foreground_chapter() {
    let notes = computer_tool_instructions();
    assert!(notes.contains("foreground_not_requested"));
    assert!(notes.contains("foreground_user_interaction"));
    let h = harness();
    let chapter = h
        .call("computer_help", json!({ "topic": "foreground" }))
        .await;
    assert!(!chapter.is_error, "{chapter:?}");
    let chapter_text = result_json(&chapter)["text"].as_str().unwrap().to_owned();
    assert!(chapter_text.contains("foreground_not_requested"));
    assert!(chapter_text.contains("foreground_user_interaction"));
    let index = h.call("computer_help", json!({})).await;
    assert!(
        result_json(&index)["topics"]
            .as_str()
            .unwrap()
            .contains("foreground")
    );
    h.dispose().await;
}

#[tokio::test]
async fn runs_a_default_background_activate_in_foreground_scope_and_restores_when_approved() {
    let h = harness();
    h.decide(ApprovalDecision::Accept);
    let modes: Arc<Mutex<Vec<ComputerDeliveryMode>>> = Arc::default();
    let log = modes.clone();
    h.backend.hooks().raise_window = Some(Arc::new(move |fake, window_id| {
        log.lock().unwrap().push(desktop_delivery_mode());
        async move { fake.raise_window(&window_id).await }.boxed()
    }));
    let result = h
        .call(
            "computer_activate_window",
            json!({ "window_id": "fake-calculator" }),
        )
        .await;
    assert!(!result.is_error, "{result:?}");
    let cards = cards(&h);
    assert_eq!(cards.len(), 1);
    assert_eq!(cards[0].tool_name, "computer_activate_window");
    assert!(
        cards[0]
            .params_display
            .contains("\"delivery_mode\":\"foreground\""),
        "{}",
        cards[0].params_display
    );
    assert_eq!(
        *modes.lock().unwrap(),
        [
            ComputerDeliveryMode::Foreground,
            ComputerDeliveryMode::Foreground
        ]
    );
    let raised: Vec<Value> = h
        .backend
        .calls_for("raiseWindow")
        .into_iter()
        .map(|call| call.args[0].clone())
        .collect();
    assert_eq!(raised, [json!("fake-calculator"), json!("fake-terminal")]);
    assert_subset(
        &result_json(&result),
        json!({ "action": "computer_activate_window", "windowId": "fake-calculator" }),
    );
    h.dispose().await;
}

#[tokio::test]
async fn touches_nothing_when_approval_refuses_the_activate() {
    let h = harness();
    h.decide(ApprovalDecision::Decline);
    let refused = h
        .call(
            "computer_activate_window",
            json!({ "window_id": "fake-calculator" }),
        )
        .await;
    assert!(refused.is_error, "{refused:?}");
    assert!(h.backend.calls_for("raiseWindow").is_empty());
    assert!(h.backend.calls_for("focusWindow").is_empty());
    h.dispose().await;
}

#[tokio::test]
async fn preserves_specialist_reads_and_image_results_through_inspect() {
    let h = harness();
    h.decide(ApprovalDecision::Accept);
    let inspector = h.tools.entry("computer_inspect").unwrap();
    assert!(!inspector.discovery_only);
    assert_eq!(
        inspector.definition.annotations.as_ref().unwrap()["readOnlyHint"],
        json!(false)
    );
    for (tool, arguments) in [
        ("computer_read_clipboard", json!({})),
        (
            "computer_get_accessibility_tree",
            json!({ "window_id": "fake-calculator" }),
        ),
        (
            "computer_get_cursor_position",
            json!({ "window_id": "fake-calculator" }),
        ),
        (
            "computer_zoom",
            json!({ "window_id": "fake-calculator", "x": 0, "y": 0, "width": 40, "height": 40 }),
        ),
    ] {
        let help = result_json(&h.call("computer_help", json!({ "tool": tool })).await);
        let route = help["inspection"]["name"].as_str().unwrap().to_owned();
        let result = h
            .call(
                &route,
                json!({ "tool": help["inspection"]["tool"], "arguments": arguments }),
            )
            .await;
        assert!(!result.is_error, "{tool}: {result:?}");
        if tool == "computer_zoom" {
            assert!(result.content.iter().any(|part| matches!(
                part,
                McpContent::Image { mime_type, .. } if mime_type == "image/jpeg"
            )));
            let payload = result_json(&result);
            assert_subset(&payload["zoom"], json!({ "windowId": "fake-calculator" }));
            assert!(!payload.to_string().contains("bytesBase64"));
            assert!(!payload.to_string().contains("screenshotId"));
        }
    }
    let cards = cards(&h);
    assert_eq!(cards.len(), 1);
    assert_eq!(cards[0].tool_name, "computer_read_clipboard");
    for method in [
        "readClipboard",
        "getAccessibilityTree",
        "getCursorPosition",
        "zoomWindow",
    ] {
        assert_eq!(h.backend.calls_for(method).len(), 1, "{method}");
    }
    assert!(h.backend.calls_for("captureScreenshot").is_empty());
    h.dispose().await;
}

#[tokio::test]
async fn refuses_unknown_routes_invalid_schemas_and_extra_fields_before_any_backend_call() {
    let h = harness();
    h.decide(ApprovalDecision::Accept);
    for arguments in [
        json!({ "tool": "computer_click", "arguments": { "x": 2, "y": 3 } }),
        json!({ "tool": "computer_inspect", "arguments": { "tool": "computer_read_clipboard" } }),
        json!({ "tool": "computer_future" }),
        json!({ "tool": "mcp__zeron__computer_read_clipboard" }),
        json!({ "tool": "computer_read_clipboard", "arguments": { "text": "private" } }),
        json!({ "tool": "computer_read_clipboard", "arguments": [] }),
        json!({ "tool": "computer_read_clipboard", "arguments": null }),
        json!({ "tool": "computer_get_cursor_position", "arguments": { "window_id": 42 } }),
        json!({ "tool": "computer_zoom", "arguments": { "window_id": "fake-calculator" } }),
        json!({
            "tool": "computer_zoom",
            "arguments": { "window_id": "fake-calculator", "x": "1", "y": 0, "width": 4, "height": 4 },
        }),
        json!({
            "tool": "computer_zoom",
            "arguments": { "window_id": "fake-calculator", "x": 1, "y": 0, "width": f64::INFINITY, "height": 4 },
        }),
        json!({ "tool": "computer_get_accessibility_tree", "delivery_mode": "foreground" }),
    ] {
        let result = h.call("computer_inspect", arguments.clone()).await;
        assert!(result.is_error, "{arguments}: {result:?}");
    }
    assert!(
        h.backend.fake.calls().is_empty(),
        "{:?}",
        h.backend.methods()
    );
    assert!(cards(&h).is_empty());
    h.dispose().await;
}

#[tokio::test]
async fn retains_clipboard_refusal_and_dead_turn_checks_before_dispatch() {
    let h = harness();
    h.decide(ApprovalDecision::Decline);
    let refused = h
        .call(
            "computer_inspect",
            json!({ "tool": "computer_read_clipboard" }),
        )
        .await;
    assert!(refused.is_error, "{refused:?}");
    assert_eq!(cards(&h).len(), 1);
    let result = h
        .call_in(
            "computer_inspect",
            json!({ "tool": "computer_get_cursor_position" }),
            &inactive_context(&h),
        )
        .await;
    assert!(result.is_error, "{result:?}");
    assert!(h.backend.calls_for("readClipboard").is_empty());
    assert!(h.backend.calls_for("getCursorPosition").is_empty());
    h.dispose().await;
}

#[tokio::test]
async fn passes_cancellation_to_a_pending_canonical_clipboard_approval() {
    let h = harness();
    *h.approvals.mode.lock().unwrap() = Some(AccessMode::Supervised);
    let context = h.context();
    let signal = context.signal.clone();
    let pending = spawn_call(
        &h,
        "computer_inspect",
        json!({ "tool": "computer_read_clipboard" }),
        context,
    );
    while cards(&h).is_empty() {
        tokio::task::yield_now().await;
    }
    signal.cancel();
    let outcome = tokio::time::timeout(std::time::Duration::from_secs(5), pending)
        .await
        .expect("the cancelled approval settles")
        .unwrap();
    assert!(outcome.is_error, "{outcome:?}");
    assert!(h.backend.calls_for("readClipboard").is_empty());
    h.dispose().await;
}
