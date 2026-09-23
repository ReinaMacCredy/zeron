use serde_json::json;
use zeron_proto::computer::{ComputerDeliveryVerification, ComputerScrollDelta, CuaEffect};

use super::*;
use crate::computer::backend::ComputerLaunchOptions;

#[tokio::test]
async fn requests_ax_keyboard_focus_only_for_explicit_observations_not_input_revalidation() {
    let f = fixture_with(FixtureOptions {
        native_revision: Some(Some(37)),
        ..Default::default()
    });
    observing(f.backend.get_state(window_state("cua:10:20")))
        .await
        .unwrap();
    assert_eq!(
        f.first_args("list_windows"),
        json!({ "include_keyboard_focus": true })
    );
    f.clear_calls();
    observing(f.backend.press_key("enter", Some("cua:10:20"), None))
        .await
        .unwrap();
    assert_eq!(f.first_args("list_windows"), json!({}));
}

#[tokio::test]
async fn keeps_advertised_retained_ax_actions_available_off_space_without_pointer_input() {
    let f = fixture_with(FixtureOptions {
        native_revision: Some(Some(37)),
        ..Default::default()
    });
    f.set_elements(vec![json!({
        "role": "AXButton",
        "label": "Import",
        "frame": { "x": -290, "y": 30, "width": 20, "height": 20 },
        "element_token": "import-token",
        "actions": ["AXPress"],
    })]);
    let state = f
        .backend
        .get_state(window_state("cua:10:20"))
        .await
        .unwrap();
    let node = state.root.unwrap().children[0].clone();
    f.set_visible(false);
    f.backend
        .perform_action(&resolved("Import", &node), "AXPress")
        .await
        .unwrap();
    let args = f.last_args("click");
    assert_subset(
        &args,
        json!({ "element_token": "import-token", "action": "press", "delivery_mode": "background" }),
    );
    assert!(args.get("x").is_none());
    let error = f
        .backend
        .press_key("enter", Some("cua:10:20"), None)
        .await
        .unwrap_err();
    assert_eq!(code(&error), Some("target_not_on_active_space"));
    assert_eq!(effect(&error), Some(CuaEffect::NotDispatched));
}

#[tokio::test]
async fn preserves_actual_native_keyboard_focus_independently_of_the_selected_target() {
    let f = fixture();
    f.set_windows(vec![json!({
        "pid": 10,
        "window_id": 21,
        "title": "Sheet",
        "bounds": { "x": 0, "y": 0, "width": 100, "height": 100 },
        "is_on_screen": true,
        "keyboard_focused": true,
    })]);
    f.backend.focus_window("cua:10:20").await.unwrap();
    let windows = f.backend.list_windows().await.unwrap();
    let selected = windows.iter().find(|w| w.id == "cua:10:20").unwrap();
    assert!(selected.focused);
    assert_eq!(selected.keyboard_focused, None);
    let sheet = windows.iter().find(|w| w.id == "cua:10:21").unwrap();
    assert!(!sheet.focused);
    assert_eq!(sheet.keyboard_focused, Some(true));
}

#[tokio::test]
async fn preserves_app_initiated_focus_changes_during_a_requested_background_launch() {
    let f = fixture();
    f.on_tool(
        "launch_app",
        |_| json!({ "structuredContent": { "pid": 10, "focus_changed_during_launch": true } }),
    );
    let launched = f
        .backend
        .launch_app("Resolve", &[], ComputerLaunchOptions::default())
        .await
        .unwrap();
    assert_eq!(launched.pid, Some(10));
    assert_eq!(launched.focus_changed_during_launch, Some(true));
    assert_eq!(f.named("launch_app").len(), 1);
}

#[tokio::test]
async fn maps_public_scrolling_to_core_graphics_wheel_signs() {
    for (direction, dx, dy, native_x, native_y) in [
        ("down", 0.0, 240.0, 0, -2),
        ("up", 0.0, -240.0, 0, 2),
        ("right", 240.0, 0.0, -2, 0),
        ("left", -240.0, 0.0, 2, 0),
    ] {
        let f = fixture();
        f.backend
            .capture_screenshot(window_capture("cua:10:20"))
            .await
            .unwrap();
        let result = f
            .backend
            .scroll(
                Some(point(-275.0, 30.0)),
                dx,
                dy,
                Some("cua:10:20"),
                &[],
                None,
            )
            .await
            .unwrap();
        assert_eq!(
            result.scroll_delta,
            Some(ComputerScrollDelta {
                delta_x: dx,
                delta_y: dy
            }),
            "{direction}"
        );
        assert_subset(
            &f.first_args("scroll"),
            json!({ "direction": direction, "delta_x": native_x, "delta_y": native_y }),
        );
    }
}

