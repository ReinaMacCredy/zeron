use super::*;

use std::sync::Arc;
use std::sync::Mutex;

use futures::FutureExt;
use zeron_proto::computer::{
    ComputerLaunchAppResult, ComputerLaunchWindowReason, ComputerLaunchWindowStatus, ComputerPoint,
    ComputerTarget, ComputerUiNode, McpContent,
};

use crate::computer::backend::{
    COMPUTER_ACTION_OBSERVATION_MAX_DIMENSION, ComputerBackend, ComputerBackendActionResult,
    ComputerBackendError, ComputerCaptureRequest, ComputerResolvedTarget,
};
use crate::computer::model_desktop_observation::is_model_desktop_observation_active;
use crate::computer::tools::computer_tool_requires_approval;
use crate::computer::tools::guidance::computer_tool_instructions;

fn content_types(result: &McpToolResult) -> Vec<&'static str> {
    result
        .content
        .iter()
        .map(|entry| match entry {
            McpContent::Text { .. } => "text",
            McpContent::Image { .. } => "image",
        })
        .collect()
}

fn text_of(result: &McpToolResult) -> String {
    result.first_text().unwrap_or_default().to_owned()
}

fn description(h: &Harness, tool: &str) -> String {
    h.tools
        .entry(tool)
        .map(|entry| entry.definition.description)
        .unwrap_or_default()
}

fn input_schema(h: &Harness, tool: &str) -> Value {
    h.tools
        .entry(tool)
        .map(|entry| entry.definition.input_schema)
        .unwrap_or(Value::Null)
}

fn property_description(h: &Harness, tool: &str, property: &str) -> String {
    input_schema(h, tool)["properties"][property]["description"]
        .as_str()
        .unwrap_or_default()
        .to_owned()
}

fn window_id_description(h: &Harness, tool: &str) -> String {
    property_description(h, tool, "window_id")
}

fn last_args(h: &Harness, method: &str) -> Vec<Value> {
    h.backend
        .calls_for(method)
        .last()
        .map(|call| call.args.clone())
        .unwrap_or_default()
}

fn assert_subset(actual: &Value, expected: &Value, path: &str) {
    match expected {
        Value::Object(fields) => {
            for (key, value) in fields {
                let next = format!("{path}.{key}");
                let Some(found) = actual.get(key) else {
                    panic!("missing {next} in {actual}");
                };
                assert_subset(found, value, &next);
            }
        }
        Value::Number(number) => assert_eq!(
            actual.as_f64(),
            number.as_f64(),
            "{path}: {actual} != {expected}"
        ),
        _ => assert_eq!(actual, expected, "{path}"),
    }
}

fn point(x: f64, y: f64) -> Value {
    json!({ "x": x, "y": y })
}

fn assert_point(actual: &Value, x: f64, y: f64) {
    assert_subset(actual, &point(x, y), "point");
}

fn launch_result(
    app: String,
    window_status: Option<ComputerLaunchWindowStatus>,
    window_reason: Option<ComputerLaunchWindowReason>,
) -> ComputerLaunchAppResult {
    ComputerLaunchAppResult {
        computer_id: "desktop".into(),
        app,
        focus_changed_during_launch: None,
        pid: None,
        window_status,
        window_reason,
        resolved_command: None,
        window: None,
    }
}

fn null_window_launch(
    backend: &Scripted,
    window_status: Option<ComputerLaunchWindowStatus>,
    window_reason: Option<ComputerLaunchWindowReason>,
) {
    backend.hooks().launch_app = Some(Arc::new(move |_, (app, _, _)| {
        let result = launch_result(app, window_status, window_reason);
        async move { Ok(result) }.boxed()
    }));
}

#[tokio::test]
async fn bounds_active_tool_context_and_directs_deferred_discovery_to_the_next_small_set() {
    let h = harness_with(HarnessOptions {
        dialect: Some(ComputerAgentDialect::Macos),
        ..HarnessOptions::default()
    });
    let definitions = h.tools.catalog();
    let descriptor_bytes = serde_json::to_string(&definitions).unwrap().len();
    assert!(descriptor_bytes <= 29_400, "{descriptor_bytes}");
    let sized = |name: &str| {
        serde_json::to_string(definitions.iter().find(|tool| tool.name == name).unwrap())
            .unwrap()
            .len()
    };
    assert!(sized("computer_run") <= 2_000, "{}", sized("computer_run"));
    assert!(
        sized("computer_inspect") <= 750,
        "{}",
        sized("computer_inspect")
    );
    let notes = computer_tool_instructions();
    assert!(notes.len() <= 3_800, "{}", notes.len());
    assert!(descriptor_bytes + notes.len() <= 33_200);
    for needle in [
        "never list the whole catalog",
        "look them up by exact name",
        "computer_launch_app",
        "foreground_not_requested",
        "same_pid_keyboard_ambiguity",
        "element_outside_target_window",
        "input_target_unavailable",
        "repeated_unverified_action",
    ] {
        assert!(notes.contains(needle), "{needle}");
    }
    for absent in [
        "relaunch visible",
        "unhide with computer_set_app_visibility",
        "computer_recording",
        "computer_replay",
    ] {
        assert!(!notes.contains(absent), "{absent}");
    }
    h.dispose().await;
}

#[tokio::test]
async fn presents_launch_hidden_as_an_explicit_posture_not_an_invisible_workspace_doctrine() {
    let h = harness();
    let hidden = property_description(&h, "computer_launch_app", "hidden");
    assert!(hidden.contains("Explicitly hide"), "{hidden}");
    assert!(
        hidden.contains("never authorizes foreground input"),
        "{hidden}"
    );
    assert!(!hidden.contains("Defaults to true"));
    assert!(!hidden.contains("agent launches stay invisible"));
    assert!(!hidden.contains("invisible workspace"));
    h.dispose().await;
}

