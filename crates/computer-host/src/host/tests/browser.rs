use super::*;
use std::collections::HashSet;

fn owner() -> CuaComputerTask {
    task("thread", Some("turn"))
}

#[tokio::test]
async fn attributes_browser_calls_to_a_per_thread_lifecycle_session_under_the_control_transport() {
    let f = fixture(Driver::default());
    let reply = f
        .send(
            call("browser_navigate")
                .task(&owner())
                .args(json!({ "url": "https://example.com" })),
        )
        .await;
    assert!(reply.ok, "{reply:?}");
    let names = f.names();
    assert!(
        names
            .iter()
            .any(|e| e.starts_with("session-begin:synara-transport-"))
    );
    assert!(names.iter().any(|e| {
        e.starts_with("browser:browser_navigate:synara-browser-thread:synara-transport-")
    }));
    let forged =
        f.send(call("browser_click").task(&owner()).args(
            json!({ "target_id": "t", "tab_id": "tab", "ref": "p1:0", "session": "forged" }),
        ))
        .await;
    assert!(forged.ok);
    let names = f.names();
    assert!(
        !names
            .iter()
            .any(|e| e.starts_with("browser:browser_click:forged"))
    );
    assert!(names.iter().any(|e| {
        e.starts_with("browser:browser_click:synara-browser-thread:synara-transport-")
    }));
}

#[tokio::test]
async fn refuses_browser_calls_without_task_attribution_before_starting_a_daemon() {
    let f = fixture(Driver::default());
    let reply = f
        .send(call("browser_navigate").args(json!({ "url": "https://example.com" })))
        .await;
    assert!(!reply.ok);
    assert!(reply.error.as_deref().unwrap().contains("task attribution"));
    assert!(f.events().is_empty());
}

#[tokio::test]
async fn ends_the_threads_browser_session_on_end_browser_thread_and_revives_it_on_the_next_call() {
    let f = fixture(Driver::default());
    let click = || {
        call("browser_click")
            .task(&owner())
            .args(json!({ "target_id": "t", "tab_id": "tab", "ref": "p1:0" }))
    };
    assert!(f.send(click()).await.ok);
    assert!(
        f.send(method(ComputerHostMethod::EndBrowserThread).task(&owner()))
            .await
            .ok
    );
    assert!(f.send(click()).await.ok);
    let lifecycle: Vec<String> = f
        .names()
        .into_iter()
        .filter(|e| {
            e.starts_with("browser:")
                || e.starts_with("start_session:")
                || e.starts_with("end_session:")
        })
        .collect();
    assert_eq!(lifecycle.len(), 4, "{lifecycle:?}");
    assert!(lifecycle[0].starts_with("browser:browser_click:synara-browser-thread:"));
    assert!(lifecycle[1].starts_with("end_session:synara-browser-thread:synara-transport-"));
    assert!(lifecycle[2].starts_with("start_session:synara-browser-thread:synara-transport-"));
    assert!(lifecycle[3].starts_with("browser:browser_click:synara-browser-thread:"));
}

#[tokio::test]
async fn keeps_end_browser_thread_a_no_op_for_a_thread_that_never_used_the_browser() {
    let f = fixture(Driver::default());
    assert!(
        f.send(method(ComputerHostMethod::EndBrowserThread).task(&owner()))
            .await
            .ok
    );
    assert!(f.events().is_empty());
}

#[tokio::test]
async fn refuses_a_browser_bind_that_names_one_of_this_apps_own_pids() {
    let f = fixture_with(Driver::default(), |o| {
        o.own_pids = Some(Box::new(|| HashSet::from([424242])))
    });
    let reply = f
        .send(
            call("get_browser_state")
                .task(&owner())
                .args(json!({ "pid": 424242, "window_id": 20 })),
        )
        .await;
    assert!(reply.ok);
    assert!(result_is_error(&reply));
    assert_eq!(structured(&reply)["effect"], json!("refused"));
    assert_eq!(code(&reply).as_deref(), Some("browser_self_target"));
    assert!(f.events().is_empty());
    let other = f
        .send(
            call("get_browser_state")
                .task(&owner())
                .args(json!({ "pid": 777, "window_id": 20 })),
        )
        .await;
    assert!(other.ok);
    assert!(
        f.names()
            .iter()
            .any(|e| e.starts_with("browser:get_browser_state:"))
    );
}
