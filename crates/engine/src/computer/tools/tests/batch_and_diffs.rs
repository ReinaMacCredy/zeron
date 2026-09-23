use super::*;

use futures::FutureExt;
use zeron_proto::computer::{
    ComputerApp, ComputerDeliveryMode, ComputerDeliveryVerification, ComputerRect, ComputerTarget,
    ComputerUiNode, ComputerWindow, CuaEffect, McpContent,
};

use crate::computer::backend::{
    ComputerBackend, ComputerBackendActionResult, ComputerBackendError,
};
use crate::computer::desktop_operation_queue::desktop_delivery_mode;
use crate::computer::manager::tests::{app, rect};
use crate::computer::model_desktop_observation::is_model_desktop_observation_active;

const WINDOW: &str = "fake-calculator";

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

fn first_args(h: &Harness, method: &str) -> Vec<Value> {
    h.backend
        .calls_for(method)
        .into_iter()
        .map(|call| call.args[0].clone())
        .collect()
}

fn with_backend(backend: Arc<Scripted>) -> Harness {
    harness_with(HarnessOptions {
        backend,
        ..HarnessOptions::default()
    })
}

fn with_root(root: ComputerUiNode) -> Harness {
    with_backend(fake_with(FakeComputerBackendOptions {
        root: Some(root),
        ..FakeComputerBackendOptions::default()
    }))
}

fn with_apps(apps: Vec<ComputerApp>) -> Harness {
    with_backend(fake_with(FakeComputerBackendOptions {
        apps: Some(apps),
        ..FakeComputerBackendOptions::default()
    }))
}

fn tree(labels: &[&str], value: &str, truncated: bool) -> ComputerUiNode {
    let node = |role: &str, label: Option<&str>, value: Option<&str>, frame: ComputerRect| {
        ComputerUiNode {
            role: role.to_owned(),
            label: label.map(str::to_owned),
            value: value.map(str::to_owned),
            description: None,
            frame,
            activation_point: None,
            on_screen: true,
            window_id: Some(WINDOW.to_owned()),
            accessibility_root: None,
            node_path: None,
            editable: None,
            truncated: None,
            children: Vec::new(),
        }
    };
    ComputerUiNode {
        truncated: Some(truncated),
        children: labels
            .iter()
            .map(|label| {
                node(
                    "text-field",
                    Some(label),
                    Some(value),
                    rect(120.0, 120.0, 100.0, 30.0),
                )
            })
            .collect(),
        ..node("window", None, None, rect(100.0, 100.0, 900.0, 700.0))
    }
}

fn cards(h: &Harness) -> Vec<ComputerConsentCard> {
    h.approvals.cards.lock().unwrap().clone()
}

fn get_state_reads_since(h: &Harness, mark: usize) -> Vec<Value> {
    h.backend.fake.calls()[mark..]
        .iter()
        .filter(|call| call.method == "getState")
        .map(|call| call.args[0].clone())
        .collect()
}

fn fresh_reads_since(h: &Harness, mark: usize) -> usize {
    get_state_reads_since(h, mark)
        .iter()
        .filter(|options| options.get("reuseRecentTree").is_none())
        .count()
}

struct DyingTurn {
    checks: AtomicUsize,
    alive: usize,
}

#[async_trait]
impl ComputerToolSeams for DyingTurn {
    fn assert_turn_active(&self) -> Result<(), ComputerError> {
        if self.checks.fetch_add(1, Ordering::SeqCst) < self.alive {
            Ok(())
        } else {
            Err(ComputerError::Other("The requesting turn ended.".into()))
        }
    }

    async fn foreground_authorization(&self) -> ComputerForegroundAuthorization {
        ComputerForegroundAuthorization {
            user_requested_visible_use: true,
        }
    }
}

#[tokio::test]
async fn reports_the_active_step_instead_of_one_generic_batch_label() {
    let h = harness();
    let result = h
        .call(
            "computer_run",
            json!({ "steps": [
                { "type": "click", "label": "Display", "window_id": WINDOW },
                { "type": "type_text", "text": "468", "window_id": WINDOW },
                { "type": "write_clipboard", "text": "copied value" },
            ] }),
        )
        .await;
    assert!(!result.is_error, "{result:?}");
    assert_eq!(
        h.manager.cursor_activity().started(),
        [
            "Running sequence",
            "Clicking",
            "Typing",
            "Writing clipboard"
        ]
    );
    h.dispose().await;
}

#[tokio::test]
async fn runs_steps_in_order_through_the_same_manager_calls_and_closes_with_fresh_state() {
    let h = harness();
    let result = h
        .call(
            "computer_run",
            json!({ "steps": [
                { "type": "click", "label": "Display", "window_id": WINDOW },
                { "type": "type_text", "text": "468", "window_id": WINDOW },
                { "type": "select_text", "label": "Display", "start": 0, "length": 2 },
                { "type": "press_key", "key": "enter", "window_id": WINDOW },
            ] }),
        )
        .await;
    assert!(!result.is_error, "{result:?}");
    let payload = result_json(&result);
    assert_eq!(payload["completed"], json!(4));
    assert_eq!(payload["stopped"], json!(false));
    let steps = payload["steps"].as_array().unwrap();
    let summary: Vec<Value> = steps
        .iter()
        .map(|entry| json!([entry["step"], entry["type"], entry["ok"]]))
        .collect();
    assert_eq!(
        summary,
        [
            json!([0, "click", true]),
            json!([1, "type_text", true]),
            json!([2, "select_text", true]),
            json!([3, "press_key", true]),
        ]
    );
    for entry in steps {
        assert!(entry["result"].get("computerId").is_none(), "{entry}");
    }
    let labels: Vec<&str> = payload["state"]["elements"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|element| element["label"].as_str())
        .collect();
    assert!(labels.contains(&"Display"), "{labels:?}");
    assert_eq!(payload["state"]["elementWindowId"], json!(WINDOW));
    assert_eq!(h.backend.calls_for("click").len(), 1);
    assert_eq!(first_args(&h, "typeText"), [json!("468")]);
    let ranges: Vec<Value> = h
        .backend
        .calls_for("selectText")
        .into_iter()
        .map(|call| call.args[1].clone())
        .collect();
    assert_eq!(ranges, [json!({ "start": 0, "length": 2 })]);
    assert_eq!(h.backend.calls_for("pressKey").len(), 1);
    h.dispose().await;
}

