//! Only chats with a live watcher get predictions, so background chats never
//! spend a model call.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard};

use serde::{Deserialize, Serialize};
use tokio::sync::watch;
use zeron_doc::{MessagePart, MessageRole, MessageStatus, SessionMessageEntry};
use zeron_proto::HarnessId;

#[cfg(target_os = "macos")]
use crate::agent_accounts::keychain;
use crate::registry::{CustomPredictionApi, HarnessRegistry, PredictionSettings};
use crate::titles::{OneShot, one_shot_target, run_one_shot};

const KEY_SERVICE: &str = "Zeron prediction API key";
const API_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(20);

const EARLIER_MESSAGES: usize = 8;
const EARLIER_MESSAGE_CHARS: usize = 500;
const LAST_MESSAGE_CHARS: usize = 2_000;
const REPLY_CHARS: usize = 4_000;
const PREDICTION_CHARS: usize = 300;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Prediction {
    pub entry_id: String,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PredictionContext {
    pub entry_id: String,
    pub earlier: Vec<String>,
    pub last_message: String,
    pub reply: String,
}

impl PredictionContext {
    pub(crate) fn from_entries(entries: &[SessionMessageEntry]) -> Option<Self> {
        let last = entries.last()?;
        if last.role != MessageRole::Assistant || last.status != Some(MessageStatus::Complete) {
            return None;
        }
        let prompt_ix = entries.iter().rposition(is_person_prompt)?;
        let reply = entries[prompt_ix + 1..]
            .iter()
            .filter(|entry| entry.role == MessageRole::Assistant)
            .map(entry_text)
            .filter(|text| !text.is_empty())
            .collect::<Vec<_>>()
            .join("\n\n");
        if reply.is_empty() {
            return None;
        }
        let mut earlier: Vec<String> = entries[..prompt_ix]
            .iter()
            .rev()
            .filter(|entry| is_person_prompt(entry))
            .map(entry_text)
            .filter(|text| !text.is_empty())
            .take(EARLIER_MESSAGES)
            .map(|text| head(&text, EARLIER_MESSAGE_CHARS))
            .collect();
        earlier.reverse();
        Some(Self {
            entry_id: last.id.clone(),
            earlier,
            last_message: head(&entry_text(&entries[prompt_ix]), LAST_MESSAGE_CHARS),
            reply: tail(&reply, REPLY_CHARS),
        })
    }

    fn data(&self) -> String {
        let data = serde_json::json!({
            "earlierMessages": self.earlier,
            "lastMessage": self.last_message,
            "agentReply": self.reply,
        });
        format!("Conversation (JSON):\n{data}")
    }

    fn prompt(&self) -> String {
        format!(
            "{}\n\n{}",
            zeron_harness::PREDICTION_INSTRUCTIONS,
            self.data()
        )
    }
}

fn is_person_prompt(entry: &SessionMessageEntry) -> bool {
    entry.role == MessageRole::User
        && entry.from_chat_id.is_none()
        && entry.answers_request_id.is_none()
}

fn entry_text(entry: &SessionMessageEntry) -> String {
    entry
        .parts
        .iter()
        .filter_map(|part| match part {
            MessagePart::Text { text, .. } => Some(text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n\n")
        .trim()
        .to_string()
}

fn head(text: &str, chars: usize) -> String {
    text.chars().take(chars).collect()
}

fn tail(text: &str, chars: usize) -> String {
    let skip = text.chars().count().saturating_sub(chars);
    text.chars().skip(skip).collect()
}

pub(crate) fn clean_prediction(raw: &str) -> String {
    let Some((_, message)) = raw.rsplit_once("Message:") else {
        return String::new();
    };
    let paragraph = message.trim().split("\n\n").next().unwrap_or("");
    let text = paragraph
        .trim()
        .trim_matches(|c| matches!(c, '"' | '\'' | '`' | '“' | '”'))
        .trim();
    head(text, PREDICTION_CHARS).trim_end().to_string()
}

#[derive(Default)]
struct ChatSlot {
    tx: Option<watch::Sender<Option<Prediction>>>,
    generation: u64,
    pending: Option<String>,
}

impl ChatSlot {
    fn sender(&mut self) -> &watch::Sender<Option<Prediction>> {
        self.tx.get_or_insert_with(|| watch::channel(None).0)
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PredictionSettingsUpdate {
    #[serde(flatten)]
    pub settings: PredictionSettings,
    /// Write-only; an empty string removes the stored key.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_key: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PredictionSettingsState {
    #[serde(flatten)]
    pub settings: PredictionSettings,
    pub has_api_key: bool,
}

fn valid_api_key(key: &str) -> bool {
    (1..=512).contains(&key.len())
        && key
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "._~+/=-".contains(c))
}

fn completion_text(reply: &serde_json::Value) -> Option<&str> {
    reply["choices"][0]["message"]["content"].as_str()
}

pub(crate) struct Predictions {
    registry: Arc<HarnessRegistry>,
    chats: Mutex<HashMap<String, ChatSlot>>,
    http: reqwest::Client,
}

impl Predictions {
    pub(crate) fn new(registry: Arc<HarnessRegistry>) -> Arc<Self> {
        Arc::new(Self {
            registry,
            chats: Mutex::new(HashMap::new()),
            http: reqwest::Client::builder()
                .timeout(API_TIMEOUT)
                .build()
                .unwrap_or_default(),
        })
    }

    /// One key per engine data dir, so a Dev bundle never replaces the app's.
    fn key_account(&self) -> Option<String> {
        self.registry
            .data_dir()
            .map(|dir| dir.to_string_lossy().into_owned())
    }

    async fn api_key(&self) -> Option<String> {
        keychain::read_secret(KEY_SERVICE, &self.key_account()?).await
    }

    pub(crate) async fn settings_state(&self) -> PredictionSettingsState {
        PredictionSettingsState {
            settings: self.registry.prediction_settings(),
            has_api_key: self.api_key().await.is_some(),
        }
    }

    pub(crate) async fn update_settings(
        &self,
        update: PredictionSettingsUpdate,
    ) -> Result<PredictionSettingsState, String> {
        let api_key = update.api_key.map(|key| key.trim().to_string());
        if let Some(key) = api_key.as_deref()
            && !key.is_empty()
            && !valid_api_key(key)
        {
            return Err("That does not look like an API key".into());
        }
        self.registry.set_prediction_settings(update.settings)?;
        if let Some(key) = api_key {
            let account = self
                .key_account()
                .ok_or_else(|| "The engine has no data directory".to_string())?;
            if key.is_empty() {
                keychain::delete_secret(KEY_SERVICE, &account).await;
            } else {
                keychain::write_secret(KEY_SERVICE, &account, &key)
                    .await
                    .map_err(|error| error.to_string())?;
            }
        }
        if !self.registry.prediction_settings().enabled {
            self.clear_all();
        }
        Ok(self.settings_state().await)
    }

    fn chats(&self) -> MutexGuard<'_, HashMap<String, ChatSlot>> {
        self.chats
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    pub(crate) fn watch(&self, chat_id: &str) -> watch::Receiver<Option<Prediction>> {
        self.chats()
            .entry(chat_id.to_string())
            .or_default()
            .sender()
            .subscribe()
    }

    fn watched(&self, chat_id: &str) -> bool {
        self.chats()
            .get(chat_id)
            .and_then(|slot| slot.tx.as_ref())
            .is_some_and(|tx| tx.receiver_count() > 0)
    }

    pub(crate) fn clear(&self, chat_id: &str) {
        if let Some(slot) = self.chats().get_mut(chat_id) {
            slot.generation += 1;
            slot.pending = None;
            slot.sender()
                .send_if_modified(|value| value.take().is_some());
        }
    }

    pub(crate) fn clear_all(&self) {
        for slot in self.chats().values_mut() {
            slot.generation += 1;
            slot.pending = None;
            slot.sender()
                .send_if_modified(|value| value.take().is_some());
        }
    }

    pub(crate) fn refresh(
        self: &Arc<Self>,
        chat_id: &str,
        session_harness: HarnessId,
        entries: impl FnOnce() -> Vec<SessionMessageEntry>,
    ) {
        let settings = self.registry.prediction_settings();
        if !settings.enabled || !self.watched(chat_id) {
            return;
        }
        let Some(context) = PredictionContext::from_entries(&entries()) else {
            return;
        };
        let generation = {
            let mut chats = self.chats();
            let Some(slot) = chats.get_mut(chat_id) else {
                return;
            };
            let sender = slot.sender();
            if sender.receiver_count() == 0
                || sender
                    .borrow()
                    .as_ref()
                    .is_some_and(|p| p.entry_id == context.entry_id)
                || slot.pending.as_deref() == Some(context.entry_id.as_str())
            {
                return;
            }
            slot.pending = Some(context.entry_id.clone());
            slot.generation
        };
        let this = self.clone();
        let chat_id = chat_id.to_string();
        tokio::spawn(async move {
            let text = match settings.custom {
                Some(api) => this.generate_with_api(&api, &context).await,
                None => {
                    this.generate(settings.harness, settings.model, session_harness, &context)
                        .await
                }
            };
            let mut chats = this.chats();
            let Some(slot) = chats.get_mut(&chat_id) else {
                return;
            };
            if slot.generation != generation {
                return;
            }
            slot.pending = None;
            if let Some(text) = text {
                slot.sender().send_replace(Some(Prediction {
                    entry_id: context.entry_id,
                    text,
                }));
            }
        });
    }

    async fn generate_with_api(
        &self,
        api: &CustomPredictionApi,
        context: &PredictionContext,
    ) -> Option<String> {
        let body = serde_json::json!({
            "model": api.model,
            "messages": [
                { "role": "system", "content": zeron_harness::PREDICTION_INSTRUCTIONS },
                { "role": "user", "content": context.data() },
            ],
        });
        let mut request = self.http.post(api.endpoint()).json(&body);
        if let Some(key) = self.api_key().await {
            request = request.bearer_auth(key);
        }
        let response = match request.send().await {
            Ok(response) => response,
            Err(error) => {
                tracing::debug!(base_url = %api.base_url, error = %error.without_url(), "prediction API unreachable");
                return None;
            }
        };
        let status = response.status();
        if !status.is_success() {
            tracing::debug!(base_url = %api.base_url, %status, "prediction API refused");
            return None;
        }
        let reply: serde_json::Value = response.json().await.ok()?;
        let text = clean_prediction(completion_text(&reply)?);
        (!text.is_empty()).then_some(text)
    }

    async fn generate(
        &self,
        harness: Option<HarnessId>,
        model: Option<String>,
        session_harness: HarnessId,
        context: &PredictionContext,
    ) -> Option<String> {
        let target = one_shot_target(&self.registry, harness, model, session_harness).await?;
        match run_one_shot(
            target.harness.as_ref(),
            target.lease,
            OneShot {
                harness: target.harness_id,
                model: target.model,
                prompt: context.prompt(),
                instructions: zeron_harness::PREDICTION_INSTRUCTIONS,
                attempt_timeout: std::time::Duration::from_secs(30),
                retry_delays_ms: &[],
            },
            clean_prediction,
        )
        .await
        {
            Ok(text) => Some(text),
            Err(err) => {
                tracing::debug!(error = %err, "composer prediction skipped");
                None
            }
        }
    }
}

#[cfg(not(target_os = "macos"))]
mod keychain {
    const UNSUPPORTED: &str = "Saving a prediction API key needs the macOS Keychain";

    pub(super) async fn read_secret(_service: &str, _account: &str) -> Option<String> {
        None
    }

    pub(super) async fn write_secret(
        _service: &str,
        _account: &str,
        _secret: &str,
    ) -> Result<(), &'static str> {
        Err(UNSUPPORTED)
    }

    pub(super) async fn delete_secret(_service: &str, _account: &str) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(id: &str, role: MessageRole, text: &str) -> SessionMessageEntry {
        SessionMessageEntry {
            id: id.into(),
            role,
            parts: vec![MessagePart::Text {
                id: format!("{id}:text"),
                text: text.into(),
            }],
            created_at: 0,
            device_id: "d".into(),
            status: (role == MessageRole::Assistant).then_some(MessageStatus::Complete),
            continuation_of: None,
            from_chat_id: None,
            fork_import: false,
            answers_request_id: None,
            duration_ms: None,
        }
    }

    #[test]
    fn context_follows_a_completed_reply_with_the_persons_earlier_messages() {
        let mut thread_prompt = entry("u0", MessageRole::User, "from another thread");
        thread_prompt.from_chat_id = Some("other".into());
        let entries = vec![
            entry("u1", MessageRole::User, "build a platformer"),
            entry("a1", MessageRole::Assistant, "Built it."),
            thread_prompt,
            entry("u2", MessageRole::User, "install it on my device"),
            entry("a2", MessageRole::Assistant, "Installing."),
            entry("a3", MessageRole::Assistant, "The ROM is ready."),
        ];
        let context = PredictionContext::from_entries(&entries).unwrap();
        assert_eq!(context.entry_id, "a3");
        assert_eq!(context.earlier, vec!["build a platformer".to_string()]);
        assert_eq!(context.last_message, "install it on my device");
        assert_eq!(context.reply, "Installing.\n\nThe ROM is ready.");
    }

    #[test]
    fn no_context_while_streaming_after_an_abort_or_without_a_prompt() {
        let mut streaming = entry("a1", MessageRole::Assistant, "Working");
        streaming.status = Some(MessageStatus::Streaming);
        let prompt = entry("u1", MessageRole::User, "go");
        assert!(PredictionContext::from_entries(&[prompt.clone(), streaming]).is_none());
        let mut aborted = entry("a1", MessageRole::Assistant, "Stopped");
        aborted.status = Some(MessageStatus::Aborted);
        assert!(PredictionContext::from_entries(&[prompt.clone()]).is_none());
        assert!(PredictionContext::from_entries(&[prompt, aborted]).is_none());
        assert!(
            PredictionContext::from_entries(&[entry("a1", MessageRole::Assistant, "Hi")]).is_none()
        );
    }

    #[test]
    fn predictions_are_cleaned() {
        assert_eq!(
            clean_prediction("Next: approves\nMessage: \"Fix the spacing.\"\n\nExplanation"),
            "Fix the spacing."
        );
        assert_eq!(
            clean_prediction("Next: reports done\nMessage:  ok ship it  "),
            "ok ship it"
        );
        assert_eq!(clean_prediction("Next: unknown\nMessage:\n"), "");
        assert_eq!(clean_prediction("Next: reports done"), "");
        assert_eq!(clean_prediction("ok ship it"), "");
        assert_eq!(
            clean_prediction(&format!("Message: {}", "x".repeat(400)))
                .chars()
                .count(),
            PREDICTION_CHARS
        );
    }

    #[tokio::test]
    async fn clear_supersedes_the_value_for_watchers() {
        let predictions = Predictions::new(Arc::new(HarnessRegistry::new()));
        let mut rx = predictions.watch("c");
        predictions
            .chats()
            .get_mut("c")
            .unwrap()
            .sender()
            .send_replace(Some(Prediction {
                entry_id: "a1".into(),
                text: "next".into(),
            }));
        assert!(rx.has_changed().unwrap());
        assert_eq!(rx.borrow_and_update().as_ref().unwrap().text, "next");
        predictions.clear("c");
        assert!(rx.has_changed().unwrap());
        assert!(rx.borrow_and_update().is_none());
    }

    fn mock_registry(reply: &str) -> Arc<HarnessRegistry> {
        let registry = Arc::new(HarnessRegistry::new());
        registry.register(Arc::new(zeron_harness::mock::MockHarness {
            script: vec![
                zeron_proto::AgentEvent::TextDelta { text: reply.into() },
                zeron_proto::AgentEvent::Done {
                    status: zeron_proto::DoneStatus::Completed,
                    result: None,
                    error: None,
                    session_id: None,
                },
            ],
        }));
        registry
    }

    fn completed_turn() -> Vec<SessionMessageEntry> {
        vec![
            entry("u1", MessageRole::User, "build it"),
            entry("a1", MessageRole::Assistant, "The card is built."),
        ]
    }

    #[tokio::test]
    async fn a_watched_completed_turn_publishes_the_cleaned_prediction() {
        let registry = mock_registry("Next: approves\nMessage: \"Fix the spacing.\"");
        registry
            .set_prediction_settings(crate::registry::PredictionSettings {
                enabled: true,
                ..Default::default()
            })
            .unwrap();
        let predictions = Predictions::new(registry);
        let mut rx = predictions.watch("c");
        predictions.refresh("c", HarnessId::Mock, completed_turn);
        tokio::time::timeout(std::time::Duration::from_secs(10), rx.changed())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            rx.borrow().clone(),
            Some(Prediction {
                entry_id: "a1".into(),
                text: "Fix the spacing.".into(),
            })
        );
    }

    #[tokio::test]
    async fn nothing_runs_when_disabled_or_unwatched() {
        let registry = mock_registry("next");
        let predictions = Predictions::new(registry.clone());
        let rx = predictions.watch("c");
        predictions.refresh("c", HarnessId::Mock, || panic!("disabled reads no entries"));
        drop(rx);
        registry
            .set_prediction_settings(crate::registry::PredictionSettings {
                enabled: true,
                ..Default::default()
            })
            .unwrap();
        predictions.refresh("c", HarnessId::Mock, || {
            panic!("unwatched reads no entries")
        });
    }

    #[test]
    fn api_keys_are_plain_tokens() {
        assert!(valid_api_key("sk-or-v1-abc123"));
        assert!(valid_api_key("gsk_AbC.def~ghi+jk/l="));
        assert!(!valid_api_key(""));
        assert!(!valid_api_key("sk abc"));
        assert!(!valid_api_key("sk\"; rm"));
        assert!(!valid_api_key(&"a".repeat(513)));
    }

    #[test]
    fn completion_text_reads_the_first_choice() {
        let reply = serde_json::json!({
            "choices": [{ "message": { "role": "assistant", "content": "Ship it" } }]
        });
        assert_eq!(completion_text(&reply), Some("Ship it"));
        let empty = serde_json::json!({ "choices": [{ "message": { "content": null } }] });
        assert_eq!(completion_text(&empty), None);
        assert_eq!(completion_text(&serde_json::json!({})), None);
    }

    #[tokio::test]
    async fn a_custom_api_answers_through_chat_completions_without_a_key() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = Vec::new();
            while !request.ends_with(b"\r\n\r\n") {
                request.push(socket.read_u8().await.unwrap());
            }
            let head = String::from_utf8(request).unwrap();
            let length: usize = head
                .lines()
                .find_map(|line| {
                    line.to_ascii_lowercase()
                        .strip_prefix("content-length:")
                        .map(|n| n.trim().parse().unwrap())
                })
                .unwrap();
            let mut body = vec![0; length];
            socket.read_exact(&mut body).await.unwrap();
            let reply = br#"{"choices":[{"message":{"role":"assistant","content":"Next: approves\nMessage: \"Fix the spacing.\""}}]}"#;
            socket
                .write_all(
                    format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n",
                        reply.len()
                    )
                    .as_bytes(),
                )
                .await
                .unwrap();
            socket.write_all(reply).await.unwrap();
            (
                head,
                serde_json::from_slice::<serde_json::Value>(&body).unwrap(),
            )
        });
        let registry = Arc::new(HarnessRegistry::new());
        registry
            .set_prediction_settings(PredictionSettings {
                enabled: true,
                custom: Some(CustomPredictionApi {
                    base_url: format!("http://127.0.0.1:{port}/v1/"),
                    model: "small-model".into(),
                }),
                ..Default::default()
            })
            .unwrap();
        let predictions = Predictions::new(registry);
        let mut rx = predictions.watch("c");
        predictions.refresh("c", HarnessId::Mock, completed_turn);
        tokio::time::timeout(std::time::Duration::from_secs(10), rx.changed())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(rx.borrow().as_ref().unwrap().text, "Fix the spacing.");
        let (head, body) = server.await.unwrap();
        assert!(head.starts_with("POST /v1/chat/completions "), "{head}");
        assert!(
            !head.to_ascii_lowercase().contains("authorization"),
            "{head}"
        );
        assert_eq!(body["model"], "small-model");
        assert_eq!(body["messages"][0]["role"], "system");
        assert_eq!(
            body["messages"][0]["content"],
            zeron_harness::PREDICTION_INSTRUCTIONS
        );
        assert!(
            body["messages"][1]["content"]
                .as_str()
                .unwrap()
                .contains("The card is built.")
        );
    }
}
