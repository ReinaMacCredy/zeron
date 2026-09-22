use super::*;

#[tokio::test]
async fn starts_the_compact_cursor_once_per_generation_and_owns_the_observation_budget() {
    let f = fixture(Driver::default());
    for _ in 0..2 {
        let reply = f
            .send(
                call("press_key")
                    .args(json!({ "key": "enter", "_synara_foreground_observation_ms": 0 })),
            )
            .await;
        assert!(reply.ok, "{reply:?}");
        assert_eq!(reply.host_platform.as_deref(), Some("darwin"));
    }
    assert_eq!(f.count("motion-100-0"), 1);
    assert_eq!(f.count("observation-budget-100"), 2);
}

async fn does_not_wait_for_another_features_permission_dialog(stop: &str) {
    let entered = Deferred::new();
    let pending = Deferred::new();
    let seam = pending_permissions(entered.clone(), pending.clone());
    let f = fixture_with(Driver::default(), |o| o.check_permissions = Some(seam));
    let check = f.spawn(call("check_permissions"));
    entered.wait().await;
    match stop {
        "stop" => f.host.stop().await,
        "suspend" => f.host.suspend().await,
        _ => f.host.pause_desktop("screen-lock").await,
    }
    .unwrap();
    assert_not_dispatched(&check.reply().await);
    pending.resolve(permissions(true));
    assert!(f.events().is_empty());
}

#[tokio::test]
async fn stop_does_not_wait_for_another_features_permission_dialog() {
    does_not_wait_for_another_features_permission_dialog("stop").await;
}

#[tokio::test]
async fn suspend_does_not_wait_for_another_features_permission_dialog() {
    does_not_wait_for_another_features_permission_dialog("suspend").await;
}

#[tokio::test]
async fn pause_desktop_does_not_wait_for_another_features_permission_dialog() {
    does_not_wait_for_another_features_permission_dialog("pause").await;
}

#[tokio::test]
async fn releases_a_disconnected_permission_check_without_retiring_a_later_native_session() {
    let entered = Deferred::new();
    let pending = Deferred::new();
    let seam = pending_permissions(entered.clone(), pending.clone());
    let f = fixture_with(Driver::default(), |o| o.check_permissions = Some(seam));
    let check = f.spawn(call("check_permissions"));
    entered.wait().await;
    f.cancel(&check.id);
    check.reply().await;
    assert!(f.send(call("get_screen_size")).await.ok);
    pending.resolve(permissions(true));
    assert!(f.send(call("press_key")).await.ok);
    assert_eq!(f.starts().len(), 1);
}

#[tokio::test]
async fn checks_permissions_through_the_fresh_shared_helper_without_starting_cua() {
    let granted = Arc::new(Mutex::new(permissions(false)));
    let seam = fixed_permissions(granted.clone());
    let f = fixture_with(Driver::default(), |o| o.check_permissions = Some(seam));
    let check = || call("check_permissions").args(json!({ "prompt": true }));
    let denied = f.send(check()).await;
    assert_eq!(
        structured(&denied),
        json!({
            "accessibility": false,
            "screen_recording": false,
            "source": {
                "attribution": "host",
                "host_bundle_id": "fixture",
                "probe": "appsnap-permission-helper",
            },
        })
    );
    *granted.lock().unwrap() = permissions(true);
    let allowed = f.send(check()).await;
    assert_eq!(structured(&allowed)["accessibility"], json!(true));
    assert_eq!(structured(&allowed)["screen_recording"], json!(true));
    assert!(f.events().is_empty());
}

#[tokio::test]
async fn requires_an_uncached_confirming_read_before_accepting_a_changed_permission_snapshot() {
    let granted = Arc::new(Mutex::new(false));
    let checks = Arc::new(Mutex::new(Vec::new()));
    let (seen, reads) = (granted.clone(), checks.clone());
    let f = fixture_with(Driver::default(), |o| {
        o.check_permissions = Some(Box::new(move |force| {
            reads.lock().unwrap().push(force);
            let granted = *seen.lock().unwrap();
            Box::pin(async move { Ok(permissions(granted)) })
        }))
    });
    f.send(call("check_permissions")).await;
    *granted.lock().unwrap() = true;
    f.send(call("check_permissions")).await;
    assert_eq!(*checks.lock().unwrap(), vec![false, false, true]);
}

