use std::cell::Cell;
use std::collections::{BTreeSet, HashMap, HashSet};
use std::rc::Rc;
use std::time::Instant;

use super::*;
use crate::pull_requests::{ContextChip, PrChatContext, Suggestion};
use crate::settings::{PrChatCardRef, PrChatHistory, PrChatPosted, PrChatSession};

pub(super) const PR_CHAT_DOCK_WIDTH: f32 = 680.0;
pub(super) const PR_CHAT_DOCK_INSET: f32 = 16.0;
pub(super) const PR_CHAT_SHEET_CAP: f32 = 0.6;
const PR_CHAT_HEADER_HEIGHT: f32 = 38.0;
const PR_CHAT_REPLY_MIN: f32 = 120.0;
const PR_CHAT_PREVIOUS_OPACITY: f32 = 0.45;
const PR_CHAT_TRANSCRIPT_YIELD: f32 = 1000.0;

pub(super) struct PrChatPanel {
    pub(super) context: PrChatContext,
    removed: BTreeSet<ContextChip>,
    pub(super) chat_id: String,
    pub(super) side: Option<u64>,
    indexed: bool,
    fresh: bool,
    resumed: bool,
    error: Option<SharedString>,
    folded: bool,
    pinned: bool,
    shown: bool,
    motion: Option<(Instant, bool)>,
    sheet_height: Rc<Cell<f32>>,
    earlier: Option<SharedString>,
    previous: Option<PreviousChat>,
    sessions_menu: bool,
    cards: HashMap<PrChatCardRef, CardState>,
    confirm: Option<PrChatCardRef>,
    parsed: std::cell::RefCell<Option<(String, String, Vec<Suggestion>)>>,
    _location: Option<Task<()>>,
    _context: Option<Task<()>>,
    _saved: Option<Subscription>,
    _sent: Option<Subscription>,
}

impl PrChatPanel {
    fn new(context: PrChatContext, chat_id: String, fresh: bool) -> Self {
        Self {
            context,
            removed: BTreeSet::new(),
            chat_id,
            side: None,
            indexed: false,
            fresh,
            resumed: false,
            error: None,
            folded: false,
            pinned: false,
            shown: false,
            motion: None,
            sheet_height: Rc::default(),
            earlier: None,
            previous: None,
            sessions_menu: false,
            cards: HashMap::new(),
            confirm: None,
            parsed: Default::default(),
            _location: None,
            _context: None,
            _saved: None,
            _sent: None,
        }
    }
}

struct PreviousChat {
    row: zeron_proto::Chat,
    tab: SideChatTab,
    idle: SharedString,
}

#[derive(Default)]
pub(super) struct PrChatTrash {
    parked: Vec<ParkedPrChat>,
    deleted: HashSet<String>,
    pruned: Option<u64>,
    posting: HashSet<(String, PrChatCardRef)>,
    locations: HashMap<String, zeron_proto::ChangeRequestChat>,
}

struct ParkedPrChat {
    chat_id: String,
    side: u64,
    landing: Landing,
    _settled: Subscription,
}

enum Landing {
    Delete,
    Index(String, bool),
    Promote,
}

#[derive(Default)]
struct CardState {
    editor: Option<Entity<ComposerInput>>,
    error: Option<SharedString>,
    posted: Option<PrChatPosted>,
    dismissed: bool,
}

struct SessionRow {
    chat_id: String,
    label: String,
    detail: String,
    state: Option<(String, bool)>,
    current: bool,
}

impl Shell {
    pub(super) fn toggle_pr_chat(&mut self, cx: &mut Context<Self>) {
        if self.pr_chat_sheet_open(cx) {
            self.fold_pr_chat(cx);
        } else {
            self.reveal_pr_chat(cx);
        }
    }

    pub(super) fn reveal_pr_chat(&mut self, cx: &mut Context<Self>) {
        self.sync_pr_chat_target(true, cx);
        let kept = self.pr_chat.as_ref().is_some_and(|panel| {
            !crate::pull_requests::live_chat_sessions(
                &panel.context.index_key(),
                &self.state.read(cx).chats,
                None,
                cx,
            )
            .is_empty()
        });
        if let Some(panel) = self.pr_chat.as_mut() {
            panel.folded = false;
            panel.pinned = kept;
        }
        self.focus_pr_chat(cx);
        self.sync_pr_chat_button(cx);
        cx.notify();
    }

    pub(super) fn sync_pr_chat_target(&mut self, check_expiry: bool, cx: &mut Context<Self>) {
        let Some(page) = self.pull_requests_page.clone() else {
            return;
        };
        if !matches!(self.route, Route::PullRequests) || page.read(cx).git_tab_active() {
            return;
        }
        let scope = page.read(cx).chat_scope();
        let current = self
            .pr_chat
            .as_ref()
            .filter(|panel| panel.context.index_key() == scope);
        let Some(panel) = current else {
            let context = page.read(cx).chat_context(cx);
            if self
                .pr_chat
                .as_ref()
                .is_some_and(|panel| panel.context.index_key() == context.index_key())
            {
                return;
            }
            let reveal = self.pr_chat_sheet_open(cx);
            self.retarget_pr_chat(context, reveal, cx);
            return;
        };
        if check_expiry && self.open_pr_chat_idle_expired(cx) {
            let context = panel.context.clone();
            let reveal = self.pr_chat_sheet_open(cx);
            self.retarget_pr_chat(context, reveal, cx);
            return;
        }
        if !matches!(panel.context, PrChatContext::List(_)) {
            return;
        }
        let context = page.read(cx).chat_context(cx);
        if context.index_key() != scope || context == panel.context {
            return;
        }
        let unanchored = !panel.indexed && panel.context.location_call().is_none();
        let draft = self
            .pr_chat_tab()
            .is_some_and(|tab| tab.composer.read(cx).has_draft(cx));
        if unanchored && context.location_call().is_some() && !draft {
            self.retarget_pr_chat(context, false, cx);
        } else if let Some(panel) = self.pr_chat.as_mut() {
            panel.context = context;
            if !panel.resumed {
                self.push_pr_chat_context(cx);
            }
            cx.notify();
        }
    }

    fn retarget_pr_chat(
        &mut self,
        context: PrChatContext,
        reveal_previous: bool,
        cx: &mut Context<Self>,
    ) {
        self.release_pr_chat(cx);
        let newest = self.newest_pr_chat_session(&context.index_key(), cx);
        match newest {
            Some((row, created_at)) if !self.pr_chat_idle_expired(&row.id, created_at, cx) => {
                self.resume_pr_chat(context, row, cx)
            }
            Some((row, created_at)) => {
                self.start_pr_chat(context, cx);
                self.attach_previous_pr_chat(row, created_at, reveal_previous, cx);
            }
            None => self.start_pr_chat(context, cx),
        }
        self.sync_pr_chat_button(cx);
        cx.notify();
    }

    fn attach_previous_pr_chat(
        &mut self,
        row: zeron_proto::Chat,
        created_at: chrono::DateTime<Utc>,
        reveal: bool,
        cx: &mut Context<Self>,
    ) {
        if self.pr_chat.is_none() {
            return;
        }
        let last = row.last_message_at.unwrap_or(created_at);
        let idle = format!("idle {}", idle_span(Utc::now() - last)).into();
        let context = self.pr_chat.as_ref().map(|panel| panel.context.clone());
        let tab = self.new_side_chat_tab(row.clone(), false, cx);
        tab.transcript
            .update(cx, |transcript, _| transcript.set_send_runway(false));
        if let Some(context) = context {
            hide_card_fences(&tab, context, cx);
        }
        if let Some(panel) = self.pr_chat.as_mut() {
            panel.folded = !reveal;
            panel.previous = Some(PreviousChat { row, tab, idle });
        }
        self.sync_hidden_chats(cx);
    }

    fn resume_previous_pr_chat(&mut self, cx: &mut Context<Self>) {
        let Some(row) = self
            .pr_chat
            .as_mut()
            .and_then(|panel| panel.previous.take())
            .map(|previous| previous.row)
        else {
            return;
        };
        self.switch_pr_chat_session(row.id, cx);
    }

    fn newest_pr_chat_session(
        &self,
        scope: &str,
        cx: &App,
    ) -> Option<(zeron_proto::Chat, chrono::DateTime<Utc>)> {
        let mut sessions = settings::pr_chat_sessions(scope, cx);
        sessions.sort_by_key(|session| std::cmp::Reverse(session.created_at));
        let state = self.state.read(cx);
        sessions.iter().find_map(|session| {
            state
                .chats
                .iter()
                .find(|chat| chat.id == session.chat_id && !chat.archived)
                .map(|chat| (chat.clone(), session.created_at))
        })
    }

    fn pr_chat_idle_expired(
        &self,
        chat_id: &str,
        created_at: chrono::DateTime<Utc>,
        cx: &App,
    ) -> bool {
        self.pr_chat_time_left(chat_id, created_at, cx)
            .is_some_and(|left| left < chrono::Duration::zero())
    }

    fn open_pr_chat_idle_expired(&self, cx: &App) -> bool {
        let Some(panel) = self.pr_chat.as_ref().filter(|panel| panel.indexed) else {
            return false;
        };
        if self
            .pr_chat_tab()
            .is_some_and(|tab| tab.composer.read(cx).is_sending())
        {
            return false;
        }
        settings::pr_chat_session(&panel.chat_id, cx).is_some_and(|session| {
            self.pr_chat_idle_expired(&panel.chat_id, session.created_at, cx)
        })
    }

    fn resume_pr_chat(
        &mut self,
        context: PrChatContext,
        row: zeron_proto::Chat,
        cx: &mut Context<Self>,
    ) {
        let placeholder = context.placeholder();
        let earlier = settings::pr_chat_idle_limit(cx)
            .is_none()
            .then(|| earlier_session(&row, chrono::Local::now()))
            .flatten();
        let chat_id = row.id.clone();
        let fresh = settings::current(cx).pr_chat_fresh(&chat_id);
        let tab = self.new_side_chat_tab(row, false, cx);
        tab.composer.update(cx, |composer, cx| {
            composer.set_pr_chat(placeholder, cx);
            composer.set_pr_context(None);
        });
        tab.transcript
            .update(cx, |transcript, _| transcript.set_send_runway(false));
        hide_card_fences(&tab, context.clone(), cx);
        let sent = Self::watch_pr_chat_sends(&tab, cx);
        self.side_chat_seq += 1;
        let id = self.side_chat_seq;
        self.side_chats.insert(id, tab);
        let mut panel = PrChatPanel::new(context, chat_id, fresh);
        panel.side = Some(id);
        panel.indexed = true;
        panel.resumed = true;
        panel.earlier = earlier;
        panel._sent = Some(sent);
        self.pr_chat = Some(panel);
        self.sync_hidden_chats(cx);
        self.focus_pr_chat(cx);
    }

    fn watch_pr_chat_sends(tab: &SideChatTab, cx: &mut Context<Self>) -> Subscription {
        cx.subscribe(
            &tab.composer,
            |shell, composer, event: &ComposerEvent, cx| {
                let current = shell
                    .pr_chat_tab()
                    .is_some_and(|tab| tab.composer == composer);
                if current
                    && matches!(
                        event,
                        ComposerEvent::Sent { .. } | ComposerEvent::Queued { .. }
                    )
                    && let Some(panel) = shell.pr_chat.as_mut()
                {
                    panel.folded = false;
                    panel.earlier = None;
                    panel.previous = None;
                    shell.sync_pr_chat_button(cx);
                    cx.notify();
                }
            },
        )
    }

    fn pr_chat_session_rows(&self, cx: &App) -> Vec<SessionRow> {
        let Some(panel) = self.pr_chat.as_ref() else {
            return Vec::new();
        };
        let now = chrono::Local::now();
        let state = self.state.read(cx);
        let mut sessions = crate::pull_requests::live_chat_sessions(
            &panel.context.index_key(),
            &state.chats,
            Some(&panel.chat_id),
            cx,
        );
        sessions.sort_by_key(|session| std::cmp::Reverse(session.created_at));
        let turns = |chat_id: &str| {
            let tab = match &panel.previous {
                Some(previous) if previous.row.id == chat_id => Some(&previous.tab),
                _ if chat_id == panel.chat_id => self.pr_chat_tab(),
                _ => None,
            }?;
            Some(user_turns(&tab.state.read(cx).transcript))
        };
        let mut rows: Vec<SessionRow> = sessions
            .iter()
            .filter_map(|session| {
                let chat = state.chats.iter().find(|chat| chat.id == session.chat_id);
                if chat.is_some_and(|chat| chat.archived) {
                    return None;
                }
                let at = chat
                    .and_then(|chat| chat.last_message_at)
                    .unwrap_or(session.created_at);
                let title = chat
                    .and_then(|chat| chat.title.as_deref())
                    .map(str::trim)
                    .filter(|title| !title.is_empty())
                    .unwrap_or("Untitled chat");
                let mut detail = when_label(at, now);
                if let Some(turns) = turns(&session.chat_id).filter(|turns| *turns > 0) {
                    detail.push_str(&format!(" · {}", turn_count(turns)));
                }
                Some(SessionRow {
                    chat_id: session.chat_id.clone(),
                    label: title.to_owned(),
                    detail,
                    state: self
                        .pr_chat_time_left(&session.chat_id, session.created_at, cx)
                        .map(|left| left_label(left)),
                    current: session.chat_id == panel.chat_id,
                })
            })
            .collect();
        if !rows.iter().any(|row| row.current) {
            rows.insert(
                0,
                SessionRow {
                    chat_id: panel.chat_id.clone(),
                    label: "New chat".into(),
                    detail: "Not sent yet".into(),
                    state: None,
                    current: true,
                },
            );
        }
        rows
    }

    fn pr_chat_time_left(
        &self,
        chat_id: &str,
        created_at: chrono::DateTime<Utc>,
        cx: &App,
    ) -> Option<chrono::Duration> {
        let limit = settings::pr_chat_idle_limit(cx)?;
        let state = self.state.read(cx);
        let busy = state.session_for(chat_id).is_some_and(|session| {
            matches!(
                session.status,
                zeron_proto::SessionStatus::Working | zeron_proto::SessionStatus::AwaitingInput
            )
        });
        if busy {
            return None;
        }
        let last = state
            .chats
            .iter()
            .find(|chat| chat.id == chat_id)
            .and_then(|chat| chat.last_message_at)
            .unwrap_or(created_at);
        Some(limit - (Utc::now() - last))
    }

    fn pr_chat_header_status(&self, cx: &App) -> Option<String> {
        let panel = self.pr_chat.as_ref()?;
        if panel.previous.is_some() {
            return Some("new chat".into());
        }
        if !panel.indexed {
            return None;
        }
        let created_at = settings::pr_chat_session(&panel.chat_id, cx)?.created_at;
        self.pr_chat_time_left(&panel.chat_id, created_at, cx)
            .map(|left| left_label(left).0)
    }

    fn toggle_pr_chat_sessions_menu(&mut self, cx: &mut Context<Self>) {
        if let Some(panel) = self.pr_chat.as_mut() {
            panel.sessions_menu = !panel.sessions_menu;
        }
        cx.notify();
    }

    pub(super) fn show_pr_chat_sessions(&mut self, cx: &mut Context<Self>) {
        if let Some(panel) = self.pr_chat.as_mut() {
            panel.sessions_menu = true;
        }
        cx.notify();
    }

    fn switch_pr_chat_session(&mut self, chat_id: String, cx: &mut Context<Self>) {
        let Some(panel) = self.pr_chat.as_mut() else {
            return;
        };
        panel.sessions_menu = false;
        if panel.chat_id == chat_id {
            cx.notify();
            return;
        }
        let context = panel.context.clone();
        let row = self
            .state
            .read(cx)
            .chats
            .iter()
            .find(|chat| chat.id == chat_id && !chat.archived)
            .cloned();
        self.discard_pr_chat(cx);
        match row {
            Some(row) => self.resume_pr_chat(context, row, cx),
            None => self.start_pr_chat(context, cx),
        }
        self.sync_pr_chat_button(cx);
        cx.notify();
    }

    fn new_pr_chat_session(&mut self, cx: &mut Context<Self>) {
        let open = self.pr_chat_sheet_open(cx);
        let Some(panel) = self.pr_chat.as_mut() else {
            return;
        };
        panel.sessions_menu = false;
        let unsent = !panel.indexed;
        let context = panel.context.clone();
        let sending = self
            .pr_chat_tab()
            .is_some_and(|tab| tab.composer.read(cx).is_sending());
        if unsent && !sending {
            if let Some(panel) = self.pr_chat.as_mut() {
                panel.previous = None;
                panel.pinned |= open;
            }
            self.focus_pr_chat(cx);
            self.sync_pr_chat_button(cx);
            cx.notify();
            return;
        }
        self.release_pr_chat(cx);
        self.start_pr_chat(context, cx);
        if let Some(panel) = self.pr_chat.as_mut() {
            panel.pinned = open;
        }
        self.sync_pr_chat_button(cx);
        cx.notify();
    }

    pub(super) fn debug_send_pr_chat(&mut self, cx: &mut Context<Self>) -> bool {
        let on_pr = self
            .pr_chat
            .as_ref()
            .is_some_and(|panel| matches!(panel.context, PrChatContext::PullRequest(_)));
        let Some(composer) = self
            .pr_chat_tab()
            .filter(|_| on_pr)
            .map(|tab| tab.composer.clone())
        else {
            return false;
        };
        composer.update(cx, |composer, cx| {
            composer.send_prompt(
                "What does this pull request change, and is it ready to merge?".into(),
                cx,
            )
        });
        true
    }

    fn pr_chat_promotable(&self, cx: &App) -> bool {
        self.pr_chat_tab()
            .is_some_and(|tab| !tab.state.read(cx).side_chat_unsaved())
    }

    pub(super) fn promote_pr_chat(&mut self, cx: &mut Context<Self>) {
        if !self.pr_chat_promotable(cx) {
            return;
        }
        let Some(panel) = self.pr_chat.take() else {
            return;
        };
        let Some(side) = panel.side else {
            return;
        };
        let chat_id = panel.chat_id.clone();
        let (sending, prompt) = self
            .side_chats
            .get(&side)
            .map(|tab| {
                (
                    tab.composer.read(cx).is_sending(),
                    crate::pull_requests::first_chat_prompt(&tab.state.read(cx).transcript),
                )
            })
            .unwrap_or_default();
        let title = panel.context.promoted_title(prompt.as_deref());
        if sending {
            self.park_pr_chat(chat_id.clone(), side, Landing::Promote, cx);
        } else {
            self.side_chats.remove(&side);
        }
        self.pr_chat_trash.deleted.remove(&chat_id);
        settings::update(SavePolicy::Immediate, cx, |settings| {
            settings.unindex_pr_chat(&chat_id)
        });
        self.sync_hidden_chats(cx);
        self.sync_pr_chat_button(cx);
        if let Some(engine) = self.state.read(cx).engine().cloned() {
            let params =
                serde_json::json!({ "op": "renameChat", "chatId": chat_id, "title": title });
            Self::spawn_mutate(engine, params, cx).detach();
        }
        self.close_pull_requests(cx);
        self.open_chat(chat_id, cx);
    }

    pub(super) fn fold_pr_chat(&mut self, cx: &mut Context<Self>) {
        if let Some(panel) = self.pr_chat.as_mut() {
            panel.folded = true;
            panel.sessions_menu = false;
        }
        self.sync_pr_chat_button(cx);
        cx.notify();
    }

    fn sync_pr_chat_button(&self, cx: &mut Context<Self>) {
        let open = self.pr_chat_sheet_open(cx);
        if let Some(page) = &self.pull_requests_page {
            page.update(cx, |page, cx| page.set_chat_open(open, cx));
        }
    }

