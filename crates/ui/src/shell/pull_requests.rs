use super::*;
use crate::pull_requests::{PullRequestsEvent, PullRequestsPage};

impl Shell {
    fn pull_requests_page(&mut self, cx: &mut Context<Self>) -> Entity<PullRequestsPage> {
        if let Some(page) = &self.pull_requests_page {
            return page.clone();
        }
        let page = cx.new(|cx| PullRequestsPage::new(self.state.clone(), cx));
        self.pull_requests_sub = Some(cx.subscribe(&page, |shell, _, event, cx| match event {
            PullRequestsEvent::Close => shell.close_pull_requests(cx),
            PullRequestsEvent::OpenChat(handoff) => shell.open_pull_request_chat(handoff.clone(), cx),
            PullRequestsEvent::ToggleChat => shell.toggle_pr_chat(cx),
            PullRequestsEvent::OpenSettings(section) => {
                if let Some(page) = &shell.pull_requests_page {
                    page.update(cx, |page, _| page.set_visible(false));
                }
                shell.open_settings(*section, cx);
            }
        }));
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
        page.update(cx, |page, _| page.set_visible(true));
        self.close_user_menu(cx);
        self.close_chat_menu(cx);
        cx.notify();
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
            page.update(cx, |page, _| page.set_visible(false));
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

    pub(super) fn render_pull_requests_page(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> gpui::Div {
        let page = self.pull_requests_page(cx);
        let overlays = self.render_overlays(window.viewport_size(), window, cx);
        let drag = self.titlebar_drag_region(
            "pull-requests-titlebar",
            div()
                .absolute()
                .top_0()
                .left_0()
                .right_0()
                .h(px(Theme::TITLEBAR_HEIGHT)),
            cx,
        );
        div()
            .size_full()
            .relative()
            .child(drag)
            .child(page)
            .children(self.render_pr_chat(cx))
            .children(overlays)
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
                icon(icons::LUCIDE_GIT_PULL_REQUEST)
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
    }
}
