use super::*;
use crate::computer::host::ComputerFrameTapHost;
use zeron_proto::computer::CuaPreviewTarget;

fn owner() -> CuaComputerTask {
    task("thread", Some("turn"))
}

fn stop(task: &CuaComputerTask) -> ComputerHostCall {
    method(ComputerHostMethod::Stop).task(task)
}

fn scope(reply: &CuaReply) -> Option<String> {
    extra(reply, "stop_scope").and_then(|v| v.as_str().map(str::to_string))
}

fn index(names: &[String], name: &str) -> usize {
    names.iter().position(|e| e == name).unwrap()
}

pub(super) struct NoopTap;

#[async_trait::async_trait(?Send)]
impl ComputerFrameTapHost for NoopTap {
    fn update(&self, _target: CuaPreviewTarget) -> Result<(), String> {
        Ok(())
    }
    async fn end_task(&self, _task: CuaComputerTask) -> Result<(), String> {
        Ok(())
    }
    async fn stop(&self) -> Result<(), String> {
        Ok(())
    }
    async fn dispose(&self) -> Result<(), String> {
        Ok(())
    }
}

#[tokio::test]
async fn end_task_succeeds_without_a_native_preview() {
    let f = fixture(Driver::default());
    let reply = f
        .send(method(ComputerHostMethod::EndTask).task(&owner()))
        .await;
    assert!(reply.ok);
    assert!(f.events().is_empty());
}

#[tokio::test]
async fn user_stop_refuses_subsequent_calls_from_the_same_turn() {
    let f = fixture(Driver::default());
    f.host.stop_task_by_user(owner()).await.unwrap();
    let blocked = f
        .send(
            call("press_key")
                .task(&owner())
                .args(json!({ "key": "enter", "pid": 42, "window_id": 10 })),
        )
        .await;
    assert_not_dispatched(&blocked);
    assert!(blocked.error.as_deref().unwrap().contains("user stopped"));
    assert!(f.events().is_empty());
    let next = f
        .send(
            call("get_window_state")
                .task(&task("thread", Some("next")))
                .observe(true)
                .args(json!({ "pid": 42, "window_id": 10 })),
        )
        .await;
    assert!(next.ok);
}

async fn stopping_a_queued_sibling_preserves_its_native_input(active_task: CuaComputerTask) {
    let f = fixture(Driver {
        input_delay_ms: Some(300),
        log_sessions: true,
        ..Driver::default()
    });
    let active = f.spawn(
        call("type_text")
            .task(&active_task)
            .args(json!({ "text": "fixture" })),
    );
    f.wait_for("dispatch").await;
    let queued = f.spawn(press().task(&owner()));
    let stopped = f.send(stop(&owner())).await;
    assert!(stopped.ok);
    assert_eq!(scope(&stopped).as_deref(), Some("task"));
    assert_not_dispatched(&queued.reply().await);
    assert!(active.reply().await.ok);
    assert!(f.send(press().task(&active_task)).await.ok);
    assert_eq!(f.count("start"), 1);
    assert_eq!(f.count("key"), 1);
    assert!(f.has("effect"));
    assert!(!f.has("interrupt"));
    assert!(!f.has("retiring"));
}

#[tokio::test]
async fn stopping_another_threads_queued_sibling_preserves_its_native_input() {
    stopping_a_queued_sibling_preserves_its_native_input(task("other-thread", Some("turn"))).await;
}

#[tokio::test]
async fn stopping_a_later_turns_queued_sibling_preserves_its_native_input() {
    stopping_a_queued_sibling_preserves_its_native_input(task("thread", Some("new-turn"))).await;
}

#[tokio::test]
async fn drains_matching_native_input_and_reports_that_queued_siblings_share_the_generation_fence()
{
    let f = fixture(Driver::default());
    let active = f.spawn(
        call("type_text")
            .task(&owner())
            .args(json!({ "text": "fixture" })),
    );
    f.wait_for("dispatch").await;
    let sibling = task("other-thread", Some("turn"));
    let queued = f.spawn(press().task(&sibling));
    let stopped = f.send(stop(&owner())).await;
    assert!(stopped.ok);
    assert_eq!(scope(&stopped).as_deref(), Some("generation"));
    let active = active.reply().await;
    assert!(!active.ok);
    assert_eq!(active.effect, Some(CuaEffect::DispatchedUnknown));
    assert_not_dispatched(&queued.reply().await);
    assert!(f.send(press().task(&sibling)).await.ok);
    let names = f.names();
    assert!(index(&names, "interrupt-ack") < index(&names, "key"));
    assert_eq!(f.count("start"), 1);
    assert!(!f.has("effect") && !f.has("cancel") && !f.has("retiring"));
}

#[tokio::test]
async fn cancels_only_the_matching_observation_without_interrupting_queued_sibling_input() {
    let f = fixture(Driver {
        delay_observation: true,
        ..Driver::default()
    });
    let observation = f.spawn(
        call("get_window_state")
            .task(&owner())
            .observe(true)
            .args(json!({ "pid": 42, "window_id": 10 })),
    );
    f.wait_for("observe").await;
    let sibling = f.spawn(press().task(&task("other-thread", Some("turn"))));
    let stopped = f.send(stop(&owner())).await;
    assert_eq!(scope(&stopped).as_deref(), Some("task"));
    assert_not_dispatched(&observation.reply().await);
    assert!(sibling.reply().await.ok);
    assert!(!f.has("interrupt"));
}

