use serde_json::json;
use zeron_proto::computer::{
    ComputerAvailability, ComputerDeliveryVerification, ComputerInputModifier, ComputerPermission,
    ComputerRect, ComputerScrollDelta, CuaEffect,
};

use super::*;

#[tokio::test]
async fn reads_the_published_action_route_and_requires_public_verification_evidence() {
    let f = fixture();
    f.action_result(json!({
        "route": "accessibility",
        "delivery": { "mode": "background" },
        "effect": "confirmed",
    }));
    let result = f
        .backend
        .type_text("abc", Some("cua:10:20"), None)
        .await
        .unwrap();
    assert_eq!(result.effect, Some(CuaEffect::DispatchedUnknown));
    assert_eq!(
        result.delivery_path.as_deref(),
        Some("cua-accessibility-background")
    );
    f.action_result(json!({
        "route": "accessibility",
        "delivery": { "mode": "background" },
        "effect": "confirmed",
        "evidence": [{ "kind": "value_readback" }],
    }));
    let result = f
        .backend
        .type_text("def", Some("cua:10:20"), None)
        .await
        .unwrap();
    assert_eq!(result.effect, Some(CuaEffect::Verified));
    assert_eq!(
        result.verified,
        Some(ComputerDeliveryVerification::Confirmed)
    );
}

#[tokio::test]
async fn preserves_public_action_refusals_even_without_the_outer_error_flag() {
    let f = fixture();
    f.action_result(json!({ "route": "accessibility", "effect": "refused" }));
    let error = f
        .backend
        .type_text("abc", Some("cua:10:20"), None)
        .await
        .unwrap_err();
    assert_eq!(effect(&error), Some(CuaEffect::NotDispatched));
    assert_eq!(f.named("type_text").len(), 1);
}

#[tokio::test]
async fn preserves_status_refusals_and_gives_a_semantic_recovery_route_for_keyboard_ambiguity() {
    let f = fixture();
    f.on_tool("press_key", |_| {
        json!({ "structuredContent": {
            "status": "refused",
            "refusal": { "code": "same_pid_keyboard_ambiguity" },
        } })
    });
    let failure = f
        .backend
        .press_key("enter", Some("cua:10:20"), None)
        .await
        .unwrap_err();
    assert_eq!(effect(&failure), Some(CuaEffect::NotDispatched));
    assert_eq!(code(&failure), Some("same_pid_keyboard_ambiguity"));
    let message = failure.to_string();
    assert!(message.contains("close it by pressing its button by element"));
    assert!(message.contains("do not send keydown/keyup"));
    assert!(!message.contains("delivery_mode"), "{message}");
    assert_eq!(f.named("press_key").len(), 1);
}

#[tokio::test(start_paused = true)]
async fn encodes_missing_grants_with_the_public_permission_schema() {
    let f = fixture();
    f.deny_permissions();
    let availability = f.backend.availability(false).await.unwrap();
    let decoded: ComputerAvailability =
        serde_json::from_value(serde_json::to_value(&availability).unwrap()).unwrap();
    let ComputerAvailability::PermissionRequired { missing, .. } = decoded else {
        panic!("expected permission-required, got {decoded:?}");
    };
    assert_eq!(
        missing,
        vec![
            ComputerPermission::Accessibility,
            ComputerPermission::ScreenRecording
        ]
    );
}

#[tokio::test]
async fn ignores_non_actionable_zero_area_windows() {
    let f = fixture();
    assert_eq!(f.backend.list_windows().await.unwrap().len(), 1);
}

#[tokio::test]
async fn reports_minimized_and_hidden_workspace_windows_honestly() {
    let f = fixture();
    let rect = json!({ "x": 10, "y": 10, "width": 200, "height": 100 });
    f.set_windows(vec![
        json!({
            "pid": 10,
            "window_id": 21,
            "title": "Minimized",
            "bounds": rect,
            "is_on_screen": false,
            "on_current_space": true,
            "space_ids": [3],
            "z_index": 2,
        }),
        json!({
            "pid": 11,
            "window_id": 22,
            "title": "Hidden app window",
            "bounds": rect,
            "is_on_screen": false,
            "on_current_space": null,
            "space_ids": null,
            "z_index": 3,
        }),
    ]);
    let windows = f.backend.list_windows().await.unwrap();
    let minimized = windows.iter().find(|w| w.id == "cua:10:21").unwrap();
    assert!(minimized.minimized);
    assert!(!minimized.visible);
    let hidden = windows.iter().find(|w| w.id == "cua:11:22").unwrap();
    assert!(!hidden.minimized);
    assert!(!hidden.visible);
}

