use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

use gpui::{
    AnyElement, App, Context, Div, KeyDownEvent, MouseButton, SharedString, Stateful, Task, Window,
    div, prelude::*, px,
};
use zeron_proto::{SessionStatus, UserInputAnswer};
use zeron_rpc::methods;

use super::background::{
    CardMotion, card_frame, card_motion, entering, frosted_pill, pill_enter, pill_exit, pill_frame,
    pill_leading, presence,
};
use super::{
    Composer, ComposerEvent, Wizard, WizardHost, WizardStep, pending_input_request, respond_input,
};
use crate::icons::{self, icon};
use crate::popover;
use crate::state::Indicator;
use crate::theme::Theme;

pub(super) const PLACEHOLDER: &str = "Type your own answer, or pick an option above";
const PICKED_PLACEHOLDER: &str =
    "Type your own answer, or leave this blank to use the selected option";
const PILL_TITLE_WIDTH: f32 = 220.0;
const ASK_PILL_FLOOR: f32 = 120.0;
const FETCH_ATTEMPTS: usize = 10;
const FETCH_RETRY: Duration = Duration::from_millis(400);
const TAB_TITLE_WIDTH: f32 = 180.0;

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
    opened_at: Option<Instant>,
    leaving: Option<(String, Instant)>,
    pressed_while_open: bool,
    pill_since: Option<Instant>,
    pill_label: Option<(String, bool)>,
    pill_ghost: Option<(String, bool, Instant)>,
}

impl Composer {
    /// Split panes and sidechats would repeat the main composer's pill.
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
                let pending = match fetch_pending(&engine, &id).await {
                    Ok(Some(raw)) => {
                        cx.background_executor()
                            .spawn(async move { decode_pending(raw) })
                            .await
                    }
                    Ok(None) => None,
                    Err(err) => {
                        tracing::debug!(chat_id = %id, error = %err, "ask snapshot failed");
                        None
                    }
                };
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

