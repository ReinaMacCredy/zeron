use super::*;
use zeron_proto::{SandboxLevel, Task, TaskPatch};

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
        let started = self.link_and_send(&task, chat_id, space_id);
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
            self.doc_host
                .queue_thread_message(chat_id, from_chat_id, &task_prompt(&task))
                .await
                .map_err(failed)?;
        }
        Ok(())
    }

    fn link_and_send(
        &self,
        task: &Task,
        chat_id: &str,
        space_id: Option<String>,
    ) -> Result<(), RpcError> {
        self.workspace
            .rename_chat(chat_id, &task.title)
            .map_err(failed)?;
        self.update_task(
            &task.id,
            &TaskPatch {
                chat_id: Some(Some(chat_id.to_string())),
                space_id: space_id.filter(|_| task.space_id.is_none()).map(Some),
                ..Default::default()
            },
        )?;
        let prompt = task_prompt(task);
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