#[tokio::test]
async fn preserves_observed_space_membership_without_inventing_missing_or_unsafe_identifiers() {
    let f = fixture();
    let base = |window_id: u64, extra: Value| {
        let mut window = json!({
            "pid": 10,
            "window_id": window_id,
            "bounds": { "x": 10, "y": 10, "width": 200, "height": 100 },
            "is_on_screen": false,
        });
        for (key, value) in extra.as_object().unwrap() {
            window[key] = value.clone();
        }
        window
    };
    f.set_windows(vec![
        base(
            21,
            json!({ "space_ids": [3, 8], "current_space_id": 8, "on_current_space": true }),
        ),
        base(
            22,
            json!({ "space_ids": [3], "current_space_id": 8, "on_current_space": false }),
        ),
        base(
            23,
            json!({ "space_ids": null, "current_space_id": null, "on_current_space": null }),
        ),
        base(
            24,
            json!({ "space_ids": [3, 9_007_199_254_740_992_u64], "current_space_id": 0 }),
        ),
        base(
            25,
            json!({ "space_ids": [], "current_space_id": 8, "on_current_space": false }),
        ),
    ]);
    let windows = f.backend.list_windows().await.unwrap();
    let by_id = |id: &str| windows.iter().find(|w| w.id == id).unwrap().clone();
    let first = by_id("cua:10:21");
    assert_eq!(first.space_ids, Some(vec![3, 8]));
    assert_eq!(first.current_space_id, Some(8));
    assert_eq!(first.on_current_space, Some(true));
    let second = by_id("cua:10:22");
    assert_eq!(second.space_ids, Some(vec![3]));
    assert_eq!(second.current_space_id, Some(8));
    assert_eq!(second.on_current_space, Some(false));
    for id in ["cua:10:23", "cua:10:24"] {
        let window = by_id(id);
        assert_eq!(window.space_ids, None, "{id}");
        assert_eq!(window.current_space_id, None, "{id}");
        assert_eq!(window.on_current_space, None, "{id}");
    }
    let empty = by_id("cua:10:25");
    assert_eq!(empty.space_ids, Some(vec![]));
    assert_eq!(empty.current_space_id, Some(8));
    assert_eq!(empty.on_current_space, Some(false));
}

#[tokio::test]
async fn distinguishes_a_native_admission_refusal_from_an_uncertain_delivery() {
    let f = fixture();
    f.refuse();
    let error = f
        .backend
        .type_text("abc", Some("cua:10:20"), None)
        .await
        .unwrap_err();
    assert_eq!(effect(&error), Some(CuaEffect::NotDispatched));
    assert_eq!(code(&error), Some("same_pid_keyboard_ambiguity"));
    assert_eq!(f.named("type_text").len(), 1);
}

fn keys(names: &[&str]) -> Vec<String> {
    names.iter().map(|name| (*name).to_owned()).collect()
}

#[tokio::test]
async fn translates_dom_key_names_without_turning_delete_into_backspace() {
    let f = fixture();
    f.backend
        .press_key("Delete", Some("cua:10:20"), None)
        .await
        .unwrap();
    f.backend
        .hotkey(&keys(&["meta", "arrowleft"]), Some("cua:10:20"), None)
        .await
        .unwrap();
    assert_subset(
        &f.first_args("press_key"),
        json!({ "key": "forward_delete" }),
    );
    assert_subset(
        &f.first_args("hotkey"),
        json!({ "keys": ["command", "left"] }),
    );
    let error = f
        .backend
        .hotkey(&keys(&["meta", "a", "s"]), Some("cua:10:20"), None)
        .await
        .unwrap_err();
    assert!(error.to_string().contains("exactly one"));
}