#[tokio::test]
async fn stops_at_the_first_failure_and_reports_which_step_and_why() {
    let h = harness();
    h.backend
        .fake
        .fail_next_with("typeText", ComputerBackendError::new("seat unavailable"));
    let result = h
        .call(
            "computer_run",
            json!({ "steps": [
                { "type": "click", "label": "Display", "window_id": WINDOW },
                { "type": "type_text", "text": "1" },
                { "type": "press_key", "key": "enter" },
            ] }),
        )
        .await;
    assert!(!result.is_error, "{result:?}");
    let payload = result_json(&result);
    assert_eq!(payload["stopped"], json!(true));
    assert_eq!(payload["completed"], json!(1));
    assert_eq!(payload["steps"].as_array().map(Vec::len), Some(2));
    assert_subset(
        &payload["steps"][1],
        json!({
            "step": 1,
            "type": "type_text",
            "ok": false,
            "error": { "message": "seat unavailable" },
        }),
    );
    assert!(h.backend.calls_for("pressKey").is_empty());
    h.dispose().await;
}

#[tokio::test]
async fn refuses_a_malformed_batch_whole_before_anything_dispatches() {
    let h = harness();
    for steps in [
        json!([{ "type": "click", "label": "Display" }, { "type": "levitate" }]),
        json!([{ "type": "type_text", "text": "hi", "bogus": true }]),
        json!([{ "type": "type_text" }]),
        json!([{ "type": "click", "label": "Display" }, 42]),
    ] {
        let result = h
            .call("computer_run", json!({ "steps": steps.clone() }))
            .await;
        assert!(result.is_error, "{steps}: {result:?}");
    }
    assert!(h.backend.calls_for("click").is_empty());
    assert!(h.backend.calls_for("typeText").is_empty());
    h.dispose().await;
}

#[tokio::test]
async fn caps_the_step_count() {
    let h = harness();
    let steps: Vec<Value> = (0..26)
        .map(|_| json!({ "type": "press_key", "key": "enter" }))
        .collect();
    let result = h.call("computer_run", json!({ "steps": steps })).await;
    assert!(result.is_error);
    assert!(h.backend.calls_for("pressKey").is_empty());
    h.dispose().await;
}

#[tokio::test]
async fn asks_approval_once_for_the_declared_list_and_dispatches_nothing_when_refused() {
    let h = harness();
    h.decide(ApprovalDecision::Decline);
    let refused = h
        .call(
            "computer_run",
            json!({ "steps": [{ "type": "click", "label": "Display", "window_id": WINDOW }] }),
        )
        .await;
    assert!(refused.is_error);
    let cards = cards(&h);
    assert_eq!(cards.len(), 1);
    assert_eq!(cards[0].tool_name, "computer_run");
    assert!(h.backend.calls_for("click").is_empty());
    h.dispose().await;
}

#[tokio::test]
async fn scopes_only_the_activate_window_step_to_foreground_delivery() {
    let h = harness();
    let modes: Arc<Mutex<Vec<ComputerDeliveryMode>>> = Arc::default();
    {
        let mut hooks = h.backend.hooks();
        let seen = modes.clone();
        hooks.raise_window = Some(Arc::new(move |fake, window_id| {
            seen.lock().unwrap().push(desktop_delivery_mode());
            async move { fake.raise_window(&window_id).await }.boxed()
        }));
        let seen = modes.clone();
        hooks.click = Some(Arc::new(move |fake, (point, window_id)| {
            seen.lock().unwrap().push(desktop_delivery_mode());
            async move { fake.click(point, window_id.as_deref(), &[]).await }.boxed()
        }));
    }
    let result = h
        .call(
            "computer_run",
            json!({ "steps": [
                { "type": "activate_window", "window_id": WINDOW },
                { "type": "click", "label": "Display", "window_id": WINDOW },
            ] }),
        )
        .await;
    assert!(!result.is_error, "{result:?}");
    let modes = modes.lock().unwrap().clone();
    assert_eq!(
        modes[..2],
        [
            ComputerDeliveryMode::Foreground,
            ComputerDeliveryMode::Foreground
        ],
        "{modes:?}"
    );
    assert!(
        modes[2..]
            .iter()
            .all(|mode| *mode == ComputerDeliveryMode::Background),
        "{modes:?}"
    );
    assert!(modes.len() > 2, "{modes:?}");
    assert_eq!(
        first_args(&h, "raiseWindow"),
        [json!(WINDOW), json!("fake-terminal"), json!(WINDOW)]
    );
    h.dispose().await;
}