    pub(super) fn pr_chat_sheet_open(&self, cx: &App) -> bool {
        let Some(panel) = self.pr_chat.as_ref().filter(|panel| !panel.folded) else {
            return false;
        };
        panel.indexed
            || panel.pinned
            || panel.previous.is_some()
            || self.pr_chat_tab().is_some_and(|tab| {
                let state = tab.state.read(cx);
                !state.transcript.is_empty()
                    || !state.pending_echoes().is_empty()
                    || tab.composer.read(cx).is_sending()
            })
    }

    pub(super) fn pr_chat_tab(&self) -> Option<&SideChatTab> {
        let id = self.pr_chat.as_ref()?.side?;
        self.side_chats.get(&id)
    }

    fn focus_pr_chat(&self, cx: &mut Context<Self>) {
        if let Some(tab) = self.pr_chat_tab() {
            tab.composer.update(cx, |composer, cx| {
                composer.focus_pending = true;
                cx.notify();
            });
        }
    }

    fn start_pr_chat(&mut self, context: PrChatContext, cx: &mut Context<Self>) {
        let Some(engine) = self.state.read(cx).engine().cloned() else {
            return;
        };
        let chat_id = uuid::Uuid::new_v4().to_string();
        let location = context.location_call();
        let calls = (context.detail_call(), context.diff_call());
        let location_key = location.as_ref().map(|call| {
            format!(
                "{}|{}|{}|{}",
                call.params["host"],
                call.params["repository"],
                call.params["targetDeviceId"],
                self.state.read(cx).link_roots_revision
            )
        });
        let known = location_key
            .as_ref()
            .and_then(|key| self.pr_chat_trash.locations.get(key))
            .cloned();
        let location_engine = engine.clone();
        let location_task = match (&known, location) {
            (Some(_), _) | (None, None) => None,
            (None, Some(call)) => Some(cx.spawn(async move |this, cx| {
                let resolved = location_engine
                    .client()
                    .call(call.method, call.params)
                    .await
                    .ok()
                    .and_then(|value| {
                        serde_json::from_value::<zeron_proto::ChangeRequestChat>(value).ok()
                    });
                this.update(cx, |shell, cx| {
                    if let (Some(key), Some(resolved)) = (location_key, &resolved) {
                        shell.pr_chat_trash.locations.insert(key, resolved.clone());
                    }
                    shell.mint_pr_chat(resolved, cx)
                })
                .ok();
            })),
        };
        let context_task = cx.spawn(async move |this, cx| {
            let (detail, diff) = calls;
            let call = |call: Option<crate::pull_requests::ContextCall>| {
                let engine = engine.clone();
                async move {
                    let call = call?;
                    engine.client().call(call.method, call.params).await.ok()
                }
            };
            let (detail, diff) = futures::join!(call(detail), call(diff));
            this.update(cx, |shell, cx| {
                let Some(panel) = shell.pr_chat.as_mut() else {
                    return;
                };
                if let Some(detail) = detail.and_then(|value| serde_json::from_value(value).ok()) {
                    panel.context.set_detail(detail);
                }
                if let Some(slice) = diff.and_then(|value| {
                    serde_json::from_value::<zeron_proto::ChangeRequestDiffSlice>(value).ok()
                }) {
                    panel.context.set_patch(&slice.patch);
                }
                shell.push_pr_chat_context(cx);
                cx.notify();
            })
            .ok();
        });
        let fresh = settings::current(cx).pr_chat_history == PrChatHistory::Fresh;
        let unresolved = location_task.is_none() && known.is_none();
        let mut panel = PrChatPanel::new(context, chat_id, fresh);
        panel._location = location_task;
        panel._context = Some(context_task);
        self.pr_chat = Some(panel);
        self.sync_hidden_chats(cx);
        if known.is_some() || unresolved {
            self.mint_pr_chat(known, cx);
        }
    }

    fn mint_pr_chat(
        &mut self,
        location: Option<zeron_proto::ChangeRequestChat>,
        cx: &mut Context<Self>,
    ) {
        let Some(panel) = self.pr_chat.as_ref() else {
            return;
        };
        let state = self.state.read(cx);
        let device = panel
            .context
            .device()
            .or_else(|| state.local_device_id.clone())
            .unwrap_or_else(|| "local".into());
        let space_id = location.as_ref().and_then(|l| l.space_id.clone());
        let head = panel.context.head_ref();
        let worktree = space_id.as_deref().zip(head).and_then(|(space, head)| {
            state
                .chats
                .iter()
                .filter(|chat| {
                    !chat.archived
                        && chat.space_id.as_deref() == Some(space)
                        && chat.cwd.is_some()
                        && chat.cwd.as_deref() != location.as_ref().map(|l| l.root.as_str())
                        && (chat.branch.as_deref() == Some(head)
                            || chat
                                .source_context
                                .as_ref()
                                .is_some_and(|source| source.branch == head))
                })
                .max_by_key(|chat| chat.created_at)
                .map(|chat| (chat.cwd.clone(), Some(head.to_owned())))
        });
        let (cwd, branch) = match (worktree, &location) {
            (Some(found), _) => found,
            (None, Some(location)) if space_id.is_some() => (Some(location.cwd.clone()), None),
            _ => (Some("~".to_string()), None),
        };
        let chat = serde_json::from_value::<zeron_proto::Chat>(serde_json::json!({
            "id": panel.chat_id,
            "deviceId": device,
            "spaceId": space_id,
            "cwd": cwd,
            "branch": branch,
            "archived": false,
            "createdAt": Utc::now(),
            "roomGen": 2,
        }));
        let chat = match chat {
            Ok(chat) => chat,
            Err(error) => {
                if let Some(panel) = self.pr_chat.as_mut() {
                    panel.error = Some(error.to_string().into());
                }
                cx.notify();
                return;
            }
        };
        let placeholder = panel.context.placeholder();
        let context = panel.context.clone();
        let tab = self.new_side_chat_tab(chat, true, cx);
        tab.composer
            .update(cx, |composer, cx| composer.set_pr_chat(placeholder, cx));
        tab.transcript
            .update(cx, |transcript, _| transcript.set_send_runway(false));
        hide_card_fences(&tab, context, cx);
        let saved = cx.observe(&tab.state, |this, _, cx| this.index_saved_pr_chat(cx));
        let sent = Self::watch_pr_chat_sends(&tab, cx);
        self.side_chat_seq += 1;
        let id = self.side_chat_seq;
        self.side_chats.insert(id, tab);
        if let Some(panel) = self.pr_chat.as_mut() {
            panel.side = Some(id);
            panel._saved = Some(saved);
            panel._sent = Some(sent);
        }
        self.push_pr_chat_context(cx);
        self.focus_pr_chat(cx);
        cx.notify();
    }

    fn index_saved_pr_chat(&mut self, cx: &mut Context<Self>) {
        let Some(panel) = self.pr_chat.as_ref().filter(|panel| !panel.indexed) else {
            return;
        };
        if self
            .pr_chat_tab()
            .is_none_or(|tab| tab.state.read(cx).side_chat_unsaved())
        {
            return;
        }
        let chat_id = panel.chat_id.clone();
        let scope = panel.context.index_key();
        let fresh = panel.fresh;
        if let Some(panel) = self.pr_chat.as_mut() {
            panel.indexed = true;
        }
        settings::update(SavePolicy::Debounced, cx, |settings| {
            settings.index_pr_chat(
                &scope,
                PrChatSession {
                    chat_id,
                    created_at: Utc::now(),
                    posted: Vec::new(),
                    dismissed: Vec::new(),
                    fresh,
                },
            )
        });
        self.sync_hidden_chats(cx);
    }

    fn push_pr_chat_context(&self, cx: &mut Context<Self>) {
        let Some(panel) = self.pr_chat.as_ref() else {
            return;
        };
        let block = panel.context.block(&panel.removed);
        if let Some(tab) = self.pr_chat_tab() {
            tab.composer
                .update(cx, |composer, _| composer.set_pr_context(block));
        }
    }

    fn remove_pr_chat_chip(&mut self, chip: ContextChip, cx: &mut Context<Self>) {
        if let Some(panel) = self.pr_chat.as_mut() {
            panel.removed.insert(chip);
        }
        self.push_pr_chat_context(cx);
        cx.notify();
    }

    fn release_pr_chat(&mut self, cx: &mut Context<Self>) {
        if settings::current(cx).pr_chat_history == PrChatHistory::Fresh
            && let Some(panel) = self.pr_chat.as_mut()
        {
            let chat_id = panel.chat_id.clone();
            if panel.fresh && settings::current(cx).pr_chat_indexed(&chat_id) {
                if let Some(side) = panel.side.take() {
                    self.side_chats.remove(&side);
                }
                self.delete_pr_chat(chat_id, cx);
            }
        }
        self.discard_pr_chat(cx);
    }

    fn park_pr_chat(
        &mut self,
        chat_id: String,
        side: u64,
        landing: Landing,
        cx: &mut Context<Self>,
    ) {
        let Some(tab) = self.side_chats.get(&side) else {
            return;
        };
        let settled = cx.observe(&tab.composer, move |shell, composer, cx| {
            if !composer.read(cx).is_sending() {
                shell.settle_parked_pr_chat(side, cx);
            }
        });
        self.pr_chat_trash.parked.push(ParkedPrChat {
            chat_id,
            side,
            landing,
            _settled: settled,
        });
    }

    fn settle_parked_pr_chat(&mut self, side: u64, cx: &mut Context<Self>) {
        let Some(at) = self
            .pr_chat_trash
            .parked
            .iter()
            .position(|parked| parked.side == side)
        else {
            return;
        };
        let parked = self.pr_chat_trash.parked.remove(at);
        let saved = self
            .side_chats
            .remove(&side)
            .is_some_and(|tab| !tab.state.read(cx).side_chat_unsaved());
        if saved {
            match parked.landing {
                Landing::Delete => self.delete_pr_chat(parked.chat_id, cx),
                Landing::Promote => {}
                Landing::Index(scope, fresh) => {
                    settings::update(SavePolicy::Debounced, cx, |settings| {
                        settings.index_pr_chat(
                            &scope,
                            PrChatSession {
                                chat_id: parked.chat_id,
                                created_at: Utc::now(),
                                posted: Vec::new(),
                                dismissed: Vec::new(),
                                fresh,
                            },
                        )
                    });
                }
            }
        }
        self.sync_hidden_chats(cx);
    }

    fn delete_pr_chat(&mut self, chat_id: String, cx: &mut Context<Self>) {
        self.pr_chat_trash.deleted.insert(chat_id.clone());
        settings::update(SavePolicy::Debounced, cx, |settings| {
            settings.unindex_pr_chat(&chat_id)
        });
        let Some(engine) = self.state.read(cx).engine().cloned() else {
            return;
        };
        let params = serde_json::json!({ "op": "deleteChat", "chatId": chat_id });
        Self::spawn_mutate(engine, params, cx).detach();
    }

    fn discard_pr_chat(&mut self, cx: &mut Context<Self>) {
        if let Some(panel) = self.pr_chat.take()
            && let Some(side) = panel.side
        {
            let sending = self
                .side_chats
                .get(&side)
                .is_some_and(|tab| tab.composer.read(cx).is_sending());
            if sending {
                let fresh_mode = settings::current(cx).pr_chat_history == PrChatHistory::Fresh;
                let landing = match fresh_mode && panel.fresh {
                    true => Landing::Delete,
                    false => Landing::Index(panel.context.index_key(), panel.fresh),
                };
                self.park_pr_chat(panel.chat_id, side, landing, cx);
            } else {
                self.side_chats.remove(&side);
            }
        }
        self.sync_hidden_chats(cx);
    }

    pub(super) fn prune_pr_chat_index(&mut self, cx: &mut Context<Self>) {
        let state = self.state.read(cx);
        let revision = state.link_roots_revision;
        if !state.chats_synced || self.pr_chat_trash.pruned == Some(revision) {
            return;
        }
        let live: Vec<&str> = self
            .pr_chat
            .iter()
            .flat_map(|panel| {
                std::iter::once(panel.chat_id.as_str()).chain(
                    panel
                        .previous
                        .as_ref()
                        .map(|previous| previous.row.id.as_str()),
                )
            })
            .chain(
                self.pr_chat_trash
                    .parked
                    .iter()
                    .map(|parked| parked.chat_id.as_str()),
            )
            .collect();
        let current = settings::current(cx);
        let mut stale = current.stale_pr_chats(&state.chats, &state.vanished_chats, &live);
        if current.pr_chat_history == PrChatHistory::Fresh {
            stale.extend(
                current
                    .pr_chat_sessions
                    .values()
                    .flatten()
                    .filter(|session| session.fresh && !live.contains(&session.chat_id.as_str()))
                    .map(|session| session.chat_id.clone()),
            );
        }
        self.pr_chat_trash.pruned = Some(revision);
        if stale.is_empty() {
            return;
        }
        settings::update(SavePolicy::Debounced, cx, |settings| {
            for chat_id in &stale {
                settings.unindex_pr_chat(chat_id);
            }
        });
        self.sync_hidden_chats(cx);
    }

    pub(super) fn sync_hidden_chats(&self, cx: &mut Context<Self>) {
        let mut hidden: HashSet<String> = settings::current(cx)
            .pr_chat_ids()
            .map(str::to_owned)
            .collect();
        if let Some(panel) = &self.pr_chat {
            hidden.insert(panel.chat_id.clone());
        }
        hidden.extend(
            self.pr_chat_trash
                .parked
                .iter()
                .filter(|parked| !matches!(parked.landing, Landing::Promote))
                .map(|parked| parked.chat_id.clone()),
        );
        hidden.extend(self.pr_chat_trash.deleted.iter().cloned());
        self.state
            .update(cx, |state, cx| state.set_hidden_chats(hidden, cx));
        if let Some(page) = &self.pull_requests_page {
            page.update(cx, |_, cx| cx.notify());
        }
    }

    pub(super) fn pr_chat_owns_focus(&self, window: &Window, cx: &App) -> bool {
        self.pr_chat_tab().is_some_and(|tab| {
            tab.composer
                .read(cx)
                .focus_handle(cx)
                .contains_focused(window, cx)
        })
    }

    fn pr_chat_suggestions(&self, cx: &App) -> Option<(String, Vec<Suggestion>)> {
        let panel = self.pr_chat.as_ref()?;
        if panel.resumed && matches!(panel.context, PrChatContext::List(_)) {
            return None;
        }
        let tab = self.pr_chat_tab()?;
        let (entry, text) =
            crate::pull_requests::latest_chat_reply(&tab.state.read(cx).transcript)?;
        let mut parsed = panel.parsed.borrow_mut();
        if !parsed
            .as_ref()
            .is_some_and(|(seen, seen_text, _)| *seen == entry && *seen_text == text)
        {
            let suggestions = panel.context.suggestions(&text);
            *parsed = Some((entry.clone(), text, suggestions));
        }
        let suggestions = parsed
            .as_ref()
            .map(|(_, _, s)| s.clone())
            .unwrap_or_default();
        (!suggestions.is_empty()).then_some((entry, suggestions))
    }

    fn pr_chat_suggestion(&self, key: &PrChatCardRef, cx: &App) -> Option<Suggestion> {
        let (entry, suggestions) = self.pr_chat_suggestions(cx)?;
        if entry != key.entry_id {
            return None;
        }
        suggestions.into_iter().find(|s| s.index == key.index)
    }

    fn edit_pr_chat_card(
        &mut self,
        key: PrChatCardRef,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(suggestion) = self.pr_chat_suggestion(&key, cx) else {
            return;
        };
        let Some(panel) = self.pr_chat.as_mut() else {
            return;
        };
        let card = panel.cards.entry(key).or_default();
        if card.editor.take().is_none() {
            let body = suggestion.body().unwrap_or_default().to_owned();
            let input = cx.new(|cx| {
                let mut input = ComposerInput::new("Edit before posting", cx);
                input.set_text(body, cx);
                input
            });
            let handle = gpui::Focusable::focus_handle(input.read(cx), cx);
            window.focus(&handle, cx);
            card.editor = Some(input);
        }
        cx.notify();
    }

    fn dismiss_pr_chat_card(&mut self, key: PrChatCardRef, cx: &mut Context<Self>) {
        let Some(panel) = self.pr_chat.as_mut() else {
            return;
        };
        let chat_id = panel.chat_id.clone();
        let card = key.clone();
        panel.cards.entry(key).or_default().dismissed = true;
        settings::update(SavePolicy::Debounced, cx, |settings| {
            settings.record_pr_chat_dismiss(&chat_id, card)
        });
        cx.notify();
    }

    fn post_pr_chat_card(
        &mut self,
        key: PrChatCardRef,
        request_changes: bool,
        cx: &mut Context<Self>,
    ) {
        let Some(suggestion) = self.pr_chat_suggestion(&key, cx) else {
            return;
        };
        let last = settings::pull_requests_merge_method(cx);
        if suggestion.confirmation(last).is_some() {
            if let Some(panel) = self.pr_chat.as_mut() {
                panel.confirm = Some(key);
            }
            cx.notify();
            return;
        }
        self.run_pr_chat_card(key, suggestion, request_changes, cx);
    }

    fn confirm_pr_chat_card(&mut self, cx: &mut Context<Self>) {
        let Some(key) = self.pr_chat.as_mut().and_then(|panel| panel.confirm.take()) else {
            return;
        };
        if let Some(suggestion) = self.pr_chat_suggestion(&key, cx) {
            self.run_pr_chat_card(key, suggestion, false, cx);
        }
        cx.notify();
    }

    fn cancel_pr_chat_confirm(&mut self, cx: &mut Context<Self>) {
        if let Some(panel) = self.pr_chat.as_mut() {
            panel.confirm = None;
        }
        cx.notify();
    }

    pub(super) fn escape_pr_chat_confirm(&mut self, cx: &mut Context<Self>) -> bool {
        let open = self
            .pr_chat
            .as_ref()
            .is_some_and(|panel| panel.confirm.is_some());
        if open {
            self.cancel_pr_chat_confirm(cx);
        }
        open
    }

