use std::sync::Arc;

use serde_json::{Value, json};
use zeron_proto::computer::{ComputerWindow, McpContent, McpToolResult};

use super::{HarnessOptions, THREAD, ToolSeams, args, harness, harness_with, result_json};
use crate::computer::approval_gate::ApprovalDecision;
use crate::computer::backend::{ComputerAgentDialect, ComputerBackend, ComputerBackendMethod};
use crate::computer::fake_backend::FakeComputerBackendOptions;
use crate::computer::manager::tests::Scripted;
use crate::computer::manager::{ComputerManager, ComputerManagerEnv, ComputerManagerOptions};
use crate::computer::tools::tool_permission::{
    ZERON_COMPUTER_TOOL_NAMES, canonical_zeron_computer_tool_name,
    is_zeron_computer_tool_family_name,
};
use crate::computer::tools::{
    COMPUTER_APPROVAL_REQUIRED_TOOLS, ComputerToolContext, ComputerTools,
};
use crate::computer::unavailable_backend::{
    UnavailableComputerBackend, UnavailableComputerBackendOptions,
};

fn text(result: &McpToolResult) -> String {
    result
        .content
        .iter()
        .map(|part| match part {
            McpContent::Text { text } => text.as_str(),
            McpContent::Image { .. } => "",
        })
        .collect::<Vec<_>>()
        .join("\n")
}

async fn windows(backend: &Scripted) -> Vec<ComputerWindow> {
    backend.fake.list_windows().await.unwrap()
}

async fn window(backend: &Scripted, id: &str) -> ComputerWindow {
    windows(backend)
        .await
        .into_iter()
        .find(|window| window.id == id)
        .unwrap()
}

fn without_visibility() -> Arc<Scripted> {
    let backend = Scripted::default_fake();
    backend.remove(ComputerBackendMethod::SetWindowMinimized);
    backend.remove(ComputerBackendMethod::SetAppVisibility);
    backend
}

async fn unavailable_call(name: &str, arguments: Value) -> McpToolResult {
    let backend: Arc<dyn ComputerBackend> = Arc::new(UnavailableComputerBackend::new(
        "the display link is gone",
        UnavailableComputerBackendOptions::default(),
    ));
    let mut options = ComputerManagerOptions::new(backend);
    options.action_settle_ms = Some(0);
    options.env = ComputerManagerEnv::default();
    options.seams = harness().approvals.clone();
    let manager = ComputerManager::new(options);
    let tools = ComputerTools::new(manager.clone(), Vec::new());
    let context = ComputerToolContext {
        chat_id: THREAD.into(),
        chat_label: None,
        turn_id: Some("turn-visibility".into()),
        session_key: "session-visibility".into(),
        request_id: json!(1),
        signal: tokio_util::sync::CancellationToken::new(),
        seams: Arc::new(ToolSeams::default()),
        mutation_dispatched: Arc::default(),
    };
    let result = tools.call(name, args(arguments), &context).await;
    manager.dispose().await;
    result
}

#[tokio::test]
async fn set_window_minimized_is_approval_gated_and_minimizes_the_exact_window_without_activating_it()
 {
    let h = harness();
    h.decide(ApprovalDecision::Accept);
    assert!(COMPUTER_APPROVAL_REQUIRED_TOOLS.contains(&"computer_set_window_minimized"));
    let result = h
        .call(
            "computer_set_window_minimized",
            json!({ "window_id": "fake-calculator", "minimized": true }),
        )
        .await;
    let cards = h.approvals.cards.lock().unwrap().clone();
    assert_eq!(cards.len(), 1);
    assert_eq!(cards[0].tool_name, "computer_set_window_minimized");
    let params: Value = serde_json::from_str(&cards[0].params_display).unwrap();
    assert_eq!(params["window_id"], json!("fake-calculator"));
    assert_eq!(params["minimized"], json!(true));
    assert!(!result.is_error, "{result:?}");
    let payload = result_json(&result);
    assert_eq!(payload["action"], json!("computer_set_window_minimized"));
    assert_eq!(payload["windowId"], json!("fake-calculator"));
    assert_eq!(payload["delivery"]["verified"], json!("confirmed"));
    assert_eq!(payload["delivery"]["effect"], json!("verified"));
    let calculator = window(&h.backend, "fake-calculator").await;
    assert!(calculator.minimized && !calculator.visible);
    assert!(window(&h.backend, "fake-terminal").await.visible);
    assert!(h.backend.calls_for("raiseWindow").is_empty());
    assert!(h.backend.calls_for("focusWindow").is_empty());
    h.dispose().await;
}

