use std::collections::{HashMap, HashSet};
use std::time::Duration;

use gpui::{
    AnyElement, Context, Div, KeyDownEvent, SharedString, Task, Window, div, prelude::*, px,
};
use zeron_proto::{SessionStatus, UserInputAnswer};
use zeron_rpc::methods;

use super::{Composer, Wizard, WizardHost, WizardStep, pending_input_request, respond_input};
use crate::icons::{self, icon};
use crate::theme::Theme;

pub(super) const PLACEHOLDER: &str = "Type your own answer, or pick an option above";
const PICKED_PLACEHOLDER: &str =
    "Type your own answer, or leave this blank to use the selected option";
const FETCH_ATTEMPTS: usize = 10;
const FETCH_RETRY: Duration = Duration::from_millis(400);

pub(super) fn placeholder(picked: bool) -> &'static str {
    if picked {
        PICKED_PLACEHOLDER
    } else {
        PLACEHOLDER
    }
}

struct AskEntry {
    chat_id: String,
    wizard: Wizard,
}

#[derive(Default)]
pub(super) struct AskPills {
    awaiting: HashSet<String>,
    fetches: HashMap<String, Task<()>>,
    entries: Vec<AskEntry>,
    pub(super) open: Option<String>,
    answered: HashMap<String, String>,
    submits: HashMap<String, Task<()>>,
    pub(super) advance_task: Option<Task<()>>,
    focus_pending: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct AskInfo {
    pub page: usize,
    pub total: usize,
    pub question: String,
    pub plan: bool,
}

impl Composer {
    /// Split panes and sidechats would track the same questions twice.
    fn hosts_asks(&self) -> bool {
        self.chat_override.is_none() && !self.side_chat
    }

    pub(super) fn sync_asks(&mut self, cx: &mut Context<Self>) {
        if !self.hosts_asks() {
            return;
        }
        let state = self.state.read(cx);
        let awaiting: HashSet<String> = state
            .sessions
            .iter()
            .filter(|session| session.status == SessionStatus::AwaitingInput)
            .filter(|session| !state.is_hidden_chat(&session.chat_id))
            .map(|session| session.chat_id.clone())
            .collect();
        if awaiting == self.ask.awaiting {
            return;
        }
        let entered: Vec<String> = awaiting.difference(&self.ask.awaiting).cloned().collect();
        self.ask
            .entries
            .retain(|entry| awaiting.contains(&entry.chat_id));
        self.ask
            .fetches
            .retain(|chat_id, _| awaiting.contains(chat_id));
        self.ask
            .answered
            .retain(|_, chat_id| awaiting.contains(chat_id));
        if self
            .ask
            .open
            .as_ref()
            .is_some_and(|chat_id| !awaiting.contains(chat_id))
        {
            self.ask.open = None;
            self.ask.advance_task = None;
        }
        self.ask.awaiting = awaiting;
        for chat_id in entered {
            self.fetch_ask(chat_id, cx);
        }
        cx.notify();
    }

    fn fetch_ask(&mut self, chat_id: String, cx: &mut Context<Self>) {
        let Some(engine) = self.state.read(cx).engine().cloned() else {
            return;
        };
        let id = chat_id.clone();
        let task = cx.spawn(async move |this, cx| {
            // The status flips to AwaitingInput before the Input part commits.
            for _ in 0..FETCH_ATTEMPTS {
                let pending = pending_ask(&engine, &id, cx.background_executor()).await;
                let landed = this.update(cx, |composer, cx| {
                    if !composer.ask.awaiting.contains(&id) {
                        return true;
                    }
                    match pending {
                        Some((request_id, questions)) => {
                            composer.land_ask(id.clone(), Wizard::new(request_id, questions), cx);
                            true
                        }
                        None => false,
                    }
                });
                match landed {
                    Ok(false) => cx.background_executor().timer(FETCH_RETRY).await,
                    _ => break,
                }
            }
            this.update(cx, |composer, _| composer.ask.fetches.remove(&id))
                .ok();
        });
        self.ask.fetches.insert(chat_id, task);
    }

