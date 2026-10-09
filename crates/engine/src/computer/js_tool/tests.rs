use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{Value, json};
use zeron_proto::computer::{McpContent, McpToolResult};

use super::{JsHost, JsOutcome, JsRequest, run};

#[derive(Default)]
struct FakeHost {
    calls: Mutex<Vec<(String, Value)>>,
    refuse: Option<(&'static str, &'static str)>,
    refuse_browser: Option<&'static str>,
    dialog: bool,
    type_warning: bool,
    native_error: Option<&'static str>,
    delay: Duration,
}

impl FakeHost {
    fn names(&self) -> Vec<String> {
        self.calls
            .lock()
            .unwrap()
            .iter()
            .map(|(name, _)| name.clone())
            .collect()
    }

    fn args(&self, tool: &str) -> Vec<Value> {
        self.calls
            .lock()
            .unwrap()
            .iter()
            .filter(|(name, _)| name == tool)
            .map(|(_, args)| args.clone())
            .collect()
    }
}

fn arguments_has(calls: &Mutex<Vec<(String, Value)>>, key: &str) -> bool {
    calls
        .lock()
        .unwrap()
        .last()
        .is_some_and(|(_, args)| args.get(key).is_some())
}

fn json_result(value: Value) -> McpToolResult {
    McpToolResult::text(value.to_string())
}

fn structured_result(value: Value) -> McpToolResult {
    McpToolResult {
        content: vec![McpContent::Text {
            text: "browser call".into(),
        }],
        structured_content: Some(value),
        ..McpToolResult::default()
    }
}

impl JsHost for FakeHost {
    fn call(&self, tool: &str, arguments: Value) -> McpToolResult {
        self.calls
            .lock()
            .unwrap()
            .push((tool.to_owned(), arguments));
        std::thread::sleep(self.delay);
        if let Some((refused, message)) = self.refuse
            && refused == tool
        {
            return json_result(
                json!({ "error": { "code": "computer_input_paused", "message": message } }),
            );
        }
        if self.native_error == Some(tool) {
            return McpToolResult::error(
                json!({
                    "error": "same_pid_keyboard_ambiguity",
                    "message": "Background input refused.",
                    "toolGuidance": "Computer routing reminder: use computer_get_state."
                })
                .to_string(),
            );
        }
        if self.refuse_browser == Some(tool) {
            return structured_result(json!({ "status": "refused", "refusal": {
                "code": "browser_origin_refused",
                "message": "That origin is blocked."
            } }));
        }
        match tool {
            "computer_list_apps" => json_result(json!({ "apps": [
                { "pid": 7, "name": "Calculator", "bundleId": "com.apple.calculator", "running": true },
                { "pid": 9, "name": "Google Chrome", "bundleId": "com.google.Chrome", "running": true },
                { "pid": 11, "name": "Helium", "bundleId": "net.imput.helium", "running": true },
                { "pid": 12, "name": "Helium", "bundleId": "net.imput.helium", "running": true }
            ] })),
            "computer_type_text" if self.type_warning => json_result(json!({
                "action": "type_text",
                "warning": "The focused text field does not show the typed text; its value is \"x\"."
            })),
            "computer_browser_prepare" if arguments_has(&self.calls, "strategy") => {
                if self.args(tool).last().unwrap()["window_id"].is_u64() {
                    structured_result(
                        json!({ "status": "ok", "action": "attached_existing_profile" }),
                    )
                } else {
                    structured_result(json!({ "status": "refused", "refusal": {
                        "code": "browser_target_required",
                        "message": "strategy existing_profile needs its exact pid and window_id."
                    } }))
                }
            }
            "computer_browser_prepare" => {
                structured_result(json!({ "status": "ok", "prepared": true, "prepared_pid": 4242 }))
            }
            "computer_browser_state" if !arguments_has(&self.calls, "target_id") => {
                let args = self.args(tool).pop().unwrap();
                if !args["pid"].is_u64() {
                    return McpToolResult::error("Missing required integer field: pid");
                }
                match &args["window_id"] {
                    Value::Null => structured_result(json!({
                        "target_id": "bt-iso",
                        "tabs": [{ "tab_id": "tab-iso", "title": "", "url": "about:blank", "active": true }]
                    })),
                    id if id.is_u64() => structured_result(json!({
                        "target_id": "bt-1",
                        "tabs": [{ "tab_id": "tab-user", "title": "Mail", "url": "https://mail.example", "active": true }]
                    })),
                    _ => McpToolResult::error("Missing required integer field: window_id"),
                }
            }
            "computer_browser_navigate" => {
                json_result(json!({ "target_id": "bt-1", "tab_id": "tab-new", "opened_tab": true }))
            }
            "computer_list_windows" => {
                let mut windows = vec![
                    json!({ "id": "cua:9:3", "title": "Mail", "appName": "Google Chrome", "pid": 9, "visible": true,
                      "bounds": { "x": 0, "y": 0, "width": 900, "height": 700 }, "stackingIndex": 2 }),
                    json!({ "id": "cua:7:1", "title": "", "appName": "Calculator", "pid": 7, "visible": false,
                      "bounds": { "x": 0, "y": 0, "width": 1512, "height": 33 }, "stackingIndex": 1 }),
                    json!({ "id": "cua:7:2", "title": "Calculator", "appName": "Calculator", "pid": 7, "visible": true,
                      "bounds": { "x": 10, "y": 10, "width": 230, "height": 408 }, "stackingIndex": 5 }),
                ];
                if self.dialog {
                    windows.push(json!({ "id": "cua:7:9", "title": "ttgshop.vn says", "appName": "Calculator", "pid": 7,
                      "visible": true, "bounds": { "x": 40, "y": 40, "width": 150, "height": 100 }, "stackingIndex": 9 }));
                }
                json_result(json!({ "windows": windows }))
            }
            "computer_get_state" => json_result(
                json!({ "tree": "Window: \"Calculator\", App: Calculator.\n7 text Value: 42" }),
            ),
            "computer_screenshot" => McpToolResult {
                content: vec![
                    McpContent::Text {
                        text: json!({ "screenshotId": "shot-1" }).to_string(),
                    },
                    McpContent::Image {
                        data: "aGk=".into(),
                        mime_type: "image/png".into(),
                    },
                ],
                ..McpToolResult::default()
            },
            _ => json_result(json!({ "ok": true })),
        }
    }
}

