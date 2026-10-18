use super::*;
use crate::tasks::{LiveThread, TaskCenter, TaskCenterEvent};
use zeron_proto::TaskPatch;

const TASKS_VIEW_SEGMENT: f32 = 72.0;
const TASKS_VIEW_THUMB: motion::MotionSpec = motion::MotionSpec::new(260, motion::EASE_DRAWER);

pub(super) struct TaskCenterPane {
    chat_id: String,
    transcript: Entity<Transcript>,
    composer: Entity<Composer>,
    _events: Subscription,
    _composer_events: Subscription,
}

impl TaskCenterPane {
    pub(super) fn composer_for(&self, transcript: &Entity<Transcript>) -> Option<Entity<Composer>> {
        (&self.transcript == transcript).then(|| self.composer.clone())
    }
}

impl Shell {
    fn task_engine(&mut self, cx: &mut Context<Self>) -> Option<crate::state::EngineHandle> {
        let engine = self.state.read(cx).engine().cloned();
        if engine.is_none() {
            self.show_notice(crate::toast::ToastKind::Error, "Engine not connected", cx);
            cx.notify();
        }
        engine
    }

    /// Detached, unlike `mutate`: a follow-up edit must not cancel the create.
    fn mutate_task_row(&mut self, params: serde_json::Value, cx: &mut Context<Self>) {
        if let Some(engine) = self.task_engine(cx) {
            Self::spawn_mutate(engine, params, cx).detach();
        }
    }

    pub fn create_task(
        &mut self,
        task_id: &str,
        title: &str,
        space_id: Option<String>,
        due_on: Option<String>,
        cx: &mut Context<Self>,
    ) {
        self.mutate_task_row(
            serde_json::json!({
                "op": "createTask", "taskId": task_id, "title": title,
                "spaceId": space_id, "dueOn": due_on,
            }),
            cx,
        );
    }

    pub fn update_task(&mut self, task_id: &str, patch: TaskPatch, cx: &mut Context<Self>) {
        self.mutate_task_row(
            serde_json::json!({ "op": "updateTask", "taskId": task_id, "patch": patch }),
            cx,
        );
    }

    fn update_card_task(
        &mut self,
        card: &Entity<crate::tasks::TaskCard>,
        task_id: &str,
        patch: &TaskPatch,
        cx: &mut Context<Self>,
    ) {
        let Some(engine) = self.task_engine(cx) else {
            card.update(cx, |card, cx| card.save_failed(task_id, patch, cx));
            return;
        };
        let params = serde_json::json!({ "op": "updateTask", "taskId": task_id, "patch": patch });
        let (card, task_id, patch) = (card.clone(), task_id.to_string(), patch.clone());
        cx.spawn(async move |this, cx| {
            if let Err(err) = engine.client().call(methods::MUTATE, params).await {
                this.update(cx, |shell, cx| {
                    shell.show_notice(crate::toast::ToastKind::Error, format!("{err}"), cx);
                    card.update(cx, |card, cx| card.save_failed(&task_id, &patch, cx));
                    cx.notify();
                })
                .ok();
            }
        })
        .detach();
    }

    pub fn delete_task(&mut self, task_id: &str, cx: &mut Context<Self>) {
        self.mutate_task_row(
            serde_json::json!({ "op": "deleteTask", "taskId": task_id }),
            cx,
        );
    }

