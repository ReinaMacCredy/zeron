use super::*;
use std::collections::HashSet;
use std::sync::atomic::AtomicBool;

const OWN_PID: u32 = 424242;

fn guarded(showing: bool) -> (Fixture, Arc<AtomicBool>) {
    let flag = Arc::new(AtomicBool::new(showing));
    let f = fixture_with(Driver::default(), {
        let flag = flag.clone();
        move |o| {
            o.own_pids = Some(Box::new(|| HashSet::from([OWN_PID])));
            o.protected_surface = Some(Box::new(move || flag.load(Ordering::SeqCst)));
        }
    });
    (f, flag)
}

#[track_caller]
fn assert_self_protected(reply: &CuaReply) {
    assert!(reply.ok, "{reply:?}");
    assert!(result_is_error(reply), "{reply:?}");
    assert_eq!(structured(reply)["effect"], json!("refused"), "{reply:?}");
    assert_eq!(
        code(reply).as_deref(),
        Some("self_protected_surface"),
        "{reply:?}"
    );
}

#[tokio::test]
async fn refuses_a_click_on_this_apps_window_while_an_approval_is_showing() {
    let (f, _) = guarded(true);
    let reply = f
        .send(call("click").args(json!({ "pid": OWN_PID, "window_id": 20, "x": 5, "y": 5 })))
        .await;
    assert_self_protected(&reply);
    assert!(f.events().is_empty(), "{:?}", f.names());
}

#[tokio::test]
async fn refuses_keys_text_and_values_aimed_at_this_app_while_an_approval_is_showing() {
    let (f, _) = guarded(true);
    for (name, args) in [
        (
            "press_key",
            json!({ "pid": OWN_PID, "window_id": 20, "key": "return" }),
        ),
        (
            "type_text",
            json!({ "pid": OWN_PID, "window_id": 20, "text": "y" }),
        ),
        (
            "set_value",
            json!({ "pid": OWN_PID, "window_id": 20, "element_index": 3, "value": "Full" }),
        ),
        (
            "click",
            json!({ "pid": OWN_PID, "window_id": 20, "element_token": "t", "action": "AXPress" }),
        ),
    ] {
        let reply = f.send(call(name).args(args)).await;
        assert_self_protected(&reply);
    }
    assert!(f.events().is_empty(), "{:?}", f.names());
}

#[tokio::test]
async fn dispatches_input_to_this_app_when_no_protected_surface_is_showing() {
    let (f, flag) = guarded(false);
    let key = || call("press_key").args(json!({ "pid": OWN_PID, "window_id": 20, "key": "a" }));
    assert_clean_ok(&f.send(key()).await);
    assert_eq!(f.count("key"), 1);
    flag.store(true, Ordering::SeqCst);
    assert_self_protected(&f.send(key()).await);
    assert_eq!(f.count("key"), 1);
    flag.store(false, Ordering::SeqCst);
    assert_clean_ok(&f.send(key()).await);
    assert_eq!(f.count("key"), 2);
}

#[tokio::test]
async fn leaves_other_apps_and_reads_of_this_app_alone_while_an_approval_is_showing() {
    let (f, _) = guarded(true);
    assert_clean_ok(
        &f.send(call("press_key").args(json!({ "pid": 777, "window_id": 20, "key": "a" })))
            .await,
    );
    assert_eq!(f.count("key"), 1);
    let read = f
        .send(call("get_window_state").args(json!({ "pid": OWN_PID, "window_id": 20 })))
        .await;
    assert!(read.ok, "{read:?}");
    assert_ne!(code(&read).as_deref(), Some("self_protected_surface"));
    assert_eq!(f.count("observe"), 1);
}
