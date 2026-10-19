use std::collections::{HashMap, HashSet};
use std::time::Duration;

use zeron_proto::supervisor::{
    SupervisorEventKind as Kind, SupervisorEventSummary, SupervisorState, SupervisorWake,
    WakeDelivery,
};
use zeron_proto::{
    Chat, PermissionMode, Session, SessionStatus, StepTurnEnd, Task, TaskMode, TaskStep,
    UserInputQuestion,
};

use super::delegation::{WAKE_QUESTION_CHARS, clip, question_lines};
use super::*;
use crate::registry::HarnessRegistry;
use crate::supervisor::{
    Flush, MissionFacts, Reply, Stall, SupervisorEvent, SupervisorStore, ThreadFacts, WakeEntry,
    WakeHeader, fold_ms, plan_flush, plan_stalls, utc, wake_text,
};

const MAX_NAP: Duration = Duration::from_secs(60);
const HOUR_MS: i64 = 60 * 60 * 1000;
const TITLE_CHARS: usize = 80;

pub(crate) fn newest_supervisor<'a>(chats: &'a [Chat], device: &str) -> Option<&'a Chat> {
    chats
        .iter()
        .filter(|chat| {
            zeron_proto::supervisor::is_supervisor_chat(&chat.id)
                && chat.device_id == device
                && !chat.archived
        })
        .max_by_key(|chat| chat.created_at)
}

fn live(status: SessionStatus) -> bool {
    matches!(
        status,
        SessionStatus::Working | SessionStatus::AwaitingInput
    )
}

fn ms(at: chrono::DateTime<chrono::Utc>) -> i64 {
    at.timestamp_millis()
}

fn step_failed(step: &TaskStep) -> bool {
    step.outcome.is_none()
        && step
            .turn
            .as_ref()
            .is_some_and(|turn| turn.ended == StepTurnEnd::Failed)
}

#[derive(Default)]
struct Observed {
    live: HashSet<String>,
    last_live: HashMap<String, i64>,
    open: Option<HashSet<String>>,
}

impl Observed {
    fn sessions(&mut self, sessions: &[Session], now: i64) {
        let mut live_now = HashSet::new();
        for session in sessions.iter().filter(|session| live(session.status)) {
            live_now.insert(session.chat_id.clone());
            self.last_live.insert(session.chat_id.clone(), now);
        }
        for ended in self.live.difference(&live_now) {
            self.last_live.insert(ended.clone(), now);
        }
        self.live = live_now;
    }

    fn missions_done(&mut self, tasks: &[Task]) -> Vec<Task> {
        let started = |task: &&Task| {
            task.mode == TaskMode::Orchestrated && task.chat_id.is_some() && !task.deleted
        };
        let open_now: HashSet<String> = tasks
            .iter()
            .filter(started)
            .filter(|task| task.completed_at.is_none())
            .map(|task| task.id.clone())
            .collect();
        let done = match &self.open {
            Some(open) => tasks
                .iter()
                .filter(started)
                .filter(|task| task.completed_at.is_some() && open.contains(&task.id))
                .cloned()
                .collect(),
            None => Vec::new(),
        };
        self.open = Some(open_now);
        done
    }
}

struct Round {
    missions: HashMap<String, MissionFacts>,
    done: Vec<Task>,
    step_titles: HashMap<String, String>,
}

impl DocHost {
    pub(crate) fn set_supervisor(&self, store: SupervisorStore) {
        let _ = self.inner.supervisor.set(store);
    }

    pub(crate) fn supervisor(&self) -> Option<&SupervisorStore> {
        self.inner.supervisor.get()
    }

    /// The chat watch publishes after a write, so a just-created Supervisor is found by its cached id.
    fn supervisor_chat(&self) -> Option<Chat> {
        let (store, workspace) = (self.supervisor()?, self.workspace()?);
        if let Some(chat) = store
            .chat_id()
            .and_then(|id| workspace.chat(&id).ok().flatten())
            .filter(|chat| !chat.archived)
        {
            return Some(chat);
        }
        let chats = workspace.watch_chats();
        let found = newest_supervisor(&chats.borrow(), self.device_id()).cloned();
        store.set_chat_id(found.as_ref().map(|chat| chat.id.clone()));
        found
    }

