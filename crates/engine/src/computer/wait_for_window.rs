use std::future::Future;
use std::time::Duration;

use futures::future::BoxFuture;
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;
use zeron_proto::computer::{
    ComputerLaunchWindowReason, ComputerLaunchWindowStatus, ComputerWindow,
};

use super::desktop_operation_queue::{
    DesktopOperationError, OperationSignal, with_desktop_operation_signal,
};

const READINESS_BOUND_MS: u64 = 2_000;

#[derive(Debug, Clone, PartialEq)]
pub struct LaunchWindowReadiness {
    pub window: Option<ComputerWindow>,
    pub window_status: ComputerLaunchWindowStatus,
    pub window_reason: Option<ComputerLaunchWindowReason>,
}

pub type InputReadyCheck = dyn Fn(String) -> BoxFuture<'static, Result<(), String>> + Send + Sync;

fn unavailable(reason: ComputerLaunchWindowReason) -> LaunchWindowReadiness {
    LaunchWindowReadiness {
        window: None,
        window_status: ComputerLaunchWindowStatus::NoUsableWindow,
        window_reason: Some(reason),
    }
}

struct ProbeFailed;

impl From<DesktopOperationError> for ProbeFailed {
    fn from(_: DesktopOperationError) -> Self {
        ProbeFailed
    }
}

pub async fn wait_for_window<R, Fut, E>(
    read: R,
    app: &str,
    timeout_ms: u64,
    signal: Option<&OperationSignal>,
    pid: Option<u32>,
    check_input_ready: Option<&InputReadyCheck>,
) -> Result<LaunchWindowReadiness, DesktopOperationError>
where
    R: FnMut() -> Fut,
    Fut: Future<Output = Result<Vec<ComputerWindow>, E>>,
{
    let controller = CancellationToken::new();
    let _abort_probe = controller.clone().drop_guard();
    let probe_signal = match signal {
        Some(signal) => signal.any(&controller.clone().into()),
        None => controller.clone().into(),
    };
    let bound = if timeout_ms == 0 {
        READINESS_BOUND_MS
    } else {
        timeout_ms.clamp(1, READINESS_BOUND_MS)
    };
    let probe = with_desktop_operation_signal(
        probe_signal.clone(),
        probe_window(read, app, timeout_ms, &probe_signal, pid, check_input_ready),
    );
    let outcome = tokio::select! {
        outcome = probe => outcome,
        () = tokio::time::sleep(Duration::from_millis(bound)) => {
            controller.cancel();
            Err(ProbeFailed)
        }
    };
    match outcome {
        Ok(readiness) => Ok(readiness),
        Err(ProbeFailed) => {
            if let Some(signal) = signal {
                signal.check()?;
            }
            Ok(unavailable(ComputerLaunchWindowReason::InputUnavailable))
        }
    }
}

fn app_match_name(app: &str) -> String {
    let last = app.rsplit(['/', '\\']).next().unwrap_or_default();
    let stem = if last.len() >= 4
        && last.is_char_boundary(last.len() - 4)
        && last[last.len() - 4..].eq_ignore_ascii_case(".app")
    {
        &last[..last.len() - 4]
    } else {
        last
    };
    stem.to_lowercase()
}

