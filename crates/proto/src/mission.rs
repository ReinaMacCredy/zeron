use crate::{
    Chat, ChatIndicator, NeedsYou, PlanProgress, Task, TaskStatus, TaskStep, ThreadState,
    plan_progress, step_state, task_status,
};

pub struct StepRow<B> {
    pub step: TaskStep,
    pub status: ThreadState,
    pub indicator: Option<ChatIndicator>,
    pub brand: Option<B>,
}

pub struct Rollup<B> {
    pub lead: Option<ChatIndicator>,
    pub steps: Vec<StepRow<B>>,
    pub status: TaskStatus,
    pub needs: Option<NeedsYou>,
    pub progress: PlanProgress,
    pub planning: bool,
    pub pending: bool,
}

impl<B> Rollup<B> {
    pub fn phase(&self, task: &Task) -> PlanPhase {
        let lead_working = self.lead == Some(ChatIndicator::Working);
        if task.completed_at.is_none() && self.pending && !self.steps.is_empty() && !lead_working {
            PlanPhase::PlanReady
        } else if task.completed_at.is_none()
            && self.steps.is_empty()
            && is_mission(self)
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
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlanPhase {
    PlanReady,
    Planning,
    Status(TaskStatus),
}

pub const COLUMNS: [TaskStatus; 4] = [
    TaskStatus::ToDo,
    TaskStatus::Working,
    TaskStatus::NeedsYou,
    TaskStatus::Done,
];

/// Only Done is a drop target that writes; the other columns follow the linked chat.
pub fn drop_completes(from: TaskStatus, to: TaskStatus) -> Option<bool> {
    match (from, to) {
        (TaskStatus::Done, TaskStatus::ToDo) => Some(false),
        (from, TaskStatus::Done) if from != TaskStatus::Done => Some(true),
        _ => None,
    }
}

pub const TASKS_VIEWS: [&str; 2] = ["List", "Kanban"];

pub fn column_title(status: TaskStatus) -> &'static str {
    match status {
        TaskStatus::ToDo => "To do",
        TaskStatus::Working => "Working",
        TaskStatus::NeedsYou => "Needs you",
        TaskStatus::Done => "Done",
    }
}

pub fn rollup<B>(
    task: &Task,
    chats: &[Chat],
    steps: &[TaskStep],
    indicator: impl Fn(&Chat) -> ChatIndicator,
    brand: impl Fn(&Chat) -> B,
) -> Rollup<B> {
    let lead_chat = linked_chat(chats, task);
    let lead = lead_chat.map(&indicator);
    let steps: Vec<StepRow<B>> = steps
        .iter()
        .filter(|step| !step.deleted && lead_chat.is_some_and(|lead| lead.id == step.owner_chat_id))
        .map(|step| {
            let chat = step
                .chat_id
                .as_deref()
                .and_then(|id| chats.iter().find(|chat| chat.id == id));
            let shown = chat.map(&indicator);
            StepRow {
                step: step.clone(),
                status: step_state(step, chat, shown),
                indicator: shown,
                brand: chat.map(&brand),
            }
        })
        .collect();
    let statuses: Vec<ThreadState> = steps.iter().map(|row| row.status).collect();
    Rollup {
        lead,
        status: task_status(task, lead_chat, lead, &statuses),
        needs: crate::needs_you(task, lead_chat, lead, &statuses),
        progress: plan_progress(&statuses),
        planning: lead_chat.is_some_and(Chat::has_plan),
        pending: lead_chat.is_some_and(|chat| chat.plan_pending),
        steps,
    }
}

/// A link whose chat is gone (deleted) reads as no link: the task can start again.
pub fn linked_chat<'a>(chats: &'a [Chat], task: &Task) -> Option<&'a Chat> {
    let id = task.chat_id.as_deref()?;
    chats.iter().find(|chat| chat.id == id)
}

pub fn is_mission<B>(plan: &Rollup<B>) -> bool {
    plan.planning || !plan.steps.is_empty()
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

pub struct IntroPage {
    pub title: &'static str,
    pub body: &'static str,
}

pub const INTRO_PAGES: [IntroPage; 4] = [
    IntroPage {
        title: "Start with a goal",
        body: "Press Shift+Tab in the composer, or turn on Plan first on a task\u{2019}s card, and describe what you want done. Add up to five chats for the lead to read before it plans.",
    },
    IntroPage {
        title: "Approve the plan",
        body: "A lead agent turns your goal into steps you can edit, reorder and add images to. By default nothing runs until you click Approve and run.",
    },
    IntroPage {
        title: "Watch every step",
        body: "Each step runs on its own thread, side by side when steps don\u{2019}t depend on each other. The tree on the lead\u{2019}s thread follows them live, and the mission page adds each step\u{2019}s token count.",
    },
    IntroPage {
        title: "A lead and a Supervisor watch",
        body: "The lead reviews each step\u{2019}s work when it finishes and sends it back if something is missing. The Supervisor gathers what needs you into one sheet per mission, and one Send answers it all.",
    },
];

pub const INTRO_START_TOUCH: &str = "Tap Start mission in the composer, or Plan first on a task\u{2019}s sheet, and describe what you want done. Add up to five chats for the lead to read before it plans.";
pub const INTRO_NEXT: &str = "Next";
pub const INTRO_DISMISS: &str = "Not now";
pub const INTRO_TRY: &str = "Try Mission Control";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_done_and_back_to_to_do_are_writing_drops() {
        use TaskStatus::*;
        for from in [ToDo, Working, NeedsYou] {
            assert_eq!(drop_completes(from, Done), Some(true), "{from:?}");
            assert_eq!(drop_completes(from, ToDo), None, "{from:?}");
        }
        assert_eq!(drop_completes(Done, ToDo), Some(false));
        assert_eq!(drop_completes(Done, Done), None);
        for to in [Working, NeedsYou] {
            for from in COLUMNS {
                assert_eq!(drop_completes(from, to), None, "{from:?} -> {to:?}");
            }
        }
    }
}
