use super::*;
use crate::computer::host::{ComputerInputMonitorState, PhysicalComputerInput, PhysicalInputKind};

fn keyboard(pid: u32) -> PhysicalComputerInput {
    PhysicalComputerInput {
        kind: PhysicalInputKind::Keyboard,
        pid: Some(pid),
        window_id: None,
    }
}

fn pointer(pid: u32, window_id: u32) -> PhysicalComputerInput {
    PhysicalComputerInput {
        kind: PhysicalInputKind::Pointer,
        pid: Some(pid),
        window_id: Some(window_id),
    }
}

fn type_text() -> ComputerHostCall {
    call("type_text").args(json!({ "text": "fixture" }))
}

fn stop() -> ComputerHostCall {
    method(ComputerHostMethod::Stop)
}

fn result_value(reply: &CuaReply) -> Value {
    serde_json::to_value(&reply.result).unwrap()
}

fn last_index(names: &[String], name: &str) -> usize {
    names.iter().rposition(|e| e == name).unwrap()
}

fn first_index(names: &[String], name: &str) -> usize {
    names.iter().position(|e| e == name).unwrap()
}

type MonitorState = Arc<Mutex<ComputerInputMonitorState>>;

fn monitor(state: &MonitorState) -> Box<dyn Fn() -> ComputerInputMonitorState + Send> {
    let state = state.clone();
    Box::new(move || state.lock().unwrap().clone())
}

#[tokio::test]
async fn ignores_the_press_when_nothing_is_driving_so_escape_stays_an_ordinary_key() {
    let f = fixture(Driver::default());
    assert!(!f.host.emergency_stop_input().await);
    assert!(f.send(press()).await.ok);
}

#[tokio::test]
async fn drains_native_input_keeps_the_generation_and_requires_fresh_observation_after_escape() {
    let releases = Arc::new(AtomicUsize::new(0));
    let seam = counting_release(releases.clone());
    let f = fixture_with(Driver::default(), |o| o.release_held_input = Some(seam));
    assert!(f.send(press()).await.ok);
    let hung = f.spawn(type_text());
    f.wait_for("dispatch").await;
    assert!(f.host.emergency_stop_input().await);
    assert!(!hung.reply().await.ok);
    f.wait_for("interrupt-ack").await;
    assert_eq!(releases.load(Ordering::SeqCst), 0);
    assert!(f.has("interrupt"));
    assert_eq!(f.count("release"), 1);
    assert!(!f.has("effect") && !f.has("cancel") && !f.has("retiring"));
    assert_eq!(f.count("start"), 1);
    let cooling = f.send(press()).await;
    assert!(cooling.ok);
    assert_paused(&cooling);
    assert_eq!(structured(&cooling)["effect"], json!("refused"));
    assert!(f.send(call("list_windows")).await.ok);
    past_cooldown().await;
    let stale = f.send(press()).await;
    assert_paused(&stale);
    assert_eq!(stale.desktop_interruptions, Some(0));
    f.send(call("get_window_state")).await;
    assert!(result_is_error(&f.send(call("check_input_ready")).await));
    f.send(call("get_window_state").observe(true)).await;
    assert_clean_ok(&f.send(press()).await);
    assert_eq!(f.count("start"), 1);
    assert_eq!(f.count("key"), 2);
    assert!(!f.has("retiring"));
}

#[tokio::test]
async fn interrupts_the_in_flight_action_on_the_backend_stop_verb_without_retiring() {
    let releases = Arc::new(AtomicUsize::new(0));
    let seam = counting_release(releases.clone());
    let f = fixture_with(Driver::default(), |o| o.release_held_input = Some(seam));
    assert!(f.send(press()).await.ok);
    let hung = f.spawn(type_text());
    f.wait_for("dispatch").await;
    assert!(f.send(stop()).await.ok);
    assert!(!hung.reply().await.ok);
    assert_eq!(releases.load(Ordering::SeqCst), 0);
    assert!(f.has("interrupt-ack"));
    assert!(!f.has("effect") && !f.has("cancel") && !f.has("retiring"));
    assert_eq!(f.count("start"), 1);
    assert!(f.send(press()).await.ok);
    assert_eq!(f.count("key"), 2);
    assert_eq!(f.count("start"), 1);
}