#[tokio::test]
async fn uses_advertised_ax_actions_and_retains_a_fresh_check_at_input_dispatch() {
    let f = fixture();
    f.set_elements(vec![
        json!({
            "role": "AXButton",
            "label": "Equals",
            "frame": { "x": -290, "y": 30, "width": 20, "height": 20 },
            "element_token": "fresh-token",
            "actions": ["AXPress"],
        }),
        json!({
            "role": "AXButton",
            "label": "Canvas",
            "frame": { "x": -270, "y": 30, "width": 20, "height": 20 },
            "element_token": "canvas-token",
        }),
    ]);
    let state = f
        .backend
        .get_state(window_state("cua:10:20"))
        .await
        .unwrap();
    let root = state.root.clone().unwrap();
    let target = resolved("Equals", &root.children[0]);
    assert_eq!(f.named("list_windows").len(), 1);
    assert_eq!(state.windows.len(), 1);
    assert!(f.backend.supports_action(&target, "AXPress"));
    assert!(
        !f.backend
            .supports_action(&resolved("Equals", &root.children[1]), "AXPress")
    );
    f.backend.focus_window("cua:10:20").await.unwrap();
    assert_eq!(f.named("list_windows").len(), 1);
    f.backend.perform_action(&target, "AXPress").await.unwrap();
    assert_eq!(f.named("list_windows").len(), 2);
    let args = f.first_args("click");
    assert_subset(
        &args,
        json!({ "element_token": "fresh-token", "pid": 10, "window_id": 20 }),
    );
    assert!(args.get("force_synthetic").is_none());
    f.close();
    let error = f
        .backend
        .perform_action(&target, "AXPress")
        .await
        .unwrap_err();
    assert_eq!(effect(&error), Some(CuaEffect::NotDispatched));
    assert_eq!(f.named("click").len(), 1);
}

#[tokio::test]
async fn dispatches_named_secondary_actions_through_the_click_element_recipe() {
    let f = fixture();
    f.set_elements(vec![
        json!({
            "role": "AXRow",
            "label": "Document.txt",
            "frame": { "x": -290, "y": 30, "width": 20, "height": 20 },
            "element_token": "open-token",
            "actions": ["AXPress", "AXOpen"],
        }),
        json!({
            "role": "AXButton",
            "label": "Menu",
            "frame": { "x": -270, "y": 30, "width": 20, "height": 20 },
            "element_token": "menu-token",
            "actions": ["AXPress", "AXShowMenu", "AXPick", "AXConfirm", "AXCancel"],
        }),
    ]);
    let root = f
        .backend
        .get_state(window_state("cua:10:20"))
        .await
        .unwrap()
        .root
        .unwrap();
    let open = resolved("Document.txt", &root.children[0]);
    let menu = resolved("Menu", &root.children[1]);
    assert!(f.backend.supports_action(&open, "open"));
    assert!(!f.backend.supports_action(&open, "show_menu"));
    assert!(f.backend.supports_action(&menu, "menu"));
    assert!(!f.backend.supports_action(&menu, "activate"));
    f.backend.perform_action(&open, "open").await.unwrap();
    for action in ["press", "show_menu", "menu", "pick", "confirm", "cancel"] {
        f.backend.perform_action(&menu, action).await.unwrap();
    }
    let dispatched: Vec<(Value, Value)> = f
        .named("click")
        .iter()
        .map(|call| {
            (
                call.args["element_token"].clone(),
                call.args["action"].clone(),
            )
        })
        .collect();
    assert_eq!(
        dispatched,
        vec![
            (json!("open-token"), json!("open")),
            (json!("menu-token"), json!("press")),
            (json!("menu-token"), json!("show_menu")),
            (json!("menu-token"), json!("show_menu")),
            (json!("menu-token"), json!("pick")),
            (json!("menu-token"), json!("confirm")),
            (json!("menu-token"), json!("cancel")),
        ]
    );
    f.backend.perform_action(&open, "AXPress").await.unwrap();
    assert_subset(
        &f.last_args("click"),
        json!({ "element_token": "open-token", "action": "press" }),
    );
}

