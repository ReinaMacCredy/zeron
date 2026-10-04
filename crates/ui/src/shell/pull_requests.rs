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
        self.close_user_menu(cx);
        self.close_chat_menu(cx);
        cx.notify();
    }

    pub(super) fn dismiss_pull_requests_overlay(&mut self, cx: &mut Context<Self>) -> bool {
        self.pull_requests_page
            .clone()
            .is_some_and(|page| page.update(cx, |page, cx| page.dismiss_overlay(cx)))
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