#[tokio::test]
async fn waits_for_a_label_mid_run_and_resolves_fresh_targets_per_step() {
    let h = harness();
    let result = h
        .call(
            "computer_run",
            json!({ "steps": [
                { "type": "wait", "duration_ms": 5_000, "label": "Display", "window_id": WINDOW },
                { "type": "set_value", "label": "Display", "window_id": WINDOW, "value": "42" },
            ] }),
        )
        .await;
    assert!(!result.is_error, "{result:?}");
    let payload = result_json(&result);
    assert_subset(
        &payload["steps"][0],
        json!({ "ok": true, "result": { "status": "ready" } }),
    );
    assert_subset(&payload["steps"][1], json!({ "ok": true }));
    assert_eq!(h.backend.calls_for("setValue").len(), 1);
    h.dispose().await;
}

#[tokio::test]
async fn carries_model_observation_authority_on_its_internal_reads() {
    let h = harness();
    let observed: Arc<Mutex<Vec<bool>>> = Arc::default();
    {
        let seen = observed.clone();
        h.backend.hooks().get_state = Some(Arc::new(move |fake, options| {
            seen.lock()
                .unwrap()
                .push(is_model_desktop_observation_active());
            async move { fake.get_state(options).await }.boxed()
        }));
    }
    let result = h
        .call(
            "computer_run",
            json!({ "steps": [
                { "type": "wait", "duration_ms": 2_000, "label": "Display", "window_id": WINDOW },
            ] }),
        )
        .await;
    assert!(!result.is_error, "{result:?}");
    let observed = observed.lock().unwrap().clone();
    assert!(observed.len() >= 2, "{observed:?}");
    assert!(observed.iter().all(|active| *active), "{observed:?}");
    h.dispose().await;
}

#[tokio::test]
async fn propagates_a_dead_turn_instead_of_reporting_a_half_run_as_data() {
    let h = harness();
    let context = ComputerToolContext {
        seams: Arc::new(DyingTurn {
            checks: AtomicUsize::new(0),
            alive: 2,
        }),
        ..h.context()
    };
    let result = h
        .call_in(
            "computer_run",
            json!({ "steps": [
                { "type": "click", "label": "Display", "window_id": WINDOW },
                { "type": "type_text", "text": "1" },
            ] }),
            &context,
        )
        .await;
    assert!(result.is_error, "{result:?}");
    assert!(
        text(&result).contains("The requesting turn ended."),
        "{result:?}"
    );
    assert_eq!(h.backend.calls_for("click").len(), 1);
    assert!(h.backend.calls_for("typeText").is_empty());
    h.dispose().await;
}

#[tokio::test]
async fn attaches_a_final_screenshot_of_the_affected_window_when_asked() {
    let h = harness();
    let result = h
        .call(
            "computer_run",
            json!({
                "steps": [{ "type": "click", "label": "Display", "window_id": WINDOW }],
                "include_screenshot": true,
            }),
        )
        .await;
    assert!(!result.is_error, "{result:?}");
    assert!(kinds(&result).contains(&"image"));
    assert_subset(
        &result_json(&result),
        json!({ "screenshot": { "windowId": WINDOW } }),
    );
    h.dispose().await;
}

#[tokio::test]
async fn pastes_through_the_clipboard_and_restores_the_users_contents() {
    let h = harness();
    h.call(
        "computer_write_clipboard",
        json!({ "text": "the user's copy" }),
    )
    .await;
    let result = h
        .call(
            "computer_run",
            json!({ "steps": [
                { "type": "click", "label": "Display", "window_id": WINDOW },
                { "type": "paste", "text": "long agent payload", "window_id": WINDOW },
            ] }),
        )
        .await;
    assert!(!result.is_error, "{result:?}");
    assert_subset(
        &result_json(&result)["steps"][1]["result"],
        json!({ "action": "computer_paste", "clipboardRestored": true }),
    );
    assert_eq!(
        first_args(&h, "writeClipboard"),
        [
            json!("the user's copy"),
            json!("long agent payload"),
            json!("the user's copy"),
        ]
    );
    assert_eq!(first_args(&h, "hotkey"), [json!(["ctrl", "v"])]);
    h.dispose().await;
}

#[tokio::test]
async fn saves_pastes_and_restores_the_shared_clipboard() {
    let h = harness();
    h.call("computer_write_clipboard", json!({ "text": "keep me" }))
        .await;
    let result = h
        .call(
            "computer_paste",
            json!({ "text": "pasted text", "window_id": WINDOW }),
        )
        .await;
    assert!(!result.is_error, "{result:?}");
    assert_subset(
        &result_json(&result),
        json!({ "action": "computer_paste", "clipboardRestored": true }),
    );
    assert_eq!(first_args(&h, "hotkey"), [json!(["ctrl", "v"])]);
    let clipboard = h.call("computer_read_clipboard", json!({})).await;
    assert_eq!(result_json(&clipboard)["value"], json!("keep me"));
    h.dispose().await;
}

#[tokio::test]
async fn uses_the_command_chord_on_a_macos_backend() {
    let h = harness_with(HarnessOptions {
        dialect: Some(ComputerAgentDialect::Macos),
        ..HarnessOptions::default()
    });
    let result = h
        .call(
            "computer_paste",
            json!({ "text": "payload", "window_id": WINDOW }),
        )
        .await;
    assert!(!result.is_error, "{result:?}");
    assert_eq!(first_args(&h, "hotkey"), [json!(["meta", "v"])]);
    h.dispose().await;
}

