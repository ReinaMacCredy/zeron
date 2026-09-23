#[allow(unused_imports)]
use super::*;

use std::sync::atomic::AtomicUsize;
use std::time::Duration;

use tokio::time::Instant;
use zeron_proto::computer::{
    ComputerDeliveryVerification, ComputerFrame, ComputerFrameHeader, ComputerPngMimeType,
    CuaEffect, decode_computer_frame, encode_computer_frame,
};

use super::super::{
    ComputerActionObservation, ComputerClickGesture, ComputerFrameSubscriber, ComputerStateRequest,
};
use crate::computer::backend::{
    COMPUTER_ACTION_OBSERVATION_MAX_DIMENSION, ComputerBackendError, ComputerSettleOutcome,
    ComputerStreamFrame,
};
use crate::computer::fake_backend::{FakeComputerOptIn, FakeSettleHandler};

const MS_60: Duration = Duration::from_millis(60);

fn proven(proof: ComputerBackendActionResult) -> Arc<Scripted> {
    let backend = Scripted::default_fake();
    backend.set_proof(proof);
    backend
}

fn proof(
    effect: Option<CuaEffect>,
    verified: Option<ComputerDeliveryVerification>,
) -> ComputerBackendActionResult {
    ComputerBackendActionResult {
        effect,
        verified,
        ..ComputerBackendActionResult::default()
    }
}

fn with_settle(
    backend: Arc<Scripted>,
    action_settle_ms: Option<u64>,
    env: ComputerManagerEnv,
) -> Built {
    build(
        backend,
        Setup {
            action_settle_ms,
            env,
            ..Setup::default()
        },
    )
}

fn conditional(enabled: bool) -> ComputerManagerEnv {
    ComputerManagerEnv {
        conditional_settle: enabled,
        ..ComputerManagerEnv::default()
    }
}

fn observer(handler: FakeComputerOptIn<FakeSettleHandler>) -> Arc<Scripted> {
    Scripted::new(FakeComputerBackendOptions {
        wait_for_settle: handler,
        ..FakeComputerBackendOptions::default()
    })
}

fn settle_handler(
    run: impl Fn() -> Result<ComputerSettleOutcome, ComputerError> + Send + Sync + 'static,
) -> FakeComputerOptIn<FakeSettleHandler> {
    FakeComputerOptIn::Handler(Arc::new(move |_options| {
        let outcome = run();
        async move { outcome }.boxed()
    }))
}

async fn press_then_observe(
    manager: &Arc<ComputerManager>,
    hint: Option<&str>,
) -> (Option<ComputerActionObservation>, Duration) {
    manager
        .with_agent_activity(
            "thread-1",
            || async {
                manager
                    .press_key(Some("thread-1"), "enter", None, None)
                    .await?;
                let started = Instant::now();
                let observation = manager.capture_action_screenshot(hint, None, true).await;
                Ok((observation, started.elapsed()))
            },
            None,
            None,
            None,
        )
        .await
        .unwrap()
}

async fn observe_timed(manager: &ComputerManager) -> Duration {
    let started = Instant::now();
    manager.capture_action_screenshot(None, None, true).await;
    started.elapsed()
}

#[tokio::test(start_paused = true)]
async fn zeron_cua_conditional_settle_0_restores_the_fixed_wait_even_on_a_verified_effect() {
    let backend = proven(proof(
        Some(CuaEffect::Verified),
        Some(ComputerDeliveryVerification::Confirmed),
    ));
    let Built { manager, .. } = with_settle(backend, Some(60), conditional(false));
    let (_, waited) = press_then_observe(&manager, None).await;
    assert!(waited >= MS_60, "waited {waited:?}");
    manager.dispose().await;
}

#[tokio::test(start_paused = true)]
async fn skips_the_wait_on_a_verified_effect_by_default_no_flag_needed() {
    let backend = proven(proof(
        Some(CuaEffect::Verified),
        Some(ComputerDeliveryVerification::Confirmed),
    ));
    let Built { manager, .. } = with_settle(backend, Some(60), ComputerManagerEnv::default());
    let (_, waited) = press_then_observe(&manager, None).await;
    assert!(waited < MS_60, "waited {waited:?}");
    manager.dispose().await;
}

#[tokio::test(start_paused = true)]
async fn still_waits_the_compiled_300_ms_when_nothing_overrides_it() {
    let Built { manager, .. } = manager(Scripted::default_fake());
    let (_, waited) = press_then_observe(&manager, None).await;
    assert!(waited >= Duration::from_millis(300), "waited {waited:?}");
    manager.dispose().await;
}

