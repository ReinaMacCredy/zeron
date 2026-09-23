use serde_json::Map;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

use super::super::ComputerCaller;
use super::*;

fn caller(chat_id: &str, turn_id: &str) -> ComputerCaller {
    ComputerCaller {
        chat_id: chat_id.into(),
        turn_id: Some(turn_id.into()),
    }
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

async fn type_if_authorized(
    manager: &ComputerManager,
    caller: &ComputerCaller,
    signal: CancellationToken,
    text: &str,
) -> Result<(), ComputerError> {
    let approved = manager
        .authorize_action("computer_type_text", &Map::new(), caller, signal)
        .await?;
    if !approved {
        return Err(ComputerError::Other("The user declined.".into()));
    }
    manager
        .with_agent_activity(
            &caller.chat_id,
            || manager.type_text(Some(&caller.chat_id), text, None),
            None,
            caller.turn_id.as_deref(),
            None,
        )
        .await
        .map(|_| ())
}

fn spawn_type(
    manager: &Arc<ComputerManager>,
    caller: ComputerCaller,
    text: &'static str,
) -> JoinHandle<Result<(), ComputerError>> {
    let manager = manager.clone();
    tokio::spawn(async move {
        type_if_authorized(&manager, &caller, CancellationToken::new(), text).await
    })
}

fn ask() -> Built {
    let built = settled(Scripted::default_fake());
    *built.seams.mode.lock().unwrap() = Some(PermissionMode::Ask);
    built
}

#[tokio::test]
async fn an_accept_for_the_previous_turn_settles_false_and_dispatches_nothing() {
    let Built {
        manager,
        backend,
        seams,
    } = ask();
    let thread_id = "approval-lease-thread";
    let first = spawn_type(&manager, caller(thread_id, "turn-1"), "late");
    settle().await;
    let previous = opened(&seams).pop().expect("turn one's card is open");
    let next = {
        let manager = manager.clone();
        tokio::spawn(async move {
            manager
                .authorize_action(
                    "computer_click",
                    &Map::new(),
                    &caller(thread_id, "turn-2"),
                    CancellationToken::new(),
                )
                .await
        })
    };
    settle().await;
    assert!(!manager.approval_gate().respond(
        thread_id,
        &previous.request_id,
        ApprovalDecision::Accept
    ));
    let current = opened(&seams)
        .into_iter()
        .find(|card| card.turn_id.as_deref() == Some("turn-2"))
        .expect("turn two's card is open");
    assert!(manager.approval_gate().respond(
        thread_id,
        &current.request_id,
        ApprovalDecision::Accept
    ));
    assert!(next.await.unwrap().unwrap());
    assert!(first.await.unwrap().is_err());
    assert!(backend.calls_for("typeText").is_empty());
    manager.dispose().await;
}

#[tokio::test]
async fn an_approval_whose_signal_ended_before_the_user_answered_dispatches_nothing() {
    let Built {
        manager,
        backend,
        seams,
    } = ask();
    let thread_id = "approval-expiry-thread";
    let expired = CancellationToken::new();
    expired.cancel();
    let error = type_if_authorized(&manager, &caller(thread_id, "turn-1"), expired, "expired")
        .await
        .unwrap_err();
    assert!(matches!(error, ComputerError::Approval(_)), "{error:?}");
    assert!(seams.consent.lock().unwrap().is_empty());
    assert!(backend.calls_for("typeText").is_empty());
    let live = spawn_type(&manager, caller(thread_id, "turn-1"), "live");
    settle().await;
    let card = opened(&seams).pop().expect("a fresh card opens");
    assert!(
        manager
            .approval_gate()
            .respond(thread_id, &card.request_id, ApprovalDecision::Accept)
    );
    live.await.unwrap().unwrap();
    assert_eq!(backend.calls_for("typeText").len(), 1);
    manager.dispose().await;
}

#[tokio::test]
async fn a_turn_that_ended_before_dispatch_approves_nothing_and_dispatches_nothing() {
    let Built {
        manager,
        backend,
        seams,
    } = settled(Scripted::default_fake());
    *seams.mode.lock().unwrap() = Some(PermissionMode::Full);
    let thread_id = "approval-pause-thread";
    seams.turn_inactive.store(true, Ordering::SeqCst);
    let error = type_if_authorized(
        &manager,
        &caller(thread_id, "turn-1"),
        CancellationToken::new(),
        "late",
    )
    .await
    .unwrap_err();
    assert!(error.to_string().contains("original turn ended"), "{error}");
    assert!(backend.calls_for("typeText").is_empty());
    seams.turn_inactive.store(false, Ordering::SeqCst);
    type_if_authorized(
        &manager,
        &caller(thread_id, "turn-1"),
        CancellationToken::new(),
        "live",
    )
    .await
    .unwrap();
    assert_eq!(backend.calls_for("typeText").len(), 1);
    manager.dispose().await;
}

#[tokio::test]
async fn a_desktop_interruption_revokes_standing_consent_before_the_next_mutating_call() {
    let Built {
        manager,
        backend,
        seams,
    } = ask();
    let thread_id = "interruption-consent-thread";
    let first = spawn_type(&manager, caller(thread_id, "turn-1"), "before");
    settle().await;
    let card = opened(&seams).pop().expect("the first card opens");
    assert!(
        manager
            .approval_gate()
            .respond(thread_id, &card.request_id, ApprovalDecision::Accept)
    );
    first.await.unwrap().unwrap();
    assert_eq!(backend.calls_for("typeText").len(), 1);
    backend.fake.emit_desktop_interrupted(&["screen-lock"]);
    let second = spawn_type(&manager, caller(thread_id, "turn-1"), "after");
    settle().await;
    let prompts = opened(&seams);
    assert_eq!(prompts.len(), 2);
    let card = prompts.last().unwrap();
    assert!(
        manager
            .approval_gate()
            .respond(thread_id, &card.request_id, ApprovalDecision::Accept)
    );
    second.await.unwrap().unwrap();
    assert_eq!(backend.calls_for("typeText").len(), 2);
    manager.approval_gate().cancel_thread(thread_id, None);
    manager.dispose().await;
}