async fn exec(chat: &str, host: &Arc<FakeHost>, code: &str) -> JsOutcome {
    exec_with(chat, host, code, Duration::from_secs(5)).await
}

async fn exec_with(chat: &str, host: &Arc<FakeHost>, code: &str, timeout: Duration) -> JsOutcome {
    run(
        chat,
        JsRequest {
            code: code.to_owned(),
            timeout,
            reset: false,
            host: host.clone(),
            cancelled: Arc::new(AtomicBool::new(false)),
        },
    )
    .await
}

fn texts(outcome: &JsOutcome) -> String {
    outcome
        .content
        .iter()
        .filter_map(|content| match content {
            McpContent::Text { text } => Some(text.as_str()),
            McpContent::Image { .. } => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[tokio::test]
async fn every_cua_action_is_one_typed_tool_call_bound_to_the_visible_window() {
    let host = Arc::new(FakeHost::default());
    let outcome = exec(
        "js-a",
        &host,
        "var calc = await cua.getApp(\"Calculator\"); await calc.click(9); await calc.typeText(\"7*6=\"); await calc.pressKey(\"super+c\");",
    )
    .await;
    assert_eq!(outcome.error, None, "{}", texts(&outcome));
    assert_eq!(
        host.names(),
        [
            "computer_list_apps",
            "computer_list_windows",
            "computer_get_state",
            "computer_list_windows",
            "computer_click",
            "computer_type_text",
            "computer_press_key"
        ]
    );
    assert_eq!(
        host.args("computer_click")[0]["window_id"],
        json!("cua:7:2")
    );
    assert_eq!(host.args("computer_click")[0]["ref"], json!(9));
    assert_eq!(host.args("computer_press_key")[0]["key"], json!("meta+c"));
    assert_eq!(outcome.actions.len(), 4);
    assert_eq!(outcome.actions[0]["tool"], json!("get_state"));
    assert_eq!(outcome.actions[2]["detail"], json!("7*6="));
    assert!(texts(&outcome).contains("7 text Value: 42"));
}

#[tokio::test]
async fn bindings_persist_between_calls_in_one_chat_and_not_across_chats() {
    let host = Arc::new(FakeHost::default());
    assert_eq!(
        exec("js-b", &host, "var kept = 41; let lex = 1;")
            .await
            .error,
        None
    );
    let again = exec("js-b", &host, "kept + lex").await;
    assert_eq!(again.error, None);
    assert!(texts(&again).ends_with("42"));
    let other = exec("js-c", &host, "typeof kept").await;
    assert!(texts(&other).ends_with("undefined"));
}

#[tokio::test]
async fn a_refusal_from_the_pipeline_surfaces_as_a_thrown_error_with_its_message() {
    let host = Arc::new(FakeHost {
        refuse: Some(("computer_click", "The user is using this window.")),
        ..FakeHost::default()
    });
    let outcome = exec(
        "js-d",
        &host,
        "var calc = await cua.getApp(\"Calculator\"); await calc.click(3); nodeRepl.write(\"AFTER_MARK\");",
    )
    .await;
    let error = outcome.error.clone().unwrap_or_default();
    assert!(error.contains("The user is using this window."), "{error}");
    assert!(!texts(&outcome).contains("AFTER_MARK"));
}

#[tokio::test]
async fn redeclaring_a_lexical_binding_reassigns_it_with_a_warning() {
    let host = Arc::new(FakeHost::default());
    exec("js-e", &host, "const step = 1;").await;
    let outcome = exec("js-e", &host, "const step = 2; step").await;
    assert_eq!(outcome.error, None, "{}", texts(&outcome));
    assert!(texts(&outcome).contains("step was already declared"));
}

#[tokio::test]
async fn a_runaway_script_stops_at_its_timeout() {
    let host = Arc::new(FakeHost::default());
    let outcome = exec_with("js-f", &host, "while (true) {}", Duration::from_millis(200)).await;
    let error = outcome.error.clone().unwrap_or_default();
    assert!(error.contains("did not finish"), "{error}");
    let after = exec("js-f", &host, "1 + 1").await;
    assert_eq!(after.error, None);
}

#[tokio::test]
async fn time_spent_in_the_engine_does_not_count_against_the_script_timeout() {
    let host = Arc::new(FakeHost {
        delay: Duration::from_millis(150),
        ..FakeHost::default()
    });
    let outcome = exec_with(
        "js-g",
        &host,
        "for (let i = 0; i < 3; i++) await cua.listApps({ emit: false });",
        Duration::from_millis(200),
    )
    .await;
    assert_eq!(outcome.error, None);
}

#[tokio::test]
async fn screenshots_reach_the_result_as_images_and_set_the_coordinate_frame() {
    let host = Arc::new(FakeHost::default());
    let outcome = exec(
        "js-h",
        &host,
        "var c = await cua.getApp(\"com.apple.calculator\"); await c.getScreenshot(); await c.click([10, 20]);",
    )
    .await;
    assert_eq!(outcome.error, None, "{}", texts(&outcome));
    assert!(
        outcome
            .content
            .iter()
            .any(|c| matches!(c, McpContent::Image { .. }))
    );
    assert_eq!(
        host.args("computer_click")[0]["screenshot_id"],
        json!("shot-1")
    );
}

#[tokio::test]
async fn the_first_call_in_a_chat_carries_the_api_documentation_once() {
    let host = Arc::new(FakeHost::default());
    let first = exec("js-i", &host, "1").await;
    assert!(texts(&first).contains("## Computer use"));
    let second = exec("js-i", &host, "2").await;
    assert!(!texts(&second).contains("## Computer use"));
}

#[tokio::test]
async fn a_tab_in_the_users_chrome_attaches_through_the_profile_route_and_opens_in_the_background()
{
    let host = Arc::new(FakeHost::default());
    let outcome = exec(
        "js-j",
        &host,
        "var tab = await cua.createBrowserTab(\"Chrome\", \"https://example.com\"); await tab.markDeliverable();",
    )
    .await;
    assert_eq!(outcome.error, None, "{}", texts(&outcome));
    let prepare = &host.args("computer_browser_prepare")[0];
    assert_eq!(prepare["strategy"], json!({ "kind": "existing_profile" }));
    assert_eq!(prepare["pid"], json!(9));
    assert_eq!(prepare["window_id"], json!(3));
    assert!(prepare.get("allow_launch").is_none());
    let navigate = &host.args("computer_browser_navigate")[0];
    assert_eq!(navigate["new_tab"], json!(true));
    assert_eq!(navigate["tab_id"], json!("tab-user"));
    assert!(
        host.args("computer_browser_state")
            .iter()
            .any(|args| args["keep"] == json!("deliverable") && args["tab_id"] == json!("tab-new"))
    );
}

#[tokio::test]
async fn an_isolated_tab_binds_with_the_prepared_pid() {
    let host = Arc::new(FakeHost::default());
    let outcome = exec(
        "js-iso",
        &host,
        "var tab = await cua.createBrowserTab(\"isolated\", \"https://example.com\"); tab.id",
    )
    .await;
    assert_eq!(outcome.error, None, "{}", texts(&outcome));
    assert_eq!(host.args("computer_browser_state")[0]["pid"], json!(4242));
    assert!(texts(&outcome).ends_with("tab-iso"), "{}", texts(&outcome));
}

#[tokio::test]
async fn a_refused_browser_call_throws_its_refusal_instead_of_continuing() {
    let host = Arc::new(FakeHost {
        refuse_browser: Some("computer_browser_navigate"),
        ..FakeHost::default()
    });
    let outcome = exec(
        "js-refused",
        &host,
        "await cua.createBrowserTab(\"Chrome\", \"https://example.com\"); nodeRepl.write(\"AFTER_MARK\");",
    )
    .await;
    let error = outcome.error.clone().unwrap_or_default();
    assert!(error.contains("browser_origin_refused"), "{error}");
    assert!(error.contains("That origin is blocked."), "{error}");
    assert!(!texts(&outcome).contains("AFTER_MARK"));
}

#[tokio::test]
async fn a_mid_line_redeclaration_after_a_semicolon_is_reassigned_too() {
    let host = Arc::new(FakeHost::default());
    exec("js-k", &host, "const st = 1;").await;
    let outcome = exec(
        "js-k",
        &host,
        "for (let i = 0; i < 2; i++) {} const st = 2; st",
    )
    .await;
    assert_eq!(outcome.error, None, "{}", texts(&outcome));
    assert!(texts(&outcome).ends_with("2"));
}

#[tokio::test]
async fn a_successful_result_leads_with_a_short_action_summary_and_has_no_structured_content() {
    let host = Arc::new(FakeHost::default());
    let result = exec(
        "js-l",
        &host,
        "var c = await cua.getApp(\"Calculator\"); for (const i of [3, 4, 5]) await c.click(i); await c.typeText(\"hello world\");",
    )
    .await
    .into_result();
    assert!(result.structured_content.is_none());
    let text: Vec<&str> = result
        .content
        .iter()
        .filter_map(|c| match c {
            McpContent::Text { text } => Some(text.as_str()),
            McpContent::Image { .. } => None,
        })
        .collect();
    assert!(text.iter().any(|t| t.contains("7 text Value: 42")));
    assert!(text[0].chars().count() <= 160, "{}", text[0]);
    assert_eq!(
        serde_json::from_str::<Value>(text[0]).unwrap(),
        json!({ "appName": "Calculator", "actions": [
            { "tool": "get_state" },
            { "tool": "click", "n": 3 },
            { "tool": "type_text", "detail": "hello world" }
        ] })
    );
}

#[test]
fn a_long_action_list_stays_within_the_doc_summary_limit() {
    let actions: Vec<Value> = (0..40)
        .map(|n| json!({ "tool": "type_text", "app": "TextEdit", "detail": format!("line {n} of a long letter") }))
        .collect();
    let summary = super::action_summary(&actions).unwrap();
    assert!(summary.chars().count() <= 160, "{summary}");
    assert!(
        serde_json::from_str::<Value>(&summary).unwrap()["more"]
            .as_u64()
            .unwrap()
            > 0
    );
}

#[tokio::test]
async fn ending_a_script_with_getaxstate_prints_the_state_once() {
    let host = Arc::new(FakeHost::default());
    exec("js-m", &host, "var c = await cua.getApp(\"Calculator\");").await;
    let outcome = exec("js-m", &host, "await c.getAXState();").await;
    assert_eq!(texts(&outcome).matches("7 text Value: 42").count(), 1);
}

#[tokio::test]
async fn set_timeout_resolves_an_awaited_pause_like_node() {
    let host = Arc::new(FakeHost::default());
    let outcome = exec(
        "js-n",
        &host,
        "const t0 = Date.now(); await new Promise((r) => setTimeout(r, 120)); let n = 0; const id = setInterval(() => n++, 10); await new Promise((r) => setTimeout(r, 60)); clearInterval(id); [Date.now() - t0 >= 120, n > 1]",
    )
    .await;
    assert_eq!(outcome.error, None, "{}", texts(&outcome));
    assert!(
        texts(&outcome).ends_with("[true,true]"),
        "{}",
        texts(&outcome)
    );
}

#[tokio::test]
async fn a_dialog_over_the_bound_window_is_named_in_its_state() {
    let host = Arc::new(FakeHost {
        dialog: true,
        ..FakeHost::default()
    });
    let outcome = exec("js-dialog", &host, "await cua.getApp(\"Calculator\");").await;
    assert_eq!(outcome.error, None, "{}", texts(&outcome));
    assert!(
        texts(&outcome)
            .contains("Dialog open over this window: \"ttgshop.vn says\" (window cua:7:9)"),
        "{}",
        texts(&outcome)
    );
    let plain = exec(
        "js-dialog-none",
        &Arc::new(FakeHost::default()),
        "await cua.getApp(\"Calculator\");",
    )
    .await;
    assert!(!texts(&plain).contains("Dialog open over this window: \""));
}

#[tokio::test]
async fn a_type_text_readback_warning_reaches_the_script_output() {
    let host = Arc::new(FakeHost {
        type_warning: true,
        ..FakeHost::default()
    });
    let outcome = exec(
        "js-warn",
        &host,
        "var c = await cua.getApp(\"Calculator\"); await c.typeText(\"5600X3D\");",
    )
    .await;
    assert_eq!(outcome.error, None, "{}", texts(&outcome));
    assert!(texts(&outcome).contains("Warning: The focused text field does not show"));
}

#[tokio::test]
async fn running_chromium_browsers_without_page_access_are_explained_in_the_state() {
    let host = Arc::new(FakeHost::default());
    let outcome = exec(
        "js-browsers",
        &host,
        "var s = await cua.getState({ emit: false }); nodeRepl.write(JSON.stringify([s.browsers.map(b => b.id), s.appOnlyBrowsers.map(b => b.name)])); try { await cua.createBrowserTab(\"Helium\", \"https://example.com\"); } catch (e) { nodeRepl.write(e.message); }",
    )
    .await;
    assert_eq!(outcome.error, None, "{}", texts(&outcome));
    let text = texts(&outcome);
    assert!(
        text.contains(r#"[["com.google.Chrome"],["Helium"]]"#),
        "{text}"
    );
    assert!(text.contains("Helium is Chromium-based"), "{text}");
}

#[tokio::test]
async fn a_native_error_throws_its_code_and_message_without_typed_tool_guidance() {
    let host = Arc::new(FakeHost {
        native_error: Some("computer_type_text"),
        ..FakeHost::default()
    });
    let outcome = exec(
        "js-native-error",
        &host,
        "var c = await cua.getApp(\"Calculator\"); await c.typeText(\"x\");",
    )
    .await;
    let error = outcome.error.clone().unwrap_or_default();
    assert!(
        error.contains("same_pid_keyboard_ambiguity: Background input refused."),
        "{error}"
    );
    assert!(!error.contains("Computer routing reminder"), "{error}");
}