#[tokio::test]
async fn keeps_admission_closed_after_a_driver_crash_until_the_held_input_release_is_confirmed() {
    let release = Deferred::<()>::new();
    let gate = release.clone();
    let f = fixture_with(
        Driver {
            crash: true,
            ..Driver::default()
        },
        |o| {
            o.release_held_input = Some(Box::new(move || {
                let gate = gate.clone();
                Box::pin(async move {
                    gate.wait().await;
                    Ok(())
                })
            }))
        },
    );
    let crashing = f.spawn(type_text());
    f.wait_for("crash").await;
    assert!(f.host.emergency_stop_input().await);
    release.resolve(());
    assert!(!crashing.reply().await.ok);
    f.host.stop().await.unwrap();
    past_cooldown().await;
    f.send(call("get_window_state").observe(true)).await;
    assert_clean_ok(&f.send(press()).await);
}

#[tokio::test]
async fn stays_fail_closed_when_the_crash_cleanup_is_unconfirmed_with_no_escape_latch_involved() {
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
    assert!(!f.send(type_text()).await.ok);
    assert!(f.host.emergency_stop_input().await);
    assert_not_dispatched(&f.send(call("check_permissions")).await);
    assert_eq!(f.count("start"), 1);
}

#[tokio::test]
async fn keeps_input_closed_after_an_incomplete_native_interrupt_and_rechecks_the_drain_before_dispatch()
 {
    let releases = Arc::new(AtomicUsize::new(0));
    let seam = counting_release(releases.clone());
    let f = fixture_with(
        Driver {
            interrupt_cleanup: Some("once-incomplete".into()),
            ..Driver::default()
        },
        |o| o.release_held_input = Some(seam),
    );
    f.send(press()).await;
    assert!(!f.send(stop()).await.ok);
    assert_eq!(releases.load(Ordering::SeqCst), 1);
    assert_eq!(f.count("key"), 1);
    assert_clean_ok(&f.send(press()).await);
    let names = f.names();
    assert_eq!(f.count("interrupt"), 2);
    assert_eq!(f.count("key"), 2);
    assert!(last_index(&names, "interrupt-ack") < last_index(&names, "key"));
    assert_eq!(f.count("start"), 1);
}

#[tokio::test]
async fn never_resumes_input_on_an_unconfirmed_interruption_acknowledgement() {
    for cleanup in ["wrong-pid", "missing-admission", "incomplete"] {
        let f = fixture(Driver {
            interrupt_cleanup: Some(cleanup.into()),
            ..Driver::default()
        });
        f.send(press()).await;
        assert!(!f.send(stop()).await.ok, "{cleanup}");
        assert_not_dispatched(&f.send(press()).await);
        assert_eq!(f.count("key"), 1, "{cleanup}");
    }
}

#[tokio::test]
async fn pauses_only_the_humans_controlled_target_and_coalesces_repeated_physical_input() {
    let f = fixture(Driver {
        delay_observation: true,
        ..Driver::default()
    });
    let owner = task("takeover", Some("turn"));
    let args = json!({ "pid": 700, "window_id": 900, "key": "enter" });
    f.send(call("press_key").args(args.clone()).task(&owner))
        .await;
    assert!(!f.host.physical_input(keyboard(701)).await);
    assert!(!f.host.physical_input(pointer(700, 901)).await);
    assert!(f.host.physical_input(pointer(700, 900)).await);
    assert!(f.host.physical_input(keyboard(700)).await);
    f.wait_for("interrupt-ack").await;
    assert_eq!(f.count("interrupt"), 1);
    let paused = f.send(press()).await;
    assert_paused(&paused);
    assert_eq!(paused.desktop_interruptions, Some(0));
    let observe = || {
        call("get_window_state")
            .args(args.clone())
            .observe(true)
            .task(&owner)
    };
    let observation = f.spawn(observe());
    f.wait_for("observe").await;
    assert!(f.host.physical_input(keyboard(700)).await);
    let observed = observation.reply().await;
    assert!(result_is_error(&observed));
    assert_eq!(observed.desktop_interruptions, Some(0));
    past_cooldown().await;
    assert!(result_is_error(&f.send(press()).await));
    f.send(observe()).await;
    let resumed = f.send(press()).await;
    assert_clean_ok(&resumed);
    assert_eq!(resumed.desktop_interruptions, Some(0));
    f.send(method(ComputerHostMethod::EndTask).task(&owner))
        .await;
    assert!(!f.host.physical_input(keyboard(700)).await);
}

