use std::future::Future;

tokio::task_local! {
    static OBSERVATION: ();
}

pub fn is_model_desktop_observation_active() -> bool {
    OBSERVATION.try_with(|_| ()).is_ok()
}

pub async fn with_model_desktop_observation<F: Future>(observe: F) -> F::Output {
    OBSERVATION.scope((), observe).await
}

#[cfg(test)]
mod tests {
    use tokio::sync::oneshot;

    use super::*;

    fn detached_probe() -> (oneshot::Sender<()>, tokio::task::JoinHandle<bool>) {
        let (release, gate) = oneshot::channel::<()>();
        let probe = tokio::spawn(async move {
            let _ = gate.await;
            is_model_desktop_observation_active()
        });
        (release, probe)
    }

    #[tokio::test]
    async fn isolates_concurrent_scopes_and_leaves_outside_work_inactive() {
        assert!(!is_model_desktop_observation_active());
        let (first_release, first_gate) = oneshot::channel::<()>();
        let (second_release, second_gate) = oneshot::channel::<()>();
        let first = tokio::spawn(with_model_desktop_observation(async move {
            assert!(is_model_desktop_observation_active());
            let _ = first_gate.await;
            assert!(is_model_desktop_observation_active());
            "first observation"
        }));
        let second = tokio::spawn(with_model_desktop_observation(async move {
            assert!(is_model_desktop_observation_active());
            let _ = second_gate.await;
            assert!(is_model_desktop_observation_active());
        }));
        tokio::task::yield_now().await;
        assert!(!is_model_desktop_observation_active());
        first_release.send(()).unwrap();
        assert_eq!(first.await.unwrap(), "first observation");
        assert!(!is_model_desktop_observation_active());
        second_release.send(()).unwrap();
        second.await.unwrap();
        assert!(!is_model_desktop_observation_active());
    }

    #[tokio::test]
    async fn expires_authority_in_a_detached_continuation_after_completion() {
        let (release, delayed) = with_model_desktop_observation(async {
            assert!(is_model_desktop_observation_active());
            detached_probe()
        })
        .await;
        release.send(()).unwrap();
        assert!(!delayed.await.unwrap());
    }

    #[tokio::test]
    async fn expires_detached_authority_after_a_failure() {
        let (result, (release, delayed)): (Result<(), &str>, _) =
            with_model_desktop_observation(async {
                let probe = detached_probe();
                assert!(is_model_desktop_observation_active());
                (Err("observation failed"), probe)
            })
            .await;
        assert_eq!(result, Err("observation failed"));
        assert!(!is_model_desktop_observation_active());
        release.send(()).unwrap();
        assert!(!delayed.await.unwrap());
    }

    #[tokio::test]
    async fn restores_the_outer_scope_while_expiring_a_completed_nested_scope() {
        with_model_desktop_observation(async {
            let (release, delayed) = with_model_desktop_observation(async {
                assert!(is_model_desktop_observation_active());
                detached_probe()
            })
            .await;
            assert!(is_model_desktop_observation_active());
            release.send(()).unwrap();
            assert!(!delayed.await.unwrap());
            assert!(is_model_desktop_observation_active());
        })
        .await;
        assert!(!is_model_desktop_observation_active());
    }
}