#[tokio::test]
async fn attaches_list_windows_guidance_to_a_null_window_launch_result() {
    let h = harness();
    null_window_launch(&h.backend, None, None);
    let result = h
        .call(
            "computer_launch_app",
            json!({ "app": "Aside", "wait_for_window": false }),
        )
        .await;
    assert!(!result.is_error, "{result:?}");
    let text = result_json(&result).to_string();
    assert!(text.contains("computer_list_windows"), "{text}");
    assert!(text.contains("never launch again"), "{text}");
    assert!(!text.contains("relaunch visible"), "{text}");
    assert!(text.contains("computer_browser_prepare"), "{text}");
    h.dispose().await;
}

#[tokio::test]
async fn returns_the_same_no_relaunch_guidance_for_batched_launches() {
    let h = harness();
    null_window_launch(&h.backend, None, None);
    let result = h
        .call(
            "computer_run",
            json!({ "steps": [{ "type": "launch_app", "app": "Helium", "wait_for_window": false }] }),
        )
        .await;
    assert!(!result.is_error, "{result:?}");
    let text = result_json(&result).to_string();
    assert!(text.contains("never launch again automatically"), "{text}");
    assert!(text.contains("not_checked"), "{text}");
    h.dispose().await;
}

#[tokio::test]
async fn stops_a_batch_after_a_delivered_launch_with_no_usable_window() {
    let h = harness();
    null_window_launch(
        &h.backend,
        Some(ComputerLaunchWindowStatus::NoUsableWindow),
        Some(ComputerLaunchWindowReason::OffSpace),
    );
    let result = h
        .call(
            "computer_run",
            json!({ "steps": [
                { "type": "launch_app", "app": "Helium", "wait_for_window": false, "continue_on_error": true },
                { "type": "press_key", "key": "enter" },
            ] }),
        )
        .await;
    assert_subset(
        &result_json(&result),
        &json!({ "stopped": true, "stoppedReason": "no_usable_window", "completed": 1 }),
        "run",
    );
    assert!(h.backend.calls_for("pressKey").is_empty());
    h.dispose().await;
}

#[tokio::test]
async fn leaves_an_off_screen_launch_result_without_unhide_choreography() {
    let h = harness();
    let result = h
        .call(
            "computer_launch_app",
            json!({ "app": "Helium", "hidden": true, "wait_for_window": false }),
        )
        .await;
    assert!(!result.is_error, "{result:?}");
    let text = result_json(&result).to_string();
    assert!(!text.contains("unhide"), "{text}");
    assert!(!text.contains("relaunch visible"), "{text}");
    assert!(!text.contains("set_app_visibility"), "{text}");
    h.dispose().await;
}

#[tokio::test]
async fn reserves_model_observation_authority_for_explicit_perception_tools() {
    let h = harness();
    let observations: Arc<Mutex<Vec<bool>>> = Arc::default();
    {
        let mut hooks = h.backend.hooks();
        let seen = observations.clone();
        hooks.get_state = Some(Arc::new(move |fake, options| {
            seen.lock()
                .unwrap()
                .push(is_model_desktop_observation_active());
            async move { fake.get_state(options).await }.boxed()
        }));
        let seen = observations.clone();
        hooks.capture_screenshot = Some(Arc::new(move |fake, request| {
            seen.lock()
                .unwrap()
                .push(is_model_desktop_observation_active());
            async move { fake.capture_screenshot(request).await }.boxed()
        }));
    }
    for (name, arguments) in [
        (
            "computer_get_state",
            json!({ "window_id": "fake-calculator" }),
        ),
        (
            "computer_screenshot",
            json!({ "window_id": "fake-calculator" }),
        ),
        (
            "computer_wait",
            json!({
                "window_id": "fake-calculator",
                "label": "Display",
                "duration_ms": 0,
                "include_screenshot": false,
            }),
        ),
    ] {
        observations.lock().unwrap().clear();
        let result = h.call(name, arguments).await;
        assert!(!result.is_error, "{name}: {result:?}");
        let seen = observations.lock().unwrap().clone();
        assert!(!seen.is_empty(), "{name}");
        assert!(seen.iter().all(|active| *active), "{name}: {seen:?}");
        assert!(!is_model_desktop_observation_active());
    }
    observations.lock().unwrap().clear();
    let set = h
        .call(
            "computer_set_value",
            json!({ "label": "Display", "value": "468" }),
        )
        .await;
    assert!(!set.is_error, "{set:?}");
    let seen = observations.lock().unwrap().clone();
    assert!(!seen.is_empty());
    assert!(seen.iter().all(|active| !active), "{seen:?}");
    h.dispose().await;
}

#[tokio::test]
async fn describes_exact_targeting_separately_from_foreground_promotion() {
    let h = harness();
    let notes = computer_tool_instructions();
    assert!(notes.contains("Act by ref (or exact label plus role)"));
    assert!(window_id_description(&h, "computer_press_key").contains("does not activate it"));
    assert!(
        window_id_description(&h, "computer_click")
            .contains("Exact window for label or x/y targeting")
    );
    h.dispose().await;
}

