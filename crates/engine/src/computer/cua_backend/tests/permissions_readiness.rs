use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use serde_json::json;
use tokio::time::Instant;
use zeron_proto::computer::{
    CUA_SETUP_TIMEOUT_MS, ComputerAvailability, ComputerDeliveryVerification, ComputerHealthStatus,
    ComputerInputModifier, ComputerPermission, ComputerScreenshot,
};

use super::*;
use crate::computer::backend::{ComputerSettleOptions, ComputerSettleOutcome, CuaErrorLayer};
use crate::computer::cua_backend::CUA_HOST_CALL_TIMEOUT_MS;

struct DeadlineHost {
    setup_delay: Duration,
    hang_setup: AtomicBool,
    hang_calls: AtomicBool,
    methods: Mutex<Vec<ComputerHostMethod>>,
}

#[async_trait]
impl ComputerHost for DeadlineHost {
    async fn request(&self, call: ComputerHostCall) -> CuaReply {
        self.methods.lock().unwrap().push(call.method);
        match call.method {
            ComputerHostMethod::Setup if self.hang_setup.load(Ordering::SeqCst) => {
                std::future::pending::<()>().await
            }
            ComputerHostMethod::Setup => tokio::time::sleep(self.setup_delay).await,
            ComputerHostMethod::Call if self.hang_calls.load(Ordering::SeqCst) => {
                std::future::pending::<()>().await
            }
            _ => {}
        }
        serde_json::from_value(json!({
            "ok": true,
            "result": { "structuredContent": { "accessibility": false, "screen_recording": false } },
        }))
        .unwrap()
    }

    fn cancel(&self, _correlation_id: &str) {}
}

fn kind(availability: &ComputerAvailability) -> &'static str {
    match availability {
        ComputerAvailability::Available { .. } => "available",
        ComputerAvailability::UnsupportedPlatform { .. } => "unsupported-platform",
        ComputerAvailability::PermissionRequired { .. } => "permission-required",
        ComputerAvailability::BackendUnavailable { .. } => "backend-unavailable",
    }
}

fn message(availability: &ComputerAvailability) -> &str {
    match availability {
        ComputerAvailability::PermissionRequired { message, .. }
        | ComputerAvailability::BackendUnavailable { message } => message,
        _ => "",
    }
}

