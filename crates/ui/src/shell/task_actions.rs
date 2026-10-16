use super::*;
use zeron_proto::TaskPatch;

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
        title: &str,
        space_id: Option<String>,
        due_on: Option<String>,
        cx: &mut Context<Self>,
    ) -> String {
        let task_id = uuid::Uuid::new_v4().to_string();
        self.mutate_task_row(
            serde_json::json!({
                "op": "createTask", "taskId": task_id, "title": title,
                "spaceId": space_id, "dueOn": due_on,
            }),
            cx,
        );
        task_id
    }

    pub fn update_task(&mut self, task_id: &str, patch: TaskPatch, cx: &mut Context<Self>) {
        self.mutate_task_row(
            serde_json::json!({ "op": "updateTask", "taskId": task_id, "patch": patch }),
            cx,
        );
    }

    fn update_card_task(
        &mut self,
        page: &Entity<crate::tasks::TasksPage>,
        task_id: &str,
        patch: &TaskPatch,
        cx: &mut Context<Self>,
    ) {
        let Some(engine) = self.task_engine(cx) else {
            page.update(cx, |page, cx| page.save_failed(task_id, patch, cx));
            return;
        };
        let params = serde_json::json!({ "op": "updateTask", "taskId": task_id, "patch": patch });
        let (page, task_id, patch) = (page.clone(), task_id.to_string(), patch.clone());
        cx.spawn(async move |this, cx| {
            if let Err(err) = engine.client().call(methods::MUTATE, params).await {
                this.update(cx, |shell, cx| {
                    shell.show_notice(crate::toast::ToastKind::Error, format!("{err}"), cx);
                    page.update(cx, |page, cx| page.save_failed(&task_id, &patch, cx));
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
    fn start_task(
        &mut self,
        page: &Entity<crate::tasks::TasksPage>,
        task_id: &str,
        patch: Option<TaskPatch>,
        harness: Option<HarnessId>,
        model: Option<String>,
        cx: &mut Context<Self>,
    ) {
        let Some(engine) = self.task_engine(cx) else {
            page.update(cx, |page, cx| {
                if let Some(patch) = &patch {
                    page.save_failed(task_id, patch, cx);
                }
                page.start_failed(task_id, cx);
            });
            return;
        };
        let chat_id = uuid::Uuid::new_v4().to_string();
        let update = patch.clone().map(
            |patch| serde_json::json!({ "op": "updateTask", "taskId": task_id, "patch": patch }),
        );
        let start = serde_json::json!({
            "op": "startTask", "taskId": task_id, "chatId": chat_id,
            "harness": harness, "model": model,
        });
        let (page, task_id) = (page.clone(), task_id.to_string());
        cx.spawn(async move |this, cx| {
            let saving = update.is_some();
            for (step, params) in update.into_iter().chain([start]).enumerate() {
                if let Err(err) = engine.client().call(methods::MUTATE, params).await {
                    this.update(cx, |shell, cx| {
                        shell.show_notice(crate::toast::ToastKind::Error, format!("{err}"), cx);
                        page.update(cx, |page, cx| {
                            if let Some(patch) = patch.as_ref().filter(|_| saving && step == 0) {
                                page.save_failed(&task_id, patch, cx);
                            }
                            page.start_failed(&task_id, cx);
                        });
                        cx.notify();
                    })
                    .ok();
                    return;
                }
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
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        use crate::tasks::TasksEvent;
        match event {
            TasksEvent::Create {
                title,
                space_id,
                open_card,
            } => {
                let task_id = self.create_task(title, space_id.clone(), None, cx);
                if *open_card {
                    let title = title.clone();
                    page.update(cx, |page, cx| {
                        page.open_card(task_id, Some(title), true, Some(window), cx)
                    });
                }
            }
            TasksEvent::Update { task_id, patch } => {
                self.update_card_task(page, task_id, patch, cx)
            }
            TasksEvent::Delete(task_id) => self.delete_task(task_id, cx),
            TasksEvent::Start { task_id, patch } => {
                let (harness, model) = self
                    .composer
                    .read(cx)
                    .pickers()
                    .read(cx)
                    .canvas_harness_model();
                self.start_task(page, task_id, patch.clone(), harness, model, cx)
            }
            TasksEvent::OpenChat(chat_id) => self.open_chat(chat_id.clone(), cx),
            TasksEvent::Refused(message) => {
                self.show_notice(crate::toast::ToastKind::Error, message.clone(), cx);
                cx.notify();
            }
        }
    }

    pub(super) fn render_tasks_title_bar(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let theme = Theme::of(cx).clone();
        let view = settings::current(cx).tasks_view;
        let toggle = crate::settings::widgets::segmented(
            &theme,
            "tasks-view-toggle",
            &[
                (settings::TasksView::List, "List"),
                (settings::TasksView::Kanban, "Kanban"),
            ],
            view,
            |view, _, cx| {
                settings::update(settings::SavePolicy::Debounced, cx, |s| s.tasks_view = view);
            },
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
