use zeron_proto::computer::CuaEffect;

use super::*;

fn space_fixture() -> Fixture {
    let f = fixture();
    f.set_bounds(json!({ "x": 0, "y": 0, "width": 200, "height": 100 }));
    f
}

fn assert_refused_off_space(error: &ComputerError) {
    assert_eq!(effect(error), Some(CuaEffect::NotDispatched));
    assert_eq!(code(error), Some("target_not_on_active_space"));
    assert_eq!(
        error
            .input_pause()
            .and_then(|pause| pause.window_id.as_deref()),
        Some("cua:10:20")
    );
}

#[tokio::test]
async fn a_space_change_between_prepare_and_dispatch_refuses_once_sends_no_drag_and_stays_observable()
 {
    let f = space_fixture();
    f.backend
        .capture_screenshot(window_capture("cua:10:20"))
        .await
        .unwrap();
    f.set_visible(false);
    let error = foreground(f.backend.drag(
        point(50.0, 50.0),
        point(100.0, 60.0),
        500.0,
        Some("cua:10:20"),
    ))
    .await
    .unwrap_err();
    assert_refused_off_space(&error);
    assert!(f.named("drag").is_empty());
    let state = f
        .backend
        .get_state(ComputerStateOptions {
            window_id: Some("cua:10:20".into()),
            ..ComputerStateOptions::default()
        })
        .await
        .unwrap();
    assert_eq!(state.computer_id, "desktop");
    assert!(f.named("drag").is_empty());
    f.backend.dispose().await;
}

#[tokio::test]
async fn refuses_a_background_drag_the_same_way_when_the_window_leaves_the_space() {
    let f = space_fixture();
    f.backend
        .capture_screenshot(window_capture("cua:10:20"))
        .await
        .unwrap();
    f.set_visible(false);
    let error = f
        .backend
        .drag(
            point(50.0, 50.0),
            point(100.0, 60.0),
            500.0,
            Some("cua:10:20"),
        )
        .await
        .unwrap_err();
    assert_refused_off_space(&error);
    assert!(f.named("drag").is_empty());
    f.backend.dispose().await;
}
