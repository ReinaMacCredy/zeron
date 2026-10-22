use zeron_doc::{MessagePart, MessageRole};
use zeron_proto::ChatConfig;
use zeron_proto::model_switch::{ModelSwitchPrompt, model_switch_prompt, needs_switch_warning};

use crate::error::{ClientError, Result};
use crate::Client;

impl Client {
    pub fn model_switch_prompt(
        &self,
        chat_id: &str,
        to_model: &str,
        from_label: &str,
        to_label: &str,
    ) -> Option<ModelSwitchPrompt> {
        let config = self.session_config(chat_id)?;
        needs_switch_warning(self.chat_has_history(chat_id), config.model.as_deref(), to_model)
            .then(|| model_switch_prompt(Some(config.harness), from_label, to_label))
    }

    /// Pending echoes count and an unloaded chat is assumed read: a fresh
    /// chat must not warn, a just-sent one must not skip.
    fn chat_has_history(&self, chat_id: &str) -> bool {
        let Some(core) = self.inner.session_core(chat_id) else {
            return true;
        };
        let snapshot = core.snapshot();
        !snapshot.hydrated
            || !snapshot.pending.is_empty()
            || snapshot.entries.iter().any(|entry| {
                entry.role() != MessageRole::System
                    || entry
                        .parts()
                        .iter()
                        .any(|part| matches!(part, MessagePart::Handoff { .. }))
            })
    }

    /// Only the chat's host writes the note; an unreachable host still gets
    /// the change, unmarked.
    pub async fn set_session_config_noted(
        &self,
        chat_id: &str,
        config: &ChatConfig,
        note: &str,
        hint: &str,
    ) -> Result<()> {
        let device_id = self
            .inner
            .workspace
            .chat(chat_id)
            .ok_or_else(|| ClientError::NotFound(chat_id.to_owned()))?
            .device_id;
        let params = serde_json::json!({
            "op": "setChatConfig",
            "chatId": chat_id,
            "config": config,
            "note": { "text": note, "hint": hint },
        });
        match self
            .inner
            .host_rpc(&device_id, zeron_rpc::methods::MUTATE, params)
            .await
        {
            Ok(_) => Ok(()),
            Err(err) => {
                tracing::debug!(error = %err, "noted setChatConfig fell back to a row write");
                self.set_session_config(chat_id, config)
            }
        }
    }
}