#[tokio::test]
async fn set_window_minimized_restores_the_same_window_in_place() {
    let h = harness();
    h.call(
        "computer_set_window_minimized",
        json!({ "window_id": "fake-calculator", "minimized": true }),
    )
    .await;
    let restored = h
        .call(
            "computer_set_window_minimized",
            json!({ "window_id": "fake-calculator", "minimized": false }),
        )
        .await;
    assert!(!restored.is_error, "{restored:?}");
    let calculator = window(&h.backend, "fake-calculator").await;
    assert!(!calculator.minimized && calculator.visible);
    h.dispose().await;
}

#[tokio::test]
async fn set_window_minimized_refuses_bad_arguments_and_a_foreground_mode_it_cannot_honor_before_dispatch()
 {
    let h = harness();
    let foreground = h
        .call(
            "computer_set_window_minimized",
            json!({ "window_id": "fake-calculator", "minimized": true, "delivery_mode": "foreground" }),
        )
        .await;
    assert!(foreground.is_error);
    assert!(text(&foreground).contains("never activates"));
    for arguments in [
        json!({ "window_id": "no-such-window", "minimized": true }),
        json!({ "window_id": "fake-calculator" }),
        json!({ "window_id": "fake-calculator", "minimized": "yes" }),
        json!({ "minimized": true }),
    ] {
        let result = h
            .call("computer_set_window_minimized", arguments.clone())
            .await;
        assert!(result.is_error, "{arguments}");
    }
    assert!(h.backend.calls_for("setWindowMinimized").is_empty());
    h.dispose().await;
}

#[tokio::test]
async fn set_window_minimized_hides_another_apps_window_without_asking() {
    let h = harness();
    let first = h
        .call(
            "computer_set_window_minimized",
            json!({ "window_id": "fake-terminal", "minimized": true }),
        )
        .await;
    assert!(!first.is_error, "{first:?}");
    let second = h
        .call(
            "computer_set_window_minimized",
            json!({ "window_id": "fake-calculator", "minimized": true }),
        )
        .await;
    assert!(!second.is_error, "{second:?}");
    assert_eq!(h.backend.calls_for("setWindowMinimized").len(), 2);
    h.dispose().await;
}

#[tokio::test]
async fn set_window_minimized_dispatches_nothing_when_approval_is_refused() {
    let h = harness();
    h.decide(ApprovalDecision::Decline);
    let result = h
        .call(
            "computer_set_window_minimized",
            json!({ "window_id": "fake-calculator", "minimized": true }),
        )
        .await;
    assert!(result.is_error);
    assert!(h.backend.calls_for("setWindowMinimized").is_empty());
    let calculator = window(&h.backend, "fake-calculator").await;
    assert!(!calculator.minimized && calculator.visible);
    h.dispose().await;
}

#[tokio::test]
async fn set_window_minimized_refuses_cleanly_on_a_backend_without_the_write_and_carries_the_unavailable_message()
 {
    let h = harness_with(HarnessOptions {
        backend: without_visibility(),
        ..HarnessOptions::default()
    });
    let result = h
        .call(
            "computer_set_window_minimized",
            json!({ "window_id": "fake-calculator", "minimized": true }),
        )
        .await;
    assert!(result.is_error);
    assert!(text(&result).contains("cannot minimize or restore windows"));
    h.dispose().await;
    let refused = unavailable_call(
        "computer_set_window_minimized",
        json!({ "window_id": "fake-calculator", "minimized": true }),
    )
    .await;
    assert!(refused.is_error);
    assert!(
        text(&refused).contains("the display link is gone"),
        "{refused:?}"
    );
}

#[tokio::test]
async fn set_app_visibility_is_approval_gated_and_hides_a_running_app_by_pid_without_activating_it()
{
    let h = harness();
    h.decide(ApprovalDecision::Accept);
    assert!(COMPUTER_APPROVAL_REQUIRED_TOOLS.contains(&"computer_set_app_visibility"));
    let result = h
        .call(
            "computer_set_app_visibility",
            json!({ "pid": 1_002, "hidden": true }),
        )
        .await;
    let cards = h.approvals.cards.lock().unwrap().clone();
    assert_eq!(cards.len(), 1);
    assert_eq!(cards[0].tool_name, "computer_set_app_visibility");
    let params: Value = serde_json::from_str(&cards[0].params_display).unwrap();
    assert_eq!(params["pid"].as_f64(), Some(1_002.0));
    assert_eq!(params["hidden"], json!(true));
    assert!(!result.is_error, "{result:?}");
    let payload = result_json(&result);
    assert_eq!(payload["action"], json!("computer_set_app_visibility"));
    assert_eq!(payload["delivery"]["verified"], json!("confirmed"));
    assert_eq!(payload["delivery"]["effect"], json!("verified"));
    assert!(!window(&h.backend, "fake-calculator").await.visible);
    assert!(window(&h.backend, "fake-terminal").await.visible);
    assert!(h.backend.calls_for("raiseWindow").is_empty());
    assert!(h.backend.calls_for("focusWindow").is_empty());
    h.dispose().await;
}