#[tokio::test]
async fn interrupts_foreground_input_even_when_the_physical_event_belongs_to_a_different_app() {
    let f = fixture(Driver::default());
    let hung = f.spawn(call("type_text").args(
        json!({ "pid": 700, "window_id": 900, "delivery_mode": "foreground", "text": "fixture" }),
    ));
    f.wait_for("dispatch").await;
    assert!(f.host.physical_input(keyboard(701)).await);
    let hung = hung.reply().await;
    assert!(!hung.ok);
    assert_eq!(hung.effect, Some(CuaEffect::DispatchedUnknown));
    f.wait_for("interrupt-ack").await;
    assert_eq!(f.count("release"), 1);
}

struct Monitored {
    state: MonitorState,
    activations: Arc<AtomicUsize>,
    armed: Arc<Mutex<Vec<bool>>>,
}

fn monitored(
    driver: Driver,
    configure: impl FnOnce(&mut CuaDriverHostOptions),
) -> (Fixture, Monitored) {
    let watch = Monitored {
        state: Arc::new(Mutex::new(ComputerInputMonitorState::failed(
            "input_monitor_idle",
        ))),
        activations: Arc::new(AtomicUsize::new(0)),
        armed: Arc::new(Mutex::new(Vec::new())),
    };
    let (activated, counter) = (watch.state.clone(), watch.activations.clone());
    let (disarmed, armed) = (watch.state.clone(), watch.armed.clone());
    let reader = monitor(&watch.state);
    let f = fixture_with(driver, |o| {
        o.activate_input_monitor = Some(Box::new(move || {
            counter.fetch_add(1, Ordering::SeqCst);
            *activated.lock().unwrap() = ComputerInputMonitorState::ready();
            Box::pin(async { Ok(()) })
        }));
        o.on_input_monitor_armed_change = Some(Box::new(move |on| {
            armed.lock().unwrap().push(on);
            if !on {
                *disarmed.lock().unwrap() = ComputerInputMonitorState::failed("input_monitor_idle");
            }
        }));
        o.input_monitor_state = Some(reader);
        configure(o);
    });
    (f, watch)
}

#[tokio::test]
async fn starts_listener_activation_only_on_use_and_disarms_it_when_the_last_task_ends() {
    let (f, watch) = monitored(Driver::default(), |_| {});
    assert_eq!(watch.activations.load(Ordering::SeqCst), 0);
    let owner = task("activation", Some("turn"));
    f.send(press().task(&owner)).await;
    assert_eq!(watch.activations.load(Ordering::SeqCst), 1);
    f.send(method(ComputerHostMethod::EndTask).task(&owner))
        .await;
    assert_eq!(watch.armed.lock().unwrap().last(), Some(&false));
    f.send(press().task(&task("activation", Some("next"))))
        .await;
    assert_eq!(watch.activations.load(Ordering::SeqCst), 2);
    assert_eq!(f.count("key"), 2);
}

