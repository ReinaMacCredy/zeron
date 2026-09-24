use std::sync::Arc;

use serde_json::{Value, json};
use zeron_proto::PermissionMode;

use super::{HarnessOptions, THREAD, fake_with, harness, harness_with, result_json};
use crate::computer::approval_gate::ApprovalDecision;
use crate::computer::engine_seams::{consent_decision, consent_question};
use crate::computer::fake_backend::FakeComputerBackendOptions;
use crate::computer::manager::ComputerConsentCard;
use crate::computer::tools::consent::COMPUTER_APP_CONSENT_DENIED_CODE;
use crate::computer::tools::tests::Harness;
use zeron_proto::UserInputAnswer;

fn cards(h: &Harness) -> Vec<ComputerConsentCard> {
    h.approvals.cards.lock().unwrap().clone()
}

fn bundle(card: &ComputerConsentCard) -> Option<&str> {
    card.app.as_ref().and_then(|app| app.bundle_id.as_deref())
}

fn decide_by_app(
    h: &Harness,
    decide: impl Fn(Option<&str>) -> ApprovalDecision + Send + Sync + 'static,
) {
    *h.approvals.mode.lock().unwrap() = Some(PermissionMode::Ask);
    *h.approvals.decide.lock().unwrap() = Some(Arc::new(move |card: &ComputerConsentCard| {
        decide(bundle(card))
    }));
}

fn type_into(window_id: &str) -> Value {
    json!({ "text": "hi", "window_id": window_id, "include_screenshot": false })
}

async fn type_in_turn(
    h: &Harness,
    turn: &str,
    window_id: &str,
) -> zeron_proto::computer::McpToolResult {
    let mut context = h.context_for(THREAD, None);
    context.turn_id = Some(turn.to_owned());
    h.call_in("computer_type_text", type_into(window_id), &context)
        .await
}

#[tokio::test]
async fn approving_one_app_raises_a_new_card_naming_the_next_app_in_the_same_turn() {
    let h = harness();
    h.decide(ApprovalDecision::Accept);
    let first = h
        .call("computer_type_text", type_into("fake-terminal"))
        .await;
    assert!(!first.is_error, "{first:?}");
    let second = h
        .call("computer_type_text", type_into("fake-calculator"))
        .await;
    assert!(!second.is_error, "{second:?}");
    let again = h
        .call("computer_type_text", type_into("fake-terminal"))
        .await;
    assert!(!again.is_error, "{again:?}");
    let opened = cards(&h);
    assert_eq!(
        opened.iter().map(bundle).collect::<Vec<_>>(),
        [Some("org.kde.konsole"), Some("org.kde.kcalc")],
        "the first app keeps its grant; the second app asks once"
    );
    let calculator = opened[1].app.as_ref().unwrap();
    assert_eq!(calculator.name, "Calculator");
    assert_eq!(calculator.pid, Some(1_002));
    let question = consent_question(&opened[1]);
    assert!(
        question
            .question
            .starts_with("Allow Computer to use Calculator (org.kde.kcalc)?"),
        "{}",
        question.question
    );
    assert_eq!(
        question.options,
        ["Allow", "Always allow Calculator", "Deny"]
    );
    assert_eq!(h.backend.calls_for("typeText").len(), 3);
    let list = h.call("computer_list_windows", json!({})).await;
    assert!(!list.is_error);
    assert_eq!(cards(&h).len(), 2, "app-less reads never ask");
    h.dispose().await;
}

#[tokio::test]
async fn always_allow_skips_the_card_in_later_turns_until_the_app_is_removed() {
    let h = harness();
    h.decide(ApprovalDecision::AcceptForSession);
    assert!(!type_in_turn(&h, "turn-1", "fake-terminal").await.is_error);
    assert_eq!(cards(&h).len(), 1);
    assert!(!type_in_turn(&h, "turn-2", "fake-terminal").await.is_error);
    assert_eq!(cards(&h).len(), 1, "an always-allowed app raises no card");
    let mut context = h.context_for(THREAD, None);
    context.turn_id = Some("turn-2".into());
    let clipboard_free = h
        .call_in("computer_write_clipboard", json!({ "text": "x" }), &context)
        .await;
    assert!(!clipboard_free.is_error, "{clipboard_free:?}");
    assert_eq!(
        cards(&h).len(),
        1,
        "the always-allowed call granted the turn for app-less tools"
    );
    h.approvals.gate.set_always_allowed([]);
    assert!(!type_in_turn(&h, "turn-3", "fake-terminal").await.is_error);
    assert_eq!(cards(&h).len(), 2, "removing the app brings the card back");
    h.dispose().await;
}

