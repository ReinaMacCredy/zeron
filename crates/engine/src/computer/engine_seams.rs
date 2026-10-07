use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use async_trait::async_trait;
use zeron_proto::computer::{
    COMPUTER_MESSAGE_MAX_LENGTH, ComputerConsentApp, ComputerConsentScope, ComputerPendingKind,
    ComputerPendingRequest,
};
use zeron_proto::{PermissionMode, UserInputAnswer, UserInputKind, UserInputQuestion};

use super::approval_gate::{ApprovalDecision, ComputerApprovalGate};
use super::backend::ComputerError;
use super::manager::{ComputerConsentCard, ComputerManagerSeams};
use crate::sessions::SessionsEngine;

pub const COMPUTER_CONSENT_ALLOW: &str = "Allow";
pub const COMPUTER_CONSENT_DECLINE: &str = "Deny";
pub const COMPUTER_CONSENT_ALWAYS_PREFIX: &str = "Always allow ";
pub const COMPUTER_FOREGROUND_QUESTION_ID: &str = "foreground";
pub const COMPUTER_FOREGROUND_ALLOW: &str = "Allow for this turn";
pub const COMPUTER_FOREGROUND_DECLINE: &str = "No";

pub type PendingSink =
    Arc<dyn Fn(&str, Option<&str>, &str, Option<ComputerPendingRequest>) + Send + Sync>;

pub struct EngineComputerSeams {
    sessions: Mutex<Option<SessionsEngine>>,
    gate: ComputerApprovalGate,
    cards: Mutex<HashMap<String, (String, String)>>,
    pending: Mutex<Option<PendingSink>>,
    approved: Mutex<Option<Arc<dyn Fn() + Send + Sync>>>,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

impl EngineComputerSeams {
    pub fn new(gate: ComputerApprovalGate) -> Self {
        Self {
            sessions: Mutex::new(None),
            gate,
            cards: Mutex::default(),
            pending: Mutex::default(),
            approved: Mutex::default(),
        }
    }

    pub fn on_approved(&self, hook: Arc<dyn Fn() + Send + Sync>) {
        *lock(&self.approved) = Some(hook);
    }

    pub fn on_pending(&self, sink: PendingSink) {
        *lock(&self.pending) = Some(sink);
    }

    fn set_pending(
        &self,
        chat_id: &str,
        turn_id: Option<&str>,
        input_id: &str,
        request: Option<ComputerPendingRequest>,
    ) {
        let sink = lock(&self.pending).clone();
        if let Some(sink) = sink {
            sink(chat_id, turn_id, input_id, request);
        }
    }

    pub fn attach(&self, sessions: SessionsEngine) {
        *lock(&self.sessions) = Some(sessions);
    }

    pub fn detach(&self) {
        lock(&self.sessions).take();
    }