    pub(crate) fn supervisor_event(&self, event: SupervisorEvent) {
        let (Some(store), Some(workspace)) = (self.supervisor(), self.workspace()) else {
            return;
        };
        let Some(supervisor) = self.supervisor_chat() else {
            return;
        };
        if event.actor.as_deref() == Some(supervisor.id.as_str()) {
            return;
        }
        if let Some(task_id) = event.summary.task_id.as_deref()
            && !self.hosts_mission(workspace, task_id)
        {
            return;
        }
        store.offer(event);
    }

    fn hosts_mission(&self, workspace: &WorkspaceHost, task_id: &str) -> bool {
        workspace
            .task(task_id)
            .filter(|task| task.mode == TaskMode::Orchestrated && !task.deleted)
            .and_then(|task| task.chat_id)
            .and_then(|lead| workspace.chat(&lead).ok().flatten())
            .is_some_and(|lead| lead.device_id == self.device_id() && !lead.archived)
    }

    pub(super) fn supervisor_step_ended(&self, step: &TaskStep, thread: &str, ended: StepTurnEnd) {
        if ended != StepTurnEnd::Failed {
            return;
        }
        let Some(store) = self.supervisor() else {
            return;
        };
        let at = now_ms();
        store.note_failed(&step.id, at);
        let mut event = SupervisorEvent::new(
            Kind::StepFailed,
            format!("failed:{}:{at}", step.id),
            Some(&step.task_id),
            format!("Step \"{}\" failed", clip(&step.title, TITLE_CHARS)),
        );
        event.summary.step_id = Some(step.id.clone());
        event.summary.thread_id = Some(thread.to_string());
        self.supervisor_event(event);
    }

    pub(super) fn supervisor_input(
        &self,
        asker: &Chat,
        request_id: &str,
        questions: &[UserInputQuestion],
    ) {
        let Some(workspace) = self.workspace() else {
            return;
        };
        let step = workspace
            .watch_task_steps()
            .borrow()
            .iter()
            .find(|step| step.outcome.is_none() && step.chat_id.as_deref() == Some(&asker.id))
            .cloned();
        let task_id = match &step {
            Some(step) => step.task_id.clone(),
            None => {
                let led = workspace
                    .watch_tasks()
                    .borrow()
                    .iter()
                    .find(|task| {
                        task.completed_at.is_none()
                            && task.mode == TaskMode::Orchestrated
                            && task.chat_id.as_deref() == Some(&asker.id)
                    })
                    .map(|task| task.id.clone());
                let Some(task_id) = led else {
                    return;
                };
                task_id
            }
        };
        let who = match &step {
            Some(step) => format!("Step \"{}\"", clip(&step.title, TITLE_CHARS)),
            None => "The lead".to_string(),
        };
        let what = if questions.iter().any(|q| q.approval) {
            "waits for a tool approval"
        } else {
            "asks a question"
        };
        let mut event = SupervisorEvent::new(
            Kind::NeedsYou,
            format!("input:{}:{request_id}", asker.id),
            Some(&task_id),
            format!("{who} {what}"),
        );
        event.summary.step_id = step.map(|step| step.id);
        event.summary.thread_id = Some(asker.id.clone());
        event.request_id = Some(request_id.to_string());
        event.detail = question_lines(questions);
        event.detail.push(format!("requestId {request_id}"));
        self.supervisor_event(event);
    }

    pub(crate) fn supervisor_plan_edited(&self, task: &Task, changes: &[String]) {
        let mut event = SupervisorEvent::new(
            Kind::PlanEdited,
            format!("plan:{}:{}", task.id, new_id()),
            Some(&task.id),
            "Plan edited by the person".to_string(),
        );
        event.detail = changes.iter().map(|change| format!("- {change}")).collect();
        self.supervisor_event(event);
    }

