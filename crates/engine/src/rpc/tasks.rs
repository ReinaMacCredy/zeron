use super::*;
use zeron_proto::{SandboxLevel, Task, TaskMode, TaskPatch, TaskStep, TaskStepPatch};

fn refused(message: &str) -> RpcError {
    RpcError::Failed(message.into())
}

fn failed(error: crate::EngineError) -> RpcError {
    RpcError::Failed(error.to_string())
}

fn task_prompt(task: &Task) -> String {
    match task.notes.trim() {
        "" => task.title.clone(),
        notes => format!("{}\n\n{notes}", task.title),
    }
}

#[derive(Debug, Clone, Copy)]
pub(super) struct Lead {
    pub approve_plan: bool,
    pub same_agents: bool,
}

fn lead_brief(task_id: &str, lead: Lead) -> String {
    let plan = format!(
        "Write the plan with the tasks tool: action \"plan\", taskId \"{task_id}\", steps [{{title, notes}}]. Keep each step small enough for one agent."
    );
    let mut lines = vec![plan.as_str()];
    if lead.approve_plan {
        lines.push(
            "Then stop. The person reviews the plan in Tasks and approves it; you get a message when it is approved.",
        );
    }
    lines.push(
        "Run each step on its own thread with create_thread: prompt set to what the step must do, taskStepId set to the step's id, title optional. Steps that do not depend on each other can run at the same time.",
    );
    lines.push(if lead.same_agents {
        "Run every step on your own harness and model: do not pass harness or model to create_thread."
    } else {
        "Pick the harness and model for each step; orchestrator_capabilities lists them."
    });
    lines.push(
        "Zeron messages you when a step's thread finishes a turn or fails. Read the thread with read_thread, then mark the step done or skipped with the tasks tool, action \"step\", send the thread a fix with send_message_to_thread, or start the step over with create_thread and the same taskStepId.",
    );
    lines.push(
        "When every step is done, check the whole result, tell the person what was built, and mark the task completed with the tasks tool, action \"update\", completed true.",
    );
    let steps: Vec<String> = lines
        .iter()
        .enumerate()
        .map(|(n, line)| format!("{}. {line}", n + 1))
        .collect();
    format!(
        "You lead this task (task id {task_id}). Break it into steps and run each step on its own thread.\n\n{}",
        steps.join("\n")
    )
}

struct Starting<'a> {
    tasks: &'a std::sync::Mutex<std::collections::HashSet<String>>,
    task_id: String,
}

impl Drop for Starting<'_> {
    fn drop(&mut self) {
        self.tasks
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(&self.task_id);
    }
}

impl EngineRpc {
    pub(super) fn create_task(
        &self,
        task_id: String,
        title: String,
        notes: String,
        space_id: Option<String>,
        due_on: Option<String>,
        order_key: Option<String>,
    ) -> Result<(), RpcError> {
        let now = chrono::Utc::now();
        self.workspace
            .create_task(&Task {
                id: task_id,
                space_id,
                title,
                notes,
                due_on,
                chat_id: None,
                completed_at: None,
                order_key: order_key.unwrap_or_default(),
                created_at: now,
                updated_at: now,
                deleted: false,
                mode: Default::default(),
                plan_pending: false,
                send_error: None,
            })
            .map_err(failed)
            .map(drop)
    }

    pub(super) fn update_task(&self, task_id: &str, patch: &TaskPatch) -> Result<(), RpcError> {
        match self.workspace.update_task(task_id, patch).map_err(failed)? {
            true => Ok(()),
            false => Err(refused("Task no longer exists")),
        }
    }

    pub(super) async fn start_task(
        &self,
        task_id: &str,
        chat_id: &str,
        space_id: Option<String>,
        harness: Option<HarnessId>,
        model: Option<String>,
        lead: Option<Lead>,
    ) -> Result<(), RpcError> {
        let first = self
            .starting_tasks
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(task_id.to_string());
        if !first {
            return Err(refused("This task is already starting"));
        }
        let _starting = Starting {
            tasks: &self.starting_tasks,
            task_id: task_id.to_string(),
        };
        let task = self
            .workspace
            .task(task_id)
            .ok_or_else(|| refused("Task no longer exists"))?;
        if task.chat_id.as_deref() == Some(chat_id) {
            return Ok(());
        }
        if let Some(linked) = task.chat_id.as_deref()
            && self.workspace.chat(linked).map_err(failed)?.is_some()
        {
            return Err(refused("This task already has a chat"));
        }
        let space = match space_id {
            Some(id) => Some(
                self.workspace
                    .space(&id)
                    .map_err(failed)?
                    .ok_or_else(|| refused("Project no longer exists"))?,
            ),
            None => match task.space_id.as_deref() {
                Some(id) => self.workspace.space(id).map_err(failed)?,
                None => None,
            },
        };
        let device = self.doc_host.device_id().to_string();
        if space
            .as_ref()
            .is_some_and(|space| space.device_id != device)
        {
            return Err(refused(
                "Start the task on the device that owns its project",
            ));
        }
        if model.is_some() && harness.is_none() {
            return Err(refused("A model needs its harness"));
        }
        let config = harness.map(|harness| ChatConfig {
            harness,
            model,
            reasoning: None,
            model_options: Default::default(),
            sandbox: SandboxLevel::WorkspaceWrite,
            permission_mode: Default::default(),
            instructions: String::new(),
            profile_id: None,
            instance_id: None,
            disabled_tools: Vec::new(),
        });
        let space_id = space.map(|space| space.id);
        self.workspace
            .create_chat_from(
                chat_id,
                space_id.as_deref(),
                Some(&device),
                config,
                None,
                None,
            )
            .map_err(failed)?;
        let started = self.link_and_send(&task, chat_id, space_id, lead);
        if started.is_err() {
            if self
                .workspace
                .task(task_id)
                .and_then(|t| t.chat_id)
                .as_deref()
                == Some(chat_id)
            {
                let cleared = TaskPatch {
                    chat_id: Some(None),
                    mode: lead.map(|_| task.mode),
                    plan_pending: lead.map(|_| task.plan_pending),
                    ..Default::default()
                };
                self.workspace.update_task(task_id, &cleared).ok();
            }
            self.delete_chat(chat_id).ok();
        }
        started
    }

