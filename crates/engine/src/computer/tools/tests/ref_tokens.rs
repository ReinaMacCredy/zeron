use std::sync::Arc;

use serde_json::{Value, json};

use super::{Approvals, THREAD, ToolSeams, args, result_json};
use crate::computer::cua_backend::tests::{Fixture, FixtureOptions, fixture_with};
use crate::computer::manager::{ComputerManager, ComputerManagerEnv, ComputerManagerOptions};
use crate::computer::tools::{ComputerToolContext, ComputerTools};

const WINDOW: &str = "cua:10:20";

fn page(f: &Fixture, buttons: usize) {
    f.set_elements(
        (0..buttons)
            .map(|index| {
                json!({
                    "role": "AXButton",
                    "label": format!("Add part {index}"),
                    "frame": { "x": -290, "y": 30 + index as i64, "width": 120, "height": 1 },
                    "element_token": format!("b{index}"),
                    "actions": ["AXPress"],
                    "in_web_content": true,
                })
            })
            .collect(),
    );
    f.knobs().token_generation = Some(0);
}

fn tools(f: &Fixture) -> (Arc<ComputerManager>, Arc<ComputerTools>) {
    let mut options = ComputerManagerOptions::new(f.backend.clone());
    options.seams = Arc::new(Approvals {
        mode: std::sync::Mutex::new(Some(zeron_proto::PermissionMode::Full)),
        decide: std::sync::Mutex::new(None),
        gate: options.approval_gate.clone(),
        cards: std::sync::Mutex::default(),
        turn_inactive: std::sync::atomic::AtomicBool::new(false),
    });
    options.action_settle_ms = Some(0);
    options.env = ComputerManagerEnv::default();
    let manager = ComputerManager::new(options);
    let tools = ComputerTools::new(manager.clone(), Vec::new());
    (manager, tools)
}

fn context() -> ComputerToolContext {
    ComputerToolContext {
        chat_id: THREAD.into(),
        chat_label: None,
        turn_id: Some("turn-refs".into()),
        session_key: "session-refs".into(),
        request_id: json!(1),
        signal: tokio_util::sync::CancellationToken::new(),
        seams: Arc::new(ToolSeams::default()),
        mutation_dispatched: Arc::default(),
    }
}

fn ref_for(listing: &Value, label: &str) -> u64 {
    let tree = listing["tree"].as_str().unwrap_or_default();
    tree.lines()
        .find_map(|line| {
            let line = line.trim();
            let (number, rest) = line.split_once(' ')?;
            (rest == format!("button {label}")).then(|| number.parse().ok())?
        })
        .unwrap_or_else(|| panic!("no ref for {label} in {tree}"))
}

#[tokio::test(start_paused = true)]
async fn a_ref_past_the_first_sixty_elements_still_clicks_after_another_action() {
    let f = fixture_with(FixtureOptions {
        native_revision: Some(Some(37)),
        ..FixtureOptions::default()
    });
    page(&f, 70);
    let (manager, tools) = tools(&f);
    let context = context();
    let listing = tools
        .call(
            "computer_get_state",
            args(json!({ "window_id": WINDOW })),
            &context,
        )
        .await;
    let listing = result_json(&listing);
    let early = ref_for(&listing, "Add part 5");
    let late = ref_for(&listing, "Add part 65");
    for element_ref in [early, late] {
        let result = tools
            .call(
                "computer_click",
                args(
                    json!({ "window_id": WINDOW, "ref": element_ref, "include_screenshot": false }),
                ),
                &context,
            )
            .await;
        assert!(!result.is_error, "ref {element_ref}: {result:?}");
    }
    let clicks: Vec<String> = f
        .named("click")
        .iter()
        .filter_map(|call| {
            call.args
                .get("element_token")
                .and_then(Value::as_str)
                .map(str::to_owned)
        })
        .collect();
    assert_eq!(clicks.len(), 2, "{clicks:?}");
    assert!(clicks[1].starts_with("b65#"), "{clicks:?}");
    manager.dispose().await;
}

async fn listed(tools: &ComputerTools, context: &ComputerToolContext) -> u64 {
    let listing = tools
        .call(
            "computer_get_state",
            args(json!({ "window_id": WINDOW })),
            context,
        )
        .await;
    ref_for(&result_json(&listing), "Add part 5")
}

async fn click(tools: &ComputerTools, context: &ComputerToolContext, element_ref: u64) -> Value {
    let result = tools
        .call(
            "computer_click",
            args(json!({ "window_id": WINDOW, "ref": element_ref, "include_screenshot": false })),
            context,
        )
        .await;
    json!({ "error": result.is_error, "body": result_json(&result) })
}

#[tokio::test(start_paused = true)]
async fn a_ref_whose_token_expired_outside_the_tool_retries_once_on_a_fresh_read() {
    let f = fixture_with(FixtureOptions {
        native_revision: Some(Some(37)),
        ..FixtureOptions::default()
    });
    page(&f, 10);
    let (manager, tools) = tools(&f);
    let context = context();
    let element_ref = listed(&tools, &context).await;
    *f.knobs().token_generation.as_mut().unwrap() += 1;
    let outcome = click(&tools, &context, element_ref).await;
    assert_eq!(outcome["error"], json!(false), "{outcome}");
    assert_eq!(f.named("click").len(), 2);
    manager.dispose().await;
}

#[tokio::test(start_paused = true)]
async fn a_ref_whose_element_is_gone_fails_after_one_fresh_read() {
    let f = fixture_with(FixtureOptions {
        native_revision: Some(Some(37)),
        ..FixtureOptions::default()
    });
    page(&f, 10);
    let (manager, tools) = tools(&f);
    let context = context();
    let element_ref = listed(&tools, &context).await;
    f.knobs().elements.remove(5);
    *f.knobs().token_generation.as_mut().unwrap() += 1;
    let outcome = click(&tools, &context, element_ref).await;
    assert_eq!(outcome["error"], json!(true), "{outcome}");
    assert_eq!(
        outcome["body"]["error"],
        json!("stale_element_token"),
        "{outcome}"
    );
    assert_eq!(f.named("click").len(), 2);
    manager.dispose().await;
}
