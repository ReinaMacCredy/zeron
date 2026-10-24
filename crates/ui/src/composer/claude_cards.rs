use gpui::{App, Context};
use zeron_proto::HarnessId;
use zeron_proto::app_commands::{ClaudeCardCommand, claude_card_command};
use zeron_proto::claude_config::{COMMAND_NOTE_PREFIX, CommandCard, LocalCommandOutput};
use zeron_proto::invocation::Invocation;
use zeron_rpc::methods;

use super::Composer;
use crate::state::EngineHandle;

const NO_SESSION: &str =
    "No turn has run in this chat yet, so these numbers are for a new session.";
const LOST_SESSION: &str =
    "The chat's Claude session was not found, so these numbers are for a new session.";
const FORKED: &str = "Read from a copy of this chat's saved session; Zeron's own tool server and per-run settings are not counted.";

#[derive(Clone, Debug)]
struct CardTarget {
    chat_id: String,
    host: Option<String>,
    has_session: bool,
}

impl CardTarget {
    fn params(&self) -> serde_json::Value {
        let mut params = serde_json::json!({ "chatId": self.chat_id });
        if let Some(host) = &self.host {
            params["targetDeviceId"] = host.clone().into();
        }
        params
    }
}

fn side_command(name: &str) -> Option<(&'static str, bool, bool)> {
    match name.trim_start_matches('/') {
        "context" => Some(("context", true, true)),
        "usage" | "cost" => Some(("usage", false, false)),
        "mcp" => Some(("mcp", true, false)),
        _ => None,
    }
}

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

async fn write_note(
    engine: &EngineHandle,
    target: &CardTarget,
    message_id: &str,
    headline: Option<String>,
    card: &CommandCard,
) -> Result<(), String> {
    let hint = serde_json::to_string(&card.clone().capped()).map_err(|e| e.to_string())?;
    let mut params = target.params();
    params["messageId"] = message_id.into();
    params["hint"] = hint.into();
    if let Some(text) = headline {
        params["text"] = text.into();
    }
    engine
        .client()
        .call(methods::WRITE_CHAT_NOTE, params)
        .await
        .map(drop)
        .map_err(|e| e.to_string())
}