#[tokio::test]
async fn releases_a_matching_permission_wait_without_cancelling_the_shared_check() {
    let entered = Deferred::new();
    let pending = Deferred::new();
    let seam = pending_permissions(entered.clone(), pending.clone());
    let f = fixture_with(Driver::default(), |o| o.check_permissions = Some(seam));
    let check = f.spawn(call("check_permissions").task(&owner()));
    entered.wait().await;
    f.host.stop_task_by_user(owner()).await.unwrap();
    assert_not_dispatched(&check.reply().await);
    assert!(f.send(press()).await.ok);
    pending.resolve(permissions(true));
    assert!(!f.has("interrupt"));
}

#[tokio::test]
async fn revokes_a_task_during_native_startup_without_retiring_the_siblings_shared_generation() {
    let f = fixture(Driver {
        metadata_delay_ms: 100,
        ..Driver::default()
    });
    let starting = f.spawn(press().task(&owner()));
    f.wait_for("start").await;
    let stopped = f.send(stop(&owner())).await;
    assert_eq!(scope(&stopped).as_deref(), Some("task"));
    assert_not_dispatched(&starting.reply().await);
    assert!(f.send(press()).await.ok);
    assert_eq!(f.count("key"), 1);
    assert_eq!(f.count("start"), 1);
    assert!(!f.has("interrupt") && !f.has("retiring"));
}

#[tokio::test]
async fn keeps_native_input_ownership_when_a_detached_launch_preview_read_finishes() {
    let f = fixture_with(
        Driver {
            delay_list_windows_ms: Some(100),
            ..Driver::default()
        },
        |o| o.frame_tap = Some(Arc::new(NoopTap)),
    );
    f.send(
        call("launch_app")
            .task(&task("launching-thread", Some("turn")))
            .args(json!({ "name": "Calculator" })),
    )
    .await;
    f.wait_for("list-windows").await;
    let active = f.spawn(
        call("type_text")
            .task(&owner())
            .args(json!({ "text": "fixture" })),
    );
    f.wait_for("dispatch").await;
    f.wait_for("list-windows-replied").await;
    let stopped = f.send(stop(&owner())).await;
    assert_eq!(scope(&stopped).as_deref(), Some("generation"));
    assert_eq!(
        active.reply().await.effect,
        Some(CuaEffect::DispatchedUnknown)
    );
    assert!(f.has("interrupt-ack"));
    assert!(!f.has("effect") && !f.has("retiring"));
}

#[tokio::test]
async fn drains_its_threads_active_native_input_when_stop_omits_a_turn_identity() {
    let f = fixture(Driver::default());
    let active = f.spawn(
        call("type_text")
            .task(&owner())
            .args(json!({ "text": "fixture" })),
    );
    f.wait_for("dispatch").await;
    let stopped = f.send(stop(&task("thread", None))).await;
    assert!(stopped.ok);
    assert_eq!(scope(&stopped).as_deref(), Some("generation"));
    assert_eq!(
        active.reply().await.effect,
        Some(CuaEffect::DispatchedUnknown)
    );
    assert!(f.has("interrupt-ack"));
    assert!(!f.has("effect") && !f.has("retiring"));
}

async fn thread_wide_stop_preserves_a_sibling(queue_own_work: bool) {
    let activations = Arc::new(AtomicUsize::new(0));
    let counter = activations.clone();
    let f = fixture_with(
        Driver {
            input_delay_ms: Some(500),
            ..Driver::default()
        },
        |o| {
            o.activate_input_monitor = Some(Box::new(move || {
                counter.fetch_add(1, Ordering::SeqCst);
                Box::pin(async { Ok(()) })
            }))
        },
    );
    f.send(
        call("get_window_state")
            .task(&owner())
            .args(json!({ "pid": 42, "window_id": 10 })),
    )
    .await;
    let sibling = f.spawn(
        call("type_text")
            .task(&task("other-thread", Some("turn")))
            .args(json!({ "text": "fixture" })),
    );
    f.wait_for("dispatch").await;
    let queued: Vec<Pending> = if queue_own_work {
        ["queued-turn", "other-queued-turn"]
            .into_iter()
            .map(|turn| f.spawn(press().task(&task("thread", Some(turn)))))
            .collect()
    } else {
        Vec::new()
    };
    if queue_own_work {
        for _ in 0..300 {
            if activations.load(Ordering::SeqCst) == 3 {
                break;
            }
            sleep_ms(10).await;
        }
        assert_eq!(activations.load(Ordering::SeqCst), 3);
    }
    let stopped = f.send(stop(&task("thread", None))).await;
    assert!(stopped.ok);
    assert_eq!(scope(&stopped).as_deref(), Some("task"));
    for call in queued {
        assert_not_dispatched(&call.reply().await);
    }
    assert!(sibling.reply().await.ok);
    assert_not_dispatched(&f.send(press().task(&owner())).await);
    assert!(
        f.send(press().task(&task("thread", Some("new-turn-after-stop"))))
            .await
            .ok
    );
    assert_eq!(f.count("key"), 1);
    assert!(f.has("effect"));
    assert!(!f.has("interrupt") && !f.has("retiring"));
}

#[tokio::test]
async fn thread_wide_stop_preserves_a_sibling_when_its_own_work_is_idle() {
    thread_wide_stop_preserves_a_sibling(false).await;
}

#[tokio::test]
async fn thread_wide_stop_preserves_a_sibling_when_its_own_work_is_queued() {
    thread_wide_stop_preserves_a_sibling(true).await;
}