#[tokio::test]
async fn an_unverified_app_says_so_and_cannot_be_always_allowed() {
    let h = harness_with(HarnessOptions {
        backend: fake_with(FakeComputerBackendOptions {
            apps: Some(Vec::new()),
            ..FakeComputerBackendOptions::default()
        }),
        ..HarnessOptions::default()
    });
    h.decide(ApprovalDecision::AcceptForSession);
    let denied = h
        .call("computer_type_text", type_into("fake-terminal"))
        .await;
    assert!(denied.is_error, "{denied:?}");
    let card = cards(&h).pop().expect("a card");
    let app = card.app.clone().expect("the card names the app");
    assert_eq!(app.bundle_id, None);
    assert_eq!(app.pid, Some(1_001));
    assert!(!app.verified());
    let question = consent_question(&card);
    assert!(
        question.question.contains("unverified") && question.question.contains("could not verify"),
        "{}",
        question.question
    );
    assert_eq!(question.options, ["Allow", "Deny"]);
    let error = &result_json(&denied)["error"];
    assert_eq!(error["code"], COMPUTER_APP_CONSENT_DENIED_CODE);
    assert_eq!(error["app"]["verified"], false);
    assert_eq!(h.backend.calls_for("typeText").len(), 0);
    let lookups = h.backend.calls_for("listApps").len();
    assert!(
        h.call("computer_type_text", type_into("fake-terminal"))
            .await
            .is_error
    );
    assert_eq!(cards(&h).len(), 1, "the denial sticks for the turn");
    assert_eq!(
        h.backend.calls_for("listApps").len(),
        lookups,
        "an unknown pid is not looked up again on every call"
    );
    h.decide(ApprovalDecision::Accept);
    let mut context = h.context_for(THREAD, None);
    context.turn_id = Some("turn-next".into());
    assert!(
        !h.call_in("computer_type_text", type_into("fake-terminal"), &context)
            .await
            .is_error
    );
    assert_eq!(
        cards(&h).len(),
        2,
        "nothing was saved for the unverified app"
    );
    h.dispose().await;
}

#[tokio::test]
async fn a_denied_app_refuses_identically_through_a_direct_action_and_computer_run() {
    let h = harness();
    decide_by_app(&h, |bundle| match bundle {
        Some("org.kde.konsole") => ApprovalDecision::Accept,
        _ => ApprovalDecision::Decline,
    });
    let direct = type_in_turn(&h, "turn-direct", "fake-calculator").await;
    assert!(direct.is_error);
    let direct = result_json(&direct);
    let mut context = h.context_for(THREAD, None);
    context.turn_id = Some("turn-run".into());
    let run = h
        .call_in(
            "computer_run",
            json!({
                "steps": [
                    { "type": "type_text", "text": "a", "window_id": "fake-terminal" },
                    { "type": "type_text", "text": "b", "window_id": "fake-calculator" },
                ],
            }),
            &context,
        )
        .await;
    assert!(run.is_error, "{run:?}");
    let run = result_json(&run);
    assert_eq!(run["error"], direct["error"]);
    assert_eq!(run["step"], 1);
    assert_eq!(direct["error"]["code"], COMPUTER_APP_CONSENT_DENIED_CODE);
    assert_eq!(direct["error"]["effect"], "not-dispatched");
    assert_eq!(direct["error"]["app"]["bundleId"], "org.kde.kcalc");
    assert_eq!(direct["error"]["app"]["name"], "Calculator");
    assert_eq!(direct["error"]["app"]["verified"], true);
    assert_eq!(
        h.backend.calls_for("typeText").len(),
        0,
        "a run with one denied app dispatches none of its steps"
    );
    h.dispose().await;
}

#[tokio::test]
async fn full_mode_raises_no_card_for_any_app() {
    let h = harness();
    for window in ["fake-terminal", "fake-calculator"] {
        let result = h.call("computer_type_text", type_into(window)).await;
        assert!(!result.is_error, "{result:?}");
    }
    assert!(cards(&h).is_empty());
    assert_eq!(h.backend.calls_for("typeText").len(), 2);
    h.dispose().await;
}

#[test]
fn the_always_label_maps_to_an_always_decision() {
    let answer = |label: &str| {
        vec![UserInputAnswer {
            question_id: "consent".into(),
            labels: vec![label.into()],
        }]
    };
    assert_eq!(consent_decision(&answer("Allow")), ApprovalDecision::Accept);
    assert_eq!(
        consent_decision(&answer("Always allow Safari")),
        ApprovalDecision::AcceptForSession
    );
    for denied in ["Deny", "Decline"] {
        assert_eq!(consent_decision(&answer(denied)), ApprovalDecision::Decline);
    }
}
