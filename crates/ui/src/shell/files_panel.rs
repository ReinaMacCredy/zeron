//! Session-owned explorer chrome, independent of the surface tab host.

use super::*;

/// Files kept per chat for the command palette's Recent Files.
const RECENT_FILES_PER_CHAT: usize = 20;

impl Shell {
    /// Record `path` as the chat's most recently opened file.
    pub(super) fn note_recent_file(&mut self, chat_id: &str, path: &str) {
        if chat_id.is_empty() {
            return;
        }
        let recent = self.recent_files.entry(chat_id.to_owned()).or_default();
        recent.retain(|candidate| candidate != path);
        recent.push_front(path.to_owned());
        recent.truncate(RECENT_FILES_PER_CHAT);
    }

    /// The chat's opened files, most recent first.
    pub(super) fn recent_files(&self, chat_id: &str) -> impl Iterator<Item = &String> {
        self.recent_files.get(chat_id).into_iter().flatten()
    }
}

impl Shell {
    /// How far the surface host is open: its open/close tween's progress,
    /// 1 at rest while open. Width-only tweens between open sizes stay at 1.
    fn right_reveal(&self) -> f32 {
        // What the host actually occupies, up to its minimum: continuous
        // through reversals, which restart the tween from a partial width.
        match self
            .right_tween
            .filter(|tween| self.tween_active(Some(*tween)))
        {
            Some(tween) => {
                (self.eval_tween(Some(tween), tween.to) / RIGHT_PANE_MIN).clamp(0.0, 1.0)
            }
            None => 1.0,
        }
    }

    pub(super) fn surface_max_width(&self) -> f32 {
        let fit = self.horizontal_fit();
        // Takeover gives the conversation's space to the surface host.
        let chat_min = if fit.right && self.right_pane_expanded && self.right_reveal() > 0.0 {
            0.0
        } else {
            CHAT_PANEL_MIN
        };
        right_pane_max_width(self.content_width(), self.sidebar_now(), chat_min)
    }

    pub(super) fn right_visible_width(&self, cx: &App) -> f32 {
        let available = (self.content_width() - self.sidebar_now()).max(0.0);
        if !self.horizontal_fit().right {
            return self
                .fit_exit_width(1)
                .map_or(0.0, |width| width.min(available));
        }
        // Leaving takeover returns the conversation's width over the same
        // tween. Applying its settled minimum here jumps the seam by 300px
        // before that first frame (and breaks mid-flight reversals).
        let max = if self.tween_active(self.main_takeover_tween) {
            available
        } else {
            self.surface_max_width()
        };
        self.right_now(max, cx).min(available)
    }

    pub(super) fn accepts_file_navigation(
        &self,
        owner: &str,
        source: &Entity<FilesSurface>,
        cx: &App,
    ) -> bool {
        matches!(self.route, Route::Chat | Route::PullRequests)
            && self.panel_key(cx) == owner
            && source.read(cx).is_current_target(cx)
    }

    pub(super) fn prune_file_explorers(&mut self, cx: &mut Context<Self>) {
        let state = self.state.read(cx);
        if !state.chats_synced {
            return;
        }
        let live = state
            .chats
            .iter()
            .map(|chat| chat.id.as_str())
            .collect::<std::collections::HashSet<_>>();
        self.files.retain(|key, _| live.contains(key.as_str()));
        self.files_subs.retain(|key, _| live.contains(key.as_str()));
    }

    pub(super) fn sync_explorer_selection(&mut self, cx: &mut Context<Self>) {
        let RightSurface::File(id) = self.resolved_right_active(cx) else {
            return;
        };
        let Some(path) = self.file_surface_paths.get(&id).cloned() else {
            return;
        };
        if let Some(files) = self.files.get(&self.panel_key(cx)).cloned() {
            files.update(cx, |files, cx| files.reveal_file(path, cx));
        }
    }