#[tokio::test]
async fn retires_a_cached_native_process_once_when_grants_change_then_requires_fresh_observation() {
    let granted = Arc::new(Mutex::new(permissions(true)));
    let seam = fixed_permissions(granted.clone());
    let f = fixture_with(Driver::default(), |o| o.check_permissions = Some(seam));
    f.send(call("check_permissions")).await;
    f.send(call("get_screen_size")).await;
    *granted.lock().unwrap() = permissions(false);
    let changed = f.send(call("check_permissions")).await;
    assert_eq!(changed.desktop_epoch, Some(1));
    assert_eq!(structured(&changed)["accessibility"], json!(false));
    assert_eq!(f.count("cleanup-ack"), 1);
    f.send(call("check_permissions")).await;
    assert_eq!(f.starts().len(), 1);
    *granted.lock().unwrap() = permissions(true);
    f.send(call("check_permissions")).await;
    assert!(result_is_error(&f.send(call("press_key")).await));
    f.send(call("get_window_state").observe(true)).await;
    assert!(f.send(call("press_key")).await.ok);
    assert_eq!(f.starts().len(), 2);
}

#[tokio::test]
async fn does_not_bypass_failed_cleanup_when_refreshed_permissions_change() {
    let granted = Arc::new(Mutex::new(permissions(true)));
    let seam = fixed_permissions(granted.clone());
    let f = fixture_with(
        Driver {
            cleanup: Some("incomplete".into()),
            ..Driver::default()
        },
        |o| o.check_permissions = Some(seam),
    );
    f.send(call("check_permissions")).await;
    f.send(call("get_screen_size")).await;
    f.send(press()).await;
    *granted.lock().unwrap() = permissions(false);
    assert!(!f.send(call("check_permissions")).await.ok);
    assert!(!f.send(call("get_window_state").observe(true)).await.ok);
    assert_eq!(f.starts().len(), 1);
}

#[tokio::test]
async fn does_not_start_while_locked_and_requires_fresh_state_after_all_desktop_pauses_end() {
    let f = fixture(Driver::default());
    f.host.pause_desktop("screen-lock").await.unwrap();
    f.host.pause_desktop("system-sleep").await.unwrap();
    f.host.resume();
    let paused = f.send(call("press_key")).await;
    assert_paused(&paused);
    assert_eq!(structured(&paused)["effect"], json!("refused"));
    assert!(f.events().is_empty());
    f.host.resume_desktop("screen-lock");
    assert!(result_is_error(&f.send(call("press_key")).await));
    f.host.resume_desktop("system-sleep");
    f.send(call("check_permissions")).await;
    assert!(result_is_error(&f.send(call("press_key")).await));
    f.send(
        call("get_window_state")
            .observe(true)
            .args(json!({ "pid": 1, "window_id": 2 })),
    )
    .await;
    assert!(f.send(call("press_key")).await.ok);
    assert_eq!(f.count("key"), 1);
}

#[tokio::test]
async fn piggybacks_sorted_pauses_and_the_never_reset_interruption_count_on_every_reply() {
    let f = fixture(Driver::default());
    let probe = || method(ComputerHostMethod::Probe);
    let first = f.send(probe()).await;
    assert_eq!(first.desktop_epoch, Some(0));
    assert_eq!(first.desktop_pauses, Some(vec![]));
    assert_eq!(first.desktop_interruptions, Some(0));
    f.host.pause_desktop("system-sleep").await.unwrap();
    f.host.pause_desktop("screen-lock").await.unwrap();
    let paused = f.send(probe()).await;
    let both = vec!["screen-lock".to_string(), "system-sleep".to_string()];
    assert_eq!(paused.desktop_pauses, Some(both.clone()));
    assert_eq!(paused.desktop_interruptions, Some(2));
    let refused = f.send(call("press_key")).await;
    assert_eq!(refused.desktop_pauses, Some(both));
    assert_eq!(refused.desktop_interruptions, Some(2));
    assert_eq!(code(&refused).as_deref(), Some("computer_input_paused"));
    f.host.resume_desktop("screen-lock");
    let one = f.send(probe()).await;
    assert_eq!(one.desktop_pauses, Some(vec!["system-sleep".to_string()]));
    assert_eq!(one.desktop_interruptions, Some(2));
    f.host.resume_desktop("system-sleep");
    let none = f.send(probe()).await;
    assert_eq!(none.desktop_pauses, Some(vec![]));
    assert_eq!(none.desktop_interruptions, Some(2));
}

