use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Arc;

use chrono::{DateTime, Utc};
use zeron_doc::WorkspaceState;
use zeron_proto::mission::{self, ApprovalState, PlanPhase, Rollup};
use zeron_proto::supervisor::{
    InputAsker, ItemWhy, NeedsYouInput, NeedsYouRow, Raiser, SendDest, SupervisorSnapshot,
    SupervisorState, SupervisorWake,
};
use zeron_proto::{Chat, ChatIndicator, HarnessId, StepStatus, Task, TaskStatus, TaskStep};

use super::view::{DeriveContext, DeviceView, ProjectRef, SessionRow, project_color_index};
use crate::session::InputRequest;

const WAKE_LOG_ROWS: usize = 30;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostRef {
    pub device_id: String,
    pub name: String,
    pub online: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TaskRow {
    pub id: String,
    pub title: String,
    pub notes: String,
    pub due_on: Option<String>,
    pub project: Option<ProjectRef>,
    pub status: TaskStatus,
    pub word: String,
    pub mission: bool,
    pub plan_ready: bool,
    pub lead_chat_id: Option<String>,
    pub host: Option<HostRef>,
    pub progress_done: u32,
    pub progress_total: u32,
    pub mission_label: Option<String>,
    pub current: Option<String>,
    pub questions: u32,
    pub brands: Vec<String>,
    pub working_since_ms: Option<i64>,
    pub created_at_ms: i64,
    pub completed_at_ms: Option<i64>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct TasksPage {
    pub missions: Vec<TaskRow>,
    pub needs_you: Vec<TaskRow>,
    pub working: Vec<TaskRow>,
    pub to_do: Vec<TaskRow>,
    pub done: Vec<TaskRow>,
    pub needs_you_count: u32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SupervisorHost {
    pub host: HostRef,
    pub chat_id: Option<String>,
    pub state: SupervisorState,
    pub blocked: Option<String>,
    pub waiting: u32,
    pub trees: u32,
    pub watch_everything: bool,
    pub wakes_this_hour: u32,
    pub last_wake_ms: Option<i64>,
    pub next_check_ms: Option<i64>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum NeedsYouKind {
    Item {
        item_id: String,
        why: ItemWhy,
        raised_by_lead: bool,
        detail: String,
        options: Vec<String>,
    },
    Input {
        chat_id: String,
        request_id: String,
        questions: Vec<zeron_proto::UserInputQuestion>,
        approval: bool,
        destructive: bool,
    },
    Plan {
        task_id: String,
        steps: Vec<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ItemSend {
    Sending { choice: String },
    Sent { choice: String },
    Failed { choice: String, reason: String },
}

#[derive(Debug, Clone, PartialEq)]
pub struct NeedsYouEntry {
    pub key: String,
    pub host: HostRef,
    pub title: String,
    pub asker: String,
    pub supervised: bool,
    pub task_id: Option<String>,
    pub task_title: Option<String>,
    pub chat_id: String,
    pub at_ms: i64,
    pub group_key: String,
    pub group_title: String,
    pub dest: String,
    pub send: Option<ItemSend>,
    pub kind: NeedsYouKind,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Ask {
    Open(InputRequest),
    Answered(String),
}

#[derive(Debug, Clone, PartialEq)]
pub struct WakeEntry {
    pub id: String,
    pub host: HostRef,
    pub at_ms: i64,
    pub events: Vec<String>,
    pub delivery: String,
    pub woke: bool,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct SupervisorPage {
    pub hosts: Vec<SupervisorHost>,
    pub done_today: Vec<TaskRow>,
    pub wakes: Vec<WakeEntry>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ApprovalView {
    pub line: String,
    pub held: bool,
    pub can_take_back: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct StepView {
    pub id: String,
    pub label: String,
    pub title: String,
    pub notes: String,
    pub status: StepStatus,
    pub chat_id: Option<String>,
    pub brand: Option<String>,
    pub question: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct MissionStrip {
    pub task_id: String,
    pub title: String,
    pub role: String,
    pub step_id: Option<String>,
    pub done: u32,
    pub total: u32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TaskDetail {
    pub row: TaskRow,
    pub phase: PlanPhase,
    pub plan: String,
    pub approval: Option<ApprovalView>,
    pub steps: Vec<StepView>,
    pub images: Vec<String>,
    pub context: Vec<ContextChat>,
    pub watch: Option<WatchState>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContextChat {
    pub chat_id: String,
    pub title: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WatchState {
    pub root_chat_id: String,
    pub host: HostRef,
    pub mission: bool,
    pub watched: bool,
    pub everything: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Brief {
    pub title: String,
    pub meta: Vec<String>,
    pub task: String,
    pub spec: String,
    pub thread_chat_id: Option<String>,
    pub lead_chat_id: Option<String>,
    pub step_labels: Vec<(String, String)>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BriefMessage {
    pub tag: String,
    pub from_you: bool,
    pub text: String,
    pub at_ms: i64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TaskDraft {
    pub title: String,
    pub notes: String,
    pub space_id: Option<String>,
    pub images: Vec<String>,
    pub context: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MissionOptions {
    pub approve_plan: bool,
    pub same_agents: bool,
    pub plan_doc: bool,
}

impl Default for MissionOptions {
    fn default() -> Self {
        Self {
            approve_plan: true,
            same_agents: false,
            plan_doc: true,
        }
    }
}

struct Ctx<'a> {
    state: &'a WorkspaceState,
    steps: &'a [TaskStep],
    rows: &'a HashMap<String, Arc<SessionRow>>,
    devices: &'a [DeviceView],
    supervisors: &'a BTreeMap<String, SupervisorSnapshot>,
    item_sends: &'a HashMap<String, ItemSend>,
    now: DateTime<Utc>,
}

impl Ctx<'_> {
    fn indicator(&self, chat: &Chat) -> ChatIndicator {
        self.rows
            .get(&chat.id)
            .map_or(ChatIndicator::Idle, |row| row.indicator)
    }

    fn host(&self, device_id: &str) -> HostRef {
        let device = self.devices.iter().find(|d| d.id == device_id);
        HostRef {
            device_id: device_id.to_owned(),
            name: device.map_or_else(|| "Unknown device".to_owned(), |d| d.name.clone()),
            online: device.is_some_and(|d| d.online),
        }
    }

    fn rollup(&self, task: &Task) -> Rollup<String> {
        mission::rollup(
            task,
            &self.state.chats,
            self.steps,
            self.supervisors.values().flat_map(|s| &s.needs_you),
            |chat| self.indicator(chat),
            |chat| brand(chat).unwrap_or_default(),
        )
    }

    fn task_device(&self, task: &Task) -> Option<String> {
        mission::linked_chat(&self.state.chats, task)
            .map(|chat| chat.device_id.clone())
            .or_else(|| {
                let space = task.space_id.as_deref()?;
                let space = self.state.spaces.iter().find(|s| s.id == space)?;
                Some(space.device_id.clone())
            })
    }

    fn approval(&self, task: &Task) -> Option<ApprovalState> {
        let device = self.task_device(task)?;
        let snapshot = self.supervisors.get(&device)?;
        mission::plan_approval(&snapshot.interventions, self.steps, &task.id, self.now)
    }

    fn row(&self, task: &Task, plan: &Rollup<String>) -> TaskRow {
        let (word, _) = plan.label(task);
        let lead = mission::linked_chat(&self.state.chats, task);
        let mut brands: Vec<String> = Vec::new();
        for mark in lead
            .and_then(brand)
            .into_iter()
            .chain(plan.steps.iter().filter_map(|row| row.brand.clone()))
            .filter(|mark| !mark.is_empty())
        {
            if brands.len() < 3 && !brands.contains(&mark) {
                brands.push(mark);
            }
        }
        let held = matches!(self.approval(task), Some(ApprovalState::Held { .. }));
        let mission_row = task
            .has_plan()
            .then(|| mission::mission_row(task, plan, held));
        let project = task
            .space_id
            .as_deref()
            .and_then(|id| self.state.spaces.iter().find(|s| s.id == id))
            .map(|space| ProjectRef {
                id: space.id.clone(),
                name: space.display_name().to_owned(),
                color_index: project_color_index(&space.path),
            });
        TaskRow {
            id: task.id.clone(),
            title: task.title.clone(),
            notes: task.notes.clone(),
            due_on: task.due_on.clone(),
            project,
            status: mission_row.as_ref().map_or(plan.status, |row| row.status),
            word: word.to_owned(),
            mission: task.has_plan(),
            plan_ready: plan.phase(task) == PlanPhase::PlanReady,
            lead_chat_id: lead.map(|chat| chat.id.clone()),
            host: self.task_device(task).map(|device| self.host(&device)),
            progress_done: plan.progress.done as u32,
            progress_total: plan.progress.total as u32,
            mission_label: mission_row.as_ref().map(|row| row.label.clone()),
            current: mission_row.and_then(|row| row.current),
            questions: plan.questions as u32,
            brands,
            working_since_ms: lead
                .and_then(|chat| self.rows.get(&chat.id))
                .and_then(|row| row.working_since_ms),
            created_at_ms: task.created_at.timestamp_millis(),
            completed_at_ms: task.completed_at.map(|at| at.timestamp_millis()),
        }
    }

    fn task_row(&self, task: &Task) -> TaskRow {
        self.row(task, &self.rollup(task))
    }

    fn watch(&self, chat_id: &str) -> Option<WatchState> {
        let chats = &self.state.chats;
        let chat = chats.iter().find(|chat| chat.id == chat_id)?;
        let root = zeron_proto::tree_path(chat, chats, &self.state.tasks, self.steps)
            .last()
            .copied()
            .unwrap_or(chat);
        let snapshot = self.supervisors.get(&root.device_id)?;
        let mission = zeron_proto::plan_of_chat(&self.state.tasks, &root.id).is_some();
        let watched = snapshot.watches(&self.state.tasks, &root.id);
        Some(WatchState {
            root_chat_id: root.id.clone(),
            host: self.host(&root.device_id),
            mission,
            watched,
            everything: !mission
                && snapshot.watch_everything
                && !snapshot.watched_roots.iter().any(|id| *id == root.id),
        })
    }
}

fn brand(chat: &Chat) -> Option<String> {
    let config = chat.config.as_ref();
    let harness = HarnessId::brand_of(
        config.map(|c| c.harness),
        config.and_then(|c| c.model.as_deref()),
    )?;
    serde_json::to_value(harness)
        .ok()?
        .as_str()
        .map(str::to_owned)
}

pub(crate) fn derive(
    state: &WorkspaceState,
    rows: &HashMap<String, Arc<SessionRow>>,
    devices: &[DeviceView],
    cx: &DeriveContext<'_>,
) -> (TasksPage, SupervisorPage, Vec<NeedsYouEntry>) {
    let ctx = Ctx {
        state,
        steps: cx.steps,
        rows,
        devices,
        supervisors: cx.supervisors,
        item_sends: cx.item_sends,
        now: cx.now,
    };
    (
        tasks_page(&ctx),
        supervisor_page(&ctx),
        needs_you(&ctx, cx.asks),
    )
}

fn tasks_page(ctx: &Ctx<'_>) -> TasksPage {
    let mut page = TasksPage::default();
    let mut missions: Vec<TaskRow> = mission::open_missions(&ctx.state.tasks)
        .into_iter()
        .map(|task| ctx.task_row(task))
        .collect();
    missions.sort_by_key(|row| row.status != TaskStatus::NeedsYou);
    page.missions = missions;
    let (open, done) = mission::split_tasks(&ctx.state.tasks);
    for task in open {
        let row = ctx.task_row(task);
        if row.status == TaskStatus::NeedsYou {
            page.needs_you_count += 1;
        }
        if task.has_plan() {
            continue;
        }
        match row.status {
            TaskStatus::NeedsYou => page.needs_you.push(row),
            TaskStatus::Working => page.working.push(row),
            TaskStatus::ToDo => page.to_do.push(row),
            TaskStatus::Done => page.done.push(row),
        }
    }
    page.done
        .extend(done.into_iter().map(|task| ctx.task_row(task)));
    page
}

fn supervisor_page(ctx: &Ctx<'_>) -> SupervisorPage {
    let mut page = SupervisorPage::default();
    for (device_id, snapshot) in ctx.supervisors {
        let host = ctx.host(device_id);
        page.hosts.push(SupervisorHost {
            host: host.clone(),
            chat_id: snapshot.chat_id.clone(),
            state: snapshot.state,
            blocked: snapshot.blocked.clone(),
            waiting: snapshot.waiting as u32,
            trees: snapshot.watched_roots.len() as u32,
            watch_everything: snapshot.watch_everything,
            wakes_this_hour: zeron_proto::supervisor::wakes_this_hour(&snapshot.wakes, ctx.now)
                as u32,
            last_wake_ms: snapshot
                .wakes
                .iter()
                .map(|w| w.at)
                .max()
                .map(|at| at.timestamp_millis()),
            next_check_ms: snapshot
                .next_check
                .as_ref()
                .map(|next| next.at.timestamp_millis()),
        });
        page.wakes
            .extend(snapshot.wakes.iter().map(|wake| wake_entry(wake, &host)));
    }
    page.wakes
        .sort_by(|a, b| b.at_ms.cmp(&a.at_ms).then_with(|| a.id.cmp(&b.id)));
    page.wakes.truncate(WAKE_LOG_ROWS);
    let today = ctx.now.with_timezone(&chrono::Local).date_naive();
    page.done_today = mission::missions_done_on(&ctx.state.tasks, today, &chrono::Local)
        .into_iter()
        .map(|task| ctx.task_row(task))
        .collect();
    page
}

fn needs_you(ctx: &Ctx<'_>, asks: &HashMap<String, Ask>) -> Vec<NeedsYouEntry> {
    let names = mission::Names {
        chats: &ctx.state.chats,
        tasks: &ctx.state.tasks,
        steps: ctx.steps,
        plan_of: |task: &Task| ctx.rollup(task),
    };
    let mut entries = Vec::new();
    let mut asked: HashSet<&str> = HashSet::new();
    for (device_id, snapshot) in ctx.supervisors {
        for row in &snapshot.needs_you {
            let mut host = device_id.as_str();
            if let NeedsYouRow::Input(input) = row {
                if matches!(asks.get(&input.chat_id), Some(Ask::Answered(id)) if *id == input.request_id)
                {
                    continue;
                }
                asked.insert(&input.chat_id);
                if let Some(chat) = ctx.rows.get(&input.chat_id) {
                    host = &chat.device_id;
                }
            }
            let key = format!("{device_id}/{}", row.key());
            entries.push(entry(ctx, &names, row, key, ctx.host(host), true));
        }
    }
    for task in mission::open_missions(&ctx.state.tasks) {
        let plan = ctx.rollup(task);
        if plan.phase(task) != PlanPhase::PlanReady {
            continue;
        }
        let Some(lead) = mission::linked_chat(&ctx.state.chats, task) else {
            continue;
        };
        entries.push(NeedsYouEntry {
            key: format!("plan:{}", task.id),
            host: ctx.host(&lead.device_id),
            title: format!(
                "Plan ready · {} step{} · the lead waits for you",
                plan.steps.len(),
                if plan.steps.len() == 1 { "" } else { "s" }
            ),
            asker: format!("Lead · {}", task.title),
            supervised: true,
            task_id: Some(task.id.clone()),
            task_title: Some(task.title.clone()),
            chat_id: lead.id.clone(),
            at_ms: task.updated_at.timestamp_millis(),
            group_key: task.id.clone(),
            group_title: task.title.clone(),
            dest: SendDest::Lead.word().to_owned(),
            send: None,
            kind: NeedsYouKind::Plan {
                task_id: task.id.clone(),
                steps: plan
                    .steps
                    .iter()
                    .map(|row| row.step.title.clone())
                    .collect(),
            },
        });
    }
    for chat in ctx.rows.values() {
        if chat.archived
            || chat.indicator != ChatIndicator::AwaitingInput
            || asked.contains(chat.id.as_str())
        {
            continue;
        }
        let (request_id, questions) = match asks.get(&chat.id) {
            Some(Ask::Open(request)) => (request.request_id.clone(), request.questions.clone()),
            Some(Ask::Answered(_)) => continue,
            None => (String::new(), Vec::new()),
        };
        let (asker, task_id, step_id) = match names.mission_thread(&chat.id) {
            Some(thread) => match thread.role {
                mission::MissionRole::Lead => (InputAsker::Lead, Some(thread.task_id), None),
                mission::MissionRole::Step { step_id, .. } => {
                    (InputAsker::Step, Some(thread.task_id), Some(step_id))
                }
            },
            None => (InputAsker::Thread, None, None),
        };
        let row = NeedsYouRow::Input(NeedsYouInput {
            chat_id: chat.id.clone(),
            request_id,
            asker,
            task_id,
            step_id,
            at: DateTime::from_timestamp_millis(chat.last_activity_ms).unwrap_or_default(),
            questions,
        });
        let key = format!("{}/{}", chat.device_id, row.key());
        entries.push(entry(ctx, &names, &row, key, ctx.host(&chat.device_id), false));
    }
    entries.sort_by(|a, b| b.at_ms.cmp(&a.at_ms).then_with(|| a.key.cmp(&b.key)));
    entries
}

fn entry<B>(
    ctx: &Ctx<'_>,
    names: &mission::Names<'_, impl Fn(&Task) -> Rollup<B>>,
    row: &NeedsYouRow,
    key: String,
    host: HostRef,
    supervised: bool,
) -> NeedsYouEntry {
    let (task_id, chat_id) = row.origin();
    let group_key = row
        .group(&|chat| names.tree_root(chat).map(|root| root.id.clone()))
        .unwrap_or_default();
    let group_title = if group_key.is_empty() {
        "Supervisor and shared".to_owned()
    } else if let Some(title) = names.mission_title(Some(&group_key)) {
        title
    } else {
        ctx.state
            .chats
            .iter()
            .find(|chat| chat.id == group_key)
            .map_or_else(|| "A mission".to_owned(), |chat| names.chat_name(chat))
    };
    NeedsYouEntry {
        send: ctx.item_sends.get(&key).cloned(),
        key,
        host,
        title: match row {
            NeedsYouRow::Input(input) if input.questions.is_empty() => "Waiting for you".to_owned(),
            row => row.title(),
        },
        asker: match row {
            NeedsYouRow::Item(item) => names.raiser_name(item),
            NeedsYouRow::Input(input) => names.asker_name(input),
        },
        supervised,
        task_id: task_id.map(str::to_owned),
        task_title: names.mission_title(task_id),
        chat_id: chat_id.to_owned(),
        at_ms: row.at().timestamp_millis(),
        group_title,
        group_key,
        dest: row.dest().word().to_owned(),
        kind: match row {
            NeedsYouRow::Item(item) => NeedsYouKind::Item {
                item_id: item.id.clone(),
                why: item.why,
                raised_by_lead: item.raised_by == Raiser::Lead,
                detail: item.detail.clone(),
                options: item.options.clone(),
            },
            NeedsYouRow::Input(input) => {
                let approval = input.questions.iter().any(|q| q.approval);
                NeedsYouKind::Input {
                    chat_id: input.chat_id.clone(),
                    request_id: input.request_id.clone(),
                    questions: input.questions.clone(),
                    approval,
                    destructive: approval && destructive_approval(&input.questions),
                }
            }
        },
    }
}

fn destructive_approval(questions: &[zeron_proto::UserInputQuestion]) -> bool {
    const WORDS: &[&str] = &[
        "delete", "remove", "rm", "rmdir", "unlink", "trash", "push", "edit", "multiedit",
        "notebookedit", "write", "modify", "patch", "apply_patch",
    ];
    questions.iter().filter(|q| q.approval).any(|q| {
        let text = format!("{} {}", q.header, q.question).to_lowercase();
        text.contains("file change")
            || text
                .split(|c: char| !c.is_alphanumeric() && c != '_')
                .any(|word| WORDS.contains(&word))
    })
}

fn wake_entry(wake: &SupervisorWake, host: &HostRef) -> WakeEntry {
    WakeEntry {
        id: wake.id.clone(),
        host: host.clone(),
        at_ms: wake.at.timestamp_millis(),
        events: wake
            .events
            .iter()
            .map(|event| event.kind.word().to_owned())
            .collect(),
        delivery: wake.delivery.label(),
        woke: wake.woke(),
    }
}

pub(crate) fn detail(
    state: &WorkspaceState,
    steps: &[TaskStep],
    snapshot: &super::WorkspaceSnapshot,
    supervisors: &BTreeMap<String, SupervisorSnapshot>,
    task_id: &str,
) -> Option<TaskDetail> {
    let task = state
        .tasks
        .iter()
        .find(|task| task.id == task_id && !task.deleted)?;
    let ctx = Ctx {
        state,
        steps,
        rows: &snapshot.sessions,
        devices: &snapshot.devices,
        supervisors,
        item_sends: &HashMap::new(),
        now: Utc::now(),
    };
    let plan = ctx.rollup(task);
    let questions: Vec<&NeedsYouRow> = supervisors
        .values()
        .flat_map(|s| &s.needs_you)
        .filter(|row| row.task_id() == Some(task.id.as_str()))
        .collect();
    let step_views = plan
        .steps
        .iter()
        .enumerate()
        .map(|(ix, row)| StepView {
            id: row.step.id.clone(),
            label: format!("Step {}", ix + 1),
            title: row.step.title.clone(),
            notes: row.step.notes.clone(),
            status: row.status,
            chat_id: row.step.chat_id.clone(),
            brand: row.brand.clone().filter(|b| !b.is_empty()),
            question: questions
                .iter()
                .find(|q| match q {
                    NeedsYouRow::Input(input) => {
                        input.step_id.as_deref() == Some(row.step.id.as_str())
                            || Some(input.chat_id.as_str()) == row.step.chat_id.as_deref()
                    }
                    NeedsYouRow::Item(item) => {
                        Some(item.raiser_chat_id.as_str()) == row.step.chat_id.as_deref()
                    }
                })
                .map(|q| q.title()),
        })
        .collect();
    let phase = plan.phase(task);
    let approval = (phase != PlanPhase::PlanReady && task.completed_at.is_none())
        .then(|| ctx.approval(task))
        .flatten()
        .filter(|state| *state != ApprovalState::TakenBack)
        .map(|state| ApprovalView {
            line: state.line(),
            held: matches!(state, ApprovalState::Held { .. }),
            can_take_back: state.can_take_back(),
        });
    Some(TaskDetail {
        row: ctx.row(task, &plan),
        phase,
        plan: task.plan.clone(),
        approval,
        steps: step_views,
        images: task.images.clone(),
        context: task
            .context
            .iter()
            .map(|id| ContextChat {
                chat_id: id.clone(),
                title: state
                    .chats
                    .iter()
                    .find(|chat| chat.id == *id)
                    .and_then(|chat| chat.title.clone())
                    .unwrap_or_else(|| "New session".to_owned()),
            })
            .collect(),
        watch: task.chat_id.as_deref().and_then(|lead| ctx.watch(lead)),
    })
}

pub(crate) fn watch(
    state: &WorkspaceState,
    steps: &[TaskStep],
    snapshot: &super::WorkspaceSnapshot,
    supervisors: &BTreeMap<String, SupervisorSnapshot>,
    chat_id: &str,
) -> Option<WatchState> {
    Ctx {
        state,
        steps,
        rows: &snapshot.sessions,
        devices: &snapshot.devices,
        supervisors,
        item_sends: &HashMap::new(),
        now: Utc::now(),
    }
    .watch(chat_id)
}

pub(crate) fn strip(
    state: &WorkspaceState,
    steps: &[TaskStep],
    snapshot: &super::WorkspaceSnapshot,
    supervisors: &BTreeMap<String, SupervisorSnapshot>,
    chat_id: &str,
) -> Option<MissionStrip> {
    let ctx = Ctx {
        state,
        steps,
        rows: &snapshot.sessions,
        devices: &snapshot.devices,
        supervisors,
        item_sends: &HashMap::new(),
        now: Utc::now(),
    };
    let thread = mission::mission_thread(&state.tasks, steps, chat_id, |task| ctx.rollup(task))?;
    let (role, step_id) = match thread.role {
        mission::MissionRole::Lead => ("Lead".to_owned(), None),
        mission::MissionRole::Step { step_id, label } => (label, Some(step_id)),
    };
    Some(MissionStrip {
        task_id: thread.task_id,
        title: thread.title,
        role,
        step_id,
        done: thread.progress.done as u32,
        total: thread.progress.total as u32,
    })
}

pub(crate) fn brief(
    state: &WorkspaceState,
    steps: &[TaskStep],
    snapshot: &super::WorkspaceSnapshot,
    supervisors: &BTreeMap<String, SupervisorSnapshot>,
    task_id: &str,
    step_id: Option<&str>,
) -> Option<Brief> {
    let task = state
        .tasks
        .iter()
        .find(|task| task.id == task_id && !task.deleted)?;
    let ctx = Ctx {
        state,
        steps,
        rows: &snapshot.sessions,
        devices: &snapshot.devices,
        supervisors,
        item_sends: &HashMap::new(),
        now: Utc::now(),
    };
    let plan = ctx.rollup(task);
    let step = step_id.and_then(|id| plan.steps.iter().find(|row| row.step.id == id));
    let thread = match step_id {
        Some(_) => step.and_then(|row| row.step.chat_id.clone()),
        None => task.chat_id.clone(),
    };
    let title = match step {
        Some(row) => format!(
            "Brief · {} · {}",
            plan.step_label(&row.step.id).unwrap_or_default(),
            row.step.title
        ),
        None if step_id.is_some() => "Brief · A removed step".to_owned(),
        None => "Brief · Lead".to_owned(),
    };
    let chat = thread
        .as_deref()
        .and_then(|id| state.chats.iter().find(|chat| chat.id == id));
    let mut meta = vec![task.title.clone()];
    if let Some(model) = chat.and_then(|chat| chat.config.as_ref()?.model.clone()) {
        meta.push(model);
    }
    let started = chat.and_then(|chat| match &chat.source_context {
        Some(source) => Some(match source.head_sha.as_deref() {
            Some(sha) => format!(
                "started on {} at {}",
                source.branch,
                &sha[..sha.len().min(7)]
            ),
            None => format!("started on {}", source.branch),
        }),
        None => chat
            .branch
            .as_deref()
            .filter(|branch| !branch.is_empty())
            .map(|branch| format!("started on {branch}")),
    });
    meta.extend(started);
    let mut spec = task.plan.trim().to_owned();
    if let Some(notes) = step
        .map(|row| row.step.notes.trim())
        .filter(|n| !n.is_empty())
    {
        if !spec.is_empty() {
            spec.push_str("\n\n");
        }
        spec.push_str(notes);
    }
    let mut task_text = task.title.clone();
    if !task.notes.trim().is_empty() {
        task_text.push_str("\n\n");
        task_text.push_str(task.notes.trim());
    }
    Some(Brief {
        title,
        meta,
        task: task_text,
        spec,
        thread_chat_id: thread,
        lead_chat_id: task.chat_id.clone(),
        step_labels: plan
            .steps
            .iter()
            .filter_map(|row| Some((row.step.chat_id.clone()?, plan.step_label(&row.step.id)?)))
            .collect(),
    })
}

#[derive(Debug, Clone, PartialEq)]
pub struct EstimateLine {
    pub step_id: String,
    pub title: String,
    pub role: Option<String>,
    pub runs_on: String,
    pub multiplier: Option<String>,
    pub fallback: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PlanEstimate {
    pub lines: Vec<EstimateLine>,
    pub mix: Vec<String>,
    pub cost: String,
    pub baseline: String,
    pub raised: Option<String>,
}

fn model_text(target: &zeron_proto::RouteTarget) -> Option<String> {
    let model = target.model.as_deref()?;
    let harness = serde_json::to_value(target.harness).ok()?;
    Some(crate::catalog::model_label(harness.as_str()?, model))
}

fn runs_as(target: &zeron_proto::RouteTarget, native: bool) -> String {
    let harness = target.harness.display_name();
    if native {
        return format!("{harness} native");
    }
    match model_text(target) {
        Some(model) => format!("{harness} · {model}"),
        None => harness.to_owned(),
    }
}

pub(crate) fn plan_estimate(
    steps: &[&TaskStep],
    chats: &[Chat],
    lead: Option<&zeron_proto::RouteTarget>,
    preview: Option<&zeron_proto::route_preview::Preview>,
) -> PlanEstimate {
    use zeron_proto::route_preview as rp;
    let clock = |ms: i64| {
        DateTime::<Utc>::from_timestamp_millis(ms)
            .map(|at| at.with_timezone(&chrono::Local).format("%H:%M").to_string())
    };
    let labels = rp::Labels {
        runs_as: &runs_as,
        clock: &clock,
    };
    let lines = rp::plan_lines(steps, chats, lead, preview, &labels);
    let totals = rp::totals(&lines);
    let count = lines.len();
    let raised = (totals.raised > 0 && totals.total > totals.planned + 1e-9).then(|| {
        let to = lines
            .iter()
            .filter(|line| line.fallback.is_some())
            .filter_map(|line| line.target.as_ref().and_then(model_text))
            .next()
            .unwrap_or_else(|| "the chat's model".to_owned());
        format!(
            "Relative cost {} → {} because {} {} fell back to {to}",
            rp::multiplier_text(totals.planned),
            rp::multiplier_text(totals.total),
            totals.raised,
            if totals.raised == 1 {
                "child"
            } else {
                "children"
            }
        )
    });
    let lead_label = lead
        .and_then(model_text)
        .unwrap_or_else(|| "model".to_owned());
    PlanEstimate {
        mix: rp::mix(&lines)
            .into_iter()
            .map(|(target, native, n)| format!("{n} {}", runs_as(&target, native)))
            .collect(),
        cost: format!("Relative cost {}", rp::multiplier_text(totals.total)),
        baseline: format!("vs {count}.0x if all ran on the lead's {lead_label}"),
        raised,
        lines: steps
            .iter()
            .zip(lines)
            .map(|(step, line)| EstimateLine {
                step_id: step.id.clone(),
                runs_on: match (&line.target, line.agent_picks) {
                    (Some(target), true) => format!(
                        "agent picks · {}",
                        model_text(target)
                            .unwrap_or_else(|| target.harness.display_name().to_owned())
                    ),
                    (None, true) => "agent picks".to_owned(),
                    (Some(target), false) => runs_as(target, line.native),
                    (None, false) => "…".to_owned(),
                },
                multiplier: line.multiplier.map(rp::multiplier_text),
                role: line.role.map(str::to_owned),
                title: line.title,
                fallback: line.fallback,
            })
            .collect(),
    }
}
