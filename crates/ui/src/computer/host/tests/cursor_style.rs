use super::*;
use crate::computer::host::CuaCursorStyle;

fn colors(fill: Option<&str>, rim: Option<&str>, shadow: Option<&str>) -> CuaCursorStyle {
    CuaCursorStyle {
        fill: fill.map(Into::into),
        rim: rim.map(Into::into),
        shadow: shadow.map(Into::into),
    }
}

fn fixed_style(style: Option<CuaCursorStyle>) -> Box<dyn Fn() -> Option<CuaCursorStyle> + Send> {
    Box::new(move || style.clone())
}

fn shared_style(
    style: Arc<Mutex<Option<CuaCursorStyle>>>,
) -> Box<dyn Fn() -> Option<CuaCursorStyle> + Send> {
    Box::new(move || style.lock().unwrap().clone())
}

fn press_key() -> ComputerHostCall {
    call("press_key").args(json!({ "key": "enter", "_synara_foreground_observation_ms": 0 }))
}

fn keys(payload: &Value) -> Vec<String> {
    let mut keys: Vec<String> = payload.as_object().unwrap().keys().cloned().collect();
    keys.sort();
    keys
}

#[tokio::test]
async fn pushes_the_custom_colors_once_per_generation_on_the_session_open() {
    let style = fixed_style(Some(colors(
        Some("#101010"),
        Some("#F0F0F0"),
        Some("#000000"),
    )));
    let f = fixture_with(Driver::default(), |o| o.cursor_style = Some(style));
    assert!(f.send(press_key()).await.ok);
    assert!(f.send(press_key()).await.ok);
    assert_eq!(f.count("motion-100-0"), 1);
    let payloads = f.style_payloads();
    assert_eq!(payloads.len(), 1);
    assert_eq!(keys(&payloads[0]), ["fill", "rim", "session", "shadow"]);
    assert!(
        payloads[0]["session"]
            .as_str()
            .unwrap()
            .starts_with("synara-")
    );
    assert_eq!(payloads[0]["fill"], json!("#101010"));
    assert_eq!(payloads[0]["rim"], json!("#f0f0f0"));
    assert_eq!(payloads[0]["shadow"], json!("#000000"));
}

#[tokio::test]
async fn sends_only_the_channels_that_carry_a_usable_color() {
    let style = fixed_style(Some(colors(Some("#ABCDEF"), Some("not-a-color"), None)));
    let f = fixture_with(Driver::default(), |o| o.cursor_style = Some(style));
    assert!(f.send(press_key()).await.ok);
    let payloads = f.style_payloads();
    assert_eq!(payloads.len(), 1);
    assert_eq!(keys(&payloads[0]), ["fill", "session"]);
    assert_eq!(payloads[0]["fill"], json!("#abcdef"));
}

#[tokio::test]
async fn makes_no_style_call_for_the_stock_default() {
    let f = fixture(Driver::default());
    assert!(f.send(press_key()).await.ok);
    assert!(f.has("motion-100-0"));
    assert!(f.style_payloads().is_empty());
}

#[tokio::test]
async fn makes_no_style_call_for_a_stock_resolving_option() {
    let f = fixture_with(Driver::default(), |o| {
        o.cursor_style = Some(fixed_style(None))
    });
    assert!(f.send(press_key()).await.ok);
    assert!(f.style_payloads().is_empty());
}

#[tokio::test]
async fn keeps_an_unpatched_upstream_driver_on_its_stock_cursor() {
    let f = fixture_with(
        Driver {
            unpatched: true,
            ..Driver::default()
        },
        |o| {
            o.native_revision = None;
            o.cursor_style = Some(fixed_style(Some(colors(Some("#101010"), None, None))));
        },
    );
    let reply = f.send(call("list_windows").args(json!({}))).await;
    assert!(reply.ok);
    assert_eq!(reply.driver_native_revision, Some(0));
    assert!(f.has("motion-100-0"));
    assert!(f.style_payloads().is_empty());
}