    fn land_ask(&mut self, chat_id: String, wizard: Wizard, cx: &mut Context<Self>) {
        if !self.ask.awaiting.contains(&chat_id)
            || self.ask.answered.contains_key(&wizard.request_id)
        {
            return;
        }
        self.ask.entries.retain(|entry| entry.chat_id != chat_id);
        self.ask.entries.push(AskEntry { chat_id, wizard });
        cx.notify();
    }

    fn live_entry(&self, chat_id: &str) -> Option<&AskEntry> {
        self.ask
            .entries
            .iter()
            .find(|entry| entry.chat_id == chat_id)
            .filter(|entry| !self.ask.answered.contains_key(&entry.wizard.request_id))
    }

    pub(crate) fn ask_info(&self, chat_id: &str) -> Option<AskInfo> {
        let wizard = &self.live_entry(chat_id)?.wizard;
        Some(AskInfo {
            page: wizard.page,
            total: wizard.questions.len(),
            question: wizard
                .current()
                .map(|question| question.question.clone())
                .unwrap_or_default(),
            plan: wizard
                .questions
                .iter()
                .any(|question| question.approval && question.header == "Approve plan"),
        })
    }

    pub(crate) fn ask_answered(&self, chat_id: &str) -> bool {
        self.ask.answered.values().any(|id| id == chat_id)
    }

    pub(crate) fn ask_signature(&self) -> Vec<(String, usize, bool)> {
        self.ask
            .entries
            .iter()
            .map(|entry| {
                (
                    entry.chat_id.clone(),
                    entry.wizard.page,
                    self.ask.answered.contains_key(&entry.wizard.request_id),
                )
            })
            .collect()
    }

    pub(super) fn host_wizard(&mut self, host: WizardHost) -> Option<&mut Wizard> {
        match host {
            WizardHost::Own => self.wizard.as_mut(),
            WizardHost::Ask => self.open_ask_entry().map(|entry| &mut entry.wizard),
        }
    }

    fn open_ask_entry(&mut self) -> Option<&mut AskEntry> {
        let chat_id = self.ask.open.as_deref()?;
        self.ask
            .entries
            .iter_mut()
            .find(|entry| entry.chat_id == chat_id)
    }

    fn stash_ask_typed(&mut self, cx: &mut Context<Self>) {
        let text = self.ask_input.read(cx).text().to_string();
        if let Some(entry) = self.open_ask_entry() {
            entry.wizard.set_typed(text);
        }
    }

    fn load_ask_typed(&mut self, cx: &mut Context<Self>) {
        let (text, picked) = self
            .open_ask_entry()
            .map(|entry| {
                let wizard = &entry.wizard;
                (
                    wizard.typed.get(wizard.page).cloned().unwrap_or_default(),
                    wizard.page_has_pick(),
                )
            })
            .unwrap_or_default();
        self.ask_input.update(cx, |input, cx| {
            input.set_text(text, cx);
            input.set_placeholder(placeholder(picked), cx);
        });
    }

    pub(crate) fn open_ask(&mut self, chat_id: String, cx: &mut Context<Self>) {
        if self.ask.open.as_deref() == Some(chat_id.as_str()) {
            return;
        }
        self.stash_ask_typed(cx);
        self.ask.advance_task = None;
        self.ask.open = Some(chat_id);
        self.load_ask_typed(cx);
        self.ask.focus_pending = true;
        cx.notify();
    }

    /// Defer, never cancel: the wizard keeps its page, picks, and typed text.
    pub(crate) fn close_ask(&mut self, cx: &mut Context<Self>) {
        if self.ask.open.is_none() {
            return;
        }
        self.stash_ask_typed(cx);
        self.ask.open = None;
        self.ask.advance_task = None;
        cx.notify();
    }

    pub(crate) fn dismiss_corner_card(&mut self, cx: &mut Context<Self>) -> bool {
        if self.config_panel_open() {
            self.close_config_panel(cx);
            return true;
        }
        if self.roster_open() {
            self.close_roster(cx);
            return true;
        }
        false
    }

