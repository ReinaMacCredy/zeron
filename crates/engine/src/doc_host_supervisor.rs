use std::collections::{HashMap, HashSet};
use std::time::Duration;

use zeron_proto::supervisor::{
    InputAsker, InterventionKind, InterventionTrigger, NeedsYouInput, NextCheck, NextCheckKind,
    PlanApproval, Raiser, SupervisorEventKind as Kind, SupervisorEventSummary,
    SupervisorIntervention, SupervisorItem, SupervisorState, SupervisorWake, WakeDelivery,
};
use zeron_proto::{
    Chat, DelegationStatus, PermissionMode, Session, SessionStatus, StepTurnEnd, Task, TaskPatch,
    TaskStep, UserInputQuestion,
};

use super::delegation::{WAKE_QUESTION_CHARS, clip, question_lines};
use super::*;
use crate::registry::HarnessRegistry;
use crate::supervisor::{
    Flush, InterventionRecord, MISSION_SILENT_MS, MissionFacts, Projections, Reply, Stall,
    StepSnap, StepThread, SupervisorEvent, SupervisorStore, ThreadFacts, WakeEntry, WakeHeader,
    chips, escalated_item, fold_ms, lead_questions, open_inputs, plan_flush, plan_stalls, utc,
    wake_text,
};
use crate::supervisor_resources::{
    CheckOutcome, Contention, PAGE_CHECK_MS, Probe, ResourceMission, find_candidates, run_check,
};
use zeron_proto::supervisor::ResourceKey;

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
    delegations: Option<HashMap<String, DelegationStatus>>,
}

impl Observed {
    fn sessions(&mut self, sessions: &[Session], now: i64) -> (Vec<String>, HashSet<String>) {
        let mut live_now = HashSet::new();
        for session in sessions.iter().filter(|session| live(session.status)) {
            live_now.insert(session.chat_id.clone());
            self.last_live.insert(session.chat_id.clone(), now);
        }
        let ended: Vec<String> = self.live.difference(&live_now).cloned().collect();
        for thread in &ended {
            self.last_live.insert(thread.clone(), now);
        }
        let edges = self.live.symmetric_difference(&live_now).cloned().collect();
        self.live = live_now;
        (ended, edges)
    }

    fn finished(&mut self, chats: &[Chat]) -> Vec<String> {
        let now: HashMap<String, DelegationStatus> = chats
            .iter()
            .filter_map(|chat| Some((chat.id.clone(), chat.delegation.as_ref()?.status)))
            .collect();
        let finished = match &self.delegations {
            Some(before) => now
                .iter()
                .filter(|(id, status)| {
                    **status != DelegationStatus::Running
                        && before.get(*id) == Some(&DelegationStatus::Running)
                })
                .map(|(id, _)| id.clone())
                .collect(),
            None => Vec::new(),
        };
        self.delegations = Some(now);
        finished
    }

    fn missions_done(&mut self, tasks: &[Task]) -> Vec<Task> {
        let started = |task: &&Task| task.has_plan() && !task.deleted;
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

pub(crate) struct Tree {
    pub root: String,
    root_hosted: bool,
    pub task_id: Option<String>,
}

fn open_step_of<'a>(steps: &'a [TaskStep], chat_id: &str) -> Option<&'a TaskStep> {
    steps.iter().find(|step| {
        step.outcome.is_none() && !step.deleted && step.chat_id.as_deref() == Some(chat_id)
    })
}

fn tree_of(chat: &Chat, chats: &[Chat], tasks: &[Task], steps: &[TaskStep], device: &str) -> Tree {
    let path = zeron_proto::tree_path(chat, chats, tasks, steps);
    let task_id = path.iter().find_map(|node| {
        let stepped = open_step_of(steps, &node.id)
            .and_then(|step| tasks.iter().find(|task| task.id == step.task_id))
            .filter(|task| !task.deleted && task.completed_at.is_none() && task.has_plan());
        stepped
            .or_else(|| zeron_proto::plan_of_chat(tasks, &node.id))
            .map(|task| task.id.clone())
    });
    let root = path.last().copied().unwrap_or(chat);
    Tree {
        root: root.id.clone(),
        root_hosted: root.device_id == device && !root.archived && !root.is_system(),
        task_id,
    }
}

