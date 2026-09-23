use super::*;

use std::time::Duration;

use zeron_proto::computer::CuaEffect;

use super::super::{COMPUTER_CONTROL_TEARDOWN_TIMEOUT_MS, ComputerClickGesture};
use crate::computer::backend::{ComputerBackendError, ComputerMenuTarget};
use crate::computer::call_context::{
    ComputerCallContext, ComputerCallTiming, with_computer_call_context,
};
use crate::computer::fake_backend::FakeComputerOptIn;
use crate::computer::visible_use::ComputerForegroundAuthorization;

fn raised_ids(backend: &Scripted) -> Vec<String> {
    backend
        .calls_for("raiseWindow")
        .into_iter()
        .map(|call| call.args[0].as_str().unwrap().to_owned())
        .collect()
}

fn action_name(event: &ComputerManagerEvent) -> Option<String> {
    match event {
        ComputerEvent::Action { action, .. } => Some(action.clone()),
        _ => None,
    }
}

fn action_window(event: &ComputerManagerEvent) -> Option<String> {
    match event {
        ComputerEvent::Action { window_id, .. } => window_id.clone(),
        _ => None,
    }
}

fn action_message(event: &ComputerManagerEvent) -> Option<String> {
    match event {
        ComputerEvent::Action { message, .. } => message.clone(),
        _ => None,
    }
}

fn restore_status(event: &ComputerManagerEvent) -> Option<&'static str> {
    match event {
        ComputerEvent::Action { restore_status, .. } => {
            restore_status.map(|status| status.as_str())
        }
        _ => None,
    }
}

fn restored_window(event: &ComputerManagerEvent) -> Option<String> {
    match event {
        ComputerEvent::Action {
            restored_window_id, ..
        } => restored_window_id.clone(),
        _ => None,
    }
}

fn masked_action(event: &ComputerManagerEvent) -> bool {
    matches!(event, ComputerEvent::Action { masked: true, .. })
}

fn not_dispatched(error: &ComputerError, expected: &str) {
    assert_eq!(code(error), Some(expected), "{error}");
    assert_eq!(error.effect(), Some(CuaEffect::NotDispatched));
}

#[tokio::test]
async fn refuses_route_before_starting_the_backend_or_acquiring_the_desktop_lease() {
    for route in [
        "activate",
        "activate-and-restore",
        "foreground-input",
        "menu",
    ] {
        let Built {
            manager, backend, ..
        } = settled(Scripted::default_fake());
        let input_called = Arc::new(AtomicBool::new(false));
        let actions = events(&manager);
        let thread = Some("refused-thread");
        let error = match route {
            "activate" => manager
                .activate_window(thread, "fake-calculator", None)
                .await
                .map(|_| ())
                .unwrap_err(),
            "activate-and-restore" => manager
                .foreground_with_restore(thread, "fake-calculator", None, None)
                .await
                .map(|_| ())
                .unwrap_err(),
            "menu" => manager
                .invoke_menu(
                    thread,
                    ComputerMenuTarget::Window("fake-calculator".into()),
                    &["File".to_owned()],
                    None,
                )
                .await
                .map(|_| ())
                .unwrap_err(),
            _ => {
                let flag = input_called.clone();
                manager
                    .with_foreground_restore(
                        thread,
                        || async move {
                            flag.store(true, Ordering::SeqCst);
                            Ok("typed")
                        },
                        None,
                    )
                    .await
                    .map(|_| ())
                    .unwrap_err()
            }
        };
        not_dispatched(&error, "foreground_not_requested");
        assert!(
            backend.methods().is_empty(),
            "{route}: {:?}",
            backend.methods()
        );
        assert!(!input_called.load(Ordering::SeqCst), "{route}");
        assert!(actions.actions().is_empty(), "{route}");
        let clicked = manager
            .click(
                Some("other-thread"),
                point(100.0, 100.0),
                &[],
                ComputerClickGesture::default(),
            )
            .await
            .unwrap();
        assert_eq!(clicked.action, "computer_click", "{route}");
        manager.dispose().await;
    }
}