    /// Saves the card's unsaved edits first, so the agent's first turn carries them.
    #[allow(clippy::too_many_arguments)]
    fn start_task(
        &mut self,
        card: &Entity<crate::tasks::TaskCard>,
        task_id: &str,
        patch: Option<TaskPatch>,
        space_id: Option<String>,
        harness: Option<HarnessId>,
        model: Option<String>,
        orchestrate: Option<crate::tasks::Orchestrate>,
        cx: &mut Context<Self>,
    ) {
        let Some(engine) = self.task_engine(cx) else {
            card.update(cx, |card, cx| {
                if let Some(patch) = &patch {
                    card.save_failed(task_id, patch, cx);
                }
                card.start_failed(task_id, cx);
            });
            return;
        };
        let chat_id = uuid::Uuid::new_v4().to_string();
        let update = patch.clone().map(
            |patch| serde_json::json!({ "op": "updateTask", "taskId": task_id, "patch": patch }),
        );
        let start = serde_json::json!({
            "op": "startTask", "taskId": task_id, "chatId": chat_id,
            "spaceId": space_id, "harness": harness, "model": model,
            "orchestrate": orchestrate.is_some(),
            "approvePlan": orchestrate.is_none_or(|o| o.approve_plan),
            "sameAgents": orchestrate.is_some_and(|o| o.same_agents),
        });
        let (card, task_id) = (card.clone(), task_id.to_string());
        cx.spawn(async move |this, cx| {
            let saving = update.is_some();
            for (step, params) in update.into_iter().chain([start]).enumerate() {
                if let Err(err) = engine.client().call(methods::MUTATE, params).await {
                    this.update(cx, |shell, cx| {
                        shell.show_notice(crate::toast::ToastKind::Error, format!("{err}"), cx);
                        card.update(cx, |card, cx| {
                            if let Some(patch) = patch.as_ref().filter(|_| saving && step == 0) {
                                card.save_failed(&task_id, patch, cx);
                            }
                            card.start_failed(&task_id, cx);
                        });
                        cx.notify();
                    })
                    .ok();
                    return;
                }
            }
            if orchestrate.is_some() {
                this.update(cx, |shell, cx| {
                    if card.read(cx).task_id() == Some(task_id.as_str()) {
                        card.update(cx, |card, cx| card.close(cx));
                    }
                    shell.open_task_center(task_id.clone(), cx);
                })
                .ok();
                return;
            }
            for _ in 0..100 {
                let opened = this
                    .update(cx, |shell, cx| {
                        let landed = shell.state.read(cx).chats.iter().any(|c| c.id == chat_id);
                        if landed {
                            shell.open_chat(chat_id.clone(), cx);
                        }
                        landed
                    })
                    .unwrap_or(true);
                if opened {
                    return;
                }
                cx.background_executor()
                    .timer(Duration::from_millis(100))
                    .await;
            }
        })
        .detach();
    }

    fn assign_task(
        &mut self,
        card: &Entity<crate::tasks::TaskCard>,
        task_id: &str,
        patch: Option<TaskPatch>,
        chat_id: &str,
        send: bool,
        cx: &mut Context<Self>,
    ) {
        let Some(engine) = self.task_engine(cx) else {
            if let Some(patch) = &patch {
                card.update(cx, |card, cx| card.save_failed(task_id, patch, cx));
            }
            return;
        };
        let update = patch.clone().map(
            |patch| serde_json::json!({ "op": "updateTask", "taskId": task_id, "patch": patch }),
        );
        let assign = serde_json::json!({
            "op": "assignTask", "taskId": task_id, "chatId": chat_id, "send": send,
        });
        let (card, task_id) = (card.clone(), task_id.to_string());
        cx.spawn(async move |this, cx| {
            let saving = update.is_some();
            for (step, params) in update.into_iter().chain([assign]).enumerate() {
                if let Err(err) = engine.client().call(methods::MUTATE, params).await {
                    this.update(cx, |shell, cx| {
                        shell.show_notice(crate::toast::ToastKind::Error, format!("{err}"), cx);
                        if let Some(patch) = patch.as_ref().filter(|_| saving && step == 0) {
                            card.update(cx, |card, cx| card.save_failed(&task_id, patch, cx));
                        }
                        cx.notify();
                    })
                    .ok();
                    return;
                }
            }
        })
        .detach();
    }

    pub(super) fn render_tasks_page(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let page = match &self.tasks_page {
            Some(page) => page.clone(),
            None => {
                let page = cx.new(|cx| crate::tasks::TasksPage::new(self.state.clone(), cx));
                self._tasks_events = Some(cx.subscribe_in(&page, window, Self::on_tasks_event));
                self.tasks_page = Some(page.clone());
                page
            }
        };
        div()
            .flex_1()
            .min_w_0()
            .h_full()
            .child(page)
            .into_any_element()
    }

