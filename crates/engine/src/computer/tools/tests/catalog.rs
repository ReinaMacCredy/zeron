use serde_json::{Value, json};

use super::{harness, result_json};
use crate::computer::tools::{
    COMPUTER_APPROVAL_REQUIRED_TOOLS, batch_step_type_for, computer_tool_requires_approval,
    is_inspection_tool_name,
};

const ADVERTISED: &[&str] = &[
    "computer_list_windows",
    "computer_get_state",
    "computer_screenshot",
    "computer_get_screen_size",
    "computer_wait",
    "computer_launch_app",
    "computer_list_apps",
    "computer_verify_state",
    "computer_inspect",
    "computer_help",
    "computer_click",
    "computer_scroll",
    "computer_type_text",
    "computer_press_key",
    "computer_paste",
    "computer_activate_window",
    "computer_set_value",
    "computer_run",
];

const HIDDEN: &[&str] = &[
    "computer_read_clipboard",
    "computer_zoom",
    "computer_get_accessibility_tree",
    "computer_get_cursor_position",
    "computer_set_window_frame",
    "computer_invoke_menu",
    "computer_kill_app",
    "computer_set_window_minimized",
    "computer_set_app_visibility",
    "computer_move_cursor",
    "computer_drag",
    "computer_write_clipboard",
    "computer_perform_action",
    "computer_select_text",
    "computer_spaces",
];

#[tokio::test]
async fn exposes_the_batch_fast_path_with_the_specialists_hidden() {
    let h = harness();
    let names: Vec<String> = h
        .tools
        .entries()
        .iter()
        .map(|entry| entry.name().to_owned())
        .collect();
    assert_eq!(
        names,
        [
            "computer_list_windows",
            "computer_get_state",
            "computer_screenshot",
            "computer_get_screen_size",
            "computer_wait",
            "computer_read_clipboard",
            "computer_launch_app",
            "computer_list_apps",
            "computer_verify_state",
            "computer_zoom",
            "computer_get_accessibility_tree",
            "computer_get_cursor_position",
            "computer_inspect",
            "computer_help",
            "computer_set_window_frame",
            "computer_invoke_menu",
            "computer_kill_app",
            "computer_set_window_minimized",
            "computer_set_app_visibility",
            "computer_click",
            "computer_move_cursor",
            "computer_drag",
            "computer_scroll",
            "computer_type_text",
            "computer_press_key",
            "computer_write_clipboard",
            "computer_paste",
            "computer_activate_window",
            "computer_set_value",
            "computer_perform_action",
            "computer_select_text",
            "computer_run",
            "computer_spaces",
        ]
    );
    let catalog: Vec<String> = h
        .tools
        .catalog()
        .into_iter()
        .map(|tool| tool.name)
        .collect();
    assert_eq!(catalog, ADVERTISED);
    let hidden: Vec<String> = h
        .tools
        .entries()
        .into_iter()
        .filter(|entry| entry.discovery_only)
        .map(|entry| entry.definition.name)
        .collect();
    assert_eq!(hidden, HIDDEN);
    for name in COMPUTER_APPROVAL_REQUIRED_TOOLS {
        assert!(computer_tool_requires_approval(name));
        assert!(names.iter().any(|entry| entry == name), "{name}");
    }
    assert!(!computer_tool_requires_approval("computer_wait"));
    assert_eq!(
        h.tools
            .entry("computer_move_cursor")
            .and_then(|entry| entry.definition.annotations)
            .map(|annotations| annotations["destructiveHint"].clone()),
        Some(json!(false))
    );
    h.dispose().await;
}

#[tokio::test]
async fn every_hidden_tool_has_an_advertised_route_and_a_help_schema() {
    let h = harness();
    for name in HIDDEN {
        let routed = batch_step_type_for(name).is_some() || is_inspection_tool_name(name);
        assert!(
            routed,
            "{name} has no computer_run or computer_inspect route"
        );
        let help = h.call("computer_help", json!({ "tool": name })).await;
        assert!(!help.is_error, "{name}: {help:?}");
        let payload = result_json(&help);
        assert_eq!(payload["definition"]["name"], json!(name));
        assert!(payload["definition"]["inputSchema"].is_object(), "{name}");
        assert_eq!(payload["advertised"], json!(false));
        assert!(
            payload.get("batchStep").is_some() || payload.get("inspection").is_some(),
            "{name}"
        );
    }
    h.dispose().await;
}

#[tokio::test]
async fn keeps_a_discovery_only_tool_callable_by_exact_name() {
    let h = harness();
    let dragged = h
        .call(
            "computer_drag",
            json!({
                "from": { "label": "Calculate", "role": "button" },
                "to": { "label": "Display", "role": "text-field" },
            }),
        )
        .await;
    assert!(!dragged.is_error, "{dragged:?}");
    assert_eq!(h.backend.calls_for("drag").len(), 1);
    h.dispose().await;
}

