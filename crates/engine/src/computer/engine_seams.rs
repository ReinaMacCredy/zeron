use std::collections::HashMap;
use std::sync::{Mutex, MutexGuard, PoisonError};

use async_trait::async_trait;
use zeron_proto::computer::{ComputerConsentApp, ComputerConsentScope};
use zeron_proto::{PermissionMode, UserInputAnswer, UserInputKind, UserInputQuestion};

use super::approval_gate::{ApprovalDecision, ComputerApprovalGate};
use super::backend::ComputerError;
use super::manager::{COMPUTER_CONTROL_ON_DISCLOSURE, ComputerConsentCard, ComputerManagerSeams};
use crate::sessions::SessionsEngine;

pub const COMPUTER_CONSENT_ALLOW: &str = "Allow";
pub const COMPUTER_CONSENT_DECLINE: &str = "Deny";
pub const COMPUTER_CONSENT_ALWAYS_PREFIX: &str = "Always allow ";

pub struct EngineComputerSeams {
    sessions: Mutex<Option<SessionsEngine>>,
    gate: ComputerApprovalGate,
    cards: Mutex<HashMap<String, (String, String)>>,
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
    match &app.bundle_id {
        Some(bundle_id) => format!("Allow Computer to use {} ({bundle_id})?", app.name),
        None => format!("Allow Computer to use {} (unverified app)?", app.name),
    }
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
            "{} The agent wants to run {} and will drive {} until the turn ends.",
            consent_app_title(app),
            card.tool_name,
            app.name
        ),
        (Some(app), ComputerConsentScope::Task) => format!(
            "{} Zeron could not verify this app's bundle id{}, so this grant lasts for this turn only and cannot be saved. The agent wants to run {}.",
            consent_app_title(app),
            app.pid
                .map(|pid| format!(" (process {pid})"))
                .unwrap_or_default(),
            card.tool_name
        ),
        (_, ComputerConsentScope::Task) => format!(
            "Allow Computer for this task? The agent wants to run {} and will drive this Mac's apps until the turn ends.",
            card.tool_name
        ),
        (app, ComputerConsentScope::Call) => format!(
            "Allow the agent to run {} once{}?",
            card.tool_name,
            app.as_ref()
                .map(|app| format!(" in {}", app.name))
                .unwrap_or_default()
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
        }),
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
        let gate = self.gate.clone();
        tokio::spawn(async move {
            if let Ok(answers) = answers.await {
                gate.respond(&card.chat_id, &card.request_id, consent_decision(&answers));
            }
        });
        Ok(())
    }

    async fn disclose_control(&self, chat_id: &str, _turn_id: Option<&str>) {
        let Some(sessions) = self.sessions() else {
            return;
        };
        sessions.write_computer_note(chat_id, COMPUTER_CONTROL_ON_DISCLOSURE, None);
    }

    fn generation_bumped(&self, chat_id: &str, generation: u64) {
        if let Some(sessions) = self.sessions() {
            sessions.record_computer_generation(chat_id, generation);
        }
    }
}
