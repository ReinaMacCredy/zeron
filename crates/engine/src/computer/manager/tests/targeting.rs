use super::*;
use std::sync::atomic::AtomicUsize;

use zeron_proto::computer::{ComputerDeliveryVerification, CuaEffect};

use crate::computer::backend::{ComputerBackendError, ComputerMenuTarget};
use crate::computer::element_identity::ObservedComputerTarget;
use crate::computer::manager::ComputerClickGesture;

fn target_not_found(error: &ComputerError) -> bool {
    matches!(error, ComputerError::Target(error) if error.not_found)
}

fn json_point(x: f64, y: f64) -> Value {
    json!({ "x": x, "y": y })
}

fn last_args(backend: &Scripted, method: &str) -> Option<Vec<Value>> {
    backend
        .calls_for(method)
        .last()
        .map(|call| call.args.clone())
}

fn window_target(window_id: &str) -> ObservedComputerTarget {
    ComputerTarget {
        window_id: Some(window_id.to_owned()),
        ..ComputerTarget::default()
    }
    .into()
}

fn range(start: u64, length: u64) -> ComputerTextRange {
    ComputerTextRange { start, length }
}

fn keys(keys: &[&str]) -> Vec<String> {
    keys.iter().map(|key| (*key).to_owned()).collect()
}

fn covered_backend() -> Arc<Scripted> {
    Scripted::new(FakeComputerBackendOptions {
        windows: Some(covered_calculator_windows()),
        ..FakeComputerBackendOptions::default()
    })
}

fn menu_apps() -> Vec<ComputerApp> {
    vec![
        app(6_001, "Helium", "net.imput.helium", false),
        app(1_001, "Terminal", "org.kde.konsole", true),
    ]
}

#[tokio::test]
async fn dispatches_a_supported_semantic_click_once_and_never_retries_an_uncertain_ax_effect() {
    let backend = Scripted::default_fake();
    backend.set_dialect(ComputerAgentDialect::Macos);
    let presses: Arc<Mutex<Vec<String>>> = Arc::default();
    let reject_next = Arc::new(AtomicBool::new(false));
    {
        let mut hooks = backend.hooks();
        hooks.supports_action = Some(Arc::new(|_, action| action == "AXPress"));
        let presses = presses.clone();
        let reject_next = reject_next.clone();
        hooks.perform_action = Some(Arc::new(move |_, (_, action)| {
            presses.lock().unwrap().push(action);
            let reject = reject_next.swap(false, Ordering::SeqCst);
            async move {
                if reject {
                    return Err(ComputerBackendError::new("Unknown dispatch").into());
                }
                Ok(ComputerBackendActionResult {
                    effect: Some(CuaEffect::DispatchedUnknown),
                    verified: Some(ComputerDeliveryVerification::Unverifiable),
                    ..ComputerBackendActionResult::default()
                })
            }
            .boxed()
        }));
    }
    let Built {
        manager, backend, ..
    } = manager(backend);
    manager
        .click(
            Some("thread-1"),
            labelled("Calculate", Some("button")),
            &[],
            ComputerClickGesture::default(),
        )
        .await
        .unwrap();
    assert_eq!(presses.lock().unwrap().len(), 1);
    assert_eq!(presses.lock().unwrap()[0], "AXPress");
    assert!(backend.calls_for("click").is_empty());
    reject_next.store(true, Ordering::SeqCst);
    let error = manager
        .click(
            Some("thread-1"),
            labelled("Calculate", Some("button")),
            &[],
            ComputerClickGesture::default(),
        )
        .await
        .unwrap_err();
    assert!(error.to_string().contains("Unknown dispatch"), "{error}");
    assert_eq!(presses.lock().unwrap().len(), 2);
    assert!(backend.calls_for("click").is_empty());
    manager
        .double_click(Some("thread-1"), labelled("Calculate", Some("button")), &[])
        .await
        .unwrap();
    manager
        .click(
            Some("thread-1"),
            labelled("Calculate", Some("button")),
            &[ComputerInputModifier::Shift],
            ComputerClickGesture::default(),
        )
        .await
        .unwrap();
    assert_eq!(presses.lock().unwrap().len(), 2);
    assert_eq!(backend.calls_for("doubleClick").len(), 1);
    assert_eq!(backend.calls_for("click").len(), 1);
    manager.dispose().await;
}

