use serde_json::json;
use zeron_proto::AccessMode;

use super::{THREAD, harness, result_json};
use crate::computer::approval_gate::ApprovalDecision;

#[tokio::test]
async fn an_intervening_explicit_screenshot_prevents_unrelated_image_reuse() {
    let h = harness();
    h.call(
        "computer_click",
        json!({ "label": "Calculate", "role": "button" }),
    )
    .await;
    let explicit = h
        .call(
            "computer_screenshot",
            json!({ "window_id": "fake-terminal" }),
        )
        .await;
    assert!(!explicit.is_error, "{explicit:?}");
    assert!(explicit.has_image());
    let result = h
        .call(
            "computer_click",
            json!({ "label": "Calculate", "role": "button" }),
        )
        .await;
    assert!(result.has_image(), "{result:?}");
    h.dispose().await;
}

#[tokio::test]
async fn routes_every_harness_through_the_same_task_gate() {
    let h = harness();
    *h.approvals.mode.lock().unwrap() = Some(AccessMode::Supervised);
    h.decide(ApprovalDecision::Accept);
    for (index, harness_id) in ["claude", "codex", "acp"].into_iter().enumerate() {
        let mut context = h.context_for(THREAD, None);
        context.session_key = format!("{harness_id}-session");
        context.turn_id = Some(format!("turn-{harness_id}"));
        let result = h
            .call_in(
                "computer_type_text",
                json!({
                    "text": format!("check-{harness_id}"),
                    "window_id": "fake-terminal",
                    "include_screenshot": false,
                }),
                &context,
            )
            .await;
        assert!(!result.is_error, "{harness_id}: {result:?}");
        let cards = h.approvals.cards.lock().unwrap().clone();
        assert_eq!(cards.len(), index + 1, "{harness_id}");
        assert_eq!(cards[index].tool_name, "computer_type_text");
        assert_eq!(
            cards[index].turn_id.as_deref(),
            Some(format!("turn-{harness_id}").as_str())
        );
    }
    assert_eq!(h.backend.calls_for("typeText").len(), 3);
    h.decide(ApprovalDecision::Decline);
    let mut context = h.context_for(THREAD, None);
    context.turn_id = Some("turn-denied".into());
    let denied = h
        .call_in(
            "computer_type_text",
            json!({ "text": "denied", "window_id": "fake-terminal", "include_screenshot": false }),
            &context,
        )
        .await;
    assert!(denied.is_error);
    assert_eq!(h.backend.calls_for("typeText").len(), 3);
    assert_eq!(result_json(&denied)["error"]["code"], "app_consent_denied");
    assert_eq!(
        result_json(&denied)["error"]["app"]["bundleId"],
        "org.kde.konsole"
    );
    h.dispose().await;
}