#[tokio::test]
async fn maps_xdotool_style_key_spellings_onto_driver_keynames() {
    let f = fixture();
    for (name, driver) in [
        ("Page_Up", "pageup"),
        ("Prior", "pageup"),
        ("pgup", "pageup"),
        ("Page_Down", "pagedown"),
        ("Next", "pagedown"),
        ("pgdn", "pagedown"),
        ("Caps_Lock", "capslock"),
        ("Super_L", "command"),
        ("Win", "command"),
        ("Shift_L", "shift"),
        ("Control_L", "ctrl"),
        ("Alt_L", "alt"),
        ("Option_L", "alt"),
    ] {
        f.clear_calls();
        f.backend
            .press_key(name, Some("cua:10:20"), None)
            .await
            .unwrap();
        assert_subset(&f.first_args("press_key"), json!({ "key": driver }));
    }
    for name in ["kp_5", "KP_Enter", "F13", "Menu", "Help", "Shift_R"] {
        f.clear_calls();
        f.backend
            .press_key(name, Some("cua:10:20"), None)
            .await
            .unwrap();
        assert_subset(
            &f.first_args("press_key"),
            json!({ "key": name.to_lowercase() }),
        );
    }
    f.clear_calls();
    f.backend
        .hotkey(&keys(&["meta", "Page_Up"]), Some("cua:10:20"), None)
        .await
        .unwrap();
    assert_subset(
        &f.first_args("hotkey"),
        json!({ "keys": ["command", "pageup"] }),
    );
    f.clear_calls();
    f.backend
        .hotkey(&keys(&["Control_L", "c"]), Some("cua:10:20"), None)
        .await
        .unwrap();
    assert_subset(&f.first_args("hotkey"), json!({ "keys": ["ctrl", "c"] }));
}

#[tokio::test]
async fn refuses_insert_spellings_and_still_refuses_modifier_only_chords() {
    let f = fixture();
    for name in ["insert", "Insert", "ins"] {
        let error = f
            .backend
            .press_key(name, Some("cua:10:20"), None)
            .await
            .unwrap_err();
        assert!(
            error.to_string().contains("no Insert key mapping"),
            "{name}"
        );
    }
    let error = f
        .backend
        .hotkey(&keys(&["meta", "ins"]), Some("cua:10:20"), None)
        .await
        .unwrap_err();
    assert!(error.to_string().contains("no Insert key mapping"));
    for chord in [["meta", "shift"], ["meta", "shift_l"]] {
        let error = f
            .backend
            .hotkey(&keys(&chord), Some("cua:10:20"), None)
            .await
            .unwrap_err();
        assert!(error.to_string().contains("exactly one"));
    }
    assert!(
        f.calls()
            .iter()
            .all(|call| !matches!(call.name.as_deref(), Some("press_key" | "hotkey")))
    );
}

async fn observed(f: &Fixture) {
    f.backend
        .capture_screenshot(window_capture("cua:10:20"))
        .await
        .unwrap();
}

#[tokio::test]
async fn converts_pixel_deltas_to_one_bounded_wheel_operation() {
    let f = fixture();
    observed(&f).await;
    let result = f
        .backend
        .scroll(
            Some(point(-275.0, 30.0)),
            0.0,
            250.0,
            Some("cua:10:20"),
            &[],
            None,
            None,
        )
        .await
        .unwrap();
    assert_eq!(
        result.scroll_delta,
        Some(ComputerScrollDelta {
            delta_x: 0.0,
            delta_y: 240.0
        })
    );
    assert_eq!(f.named("scroll").len(), 1);
    assert_subset(
        &f.first_args("scroll"),
        json!({ "delta_x": 0, "delta_y": -2, "direction": "down" }),
    );
}