    pub(super) fn add_files_surface(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.active_chat.is_empty() {
            return;
        }
        let key = self.panel_key(cx);
        if !self.files.contains_key(&key) {
            let files = cx.new(|cx| {
                FilesSurface::new_explorer(
                    self.state.clone(),
                    self.active_chat.clone(),
                    self.settings.files_show_all,
                    cx,
                )
            });
            let owner = key.clone();
            let sub = cx.subscribe_in(
                &files,
                window,
                move |this: &mut Self, source, event, window, cx| match event {
                    FilesEvent::HoldMutation { origin, paths } => {
                        let surfaces = this
                            .files
                            .values()
                            .chain(this.file_surfaces.values())
                            .filter(|s| s.read(cx).shares_workspace(origin))
                            .cloned()
                            .collect::<Vec<_>>();
                        for surface in surfaces {
                            surface.update(cx, |files, cx| files.hold_mutation(paths.clone(), cx));
                        }
                    }
                    FilesEvent::AddToChat {
                        path,
                        is_directory,
                        origin,
                    } => {
                        let payload = WorkspacePathDrag::new(path.clone(), *is_directory)
                            .with_origin(
                                Some(origin.clone()),
                                crate::files::WorkspacePathSource::Tree,
                                None,
                            );
                        let composer = this.composer.clone();
                        this.attach_workspace_drag(&payload, &composer, window, cx);
                    }
                    FilesEvent::Mutate(intent)
                        if this.accepts_file_navigation(&owner, &source, cx) =>
                    {
                        this.start_file_mutation(source.clone(), intent.clone(), cx);
                    }
                    FilesEvent::MutateMany(intents)
                        if this.accepts_file_navigation(&owner, &source, cx) =>
                    {
                        this.start_file_mutations(source.clone(), intents.clone(), cx);
                    }
                    FilesEvent::OpenFile(path)
                        if this.accepts_file_navigation(&owner, &source, cx) =>
                    {
                        this.add_file_surface(path.clone(), window, cx);
                    }
                    FilesEvent::OpenWebLink(activation) => {
                        if let crate::markdown::render::LinkOutcome::External(url) =
                            this.activate_session_link(activation, window, cx)
                        {
                            cx.open_url(&url);
                        }
                    }
                    FilesEvent::ShowAllFilesChanged(show_all) => {
                        this.set_files_show_all(*show_all, cx)
                    }
                    // Footer rows land in the surface host beside the explorer,
                    // through the same paths a spawn chip and a side-chat tab use.
                    FilesEvent::OpenSubagent {
                        doc_id,
                        title,
                        frozen,
                    } => this.open_child(
                        super::children::ChildRef::Subagent {
                            chat_id: this.active_chat.clone(),
                            doc_id: doc_id.clone(),
                            title: title.clone(),
                            frozen: *frozen,
                        },
                        cx,
                    ),
                    FilesEvent::OpenChildChat(chat_id) => {
                        this.open_child(super::children::ChildRef::Chat(chat_id.clone()), cx)
                    }
                    FilesEvent::RenameChildChat(chat_id) => {
                        this.open_rename_chat(chat_id.clone(), cx)
                    }
                    FilesEvent::ChildChatContextMenu { chat_id, position } => {
                        this.chat_menu.open(ChatMenuState {
                            chat_id: chat_id.clone(),
                            tab: None,
                            position: *position,
                            page: ChatMenuPage::Root,
                        });
                        cx.notify();
                    }
                    FilesEvent::NewChildChat => this.create_child_chat(None, cx),
                    FilesEvent::ForkChat => this.create_side_chat(cx),
                    _ => cx.notify(),
                },
            );
            self.files.insert(key.clone(), files);
            self.files_subs.insert(key.clone(), sub);
        }
        push_unique_right_surface(
            self.right_tabs.entry(key.clone()).or_default(),
            RightSurface::Files,
        );
        self.set_surfaces_open(true, cx);
        self.set_right_active(RightSurface::Files, cx);
        if let Some(files) = self.files.get(&key).cloned() {
            files.update(cx, |files, cx| {
                files.ensure_loaded(cx);
                files.focus_explorer(window, cx);
            });
        }
        self.composer
            .update(cx, |composer, _| composer.focus_pending = false);
        cx.notify();
    }

    pub(super) fn toggle_files_tool(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.right_pane_open(cx) && self.resolved_right_active(cx) == RightSurface::Files {
            self.toggle_right_pane(cx);
            window.focus(&self.composer.focus_handle(cx), cx);
        } else {
            self.add_files_surface(window, cx);
        }
    }

    pub(super) fn files_tool_shown(&self, cx: &App) -> bool {
        matches!(self.route, Route::Chat)
            && self.right_pane_open(cx)
            && self.resolved_right_active(cx) == RightSurface::Files
            && self.files.contains_key(&self.panel_key(cx))
    }

