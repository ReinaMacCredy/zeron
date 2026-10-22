use super::*;
use crate::supervisor::{NeedsYouRoute, Pane, SupervisorEvent, SupervisorPage};
use zeron_proto::supervisor::NeedsYouRow;

#[derive(Default)]
pub(super) struct SupervisorShell {
    page: Option<(Entity<SupervisorPage>, Subscription)>,
    pane: Option<super::task_actions::DocPane>,
    doc: Option<String>,
    seen: Option<std::collections::HashSet<String>>,
    badge: Option<(usize, crate::motion::Tween)>,
}

impl SupervisorShell {
    pub(super) fn holds(&self, chat_id: &str) -> bool {
        self.doc.as_deref() == Some(chat_id)
            || self
                .pane
                .as_ref()
                .is_some_and(|pane| pane.chat_id == chat_id)
    }

    pub(super) fn composer_for(&self, transcript: &Entity<Transcript>) -> Option<Entity<Composer>> {
        self.pane
            .as_ref()
            .and_then(|pane| pane.composer_for(transcript))
    }
}

impl Shell {
    pub(crate) fn open_supervisor(&mut self, cx: &mut Context<Self>) {
        if matches!(self.route, Route::Supervisor) {
            return;
        }
        self.enter_supervisor_route(cx);
        self.nav.push(NavEntry::Supervisor);
    }

    pub(super) fn enter_supervisor_route(&mut self, cx: &mut Context<Self>) {
        self.set_route(Route::Supervisor, cx);
        self.close_user_menu(cx);
        self.close_chat_menu(cx);
        if let Some((page, _)) = &self.supervisor.page {
            page.update(cx, |page, cx| page.entered(cx));
        }
        cx.notify();
    }

    pub(super) fn leave_supervisor(&mut self, cx: &mut Context<Self>) {
        self.close_supervisor_pane(cx);
        self.set_supervisor_doc(None, cx);
        if let Some((page, _)) = &self.supervisor.page {
            page.update(cx, |page, cx| page.left(cx));
        }
    }

    fn set_supervisor_doc(&mut self, doc: Option<String>, cx: &mut Context<Self>) {
        let previous = std::mem::replace(&mut self.supervisor.doc, doc.clone());
        if previous == doc {
            return;
        }
        if let Some(previous) = previous {
            self.release_doc(&previous, cx);
        }
        if let Some(doc) = doc {
            self.state
                .update(cx, |state, cx| state.watch_subagent_doc(doc, cx));
        }
    }

    fn close_supervisor_pane(&mut self, cx: &mut Context<Self>) {
        let Some(pane) = self.supervisor.pane.take() else {
            return;
        };
        if let Some((page, _)) = &self.supervisor.page {
            page.update(cx, |page, cx| page.set_pane(None, cx));
        }
        self.release_doc(&pane.chat_id, cx);
    }

    fn open_supervisor_pane(
        &mut self,
        page: &Entity<SupervisorPage>,
        chat_id: String,
        cx: &mut Context<Self>,
    ) {
        let pane = self.open_doc_pane(chat_id, Some(crate::supervisor::PLACEHOLDER), false, cx);
        page.update(cx, |page, cx| {
            page.set_pane(
                Some(Pane {
                    transcript: pane.transcript.clone(),
                    composer: pane.composer.clone(),
                }),
                cx,
            )
        });
        self.supervisor.pane = Some(pane);
    }

    pub(super) fn render_supervisor_page(
        &mut self,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let page = match &self.supervisor.page {
            Some((page, _)) => page.clone(),
            None => {
                let page = cx.new(|cx| SupervisorPage::new(self.state.clone(), cx));
                let events = cx.subscribe(&page, |shell, _, event: &SupervisorEvent, cx| {
                    shell.on_supervisor_event(event, cx)
                });
                page.update(cx, |page, cx| page.entered(cx));
                self.supervisor.page = Some((page.clone(), events));
                page
            }
        };
        let target = page.read(cx).chat_id(cx);
        let current = self
            .supervisor
            .pane
            .as_ref()
            .map(|pane| pane.chat_id.clone());
        if target != current {
            self.close_supervisor_pane(cx);
            if let Some(chat_id) = target {
                self.open_supervisor_pane(&page, chat_id, cx);
            }
        }
        div()
            .flex_1()
            .min_w_0()
            .h_full()
            .child(page)
            .into_any_element()
    }

