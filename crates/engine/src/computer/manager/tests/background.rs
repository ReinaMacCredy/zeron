use std::sync::atomic::AtomicUsize;

use super::*;
use crate::computer::manager::ComputerStateRequest;
use crate::computer::task_context::with_computer_task;

fn text_area(window_id: &str) -> ComputerTarget {
    ComputerTarget {
        window_id: Some(window_id.to_owned()),
        role: Some("AXTextArea".into()),
        ..ComputerTarget::default()
    }
}

fn task(thread_id: &str, turn_id: Option<&str>) -> CuaComputerTask {
    CuaComputerTask {
        thread_id: thread_id.to_owned(),
        turn_id: turn_id.map(str::to_owned),
        label: None,
    }
}

fn state_of(window_id: &str) -> ComputerStateRequest {
    ComputerStateRequest {
        include_screenshot: None,
        include_text: false,
        include_tree: None,
        window_id: Some(window_id.to_owned()),
    }
}

fn hold_first_press(
    backend: &Scripted,
    started: Arc<Deferred>,
    finish: Arc<Deferred>,
) -> Arc<AtomicUsize> {
    let presses = Arc::new(AtomicUsize::new(0));
    let counter = presses.clone();
    backend.hooks().press_key = Some(Arc::new(move |fake, (key, window, target)| {
        let first = counter.fetch_add(1, Ordering::SeqCst) == 0;
        let started = started.clone();
        let finish = finish.clone();
        async move {
            if first {
                started.resolve();
                finish.wait().await;
                return Ok(ComputerBackendActionResult::default());
            }
            fake.press_key(&key, window.as_deref(), target.as_ref())
                .await
        }
        .boxed()
    }));
    presses
}

#[tokio::test]
async fn types_through_keyboard_focus_when_the_windows_text_field_cannot_be_singled_out() {
    let backend = background_target_backend();
    backend
        .fake
        .set_root(semantic_text_root(&["editor-a", "editor-a"]));
    let targets: Arc<Mutex<Vec<TypedTarget>>> = Arc::default();
    let sink = targets.clone();
    backend.hooks().type_text = Some(Arc::new(move |fake, (text, window, target)| {
        sink.lock().unwrap().push((text.clone(), target.clone()));
        async move {
            fake.type_text(&text, window.as_deref(), target.as_ref())
                .await
        }
        .boxed()
    }));
    let Built {
        manager, backend, ..
    } = settled(backend);

    let result = manager
        .type_text(Some("a"), "hello world", Some("editor-a"))
        .await
        .unwrap();
    assert_eq!(result.action, "computer_type_text");
    assert_eq!(result.window_id.as_deref(), Some("editor-a"));
    let call = targets.lock().unwrap().last().cloned().unwrap();
    assert_eq!(call.0, "hello world");
    assert!(call.1.is_none());

    let type_calls = backend.calls_for("typeText").len();
    let error = manager
        .type_text(Some("a"), "x", Some("gone"))
        .await
        .unwrap_err();
    assert_eq!(code(&error), Some("computer_target_not_found"));
    assert_eq!(backend.calls_for("typeText").len(), type_calls);

    manager.dispose().await;
}

#[tokio::test]
async fn lets_separate_apps_progress_while_protecting_one_apps_keyboard_and_modal_state() {
    let Built {
        manager, backend, ..
    } = manager(background_target_backend());
    manager
        .click(
            Some("a"),
            scoped(20.0, 50.0, "editor-a"),
            &[],
            Default::default(),
        )
        .await
        .unwrap();
    manager
        .click(
            Some("b"),
            scoped(420.0, 50.0, "editor-b"),
            &[],
            Default::default(),
        )
        .await
        .unwrap();
    manager
        .press_key(Some("a"), "enter", Some("editor-a"), None)
        .await
        .unwrap();
    manager
        .press_key(Some("b"), "enter", Some("editor-b"), None)
        .await
        .unwrap();
    assert_eq!(backend.calls_for("pressKey").len(), 2);
    assert!(backend.calls_for("raiseWindow").is_empty());
    let error = manager
        .press_key(Some("b"), "enter", Some("editor-a-other"), None)
        .await
        .unwrap_err();
    assert_eq!(code(&error), Some("computer_controlled_by_other_thread"));
    let state = manager.get_thread_state("b").await;
    assert!(!state.controlled_by_other_thread);
    assert_eq!(state.shared_preview_unavailable, Some(true));
    manager.release_desktop_control("a", None).await.unwrap();
    assert_eq!(
        manager
            .get_thread_state("b")
            .await
            .shared_preview_unavailable,
        None
    );
    manager
        .press_key(Some("b"), "enter", Some("editor-a-other"), None)
        .await
        .unwrap();
    manager.dispose().await;
}