    pub(crate) fn ask_kinds(&self) -> HashMap<String, bool> {
        self.ask
            .entries
            .iter()
            .filter(|entry| !self.ask.answered.contains_key(&entry.wizard.request_id))
            .map(|entry| {
                let approval = entry.wizard.questions.iter().any(|q| q.approval);
                (entry.chat_id.clone(), approval)
            })
            .collect()
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

    fn visible_asks(&self, cx: &App) -> Vec<String> {
        if !self.hosts_asks() {
            return Vec::new();
        }
        let state = self.state.read(cx);
        let bound = self.bound_chat_id(cx);
        let now = chrono::Utc::now();
        self.ask
            .entries
            .iter()
            .filter(|entry| !self.ask.answered.contains_key(&entry.wizard.request_id))
            .filter(|entry| bound.as_deref() != Some(entry.chat_id.as_str()))
            .filter(|entry| state.indicator_for(&entry.chat_id, now) == Indicator::AwaitingInput)
            .map(|entry| entry.chat_id.clone())
            .collect()
    }

    fn ask_title(&self, chat_id: &str, cx: &App) -> String {
        self.state
            .read(cx)
            .chats
            .iter()
            .find(|chat| chat.id == chat_id)
            .and_then(|chat| chat.title.as_deref())
            .and_then(|title| title.lines().map(str::trim).find(|line| !line.is_empty()))
            .unwrap_or("New session")
            .to_owned()
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

    fn open_ask_tray(&mut self, chat_id: String, cx: &mut Context<Self>) {
        self.stash_ask_typed(cx);
        self.ask.advance_task = None;
        if self.ask.open.is_none() {
            self.ask.opened_at = Some(Instant::now());
        }
        self.ask.leaving = None;
        self.ask.open = Some(chat_id);
        self.load_ask_typed(cx);
        self.drop_roster();
        self.ask.focus_pending = true;
        cx.notify();
    }

    /// Defer, never cancel: the wizard keeps its page, picks, and typed text.
    pub(super) fn close_ask_tray(&mut self, cx: &mut Context<Self>) {
        let Some(chat_id) = self.ask.open.take() else {
            return;
        };
        self.stash_ask_typed(cx);
        self.ask.leaving = Some((chat_id, Instant::now()));
        self.ask.advance_task = None;
        self.focus_pending = true;
        cx.notify();
    }

    pub(super) fn drop_ask_tray(&mut self, cx: &mut Context<Self>) {
        self.close_ask_tray(cx);
        self.ask.leaving = None;
    }

    pub(crate) fn dismiss_corner_card(&mut self, cx: &mut Context<Self>) -> bool {
        if self.ask.open.is_some() {
            self.close_ask_tray(cx);
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
        match self.visible_asks(cx).into_iter().next() {
            Some(next) => {
                self.ask.open = Some(next);
                self.load_ask_typed(cx);
                self.ask.focus_pending = true;
            }
            None => {
                self.ask.leaving = Some((chat_id.clone(), Instant::now()));
                self.ask_input
                    .update(cx, |input, cx| input.set_text(String::new(), cx));
                self.focus_pending = true;
            }
        }
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

    pub(super) fn render_corner(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> (Option<AnyElement>, Option<AnyElement>) {
        let visible = self.visible_asks(cx);
        if let Some(chat_id) = self
            .ask
            .open
            .clone()
            .filter(|chat_id| !visible.contains(chat_id))
        {
            self.ask.open = None;
            self.ask.advance_task = None;
            self.ask.leaving = Some((chat_id, Instant::now()));
        }
        let (pills, roster) = self
            .render_background_corner(window, cx)
            .unwrap_or_default();
        let theme = Theme::of(cx).clone();
        let limit = (!self.side_chat)
            .then(|| self.render_limit_pill(&theme, cx))
            .flatten();
        let now = Instant::now();
        let reduced = crate::motion::reduced_motion(cx);

        let tray_motion = match (&self.ask.open, &self.ask.leaving) {
            (Some(chat_id), _) => Some((
                chat_id.clone(),
                card_motion(self.ask.opened_at.unwrap_or(now), None, reduced, now),
            )),
            (None, Some((chat_id, since))) => Some((
                chat_id.clone(),
                card_motion(
                    self.ask.opened_at.unwrap_or(*since),
                    Some(*since),
                    reduced,
                    now,
                ),
            )),
            _ => None,
        };
        let tray = tray_motion
            .filter(|(_, motion)| !motion.gone)
            .and_then(|(chat_id, motion)| {
                self.render_ask_tray(&chat_id, &visible, motion, &theme, cx)
                    .map(|tray| (tray, motion))
            });
        if tray.is_none() {
            self.ask.leaving = None;
        }
        if tray.as_ref().is_some_and(|(_, motion)| motion.live) && self.ask.focus_pending {
            self.ask.focus_pending = false;
            window.focus(&self.ask_focus, cx);
        }

        let label = match visible.as_slice() {
            [] => None,
            [one] => Some((self.ask_title(one, cx), true)),
            _ => Some((format!("{} questions", visible.len()), false)),
        };
        match (&label, self.ask.pill_label.take()) {
            (None, Some((text, single))) => self.ask.pill_ghost = Some((text, single, now)),
            (Some(_), _) => self.ask.pill_ghost = None,
            _ => {}
        }
        self.ask.pill_label = label.clone();
        let pill_in = pill_enter(
            presence(&mut self.ask.pill_since, label.is_some(), now),
            reduced,
            now,
        );
        let ghost_out = self
            .ask
            .pill_ghost
            .as_ref()
            .map(|(_, _, since)| pill_exit(*since, reduced, now));
        if ghost_out.is_some_and(|out| out >= 1.0) {
            self.ask.pill_ghost = None;
        }
        let ghost = self
            .ask
            .pill_ghost
            .as_ref()
            .zip(ghost_out)
            .map(|((text, single, _), out)| {
                let shown = 1.0 - out;
                let pill = ask_pill_body(
                    pill_frame("ask-pill-ghost", &theme).opacity(shown),
                    text.clone(),
                    *single,
                    false,
                    &theme,
                );
                frosted_pill(pill, shown).into_any_element()
            });
        let pill = label.map(|(text, single)| {
            self.render_ask_pill(&visible, text, single, pill_in, &theme, cx)
        });
        if tray.as_ref().is_some_and(|(_, motion)| motion.animating)
            || pill_in < 1.0
            || ghost.is_some()
        {
            window.request_animation_frame();
        }

        let row = (!pills.is_empty() || limit.is_some() || pill.is_some() || ghost.is_some()).then(
            || {
                div()
                    .flex()
                    .flex_row()
                    .items_end()
                    .gap(px(6.0))
                    .children(pills)
                    .child(div().flex_1().min_w_0())
                    .children(limit)
                    .children(pill)
                    .children(ghost)
                    .into_any_element()
            },
        );
        (tray.map(|(tray, _)| tray).or(roster), row)
    }

    fn render_ask_pill(
        &self,
        visible: &[String],
        text: String,
        single: bool,
        enter: f32,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let first = visible.first().cloned().unwrap_or_default();
        let pill = pill_frame("ask-pill", theme).debug_selector(|| "ask-pill".into());
        let pill = ask_pill_body(pill, text, single, self.ask.open.is_some(), theme)
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, _, _| {
                    this.ask.pressed_while_open =
                        this.ask.open.is_some() || this.ask.leaving.is_some();
                }),
            )
            .on_click(cx.listener(move |this, _, _, cx| {
                if !std::mem::take(&mut this.ask.pressed_while_open) {
                    this.open_ask_tray(first.clone(), cx);
                }
            }));
        frosted_pill(entering(pill, enter), enter).into_any_element()
    }

    fn render_ask_tray(
        &self,
        chat_id: &str,
        visible: &[String],
        motion: CardMotion,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let entry = self
            .ask
            .entries
            .iter()
            .find(|entry| entry.chat_id == chat_id)?;
        let (body, footer) =
            self.wizard_body(&entry.wizard, &self.ask_input, WizardHost::Ask, cx)?;
        let open_id = chat_id.to_owned();
        let header = div()
            .h(px(36.0))
            .flex()
            .flex_row()
            .items_center()
            .gap(px(8.0))
            .pl(px(12.0))
            .pr(px(8.0))
            .border_b_1()
            .border_color(crate::theme::hairline(0.06))
            .text_size(crate::typography::ui_rems(13.0))
            .text_color(theme.text.opacity(0.8))
            .child(question_glyph(theme))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .child(SharedString::from(self.ask_title(chat_id, cx))),
            )
            .child(
                div()
                    .id("ask-tray-open-thread")
                    .debug_selector(|| "ask-tray-open-thread".into())
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(4.0))
                    .px(px(6.0))
                    .h(px(22.0))
                    .rounded(px(6.0))
                    .cursor_pointer()
                    .text_size(crate::typography::ui_rems(12.0))
                    .text_color(theme.text_muted)
                    .hover(|s| s.bg(theme.glass_hover()).text_color(theme.text))
                    .child("Open thread")
                    .child(icon(icons::ARROW_UP_RIGHT).size(px(11.0)))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.close_ask_tray(cx);
                        cx.emit(ComposerEvent::OpenChat(open_id.clone()));
                    })),
            )
            .child(
                div()
                    .id("ask-tray-close")
                    .debug_selector(|| "ask-tray-close".into())
                    .size(px(20.0))
                    .flex_none()
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(6.0))
                    .cursor_pointer()
                    .hover(|s| s.bg(theme.glass_hover()))
                    .child(
                        icon(icons::CLOSE)
                            .size(px(12.0))
                            .text_color(theme.text_muted),
                    )
                    .on_click(cx.listener(|this, _, _, cx| this.close_ask_tray(cx))),
            );
        let tabs =
            (visible.len() > 1).then(|| {
                div()
                    .flex()
                    .flex_row()
                    .flex_wrap()
                    .gap(px(4.0))
                    .px(px(12.0))
                    .pt(px(8.0))
                    .children(visible.iter().enumerate().map(|(ix, id)| {
                        let selected = id == chat_id;
                        let target = id.clone();
                        div()
                            .id(("ask-tray-tab", ix))
                            .debug_selector(move || format!("ask-tray-tab-{ix}"))
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap(px(6.0))
                            .h(px(22.0))
                            .px(px(8.0))
                            .rounded(px(6.0))
                            .cursor_pointer()
                            .text_size(crate::typography::ui_rems(11.5))
                            .text_color(if selected {
                                theme.text
                            } else {
                                theme.text_muted
                            })
                            .when(selected, |el| el.bg(crate::theme::ink(0.08)))
                            .when(!selected, |el| el.hover(|s| s.bg(theme.glass_hover())))
                            .child(
                                div()
                                    .size(px(6.0))
                                    .flex_none()
                                    .rounded_full()
                                    .bg(theme.accent.opacity(0.8)),
                            )
                            .child(
                                div()
                                    .max_w(px(TAB_TITLE_WIDTH))
                                    .truncate()
                                    .child(SharedString::from(self.ask_title(id, cx))),
                            )
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.open_ask_tray(target.clone(), cx)
                            }))
                    }))
            });
        let tray = div()
            .id("ask-tray")
            .when(motion.live, |el| {
                el.debug_selector(|| "ask-tray".into())
                    .track_focus(&self.ask_focus)
                    .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                        this.on_wizard_key(WizardHost::Ask, event, window, cx)
                    }))
                    .on_mouse_down_out(cx.listener(|this, _, _, cx| this.close_ask_tray(cx)))
            })
            .occlude()
            .w_full()
            .rounded(px(14.0))
            .border_1()
            .border_color(theme.border)
            .bg(popover::surface_bg(theme))
            .when(!theme.is_frost(), |el| el.shadow_lg())
            .overflow_hidden()
            .flex()
            .flex_col()
            .child(header)
            .children(tabs)
            .child(body)
            .child(footer);
        Some(card_frame(tray, motion).into_any_element())
    }
}