#[tokio::test(start_paused = true)]
async fn zeron_cua_action_settle_ms_overrides_the_wait_and_an_explicit_0_removes_it() {
    let env = |ms: f64| ComputerManagerEnv {
        action_settle_ms: Some(ms),
        ..ComputerManagerEnv::default()
    };
    let Built { manager, .. } = with_settle(Scripted::default_fake(), None, env(45.0));
    let (_, waited) = press_then_observe(&manager, None).await;
    assert_eq!(manager.action_settle_ms, 45);
    assert!(
        waited >= Duration::from_millis(45) && waited < Duration::from_millis(90),
        "waited {waited:?}"
    );
    manager.dispose().await;

    let Built {
        manager: zeroed, ..
    } = with_settle(Scripted::default_fake(), None, env(0.0));
    let (_, waited) = press_then_observe(&zeroed, None).await;
    assert_eq!(zeroed.action_settle_ms, 0);
    assert_eq!(waited, Duration::ZERO);
    zeroed.dispose().await;
}

#[tokio::test(start_paused = true)]
async fn a_constructor_override_wins_over_zeron_cua_action_settle_ms() {
    let env = ComputerManagerEnv {
        action_settle_ms: Some(45.0),
        ..ComputerManagerEnv::default()
    };
    let Built { manager, .. } = with_settle(Scripted::default_fake(), Some(60), env);
    let (_, waited) = press_then_observe(&manager, None).await;
    assert_eq!(manager.action_settle_ms, 60);
    assert!(
        waited >= MS_60 && waited < Duration::from_millis(105),
        "waited {waited:?}"
    );
    manager.dispose().await;
}

#[tokio::test(start_paused = true)]
async fn zeron_cua_conditional_settle_skips_the_wait_on_a_verified_effect() {
    let backend = proven(proof(Some(CuaEffect::Verified), None));
    let Built { manager, .. } = with_settle(backend, Some(60), conditional(true));
    let (_, waited) = press_then_observe(&manager, None).await;
    assert!(waited < MS_60, "waited {waited:?}");
    manager.dispose().await;
}

#[tokio::test(start_paused = true)]
async fn zeron_cua_conditional_settle_skips_the_wait_on_a_confirmed_read_back() {
    let backend = proven(proof(None, Some(ComputerDeliveryVerification::Confirmed)));
    let Built { manager, .. } = with_settle(backend, Some(60), conditional(true));
    let (_, waited) = press_then_observe(&manager, None).await;
    assert!(waited < MS_60, "waited {waited:?}");
    manager.dispose().await;
}

async fn keeps_the_wait_after(verdict: ComputerBackendActionResult) {
    let backend = proven(verdict);
    let Built { manager, .. } = with_settle(backend, Some(60), conditional(true));
    let (_, waited) = press_then_observe(&manager, None).await;
    assert!(waited >= MS_60, "waited {waited:?}");
    manager.dispose().await;
}

#[tokio::test(start_paused = true)]
async fn zeron_cua_conditional_settle_keeps_the_wait_after_an_unconfirmed_read_back() {
    keeps_the_wait_after(proof(None, Some(ComputerDeliveryVerification::Unconfirmed))).await;
}

#[tokio::test(start_paused = true)]
async fn zeron_cua_conditional_settle_keeps_the_wait_after_an_unverifiable_surface() {
    keeps_the_wait_after(proof(
        None,
        Some(ComputerDeliveryVerification::Unverifiable),
    ))
    .await;
}

#[tokio::test(start_paused = true)]
async fn zeron_cua_conditional_settle_keeps_the_wait_after_an_unknown_dispatch() {
    keeps_the_wait_after(proof(Some(CuaEffect::DispatchedUnknown), None)).await;
}

#[tokio::test(start_paused = true)]
async fn zeron_cua_conditional_settle_keeps_the_wait_after_no_verdict_at_all() {
    keeps_the_wait_after(proof(None, None)).await;
}

#[tokio::test(start_paused = true)]
async fn consumes_the_proof_once_a_second_observation_in_the_same_call_settles_again() {
    let backend = proven(proof(Some(CuaEffect::Verified), None));
    let Built { manager, .. } = with_settle(backend, Some(60), conditional(true));
    manager
        .with_agent_activity(
            "thread-1",
            || async {
                manager
                    .press_key(Some("thread-1"), "enter", None, None)
                    .await?;
                let first = observe_timed(&manager).await;
                assert!(first < MS_60, "first waited {first:?}");
                let second = observe_timed(&manager).await;
                assert!(second >= MS_60, "second waited {second:?}");
                Ok(())
            },
            None,
            None,
            None,
        )
        .await
        .unwrap();
    manager.dispose().await;
}

