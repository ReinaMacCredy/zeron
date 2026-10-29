use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

use serde::Deserialize;
use serde_json::{Value, json};
use tokio::sync::watch;
use zeron_proto::view::SecretStatus;
use zeron_proto::{UserInputKind, UserInputQuestion};

use crate::sessions::{SECRET_QUESTION_ID, SessionsEngine};

pub const TTL: Duration = Duration::from_secs(24 * 60 * 60);
pub const DEFAULT_WAIT: Duration = Duration::from_secs(10 * 60);
const MIN_WAIT: Duration = Duration::from_secs(1);
const MAX_WAIT: Duration = Duration::from_secs(60 * 60);

type Outcome = Option<(String, SecretStatus)>;

#[derive(Clone)]
pub struct SecretRequests {
    dir: PathBuf,
    retries: Arc<Mutex<HashMap<String, watch::Receiver<Outcome>>>>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RequestSecretParams {
    pub chat_id: String,
    pub run_secret: String,
    pub label: String,
    pub reason: String,
    #[serde(default)]
    pub placeholder: Option<String>,
    #[serde(default)]
    pub timeout_ms: Option<u64>,
    #[serde(default)]
    pub client_request_id: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AnswerSecretParams {
    pub chat_id: String,
    pub request_id: String,
    #[serde(default)]
    pub secret: Option<String>,
}

impl std::fmt::Debug for AnswerSecretParams {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AnswerSecretParams")
            .field("chat_id", &self.chat_id)
            .field("request_id", &self.request_id)
            .finish_non_exhaustive()
    }
}

impl SecretRequests {
    pub fn new(data_dir: &Path) -> Self {
        let requests = Self {
            dir: data_dir.join("secrets"),
            retries: Default::default(),
        };
        requests.sweep(SystemTime::now());
        requests
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    fn file(&self, request_id: &str) -> PathBuf {
        self.dir.join(request_id)
    }

    pub fn sweep(&self, now: SystemTime) -> usize {
        let Ok(entries) = std::fs::read_dir(&self.dir) else {
            return 0;
        };
        entries
            .flatten()
            .filter(|entry| {
                entry
                    .metadata()
                    .and_then(|meta| meta.modified())
                    .is_ok_and(|modified| modified + TTL <= now)
            })
            .filter(|entry| std::fs::remove_file(entry.path()).is_ok())
            .count()
    }

    pub async fn request(
        &self,
        sessions: &SessionsEngine,
        p: RequestSecretParams,
    ) -> Result<Value, String> {
        let label = p.label.trim();
        let reason = p.reason.trim();
        if label.is_empty() || reason.is_empty() {
            return Err("label and reason must not be empty".into());
        }
        let wait = p
            .timeout_ms
            .map_or(DEFAULT_WAIT, Duration::from_millis)
            .clamp(MIN_WAIT, MAX_WAIT);
        let question = UserInputQuestion {
            id: SECRET_QUESTION_ID.into(),
            header: label.into(),
            question: reason.into(),
            options: Vec::new(),
            multi_select: false,
            kind: Some(UserInputKind::Secret {
                placeholder: p
                    .placeholder
                    .map(|text| text.trim().to_owned())
                    .filter(|text| !text.is_empty()),
            }),
            prefill: None,
            multiline: false,
            approval: false,
        };
        let key = p
            .client_request_id
            .filter(|id| !id.is_empty())
            .map(|id| format!("{}\0{id}", p.chat_id));
        let retry = key.as_ref().and_then(|key| {
            self.retries
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .get(key)
                .cloned()
        });
        let (request_id, status) = match retry {
            Some(mut outcome) => {
                let settled = outcome
                    .wait_for(Option::is_some)
                    .await
                    .map_err(|_| "The first ask with this clientRequestId failed.".to_owned())?;
                settled.clone().expect("waited for an outcome")
            }
            None => {
                let (tx, rx) = watch::channel(None);
                if let Some(key) = &key {
                    self.retries
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                        .insert(key.clone(), rx);
                }
                let asked = sessions
                    .ask_secret(&p.chat_id, &p.run_secret, question, wait)
                    .await;
                if let Some(key) = &key
                    && !matches!(asked, Ok((_, SecretStatus::Saved)))
                {
                    self.retries
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                        .remove(key);
                }
                let asked = asked?;
                let _ = tx.send(Some(asked.clone()));
                asked
            }
        };
        Ok(match status {
            SecretStatus::Saved if self.file(&request_id).is_file() => json!({
                "status": SecretStatus::SAVED,
                "file": self.file(&request_id).to_string_lossy(),
            }),
            SecretStatus::Saved => {
                return Err(
                    "The saved secret expired. Ask again with a new clientRequestId.".into(),
                );
            }
            SecretStatus::Declined => json!({ "status": SecretStatus::DECLINED }),
            SecretStatus::TimedOut => json!({ "status": SecretStatus::TIMED_OUT }),
            SecretStatus::Pending | SecretStatus::Cancelled => {
                json!({ "status": SecretStatus::CANCELLED })
            }
        })
    }

    pub fn answer(&self, sessions: &SessionsEngine, p: AnswerSecretParams) -> Result<(), String> {
        let secret = p.secret.as_deref().map(str::trim);
        if secret.is_some_and(str::is_empty) {
            return Err("Enter the secret, or decline.".into());
        }
        let answer = sessions.take_secret_input(&p.chat_id, &p.request_id)?;
        let Some(secret) = secret else {
            answer.finish(SecretStatus::DECLINED);
            return Ok(());
        };
        let file = self.file(&p.request_id);
        crate::agent_accounts::private_dir(&self.dir)
            .map_err(|_| "Could not store the secret.".to_owned())?;
        crate::runner::write_private_atomic(&file, secret.as_bytes())
            .map_err(|_| "Could not store the secret.".to_owned())?;
        if !answer.finish(SecretStatus::SAVED) {
            let _ = std::fs::remove_file(&file);
            return Err("The agent that asked has stopped, so this secret can't be used.".into());
        }
        tokio::spawn(async move {
            tokio::time::sleep(TTL).await;
            let _ = std::fs::remove_file(file);
        });
        Ok(())
    }
}