struct PendingPlan {
    task_id: String,
    lead: String,
    steps: usize,
    newest: i64,
}

struct Round {
    missions: HashMap<String, MissionFacts>,
    nodes: Vec<SupervisorEvent>,
    plans: Vec<PendingPlan>,
    done: Vec<Task>,
    step_titles: HashMap<String, String>,
    resources: Vec<ResourceMission>,
    edges: HashSet<String>,
}

/// What an intervention row needs from before the action: a re-brief's snapshot predates its own edit.
pub(crate) struct InterventionBasis {
    id: String,
    kind: InterventionKind,
    task_id: Option<String>,
    steps: Vec<StepSnap>,
    target: Option<String>,
    trigger: Option<InterventionTrigger>,
    item_id: Option<String>,
    approval: Option<PlanApproval>,
}

impl InterventionBasis {
    pub(crate) fn id(&self) -> &str {
        &self.id
    }

    pub(crate) fn with_task(mut self, task_id: &str, target: &str) -> Self {
        self.task_id = Some(task_id.to_string());
        self.target = Some(target.to_string());
        self
    }

    pub(crate) fn with_item(mut self, item_id: &str) -> Self {
        self.item_id = Some(item_id.to_string());
        self
    }

    pub(crate) fn with_approval(mut self, approval: PlanApproval) -> Self {
        self.approval = Some(approval);
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
        let hosted = match (&event.summary.task_id, &event.summary.root_chat_id) {
            (Some(task_id), _) => self.hosts_mission(workspace, task_id),
            (None, Some(root)) => workspace.chat(root).ok().flatten().is_some_and(|root| {
                root.device_id == self.device_id() && !root.archived && !root.is_system()
            }),
            (None, None) => true,
        };
        if hosted {
            store.offer(event);
        }
    }

    /// Reads the rows, not the watches: a call can follow its rows' write.
    pub(crate) fn tree_of_chat(&self, chat: &Chat) -> Result<Tree, EngineError> {
        let workspace = self
            .workspace()
            .ok_or_else(|| EngineError::Other("workspace not wired".into()))?;
        let (chats, tasks, steps) = (
            workspace.read_chats()?,
            workspace.read_tasks()?,
            workspace.read_task_steps()?,
        );
        Ok(tree_of(chat, &chats, &tasks, &steps, self.device_id()))
    }

    pub(crate) fn tree_watched(&self, store: &SupervisorStore, tree: &Tree) -> bool {
        match (&tree.task_id, self.workspace()) {
            (Some(task_id), Some(workspace)) => {
                !store.is_unwatched(task_id) && self.hosts_mission(workspace, task_id)
            }
            (Some(_), None) => false,
            (None, _) => tree.root_hosted && store.watches_tree(&tree.root),
        }
    }