    pub(super) fn ask_advance(&mut self, cx: &mut Context<Self>) {
        let text = self.ask_input.read(cx).text().to_string();
        let Some(entry) = self.open_ask_entry() else {
            return;
        };
        let multiline = entry
            .wizard
            .current()
            .is_some_and(|question| question.multiline);
        if !entry.wizard.page_has_pick() && text.trim().is_empty() && !multiline {
            return;
        }
        entry.wizard.set_typed(text);
        match entry.wizard.advance() {
            WizardStep::Done(answers) => self.ask_finish(answers, cx),
            _ => {
                self.load_ask_typed(cx);
                cx.notify();
            }
        }
    }

    pub(super) fn ask_back(&mut self, cx: &mut Context<Self>) {
        self.stash_ask_typed(cx);
        if let Some(entry) = self.open_ask_entry()
            && entry.wizard.back()
        {
            self.load_ask_typed(cx);
            cx.notify();
        }
    }

    pub(super) fn ask_finish(&mut self, answers: Vec<UserInputAnswer>, cx: &mut Context<Self>) {
        let Some(chat_id) = self.ask.open.clone() else {
            return;
        };
        let Some(request_id) = self
            .open_ask_entry()
            .map(|entry| entry.wizard.request_id.clone())
        else {
            return;
        };
        self.ask.advance_task = None;
        self.ask
            .answered
            .insert(request_id.clone(), chat_id.clone());
        self.ask.open = None;
        self.ask_input
            .update(cx, |input, cx| input.set_text(String::new(), cx));
        cx.notify();
        let Some(engine) = self.state.read(cx).engine().cloned() else {
            return;
        };
        let key = request_id.clone();
        let task = cx.spawn(async move |this, cx| {
            let result = respond_input(&engine, &chat_id, &request_id, answers).await;
            this.update(cx, |composer, cx| {
                composer.ask.submits.remove(&request_id);
                if let Err(err) = result {
                    composer.ask.answered.remove(&request_id);
                    composer.failure = Some(format!("Answer failed: {err}").into());
                    composer.failure_key = None;
                    cx.notify();
                }
            })
            .ok();
        });
        self.ask.submits.insert(key, task);
    }

    pub(crate) fn render_ask_panel(
        &mut self,
        chat_id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        if self.ask.open.as_deref() != Some(chat_id) {
            self.open_ask(chat_id.to_owned(), cx);
        }
        let theme = Theme::of(cx).clone();
        let entry = self.live_entry(chat_id)?;
        let (body, footer) = match entry.wizard.current().filter(|q| q.is_secret()) {
            Some(question) => (secret_body(question, &theme), div()),
            None => self.wizard_body(&entry.wizard, &self.ask_input, WizardHost::Ask, cx)?,
        };
        if std::mem::take(&mut self.ask.focus_pending) {
            window.focus(&self.ask_focus, cx);
        }
        Some(
            div()
                .id("ask-panel")
                .debug_selector(|| "ask-panel".into())
                .role(gpui::Role::Group)
                .aria_label("Question")
                .track_focus(&self.ask_focus)
                .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                    this.on_wizard_key(WizardHost::Ask, event, window, cx)
                }))
                .w_full()
                .flex()
                .flex_col()
                .child(body)
                .child(footer)
                .into_any_element(),
        )
    }
}

