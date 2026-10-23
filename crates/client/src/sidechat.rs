use chrono::Utc;
use zeron_proto::sidechat::{sidechat_tab_title, sidechat_time_left};
use zeron_rpc::methods;

use crate::Client;
use crate::error::{ClientError, Result};
use crate::session::SendRequest;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SidechatRow {
    pub chat_id: String,
    pub title: String,
    pub time_left: String,
}

impl Client {
    /// Live sidechats of `source_chat_id`, oldest first.
    pub fn sidechats_of(&self, source_chat_id: &str) -> Vec<SidechatRow> {
        let now = Utc::now();
        let (state, _) = self.inner.workspace.state();
        let mut live: Vec<_> = state
            .chats
            .iter()
            .filter(|chat| chat.live_sidechat_source(now) == Some(source_chat_id))
            .collect();
        live.sort_by_key(|chat| chat.created_at);
        live.into_iter()
            .map(|chat| SidechatRow {
                chat_id: chat.id.clone(),
                title: sidechat_tab_title(chat.last_message_preview.as_deref()),
                time_left: sidechat_time_left(chat.created_at, now),
            })
            .collect()
    }

    /// A bare `/btw` reopens the newest live sidechat; a prompt always gets a
    /// fresh one, and is sent into it.
    pub async fn open_sidechat(
        &self,
        source_chat_id: &str,
        prompt: Option<String>,
    ) -> Result<String> {
        if prompt.is_none()
            && let Some(last) = self.sidechats_of(source_chat_id).pop()
        {
            return Ok(last.chat_id);
        }
        let chat_id = self.create_sidechat(source_chat_id).await?;
        if let Some(prompt) = prompt {
            self.open_session(&chat_id)?
                .send(SendRequest::text(prompt))?;
        }
        Ok(chat_id)
    }

    pub async fn create_sidechat(&self, source_chat_id: &str) -> Result<String> {
        let device_id = self
            .inner
            .workspace
            .chat(source_chat_id)
            .map(|chat| chat.device_id)
            .ok_or_else(|| ClientError::NotFound(source_chat_id.to_owned()))?;
        let params = serde_json::json!({
            "sourceChatId": source_chat_id,
            "chatId": crate::new_id(),
        });
        let reply = self
            .inner
            .host_rpc(&device_id, methods::CREATE_SIDECHAT, params)
            .await?;
        self.host_created_chat(reply).await
    }
}