#[tokio::test(start_paused = true)]
async fn a_second_actions_verdict_replaces_the_first_inside_one_call() {
    let Built {
        manager, backend, ..
    } = with_settle(Scripted::default_fake(), Some(60), conditional(true));
    manager
        .with_agent_activity(
            "thread-1",
            || async {
                backend.set_proof(proof(Some(CuaEffect::Verified), None));
                manager
                    .press_key(Some("thread-1"), "enter", None, None)
                    .await?;
                backend.set_proof(proof(Some(CuaEffect::DispatchedUnknown), None));
                manager
                    .press_key(Some("thread-1"), "enter", None, None)
                    .await?;
                let waited = observe_timed(&manager).await;
                assert!(waited >= MS_60, "waited {waited:?}");
                Ok(())
            },
            None,
            None,
            None,
        )
        .await
        .unwrap();
    manager.dispose().await;
}

#[tokio::test(start_paused = true)]
async fn a_verdict_never_waives_a_later_calls_settle() {
    let backend = proven(proof(Some(CuaEffect::Verified), None));
    let Built { manager, .. } = with_settle(backend, Some(60), conditional(true));
    manager
        .with_agent_activity(
            "thread-1",
            || async {
                manager
                    .press_key(Some("thread-1"), "enter", None, None)
                    .await?;
                Ok(())
            },
            None,
            None,
            None,
        )
        .await
        .unwrap();
    manager
        .with_agent_activity(
            "thread-1",
            || async {
                let waited = observe_timed(&manager).await;
                assert!(waited >= MS_60, "waited {waited:?}");
                Ok(())
            },
            None,
            None,
            None,
        )
        .await
        .unwrap();
    manager.dispose().await;
}

#[tokio::test(start_paused = true)]
async fn prefers_the_drivers_observed_settle_when_the_backend_offers_it_and_a_window_is_known() {
    let Built {
        manager, backend, ..
    } = with_settle(
        observer(FakeComputerOptIn::BuiltIn),
        Some(60),
        ComputerManagerEnv::default(),
    );
    let (_, waited) = press_then_observe(&manager, Some("fake-terminal")).await;
    let settle_calls = backend.calls_for("waitForSettle");
    assert_eq!(settle_calls.len(), 1);
    let options = &settle_calls[0].args[0];
    assert_eq!(options["windowId"], json!("fake-terminal"));
    assert_eq!(options["timeoutMs"].as_f64(), Some(5_000.0));
    assert_eq!(options["quietMs"].as_f64(), Some(60.0));
    assert!(waited < MS_60, "waited {waited:?}");
    manager.dispose().await;
}

#[tokio::test(start_paused = true)]
async fn a_busy_verdict_from_the_observer_still_ends_the_wait_the_timeout_already_covered_the_bound()
 {
    let handler = settle_handler(|| {
        Ok(ComputerSettleOutcome {
            settled: false,
            waited_ms: 5_000.0,
            events_seen: Some(14.0),
        })
    });
    let Built {
        manager, backend, ..
    } = with_settle(observer(handler), Some(60), ComputerManagerEnv::default());
    let (_, waited) = press_then_observe(&manager, Some("fake-terminal")).await;
    assert_eq!(backend.calls_for("waitForSettle").len(), 1);
    assert!(waited < MS_60, "waited {waited:?}");
    manager.dispose().await;
}

#[tokio::test(start_paused = true)]
async fn falls_back_to_the_fixed_wait_without_a_window_hint_even_when_the_observer_exists() {
    let Built {
        manager, backend, ..
    } = with_settle(
        observer(FakeComputerOptIn::BuiltIn),
        Some(60),
        ComputerManagerEnv::default(),
    );
    let (_, waited) = press_then_observe(&manager, None).await;
    assert!(backend.calls_for("waitForSettle").is_empty());
    assert!(waited >= MS_60, "waited {waited:?}");
    manager.dispose().await;
}

async fn falls_back_once_and_is_never_probed_again(refusal: &'static str) {
    let probes = Arc::new(AtomicUsize::new(0));
    let counter = probes.clone();
    let handler = settle_handler(move || {
        counter.fetch_add(1, Ordering::SeqCst);
        Err(ComputerBackendError::new(refusal).into())
    });
    let Built {
        manager, backend, ..
    } = with_settle(observer(handler), Some(60), ComputerManagerEnv::default());
    let (_, waited) = press_then_observe(&manager, Some("fake-terminal")).await;
    assert_eq!(probes.load(Ordering::SeqCst), 1);
    assert!(waited >= MS_60, "first waited {waited:?}");
    let (_, waited) = press_then_observe(&manager, Some("fake-terminal")).await;
    assert_eq!(probes.load(Ordering::SeqCst), 1);
    assert_eq!(backend.calls_for("waitForSettle").len(), 1);
    assert!(waited >= MS_60, "second waited {waited:?}");
    manager.dispose().await;
}