#[tokio::test]
async fn retires_a_driver_ended_session_and_retries_once_with_a_fresh_one() {
    let f = fixture(Driver {
        session_death_once: true,
        ..Driver::default()
    });
    assert_clean_ok(&f.send(press()).await);
    assert_eq!(f.starts().len(), 2);
    assert_eq!(f.count("key"), 1);
}

#[tokio::test]
async fn retires_a_transport_reported_session_death_and_retries_once_fresh() {
    let f = fixture(Driver {
        session_death_transport: true,
        ..Driver::default()
    });
    assert_clean_ok(&f.send(press()).await);
    assert_eq!(f.starts().len(), 2);
    assert_eq!(f.count("key"), 1);
}

#[tokio::test]
async fn locking_cancels_active_native_input_and_rejects_waiting_input_before_dispatch() {
    let f = fixture(Driver::default());
    let active = f.spawn(call("type_text").args(json!({ "text": "fixture" })));
    f.wait_for("dispatch").await;
    let queued = f.spawn(call("press_key"));
    f.host.pause_desktop("screen-lock").await.unwrap();
    assert!(!active.reply().await.ok);
    let queued = queued.reply().await;
    assert!(!queued.ok || result_is_error(&queued), "{queued:?}");
    assert!(f.has("cleanup-ack"));
    assert!(!f.has("effect") && !f.has("key"));
}

#[tokio::test]
async fn unlock_does_not_bypass_an_unacknowledged_cleanup_barrier() {
    let f = fixture(Driver {
        cleanup: Some("incomplete".into()),
        ..Driver::default()
    });
    f.send(call("check_permissions")).await;
    f.send(press()).await;
    let error = f.host.pause_desktop("screen-lock").await.unwrap_err();
    assert!(
        error.contains("did not confirm native input cleanup"),
        "{error}"
    );
    f.host.resume_desktop("screen-lock");
    assert!(!f.send(call("get_window_state")).await.ok);
    assert_eq!(f.starts().len(), 1);
}

#[tokio::test]
async fn preview_and_readiness_reads_cannot_release_the_post_unlock_model_observation_gate() {
    let f = fixture(Driver::default());
    f.host.pause_desktop("screen-lock").await.unwrap();
    f.host.resume_desktop("screen-lock");
    let preview = f.send(call("get_desktop_state")).await;
    assert!(preview.ok);
    assert_eq!(preview.desktop_epoch, Some(1));
    f.send(call("get_window_state")).await;
    f.send(
        call("get_window_state")
            .observe(true)
            .args(json!({ "empty": true })),
    )
    .await;
    assert!(result_is_error(&f.send(call("check_input_ready")).await));
    assert!(result_is_error(&f.send(call("press_key")).await));
    f.send(call("get_window_state").observe(true)).await;
    assert!(f.send(call("press_key")).await.ok);
    assert_eq!(f.count("key"), 1);
}

#[tokio::test]
async fn a_disconnected_observation_cannot_release_the_post_unlock_gate() {
    let f = fixture(Driver {
        delay_observation: true,
        ..Driver::default()
    });
    f.host.pause_desktop("screen-lock").await.unwrap();
    f.host.resume_desktop("screen-lock");
    let observation = f.spawn(call("get_window_state").observe(true));
    f.wait_for("observe").await;
    f.cancel(&observation.id);
    assert!(!observation.reply().await.ok);
    sleep_ms(100).await;
    assert!(result_is_error(&f.send(call("press_key")).await));
    assert!(!f.has("key"));
}

