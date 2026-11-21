use zeron_proto::{AgentProfile, AgentProfileDraft, Bot, BotAvatar, BotPatch, ChatConfig};
use zeron_rpc::RpcError;

use super::EngineRpc;
use super::tasks::failed;
use crate::agent_profiles::normalize_draft;

fn draft_of(name: String, config: &ChatConfig) -> AgentProfileDraft {
    AgentProfileDraft {
        name,
        harness: config.harness.clone(),
        model: config.model.clone(),
        reasoning: config.reasoning,
        access: Some(config.access),
        model_options: config.model_options.clone(),
        notes: String::new(),
        instructions: config.instructions.clone(),
        disabled_tools: config.disabled_tools.clone(),
    }
}

impl EngineRpc {
    #[allow(clippy::too_many_arguments)]
    pub(super) fn create_bot(
        &self,
        bot_id: String,
        chat_id: String,
        name: String,
        mut config: ChatConfig,
        avatar: BotAvatar,
        template: Option<String>,
        space_id: Option<String>,
        device_id: Option<String>,
        pinned: bool,
    ) -> Result<(), RpcError> {
        if self.workspace.bot(&bot_id).is_some() {
            return Ok(());
        }
        let draft = normalize_draft(draft_of(name, &config)).map_err(failed)?;
        let avatar = avatar
            .normalized()
            .map_err(|part| RpcError::BadParams(format!("invalid avatar: {part}")))?;
        config.instructions = draft.instructions.clone();
        config.model = draft.model.clone();
        config.disabled_tools = draft.disabled_tools.clone();
        let now = chrono::Utc::now();
        let bot = Bot {
            profile: AgentProfile {
                id: bot_id,
                name: draft.name,
                harness: draft.harness,
                model: draft.model,
                reasoning: draft.reasoning,
                access: draft.access,
                model_options: draft.model_options,
                notes: draft.notes,
                instructions: draft.instructions,
                disabled_tools: draft.disabled_tools,
                by_agent: false,
            },
            avatar,
            chat_id: chat_id.clone(),
            pinned,
            order_key: String::new(),
            archived: false,
            unread: false,
            template,
            created_at: now,
            updated_at: now,
        };
        // (Checked before its chat is made, so a refused Bot leaves no chat behind.)
        zeron_doc::check_bot_syncs(&bot).map_err(|e| RpcError::Failed(e.to_string()))?;
        self.workspace
            .create_child_chat(
                &chat_id,
                space_id.as_deref(),
                device_id.as_deref(),
                Some(config),
                None,
                None,
                None,
            )
            .map_err(failed)?;
        match self.workspace.create_bot(&bot) {
            Ok(_) => Ok(()),
            Err(err) => {
                self.delete_chat(&chat_id).ok();
                Err(failed(err))
            }
        }
    }

    pub(super) fn update_bot(&self, bot_id: &str, mut patch: BotPatch) -> Result<(), RpcError> {
        if let Some(draft) = patch.profile.take() {
            patch.profile = Some(normalize_draft(draft).map_err(failed)?);
        }
        if let Some(avatar) = patch.avatar.take() {
            patch.avatar = Some(
                avatar
                    .normalized()
                    .map_err(|part| RpcError::BadParams(format!("invalid avatar: {part}")))?,
            );
        }
        if !self.workspace.update_bot(bot_id, &patch).map_err(failed)? {
            return Err(RpcError::Failed("Bot no longer exists".into()));
        }
        let (Some(draft), Some(bot)) = (&patch.profile, self.workspace.bot(bot_id)) else {
            return Ok(());
        };
        let Some(mut config) = self
            .workspace
            .chat(&bot.chat_id)
            .ok()
            .flatten()
            .and_then(|chat| chat.config)
        else {
            return Ok(());
        };
        let before = config.clone();
        config.harness = draft.harness.clone();
        config.model = draft.model.clone();
        config.reasoning = draft.reasoning;
        config.model_options = draft.model_options.clone();
        config.instructions = draft.instructions.clone();
        config.disabled_tools = draft.disabled_tools.clone();
        if let Some(access) = draft.access {
            config.access = access;
        }
        if config != before {
            self.workspace
                .set_chat_config(&bot.chat_id, &config)
                .map_err(failed)?;
        }
        Ok(())
    }