#[tokio::test(start_paused = true)]
async fn a_backend_that_cannot_name_the_tool_falls_back_once_and_is_never_probed_again_unknown_tool()
 {
    falls_back_once_and_is_never_probed_again(
        "Unknown tool: wait_for_settle [effect=not-dispatched; automatic replay is forbidden]",
    )
    .await;
}

#[tokio::test(start_paused = true)]
async fn a_backend_that_cannot_name_the_tool_falls_back_once_and_is_never_probed_again_unsupported_host_request()
 {
    falls_back_once_and_is_never_probed_again("Unsupported computer host request.").await;
}

#[tokio::test(start_paused = true)]
async fn a_transient_observer_failure_falls_back_for_that_call_but_does_not_poison_the_probe() {
    let fail = Arc::new(AtomicBool::new(true));
    let failing = fail.clone();
    let handler = settle_handler(move || {
        if failing.load(Ordering::SeqCst) {
            return Err(ComputerBackendError::new(
                "wait_for_settle: window_id 42 is closed, stale, or unknown to WindowServer",
            )
            .into());
        }
        Ok(ComputerSettleOutcome {
            settled: true,
            waited_ms: 30.0,
            events_seen: None,
        })
    });
    let Built {
        manager, backend, ..
    } = with_settle(observer(handler), Some(60), ComputerManagerEnv::default());
    let (_, waited) = press_then_observe(&manager, Some("fake-terminal")).await;
    assert_eq!(backend.calls_for("waitForSettle").len(), 1);
    assert!(waited >= MS_60, "first waited {waited:?}");
    fail.store(false, Ordering::SeqCst);
    let (_, waited) = press_then_observe(&manager, Some("fake-terminal")).await;
    assert_eq!(backend.calls_for("waitForSettle").len(), 2);
    assert!(waited < MS_60, "second waited {waited:?}");
    manager.dispose().await;
}

#[tokio::test(start_paused = true)]
async fn a_proven_effect_still_skips_the_wait_entirely_observer_included() {
    let backend = observer(FakeComputerOptIn::BuiltIn);
    backend.set_proof(proof(Some(CuaEffect::Verified), None));
    let Built {
        manager, backend, ..
    } = with_settle(backend, Some(60), conditional(true));
    let (_, waited) = press_then_observe(&manager, Some("fake-terminal")).await;
    assert!(backend.calls_for("waitForSettle").is_empty());
    assert!(waited < MS_60, "waited {waited:?}");
    manager.dispose().await;
}

#[tokio::test(start_paused = true)]
async fn an_observer_refusal_mid_observation_never_replays_the_action() {
    let handler =
        settle_handler(|| Err(ComputerBackendError::new("Unknown tool: wait_for_settle").into()));
    let Built {
        manager, backend, ..
    } = with_settle(observer(handler), Some(60), ComputerManagerEnv::default());
    press_then_observe(&manager, Some("fake-terminal")).await;
    assert_eq!(backend.logged("pressKey").len(), 1);
    assert_eq!(backend.calls_for("captureScreenshot").len(), 1);
    manager.dispose().await;
}

fn recording_sink() -> (
    Arc<Mutex<Vec<ComputerStreamFrame>>>,
    ComputerFrameSubscriber,
) {
    let received: Arc<Mutex<Vec<ComputerStreamFrame>>> = Arc::default();
    let sink = received.clone();
    (
        received,
        Arc::new(move |frame: &ComputerStreamFrame| sink.lock().unwrap().push(frame.clone())),
    )
}

async fn flush(manager: &ComputerManager) {
    settle().await;
    manager.flush_stream_transitions().await;
}

fn wire_round_trip(manager: &ComputerManager, frame: &ComputerStreamFrame) -> bool {
    let wire = ComputerFrame {
        header: ComputerFrameHeader {
            computer_id: manager.computer_id().to_owned(),
            sequence: frame.sequence as u32,
            timestamp_ms: frame.timestamp_ms,
            keyframe: frame.keyframe,
            codec_config: frame.codec_config,
        },
        payload: frame.data.clone(),
    };
    let Ok(bytes) = encode_computer_frame(&wire) else {
        return false;
    };
    decode_computer_frame(&bytes).is_ok_and(|decoded| decoded == wire)
}

fn last_capture_request(backend: &Scripted) -> Value {
    backend
        .calls_for("captureScreenshot")
        .last()
        .map(|call| call.args[0].clone())
        .unwrap()
}