#[tokio::test]
async fn set_app_visibility_unhides_the_apps_windows_in_place() {
    let h = harness();
    h.call(
        "computer_set_app_visibility",
        json!({ "pid": 1_002, "hidden": true }),
    )
    .await;
    let restored = h
        .call(
            "computer_set_app_visibility",
            json!({ "pid": 1_002, "hidden": false }),
        )
        .await;
    assert!(!restored.is_error, "{restored:?}");
    assert!(window(&h.backend, "fake-calculator").await.visible);
    h.dispose().await;
}

#[tokio::test]
async fn set_app_visibility_refuses_a_bad_pid_a_missing_flag_and_foreground_delivery_before_dispatch()
 {
    let h = harness();
    for arguments in [
        json!({ "pid": 0, "hidden": true }),
        json!({ "pid": -3, "hidden": true }),
        json!({ "pid": 1.5, "hidden": true }),
        json!({ "pid": "1002", "hidden": true }),
        json!({ "pid": 1_002 }),
        json!({ "pid": 1_002, "hidden": "yes" }),
        json!({ "hidden": true }),
        json!({ "pid": 1_002, "hidden": true, "delivery_mode": "foreground" }),
    ] {
        let result = h
            .call("computer_set_app_visibility", arguments.clone())
            .await;
        assert!(result.is_error, "{arguments}");
    }
    assert!(h.backend.calls_for("setAppVisibility").is_empty());
    h.dispose().await;
}

#[tokio::test]
async fn set_app_visibility_hides_the_app_a_pid_resolves_to_without_asking() {
    let h = harness();
    let first = h
        .call(
            "computer_set_window_minimized",
            json!({ "window_id": "fake-terminal", "minimized": true }),
        )
        .await;
    assert!(!first.is_error, "{first:?}");
    let second = h
        .call(
            "computer_set_app_visibility",
            json!({ "pid": 1_002, "hidden": true }),
        )
        .await;
    assert!(!second.is_error, "{second:?}");
    assert_eq!(h.backend.calls_for("setAppVisibility").len(), 1);
    h.dispose().await;
}

#[tokio::test]
async fn set_app_visibility_reaches_the_backend_refusal_for_a_pid_that_names_no_running_app() {
    let h = harness();
    let result = h
        .call(
            "computer_set_app_visibility",
            json!({ "pid": 9_999, "hidden": true }),
        )
        .await;
    assert!(result.is_error);
    assert!(
        text(&result).contains("No running application has pid 9999"),
        "{result:?}"
    );
    assert_eq!(h.backend.calls_for("setAppVisibility").len(), 1);
    h.dispose().await;
}

#[tokio::test]
async fn set_app_visibility_dispatches_nothing_when_approval_is_refused() {
    let h = harness();
    h.decide(ApprovalDecision::Decline);
    let result = h
        .call(
            "computer_set_app_visibility",
            json!({ "pid": 1_002, "hidden": true }),
        )
        .await;
    assert!(result.is_error);
    assert!(h.backend.calls_for("setAppVisibility").is_empty());
    h.dispose().await;
}

#[tokio::test]
async fn set_app_visibility_refuses_cleanly_on_a_backend_without_the_write_and_carries_the_unavailable_message()
 {
    let h = harness_with(HarnessOptions {
        backend: without_visibility(),
        ..HarnessOptions::default()
    });
    let result = h
        .call(
            "computer_set_app_visibility",
            json!({ "pid": 1_002, "hidden": true }),
        )
        .await;
    assert!(result.is_error);
    assert!(text(&result).contains("cannot hide or unhide applications"));
    h.dispose().await;
    let refused = unavailable_call(
        "computer_set_app_visibility",
        json!({ "pid": 1_002, "hidden": true }),
    )
    .await;
    assert!(refused.is_error);
    assert!(
        text(&refused).contains("the display link is gone"),
        "{refused:?}"
    );
}

#[tokio::test]
async fn launch_app_hidden_launches_the_app_off_screen_created_but_never_rendered_never_focused() {
    let h = harness();
    let result = h
        .call(
            "computer_launch_app",
            json!({ "app": "TextEdit", "hidden": true, "wait_for_window": false }),
        )
        .await;
    assert!(!result.is_error, "{result:?}");
    assert_eq!(
        h.backend.calls_for("launchApp").last().unwrap().args,
        vec![json!("TextEdit"), json!([]), json!({ "hidden": true })]
    );
    let all = windows(&h.backend).await;
    let launched = all
        .iter()
        .find(|window| window.app_name.as_deref() == Some("TextEdit"))
        .unwrap();
    assert!(!launched.focused && !launched.visible);
    assert!(
        all.iter()
            .find(|window| window.id == "fake-terminal")
            .unwrap()
            .visible
    );
    assert!(h.backend.calls_for("raiseWindow").is_empty());
    h.dispose().await;
}

