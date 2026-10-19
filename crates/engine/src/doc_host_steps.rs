use zeron_proto::{Delegation, SessionStatus, StepTurn, StepTurnEnd, TaskStep, TaskStepPatch};

use super::delegation::{agent_response, failure, handback_settle, handback_text};
use super::*;
use crate::registry::DelegationSettings;

fn live(status: Option<SessionStatus>) -> bool {
    matches!(
        status,
        Some(SessionStatus::Working | SessionStatus::AwaitingInput)
    )
}

fn turn_end(
    status: SessionStatus,
    was_active: bool,
    completed: Option<&str>,
    recorded: Option<&StepTurn>,
) -> Option<StepTurn> {
    let failed_before = recorded.is_some_and(|turn| turn.ended == StepTurnEnd::Failed);
    let ended = if status == SessionStatus::Errored && (was_active || !failed_before) {
        StepTurnEnd::Failed
    } else if completed.is_some() && completed != recorded.and_then(|turn| turn.turn.as_deref()) {
        StepTurnEnd::Completed
    } else if was_active {
        StepTurnEnd::Stopped
    } else {
        return None;
    };
    Some(StepTurn {
        ended,
        turn: completed.map(str::to_owned),
    })
}

struct StepHandback {
    id: String,
    title: String,
    thread: String,
    ended: StepTurnEnd,
    reply: Option<String>,
}

fn step_wake_text(steps: &[StepHandback]) -> String {
    let next = match steps {
        [one] if one.ended == StepTurnEnd::Failed => format!(
            "Read it with read_thread, then send the thread a fix with send_message_to_thread, run the step again with create_thread and taskStepId {}, or mark it skipped with the tasks tool.",
            one.id
        ),
        [_] => "Read it with read_thread, then mark the step done or skipped with the tasks tool, action \"step\", or tell the thread what is still missing with send_message_to_thread.".to_string(),
        many => {
            let mut next = "Read them with read_thread, then mark each step done or skipped with the tasks tool, action \"step\", or tell its thread what is still missing with send_message_to_thread.".to_string();
            if many.iter().any(|step| step.ended == StepTurnEnd::Failed) {
                next.push_str(" A failed step can also run again with create_thread and its taskStepId.");
            }
            next
        }
    };
    let children = steps.iter().map(|step| {
        let what = match step.ended {
            StepTurnEnd::Failed => "failed",
            _ => "finished a turn",
        };
        (
            format!(
                "Step \"{}\" (taskStepId {}) {what} on its thread {}.",
                zeron_proto::view::single_line(&step.title),
                step.id,
                step.thread
            ),
            step.reply
                .as_deref()
                .and_then(|reply| agent_response(reply, "read_thread")),
        )
    });
    handback_text(children, next)
}

fn step_lead_to_wake(workspace: &WorkspaceHost, step: &TaskStep) -> Option<String> {
    let lead = workspace
        .task(&step.task_id)
        .filter(|task| task.completed_at.is_none())
        .and_then(|task| task.chat_id)?;
    workspace
        .chat(&lead)
        .ok()
        .flatten()
        .is_some_and(|chat| !chat.archived)
        .then_some(lead)
}

impl DocHost {
    /// Call under `delegation_lock`, so a parallel run cannot slip past the count.
    pub(super) fn runnable_step(
        &self,
        workspace: &WorkspaceHost,
        lead: &str,
        step_id: &str,
        limits: DelegationSettings,
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
        if let Some(hold) = self.supervisor().and_then(|store| store.hold_of(step_id)) {
            return Err(failure(
                "step_paused",
                format!(
                    "The Supervisor paused this step because {} is contended ({}). Run it again only after the Supervisor's resume message.",
                    hold.resource, hold.reason
                ),
            ));
        }
        let status = |chat_id: &str| {
            self.sessions()
                .and_then(|sessions| sessions.session_status(chat_id))
                .map(|session| session.status)
        };
        if step
            .chat_id
            .as_deref()
            .is_some_and(|bound| live(status(bound)))
        {
            return Err(refused(
                "This step's thread is still running; send it a message with send_message_to_thread instead",
            ));
        }
        let chats = workspace.read_chats()?;
        let mut led: Vec<String> = workspace
            .watch_tasks()
            .borrow()
            .iter()
            .filter(|task| task.chat_id.as_deref() == Some(lead))
            .map(|task| task.id.clone())
            .collect();
        if !led.contains(&task.id) {
            led.push(task.id.clone());
        }
        let running = led
            .iter()
            .flat_map(|task_id| workspace.task_steps_of(task_id))
            .filter(|step| step.outcome.is_none())
            .filter(|step| {
                let Some(thread) = step
                    .chat_id
                    .as_deref()
                    .and_then(|id| chats.iter().find(|chat| chat.id == id))
                else {
                    return false;
                };
                live(status(&thread.id))
                    || thread
                        .delegation
                        .as_ref()
                        .map_or(step.turn.is_none(), Delegation::is_running)
            })
            .count();
        if running >= limits.max_running_children as usize {
            return Err(failure(
                "child_limit",
                format!("this chat already runs {running} step threads, the limit set in Settings"),
            ));
        }
        Ok(step)
    }

