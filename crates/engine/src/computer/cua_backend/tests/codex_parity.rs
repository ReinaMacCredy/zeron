use serde_json::json;
use zeron_proto::computer::{ComputerRect, CuaEffect};

use super::*;
use crate::computer::backend::ComputerBackendActionResult;

// Catches dropping the driver's display-only rows (Calculator's result text),
// pointing a tree line at the driver's element_index instead of the engine's
// root child, and losing the focused element or selected text.
#[tokio::test]
async fn parses_the_driver_tree_with_static_values_focus_and_root_targets() {
    let f = fixture();
    f.set_elements(vec![
        json!({
            "element_index": 0,
            "role": "AXWindow",
            "label": "Calculator",
            "element_token": "s1:0",
            "actions": ["AXRaise"],
        }),
        json!({
            "element_index": 1,
            "role": "AXButton",
            "label": "All Clear",
            "frame": { "x": 10, "y": 10, "width": 20, "height": 20 },
            "element_token": "s1:1",
            "actions": ["AXPress"],
        }),
    ]);
    f.knobs().window_state_extra = json!({
        "tree_nodes": [
            { "depth": 0, "role": "AXWindow", "element_index": 0, "title": "Calculator", "identifier": "main" },
            { "depth": 1, "role": "AXStaticText", "description": "Edit field", "value": "408" },
            { "depth": 1, "role": "AXButton", "element_index": 1, "description": "All Clear", "identifier": "AllClear" },
        ],
        "focused_element": { "depth": 0, "role": "AXButton", "element_index": 1, "description": "All Clear" },
        "selected_text": "40",
        "window_url": "file:///tmp/calc",
    })
    .as_object()
    .unwrap()
    .clone();
    let state = f
        .backend
        .get_state(window_state("cua:10:20"))
        .await
        .unwrap();
    let tree = state.ax_tree.expect("tree");
    assert_eq!(tree.lines.len(), 3);
    assert_eq!(tree.lines[0].actions, vec!["AXRaise".to_owned()]);
    assert_eq!(tree.lines[0].target, None);
    assert_eq!(tree.lines[1].value.as_deref(), Some("408"));
    assert_eq!(tree.lines[1].target, None);
    assert_eq!(tree.lines[2].target, Some(vec![0]));
    assert_eq!(
        state.root.unwrap().children[0].label.as_deref(),
        Some("All Clear")
    );
    assert_eq!(tree.focused.unwrap().target, Some(vec![0]));
    assert_eq!(tree.selected_text.as_deref(), Some("40"));
    assert_eq!(tree.url.as_deref(), Some("file:///tmp/calc"));
}

fn press_fixture(ax_error: i64, mutate_on_press: bool, pixel_uncertain: bool) -> Fixture {
    let f = fixture();
    f.set_elements(vec![
        json!({
            "element_index": 0,
            "role": "AXButton",
            "label": "Equals",
            "frame": { "x": -290, "y": 30, "width": 20, "height": 20 },
            "element_token": "s1:0",
            "actions": ["AXPress"],
        }),
        json!({
            "element_index": 1,
            "role": "AXTextField",
            "label": "Display",
            "value": "12",
            "frame": { "x": -260, "y": 30, "width": 40, "height": 20 },
            "element_token": "s1:1",
        }),
    ]);
    let knobs = f.knobs.clone();
    f.on_tool("click", move |args| {
        if args.get("element_token").is_none() && pixel_uncertain {
            return json!({
                "isError": true,
                "structuredContent": { "verified": false, "effect": "unverifiable", "code": "pixel_uncertain" },
                "content": [{ "type": "text", "text": "pointer delivery unknown" }],
            });
        }
        if args.get("element_token").is_none() {
            return json!({ "structuredContent": {
                "route": "pixel",
                "delivery": { "mode": "background" },
                "effect": "unverifiable",
            } });
        }
        if mutate_on_press {
            knobs.lock().unwrap().elements[1]["value"] = json!("408");
        }
        json!({
            "isError": true,
            "structuredContent": {
                "verified": false,
                "effect": "unverifiable",
                "code": "ax_action_uncertain",
                "diagnostics": {
                    "delivery_path": "ax",
                    "actuator": "ax_press",
                    "error_code": "ax_dispatch_failed",
                    "ax_error": ax_error,
                },
            },
            "content": [{ "type": "text", "text": "AX action failed" }],
        })
    });
    f
}

