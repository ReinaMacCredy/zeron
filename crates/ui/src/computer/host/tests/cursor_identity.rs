use super::*;
use crate::computer::host::{ComputerFrameTapHost, ComputerShieldEngagement, ComputerShieldHost};
use zeron_proto::computer::CuaPreviewTarget;

fn press_for(task: Option<&CuaComputerTask>) -> ComputerHostCall {
    match task {
        Some(task) => press().task(task),
        None => press(),
    }
}

fn end_task(task: &CuaComputerTask) -> ComputerHostCall {
    method(ComputerHostMethod::EndTask).task(task)
}

#[tokio::test]
async fn parks_between_actions_hides_only_the_completed_turn_and_preserves_its_session() {
    let f = fixture(Driver {
        log_sessions: true,
        log_cursor_state: true,
        ..Driver::default()
    });
    let first = task("cursor-thread", Some("turn-1"));
    let next = task("cursor-thread", Some("turn-2"));
    f.send(press_for(Some(&first))).await;
    f.send(press_for(Some(&next))).await;
    f.send(end_task(&first)).await;
    assert!(!f.has("cursor-enabled:false:agent·cursor-thread"));
    f.send(end_task(&next)).await;
    assert!(f.has("cursor-enabled:false:agent·cursor-thread"));
    assert!(!f.names().iter().any(|e| e.contains("end_session")));
    f.send(press_for(Some(&task("cursor-thread", Some("turn-3")))))
        .await;
    assert_eq!(f.count("cursor-enabled:true:agent·cursor-thread"), 2);
    assert_eq!(f.count("start"), 1);
}

