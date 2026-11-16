use zeron_proto::{SessionStatus, TaskStep};

use super::delegation::failure;
use super::*;

fn live(status: Option<SessionStatus>) -> bool {
    matches!(
        status,
        Some(SessionStatus::Working | SessionStatus::AwaitingInput)
    )
}

fn step_threads(step: &TaskStep) -> impl Iterator<Item = &str> {
    step.chat_id.iter().chain(&step.earlier).map(String::as_str)
}

pub(super) fn marked_step_threads(workspace: &WorkspaceHost) -> HashSet<String> {
    let steps = workspace.watch_task_steps();
    let steps = steps.borrow();
    let open: HashSet<&str> = steps
        .iter()
        .filter(|step| step.outcome.is_none())
        .flat_map(step_threads)
        .collect();
    steps
        .iter()
        .flat_map(step_threads)
        .filter(|thread| !open.contains(thread))
        .map(str::to_owned)
        .collect()
}

fn unapproved() -> EngineError {
    failure(
        "plan_pending",
        "The person has not approved the plan yet. Stop and wait for the approval message.",
    )
}

impl DocHost {
    pub(crate) fn refuse_unapproved_mission(&self, owner: &str) -> Result<(), EngineError> {
        let pending = self
            .workspace()
            .and_then(|workspace| workspace.chat(owner).ok().flatten())
            .is_some_and(|chat| chat.plan_pending && chat.mission().is_some());
        if pending { Err(unapproved()) } else { Ok(()) }
    }

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
            .ok_or_else(|| refused("That plan step no longer exists"))?;
        if step.owner_chat_id != lead {
            return Err(refused("Only the chat whose plan holds this step runs it"));
        }
        if workspace.chat(lead)?.is_some_and(|chat| chat.plan_pending) {
            return Err(unapproved());
        }
        if step.chat_id.is_none()
            && workspace
                .chat(lead)?
                .is_some_and(|chat| !chat.in_current_plan(&step))
        {
            return Err(refused(
                "This step belongs to a closed plan and never ran; write it into the current plan first",
            ));
        }
        if step.outcome.is_some() {
            return Err(refused(
                "This step is already marked done or skipped; reopen it with the plan tool first",
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
}