#[tokio::test]
async fn refuses_an_activate_with_no_task_text_authorization_and_raises_nothing() {
    let Built {
        manager, backend, ..
    } = settled(Scripted::default_fake());
    let actions = events(&manager);
    let refused = manager
        .foreground_with_restore(Some("thread-1"), "fake-calculator", None, None)
        .await
        .unwrap_err();
    not_dispatched(&refused, "foreground_not_requested");
    assert!(raised_ids(&backend).is_empty());
    assert!(backend.calls_for("focusWindow").is_empty());
    assert!(actions.actions().is_empty());
    manager.dispose().await;
}

#[tokio::test]
async fn refuses_an_explicit_false_authorization_the_same_way() {
    let Built {
        manager, backend, ..
    } = settled(Scripted::default_fake());
    let refused = manager
        .foreground_with_restore(
            Some("thread-1"),
            "fake-calculator",
            None,
            Some(ComputerForegroundAuthorization {
                user_requested_visible_use: false,
            }),
        )
        .await
        .unwrap_err();
    assert_eq!(code(&refused), Some("foreground_not_requested"));
    assert!(raised_ids(&backend).is_empty());
    manager.dispose().await;
}

#[tokio::test]
async fn refuses_a_foreground_call_and_a_plain_activate_without_authorization() {
    let Built {
        manager, backend, ..
    } = settled(Scripted::default_fake());
    let refused = manager
        .with_foreground_restore(Some("thread-1"), || async { Ok("typed") }, None)
        .await
        .unwrap_err();
    assert_eq!(code(&refused), Some("foreground_not_requested"));
    let refused = manager
        .activate_window(Some("thread-1"), "fake-calculator", None)
        .await
        .unwrap_err();
    assert_eq!(code(&refused), Some("foreground_not_requested"));
    assert!(raised_ids(&backend).is_empty());
    manager.dispose().await;
}

#[tokio::test]
async fn refuses_the_raise_while_the_user_was_just_interacting_through_the_pane_then_allows_it_after_quiet()
 {
    let (now, clock_fn) = clock(1_000_000);
    let Built {
        manager, backend, ..
    } = build(
        Scripted::default_fake(),
        Setup {
            action_settle_ms: Some(0),
            now: Some(clock_fn),
            ..Setup::default()
        },
    );
    manager
        .click(
            None,
            point(100.0, 100.0),
            &[],
            ComputerClickGesture::default(),
        )
        .await
        .unwrap();
    let refused = manager
        .foreground_with_restore(Some("thread-1"), "fake-calculator", None, visible_use())
        .await
        .unwrap_err();
    not_dispatched(&refused, "foreground_user_interaction");
    assert!(raised_ids(&backend).is_empty());
    *now.lock().unwrap() += 2_001;
    let result = manager
        .foreground_with_restore(Some("thread-1"), "fake-calculator", None, visible_use())
        .await
        .unwrap();
    assert_eq!(result.result.window_id.as_deref(), Some("fake-calculator"));
    assert_eq!(raised_ids(&backend), ["fake-calculator", "fake-terminal"]);
    manager.dispose().await;
}

#[tokio::test]
async fn does_not_let_a_pane_raise_be_blocked_by_the_panes_own_interaction_stamp() {
    let Built {
        manager, backend, ..
    } = settled(Scripted::default_fake());
    manager
        .click(
            None,
            point(100.0, 100.0),
            &[],
            ComputerClickGesture::default(),
        )
        .await
        .unwrap();
    let result = manager
        .activate_window(None, "fake-calculator", None)
        .await
        .unwrap();
    assert_eq!(result.window_id.as_deref(), Some("fake-calculator"));
    assert_eq!(raised_ids(&backend), ["fake-calculator"]);
    manager.dispose().await;
}

#[tokio::test]
async fn restores_the_previously_frontmost_window_after_raising_the_target() {
    let Built {
        manager, backend, ..
    } = settled(Scripted::default_fake());
    let events = events(&manager);
    let result = manager
        .foreground_with_restore(Some("thread-1"), "fake-calculator", None, visible_use())
        .await
        .unwrap();
    assert_eq!(result.result.window_id.as_deref(), Some("fake-calculator"));
    assert_eq!(result.note, None);
    assert_eq!(raised_ids(&backend), ["fake-calculator", "fake-terminal"]);
    let actions = events.actions();
    assert_eq!(actions.len(), 1);
    assert_eq!(
        action_name(&actions[0]).as_deref(),
        Some("computer_activate_window")
    );
    assert_eq!(
        action_window(&actions[0]).as_deref(),
        Some("fake-calculator")
    );
    assert_eq!(
        restored_window(&actions[0]).as_deref(),
        Some("fake-terminal")
    );
    assert_eq!(restore_status(&actions[0]), Some("restored"));
    assert_eq!(action_message(&actions[0]), None);
    manager.dispose().await;
}