    fn run_pr_chat_card(
        &mut self,
        key: PrChatCardRef,
        suggestion: Suggestion,
        request_changes: bool,
        cx: &mut Context<Self>,
    ) {
        let Some(engine) = self.state.read(cx).engine().cloned() else {
            return;
        };
        let last = settings::pull_requests_merge_method(cx);
        let Some(panel) = self.pr_chat.as_mut() else {
            return;
        };
        let chat_id = panel.chat_id.clone();
        let done = settings::pr_chat_session(&chat_id, cx)
            .is_some_and(|session| session.posted.iter().any(|posted| posted.card == key));
        let posting = (chat_id.clone(), key.clone());
        let card = panel.cards.entry(key.clone()).or_default();
        if done || self.pr_chat_trash.posting.contains(&posting) || card.posted.is_some() {
            return;
        }
        let edited = card
            .editor
            .as_ref()
            .map(|editor| editor.read(cx).text().to_string());
        let Some(call) = suggestion.call(edited.as_deref(), request_changes, last) else {
            card.error = Some("Write something before posting.".into());
            cx.notify();
            return;
        };
        self.pr_chat_trash.posting.insert(posting);
        card.error = None;
        let label = suggestion.posted_label(request_changes);
        cx.spawn(async move |this, cx| {
            let result = engine
                .client()
                .call(call.method, call.params)
                .await
                .map(|_| ())
                .map_err(|error| error.to_string());
            this.update(cx, |shell, cx| {
                shell.finish_pr_chat_card(chat_id, key, suggestion, label, result, cx)
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    fn finish_pr_chat_card(
        &mut self,
        chat_id: String,
        key: PrChatCardRef,
        suggestion: Suggestion,
        label: String,
        result: Result<(), String>,
        cx: &mut Context<Self>,
    ) {
        let posted = result.as_ref().ok().map(|_| PrChatPosted {
            card: key.clone(),
            label,
            at: Utc::now(),
        });
        self.pr_chat_trash
            .posting
            .remove(&(chat_id.clone(), key.clone()));
        if let Some(posted) = posted.clone() {
            settings::update(SavePolicy::Immediate, cx, |settings| {
                settings.record_pr_chat_post(&chat_id, posted)
            });
        }
        if let Some(card) = self
            .pr_chat
            .as_mut()
            .filter(|panel| panel.chat_id == chat_id)
            .map(|panel| panel.cards.entry(key).or_default())
        {
            match &result {
                Ok(()) => {
                    card.editor = None;
                    card.posted = posted;
                }
                Err(error) => {
                    card.error = Some(crate::pull_requests::chat_write_failure(error).into())
                }
            }
        }
        if result.is_ok()
            && let Some(page) = self.pull_requests_page.clone()
        {
            page.update(cx, |page, cx| page.reload_after_chat_write(&suggestion, cx));
        }
        cx.notify();
    }

    pub(super) fn render_pr_chat(
        &mut self,
        page_width: f32,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        if self.pr_chat.as_ref().is_some_and(|panel| {
            panel
                .side
                .is_some_and(|id| !self.side_chats.contains_key(&id))
        }) {
            self.pr_chat = None;
        }
        if self
            .pull_requests_page
            .as_ref()
            .is_some_and(|page| page.read(cx).git_tab_active())
        {
            return None;
        }
        let theme = Theme::of(cx).clone();
        let page_height =
            (f32::from(window.viewport_size().height) - Theme::TITLEBAR_HEIGHT).max(0.0);
        let width = PR_CHAT_DOCK_WIDTH
            .min(page_width - 2.0 * PR_CHAT_DOCK_INSET)
            .max(0.0);
        let cap = page_height * PR_CHAT_SHEET_CAP;
        let visible = self.pr_chat_sheet_open(cx);
        let reduced = motion::reduced_motion(cx);
        let panel = self.pr_chat.as_mut()?;
        let flipped = panel.shown != visible;
        if flipped {
            panel.shown = visible;
            panel.motion = (!reduced).then(|| (Instant::now(), visible));
            if !visible {
                panel.sessions_menu = false;
            }
        }
        let reveal = match panel.motion {
            Some((at, opening)) => {
                let total = motion::RESIZE.total().as_secs_f32() * motion::speed_scale();
                let raw = if total > 0.0 {
                    at.elapsed().as_secs_f32() / total
                } else {
                    1.0
                };
                if raw >= 1.0 {
                    panel.motion = None;
                    None
                } else {
                    window.request_animation_frame();
                    let eased = motion::RESIZE.progress(raw);
                    Some(if opening { eased } else { 1.0 - eased })
                }
            }
            None => None,
        };
        if flipped {
            self.sync_pr_chat_button(cx);
        }
        let panel = self.pr_chat.as_ref()?;
        let frame = div()
            .id("pr-chat-dock")
            .debug_selector(|| "pr-chat-dock".into())
            .occlude()
            .absolute()
            .bottom(px(PR_CHAT_DOCK_INSET))
            .left(px(((page_width - width) / 2.0).max(0.0)))
            .w(px(width))
            .flex()
            .flex_col()
            .rounded(px(18.0))
            .border_1()
            .border_color(theme.border)
            .bg(theme.surface_dialog)
            .shadow_lg();
        let Some((transcript, composer)) = panel
            .side
            .and_then(|id| self.side_chats.get(&id))
            .map(|tab| (tab.transcript.clone(), tab.composer.clone()))
        else {
            return panel.error.clone().map(|error| {
                frame
                    .px(px(16.0))
                    .py(px(14.0))
                    .text_size(px(12.0))
                    .text_color(theme.text_muted)
                    .child(error)
                    .into_any_element()
            });
        };
        let chips: Vec<(ContextChip, SharedString)> = panel
            .context
            .chips()
            .into_iter()
            .filter(|(chip, _)| !panel.removed.contains(chip))
            .map(|(chip, label)| (chip, label.into()))
            .collect();
        let shell = cx.weak_entity();
        composer.update(cx, |composer, cx| {
            composer.set_dock_frame(crate::composer_dock::DockFrame::settled(true), cx);
            composer.set_available_width(width, cx);
            composer.set_pr_chips(
                chips,
                Rc::new(move |chip, cx| {
                    shell
                        .update(cx, |shell, cx| shell.remove_pr_chat_chip(chip, cx))
                        .ok();
                }),
                cx,
            );
        });
        let bar = div()
            .flex_none()
            .debug_selector(|| "pr-chat-bar".into())
            .child(composer.clone());
        if !visible && reveal.is_none() {
            return Some(frame.child(bar).into_any_element());
        }

        let sheet = self.render_pr_chat_sheet(transcript, composer, &theme, cx);
        let panel = self.pr_chat.as_ref()?;
        let measured = panel.sheet_height.clone();
        let limit = reveal.map(|t| match visible {
            true => t * cap,
            false => t * measured.get(),
        });
        let sheet = div()
            .id("pr-chat-sheet")
            .debug_selector(|| "pr-chat-sheet".into())
            .relative()
            .flex_shrink_1()
            .min_h_0()
            .flex()
            .flex_col()
            .overflow_hidden()
            .when_some(limit, |el, limit| el.max_h(px(limit)))
            .children(sheet)
            .child(
                gpui::canvas(
                    move |bounds, _, _| {
                        if visible {
                            measured.set(f32::from(bounds.size.height));
                        }
                    },
                    |_, _, _, _| {},
                )
                .absolute()
                .inset_0(),
            );
        let menu = panel
            .sessions_menu
            .then(|| self.render_pr_chat_sessions_menu(&theme, cx));
        let confirm = self.render_pr_chat_confirm(&theme, cx);
        Some(
            frame
                .max_h(px(cap))
                .child(sheet)
                .child(bar)
                .children(menu)
                .children(confirm)
                .into_any_element(),
        )
    }

    fn render_pr_chat_sheet(
        &self,
        transcript: Entity<Transcript>,
        composer: Entity<Composer>,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        let Some(panel) = self.pr_chat.as_ref() else {
            return Vec::new();
        };
        let promotable = self.pr_chat_promotable(cx);
        let menu_open = panel.sessions_menu;
        let (number, title) = panel.context.dock_title();
        let status = self.pr_chat_header_status(cx);
        let prompts = panel.context.continue_prompts();
        let header_button =
            |id: &'static str, label: &'static str, glyph: &'static str, enabled: bool| {
                div()
                    .id(id)
                    .debug_selector(move || id.into())
                    .role(gpui::Role::Button)
                    .aria_label(label)
                    .size(px(26.0))
                    .flex_none()
                    .rounded(px(6.0))
                    .flex()
                    .items_center()
                    .justify_center()
                    .map(|el| match enabled {
                        true => el.cursor_pointer().hover(|s| s.bg(theme.ink(0.06))),
                        false => el.opacity(0.4),
                    })
                    .child(icon(glyph).size(px(14.0)).text_color(theme.text_muted))
            };
        let header = div()
            .flex_none()
            .h(px(PR_CHAT_HEADER_HEIGHT))
            .flex()
            .items_center()
            .gap(px(4.0))
            .pl(px(10.0))
            .pr(px(8.0))
            .border_b_1()
            .border_color(theme.border.opacity(0.6))
            .child(
                div()
                    .id("pr-chat-sessions")
                    .debug_selector(|| "pr-chat-sessions".into())
                    .role(gpui::Role::Button)
                    .aria_label("Chat sessions")
                    .min_w_0()
                    .flex()
                    .items_center()
                    .gap(px(6.0))
                    .px(px(7.0))
                    .py(px(3.0))
                    .rounded(px(7.0))
                    .text_size(px(13.0))
                    .font_weight(gpui::FontWeight::MEDIUM)
                    .text_color(theme.text)
                    .cursor_pointer()
                    .when(menu_open, |el| el.bg(theme.ink(0.06)))
                    .hover(|s| s.bg(theme.ink(0.06)))
                    .on_click(cx.listener(|this, _, _, cx| this.toggle_pr_chat_sessions_menu(cx)))
                    .children(
                        number.map(|number| {
                            div().flex_none().text_color(theme.success).child(number)
                        }),
                    )
                    .child(div().min_w_0().truncate().child(title))
                    .child(
                        icon(icons::LUCIDE_CHEVRON_DOWN)
                            .flex_none()
                            .size(px(12.0))
                            .text_color(theme.text_muted),
                    ),
            )
            .children(status.map(|status| {
                div()
                    .debug_selector(|| "pr-chat-time-left".into())
                    .flex_none()
                    .text_size(px(11.0))
                    .text_color(theme.text_muted.opacity(0.75))
                    .child(status)
            }))
            .child(div().flex_1())
            .child(
                header_button("pr-chat-new", "New chat", icons::PEN_NEW_SQUARE, true)
                    .on_click(cx.listener(|this, _, _, cx| this.new_pr_chat_session(cx))),
            )
            .child(
                header_button(
                    "pr-chat-promote",
                    "Open as thread",
                    icons::LUCIDE_EXTERNAL_LINK,
                    promotable,
                )
                .when(promotable, |el| {
                    el.on_click(cx.listener(|this, _, _, cx| this.promote_pr_chat(cx)))
                }),
            )
            .child(
                header_button(
                    "pr-chat-fold",
                    "Fold chat",
                    icons::LUCIDE_CHEVRON_DOWN,
                    true,
                )
                .on_click(cx.listener(|this, _, _, cx| this.fold_pr_chat(cx))),
            );

        let content_height =
            |transcript: &Entity<Transcript>, cx: &App| transcript.read(cx).content_height();
        let mut watched = Vec::new();
        let body = div()
            .id("pr-chat-body")
            .debug_selector(|| "pr-chat-body".into())
            .flex_shrink_1()
            .min_h_0()
            .flex()
            .flex_col()
            .gap(px(10.0))
            .pt(px(10.0))
            .pb(px(8.0));
        let body = if let Some(previous) = &panel.previous {
            let height = content_height(&previous.tab.transcript, cx);
            watched.push((previous.tab.transcript.clone(), height));
            let back = div()
                .id("pr-chat-back-to-previous")
                .debug_selector(|| "pr-chat-back-to-previous".into())
                .role(gpui::Role::Button)
                .flex_none()
                .text_color(theme.text_muted)
                .underline()
                .cursor_pointer()
                .hover(|s| s.text_color(theme.text))
                .on_click(cx.listener(|this, _, _, cx| this.resume_previous_pr_chat(cx)))
                .child("back to previous");
            body.child(
                div()
                    .debug_selector(|| "pr-chat-previous".into())
                    .h(px(height))
                    .flex_shrink_1()
                    .min_h_0()
                    .opacity(PR_CHAT_PREVIOUS_OPACITY)
                    .child(previous.tab.transcript.clone()),
            )
            .child(
                divider_row(theme)
                    .debug_selector(|| "pr-chat-expired-divider".into())
                    .mx(px(18.0))
                    .child(rule(theme))
                    .child(SharedString::from(format!(
                        "{} · new chat started ·",
                        previous.idle
                    )))
                    .child(back)
                    .child(rule(theme)),
            )
            .child(continue_pills(&prompts, &composer, theme).mx(px(18.0)))
        } else {
            let height = content_height(&transcript, cx);
            watched.push((transcript.clone(), height));
            let cards = self
                .pr_chat_suggestions(cx)
                .and_then(|(entry, suggestions)| {
                    self.render_pr_chat_cards(entry, suggestions, theme, cx)
                });
            let floor = match cards.is_some() {
                true => height.min(PR_CHAT_REPLY_MIN),
                false => 0.0,
            };
            let earlier = panel.earlier.clone();
            body.children(earlier.clone().map(|label| {
                divider_row(theme)
                    .mx(px(18.0))
                    .child(rule(theme))
                    .child(label)
                    .child(rule(theme))
            }))
            .child(
                div()
                    .debug_selector(|| "pr-chat-transcript".into())
                    .h(px(height))
                    .min_h(px(floor))
                    .flex_shrink(PR_CHAT_TRANSCRIPT_YIELD)
                    .when(earlier.is_some(), |el| {
                        el.opacity(0.6).debug_selector(|| "pr-chat-earlier".into())
                    })
                    .child(transcript),
            )
            .children(cards)
            .children(earlier.map(|_| {
                div()
                    .flex_none()
                    .flex()
                    .flex_col()
                    .gap(px(6.0))
                    .mx(px(18.0))
                    .child(
                        divider_row(theme)
                            .child(rule(theme))
                            .child("Today")
                            .child(rule(theme)),
                    )
                    .child(continue_pills(&prompts, &composer, theme))
            }))
        };
        let shell = cx.weak_entity();
        let watch = gpui::canvas(
            move |_, window, cx| {
                let stale = watched.iter().any(|(transcript, used)| {
                    (transcript.read(cx).content_height() - used).abs() > 0.5
                });
                if stale {
                    let shell = shell.clone();
                    window.on_next_frame(move |_, cx| {
                        shell.update(cx, |_, cx| cx.notify()).ok();
                    });
                }
            },
            |_, _, _, _| {},
        )
        .absolute()
        .size_0();
        vec![
            header.into_any_element(),
            body.child(watch).into_any_element(),
        ]
    }

    fn render_pr_chat_sessions_menu(&self, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let rows = self.pr_chat_session_rows(cx);
        let entry = |id: SharedString, active: bool, label: String, detail: Option<String>| {
            crate::popover::menu_row(theme, active, id.clone())
                .id(id.clone())
                .debug_selector(move || id.to_string())
                .flex()
                .items_center()
                .gap(px(8.0))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .gap(px(1.0))
                        .child(div().w_full().truncate().child(label))
                        .children(detail.map(|detail| {
                            div()
                                .w_full()
                                .truncate()
                                .text_size(px(11.0))
                                .text_color(theme.text_muted)
                                .child(detail)
                        })),
                )
        };
        crate::popover::popover_card(theme)
            .id("pr-chat-sessions-menu")
            .debug_selector(|| "pr-chat-sessions-menu".into())
            .bg(theme.surface_dialog)
            .occlude()
            .absolute()
            .top(px(PR_CHAT_HEADER_HEIGHT - 2.0))
            .left(px(8.0))
            .w(px(310.0))
            .flex()
            .flex_col()
            .children(rows.into_iter().enumerate().map(|(ix, row)| {
                let chat_id = row.chat_id;
                entry(
                    format!("pr-chat-session-{ix}").into(),
                    row.current,
                    row.label,
                    Some(row.detail),
                )
                .children(row.state.map(|(label, live)| {
                    div()
                        .debug_selector(move || format!("pr-chat-session-state-{ix}"))
                        .flex_none()
                        .text_size(px(10.5))
                        .text_color(match live {
                            true => theme.success,
                            false => theme.text_muted.opacity(0.7),
                        })
                        .child(label)
                }))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.switch_pr_chat_session(chat_id.clone(), cx)
                }))
            }))
            .child(crate::popover::menu_separator())
            .child(
                entry(
                    "pr-chat-new-session".into(),
                    false,
                    "+ New chat".into(),
                    None,
                )
                .on_click(cx.listener(|this, _, _, cx| this.new_pr_chat_session(cx))),
            )
            .into_any_element()
    }

    fn render_pr_chat_cards(
        &self,
        entry: String,
        suggestions: Vec<Suggestion>,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let panel = self.pr_chat.as_ref()?;
        let (posted, dismissed) = settings::pr_chat_session(&panel.chat_id, cx)
            .map(|session| (session.posted, session.dismissed))
            .unwrap_or_default();
        let last = settings::pull_requests_merge_method(cx);
        let now = chrono::Local::now();
        let cards: Vec<AnyElement> = suggestions
            .into_iter()
            .filter_map(|suggestion| {
                let ix = suggestion.index;
                let key = PrChatCardRef {
                    entry_id: entry.clone(),
                    index: ix,
                };
                let state = panel.cards.get(&key);
                if state.is_some_and(|state| state.dismissed) || dismissed.contains(&key) {
                    return None;
                }
                let done = state
                    .and_then(|state| state.posted.clone())
                    .or_else(|| posted.iter().find(|posted| posted.card == key).cloned());
                let frame = div()
                    .id(("pr-chat-card", ix))
                    .flex_none()
                    .flex()
                    .flex_col()
                    .rounded(px(10.0))
                    .border_1()
                    .overflow_hidden()
                    .text_size(px(12.0));
                if let Some(done) = done {
                    let at = done.at.with_timezone(&chrono::Local);
                    return Some(
                        frame
                            .debug_selector(move || format!("pr-chat-card-posted-{ix}"))
                            .border_color(theme.border)
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap(px(6.0))
                                    .px(px(10.0))
                                    .py(px(8.0))
                                    .bg(theme.success.opacity(0.06))
                                    .text_color(theme.success)
                                    .child(
                                        icon(icons::LUCIDE_CHECK)
                                            .size(px(13.0))
                                            .text_color(theme.success),
                                    )
                                    .child(SharedString::from(format!(
                                        "{} · {} {}",
                                        done.label,
                                        day_label(done.at, now),
                                        at.format("%H:%M")
                                    ))),
                            )
                            .into_any_element(),
                    );
                }
                let pending = self
                    .pr_chat_trash
                    .posting
                    .contains(&(panel.chat_id.clone(), key.clone()));
                let editor = state.and_then(|state| state.editor.clone());
                let error = state.and_then(|state| state.error.clone());
                let header = suggestion.card_header();
                let text = suggestion
                    .summary(last)
                    .or_else(|| suggestion.body().map(str::to_owned))
                    .filter(|text| !text.is_empty());
                let content = match editor {
                    Some(editor) => Some(
                        div()
                            .id(("pr-chat-card-input", ix))
                            .debug_selector(move || format!("pr-chat-card-input-{ix}"))
                            .role(gpui::Role::TextInput)
                            .aria_label("Edit before posting")
                            .mx(px(10.0))
                            .my(px(8.0))
                            .min_h(px(64.0))
                            .rounded(px(8.0))
                            .border_1()
                            .border_color(theme.hairline(0.14))
                            .px(px(8.0))
                            .py(px(6.0))
                            .child(editor)
                            .into_any_element(),
                    ),
                    None => text.map(|text| {
                        div()
                            .id(("pr-chat-card-body", ix))
                            .debug_selector(move || format!("pr-chat-card-body-{ix}"))
                            .max_h(px(140.0))
                            .overflow_y_scroll()
                            .px(px(10.0))
                            .py(px(8.0))
                            .text_color(theme.text.opacity(0.85))
                            .child(SharedString::from(text))
                            .into_any_element()
                    }),
                };
                let post_key = key.clone();
                let mut footer = div()
                    .flex()
                    .flex_wrap()
                    .gap(px(6.0))
                    .px(px(10.0))
                    .py(px(8.0))
                    .border_t_1()
                    .border_color(theme.border.opacity(0.6))
                    .child(
                        card_button(
                            format!("pr-chat-card-post-{ix}"),
                            if pending {
                                "Posting..."
                            } else {
                                suggestion.primary_label()
                            },
                            true,
                            theme,
                        )
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.post_pr_chat_card(post_key.clone(), false, cx)
                        })),
                    );
                if suggestion.is_draft() {
                    let edit_key = key.clone();
                    footer = footer.child(
                        card_button(
                            format!("pr-chat-card-edit-{ix}"),
                            if state.is_some_and(|state| state.editor.is_some()) {
                                "Cancel edit"
                            } else {
                                "Edit"
                            },
                            false,
                            theme,
                        )
                        .on_click(cx.listener(
                            move |this, _, window, cx| {
                                this.edit_pr_chat_card(edit_key.clone(), window, cx)
                            },
                        )),
                    );
                }
                if suggestion.offers_request_changes() {
                    let request_key = key.clone();
                    footer = footer.child(
                        card_button(
                            format!("pr-chat-card-request-changes-{ix}"),
                            "Request changes",
                            false,
                            theme,
                        )
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.post_pr_chat_card(request_key.clone(), true, cx)
                        })),
                    );
                }
                if !suggestion.is_draft() {
                    let dismiss_key = key.clone();
                    footer = footer.child(
                        card_button(
                            format!("pr-chat-card-dismiss-{ix}"),
                            "Dismiss",
                            false,
                            theme,
                        )
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.dismiss_pr_chat_card(dismiss_key.clone(), cx)
                        })),
                    );
                }
                Some(
                    frame
                        .debug_selector(move || format!("pr-chat-card-{ix}"))
                        .border_color(theme.accent.opacity(0.3))
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap(px(6.0))
                                .px(px(10.0))
                                .py(px(8.0))
                                .bg(theme.accent.opacity(0.08))
                                .text_color(theme.text_muted)
                                .child(div().text_color(theme.accent).child("✦"))
                                .child(SharedString::from(header)),
                        )
                        .children(content)
                        .children(error.map(|error| {
                            div()
                                .px(px(10.0))
                                .pb(px(6.0))
                                .text_color(theme.danger)
                                .child(error)
                        }))
                        .child(footer)
                        .into_any_element(),
                )
            })
            .collect();
        (!cards.is_empty()).then(|| {
            div()
                .id("pr-chat-cards")
                .debug_selector(|| "pr-chat-cards".into())
                .min_h_0()
                .overflow_y_scroll()
                .flex()
                .flex_col()
                .gap(px(8.0))
                .px(px(12.0))
                .pb(px(8.0))
                .children(cards)
                .into_any_element()
        })
    }

    fn render_pr_chat_confirm(&self, theme: &Theme, cx: &mut Context<Self>) -> Option<AnyElement> {
        let key = self.pr_chat.as_ref()?.confirm.clone()?;
        let last = settings::pull_requests_merge_method(cx);
        let confirm = self.pr_chat_suggestion(&key, cx)?.confirmation(last)?;
        Some(
            div()
                .id("pr-chat-confirm-scrim")
                .absolute()
                .inset_0()
                .occlude()
                .flex()
                .items_center()
                .justify_center()
                .bg(gpui::hsla(0.0, 0.0, 0.0, 0.3))
                .child(
                    crate::popover::popover_card(theme)
                        .id("pr-chat-confirm")
                        .debug_selector(|| "pr-chat-confirm".into())
                        .role(gpui::Role::Dialog)
                        .aria_label(SharedString::from(confirm.title.clone()))
                        .bg(theme.surface_dialog)
                        .w(px(320.0))
                        .p(px(16.0))
                        .flex()
                        .flex_col()
                        .gap(px(8.0))
                        .child(
                            div()
                                .text_size(px(14.0))
                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                .text_color(theme.text)
                                .child(confirm.title),
                        )
                        .child(
                            div()
                                .text_size(px(12.0))
                                .text_color(theme.text_muted)
                                .child(confirm.description),
                        )
                        .child(
                            div()
                                .mt(px(8.0))
                                .flex()
                                .justify_end()
                                .gap(px(8.0))
                                .child(
                                    card_button("pr-chat-confirm-cancel", "Cancel", false, theme)
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            this.cancel_pr_chat_confirm(cx)
                                        })),
                                )
                                .child(
                                    card_button("pr-chat-confirm-ok", confirm.button, true, theme)
                                        .when(confirm.destructive, |el| el.bg(theme.danger))
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            this.confirm_pr_chat_card(cx)
                                        })),
                                ),
                        ),
                )
                .into_any_element(),
        )
    }
}