#[tokio::test]
async fn ignores_a_permission_probe_that_reverts_on_the_confirming_re_read() {
    let probes = Arc::new(AtomicUsize::new(0));
    let counter = probes.clone();
    let f = fixture_with(Driver::default(), |o| {
        o.check_permissions = Some(Box::new(move |_| {
            let probe = counter.fetch_add(1, Ordering::SeqCst) + 1;
            Box::pin(async move {
                Ok(HostPermissions {
                    accessibility: true,
                    screen_recording: probe != 2,
                    input_monitoring: None,
                })
            })
        }))
    });
    assert_eq!(
        f.send(call("check_permissions")).await.desktop_epoch,
        Some(0)
    );
    let second = f.send(call("check_permissions")).await;
    assert_eq!(second.desktop_epoch, Some(0));
    assert_eq!(structured(&second)["screen_recording"], json!(true));
    assert_eq!(probes.load(Ordering::SeqCst), 3);
    assert!(f.send(call("press_key")).await.ok);
}

#[tokio::test]
async fn a_flapping_permission_helper_cannot_deadlock_input_behind_the_observation_gate() {
    let probes = Arc::new(AtomicUsize::new(0));
    let counter = probes.clone();
    let f = fixture_with(Driver::default(), |o| {
        o.check_permissions = Some(Box::new(move |_| {
            let probe = counter.fetch_add(1, Ordering::SeqCst) + 1;
            Box::pin(async move {
                Ok(HostPermissions {
                    accessibility: true,
                    screen_recording: probe % 2 == 1,
                    input_monitoring: None,
                })
            })
        }))
    });
    let observe = || {
        call("get_window_state")
            .observe(true)
            .args(json!({ "pid": 1, "window_id": 2 }))
    };
    f.send(call("check_permissions")).await;
    f.host.pause_desktop("screen-lock").await.unwrap();
    f.host.resume_desktop("screen-lock");
    for _ in 0..3 {
        f.send(observe()).await;
        f.send(call("check_permissions")).await;
        assert!(f.send(call("press_key")).await.ok);
    }
}

#[tokio::test]
async fn refuses_an_observation_a_stop_interrupted_instead_of_silently_voiding_the_clear() {
    let f = fixture(Driver {
        delay_observation: true,
        ..Driver::default()
    });
    f.host.pause_desktop("screen-lock").await.unwrap();
    f.host.resume_desktop("screen-lock");
    let observe = || {
        call("get_window_state")
            .observe(true)
            .args(json!({ "pid": 1, "window_id": 2 }))
    };
    let interrupted = f.spawn(observe());
    f.wait_for("observe").await;
    f.send(method(ComputerHostMethod::Stop)).await;
    assert_paused(&interrupted.reply().await);
    f.send(observe()).await;
    assert!(f.send(call("press_key")).await.ok);
}

#[tokio::test]
async fn releases_uncertain_input_before_termination_and_waits_for_exit_before_replacement() {
    let f = fixture(Driver::default());
    f.send(call("check_permissions").args(json!({}))).await;
    let active = f.spawn(call("type_text").args(json!({ "text": "fixture" })));
    f.wait_for("dispatch").await;
    f.host.stop().await.unwrap();
    assert!(!active.reply().await.ok);
    assert!(f.send(call("check_permissions").args(json!({}))).await.ok);
    let starts = f.starts();
    assert_eq!(starts.len(), 2);
    let events = f.events();
    let exit = events
        .iter()
        .find(|e| e.event == "exit" && e.pid == starts[0].pid)
        .unwrap();
    assert!(starts[1].time >= exit.time);
    assert!(!f.has("effect"));
    assert_eq!(f.count("dispatch"), 1);
    assert_eq!(
        f.pid_events(starts[0].pid),
        [
            "start",
            "motion-100-0",
            "dispatch",
            "cancel",
            "release",
            "cleanup-ack",
            "retiring",
            "exit"
        ]
    );
}