    pub(super) fn sessions(&self) -> Option<SessionsEngine> {
        lock(&self.sessions).clone()
    }
}

pub fn consent_app_title(app: &ComputerConsentApp) -> String {
    format!("Allow Computer to use {}?", app.name)
}

pub fn consent_always_label(app: &ComputerConsentApp) -> Option<String> {
    app.verified()
        .then(|| format!("{COMPUTER_CONSENT_ALWAYS_PREFIX}{}", app.name))
}

pub fn consent_question(card: &ComputerConsentCard) -> UserInputQuestion {
    let always = card
        .app
        .as_ref()
        .filter(|_| card.scope == ComputerConsentScope::Task)
        .and_then(consent_always_label);
    let question = match (&card.app, card.scope) {
        (Some(app), ComputerConsentScope::Task) if app.verified() => format!(
            "{} The agent will use {} until this response ends.",
            consent_app_title(app),
            app.name
        ),
        (Some(app), ComputerConsentScope::Task) => format!(
            "{} Zeron could not confirm which app this is, so this permission lasts for this response only.",
            consent_app_title(app)
        ),
        (_, ComputerConsentScope::Task) => "Allow Computer for this task? The agent will use this Mac's apps until this response ends.".to_owned(),
        (_, ComputerConsentScope::Call) if card.tool_name == "computer_read_clipboard" => {
            "Allow the agent to read the clipboard once?".to_owned()
        }
        (app, ComputerConsentScope::Call) => format!(
            "Allow the agent to use {} once?",
            app.as_ref().map_or("the desktop", |app| app.name.as_str())
        ),
    };
    let mut options = vec![COMPUTER_CONSENT_ALLOW.to_owned()];
    options.extend(always);
    options.push(COMPUTER_CONSENT_DECLINE.into());
    UserInputQuestion {
        id: "consent".into(),
        header: "Computer".into(),
        question,
        options,
        multi_select: false,
        kind: Some(UserInputKind::Consent {
            scope: card.scope,
            app: card.app.clone(),
            tool: Some(card.tool_name.clone()),
            params: Some(card.params_display.clone()),
        }),
        prefill: None,
        multiline: false,
    }
}

pub fn consent_pending(card: &ComputerConsentCard, input_id: &str) -> ComputerPendingRequest {
    let question = consent_question(card);
    ComputerPendingRequest {
        input_id: input_id.to_owned(),
        question_id: question.id,
        kind: ComputerPendingKind::Consent {
            scope: card.scope,
            app: card.app.clone(),
            tool: Some(card.tool_name.clone()).filter(|tool| tool.encode_utf16().count() <= 128),
            params: Some(card.params_display.clone())
                .filter(|params| params.encode_utf16().count() <= COMPUTER_MESSAGE_MAX_LENGTH),
        },
    }
}

pub fn foreground_question(app: Option<&ComputerConsentApp>) -> UserInputQuestion {
    UserInputQuestion {
        id: COMPUTER_FOREGROUND_QUESTION_ID.into(),
        header: "Computer".into(),
        question: format!(
            "{} needs to come to the front to type. Allow for the rest of this turn?",
            app.map_or("The app", |app| app.name.as_str())
        ),
        options: vec![
            COMPUTER_FOREGROUND_ALLOW.into(),
            COMPUTER_FOREGROUND_DECLINE.into(),
        ],
        multi_select: false,
        kind: Some(UserInputKind::Foreground { app: app.cloned() }),
        prefill: None,
        multiline: false,
    }
}

pub fn foreground_allowed(answers: &[UserInputAnswer]) -> Option<bool> {
    let labels: Vec<&str> = answers
        .iter()
        .flat_map(|answer| answer.labels.iter().map(String::as_str))
        .collect();
    if labels.is_empty() {
        None
    } else {
        Some(labels.contains(&COMPUTER_FOREGROUND_ALLOW))
    }
}

pub fn consent_decision(answers: &[UserInputAnswer]) -> ApprovalDecision {
    let labels: Vec<&str> = answers
        .iter()
        .flat_map(|answer| answer.labels.iter().map(String::as_str))
        .collect();
    if labels.contains(&COMPUTER_CONSENT_ALLOW) {
        ApprovalDecision::Accept
    } else if labels
        .iter()
        .any(|label| label.starts_with(COMPUTER_CONSENT_ALWAYS_PREFIX))
    {
        ApprovalDecision::AcceptForSession
    } else if labels.is_empty() {
        ApprovalDecision::Cancel
    } else {
        ApprovalDecision::Decline
    }
}

#[async_trait]
impl ComputerManagerSeams for EngineComputerSeams {
    async fn caller(
        &self,
        chat_id: &str,
        turn_id: Option<&str>,
    ) -> Result<Option<PermissionMode>, ComputerError> {
        let Some(sessions) = self.sessions() else {
            return Ok(None);
        };
        sessions
            .computer_caller(chat_id, turn_id)
            .map_err(ComputerError::Other)
    }

