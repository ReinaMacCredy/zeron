use super::*;

use std::path::PathBuf;

use futures::FutureExt;
use zeron_proto::computer::{
    COMPUTER_SELECT_TEXT_RANGE_MAX, COMPUTER_TEXT_MAX_LENGTH, COMPUTER_WAIT_MAX_MS, ComputerPoint,
    ComputerRect, ComputerUiNode, ComputerWindow, CuaEffect, McpContent,
    parse_cua_action_diagnostics,
};

use crate::computer::audit_log::COMPUTER_AUDIT_FILE_NAME;
use crate::computer::backend::{
    ComputerBackend, ComputerBackendActionResult, ComputerBackendMethod, CuaActionError,
    DEFAULT_COMPUTER_CAPTURE_MAX_DIMENSION, MAX_COMPUTER_CLIPBOARD_BYTES,
};
use crate::computer::manager::tests::{rect, window};
use crate::computer::tools::computer_tool_requires_approval;
use crate::computer::tools::guidance::computer_tool_instructions;

fn built(backend: Arc<Scripted>, env: ComputerManagerEnv, data_dir: Option<PathBuf>) -> Harness {
    let gate = ComputerApprovalGate::default();
    let approvals = Arc::new(Approvals {
        mode: Mutex::new(Some(PermissionMode::Full)),
        decide: Mutex::new(None),
        gate: gate.clone(),
        cards: Mutex::default(),
        turn_inactive: AtomicBool::new(false),
    });
    let mut options = ComputerManagerOptions::new(backend.clone());
    options.approval_gate = gate;
    options.seams = approvals.clone();
    options.action_settle_ms = Some(0);
    options.env = env;
    options.data_dir = data_dir;
    let manager = ComputerManager::new(options);
    Harness {
        tools: ComputerTools::new(manager.clone(), Vec::new()),
        manager,
        backend,
        approvals,
        seams: Arc::new(ToolSeams::default()),
    }
}

fn with_backend(backend: Arc<Scripted>) -> Harness {
    harness_with(HarnessOptions {
        backend,
        ..HarnessOptions::default()
    })
}

fn macos() -> Harness {
    harness_with(HarnessOptions {
        dialect: Some(ComputerAgentDialect::Macos),
        ..HarnessOptions::default()
    })
}