#[tokio::test]
async fn releases_held_input_through_the_helper_when_the_driver_dies_mid_action() {
    let releases = Arc::new(AtomicUsize::new(0));
    let seam = counting_release(releases.clone());
    let f = fixture_with(
        Driver {
            crash: true,
            ..Driver::default()
        },
        |o| o.release_held_input = Some(seam),
    );
    f.send(call("check_permissions")).await;
    assert!(
        !f.send(call("type_text").args(json!({ "text": "fixture" })))
            .await
            .ok
    );
    assert_eq!(releases.load(Ordering::SeqCst), 1);
    assert!(f.send(call("check_permissions")).await.ok);
    assert_eq!(f.starts().len(), 2);
}

#[tokio::test]
async fn keeps_admission_closed_for_the_hosts_lifetime_when_held_input_release_fails() {
    let f = fixture_with(
        Driver {
            crash: true,
            ..Driver::default()
        },
        |o| {
            o.release_held_input = Some(Box::new(|| {
                Box::pin(async { Err("helper gone".to_string()) })
            }))
        },
    );
    assert!(
        !f.send(call("type_text").args(json!({ "text": "fixture" })))
            .await
            .ok
    );
    assert_not_dispatched(&f.send(call("check_permissions")).await);
    assert_eq!(f.starts().len(), 1);
    let error = f.host.stop().await.unwrap_err();
    assert!(error.contains("admission is closed"), "{error}");
}

#[tokio::test]
async fn replaces_a_driver_that_wedges_during_startup_instead_of_closing_admission() {
    let f = fixture_with(
        Driver {
            hang_session: true,
            drop_cancel: true,
            ..Driver::default()
        },
        |o| o.startup_timeout = Duration::from_millis(150),
    );
    assert_not_dispatched(&f.send(press()).await);
    assert!(f.send(call("check_permissions")).await.ok);
    let starts = f.starts();
    assert_eq!(starts.len(), 2);
    assert_eq!(
        f.pid_events(starts[0].pid),
        ["start", "session-hang", "cancel", "retiring", "exit"]
    );
}

#[tokio::test]
async fn rejects_later_backend_requests_throughout_suspension_and_resumes_only_on_explicit_restart()
{
    let f = fixture(Driver::default());
    f.send(call("check_permissions")).await;
    let stopping = f.host.suspend();
    let refused = f
        .send(call("type_text").args(json!({ "text": "must not arrive" })))
        .await;
    assert_not_dispatched(&refused);
    assert!(refused.error.as_deref().unwrap().contains("suspended"));
    stopping.await.unwrap();
    assert_not_dispatched(&f.send(call("check_permissions")).await);
    assert_eq!(f.starts().len(), 1);
    assert!(!f.has("dispatch"));
    f.host.resume();
    assert!(f.send(call("check_permissions")).await.ok);
    let starts = f.starts();
    assert_eq!(starts.len(), 2);
    let events = f.events();
    let exit = events
        .iter()
        .find(|e| e.event == "exit" && e.pid == starts[0].pid)
        .unwrap();
    assert!(starts[1].time >= exit.time);
}

#[tokio::test]
async fn does_not_let_resume_bypass_failed_cleanup_during_backend_suspension() {
    let f = fixture(Driver {
        cleanup: Some("incomplete".into()),
        ..Driver::default()
    });
    f.send(call("check_permissions")).await;
    f.send(press()).await;
    let error = f.host.suspend().await.unwrap_err();
    assert!(
        error.contains("did not confirm native input cleanup"),
        "{error}"
    );
    f.host.resume();
    assert_not_dispatched(
        &f.send(call("type_text").args(json!({ "text": "must not arrive" })))
            .await,
    );
    assert_eq!(f.starts().len(), 1);
    assert!(!f.has("dispatch"));
}

