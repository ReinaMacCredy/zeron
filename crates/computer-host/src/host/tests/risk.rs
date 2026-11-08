use super::*;
use std::os::unix::fs::MetadataExt;
use std::time::Instant;

#[tokio::test]
async fn an_exited_generation_aborts_the_handshake_loop_instead_of_retrying_to_the_limit() {
    let f = fixture(Driver {
        unpatched: true,
        ..Driver::default()
    });
    let started = Instant::now();
    let reply = f.send(call("type_text").args(json!({ "text": "x" }))).await;
    assert_not_dispatched(&reply);
    assert!(
        reply
            .error
            .as_deref()
            .unwrap()
            .contains("stopped during startup"),
        "{reply:?}"
    );
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "{:?}",
        started.elapsed()
    );
}

#[tokio::test]
async fn the_handshake_abandons_a_metadata_reply_slower_than_its_attempt_timeout_and_retries() {
    let f = fixture(Driver {
        metadata_first_delay_ms: Some(600),
        ..Driver::default()
    });
    assert_clean_ok(&f.send(press()).await);
    assert_eq!(f.count("metadata"), 2);
    assert_eq!(f.count("start"), 1);
}

#[tokio::test]
async fn the_handshake_gives_up_after_eighty_unanswered_metadata_attempts_and_retires() {
    let f = fixture(Driver {
        metadata_unanswered: true,
        ..Driver::default()
    });
    let started = Instant::now();
    let reply = f.send(press()).await;
    assert_not_dispatched(&reply);
    assert!(
        reply.error.as_deref().unwrap().contains("handshake failed"),
        "{reply:?}"
    );
    let elapsed = started.elapsed();
    assert!(
        elapsed >= Duration::from_millis(80 * 50) && elapsed < Duration::from_secs(8),
        "{elapsed:?}"
    );
    f.wait_for("exit").await;
    assert!((70..=80).contains(&f.count("metadata")), "{:?}", f.names());
    assert!(!f.has("cancel") && !f.has("key"));
}

#[tokio::test]
async fn a_handshake_with_the_wrong_identity_fails_and_retires_the_generation_without_cancel() {
    let cases = [
        (
            "pid",
            Driver {
                metadata_pid_offset: 1,
                ..Driver::default()
            },
        ),
        (
            "revision",
            Driver {
                reported_revision: Some(38),
                ..Driver::default()
            },
        ),
        (
            "version",
            Driver {
                reported_version: Some("0.28.1".into()),
                ..Driver::default()
            },
        ),
        (
            "embedded",
            Driver {
                not_embedded: true,
                ..Driver::default()
            },
        ),
    ];
    for (label, driver) in cases {
        let f = fixture(driver);
        let reply = f.send(press()).await;
        assert_not_dispatched(&reply);
        assert!(
            reply.error.as_deref().unwrap().contains("handshake failed"),
            "{label}: {reply:?}"
        );
        f.wait_for("exit").await;
        assert_eq!(f.names(), ["start", "retiring", "exit"], "{label}");
    }
}

#[tokio::test]
async fn a_validated_handshake_locks_the_driver_socket_to_the_owner() {
    let f = fixture(Driver {
        log_socket: true,
        ..Driver::default()
    });
    assert!(f.send(press()).await.ok);
    let socket = f
        .names()
        .into_iter()
        .find_map(|e| e.strip_prefix("socket:").map(str::to_string))
        .unwrap();
    let mode = std::fs::metadata(&socket).unwrap().mode() & 0o777;
    assert_eq!(mode, 0o600, "{socket}");
}

#[tokio::test]
async fn an_unpatched_generation_makes_no_cleanup_claim_and_is_terminated_directly() {
    let f = fixture_with(
        Driver {
            unpatched: true,
            ..Driver::default()
        },
        |o| o.native_revision = None,
    );
    assert!(f.send(call("list_windows")).await.ok);
    f.host.stop().await.unwrap();
    assert_eq!(
        f.names(),
        ["start", "motion-100-0", "list-windows", "retiring", "exit"]
    );
}

#[tokio::test]
async fn never_resumes_input_on_an_interrupt_ack_whose_epoch_did_not_advance() {
    let f = fixture(Driver {
        interrupt_cleanup: Some("stale-epoch".into()),
        ..Driver::default()
    });
    f.send(press()).await;
    assert!(!f.send(method(ComputerHostMethod::Stop)).await.ok);
    assert_not_dispatched(&f.send(press()).await);
    assert_eq!(f.count("key"), 1);
}

#[tokio::test]
async fn releases_held_input_captured_at_retire_entry_even_after_cancel_ends_the_action() {
    let releases = Arc::new(AtomicUsize::new(0));
    let seam = counting_release(releases.clone());
    let f = fixture_with(
        Driver {
            cleanup: Some("incomplete".into()),
            ..Driver::default()
        },
        |o| o.release_held_input = Some(seam),
    );
    let active = f.spawn(call("type_text").args(json!({ "text": "fixture" })));
    f.wait_for("dispatch").await;
    let error = f.host.stop().await.unwrap_err();
    assert!(
        error.contains("did not confirm native input cleanup"),
        "{error}"
    );
    assert!(!active.reply().await.ok);
    assert_eq!(releases.load(Ordering::SeqCst), 1);
    assert!(f.has("release"));
    assert!(pid_alive(f.starts()[0].pid));
    assert!(!f.has("retiring"));
}