#[tokio::test]
async fn refuses_a_secondary_action_the_element_does_not_advertise_without_dispatching() {
    let f = fixture();
    f.set_elements(vec![
        json!({
            "role": "AXButton",
            "label": "Plain",
            "frame": { "x": -290, "y": 30, "width": 20, "height": 20 },
            "element_token": "plain-token",
            "actions": ["AXPress"],
        }),
        json!({
            "role": "AXStaticText",
            "label": "Passive",
            "frame": { "x": -270, "y": 30, "width": 20, "height": 20 },
            "element_token": "passive-token",
        }),
    ]);
    let root = f
        .backend
        .get_state(window_state("cua:10:20"))
        .await
        .unwrap()
        .root
        .unwrap();
    let target = resolved("Plain", &root.children[0]);
    let passive = resolved("Passive", &root.children[1]);
    for action in ["open", "show_menu", "menu", "pick", "confirm", "cancel"] {
        let error = f.backend.perform_action(&target, action).await.unwrap_err();
        assert_eq!(effect(&error), Some(CuaEffect::NotDispatched));
        assert_eq!(code(&error), Some("unsupported_operation"));
    }
    let error = f
        .backend
        .perform_action(&passive, "open")
        .await
        .unwrap_err();
    assert_eq!(code(&error), Some("unsupported_operation"));
    let error = f
        .backend
        .perform_action(&target, "toggle")
        .await
        .unwrap_err();
    assert_eq!(effect(&error), Some(CuaEffect::NotDispatched));
    assert_eq!(code(&error), Some("unsupported_operation"));
    assert!(f.named("click").is_empty());
    f.backend.perform_action(&target, "press").await.unwrap();
    assert_eq!(f.named("click").len(), 1);
}

#[tokio::test]
async fn writes_through_a_live_token_and_refuses_a_stale_one_without_a_second_dispatch() {
    let f = fixture();
    f.set_elements(vec![json!({
        "role": "AXTextField",
        "label": "Display",
        "frame": { "x": -290, "y": 30, "width": 20, "height": 20 },
        "element_token": "fresh-token",
    })]);
    let root = f
        .backend
        .get_state(window_state("cua:10:20"))
        .await
        .unwrap()
        .root
        .unwrap();
    let target = resolved("Display", &root.children[0]);
    f.backend.set_value(&target, "1").await.unwrap();
    assert_subset(
        &f.first_args("set_value"),
        json!({ "element_token": "fresh-token", "value": "1" }),
    );
    f.close();
    let error = f.backend.set_value(&target, "2").await.unwrap_err();
    assert_eq!(effect(&error), Some(CuaEffect::NotDispatched));
    assert_eq!(f.named("set_value").len(), 1);
}

#[tokio::test]
async fn selects_an_exact_text_range_on_a_live_element_and_trusts_only_native_read_back() {
    let f = fixture();
    f.set_elements(vec![json!({
        "role": "AXTextField",
        "label": "Display",
        "frame": { "x": -290, "y": 30, "width": 20, "height": 20 },
        "element_token": "fresh-token",
    })]);
    let root = f
        .backend
        .get_state(window_state("cua:10:20"))
        .await
        .unwrap()
        .root
        .unwrap();
    let node = root.children[0].clone();
    let target = resolved("Display", &node);
    f.on_tool("select_text", |_| {
        json!({ "structuredContent": {
            "route": "ax_semantic",
            "delivery": { "mode": "background" },
            "effect": "confirmed",
            "evidence": [{ "kind": "value_readback" }],
        } })
    });
    let result = f.backend.select_text(&target, range(2, 4)).await.unwrap();
    assert_eq!(
        result.verified,
        Some(ComputerDeliveryVerification::Confirmed)
    );
    assert_eq!(result.effect, Some(CuaEffect::Verified));
    let args = f.first_args("select_text");
    assert_subset(
        &args,
        json!({ "pid": 10, "window_id": 20, "element_token": "fresh-token", "start": 2, "length": 4 }),
    );
    assert!(args.get("delivery_mode").is_none());
    f.on_tool("select_text", |_| {
        json!({
            "isError": true,
            "structuredContent": { "effect": "refused", "code": "attribute_not_settable" },
            "content": [{ "type": "text", "text": "AXSelectedTextRange is not settable." }],
        })
    });
    let error = f
        .backend
        .select_text(&target, range(0, 1))
        .await
        .unwrap_err();
    assert_eq!(effect(&error), Some(CuaEffect::NotDispatched));
    assert_eq!(code(&error), Some("attribute_not_settable"));
    f.on_tool("select_text", |_| {
        json!({ "structuredContent": {
            "route": "ax_semantic",
            "delivery": { "mode": "background" },
            "effect": "unconfirmed",
        } })
    });
    let result = f.backend.select_text(&target, range(0, 1)).await.unwrap();
    assert_eq!(
        result.verified,
        Some(ComputerDeliveryVerification::Unconfirmed)
    );
    assert_eq!(result.effect, Some(CuaEffect::DispatchedUnknown));
    assert_eq!(f.named("select_text").len(), 3);
    let unobserved = resolved(
        "Display",
        &ComputerUiNode {
            node_path: None,
            ..node.clone()
        },
    );
    let error = f
        .backend
        .select_text(&unobserved, range(0, 1))
        .await
        .unwrap_err();
    assert_eq!(effect(&error), Some(CuaEffect::NotDispatched));
    assert_eq!(code(&error), Some("stale_target"));
    f.close();
    let error = f
        .backend
        .select_text(&target, range(0, 1))
        .await
        .unwrap_err();
    assert_eq!(effect(&error), Some(CuaEffect::NotDispatched));
    assert_eq!(f.named("select_text").len(), 3);
}