#[tokio::test]
async fn re_walks_a_fresh_tree_when_a_cached_tree_target_lookup_misses() {
    let backend = Scripted::default_fake();
    let calls = Arc::new(AtomicUsize::new(0));
    {
        let calls = calls.clone();
        backend.hooks().get_state = Some(Arc::new(move |fake, options| {
            let calls = calls.clone();
            async move {
                let count = calls.fetch_add(1, Ordering::SeqCst) + 1;
                let mut state = fake.get_state(options).await?;
                if count == 1
                    && let Some(root) = state.root.as_mut()
                {
                    for child in &mut root.children {
                        child.children.clear();
                    }
                }
                Ok(state)
            }
            .boxed()
        }));
    }
    let Built {
        manager, backend, ..
    } = manager(backend);
    let result = manager
        .click(
            Some("thread-1"),
            labelled("Calculate", Some("button")),
            &[],
            ComputerClickGesture::default(),
        )
        .await
        .unwrap();
    assert_eq!(
        result.point,
        Some(ComputerPoint {
            x: 1_180.0,
            y: 228.0
        })
    );
    assert_eq!(calls.load(Ordering::SeqCst), 2);
    let states = backend.calls_for("getState");
    assert_eq!(states[0].args[0]["reuseRecentTree"], json!(true));
    assert_ne!(states[1].args[0]["reuseRecentTree"], json!(true));
    manager.dispose().await;
}

#[tokio::test]
async fn falls_back_to_a_coordinate_click_when_no_axpress_token_is_advertised() {
    let backend = Scripted::default_fake();
    backend.set_dialect(ComputerAgentDialect::Macos);
    backend.hooks().supports_action = Some(Arc::new(|_, _| false));
    let Built {
        manager, backend, ..
    } = manager(backend);
    manager
        .click(
            Some("thread-1"),
            labelled("Calculate", Some("button")),
            &[],
            ComputerClickGesture::default(),
        )
        .await
        .unwrap();
    assert!(backend.logged("performAction").is_empty());
    assert_eq!(backend.calls_for("click").len(), 1);
    manager.dispose().await;
}

#[tokio::test]
async fn performs_semantic_writes_only_against_a_fresh_unambiguous_snapshot() {
    let Built { manager, .. } = manager(Scripted::default_fake());
    let written = manager
        .set_value(Some("thread-1"), labelled("Display", None), "468")
        .await
        .unwrap();
    assert_eq!(written.action, "computer_set_value");
    assert_eq!(written.value.as_deref(), Some("468"));
    let performed = manager
        .perform_action(
            Some("thread-1"),
            labelled("Calculate", Some("button")),
            "activate",
        )
        .await
        .unwrap();
    assert_eq!(performed.action, "computer_perform_action");
    assert_eq!(
        performed.point,
        Some(ComputerPoint {
            x: 1_180.0,
            y: 228.0
        })
    );
    let selected = manager
        .select_text(Some("thread-1"), labelled("Display", None), range(0, 2))
        .await
        .unwrap();
    assert_eq!(selected.action, "computer_select_text");
    assert_eq!(selected.value.as_deref(), Some("46"));

    let error = manager
        .click(
            Some("thread-1"),
            point(1_920.0, 1_080.0),
            &[],
            ComputerClickGesture::default(),
        )
        .await
        .unwrap_err();
    assert_eq!(code(&error), Some("computer_target_offscreen"));
    let half = ComputerTarget {
        x: Some(10.0),
        ..ComputerTarget::default()
    };
    let error = manager
        .click(Some("thread-1"), half, &[], ComputerClickGesture::default())
        .await
        .unwrap_err();
    assert_eq!(code(&error), Some("computer_target_invalid"));

    let scrolled = manager
        .scroll(
            Some("thread-1"),
            Some(window_target("fake-calculator")),
            0.0,
            300.0,
        )
        .await
        .unwrap();
    assert_eq!(scrolled.action, "computer_scroll");
    assert_eq!(
        scrolled.point,
        Some(ComputerPoint {
            x: 1_260.0,
            y: 430.0
        })
    );
    let error = manager
        .set_value(Some("thread-1"), ComputerTarget::default(), "468")
        .await
        .unwrap_err();
    assert_eq!(code(&error), Some("computer_target_invalid"));
    let error = manager
        .perform_action(Some("thread-1"), ComputerTarget::default(), "activate")
        .await
        .unwrap_err();
    assert_eq!(code(&error), Some("computer_target_invalid"));
    let error = manager
        .select_text(Some("thread-1"), ComputerTarget::default(), range(0, 1))
        .await
        .unwrap_err();
    assert_eq!(code(&error), Some("computer_target_invalid"));

    manager.dispose().await;
}