async fn press(f: &Fixture) -> Result<ComputerBackendActionResult, ComputerError> {
    let state = f
        .backend
        .get_state(window_state("cua:10:20"))
        .await
        .unwrap();
    let node = state.root.unwrap().children[0].clone();
    f.clear_calls();
    f.backend
        .perform_action(&resolved("Equals", &node), "AXPress")
        .await
}

fn pixel_clicks(f: &Fixture) -> usize {
    f.named("click")
        .iter()
        .filter(|call| call.args.get("element_token").is_none())
        .count()
}

// Catches giving up on Calculator-style buttons whose AXPress returns
// kAXErrorCannotComplete with no effect, or hiding that a pointer click
// replaced the AX press.
#[tokio::test]
async fn retries_a_cannot_complete_press_with_no_effect_as_one_background_pointer_click() {
    let f = press_fixture(-25204, false, false);
    let result = press(&f).await.expect("fallback click");
    assert_eq!(
        result.delivery_path.as_deref(),
        Some("cua-ax-then-pixel-background")
    );
    assert_eq!(pixel_clicks(&f), 1);
    assert_subset(&f.last_args("click"), json!({ "x": 20, "y": 20 }));
    assert_eq!(f.named("get_window_state").len(), 1);
    assert!(
        f.backend
            .inner
            .state()
            .observed_geometry
            .get("cua:10:20")
            .is_none()
    );
}

// Catches replaying an uncertain press whose effect is visible (the display
// changed) or one that failed for any reason other than CannotComplete.
#[tokio::test]
async fn never_replays_a_press_that_changed_state_or_failed_differently() {
    for (ax_error, mutate) in [(-25204, true), (-25200, false)] {
        let f = press_fixture(ax_error, mutate, false);
        let error = press(&f).await.unwrap_err();
        assert_eq!(effect(&error), Some(CuaEffect::DispatchedUnknown));
        assert_eq!(pixel_clicks(&f), 0, "ax_error {ax_error}");
    }
}

// Catches the fallback restoring a window's observed geometry after its own
// pointer click went uncertain, which would re-validate a stale screenshot.
#[tokio::test]
async fn an_uncertain_fallback_click_leaves_observed_geometry_invalidated() {
    let f = press_fixture(-25204, false, true);
    let state = f
        .backend
        .get_state(window_state("cua:10:20"))
        .await
        .unwrap();
    let node = state.root.unwrap().children[0].clone();
    f.backend.inner.state().observed_geometry.insert(
        "cua:10:20".to_owned(),
        ComputerRect {
            x: -300.0,
            y: 20.0,
            width: 200.0,
            height: 100.0,
        },
    );
    let error = f
        .backend
        .perform_action(&resolved("Equals", &node), "AXPress")
        .await
        .unwrap_err();
    assert_eq!(code(&error), Some("pixel_uncertain"));
    assert!(
        f.backend
            .inner
            .state()
            .observed_geometry
            .get("cua:10:20")
            .is_none()
    );
}

// Catches a rich paste reaching the driver without its plain-text fallback.
#[tokio::test]
async fn writes_html_beside_its_plain_text_through_clipboard_write() {
    let f = fixture();
    f.backend
        .write_clipboard_html("Hi & bye", "<b>Hi</b> &amp; bye")
        .await
        .unwrap();
    assert_eq!(
        f.last_args("clipboard_write"),
        json!({ "text": "Hi & bye", "html": "<b>Hi</b> &amp; bye" })
    );
}

// Catches treating a driver that predates tree_nodes as an empty tree; the
// tools layer must fall back to the actionable root instead.
#[tokio::test]
async fn leaves_the_tree_absent_for_a_driver_without_tree_nodes() {
    let f = fixture();
    let state = f
        .backend
        .get_state(window_state("cua:10:20"))
        .await
        .unwrap();
    assert!(state.ax_tree.is_none());
}
