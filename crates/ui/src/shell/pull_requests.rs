use super::*;
use crate::pull_requests::{PageMode, PageTab, PullRequestsEvent, PullRequestsPage};

impl Shell {
    fn pull_requests_page(&mut self, cx: &mut Context<Self>) -> Entity<PullRequestsPage> {
        if let Some(page) = &self.pull_requests_page {
            return page.clone();
        }
        let page = cx.new(|cx| PullRequestsPage::new(self.state.clone(), cx));
        self.pull_requests_sub = Some(cx.subscribe(&page, |shell, _, event, cx| match event {
            PullRequestsEvent::OpenInPage(tab) => shell.open_pull_requests_on(*tab, cx),
            PullRequestsEvent::OpenChat(handoff) => shell.open_pull_request_chat(handoff.clone(), cx),
            PullRequestsEvent::ToggleChat => shell.toggle_pr_chat(cx),
            PullRequestsEvent::OpenSettings(section) => {
                if let Some(page) = &shell.pull_requests_page {
                    page.update(cx, |page, cx| page.set_visible(false, cx));
                }
                shell.open_settings(*section, cx);
            }
        }));
        self.pull_requests_observe =
            Some(cx.observe(&page, |shell, _, cx| shell.sync_pr_chat_target(false, cx)));
        self.pull_requests_page = Some(page.clone());
        page
    }

    pub(super) fn open_pull_requests(&mut self, cx: &mut Context<Self>) {
        self.command_palette = None;
        let fresh = self.pull_requests_page.is_none();
        let page = self.pull_requests_page(cx);
        if !fresh {
            page.update(cx, |page, cx| page.refresh(cx));
        }
        self.route = Route::PullRequests;
        self.pull_requests_tab_open = true;
        page.update(cx, |page, cx| page.set_visible(true, cx));
        self.close_user_menu(cx);
        self.close_chat_menu(cx);
        self.sync_pr_chat_target(true, cx);
        cx.notify();
    }

    fn open_pull_requests_on(&mut self, tab: PageTab, cx: &mut Context<Self>) {
        self.open_pull_requests(cx);
        if let Some(page) = self.pull_requests_page.clone() {
            page.update(cx, |page, cx| page.show_page_tab(tab, cx));
        }
    }

    pub(super) fn add_pull_requests_surface(&mut self, tab: PageTab, cx: &mut Context<Self>) {
        let key = self.panel_key(cx);
        self.pr_surface_seq += 1;
        let id = self.pr_surface_seq;
        let page = cx
            .new(|cx| PullRequestsPage::with_mode(self.state.clone(), PageMode::Surface(tab), cx));
        let sub = cx.subscribe(&page, |shell, _, event, cx| match event {
            PullRequestsEvent::OpenInPage(tab) => shell.open_pull_requests_on(*tab, cx),
            PullRequestsEvent::OpenChat(handoff) => {
                shell.open_pull_request_chat(handoff.clone(), cx)
            }
            PullRequestsEvent::OpenSettings(section) => shell.open_settings(*section, cx),
            PullRequestsEvent::ToggleChat => {}
        });
        self.pr_surfaces.insert(id, (page, sub));
        let surface = match tab {
            PageTab::PullRequests => RightSurface::PullRequests(id),
            PageTab::Git => RightSurface::Git(id),
        };
        self.right_tabs.entry(key).or_default().push(surface);
        self.set_right_active(surface, cx);
    }

    pub(super) fn render_pull_requests_surface(
        &mut self,
        id: u64,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Some((page, _)) = self.pr_surfaces.get(&id) else {
            return self.render_surface_picker(cx);
        };
        let page = page.clone();
        let width = self.right_visible_width(cx);
        page.update(cx, |page, _| page.set_layout_width(width));
        page.into_any_element()
    }

    pub(super) fn sync_pull_request_surfaces(&mut self, cx: &mut Context<Self>) {
        let active = (!matches!(self.route, Route::Settings(_)) && self.right_pane_open(cx))
            .then(|| self.resolved_right_active(cx));
        let pages: Vec<_> = self
            .pr_surfaces
            .iter()
            .map(|(id, (page, _))| (*id, page.clone()))
            .collect();
        for (id, page) in pages {
            let shown = matches!(
                active,
                Some(RightSurface::PullRequests(open) | RightSurface::Git(open)) if open == id
            );
            if page.read(cx).visible() != shown {
                page.update(cx, |page, cx| page.set_visible(shown, cx));
            }
        }
    }

