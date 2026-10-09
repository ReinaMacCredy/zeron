use super::*;

use zeron_proto::computer::{ComputerDeliveryMode, ComputerDeliveryVerification, ComputerHostCall};

use crate::computer::cua_backend::tests::{Fixture, FixtureOptions, assert_subset, fixture_with};
use crate::computer::desktop_operation_queue::with_desktop_delivery_mode;
use crate::computer::element_identity::{
    ObservedComputerTarget, bind_computer_target_ref, computer_element_ref_identity,
};
use crate::computer::manager::ComputerClickGesture;
use crate::computer::model_desktop_observation::with_model_desktop_observation;
use crate::computer::task_context::with_computer_task;
use crate::computer::ui_tree_targeting::{
    ComputerActionableElementFilter, ComputerActionableElements, actionable_elements,
};

const THREAD: &str = "test-thread";
const TURN: &str = "test-turn";
const WINDOW: &str = "cua:10:20";

fn native(revision: u64) -> Fixture {
    fixture_with(FixtureOptions {
        native_revision: Some(Some(revision)),
        ..FixtureOptions::default()
    })
}

fn gateway_manager(f: &Fixture) -> Arc<ComputerManager> {
    let mut options = ComputerManagerOptions::new(f.backend.clone());
    options.action_settle_ms = Some(0);
    options.env = ComputerManagerEnv::default();
    ComputerManager::new(options)
}

fn task() -> CuaComputerTask {
    CuaComputerTask {
        thread_id: THREAD.to_owned(),
        turn_id: Some(TURN.to_owned()),
        label: None,
    }
}

async fn observe(f: &Fixture) -> ComputerActionableElements {
    let state = with_computer_task(
        task(),
        with_model_desktop_observation(f.backend.get_state(ComputerStateOptions {
            include_tree: true,
            window_id: Some(WINDOW.to_owned()),
            ..ComputerStateOptions::default()
        })),
    )
    .await
    .unwrap();
    let root = state.root.unwrap();
    actionable_elements(
        &root,
        &ComputerActionableElementFilter {
            window_id: Some(WINDOW.to_owned()),
            label_contains: None,
        },
        |node| f.backend.native_element_identity(node),
    )
}

fn bind(listing: &ComputerActionableElements, index: usize) -> ObservedComputerTarget {
    let entry = &listing.ref_index[index];
    bind_computer_target_ref(
        ComputerTarget {
            label: Some(entry.label.clone()),
            role: Some(entry.role.clone()),
            window_id: entry.window_id.clone(),
            ref_ordinal: Some(entry.ordinal as u64),
            ..ComputerTarget::default()
        },
        entry,
    )
}

fn identities(listing: &ComputerActionableElements) -> Vec<Option<&str>> {
    listing
        .ref_index
        .iter()
        .map(computer_element_ref_identity)
        .collect()
}

async fn drive<A, F, Fut>(manager: &ComputerManager, action: F) -> Result<A, ComputerError>
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = Result<A, ComputerError>>,
{
    manager
        .with_agent_activity(
            THREAD,
            || {
                with_desktop_delivery_mode(
                    ComputerDeliveryMode::Background,
                    with_computer_task(task(), action()),
                )
            },
            None,
            Some(TURN),
            None,
        )
        .await
}

fn since(f: &Fixture, before: usize) -> Vec<ComputerHostCall> {
    f.calls().split_off(before)
}

fn names_since(f: &Fixture, before: usize) -> Vec<String> {
    since(f, before)
        .into_iter()
        .filter_map(|call| call.name)
        .collect()
}

fn element_value(f: &Fixture, token: &str) -> Value {
    f.knobs()
        .elements
        .iter()
        .find(|element| element["element_token"] == json!(token))
        .map(|element| element["value"].clone())
        .unwrap_or(Value::Null)
}

#[tokio::test]
async fn carries_a_providers_observed_field_ref_through_the_gateway_and_manager_to_native_return() {
    let f = native(37);
    f.set_elements(vec![json!({
        "role": "AXTextField",
        "label": "Address",
        "frame": { "x": -290, "y": 30, "width": 120, "height": 20 },
        "element_token": "address-token",
    })]);
    let manager = gateway_manager(&f);
    let listing = observe(&f).await;
    let index = listing
        .ref_index
        .iter()
        .position(|entry| entry.label == "Address")
        .unwrap();
    assert_eq!(
        computer_element_ref_identity(&listing.ref_index[index]),
        Some("address-token")
    );
    let field = bind(&listing, index);
    drive(&manager, || {
        manager.press_key(Some(THREAD), "enter", Some(WINDOW), Some(field.clone()))
    })
    .await
    .unwrap();
    assert_subset(
        &f.last_args("press_key"),
        json!({
            "key": "enter",
            "element_token": "address-token",
            "pid": 10,
            "window_id": 20,
        }),
    );
    manager.dispose().await;
}