#[tokio::test]
async fn refuses_to_paste_over_a_clipboard_it_cannot_restore() {
    let h = harness();
    h.backend.fake.fail_next_with(
        "readClipboard",
        crate::computer::backend::CuaActionError::new(
            "non-text clipboard",
            zeron_proto::computer::CuaEffect::NotDispatched,
        )
        .code(crate::computer::cua_backend::CLIPBOARD_NOT_RESTORABLE_CODE),
    );
    let result = h
        .call(
            "computer_paste",
            json!({ "text": "agent text", "window_id": WINDOW }),
        )
        .await;
    assert!(result.is_error, "{result:?}");
    assert!(h.backend.calls_for("writeClipboard").is_empty());
    assert!(h.backend.calls_for("hotkey").is_empty());
    h.dispose().await;
}

#[tokio::test]
async fn still_restores_the_clipboard_when_the_paste_dispatch_fails() {
    let h = harness();
    h.call("computer_write_clipboard", json!({ "text": "user text" }))
        .await;
    h.backend.fake.fail_next("hotkey");
    let result = h
        .call(
            "computer_paste",
            json!({ "text": "agent text", "window_id": WINDOW }),
        )
        .await;
    assert!(result.is_error);
    assert_eq!(
        first_args(&h, "writeClipboard"),
        [json!("user text"), json!("agent text"), json!("user text")]
    );
    h.dispose().await;
}

#[tokio::test]
async fn reports_the_first_scoped_read_as_all_added_then_only_the_value_that_moved() {
    let h = harness();
    let baseline = result_json(
        &h.call(
            "computer_get_state",
            json!({ "window_id": WINDOW, "diff": true }),
        )
        .await,
    );
    let mut added: Vec<&str> = baseline["elementChanges"]["added"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|item| item["label"].as_str())
        .collect();
    added.sort_unstable();
    assert_eq!(added, ["Calculate", "Display"]);
    assert_eq!(baseline["elementChanges"]["removed"], json!([]));
    assert_eq!(baseline["elementChanges"]["changed"], json!([]));

    h.manager
        .set_value(
            Some(THREAD),
            ComputerTarget {
                label: Some("Display".into()),
                window_id: Some(WINDOW.into()),
                ..ComputerTarget::default()
            },
            "468",
        )
        .await
        .unwrap();
    let diff = result_json(
        &h.call(
            "computer_get_state",
            json!({ "window_id": WINDOW, "diff": true }),
        )
        .await,
    );
    assert!(diff.get("elements").is_none(), "{diff}");
    assert_eq!(diff["elementWindowId"], json!(WINDOW));
    assert_eq!(
        diff["elementChanges"],
        json!({
            "added": [],
            "removed": [],
            "changed": [{
                "ref": 1,
                "role": "text-field",
                "label": "Display",
                "was": "0",
                "value": "468",
            }],
        })
    );
    let steady = result_json(
        &h.call(
            "computer_get_state",
            json!({ "window_id": WINDOW, "diff": true }),
        )
        .await,
    );
    assert_eq!(
        steady["elementChanges"],
        json!({ "added": [], "removed": [], "changed": [] })
    );
    h.dispose().await;
}

#[tokio::test]
async fn keeps_scopes_apart_so_a_windowed_read_does_not_diff_the_desktop_digest() {
    let h = harness();
    h.call("computer_get_state", json!({ "window_id": WINDOW }))
        .await;
    let other = result_json(&h.call("computer_get_state", json!({ "diff": true })).await);
    assert!(
        other["elementChanges"]["added"]
            .as_array()
            .is_some_and(|added| !added.is_empty()),
        "{other}"
    );
    h.dispose().await;
}

#[tokio::test]
async fn attaches_a_click_diff_preserves_delivery_and_advances_the_baseline_without_a_screenshot() {
    let h = with_root(tree(&["Field"], "before", false));
    h.call("computer_get_state", json!({ "window_id": WINDOW }))
        .await;
    h.backend.hooks().click = Some(Arc::new(|fake, (point, window_id)| {
        async move {
            let result = fake.click(point, window_id.as_deref(), &[]).await?;
            fake.set_root(tree(&["Field"], "after", false));
            Ok(ComputerBackendActionResult {
                delivery_path: Some("ax".into()),
                verified: Some(ComputerDeliveryVerification::Unverifiable),
                effect: Some(CuaEffect::DispatchedUnknown),
                ..result
            })
        }
        .boxed()
    }));
    let mark = h.backend.fake.calls().len();
    let response = h
        .call(
            "computer_click",
            json!({ "window_id": WINDOW, "label": "Field", "include_screenshot": false }),
        )
        .await;
    assert!(!response.is_error, "{response:?}");
    let result = result_json(&response);
    assert_subset(
        &result,
        json!({
            "elementWindowId": WINDOW,
            "elementChanges": {
                "added": [],
                "removed": [],
                "changed": [{ "label": "Field", "was": "before", "value": "after", "ref": 0 }],
            },
            "delivery": { "path": "ax", "verified": "unverifiable", "effect": "dispatched-unknown" },
        }),
    );
    assert_eq!(kinds(&response), ["text"]);
    let last = get_state_reads_since(&h, mark).pop().unwrap();
    assert_eq!(last, json!({ "includeTree": true, "windowId": WINDOW }));
    assert!(h.backend.calls_for("captureScreenshot").is_empty());
    let next = h
        .call(
            "computer_get_state",
            json!({ "window_id": WINDOW, "diff": true }),
        )
        .await;
    assert_subset(
        &result_json(&next),
        json!({ "elementChanges": { "added": [], "removed": [], "changed": [] } }),
    );
    h.dispose().await;
}

