use std::collections::HashSet;
use std::time::Duration;

use tokio_util::sync::CancellationToken;
use zeron_proto::computer::{ComputerHostMethod, CuaComputerTask, CuaEffect};

use super::*;
use crate::computer::backend::ComputerError;
use crate::computer::desktop_operation_queue::{
    DesktopOperationError, OperationSignal, with_desktop_operation_signal,
};
use crate::computer::task_context::with_computer_task;

fn cancelled(f: &Fixture) -> Vec<String> {
    f.cancelled.lock().unwrap().clone()
}

fn correlation(f: &Fixture, name: &str) -> String {
    f.named(name)
        .last()
        .map(|call| call.correlation_id.clone())
        .expect("call recorded")
}

async fn until_typing(f: &Fixture) {
    while f.knobs().typing_in_flight == 0 {
        tokio::task::yield_now().await;
    }
}

#[tokio::test]
async fn carries_an_engine_minted_correlation_id_task_and_delivery_mode_on_every_call() {
    let f = fixture();
    let task = CuaComputerTask {
        thread_id: "chat-1".into(),
        turn_id: Some("turn-1".into()),
        label: None,
    };
    with_computer_task(
        task.clone(),
        observing(f.backend.get_state(ComputerStateOptions {
            window_id: Some("cua:10:20".into()),
            include_screenshot: true,
            ..ComputerStateOptions::default()
        })),
    )
    .await
    .unwrap();
    let calls = f.calls();
    let ids: HashSet<&str> = calls
        .iter()
        .map(|call| call.correlation_id.as_str())
        .collect();
    assert_eq!(ids.len(), calls.len());
    assert!(calls.iter().all(|call| !call.correlation_id.is_empty()));
    for call in &calls {
        assert_eq!(call.method, ComputerHostMethod::Call);
        assert_eq!(call.task.as_ref(), Some(&task));
        assert_eq!(call.delivery_mode, Some(ComputerDeliveryMode::Background));
    }
    let observation = f.named("get_window_state");
    assert_eq!(observation[0].model_observation, Some(true));
    assert_eq!(f.named("list_windows")[0].model_observation, None);
    assert!(cancelled(&f).is_empty());
}

#[tokio::test]
async fn dropping_an_in_flight_action_cancels_its_host_request() {
    let f = fixture();
    let (_opener, gate) = gate();
    f.gate_type_text(Some(gate));
    let outcome = tokio::time::timeout(
        Duration::from_millis(50),
        f.backend.type_text("abc", Some("cua:10:20"), None),
    )
    .await;
    assert!(outcome.is_err());
    assert_eq!(cancelled(&f), vec![correlation(&f, "type_text")]);
}

#[tokio::test(start_paused = true)]
async fn a_hung_host_fails_a_mutation_at_the_deadline_as_dispatched_unknown_and_cancels_it() {
    let f = fixture();
    let (_opener, gate) = gate();
    f.gate_type_text(Some(gate));
    let started = tokio::time::Instant::now();
    let error = f
        .backend
        .type_text("abc", Some("cua:10:20"), None)
        .await
        .unwrap_err();
    assert_eq!(effect(&error), Some(CuaEffect::DispatchedUnknown));
    assert!(error.to_string().contains("timed out"));
    assert!(started.elapsed() >= Duration::from_millis(super::super::CUA_HOST_CALL_TIMEOUT_MS));
    assert_eq!(cancelled(&f), vec![correlation(&f, "type_text")]);
}

#[tokio::test(start_paused = true)]
async fn a_hung_host_fails_a_read_at_the_deadline_as_not_dispatched() {
    let f = fixture();
    let (_opener, gate) = gate();
    f.delay_window_state(Some(gate));
    let error = f
        .backend
        .capture_screenshot(window_capture("cua:10:20"))
        .await
        .unwrap_err();
    assert_eq!(effect(&error), Some(CuaEffect::NotDispatched));
    assert_eq!(cancelled(&f), vec![correlation(&f, "get_window_state")]);
}

