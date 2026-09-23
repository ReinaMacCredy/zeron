use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use serde_json::json;
use tokio_util::sync::CancellationToken;
use zeron_proto::computer::{ComputerDeliveryVerification, CuaEffect};

use super::*;
use crate::computer::desktop_operation_queue::{
    DesktopOperationError, OperationSignal, with_desktop_operation_signal,
};

fn text_field(label: &str, y: i64, token: &str) -> Value {
    json!({
        "role": "AXTextField",
        "label": label,
        "frame": { "x": -290, "y": y, "width": 120, "height": 20 },
        "element_token": token,
    })
}

fn web_field() -> Value {
    json!({
        "role": "AXTextField",
        "label": "Message",
        "frame": { "x": -290, "y": 30, "width": 120, "height": 20 },
        "element_token": "web-token",
        "element_index": 2,
        "in_web_content": true,
        "value": "seed",
    })
}

fn extra_window(pid: u64, window_id: u64, title: &str, x: i64) -> Value {
    json!({
        "pid": pid,
        "window_id": window_id,
        "title": title,
        "bounds": { "x": x, "y": 20, "width": 200, "height": 100 },
        "is_on_screen": true,
        "on_current_space": true,
        "z_index": 0,
    })
}

async fn observe(f: &Fixture, window_id: &str, index: usize) -> ComputerResolvedTarget {
    let root = f
        .backend
        .get_state(window_state(window_id))
        .await
        .unwrap()
        .root
        .unwrap();
    let node = &root.children[index];
    resolved_in(node.label.as_deref().unwrap_or(""), Some(window_id), node)
}

async fn wait_for(condition: impl Fn() -> bool) {
    let started = std::time::Instant::now();
    while !condition() {
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "condition never held"
        );
        tokio::time::sleep(Duration::from_millis(1)).await;
    }
}

fn texts(f: &Fixture, name: &str, field: &str) -> Vec<Value> {
    f.named(name)
        .into_iter()
        .map(|call| call.args[field].clone())
        .collect()
}

fn is_cancelled(error: &ComputerError) -> bool {
    matches!(
        error,
        ComputerError::Operation(DesktopOperationError::Aborted)
    )
}

#[tokio::test]
async fn reports_dispatched_unknown_when_a_web_set_value_does_not_land() {
    let f = fixture();
    f.set_elements(vec![web_field()]);
    let target = observe(&f, "cua:10:20", 0).await;
    f.swallow_set_value();
    let result = f
        .backend
        .type_text("-typed", Some("cua:10:20"), Some(&target))
        .await
        .unwrap();
    assert_eq!(
        result.verified,
        Some(ComputerDeliveryVerification::Unconfirmed)
    );
    assert_eq!(result.effect, Some(CuaEffect::DispatchedUnknown));
    let reads = f.named("get_window_state");
    assert_eq!(reads.len(), 3);
    for read in reads {
        assert_eq!(read.args["include_screenshot"], json!(false));
    }
}

#[tokio::test]
async fn appends_label_targeted_web_text_through_the_driver_not_the_snapshot_value() {
    let f = fixture_with(FixtureOptions {
        native_revision: Some(Some(39)),
        ..Default::default()
    });
    let mut field = web_field();
    field["value"] = json!("SAVE10");
    f.set_elements(vec![field]);
    let target = observe(&f, "cua:10:20", 0).await;
    f.backend
        .type_text("save10", Some("cua:10:20"), Some(&target))
        .await
        .unwrap();
    let writes = f.named("set_value");
    assert_eq!(writes.len(), 1);
    assert_eq!(writes[0].args["value"], json!("save10"));
    assert_eq!(writes[0].args["append"], json!(true));
}

