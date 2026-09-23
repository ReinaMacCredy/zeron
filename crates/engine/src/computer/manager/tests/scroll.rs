#[allow(unused_imports)]
use super::*;

use std::sync::atomic::AtomicUsize;
use std::time::Duration;

use tokio::time::Instant;
use zeron_proto::computer::{
    ComputerDelivery, ComputerDeliveryVerification, ComputerScrollDelta, ComputerScrollTelemetry,
    CuaEffect,
};

use super::super::{
    ComputerActionObservation, ComputerClickGesture, ComputerScrollOptions, ComputerScrollOutcome,
};

const MS_60: Duration = Duration::from_millis(60);

struct Fixture {
    manager: Arc<ComputerManager>,
    backend: Arc<Scripted>,
}

fn scripted_fixture(
    travels: Vec<Option<f64>>,
    backend: Arc<Scripted>,
    action_settle_ms: u64,
    env: ComputerManagerEnv,
    screenshot_count: usize,
) -> Fixture {
    let measured = Arc::new(AtomicUsize::new(0));
    let Built {
        manager, backend, ..
    } = build(
        backend,
        Setup {
            action_settle_ms: Some(action_settle_ms),
            env,
            measure_scroll_travel: Some(Arc::new(move |_before, _after, _column| {
                let index = measured.fetch_add(1, Ordering::SeqCst);
                let travel = travels.get(index).copied().flatten();
                async move { travel }.boxed()
            })),
            ..Setup::default()
        },
    );
    backend
        .fake
        .queue_screenshots((0..screenshot_count).map(|index| format!("capture-{index}")));
    Fixture { manager, backend }
}

fn calibrated_scroll_fixture(travels: &[Option<f64>], backend: Arc<Scripted>) -> Fixture {
    scripted_fixture(
        travels.to_vec(),
        backend,
        0,
        ComputerManagerEnv::default(),
        12,
    )
}

fn settle_scroll_fixture(
    travels: &[Option<f64>],
    backend: Arc<Scripted>,
    conditional_settle: bool,
    screenshot_count: usize,
) -> Fixture {
    scripted_fixture(
        travels.to_vec(),
        backend,
        60,
        ComputerManagerEnv {
            conditional_settle,
            ..ComputerManagerEnv::default()
        },
        screenshot_count,
    )
}

fn observe(observe: bool) -> ComputerScrollOptions {
    ComputerScrollOptions {
        observe,
        modifiers: Vec::new(),
    }
}

async fn scroll_at(
    manager: &ComputerManager,
    x: f64,
    y: f64,
    delta_y: f64,
    observed: bool,
) -> ComputerScrollOutcome {
    manager
        .scroll_calibrated(
            Some("thread-1"),
            Some(point(x, y).into()),
            0.0,
            delta_y,
            observe(observed),
        )
        .await
        .unwrap()
}

fn scroll_legs(backend: &Scripted) -> Vec<f64> {
    backend
        .calls_for("scroll")
        .iter()
        .map(|call| call.args[2].as_f64().unwrap())
        .collect()
}

fn telemetry(outcome: &ComputerScrollOutcome) -> &ComputerScrollTelemetry {
    outcome.result.scroll.as_ref().unwrap()
}

fn has_screenshot(outcome: &ComputerScrollOutcome) -> bool {
    outcome
        .observation
        .as_ref()
        .and_then(ComputerActionObservation::screenshot)
        .is_some()
}

fn captures(backend: &Scripted) -> usize {
    backend.calls_for("captureScreenshot").len()
}

fn close(actual: f64, expected: f64) -> bool {
    (actual - expected).abs() < 5e-7
}