#[tokio::test]
async fn aborting_the_operation_cancels_the_in_flight_host_request() {
    let f = fixture();
    let (_opener, gate) = gate();
    f.gate_type_text(Some(gate));
    let token = CancellationToken::new();
    let (outcome, ()) = tokio::join!(
        with_desktop_operation_signal(
            OperationSignal::from(token.clone()),
            f.backend.type_text("abc", Some("cua:10:20"), None),
        ),
        async {
            until_typing(&f).await;
            token.cancel();
        }
    );
    let error = outcome.unwrap_err();
    assert_eq!(effect(&error), Some(CuaEffect::DispatchedUnknown));
    assert!(error.to_string().contains("cancelled"));
    assert_eq!(cancelled(&f), vec![correlation(&f, "type_text")]);
}

#[tokio::test]
async fn an_already_aborted_operation_never_reaches_the_host() {
    let f = fixture();
    let token = CancellationToken::new();
    token.cancel();
    let error = with_desktop_operation_signal(
        OperationSignal::from(token),
        f.backend.type_text("abc", Some("cua:10:20"), None),
    )
    .await
    .unwrap_err();
    assert_eq!(
        error,
        ComputerError::Operation(DesktopOperationError::Aborted)
    );
    assert!(f.calls().is_empty());
}

#[tokio::test]
async fn refuses_a_request_past_the_byte_budget_before_dispatch() {
    let f = fixture();
    f.backend.focus_window("cua:10:20").await.unwrap();
    let error = f
        .backend
        .type_text(&"x".repeat(1024 * 1024), Some("cua:10:20"), None)
        .await
        .unwrap_err();
    assert_eq!(effect(&error), Some(CuaEffect::NotDispatched));
    assert!(error.to_string().contains("oversized"));
    assert!(f.named("type_text").is_empty());
    assert!(cancelled(&f).is_empty());
}

#[tokio::test]
async fn a_completed_call_is_never_cancelled() {
    let f = fixture();
    f.backend
        .type_text("abc", Some("cua:10:20"), None)
        .await
        .unwrap();
    assert!(cancelled(&f).is_empty());
}

#[tokio::test]
async fn native_element_identity_resolves_only_nodes_from_the_latest_snapshot() {
    let f = fixture();
    f.set_elements(vec![json!({
        "role": "AXButton",
        "label": "Save",
        "frame": { "x": -290, "y": 30, "width": 20, "height": 20 },
        "element_token": "save-token",
    })]);
    let root = f
        .backend
        .get_state(window_state("cua:10:20"))
        .await
        .unwrap()
        .root
        .unwrap();
    let node = root.children[0].clone();
    assert_eq!(node.node_path, Some(vec![0]));
    assert_eq!(
        f.backend.native_element_identity(&node).as_deref(),
        Some("save-token")
    );
    let moved = ComputerUiNode {
        label: Some("Save as".into()),
        ..node.clone()
    };
    assert_eq!(f.backend.native_element_identity(&moved), None);
    assert!(!serde_json::to_string(&root).unwrap().contains("save-token"));
}

#[tokio::test]
async fn announces_a_desktop_interruption_once_the_host_count_moves_past_its_baseline() {
    let f = fixture();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let sink = seen.clone();
    let _subscription = f.backend.on_event(Arc::new(move |event| {
        if let crate::computer::backend::ComputerBackendEvent::DesktopInterrupted { pauses } = event
        {
            sink.lock().unwrap().push(pauses.clone());
        }
    }));
    f.knobs().interruptions = Some(3);
    f.backend.probe_availability().await.unwrap();
    assert!(seen.lock().unwrap().is_empty());
    {
        let mut knobs = f.knobs();
        knobs.interruptions = Some(4);
        knobs.pauses = Some(vec!["screen-lock".into()]);
    }
    f.backend.probe_availability().await.unwrap();
    f.backend.probe_availability().await.unwrap();
    {
        let mut knobs = f.knobs();
        knobs.interruptions = Some(0);
        knobs.pauses = Some(Vec::new());
    }
    f.backend.probe_availability().await.unwrap();
    assert_eq!(
        *seen.lock().unwrap(),
        vec![vec!["screen-lock".to_owned()], Vec::<String>::new()]
    );
}
