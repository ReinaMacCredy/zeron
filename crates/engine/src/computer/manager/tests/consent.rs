use super::*;

use serde_json::Map;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

use super::super::{ComputerCaller, ComputerConsentScope};

const CHAT: &str = "chat-1";

fn caller(turn_id: Option<&str>) -> ComputerCaller {
    ComputerCaller {
        chat_id: CHAT.into(),
        turn_id: turn_id.map(str::to_owned),
    }
}

fn args(value: Value) -> Map<String, Value> {
    value.as_object().unwrap().clone()
}

fn with_mode(mode: Option<PermissionMode>) -> Built {
    let built = settled(Scripted::default_fake());
    *built.seams.mode.lock().unwrap() = mode;
    built
}

fn opened(seams: &Seams) -> Vec<ComputerConsentCard> {
    seams
        .consent
        .lock()
        .unwrap()
        .iter()
        .filter(|(_, decision)| decision.is_none())
        .map(|(card, _)| card.clone())
        .collect()
}

fn dismissal(seams: &Seams, request_id: &str) -> Option<ApprovalDecision> {
    seams
        .consent
        .lock()
        .unwrap()
        .iter()
        .find(|(card, decision)| card.request_id == request_id && decision.is_some())
        .and_then(|(_, decision)| *decision)
}

fn disclosures(seams: &Seams) -> Vec<(String, Option<String>)> {
    seams.disclosures.lock().unwrap().clone()
}

fn authorize(
    manager: &Arc<ComputerManager>,
    tool: &str,
    turn_id: Option<&str>,
) -> JoinHandle<Result<bool, ComputerError>> {
    let manager = manager.clone();
    let tool = tool.to_owned();
    let caller = caller(turn_id);
    tokio::spawn(async move {
        manager
            .authorize_action(
                &tool,
                &args(json!({ "x": 1, "y": 2 })),
                &caller,
                CancellationToken::new(),
            )
            .await
    })
}

async fn answer(
    manager: &Arc<ComputerManager>,
    seams: &Seams,
    pending: JoinHandle<Result<bool, ComputerError>>,
    decision: ApprovalDecision,
) -> Result<bool, ComputerError> {
    settle().await;
    let card = opened(seams).pop().expect("a consent card is open");
    assert!(
        manager
            .approval_gate()
            .respond(CHAT, &card.request_id, decision)
    );
    pending.await.unwrap()
}

#[tokio::test]
async fn ask_raises_exactly_one_task_card_for_the_first_gated_call_of_a_turn() {
    let Built { manager, seams, .. } = with_mode(Some(PermissionMode::Ask));
    let first = authorize(&manager, "computer_click", Some("turn-1"));
    settle().await;
    let cards = opened(&seams);
    assert_eq!(cards.len(), 1);
    assert_eq!(cards[0].scope, ComputerConsentScope::Task);
    assert_eq!(cards[0].chat_id, CHAT);
    assert_eq!(cards[0].turn_id.as_deref(), Some("turn-1"));
    assert_eq!(cards[0].tool_name, "computer_click");
    assert!(
        answer(&manager, &seams, first, ApprovalDecision::Accept)
            .await
            .unwrap()
    );
    let second = authorize(&manager, "computer_type_text", Some("turn-1"))
        .await
        .unwrap()
        .unwrap();
    assert!(second);
    assert_eq!(opened(&seams).len(), 1);
    manager.dispose().await;
}

#[tokio::test]
async fn a_decline_is_not_re_asked_within_the_turn() {
    let Built { manager, seams, .. } = with_mode(Some(PermissionMode::Ask));
    let first = authorize(&manager, "computer_click", Some("turn-1"));
    assert!(
        !answer(&manager, &seams, first, ApprovalDecision::Decline)
            .await
            .unwrap()
    );
    let again = authorize(&manager, "computer_click", Some("turn-1"))
        .await
        .unwrap()
        .unwrap();
    assert!(!again);
    assert_eq!(opened(&seams).len(), 1);
    manager.dispose().await;
}

#[tokio::test]
async fn a_new_turn_asks_again() {
    let Built { manager, seams, .. } = with_mode(Some(PermissionMode::Ask));
    let first = authorize(&manager, "computer_click", Some("turn-1"));
    assert!(
        answer(&manager, &seams, first, ApprovalDecision::Accept)
            .await
            .unwrap()
    );
    let next = authorize(&manager, "computer_click", Some("turn-2"));
    settle().await;
    let cards = opened(&seams);
    assert_eq!(cards.len(), 2);
    assert_eq!(cards[1].turn_id.as_deref(), Some("turn-2"));
    assert_eq!(cards[1].scope, ComputerConsentScope::Task);
    assert!(
        answer(&manager, &seams, next, ApprovalDecision::Accept)
            .await
            .unwrap()
    );
    manager.dispose().await;
}