#[tokio::test]
async fn keeps_passive_status_checks_and_previews_idle_until_actual_model_computer_work_starts() {
    let seam = fixed_permissions(Arc::new(Mutex::new(HostPermissions {
        accessibility: true,
        screen_recording: true,
        input_monitoring: Some(true),
    })));
    let (f, watch) = monitored(Driver::default(), |o| o.check_permissions = Some(seam));
    for _ in 0..3 {
        f.send(method(ComputerHostMethod::Probe)).await;
        let status = f.send(call("check_permissions")).await;
        let status = structured(&status);
        assert!(status.get("input_monitor_ready").is_none(), "{status}");
        assert!(status.get("input_monitor_error").is_none(), "{status}");
    }
    let owner = task("passive-preview", Some("turn"));
    let window = json!({ "pid": 700, "window_id": 900 });
    f.send(
        call("get_window_state")
            .args(window.clone())
            .task(&owner)
            .observe(false),
    )
    .await;
    assert_eq!(watch.activations.load(Ordering::SeqCst), 0);
    assert!(watch.armed.lock().unwrap().is_empty());
    assert!(!f.host.is_input_monitor_requested().await);
    f.send(
        call("get_window_state")
            .args(window)
            .task(&owner)
            .observe(true),
    )
    .await;
    assert_eq!(watch.activations.load(Ordering::SeqCst), 1);
    assert!(f.host.is_input_monitor_requested().await);
    f.send(method(ComputerHostMethod::EndTask).task(&owner))
        .await;
    f.send(call("check_permissions")).await;
    assert_eq!(watch.activations.load(Ordering::SeqCst), 1);
    assert!(!f.host.is_input_monitor_requested().await);
    assert_eq!(watch.armed.lock().unwrap().last(), Some(&false));
}

#[tokio::test]
async fn exposes_granted_but_unavailable_monitoring_and_requires_recovery_before_input() {
    let state: MonitorState = Arc::new(Mutex::new(ComputerInputMonitorState::failed(
        "event_tap_unavailable",
    )));
    let reader = monitor(&state);
    let seam = fixed_permissions(Arc::new(Mutex::new(HostPermissions {
        accessibility: true,
        screen_recording: true,
        input_monitoring: Some(true),
    })));
    let f = fixture_with(Driver::default(), |o| {
        o.input_monitor_state = Some(reader);
        o.check_permissions = Some(seam);
    });
    let refused = f.send(press()).await;
    assert_eq!(code(&refused).as_deref(), Some("input_monitor_unavailable"));
    assert_eq!(structured(&refused)["effect"], json!("refused"));
    let status = structured(&f.send(call("check_permissions")).await);
    assert_eq!(status["input_monitoring"], json!(true));
    assert_eq!(status["input_monitor_ready"], json!(false));
    assert_eq!(
        status["input_monitor_error"],
        json!("event_tap_unavailable")
    );
    *state.lock().unwrap() = ComputerInputMonitorState::ready();
    assert_clean_ok(&f.send(press()).await);
    let lost = ComputerInputMonitorState::failed("input_monitor_unavailable");
    *state.lock().unwrap() = lost.clone();
    f.host.input_monitor_state_changed(lost);
    f.wait_for("interrupt-ack").await;
    let unavailable = f.send(press()).await;
    assert_eq!(
        code(&unavailable).as_deref(),
        Some("input_monitor_unavailable")
    );
    assert_eq!(unavailable.desktop_interruptions, Some(0));
    *state.lock().unwrap() = ComputerInputMonitorState::ready();
    assert_paused(&f.send(press()).await);
    f.send(call("get_window_state").observe(true)).await;
    assert_clean_ok(&f.send(press()).await);
}

#[tokio::test]
async fn drains_an_interrupted_browser_mutation_and_uses_the_new_epoch_without_retiring_its_binding()
 {
    let f = fixture(Driver {
        browser_hang: true,
        ..Driver::default()
    });
    let owner = task("browser-stop", Some("turn"));
    let hung = f.spawn(
        call("browser_type")
            .args(json!({ "text": "fixture" }))
            .task(&owner),
    );
    f.wait_for("browser-dispatch").await;
    assert!(f.send(stop()).await.ok);
    assert_eq!(
        hung.reply().await.effect,
        Some(CuaEffect::DispatchedUnknown)
    );
    let navigated = f
        .send(call("browser_navigate").args(json!({})).task(&owner))
        .await;
    assert_clean_ok(&navigated);
    assert_eq!(result_value(&navigated), json!({}));
    assert_eq!(f.count("release"), 1);
    assert_eq!(f.count("start"), 1);
    assert!(!f.has("browser-effect"));
}