    pub(crate) fn supervisor_replanned(
        &self,
        task: &Task,
        before: &[TaskStep],
        steps: &[TaskStep],
        from: Option<&str>,
    ) {
        if task.plan_pending || before.is_empty() {
            return;
        }
        let mut event = SupervisorEvent::new(
            Kind::PlanEdited,
            format!("plan:{}:{}", task.id, new_id()),
            Some(&task.id),
            "Plan rewritten by the lead".to_string(),
        );
        event.detail = steps
            .iter()
            .map(|step| format!("- new step \"{}\"", clip(&step.title, TITLE_CHARS)))
            .collect();
        event.actor = from.map(str::to_owned);
        self.supervisor_event(event);
    }

    pub(crate) fn spawn_supervisor_router(&self, registry: std::sync::Arc<HarnessRegistry>) {
        let (Some(store), Some(workspace), Some(sessions)) = (
            self.supervisor().cloned(),
            self.workspace().cloned(),
            self.sessions(),
        ) else {
            return;
        };
        let host = self.clone();
        self.spawn_worker(async move {
            let mut statuses = sessions.watch_sessions();
            let mut chats = workspace.watch_chats();
            let mut tasks = workspace.watch_tasks();
            let mut steps = workspace.watch_task_steps();
            let mut observed = Observed::default();
            let started = now_ms();
            loop {
                let now = now_ms();
                let round = host.observe(
                    &store,
                    &mut observed,
                    now,
                    &statuses.borrow_and_update(),
                    &chats.borrow_and_update(),
                    &tasks.borrow_and_update(),
                    &steps.borrow_and_update(),
                );
                let next = host
                    .supervise(
                        &store, &workspace, &sessions, &registry, round, now, started,
                    )
                    .await;
                let nap = next.map(|at| {
                    Duration::from_millis(u64::try_from(at - now).unwrap_or(0)).min(MAX_NAP)
                });
                tokio::select! {
                    changed = statuses.changed() => if changed.is_err() { return },
                    changed = chats.changed() => if changed.is_err() { return },
                    changed = tasks.changed() => if changed.is_err() { return },
                    changed = steps.changed() => if changed.is_err() { return },
                    () = store.poked() => {}
                    () = tokio::time::sleep(nap.unwrap_or_default()), if nap.is_some() => {}
                }
            }
        });
    }

    #[allow(clippy::too_many_arguments)]
    fn observe(
        &self,
        store: &SupervisorStore,
        observed: &mut Observed,
        now: i64,
        sessions: &[Session],
        chats: &[Chat],
        tasks: &[Task],
        steps: &[TaskStep],
    ) -> Round {
        observed.sessions(sessions, now);
        let done = observed.missions_done(tasks);
        let sessions: HashMap<&str, &Session> = sessions
            .iter()
            .map(|session| (session.chat_id.as_str(), session))
            .collect();
        let chats: HashMap<&str, &Chat> =
            chats.iter().map(|chat| (chat.id.as_str(), chat)).collect();
        let thread = |id: &str| ThreadFacts {
            live: sessions.get(id).is_some_and(|s| live(s.status)),
            background: sessions
                .get(id)
                .is_some_and(|s| !s.background_tasks.is_empty()),
            last_message: chats.get(id).and_then(|c| c.last_message_at).map(ms),
            last_live: observed.last_live.get(id).copied(),
        };
        let mut missions = HashMap::new();
        let mut step_titles = HashMap::new();
        for task in tasks {
            if task.mode != TaskMode::Orchestrated
                || task.deleted
                || task.completed_at.is_some()
                || store.is_unwatched(&task.id)
            {
                continue;
            }
            let Some(lead) = task.chat_id.as_deref().and_then(|id| chats.get(id)) else {
                continue;
            };
            if lead.device_id != self.device_id() || lead.archived {
                continue;
            }
            let own: Vec<&TaskStep> = steps.iter().filter(|s| s.task_id == task.id).collect();
            let mut facts = MissionFacts {
                task_id: task.id.clone(),
                waiting_on_person: task.plan_pending && !own.is_empty(),
                lead: thread(&lead.id),
                steps: Vec::new(),
                failed: Vec::new(),
            };
            for step in own {
                step_titles.insert(step.id.clone(), step.title.clone());
                let Some(bound) = step.chat_id.as_deref() else {
                    continue;
                };
                let step_thread = thread(bound);
                if step_failed(step) && !step_thread.live {
                    facts.failed.push(crate::supervisor::FailedStep {
                        step_id: step.id.clone(),
                        failed_at: store
                            .failed_at(&step.id)
                            .unwrap_or_else(|| ms(step.updated_at)),
                    });
                }
                facts.steps.push(step_thread);
            }
            missions.insert(task.id.clone(), facts);
        }
        Round {
            missions,
            done,
            step_titles,
        }
    }

