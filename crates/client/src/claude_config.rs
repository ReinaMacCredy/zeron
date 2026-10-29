use zeron_proto::WorkspaceFileText;
use zeron_proto::app_commands::ClaudeCardCommand;
use zeron_proto::claude_cards::{
    FORKED, LOST_SESSION, NO_SESSION, SideRun, lost_session, model_card, model_headline,
    model_match, no_match_note, output_card, pending_card, side_command, side_headline, side_run,
    switched_note, terminal_card,
};
use zeron_proto::claude_config::{ClaudeConfigSnapshot, CommandCard, LocalCommandOutput};
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
        live_session: bool,
    ) -> Result<LocalCommandOutput> {
        let (device, mut params) = self.claude_target(chat_id)?;
        params["command"] = command.into();
        params["connectMcp"] = connect_mcp.into();
        params["liveSession"] = live_session.into();
        self.host_call(&device, methods::RUN_CLAUDE_LOCAL_COMMAND, params)
            .await
    }

    pub async fn write_chat_note(
        &self,
        chat_id: &str,
        message_id: &str,
        text: Option<&str>,
        hint: Option<&str>,
    ) -> Result<()> {
        let (device, mut params) = self.claude_target(chat_id)?;
        params["messageId"] = message_id.into();
        if let Some(text) = text {
            params["text"] = text.into();
        }
        if let Some(hint) = hint {
            params["hint"] = hint.into();
        }
        self.inner
            .host_rpc(&device, methods::WRITE_CHAT_NOTE, params)
            .await
            .map(drop)
    }

    pub async fn add_chat_note(&self, chat_id: &str, text: &str) -> Result<()> {
        self.write_chat_note(chat_id, &crate::new_id(), Some(text), None)
            .await
    }

    async fn write_card(
        &self,
        chat_id: &str,
        message_id: &str,
        headline: Option<&str>,
        card: CommandCard,
    ) -> Result<()> {
        let hint = serde_json::to_string(&card.capped())
            .map_err(|e| ClientError::Internal(e.to_string()))?;
        self.write_chat_note(chat_id, message_id, headline, Some(&hint))
            .await
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
            permission: config.map(|c| c.access).unwrap_or_default(),
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
            self.run_claude_local_command(chat_id, "/usage", true, false),
        )
    }

    pub async fn run_claude_card(
        &self,
        chat_id: &str,
        command: ClaudeCardCommand,
    ) -> Result<Option<(String, String)>> {
        let message_id = crate::new_id();
        let Some(run) = side_run(&command) else {
            let mut switch = None;
            let (headline, card) = match command {
                ClaudeCardCommand::Model(arg) => {
                    let chips = self.chip_choices(chat_id).await;
                    let mut model = chips.model.clone();
                    let note = arg.map(|arg| match model_match(&chips.models, &arg) {
                        Some((id, label)) => {
                            switch = Some((id.clone(), label.clone()));
                            model = Some(id.clone());
                            switched_note(label)
                        }
                        None => no_match_note(&arg),
                    });
                    let card = model_card(chips.models, model, note);
                    (model_headline(&card), card)
                }
                ClaudeCardCommand::TerminalOnly(name) => terminal_card(&name),
                _ => return Ok(None),
            };
            self.write_card(chat_id, &message_id, Some(&headline), card)
                .await?;
            return Ok(switch);
        };
        self.side_card(chat_id, &message_id, run, true).await?;
        Ok(None)
    }

    pub async fn retry_claude_card(
        &self,
        chat_id: &str,
        message_id: &str,
        command: &str,
    ) -> Result<()> {
        let run = side_command(command)
            .ok_or_else(|| ClientError::NotFound(format!("{command} can't be retried")))?;
        self.side_card(chat_id, message_id, run, false).await
    }

    async fn side_card(
        &self,
        chat_id: &str,
        message_id: &str,
        run: SideRun,
        headline: bool,
    ) -> Result<()> {
        let now = chrono::Utc::now().timestamp_millis();
        self.write_card(
            chat_id,
            message_id,
            headline.then(|| side_headline(run.name)).as_deref(),
            pending_card(run.name, now),
        )
        .await?;
        let has_session = self
            .chat(chat_id)
            .and_then(|chat| chat.harness_session_id)
            .is_some_and(|id| !id.is_empty());
        let live = run.live && has_session;
        let command = format!("/{}", run.name);
        let mut note = (run.live && !has_session).then_some(NO_SESSION);
        let mut reply = self
            .run_claude_local_command(chat_id, &command, run.connect_mcp, live)
            .await
            .map_err(|e| e.to_string());
        if live {
            note = Some(FORKED);
            if lost_session(&reply) {
                reply = self
                    .run_claude_local_command(chat_id, &command, run.connect_mcp, false)
                    .await
                    .map_err(|e| e.to_string());
                note = Some(LOST_SESSION);
            }
        }
        self.write_card(
            chat_id,
            message_id,
            None,
            output_card(run.name, reply, note),
        )
        .await
    }

    pub async fn claude_settings_file(&self, chat_id: &str) -> Result<WorkspaceFileText> {
        let snapshot = self.read_claude_config(chat_id).await?;
        if snapshot.user_settings.is_empty() {
            return Err(ClientError::NotFound(
                "Claude Code has no settings file here".into(),
            ));
        }
        let (device, mut params) = self.claude_target(chat_id)?;
        params["path"] = snapshot.user_settings.into();
        self.host_call(&device, methods::READ_WORKSPACE_FILE, params)
            .await
    }
}