fn hide_card_fences(tab: &SideChatTab, context: PrChatContext, cx: &mut Context<Shell>) {
    tab.transcript.update(cx, |transcript, cx| {
        transcript.hide_code_fences(
            std::sync::Arc::new(move |language, code| context.is_card_fence(language, code)),
            cx,
        )
    });
}

fn card_button(
    id: impl Into<SharedString>,
    label: impl Into<SharedString>,
    primary: bool,
    theme: &Theme,
) -> gpui::Stateful<gpui::Div> {
    let id: SharedString = id.into();
    let label: SharedString = label.into();
    let selector = id.clone();
    div()
        .id(id)
        .debug_selector(move || selector.to_string())
        .role(gpui::Role::Button)
        .aria_label(label.clone())
        .h(px(26.0))
        .px(px(10.0))
        .flex()
        .flex_none()
        .items_center()
        .rounded(px(7.0))
        .text_size(px(12.0))
        .cursor_pointer()
        .map(|el| match primary {
            true => el
                .bg(theme.accent)
                .text_color(gpui::hsla(0.0, 0.0, 1.0, 1.0))
                .hover(|style| style.opacity(0.9)),
            false => el
                .border_1()
                .border_color(theme.border)
                .text_color(theme.text.opacity(0.8))
                .hover(|style| style.bg(theme.ink(0.05))),
        })
        .child(label)
}

fn day_label(at: chrono::DateTime<Utc>, now: chrono::DateTime<chrono::Local>) -> String {
    let day = at.with_timezone(&chrono::Local).date_naive();
    match (now.date_naive() - day).num_days() {
        ..=0 => "Today".into(),
        1 => "Yesterday".into(),
        days @ 2..=6 => format!("{days} days ago"),
        _ => day.format("%b %-d").to_string(),
    }
}

fn session_label(day: &str, title: Option<&str>) -> String {
    match title.map(str::trim).filter(|title| !title.is_empty()) {
        Some(title) => format!("{day} · {title}"),
        None => day.to_owned(),
    }
}

fn earlier_session(
    chat: &zeron_proto::Chat,
    now: chrono::DateTime<chrono::Local>,
) -> Option<SharedString> {
    let at = chat.last_message_at?;
    let day = day_label(at, now);
    (day != "Today").then(|| session_label(&day, chat.title.as_deref()).into())
}

fn divider_row(theme: &Theme) -> gpui::Div {
    div()
        .flex_none()
        .flex()
        .items_center()
        .gap(px(8.0))
        .text_size(px(11.0))
        .text_color(theme.text_muted.opacity(0.7))
        .whitespace_nowrap()
}

fn rule(theme: &Theme) -> gpui::Div {
    div()
        .flex_1()
        .min_w(px(12.0))
        .h(px(1.0))
        .bg(theme.border.opacity(0.6))
}