#[tokio::test]
async fn counts_one_foreground_excursion_on_the_call_timing_record() {
    let Built { manager, .. } = settled(Scripted::default_fake());
    let context = Arc::new(ComputerCallContext::new(Some(ComputerCallTiming::new(
        Arc::new(|| 0.0),
    ))));
    with_computer_call_context(
        context.clone(),
        manager.foreground_with_restore(Some("thread-1"), "fake-calculator", None, visible_use()),
    )
    .await
    .unwrap();
    let line = context.timing.as_ref().unwrap().finish().unwrap();
    assert!(line.contains("foreground_excursion=1"), "{line}");
    manager.dispose().await;
}

#[tokio::test]
async fn reports_a_missed_restore_as_success_with_a_note_naming_the_unrestored_window() {
    let backend = Scripted::default_fake();
    let attempts: Arc<Mutex<Vec<String>>> = Arc::default();
    let log = attempts.clone();
    backend.hooks().raise_window = Some(Arc::new(move |fake, window_id: String| {
        log.lock().unwrap().push(window_id.clone());
        async move {
            if window_id == "fake-terminal" {
                return Err(ComputerBackendError::new("The window closed.").into());
            }
            fake.raise_window(&window_id).await
        }
        .boxed()
    }));
    let Built { manager, .. } = settled(backend);
    let events = events(&manager);
    let result = manager
        .foreground_with_restore(Some("thread-1"), "fake-calculator", None, visible_use())
        .await
        .unwrap();
    assert_eq!(result.result.window_id.as_deref(), Some("fake-calculator"));
    assert!(result.note.as_deref().unwrap().contains("fake-terminal"));
    assert_eq!(
        *attempts.lock().unwrap(),
        ["fake-calculator", "fake-terminal"]
    );
    let actions = events.actions();
    assert_eq!(actions.len(), 1);
    assert_eq!(restore_status(&actions[0]), Some("restore-missed"));
    assert_eq!(
        restored_window(&actions[0]).as_deref(),
        Some("fake-terminal")
    );
    assert!(
        action_message(&actions[0])
            .unwrap()
            .contains("fake-terminal")
    );
    manager.dispose().await;
}

#[tokio::test]
async fn runs_the_approved_input_between_raise_and_restore_without_a_second_approval() {
    let backend = Scripted::default_fake();
    let order: Arc<Mutex<Vec<String>>> = Arc::default();
    let log = order.clone();
    backend.hooks().raise_window = Some(Arc::new(move |fake, window_id: String| {
        log.lock().unwrap().push(format!("raise:{window_id}"));
        async move { fake.raise_window(&window_id).await }.boxed()
    }));
    let Built { manager, seams, .. } = settled(backend);
    let input_log = order.clone();
    manager
        .foreground_with_restore(
            Some("thread-1"),
            "fake-calculator",
            Some(Box::pin(async move {
                input_log.lock().unwrap().push("input".into());
                Ok(())
            })),
            visible_use(),
        )
        .await
        .unwrap();
    assert_eq!(
        *order.lock().unwrap(),
        ["raise:fake-calculator", "input", "raise:fake-terminal"]
    );
    assert!(seams.consent.lock().unwrap().is_empty());
    manager.approval_gate().cancel_thread("thread-1", None);
    manager.dispose().await;
}

#[tokio::test]
async fn still_restores_when_the_approved_input_fails_then_reports_the_input_failure() {
    let Built {
        manager, backend, ..
    } = settled(Scripted::default_fake());
    let events = events(&manager);
    let error = manager
        .foreground_with_restore(
            Some("thread-1"),
            "fake-calculator",
            Some(Box::pin(async {
                Err(ComputerError::Other("input blew up".into()))
            })),
            visible_use(),
        )
        .await
        .unwrap_err();
    assert!(error.to_string().contains("input blew up"), "{error}");
    assert_eq!(raised_ids(&backend), ["fake-calculator", "fake-terminal"]);
    assert!(events.actions().is_empty());
    manager.dispose().await;
}

