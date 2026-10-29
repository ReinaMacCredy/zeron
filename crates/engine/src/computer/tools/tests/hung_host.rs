use std::sync::Arc;
use std::time::Duration;

use serde_json::json;
use zeron_proto::computer::McpToolResult;

use super::{Approvals, THREAD, ToolSeams, args, result_json};
use crate::computer::cua_backend::CUA_HOST_CALL_TIMEOUT_MS;
use crate::computer::cua_backend::tests::{FixtureOptions, fixture_with, gate};
use crate::computer::manager::{ComputerManager, ComputerManagerEnv, ComputerManagerOptions};
use crate::computer::tools::{ComputerToolContext, ComputerTools};

#[tokio::test(start_paused = true)]
async fn a_hung_host_fails_the_tool_call_at_its_deadline_instead_of_hanging() {
    let fixture = fixture_with(FixtureOptions::default());
    let (_opener, held) = gate();
    fixture.gate_type_text(Some(held));
    let mut options = ComputerManagerOptions::new(fixture.backend.clone());
    options.seams = Arc::new(Approvals {
        mode: std::sync::Mutex::new(Some(zeron_proto::AccessMode::FullAccess)),
        decide: std::sync::Mutex::new(None),
        gate: options.approval_gate.clone(),
        cards: std::sync::Mutex::default(),
        turn_inactive: std::sync::atomic::AtomicBool::new(false),
    });
    options.action_settle_ms = Some(0);
    options.env = ComputerManagerEnv::default();
    let manager = ComputerManager::new(options);
    let tools = ComputerTools::new(manager.clone(), Vec::new());
    let context = ComputerToolContext {
        chat_id: THREAD.into(),
        chat_label: None,
        turn_id: Some("turn-hung".into()),
        session_key: "session-hung".into(),
        request_id: json!(1),
        signal: tokio_util::sync::CancellationToken::new(),
        seams: Arc::new(ToolSeams::default()),
        mutation_dispatched: Arc::default(),
    };
    let started = tokio::time::Instant::now();
    let result: McpToolResult = tools
        .call(
            "computer_type_text",
            args(json!({ "text": "abc", "window_id": "cua:10:20", "include_screenshot": false })),
            &context,
        )
        .await;
    assert!(result.is_error, "{result:?}");
    let payload = result_json(&result);
    assert_eq!(payload["effect"], json!("dispatched-unknown"), "{result:?}");
    assert!(
        payload["message"]
            .as_str()
            .unwrap_or_default()
            .contains("timed out")
    );
    assert!(started.elapsed() >= Duration::from_millis(CUA_HOST_CALL_TIMEOUT_MS));
    assert!(started.elapsed() < Duration::from_millis(CUA_HOST_CALL_TIMEOUT_MS * 2));
    assert!(context.mutation_dispatched());
    assert_eq!(fixture.cancelled.lock().unwrap().len(), 1);
    manager.dispose().await;
}
