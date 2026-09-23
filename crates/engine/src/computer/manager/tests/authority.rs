use super::*;

#[tokio::test]
async fn restores_archived_admission_without_reviving_work_or_an_explicit_user_revocation() {
    let Built { manager, .. } = manager(Scripted::default_fake());
    let generation = manager.stop_thread("fixture").await.unwrap();
    assert!(!manager.can_activate_control("fixture", generation - 1));
    manager.handle_thread_removed("fixture").await;
    assert!(!manager.can_activate_control("fixture", generation));
    let error = manager
        .with_agent_activity("fixture", || async { Ok(()) }, None, None, None)
        .await
        .unwrap_err();
    assert!(error.control_revoked());
    assert!(error.to_string().contains("revoked"));

    manager.handle_thread_restored("fixture").await;
    assert!(!manager.can_activate_control("fixture", generation - 1));
    assert!(manager.can_activate_control("fixture", generation));
    assert_eq!(
        manager
            .with_agent_activity("fixture", || async { Ok("new call") }, None, None, None)
            .await
            .unwrap(),
        "new call"
    );
    manager.dispose().await;
}

#[tokio::test]
async fn ignores_an_older_turn_ending_after_the_same_thread_took_a_new_lease() {
    let Built { manager, .. } = manager(Scripted::default_fake());
    let inner = manager.clone();
    manager
        .with_agent_activity(
            "fixture",
            move || async move {
                inner
                    .click(Some("fixture"), point(5.0, 5.0), &[], Default::default())
                    .await
            },
            None,
            Some("turn-new"),
            None,
        )
        .await
        .unwrap();
    manager
        .release_desktop_control("fixture", Some("turn-old"))
        .await
        .unwrap();
    let error = manager
        .click(Some("other"), point(5.0, 5.0), &[], Default::default())
        .await
        .unwrap_err();
    assert_eq!(code(&error), Some("computer_controlled_by_other_thread"));
    manager
        .release_desktop_control("fixture", Some("turn-new"))
        .await
        .unwrap();
    manager
        .click(Some("other"), point(5.0, 5.0), &[], Default::default())
        .await
        .unwrap();
    manager.dispose().await;
}

#[tokio::test]
async fn revokes_queued_admission_and_aborts_the_active_operation() {
    let Built { manager, .. } = manager(Scripted::default_fake());
    let entered = deferred();
    let blocked = deferred();
    let first = {
        let (manager, entered, blocked) = (manager.clone(), entered.clone(), blocked.clone());
        tokio::spawn(async move {
            manager
                .with_agent_activity(
                    "fixture",
                    move || async move {
                        entered.resolve();
                        blocked.wait().await;
                        Ok("ended")
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
                    "fixture",
                    move || async move {
                        worked.store(true, Ordering::SeqCst);
                        Ok("input")
                    },
                    None,
                    None,
                    None,
                )
                .await
        })
    };
    settle().await;
    manager.stop_thread("fixture").await.unwrap();
    blocked.resolve();
    assert_eq!(first.await.unwrap().unwrap(), "ended");
    let error = queued.await.unwrap().unwrap_err();
    assert!(error.to_string().contains("revoked"));
    assert!(!worked.load(Ordering::SeqCst));
    let worked_again = worked.clone();
    assert_eq!(
        manager
            .with_agent_activity(
                "fixture",
                move || async move {
                    worked_again.store(true, Ordering::SeqCst);
                    Ok("input")
                },
                None,
                None,
                None,
            )
            .await
            .unwrap(),
        "input"
    );
    assert!(worked.load(Ordering::SeqCst));
    manager.dispose().await;
}
