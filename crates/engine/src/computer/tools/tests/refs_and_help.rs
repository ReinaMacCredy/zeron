use super::*;

use zeron_proto::computer::{ComputerUiNode, McpContent};

use crate::computer::tools::guidance::computer_tool_instructions;

fn image_parts(result: &McpToolResult) -> usize {
    result
        .content
        .iter()
        .filter(|part| matches!(part, McpContent::Image { .. }))
        .count()
}

fn text_of(result: &McpToolResult) -> String {
    result.first_text().unwrap_or_default().to_owned()
}

fn elements_of(result: &McpToolResult) -> Vec<Value> {
    result_json(result)["elements"]
        .as_array()
        .cloned()
        .unwrap_or_default()
}

fn find_label<'a>(elements: &'a [Value], label: &str) -> &'a Value {
    elements
        .iter()
        .find(|element| element["label"] == json!(label))
        .unwrap_or_else(|| panic!("no {label} in {elements:?}"))
}

fn last_click(h: &Harness) -> Value {
    h.backend
        .calls_for("click")
        .last()
        .map(|call| call.args[0].clone())
        .unwrap_or(Value::Null)
}

fn assert_point(point: &Value, x: f64, y: f64) {
    assert_eq!(point["x"].as_f64(), Some(x), "{point}");
    assert_eq!(point["y"].as_f64(), Some(y), "{point}");
}

fn duplicate_save_root() -> ComputerUiNode {
    let button = |x: f64| {
        json!({
            "role": "button",
            "label": "Save",
            "value": null,
            "description": null,
            "frame": { "x": x, "y": 100, "width": 60, "height": 30 },
            "activationPoint": null,
            "onScreen": true,
            "windowId": "w1",
            "children": [],
        })
    };
    serde_json::from_value(json!({
        "role": "desktop",
        "label": null,
        "value": null,
        "description": null,
        "frame": { "x": 0, "y": 0, "width": 1920, "height": 1080 },
        "activationPoint": null,
        "onScreen": true,
        "windowId": null,
        "children": [{
            "role": "window",
            "label": "Editor",
            "value": null,
            "description": null,
            "frame": { "x": 0, "y": 0, "width": 800, "height": 600 },
            "activationPoint": null,
            "onScreen": true,
            "windowId": "w1",
            "children": [button(20.0), button(200.0)],
        }],
    }))
    .unwrap()
}

fn duplicate_save_harness() -> Harness {
    harness_with(HarnessOptions {
        backend: fake_with(FakeComputerBackendOptions {
            root: Some(duplicate_save_root()),
            ..FakeComputerBackendOptions::default()
        }),
        ..HarnessOptions::default()
    })
}

#[tokio::test]
async fn ships_a_fresh_image_for_every_read_under_the_kill_switch_even_a_byte_identical_one() {
    let h = harness();
    h.tools.set_capture_reuse(false);
    let first = h.see().await;
    let second = h
        .call("computer_get_state", json!({ "include_screenshot": true }))
        .await;
    let payload = result_json(&second);
    assert_ne!(payload["screenshot"]["screenshotId"], first["screenshotId"]);
    assert!(payload.get("screenshotUnchanged").is_none());
    assert_eq!(image_parts(&second), 1);
    h.dispose().await;
}

#[tokio::test]
async fn names_the_earlier_frame_by_default_reuse_no_longer_needs_the_flag() {
    let h = harness();
    let first = h.see().await;
    let second = h
        .call("computer_get_state", json!({ "include_screenshot": true }))
        .await;
    let payload = result_json(&second);
    assert_eq!(payload["screenshotUnchanged"], json!(true));
    assert_eq!(payload["screenshot"]["screenshotId"], first["screenshotId"]);
    assert_eq!(image_parts(&second), 0);
    h.dispose().await;
}

#[tokio::test]
async fn names_the_earlier_frame_when_the_fresh_capture_is_byte_identical() {
    let h = harness();
    h.tools.set_capture_reuse(true);
    let first = h.see().await;
    let second = h
        .call("computer_get_state", json!({ "include_screenshot": true }))
        .await;
    let payload = result_json(&second);
    assert_eq!(h.backend.calls_for("getState").len(), 2);
    assert_eq!(payload["screenshotUnchanged"], json!(true));
    assert_eq!(payload["screenshot"]["screenshotId"], first["screenshotId"]);
    assert_eq!(image_parts(&second), 0);
    h.dispose().await;
}

