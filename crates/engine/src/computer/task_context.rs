use std::future::Future;

use zeron_proto::computer::CuaComputerTask;

tokio::task_local! {
    static COMPUTER_TASK: CuaComputerTask;
}

pub fn current_computer_task() -> Option<CuaComputerTask> {
    COMPUTER_TASK.try_with(Clone::clone).ok()
}

pub async fn with_computer_task<F: Future>(task: CuaComputerTask, run: F) -> F::Output {
    COMPUTER_TASK.scope(task, run).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn is_absent_in_ordinary_work_and_expires_in_detached_continuations() {
        assert_eq!(current_computer_task(), None);
        let task = CuaComputerTask {
            thread_id: "thread".to_owned(),
            turn_id: Some("turn".to_owned()),
            label: None,
        };
        let (wake, gate) = tokio::sync::oneshot::channel::<()>();
        let (inside, detached) = with_computer_task(task.clone(), async {
            let detached = tokio::spawn(async move {
                let _ = gate.await;
                current_computer_task()
            });
            (current_computer_task(), detached)
        })
        .await;
        assert_eq!(inside, Some(task));
        wake.send(()).unwrap();
        assert_eq!(current_computer_task(), None);
        assert_eq!(detached.await.unwrap(), None);
    }
}
