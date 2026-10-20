use super::*;
use zeron_proto::{Task, TaskMode, TaskPatch, TaskStep, TaskStepPatch};

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
        lead_brief(&task.id, lead, reads.is_some())
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

fn lead_patch(
    chat_id: &str,
    space_id: Option<String>,
    task: &Task,
    lead: Option<Lead>,
) -> TaskPatch {
    TaskPatch {
        chat_id: Some(Some(chat_id.to_string())),
        space_id: space_id.filter(|_| task.space_id.is_none()).map(Some),
        mode: lead.map(|_| TaskMode::Orchestrated),
        plan_pending: lead.map(|lead| lead.approve_plan),
        plan_doc: lead.map(|lead| lead.plan_doc),
        ..Default::default()
    }
}

#[derive(Debug, Clone, Copy)]
pub(super) struct Lead {
    pub approve_plan: bool,
    pub same_agents: bool,
    pub plan_doc: bool,
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct PlanEdit {
    #[serde(default)]
    pub step_id: Option<String>,
    pub title: String,
    #[serde(default)]
    pub notes: String,
    #[serde(default)]
    pub images: Option<Vec<String>>,
}

const REORDER_REFUSED: &str =
    "Only steps that have not started can move, and only below the last started step";

fn new_images(before: &[String], after: &[String]) -> Vec<String> {
    after
        .iter()
        .filter(|path| !before.contains(path))
        .cloned()
        .collect()
}

fn image_change(before: &[String], after: &[String]) -> Option<String> {
    let added = new_images(before, after).len();
    let removed = before.iter().filter(|path| !after.contains(path)).count();
    match (added, removed) {
        (0, 0) => None,
        (1, 0) => Some("1 new image".into()),
        (n, 0) => Some(format!("{n} new images")),
        (0, 1) => Some("1 image removed".into()),
        (0, n) => Some(format!("{n} images removed")),
        (added, removed) => Some(format!("{added} new and {removed} removed images")),
    }
}

fn count_images(n: usize) -> String {
    match n {
        1 => "1 image".into(),
        n => format!("{n} images"),
    }
}

fn carry_images(before: &[TaskStep], steps: &mut [TaskStep]) -> (Vec<String>, Vec<String>) {
    let replaced: Vec<&TaskStep> = before
        .iter()
        .filter(|step| step.chat_id.is_none() && step.outcome.is_none())
        .collect();
    let mut moved = Vec::new();
    let mut orphans = Vec::new();
    for old in replaced.iter().filter(|step| !step.images.is_empty()) {
        let title = old.title.trim();
        let namesakes = replaced
            .iter()
            .filter(|step| step.title.trim() == title)
            .count();
        let mut matches = steps.iter_mut().filter(|step| step.title.trim() == title);
        let what = count_images(old.images.len());
        match (namesakes, matches.next(), matches.next()) {
            (1, Some(new), None) => {
                new.images = old.images.clone();
                moved.push(format!(
                    "Step \"{}\" (taskStepId {}) had {what}, now on the new step with its title (taskStepId {}).",
                    old.title, old.id, new.id
                ));
            }
            _ => {
                orphans.extend(old.images.iter().cloned());
                moved.push(format!(
                    "Step \"{}\" (taskStepId {}) had {what}, now on the overview: no single new step has its title.",
                    old.title, old.id
                ));
            }
        }
    }
    (moved, orphans)
}

fn reorder_keys(
    steps: &[(&TaskStep, bool)],
    order: &[String],
    mut nonce: impl FnMut() -> String,
) -> Result<Vec<(String, String)>, &'static str> {
    let listed: Vec<(&TaskStep, bool)> = order
        .iter()
        .filter_map(|id| steps.iter().find(|(step, _)| &step.id == id).copied())
        .collect();
    let unique: std::collections::HashSet<&str> = order.iter().map(String::as_str).collect();
    if listed.len() != steps.len() || unique.len() != steps.len() {
        return Err("The new order must list every remaining step once");
    }
    let key = |step: &TaskStep| (step.order_key.is_empty(), step.order_key.clone());
    let fixed = listed
        .iter()
        .rposition(|(_, started)| *started)
        .map_or(0, |last| last + 1);
    if listed[..fixed]
        .windows(2)
        .any(|pair| key(pair[0].0) >= key(pair[1].0))
    {
        return Err(REORDER_REFUSED);
    }
    let floor = fixed.checked_sub(1).map(|ix| key(listed[ix].0));
    let tail = &listed[fixed..];
    let eligible = |ix: usize| floor.as_ref().is_none_or(|floor| key(tail[ix].0) > *floor);
    let mut best: Vec<(usize, Option<usize>)> = Vec::with_capacity(tail.len());
    for ix in 0..tail.len() {
        let run = (0..ix)
            .filter(|&before| eligible(before) && key(tail[before].0) < key(tail[ix].0))
            .max_by_key(|&before| best[before].0)
            .map_or((0, None), |before| (best[before].0, Some(before)));
        best.push(if eligible(ix) {
            (run.0 + 1, run.1)
        } else {
            (0, None)
        });
    }
    let mut kept = vec![false; tail.len()];
    let mut at = (0..tail.len())
        .filter(|&ix| best[ix].0 > 0)
        .max_by_key(|&ix| best[ix].0);
    while let Some(ix) = at {
        kept[ix] = true;
        at = best[ix].1;
    }
    let mut lower = fixed
        .checked_sub(1)
        .map(|ix| listed[ix].0.order_key.clone());
    let mut moves = Vec::new();
    for (ix, (step, _)) in tail.iter().enumerate() {
        if kept[ix] {
            lower = Some(step.order_key.clone());
            continue;
        }
        let upper = (ix + 1..tail.len())
            .find(|&next| kept[next])
            .map(|next| tail[next].0.order_key.as_str());
        let key = zeron_proto::pin_order_key_between(lower.as_deref(), upper, &nonce())
            .map_err(|_| "This plan's steps cannot be reordered")?;
        moves.push((step.id.clone(), key.clone()));
        lower = Some(key);
    }
    Ok(moves)
}