fn secret_body(question: &zeron_proto::UserInputQuestion, theme: &Theme) -> Div {
    div()
        .px(px(16.0))
        .py(px(14.0))
        .flex()
        .flex_col()
        .gap(px(6.0))
        .child(
            div()
                .text_size(crate::typography::ui_rems(15.0))
                .font_weight(gpui::FontWeight::MEDIUM)
                .text_color(theme.text)
                .child(SharedString::from(question.header.clone())),
        )
        .child(
            div()
                .text_size(crate::typography::ui_rems(13.0))
                .text_color(theme.text_muted)
                .child(SharedString::from(question.question.clone())),
        )
        .child(
            div()
                .mt(px(4.0))
                .flex()
                .flex_row()
                .items_center()
                .gap(px(6.0))
                .text_size(crate::typography::ui_rems(12.0))
                .text_color(theme.text_muted)
                .child(
                    icon(icons::LUCIDE_SHIELD)
                        .size(px(13.0))
                        .text_color(theme.text_muted),
                )
                .child("Open the thread to enter it. It stays out of chat."),
        )
}

async fn pending_ask(
    engine: &crate::state::EngineHandle,
    chat_id: &str,
    executor: &gpui::BackgroundExecutor,
) -> Option<(String, Vec<zeron_proto::UserInputQuestion>)> {
    match fetch_pending(engine, chat_id).await {
        Ok(Some(raw)) => executor.spawn(async move { decode_pending(raw) }).await,
        Ok(None) => None,
        Err(err) => {
            tracing::debug!(chat_id = %chat_id, error = %err, "ask snapshot failed");
            None
        }
    }
}

pub(crate) async fn pending_questions(
    engine: &crate::state::EngineHandle,
    chat_id: &str,
    executor: &gpui::BackgroundExecutor,
) -> Option<Vec<zeron_proto::UserInputQuestion>> {
    for _ in 0..FETCH_ATTEMPTS {
        if let Some((_, questions)) = pending_ask(engine, chat_id, executor).await {
            return Some(questions);
        }
        executor.timer(FETCH_RETRY).await;
    }
    None
}

/// Dropping the scoped subscription after one frame stops the engine building the full mirror.
async fn fetch_pending(
    engine: &crate::state::EngineHandle,
    chat_id: &str,
) -> Result<Option<serde_json::Value>, String> {
    let params = serde_json::json!({ "chatId": chat_id, "openingTail": true });
    let mut subscription = engine
        .client()
        .subscribe_scoped(methods::WATCH_DOC_MESSAGES, params)
        .await
        .map_err(|err| err.to_string())?;
    Ok(subscription.recv().await)
}

