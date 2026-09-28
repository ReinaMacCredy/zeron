use std::sync::Arc;

use serde_json::{Value, json};
use zeron_proto::PermissionMode;

use super::{HarnessOptions, THREAD, fake_with, harness, harness_with, result_json};
use crate::computer::approval_gate::ApprovalDecision;
use crate::computer::engine_seams::{consent_decision, consent_question};
use crate::computer::fake_backend::FakeComputerBackendOptions;
use crate::computer::manager::ComputerConsentCard;
use crate::computer::tools::consent::{
    COMPUTER_APP_CONSENT_DENIED_CODE, COMPUTER_APP_WINDOW_REQUIRED_CODE,
};
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
            .starts_with("Allow Computer to use Calculator?"),
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
        question
            .question
            .contains("could not confirm which app this is")
            && !question.question.contains("1001"),
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

struct DisplayFrame {
    id: Value,
    region: Value,
    scale: f64,
}

impl DisplayFrame {
    fn at(&self, x: f64, y: f64) -> Value {
        let pixel = |desktop: f64, origin: &str| {
            ((desktop - self.region[origin].as_f64().unwrap()) * self.scale).round()
        };
        json!({ "x": pixel(x, "x"), "y": pixel(y, "y"), "screenshot_id": self.id })
    }
}

async fn display_frame(h: &Harness) -> DisplayFrame {
    h.see().await;
    let shot = h
        .call(
            "computer_screenshot",
            json!({ "x": 0, "y": 0, "width": 1920, "height": 1080 }),
        )
        .await;
    assert!(!shot.is_error, "{shot:?}");
    let shot = result_json(&shot)["screenshot"].clone();
    assert!(shot.get("windowId").is_none(), "{shot}");
    DisplayFrame {
        id: shot["screenshotId"].clone(),
        region: shot["region"].clone(),
        scale: shot["scale"].as_f64().unwrap(),
    }
}

fn grant_terminal_deny_calculator(h: &Harness) {
    decide_by_app(h, |bundle| match bundle {
        Some("org.kde.konsole") => ApprovalDecision::Accept,
        _ => ApprovalDecision::Decline,
    });
}

#[tokio::test]
async fn a_display_coordinate_click_asks_for_the_app_under_the_point() {
    let h = harness();
    grant_terminal_deny_calculator(&h);
    assert!(
        !h.call("computer_type_text", type_into("fake-terminal"))
            .await
            .is_error
    );
    let display = display_frame(&h).await;
    let at = |x: f64, y: f64| display.at(x, y);
    let on_terminal = h.call("computer_click", at(200.0, 300.0)).await;
    assert!(!on_terminal.is_error, "{on_terminal:?}");
    assert_eq!(h.backend.calls_for("click").len(), 1);
    let on_calculator = h.call("computer_click", at(1200.0, 400.0)).await;
    assert!(on_calculator.is_error, "{on_calculator:?}");
    let error = &result_json(&on_calculator)["error"];
    assert_eq!(error["code"], COMPUTER_APP_CONSENT_DENIED_CODE);
    assert_eq!(error["app"]["bundleId"], "org.kde.kcalc");
    assert_eq!(
        cards(&h).iter().map(bundle).collect::<Vec<_>>(),
        [Some("org.kde.konsole"), Some("org.kde.kcalc")],
        "the click under the point raised a card naming its app"
    );
    assert_eq!(
        h.backend.calls_for("click").len(),
        1,
        "the denied click posted nothing"
    );
    let drag = h
        .call(
            "computer_drag",
            json!({ "from": at(200.0, 300.0), "to": at(1200.0, 400.0) }),
        )
        .await;
    assert!(drag.is_error, "{drag:?}");
    assert_eq!(
        result_json(&drag)["error"]["app"]["bundleId"],
        "org.kde.kcalc"
    );
    assert!(h.backend.calls_for("drag").is_empty());
    h.dispose().await;
}

#[tokio::test]
async fn a_display_coordinate_run_step_asks_for_the_app_under_the_point() {
    let h = harness();
    grant_terminal_deny_calculator(&h);
    let display = display_frame(&h).await;
    let step = |x: f64, y: f64| {
        let mut step = display.at(x, y);
        step["type"] = json!("click");
        step
    };
    let run = h
        .call(
            "computer_run",
            json!({ "steps": [step(200.0, 300.0), step(1200.0, 400.0)] }),
        )
        .await;
    assert!(run.is_error, "{run:?}");
    let run = result_json(&run);
    assert_eq!(run["error"]["code"], COMPUTER_APP_CONSENT_DENIED_CODE);
    assert_eq!(run["error"]["app"]["bundleId"], "org.kde.kcalc");
    assert_eq!(run["step"], 1);
    assert!(h.backend.calls_for("click").is_empty());
    h.dispose().await;
}

