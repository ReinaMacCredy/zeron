//! The engine's commands over RPC (`RunCommand` & co.), against a real
//! engine: what a caller runs, leaves in the background, reads and stops,
//! with real processes.
#![cfg(unix)]

use std::sync::Arc;
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use zeron_engine::{EngineCore, HarnessId, default_registry};
use zeron_rpc::{memory_client, methods};

#[tokio::test]
async fn the_engine_runs_commands_as_an_agent_s_own_shell_tool_would() {
    let dir = tempfile::tempdir().unwrap();
    let core = EngineCore::assemble(
        dir.path(),
        Arc::new(default_registry()),
        HarnessId::Mock,
        None,
    )
    .unwrap();
    let client = memory_client(core.rpc_service());
    let work = tempfile::tempdir().unwrap();
    std::fs::create_dir(work.path().join("app")).unwrap();
    let app = std::fs::canonicalize(work.path().join("app")).unwrap();

    // Output and exit code; the folder it ended in, for the next command.
    let ran = client
        .call(
            methods::RUN_COMMAND,
            json!({ "command": "cd app && echo hi && exit 3", "cwd": work.path() }),
        )
        .await
        .unwrap();
    assert_eq!(
        (ran["output"].as_str(), ran["exitCode"].as_i64()),
        (Some("hi\n"), Some(3)),
        "{ran}"
    );
    let ended_in = ran["cwd"].as_str().expect("where it ended").to_owned();
    let ran = client
        .call(
            methods::RUN_COMMAND,
            json!({ "command": "pwd -P", "cwd": ended_in }),
        )
        .await
        .unwrap();
    assert_eq!(ran["output"].as_str().map(str::trim), app.to_str(), "{ran}");

    // A server left running: read as it goes, then stopped.
    let started = client
        .call(
            methods::RUN_COMMAND,
            json!({ "command": "echo listening; while true; do sleep 0.05; done", "background": true, "cwd": work.path() }),
        )
        .await
        .unwrap();
    let id = started["id"].as_str().unwrap().to_owned();
    let mut printed = String::new();
    let give_up = Instant::now() + Duration::from_secs(10);
    while !printed.contains("listening") {
        assert!(Instant::now() < give_up, "{printed:?}");
        let read: Value = client
            .call(methods::READ_COMMAND_OUTPUT, json!({ "id": id }))
            .await
            .unwrap();
        printed.push_str(read["output"].as_str().unwrap_or_default());
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let listed = client
        .call(methods::LIST_BACKGROUND_COMMANDS, json!({}))
        .await
        .unwrap();
    assert!(
        listed
            .as_array()
            .unwrap()
            .iter()
            .any(|c| c["id"] == id.as_str() && c["status"] == "running"),
        "{listed}"
    );
    let stopped = client
        .call(methods::STOP_COMMAND, json!({ "id": id }))
        .await
        .unwrap();
    assert_eq!(stopped["status"], "stopped", "{stopped}");
    core.sessions.shutdown().await;
}