#[tokio::test]
async fn never_sends_web_action_after_interruption_while_its_field_read_was_pending() {
    for (action, interruption) in [
        ("typeText", "timeout"),
        ("typeText", "cancellation"),
        ("typeText", "elapsed deadline"),
        ("setValue", "timeout"),
        ("setValue", "cancellation"),
        ("setValue", "elapsed deadline"),
    ] {
        let f = fixture_with(FixtureOptions {
            lane_gap_ms: Some(0),
            lane_hold_ms: Some(if interruption == "timeout" { 80 } else { 1_000 }),
            ..Default::default()
        });
        f.set_elements(vec![web_field()]);
        let target = observe(&f, "cua:10:20", 0).await;
        let (opener, read) = gate();
        f.delay_window_state(Some(read));
        let token = CancellationToken::new();
        let write = async {
            if action == "typeText" {
                f.backend
                    .type_text("expired", Some("cua:10:20"), Some(&target))
                    .await
            } else {
                f.backend.set_value(&target, "expired").await
            }
        };
        let pending = with_desktop_operation_signal(OperationSignal::from(token.clone()), write);
        let driver = async {
            wait_for(|| f.named("get_window_state").len() == 2).await;
            match interruption {
                "cancellation" => {
                    token.cancel();
                    None
                }
                "elapsed deadline" => {
                    tokio::time::pause();
                    tokio::time::advance(Duration::from_millis(2_000)).await;
                    opener.open();
                    None
                }
                _ => Some(opener),
            }
        };
        let (outcome, opener) = tokio::join!(pending, driver);
        if interruption == "elapsed deadline" {
            tokio::time::resume();
        }
        let error = outcome.unwrap_err();
        if interruption == "cancellation" {
            assert!(is_cancelled(&error), "{action} {interruption}: {error}");
        } else {
            assert_eq!(
                effect(&error),
                Some(CuaEffect::NotDispatched),
                "{action} {interruption}: {error}"
            );
        }
        assert!(f.named("set_value").is_empty(), "{action} {interruption}");

        f.delay_window_state(None);
        if let Some(opener) = opener {
            opener.open();
        }
        let allowed = f.backend.set_value(&target, "allowed").await.unwrap();
        assert_eq!(
            allowed.effect,
            Some(CuaEffect::Verified),
            "{action} {interruption}"
        );
        assert_eq!(texts(&f, "set_value", "value"), vec![json!("allowed")]);
        assert!(f.named("type_text").is_empty());
    }
}

#[tokio::test]
async fn semantic_text_lane_serializes_same_window_writes() {
    let f = fixture_with(FixtureOptions {
        lane_gap_ms: Some(0),
        ..Default::default()
    });
    f.set_elements(vec![
        text_field("Message", 30, "message-token"),
        text_field("Notes", 60, "notes-token"),
    ]);
    let root = f
        .backend
        .get_state(window_state("cua:10:20"))
        .await
        .unwrap()
        .root
        .unwrap();
    let first_target = resolved_in("Message", Some("cua:10:20"), &root.children[0]);
    let second_target = resolved_in("Notes", Some("cua:10:20"), &root.children[1]);
    let (opener, held) = gate();
    f.gate_type_text(Some(held));

    let first = f
        .backend
        .type_text("alpha", Some("cua:10:20"), Some(&first_target));
    let driver = async {
        wait_for(|| f.named("type_text").len() == 1).await;
        let second = f
            .backend
            .type_text("bravo", Some("cua:10:20"), Some(&second_target));
        let check = async {
            tokio::time::sleep(Duration::from_millis(50)).await;
            assert_eq!(f.named("type_text").len(), 1);
            assert_eq!(f.typing_max_in_flight(), 1);
            opener.open();
        };
        tokio::join!(second, check).0
    };
    let (first, second) = tokio::join!(first, driver);
    first.unwrap();
    second.unwrap();
    assert_eq!(
        texts(&f, "type_text", "text"),
        vec![json!("alpha"), json!("bravo")]
    );
}