fn observed_window(observation: &Option<ComputerActionObservation>) -> Option<&str> {
    observation
        .as_ref()
        .and_then(|observed| observed.window_id())
}

#[tokio::test]
async fn runs_the_synthetic_frame_attach_publish_and_detach_loop() {
    let Built {
        manager, backend, ..
    } = manager(Scripted::default_fake());
    let events = events(&manager);
    let (received, sink) = recording_sink();
    let subscription = manager.subscribe_frames(sink);
    flush(&manager).await;

    assert_eq!(backend.calls_for("attachStream").len(), 1);
    assert_eq!(received.lock().unwrap().len(), 2);
    assert!(wire_round_trip(&manager, &received.lock().unwrap()[0]));
    backend.fake.emit_frame(false, false, vec![7, 8]);
    assert_eq!(received.lock().unwrap().len(), 3);

    drop(subscription);
    flush(&manager).await;
    assert_eq!(backend.calls_for("detachStream").len(), 1);
    let count = received.lock().unwrap().len();
    backend.fake.emit_frame(true, false, Vec::new());
    assert_eq!(received.lock().unwrap().len(), count);

    assert!(!events.types().contains(&"computer.frame"));

    manager.dispose().await;
}

#[tokio::test]
async fn drops_late_frames_and_state_updates_after_a_thread_is_removed() {
    let Built {
        manager, backend, ..
    } = manager(Scripted::default_fake());
    let (_received, sink) = recording_sink();
    let subscription = manager.subscribe_frames(sink);
    flush(&manager).await;
    manager.get_thread_state("thread-removed").await;
    manager.handle_thread_removed("thread-removed").await;

    backend.fake.emit_frame(false, false, vec![9]);
    let error = manager
        .with_agent_activity("thread-removed", || async { Ok(()) }, None, None, None)
        .await
        .unwrap_err();
    assert!(error.to_string().contains("revoked"), "{error}");
    manager
        .record_thread_error("thread-removed", "late error")
        .await;

    assert!(
        !manager
            .state()
            .threads
            .iter()
            .any(|(id, _)| id == "thread-removed")
    );

    drop(subscription);
    manager.dispose().await;
}

#[tokio::test]
async fn does_not_reattach_a_stream_after_disposal_wins_during_keyframe_recovery() {
    let backend = Scripted::default_fake();
    let detach_started = deferred();
    let allow_detach = deferred();
    backend.remove(ComputerBackendMethod::RequestKeyframe);
    {
        let started = detach_started.clone();
        let allow = allow_detach.clone();
        backend.hooks().detach_stream = Some(Arc::new(move |fake, ()| {
            let started = started.clone();
            let allow = allow.clone();
            async move {
                started.resolve();
                allow.wait().await;
                fake.detach_stream().await
            }
            .boxed()
        }));
    }
    let Built {
        manager, backend, ..
    } = manager(backend);
    let (_received, sink) = recording_sink();
    let subscription = manager.subscribe_frames(sink);
    flush(&manager).await;

    let request = tokio::spawn({
        let manager = manager.clone();
        async move { manager.request_keyframe().await }
    });
    detach_started.wait().await;
    let disposal = tokio::spawn({
        let manager = manager.clone();
        async move { manager.dispose().await }
    });
    settle().await;
    allow_detach.resolve();
    request.await.unwrap().unwrap();
    disposal.await.unwrap();

    assert_eq!(backend.calls_for("attachStream").len(), 1);
    assert_eq!(backend.calls_for("detachStream").len(), 1);
    drop(subscription);
}

#[tokio::test]
async fn carries_the_backends_capabilities_onto_every_thread_snapshot() {
    let backend = Scripted::new(FakeComputerBackendOptions {
        capabilities: Some(ComputerCapabilities {
            windows: true,
            window_bounds: false,
            stacking: false,
            capture: true,
            input: true,
            clipboard: false,
            focus: false,
            raise: false,
            ghost_cursor: false,
            visible_desktop: false,
        }),
        ..FakeComputerBackendOptions::default()
    });
    let Built {
        manager, backend, ..
    } = manager(backend);

    let state = manager.get_thread_state("thread-1").await;

    assert_eq!(state.capabilities, backend.capabilities());
    manager.dispose().await;
}

#[tokio::test]
async fn refuses_a_window_scoped_click_when_the_desktop_reports_no_geometry() {
    let backend = Scripted::new(FakeComputerBackendOptions {
        windows: Some(vec![ComputerWindow {
            app_name: Some("org.kde.kcalc".into()),
            focused: true,
            ..window("boundless", "Calculator", None)
        }]),
        ..FakeComputerBackendOptions::default()
    });
    let Built {
        manager, backend, ..
    } = manager(backend);

    let error = manager
        .click(
            Some("thread-1"),
            scoped(100.0, 100.0, "boundless"),
            &[],
            ComputerClickGesture::default(),
        )
        .await
        .unwrap_err();
    assert_eq!(code(&error), Some("computer_target_offscreen"));
    assert!(backend.calls_for("click").is_empty());
    manager.dispose().await;
}

