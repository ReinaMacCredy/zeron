use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use chrono::{DateTime, Utc};
use zeron_doc::WorkspaceState;
use zeron_proto::mission::{self, ApprovalState, PlanPhase, Rollup};
use zeron_proto::supervisor::{
    ItemWhy, NeedsYouRow, Raiser, SupervisorSnapshot, SupervisorState, SupervisorWake,
};
use zeron_proto::{Chat, ChatIndicator, HarnessId, StepStatus, Task, TaskStatus, TaskStep};

use super::view::{DeriveContext, DeviceView, ProjectRef, SessionRow, project_color_index};

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
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct NeedsYouEntry {
    pub key: String,
    pub host: HostRef,
    pub title: String,
    pub task_id: Option<String>,
    pub task_title: Option<String>,
    pub chat_id: String,
    pub at_ms: i64,
    pub kind: NeedsYouKind,
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
    pub needs_you: Vec<NeedsYouEntry>,
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
pub struct TaskDetail {
    pub row: TaskRow,
    pub phase: PlanPhase,
    pub plan: String,
    pub approval: Option<ApprovalView>,
    pub steps: Vec<StepView>,
}

struct Ctx<'a> {
    state: &'a WorkspaceState,
    steps: &'a [TaskStep],
    rows: &'a HashMap<String, Arc<SessionRow>>,
    devices: &'a [DeviceView],
    supervisors: &'a BTreeMap<String, SupervisorSnapshot>,
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
) -> (TasksPage, SupervisorPage) {
    let ctx = Ctx {
        state,
        steps: cx.steps,
        rows,
        devices,
        supervisors: cx.supervisors,
        now: cx.now,
    };
    (tasks_page(&ctx), supervisor_page(&ctx))
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
    let tasks: HashMap<&str, &Task> = ctx
        .state
        .tasks
        .iter()
        .map(|task| (task.id.as_str(), task))
        .collect();
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
        for row in &snapshot.needs_you {
            let (task_id, chat_id) = row.origin();
            page.needs_you.push(NeedsYouEntry {
                key: format!("{device_id}/{}", row.key()),
                host: host.clone(),
                title: row.title(),
                task_id: task_id.map(str::to_owned),
                task_title: task_id
                    .and_then(|id| tasks.get(id))
                    .map(|task| task.title.clone()),
                chat_id: chat_id.to_owned(),
                at_ms: row.at().timestamp_millis(),
                kind: match row {
                    NeedsYouRow::Item(item) => NeedsYouKind::Item {
                        item_id: item.id.clone(),
                        why: item.why,
                        raised_by_lead: item.raised_by == Raiser::Lead,
                        detail: item.detail.clone(),
                        options: item.options.clone(),
                    },
                    NeedsYouRow::Input(input) => NeedsYouKind::Input {
                        chat_id: input.chat_id.clone(),
                        request_id: input.request_id.clone(),
                        questions: input.questions.clone(),
                    },
                },
            });
        }
        page.wakes
            .extend(snapshot.wakes.iter().map(|wake| wake_entry(wake, &host)));
    }
    page.needs_you
        .sort_by(|a, b| b.at_ms.cmp(&a.at_ms).then_with(|| a.key.cmp(&b.key)));
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
    })
}
