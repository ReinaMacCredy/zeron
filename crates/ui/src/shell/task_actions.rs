use super::*;
use crate::tasks::{LiveThread, TaskCenter, TaskCenterEvent};
use zeron_proto::TaskPatch;

const TASKS_VIEW_SEGMENT: f32 = 72.0;
const TREE_VIEW_SEGMENT: f32 = 76.0;
const TASKS_VIEW_THUMB: motion::MotionSpec = motion::MotionSpec::new(260, motion::EASE_DRAWER);
const CENTER_SLIDE_IN: motion::MotionSpec = motion::MotionSpec::new(380, motion::EASE_DRAWER);
const CENTER_SLIDE_OUT: motion::MotionSpec = motion::MotionSpec::new(300, motion::EASE_DRAWER);
pub(super) const CENTER_FADE: motion::MotionSpec = motion::MotionSpec::new(150, motion::EASE);
const UNDER_SHIFT: f32 = 0.24;
const UNDER_DIM: f32 = 0.45;
const CENTER_SHADOW: f32 = 0.55;
const CHAT_SLIDE_IN: motion::MotionSpec = motion::MotionSpec::new(300, motion::EASE_OUT_QUINT);
const CHAT_SLIDE_OUT: motion::MotionSpec = motion::MotionSpec::new(300, motion::EASE_TAILWIND);
const CHAT_PARALLAX: f32 = 30.0;

pub(super) struct TaskSlide {
    tween: motion::Tween,
    under: Route,
    chat: bool,
    pub(super) strip: (Option<String>, motion::Tween),
    pub(super) folded: std::collections::HashMap<String, motion::Tween>,
}

impl TaskSlide {
    pub(super) fn resting(route: Route) -> Self {
        let now = motion::now();
        Self {
            tween: motion::Tween::at(Self::rest(route), now),
            under: Route::Tasks,
            chat: false,
            strip: (None, motion::Tween::at(0.0, now)),
            folded: Default::default(),
        }
    }

    fn rest(route: Route) -> f32 {
        if route == Route::TaskCenter { 1.0 } else { 0.0 }
    }
}

fn slides_under_center(route: Route) -> bool {
    matches!(route, Route::Tasks | Route::Inbox)
}

pub(super) struct DocPane {
    pub(super) chat_id: String,
    pub(super) transcript: Entity<Transcript>,
    pub(super) composer: Entity<Composer>,
    _events: Subscription,
    _composer_events: Subscription,
}

impl DocPane {
    pub(super) fn composer_for(&self, transcript: &Entity<Transcript>) -> Option<Entity<Composer>> {
        (&self.transcript == transcript).then(|| self.composer.clone())
    }
}

enum TaskOrigin {
    Card {
        card: Entity<crate::tasks::TaskCard>,
        patch: Option<TaskPatch>,
    },
    Plan {
        create: Option<serde_json::Value>,
        approval: Option<PlanApproval>,
    },
}

pub(super) struct PlanApproval {
    chat_id: String,
    request_id: String,
    restore_plan: bool,
}

impl TaskOrigin {
    fn first_step(&self, task_id: &str) -> Option<serde_json::Value> {
        match self {
            Self::Card { patch, .. } => patch.as_ref().map(
                |patch| serde_json::json!({ "op": "updateTask", "taskId": task_id, "patch": patch }),
            ),
            Self::Plan { create, .. } => create.clone(),
        }
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
        self.mutate_task_row(params, cx);
    }

    pub(super) fn add_plan(
        &mut self,
        chat_id: &str,
        lead: crate::tasks::Orchestrate,
        approval: Option<PlanApproval>,
        cx: &mut Context<Self>,
    ) {
        let state = self.state.read(cx);
        if let Some(reason) = crate::tasks::chat_plan_blocker(state, chat_id) {
            if let Some(approval) = approval {
                self.show_notice(crate::toast::ToastKind::Error, reason, cx);
                self.plan_task_failed(&approval, cx);
            }
            return;
        }
        let Some(chat) = state.chats.iter().find(|chat| chat.id == chat_id) else {
            return;
        };
        let (task_id, create) = match crate::tasks::task_led_by(&state.tasks, chat_id)
            .filter(|task| task.completed_at.is_none())
        {
            Some(task) => (task.id.clone(), None),
            None => {
                let task_id = uuid::Uuid::new_v4().to_string();
                let title = crate::home::chat_title(chat);
                let create =
                    Self::create_task_params(&task_id, &title, "", chat.space_id.clone(), None);
                (task_id, Some(create))
            }
        };
        self.assign_task(
            TaskOrigin::Plan { create, approval },
            &task_id,
            chat_id,
            true,
            Some(lead),
            cx,
        );
    }

