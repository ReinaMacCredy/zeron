use zeron_proto::{SessionStatus, TaskStep};

use super::delegation::failure;
use super::*;

fn live(status: Option<SessionStatus>) -> bool {
    matches!(
        status,
        Some(SessionStatus::Working | SessionStatus::AwaitingInput)
    )
}

fn open_mission_lead(workspace: &WorkspaceHost, task_id: &str) -> Option<String> {
    workspace
        .task(task_id)
        .filter(|task| task.completed_at.is_none())
        .and_then(|task| task.chat_id)
}

pub(super) fn marked_step_threads(workspace: &WorkspaceHost) -> HashSet<String> {
    let steps = workspace.watch_task_steps();
    let steps = steps.borrow();
    let open: HashSet<&str> = steps
        .iter()
        .filter(|step| step.outcome.is_none())
        .filter_map(|step| step.chat_id.as_deref())
        .collect();
    steps
        .iter()
        .filter_map(|step| step.chat_id.as_deref())
        .filter(|thread| !open.contains(thread))
        .map(str::to_owned)
        .collect()
}

impl DocHost {
    /// Call under `delegation_lock`, so two starts cannot bind one step.
    pub(super) fn runnable_step(
        &self,
        workspace: &WorkspaceHost,
        lead: &str,
        step_id: &str,
    ) -> Result<TaskStep, EngineError> {
        let refused = |message: &str| failure("invalid_request", message);
        let step = workspace
            .task_step(step_id)
            .ok_or_else(|| refused("That task step no longer exists"))?;
        let task = workspace
            .task(&step.task_id)
            .ok_or_else(|| refused("That step's task no longer exists"))?;
        if task.chat_id.as_deref() != Some(lead) {
            return Err(refused("Only the task's lead runs its steps"));
        }
        if task.plan_pending {
            return Err(failure(
                "plan_pending",
                "The person has not approved the plan yet. Stop and wait for the approval message.",
            ));
        }
        if step.outcome.is_some() {
            return Err(refused(
                "This step is already marked done or skipped; reopen it with the tasks tool first",
            ));
        }
        let running = |thread: &str| {
            live(
                self.sessions()
                    .and_then(|sessions| sessions.session_status(thread))
                    .map(|session| session.status),
            ) || workspace
                .chat(thread)
                .ok()
                .flatten()
                .and_then(|chat| chat.delegation)
                .is_some_and(|delegation| delegation.is_running())
        };
        if step.chat_id.as_deref().is_some_and(running) {
            return Err(refused(
                "This step's thread is still running; send it a message with send_message_to_thread instead",
            ));
        }
        Ok(step)
    }

    pub(super) fn step_lead_of(&self, thread: &str) -> Option<String> {
        let workspace = self.workspace()?;
        let task_id = workspace
            .watch_task_steps()
            .borrow()
            .iter()
            .find(|step| step.outcome.is_none() && step.chat_id.as_deref() == Some(thread))
            .map(|step| step.task_id.clone())?;
        open_mission_lead(workspace, &task_id)
    }
}
