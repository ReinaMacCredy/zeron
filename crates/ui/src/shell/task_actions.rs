use super::*;
use zeron_proto::TaskPatch;

const TASKS_VIEW_SEGMENT: f32 = 72.0;
const TASKS_VIEW_THUMB: motion::MotionSpec = motion::MotionSpec::new(260, motion::EASE_DRAWER);

struct TaskOrigin {
    card: Entity<crate::tasks::TaskCard>,
    patch: Option<TaskPatch>,
}

impl TaskOrigin {
    fn first_step(&self, task_id: &str) -> Option<serde_json::Value> {
        self.patch.as_ref().map(
            |patch| serde_json::json!({ "op": "updateTask", "taskId": task_id, "patch": patch }),
        )
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

    fn create_task_params(
        task_id: &str,
        title: &str,
        notes: &str,
        space_id: Option<String>,
        due_on: Option<String>,
    ) -> serde_json::Value {
        serde_json::json!({
            "op": "createTask", "taskId": task_id, "title": title, "notes": notes,
            "spaceId": space_id, "dueOn": due_on,
        })
    }

    pub fn create_task(
        &mut self,
        task_id: &str,
        title: &str,
        space_id: Option<String>,
        due_on: Option<String>,
        images: &[String],
        cx: &mut Context<Self>,
    ) {
        let mut params = Self::create_task_params(task_id, title, "", space_id, due_on);
        if !images.is_empty() {
            params["images"] = images.into();
        }
        self.mutate_task_row(params, cx);
    }

    pub(super) fn create_captured_task(
        &mut self,
        capture: &crate::quick_capture::Capture,
        cx: &mut Context<Self>,
    ) {
        let mut params = Self::create_task_params(
            &capture.task_id,
            &capture.title,
            &capture.notes,
            capture.space_id.clone(),
            capture.due_on.clone(),
        );
        let (open, _) = zeron_proto::mission::split_tasks(&self.state.read(cx).tasks);
        if let Some(key) = crate::tasks::top_order_key(&open, &capture.task_id) {
            params["orderKey"] = key.into();
        }
        if !capture.images.is_empty() {
            params["images"] = capture.images.clone().into();
        }
        if !capture.depends_on.is_empty() {
            params["dependsOn"] = capture.depends_on.clone().into();
        }
        self.mutate_task_row(params, cx);
    }

    pub(crate) fn approve_plan(
        &mut self,
        chat_id: &str,
        answer: crate::plan_proposal::PlanAnswer,
        cx: &mut Context<Self>,
    ) {
        if self.state.read(cx).plan_approval(&answer.request_id) {
            return;
        }
        let request_id = answer.request_id.clone();
        self.state.update(cx, |state, cx| {
            state.begin_plan_approval(&request_id);
            cx.notify();
        });
        let sent = self.composer.update(cx, |composer, cx| {
            composer.answer_plan(chat_id, answer, true, cx)
        });
        if !sent {
            self.end_plan_approval(&request_id, cx);
        }
    }

    pub(super) fn on_ask_answered(
        &mut self,
        chat_id: &str,
        request_id: &str,
        error: Option<&str>,
        cx: &mut Context<Self>,
    ) {
        let flight = self.state.read(cx).plan_approval(request_id);
        if let Some(err) = error {
            if flight {
                self.end_plan_approval(request_id, cx);
            }
            if self.composer.read(cx).bound_chat_id(cx).as_deref() != Some(chat_id) {
                let title = self
                    .state
                    .read(cx)
                    .chats
                    .iter()
                    .find(|chat| chat.id == chat_id)
                    .map(zeron_proto::Chat::display_title)
                    .unwrap_or_default();
                self.show_notice(
                    crate::toast::ToastKind::Error,
                    format!("Answer to {title} failed: {err}"),
                    cx,
                );
            }
            return;
        }
        if !flight {
            return;
        }
        if let Some(leave) = self.set_plan_mode(chat_id, false, cx) {
            cx.spawn(async move |_, _| {
                if let Err(err) = leave.await {
                    tracing::warn!(error = %err, "leaving plan mode failed");
                }
            })
            .detach();
        }
    }

    fn end_plan_approval(&mut self, request_id: &str, cx: &mut Context<Self>) {
        self.state.update(cx, |state, cx| {
            state.end_plan_approval(request_id);
            cx.notify();
        });
    }

    fn set_plan_mode(
        &mut self,
        chat_id: &str,
        on: bool,
        cx: &mut Context<Self>,
    ) -> Option<Task<Result<(), String>>> {
        let mut config = self
            .state
            .read(cx)
            .chats
            .iter()
            .find(|chat| chat.id == chat_id)
            .and_then(|chat| chat.config.clone())
            .filter(|config| config.plan != on)?;
        let engine = self.task_engine(cx)?;
        let before = config.clone();
        config.plan = on;
        let chat_id = chat_id.to_owned();
        self.state.update(cx, |state, cx| {
            state.apply_chat_config(&chat_id, config.clone());
            cx.notify();
        });
        Some(cx.spawn(async move |this, cx| {
            let params = serde_json::json!({
                "op": "setChatConfig", "chatId": chat_id, "config": config,
            });
            let result = engine.client().call(methods::MUTATE, params).await;
            let Err(err) = result else {
                return Ok(());
            };
            this.update(cx, |shell, cx| {
                shell.state.update(cx, |state, cx| {
                    state.apply_chat_config(&chat_id, before);
                    cx.notify();
                })
            })
            .ok();
            Err(err.to_string())
        }))
    }

    fn task_origin_failed(
        &mut self,
        origin: &TaskOrigin,
        task_id: &str,
        save_failed: bool,
        start_failed: bool,
        cx: &mut Context<Self>,
    ) {
        origin.card.update(cx, |card, cx| {
            if let Some(patch) = origin.patch.as_ref().filter(|_| save_failed) {
                card.save_failed(task_id, patch, cx);
            }
            if start_failed {
                card.start_failed(task_id, cx);
            }
        });
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

    /// Saves the card's edits, or creates the composer's task, first, so the agent's first turn carries them.
    fn start_task(
        &mut self,
        origin: TaskOrigin,
        task_id: &str,
        space_id: Option<Option<String>>,
        config: Option<ChatConfig>,
        cx: &mut Context<Self>,
    ) {
        let Some(engine) = self.task_engine(cx) else {
            self.task_origin_failed(&origin, task_id, true, true, cx);
            return;
        };
        let chat_id = uuid::Uuid::new_v4().to_string();
        let update = origin.first_step(task_id);
        let mut start = serde_json::json!({
            "op": "startTask", "taskId": task_id, "chatId": chat_id, "config": config,
        });
        if let Some(space_id) = space_id {
            start["spaceId"] = space_id.into();
        }
        let task_id = task_id.to_string();
        let route = self.route;
        cx.spawn(async move |this, cx| {
            let saving = update.is_some();
            for (step, params) in update.into_iter().chain([start]).enumerate() {
                if let Err(err) = engine.client().call(methods::MUTATE, params).await {
                    this.update(cx, |shell, cx| {
                        shell.show_notice(crate::toast::ToastKind::Error, format!("{err}"), cx);
                        shell.task_origin_failed(&origin, &task_id, saving && step == 0, true, cx);
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
                        if landed && shell.route == route {
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
        origin: TaskOrigin,
        task_id: &str,
        chat_id: &str,
        send: bool,
        cx: &mut Context<Self>,
    ) {
        let Some(engine) = self.task_engine(cx) else {
            self.task_origin_failed(&origin, task_id, true, false, cx);
            return;
        };
        let update = origin.first_step(task_id);
        let assign = serde_json::json!({
            "op": "assignTask", "taskId": task_id, "chatId": chat_id, "send": send,
        });
        let task_id = task_id.to_string();
        cx.spawn(async move |this, cx| {
            let saving = update.is_some();
            for (step, params) in update.into_iter().chain([assign]).enumerate() {
                if let Err(err) = engine.client().call(methods::MUTATE, params).await {
                    this.update(cx, |shell, cx| {
                        shell.show_notice(crate::toast::ToastKind::Error, format!("{err}"), cx);
                        shell.task_origin_failed(&origin, &task_id, saving && step == 0, false, cx);
                        cx.notify();
                    })
                    .ok();
                    return;
                }
            }
        })
        .detach();
    }

    fn tasks_page(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<crate::tasks::TasksPage> {
        match &self.tasks_page {
            Some(page) => page.clone(),
            None => {
                let page = cx.new(|cx| crate::tasks::TasksPage::new(self.state.clone(), cx));
                self._tasks_events = Some(cx.subscribe_in(&page, window, Self::on_tasks_event));
                self.tasks_page = Some(page.clone());
                page
            }
        }
    }

    /// The Tasks page with this task's card open, from a task link anywhere.
    pub(crate) fn open_task(
        &mut self,
        task_id: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_tasks(cx);
        let page = self.tasks_page(window, cx);
        page.update(cx, |page, cx| page.open_card(task_id, window, cx));
    }

    pub(super) fn open_captured_task(
        &mut self,
        task_id: String,
        title: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_tasks(cx);
        let card = self.tasks_page(window, cx).read(cx).card().clone();
        card.update(cx, |card, cx| {
            card.open(
                task_id,
                Some(title),
                crate::tasks::CardFocus::Notes,
                Some(window),
                cx,
            )
        });
    }

    pub(super) fn defer_open_task(&mut self, task_id: String, cx: &mut Context<Self>) {
        self.defer_in_active_window(cx, move |shell, window, cx| {
            shell.open_task(task_id, window, cx)
        });
    }

    pub(super) fn render_tasks_page(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let page = self.tasks_page(window, cx);
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
                images,
            } => self.create_task(task_id, title, space_id.clone(), None, images, cx),
            TasksEvent::Update { task_id, patch } => {
                self.update_card_task(&card, task_id, patch, cx)
            }
            TasksEvent::Card(event) => self.on_task_card_event(&card, event, cx),
            TasksEvent::OpenChat(chat_id) => self.open_chat(chat_id.clone(), cx),
            TasksEvent::Gather { task_ids, chat_id } => {
                let config = card.read(cx).chat_config(cx);
                self.gather_tasks(task_ids.clone(), chat_id.clone(), config, cx)
            }
        }
    }

    /// One thread for several tasks: a new one, or an existing one that gets them after its turn.
    fn gather_tasks(
        &mut self,
        task_ids: Vec<String>,
        chat_id: Option<String>,
        config: Option<ChatConfig>,
        cx: &mut Context<Self>,
    ) {
        let Some(engine) = self.task_engine(cx) else {
            return;
        };
        let (params, chat_id) = match chat_id {
            Some(chat_id) => (
                serde_json::json!({ "op": "addTasksToThread", "taskIds": task_ids, "chatId": chat_id }),
                chat_id,
            ),
            None => {
                let chat_id = uuid::Uuid::new_v4().to_string();
                (
                    serde_json::json!({
                        "op": "startTasks", "taskIds": task_ids, "chatId": chat_id, "config": config,
                    }),
                    chat_id,
                )
            }
        };
        let route = self.route;
        cx.spawn(async move |this, cx| {
            if let Err(err) = engine.client().call(methods::MUTATE, params).await {
                this.update(cx, |shell, cx| {
                    shell.show_notice(crate::toast::ToastKind::Error, format!("{err}"), cx);
                })
                .ok();
                return;
            }
            for _ in 0..100 {
                let opened = this
                    .update(cx, |shell, cx| {
                        let landed = shell.state.read(cx).chats.iter().any(|c| c.id == chat_id);
                        if landed && shell.route == route {
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
                config,
            } => self.start_task(
                TaskOrigin {
                    card: card.clone(),
                    patch: patch.clone(),
                },
                task_id,
                space_id.clone(),
                config.clone(),
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
            } => self.assign_task(
                TaskOrigin {
                    card: card.clone(),
                    patch: patch.clone(),
                },
                task_id,
                chat_id,
                *send,
                cx,
            ),
            TaskCardEvent::OpenChat(chat_id) => self.open_chat(chat_id.clone(), cx),
            TaskCardEvent::Refused(message) => {
                self.show_notice(crate::toast::ToastKind::Error, message.clone(), cx);
                cx.notify();
            }
        }
    }

    pub(super) fn tasks_view_switch(&self, theme: &Theme, cx: &App) -> gpui::Stateful<gpui::Div> {
        let views = [settings::TasksView::List, settings::TasksView::Kanban];
        let current = crate::tasks::tasks_view(cx);
        self.view_switch(
            "tasks-view",
            zeron_proto::mission::TASKS_VIEWS,
            views.iter().position(|view| *view == current).unwrap_or(0),
            TASKS_VIEW_SEGMENT,
            theme,
            move |ix, s| s.tasks_view = views[ix],
        )
    }

    fn view_switch<const N: usize>(
        &self,
        id: &'static str,
        labels: [&'static str; N],
        chosen: usize,
        segment: f32,
        theme: &Theme,
        pick: impl Fn(usize, &mut settings::UiSettings) + Copy + 'static,
    ) -> gpui::Stateful<gpui::Div> {
        let thumb: f32 = (1..N)
            .map(|k| {
                motion::state_t(
                    &format!("{id}-past-{k}"),
                    chosen >= k,
                    TASKS_VIEW_THUMB,
                    self.reduced_motion,
                )
            })
            .sum();
        let text = theme.text;
        div()
            .id(SharedString::from(format!("{id}-toggle")))
            .debug_selector(move || format!("{id}-toggle"))
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
                    .left(px(2.0 + segment * thumb))
                    .w(px(segment))
                    .rounded(px(7.0))
                    .bg(theme.bg)
                    .border_1()
                    .border_color(theme.border.opacity(0.8)),
            )
            .children(labels.into_iter().enumerate().map(|(ix, label)| {
                let on = ix == chosen;
                div()
                    .id((SharedString::from(format!("{id}-toggle")), ix))
                    .debug_selector(move || format!("{id}-{label}"))
                    .role(gpui::Role::Button)
                    .aria_label(label)
                    .aria_selected(on)
                    .w(px(segment))
                    .py(px(6.0))
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_size(px(12.0))
                    .text_color(if on { theme.text } else { theme.text_muted })
                    .when(!on, |el| {
                        el.cursor_pointer()
                            .hover(move |s| s.text_color(text))
                            .on_click(move |_, _, cx| {
                                settings::update(settings::SavePolicy::Debounced, cx, |s| {
                                    pick(ix, s)
                                });
                            })
                    })
                    .child(label)
            }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plan_proposal::PlanAnswer;
    use gpui::{AppContext, TestAppContext, VisualTestContext};
    use zeron_rpc::{ClientFrame, ServerFrame};

    struct Wire {
        runtime: tokio::runtime::Runtime,
        frames: tokio::sync::mpsc::Receiver<String>,
        replies: tokio::sync::mpsc::Sender<String>,
    }

    impl Wire {
        fn take(&mut self, cx: &mut VisualTestContext) -> Vec<ClientFrame> {
            for _ in 0..4 {
                self.runtime.block_on(async {
                    for _ in 0..8 {
                        tokio::task::yield_now().await;
                    }
                });
                cx.run_until_parked();
            }
            std::iter::from_fn(|| self.frames.try_recv().ok())
                .map(|frame| serde_json::from_str::<ClientFrame>(&frame).unwrap())
                .filter(|frame| {
                    matches!(
                        frame.method.as_deref(),
                        Some(methods::MUTATE | methods::QUEUE_COMMAND)
                    )
                })
                .collect()
        }

        fn reply(&self, frame: &ClientFrame, err: Option<&str>) {
            let reply = ServerFrame {
                id: frame.id,
                ok: err.is_none().then(|| serde_json::json!({})),
                err: err.map(str::to_owned),
                ..Default::default()
            };
            self.replies
                .try_send(serde_json::to_string(&reply).unwrap())
                .unwrap();
        }
    }

    fn chat(id: &str, device: &str) -> zeron_proto::Chat {
        serde_json::from_value(serde_json::json!({
            "id": id, "deviceId": device, "cwd": "/repo", "archived": false,
            "createdAt": Utc::now(),
            "config": { "harness": "claude-code", "sandbox": "workspace-write", "plan": true },
        }))
        .unwrap()
    }

    fn setup(
        cx: &mut TestAppContext,
    ) -> (
        Entity<Shell>,
        &mut VisualTestContext,
        Wire,
        tempfile::TempDir,
    ) {
        let (out, frames) = tokio::sync::mpsc::channel::<String>(256);
        let (replies, inbound) = tokio::sync::mpsc::channel::<String>(256);
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
            settings::init(UiSettings::default(), dir.path(), cx);
            crate::history::init(
                Default::default(),
                Default::default(),
                Default::default(),
                Default::default(),
                cx,
            );
        });
        let client = {
            let _guard = runtime.enter();
            zeron_rpc::RpcClient::new(out, inbound)
        };
        let state = cx.new(|_| AppState::new());
        state.update(cx, |state, _| {
            state.set_test_engine(crate::state::EngineHandle::from_test_client(client));
            state.local_device_id = Some("mac".into());
            state.chats = vec![chat("c", "mac"), chat("far", "pc")];
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
        let mut wire = Wire {
            runtime,
            frames,
            replies,
        };
        wire.take(cx);
        (shell, cx, wire, dir)
    }

    fn question(request_id: &str) -> PlanAnswer {
        PlanAnswer {
            request_id: request_id.into(),
            question_id: "plan".into(),
        }
    }

    fn approve(
        shell: &Entity<Shell>,
        cx: &mut VisualTestContext,
        chat_id: &str,
        answer: PlanAnswer,
    ) {
        shell.update(cx, |shell, cx| shell.approve_plan(chat_id, answer, cx));
    }

    fn plan_on(shell: &Entity<Shell>, cx: &mut VisualTestContext, chat_id: &str) -> bool {
        shell.read_with(cx, |shell, cx| {
            shell
                .state
                .read(cx)
                .chats
                .iter()
                .find(|chat| chat.id == chat_id)
                .and_then(|chat| chat.config.as_ref())
                .is_some_and(|config| config.plan)
        })
    }

    fn flight(shell: &Entity<Shell>, cx: &mut VisualTestContext, request_id: &str) -> bool {
        shell.read_with(cx, |shell, cx| {
            shell.state.read(cx).plan_approval(request_id)
        })
    }

    fn ops(frames: &[ClientFrame]) -> Vec<String> {
        frames
            .iter()
            .map(|frame| match frame.method.as_deref() {
                Some(methods::QUEUE_COMMAND) => format!(
                    "answer {}",
                    frame.params["command"]["requestId"].as_str().unwrap_or("?")
                ),
                _ => frame.params["op"].as_str().unwrap_or("?").to_owned(),
            })
            .collect()
    }

    #[gpui::test]
    fn approve_answers_the_clicked_plan(cx: &mut TestAppContext) {
        let (shell, cx, mut wire, _dir) = setup(cx);
        let _guard = wire.runtime.enter();

        approve(&shell, cx, "c", question("req-1"));
        let frames = wire.take(cx);
        assert_eq!(ops(&frames), ["answer req-1"]);
        let answers = &frames[0].params["command"]["answers"];
        assert_eq!(answers[0]["labels"][0], "Yes", "{answers}");
    }

    #[gpui::test]
    fn approve_leaves_plan_mode_only_once_answered(cx: &mut TestAppContext) {
        let (shell, cx, mut wire, _dir) = setup(cx);
        let _guard = wire.runtime.enter();

        approve(&shell, cx, "c", question("req-2"));
        let frames = wire.take(cx);
        assert_eq!(ops(&frames), ["answer req-2"]);
        assert!(flight(&shell, cx, "req-2"));
        wire.reply(&frames[0], Some("host gone"));
        assert!(
            wire.take(cx).is_empty(),
            "a failed answer changes nothing else"
        );
        assert!(!flight(&shell, cx, "req-2"));
        assert!(plan_on(&shell, cx, "c"));

        approve(&shell, cx, "c", question("req-2"));
        let frames = wire.take(cx);
        assert_eq!(ops(&frames), ["answer req-2"]);
        wire.reply(&frames[0], None);
        let frames = wire.take(cx);
        assert_eq!(ops(&frames), ["setChatConfig"]);
        assert_ne!(frames[0].params["config"]["plan"], true);
        assert!(!plan_on(&shell, cx, "c"));
    }
}