#[tokio::test]
async fn probes_an_unmeasured_window_then_delivers_the_remainder_pre_corrected() {
    let Fixture {
        manager, backend, ..
    } = calibrated_scroll_fixture(
        &[Some(336.0), Some(64.0), Some(400.0)],
        Scripted::default_fake(),
    );

    let first = scroll_at(&manager, 1_100.0, 200.0, 400.0, true).await;
    assert_eq!(
        first.result.scroll,
        Some(ComputerScrollTelemetry {
            requested: ComputerScrollDelta {
                delta_x: 0.0,
                delta_y: 400.0
            },
            injected: ComputerScrollDelta {
                delta_x: 0.0,
                delta_y: 57.14
            },
            limited_to: None,
            traveled_y: Some(400.0),
            gearing: Some(7.0),
            routes: Some(vec!["wheel".into(), "wheel".into()]),
        })
    );
    let legs = scroll_legs(&backend);
    assert_eq!(legs[0], 48.0);
    assert!(close(legs[1], 64.0 / 7.0), "leg {}", legs[1]);
    assert!(has_screenshot(&first));

    let second = scroll_at(&manager, 1_100.0, 200.0, 400.0, true).await;
    assert_eq!(telemetry(&second).injected.delta_y, 57.14);
    assert_eq!(telemetry(&second).traveled_y, Some(400.0));
    assert_eq!(telemetry(&second).gearing, Some(7.0));
    let legs = scroll_legs(&backend);
    assert_eq!(legs.len(), 3);
    assert!(close(legs[2], 400.0 / 7.0), "leg {}", legs[2]);

    manager.dispose().await;
}

#[tokio::test]
async fn falls_back_to_the_full_request_when_the_probe_cannot_be_measured() {
    let Fixture {
        manager, backend, ..
    } = calibrated_scroll_fixture(&[None, None], Scripted::default_fake());

    let result = scroll_at(&manager, 1_100.0, 200.0, 400.0, true).await;

    assert_eq!(
        telemetry(&result).injected,
        ComputerScrollDelta {
            delta_x: 0.0,
            delta_y: 400.0
        }
    );
    assert_eq!(telemetry(&result).traveled_y, None);
    assert_eq!(telemetry(&result).gearing, Some(1.0));
    assert_eq!(scroll_legs(&backend), [48.0, 352.0]);

    manager.dispose().await;
}

#[tokio::test]
async fn still_identifies_the_window_for_an_untargeted_scroll_after_the_clear() {
    let Fixture {
        manager, backend, ..
    } = calibrated_scroll_fixture(&[Some(336.0), Some(64.0)], Scripted::default_fake());

    manager
        .click(
            Some("thread-1"),
            point(1_100.0, 200.0),
            &[],
            ComputerClickGesture::default(),
        )
        .await
        .unwrap();
    let result = manager
        .scroll_calibrated(Some("thread-1"), None, 0.0, 400.0, observe(true))
        .await
        .unwrap();

    assert!(!backend.calls_for("clearFocusWindow").is_empty());
    assert_eq!(telemetry(&result).gearing, Some(7.0));
    assert_eq!(telemetry(&result).traveled_y, Some(400.0));
    assert!(has_screenshot(&result));

    manager.dispose().await;
}

#[tokio::test]
async fn converts_measured_travel_out_of_capture_pixels_before_reporting_or_learning_it() {
    let backend = Scripted::new(FakeComputerBackendOptions {
        windows: Some(vec![ComputerWindow {
            focused: true,
            stacking_index: Some(0),
            ..window(
                "fake-browser",
                "Browser",
                Some(rect(0.0, 0.0, 1_920.0, 1_080.0)),
            )
        }]),
        ..FakeComputerBackendOptions::default()
    });
    let Fixture { manager, .. } = calibrated_scroll_fixture(&[Some(800.0)], backend);

    let result = scroll_at(&manager, 900.0, 500.0, 40.0, true).await;

    assert_eq!(telemetry(&result).traveled_y, Some(1_000.0));
    assert_eq!(telemetry(&result).gearing, Some(25.0));

    manager.dispose().await;
}

#[tokio::test]
async fn measures_travel_around_the_scroll_point_column() {
    let backend = Scripted::new(FakeComputerBackendOptions {
        windows: Some(vec![ComputerWindow {
            focused: true,
            stacking_index: Some(0),
            ..window(
                "fake-browser",
                "Browser",
                Some(rect(0.0, 0.0, 1_920.0, 1_080.0)),
            )
        }]),
        ..FakeComputerBackendOptions::default()
    });
    let columns = Arc::new(std::sync::Mutex::new(Vec::new()));
    let seen = columns.clone();
    let Built { manager, .. } = build(
        backend.clone(),
        Setup {
            action_settle_ms: Some(0),
            measure_scroll_travel: Some(Arc::new(move |_before, _after, column| {
                seen.lock().unwrap().push(column);
                async { None }.boxed()
            })),
            ..Setup::default()
        },
    );
    backend
        .fake
        .queue_screenshots((0..4).map(|index| format!("capture-{index}")));

    scroll_at(&manager, 192.0, 500.0, 40.0, true).await;

    let columns = columns.lock().unwrap().clone();
    assert!(!columns.is_empty());
    assert!(
        columns
            .iter()
            .all(|column| column.is_some_and(|column| close(column, 0.1))),
        "{columns:?}"
    );

    manager.dispose().await;
}