#[tokio::test]
async fn covers_routine_foreground_delivery_with_the_active_task_consent_on_macos() {
    let h = harness_with(HarnessOptions {
        dialect: Some(ComputerAgentDialect::Macos),
        ..HarnessOptions::default()
    });
    let notes = computer_tool_instructions();
    assert!(notes.contains("the user's visible-use request or direct confirmation"));
    assert!(!notes.contains("without bringing it to the front"));
    assert!(
        window_id_description(&h, "computer_click")
            .contains("Exact window for label or x/y targeting")
    );
    assert!(window_id_description(&h, "computer_type_text").contains("does not activate it"));
    assert!(
        description(&h, "computer_activate_window")
            .contains("only when the user's own task text asked to see the screen")
    );
    assert!(!description(&h, "computer_list_windows").contains("into view automatically"));
    assert!(description(&h, "computer_get_state").contains("primary display"));
    assert!(window_id_description(&h, "computer_get_state").contains("any requested screenshot"));
    assert!(
        description(&h, "computer_screenshot")
            .contains("Rectangular region capture is unavailable")
    );
    let mut properties: Vec<String> = input_schema(&h, "computer_screenshot")["properties"]
        .as_object()
        .unwrap()
        .keys()
        .cloned()
        .collect();
    properties.sort();
    assert_eq!(properties, ["max_dimension", "window_id"]);
    h.dispose().await;
}

#[tokio::test]
async fn spells_out_all_three_delivery_verdicts_once_in_the_shared_notes() {
    let h = harness();
    let notes = computer_tool_instructions();
    for needle in [
        "\"verified\"",
        "\"dispatched-unknown\"",
        "\"not-dispatched\"",
        "never replay it",
    ] {
        assert!(notes.contains(needle), "{needle}");
    }
    for name in ["computer_type_text", "computer_press_key"] {
        let text = description(&h, name);
        assert!(text.contains("delivery.verified"), "{name}");
        assert!(text.contains("never replay an uncertain action"), "{name}");
    }
    h.dispose().await;
}

#[tokio::test]
async fn carries_the_callers_name_to_the_backend_that_draws_the_agent_cursor() {
    let h = harness();
    let luna = h.context_for(THREAD, Some("Luna"));
    h.see_in(&luna).await;
    h.call_in("computer_click", json!({ "x": 4, "y": 4 }), &luna)
        .await;
    assert_eq!(h.backend.logged("setDrivingAgent"), [json!("Luna")]);
    h.call_in("computer_press_key", json!({ "key": "enter" }), &luna)
        .await;
    assert_eq!(h.backend.logged("setDrivingAgent"), [json!("Luna")]);
    h.dispose().await;
}

#[tokio::test]
async fn does_not_force_provider_preloading_and_keeps_every_computer_tool_capability_gated() {
    let h = harness();
    for entry in h.tools.entries() {
        let definition = serde_json::to_value(&entry.definition).unwrap();
        assert!(definition.get("_meta").is_none(), "{}", entry.name());
    }
    assert_subset(
        h.tools
            .entry("computer_click")
            .and_then(|entry| entry.definition.annotations)
            .as_ref()
            .unwrap(),
        &json!({ "readOnlyHint": false }),
        "annotations",
    );
    h.dispose().await;
}

#[tokio::test]
async fn filters_discovery_to_the_requested_app_without_losing_availability_or_choosing_a_window() {
    let h = harness();
    let mut windows = h.backend.fake.list_windows().await.unwrap();
    let first = windows[0].clone();
    windows.push(zeron_proto::computer::ComputerWindow {
        id: "same-app-second-window".into(),
        ..first.clone()
    });
    h.backend.fake.emit_windows_changed(windows.clone());
    let all = result_json(&h.call("computer_list_windows", json!({})).await);
    assert_eq!(all["windows"].as_array().map(Vec::len), Some(windows.len()));
    let filtered = result_json(
        &h.call(
            "computer_list_windows",
            json!({ "app": first.app_name.clone().unwrap().to_uppercase() }),
        )
        .await,
    );
    let ids: Vec<&str> = filtered["windows"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|window| window["id"].as_str())
        .collect();
    assert_eq!(ids, [first.id.as_str(), "same-app-second-window"]);
    assert!(
        filtered
            .get("availability")
            .is_some_and(|value| !value.is_null())
    );
    let missing = result_json(
        &h.call("computer_list_windows", json!({ "app": "missing-app" }))
            .await,
    );
    assert_eq!(missing["windows"], json!([]));
    h.dispose().await;
}

#[tokio::test]
async fn refreshes_computer_routing_guidance_per_thread_without_repeating_every_call() {
    let h = harness();
    let mut guided = Vec::new();
    for _ in 0..11 {
        let payload = result_json(&h.call("computer_list_windows", json!({})).await);
        guided.push(
            payload["toolGuidance"]
                .as_str()
                .is_some_and(|text| text.contains("Computer routing reminder")),
        );
    }
    assert_eq!(
        guided,
        [
            true, false, false, false, false, false, false, false, false, false, true
        ]
    );
    let other = result_json(
        &h.call_in(
            "computer_list_windows",
            json!({}),
            &h.context_for("other-thread", None),
        )
        .await,
    );
    assert!(
        other["toolGuidance"]
            .as_str()
            .is_some_and(|text| text.contains("Computer routing reminder"))
    );
    h.dispose().await;
}

#[tokio::test]
async fn returns_perception_payloads_and_preserves_screenshot_image_content() {
    let h = harness();
    let list = h.call("computer_list_windows", json!({})).await;
    assert!(!list.is_error);
    let state = h
        .call(
            "computer_get_state",
            json!({ "include_screenshot": true, "include_text": true }),
        )
        .await;
    assert_eq!(content_types(&state), ["text", "image"]);
    assert!(state.content.iter().any(|entry| matches!(
        entry,
        McpContent::Image { mime_type, .. } if mime_type == "image/png"
    )));
    let text = text_of(&state);
    let parsed: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(text, serde_json::to_string(&parsed).unwrap());
    assert_subset(
        &parsed,
        &json!({ "screenshot": {
            "screenshotId": "shot-1",
            "width": 1_536,
            "height": 864,
            "region": { "x": 0, "y": 0, "width": 1_920, "height": 1_080 },
            "scale": 0.8,
        } }),
        "state",
    );
    h.dispose().await;
}

