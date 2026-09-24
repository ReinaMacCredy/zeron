use std::sync::atomic::AtomicUsize;
use std::time::Duration;

use tokio_util::sync::CancellationToken;
use zeron_proto::computer::{ComputerHealthFailure, ComputerHealthStatus, ComputerInputPause};

use super::*;
use crate::computer::approval_gate::ApprovalRequest;
use crate::computer::backend::ComputerBackendError;
use crate::computer::manager::{ComputerScrollOptions, ComputerStateRequest};
use crate::computer::task_context::with_computer_task;

fn state_of(window_id: &str) -> ComputerStateRequest {
    ComputerStateRequest {
        include_screenshot: None,
        include_text: false,
        include_tree: None,
        window_id: Some(window_id.to_owned()),
    }
}

fn task(thread_id: &str) -> CuaComputerTask {
    CuaComputerTask {
        thread_id: thread_id.to_owned(),
        turn_id: Some("turn".into()),
        label: None,
    }
}

fn available() -> ComputerAvailability {
    ComputerAvailability::Available {
        backend: Some("fake".into()),
    }
}

fn unavailable_message(availability: &ComputerAvailability) -> &str {
    match availability {
        ComputerAvailability::BackendUnavailable { message } => message,
        other => panic!("expected backend-unavailable, got {other:?}"),
    }
}

fn health(
    status: ComputerHealthStatus,
    consecutive_failures: u64,
    reconnects: u64,
    capture_available: bool,
) -> ComputerHealth {
    ComputerHealth {
        status,
        consecutive_failures,
        reconnects,
        last_failure: Some(ComputerHealthFailure {
            message: "The backend vanished".into(),
            at: "2026-08-16T10:00:00.000Z".into(),
        }),
        capture_available,
        background_input_degraded: None,
    }
}

fn calculator_pause(message: &str) -> ComputerInputPause {
    ComputerInputPause {
        window_id: Some("fake-calculator".into()),
        pid: None,
        message: message.to_owned(),
    }
}

fn paused(pause: &ComputerInputPause) -> ComputerError {
    ComputerBackendError {
        input_pause: Some(pause.clone()),
        ..ComputerBackendError::new(pause.message.clone())
    }
    .into()
}

async fn idle(manager: &ComputerManager, thread_id: &str) {
    manager
        .with_agent_activity(thread_id, || async { Ok(()) }, None, None, None)
        .await
        .unwrap();
}