#[tokio::test]
async fn returns_perception_payloads_with_optional_text_and_screenshot() {
    let Built { manager, .. } = manager(Scripted::default_fake());
    let state = manager
        .get_state(ComputerStateRequest {
            include_screenshot: Some(true),
            include_text: true,
            include_tree: None,
            window_id: None,
        })
        .await
        .unwrap();

    let screenshot = state.screenshot.as_ref().unwrap();
    assert_eq!(screenshot.mime_type, ComputerPngMimeType::Png);
    assert!(!screenshot.bytes_base64.is_empty());
    assert!(state.text.as_deref().unwrap().contains("Calculate"));
    assert!(!state.root.as_ref().unwrap().children.is_empty());

    manager.dispose().await;
}

#[tokio::test]
async fn captures_the_focused_window_and_the_workspace_when_nothing_capturable_has_focus() {
    let Built {
        manager, backend, ..
    } = settled(Scripted::default_fake());

    let focused = manager.capture_focused_window(None, false).await.unwrap();
    assert_eq!(focused.window_id.as_deref(), Some("fake-terminal"));
    assert_eq!(
        last_capture_request(&backend),
        json!({ "kind": "window", "windowId": "fake-terminal" })
    );

    backend.fake.emit_windows_changed(vec![
        ComputerWindow {
            focused: true,
            minimized: true,
            visible: false,
            stacking_index: Some(1),
            ..window(
                "fake-terminal",
                "Terminal",
                Some(rect(40.0, 40.0, 960.0, 720.0)),
            )
        },
        ComputerWindow {
            stacking_index: Some(0),
            ..window(
                "fake-calculator",
                "Calculator",
                Some(rect(1_050.0, 120.0, 420.0, 620.0)),
            )
        },
    ]);
    let topmost = manager.capture_focused_window(None, false).await.unwrap();
    assert_eq!(topmost.window_id.as_deref(), Some("fake-calculator"));

    backend.fake.emit_windows_changed(Vec::new());
    let workspace = manager
        .capture_focused_window(Some(1_024), false)
        .await
        .unwrap();
    assert_eq!(workspace.window_id, None);
    assert_eq!(
        last_capture_request(&backend),
        json!({
            "kind": "region",
            "region": rect(0.0, 0.0, 1_920.0, 1_080.0),
            "maxDimension": 1_024,
        })
    );

    manager.dispose().await;
}

#[tokio::test]
async fn captures_the_actions_window_on_a_hint_reports_a_vanished_target_and_never_throws() {
    let Built {
        manager, backend, ..
    } = settled(Scripted::default_fake());

    let hinted = manager
        .capture_action_screenshot(Some("fake-calculator"), None, true)
        .await;
    assert_eq!(observed_window(&hinted), Some("fake-calculator"));

    let closed = manager
        .capture_action_screenshot(Some("gone-window"), None, true)
        .await;
    assert_eq!(closed, Some(ComputerActionObservation::TargetWindowClosed));

    backend.fake.fail_next("captureScreenshot");
    assert_eq!(
        manager
            .capture_action_screenshot(Some("fake-calculator"), None, true)
            .await,
        None
    );

    backend.fake.fail_next("captureScreenshot");
    assert_eq!(
        manager.capture_action_screenshot(None, None, true).await,
        None
    );

    manager.dispose().await;
}

#[tokio::test]
async fn observes_only_the_agents_focus_target_after_an_untargeted_action_never_the_active_window()
{
    let Built {
        manager, backend, ..
    } = settled(Scripted::default_fake());

    backend.fake.emit_windows_changed(vec![ComputerWindow {
        active: Some(true),
        stacking_index: Some(0),
        ..window(
            "human-browser",
            "Browser",
            Some(rect(100.0, 100.0, 1_200.0, 800.0)),
        )
    }]);
    let observed = manager.capture_action_screenshot(None, None, true).await;
    assert_eq!(observed_window(&observed), None);
    assert_eq!(last_capture_request(&backend)["kind"], json!("region"));

    let perception = manager.capture_focused_window(None, false).await.unwrap();
    assert_eq!(perception.window_id.as_deref(), Some("human-browser"));

    manager.dispose().await;
}