    fn on_tasks_event(
        &mut self,
        page: &Entity<crate::tasks::TasksPage>,
        event: &crate::tasks::TasksEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        use crate::tasks::TasksEvent;
        let card = page.read(cx).card().clone();
        match event {
            TasksEvent::Create {
                task_id,
                title,
                space_id,
            } => self.create_task(task_id, title, space_id.clone(), None, cx),
            TasksEvent::Update { task_id, patch } => {
                self.update_card_task(&card, task_id, patch, cx)
            }
            TasksEvent::Card(event) => self.on_task_card_event(&card, event, cx),
            TasksEvent::OpenCenter(task_id) => self.open_task_center(task_id.clone(), cx),
        }
    }

    pub(super) fn on_task_card_event(
        &mut self,
        card: &Entity<crate::tasks::TaskCard>,
        event: &crate::tasks::TaskCardEvent,
        cx: &mut Context<Self>,
    ) {
        use crate::tasks::TaskCardEvent;
        match event {
            TaskCardEvent::Update { task_id, patch } => {
                self.update_card_task(card, task_id, patch, cx)
            }
            TaskCardEvent::Delete(task_id) => self.delete_task(task_id, cx),
            TaskCardEvent::Start {
                task_id,
                patch,
                space_id,
                harness,
                model,
                orchestrate,
            } => self.start_task(
                card,
                task_id,
                patch.clone(),
                space_id.clone(),
                *harness,
                model.clone(),
                *orchestrate,
                cx,
            ),
            TaskCardEvent::CreateSpace {
                space_id,
                device_id,
                path,
            } => self.mutate_task_row(
                serde_json::json!({
                    "op": "createSpace", "spaceId": space_id, "deviceId": device_id,
                    "path": path, "gitDetected": false,
                }),
                cx,
            ),
            TaskCardEvent::Assign {
                task_id,
                patch,
                chat_id,
                send,
            } => self.assign_task(card, task_id, patch.clone(), chat_id, *send, cx),
            TaskCardEvent::OpenChat(chat_id) => self.open_chat(chat_id.clone(), cx),
            TaskCardEvent::Refused(message) => {
                self.show_notice(crate::toast::ToastKind::Error, message.clone(), cx);
                cx.notify();
            }
        }
    }

    pub(crate) fn open_task_center(&mut self, task_id: String, cx: &mut Context<Self>) {
        if matches!(self.route, Route::TaskCenter)
            && self.task_center_task.as_deref() == Some(task_id.as_str())
        {
            return;
        }
        self.enter_task_center_route(task_id.clone(), cx);
        self.nav.push(NavEntry::TaskCenter(task_id));
    }

    pub(super) fn enter_task_center_route(&mut self, task_id: String, cx: &mut Context<Self>) {
        self.set_route(Route::TaskCenter, cx);
        self.task_center_task = Some(task_id);
        self.close_user_menu(cx);
        self.close_chat_menu(cx);
        cx.notify();
    }

    pub(super) fn task_center_holds(&self, chat_id: &str) -> bool {
        self.task_center_pane
            .as_ref()
            .is_some_and(|pane| pane.chat_id == chat_id)
    }

    pub(super) fn close_task_center(&mut self, cx: &mut Context<Self>) {
        self.close_task_center_pane(cx);
        self.task_center = None;
    }

    fn close_task_center_pane(&mut self, cx: &mut Context<Self>) {
        let Some(pane) = self.task_center_pane.take() else {
            return;
        };
        if let Some((center, _)) = &self.task_center {
            center.update(cx, |center, cx| center.set_live(None, cx));
        }
        let held = self.btw_tabs.contains_key(&pane.chat_id)
            || self
                .sidechat_tabs
                .values()
                .any(|tab| tab.chat_id == pane.chat_id)
            || self
                .subagent_tabs
                .values()
                .any(|tab| tab.doc_id == pane.chat_id)
            || self.split_chat_id() == Some(pane.chat_id.as_str());
        if !held {
            self.state
                .update(cx, |state, _| state.unwatch_subagent_doc(&pane.chat_id));
        }
    }