async fn keeps_the_original_native_identity_after_identical_label_controls_reorder(action: &str) {
    let f = native(37);
    let first = json!({
        "role": if action == "click" { "AXButton" } else { "AXTextField" },
        "label": "Duplicate",
        "value": "first",
        "frame": { "x": -290, "y": 30, "width": 70, "height": 20 },
        "element_token": "original-first-token",
        "actions": ["AXPress"],
        "in_web_content": true,
    });
    let mut second = first.clone();
    second["value"] = json!("second");
    second["frame"]["x"] = json!(-200);
    second["element_token"] = json!("original-second-token");
    f.set_elements(vec![first.clone(), second.clone()]);
    f.on_tool("set_value", |_| {
        json!({
            "structuredContent": { "effect": "confirmed", "evidence": [{ "kind": "value_readback" }] },
        })
    });
    let manager = gateway_manager(&f);

    let original = observe(&f).await;
    assert_eq!(
        identities(&original),
        [Some("original-first-token"), Some("original-second-token")]
    );
    assert!(!format!("{:?}", original.items).contains("original-first-token"));
    let first_ref = bind(&original, 0);
    f.set_elements(vec![second, first]);
    let reordered = observe(&f).await;
    assert_eq!(
        identities(&reordered),
        [Some("original-second-token"), Some("original-first-token")]
    );

    let before_action = f.call_count();
    let result = drive(&manager, || async {
        match action {
            "press_key" => {
                manager
                    .press_key(Some(THREAD), "enter", Some(WINDOW), Some(first_ref.clone()))
                    .await
            }
            "click" => {
                manager
                    .click(
                        Some(THREAD),
                        first_ref.clone(),
                        &[],
                        ComputerClickGesture::default(),
                    )
                    .await
            }
            "set_value" => {
                manager
                    .set_value(Some(THREAD), first_ref.clone(), "updated")
                    .await
            }
            _ => {
                manager
                    .type_text_at(Some(THREAD), " appended", first_ref.clone())
                    .await
            }
        }
    })
    .await;
    assert!(result.is_ok(), "{action}: {result:?}");
    let native_action = if action == "type_text" {
        "set_value"
    } else {
        action
    };
    assert_eq!(
        f.last_args(native_action)["element_token"],
        json!("original-first-token"),
        "{action}"
    );
    let action_calls = names_since(&f, before_action);
    let write_index = action_calls
        .iter()
        .position(|name| name == native_action)
        .unwrap();
    assert!(
        !action_calls[..write_index]
            .iter()
            .any(|name| name == "get_window_state"),
        "{action}: {action_calls:?}"
    );
    if action == "type_text" {
        let write = since(&f, before_action)
            .into_iter()
            .find(|call| call.name.as_deref() == Some(native_action))
            .unwrap();
        assert_subset(&write.args, json!({ "append": true, "value": " appended" }));
        assert_eq!(
            element_value(&f, "original-first-token"),
            json!("first appended")
        );
        assert_eq!(element_value(&f, "original-second-token"), json!("second"));
    }
    manager.dispose().await;
}

#[tokio::test]
async fn keeps_the_original_native_identity_after_identical_label_controls_reorder_for_press_key() {
    keeps_the_original_native_identity_after_identical_label_controls_reorder("press_key").await;
}

#[tokio::test]
async fn keeps_the_original_native_identity_after_identical_label_controls_reorder_for_click() {
    keeps_the_original_native_identity_after_identical_label_controls_reorder("click").await;
}

#[tokio::test]
async fn keeps_the_original_native_identity_after_identical_label_controls_reorder_for_set_value() {
    keeps_the_original_native_identity_after_identical_label_controls_reorder("set_value").await;
}

#[tokio::test]
async fn keeps_the_original_native_identity_after_identical_label_controls_reorder_for_type_text() {
    keeps_the_original_native_identity_after_identical_label_controls_reorder("type_text").await;
}

#[tokio::test]
async fn refuses_a_retained_web_append_when_an_identical_replacement_occupies_the_same_geometry() {
    let f = native(37);
    let field = json!({
        "role": "AXTextField",
        "label": "Search",
        "value": "old",
        "frame": { "x": -290, "y": 30, "width": 120, "height": 20 },
        "element_token": "original-token",
        "in_web_content": true,
    });
    f.set_elements(vec![field.clone()]);
    let manager = gateway_manager(&f);
    let original = bind(&observe(&f).await, 0);
    let mut replacement = field;
    replacement["value"] = json!("replacement");
    replacement["element_token"] = json!("replacement-token");
    f.set_elements(vec![replacement]);
    observe(&f).await;
    f.on_tool("set_value", |_| {
        json!({
            "isError": true,
            "structuredContent": {
                "effect": "not-dispatched",
                "code": "stale_target",
                "message": "The original token expired.",
            },
        })
    });
    let before_action = f.call_count();
    let result = drive(&manager, || {
        manager.type_text_at(Some(THREAD), " appended", original.clone())
    })
    .await;
    assert_eq!(code(&result.unwrap_err()), Some("stale_target"));
    let writes: Vec<ComputerHostCall> = since(&f, before_action)
        .into_iter()
        .filter(|call| matches!(call.name.as_deref(), Some("set_value" | "type_text")))
        .collect();
    assert_eq!(writes.len(), 1);
    assert_subset(
        &writes[0].args,
        json!({ "element_token": "original-token", "append": true }),
    );
    assert!(
        !names_since(&f, before_action)
            .iter()
            .any(|name| name == "get_window_state")
    );
    manager.dispose().await;
}

