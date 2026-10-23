use chrono::{DateTime, Utc};

use crate::supervisor::{
    InputAsker, InterventionKind, NeedsYouInput, NeedsYouRow, PlanApproval, Raiser,
    SupervisorIntervention, SupervisorItem, is_supervisor_chat,
};
use crate::{
    Chat, ChatIndicator, PlanProgress, StepStatus, Task, TaskStatus, TaskStep, plan_of_chat,
    plan_progress, step_status, task_status,
};

pub struct StepRow<B> {
    pub step: TaskStep,
    pub status: StepStatus,
    pub indicator: Option<ChatIndicator>,
    pub brand: Option<B>,
}

pub struct Rollup<B> {
    pub lead: Option<ChatIndicator>,
    pub steps: Vec<StepRow<B>>,
    pub status: TaskStatus,
    pub progress: PlanProgress,
    pub questions: usize,
}

impl<B> Rollup<B> {
    pub fn count(&self, status: StepStatus) -> usize {
        self.steps.iter().filter(|row| row.status == status).count()
    }

    pub fn step_label(&self, step_id: &str) -> Option<String> {
        let ix = self.steps.iter().position(|row| row.step.id == step_id)?;
        Some(format!("Step {} of {}", ix + 1, self.steps.len()))
    }

    pub fn phase(&self, task: &Task) -> PlanPhase {
        let lead_working = self.lead == Some(ChatIndicator::Working);
        if task.completed_at.is_none()
            && task.plan_pending
            && !self.steps.is_empty()
            && !lead_working
        {
            PlanPhase::PlanReady
        } else if task.completed_at.is_none()
            && self.steps.is_empty()
            && is_mission(task, self)
            && matches!(
                self.lead,
                Some(ChatIndicator::Working | ChatIndicator::Idle | ChatIndicator::Completed)
            )
        {
            PlanPhase::Planning
        } else {
            PlanPhase::Status(self.status)
        }
    }

    pub fn label(&self, task: &Task) -> (&'static str, TaskStatus) {
        match self.phase(task) {
            PlanPhase::PlanReady => ("Plan ready", TaskStatus::NeedsYou),
            PlanPhase::Planning => ("Planning", TaskStatus::Working),
            PlanPhase::Status(status) => (column_title(status), status),
        }
    }

    pub fn current(&self) -> Option<usize> {
        self.steps
            .iter()
            .position(|row| !matches!(row.status, StepStatus::Done | StepStatus::Skipped))
            .or_else(|| {
                self.steps
                    .iter()
                    .rposition(|row| row.status != StepStatus::Skipped)
            })
            .or(self.steps.len().checked_sub(1))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlanPhase {
    PlanReady,
    Planning,
    Status(TaskStatus),
}

pub fn column_title(status: TaskStatus) -> &'static str {
    match status {
        TaskStatus::ToDo => "To do",
        TaskStatus::Working => "Working",
        TaskStatus::NeedsYou => "Needs you",
        TaskStatus::Done => "Done",
    }
}

pub fn rollup<'a, B>(
    task: &Task,
    chats: &[Chat],
    steps: &[TaskStep],
    needs_you: impl IntoIterator<Item = &'a NeedsYouRow>,
    indicator: impl Fn(&Chat) -> ChatIndicator,
    brand: impl Fn(&Chat) -> B,
) -> Rollup<B> {
    let lead = linked_chat(chats, task).map(&indicator);
    let steps: Vec<StepRow<B>> = steps
        .iter()
        .filter(|step| step.task_id == task.id && !step.deleted)
        .map(|step| {
            let chat = step_chat(chats, step);
            let shown = chat.map(&indicator);
            StepRow {
                step: step.clone(),
                status: step_status(step, chat, shown),
                indicator: shown,
                brand: chat.map(&brand),
            }
        })
        .collect();
    let statuses: Vec<StepStatus> = steps.iter().map(|row| row.status).collect();
    let questions = needs_you
        .into_iter()
        .filter(|row| row.task_id() == Some(task.id.as_str()))
        .count();
    Rollup {
        lead,
        status: task_status(task, lead, &statuses),
        progress: plan_progress(&statuses),
        steps,
        questions,
    }
}

/// A link whose chat is gone (deleted) reads as no link: the task can start again.
pub fn linked_chat<'a>(chats: &'a [Chat], task: &Task) -> Option<&'a Chat> {
    let id = task.chat_id.as_deref()?;
    chats.iter().find(|chat| chat.id == id)
}

pub fn is_mission<B>(task: &Task, plan: &Rollup<B>) -> bool {
    task.has_plan() || task.plan_pending || !plan.steps.is_empty()
}

pub fn step_chat<'a>(chats: &'a [Chat], step: &TaskStep) -> Option<&'a Chat> {
    let id = step.chat_id.as_deref()?;
    chats.iter().find(|chat| chat.id == id)
}