#[tokio::test]
async fn attaches_nothing_without_a_matching_baseline() {
    for scope in ["none", "filtered", "other-window", "other-thread"] {
        let h = harness();
        if scope != "none" {
            let mut arguments = json!({
                "window_id": if scope == "other-window" { "fake-editor" } else { WINDOW },
            });
            if scope == "filtered" {
                arguments["label_contains"] = json!("Display");
            }
            let thread = if scope == "other-thread" {
                "other-thread"
            } else {
                THREAD
            };
            let baseline = h
                .call_in(
                    "computer_get_state",
                    arguments,
                    &h.context_for(thread, None),
                )
                .await;
            assert!(!baseline.is_error, "{scope}: {baseline:?}");
        }
        let mark = h.backend.fake.calls().len();
        let result = h
            .call(
                "computer_set_value",
                json!({
                    "window_id": WINDOW,
                    "label": "Display",
                    "value": "123",
                    "include_screenshot": false,
                }),
            )
            .await;
        assert!(!result.is_error, "{scope}: {result:?}");
        assert!(
            result_json(&result).get("elementChanges").is_none(),
            "{scope}"
        );
        let calls = h.backend.fake.calls();
        let set = calls[mark..]
            .iter()
            .position(|call| call.method == "setValue")
            .expect("setValue dispatched");
        assert!(
            calls[mark + set..]
                .iter()
                .all(|call| call.method != "getState"),
            "{scope}"
        );
        h.dispose().await;
    }
}

#[tokio::test]
async fn attaches_a_scoped_diff_for_each_element_changing_tool() {
    for (tool, arguments) in [
        ("computer_type_text", json!({ "text": "hello there" })),
        ("computer_paste", json!({ "text": "hello there" })),
        (
            "computer_set_value",
            json!({ "label": "Display", "value": "123" }),
        ),
        (
            "computer_perform_action",
            json!({ "label": "Calculate", "action": "activate" }),
        ),
        ("computer_invoke_menu", json!({ "path": ["File", "Save"] })),
    ] {
        let h = harness();
        h.decide(ApprovalDecision::Accept);
        h.call("computer_get_state", json!({ "window_id": WINDOW }))
            .await;
        let mut arguments = arguments;
        arguments["window_id"] = json!(WINDOW);
        arguments["include_screenshot"] = json!(false);
        let result = h.call(tool, arguments).await;
        assert!(!result.is_error, "{tool}: {result:?}");
        assert!(
            result_json(&result).get("elementChanges").is_some(),
            "{tool}: {result:?}"
        );
        h.dispose().await;
    }
}

#[tokio::test]
async fn propagates_cancellation_during_the_action_tree_read() {
    let h = harness();
    h.call("computer_get_state", json!({ "window_id": WINDOW }))
        .await;
    let context = h.context();
    let clicked = Arc::new(AtomicBool::new(false));
    {
        let mut hooks = h.backend.hooks();
        let flag = clicked.clone();
        hooks.click = Some(Arc::new(move |fake, (point, window_id)| {
            let flag = flag.clone();
            async move {
                let result = fake.click(point, window_id.as_deref(), &[]).await;
                flag.store(true, Ordering::SeqCst);
                result
            }
            .boxed()
        }));
        let flag = clicked.clone();
        let signal = context.signal.clone();
        hooks.get_state = Some(Arc::new(move |fake, options| {
            if flag.load(Ordering::SeqCst) {
                signal.cancel();
            }
            async move { fake.get_state(options).await }.boxed()
        }));
    }
    let result = h
        .call_in(
            "computer_click",
            json!({ "window_id": WINDOW, "label": "Calculate", "include_screenshot": false }),
            &context,
        )
        .await;
    assert!(result.is_error, "{result:?}");
    assert!(text(&result).contains("cancelled"), "{result:?}");
    assert!(result_json(&result).get("elementChanges").is_none());
    assert_eq!(h.backend.calls_for("click").len(), 1);
    h.dispose().await;
}

#[tokio::test]
async fn caps_added_removed_and_changed_entries_together_and_counts_omissions() {
    let labels: Vec<String> = (0..40).map(|i| format!("Field {i}")).collect();
    let before: Vec<&str> = labels.iter().map(String::as_str).collect();
    let h = with_root(tree(&before, "before", false));
    h.call("computer_get_state", json!({ "window_id": WINDOW }))
        .await;
    let added: Vec<String> = (0..25).map(|i| format!("New {i}")).collect();
    let after: Vec<&str> = before[..20]
        .iter()
        .copied()
        .chain(added.iter().map(String::as_str))
        .collect();
    h.backend.fake.set_root(tree(&after, "after", false));
    let result = result_json(
        &h.call(
            "computer_press_key",
            json!({ "window_id": WINDOW, "key": "tab", "include_screenshot": false }),
        )
        .await,
    );
    let changes = &result["elementChanges"];
    let total: usize = ["added", "removed", "changed"]
        .iter()
        .map(|kind| changes[kind].as_array().map_or(0, Vec::len))
        .sum();
    assert_eq!(total, 40, "{result}");
    assert_eq!(result["elementChangesOmitted"], json!(25));
    assert!(
        changes["removed"]
            .as_array()
            .unwrap()
            .iter()
            .all(|entry| entry.get("ref").is_none()),
        "{result}"
    );
    h.dispose().await;
}