#[tokio::test]
async fn reveals_a_hover_target_without_changing_keyboard_aim() {
    let Built {
        manager, backend, ..
    } = manager(covered_backend());
    manager
        .move_cursor(Some("thread-1"), point(100.0, 100.0))
        .await
        .unwrap();
    let cleared = backend.calls_for("clearFocusWindow").len();
    manager
        .move_cursor(Some("thread-1"), scoped(1_100.0, 200.0, "fake-calculator"))
        .await
        .unwrap();
    assert_eq!(
        last_args(&backend, "raiseWindow"),
        Some(vec![json!("fake-calculator")])
    );
    assert!(backend.calls_for("focusWindow").is_empty());
    assert_eq!(backend.calls_for("clearFocusWindow").len(), cleared);
    assert_eq!(
        last_args(&backend, "moveCursor").unwrap()[0],
        json_point(1_100.0, 200.0)
    );
    manager.dispose().await;
}

#[tokio::test]
async fn raises_a_target_window_before_focusing_it_and_scopes_a_coordinate_click_to_it() {
    let Built {
        manager, backend, ..
    } = manager(covered_backend());

    manager
        .click(
            Some("thread-1"),
            labelled("Calculate", Some("button")),
            &[],
            ComputerClickGesture::default(),
        )
        .await
        .unwrap();
    let order: Vec<String> = backend
        .methods()
        .into_iter()
        .filter(|method| ["raiseWindow", "focusWindow", "click"].contains(&method.as_str()))
        .collect();
    assert_eq!(order, ["raiseWindow", "focusWindow", "click"]);
    assert_eq!(
        last_args(&backend, "raiseWindow"),
        Some(vec![json!("fake-calculator")])
    );

    let perception_calls = backend.calls_for("getState").len();
    let scoped_click = manager
        .click(
            Some("thread-1"),
            scoped(1_100.0, 200.0, "fake-calculator"),
            &[],
            ComputerClickGesture::default(),
        )
        .await
        .unwrap();
    assert_eq!(
        scoped_click.point,
        Some(ComputerPoint {
            x: 1_100.0,
            y: 200.0
        })
    );
    assert_eq!(
        last_args(&backend, "raiseWindow"),
        Some(vec![json!("fake-calculator")])
    );
    assert_eq!(
        last_args(&backend, "focusWindow"),
        Some(vec![json!("fake-calculator")])
    );
    assert_eq!(
        last_args(&backend, "click").unwrap()[0],
        json_point(1_100.0, 200.0)
    );
    assert_eq!(backend.calls_for("getState").len(), perception_calls);

    let error = manager
        .click(
            Some("thread-1"),
            scoped(40.0, 40.0, "fake-calculator"),
            &[],
            ComputerClickGesture::default(),
        )
        .await
        .unwrap_err();
    assert_eq!(code(&error), Some("computer_target_offscreen"));
    let error = manager
        .click(
            Some("thread-1"),
            scoped(40.0, 40.0, "gone"),
            &[],
            ComputerClickGesture::default(),
        )
        .await
        .unwrap_err();
    assert_eq!(code(&error), Some("computer_target_not_found"));
    assert!(target_not_found(&error));

    manager.dispose().await;
}

#[tokio::test]
async fn scopes_a_drag_to_the_window_its_origin_names() {
    let Built {
        manager, backend, ..
    } = manager(Scripted::default_fake());

    manager
        .drag(
            Some("thread-1"),
            scoped(1_100.0, 200.0, "fake-calculator"),
            scoped(1_200.0, 400.0, "fake-calculator"),
            Some(400.0),
        )
        .await
        .unwrap();
    assert_eq!(
        last_args(&backend, "focusWindow"),
        Some(vec![json!("fake-calculator")])
    );
    assert_eq!(
        last_args(&backend, "drag"),
        Some(vec![
            json_point(1_100.0, 200.0),
            json_point(1_200.0, 400.0),
            json!(400.0),
        ])
    );

    manager.dispose().await;
}

#[tokio::test]
async fn keeps_window_targeting_working_on_a_backend_that_cannot_raise_windows() {
    let backend = Scripted::default_fake();
    backend.remove(ComputerBackendMethod::RaiseWindow);
    let Built {
        manager, backend, ..
    } = manager(backend);

    let result = manager
        .click(
            Some("thread-1"),
            labelled("Calculate", Some("button")),
            &[],
            ComputerClickGesture::default(),
        )
        .await
        .unwrap();
    assert_eq!(
        result.point,
        Some(ComputerPoint {
            x: 1_180.0,
            y: 228.0
        })
    );
    assert!(backend.calls_for("raiseWindow").is_empty());
    assert_eq!(
        last_args(&backend, "focusWindow"),
        Some(vec![json!("fake-calculator")])
    );

    manager.dispose().await;
}