/// The open plan, else the one that finished last, so a done lead keeps its strip.
pub fn plan_led_by<'a>(tasks: &'a [Task], chat_id: &str) -> Option<&'a Task> {
    plan_of_chat(tasks, chat_id).or_else(|| {
        tasks
            .iter()
            .filter(|task| {
                !task.deleted && task.has_plan() && task.chat_id.as_deref() == Some(chat_id)
            })
            .max_by_key(|task| task.completed_at)
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MissionRole {
    Lead,
    Step { step_id: String, label: String },
}

impl MissionRole {
    pub fn label(&self) -> String {
        match self {
            Self::Lead => "Lead".into(),
            Self::Step { label, .. } => label.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct MissionThread {
    pub task_id: String,
    pub title: String,
    pub role: MissionRole,
    pub progress: PlanProgress,
}

pub fn mission_thread<B>(
    tasks: &[Task],
    steps: &[TaskStep],
    chat_id: &str,
    plan_of: impl FnOnce(&Task) -> Rollup<B>,
) -> Option<MissionThread> {
    let mission = |task: &&Task| task.has_plan() && !task.deleted;
    let (task, step_id) = match plan_led_by(tasks, chat_id) {
        Some(task) => (task, None),
        None => steps
            .iter()
            .filter(|step| !step.deleted && step.chat_id.as_deref() == Some(chat_id))
            .find_map(|step| {
                let task = tasks
                    .iter()
                    .filter(mission)
                    .find(|task| task.id == step.task_id)?;
                Some((task, Some(step.id.clone())))
            })?,
    };
    let plan = plan_of(task);
    let role = match step_id {
        None => MissionRole::Lead,
        Some(step_id) => MissionRole::Step {
            label: plan.step_label(&step_id)?,
            step_id,
        },
    };
    Some(MissionThread {
        task_id: task.id.clone(),
        title: task.title.clone(),
        role,
        progress: plan.progress,
    })
}

#[derive(Debug, Clone, PartialEq)]
pub struct MissionRow {
    pub task_id: String,
    pub title: String,
    pub progress: PlanProgress,
    pub status: TaskStatus,
    pub label: String,
    pub current: Option<String>,
}

pub fn split_tasks(tasks: &[Task]) -> (Vec<&Task>, Vec<&Task>) {
    let (mut open, mut done): (Vec<&Task>, Vec<&Task>) = tasks
        .iter()
        .filter(|task| !task.deleted)
        .partition(|task| task.completed_at.is_none());
    open.sort_by(|a, b| {
        (&a.order_key, a.created_at, &a.id).cmp(&(&b.order_key, b.created_at, &b.id))
    });
    done.sort_by(|a, b| b.completed_at.cmp(&a.completed_at).then(a.id.cmp(&b.id)));
    (open, done)
}

pub fn open_missions(tasks: &[Task]) -> Vec<&Task> {
    let mut open: Vec<&Task> = tasks
        .iter()
        .filter(|task| task.has_plan() && !task.deleted && task.completed_at.is_none())
        .collect();
    open.sort_by(|a, b| {
        (&a.order_key, a.created_at, &a.id).cmp(&(&b.order_key, b.created_at, &b.id))
    });
    open
}

pub fn missions_done_on<'a, Tz: chrono::TimeZone>(
    tasks: &'a [Task],
    day: chrono::NaiveDate,
    zone: &Tz,
) -> Vec<&'a Task> {
    let mut done: Vec<&'a Task> = tasks
        .iter()
        .filter(|task| {
            task.has_plan()
                && !task.deleted
                && task
                    .completed_at
                    .is_some_and(|at| at.with_timezone(zone).date_naive() == day)
        })
        .collect();
    done.sort_by_key(|task| std::cmp::Reverse(task.completed_at));
    done
}

pub fn mission_row<B>(task: &Task, plan: &Rollup<B>, held: bool) -> MissionRow {
    let (word, status) = plan.label(task);
    let current = plan.current();
    let label = match plan.phase(task) {
        PlanPhase::PlanReady => "Needs you · plan review".to_owned(),
        _ if held => HELD_APPROVAL.to_owned(),
        PlanPhase::Planning => "Planning".to_owned(),
        PlanPhase::Status(TaskStatus::NeedsYou) => plan
            .steps
            .iter()
            .position(|row| matches!(row.status, StepStatus::NeedsYou | StepStatus::Failed))
            .map(|index| format!("Needs you · step {}", index + 1))
            .unwrap_or_else(|| word.to_owned()),
        _ => match current {
            Some(ix) => format!("Step {}/{} · {word}", ix + 1, plan.steps.len()),
            None => word.to_owned(),
        },
    };
    MissionRow {
        task_id: task.id.clone(),
        title: task.title.clone(),
        progress: plan.progress,
        status: if held { TaskStatus::Working } else { status },
        label,
        current: current.map(|ix| {
            let row = &plan.steps[ix];
            format!(
                "Step {} of {} · {}",
                ix + 1,
                plan.steps.len(),
                row.step.title
            )
        }),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApprovalState {
    Held { secs: i64 },
    Sent,
    Started,
    TakenBack,
}

impl ApprovalState {
    pub fn line(self) -> String {
        match self {
            Self::Held { secs } => format!(
                "Approved by the Supervisor · starts in {}:{:02}",
                secs / 60,
                secs % 60
            ),
            Self::Sent => "Approved by the Supervisor · no step has started yet".into(),
            Self::Started => "Approved by the Supervisor · steps already started. To change course, edit the plan on the mission page or message the lead.".into(),
            Self::TakenBack => "You took this approval back · the plan went back to review".into(),
        }
    }

    pub fn can_take_back(self) -> bool {
        matches!(self, Self::Held { .. } | Self::Sent)
    }
}

pub const HELD_APPROVAL: &str = "Plan approved · can take back";

pub fn approval_state(
    row: &SupervisorIntervention,
    steps: &[TaskStep],
    now: DateTime<Utc>,
) -> Option<ApprovalState> {
    let task_id = row.task_id.as_deref()?;
    let started = || {
        steps.iter().any(|step| {
            step.task_id == task_id
                && !step.deleted
                && (step.chat_id.is_some() || step.outcome.is_some())
        })
    };
    Some(match row.approval.as_ref()? {
        PlanApproval::TakenBack { .. } => ApprovalState::TakenBack,
        _ if started() => ApprovalState::Started,
        PlanApproval::Held { starts_at } => ApprovalState::Held {
            secs: ((*starts_at - now).num_milliseconds().max(0) + 999) / 1000,
        },
        PlanApproval::Sent => ApprovalState::Sent,
    })
}

/// The Supervisor's newest approval of the mission's plan, as the person can still act on it.
pub fn plan_approval(
    interventions: &[SupervisorIntervention],
    steps: &[TaskStep],
    task_id: &str,
    now: DateTime<Utc>,
) -> Option<ApprovalState> {
    let row = interventions.iter().find(|row| {
        row.kind == InterventionKind::Approve && row.task_id.as_deref() == Some(task_id)
    })?;
    approval_state(row, steps, now)
}

pub struct Names<'a, F> {
    pub chats: &'a [Chat],
    pub tasks: &'a [Task],
    pub steps: &'a [TaskStep],
    pub plan_of: F,
}

impl<'a, B, F: Fn(&Task) -> Rollup<B>> Names<'a, F> {
    pub fn mission_title(&self, task_id: Option<&str>) -> Option<String> {
        let task_id = task_id?;
        self.tasks
            .iter()
            .find(|task| task.id == task_id)
            .map(|task| task.title.clone())
    }

    pub fn mission_thread(&self, chat_id: &str) -> Option<MissionThread> {
        mission_thread(self.tasks, self.steps, chat_id, |task| (self.plan_of)(task))
    }

    pub fn plan_thread(&self, chat_id: &str) -> Option<(String, String)> {
        let thread = self.mission_thread(chat_id)?;
        let who = match thread.role {
            MissionRole::Lead => "Lead".to_owned(),
            MissionRole::Step { step_id, label } => {
                let step = self.steps.iter().find(|step| step.id == step_id)?;
                format!("{label} · {}", step.title)
            }
        };
        Some((who, thread.title))
    }

    pub fn chat_name(&self, chat: &Chat) -> String {
        crate::view::single_line(&chat.display_title())
    }

    pub fn thread_name(&self, chat_id: &str) -> String {
        if is_supervisor_chat(chat_id) {
            return "Supervisor".into();
        }
        if let Some((who, mission)) = self.plan_thread(chat_id) {
            return format!("{who} · {mission}");
        }
        self.chats
            .iter()
            .find(|chat| chat.id == chat_id)
            .map(|chat| self.chat_name(chat))
            .unwrap_or_else(|| "a thread".into())
    }

    pub fn tree_root(&self, chat_id: &str) -> Option<&'a Chat> {
        let chat = self.chats.iter().find(|chat| chat.id == chat_id)?;
        crate::tree_path(chat, self.chats, self.tasks, self.steps)
            .last()
            .copied()
    }

    pub fn asker_name(&self, input: &NeedsYouInput) -> String {
        if matches!(input.asker, InputAsker::Step | InputAsker::Lead)
            && let Some((who, mission)) = self.plan_thread(&input.chat_id)
        {
            return format!("{who} · {mission}");
        }
        let mission = self.mission_title(input.task_id.as_deref());
        let who = match input.asker {
            InputAsker::Supervisor => return "Supervisor".into(),
            InputAsker::Thread => self.thread_asker(&input.chat_id),
            InputAsker::Step => "A step".into(),
            InputAsker::Lead => "Lead".into(),
        };
        match mission {
            Some(mission) => format!("{who} · {mission}"),
            None => who,
        }
    }

    pub fn thread_asker(&self, chat_id: &str) -> String {
        let name = self.thread_name(chat_id);
        match self
            .tree_root(chat_id)
            .filter(|root| root.id != chat_id)
            .map(|root| self.chat_name(root))
        {
            Some(root) => format!("{name} · {root}"),
            None => name,
        }
    }

    pub fn raiser_name(&self, item: &SupervisorItem) -> String {
        match item.raised_by {
            Raiser::Supervisor => "Supervisor".into(),
            Raiser::Lead => match self.mission_title(item.task_id.as_deref()) {
                Some(mission) => format!("Lead · {mission}"),
                None => "Lead".into(),
            },
        }
    }
}
