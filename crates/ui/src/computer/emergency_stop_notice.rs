use std::future::Future;
use std::time::Duration;

pub const EMERGENCY_STOP_NOTICE_TIMEOUT: Duration = Duration::from_secs(5);
pub const EMERGENCY_STOP_RETRY_DELAYS: [Duration; 2] =
    [Duration::from_secs(1), Duration::from_secs(3)];

pub async fn notify_engine_emergency_stop<F, Fut>(send: F, on_error: impl Fn(String))
where
    F: Fn() -> Fut,
    Fut: Future<Output = Result<(), String>>,
{
    notify_with_delays(send, on_error, &EMERGENCY_STOP_RETRY_DELAYS).await;
}

async fn notify_with_delays<F, Fut>(send: F, on_error: impl Fn(String), delays: &[Duration])
where
    F: Fn() -> Fut,
    Fut: Future<Output = Result<(), String>>,
{
    for attempt in 0..=delays.len() {
        match tokio::time::timeout(EMERGENCY_STOP_NOTICE_TIMEOUT, send()).await {
            Ok(Ok(())) => return,
            Ok(Err(error)) => on_error(format!("computer emergency-stop notice failed: {error}")),
            Err(_) => on_error("computer emergency-stop notice timed out".into()),
        }
        if let Some(delay) = delays.get(attempt) {
            tokio::time::sleep(*delay).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicUsize, Ordering};

    const SHORT: [Duration; 2] = [Duration::from_millis(1), Duration::from_millis(1)];

    async fn run(outcomes: Vec<Result<(), String>>) -> (usize, Vec<String>) {
        let attempts = AtomicUsize::new(0);
        let errors = Mutex::new(Vec::new());
        notify_with_delays(
            || {
                let attempt = attempts.fetch_add(1, Ordering::SeqCst);
                let outcome = outcomes[attempt].clone();
                async move { outcome }
            },
            |error| errors.lock().unwrap().push(error),
            &SHORT,
        )
        .await;
        (
            attempts.load(Ordering::SeqCst),
            errors.into_inner().unwrap(),
        )
    }

    #[tokio::test]
    async fn stops_at_the_first_accepted_notice() {
        assert_eq!(run(vec![Ok(())]).await, (1, vec![]));
    }

    #[tokio::test]
    async fn retries_a_refused_notice_and_reports_each_failure() {
        let (attempts, errors) = run(vec![Err("HTTP 503".into()), Ok(())]).await;
        assert_eq!(attempts, 2);
        assert_eq!(
            errors,
            vec!["computer emergency-stop notice failed: HTTP 503".to_string()]
        );
    }

    #[tokio::test]
    async fn gives_up_after_the_retry_schedule() {
        let (attempts, errors) = run(vec![
            Err("down".into()),
            Err("down".into()),
            Err("down".into()),
        ])
        .await;
        assert_eq!(attempts, 3);
        assert_eq!(errors.len(), 3);
        assert_eq!(
            EMERGENCY_STOP_RETRY_DELAYS,
            [Duration::from_secs(1), Duration::from_secs(3)]
        );
    }
}