#[tokio::test]
async fn lists_actionable_elements_on_every_get_state_without_needing_include_text() {
    let h = harness();
    let payload = result_json(&h.call("computer_get_state", json!({})).await);
    let elements = payload["elements"].as_array().expect("elements");
    let roles: Vec<&str> = elements
        .iter()
        .filter_map(|element| element["role"].as_str())
        .collect();
    assert!(roles.contains(&"button"), "{roles:?}");
    assert!(roles.contains(&"text-field"), "{roles:?}");
    for element in elements {
        assert!(
            element["label"]
                .as_str()
                .is_some_and(|label| !label.is_empty()),
            "{element}"
        );
        assert!(element.get("windowId").is_none(), "{element}");
    }
    assert_eq!(payload["elementWindowId"], json!("fake-calculator"));
    assert!(payload.get("text").is_none());
    let with_text = result_json(
        &h.call("computer_get_state", json!({ "include_text": true }))
            .await,
    );
    assert!(
        with_text["text"]
            .as_str()
            .is_some_and(|text| text.contains("button"))
    );
    h.dispose().await;
}

fn node(value: Value) -> ComputerUiNode {
    serde_json::from_value(value).unwrap()
}

#[tokio::test]
async fn reports_an_elements_digest_that_omits_nothing_silently_when_truncated() {
    let children: Vec<Value> = (0..90)
        .map(|index| {
            json!({
                "role": "push button",
                "label": format!("Button {index}"),
                "value": null,
                "description": null,
                "frame": { "x": 0, "y": 0, "width": 80, "height": 30 },
                "activationPoint": null,
                "onScreen": true,
                "windowId": "w1",
                "children": [],
            })
        })
        .collect();
    let root = node(json!({
        "role": "desktop",
        "label": null,
        "value": null,
        "description": null,
        "frame": { "x": 0, "y": 0, "width": 1_920, "height": 1_080 },
        "activationPoint": null,
        "onScreen": true,
        "windowId": null,
        "children": children,
    }));
    let h = harness_with(HarnessOptions {
        backend: fake_with(FakeComputerBackendOptions {
            root: Some(root),
            ..FakeComputerBackendOptions::default()
        }),
        ..HarnessOptions::default()
    });
    let payload = result_json(&h.call("computer_get_state", json!({})).await);
    let elements = payload["elements"].as_array().unwrap();
    assert_eq!(elements.len(), 60);
    assert_eq!(payload["elementsTruncated"], json!(true));
    assert_eq!(payload["elementWindowId"], json!("w1"));
    assert!(
        elements
            .iter()
            .all(|element| element.get("windowId").is_none())
    );
    h.dispose().await;
}

#[tokio::test]
async fn names_label_contains_when_a_truncated_tree_misses_the_label() {
    let root = ComputerUiNode {
        truncated: Some(true),
        ..Scripted::default_fake().fake.root()
    };
    let h = harness_with(HarnessOptions {
        backend: fake_with(FakeComputerBackendOptions {
            root: Some(root),
            ..FakeComputerBackendOptions::default()
        }),
        ..HarnessOptions::default()
    });
    let result = h
        .call("computer_click", json!({ "label": "Missing control" }))
        .await;
    assert!(result.is_error);
    assert!(text_of(&result).contains("label_contains"), "{result:?}");
    h.dispose().await;
}

#[tokio::test]
async fn tells_the_model_to_point_in_screenshot_pixels_and_never_to_convert_them() {
    let h = harness();
    for name in ["computer_get_state", "computer_screenshot"] {
        let text = description(&h, name);
        assert!(text.contains("screenshotId"), "{name}");
        assert!(
            text.contains("pass x/y as pixel coordinates in that image"),
            "{name}"
        );
        assert!(!text.contains("region.x"), "{name}");
    }
    for name in [
        "computer_click",
        "computer_move_cursor",
        "computer_drag",
        "computer_scroll",
    ] {
        let text = description(&h, name);
        assert!(text.contains("never desktop coordinates"), "{name}");
        assert!(!text.contains("global desktop coordinates"), "{name}");
        assert!(
            input_schema(&h, name).to_string().contains("screenshot_id"),
            "{name}"
        );
    }
    assert!(description(&h, "computer_get_screen_size").contains("Informational only"));
    h.dispose().await;
}

#[tokio::test]
async fn tells_the_model_the_cursor_is_overlay_only_and_no_background_hover_exists() {
    let h = harness();
    let text = description(&h, "computer_move_cursor");
    assert!(text.contains("does not deliver hover events"));
    assert!(text.contains("a real background hover is not available on this backend"));
    assert!(text.contains("real system pointer never moves"));
    h.dispose().await;
}

#[tokio::test]
async fn tells_the_model_how_to_click_a_window_another_window_covers() {
    let h = harness();
    let list = description(&h, "computer_list_windows");
    for needle in ["stackingIndex", "occludedBy", "window_id"] {
        assert!(list.contains(needle), "{needle}");
    }
    for name in ["computer_click", "computer_move_cursor", "computer_drag"] {
        assert!(
            input_schema(&h, name)
                .to_string()
                .contains("Exact window for label or x/y targeting"),
            "{name}"
        );
    }
    h.dispose().await;
}