#[tokio::test]
async fn skips_the_restore_when_the_target_is_already_frontmost() {
    let Built {
        manager, backend, ..
    } = settled(Scripted::default_fake());
    let events = events(&manager);
    let result = manager
        .foreground_with_restore(Some("thread-1"), "fake-terminal", None, visible_use())
        .await
        .unwrap();
    assert_eq!(result.result.window_id.as_deref(), Some("fake-terminal"));
    assert_eq!(result.note, None);
    assert_eq!(raised_ids(&backend), ["fake-terminal"]);
    let actions = events.actions();
    assert_eq!(actions.len(), 1);
    assert_eq!(restore_status(&actions[0]), Some("already-frontmost"));
    assert_eq!(restored_window(&actions[0]), None);
    manager.dispose().await;
}

#[tokio::test]
async fn notes_when_no_frontmost_window_was_observable_and_restores_nothing() {
    let hidden = vec![
        ComputerWindow {
            app_name: Some("org.kde.konsole".into()),
            minimized: true,
            visible: false,
            ..window(
                "hidden-terminal",
                "Terminal",
                Some(rect(40.0, 40.0, 960.0, 720.0)),
            )
        },
        ComputerWindow {
            app_name: Some("org.kde.kcalc".into()),
            minimized: true,
            visible: false,
            ..window(
                "hidden-calculator",
                "Calculator",
                Some(rect(1_050.0, 120.0, 420.0, 620.0)),
            )
        },
    ];
    let Built {
        manager, backend, ..
    } = settled(Scripted::new(FakeComputerBackendOptions {
        windows: Some(hidden),
        ..FakeComputerBackendOptions::default()
    }));
    let events = events(&manager);
    let result = manager
        .foreground_with_restore(Some("thread-1"), "hidden-calculator", None, visible_use())
        .await
        .unwrap();
    assert_eq!(
        result.result.window_id.as_deref(),
        Some("hidden-calculator")
    );
    assert!(
        result
            .note
            .as_deref()
            .unwrap()
            .contains("nothing was restored")
    );
    assert_eq!(raised_ids(&backend), ["hidden-calculator"]);
    let actions = events.actions();
    assert_eq!(actions.len(), 1);
    assert_eq!(restore_status(&actions[0]), Some("frontmost-unobservable"));
    assert_eq!(restored_window(&actions[0]), None);
    manager.dispose().await;
}

fn terminal_first() -> Vec<ComputerWindow> {
    vec![
        ComputerWindow {
            app_name: Some("org.kde.konsole".into()),
            focused: true,
            ..window(
                "fake-terminal",
                "Terminal",
                Some(rect(40.0, 40.0, 960.0, 720.0)),
            )
        },
        ComputerWindow {
            app_name: Some("org.kde.kcalc".into()),
            ..window(
                "fake-calculator",
                "Calculator",
                Some(rect(1_050.0, 120.0, 420.0, 620.0)),
            )
        },
    ]
}

fn calculator_first() -> Vec<ComputerWindow> {
    let mut windows = terminal_first();
    windows.reverse();
    windows
}

type Listing = Arc<Mutex<Vec<ComputerWindow>>>;

fn scripted_listing(backend: &Scripted, listing: &Listing) {
    let listing = listing.clone();
    backend.hooks().list_windows = Some(Arc::new(move |_, ()| {
        let current = listing.lock().unwrap().clone();
        async move { Ok(current) }.boxed()
    }));
}

fn focused_ids(backend: &Scripted) -> Vec<String> {
    backend
        .calls_for("focusWindow")
        .into_iter()
        .map(|call| call.args[0].as_str().unwrap().to_owned())
        .collect()
}

#[tokio::test]
async fn puts_the_users_window_back_after_a_foreground_call_moves_focus() {
    let backend = Scripted::default_fake();
    let listing: Listing = Arc::new(Mutex::new(terminal_first()));
    scripted_listing(&backend, &listing);
    let Built {
        manager, backend, ..
    } = settled(backend);
    let moved = listing.clone();
    let result = manager
        .with_foreground_restore(
            Some("thread-1"),
            || async move {
                *moved.lock().unwrap() = calculator_first();
                Ok("typed")
            },
            visible_use(),
        )
        .await
        .unwrap();
    assert_eq!(result, "typed");
    assert_eq!(raised_ids(&backend), ["fake-terminal"]);
    assert_eq!(focused_ids(&backend), ["fake-terminal"]);
    manager.dispose().await;
}

