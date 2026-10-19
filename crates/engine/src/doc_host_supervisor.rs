use std::collections::{HashMap, HashSet};
use std::time::Duration;

use zeron_proto::supervisor::{
    InputAsker, InterventionKind, InterventionTrigger, NeedsYouInput, Raiser,
    SupervisorEventKind as Kind, SupervisorEventSummary, SupervisorIntervention, SupervisorItem,
    SupervisorState, SupervisorWake, WakeDelivery,
};
use zeron_proto::{
    Chat, PermissionMode, Session, SessionStatus, StepTurnEnd, Task, TaskMode, TaskStep,
    UserInputQuestion,
};

use super::delegation::{WAKE_QUESTION_CHARS, clip, question_lines};
use super::*;
use crate::registry::HarnessRegistry;
use crate::supervisor::{
    Flush, InterventionRecord, MissionFacts, Projections, Reply, Stall, StepSnap, StepThread,
    SupervisorEvent, SupervisorStore, ThreadFacts, WakeEntry, WakeHeader, chips, fold_ms,
    lead_questions, open_inputs, plan_flush, plan_stalls, utc, wake_text,
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
    fn sessions(&mut self, sessions: &[Session], now: i64) -> Vec<String> {
        let mut live_now = HashSet::new();
        for session in sessions.iter().filter(|session| live(session.status)) {
            live_now.insert(session.chat_id.clone());
            self.last_live.insert(session.chat_id.clone(), now);
        }
        let ended: Vec<String> = self.live.difference(&live_now).cloned().collect();
        for thread in &ended {
            self.last_live.insert(thread.clone(), now);
        }
        self.live = live_now;
        ended
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

/// What an intervention row needs from before the action: a re-brief's snapshot predates its own edit.
pub(crate) struct InterventionBasis {
    kind: InterventionKind,
    task_id: Option<String>,
    steps: Vec<StepSnap>,
    target: Option<String>,
    trigger: Option<InterventionTrigger>,
}

impl InterventionBasis {
    pub(crate) fn with_task(mut self, task_id: &str, target: &str) -> Self {
        self.task_id = Some(task_id.to_string());
        self.target = Some(target.to_string());
        self
    }
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

    pub(crate) fn supervisor_stale(&self) {
        if let Some(store) = self.supervisor() {
            store.mark_stale();
        }
    }

    pub(super) fn supervisor_step_ended(&self, step: &TaskStep, thread: &str, ended: StepTurnEnd) {
        let Some(store) = self.supervisor() else {
            return;
        };
        store.mark_stale();
        if ended != StepTurnEnd::Failed {
            return;
        }
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
        self.supervisor_stale();
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

    pub(crate) fn supervisor_plan_edited(
        &self,
        task: &Task,
        changes: &[String],
        actor: Option<&str>,
    ) {
        let mut event = SupervisorEvent::new(
            Kind::PlanEdited,
            format!("plan:{}:{}", task.id, new_id()),
            Some(&task.id),
            "Plan edited by the person".to_string(),
        );
        event.detail = changes.iter().map(|change| format!("- {change}")).collect();
        event.actor = actor.map(str::to_owned);
        self.supervisor_event(event);
    }

    pub(crate) fn supervisor_escalated(&self, item: &SupervisorItem) {
        let mut event = SupervisorEvent::new(
            Kind::Escalation,
            format!("escalation:{}", item.id),
            item.task_id.as_deref(),
            "Lead escalated".to_string(),
        );
        let why = serde_json::to_value(item.why)
            .ok()
            .and_then(|why| why.as_str().map(str::to_owned))
            .unwrap_or_default();
        event.detail = vec![format!(
            "itemId {} · why: {why} · \"{}\"",
            item.id,
            clip(&item.title, TITLE_CHARS)
        )];
        if !item.detail.trim().is_empty() {
            event
                .detail
                .push(clip(&item.detail, WAKE_QUESTION_CHARS).replace('\n', "\n   "));
        }
        if !item.options.is_empty() {
            event
                .detail
                .push(format!("Options: {}", item.options.join(" | ")));
        }
        event.detail.push(
            "The person answers it on the Supervisor page; their choice goes to the lead.".into(),
        );
        event.actor = (item.raised_by == Raiser::Supervisor).then(|| item.raiser_chat_id.clone());
        self.supervisor_event(event);
    }

    pub(crate) fn is_live_supervisor(&self, chat_id: &str) -> bool {
        self.supervisor_chat()
            .is_some_and(|supervisor| supervisor.id == chat_id)
    }

    fn mission_of_thread(
        &self,
        workspace: &WorkspaceHost,
        thread: &str,
    ) -> Option<(Task, Vec<TaskStep>)> {
        let tasks = workspace.watch_tasks().borrow().clone();
        if let Some(task) = tasks.iter().find(|task| {
            task.mode == TaskMode::Orchestrated
                && !task.deleted
                && task.completed_at.is_none()
                && task.chat_id.as_deref() == Some(thread)
        }) {
            let steps = workspace.task_steps_of(&task.id);
            return Some((task.clone(), steps));
        }
        let step = workspace
            .watch_task_steps()
            .borrow()
            .iter()
            .filter(|step| step.chat_id.as_deref() == Some(thread))
            .max_by_key(|step| step.outcome.is_none())
            .cloned()?;
        let task = workspace.task(&step.task_id)?;
        Some((task, vec![step]))
    }

    fn supervisor_trigger(&self, supervisor: &str) -> Option<InterventionTrigger> {
        let store = self.supervisor()?;
        let entries = self.open(supervisor).ok()?.doc.read_entries().ok()?;
        let last = entries
            .iter()
            .rev()
            .find(|entry| entry.role == MessageRole::User)?;
        if store.wake_ids().contains(&last.id) {
            Some(InterventionTrigger::Wake {
                wake_id: last.id.clone(),
            })
        } else {
            last.from_chat_id
                .is_none()
                .then_some(InterventionTrigger::Person)
        }
    }

    pub(crate) fn intervention_basis(
        &self,
        kind: InterventionKind,
        supervisor: &str,
        target: Option<&str>,
        step: Option<&str>,
    ) -> InterventionBasis {
        let workspace = self.workspace();
        let named = match (workspace, step, target) {
            (Some(workspace), Some(step), _) => workspace
                .task_step(step)
                .and_then(|step| workspace.task(&step.task_id).map(|task| (task, vec![step]))),
            (Some(workspace), None, Some(target)) => self.mission_of_thread(workspace, target),
            _ => None,
        };
        let (task_id, steps) = match named {
            Some((task, steps)) => (Some(task.id), steps.iter().map(StepSnap::from).collect()),
            None => (None, Vec::new()),
        };
        InterventionBasis {
            kind,
            task_id,
            steps,
            target: target.map(str::to_owned),
            trigger: self.supervisor_trigger(supervisor),
        }
    }

    pub(crate) fn record_intervention(
        &self,
        basis: InterventionBasis,
        message_id: Option<String>,
        request_id: Option<String>,
        text: &str,
    ) {
        let Some(store) = self.supervisor() else {
            return;
        };
        store.add_intervention(InterventionRecord {
            row: SupervisorIntervention {
                id: new_id(),
                at: chrono::Utc::now(),
                kind: basis.kind,
                task_id: basis.task_id,
                step_ids: basis.steps.iter().map(|step| step.id.clone()).collect(),
                target: basis.target,
                message_id,
                request_id,
                text: text.to_string(),
                trigger: basis.trigger,
                chips: Vec::new(),
            },
            steps: basis.steps,
        });
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
        let ended = observed.sessions(sessions, now);
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
            let threads = std::iter::once(lead.id.as_str())
                .chain(own.iter().filter_map(|step| step.chat_id.as_deref()));
            if threads
                .into_iter()
                .any(|thread| ended.contains(&thread.to_string()))
            {
                store.mark_stale();
            }
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
        if store
            .chat_id()
            .is_some_and(|supervisor| ended.contains(&supervisor))
        {
            store.mark_stale();
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
        let supervisor = self.supervisor_chat();
        if store.take_refresh(now) {
            store.set_projections(self.projections(
                store,
                workspace,
                sessions,
                supervisor.as_ref().map(|chat| chat.id.as_str()),
            ));
        }
        let refresh = store.refresh_at();
        let Some(supervisor) = supervisor else {
            store.take_pending();
            store.publish(None, SupervisorState::Off, 0);
            return refresh;
        };
        let missions: Vec<MissionFacts> = round.missions.values().cloned().collect();
        let (stalls, mut next) = plan_stalls(now, &missions, |key| store.reported(key));
        if let Some(at) = refresh {
            next = Some(next.map_or(at, |n| n.min(at)));
        }
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
        let (state, waiting) = match sessions.session_status(&supervisor.id) {
            Some(session) if live(session.status) => (SupervisorState::Awake, 0),
            _ if self.queue_paused(&supervisor.id) => (
                SupervisorState::Stopped,
                self.waiting_wakes(store, &supervisor.id),
            ),
            _ => (SupervisorState::Asleep, 0),
        };
        store.publish(Some(supervisor.id), state, waiting);
        next
    }

    fn queue_paused(&self, chat_id: &str) -> bool {
        self.open(chat_id)
            .is_ok_and(|handle| handle.queue_paused.load(Ordering::Acquire))
    }

    fn waiting_wakes(&self, store: &SupervisorStore, chat_id: &str) -> usize {
        let wakes = store.wake_ids();
        self.open(chat_id)
            .ok()
            .and_then(|handle| handle.doc.read_queue().ok())
            .map_or(0, |queue| {
                queue.iter().filter(|row| wakes.contains(&row.id)).count()
            })
    }

    fn entries_of(&self, chat_id: &str) -> Vec<zeron_doc::SessionMessageEntry> {
        self.open(chat_id)
            .ok()
            .and_then(|handle| handle.doc.read_entries().ok())
            .unwrap_or_default()
    }

    fn projections(
        &self,
        store: &SupervisorStore,
        workspace: &WorkspaceHost,
        sessions: &SessionsEngine,
        supervisor: Option<&str>,
    ) -> Projections {
        let interventions = store.interventions();
        let mut transcripts: HashMap<String, Vec<zeron_doc::SessionMessageEntry>> = HashMap::new();
        let mut read = |chat_id: &str| {
            transcripts
                .entry(chat_id.to_string())
                .or_insert_with(|| self.entries_of(chat_id))
                .clone()
        };
        let mut projections = Projections::default();
        let pending = |chat_id: &str,
                       entries: &[zeron_doc::SessionMessageEntry],
                       asker: InputAsker,
                       task_id: Option<&str>,
                       step_id: Option<&str>| {
            open_inputs(entries)
                .into_iter()
                .filter(|(request_id, questions, _)| {
                    (asker != InputAsker::Step || questions.iter().any(|q| q.approval))
                        && sessions.awaits_input(chat_id, request_id)
                })
                .map(|(request_id, questions, at)| NeedsYouInput {
                    chat_id: chat_id.to_string(),
                    request_id,
                    asker,
                    task_id: task_id.map(str::to_owned),
                    step_id: step_id.map(str::to_owned),
                    at: utc(at),
                    questions,
                })
                .collect::<Vec<_>>()
        };
        let tasks = workspace.watch_tasks().borrow().clone();
        let steps = workspace.watch_task_steps().borrow().clone();
        for task in tasks.iter().filter(|task| {
            task.completed_at.is_none()
                && !store.is_unwatched(&task.id)
                && self.hosts_mission(workspace, &task.id)
        }) {
            let Some(lead) = task.chat_id.as_deref() else {
                continue;
            };
            let lead_entries = read(lead);
            projections.inputs.extend(pending(
                lead,
                &lead_entries,
                InputAsker::Lead,
                Some(&task.id),
                None,
            ));
            for step in steps.iter().filter(|step| step.task_id == task.id) {
                let Some(thread) = step.chat_id.as_deref() else {
                    continue;
                };
                let entries = read(thread);
                projections.inputs.extend(pending(
                    thread,
                    &entries,
                    InputAsker::Step,
                    Some(&task.id),
                    Some(&step.id),
                ));
                let at = StepThread {
                    task_id: &task.id,
                    step_id: &step.id,
                    chat_id: thread,
                };
                projections.questions.extend(lead_questions(
                    &at,
                    &entries,
                    &lead_entries,
                    &interventions,
                ));
            }
        }
        if let Some(supervisor) = supervisor {
            let entries = read(supervisor);
            projections.inputs.extend(pending(
                supervisor,
                &entries,
                InputAsker::Supervisor,
                None,
                None,
            ));
        }
        projections
            .inputs
            .sort_by_key(|input| std::cmp::Reverse(input.at));
        projections
            .questions
            .sort_by_key(|question| std::cmp::Reverse(question.at));
        for record in &interventions {
            let task = record
                .row
                .task_id
                .as_deref()
                .and_then(|id| workspace.task(id));
            let plan = task
                .as_ref()
                .map(|task| workspace.task_steps_of(&task.id))
                .unwrap_or_default();
            let lead = task.and_then(|task| task.chat_id);
            let target = record
                .row
                .target
                .clone()
                .filter(|target| lead.as_ref() == Some(target))
                .map(|target| read(&target));
            projections.chips.insert(
                record.row.id.clone(),
                chips(record, &plan, target.as_deref()),
            );
        }
        projections
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
                let delivered = if self.queue_paused(&supervisor.id) {
                    self.push_thread_row(&supervisor.id, None, &text, Vec::new())
                        .map(|(_, id)| (id, true))
                } else {
                    self.queue_thread_message(&supervisor.id, None, &text)
                        .await
                        .map(|id| (id, false))
                };
                let (id, delivery) = match delivered {
                    Ok((id, true)) => (id, WakeDelivery::HeldByStop),
                    Ok((id, _)) if self.queued_row(&supervisor.id, &id).is_some() => {
                        (id, WakeDelivery::HeldForTurnEnd)
                    }
                    Ok((id, _)) if busy => (id, WakeDelivery::Steered),
                    Ok((id, _)) => (id, WakeDelivery::Started),
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
            Kind::Escalation => summary
                .key
                .strip_prefix("escalation:")
                .and_then(|item| self.supervisor().map(|store| store.item_open(item)))
                .unwrap_or(false),
            Kind::MissionDone | Kind::PlanEdited | Kind::Resource | Kind::Heartbeat => true,
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
                    text: self
                        .last_reply(&thread)
                        .map(|text| clip(&text, WAKE_QUESTION_CHARS)),
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

    pub(super) fn last_reply(&self, chat_id: &str) -> Option<String> {
        let entries = self.open(chat_id).ok()?.doc.read_entries().ok()?;
        entries
            .iter()
            .rev()
            .filter(|entry| entry.role == MessageRole::Assistant)
            .find_map(|entry| {
                entry.parts.iter().rev().find_map(|part| match part {
                    MessagePart::Text { text, .. } if !text.trim().is_empty() => {
                        Some(text.trim().to_string())
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