#[tokio::test]
async fn semantic_text_lane_holds_select_text_behind_a_same_window_write() {
    let f = fixture_with(FixtureOptions {
        lane_gap_ms: Some(0),
        ..Default::default()
    });
    f.set_elements(vec![text_field("Message", 30, "message-token")]);
    let target = observe(&f, "cua:10:20", 0).await;
    let (opener, held) = gate();
    f.gate_type_text(Some(held));

    let typing = f
        .backend
        .type_text("alpha", Some("cua:10:20"), Some(&target));
    let driver = async {
        wait_for(|| !f.named("type_text").is_empty()).await;
        let selecting = f.backend.select_text(&target, range(0, 2));
        let check = async {
            tokio::time::sleep(Duration::from_millis(50)).await;
            assert!(f.named("select_text").is_empty());
            opener.open();
        };
        tokio::join!(selecting, check).0
    };
    let (typing, selecting) = tokio::join!(typing, driver);
    typing.unwrap();
    selecting.unwrap();
    let order: Vec<String> = f
        .calls()
        .into_iter()
        .filter_map(|call| call.name)
        .filter(|name| name == "type_text" || name == "select_text")
        .collect();
    assert_eq!(order, vec!["type_text", "select_text"]);
}

#[tokio::test]
async fn semantic_text_lane_holds_set_value_behind_a_same_window_write() {
    let f = fixture_with(FixtureOptions {
        lane_gap_ms: Some(0),
        ..Default::default()
    });
    f.set_elements(vec![text_field("Message", 30, "message-token")]);
    let target = observe(&f, "cua:10:20", 0).await;
    let (opener, held) = gate();
    f.gate_type_text(Some(held));

    let typing = f
        .backend
        .type_text("alpha", Some("cua:10:20"), Some(&target));
    let driver = async {
        wait_for(|| !f.named("type_text").is_empty()).await;
        let writing = f.backend.set_value(&target, "beta");
        let check = async {
            tokio::time::sleep(Duration::from_millis(50)).await;
            assert!(f.named("set_value").is_empty());
            opener.open();
        };
        tokio::join!(writing, check).0
    };
    let (typing, writing) = tokio::join!(typing, driver);
    typing.unwrap();
    writing.unwrap();
    let order: Vec<String> = f
        .calls()
        .into_iter()
        .filter_map(|call| call.name)
        .filter(|name| name == "type_text" || name == "set_value")
        .collect();
    assert_eq!(order, vec!["type_text", "set_value"]);
}

#[tokio::test]
async fn set_value_dispatches_to_a_window_that_is_not_on_the_current_space() {
    let f = fixture();
    f.set_visible(false);
    f.set_elements(vec![text_field("Message", 30, "message-token")]);
    let target = observe(&f, "cua:10:20", 0).await;
    f.backend.set_value(&target, "beta").await.unwrap();
    assert!(!f.named("set_value").is_empty());
    assert_subset(
        &f.first_args("set_value"),
        json!({ "element_token": "message-token", "pid": 10, "window_id": 20 }),
    );
}

#[tokio::test]
async fn semantic_text_lane_overlaps_same_pid_writes_to_different_windows() {
    let f = fixture_with(FixtureOptions {
        lane_gap_ms: Some(0),
        ..Default::default()
    });
    f.set_windows(vec![extra_window(10, 21, "Owned fixture B", 100)]);
    f.set_elements(vec![text_field("Message", 30, "message-token")]);
    let first_target = observe(&f, "cua:10:20", 0).await;
    f.capture_window(21, 10);
    let second_target = observe(&f, "cua:10:21", 0).await;
    let (opener, held) = gate();
    f.gate_type_text(Some(held));

    let first = f
        .backend
        .type_text("alpha", Some("cua:10:20"), Some(&first_target));
    let second = f
        .backend
        .type_text("bravo", Some("cua:10:21"), Some(&second_target));
    let driver = async {
        wait_for(|| f.named("type_text").len() == 2).await;
        assert_eq!(f.typing_max_in_flight(), 2);
        opener.open();
    };
    let (first, second, ()) = tokio::join!(first, second, driver);
    first.unwrap();
    second.unwrap();
}