    fn hosts_mission(&self, workspace: &WorkspaceHost, task_id: &str) -> bool {
        workspace
            .task(task_id)
            .filter(|task| task.has_plan() && !task.deleted)
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
        if asker.is_system() {
            return;
        }
        let chats = workspace.watch_chats().borrow().clone();
        let tasks = workspace.watch_tasks().borrow().clone();
        let steps = workspace.watch_task_steps().borrow().clone();
        let step = open_step_of(&steps, &asker.id).cloned();
        let tree = tree_of(asker, &chats, &tasks, &steps, self.device_id());
        let led = zeron_proto::plan_of_chat(&tasks, &asker.id).map(|task| task.id.clone());
        let (who, task_id) = match (&step, led) {
            (Some(step), _) => (
                format!("Step \"{}\"", clip(&step.title, TITLE_CHARS)),
                Some(step.task_id.clone()),
            ),
            (None, Some(task_id)) => ("The lead".to_string(), Some(task_id)),
            (None, None) => (thread_label(asker), tree.task_id.clone()),
        };
        let what = if questions.iter().any(|q| q.approval) {
            "waits for a tool approval"
        } else {
            "asks a question"
        };
        let mut event = SupervisorEvent::new(
            Kind::NeedsYou,
            format!("input:{}:{request_id}", asker.id),
            task_id.as_deref(),
            format!("{who} {what}"),
        );
        event.summary.step_id = step.map(|step| step.id);
        event.summary.thread_id = Some(asker.id.clone());
        event.summary.root_chat_id = Some(tree.root);
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

    /// A re-wake gets its own key: the first wake of the item is already reported.
    pub(crate) fn supervisor_escalated(&self, item: &SupervisorItem, rewake: bool) {
        let key = match rewake {
            true => format!("escalation:{}:{}", item.id, now_ms()),
            false => format!("escalation:{}", item.id),
        };
        let mut event = SupervisorEvent::new(
            Kind::Escalation,
            key,
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
        if let Some(task) = zeron_proto::plan_of_chat(&tasks, thread) {
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
            id: new_id(),
            kind,
            task_id,
            steps,
            target: target.map(str::to_owned),
            trigger: self.supervisor_trigger(supervisor),
            item_id: None,
            approval: None,
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
                id: basis.id,
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
                item: None,
                approval: basis.approval,
            },
            steps: basis.steps,
            item_id: basis.item_id,
        });
    }

    /// A failed send marks the task, so its card says the thread missed the message.
    pub(crate) async fn send_to_task_thread(
        &self,
        workspace: &WorkspaceHost,
        task: &Task,
        chat_id: &str,
        from_chat_id: Option<&str>,
        text: &str,
        images: Vec<String>,
    ) -> Result<String, EngineError> {
        let sent = self
            .queue_thread_message_with(chat_id, from_chat_id, text, images)
            .await;
        let send_error = match &sent {
            Ok(id) if task.send_error.is_none() => return Ok(id.clone()),
            Ok(_) => None,
            Err(error) => Some(error.to_string()),
        };
        let patch = TaskPatch {
            send_error: Some(send_error),
            ..Default::default()
        };
        workspace.update_task(&task.id, &patch)?;
        sent
    }

