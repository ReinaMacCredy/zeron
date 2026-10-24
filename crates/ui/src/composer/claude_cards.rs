use gpui::{App, AppContext, Context};
use zeron_proto::HarnessId;
use zeron_proto::app_commands::{ClaudeCardCommand, claude_card_command};
use zeron_proto::claude_cards::{
    FORKED, LOST_SESSION, NO_SESSION, headline, lost_session, model_card, model_match,
    no_match_note, output_card, side_run, switched_note, terminal_card,
};
use zeron_proto::claude_config::{CommandCard, LocalCommandOutput};
use zeron_proto::invocation::Invocation;
use zeron_rpc::methods;

use super::Composer;

impl Composer {
    pub(super) fn claude_card_for(&self, plain: &str, cx: &App) -> Option<ClaudeCardCommand> {
        if self.pickers.read(cx).resolved(cx).harness != Some(HarnessId::ClaudeCode) {
            return None;
        }
        let rows = self.slash_cache.get(&self.slash.context);
        claude_card_command(plain, |name| {
            rows.is_some_and(|rows| {
                rows.iter().any(|row| {
                    matches!(&row.invocation, Invocation::Skill { name: skill, .. } if skill == name)
                })
            })
        })
    }

    pub(super) fn run_claude_card(&mut self, command: ClaudeCardCommand, cx: &mut Context<Self>) {
        let Some(chat) = self.bound_chat_row(cx).cloned() else {
            self.set_chat_failure(
                "Send a first message, then run Claude Code commands.".into(),
                cx,
            );
            return;
        };
        if Some(&chat.device_id) != self.state.read(cx).local_device_id.as_ref() {
            self.set_chat_failure(
                "Claude Code commands run on the device that hosts this chat.".into(),
                cx,
            );
            return;
        }
        let Some(run) = side_run(&command) else {
            return match command {
                ClaudeCardCommand::Model(arg) => self.model_card(chat.id, arg, cx),
                ClaudeCardCommand::TerminalOnly(name) => {
                    let (headline, card) = terminal_card(&name);
                    self.write_card(chat.id, headline, card, cx)
                }
                _ => {}
            };
        };
        let (name, connect_mcp, live) = (run.name, run.connect_mcp, run.live);
        let Some(engine) = self.state.read(cx).engine().cloned() else {
            self.set_chat_failure("Engine not connected".into(), cx);
            return;
        };
        let has_session = chat
            .harness_session_id
            .as_deref()
            .is_some_and(|id| !id.is_empty());
        let mut params = self.catalog_params(cx);
        params["command"] = format!("/{name}").into();
        params["connectMcp"] = connect_mcp.into();
        params["liveSession"] = (live && has_session).into();
        let chat_id = chat.id;
        cx.spawn(async move |this, cx| {
            let call = |params: serde_json::Value| {
                let engine = engine.clone();
                async move {
                    engine
                        .client()
                        .call_as::<LocalCommandOutput>(methods::RUN_CLAUDE_LOCAL_COMMAND, params)
                        .await
                        .map_err(|e| e.to_string())
                }
            };
            let mut note = (live && !has_session).then_some(NO_SESSION);
            let mut reply = call(params.clone()).await;
            if live && has_session {
                note = Some(FORKED);
                if lost_session(&reply) {
                    params["liveSession"] = false.into();
                    reply = call(params).await;
                    note = Some(LOST_SESSION);
                }
            }
            let card = output_card(name, reply, note);
            this.update(cx, |this, cx| {
                let headline = headline(&card);
                this.write_card(chat_id, headline, card, cx)
            })
            .ok();
        })
        .detach();
    }

    fn model_card(&mut self, chat_id: String, arg: Option<String>, cx: &mut Context<Self>) {
        let chips = self.pickers.read(cx).chip_choices(cx);
        let mut note = None;
        let mut model = chips.model.clone();
        if let Some(arg) = arg {
            match model_match(&chips.models, &arg).cloned() {
                Some((id, label)) => {
                    self.pickers
                        .update(cx, |pickers, cx| pickers.set_chip_model(id.clone(), cx));
                    note = Some(switched_note(&label));
                    model = Some(id);
                }
                None => note = Some(no_match_note(&arg)),
            }
        }
        let card = model_card(chips.models, model, note);
        let headline = headline(&card);
        self.write_card(chat_id, headline, card, cx);
    }

    fn write_card(
        &mut self,
        chat_id: String,
        text: String,
        card: CommandCard,
        cx: &mut Context<Self>,
    ) {
        let Some(engine) = self.state.read(cx).engine().cloned() else {
            return;
        };
        let Ok(hint) = serde_json::to_string(&card.capped()) else {
            return;
        };
        cx.background_spawn(async move {
            if let Err(error) = engine
                .client()
                .call(
                    methods::MUTATE,
                    serde_json::json!({
                        "op": "addChatNote",
                        "chatId": chat_id,
                        "text": text,
                        "hint": hint,
                    }),
                )
                .await
            {
                tracing::warn!(%error, "Claude Code command card was not written");
            }
        })
        .detach();
    }
}