pub(super) struct BySupervisor<'a> {
    pub supervisor: &'a str,
    pub reason: &'a str,
}

fn lead_brief(task_id: &str, lead: Lead, reads: bool) -> String {
    let plan = if lead.plan_doc {
        format!(
            "Write the plan with the tasks tool: action \"plan\", taskId \"{task_id}\", overview, steps [{{title, notes}}]. The overview is the goal, the approach, and how you will check the result. Each step's notes are its spec: what it builds, where, and how to tell it is done. Keep each step small enough for one agent."
        )
    } else {
        format!(
            "Write the plan with the tasks tool: action \"plan\", taskId \"{task_id}\", steps [{{title, notes}}]. Keep each step small enough for one agent."
        )
    };
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
        "When a decision changes the goal or costs money the person has not approved, or you cannot settle it from the plan, use the tasks tool action \"escalate\" and keep working on what does not depend on it.",
    );
    lines.push(
        "When every step is done, check the whole result, tell the person what was built, and mark the task completed with the tasks tool, action \"update\", completed true.",
    );
    if reads {
        lines.push(
            "When a step needs one of the chats you read, name it with its threadId in that step's prompt.",
        );
    }
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
        images: Vec<String>,
        context: Vec<String>,
    ) -> Result<(), RpcError> {
        self.check_context_chats(&context)?;
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
                plan: String::new(),
                plan_doc: false,
                resources: Vec::new(),
                images,
                context,
            })
            .map_err(failed)
            .map(drop)
    }

    pub(super) fn update_task(&self, task_id: &str, patch: &TaskPatch) -> Result<(), RpcError> {
        if let Some(context) = &patch.context {
            let task = self.task_or_refuse(task_id)?;
            if task.mode == TaskMode::Orchestrated && task.chat_id.is_some() {
                return Err(refused(
                    "The chats a mission reads are fixed once it starts; write to its lead instead",
                ));
            }
            self.check_context_chats(context)?;
        }
        match self.workspace.update_task(task_id, patch).map_err(failed)? {
            true => Ok(()),
            false => Err(refused("Task no longer exists")),
        }
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
        let chats: Vec<ContextChat> = task
            .context
            .iter()
            .filter_map(|id| self.workspace.chat(id).ok().flatten())
            .map(|chat| ContextChat {
                title: chat
                    .title
                    .as_deref()
                    .map(|title| title.split_whitespace().collect::<Vec<_>>().join(" "))
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
        space_id: Option<String>,
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
                    mode: lead.map(|_| task.mode),
                    plan_pending: lead.map(|_| task.plan_pending),
                    plan_doc: lead.map(|_| task.plan_doc),
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
        if lead.is_some()
            && self
                .workspace
                .read_tasks()
                .map_err(failed)?
                .iter()
                .any(|other| {
                    other.id != task_id
                        && other.completed_at.is_none()
                        && other.mode == TaskMode::Orchestrated
                        && other.chat_id.as_deref() == Some(chat_id)
                })
        {
            return Err(refused("This chat already leads a mission"));
        }
        let send = send || lead.is_some();
        if send && from_chat_id == Some(chat_id) {
            return Err(refused("A thread cannot send a task to itself"));
        }
        let reads = lead.and_then(|_| {
            self.context_reads(&task, chat.space_id.as_deref(), chat.config.as_ref())
        });
        self.update_task(
            task_id,
            &TaskPatch {
                completed: task.completed_at.is_some().then_some(false),
                ..lead_patch(chat_id, chat.space_id, &task, lead)
            },
        )?;
        if send {
            self.send_to_task_thread(
                &task,
                chat_id,
                from_chat_id,
                &first_turn(&task, lead, reads.as_deref()),
                task.images.clone(),
            )
            .await?;
        }
        Ok(())
    }

    pub(super) async fn send_to_task_thread(
        &self,
        task: &Task,
        chat_id: &str,
        from_chat_id: Option<&str>,
        text: &str,
        images: Vec<String>,
    ) -> Result<String, RpcError> {
        let sent = self
            .doc_host
            .queue_thread_message_with(chat_id, from_chat_id, text, images)
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

    pub(super) fn lead_of(&self, task: &Task) -> Result<String, RpcError> {
        let lead = task
            .chat_id
            .clone()
            .ok_or_else(|| refused("This task has no lead thread"))?;
        match self.workspace.chat(&lead).map_err(failed)? {
            Some(_) => Ok(lead),
            None => Err(refused("This task has no lead thread")),
        }
    }

    pub(super) fn task_or_refuse(&self, task_id: &str) -> Result<Task, RpcError> {
        self.workspace
            .task(task_id)
            .ok_or_else(|| refused("Task no longer exists"))
    }

    pub(super) fn step_or_refuse(&self, step_id: &str) -> Result<TaskStep, RpcError> {
        self.workspace
            .task_step(step_id)
            .ok_or_else(|| refused("Step no longer exists"))
    }

    fn check_lead(task: &Task, from_chat_id: Option<&str>, does: &str) -> Result<(), RpcError> {
        match from_chat_id {
            Some(from) if task.chat_id.as_deref() != Some(from) => {
                Err(refused(&format!("Only the task's lead thread {does}")))
            }
            _ => Ok(()),
        }
    }

    pub(super) fn escalate_task(
        &self,
        task_id: &str,
        from_chat_id: Option<&str>,
        title: String,
        why: zeron_proto::supervisor::ItemWhy,
        detail: String,
        options: Vec<String>,
    ) -> Result<String, RpcError> {
        let task = self.task_or_refuse(task_id)?;
        let from = from_chat_id.ok_or_else(|| refused("Only the task's lead thread escalates"))?;
        Self::check_lead(&task, Some(from), "escalates")?;
        let item = self.raise_item(
            zeron_proto::supervisor::Raiser::Lead,
            from,
            Some(task.id),
            title,
            why,
            detail,
            options,
        )?;
        self.doc_host.supervisor_escalated(&item, false);
        Ok(item.id)
    }

    pub(super) fn plan_steps(
        &self,
        task_id: &str,
        steps: Vec<(String, String)>,
        overview: Option<String>,
        resources: Option<Vec<String>>,
        from_chat_id: Option<&str>,
    ) -> Result<Vec<String>, RpcError> {
        let task = self.task_or_refuse(task_id)?;
        Self::check_lead(&task, from_chat_id, "writes its plan")?;
        let resources = resources
            .map(|keys| crate::supervisor_resources::checked_keys(&keys))
            .transpose()
            .map_err(|message| refused(&message))?
            .filter(|keys| *keys != task.resources);
        let before = self.workspace.task_steps_of(task_id);
        let now = chrono::Utc::now();
        let mut steps: Vec<TaskStep> = steps
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
                images: Vec::new(),
            })
            .collect();
        let (mut moved, orphans) = carry_images(&before, &mut steps);
        let mut images = task.images.clone();
        let mut left_out = Vec::new();
        for image in orphans {
            if images.contains(&image) {
                continue;
            }
            if images.len() < zeron_doc::TASK_IMAGES_MAX {
                images.push(image);
            } else {
                left_out.push(image);
            }
        }
        if !left_out.is_empty() {
            moved.push(format!(
                "The overview holds {} images, so these were left out; tell the person: {}",
                zeron_doc::TASK_IMAGES_MAX,
                left_out.join(", ")
            ));
        }
        self.workspace
            .plan_task_steps(task_id, &steps)
            .map_err(failed)?;
        let patch = TaskPatch {
            mode: (task.mode != TaskMode::Orchestrated).then_some(TaskMode::Orchestrated),
            plan: overview,
            resources,
            images: (images != task.images).then_some(images),
            ..Default::default()
        };
        if patch != TaskPatch::default() {
            self.update_task(task_id, &patch)?;
        }
        self.doc_host
            .supervisor_replanned(&task, &before, &steps, from_chat_id);
        Ok(moved)
    }

    pub(super) fn create_task_step(
        &self,
        task_id: String,
        step_id: String,
        title: String,
        notes: String,
        order_key: Option<String>,
        images: Vec<String>,
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
                images,
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
            Self::check_lead(
                &self.task_or_refuse(&step.task_id)?,
                from_chat_id,
                "writes its plan",
            )?;
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

    pub(super) async fn approve_plan(
        &self,
        task_id: &str,
        by: Option<BySupervisor<'_>>,
    ) -> Result<String, RpcError> {
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
        let (text, from) = match by {
            Some(by) => (
                format!(
                    "The Supervisor approved the plan. Run it.\n\nWhy: {}",
                    by.reason.trim()
                ),
                Some(by.supervisor),
            ),
            None => ("The plan is approved. Run it.".to_string(), None),
        };
        self.send_to_task_thread(&task, &lead, from, &text, Vec::new())
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
        self.send_to_task_thread(&task, &lead, None, &text, Vec::new())
            .await
            .map(drop)
    }

    fn unstarted(&self, step: &TaskStep) -> Result<bool, RpcError> {
        if step.outcome.is_some() {
            return Ok(false);
        }
        match step.chat_id.as_deref() {
            Some(chat_id) => Ok(self.workspace.chat(chat_id).map_err(failed)?.is_none()),
            None => Ok(true),
        }
    }

    /// Every edit is checked before the first write, so a refusal applies nothing.
    pub(super) async fn edit_task_plan(
        &self,
        task_id: &str,
        overview: Option<String>,
        overview_images: Option<Vec<String>>,
        steps: Vec<PlanEdit>,
        remove_step_ids: &[String],
        order: &[String],
        rebrief: Option<BySupervisor<'_>>,
    ) -> Result<Option<String>, RpcError> {
        if rebrief.is_some()
            && (overview.is_some()
                || overview_images.is_some()
                || !remove_step_ids.is_empty()
                || !order.is_empty()
                || steps
                    .iter()
                    .any(|edit| edit.step_id.is_none() || edit.images.is_some()))
        {
            return Err(refused(
                "A re-brief changes an existing step's title and spec only: the overview, new steps, removals and the order are the person's",
            ));
        }
        let task = self.task_or_refuse(task_id)?;
        let lead = self.lead_of(&task)?;
        let current = self.workspace.task_steps_of(task_id);
        let find = |id: &str| {
            current
                .iter()
                .find(|step| step.id == id)
                .ok_or_else(|| refused("Step no longer exists"))
        };
        let mut removed = Vec::new();
        for id in remove_step_ids {
            let step = find(id)?;
            if !self.unstarted(step)? {
                return Err(refused(&format!(
                    "Step \"{}\" has started, so it cannot be removed; skip it instead",
                    step.title
                )));
            }
            removed.push(step);
        }
        let too_long = |text: &str, max: usize| text.chars().count() > max;
        if overview
            .as_deref()
            .is_some_and(|text| too_long(text, zeron_doc::TASK_NOTES_MAX_CHARS))
        {
            return Err(refused("The overview is limited to 10,000 characters"));
        }
        let images_ok = |images: &Option<Vec<String>>| {
            images
                .as_deref()
                .map_or(Ok(()), zeron_doc::check_images)
                .map_err(|error| match error {
                    zeron_doc::DocError::Schema(message) => refused(&message),
                    other => failed(other.into()),
                })
        };
        images_ok(&overview_images)?;
        for edit in &steps {
            images_ok(&edit.images)?;
            let title = edit.title.trim();
            if title.is_empty() || too_long(title, zeron_doc::TASK_TITLE_MAX_CHARS) {
                return Err(refused("A step needs a title of up to 500 characters"));
            }
            if too_long(&edit.notes, zeron_doc::TASK_NOTES_MAX_CHARS) {
                return Err(refused("A step's spec is limited to 10,000 characters"));
            }
            if let Some(id) = &edit.step_id {
                find(id)?;
                if remove_step_ids.contains(id) {
                    return Err(refused("A step cannot be both edited and removed"));
                }
            }
        }
        let mut moves = Vec::new();
        if !order.is_empty() {
            let mut remaining = Vec::new();
            for step in current
                .iter()
                .filter(|step| !remove_step_ids.contains(&step.id))
            {
                remaining.push((step, !self.unstarted(step)?));
            }
            moves = reorder_keys(&remaining, order, crate::new_id).map_err(refused)?;
        }
        let mut changes = Vec::new();
        let overview = overview.filter(|overview| *overview != task.plan);
        let overview_images = overview_images.filter(|images| *images != task.images);
        let attached = overview_images
            .as_deref()
            .map(|images| new_images(&task.images, images))
            .unwrap_or_default();
        if overview.is_some() {
            changes.push("The overview changed.".to_string());
        }
        if let Some(what) = overview_images
            .as_deref()
            .and_then(|images| image_change(&task.images, images))
        {
            let note = if attached.is_empty() {
                ""
            } else {
                "; the new ones are attached to this message"
            };
            changes.push(format!("The overview has {what}{note}."));
        }
        if overview.is_some() || overview_images.is_some() {
            let patch = TaskPatch {
                plan: overview,
                images: overview_images,
                ..Default::default()
            };
            self.update_task(task_id, &patch)?;
        }
        if !moves.is_empty() {
            for (id, key) in &moves {
                let patch = TaskStepPatch {
                    order_key: Some(key.clone()),
                    ..Default::default()
                };
                self.update_task_step(id, &patch, None)?;
            }
            let title = |id: &str| {
                steps
                    .iter()
                    .find(|edit| edit.step_id.as_deref() == Some(id))
                    .map(|edit| edit.title.trim().to_string())
                    .or_else(|| find(id).ok().map(|step| step.title.clone()))
                    .unwrap_or_default()
            };
            let listed: Vec<String> = order
                .iter()
                .enumerate()
                .map(
                    |(ix, id)| match moves.iter().any(|(moved, _)| moved == id) {
                        true => format!("{}. \"{}\" (moved, taskStepId {id})", ix + 1, title(id)),
                        false => format!("{}. \"{}\"", ix + 1, title(id)),
                    },
                )
                .collect();
            changes.push(format!(
                "The steps are in a new order: {}.",
                listed.join(", ")
            ));
        }
        for edit in steps {
            let title = edit.title.trim().to_string();
            let Some(id) = edit.step_id else {
                let id = crate::new_id();
                let images = edit.images.unwrap_or_default();
                let with = match images.len() {
                    0 => String::new(),
                    1 => " with 1 image".to_string(),
                    n => format!(" with {n} images"),
                };
                self.create_task_step(
                    task_id.to_string(),
                    id.clone(),
                    title.clone(),
                    edit.notes,
                    None,
                    images,
                )?;
                changes.push(format!("New step \"{title}\" (taskStepId {id}){with}."));
                continue;
            };
            let step = find(&id)?;
            let images = edit.images.filter(|images| *images != step.images);
            let started = !self.unstarted(step)?;
            let image_note = images.as_deref().and_then(|images| {
                let what = image_change(&step.images, images)?;
                Some(
                    match started && !new_images(&step.images, images).is_empty() {
                        true => {
                            format!("{what} (its thread already started and does not get them)")
                        }
                        false => what,
                    },
                )
            });
            let patch = TaskStepPatch {
                title: (title != step.title).then(|| title.clone()),
                notes: (edit.notes != step.notes).then_some(edit.notes),
                images,
                ..Default::default()
            };
            let text = match (patch.title.is_some(), patch.notes.is_some()) {
                (true, true) => Some(format!("is now \"{title}\", and its spec changed")),
                (true, false) => Some(format!("is now \"{title}\"")),
                (false, true) => Some("has a new spec".to_string()),
                (false, false) => None,
            };
            let what = match (text, image_note) {
                (Some(text), Some(note)) => format!("{text}, and has {note}"),
                (Some(text), None) => text,
                (None, Some(note)) => format!("has {note}"),
                (None, None) => continue,
            };
            self.update_task_step(&id, &patch, None)?;
            changes.push(format!("Step \"{}\" (taskStepId {id}) {what}.", step.title));
        }
        for step in removed {
            self.delete_task_step(&step.id)?;
            changes.push(format!(
                "Step \"{}\" (taskStepId {}) is removed.",
                step.title, step.id
            ));
        }
        if changes.is_empty() {
            return Ok(None);
        }
        let next = if task.plan_pending {
            "The plan still waits for the person's approval: do not run any step until the approval message comes."
        } else {
            "Run the new and changed steps from this plan, and send a step whose thread is already running what changed with send_message_to_thread."
        };
        let (who, why, from) = match &rebrief {
            Some(rebrief) => (
                "The Supervisor re-briefed",
                format!("\n\nWhy: {}", rebrief.reason.trim()),
                Some(rebrief.supervisor),
            ),
            None => ("The person edited", String::new(), None),
        };
        let text = format!(
            "{who} the plan in Mission Control (task id {task_id}):\n- {}{why}\n\nRead the whole plan with the tasks tool: action \"list\", taskId \"{task_id}\". {next}",
            changes.join("\n- ")
        );
        self.doc_host.supervisor_plan_edited(&task, &changes, from);
        self.send_to_task_thread(&task, &lead, from, &text, attached)
            .await
            .map(Some)
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
                ..lead_patch(chat_id, space_id, task, lead)
            },
        )?;
        let message_id = crate::new_id();
        let prompt =
            crate::doc_host::queued_message_prompt(&first_turn(task, lead, reads), &task.images);
        self.doc_host
            .request_from_chat_row(chat_id, &prompt)
            .ok_or_else(|| crate::EngineError::Other(format!("no such chat: {chat_id}")))
            .and_then(|mut request| {
                request.attachments = task.images.clone();
                self.doc_host.queue_command_with_transfers(
                    chat_id,
                    SessionCommandPayload::Run {
                        request,
                        message_id: message_id.clone(),
                    },
                    Vec::new(),
                )
            })
            .map(|_| message_id)
            .map_err(failed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn step(id: &str, key: &str) -> TaskStep {
        TaskStep {
            id: id.into(),
            task_id: "t".into(),
            title: id.into(),
            notes: String::new(),
            order_key: key.into(),
            chat_id: None,
            outcome: None,
            created_at: Default::default(),
            updated_at: Default::default(),
            deleted: false,
            turn: None,
            images: Vec::new(),
        }
    }

    fn reorder(plan: &[(&str, bool)], order: &[&str]) -> Result<Vec<String>, &'static str> {
        let rows: Vec<TaskStep> = plan
            .iter()
            .enumerate()
            .map(|(ix, (id, _))| step(id, &format!("{}8", ix + 1)))
            .collect();
        let steps: Vec<(&TaskStep, bool)> = rows
            .iter()
            .zip(plan)
            .map(|(row, (_, started))| (row, *started))
            .collect();
        let order: Vec<String> = order.iter().map(|id| id.to_string()).collect();
        let mut n = 0;
        let moves = reorder_keys(&steps, &order, || {
            n += 1;
            format!("n{n}")
        })?;
        let mut keyed: Vec<(String, String)> = rows
            .iter()
            .map(|row| {
                let key = moves
                    .iter()
                    .find(|(id, _)| *id == row.id)
                    .map_or(row.order_key.clone(), |(_, key)| key.clone());
                (key, row.id.clone())
            })
            .collect();
        keyed.sort();
        assert_eq!(
            keyed.iter().map(|(_, id)| id.as_str()).collect::<Vec<_>>(),
            order,
            "the new keys sort into the new order"
        );
        Ok(moves.into_iter().map(|(id, _)| id).collect())
    }

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
    fn only_the_moved_step_gets_a_new_key() {
        let plan = [
            ("a", true),
            ("b", true),
            ("c", false),
            ("d", false),
            ("e", false),
        ];
        assert_eq!(
            reorder(&plan, &["a", "b", "e", "c", "d"]),
            Ok(vec!["e".into()])
        );
        assert_eq!(
            reorder(&plan, &["a", "b", "d", "e", "c"]),
            Ok(vec!["c".into()])
        );
        assert_eq!(reorder(&plan, &["a", "b", "c", "d", "e"]), Ok(vec![]));
    }

    #[test]
    fn an_unstarted_step_above_a_running_one_may_drop_below_it() {
        let plan = [("a", false), ("b", true), ("c", false)];
        assert_eq!(reorder(&plan, &["b", "a", "c"]), Ok(vec!["a".into()]));
        assert_eq!(reorder(&plan, &["b", "c", "a"]), Ok(vec!["a".into()]));
    }

    #[test]
    fn started_steps_keep_their_place() {
        let plan = [("a", true), ("b", true), ("c", false)];
        assert_eq!(reorder(&plan, &["b", "a", "c"]), Err(REORDER_REFUSED));
        assert_eq!(reorder(&plan, &["a", "c", "b"]), Err(REORDER_REFUSED));
        assert_eq!(reorder(&plan, &["c", "a", "b"]), Err(REORDER_REFUSED));
    }

    #[test]
    fn the_order_lists_every_step_once() {
        let plan = [("a", false), ("b", false)];
        assert!(reorder(&plan, &["a"]).is_err());
        assert!(reorder(&plan, &["a", "a"]).is_err());
        assert!(reorder(&plan, &["a", "x"]).is_err());
    }
}