#[tokio::test]
async fn reads_cursor_state_only_on_mint_or_first_action_and_logs_no_labels_or_content() {
    let (f, logs) = logged_fixture(
        Driver {
            log_cursor_state: true,
            ..Driver::default()
        },
        |_| {},
    );
    let owner = CuaComputerTask {
        label: Some("PRIVATE LABEL".into()),
        ..task("cursor-thread", Some("turn-1"))
    };
    f.send(call("get_window_state").args(json!({})).task(&owner))
        .await;
    f.send(press_for(Some(&owner))).await;
    f.send(press_for(Some(&owner))).await;
    assert_eq!(
        f.names()
            .iter()
            .filter(|e| e.starts_with("cursor-state:"))
            .count(),
        2
    );
    let text = logs.text();
    assert!(text.contains(r#""stage":"session-created""#), "{text}");
    assert!(text.contains(r#""stage":"first-action""#), "{text}");
    assert!(text.contains(r#""overlay_scope":"main_display""#), "{text}");
    assert!(!text.contains("PRIVATE LABEL"), "{text}");
    assert!(!text.contains(r#""x":10"#), "{text}");
}

#[tokio::test]
async fn retries_unacknowledged_cursor_visibility_without_retrying_input() {
    let f = fixture(Driver {
        log_cursor_state: true,
        cursor_enable_failures: 1,
        cursor_hide_failures: 1,
        ..Driver::default()
    });
    let owner = task("cursor-retry", Some("turn-1"));
    f.send(press_for(Some(&owner))).await;
    f.send(press_for(Some(&owner))).await;
    for _ in 0..3 {
        f.send(end_task(&owner)).await;
    }
    assert_eq!(f.count("cursor-enabled:true:agent·cursor-retry"), 2);
    assert_eq!(f.count("cursor-enabled:false:agent·cursor-retry"), 2);
    assert_eq!(f.count("key"), 2);
}

struct EndSignalTap(Deferred<()>);

#[async_trait::async_trait(?Send)]
impl ComputerFrameTapHost for EndSignalTap {
    fn update(&self, _target: CuaPreviewTarget) -> Result<(), String> {
        Ok(())
    }
    fn touch(&self, _task: &CuaComputerTask) {}
    async fn end_task(&self, _task: CuaComputerTask) -> Result<(), String> {
        self.0.resolve(());
        Ok(())
    }
    async fn stop(&self) -> Result<(), String> {
        Ok(())
    }
    async fn dispose(&self) -> Result<(), String> {
        Ok(())
    }
}

struct EndSignalShield(Deferred<()>);

#[async_trait::async_trait(?Send)]
impl ComputerShieldHost for EndSignalShield {
    async fn engage(
        &self,
        _request: ComputerShieldEngagement,
        _task: Option<CuaComputerTask>,
    ) -> Result<(), String> {
        Ok(())
    }
    async fn release(&self, _shield_id: String) -> Result<(), String> {
        Ok(())
    }
    async fn release_all(&self) -> Result<u64, String> {
        Ok(0)
    }
    async fn end_task(&self, _task: CuaComputerTask) -> Result<(), String> {
        self.0.resolve(());
        Ok(())
    }
    async fn stop(&self) -> Result<(), String> {
        Ok(())
    }
    async fn dispose(&self) -> Result<(), String> {
        Ok(())
    }
}

#[tokio::test]
async fn starts_preview_and_shield_cleanup_before_waiting_for_the_cursor_queue() {
    let frame_ended = Deferred::new();
    let shield_ended = Deferred::new();
    let (tap, shield) = (frame_ended.clone(), shield_ended.clone());
    let owner = task("queued-cleanup", Some("turn-1"));
    let f = fixture_with(
        Driver {
            input_delay_ms: Some(300),
            ..Driver::default()
        },
        |o| {
            o.frame_tap = Some(Arc::new(EndSignalTap(tap)));
            o.shield = Some(Arc::new(EndSignalShield(shield)));
        },
    );
    let action = f.spawn(
        call("type_text")
            .args(json!({ "text": "fixture" }))
            .task(&owner),
    );
    f.wait_for("dispatch").await;
    let end = f.spawn(end_task(&owner));
    frame_ended.wait().await;
    shield_ended.wait().await;
    assert!(!f.has("effect"));
    action.reply().await;
    end.reply().await;
}

#[tokio::test]
async fn reports_a_failed_cursor_query_without_replaying_input_or_retiring_the_driver() {
    let (f, logs) = logged_fixture(
        Driver {
            cursor_unavailable: true,
            ..Driver::default()
        },
        |_| {},
    );
    let owner = task("cursor-thread", None);
    assert!(f.send(press_for(Some(&owner))).await.ok);
    assert!(f.send(press_for(Some(&owner))).await.ok);
    assert_eq!(f.count("key"), 2);
    assert_eq!(f.count("start"), 1);
    let text = logs.text();
    assert!(text.contains(r#""status":"unavailable""#), "{text}");
    assert!(!text.contains("private overlay error"), "{text}");
}

#[tokio::test]
async fn logs_structured_actuator_failures_with_attribution_but_no_raw_native_message() {
    let (f, logs) = logged_fixture(
        Driver {
            action_result: Some(json!({
                "isError": true,
                "content": [{ "type": "text", "text": "PRIVATE FIELD VALUE" }],
                "structuredContent": {
                    "effect": "dispatched-unknown",
                    "diagnostics": {
                        "delivery_path": "ax",
                        "actuator": "ax_press",
                        "error_code": "ax_dispatch_failed",
                        "ax_error": -25204,
                        "message": "PRIVATE FIELD VALUE",
                        "title": "PRIVATE WINDOW",
                    },
                },
            })),
            ..Driver::default()
        },
        |_| {},
    );
    f.send(press_for(Some(&task(
        "failure-thread",
        Some("failure-turn"),
    ))))
    .await;
    let text = logs.text();
    assert!(text.contains(r#""event":"computer_action""#), "{text}");
    assert!(text.contains(r#""turn":"failure-turn""#), "{text}");
    assert!(text.contains(r#""ax_error":-25204"#), "{text}");
    assert!(
        text.contains("The accessibility actuator reported a native error."),
        "{text}"
    );
    assert!(!text.contains("PRIVATE FIELD VALUE"), "{text}");
    assert!(!text.contains("PRIVATE WINDOW"), "{text}");
}

#[tokio::test]
async fn dispatches_each_tasks_calls_under_its_own_cursor_session_label() {
    let f = fixture(Driver {
        log_sessions: true,
        ..Driver::default()
    });
    assert!(
        f.send(press_for(Some(&labelled("t-1", "Research run"))))
            .await
            .ok
    );
    assert!(
        f.send(press_for(Some(&labelled("t-2", "Docs pass"))))
            .await
            .ok
    );
    assert!(
        f.send(press_for(Some(&labelled("t-1", "Research run"))))
            .await
            .ok
    );
    assert!(f.send(press_for(None)).await.ok);
    let names = f.names();
    assert!(f.has("session:agent·Research run·t-1:press_key"));
    assert!(f.has("session:agent·Docs pass·t-2:press_key"));
    assert!(
        names
            .iter()
            .any(|e| e.starts_with("session:synara-") && e.ends_with(":press_key"))
    );
    let opened: Vec<&String> = names
        .iter()
        .filter(|e| e.starts_with("open_session:start_session:"))
        .collect();
    assert_eq!(opened.len(), 1, "{opened:?}");
    let session = opened[0]
        .strip_prefix("open_session:start_session:synara-")
        .unwrap();
    assert!(
        !session.is_empty() && session.chars().all(|c| c.is_ascii_hexdigit() || c == '-'),
        "{session}"
    );
}

#[tokio::test]
async fn keeps_cursors_distinct_when_two_threads_share_one_display_label() {
    let f = fixture(Driver {
        log_sessions: true,
        ..Driver::default()
    });
    assert!(
        f.send(press_for(Some(&labelled("t-1", "Research run"))))
            .await
            .ok
    );
    assert!(
        f.send(press_for(Some(&labelled("t-2", "Research run"))))
            .await
            .ok
    );
    assert!(f.has("session:agent·Research run·t-1:press_key"));
    assert!(f.has("session:agent·Research run·t-2:press_key"));
}

#[tokio::test]
async fn falls_back_to_the_thread_id_when_a_task_carries_no_display_label() {
    let f = fixture(Driver {
        log_sessions: true,
        ..Driver::default()
    });
    assert!(f.send(press_for(Some(&task("t-9", None)))).await.ok);
    assert!(f.has("session:agent·t-9:press_key"));
}

#[tokio::test]
async fn sanitizes_badge_breaking_characters_out_of_the_minted_label() {
    let f = fixture(Driver {
        log_sessions: true,
        ..Driver::default()
    });
    assert!(
        f.send(press_for(Some(&labelled(
            "t-1",
            "Res\u{0}earch\nrun\u{200B}"
        ))))
        .await
        .ok
    );
    assert!(f.has("session:agent·Researchrun·t-1:press_key"));
}

#[tokio::test]
async fn a_caller_session_arg_can_never_override_the_minted_agent_label() {
    let f = fixture(Driver {
        log_sessions: true,
        ..Driver::default()
    });
    let reply = f
        .send(
            call("press_key")
                .task(&labelled("t-1", "Research run"))
                .args(json!({ "key": "enter", "session": "forged" })),
        )
        .await;
    assert!(reply.ok);
    assert!(!f.names().iter().any(|e| e.starts_with("session:forged")));
    assert!(f.has("session:agent·Research run·t-1:press_key"));
}

#[tokio::test]
async fn revives_an_ended_task_session_in_place_instead_of_retiring_the_generation() {
    let f = fixture(Driver {
        session_death_once: true,
        log_sessions: true,
        ..Driver::default()
    });
    assert_clean_ok(
        &f.send(press_for(Some(&labelled("t-1", "Research run"))))
            .await,
    );
    assert_eq!(f.count("start"), 1);
    assert!(f.has("open_session:start_session:agent·Research run·t-1"));
    assert_eq!(f.count("key"), 1);
    assert!(f.has("session:agent·Research run·t-1:press_key"));
}