#[tokio::test]
async fn read_clipboard_raises_its_own_per_call_card_every_time_in_ask() {
    let Built { manager, seams, .. } = with_mode(Some(PermissionMode::Ask));
    let task = authorize(&manager, "computer_click", Some("turn-1"));
    assert!(
        answer(&manager, &seams, task, ApprovalDecision::Accept)
            .await
            .unwrap()
    );
    for expected in 2..=3 {
        let read = authorize(&manager, "computer_read_clipboard", Some("turn-1"));
        settle().await;
        let cards = opened(&seams);
        assert_eq!(cards.len(), expected);
        assert_eq!(cards[expected - 1].scope, ComputerConsentScope::Call);
        assert_eq!(cards[expected - 1].tool_name, "computer_read_clipboard");
        assert!(
            answer(&manager, &seams, read, ApprovalDecision::Accept)
                .await
                .unwrap()
        );
    }
    manager.dispose().await;
}

#[tokio::test]
async fn auto_and_full_raise_no_card_and_return_true() {
    for mode in [PermissionMode::Auto, PermissionMode::Full] {
        let Built { manager, seams, .. } = with_mode(Some(mode));
        for tool in [
            "computer_click",
            "computer_read_clipboard",
            "computer_click",
        ] {
            let approved = authorize(&manager, tool, Some("turn-1"))
                .await
                .unwrap()
                .unwrap();
            assert!(approved, "{mode:?} {tool}");
        }
        settle().await;
        assert!(seams.consent.lock().unwrap().is_empty(), "{mode:?}");
        manager.dispose().await;
    }
}

#[tokio::test]
async fn the_control_disclosure_posts_once_per_chat_and_turn_after_an_ask_approval() {
    let Built { manager, seams, .. } = with_mode(Some(PermissionMode::Ask));
    let first = authorize(&manager, "computer_click", Some("turn-1"));
    assert!(disclosures(&seams).is_empty());
    assert!(
        answer(&manager, &seams, first, ApprovalDecision::Accept)
            .await
            .unwrap()
    );
    assert_eq!(
        disclosures(&seams),
        [(CHAT.to_owned(), Some("turn-1".to_owned()))]
    );
    authorize(&manager, "computer_click", Some("turn-1"))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(disclosures(&seams).len(), 1);
    let next = authorize(&manager, "computer_click", Some("turn-2"));
    assert!(
        answer(&manager, &seams, next, ApprovalDecision::Accept)
            .await
            .unwrap()
    );
    assert_eq!(
        disclosures(&seams),
        [
            (CHAT.to_owned(), Some("turn-1".to_owned())),
            (CHAT.to_owned(), Some("turn-2".to_owned())),
        ]
    );
    manager.dispose().await;
}

#[tokio::test]
async fn the_control_disclosure_posts_once_per_chat_and_turn_on_auto_and_full_accept() {
    for mode in [PermissionMode::Auto, PermissionMode::Full] {
        let Built { manager, seams, .. } = with_mode(Some(mode));
        for turn in ["turn-1", "turn-1", "turn-2", "turn-2"] {
            authorize(&manager, "computer_click", Some(turn))
                .await
                .unwrap()
                .unwrap();
        }
        assert_eq!(
            disclosures(&seams),
            [
                (CHAT.to_owned(), Some("turn-1".to_owned())),
                (CHAT.to_owned(), Some("turn-2".to_owned())),
            ],
            "{mode:?}"
        );
        manager.dispose().await;
    }
}

#[tokio::test]
async fn the_control_disclosure_never_posts_after_a_decline() {
    let Built { manager, seams, .. } = with_mode(Some(PermissionMode::Ask));
    let task = authorize(&manager, "computer_click", Some("turn-1"));
    assert!(
        !answer(&manager, &seams, task, ApprovalDecision::Decline)
            .await
            .unwrap()
    );
    let read = authorize(&manager, "computer_read_clipboard", Some("turn-2"));
    assert!(
        !answer(&manager, &seams, read, ApprovalDecision::Decline)
            .await
            .unwrap()
    );
    assert!(disclosures(&seams).is_empty());
    manager.dispose().await;
}

