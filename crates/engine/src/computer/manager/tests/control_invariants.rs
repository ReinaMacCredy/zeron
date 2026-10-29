use std::time::Duration;

use serde_json::Map;
use tokio_util::sync::CancellationToken;

use super::super::ComputerCaller;
use super::*;
use crate::computer::backend::ComputerBackendActionResult;

#[tokio::test]
async fn hover_does_not_change_keyboard_focus() {
    let Built {
        manager, backend, ..
    } = settled(Scripted::default_fake());
    manager
        .move_cursor(Some("audit"), scoped(1_180.0, 228.0, "fake-calculator"))
        .await
        .unwrap();
    assert!(backend.calls_for("focusWindow").is_empty());
    manager.dispose().await;
}

#[tokio::test]
async fn concurrent_keyboard_calls_preserve_each_named_target() {
    let backend = Scripted::default_fake();
    let entered = deferred();
    let release = deferred();
    let aim: Arc<Mutex<String>> = Arc::default();
    let deliveries: Arc<Mutex<Vec<(String, String)>>> = Arc::default();
    {
        let (aim, entered, release) = (aim.clone(), entered.clone(), release.clone());
        backend.hooks().focus_window = Some(Arc::new(move |_, window_id: String| {
            *aim.lock().unwrap() = window_id.clone();
            let (entered, release) = (entered.clone(), release.clone());
            async move {
                if window_id == "fake-calculator" {
                    entered.resolve();
                    release.wait().await;
                }
                Ok(())
            }
            .boxed()
        }));
    }
    {
        let (aim, deliveries) = (aim.clone(), deliveries.clone());
        backend.hooks().type_text = Some(Arc::new(move |_, (text, _, _)| {
            deliveries
                .lock()
                .unwrap()
                .push((text, aim.lock().unwrap().clone()));
            async { Ok(ComputerBackendActionResult::default()) }.boxed()
        }));
    }
    let Built { manager, .. } = settled(backend);
    manager.list_windows().await.unwrap();
    let first = {
        let manager = manager.clone();
        tokio::spawn(async move {
            manager
                .type_text(Some("audit"), "calculator text", Some("fake-calculator"))
                .await
        })
    };
    entered.wait().await;
    let second = {
        let manager = manager.clone();
        tokio::spawn(async move {
            manager
                .type_text(Some("audit"), "browser text", Some("fake-terminal"))
                .await
        })
    };
    settle().await;
    assert!(deliveries.lock().unwrap().is_empty());
    release.resolve();
    first.await.unwrap().unwrap();
    second.await.unwrap().unwrap();
    manager.dispose().await;
    let deliveries = deliveries.lock().unwrap().clone();
    let calculator = deliveries
        .iter()
        .find(|(text, _)| text == "calculator text")
        .map(|(_, aim)| aim.as_str());
    assert_eq!(calculator, Some("fake-calculator"));
}

#[tokio::test]
async fn aborting_a_call_during_targeting_prevents_its_later_click() {
    let backend = Scripted::default_fake();
    let entered = deferred();
    let release = deferred();
    {
        let (entered, release) = (entered.clone(), release.clone());
        backend.hooks().get_state = Some(Arc::new(move |fake, options| {
            let (entered, release) = (entered.clone(), release.clone());
            async move {
                entered.resolve();
                release.wait().await;
                fake.get_state(options).await
            }
            .boxed()
        }));
    }
    let Built {
        manager, backend, ..
    } = settled(backend);
    let cancel = CancellationToken::new();
    let call = {
        let (manager, cancel) = (manager.clone(), cancel.clone());
        tokio::spawn(async move {
            let inner = manager.clone();
            manager
                .with_agent_activity(
                    "audit",
                    move || async move {
                        inner
                            .click(
                                Some("audit"),
                                labelled("Calculate", Some("button")),
                                &[],
                                Default::default(),
                            )
                            .await
                    },
                    Some(cancel.into()),
                    Some("turn"),
                    None,
                )
                .await
        })
    };
    entered.wait().await;
    cancel.cancel();
    release.resolve();
    let _ = call.await.unwrap();
    tokio::time::sleep(Duration::from_millis(20)).await;
    manager.dispose().await;
    assert!(backend.calls_for("click").is_empty());
}

#[tokio::test]
async fn a_moved_window_delivers_its_new_screenshot_geometry() {
    let Built {
        manager, backend, ..
    } = settled(Scripted::default_fake());
    manager
        .capture_action_screenshot(Some("fake-calculator"), None, true)
        .await
        .unwrap();
    let moved = backend
        .fake
        .list_windows()
        .await
        .unwrap()
        .into_iter()
        .map(|window| match window.bounds {
            Some(bounds) if window.id == "fake-calculator" => ComputerWindow {
                bounds: Some(ComputerRect { x: 800.0, ..bounds }),
                ..window
            },
            _ => window,
        })
        .collect();
    backend.fake.emit_windows_changed(moved);
    let observation = manager
        .capture_action_screenshot(Some("fake-calculator"), None, true)
        .await
        .unwrap();
    manager.dispose().await;
    let region = observation.screenshot().and_then(|shot| shot.region);
    assert_eq!(region.map(|region| region.x), Some(800.0));
}

#[tokio::test]
async fn a_fresh_invocation_re_arms_after_stop_but_an_older_queued_invocation_cannot() {
    let Built { manager, .. } = settled(Scripted::default_fake());
    let generation = manager.stop_thread("audit").await.unwrap();
    assert_eq!(
        manager.get_thread_state("audit").await.control_generation,
        Some(generation)
    );
    assert!(!manager.can_activate_control("audit", 0));
    assert!(manager.can_activate_control("audit", generation));
    manager.dispose().await;
}

#[tokio::test]
async fn rechecks_the_original_turn_after_waiting_for_the_desktop() {
    let Built {
        manager,
        backend,
        seams,
    } = settled(Scripted::default_fake());
    *seams.mode.lock().unwrap() = Some(AccessMode::FullAccess);
    let entered = deferred();
    let release = deferred();
    let first = {
        let (manager, entered, release) = (manager.clone(), entered.clone(), release.clone());
        tokio::spawn(async move {
            manager
                .with_agent_activity(
                    "audit",
                    move || async move {
                        entered.resolve();
                        release.wait().await;
                        Ok(())
                    },
                    None,
                    Some("turn"),
                    None,
                )
                .await
        })
    };
    entered.wait().await;
    let second = {
        let manager = manager.clone();
        tokio::spawn(async move {
            let inner = manager.clone();
            manager
                .with_agent_activity(
                    "audit",
                    move || async move {
                        let caller = ComputerCaller {
                            chat_id: "audit".into(),
                            turn_id: Some("turn".into()),
                        };
                        let approved = inner
                            .authorize_action(
                                "computer_type_text",
                                &Map::new(),
                                &caller,
                                CancellationToken::new(),
                            )
                            .await?;
                        assert!(approved);
                        inner.type_text(Some("audit"), "never", None).await
                    },
                    None,
                    Some("turn"),
                    None,
                )
                .await
        })
    };
    settle().await;
    seams.turn_inactive.store(true, Ordering::SeqCst);
    release.resolve();
    first.await.unwrap().unwrap();
    let error = second.await.unwrap().unwrap_err();
    assert!(error.to_string().contains("original turn ended"), "{error}");
    assert!(backend.calls_for("typeText").is_empty());
    manager.dispose().await;
}