#[tokio::test]
async fn carries_both_axes_and_modifiers_in_one_wheel_gesture() {
    let f = fixture();
    observed(&f).await;
    let result = f
        .backend
        .scroll(
            Some(point(-275.0, 30.0)),
            -140.0,
            250.0,
            Some("cua:10:20"),
            &[ComputerInputModifier::Meta, ComputerInputModifier::Shift],
            None,
            None,
        )
        .await
        .unwrap();
    assert_eq!(
        result.scroll_delta,
        Some(ComputerScrollDelta {
            delta_x: -120.0,
            delta_y: 240.0
        })
    );
    assert_eq!(f.named("scroll").len(), 1);
    assert_subset(
        &f.first_args("scroll"),
        json!({
            "delta_x": 1,
            "delta_y": -2,
            "direction": "down",
            "modifiers": ["command", "shift"],
        }),
    );
}

#[tokio::test]
async fn refuses_a_single_axis_beyond_the_50_notch_bound_before_dispatch() {
    let f = fixture();
    observed(&f).await;
    for (dx, dy) in [(0.0, 61.0 * 120.0), (61.0 * 120.0, 0.0)] {
        let error = f
            .backend
            .scroll(
                Some(point(-275.0, 30.0)),
                dx,
                dy,
                Some("cua:10:20"),
                &[],
                None,
                None,
            )
            .await
            .unwrap_err();
        assert_eq!(effect(&error), Some(CuaEffect::NotDispatched));
        assert_eq!(code(&error), Some("unsupported_operation"));
    }
    assert!(f.named("scroll").is_empty());
}

#[tokio::test]
async fn prefers_the_ax_scroll_bar_route_for_an_unmodified_vertical_element_scroll() {
    let f = fixture();
    f.set_elements(vec![json!({
        "role": "AXScrollArea",
        "label": "Content",
        "frame": { "x": -290, "y": 30, "width": 20, "height": 20 },
        "element_token": "scroll-token",
    })]);
    let root = f
        .backend
        .get_state(window_state("cua:10:20"))
        .await
        .unwrap()
        .root
        .unwrap();
    let target = resolved_in("Content", Some("cua:10:20"), &root.children[0]);
    observed(&f).await;
    f.backend
        .scroll(
            Some(target.point),
            0.0,
            250.0,
            Some("cua:10:20"),
            &[],
            Some(&target),
            None,
        )
        .await
        .unwrap();
    assert_subset(
        &f.first_args("scroll"),
        json!({ "element_token": "scroll-token", "direction": "down", "amount": 2, "by": "line", "delta_x": 0, "delta_y": -2 }),
    );
    f.backend
        .scroll(
            Some(target.point),
            -140.0,
            250.0,
            Some("cua:10:20"),
            &[],
            Some(&target),
            None,
        )
        .await
        .unwrap();
    let second = f.named("scroll")[1].args.clone();
    assert_subset(&second, json!({ "delta_x": 1, "delta_y": -2 }));
    assert!(second.get("element_token").is_none());
}

#[tokio::test]
async fn scrolls_an_element_or_a_freshly_bounded_point_without_screenshot_geometry() {
    let f = fixture();
    f.set_elements(vec![json!({
        "role": "AXScrollArea",
        "label": "Content",
        "frame": { "x": -290, "y": 30, "width": 20, "height": 20 },
        "element_token": "scroll-token",
    })]);
    let root = f
        .backend
        .get_state(window_state("cua:10:20"))
        .await
        .unwrap()
        .root
        .unwrap();
    let target = resolved_in("Content", Some("cua:10:20"), &root.children[0]);
    f.backend
        .scroll(
            Some(target.point),
            0.0,
            250.0,
            Some("cua:10:20"),
            &[],
            Some(&target),
            None,
        )
        .await
        .unwrap();
    let element = f.first_args("scroll");
    assert_subset(&element, json!({ "element_token": "scroll-token" }));
    assert!(element.get("x").is_none(), "{element}");
    let window = ComputerRect {
        x: -300.0,
        y: 20.0,
        width: 200.0,
        height: 100.0,
    };
    f.backend
        .scroll(
            Some(point(-200.0, 70.0)),
            0.0,
            250.0,
            Some("cua:10:20"),
            &[],
            None,
            Some(window),
        )
        .await
        .unwrap();
    assert_subset(
        &f.named("scroll")[1].args,
        json!({ "x": 100, "y": 50, "expected_window_bounds": { "x": -300, "y": 20 } }),
    );
}