async fn drains_already_dispatched_input_when_its_caller_disconnects(name: &str) {
    let f = fixture(Driver {
        browser_hang: true,
        input_delay_ms: Some(150),
        ..Driver::default()
    });
    let pending = f.spawn(
        call(name)
            .args(json!({ "text": "fixture" }))
            .task(&task("disconnect", Some("turn"))),
    );
    f.wait_for(if name == "browser_type" {
        "browser-dispatch"
    } else {
        "dispatch"
    })
    .await;
    f.cancel(&pending.id);
    assert_eq!(
        pending.reply().await.effect,
        Some(CuaEffect::DispatchedUnknown)
    );
    f.wait_for("interrupt").await;
    assert_clean_ok(&f.send(press()).await);
    sleep_ms(180).await;
    let names = f.names();
    assert_eq!(f.count("release"), 1);
    assert_eq!(f.count("start"), 1);
    assert!(first_index(&names, "key") > first_index(&names, "interrupt-ack"));
    assert!(!f.has("effect") && !f.has("browser-effect"));
}

#[tokio::test]
async fn drains_already_dispatched_type_text_when_its_caller_disconnects() {
    drains_already_dispatched_input_when_its_caller_disconnects("type_text").await;
}

#[tokio::test]
async fn drains_already_dispatched_browser_type_when_its_caller_disconnects() {
    drains_already_dispatched_input_when_its_caller_disconnects("browser_type").await;
}

#[tokio::test]
async fn requires_a_live_escape_listener_for_browser_mutations_but_keeps_browser_reads_available() {
    let state: MonitorState = Arc::new(Mutex::new(ComputerInputMonitorState::failed(
        "event_tap_unavailable",
    )));
    let reader = monitor(&state);
    let f = fixture_with(Driver::default(), |o| o.input_monitor_state = Some(reader));
    let owner = task("listener-browser", Some("turn"));
    let action = || call("browser_navigate").args(json!({})).task(&owner);
    assert_eq!(
        code(&f.send(action()).await).as_deref(),
        Some("input_monitor_unavailable")
    );
    let read = f
        .send(call("get_browser_state").args(json!({})).task(&owner))
        .await;
    assert_clean_ok(&read);
    assert_eq!(result_value(&read), json!({}));
    *state.lock().unwrap() = ComputerInputMonitorState::ready();
    assert_eq!(result_value(&f.send(action()).await), json!({}));
    assert_eq!(
        f.names()
            .iter()
            .filter(|e| e.starts_with("browser:browser_navigate:"))
            .count(),
        1
    );
}

#[tokio::test]
async fn rechecks_browser_listener_readiness_after_asynchronous_session_setup() {
    let checks = Arc::new(AtomicUsize::new(0));
    let counter = checks.clone();
    let f = fixture_with(Driver::default(), |o| {
        o.input_monitor_state = Some(Box::new(move || {
            if counter.fetch_add(1, Ordering::SeqCst) == 0 {
                ComputerInputMonitorState::ready()
            } else {
                ComputerInputMonitorState::failed("event_tap_unavailable")
            }
        }))
    });
    let reply = f
        .send(
            call("browser_navigate")
                .args(json!({}))
                .task(&task("late-listener-failure", Some("turn"))),
        )
        .await;
    assert_eq!(code(&reply).as_deref(), Some("input_monitor_unavailable"));
    assert!(
        !f.names()
            .iter()
            .any(|e| e.starts_with("browser:browser_navigate:"))
    );
}

#[tokio::test]
async fn does_not_mistake_os_releases_or_later_reads_for_an_unconfirmed_browser_release() {
    let releases = Arc::new(AtomicUsize::new(0));
    let seam = counting_release(releases.clone());
    let f = fixture_with(
        Driver {
            browser_cleanup_unconfirmed: true,
            ..Driver::default()
        },
        |o| o.release_held_input = Some(seam),
    );
    let owner = task("browser-cleanup", Some("turn"));
    let typed = f
        .send(call("browser_type").args(json!({})).task(&owner))
        .await;
    assert!(result_is_error(&typed));
    assert_eq!(structured(&typed)["input_cleanup_unconfirmed"], json!(true));
    f.send(call("get_browser_state").args(json!({})).task(&owner))
        .await;
    assert!(!f.send(stop()).await.ok);
    assert_not_dispatched(&f.send(press()).await);
    assert_eq!(releases.load(Ordering::SeqCst), 0);
    assert!(!f.has("key"));
}

