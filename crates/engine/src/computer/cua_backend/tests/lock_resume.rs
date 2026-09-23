use std::time::Duration;

use zeron_proto::computer::CuaEffect;

use super::*;
use crate::computer::backend::{ComputerBackendEvent, ComputerBackendSubscription};

fn lockable() -> Fixture {
    let f = fixture();
    f.set_bounds(json!({ "x": 0, "y": 0, "width": 200, "height": 100 }));
    f
}

fn interrupt(f: &Fixture, pauses: &[&str]) {
    let mut knobs = f.knobs();
    knobs.desktop_epoch += 1;
    knobs.interruptions = Some(knobs.interruptions.unwrap_or(0) + 1);
    knobs.pauses = Some(pauses.iter().map(|pause| (*pause).to_owned()).collect());
}

fn interruptions(f: &Fixture) -> (Arc<Mutex<Vec<Vec<String>>>>, ComputerBackendSubscription) {
    if f.knobs().interruptions.is_none() {
        let mut knobs = f.knobs();
        knobs.interruptions = Some(0);
        knobs.pauses = Some(Vec::new());
    }
    let seen: Arc<Mutex<Vec<Vec<String>>>> = Arc::default();
    let sink = seen.clone();
    let subscription = f.backend.on_event(Arc::new(move |event| {
        if let ComputerBackendEvent::DesktopInterrupted { pauses } = event {
            sink.lock().unwrap().push(pauses.clone());
        }
    }));
    (seen, subscription)
}

#[tokio::test]
async fn refuses_an_in_flight_reply_that_straddles_a_lock_without_replaying_it() {
    let f = lockable();
    f.backend
        .capture_screenshot(window_capture("cua:10:20"))
        .await
        .unwrap();
    let (open, gate) = gate();
    f.gate_clicks(Some(gate));
    let (in_flight, ()) = tokio::join!(
        f.backend.click(point(50.0, 50.0), Some("cua:10:20"), &[]),
        async {
            while f.named("click").is_empty() {
                tokio::time::sleep(Duration::from_millis(1)).await;
            }
            f.change_desktop();
            f.backend.check_input_ready("cua:10:20").await.unwrap();
            open.open();
        }
    );
    let error = in_flight.unwrap_err();
    assert_eq!(effect(&error), Some(CuaEffect::DispatchedUnknown));
    assert_eq!(code(&error), Some("stale_desktop_epoch"));
    assert_eq!(f.named("click").len(), 1);
    f.backend.dispose().await;
}

#[tokio::test]
async fn requires_a_fresh_observation_after_resume_and_never_auto_resumes() {
    let f = lockable();
    f.backend
        .capture_screenshot(window_capture("cua:10:20"))
        .await
        .unwrap();
    f.change_desktop();
    let error = f
        .backend
        .click(point(50.0, 50.0), Some("cua:10:20"), &[])
        .await
        .unwrap_err();
    assert_eq!(effect(&error), Some(CuaEffect::NotDispatched));
    assert_eq!(code(&error), Some("stale_geometry"));
    assert!(f.named("click").is_empty());
    let state = f
        .backend
        .get_state(ComputerStateOptions {
            window_id: Some("cua:10:20".into()),
            ..ComputerStateOptions::default()
        })
        .await
        .unwrap();
    assert_eq!(state.computer_id, "desktop");
    let error = f
        .backend
        .click(point(50.0, 50.0), Some("cua:10:20"), &[])
        .await
        .unwrap_err();
    assert_eq!(code(&error), Some("stale_geometry"));
    assert!(f.named("click").is_empty());
    f.backend
        .capture_screenshot(window_capture("cua:10:20"))
        .await
        .unwrap();
    f.backend
        .click(point(50.0, 50.0), Some("cua:10:20"), &[])
        .await
        .unwrap();
    assert_eq!(f.named("click").len(), 1);
    f.backend.dispose().await;
}

#[tokio::test]
async fn announces_an_interruption_cycle_once_even_when_it_ended_between_replies() {
    let f = lockable();
    let (seen, _subscription) = interruptions(&f);
    f.backend
        .capture_screenshot(window_capture("cua:10:20"))
        .await
        .unwrap();
    assert!(seen.lock().unwrap().is_empty());
    interrupt(&f, &[]);
    f.backend.check_input_ready("cua:10:20").await.unwrap();
    assert_eq!(*seen.lock().unwrap(), vec![Vec::<String>::new()]);
    f.backend.check_input_ready("cua:10:20").await.unwrap();
    assert_eq!(seen.lock().unwrap().len(), 1);
    f.backend.dispose().await;
}

#[tokio::test]
async fn reports_a_still_active_pause_on_the_refusal_reply_it_produces() {
    let f = lockable();
    let (seen, _subscription) = interruptions(&f);
    f.backend
        .capture_screenshot(window_capture("cua:10:20"))
        .await
        .unwrap();
    interrupt(&f, &["screen-lock"]);
    let error = f.backend.check_input_ready("cua:10:20").await.unwrap_err();
    assert_eq!(effect(&error), Some(CuaEffect::NotDispatched));
    assert_eq!(code(&error), Some("computer_input_paused"));
    assert_eq!(*seen.lock().unwrap(), vec![vec!["screen-lock".to_owned()]]);
    f.knobs().pauses = Some(Vec::new());
    f.backend.check_input_ready("cua:10:20").await.unwrap();
    assert_eq!(seen.lock().unwrap().len(), 1);
    f.backend.dispose().await;
}
