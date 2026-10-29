use super::*;

use std::path::{Path, PathBuf};

use zeron_proto::computer::{ComputerApp, ComputerPoint, McpContent};

use crate::computer::approval_gate::ApprovalDecision;
use crate::computer::backend::{ComputerBackend, ComputerBackendError, ComputerBackendMethod};
use crate::computer::manager::tests::rect;
use crate::computer::tools::COMPUTER_APPROVAL_REQUIRED_TOOLS;
use crate::computer::tools::tool_permission::{
    ZERON_COMPUTER_TOOL_NAMES, canonical_zeron_computer_tool_name,
    is_zeron_computer_tool_family_name,
};
use crate::computer::unavailable_backend::UnavailableComputerBackend;

fn result_text(result: &McpToolResult) -> String {
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

fn read_only(h: &Harness, name: &str) -> Value {
    h.tools
        .entry(name)
        .and_then(|entry| entry.definition.annotations)
        .map(|annotations| annotations["readOnlyHint"].clone())
        .unwrap_or(Value::Null)
}

fn requires_approval(name: &str) -> bool {
    COMPUTER_APPROVAL_REQUIRED_TOOLS.contains(&name)
}

fn cards(h: &Harness) -> Vec<ComputerConsentCard> {
    h.approvals.cards.lock().unwrap().clone()
}

struct Unavailable {
    tools: Arc<ComputerTools>,
    manager: Arc<ComputerManager>,
    seams: Arc<ToolSeams>,
}

impl Unavailable {
    fn new(message: &str) -> Self {
        let backend = Arc::new(UnavailableComputerBackend::new(message, Default::default()));
        let mut options = ComputerManagerOptions::new(backend);
        options.seams = Arc::new(Approvals {
            mode: Mutex::new(Some(AccessMode::FullAccess)),
            decide: Mutex::new(None),
            gate: options.approval_gate.clone(),
            cards: Mutex::default(),
            turn_inactive: AtomicBool::new(false),
        });
        options.action_settle_ms = Some(0);
        options.env = ComputerManagerEnv::default();
        let manager = ComputerManager::new(options);
        Self {
            tools: ComputerTools::new(manager.clone(), Vec::new()),
            manager,
            seams: Arc::new(ToolSeams::default()),
        }
    }

    async fn call(&self, name: &str, arguments: Value) -> McpToolResult {
        let context = ComputerToolContext {
            chat_id: THREAD.to_owned(),
            chat_label: None,
            turn_id: Some(TURN.to_owned()),
            session_key: "session-computer".to_owned(),
            request_id: json!(1),
            signal: CancellationToken::new(),
            seams: self.seams.clone(),
            mutation_dispatched: Arc::default(),
        };
        self.tools.call(name, args(arguments), &context).await
    }
}

async fn bounds_of(h: &Harness, window_id: &str) -> Option<zeron_proto::computer::ComputerRect> {
    h.backend
        .list_windows()
        .await
        .unwrap()
        .into_iter()
        .find(|window| window.id == window_id)
        .and_then(|window| window.bounds)
}

#[tokio::test]
async fn lists_apps_with_the_metadata_the_driver_reports_without_an_approval_gate() {
    let h = harness_with(HarnessOptions {
        backend: fake_with(FakeComputerBackendOptions {
            apps: Some(vec![
                ComputerApp {
                    pid: 42,
                    name: "TextEdit".into(),
                    bundle_id: Some("com.apple.TextEdit".into()),
                    team_id: None,
                    running: true,
                    active: true,
                    launch_path: Some("/System/Applications/TextEdit.app".into()),
                    window_count: Some(1),
                    last_used: None,
                },
                ComputerApp {
                    pid: 0,
                    name: "Chess".into(),
                    bundle_id: Some("com.apple.Chess".into()),
                    team_id: None,
                    running: false,
                    active: false,
                    launch_path: Some("/System/Applications/Chess.app".into()),
                    window_count: None,
                    last_used: None,
                },
            ]),
            ..FakeComputerBackendOptions::default()
        }),
        ..HarnessOptions::default()
    });
    h.decide(ApprovalDecision::Accept);
    assert_eq!(read_only(&h, "computer_list_apps"), json!(true));
    assert!(!requires_approval("computer_list_apps"));

    let result = h.call("computer_list_apps", json!({})).await;
    assert!(!result.is_error, "{result:?}");
    let payload = result_json(&result);
    assert_eq!(payload["computerId"], json!("desktop"));
    assert_eq!(payload["availability"]["kind"], json!("available"));
    let apps = payload["apps"].as_array().unwrap();
    assert_eq!(apps.len(), 2);
    assert_eq!(apps[0]["pid"].as_f64(), Some(42.0));
    assert_eq!(apps[0]["name"], json!("TextEdit"));
    assert_eq!(apps[0]["bundleId"], json!("com.apple.TextEdit"));
    assert_eq!(apps[0]["running"], json!(true));
    assert_eq!(apps[0]["active"], json!(true));
    assert_eq!(
        apps[0]["launchPath"],
        json!("/System/Applications/TextEdit.app")
    );
    assert_eq!(apps[0]["windowCount"].as_f64(), Some(1.0));
    assert_eq!(apps[1]["pid"].as_f64(), Some(0.0));
    assert_eq!(apps[1]["name"], json!("Chess"));
    assert_eq!(apps[1]["running"], json!(false));
    assert_eq!(apps[1]["active"], json!(false));
    assert!(cards(&h).is_empty());
    h.dispose().await;
}

#[tokio::test]
async fn refuses_cleanly_on_a_backend_that_cannot_enumerate_applications() {
    let h = harness();
    h.backend.remove(ComputerBackendMethod::ListApps);
    h.backend.remove(ComputerBackendMethod::SetWindowFrame);
    h.backend.remove(ComputerBackendMethod::InvokeMenu);
    let result = h.call("computer_list_apps", json!({})).await;
    assert!(result.is_error);
    assert!(
        result_text(&result).contains("cannot enumerate applications"),
        "{result:?}"
    );
    h.dispose().await;
}

#[tokio::test]
async fn surfaces_the_unavailable_backend_s_own_message() {
    let unavailable = Unavailable::new("the display link is gone");
    let result = unavailable.call("computer_list_apps", json!({})).await;
    assert!(result.is_error);
    assert!(
        result_text(&result).contains("the display link is gone"),
        "{result:?}"
    );
    unavailable.manager.dispose().await;
}

#[tokio::test]
async fn is_approval_gated_and_moves_the_exact_window_reporting_a_verified_result() {
    let h = harness();
    h.decide(ApprovalDecision::Accept);
    assert!(requires_approval("computer_set_window_frame"));

    let result = h
        .call(
            "computer_set_window_frame",
            json!({ "window_id": "fake-calculator", "x": 200, "y": 300, "width": 800, "height": 600 }),
        )
        .await;
    let raised = cards(&h);
    assert_eq!(raised.len(), 1);
    assert_eq!(raised[0].tool_name, "computer_set_window_frame");
    assert!(
        raised[0].params_display.contains("fake-calculator"),
        "{:?}",
        raised[0]
    );
    assert!(!result.is_error, "{result:?}");
    let payload = result_json(&result);
    assert_eq!(payload["action"], json!("computer_set_window_frame"));
    assert_eq!(payload["windowId"], json!("fake-calculator"));
    assert_eq!(payload["delivery"]["verified"], json!("confirmed"));
    assert_eq!(payload["delivery"]["effect"], json!("verified"));
    assert_eq!(
        bounds_of(&h, "fake-calculator").await,
        Some(rect(200.0, 300.0, 800.0, 600.0))
    );
    h.dispose().await;
}

#[tokio::test]
async fn reports_dispatched_unknown_when_the_readback_does_not_show_the_frame() {
    let h = harness();
    h.backend.fake.set_frame_applies(false);
    let result = h
        .call(
            "computer_set_window_frame",
            json!({ "window_id": "fake-calculator", "x": 0, "y": 0, "width": 640, "height": 480 }),
        )
        .await;
    assert!(!result.is_error, "{result:?}");
    let payload = result_json(&result);
    assert_eq!(payload["delivery"]["verified"], json!("unconfirmed"));
    assert_eq!(payload["delivery"]["effect"], json!("dispatched-unknown"));
    h.dispose().await;
}

#[tokio::test]
async fn refuses_a_nonpositive_frame_and_an_unknown_window_before_dispatch() {
    let h = harness();
    h.decide(ApprovalDecision::Accept);
    let bad_size = h
        .call(
            "computer_set_window_frame",
            json!({ "window_id": "fake-calculator", "x": 0, "y": 0, "width": 0, "height": 480 }),
        )
        .await;
    assert!(bad_size.is_error, "{bad_size:?}");
    let missing_window = h
        .call(
            "computer_set_window_frame",
            json!({ "window_id": "no-such-window", "x": 0, "y": 0, "width": 100, "height": 100 }),
        )
        .await;
    assert!(missing_window.is_error, "{missing_window:?}");
    let missing_id = h
        .call(
            "computer_set_window_frame",
            json!({ "x": 0, "y": 0, "width": 100, "height": 100 }),
        )
        .await;
    assert!(missing_id.is_error, "{missing_id:?}");
    assert!(h.backend.calls_for("setWindowFrame").is_empty());
    h.dispose().await;
}

#[tokio::test]
async fn frame_dispatches_nothing_when_approval_is_refused() {
    let h = harness();
    h.decide(ApprovalDecision::Decline);
    let result = h
        .call(
            "computer_set_window_frame",
            json!({ "window_id": "fake-calculator", "x": 0, "y": 0, "width": 100, "height": 100 }),
        )
        .await;
    assert!(result.is_error, "{result:?}");
    assert_eq!(cards(&h).len(), 1);
    assert!(h.backend.calls_for("setWindowFrame").is_empty());
    assert_eq!(
        bounds_of(&h, "fake-calculator").await,
        Some(rect(1_050.0, 120.0, 420.0, 620.0))
    );
    h.dispose().await;
}

#[tokio::test]
async fn does_not_promise_background_menu_execution_even_when_visible_use_is_authorized() {
    let h = harness();
    h.decide(ApprovalDecision::Accept);
    h.authorize_visible_use(true);
    let schema = h
        .tools
        .entry("computer_invoke_menu")
        .unwrap()
        .definition
        .input_schema;
    assert_eq!(
        schema["properties"]["delivery_mode"]["enum"],
        json!(["foreground"])
    );
    let result = h
        .call(
            "computer_invoke_menu",
            json!({ "window_id": "fake-calculator", "path": ["File"], "delivery_mode": "background" }),
        )
        .await;
    assert!(result.is_error);
    assert!(
        result_text(&result).contains("cannot preserve background focus"),
        "{result:?}"
    );
    assert!(cards(&h).is_empty());
    assert!(h.backend.calls_for("invokeMenu").is_empty());
    h.dispose().await;
}

#[tokio::test]
async fn is_approval_gated_and_invokes_the_exact_path_on_the_window_s_app() {
    let h = harness();
    h.decide(ApprovalDecision::Accept);
    assert!(requires_approval("computer_invoke_menu"));

    let result = h
        .call(
            "computer_invoke_menu",
            json!({ "window_id": "fake-terminal", "path": ["File", "Save"] }),
        )
        .await;
    assert!(!result.is_error, "{result:?}");
    let raised = cards(&h);
    assert_eq!(raised.len(), 1);
    assert_eq!(raised[0].tool_name, "computer_invoke_menu");
    let payload = result_json(&result);
    assert_eq!(payload["action"], json!("computer_invoke_menu"));
    assert_eq!(payload["windowId"], json!("fake-terminal"));
    assert_eq!(
        h.backend.calls_for("invokeMenu").last().unwrap().args,
        vec![
            json!({ "windowId": "fake-terminal" }),
            json!(["File", "Save"])
        ]
    );
    h.dispose().await;
}

#[tokio::test]
async fn bounds_the_path_the_same_way_the_schema_advertises() {
    let h = harness();
    h.decide(ApprovalDecision::Accept);
    let empty = h
        .call(
            "computer_invoke_menu",
            json!({ "window_id": "fake-terminal", "path": [] }),
        )
        .await;
    assert!(empty.is_error, "{empty:?}");
    let too_deep = h
        .call(
            "computer_invoke_menu",
            json!({ "window_id": "fake-terminal", "path": ["1", "2", "3", "4", "5", "6", "7"] }),
        )
        .await;
    assert!(too_deep.is_error, "{too_deep:?}");
    let blank = h
        .call(
            "computer_invoke_menu",
            json!({ "window_id": "fake-terminal", "path": ["File", "   "] }),
        )
        .await;
    assert!(blank.is_error, "{blank:?}");
    assert!(h.backend.calls_for("invokeMenu").is_empty());
    h.dispose().await;
}

#[tokio::test]
async fn preserves_a_disabled_item_refusal_instead_of_falling_back_to_pixels() {
    let h = harness();
    h.decide(ApprovalDecision::Accept);
    h.backend.fake.refuse_menu_path(
        &["Edit", "Undo"],
        ComputerBackendError {
            rejected_operation: Some("invokeMenu".into()),
            ..ComputerBackendError::new("The menu item is disabled.")
        },
    );
    let result = h
        .call(
            "computer_invoke_menu",
            json!({ "window_id": "fake-terminal", "path": ["Edit", "Undo"] }),
        )
        .await;
    assert!(result.is_error);
    assert!(result_text(&result).contains("disabled"), "{result:?}");
    let replay = h
        .call(
            "computer_invoke_menu",
            json!({ "window_id": "fake-terminal", "path": ["Edit", "Undo"] }),
        )
        .await;
    assert!(replay.is_error);
    assert!(h.backend.calls_for("click").is_empty());
    h.dispose().await;
}

#[tokio::test]
async fn drives_another_app_s_menus_without_asking() {
    let h = harness();
    let first = h
        .call(
            "computer_invoke_menu",
            json!({ "window_id": "fake-terminal", "path": ["File"] }),
        )
        .await;
    assert!(!first.is_error, "{first:?}");
    let second = h
        .call(
            "computer_invoke_menu",
            json!({ "window_id": "fake-calculator", "path": ["File"] }),
        )
        .await;
    assert!(!second.is_error, "{second:?}");
    assert_eq!(h.backend.calls_for("invokeMenu").len(), 2);
    assert!(cards(&h).is_empty());
    h.dispose().await;
}

#[tokio::test]
async fn menu_dispatches_nothing_when_approval_is_refused() {
    let h = harness();
    h.decide(ApprovalDecision::Decline);
    let result = h
        .call(
            "computer_invoke_menu",
            json!({ "window_id": "fake-terminal", "path": ["File", "Quit"] }),
        )
        .await;
    assert!(result.is_error, "{result:?}");
    assert_eq!(cards(&h).len(), 1);
    assert!(h.backend.calls_for("invokeMenu").is_empty());
    h.dispose().await;
}

#[tokio::test]
async fn returns_the_desktop_inventory_without_an_approval_gate() {
    let h = harness();
    h.decide(ApprovalDecision::Accept);
    assert_eq!(
        read_only(&h, "computer_get_accessibility_tree"),
        json!(true)
    );
    assert!(!requires_approval("computer_get_accessibility_tree"));

    let result = h.call("computer_get_accessibility_tree", json!({})).await;
    assert!(!result.is_error, "{result:?}");
    let payload = result_json(&result);
    assert_eq!(payload["computerId"], json!("desktop"));
    assert_eq!(payload["truncated"], json!(false));
    let pids: Vec<Option<f64>> = payload["apps"]
        .as_array()
        .unwrap()
        .iter()
        .map(|app| app["pid"].as_f64())
        .collect();
    assert_eq!(pids, [Some(1_001.0), Some(1_002.0)]);
    let windows: Vec<&Value> = payload["windows"]
        .as_array()
        .unwrap()
        .iter()
        .map(|window| &window["id"])
        .collect();
    assert_eq!(
        windows,
        [&json!("fake-terminal"), &json!("fake-calculator")]
    );
    assert!(cards(&h).is_empty());
    let calls = h.backend.calls_for("getAccessibilityTree");
    assert_eq!(calls.len(), 1);
    assert!(calls[0].args.is_empty());
    h.dispose().await;
}

#[tokio::test]
async fn scopes_the_snapshot_to_the_app_owning_the_exact_window() {
    let h = harness();
    let scoped = h
        .call(
            "computer_get_accessibility_tree",
            json!({ "window_id": "fake-calculator" }),
        )
        .await;
    assert!(!scoped.is_error, "{scoped:?}");
    let payload = result_json(&scoped);
    assert_eq!(payload["windowId"], json!("fake-calculator"));
    let apps = payload["apps"].as_array().unwrap();
    assert_eq!(apps.len(), 1);
    assert_eq!(apps[0]["pid"].as_f64(), Some(1_002.0));
    let windows: Vec<&Value> = payload["windows"]
        .as_array()
        .unwrap()
        .iter()
        .map(|window| &window["id"])
        .collect();
    assert_eq!(windows, [&json!("fake-calculator")]);
    assert_eq!(
        h.backend.calls_for("getAccessibilityTree")[0].args,
        vec![json!("fake-calculator")]
    );

    let missing = h
        .call(
            "computer_get_accessibility_tree",
            json!({ "window_id": "no-such-window" }),
        )
        .await;
    assert!(missing.is_error, "{missing:?}");
    assert_eq!(h.backend.calls_for("getAccessibilityTree").len(), 1);
    h.dispose().await;
}

#[tokio::test]
async fn tree_refuses_cleanly_on_a_backend_without_the_read_and_carries_the_unavailable_message() {
    let h = harness();
    h.backend
        .remove(ComputerBackendMethod::GetAccessibilityTree);
    let result = h.call("computer_get_accessibility_tree", json!({})).await;
    assert!(result.is_error);
    assert!(
        result_text(&result).contains("cannot read the desktop inventory"),
        "{result:?}"
    );
    h.dispose().await;

    let unavailable = Unavailable::new("the display link is gone");
    let refused = unavailable
        .call("computer_get_accessibility_tree", json!({}))
        .await;
    assert!(refused.is_error);
    assert!(
        result_text(&refused).contains("the display link is gone"),
        "{refused:?}"
    );
    unavailable.manager.dispose().await;
}

#[tokio::test]
async fn reads_the_pointer_without_an_approval_gate_or_a_control_lease() {
    let h = harness();
    h.decide(ApprovalDecision::Accept);
    h.backend
        .fake
        .set_cursor_position(ComputerPoint { x: 410.0, y: 240.0 });
    assert_eq!(read_only(&h, "computer_get_cursor_position"), json!(true));
    assert!(!requires_approval("computer_get_cursor_position"));

    let result = h.call("computer_get_cursor_position", json!({})).await;
    assert!(!result.is_error, "{result:?}");
    let payload = result_json(&result);
    assert_eq!(payload["computerId"], json!("desktop"));
    assert_eq!(payload["x"].as_f64(), Some(410.0));
    assert_eq!(payload["y"].as_f64(), Some(240.0));
    assert!(cards(&h).is_empty());
    assert_eq!(h.backend.calls_for("getCursorPosition").len(), 1);
    h.dispose().await;
}

#[tokio::test]
async fn reports_containment_when_scoped_to_a_window_and_refuses_a_dead_one() {
    let h = harness();
    h.backend.fake.set_cursor_position(ComputerPoint {
        x: 1_100.0,
        y: 200.0,
    });
    let inside = h
        .call(
            "computer_get_cursor_position",
            json!({ "window_id": "fake-calculator" }),
        )
        .await;
    assert!(!inside.is_error, "{inside:?}");
    let payload = result_json(&inside);
    assert_eq!(payload["windowId"], json!("fake-calculator"));
    assert_eq!(payload["insideWindow"], json!(true));

    h.backend
        .fake
        .set_cursor_position(ComputerPoint { x: 10.0, y: 10.0 });
    let outside = h
        .call(
            "computer_get_cursor_position",
            json!({ "window_id": "fake-calculator" }),
        )
        .await;
    assert_eq!(result_json(&outside)["insideWindow"], json!(false));

    let missing = h
        .call(
            "computer_get_cursor_position",
            json!({ "window_id": "no-such-window" }),
        )
        .await;
    assert!(missing.is_error, "{missing:?}");
    h.dispose().await;
}

#[tokio::test]
async fn cursor_refuses_cleanly_on_a_backend_without_the_read_and_carries_the_unavailable_message()
{
    let h = harness();
    h.backend.remove(ComputerBackendMethod::GetCursorPosition);
    let result = h.call("computer_get_cursor_position", json!({})).await;
    assert!(result.is_error);
    assert!(
        result_text(&result).contains("cannot read the cursor position"),
        "{result:?}"
    );
    h.dispose().await;

    let unavailable = Unavailable::new("the display link is gone");
    let refused = unavailable
        .call("computer_get_cursor_position", json!({}))
        .await;
    assert!(refused.is_error);
    assert!(
        result_text(&refused).contains("the display link is gone"),
        "{refused:?}"
    );
    unavailable.manager.dispose().await;
}

#[tokio::test]
async fn owns_every_served_tool_name_in_all_three_provider_spellings() {
    let h = harness();
    let served: Vec<String> = h
        .tools
        .entries()
        .iter()
        .map(|entry| entry.name().to_owned())
        .collect();
    for name in [
        "computer_list_apps",
        "computer_set_window_frame",
        "computer_invoke_menu",
        "computer_verify_state",
        "computer_zoom",
        "computer_get_accessibility_tree",
        "computer_get_cursor_position",
        "computer_kill_app",
    ] {
        assert!(served.iter().any(|entry| entry == name), "served {name}");
    }
    for name in &served {
        let name = name.as_str();
        assert!(ZERON_COMPUTER_TOOL_NAMES.contains(&name), "{name}");
        assert_eq!(canonical_zeron_computer_tool_name(name), Some(name));
        assert_eq!(
            canonical_zeron_computer_tool_name(&format!("zeron_{name}")),
            Some(name)
        );
        assert_eq!(
            canonical_zeron_computer_tool_name(&format!("mcp__zeron__{name}")),
            Some(name)
        );
        assert!(is_zeron_computer_tool_family_name(&json!(name)), "{name}");
    }
    h.dispose().await;
}

#[tokio::test]
async fn runs_frame_and_menu_steps_in_order_inside_one_approved_sequence() {
    let h = harness();
    h.decide(ApprovalDecision::Accept);
    let result = h
        .call(
            "computer_run",
            json!({ "steps": [
                {
                    "type": "set_window_frame",
                    "window_id": "fake-calculator",
                    "x": 10,
                    "y": 10,
                    "width": 500,
                    "height": 400,
                },
                { "type": "invoke_menu", "window_id": "fake-terminal", "path": ["File"] },
            ] }),
        )
        .await;
    assert!(!result.is_error, "{result:?}");
    assert_eq!(result_json(&result)["completed"], json!(2), "{result:?}");
    assert_eq!(
        h.backend.calls_for("setWindowFrame").last().unwrap().args,
        vec![
            json!("fake-calculator"),
            json!(rect(10.0, 10.0, 500.0, 400.0))
        ]
    );
    assert_eq!(
        h.backend.calls_for("invokeMenu").last().unwrap().args,
        vec![json!({ "windowId": "fake-terminal" }), json!(["File"])]
    );
    let methods = h.backend.methods();
    let frame_at = methods.iter().position(|m| m == "setWindowFrame").unwrap();
    let menu_at = methods.iter().position(|m| m == "invokeMenu").unwrap();
    assert!(frame_at < menu_at, "{methods:?}");
    h.dispose().await;
}

#[tokio::test]
async fn refuses_a_run_step_missing_its_window_or_a_menu_path_that_is_too_deep() {
    let h = harness();
    h.decide(ApprovalDecision::Accept);
    let missing_window = h
        .call(
            "computer_run",
            json!({ "steps": [{ "type": "set_window_frame", "x": 0, "y": 0, "width": 10, "height": 10 }] }),
        )
        .await;
    assert!(missing_window.is_error, "{missing_window:?}");
    let deep_menu = h
        .call(
            "computer_run",
            json!({ "steps": [{
                "type": "invoke_menu",
                "window_id": "fake-terminal",
                "path": ["1", "2", "3", "4", "5", "6", "7"],
            }] }),
        )
        .await;
    assert!(deep_menu.is_error, "{deep_menu:?}");
    assert!(h.backend.calls_for("invokeMenu").is_empty());
    assert!(h.backend.calls_for("setWindowFrame").is_empty());
    h.dispose().await;
}

const TOOL_SERVER_HARNESSES: &[&str] = &["acp", "claude", "codex"];

fn harness_src() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../harness/src")
}