#[tokio::test]
async fn does_not_let_another_task_window_preview_or_mismatched_read_clear_native_takeover() {
    let f = fixture(Driver::default());
    let (task_a, task_b) = (task("task-a", Some("turn")), task("task-b", Some("turn")));
    let window_a = json!({ "pid": 701, "window_id": 901 });
    let window_b = json!({ "pid": 700, "window_id": 900 });
    let observe = |owner: &CuaComputerTask, args: Value, model: bool| {
        call("get_window_state")
            .args(args)
            .observe(model)
            .task(owner)
    };
    let click = |owner: &CuaComputerTask, args: &Value| {
        let mut args = args.clone();
        args["key"] = json!("enter");
        call("press_key").args(args).task(owner)
    };
    f.send(observe(&task_a, window_a.clone(), true)).await;
    f.send(observe(&task_b, window_b.clone(), true)).await;
    assert!(f.host.physical_input(pointer(700, 900)).await);
    f.wait_for("interrupt-ack").await;
    f.send(observe(&task_a, window_b.clone(), true)).await;
    f.send(observe(&task_b, window_a.clone(), true)).await;
    f.send(observe(&task_b, window_b.clone(), false)).await;
    let mut wrong = window_b.clone();
    wrong["fixture_wrong_window"] = json!(true);
    f.send(observe(&task_b, wrong, true)).await;
    f.send(call("get_desktop_state").observe(true).task(&task_b))
        .await;
    past_cooldown().await;
    assert_clean_ok(&f.send(click(&task_a, &window_a)).await);
    let blocked = f.send(click(&task_b, &window_b)).await;
    assert_paused(&blocked);
    assert_eq!(blocked.desktop_interruptions, Some(0));
    f.send(observe(&task_b, window_b.clone(), true)).await;
    assert_clean_ok(&f.send(click(&task_b, &window_b)).await);
}

#[tokio::test]
async fn a_takeover_refuses_only_the_touched_app_while_the_task_works_in_another() {
    let f = fixture(Driver::default());
    let owner = task("moved-on", Some("turn"));
    let touched = json!({ "pid": 700, "window_id": 900 });
    let sibling = json!({ "pid": 700, "window_id": 901 });
    let other = json!({ "pid": 701, "window_id": 950 });
    let observe = |args: &Value| {
        call("get_window_state")
            .args(args.clone())
            .observe(true)
            .task(&owner)
    };
    let press = |args: &Value| {
        let mut args = args.clone();
        args["key"] = json!("enter");
        call("press_key").args(args).task(&owner)
    };
    f.send(observe(&touched)).await;
    assert!(f.host.physical_input(pointer(700, 900)).await);
    f.wait_for("interrupt-ack").await;
    past_cooldown().await;
    f.send(observe(&other)).await;
    assert_clean_ok(&f.send(press(&other)).await);
    assert_paused(&f.send(press(&sibling)).await);
    assert_paused(&f.send(press(&touched)).await);
    f.send(observe(&touched)).await;
    assert_clean_ok(&f.send(press(&touched)).await);
}

#[tokio::test]
async fn fences_input_after_uncertain_focus_restoration_until_a_fresh_model_observation() {
    let restore_failure = json!({
        "isError": true,
        "structuredContent": { "effect": "unverifiable", "code": "focus_restore_failed" },
    });
    let f = fixture(Driver {
        action_result: Some(restore_failure.clone()),
        ..Driver::default()
    });
    let owner = task("restore-failure", Some("turn"));
    let target = json!({ "pid": 700, "window_id": 900 });
    let act = || {
        let mut args = target.clone();
        args["key"] = json!("enter");
        call("press_key").args(args).task(&owner)
    };
    let read = |model: bool| {
        call("get_window_state")
            .args(target.clone())
            .observe(model)
            .task(&owner)
    };
    assert_eq!(result_value(&f.send(act()).await), restore_failure);
    f.send(read(false)).await;
    assert_paused(&f.send(act()).await);
    assert_eq!(f.count("key"), 1);
    f.send(read(true)).await;
    assert_eq!(result_value(&f.send(act()).await), restore_failure);
    assert_eq!(f.count("key"), 2);
}

