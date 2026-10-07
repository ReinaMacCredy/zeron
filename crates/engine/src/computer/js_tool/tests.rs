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

fn json_result(value: Value) -> McpToolResult {
    McpToolResult::text(value.to_string())
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
        match tool {
            "computer_list_apps" => json_result(json!({ "apps": [
                { "pid": 7, "name": "Calculator", "bundleId": "com.apple.calculator", "running": true }
            ] })),
            "computer_list_windows" => json_result(json!({ "windows": [
                { "id": "cua:7:1", "title": "", "appName": "Calculator", "pid": 7, "visible": false,
                  "bounds": { "x": 0, "y": 0, "width": 1512, "height": 33 }, "stackingIndex": 1 },
                { "id": "cua:7:2", "title": "Calculator", "appName": "Calculator", "pid": 7, "visible": true,
                  "bounds": { "x": 10, "y": 10, "width": 230, "height": 408 }, "stackingIndex": 5 }
            ] })),
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
    assert_eq!(outcome.actions.len(), 3);
    assert_eq!(outcome.actions[1]["detail"], json!("7*6="));
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
