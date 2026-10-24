use zeron_proto::claude_config::{ClaudeConfigSnapshot, LocalCommandOutput};
use zeron_proto::claude_panel::{ChipChoices, STATS_DAYS};
use zeron_proto::usage::UsageSummary;
use zeron_proto::{HarnessId, ReasoningLevel};
use zeron_rpc::methods;

use crate::Client;
use crate::error::{ClientError, Result};

impl Client {
    fn claude_target(&self, chat_id: &str) -> Result<(String, serde_json::Value)> {
        let chat = self
            .chat(chat_id)
            .ok_or_else(|| ClientError::NotFound(chat_id.to_owned()))?;
        let params = serde_json::json!({ "chatId": chat_id, "targetDeviceId": chat.device_id });
        Ok((chat.device_id, params))
    }

    pub async fn read_claude_config(&self, chat_id: &str) -> Result<ClaudeConfigSnapshot> {
        let (device, params) = self.claude_target(chat_id)?;
        self.host_call(&device, methods::READ_CLAUDE_CONFIG, params)
            .await
    }

    pub async fn run_claude_local_command(
        &self,
        chat_id: &str,
        command: &str,
        connect_mcp: bool,
    ) -> Result<LocalCommandOutput> {
        let (device, mut params) = self.claude_target(chat_id)?;
        params["command"] = command.into();
        params["connectMcp"] = connect_mcp.into();
        self.host_call(&device, methods::RUN_CLAUDE_LOCAL_COMMAND, params)
            .await
    }

    pub async fn add_chat_note(&self, chat_id: &str, text: &str) -> Result<()> {
        let (device, _) = self.claude_target(chat_id)?;
        self.inner
            .host_rpc(
                &device,
                methods::MUTATE,
                serde_json::json!({ "op": "addChatNote", "chatId": chat_id, "text": text }),
            )
            .await
            .map(drop)
    }

    pub async fn claude_stats_summary(&self, chat_id: &str) -> Result<UsageSummary> {
        let (device, _) = self.claude_target(chat_id)?;
        self.host_call(
            &device,
            methods::GET_USAGE_SUMMARY,
            serde_json::json!({ "days": STATS_DAYS }),
        )
        .await
    }

    pub async fn chip_choices(&self, chat_id: &str) -> ChipChoices {
        let Ok((device, _)) = self.claude_target(chat_id) else {
            return ChipChoices::default();
        };
        let config = self.session_config(chat_id);
        let harness = config.as_ref().map_or(HarnessId::ClaudeCode, |c| c.harness);
        let harness = serde_json::to_value(harness)
            .ok()
            .and_then(|v| v.as_str().map(str::to_owned))
            .unwrap_or_default();
        let models = self.list_models(&device, &harness).await;
        let model = config
            .as_ref()
            .and_then(|c| c.model.clone())
            .or_else(|| models.first().map(|m| m.id.clone()));
        let efforts = models
            .iter()
            .find(|m| Some(&m.id) == model.as_ref())
            .map(|m| {
                m.reasoning_levels
                    .iter()
                    .filter_map(|l| {
                        serde_json::from_value::<ReasoningLevel>(l.as_str().into()).ok()
                    })
                    .collect()
            })
            .unwrap_or_default();
        ChipChoices {
            models: models.into_iter().map(|m| (m.id, m.label)).collect(),
            model,
            efforts,
            effort: config.as_ref().and_then(|c| c.reasoning),
            permission: config.map(|c| c.permission_mode).unwrap_or_default(),
        }
    }

    pub async fn open_claude_panel(
        &self,
        chat_id: &str,
    ) -> (
        Result<ClaudeConfigSnapshot>,
        ChipChoices,
        Result<LocalCommandOutput>,
    ) {
        futures::join!(
            self.read_claude_config(chat_id),
            self.chip_choices(chat_id),
            self.run_claude_local_command(chat_id, "/usage", true),
        )
    }
}