async fn probe_window<R, Fut, E>(
    mut read: R,
    app: &str,
    timeout_ms: u64,
    signal: &OperationSignal,
    pid: Option<u32>,
    check_input_ready: Option<&InputReadyCheck>,
) -> Result<LaunchWindowReadiness, ProbeFailed>
where
    R: FnMut() -> Fut,
    Fut: Future<Output = Result<Vec<ComputerWindow>, E>>,
{
    let name = app_match_name(app);
    let deadline = Instant::now() + Duration::from_millis(timeout_ms.min(READINESS_BOUND_MS));
    loop {
        signal.check()?;
        let windows = read().await.map_err(|_| ProbeFailed)?;
        let mut matches: Vec<ComputerWindow> = windows
            .into_iter()
            .filter(|window| match pid {
                Some(pid) => window.pid == Some(pid),
                None => window
                    .app_name
                    .as_ref()
                    .is_some_and(|app_name| app_name.to_lowercase() == name),
            })
            .collect();
        signal.check()?;
        if matches.len() > 1 {
            return Ok(unavailable(ComputerLaunchWindowReason::Ambiguous));
        }
        let candidate = matches.pop();
        let reason = match &candidate {
            None => Some(ComputerLaunchWindowReason::NoWindow),
            Some(window) if window.on_current_space == Some(false) => {
                Some(ComputerLaunchWindowReason::OffSpace)
            }
            Some(window) if !window.visible || window.minimized => {
                Some(ComputerLaunchWindowReason::Hidden)
            }
            Some(_) => None,
        };
        if let (Some(window), None) = (candidate, reason) {
            if let Some(check) = check_input_ready
                && check(window.id.clone()).await.is_err()
            {
                signal.check()?;
                return Ok(unavailable(ComputerLaunchWindowReason::InputUnavailable));
            }
            signal.check()?;
            return Ok(LaunchWindowReadiness {
                window: Some(window),
                window_status: ComputerLaunchWindowStatus::Ready,
                window_reason: None,
            });
        }
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Ok(unavailable(
                reason.unwrap_or(ComputerLaunchWindowReason::InputUnavailable),
            ));
        }
        tokio::select! {
            () = tokio::time::sleep(remaining.min(Duration::from_millis(150))) => {}
            () = signal.aborted() => return Err(ProbeFailed),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use futures::FutureExt;
    use zeron_proto::computer::ComputerRect;

    use super::*;

    fn helium() -> ComputerWindow {
        ComputerWindow {
            id: "7".to_owned(),
            title: "Draft".to_owned(),
            app_name: Some("Helium".to_owned()),
            pid: None,
            bounds: None,
            focused: false,
            keyboard_focused: None,
            active: None,
            minimized: false,
            visible: true,
            space_ids: None,
            current_space_id: None,
            on_current_space: None,
            stacking_index: None,
            occluded_by: None,
        }
    }

    fn rect(width: f64, height: f64) -> Option<ComputerRect> {
        Some(ComputerRect {
            x: 0.0,
            y: 0.0,
            width,
            height,
        })
    }

    fn listing(
        windows: Vec<ComputerWindow>,
    ) -> impl FnMut() -> std::future::Ready<Result<Vec<ComputerWindow>, ()>> {
        move || std::future::ready(Ok(windows.clone()))
    }

    fn ready(window: ComputerWindow) -> LaunchWindowReadiness {
        LaunchWindowReadiness {
            window: Some(window),
            window_status: ComputerLaunchWindowStatus::Ready,
            window_reason: None,
        }
    }

    fn counting_check(calls: Arc<AtomicUsize>) -> Box<InputReadyCheck> {
        Box::new(move |_| {
            calls.fetch_add(1, Ordering::SeqCst);
            async { Ok(()) }.boxed()
        })
    }

    async fn wait(
        windows: Vec<ComputerWindow>,
        app: &str,
        timeout_ms: u64,
    ) -> LaunchWindowReadiness {
        wait_for_window(listing(windows), app, timeout_ms, None, None, None)
            .await
            .unwrap()
    }

    #[tokio::test(start_paused = true)]
    async fn bounds_a_hung_native_window_read_and_cancels_its_observation_context() {
        let started = Instant::now();
        let result = wait_for_window(
            futures::future::pending::<Result<Vec<ComputerWindow>, ()>>,
            "Helium",
            500,
            None,
            None,
            None,
        )
        .await
        .unwrap();
        assert_eq!(
            result,
            unavailable(ComputerLaunchWindowReason::InputUnavailable)
        );
        assert_eq!(started.elapsed(), Duration::from_millis(500));
    }

    #[tokio::test(start_paused = true)]
    async fn returns_an_existing_matching_app_window_immediately() {
        assert_eq!(
            wait(vec![helium()], "/Applications/Helium.app", 2_000).await,
            ready(helium())
        );
    }

    #[tokio::test(start_paused = true)]
    async fn observes_again_when_launch_has_not_produced_a_window_yet() {
        let reads = Arc::new(AtomicUsize::new(0));
        let counter = reads.clone();
        let result = wait_for_window(
            move || {
                let read = counter.fetch_add(1, Ordering::SeqCst) + 1;
                std::future::ready(Ok::<_, ()>(if read == 1 { vec![] } else { vec![helium()] }))
            },
            "Helium",
            500,
            None,
            None,
            None,
        )
        .await
        .unwrap();
        assert_eq!(result, ready(helium()));
        assert_eq!(reads.load(Ordering::SeqCst), 2);
    }

    #[tokio::test(start_paused = true)]
    async fn does_not_infer_a_primary_notes_window_from_accessory_sizes() {
        let notes = ComputerWindow {
            title: "Notes".to_owned(),
            app_name: Some("Notes".to_owned()),
            bounds: rect(1_000.0, 660.0),
            ..helium()
        };
        let calls = Arc::new(AtomicUsize::new(0));
        let check = counting_check(calls.clone());
        let windows = vec![
            ComputerWindow {
                id: "8".to_owned(),
                title: String::new(),
                visible: false,
                bounds: rect(500.0, 500.0),
                ..notes.clone()
            },
            ComputerWindow {
                id: "9".to_owned(),
                title: "Window".to_owned(),
                bounds: rect(66.0, 20.0),
                ..notes.clone()
            },
            notes,
        ];
        let result = wait_for_window(listing(windows), "Notes", 0, None, None, Some(&*check))
            .await
            .unwrap();
        assert_eq!(result, unavailable(ComputerLaunchWindowReason::Ambiguous));
        assert_eq!(calls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test(start_paused = true)]
    async fn does_not_bind_a_titled_inspector_over_a_document() {
        for document in [
            ComputerWindow {
                title: String::new(),
                bounds: rect(1_000.0, 660.0),
                ..helium()
            },
            ComputerWindow {
                title: "Document".to_owned(),
                bounds: rect(0.0, 0.0),
                ..helium()
            },
        ] {
            let calls = Arc::new(AtomicUsize::new(0));
            let check = counting_check(calls.clone());
            let inspector = ComputerWindow {
                id: "8".to_owned(),
                title: "Inspector".to_owned(),
                bounds: rect(240.0, 160.0),
                ..helium()
            };
            let windows = vec![document.clone(), inspector.clone()];
            for candidates in [windows.clone(), windows.into_iter().rev().collect()] {
                let result =
                    wait_for_window(listing(candidates), "Helium", 0, None, None, Some(&*check))
                        .await
                        .unwrap();
                assert_eq!(result, unavailable(ComputerLaunchWindowReason::Ambiguous));
            }
            assert_eq!(calls.load(Ordering::SeqCst), 0);
        }
    }

    #[tokio::test(start_paused = true)]
    async fn keeps_two_real_titled_windows_ambiguous_regardless_of_size_or_order() {
        let first = ComputerWindow {
            bounds: rect(120.0, 80.0),
            ..helium()
        };
        let second = ComputerWindow {
            id: "8".to_owned(),
            title: "Other".to_owned(),
            bounds: rect(1_000.0, 660.0),
            ..first.clone()
        };
        assert_eq!(
            wait(vec![first, second], "Helium", 0).await,
            unavailable(ComputerLaunchWindowReason::Ambiguous)
        );
    }

    #[tokio::test(start_paused = true)]
    async fn keeps_multiple_hidden_windows_ambiguous() {
        assert_eq!(
            wait(
                vec![
                    ComputerWindow {
                        visible: false,
                        ..helium()
                    },
                    ComputerWindow {
                        id: "8".to_owned(),
                        visible: false,
                        ..helium()
                    },
                ],
                "Helium",
                0
            )
            .await,
            unavailable(ComputerLaunchWindowReason::Ambiguous)
        );
    }

    #[tokio::test(start_paused = true)]
    async fn keeps_a_single_untitled_window_without_bounds_usable() {
        let untitled = ComputerWindow {
            title: String::new(),
            ..helium()
        };
        assert_eq!(
            wait(vec![untitled.clone()], "Helium", 0).await,
            ready(untitled)
        );
    }

    #[tokio::test(start_paused = true)]
    async fn does_not_choose_between_multiple_app_windows_or_unrelated_apps() {
        let result = wait(
            vec![
                helium(),
                ComputerWindow {
                    id: "8".to_owned(),
                    ..helium()
                },
            ],
            "Helium",
            0,
        )
        .await;
        assert_eq!(result.window, None);
        assert_eq!(
            result.window_status,
            ComputerLaunchWindowStatus::NoUsableWindow
        );
        let result = wait(vec![helium()], "Other", 0).await;
        assert_eq!(result.window, None);
        assert_eq!(
            result.window_status,
            ComputerLaunchWindowStatus::NoUsableWindow
        );
    }

    #[tokio::test(start_paused = true)]
    async fn stops_without_another_observation_when_cancelled() {
        let controller = CancellationToken::new();
        let signal: OperationSignal = controller.clone().into();
        let reads = Arc::new(AtomicUsize::new(0));
        let counter = reads.clone();
        let result = wait_for_window(
            move || {
                counter.fetch_add(1, Ordering::SeqCst);
                controller.cancel();
                std::future::ready(Ok::<_, ()>(vec![]))
            },
            "Helium",
            500,
            Some(&signal),
            None,
            None,
        )
        .await;
        assert_eq!(result, Err(DesktopOperationError::Aborted));
        assert_eq!(reads.load(Ordering::SeqCst), 1);
    }

    #[tokio::test(start_paused = true)]
    async fn uses_the_launched_pid_rather_than_a_same_name_process() {
        let launched = ComputerWindow {
            pid: Some(20),
            ..helium()
        };
        let other = ComputerWindow {
            pid: Some(10),
            ..helium()
        };
        let result = wait_for_window(
            listing(vec![other, launched.clone()]),
            "com.vendor.Helium",
            0,
            None,
            Some(20),
            None,
        )
        .await
        .unwrap();
        assert_eq!(result, ready(launched));
    }

    #[tokio::test(start_paused = true)]
    async fn does_not_bind_an_unusable_window() {
        for (window, reason) in [
            (
                ComputerWindow {
                    visible: false,
                    ..helium()
                },
                ComputerLaunchWindowReason::Hidden,
            ),
            (
                ComputerWindow {
                    minimized: true,
                    ..helium()
                },
                ComputerLaunchWindowReason::Hidden,
            ),
            (
                ComputerWindow {
                    on_current_space: Some(false),
                    ..helium()
                },
                ComputerLaunchWindowReason::OffSpace,
            ),
        ] {
            assert_eq!(wait(vec![window], "Helium", 0).await, unavailable(reason));
        }
    }

    #[tokio::test(start_paused = true)]
    async fn does_not_mistake_a_listed_window_for_native_input_readiness() {
        let check: Box<InputReadyCheck> =
            Box::new(|_| async { Err("ax_window_unresolved".to_owned()) }.boxed());
        let result = wait_for_window(
            listing(vec![helium()]),
            "Helium",
            0,
            None,
            None,
            Some(&*check),
        )
        .await
        .unwrap();
        assert_eq!(
            result,
            unavailable(ComputerLaunchWindowReason::InputUnavailable)
        );
    }

    #[tokio::test(start_paused = true)]
    async fn does_not_return_readiness_if_stop_arrives_during_the_probe() {
        let controller = CancellationToken::new();
        let signal: OperationSignal = controller.clone().into();
        let check: Box<InputReadyCheck> = Box::new(move |_| {
            controller.cancel();
            async { Ok(()) }.boxed()
        });
        let result = wait_for_window(
            listing(vec![helium()]),
            "Helium",
            0,
            Some(&signal),
            None,
            Some(&*check),
        )
        .await;
        assert_eq!(result, Err(DesktopOperationError::Aborted));
    }
}