#[tokio::test]
async fn delivers_normally_when_the_bytes_differ_or_the_coordinate_frame_moved() {
    use base64::Engine as _;
    let h = harness();
    h.tools.set_capture_reuse(true);
    h.see().await;
    let different = base64::engine::general_purpose::STANDARD.encode("a-different-desktop");
    h.backend
        .fake
        .queue_screenshots([different.clone(), different]);
    let changed = h
        .call(
            "computer_screenshot",
            json!({ "window_id": "fake-calculator" }),
        )
        .await;
    let changed_payload = result_json(&changed);
    assert!(
        changed_payload.get("screenshotUnchanged").is_none(),
        "{changed_payload}"
    );
    assert_eq!(image_parts(&changed), 1);

    let captures = h.backend.calls_for("captureScreenshot").len();
    let again = h
        .call(
            "computer_screenshot",
            json!({ "window_id": "fake-calculator" }),
        )
        .await;
    let again_payload = result_json(&again);
    assert_eq!(h.backend.calls_for("captureScreenshot").len(), captures + 1);
    assert_eq!(again_payload["screenshotUnchanged"], json!(true));
    assert_eq!(
        again_payload["screenshot"]["screenshotId"],
        changed_payload["screenshot"]["screenshotId"]
    );
    assert_eq!(image_parts(&again), 0);

    let other = h
        .call(
            "computer_screenshot",
            json!({ "window_id": "fake-terminal" }),
        )
        .await;
    let other_payload = result_json(&other);
    assert!(
        other_payload.get("screenshotUnchanged").is_none(),
        "{other_payload}"
    );
    assert_ne!(
        other_payload["screenshot"]["screenshotId"],
        changed_payload["screenshot"]["screenshotId"]
    );
    assert_eq!(
        other_payload["screenshot"]["windowId"],
        json!("fake-terminal")
    );
    assert_eq!(image_parts(&other), 1);
    h.dispose().await;
}

#[tokio::test]
async fn never_lets_one_threads_picture_stand_in_for_anothers() {
    let h = harness();
    h.tools.set_capture_reuse(true);
    let first = result_json(
        &h.call(
            "computer_screenshot",
            json!({ "window_id": "fake-calculator" }),
        )
        .await,
    );
    let other = h.context_for("other-thread", None);
    let second = result_json(
        &h.call_in(
            "computer_screenshot",
            json!({ "window_id": "fake-calculator" }),
            &other,
        )
        .await,
    );
    assert_ne!(
        second["screenshot"]["screenshotId"],
        first["screenshot"]["screenshotId"]
    );
    assert!(second.get("screenshotUnchanged").is_none());
    h.dispose().await;
}

#[tokio::test]
async fn lists_a_stable_ref_per_element_and_clicks_it_without_a_label() {
    let h = harness();
    let elements = elements_of(&h.call("computer_get_state", json!({})).await);
    let calculate = find_label(&elements, "Calculate");
    let result = h
        .call("computer_click", json!({ "ref": calculate["ref"] }))
        .await;
    assert!(!result.is_error, "{result:?}");
    assert_point(&last_click(&h), 1180.0, 228.0);
    h.dispose().await;
}

#[tokio::test]
async fn keeps_a_ref_bound_to_the_same_element_across_listings() {
    let h = harness();
    let first = elements_of(&h.call("computer_get_state", json!({})).await);
    let calculate = find_label(&first, "Calculate").clone();
    let second = elements_of(
        &h.call(
            "computer_get_state",
            json!({ "window_id": "fake-calculator" }),
        )
        .await,
    );
    assert_eq!(find_label(&second, "Calculate")["ref"], calculate["ref"]);
    let result = h
        .call("computer_click", json!({ "ref": calculate["ref"] }))
        .await;
    assert!(!result.is_error, "{result:?}");
    assert_point(&last_click(&h), 1180.0, 228.0);
    h.dispose().await;
}

