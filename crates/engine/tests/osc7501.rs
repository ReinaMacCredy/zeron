//! Real PTY → engine projection → registry → sidebar derivation and MCP.
#![cfg(unix)]

use std::sync::Arc;
use std::time::Duration;

use base64::Engine as _;
use chrono::Utc;
use zeron_engine::{EngineCore, HarnessRegistry};
use zeron_proto::{ChatIndicator, HarnessId, Session, SessionStatus, TerminalEvent};

fn assemble(path: &std::path::Path) -> EngineCore {
    EngineCore::assemble(
        path,
        Arc::new(HarnessRegistry::new()),
        HarnessId::Mock,
        None,
    )
    .unwrap()
}

async fn state(
    rows: &mut tokio::sync::watch::Receiver<Vec<Session>>,
    expected: ChatIndicator,
    core: &EngineCore,
) -> Session {
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let snapshot = rows.borrow_and_update().clone();
            let chat = core.workspace.chat("chat").unwrap().unwrap();
            if let Some(row) = snapshot.into_iter().find(|row| row.chat_id == "chat")
                && zeron_proto::view::display_status(&chat, Some(&row), Utc::now()) == expected
            {
                return row;
            }
            rows.changed().await.unwrap();
        }
    })
    .await
    .expect("program status reaches the shared session projection")
}

fn emit(core: &EngineCore, id: &str, body: &str) {
    // Shell input contains printable escapes, so command echo cannot itself
    // impersonate an OSC report before printf actually runs.
    core.terminals
        .write_bytes(id, format!("printf '\\033]7501;{body}\\007'\r").as_bytes())
        .unwrap();
}

#[tokio::test]
async fn pty_states_match_sidebar_and_synced_registry_without_a_viewport() {
    let temp = tempfile::tempdir().unwrap();
    let core = assemble(temp.path());
    core.workspace
        .create_chat(
            "chat",
            None,
            Some(&core.device_id),
            None,
            Some(temp.path().display().to_string()),
        )
        .unwrap();
    let terminal = core
        .terminals
        .open_with_shell(temp.path().to_str().unwrap(), 80, 24, Some("/bin/sh"))
        .unwrap();
    core.terminals.associate_chat(&terminal.id, "chat").unwrap();
    let mut rows = core.sessions.watch_sessions();

    emit(&core, &terminal.id, "state=working:app=demo");
    let working = state(&mut rows, ChatIndicator::Working, &core).await;
    assert_eq!(
        working.status,
        SessionStatus::Idle,
        "native control state is untouched"
    );
    assert!(working.last_completed_turn.is_none());

    emit(
        &core,
        &terminal.id,
        "state=blocked:kind=question:app=demo:msg=V2hpY2ggYnJhbmNoPw==",
    );
    let blocked = state(&mut rows, ChatIndicator::AwaitingInput, &core).await;
    let report = &blocked.terminal_activity.as_ref().unwrap().report;
    assert_eq!(report.message.as_deref(), Some("Which branch?"));
    assert_eq!(
        core.workspace.watch_session_rows().borrow()[0].terminal_activity,
        blocked.terminal_activity
    );

    emit(&core, &terminal.id, "state=working:app=demo");
    state(&mut rows, ChatIndicator::Working, &core).await;
    emit(&core, &terminal.id, "state=done:app=demo");
    let done = state(&mut rows, ChatIndicator::Completed, &core).await;
    assert!(
        done.last_completed_turn.is_none(),
        "OSC done cannot forge a native completion receipt"
    );

    emit(
        &core,
        &terminal.id,
        "state=blocked:id=worker:kind=permission:app=demo",
    );
    state(&mut rows, ChatIndicator::AwaitingInput, &core).await;
    core.terminals.close(&terminal.id).unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if rows
                .borrow_and_update()
                .iter()
                .all(|row| row.terminal_activity.is_none())
            {
                break;
            }
            rows.changed().await.unwrap();
        }
    })
    .await
    .unwrap();
    core.shutdown().await;
}

#[tokio::test]
async fn feature_detection_is_answered_by_the_pty_owner_without_a_terminal_emulator() {
    let temp = tempfile::tempdir().unwrap();
    let core = assemble(temp.path());
    let terminal = core
        .terminals
        .open_with_shell(temp.path().to_str().unwrap(), 80, 24, Some("/bin/sh"))
        .unwrap();
    let reply =
        b"\x1b]7501;?:states=idle,working,done,blocked,error:kinds=permission,question,auth\x1b\\";
    core.terminals.write_bytes(&terminal.id, format!(
        "stty -echo -icanon min 1 time 0; printf '\\033]7501;?\\007'; reply=$(dd bs=1 count={} 2>/dev/null); case \"$reply\" in *states=idle,working,done,blocked,error*) printf '\\nFEATURE_OK\\n';; esac; exit\r", reply.len()
    ).as_bytes()).unwrap();
    // This subscribes only to raw output; no terminal emulator is present to
    // implement detection on the engine's behalf.
    let mut output = core.terminals.subscribe(&terminal.id, None).unwrap();
    let captured = tokio::time::timeout(Duration::from_secs(10), async {
        let mut bytes = Vec::new();
        while let Some(event) = output.recv().await {
            if let TerminalEvent::Data { data, .. } = event {
                bytes.extend(
                    base64::engine::general_purpose::STANDARD
                        .decode(data)
                        .unwrap(),
                );
            }
        }
        bytes
    })
    .await;
    core.shutdown().await;
    let captured = String::from_utf8_lossy(captured.as_ref().expect("feature probe completes"));
    assert!(captured.contains("\r\nFEATURE_OK\r\n"), "{captured}");
}