#[tokio::test]
async fn uses_semantic_only_text_delivery_for_an_exact_live_control() {
    let f = fixture();
    f.set_elements(vec![json!({
        "role": "AXTextField",
        "label": "Message",
        "frame": { "x": -290, "y": 30, "width": 120, "height": 20 },
        "element_token": "message-token",
    })]);
    let root = f
        .backend
        .get_state(window_state("cua:10:20"))
        .await
        .unwrap()
        .root
        .unwrap();
    let target = resolved_in("Message", Some("cua:10:20"), &root.children[0]);
    f.backend
        .type_text("hello", Some("cua:10:20"), Some(&target))
        .await
        .unwrap();
    let args = f.first_args("type_text");
    assert_subset(
        &args,
        json!({
            "text": "hello",
            "pid": 10,
            "window_id": 20,
            "element_token": "message-token",
            "semantic_only": true,
            "delay_ms": 0,
        }),
    );
    assert!(args.get("force_synthetic").is_none());
    f.backend
        .type_text("keyboard", Some("cua:10:20"), None)
        .await
        .unwrap();
    let focused = f.named("type_text")[1].args.clone();
    assert_subset(&focused, json!({ "text": "keyboard", "delay_ms": 10 }));
    assert!(focused.get("force_synthetic").is_none());
    assert!(focused.get("semantic_only").is_none());
}

fn web_field(token: &str, index: u64) -> Value {
    json!({
        "role": "AXTextField",
        "label": "Message",
        "frame": { "x": -290, "y": 30, "width": 120, "height": 20 },
        "element_token": token,
        "element_index": index,
        "in_web_content": true,
        "value": "seed",
    })
}

#[tokio::test]
async fn types_into_web_content_through_a_composed_set_value_and_verifies_on_re_read() {
    let f = fixture();
    f.set_elements(vec![web_field("web-token", 2)]);
    let root = f
        .backend
        .get_state(window_state("cua:10:20"))
        .await
        .unwrap()
        .root
        .unwrap();
    let target = resolved_in("Message", Some("cua:10:20"), &root.children[0]);
    let result = f
        .backend
        .type_text("-typed", Some("cua:10:20"), Some(&target))
        .await
        .unwrap();
    assert!(f.named("type_text").is_empty());
    assert_subset(
        &f.first_args("set_value"),
        json!({
            "element_token": "web-token",
            "element_index": 2,
            "value": "seed-typed",
            "pid": 10,
            "window_id": 20,
        }),
    );
    assert_eq!(
        result.verified,
        Some(ComputerDeliveryVerification::Confirmed)
    );
    assert_eq!(result.effect, Some(CuaEffect::Verified));
    let reads = f.named("get_window_state");
    assert_eq!(reads.len(), 3);
    for read in reads {
        assert_eq!(read.args["include_screenshot"], json!(false));
    }
}

#[tokio::test]
async fn refuses_a_stale_semantic_field_when_fresh_state_has_two_matching_controls() {
    let f = fixture();
    f.set_elements(vec![web_field("one", 2)]);
    let root = f
        .backend
        .get_state(window_state("cua:10:20"))
        .await
        .unwrap()
        .root
        .unwrap();
    let node = root.children[0].clone();
    f.set_elements(vec![web_field("one", 2), web_field("two", 3)]);
    let error = f
        .backend
        .set_value(
            &resolved_in("Message", Some("cua:10:20"), &node),
            "replacement",
        )
        .await
        .unwrap_err();
    assert_eq!(effect(&error), Some(CuaEffect::NotDispatched));
    assert_eq!(code(&error), Some("stale_target"));
    assert!(f.named("set_value").is_empty());
}