#[tokio::test]
async fn allows_independent_semantic_windows_but_blocks_conflicting_app_wide_input() {
    let Built {
        manager, backend, ..
    } = manager(background_target_backend());
    manager
        .type_text(Some("a"), "alpha", Some("editor-a"))
        .await
        .unwrap();
    manager
        .set_value(Some("b"), text_area("editor-a-other"), "bravo")
        .await
        .unwrap();
    assert!(backend.calls_for("focusWindow").is_empty());
    let error = manager
        .set_value(Some("b"), text_area("editor-a"), "wrong")
        .await
        .unwrap_err();
    assert_eq!(code(&error), Some("computer_controlled_by_other_thread"));
    let error = manager
        .press_key(Some("b"), "enter", Some("editor-a-other"), None)
        .await
        .unwrap_err();
    assert_eq!(code(&error), Some("computer_controlled_by_other_thread"));
    manager.release_desktop_control("a", None).await.unwrap();
    manager
        .press_key(Some("b"), "enter", Some("editor-a-other"), None)
        .await
        .unwrap();
    manager.dispose().await;
}

#[tokio::test]
async fn keeps_foreground_clipboard_and_drags_globally_exclusive() {
    let Built {
        manager, backend, ..
    } = manager(background_target_backend());
    manager
        .press_key(Some("a"), "enter", Some("editor-a"), None)
        .await
        .unwrap();
    let error = manager
        .write_clipboard(Some("b"), "clipboard")
        .await
        .unwrap_err();
    assert_eq!(code(&error), Some("computer_controlled_by_other_thread"));
    let error = manager
        .drag(
            Some("b"),
            scoped(420.0, 50.0, "editor-b"),
            scoped(440.0, 60.0, "editor-b"),
            None,
        )
        .await
        .unwrap_err();
    assert_eq!(code(&error), Some("computer_controlled_by_other_thread"));
    let error = manager
        .activate_window(Some("b"), "editor-b", visible_use())
        .await
        .unwrap_err();
    assert_eq!(code(&error), Some("computer_controlled_by_other_thread"));
    assert!(backend.calls_for("drag").is_empty());
    assert!(backend.calls_for("writeClipboard").is_empty());
    assert!(backend.calls_for("raiseWindow").is_empty());
    manager.dispose().await;
}

#[tokio::test]
async fn protects_a_running_app_from_another_tasks_launch_alias_and_permits_other_apps() {
    let Built {
        manager, backend, ..
    } = manager(background_target_backend());
    manager
        .press_key(Some("a"), "enter", Some("editor-a"), None)
        .await
        .unwrap();
    let error = manager
        .launch_app(
            Some("b"),
            "app.editor.a",
            &[],
            0,
            ComputerLaunchOptions::default(),
        )
        .await
        .unwrap_err();
    assert_eq!(code(&error), Some("computer_controlled_by_other_thread"));
    manager
        .launch_app(
            Some("b"),
            "Editor B",
            &[],
            0,
            ComputerLaunchOptions::default(),
        )
        .await
        .unwrap();
    assert_eq!(backend.calls_for("launchApp").len(), 1);
    manager.dispose().await;
}