#[tokio::test]
async fn live_pushes_a_preference_change_to_the_warm_session_once() {
    let style = Arc::new(Mutex::new(Some(colors(Some("#101010"), None, None))));
    let getter = shared_style(style.clone());
    let f = fixture_with(Driver::default(), |o| o.cursor_style = Some(getter));
    assert!(f.send(press_key()).await.ok);
    let next = colors(Some("#101010"), Some("#f0f0f0"), None);
    *style.lock().unwrap() = Some(next.clone());
    f.host.set_cursor_style(Some(next.clone())).await;
    f.host.set_cursor_style(Some(next)).await;
    let payloads = f.style_payloads();
    assert_eq!(payloads.len(), 2);
    assert_eq!(payloads[1]["fill"], json!("#101010"));
    assert_eq!(payloads[1]["rim"], json!("#f0f0f0"));
    assert_eq!(payloads[1]["session"], payloads[0]["session"]);
}

#[tokio::test]
async fn resets_a_warm_session_to_stock_when_the_preference_clears() {
    let style = Arc::new(Mutex::new(Some(colors(Some("#101010"), None, None))));
    let getter = shared_style(style.clone());
    let f = fixture_with(Driver::default(), |o| o.cursor_style = Some(getter));
    assert!(f.send(press_key()).await.ok);
    *style.lock().unwrap() = None;
    f.host.set_cursor_style(None).await;
    let payloads = f.style_payloads();
    assert_eq!(payloads.len(), 2);
    assert_eq!(keys(&payloads[1]), ["session"]);
}

#[tokio::test]
async fn never_spawns_a_driver_just_to_apply_a_settings_change() {
    let style = fixed_style(Some(colors(Some("#101010"), None, None)));
    let f = fixture_with(Driver::default(), |o| o.cursor_style = Some(style));
    f.host
        .set_cursor_style(Some(colors(Some("#101010"), None, None)))
        .await;
    assert!(f.events().is_empty());
    assert!(f.send(press_key()).await.ok);
    assert_eq!(f.style_payloads().len(), 1);
}

#[tokio::test]
async fn styles_a_tasks_own_cursor_session_before_its_first_action_once() {
    let style = fixed_style(Some(colors(Some("#101010"), None, None)));
    let f = fixture_with(Driver::default(), |o| o.cursor_style = Some(style));
    let owner = task("thread", Some("turn"));
    for _ in 0..2 {
        assert!(f.send(press_key().task(&owner)).await.ok);
    }
    let payloads = f.style_payloads();
    assert_eq!(payloads.len(), 2);
    assert_eq!(payloads[1]["session"], json!("agent·thread"));
    assert_eq!(payloads[1]["fill"], json!("#101010"));
}

#[tokio::test]
async fn resets_a_task_cursor_session_to_stock_on_its_next_action() {
    let style = Arc::new(Mutex::new(Some(colors(Some("#101010"), None, None))));
    let getter = shared_style(style.clone());
    let f = fixture_with(Driver::default(), |o| o.cursor_style = Some(getter));
    let owner = task("thread", Some("turn"));
    assert!(f.send(press_key().task(&owner)).await.ok);
    *style.lock().unwrap() = None;
    f.host.set_cursor_style(None).await;
    assert!(f.send(press_key().task(&owner)).await.ok);
    assert!(f.send(press_key().task(&owner)).await.ok);
    let payloads = f.style_payloads();
    assert_eq!(payloads.len(), 4);
    assert_eq!(keys(&payloads[2]), ["session"]);
    assert_eq!(payloads[3]["session"], json!("agent·thread"));
    assert_eq!(keys(&payloads[3]), ["session"]);
}

#[tokio::test]
async fn sends_no_style_call_for_a_stock_task_session() {
    let f = fixture(Driver::default());
    assert!(
        f.send(press_key().task(&task("thread", Some("turn"))))
            .await
            .ok
    );
    assert!(f.style_payloads().is_empty());
}