#[tokio::test]
async fn skips_the_raise_when_the_foreground_call_never_moved_focus() {
    let backend = Scripted::default_fake();
    let listing: Listing = Arc::new(Mutex::new(terminal_first()));
    scripted_listing(&backend, &listing);
    let Built {
        manager, backend, ..
    } = settled(backend);
    manager
        .with_foreground_restore(Some("thread-1"), || async { Ok("typed") }, visible_use())
        .await
        .unwrap();
    assert!(raised_ids(&backend).is_empty());
    assert!(backend.calls_for("focusWindow").is_empty());
    manager.dispose().await;
}

#[tokio::test]
async fn still_restores_when_the_wrapped_call_fails_then_reports_the_failure() {
    let backend = Scripted::default_fake();
    let listing: Listing = Arc::new(Mutex::new(terminal_first()));
    scripted_listing(&backend, &listing);
    let Built {
        manager, backend, ..
    } = settled(backend);
    let moved = listing.clone();
    let error = manager
        .with_foreground_restore(
            Some("thread-1"),
            || async move {
                *moved.lock().unwrap() = calculator_first();
                Err::<(), _>(ComputerError::Other("input blew up".into()))
            },
            visible_use(),
        )
        .await
        .unwrap_err();
    assert!(error.to_string().contains("input blew up"), "{error}");
    assert_eq!(raised_ids(&backend), ["fake-terminal"]);
    manager.dispose().await;
}

#[tokio::test]
async fn restores_nothing_when_no_frontmost_window_was_observable() {
    let backend = Scripted::default_fake();
    let listing: Listing = Arc::new(Mutex::new(Vec::new()));
    scripted_listing(&backend, &listing);
    let Built {
        manager, backend, ..
    } = settled(backend);
    manager
        .with_foreground_restore(Some("thread-1"), || async { Ok("typed") }, visible_use())
        .await
        .unwrap();
    assert!(raised_ids(&backend).is_empty());
    manager.dispose().await;
}

#[tokio::test]
async fn warns_when_the_post_call_read_fails_and_the_restore_cannot_run_blind() {
    let backend = Scripted::default_fake();
    let action_done = Arc::new(AtomicBool::new(false));
    let done = action_done.clone();
    backend.hooks().list_windows = Some(Arc::new(move |_, ()| {
        let done = done.load(Ordering::SeqCst);
        async move {
            if !done {
                return Ok(terminal_first());
            }
            Err(ComputerError::Other("listing wedged".into()))
        }
        .boxed()
    }));
    let Built {
        manager,
        backend,
        seams,
    } = settled(backend);
    let flag = action_done.clone();
    manager
        .with_foreground_restore(
            Some("thread-1"),
            || async move {
                flag.store(true, Ordering::SeqCst);
                Ok("typed")
            },
            visible_use(),
        )
        .await
        .unwrap();
    let warnings = seams.warnings.lock().unwrap().clone();
    assert!(
        warnings.contains(&(
            "[computer] foreground call left focus unverified".to_owned(),
            json!({ "previousWindowId": "fake-terminal" }),
        )),
        "{warnings:?}"
    );
    assert!(raised_ids(&backend).is_empty());
    manager.dispose().await;
}

fn masked(backend: Arc<Scripted>, apps: &str) -> Built {
    build(
        backend,
        Setup {
            action_settle_ms: Some(0),
            env: ComputerManagerEnv {
                masked_activation: true,
                masked_apps: [apps].into_iter().map(String::from).collect(),
                ..ComputerManagerEnv::default()
            },
            ..Setup::default()
        },
    )
}

fn shielded_mac_backend(options: FakeComputerBackendOptions) -> Arc<Scripted> {
    Scripted::new(FakeComputerBackendOptions {
        agent_dialect: Some(ComputerAgentDialect::Macos),
        shield: FakeComputerOptIn::BuiltIn,
        ..options
    })
}

fn shield_excursion_order(backend: &Scripted) -> Vec<String> {
    backend
        .fake
        .calls()
        .into_iter()
        .filter(|call| {
            ["engageShield", "raiseWindow", "releaseShield"].contains(&call.method.as_str())
        })
        .map(|call| {
            if call.method == "raiseWindow" {
                format!("{}:{}", call.method, call.args[0].as_str().unwrap())
            } else {
                call.method
            }
        })
        .collect()
}

