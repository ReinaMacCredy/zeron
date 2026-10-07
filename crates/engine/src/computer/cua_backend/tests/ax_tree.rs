use serde_json::json;

use super::*;

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