    async fn publish_consent(
        &self,
        card: ComputerConsentCard,
        decision: Option<ApprovalDecision>,
    ) -> Result<(), String> {
        let sessions = self
            .sessions()
            .ok_or_else(|| "Computer consent needs a running engine.".to_owned())?;
        if decision.is_some() {
            if let Some((chat_id, input_id)) = lock(&self.cards).remove(&card.request_id) {
                sessions.settle_input(&chat_id, &input_id);
                self.set_pending(&chat_id, None, &input_id, None);
            }
            return Ok(());
        }
        let input_id = format!("computer-consent-{}", uuid::Uuid::new_v4());
        lock(&self.cards).insert(
            card.request_id.clone(),
            (card.chat_id.clone(), input_id.clone()),
        );
        let answers = sessions
            .raise_input(&card.chat_id, &input_id, vec![consent_question(&card)])
            .inspect_err(|_| {
                lock(&self.cards).remove(&card.request_id);
            })?;
        self.set_pending(
            &card.chat_id,
            card.turn_id.as_deref(),
            &input_id,
            Some(consent_pending(&card, &input_id)),
        );
        let gate = self.gate.clone();
        let pending = lock(&self.pending).clone();
        let approved = lock(&self.approved).clone();
        tokio::spawn(async move {
            let answers = answers.await;
            if let Some(pending) = pending {
                pending(&card.chat_id, None, &input_id, None);
            }
            if let Ok(answers) = answers {
                let decision = consent_decision(&answers);
                if matches!(
                    decision,
                    ApprovalDecision::Accept | ApprovalDecision::AcceptForSession
                ) && let Some(approved) = approved
                {
                    approved();
                }
                gate.respond(&card.chat_id, &card.request_id, decision);
            }
        });
        Ok(())
    }

    fn generation_bumped(&self, chat_id: &str, generation: u64) {
        if let Some(sessions) = self.sessions() {
            sessions.record_computer_generation(chat_id, generation);
        }
    }

