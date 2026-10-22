
use zeron_client as zc;
use zeron_proto::supervisor as sv;

use super::{CoreClient, CoreResult, ProjectRef, UserInputQuestion, on_runtime};

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum TaskStatus {
    ToDo,
    Working,
    NeedsYou,
    Done,
}

impl From<zeron_proto::TaskStatus> for TaskStatus {
    fn from(s: zeron_proto::TaskStatus) -> Self {
        match s {
            zeron_proto::TaskStatus::ToDo => Self::ToDo,
            zeron_proto::TaskStatus::Working => Self::Working,
            zeron_proto::TaskStatus::NeedsYou => Self::NeedsYou,
            zeron_proto::TaskStatus::Done => Self::Done,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum StepStatus {
    Planned,
    Working,
    NeedsYou,
    Done,
    Skipped,
    Failed,
}

impl From<zeron_proto::StepStatus> for StepStatus {
    fn from(s: zeron_proto::StepStatus) -> Self {
        match s {
            zeron_proto::StepStatus::Planned => Self::Planned,
            zeron_proto::StepStatus::Working => Self::Working,
            zeron_proto::StepStatus::NeedsYou => Self::NeedsYou,
            zeron_proto::StepStatus::Done => Self::Done,
            zeron_proto::StepStatus::Skipped => Self::Skipped,
            zeron_proto::StepStatus::Failed => Self::Failed,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum PlanPhase {
    PlanReady,
    Planning,
    Status { status: TaskStatus },
}

impl From<zeron_proto::mission::PlanPhase> for PlanPhase {
    fn from(p: zeron_proto::mission::PlanPhase) -> Self {
        match p {
            zeron_proto::mission::PlanPhase::PlanReady => Self::PlanReady,
            zeron_proto::mission::PlanPhase::Planning => Self::Planning,
            zeron_proto::mission::PlanPhase::Status(status) => Self::Status {
                status: status.into(),
            },
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum SupervisorState {
    Off,
    Asleep,
    Awake,
    Stopped,
}

impl From<sv::SupervisorState> for SupervisorState {
    fn from(s: sv::SupervisorState) -> Self {
        match s {
            sv::SupervisorState::Off => Self::Off,
            sv::SupervisorState::Asleep => Self::Asleep,
            sv::SupervisorState::Awake => Self::Awake,
            sv::SupervisorState::Stopped => Self::Stopped,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct HostRef {
    pub device_id: String,
    pub name: String,
    pub online: bool,
}

impl From<&zc::HostRef> for HostRef {
    fn from(h: &zc::HostRef) -> Self {
        Self {
            device_id: h.device_id.clone(),
            name: h.name.clone(),
            online: h.online,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
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

impl From<&zc::TaskRow> for TaskRow {
    fn from(r: &zc::TaskRow) -> Self {
        Self {
            id: r.id.clone(),
            title: r.title.clone(),
            notes: r.notes.clone(),
            due_on: r.due_on.clone(),
            project: r.project.as_ref().map(|p| ProjectRef {
                id: p.id.clone(),
                name: p.name.clone(),
                color_index: p.color_index,
            }),
            status: r.status.into(),
            word: r.word.clone(),
            mission: r.mission,
            plan_ready: r.plan_ready,
            lead_chat_id: r.lead_chat_id.clone(),
            host: r.host.as_ref().map(Into::into),
            progress_done: r.progress_done,
            progress_total: r.progress_total,
            mission_label: r.mission_label.clone(),
            current: r.current.clone(),
            questions: r.questions,
            brands: r.brands.clone(),
            working_since_ms: r.working_since_ms,
            created_at_ms: r.created_at_ms,
            completed_at_ms: r.completed_at_ms,
        }
    }
}

fn task_rows(rows: &[zc::TaskRow]) -> Vec<TaskRow> {
    rows.iter().map(Into::into).collect()
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct TasksPage {
    pub missions: Vec<TaskRow>,
    pub needs_you: Vec<TaskRow>,
    pub working: Vec<TaskRow>,
    pub to_do: Vec<TaskRow>,
    pub done: Vec<TaskRow>,
    pub needs_you_count: u32,
}

impl From<&zc::TasksPage> for TasksPage {
    fn from(p: &zc::TasksPage) -> Self {
        Self {
            missions: task_rows(&p.missions),
            needs_you: task_rows(&p.needs_you),
            working: task_rows(&p.working),
            to_do: task_rows(&p.to_do),
            done: task_rows(&p.done),
            needs_you_count: p.needs_you_count,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct SupervisorHost {
    pub host: HostRef,
    pub chat_id: Option<String>,
    pub state: SupervisorState,
    pub blocked: Option<String>,
    pub waiting: u32,
    pub trees: u32,
    pub watch_everything: bool,
    pub wakes_this_hour: u32,
    pub wakes_per_hour: u32,
    pub last_wake_ms: Option<i64>,
    pub next_check_ms: Option<i64>,
}

impl From<&zc::SupervisorHost> for SupervisorHost {
    fn from(h: &zc::SupervisorHost) -> Self {
        Self {
            host: (&h.host).into(),
            chat_id: h.chat_id.clone(),
            state: h.state.into(),
            blocked: h.blocked.clone(),
            waiting: h.waiting,
            trees: h.trees,
            watch_everything: h.watch_everything,
            wakes_this_hour: h.wakes_this_hour,
            wakes_per_hour: sv::WAKES_PER_HOUR as u32,
            last_wake_ms: h.last_wake_ms,
            next_check_ms: h.next_check_ms,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum NeedsYouKind {
    Item {
        item_id: String,
        why: String,
        raised_by_lead: bool,
        detail: String,
        options: Vec<String>,
    },
    Input {
        chat_id: String,
        request_id: String,
        questions: Vec<UserInputQuestion>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
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

impl From<&zc::NeedsYouEntry> for NeedsYouEntry {
    fn from(e: &zc::NeedsYouEntry) -> Self {
        Self {
            key: e.key.clone(),
            host: (&e.host).into(),
            title: e.title.clone(),
            task_id: e.task_id.clone(),
            task_title: e.task_title.clone(),
            chat_id: e.chat_id.clone(),
            at_ms: e.at_ms,
            kind: match &e.kind {
                zc::NeedsYouKind::Item {
                    item_id,
                    why,
                    raised_by_lead,
                    detail,
                    options,
                } => NeedsYouKind::Item {
                    item_id: item_id.clone(),
                    why: why.label().to_owned(),
                    raised_by_lead: *raised_by_lead,
                    detail: detail.clone(),
                    options: options.clone(),
                },
                zc::NeedsYouKind::Input {
                    chat_id,
                    request_id,
                    questions,
                } => NeedsYouKind::Input {
                    chat_id: chat_id.clone(),
                    request_id: request_id.clone(),
                    questions: questions
                        .iter()
                        .map(|q| UserInputQuestion {
                            id: q.id.clone(),
                            header: q.header.clone(),
                            question: q.question.clone(),
                            options: q.options.clone(),
                            multi_select: q.multi_select,
                            prefill: q.prefill.clone(),
                            multiline: q.multiline,
                        })
                        .collect(),
                },
            },
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct WakeEntry {
    pub id: String,
    pub host: HostRef,
    pub at_ms: i64,
    pub events: Vec<String>,
    pub delivery: String,
    pub woke: bool,
}

impl From<&zc::WakeEntry> for WakeEntry {
    fn from(w: &zc::WakeEntry) -> Self {
        Self {
            id: w.id.clone(),
            host: (&w.host).into(),
            at_ms: w.at_ms,
            events: w.events.clone(),
            delivery: w.delivery.clone(),
            woke: w.woke,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct SupervisorPage {
    pub hosts: Vec<SupervisorHost>,
    pub needs_you: Vec<NeedsYouEntry>,
    pub done_today: Vec<TaskRow>,
    pub wakes: Vec<WakeEntry>,
}

impl From<&zc::SupervisorPage> for SupervisorPage {
    fn from(p: &zc::SupervisorPage) -> Self {
        Self {
            hosts: p.hosts.iter().map(Into::into).collect(),
            needs_you: p.needs_you.iter().map(Into::into).collect(),
            done_today: task_rows(&p.done_today),
            wakes: p.wakes.iter().map(Into::into).collect(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct ApprovalView {
    pub line: String,
    pub held: bool,
    pub can_take_back: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
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

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct TaskDetail {
    pub row: TaskRow,
    pub phase: PlanPhase,
    pub plan: String,
    pub approval: Option<ApprovalView>,
    pub steps: Vec<StepView>,
}

impl From<&zc::TaskDetail> for TaskDetail {
    fn from(d: &zc::TaskDetail) -> Self {
        Self {
            row: (&d.row).into(),
            phase: d.phase.into(),
            plan: d.plan.clone(),
            approval: d.approval.as_ref().map(|a| ApprovalView {
                line: a.line.clone(),
                held: a.held,
                can_take_back: a.can_take_back,
            }),
            steps: d
                .steps
                .iter()
                .map(|s| StepView {
                    id: s.id.clone(),
                    label: s.label.clone(),
                    title: s.title.clone(),
                    notes: s.notes.clone(),
                    status: s.status.into(),
                    chat_id: s.chat_id.clone(),
                    brand: s.brand.clone(),
                    question: s.question.clone(),
                })
                .collect(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Record)]
pub struct SupervisorSettings {
    pub message_threads: bool,
    pub rebrief: bool,
    pub pause_steps: bool,
    pub start_missions: bool,
    pub heartbeat_minutes: u32,
    pub decide_for_me: bool,
    pub goal_and_spend: bool,
}

impl From<sv::SupervisorSettings> for SupervisorSettings {
    fn from(s: sv::SupervisorSettings) -> Self {
        Self {
            message_threads: s.message_threads,
            rebrief: s.rebrief,
            pause_steps: s.pause_steps,
            start_missions: s.start_missions,
            heartbeat_minutes: s.heartbeat_minutes,
            decide_for_me: s.decide_for_me,
            goal_and_spend: s.goal_and_spend,
        }
    }
}

impl From<SupervisorSettings> for sv::SupervisorSettings {
    fn from(s: SupervisorSettings) -> Self {
        Self {
            message_threads: s.message_threads,
            rebrief: s.rebrief,
            pause_steps: s.pause_steps,
            start_missions: s.start_missions,
            heartbeat_minutes: s.heartbeat_minutes,
            decide_for_me: s.decide_for_me,
            goal_and_spend: s.goal_and_spend,
        }
    }
}

#[uniffi::export]
impl CoreClient {
    pub fn tasks_page(&self) -> TasksPage {
        (&self.client.workspace().tasks).into()
    }

    pub fn supervisor_page(&self) -> SupervisorPage {
        (&self.client.workspace().supervisor).into()
    }

    pub fn task_detail(&self, task_id: String) -> Option<TaskDetail> {
        self.client.task_detail(&task_id).as_ref().map(Into::into)
    }

    pub async fn approve_plan(&self, task_id: String) -> CoreResult<()> {
        let client = self.client.clone();
        on_runtime(async move { client.approve_plan(&task_id).await }).await
    }

    pub async fn take_back_plan_approval(&self, task_id: String) -> CoreResult<()> {
        let client = self.client.clone();
        on_runtime(async move { client.take_back_plan_approval(&task_id).await }).await
    }

    pub async fn retry_step(&self, step_id: String) -> CoreResult<()> {
        let client = self.client.clone();
        on_runtime(async move { client.retry_step(&step_id).await }).await
    }

    pub async fn set_step_skipped(&self, step_id: String, skipped: bool) -> CoreResult<()> {
        let client = self.client.clone();
        on_runtime(async move { client.set_step_skipped(&step_id, skipped).await }).await
    }

    pub async fn set_task_completed(&self, task_id: String, completed: bool) -> CoreResult<()> {
        let client = self.client.clone();
        on_runtime(async move { client.set_task_completed(&task_id, completed).await }).await
    }

    pub async fn delete_task(&self, task_id: String) -> CoreResult<()> {
        let client = self.client.clone();
        on_runtime(async move { client.delete_task(&task_id).await }).await
    }

    pub async fn resolve_supervisor_item(
        &self,
        device_id: String,
        item_id: String,
        choice: String,
    ) -> CoreResult<()> {
        let client = self.client.clone();
        on_runtime(async move {
            client
                .resolve_supervisor_item(&device_id, &item_id, &choice)
                .await
        })
        .await
    }

    pub async fn supervisor_settings(&self, device_id: String) -> CoreResult<SupervisorSettings> {
        let client = self.client.clone();
        on_runtime(async move { client.supervisor_settings(&device_id).await })
            .await
            .map(Into::into)
    }

    pub async fn set_supervisor_settings(
        &self,
        device_id: String,
        before: SupervisorSettings,
        after: SupervisorSettings,
    ) -> CoreResult<SupervisorSettings> {
        let client = self.client.clone();
        on_runtime(async move {
            client
                .set_supervisor_settings(&device_id, before.into(), after.into())
                .await
        })
        .await
        .map(Into::into)
    }
}