#[tokio::test]
async fn window_less_keyboard_input_is_refused_in_ask_mode_and_unchanged_in_full() {
    let h = harness();
    for (tool, arguments) in [
        ("computer_press_key", json!({ "key": "Return" })),
        ("computer_type_text", json!({ "text": "echo hi" })),
        ("computer_paste", json!({ "text": "x" })),
    ] {
        let full = h.call(tool, arguments.clone()).await;
        assert!(!full.is_error, "{tool} in Full: {full:?}");
    }
    let dispatched = h.backend.methods().len();
    h.decide(ApprovalDecision::Accept);
    assert!(
        !h.call("computer_type_text", type_into("fake-terminal"))
            .await
            .is_error
    );
    let granted_only = h.backend.methods().len();
    for (tool, arguments) in [
        ("computer_press_key", json!({ "key": "Return" })),
        ("computer_type_text", json!({ "text": "echo hi" })),
        ("computer_paste", json!({ "text": "x" })),
        (
            "computer_click",
            json!({ "label": "Calculate", "role": "button" }),
        ),
    ] {
        let refused = h.call(tool, arguments).await;
        assert!(refused.is_error, "{tool}: {refused:?}");
        assert_eq!(
            result_json(&refused)["error"]["code"],
            COMPUTER_APP_WINDOW_REQUIRED_CODE,
            "{tool}"
        );
        assert_eq!(result_json(&refused)["error"]["effect"], "not-dispatched");
    }
    assert!(granted_only > dispatched);
    assert_eq!(
        h.backend
            .methods()
            .into_iter()
            .skip(granted_only)
            .filter(|method| matches!(
                method.as_str(),
                "pressKey" | "typeText" | "hotkey" | "click" | "writeClipboard"
            ))
            .count(),
        0,
        "no keyboard, click or clipboard input left for an unnamed app"
    );
    assert_eq!(cards(&h).len(), 1, "an unnamed app never raises a card");
    h.dispose().await;
}

#[tokio::test]
async fn dispatch_refuses_window_less_input_that_reaches_an_ungranted_app() {
    use crate::computer::manager::{
        COMPUTER_APP_CONSENT_REQUIRED_CODE, COMPUTER_APP_WINDOW_REQUIRED_CODE, ComputerClickGesture,
    };
    use crate::computer::task_context::with_computer_task;
    use zeron_proto::computer::{ComputerTarget, CuaComputerTask};

    let h = harness();
    grant_terminal_deny_calculator(&h);
    assert!(
        !h.call("computer_type_text", type_into("fake-terminal"))
            .await
            .is_error
    );
    let task = CuaComputerTask {
        thread_id: THREAD.into(),
        turn_id: Some(super::TURN.into()),
        label: None,
    };
    let click_at = |x: f64, y: f64| {
        let manager = h.manager.clone();
        let task = task.clone();
        async move {
            with_computer_task(
                task,
                manager.click(
                    Some(THREAD),
                    ComputerTarget {
                        x: Some(x),
                        y: Some(y),
                        ..ComputerTarget::default()
                    },
                    &[],
                    ComputerClickGesture::default(),
                ),
            )
            .await
        }
    };
    assert!(
        click_at(200.0, 300.0).await.is_ok(),
        "the granted app takes the click"
    );
    let error = click_at(1200.0, 400.0).await.unwrap_err();
    assert_eq!(error.code(), Some(COMPUTER_APP_CONSENT_REQUIRED_CODE));
    assert!(error.to_string().contains("org.kde.kcalc"), "{error}");
    assert_eq!(h.backend.calls_for("click").len(), 1);
    let unnamed = with_computer_task(
        task.clone(),
        h.manager.press_key(Some(THREAD), "Return", None, None),
    )
    .await
    .unwrap_err();
    assert_eq!(unnamed.code(), Some(COMPUTER_APP_WINDOW_REQUIRED_CODE));
    assert!(h.backend.calls_for("pressKey").is_empty());
    h.dispose().await;
}