#[tokio::test]
async fn launch_app_creates_an_available_background_window_without_foreground_authorization() {
    let h = harness_with(HarnessOptions {
        backend: Scripted::new(FakeComputerBackendOptions {
            agent_dialect: Some(ComputerAgentDialect::Macos),
            ..FakeComputerBackendOptions::default()
        }),
        ..HarnessOptions::default()
    });
    h.authorize_visible_use(false);
    let before = windows(&h.backend).await;
    for options in [json!({}), json!({ "hidden": false })] {
        let mut arguments = json!({ "app": "TextEdit", "wait_for_window": false });
        arguments
            .as_object_mut()
            .unwrap()
            .extend(options.as_object().unwrap().clone());
        let result = h.call("computer_launch_app", arguments).await;
        assert!(!result.is_error, "{result:?}");
    }
    let launches: Vec<Vec<Value>> = h
        .backend
        .calls_for("launchApp")
        .into_iter()
        .map(|call| call.args)
        .collect();
    assert_eq!(
        launches,
        vec![
            vec![json!("TextEdit"), json!([])],
            vec![json!("TextEdit"), json!([]), json!({ "hidden": false })],
        ]
    );
    let after = windows(&h.backend).await;
    let launched: Vec<&ComputerWindow> = after
        .iter()
        .filter(|window| window.app_name.as_deref() == Some("TextEdit"))
        .collect();
    assert_eq!(launched.len(), 2);
    assert!(
        launched
            .iter()
            .all(|window| !window.focused && window.visible)
    );
    for existing in &before {
        assert_eq!(
            after
                .iter()
                .find(|window| window.id == existing.id)
                .map(|window| window.visible),
            Some(existing.visible)
        );
    }
    assert!(h.backend.calls_for("raiseWindow").is_empty());
    assert!(h.backend.calls_for("focusWindow").is_empty());
    h.dispose().await;
}

#[tokio::test]
async fn runs_the_visibility_lifecycle_steps_in_order_inside_one_approved_sequence() {
    let h = harness();
    let result = h
        .call(
            "computer_run",
            json!({ "steps": [
                { "type": "launch_app", "app": "org.kde.konsole", "hidden": true, "wait_for_window": false },
                { "type": "set_window_minimized", "window_id": "fake-terminal", "minimized": true },
                { "type": "set_app_visibility", "pid": 1_002, "hidden": true },
            ] }),
        )
        .await;
    assert!(!result.is_error, "{result:?}");
    assert_eq!(result_json(&result)["completed"], json!(3));
    assert_eq!(
        h.backend.calls_for("launchApp").last().unwrap().args,
        vec![
            json!("org.kde.konsole"),
            json!([]),
            json!({ "hidden": true })
        ]
    );
    assert_eq!(
        h.backend
            .calls_for("setWindowMinimized")
            .last()
            .unwrap()
            .args,
        vec![json!("fake-terminal"), json!(true)]
    );
    assert_eq!(
        h.backend.calls_for("setAppVisibility").last().unwrap().args,
        vec![json!(1_002), json!(true)]
    );
    h.dispose().await;
}

#[tokio::test]
async fn refuses_a_malformed_visibility_step_before_anything_dispatches() {
    let h = harness();
    for steps in [
        json!([{ "type": "set_window_minimized", "minimized": true }]),
        json!([{ "type": "set_window_minimized", "window_id": "fake-terminal" }]),
        json!([{ "type": "set_app_visibility", "hidden": true }]),
        json!([{ "type": "set_app_visibility", "pid": 1_002 }]),
        json!([{ "type": "set_app_visibility", "pid": -1, "hidden": true }]),
    ] {
        let result = h.call("computer_run", json!({ "steps": steps })).await;
        assert!(result.is_error, "{steps}");
    }
    assert!(h.backend.calls_for("setWindowMinimized").is_empty());
    assert!(h.backend.calls_for("setAppVisibility").is_empty());
    h.dispose().await;
}

#[tokio::test]
async fn owns_the_visibility_lifecycle_names_in_all_three_provider_spellings() {
    let h = harness();
    for name in [
        "computer_set_window_minimized",
        "computer_set_app_visibility",
    ] {
        assert!(h.tools.entry(name).is_some(), "serves {name}");
        assert!(ZERON_COMPUTER_TOOL_NAMES.contains(&name));
        assert_eq!(
            canonical_zeron_computer_tool_name(&format!("zeron_{name}")),
            Some(name)
        );
        assert_eq!(
            canonical_zeron_computer_tool_name(&format!("mcp__zeron__{name}")),
            Some(name)
        );
        assert!(is_zeron_computer_tool_family_name(&json!(name)));
    }
    h.dispose().await;
}