    fn note(&self, chat_id: &str, text: &str) {
        if let Some(sessions) = self.sessions() {
            sessions.write_computer_note(chat_id, text, None);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn card(
        tool: &str,
        scope: ComputerConsentScope,
        app: Option<ComputerConsentApp>,
    ) -> ComputerConsentCard {
        ComputerConsentCard {
            request_id: "r1".into(),
            chat_id: "chat".into(),
            turn_id: Some("turn".into()),
            tool_name: tool.into(),
            params_display: r#"{"window_id":"cua:4242:17"}"#.into(),
            scope,
            app,
        }
    }

    fn ghostty(bundle_id: Option<&str>) -> ComputerConsentApp {
        ComputerConsentApp {
            name: "Ghostty".into(),
            bundle_id: bundle_id.map(str::to_owned),
            pid: Some(4242),
        }
    }

    #[test]
    fn the_foreground_question_names_the_app_only_and_maps_its_labels() {
        let app = ghostty(Some("com.mitchellh.ghostty"));
        let question = foreground_question(Some(&app));
        assert_eq!(
            question.question,
            "Ghostty needs to come to the front to type. Allow for the rest of this turn?"
        );
        assert_eq!(question.options, ["Allow for this turn", "No"]);
        for leak in ["computer_", "com.mitchellh", "4242"] {
            assert!(!question.question.contains(leak), "{leak}");
        }
        assert!(
            foreground_question(None)
                .question
                .starts_with("The app needs")
        );
        let answer = |labels: &[&str]| {
            foreground_allowed(&[UserInputAnswer {
                question_id: COMPUTER_FOREGROUND_QUESTION_ID.into(),
                labels: labels.iter().map(|label| (*label).to_owned()).collect(),
            }])
        };
        assert_eq!(answer(&["Allow for this turn"]), Some(true));
        assert_eq!(answer(&["No"]), Some(false));
        assert_eq!(answer(&["something typed"]), Some(false));
        assert_eq!(answer(&[]), None);
    }

    #[test]
    fn the_consent_pending_request_carries_the_card_and_drops_oversized_params() {
        let task = card(
            "computer_type_text",
            ComputerConsentScope::Task,
            Some(ghostty(Some("com.mitchellh.ghostty"))),
        );
        let pending = consent_pending(&task, "computer-consent-1");
        assert_eq!(pending.input_id, "computer-consent-1");
        assert_eq!(pending.question_id, "consent");
        assert_eq!(
            pending.kind,
            ComputerPendingKind::Consent {
                scope: ComputerConsentScope::Task,
                app: task.app.clone(),
                tool: Some("computer_type_text".into()),
                params: Some(task.params_display.clone()),
            }
        );
        let huge = ComputerConsentCard {
            params_display: format!(r#"{{"steps":"{}"}}"#, "s".repeat(4096)),
            ..task
        };
        let ComputerPendingKind::Consent { params, tool, .. } =
            consent_pending(&huge, "computer-consent-2").kind
        else {
            panic!("a consent request");
        };
        assert_eq!(params, None);
        assert_eq!(tool.as_deref(), Some("computer_type_text"));
    }

    #[test]
    fn consent_params_keep_only_what_the_sentence_reads_and_never_a_secret() {
        let args = serde_json::json!({
            "target_id": "t",
            "control_token": "ctl-secret-token",
            "url": "https://user:pw@example.com:8443/cb?code=oauth-secret&token=abc#frag",
            "script": "document.querySelector('#pw').value='hunter2'",
            "files": ["/Users/me/private/tax.pdf", "/Users/me/id.png"],
            "text": "typed secret",
            "value": "set secret",
            "arguments": ["--token=launch-secret"],
            "label": "Send",
            "window_id": "cua:4242:17",
            "keys": ["cmd", "v"],
            "steps": [{ "type": "type_text", "text": "nested secret" }],
            "from": { "label": "A", "text": "drag secret" },
        });
        let params = crate::computer::manager::consent_params(args.as_object().unwrap());
        let card = ComputerConsentCard {
            params_display: params,
            ..card(
                "computer_browser_navigate",
                ComputerConsentScope::Task,
                None,
            )
        };
        let Some(UserInputKind::Consent {
            params: Some(persisted),
            ..
        }) = consent_question(&card).kind
        else {
            panic!("a consent kind with params");
        };
        let ComputerPendingKind::Consent {
            params: Some(pending),
            ..
        } = consent_pending(&card, "computer-consent-1").kind
        else {
            panic!("a pending consent with params");
        };
        assert_eq!(persisted, pending, "one projection for both copies");
        for secret in [
            "ctl-secret-token",
            "oauth-secret",
            "token=abc",
            "user:pw",
            "/cb",
            "hunter2",
            "querySelector",
            "tax.pdf",
            "/Users/me",
            "typed secret",
            "set secret",
            "launch-secret",
            "nested secret",
            "drag secret",
        ] {
            assert!(!persisted.contains(secret), "{secret}: {persisted}");
        }
        let kept: serde_json::Value = serde_json::from_str(&persisted).unwrap();
        assert_eq!(
            kept,
            serde_json::json!({
                "url": "https://example.com:8443",
                "files": ["", ""],
                "label": "Send",
                "window_id": "cua:4242:17",
                "keys": ["cmd", "v"],
                "steps": [{ "type": "type_text" }],
                "from": { "label": "A" },
            })
        );
        let huge =
            serde_json::json!({ "steps": vec![serde_json::json!({ "type": "click" }); 400] });
        assert_eq!(
            crate::computer::manager::consent_params(huge.as_object().unwrap()),
            "{}",
            "an oversized projection is bounded"
        );
    }

    #[test]
    fn consent_question_names_no_tool_bundle_or_pid() {
        let task = ComputerConsentScope::Task;
        let call = ComputerConsentScope::Call;
        let cases = [
            (
                card(
                    "computer_type_text",
                    task,
                    Some(ghostty(Some("com.mitchellh.ghostty"))),
                ),
                "Allow Computer to use Ghostty? The agent will use Ghostty until this response ends.",
            ),
            (
                card("computer_type_text", task, Some(ghostty(None))),
                "Allow Computer to use Ghostty? Zeron could not confirm which app this is, so this permission lasts for this response only.",
            ),
            (
                card("computer_click", task, None),
                "Allow Computer for this task? The agent will use this Mac's apps until this response ends.",
            ),
            (
                card("computer_read_clipboard", call, None),
                "Allow the agent to read the clipboard once?",
            ),
            (
                card("computer_click", call, Some(ghostty(None))),
                "Allow the agent to use Ghostty once?",
            ),
        ];
        for (card, expected) in cases {
            let question = consent_question(&card);
            assert_eq!(question.question, expected);
            assert_eq!(question.header, "Computer");
            for leak in ["computer_", "com.mitchellh", "4242", "process"] {
                assert!(
                    !question.question.contains(leak),
                    "{leak}: {}",
                    question.question
                );
            }
            let Some(UserInputKind::Consent { tool, params, .. }) = question.kind else {
                panic!("a consent kind");
            };
            assert_eq!(tool.as_deref(), Some(card.tool_name.as_str()));
            assert_eq!(params.as_deref(), Some(card.params_display.as_str()));
        }
    }
}