#[tokio::test]
async fn does_not_release_a_newer_background_turn_after_a_late_old_completion() {
    let Built { manager, .. } = manager(background_target_backend());
    manager
        .with_agent_activity(
            "a",
            || manager.press_key(Some("a"), "enter", Some("editor-a"), None),
            None,
            Some("old"),
            None,
        )
        .await
        .unwrap();
    manager
        .release_desktop_control("a", Some("old"))
        .await
        .unwrap();
    manager
        .with_agent_activity(
            "a",
            || manager.press_key(Some("a"), "enter", Some("editor-a"), None),
            None,
            Some("new"),
            None,
        )
        .await
        .unwrap();
    manager
        .release_desktop_control("a", Some("old"))
        .await
        .unwrap();
    let error = manager
        .press_key(Some("b"), "enter", Some("editor-a"), None)
        .await
        .unwrap_err();
    assert_eq!(code(&error), Some("computer_controlled_by_other_thread"));
    manager
        .release_desktop_control("a", Some("new"))
        .await
        .unwrap();
    manager
        .press_key(Some("b"), "enter", Some("editor-a"), None)
        .await
        .unwrap();
    manager.dispose().await;
}

#[tokio::test]
async fn does_not_inherit_an_evicted_background_turn_on_a_later_anonymous_claim() {
    let (now, clock_fn) = clock(0);
    let Built { manager, .. } = build(
        background_target_backend(),
        Setup {
            now: Some(clock_fn),
            lease_idle_ms: Some(100),
            ..Setup::default()
        },
    );
    manager
        .with_agent_activity(
            "a",
            || manager.press_key(Some("a"), "enter", Some("editor-a"), None),
            None,
            Some("old-a"),
            None,
        )
        .await
        .unwrap();
    *now.lock().unwrap() = 200;
    manager
        .with_agent_activity(
            "b",
            || manager.press_key(Some("b"), "enter", Some("editor-a"), None),
            None,
            Some("turn-b"),
            None,
        )
        .await
        .unwrap();
    manager
        .release_desktop_control("b", Some("turn-b"))
        .await
        .unwrap();
    manager
        .with_agent_activity(
            "a",
            || manager.press_key(Some("a"), "enter", Some("editor-a"), None),
            None,
            None,
            None,
        )
        .await
        .unwrap();
    manager
        .release_desktop_control("a", Some("current-a"))
        .await
        .unwrap();
    manager
        .press_key(Some("b"), "enter", Some("editor-a"), None)
        .await
        .unwrap();
    manager.dispose().await;
}

#[tokio::test]
async fn releases_a_completed_background_turn_only_after_its_admitted_input_drains() {
    let backend = background_target_backend();
    let started = deferred();
    let finish = deferred();
    let presses = hold_first_press(&backend, started.clone(), finish.clone());
    let Built { manager, .. } = manager(backend);
    let active = tokio::spawn({
        let manager = manager.clone();
        async move {
            manager
                .with_agent_activity(
                    "a",
                    || manager.press_key(Some("a"), "enter", Some("editor-a"), None),
                    None,
                    Some("turn-a"),
                    None,
                )
                .await
        }
    });
    started.wait().await;
    manager
        .release_desktop_control("a", Some("turn-a"))
        .await
        .unwrap();
    let next = tokio::spawn({
        let manager = manager.clone();
        async move {
            manager
                .with_agent_activity(
                    "b",
                    || manager.press_key(Some("b"), "enter", Some("editor-a"), None),
                    None,
                    Some("turn-b"),
                    None,
                )
                .await
        }
    });
    settle().await;
    assert_eq!(presses.load(Ordering::SeqCst), 1);
    finish.resolve();
    active.await.unwrap().unwrap();
    next.await.unwrap().unwrap();
    assert_eq!(presses.load(Ordering::SeqCst), 2);
    manager.dispose().await;
}