#[tokio::test]
async fn refuses_a_covered_target_the_desktop_cannot_raise_and_clicks_it_once_it_can() {
    let backend = covered_backend();
    backend.remove(ComputerBackendMethod::RaiseWindow);
    let Built {
        manager, backend, ..
    } = manager(backend);

    let covered = manager
        .click(
            Some("thread-1"),
            scoped(1_100.0, 200.0, "fake-calculator"),
            &[],
            ComputerClickGesture::default(),
        )
        .await
        .unwrap_err();
    assert_eq!(code(&covered), Some("computer_target_occluded"));
    assert!(covered.to_string().contains("Browser"), "{covered}");
    assert!(backend.calls_for("click").is_empty());
    assert!(backend.calls_for("focusWindow").is_empty());

    let error = manager
        .click(
            Some("thread-1"),
            labelled("Calculate", Some("button")),
            &[],
            ComputerClickGesture::default(),
        )
        .await
        .unwrap_err();
    assert_eq!(code(&error), Some("computer_target_occluded"));

    let result = manager
        .click(
            Some("thread-1"),
            scoped(1_100.0, 200.0, "fake-browser"),
            &[],
            ComputerClickGesture::default(),
        )
        .await
        .unwrap();
    assert_eq!(
        result.point,
        Some(ComputerPoint {
            x: 1_100.0,
            y: 200.0
        })
    );
    assert_eq!(result.window_id.as_deref(), Some("fake-browser"));

    manager.dispose().await;
}

#[tokio::test]
async fn refuses_a_covered_target_when_the_raise_itself_fails() {
    let Built {
        manager, backend, ..
    } = manager(covered_backend());
    backend.fake.fail_next_with(
        "raiseWindow",
        ComputerBackendError::new("plugin has no raiseWindow"),
    );

    let error = manager
        .click(
            Some("thread-1"),
            scoped(1_100.0, 200.0, "fake-calculator"),
            &[],
            ComputerClickGesture::default(),
        )
        .await
        .unwrap_err();
    assert!(
        error.to_string().contains("plugin has no raiseWindow"),
        "{error}"
    );
    assert!(backend.calls_for("click").is_empty());

    let result = manager
        .click(
            Some("thread-1"),
            scoped(1_100.0, 200.0, "fake-calculator"),
            &[],
            ComputerClickGesture::default(),
        )
        .await
        .unwrap();
    assert_eq!(
        result.point,
        Some(ComputerPoint {
            x: 1_100.0,
            y: 200.0
        })
    );
    assert_eq!(result.window_id.as_deref(), Some("fake-calculator"));

    manager.dispose().await;
}

#[tokio::test]
async fn routes_keyboard_input_to_a_named_window_and_leaves_focus_alone_without_one() {
    let Built {
        manager, backend, ..
    } = settled(covered_backend());

    let typed = manager
        .type_text(Some("thread-1"), "12", Some("fake-calculator"))
        .await
        .unwrap();
    assert_eq!(typed.action, "computer_type_text");
    assert_eq!(typed.window_id.as_deref(), Some("fake-calculator"));
    assert_eq!(
        last_args(&backend, "raiseWindow"),
        Some(vec![json!("fake-calculator")])
    );
    assert_eq!(
        last_args(&backend, "focusWindow"),
        Some(vec![json!("fake-calculator")])
    );

    let pressed = manager
        .press_key(Some("thread-1"), "enter", Some("fake-browser"), None)
        .await
        .unwrap();
    assert_eq!(pressed.window_id.as_deref(), Some("fake-browser"));
    assert_eq!(
        last_args(&backend, "focusWindow"),
        Some(vec![json!("fake-browser")])
    );
    let chord = manager
        .hotkey(
            Some("thread-1"),
            &keys(&["ctrl", "t"]),
            Some("fake-browser"),
            None,
        )
        .await
        .unwrap();
    assert_eq!(chord.window_id.as_deref(), Some("fake-browser"));

    let focus_calls = backend.calls_for("focusWindow").len();
    let unscoped = manager
        .type_text(Some("thread-1"), "9", None)
        .await
        .unwrap();
    assert_eq!(unscoped.window_id, None);
    assert_eq!(backend.calls_for("focusWindow").len(), focus_calls);
    assert_eq!(backend.calls_for("clearFocusWindow").len(), 1);

    let type_calls = backend.calls_for("typeText").len();
    let error = manager
        .type_text(Some("thread-1"), "9", Some("gone"))
        .await
        .unwrap_err();
    assert_eq!(code(&error), Some("computer_target_not_found"));
    assert!(target_not_found(&error));
    assert_eq!(backend.calls_for("typeText").len(), type_calls);

    manager.dispose().await;
}

