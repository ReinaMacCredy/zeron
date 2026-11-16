use super::plan::written;
use super::*;
use zeron_proto::{ChatPlan, PlanSize, Task, TaskPatch};

pub(super) fn refused(message: &str) -> RpcError {
    RpcError::Failed(message.into())
}

pub(super) fn failed(error: crate::EngineError) -> RpcError {
    RpcError::Failed(error.to_string())
}

fn task_prompt(task: &Task) -> String {
    match task.notes.trim() {
        "" => task.title.clone(),
        notes => format!("{}\n\n{notes}", task.title),
    }
}

fn first_turn(task: &Task, lead: Option<Lead>, reads: Option<&str>) -> String {
    let Some(lead) = lead else {
        return task_prompt(task);
    };
    let reads_part = reads
        .map(|reads| format!("{reads}\n\n"))
        .unwrap_or_default();
    format!(
        "{}\n\n{reads_part}{}",
        task_prompt(task),
        lead_brief(Some(&task.id), lead, reads.is_some())
    )
}

struct ContextChat {
    id: String,
    title: String,
    project: Option<String>,
    working: bool,
}

fn reads_block(chats: &[ContextChat]) -> Option<String> {
    if chats.is_empty() {
        return None;
    }
    let lines: Vec<String> = chats
        .iter()
        .map(|chat| {
            let mut line = format!("- \"{}\" (threadId {}", chat.title, chat.id);
            if let Some(project) = &chat.project {
                line.push_str(&format!(", project {project}"));
            }
            if chat.working {
                line.push_str(", still working: read it again before you send the plan");
            }
            line.push(')');
            line
        })
        .collect();
    Some(format!(
        "Before you plan, read these chats with read_thread. They are context only: do not message them or answer their questions.\n{}",
        lines.join("\n")
    ))
}

fn lead_patch(chat_id: &str, space_id: Option<String>, task: &Task) -> TaskPatch {
    TaskPatch {
        chat_id: Some(Some(chat_id.to_string())),
        space_id: space_id.filter(|_| task.space_id.is_none()).map(Some),
        ..Default::default()
    }
}

#[derive(Debug, Clone, Copy)]
pub(super) struct Lead {
    pub approve_plan: bool,
    pub same_agents: bool,
    pub plan_doc: bool,
    pub attach: bool,
    pub size: PlanSize,
}

impl Lead {
    fn plan(self, task: &Task) -> ChatPlan {
        ChatPlan {
            plan_doc: self.plan_doc,
            same_agents: self.same_agents,
            size: Some(self.size),
            images: task.images.clone(),
            ..Default::default()
        }
    }
}

pub(crate) fn mission_brief(mission: zeron_proto::ChatMission) -> String {
    let lead = Lead {
        approve_plan: mission.approve,
        same_agents: mission.same_agents,
        plan_doc: false,
        attach: false,
        size: mission.size,
    };
    lead_brief(None, lead, false)
}

