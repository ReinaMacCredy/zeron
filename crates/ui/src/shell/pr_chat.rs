use std::collections::{BTreeSet, HashSet};

use super::*;
use crate::pull_requests::{ContextChip, PrChatContext};
use crate::settings::{PrChatHistory, PrChatSession};

pub(super) const PR_CHAT_WIDTH: f32 = 400.0;
pub(super) const PR_CHAT_HEIGHT: f32 = 560.0;
const PR_CHAT_INSET: f32 = 20.0;

pub(super) struct PrChatPanel {
    pub(super) context: PrChatContext,
    removed: BTreeSet<ContextChip>,
    pub(super) chat_id: String,
    pub(super) side: Option<u64>,
    indexed: bool,
    error: Option<SharedString>,
    pub(super) open: bool,
    _location: Option<Task<()>>,
    _context: Option<Task<()>>,
    _saved: Option<Subscription>,
}

impl Shell {
    pub(super) fn toggle_pr_chat(&mut self, cx: &mut Context<Self>) {
        if self.pr_chat.as_ref().is_some_and(|panel| panel.open) {
            self.close_pr_chat(cx);
            return;
        }
        let Some(page) = self.pull_requests_page.clone() else {
            return;
        };
        let context = page.read(cx).chat_context(cx);
        let resumable = self.pr_chat.as_ref().is_some_and(|panel| {
            panel.context.index_key() == context.index_key()
                && panel
                    .side
                    .is_none_or(|id| self.side_chats.contains_key(&id))
        });
        if resumable {
            if let Some(panel) = self.pr_chat.as_mut() {
                panel.open = true;
            }
            self.focus_pr_chat(cx);
        } else {
            self.discard_pr_chat(cx);
            self.start_pr_chat(context, cx);
        }
        cx.notify();
    }