    pub(crate) fn approve_plan(
        &mut self,
        chat_id: &str,
        target: crate::plan_proposal::PlanTarget,
        as_task: bool,
        cx: &mut Context<Self>,
    ) {
        let answer = match target {
            crate::plan_proposal::PlanTarget::Task(task_id) => {
                self.mutate(
                    serde_json::json!({ "op": "approvePlan", "taskId": task_id }),
                    cx,
                );
                return;
            }
            crate::plan_proposal::PlanTarget::Question(answer) => answer,
        };
        let state = self.state.read(cx);
        if state.plan_approval(&answer.request_id).is_some() {
            return;
        }
        if let Some(reason) = as_task
            .then(|| crate::tasks::chat_plan_blocker(state, chat_id))
            .flatten()
        {
            self.show_notice(crate::toast::ToastKind::Error, reason, cx);
            return;
        }
        let request_id = answer.request_id.clone();
        self.state.update(cx, |state, cx| {
            state.begin_plan_approval(&request_id, chat_id, as_task);
            cx.notify();
        });
        let sent = self.composer.update(cx, |composer, cx| {
            composer.answer_plan(chat_id, answer, !as_task, cx)
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
            if flight.is_some() {
                self.end_plan_approval(request_id, cx);
            }
            if self.composer.read(cx).bound_chat_id(cx).as_deref() != Some(chat_id) {
                let title = self
                    .state
                    .read(cx)
                    .chats
                    .iter()
                    .find(|chat| chat.id == chat_id)
                    .map(crate::home::chat_title)
                    .unwrap_or_default();
                self.show_notice(
                    crate::toast::ToastKind::Error,
                    format!("Answer to {title} failed: {err}"),
                    cx,
                );
            }
            return;
        }
        let Some(as_task) = flight else {
            return;
        };
        let leave = self.set_plan_mode(chat_id, false, cx);
        let restore_plan = leave.is_some();
        let (chat_id, request_id) = (chat_id.to_owned(), request_id.to_owned());
        cx.spawn(async move |this, cx| {
            let left = match leave {
                Some(leave) => leave.await,
                None => Ok(()),
            };
            this.update(cx, |shell, cx| match left {
                Err(err) if as_task => {
                    shell.show_notice(
                        crate::toast::ToastKind::Error,
                        format!("Could not leave plan mode: {err}"),
                        cx,
                    );
                    shell.plan_task_failed(
                        &PlanApproval {
                            chat_id,
                            request_id,
                            restore_plan,
                        },
                        cx,
                    );
                }
                Err(err) => tracing::warn!(error = %err, "leaving plan mode failed"),
                Ok(()) if as_task => {
                    let lead = crate::tasks::Orchestrate {
                        approve_plan: false,
                        ..Default::default()
                    };
                    let approval = PlanApproval {
                        chat_id: chat_id.clone(),
                        request_id,
                        restore_plan,
                    };
                    shell.add_plan(&chat_id, lead, Some(approval), cx);
                }
                Ok(()) => {}
            })
            .ok();
        })
        .detach();
    }

    fn end_plan_approval(&mut self, request_id: &str, cx: &mut Context<Self>) {
        self.state.update(cx, |state, cx| {
            state.end_plan_approval(request_id);
            cx.notify();
        });
    }

    fn plan_task_failed(&mut self, approval: &PlanApproval, cx: &mut Context<Self>) {
        self.end_plan_approval(&approval.request_id, cx);
        if approval.restore_plan
            && let Some(restore) = self.set_plan_mode(&approval.chat_id, true, cx)
        {
            cx.spawn(async move |_, _| {
                if let Err(err) = restore.await {
                    tracing::warn!(error = %err, "restoring plan mode failed");
                }
            })
            .detach();
        }
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
        match origin {
            TaskOrigin::Card { card, patch } => card.update(cx, |card, cx| {
                if let Some(patch) = patch.as_ref().filter(|_| save_failed) {
                    card.save_failed(task_id, patch, cx);
                }
                if start_failed {
                    card.start_failed(task_id, cx);
                }
            }),
            TaskOrigin::Plan { create, approval } => {
                if create.is_some() && !save_failed {
                    self.delete_task(task_id, cx);
                }
                if let Some(approval) = approval {
                    self.plan_task_failed(approval, cx);
                }
            }
        }
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
        orchestrate: Option<crate::tasks::Orchestrate>,
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
            "orchestrate": orchestrate.is_some(),
            "approvePlan": orchestrate.is_none_or(|o| o.approve_plan),
            "sameAgents": orchestrate.is_some_and(|o| o.same_agents),
            "planDoc": orchestrate.is_some_and(|o| o.plan_doc),
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
            if orchestrate.is_some() {
                this.update(cx, |shell, cx| match &origin {
                    TaskOrigin::Card { card, .. } => {
                        if card.read(cx).task_id() == Some(task_id.as_str()) {
                            card.update(cx, |card, cx| card.close(None, cx));
                        }
                        shell.open_task_center(task_id.clone(), cx);
                    }
                    TaskOrigin::Plan { .. } => {}
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

    fn assign_task(
        &mut self,
        origin: TaskOrigin,
        task_id: &str,
        chat_id: &str,
        send: bool,
        lead: Option<crate::tasks::Orchestrate>,
        cx: &mut Context<Self>,
    ) {
        let Some(engine) = self.task_engine(cx) else {
            self.task_origin_failed(&origin, task_id, true, false, cx);
            return;
        };
        let update = origin.first_step(task_id);
        let assign = serde_json::json!({
            "op": "assignTask", "taskId": task_id, "chatId": chat_id, "send": send,
            "orchestrate": lead.is_some(),
            "approvePlan": lead.is_none_or(|lead| lead.approve_plan),
            "sameAgents": lead.is_some_and(|lead| lead.same_agents),
            "planDoc": lead.is_some_and(|lead| lead.plan_doc),
            "attachPlan": matches!(origin, TaskOrigin::Plan { .. }),
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
            TasksEvent::OpenCenter(task_id) => self.open_task_center(task_id.clone(), cx),
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
                orchestrate,
            } => self.start_task(
                TaskOrigin::Card {
                    card: card.clone(),
                    patch: patch.clone(),
                },
                task_id,
                space_id.clone(),
                config.clone(),
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
            } => self.assign_task(
                TaskOrigin::Card {
                    card: card.clone(),
                    patch: patch.clone(),
                },
                task_id,
                chat_id,
                *send,
                None,
                cx,
            ),
            TaskCardEvent::OpenChat(chat_id) => self.open_chat(chat_id.clone(), cx),
            TaskCardEvent::Refused(message) => {
                self.show_notice(crate::toast::ToastKind::Error, message.clone(), cx);
                cx.notify();
            }
        }
    }

    pub(crate) fn open_task_center(&mut self, id: String, cx: &mut Context<Self>) {
        let key = tree_key(self.state.read(cx), &id);
        if matches!(self.route, Route::TaskCenter)
            && self.task_center_task.as_deref() == Some(key.as_str())
        {
            return;
        }
        self.enter_task_center_route(key.clone(), cx);
        self.nav.push(NavEntry::TaskCenter(key));
    }

    pub(super) fn enter_task_center_route(&mut self, id: String, cx: &mut Context<Self>) {
        let key = tree_key(self.state.read(cx), &id);
        self.set_route(Route::TaskCenter, cx);
        self.task_center_task = Some(key);
        self.close_chat_menu(cx);
        cx.notify();
    }

    fn back_to_tasks(&mut self, cx: &mut Context<Self>) {
        if self.nav.previous() == Some(&NavEntry::Tasks) {
            self.navigate_back(cx);
        } else {
            self.open_tasks(cx);
        }
    }

    pub(super) fn route_task_slide(&mut self, to: Route) {
        let from = self.route;
        if from == to {
            return;
        }
        if matches!(
            (from, to),
            (Route::Chat, Route::TaskCenter) | (Route::TaskCenter, Route::Chat)
        ) {
            let spec = match (self.reduced_motion, to) {
                (true, _) => CENTER_FADE,
                (false, Route::TaskCenter) => CHAT_SLIDE_IN,
                (false, _) => CHAT_SLIDE_OUT,
            };
            self.task_slide.chat = true;
            self.task_slide
                .tween
                .retarget(TaskSlide::rest(to), spec, false, self.motion_time());
            self.swipe.yield_to_slide();
            return;
        }
        let under = match (from, to) {
            (Route::TaskCenter, page) | (page, Route::TaskCenter) if slides_under_center(page) => {
                page
            }
            _ => {
                self.task_slide = TaskSlide::resting(to);
                return;
            }
        };
        let spec = if self.reduced_motion {
            CENTER_FADE
        } else if to == Route::TaskCenter {
            CENTER_SLIDE_IN
        } else {
            CENTER_SLIDE_OUT
        };
        self.task_slide.under = under;
        self.task_slide.chat = false;
        self.task_slide
            .tween
            .retarget(TaskSlide::rest(to), spec, false, self.motion_time());
        self.swipe.yield_to_slide();
    }

    fn task_slide_t(&self) -> Option<f32> {
        if self.task_slide.chat
            || !matches!(self.route, Route::Tasks | Route::TaskCenter | Route::Inbox)
        {
            return None;
        }
        let now = self.render_time.unwrap_or_else(|| self.motion_time());
        if self.task_slide.tween.settled_at(now) {
            return None;
        }
        self.motion_active.set(true);
        Some(self.task_slide.tween.value_at(now))
    }

    pub(super) fn task_chat_slide_t(&self) -> Option<f32> {
        if !self.task_slide.chat || !matches!(self.route, Route::Chat | Route::TaskCenter) {
            return None;
        }
        let now = self.render_time.unwrap_or_else(|| self.motion_time());
        if self.task_slide.tween.settled_at(now) {
            return None;
        }
        self.motion_active.set(true);
        Some(self.task_slide.tween.value_at(now))
    }

    pub(super) fn render_task_chat_slide(
        &mut self,
        chat: gpui::Div,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Some(t) = self.task_chat_slide_t() else {
            return chat.into_any_element();
        };
        let travel = !self.reduced_motion;
        let leaving = self.route == Route::Chat;
        let inset = self.settings.inset_shell;
        let body = Theme::of(cx).inset_tones().body;
        let center = self.render_task_center_page(window, cx);
        let backdrop = div()
            .absolute()
            .left_0()
            .right_0()
            .bottom_0()
            .when(inset, |el| {
                el.top(px(Theme::TITLEBAR_HEIGHT)).rounded(px(CARD_RADIUS))
            })
            .when(!inset, |el| el.top_0())
            .bg(body)
            .when(travel, |el| {
                el.shadow(vec![gpui::BoxShadow {
                    color: gpui::black().opacity(CENTER_SHADOW * t),
                    offset: gpui::point(px(-20.0), px(0.0)),
                    blur_radius: px(40.0),
                    spread_radius: px(0.0),
                    inset: false,
                }])
            });
        let center = div()
            .absolute()
            .top_0()
            .bottom_0()
            .w_full()
            .left(gpui::relative(if travel { 1.0 - t } else { 0.0 }))
            .flex()
            .opacity(if travel { 0.6 + 0.4 * t } else { t })
            .child(backdrop)
            .child(center)
            .when(leaving, |el| el.child(div().absolute().inset_0().occlude()));
        let shift = if travel && leaving {
            -CHAT_PARALLAX * t
        } else {
            0.0
        };
        let alpha = if travel && leaving {
            1.0 - 0.3 * t
        } else {
            1.0
        };
        div()
            .flex_1()
            .min_w_0()
            .h_full()
            .relative()
            .overflow_hidden()
            .child(
                chat.size_full()
                    .relative()
                    .left(px(shift))
                    .opacity(alpha)
                    .when(!leaving, |el| {
                        el.child(div().absolute().inset_0().occlude())
                    }),
            )
            .child(center)
            .into_any_element()
    }

    pub(super) fn render_task_slide(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let Some(t) = self.task_slide_t() else {
            if self.route != Route::TaskCenter {
                if self.task_center_ended(cx) {
                    self.close_task_center(cx);
                } else {
                    self.watch_task_center_docs(Vec::new(), cx);
                }
            }
            return None;
        };
        let under = match self.task_slide.under {
            Route::Inbox => self.render_inbox_page(cx),
            _ => self.render_tasks_page(window, cx),
        };
        let center = self.render_task_center_page(window, cx);
        let entering = self.route == Route::TaskCenter;
        let travel = !self.reduced_motion;
        let inset = self.settings.inset_shell;
        // A flush shell has no opaque body to cover the page under the center, so it fades.
        let opaque = travel && inset;
        let body = Theme::of(cx).inset_tones().body;
        let card = |el: gpui::Div| {
            if inset {
                el.top(px(Theme::TITLEBAR_HEIGHT)).rounded(px(CARD_RADIUS))
            } else {
                el.top_0()
            }
        };
        let layer = |left: f32| {
            div()
                .absolute()
                .top_0()
                .bottom_0()
                .w_full()
                .left(gpui::relative(left))
                .flex()
        };
        let blocker = || div().absolute().inset_0().occlude();
        let under = layer(if travel { -UNDER_SHIFT * t } else { 0.0 })
            .opacity(if opaque { 1.0 } else { 1.0 - t })
            .child(under)
            .when(entering, |el| el.child(blocker()));
        let scrim = opaque.then(|| {
            card(div().absolute().left_0().right_0().bottom_0())
                .bg(gpui::black().opacity(UNDER_DIM * t))
        });
        let backdrop = inset.then(|| {
            card(div().absolute().left_0().right_0().bottom_0())
                .bg(body)
                .when(travel, |el| {
                    el.shadow(vec![gpui::BoxShadow {
                        color: gpui::black().opacity(CENTER_SHADOW * t),
                        offset: gpui::point(px(-20.0), px(0.0)),
                        blur_radius: px(40.0),
                        spread_radius: px(0.0),
                        inset: false,
                    }])
                })
        });
        let center = layer(if travel { 1.0 - t } else { 0.0 })
            .opacity(if travel { 1.0 } else { t })
            .children(backdrop)
            .child(center)
            .when(!entering, |el| el.child(blocker()));
        Some(
            div()
                .flex_1()
                .min_w_0()
                .h_full()
                .relative()
                .overflow_hidden()
                .child(under)
                .children(scrim)
                .child(center)
                .into_any_element(),
        )
    }

    pub(super) fn task_center_holds(&self, chat_id: &str) -> bool {
        self.task_center_pane
            .as_ref()
            .is_some_and(|pane| pane.chat_id == chat_id)
            || self.task_center_docs.iter().any(|id| id == chat_id)
    }

    /// Native sub-agent chips come from the root's doc and each node's that reports running sub-agents.
    fn watch_task_center_docs(&mut self, docs: Vec<String>, cx: &mut Context<Self>) {
        if self.task_center_docs == docs {
            return;
        }
        let previous = std::mem::replace(&mut self.task_center_docs, docs.clone());
        for id in previous.iter().filter(|id| !docs.contains(id)) {
            self.release_doc(id, cx);
        }
        for id in docs.into_iter().filter(|id| !previous.contains(id)) {
            self.state
                .update(cx, |state, cx| state.watch_subagent_doc(id, cx));
        }
    }

    fn task_center_ended(&self, cx: &App) -> bool {
        let Some((center, _)) = &self.task_center else {
            return false;
        };
        let center = center.read(cx);
        let state = self.state.read(cx);
        match center.resolve(state) {
            (_, Some(task)) => task.completed_at.is_some(),
            (Some(root), None) => {
                crate::tasks::tree_rows(state, root, false, chrono::Utc::now()).len() <= 1
            }
            (None, None) => true,
        }
    }

    pub(super) fn close_task_center(&mut self, cx: &mut Context<Self>) {
        self.close_task_center_pane(cx);
        self.watch_task_center_docs(Vec::new(), cx);
        self.task_center = None;
    }

    fn close_task_center_pane(&mut self, cx: &mut Context<Self>) {
        let Some(pane) = self.task_center_pane.take() else {
            return;
        };
        if let Some((center, _)) = &self.task_center {
            center.update(cx, |center, cx| center.set_live(None, cx));
        }
        self.release_doc(&pane.chat_id, cx);
    }

    pub(super) fn open_task_center_pane(
        &mut self,
        center: &Entity<TaskCenter>,
        chat_id: String,
        cx: &mut Context<Self>,
    ) {
        let pane = self.open_doc_pane(chat_id, None, true, cx);
        center.update(cx, |center, cx| {
            center.set_live(
                Some(LiveThread {
                    chat_id: pane.chat_id.clone(),
                    transcript: pane.transcript.clone(),
                    composer: pane.composer.clone(),
                }),
                cx,
            )
        });
        self.task_center_pane = Some(pane);
    }

    pub(super) fn open_doc_pane(
        &mut self,
        chat_id: String,
        placeholder: Option<&'static str>,
        opens_chat: bool,
        cx: &mut Context<Self>,
    ) -> DocPane {
        let transcript =
            cx.new(|cx| Transcript::for_doc(self.state.clone(), chat_id.clone(), true, cx));
        let links = Self::session_links(Some(chat_id.clone()), cx);
        transcript.update(cx, |transcript, _| {
            transcript.set_workspace_link_handler(links)
        });
        let events = cx.subscribe(&transcript, Self::on_transcript_event);
        self.state
            .update(cx, |s, cx| s.watch_subagent_doc(chat_id.clone(), cx));
        let composer = cx.new(|cx| {
            let mut composer = Composer::for_chat(self.state.clone(), chat_id.clone(), cx);
            composer.set_full_chat(cx);
            if let Some(placeholder) = placeholder {
                composer.set_idle_placeholder(placeholder, cx);
            }
            composer
        });
        let composer_events = cx.subscribe(&composer, {
            let transcript = transcript.clone();
            let chat_id = chat_id.clone();
            move |this: &mut Shell, composer, event: &ComposerEvent, cx| {
                this.on_doc_pane_composer_event(
                    &chat_id,
                    opens_chat,
                    &composer,
                    &transcript,
                    event,
                    cx,
                )
            }
        });
        DocPane {
            chat_id,
            transcript,
            composer,
            _events: events,
            _composer_events: composer_events,
        }
    }

    fn on_doc_pane_composer_event(
        &mut self,
        chat_id: &str,
        opens_chat: bool,
        composer: &Entity<Composer>,
        transcript: &Entity<Transcript>,
        event: &ComposerEvent,
        cx: &mut Context<Self>,
    ) {
        use crate::composer::WorkspaceCommand;
        match event {
            ComposerEvent::OpenSidechat { .. }
            | ComposerEvent::WorkspaceCommand(
                WorkspaceCommand::Terminal
                | WorkspaceCommand::Diff
                | WorkspaceCommand::Files
                | WorkspaceCommand::Rename,
            )
            | ComposerEvent::OpenChanges
                if !opens_chat => {}
            ComposerEvent::OpenSidechat {
                source_chat_id,
                prompt,
                staged,
            } => {
                self.open_chat(source_chat_id.clone(), cx);
                self.open_sidechat(source_chat_id.clone(), prompt.clone(), staged.clone(), cx);
            }
            ComposerEvent::WorkspaceCommand(command) => match command {
                WorkspaceCommand::Stop => {
                    composer.update(cx, |composer, cx| composer.interrupt_selected(cx))
                }
                WorkspaceCommand::Model => {
                    let composer = composer.clone();
                    self.defer_in_active_window(cx, move |_, window, cx| {
                        composer.update(cx, |composer, cx| composer.open_model_menu(window, cx))
                    });
                }
                WorkspaceCommand::New | WorkspaceCommand::Resume | WorkspaceCommand::Settings => {
                    self.pending_workspace_command = Some(*command);
                    cx.notify();
                }
                WorkspaceCommand::Terminal
                | WorkspaceCommand::Diff
                | WorkspaceCommand::Files
                | WorkspaceCommand::Rename => {
                    self.open_chat(chat_id.to_string(), cx);
                    self.pending_workspace_command = Some(*command);
                }
            },
            ComposerEvent::OpenChanges => {
                self.open_chat(chat_id.to_string(), cx);
                self.defer_in_active_window(cx, |shell, window, cx| {
                    shell.focus_changes(window, cx)
                });
            }
            _ => self.on_pane_composer_event(transcript, event, cx),
        }
    }

    pub(super) fn render_task_center_page(
        &mut self,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Some(task_id) = self.task_center_task.clone() else {
            return div().flex_1().into_any_element();
        };
        let center = self.task_center_for(task_id, cx);
        let docs = {
            let state = self.state.read(cx);
            let now = chrono::Utc::now();
            center
                .read(cx)
                .resolve(state)
                .0
                .map(|root| crate::tasks::watched_docs(state, root, &self.task_center_docs, now))
                .unwrap_or_default()
        };
        self.watch_task_center_docs(docs, cx);
        let target = center.read(cx).live_target(cx);
        if target.as_deref() != center.read(cx).live_chat_id() {
            self.close_task_center_pane(cx);
            if let Some(chat_id) = target {
                self.open_task_center_pane(&center, chat_id, cx);
            }
        }
        if let Some(pane) = &self.task_center_pane {
            let open = self.panels.get(&pane.chat_id).terminal_open;
            let pickers = pane.composer.read(cx).pickers().clone();
            pickers.update(cx, |pickers, cx| pickers.set_terminal_open(Some(open), cx));
        }
        div()
            .flex_1()
            .min_w_0()
            .h_full()
            .child(center)
            .into_any_element()
    }

    pub(super) fn task_center_for(
        &mut self,
        task_id: String,
        cx: &mut Context<Self>,
    ) -> Entity<TaskCenter> {
        if let Some((center, _)) = &self.task_center
            && center.read(cx).key() == task_id
        {
            return center.clone();
        }
        self.close_task_center(cx);
        let center = cx.new(|cx| TaskCenter::new(self.state.clone(), task_id, cx));
        let events = cx.subscribe(&center, |shell, _, event: &TaskCenterEvent, cx| {
            shell.on_task_center_event(event, cx)
        });
        self.task_center = Some((center.clone(), events));
        center
    }

    fn on_task_center_event(&mut self, event: &TaskCenterEvent, cx: &mut Context<Self>) {
        let params = match event {
            TaskCenterEvent::Update { task_id, patch } => {
                serde_json::json!({ "op": "updateTask", "taskId": task_id, "patch": patch })
            }
            TaskCenterEvent::ApprovePlan(task_id) => {
                serde_json::json!({ "op": "approvePlan", "taskId": task_id })
            }
            TaskCenterEvent::TakeBackApproval(task_id) => {
                serde_json::json!({ "op": "takeBackPlanApproval", "taskId": task_id })
            }
            TaskCenterEvent::RetryStep(step_id) => {
                serde_json::json!({ "op": "retryTaskStep", "stepId": step_id })
            }
            TaskCenterEvent::UpdateStep { step_id, patch } => {
                serde_json::json!({ "op": "updateTaskStep", "stepId": step_id, "patch": patch })
            }
            TaskCenterEvent::EditPlan {
                task_id,
                edit,
                approve,
            } => return self.edit_task_plan(task_id, edit, *approve, cx),
            TaskCenterEvent::OpenChat(chat_id) => return self.open_chat(chat_id.clone(), cx),
            TaskCenterEvent::OpenNative {
                parent_chat_id,
                doc_id,
                title,
                frozen,
            } => {
                return self.open_native(
                    parent_chat_id.clone(),
                    doc_id.clone(),
                    title.clone(),
                    *frozen,
                    cx,
                );
            }
            TaskCenterEvent::AddPlan(chat_id) => {
                return self.add_plan(chat_id, crate::tasks::Orchestrate::default(), None, cx);
            }
            TaskCenterEvent::Watch {
                task_id: Some(task_id),
                on,
                ..
            } => {
                serde_json::json!({ "op": "setMissionSupervised", "taskId": task_id, "supervised": on })
            }
            TaskCenterEvent::Watch {
                root,
                task_id: None,
                on,
            } => {
                serde_json::json!({ "op": "setTreeWatched", "rootChatId": root, "watched": on })
            }
            TaskCenterEvent::OpenSupervisor => return self.open_supervisor(cx),
            TaskCenterEvent::Settle { chat_id, settled } => {
                return self.set_chat_settled(chat_id.clone(), *settled, cx);
            }
            TaskCenterEvent::Back => return self.back_to_tasks(cx),
            TaskCenterEvent::Selected => return cx.notify(),
        };
        self.mutate_task_row(params, cx);
    }

    /// One spawn, so the approval never lands before the edits it follows.
    fn edit_task_plan(
        &mut self,
        task_id: &str,
        edit: &crate::tasks::PlanEdit,
        approve: bool,
        cx: &mut Context<Self>,
    ) {
        let center = self.task_center.as_ref().map(|(center, _)| center.clone());
        let Some(engine) = self.task_engine(cx) else {
            if let Some(center) = center {
                center.update(cx, |center, cx| center.edit_failed(cx));
            }
            return;
        };
        let mut params = serde_json::to_value(edit).unwrap_or_default();
        params["op"] = "editTaskPlan".into();
        params["taskId"] = task_id.into();
        let approval =
            approve.then(|| serde_json::json!({ "op": "approvePlan", "taskId": task_id }));
        cx.spawn(async move |this, cx| {
            let edited = engine.client().call(methods::MUTATE, params).await;
            this.update(cx, |shell, cx| {
                if let Some(center) = &center {
                    center.update(cx, |center, cx| match &edited {
                        Ok(_) => center.edit_sent(),
                        Err(_) => center.edit_failed(cx),
                    });
                }
                if let Err(err) = &edited {
                    shell.show_notice(crate::toast::ToastKind::Error, format!("{err}"), cx);
                    cx.notify();
                }
            })
            .ok();
            if let (Ok(_), Some(approval)) = (edited, approval)
                && let Err(err) = engine.client().call(methods::MUTATE, approval).await
            {
                this.update(cx, |shell, cx| {
                    shell.show_notice(crate::toast::ToastKind::Error, format!("{err}"), cx);
                    cx.notify();
                })
                .ok();
            }
        })
        .detach();
    }

    pub(super) fn tree_view_switch(&self, theme: &Theme, cx: &App) -> gpui::Stateful<gpui::Div> {
        let views = [
            settings::TreeView::Spine,
            settings::TreeView::Outline,
            settings::TreeView::Timeline,
        ];
        let current = settings::current(cx).tree_view;
        self.view_switch(
            "tree-view",
            zeron_doc::tree::TREE_VIEWS,
            views.iter().position(|view| *view == current).unwrap_or(0),
            TREE_VIEW_SEGMENT,
            theme,
            move |ix, s| s.tree_view = views[ix],
        )
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

/// A task id maps to its lead chat, so a tree has one page whichever id opened it.
pub(super) fn tree_key(state: &crate::state::AppState, id: &str) -> String {
    state
        .tasks
        .iter()
        .find(|task| task.id == id && !task.deleted)
        .and_then(|task| task.chat_id.clone())
        .unwrap_or_else(|| id.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plan_proposal::{PlanAnswer, PlanTarget};
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

    fn question(request_id: &str) -> PlanTarget {
        PlanTarget::Question(PlanAnswer {
            request_id: request_id.into(),
            question_id: "plan".into(),
        })
    }

    fn approve(
        shell: &Entity<Shell>,
        cx: &mut VisualTestContext,
        chat_id: &str,
        target: PlanTarget,
        as_task: bool,
    ) {
        shell.update(cx, |shell, cx| {
            shell.approve_plan(chat_id, target, as_task, cx)
        });
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

    fn flight(shell: &Entity<Shell>, cx: &mut VisualTestContext, request_id: &str) -> Option<bool> {
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
    fn approve_answers_the_clicked_plan_even_when_the_chat_leads_a_waiting_task(
        cx: &mut TestAppContext,
    ) {
        let (shell, cx, mut wire, _dir) = setup(cx);
        let _guard = wire.runtime.enter();
        shell.update(cx, |shell, cx| {
            shell.state.update(cx, |state, _| {
                state.tasks = vec![
                    serde_json::from_value(serde_json::json!({
                        "id": "t1", "chatId": "c", "mode": "orchestrated", "planPending": true,
                    }))
                    .unwrap(),
                ];
            })
        });

        approve(&shell, cx, "c", question("req-1"), true);
        assert!(
            wire.take(cx).is_empty(),
            "a Task lead cannot become another Task"
        );

        approve(&shell, cx, "c", question("req-1"), false);
        let frames = wire.take(cx);
        assert_eq!(ops(&frames), ["answer req-1"]);
        let answers = &frames[0].params["command"]["answers"];
        assert_eq!(answers[0]["labels"][0], "Yes", "{answers}");

        approve(&shell, cx, "c", PlanTarget::Task("t1".into()), false);
        let frames = wire.take(cx);
        assert_eq!(ops(&frames), ["approvePlan"]);
        assert_eq!(frames[0].params["taskId"], "t1");
    }

    #[gpui::test]
    fn approve_as_task_leaves_plan_mode_only_once_answered_and_restores_it_when_the_task_fails(
        cx: &mut TestAppContext,
    ) {
        let (shell, cx, mut wire, _dir) = setup(cx);
        let _guard = wire.runtime.enter();

        approve(&shell, cx, "far", question("req-far"), true);
        assert!(wire.take(cx).is_empty(), "another device's chat");
        assert_eq!(flight(&shell, cx, "req-far"), None);

        approve(&shell, cx, "c", question("req-2"), true);
        let frames = wire.take(cx);
        assert_eq!(ops(&frames), ["answer req-2"]);
        assert_eq!(flight(&shell, cx, "req-2"), Some(true));
        wire.reply(&frames[0], Some("host gone"));
        assert!(
            wire.take(cx).is_empty(),
            "a failed answer changes nothing else"
        );
        assert_eq!(flight(&shell, cx, "req-2"), None);
        assert!(plan_on(&shell, cx, "c"));

        approve(&shell, cx, "c", question("req-2"), true);
        let frames = wire.take(cx);
        assert_eq!(ops(&frames), ["answer req-2"]);
        wire.reply(&frames[0], None);
        let frames = wire.take(cx);
        assert_eq!(ops(&frames), ["setChatConfig"]);
        assert_ne!(frames[0].params["config"]["plan"], true);
        assert!(!plan_on(&shell, cx, "c"));
        wire.reply(&frames[0], None);
        let frames = wire.take(cx);
        assert_eq!(ops(&frames), ["createTask"]);
        wire.reply(&frames[0], None);
        let frames = wire.take(cx);
        assert_eq!(ops(&frames), ["assignTask"]);
        assert_eq!(flight(&shell, cx, "req-2"), Some(true));
        wire.reply(&frames[0], Some("no lead"));
        let frames = wire.take(cx);
        let mut seen = ops(&frames);
        seen.sort();
        assert_eq!(seen, ["deleteTask", "setChatConfig"]);
        assert_eq!(flight(&shell, cx, "req-2"), None);
        assert!(plan_on(&shell, cx, "c"), "the chat is back in plan mode");
    }
}