#[tokio::test]
async fn keeps_a_window_scoped_state_capture_bound_to_its_native_target() {
    let h = harness();
    h.backend.hooks().get_state = Some(Arc::new(|fake, options| {
        async move {
            let mut state = fake.get_state(options).await?;
            let mut screenshot = fake
                .capture_screenshot(ComputerCaptureRequest::Window {
                    window_id: "fake-calculator".into(),
                    max_dimension: None,
                })
                .await?;
            screenshot.window_id = Some("fake-calculator".into());
            state.screenshot = Some(screenshot);
            Ok(state)
        }
        .boxed()
    }));
    let observed = h
        .call(
            "computer_get_state",
            json!({ "window_id": "fake-calculator", "include_screenshot": true }),
        )
        .await;
    assert_eq!(
        result_json(&observed)["screenshot"]["windowId"],
        json!("fake-calculator")
    );
    let mismatched = h
        .call(
            "computer_click",
            json!({ "window_id": "fake-editor", "x": 5, "y": 5, "include_screenshot": false }),
        )
        .await;
    assert!(mismatched.is_error);
    assert!(h.backend.calls_for("click").is_empty());
    let clicked = h
        .call(
            "computer_click",
            json!({ "x": 5, "y": 5, "include_screenshot": false }),
        )
        .await;
    assert!(!clicked.is_error, "{clicked:?}");
    assert_point(&last_args(&h, "click")[0], 1_055.0, 125.0);
    h.dispose().await;
}

#[tokio::test]
async fn zooms_into_a_window_and_reads_the_next_coordinates_in_that_windows_pixels() {
    let h = harness();
    let result = h
        .call(
            "computer_screenshot",
            json!({ "window_id": "fake-calculator" }),
        )
        .await;
    assert!(!result.is_error, "{result:?}");
    assert_eq!(content_types(&result), ["text", "image"]);
    assert!(result.content.iter().any(|entry| matches!(
        entry,
        McpContent::Image { mime_type, .. } if mime_type == "image/png"
    )));
    assert_subset(
        &result_json(&result),
        &json!({ "screenshot": {
            "screenshotId": "shot-1",
            "windowId": "fake-calculator",
            "mimeType": "image/png",
            "width": 420,
            "height": 620,
            "region": { "x": 1_050, "y": 120, "width": 420, "height": 620 },
            "scale": 1,
        } }),
        "zoom",
    );
    assert_eq!(
        last_args(&h, "captureScreenshot")[0],
        json!({ "kind": "window", "windowId": "fake-calculator" })
    );

    h.call(
        "computer_click",
        json!({ "x": 5, "y": 5, "include_screenshot": false }),
    )
    .await;
    assert_point(&last_args(&h, "click")[0], 1_055.0, 125.0);

    let outside = h
        .call(
            "computer_click",
            json!({ "x": 500, "y": 10, "include_screenshot": false }),
        )
        .await;
    assert!(outside.is_error);
    let error = result_json(&outside)["error"].clone();
    assert_eq!(
        error["code"],
        json!("computer_target_offscreen"),
        "{outside:?}"
    );
    assert!(
        error["message"]
            .as_str()
            .is_some_and(|text| text.contains("420x620 screenshot shot-1")),
        "{error}"
    );
    assert_eq!(h.backend.calls_for("click").len(), 1);

    let workspace = h.see().await;
    assert_eq!(workspace["screenshotId"], json!("shot-2"));
    h.call(
        "computer_click",
        json!({ "x": 5, "y": 5, "screenshot_id": "shot-1", "include_screenshot": false }),
    )
    .await;
    assert_point(&last_args(&h, "click")[0], 1_055.0, 125.0);
    h.call(
        "computer_click",
        json!({ "x": 8, "y": 8, "include_screenshot": false }),
    )
    .await;
    assert_point(&last_args(&h, "click")[0], 10.0, 10.0);

    let unknown = h
        .call(
            "computer_click",
            json!({ "x": 7, "y": 9, "screenshot_id": "shot-9" }),
        )
        .await;
    let error = result_json(&unknown)["error"].clone();
    assert_eq!(
        error["code"],
        json!("computer_target_not_found"),
        "{unknown:?}"
    );
    assert!(
        error["message"]
            .as_str()
            .is_some_and(|text| text.contains("shot-1, shot-2")),
        "{error}"
    );
    h.dispose().await;
}

#[tokio::test]
async fn refuses_to_point_before_the_conversation_has_seen_a_screenshot() {
    let h = harness();
    let blind = h.call("computer_click", json!({ "x": 4, "y": 4 })).await;
    assert!(blind.is_error);
    let error = result_json(&blind)["error"].clone();
    assert_eq!(error["code"], json!("computer_target_invalid"), "{blind:?}");
    assert!(
        error["message"]
            .as_str()
            .is_some_and(|text| text.contains("computer_screenshot")),
        "{error}"
    );
    let scroll = h
        .call("computer_scroll", json!({ "delta_x": 0, "delta_y": 100 }))
        .await;
    assert!(scroll.is_error);
    assert!(h.backend.calls_for("click").is_empty());
    assert!(h.backend.calls_for("scroll").is_empty());
    let by_label = h
        .call(
            "computer_click",
            json!({ "label": "Calculate", "role": "button" }),
        )
        .await;
    assert!(!by_label.is_error, "{by_label:?}");
    assert_eq!(h.backend.calls_for("click").len(), 1);
    h.dispose().await;
}

#[tokio::test]
async fn keeps_each_conversations_screenshots_apart() {
    let h = harness();
    h.see_in(&h.context_for("thread-a", None)).await;
    let blind = h
        .call_in(
            "computer_click",
            json!({ "x": 1, "y": 1 }),
            &h.context_for("thread-b", None),
        )
        .await;
    assert_eq!(
        result_json(&blind)["error"]["code"],
        json!("computer_target_invalid"),
        "{blind:?}"
    );
    h.dispose().await;
}