#[tokio::test]
async fn explains_a_scoped_injection_the_desktop_refused_and_passes_other_failures_through() {
    let Built {
        manager, backend, ..
    } = settled(covered_backend());

    backend.fake.fail_next_with(
        "click",
        ComputerBackendError {
            retryable: true,
            rejected_operation: Some("pressButton".into()),
            ..ComputerBackendError::new("The computer backend rejected pressButton.")
        },
    );
    let refused = manager
        .click(
            Some("thread-1"),
            scoped(1_100.0, 200.0, "fake-calculator"),
            &[],
            ComputerClickGesture::default(),
        )
        .await
        .unwrap_err();
    assert_eq!(code(&refused), Some("computer_target_refused"));
    assert!(
        refused.to_string().contains("no input was sent"),
        "{refused}"
    );
    assert!(
        refused
            .to_string()
            .contains("label instead of a coordinate"),
        "{refused}"
    );

    backend.fake.fail_next_with(
        "click",
        ComputerBackendError {
            rejected_operation: Some("pressButton".into()),
            ..ComputerBackendError::new("The computer backend rejected pressButton.")
        },
    );
    let error = manager
        .click(
            Some("thread-1"),
            point(1_100.0, 200.0),
            &[],
            ComputerClickGesture::default(),
        )
        .await
        .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("computer backend rejected pressButton"),
        "{error}"
    );

    backend.fake.fail_next_with(
        "click",
        ComputerBackendError::new("session bus disconnected"),
    );
    let error = manager
        .click(
            Some("thread-1"),
            scoped(1_100.0, 200.0, "fake-calculator"),
            &[],
            ComputerClickGesture::default(),
        )
        .await
        .unwrap_err();
    assert!(
        error.to_string().contains("session bus disconnected"),
        "{error}"
    );

    manager.dispose().await;
}

#[tokio::test]
async fn attributes_every_action_event_to_the_thread_that_drove_it() {
    let Built { manager, .. } = manager(Scripted::default_fake());
    let events = events(&manager);
    let t = Some("thread-1");

    manager
        .launch_app(t, "kcalc", &[], 0, ComputerLaunchOptions::default())
        .await
        .unwrap();
    manager
        .click(t, point(10.0, 10.0), &[], ComputerClickGesture::default())
        .await
        .unwrap();
    manager
        .double_click(t, point(10.0, 10.0), &[])
        .await
        .unwrap();
    manager
        .right_click(t, point(10.0, 10.0), &[])
        .await
        .unwrap();
    manager.move_cursor(t, point(10.0, 10.0)).await.unwrap();
    manager
        .drag(t, point(10.0, 10.0), point(20.0, 20.0), None)
        .await
        .unwrap();
    manager.scroll(t, None, 0.0, 12.0).await.unwrap();
    manager.type_text(t, "hi", None).await.unwrap();
    manager.press_key(t, "enter", None, None).await.unwrap();
    manager
        .hotkey(t, &keys(&["ctrl", "s"]), None, None)
        .await
        .unwrap();
    manager.write_clipboard(t, "clip").await.unwrap();
    manager.read_clipboard(t).await.unwrap();
    manager
        .set_value(t, labelled("Display", None), "12")
        .await
        .unwrap();
    manager
        .perform_action(t, labelled("Calculate", Some("button")), "activate")
        .await
        .unwrap();
    manager
        .select_text(t, labelled("Display", None), range(0, 1))
        .await
        .unwrap();

    let actions: Vec<(String, Option<String>)> = events
        .actions()
        .into_iter()
        .map(|event| match event.event {
            ComputerEvent::Action {
                action, thread_id, ..
            } => (action, thread_id),
            _ => unreachable!(),
        })
        .collect();
    let expected: Vec<(String, Option<String>)> = [
        "computer_launch_app",
        "computer_click",
        "computer_click",
        "computer_click",
        "computer_move_cursor",
        "computer_drag",
        "computer_scroll",
        "computer_type_text",
        "computer_press_key",
        "computer_press_key",
        "computer_write_clipboard",
        "computer_read_clipboard",
        "computer_set_value",
        "computer_perform_action",
        "computer_select_text",
    ]
    .into_iter()
    .map(|action| (action.to_owned(), Some("thread-1".to_owned())))
    .collect();
    assert_eq!(actions, expected);

    manager.dispose().await;
}

