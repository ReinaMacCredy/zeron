use crate::computer::desktop_operation_queue::current_desktop_operation;

use super::*;

#[tokio::test]
async fn refuses_detached_input_after_its_original_operation_ends_while_allowing_a_fresh_admitted_call()
 {
    let Built {
        manager, backend, ..
    } = manager(Scripted::default_fake());
    let release = deferred();
    let detached = manager
        .with_agent_activity(
            "owner",
            || async {
                let scope = current_desktop_operation();
                let manager = manager.clone();
                let release = release.wait();
                Ok(tokio::spawn(scope.run(async move {
                    release.await;
                    manager.type_text(Some("owner"), "stale input", None).await
                })))
            },
            None,
            Some("old-turn"),
            None,
        )
        .await
        .unwrap();
    release.resolve();
    let error = detached.await.unwrap().unwrap_err();
    assert!(error.to_string().contains("operation has ended"), "{error}");
    assert!(backend.logged("typeText").is_empty());
    manager
        .with_agent_activity(
            "owner",
            || manager.type_text(Some("owner"), "fresh input", None),
            None,
            Some("new-turn"),
            None,
        )
        .await
        .unwrap();
    assert_eq!(backend.logged("typeText").len(), 1);
    manager.dispose().await;
}

#[tokio::test]
async fn never_re_admits_a_detached_tool_continuation_after_revocation_and_re_arm() {
    let Built {
        manager, backend, ..
    } = manager(Scripted::default_fake());
    let release = deferred();
    let detached = manager
        .with_agent_activity(
            "owner",
            || async {
                let scope = current_desktop_operation();
                let manager = manager.clone();
                let release = release.wait();
                Ok(tokio::spawn(scope.run(async move {
                    release.await;
                    manager
                        .with_agent_activity(
                            "owner",
                            || manager.type_text(Some("owner"), "stale input", None),
                            None,
                            None,
                            None,
                        )
                        .await
                })))
            },
            None,
            None,
            None,
        )
        .await
        .unwrap();
    manager.stop_thread("owner").await.unwrap();
    manager
        .with_agent_activity("owner", || async { Ok(()) }, None, None, None)
        .await
        .unwrap();
    release.resolve();
    let error = detached.await.unwrap().unwrap_err();
    assert!(error.to_string().contains("operation has ended"), "{error}");
    assert!(backend.logged("typeText").is_empty());
    manager.dispose().await;
}

#[tokio::test]
async fn a_fresh_call_waits_for_a_stop_to_drain_before_it_dispatches() {
    let Built {
        manager, backend, ..
    } = manager(Scripted::default_fake());
    manager
        .click(Some("owner"), point(10.0, 10.0), &[], Default::default())
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
        tokio::spawn(async move { manager.stop_thread("owner").await })
    };
    settle().await;
    let fresh = {
        let manager = manager.clone();
        tokio::spawn(async move {
            manager
                .with_agent_activity(
                    "owner",
                    || manager.type_text(Some("owner"), "after stop", None),
                    None,
                    Some("next-turn"),
                    None,
                )
                .await
        })
    };
    settle().await;
    assert!(backend.logged("typeText").is_empty());
    wedged.resolve();
    stopping.await.unwrap().unwrap();
    fresh.await.unwrap().unwrap();
    assert_eq!(backend.logged("typeText").len(), 1);
    manager.dispose().await;
}
