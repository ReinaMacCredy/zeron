use super::*;

use zeron_proto::computer::{
    ComputerAxLine, ComputerAxTree, ComputerRect, ComputerUiNode, ComputerWindow,
};

use crate::computer::fake_backend::FakeComputerBackendOptions;

const CALC: &str = "fake-calculator";

fn ui(
    role: &str,
    label: Option<&str>,
    description: Option<&str>,
    value: Option<&str>,
    x: f64,
) -> Value {
    json!({
        "role": role,
        "label": label,
        "value": value,
        "description": description,
        "frame": { "x": 1_100.0 + x, "y": 300, "width": 40, "height": 30 },
        "activationPoint": null,
        "onScreen": true,
        "windowId": CALC,
        "children": [],
    })
}

fn calculator_root() -> ComputerUiNode {
    let mut window = ui("AXWindow", Some("Calculator"), None, None, 0.0);
    window["frame"] = json!({ "x": 1_050, "y": 120, "width": 420, "height": 620 });
    window["children"] = json!([
        ui("AXStaticText", None, Some("Edit field"), Some("408"), 0.0),
        ui("text-field", Some("Display"), None, Some("0"), 40.0),
        ui("AXButton", Some("All Clear"), None, None, 80.0),
        ui("AXButton", Some("Delete"), None, None, 120.0),
        ui(
            "AXTextArea",
            Some("Notes"),
            None,
            Some("😀 alpha beta gamma beta"),
            160.0
        ),
    ]);
    serde_json::from_value(json!({
        "role": "desktop",
        "label": null,
        "value": null,
        "description": null,
        "frame": { "x": 0, "y": 0, "width": 1920, "height": 1080 },
        "activationPoint": null,
        "onScreen": true,
        "windowId": null,
        "children": [window],
    }))
    .unwrap()
}

fn calculator_harness() -> Harness {
    harness_with(HarnessOptions {
        backend: fake_with(FakeComputerBackendOptions {
            root: Some(calculator_root()),
            ..FakeComputerBackendOptions::default()
        }),
        ..HarnessOptions::default()
    })
}

async fn read_tree(h: &Harness, arguments: Value) -> (String, Vec<Value>) {
    let result = h.call("computer_get_state", arguments).await;
    assert!(!result.is_error, "{result:?}");
    let payload = result_json(&result);
    (
        payload["tree"].as_str().unwrap_or_default().to_owned(),
        payload["elements"].as_array().cloned().unwrap_or_default(),
    )
}

fn ref_of(elements: &[Value], label: &str) -> u64 {
    elements
        .iter()
        .find(|element| element["label"] == json!(label))
        .and_then(|element| element["ref"].as_u64())
        .unwrap_or_else(|| panic!("no {label} in {elements:?}"))
}

// Catches discarding display-only value lines (Calculator's result), numbering
// lines with driver DFS indices instead of refs the ref-taking tools accept,
// and losing the hierarchy.
#[tokio::test]
async fn renders_a_calculator_tree_with_its_static_value_and_usable_refs() {
    let h = calculator_harness();
    let (tree, elements) = read_tree(&h, json!({ "window_id": CALC })).await;
    let clear = ref_of(&elements, "All Clear");
    let mut lines = tree.lines();
    assert_eq!(
        lines.next(),
        Some("Window: \"Calculator\", App: org.kde.kcalc.")
    );
    assert_eq!(lines.next(), Some("window Calculator"));
    assert!(
        tree.contains("\n\ttext Description: Edit field, Value: 408\n"),
        "{tree}"
    );
    assert!(
        tree.contains(&format!("\n\t{clear} button All Clear\n")),
        "{tree}"
    );
    let clicked = h.call("computer_click", json!({ "ref": clear })).await;
    assert!(!clicked.is_error, "{clicked:?}");
    h.dispose().await;
}

// Catches minting fresh numbers on every read: a re-read must show the same
// numbers for the same elements.
#[tokio::test]
async fn keeps_tree_numbers_stable_across_reads() {
    let h = calculator_harness();
    let (first, _) = read_tree(&h, json!({ "window_id": CALC })).await;
    let (second, _) = read_tree(&h, json!({ "window_id": CALC, "full": true })).await;
    assert_eq!(first, second);
    h.dispose().await;
}

// Catches re-sending the whole tree on every read, marking unchanged lines,
// and reporting no change after a value moved.
#[tokio::test]
async fn returns_a_codex_diff_after_a_value_change_and_a_no_change_sentence_after() {
    let h = calculator_harness();
    let (_, elements) = read_tree(&h, json!({ "window_id": CALC })).await;
    let display = ref_of(&elements, "Display");
    let set = h
        .call(
            "computer_set_value",
            json!({ "ref": display, "value": "12", "include_screenshot": false }),
        )
        .await;
    assert!(!set.is_error, "{set:?}");
    let (diff, _) = read_tree(&h, json!({ "window_id": CALC })).await;
    assert_eq!(
        diff,
        format!(
            "The following is a diff from the previous accessibility tree for Window: \"Calculator\" with ~ and + representing changed and added elements, respectively. Removed elements are summarized by ID range.\n~ {display} text field Display, Value: 12"
        )
    );
    let (unchanged, _) = read_tree(&h, json!({ "window_id": CALC })).await;
    assert_eq!(
        unchanged,
        "There has been no change in the accessibility tree for Window: \"Calculator\"."
    );
    h.dispose().await;
}

