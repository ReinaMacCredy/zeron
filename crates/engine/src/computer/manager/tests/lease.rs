use super::*;

use std::sync::atomic::AtomicUsize;
use std::time::Duration;

use crate::computer::backend::ComputerBackendError;
use crate::computer::manager::{ComputerLeaseLifecycle, ComputerStateRequest};

async fn wait_until(mut done: impl FnMut() -> bool) {
    for _ in 0..400 {
        if done() {
            return;
        }
        settle().await;
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    assert!(done(), "condition never held");
}

async fn wait_until_free(manager: &ComputerManager, thread_id: &str) {
    for _ in 0..400 {
        if !manager
            .get_thread_state(thread_id)
            .await
            .controlled_by_other_thread
        {
            return;
        }
        settle().await;
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    panic!("{thread_id} stayed blocked");
}

async fn click(
    manager: &ComputerManager,
    thread_id: Option<&str>,
    x: f64,
    y: f64,
) -> Result<zeron_proto::computer::ComputerActionResult, ComputerError> {
    manager
        .click(thread_id, point(x, y), &[], Default::default())
        .await
}

fn assert_blocked(error: &ComputerError) {
    assert_eq!(code(error), Some("computer_controlled_by_other_thread"));
    assert!(error.to_string().contains("another conversation"));
}

fn lease_record(
    ts: &str,
    event: ComputerLeaseLifecycle,
    thread_id: &str,
    turn_id: &str,
) -> ComputerLeaseRecord {
    ComputerLeaseRecord {
        ts: ts.to_owned(),
        event,
        thread_id: thread_id.to_owned(),
        turn_id: Some(turn_id.to_owned()),
        idle_ms: None,
        next_thread_id: None,
    }
}

#[tokio::test]
async fn the_lease_holder_keeps_one_still_stream_through_another_chats_calls() {
    let Built {
        manager, backend, ..
    } = manager(Scripted::default_fake());
    settle().await;
    manager.flush_stream_transitions().await;
    assert!(backend.calls_for("attachStream").is_empty());
    click(&manager, Some("thread-a"), 10.0, 10.0).await.unwrap();
    settle().await;
    manager.flush_stream_transitions().await;
    assert_eq!(backend.calls_for("attachStream").len(), 1);
    assert!(manager.latest_frame().is_some());
    assert!(manager.holds_desktop("thread-a"));

    let error = click(&manager, Some("thread-b"), 20.0, 20.0)
        .await
        .unwrap_err();
    assert_blocked(&error);
    manager
        .with_agent_activity("thread-b", || async { Ok(()) }, None, None, None)
        .await
        .unwrap();
    settle().await;
    manager.flush_stream_transitions().await;
    assert_eq!(
        backend.calls_for("attachStream").len(),
        1,
        "another chat's passing call does not restart the holder's stream"
    );
    assert!(backend.calls_for("detachStream").is_empty());
    assert!(manager.latest_frame().is_some());

    manager
        .release_desktop_control("thread-a", None)
        .await
        .unwrap();
    settle().await;
    manager.flush_stream_transitions().await;
    assert!(!manager.holds_desktop("thread-a"));
    assert_eq!(backend.calls_for("detachStream").len(), 1);
    assert!(manager.latest_frame().is_none());
    manager.dispose().await;
}

#[tokio::test]
async fn gives_the_desktop_to_the_first_thread_that_drives_it_and_refuses_the_second() {
    let Built {
        manager, backend, ..
    } = manager(Scripted::default_fake());
    let events = events(&manager);
    manager.get_thread_state("thread-a").await;
    manager.get_thread_state("thread-b").await;

    click(&manager, Some("thread-a"), 10.0, 10.0).await.unwrap();
    let error = click(&manager, Some("thread-b"), 20.0, 20.0)
        .await
        .unwrap_err();
    assert_eq!(code(&error), Some("computer_controlled_by_other_thread"));
    assert!(!error.retryable());
    assert!(
        error
            .to_string()
            .contains("another conversation; no input was sent. Do not retry")
    );

    assert_eq!(
        manager.list_windows().await.unwrap().computer_id,
        backend.computer_id()
    );
    assert_eq!(
        manager
            .get_state(ComputerStateRequest::default())
            .await
            .unwrap()
            .computer_id,
        backend.computer_id()
    );
    assert_eq!(
        manager.get_screen_size().await.unwrap().computer_id,
        backend.computer_id()
    );
    assert!(
        manager
            .get_thread_state("thread-b")
            .await
            .controlled_by_other_thread
    );
    assert!(
        !manager
            .get_thread_state("thread-a")
            .await
            .controlled_by_other_thread
    );

    assert_eq!(
        click(&manager, None, 30.0, 30.0).await.unwrap().action,
        "computer_click"
    );
    let error = click(&manager, Some("thread-b"), 20.0, 20.0)
        .await
        .unwrap_err();
    assert_eq!(code(&error), Some("computer_controlled_by_other_thread"));

    assert!(
        events
            .states()
            .iter()
            .any(|state| state.thread_id == "thread-b" && state.controlled_by_other_thread)
    );
    assert_eq!(
        events
            .states_for("thread-a")
            .last()
            .map(|state| state.controlled_by_other_thread),
        Some(false)
    );

    manager.dispose().await;
}

#[tokio::test]
async fn allows_different_threads_to_overlap_exact_window_semantic_text() {
    let release = deferred();
    let active = Arc::new(AtomicUsize::new(0));
    let peak = Arc::new(AtomicUsize::new(0));
    let window_ids = ["editor-a", "editor-b"];
    let windows = window_ids
        .iter()
        .enumerate()
        .map(|(index, id)| {
            window(
                id,
                &format!("Editor {}", index + 1),
                Some(rect(index as f64 * 400.0, 0.0, 360.0, 300.0)),
            )
        })
        .collect();
    let backend = Scripted::new(FakeComputerBackendOptions {
        windows: Some(windows),
        root: Some(semantic_text_root(&window_ids)),
        ..FakeComputerBackendOptions::default()
    });
    backend.set_background_target(false, true);
    {
        let (active, peak, release) = (active.clone(), peak.clone(), release.clone());
        backend.hooks().type_text = Some(Arc::new(move |_, _| {
            let (active, peak, release) = (active.clone(), peak.clone(), release.clone());
            async move {
                let now = active.fetch_add(1, Ordering::SeqCst) + 1;
                peak.fetch_max(now, Ordering::SeqCst);
                release.wait().await;
                active.fetch_sub(1, Ordering::SeqCst);
                Ok(ComputerBackendActionResult::default())
            }
            .boxed()
        }));
    }
    let Built {
        manager, backend, ..
    } = settled(backend);

    let spawn_typing = |thread: &'static str, text: &'static str, window: &'static str, turn| {
        let manager = manager.clone();
        tokio::spawn(async move {
            let inner = manager.clone();
            manager
                .with_agent_activity(
                    thread,
                    move || async move { inner.type_text(Some(thread), text, Some(window)).await },
                    None,
                    Some(turn),
                    Some(window),
                )
                .await
        })
    };
    let first = spawn_typing("thread-a", "alpha", "editor-a", "turn-a");
    let second = spawn_typing("thread-b", "bravo", "editor-b", "turn-b");
    wait_until(|| peak.load(Ordering::SeqCst) == 2).await;
    release.resolve();

    first.await.unwrap().unwrap();
    second.await.unwrap().unwrap();
    assert!(backend.calls_for("clearFocusWindow").is_empty());
    assert!(backend.calls_for("focusWindow").is_empty());
    manager.dispose().await;
}

#[tokio::test]
async fn hands_the_desktop_to_the_next_thread_when_the_owners_turn_ends() {
    let Built { manager, .. } = manager(Scripted::default_fake());
    manager.get_thread_state("thread-a").await;
    manager.get_thread_state("thread-b").await;

    manager
        .launch_app(
            Some("thread-a"),
            "kcalc",
            &[],
            0,
            ComputerLaunchOptions::default(),
        )
        .await
        .unwrap();
    assert_blocked(
        &manager
            .type_text(Some("thread-b"), "hi", None)
            .await
            .unwrap_err(),
    );

    manager
        .release_desktop_control("thread-a", None)
        .await
        .unwrap();
    assert!(
        !manager
            .get_thread_state("thread-b")
            .await
            .controlled_by_other_thread
    );

    assert_eq!(
        manager
            .type_text(Some("thread-b"), "hi", None)
            .await
            .unwrap()
            .action,
        "computer_type_text"
    );
    assert_blocked(
        &click(&manager, Some("thread-a"), 10.0, 10.0)
            .await
            .unwrap_err(),
    );
    manager
        .release_desktop_control("thread-a", None)
        .await
        .unwrap();
    assert_blocked(
        &click(&manager, Some("thread-a"), 10.0, 10.0)
            .await
            .unwrap_err(),
    );
    assert!(
        manager
            .get_thread_state("thread-a")
            .await
            .controlled_by_other_thread
    );

    manager.handle_thread_removed("thread-b").await;
    assert_eq!(
        click(&manager, Some("thread-a"), 10.0, 10.0)
            .await
            .unwrap()
            .action,
        "computer_click"
    );

    manager.dispose().await;
}

#[tokio::test]
async fn releases_desktop_control_even_when_preview_cleanup_is_delayed_or_fails() {
    let pending = deferred();
    let backend = Scripted::default_fake();
    {
        let pending = pending.clone();
        backend.hooks().end_task = Some(Arc::new(move |_, _| {
            let pending = pending.clone();
            async move {
                pending.wait().await;
                Err(ComputerBackendError::new("Preview cleanup failed").into())
            }
            .boxed()
        }));
    }
    let Built {
        manager, backend, ..
    } = manager(backend);
    let events = events(&manager);
    manager
        .launch_app(
            Some("thread-a"),
            "kcalc",
            &[],
            0,
            ComputerLaunchOptions::default(),
        )
        .await
        .unwrap();
    let cleared = backend.calls_for("clearFocusWindow").len();
    let release = {
        let manager = manager.clone();
        tokio::spawn(async move {
            manager
                .release_desktop_control("thread-a", Some("turn-one"))
                .await
        })
    };
    wait_until(|| backend.calls_for("clearFocusWindow").len() == cleared + 1).await;
    wait_until_free(&manager, "thread-b").await;
    assert_eq!(
        manager
            .type_text(Some("thread-b"), "hi", None)
            .await
            .unwrap()
            .action,
        "computer_type_text"
    );
    let released = tokio::time::timeout(Duration::from_secs(2), release).await;
    pending.resolve();
    released
        .expect("release settled while endTask was pending")
        .unwrap()
        .unwrap();
    assert!(
        backend
            .logged("endTask")
            .contains(&json!(["thread-a", "turn-one"]))
    );
    wait_until(|| {
        events.states().iter().any(|state| {
            state.thread_id == "thread-a"
                && state
                    .last_error
                    .as_deref()
                    .is_some_and(|error| error.contains("Preview cleanup failed"))
        })
    })
    .await;
    manager.dispose().await;
}

#[tokio::test]
async fn records_lease_transitions_without_text_or_cursor_labels_and_discourages_blocked_retries() {
    let (now, clock_fn) = clock(0);
    let Built { manager, seams, .. } = build(
        Scripted::default_fake(),
        Setup {
            now: Some(clock_fn),
            lease_idle_ms: Some(1_000),
            ..Setup::default()
        },
    );
    manager.set_thread_label("thread-a", Some("Private window title"));
    let inner = manager.clone();
    manager
        .with_agent_activity(
            "thread-a",
            move || async move {
                inner
                    .type_text(Some("thread-a"), "private typed value", None)
                    .await
            },
            None,
            Some("turn-a"),
            None,
        )
        .await
        .unwrap();
    let error = manager
        .type_text(Some("thread-b"), "blocked text", None)
        .await
        .unwrap_err();
    assert_eq!(code(&error), Some("computer_controlled_by_other_thread"));
    assert!(!error.retryable());
    assert!(error.to_string().contains("Do not retry"));
    *now.lock().unwrap() = 2_000;
    let inner = manager.clone();
    manager
        .with_agent_activity(
            "thread-b",
            move || async move { click(&inner, Some("thread-b"), 10.0, 10.0).await },
            None,
            Some("turn-b"),
            None,
        )
        .await
        .unwrap();
    manager
        .release_desktop_control("thread-b", Some("turn-b"))
        .await
        .unwrap();
    let entries = seams.leases.lock().unwrap().clone();
    assert_eq!(
        entries,
        vec![
            lease_record(
                "1970-01-01T00:00:00.000Z",
                ComputerLeaseLifecycle::Acquired,
                "thread-a",
                "turn-a",
            ),
            ComputerLeaseRecord {
                idle_ms: Some(2_000),
                next_thread_id: Some("thread-b".into()),
                ..lease_record(
                    "1970-01-01T00:00:02.000Z",
                    ComputerLeaseLifecycle::StaleReclaimed,
                    "thread-a",
                    "turn-a",
                )
            },
            lease_record(
                "1970-01-01T00:00:02.000Z",
                ComputerLeaseLifecycle::Acquired,
                "thread-b",
                "turn-b",
            ),
            lease_record(
                "1970-01-01T00:00:02.000Z",
                ComputerLeaseLifecycle::Released,
                "thread-b",
                "turn-b",
            ),
        ]
    );
    let serialized = format!("{entries:?}").to_lowercase();
    for word in ["private", "blocked", "title"] {
        assert!(
            !serialized.contains(word),
            "{word} leaked into {serialized}"
        );
    }
    manager.dispose().await;
}

#[tokio::test]
async fn defers_a_release_until_the_owners_in_flight_call_drains() {
    let Built { manager, .. } = manager(Scripted::default_fake());
    manager.get_thread_state("thread-a").await;
    manager.get_thread_state("thread-b").await;

    let started = deferred();
    let finish = deferred();
    let in_flight = {
        let (manager, started, finish) = (manager.clone(), started.clone(), finish.clone());
        tokio::spawn(async move {
            let inner = manager.clone();
            manager
                .with_agent_activity(
                    "thread-a",
                    move || async move {
                        click(&inner, Some("thread-a"), 10.0, 10.0).await?;
                        started.resolve();
                        finish.wait().await;
                        Ok(())
                    },
                    None,
                    None,
                    None,
                )
                .await
        })
    };
    started.wait().await;

    manager
        .release_desktop_control("thread-a", None)
        .await
        .unwrap();
    assert_blocked(
        &manager
            .type_text(Some("thread-b"), "hi", None)
            .await
            .unwrap_err(),
    );
    assert!(
        manager
            .get_thread_state("thread-b")
            .await
            .controlled_by_other_thread
    );

    finish.resolve();
    in_flight.await.unwrap().unwrap();
    assert!(
        !manager
            .get_thread_state("thread-b")
            .await
            .controlled_by_other_thread
    );
    assert_eq!(
        manager
            .type_text(Some("thread-b"), "hi", None)
            .await
            .unwrap()
            .action,
        "computer_type_text"
    );
    assert_blocked(
        &click(&manager, Some("thread-a"), 10.0, 10.0)
            .await
            .unwrap_err(),
    );

    manager.dispose().await;
}

#[tokio::test]
async fn keeps_a_deferred_release_when_the_owner_renews_the_lease_mid_drain() {
    let Built {
        manager, backend, ..
    } = manager(Scripted::default_fake());
    manager.get_thread_state("thread-a").await;
    manager.get_thread_state("thread-b").await;

    let started = deferred();
    let release_recorded = deferred();
    let in_flight = {
        let (manager, started, recorded) =
            (manager.clone(), started.clone(), release_recorded.clone());
        tokio::spawn(async move {
            let inner = manager.clone();
            manager
                .with_agent_activity(
                    "thread-a",
                    move || async move {
                        click(&inner, Some("thread-a"), 10.0, 10.0).await?;
                        started.resolve();
                        recorded.wait().await;
                        click(&inner, Some("thread-a"), 11.0, 11.0).await?;
                        Ok(())
                    },
                    None,
                    None,
                    None,
                )
                .await
        })
    };
    started.wait().await;
    manager
        .release_desktop_control("thread-a", None)
        .await
        .unwrap();
    release_recorded.resolve();
    in_flight.await.unwrap().unwrap();

    assert_eq!(backend.calls_for("click").len(), 2);
    assert!(
        !manager
            .get_thread_state("thread-b")
            .await
            .controlled_by_other_thread
    );
    assert_eq!(
        manager
            .type_text(Some("thread-b"), "hi", None)
            .await
            .unwrap()
            .action,
        "computer_type_text"
    );

    manager.dispose().await;
}

#[tokio::test]
async fn does_not_tear_down_a_renewed_lease_for_a_stale_deferred_release() {
    let Built { manager, .. } = manager(Scripted::default_fake());
    manager.get_thread_state("thread-a").await;
    manager.get_thread_state("thread-b").await;

    let started = deferred();
    let release_recorded = deferred();
    let finish = deferred();
    let in_flight = {
        let (manager, started, recorded, finish) = (
            manager.clone(),
            started.clone(),
            release_recorded.clone(),
            finish.clone(),
        );
        tokio::spawn(async move {
            let inner = manager.clone();
            manager
                .with_agent_activity(
                    "thread-a",
                    move || async move {
                        click(&inner, Some("thread-a"), 10.0, 10.0).await?;
                        started.resolve();
                        recorded.wait().await;
                        let nested = inner.clone();
                        inner
                            .with_agent_activity(
                                "thread-a",
                                move || async move {
                                    click(&nested, Some("thread-a"), 11.0, 11.0).await
                                },
                                None,
                                Some("turn-2"),
                                None,
                            )
                            .await?;
                        finish.wait().await;
                        Ok(())
                    },
                    None,
                    Some("turn-1"),
                    None,
                )
                .await
        })
    };
    started.wait().await;
    manager
        .release_desktop_control("thread-a", Some("turn-1"))
        .await
        .unwrap();
    release_recorded.resolve();
    finish.resolve();
    in_flight.await.unwrap().unwrap();

    assert!(
        manager
            .get_thread_state("thread-b")
            .await
            .controlled_by_other_thread
    );
    assert_blocked(
        &manager
            .type_text(Some("thread-b"), "hi", None)
            .await
            .unwrap_err(),
    );

    manager
        .release_desktop_control("thread-a", Some("turn-2"))
        .await
        .unwrap();
    assert!(
        !manager
            .get_thread_state("thread-b")
            .await
            .controlled_by_other_thread
    );

    manager.dispose().await;
}

#[tokio::test]
async fn does_not_let_a_queued_thread_level_release_tear_down_a_newer_turn() {
    let backend = Scripted::default_fake();
    backend.hooks().end_task = Some(Arc::new(|_, _| async { Ok(()) }.boxed()));
    let Built {
        manager, backend, ..
    } = manager(backend);
    let started = deferred();
    let finish = deferred();

    let inner = manager.clone();
    manager
        .with_agent_activity(
            "thread-a",
            move || async move { click(&inner, Some("thread-a"), 10.0, 10.0).await },
            None,
            Some("turn-old"),
            None,
        )
        .await
        .unwrap();
    let blocker = {
        let (manager, started, finish) = (manager.clone(), started.clone(), finish.clone());
        tokio::spawn(async move {
            manager
                .with_agent_activity(
                    "reader",
                    move || async move {
                        started.resolve();
                        finish.wait().await;
                        Ok(())
                    },
                    None,
                    None,
                    None,
                )
                .await
        })
    };
    started.wait().await;
    let renewed = {
        let manager = manager.clone();
        tokio::spawn(async move {
            let inner = manager.clone();
            manager
                .with_agent_activity(
                    "thread-a",
                    move || async move { click(&inner, Some("thread-a"), 20.0, 20.0).await },
                    None,
                    Some("turn-new"),
                    None,
                )
                .await
        })
    };
    settle().await;
    let released = {
        let manager = manager.clone();
        tokio::spawn(async move { manager.release_desktop_control("thread-a", None).await })
    };
    settle().await;
    finish.resolve();
    blocker.await.unwrap().unwrap();
    renewed.await.unwrap().unwrap();
    released.await.unwrap().unwrap();
    settle().await;
    assert_eq!(
        backend.logged("endTask"),
        vec![json!(["thread-a", "turn-old"])]
    );
    assert_blocked(
        &manager
            .type_text(Some("thread-b"), "second", None)
            .await
            .unwrap_err(),
    );
    manager
        .release_desktop_control("thread-a", Some("turn-new"))
        .await
        .unwrap();
    assert_eq!(
        manager
            .type_text(Some("thread-b"), "second", None)
            .await
            .unwrap()
            .action,
        "computer_type_text"
    );
    manager.dispose().await;
}

#[tokio::test]
async fn does_not_stamp_a_released_turn_onto_a_later_turn_id_less_claim() {
    let Built { manager, .. } = manager(Scripted::default_fake());
    manager.get_thread_state("thread-a").await;
    manager.get_thread_state("thread-b").await;

    let inner = manager.clone();
    manager
        .with_agent_activity(
            "thread-a",
            move || async move { click(&inner, Some("thread-a"), 10.0, 10.0).await },
            None,
            Some("turn-1"),
            None,
        )
        .await
        .unwrap();
    manager
        .release_desktop_control("thread-a", Some("turn-1"))
        .await
        .unwrap();

    let inner = manager.clone();
    manager
        .with_agent_activity(
            "thread-a",
            move || async move { click(&inner, Some("thread-a"), 11.0, 11.0).await },
            None,
            None,
            None,
        )
        .await
        .unwrap();
    assert!(
        manager
            .get_thread_state("thread-b")
            .await
            .controlled_by_other_thread
    );
    manager
        .release_desktop_control("thread-a", Some("turn-2"))
        .await
        .unwrap();
    assert!(
        !manager
            .get_thread_state("thread-b")
            .await
            .controlled_by_other_thread
    );

    manager.dispose().await;
}

#[tokio::test]
async fn still_fully_removes_a_thread_whose_stop_rejects_in_preview_cleanup() {
    let backend = Scripted::default_fake();
    backend.hooks().end_task = Some(Arc::new(|_, _| {
        async { Err(ComputerBackendError::new("Preview cleanup failed").into()) }.boxed()
    }));
    let Built { manager, .. } = manager(backend);
    manager.get_thread_state("thread-b").await;
    manager
        .launch_app(
            Some("thread-a"),
            "kcalc",
            &[],
            0,
            ComputerLaunchOptions::default(),
        )
        .await
        .unwrap();

    manager.handle_thread_removed("thread-a").await;
    assert!(
        !manager
            .get_thread_state("thread-b")
            .await
            .controlled_by_other_thread
    );
    assert_eq!(
        manager
            .type_text(Some("thread-b"), "hi", None)
            .await
            .unwrap()
            .action,
        "computer_type_text"
    );
    manager.dispose().await;
}

#[tokio::test]
async fn clears_an_evicted_owners_turn_stamp_along_with_its_stale_lease() {
    let (now, clock_fn) = clock(0);
    let Built { manager, .. } = build(
        Scripted::default_fake(),
        Setup {
            now: Some(clock_fn),
            lease_idle_ms: Some(1_000),
            ..Setup::default()
        },
    );
    manager.get_thread_state("thread-a").await;
    manager.get_thread_state("thread-b").await;

    let inner = manager.clone();
    manager
        .with_agent_activity(
            "thread-a",
            move || async move { click(&inner, Some("thread-a"), 10.0, 10.0).await },
            None,
            Some("turn-1"),
            None,
        )
        .await
        .unwrap();
    *now.lock().unwrap() = 2_000;
    let inner = manager.clone();
    manager
        .with_agent_activity(
            "thread-b",
            move || async move { click(&inner, Some("thread-b"), 10.0, 10.0).await },
            None,
            Some("turn-9"),
            None,
        )
        .await
        .unwrap();
    *now.lock().unwrap() = 4_000;
    let inner = manager.clone();
    manager
        .with_agent_activity(
            "thread-a",
            move || async move { click(&inner, Some("thread-a"), 11.0, 11.0).await },
            None,
            None,
            None,
        )
        .await
        .unwrap();
    manager
        .release_desktop_control("thread-a", Some("turn-5"))
        .await
        .unwrap();
    assert!(
        !manager
            .get_thread_state("thread-b")
            .await
            .controlled_by_other_thread
    );
    manager.dispose().await;
}

#[tokio::test]
async fn drops_an_anonymous_deferred_release_once_a_turn_renews_the_lease() {
    let Built { manager, .. } = manager(Scripted::default_fake());
    manager.get_thread_state("thread-a").await;
    manager.get_thread_state("thread-b").await;

    let started = deferred();
    let release_recorded = deferred();
    let finish = deferred();
    let in_flight = {
        let (manager, started, recorded, finish) = (
            manager.clone(),
            started.clone(),
            release_recorded.clone(),
            finish.clone(),
        );
        tokio::spawn(async move {
            let inner = manager.clone();
            manager
                .with_agent_activity(
                    "thread-a",
                    move || async move {
                        click(&inner, Some("thread-a"), 10.0, 10.0).await?;
                        started.resolve();
                        recorded.wait().await;
                        let nested = inner.clone();
                        inner
                            .with_agent_activity(
                                "thread-a",
                                move || async move {
                                    click(&nested, Some("thread-a"), 11.0, 11.0).await
                                },
                                None,
                                Some("turn-2"),
                                None,
                            )
                            .await?;
                        finish.wait().await;
                        Ok(())
                    },
                    None,
                    None,
                    None,
                )
                .await
        })
    };
    started.wait().await;
    manager
        .release_desktop_control("thread-a", None)
        .await
        .unwrap();
    release_recorded.resolve();
    finish.resolve();
    in_flight.await.unwrap().unwrap();

    assert!(
        manager
            .get_thread_state("thread-b")
            .await
            .controlled_by_other_thread
    );
    assert_blocked(
        &manager
            .type_text(Some("thread-b"), "hi", None)
            .await
            .unwrap_err(),
    );
    manager.dispose().await;
}

#[tokio::test]
async fn reacquires_the_lease_for_a_new_turn_after_the_previous_operation_drains() {
    let Built {
        manager, backend, ..
    } = manager(Scripted::default_fake());
    manager.get_thread_state("thread-a").await;

    let started = deferred();
    let finish = deferred();
    let in_flight = {
        let (manager, started, finish) = (manager.clone(), started.clone(), finish.clone());
        tokio::spawn(async move {
            let inner = manager.clone();
            manager
                .with_agent_activity(
                    "thread-a",
                    move || async move {
                        click(&inner, Some("thread-a"), 10.0, 10.0).await?;
                        started.resolve();
                        finish.wait().await;
                        Ok(())
                    },
                    None,
                    None,
                    None,
                )
                .await
        })
    };
    started.wait().await;
    manager
        .release_desktop_control("thread-a", None)
        .await
        .unwrap();
    let next_turn = {
        let manager = manager.clone();
        tokio::spawn(async move { click(&manager, Some("thread-a"), 20.0, 20.0).await })
    };
    settle().await;
    assert_eq!(backend.calls_for("click").len(), 1);
    finish.resolve();
    in_flight.await.unwrap().unwrap();
    next_turn.await.unwrap().unwrap();

    assert_blocked(
        &manager
            .type_text(Some("thread-b"), "hi", None)
            .await
            .unwrap_err(),
    );
    assert!(
        manager
            .get_thread_state("thread-b")
            .await
            .controlled_by_other_thread
    );

    manager.dispose().await;
}

#[tokio::test]
async fn expires_an_idle_lease_as_a_backstop_but_never_one_whose_owner_is_still_acting() {
    let (now, clock_fn) = clock(0);
    let Built { manager, .. } = build(
        Scripted::default_fake(),
        Setup {
            now: Some(clock_fn),
            lease_idle_ms: Some(1_000),
            ..Setup::default()
        },
    );
    manager.get_thread_state("thread-a").await;

    click(&manager, Some("thread-a"), 10.0, 10.0).await.unwrap();
    *now.lock().unwrap() = 999;
    assert_blocked(
        &click(&manager, Some("thread-b"), 20.0, 20.0)
            .await
            .unwrap_err(),
    );

    *now.lock().unwrap() = 10_000;
    let started = deferred();
    let finish = deferred();
    let in_flight = {
        let (manager, started, finish) = (manager.clone(), started.clone(), finish.clone());
        tokio::spawn(async move {
            manager
                .with_agent_activity(
                    "thread-a",
                    move || async move {
                        started.resolve();
                        finish.wait().await;
                        Ok(())
                    },
                    None,
                    None,
                    None,
                )
                .await
        })
    };
    started.wait().await;
    assert_blocked(
        &click(&manager, Some("thread-b"), 20.0, 20.0)
            .await
            .unwrap_err(),
    );
    finish.resolve();
    in_flight.await.unwrap().unwrap();

    assert_eq!(
        click(&manager, Some("thread-b"), 20.0, 20.0)
            .await
            .unwrap()
            .action,
        "computer_click"
    );
    assert_blocked(
        &click(&manager, Some("thread-a"), 10.0, 10.0)
            .await
            .unwrap_err(),
    );

    manager.dispose().await;
}

#[tokio::test]
async fn counts_an_agents_in_flight_call_even_when_no_panel_ever_asked_about_it() {
    let capabilities = ComputerCapabilities {
        visible_desktop: true,
        ..Scripted::default_fake().capabilities()
    };
    let (now, clock_fn) = clock(0);
    let Built { manager, .. } = build(
        Scripted::new(FakeComputerBackendOptions {
            capabilities: Some(capabilities),
            ..FakeComputerBackendOptions::default()
        }),
        Setup {
            now: Some(clock_fn),
            lease_idle_ms: Some(1_000),
            ..Setup::default()
        },
    );

    let started = deferred();
    let finish = deferred();
    let in_flight = {
        let (manager, started, finish) = (manager.clone(), started.clone(), finish.clone());
        tokio::spawn(async move {
            let inner = manager.clone();
            manager
                .with_agent_activity(
                    "thread-a",
                    move || async move {
                        click(&inner, Some("thread-a"), 10.0, 10.0).await?;
                        started.resolve();
                        finish.wait().await;
                        Ok(())
                    },
                    None,
                    None,
                    None,
                )
                .await
        })
    };
    started.wait().await;

    *now.lock().unwrap() = 10_000;
    assert_blocked(
        &click(&manager, Some("thread-b"), 20.0, 20.0)
            .await
            .unwrap_err(),
    );

    finish.resolve();
    in_flight.await.unwrap().unwrap();
    assert_eq!(
        click(&manager, Some("thread-b"), 20.0, 20.0)
            .await
            .unwrap()
            .action,
        "computer_click"
    );

    manager.dispose().await;
}

#[tokio::test]
async fn coalesces_a_burst_of_window_changes_into_a_single_publish_pass() {
    let Built {
        manager, backend, ..
    } = build(
        Scripted::default_fake(),
        Setup {
            windows_publish_debounce_ms: Some(5),
            ..Setup::default()
        },
    );
    manager.list_windows().await.unwrap();
    settle().await;
    manager.get_thread_state("thread-a").await;
    manager.get_thread_state("thread-b").await;

    let events = events(&manager);
    let reads_before = backend.calls_for("listWindows").len();

    for index in 0..20 {
        backend.fake.emit_windows_changed(vec![window(
            "clock",
            &format!("Clock — 12:00:{index}"),
            Some(rect(0.0, 0.0, 100.0, 40.0)),
        )]);
    }
    tokio::time::sleep(Duration::from_millis(40)).await;

    let publishes: Vec<String> = events
        .states()
        .into_iter()
        .map(|state| state.thread_id)
        .collect();
    assert_eq!(publishes, vec!["thread-a", "thread-b"]);
    assert_eq!(backend.calls_for("listWindows").len() - reads_before, 1);

    manager.dispose().await;
}

#[tokio::test]
async fn lets_one_thread_keep_driving_across_a_long_think_and_keeps_perception_free() {
    let (now, clock_fn) = clock(0);
    let Built { manager, .. } = build(
        Scripted::default_fake(),
        Setup {
            now: Some(clock_fn),
            lease_idle_ms: Some(1_000),
            ..Setup::default()
        },
    );

    click(&manager, Some("thread-a"), 10.0, 10.0).await.unwrap();
    *now.lock().unwrap() = 60_000;
    assert_eq!(
        click(&manager, Some("thread-a"), 10.0, 10.0)
            .await
            .unwrap()
            .action,
        "computer_click"
    );
    manager
        .get_state(ComputerStateRequest::default())
        .await
        .unwrap();
    manager.list_windows().await.unwrap();
    assert_eq!(
        click(&manager, Some("thread-a"), 10.0, 10.0)
            .await
            .unwrap()
            .action,
        "computer_click"
    );

    manager.dispose().await;
}

#[tokio::test]
async fn zeron_exclusive_lease_refuses_other_chat_input_until_turn_end_idle_or_removal() {
    let (now, clock_fn) = clock(0);
    let Built { manager, .. } = build(
        background_target_backend(),
        Setup {
            now: Some(clock_fn),
            ..Setup::default()
        },
    );
    let claim_exclusive = |turn: &'static str| {
        let inner = manager.clone();
        let manager = manager.clone();
        async move {
            manager
                .with_agent_activity(
                    "chat-a",
                    move || async move { click(&inner, Some("chat-a"), 10.0, 10.0).await },
                    None,
                    Some(turn),
                    None,
                )
                .await
                .unwrap();
        }
    };
    let assert_b_refused_but_perceives = || {
        let manager = manager.clone();
        async move {
            assert_blocked(
                &click(&manager, Some("chat-b"), 20.0, 20.0)
                    .await
                    .unwrap_err(),
            );
            assert_blocked(
                &manager
                    .click(
                        Some("chat-b"),
                        scoped(420.0, 50.0, "editor-b"),
                        &[],
                        Default::default(),
                    )
                    .await
                    .unwrap_err(),
            );
            assert_blocked(
                &manager
                    .launch_app(
                        Some("chat-b"),
                        "Editor B",
                        &[],
                        0,
                        ComputerLaunchOptions::default(),
                    )
                    .await
                    .unwrap_err(),
            );
            let state = manager
                .get_state(ComputerStateRequest {
                    include_screenshot: Some(true),
                    ..ComputerStateRequest::default()
                })
                .await
                .unwrap();
            assert!(state.screenshot.is_some());
            manager
                .capture_screenshot(ComputerCaptureRequest::Window {
                    window_id: "editor-b".into(),
                    max_dimension: None,
                })
                .await
                .unwrap();
        }
    };
    let b_drives = || {
        let manager = manager.clone();
        async move {
            manager
                .click(
                    Some("chat-b"),
                    scoped(420.0, 50.0, "editor-b"),
                    &[],
                    Default::default(),
                )
                .await
                .unwrap();
            manager
                .release_desktop_control("chat-b", None)
                .await
                .unwrap();
        }
    };

    claim_exclusive("turn-1").await;
    assert_b_refused_but_perceives().await;
    manager
        .release_desktop_control("chat-a", Some("turn-1"))
        .await
        .unwrap();
    b_drives().await;

    claim_exclusive("turn-2").await;
    *now.lock().unwrap() = 299_999;
    assert_b_refused_but_perceives().await;
    *now.lock().unwrap() = 300_000;
    b_drives().await;

    claim_exclusive("turn-3").await;
    assert_b_refused_but_perceives().await;
    manager.handle_thread_removed("chat-a").await;
    b_drives().await;

    manager.dispose().await;
}
