use gpui::{App, AppContext, Context};
use zeron_proto::HarnessId;
use zeron_proto::app_commands::{ClaudeCardCommand, claude_card_command};
use zeron_proto::claude_config::{COMMAND_NOTE_PREFIX, CommandCard, LocalCommandOutput};
use zeron_proto::invocation::Invocation;
use zeron_rpc::methods;

use super::Composer;

const NO_SESSION: &str =
    "No turn has run in this chat yet, so these numbers are for a new session.";
const LOST_SESSION: &str =
    "The chat's Claude session was not found, so these numbers are for a new session.";
const FORKED: &str = "Read from a copy of this chat's saved session; Zeron's own tool server and per-run settings are not counted.";

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
        let (name, connect_mcp, live) = match command {
            ClaudeCardCommand::Model(arg) => return self.model_card(chat.id, arg, cx),
            ClaudeCardCommand::TerminalOnly(name) => {
                let card = CommandCard {
                    command: format!("/{name}"),
                    ..Default::default()
                };
                return self.write_card(
                    chat.id,
                    format!("{COMMAND_NOTE_PREFIX}{name} · terminal only"),
                    card,
                    cx,
                );
            }
            ClaudeCardCommand::Context => ("context", true, true),
            ClaudeCardCommand::Usage => ("usage", false, false),
            ClaudeCardCommand::Mcp => ("mcp", true, false),
        };
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
                if reply
                    .as_ref()
                    .is_err_and(|e| e.contains("No conversation found"))
                {
                    params["liveSession"] = false.into();
                    reply = call(params).await;
                    note = Some(LOST_SESSION);
                }
            }
            let card = match reply {
                Ok(out) => CommandCard {
                    command: format!("/{name}"),
                    mcp_servers: (name == "mcp")
                        .then(|| out.init.as_ref()?.get("mcp_servers").cloned())
                        .flatten(),
                    text: out.text,
                    note: note.map(str::to_owned),
                    ..Default::default()
                },
                Err(error) => CommandCard {
                    command: format!("/{name}"),
                    error: Some(error),
                    ..Default::default()
                },
            };
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
            let wanted = arg.to_lowercase();
            let found = chips
                .models
                .iter()
                .find(|(id, label)| id.to_lowercase() == wanted || label.to_lowercase() == wanted)
                .or_else(|| {
                    chips.models.iter().find(|(id, label)| {
                        id.to_lowercase().contains(&wanted)
                            || label.to_lowercase().contains(&wanted)
                    })
                })
                .cloned();
            match found {
                Some((id, label)) => {
                    self.pickers
                        .update(cx, |pickers, cx| pickers.set_chip_model(id.clone(), cx));
                    note = Some(format!("Switched this chat to {label}."));
                    model = Some(id);
                }
                None => note = Some(format!("No model matches \u{201c}{arg}\u{201d}.")),
            }
        }
        let card = CommandCard {
            command: "/model".into(),
            models: chips.models,
            model,
            note,
            ..Default::default()
        };
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

fn headline(card: &CommandCard) -> String {
    let detail = match card.command.as_str() {
        _ if card.error.is_some() => Some("failed".to_string()),
        "/context" => card
            .text
            .lines()
            .find_map(|l| l.trim().strip_prefix("**Tokens:**"))
            .map(|t| t.trim().to_string()),
        "/usage" => card
            .text
            .lines()
            .find(|l| l.contains("% used"))
            .map(|l| l.trim().to_string()),
        "/model" => card.model.clone(),
        _ => None,
    };
    let name = card.command.trim_start_matches('/');
    match detail {
        Some(detail) => format!("{COMMAND_NOTE_PREFIX}{name} · {detail}"),
        None => format!("{COMMAND_NOTE_PREFIX}{name}"),
    }
}