    /// Sends each held Supervisor approval whose hold ran out; returns when the next one does.
    async fn release_approvals(
        &self,
        store: &SupervisorStore,
        workspace: &WorkspaceHost,
        now: i64,
    ) -> Option<i64> {
        let mut next = None;
        for (id, task_id, due, reason) in store.held_approvals() {
            if due > now {
                next = Some(next.map_or(due, |next: i64| next.min(due)));
                continue;
            }
            let approving = store.approving();
            let _approving = approving.lock().await;
            let still_held = matches!(
                store.approval_of(&task_id),
                Some((held, PlanApproval::Held { .. })) if held == id
            );
            if !still_held {
                continue;
            }
            let task = workspace
                .task(&task_id)
                .filter(|task| !task.deleted && task.completed_at.is_none() && !task.plan_pending);
            let lead = task.as_ref().and_then(|task| task.chat_id.clone());
            let (Some(task), Some(lead)) = (task, lead) else {
                store.set_approval(&id, None, None);
                continue;
            };
            let text = format!(
                "The Supervisor approved the plan; the person gave it this decision. Run it.\n\nWhy: {}",
                reason.trim()
            );
            match self
                .send_to_task_thread(workspace, &task, &lead, None, &text, Vec::new())
                .await
            {
                Ok(message_id) => {
                    store.set_approval(&id, Some(PlanApproval::Sent), Some(message_id))
                }
                Err(err) => {
                    tracing::warn!(error = %err, task = %task_id, "sending a held plan approval failed");
                    store.set_approval(&id, Some(PlanApproval::Sent), None);
                }
            }
        }
        next
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
        let (ended, edges) = observed.sessions(sessions, now);
        let done = observed.missions_done(tasks);
        let nodes = self.node_events(observed, sessions, &ended, chats, tasks, steps);
        if !ended.is_empty() && store.watches_any_tree() {
            store.mark_stale();
        }
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
        let mut plans = Vec::new();
        let mut step_titles = HashMap::new();
        let mut resources = Vec::new();
        for task in tasks {
            if !task.has_plan()
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
            if task.plan_pending && !own.is_empty() {
                plans.push(PendingPlan {
                    task_id: task.id.clone(),
                    lead: lead.id.clone(),
                    steps: own.len(),
                    newest: own
                        .iter()
                        .map(|step| ms(step.created_at))
                        .max()
                        .unwrap_or(0),
                });
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
            let threads: Vec<&Chat> = std::iter::once(*lead)
                .chain(
                    steps
                        .iter()
                        .filter(|step| step.task_id == task.id)
                        .filter_map(|step| step.chat_id.as_deref())
                        .filter_map(|id| chats.get(id).copied()),
                )
                .collect();
            let mut cwds: Vec<String> = threads.iter().filter_map(|t| t.cwd.clone()).collect();
            cwds.sort();
            cwds.dedup();
            let mut checkouts: Vec<String> = threads
                .iter()
                .filter_map(|t| t.checkout_id.clone().or_else(|| t.cwd.clone()))
                .collect();
            checkouts.sort();
            checkouts.dedup();
            let held = |facts: &ThreadFacts| {
                facts.live
                    || facts.background
                    || facts
                        .last_active()
                        .is_some_and(|at| now - at < MISSION_SILENT_MS)
            };
            resources.push(ResourceMission {
                task_id: task.id.clone(),
                keys: task
                    .resources
                    .iter()
                    .filter_map(|key| ResourceKey::parse(key))
                    .collect(),
                active: std::iter::once(&facts.lead).chain(&facts.steps).any(held),
                threads: threads.iter().map(|t| t.id.clone()).collect(),
                cwds,
                checkouts,
            });
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
            nodes,
            plans,
            done,
            step_titles,
            resources,
            edges,
        }
    }

    fn node_events(
        &self,
        observed: &mut Observed,
        sessions: &[Session],
        ended: &[String],
        chats: &[Chat],
        tasks: &[Task],
        steps: &[TaskStep],
    ) -> Vec<SupervisorEvent> {
        let finished = observed.finished(chats);
        let errored: HashSet<&str> = sessions
            .iter()
            .filter(|session| session.status == SessionStatus::Errored)
            .map(|session| session.chat_id.as_str())
            .collect();
        let failed = ended
            .iter()
            .filter(|thread| errored.contains(thread.as_str()))
            .map(|thread| (thread, Kind::NodeFailed));
        let engine = self.sessions();
        let busy = |thread: &str| {
            sessions
                .iter()
                .find(|session| session.chat_id == thread)
                .is_some_and(|session| session.running_subagents > 0 || session.held_by_subagents)
                || engine
                    .as_ref()
                    .is_some_and(|engine| engine.holds_background_work(thread))
        };
        let mut events = Vec::new();
        for (thread, kind) in
            failed.chain(finished.iter().map(|thread| (thread, Kind::NodeFinished)))
        {
            let Some(chat) = chats.iter().find(|chat| &chat.id == thread) else {
                continue;
            };
            if chat.is_system() || open_step_of(steps, thread).is_some() || busy(thread) {
                continue;
            }
            let tree = tree_of(chat, chats, tasks, steps, self.device_id());
            let label = match (kind, chat.delegation.as_ref()) {
                (Kind::NodeFinished, Some(delegation)) => format!(
                    "Delegated task \"{}\" {}",
                    clip(chat.title.as_deref().unwrap_or("Untitled"), TITLE_CHARS),
                    match delegation.status {
                        DelegationStatus::Completed => "completed",
                        DelegationStatus::Failed => "failed",
                        DelegationStatus::Cancelled => "was cancelled",
                        DelegationStatus::Interrupted => "was interrupted",
                        DelegationStatus::Running => "ended",
                    }
                ),
                (Kind::NodeFinished, None) => continue,
                _ => format!("{} failed", thread_label(chat)),
            };
            let at = chat
                .delegation
                .as_ref()
                .and_then(|delegation| delegation.completed_at)
                .map_or_else(now_ms, ms);
            let key = match kind {
                Kind::NodeFinished => format!("node-done:{thread}:{at}"),
                _ => format!("node-failed:{thread}:{}", now_ms()),
            };
            let mut event = SupervisorEvent::new(kind, key, tree.task_id.as_deref(), label);
            event.summary.thread_id = Some(thread.clone());
            event.summary.root_chat_id = Some(tree.root);
            events.push(event);
        }
        events
    }

    #[allow(clippy::too_many_arguments)]
    async fn supervise(
        &self,
        store: &SupervisorStore,
        workspace: &WorkspaceHost,
        sessions: &SessionsEngine,
        registry: &HarnessRegistry,
        mut round: Round,
        now: i64,
        started: i64,
    ) -> Option<i64> {
        for event in std::mem::take(&mut round.nodes) {
            self.supervisor_event(event);
        }
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
        let released = self.release_approvals(store, workspace, now).await;
        let refresh = [store.refresh_at(), released].into_iter().flatten().min();
        let Some(supervisor) = supervisor else {
            store.take_pending();
            store.set_next_check(None);
            store.publish(None, SupervisorState::Off, 0);
            return refresh;
        };
        let missions: Vec<MissionFacts> = round.missions.values().cloned().collect();
        let (stalls, stall_next) = plan_stalls(now, &missions, |key| store.reported(key));
        let mut check = stall_next.map(|(at, stall)| stall.check(at));
        let mut sooner = |at: i64, kind: NextCheckKind| {
            if check.as_ref().is_none_or(|c| at < c.at.timestamp_millis()) {
                check = Some(NextCheck {
                    at: utc(at),
                    kind,
                    task_id: None,
                });
            }
        };
        for stall in stalls {
            self.supervisor_event(stall_event(stall, &round.step_titles));
        }
        let settings = registry.supervisor_settings();
        if settings.decide_for_me {
            for plan in &round.plans {
                let taken_back = matches!(
                    store.approval_of(&plan.task_id),
                    Some((_, PlanApproval::TakenBack { .. }))
                );
                if !taken_back {
                    self.supervisor_event(plan_event(plan));
                }
            }
        }
        let missions_held = std::mem::take(&mut round.resources);
        if let Some(at) = self.supervise_resources(store, missions_held, &round.edges, now) {
            sooner(at, NextCheckKind::Resource);
        }
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
                sooner(due, NextCheckKind::Heartbeat);
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
            sooner(until, NextCheckKind::HeldWake);
        }
        let (state, waiting) = match sessions.session_status(&supervisor.id) {
            Some(session) if live(session.status) => (SupervisorState::Awake, 0),
            _ if self.queue_paused(&supervisor.id) => (
                SupervisorState::Stopped,
                self.waiting_wakes(store, &supervisor.id),
            ),
            _ => (SupervisorState::Asleep, 0),
        };
        let next = [refresh, check.as_ref().map(|c| c.at.timestamp_millis())]
            .into_iter()
            .flatten()
            .min();
        store.set_next_check(check);
        store.publish(Some(supervisor.id), state, waiting);
        next
    }