    pub(super) fn delete_bot(&self, bot_id: &str) -> Result<(), RpcError> {
        let Some(bot) = self.workspace.bot(bot_id) else {
            return Ok(());
        };
        self.delete_chat(&bot.chat_id)?;
        self.workspace.delete_bot(bot_id).map_err(failed).map(drop)
    }
}

#[cfg(test)]
mod tests {
    use crate::registry::HarnessRegistry;
    use futures::StreamExt;
    use serde_json::{Value, json};
    use zeron_proto::{Bot, HarnessId};
    use zeron_rpc::{Caller, RpcReply, RpcService, methods, with_caller};

    async fn core() -> (tempfile::TempDir, crate::EngineCore) {
        let temp = tempfile::tempdir().unwrap();
        let core = crate::EngineCore::assemble_with_profile(
            crate::EngineProfile::local(temp.path()).unwrap(),
            std::sync::Arc::new(HarnessRegistry::new()),
            HarnessId::Codex,
            None,
        )
        .unwrap();
        (temp, core)
    }

    fn create(core: &crate::EngineCore, bot: &str, chat: &str, name: &str) -> Value {
        json!({"op":"createBot", "botId":bot, "chatId":chat, "name":name, "deviceId":core.device_id,
            "config":{"harness":"codex","model":" gpt ","sandbox":"workspace-write","access":"supervised","instructions":"  Keep the build green.  "},
            "avatar":{"glyph":"rocket","frame":"squircle","color":"violet"}, "template":"qa"})
    }

    fn ids(bots: &[Bot]) -> Vec<&str> {
        bots.iter().map(|b| b.profile.id.as_str()).collect()
    }

    #[tokio::test]
    async fn a_bot_is_made_with_its_chat_changed_pinned_archived_and_deleted_with_it() {
        let (_temp, core) = core().await;
        let rpc = core.rpc_service();
        let RpcReply::Stream(mut watch) = rpc.handle(methods::WATCH_BOTS, json!({})).await.unwrap()
        else {
            panic!("expected a Bots watch")
        };
        assert_eq!(watch.next().await.unwrap(), json!([]));

        rpc.handle(
            methods::MUTATE,
            create(&core, "b1", "c1", "  Build keeper "),
        )
        .await
        .unwrap();
        let listed: Vec<Bot> = serde_json::from_value(watch.next().await.unwrap()).unwrap();
        assert_eq!(ids(&listed), ["b1"]);
        let made = &listed[0];
        assert_eq!(made.profile.name, "Build keeper");
        assert_eq!(made.profile.harness, HarnessId::Codex);
        assert_eq!(made.profile.model.as_deref(), Some("gpt"));
        assert_eq!(made.profile.instructions, "Keep the build green.");
        assert_eq!(
            made.profile.access,
            Some(zeron_proto::AccessMode::Supervised)
        );
        assert_eq!(
            (made.avatar.glyph.as_str(), made.template.as_deref()),
            ("rocket", Some("qa"))
        );
        assert!(!made.pinned && !made.archived && !made.unread);
        let chat = core.workspace.chat("c1").unwrap().expect("the Bot's chat");
        let config = chat.config.unwrap();
        assert_eq!(config.instructions, "Keep the build green.");
        assert_eq!(config.access, zeron_proto::AccessMode::Supervised);
        rpc.handle(methods::MUTATE, create(&core, "b1", "c1-again", "Retried"))
            .await
            .unwrap();
        assert_eq!(ids(&core.workspace.read_bots().unwrap()), ["b1"]);
        assert!(core.workspace.chat("c1-again").unwrap().is_none());

        rpc.handle(
            methods::MUTATE,
            json!({"op":"updateBot", "botId":"b1", "patch":{"pinned":true, "unread":true,
                "profile":{"name":"Release keeper","harness":"codex","model":"gpt","access":"full-access","instructions":"Ship on Fridays."}}}),
        )
        .await
        .unwrap();
        let bot = core.workspace.bot("b1").unwrap();
        assert_eq!(bot.profile.name, "Release keeper");
        assert!(bot.pinned && bot.unread);
        let config = core.workspace.chat("c1").unwrap().unwrap().config.unwrap();
        assert_eq!(config.instructions, "Ship on Fridays.");
        assert_eq!(config.access, zeron_proto::AccessMode::FullAccess);

        for (bot, chat) in [("b2", "c2"), ("b3", "c3"), ("b4", "c4")] {
            // (A millisecond apart, so "newest first" has an order to keep.)
            tokio::time::sleep(std::time::Duration::from_millis(3)).await;
            rpc.handle(methods::MUTATE, create(&core, bot, chat, bot))
                .await
                .unwrap();
        }
        rpc.handle(
            methods::MUTATE,
            json!({"op":"updateBot", "botId":"b3", "patch":{"pinned":true}}),
        )
        .await
        .unwrap();
        assert_eq!(
            ids(&core.workspace.read_bots().unwrap()),
            ["b1", "b3", "b4", "b2"]
        );
        rpc.handle(
            methods::MUTATE,
            json!({"op":"updateBot", "botId":"b1", "patch":{"pinned":false}}),
        )
        .await
        .unwrap();
        assert_eq!(
            ids(&core.workspace.read_bots().unwrap()),
            ["b3", "b4", "b2", "b1"]
        );

        rpc.handle(
            methods::MUTATE,
            json!({"op":"updateBot", "botId":"b1", "patch":{"archived":true}}),
        )
        .await
        .unwrap();
        assert!(core.workspace.bot("b1").unwrap().archived);
        assert!(core.workspace.chat("c1").unwrap().is_some());

        rpc.handle(methods::MUTATE, json!({"op":"deleteBot", "botId":"b1"}))
            .await
            .unwrap();
        assert_eq!(
            ids(&core.workspace.read_bots().unwrap()),
            ["b3", "b4", "b2"]
        );
        assert!(core.workspace.chat("c1").unwrap().is_none());
        assert!(core.workspace.chat("c3").unwrap().is_some());
        let gone = rpc
            .handle(
                methods::MUTATE,
                json!({"op":"updateBot", "botId":"b1", "patch":{"unread":true}}),
            )
            .await;
        assert!(gone.is_err());
        rpc.handle(methods::MUTATE, json!({"op":"deleteBot", "botId":"b1"}))
            .await
            .unwrap();
        core.sessions.shutdown().await;
    }