    fn open_task_center_pane(
        &mut self,
        center: &Entity<TaskCenter>,
        chat_id: String,
        cx: &mut Context<Self>,
    ) {
        let transcript =
            cx.new(|cx| Transcript::for_doc(self.state.clone(), chat_id.clone(), true, cx));
        let links = Self::session_links(Some(chat_id.clone()), cx);
        transcript.update(cx, |transcript, _| {
            transcript.set_workspace_link_handler(links)
        });
        let events = cx.subscribe(&transcript, Self::on_transcript_event);
        self.state
            .update(cx, |s, cx| s.watch_subagent_doc(chat_id.clone(), cx));
        let composer = cx.new(|cx| Composer::for_chat(self.state.clone(), chat_id.clone(), cx));
        let composer_events = cx.subscribe(&composer, {
            let transcript = transcript.clone();
            move |this: &mut Shell, _, event: &ComposerEvent, cx| {
                this.on_pane_composer_event(&transcript, event, cx)
            }
        });
        center.update(cx, |center, cx| {
            center.set_live(
                Some(LiveThread {
                    chat_id: chat_id.clone(),
                    transcript: transcript.clone(),
                    composer: composer.clone(),
                }),
                cx,
            )
        });
        self.task_center_pane = Some(TaskCenterPane {
            chat_id,
            transcript,
            composer,
            _events: events,
            _composer_events: composer_events,
        });
    }

    pub(super) fn render_task_center_page(
        &mut self,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Some(task_id) = self.task_center_task.clone() else {
            return div().flex_1().into_any_element();
        };
        let center = match &self.task_center {
            Some((center, _)) if center.read(cx).task_id() == task_id => center.clone(),
            _ => {
                self.close_task_center(cx);
                let center = cx.new(|cx| TaskCenter::new(self.state.clone(), task_id, cx));
                let events = cx.subscribe(&center, |shell, _, event: &TaskCenterEvent, cx| {
                    shell.on_task_center_event(event, cx)
                });
                self.task_center = Some((center.clone(), events));
                center
            }
        };
        let target = center.read(cx).live_target(cx);
        if target.as_deref() != center.read(cx).live_chat_id() {
            self.close_task_center_pane(cx);
            if let Some(chat_id) = target {
                self.open_task_center_pane(&center, chat_id, cx);
            }
        }
        div()
            .flex_1()
            .min_w_0()
            .h_full()
            .child(center)
            .into_any_element()
    }

    fn on_task_center_event(&mut self, event: &TaskCenterEvent, cx: &mut Context<Self>) {
        let params = match event {
            TaskCenterEvent::Update { task_id, patch } => {
                serde_json::json!({ "op": "updateTask", "taskId": task_id, "patch": patch })
            }
            TaskCenterEvent::ApprovePlan(task_id) => {
                serde_json::json!({ "op": "approvePlan", "taskId": task_id })
            }
            TaskCenterEvent::RetryStep(step_id) => {
                serde_json::json!({ "op": "retryTaskStep", "stepId": step_id })
            }
            TaskCenterEvent::UpdateStep { step_id, patch } => {
                serde_json::json!({ "op": "updateTaskStep", "stepId": step_id, "patch": patch })
            }
            TaskCenterEvent::DeleteStep(step_id) => {
                serde_json::json!({ "op": "deleteTaskStep", "stepId": step_id })
            }
            TaskCenterEvent::CreateStep {
                task_id,
                step_id,
                title,
            } => serde_json::json!({
                "op": "createTaskStep", "taskId": task_id, "stepId": step_id,
                "title": title, "notes": "", "orderKey": null,
            }),
            TaskCenterEvent::OpenChat(chat_id) => return self.open_chat(chat_id.clone(), cx),
            TaskCenterEvent::Back => return self.open_tasks(cx),
            TaskCenterEvent::Selected => return cx.notify(),
        };
        self.mutate_task_row(params, cx);
    }