    pub(super) fn bound_step_threads(&self, workspace: &WorkspaceHost) -> HashSet<String> {
        workspace
            .watch_task_steps()
            .borrow()
            .iter()
            .filter_map(|step| step.chat_id.clone())
            .collect()
    }

    pub(super) fn step_lead_of(&self, thread: &str) -> Option<String> {
        let workspace = self.workspace()?;
        let task_id = workspace
            .watch_task_steps()
            .borrow()
            .iter()
            .find(|step| step.outcome.is_none() && step.chat_id.as_deref() == Some(thread))
            .map(|step| step.task_id.clone())?;
        workspace.task(&task_id)?.chat_id
    }

    /// Steps whose thread a delegate_task child holds settle through their delegation.
    fn local_step_thread(&self, workspace: &WorkspaceHost, thread: &str) -> bool {
        workspace.chat(thread).ok().flatten().is_some_and(|chat| {
            chat.device_id == self.inner.config.device_id && chat.delegation.is_none()
        })
    }

    pub(super) async fn settle_task_steps(&self) {
        let (Some(workspace), Some(sessions)) = (self.workspace(), self.sessions()) else {
            return;
        };
        let open: Vec<String> = workspace
            .watch_task_steps()
            .borrow()
            .iter()
            .filter(|step| step.outcome.is_none() && step.chat_id.is_some())
            .map(|step| step.id.clone())
            .collect();
        if open.is_empty() {
            return;
        }
        let mut wakes = Vec::new();
        {
            let _settling = self.inner.delegation_lock.lock().await;
            for id in open {
                let Some(step) = workspace.task_step(&id).filter(|s| s.outcome.is_none()) else {
                    continue;
                };
                let Some(thread) = step.chat_id.clone() else {
                    continue;
                };
                let Some(session) = sessions.session_status(&thread) else {
                    continue;
                };
                if !self.local_step_thread(workspace, &thread) {
                    continue;
                }
                if live(Some(session.status)) {
                    lock(&self.inner.active_step_threads).insert(thread);
                    continue;
                }
                let was_active = lock(&self.inner.active_step_threads).remove(&thread);
                let Some(turn) = turn_end(
                    session.status,
                    was_active,
                    session.last_completed_turn.as_deref(),
                    step.turn.as_ref(),
                ) else {
                    continue;
                };
                if step.turn.as_ref() != Some(&turn) {
                    let patch = TaskStepPatch {
                        turn: Some(Some(turn.clone())),
                        ..Default::default()
                    };
                    if let Err(err) = workspace.update_task_step(&step.id, &patch) {
                        tracing::warn!(step = %step.id, error = %err, "recording a step's turn failed");
                        continue;
                    }
                }
                if turn.ended != StepTurnEnd::Stopped {
                    wakes.push((step, thread, turn.ended));
                }
            }
        }
        let mut leads: Vec<(String, Vec<String>)> = Vec::new();
        for (step, thread, ended) in wakes {
            self.supervisor_step_ended(&step, &thread, ended);
            let Some(lead) = step_lead_to_wake(workspace, &step) else {
                continue;
            };
            match leads.iter_mut().find(|(id, _)| *id == lead) {
                Some((_, steps)) => steps.push(step.id),
                None => leads.push((lead, vec![step.id])),
            }
        }
        for (lead, steps) in leads {
            if let Err(err) = self.offer_step_wake(workspace, &lead, steps).await {
                tracing::warn!(chat = %lead, error = %err, "waking a step's lead failed");
            }
        }
    }

    fn step_wake(
        &self,
        workspace: &WorkspaceHost,
        step_ids: &[String],
    ) -> Option<(String, String)> {
        let steps: Vec<StepHandback> = step_ids
            .iter()
            .filter_map(|id| {
                let step = workspace.task_step(id)?;
                let thread = step.chat_id?;
                Some(StepHandback {
                    reply: self.last_reply(&thread),
                    ended: step.turn.map_or(StepTurnEnd::Completed, |turn| turn.ended),
                    id: step.id,
                    title: step.title,
                    thread,
                })
            })
            .collect();
        let first = steps.first()?.thread.clone();
        Some((step_wake_text(&steps), first))
    }