    #[allow(clippy::too_many_arguments)]
    async fn supervise(
        &self,
        store: &SupervisorStore,
        workspace: &WorkspaceHost,
        sessions: &SessionsEngine,
        registry: &HarnessRegistry,
        round: Round,
        now: i64,
        started: i64,
    ) -> Option<i64> {
        for task in &round.done {
            let completed = task.completed_at.map(ms).unwrap_or(now);
            self.supervisor_event(SupervisorEvent::new(
                Kind::MissionDone,
                format!("done:{}:{completed}", task.id),
                Some(&task.id),
                "Mission marked done".to_string(),
            ));
        }
        let Some(supervisor) = self.supervisor_chat() else {
            store.take_pending();
            store.publish(None, SupervisorState::Off);
            return None;
        };
        let missions: Vec<MissionFacts> = round.missions.values().cloned().collect();
        let (stalls, mut next) = plan_stalls(now, &missions, |key| store.reported(key));
        for stall in stalls {
            self.supervisor_event(stall_event(stall, &round.step_titles));
        }
        let settings = registry.supervisor_settings();
        let interval = i64::from(settings.heartbeat_minutes) * 60_000;
        if interval > 0 && !missions.is_empty() {
            let last = store.wake_times().into_iter().max().unwrap_or(0);
            let due = last.max(started) + interval;
            if due <= now {
                self.supervisor_event(SupervisorEvent::new(
                    Kind::Heartbeat,
                    format!("beat:{due}"),
                    None,
                    "Heartbeat".to_string(),
                ));
            } else {
                next = Some(next.map_or(due, |n| n.min(due)));
            }
        }
        let held = self
            .flush_supervisor(
                store,
                workspace,
                sessions,
                &supervisor,
                &round.missions,
                settings,
                now,
            )
            .await;
        if let Some(until) = held {
            next = Some(next.map_or(until, |n| n.min(until)));
        }
        let state = match sessions.session_status(&supervisor.id) {
            Some(session) if live(session.status) => SupervisorState::Awake,
            _ => SupervisorState::Asleep,
        };
        store.publish(Some(supervisor.id), state);
        next
    }