#[tokio::test]
async fn recovers_native_takeover_through_a_fresh_usable_sibling_never_an_empty_or_stale_read() {
    let f = fixture(Driver::default());
    let owner = task("sibling-recovery", Some("turn"));
    let observe = |args: Value| {
        call("get_window_state")
            .args(args)
            .observe(true)
            .task(&owner)
    };
    let act = || {
        call("press_key")
            .args(json!({ "pid": 700, "window_id": 901, "key": "enter" }))
            .task(&owner)
    };
    f.send(observe(json!({ "pid": 700, "window_id": 900 })))
        .await;
    f.host.physical_input(pointer(700, 900)).await;
    f.wait_for("interrupt-ack").await;
    let cooling = structured(&f.send(act()).await);
    assert!(cooling["wait_seconds"].as_f64().unwrap() > 0.0, "{cooling}");
    assert!(cooling["requery_hint"].is_string());
    past_cooldown().await;
    for extra in [
        json!({}),
        json!({ "fixture_usable": true, "fixture_degraded": "ax_window_unresolved" }),
        json!({ "fixture_usable": true, "fixture_stale": true }),
    ] {
        let mut args = json!({ "pid": 700, "window_id": 901 });
        args.as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        f.send(observe(args)).await;
        assert_eq!(
            code(&f.send(act()).await).as_deref(),
            Some("computer_input_paused"),
            "{extra}"
        );
    }
    f.send(observe(
        json!({ "pid": 700, "window_id": 901, "fixture_usable": true }),
    ))
    .await;
    assert_clean_ok(&f.send(act()).await);
}

async fn requires_the_affected_tasks_exact_browser_snapshot_after_takeover(snapshot_format: &str) {
    let f = fixture(Driver {
        browser_observations: true,
        ..Driver::default()
    });
    let owner = task("browser-takeover", Some("turn"));
    let other = task("other-browser-task", Some("turn"));
    let window = json!({ "pid": 700, "window_id": 900 });
    let browser = json!({ "target_id": "target-700-900", "tab_id": "tab-a", "snapshot_format": snapshot_format });
    let observe = |args: Value, model: bool, by: &CuaComputerTask| {
        call("get_browser_state").args(args).observe(model).task(by)
    };
    let action = || call("browser_click").args(browser.clone()).task(&owner);
    let with = |base: &Value, key: &str, value: Value| {
        let mut args = base.clone();
        args[key] = value;
        args
    };
    f.send(observe(window.clone(), true, &owner)).await;
    f.send(observe(browser.clone(), true, &owner)).await;
    assert!(f.host.physical_input(pointer(700, 900)).await);
    f.wait_for("interrupt-ack").await;
    f.send(observe(browser.clone(), true, &other)).await;
    f.send(observe(window.clone(), true, &owner)).await;
    f.send(observe(browser.clone(), false, &owner)).await;
    f.send(observe(
        with(&browser, "fixture_wrong_target", json!(true)),
        true,
        &owner,
    ))
    .await;
    f.send(observe(
        with(&browser, "tab_id", json!("other-tab")),
        true,
        &owner,
    ))
    .await;
    f.send(observe(
        with(&window, "fixture_target_id", json!("target-rebound")),
        true,
        &owner,
    ))
    .await;
    f.send(observe(
        json!({ "target_id": "target-rebound", "tab_id": "new-tab", "snapshot_format": snapshot_format }),
        true,
        &owner,
    ))
    .await;
    f.send(
        call("get_window_state")
            .args(window.clone())
            .observe(true)
            .task(&owner),
    )
    .await;
    past_cooldown().await;
    let blocked = f.send(action()).await;
    assert_paused(&blocked);
    assert_eq!(blocked.desktop_interruptions, Some(0));
    f.send(observe(browser.clone(), true, &owner)).await;
    assert_eq!(result_value(&f.send(action()).await), json!({}));
}

#[tokio::test]
async fn requires_the_affected_tasks_exact_dom_refs_v1_browser_snapshot_after_takeover() {
    requires_the_affected_tasks_exact_browser_snapshot_after_takeover("dom_refs_v1").await;
}