#[tokio::test]
async fn photographs_the_window_under_an_untargeted_actions_point_instead_of_the_workspace() {
    let Built {
        manager, backend, ..
    } = settled(Scripted::default_fake());

    let observed = manager
        .capture_action_screenshot(
            None,
            Some(ComputerPoint {
                x: 1_100.0,
                y: 200.0,
            }),
            true,
        )
        .await;
    assert_eq!(observed_window(&observed), Some("fake-calculator"));
    assert_eq!(
        last_capture_request(&backend),
        json!({
            "kind": "window",
            "windowId": "fake-calculator",
            "maxDimension": COMPUTER_ACTION_OBSERVATION_MAX_DIMENSION,
        })
    );

    let desktop = manager
        .capture_action_screenshot(
            None,
            Some(ComputerPoint {
                x: 1_800.0,
                y: 1_000.0,
            }),
            true,
        )
        .await;
    assert_eq!(observed_window(&desktop), Some("fake-terminal"));

    manager.dispose().await;
}

#[tokio::test]
async fn resolves_overlapping_point_candidates_by_stacking_order_and_refuses_to_guess_without_one()
{
    let Built {
        manager, backend, ..
    } = settled(Scripted::default_fake());
    let overlapping = |stacked: bool| {
        vec![
            ComputerWindow {
                stacking_index: stacked.then_some(1),
                ..window("under", "Under", Some(rect(100.0, 100.0, 800.0, 600.0)))
            },
            ComputerWindow {
                stacking_index: stacked.then_some(0),
                ..window("over", "Over", Some(rect(300.0, 200.0, 400.0, 300.0)))
            },
        ]
    };
    let point = ComputerPoint { x: 400.0, y: 300.0 };

    backend.fake.emit_windows_changed(overlapping(true));
    let observed = manager
        .capture_action_screenshot(None, Some(point), true)
        .await;
    assert_eq!(observed_window(&observed), Some("over"));

    backend.fake.emit_windows_changed(overlapping(false));
    let widened = manager
        .capture_action_screenshot(None, Some(point), true)
        .await;
    assert_eq!(observed_window(&widened), None);
    assert_eq!(last_capture_request(&backend)["kind"], json!("region"));

    manager.dispose().await;
}

#[tokio::test]
async fn falls_back_to_the_focus_path_when_the_point_window_vanishes_before_its_capture() {
    let Built {
        manager, backend, ..
    } = settled(Scripted::default_fake());

    backend.fake.fail_next("captureScreenshot");
    let observed = manager
        .capture_action_screenshot(
            None,
            Some(ComputerPoint {
                x: 1_100.0,
                y: 200.0,
            }),
            true,
        )
        .await;
    assert_eq!(observed_window(&observed), Some("fake-terminal"));

    manager.dispose().await;
}

#[tokio::test]
async fn skips_the_post_action_capture_entirely_on_a_backend_that_cannot_capture() {
    let backend = Scripted::new(FakeComputerBackendOptions {
        capabilities: Some(ComputerCapabilities {
            capture: false,
            ..Scripted::default_fake().capabilities()
        }),
        ..FakeComputerBackendOptions::default()
    });
    let Built {
        manager, backend, ..
    } = settled(backend);

    assert_eq!(
        manager
            .capture_action_screenshot(Some("fake-terminal"), None, true)
            .await,
        None
    );
    assert!(backend.calls_for("captureScreenshot").is_empty());

    manager.dispose().await;
}

#[tokio::test]
async fn seeds_panels_from_the_passive_probe_and_goes_live_from_the_first_real_use() {
    let Built {
        manager, backend, ..
    } = settled(Scripted::default_fake());
    let events = events(&manager);

    let seeded = manager.get_thread_state("thread-1").await;
    assert_eq!(
        seeded.availability,
        ComputerAvailability::Available {
            backend: Some("fake".into())
        }
    );
    assert!(seeded.windows.is_empty());
    assert_eq!(
        seeded.screen_size,
        ComputerScreenSize {
            width: 1,
            height: 1,
            scale: None
        }
    );
    assert_eq!(backend.methods(), ["probeAvailability"]);

    manager.get_thread_state("thread-1").await;
    manager.get_thread_state("thread-2").await;
    assert_eq!(
        backend.methods(),
        [
            "probeAvailability",
            "probeAvailability",
            "probeAvailability"
        ]
    );

    manager
        .with_agent_activity(
            "thread-1",
            || async { manager.list_windows().await },
            None,
            None,
            None,
        )
        .await
        .unwrap();
    let live = manager.get_thread_state("thread-1").await;
    assert_eq!(
        live.windows
            .iter()
            .map(|window| window.title.as_str())
            .collect::<Vec<_>>(),
        ["Terminal", "Calculator"]
    );
    assert_eq!(
        live.screen_size,
        ComputerScreenSize {
            width: 1_920,
            height: 1_080,
            scale: Some(1.0)
        }
    );
    assert!(!backend.calls_for("availability").is_empty());
    settle().await;
    assert_eq!(
        events
            .states_for("thread-2")
            .last()
            .map(|state| state.windows.len()),
        Some(2)
    );

    manager.dispose().await;
}