#[tokio::test]
async fn shields_the_opted_in_windows_raise_and_releases_the_mask_after_the_restore() {
    let Built {
        manager, backend, ..
    } = masked(
        shielded_mac_backend(FakeComputerBackendOptions::default()),
        "org.kde.kcalc",
    );
    let events = events(&manager);
    let result = manager
        .foreground_with_restore(Some("thread-1"), "fake-calculator", None, visible_use())
        .await
        .unwrap();
    assert_eq!(result.result.window_id.as_deref(), Some("fake-calculator"));
    assert_eq!(
        shield_excursion_order(&backend),
        [
            "engageShield",
            "raiseWindow:fake-calculator",
            "raiseWindow:fake-terminal",
            "releaseShield",
        ]
    );
    let engage = backend.calls_for("engageShield").remove(0).args.remove(0);
    assert_eq!(engage["windowId"], json!("fake-calculator"));
    assert_eq!(
        engage["frame"],
        json!({ "x": 1_050.0, "y": 120.0, "width": 420.0, "height": 620.0 })
    );
    assert_eq!(engage["label"], json!("Zeron is activating Calculator"));
    let shield_id = engage["shieldId"].as_str().unwrap().to_owned();
    assert!(
        shield_id.len() == 15
            && shield_id.starts_with("shield-")
            && shield_id[7..]
                .chars()
                .all(|c| c.is_ascii_digit() || ('a'..='f').contains(&c)),
        "{shield_id}"
    );
    let releases: Vec<Value> = backend
        .calls_for("releaseShield")
        .into_iter()
        .map(|call| call.args[0].clone())
        .collect();
    assert_eq!(releases, [json!(shield_id)]);
    assert!(backend.fake.active_shields().is_empty());
    let actions = events.actions();
    assert_eq!(actions.len(), 1);
    assert_eq!(
        action_name(&actions[0]).as_deref(),
        Some("computer_activate_window")
    );
    assert!(masked_action(&actions[0]));
    assert_eq!(restore_status(&actions[0]), Some("restored"));
    manager.dispose().await;
}

#[tokio::test]
async fn masks_nothing_while_the_canary_flag_is_unset_even_with_a_shield_surface() {
    let Built {
        manager, backend, ..
    } = settled(shielded_mac_backend(FakeComputerBackendOptions::default()));
    let events = events(&manager);
    manager
        .foreground_with_restore(Some("thread-1"), "fake-calculator", None, visible_use())
        .await
        .unwrap();
    assert!(backend.calls_for("engageShield").is_empty());
    assert!(!masked_action(&events.actions()[0]));
    manager.dispose().await;
}

#[tokio::test]
async fn masks_nothing_when_the_opt_in_list_names_a_different_app() {
    let Built {
        manager, backend, ..
    } = masked(
        shielded_mac_backend(FakeComputerBackendOptions::default()),
        "com.example.other",
    );
    manager
        .foreground_with_restore(Some("thread-1"), "fake-calculator", None, visible_use())
        .await
        .unwrap();
    assert!(backend.calls_for("engageShield").is_empty());
    manager.dispose().await;
}

#[tokio::test]
async fn masks_nothing_on_a_non_macos_dialect_even_when_armed_and_opted_in() {
    let Built {
        manager, backend, ..
    } = masked(
        Scripted::new(FakeComputerBackendOptions {
            shield: FakeComputerOptIn::BuiltIn,
            ..FakeComputerBackendOptions::default()
        }),
        "org.kde.kcalc",
    );
    manager
        .foreground_with_restore(Some("thread-1"), "fake-calculator", None, visible_use())
        .await
        .unwrap();
    assert!(backend.calls_for("engageShield").is_empty());
    manager.dispose().await;
}