#[tokio::test]
async fn suppresses_a_wrong_way_measurement_instead_of_reporting_or_learning_it() {
    let Fixture {
        manager, backend, ..
    } = calibrated_scroll_fixture(&[Some(-752.0), None], Scripted::default_fake());

    let result = scroll_at(&manager, 1_100.0, 200.0, 100.0, true).await;

    assert_eq!(telemetry(&result).traveled_y, None);
    assert_eq!(telemetry(&result).gearing, Some(1.0));
    assert_eq!(scroll_legs(&backend), [48.0, 52.0]);

    manager.dispose().await;
}

#[tokio::test]
async fn reads_byte_identical_captures_as_no_movement_and_learns_nothing_from_it() {
    let measurements = Arc::new(AtomicUsize::new(0));
    let measured = measurements.clone();
    let Built { manager, .. } = build(
        Scripted::default_fake(),
        Setup {
            action_settle_ms: Some(0),
            measure_scroll_travel: Some(Arc::new(move |_before, _after, _column| {
                measured.fetch_add(1, Ordering::SeqCst);
                async { None }.boxed()
            })),
            ..Setup::default()
        },
    );

    let first = scroll_at(&manager, 1_100.0, 200.0, 400.0, true).await;
    assert_eq!(telemetry(&first).traveled_y, Some(0.0));
    assert_eq!(telemetry(&first).gearing, Some(1.0));
    assert_eq!(measurements.load(Ordering::SeqCst), 0);

    let second = scroll_at(&manager, 1_100.0, 200.0, 400.0, true).await;
    assert_eq!(
        telemetry(&second).injected,
        ComputerScrollDelta {
            delta_x: 0.0,
            delta_y: 400.0
        }
    );
    assert_eq!(telemetry(&second).traveled_y, Some(0.0));
    assert!(has_screenshot(&second));
    assert_eq!(
        second
            .observation
            .as_ref()
            .and_then(ComputerActionObservation::window_id),
        Some("fake-calculator")
    );
    assert_eq!(measurements.load(Ordering::SeqCst), 0);

    manager.dispose().await;
}

#[tokio::test]
async fn keeps_the_correction_but_takes_no_captures_when_the_caller_wants_no_observation() {
    let Fixture {
        manager, backend, ..
    } = calibrated_scroll_fixture(&[Some(336.0), Some(64.0)], Scripted::default_fake());

    scroll_at(&manager, 1_100.0, 200.0, 400.0, true).await;
    assert_eq!(captures(&backend), 3);

    let unobserved = scroll_at(&manager, 1_100.0, 200.0, 400.0, false).await;
    assert_eq!(captures(&backend), 3);
    assert_eq!(unobserved.observation, None);
    assert_eq!(telemetry(&unobserved).traveled_y, None);
    assert_eq!(telemetry(&unobserved).injected.delta_y, 57.14);
    let legs = scroll_legs(&backend);
    assert!(close(*legs.last().unwrap(), 400.0 / 7.0));

    manager.dispose().await;
}

#[tokio::test]
async fn never_re_gears_the_panes_own_scroll_whatever_the_agent_learned() {
    let Fixture {
        manager, backend, ..
    } = calibrated_scroll_fixture(&[Some(336.0), Some(64.0)], Scripted::default_fake());
    scroll_at(&manager, 1_100.0, 200.0, 400.0, true).await;
    let captures_after_learning = captures(&backend);

    manager
        .scroll(None, Some(point(1_100.0, 200.0).into()), 0.0, 400.0)
        .await
        .unwrap();
    assert_eq!(
        backend.calls_for("scroll").last().unwrap().args,
        [
            json!({ "x": 1_100.0, "y": 200.0 }),
            json!(0.0),
            json!(400.0)
        ]
    );
    assert_eq!(captures(&backend), captures_after_learning);

    manager.dispose().await;
}