    pub(super) fn pr_chat_open(&self) -> bool {
        self.pr_chat.as_ref().is_some_and(|panel| panel.open)
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
        let location_engine = engine.clone();
        let location_task = cx.spawn(async move |this, cx| {
            let resolved = match location {
                Some(call) => location_engine
                    .client()
                    .call(call.method, call.params)
                    .await
                    .ok()
                    .and_then(|value| {
                        serde_json::from_value::<zeron_proto::ChangeRequestChat>(value).ok()
                    }),
                None => None,
            };
            this.update(cx, |shell, cx| shell.mint_pr_chat(resolved, cx))
                .ok();
        });
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
        self.pr_chat = Some(PrChatPanel {
            context,
            removed: BTreeSet::new(),
            chat_id,
            side: None,
            indexed: false,
            error: None,
            open: true,
            _location: Some(location_task),
            _context: Some(context_task),
            _saved: None,
        });
        self.sync_hidden_chats(cx);
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
        let tab = self.new_side_chat_tab(chat, true, cx);
        tab.composer
            .update(cx, |composer, cx| composer.set_pr_chat(cx));
        let saved = cx.observe(&tab.state, |this, _, cx| this.index_saved_pr_chat(cx));
        self.side_chat_seq += 1;
        let id = self.side_chat_seq;
        self.side_chats.insert(id, tab);
        if let Some(panel) = self.pr_chat.as_mut() {
            panel.side = Some(id);
            panel._saved = Some(saved);
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
        if let Some(panel) = self.pr_chat.as_mut() {
            panel.indexed = true;
        }
        settings::update(SavePolicy::Debounced, cx, |settings| {
            settings.index_pr_chat(
                &scope,
                PrChatSession {
                    chat_id,
                    created_at: Utc::now(),
                },
            )
        });
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

    pub(super) fn close_pr_chat(&mut self, cx: &mut Context<Self>) {
        let Some(panel) = self.pr_chat.as_mut() else {
            return;
        };
        panel.open = false;
        if settings::current(cx).pr_chat_history == PrChatHistory::Fresh {
            let chat_id = panel.chat_id.clone();
            if settings::current(cx).pr_chat_indexed(&chat_id) {
                self.delete_pr_chat(chat_id, cx);
            }
            self.discard_pr_chat(cx);
        }
        cx.notify();
    }

    fn delete_pr_chat(&mut self, chat_id: String, cx: &mut Context<Self>) {
        settings::update(SavePolicy::Debounced, cx, |settings| {
            settings.unindex_pr_chat(&chat_id)
        });
        let Some(engine) = self.state.read(cx).engine().cloned() else {
            return;
        };
        let params = serde_json::json!({ "op": "deleteChat", "chatId": chat_id });
        cx.spawn(async move |this, cx| {
            if let Err(error) = engine.client().call(methods::MUTATE, params).await {
                this.update(cx, |shell, cx| {
                    shell.sidebar_notice = Some(format!("{error}").into());
                    cx.notify();
                })
                .ok();
            }
        })
        .detach();
    }

    fn discard_pr_chat(&mut self, cx: &mut Context<Self>) {
        if let Some(panel) = self.pr_chat.take()
            && let Some(id) = panel.side
        {
            self.side_chats.remove(&id);
        }
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
        self.state
            .update(cx, |state, cx| state.set_hidden_chats(hidden, cx));
    }

    pub(super) fn pr_chat_owns_focus(&self, window: &Window, cx: &App) -> bool {
        self.pr_chat_open()
            && self.pr_chat_tab().is_some_and(|tab| {
                tab.composer
                    .read(cx)
                    .focus_handle(cx)
                    .contains_focused(window, cx)
            })
    }

    pub(super) fn render_pr_chat(&mut self, cx: &mut Context<Self>) -> Option<AnyElement> {
        if self.pr_chat.as_ref().is_some_and(|panel| {
            panel
                .side
                .is_some_and(|id| !self.side_chats.contains_key(&id))
        }) {
            self.pr_chat = None;
        }
        let panel = self.pr_chat.as_ref().filter(|panel| panel.open)?;
        let theme = Theme::of(cx).clone();
        let subtitle = panel.context.subtitle();
        let chips: Vec<(ContextChip, String)> = panel
            .context
            .chips()
            .into_iter()
            .filter(|(chip, _)| !panel.removed.contains(chip))
            .collect();
        let error = panel.error.clone();
        let tab = panel
            .side
            .and_then(|id| self.side_chats.get(&id))
            .map(|tab| (tab.transcript.clone(), tab.composer.clone()));
        let header = div()
            .flex_none()
            .h(px(44.0))
            .flex()
            .items_center()
            .gap(px(8.0))
            .pl(px(14.0))
            .pr(px(10.0))
            .border_b_1()
            .border_color(theme.border.opacity(0.6))
            .child(
                div()
                    .font_weight(gpui::FontWeight::MEDIUM)
                    .text_color(theme.text)
                    .child("Chat"),
            )
            .child(
                div()
                    .text_size(px(12.0))
                    .text_color(theme.text_muted)
                    .child(SharedString::from(subtitle)),
            )
            .child(div().flex_1())
            .child(
                div()
                    .id("pr-chat-close")
                    .debug_selector(|| "pr-chat-close".into())
                    .role(gpui::Role::Button)
                    .aria_label("Close chat")
                    .size(px(26.0))
                    .rounded(px(6.0))
                    .flex()
                    .items_center()
                    .justify_center()
                    .cursor_pointer()
                    .hover(|s| s.bg(theme.ink(0.06)))
                    .on_click(cx.listener(|this, _, _, cx| this.close_pr_chat(cx)))
                    .child(
                        icon(icons::LUCIDE_X)
                            .size(px(14.0))
                            .text_color(theme.text_muted),
                    ),
            );
        let chip_row = (!chips.is_empty()).then(|| {
            div()
                .flex_none()
                .flex()
                .flex_wrap()
                .gap(px(6.0))
                .px(px(12.0))
                .py(px(8.0))
                .border_b_1()
                .border_color(theme.border.opacity(0.6))
                .children(chips.into_iter().map(|(chip, label)| {
                    div()
                        .flex()
                        .items_center()
                        .gap(px(5.0))
                        .h(px(22.0))
                        .px(px(7.0))
                        .rounded(px(6.0))
                        .bg(theme.ink(0.05))
                        .text_size(px(11.0))
                        .text_color(theme.text.opacity(0.8))
                        .child(SharedString::from(label))
                        .child(
                            div()
                                .id(("pr-chat-chip-remove", chip as usize))
                                .debug_selector(move || format!("pr-chat-chip-{chip:?}"))
                                .role(gpui::Role::Button)
                                .aria_label("Remove context")
                                .cursor_pointer()
                                .text_color(theme.text_muted.opacity(0.6))
                                .hover(|s| s.text_color(theme.text))
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.remove_pr_chat_chip(chip, cx)
                                }))
                                .child("\u{00d7}"),
                        )
                }))
        });
        let body = match (tab, error) {
            (Some((transcript, composer)), _) => {
                composer.update(cx, |composer, cx| {
                    composer.set_dock_frame(crate::composer_dock::DockFrame::settled(true), cx);
                    composer.set_available_width(PR_CHAT_WIDTH - 20.0, cx);
                });
                div()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .flex_col()
                    .child(div().flex_1().min_h_0().child(transcript))
                    .child(div().flex_none().child(composer))
                    .into_any_element()
            }
            (None, error) => div()
                .flex_1()
                .flex()
                .items_center()
                .justify_center()
                .px(px(24.0))
                .text_size(px(12.0))
                .text_color(theme.text_muted)
                .child(error.unwrap_or_else(|| "Preparing the chat...".into()))
                .into_any_element(),
        };
        Some(
            div()
                .id("pr-chat-panel")
                .debug_selector(|| "pr-chat-panel".into())
                .occlude()
                .absolute()
                .right(px(PR_CHAT_INSET))
                .bottom(px(PR_CHAT_INSET))
                .w(px(PR_CHAT_WIDTH))
                .h(px(PR_CHAT_HEIGHT))
                .flex()
                .flex_col()
                .overflow_hidden()
                .rounded(px(14.0))
                .border_1()
                .border_color(theme.border)
                .bg(theme.surface_dialog)
                .shadow_lg()
                .child(header)
                .children(chip_row)
                .child(body)
                .into_any_element(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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
        serde_json::from_value(serde_json::json!({
            "provider": "github", "host": "github.com", "repository": "acme/zeron",
            "number": 7, "title": "Fix the thing",
            "url": "https://github.com/acme/zeron/pull/7",
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

    fn setup(
        cx: &mut TestAppContext,
    ) -> (
        tokio::runtime::Runtime,
        Entity<Shell>,
        &mut VisualTestContext,
        Wire,
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
            crate::app_menus::init(cx);
            crate::terminal::panel::init(cx);
            settings::init(UiSettings::default(), dir.path(), cx);
            crate::history::init(
                Default::default(),
                Default::default(),
                Default::default(),
                Default::default(),
                cx,
            );
        });
        let (out, frames) = tokio::sync::mpsc::channel::<String>(256);
        let (replies, inbound) = tokio::sync::mpsc::channel::<String>(256);
        let client = {
            let _guard = runtime.enter();
            zeron_rpc::RpcClient::new(out, inbound)
        };
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
        (runtime, shell, cx, Wire { frames, replies }, dir)
    }

    #[gpui::test]
    fn the_mini_chat_mints_an_ask_chat_with_the_context_and_stays_out_of_the_sidebar(
        cx: &mut TestAppContext,
    ) {
        let (runtime, shell, cx, mut wire, _dir) = setup(cx);
        let _guard = runtime.enter();
        let panel_key = shell.update(cx, |shell, cx| shell.panel_key(cx));

        shell.update(cx, |shell, cx| shell.open_pull_requests(cx));
        let page = shell.read_with(cx, |shell, _| shell.pull_requests_page.clone().unwrap());
        page.update(cx, |page, cx| page.open_linked(item(), None, cx));
        let frames = wire.serve(&runtime, cx, |method, _| {
            (method == methods::GET_CHANGE_REQUEST).then(detail)
        });
        assert_eq!(calls(&frames, methods::GET_CHANGE_REQUEST).len(), 1);

        shell.update(cx, |shell, cx| shell.toggle_pr_chat(cx));
        let frames = wire.serve(&runtime, cx, |method, _| match method {
            methods::PREPARE_CHANGE_REQUEST_CHAT => Some(serde_json::json!({
                "spaceId": "s1", "root": "/repo", "cwd": "/repo",
                "worktree": false, "onHead": false,
            })),
            methods::GET_CHANGE_REQUEST_DIFF => Some(serde_json::json!({
                "patch": "diff --git a/x b/x\n@@ -1 +1 @@\n-a\n+b\n", "truncated": false,
            })),
            _ => None,
        });
        let prepare = calls(&frames, methods::PREPARE_CHANGE_REQUEST_CHAT);
        assert_eq!(prepare.len(), 1);
        assert_eq!(prepare[0].params["checkout"], "none");
        assert_eq!(prepare[0].params["number"], 7);
        assert!(calls(&frames, methods::GET_CHANGE_REQUEST).is_empty());
        assert!(
            calls(&frames, methods::MUTATE).is_empty(),
            "nothing written yet"
        );

        let (side, side_state, chat_id) = shell.update(cx, |shell, cx| {
            let panel = shell.pr_chat.as_ref().expect("panel");
            assert!(panel.open);
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
        let panel = cx.debug_bounds("pr-chat-panel").expect("panel drawn");
        let window = cx.update(|window, _| window.viewport_size());
        assert_eq!(panel.size.width, px(PR_CHAT_WIDTH));
        assert_eq!(panel.size.height, px(PR_CHAT_HEIGHT));
        assert_eq!(panel.right(), window.width - px(PR_CHAT_INSET));
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
                "escape stays in the panel"
            );
            assert!(shell.pr_chat_open());
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
        assert_eq!(create["config"]["permissionMode"], "ask");
        let frames = wire.serve(&runtime, cx, |method, _| {
            (method == methods::QUEUE_COMMAND).then(|| serde_json::json!({}))
        });
        let run = calls(&frames, methods::QUEUE_COMMAND);
        assert_eq!(run.len(), 1, "{frames:?}");
        let request = &run[0].params["command"]["request"];
        assert_eq!(request["permissionMode"], "ask");
        assert_eq!(request["cwd"], "/wt/x");
        let prompt = request["prompt"].as_str().unwrap();
        assert!(
            prompt.starts_with(&format!(
                "Review this\n\n{}\n> Pull request #7",
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

            shell.close_pr_chat(cx);
            assert!(!shell.pr_chat_open());
            shell.toggle_pr_chat(cx);
            assert!(shell.pr_chat_open());
            assert_eq!(
                shell.pr_chat.as_ref().unwrap().chat_id,
                chat_id,
                "kept per PR"
            );
        });
        let frames = wire.serve(&runtime, cx, |_, _| None);
        assert!(
            calls(&frames, methods::MUTATE).is_empty(),
            "Keep per PR deletes nothing"
        );

        shell.update(cx, |shell, cx| {
            settings::update(SavePolicy::Debounced, cx, |settings| {
                settings.pr_chat_history = PrChatHistory::Fresh
            });
            shell.close_pr_chat(cx);
            assert!(shell.pr_chat.is_none());
            assert!(!settings::current(cx).pr_chat_indexed(&chat_id));
        });
        let frames = wire.serve(&runtime, cx, |_, _| Some(serde_json::json!({})));
        let deletes = calls(&frames, methods::MUTATE);
        assert_eq!(deletes.len(), 1, "{frames:?}");
        assert_eq!(deletes[0].params["op"], "deleteChat");
        assert_eq!(deletes[0].params["chatId"], chat_id.as_str());
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
        shell.read_with(cx, |shell, _| {
            assert!(shell.pr_chat_open());
            assert!(!shell.panels.get(&key).terminal_open);
        });
        cx.simulate_keystrokes(combo);
        shell.read_with(cx, |shell, _| assert!(!shell.pr_chat_open()));
    }
}