#[tokio::test]
async fn semantic_text_lane_overlaps_different_pid_writes() {
    let f = fixture_with(FixtureOptions {
        lane_gap_ms: Some(0),
        ..Default::default()
    });
    f.set_windows(vec![extra_window(11, 21, "Owned fixture B", 100)]);
    f.set_elements(vec![text_field("Message", 30, "message-token")]);
    let first_target = observe(&f, "cua:10:20", 0).await;
    f.capture_window(21, 11);
    let second_target = observe(&f, "cua:11:21", 0).await;
    let (opener, held) = gate();
    f.gate_type_text(Some(held));

    let first = f
        .backend
        .type_text("alpha", Some("cua:10:20"), Some(&first_target));
    let second = f
        .backend
        .type_text("bravo", Some("cua:11:21"), Some(&second_target));
    let driver = async {
        wait_for(|| f.named("type_text").len() == 2).await;
        assert_eq!(f.typing_max_in_flight(), 2);
        opener.open();
    };
    let (first, second, ()) = tokio::join!(first, second, driver);
    first.unwrap();
    second.unwrap();
}

#[tokio::test]
async fn semantic_text_lane_interleaves_three_same_pid_windows_truly_concurrently() {
    let f = fixture_with(FixtureOptions {
        lane_gap_ms: Some(0),
        ..Default::default()
    });
    f.set_windows(vec![
        extra_window(10, 21, "Owned fixture B", 100),
        extra_window(10, 22, "Owned fixture C", 500),
    ]);
    f.set_elements(vec![text_field("Message", 30, "message-token")]);
    let first_target = observe(&f, "cua:10:20", 0).await;
    f.capture_window(21, 10);
    let second_target = observe(&f, "cua:10:21", 0).await;
    f.capture_window(22, 10);
    let third_target = observe(&f, "cua:10:22", 0).await;
    let (opener, held) = gate();
    f.gate_type_text(Some(held));

    let first = f
        .backend
        .type_text("alpha", Some("cua:10:20"), Some(&first_target));
    let second = f
        .backend
        .type_text("bravo", Some("cua:10:21"), Some(&second_target));
    let third = f
        .backend
        .type_text("charlie", Some("cua:10:22"), Some(&third_target));
    let driver = async {
        wait_for(|| f.named("type_text").len() == 3).await;
        assert_eq!(f.typing_max_in_flight(), 3);
        opener.open();
    };
    let (first, second, third, ()) = tokio::join!(first, second, third, driver);
    first.unwrap();
    second.unwrap();
    third.unwrap();
    assert_eq!(
        texts(&f, "type_text", "window_id"),
        vec![json!(20), json!(21), json!(22)]
    );
}

#[tokio::test]
async fn semantic_text_lane_leaves_synthetic_keyboard_writes_alone() {
    let f = fixture_with(FixtureOptions {
        lane_gap_ms: Some(0),
        ..Default::default()
    });
    let (opener, held) = gate();
    f.gate_type_text(Some(held));

    let first = f.backend.type_text("a", Some("cua:10:20"), None);
    let second = f.backend.type_text("b", Some("cua:10:20"), None);
    let driver = async {
        wait_for(|| f.named("type_text").len() == 2).await;
        opener.open();
    };
    let (first, second, ()) = tokio::join!(first, second, driver);
    first.unwrap();
    second.unwrap();
}

