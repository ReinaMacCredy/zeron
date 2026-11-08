use super::*;

fn warming(driver: Driver, configure: impl FnOnce(&mut CuaDriverHostOptions)) -> Fixture {
    fixture_with(driver, |o| {
        o.warm_on_first_touch = true;
        configure(o);
    })
}

#[tokio::test]
async fn leaves_the_driver_cold_when_warm_up_is_off() {
    let seam = fixed_permissions(Arc::new(Mutex::new(permissions(true))));
    let f = fixture_with(Driver::default(), |o| o.check_permissions = Some(seam));
    assert!(f.send(method(ComputerHostMethod::Probe)).await.ok);
    assert!(f.send(call("check_permissions")).await.ok);
    sleep_ms(100).await;
    assert!(f.events().is_empty());
}

#[tokio::test]
async fn warms_spawn_and_handshake_on_the_first_probe_without_opening_a_session() {
    let f = warming(Driver::default(), |_| {});
    assert!(f.send(method(ComputerHostMethod::Probe)).await.ok);
    f.wait_for("start").await;
    assert_eq!(f.count("start"), 1);
    sleep_ms(150).await;
    assert!(!f.has("motion-100-0"));
    assert!(f.send(press()).await.ok);
    assert_eq!(f.count("start"), 1);
    assert_eq!(f.count("motion-100-0"), 1);
    assert_eq!(f.count("key"), 1);
}

#[tokio::test]
async fn does_not_prewarm_a_macos_driver_before_the_first_real_permission_grants() {
    let granted = Arc::new(Mutex::new(permissions(false)));
    let seam = fixed_permissions(granted.clone());
    let f = warming(Driver::default(), |o| o.check_permissions = Some(seam));
    f.send(method(ComputerHostMethod::Probe)).await;
    f.send(call("check_permissions")).await;
    sleep_ms(100).await;
    assert!(f.events().is_empty());
    *granted.lock().unwrap() = permissions(true);
    f.send(call("check_permissions")).await;
    f.wait_for("start").await;
    assert_eq!(f.count("start"), 1);
}

#[tokio::test]
async fn warms_on_a_permission_check_too_and_only_once_per_host_lifetime() {
    let seam = fixed_permissions(Arc::new(Mutex::new(permissions(true))));
    let f = warming(Driver::default(), |o| o.check_permissions = Some(seam));
    f.send(call("check_permissions")).await;
    f.wait_for("start").await;
    f.send(method(ComputerHostMethod::Probe)).await;
    f.send(call("check_permissions")).await;
    sleep_ms(100).await;
    assert_eq!(f.count("start"), 1);
    f.host.stop().await.unwrap();
    f.send(method(ComputerHostMethod::Probe)).await;
    sleep_ms(100).await;
    assert_eq!(f.count("start"), 1);
    assert!(f.send(call("press_key")).await.ok);
    assert_eq!(f.count("start"), 2);
}

#[tokio::test]
async fn does_not_treat_housekeeping_requests_as_first_touches() {
    let f = warming(Driver::default(), |_| {});
    f.send(method(ComputerHostMethod::EndTask).task(&task("thread", Some("turn"))))
        .await;
    sleep_ms(100).await;
    assert!(f.events().is_empty());
}

#[tokio::test]
async fn logs_a_failed_warm_and_leaves_the_first_real_calls_own_startup_intact() {
    let (f, logs) = logged_fixture(
        Driver {
            unpatched: true,
            ..Driver::default()
        },
        |o| o.warm_on_first_touch = true,
    );
    assert!(f.send(method(ComputerHostMethod::Probe)).await.ok);
    let events = f.wait_for("start").await;
    let pid = events.iter().find(|e| e.event == "start").unwrap().pid;
    for _ in 0..300 {
        if !pid_alive(pid) {
            break;
        }
        sleep_ms(10).await;
    }
    assert!(!pid_alive(pid));
    logs.wait_for("driver warm-up failed").await;
    assert_not_dispatched(&f.send(press()).await);
    assert_eq!(f.count("start"), 2);
}