    fn on_supervisor_event(&mut self, event: &SupervisorEvent, cx: &mut Context<Self>) {
        match event {
            SupervisorEvent::OpenTaskCenter(task_id) => self.open_task_center(task_id.clone(), cx),
            SupervisorEvent::OpenMission(task_id, selection) => {
                self.open_mission(task_id.clone(), selection.clone(), cx)
            }
            SupervisorEvent::OpenChat(chat_id) => self.open_chat(chat_id.clone(), cx),
            SupervisorEvent::WatchDoc(doc) => self.set_supervisor_doc(doc.clone(), cx),
            SupervisorEvent::Discuss(text) => self.discuss_with_supervisor(text.clone(), cx),
        }
    }

    fn discuss_with_supervisor(&mut self, text: String, cx: &mut Context<Self>) {
        let Some(pane) = &self.supervisor.pane else {
            return;
        };
        let (chat_id, composer) = (pane.chat_id.clone(), pane.composer.clone());
        self.state.update(cx, |state, cx| {
            state.add_quote(&chat_id, crate::quotes::Quote::new(text));
            cx.notify();
        });
        self.defer_in_active_window(cx, move |_, window, cx| {
            window.focus(&composer.focus_handle(cx), cx)
        });
    }

    fn looking_at_needs_you(&self, row: &NeedsYouRow, cx: &App) -> bool {
        let (task_id, raiser) = crate::supervisor::row_origin(row);
        match self.route {
            Route::Supervisor => true,
            Route::TaskCenter => {
                let state = self.state.read(cx);
                task_id.is_some_and(|task_id| {
                    self.task_center_task.as_deref()
                        == Some(super::task_actions::tree_key(state, task_id).as_str())
                })
            }
            Route::Chat => self.state.read(cx).selected_chat.as_deref() == Some(raiser),
            _ => false,
        }
    }

    pub(super) fn supervisor_badge_pop(&mut self, count: usize, cx: &App) -> Option<f32> {
        let now = std::time::Instant::now();
        let reduced = crate::motion::reduced_motion(cx);
        let (seen, pop) = self
            .supervisor
            .badge
            .get_or_insert_with(|| (count, crate::motion::Tween::at(1.0, now)));
        if count > *seen {
            *pop = crate::motion::Tween::at(0.0, now);
            pop.retarget(1.0, crate::supervisor::BADGE_POP, reduced, now);
        }
        *seen = count;
        (!pop.settled_at(now)).then(|| crate::supervisor::badge_scale(pop.value_at(now)))
    }

    pub(super) fn route_supervisor_needs(&mut self, cx: &mut Context<Self>) {
        let fresh: Vec<NeedsYouRow> = match self.state.read(cx).supervisor.as_ref() {
            None => {
                self.supervisor.seen = None;
                return;
            }
            Some(snapshot) => {
                crate::supervisor::fresh_needs_you(&mut self.supervisor.seen, &snapshot.needs_you)
            }
        };
        let focused = cx.active_window().is_some();
        let since = self.last_interaction.as_ref().map(|(_, at)| at.elapsed());
        for row in fresh {
            let looking = self.looking_at_needs_you(&row, cx);
            let title = crate::supervisor::row_title(&row);
            match crate::supervisor::needs_you_route(looking, focused, since) {
                NeedsYouRoute::Quiet => {}
                NeedsYouRoute::InApp => crate::toast::show(
                    cx,
                    crate::toast::ToastKind::Info,
                    format!("Supervisor needs you: {title}"),
                ),
                NeedsYouRoute::Banner if self.settings.notifications_enabled => {
                    crate::notify::post(
                        "Supervisor needs you",
                        &title,
                        Some(crate::notify::SUPERVISOR_TARGET),
                    )
                }
                NeedsYouRoute::Banner => {}
            }
        }
    }
}
