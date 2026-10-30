use gpui::{App, Context};
use zeron_proto::HarnessId;
use zeron_proto::app_commands::{ClaudeCardCommand, claude_card_command};
use zeron_proto::claude_cards::{
    FORKED, LOST_SESSION, NO_SESSION, lost_session, model_card, model_headline, model_match,
    no_match_note, output_card, pending_card, side_command, side_headline, switched_note,
    terminal_card,
};
use zeron_proto::claude_config::{CommandCard, LocalCommandOutput};
use zeron_proto::invocation::Invocation;
use zeron_rpc::methods;

use super::Composer;
use crate::state::EngineHandle;

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
        if lost_session(&reply) {
            reply = call(false).await;
            note = Some(LOST_SESSION);
        }
    }
    output_card(name, reply, note)
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
        if command == ClaudeCardCommand::Usage {
            cx.emit(super::ComposerEvent::OpenUsage);
            return;
        }
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
                let (headline, card) = terminal_card(&name);
                self.write_card(target, Some(headline), card, cx);
            }
            ClaudeCardCommand::Context => self.side_card(target, "context", None, cx),
            ClaudeCardCommand::Usage => cx.emit(super::ComposerEvent::OpenUsage),
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
        let Some(run) = side_command(name) else {
            return;
        };
        let (name, connect_mcp, live) = (run.name, run.connect_mcp, run.live);
        let Some(engine) = self.state.read(cx).engine().cloned() else {
            self.set_chat_failure("Engine not connected".into(), cx);
            return;
        };
        let pending = pending_card(name, now_ms());
        let headline = retry_of.is_none().then(|| side_headline(name));
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
        let headline = model_headline(&card);
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
