use super::*;

use zeron_proto::computer::{ComputerRect, ComputerWindow};

use crate::computer::fake_backend::FakeComputerBackendOptions;

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