#[tokio::test]
async fn mints_different_refs_for_duplicate_labels_and_clicks_the_right_one() {
    let h = duplicate_save_harness();
    let saves: Vec<Value> = elements_of(&h.call("computer_get_state", json!({})).await)
        .into_iter()
        .filter(|element| element["label"] == json!("Save"))
        .collect();
    assert_eq!(saves.len(), 2);
    assert_ne!(saves[0]["ref"], saves[1]["ref"]);
    let result = h
        .call("computer_click", json!({ "ref": saves[1]["ref"] }))
        .await;
    assert!(!result.is_error, "{result:?}");
    assert_point(&last_click(&h), 230.0, 115.0);
    h.dispose().await;
}

#[tokio::test]
async fn targets_a_duplicate_by_ref_ordinal_without_a_ref() {
    let h = duplicate_save_harness();
    h.call("computer_get_state", json!({})).await;
    let result = h
        .call(
            "computer_click",
            json!({ "label": "Save", "ref_ordinal": 1 }),
        )
        .await;
    assert!(!result.is_error, "{result:?}");
    assert_point(&last_click(&h), 230.0, 115.0);
    h.dispose().await;
}

#[tokio::test]
async fn refuses_a_ref_no_listing_ever_minted_and_a_ref_mixed_with_coordinates() {
    let h = harness();
    let before = h.call("computer_click", json!({ "ref": 0 })).await;
    assert!(before.is_error);
    assert!(
        text_of(&before).contains("computer_get_state"),
        "{before:?}"
    );

    h.call("computer_get_state", json!({})).await;
    let out_of_range = h.call("computer_click", json!({ "ref": 999 })).await;
    assert!(out_of_range.is_error);
    assert!(text_of(&out_of_range).contains("999"), "{out_of_range:?}");

    let mixed = h
        .call("computer_click", json!({ "ref": 0, "x": 10, "y": 10 }))
        .await;
    assert!(mixed.is_error);
    assert!(text_of(&mixed).contains("x/y"), "{mixed:?}");
    h.dispose().await;
}

#[tokio::test]
async fn refuses_when_a_claim_beside_the_ref_names_a_different_element() {
    let h = harness();
    let elements = elements_of(&h.call("computer_get_state", json!({})).await);
    let calculate = find_label(&elements, "Calculate")["ref"].clone();

    let wrong_label = h
        .call(
            "computer_click",
            json!({ "ref": calculate, "label": "Definitely not this" }),
        )
        .await;
    assert!(wrong_label.is_error);
    assert!(
        text_of(&wrong_label).contains("Calculate"),
        "{wrong_label:?}"
    );

    let wrong_role = h
        .call(
            "computer_click",
            json!({ "ref": calculate, "role": "text-field" }),
        )
        .await;
    assert!(wrong_role.is_error, "{wrong_role:?}");

    let right = h
        .call(
            "computer_click",
            json!({ "ref": calculate, "label": "Calculate" }),
        )
        .await;
    assert!(!right.is_error, "{right:?}");
    h.dispose().await;
}

#[tokio::test]
async fn keeps_refs_thread_scoped() {
    let h = harness();
    let elements = elements_of(&h.call("computer_get_state", json!({})).await);
    let reference = elements[0]["ref"].clone();
    let other = h.context_for("other-thread", None);
    let result = h
        .call_in("computer_click", json!({ "ref": reference }), &other)
        .await;
    assert!(result.is_error);
    assert!(
        text_of(&result).contains("computer_get_state"),
        "{result:?}"
    );
    h.dispose().await;
}

#[tokio::test]
async fn targets_text_and_run_steps_by_ref() {
    let h = harness();
    let elements = elements_of(&h.call("computer_get_state", json!({})).await);
    let display = find_label(&elements, "Display")["ref"].clone();

    let selected = h
        .call(
            "computer_select_text",
            json!({ "ref": display, "start": 0, "length": 1 }),
        )
        .await;
    assert!(!selected.is_error, "{selected:?}");
    let select_calls = h.backend.calls_for("selectText");
    assert_eq!(
        select_calls.last().map(|call| call.args[1].clone()),
        Some(json!({ "start": 0, "length": 1 }))
    );

    let calculate = find_label(&elements, "Calculate")["ref"].clone();
    let run = h
        .call(
            "computer_run",
            json!({ "steps": [{ "type": "click", "ref": calculate }] }),
        )
        .await;
    assert!(!run.is_error, "{run:?}");
    assert_point(&last_click(&h), 1180.0, 228.0);
    h.dispose().await;
}