#[tokio::test]
async fn publishes_thread_snapshots_activity_transitions_and_backend_window_events() {
    let Built {
        manager, backend, ..
    } = manager(Scripted::new(FakeComputerBackendOptions {
        now: Some(Arc::new(|| "2026-08-15T00:00:00.000Z".into())),
        ..FakeComputerBackendOptions::default()
    }));
    let events = events(&manager);

    let initial = manager.get_thread_state("thread-1").await;
    assert_eq!(initial.thread_id, "thread-1");
    assert!(initial.windows.is_empty());
    assert_eq!(backend.methods(), vec!["probeAvailability"]);
    assert_eq!(initial.availability, available());

    let result = manager
        .with_agent_activity(
            "thread-1",
            || async {
                let active = events.states().last().cloned().unwrap();
                assert!(active.agent_active);
                manager
                    .click(
                        Some("thread-1"),
                        labelled("Calculate", Some("button")),
                        &[],
                        Default::default(),
                    )
                    .await
            },
            None,
            None,
            None,
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
    assert!(events.states().len() >= 3);
    match events.all().last().unwrap() {
        ComputerEvent::ThreadState { state } => assert!(!state.agent_active),
        other => panic!("expected a thread-state event last, got {other:?}"),
    }

    let new_window = ComputerWindow {
        app_name: Some("org.kde.kwrite".into()),
        focused: true,
        ..window(
            "fake-notes",
            "Notes",
            Some(rect(200.0, 200.0, 500.0, 400.0)),
        )
    };
    backend.fake.emit_windows_changed(vec![new_window.clone()]);
    settle().await;
    let refreshed = manager.get_thread_state("thread-1").await;
    assert_eq!(refreshed.windows, vec![new_window]);
    assert!(events.types().contains(&"computer.windows-changed"));

    manager.dispose().await;
}

#[tokio::test]
async fn republishes_every_thread_when_backend_health_changes_without_touching_the_backend() {
    let Built {
        manager, backend, ..
    } = manager(Scripted::default_fake());
    let events = events(&manager);
    manager.list_windows().await.unwrap();
    let seeded = futures::join!(
        manager.get_thread_state("thread-a"),
        manager.get_thread_state("thread-b")
    );
    let seeded = [seeded.0, seeded.1];
    assert_eq!(
        seeded
            .iter()
            .map(|state| state.health.status)
            .collect::<Vec<_>>(),
        vec![
            ComputerHealthStatus::Connected,
            ComputerHealthStatus::Connected
        ]
    );
    let calls_before_health = backend.methods().len();

    backend
        .fake
        .emit_health_changed(health(ComputerHealthStatus::Reconnecting, 1, 0, false));

    assert_eq!(backend.methods().len(), calls_before_health);
    let degraded: Vec<ThreadComputerState> = ["thread-a", "thread-b"]
        .iter()
        .map(|thread_id| events.states_for(thread_id).last().cloned().unwrap())
        .collect();
    assert_eq!(
        degraded
            .iter()
            .map(|state| state.health.status)
            .collect::<Vec<_>>(),
        vec![
            ComputerHealthStatus::Reconnecting,
            ComputerHealthStatus::Reconnecting
        ]
    );
    for state in &degraded {
        unavailable_message(&state.availability);
    }
    assert!(unavailable_message(&degraded[0].availability).contains("The backend vanished"));
    assert!(degraded[0].version > seeded[0].version);

    backend
        .fake
        .emit_health_changed(health(ComputerHealthStatus::Connected, 0, 1, true));

    let recovered = manager.get_thread_state("thread-a").await;
    assert_eq!(recovered.availability, available());
    assert_eq!(recovered.health.status, ComputerHealthStatus::Connected);
    assert_eq!(recovered.health.reconnects, 1);

    manager.dispose().await;
}

#[tokio::test]
async fn answers_get_status_without_a_thread_corrected_by_live_health() {
    let Built {
        manager, backend, ..
    } = manager(Scripted::default_fake());

    let status = manager.get_status().await;
    assert_eq!(status.computer_id, "desktop");
    assert_eq!(status.availability, available());
    assert_eq!(status.health.status, ComputerHealthStatus::Connected);
    assert!(status.capabilities.input);
    let methods = backend.methods();
    assert!(!methods.iter().any(|method| method == "getState"));
    assert!(methods.iter().any(|method| method == "probeAvailability"));
    assert!(!methods.iter().any(|method| method == "availability"));

    manager.list_windows().await.unwrap();

    backend
        .fake
        .emit_health_changed(health(ComputerHealthStatus::Reconnecting, 2, 1, false));
    let degraded = manager.get_status().await;
    assert_eq!(degraded.health.status, ComputerHealthStatus::Reconnecting);
    assert!(unavailable_message(&degraded.availability).contains("The backend vanished"));

    manager.dispose().await;
}

#[tokio::test]
async fn reports_a_failed_availability_probe_as_backend_unavailable_instead_of_throwing() {
    let Built {
        manager, backend, ..
    } = manager(Scripted::default_fake());

    backend.fake.fail_next_with(
        "probeAvailability",
        ComputerBackendError::new("probe exploded"),
    );
    let status = manager.get_status().await;
    assert!(unavailable_message(&status.availability).contains("probe exploded"));

    manager.list_windows().await.unwrap();
    backend.fake.fail_next_with(
        "availability",
        ComputerBackendError::new("live read exploded"),
    );
    let engaged = manager.get_status().await;
    assert!(unavailable_message(&engaged.availability).contains("live read exploded"));

    manager.dispose().await;
}

#[tokio::test]
async fn clamps_an_oversized_backend_error_before_it_reaches_a_state_payload() {
    let Built {
        manager, backend, ..
    } = manager(Scripted::default_fake());

    let oversized = "E".repeat(100_000);
    backend
        .fake
        .fail_next_with("probeAvailability", ComputerBackendError::new(oversized));
    let state = manager.get_thread_state("thread-oversize").await;

    let last_error = state.last_error.clone().unwrap();
    assert!(last_error.encode_utf16().count() <= 2_048);
    assert!(
        unavailable_message(&state.availability)
            .encode_utf16()
            .count()
            <= 2_048
    );

    manager.dispose().await;
}

#[tokio::test]
async fn holds_refused_input_until_a_scoped_observation_establishes_readiness() {
    let backend = Scripted::default_fake();
    let ready = Arc::new(AtomicBool::new(false));
    let attempts = Arc::new(AtomicUsize::new(0));
    let checks = Arc::new(AtomicUsize::new(0));
    let pause = calculator_pause("Return to the target window.");
    backend.hooks().type_text = Some(Arc::new({
        let ready = ready.clone();
        let attempts = attempts.clone();
        move |fake, (text, window, target)| {
            attempts.fetch_add(1, Ordering::SeqCst);
            let refused = (!ready.load(Ordering::SeqCst)).then(|| paused(&pause));
            async move {
                if let Some(error) = refused {
                    return Err(error);
                }
                fake.type_text(&text, window.as_deref(), target.as_ref())
                    .await
            }
            .boxed()
        }
    }));
    backend.hooks().check_input_ready = Some(Arc::new({
        let ready = ready.clone();
        let checks = checks.clone();
        move |_, _| {
            checks.fetch_add(1, Ordering::SeqCst);
            let ready = ready.load(Ordering::SeqCst);
            async move {
                if ready {
                    Ok(())
                } else {
                    Err(ComputerBackendError::new("still unavailable").into())
                }
            }
            .boxed()
        }
    }));
    let Built { manager, .. } = manager(backend);

    let error = manager
        .type_text(Some("thread-a"), "hello", None)
        .await
        .unwrap_err();
    assert!(error.input_pause().is_some());
    let error = manager
        .type_text(Some("thread-a"), "hello", None)
        .await
        .unwrap_err();
    assert!(error.input_pause().is_some());
    assert_eq!(attempts.load(Ordering::SeqCst), 1);
    assert_eq!(
        manager
            .get_thread_state("thread-a")
            .await
            .input_pause
            .and_then(|pause| pause.window_id)
            .as_deref(),
        Some("fake-calculator")
    );
    manager
        .release_desktop_control("thread-a", None)
        .await
        .unwrap();
    assert!(
        manager
            .get_state(state_of("fake-calculator"))
            .await
            .unwrap()
            .input_pause
            .is_some()
    );
    assert_eq!(checks.load(Ordering::SeqCst), 1);
    assert!(
        manager
            .get_thread_state("thread-a")
            .await
            .input_pause
            .is_some()
    );
    ready.store(true, Ordering::SeqCst);
    manager
        .get_state(state_of("different-window"))
        .await
        .unwrap();
    assert_eq!(checks.load(Ordering::SeqCst), 1);
    manager
        .get_state(state_of("fake-calculator"))
        .await
        .unwrap();
    assert!(
        manager
            .get_thread_state("thread-a")
            .await
            .input_pause
            .is_none()
    );
    manager
        .type_text(Some("thread-a"), "hello", None)
        .await
        .unwrap();
    assert_eq!(attempts.load(Ordering::SeqCst), 2);
    manager.dispose().await;
}

#[tokio::test]
async fn clears_an_app_pause_only_for_the_observing_task_and_a_ready_same_pid_sibling() {
    let mut windows: Vec<ComputerWindow> = covered_calculator_windows()
        .into_iter()
        .enumerate()
        .map(|(index, window)| ComputerWindow {
            pid: Some(if index == 0 { 20 } else { 10 }),
            ..window
        })
        .collect();
    windows.push(ComputerWindow {
        id: "sibling".into(),
        on_current_space: Some(true),
        ..windows[1].clone()
    });
    let backend = Scripted::new(FakeComputerBackendOptions {
        windows: Some(windows),
        ..FakeComputerBackendOptions::default()
    });
    let attempts = Arc::new(AtomicUsize::new(0));
    let checks = Arc::new(AtomicUsize::new(0));
    let pause = ComputerInputPause {
        window_id: Some("fake-calculator".into()),
        pid: Some(10),
        message: "Observe the app again".into(),
    };
    backend.hooks().type_text = Some(Arc::new({
        let attempts = attempts.clone();
        move |fake, (text, window, target)| {
            let first = attempts.fetch_add(1, Ordering::SeqCst) == 0;
            let refused = first.then(|| paused(&pause));
            async move {
                if let Some(error) = refused {
                    return Err(error);
                }
                fake.type_text(&text, window.as_deref(), target.as_ref())
                    .await
            }
            .boxed()
        }
    }));
    backend.hooks().check_input_ready = Some(Arc::new({
        let checks = checks.clone();
        move |_, _| {
            checks.fetch_add(1, Ordering::SeqCst);
            async { Ok(()) }.boxed()
        }
    }));
    let Built { manager, .. } = manager(backend);

    let error = manager
        .type_text(Some("owner"), "hello", None)
        .await
        .unwrap_err();
    assert!(error.input_pause().is_some());
    manager
        .release_desktop_control("owner", None)
        .await
        .unwrap();
    with_computer_task(task("other"), manager.get_state(state_of("sibling")))
        .await
        .unwrap();
    assert_eq!(checks.load(Ordering::SeqCst), 0);
    with_computer_task(task("owner"), manager.get_state(state_of("fake-browser")))
        .await
        .unwrap();
    assert_eq!(checks.load(Ordering::SeqCst), 0);
    assert!(
        manager
            .get_thread_state("owner")
            .await
            .input_pause
            .is_some()
    );
    with_computer_task(task("owner"), manager.get_state(state_of("sibling")))
        .await
        .unwrap();
    assert_eq!(checks.load(Ordering::SeqCst), 1);
    assert!(
        manager
            .get_thread_state("owner")
            .await
            .input_pause
            .is_none()
    );
    manager.dispose().await;
}

#[tokio::test(start_paused = true)]
async fn reports_a_launched_app_with_an_unusable_window_without_replaying_launch() {
    let backend = Scripted::new(FakeComputerBackendOptions {
        windows: Some(
            covered_calculator_windows()
                .into_iter()
                .map(|window| ComputerWindow {
                    pid: Some(if window.id == "fake-calculator" {
                        10
                    } else {
                        20
                    }),
                    ..window
                })
                .collect(),
        ),
        ..FakeComputerBackendOptions::default()
    });
    let launches = Arc::new(AtomicUsize::new(0));
    backend.hooks().launch_app = Some(Arc::new({
        let launches = launches.clone();
        move |_, (app, _, _)| {
            launches.fetch_add(1, Ordering::SeqCst);
            async move {
                Ok(ComputerLaunchAppResult {
                    computer_id: "desktop".into(),
                    app,
                    focus_changed_during_launch: None,
                    pid: Some(10),
                    window_status: None,
                    window_reason: None,
                    resolved_command: None,
                    window: None,
                })
            }
            .boxed()
        }
    }));
    backend.hooks().check_input_ready = Some(Arc::new(|_, _| {
        async { Err(ComputerBackendError::new("ax_window_unresolved").into()) }.boxed()
    }));
    let Built { manager, .. } = manager(backend);

    let result = manager
        .launch_app(
            Some("owner"),
            "com.apple.Calculator",
            &[],
            2_000,
            ComputerLaunchOptions::default(),
        )
        .await
        .unwrap();
    assert_eq!(result.pid, Some(10));
    assert!(result.window.is_none());
    assert_eq!(
        result.window_status,
        Some(zeron_proto::computer::ComputerLaunchWindowStatus::NoUsableWindow)
    );
    assert_eq!(
        result.window_reason,
        Some(zeron_proto::computer::ComputerLaunchWindowReason::InputUnavailable)
    );
    assert_eq!(launches.load(Ordering::SeqCst), 1);
    manager.dispose().await;
}

#[tokio::test]
async fn publishes_activity_without_additional_desktop_reads() {
    let Built {
        manager, backend, ..
    } = manager(Scripted::default_fake());
    manager.list_windows().await.unwrap();
    manager.get_thread_state("watched").await;
    let before = backend.methods().len();
    idle(&manager, "watched").await;
    assert!(backend.methods()[before..].is_empty());
    manager.dispose().await;
}

#[tokio::test]
async fn evicts_idle_thread_records_while_preserving_increasing_versions() {
    let Built { manager, .. } = manager(Scripted::default_fake());
    let first = manager.get_thread_state("old").await;
    for i in 0..260 {
        manager.get_thread_state(&format!("idle-{i}")).await;
    }
    let events = events(&manager);
    idle(&manager, "old").await;
    assert_eq!(events.states().len(), 0);
    assert!(manager.get_thread_state("old").await.version > first.version);
    manager.dispose().await;
}

#[tokio::test]
async fn assigns_a_newer_version_to_each_refreshed_thread_snapshot() {
    let Built {
        manager, backend, ..
    } = manager(Scripted::default_fake());
    let initial = manager.get_thread_state("refreshed").await;
    backend
        .fake
        .set_availability(ComputerAvailability::BackendUnavailable {
            message: "Paused".into(),
        });
    let refreshed = manager.get_thread_state("refreshed").await;
    assert_eq!(
        refreshed.availability,
        ComputerAvailability::BackendUnavailable {
            message: "Paused".into()
        }
    );
    assert!(refreshed.version > initial.version);
    manager.dispose().await;
}

#[tokio::test]
async fn versions_a_delayed_refresh_after_cached_activity_publications() {
    let held = deferred();
    let entered = deferred();
    let delay = Arc::new(AtomicBool::new(false));
    let backend = Scripted::default_fake();
    backend.hooks().probe_availability = Some(Arc::new({
        let held = held.clone();
        let entered = entered.clone();
        let delay = delay.clone();
        move |fake, ()| {
            let delayed = delay.load(Ordering::SeqCst);
            let held = held.clone();
            let entered = entered.clone();
            async move {
                if delayed {
                    entered.resolve();
                    held.wait().await;
                }
                fake.probe_availability().await
            }
            .boxed()
        }
    }));
    let Built {
        manager, backend, ..
    } = manager(backend);
    manager.get_thread_state("refreshed").await;
    let events = events(&manager);
    delay.store(true, Ordering::SeqCst);
    backend
        .fake
        .set_availability(ComputerAvailability::BackendUnavailable {
            message: "Paused".into(),
        });
    let refreshing = tokio::spawn({
        let manager = manager.clone();
        async move { manager.get_thread_state("refreshed").await }
    });
    entered.wait().await;
    idle(&manager, "refreshed").await;
    let cached = events.states().last().cloned().unwrap();
    assert!(matches!(
        cached.availability,
        ComputerAvailability::Available { .. }
    ));
    held.resolve();
    let refreshed = refreshing.await.unwrap();
    assert!(refreshed.version > cached.version);
    assert!(matches!(
        refreshed.availability,
        ComputerAvailability::BackendUnavailable { .. }
    ));
    assert_eq!(events.states().last(), Some(&refreshed));
    manager.dispose().await;
}

#[tokio::test]
async fn pauses_calibrated_scrolling_before_any_second_input_or_launch_and_preserves_pause_during_idle_eviction()
 {
    let backend = Scripted::default_fake();
    let pause = calculator_pause("Return this window to the current Space.");
    let attempts = Arc::new(AtomicUsize::new(0));
    backend.hooks().scroll = Some(Arc::new({
        let attempts = attempts.clone();
        let pause = pause.clone();
        move |fake, (point, dx, dy, window, modifiers, target)| {
            let first = attempts.fetch_add(1, Ordering::SeqCst) == 0;
            let refused = first.then(|| paused(&pause));
            async move {
                if let Some(error) = refused {
                    return Err(error);
                }
                fake.scroll(
                    point,
                    dx,
                    dy,
                    window.as_deref(),
                    &modifiers,
                    target.as_ref(),
                )
                .await
            }
            .boxed()
        }
    }));
    let Built {
        manager, backend, ..
    } = manager(backend);
    let options = || ComputerScrollOptions {
        observe: false,
        modifiers: Vec::new(),
    };

    let target = ComputerTarget {
        window_id: Some("fake-calculator".into()),
        ..ComputerTarget::default()
    };
    let error = manager
        .scroll_calibrated(Some("paused"), Some(target.into()), 0.0, 40.0, options())
        .await
        .unwrap_err();
    assert!(error.input_pause().is_some());
    manager
        .release_desktop_control("paused", None)
        .await
        .unwrap();
    for i in 0..260 {
        manager.get_thread_state(&format!("other-{i}")).await;
    }
    assert_eq!(
        manager.get_thread_state("paused").await.input_pause,
        Some(pause)
    );
    let error = manager
        .scroll_calibrated(Some("paused"), None, 0.0, 40.0, options())
        .await
        .unwrap_err();
    assert!(error.input_pause().is_some());
    let error = manager
        .launch_app(
            Some("paused"),
            "Calculator",
            &[],
            0,
            ComputerLaunchOptions::default(),
        )
        .await
        .unwrap_err();
    assert!(error.input_pause().is_some());
    assert_eq!(attempts.load(Ordering::SeqCst), 1);
    assert!(backend.calls_for("launchApp").is_empty());
    manager.dispose().await;
}

#[tokio::test]
async fn settles_a_pending_approval_prompt_when_control_is_switched_off_mid_turn() {
    let Built { manager, .. } = manager(Scripted::default_fake());
    let prompt = tokio::spawn({
        let manager = manager.clone();
        async move {
            manager
                .approval_gate()
                .request(ApprovalRequest {
                    chat_id: "owner".into(),
                    turn_id: Some("turn-1".into()),
                    mode: PermissionMode::Ask,
                    signal: CancellationToken::new(),
                    publish: Arc::new(|_, _| async { Ok(()) }.boxed()),
                    app: None,
                })
                .await
        }
    });
    settle().await;
    manager.stop_thread("owner").await.unwrap();
    let decision = tokio::time::timeout(Duration::from_secs(5), prompt)
        .await
        .expect("the prompt settles without waiting out the gate timeout")
        .unwrap();
    assert_eq!(decision, Ok(false));
    manager.approval_gate().cancel_thread("owner", None);
    manager.dispose().await;
}

#[tokio::test]
async fn keeps_a_pause_when_the_thread_is_re_armed_while_its_readiness_probe_is_in_flight() {
    let entered = deferred();
    let gate = deferred();
    let ready = Arc::new(AtomicBool::new(false));
    let pause = calculator_pause("Return to the target window.");
    let backend = Scripted::default_fake();
    backend.hooks().type_text = Some(Arc::new({
        let ready = ready.clone();
        move |fake, (text, window, target)| {
            let refused = (!ready.load(Ordering::SeqCst)).then(|| paused(&pause));
            async move {
                if let Some(error) = refused {
                    return Err(error);
                }
                fake.type_text(&text, window.as_deref(), target.as_ref())
                    .await
            }
            .boxed()
        }
    }));
    backend.hooks().check_input_ready = Some(Arc::new({
        let entered = entered.clone();
        let gate = gate.clone();
        let ready = ready.clone();
        move |_, _| {
            let entered = entered.clone();
            let gate = gate.clone();
            let ready = ready.clone();
            async move {
                entered.resolve();
                gate.wait().await;
                if ready.load(Ordering::SeqCst) {
                    Ok(())
                } else {
                    Err(ComputerBackendError::new("still unavailable").into())
                }
            }
            .boxed()
        }
    }));
    let Built { manager, .. } = manager(backend);

    let error = manager
        .type_text(Some("thread-a"), "hello", None)
        .await
        .unwrap_err();
    assert!(error.input_pause().is_some());
    let probing = tokio::spawn({
        let manager = manager.clone();
        async move { manager.get_state(state_of("fake-calculator")).await }
    });
    entered.wait().await;
    manager.stop_thread("thread-a").await.unwrap();
    idle(&manager, "thread-a").await;
    ready.store(true, Ordering::SeqCst);
    gate.resolve();
    probing.await.unwrap().unwrap();
    assert!(
        manager
            .get_thread_state("thread-a")
            .await
            .input_pause
            .is_some()
    );
    let error = manager
        .type_text(Some("thread-a"), "hello", None)
        .await
        .unwrap_err();
    assert!(error.input_pause().is_some());
    manager.dispose().await;
}
