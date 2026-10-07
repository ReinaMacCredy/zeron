use std::future::Future;
use std::time::Duration;

use tokio::time::Instant;
use zeron_proto::computer::{
    ComputerAccessibilityStatus, ComputerState, ComputerTarget, ComputerUiNode,
};

use super::desktop_operation_queue::{DesktopOperationError, OperationSignal};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ComputerControlReadinessStatus {
    Ready,
    Timeout,
    Unavailable,
    Ambiguous,
    Closed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ComputerControlReadiness {
    pub status: ComputerControlReadinessStatus,
    pub waited_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ComputerTargetLookup<E> {
    NotFound,
    Ambiguous,
    Failed(E),
}

async fn delay(
    duration: Duration,
    signal: Option<&OperationSignal>,
) -> Result<(), DesktopOperationError> {
    match signal {
        Some(signal) => tokio::select! {
            () = tokio::time::sleep(duration) => Ok(()),
            () = signal.aborted() => Err(DesktopOperationError::Aborted),
        },
        None => {
            tokio::time::sleep(duration).await;
            Ok(())
        }
    }
}

fn check(signal: Option<&OperationSignal>) -> Result<(), DesktopOperationError> {
    signal.map_or(Ok(()), OperationSignal::check)
}

pub async fn wait_for_control<E, R, Fut, V>(
    mut read: R,
    resolve: V,
    target: &ComputerTarget,
    timeout_ms: u64,
    signal: Option<&OperationSignal>,
    absent: bool,
) -> Result<ComputerControlReadiness, E>
where
    E: From<DesktopOperationError>,
    R: FnMut() -> Fut,
    Fut: Future<Output = Result<ComputerState, E>>,
    V: Fn(&ComputerUiNode, &ComputerTarget) -> Result<(), ComputerTargetLookup<E>>,
{
    use ComputerControlReadinessStatus as Status;
    let started = Instant::now();
    let result = |status| ComputerControlReadiness {
        status,
        waited_ms: started.elapsed().as_millis() as u64,
    };
    loop {
        check(signal)?;
        let state = read().await?;
        check(signal)?;
        if let Some(window_id) = &target.window_id
            && !state.windows.iter().any(|window| &window.id == window_id)
        {
            return Ok(result(if absent {
                Status::Ready
            } else {
                Status::Closed
            }));
        }
        let accessibility_unavailable = state.accessibility.as_ref().is_some_and(|report| {
            report.status == ComputerAccessibilityStatus::Unavailable
                || target
                    .window_id
                    .as_ref()
                    .is_some_and(|window_id| report.unavailable_window_ids.contains(window_id))
        });
        let Some(root) = state.root.as_ref().filter(|_| !accessibility_unavailable) else {
            return Ok(result(Status::Unavailable));
        };
        let incomplete = root.truncated == Some(true);
        match resolve(root, target) {
            Ok(()) if !absent => return Ok(result(Status::Ready)),
            Ok(()) => {}
            Err(ComputerTargetLookup::Ambiguous) if !absent => {
                return Ok(result(Status::Ambiguous));
            }
            Err(ComputerTargetLookup::Ambiguous) => {}
            Err(ComputerTargetLookup::NotFound) if absent && !incomplete => {
                return Ok(result(Status::Ready));
            }
            Err(ComputerTargetLookup::NotFound) => {}
            Err(ComputerTargetLookup::Failed(error)) => return Err(error),
        }
        let elapsed = started.elapsed().as_millis() as u64;
        let Some(remaining) = timeout_ms.checked_sub(elapsed).filter(|left| *left > 0) else {
            // A truncated tree cannot prove the control is absent.
            return Ok(result(if incomplete {
                Status::Unavailable
            } else {
                Status::Timeout
            }));
        };
        delay(Duration::from_millis(remaining.min(100)), signal).await?;
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use tokio_util::sync::CancellationToken;
    use zeron_proto::computer::{
        ComputerAccessibilityReport, ComputerRect, ComputerScreenSize, ComputerWindow,
    };

    use super::*;

    type Error = DesktopOperationError;

    fn node(role: &str, label: Option<&str>, children: Vec<ComputerUiNode>) -> ComputerUiNode {
        ComputerUiNode {
            role: role.to_owned(),
            label: label.map(str::to_owned),
            value: None,
            description: None,
            frame: ComputerRect {
                x: 0.0,
                y: 0.0,
                width: 10.0,
                height: 10.0,
            },
            activation_point: None,
            on_screen: true,
            window_id: Some("fake-calculator".to_owned()),
            accessibility_root: None,
            node_path: None,
            editable: None,
            truncated: None,
            children,
        }
    }

    fn calculator_state() -> ComputerState {
        ComputerState {
            input_pause: None,
            preview_note: None,
            accessibility: None,
            computer_id: "desktop".to_owned(),
            windows: vec![ComputerWindow {
                id: "fake-calculator".to_owned(),
                title: "Calculator".to_owned(),
                app_name: Some("org.kde.kcalc".to_owned()),
                pid: Some(1_002),
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
            }],
            screen_size: ComputerScreenSize {
                width: 1_920,
                height: 1_080,
                scale: None,
            },
            availability: None,
            root: Some(node(
                "desktop",
                None,
                vec![node(
                    "window",
                    Some("Calculator"),
                    vec![
                        node("button", Some("Calculate"), vec![]),
                        node("text", Some("Display"), vec![]),
                    ],
                )],
            )),
            text: None,
            screenshot: None,
            captured_at: "2026-09-23T00:00:00.000Z".to_owned(),
            ax_tree: None,
        }
    }

    fn count(root: &ComputerUiNode, label: &str) -> usize {
        usize::from(root.label.as_deref() == Some(label))
            + root
                .children
                .iter()
                .map(|child| count(child, label))
                .sum::<usize>()
    }

    fn resolve(
        root: &ComputerUiNode,
        target: &ComputerTarget,
    ) -> Result<(), ComputerTargetLookup<Error>> {
        match count(root, target.label.as_deref().unwrap_or_default()) {
            0 => Err(ComputerTargetLookup::NotFound),
            1 => Ok(()),
            _ => Err(ComputerTargetLookup::Ambiguous),
        }
    }

    fn target(label: &str) -> ComputerTarget {
        ComputerTarget {
            window_id: Some("fake-calculator".to_owned()),
            label: Some(label.to_owned()),
            ..ComputerTarget::default()
        }
    }

    fn without_controls(mut state: ComputerState) -> ComputerState {
        state.root.as_mut().unwrap().children.clear();
        state
    }

    fn duplicated(mut state: ComputerState) -> ComputerState {
        let root = state.root.as_mut().unwrap();
        let copy = root.children.clone();
        root.children.extend(copy);
        state
    }

    struct Reads {
        count: Arc<AtomicUsize>,
    }

    impl Reads {
        fn new() -> Self {
            Self {
                count: Arc::default(),
            }
        }

        fn sequence(
            &self,
            states: Vec<ComputerState>,
        ) -> impl FnMut() -> std::future::Ready<Result<ComputerState, Error>> + use<> {
            let count = self.count.clone();
            move || {
                let index = count.fetch_add(1, Ordering::SeqCst);
                std::future::ready(Ok(states[index.min(states.len() - 1)].clone()))
            }
        }

        fn calls(&self) -> usize {
            self.count.load(Ordering::SeqCst)
        }
    }

    async fn wait(
        read: impl FnMut() -> std::future::Ready<Result<ComputerState, Error>>,
        target: &ComputerTarget,
        timeout_ms: u64,
        absent: bool,
    ) -> ComputerControlReadinessStatus {
        wait_for_control(read, resolve, target, timeout_ms, None, absent)
            .await
            .unwrap()
            .status
    }

    use ComputerControlReadinessStatus as Status;

    #[tokio::test(start_paused = true)]
    async fn returns_immediately_for_an_existing_control_without_waiting_the_timeout() {
        let reads = Reads::new();
        let readiness = wait_for_control(
            reads.sequence(vec![calculator_state()]),
            resolve,
            &target("Display"),
            10_000,
            None,
            false,
        )
        .await
        .unwrap();
        assert_eq!(readiness.status, Status::Ready);
        assert_eq!(readiness.waited_ms, 0);
        assert_eq!(reads.calls(), 1);
    }

    #[tokio::test(start_paused = true)]
    async fn polls_until_a_delayed_control_appears() {
        let reads = Reads::new();
        let read = reads.sequence(vec![
            without_controls(calculator_state()),
            calculator_state(),
        ]);
        assert_eq!(
            wait(read, &target("Display"), 1_000, false).await,
            Status::Ready
        );
        assert_eq!(reads.calls(), 2);
    }

    #[tokio::test(start_paused = true)]
    async fn stops_at_timeout_and_never_invents_readiness() {
        let reads = Reads::new();
        let read = reads.sequence(vec![calculator_state()]);
        assert_eq!(
            wait(read, &target("Missing"), 0, false).await,
            Status::Timeout
        );
    }

    #[tokio::test(start_paused = true)]
    async fn does_not_poll_unavailable_windows() {
        let mut partial = calculator_state();
        partial.accessibility = Some(ComputerAccessibilityReport {
            status: ComputerAccessibilityStatus::Partial,
            unavailable_window_ids: vec!["fake-calculator".to_owned()],
        });
        let reads = Reads::new();
        let read = reads.sequence(vec![partial]);
        assert_eq!(
            wait(read, &target("Display"), 10_000, false).await,
            Status::Unavailable
        );
        assert_eq!(reads.calls(), 1);
    }

    fn truncated(mut state: ComputerState) -> ComputerState {
        state.root.as_mut().unwrap().truncated = Some(true);
        state
    }

    #[tokio::test(start_paused = true)]
    async fn keeps_polling_a_truncated_tree_until_the_control_appears() {
        let reads = Reads::new();
        let read = reads.sequence(vec![
            truncated(without_controls(calculator_state())),
            truncated(calculator_state()),
        ]);
        assert_eq!(
            wait(read, &target("Display"), 1_000, false).await,
            Status::Ready
        );
        assert_eq!(reads.calls(), 2);
    }

    #[tokio::test(start_paused = true)]
    async fn a_truncated_tree_that_never_shows_the_control_ends_unavailable_not_timeout() {
        let reads = Reads::new();
        let read = reads.sequence(vec![truncated(without_controls(calculator_state()))]);
        assert_eq!(
            wait(read, &target("Display"), 1_000, false).await,
            Status::Unavailable
        );
        assert!(reads.calls() > 1);
    }

    #[tokio::test(start_paused = true)]
    async fn does_not_wait_for_a_closed_window() {
        let mut closed = calculator_state();
        closed.windows.clear();
        let reads = Reads::new();
        let read = reads.sequence(vec![closed]);
        assert_eq!(
            wait(read, &target("Display"), 10_000, false).await,
            Status::Closed
        );
        assert_eq!(reads.calls(), 1);
    }

    #[tokio::test(start_paused = true)]
    async fn does_not_claim_a_duplicate_label_is_ready() {
        let reads = Reads::new();
        let read = reads.sequence(vec![duplicated(calculator_state())]);
        assert_eq!(
            wait(read, &target("Display"), 10_000, false).await,
            Status::Ambiguous
        );
        assert_eq!(reads.calls(), 1);
    }

    #[tokio::test(start_paused = true)]
    async fn stops_polling_when_its_turn_is_cancelled() {
        let controller = CancellationToken::new();
        let signal: OperationSignal = controller.clone().into();
        let reads = Arc::new(AtomicUsize::new(0));
        let counter = reads.clone();
        let result = wait_for_control(
            move || {
                counter.fetch_add(1, Ordering::SeqCst);
                controller.cancel();
                std::future::ready(Ok::<_, Error>(without_controls(calculator_state())))
            },
            resolve,
            &target("Display"),
            10_000,
            Some(&signal),
            false,
        )
        .await;
        assert_eq!(result, Err(DesktopOperationError::Aborted));
        assert_eq!(reads.load(Ordering::SeqCst), 1);
    }

    #[tokio::test(start_paused = true)]
    async fn is_ready_immediately_when_the_control_is_already_absent() {
        let reads = Reads::new();
        let read = reads.sequence(vec![calculator_state()]);
        assert_eq!(
            wait(read, &target("Missing"), 10_000, true).await,
            Status::Ready
        );
        assert_eq!(reads.calls(), 1);
    }

    #[tokio::test(start_paused = true)]
    async fn keeps_polling_while_the_control_is_present_then_readies_after_it_vanishes() {
        let reads = Reads::new();
        let read = reads.sequence(vec![
            calculator_state(),
            without_controls(calculator_state()),
        ]);
        assert_eq!(
            wait(read, &target("Display"), 10_000, true).await,
            Status::Ready
        );
        assert_eq!(reads.calls(), 2);
    }

    #[tokio::test(start_paused = true)]
    async fn is_ready_when_the_target_window_closes() {
        let mut closed = calculator_state();
        closed.windows.clear();
        let reads = Reads::new();
        let read = reads.sequence(vec![closed]);
        assert_eq!(
            wait(read, &target("Display"), 10_000, true).await,
            Status::Ready
        );
        assert_eq!(reads.calls(), 1);
    }

    #[tokio::test(start_paused = true)]
    async fn does_not_call_disappearance_while_duplicates_remain_or_the_tree_is_incomplete() {
        let reads = Reads::new();
        let read = reads.sequence(vec![duplicated(calculator_state())]);
        assert_eq!(
            wait(read, &target("Display"), 0, true).await,
            Status::Timeout
        );
        let reads = Reads::new();
        let read = reads.sequence(vec![truncated(calculator_state())]);
        assert_eq!(
            wait(read, &target("Display"), 10_000, true).await,
            Status::Unavailable
        );
        let reads = Reads::new();
        let read = reads.sequence(vec![truncated(without_controls(calculator_state()))]);
        assert_eq!(
            wait(read, &target("Display"), 1_000, true).await,
            Status::Unavailable
        );
    }

    #[tokio::test(start_paused = true)]
    async fn propagates_a_resolver_failure_that_is_not_a_lookup_verdict() {
        let reads = Reads::new();
        let result = wait_for_control(
            reads.sequence(vec![calculator_state()]),
            |_: &ComputerUiNode, _: &ComputerTarget| {
                Err(ComputerTargetLookup::Failed(Error::Closed))
            },
            &target("Display"),
            10_000,
            None,
            false,
        )
        .await;
        assert_eq!(result, Err(Error::Closed));
    }
}