// Catches rendering the driver's menu bar into the window tree, dropping the
// focused element's ref, or omitting the selected text.
#[tokio::test]
async fn renders_the_backend_tree_with_focus_and_selection() {
    let h = calculator_harness();
    let line =
        |depth: u32, role: &str, title: Option<&str>, target: Option<Vec<u32>>| ComputerAxLine {
            depth,
            role: role.into(),
            title: title.map(str::to_owned),
            target,
            ..ComputerAxLine::default()
        };
    h.backend.fake.set_ax_tree(Some(ComputerAxTree {
        url: None,
        lines: vec![
            line(0, "AXMenuBar", None, None),
            line(1, "AXMenuBarItem", Some("File"), None),
            line(0, "AXWindow", Some("Calculator"), None),
            ComputerAxLine {
                value: Some("408".into()),
                ..line(1, "AXStaticText", None, None)
            },
            line(1, "AXButton", Some("Delete"), Some(vec![0, 3])),
        ],
        focused: Some(line(1, "AXButton", Some("Delete"), Some(vec![0, 3]))),
        selected_text: Some("40".into()),
        truncated: false,
    }));
    let (tree, elements) = read_tree(&h, json!({ "window_id": CALC })).await;
    let delete = ref_of(&elements, "Delete");
    assert_eq!(
        tree,
        format!(
            "Window: \"Calculator\", App: org.kde.kcalc.\nwindow Calculator\n\ttext Value: 408\n\t{delete} button Delete\nThe focused UI element is {delete} button Delete\nSelected text: ```40```"
        )
    );
    h.dispose().await;
}

fn window(id: &str, app: &str, bounds: ComputerRect, visible: bool) -> ComputerWindow {
    ComputerWindow {
        id: id.into(),
        title: id.into(),
        app_name: Some(app.into()),
        pid: None,
        bounds: Some(bounds),
        focused: false,
        keyboard_focused: None,
        active: None,
        minimized: false,
        visible,
        space_ids: None,
        current_space_id: None,
        on_current_space: None,
        stacking_index: None,
        occluded_by: None,
    }
}

fn rect(x: f64, y: f64, width: f64, height: f64) -> ComputerRect {
    ComputerRect {
        x,
        y,
        width,
        height,
    }
}

fn strip_harness() -> Harness {
    harness_with(HarnessOptions {
        backend: fake_with(FakeComputerBackendOptions {
            windows: Some(vec![
                window("doc", "TextEdit", rect(40.0, 40.0, 800.0, 600.0), true),
                window("pill", "Zoom", rect(600.0, 0.0, 120.0, 24.0), false),
                window("extra", "Bartender", rect(900.0, -30.0, 200.0, 24.0), true),
                window("ghost", "Finder", rect(0.0, 0.0, 0.0, 0.0), true),
            ]),
            ..FakeComputerBackendOptions::default()
        }),
        ..HarnessOptions::default()
    })
}