    #[allow(clippy::too_many_arguments)]
    async fn flush_supervisor(
        &self,
        store: &SupervisorStore,
        workspace: &WorkspaceHost,
        sessions: &SessionsEngine,
        supervisor: &Chat,
        missions: &HashMap<String, MissionFacts>,
        settings: zeron_proto::supervisor::SupervisorSettings,
        now: i64,
    ) -> Option<i64> {
        if !store.has_pending() {
            return None;
        }
        let held = store.held();
        let undelivered = held.as_ref().map(|(id, _)| id.as_str()).filter(|id| {
            self.queued_row(&supervisor.id, id)
                .is_some_and(|row| row.delivery_gate.is_none())
        });
        let plan = plan_flush(now, fold_ms(), &store.wake_times(), undelivered);
        if let Flush::HoldUntil(until) = plan {
            return Some(until);
        }
        let (keep, dropped): (Vec<_>, Vec<_>) = store
            .take_pending()
            .into_iter()
            .partition(|event| self.still_true(event, workspace, sessions, missions));
        let dropped: Vec<SupervisorEventSummary> =
            dropped.into_iter().map(|event| event.summary).collect();
        let permission = supervisor
            .config
            .as_ref()
            .map_or(PermissionMode::Full, |config| config.permission_mode);
        let recent = store
            .wake_times()
            .into_iter()
            .filter(|at| now - at < HOUR_MS)
            .count();
        match plan {
            Flush::Merge(row) => {
                let Some((_, earlier)) = held else {
                    store.requeue(keep);
                    return Some(now);
                };
                if keep.is_empty() {
                    store.merge(&row, Vec::new(), dropped);
                    return None;
                }
                let events: Vec<&SupervisorEvent> = earlier.iter().chain(&keep).collect();
                let header = WakeHeader {
                    number: recent,
                    since: earliest(&events, now),
                    held_by_cap_min: None,
                };
                let text =
                    self.render_wake(workspace, header, &events, &dropped, settings, permission);
                if self.rewrite_queued(&supervisor.id, &row, text).await {
                    store.merge(&row, keep, dropped);
                    None
                } else {
                    store.requeue(keep);
                    Some(now)
                }
            }
            _ => {
                if keep.is_empty() {
                    if !dropped.is_empty() {
                        let wake = SupervisorWake {
                            id: new_id(),
                            at: utc(now),
                            events: Vec::new(),
                            dropped,
                            delivery: WakeDelivery::Dropped,
                        };
                        store.record(wake, Vec::new());
                    }
                    return None;
                }
                let events: Vec<&SupervisorEvent> = keep.iter().collect();
                let since = earliest(&events, now);
                let held = now - since - fold_ms();
                let header = WakeHeader {
                    number: recent + 1,
                    since,
                    held_by_cap_min: (held > 60_000).then_some(held / 60_000),
                };
                let text =
                    self.render_wake(workspace, header, &events, &dropped, settings, permission);
                let busy = sessions.turn_in_flight(&supervisor.id);
                let (id, delivery) =
                    match self.queue_thread_message(&supervisor.id, None, &text).await {
                        Ok(id) if self.queued_row(&supervisor.id, &id).is_some() => {
                            (id, WakeDelivery::HeldForTurnEnd)
                        }
                        Ok(id) if busy => (id, WakeDelivery::Steered),
                        Ok(id) => (id, WakeDelivery::Started),
                        Err(err) => (
                            new_id(),
                            WakeDelivery::Failed {
                                error: err.to_string(),
                            },
                        ),
                    };
                let wake = SupervisorWake {
                    id,
                    at: utc(now),
                    events: keep.iter().map(|event| event.summary.clone()).collect(),
                    dropped,
                    delivery,
                };
                store.record(wake, keep);
                None
            }
        }
    }

    fn still_true(
        &self,
        event: &SupervisorEvent,
        workspace: &WorkspaceHost,
        sessions: &SessionsEngine,
        missions: &HashMap<String, MissionFacts>,
    ) -> bool {
        let summary = &event.summary;
        let mission = summary.task_id.as_deref().and_then(|id| missions.get(id));
        match summary.kind {
            Kind::StepFailed => summary
                .step_id
                .as_deref()
                .and_then(|id| workspace.task_step(id))
                .is_some_and(|step| step_failed(&step)),
            Kind::NeedsYou => match (&summary.thread_id, &event.request_id) {
                (Some(thread), Some(request)) => sessions.awaits_input(thread, request),
                _ => false,
            },
            Kind::MissionSilent => {
                event.since.is_some() && mission.and_then(MissionFacts::quiet_since) == event.since
            }
            Kind::LeadSilent => mission.is_some_and(|mission| {
                mission.failed.iter().any(|failed| {
                    Some(&failed.step_id) == summary.step_id.as_ref()
                        && Some(failed.failed_at) == event.since
                        && mission.lead_silent_after(failed.failed_at)
                })
            }),
            Kind::MissionDone
            | Kind::PlanEdited
            | Kind::Escalation
            | Kind::Resource
            | Kind::Heartbeat => true,
        }
    }

