use super::*;
use crate::tasks::{LiveThread, TaskCenter, TaskCenterEvent};
use zeron_proto::TaskPatch;

const TASKS_VIEW_SEGMENT: f32 = 72.0;
const TASKS_VIEW_THUMB: motion::MotionSpec = motion::MotionSpec::new(260, motion::EASE_DRAWER);
const CENTER_SLIDE_IN: motion::MotionSpec = motion::MotionSpec::new(380, motion::EASE_DRAWER);
const CENTER_SLIDE_OUT: motion::MotionSpec = motion::MotionSpec::new(300, motion::EASE_DRAWER);
const CENTER_FADE: motion::MotionSpec = motion::MotionSpec::new(150, motion::EASE);
const UNDER_SHIFT: f32 = 0.24;
const UNDER_DIM: f32 = 0.45;
const CENTER_SHADOW: f32 = 0.55;

pub(super) struct TaskSlide {
    tween: motion::Tween,
    under: Route,
}

impl TaskSlide {
    pub(super) fn resting(route: Route) -> Self {
        Self {
            tween: motion::Tween::at(Self::rest(route), std::time::Instant::now()),
            under: Route::Tasks,
        }
    }

    fn rest(route: Route) -> f32 {
        if route == Route::TaskCenter { 1.0 } else { 0.0 }
    }
}

fn slides_under_center(route: Route) -> bool {
    matches!(route, Route::Tasks | Route::Inbox)
}

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
        config: Option<ChatConfig>,
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
            "spaceId": space_id, "config": config,
            "orchestrate": orchestrate.is_some(),
            "approvePlan": orchestrate.is_none_or(|o| o.approve_plan),
            "sameAgents": orchestrate.is_some_and(|o| o.same_agents),
            "planDoc": orchestrate.is_some_and(|o| o.plan_doc),
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
                        card.update(cx, |card, cx| card.close(None, cx));
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
                config,
                orchestrate,
            } => self.start_task(
                card,
                task_id,
                patch.clone(),
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

    fn back_to_tasks(&mut self, cx: &mut Context<Self>) {
        if self.nav.previous() == Some(&NavEntry::Tasks) {
            self.navigate_back(cx);
        } else {
            self.open_tasks(cx);
        }
    }

    pub(super) fn route_task_slide(&mut self, to: Route, cx: &mut Context<Self>) {
        let from = self.route;
        if from == to {
            return;
        }
        let under = match (from, to) {
            (Route::TaskCenter, page) | (page, Route::TaskCenter) if slides_under_center(page) => {
                page
            }
            _ => {
                if to != Route::TaskCenter {
                    self.close_task_center(cx);
                }
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
        self.task_slide
            .tween
            .retarget(TaskSlide::rest(to), spec, false, self.motion_time());
        self.swipe.yield_to_slide();
    }

    fn task_slide_t(&self) -> Option<f32> {
        if !matches!(self.route, Route::Tasks | Route::TaskCenter | Route::Inbox) {
            return None;
        }
        let now = self.render_time.unwrap_or_else(|| self.motion_time());
        if self.task_slide.tween.settled_at(now) {
            return None;
        }
        self.motion_active.set(true);
        Some(self.task_slide.tween.value_at(now))
    }

    pub(super) fn render_task_slide(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let Some(t) = self.task_slide_t() else {
            if self.route != Route::TaskCenter && self.task_center.is_some() {
                self.close_task_center(cx);
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

    pub(super) fn render_task_slide_title_bar(
        &mut self,
        column: f32,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let t = self.task_slide_t()?;
        let under = match self.task_slide.under {
            Route::Inbox => self.render_session_title_bar(window, cx),
            _ => self.render_tasks_title_bar(cx),
        };
        let center = self.render_task_center_title_bar(cx);
        let entering = self.route == Route::TaskCenter;
        let reduced = self.reduced_motion;
        let width = self.viewport_width - self.content_inset();
        let edge = self.sidebar_edge() + 1.0;
        let split = edge + column * (1.0 - t);
        let layer =
            |bar: AnyElement, clip: Option<(f32, f32)>, shift: f32, alpha: f32, live: bool| {
                let (left, right) = clip.unwrap_or((0.0, width));
                div()
                    .absolute()
                    .top_0()
                    .bottom_0()
                    .left(px(left))
                    .w(px((right - left).max(0.0)))
                    .overflow_hidden()
                    .opacity(alpha)
                    .child(
                        div()
                            .absolute()
                            .top_0()
                            .left(px(shift - left))
                            .w(px(width))
                            .child(bar),
                    )
                    .when(!live, |el| el.child(div().absolute().inset_0().occlude()))
            };
        let (under, center) = if reduced {
            (
                layer(under, None, 0.0, 1.0 - t, !entering),
                layer(center, None, 0.0, t, entering),
            )
        } else {
            (
                layer(
                    under,
                    Some((edge.max(self.title_bar_content_start()), split)),
                    -UNDER_SHIFT * column * t,
                    1.0 - UNDER_DIM * t,
                    !entering,
                ),
                layer(
                    center,
                    Some((split, width)),
                    column * (1.0 - t),
                    1.0,
                    entering,
                ),
            )
        };
        let (back, front) = if entering {
            (under, center)
        } else {
            (center, under)
        };
        Some(
            div()
                .relative()
                .w_full()
                .h(px(Theme::TITLEBAR_HEIGHT))
                .when(!reduced, |el| el.children(self.titlebar_seam_divider(cx)))
                .child(back)
                .child(front)
                .into_any_element(),
        )
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
            TaskCenterEvent::EditPlan {
                task_id,
                edit,
                approve,
            } => return self.edit_task_plan(task_id, edit, *approve, cx),
            TaskCenterEvent::OpenChat(chat_id) => return self.open_chat(chat_id.clone(), cx),
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
            .on_click(cx.listener(|shell, _: &gpui::ClickEvent, _, cx| shell.back_to_tasks(cx)));
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