    fn supervise_resources(
        &self,
        store: &SupervisorStore,
        missions: Vec<ResourceMission>,
        edges: &HashSet<String>,
        now: i64,
    ) -> Option<i64> {
        let candidates = if cfg!(unix) {
            find_candidates(&missions)
        } else {
            Vec::new()
        };
        let (next, wanted) = store.resources(|resources| {
            let next = resources.observe(missions, candidates, edges, now);
            (next, resources.wanted())
        });
        if wanted {
            self.spawn_resource_check(store);
        }
        next
    }

    fn hold_probes(&self, store: &SupervisorStore) -> Vec<Probe> {
        store
            .holds()
            .into_iter()
            .filter(|hold| hold.resumed_at.is_none() && hold.freed_at.is_none())
            .filter_map(|hold| {
                Some(Probe {
                    key: ResourceKey::parse(&hold.resource)?,
                    row_id: hold.row_id,
                    pids: hold.pids,
                })
            })
            .collect()
    }

    fn spawn_resource_check(&self, store: &SupervisorStore) {
        let Ok(checking) = store.checking().try_lock_owned() else {
            return;
        };
        let probes = self.hold_probes(store);
        let Some(input) = store.resources(|resources| resources.take_input(probes)) else {
            return;
        };
        let (host, store) = (self.clone(), store.clone());
        self.spawn_worker(async move {
            let outcome = run_check(input).await;
            host.apply_resource_check(&store, outcome);
            drop(checking);
            store.poke();
        });
    }