#[tokio::test]
async fn carries_clipboard_text_on_the_shared_action_result_and_refuses_it_without_backend_support()
{
    let Built { manager, .. } = manager(Scripted::default_fake());

    manager
        .write_clipboard(Some("thread-1"), "shared text")
        .await
        .unwrap();
    let read = manager.read_clipboard(Some("thread-1")).await.unwrap();
    assert_eq!(read.action, "computer_read_clipboard");
    assert_eq!(read.value.as_deref(), Some("shared text"));

    manager
        .write_clipboard(Some("thread-1"), &"x".repeat(16 * 1024 + 1))
        .await
        .unwrap();
    let error = manager.read_clipboard(Some("thread-1")).await.unwrap_err();
    assert!(error.to_string().contains("more than the 16384"), "{error}");

    let bare = Scripted::default_fake();
    bare.remove(ComputerBackendMethod::ReadClipboard);
    bare.remove(ComputerBackendMethod::WriteClipboard);
    let Built {
        manager: without_clipboard,
        ..
    } = super::manager(bare);
    let error = without_clipboard
        .read_clipboard(Some("thread-1"))
        .await
        .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("does not support clipboard access"),
        "{error}"
    );
    let error = without_clipboard
        .write_clipboard(Some("thread-1"), "nope")
        .await
        .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("does not support clipboard access"),
        "{error}"
    );

    manager.dispose().await;
    without_clipboard.dispose().await;
}

#[tokio::test]
async fn tells_the_backend_which_thread_is_driving_so_the_agent_cursor_can_name_it() {
    let Built {
        manager, backend, ..
    } = manager(Scripted::default_fake());
    let names = || backend.logged("setDrivingAgent");

    manager
        .click(
            None,
            point(10.0, 10.0),
            &[],
            ComputerClickGesture::default(),
        )
        .await
        .unwrap();
    assert_eq!(names(), Vec::<Value>::new());

    manager.set_thread_label("thread-1", Some("Luna"));
    manager
        .click(
            Some("thread-1"),
            point(10.0, 10.0),
            &[],
            ComputerClickGesture::default(),
        )
        .await
        .unwrap();
    assert_eq!(names(), vec![json!("Luna")]);

    manager.set_thread_label("thread-1", Some("Luna · seat fix"));
    assert_eq!(names(), vec![json!("Luna"), json!("Luna · seat fix")]);

    manager
        .release_desktop_control("thread-1", None)
        .await
        .unwrap();
    manager
        .click(
            Some("thread-2"),
            point(10.0, 10.0),
            &[],
            ComputerClickGesture::default(),
        )
        .await
        .unwrap();
    assert_eq!(
        names(),
        vec![
            json!("Luna"),
            json!("Luna · seat fix"),
            Value::Null,
            Value::Null
        ]
    );

    manager.set_thread_label("thread-1", Some("Luna again"));
    assert_eq!(names().len(), 4);

    manager.dispose().await;
}

#[tokio::test]
async fn asks_the_ui_to_open_the_pane_once_per_thread_and_only_for_agent_actions() {
    let Built { manager, .. } = manager(Scripted::default_fake());
    let events = events(&manager);

    manager
        .click(
            None,
            point(10.0, 10.0),
            &[],
            ComputerClickGesture::default(),
        )
        .await
        .unwrap();
    assert_eq!(events.open_pane_requests(), Vec::<String>::new());

    manager
        .click(
            Some("thread-1"),
            point(10.0, 10.0),
            &[],
            ComputerClickGesture::default(),
        )
        .await
        .unwrap();
    manager
        .type_text(Some("thread-1"), "hi", None)
        .await
        .unwrap();
    assert_eq!(events.open_pane_requests(), ["thread-1"]);

    manager
        .release_desktop_control("thread-1", None)
        .await
        .unwrap();
    manager
        .click(
            Some("thread-1"),
            point(20.0, 20.0),
            &[],
            ComputerClickGesture::default(),
        )
        .await
        .unwrap();
    assert_eq!(events.open_pane_requests(), ["thread-1", "thread-1"]);
    manager
        .type_text(Some("thread-1"), "again", None)
        .await
        .unwrap();
    assert_eq!(events.open_pane_requests(), ["thread-1", "thread-1"]);

    manager
        .release_desktop_control("thread-1", None)
        .await
        .unwrap();
    manager
        .press_key(Some("thread-2"), "enter", None, None)
        .await
        .unwrap();
    assert_eq!(
        events.open_pane_requests(),
        ["thread-1", "thread-1", "thread-2"]
    );

    manager.handle_thread_removed("thread-2").await;
    let error = manager
        .press_key(Some("thread-2"), "enter", None, None)
        .await
        .unwrap_err();
    assert!(error.to_string().contains("revoked"), "{error}");
    manager.handle_thread_restored("thread-2").await;
    manager
        .press_key(Some("thread-2"), "enter", None, None)
        .await
        .unwrap();
    assert_eq!(
        events.open_pane_requests(),
        ["thread-1", "thread-1", "thread-2", "thread-2"]
    );

    manager.dispose().await;
}