    /// The explorer of a Cloud chat whose machine is asleep (or waking).
    fn render_machine_asleep(
        &mut self,
        session: String,
        state: zeron_proto::CloudState,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        use zeron_proto::CloudState;
        let waking = matches!(state, CloudState::Provisioning | CloudState::Starting);
        div()
            .size_full()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap(px(10.0))
            .px(px(28.0))
            .child(
                icon(icons::CLOUD)
                    .size(px(20.0))
                    .text_color(theme.text_muted.opacity(0.7)),
            )
            .child(
                div()
                    .text_center()
                    .text_size(px(12.5))
                    .font_weight(gpui::FontWeight::MEDIUM)
                    .text_color(theme.text)
                    .child(if waking {
                        "Waking up the Cloud agent…"
                    } else {
                        "Cloud agent is asleep"
                    }),
            )
            .child(
                div()
                    .text_center()
                    .text_size(px(12.0))
                    .text_color(theme.text_muted)
                    .child(if waking {
                        "Its files show as soon as it's up."
                    } else {
                        "Wake it to browse its files. Sending a message wakes it too."
                    }),
            )
            .child(if waking {
                div()
                    .h(px(28.0))
                    .flex()
                    .items_center()
                    .child(crate::loaders::ring_spinner(
                        "files-cloud-waking",
                        14.0,
                        theme.text_muted,
                    ))
                    .into_any_element()
            } else {
                div()
                    .id("files-cloud-wake")
                    .h(px(28.0))
                    .px(px(12.0))
                    .rounded(px(7.0))
                    .border_1()
                    .border_color(theme.border)
                    .bg(crate::theme::wash(0.04))
                    .hover(|style| style.bg(crate::theme::wash(0.09)))
                    .cursor_pointer()
                    .flex()
                    .items_center()
                    .text_size(px(11.5))
                    .text_color(theme.text)
                    .child("Wake up")
                    .on_click(cx.listener(move |this, _, _, cx| {
                        let session = session.clone();
                        this.state
                            .update(cx, |state, cx| state.wake_cloud_session(session, cx));
                    }))
                    .into_any_element()
            })
            .into_any_element()
    }

