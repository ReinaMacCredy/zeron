use std::sync::atomic::AtomicUsize;
use std::time::Duration;

use super::*;
use crate::computer::desktop_operation_queue::desktop_operation_signal;

fn count_stops(backend: &Scripted) -> Arc<AtomicUsize> {
    let stops = Arc::new(AtomicUsize::new(0));
    let counted = stops.clone();
    backend.hooks().stop_input = Some(Arc::new(move |_, _| {
        counted.fetch_add(1, Ordering::SeqCst);
        async { Ok(()) }.boxed()
    }));
    stops
}

fn input_stopped(events: &Events, stopped: bool) -> usize {
    events
        .all()
        .iter()
        .filter(|event| {
            matches!(**event, ComputerEvent::InputStopped { stopped: seen } if seen == stopped)
        })
        .count()
}

#[tokio::test]
async fn interrupts_live_work_once_and_the_next_action_succeeds_without_a_re_arm() {
    let Built {
        manager, backend, ..
    } = settled(Scripted::default_fake());
    let stops = count_stops(&backend);
    let events = events(&manager);
    manager.get_thread_state("esc-thread").await;
    manager
        .click(
            Some("esc-thread"),
            point(10.0, 10.0),
            &[],
            Default::default(),
        )
        .await
        .unwrap();
    let entered = deferred();
    let live = {
        let (manager, entered) = (manager.clone(), entered.clone());
        tokio::spawn(async move {
            manager
                .with_agent_activity(
                    "esc-thread",
                    move || async move {
                        entered.resolve();
                        let signal = desktop_operation_signal().expect("an operation signal");
                        while !signal.is_aborted() {
                            tokio::time::sleep(Duration::from_millis(1)).await;
                        }
                        signal.check()?;
                        Ok("unreachable")
                    },
                    None,
                    None,
                    None,
                )
                .await
        })
    };
    entered.wait().await;
    manager.emergency_stop_input().await.unwrap();
    assert_eq!(stops.load(Ordering::SeqCst), 1);
    let error = live.await.unwrap().unwrap_err();
    assert!(error.control_revoked(), "{error}");
    assert_eq!(input_stopped(&events, true), 1);
    assert_eq!(input_stopped(&events, false), 1);
    manager
        .click(
            Some("esc-thread"),
            point(11.0, 11.0),
            &[],
            Default::default(),
        )
        .await
        .unwrap();
    assert_eq!(backend.calls_for("click").len(), 2);
    assert_eq!(manager.get_status().await.input_stopped, None);
    assert_eq!(
        manager.get_thread_state("esc-thread").await.input_stopped,
        None
    );
    manager.dispose().await;
}

#[tokio::test]
async fn fails_queued_work_at_its_wait_then_admits_the_next_call() {
    let Built {
        manager, backend, ..
    } = settled(Scripted::default_fake());
    count_stops(&backend);
    let entered = deferred();
    let release = deferred();
    let active = {
        let (manager, entered, release) = (manager.clone(), entered.clone(), release.clone());
        tokio::spawn(async move {
            manager
                .with_agent_activity(
                    "esc-queued",
                    move || async move {
                        entered.resolve();
                        release.wait().await;
                        Ok("active-finished")
                    },
                    None,
                    None,
                    None,
                )
                .await
        })
    };
    entered.wait().await;
    let worked = Arc::new(AtomicBool::new(false));
    let queued = {
        let (manager, worked) = (manager.clone(), worked.clone());
        tokio::spawn(async move {
            manager
                .with_agent_activity(
                    "esc-queued",
                    move || async move {
                        worked.store(true, Ordering::SeqCst);
                        Ok("queued")
                    },
                    None,
                    None,
                    None,
                )
                .await
        })
    };
    settle().await;
    manager.emergency_stop_input().await.unwrap();
    release.resolve();
    let error = queued.await.unwrap().unwrap_err();
    assert!(error.to_string().contains("Escape"), "{error}");
    assert_eq!(active.await.unwrap().unwrap(), "active-finished");
    assert!(!worked.load(Ordering::SeqCst));
    assert!(backend.calls_for("click").is_empty());
    manager
        .click(Some("esc-queued"), point(1.0, 1.0), &[], Default::default())
        .await
        .unwrap();
    assert_eq!(backend.calls_for("click").len(), 1);
    manager.dispose().await;
}

#[tokio::test]
async fn does_not_latch_when_the_backend_stop_relay_fails() {
    let Built {
        manager, backend, ..
    } = settled(Scripted::default_fake());
    let stops = Arc::new(AtomicUsize::new(0));
    let counted = stops.clone();
    backend.hooks().stop_input = Some(Arc::new(move |_, _| {
        let first = counted.fetch_add(1, Ordering::SeqCst) == 0;
        async move {
            if first {
                Err(ComputerError::Other("backend wedged".into()))
            } else {
                Ok(())
            }
        }
        .boxed()
    }));
    let error = manager.emergency_stop_input().await.unwrap_err();
    assert!(error.to_string().contains("backend wedged"), "{error}");
    manager
        .click(Some("esc-thread"), point(1.0, 1.0), &[], Default::default())
        .await
        .unwrap();
    assert_eq!(backend.calls_for("click").len(), 1);
    manager.dispose().await;
}

#[tokio::test]
async fn treats_a_pane_escape_as_ordinary_input_never_as_a_stop() {
    let Built {
        manager, backend, ..
    } = settled(Scripted::default_fake());
    let stops = count_stops(&backend);
    let events = events(&manager);
    manager.press_key(None, "escape", None, None).await.unwrap();
    let keys: Vec<Value> = backend
        .calls_for("pressKey")
        .into_iter()
        .map(|call| call.args[0].clone())
        .collect();
    assert_eq!(keys, vec![json!("escape")]);
    assert_eq!(stops.load(Ordering::SeqCst), 0);
    assert_eq!(
        input_stopped(&events, true) + input_stopped(&events, false),
        0
    );
    manager.dispose().await;
}

#[tokio::test]
async fn keeps_reads_open_through_the_interrupt() {
    let Built {
        manager, backend, ..
    } = settled(Scripted::default_fake());
    count_stops(&backend);
    manager.emergency_stop_input().await.unwrap();
    manager.list_windows().await.unwrap();
    manager.get_status().await;
    manager.dispose().await;
}