#[tokio::test]
async fn zooms_into_a_region_and_maps_points_and_scroll_distances_through_its_scale() {
    let h = harness();
    h.see().await;
    let result = h
        .call(
            "computer_screenshot",
            json!({ "x": 840, "y": 96, "width": 320, "height": 640, "max_dimension": 400 }),
        )
        .await;
    assert!(!result.is_error, "{result:?}");
    let payload = result_json(&result);
    assert_subset(
        &payload,
        &json!({ "screenshot": {
            "width": 200,
            "height": 400,
            "region": { "x": 1_050, "y": 120, "width": 400, "height": 800 },
            "scale": 0.5,
        } }),
        "zoom",
    );
    assert!(payload.get("windowId").is_none());
    let capture = &last_args(&h, "captureScreenshot")[0];
    assert_subset(
        capture,
        &json!({
            "kind": "region",
            "region": { "x": 1_050, "y": 120, "width": 400, "height": 800 },
            "maxDimension": 400,
        }),
        "capture",
    );
    assert_eq!(capture.as_object().map(|map| map.len()), Some(3));

    h.call(
        "computer_click",
        json!({ "x": 100, "y": 100, "include_screenshot": false }),
    )
    .await;
    assert_point(&last_args(&h, "click")[0], 1_250.0, 320.0);
    h.call(
        "computer_scroll",
        json!({ "x": 100, "y": 100, "delta_x": 0, "delta_y": 40, "include_screenshot": false }),
    )
    .await;
    let scroll = last_args(&h, "scroll");
    assert_eq!(scroll.len(), 3, "{scroll:?}");
    assert_point(&scroll[0], 1_250.0, 320.0);
    assert_eq!(scroll[1].as_f64(), Some(0.0));
    assert_eq!(scroll[2].as_f64(), Some(80.0));
    h.call(
        "computer_drag",
        json!({
            "from": { "x": 0, "y": 0 },
            "to": { "x": 100, "y": 100 },
            "include_screenshot": false,
        }),
    )
    .await;
    let drag = last_args(&h, "drag");
    assert_point(&drag[0], 1_050.0, 120.0);
    assert_point(&drag[1], 1_250.0, 320.0);
    h.call(
        "computer_screenshot",
        json!({ "x": 100, "y": 300, "width": 200, "height": 200 }),
    )
    .await;
    let capture = &last_args(&h, "captureScreenshot")[0];
    assert_subset(
        capture,
        &json!({
            "kind": "region",
            "region": { "x": 1_250, "y": 720, "width": 200, "height": 200 },
        }),
        "capture",
    );
    assert!(capture.get("maxDimension").is_none(), "{capture}");
    h.dispose().await;
}

#[tokio::test]
async fn refuses_an_ambiguous_or_incomplete_screenshot_request_without_capturing() {
    let h = harness();
    let both = h
        .call(
            "computer_screenshot",
            json!({ "window_id": "fake-calculator", "x": 0 }),
        )
        .await;
    assert!(both.is_error);
    assert!(text_of(&both).contains("never both"), "{both:?}");
    let partial = h
        .call(
            "computer_screenshot",
            json!({ "x": 10, "y": 20, "width": 30 }),
        )
        .await;
    assert!(partial.is_error);
    assert!(text_of(&partial).contains("height"), "{partial:?}");
    let empty = h
        .call(
            "computer_screenshot",
            json!({ "x": 10, "y": 20, "width": 0, "height": 30 }),
        )
        .await;
    assert!(empty.is_error);
    assert!(text_of(&empty).contains("greater than zero"), "{empty:?}");
    assert!(h.backend.calls_for("captureScreenshot").is_empty());
    h.dispose().await;
}

#[tokio::test]
async fn captures_the_focused_window_when_called_without_a_target() {
    let h = harness();
    let result = h.call("computer_screenshot", json!({})).await;
    assert!(!result.is_error, "{result:?}");
    assert_eq!(content_types(&result), ["text", "image"]);
    assert_subset(
        &result_json(&result),
        &json!({ "screenshot": {
            "screenshotId": "shot-1",
            "windowId": "fake-terminal",
            "region": { "x": 40, "y": 40, "width": 960, "height": 720 },
        } }),
        "zoom",
    );
    assert_eq!(
        last_args(&h, "captureScreenshot")[0],
        json!({ "kind": "window", "windowId": "fake-terminal" })
    );
    h.dispose().await;
}

#[tokio::test]
async fn surfaces_a_compositor_capture_failure_as_a_readable_error_result() {
    let h = harness();
    h.backend.fake.fail_next_with(
        "captureScreenshot",
        ComputerBackendError::new("org.zeron.ComputerUse.Error.CaptureFailed: window not visible"),
    );
    let result = h
        .call(
            "computer_screenshot",
            json!({ "window_id": "fake-calculator" }),
        )
        .await;
    assert!(result.is_error);
    assert!(
        text_of(&result).contains("window not visible"),
        "{result:?}"
    );
    h.dispose().await;
}

#[tokio::test]
async fn keeps_the_zoom_tool_read_only_and_free_of_an_approval_gate() {
    let h = harness();
    assert!(!computer_tool_requires_approval("computer_screenshot"));
    assert_subset(
        h.tools
            .entry("computer_screenshot")
            .and_then(|entry| entry.definition.annotations)
            .as_ref()
            .unwrap(),
        &json!({ "readOnlyHint": true }),
        "annotations",
    );
    h.decide(ApprovalDecision::Decline);
    let result = h
        .call(
            "computer_screenshot",
            json!({ "window_id": "fake-calculator" }),
        )
        .await;
    assert!(!result.is_error, "{result:?}");
    assert_eq!(h.backend.calls_for("captureScreenshot").len(), 1);
    assert!(h.approvals.cards.lock().unwrap().is_empty());
    h.dispose().await;
}