#[tokio::test]
async fn every_hidden_tool_dispatches_by_exact_name_and_through_its_route() {
    let h = harness();
    let direct: &[(&str, Value)] = &[
        ("computer_read_clipboard", json!({})),
        (
            "computer_zoom",
            json!({ "window_id": "fake-calculator", "x": 0, "y": 0, "width": 50, "height": 40 }),
        ),
        ("computer_get_accessibility_tree", json!({})),
        ("computer_get_cursor_position", json!({})),
        (
            "computer_set_window_frame",
            json!({ "window_id": "fake-calculator", "x": 10, "y": 10, "width": 400, "height": 500 }),
        ),
        (
            "computer_invoke_menu",
            json!({ "window_id": "fake-calculator", "path": ["File", "Close"] }),
        ),
        (
            "computer_set_window_minimized",
            json!({ "window_id": "fake-calculator", "minimized": true }),
        ),
        (
            "computer_set_app_visibility",
            json!({ "pid": 1_002, "hidden": false }),
        ),
        (
            "computer_move_cursor",
            json!({ "label": "Calculate", "role": "button", "include_screenshot": false }),
        ),
        (
            "computer_drag",
            json!({
                "from": { "label": "Calculate", "role": "button" },
                "to": { "label": "Display", "role": "text-field" },
                "include_screenshot": false,
            }),
        ),
        ("computer_write_clipboard", json!({ "text": "copied" })),
        (
            "computer_perform_action",
            json!({ "label": "Calculate", "role": "button", "action": "activate", "include_screenshot": false }),
        ),
        (
            "computer_select_text",
            json!({ "label": "Display", "start": 0, "length": 1, "include_screenshot": false }),
        ),
        ("computer_kill_app", json!({ "window_id": "fake-terminal" })),
        ("computer_spaces", json!({ "operation": "release" })),
    ];
    assert_eq!(direct.len(), HIDDEN.len());
    for (name, arguments) in direct {
        let result = h.call(name, arguments.clone()).await;
        let text = result.first_text().unwrap_or_default().to_owned();
        assert!(
            !text.contains("Unknown computer tool"),
            "{name} is not callable by exact name: {text}"
        );
        if is_inspection_tool_name(name) {
            let mut arguments = arguments.clone();
            let inspected = h
                .call(
                    "computer_inspect",
                    json!({ "tool": name, "arguments": arguments.take() }),
                )
                .await;
            assert!(!inspected.is_error, "{name}: {inspected:?}");
        } else {
            let step_type = batch_step_type_for(name).unwrap();
            let mut step = arguments.as_object().unwrap().clone();
            step.remove("include_screenshot");
            step.insert("type".into(), json!(step_type));
            let run = h.call("computer_run", json!({ "steps": [step] })).await;
            let payload = result_json(&run);
            assert!(
                payload["steps"][0]["type"] == json!(step_type),
                "{name}: {run:?}"
            );
            assert_ne!(
                payload["steps"][0]["error"]["code"],
                json!("invalid_step"),
                "{name}: {run:?}"
            );
        }
    }
    h.dispose().await;
}

const SYNARA_REGISTERED: &[&str] = &[
    "computer_spaces",
    "computer_list_windows",
    "computer_get_state",
    "computer_screenshot",
    "computer_get_screen_size",
    "computer_wait",
    "computer_read_clipboard",
    "computer_launch_app",
    "computer_list_apps",
    "computer_verify_state",
    "computer_zoom",
    "computer_get_accessibility_tree",
    "computer_get_cursor_position",
    "computer_inspect",
    "computer_help",
    "computer_set_window_frame",
    "computer_invoke_menu",
    "computer_kill_app",
    "computer_set_window_minimized",
    "computer_set_app_visibility",
    "computer_click",
    "computer_move_cursor",
    "computer_drag",
    "computer_scroll",
    "computer_type_text",
    "computer_press_key",
    "computer_write_clipboard",
    "computer_paste",
    "computer_activate_window",
    "computer_set_value",
    "computer_perform_action",
    "computer_select_text",
    "computer_run",
    "computer_browser_state",
    "computer_browser_prepare",
    "computer_browser_navigate",
    "computer_browser_click",
    "computer_browser_type",
    "computer_browser_dialog",
    "computer_browser_upload",
    "computer_browser_download",
    "computer_browser_pointer",
    "computer_browser_press",
];

#[tokio::test]
async fn registers_synara_tools_plus_the_macos_native_browser_when_the_backend_has_a_browser() {
    let h = super::harness_with(super::HarnessOptions {
        backend: super::fake_with(crate::computer::fake_backend::FakeComputerBackendOptions {
            browser: crate::computer::fake_backend::FakeComputerOptIn::BuiltIn,
            ..Default::default()
        }),
        ..super::HarnessOptions::default()
    });
    let mut registered: Vec<String> = h
        .tools
        .entries()
        .iter()
        .map(|entry| entry.name().to_owned())
        .collect();
    registered.sort();
    let mut expected: Vec<&str> = SYNARA_REGISTERED.to_vec();
    #[cfg(target_os = "macos")]
    expected.push("computer_zeron_browser");
    expected.sort_unstable();
    assert_eq!(registered, expected);
    let catalog: Vec<String> = h
        .tools
        .catalog()
        .into_iter()
        .map(|tool| tool.name)
        .collect();
    let mut advertised: Vec<&str> = ADVERTISED.to_vec();
    advertised.extend_from_slice(&crate::computer::tools::browser::COMPUTER_BROWSER_TOOL_NAMES);
    #[cfg(target_os = "macos")]
    advertised.push("computer_zeron_browser");
    assert_eq!(catalog, advertised);
    let hidden: Vec<String> = h
        .tools
        .entries()
        .into_iter()
        .filter(|entry| entry.discovery_only)
        .map(|entry| entry.definition.name)
        .collect();
    assert_eq!(hidden.len(), 15);
    assert_eq!(hidden, HIDDEN);
    for name in SYNARA_REGISTERED {
        assert!(
            super::super::tool_permission::canonical_zeron_computer_tool_name(name).is_some(),
            "{name}"
        );
    }
    h.dispose().await;
}