fn production_source(harness_id: &str) -> String {
    let source = std::fs::read_to_string(harness_src().join(harness_id).join("mod.rs")).unwrap();
    let production = source
        .split("#[cfg(test)]\nmod tests")
        .next()
        .unwrap_or_default();
    production
        .lines()
        .map(|line| match line.find("//") {
            Some(at) if !line[..at].contains('"') => &line[..at],
            _ => line,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn covers_every_harness_that_forwards_the_tool_server() {
    let mut forwarding: Vec<String> = std::fs::read_dir(harness_src())
        .unwrap()
        .filter_map(Result::ok)
        .filter(|entry| entry.path().is_dir())
        .filter_map(|entry| entry.file_name().into_string().ok())
        .filter(|harness_id| {
            harness_src().join(harness_id).join("mod.rs").exists()
                && production_source(harness_id).contains("tool_server")
        })
        .collect();
    forwarding.sort();
    assert_eq!(forwarding, TOOL_SERVER_HARNESSES);
}

#[test]
fn every_harness_forwards_the_tool_server_env_it_was_handed() {
    let sites: &[(&str, &[&str])] = &[
        ("acp", &["\"env\":server.env.iter()"]),
        ("claude", &["server.mcp_config_json(\"zeron\")"]),
        (
            "codex",
            &[
                "\"mcp_servers.zeron.env\"",
                "Value::Object(server.env_map())",
            ],
        ),
    ];
    for (harness_id, needles) in sites {
        let source: String = production_source(harness_id).split_whitespace().collect();
        for needle in *needles {
            assert!(
                source.contains(needle),
                "{harness_id} no longer forwards ToolServer.env via {needle:?}"
            );
        }
    }
    let lib = std::fs::read_to_string(harness_src().join("lib.rs")).unwrap();
    let lib: String = lib.split_whitespace().collect();
    assert!(lib.contains("\"env\":self.env_map(),"));
}

#[test]
fn no_harness_names_a_computer_capability_or_the_run_secret_itself() {
    for harness_id in TOOL_SERVER_HARNESSES {
        let source = production_source(harness_id);
        for needle in [
            zeron_rpc::COMPUTER_RUN_SECRET_ENV,
            "COMPUTER_RUN_SECRET_ENV",
            "computer:control",
        ] {
            assert!(
                !source.contains(needle),
                "{harness_id} names {needle:?}; the engine owns the run secret"
            );
        }
    }
}