#[tokio::test]
async fn delivers_the_scroll_unmeasured_when_the_capture_fails() {
    let Fixture {
        manager, backend, ..
    } = calibrated_scroll_fixture(&[Some(336.0)], Scripted::default_fake());
    backend.fake.fail_next("captureScreenshot");

    let result = scroll_at(&manager, 1_100.0, 200.0, 400.0, true).await;

    assert_eq!(result.result.action, "computer_scroll");
    assert_eq!(telemetry(&result).traveled_y, None);
    assert_eq!(result.observation, None);
    assert_eq!(backend.calls_for("scroll").len(), 1);
    assert_eq!(captures(&backend), 1);

    manager.dispose().await;
}

async fn teach_gearing(manager: &ComputerManager) {
    scroll_at(manager, 1_100.0, 200.0, 400.0, true).await;
}

async fn timed_scroll(manager: &ComputerManager) -> (ComputerScrollOutcome, Duration) {
    let started = Instant::now();
    let outcome = scroll_at(manager, 1_100.0, 200.0, 400.0, true).await;
    (outcome, started.elapsed())
}

fn settle_waited(elapsed: Duration) -> u32 {
    (elapsed.as_millis() / MS_60.as_millis()) as u32
}

#[tokio::test(start_paused = true)]
async fn waives_the_leg_settle_when_measured_travel_already_proves_arrival() {
    let Fixture {
        manager, backend, ..
    } = settle_scroll_fixture(
        &[Some(336.0), Some(64.0), Some(400.0)],
        Scripted::default_fake(),
        true,
        16,
    );
    teach_gearing(&manager).await;
    let captures_before = captures(&backend);

    let (result, elapsed) = timed_scroll(&manager).await;

    assert_eq!(settle_waited(elapsed), 0, "elapsed {elapsed:?}");
    assert_eq!(telemetry(&result).traveled_y, Some(400.0));
    assert_eq!(telemetry(&result).gearing, Some(7.0));
    assert_eq!(captures(&backend), captures_before + 2);
    assert!(has_screenshot(&result));

    manager.dispose().await;
}

#[tokio::test(start_paused = true)]
async fn keeps_the_settle_when_the_correlation_refuses_the_early_capture() {
    let Fixture {
        manager, backend, ..
    } = settle_scroll_fixture(
        &[Some(336.0), Some(64.0), None, None],
        Scripted::default_fake(),
        true,
        16,
    );
    teach_gearing(&manager).await;
    let captures_before = captures(&backend);

    let (result, elapsed) = timed_scroll(&manager).await;

    assert_eq!(settle_waited(elapsed), 1, "elapsed {elapsed:?}");
    assert_eq!(telemetry(&result).traveled_y, None);
    assert_eq!(telemetry(&result).gearing, Some(7.0));
    assert_eq!(captures(&backend), captures_before + 3);

    manager.dispose().await;
}

#[tokio::test(start_paused = true)]
async fn keeps_the_settle_when_early_travel_misses_the_prediction_and_never_learns_it() {
    let Fixture {
        manager, backend, ..
    } = settle_scroll_fixture(
        &[Some(336.0), Some(64.0), Some(250.0), Some(400.0)],
        Scripted::default_fake(),
        true,
        16,
    );
    teach_gearing(&manager).await;
    let captures_before = captures(&backend);

    let (result, elapsed) = timed_scroll(&manager).await;

    assert_eq!(settle_waited(elapsed), 1, "elapsed {elapsed:?}");
    assert_eq!(telemetry(&result).traveled_y, Some(400.0));
    assert_eq!(telemetry(&result).gearing, Some(7.0));
    assert_eq!(captures(&backend), captures_before + 3);

    manager.dispose().await;
}

#[tokio::test(start_paused = true)]
async fn keeps_the_probe_legs_settle_an_unmeasured_route_has_no_prediction_to_prove() {
    let Fixture {
        manager, backend, ..
    } = settle_scroll_fixture(
        &[Some(336.0), Some(64.0)],
        Scripted::default_fake(),
        true,
        16,
    );

    let (result, elapsed) = timed_scroll(&manager).await;

    assert_eq!(settle_waited(elapsed), 1, "elapsed {elapsed:?}");
    assert_eq!(telemetry(&result).traveled_y, Some(400.0));
    assert_eq!(telemetry(&result).gearing, Some(7.0));
    assert_eq!(
        telemetry(&result).routes,
        Some(vec!["wheel".into(), "wheel".into()])
    );
    assert_eq!(captures(&backend), 3);

    manager.dispose().await;
}