#[tokio::test]
async fn params_display_omits_text_value_and_prompt_text() {
    let Built { manager, seams, .. } = with_mode(Some(PermissionMode::Ask));
    let pending = tokio::spawn({
        let manager = manager.clone();
        async move {
            manager
                .authorize_action(
                    "computer_type_text",
                    &args(json!({
                        "text": "hunter2",
                        "value": "secret value",
                        "prompt_text": "secret prompt",
                        "window_id": "fake-calculator",
                        "x": 1,
                    })),
                    &caller(Some("turn-1")),
                    CancellationToken::new(),
                )
                .await
        }
    });
    settle().await;
    let card = opened(&seams).pop().unwrap();
    let display: Value = serde_json::from_str(&card.params_display).unwrap();
    assert_eq!(display, json!({ "window_id": "fake-calculator", "x": 1 }));
    for secret in ["hunter2", "secret value", "secret prompt"] {
        assert!(!card.params_display.contains(secret), "{secret}");
    }
    assert!(
        manager
            .approval_gate()
            .respond(CHAT, &card.request_id, ApprovalDecision::Accept)
    );
    assert!(pending.await.unwrap().unwrap());
    manager.dispose().await;
}

#[tokio::test]
async fn the_ninth_pending_clipboard_card_in_one_chat_fails_with_approval_queue_full() {
    let Built { manager, seams, .. } = with_mode(Some(PermissionMode::Ask));
    let waiting: Vec<_> = (0..8)
        .map(|_| authorize(&manager, "computer_read_clipboard", Some("turn-1")))
        .collect();
    settle().await;
    assert_eq!(opened(&seams).len(), 8);
    let error = authorize(&manager, "computer_read_clipboard", Some("turn-1"))
        .await
        .unwrap()
        .unwrap_err();
    assert_eq!(code(&error), Some("approval_queue_full"));
    assert_eq!(opened(&seams).len(), 8);
    manager.stop_thread(CHAT).await.unwrap();
    for waiter in waiting {
        assert!(!waiter.await.unwrap().unwrap());
    }
    manager.dispose().await;
}

#[tokio::test]
async fn a_chat_that_is_gone_is_declined_without_a_card() {
    let Built { manager, seams, .. } = with_mode(None);
    for tool in ["computer_click", "computer_read_clipboard"] {
        let approved = authorize(&manager, tool, Some("turn-1"))
            .await
            .unwrap()
            .unwrap();
        assert!(!approved, "{tool}");
    }
    settle().await;
    assert!(seams.consent.lock().unwrap().is_empty());
    assert!(disclosures(&seams).is_empty());
    manager.dispose().await;
}

#[tokio::test]
async fn stop_thread_and_thread_removal_settle_an_open_card_as_declined() {
    for route in ["stop_thread", "handle_thread_removed"] {
        for tool in ["computer_click", "computer_read_clipboard"] {
            let Built { manager, seams, .. } = with_mode(Some(PermissionMode::Ask));
            let pending = authorize(&manager, tool, Some("turn-1"));
            settle().await;
            let card = opened(&seams).pop().unwrap();
            if route == "stop_thread" {
                manager.stop_thread(CHAT).await.unwrap();
            } else {
                manager.handle_thread_removed(CHAT).await;
            }
            assert!(!pending.await.unwrap().unwrap(), "{route} {tool}");
            settle().await;
            assert_eq!(
                dismissal(&seams, &card.request_id),
                Some(ApprovalDecision::Cancel),
                "{route} {tool}"
            );
            assert!(
                !manager
                    .approval_gate()
                    .respond(CHAT, &card.request_id, ApprovalDecision::Accept),
                "{route} {tool}"
            );
            assert!(disclosures(&seams).is_empty(), "{route} {tool}");
            manager.dispose().await;
        }
    }
}

#[tokio::test]
async fn a_desktop_interrupted_event_revokes_the_task_grant_so_the_next_gated_call_asks_again() {
    let Built {
        manager,
        backend,
        seams,
    } = with_mode(Some(PermissionMode::Ask));
    let first = authorize(&manager, "computer_click", Some("turn-1"));
    assert!(
        answer(&manager, &seams, first, ApprovalDecision::Accept)
            .await
            .unwrap()
    );
    assert!(
        authorize(&manager, "computer_click", Some("turn-1"))
            .await
            .unwrap()
            .unwrap()
    );
    assert_eq!(opened(&seams).len(), 1);
    backend.fake.emit_desktop_interrupted(&[]);
    let next = authorize(&manager, "computer_click", Some("turn-1"));
    settle().await;
    let cards = opened(&seams);
    assert_eq!(cards.len(), 2);
    assert_eq!(cards[1].scope, ComputerConsentScope::Task);
    assert!(
        answer(&manager, &seams, next, ApprovalDecision::Accept)
            .await
            .unwrap()
    );
    assert_eq!(disclosures(&seams).len(), 1);
    manager.dispose().await;
}