fn window_ids(value: &Value) -> Vec<String> {
    value
        .as_array()
        .map(|windows| {
            windows
                .iter()
                .filter_map(|window| window["id"].as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default()
}

// Catches listing hidden status pills, off-screen menu-bar strips and
// zero-area windows the agent cannot act on.
#[tokio::test]
async fn list_windows_drops_strip_windows_unless_asked() {
    let h = strip_harness();
    let listed = result_json(&h.call("computer_list_windows", json!({})).await);
    assert_eq!(window_ids(&listed["windows"]), vec!["doc"]);
    let all = result_json(
        &h.call("computer_list_windows", json!({ "include_strips": true }))
            .await,
    );
    assert_eq!(window_ids(&all["windows"]).len(), 4);
    let state = result_json(&h.call("computer_get_state", json!({})).await);
    assert_eq!(window_ids(&state["windows"]), vec!["doc"]);
    h.dispose().await;
}

fn app_harness() -> Harness {
    harness_with(HarnessOptions {
        backend: fake_with(FakeComputerBackendOptions {
            windows: Some(vec![
                window("calc", "Calculator", rect(40.0, 40.0, 300.0, 400.0), true),
                window("term", "Ghostty", rect(400.0, 40.0, 600.0, 400.0), true),
                window("misc", "Preview", rect(40.0, 500.0, 300.0, 300.0), true),
            ]),
            ..FakeComputerBackendOptions::default()
        }),
        ..HarnessOptions::default()
    })
}

// Catches a playbook that only knows Slack, repeats on every read, leaks
// across threads, or attaches to apps with no matching topic.
#[tokio::test]
async fn attaches_the_matching_app_playbook_once_per_thread_per_app() {
    let h = app_harness();
    let read = |window: &str| json!({ "window_id": window });
    let calc = result_json(&h.call("computer_get_state", read("calc")).await);
    assert_eq!(
        calc["appHint"].as_str(),
        crate::computer::tools::guidance::computer_help_section("calculator")
    );
    let again = result_json(&h.call("computer_get_state", read("calc")).await);
    assert!(again.get("appHint").is_none(), "{again}");
    let term = result_json(&h.call("computer_get_state", read("term")).await);
    assert_eq!(
        term["appHint"].as_str(),
        crate::computer::tools::guidance::computer_help_section("terminals")
    );
    let misc = result_json(&h.call("computer_get_state", read("misc")).await);
    assert!(misc.get("appHint").is_none(), "{misc}");
    let other = h.context_for("thread-other", None);
    let elsewhere = result_json(&h.call_in("computer_get_state", read("calc"), &other).await);
    assert!(elsewhere["appHint"].is_string(), "{elsewhere}");
    h.dispose().await;
}

// Catches a run's closing observation skipping the playbook on an app the
// thread never read with computer_get_state.
#[tokio::test]
async fn a_run_closing_on_a_new_app_attaches_its_playbook() {
    let h = app_harness();
    let result = result_json(
        &h.call(
            "computer_run",
            json!({ "steps": [{ "type": "click", "label": "Nowhere", "window_id": "term" }] }),
        )
        .await,
    );
    assert_eq!(
        result["appHint"].as_str(),
        crate::computer::tools::guidance::computer_help_section("terminals"),
        "{result}"
    );
    h.dispose().await;
}

// Catches a run whose first step fails before any window is known dumping
// the whole desktop window list into its closing state.
#[tokio::test]
async fn a_failed_run_never_returns_the_desktop_window_list() {
    let h = strip_harness();
    let result = result_json(
        &h.call(
            "computer_run",
            json!({ "steps": [{ "type": "click", "label": "Nowhere" }] }),
        )
        .await,
    );
    assert_eq!(result["steps"][0]["ok"], json!(false), "{result}");
    assert!(result["state"].get("windows").is_none(), "{result}");
    h.dispose().await;
}

// Catches scoping only by successful steps: a failing step that names its
// window must still scope the closing state to that one window.
#[tokio::test]
async fn a_failed_run_scopes_its_state_to_the_failing_steps_window() {
    let h = harness();
    let result = result_json(
        &h.call(
            "computer_run",
            json!({ "steps": [{ "type": "click", "label": "Nowhere", "window_id": "fake-calculator" }] }),
        )
        .await,
    );
    assert_eq!(result["steps"][0]["ok"], json!(false), "{result}");
    assert_eq!(
        window_ids(&result["state"]["windows"]),
        vec!["fake-calculator"]
    );
    h.dispose().await;
}

async fn select(h: &Harness, extra: Value) -> McpToolResult {
    let mut arguments = json!({ "label": "Notes", "window_id": CALC, "include_screenshot": false });
    for (key, value) in extra.as_object().unwrap() {
        arguments[key] = value.clone();
    }
    h.call("computer_select_text", arguments).await
}

fn last_range(h: &Harness) -> Value {
    h.backend
        .calls_for("selectText")
        .last()
        .map(|call| call.args[1].clone())
        .unwrap_or(Value::Null)
}

// Catches byte offsets instead of UTF-16 units (the emoji shifts them),
// picking the first of several matches, and ignoring prefix narrowing.
#[tokio::test]
async fn selects_text_by_match_in_utf16_units_and_refuses_ambiguity() {
    let h = calculator_harness();
    let ambiguous = select(&h, json!({ "text": "beta" })).await;
    assert!(ambiguous.is_error, "{ambiguous:?}");
    assert!(
        text_of(&ambiguous).contains("occurs 2 times"),
        "{ambiguous:?}"
    );
    assert!(h.backend.calls_for("selectText").is_empty());

    let picked = select(&h, json!({ "text": "beta", "prefix": "gamma " })).await;
    assert!(!picked.is_error, "{picked:?}");
    assert_eq!(last_range(&h), json!({ "start": 20, "length": 4 }));
    assert_eq!(result_json(&picked)["value"], json!("beta"));

    let caret = select(
        &h,
        json!({ "text": "alpha", "suffix": " beta", "selection": "cursor_after" }),
    )
    .await;
    assert!(!caret.is_error, "{caret:?}");
    assert_eq!(last_range(&h), json!({ "start": 8, "length": 0 }));

    let missing = select(&h, json!({ "text": "delta" })).await;
    assert!(text_of(&missing).contains("not found"), "{missing:?}");
    let mixed = select(&h, json!({ "text": "alpha", "start": 0, "length": 1 })).await;
    assert!(text_of(&mixed).contains("never both"), "{mixed:?}");
    h.dispose().await;
}

fn text_of(result: &McpToolResult) -> String {
    result.first_text().unwrap_or_default().to_owned()
}