#[tokio::test]
async fn re_surfaces_the_pane_for_an_evicted_owner_without_waiting_for_its_release() {
    let (now, clock_fn) = clock(0);
    let Built { manager, .. } = build(
        Scripted::default_fake(),
        Setup {
            now: Some(clock_fn),
            lease_idle_ms: Some(1_000),
            ..Setup::default()
        },
    );
    let events = events(&manager);

    manager
        .click(
            Some("thread-a"),
            point(10.0, 10.0),
            &[],
            ComputerClickGesture::default(),
        )
        .await
        .unwrap();
    assert_eq!(events.open_pane_requests(), ["thread-a"]);

    *now.lock().unwrap() = 2_000;
    manager
        .click(
            Some("thread-b"),
            point(20.0, 20.0),
            &[],
            ComputerClickGesture::default(),
        )
        .await
        .unwrap();
    assert_eq!(events.open_pane_requests(), ["thread-a", "thread-b"]);

    *now.lock().unwrap() = 4_000;
    manager
        .click(
            Some("thread-a"),
            point(30.0, 30.0),
            &[],
            ComputerClickGesture::default(),
        )
        .await
        .unwrap();
    assert_eq!(
        events.open_pane_requests(),
        ["thread-a", "thread-b", "thread-a"]
    );

    manager.dispose().await;
}

#[tokio::test]
async fn drives_a_second_ordinary_app_without_asking() {
    let Built {
        manager,
        backend,
        seams,
    } = manager(Scripted::default_fake());
    manager
        .launch_app(
            Some("thread-1"),
            "kcalc",
            &[],
            0,
            ComputerLaunchOptions::default(),
        )
        .await
        .unwrap();
    manager
        .launch_app(
            Some("thread-1"),
            "firefox",
            &[],
            0,
            ComputerLaunchOptions::default(),
        )
        .await
        .unwrap();
    assert_eq!(backend.calls_for("launchApp").len(), 2);
    assert!(seams.consent.lock().unwrap().is_empty());
    manager.approval_gate().cancel_thread("thread-1", None);
    manager.dispose().await;
}

#[tokio::test]
async fn applies_visibility_writes_to_any_non_denied_app() {
    let Built {
        manager, backend, ..
    } = manager(Scripted::default_fake());
    manager
        .set_window_minimized(Some("thread-1"), "fake-terminal", true)
        .await
        .unwrap();
    manager
        .set_app_visibility(Some("thread-1"), 1_002, true)
        .await
        .unwrap();
    assert_eq!(backend.calls_for("setAppVisibility").len(), 1);
    let error = manager
        .set_app_visibility(Some("thread-1"), 9_999, true)
        .await
        .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("No running application has pid 9999"),
        "{error}"
    );
    manager.approval_gate().cancel_thread("thread-1", None);
    manager.dispose().await;
}

#[tokio::test]
async fn invokes_a_windowless_menu_target_on_the_app_names_live_pid() {
    let Built {
        manager, backend, ..
    } = manager(Scripted::new(FakeComputerBackendOptions {
        apps: Some(menu_apps()),
        ..FakeComputerBackendOptions::default()
    }));
    let file = keys(&["File"]);
    let error = manager
        .invoke_menu(
            Some("thread-1"),
            ComputerMenuTarget::App("Helium".into()),
            &file,
            None,
        )
        .await
        .unwrap_err();
    assert_eq!(code(&error), Some("foreground_not_requested"));
    assert_eq!(error.effect(), Some(CuaEffect::NotDispatched));
    assert!(backend.calls_for("invokeMenu").is_empty());
    let result = manager
        .invoke_menu(
            Some("thread-1"),
            ComputerMenuTarget::App("Helium".into()),
            &keys(&["File", "New Window"]),
            visible_use(),
        )
        .await
        .unwrap();
    assert_eq!(
        last_args(&backend, "invokeMenu"),
        Some(vec![json!({ "pid": 6_001 }), json!(["File", "New Window"])])
    );
    assert_eq!(result.window_id, None);
    let error = manager
        .invoke_menu(
            Some("thread-1"),
            ComputerMenuTarget::App("Ghost".into()),
            &file,
            visible_use(),
        )
        .await
        .unwrap_err();
    assert_eq!(code(&error), Some("computer_target_not_found"));
    assert!(target_not_found(&error));
    assert_eq!(backend.calls_for("invokeMenu").len(), 1);
    let error = manager
        .invoke_menu(
            Some("thread-1"),
            ComputerMenuTarget::Pid(9_999),
            &file,
            visible_use(),
        )
        .await
        .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("No running application has pid 9999"),
        "{error}"
    );
    manager.approval_gate().cancel_thread("thread-1", None);
    manager.dispose().await;
}

