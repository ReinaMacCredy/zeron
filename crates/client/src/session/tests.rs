use std::sync::Arc;
use std::time::{Duration, Instant};

use zeron_doc::{MessagePart, MessageRole, SessionCommandPayload, SessionMessageEntry};
use zeron_proto::{UserInputKind, UserInputQuestion};

use super::{Entry, SessionHandle, open_input};
use crate::config::{ClientConfig, Credentials, DemoOptions, StreamSpeed};
use crate::events::NullListener;
use crate::{Client, SendRequest};

fn demo(computer_still: bool) -> (Client, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let mut config = ClientConfig::new("https://edge.invalid", dir.path());
    config.device_id = "ios-test".into();
    config.device_name = "Test iPhone".into();
    let options = DemoOptions {
        stream_speed: StreamSpeed::Fast,
        computer_still,
        ..Default::default()
    };
    let client = Client::new(config, Credentials::Demo(options), Arc::new(NullListener)).unwrap();
    (client, dir)
}

fn wait_for(what: &str, mut f: impl FnMut() -> bool) {
    let start = Instant::now();
    while !f() {
        assert!(
            start.elapsed() < Duration::from_secs(10),
            "timed out waiting for {what}"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn question(id: &str, consent: bool) -> UserInputQuestion {
    UserInputQuestion {
        approval: false,
        id: id.into(),
        header: "H".into(),
        question: "Q".into(),
        options: vec!["a".into(), "b".into()],
        multi_select: false,
        kind: consent.then_some(UserInputKind::Consent {
            scope: zeron_proto::computer::ComputerConsentScope::Task,
            app: None,
            tool: None,
            params: None,
        }),
        prefill: None,
        multiline: false,
    }
}

fn input(request_id: &str, consent: bool) -> MessagePart {
    MessagePart::Input {
        id: request_id.into(),
        request_id: request_id.into(),
        questions: vec![question("q", consent)],
        resolved: false,
        answers: None,
        asked_at_ms: None,
        answered_at_ms: None,
    }
}

fn entry(parts: Vec<MessagePart>) -> Arc<Entry> {
    Arc::new(Entry {
        id: "m1".into(),
        rev: 1,
        message: Arc::new(SessionMessageEntry {
            id: "m1".into(),
            role: MessageRole::Assistant,
            parts,
            created_at: 1,
            device_id: "mac".into(),
            status: None,
            continuation_of: None,
            duration_ms: None,
            from_chat_id: None,
            fork_import: false,
            answers_request_id: None,
        }),
        echo: None,
        append: None,
    })
}

#[test]
fn a_consent_question_never_takes_the_composer() {
    assert_eq!(open_input(&[entry(vec![input("consent", true)])]), None);
    let both = open_input(&[entry(vec![input("plain", false), input("consent", true)])]);
    assert_eq!(both.map(|i| i.request_id), Some("plain".to_owned()));
}

fn generation(session: &SessionHandle, kind: &str) -> Option<Option<u64>> {
    session
        .core
        .doc()
        .read_commands()
        .unwrap()
        .into_iter()
        .find(|command| {
            matches!(
                (&command.payload, kind),
                (SessionCommandPayload::Run { .. }, "run")
                    | (SessionCommandPayload::Steer { .. }, "steer")
                    | (SessionCommandPayload::RespondInput { .. }, "respondInput")
                    | (SessionCommandPayload::Interrupt { .. }, "interrupt")
            )
        })
        .map(|command| command.computer_generation)
}

#[test]
fn run_steer_and_queue_rows_carry_the_chats_computer_generation() {
    // chat-blog's host is offline in the demo, so nothing consumes the ledger.
    let (client, _dir) = demo(false);
    let session = client.open_session("chat-blog").unwrap();
    session.core.doc().set_computer_generation(3).unwrap();
    session
        .send(SendRequest::text("/computer-use open Calculator"))
        .unwrap();
    session
        .core
        .queue_command(
            "ios-test",
            SessionCommandPayload::Steer {
                prompt: "/computer-use and press 2".into(),
                message_id: Some("m-steer".into()),
            },
        )
        .unwrap();
    session.respond_input("req", Vec::new()).unwrap();
    session.interrupt().unwrap();
    session
        .enqueue("/computer-use later", Vec::new(), true)
        .unwrap();

    assert_eq!(generation(&session, "run"), Some(Some(3)));
    assert_eq!(generation(&session, "steer"), Some(Some(3)));
    assert_eq!(
        generation(&session, "respondInput"),
        Some(None),
        "the engine stamps messages only"
    );
    assert_eq!(generation(&session, "interrupt"), Some(None));
    let queue = session.core.doc().read_queue().unwrap();
    assert_eq!(queue[0].computer_generation, Some(3));
}

#[test]
fn a_consent_answer_settles_the_card_once() {
    let (client, _dir) = demo(false);
    let session = client.open_session("chat-computer").unwrap();
    wait_for("the fixture", || session.snapshot().transcript_len > 0);
    assert_eq!(
        session.composer().open_input,
        None,
        "a consent card is answered in the transcript"
    );
    let request = "computer-consent-demo-task";
    assert!(session.answer_consent(request, "consent", "Allow").unwrap());
    assert!(!session.answer_consent(request, "consent", "Deny").unwrap());
    let sent = session.snapshot().input_answers.get(request).cloned();
    assert_eq!(
        sent.map(|answers| answers[0].labels.clone()),
        Some(vec!["Allow".to_owned()])
    );
    let answers = session
        .core
        .doc()
        .read_commands()
        .unwrap()
        .into_iter()
        .filter(|c| matches!(&c.payload, SessionCommandPayload::RespondInput { request_id, .. } if request_id == request))
        .count();
    assert_eq!(answers, 1, "a second tap on a settled card sends nothing");
    wait_for("the host folds the answer", || {
        session.snapshot().transcript().iter().any(|entry| {
            entry.message.parts.iter().any(|part| {
                matches!(part, MessagePart::Input { request_id, resolved: true, answers: Some(_), .. } if request_id == request)
            })
        })
    });
}

#[test]
fn still_frames_come_from_an_online_host_only() {
    let runtime = crate::runtime::shared();
    let (client, _dir) = demo(true);
    let offline = client.open_session("chat-blog").unwrap();
    assert!(!offline.composer().host.online);
    assert_eq!(
        runtime.block_on(offline.computer_still_frame()).unwrap(),
        None,
        "an offline host is never asked (the demo would refuse the call)"
    );
    let session = client.open_session("chat-computer").unwrap();
    wait_for("the host", || session.composer().host.online);
    let still = runtime
        .block_on(session.computer_still_frame())
        .unwrap()
        .expect("a frame");
    assert_eq!(still.sequence, 1);
    assert_eq!(&still.image[1..4], b"PNG");

    let (client, _dir) = demo(false);
    let session = client.open_session("chat-computer").unwrap();
    wait_for("the host", || session.composer().host.online);
    assert_eq!(
        runtime.block_on(session.computer_still_frame()).unwrap(),
        None
    );
}

#[test]
fn a_turn_parked_on_subagents_is_not_running() {
    let (client, _dir) = demo(false);
    let session = client.open_session("chat-veil").unwrap();
    wait_for("the turn", || session.composer().live.turn_running);
    wait_for("the turn to end", || !session.composer().live.turn_running);
    let (state, _) = client.inner.workspace.state();
    let mut parked = state
        .sessions
        .iter()
        .find(|s| s.chat_id == "chat-veil")
        .cloned()
        .unwrap();
    parked.status = zeron_proto::SessionStatus::Working;
    parked.held_by_subagents = true;
    parked.started_at = Some(chrono::Utc::now());
    parked.updated_at = chrono::Utc::now();
    client
        .inner
        .registry_write(|doc| doc.upsert_session(&parked))
        .unwrap();
    let row = || client.inner.workspace.snapshot().session("chat-veil").cloned().unwrap();
    wait_for("the parked row", || row().held_by_subagents);
    assert_eq!(row().host_indicator, zeron_proto::ChatIndicator::Working, "the dot still reads Working");
    session.core.refresh();
    assert!(!session.composer().live.turn_running);
    assert!(!session.snapshot().working);
}