#[tokio::test]
async fn terminate_sends_sigterm_after_the_stdin_grace() {
    let f = fixture(Driver {
        ignore_stdin_eof: true,
        ..Driver::default()
    });
    assert!(f.send(press()).await.ok);
    let started = Instant::now();
    f.host.stop().await.unwrap();
    let elapsed = started.elapsed();
    assert!(elapsed >= Duration::from_millis(450), "{elapsed:?}");
    assert!(elapsed < Duration::from_millis(1_400), "{elapsed:?}");
    assert!(f.has("retiring") && f.has("exit"));
}

#[tokio::test]
async fn terminate_kills_a_driver_that_ignores_stdin_eof_and_sigterm_then_replaces_it() {
    let f = fixture(Driver {
        ignore_termination: true,
        ..Driver::default()
    });
    assert!(f.send(press()).await.ok);
    let pid = f.starts()[0].pid;
    let started = Instant::now();
    f.host.stop().await.unwrap();
    let elapsed = started.elapsed();
    assert!(elapsed >= Duration::from_millis(1_400), "{elapsed:?}");
    assert!(elapsed < Duration::from_millis(3_500), "{elapsed:?}");
    assert!(!pid_alive(pid));
    assert!(!f.has("retiring"));
    assert!(f.send(press()).await.ok);
    assert_eq!(f.starts().len(), 2);
}

#[tokio::test]
async fn a_dispatch_failure_that_was_not_a_cancel_retires_the_generation() {
    let f = fixture(Driver {
        fail_action: true,
        ..Driver::default()
    });
    let reply = f
        .send(call("type_text").args(json!({ "text": "fixture" })))
        .await;
    assert!(!reply.ok);
    assert_eq!(reply.effect, Some(CuaEffect::DispatchedUnknown));
    assert!(f.has("cleanup-ack") && f.has("retiring"));
    assert!(f.send(press()).await.ok);
    assert_eq!(f.starts().len(), 2);
}

#[tokio::test]
async fn closing_a_caller_before_its_mutation_is_dispatched_sends_no_interrupt() {
    let f = fixture(Driver {
        input_delay_ms: Some(300),
        ..Driver::default()
    });
    let active = f.spawn(call("type_text").args(json!({ "text": "fixture" })));
    f.wait_for("dispatch").await;
    let queued = f.spawn(press());
    f.cancel(&queued.id);
    assert_not_dispatched(&queued.reply().await);
    assert_clean_ok(&active.reply().await);
    assert!(f.has("effect"));
    assert!(!f.has("interrupt") && !f.has("key"));
}

#[tokio::test]
async fn a_late_cancel_for_an_answered_request_is_a_no_op() {
    let f = fixture(Driver::default());
    let answered = f.spawn(press());
    let answered_id = answered.id.clone();
    assert!(answered.reply().await.ok);
    f.cancel(&answered_id);
    f.cancel("never-issued");
    assert!(f.send(press()).await.ok);
    assert!(!f.has("interrupt"));
    assert_eq!(f.count("key"), 2);
}

async fn dead_driver_with_input_in_flight(release_ok: bool) -> (Fixture, Arc<AtomicUsize>) {
    let releases = Arc::new(AtomicUsize::new(0));
    let counter = releases.clone();
    let f = fixture_with(
        Driver {
            action_result: Some(json!({
                "isError": true,
                "structuredContent": { "input_cleanup_unconfirmed": true },
            })),
            ..Driver::default()
        },
        |o| {
            o.release_held_input = Some(Box::new(move || {
                counter.fetch_add(1, Ordering::SeqCst);
                Box::pin(async move {
                    if release_ok {
                        Ok(())
                    } else {
                        Err("helper gone".to_string())
                    }
                })
            }))
        },
    );
    let uncertain = f.send(press()).await;
    assert_eq!(
        structured(&uncertain)["input_cleanup_unconfirmed"],
        json!(true)
    );
    let pid = f.starts()[0].pid;
    unsafe { libc::kill(pid, libc::SIGKILL) };
    for _ in 0..300 {
        if !pid_alive(pid) {
            break;
        }
        sleep_ms(10).await;
    }
    assert!(!pid_alive(pid));
    (f, releases)
}

#[tokio::test]
async fn a_driver_found_dead_with_input_in_flight_closes_admission_when_the_release_fails() {
    let (f, releases) = dead_driver_with_input_in_flight(false).await;
    let error = f.host.stop().await.unwrap_err();
    assert!(error.contains("admission is closed"), "{error}");
    assert_eq!(releases.load(Ordering::SeqCst), 1);
    assert!(!f.has("cancel"));
    assert_not_dispatched(&f.send(call("check_permissions")).await);
    assert_eq!(f.starts().len(), 1);
}

#[tokio::test]
async fn a_driver_found_dead_with_input_in_flight_is_replaced_after_a_confirmed_release() {
    let (f, releases) = dead_driver_with_input_in_flight(true).await;
    f.host.stop().await.unwrap();
    assert_eq!(releases.load(Ordering::SeqCst), 1);
    assert!(!f.has("cancel"));
    assert!(f.send(call("check_permissions")).await.ok);
    assert_eq!(f.starts().len(), 2);
}