#[tokio::test]
async fn marks_a_truncated_tree_as_incomplete_on_either_side() {
    for side in ["before", "after"] {
        let h = with_root(tree(&["Field"], "before", side == "before"));
        h.call("computer_get_state", json!({ "window_id": WINDOW }))
            .await;
        h.backend
            .fake
            .set_root(tree(&["Field"], "after", side == "after"));
        let result = result_json(
            &h.call(
                "computer_press_key",
                json!({ "window_id": WINDOW, "key": "tab", "include_screenshot": false }),
            )
            .await,
        );
        assert_eq!(
            result["elementChangesIncomplete"],
            json!(true),
            "{side}: {result}"
        );
        h.dispose().await;
    }
}

#[tokio::test]
async fn returns_the_unchanged_action_result_if_the_tree_read_fails() {
    let h = harness();
    h.call("computer_get_state", json!({ "window_id": WINDOW }))
        .await;
    let clicked = Arc::new(AtomicBool::new(false));
    {
        let mut hooks = h.backend.hooks();
        let flag = clicked.clone();
        hooks.click = Some(Arc::new(move |fake, (point, window_id)| {
            let flag = flag.clone();
            async move {
                let result = fake.click(point, window_id.as_deref(), &[]).await;
                flag.store(true, Ordering::SeqCst);
                result
            }
            .boxed()
        }));
        let flag = clicked.clone();
        hooks.get_state = Some(Arc::new(move |fake, options| {
            let failed = flag.load(Ordering::SeqCst);
            async move {
                if failed {
                    Err(ComputerBackendError::new("tree unavailable").into())
                } else {
                    fake.get_state(options).await
                }
            }
            .boxed()
        }));
    }
    let arguments =
        json!({ "window_id": WINDOW, "label": "Calculate", "include_screenshot": false });
    let result = h.call("computer_click", arguments.clone()).await;
    assert!(!result.is_error, "{result:?}");

    let plain = harness();
    plain.call("computer_get_state", json!({})).await;
    let unchanged = plain.call("computer_click", arguments).await;
    assert!(!unchanged.is_error, "{unchanged:?}");
    let payload = result_json(&result);
    assert!(payload.get("elementChanges").is_none(), "{payload}");
    assert_eq!(payload, result_json(&unchanged));
    h.dispose().await;
    plain.dispose().await;
}

#[tokio::test]
async fn attaches_one_final_batch_diff_without_per_step_observations() {
    let h = harness();
    h.call("computer_get_state", json!({ "window_id": WINDOW }))
        .await;
    let mark = h.backend.fake.calls().len();
    let result = result_json(
        &h.call(
            "computer_run",
            json!({ "steps": [
                { "type": "set_value", "window_id": WINDOW, "label": "Display", "value": "1" },
                { "type": "set_value", "window_id": WINDOW, "label": "Display", "value": "12" },
            ] }),
        )
        .await,
    );
    assert_subset(
        &result,
        json!({
            "elementChanges": {
                "added": [],
                "removed": [],
                "changed": [{ "label": "Display", "was": "0", "value": "12" }],
            },
        }),
    );
    let steps = result["steps"].as_array().unwrap();
    assert_eq!(steps.len(), 2);
    for step in steps {
        assert!(step["result"].get("elementChanges").is_none(), "{step}");
    }
    assert_eq!(fresh_reads_since(&h, mark), 1);
    h.dispose().await;
}

#[tokio::test]
async fn attaches_a_verified_note_once_per_thread_for_a_scoped_app_read() {
    let h = with_backend(fake_with(FakeComputerBackendOptions {
        windows: Some(vec![ComputerWindow {
            id: "slack-window".into(),
            title: "general - Slack".into(),
            app_name: Some("Slack".into()),
            pid: None,
            bounds: Some(rect(10.0, 10.0, 900.0, 700.0)),
            focused: true,
            keyboard_focused: None,
            active: None,
            minimized: false,
            visible: true,
            space_ids: None,
            current_space_id: None,
            on_current_space: None,
            stacking_index: None,
            occluded_by: None,
        }]),
        ..FakeComputerBackendOptions::default()
    }));
    let arguments = json!({ "window_id": "slack-window" });
    let first = result_json(&h.call("computer_get_state", arguments.clone()).await);
    assert!(
        first["appHint"]
            .as_str()
            .is_some_and(|hint| hint.contains("set_value")),
        "{first}"
    );
    let second = result_json(&h.call("computer_get_state", arguments.clone()).await);
    assert!(second.get("appHint").is_none(), "{second}");
    let other = result_json(
        &h.call_in(
            "computer_get_state",
            arguments,
            &h.context_for("other-thread", None),
        )
        .await,
    );
    assert!(
        other["appHint"]
            .as_str()
            .is_some_and(|hint| hint.contains("set_value")),
        "{other}"
    );
    h.dispose().await;
}

#[tokio::test]
async fn drives_a_second_ordinary_app_without_a_further_prompt() {
    let h = harness();
    for app in ["kcalc", "firefox"] {
        let result = h.call("computer_launch_app", json!({ "app": app })).await;
        assert!(!result.is_error, "{app}: {result:?}");
    }
    h.dispose().await;
}

