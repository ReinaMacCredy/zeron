use zeron_proto::computer::CuaComputerTask;

use super::*;
use crate::computer::task_context::with_computer_task;

fn task(thread_id: &str, turn_id: &str) -> CuaComputerTask {
    CuaComputerTask {
        thread_id: thread_id.to_owned(),
        turn_id: Some(turn_id.to_owned()),
        label: None,
    }
}

fn end_task_calls(f: &Fixture) -> usize {
    f.calls()
        .iter()
        .filter(|call| call.method == ComputerHostMethod::EndTask)
        .count()
}

#[tokio::test]
async fn a_dispatching_task_survives_flooding_while_idle_tasks_are_evicted() {
    let f = fixture();
    f.set_bounds(json!({ "x": 0, "y": 0, "width": 200, "height": 100 }));
    let observe = |thread_id: String, turn_id: String| {
        with_computer_task(
            task(&thread_id, &turn_id),
            f.backend.get_state(window_state("cua:10:20")),
        )
    };
    observe("evict-live".into(), "turn-live".into())
        .await
        .unwrap();
    for i in 0..600 {
        observe(format!("flood-{i}"), format!("turn-{i}"))
            .await
            .unwrap();
        if i % 50 == 0 {
            observe("evict-live".into(), "turn-live".into())
                .await
                .unwrap();
        }
    }
    f.backend
        .end_task("evict-live", Some("turn-live"))
        .await
        .unwrap();
    assert_eq!(end_task_calls(&f), 1);
    f.backend.end_task("flood-0", Some("turn-0")).await.unwrap();
    assert_eq!(end_task_calls(&f), 1);
    f.backend.dispose().await;
}
