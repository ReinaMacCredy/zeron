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

#[tokio::test(start_paused = true)]
async fn a_gone_popup_close_never_rebinds_to_the_tab_close_with_the_same_label() {
    let f = fixture_with(FixtureOptions {
        native_revision: Some(Some(37)),
        ..FixtureOptions::default()
    });
    let close = |token: &str, y: i64| {
        json!({
            "role": "AXButton",
            "label": "Close",
            "frame": { "x": -290, "y": y, "width": 40, "height": 20 },
            "element_token": token,
            "actions": ["AXPress"],
        })
    };
    f.set_elements(vec![close("popup-close", 300), close("tab-close", 10)]);
    f.knobs().token_generation = Some(0);
    let (manager, tools) = tools(&f);
    let context = context();
    let listing = tools
        .call(
            "computer_get_state",
            args(json!({ "window_id": WINDOW })),
            &context,
        )
        .await;
    let popup = ref_for(&result_json(&listing), "Close");
    f.knobs().elements.remove(0);
    *f.knobs().token_generation.as_mut().unwrap() += 1;
    let outcome = click(&tools, &context, popup).await;
    assert_eq!(outcome["error"], json!(true), "{outcome}");
    tools
        .call(
            "computer_get_state",
            args(json!({ "window_id": WINDOW })),
            &context,
        )
        .await;
    let outcome = click(&tools, &context, popup).await;
    assert_eq!(outcome["error"], json!(true), "{outcome}");
    assert!(f.named("click").iter().all(|call| {
        call.args
            .get("element_token")
            .and_then(Value::as_str)
            .is_none_or(|token| !token.starts_with("tab-close"))
    }));
    manager.dispose().await;
}

fn wizard(f: &Fixture, step: usize) {
    let button = |label: String, token: &str, y: i64| {
        json!({
            "role": "AXButton",
            "label": label,
            "frame": { "x": -290, "y": y, "width": 60, "height": 20 },
            "element_token": token,
            "actions": ["AXPress"],
        })
    };
    f.set_elements(vec![
        button("Back".into(), "back", 30),
        button(format!("Continue to step {}", step + 1), "next", 60),
    ]);
    f.knobs().token_generation = Some(0);
}

async fn back_ref(tools: &ComputerTools, context: &ComputerToolContext) -> u64 {
    let listing = tools
        .call(
            "computer_get_state",
            args(json!({ "window_id": WINDOW })),
            context,
        )
        .await;
    ref_for(&result_json(&listing), "Back")
}

#[tokio::test(start_paused = true)]
async fn the_same_click_on_a_page_it_changed_is_progress_not_a_repeat() {
    let f = fixture_with(FixtureOptions {
        native_revision: Some(Some(37)),
        ..FixtureOptions::default()
    });
    wizard(&f, 8);
    let knobs = f.knobs.clone();
    f.knobs().tool_handlers.insert(
        "click".into(),
        Arc::new(move |_| {
            let mut knobs = knobs.lock().unwrap();
            let label = knobs.elements[1]["label"]
                .as_str()
                .unwrap_or_default()
                .to_owned();
            let step: usize = label.rsplit(' ').next().unwrap().parse().unwrap();
            knobs.elements[1]["label"] = json!(format!("Continue to step {}", step - 1));
            json!({ "structuredContent": { "effect": "unverifiable" } })
        }),
    );
    let (manager, tools) = tools(&f);
    let context = context();
    let back = back_ref(&tools, &context).await;
    for _ in 0..4 {
        let outcome = click(&tools, &context, back).await;
        assert_eq!(outcome["error"], json!(false), "{outcome}");
    }
    assert_eq!(f.named("click").len(), 4);
    manager.dispose().await;
}

#[tokio::test(start_paused = true)]
async fn the_third_same_click_on_an_unchanged_page_is_still_refused() {
    let f = fixture_with(FixtureOptions {
        native_revision: Some(Some(37)),
        ..FixtureOptions::default()
    });
    wizard(&f, 8);
    let (manager, tools) = tools(&f);
    let context = context();
    let back = back_ref(&tools, &context).await;
    for _ in 0..2 {
        let outcome = click(&tools, &context, back).await;
        assert_eq!(outcome["error"], json!(false), "{outcome}");
        tools
            .call(
                "computer_get_state",
                args(json!({ "window_id": WINDOW })),
                &context,
            )
            .await;
    }
    let outcome = click(&tools, &context, back).await;
    assert_eq!(
        outcome["body"]["error"]["code"],
        json!("repeated_unverified_action"),
        "{outcome}"
    );
    assert_eq!(f.named("click").len(), 2);
    manager.dispose().await;
}

#[tokio::test(start_paused = true)]
async fn a_moved_unique_ref_rebinds_after_an_internal_reread() {
    let f = fixture_with(FixtureOptions {
        native_revision: Some(Some(37)),
        ..FixtureOptions::default()
    });
    page(&f, 10);
    let (manager, tools) = tools(&f);
    let context = context();
    let element_ref = listed(&tools, &context).await;
    f.knobs().elements[5]["frame"]["y"] = json!(80);
    *f.knobs().token_generation.as_mut().unwrap() += 1;
    let outcome = click(&tools, &context, element_ref).await;
    assert_eq!(outcome["error"], json!(false), "{outcome}");
    let clicks = f.named("click");
    assert_eq!(clicks.len(), 2);
    assert!(
        clicks[1].args["element_token"]
            .as_str()
            .is_some_and(|token| token.starts_with("b5#")),
        "{:?}",
        clicks[1].args
    );
    manager.dispose().await;
}