    pub(super) fn render_task_center_title_bar(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let theme = Theme::of(cx).clone();
        let plus_inset = TITLEBAR_ACTION_SLOT_WIDTH * self.titlebar_plus_alpha(cx);
        let back = div()
            .id("task-center-back")
            .debug_selector(|| "task-center-back".into())
            .role(gpui::Role::Button)
            .aria_label("Back to Tasks")
            .flex()
            .items_center()
            .gap(px(6.0))
            .px(px(8.0))
            .py(px(4.0))
            .rounded(px(8.0))
            .cursor_pointer()
            .text_size(crate::typography::ui_rems(12.5))
            .text_color(theme.text_muted)
            .hover(|s| s.bg(theme.glass_hover()).text_color(theme.text))
            .child(
                icon(icons::LUCIDE_ARROW_LEFT)
                    .size(px(14.0))
                    .text_color(theme.text_muted),
            )
            .child("Tasks")
            .on_click(cx.listener(|shell, _: &gpui::ClickEvent, _, cx| shell.open_tasks(cx)));
        let inner = div()
            .size_full()
            .flex()
            .items_center()
            .pl(px((self.sidebar_edge() + Theme::SPACE_LG)
                .max(self.title_bar_content_start() + plus_inset)))
            .pr(px(self.titlebar_right_pad(TITLEBAR_ACTION_EDGE_INSET)))
            .child(back);
        let bar = div()
            .relative()
            .h(px(Theme::TITLEBAR_HEIGHT))
            .flex_none()
            .children(self.titlebar_seam_divider(cx))
            .child(inner);
        self.titlebar_drag_region("task-center-titlebar", bar, cx)
            .into_any_element()
    }

    pub(super) fn render_tasks_title_bar(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let theme = Theme::of(cx).clone();
        let view = settings::current(cx).tasks_view;
        let thumb = motion::state_t(
            "tasks-view-thumb",
            view == settings::TasksView::Kanban,
            TASKS_VIEW_THUMB,
            self.reduced_motion,
        );
        let text = theme.text;
        let toggle = div()
            .id("tasks-view-toggle")
            .relative()
            .flex_none()
            .flex()
            .flex_row()
            .rounded(px(9.0))
            .p(px(2.0))
            .bg(crate::theme::ink(0.04))
            .child(
                div()
                    .absolute()
                    .top(px(2.0))
                    .bottom(px(2.0))
                    .left(px(2.0 + TASKS_VIEW_SEGMENT * thumb))
                    .w(px(TASKS_VIEW_SEGMENT))
                    .rounded(px(7.0))
                    .bg(theme.bg)
                    .border_1()
                    .border_color(theme.border.opacity(0.8)),
            )
            .children(
                [
                    (settings::TasksView::List, "List"),
                    (settings::TasksView::Kanban, "Kanban"),
                ]
                .into_iter()
                .enumerate()
                .map(|(ix, (value, label))| {
                    let chosen = value == view;
                    div()
                        .id(("tasks-view-toggle", ix))
                        .w(px(TASKS_VIEW_SEGMENT))
                        .py(px(6.0))
                        .flex()
                        .items_center()
                        .justify_center()
                        .text_size(px(12.0))
                        .text_color(if chosen { theme.text } else { theme.text_muted })
                        .when(!chosen, |el| {
                            el.cursor_pointer()
                                .hover(move |s| s.text_color(text))
                                .on_click(move |_, _, cx| {
                                    settings::update(settings::SavePolicy::Debounced, cx, |s| {
                                        s.tasks_view = value
                                    });
                                })
                        })
                        .child(label)
                }),
            );
        let inner = div()
            .size_full()
            .flex()
            .items_center()
            .justify_end()
            .pr(px(self.titlebar_right_pad(TITLEBAR_ACTION_EDGE_INSET)))
            .child(toggle);
        let bar = div()
            .relative()
            .h(px(Theme::TITLEBAR_HEIGHT))
            .flex_none()
            .children(self.titlebar_seam_divider(cx))
            .child(inner);
        self.titlebar_drag_region("tasks-titlebar", bar, cx)
            .into_any_element()
    }
}