#[tokio::test]
async fn lists_elements_mid_run_with_get_state_and_mints_refs_the_model_can_use_after() {
    let h = harness();
    let run = result_json(
        &h.call(
            "computer_run",
            json!({ "steps": [
                { "type": "get_state", "window_id": "fake-calculator" },
                { "type": "click", "label": "Calculate", "window_id": "fake-calculator" },
            ] }),
        )
        .await,
    );
    let listed = run["steps"][0]["result"]["elements"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    assert!(
        listed
            .iter()
            .any(|element| element["label"] == json!("Calculate")),
        "{run}"
    );
    assert_eq!(
        run["steps"][0]["result"]["elementWindowId"],
        json!("fake-calculator")
    );
    let calculate = find_label(&listed, "Calculate")["ref"].clone();
    let followup = h.call("computer_click", json!({ "ref": calculate })).await;
    assert!(!followup.is_error, "{followup:?}");
    assert_point(&last_click(&h), 1180.0, 228.0);
    h.dispose().await;
}

#[tokio::test]
async fn runs_verify_state_predicates_mid_run() {
    let h = harness();
    h.call("computer_get_state", json!({})).await;
    let run = result_json(
        &h.call(
            "computer_run",
            json!({ "steps": [{
                "type": "verify_state",
                "window_id": "fake-calculator",
                "expect": [{ "label": "Display", "value": "0" }],
            }] }),
        )
        .await,
    );
    assert_eq!(run["steps"][0]["ok"], json!(true), "{run}");
    assert_eq!(
        h.backend
            .calls_for("verifyState")
            .last()
            .map(|call| call.args.clone()),
        Some(vec![
            json!("fake-calculator"),
            json!([{ "label": "Display", "value": "0" }]),
        ])
    );
    h.dispose().await;
}

#[tokio::test]
async fn skips_a_step_whose_if_element_is_absent_and_runs_it_when_present() {
    let h = harness();
    h.call("computer_get_state", json!({})).await;
    let run = result_json(
        &h.call(
            "computer_run",
            json!({ "steps": [
                {
                    "type": "click",
                    "label": "Calculate",
                    "window_id": "fake-calculator",
                    "if_element": { "label": "Does not exist" },
                },
                {
                    "type": "click",
                    "label": "Calculate",
                    "window_id": "fake-calculator",
                    "if_element": { "label": "Calculate", "window_id": "fake-calculator" },
                },
            ] }),
        )
        .await,
    );
    assert_eq!(run["steps"][0]["ok"], json!(true), "{run}");
    assert_eq!(run["steps"][0]["skipped"], json!(true));
    assert_eq!(run["steps"][0]["skippedReason"], json!("if_element_absent"));
    assert_eq!(run["steps"][1]["ok"], json!(true));
    assert!(run["steps"][1].get("skipped").is_none(), "{run}");
    assert_eq!(run["skipped"], json!(1));
    assert_eq!(h.backend.calls_for("click").len(), 1);
    h.dispose().await;
}

#[tokio::test]
async fn skips_a_step_whose_unless_element_is_present() {
    let h = harness();
    h.call("computer_get_state", json!({})).await;
    let run = result_json(
        &h.call(
            "computer_run",
            json!({ "steps": [
                {
                    "type": "click",
                    "label": "Calculate",
                    "window_id": "fake-calculator",
                    "unless_element": { "label": "Calculate", "window_id": "fake-calculator" },
                },
                {
                    "type": "click",
                    "label": "Calculate",
                    "window_id": "fake-calculator",
                    "unless_element": { "label": "Not there" },
                },
            ] }),
        )
        .await,
    );
    assert_eq!(
        run["steps"][0]["skippedReason"],
        json!("unless_element_present"),
        "{run}"
    );
    assert!(run["steps"][1].get("skipped").is_none(), "{run}");
    assert_eq!(h.backend.calls_for("click").len(), 1);
    h.dispose().await;
}

#[tokio::test]
async fn evaluates_an_if_element_ref_against_the_element_it_was_minted_for() {
    let h = harness();
    let elements = elements_of(&h.call("computer_get_state", json!({})).await);
    let calculate = find_label(&elements, "Calculate")["ref"].clone();
    let run = result_json(
        &h.call(
            "computer_run",
            json!({ "steps": [{
                "type": "click",
                "label": "Calculate",
                "window_id": "fake-calculator",
                "if_element": { "ref": calculate },
            }] }),
        )
        .await,
    );
    assert!(run["steps"][0].get("skipped").is_none(), "{run}");
    assert_eq!(run["steps"][0]["ok"], json!(true));
    assert_eq!(h.backend.calls_for("click").len(), 1);
    h.dispose().await;
}

#[tokio::test]
async fn continues_past_a_failed_step_with_continue_on_error() {
    let h = harness();
    h.call("computer_get_state", json!({})).await;
    let run = result_json(
        &h.call(
            "computer_run",
            json!({ "steps": [
                {
                    "type": "click",
                    "label": "Missing control",
                    "window_id": "fake-calculator",
                    "continue_on_error": true,
                },
                { "type": "click", "label": "Calculate", "window_id": "fake-calculator" },
            ] }),
        )
        .await,
    );
    assert_eq!(run["steps"][0]["ok"], json!(false), "{run}");
    assert_eq!(run["steps"][1]["ok"], json!(true));
    assert_eq!(run["stopped"], json!(false));
    assert_eq!(run["completed"], json!(1));
    assert_eq!(h.backend.calls_for("click").len(), 1);
    h.dispose().await;
}

#[tokio::test]
async fn still_stops_the_run_on_a_failure_without_continue_on_error() {
    let h = harness();
    h.call("computer_get_state", json!({})).await;
    let run = result_json(
        &h.call(
            "computer_run",
            json!({ "steps": [
                { "type": "click", "label": "Missing control", "window_id": "fake-calculator" },
                { "type": "click", "label": "Calculate", "window_id": "fake-calculator" },
            ] }),
        )
        .await,
    );
    assert_eq!(run["steps"].as_array().map(Vec::len), Some(1), "{run}");
    assert_eq!(run["stopped"], json!(true));
    assert!(h.backend.calls_for("click").is_empty());
    h.dispose().await;
}

#[tokio::test]
async fn waits_for_an_element_to_be_absent_with_absent_true() {
    let h = harness();
    h.call("computer_get_state", json!({})).await;
    let run = result_json(
        &h.call(
            "computer_run",
            json!({ "steps": [{
                "type": "wait",
                "duration_ms": 500,
                "absent": true,
                "label": "Never present",
                "window_id": "fake-calculator",
            }] }),
        )
        .await,
    );
    assert_eq!(run["steps"][0]["ok"], json!(true), "{run}");
    assert_eq!(run["steps"][0]["result"]["status"], json!("ready"));
    h.dispose().await;
}

#[tokio::test]
async fn refuses_a_condition_with_no_label_carrying_target() {
    let h = harness();
    let run = h
        .call(
            "computer_run",
            json!({ "steps": [{
                "type": "click",
                "label": "Calculate",
                "window_id": "fake-calculator",
                "if_element": { "window_id": "fake-calculator" },
            }] }),
        )
        .await;
    assert!(run.is_error, "{run:?}");
    assert!(text_of(&run).contains("if_element"), "{run:?}");
    h.dispose().await;
}

#[tokio::test]
async fn looks_up_the_registered_browser_catalog_and_the_returned_prepare_tool_is_callable() {
    let h = harness_with(HarnessOptions {
        backend: fake_with(crate::computer::fake_backend::FakeComputerBackendOptions {
            browser: crate::computer::fake_backend::FakeComputerOptIn::BuiltIn,
            ..Default::default()
        }),
        ..HarnessOptions::default()
    });
    let help = result_json(
        &h.call(
            "computer_help",
            json!({ "tool": "computer_browser_prepare" }),
        )
        .await,
    );
    let definition = h
        .tools
        .entry("computer_browser_prepare")
        .unwrap()
        .definition;
    assert_eq!(
        help["definition"],
        serde_json::to_value(&definition).unwrap()
    );
    assert_eq!(help["advertised"], json!(true));
    let index = result_json(&h.call("computer_help", json!({ "topic": "tools" })).await);
    let text = index["text"].as_str().unwrap_or_default();
    assert!(text.contains("computer_browser_prepare"), "{index}");
    assert!(text.contains("computer_browser_navigate"), "{index}");
    let prepared = h
        .call(
            "computer_browser_prepare",
            json!({ "allow_launch": true, "profile": { "mode": "isolated_new" } }),
        )
        .await;
    assert!(!prepared.is_error, "{prepared:?}");
    assert_eq!(h.backend.calls_for("browser.browser_prepare").len(), 1);
    h.dispose().await;
}

#[tokio::test]
async fn does_not_invent_browser_entries_for_a_desktop_only_backend() {
    let h = harness();
    let help = h
        .call(
            "computer_help",
            json!({ "tool": "computer_browser_prepare" }),
        )
        .await;
    assert!(help.is_error);
    let index = result_json(&h.call("computer_help", json!({ "topic": "tools" })).await);
    assert!(
        !index["text"]
            .as_str()
            .unwrap_or_default()
            .contains("computer_browser_prepare"),
        "{index}"
    );
    h.dispose().await;
}

#[tokio::test]
async fn indexes_the_chapters_when_called_bare() {
    let h = harness();
    let result = h.call("computer_help", json!({})).await;
    assert!(!result.is_error);
    let topics = result_json(&result)["topics"]
        .as_str()
        .unwrap_or_default()
        .to_owned();
    for topic in ["browser", "menus", "hidden", "foreground", "forms", "tools"] {
        assert!(topics.contains(topic), "{topic}");
    }
    assert!(!topics.contains("recording"));
    h.dispose().await;
}

#[tokio::test]
async fn prefers_element_refs_gates_menus_on_visible_use_consent_and_teaches_whole_string_insertion()
 {
    let h = harness_with(HarnessOptions {
        dialect: Some(ComputerAgentDialect::Macos),
        ..HarnessOptions::default()
    });
    let menus = result_json(&h.call("computer_help", json!({ "topic": "menus" })).await)["text"]
        .as_str()
        .unwrap_or_default()
        .to_owned();
    assert!(menus.contains("On macOS"));
    assert!(menus.contains("act on an element ref first"));
    assert!(menus.contains("only when the user asked to see the screen"));
    let element_ref = menus.find("element ref").unwrap();
    let invoke_menu = menus.find("computer_invoke_menu").unwrap();
    let coordinate = menus.find("coordinate click").unwrap();
    assert!(element_ref < invoke_menu);
    assert!(invoke_menu < coordinate);
    let editors =
        result_json(&h.call("computer_help", json!({ "topic": "editors" })).await)["text"]
            .as_str()
            .unwrap_or_default()
            .to_owned();
    assert!(editors.contains("computer_type_text with window_id alone inserts the whole string"));
    assert!(editors.contains("never spell text out through computer_press_key"));
    h.dispose().await;
}

#[tokio::test]
async fn serves_one_chapter_verbatim_on_its_topic() {
    let h = harness();
    let result = h.call("computer_help", json!({ "topic": "browser" })).await;
    assert!(!result.is_error);
    let payload = result_json(&result);
    assert_eq!(payload["topic"], json!("browser"));
    let text = payload["text"].as_str().unwrap_or_default();
    assert!(text.contains("computer_browser_prepare"));
    assert!(text.contains("never mix their refs"), "{text}");
    assert!(!text.contains("computer_recording_start"));
    h.dispose().await;
}

#[tokio::test]
async fn returns_one_canonical_schema_and_routes_hidden_actions_through_the_advertised_batch_tool()
{
    let h = harness();
    assert!(
        h.tools
            .catalog()
            .iter()
            .any(|tool| tool.name == "computer_run")
    );
    let help = result_json(
        &h.call("computer_help", json!({ "tool": "computer_select_text" }))
            .await,
    );
    assert_eq!(
        help["definition"],
        serde_json::to_value(&h.tools.entry("computer_select_text").unwrap().definition).unwrap()
    );
    assert_eq!(help["advertised"], json!(false));
    assert_eq!(help["batchStep"]["type"], json!("select_text"));
    let fields: Vec<&str> = help["batchStep"]["fields"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(Value::as_str)
        .collect();
    for field in [
        "label",
        "start",
        "length",
        "if_element",
        "continue_on_error",
    ] {
        assert!(fields.contains(&field), "{field}: {fields:?}");
    }
    assert!(!fields.contains(&"include_screenshot"));
    assert!(!help.to_string().contains("computer_drag"));
    let step_type = help["batchStep"]["type"].clone();
    let run = h
        .call(
            "computer_run",
            json!({ "steps": [
                { "type": "set_value", "window_id": "fake-calculator", "label": "Display", "value": "12345" },
                {
                    "type": step_type,
                    "window_id": "fake-calculator",
                    "label": "Display",
                    "start": 1,
                    "length": 2,
                },
            ] }),
        )
        .await;
    assert!(!run.is_error, "{run:?}");
    let payload = result_json(&run);
    assert_eq!(payload["completed"], json!(2), "{payload}");
    assert_eq!(payload["stopped"], json!(false));
    assert_eq!(h.backend.calls_for("selectText").len(), 1);
    h.dispose().await;
}

#[tokio::test]
async fn exposes_an_image_preserving_inspection_route_for_hidden_specialists() {
    let h = harness();
    let help = result_json(
        &h.call("computer_help", json!({ "tool": "computer_zoom" }))
            .await,
    );
    assert_eq!(help["advertised"], json!(false));
    assert_eq!(help["inspection"]["name"], json!("computer_inspect"));
    assert_eq!(help["inspection"]["tool"], json!("computer_zoom"));
    assert!(help.get("batchStep").is_none());
    for arguments in [
        json!({ "tool": "computer_future" }),
        json!({ "tool": "computer_select_text", "topic": "tools" }),
    ] {
        let result = h.call("computer_help", arguments.clone()).await;
        assert!(result.is_error, "{arguments}");
    }
    h.dispose().await;
}

#[tokio::test]
async fn gives_every_hidden_desktop_tool_a_currently_advertised_canonical_route() {
    let h = harness();
    let advertised: Vec<String> = h
        .tools
        .catalog()
        .into_iter()
        .map(|tool| tool.name)
        .collect();
    let inspection_names: Vec<String> = h
        .tools
        .entry("computer_inspect")
        .unwrap()
        .definition
        .input_schema["properties"]["tool"]["enum"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .filter_map(|name| name.as_str().map(str::to_owned))
        .collect();
    let hidden: Vec<String> = h
        .tools
        .entries()
        .into_iter()
        .filter(|entry| entry.discovery_only)
        .map(|entry| entry.definition.name)
        .collect();
    assert!(!hidden.is_empty());
    for name in hidden {
        let help = result_json(&h.call("computer_help", json!({ "tool": name })).await);
        assert_eq!(help["advertised"], json!(false), "{name}");
        if let Some(step) = help["batchStep"]["type"].as_str() {
            assert!(
                advertised.iter().any(|tool| tool == "computer_run"),
                "{name}"
            );
            assert_eq!(name, format!("computer_{step}"));
        } else {
            assert_eq!(
                help["inspection"],
                json!({
                    "name": "computer_inspect",
                    "tool": name,
                    "instruction": "Pass this schema's arguments in the arguments object.",
                }),
                "{name}"
            );
            assert!(
                advertised.iter().any(|tool| tool == "computer_inspect"),
                "{name}"
            );
            assert!(inspection_names.contains(&name), "{name}");
        }
    }
    h.dispose().await;
}

#[tokio::test]
async fn serves_every_chapter_under_all() {
    let h = harness();
    let result = h.call("computer_help", json!({ "topic": "all" })).await;
    assert!(!result.is_error);
    let chapters = result_json(&result)["chapters"]
        .as_str()
        .unwrap_or_default()
        .to_owned();
    for expected in [
        "computer_browser_prepare",
        "computer_invoke_menu",
        "set_window_minimized",
        "computer_write_clipboard",
        "Available as computer_run steps",
    ] {
        assert!(chapters.contains(expected), "{expected}");
    }
    assert!(!chapters.contains("computer_recording"));
    assert!(!chapters.contains("computer_replay"));
    h.dispose().await;
}

#[tokio::test]
async fn refuses_an_unknown_topic_and_names_the_valid_ones() {
    let h = harness();
    let result = h
        .call("computer_help", json!({ "topic": "unknown_chapter" }))
        .await;
    assert!(result.is_error);
    assert!(text_of(&result).contains("browser"), "{result:?}");
    h.dispose().await;
}

#[tokio::test]
async fn keeps_the_injected_block_to_the_every_turn_core_and_points_at_the_tool() {
    let notes = computer_tool_instructions();
    assert!(notes.contains("computer_help"));
    assert!(notes.contains("computer_help({tool:\"computer_invoke_menu\"})"));
    assert!(!notes.contains("computer_recording_start"));
    assert!(!notes.contains("set_window_minimized"));
    assert!(notes.contains("never replay it"));
    assert!(notes.contains("delivery.effect"));
}