    #[tokio::test]
    async fn a_bot_that_cannot_sync_is_refused_and_leaves_no_chat() {
        let (_temp, core) = core().await;
        let rpc = core.rpc_service();
        let mut long = create(&core, "b1", "c1", "Long");
        long["config"]["instructions"] = json!("x".repeat(15_900));
        let refused = rpc
            .handle(methods::MUTATE, long)
            .await
            .err()
            .map(|e| e.to_string())
            .unwrap_or_default();
        assert!(refused.contains("shorten its instructions"), "{refused}");
        let mut nameless = create(&core, "b2", "c2", "   ");
        nameless["avatar"]["glyph"] = json!("rocket");
        assert!(rpc.handle(methods::MUTATE, nameless).await.is_err());
        let mut odd = create(&core, "b3", "c3", "Odd");
        odd["avatar"]["image"] = json!("https://example.com/a.png");
        assert!(rpc.handle(methods::MUTATE, odd).await.is_err());
        assert!(core.workspace.read_bots().unwrap().is_empty());
        for chat in ["c1", "c2", "c3"] {
            assert!(
                core.workspace.chat(chat).unwrap().is_none(),
                "{chat} was left behind"
            );
        }
        core.sessions.shutdown().await;
    }

    #[tokio::test]
    async fn an_agent_cannot_make_change_or_delete_a_bot() {
        let (_temp, core) = core().await;
        let rpc = core.rpc_service();
        rpc.handle(methods::MUTATE, create(&core, "b1", "c1", "Mine"))
            .await
            .unwrap();
        let agent = || Caller::Agent {
            chat: Some("c1".into()),
        };
        for op in [
            create(&core, "b2", "c2", "Theirs"),
            json!({"op":"updateBot", "botId":"b1", "patch":{"profile":{"name":"Theirs","harness":"codex"}}}),
            json!({"op":"updateBot", "botId":"b1", "patch":{"archived":true}}),
            json!({"op":"deleteBot", "botId":"b1"}),
        ] {
            let refused = with_caller(agent(), rpc.handle(methods::MUTATE, op.clone())).await;
            assert!(refused.is_err(), "{op}");
        }
        let bots = core.workspace.read_bots().unwrap();
        assert_eq!(ids(&bots), ["b1"]);
        assert_eq!(bots[0].profile.name, "Mine");
        assert!(!bots[0].archived);
        core.sessions.shutdown().await;
    }
}