#[tokio::test(start_paused = true)]
async fn keeps_every_settle_and_takes_no_early_capture_under_the_kill_switch() {
    let Fixture {
        manager, backend, ..
    } = settle_scroll_fixture(
        &[Some(336.0), Some(64.0), Some(400.0)],
        Scripted::default_fake(),
        false,
        16,
    );
    teach_gearing(&manager).await;
    let captures_before = captures(&backend);

    let (result, elapsed) = timed_scroll(&manager).await;

    assert_eq!(settle_waited(elapsed), 1, "elapsed {elapsed:?}");
    assert_eq!(telemetry(&result).traveled_y, Some(400.0));
    assert_eq!(captures(&backend), captures_before + 2);

    manager.dispose().await;
}

#[tokio::test(start_paused = true)]
async fn keeps_the_settle_on_an_unchanged_scroll_and_still_reports_traveled_y_0() {
    let Fixture {
        manager, backend, ..
    } = settle_scroll_fixture(
        &[Some(336.0), Some(64.0)],
        Scripted::default_fake(),
        true,
        3,
    );
    teach_gearing(&manager).await;
    let captures_before = captures(&backend);

    let (result, elapsed) = timed_scroll(&manager).await;

    assert_eq!(settle_waited(elapsed), 1, "elapsed {elapsed:?}");
    assert_eq!(telemetry(&result).traveled_y, Some(0.0));
    assert_eq!(captures(&backend), captures_before + 3);

    manager.dispose().await;
}

#[tokio::test(start_paused = true)]
async fn does_not_let_a_waived_settle_upgrade_a_dispatched_unknown_verdict() {
    let backend = Scripted::default_fake();
    backend.hooks().scroll = Some(Arc::new(
        |fake, (point, delta_x, delta_y, window_id, modifiers, target)| {
            async move {
                fake.scroll(
                    point,
                    delta_x,
                    delta_y,
                    window_id.as_deref(),
                    &modifiers,
                    target.as_ref(),
                )
                .await?;
                Ok(ComputerBackendActionResult {
                    delivery_path: Some("fake-scroll".into()),
                    verified: Some(ComputerDeliveryVerification::Unconfirmed),
                    effect: Some(CuaEffect::DispatchedUnknown),
                    ..ComputerBackendActionResult::default()
                })
            }
            .boxed()
        },
    ));
    let Fixture { manager, .. } =
        settle_scroll_fixture(&[Some(336.0), Some(64.0), Some(400.0)], backend, true, 16);
    teach_gearing(&manager).await;

    let (result, elapsed) = timed_scroll(&manager).await;

    assert_eq!(settle_waited(elapsed), 0, "elapsed {elapsed:?}");
    assert_eq!(telemetry(&result).traveled_y, Some(400.0));
    assert_eq!(
        result.result.delivery,
        Some(ComputerDelivery {
            path: "fake-scroll".into(),
            verified: ComputerDeliveryVerification::Unconfirmed,
            effect: Some(CuaEffect::DispatchedUnknown),
        })
    );

    manager.dispose().await;
}

#[tokio::test]
async fn measures_a_macos_scroll_inside_the_two_leg_three_capture_budget() {
    let backend = Scripted::new(FakeComputerBackendOptions {
        agent_dialect: Some(ComputerAgentDialect::Macos),
        ..FakeComputerBackendOptions::default()
    });
    let Built {
        manager, backend, ..
    } = settled(backend);

    let outcome = scroll_at(&manager, 1_100.0, 200.0, 400.0, true).await;
    assert_eq!(backend.calls_for("scroll").len(), 2);
    assert_eq!(captures(&backend), 3);
    assert!(outcome.observation.is_some());
    assert_eq!(telemetry(&outcome).traveled_y, Some(0.0));
    assert_eq!(
        telemetry(&outcome).requested,
        ComputerScrollDelta {
            delta_x: 0.0,
            delta_y: 400.0
        }
    );
    assert_eq!(
        telemetry(&outcome).routes,
        Some(vec!["wheel".into(), "wheel".into()])
    );
    let unobserved = scroll_at(&manager, 1_100.0, 200.0, 400.0, false).await;
    assert_eq!(unobserved.observation, None);
    assert_eq!(captures(&backend), 3);

    manager.dispose().await;
}