async fn until(done: impl Fn() -> bool) {
    for _ in 0..2_000 {
        if done() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    panic!("condition never held");
}

fn linux() -> Fixture {
    fixture_with(FixtureOptions {
        host_platform: Some("linux"),
        ..Default::default()
    })
}

fn linux_x11_permissions(f: &Fixture) {
    f.on_tool("check_permissions", |_| {
        json!({ "structuredContent": { "atspi": true, "x11": true, "wayland": false, "wayland_enabled": false } })
    });
}

fn overview() -> ComputerStateOptions {
    ComputerStateOptions {
        include_screenshot: true,
        ..ComputerStateOptions::default()
    }
}

#[tokio::test(start_paused = true)]
async fn allows_the_bounded_native_permission_request_to_finish_without_extending_action_deadlines()
{
    let host = Arc::new(DeadlineHost {
        setup_delay: Duration::from_millis(CUA_SETUP_TIMEOUT_MS - 1_000),
        hang_setup: AtomicBool::new(false),
        hang_calls: AtomicBool::new(false),
        methods: Mutex::new(Vec::new()),
    });
    let backend = CuaComputerBackend::new(host.clone(), CuaComputerBackendOptions::default());
    assert!(backend.provision().await.unwrap().contains("Allow"));
    let methods = host.methods.lock().unwrap().clone();
    assert_eq!(methods[0], ComputerHostMethod::Setup);
    assert!(
        methods[1..]
            .iter()
            .all(|method| *method == ComputerHostMethod::Call)
    );
    host.hang_calls.store(true, Ordering::SeqCst);
    let started = Instant::now();
    let availability = backend.availability(true).await.unwrap();
    let elapsed = started.elapsed();
    assert_eq!(kind(&availability), "backend-unavailable");
    assert!(message(&availability).contains("timed out"));
    assert!(elapsed >= Duration::from_millis(CUA_HOST_CALL_TIMEOUT_MS));
    assert!(elapsed < Duration::from_millis(CUA_HOST_CALL_TIMEOUT_MS + 1_000));
    host.hang_setup.store(true, Ordering::SeqCst);
    let started = Instant::now();
    let error = backend.provision().await.unwrap_err();
    let elapsed = started.elapsed();
    assert!(error.to_string().contains("timed out"));
    assert!(elapsed >= Duration::from_millis(CUA_SETUP_TIMEOUT_MS));
    assert!(elapsed < Duration::from_millis(CUA_SETUP_TIMEOUT_MS + 1_000));
}

#[tokio::test(start_paused = true)]
async fn reports_only_the_current_missing_permission_and_the_responsible_app() {
    let f = fixture();
    f.deny_screen_recording();
    let availability = f.backend.availability(false).await.unwrap();
    let ComputerAvailability::PermissionRequired {
        missing,
        bundle_id,
        message,
        ..
    } = &availability
    else {
        panic!("expected permission-required, got {availability:?}");
    };
    assert_eq!(missing, &vec![ComputerPermission::ScreenRecording]);
    assert_eq!(bundle_id.as_deref(), Some("com.synara.test"));
    assert!(!message.contains("Accessibility"));
    assert!(
        f.backend
            .provision()
            .await
            .unwrap()
            .contains("Allow Screen Recording")
    );
}

#[tokio::test(start_paused = true)]
async fn an_explicit_status_refresh_consumes_a_newly_granted_permission_without_waiting_for_the_action_cache()
 {
    let f = fixture();
    f.deny_permissions();
    assert_eq!(
        kind(&f.backend.availability(false).await.unwrap()),
        "permission-required"
    );
    f.grant_permissions();
    assert_eq!(
        kind(&f.backend.availability(false).await.unwrap()),
        "permission-required"
    );
    assert_eq!(
        kind(&f.backend.availability(true).await.unwrap()),
        "available"
    );
}

#[tokio::test]
async fn does_not_reuse_an_in_flight_permission_denial_for_a_grant_triggered_status_refresh() {
    let f = fixture();
    f.deny_permissions();
    f.backend.availability(false).await.unwrap();
    let previous = f.named("check_permissions").len();
    let (opener, gate) = gate();
    f.wait_for_permission(Some(gate));
    let old = f.backend.availability(true);
    tokio::pin!(old);
    assert!(futures::poll!(&mut old).is_pending());
    until(|| f.named("check_permissions").len() == previous + 1).await;
    let updated = f.backend.availability(true);
    tokio::pin!(updated);
    assert!(futures::poll!(&mut updated).is_pending());
    f.grant_permissions();
    opener.open();
    assert_eq!(kind(&old.await.unwrap()), "permission-required");
    assert_eq!(kind(&updated.await.unwrap()), "available");
}

#[tokio::test]
async fn refreshes_after_a_pre_setup_check_settles_and_reports_granted_permissions_accurately() {
    let f = fixture();
    f.deny_permissions();
    let (opener, gate) = gate();
    f.wait_for_permission(Some(gate));
    let previous = f.backend.availability(false);
    tokio::pin!(previous);
    assert!(futures::poll!(&mut previous).is_pending());
    until(|| f.named("check_permissions").len() == 1).await;
    let provision = f.backend.provision();
    tokio::pin!(provision);
    assert!(futures::poll!(&mut provision).is_pending());
    f.grant_permissions();
    opener.open();
    previous.await.unwrap();
    assert!(provision.await.unwrap().contains("permissions are ready"));
    assert_eq!(
        kind(&f.backend.availability(false).await.unwrap()),
        "available"
    );
}

#[tokio::test(start_paused = true)]
async fn names_input_monitoring_when_the_physical_interruption_listener_lacks_its_grant() {
    let f = fixture_with(FixtureOptions {
        host_platform: Some("darwin"),
        ..Default::default()
    });
    f.set_input_monitor(false, false);
    let availability = f.backend.availability(false).await.unwrap();
    let ComputerAvailability::PermissionRequired {
        missing, message, ..
    } = &availability
    else {
        panic!("expected permission-required, got {availability:?}");
    };
    assert_eq!(missing, &vec![ComputerPermission::InputMonitoring]);
    assert!(message.contains("Input Monitoring"));
    let decoded: ComputerAvailability =
        serde_json::from_value(serde_json::to_value(&availability).unwrap()).unwrap();
    assert_eq!(decoded, availability);
    assert!(
        f.backend
            .provision()
            .await
            .unwrap()
            .contains("Allow Input Monitoring")
    );
}

#[tokio::test]
async fn reports_a_failed_listener_separately_from_permissions_and_recovers_after_it_starts() {
    let f = fixture_with(FixtureOptions {
        host_platform: Some("darwin"),
        ..Default::default()
    });
    f.set_input_monitor(true, false);
    let availability = f.backend.availability(false).await.unwrap();
    assert_eq!(kind(&availability), "backend-unavailable");
    assert!(message(&availability).contains("Escape and human-input listener"));
    let health = f.backend.health();
    assert_eq!(health.status, ComputerHealthStatus::Unavailable);
    assert!(health.capture_available);
    assert!(
        !f.backend
            .provision()
            .await
            .unwrap()
            .contains("permissions are ready")
    );
    f.set_input_monitor(true, true);
    assert!(
        f.backend
            .provision()
            .await
            .unwrap()
            .contains("permissions are ready")
    );
    assert_eq!(
        kind(&f.backend.availability(false).await.unwrap()),
        "available"
    );
    assert_eq!(f.backend.health().status, ComputerHealthStatus::Connected);
}

#[tokio::test]
async fn does_not_impose_macos_input_monitoring_on_a_remote_linux_host() {
    let f = linux();
    f.set_input_monitor(false, false);
    assert_eq!(
        kind(&f.backend.availability(false).await.unwrap()),
        "available"
    );
}

#[tokio::test]
async fn recognizes_native_linux_display_and_at_spi_prerequisites_without_inventing_tcc_grants() {
    let f = linux();
    linux_x11_permissions(&f);
    assert_eq!(
        kind(&f.backend.availability(false).await.unwrap()),
        "available"
    );
    assert!(f.backend.missing_permissions().await.is_empty());
    assert!(f.backend.health().capture_available);
    let capabilities = f.backend.capabilities();
    assert!(!capabilities.input);
    assert!(!capabilities.focus);
    assert!(!capabilities.raise);
    assert!(capabilities.capture);
    assert!(capabilities.windows);
}

#[tokio::test]
async fn reports_unavailable_wayland_geometry_without_failing_status_or_concealing_it_behind_a_live_socket()
 {
    let f = linux();
    f.on_tool("check_permissions", |_| {
        json!({ "structuredContent": { "atspi": true, "x11": false, "wayland": true, "wayland_enabled": true } })
    });
    f.on_tool("get_screen_size", |_| {
        json!({
            "isError": true,
            "content": [{ "type": "text", "text": "$DISPLAY variable not set and no value was provided explicitly" }],
        })
    });
    let unavailable = f.backend.availability(false).await.unwrap();
    assert_eq!(kind(&unavailable), "backend-unavailable");
    assert!(message(&unavailable).contains("$DISPLAY"));
    let health = f.backend.health();
    assert_eq!(health.status, ComputerHealthStatus::Unavailable);
    assert!(!health.capture_available);
    let before_probe = f.call_count();
    assert_eq!(f.backend.probe_availability().await.unwrap(), unavailable);
    assert!(
        f.calls()[before_probe..]
            .iter()
            .all(|call| call.name.as_deref() != Some("get_screen_size"))
    );
    let error = f.backend.get_screen_size().await.unwrap_err();
    assert!(error.to_string().contains("$DISPLAY"));
    f.on_tool(
        "get_screen_size",
        |_| json!({ "structuredContent": { "width": 1280, "height": 800 } }),
    );
    assert_eq!(
        kind(&f.backend.availability(false).await.unwrap()),
        "available"
    );
    let size = f.backend.get_screen_size().await.unwrap();
    assert_eq!((size.width, size.height), (1280, 800));
}

#[tokio::test]
async fn recovers_linux_capture_only_after_real_pixels_not_a_compositor_permission_probe() {
    let f = linux();
    linux_x11_permissions(&f);
    f.backend.availability(false).await.unwrap();
    f.fail_overview();
    let error = f.backend.get_state(overview()).await.unwrap_err();
    assert!(error.to_string().contains("Capture denied"));
    assert!(!f.backend.health().capture_available);
    f.backend.provision().await.unwrap();
    assert!(!f.backend.health().capture_available);
    f.backend
        .capture_screenshot(window_capture("cua:10:20"))
        .await
        .unwrap();
    let health = f.backend.health();
    assert_eq!(health.status, ComputerHealthStatus::Connected);
    assert!(health.capture_available);
    assert_eq!(health.consecutive_failures, 0);
}

#[tokio::test]
async fn re_probes_a_transient_missing_report_before_publishing_availability() {
    let f = fixture();
    f.deny_permissions();
    let (opener, gate) = gate();
    f.wait_for_permission(Some(gate));
    let pending = f.backend.availability(false);
    tokio::pin!(pending);
    assert!(futures::poll!(&mut pending).is_pending());
    tokio::time::sleep(Duration::from_millis(10)).await;
    f.grant_permissions();
    opener.open();
    assert_eq!(kind(&pending.await.unwrap()), "available");
}

fn arm(f: &Fixture, openers: &mut VecDeque<GateOpener>) {
    let (opener, gate) = gate();
    f.wait_for_permission(Some(gate));
    openers.push_back(opener);
}

async fn tick() {
    tokio::time::sleep(Duration::from_millis(10)).await;
}

#[tokio::test]
async fn keeps_re_probing_until_a_delayed_grant_lands() {
    let f = fixture();
    f.deny_permissions();
    let mut openers = VecDeque::new();
    arm(&f, &mut openers);
    let pending = f.backend.availability(false);
    tokio::pin!(pending);
    assert!(futures::poll!(&mut pending).is_pending());
    tick().await;
    openers.pop_front().unwrap().open();
    tick().await;
    arm(&f, &mut openers);
    tick().await;
    openers.pop_front().unwrap().open();
    tick().await;
    arm(&f, &mut openers);
    f.grant_permissions();
    tick().await;
    openers.pop_front().unwrap().open();
    assert_eq!(kind(&pending.await.unwrap()), "available");
}

#[tokio::test]
async fn clears_an_old_capture_failure_after_explicit_setup_so_recovery_can_be_retried() {
    let f = fixture();
    f.fail_overview();
    let error = f.backend.get_state(overview()).await.unwrap_err();
    assert!(error.to_string().contains("Capture denied"));
    assert!(!f.backend.health().capture_available);
    f.backend.provision().await.unwrap();
    let health = f.backend.health();
    assert_eq!(health.status, ComputerHealthStatus::Connected);
    assert!(health.capture_available);
    assert_eq!(f.named("get_desktop_state").len(), 1);
}

#[tokio::test]
async fn marks_scoped_model_state_reads_and_never_marks_a_pane_still_as_a_model_observation() {
    let f = fixture();
    f.backend
        .capture_screenshot(window_capture("cua:10:20"))
        .await
        .unwrap();
    assert_eq!(
        f.named("get_window_state")[0].model_observation,
        Some(false)
    );
    f.backend.focus_window("cua:10:20").await.unwrap();
    f.clear_calls();
    observing(async {
        f.backend
            .get_state(window_state("cua:10:20"))
            .await
            .unwrap();
        f.backend.get_state(overview()).await.unwrap();
        assert!(f.backend.capture_still().await.unwrap().is_some());
    })
    .await;
    let marks = |name: &str| -> Vec<Option<bool>> {
        f.named(name)
            .iter()
            .map(|call| call.model_observation)
            .collect()
    };
    assert_eq!(marks("get_window_state"), vec![Some(true), Some(false)]);
    assert_eq!(marks("get_desktop_state"), vec![Some(true)]);
    assert!(
        f.calls()
            .iter()
            .filter(|call| !matches!(
                call.name.as_deref(),
                Some("get_window_state" | "get_desktop_state")
            ))
            .all(|call| call.model_observation.is_none())
    );
    f.backend.dispose().await;
}

#[tokio::test]
async fn invalidates_old_coordinate_grounding_when_lock_and_resume_occurred_between_requests() {
    let f = fixture();
    f.backend
        .capture_screenshot(window_capture("cua:10:20"))
        .await
        .unwrap();
    f.change_desktop();
    let error = f
        .backend
        .click(point(-275.0, 30.0), Some("cua:10:20"), &[])
        .await
        .unwrap_err();
    assert_eq!(effect(&error), Some(CuaEffect::NotDispatched));
    assert_eq!(code(&error), Some("stale_geometry"));
    assert!(f.named("click").is_empty());
    observing(f.backend.capture_screenshot(window_capture("cua:10:20")))
        .await
        .unwrap();
    f.backend
        .click(point(-275.0, 30.0), Some("cua:10:20"), &[])
        .await
        .unwrap();
}

#[tokio::test]
async fn rejects_a_delayed_observation_from_before_a_known_desktop_interruption() {
    let f = fixture();
    let (finish, gate) = gate();
    f.delay_overview(gate);
    let (observed, ()) = tokio::join!(f.backend.get_state(overview()), async {
        until(|| !f.named("get_desktop_state").is_empty()).await;
        f.change_desktop();
        f.backend.check_input_ready("cua:10:20").await.unwrap();
        finish.open();
    });
    let error = observed.unwrap_err();
    assert_eq!(effect(&error), Some(CuaEffect::NotDispatched));
    assert_eq!(code(&error), Some("stale_desktop_epoch"));
}

fn names(f: &Fixture) -> Vec<String> {
    f.calls()
        .iter()
        .map(|call| call.name.clone().unwrap_or_default())
        .collect()
}

#[tokio::test]
async fn checks_exact_native_input_readiness_without_capturing_or_dispatching_input() {
    let f = fixture();
    f.backend.check_input_ready("cua:10:20").await.unwrap();
    assert_eq!(names(&f), vec!["list_windows", "check_input_ready"]);
    assert_eq!(f.calls()[1].args, json!({ "pid": 10, "window_id": 20 }));
    f.readiness(json!({ "ready": true, "pid": 10, "window_id": 21 }));
    let error = f.backend.check_input_ready("cua:10:20").await.unwrap_err();
    assert_eq!(code(&error), Some("invalid_readiness"));
    assert_eq!(effect(&error), Some(CuaEffect::NotDispatched));
    f.readiness(json!({ "ready": false }));
    let error = f.backend.check_input_ready("cua:10:20").await.unwrap_err();
    assert_eq!(code(&error), Some("invalid_readiness"));
}

#[tokio::test]
async fn refreshes_linux_readiness_from_exact_visible_window_identity_without_a_missing_native_tool()
 {
    let f = linux();
    f.backend.check_input_ready("cua:10:20").await.unwrap();
    assert_eq!(names(&f), vec!["list_windows"]);
    f.set_visible(false);
    let error = f.backend.check_input_ready("cua:10:20").await.unwrap_err();
    assert_eq!(code(&error), Some("target_not_on_active_space"));
    assert_eq!(effect(&error), Some(CuaEffect::NotDispatched));
    f.close();
    let error = f.backend.check_input_ready("cua:10:20").await.unwrap_err();
    assert_eq!(code(&error), Some("stale_target"));
    assert_eq!(effect(&error), Some(CuaEffect::NotDispatched));
    assert!(names(&f).iter().all(|name| name == "list_windows"));
}

fn settle(timeout_ms: f64, quiet_ms: f64) -> ComputerSettleOptions {
    ComputerSettleOptions {
        window_id: "cua:10:20".into(),
        timeout_ms,
        quiet_ms,
    }
}

#[tokio::test]
async fn asks_the_driver_to_observe_settle_for_the_exact_window_without_touching_input() {
    let f = fixture();
    f.on_tool("wait_for_settle", |args| {
        json!({ "structuredContent": {
            "settled": true,
            "waited_ms": 120,
            "events_seen": 3,
            "pid": args.get("pid"),
            "window_id": args.get("window_id"),
            "scope": "window",
        } })
    });
    let outcome = f
        .backend
        .wait_for_settle(settle(5_000.0, 1_000.0))
        .await
        .unwrap();
    assert_eq!(
        outcome,
        ComputerSettleOutcome {
            settled: true,
            waited_ms: 120.0,
            events_seen: Some(3.0),
        }
    );
    assert_eq!(names(&f), vec!["list_windows", "wait_for_settle"]);
    assert_eq!(
        f.calls()[1].args,
        json!({ "pid": 10, "window_id": 20, "timeout_ms": 5000, "quiet_ms": 1000 })
    );
}

#[tokio::test]
async fn clamps_the_settle_bounds_the_driver_caps_and_refuses_a_malformed_verdict() {
    let f = fixture();
    f.on_tool("wait_for_settle", |_| {
        json!({ "structuredContent": { "settled": false, "waited_ms": 30000, "events_seen": 41 } })
    });
    let outcome = f
        .backend
        .wait_for_settle(settle(999_999.0, 999_999.0))
        .await
        .unwrap();
    assert!(!outcome.settled);
    assert_eq!(outcome.events_seen, Some(41.0));
    assert_subset(
        &f.calls()[1].args,
        json!({ "timeout_ms": 30000, "quiet_ms": 5000 }),
    );
    f.on_tool(
        "wait_for_settle",
        |_| json!({ "structuredContent": { "waited_ms": 5 } }),
    );
    let error = f
        .backend
        .wait_for_settle(settle(1_000.0, 100.0))
        .await
        .unwrap_err();
    assert_eq!(code(&error), Some("invalid_settle_read"));
    assert_eq!(effect(&error), Some(CuaEffect::NotDispatched));
    f.on_tool("wait_for_settle", |_| {
        json!({
            "isError": true,
            "content": [{ "type": "text", "text": "Unknown tool: wait_for_settle" }],
        })
    });
    let error = f
        .backend
        .wait_for_settle(settle(1_000.0, 100.0))
        .await
        .unwrap_err();
    assert_eq!(effect(&error), Some(CuaEffect::NotDispatched));
}

#[tokio::test]
async fn pauses_input_on_another_space_while_leaving_observation_available() {
    let f = fixture();
    f.set_visible(false);
    let error = f
        .backend
        .type_text("abc", Some("cua:10:20"), None)
        .await
        .unwrap_err();
    assert_eq!(effect(&error), Some(CuaEffect::NotDispatched));
    assert_eq!(code(&error), Some("target_not_on_active_space"));
    assert_eq!(
        error
            .input_pause()
            .and_then(|pause| pause.window_id.as_deref()),
        Some("cua:10:20")
    );
    assert!(f.named("type_text").is_empty());
    let state = f
        .backend
        .get_state(ComputerStateOptions {
            window_id: Some("cua:10:20".into()),
            ..ComputerStateOptions::default()
        })
        .await
        .unwrap();
    assert_eq!(state.computer_id, "desktop");
}

#[tokio::test]
async fn preserves_a_native_off_space_refusal_during_read_only_readiness() {
    let f = fixture();
    f.readiness(json!({
        "ready": false,
        "effect": "refused",
        "code": "target_not_on_active_space",
        "pid": 10,
        "window_id": 20,
        "reason": "The target is on another Space.",
    }));
    let error = f.backend.check_input_ready("cua:10:20").await.unwrap_err();
    assert_eq!(effect(&error), Some(CuaEffect::NotDispatched));
    assert_eq!(code(&error), Some("target_not_on_active_space"));
    let pause = error.input_pause().unwrap();
    assert_eq!(pause.window_id.as_deref(), Some("cua:10:20"));
    assert_eq!(pause.message, "The target is on another Space.");
    assert_eq!(names(&f), vec!["list_windows", "check_input_ready"]);
}

#[tokio::test]
async fn maps_only_proven_native_space_refusals_to_recoverable_pause() {
    let f = fixture();
    f.action_result(json!({
        "effect": "refused",
        "code": "target_not_on_active_space",
        "message": "Target is on another Space.",
    }));
    let error = f
        .backend
        .type_text("abc", Some("cua:10:20"), None)
        .await
        .unwrap_err();
    assert_eq!(effect(&error), Some(CuaEffect::NotDispatched));
    assert_eq!(
        error
            .input_pause()
            .and_then(|pause| pause.window_id.as_deref()),
        Some("cua:10:20")
    );
    f.fail(json!({ "ok": false, "error": "Space changed after dispatch", "effect": "dispatched-unknown" }));
    let error = f
        .backend
        .type_text("abc", Some("cua:10:20"), None)
        .await
        .unwrap_err();
    assert_eq!(effect(&error), Some(CuaEffect::DispatchedUnknown));
    assert!(error.input_pause().is_none());
    assert_eq!(f.named("type_text").len(), 2);
}

#[tokio::test]
async fn rejects_a_drag_if_its_prepared_window_moves_before_input_admission() {
    let f = fixture();
    f.backend
        .capture_screenshot(window_capture("cua:10:20"))
        .await
        .unwrap();
    f.move_window();
    let error = foreground(f.backend.drag(
        point(-275.0, 30.0),
        point(-225.0, 50.0),
        500.0,
        Some("cua:10:20"),
    ))
    .await
    .unwrap_err();
    assert_eq!(effect(&error), Some(CuaEffect::NotDispatched));
    assert_eq!(code(&error), Some("stale_geometry"));
    assert!(f.named("drag").is_empty());
}

#[tokio::test]
async fn binds_foreground_drag_pixels_to_the_exact_observed_native_bounds() {
    let f = fixture();
    f.backend
        .capture_screenshot(window_capture("cua:10:20"))
        .await
        .unwrap();
    foreground(f.backend.drag(
        point(-275.0, 30.0),
        point(-225.0, 50.0),
        500.0,
        Some("cua:10:20"),
    ))
    .await
    .unwrap();
    let drags = f.named("drag");
    assert_eq!(drags.len(), 1);
    assert_eq!(
        drags[0].args,
        json!({
            "pid": 10,
            "window_id": 20,
            "delivery_mode": "foreground",
            "from_x": 25,
            "from_y": 10,
            "to_x": 75,
            "to_y": 30,
            "coordinate_space": "window_points",
            "duration_ms": 500,
            "expected_window_bounds": { "x": -300, "y": 20, "width": 200, "height": 100 },
        })
    );
}

#[tokio::test]
async fn preserves_capture_identity_and_rejects_a_different_native_window() {
    let f = fixture();
    let image = f
        .backend
        .capture_screenshot(window_capture("cua:10:20"))
        .await
        .unwrap();
    let decoded: ComputerScreenshot =
        serde_json::from_value(serde_json::to_value(&image).unwrap()).unwrap();
    assert_eq!(decoded.window_id.as_deref(), Some("cua:10:20"));
    let state = f
        .backend
        .get_state(ComputerStateOptions {
            window_id: Some("cua:10:20".into()),
            include_screenshot: true,
            ..ComputerStateOptions::default()
        })
        .await
        .unwrap();
    assert_eq!(
        state.screenshot.unwrap().window_id.as_deref(),
        Some("cua:10:20")
    );
    f.capture_window(21, 10);
    let error = f
        .backend
        .capture_screenshot(window_capture("cua:10:20"))
        .await
        .unwrap_err();
    assert_eq!(effect(&error), Some(CuaEffect::NotDispatched));
    let error = f
        .backend
        .get_state(window_state("cua:10:20"))
        .await
        .unwrap_err();
    assert_eq!(effect(&error), Some(CuaEffect::NotDispatched));
    assert!(f.named("click").is_empty());
}

#[tokio::test]
async fn clears_targeting_after_desktop_pause_and_requires_a_fresh_observation() {
    let f = fixture();
    f.backend
        .capture_screenshot(window_capture("cua:10:20"))
        .await
        .unwrap();
    f.pause_desktop(true);
    let error = f
        .backend
        .click(point(-275.0, 30.0), Some("cua:10:20"), &[])
        .await
        .unwrap_err();
    assert_eq!(effect(&error), Some(CuaEffect::NotDispatched));
    assert_eq!(code(&error), Some("computer_input_paused"));
    assert_eq!(error.cua().unwrap().layer, Some(CuaErrorLayer::DriverHost));
    assert_eq!(
        error
            .input_pause()
            .and_then(|pause| pause.window_id.as_deref()),
        Some("cua:10:20")
    );
    f.pause_desktop(false);
    let error = f
        .backend
        .click(point(-275.0, 30.0), Some("cua:10:20"), &[])
        .await
        .unwrap_err();
    assert_eq!(code(&error), Some("stale_geometry"));
    f.backend
        .capture_screenshot(window_capture("cua:10:20"))
        .await
        .unwrap();
    f.backend
        .click(point(-275.0, 30.0), Some("cua:10:20"), &[])
        .await
        .unwrap();
}

#[tokio::test]
async fn dispatches_logical_coordinate_input_without_a_preparation_png() {
    let f = fixture();
    f.backend
        .capture_screenshot(window_capture("cua:10:20"))
        .await
        .unwrap();
    f.clear_calls();
    f.backend
        .click(point(-275.0, 30.0), Some("cua:10:20"), &[])
        .await
        .unwrap();
    f.backend
        .scroll(
            Some(point(-275.0, 30.0)),
            0.0,
            120.0,
            Some("cua:10:20"),
            &[],
            None,
        )
        .await
        .unwrap();
    foreground(f.backend.drag(
        point(-275.0, 30.0),
        point(-225.0, 50.0),
        500.0,
        Some("cua:10:20"),
    ))
    .await
    .unwrap();
    assert!(f.named("get_window_state").is_empty());
    let inputs: Vec<ComputerHostCall> = f
        .calls()
        .into_iter()
        .filter(|call| matches!(call.name.as_deref(), Some("click" | "scroll" | "drag")))
        .collect();
    assert_eq!(inputs.len(), 3);
    for call in inputs {
        assert_subset(
            &call.args,
            json!({
                "pid": 10,
                "window_id": 20,
                "coordinate_space": "window_points",
                "expected_window_bounds": { "x": -300, "y": 20, "width": 200, "height": 100 },
            }),
        );
    }
}

#[tokio::test]
async fn permits_advertised_axpress_for_plain_clicks_and_preserves_physical_click_gestures() {
    let f = fixture();
    let at = point(-275.0, 30.0);
    f.backend
        .capture_screenshot(window_capture("cua:10:20"))
        .await
        .unwrap();
    f.backend.click(at, Some("cua:10:20"), &[]).await.unwrap();
    f.backend
        .click(at, Some("cua:10:20"), &[ComputerInputModifier::Shift])
        .await
        .unwrap();
    f.backend
        .double_click(at, Some("cua:10:20"), &[])
        .await
        .unwrap();
    f.backend
        .triple_click(at, Some("cua:10:20"), &[])
        .await
        .unwrap();
    foreground(f.backend.right_click(at, Some("cua:10:20"), &[]))
        .await
        .unwrap();
    let clicks: Vec<Value> = f.named("click").into_iter().map(|call| call.args).collect();
    assert_eq!(clicks.len(), 5);
    assert!(clicks[0].get("force_synthetic").is_none());
    for click in &clicks[1..] {
        assert_eq!(click["force_synthetic"], json!(true));
    }
    assert_subset(&clicks[1], json!({ "modifier": ["shift"] }));
    assert_subset(&clicks[2], json!({ "count": 2 }));
    assert_subset(&clicks[3], json!({ "count": 3 }));
    assert_subset(&clicks[4], json!({ "button": "right" }));
}

#[tokio::test]
async fn refuses_a_background_right_click_whose_context_menu_it_could_not_observe() {
    let f = fixture();
    let at = point(-275.0, 30.0);
    f.backend
        .capture_screenshot(window_capture("cua:10:20"))
        .await
        .unwrap();
    let error = f
        .backend
        .right_click(at, Some("cua:10:20"), &[])
        .await
        .unwrap_err();
    assert_eq!(effect(&error), Some(CuaEffect::NotDispatched));
    assert_eq!(code(&error), Some("context_menu_unobservable"));
    assert!(f.named("click").is_empty());

    foreground(f.backend.right_click(at, Some("cua:10:20"), &[]))
        .await
        .unwrap();
    assert_subset(&f.first_args("click"), json!({ "button": "right" }));
}

#[tokio::test]
async fn keeps_plain_clicks_synthetic_for_older_or_unknown_native_revision() {
    for native_revision in [Some(0), Some(33), None] {
        let f = fixture_with(FixtureOptions {
            native_revision: Some(native_revision),
            ..Default::default()
        });
        f.backend
            .capture_screenshot(window_capture("cua:10:20"))
            .await
            .unwrap();
        f.backend
            .click(point(-275.0, 30.0), Some("cua:10:20"), &[])
            .await
            .unwrap();
        assert_subset(&f.first_args("click"), json!({ "force_synthetic": true }));
    }
}

#[tokio::test]
async fn carries_safe_actuator_diagnostics_for_uncertain_clicks_without_replaying() {
    let f = fixture();
    f.backend
        .capture_screenshot(window_capture("cua:10:20"))
        .await
        .unwrap();
    f.on_tool("click", |_| {
        json!({
            "isError": true,
            "structuredContent": {
                "diagnostics": {
                    "delivery_path": "ax",
                    "actuator": "ax_press",
                    "error_code": "ax_dispatch_failed",
                    "ax_error": -25202,
                    "message": "private focused content",
                    "text": "private typed text",
                },
            },
        })
    });
    let error = f
        .backend
        .click(point(-275.0, 30.0), Some("cua:10:20"), &[])
        .await
        .unwrap_err();
    let failure = error.cua().expect("a Cua action error");
    assert_eq!(failure.code, "cua_action_failed");
    assert_eq!(failure.effect, CuaEffect::DispatchedUnknown);
    let diagnostics = serde_json::to_value(&failure.diagnostics).unwrap();
    assert_subset(
        &diagnostics,
        json!({
            "delivery_path": "ax",
            "actuator": "ax_press",
            "error_code": "ax_dispatch_failed",
            "ax_error": -25202,
        }),
    );
    assert!(!diagnostics.to_string().contains("private"));
    assert_eq!(f.named("click").len(), 1);
}

#[tokio::test]
async fn preserves_an_explicit_not_dispatched_error_and_retains_usable_observation_geometry() {
    let f = fixture();
    f.backend
        .capture_screenshot(window_capture("cua:10:20"))
        .await
        .unwrap();
    f.on_tool("click", |_| {
        json!({
            "isError": true,
            "structuredContent": {
                "effect": "not-dispatched",
                "code": "ax_action_refused",
                "diagnostics": { "error_code": "ax_action_refused", "delivery_path": "ax" },
            },
        })
    });
    let error = f
        .backend
        .click(point(-275.0, 30.0), Some("cua:10:20"), &[])
        .await
        .unwrap_err();
    assert_eq!(effect(&error), Some(CuaEffect::NotDispatched));
    assert_eq!(code(&error), Some("ax_action_refused"));
    f.on_tool(
        "click",
        |_| json!({ "structuredContent": { "effect": "unverifiable" } }),
    );
    let result = f
        .backend
        .click(point(-270.0, 30.0), Some("cua:10:20"), &[])
        .await
        .unwrap();
    assert_eq!(result.effect, Some(CuaEffect::DispatchedUnknown));
    assert_eq!(f.named("click").len(), 2);
}

#[tokio::test]
async fn never_downgrades_explicit_input_to_a_legacy_status_refusal() {
    for native_effect in ["dispatched-unknown", "unverifiable", "confirmed"] {
        let f = fixture();
        f.on_tool("press_key", move |_| {
            json!({
                "isError": true,
                "structuredContent": { "effect": native_effect, "status": "refused", "code": "ax_action_refused" },
            })
        });
        let error = f
            .backend
            .press_key("enter", Some("cua:10:20"), None)
            .await
            .unwrap_err();
        assert_eq!(
            effect(&error),
            Some(CuaEffect::DispatchedUnknown),
            "{native_effect}"
        );
        assert_eq!(f.named("press_key").len(), 1);
    }
}

#[tokio::test]
async fn keeps_explicitly_approved_foreground_text_on_the_native_foreground_tool() {
    let f = fixture();
    foreground(f.backend.type_text("abc", Some("cua:10:20"), None))
        .await
        .unwrap();
    assert_subset(
        &f.first_args("type_text"),
        json!({ "delivery_mode": "foreground", "force_synthetic": true }),
    );
}

#[tokio::test]
async fn sends_background_hotkey_by_default_and_keeps_approved_foreground_hotkey() {
    let f = fixture();
    let keys = ["meta".to_owned(), "a".to_owned()];
    f.backend
        .hotkey(&keys, Some("cua:10:20"), None)
        .await
        .unwrap();
    assert_subset(
        &f.first_args("hotkey"),
        json!({ "delivery_mode": "background", "keys": ["command", "a"] }),
    );
    f.clear_calls();
    foreground(f.backend.hotkey(&keys, Some("cua:10:20"), None))
        .await
        .unwrap();
    assert_subset(
        &f.first_args("hotkey"),
        json!({ "delivery_mode": "foreground", "keys": ["command", "a"] }),
    );
}

#[tokio::test]
async fn keeps_movecursor_as_background_overlay_only_and_unverifiable() {
    let f = fixture();
    let result = with_desktop_delivery_mode(
        ComputerDeliveryMode::Background,
        f.backend.move_cursor(point(10.0, 20.0), Some("cua:10:20")),
    )
    .await
    .unwrap();
    assert_eq!(result.point, Some(point(10.0, 20.0)));
    assert_eq!(result.delivery_path.as_deref(), Some("cua-overlay-only"));
    assert_eq!(
        result.verified,
        Some(ComputerDeliveryVerification::Unverifiable)
    );
    assert_eq!(f.first_args("move_cursor"), json!({ "x": 10, "y": 20 }));
}