    pub(super) async fn assign_task(
        &self,
        task_id: &str,
        chat_id: &str,
        send: bool,
        from_chat_id: Option<&str>,
    ) -> Result<(), RpcError> {
        let task = self
            .workspace
            .task(task_id)
            .ok_or_else(|| refused("Task no longer exists"))?;
        let chat = self
            .workspace
            .chat(chat_id)
            .map_err(failed)?
            .ok_or_else(|| refused("That thread no longer exists"))?;
        if chat.device_id != self.doc_host.device_id() {
            return Err(refused("Assign the task to a thread on this device"));
        }
        if send && from_chat_id == Some(chat_id) {
            return Err(refused("A thread cannot send a task to itself"));
        }
        self.update_task(
            task_id,
            &TaskPatch {
                chat_id: Some(Some(chat_id.to_string())),
                space_id: chat.space_id.filter(|_| task.space_id.is_none()).map(Some),
                completed: task.completed_at.is_some().then_some(false),
                ..Default::default()
            },
        )?;
        if send {
            self.send_to_task_thread(&task, chat_id, from_chat_id, &task_prompt(&task))
                .await?;
        }
        Ok(())
    }

    async fn send_to_task_thread(
        &self,
        task: &Task,
        chat_id: &str,
        from_chat_id: Option<&str>,
        text: &str,
    ) -> Result<(), RpcError> {
        let sent = self
            .doc_host
            .queue_thread_message(chat_id, from_chat_id, text)
            .await;
        let send_error = match &sent {
            Ok(_) if task.send_error.is_none() => return Ok(()),
            Ok(_) => None,
            Err(error) => Some(error.to_string()),
        };
        let patch = TaskPatch {
            send_error: Some(send_error),
            ..Default::default()
        };
        self.workspace
            .update_task(&task.id, &patch)
            .map_err(failed)?;
        sent.map(drop).map_err(failed)
    }

    fn lead_of(&self, task: &Task) -> Result<String, RpcError> {
        let lead = task
            .chat_id
            .clone()
            .ok_or_else(|| refused("This task has no lead thread"))?;
        match self.workspace.chat(&lead).map_err(failed)? {
            Some(_) => Ok(lead),
            None => Err(refused("This task has no lead thread")),
        }
    }

    fn task_or_refuse(&self, task_id: &str) -> Result<Task, RpcError> {
        self.workspace
            .task(task_id)
            .ok_or_else(|| refused("Task no longer exists"))
    }

    fn step_or_refuse(&self, step_id: &str) -> Result<TaskStep, RpcError> {
        self.workspace
            .task_step(step_id)
            .ok_or_else(|| refused("Step no longer exists"))
    }

    fn check_lead(task: &Task, from_chat_id: Option<&str>) -> Result<(), RpcError> {
        match from_chat_id {
            Some(from) if task.chat_id.as_deref() != Some(from) => {
                Err(refused("Only the task's lead thread writes its plan"))
            }
            _ => Ok(()),
        }
    }

    pub(super) fn plan_steps(
        &self,
        task_id: &str,
        steps: Vec<(String, String)>,
        from_chat_id: Option<&str>,
    ) -> Result<(), RpcError> {
        let task = self.task_or_refuse(task_id)?;
        Self::check_lead(&task, from_chat_id)?;
        let now = chrono::Utc::now();
        let steps: Vec<TaskStep> = steps
            .into_iter()
            .map(|(title, notes)| TaskStep {
                id: crate::new_id(),
                task_id: task_id.to_string(),
                title,
                notes,
                order_key: String::new(),
                chat_id: None,
                outcome: None,
                created_at: now,
                updated_at: now,
                deleted: false,
                turn: None,
            })
            .collect();
        self.workspace
            .plan_task_steps(task_id, &steps)
            .map_err(failed)?;
        if task.mode != TaskMode::Orchestrated {
            let patch = TaskPatch {
                mode: Some(TaskMode::Orchestrated),
                ..Default::default()
            };
            self.update_task(task_id, &patch)?;
        }
        Ok(())
    }