    pub(super) fn open_pull_request_link(
        &mut self,
        url: String,
        in_zeron: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some((host, repository, number)) = crate::pull_requests::parse_pull_request_url(&url)
        else {
            self.open_link_elsewhere(url, in_zeron, window, cx);
            return;
        };
        let Some(engine) = self.state.read(cx).engine().cloned() else {
            self.open_link_elsewhere(url, in_zeron, window, cx);
            return;
        };
        let targets = crate::pull_requests::link_targets(self.state.read(cx));
        let params = serde_json::json!({
            "host": host,
            "repository": repository,
            "number": number,
        });
        cx.spawn_in(window, async move |this, cx| {
            let mut found = None;
            for target in targets {
                let mut params = params.clone();
                if let Some(device) = &target {
                    params["targetDeviceId"] = serde_json::Value::String(device.clone());
                }
                if let Ok(detail) = engine
                    .client()
                    .call(methods::GET_CHANGE_REQUEST, params)
                    .await
                    .map_err(|_| ())
                    .and_then(|value| {
                        serde_json::from_value::<zeron_proto::ChangeRequestDetail>(value)
                            .map_err(|_| ())
                    })
                {
                    found = Some((detail.item, target));
                    break;
                }
            }
            this.update_in(cx, |shell, window, cx| match found {
                Some((item, device)) => {
                    shell.open_pull_requests(cx);
                    if let Some(page) = shell.pull_requests_page.clone() {
                        page.update(cx, |page, cx| page.open_linked(item, device, cx));
                    }
                }
                None => shell.open_link_elsewhere(url, in_zeron, window, cx),
            })
            .ok();
        })
        .detach();
    }

    fn open_link_elsewhere(
        &mut self,
        url: String,
        in_zeron: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if in_zeron {
            self.set_surfaces_open(true, cx);
            self.add_browser_surface(Some(url), window, cx);
        } else {
            cx.open_url(&url);
        }
    }

    pub(super) fn dismiss_pull_requests_overlay(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        self.pull_requests_page
            .clone()
            .is_some_and(|page| page.update(cx, |page, cx| page.dismiss_overlay(window, cx)))
    }

