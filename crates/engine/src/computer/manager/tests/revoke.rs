use std::sync::atomic::AtomicUsize;

use tokio_util::sync::CancellationToken;

use super::*;
use crate::computer::approval_gate::{ApprovalPublish, ApprovalRequest};
use crate::computer::desktop_operation_queue::assert_desktop_operation_active;

#[tokio::test]
async fn a_stop_revokes_queued_admission_aborts_the_active_operation_and_lands_no_click() {
    let Built {
        manager, backend, ..
    } = settled(Scripted::default_fake());
    let entered = deferred();
    let release = deferred();
    let active = {
        let (manager, entered, release) = (manager.clone(), entered.clone(), release.clone());
        tokio::spawn(async move {
            let inner = manager.clone();
            manager
                .with_agent_activity(
                    "revoke-thread",
                    move || async move {
                        entered.resolve();
                        release.wait().await;
                        inner
                            .click(
                                Some("revoke-thread"),
                                point(10.0, 10.0),
                                &[],
                                Default::default(),
                            )
                            .await?;
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
                    "revoke-thread",
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
    manager.stop_thread("revoke-thread").await.unwrap();
    release.resolve();
    let queued = queued.await.unwrap().unwrap_err();
    assert!(queued.to_string().contains("revoked"), "{queued}");
    let active = active.await.unwrap().unwrap_err();
    assert!(active.to_string().contains("revoked"), "{active}");
    assert!(!worked.load(Ordering::SeqCst));
    assert!(backend.calls_for("click").is_empty());
    manager.dispose().await;
}

#[tokio::test]
async fn a_late_accept_for_a_prompt_open_at_stop_settles_false() {
    let Built {
        manager, backend, ..
    } = settled(Scripted::default_fake());
    let thread_id = "revoke-gate-thread";
    let opened: Arc<Mutex<Option<String>>> = Arc::default();
    let record = opened.clone();
    let publish: ApprovalPublish = Arc::new(move |request_id, decision| {
        if decision.is_none() {
            *record.lock().unwrap() = Some(request_id);
        }
        async { Ok(()) }.boxed()
    });
    let pending = {
        let gate = manager.approval_gate().clone();
        tokio::spawn(async move {
            gate.request(ApprovalRequest {
                chat_id: thread_id.into(),
                turn_id: None,
                mode: PermissionMode::Ask,
                signal: CancellationToken::new(),
                publish,
                app: None,
            })
            .await
        })
    };
    settle().await;
    let request_id = opened.lock().unwrap().clone().expect("the prompt opened");
    manager.stop_thread(thread_id).await.unwrap();
    assert!(
        !manager
            .approval_gate()
            .respond(thread_id, &request_id, ApprovalDecision::Accept)
    );
    assert!(!pending.await.unwrap().unwrap());
    assert!(backend.calls_for("click").is_empty());
    manager.dispose().await;
}

#[tokio::test]
async fn a_stop_still_stops_input_while_the_first_of_two_overlapping_calls_is_active() {
    let Built {
        manager, backend, ..
    } = settled(Scripted::default_fake());
    let stops = Arc::new(AtomicUsize::new(0));
    let counted = stops.clone();
    backend.hooks().stop_input = Some(Arc::new(move |_, _| {
        counted.fetch_add(1, Ordering::SeqCst);
        async { Ok(()) }.boxed()
    }));
    let entered = deferred();
    let release = deferred();
    let first = {
        let (manager, entered, release) = (manager.clone(), entered.clone(), release.clone());
        tokio::spawn(async move {
            let inner = manager.clone();
            manager
                .with_agent_activity(
                    "overlap-thread",
                    move || async move {
                        entered.resolve();
                        inner
                            .with_agent_activity(
                                "overlap-thread",
                                || async { Ok("inner") },
                                None,
                                None,
                                None,
                            )
                            .await?;
                        release.wait().await;
                        inner
                            .click(
                                Some("overlap-thread"),
                                point(10.0, 10.0),
                                &[],
                                Default::default(),
                            )
                            .await?;
                        Ok("first-finished")
                    },
                    None,
                    None,
                    None,
                )
                .await
        })
    };
    entered.wait().await;
    settle().await;
    manager.stop_thread("overlap-thread").await.unwrap();
    assert_eq!(stops.load(Ordering::SeqCst), 1);
    release.resolve();
    let error = first.await.unwrap().unwrap_err();
    assert!(error.to_string().contains("revoked"), "{error}");
    assert!(backend.calls_for("click").is_empty());
    manager.dispose().await;
}

#[tokio::test]
async fn a_stale_generation_cannot_revive_control_after_stop_and_a_fresh_call_does_not_resurrect_it()
 {
    let Built {
        manager, backend, ..
    } = settled(Scripted::default_fake());
    let thread_id = "stale-generation-thread";
    assert!(manager.can_activate_control(thread_id, 0));
    assert_eq!(manager.stop_thread(thread_id).await.unwrap(), 1);
    assert!(!manager.can_activate_control(thread_id, 0));
    assert!(manager.can_activate_control(thread_id, 1));
    manager
        .with_agent_activity(thread_id, || async { Ok(()) }, None, None, None)
        .await
        .unwrap();
    assert!(!manager.can_activate_control(thread_id, 0));
    assert!(manager.can_activate_control(thread_id, 1));
    assert_eq!(manager.stop_thread(thread_id).await.unwrap(), 2);
    assert!(!manager.can_activate_control(thread_id, 1));
    assert!(manager.can_activate_control(thread_id, 2));
    assert!(backend.calls_for("click").is_empty());
    manager.dispose().await;
}

#[tokio::test]
async fn a_queued_invocation_cannot_slip_the_gap_between_the_stop_and_its_teardown() {
    let Built {
        manager, backend, ..
    } = settled(Scripted::default_fake());
    let thread_id = "stop-race-thread";
    manager
        .click(Some(thread_id), point(10.0, 10.0), &[], Default::default())
        .await
        .unwrap();
    let wedged = deferred();
    let hold = wedged.wait();
    backend.hooks().stop_input = Some(Arc::new(move |_, _| {
        let hold = hold.clone();
        async move {
            hold.await;
            Ok(())
        }
        .boxed()
    }));
    let stopping = {
        let manager = manager.clone();
        tokio::spawn(async move { manager.stop_thread(thread_id).await })
    };
    settle().await;
    assert!(!stopping.is_finished());
    assert!(!manager.can_activate_control(thread_id, 0));
    wedged.resolve();
    assert_eq!(stopping.await.unwrap().unwrap(), 1);
    assert!(!manager.can_activate_control(thread_id, 0));
    manager.dispose().await;
}

#[tokio::test]
async fn a_stop_aborts_live_work_with_a_control_revoked_reason_not_a_bare_abort() {
    let Built { manager, .. } = settled(Scripted::default_fake());
    let entered = deferred();
    let release = deferred();
    let active = {
        let (manager, entered, release) = (manager.clone(), entered.clone(), release.clone());
        tokio::spawn(async move {
            manager
                .with_agent_activity(
                    "reason-thread",
                    move || async move {
                        entered.resolve();
                        release.wait().await;
                        assert_desktop_operation_active()?;
                        Ok(())
                    },
                    None,
                    None,
                    None,
                )
                .await
        })
    };
    entered.wait().await;
    let stopping = {
        let manager = manager.clone();
        tokio::spawn(async move { manager.stop_thread("reason-thread").await })
    };
    settle().await;
    release.resolve();
    stopping.await.unwrap().unwrap();
    let error = active.await.unwrap().unwrap_err();
    assert!(matches!(error, ComputerError::Backend(_)), "{error:?}");
    assert!(error.control_revoked());
    manager.dispose().await;
}