#[tokio::test]
async fn refuses_a_scroll_point_whose_frame_no_longer_matches_the_window() {
    let f = fixture();
    let moved = ComputerRect {
        x: -250.0,
        y: 20.0,
        width: 200.0,
        height: 100.0,
    };
    for prepared in [Some(moved), None] {
        let error = f
            .backend
            .scroll(
                Some(point(-275.0, 30.0)),
                0.0,
                250.0,
                Some("cua:10:20"),
                &[],
                None,
                prepared,
            )
            .await
            .unwrap_err();
        assert_eq!(effect(&error), Some(CuaEffect::NotDispatched));
        assert_eq!(code(&error), Some("stale_geometry"));
    }
    assert!(f.named("scroll").is_empty());
}

#[tokio::test]
async fn coalesces_physical_state_across_concurrent_thread_projections() {
    let f = fixture();
    let projections = (0..12).map(|_| async {
        let (availability, windows, size) = tokio::join!(
            f.backend.availability(false),
            f.backend.list_windows(),
            f.backend.get_screen_size()
        );
        availability.unwrap();
        windows.unwrap();
        size.unwrap();
    });
    futures::future::join_all(projections).await;
    let names: Vec<Option<String>> = f.calls().into_iter().map(|call| call.name).collect();
    assert_eq!(
        names,
        vec![
            Some("check_permissions".to_owned()),
            Some("list_windows".to_owned()),
            Some("get_screen_size".to_owned()),
        ]
    );
}

#[tokio::test]
async fn does_not_replay_identical_text_when_native_readback_is_unverifiable() {
    let f = fixture();
    f.backend.availability(false).await.unwrap();
    f.backend.focus_window("cua:10:20").await.unwrap();
    let result = f.backend.type_text("abc", None, None).await.unwrap();
    assert_eq!(
        result.verified,
        Some(ComputerDeliveryVerification::Unverifiable)
    );
    assert_eq!(f.named("type_text").len(), 1);
    assert_subset(
        &f.first_args("type_text"),
        json!({ "delivery_mode": "background", "text": "abc", "pid": 10, "window_id": 20 }),
    );
}

#[tokio::test]
async fn preserves_unknown_dispatch_on_transport_timeout_without_retry() {
    let f = fixture();
    f.backend.availability(false).await.unwrap();
    f.fail(json!({ "ok": false, "error": "timeout", "effect": "dispatched-unknown" }));
    let error = f
        .backend
        .type_text("abc", Some("cua:10:20"), None)
        .await
        .unwrap_err();
    assert_eq!(effect(&error), Some(CuaEffect::DispatchedUnknown));
    assert_eq!(f.named("type_text").len(), 1);
}

#[tokio::test]
async fn maps_negative_desktop_coordinates_using_the_captured_geometry_and_scale() {
    let f = fixture();
    f.backend.availability(false).await.unwrap();
    let image = f
        .backend
        .capture_screenshot(window_capture("cua:10:20"))
        .await
        .unwrap();
    assert_eq!(image.width, 400);
    assert_eq!(image.height, 200);
    assert_eq!(image.scale, Some(2.0));
    let region = image.region.unwrap();
    assert_eq!((region.x, region.y), (-300.0, 20.0));
    f.backend
        .click(point(-275.0, 30.0), Some("cua:10:20"), &[])
        .await
        .unwrap();
    assert_subset(
        &f.first_args("click"),
        json!({
            "x": 25,
            "y": 10,
            "coordinate_space": "window_points",
            "expected_window_bounds": { "x": -300, "y": 20, "width": 200, "height": 100 },
        }),
    );
}

#[tokio::test]
async fn refuses_a_moved_or_closed_window_without_injecting() {
    let f = fixture();
    f.backend.availability(false).await.unwrap();
    observed(&f).await;
    f.move_window();
    let error = f
        .backend
        .click(point(-275.0, 30.0), Some("cua:10:20"), &[])
        .await
        .unwrap_err();
    assert_eq!(effect(&error), Some(CuaEffect::NotDispatched));
    assert_eq!(code(&error), Some("stale_geometry"));
    f.close();
    let error = f
        .backend
        .type_text("abc", Some("cua:10:20"), None)
        .await
        .unwrap_err();
    assert_eq!(effect(&error), Some(CuaEffect::NotDispatched));
    assert_eq!(code(&error), Some("stale_target"));
    assert!(
        f.calls()
            .iter()
            .all(|call| !matches!(call.name.as_deref(), Some("click" | "type_text")))
    );
}