async fn audit_rows(h: &Harness, dir: &tempfile::TempDir) -> Vec<Value> {
    h.manager.dispose().await;
    std::fs::read_to_string(dir.path().join(COMPUTER_AUDIT_FILE_NAME))
        .unwrap_or_default()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

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

fn args_of(h: &Harness, method: &str) -> Vec<Value> {
    h.backend
        .calls_for(method)
        .into_iter()
        .map(|call| Value::Array(call.args))
        .collect()
}

fn definition(h: &Harness, name: &str) -> zeron_proto::computer::McpToolDefinition {
    h.tools.entry(name).unwrap().definition
}

fn schema_text(h: &Harness, name: &str) -> String {
    definition(h, name).input_schema.to_string()
}

fn node(
    role: &str,
    label: Option<&str>,
    frame: ComputerRect,
    activation: Option<ComputerPoint>,
    window_id: Option<&str>,
    children: Vec<ComputerUiNode>,
) -> ComputerUiNode {
    ComputerUiNode {
        role: role.to_owned(),
        label: label.map(str::to_owned),
        value: None,
        description: None,
        frame,
        activation_point: activation,
        on_screen: true,
        window_id: window_id.map(str::to_owned),
        accessibility_root: None,
        node_path: None,
        editable: None,
        truncated: None,
        children,
    }
}

fn refusing_key(
    message: &str,
    effect: CuaEffect,
    code: &str,
    calls: Arc<AtomicUsize>,
) -> Arc<Scripted> {
    let backend = Scripted::default_fake();
    let message = message.to_owned();
    let code = code.to_owned();
    backend.hooks().press_key = Some(Arc::new(move |_, _| {
        calls.fetch_add(1, Ordering::SeqCst);
        let error = CuaActionError::new(message.clone(), effect).code(code.clone());
        async move { Err(error.into()) }.boxed()
    }));
    backend
}

#[tokio::test]
async fn refuses_a_fourth_consecutive_unchanged_scroll_on_the_same_window() {
    let h = harness();
    h.see().await;
    let first_args = json!({ "window_id": "fake-calculator", "delta_x": 0, "delta_y": 20 });
    let other_args = json!({ "window_id": "fake-calculator", "delta_x": 0, "delta_y": 40 });

    let first = h.call("computer_scroll", first_args.clone()).await;
    assert!(!first.is_error, "{first:?}");
    let payload = result_json(&first);
    assert_subset(
        &payload,
        json!({
            "action": "computer_scroll",
            "scroll": { "traveledY": 0 },
            "scrollObservation": {
                "status": "no-visible-movement",
                "code": "scroll_noop",
                "measuredDeltaY": 0,
            },
        }),
    );
    assert!(
        payload["scrollObservation"]["message"]
            .as_str()
            .unwrap()
            .contains("dropped delivery")
    );

    let second = h.call("computer_scroll", other_args.clone()).await;
    assert!(!second.is_error, "{second:?}");
    assert_subset(
        &result_json(&second),
        json!({ "screenshotUnchanged": true }),
    );

    let third = h.call("computer_scroll", first_args.clone()).await;
    assert!(!third.is_error, "{third:?}");
    assert_eq!(h.backend.calls_for("scroll").len(), 3);

    let fourth = h.call("computer_scroll", other_args.clone()).await;
    assert!(fourth.is_error);
    assert!(text(&fourth).contains("computer_get_state"));
    assert!(text(&fourth).contains("label_contains"));
    assert_eq!(h.backend.calls_for("scroll").len(), 3);

    h.call("computer_get_state", json!({ "include_screenshot": false }))
        .await;
    h.backend
        .fake
        .queue_screenshots(["changed-before", "changed-after"]);
    let changed = h.call("computer_scroll", first_args).await;
    assert!(!changed.is_error, "{changed:?}");
    assert_eq!(kinds(&changed), ["text", "image"]);

    let after = h.call("computer_scroll", other_args).await;
    assert!(!after.is_error, "{after:?}");
    assert!(h.backend.calls_for("scroll").len() > 3);
    h.dispose().await;
}

#[tokio::test]
async fn still_refuses_a_fourth_unchanged_scroll_with_conditional_settle_set() {
    let h = built(
        Scripted::default_fake(),
        ComputerManagerEnv {
            conditional_settle: true,
            ..ComputerManagerEnv::default()
        },
        None,
    );
    h.see().await;
    let first_args = json!({ "window_id": "fake-calculator", "delta_x": 0, "delta_y": 20 });
    let other_args = json!({ "window_id": "fake-calculator", "delta_x": 0, "delta_y": 40 });
    for attempt in 0..3 {
        let arguments = if attempt % 2 == 0 {
            &first_args
        } else {
            &other_args
        };
        let result = h.call("computer_scroll", arguments.clone()).await;
        assert!(!result.is_error, "{result:?}");
    }
    let fourth = h.call("computer_scroll", first_args).await;
    assert!(fourth.is_error);
    assert!(text(&fourth).contains("no visible movement"), "{fourth:?}");
    assert_eq!(h.backend.calls_for("scroll").len(), 3);
    h.dispose().await;
}

#[tokio::test]
async fn refuses_the_third_identical_mutating_call_that_observed_nothing() {
    let h = harness();
    let arguments = json!({ "key": "enter", "include_screenshot": false });
    let first = h.call("computer_press_key", arguments.clone()).await;
    assert!(!first.is_error, "{first:?}");
    let second = h.call("computer_press_key", arguments.clone()).await;
    assert!(!second.is_error, "{second:?}");
    let third = h.call("computer_press_key", arguments).await;
    assert!(third.is_error);
    assert_subset(
        &result_json(&third),
        json!({ "error": { "code": "repeated_unverified_action" } }),
    );
    assert_eq!(h.backend.calls_for("pressKey").len(), 2);
    h.dispose().await;
}

#[tokio::test]
async fn does_not_turn_lease_refusals_into_repeated_input_or_block_a_later_corrected_call() {
    let h = harness();
    h.manager
        .press_key(Some("owner"), "tab", None, None)
        .await
        .unwrap();
    let arguments = json!({ "key": "enter", "include_screenshot": false });
    for _ in 0..4 {
        let refused = h.call("computer_press_key", arguments.clone()).await;
        assert_subset(
            &result_json(&refused),
            json!({ "error": { "code": "computer_controlled_by_other_thread" } }),
        );
    }
    assert_eq!(h.backend.calls_for("pressKey").len(), 1);
    h.manager
        .release_desktop_control("owner", None)
        .await
        .unwrap();
    let corrected = h.call("computer_press_key", arguments).await;
    assert!(!corrected.is_error, "{corrected:?}");
    assert_eq!(h.backend.calls_for("pressKey").len(), 2);
    h.dispose().await;
}

#[tokio::test]
async fn bounds_repeated_native_refusals_without_claiming_any_input_was_dispatched() {
    let calls = Arc::new(AtomicUsize::new(0));
    let h = with_backend(refusing_key(
        "No input was sent.",
        CuaEffect::NotDispatched,
        "same_pid_keyboard_ambiguity",
        calls.clone(),
    ));
    let arguments = json!({ "key": "enter", "include_screenshot": false });
    for _ in 0..3 {
        let refused = h.call("computer_press_key", arguments.clone()).await;
        assert_subset(
            &result_json(&refused),
            json!({ "error": "same_pid_keyboard_ambiguity", "effect": "not-dispatched" }),
        );
    }
    let bounded = h.call("computer_press_key", arguments).await;
    assert_subset(
        &result_json(&bounded),
        json!({
            "error": {
                "code": "repeated_computer_refusal",
                "effect": "not-dispatched",
                "previousInputMayHaveTakenEffect": false,
            },
        }),
    );
    assert_eq!(calls.load(Ordering::SeqCst), 3);
    h.dispose().await;
}

#[tokio::test]
async fn bounds_batches_that_repeatedly_stop_at_the_same_refused_first_action() {
    let calls = Arc::new(AtomicUsize::new(0));
    let h = with_backend(refusing_key(
        "No input was sent.",
        CuaEffect::NotDispatched,
        "same_pid_keyboard_ambiguity",
        calls.clone(),
    ));
    let arguments = json!({
        "steps": [{ "type": "press_key", "key": "enter", "window_id": "fake-calculator" }],
        "include_screenshot": false,
    });
    for _ in 0..3 {
        let run = h.call("computer_run", arguments.clone()).await;
        assert_subset(&result_json(&run), json!({ "completed": 0 }));
    }
    let bounded = h.call("computer_run", arguments).await;
    assert_subset(
        &result_json(&bounded),
        json!({
            "error": { "code": "repeated_computer_refusal", "previousInputMayHaveTakenEffect": false },
        }),
    );
    assert_eq!(calls.load(Ordering::SeqCst), 3);
    h.dispose().await;
}

#[tokio::test]
async fn preserves_verified_delivery_and_window_identity_when_the_result_includes_an_image() {
    let dir = tempfile::tempdir().unwrap();
    let backend = Scripted::default_fake();
    backend.hooks().press_key = Some(Arc::new(|_, _| {
        async {
            Ok(ComputerBackendActionResult {
                effect: Some(CuaEffect::Verified),
                verified: Some(zeron_proto::computer::ComputerDeliveryVerification::Confirmed),
                delivery_path: Some("semantic".into()),
                window_id: Some("fake-calculator".into()),
                ..ComputerBackendActionResult::default()
            })
        }
        .boxed()
    }));
    let h = built(
        backend,
        ComputerManagerEnv::default(),
        Some(dir.path().to_path_buf()),
    );
    let result = h
        .call(
            "computer_press_key",
            json!({ "key": "enter", "window_id": "fake-calculator" }),
        )
        .await;
    assert!(result.has_image(), "{result:?}");
    let rows = audit_rows(&h, &dir).await;
    assert!(
        rows.iter().any(|row| subset(
            row,
            &json!({
                "tool": "computer_press_key",
                "effect": "verified",
                "target": { "windowId": "fake-calculator" },
                "diagnostics": { "observation": "fresh-frame" },
            })
        ) && row["diagnostics"] == json!({ "observation": "fresh-frame" })),
        "{rows:?}"
    );
}

#[tokio::test]
async fn retains_uncertain_native_errors_and_their_diagnostics_in_the_audit() {
    let dir = tempfile::tempdir().unwrap();
    let diagnostics = json!({ "delivery_path": "ax", "actuator": "ax_press", "ax_error": -25202 });
    let parsed = parse_cua_action_diagnostics(&json!({ "diagnostics": diagnostics })).unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
    let backend = Scripted::default_fake();
    let counter = calls.clone();
    backend.hooks().press_key = Some(Arc::new(move |_, _| {
        counter.fetch_add(1, Ordering::SeqCst);
        let mut error = CuaActionError::new("Native action failed.", CuaEffect::DispatchedUnknown)
            .code("cua_action_failed");
        error.diagnostics = Some(parsed.clone());
        async move { Err(error.into()) }.boxed()
    }));
    let h = built(
        backend,
        ComputerManagerEnv::default(),
        Some(dir.path().to_path_buf()),
    );
    let arguments = json!({ "key": "enter", "include_screenshot": false });
    let first = h.call("computer_press_key", arguments.clone()).await;
    assert_subset(
        &result_json(&first),
        json!({
            "error": "cua_action_failed",
            "effect": "dispatched-unknown",
            "diagnostics": diagnostics.clone(),
        }),
    );
    h.call("computer_press_key", arguments.clone()).await;
    let refused = h.call("computer_press_key", arguments).await;
    assert_subset(
        &result_json(&refused),
        json!({ "error": { "code": "repeated_unverified_action" } }),
    );
    assert_eq!(calls.load(Ordering::SeqCst), 2);
    let rows = audit_rows(&h, &dir).await;
    assert!(
        rows.iter().any(|row| subset(
            row,
            &json!({
                "effect": "dispatched-unknown",
                "code": "cua_action_failed",
                "diagnostics": diagnostics.clone(),
            })
        )),
        "{rows:?}"
    );
}

#[tokio::test]
async fn does_not_mistake_an_uncertain_first_batch_step_for_a_pre_dispatch_refusal() {
    let dir = tempfile::tempdir().unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
    let backend = refusing_key(
        "Input may have been sent.",
        CuaEffect::DispatchedUnknown,
        "cua_action_failed",
        calls.clone(),
    );
    let h = built(
        backend,
        ComputerManagerEnv::default(),
        Some(dir.path().to_path_buf()),
    );
    let arguments = json!({
        "steps": [{ "type": "press_key", "key": "enter", "window_id": "fake-calculator" }],
    });
    let first = h.call("computer_run", arguments.clone()).await;
    assert_subset(&result_json(&first), json!({ "completed": 0 }));
    assert_subset(
        &result_json(&first)["steps"],
        json!([{ "ok": false, "error": { "effect": "dispatched-unknown" } }]),
    );
    h.call("computer_run", arguments.clone()).await;
    let refused = h.call("computer_run", arguments).await;
    assert_subset(
        &result_json(&refused),
        json!({ "error": { "code": "repeated_unverified_action" } }),
    );
    assert_eq!(calls.load(Ordering::SeqCst), 2);
    let rows = audit_rows(&h, &dir).await;
    assert!(
        rows.iter().any(|row| subset(
            row,
            &json!({ "tool": "computer_run", "effect": "dispatched-unknown" })
        )),
        "{rows:?}"
    );
}

#[tokio::test]
async fn refuses_the_repeat_before_the_approval_prompt_and_before_dispatch() {
    let h = harness();
    h.decide(ApprovalDecision::Accept);
    let arguments =
        json!({ "key": "enter", "window_id": "fake-terminal", "include_screenshot": false });
    assert!(
        !h.call("computer_press_key", arguments.clone())
            .await
            .is_error
    );
    assert!(
        !h.call("computer_press_key", arguments.clone())
            .await
            .is_error
    );
    assert_eq!(h.approvals.cards.lock().unwrap().len(), 1);
    h.approvals.turn_inactive.store(true, Ordering::SeqCst);
    let refused = h.call("computer_press_key", arguments).await;
    assert!(refused.is_error);
    assert_subset(
        &result_json(&refused),
        json!({ "error": { "code": "repeated_unverified_action" } }),
    );
    assert_eq!(h.approvals.cards.lock().unwrap().len(), 1);
    assert_eq!(h.backend.calls_for("pressKey").len(), 2);
    h.dispose().await;
}

#[tokio::test]
async fn treats_screenshot_only_argument_changes_as_the_same_action() {
    let h = harness();
    let first = h
        .call(
            "computer_press_key",
            json!({ "key": "enter", "include_screenshot": false }),
        )
        .await;
    let second = h
        .call(
            "computer_press_key",
            json!({ "key": "enter", "include_screenshot": true }),
        )
        .await;
    let third = h
        .call(
            "computer_press_key",
            json!({ "key": "enter", "include_screenshot": false }),
        )
        .await;
    assert!(!first.is_error, "{first:?}");
    assert!(!second.is_error, "{second:?}");
    assert!(third.is_error);
    assert_subset(
        &result_json(&third),
        json!({ "error": { "code": "repeated_unverified_action" } }),
    );
    assert_eq!(h.backend.calls_for("pressKey").len(), 2);
    h.dispose().await;
}

#[tokio::test]
async fn clears_the_streak_when_a_call_reports_verified_so_the_refusal_comes_later() {
    let h = harness();
    let arguments =
        json!({ "window_id": "fake-calculator", "x": 10, "y": 10, "width": 400, "height": 300 });
    h.backend.fake.set_frame_applies(false);
    assert!(
        !h.call("computer_set_window_frame", arguments.clone())
            .await
            .is_error
    );
    h.backend.fake.set_frame_applies(true);
    assert!(
        !h.call("computer_set_window_frame", arguments.clone())
            .await
            .is_error
    );
    h.backend.fake.set_frame_applies(false);
    assert!(
        !h.call("computer_set_window_frame", arguments.clone())
            .await
            .is_error
    );
    assert!(
        !h.call("computer_set_window_frame", arguments.clone())
            .await
            .is_error
    );
    let refused = h.call("computer_set_window_frame", arguments).await;
    assert!(refused.is_error);
    assert_subset(
        &result_json(&refused),
        json!({ "error": { "code": "repeated_unverified_action" } }),
    );
    assert_eq!(h.backend.calls_for("setWindowFrame").len(), 4);
    h.dispose().await;
}

#[tokio::test]
async fn clears_the_streak_when_a_different_action_intervenes() {
    let h = harness();
    let arguments = json!({ "key": "enter", "include_screenshot": false });
    h.call("computer_press_key", arguments.clone()).await;
    h.call("computer_press_key", arguments.clone()).await;
    let other = h
        .call(
            "computer_press_key",
            json!({ "key": "tab", "include_screenshot": false }),
        )
        .await;
    assert!(!other.is_error, "{other:?}");
    let resumed = h.call("computer_press_key", arguments).await;
    assert!(!resumed.is_error, "{resumed:?}");
    assert_eq!(h.backend.calls_for("pressKey").len(), 4);
    h.dispose().await;
}

#[tokio::test]
async fn does_not_guard_reads_repeated_get_state_calls_still_answer() {
    let h = harness();
    for _ in 0..4 {
        let result = h
            .call("computer_get_state", json!({ "include_screenshot": false }))
            .await;
        assert!(!result.is_error, "{result:?}");
    }
    for _ in 0..4 {
        let result = h.call("computer_read_clipboard", json!({})).await;
        assert!(!result.is_error, "{result:?}");
    }
    assert_eq!(h.backend.calls_for("readClipboard").len(), 4);
    h.dispose().await;
}

#[tokio::test]
async fn tells_the_model_the_observation_is_downscaled_and_what_unchanged_means() {
    let h = harness();
    assert!(schema_text(&h, "computer_screenshot").contains(&format!(
        "capped at {DEFAULT_COMPUTER_CAPTURE_MAX_DIMENSION}"
    )));
    assert!(
        definition(&h, "computer_click")
            .description
            .contains("Returns a screenshot of the affected window")
    );
    assert!(schema_text(&h, "computer_click").contains("Post-action screenshot, default true"));
    h.dispose().await;
}

#[tokio::test]
async fn keeps_a_successful_action_result_when_the_post_action_capture_fails() {
    let h = harness();
    h.backend.fake.fail_next("captureScreenshot");
    let result = h
        .call("computer_press_key", json!({ "key": "enter" }))
        .await;
    assert!(!result.is_error, "{result:?}");
    assert_eq!(kinds(&result), ["text"]);
    assert_subset(
        &result_json(&result),
        json!({ "action": "computer_press_key" }),
    );
    h.dispose().await;
}

#[tokio::test]
async fn tells_the_model_every_observed_action_already_carries_its_screenshot() {
    let h = harness();
    for name in [
        "computer_click",
        "computer_move_cursor",
        "computer_drag",
        "computer_scroll",
        "computer_type_text",
        "computer_press_key",
        "computer_set_value",
        "computer_perform_action",
        "computer_select_text",
    ] {
        assert!(
            definition(&h, name)
                .description
                .contains("Returns a screenshot of the affected window"),
            "{name}"
        );
        let schema = schema_text(&h, name);
        assert!(schema.contains("include_screenshot"), "{name}");
        assert!(
            schema.contains("Post-action screenshot, default true"),
            "{name}"
        );
    }
    for name in ["computer_launch_app", "computer_write_clipboard"] {
        assert!(
            !definition(&h, name)
                .description
                .contains("screenshot taken after"),
            "{name}"
        );
        assert!(
            !schema_text(&h, name).contains("include_screenshot"),
            "{name}"
        );
    }
    h.dispose().await;
}

#[tokio::test]
async fn resolves_semantic_actions_from_a_fresh_snapshot_and_reports_backend_calls() {
    let h = harness();
    let result = h
        .call(
            "computer_click",
            json!({ "label": "Calculate", "role": "button" }),
        )
        .await;
    assert!(!result.is_error, "{result:?}");
    let clicks = h.backend.calls_for("click");
    assert_eq!(clicks.len(), 1);
    assert_subset(&clicks[0].args[0], json!({ "x": 1_180, "y": 228 }));

    let set_value = h
        .call(
            "computer_set_value",
            json!({ "label": "Display", "value": "468" }),
        )
        .await;
    assert!(!set_value.is_error, "{set_value:?}");
    assert_eq!(h.backend.calls_for("setValue").len(), 1);

    let select_text = h
        .call(
            "computer_select_text",
            json!({ "label": "Display", "start": 0, "length": 2 }),
        )
        .await;
    assert!(!select_text.is_error, "{select_text:?}");
    let selects = h.backend.calls_for("selectText");
    assert_eq!(selects.len(), 1);
    assert_eq!(selects[0].args[0]["node"]["label"], json!("Display"));
    assert_subset(&selects[0].args[1], json!({ "start": 0, "length": 2 }));
    assert_subset(
        &result_json(&select_text),
        json!({ "action": "computer_select_text", "value": "46" }),
    );
    h.dispose().await;
}

#[tokio::test]
async fn preserves_raw_text_values_including_whitespace_and_an_empty_value() {
    let h = harness();
    for value in ["  hello  ", " ", "\n"] {
        let result = h.call("computer_type_text", json!({ "text": value })).await;
        assert!(!result.is_error, "{value:?}: {result:?}");
        assert_eq!(
            h.backend.calls_for("typeText").last().unwrap().args,
            [json!(value)]
        );
    }
    let empty = h
        .call(
            "computer_set_value",
            json!({ "label": "Display", "value": "" }),
        )
        .await;
    assert!(!empty.is_error, "{empty:?}");
    assert_eq!(
        h.backend.calls_for("setValue").last().unwrap().args.last(),
        Some(&json!(""))
    );
    h.dispose().await;
}

#[tokio::test]
async fn fills_a_form_field_whose_visible_label_contains_a_non_breaking_space() {
    let root = ComputerUiNode {
        value: Some(String::new()),
        editable: Some(true),
        ..node(
            "AXTextField",
            Some("First name\u{a0}*"),
            rect(100.0, 200.0, 300.0, 40.0),
            None,
            Some("w1"),
            Vec::new(),
        )
    };
    let h = with_backend(fake_with(FakeComputerBackendOptions {
        root: Some(root),
        ..FakeComputerBackendOptions::default()
    }));
    let result = h
        .call(
            "computer_set_value",
            json!({
                "window_id": "w1",
                "label": "First name *",
                "value": "Ada",
                "include_screenshot": false,
            }),
        )
        .await;
    assert!(!result.is_error, "{result:?}");
    let set = h.backend.calls_for("setValue");
    assert_eq!(set.len(), 1);
    assert_eq!(set[0].args.last(), Some(&json!("Ada")));
    assert!(h.backend.calls_for("typeText").is_empty());
    h.dispose().await;
}

#[tokio::test]
async fn treats_a_camel_case_window_id_as_a_scroll_target() {
    let root = node(
        "window",
        Some("Calculator"),
        rect(100.0, 200.0, 300.0, 400.0),
        Some(ComputerPoint { x: 250.0, y: 400.0 }),
        Some("w1"),
        Vec::new(),
    );
    let h = with_backend(fake_with(FakeComputerBackendOptions {
        root: Some(root),
        ..FakeComputerBackendOptions::default()
    }));
    h.see().await;
    let result = h
        .call(
            "computer_scroll",
            json!({ "windowId": "w1", "delta_x": 10, "delta_y": -20 }),
        )
        .await;
    assert!(!result.is_error, "{result:?}");
    assert_subset(
        &h.backend.calls_for("scroll").last().unwrap().args[0],
        json!({ "x": 250, "y": 400 }),
    );
    h.dispose().await;
}

#[tokio::test]
async fn refuses_invalid_targets_with_structured_candidate_data() {
    let h = harness();
    let result = h
        .call("computer_click", json!({ "label": "does not exist" }))
        .await;
    assert!(result.is_error);
    let structured = result_json(&result);
    assert_eq!(
        structured["error"]["code"],
        json!("computer_target_not_found")
    );
    assert!(
        structured["error"]["candidates"]
            .as_array()
            .is_some_and(|candidates| !candidates.is_empty()),
        "{structured}"
    );
    h.dispose().await;
}

#[tokio::test]
async fn round_trips_the_shared_clipboard_and_starts_from_an_empty_one() {
    let h = harness();
    let empty = h.call("computer_read_clipboard", json!({})).await;
    assert_subset(
        &result_json(&empty),
        json!({ "action": "computer_read_clipboard", "value": "" }),
    );
    let write = h
        .call(
            "computer_write_clipboard",
            json!({ "text": "  copied\ntext  " }),
        )
        .await;
    assert!(!write.is_error, "{write:?}");
    assert_eq!(
        h.backend.calls_for("writeClipboard").last().unwrap().args,
        [json!("  copied\ntext  ")]
    );
    let read = h.call("computer_read_clipboard", json!({})).await;
    assert_subset(&result_json(&read), json!({ "value": "  copied\ntext  " }));
    h.dispose().await;
}

#[tokio::test]
async fn tells_the_model_the_clipboard_belongs_to_the_user_too() {
    let h = harness();
    for name in ["computer_read_clipboard", "computer_write_clipboard"] {
        assert!(
            definition(&h, name)
                .description
                .contains("shared with the human user"),
            "{name}"
        );
    }
    h.dispose().await;
}

#[tokio::test]
async fn refuses_clipboard_text_past_the_byte_limit_before_it_reaches_the_backend() {
    let h = harness();
    let result = h
        .call(
            "computer_write_clipboard",
            json!({ "text": "x".repeat(MAX_COMPUTER_CLIPBOARD_BYTES + 1) }),
        )
        .await;
    assert!(result.is_error);
    assert!(h.backend.calls_for("writeClipboard").is_empty());
    h.dispose().await;
}

#[tokio::test]
async fn refuses_a_set_value_past_the_text_bound_before_it_reaches_the_backend() {
    let h = harness();
    let result = h
        .call(
            "computer_set_value",
            json!({
                "label": "Display",
                "role": "text-field",
                "value": "x".repeat(COMPUTER_TEXT_MAX_LENGTH + 1),
            }),
        )
        .await;
    assert!(result.is_error);
    assert!(h.backend.calls_for("setValue").is_empty());
    let within = h
        .call(
            "computer_set_value",
            json!({
                "label": "Display",
                "role": "text-field",
                "value": "x".repeat(COMPUTER_TEXT_MAX_LENGTH),
            }),
        )
        .await;
    assert!(!within.is_error, "{within:?}");
    assert_eq!(h.backend.calls_for("setValue").len(), 1);
    h.dispose().await;
}

#[tokio::test]
async fn refuses_a_press_key_chord_past_the_contracts_shape_before_dispatch() {
    let h = harness();
    let too_many = (0..17)
        .map(|index| format!("Key{index}"))
        .collect::<Vec<_>>()
        .join("+");
    let result = h
        .call("computer_press_key", json!({ "key": too_many }))
        .await;
    assert!(result.is_error);
    assert!(h.backend.calls_for("hotkey").is_empty());
    let long_key = h
        .call(
            "computer_press_key",
            json!({ "key": format!("ctrl+{}", "k".repeat(129)) }),
        )
        .await;
    assert!(long_key.is_error);
    assert!(h.backend.calls_for("hotkey").is_empty());
    let within = h
        .call("computer_press_key", json!({ "key": "Control+L" }))
        .await;
    assert!(!within.is_error, "{within:?}");
    let chords = h.backend.calls_for("hotkey");
    assert_eq!(chords.len(), 1);
    assert_eq!(chords[0].args[0], json!(["Control", "L"]));
    h.dispose().await;
}

#[tokio::test]
async fn passes_xdotool_style_key_spellings_through_to_the_backend_unchanged() {
    let h = harness();
    let pressed = h
        .call("computer_press_key", json!({ "key": "Page_Up" }))
        .await;
    assert!(!pressed.is_error, "{pressed:?}");
    assert_eq!(args_of(&h, "pressKey"), [json!(["Page_Up"])]);
    let chord = h
        .call("computer_press_key", json!({ "key": "meta+KP_Enter" }))
        .await;
    assert!(!chord.is_error, "{chord:?}");
    assert_eq!(args_of(&h, "hotkey"), [json!([["meta", "KP_Enter"]])]);
    h.dispose().await;
}

#[tokio::test]
async fn refuses_control_off_mutations_even_for_a_gated_provider_without_touching_the_backend() {
    let h = harness();
    h.manager.stop_thread(THREAD).await.unwrap();
    let refused = h.call("computer_click", json!({ "x": 10, "y": 10 })).await;
    assert!(refused.is_error, "{refused:?}");
    assert!(h.backend.calls_for("click").is_empty());
    h.dispose().await;
}

#[tokio::test]
async fn includes_the_control_disclosure_on_the_first_mutation_payload_of_a_turn() {
    let h = harness();
    assert!(
        crate::computer::manager::COMPUTER_CONTROL_ON_DISCLOSURE.contains("Computer control is on")
    );
    let first = h
        .call("computer_press_key", json!({ "key": "enter" }))
        .await;
    assert!(!first.is_error, "{first:?}");
    assert!(text(&first).contains("Computer control is on"));
    let second = h
        .call("computer_press_key", json!({ "key": "enter" }))
        .await;
    assert!(!second.is_error, "{second:?}");
    assert!(!text(&second).contains("Computer control is on"));
    h.dispose().await;
}

#[tokio::test]
async fn refuses_a_semantic_action_name_past_the_contracts_bound() {
    let h = harness();
    let result = h
        .call(
            "computer_perform_action",
            json!({ "label": "Display", "action": "a".repeat(257) }),
        )
        .await;
    assert!(result.is_error);
    assert!(h.backend.calls_for("performAction").is_empty());
    h.dispose().await;
}

const MAC_ACTIONS: [&str; 8] = [
    "AXPress",
    "press",
    "open",
    "show_menu",
    "menu",
    "pick",
    "confirm",
    "cancel",
];

#[tokio::test]
async fn passes_the_macos_secondary_action_names_through_to_the_backend_verbatim() {
    let h = macos();
    for action in MAC_ACTIONS {
        let result = h
            .call(
                "computer_perform_action",
                json!({ "label": "Calculate", "action": action }),
            )
            .await;
        assert!(!result.is_error, "{action}: {result:?}");
    }
    let sent: Vec<Value> = h
        .backend
        .calls_for("performAction")
        .into_iter()
        .map(|call| call.args[1].clone())
        .collect();
    assert_eq!(sent, MAC_ACTIONS.map(|action| json!(action)));
    h.dispose().await;
}

#[tokio::test]
async fn refuses_malformed_select_text_ranges_and_x_y_targets_before_the_backend() {
    let h = harness();
    for arguments in [
        json!({ "label": "Display" }),
        json!({ "label": "Display", "start": 0 }),
        json!({ "label": "Display", "start": -1, "length": 1 }),
        json!({ "label": "Display", "start": 0, "length": -1 }),
        json!({ "label": "Display", "start": 0.5, "length": 1 }),
        json!({ "label": "Display", "start": 0, "length": COMPUTER_SELECT_TEXT_RANGE_MAX + 1 }),
        json!({ "x": 100, "y": 200, "start": 0, "length": 1 }),
    ] {
        let result = h.call("computer_select_text", arguments.clone()).await;
        assert!(result.is_error, "{arguments}: {result:?}");
    }
    assert!(h.backend.calls_for("selectText").is_empty());
    assert!(h.backend.calls_for("getState").is_empty());
    let caret = h
        .call(
            "computer_select_text",
            json!({ "label": "Display", "start": 1, "length": 0 }),
        )
        .await;
    assert!(!caret.is_error, "{caret:?}");
    assert_eq!(h.backend.calls_for("selectText").len(), 1);
    h.dispose().await;
}

#[tokio::test]
async fn reports_clipboard_tools_as_unsupported_on_a_backend_without_them() {
    let backend = Scripted::default_fake();
    backend.remove(ComputerBackendMethod::ReadClipboard);
    backend.remove(ComputerBackendMethod::WriteClipboard);
    let h = with_backend(backend);
    for (name, arguments) in [
        ("computer_read_clipboard", json!({})),
        ("computer_write_clipboard", json!({ "text": "nope" })),
    ] {
        let result = h.call(name, arguments).await;
        assert!(result.is_error, "{name}");
        assert!(
            text(&result).contains("does not support clipboard access"),
            "{name}: {result:?}"
        );
    }
    h.dispose().await;
}

#[tokio::test]
async fn keeps_the_clipboard_read_behind_approval_instead_of_the_perception_set() {
    let h = harness();
    let annotations = definition(&h, "computer_read_clipboard")
        .annotations
        .unwrap();
    assert_subset(
        &annotations,
        json!({ "readOnlyHint": false, "destructiveHint": false }),
    );
    h.decide(ApprovalDecision::Decline);
    let refused = h.call("computer_read_clipboard", json!({})).await;
    assert!(refused.is_error);
    assert!(h.backend.calls_for("readClipboard").is_empty());
    assert_eq!(h.approvals.cards.lock().unwrap().len(), 1);
    h.dispose().await;
}

#[tokio::test]
async fn refuses_a_second_threads_actions_without_encouraging_retry_loops_and_keeps_its_perception()
{
    let h = harness();
    let a = h.context_for("thread-a", None);
    let b = h.context_for("thread-b", None);
    h.see_in(&a).await;
    let owned = h
        .call_in("computer_click", json!({ "x": 10, "y": 10 }), &a)
        .await;
    assert!(!owned.is_error, "{owned:?}");

    let blocked = h
        .call_in("computer_type_text", json!({ "text": "hello" }), &b)
        .await;
    assert!(blocked.is_error);
    let payload = result_json(&blocked);
    assert_subset(
        &payload,
        json!({ "error": { "code": "computer_controlled_by_other_thread", "retryable": false } }),
    );
    assert!(
        payload["error"]["message"]
            .as_str()
            .unwrap()
            .contains("another conversation")
    );
    assert!(h.backend.calls_for("typeText").is_empty());

    for (name, arguments) in [
        ("computer_list_windows", json!({})),
        ("computer_get_state", json!({ "include_screenshot": true })),
        ("computer_get_screen_size", json!({})),
        (
            "computer_screenshot",
            json!({ "x": 0, "y": 0, "width": 100, "height": 100 }),
        ),
    ] {
        let perception = h.call_in(name, arguments, &b).await;
        assert!(!perception.is_error, "{name}: {perception:?}");
    }

    h.manager
        .release_desktop_control("thread-a", None)
        .await
        .unwrap();
    let handover = h
        .call_in("computer_type_text", json!({ "text": "hello" }), &b)
        .await;
    assert!(!handover.is_error, "{handover:?}");
    let now_blocked = h
        .call_in("computer_click", json!({ "x": 1, "y": 1 }), &a)
        .await;
    assert_subset(
        &result_json(&now_blocked),
        json!({ "error": { "code": "computer_controlled_by_other_thread" } }),
    );
    h.dispose().await;
}

#[tokio::test]
async fn reads_an_explicitly_null_scroll_target_as_no_target() {
    let h = harness();
    h.see().await;
    let result = h
        .call(
            "computer_scroll",
            json!({
                "x": null,
                "y": null,
                "label": null,
                "window_id": null,
                "delta_x": 0,
                "delta_y": 120,
            }),
        )
        .await;
    assert!(!result.is_error, "{result:?}");
    assert_subset(&json!(args_of(&h, "scroll")), json!([[null, 0, 150]]));
    h.dispose().await;
}

#[tokio::test]
async fn reports_scroll_travel_and_spends_no_extra_capture_doing_it() {
    let h = harness();
    h.see().await;
    let seen = h.backend.calls_for("captureScreenshot").len();
    let result = h
        .call(
            "computer_scroll",
            json!({ "x": 1_100, "y": 200, "delta_x": 0, "delta_y": 300 }),
        )
        .await;
    assert!(!result.is_error, "{result:?}");
    assert_eq!(kinds(&result), ["text", "image"]);
    assert_subset(
        &result_json(&result),
        json!({
            "action": "computer_scroll",
            "scroll": {
                "requested": { "deltaX": 0, "deltaY": 375 },
                "injected": { "deltaX": 0, "deltaY": 375 },
                "gearing": 1,
            },
        }),
    );
    assert_eq!(h.backend.calls_for("captureScreenshot").len(), seen + 3);
    h.dispose().await;
}

#[tokio::test]
async fn opts_out_of_the_captures_with_the_screenshot_keeping_the_request_telemetry() {
    let h = harness();
    h.see().await;
    let seen = h.backend.calls_for("captureScreenshot").len();
    let result = h
        .call(
            "computer_scroll",
            json!({
                "x": 1_100,
                "y": 200,
                "delta_x": 0,
                "delta_y": 300,
                "include_screenshot": false,
            }),
        )
        .await;
    assert_eq!(kinds(&result), ["text"]);
    assert_eq!(h.backend.calls_for("captureScreenshot").len(), seen);
    let payload = result_json(&result);
    assert_subset(
        &payload["scroll"]["requested"],
        json!({ "deltaX": 0, "deltaY": 375 }),
    );
    assert_subset(
        &payload["scroll"]["injected"],
        json!({ "deltaX": 0, "deltaY": 375 }),
    );
    assert_eq!(payload["scroll"]["requested"].as_object().unwrap().len(), 2);
    assert_eq!(payload["scroll"]["injected"].as_object().unwrap().len(), 2);
    assert!(payload["scroll"].get("traveledY").is_none(), "{payload}");
    h.dispose().await;
}

#[tokio::test]
async fn tells_the_model_that_scroll_distance_is_verified_rather_than_assumed() {
    let h = harness();
    let description = definition(&h, "computer_scroll").description;
    assert!(description.contains("scroll.traveledY"));
    assert!(description.contains("delta_x and delta_y"));
    assert!(description.contains("edge or dropped input"));
    assert!(description.contains("computer_get_state"));
    h.dispose().await;
}

#[tokio::test]
async fn defaults_the_omitted_scroll_axis_to_zero_in_direct_and_batch_calls() {
    for (axes, expected) in [
        (json!({ "delta_y": 80 }), json!([0, 100])),
        (json!({ "delta_x": -40 }), json!([-50, 0])),
    ] {
        let h = harness();
        h.see().await;
        let required = definition(&h, "computer_scroll").input_schema["required"].clone();
        let required = required.as_array().cloned().unwrap_or_default();
        assert!(!required.contains(&json!("delta_x")));
        assert!(!required.contains(&json!("delta_y")));
        let mut direct = axes.as_object().unwrap().clone();
        direct.insert("include_screenshot".into(), json!(false));
        let result = h.call("computer_scroll", Value::Object(direct)).await;
        assert!(!result.is_error, "{axes}: {result:?}");
        let last = |h: &Harness| {
            Value::Array(h.backend.calls_for("scroll").last().unwrap().args[1..3].to_vec())
        };
        assert_subset(&last(&h), expected.clone());
        let mut step = axes.as_object().unwrap().clone();
        step.insert("type".into(), json!("scroll"));
        let batch = h.call("computer_run", json!({ "steps": [step] })).await;
        assert_subset(
            &result_json(&batch),
            json!({ "completed": 1, "stopped": false }),
        );
        assert_subset(&last(&h), expected);
        h.dispose().await;
    }
}

#[tokio::test]
async fn refuses_an_empty_or_zero_scroll_before_dispatch() {
    let h = harness();
    h.see().await;
    for axes in [json!({}), json!({ "delta_x": 0, "delta_y": 0 })] {
        let result = h.call("computer_scroll", axes.clone()).await;
        assert!(result.is_error, "{axes}");
        assert!(
            serde_json::to_string(&result).unwrap().contains("nonzero"),
            "{result:?}"
        );
    }
    assert!(h.backend.calls_for("scroll").is_empty());
    h.dispose().await;
}

#[tokio::test]
async fn still_resolves_a_scroll_target_when_one_is_actually_given() {
    let h = harness();
    h.see().await;
    h.call(
        "computer_scroll",
        json!({ "x": 100, "y": 100, "delta_x": 0, "delta_y": -50 }),
    )
    .await;
    assert_subset(
        &json!(args_of(&h, "scroll")),
        json!([
            [{ "x": 125, "y": 125 }, 0, -48],
            [{ "x": 125, "y": 125 }, 0, -14.5],
        ]),
    );
    h.dispose().await;
}

#[tokio::test]
async fn never_hands_the_model_an_image_larger_than_it_will_actually_be_shown() {
    let h = harness();
    let schema = definition(&h, "computer_screenshot").input_schema;
    assert_eq!(
        schema["properties"]["max_dimension"]["maximum"].as_f64(),
        Some(f64::from(DEFAULT_COMPUTER_CAPTURE_MAX_DIMENSION))
    );
    assert_eq!(DEFAULT_COMPUTER_CAPTURE_MAX_DIMENSION, 1_536);
    h.call(
        "computer_screenshot",
        json!({ "window_id": "fake-terminal", "max_dimension": 8_000 }),
    )
    .await;
    assert_subset(
        &h.backend
            .calls_for("captureScreenshot")
            .last()
            .unwrap()
            .args[0],
        json!({
            "kind": "window",
            "windowId": "fake-terminal",
            "maxDimension": DEFAULT_COMPUTER_CAPTURE_MAX_DIMENSION,
        }),
    );
    h.dispose().await;
}

#[tokio::test(start_paused = true)]
async fn waits_without_touching_the_desktop_and_never_for_longer_than_its_bound() {
    let h = harness();
    let started = tokio::time::Instant::now();
    let result = h.call("computer_wait", json!({ "duration_ms": 5 })).await;
    assert!(started.elapsed() >= std::time::Duration::from_millis(4));
    assert!(!result.is_error, "{result:?}");
    assert_subset(&result_json(&result), json!({ "waitedMs": 5 }));
    assert!(h.backend.calls_for("captureScreenshot").is_empty());
    assert!(h.backend.calls_for("click").is_empty());
    let clamped = h
        .call("computer_wait", json!({ "duration_ms": 60 * 60 * 1_000 }))
        .await;
    assert_subset(
        &result_json(&clamped),
        json!({ "waitedMs": COMPUTER_WAIT_MAX_MS }),
    );
    let negative = h.call("computer_wait", json!({ "duration_ms": -5 })).await;
    assert_subset(&result_json(&negative), json!({ "waitedMs": 0 }));
    h.dispose().await;
}

#[tokio::test]
async fn holds_modifiers_across_a_click_and_a_scroll_and_refuses_a_name_it_cannot_press() {
    let h = harness();
    h.see().await;
    h.call(
        "computer_click",
        json!({ "x": 40, "y": 40, "modifiers": ["shift"], "include_screenshot": false }),
    )
    .await;
    assert_subset(
        &Value::Array(h.backend.calls_for("click").last().unwrap().args.clone()),
        json!([{ "x": 50, "y": 50 }, ["shift"]]),
    );
    h.call(
        "computer_scroll",
        json!({
            "x": 40,
            "y": 40,
            "delta_x": 0,
            "delta_y": 8,
            "modifiers": ["ctrl", "ctrl"],
            "include_screenshot": false,
        }),
    )
    .await;
    assert_subset(
        &Value::Array(h.backend.calls_for("scroll").last().unwrap().args.clone()),
        json!([{ "x": 50, "y": 50 }, 0, 10, ["ctrl"]]),
    );
    let refused = h
        .call(
            "computer_click",
            json!({ "x": 40, "y": 40, "modifiers": ["hyper"] }),
        )
        .await;
    assert!(refused.is_error);
    assert!(text(&refused).contains("hyper"), "{refused:?}");
    h.dispose().await;
}

#[tokio::test]
async fn sends_a_triple_click_as_one_gesture_and_refuses_where_it_cannot_be_one() {
    let h = harness();
    h.see().await;
    let tripled = h
        .call(
            "computer_click",
            json!({ "x": 40, "y": 40, "count": 3, "include_screenshot": false }),
        )
        .await;
    assert!(!tripled.is_error, "{tripled:?}");
    assert_subset(
        &result_json(&tripled),
        json!({ "action": "computer_click" }),
    );
    assert_eq!(h.backend.calls_for("tripleClick").len(), 1);
    assert!(h.backend.calls_for("click").is_empty());
    h.dispose().await;

    let without = Scripted::default_fake();
    without.remove(ComputerBackendMethod::TripleClick);
    let limited = with_backend(without);
    limited.see().await;
    let refused = limited
        .call("computer_click", json!({ "x": 40, "y": 40, "count": 3 }))
        .await;
    assert!(refused.is_error);
    assert!(
        text(&refused).contains("cannot send a triple click"),
        "{refused:?}"
    );
    limited.dispose().await;
}

#[tokio::test]
async fn photographs_a_window_the_action_opened_instead_of_reporting_nothing_changed() {
    let h = harness();
    let first = h
        .call(
            "computer_press_key",
            json!({ "key": "enter", "window_id": "fake-terminal" }),
        )
        .await;
    assert_eq!(kinds(&first), ["text", "image"]);

    let before = h.backend.fake.list_windows().await.unwrap();
    let mut opened_windows: Vec<ComputerWindow> = before.clone();
    opened_windows.push(ComputerWindow {
        app_name: Some("org.kde.konsole".into()),
        pid: before
            .iter()
            .find(|window| window.id == "fake-terminal")
            .and_then(|window| window.pid),
        ..window(
            "fake-dialog",
            "Save changes?",
            Some(rect(200.0, 200.0, 300.0, 200.0)),
        )
    });
    h.backend.hooks().press_key = Some(Arc::new(move |fake, _| {
        let windows = opened_windows.clone();
        async move {
            fake.emit_windows_changed(windows);
            Ok(ComputerBackendActionResult::default())
        }
        .boxed()
    }));

    let opened = h
        .call(
            "computer_press_key",
            json!({ "key": "enter", "window_id": "fake-terminal" }),
        )
        .await;
    assert!(!opened.is_error, "{opened:?}");
    assert_subset(
        &result_json(&opened),
        json!({ "screenshot": { "windowId": "fake-dialog" } }),
    );
    assert_eq!(kinds(&opened), ["text", "image"]);
    h.dispose().await;
}

#[tokio::test]
async fn keeps_the_target_window_when_the_only_new_window_is_the_capture_indicator() {
    let h = harness();
    h.call(
        "computer_press_key",
        json!({ "key": "enter", "window_id": "fake-terminal" }),
    )
    .await;

    let before = h.backend.fake.list_windows().await.unwrap();
    let mut with_pill: Vec<ComputerWindow> = before.clone();
    with_pill.push(ComputerWindow {
        app_name: Some("org.kde.konsole".into()),
        pid: before
            .iter()
            .find(|window| window.id == "fake-terminal")
            .and_then(|window| window.pid),
        ..window("fake-sharing-pill", "", Some(rect(16.0, 16.0, 66.0, 20.0)))
    });
    h.backend.hooks().press_key = Some(Arc::new(move |fake, _| {
        let windows = with_pill.clone();
        async move {
            fake.emit_windows_changed(windows);
            Ok(ComputerBackendActionResult::default())
        }
        .boxed()
    }));

    let pressed = h
        .call(
            "computer_press_key",
            json!({ "key": "enter", "window_id": "fake-terminal" }),
        )
        .await;
    assert!(!pressed.is_error, "{pressed:?}");
    assert_eq!(
        result_json(&pressed)["screenshot"]["windowId"],
        json!("fake-terminal"),
        "{pressed:?}"
    );
    h.dispose().await;
}

async fn press_enter_while_windows_appear(extra: Vec<ComputerWindow>) -> Value {
    let h = harness();
    h.call(
        "computer_press_key",
        json!({ "key": "enter", "window_id": "fake-terminal" }),
    )
    .await;
    let mut after = h.backend.fake.list_windows().await.unwrap();
    after.extend(extra);
    h.backend.hooks().press_key = Some(Arc::new(move |fake, _| {
        let windows = after.clone();
        async move {
            fake.emit_windows_changed(windows);
            Ok(ComputerBackendActionResult::default())
        }
        .boxed()
    }));
    let pressed = h
        .call(
            "computer_press_key",
            json!({ "key": "enter", "window_id": "fake-terminal" }),
        )
        .await;
    assert!(!pressed.is_error, "{pressed:?}");
    let window_id = result_json(&pressed)["screenshot"]["windowId"].clone();
    h.dispose().await;
    window_id
}

fn save_panel_at(bounds: ComputerRect) -> ComputerWindow {
    ComputerWindow {
        app_name: Some("Open and Save Panel Service".into()),
        pid: Some(4_242),
        stacking_index: Some(0),
        ..window("fake-save-panel", "Save", Some(bounds))
    }
}

#[tokio::test]
async fn observes_a_save_panel_that_another_process_opens_over_the_target() {
    assert_eq!(
        press_enter_while_windows_appear(vec![save_panel_at(rect(100.0, 80.0, 600.0, 400.0))])
            .await,
        json!("fake-save-panel")
    );
    assert_eq!(
        press_enter_while_windows_appear(vec![save_panel_at(rect(2_000.0, 2_000.0, 600.0, 400.0))])
            .await,
        json!("fake-terminal")
    );
    let same_app_behind = ComputerWindow {
        app_name: Some("org.kde.konsole".into()),
        pid: Some(1_001),
        stacking_index: Some(5),
        ..window(
            "fake-terminal-2",
            "Other",
            Some(rect(60.0, 60.0, 400.0, 300.0)),
        )
    };
    assert_eq!(
        press_enter_while_windows_appear(vec![
            same_app_behind,
            save_panel_at(rect(100.0, 80.0, 600.0, 400.0)),
        ])
        .await,
        json!("fake-save-panel")
    );
}

#[tokio::test]
async fn says_an_unchanged_frame_is_unsettled_rather_than_asserting_the_action_missed() {
    let h = harness();
    h.call(
        "computer_press_key",
        json!({ "key": "enter", "window_id": "fake-terminal" }),
    )
    .await;
    let quiet = h
        .call(
            "computer_press_key",
            json!({ "key": "enter", "window_id": "fake-terminal" }),
        )
        .await;
    let payload = result_json(&quiet);
    assert_subset(&payload, json!({ "screenshotUnchanged": true }));
    assert!(
        payload["note"]
            .as_str()
            .is_some_and(|note| note.contains("does not prove the action missed")),
        "{payload}"
    );
    h.dispose().await;
}

#[tokio::test]
async fn scopes_the_elements_digest_by_window_and_by_label_and_counts_what_it_drops() {
    let h = harness();
    let all = result_json(&h.call("computer_get_state", json!({})).await);
    let window_id = all["elementWindowId"].as_str().unwrap().to_owned();
    assert_eq!(window_id, "fake-calculator");

    let scoped = result_json(
        &h.call("computer_get_state", json!({ "window_id": window_id }))
            .await,
    );
    let elements = scoped["elements"].as_array().unwrap();
    assert!(!elements.is_empty());
    assert_eq!(scoped["elementWindowId"], json!(window_id));
    assert!(
        elements
            .iter()
            .all(|element| element.get("windowId").is_none())
    );

    let label = all["elements"][0]["label"].as_str().unwrap().to_owned();
    let filtered = result_json(
        &h.call(
            "computer_get_state",
            json!({ "label_contains": label.to_uppercase() }),
        )
        .await,
    );
    let filtered = filtered["elements"].as_array().unwrap().clone();
    assert!(!filtered.is_empty());
    assert!(filtered.iter().all(|element| {
        element["label"]
            .as_str()
            .unwrap()
            .to_lowercase()
            .contains(&label.to_lowercase())
    }));

    let none = result_json(
        &h.call(
            "computer_get_state",
            json!({ "label_contains": "no control is called this" }),
        )
        .await,
    );
    assert_eq!(none["elements"], json!([]));
    assert!(none.get("elementsTruncated").is_none(), "{none}");
    h.dispose().await;
}

#[tokio::test]
async fn keeps_per_element_window_ids_when_one_listing_spans_windows() {
    let entry = |window_id: &str, label: &str| {
        node(
            "button",
            Some(label),
            rect(0.0, 0.0, 80.0, 30.0),
            None,
            Some(window_id),
            Vec::new(),
        )
    };
    let root = node(
        "desktop",
        None,
        rect(0.0, 0.0, 1_920.0, 1_080.0),
        None,
        None,
        vec![
            node(
                "window",
                Some("Terminal"),
                rect(40.0, 40.0, 960.0, 720.0),
                None,
                Some("fake-terminal"),
                vec![entry("fake-terminal", "Run")],
            ),
            node(
                "window",
                Some("Calculator"),
                rect(1_050.0, 120.0, 420.0, 620.0),
                None,
                Some("fake-calculator"),
                vec![entry("fake-calculator", "Calculate")],
            ),
        ],
    );
    let h = with_backend(fake_with(FakeComputerBackendOptions {
        root: Some(root),
        ..FakeComputerBackendOptions::default()
    }));
    let payload = result_json(&h.call("computer_get_state", json!({})).await);
    assert!(payload.get("elementWindowId").is_none(), "{payload}");
    let listed: Vec<(Value, Value)> = payload["elements"]
        .as_array()
        .unwrap()
        .iter()
        .map(|element| (element["label"].clone(), element["windowId"].clone()))
        .collect();
    assert_eq!(
        listed,
        [
            (json!("Run"), json!("fake-terminal")),
            (json!("Calculate"), json!("fake-calculator")),
        ]
    );
    h.dispose().await;
}

#[tokio::test]
async fn brings_a_window_forward_only_through_the_explicit_tool_and_refuses_where_it_cannot() {
    let raised: Arc<Mutex<Vec<String>>> = Arc::default();
    let backend = Scripted::default_fake();
    let sink = raised.clone();
    backend.hooks().raise_window = Some(Arc::new(move |_, window_id| {
        sink.lock().unwrap().push(window_id);
        async { Ok(()) }.boxed()
    }));
    let h = with_backend(backend);
    h.decide(ApprovalDecision::Accept);
    let result = h
        .call(
            "computer_activate_window",
            json!({ "window_id": "fake-terminal" }),
        )
        .await;
    {
        let cards = h.approvals.cards.lock().unwrap();
        assert_eq!(cards.len(), 1);
        assert_eq!(cards[0].tool_name, "computer_activate_window");
        let params: Value = serde_json::from_str(&cards[0].params_display).unwrap();
        assert_eq!(params["delivery_mode"], json!("foreground"));
    }
    assert!(!result.is_error, "{result:?}");
    assert_eq!(*raised.lock().unwrap(), ["fake-terminal"]);
    assert_subset(
        &result_json(&result),
        json!({ "action": "computer_activate_window", "windowId": "fake-terminal" }),
    );
    let missing = h
        .call(
            "computer_activate_window",
            json!({ "window_id": "no-such-window" }),
        )
        .await;
    assert!(missing.is_error);
    h.dispose().await;

    let without = Scripted::default_fake();
    without.remove(ComputerBackendMethod::RaiseWindow);
    let plain = with_backend(without);
    plain.decide(ApprovalDecision::Accept);
    let refused = plain
        .call(
            "computer_activate_window",
            json!({ "window_id": "fake-terminal" }),
        )
        .await;
    assert!(refused.is_error);
    assert!(
        text(&refused).contains("cannot bring a window forward"),
        "{refused:?}"
    );
    assert!(computer_tool_requires_approval("computer_activate_window"));
    plain.dispose().await;
}

#[tokio::test]
async fn describes_the_shortcut_form_and_the_semantic_actions_this_desktop_actually_accepts() {
    let linux = harness();
    let hotkey = definition(&linux, "computer_press_key").description;
    assert!(hotkey.contains("A chord"));
    assert!(!hotkey.contains("ordered key sequence"));
    assert!(hotkey.contains("releases in reverse"));
    assert_eq!(
        definition(&linux, "computer_perform_action").input_schema["properties"]["action"]["enum"],
        json!(["activate", "click"])
    );
    assert!(
        definition(&linux, "computer_launch_app")
            .description
            .contains("executable on PATH")
    );
    linux.dispose().await;

    let mac = macos();
    let mac_hotkey = definition(&mac, "computer_press_key").description;
    assert!(mac_hotkey.contains("exactly one other key"));
    assert!(mac_hotkey.contains("More than one non-modifier key is refused"));
    let action =
        definition(&mac, "computer_perform_action").input_schema["properties"]["action"].clone();
    assert_eq!(action["enum"], json!(MAC_ACTIONS));
    assert!(
        action["description"]
            .as_str()
            .unwrap()
            .contains("does not advertise")
    );
    let launch = definition(&mac, "computer_launch_app");
    assert!(launch.description.contains("the way macOS does"));
    assert!(
        launch
            .description
            .contains("requests no foreground activation")
    );
    assert!(launch.description.contains("may create no usable window"));
    let app = launch.input_schema["properties"]["app"]["description"]
        .as_str()
        .unwrap()
        .to_owned();
    assert!(app.contains("com.apple.Safari"));
    assert!(!app.contains("/Applications/Safari.app"));
    mac.dispose().await;
}

#[tokio::test]
async fn separates_admission_refusals_from_uncertain_dispatched_input() {
    let notes = computer_tool_instructions();
    assert!(notes.contains("\"not-dispatched\""));
    assert!(notes.contains("\"dispatched-unknown\""));
    assert!(notes.contains("never replay it"));
    assert!(notes.contains("repeated_unverified_action"));
}

#[tokio::test]
async fn matches_a_label_exactly_as_written_spaces_included() {
    let h = harness();
    let refused = h
        .call("computer_click", json!({ "label": "Calculate " }))
        .await;
    assert!(refused.is_error);
    assert_subset(
        &result_json(&refused),
        json!({ "error": { "code": "computer_target_not_found" } }),
    );
    let found = h
        .call("computer_click", json!({ "label": "Calculate" }))
        .await;
    assert!(!found.is_error, "{found:?}");
    h.dispose().await;
}

#[tokio::test]
async fn clamps_a_drag_duration_to_the_bound_its_schema_advertises() {
    let h = harness();
    h.see().await;
    h.call(
        "computer_drag",
        json!({ "from": { "x": 1, "y": 1 }, "to": { "x": 2, "y": 2 }, "duration_ms": 1e9 }),
    )
    .await;
    h.call(
        "computer_drag",
        json!({ "from": { "x": 1, "y": 1 }, "to": { "x": 2, "y": 2 }, "duration_ms": -5 }),
    )
    .await;
    let durations: Vec<Option<f64>> = h
        .backend
        .calls_for("drag")
        .iter()
        .map(|call| call.args[2].as_f64())
        .collect();
    assert_eq!(durations, [Some(30_000.0), Some(0.0)]);
    assert_subset(
        &definition(&h, "computer_drag").input_schema["properties"]["duration_ms"],
        json!({ "maximum": 30_000, "minimum": 0 }),
    );
    h.dispose().await;
}