fn decode_pending(raw: serde_json::Value) -> Option<(String, Vec<zeron_proto::UserInputQuestion>)> {
    let update: zeron_doc::TranscriptUpdate = serde_json::from_value(raw).ok()?;
    let mut rows = Vec::new();
    zeron_doc::apply_transcript_frame(&mut rows, update.frame).ok()?;
    pending_input_request(&rows)
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{Entity, Render, TestAppContext, VisualTestContext};
    use zeron_doc::{MessagePart, MessageRole, SessionCommandPayload, SessionMessageEntry};
    use zeron_proto::{Session, UserInputQuestion};

    use crate::composer::AUTO_ADVANCE_MS;
    use crate::state::{AppState, EngineHandle};

    struct Fixture(Entity<Composer>);

    impl Render for Fixture {
        fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            let open = self.0.read(cx).ask.open.clone();
            let panel = open.and_then(|chat_id| {
                self.0.update(cx, |composer, cx| {
                    composer.render_ask_panel(&chat_id, window, cx)
                })
            });
            div()
                .size_full()
                .flex()
                .flex_col()
                .justify_end()
                .children(panel)
                .child(self.0.clone())
        }
    }

    struct Engine {
        frames: tokio::sync::mpsc::Receiver<String>,
    }

    impl Engine {
        fn take(&mut self, cx: &mut VisualTestContext, method: &str) -> Vec<serde_json::Value> {
            cx.run_until_parked();
            let mut out = Vec::new();
            while let Ok(frame) = self.frames.try_recv() {
                let frame: serde_json::Value = serde_json::from_str(&frame).unwrap();
                if frame["method"] == method {
                    out.push(frame);
                }
            }
            out
        }
    }

    /// The test scheduler rejects wakes from the RPC client's tokio reader thread.
    fn land(
        cx: &mut VisualTestContext,
        composer: &Entity<Composer>,
        chat_id: &str,
        request_id: &str,
    ) {
        let (request_id, questions) =
            pending_input_request(&asking(request_id)).expect("pending question");
        composer.update(cx, |composer, cx| {
            composer.land_ask(chat_id.into(), Wizard::new(request_id, questions), cx)
        });
        cx.run_until_parked();
    }

    fn chat(id: &str) -> zeron_proto::Chat {
        serde_json::from_value(serde_json::json!({
            "id": id,
            "deviceId": "mac",
            "title": format!("Thread {id}"),
            "archived": false,
            "createdAt": "2026-09-23T00:00:00Z"
        }))
        .unwrap()
    }

    fn awaiting(chat_id: &str) -> Session {
        let now = chrono::Utc::now();
        Session {
            last_completed_turn: None,
            chat_id: chat_id.into(),
            device_id: "mac".into(),
            status: SessionStatus::AwaitingInput,
            held_by_subagents: false,
            compacting: false,
            background_tasks: Vec::new(),
            started_at: Some(now),
            updated_at: now,
            running_subagents: 0,
            compact_plan: None,
            classifying: None,
            last_turn_outcome: Default::default(),
            terminal_activity: Default::default(),
        }
    }

    fn asking(request_id: &str) -> Vec<SessionMessageEntry> {
        vec![SessionMessageEntry {
            id: "assistant".into(),
            role: MessageRole::Assistant,
            parts: vec![MessagePart::Input {
                id: "input".into(),
                request_id: request_id.into(),
                questions: vec![UserInputQuestion {
                    approval: false,
                    id: "side".into(),
                    header: "Conflict".into(),
                    question: "Which side wins?".into(),
                    options: vec!["Fork".into(), "Upstream".into()],
                    multi_select: false,
                    kind: None,
                    prefill: None,
                    multiline: false,
                }],
                resolved: false,
                answers: None,
                asked_at_ms: None,
                answered_at_ms: None,
            }],
            created_at: 0,
            device_id: "mac".into(),
            status: Some(zeron_doc::MessageStatus::Streaming),
            continuation_of: None,
            duration_ms: None,
            from_chat_id: None,
            fork_import: false,
            answers_request_id: None,
        }]
    }

    fn mount<'a>(
        cx: &'a mut TestAppContext,
        selected: &str,
    ) -> (Entity<Composer>, Engine, &'a mut VisualTestContext) {
        let dir = tempfile::tempdir().unwrap();
        cx.update(|cx| {
            gpui_base::init(cx);
            cx.set_global(Theme::dark());
            crate::app_menus::init(cx);
            crate::history::init(
                Default::default(),
                Default::default(),
                Default::default(),
                Default::default(),
                cx,
            );
            crate::settings::init(crate::settings::UiSettings::default(), dir.path(), cx);
        });
        std::mem::forget(dir);
        let (out, frames) = tokio::sync::mpsc::channel::<String>(64);
        let (replies, inbound) = tokio::sync::mpsc::channel::<String>(64);
        std::mem::forget(replies);
        let state = cx.new(|_| AppState::new());
        let selected = selected.to_owned();
        state.update(cx, |state, _| {
            state.set_test_engine(EngineHandle::from_test_client(zeron_rpc::RpcClient::new(
                out, inbound,
            )));
            state.chats = vec![chat("a"), chat("b")];
            state.selected_chat = Some(selected);
            state.local_device_id = Some("mac".into());
        });
        let composer = cx.new(|cx| Composer::new(state.clone(), cx));
        let (_fixture, cx) = cx.add_window_view(|_, cx| {
            cx.observe(&composer, |_, _, cx| cx.notify()).detach();
            Fixture(composer.clone())
        });
        state.update(cx, |state, cx| {
            state.sessions = vec![awaiting("b")];
            cx.notify();
        });
        (composer, Engine { frames }, cx)
    }

    #[gpui::test]
    fn another_threads_question_is_answered_from_its_panel(cx: &mut TestAppContext) {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let _guard = runtime.enter();
        let (composer, mut engine, cx) = mount(cx, "a");

        let watches = engine.take(cx, methods::WATCH_DOC_MESSAGES);
        assert_eq!(watches.len(), 1, "{watches:?}");
        assert_eq!(watches[0]["params"]["chatId"], "b");
        assert_eq!(watches[0]["params"]["openingTail"], true);
        land(cx, &composer, "b", "req-b");
        assert!(cx.debug_bounds("ask-panel").is_none());
        let info = cx.update(|_, cx| composer.read(cx).ask_info("b"));
        assert_eq!(info.map(|info| (info.page, info.total)), Some((0, 1)));

        composer.update(cx, |composer, cx| composer.open_ask("b".into(), cx));
        cx.run_until_parked();
        assert!(cx.debug_bounds("ask-panel").is_some());

        cx.simulate_keystrokes("2");
        cx.executor()
            .advance_clock(Duration::from_millis(AUTO_ADVANCE_MS + 20));
        let answers = engine.take(cx, methods::QUEUE_COMMAND);
        assert_eq!(answers.len(), 1, "{answers:?}");
        assert_eq!(answers[0]["params"]["chatId"], "b");
        let command = serde_json::to_value(SessionCommandPayload::RespondInput {
            request_id: "req-b".into(),
            answers: vec![UserInputAnswer {
                question_id: "side".into(),
                labels: vec!["Upstream".into()],
            }],
        })
        .unwrap();
        assert_eq!(answers[0]["params"]["command"], command);
        assert!(cx.debug_bounds("ask-panel").is_none());
        assert!(cx.update(|_, cx| composer.read(cx).ask_answered("b")));
    }

    #[gpui::test]
    fn the_open_threads_own_question_still_lands(cx: &mut TestAppContext) {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let _guard = runtime.enter();
        let (composer, mut engine, cx) = mount(cx, "b");

        let watches = engine.take(cx, methods::WATCH_DOC_MESSAGES);
        assert_eq!(watches.len(), 1, "{watches:?}");
        land(cx, &composer, "b", "req-b");
        assert_eq!(
            cx.update(|_, cx| composer.read(cx).ask.entries.len()),
            1,
            "the question landed"
        );
    }

    #[gpui::test]
    fn a_pr_chats_question_shows_in_the_mini_composer_not_the_tray(cx: &mut TestAppContext) {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let _guard = runtime.enter();
        let (composer, mut engine, cx) = mount(cx, "a");
        engine.take(cx, methods::WATCH_DOC_MESSAGES);
        let state = cx.update(|_, cx| composer.read(cx).state.clone());
        state.update(cx, |state, cx| {
            state.sessions.clear();
            cx.notify();
        });
        cx.run_until_parked();
        state.update(cx, |state, cx| {
            state.set_hidden_chats(["b".to_string()].into(), cx);
            state.sessions = vec![awaiting("b")];
            cx.notify();
        });
        assert!(engine.take(cx, methods::WATCH_DOC_MESSAGES).is_empty());
        land(cx, &composer, "b", "req-b");
        assert!(cx.update(|_, cx| composer.read(cx).ask_info("b").is_none()));

        let mini_state = cx.new(|_| AppState::new());
        mini_state.update(cx, |state, cx| {
            state.chats = vec![chat("b")];
            state.selected_chat = Some("b".into());
            state.transcript = asking("req-b");
            cx.notify();
        });
        let mini = cx.new(|cx| {
            let mut composer = Composer::new(mini_state.clone(), cx);
            composer.set_pr_chat("Ask or hand off work for this PR...", cx);
            composer
        });
        mini_state.update(cx, |_, cx| cx.notify());
        cx.run_until_parked();
        mini.read_with(cx, |mini, _| {
            let wizard = mini
                .wizard
                .as_ref()
                .expect("the question is answered in the panel");
            assert_eq!(wizard.request_id, "req-b");
            assert_eq!(wizard.questions[0].options, ["Fork", "Upstream"]);
        });
    }
}