    pub(crate) async fn check_resources_now(&self) {
        let Some(store) = self.supervisor().cloned() else {
            return;
        };
        let checking = store.checking().lock_owned().await;
        let probes = self.hold_probes(&store);
        if let Some(input) = store.resources(|resources| resources.take_input(probes)) {
            let outcome = run_check(input).await;
            self.apply_resource_check(&store, outcome);
        }
        drop(checking);
        store.poke();
    }

    pub(crate) fn request_resource_check(&self) {
        if let Some(store) = self.supervisor() {
            store.resources(|resources| resources.request_if_older(now_ms(), PAGE_CHECK_MS));
            store.poke();
        }
    }

    fn apply_resource_check(&self, store: &SupervisorStore, outcome: CheckOutcome) {
        let opened = store.resources(|resources| resources.apply(outcome.verdicts, now_ms()));
        if !outcome.freed.is_empty()
            && let Err(err) = store.mark_freed(&outcome.freed)
        {
            tracing::warn!(error = %err, "recording a freed resource failed");
        }
        store.mark_stale();
        for (key, contention) in opened {
            self.supervisor_event(self.contention_event(&key, &contention));
        }
    }

    fn contention_event(&self, key: &ResourceKey, contention: &Contention) -> SupervisorEvent {
        let workspace = self.workspace();
        let mission = |id: &str| {
            workspace
                .and_then(|workspace| workspace.task(id))
                .map_or_else(
                    || format!("task {id}"),
                    |task| format!("{} (task {})", clip(&task.title, TITLE_CHARS), task.id),
                )
        };
        let mut event = SupervisorEvent::new(
            Kind::Resource,
            format!("res:{key}:{}", contention.since),
            contention.blocked.as_deref(),
            format!("Resource contention on {key}"),
        );
        event.since = Some(contention.since);
        let involved: Vec<String> = contention.missions.iter().map(|id| mission(id)).collect();
        event
            .detail
            .push(format!("Missions: {}.", involved.join("; ")));
        let at = contention
            .check
            .at
            .with_timezone(&chrono::Local)
            .format("%H:%M:%S");
        match key {
            ResourceKey::Port(port) => {
                for used in &contention.check.uses {
                    let owner = used
                        .task_id
                        .as_deref()
                        .map(|id| format!(", which belongs to {}", mission(id)))
                        .unwrap_or_default();
                    event.detail.push(format!(
                        "Verified by lsof {at}: port {port} is open by {} (pid {}) in {}{owner}.",
                        used.command,
                        used.pid,
                        used.cwd.as_deref().unwrap_or("an unknown folder"),
                    ));
                }
            }
            _ => {
                for load in &contention.loads {
                    let who = match load.owners.as_slice() {
                        [one] => mission(one),
                        _ => format!(
                            "a process in {}",
                            load.cwd
                                .as_deref()
                                .unwrap_or("a folder these missions share")
                        ),
                    };
                    event.detail.push(format!(
                        "Verified by ps {at}, two samples 10 s apart: {who} used {:.0}% of a core in both.",
                        load.cpu
                    ));
                }
            }
        }
        if let Some(workspace) = workspace {
            for id in &contention.missions {
                for step in workspace
                    .task_steps_of(id)
                    .into_iter()
                    .filter(|step| step.outcome.is_none())
                {
                    if let Some(thread) = &step.chat_id {
                        event.detail.push(format!(
                            "{}: step \"{}\" (taskStepId {}) on thread {thread}.",
                            mission(id),
                            clip(&step.title, TITLE_CHARS),
                            step.id
                        ));
                    }
                }
            }
        }
        event.detail.push(format!(
            "To stop the step that blocks it, use the supervisor tool, action \"pause\", with its stepId, resource \"{key}\" and a reason; \"resume\" it once the resource is free."
        ));
        event
    }