#[tokio::test]
async fn lists_apps_verifies_state_and_zooms_without_approval_or_dispatch() {
    let h = harness();
    h.decide(ApprovalDecision::Decline);
    let apps = h.call("computer_list_apps", json!({})).await;
    assert!(!apps.is_error, "{apps:?}");
    let pids: Vec<u64> = result_json(&apps)["apps"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|app| app["pid"].as_u64())
        .collect();
    assert!(pids.contains(&1_002), "{pids:?}");

    let verified = h
        .call(
            "computer_verify_state",
            json!({
                "window_id": WINDOW,
                "expect": [{ "window": { "bounds": { "x": 1050, "y": 120, "tolerance_px": 4 } } }],
            }),
        )
        .await;
    assert!(!verified.is_error, "{verified:?}");
    assert_subset(&result_json(&verified), json!({ "status": "satisfied" }));
    assert_eq!(first_args(&h, "verifyState"), [json!(WINDOW)]);

    h.backend.fake.set_verify_satisfied(false);
    let unsatisfied = h
        .call(
            "computer_verify_state",
            json!({
                "window_id": WINDOW,
                "expect": [{ "element": { "selector": { "role": "AXButton" }, "exists": true } }],
            }),
        )
        .await;
    assert_subset(
        &result_json(&unsatisfied),
        json!({ "status": "unsatisfied" }),
    );

    let zoomed = h
        .call(
            "computer_zoom",
            json!({ "window_id": WINDOW, "x": 10, "y": 10, "width": 100, "height": 80 }),
        )
        .await;
    assert!(!zoomed.is_error, "{zoomed:?}");
    assert_eq!(kinds(&zoomed), ["text", "image"]);
    assert!(
        matches!(&zoomed.content[1], McpContent::Image { mime_type, .. } if mime_type == "image/jpeg"),
        "{zoomed:?}"
    );
    assert!(
        result_json(&zoomed)
            .get("screenshot")
            .and_then(|screenshot| screenshot.get("screenshotId"))
            .is_none()
    );

    let out_of_bounds = h
        .call(
            "computer_zoom",
            json!({ "window_id": WINDOW, "x": 400, "y": 0, "width": 100, "height": 80 }),
        )
        .await;
    assert!(out_of_bounds.is_error);
    assert!(cards(&h).is_empty());
    h.dispose().await;
}

#[tokio::test]
async fn moves_a_window_through_approval_and_reports_the_read_back() {
    let h = harness();
    h.decide(ApprovalDecision::Accept);
    let moved = h
        .call(
            "computer_set_window_frame",
            json!({ "window_id": WINDOW, "x": 300, "y": 200, "width": 500, "height": 400 }),
        )
        .await;
    assert!(!moved.is_error, "{moved:?}");
    let cards = cards(&h);
    assert_eq!(cards.len(), 1);
    assert_eq!(cards[0].tool_name, "computer_set_window_frame");
    assert!(
        cards[0].params_display.contains(WINDOW),
        "{}",
        cards[0].params_display
    );
    assert_subset(
        &Value::Array(args_of(&h, "setWindowFrame")),
        json!([[WINDOW, { "x": 300, "y": 200, "width": 500, "height": 400 }]]),
    );
    let windows = h.backend.list_windows().await.unwrap();
    assert_eq!(
        windows
            .iter()
            .find(|window| window.id == WINDOW)
            .and_then(|window| window.bounds),
        Some(rect(300.0, 200.0, 500.0, 400.0))
    );

    let invalid = h
        .call(
            "computer_set_window_frame",
            json!({ "window_id": WINDOW, "x": 0, "y": 0, "width": 0, "height": 400 }),
        )
        .await;
    assert!(invalid.is_error);
    h.dispose().await;
}

#[tokio::test]
async fn asks_approval_before_invoking_menus_and_force_quitting_dispatching_nothing_when_refused() {
    let h = harness();
    h.decide(ApprovalDecision::Decline);
    for (name, arguments) in [
        (
            "computer_invoke_menu",
            json!({ "window_id": WINDOW, "path": ["File", "Save"] }),
        ),
        ("computer_kill_app", json!({ "window_id": WINDOW })),
    ] {
        let context = ComputerToolContext {
            turn_id: Some(format!("turn-{name}")),
            ..h.context()
        };
        let refused = h.call_in(name, arguments, &context).await;
        assert!(refused.is_error, "{name}: {refused:?}");
    }
    assert!(h.backend.calls_for("invokeMenu").is_empty());
    assert!(h.backend.calls_for("killApp").is_empty());
    let names: Vec<String> = cards(&h).into_iter().map(|card| card.tool_name).collect();
    assert_eq!(names, ["computer_invoke_menu", "computer_kill_app"]);
    h.dispose().await;
}

#[tokio::test]
async fn resolves_kill_apps_window_to_its_owning_pid_before_dispatch() {
    let h = harness();
    h.decide(ApprovalDecision::Accept);
    let killed = h
        .call("computer_kill_app", json!({ "window_id": WINDOW }))
        .await;
    assert!(!killed.is_error, "{killed:?}");
    assert_eq!(first_args(&h, "killApp"), [json!(1_002)]);
    let gone = h
        .call("computer_kill_app", json!({ "window_id": WINDOW }))
        .await;
    assert!(gone.is_error);
    assert_eq!(h.backend.calls_for("killApp").len(), 1);
    h.dispose().await;
}

#[tokio::test]
async fn drives_the_app_a_window_targeted_mutation_names_without_asking() {
    let h = harness();
    h.decide(ApprovalDecision::Accept);
    h.call("computer_launch_app", json!({ "app": "TextEdit" }))
        .await;
    let moved = h
        .call(
            "computer_set_window_frame",
            json!({ "window_id": WINDOW, "x": 0, "y": 0, "width": 500, "height": 400 }),
        )
        .await;
    assert!(!moved.is_error, "{moved:?}");
    h.dispose().await;
}