#[tokio::test]
async fn keeps_a_timed_out_semantic_write_in_its_lane_until_late_outcome() {
    for outcome in ["success", "failure"] {
        let f = fixture_with(FixtureOptions {
            lane_gap_ms: Some(0),
            lane_hold_ms: Some(80),
            ..Default::default()
        });
        f.set_elements(vec![text_field("Message", 30, "message-token")]);
        let target = observe(&f, "cua:10:20", 0).await;
        let (opener, held) = gate();
        f.gate_type_text(Some(held));

        let error = f
            .backend
            .type_text("alpha", Some("cua:10:20"), Some(&target))
            .await
            .unwrap_err();
        assert_eq!(
            effect(&error),
            Some(CuaEffect::DispatchedUnknown),
            "{outcome}"
        );
        let second = f
            .backend
            .type_text("beta", Some("cua:10:20"), Some(&target));
        let driver = async {
            tokio::time::sleep(Duration::from_millis(10)).await;
            assert_eq!(f.named("type_text").len(), 1);
            assert_eq!(f.typing_max_in_flight(), 1);
            f.gate_type_text(None);
            if outcome == "failure" {
                let failed = Arc::new(AtomicBool::new(false));
                f.on_tool("type_text", move |_| {
                    if failed.swap(true, Ordering::SeqCst) {
                        json!({ "structuredContent": {
                            "route": "synthetic_events",
                            "delivery": { "mode": "background" },
                            "effect": "unverifiable",
                        } })
                    } else {
                        json!({
                            "isError": true,
                            "content": [{ "type": "text", "text": "Late native failure" }],
                        })
                    }
                });
            }
            opener.open();
        };
        let (second, ()) = tokio::join!(second, driver);
        assert_eq!(
            second.unwrap().window_id.as_deref(),
            Some("cua:10:20"),
            "{outcome}"
        );
        assert_eq!(
            texts(&f, "type_text", "text"),
            vec![json!("alpha"), json!("beta")]
        );
        assert_eq!(f.typing_max_in_flight(), 1);
    }
}

#[tokio::test]
async fn expires_a_queued_semantic_write_without_dispatching_it_after_the_lane_drains() {
    let f = fixture_with(FixtureOptions {
        lane_gap_ms: Some(0),
        lane_hold_ms: Some(40),
        ..Default::default()
    });
    f.set_elements(vec![text_field("Message", 30, "message-token")]);
    let target = observe(&f, "cua:10:20", 0).await;
    let (opener, held) = gate();
    f.gate_type_text(Some(held));

    let error = f
        .backend
        .type_text("alpha", Some("cua:10:20"), Some(&target))
        .await
        .unwrap_err();
    assert_eq!(effect(&error), Some(CuaEffect::DispatchedUnknown));
    assert_eq!(code(&error), Some("cua_action_failed"));
    let error = f
        .backend
        .type_text("expired", Some("cua:10:20"), Some(&target))
        .await
        .unwrap_err();
    assert_eq!(effect(&error), Some(CuaEffect::NotDispatched));
    assert_eq!(f.named("type_text").len(), 1);
    f.gate_type_text(None);
    opener.open();
    let result = f
        .backend
        .type_text("beta", Some("cua:10:20"), Some(&target))
        .await
        .unwrap();
    assert_eq!(result.window_id.as_deref(), Some("cua:10:20"));
    assert_eq!(
        texts(&f, "type_text", "text"),
        vec![json!("alpha"), json!("beta")]
    );
    assert_eq!(f.typing_max_in_flight(), 1);
}

#[tokio::test]
async fn cancels_a_queued_semantic_write_without_waiting_for_or_bypassing_its_predecessor() {
    let f = fixture_with(FixtureOptions {
        lane_gap_ms: Some(0),
        ..Default::default()
    });
    f.set_elements(vec![text_field("Message", 30, "message-token")]);
    let target = observe(&f, "cua:10:20", 0).await;
    let (opener, held) = gate();
    f.gate_type_text(Some(held));

    let first = f
        .backend
        .type_text("alpha", Some("cua:10:20"), Some(&target));
    let driver = async {
        wait_for(|| f.named("type_text").len() == 1).await;
        let token = CancellationToken::new();
        let second = with_desktop_operation_signal(
            OperationSignal::from(token.clone()),
            f.backend
                .type_text("cancelled", Some("cua:10:20"), Some(&target)),
        );
        let cancel = async {
            tokio::time::sleep(Duration::from_millis(10)).await;
            token.cancel();
        };
        let (second, ()) = tokio::join!(second, cancel);
        let error = second.unwrap_err();
        assert!(
            matches!(
                error,
                ComputerError::Operation(DesktopOperationError::Aborted)
            ),
            "{error}"
        );
        assert_eq!(f.named("type_text").len(), 1);
        f.gate_type_text(None);
        opener.open();
    };
    let (first, ()) = tokio::join!(first, driver);
    first.unwrap();
    f.backend
        .type_text("beta", Some("cua:10:20"), Some(&target))
        .await
        .unwrap();
    assert_eq!(
        texts(&f, "type_text", "text"),
        vec![json!("alpha"), json!("beta")]
    );
    assert_eq!(f.typing_max_in_flight(), 1);
}