#[tokio::test]
async fn requires_the_affected_tasks_exact_semantic_v2_browser_snapshot_after_takeover() {
    requires_the_affected_tasks_exact_browser_snapshot_after_takeover("semantic_v2").await;
}

#[tokio::test]
async fn keeps_escape_browser_recovery_separate_from_native_reads_and_rejects_an_interrupted_browser_snapshot()
 {
    let f = fixture(Driver {
        browser_observations: true,
        delay_browser_observation: true,
        ..Driver::default()
    });
    let owner = task("browser-escape", Some("turn"));
    let window = json!({ "pid": 700, "window_id": 900 });
    let browser = json!({ "target_id": "target-700-900", "tab_id": "tab-a" });
    f.send(call("get_browser_state").args(window.clone()).task(&owner))
        .await;
    assert!(f.host.emergency_stop_input().await);
    f.wait_for("interrupt-ack").await;
    f.send(call("get_desktop_state").observe(true).task(&owner))
        .await;
    f.send(
        call("get_browser_state")
            .args(window)
            .observe(true)
            .task(&owner),
    )
    .await;
    let snapshot = || {
        call("get_browser_state")
            .args(browser.clone())
            .observe(true)
            .task(&owner)
    };
    let reading = f.spawn(snapshot());
    f.wait_for("browser-observe").await;
    assert!(f.host.physical_input(keyboard(700)).await);
    assert_paused(&reading.reply().await);
    past_cooldown().await;
    let navigate = || call("browser_navigate").args(browser.clone()).task(&owner);
    assert!(result_is_error(&f.send(navigate()).await));
    f.send(snapshot()).await;
    assert_eq!(result_value(&f.send(navigate()).await), json!({}));
}

#[tokio::test]
async fn permits_separate_isolated_setup_after_retirement_while_old_browser_targets_stay_paused() {
    let f = fixture(Driver {
        browser_observations: true,
        ..Driver::default()
    });
    let owner = task("browser-recovery-setup", Some("turn"));
    let old_target = json!({ "target_id": "old-browser", "tab_id": "old-tab" });
    let new_target = json!({ "target_id": "new-browser", "tab_id": "new-tab" });
    let run =
        |name: &str, args: Value, model: bool| call(name).args(args).observe(model).task(&owner);
    f.send(run("get_browser_state", old_target.clone(), true))
        .await;
    f.host.pause_desktop("screen-lock").await.unwrap();
    f.host.resume_desktop("screen-lock");
    assert_paused(
        &f.send(run(
            "browser_prepare",
            json!({ "pid": 700, "allow_launch": true, "profile": { "mode": "isolated_new" } }),
            false,
        ))
        .await,
    );
    let prepared = f
        .send(run(
            "browser_prepare",
            json!({ "allow_launch": true, "profile": { "mode": "isolated_new" } }),
            false,
        ))
        .await;
    assert_eq!(result_value(&prepared), json!({}));
    assert_paused(
        &f.send(run("browser_navigate", new_target.clone(), false))
            .await,
    );
    f.send(run(
        "get_browser_state",
        json!({ "pid": 701, "fixture_target_id": "new-browser" }),
        true,
    ))
    .await;
    assert_paused(
        &f.send(run("browser_navigate", new_target.clone(), false))
            .await,
    );
    f.send(run("get_browser_state", new_target.clone(), true))
        .await;
    assert_eq!(
        result_value(&f.send(run("browser_navigate", new_target, false)).await),
        json!({})
    );
    assert_paused(
        &f.send(run("browser_navigate", old_target.clone(), false))
            .await,
    );
    f.send(run("get_browser_state", old_target.clone(), true))
        .await;
    assert_eq!(
        result_value(&f.send(run("browser_navigate", old_target, false)).await),
        json!({})
    );
}

#[test]
fn has_no_rearm_method_left_on_the_host_protocol() {
    assert!(serde_json::from_value::<ComputerHostMethod>(json!("rearm")).is_err());
    assert!(serde_json::from_value::<ComputerHostMethod>(json!("stop")).is_ok());
}