fn ask_pill_body(
    pill: Stateful<Div>,
    text: String,
    single: bool,
    active: bool,
    theme: &Theme,
) -> Stateful<Div> {
    pill.flex_shrink(1.0)
        .min_w(px(ASK_PILL_FLOOR))
        .border_color(if active {
            theme.accent
        } else {
            theme.accent.opacity(0.45)
        })
        .text_color(theme.text)
        .child(pill_leading(question_glyph(theme)))
        .child(
            div()
                .min_w_0()
                .max_w(px(PILL_TITLE_WIDTH))
                .truncate()
                .child(SharedString::from(text)),
        )
        .when(single, |el| {
            el.child(
                div()
                    .flex_none()
                    .text_color(theme.text_muted)
                    .child("· Input"),
            )
        })
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

fn question_glyph(theme: &Theme) -> AnyElement {
    div()
        .size(px(14.0))
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .rounded_full()
        .border_1()
        .border_color(theme.accent.opacity(0.8))
        .text_size(px(9.5))
        .font_weight(gpui::FontWeight::BOLD)
        .text_color(theme.accent)
        .child("?")
        .into_any_element()
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
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            div()
                .size_full()
                .flex()
                .flex_col()
                .justify_end()
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

    fn finish_exit(cx: &mut VisualTestContext, composer: &Entity<Composer>) {
        let span = crate::motion::MENU_OUT.total() + Duration::from_millis(30);
        std::thread::sleep(span);
        cx.executor().advance_clock(span);
        composer.update(cx, |_, cx| cx.notify());
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
        let (_fixture, cx) = cx.add_window_view(|_, _| Fixture(composer.clone()));
        state.update(cx, |state, cx| {
            state.sessions = vec![awaiting("b")];
            cx.notify();
        });
        (composer, Engine { frames }, cx)
    }

    #[gpui::test]
    fn another_threads_question_is_answered_from_the_pill(cx: &mut TestAppContext) {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let _guard = runtime.enter();
        let (composer, mut engine, cx) = mount(cx, "a");

        let watches = engine.take(cx, methods::WATCH_DOC_MESSAGES);
        assert_eq!(watches.len(), 1, "{watches:?}");
        assert_eq!(watches[0]["params"]["chatId"], "b");
        assert_eq!(watches[0]["params"]["openingTail"], true);
        land(cx, &composer, "b", "req-b");
        let pill = cx.debug_bounds("ask-pill").expect("pill for thread b");
        assert!(cx.debug_bounds("ask-tray").is_none());

        cx.simulate_click(pill.center(), gpui::Modifiers::default());
        cx.run_until_parked();
        assert!(cx.debug_bounds("ask-tray").is_some());
        assert!(cx.debug_bounds("ask-pill").is_some(), "the pill stays put");

        cx.simulate_click(gpui::point(px(4.0), px(4.0)), gpui::Modifiers::default());
        cx.run_until_parked();
        assert!(
            cx.debug_bounds("ask-tray").is_none(),
            "a click outside defers"
        );
        finish_exit(cx, &composer);
        cx.simulate_click(pill.center(), gpui::Modifiers::default());
        cx.run_until_parked();
        assert!(cx.debug_bounds("ask-tray").is_some());

        cx.simulate_keystrokes("escape");
        cx.run_until_parked();
        assert!(cx.debug_bounds("ask-tray").is_none(), "escape defers");
        let pill = cx.debug_bounds("ask-pill").expect("deferred, still asking");
        finish_exit(cx, &composer);
        cx.simulate_click(pill.center(), gpui::Modifiers::default());
        cx.run_until_parked();

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
        assert!(cx.debug_bounds("ask-tray").is_none());
        assert!(cx.debug_bounds("ask-pill").is_none());
    }

    #[gpui::test]
    fn a_long_subagent_title_truncates_before_the_question_pill(cx: &mut TestAppContext) {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let _guard = runtime.enter();
        let (composer, _engine, cx) = mount(cx, "a");
        let title = "Audit pstack sub-skills against every maestro skill and router path";
        let state = cx.update(|_, cx| composer.read(cx).state.clone());
        state.update(cx, |state, cx| {
            state.transcript = vec![
                SessionMessageEntry {
                    id: "prompt".into(),
                    role: MessageRole::User,
                    parts: Vec::new(),
                    ..asking("unused").remove(0)
                },
                SessionMessageEntry {
                    id: "spawned".into(),
                    parts: vec![MessagePart::Tool {
                        id: "p1".into(),
                        call: zeron_proto::ToolCall::Unknown {
                            name: format!("Agent: {title}"),
                            input: Some(serde_json::json!({ "description": title })),
                        },
                        is_error: false,
                        resolved: true,
                        output: None,
                        diff: None,
                        output_ref: None,
                        output_bytes: None,
                        diff_ref: None,
                        diff_stats: None,
                        subagent_ref: Some("a--sub--p1".into()),
                        subagent_status: Some(zeron_doc::SubagentStatus::Running),
                        subagent_tail: None,
                        html_render: None,
                    }],
                    ..asking("unused").remove(0)
                },
            ];
            cx.notify();
        });
        land(cx, &composer, "b", "req-b");
        cx.simulate_resize(gpui::size(px(520.0), px(600.0)));
        cx.run_until_parked();
        let agents = cx
            .debug_bounds("background-agents-pill")
            .expect("agents pill");
        let ask = cx.debug_bounds("ask-pill").expect("ask pill");
        let tab = cx
            .debug_bounds("composer-sidechat-tab")
            .expect("sidechat tab");
        assert!(
            agents.right() <= ask.left() && ask.right() <= tab.left(),
            "pills overflow the row: agents {agents:?}, question {ask:?}, tab {tab:?}"
        );
    }

    #[gpui::test]
    fn the_open_threads_own_question_gets_no_pill(cx: &mut TestAppContext) {
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
        assert!(cx.debug_bounds("ask-pill").is_none());
    }

    #[gpui::test]
    fn a_pr_chats_question_shows_in_the_mini_composer_not_as_a_main_pill(cx: &mut TestAppContext) {
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
        assert!(cx.debug_bounds("ask-pill").is_none());
        assert!(cx.update(|_, cx| composer.read(cx).ask.entries.is_empty()));

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