#[tokio::test]
async fn keeps_the_process_alive_and_blocks_replacement_after_an_unacknowledged_cleanup() {
    for cleanup in ["incomplete", "wrong-pid", "missing-admission"] {
        let f = fixture(Driver {
            cleanup: Some(cleanup.into()),
            ..Driver::default()
        });
        f.send(call("check_permissions")).await;
        f.send(press()).await;
        let error = f.host.stop().await.unwrap_err();
        assert!(
            error.contains("did not confirm native input cleanup"),
            "{cleanup}: {error}"
        );
        assert_not_dispatched(&f.send(call("check_permissions")).await);
        assert_eq!(
            f.names(),
            [
                "start",
                "motion-100-0",
                "key",
                "observation-budget-100",
                "cancel",
                "cleanup-ack"
            ],
            "{cleanup}"
        );
        assert!(pid_alive(f.starts()[0].pid), "{cleanup}");
    }
}

#[tokio::test]
async fn preserves_an_uncertain_action_effect_when_cleanup_also_fails() {
    let f = fixture(Driver {
        cleanup: Some("incomplete".into()),
        fail_action: true,
        ..Driver::default()
    });
    let reply = f
        .send(call("type_text").args(json!({ "text": "fixture" })))
        .await;
    assert!(!reply.ok);
    assert_eq!(reply.effect, Some(CuaEffect::DispatchedUnknown));
    assert!(
        reply
            .error
            .as_deref()
            .unwrap()
            .contains("did not confirm native input cleanup"),
        "{reply:?}"
    );
    assert!(!f.has("retiring"));
}

#[tokio::test]
async fn blocks_replacement_when_a_driver_crashes_during_input() {
    let f = fixture(Driver {
        crash: true,
        ..Driver::default()
    });
    let reply = f
        .send(call("type_text").args(json!({ "text": "fixture" })))
        .await;
    assert!(!reply.ok);
    assert_eq!(reply.effect, Some(CuaEffect::DispatchedUnknown));
    assert_not_dispatched(&f.send(call("check_permissions")).await);
    assert_eq!(f.starts().len(), 1);
}

#[tokio::test]
async fn rejects_an_upstream_binary_before_native_input_is_admitted() {
    let f = fixture(Driver {
        unpatched: true,
        ..Driver::default()
    });
    assert_not_dispatched(
        &f.send(call("type_text").args(json!({ "text": "fixture" })))
            .await,
    );
    assert_eq!(f.names(), ["start"]);
}

#[tokio::test]
async fn drives_an_unpatched_upstream_binary_when_no_native_revision_is_expected() {
    let f = fixture_with(
        Driver {
            unpatched: true,
            ..Driver::default()
        },
        |o| o.native_revision = None,
    );
    let reply = f.send(call("list_windows").args(json!({}))).await;
    assert!(reply.ok);
    assert_eq!(reply.driver_native_revision, Some(0));
    assert_eq!(f.names(), ["start", "motion-100-0", "list-windows"]);
}

#[tokio::test]
async fn refuses_unattributed_browser_operations_before_starting_a_daemon() {
    let f = fixture(Driver::default());
    let reply = f
        .send(call("browser_navigate").args(json!({ "url": "https://example.com" })))
        .await;
    assert!(!reply.ok);
    assert!(f.events().is_empty());
}

#[tokio::test]
async fn refuses_unlisted_driver_operations_before_starting_a_daemon() {
    let f = fixture(Driver::default());
    for name in ["start_session", "set_config", "page"] {
        let reply = f.send(call(name)).await;
        assert!(!reply.ok);
        assert_eq!(
            reply.error.as_deref(),
            Some("Unsupported computer host request.")
        );
    }
    assert!(f.events().is_empty());
}

#[tokio::test]
async fn admits_wait_for_settle_as_a_read_through_the_allowlist() {
    let f = fixture(Driver::default());
    let reply = f
        .send(
            call("wait_for_settle")
                .args(json!({ "pid": 42, "window_id": 10, "timeout_ms": 5000, "quiet_ms": 1000 })),
        )
        .await;
    assert!(reply.ok);
    assert!(f.has("start"));
}