#[tokio::test]
async fn runs_the_window_management_steps_through_computer_run_in_order() {
    let h = harness();
    h.decide(ApprovalDecision::Accept);
    let result = h
        .call(
            "computer_run",
            json!({ "steps": [
                {
                    "type": "set_window_frame",
                    "window_id": WINDOW,
                    "x": 50,
                    "y": 60,
                    "width": 500,
                    "height": 400,
                },
                { "type": "invoke_menu", "window_id": WINDOW, "path": ["File", "Save"] },
                { "type": "kill_app", "window_id": WINDOW },
            ] }),
        )
        .await;
    assert!(!result.is_error, "{result:?}");
    let payload = result_json(&result);
    let summary: Vec<Value> = payload["steps"]
        .as_array()
        .unwrap()
        .iter()
        .map(|entry| json!([entry["step"], entry["type"], entry["ok"]]))
        .collect();
    assert_eq!(
        summary,
        [
            json!([0, "set_window_frame", true]),
            json!([1, "invoke_menu", true]),
            json!([2, "kill_app", true]),
        ]
    );
    assert_eq!(payload["completed"], json!(3));
    assert_eq!(first_args(&h, "setWindowFrame"), [json!(WINDOW)]);
    let paths: Vec<Value> = h
        .backend
        .calls_for("invokeMenu")
        .into_iter()
        .map(|call| call.args[1].clone())
        .collect();
    assert_eq!(paths, [json!(["File", "Save"])]);
    assert_eq!(first_args(&h, "killApp"), [json!(1_002)]);
    h.dispose().await;
}

#[tokio::test]
async fn invokes_a_windowless_apps_menu_from_its_live_pid_attributed_to_the_app() {
    let h = with_apps(vec![
        app(6_001, "Helium", "net.imput.helium", false),
        app(1_001, "Terminal", "org.kde.konsole", true),
        app(1_002, "Calculator", "org.kde.kcalc", false),
    ]);
    h.decide(ApprovalDecision::Accept);
    let result = h
        .call(
            "computer_invoke_menu",
            json!({ "app": "Helium", "path": ["File", "New Window"] }),
        )
        .await;
    assert!(!result.is_error, "{result:?}");
    let dispatched = h.backend.calls_for("invokeMenu").pop().unwrap().args[0].clone();
    assert_eq!(dispatched, json!({ "pid": 6_001 }));
    assert!(dispatched.get("window_id").is_none());
    assert!(result_json(&result).get("windowId").is_none());
    assert_eq!(cards(&h).len(), 1);
    h.dispose().await;
}

#[tokio::test]
async fn refuses_a_menu_call_that_names_no_target_or_two_different_routes() {
    let h = harness();
    h.decide(ApprovalDecision::Accept);
    let none = h
        .call("computer_invoke_menu", json!({ "path": ["File"] }))
        .await;
    assert!(none.is_error);
    assert!(text(&none).contains("Name the target one way"), "{none:?}");
    let two = h
        .call(
            "computer_invoke_menu",
            json!({ "window_id": "fake-terminal", "app": "Terminal", "path": ["File"] }),
        )
        .await;
    assert!(two.is_error);
    assert!(text(&two).contains("exactly one"), "{two:?}");
    assert!(h.backend.calls_for("invokeMenu").is_empty());
    h.dispose().await;
}

#[tokio::test]
async fn runs_a_windowless_app_menu_step_through_computer_run() {
    let h = with_apps(vec![
        app(6_001, "Helium", "net.imput.helium", false),
        app(1_001, "Terminal", "org.kde.konsole", true),
    ]);
    h.decide(ApprovalDecision::Accept);
    let result = h
        .call(
            "computer_run",
            json!({ "steps": [
                { "type": "invoke_menu", "app": "Helium", "path": ["File", "New Window"] },
            ] }),
        )
        .await;
    assert!(!result.is_error, "{result:?}");
    assert_eq!(result_json(&result)["completed"], json!(1));
    assert_eq!(
        args_of(&h, "invokeMenu").pop(),
        Some(json!([{ "pid": 6_001 }, ["File", "New Window"]]))
    );
    h.dispose().await;
}

#[tokio::test]
async fn refuses_run_steps_with_missing_or_oversized_arguments_whole() {
    let h = harness();
    h.decide(ApprovalDecision::Accept);
    for steps in [
        json!([{
            "type": "set_window_frame",
            "window_id": WINDOW,
            "x": 0,
            "y": 0,
            "width": 0,
            "height": 5,
        }]),
        json!([{ "type": "set_window_frame", "x": 0, "y": 0, "width": 5, "height": 5 }]),
        json!([{ "type": "invoke_menu", "window_id": WINDOW, "path": [] }]),
        json!([{ "type": "invoke_menu", "window_id": WINDOW }]),
        json!([{ "type": "kill_app" }]),
    ] {
        let result = h
            .call("computer_run", json!({ "steps": steps.clone() }))
            .await;
        assert!(result.is_error, "{steps}: {result:?}");
    }
    assert!(h.backend.calls_for("setWindowFrame").is_empty());
    assert!(h.backend.calls_for("invokeMenu").is_empty());
    assert!(h.backend.calls_for("killApp").is_empty());
    h.dispose().await;
}