    pub(crate) fn stop_for_pause(&self, thread: &str) -> Result<bool, EngineError> {
        let Some(sessions) = self.sessions() else {
            return Ok(false);
        };
        if !sessions.turn_in_flight(thread) && !sessions.holds_background_work(thread) {
            return Ok(false);
        }
        self.queue_command_with_transfers(
            thread,
            SessionCommandPayload::Interrupt { teardown: true },
            Vec::new(),
        )?;
        Ok(true)
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
        let holds = store.holds();
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
        let mut covered: HashSet<String> = supervisor.iter().map(|id| id.to_string()).collect();
        for task in tasks.iter().filter(|task| {
            task.completed_at.is_none()
                && !store.is_unwatched(&task.id)
                && self.hosts_mission(workspace, &task.id)
        }) {
            let Some(lead) = task.chat_id.as_deref() else {
                continue;
            };
            covered.insert(lead.to_string());
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
                covered.insert(thread.to_string());
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
        let chats = workspace.watch_chats().borrow().clone();
        for thread in sessions.chats_awaiting_input() {
            if covered.contains(&thread) {
                continue;
            }
            let Some(chat) = chats.iter().find(|chat| chat.id == thread) else {
                continue;
            };
            if chat.is_system() || chat.archived || chat.device_id != self.device_id() {
                continue;
            }
            let tree = tree_of(chat, &chats, &tasks, &steps, self.device_id());
            if !self.tree_watched(store, &tree) {
                continue;
            }
            let entries = read(&thread);
            projections.inputs.extend(pending(
                &thread,
                &entries,
                InputAsker::Thread,
                tree.task_id.as_deref(),
                None,
            ));
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
            let hold = holds
                .iter()
                .find(|hold| hold.row_id == record.row.id)
                .map(|hold| {
                    let running = hold
                        .thread_id
                        .as_deref()
                        .and_then(|thread| sessions.session_status(thread))
                        .is_some_and(|session| live(session.status));
                    (hold, running)
                });
            projections.chips.insert(
                record.row.id.clone(),
                chips(record, &plan, target.as_deref(), hold),
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
            .partition(|event| self.still_true(event, workspace, sessions, missions, settings));
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
                let paused = self.queue_paused(&supervisor.id);
                let own = Some(supervisor.id.as_str());
                let delivered = match self.push_thread_row(
                    &supervisor.id,
                    own,
                    &text,
                    Vec::new(),
                    Vec::new(),
                ) {
                    Ok((handle, id)) => {
                        if !paused {
                            self.send_thread_row(&handle, &id).await;
                        }
                        Ok((id, paused))
                    }
                    Err(err) => Err(err),
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
        settings: zeron_proto::supervisor::SupervisorSettings,
    ) -> bool {
        let summary = &event.summary;
        let mission = summary.task_id.as_deref().and_then(|id| missions.get(id));
        match summary.kind {
            Kind::StepFailed => summary
                .step_id
                .as_deref()
                .and_then(|id| workspace.task_step(id))
                .is_some_and(|step| step_failed(&step)),
            Kind::NeedsYou if summary.key.starts_with(PLAN_READY) => {
                settings.decide_for_me
                    && summary
                        .task_id
                        .as_deref()
                        .and_then(|id| workspace.task(id))
                        .is_some_and(|task| {
                            task.plan_pending && !workspace.task_steps_of(&task.id).is_empty()
                        })
            }
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
            Kind::Escalation => escalated_item(&summary.key)
                .and_then(|item| self.supervisor().map(|store| store.item_open(item)))
                .unwrap_or(false),
            Kind::Resource => summary
                .key
                .strip_prefix("res:")
                .and_then(|rest| rest.rsplit_once(':'))
                .and_then(|(key, since)| {
                    Some((ResourceKey::parse(key)?, since.parse::<i64>().ok()?))
                })
                .zip(self.supervisor())
                .is_some_and(|((key, since), store)| {
                    store.resources(|r| r.open(&key).is_some_and(|open| open.since == since))
                }),
            Kind::NodeFailed => summary
                .thread_id
                .as_deref()
                .and_then(|thread| sessions.session_status(thread))
                .is_some_and(|session| session.status == SessionStatus::Errored),
            Kind::MissionDone | Kind::PlanEdited | Kind::Heartbeat | Kind::NodeFinished => true,
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
                let tree = || {
                    let root = event.summary.root_chat_id.as_deref()?;
                    let title = workspace.chat(root).ok().flatten()?.title;
                    Some(format!(
                        "tree \"{}\" (root thread {root})",
                        clip(title.as_deref().unwrap_or("Untitled"), TITLE_CHARS)
                    ))
                };
                let mission = task
                    .as_ref()
                    .map(|task| format!("{} (task {})", clip(&task.title, TITLE_CHARS), task.id))
                    .or_else(tree);
                let lead = task.and_then(|task| task.chat_id);
                let source = event.summary.thread_id.clone().or(lead.clone());
                let reply = source.map(|thread| Reply {
                    from_lead: Some(&thread) == lead.as_ref(),
                    text: self
                        .last_reply(&thread)
                        .map(|text| clip(&text, WAKE_QUESTION_CHARS)),
                });
                let escalation = (event.summary.kind == Kind::Escalation)
                    .then(|| escalated_item(&event.summary.key))
                    .flatten()
                    .and_then(|item| self.supervisor()?.item(item))
                    .map(|item| item.why);
                WakeEntry {
                    event,
                    mission,
                    reply,
                    escalation,
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

fn thread_label(chat: &Chat) -> String {
    format!(
        "Thread \"{}\"",
        clip(chat.title.as_deref().unwrap_or("Untitled"), TITLE_CHARS)
    )
}

fn earliest(events: &[&SupervisorEvent], now: i64) -> i64 {
    events
        .iter()
        .map(|event| ms(event.summary.at))
        .min()
        .unwrap_or(now)
}

const PLAN_READY: &str = "plan-ready:";

fn plan_event(plan: &PendingPlan) -> SupervisorEvent {
    let mut event = SupervisorEvent::new(
        Kind::NeedsYou,
        format!("{PLAN_READY}{}:{}", plan.task_id, plan.newest),
        Some(&plan.task_id),
        "Plan ready for review".to_string(),
    );
    event.summary.thread_id = Some(plan.lead.clone());
    let steps = match plan.steps {
        1 => "1 step waits".to_string(),
        n => format!("{n} steps wait"),
    };
    event.detail = vec![format!(
        "{steps} for approval. Read the plan with the tasks tool (action \"list\", taskId {}), then approve it with the supervisor tool, action \"approve\" (taskId, reason) when it fits the mission, or message the lead what to change.",
        plan.task_id
    )];
    event
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