fn continue_pills(
    prompts: &[&'static str; 2],
    composer: &Entity<Composer>,
    theme: &Theme,
) -> gpui::Div {
    div()
        .flex_none()
        .flex()
        .flex_wrap()
        .gap(px(6.0))
        .children(prompts.iter().enumerate().map(|(ix, prompt)| {
            let send = composer.clone();
            let prompt = *prompt;
            div()
                .id(("pr-chat-continue", ix))
                .debug_selector(move || format!("pr-chat-continue-{ix}"))
                .role(gpui::Role::Button)
                .flex()
                .items_center()
                .gap(px(5.0))
                .h(px(26.0))
                .px(px(10.0))
                .rounded(px(13.0))
                .border_1()
                .border_color(theme.border.opacity(0.8))
                .text_size(px(12.0))
                .text_color(theme.text.opacity(0.85))
                .cursor_pointer()
                .hover(|s| s.bg(theme.ink(0.05)))
                .on_click(move |_, _, cx| {
                    send.update(cx, |composer, cx| composer.send_prompt(prompt.into(), cx))
                })
                .child(div().text_color(theme.accent).child("✦"))
                .child(prompt)
        }))
}

fn idle_span(idle: chrono::Duration) -> String {
    let minutes = idle.num_minutes().max(1);
    match minutes {
        ..60 => format!("{minutes}m"),
        60..1440 => format!("{}h", minutes / 60),
        _ => format!("{}d", minutes / 1440),
    }
}

fn left_label(left: chrono::Duration) -> (String, bool) {
    if left <= chrono::Duration::zero() {
        return ("expired".into(), false);
    }
    let minutes = (left.num_seconds() + 59) / 60;
    let label = match minutes {
        ..60 => format!("{minutes}m left"),
        60..1440 => format!("{}h left", minutes / 60),
        _ => format!("{}d left", minutes / 1440),
    };
    (label, true)
}

fn when_label(at: chrono::DateTime<Utc>, now: chrono::DateTime<chrono::Local>) -> String {
    match day_label(at, now).as_str() {
        "Today" => format!("Today {}", at.with_timezone(&chrono::Local).format("%H:%M")),
        day => day.to_owned(),
    }
}

fn user_turns(transcript: &[zeron_doc::SessionMessageEntry]) -> usize {
    transcript
        .iter()
        .filter(|entry| entry.role == zeron_doc::MessageRole::User)
        .count()
}

fn turn_count(turns: usize) -> String {
    match turns {
        1 => "1 turn".into(),
        n => format!("{n} turns"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::PrChatIdleExpiry;
    use gpui::{AppContext, TestAppContext, VisualTestContext};
    use zeron_rpc::{ClientFrame, ServerFrame};

    struct Wire {
        frames: tokio::sync::mpsc::Receiver<String>,
        replies: tokio::sync::mpsc::Sender<String>,
    }

    impl Wire {
        fn pump(runtime: &tokio::runtime::Runtime, cx: &mut VisualTestContext) {
            for _ in 0..4 {
                runtime.block_on(async {
                    for _ in 0..8 {
                        tokio::task::yield_now().await;
                    }
                });
                cx.run_until_parked();
            }
        }

        fn serve(
            &mut self,
            runtime: &tokio::runtime::Runtime,
            cx: &mut VisualTestContext,
            answer: impl Fn(&str, &serde_json::Value) -> Option<serde_json::Value>,
        ) -> Vec<ClientFrame> {
            Self::pump(runtime, cx);
            let mut seen = Vec::new();
            while let Ok(frame) = self.frames.try_recv() {
                let frame: ClientFrame = serde_json::from_str(&frame).unwrap();
                if let Some(ok) = answer(frame.method.as_deref().unwrap_or(""), &frame.params) {
                    let reply = ServerFrame {
                        id: frame.id,
                        ok: Some(ok),
                        ..Default::default()
                    };
                    self.replies
                        .try_send(serde_json::to_string(&reply).unwrap())
                        .unwrap();
                }
                seen.push(frame);
            }
            Self::pump(runtime, cx);
            seen
        }
    }

    fn calls<'a>(frames: &'a [ClientFrame], method: &str) -> Vec<&'a ClientFrame> {
        frames
            .iter()
            .filter(|frame| frame.method.as_deref() == Some(method))
            .collect()
    }

    fn item() -> zeron_proto::ChangeRequestListItem {
        item_n(7)
    }

    fn item_n(number: u64) -> zeron_proto::ChangeRequestListItem {
        serde_json::from_value(serde_json::json!({
            "provider": "github", "host": "github.com", "repository": "acme/zeron",
            "number": number, "title": "Fix the thing",
            "url": format!("https://github.com/acme/zeron/pull/{number}"),
            "author": { "login": "bob" }, "headRef": "feature/x", "baseRef": "main",
            "state": "open", "isDraft": false, "mergeability": "mergeable",
            "additions": 4, "deletions": 1,
            "createdAt": "2026-10-01T00:00:00Z", "updatedAt": "2026-10-01T00:00:00Z",
            "viewerReviewRequested": false, "labels": [],
        }))
        .unwrap()
    }

    fn detail() -> serde_json::Value {
        serde_json::json!({
            "item": item(), "body": "Real body.", "changedFiles": 1,
            "isCrossRepository": false, "reviewers": [],
            "checks": [{ "name": "test", "status": "failure" }],
            "comments": [{ "id": "c1", "kind": "comment", "author": { "login": "amy" },
                "body": "Does this cover Pi?", "createdAt": "2026-10-02T00:00:00Z" }],
            "commentCount": 1, "commitCount": 1,
        })
    }

    fn answer_open(method: &str) -> Option<serde_json::Value> {
        match method {
            methods::GET_CHANGE_REQUEST => Some(detail()),
            methods::PREPARE_CHANGE_REQUEST_CHAT => Some(serde_json::json!({
                "spaceId": "s1", "root": "/repo", "cwd": "/repo",
                "worktree": false, "onHead": false,
            })),
            methods::GET_CHANGE_REQUEST_DIFF => Some(serde_json::json!({
                "patch": "diff --git a/x b/x\n@@ -1 +1 @@\n-a\n+b\n", "truncated": false,
            })),
            _ => None,
        }
    }

    fn fill(shell: &Entity<Shell>, cx: &mut VisualTestContext, text: &str) {
        shell.update(cx, |shell, cx| {
            let tab = shell.pr_chat_tab().expect("chat minted");
            tab.state.update(cx, |state, cx| {
                state.apply_transcript(reply_entry(text));
                cx.notify();
            });
            cx.notify();
        });
        cx.run_until_parked();
    }

    fn inside(outer: gpui::Bounds<Pixels>, inner: gpui::Bounds<Pixels>) -> bool {
        outer.contains(&inner.origin) && outer.contains(&inner.bottom_right())
    }

    fn setup(
        cx: &mut TestAppContext,
    ) -> (
        tokio::runtime::Runtime,
        Entity<Shell>,
        &mut VisualTestContext,
        Wire,
        tempfile::TempDir,
    ) {
        let (out, frames) = tokio::sync::mpsc::channel::<String>(256);
        let (replies, inbound) = tokio::sync::mpsc::channel::<String>(256);
        let (runtime, shell, cx, dir) = boot(cx, |runtime| {
            let _guard = runtime.enter();
            zeron_rpc::RpcClient::new(out, inbound)
        });
        (runtime, shell, cx, Wire { frames, replies }, dir)
    }

    fn boot(
        cx: &mut TestAppContext,
        client: impl FnOnce(&tokio::runtime::Runtime) -> zeron_rpc::RpcClient,
    ) -> (
        tokio::runtime::Runtime,
        Entity<Shell>,
        &mut VisualTestContext,
        tempfile::TempDir,
    ) {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let dir = tempfile::tempdir().unwrap();
        cx.update(|cx| {
            gpui_base::init(cx);
            cx.set_global(Theme::default());
            crate::motion::set_reduced_motion(cx, true);
            crate::app_menus::init(cx);
            crate::terminal::panel::init(cx);
            let mut initial = UiSettings::default();
            initial.pr_chat_idle_expiry = PrChatIdleExpiry::Never;
            settings::init(initial, dir.path(), cx);
            crate::history::init(
                Default::default(),
                Default::default(),
                Default::default(),
                Default::default(),
                cx,
            );
        });
        let client = client(&runtime);
        let state = cx.new(|_| AppState::new());
        state.update(cx, |state, _| {
            state.set_test_engine(crate::state::EngineHandle::from_test_client(client));
            state.workspace_scope = Some(WorkspaceScope::Local);
            state.connection = ConnectionStatus::Ready;
            state.local_device_id = Some("local".into());
            state.spaces = vec![
                serde_json::from_value(serde_json::json!({
                    "id": "s1", "deviceId": "local", "path": "/repo", "gitDetected": true,
                    "createdAt": Utc::now(),
                }))
                .unwrap(),
            ];
            state.chats = vec![
                serde_json::from_value(serde_json::json!({
                    "id": "wt", "deviceId": "local", "spaceId": "s1", "cwd": "/wt/x",
                    "branch": "feature/x", "archived": false, "createdAt": Utc::now(),
                }))
                .unwrap(),
            ];
        });
        let boot = EngineBootConfig {
            data_dir: dir.path().into(),
            ipc_port: 0,
            edge_url: "http://127.0.0.1:1".into(),
            edge_token: None,
            org_id: None,
            workos_client_id: None,
            default_harness: zeron_proto::HarnessId::Mock,
        };
        let (shell, cx) = cx.add_window_view(|_, cx| Shell::new(state, boot, cx));
        (runtime, shell, cx, dir)
    }

    #[gpui::test]
    fn the_dock_bar_mints_a_chat_with_the_context_and_stays_out_of_the_sidebar(
        cx: &mut TestAppContext,
    ) {
        let (runtime, shell, cx, mut wire, _dir) = setup(cx);
        let _guard = runtime.enter();
        let panel_key = shell.update(cx, |shell, cx| shell.panel_key(cx));

        shell.update(cx, |shell, cx| shell.open_pull_requests(cx));
        let page = shell.read_with(cx, |shell, _| shell.pull_requests_page.clone().unwrap());
        page.update(cx, |page, cx| page.open_linked(item(), None, cx));
        let frames = wire.serve(&runtime, cx, |method, _| answer_open(method));
        let prepare = calls(&frames, methods::PREPARE_CHANGE_REQUEST_CHAT);
        assert_eq!(prepare.len(), 1, "{frames:?}");
        assert_eq!(prepare[0].params["checkout"], "none");
        assert_eq!(prepare[0].params["number"], 7);
        assert!(
            calls(&frames, methods::MUTATE).is_empty(),
            "nothing written yet"
        );

        let (side, side_state, chat_id) = shell.update(cx, |shell, cx| {
            let panel = shell.pr_chat.as_ref().expect("panel");
            assert!(!shell.pr_chat_sheet_open(cx), "nothing to show yet");
            let tab = shell.pr_chat_tab().expect("chat minted locally");
            let chat_id = panel.chat_id.clone();
            let row = tab.state.read(cx).selected_chat_row().cloned().unwrap();
            assert_eq!(row.space_id.as_deref(), Some("s1"));
            assert_eq!(row.cwd.as_deref(), Some("/wt/x"), "the PR's worktree chat");
            assert_eq!(row.branch.as_deref(), Some("feature/x"));
            assert_eq!(row.created_by_chat_id, None);
            assert!(tab.state.read(cx).side_chat_unsaved());
            assert!(shell.state.read(cx).is_hidden_chat(&chat_id));
            assert!(!settings::current(cx).pr_chat_indexed(&chat_id));
            assert!(!shell.panels.get(&panel_key).terminal_open);
            (tab.composer.clone(), tab.state.clone(), chat_id)
        });
        assert!(
            cx.debug_bounds("pr-chat-sheet").is_none(),
            "idle: the bar only"
        );
        assert!(cx.debug_bounds("pr-chat-fold").is_none());
        assert_eq!(
            cx.update(|_, cx| side.read(cx).placeholder(cx)),
            "Ask or hand off work for this PR..."
        );
        let dock = cx.debug_bounds("pr-chat-dock").expect("bar drawn");
        let column = cx
            .debug_bounds("pull-requests-column")
            .expect("page column");
        let expected =
            PR_CHAT_DOCK_WIDTH.min(f32::from(column.size.width) - 2.0 * PR_CHAT_DOCK_INSET);
        assert!((dock.size.width - px(expected)).abs() < px(0.5), "{dock:?}");
        assert!((dock.center().x - column.center().x).abs() < px(0.5));
        assert!((column.bottom() - px(PR_CHAT_DOCK_INSET) - dock.bottom()).abs() < px(1.0));
        for selector in [
            "composer-footer",
            "picker-permission",
            "pr-chat-chip-label-PullRequest",
            "pr-chat-chip-label-Diff",
        ] {
            let bounds = cx
                .debug_bounds(selector)
                .unwrap_or_else(|| panic!("{selector} drawn"));
            assert!(
                inside(dock, bounds),
                "{selector} {bounds:?} inside {dock:?}"
            );
        }
        let chip = cx
            .debug_bounds("pr-chat-chip-Comments")
            .expect("comments chip");
        cx.simulate_click(chip.center(), gpui::Modifiers::default());
        cx.run_until_parked();
        assert!(cx.debug_bounds("pr-chat-chip-Comments").is_none());
        assert!(cx.debug_bounds("pr-chat-chip-Checks").is_some());
        cx.update(|window, cx| {
            let handle = side.read(cx).focus_handle(cx);
            window.focus(&handle, cx);
        });
        cx.simulate_keystrokes("escape");
        shell.read_with(cx, |shell, _| {
            assert!(
                matches!(shell.route, Route::PullRequests),
                "escape stays in the bar"
            );
            assert!(shell.pr_chat.is_some());
        });

        side.update(cx, |composer, cx| composer.send_prompt("Hi".into(), cx));
        let frames = wire.serve(&runtime, cx, |_, _| None);
        assert!(
            calls(&frames, methods::MUTATE).is_empty(),
            "no agent resolved yet"
        );
        assert!(side.read_with(cx, |composer, _| composer.failure().is_some()));
        let config: zeron_proto::ChatConfig = serde_json::from_value(serde_json::json!({
            "harness": "mock", "sandbox": "workspace-write", "permissionMode": "full",
        }))
        .unwrap();
        side_state.update(cx, |state, _| state.apply_chat_config(&chat_id, config));
        side.update(cx, |composer, cx| {
            composer.send_prompt("Review this".into(), cx)
        });
        let frames = wire.serve(&runtime, cx, |method, params| {
            (method == methods::MUTATE && params["op"] == "createChat")
                .then(|| serde_json::json!({}))
        });
        let create = calls(&frames, methods::MUTATE);
        assert_eq!(create.len(), 1, "{frames:?}");
        let create = &create[0].params;
        assert_eq!(create["chatId"], chat_id.as_str());
        assert_eq!(create["spaceId"], "s1");
        assert_eq!(create["cwd"], "/wt/x");
        assert_eq!(create["createdByChatId"], serde_json::Value::Null);
        assert_eq!(create["config"]["permissionMode"], "full");
        let frames = wire.serve(&runtime, cx, |method, _| {
            (method == methods::QUEUE_COMMAND).then(|| serde_json::json!({}))
        });
        let run = calls(&frames, methods::QUEUE_COMMAND);
        assert_eq!(run.len(), 1, "{frames:?}");
        let request = &run[0].params["command"]["request"];
        assert_eq!(request["permissionMode"], "full");
        assert_eq!(request["cwd"], "/wt/x");
        let prompt = request["prompt"].as_str().unwrap();
        assert!(
            prompt.starts_with(&format!(
                "Review this\n\n{}\n\n{}\n> Pull request #7",
                crate::pull_requests::CHAT_ACTION_NOTE,
                crate::pull_requests::CHAT_CONTEXT_HEADER
            )),
            "{prompt}"
        );
        assert!(prompt.contains("> ```diff\n> diff --git a/x b/x"));
        assert!(prompt.contains("> test: failed"));
        assert!(!prompt.contains("Does this cover Pi?"), "the removed chip");

        let row: zeron_proto::Chat = serde_json::from_value(serde_json::json!({
            "id": chat_id, "deviceId": "local", "spaceId": "s1", "cwd": "/wt/x",
            "archived": false, "createdAt": Utc::now(),
        }))
        .unwrap();
        shell.update(cx, |shell, cx| {
            let sessions = settings::current(cx).pr_chat_sessions;
            assert_eq!(sessions["github.com/acme/zeron#7"][0].chat_id, chat_id);
            shell.state.update(cx, |state, _| state.chats.push(row));
            let state = shell.state.read(cx);
            assert!(state.visible_chats().all(|chat| chat.id != chat_id));
            assert!(
                state
                    .sidebar_chats(Utc::now(), None)
                    .iter()
                    .all(|(_, chat)| chat.id != chat_id)
            );
            assert!(state.visible_chats().any(|chat| chat.id == "wt"));
            assert!(shell.pr_chat_sheet_open(cx), "the send grew the sheet");
        });
        let frames = wire.serve(&runtime, cx, |_, _| None);
        assert!(
            calls(&frames, methods::MUTATE).is_empty(),
            "Keep per PR deletes nothing"
        );

        cx.update(|_, cx| {
            settings::update(SavePolicy::Debounced, cx, |settings| {
                settings.pr_chat_history = PrChatHistory::Fresh
            })
        });
        page.update(cx, |page, cx| page.open_linked(item_n(8), None, cx));
        let frames = wire.serve(&runtime, cx, |_, _| Some(serde_json::json!({})));
        assert!(
            calls(&frames, methods::MUTATE)
                .iter()
                .all(|frame| frame.params["op"] != "deleteChat"),
            "{frames:?}"
        );
        shell.read_with(cx, |shell, cx| {
            assert_ne!(shell.pr_chat.as_ref().unwrap().chat_id, chat_id);
            assert!(
                settings::current(cx).pr_chat_indexed(&chat_id),
                "a chat kept under Keep per PR stays kept"
            );
            assert!(shell.state.read(cx).is_hidden_chat(&chat_id));
        });
    }

    #[gpui::test]
    fn under_fresh_a_chat_no_panel_holds_comes_back_as_a_thread(cx: &mut TestAppContext) {
        let (runtime, shell, cx, mut wire, _dir) = setup(cx);
        let _guard = runtime.enter();
        cx.update(|_, cx| {
            settings::update(SavePolicy::Debounced, cx, |settings| {
                settings.pr_chat_history = PrChatHistory::Fresh;
                for (id, fresh) in [("orphan", true), ("kept", false)] {
                    settings.index_pr_chat(
                        "github.com/acme/zeron#7",
                        PrChatSession {
                            chat_id: id.into(),
                            created_at: Utc::now(),
                            posted: Vec::new(),
                            dismissed: Vec::new(),
                            fresh,
                        },
                    );
                }
            })
        });
        shell.update(cx, |shell, cx| {
            shell.sync_hidden_chats(cx);
            shell.state.update(cx, |state, cx| {
                state.apply_chats(vec![
                    row("wt", false, serde_json::json!({})),
                    row("orphan", false, serde_json::json!({})),
                    row("kept", false, serde_json::json!({})),
                ]);
                cx.notify();
            })
        });
        cx.run_until_parked();
        shell.read_with(cx, |shell, cx| {
            let settings = settings::current(cx);
            assert!(!settings.pr_chat_indexed("orphan"));
            assert!(settings.pr_chat_indexed("kept"), "made under Keep per PR");
            let state = shell.state.read(cx);
            assert!(state.visible_chats().any(|chat| chat.id == "orphan"));
            assert!(state.is_hidden_chat("kept"));
        });

        let (chat_id, _) = send_review(&runtime, &shell, cx, &mut wire);
        shell.read_with(cx, |_, cx| {
            assert!(settings::current(cx).pr_chat_fresh(&chat_id))
        });
    }

    fn open_pr_chat(
        runtime: &tokio::runtime::Runtime,
        shell: &Entity<Shell>,
        cx: &mut VisualTestContext,
        wire: &mut Wire,
    ) -> (String, Entity<Composer>, Entity<AppState>) {
        shell.update(cx, |shell, cx| shell.open_pull_requests(cx));
        let page = shell.read_with(cx, |shell, _| shell.pull_requests_page.clone().unwrap());
        page.update(cx, |page, cx| page.open_linked(item(), None, cx));
        wire.serve(runtime, cx, |method, _| answer_open(method));
        shell.read_with(cx, |shell, _| {
            let tab = shell.pr_chat_tab().expect("chat minted locally");
            (
                shell.pr_chat.as_ref().unwrap().chat_id.clone(),
                tab.composer.clone(),
                tab.state.clone(),
            )
        })
    }

    #[gpui::test]
    fn the_mini_chat_sends_with_any_agent_and_the_picked_permission_mode(cx: &mut TestAppContext) {
        let (runtime, shell, cx, mut wire, _dir) = setup(cx);
        let _guard = runtime.enter();
        let (chat_id, composer, side_state) = open_pr_chat(&runtime, &shell, cx, &mut wire);
        let config: zeron_proto::ChatConfig = serde_json::from_value(serde_json::json!({
            "harness": "pi", "sandbox": "workspace-write", "permissionMode": "auto",
        }))
        .unwrap();
        side_state.update(cx, |state, _| state.apply_chat_config(&chat_id, config));
        composer.update(cx, |composer, cx| {
            composer.send_prompt("Approve and merge it".into(), cx)
        });
        let frames = wire.serve(&runtime, cx, |method, params| {
            (method == methods::MUTATE && params["op"] == "createChat")
                .then(|| serde_json::json!({}))
        });
        let create = calls(&frames, methods::MUTATE);
        assert_eq!(create.len(), 1, "{frames:?}");
        assert_eq!(create[0].params["config"]["harness"], "pi");
        assert_eq!(create[0].params["config"]["permissionMode"], "auto");
        assert!(composer.read_with(cx, |composer, _| composer.failure().is_none()));
        let frames = wire.serve(&runtime, cx, |method, _| {
            (method == methods::QUEUE_COMMAND).then(|| serde_json::json!({}))
        });
        let run = calls(&frames, methods::QUEUE_COMMAND);
        assert_eq!(run.len(), 1, "{frames:?}");
        assert_eq!(
            run[0].params["command"]["request"]["permissionMode"],
            "auto"
        );
    }

    #[gpui::test]
    fn the_mini_chat_composer_shows_the_footer_and_permission_chip(cx: &mut TestAppContext) {
        let (runtime, shell, cx, mut wire, _dir) = setup(cx);
        let _guard = runtime.enter();
        let (_, composer, _) = open_pr_chat(&runtime, &shell, cx, &mut wire);
        let panel = cx.debug_bounds("pr-chat-dock").expect("bar drawn");
        for selector in ["composer-footer", "picker-permission"] {
            let bounds = cx
                .debug_bounds(selector)
                .unwrap_or_else(|| panic!("{selector} drawn"));
            assert!(
                panel.contains(&bounds.origin) && panel.contains(&bounds.bottom_right()),
                "{selector} {bounds:?} inside {panel:?}"
            );
        }
        let permission = cx.debug_bounds("picker-permission").unwrap();
        let footer = cx.debug_bounds("composer-footer").unwrap();
        assert!(
            (permission.center().y - footer.center().y).abs() < px(4.0),
            "one footer row: {permission:?} {footer:?}"
        );
        assert_eq!(
            cx.update(|_, cx| composer
                .read(cx)
                .pickers()
                .read(cx)
                .effective_permission_mode(cx)),
            zeron_proto::PermissionMode::Full,
            "a new PR chat starts in the same mode as any new chat"
        );
    }

    #[gpui::test]
    fn sending_grows_the_sheet_and_fold_or_escape_shrinks_it_back_to_the_bar(
        cx: &mut TestAppContext,
    ) {
        let (runtime, shell, cx, mut wire, _dir) = setup(cx);
        let _guard = runtime.enter();
        let (chat_id, composer) = send_review(&runtime, &shell, cx, &mut wire);
        wire.serve(&runtime, cx, |method, _| {
            (method == methods::QUEUE_COMMAND).then(|| serde_json::json!({}))
        });
        fill(&shell, cx, "Looks good.");
        let sheet = cx
            .debug_bounds("pr-chat-sheet")
            .expect("the send grew the sheet");
        let dock = cx.debug_bounds("pr-chat-dock").expect("one surface");
        let bar = cx
            .debug_bounds("pr-chat-bar")
            .expect("composer at the bottom");
        assert!(
            inside(dock, sheet) && inside(dock, bar),
            "{dock:?} {sheet:?} {bar:?}"
        );
        assert!(
            sheet.bottom() <= bar.top() + px(0.5),
            "{sheet:?} above {bar:?}"
        );
        assert!(cx.debug_bounds("pull-requests-chat-active").is_some());
        for selector in [
            "pr-chat-sessions",
            "pr-chat-new",
            "pr-chat-promote",
            "pr-chat-fold",
        ] {
            assert!(
                cx.debug_bounds(selector).is_some(),
                "{selector} in the header"
            );
        }
        assert!(cx.debug_bounds("pr-chat-minimize").is_none());
        assert!(cx.debug_bounds("pr-chat-close").is_none());
        assert!(cx.debug_bounds("pr-chat-pill").is_none());

        click(cx, "pr-chat-fold");
        assert!(cx.debug_bounds("pr-chat-sheet").is_none());
        assert!(cx.debug_bounds("pr-chat-dock").is_some(), "the bar stays");
        assert!(cx.debug_bounds("pull-requests-chat-active").is_none());
        shell.read_with(cx, |shell, cx| {
            assert!(!shell.pr_chat_sheet_open(cx));
            assert_eq!(shell.pr_chat.as_ref().unwrap().chat_id, chat_id, "kept");
        });

        click(cx, "pull-requests-chat");
        assert!(
            cx.debug_bounds("pr-chat-sheet").is_some(),
            "the chat button unfolds"
        );
        cx.update(|window, cx| {
            assert!(
                composer
                    .read(cx)
                    .focus_handle(cx)
                    .contains_focused(window, cx),
                "and focuses the bar"
            );
        });
        cx.simulate_keystrokes("escape");
        cx.run_until_parked();
        assert!(cx.debug_bounds("pr-chat-sheet").is_none(), "Esc folds");
        shell.read_with(cx, |shell, _| {
            assert!(matches!(shell.route, Route::PullRequests));
            assert_eq!(shell.pr_chat.as_ref().unwrap().chat_id, chat_id);
        });

        let git = cx.debug_bounds("pull-requests-tab-Git").expect("git tab");
        cx.simulate_click(git.center(), gpui::Modifiers::default());
        wire.serve(&runtime, cx, |_, _| None);
        assert!(
            cx.debug_bounds("pr-chat-dock").is_none(),
            "the Git tab's commit button owns that corner"
        );
        let prs = cx
            .debug_bounds("pull-requests-tab-PullRequests")
            .expect("pull requests tab");
        cx.simulate_click(prs.center(), gpui::Modifiers::default());
        wire.serve(&runtime, cx, |_, _| None);
        assert!(cx.debug_bounds("pr-chat-dock").is_some());
        shell.read_with(cx, |shell, _| {
            assert_eq!(shell.pr_chat.as_ref().unwrap().chat_id, chat_id)
        });
    }

    #[gpui::test]
    fn selecting_another_pr_keeps_the_sheet_and_switches_to_its_session(cx: &mut TestAppContext) {
        let (runtime, shell, cx, mut wire, _dir) = setup(cx);
        let _guard = runtime.enter();
        shell.update(cx, |shell, cx| {
            shell.state.update(cx, |state, _| {
                state.chats.push(row("s8", false, serde_json::json!({})))
            })
        });
        index(cx, "github.com/acme/zeron#8", &[("s8", 1)]);
        let (chat_id, composer) = send_review(&runtime, &shell, cx, &mut wire);
        wire.serve(&runtime, cx, |method, _| {
            (method == methods::QUEUE_COMMAND).then(|| serde_json::json!({}))
        });
        shell.update(cx, |shell, cx| {
            shell.state.update(cx, |state, _| {
                state
                    .chats
                    .push(row(&chat_id, false, serde_json::json!({})))
            })
        });
        fill(&shell, cx, "Looks good.");
        assert!(cx.debug_bounds("pr-chat-sheet").is_some());

        let page = shell.read_with(cx, |shell, _| shell.pull_requests_page.clone().unwrap());
        page.update(cx, |page, cx| page.open_linked(item_n(8), None, cx));
        let frames = wire.serve(&runtime, cx, |method, _| answer_open(method));
        assert!(calls(&frames, methods::MUTATE).is_empty(), "{frames:?}");
        shell.read_with(cx, |shell, cx| {
            let panel = shell.pr_chat.as_ref().unwrap();
            assert_eq!(panel.chat_id, "s8", "the PR's newest session");
            assert!(shell.pr_chat_sheet_open(cx), "the list click does not fold");
            assert!(settings::current(cx).pr_chat_indexed(&chat_id), "untouched");
            assert!(shell.state.read(cx).is_hidden_chat(&chat_id));
        });
        assert!(cx.debug_bounds("pr-chat-sheet").is_some());

        let item9 = item_n(9);
        page.update(cx, |page, cx| page.open_linked(item9, None, cx));
        wire.serve(&runtime, cx, |method, _| answer_open(method));
        shell.read_with(cx, |shell, cx| {
            let panel = shell.pr_chat.as_ref().unwrap();
            assert!(!panel.indexed, "no session for #9: a new chat");
            assert_ne!(panel.chat_id, "s8");
            assert!(!shell.pr_chat_sheet_open(cx), "nothing to show");
        });
        assert!(cx.debug_bounds("pr-chat-sheet").is_none());
        assert!(cx.debug_bounds("pr-chat-dock").is_some());

        page.update(cx, |page, cx| page.open_linked(item(), None, cx));
        wire.serve(&runtime, cx, |method, _| answer_open(method));
        shell.read_with(cx, |shell, cx| {
            assert_eq!(shell.pr_chat.as_ref().unwrap().chat_id, chat_id);
            assert!(
                shell.pr_chat_sheet_open(cx),
                "a live session grows the sheet"
            );
            assert_ne!(
                shell.pr_chat_tab().unwrap().composer,
                composer,
                "a fresh tab for the resumed session"
            );
        });
    }

    #[gpui::test]
    fn the_sheet_follows_its_content_and_caps_at_sixty_percent_of_the_page(
        cx: &mut TestAppContext,
    ) {
        let (runtime, shell, cx, mut wire, _dir) = setup(cx);
        let _guard = runtime.enter();
        send_review(&runtime, &shell, cx, &mut wire);
        wire.serve(&runtime, cx, |method, _| {
            (method == methods::QUEUE_COMMAND).then(|| serde_json::json!({}))
        });
        fill(&shell, cx, "Short.");
        settle(cx);
        let column = cx.debug_bounds("pull-requests-column").unwrap();
        let page_height = column.size.height - px(Theme::TITLEBAR_HEIGHT);
        let cap = page_height * PR_CHAT_SHEET_CAP;
        let short = cx.debug_bounds("pr-chat-dock").unwrap();
        assert!(
            short.size.height < cap - px(40.0),
            "{short:?} follows a short reply"
        );

        let long = (0..80)
            .map(|line| format!("Line {line} of a long review."))
            .collect::<Vec<_>>()
            .join("\n\n");
        fill(&shell, cx, &long);
        settle(cx);
        let tall = cx.debug_bounds("pr-chat-dock").unwrap();
        assert!(tall.size.height > short.size.height, "{tall:?} grew");
        assert!(
            tall.size.height <= cap + px(0.5),
            "{tall:?} capped at {cap:?}"
        );
        assert!(
            (tall.size.height - cap).abs() < px(1.0),
            "{tall:?} reaches the cap"
        );
        let bar = cx.debug_bounds("pr-chat-bar").unwrap();
        assert!(inside(tall, bar), "the composer stays inside: {bar:?}");
    }

    #[gpui::test]
    fn escape_folds_the_unfocused_sheet_before_closing_the_page(cx: &mut TestAppContext) {
        let (runtime, shell, cx, mut wire, _dir) = setup(cx);
        let _guard = runtime.enter();
        let (_, composer, _) = open_pr_chat(&runtime, &shell, cx, &mut wire);
        fill(&shell, cx, "Looks good.");
        let page = shell.read_with(cx, |shell, _| shell.pull_requests_page.clone().unwrap());
        cx.update(|window, cx| page.update(cx, |page, cx| page.focus_search(window, cx)));
        cx.run_until_parked();
        cx.update(|window, cx| {
            assert!(
                !composer
                    .read(cx)
                    .focus_handle(cx)
                    .contains_focused(window, cx)
            );
        });
        shell.read_with(cx, |shell, cx| assert!(shell.pr_chat_sheet_open(cx)));
        cx.simulate_keystrokes("escape");
        shell.read_with(cx, |shell, cx| {
            assert!(matches!(shell.route, Route::PullRequests));
            assert!(!shell.pr_chat_sheet_open(cx));
            assert!(shell.pr_chat.is_some());
        });
        cx.simulate_keystrokes("escape");
        shell.read_with(cx, |shell, _| assert!(matches!(shell.route, Route::Chat)));
    }

    #[gpui::test]
    fn fresh_switching_prs_during_the_first_send_deletes_the_chat_once_it_lands(
        cx: &mut TestAppContext,
    ) {
        let (runtime, shell, cx, mut wire, _dir) = setup(cx);
        let _guard = runtime.enter();
        cx.update(|_, cx| {
            settings::update(SavePolicy::Debounced, cx, |settings| {
                settings.pr_chat_history = PrChatHistory::Fresh;
            })
        });
        let (chat_id, composer, side_state) = open_pr_chat(&runtime, &shell, cx, &mut wire);
        let config: zeron_proto::ChatConfig = serde_json::from_value(serde_json::json!({
            "harness": "mock", "sandbox": "workspace-write",
        }))
        .unwrap();
        side_state.update(cx, |state, _| state.apply_chat_config(&chat_id, config));
        composer.update(cx, |composer, cx| {
            composer.send_prompt("Review this".into(), cx)
        });
        let frames = wire.serve(&runtime, cx, |_, _| None);
        let create = calls(&frames, methods::MUTATE);
        assert_eq!(create.len(), 1, "{frames:?}");
        assert_eq!(create[0].params["op"], "createChat");
        let create_id = create[0].id.clone();

        let page = shell.read_with(cx, |shell, _| shell.pull_requests_page.clone().unwrap());
        page.update(cx, |page, cx| page.open_linked(item_n(8), None, cx));
        shell.update(cx, |shell, cx| {
            assert_ne!(shell.pr_chat.as_ref().unwrap().chat_id, chat_id);
            assert!(shell.state.read(cx).is_hidden_chat(&chat_id));
        });
        let frames = wire.serve(&runtime, cx, |_, _| None);
        assert!(calls(&frames, methods::MUTATE).is_empty(), "{frames:?}");
        assert!(cx.debug_bounds("pr-chat-dock").is_some());

        let reply = ServerFrame {
            id: create_id,
            ok: Some(serde_json::json!({})),
            ..Default::default()
        };
        wire.replies
            .try_send(serde_json::to_string(&reply).unwrap())
            .unwrap();
        let mut frames = wire.serve(&runtime, cx, |method, _| {
            (method == methods::QUEUE_COMMAND).then(|| serde_json::json!({}))
        });
        assert_eq!(
            calls(&frames, methods::QUEUE_COMMAND).len(),
            1,
            "{frames:?}"
        );
        frames.extend(wire.serve(&runtime, cx, |_, _| Some(serde_json::json!({}))));
        let deletes: Vec<_> = calls(&frames, methods::MUTATE)
            .into_iter()
            .filter(|frame| frame.params["op"] == "deleteChat")
            .collect();
        assert_eq!(deletes.len(), 1, "{frames:?}");
        assert_eq!(deletes[0].params["chatId"], chat_id.as_str());

        let row: zeron_proto::Chat = serde_json::from_value(serde_json::json!({
            "id": chat_id, "deviceId": "local", "spaceId": "s1", "cwd": "/wt/x",
            "archived": false, "createdAt": Utc::now(),
        }))
        .unwrap();
        shell.update(cx, |shell, cx| {
            assert!(shell.pr_chat_trash.parked.is_empty());
            assert!(!settings::current(cx).pr_chat_indexed(&chat_id));
            shell.state.update(cx, |state, _| state.chats.push(row));
            let state = shell.state.read(cx);
            assert!(state.visible_chats().all(|chat| chat.id != chat_id));
        });
    }

    fn row(id: &str, archived: bool, extra: serde_json::Value) -> zeron_proto::Chat {
        let mut row = serde_json::json!({
            "id": id, "deviceId": "local", "spaceId": "s1", "cwd": "/wt/x",
            "archived": archived, "createdAt": Utc::now() - chrono::Duration::days(3),
            "config": { "harness": "mock", "sandbox": "workspace-write" },
        });
        row.as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        serde_json::from_value(row).unwrap()
    }

    fn index(cx: &mut VisualTestContext, scope: &str, sessions: &[(&str, i64)]) {
        cx.update(|_, cx| {
            settings::update(SavePolicy::Debounced, cx, |settings| {
                for (id, days) in sessions {
                    settings.index_pr_chat(
                        scope,
                        PrChatSession {
                            chat_id: (*id).into(),
                            created_at: Utc::now() - chrono::Duration::days(*days),
                            posted: Vec::new(),
                            dismissed: Vec::new(),
                            fresh: false,
                        },
                    );
                }
            })
        });
    }

    fn settle(cx: &mut VisualTestContext) {
        for _ in 0..4 {
            cx.update(|window, _| window.refresh());
            cx.run_until_parked();
        }
    }

    fn click(cx: &mut VisualTestContext, selector: &'static str) {
        settle(cx);
        let bounds = cx
            .debug_bounds(selector)
            .unwrap_or_else(|| panic!("{selector} drawn"));
        cx.simulate_click(bounds.center(), gpui::Modifiers::default());
        cx.run_until_parked();
    }

    #[gpui::test]
    fn keep_per_pr_resumes_the_newest_session_and_switches_between_sessions(
        cx: &mut TestAppContext,
    ) {
        let (runtime, shell, cx, mut wire, _dir) = setup(cx);
        let _guard = runtime.enter();
        let yesterday = Utc::now() - chrono::Duration::days(1);
        shell.update(cx, |shell, cx| {
            shell.state.update(cx, |state, _| {
                state.chats.push(row("older", false, serde_json::json!({})));
                state.chats.push(row(
                    "newer",
                    false,
                    serde_json::json!({
                        "title": "Quick review", "lastMessageAt": yesterday,
                        "lastMessagePreview": "Three points",
                    }),
                ));
            })
        });
        index(cx, "github.com/acme/zeron#7", &[("older", 3), ("newer", 1)]);
        shell.update(cx, |shell, cx| shell.sync_hidden_chats(cx));

        shell.update(cx, |shell, cx| shell.open_pull_requests(cx));
        wire.serve(&runtime, cx, |method, _| {
            (method == methods::LIST_CHANGE_REQUESTS).then(|| {
                serde_json::json!({
                    "viewers": {}, "items": [item_n(7), item_n(8)],
                    "errors": [], "truncated": false,
                })
            })
        });
        assert!(cx.debug_bounds("pr-chat-badge-7").is_some(), "bubble on #7");
        assert!(
            cx.debug_bounds("pr-chat-badge-8").is_none(),
            "#8 has no chat"
        );

        let page = shell.read_with(cx, |shell, _| shell.pull_requests_page.clone().unwrap());
        page.update(cx, |page, cx| page.open_linked(item(), None, cx));
        let frames = wire.serve(&runtime, cx, |method, _| {
            (method == methods::GET_CHANGE_REQUEST).then(detail)
        });
        assert!(
            calls(&frames, methods::PREPARE_CHANGE_REQUEST_CHAT).is_empty(),
            "a resumed session mints nothing: {frames:?}"
        );
        shell.read_with(cx, |shell, cx| {
            let panel = shell.pr_chat.as_ref().unwrap();
            assert_eq!(panel.chat_id, "newer");
            assert!(panel.indexed);
            assert_eq!(panel.earlier.as_deref(), Some("Yesterday · Quick review"));
            let tab = shell.pr_chat_tab().unwrap();
            assert!(!tab.state.read(cx).side_chat_unsaved());
            assert_eq!(tab.state.read(cx).selected_chat.as_deref(), Some("newer"));
            assert!(
                shell.pr_chat_sheet_open(cx),
                "a PR with a live session grows the sheet"
            );
        });
        assert!(cx.debug_bounds("pr-chat-earlier").is_some());
        assert!(cx.debug_bounds("pr-chat-continue-1").is_some());

        click(cx, "pr-chat-sessions");
        assert!(cx.debug_bounds("pr-chat-sessions-menu").is_some());
        assert!(
            cx.debug_bounds("pr-chat-session-2").is_none(),
            "two sessions"
        );
        click(cx, "pr-chat-session-1");
        let frames = wire.serve(&runtime, cx, |_, _| None);
        assert!(calls(&frames, methods::PREPARE_CHANGE_REQUEST_CHAT).is_empty());
        assert!(cx.debug_bounds("pr-chat-sessions-menu").is_none());
        shell.read_with(cx, |shell, _| {
            let panel = shell.pr_chat.as_ref().unwrap();
            assert_eq!(panel.chat_id, "older");
            assert!(panel.earlier.is_none(), "an older session with no messages");
        });

        click(cx, "pr-chat-sessions");
        click(cx, "pr-chat-session-0");
        shell.read_with(cx, |shell, _| {
            assert_eq!(shell.pr_chat.as_ref().unwrap().chat_id, "newer")
        });
        click(cx, "pr-chat-continue-0");
        let frames = wire.serve(&runtime, cx, |method, _| {
            (method == methods::QUEUE_COMMAND).then(|| serde_json::json!({}))
        });
        assert!(
            calls(&frames, methods::MUTATE)
                .iter()
                .all(|frame| frame.params["op"] == "markChatSeen"),
            "the session exists already: {frames:?}"
        );
        let run = calls(&frames, methods::QUEUE_COMMAND);
        assert_eq!(run.len(), 1, "{frames:?}");
        let request = &run[0].params["command"]["request"];
        assert_eq!(request["prompt"], "What changed since we last talked?");
        assert_eq!(request["permissionMode"], "full");
        shell.read_with(cx, |shell, _| {
            assert!(shell.pr_chat.as_ref().unwrap().earlier.is_none())
        });
        assert!(cx.debug_bounds("pr-chat-earlier").is_none());

        click(cx, "pr-chat-sessions");
        click(cx, "pr-chat-new-session");
        let frames = wire.serve(&runtime, cx, |method, _| match method {
            methods::PREPARE_CHANGE_REQUEST_CHAT => Some(serde_json::json!({
                "spaceId": "s1", "root": "/repo", "cwd": "/repo",
                "worktree": false, "onHead": false,
            })),
            _ => None,
        });
        assert_eq!(
            calls(&frames, methods::PREPARE_CHANGE_REQUEST_CHAT).len(),
            1
        );
        assert!(
            calls(&frames, methods::MUTATE)
                .iter()
                .all(|frame| frame.params["op"] != "deleteChat"),
            "nothing deleted: {frames:?}"
        );
        shell.read_with(cx, |shell, cx| {
            let panel = shell.pr_chat.as_ref().unwrap();
            assert!(panel.chat_id != "newer" && panel.chat_id != "older");
            assert!(!panel.indexed);
            let settings = settings::current(cx);
            assert!(settings.pr_chat_indexed("newer") && settings.pr_chat_indexed("older"));
            assert_eq!(shell.pr_chat_session_rows(cx).len(), 3);
        });
        click(cx, "pr-chat-sessions");
        assert!(cx.debug_bounds("pr-chat-session-2").is_some());

        click(cx, "pr-chat-session-0");

        cx.update(|_, cx| {
            settings::update(SavePolicy::Debounced, cx, |settings| {
                settings.pr_chat_history = PrChatHistory::Fresh
            })
        });
        shell.update(cx, |_, cx| cx.notify());
        page.update(cx, |_, cx| cx.notify());
        cx.run_until_parked();
        assert!(cx.debug_bounds("pr-chat-badge-7").is_none());
        click(cx, "pr-chat-sessions");
        assert!(
            cx.debug_bounds("pr-chat-session-0").is_some(),
            "Fresh keeps the menu"
        );
        assert!(
            cx.debug_bounds("pr-chat-session-1").is_none(),
            "only the open chat"
        );
        assert!(cx.debug_bounds("pr-chat-new-session").is_some());
    }

    #[gpui::test]
    fn keep_per_pr_keeps_a_chat_sending_when_its_panel_is_replaced(cx: &mut TestAppContext) {
        let (runtime, shell, cx, mut wire, _dir) = setup(cx);
        let _guard = runtime.enter();
        let (chat_id, composer, side_state) = open_pr_chat(&runtime, &shell, cx, &mut wire);
        let config: zeron_proto::ChatConfig = serde_json::from_value(serde_json::json!({
            "harness": "mock", "sandbox": "workspace-write",
        }))
        .unwrap();
        side_state.update(cx, |state, _| state.apply_chat_config(&chat_id, config));
        composer.update(cx, |composer, cx| {
            composer.send_prompt("Review this".into(), cx)
        });
        let frames = wire.serve(&runtime, cx, |_, _| None);
        let create = calls(&frames, methods::MUTATE);
        assert_eq!(create.len(), 1, "{frames:?}");
        let create_id = create[0].id.clone();

        let page = shell.read_with(cx, |shell, _| shell.pull_requests_page.clone().unwrap());
        page.update(cx, |page, cx| page.open_linked(item_n(8), None, cx));
        let frames = wire.serve(&runtime, cx, |method, _| answer_open(method));
        assert!(calls(&frames, methods::MUTATE).is_empty(), "{frames:?}");
        shell.read_with(cx, |shell, cx| {
            assert_ne!(shell.pr_chat.as_ref().unwrap().chat_id, chat_id);
            assert!(shell.state.read(cx).is_hidden_chat(&chat_id));
        });

        let reply = ServerFrame {
            id: create_id,
            ok: Some(serde_json::json!({})),
            ..Default::default()
        };
        wire.replies
            .try_send(serde_json::to_string(&reply).unwrap())
            .unwrap();
        let mut frames = wire.serve(&runtime, cx, |method, _| {
            (method == methods::QUEUE_COMMAND).then(|| serde_json::json!({}))
        });
        frames.extend(wire.serve(&runtime, cx, |_, _| None));
        let run = calls(&frames, methods::QUEUE_COMMAND);
        assert_eq!(run.len(), 1, "the send still reaches the agent: {frames:?}");
        assert!(
            run[0].params["command"]["request"]["prompt"]
                .as_str()
                .unwrap()
                .starts_with("Review this"),
            "{frames:?}"
        );
        assert!(calls(&frames, methods::MUTATE).is_empty(), "{frames:?}");

        let landed = row(&chat_id, false, serde_json::json!({}));
        shell.update(cx, |shell, cx| {
            assert!(shell.pr_chat_trash.parked.is_empty());
            let sessions = settings::current(cx).pr_chat_sessions;
            assert_eq!(sessions["github.com/acme/zeron#7"][0].chat_id, chat_id);
            assert!(!sessions.contains_key("github.com/acme/zeron#8"));
            shell.state.update(cx, |state, _| state.chats.push(landed));
            let state = shell.state.read(cx);
            assert!(state.visible_chats().all(|chat| chat.id != chat_id));
        });
        page.update(cx, |page, cx| page.open_linked(item(), None, cx));
        let frames = wire.serve(&runtime, cx, |method, _| {
            (method == methods::GET_CHANGE_REQUEST).then(detail)
        });
        assert!(calls(&frames, methods::PREPARE_CHANGE_REQUEST_CHAT).is_empty());
        shell.read_with(cx, |shell, _| {
            assert_eq!(shell.pr_chat.as_ref().unwrap().chat_id, chat_id, "resumed")
        });
    }

    #[gpui::test]
    fn the_index_drops_only_chats_the_state_knows_are_deleted_or_archived(cx: &mut TestAppContext) {
        let (runtime, shell, cx, _wire, _dir) = setup(cx);
        let _guard = runtime.enter();
        index(
            cx,
            "github.com/acme/zeron#7",
            &[("deleted", 2), ("archived", 2), ("kept", 2), ("remote", 2)],
        );
        shell.update(cx, |shell, cx| {
            shell.sync_hidden_chats(cx);
            shell.state.update(cx, |_, cx| cx.notify());
        });
        cx.run_until_parked();
        shell.read_with(cx, |_, cx| {
            assert_eq!(
                settings::current(cx).pr_chat_ids().count(),
                4,
                "nothing pruned before the chats frame"
            )
        });
        let ids = |cx: &mut VisualTestContext| {
            cx.update(|_, cx| {
                settings::current(cx)
                    .pr_chat_ids()
                    .map(str::to_owned)
                    .collect::<Vec<_>>()
            })
        };
        let frame = |cx: &mut VisualTestContext, rows: Vec<zeron_proto::Chat>| {
            shell.update(cx, |shell, cx| {
                shell.state.update(cx, |state, cx| {
                    state.apply_chats(rows);
                    cx.notify();
                })
            });
            cx.run_until_parked();
        };
        frame(
            cx,
            vec![
                row("wt", false, serde_json::json!({})),
                row("deleted", false, serde_json::json!({})),
                row("archived", true, serde_json::json!({})),
                row("kept", false, serde_json::json!({})),
            ],
        );
        assert_eq!(
            ids(cx),
            ["deleted", "kept", "remote"],
            "a chat this profile does not carry is not proof of a delete"
        );
        frame(
            cx,
            vec![
                row("wt", false, serde_json::json!({})),
                row("kept", false, serde_json::json!({})),
            ],
        );
        assert_eq!(ids(cx), ["kept", "remote"]);
        shell.read_with(cx, |shell, cx| {
            let state = shell.state.read(cx);
            assert!(state.is_hidden_chat("kept"));
            assert!(state.is_hidden_chat("remote"));
            assert!(!state.is_hidden_chat("deleted"));
            assert!(!state.is_hidden_chat("archived"));
        });
        shell.update(cx, |shell, cx| {
            shell
                .state
                .update(cx, |state, cx| state.prepare_runtime_replacement(cx))
        });
        frame(cx, vec![row("wt", false, serde_json::json!({}))]);
        assert_eq!(
            ids(cx),
            ["kept", "remote"],
            "another profile's frame lacks these chats"
        );
    }

    #[derive(Default)]
    struct Recorder {
        calls: std::sync::Mutex<Vec<(String, serde_json::Value)>>,
    }

    #[async_trait::async_trait]
    impl zeron_rpc::RpcService for Recorder {
        async fn handle(
            &self,
            method: &str,
            params: serde_json::Value,
        ) -> Result<zeron_rpc::RpcReply, zeron_rpc::RpcError> {
            use futures::StreamExt;
            self.calls.lock().unwrap().push((method.to_owned(), params));
            if method.starts_with("Watch") {
                return Ok(zeron_rpc::RpcReply::Stream(
                    futures::stream::pending().boxed(),
                ));
            }
            Ok(zeron_rpc::RpcReply::Value(serde_json::json!({})))
        }
    }

    impl Recorder {
        fn writes(&self) -> Vec<(String, serde_json::Value)> {
            const WRITES: [&str; 4] = [
                methods::POST_CHANGE_REQUEST_COMMENT,
                methods::SUBMIT_CHANGE_REQUEST_REVIEW,
                methods::REPLY_TO_CHANGE_REQUEST_THREAD,
                methods::RUN_CHANGE_REQUEST_ACTION,
            ];
            self.calls
                .lock()
                .unwrap()
                .iter()
                .filter(|(method, _)| WRITES.contains(&method.as_str()))
                .cloned()
                .collect()
        }
    }

    fn reply_entry(text: &str) -> Vec<zeron_doc::SessionMessageEntry> {
        use zeron_doc::{MessagePart, MessageRole, MessageStatus, SessionMessageEntry};
        let entry = |id: &str, role, text: &str, status| SessionMessageEntry {
            id: id.into(),
            role,
            parts: vec![MessagePart::Text {
                id: format!("{id}-t"),
                text: text.into(),
            }],
            created_at: 0,
            device_id: "local".into(),
            status,
            continuation_of: None,
            from_chat_id: None,
            fork_import: false,
            answers_request_id: None,
            duration_ms: None,
        };
        vec![
            entry("u1", MessageRole::User, "Review this", None),
            entry(
                "a1",
                MessageRole::Assistant,
                text,
                Some(MessageStatus::Complete),
            ),
        ]
    }

    #[gpui::test]
    fn suggestion_cards_post_only_on_click_and_remember_it(cx: &mut TestAppContext) {
        let recorder = std::sync::Arc::new(Recorder::default());
        let service = recorder.clone();
        let (runtime, shell, cx, _dir) = boot(cx, move |runtime| {
            let _guard = runtime.enter();
            zeron_rpc::memory_client(service)
        });
        let _guard = runtime.enter();
        let pump = |cx: &mut VisualTestContext| Wire::pump(&runtime, cx);
        shell.update(cx, |shell, cx| {
            shell.state.update(cx, |state, _| {
                state.chats.push(row("s7", false, serde_json::json!({})))
            })
        });
        index(cx, "github.com/acme/zeron#7", &[("s7", 1)]);
        shell.update(cx, |shell, cx| shell.open_pull_requests(cx));
        let page = shell.read_with(cx, |shell, _| shell.pull_requests_page.clone().unwrap());
        page.update(cx, |page, cx| page.open_linked(item(), None, cx));
        pump(cx);
        let fence = |json: &str| format!("```zeron-github-action\n{json}\n```\n");
        let reply = [
            "Two notes before merge.\n\n".to_string(),
            fence(r#"{"action":"review","event":"comment","body":"Pi returns matches."}"#),
            fence(r#"{"action":"reply","body":"No thread id"}"#),
            fence(r#"{"action":"merge","method":"squash"}"#),
        ]
        .concat();
        let inject = |shell: &Entity<Shell>, cx: &mut VisualTestContext| {
            shell.update(cx, |shell, cx| {
                shell.reveal_pr_chat(cx);
                let tab = shell.pr_chat_tab().expect("resumed session");
                assert_eq!(shell.pr_chat.as_ref().unwrap().chat_id, "s7");
                tab.state.update(cx, |state, cx| {
                    state.apply_transcript(reply_entry(&reply));
                    cx.notify();
                });
            });
            pump(cx);
        };
        inject(&shell, cx);

        assert!(cx.debug_bounds("pr-chat-card-0").is_some());
        assert!(cx.debug_bounds("pr-chat-card-1").is_none(), "malformed");
        assert!(cx.debug_bounds("pr-chat-card-2").is_some());
        assert!(cx.debug_bounds("pr-chat-card-request-changes-0").is_some());
        assert!(cx.debug_bounds("pr-chat-card-edit-2").is_none());
        assert!(recorder.writes().is_empty(), "nothing before a click");

        click(cx, "pr-chat-card-edit-0");
        let editor = shell.read_with(cx, |shell, _| {
            let key = PrChatCardRef {
                entry_id: "a1".into(),
                index: 0,
            };
            let card = &shell.pr_chat.as_ref().unwrap().cards[&key];
            card.editor.clone().expect("editing")
        });
        assert!(cx.debug_bounds("pr-chat-card-input-0").is_some());
        assert_eq!(
            cx.update(|_, cx| editor.read(cx).text().to_string()),
            "Pi returns matches."
        );
        cx.update(|_, cx| {
            editor.update(cx, |input, cx| input.set_text("Pi returns `matches`.", cx))
        });
        pump(cx);
        assert!(recorder.writes().is_empty(), "editing writes nothing");
        click(cx, "pr-chat-card-request-changes-0");
        pump(cx);
        let writes = recorder.writes();
        assert_eq!(writes.len(), 1, "{writes:?}");
        assert_eq!(writes[0].0, methods::SUBMIT_CHANGE_REQUEST_REVIEW);
        assert_eq!(writes[0].1["event"], "requestChanges");
        assert_eq!(writes[0].1["body"], "Pi returns `matches`.");
        assert_eq!(writes[0].1["number"], 7);
        assert_eq!(writes[0].1["repository"], "acme/zeron");
        assert!(cx.debug_bounds("pr-chat-card-posted-0").is_some());
        assert!(cx.debug_bounds("pr-chat-card-post-0").is_none());

        click(cx, "pr-chat-card-post-2");
        assert!(cx.debug_bounds("pr-chat-confirm").is_some());
        assert_eq!(recorder.writes().len(), 1, "merge waits for the confirm");
        click(cx, "pr-chat-confirm-cancel");
        pump(cx);
        assert!(cx.debug_bounds("pr-chat-confirm").is_none());
        assert_eq!(recorder.writes().len(), 1, "cancel writes nothing");
        click(cx, "pr-chat-card-post-2");
        click(cx, "pr-chat-confirm-ok");
        pump(cx);
        let writes = recorder.writes();
        assert_eq!(writes.len(), 2, "{writes:?}");
        assert_eq!(writes[1].0, methods::RUN_CHANGE_REQUEST_ACTION);
        assert_eq!(
            writes[1].1["action"],
            serde_json::json!({ "kind": "merge", "method": "squash" })
        );
        assert!(cx.debug_bounds("pr-chat-card-posted-2").is_some());

        let posted = cx
            .update(|_, cx| settings::pr_chat_session("s7", cx))
            .map(|session| session.posted)
            .unwrap_or_default();
        let saved: Vec<(&str, usize, &str)> = posted
            .iter()
            .map(|p| (p.card.entry_id.as_str(), p.card.index, p.label.as_str()))
            .collect();
        assert_eq!(
            saved,
            [("a1", 0, "Requested changes on #7"), ("a1", 2, "Merged #7")]
        );

        shell.update(cx, |shell, cx| shell.discard_pr_chat(cx));
        inject(&shell, cx);
        assert!(
            cx.debug_bounds("pr-chat-card-posted-0").is_some(),
            "reopened"
        );
        assert!(cx.debug_bounds("pr-chat-card-posted-2").is_some());
        assert!(cx.debug_bounds("pr-chat-card-post-0").is_none());
        assert!(cx.debug_bounds("pr-chat-card-post-2").is_none());
        assert_eq!(recorder.writes().len(), 2);
    }

    fn send_review(
        runtime: &tokio::runtime::Runtime,
        shell: &Entity<Shell>,
        cx: &mut VisualTestContext,
        wire: &mut Wire,
    ) -> (String, Entity<Composer>) {
        let (chat_id, composer, side_state) = open_pr_chat(runtime, shell, cx, wire);
        let config: zeron_proto::ChatConfig = serde_json::from_value(serde_json::json!({
            "harness": "mock", "sandbox": "workspace-write",
        }))
        .unwrap();
        side_state.update(cx, |state, _| state.apply_chat_config(&chat_id, config));
        composer.update(cx, |composer, cx| {
            composer.send_prompt("Review this".into(), cx)
        });
        let frames = wire.serve(runtime, cx, |method, params| {
            (method == methods::MUTATE && params["op"] == "createChat")
                .then(|| serde_json::json!({}))
        });
        assert_eq!(calls(&frames, methods::MUTATE).len(), 1, "{frames:?}");
        (chat_id, composer)
    }

    fn renames(frames: &[ClientFrame]) -> Vec<(String, String)> {
        calls(frames, methods::MUTATE)
            .into_iter()
            .filter(|frame| frame.params["op"] == "renameChat")
            .map(|frame| {
                (
                    frame.params["chatId"].as_str().unwrap().to_owned(),
                    frame.params["title"].as_str().unwrap().to_owned(),
                )
            })
            .collect()
    }

    #[gpui::test]
    fn promote_opens_the_chat_as_a_sidebar_thread_that_fresh_mode_never_deletes(
        cx: &mut TestAppContext,
    ) {
        let (runtime, shell, cx, mut wire, _dir) = setup(cx);
        let _guard = runtime.enter();
        cx.update(|_, cx| {
            settings::update(SavePolicy::Debounced, cx, |settings| {
                settings.pr_chat_history = PrChatHistory::Fresh;
            })
        });
        let (chat_id, _) = send_review(&runtime, &shell, cx, &mut wire);
        wire.serve(&runtime, cx, |method, _| {
            (method == methods::QUEUE_COMMAND).then(|| serde_json::json!({}))
        });
        shell.update(cx, |shell, cx| {
            assert!(settings::current(cx).pr_chat_indexed(&chat_id));
            shell.state.update(cx, |state, _| {
                state
                    .chats
                    .push(row(&chat_id, false, serde_json::json!({})))
            });
            assert!(shell.state.read(cx).is_hidden_chat(&chat_id));
        });

        click(cx, "pr-chat-promote");
        let frames = wire.serve(&runtime, cx, |_, _| Some(serde_json::json!({})));
        assert_eq!(
            renames(&frames),
            [(chat_id.clone(), "PR #7: Fix the thing".to_string())]
        );
        shell.read_with(cx, |shell, cx| {
            assert!(shell.pr_chat.is_none());
            assert!(matches!(shell.route, Route::Chat));
            assert!(!settings::current(cx).pr_chat_indexed(&chat_id));
            let state = shell.state.read(cx);
            assert_eq!(state.selected_chat.as_deref(), Some(chat_id.as_str()));
            assert!(!state.is_hidden_chat(&chat_id));
            assert!(
                state
                    .sidebar_chats(Utc::now(), None)
                    .iter()
                    .any(|(_, chat)| chat.id == chat_id),
                "the promoted chat is a sidebar thread"
            );
        });

        let (next, _) = send_review(&runtime, &shell, cx, &mut wire);
        let mut frames = wire.serve(&runtime, cx, |method, _| {
            (method == methods::QUEUE_COMMAND).then(|| serde_json::json!({}))
        });
        assert_ne!(next, chat_id);
        shell.update(cx, |shell, cx| shell.new_pr_chat_session(cx));
        frames.extend(wire.serve(&runtime, cx, |_, _| Some(serde_json::json!({}))));
        let deletes: Vec<_> = calls(&frames, methods::MUTATE)
            .into_iter()
            .filter(|frame| frame.params["op"] == "deleteChat")
            .map(|frame| frame.params["chatId"].as_str().unwrap().to_owned())
            .collect();
        assert_eq!(deletes, [next], "Fresh New chat deletes only the new chat");
        shell.read_with(cx, |shell, cx| {
            assert!(!shell.state.read(cx).is_hidden_chat(&chat_id))
        });
    }

    #[gpui::test]
    fn promoting_while_the_send_is_in_flight_does_not_reindex_the_chat(cx: &mut TestAppContext) {
        let (runtime, shell, cx, mut wire, _dir) = setup(cx);
        let _guard = runtime.enter();
        let (chat_id, composer) = send_review(&runtime, &shell, cx, &mut wire);
        let frames = wire.serve(&runtime, cx, |_, _| None);
        let run = calls(&frames, methods::QUEUE_COMMAND);
        assert_eq!(run.len(), 1, "{frames:?}");
        let run_id = run[0].id.clone();
        assert!(composer.read_with(cx, |composer, _| composer.is_sending()));
        shell.read_with(cx, |_, cx| {
            assert!(settings::current(cx).pr_chat_indexed(&chat_id))
        });

        click(cx, "pr-chat-promote");
        shell.read_with(cx, |shell, cx| {
            assert!(shell.pr_chat.is_none());
            assert_eq!(shell.pr_chat_trash.parked.len(), 1, "the send stays alive");
            assert!(!settings::current(cx).pr_chat_indexed(&chat_id));
            assert!(!shell.state.read(cx).is_hidden_chat(&chat_id));
            assert!(matches!(shell.route, Route::Chat));
        });

        let reply = ServerFrame {
            id: run_id,
            ok: Some(serde_json::json!({})),
            ..Default::default()
        };
        wire.replies
            .try_send(serde_json::to_string(&reply).unwrap())
            .unwrap();
        let frames = wire.serve(&runtime, cx, |_, _| Some(serde_json::json!({})));
        assert!(
            calls(&frames, methods::MUTATE)
                .iter()
                .all(|frame| frame.params["op"] != "deleteChat"),
            "{frames:?}"
        );
        shell.update(cx, |shell, cx| {
            assert!(shell.pr_chat_trash.parked.is_empty(), "settled");
            assert!(!settings::current(cx).pr_chat_indexed(&chat_id));
            assert!(settings::current(cx).pr_chat_sessions.is_empty());
            shell.state.update(cx, |state, _| {
                state
                    .chats
                    .push(row(&chat_id, false, serde_json::json!({})))
            });
            assert!(
                shell
                    .state
                    .read(cx)
                    .visible_chats()
                    .any(|chat| chat.id == chat_id)
            );
        });
    }

    fn open_cards(
        cx: &mut TestAppContext,
    ) -> (
        tokio::runtime::Runtime,
        Entity<Shell>,
        &mut VisualTestContext,
        std::sync::Arc<Recorder>,
        tempfile::TempDir,
    ) {
        let recorder = std::sync::Arc::new(Recorder::default());
        let service = recorder.clone();
        let (runtime, shell, cx, dir) = boot(cx, move |runtime| {
            let _guard = runtime.enter();
            zeron_rpc::memory_client(service)
        });
        {
            let _guard = runtime.enter();
            shell.update(cx, |shell, cx| {
                shell.state.update(cx, |state, _| {
                    state.chats.push(row("s7", false, serde_json::json!({})))
                })
            });
            index(cx, "github.com/acme/zeron#7", &[("s7", 1)]);
            shell.update(cx, |shell, cx| shell.open_pull_requests(cx));
            let page = shell.read_with(cx, |shell, _| shell.pull_requests_page.clone().unwrap());
            page.update(cx, |page, cx| page.open_linked(item(), None, cx));
            Wire::pump(&runtime, cx);
        }
        (runtime, shell, cx, recorder, dir)
    }

    fn inject_cards(
        runtime: &tokio::runtime::Runtime,
        shell: &Entity<Shell>,
        cx: &mut VisualTestContext,
    ) {
        let fence = |json: &str| format!("```zeron-github-action\n{json}\n```\n");
        let reply = [
            "Two notes before merge.\n\n".to_string(),
            fence(r#"{"action":"review","event":"comment","body":"Pi returns matches."}"#),
            fence(r#"{"action":"reply","body":"No thread id"}"#),
            fence(r#"{"action":"merge","method":"squash"}"#),
        ]
        .concat();
        shell.update(cx, |shell, cx| {
            shell.reveal_pr_chat(cx);
            let tab = shell.pr_chat_tab().expect("resumed session");
            tab.state.update(cx, |state, cx| {
                state.apply_transcript(reply_entry(&reply));
                cx.notify();
            });
        });
        Wire::pump(runtime, cx);
    }

    #[gpui::test]
    fn card_fences_leave_the_mini_transcript_and_other_fences_stay(cx: &mut TestAppContext) {
        let (runtime, shell, cx, _recorder, _dir) = open_cards(cx);
        let _guard = runtime.enter();
        inject_cards(&runtime, &shell, cx);
        assert!(cx.debug_bounds("pr-chat-card-0").is_some());
        let ids: Vec<String> = shell.read_with(cx, |shell, cx| {
            shell
                .pr_chat_tab()
                .unwrap()
                .transcript
                .read(cx)
                .rows()
                .iter()
                .map(|row| row.id.to_string())
                .filter(|id| id.starts_with("a1#"))
                .collect()
        });
        assert_eq!(
            ids,
            ["a1#a1-t.0", "a1#a1-t.2"],
            "the reply text and the malformed fence"
        );
    }

    #[gpui::test]
    fn a_dismissed_card_stays_dismissed_after_reopening(cx: &mut TestAppContext) {
        let (runtime, shell, cx, recorder, _dir) = open_cards(cx);
        let _guard = runtime.enter();
        inject_cards(&runtime, &shell, cx);
        click(cx, "pr-chat-card-dismiss-2");
        assert!(cx.debug_bounds("pr-chat-card-2").is_none());
        shell.update(cx, |shell, cx| shell.discard_pr_chat(cx));
        inject_cards(&runtime, &shell, cx);
        assert!(cx.debug_bounds("pr-chat-card-0").is_some(), "reopened");
        assert!(
            cx.debug_bounds("pr-chat-card-2").is_none(),
            "still dismissed"
        );
        assert!(recorder.writes().is_empty());
    }

    #[gpui::test]
    fn a_post_in_flight_is_not_offered_again_after_the_panel_is_rebuilt(cx: &mut TestAppContext) {
        let (runtime, shell, cx, mut wire, _dir) = setup(cx);
        let _guard = runtime.enter();
        shell.update(cx, |shell, cx| {
            shell.state.update(cx, |state, _| {
                state.chats.push(row("s7", false, serde_json::json!({})))
            })
        });
        index(cx, "github.com/acme/zeron#7", &[("s7", 1)]);
        shell.update(cx, |shell, cx| shell.open_pull_requests(cx));
        let page = shell.read_with(cx, |shell, _| shell.pull_requests_page.clone().unwrap());
        page.update(cx, |page, cx| page.open_linked(item(), None, cx));
        wire.serve(&runtime, cx, |method, _| {
            (method == methods::GET_CHANGE_REQUEST).then(detail)
        });
        inject_cards(&runtime, &shell, cx);
        click(cx, "pr-chat-card-post-0");
        let frames = wire.serve(&runtime, cx, |_, _| None);
        let posts = calls(&frames, methods::SUBMIT_CHANGE_REQUEST_REVIEW);
        assert_eq!(posts.len(), 1, "{frames:?}");
        let post_id = posts[0].id.clone();

        shell.update(cx, |shell, cx| shell.discard_pr_chat(cx));
        inject_cards(&runtime, &shell, cx);
        click(cx, "pr-chat-card-post-0");
        let frames = wire.serve(&runtime, cx, |_, _| None);
        assert!(
            calls(&frames, methods::SUBMIT_CHANGE_REQUEST_REVIEW).is_empty(),
            "{frames:?}"
        );

        let reply = ServerFrame {
            id: post_id,
            ok: Some(serde_json::json!({})),
            ..Default::default()
        };
        wire.replies
            .try_send(serde_json::to_string(&reply).unwrap())
            .unwrap();
        wire.serve(&runtime, cx, |_, _| None);
        assert!(cx.debug_bounds("pr-chat-card-posted-0").is_some());
        shell.read_with(cx, |shell, _| {
            assert!(shell.pr_chat_trash.posting.is_empty())
        });
    }

    #[gpui::test]
    fn a_resumed_list_session_offers_no_cards(cx: &mut TestAppContext) {
        let (runtime, shell, cx, mut wire, _dir) = setup(cx);
        let _guard = runtime.enter();
        shell.update(cx, |shell, cx| {
            shell.state.update(cx, |state, _| {
                state
                    .chats
                    .push(row("list-1", false, serde_json::json!({})))
            })
        });
        index(cx, "list", &[("list-1", 1)]);
        shell.update(cx, |shell, cx| shell.open_pull_requests(cx));
        wire.serve(&runtime, cx, |method, _| {
            (method == methods::LIST_CHANGE_REQUESTS).then(|| {
                serde_json::json!({
                    "viewers": {}, "items": [item_n(7), item_n(8)],
                    "errors": [], "truncated": false,
                })
            })
        });
        let reply = "```zeron-github-action\n{\"action\":\"merge\",\"number\":7}\n```\n";
        shell.update(cx, |shell, cx| {
            shell.reveal_pr_chat(cx);
            assert_eq!(shell.pr_chat.as_ref().unwrap().chat_id, "list-1");
            let tab = shell.pr_chat_tab().expect("resumed session");
            tab.state.update(cx, |state, cx| {
                state.apply_transcript(reply_entry(reply));
                cx.notify();
            });
        });
        wire.serve(&runtime, cx, |_, _| None);
        shell.read_with(cx, |shell, cx| {
            let panel = shell.pr_chat.as_ref().unwrap();
            assert_eq!(panel.context.suggestions(reply).len(), 1);
            assert!(shell.pr_chat_suggestions(cx).is_none());
        });
        assert!(cx.debug_bounds("pr-chat-card-0").is_none());
    }

    #[gpui::test]
    fn escape_closes_the_merge_confirm_and_keeps_the_panel_open(cx: &mut TestAppContext) {
        let (runtime, shell, cx, recorder, _dir) = open_cards(cx);
        let _guard = runtime.enter();
        inject_cards(&runtime, &shell, cx);
        click(cx, "pr-chat-card-post-2");
        assert!(cx.debug_bounds("pr-chat-confirm").is_some());
        cx.simulate_keystrokes("escape");
        Wire::pump(&runtime, cx);
        assert!(cx.debug_bounds("pr-chat-confirm").is_none());
        shell.read_with(cx, |shell, cx| {
            assert!(shell.pr_chat_sheet_open(cx), "only the confirm closed");
            assert!(shell.pr_chat.as_ref().unwrap().confirm.is_none());
            assert!(matches!(shell.route, Route::PullRequests));
        });
        assert!(recorder.writes().is_empty());
    }

    #[gpui::test]
    fn sessions_without_a_chat_row_draw_no_bubble_or_menu_row_and_stay_indexed(
        cx: &mut TestAppContext,
    ) {
        let (runtime, shell, cx, mut wire, _dir) = setup(cx);
        let _guard = runtime.enter();
        shell.update(cx, |shell, cx| {
            shell.state.update(cx, |state, _| {
                state.chats.push(row("kept", false, serde_json::json!({})))
            })
        });
        index(cx, "github.com/acme/zeron#7", &[("kept", 2), ("gone", 1)]);
        index(cx, "github.com/acme/zeron#8", &[("gone8", 1)]);
        shell.update(cx, |shell, cx| shell.sync_hidden_chats(cx));
        shell.update(cx, |shell, cx| shell.open_pull_requests(cx));
        wire.serve(&runtime, cx, |method, _| {
            (method == methods::LIST_CHANGE_REQUESTS).then(|| {
                serde_json::json!({
                    "viewers": {}, "items": [item_n(7), item_n(8)],
                    "errors": [], "truncated": false,
                })
            })
        });
        assert!(cx.debug_bounds("pr-chat-badge-7").is_some());
        assert!(cx.debug_bounds("pr-chat-badge-8").is_none(), "only a ghost");

        let page = shell.read_with(cx, |shell, _| shell.pull_requests_page.clone().unwrap());
        page.update(cx, |page, cx| page.open_linked(item(), None, cx));
        wire.serve(&runtime, cx, |method, _| {
            (method == methods::GET_CHANGE_REQUEST).then(detail)
        });
        shell.update(cx, |shell, cx| shell.reveal_pr_chat(cx));
        wire.serve(&runtime, cx, |_, _| None);
        shell.read_with(cx, |shell, cx| {
            assert_eq!(shell.pr_chat.as_ref().unwrap().chat_id, "kept");
            let rows = shell.pr_chat_session_rows(cx);
            let ids: Vec<&str> = rows.iter().map(|row| row.chat_id.as_str()).collect();
            assert_eq!(ids, ["kept"]);
            assert_eq!(
                crate::pull_requests::live_chat_sessions(
                    "github.com/acme/zeron#7",
                    &shell.state.read(cx).chats,
                    None,
                    cx
                )
                .len(),
                1
            );
            let settings = settings::current(cx);
            assert!(settings.pr_chat_indexed("gone") && settings.pr_chat_indexed("gone8"));
        });
    }

    fn send_mock(
        runtime: &tokio::runtime::Runtime,
        shell: &Entity<Shell>,
        cx: &mut VisualTestContext,
        wire: &mut Wire,
    ) -> String {
        let (chat_id, composer, side_state) = shell.read_with(cx, |shell, _| {
            let tab = shell.pr_chat_tab().expect("chat minted");
            (
                shell.pr_chat.as_ref().unwrap().chat_id.clone(),
                tab.composer.clone(),
                tab.state.clone(),
            )
        });
        let config: zeron_proto::ChatConfig = serde_json::from_value(serde_json::json!({
            "harness": "mock", "sandbox": "workspace-write",
        }))
        .unwrap();
        side_state.update(cx, |state, _| state.apply_chat_config(&chat_id, config));
        composer.update(cx, |composer, cx| composer.send_prompt("Hi".into(), cx));
        wire.serve(runtime, cx, |method, _| {
            matches!(method, methods::MUTATE | methods::QUEUE_COMMAND)
                .then(|| serde_json::json!({}))
        });
        wire.serve(runtime, cx, |_, _| Some(serde_json::json!({})));
        shell.read_with(cx, |_, cx| {
            assert!(settings::current(cx).pr_chat_indexed(&chat_id))
        });
        chat_id
    }

    fn serve_prepare(
        runtime: &tokio::runtime::Runtime,
        cx: &mut VisualTestContext,
        wire: &mut Wire,
    ) -> usize {
        let frames = wire.serve(runtime, cx, |method, _| {
            (method == methods::PREPARE_CHANGE_REQUEST_CHAT).then(|| {
                serde_json::json!({
                    "spaceId": "s1", "root": "/repo", "cwd": "/repo",
                    "worktree": false, "onHead": false,
                })
            })
        });
        calls(&frames, methods::PREPARE_CHANGE_REQUEST_CHAT).len()
    }

    fn deleted(frames: &[ClientFrame]) -> Vec<String> {
        calls(frames, methods::MUTATE)
            .into_iter()
            .filter(|frame| frame.params["op"] == "deleteChat")
            .map(|frame| frame.params["chatId"].as_str().unwrap().to_owned())
            .collect()
    }

    #[gpui::test]
    fn the_header_new_chat_button_starts_a_new_session_in_both_history_modes(
        cx: &mut TestAppContext,
    ) {
        let (runtime, shell, cx, mut wire, _dir) = setup(cx);
        let _guard = runtime.enter();
        let (unsent, _, _) = open_pr_chat(&runtime, &shell, cx, &mut wire);
        shell.update(cx, |shell, cx| shell.new_pr_chat_session(cx));
        assert_eq!(
            serve_prepare(&runtime, cx, &mut wire),
            0,
            "an unsent chat is already new"
        );
        shell.read_with(cx, |shell, _| {
            assert_eq!(shell.pr_chat.as_ref().unwrap().chat_id, unsent)
        });

        let kept = send_mock(&runtime, &shell, cx, &mut wire);
        click(cx, "pr-chat-new");
        let frames = wire.serve(&runtime, cx, |_, _| None);
        assert!(
            calls(&frames, methods::PREPARE_CHANGE_REQUEST_CHAT).is_empty(),
            "the PR's checkout is already known: {frames:?}"
        );
        assert!(deleted(&frames).is_empty(), "{frames:?}");
        shell.read_with(cx, |shell, cx| {
            let panel = shell.pr_chat.as_ref().unwrap();
            assert_ne!(panel.chat_id, kept);
            assert!(!panel.indexed);
            assert!(
                shell.pr_chat_sheet_open(cx),
                "the sheet stays for the new chat"
            );
            assert!(shell.pr_chat_tab().is_some());
            assert!(settings::current(cx).pr_chat_indexed(&kept));
        });
        assert!(cx.debug_bounds("pr-chat-sheet").is_some());

        cx.update(|_, cx| {
            settings::update(SavePolicy::Debounced, cx, |settings| {
                settings.pr_chat_history = PrChatHistory::Fresh
            })
        });
        shell.update(cx, |shell, cx| {
            shell.discard_pr_chat(cx);
            shell.reveal_pr_chat(cx);
        });
        assert_eq!(serve_prepare(&runtime, cx, &mut wire), 0);
        let fresh = send_mock(&runtime, &shell, cx, &mut wire);
        click(cx, "pr-chat-new");
        let mut frames = wire.serve(&runtime, cx, |_, _| Some(serde_json::json!({})));
        frames.extend(wire.serve(&runtime, cx, |_, _| Some(serde_json::json!({}))));
        assert_eq!(
            deleted(&frames),
            [fresh.clone()],
            "Fresh discards the replaced chat"
        );
        shell.read_with(cx, |shell, cx| {
            let panel = shell.pr_chat.as_ref().unwrap();
            assert!(panel.chat_id != fresh && panel.chat_id != kept);
            assert!(
                settings::current(cx).pr_chat_indexed(&kept),
                "made under Keep per PR"
            );
        });
    }

    fn set_last_message(
        shell: &Entity<Shell>,
        cx: &mut VisualTestContext,
        chat_id: &str,
        ago: chrono::Duration,
    ) {
        shell.update(cx, |shell, cx| {
            shell.state.update(cx, |state, _| {
                let chat = state
                    .chats
                    .iter_mut()
                    .find(|chat| chat.id == chat_id)
                    .unwrap();
                chat.last_message_at = Some(Utc::now() - ago);
            })
        })
    }

    fn expiring(cx: &mut VisualTestContext, expiry: PrChatIdleExpiry) {
        cx.update(|_, cx| {
            settings::update(SavePolicy::Debounced, cx, |settings| {
                settings.pr_chat_idle_expiry = expiry
            })
        });
    }

    #[gpui::test]
    fn an_idle_session_past_the_expiry_opens_a_new_chat_and_a_recent_one_resumes(
        cx: &mut TestAppContext,
    ) {
        let (runtime, shell, cx, mut wire, _dir) = setup(cx);
        let _guard = runtime.enter();
        shell.update(cx, |shell, cx| {
            shell.state.update(cx, |state, _| {
                state.chats.push(row("old", false, serde_json::json!({})))
            })
        });
        index(cx, "github.com/acme/zeron#7", &[("old", 3)]);
        expiring(cx, PrChatIdleExpiry::ThirtyMinutes);
        set_last_message(&shell, cx, "old", chrono::Duration::hours(2));
        shell.update(cx, |shell, cx| shell.open_pull_requests(cx));
        let page = shell.read_with(cx, |shell, _| shell.pull_requests_page.clone().unwrap());
        page.update(cx, |page, cx| page.open_linked(item(), None, cx));
        let frames = wire.serve(&runtime, cx, |method, _| answer_open(method));
        assert_eq!(
            calls(&frames, methods::PREPARE_CHANGE_REQUEST_CHAT).len(),
            1,
            "past 30 min: a new chat"
        );
        shell.read_with(cx, |shell, cx| {
            let panel = shell.pr_chat.as_ref().unwrap();
            assert_ne!(panel.chat_id, "old");
            assert!(panel.earlier.is_none());
            assert_eq!(
                panel.previous.as_ref().map(|p| p.row.id.as_str()),
                Some("old")
            );
            assert!(
                !shell.pr_chat_sheet_open(cx),
                "opening the PR shows the bar only"
            );
            assert!(settings::current(cx).pr_chat_indexed("old"));
            let rows = shell.pr_chat_session_rows(cx);
            assert!(rows.iter().any(|row| row.chat_id == "old" && !row.current));
        });

        set_last_message(&shell, cx, "old", chrono::Duration::minutes(10));
        shell.update(cx, |shell, cx| {
            shell.discard_pr_chat(cx);
            shell.toggle_pr_chat(cx);
        });
        assert_eq!(
            serve_prepare(&runtime, cx, &mut wire),
            0,
            "inside 30 min: resumed"
        );
        shell.read_with(cx, |shell, _| {
            let panel = shell.pr_chat.as_ref().unwrap();
            assert_eq!(panel.chat_id, "old");
            assert!(panel.indexed);
            assert!(panel.previous.is_none());
        });

        shell.update(cx, |shell, cx| shell.fold_pr_chat(cx));
        set_last_message(&shell, cx, "old", chrono::Duration::hours(2));
        shell.update(cx, |shell, cx| shell.toggle_pr_chat(cx));
        shell.read_with(cx, |shell, cx| {
            let panel = shell.pr_chat.as_ref().unwrap();
            assert_ne!(panel.chat_id, "old", "the kept panel expires too");
            assert!(panel.previous.is_some());
            assert!(
                shell.pr_chat_sheet_open(cx),
                "the chat button shows what expired"
            );
        });

        expiring(cx, PrChatIdleExpiry::Never);
        set_last_message(&shell, cx, "old", chrono::Duration::days(2));
        shell.update(cx, |shell, cx| {
            shell.discard_pr_chat(cx);
            shell.toggle_pr_chat(cx);
        });
        assert_eq!(
            serve_prepare(&runtime, cx, &mut wire),
            0,
            "Never always resumes"
        );
        shell.read_with(cx, |shell, _| {
            let panel = shell.pr_chat.as_ref().unwrap();
            assert_eq!(panel.chat_id, "old");
            assert!(panel.earlier.is_some(), "Never keeps the day divider");
        });
    }

    #[gpui::test]
    fn an_expired_reopen_shows_the_previous_chat_and_back_to_previous_resumes_it(
        cx: &mut TestAppContext,
    ) {
        let (runtime, shell, cx, mut wire, _dir) = setup(cx);
        let _guard = runtime.enter();
        shell.update(cx, |shell, cx| {
            shell.state.update(cx, |state, _| {
                state.chats.push(row(
                    "old",
                    false,
                    serde_json::json!({ "title": "Why the close button vanishes" }),
                ))
            })
        });
        index(cx, "github.com/acme/zeron#7", &[("old", 0)]);
        expiring(cx, PrChatIdleExpiry::ThirtyMinutes);
        set_last_message(&shell, cx, "old", chrono::Duration::hours(2));
        shell.update(cx, |shell, cx| shell.open_pull_requests(cx));
        let page = shell.read_with(cx, |shell, _| shell.pull_requests_page.clone().unwrap());
        page.update(cx, |page, cx| page.open_linked(item(), None, cx));
        wire.serve(&runtime, cx, |method, _| answer_open(method));
        shell.update(cx, |shell, cx| {
            shell.reveal_pr_chat(cx);
            let previous = &shell
                .pr_chat
                .as_ref()
                .unwrap()
                .previous
                .as_ref()
                .unwrap()
                .tab;
            previous.state.update(cx, |state, cx| {
                state.apply_transcript(reply_entry("Segoe Fluent."));
                cx.notify();
            });
        });
        cx.run_until_parked();
        assert!(
            cx.debug_bounds("pr-chat-sheet").is_some(),
            "state 3 is a sheet"
        );
        assert!(
            cx.debug_bounds("pr-chat-previous").is_some(),
            "the old chat, faded"
        );
        assert!(cx.debug_bounds("pr-chat-expired-divider").is_some());
        assert!(cx.debug_bounds("pr-chat-continue-0").is_some());
        assert!(cx.debug_bounds("pr-chat-continue-1").is_some());
        assert!(cx.debug_bounds("pr-chat-time-left").is_some());
        shell.read_with(cx, |shell, cx| {
            assert_eq!(shell.pr_chat_header_status(cx).as_deref(), Some("new chat"));
        });

        click(cx, "pr-chat-sessions");
        shell.read_with(cx, |shell, cx| {
            let rows = shell.pr_chat_session_rows(cx);
            let old = rows.iter().find(|row| row.chat_id == "old").unwrap();
            assert_eq!(old.label, "Why the close button vanishes");
            assert_eq!(old.state, Some(("expired".to_string(), false)));
            assert!(old.detail.ends_with("· 1 turn"), "{}", old.detail);
        });
        assert!(cx.debug_bounds("pr-chat-session-state-1").is_some());
        click(cx, "pr-chat-sessions");

        click(cx, "pr-chat-back-to-previous");
        let frames = wire.serve(&runtime, cx, |_, _| None);
        assert!(calls(&frames, methods::PREPARE_CHANGE_REQUEST_CHAT).is_empty());
        shell.read_with(cx, |shell, cx| {
            let panel = shell.pr_chat.as_ref().unwrap();
            assert_eq!(panel.chat_id, "old", "resumed the old session");
            assert!(panel.previous.is_none());
            assert!(shell.pr_chat_sheet_open(cx));
        });
        assert!(cx.debug_bounds("pr-chat-expired-divider").is_none());
    }

    #[gpui::test]
    fn the_sessions_menu_marks_the_live_session_with_its_time_left(cx: &mut TestAppContext) {
        let (runtime, shell, cx, mut wire, _dir) = setup(cx);
        let _guard = runtime.enter();
        shell.update(cx, |shell, cx| {
            shell.state.update(cx, |state, _| {
                state
                    .chats
                    .push(row("live", false, serde_json::json!({ "title": "Live" })));
                state
                    .chats
                    .push(row("stale", false, serde_json::json!({ "title": "Stale" })));
            })
        });
        index(cx, "github.com/acme/zeron#7", &[("stale", 2), ("live", 1)]);
        expiring(cx, PrChatIdleExpiry::ThirtyMinutes);
        set_last_message(&shell, cx, "live", chrono::Duration::minutes(12));
        set_last_message(&shell, cx, "stale", chrono::Duration::hours(5));
        shell.update(cx, |shell, cx| shell.open_pull_requests(cx));
        let page = shell.read_with(cx, |shell, _| shell.pull_requests_page.clone().unwrap());
        page.update(cx, |page, cx| page.open_linked(item(), None, cx));
        wire.serve(&runtime, cx, |method, _| answer_open(method));
        shell.read_with(cx, |shell, cx| {
            assert_eq!(shell.pr_chat.as_ref().unwrap().chat_id, "live");
            assert_eq!(shell.pr_chat_header_status(cx).as_deref(), Some("18m left"));
        });
        click(cx, "pr-chat-sessions");
        assert!(cx.debug_bounds("pr-chat-sessions-menu").is_some());
        shell.read_with(cx, |shell, cx| {
            let states: Vec<_> = shell
                .pr_chat_session_rows(cx)
                .into_iter()
                .map(|row| (row.chat_id, row.state))
                .collect();
            assert_eq!(
                states,
                [
                    ("live".to_string(), Some(("18m left".to_string(), true))),
                    ("stale".to_string(), Some(("expired".to_string(), false))),
                ]
            );
        });
        assert!(cx.debug_bounds("pr-chat-session-state-0").is_some());
        assert!(cx.debug_bounds("pr-chat-session-state-1").is_some());
        assert!(cx.debug_bounds("pr-chat-new-session").is_some());

        expiring(cx, PrChatIdleExpiry::Never);
        shell.update(cx, |_, cx| cx.notify());
        cx.run_until_parked();
        assert!(
            cx.debug_bounds("pr-chat-time-left").is_none(),
            "Never shows no clock"
        );
        assert!(cx.debug_bounds("pr-chat-session-state-0").is_none());
    }

    #[test]
    fn day_labels_count_calendar_days() {
        let now = chrono::Local::now();
        let at = |days: i64| (now - chrono::Duration::days(days)).with_timezone(&Utc);
        assert_eq!(day_label(at(0), now), "Today");
        assert_eq!(day_label(at(1), now), "Yesterday");
        assert_eq!(day_label(at(3), now), "3 days ago");
        assert_eq!(
            day_label(at(30), now),
            (now - chrono::Duration::days(30))
                .format("%b %-d")
                .to_string()
        );
        assert_eq!(session_label("Today", Some(" ")), "Today");
        assert_eq!(session_label("Yesterday", Some("Fix")), "Yesterday · Fix");
    }

    #[gpui::test]
    fn cmd_j_toggles_the_terminal_in_a_chat_and_the_mini_chat_on_pull_requests(
        cx: &mut TestAppContext,
    ) {
        let (runtime, shell, cx, mut wire, _dir) = setup(cx);
        let _guard = runtime.enter();
        let key = shell.update(cx, |shell, cx| shell.panel_key(cx));
        let combo = if cfg!(target_os = "macos") {
            "cmd-j"
        } else {
            "ctrl-j"
        };
        cx.simulate_keystrokes(combo);
        wire.serve(&runtime, cx, |_, _| None);
        shell.read_with(cx, |shell, _| {
            assert!(shell.panels.get(&key).terminal_open);
            assert!(shell.pr_chat.is_none());
        });
        cx.simulate_keystrokes(combo);
        shell.update(cx, |shell, cx| shell.open_pull_requests(cx));
        wire.serve(&runtime, cx, |_, _| None);
        cx.simulate_keystrokes(combo);
        wire.serve(&runtime, cx, |_, _| None);
        cx.update(|window, cx| {
            shell.update(cx, |shell, cx| {
                assert!(shell.pr_chat_owns_focus(window, cx), "focuses the bar");
                assert!(!shell.pr_chat_sheet_open(cx), "nothing to show yet");
                assert!(!shell.panels.get(&key).terminal_open);
            })
        });
        fill(&shell, cx, "Done.");
        shell.read_with(cx, |shell, cx| assert!(shell.pr_chat_sheet_open(cx)));
        cx.simulate_keystrokes(combo);
        shell.read_with(cx, |shell, cx| assert!(!shell.pr_chat_sheet_open(cx)));
        cx.simulate_keystrokes(combo);
        shell.read_with(cx, |shell, cx| assert!(shell.pr_chat_sheet_open(cx)));
    }
}