    pub(super) fn create_task_step(
        &self,
        task_id: String,
        step_id: String,
        title: String,
        notes: String,
        order_key: Option<String>,
    ) -> Result<(), RpcError> {
        let now = chrono::Utc::now();
        self.workspace
            .create_task_step(&TaskStep {
                id: step_id,
                task_id,
                title,
                notes,
                order_key: order_key.unwrap_or_default(),
                chat_id: None,
                outcome: None,
                created_at: now,
                updated_at: now,
                deleted: false,
                turn: None,
            })
            .map_err(failed)
            .map(drop)
    }

    pub(super) fn update_task_step(
        &self,
        step_id: &str,
        patch: &TaskStepPatch,
        from_chat_id: Option<&str>,
    ) -> Result<(), RpcError> {
        let step = self.step_or_refuse(step_id)?;
        if from_chat_id.is_some() {
            Self::check_lead(&self.task_or_refuse(&step.task_id)?, from_chat_id)?;
        }
        match self
            .workspace
            .update_task_step(step_id, patch)
            .map_err(failed)?
        {
            true => Ok(()),
            false => Err(refused("Step no longer exists")),
        }
    }

    pub(super) fn delete_task_step(&self, step_id: &str) -> Result<(), RpcError> {
        let Some(step) = self.workspace.task_step(step_id) else {
            return Ok(());
        };
        if let Some(chat_id) = step.chat_id.as_deref()
            && self.workspace.chat(chat_id).map_err(failed)?.is_some()
        {
            return Err(refused("This step has a thread; skip it instead"));
        }
        self.workspace
            .delete_task_step(step_id)
            .map_err(failed)
            .map(drop)
    }

    pub(super) async fn approve_plan(&self, task_id: &str) -> Result<(), RpcError> {
        let task = self.task_or_refuse(task_id)?;
        if !task.plan_pending {
            return Err(refused("This plan is not waiting for approval"));
        }
        let lead = self.lead_of(&task)?;
        if self.workspace.task_steps_of(task_id).is_empty() {
            return Err(refused("The plan has no steps yet"));
        }
        let patch = TaskPatch {
            plan_pending: Some(false),
            ..Default::default()
        };
        self.update_task(task_id, &patch)?;
        self.send_to_task_thread(&task, &lead, None, "The plan is approved. Run it.")
            .await
    }

    pub(super) async fn retry_task_step(&self, step_id: &str) -> Result<(), RpcError> {
        let step = self.step_or_refuse(step_id)?;
        let task = self.task_or_refuse(&step.task_id)?;
        let lead = self.lead_of(&task)?;
        let (title, id) = (&step.title, &step.id);
        let text = match step.chat_id {
            Some(thread) => format!(
                "Step \"{title}\" needs another run (taskStepId {id}). Check its thread {thread} with read_thread, then send it what to fix with send_message_to_thread, or start the step again with create_thread and taskStepId {id}."
            ),
            None => format!(
                "Step \"{title}\" has not started (taskStepId {id}). Run it with create_thread and taskStepId {id}."
            ),
        };
        self.send_to_task_thread(&task, &lead, None, &text).await
    }

    fn link_and_send(
        &self,
        task: &Task,
        chat_id: &str,
        space_id: Option<String>,
        lead: Option<Lead>,
    ) -> Result<(), RpcError> {
        self.workspace
            .rename_chat(chat_id, &task.title)
            .map_err(failed)?;
        self.update_task(
            &task.id,
            &TaskPatch {
                chat_id: Some(Some(chat_id.to_string())),
                space_id: space_id.filter(|_| task.space_id.is_none()).map(Some),
                mode: lead.map(|_| TaskMode::Orchestrated),
                plan_pending: lead.map(|lead| lead.approve_plan),
                send_error: task.send_error.is_some().then_some(None),
                ..Default::default()
            },
        )?;
        let prompt = match lead {
            Some(lead) => format!("{}\n\n{}", task_prompt(task), lead_brief(&task.id, lead)),
            None => task_prompt(task),
        };
        self.doc_host
            .request_from_chat_row(chat_id, &prompt)
            .ok_or_else(|| crate::EngineError::Other(format!("no such chat: {chat_id}")))
            .and_then(|request| {
                self.doc_host.queue_command_with_transfers(
                    chat_id,
                    SessionCommandPayload::Run {
                        request,
                        message_id: crate::new_id(),
                    },
                    Vec::new(),
                )
            })
            .map(drop)
            .map_err(failed)
    }
}