#[tokio::test]
async fn sends_an_exact_target_background_drag_as_window_local_points() {
    let f = fixture();
    observed(&f).await;
    f.backend
        .drag(
            point(-275.0, 30.0),
            point(-225.0, 50.0),
            500.0,
            Some("cua:10:20"),
        )
        .await
        .unwrap();
    let drags = f.named("drag");
    assert_eq!(drags.len(), 1);
    assert_eq!(
        drags[0].args,
        json!({
            "pid": 10,
            "window_id": 20,
            "delivery_mode": "background",
            "from_x": 25,
            "from_y": 10,
            "to_x": 75,
            "to_y": 30,
            "coordinate_space": "window_points",
            "duration_ms": 500,
            "expected_window_bounds": { "x": -300, "y": 20, "width": 200, "height": 100 },
        })
    );
}

#[tokio::test]
async fn refuses_a_background_drag_when_no_fresh_observation_grounds_the_frame() {
    let f = fixture();
    let error = f
        .backend
        .drag(
            point(-275.0, 30.0),
            point(-225.0, 50.0),
            500.0,
            Some("cua:10:20"),
        )
        .await
        .unwrap_err();
    assert_eq!(effect(&error), Some(CuaEffect::NotDispatched));
    assert_eq!(code(&error), Some("stale_geometry"));
    assert!(f.named("drag").is_empty());
}

#[tokio::test]
async fn refuses_a_background_drag_whose_endpoint_leaves_the_exact_window() {
    let f = fixture();
    observed(&f).await;
    let error = f
        .backend
        .drag(
            point(-275.0, 30.0),
            point(-90.0, 50.0),
            500.0,
            Some("cua:10:20"),
        )
        .await
        .unwrap_err();
    assert_eq!(effect(&error), Some(CuaEffect::NotDispatched));
    assert!(f.named("drag").is_empty());
}

#[tokio::test]
async fn refuses_a_drag_that_names_no_exact_window_before_reaching_cua() {
    let f = fixture();
    let error = f
        .backend
        .drag(point(0.0, 0.0), point(10.0, 10.0), 500.0, None)
        .await
        .unwrap_err();
    assert_eq!(effect(&error), Some(CuaEffect::NotDispatched));
    assert_eq!(code(&error), Some("window_required"));
    assert!(f.named("drag").is_empty());
}

#[tokio::test]
async fn propagates_the_native_background_admission_refusal_without_replay() {
    let f = fixture();
    observed(&f).await;
    f.on_tool("drag", |_| {
        json!({
            "isError": true,
            "structuredContent": {
                "effect": "refused",
                "code": "background_unavailable",
                "pid": 10,
                "window_id": 20,
            },
            "content": [{
                "type": "text",
                "text": "Background drag is unavailable on this driver; use delivery_mode:\"foreground\".",
            }],
        })
    });
    let error = f
        .backend
        .drag(
            point(-275.0, 30.0),
            point(-225.0, 50.0),
            500.0,
            Some("cua:10:20"),
        )
        .await
        .unwrap_err();
    assert_eq!(effect(&error), Some(CuaEffect::NotDispatched));
    assert_eq!(code(&error), Some("background_unavailable"));
    assert_eq!(f.named("drag").len(), 1);
}

#[tokio::test]
async fn still_sends_a_foreground_drag_as_window_local_points() {
    let f = fixture();
    observed(&f).await;
    foreground(f.backend.drag(
        point(-275.0, 30.0),
        point(-225.0, 50.0),
        500.0,
        Some("cua:10:20"),
    ))
    .await
    .unwrap();
    let drags = f.named("drag");
    assert_eq!(drags.len(), 1);
    assert_subset(
        &drags[0].args,
        json!({
            "delivery_mode": "foreground",
            "from_x": 25,
            "from_y": 10,
            "to_x": 75,
            "to_y": 30,
        }),
    );
}