#[tokio::test]
async fn revokes_a_queued_task_without_stopping_the_other_apps_active_input() {
    let backend = background_target_backend();
    let started = deferred();
    let finish = deferred();
    backend.hooks().stop_input = Some(Arc::new(|_, _| async { Ok(()) }.boxed()));
    let presses = hold_first_press(&backend, started.clone(), finish.clone());
    let Built {
        manager, backend, ..
    } = manager(backend);
    let active = tokio::spawn({
        let manager = manager.clone();
        async move {
            manager
                .with_agent_activity(
                    "b",
                    || manager.press_key(Some("b"), "enter", Some("editor-b"), None),
                    None,
                    Some("turn-b"),
                    None,
                )
                .await
        }
    });
    started.wait().await;
    let queued = tokio::spawn({
        let manager = manager.clone();
        async move {
            manager
                .with_agent_activity(
                    "a",
                    || manager.press_key(Some("a"), "enter", Some("editor-a"), None),
                    None,
                    Some("turn-a"),
                    None,
                )
                .await
        }
    });
    settle().await;
    manager.stop_thread("a").await.unwrap();
    assert!(backend.logged("stopInput").is_empty());
    finish.resolve();
    active.await.unwrap().unwrap();
    let error = queued.await.unwrap().unwrap_err();
    assert!(error.control_revoked());
    assert_eq!(presses.load(Ordering::SeqCst), 1);
    manager.dispose().await;
}

#[tokio::test]
async fn forwards_exact_key_targets_without_focus_changes_and_rejects_window_mismatches() {
    let Built {
        manager, backend, ..
    } = manager(background_target_backend());
    manager
        .press_key(
            Some("a"),
            "enter",
            Some("editor-a"),
            Some(text_area("editor-a").into()),
        )
        .await
        .unwrap();
    let presses = backend.logged("pressKey");
    assert_eq!(presses.len(), 1);
    assert_eq!(presses[0]["key"], "enter");
    assert_eq!(presses[0]["windowId"], "editor-a");
    assert_eq!(presses[0]["target"]["windowId"], "editor-a");
    assert!(backend.calls_for("focusWindow").is_empty());
    let error = manager
        .press_key(
            Some("a"),
            "enter",
            Some("editor-a"),
            Some(text_area("editor-b").into()),
        )
        .await
        .unwrap_err();
    assert_eq!(code(&error), Some("computer_target_invalid"));
    assert_eq!(backend.logged("pressKey").len(), 1);
    manager.dispose().await;
}

#[tokio::test]
async fn pauses_after_observed_launch_activation_without_losing_the_launch_outcome_or_replaying() {
    let backend = background_target_backend();
    let first = backend.fake.list_windows().await.unwrap()[0].clone();
    let computer_id = backend.computer_id().to_owned();
    backend.hooks().launch_app = Some(Arc::new(move |_, _| {
        let result = ComputerLaunchAppResult {
            computer_id: computer_id.clone(),
            app: "Editor A".into(),
            focus_changed_during_launch: Some(true),
            pid: Some(110),
            window_status: None,
            window_reason: None,
            resolved_command: None,
            window: Some(first.clone()),
        };
        async move { Ok(result) }.boxed()
    }));
    backend.hooks().check_input_ready = Some(Arc::new(|_, _| async { Ok(()) }.boxed()));
    let Built {
        manager, backend, ..
    } = manager(backend);
    let result = manager
        .launch_app(
            Some("a"),
            "Editor A",
            &[],
            0,
            ComputerLaunchOptions::default(),
        )
        .await
        .unwrap();
    assert_eq!(result.focus_changed_during_launch, Some(true));
    let error = manager
        .press_key(Some("a"), "enter", Some("editor-a"), None)
        .await
        .unwrap_err();
    assert!(error.input_pause().is_some());
    assert_eq!(backend.logged("launchApp").len(), 1);
    with_computer_task(task("a", None), manager.get_state(state_of("editor-a")))
        .await
        .unwrap();
    manager
        .press_key(Some("a"), "enter", Some("editor-a"), None)
        .await
        .unwrap();
    manager.dispose().await;
}

#[tokio::test]
async fn retains_a_bounded_list_of_previously_observed_app_names_without_probing() {
    let Built {
        manager, backend, ..
    } = manager(background_target_backend());
    manager.list_windows().await.unwrap();
    backend.fake.emit_windows_changed(Vec::new());
    let reads = backend.calls_for("listWindows").len();
    assert_eq!(manager.observed_app_names(), vec!["Editor B", "Editor A"]);
    assert_eq!(backend.calls_for("listWindows").len(), reads);
    manager.dispose().await;
}
