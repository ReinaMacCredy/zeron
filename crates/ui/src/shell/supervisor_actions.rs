use super::*;
use crate::supervisor::{NeedsYouRoute, Pane, SupervisorEvent, SupervisorPage};
use zeron_proto::supervisor::NeedsYouRow;

#[derive(Default)]
pub(super) struct SupervisorShell {
    page: Option<(Entity<SupervisorPage>, Subscription)>,
    pane: Option<SupervisorPane>,
    doc: Option<String>,
    seen: Option<std::collections::HashSet<String>>,
    badge: Option<(usize, crate::motion::Tween)>,
}

pub(super) struct SupervisorPane {
    chat_id: String,
    transcript: Entity<Transcript>,
    composer: Entity<Composer>,
    _events: Subscription,
    _composer_events: Subscription,
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
            .filter(|pane| &pane.transcript == transcript)
            .map(|pane| pane.composer.clone())
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

    fn supervisor_doc_held(&self, chat_id: &str) -> bool {
        self.supervisor.holds(chat_id)
            || self.btw_tabs.contains_key(chat_id)
            || self
                .sidechat_tabs
                .values()
                .any(|tab| tab.chat_id == chat_id)
            || self.subagent_tabs.values().any(|tab| tab.doc_id == chat_id)
            || self.split_chat_id() == Some(chat_id)
            || self.task_center_holds(chat_id)
    }

    fn set_supervisor_doc(&mut self, doc: Option<String>, cx: &mut Context<Self>) {
        let previous = std::mem::replace(&mut self.supervisor.doc, doc.clone());
        if previous == doc {
            return;
        }
        if let Some(previous) = previous.filter(|id| !self.supervisor_doc_held(id)) {
            self.state
                .update(cx, |state, _| state.unwatch_subagent_doc(&previous));
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
        if !self.supervisor_doc_held(&pane.chat_id) {
            self.state
                .update(cx, |state, _| state.unwatch_subagent_doc(&pane.chat_id));
        }
    }

    fn open_supervisor_pane(
        &mut self,
        page: &Entity<SupervisorPage>,
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
        let composer = cx.new(|cx| {
            let mut composer = Composer::for_chat(self.state.clone(), chat_id.clone(), cx);
            composer.set_full_chat(cx);
            composer.set_idle_placeholder(crate::supervisor::PLACEHOLDER, cx);
            composer
        });
        let composer_events = cx.subscribe(&composer, {
            let transcript = transcript.clone();
            let chat_id = chat_id.clone();
            move |this: &mut Shell, composer, event: &ComposerEvent, cx| {
                this.on_supervisor_composer_event(&chat_id, &composer, &transcript, event, cx)
            }
        });
        page.update(cx, |page, cx| {
            page.set_pane(
                Some(Pane {
                    transcript: transcript.clone(),
                    composer: composer.clone(),
                }),
                cx,
            )
        });
        self.supervisor.pane = Some(SupervisorPane {
            chat_id,
            transcript,
            composer,
            _events: events,
            _composer_events: composer_events,
        });
    }

    fn on_supervisor_composer_event(
        &mut self,
        chat_id: &str,
        composer: &Entity<Composer>,
        transcript: &Entity<Transcript>,
        event: &ComposerEvent,
        cx: &mut Context<Self>,
    ) {
        use crate::composer::WorkspaceCommand;
        match event {
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
            ComposerEvent::OpenChat(target) => self.open_chat(target.clone(), cx),
            ComposerEvent::OpenChanges => {
                self.open_chat(chat_id.to_string(), cx);
                self.defer_in_active_window(cx, |shell, window, cx| {
                    shell.focus_changes(window, cx)
                });
            }
            _ => self.on_pane_composer_event(transcript, event, cx),
        }
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
        if let Some(pane) = &self.supervisor.pane {
            let open = self.panels.get(&pane.chat_id).terminal_open;
            let pickers = pane.composer.read(cx).pickers().clone();
            pickers.update(cx, |pickers, cx| pickers.set_terminal_open(Some(open), cx));
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
            Route::TaskCenter => task_id.is_some() && self.task_center_task.as_deref() == task_id,
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