#[tokio::test]
async fn says_plainly_when_an_unhide_ran_on_an_app_with_no_windows() {
    let mut apps = menu_apps();
    apps.push(app(1_002, "Calculator", "org.kde.kcalc", false));
    let Built { manager, .. } = manager(Scripted::new(FakeComputerBackendOptions {
        apps: Some(apps),
        ..FakeComputerBackendOptions::default()
    }));
    manager.list_windows().await.unwrap();
    let shown = manager
        .set_app_visibility(None, 6_001, false)
        .await
        .unwrap();
    assert_eq!(
        shown
            .result
            .delivery
            .as_ref()
            .map(|delivery| delivery.verified),
        Some(ComputerDeliveryVerification::Confirmed)
    );
    let note = shown.note.unwrap();
    assert!(note.contains("no window"), "{note}");
    assert!(note.contains("computer_invoke_menu"), "{note}");
    assert!(note.contains("computer_browser_prepare"), "{note}");
    assert!(note.contains("computer_browser_state"), "{note}");
    assert!(!note.contains("allow_launch"), "{note}");
    let hidden = manager.set_app_visibility(None, 6_001, true).await.unwrap();
    assert_eq!(hidden.note, None);
    let with_window = manager
        .set_app_visibility(None, 1_002, false)
        .await
        .unwrap();
    assert_eq!(with_window.note, None);
    manager.dispose().await;
}

#[tokio::test]
async fn keeps_background_launch_separate_from_explicitly_hiding_the_application() {
    let Built {
        manager, backend, ..
    } = manager(Scripted::default_fake());
    manager
        .launch_app(
            Some("thread-1"),
            "kcalc",
            &[],
            0,
            ComputerLaunchOptions::default(),
        )
        .await
        .unwrap();
    assert_eq!(
        last_args(&backend, "launchApp"),
        Some(vec![json!("kcalc"), json!([])])
    );
    manager
        .launch_app(
            Some("thread-1"),
            "kcalc",
            &[],
            0,
            ComputerLaunchOptions { hidden: Some(true) },
        )
        .await
        .unwrap();
    assert_eq!(
        last_args(&backend, "launchApp"),
        Some(vec![json!("kcalc"), json!([]), json!({ "hidden": true })])
    );
    manager
        .launch_app(
            Some("thread-1"),
            "kcalc",
            &[],
            0,
            ComputerLaunchOptions {
                hidden: Some(false),
            },
        )
        .await
        .unwrap();
    assert_eq!(
        last_args(&backend, "launchApp"),
        Some(vec![json!("kcalc"), json!([]), json!({ "hidden": false })])
    );
    manager.approval_gate().cancel_thread("thread-1", None);
    manager.dispose().await;
}

#[tokio::test]
async fn still_asks_for_the_pane_when_the_agent_drives_the_humans_visible_desktop() {
    let capabilities = ComputerCapabilities {
        visible_desktop: true,
        ..ComputerBackend::capabilities(&FakeComputerBackend::default())
    };
    let Built { manager, .. } = manager(Scripted::new(FakeComputerBackendOptions {
        capabilities: Some(capabilities),
        ..FakeComputerBackendOptions::default()
    }));
    let events = events(&manager);

    manager
        .click(
            Some("thread-1"),
            point(10.0, 10.0),
            &[],
            ComputerClickGesture::default(),
        )
        .await
        .unwrap();
    manager
        .type_text(Some("thread-1"), "hi", None)
        .await
        .unwrap();

    assert_eq!(events.open_pane_requests(), ["thread-1"]);
    manager.dispose().await;
}

#[tokio::test]
async fn leaves_pane_input_unattributed_instead_of_borrowing_a_thread() {
    let Built { manager, .. } = manager(Scripted::default_fake());
    let events = events(&manager);

    manager
        .click(
            None,
            point(10.0, 10.0),
            &[],
            ComputerClickGesture::default(),
        )
        .await
        .unwrap();
    manager
        .launch_app(None, "kcalc", &[], 0, ComputerLaunchOptions::default())
        .await
        .unwrap();
    manager
        .press_key(Some("  "), "enter", None, None)
        .await
        .unwrap();

    let actions = events.actions();
    assert!(actions.iter().all(|event| matches!(
        &event.event,
        ComputerEvent::Action {
            thread_id: None,
            ..
        }
    )));
    assert_eq!(actions.len(), 3);

    manager.dispose().await;
}