#[tokio::test]
async fn passes_a_clamped_pointer_landing_point_back_to_the_caller() {
    let h = harness();
    h.backend.hooks().click = Some(Arc::new(|_, (point, _)| {
        async move {
            Ok(ComputerBackendActionResult {
                point: Some(point),
                clamped_to: Some(ComputerPoint {
                    x: point.x,
                    y: 1_080.0,
                }),
                ..ComputerBackendActionResult::default()
            })
        }
        .boxed()
    }));
    h.see().await;
    let result = h.call("computer_click", json!({ "x": 44, "y": 44 })).await;
    assert!(!result.is_error, "{result:?}");
    assert_subset(
        &result_json(&result),
        &json!({ "point": { "x": 55, "y": 55 }, "clampedTo": { "x": 55, "y": 1_080 } }),
        "click",
    );
    h.dispose().await;
}

fn observation_capture(window_id: &str) -> Value {
    json!({
        "kind": "window",
        "windowId": window_id,
        "maxDimension": COMPUTER_ACTION_OBSERVATION_MAX_DIMENSION,
    })
}

#[tokio::test]
async fn attaches_a_post_action_screenshot_of_the_focused_window_to_action_results() {
    let h = harness();
    h.see().await;
    let result = h
        .call("computer_click", json!({ "x": 100, "y": 100 }))
        .await;
    assert!(!result.is_error, "{result:?}");
    assert_eq!(content_types(&result), ["text", "image"]);
    assert_subset(
        &result_json(&result),
        &json!({
            "action": "computer_click",
            "point": { "x": 125, "y": 125 },
            "screenshot": {
                "screenshotId": "shot-2",
                "windowId": "fake-terminal",
                "region": { "x": 40, "y": 40, "width": 960, "height": 720 },
                "scale": 1,
            },
        }),
        "click",
    );
    assert_eq!(
        last_args(&h, "captureScreenshot")[0],
        observation_capture("fake-terminal")
    );
    h.call("computer_click", json!({ "x": 5, "y": 5 })).await;
    assert_point(&last_args(&h, "click")[0], 45.0, 45.0);
    let repeat = h.call("computer_click", json!({ "x": 5, "y": 5 })).await;
    assert_eq!(
        result_json(&repeat)["screenshotUnchanged"],
        json!(true),
        "{repeat:?}"
    );
    h.call("computer_click", json!({ "x": 6, "y": 6 })).await;
    assert_point(&last_args(&h, "click")[0], 46.0, 46.0);
    h.dispose().await;
}

#[tokio::test]
async fn captures_the_window_a_scoped_action_named_rather_than_the_focused_one() {
    let h = harness();
    h.see().await;
    let result = h
        .call(
            "computer_click",
            json!({ "x": 1_100, "y": 200, "window_id": "fake-calculator" }),
        )
        .await;
    assert!(!result.is_error, "{result:?}");
    assert_eq!(
        last_args(&h, "captureScreenshot")[0],
        observation_capture("fake-calculator")
    );
    h.dispose().await;
}

#[tokio::test]
async fn reports_a_closed_target_instead_of_photographing_another_window() {
    let h = harness();
    h.backend.hooks().click = Some(Arc::new(|fake, (point, window_id)| {
        async move {
            let result = fake.click(point, window_id.as_deref(), &[]).await;
            fake.emit_windows_changed(Vec::new());
            result
        }
        .boxed()
    }));
    h.see().await;
    let result = h
        .call(
            "computer_click",
            json!({ "x": 1_100, "y": 200, "window_id": "fake-calculator" }),
        )
        .await;
    assert!(!result.is_error, "{result:?}");
    assert_eq!(content_types(&result), ["text"]);
    assert_subset(
        &result_json(&result),
        &json!({ "action": "computer_click", "targetWindowClosed": true }),
        "click",
    );
    h.dispose().await;
}

#[tokio::test]
async fn skips_the_post_action_screenshot_when_the_model_opts_out() {
    let h = harness();
    let result = h
        .call(
            "computer_type_text",
            json!({ "text": "hi", "include_screenshot": false }),
        )
        .await;
    assert!(!result.is_error, "{result:?}");
    assert_eq!(content_types(&result), ["text"]);
    assert!(h.backend.calls_for("captureScreenshot").is_empty());
    h.dispose().await;
}

#[tokio::test]
async fn focuses_a_named_window_before_keyboard_input_and_zooms_the_result_to_it() {
    let h = harness();
    let hotkey = h
        .call(
            "computer_press_key",
            json!({ "key": "ctrl+t", "window_id": "fake-calculator" }),
        )
        .await;
    assert!(!hotkey.is_error, "{hotkey:?}");
    assert_eq!(last_args(&h, "raiseWindow"), [json!("fake-calculator")]);
    assert_eq!(last_args(&h, "focusWindow"), [json!("fake-calculator")]);
    assert_eq!(result_json(&hotkey)["windowId"], json!("fake-calculator"));
    assert_eq!(
        last_args(&h, "captureScreenshot")[0],
        observation_capture("fake-calculator")
    );

    let typed = h
        .call(
            "computer_type_text",
            json!({ "text": "hi", "windowId": "fake-terminal" }),
        )
        .await;
    assert!(!typed.is_error, "{typed:?}");
    assert_eq!(last_args(&h, "focusWindow"), [json!("fake-terminal")]);
    assert_eq!(last_args(&h, "typeText"), [json!("hi")]);

    let pressed = h
        .call(
            "computer_press_key",
            json!({ "key": "enter", "window_id": "gone" }),
        )
        .await;
    assert!(pressed.is_error);
    assert_eq!(
        result_json(&pressed)["error"]["code"],
        json!("computer_target_not_found"),
        "{pressed:?}"
    );
    assert!(h.backend.calls_for("pressKey").is_empty());
    h.dispose().await;
}