#[tokio::test]
async fn resolves_the_opt_in_through_the_owning_apps_bundle_id_not_the_window_title() {
    let windows = vec![
        ComputerWindow {
            app_name: Some("Terminal".into()),
            pid: Some(1_001),
            focused: true,
            ..window(
                "fake-terminal",
                "Terminal",
                Some(rect(40.0, 40.0, 960.0, 720.0)),
            )
        },
        ComputerWindow {
            app_name: Some("Calculator".into()),
            pid: Some(1_002),
            ..window(
                "fake-calculator",
                "Calculator",
                Some(rect(1_050.0, 120.0, 420.0, 620.0)),
            )
        },
    ];
    let apps = vec![
        ComputerApp {
            window_count: Some(1),
            ..app(1_001, "Terminal", "org.kde.konsole", true)
        },
        ComputerApp {
            window_count: Some(1),
            ..app(1_002, "Calculator", "org.kde.kcalc", false)
        },
    ];
    let Built {
        manager, backend, ..
    } = masked(
        shielded_mac_backend(FakeComputerBackendOptions {
            windows: Some(windows),
            apps: Some(apps),
            ..FakeComputerBackendOptions::default()
        }),
        "org.kde.kcalc",
    );
    manager
        .foreground_with_restore(Some("thread-1"), "fake-calculator", None, visible_use())
        .await
        .unwrap();
    assert_eq!(backend.calls_for("engageShield").len(), 1);
    manager.dispose().await;
}

#[tokio::test]
async fn refuses_the_activation_when_the_opt_in_is_armed_but_no_shield_surface_exists() {
    let Built {
        manager, backend, ..
    } = masked(
        Scripted::new(FakeComputerBackendOptions {
            agent_dialect: Some(ComputerAgentDialect::Macos),
            ..FakeComputerBackendOptions::default()
        }),
        "org.kde.kcalc",
    );
    let error = manager
        .foreground_with_restore(Some("thread-1"), "fake-calculator", None, visible_use())
        .await
        .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("activation shield is unavailable"),
        "{error}"
    );
    assert!(raised_ids(&backend).is_empty());
    manager.dispose().await;
}

#[tokio::test]
async fn refuses_the_activation_when_the_shield_cannot_engage_and_releases_the_minted_id() {
    let backend = shielded_mac_backend(FakeComputerBackendOptions::default());
    backend.fake.fail_next_with(
        "engageShield",
        ComputerBackendError::new("mask_unavailable"),
    );
    let Built {
        manager, backend, ..
    } = masked(backend, "org.kde.kcalc");
    let error = manager
        .foreground_with_restore(Some("thread-1"), "fake-calculator", None, visible_use())
        .await
        .unwrap_err();
    assert!(error.to_string().contains("mask_unavailable"), "{error}");
    let engages = backend.calls_for("engageShield");
    let releases = backend.calls_for("releaseShield");
    assert_eq!(engages.len(), 1);
    assert_eq!(
        releases
            .iter()
            .map(|call| call.args[0].clone())
            .collect::<Vec<_>>(),
        [engages[0].args[0]["shieldId"].clone()]
    );
    assert!(raised_ids(&backend).is_empty());
    assert!(backend.fake.active_shields().is_empty());
    manager.dispose().await;
}

#[tokio::test]
async fn drops_the_shield_when_the_masked_excursion_itself_fails() {
    let backend = shielded_mac_backend(FakeComputerBackendOptions::default());
    backend.hooks().raise_window = Some(Arc::new(|_, _| {
        async { Err(ComputerBackendError::new("The window closed.").into()) }.boxed()
    }));
    let Built {
        manager, backend, ..
    } = masked(backend, "org.kde.kcalc");
    let error = manager
        .foreground_with_restore(Some("thread-1"), "fake-calculator", None, visible_use())
        .await
        .unwrap_err();
    assert!(error.to_string().contains("window closed"), "{error}");
    assert_eq!(backend.calls_for("releaseShield").len(), 1);
    assert!(backend.fake.active_shields().is_empty());
    manager.dispose().await;
}