    fn render_wake(
        &self,
        workspace: &WorkspaceHost,
        header: WakeHeader,
        events: &[&SupervisorEvent],
        dropped: &[SupervisorEventSummary],
        settings: zeron_proto::supervisor::SupervisorSettings,
        permission: PermissionMode,
    ) -> String {
        let entries: Vec<WakeEntry> = events
            .iter()
            .map(|event| {
                let task = event
                    .summary
                    .task_id
                    .as_deref()
                    .and_then(|id| workspace.task(id));
                let mission = task
                    .as_ref()
                    .map(|task| format!("{} (task {})", clip(&task.title, TITLE_CHARS), task.id));
                let lead = task.and_then(|task| task.chat_id);
                let source = event.summary.thread_id.clone().or(lead.clone());
                let reply = source.map(|thread| Reply {
                    from_lead: Some(&thread) == lead.as_ref(),
                    text: self.last_reply(&thread),
                });
                WakeEntry {
                    event,
                    mission,
                    reply,
                }
            })
            .collect();
        wake_text(header, &entries, dropped, settings, permission)
    }

    fn last_reply(&self, chat_id: &str) -> Option<String> {
        let entries = self.open(chat_id).ok()?.doc.read_entries().ok()?;
        entries
            .iter()
            .rev()
            .filter(|entry| entry.role == MessageRole::Assistant)
            .find_map(|entry| {
                entry.parts.iter().rev().find_map(|part| match part {
                    MessagePart::Text { text, .. } if !text.trim().is_empty() => {
                        Some(clip(text, WAKE_QUESTION_CHARS))
                    }
                    _ => None,
                })
            })
    }

    fn queued_row(&self, chat_id: &str, row: &str) -> Option<QueuedMessage> {
        self.open(chat_id)
            .ok()?
            .doc
            .read_queue()
            .ok()?
            .into_iter()
            .find(|queued| queued.id == row)
    }

    async fn rewrite_queued(&self, chat_id: &str, row_id: &str, text: String) -> bool {
        let Ok(handle) = self.open(chat_id) else {
            return false;
        };
        let _drain = handle.drain_lock.lock().await;
        let Some((index, mut row)) = handle.doc.read_queue().ok().and_then(|queue| {
            queue
                .into_iter()
                .enumerate()
                .find(|(_, row)| row.id == row_id && row.delivery_gate.is_none())
        }) else {
            return false;
        };
        row.text = text;
        let rewritten = handle
            .doc
            .take_queued(row_id)
            .and_then(|_| handle.doc.insert_queued(index, &row));
        if let Err(err) = rewritten {
            tracing::warn!(chat = %chat_id, error = %err, "folding events into a queued supervisor wake failed");
            return false;
        }
        handle.publish_queue();
        self.save_snapshot(&handle);
        true
    }
}

fn earliest(events: &[&SupervisorEvent], now: i64) -> i64 {
    events
        .iter()
        .map(|event| ms(event.summary.at))
        .min()
        .unwrap_or(now)
}

fn stall_event(stall: Stall, step_titles: &HashMap<String, String>) -> SupervisorEvent {
    let key = stall.key();
    match stall {
        Stall::MissionSilent { task_id, since } => {
            let mut event = SupervisorEvent::new(
                Kind::MissionSilent,
                key,
                Some(&task_id),
                "Mission silent 30 min".to_string(),
            );
            event.detail = vec![format!(
                "No thread of this mission has worked or written since {}, and nothing waits on the person.",
                utc(since).with_timezone(&chrono::Local).format("%H:%M")
            )];
            event.since = Some(since);
            event
        }
        Stall::LeadSilent {
            task_id,
            step_id,
            failed_at,
        } => {
            let title = step_titles.get(&step_id).map_or("", String::as_str);
            let mut event = SupervisorEvent::new(
                Kind::LeadSilent,
                key,
                Some(&task_id),
                format!(
                    "Lead silent 10 min after step \"{}\" failed",
                    clip(title, TITLE_CHARS)
                ),
            );
            event.summary.step_id = Some(step_id);
            event.since = Some(failed_at);
            event
        }
    }
}