#[tokio::test]
async fn engages_the_backend_when_the_pane_attaches_or_the_human_drives_it() {
    let Built {
        manager: pane_manager,
        backend: pane_backend,
        ..
    } = manager(Scripted::default_fake());
    let (_received, sink) = recording_sink();
    let detach = pane_manager.subscribe_frames(sink);
    flush(&pane_manager).await;
    assert_eq!(pane_backend.calls_for("attachStream").len(), 1);
    assert_eq!(
        pane_manager
            .get_thread_state("thread-pane")
            .await
            .windows
            .len(),
        2
    );
    drop(detach);
    pane_manager.dispose().await;

    let Built {
        manager: input_manager,
        ..
    } = manager(Scripted::default_fake());
    input_manager
        .click(
            None,
            point(10.0, 10.0),
            &[],
            ComputerClickGesture::default(),
        )
        .await
        .unwrap();
    assert_eq!(
        input_manager
            .get_thread_state("thread-pane")
            .await
            .windows
            .len(),
        2
    );
    input_manager.dispose().await;
}

#[tokio::test]
async fn downscales_large_action_observations_while_keeping_the_coordinate_mapping_exact() {
    let tall = f64::from(COMPUTER_ACTION_OBSERVATION_MAX_DIMENSION) + 1_000.0;
    let backend = Scripted::new(FakeComputerBackendOptions {
        screen_size: Some(ComputerScreenSize {
            width: 3_000,
            height: tall as u32 + 400,
            scale: Some(1.0),
        }),
        windows: Some(vec![ComputerWindow {
            focused: true,
            ..window(
                "fake-editor",
                "Editor",
                Some(rect(100.0, 100.0, 1_280.0, tall)),
            )
        }]),
        ..FakeComputerBackendOptions::default()
    });
    let Built {
        manager, backend, ..
    } = settled(backend);

    let observed = manager
        .capture_action_screenshot(Some("fake-editor"), None, true)
        .await;
    assert_eq!(
        last_capture_request(&backend),
        json!({
            "kind": "window",
            "windowId": "fake-editor",
            "maxDimension": COMPUTER_ACTION_OBSERVATION_MAX_DIMENSION,
        })
    );
    let screenshot = observed
        .as_ref()
        .and_then(ComputerActionObservation::screenshot)
        .expect("the action observation carried no screenshot");
    let region = screenshot.region.expect("no coordinate mapping");
    let scale = screenshot.scale.expect("no coordinate mapping");
    assert!(
        (scale - f64::from(COMPUTER_ACTION_OBSERVATION_MAX_DIMENSION) / tall).abs() < 5e-11,
        "scale {scale}"
    );
    assert_eq!(region, rect(100.0, 100.0, 1_280.0, tall));
    let middle_x = region.x + f64::from(screenshot.width) / 2.0 / scale;
    let middle_y = region.y + f64::from(screenshot.height) / 2.0 / scale;
    assert!((middle_x - (region.x + region.width / 2.0)).abs() < 0.5);
    assert!((middle_y - (region.y + region.height / 2.0)).abs() < 0.5);

    manager.capture_focused_window(None, false).await.unwrap();
    assert_eq!(
        last_capture_request(&backend),
        json!({ "kind": "window", "windowId": "fake-editor" })
    );

    manager.dispose().await;
}

#[tokio::test]
async fn returns_every_capture_for_the_gateway_to_compare_with_delivered_screenshots() {
    let Built {
        manager, backend, ..
    } = settled(Scripted::default_fake());
    let first = manager
        .capture_action_screenshot(Some("fake-terminal"), None, true)
        .await;
    let second = manager
        .capture_action_screenshot(Some("fake-terminal"), None, true)
        .await;
    let Some(ComputerActionObservation::Captured(first)) = first else {
        panic!("Expected a captured screenshot");
    };
    let Some(ComputerActionObservation::Captured(second)) = second else {
        panic!("Expected a captured screenshot, got {second:?}");
    };
    assert!(!second.screenshot.captured_at.is_empty());
    let mut expected = first.clone();
    expected.screenshot.captured_at = second.screenshot.captured_at.clone();
    assert_eq!(second, expected);
    assert_eq!(backend.calls_for("captureScreenshot").len(), 2);
    manager.dispose().await;
}