#[tokio::test]
async fn an_action_resolving_after_thread_removal_does_not_resurrect_the_threads_state() {
    let pending = deferred();
    let backend = Scripted::default_fake();
    let gate = pending.clone();
    backend.hooks().launch_app = Some(Arc::new(move |_, _| {
        let wait = gate.wait();
        async move {
            wait.await;
            Ok(ComputerLaunchAppResult {
                computer_id: "desktop".into(),
                app: "kcalc".into(),
                focus_changed_during_launch: None,
                pid: None,
                window_status: None,
                window_reason: None,
                resolved_command: None,
                window: None,
            })
        }
        .boxed()
    }));
    let Built {
        manager, backend, ..
    } = settled(backend);
    let events = events(&manager);
    let launching = tokio::spawn({
        let manager = manager.clone();
        async move {
            let _ = manager
                .launch_app(
                    Some("removed-thread"),
                    "kcalc",
                    &[],
                    0,
                    ComputerLaunchOptions::default(),
                )
                .await;
        }
    });
    while backend.logged("launchApp").is_empty() {
        tokio::task::yield_now().await;
    }
    let removing = tokio::spawn({
        let manager = manager.clone();
        async move { manager.handle_thread_removed("removed-thread").await }
    });
    settle().await;
    pending.resolve();
    launching.await.unwrap();
    removing.await.unwrap();
    settle().await;
    assert!(events.open_pane_requests().is_empty());
    let states_before = events.states().len();
    let state = manager.get_thread_state("removed-thread").await;
    assert!(!state.agent_active);
    assert_eq!(events.states().len(), states_before);
    manager.dispose().await;
}

#[tokio::test]
async fn publishes_a_reported_thread_error_exactly_once_then_lets_refresh_own_the_field() {
    let Built { manager, .. } = manager(Scripted::default_fake());
    let events = events(&manager);
    manager.get_thread_state("err-thread").await;
    manager
        .record_thread_error("err-thread", "backend hiccup")
        .await;
    let reported = |events: &Events| {
        events
            .states_for("err-thread")
            .iter()
            .filter(|state| state.last_error.as_deref() == Some("backend hiccup"))
            .count()
    };
    assert_eq!(reported(&events), 1);
    manager.get_thread_state("err-thread").await;
    assert_eq!(
        events.states_for("err-thread").last().unwrap().last_error,
        None
    );
    assert_eq!(reported(&events), 1);
    manager.dispose().await;
}

#[tokio::test]
async fn a_pause_arriving_after_control_was_revoked_leaves_no_stale_pause_state() {
    let pending = deferred();
    let backend = Scripted::default_fake();
    let gate = pending.clone();
    backend.hooks().type_text = Some(Arc::new(move |_, _| {
        let wait = gate.wait();
        async move {
            wait.await;
            Err(ComputerBackendError {
                input_pause: Some(zeron_proto::computer::ComputerInputPause {
                    window_id: Some("fake-calculator".into()),
                    pid: None,
                    message: "Return to the target window.".into(),
                }),
                ..ComputerBackendError::new("Return to the target window.")
            }
            .into())
        }
        .boxed()
    }));
    let Built {
        manager, backend, ..
    } = manager(backend);
    let typing = tokio::spawn({
        let manager = manager.clone();
        async move {
            let _ = manager.type_text(Some("paused-thread"), "hi", None).await;
        }
    });
    while backend.logged("typeText").is_empty() {
        tokio::task::yield_now().await;
    }
    let disabling = manager.revoke_control("paused-thread");
    pending.resolve();
    let _ = disabling.await;
    typing.await.unwrap();
    assert_eq!(
        manager.get_thread_state("paused-thread").await.input_pause,
        None
    );
    manager.dispose().await;
}

#[tokio::test]
async fn thread_removal_completes_on_a_wedged_stop_the_teardown_wait_is_bounded() {
    let backend = Scripted::default_fake();
    backend.hooks().stop_input = Some(Arc::new(|_, _| futures::future::pending().boxed()));
    let Built {
        manager, backend, ..
    } = settled(backend);
    manager
        .launch_app(
            Some("wedged"),
            "kcalc",
            &[],
            0,
            ComputerLaunchOptions::default(),
        )
        .await
        .unwrap();
    tokio::time::pause();
    let started = tokio::time::Instant::now();
    manager.handle_thread_removed("wedged").await;
    assert!(
        started.elapsed() <= Duration::from_millis(COMPUTER_CONTROL_TEARDOWN_TIMEOUT_MS + 1_000),
        "{:?}",
        started.elapsed()
    );
    assert!(!backend.logged("stopInput").is_empty());
    let started = tokio::time::Instant::now();
    manager.dispose().await;
    assert!(
        started.elapsed()
            <= Duration::from_millis(COMPUTER_CONTROL_TEARDOWN_TIMEOUT_MS * 2 + 2_000),
        "{:?}",
        started.elapsed()
    );
}