fn record_typed_targets(backend: &Scripted) -> Arc<Mutex<Vec<Option<ComputerResolvedTarget>>>> {
    let targets: Arc<Mutex<Vec<Option<ComputerResolvedTarget>>>> = Arc::default();
    let sink = targets.clone();
    backend.hooks().type_text = Some(Arc::new(move |fake, (text, window_id, target)| {
        sink.lock().unwrap().push(target.clone());
        async move {
            fake.type_text(&text, window_id.as_deref(), target.as_ref())
                .await
        }
        .boxed()
    }));
    targets
}

#[tokio::test]
async fn uses_exact_semantic_text_input_without_focusing_the_target_window() {
    let h = harness();
    h.backend.set_background_target(false, true);
    let targets = record_typed_targets(&h.backend);
    let typed = h
        .call(
            "computer_type_text",
            json!({
                "text": "42",
                "label": "Display",
                "role": "text-field",
                "window_id": "fake-calculator",
                "include_screenshot": false,
            }),
        )
        .await;
    assert!(!typed.is_error, "{typed:?}");
    let targets = targets.lock().unwrap().clone();
    let requested = &targets
        .last()
        .and_then(Option::as_ref)
        .expect("semantic target")
        .target
        .target;
    assert_eq!(requested.label.as_deref(), Some("Display"));
    assert_eq!(requested.role.as_deref(), Some("text-field"));
    assert_eq!(requested.window_id.as_deref(), Some("fake-calculator"));
    assert!(h.backend.calls_for("focusWindow").is_empty());
    assert!(h.backend.calls_for("raiseWindow").is_empty());
    assert_eq!(last_args(&h, "typeText")[0], json!("42"));
    h.dispose().await;
}

#[tokio::test]
async fn uses_the_sole_writable_control_for_exact_window_text_without_a_label() {
    let h = harness();
    h.backend.set_background_target(false, true);
    let targets = record_typed_targets(&h.backend);
    let typed = h
        .call(
            "computer_type_text",
            json!({ "text": "42", "window_id": "fake-calculator", "include_screenshot": false }),
        )
        .await;
    assert!(!typed.is_error, "{typed:?}");
    let targets = targets.lock().unwrap().clone();
    let requested = targets
        .last()
        .and_then(Option::as_ref)
        .expect("semantic target")
        .target
        .target
        .clone();
    assert_eq!(
        requested,
        ComputerTarget {
            window_id: Some("fake-calculator".into()),
            ..ComputerTarget::default()
        }
    );
    assert!(h.backend.calls_for("focusWindow").is_empty());
    assert!(h.backend.calls_for("raiseWindow").is_empty());
    h.dispose().await;
}

#[tokio::test]
async fn zooms_the_post_action_screenshot_to_the_window_under_an_untargeted_actions_point() {
    let h = harness();
    h.see().await;
    let result = h
        .call(
            "computer_scroll",
            json!({ "x": 1_100, "y": 200, "delta_x": 0, "delta_y": 300 }),
        )
        .await;
    assert!(!result.is_error, "{result:?}");
    assert_eq!(content_types(&result), ["text", "image"]);
    assert_subset(
        &result_json(&result),
        &json!({ "action": "computer_scroll", "screenshot": { "windowId": "fake-calculator" } }),
        "scroll",
    );
    assert_eq!(
        last_args(&h, "captureScreenshot")[0],
        observation_capture("fake-calculator")
    );
    h.dispose().await;
}

#[tokio::test]
async fn tells_the_model_where_keyboard_input_lands_and_when_not_to_skip_a_screenshot() {
    let h = harness();
    for name in ["computer_type_text", "computer_press_key", "computer_paste"] {
        assert!(
            description(&h, name).contains("Pass window_id or use the last aimed window"),
            "{name}"
        );
        assert!(
            input_schema(&h, name).to_string().contains("window_id"),
            "{name}"
        );
    }
    assert!(
        input_schema(&h, "computer_click")
            .to_string()
            .contains("verify with fresh state or a final screenshot")
    );
    h.dispose().await;
}

#[tokio::test]
async fn reports_an_unchanged_screen_instead_of_resending_the_identical_image() {
    let h = harness();
    let first = h
        .call("computer_press_key", json!({ "key": "enter" }))
        .await;
    assert_eq!(content_types(&first), ["text", "image"], "{first:?}");
    let repeat = h
        .call("computer_press_key", json!({ "key": "enter" }))
        .await;
    assert!(!repeat.is_error, "{repeat:?}");
    assert_eq!(content_types(&repeat), ["text"]);
    let payload = result_json(&repeat);
    assert_eq!(payload["action"], json!("computer_press_key"));
    assert_eq!(payload["screenshotUnchanged"], json!(true));
    assert!(
        payload["note"]
            .as_str()
            .is_some_and(|note| note.contains("byte-for-byte what your previous screenshot showed")),
        "{payload}"
    );
    assert_eq!(h.backend.calls_for("captureScreenshot").len(), 2);
    let other = h
        .call(
            "computer_press_key",
            json!({ "key": "enter", "window_id": "fake-calculator" }),
        )
        .await;
    assert_eq!(content_types(&other), ["text", "image"], "{other:?}");
    h.dispose().await;
}