    async fn offer_step_wake(
        &self,
        workspace: &WorkspaceHost,
        lead: &str,
        step_ids: Vec<String>,
    ) -> Result<(), EngineError> {
        let handle = self.open(lead)?;
        {
            let _drain = handle.drain_lock.lock().await;
            let waiting = handle
                .doc
                .read_queue()?
                .into_iter()
                .enumerate()
                .find(|(_, row)| !row.task_steps.is_empty() && row.delivery_gate.is_none());
            if let Some((index, mut row)) = waiting {
                for id in &step_ids {
                    if !row.task_steps.contains(id) {
                        row.task_steps.push(id.clone());
                    }
                }
                // wait_threads takes rows without the drain lock, so the row may be gone.
                if let Some((text, _)) = self.step_wake(workspace, &row.task_steps)
                    && handle.doc.take_queued(&row.id)?.is_some()
                {
                    row.text = text;
                    handle.doc.insert_queued(index, &row)?;
                    handle.publish_queue();
                    self.save_snapshot(&handle);
                    return Ok(());
                }
            }
        }
        let Some((text, from)) = self.step_wake(workspace, &step_ids) else {
            return Ok(());
        };
        let (handle, row_id) = self.push_thread_row(lead, Some(&from), &text, step_ids)?;
        let host = self.clone();
        self.spawn_worker(async move {
            tokio::time::sleep(handback_settle()).await;
            host.send_thread_row(&handle, &row_id).await;
        });
        Ok(())
    }

    /// Runs after `recover_stale`: a step thread nothing revives lost its turn with the previous engine.
    pub(super) async fn settle_task_steps_after_boot(&self) {
        let (Some(workspace), Some(sessions)) = (self.workspace(), self.sessions()) else {
            return;
        };
        let _settling = self.inner.delegation_lock.lock().await;
        let open: Vec<TaskStep> = workspace
            .watch_task_steps()
            .borrow()
            .iter()
            .filter(|step| step.outcome.is_none() && step.turn.is_none())
            .cloned()
            .collect();
        for step in open {
            let Some(thread) = step.chat_id.as_deref() else {
                continue;
            };
            if !self.local_step_thread(workspace, thread)
                || sessions.is_reviving(thread)
                || live(sessions.session_status(thread).map(|s| s.status))
            {
                continue;
            }
            let queued = self
                .open(thread)
                .ok()
                .and_then(|handle| handle.doc.read_queue().ok())
                .is_none_or(|queue| !queue.is_empty());
            if queued {
                continue;
            }
            let patch = TaskStepPatch {
                turn: Some(Some(StepTurn {
                    ended: StepTurnEnd::Stopped,
                    turn: None,
                })),
                ..Default::default()
            };
            if let Err(err) = workspace.update_task_step(&step.id, &patch) {
                tracing::warn!(step = %step.id, error = %err, "settling a step after boot failed");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn recorded(ended: StepTurnEnd, turn: Option<&str>) -> StepTurn {
        StepTurn {
            ended,
            turn: turn.map(str::to_owned),
        }
    }

    #[test]
    fn a_step_thread_turn_ends_once_per_change() {
        use SessionStatus::{Errored, Idle};
        use StepTurnEnd::{Completed, Failed, Stopped};
        let done_m1 = recorded(Completed, Some("m1"));
        let failed = recorded(Failed, Some("m1"));
        let cases: [(
            SessionStatus,
            bool,
            Option<&str>,
            Option<&StepTurn>,
            Option<StepTurnEnd>,
        ); 11] = [
            (Errored, true, None, None, Some(Failed)),
            (Errored, false, None, None, Some(Failed)),
            (Errored, true, Some("m1"), Some(&failed), Some(Failed)),
            (Errored, false, Some("m1"), Some(&failed), None),
            (Idle, true, Some("m1"), None, Some(Completed)),
            (Idle, false, Some("m1"), None, Some(Completed)),
            (Idle, false, Some("m2"), Some(&done_m1), Some(Completed)),
            (Idle, false, Some("m1"), Some(&done_m1), None),
            (Idle, true, Some("m1"), Some(&done_m1), Some(Stopped)),
            (Idle, true, None, None, Some(Stopped)),
            (Idle, false, None, None, None),
        ];
        for (status, was_active, completed, before, ended) in cases {
            let turn = turn_end(status, was_active, completed, before);
            assert_eq!(
                turn.as_ref().map(|turn| turn.ended),
                ended,
                "{status:?} active={was_active} completed={completed:?} recorded={before:?}"
            );
            if let Some(turn) = turn {
                assert_eq!(turn.turn.as_deref(), completed);
            }
        }
    }
}