    /// A new chat in the pull request's project (or its prepared worktree), with the hand-off's
    /// text waiting in the composer rather than sent.
    fn open_pull_request_chat(
        &mut self,
        handoff: crate::pull_requests::ChatHandoff,
        cx: &mut Context<Self>,
    ) {
        let Some(engine) = self.state.read(cx).engine().cloned() else {
            return;
        };
        let chat_id = uuid::Uuid::new_v4().to_string();
        crate::draft_store::set(
            &chat_id,
            crate::draft_store::DraftEntry {
                prompt: handoff.prompt.clone(),
                ..Default::default()
            },
            cx,
        );
        let mut params = serde_json::json!({
            "op": "createChat",
            "chatId": chat_id,
            "spaceId": handoff.space_id,
        });
        if let Some(worktree) = &handoff.worktree {
            params["cwd"] = serde_json::Value::String(worktree.clone());
        }
        if let Some(branch) = &handoff.branch {
            params["branch"] = serde_json::Value::String(branch.clone());
        }
        cx.spawn(async move |this, cx| {
            let created = engine.client().call(methods::MUTATE, params).await;
            this.update(cx, |shell, cx| match created {
                Ok(_) => shell.open_chat(chat_id, cx),
                Err(err) => {
                    shell.sidebar_notice = Some(format!("{err}").into());
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
    }

    pub(super) fn close_pull_requests(&mut self, cx: &mut Context<Self>) {
        if let Some(page) = &self.pull_requests_page {
            page.update(cx, |page, cx| page.set_visible(false, cx));
        }
        self.route = Route::Chat;
        self.focus_composer(cx);
        cx.notify();
    }

    fn toggle_pull_requests(&mut self, cx: &mut Context<Self>) {
        if matches!(self.route, Route::PullRequests) {
            self.close_pull_requests(cx);
        } else {
            self.open_pull_requests(cx);
        }
    }

    pub(super) fn close_pull_requests_tab(&mut self, cx: &mut Context<Self>) {
        self.pull_requests_tab_open = false;
        if matches!(self.route, Route::PullRequests) {
            self.close_pull_requests(cx);
        } else {
            cx.notify();
        }
    }

    pub(super) fn sync_pull_requests_visibility(&mut self, cx: &mut Context<Self>) {
        let on_route = matches!(self.route, Route::PullRequests);
        if let Some(page) = self.pull_requests_page.clone()
            && page.read(cx).visible() != on_route
        {
            page.update(cx, |page, cx| page.set_visible(on_route, cx));
        }
    }

    pub(super) fn render_pull_requests_column(
        &mut self,
        width: f32,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let page = self.pull_requests_page(cx);
        page.update(cx, |page, _| page.set_layout_width(width));
        div()
            .debug_selector(|| "pull-requests-column".into())
            .h_full()
            .w(px(width))
            .flex_none()
            .relative()
            .pt(px(Theme::TITLEBAR_HEIGHT))
            .child(page)
            .children(self.render_pr_chat(width, window, cx))
            .into_any_element()
    }

    pub(super) fn render_pull_requests_strip_tab(
        &self,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        if !self.pull_requests_tab_open {
            return None;
        }
        let selected = matches!(self.route, Route::PullRequests);
        let git = self
            .pull_requests_page
            .as_ref()
            .is_some_and(|page| page.read(cx).git_tab_active());
        let (glyph, title) = if git {
            (icons::FORK, "Git")
        } else {
            (icons::PULL_REQUEST, "Pull Requests")
        };
        let (text_color, bg) = if selected {
            (theme.text, crate::theme::glass_selected_bg())
        } else {
            (theme.text_muted.opacity(0.6), crate::theme::wash(0.0))
        };
        Some(
            div()
                .id("pull-requests-strip-tab")
                .debug_selector(|| "pull-requests-strip-tab".into())
                .role(gpui::Role::Tab)
                .aria_label(title)
                .group("pull-requests-strip-tab")
                .h(px(28.0))
                .flex_none()
                .flex()
                .flex_row()
                .items_center()
                .gap(px(6.0))
                .pl(px(8.0))
                .pr(px(4.0))
                .rounded(px(8.0))
                .text_size(px(12.0))
                .text_color(text_color)
                .bg(bg)
                .when(selected, |el| {
                    el.shadow(crate::theme::glass_selected_shadows())
                })
                .when(!selected, |el| {
                    el.hover(|style| style.bg(theme.glass_hover()))
                })
                .cursor_pointer()
                .block_mouse_except_scroll()
                .on_mouse_down(MouseButton::Left, |_, window, _| window.prevent_default())
                .on_click(cx.listener(|this, _, _, cx| {
                    cx.stop_propagation();
                    this.open_pull_requests(cx);
                }))
                .child(
                    icon(glyph)
                        .size(px(14.0))
                        .flex_none()
                        .text_color(text_color),
                )
                .child(div().whitespace_nowrap().child(title))
                .child(
                    div()
                        .id("pull-requests-strip-tab-close")
                        .debug_selector(|| "pull-requests-strip-tab-close".into())
                        .role(gpui::Role::Button)
                        .aria_label("Close Pull Requests")
                        .size(px(16.0))
                        .rounded(px(4.0))
                        .flex()
                        .items_center()
                        .justify_center()
                        .opacity(if selected { 1.0 } else { 0.0 })
                        .group_hover("pull-requests-strip-tab", |style| style.opacity(1.0))
                        .hover(|style| style.bg(theme.ink(0.1)))
                        .on_click(cx.listener(|this, _, _, cx| {
                            cx.stop_propagation();
                            this.close_pull_requests_tab(cx);
                        }))
                        .child(
                            icon(icons::CLOSE)
                                .size(px(12.0))
                                .text_color(theme.text_muted),
                        ),
                )
                .into_any_element(),
        )
    }

    pub(super) fn render_pull_requests_trigger(
        &self,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let open = matches!(self.route, Route::PullRequests);
        div()
            .id("pull-requests-trigger")
            .role(gpui::Role::Button)
            .aria_label("Pull Requests")
            .aria_toggled(if open {
                gpui::Toggled::True
            } else {
                gpui::Toggled::False
            })
            .tooltip(|_, cx| {
                cx.new(|_| SurfaceTabTooltip {
                    text: "Pull Requests".into(),
                })
                .into()
            })
            .tab_index(0)
            .size(px(SIDEBAR_FOOTER_BUTTON_SIZE))
            .flex_none()
            .rounded(px(8.0))
            .flex()
            .items_center()
            .justify_center()
            .cursor_pointer()
            .bg(if open {
                theme.glass_hover()
            } else {
                motion::hover_blend(
                    "pull-requests-trigger",
                    theme.glass_hover().opacity(0.0),
                    theme.glass_hover(),
                )
            })
            .on_hover(motion::hover_listener("pull-requests-trigger"))
            .focus_visible(|s| s.border_2().border_color(theme.accent))
            .on_click(cx.listener(|this, _, _, cx| this.toggle_pull_requests(cx)))
            .child(
                icon(icons::PULL_REQUEST)
                    .size(px(15.0))
                    .text_color(if open {
                        theme.text
                    } else {
                        motion::hover_blend("pull-requests-trigger", theme.text_muted, theme.text)
                    }),
            )
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use gpui::{TestAppContext, VisualTestContext};
    use zeron_rpc::{RpcError, RpcReply, RpcService};

    use super::*;
    use crate::pull_requests::{PageTab, PullRequestListPrefs};

    #[derive(Default)]
    struct GitLab {
        calls: Mutex<Vec<String>>,
    }

    #[async_trait::async_trait]
    impl RpcService for GitLab {
        async fn handle(
            &self,
            method: &str,
            _params: serde_json::Value,
        ) -> Result<RpcReply, RpcError> {
            self.calls.lock().unwrap().push(method.to_string());
            match method {
                methods::GET_WORKING_TREE => RpcReply::value(&serde_json::json!({
                    "checkoutId": "checkout-1", "root": "/repo", "branch": "feature/x",
                    "headSha": "aaa", "upstream": "origin/feature/x", "ahead": 0, "behind": 0,
                    "pushRemote": "origin", "defaultBranch": "main", "revision": "r1",
                    "files": [{"path": "src/lib.rs", "index": "unchanged", "worktree": "modified",
                               "unstaged": {"additions": 1, "deletions": 0}}],
                })),
                methods::LIST_GIT_HISTORY => RpcReply::value(&serde_json::json!({
                    "commits": [], "headSha": "aaa", "nextCursor": null,
                    "totalCount": 0, "headCommitCount": 0,
                })),
                _ => Err(RpcError::UnknownMethod(method.to_string())),
            }
        }
    }

    fn settle(cx: &mut VisualTestContext, mut done: impl FnMut(&mut VisualTestContext) -> bool) {
        for _ in 0..200 {
            cx.run_until_parked();
            if done(cx) {
                return;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        panic!("condition never settled");
    }

    fn boot(
        cx: &mut TestAppContext,
        lab: Arc<GitLab>,
    ) -> (Entity<Shell>, &mut VisualTestContext, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        cx.update(|cx| {
            gpui_base::init(cx);
            cx.set_global(Theme::default());
            crate::app_menus::init(cx);
            crate::terminal::panel::init(cx);
            let mut ui = UiSettings::default();
            ui.pull_requests_list = PullRequestListPrefs {
                tab: PageTab::Git,
                git_space_id: Some("s1".into()),
                ..Default::default()
            };
            settings::init(ui, dir.path(), cx);
            crate::history::init(
                Default::default(),
                Default::default(),
                Default::default(),
                Default::default(),
                cx,
            );
        });
        cx.executor().allow_parking();
        let client = zeron_rpc::memory_client(lab);
        let state = cx.new(|_| AppState::new());
        state.update(cx, |state, _| {
            state.set_test_engine(crate::state::EngineHandle::from_test_client(client));
            state.workspace_scope = Some(WorkspaceScope::Local);
            state.connection = ConnectionStatus::Ready;
            state.local_device_id = Some("dev-local".into());
            state.devices = vec![
                serde_json::from_value(serde_json::json!({
                    "id": "dev-local", "name": "Lab Mac", "platform": "macos",
                    "capabilities": [
                        zeron_proto::capabilities::SOURCE_CONTROL_V1,
                        zeron_proto::capabilities::SOURCE_CONTROL_SYNC_V1,
                        zeron_proto::capabilities::SOURCE_CONTROL_COMMITS_V1,
                    ],
                }))
                .unwrap(),
            ];
            state.spaces = vec![
                serde_json::from_value(serde_json::json!({
                    "id": "s1", "deviceId": "dev-local", "path": "/repo", "gitDetected": true,
                    "createdAt": Utc::now(),
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
        (shell, cx, dir)
    }

    fn click(cx: &mut VisualTestContext, selector: &'static str) {
        let bounds = cx
            .debug_bounds(selector)
            .unwrap_or_else(|| panic!("{selector} is drawn"));
        cx.simulate_click(bounds.center(), gpui::Modifiers::default());
    }

    fn tree_reads(lab: &GitLab) -> usize {
        lab.calls
            .lock()
            .unwrap()
            .iter()
            .filter(|method| *method == methods::GET_WORKING_TREE)
            .count()
    }

    fn select_lab_chat(shell: &Entity<Shell>, cx: &mut VisualTestContext) {
        shell.update(cx, |shell, cx| {
            let mut chat = crate::shell::tests::chat_with_path(Some("/repo"), None);
            chat.id = "c1".into();
            chat.device_id = "dev-local".into();
            chat.space_id = Some("s1".into());
            shell.state.update(cx, |state, cx| {
                state.chats = vec![chat];
                state.select_chat(Some("c1".into()), cx);
            });
        });
        settle(cx, |cx| {
            shell.read_with(cx, |shell, _| shell.active_chat == "c1")
        });
    }

    #[gpui::test]
    fn the_page_keeps_the_sidebar_and_a_git_surface_stops_polling_once_closed(
        cx: &mut TestAppContext,
    ) {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let _guard = runtime.enter();
        let lab = Arc::new(GitLab::default());
        let (shell, cx, _dir) = boot(cx, lab.clone());
        select_lab_chat(&shell, cx);
        shell.update(cx, |shell, cx| shell.open_pull_requests(cx));
        settle(cx, |cx| {
            cx.debug_bounds("pull-requests-strip-tab").is_some()
        });
        assert!(cx.debug_bounds("pull-requests-back").is_none());
        assert!(
            cx.debug_bounds("pull-requests-project").is_some()
                || cx.debug_bounds("pull-requests-git-project").is_some()
        );

        shell.update(cx, |shell, cx| {
            shell.toggle_sidebar(cx);
            shell.toggle_right_pane(cx);
        });
        cx.run_until_parked();
        shell.read_with(cx, |shell, cx| {
            assert!(matches!(shell.route, Route::PullRequests));
            assert!(
                shell.settings.sidebar_collapsed,
                "the sidebar toggles on the page"
            );
            assert!(
                shell.right_pane_open(cx),
                "the right pane opens on the page"
            );
        });

        shell.update(cx, |shell, cx| {
            shell.add_pull_requests_surface(PageTab::Git, cx)
        });
        settle(cx, |cx| {
            cx.debug_bounds("pull-requests-open-page").is_some()
        });
        shell.read_with(cx, |shell, cx| {
            assert_eq!(shell.resolved_right_active(cx), RightSurface::Git(1));
        });
        settle(cx, |_| tree_reads(&lab) >= 2);
        settle(cx, |cx| {
            shell.read_with(cx, |shell, _| {
                !shell.tween_active(shell.sidebar_tween) && !shell.tween_active(shell.right_tween)
            })
        });

        click(cx, "pull-requests-strip-tab-close");
        cx.run_until_parked();
        shell.read_with(cx, |shell, _| assert!(matches!(shell.route, Route::Chat)));
        cx.update(|window, cx| {
            shell.update(cx, |shell, cx| {
                shell.close_right_surface(RightSurface::Git(1), window, cx)
            })
        });
        cx.run_until_parked();
        shell.read_with(cx, |shell, _| assert!(shell.pr_surfaces.is_empty()));
        let closed = tree_reads(&lab);
        for _ in 0..10 {
            cx.executor().advance_clock(Duration::from_secs(1));
            cx.run_until_parked();
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(
            tree_reads(&lab),
            closed,
            "nothing polls once both are closed"
        );
    }

    #[gpui::test]
    fn escape_on_the_git_tab_cancels_its_confirm_before_it_closes_the_page(
        cx: &mut TestAppContext,
    ) {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let _guard = runtime.enter();
        let lab = Arc::new(GitLab::default());
        let (shell, cx, _dir) = boot(cx, lab.clone());
        shell.update(cx, |shell, cx| shell.open_pull_requests(cx));
        settle(cx, |cx| cx.debug_bounds("git-changes-discard-0").is_some());

        click(cx, "git-changes-discard-0");
        cx.run_until_parked();
        assert!(cx.debug_bounds("git-discard-confirm").is_some());
        cx.simulate_keystrokes("escape");
        cx.run_until_parked();
        shell.read_with(cx, |shell, _| {
            assert!(
                matches!(shell.route, Route::PullRequests),
                "the first Escape closes the confirm, not the page"
            );
        });
        assert!(cx.debug_bounds("git-discard-confirm").is_none());

        cx.simulate_keystrokes("escape");
        cx.run_until_parked();
        shell.read_with(cx, |shell, _| assert!(matches!(shell.route, Route::Chat)));
        let closed = tree_reads(&lab);
        for _ in 0..10 {
            cx.executor().advance_clock(Duration::from_secs(1));
            cx.run_until_parked();
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(
            tree_reads(&lab),
            closed,
            "a closed page stops polling the working tree"
        );
    }
}