#[tokio::test]
async fn refuses_retained_web_append_on_older_native_revisions_without_a_snapshot_or_write() {
    let f = native(36);
    f.set_elements(vec![json!({
        "role": "AXTextField",
        "label": "Search",
        "value": "old",
        "frame": { "x": -290, "y": 30, "width": 120, "height": 20 },
        "element_token": "original-token",
        "in_web_content": true,
    })]);
    let manager = gateway_manager(&f);
    let original = bind(&observe(&f).await, 0);
    let before_action = f.call_count();
    let result = drive(&manager, || {
        manager.type_text_at(Some(THREAD), " appended", original.clone())
    })
    .await;
    assert_eq!(code(&result.unwrap_err()), Some("unsupported_operation"));
    assert!(
        names_since(&f, before_action).iter().all(|name| ![
            "get_window_state",
            "set_value",
            "type_text"
        ]
        .contains(&name.as_str()))
    );
    manager.dispose().await;
}

#[tokio::test]
async fn refuses_a_native_retained_ref_without_a_semantic_click_instead_of_using_its_stale_coordinates()
 {
    let f = native(37);
    f.set_elements(vec![json!({
        "role": "AXTextField",
        "label": "Search",
        "frame": { "x": -290, "y": 30, "width": 120, "height": 20 },
        "element_token": "original-token",
    })]);
    let manager = gateway_manager(&f);
    let original = bind(&observe(&f).await, 0);
    let result = drive(&manager, || {
        manager.click(
            Some(THREAD),
            original.clone(),
            &[],
            ComputerClickGesture::default(),
        )
    })
    .await;
    assert_eq!(code(&result.unwrap_err()), Some("computer_target_refused"));
    assert!(f.named("click").is_empty());
    manager.dispose().await;
}

#[tokio::test]
async fn presses_a_web_button_that_advertises_no_actions_instead_of_refusing_its_ref() {
    let f = native(37);
    f.set_elements(vec![json!({
        "role": "AXButton",
        "label": "Chọn CPU",
        "frame": { "x": -290, "y": 30, "width": 120, "height": 20 },
        "element_token": "web-button",
        "in_web_content": true,
    })]);
    let manager = gateway_manager(&f);
    let button = bind(&observe(&f).await, 0);
    drive(&manager, || {
        manager.click(
            Some(THREAD),
            button.clone(),
            &[],
            ComputerClickGesture::default(),
        )
    })
    .await
    .unwrap();
    let clicks = f.named("click");
    assert_eq!(clicks.len(), 1);
    assert_eq!(f.last_args("click")["element_token"], json!("web-button"));
    manager.dispose().await;
}

#[tokio::test]
async fn types_into_the_focused_web_field_when_the_window_also_has_an_address_bar() {
    let f = native(37);
    f.set_elements(vec![
        json!({
            "element_index": 0,
            "role": "AXTextField",
            "label": "Address and search bar",
            "frame": { "x": -290, "y": 10, "width": 120, "height": 20 },
            "element_token": "omnibox",
        }),
        json!({
            "element_index": 1,
            "role": "AXTextField",
            "frame": { "x": -290, "y": 40, "width": 120, "height": 20 },
            "element_token": "web-field",
            "in_web_content": true,
        }),
    ]);
    f.knobs().window_state_extra = json!({
        "tree_nodes": [
            { "depth": 0, "role": "AXTextField", "element_index": 0, "title": "Address and search bar" },
            { "depth": 0, "role": "AXTextField", "element_index": 1, "settable": true },
        ],
        "focused_element": { "depth": 0, "role": "AXTextField", "element_index": 1, "settable": true },
    })
    .as_object()
    .unwrap()
    .clone();
    let manager = gateway_manager(&f);
    let result = drive(&manager, || {
        manager.type_text(Some(THREAD), "5600X3D", Some(WINDOW))
    })
    .await
    .unwrap();
    assert_eq!(
        f.last_args("set_value")["element_token"],
        json!("web-field")
    );
    assert_eq!(f.last_args("set_value")["append"], json!(true));
    assert_eq!(
        result.delivery.map(|delivery| delivery.verified),
        Some(ComputerDeliveryVerification::Confirmed)
    );
    assert!(f.named("type_text").is_empty());
    manager.dispose().await;
}