    pub(super) fn render_files_tool(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let key = self.panel_key(cx);
        let visible = self.files_tool_shown(cx);
        let Some(files) = self.files.get(&key).cloned() else {
            return Empty.into_any_element();
        };
        let theme = Theme::of(cx).clone();
        // A Cloud chat whose machine is asleep: say so, with Wake, instead of
        // dialing a machine that won't answer and showing its errors.
        if let Some((session, state)) = self.state.read(cx).chat_machine_down(&self.active_chat) {
            self.files_machine_down.insert(self.active_chat.clone());
            files.update(cx, |files, _| {
                files.release_git_status();
                files.release_watch();
            });
            return self.render_machine_asleep(session, state, &theme, cx);
        }
        let woke = self.files_machine_down.remove(&self.active_chat);
        files.update(cx, |files, cx| {
            // Back up: list again what failed while it slept.
            if woke {
                files.retry_root(cx);
            }
            files.ensure_loaded(cx);
            if visible {
                files.ensure_git_status(cx);
            }
        });
        files.into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{AppContext, TestAppContext};

    #[gpui::test]
    fn the_files_tool_keeps_one_explorer_per_chat_beside_file_tabs(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        cx.update(|cx| {
            gpui_base::init(cx);
            cx.set_global(Theme::default());
            crate::app_menus::init(cx);
        });
        let window = cx.add_window(|_, cx| {
            let state = cx.new(|_| AppState::new());
            Shell::new(
                state,
                EngineBootConfig {
                    data_dir: dir.path().into(),
                    ipc_port: 0,
                    edge_url: "http://127.0.0.1:1".into(),
                    edge_token: None,
                    org_id: None,
                    workos_client_id: None,
                    default_harness: zeron_proto::HarnessId::Mock,
                },
                cx,
            )
        });
        window
            .update(cx, |shell, window, cx| {
                shell.add_files_surface(window, cx);
                assert!(
                    shell.files.is_empty(),
                    "the new-session canvas has no explorer"
                );
                shell.active_chat = "first".into();
                shell.add_files_surface(window, cx);
                let explorer = shell.files["first"].entity_id();
                assert!(shell.files_tool_shown(cx));
                assert_eq!(shell.right_surface_rows(cx).len(), 1);
                shell.add_files_surface(window, cx);
                assert_eq!(shell.files["first"].entity_id(), explorer);
                assert_eq!(shell.right_surface_rows(cx).len(), 1);
                shell.add_file_surface("src/main.rs".into(), window, cx);
                shell.add_file_surface("src/main.rs".into(), window, cx);
                assert_eq!(shell.file_surfaces.len(), 1);
                assert_eq!(shell.right_surface_rows(cx).len(), 2);
                assert!(!shell.files_tool_shown(cx), "the file tab is in front");
                shell.toggle_files_tool(window, cx);
                assert!(
                    shell.files_tool_shown(cx),
                    "the shortcut brings Files forward"
                );
                shell.toggle_files_tool(window, cx);
                assert!(
                    !shell.right_pane_open(cx),
                    "and a second press folds the pane"
                );
                assert_eq!(shell.file_surfaces.len(), 1);
                assert!(shell.pending_file_closes.is_empty());
                shell.active_chat = "second".into();
                shell.add_files_surface(window, cx);
                assert_ne!(shell.files["second"].entity_id(), explorer);
                shell.active_chat = "first".into();
                shell.add_files_surface(window, cx);
                assert_eq!(shell.files["first"].entity_id(), explorer);
                shell.route = Route::Settings(SettingsSection::Files);
                assert!(!shell.files_tool_shown(cx));
                shell.route = Route::Chat;
                assert!(shell.files_tool_shown(cx));
            })
            .unwrap();
    }

    #[gpui::test]
    fn the_pane_toggle_lands_on_a_tool_and_closing_every_tab_closes_it(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        cx.update(|cx| {
            gpui_base::init(cx);
            cx.set_global(Theme::default());
            crate::app_menus::init(cx);
        });
        let window = cx.add_window(|_, cx| {
            let state = cx.new(|_| AppState::new());
            Shell::new(
                state,
                EngineBootConfig {
                    data_dir: dir.path().into(),
                    ipc_port: 0,
                    edge_url: "http://127.0.0.1:1".into(),
                    edge_token: None,
                    org_id: None,
                    workos_client_id: None,
                    default_harness: zeron_proto::HarnessId::Mock,
                },
                cx,
            )
        });
        window
            .update(cx, |shell, window, cx| {
                window.activate_window();
                shell.active_chat = "chat".into();
                shell.reduced_motion = true;
                shell.toggle_right_pane(cx);
            })
            .unwrap();
        cx.run_until_parked();
        window
            .update(cx, |shell, window, cx| {
                assert!(shell.right_pane_open(cx));
                assert_eq!(
                    shell.resolved_right_active(cx),
                    RightSurface::Files,
                    "a chat without git lands on Files"
                );
                shell.add_file_surface("src/main.rs".into(), window, cx);
                let file = shell.resolved_right_active(cx);
                assert!(matches!(file, RightSurface::File(_)));
                shell.close_right_surface(file, window, cx);
                assert!(shell.file_surfaces.is_empty());
                assert!(
                    shell.right_pane_open(cx),
                    "the Files tab keeps the pane open"
                );
                assert_eq!(shell.resolved_right_active(cx), RightSurface::Files);
                shell.close_right_surface(RightSurface::Files, window, cx);
                assert!(!shell.right_pane_open(cx));
            })
            .unwrap();
    }

    #[gpui::test]
    fn pane_opens_on_new_chat_canvas_and_rides_through_the_send(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        cx.update(|cx| {
            gpui_base::init(cx);
            cx.set_global(Theme::default());
            crate::app_menus::init(cx);
        });
        let window = cx.add_window(|_, cx| {
            let state = cx.new(|_| AppState::new());
            Shell::new(
                state,
                EngineBootConfig {
                    data_dir: dir.path().into(),
                    ipc_port: 0,
                    edge_url: "http://127.0.0.1:1".into(),
                    edge_token: None,
                    org_id: None,
                    workos_client_id: None,
                    default_harness: zeron_proto::HarnessId::Mock,
                },
                cx,
            )
        });
        window
            .update(cx, |shell, window, cx| {
                shell
                    .state
                    .update(cx, |state, _| state.active_draft = Some("draft-1".into()));
                assert!(shell.active_chat.is_empty());
                shell.reduced_motion = true;

                shell.toggle_right_pane(cx);
                assert!(shell.right_pane_open(cx));
                assert!(shell.right_target(cx) > 0.0);
                shell.add_browser_surface(None, window, cx);
                assert_eq!(shell.right_surface_rows(cx).len(), 1);
                shell.toggle_right_pane(cx);
                assert!(!shell.right_pane_open(cx));
                shell.toggle_right_pane(cx);

                shell.active_chat = "draft-1".into();
                shell.prune_canvas_browsers(cx);
                assert!(shell.right_pane_open(cx));
                assert_eq!(shell.browsers.len(), 1);

                shell.active_chat.clear();
                shell
                    .state
                    .update(cx, |state, _| state.active_draft = Some("draft-2".into()));
                assert!(!shell.right_pane_open(cx));
            })
            .unwrap();
    }
}

#[cfg(all(test, target_os = "linux"))]
#[path = "files_panel_workspace_tests.rs"]
mod workspace_tests;