#[tokio::test(start_paused = true)]
async fn a_moved_ref_whose_label_is_duplicated_still_refuses() {
    let f = fixture_with(FixtureOptions {
        native_revision: Some(Some(37)),
        ..FixtureOptions::default()
    });
    let close = |token: &str, y: i64| {
        json!({
            "role": "AXButton",
            "label": "Close",
            "frame": { "x": -290, "y": y, "width": 40, "height": 10 },
            "element_token": token,
            "actions": ["AXPress"],
        })
    };
    f.set_elements(vec![close("first-close", 30), close("second-close", 60)]);
    f.knobs().token_generation = Some(0);
    let (manager, tools) = tools(&f);
    let context = context();
    let listing = tools
        .call(
            "computer_get_state",
            args(json!({ "window_id": WINDOW })),
            &context,
        )
        .await;
    let first = ref_for(&result_json(&listing), "Close");
    f.knobs().elements[0]["frame"]["y"] = json!(90);
    *f.knobs().token_generation.as_mut().unwrap() += 1;
    let outcome = click(&tools, &context, first).await;
    assert_eq!(outcome["error"], json!(true), "{outcome}");
    assert_eq!(outcome["body"]["error"], json!("stale_element_token"));
    manager.dispose().await;
}

fn refuse_outside_window(f: &Fixture, refusals: usize) {
    let remaining = Arc::new(std::sync::atomic::AtomicUsize::new(refusals));
    f.knobs().tool_handlers.insert(
        "click".into(),
        Arc::new(move |_| {
            let refuse = remaining
                .fetch_update(
                    std::sync::atomic::Ordering::SeqCst,
                    std::sync::atomic::Ordering::SeqCst,
                    |left| left.checked_sub(1),
                )
                .is_ok();
            if refuse {
                json!({
                    "isError": true,
                    "structuredContent": {
                        "code": "element_outside_target_window",
                        "effect": "refused",
                    },
                })
            } else {
                json!({ "structuredContent": { "effect": "unverifiable" } })
            }
        }),
    );
}

#[tokio::test(start_paused = true)]
async fn an_element_not_proven_in_its_window_is_reread_and_retried_once() {
    let f = fixture_with(FixtureOptions {
        native_revision: Some(Some(37)),
        ..FixtureOptions::default()
    });
    page(&f, 10);
    refuse_outside_window(&f, 1);
    let (manager, tools) = tools(&f);
    let context = context();
    let element_ref = listed(&tools, &context).await;
    let outcome = click(&tools, &context, element_ref).await;
    assert_eq!(outcome["error"], json!(false), "{outcome}");
    assert_eq!(f.named("click").len(), 2);
    manager.dispose().await;
}

#[tokio::test(start_paused = true)]
async fn an_element_that_stays_outside_its_window_still_refuses() {
    let f = fixture_with(FixtureOptions {
        native_revision: Some(Some(37)),
        ..FixtureOptions::default()
    });
    page(&f, 10);
    refuse_outside_window(&f, usize::MAX);
    let (manager, tools) = tools(&f);
    let context = context();
    let element_ref = listed(&tools, &context).await;
    let outcome = click(&tools, &context, element_ref).await;
    assert_eq!(
        outcome["body"]["error"],
        json!("element_outside_target_window"),
        "{outcome}"
    );
    assert_eq!(f.named("click").len(), 2);
    manager.dispose().await;
}

#[tokio::test(start_paused = true)]
async fn ref_and_window_scrolls_need_no_screenshot_after_the_desktop_changed() {
    let f = fixture_with(FixtureOptions {
        native_revision: Some(Some(37)),
        ..FixtureOptions::default()
    });
    page(&f, 10);
    let (manager, tools) = tools(&f);
    let context = context();
    let shot = tools
        .call(
            "computer_screenshot",
            args(json!({ "window_id": WINDOW })),
            &context,
        )
        .await;
    assert!(!shot.is_error, "{shot:?}");
    let element_ref = listed(&tools, &context).await;
    f.knobs().desktop_epoch += 1;
    for scroll in [
        json!({ "window_id": WINDOW, "ref": element_ref, "delta_y": 300, "include_screenshot": false }),
        json!({ "window_id": WINDOW, "delta_y": 300, "include_screenshot": false }),
    ] {
        let result = tools
            .call("computer_scroll", args(scroll.clone()), &context)
            .await;
        assert!(!result.is_error, "{scroll}: {:?}", result_json(&result));
    }
    let scrolls = f.named("scroll");
    assert_eq!(scrolls.len(), 2);
    assert!(
        scrolls[0].args["element_token"]
            .as_str()
            .is_some_and(|token| token.starts_with("b5#"))
    );
    manager.dispose().await;
}
