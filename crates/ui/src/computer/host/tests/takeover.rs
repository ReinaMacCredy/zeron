use std::time::Instant;

use super::*;
use crate::computer::host::{PhysicalComputerInput, PhysicalInputKind, TakeoverEvent};

const QUIET: Duration = Duration::from_millis(2_000);
const LATE: Duration = Duration::from_millis(60);

type Events = Arc<Mutex<Vec<TakeoverEvent>>>;

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

fn quiet_fixture() -> (Fixture, Events) {
    let events = Events::default();
    let sink = events.clone();
    let f = fixture_with(Driver::default(), |o| {
        o.input_quiet = QUIET;
        o.on_takeover = Some(Box::new(move |event| sink.lock().unwrap().push(event)));
    });
    (f, events)
}

fn act(owner: &CuaComputerTask) -> ComputerHostCall {
    call("press_key")
        .args(json!({ "pid": 700, "window_id": 900, "key": "enter" }))
        .task(owner)
}

fn read(owner: &CuaComputerTask) -> ComputerHostCall {
    call("get_window_state")
        .args(json!({ "pid": 700, "window_id": 900 }))
        .observe(true)
        .task(owner)
}

async fn take_over(f: &Fixture, owner: &CuaComputerTask) {
    assert_clean_ok(&f.send(act(owner)).await);
    assert!(f.host.physical_input(pointer(700, 900)).await);
    f.wait_for("interrupt-ack").await;
}

fn paused(thread: &str) -> TakeoverEvent {
    TakeoverEvent::Paused {
        thread_id: thread.into(),
        pid: 700,
        window_id: Some(900),
    }
}

fn resumed(thread: &str) -> TakeoverEvent {
    TakeoverEvent::Resumed {
        thread_id: thread.into(),
    }
}

#[tokio::test]
async fn a_takeover_holds_the_next_action_until_quiet_then_refuses_with_requery() {
    let (f, events) = quiet_fixture();
    let owner = task("takeover", Some("turn"));
    take_over(&f, &owner).await;
    let held = f.spawn(act(&owner));
    sleep_ms(700).await;
    assert!(f.host.physical_input(keyboard(700)).await);
    let last_input = Instant::now();
    sleep_ms(1_000).await;
    assert!(
        !held.handle.is_finished(),
        "the action waits while the user keeps typing"
    );
    let reply = held.reply().await;
    assert!(
        last_input.elapsed() + LATE >= QUIET,
        "released {:?} after the last input",
        last_input.elapsed()
    );
    assert_paused(&reply);
    let refusal = structured(&reply);
    assert_eq!(refusal["cause"], json!("user_input"), "{refusal}");
    assert!(refusal.get("wait_seconds").is_none(), "{refusal}");
    let message = refusal["message"].as_str().unwrap();
    assert!(message.contains("has stopped"), "{message}");
    assert!(message.contains("Do not ask the user"), "{message}");
    assert_eq!(
        f.count("key"),
        1,
        "the held action never reached the driver"
    );
    assert_eq!(*events.lock().unwrap(), vec![paused("takeover")]);

    assert_clean_ok(&f.send(read(&owner)).await);
    assert_clean_ok(&f.send(act(&owner)).await);
    assert_eq!(f.count("key"), 2);
    assert_eq!(
        *events.lock().unwrap(),
        vec![paused("takeover"), resumed("takeover")],
        "one pause per episode however many keys the user pressed"
    );
}

#[tokio::test]
async fn a_read_during_typing_waits_and_then_clears_the_gate() {
    let (f, events) = quiet_fixture();
    let owner = task("reader", Some("turn"));
    take_over(&f, &owner).await;
    let reading = f.spawn(read(&owner));
    sleep_ms(500).await;
    assert!(f.host.physical_input(keyboard(700)).await);
    let last_input = Instant::now();
    sleep_ms(1_000).await;
    assert!(!f.has("observe"), "the read waits for the user to stop");
    let observed = reading.reply().await;
    assert!(
        last_input.elapsed() + LATE >= QUIET,
        "released {:?} after the last input",
        last_input.elapsed()
    );
    assert_clean_ok(&observed);
    assert_eq!(f.count("observe"), 1);
    assert_clean_ok(&f.send(act(&owner)).await);
    assert_eq!(f.count("key"), 2, "the read resumed input with no refusal");
    assert_eq!(
        *events.lock().unwrap(),
        vec![paused("reader"), resumed("reader")]
    );
}

#[tokio::test]
async fn a_stop_during_the_quiet_wait_returns_task_stopped() {
    let (f, _) = quiet_fixture();
    let owner = task("stopper", Some("turn"));
    take_over(&f, &owner).await;
    let held = f.spawn(act(&owner));
    sleep_ms(300).await;
    let stopped_at = Instant::now();
    assert!(
        f.send(method(ComputerHostMethod::Stop).task(&owner))
            .await
            .ok
    );
    let reply = held.reply().await;
    assert!(
        stopped_at.elapsed() < Duration::from_millis(800),
        "the stop ended the wait after {:?}",
        stopped_at.elapsed()
    );
    assert_not_dispatched(&reply);
    assert!(
        reply
            .error
            .as_deref()
            .is_some_and(|error| error.contains("stopped computer use")),
        "{reply:?}"
    );
    assert_eq!(f.count("key"), 1);
}