#[tokio::test]
async fn semantic_text_lane_holds_the_gap_between_consecutive_writes() {
    let f = fixture_with(FixtureOptions {
        lane_gap_ms: Some(60),
        ..Default::default()
    });
    f.set_elements(vec![text_field("Message", 30, "message-token")]);
    let target = observe(&f, "cua:10:20", 0).await;
    let started = std::time::Instant::now();
    let (first, second) = tokio::join!(
        f.backend
            .type_text("alpha", Some("cua:10:20"), Some(&target)),
        f.backend
            .type_text("bravo", Some("cua:10:20"), Some(&target)),
    );
    first.unwrap();
    second.unwrap();
    assert_eq!(f.named("type_text").len(), 2);
    assert!(started.elapsed() >= Duration::from_millis(40));
}

#[tokio::test]
async fn keeps_retained_semantic_text_available_after_its_exact_window_moves_off_space() {
    let f = fixture();
    f.set_elements(vec![text_field("Message", 30, "message-token")]);
    let target = observe(&f, "cua:10:20", 0).await;
    f.set_visible(false);
    let result = f
        .backend
        .type_text("hello", Some("cua:10:20"), Some(&target))
        .await
        .unwrap();
    assert_eq!(result.window_id.as_deref(), Some("cua:10:20"));
    assert_subset(
        &f.last_args("type_text"),
        json!({
            "pid": 10,
            "window_id": 20,
            "text": "hello",
            "element_token": "message-token",
            "semantic_only": true,
        }),
    );
}

fn equals_button() -> Value {
    json!({
        "role": "AXButton",
        "label": "Equals",
        "frame": { "x": -290, "y": 30, "width": 20, "height": 20 },
        "element_token": "fresh-token",
    })
}

fn reuse_recent_tree(window_id: &str) -> ComputerStateOptions {
    ComputerStateOptions {
        reuse_recent_tree: true,
        ..window_state(window_id)
    }
}

#[tokio::test]
async fn serves_internal_target_resolution_from_a_recent_tree_instead_of_walking_again() {
    let f = fixture();
    f.set_elements(vec![equals_button()]);
    let observed = f
        .backend
        .get_state(window_state("cua:10:20"))
        .await
        .unwrap();
    assert_eq!(f.named("get_window_state").len(), 1);
    let resolved = f
        .backend
        .get_state(reuse_recent_tree("cua:10:20"))
        .await
        .unwrap();
    assert_eq!(f.named("get_window_state").len(), 1);
    assert_eq!(resolved.root, observed.root);
    f.backend
        .get_state(window_state("cua:10:20"))
        .await
        .unwrap();
    assert_eq!(f.named("get_window_state").len(), 2);
}

#[tokio::test(start_paused = true)]
async fn scopes_the_recent_tree_to_its_window_and_re_walks_after_it_ages_out() {
    let f = fixture();
    f.set_elements(vec![equals_button()]);
    f.backend
        .get_state(window_state("cua:10:20"))
        .await
        .unwrap();
    assert_eq!(f.named("get_window_state").len(), 1);
    let error = f
        .backend
        .get_state(reuse_recent_tree("cua:30:40"))
        .await
        .unwrap_err();
    assert_eq!(effect(&error), Some(CuaEffect::NotDispatched));
    assert_eq!(f.named("get_window_state").len(), 1);
    tokio::time::advance(Duration::from_secs(10)).await;
    f.backend
        .get_state(reuse_recent_tree("cua:10:20"))
        .await
        .unwrap();
    assert_eq!(f.named("get_window_state").len(), 2);
}
