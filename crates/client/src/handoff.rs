use zeron_doc::MessageRole;
use zeron_proto::Chat;
use zeron_proto::handoff::handoff_blocked;
use zeron_rpc::methods;

use crate::Client;
use crate::error::{ClientError, Result};
use crate::session::SessionHandle;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HandoffTarget {
    pub harness: String,
    pub name: String,
    pub model: String,
    pub model_label: String,
}

impl Client {
    /// Every offered harness with the model a handoff lands on: its catalog
    /// default, the first row.
    pub async fn handoff_targets(&self, device_id: &str) -> Vec<HandoffTarget> {
        let mut targets = Vec::new();
        for harness in self.list_harnesses(device_id).await {
            if !harness.offered() {
                continue;
            }
            let models = self.list_models(device_id, &harness.id).await;
            if let Some(model) = models.into_iter().next() {
                targets.push(HandoffTarget {
                    harness: harness.id,
                    name: harness.label,
                    model: model.id,
                    model_label: model.label,
                });
            }
        }
        targets
    }

    /// Returns the new chat's id. `target` is a `(harness, model)` pick;
    /// `None` keeps the source's agent (a project handoff).
    pub async fn continue_in_new_thread(
        &self,
        source_chat_id: &str,
        target: Option<(&str, &str)>,
        target_space_id: Option<&str>,
    ) -> Result<String> {
        let device_id = self
            .inner
            .workspace
            .chat(source_chat_id)
            .map(|chat| chat.device_id)
            .ok_or_else(|| ClientError::NotFound(source_chat_id.to_owned()))?;
        let mut params = serde_json::json!({
            "sourceChatId": source_chat_id,
            "chatId": crate::new_id(),
            "targetSpaceId": target_space_id,
        });
        if let Some((harness, model)) = target {
            params["harness"] = harness.into();
            params["model"] = model.into();
        }
        let reply = self
            .inner
            .host_rpc(&device_id, methods::CONTINUE_IN_NEW_THREAD, params)
            .await?;
        let chat: Chat =
            serde_json::from_value(reply).map_err(|err| ClientError::HostError(err.to_string()))?;
        Ok(chat.id)
    }
}

impl SessionHandle {
    pub fn handoff_blocked(&self) -> bool {
        let snapshot = self.snapshot();
        let finished = snapshot.hydrated
            && snapshot.transcript().iter().any(|entry| {
                matches!(entry.role(), MessageRole::User | MessageRole::Assistant)
                    && !entry.is_streaming()
            });
        handoff_blocked(self.composer().live.indicator, finished)
    }
}