fn lead_brief(task_id: Option<&str>, lead: Lead, reads: bool) -> String {
    let mut lines = vec![if lead.plan_doc {
        "Write the plan with the plan tool: action \"write\", overview, steps [{title, notes, role, target}]. The overview is the goal, the approach, and how you will check the result. Each step's notes are its spec: what it builds, where, and how to tell it is done. Keep each step small enough for one agent."
    } else {
        "Write the plan with the plan tool: action \"write\", steps [{title, notes, role, target}]. Keep each step small enough for one agent."
    }];
    let sized = lead
        .size
        .cap()
        .map(|cap| format!("Plan at most {cap} steps. Merge small steps instead of going over."));
    if let Some(sized) = &sized {
        lines.push(sized);
    }
    if lead.attach {
        lines.push(
            "When a thread you already started with delegate_task does a step's work, set that step's threadId to the thread's id so the thread runs the step.",
        );
    }
    if lead.approve_plan {
        lines.push(
            "Then stop. The person reviews the plan and approves it; you get a message when it is approved.",
        );
    }
    lines.push(if lead.attach {
        "Do a step yourself, or with your own sub-agents, then mark it done with the plan tool, action \"step\". Run a step on its own thread with delegate_task only when it needs one: task set to what the step must do, stepId set to the step's id."
    } else {
        "Run each step on its own thread with delegate_task: task set to what the step must do, stepId set to the step's id. Steps that do not depend on each other can run at the same time."
    });
    lines.push(if lead.same_agents {
        "Run every step on your own harness and model: do not pass target or profile to delegate_task."
    } else {
        "Give each step a role (worker, validator, research, design or general). Choose each step's model and effort from the notes in orchestrator_capabilities, the cheapest that is enough, and write it as the step's target in the plan, with a one-line reason in its notes; a step with no target runs on its role's route in Settings > Routing."
    });
    lines.push(
        "Zeron messages you when a step's thread finishes a turn or fails. Read the thread with read_thread, then mark the step done or skipped with the plan tool, action \"step\", send the thread a fix with send_message_to_thread, or start the step over with delegate_task and the same stepId. When a step fails, first make sure it can verify its own work, then raise its effort one level, then move it to the next model up at a lower effort.",
    );
    lines.push(if task_id.is_some() {
        "When every step is done, check the whole result, tell the person what was built, and mark the task completed with the tasks tool, action \"update\", completed true."
    } else {
        "When every step is done, check the whole result and tell the person what was built."
    });
    if reads {
        lines.push(
            "When a step needs one of the chats you read, name it with its threadId in that step's task.",
        );
    }
    let steps: Vec<String> = lines
        .iter()
        .enumerate()
        .map(|(n, line)| format!("{}. {line}", n + 1))
        .collect();
    let intro = if lead.attach {
        "This task is a plan for the work this chat is already doing. Break that work into steps; do a step yourself or run it on a thread."
    } else {
        "Break it into steps and run each step on its own thread."
    };
    match task_id {
        Some(task_id) => format!(
            "You lead this task (task id {task_id}). {intro}\n\n{}",
            steps.join("\n")
        ),
        None => format!(
            "{}\n\n{}",
            zeron_proto::MISSION_BRIEF_OPENING,
            steps.join("\n")
        ),
    }
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
        images: Vec<String>,
        context: Vec<String>,
        depends_on: Vec<String>,
    ) -> Result<(), RpcError> {
        self.check_context_chats(&context)?;
        self.check_dependencies(&task_id, &depends_on, &[])?;
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
                send_error: None,
                images,
                context,
                depends_on,
            })
            .map_err(failed)
            .map(drop)
    }

    pub(super) fn update_task(&self, task_id: &str, patch: &TaskPatch) -> Result<(), RpcError> {
        self.update_task_when(task_id, patch, None)
    }

    fn update_task_when(
        &self,
        task_id: &str,
        patch: &TaskPatch,
        when: Option<&dyn Fn(&Task) -> Result<(), RpcError>>,
    ) -> Result<(), RpcError> {
        let _plans = patch.depends_on.is_some().then(|| self.plan_writes());
        if let Some(depends_on) = &patch.depends_on {
            let kept = self.task_or_refuse(task_id)?.depends_on;
            self.check_dependencies(task_id, depends_on, &kept)?;
        }
        if patch.context.is_some() || when.is_some() {
            let task = self.task_or_refuse(task_id)?;
            if let Some(context) = &patch.context {
                if self.leads_plan(&task) {
                    return Err(refused(
                        "The chats a mission reads are fixed once it starts; write to its lead instead",
                    ));
                }
                let added: Vec<String> = context
                    .iter()
                    .filter(|id| !task.context.contains(id))
                    .cloned()
                    .collect();
                self.check_context_chats(&added)?;
            }
            if let Some(when) = when {
                when(&task)?;
            }
        }
        match self.workspace.update_task(task_id, patch).map_err(failed)? {
            true => Ok(()),
            false => Err(refused("Task no longer exists")),
        }
    }

    pub(super) fn leads_plan(&self, task: &Task) -> bool {
        task.chat_id
            .as_deref()
            .and_then(|id| self.workspace.chat(id).ok().flatten())
            .is_some_and(|chat| chat.has_plan())
    }

    fn check_dependencies(
        &self,
        task_id: &str,
        depends_on: &[String],
        kept: &[String],
    ) -> Result<(), RpcError> {
        if depends_on.is_empty() {
            return Ok(());
        }
        let tasks = self.workspace.read_tasks().map_err(failed)?;
        dependency_problem(task_id, depends_on, kept, &tasks)
            .map_or(Ok(()), |problem| Err(refused(&problem)))
    }

    /// Held from a plan check to its write, so two calls cannot both pass the check.
    pub(super) fn plan_writes(&self) -> std::sync::MutexGuard<'_, ()> {
        self.plan_writes
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn check_context_chats(&self, context: &[String]) -> Result<(), RpcError> {
        for id in context {
            let chat = self.workspace.chat(id).map_err(failed)?;
            if chat.is_none_or(|chat| chat.device_id != self.doc_host.device_id()) {
                return Err(refused("A mission reads chats on this device only"));
            }
        }
        Ok(())
    }

    fn context_reads(
        &self,
        task: &Task,
        home: Option<&str>,
        config: Option<&ChatConfig>,
    ) -> Option<String> {
        if task.context.is_empty() || !self.sessions.reads_threads(config) {
            return None;
        }
        let device = self.doc_host.device_id();
        let chats: Vec<ContextChat> = task
            .context
            .iter()
            .filter_map(|id| self.workspace.chat(id).ok().flatten())
            .filter(|chat| chat.device_id == device)
            .map(|chat| ContextChat {
                title: chat
                    .title
                    .as_deref()
                    .map(|title| {
                        let title = title.split_whitespace().collect::<Vec<_>>().join(" ");
                        title
                            .replace('"', "'")
                            .chars()
                            .take(120)
                            .collect::<String>()
                    })
                    .filter(|title| !title.is_empty())
                    .unwrap_or_else(|| "Untitled chat".into()),
                project: chat
                    .space_id
                    .as_deref()
                    .filter(|space| Some(*space) != home)
                    .and_then(|space| self.workspace.space(space).ok().flatten())
                    .map(|space| space.display_name().to_string()),
                working: self
                    .sessions
                    .session_status(&chat.id)
                    .is_some_and(|session| session.status == zeron_proto::SessionStatus::Working),
                id: chat.id,
            })
            .collect();
        reads_block(&chats)
    }

    pub(super) async fn start_task(
        &self,
        task_id: &str,
        chat_id: &str,
        space_id: Option<Option<String>>,
        config: Option<ChatConfig>,
        lead: Option<Lead>,
    ) -> Result<Option<String>, RpcError> {
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
            return Ok(None);
        }
        if let Some(linked) = task.chat_id.as_deref()
            && self.workspace.chat(linked).map_err(failed)?.is_some()
        {
            return Err(refused("This task already has a chat"));
        }
        let space = match space_id {
            Some(None) => None,
            Some(Some(id)) => Some(
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
        let space_id = space.map(|space| space.id);
        self.workspace
            .create_chat_from(
                chat_id,
                space_id.as_deref(),
                Some(&device),
                config.clone(),
                None,
                None,
            )
            .map_err(failed)?;
        let reads =
            lead.and_then(|_| self.context_reads(&task, space_id.as_deref(), config.as_ref()));
        let started = self.link_and_send(&task, chat_id, space_id, lead, reads.as_deref());
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
                    space_id: task.space_id.is_none().then_some(None),
                    send_error: task.send_error.clone().map(Some),
                    ..Default::default()
                };
                self.workspace.update_task(task_id, &cleared).ok();
            }
            self.delete_chat(chat_id).ok();
        }
        started.map(Some)
    }

    /// Unlike `start_task`, a lead chat keeps its title and a busy turn is steered, not interrupted.
    pub(super) async fn assign_task(
        &self,
        task_id: &str,
        chat_id: &str,
        send: bool,
        from_chat_id: Option<&str>,
        lead: Option<Lead>,
        claim: bool,
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
        let send = send || lead.is_some();
        if send && from_chat_id == Some(chat_id) {
            return Err(refused("A thread cannot send a task to itself"));
        }
        if lead.is_some() && chat.sidechat_source_chat_id.is_some() {
            return Err(refused("A side chat cannot lead a mission"));
        }
        if lead.is_some() && chat.has_plan() {
            return Err(refused(
                "This chat already has a plan; a chat holds one plan for its life",
            ));
        }
        let reads = lead.and_then(|_| {
            self.context_reads(&task, chat.space_id.as_deref(), chat.config.as_ref())
        });
        let unclaimed = |now: &Task| -> Result<(), RpcError> {
            if now.completed_at.is_some() {
                return Err(refused(&format!("task {task_id} is already done")));
            }
            match now.chat_id.as_deref() {
                Some(holder) if holder != chat_id => Err(refused(&format!(
                    "task {task_id} is claimed by thread {holder}; leave it unless the person hands it over"
                ))),
                _ => Ok(()),
            }
        };
        self.update_task_when(
            task_id,
            &TaskPatch {
                completed: task.completed_at.is_some().then_some(false),
                ..lead_patch(chat_id, chat.space_id, &task)
            },
            claim.then_some(&unclaimed as &dyn Fn(&Task) -> Result<(), RpcError>),
        )?;
        let restore = TaskPatch {
            chat_id: Some(task.chat_id.clone()),
            space_id: task.space_id.is_none().then_some(None),
            completed: task.completed_at.is_some().then_some(true),
            ..Default::default()
        };
        if let Some(lead) = lead
            && let Err(error) =
                self.workspace
                    .set_chat_plan(chat_id, Some(&lead.plan(&task)), lead.approve_plan)
        {
            self.workspace.update_task(task_id, &restore).ok();
            return Err(written(error));
        }
        if send {
            let sent = self
                .send_to_task_thread(
                    &task,
                    chat_id,
                    from_chat_id,
                    &first_turn(&task, lead, reads.as_deref()),
                    task.images.clone(),
                )
                .await;
            if sent.is_err() && lead.is_some() {
                self.workspace.update_task(task_id, &restore).ok();
                self.workspace.set_chat_plan(chat_id, None, false).ok();
            }
            sent?;
        }
        Ok(())
    }

    fn gathered(&self, task_ids: &[String]) -> Result<Vec<Task>, RpcError> {
        if task_ids.is_empty() {
            return Err(refused("Pick at least one task"));
        }
        let mut tasks: Vec<Task> = Vec::with_capacity(task_ids.len());
        for id in task_ids {
            let task = self.task_or_refuse(id)?;
            if self.leads_plan(&task) {
                return Err(refused(&format!(
                    "\"{}\" is a mission; start it on its own",
                    task.title
                )));
            }
            if !tasks.iter().any(|known| known.id == task.id) {
                tasks.push(task);
            }
        }
        Ok(tasks)
    }

    fn link_gathered(
        &self,
        tasks: &[Task],
        chat_id: &str,
        space_id: Option<String>,
    ) -> Result<(), RpcError> {
        for task in tasks {
            self.update_task(
                &task.id,
                &TaskPatch {
                    send_error: task.send_error.is_some().then_some(None),
                    ..lead_patch(chat_id, space_id.clone(), task)
                },
            )?;
        }
        Ok(())
    }

    fn gathered_message(tasks: &[Task], existing: bool) -> (String, Vec<String>) {
        let text = zeron_proto::task_mentions::gathered_tasks_prompt(
            tasks
                .iter()
                .map(|task| (task.id.as_str(), task.title.as_str())),
            existing,
        );
        let images = tasks.iter().flat_map(|task| task.images.clone()).collect();
        crate::doc_host::images_here(&text, images)
    }

    /// One new thread for several tasks: every task links to it and one message lists them in order.
    pub(super) async fn start_tasks(
        &self,
        task_ids: &[String],
        chat_id: &str,
        config: Option<ChatConfig>,
    ) -> Result<(), RpcError> {
        let mut starting = Vec::new();
        for task_id in task_ids {
            let first = self
                .starting_tasks
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .insert(task_id.clone());
            if !first {
                return Err(refused("One of these tasks is already starting"));
            }
            starting.push(Starting {
                tasks: &self.starting_tasks,
                task_id: task_id.clone(),
            });
        }
        let tasks = self.gathered(task_ids)?;
        let space = match tasks.iter().find_map(|task| task.space_id.clone()) {
            Some(id) => self.workspace.space(&id).map_err(failed)?,
            None => None,
        };
        let device = self.doc_host.device_id().to_string();
        if space
            .as_ref()
            .is_some_and(|space| space.device_id != device)
        {
            return Err(refused(
                "Start the tasks on the device that owns their project",
            ));
        }
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
        let title = match tasks.len() {
            1 => tasks[0].title.clone(),
            n => format!("{} +{}", tasks[0].title, n - 1),
        };
        let started = self
            .workspace
            .rename_chat(chat_id, &title)
            .map_err(failed)
            .and_then(|_| self.link_gathered(&tasks, chat_id, space_id))
            .and_then(|_| {
                let (text, images) = Self::gathered_message(&tasks, false);
                self.doc_host
                    .queue_message_detached(chat_id, &text, images)
                    .map_err(failed)
            });
        if started.is_err() {
            for task in &tasks {
                if self
                    .workspace
                    .task(&task.id)
                    .and_then(|t| t.chat_id)
                    .as_deref()
                    == Some(chat_id)
                {
                    let restore = TaskPatch {
                        chat_id: Some(task.chat_id.clone()),
                        space_id: task.space_id.is_none().then_some(None),
                        ..Default::default()
                    };
                    self.workspace.update_task(&task.id, &restore).ok();
                }
            }
            self.delete_chat(chat_id).ok();
        }
        drop(starting);
        started.map(drop)
    }

    /// Several tasks handed to an existing thread in one message; a busy thread gets it after its turn.
    pub(super) fn add_tasks_to_thread(
        &self,
        task_ids: &[String],
        chat_id: &str,
    ) -> Result<(), RpcError> {
        let tasks = self.gathered(task_ids)?;
        let chat = self
            .workspace
            .chat(chat_id)
            .map_err(failed)?
            .ok_or_else(|| refused("That thread no longer exists"))?;
        if chat.device_id != self.doc_host.device_id() {
            return Err(refused("Add the tasks to a thread on this device"));
        }
        self.link_gathered(&tasks, chat_id, chat.space_id)?;
        let (text, images) = Self::gathered_message(&tasks, true);
        self.doc_host
            .queue_message_after_turn(chat_id, &text, images)
            .map_err(failed)
            .map(drop)
    }

    /// A failed send marks the task, so its card says the thread missed the message.
    pub(super) async fn send_to_task_thread(
        &self,
        task: &Task,
        chat_id: &str,
        from_chat_id: Option<&str>,
        text: &str,
        images: Vec<String>,
    ) -> Result<String, RpcError> {
        let (text, images) = crate::doc_host::images_here(text, images);
        let sent = self
            .doc_host
            .queue_thread_message_with(chat_id, from_chat_id, &text, images)
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
        self.workspace
            .update_task(&task.id, &patch)
            .map_err(failed)?;
        sent.map_err(failed)
    }

    pub(super) fn task_or_refuse(&self, task_id: &str) -> Result<Task, RpcError> {
        self.workspace
            .task(task_id)
            .ok_or_else(|| refused("Task no longer exists"))
    }

    fn link_and_send(
        &self,
        task: &Task,
        chat_id: &str,
        space_id: Option<String>,
        lead: Option<Lead>,
        reads: Option<&str>,
    ) -> Result<String, RpcError> {
        self.workspace
            .rename_chat(chat_id, &task.title)
            .map_err(failed)?;
        self.update_task(
            &task.id,
            &TaskPatch {
                send_error: task.send_error.is_some().then_some(None),
                ..lead_patch(chat_id, space_id, task)
            },
        )?;
        if let Some(lead) = lead {
            self.workspace
                .set_chat_plan(chat_id, Some(&lead.plan(task)), lead.approve_plan)
                .map_err(written)?;
        }
        let (text, images) =
            crate::doc_host::images_here(&first_turn(task, lead, reads), task.images.clone());
        self.doc_host
            .queue_message_detached(chat_id, &text, images)
            .map_err(failed)
    }
}