async fn side_run(
    engine: &EngineHandle,
    target: &CardTarget,
    name: &'static str,
    connect_mcp: bool,
    live: bool,
) -> CommandCard {
    let call = |live_session: bool| {
        let mut params = target.params();
        params["command"] = format!("/{name}").into();
        params["connectMcp"] = connect_mcp.into();
        params["liveSession"] = live_session.into();
        async move {
            engine
                .client()
                .call_as::<LocalCommandOutput>(methods::RUN_CLAUDE_LOCAL_COMMAND, params)
                .await
                .map_err(|e| e.to_string())
        }
    };
    let live_session = live && target.has_session;
    let mut note = (live && !target.has_session).then_some(NO_SESSION);
    let mut reply = call(live_session).await;
    if live_session {
        note = Some(FORKED);
        if reply
            .as_ref()
            .is_err_and(|e| e.contains("No conversation found"))
        {
            reply = call(false).await;
            note = Some(LOST_SESSION);
        }
    }
    match reply {
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
    }
}

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

    fn card_target(&self, chat_id: &str, cx: &App) -> Option<CardTarget> {
        let state = self.state.read(cx);
        let chat = state.chats.iter().find(|chat| chat.id == chat_id)?;
        Some(CardTarget {
            chat_id: chat.id.clone(),
            host: (Some(&chat.device_id) != state.local_device_id.as_ref())
                .then(|| chat.device_id.clone()),
            has_session: chat
                .harness_session_id
                .as_deref()
                .is_some_and(|id| !id.is_empty()),
        })
    }

    pub(super) fn run_claude_card(&mut self, command: ClaudeCardCommand, cx: &mut Context<Self>) {
        match self
            .bound_chat_id(cx)
            .and_then(|id| self.card_target(&id, cx))
        {
            Some(target) => self.dispatch_card(target, command, cx),
            None if self.bound_chat_id(cx).is_none() => self.create_card_chat(command, cx),
            None => self.set_chat_failure("This chat is not loaded yet.".into(), cx),
        }
    }

    fn dispatch_card(
        &mut self,
        target: CardTarget,
        command: ClaudeCardCommand,
        cx: &mut Context<Self>,
    ) {
        match command {
            ClaudeCardCommand::Model(arg) => self.model_card(target, arg, cx),
            ClaudeCardCommand::TerminalOnly(name) => {
                let card = CommandCard {
                    command: format!("/{name}"),
                    ..Default::default()
                };
                let headline = format!("{COMMAND_NOTE_PREFIX}{name} · terminal only");
                self.write_card(target, Some(headline), card, cx);
            }
            ClaudeCardCommand::Context => self.side_card(target, "context", None, cx),
            ClaudeCardCommand::Usage => self.side_card(target, "usage", None, cx),
            ClaudeCardCommand::Mcp => self.side_card(target, "mcp", None, cx),
        }
    }

    fn side_card(
        &mut self,
        target: CardTarget,
        name: &str,
        retry_of: Option<String>,
        cx: &mut Context<Self>,
    ) {
        let Some((name, connect_mcp, live)) = side_command(name) else {
            return;
        };
        let Some(engine) = self.state.read(cx).engine().cloned() else {
            self.set_chat_failure("Engine not connected".into(), cx);
            return;
        };
        let pending = CommandCard {
            command: format!("/{name}"),
            pending: true,
            started_at: Some(now_ms()),
            ..Default::default()
        };
        let headline = retry_of
            .is_none()
            .then(|| format!("{COMMAND_NOTE_PREFIX}{name}"));
        let message_id = retry_of.unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
        cx.spawn(async move |_, _| {
            if let Err(error) = write_note(&engine, &target, &message_id, headline, &pending).await
            {
                tracing::warn!(%error, "Claude Code command placeholder was not written");
                return;
            }
            let card = side_run(&engine, &target, name, connect_mcp, live).await;
            if let Err(error) = write_note(&engine, &target, &message_id, None, &card).await {
                tracing::warn!(%error, "Claude Code command card was not written");
            }
        })
        .detach();
    }

    pub(crate) fn retry_claude_card(
        &mut self,
        chat_id: &str,
        message_id: String,
        command: &str,
        cx: &mut Context<Self>,
    ) {
        if let Some(target) = self.card_target(chat_id, cx) {
            self.side_card(target, command, Some(message_id), cx);
        }
    }

    fn create_card_chat(&mut self, command: ClaudeCardCommand, cx: &mut Context<Self>) {
        let Some(engine) = self.state.read(cx).engine().cloned() else {
            self.set_chat_failure("Engine not connected".into(), cx);
            return;
        };
        let resolved = self.pickers.read(cx).resolved(cx);
        let Some(config) = resolved.chat_config() else {
            self.set_chat_failure("Pick an agent first.".into(), cx);
            return;
        };
        let (chat_id, space, local, device) = {
            let state = self.state.read(cx);
            (
                state
                    .active_draft
                    .clone()
                    .unwrap_or_else(|| uuid::Uuid::new_v4().to_string()),
                state.selected_space_row().cloned(),
                state.local_device_id.clone(),
                state.effective_device_id(),
            )
        };
        let mut mutate = serde_json::json!({
            "op": "createChat",
            "chatId": chat_id,
            "config": config,
        });
        let host = match &space {
            Some(space) => {
                mutate["spaceId"] = space.id.clone().into();
                if let crate::pickers::CheckoutPlan::CurrentCheckout {
                    branch: Some(branch),
                } = self.pickers.read(cx).session_checkout_plan(cx)
                {
                    mutate["branch"] = branch.into();
                }
                space.device_id.clone()
            }
            None => {
                let device = device
                    .or_else(|| local.clone())
                    .unwrap_or_else(|| "local".into());
                let scratch = self
                    .state
                    .read(cx)
                    .device_supports(&device, zeron_proto::capabilities::PROJECTLESS_SCRATCH_V1);
                mutate["deviceId"] = device.clone().into();
                mutate["cwd"] = if scratch {
                    zeron_proto::projectless::scratch_cwd(&chat_id, chrono::Utc::now())
                } else {
                    "~".to_string()
                }
                .into();
                device
            }
        };
        let target = CardTarget {
            chat_id: chat_id.clone(),
            host: (Some(&host) != local.as_ref()).then_some(host),
            has_session: false,
        };
        self.input.update(cx, |input, cx| input.set_text("", cx));
        self.sync_draft(cx);
        cx.spawn(async move |this, cx| {
            if let Err(error) = engine.client().call(methods::MUTATE, mutate).await {
                this.update(cx, |this, cx| {
                    this.set_chat_failure(format!("Couldn't create the chat: {error}").into(), cx)
                })
                .ok();
                return;
            }
            this.update(cx, |this, cx| {
                this.state.update(cx, |state, cx| {
                    state.select_chat(Some(chat_id.clone()), cx);
                    cx.notify();
                });
                this.dispatch_card(target, command, cx);
            })
            .ok();
        })
        .detach();
    }

    fn model_card(&mut self, target: CardTarget, arg: Option<String>, cx: &mut Context<Self>) {
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
        let headline = format!(
            "{COMMAND_NOTE_PREFIX}model{}",
            card.model
                .as_deref()
                .map(|m| format!(" · {m}"))
                .unwrap_or_default()
        );
        self.write_card(target, Some(headline), card, cx);
    }

    fn write_card(
        &mut self,
        target: CardTarget,
        headline: Option<String>,
        card: CommandCard,
        cx: &mut Context<Self>,
    ) {
        let Some(engine) = self.state.read(cx).engine().cloned() else {
            return;
        };
        let message_id = uuid::Uuid::new_v4().to_string();
        cx.spawn(async move |_, _| {
            if let Err(error) = write_note(&engine, &target, &message_id, headline, &card).await {
                tracing::warn!(%error, "Claude Code command card was not written");
            }
        })
        .detach();
    }
}