fn dependency_problem(
    task_id: &str,
    depends_on: &[String],
    kept: &[String],
    tasks: &[Task],
) -> Option<String> {
    for id in depends_on {
        if id == task_id {
            return Some("A task cannot depend on itself".into());
        }
        if !kept.contains(id) && !tasks.iter().any(|task| task.id == *id) {
            return Some(format!("dependsOn {id} is not a task"));
        }
    }
    let mut stack: Vec<&str> = depends_on.iter().map(String::as_str).collect();
    let mut seen = std::collections::HashSet::new();
    while let Some(id) = stack.pop() {
        if id == task_id {
            return Some("That dependency would make a cycle".into());
        }
        if seen.insert(id)
            && let Some(task) = tasks.iter().find(|task| task.id == id)
        {
            stack.extend(task.depends_on.iter().map(String::as_str));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_reads_block_marks_other_projects_and_working_chats() {
        let chat = |id: &str, project: Option<&str>, working: bool| ContextChat {
            id: id.into(),
            title: format!("Chat {id}"),
            project: project.map(str::to_owned),
            working,
        };
        assert_eq!(reads_block(&[]), None);
        assert_eq!(
            reads_block(&[chat("a", None, true), chat("b", Some("astra"), false)]).unwrap(),
            "Before you plan, read these chats with read_thread. They are context only: do not message them or answer their questions.\n- \"Chat a\" (threadId a, still working: read it again before you send the plan)\n- \"Chat b\" (threadId b, project astra)"
        );
    }

    #[test]
    fn a_dependency_is_a_known_task_other_than_itself_and_closes_no_cycle() {
        let task = |id: &str, depends_on: &[&str]| -> Task {
            let mut task: Task = serde_json::from_value(serde_json::json!({ "id": id })).unwrap();
            task.depends_on = depends_on.iter().map(|id| id.to_string()).collect();
            task
        };
        let tasks = [task("a", &[]), task("b", &["a"]), task("c", &["b"])];
        let deps = |ids: &[&str]| -> Vec<String> { ids.iter().map(|id| id.to_string()).collect() };
        assert_eq!(
            dependency_problem("new", &deps(&["a", "c"]), &[], &tasks),
            None
        );
        assert_eq!(dependency_problem("a", &deps(&[]), &[], &tasks), None);
        assert_eq!(
            dependency_problem("a", &deps(&["a"]), &[], &tasks).as_deref(),
            Some("A task cannot depend on itself")
        );
        assert_eq!(
            dependency_problem("new", &deps(&["gone"]), &[], &tasks).as_deref(),
            Some("dependsOn gone is not a task")
        );
        assert_eq!(
            dependency_problem("a", &deps(&["c"]), &[], &tasks).as_deref(),
            Some("That dependency would make a cycle")
        );
        assert_eq!(
            dependency_problem("b", &deps(&["a"]), &[], &tasks),
            None,
            "rewriting the same edge is not a cycle"
        );
        assert_eq!(
            dependency_problem("c", &deps(&["gone", "a"]), &deps(&["gone"]), &tasks),
            None,
            "a kept dependency on a deleted task does not block the update"
        );
    }
}
