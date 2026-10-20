use super::*;
use crate::supervisor::{PauseHold, SupervisorStore};
use crate::supervisor_resources::AFTER_PAUSE_MS;
use zeron_proto::supervisor::{
    CHAT_PREFIX, CHAT_TITLE, InterventionKind, ItemWhy, Raiser, ResourceKey, SupervisorItem,
    SupervisorSettings, is_supervisor_chat,
};

pub(super) const MESSAGE_OFF: &str = "Messaging leads and step threads is off in the Supervisor's Authority; the person can turn it on on the Supervisor page.";
pub(super) const START_OFF: &str = "Starting missions is off in the Supervisor's Authority, and create_thread and delegate_task go with it; the person can turn it on on the Supervisor page.";
pub(super) const PERSON_ONLY: &str =
    "Only the person stops the Supervisor or answers its questions; send it a message instead.";
pub(super) const NO_INTERRUPT: &str = "The Supervisor does not interrupt threads: a step that blocks a shared resource is stopped with supervisor pause, which needs a verified contention. Otherwise message the step's lead.";
const PAUSE_OFF: &str = "Pausing steps is off in the Supervisor's Authority; the person can turn it on on the Supervisor page.";
const UNIX_ONLY: &str = "Shared resources are checked against the machine on macOS and Linux only";
pub(super) const GOAL_OR_SPEND: &str = "A task is the person's: the Supervisor cannot change it, mark it done, delete it or reassign it. Raise it with the supervisor tool, action \"ask\".";
const MAX_OPTIONS: usize = 3;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct EnsureSupervisorParams {
    config: ChatConfig,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "action", rename_all = "camelCase")]
enum SupervisorAction {
    #[serde(rename_all = "camelCase")]
    Ask {
        #[serde(default)]
        task_id: Option<String>,
        title: String,
        why: ItemWhy,
        #[serde(default)]
        detail: String,
        options: Vec<String>,
    },
    #[serde(rename_all = "camelCase")]
    Rebrief {
        step_id: String,
        #[serde(default)]
        title: Option<String>,
        notes: String,
        reason: String,
    },
    #[serde(rename_all = "camelCase")]
    Start {
        title: String,
        #[serde(default)]
        notes: String,
        #[serde(default)]
        project_id: Option<String>,
        #[serde(default)]
        plan_doc: bool,
    },
    #[serde(rename_all = "camelCase")]
    Pause {
        step_id: String,
        resource: String,
        reason: String,
    },
    #[serde(rename_all = "camelCase")]
    Resume {
        step_id: String,
        #[serde(default)]
        resource: Option<String>,
        reason: String,
    },
    #[serde(rename_all = "camelCase")]
    Check {
        #[serde(default)]
        task_id: Option<String>,
    },
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ResumeStepParams {
    step_id: String,
    #[serde(default)]
    from_chat_id: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SupervisorActParams {
    from_chat_id: String,
    #[serde(flatten)]
    action: SupervisorAction,
}

fn failed(error: crate::EngineError) -> RpcError {
    RpcError::Failed(error.to_string())
}

fn refused(message: &str) -> RpcError {
    RpcError::Failed(message.into())
}

pub(super) fn refuse_supervisor(from: Option<&str>, refusal: &str) -> Result<(), RpcError> {
    match from {
        Some(from) if is_supervisor_chat(from) => Err(refused(refusal)),
        _ => Ok(()),
    }
}

impl EngineRpc {
    pub(super) fn supervisor_gate(
        &self,
        from: Option<&str>,
        allowed: fn(&SupervisorSettings) -> bool,
        refusal: &str,
    ) -> Result<bool, RpcError> {
        match from {
            Some(from) if is_supervisor_chat(from) => {
                if allowed(&self.registry.supervisor_settings()) {
                    Ok(true)
                } else {
                    Err(refused(refusal))
                }
            }
            _ => Ok(false),
        }
    }

    pub(super) fn refuse_held_thread(&self, chat_id: &str) -> Result<(), RpcError> {
        let Some(hold) = self.doc_host.supervisor().and_then(|store| {
            store.holds().into_iter().find(|hold| {
                hold.resumed_at.is_none() && hold.thread_id.as_deref() == Some(chat_id)
            })
        }) else {
            return Ok(());
        };
        Err(refused(&format!(
            "Thread {chat_id} runs step {}, which the Supervisor paused for {}: it takes no messages until the step is resumed.",
            hold.step_id, hold.resource
        )))
    }

    pub(super) fn supervisor_store(&self) -> Result<&SupervisorStore, RpcError> {
        self.doc_host
            .supervisor()
            .ok_or_else(|| RpcError::Failed("This engine has no Supervisor".into()))
    }

    pub(super) async fn supervisor_rpc(
        &self,
        method: &str,
        params: serde_json::Value,
    ) -> Result<RpcReply, RpcError> {
        match method {
            methods::ENSURE_SUPERVISOR => {
                let p: EnsureSupervisorParams = parse_params(params)?;
                let chat_id = self.ensure_supervisor(p.config)?;
                RpcReply::value(&serde_json::json!({ "chatId": chat_id }))
            }
            methods::WATCH_SUPERVISOR => Ok(RpcReply::Stream(watch_stream(
                self.supervisor_store()?.watch(),
            ))),
            methods::GET_SUPERVISOR_SETTINGS => {
                RpcReply::value(&self.registry.supervisor_settings())
            }
            methods::SET_SUPERVISOR_SETTINGS => {
                let p: SupervisorSettings = parse_params(params)?;
                self.registry.set_supervisor_settings(p);
                self.supervisor_store()?.poke();
                RpcReply::value(&self.registry.supervisor_settings())
            }
            methods::SUPERVISOR_ACT => {
                let p: SupervisorActParams = parse_params(params)?;
                let reply = self.supervisor_act(&p.from_chat_id, p.action).await?;
                RpcReply::value(&reply)
            }
            methods::REQUEST_SUPERVISOR_CHECK => {
                self.doc_host.request_resource_check();
                RpcReply::value(&serde_json::json!({}))
            }
            methods::RESUME_SUPERVISOR_STEP => {
                let p: ResumeStepParams = parse_params(params)?;
                if p.from_chat_id.is_some() {
                    return Err(refused(
                        "Only the person resumes a step from the Supervisor page",
                    ));
                }
                let reply = self.resume_step(None, &p.step_id, None, "").await?;
                RpcReply::value(&reply)
            }
            _ => Err(RpcError::UnknownMethod(method.to_string())),
        }
    }

    fn ensure_supervisor(&self, config: ChatConfig) -> Result<String, RpcError> {
        let store = self.supervisor_store()?;
        let _ensuring = store.ensuring();
        let device = self.doc_host.device_id().to_string();
        let chats = self.workspace.read_chats().map_err(failed)?;
        if let Some(chat) = crate::doc_host::newest_supervisor(&chats, &device) {
            return Ok(chat.id.clone());
        }
        let chat_id = format!("{CHAT_PREFIX}{}", uuid::Uuid::new_v4());
        self.workspace
            .create_chat(&chat_id, None, Some(&device), Some(config), None)
            .map_err(failed)?;
        self.workspace
            .rename_chat(&chat_id, CHAT_TITLE)
            .map_err(failed)?;
        store.set_chat_id(Some(chat_id.clone()));
        store.poke();
        Ok(chat_id)
    }

    async fn supervisor_act(
        &self,
        from: &str,
        action: SupervisorAction,
    ) -> Result<serde_json::Value, RpcError> {
        if !self.doc_host.is_live_supervisor(from) {
            return Err(refused("Only the Supervisor uses the supervisor tool"));
        }
        let settings = self.registry.supervisor_settings();
        match action {
            SupervisorAction::Ask {
                task_id,
                title,
                why,
                detail,
                options,
            } => {
                if let Some(task_id) = &task_id {
                    self.task_or_refuse(task_id)?;
                }
                let item = self.raise_item(
                    Raiser::Supervisor,
                    from,
                    task_id,
                    title,
                    why,
                    detail,
                    options,
                )?;
                Ok(serde_json::json!({ "itemId": item.id }))
            }
            SupervisorAction::Rebrief {
                step_id,
                title,
                notes,
                reason,
            } => {
                if !settings.rebrief {
                    return Err(refused(
                        "Re-briefing steps is off in the Supervisor's Authority; the person can turn it on on the Supervisor page.",
                    ));
                }
                if reason.trim().is_empty() {
                    return Err(refused(
                        "reason is required: the lead reads why the step changed",
                    ));
                }
                let step = self.step_or_refuse(&step_id)?;
                let task = self.task_or_refuse(&step.task_id)?;
                let lead = self.lead_of(&task)?;
                let basis = self.doc_host.intervention_basis(
                    InterventionKind::Rebrief,
                    from,
                    Some(&lead),
                    Some(&step_id),
                );
                let edit = tasks::PlanEdit {
                    step_id: Some(step_id.clone()),
                    title: title.unwrap_or_else(|| step.title.clone()),
                    notes,
                    images: None,
                };
                let rebrief = tasks::Rebrief {
                    supervisor: from,
                    reason: &reason,
                };
                let message_id = self
                    .edit_task_plan(&task.id, None, None, vec![edit], &[], &[], Some(rebrief))
                    .await?
                    .ok_or_else(|| refused("That step already has this title and spec"))?;
                self.doc_host
                    .record_intervention(basis, Some(message_id.clone()), None, &reason);
                Ok(
                    serde_json::json!({ "stepId": step_id, "leadThreadId": lead, "messageId": message_id }),
                )
            }
            SupervisorAction::Start {
                title,
                notes,
                project_id,
                plan_doc,
            } => {
                if !settings.start_missions {
                    return Err(refused(START_OFF));
                }
                if title.trim().is_empty() {
                    return Err(refused("title is required"));
                }
                let config = self
                    .workspace
                    .chat(from)
                    .map_err(failed)?
                    .and_then(|chat| chat.config)
                    .map(|mut config| {
                        config.profile_id = None;
                        config.instructions.clear();
                        config
                    });
                let task_id = crate::new_id();
                let lead = uuid::Uuid::new_v4().to_string();
                let basis = self
                    .doc_host
                    .intervention_basis(InterventionKind::Start, from, None, None)
                    .with_task(&task_id, &lead);
                let text = match notes.trim() {
                    "" => title.trim().to_string(),
                    notes => format!("{}\n\n{notes}", title.trim()),
                };
                self.create_task(
                    task_id.clone(),
                    title.trim().to_string(),
                    notes.clone(),
                    project_id.clone(),
                    None,
                    None,
                    Vec::new(),
                    Vec::new(),
                )?;
                let lead_brief = tasks::Lead {
                    approve_plan: true,
                    same_agents: false,
                    plan_doc,
                };
                let message_id = match self
                    .start_task(&task_id, &lead, project_id, config, Some(lead_brief))
                    .await
                {
                    Ok(message_id) => message_id,
                    Err(err) => {
                        self.workspace.delete_task(&task_id).ok();
                        return Err(err);
                    }
                };
                self.doc_host
                    .record_intervention(basis, message_id, None, &text);
                Ok(serde_json::json!({ "taskId": task_id, "leadThreadId": lead }))
            }
            SupervisorAction::Pause {
                step_id,
                resource,
                reason,
            } => self.pause_step(from, &step_id, &resource, &reason).await,
            SupervisorAction::Resume {
                step_id,
                resource,
                reason,
            } => {
                self.resume_step(Some(from), &step_id, resource.as_deref(), &reason)
                    .await
            }
            SupervisorAction::Check { task_id } => self.check_resources(task_id.as_deref()).await,
        }
    }

    fn reason_ready(&self, reason: &str, does: &str) -> Result<(), RpcError> {
        if !cfg!(unix) {
            return Err(refused(UNIX_ONLY));
        }
        if reason.trim().is_empty() {
            return Err(refused(&format!(
                "reason is required: the lead reads why its step {does}"
            )));
        }
        Ok(())
    }

    async fn pause_step(
        &self,
        from: &str,
        step_id: &str,
        resource: &str,
        reason: &str,
    ) -> Result<serde_json::Value, RpcError> {
        if !self.registry.supervisor_settings().pause_steps {
            return Err(refused(PAUSE_OFF));
        }
        self.reason_ready(reason, "stopped")?;
        let key = ResourceKey::parse(resource).ok_or_else(|| {
            refused("resource is port:<n>, slot:<name> or path:<path>, as the mission declared it")
        })?;
        let step = self.step_or_refuse(step_id)?;
        if step.outcome.is_some() {
            return Err(refused("This step is already marked done or skipped"));
        }
        let task = self.task_or_refuse(&step.task_id)?;
        let lead = self.lead_of(&task)?;
        let store = self.supervisor_store()?;
        if store.hold_of(step_id).is_some() {
            return Err(refused("This step is already paused"));
        }
        let contention = store
            .resources(|resources| resources.open(&key).cloned())
            .filter(|contention| contention.missions.contains(&task.id))
            .ok_or_else(|| {
                refused(&format!(
                    "No verified contention on {key} involves this step's mission: a pause needs the machine to show one. Check with the supervisor tool, action \"check\"."
                ))
            })?;
        let basis = self.doc_host.intervention_basis(
            InterventionKind::Pause,
            from,
            Some(&lead),
            Some(step_id),
        );
        let used = |own: bool| -> Vec<u32> {
            contention
                .check
                .uses
                .iter()
                .filter(|used| !own || used.task_id.as_deref() == Some(task.id.as_str()))
                .map(|used| used.pid)
                .collect()
        };
        let pids = match used(true) {
            own if own.is_empty() => used(false),
            own => own,
        };
        let stopped = match step.chat_id.as_deref() {
            Some(thread) => self.doc_host.stop_for_pause(thread).map_err(failed)?,
            None => false,
        };
        let row_id = basis.id().to_string();
        let reason = reason.trim();
        store
            .add_hold(PauseHold {
                row_id: row_id.clone(),
                step_id: step.id.clone(),
                task_id: task.id.clone(),
                resource: key.to_string(),
                reason: reason.to_string(),
                at: chrono::Utc::now(),
                thread_id: step.chat_id.clone(),
                pids,
                freed_at: None,
                resumed_at: None,
            })
            .map_err(failed)?;
        let thread = match (&step.chat_id, stopped) {
            (Some(thread), true) => format!("Its thread {thread} was stopped."),
            (Some(thread), false) => format!("Its thread {thread} was not running."),
            (None, _) => "It has not started.".to_string(),
        };
        let text = format!(
            "The Supervisor paused step \"{}\" (taskStepId {}) because {key} is contended with another mission.\n\nWhy: {reason}\n\n{thread} Do not run it again until the Supervisor resumes it: create_thread with this taskStepId and send_message_to_thread to its thread are refused until then. Keep working on steps that do not need {key}.",
            step.title, step.id
        );
        let message_id = match self
            .send_to_task_thread(&task, &lead, Some(from), &text, Vec::new())
            .await
        {
            Ok(id) => id,
            Err(err) => {
                store.drop_hold(&row_id).ok();
                return Err(err);
            }
        };
        self.doc_host
            .record_intervention(basis, Some(message_id.clone()), None, reason);
        let now = chrono::Utc::now().timestamp_millis();
        store.resources(|resources| {
            for after in AFTER_PAUSE_MS {
                resources.schedule(now + after);
            }
        });
        store.poke();
        Ok(serde_json::json!({
            "stepId": step.id, "leadThreadId": lead, "messageId": message_id, "stopped": stopped,
        }))
    }

    async fn resume_step(
        &self,
        from: Option<&str>,
        step_id: &str,
        resource: Option<&str>,
        reason: &str,
    ) -> Result<serde_json::Value, RpcError> {
        if from.is_some() {
            self.reason_ready(reason, "resumes")?;
        }
        let store = self.supervisor_store()?;
        let hold = store
            .hold_of(step_id)
            .ok_or_else(|| refused("This step is not paused"))?;
        if let Some(resource) = resource
            && ResourceKey::parse(resource).map(|key| key.to_string())
                != Some(hold.resource.clone())
        {
            return Err(refused(&format!(
                "This step is paused for {}, not {resource}",
                hold.resource
            )));
        }
        let Some(step) = self.workspace.task_step(step_id) else {
            store.resume_hold(&hold.row_id).map_err(failed)?;
            return Ok(serde_json::json!({ "stepId": step_id, "removed": true }));
        };
        let task = self.task_or_refuse(&step.task_id)?;
        let lead = self.lead_of(&task)?;
        let basis = from.map(|from| {
            self.doc_host.intervention_basis(
                InterventionKind::Resume,
                from,
                Some(&lead),
                Some(step_id),
            )
        });
        store.resume_hold(&hold.row_id).map_err(failed)?;
        let reason = reason.trim();
        let who = if from.is_some() {
            "The Supervisor"
        } else {
            "The person"
        };
        let why = if reason.is_empty() {
            String::new()
        } else {
            format!("\n\nWhy: {reason}")
        };
        let text = format!(
            "{who} resumed step \"{}\" (taskStepId {}), paused for {}.{why}\n\nStart it again with create_thread and taskStepId {} when it is next to run.",
            step.title, step.id, hold.resource, step.id
        );
        let message_id = self
            .send_to_task_thread(&task, &lead, from, &text, Vec::new())
            .await?;
        if let Some(basis) = basis {
            self.doc_host
                .record_intervention(basis, Some(message_id.clone()), None, reason);
        }
        Ok(serde_json::json!({ "stepId": step.id, "leadThreadId": lead, "messageId": message_id }))
    }

    async fn check_resources(&self, task_id: Option<&str>) -> Result<serde_json::Value, RpcError> {
        if !cfg!(unix) {
            return Err(refused(UNIX_ONLY));
        }
        if let Some(task_id) = task_id {
            self.task_or_refuse(task_id)?;
        }
        self.doc_host.check_resources_now().await;
        let store = self.supervisor_store()?;
        let (rows, checked) =
            store.resources(|resources| (resources.rows().to_vec(), resources.checked()));
        let mine = |id: &str| task_id.is_none_or(|task_id| task_id == id);
        let rows: Vec<_> = rows
            .into_iter()
            .filter(|row| row.declared_by.iter().any(|id| mine(id)))
            .collect();
        let paused: Vec<serde_json::Value> = store
            .holds()
            .into_iter()
            .filter(|hold| hold.resumed_at.is_none() && mine(&hold.task_id))
            .map(|hold| {
                serde_json::json!({
                    "stepId": hold.step_id, "taskId": hold.task_id, "resource": hold.resource,
                    "reason": hold.reason, "at": hold.at, "freed": hold.freed_at.is_some(),
                })
            })
            .collect();
        let mut reply = serde_json::json!({ "resources": rows, "paused": paused });
        if !checked {
            reply["note"] = "No two running missions share a declared resource, so the machine was not checked.".into();
        }
        Ok(reply)
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn raise_item(
        &self,
        raised_by: Raiser,
        raiser: &str,
        task_id: Option<String>,
        title: String,
        why: ItemWhy,
        detail: String,
        options: Vec<String>,
    ) -> Result<SupervisorItem, RpcError> {
        let title = title.trim().to_string();
        if title.is_empty() || title.chars().count() > zeron_doc::TASK_TITLE_MAX_CHARS {
            return Err(refused("An item needs a title of up to 500 characters"));
        }
        if detail.chars().count() > zeron_doc::TASK_NOTES_MAX_CHARS {
            return Err(refused("An item's detail is limited to 10,000 characters"));
        }
        let options: Vec<String> = options
            .iter()
            .map(|option| option.trim().to_string())
            .filter(|option| !option.is_empty())
            .collect();
        if options.is_empty() || options.len() > MAX_OPTIONS {
            return Err(refused("An item offers one to three options"));
        }
        let item = SupervisorItem {
            id: crate::new_id(),
            at: chrono::Utc::now(),
            raised_by,
            raiser_chat_id: raiser.to_string(),
            task_id,
            title,
            why,
            detail,
            options,
            answer: None,
        };
        self.supervisor_store()?
            .add_item(item.clone())
            .map_err(failed)?;
        Ok(item)
    }

    pub(super) async fn resolve_supervisor_item(
        &self,
        item_id: &str,
        choice: &str,
    ) -> Result<(), RpcError> {
        let choice = choice.trim();
        if choice.is_empty() {
            return Err(refused("choice is required"));
        }
        let item = self
            .supervisor_store()?
            .answer_item(item_id, choice)
            .map_err(failed)?;
        let raiser = match item.raised_by {
            Raiser::Lead => "the lead",
            Raiser::Supervisor => "the Supervisor",
        };
        if let Some(task) = item
            .task_id
            .as_deref()
            .and_then(|id| self.workspace.task(id))
            && let Ok(lead) = self.lead_of(&task)
        {
            let text = format!(
                "The person answered Needs-you item {} \"{}\", raised by {raiser}: {choice}\n\nApply it to this mission (task id {}).",
                item.id, item.title, task.id
            );
            self.send_to_task_thread(&task, &lead, None, &text, Vec::new())
                .await?;
        }
        if item.raised_by == Raiser::Supervisor
            && self.doc_host.is_live_supervisor(&item.raiser_chat_id)
        {
            let text = format!(
                "The person answered your Needs-you item {} \"{}\": {choice}",
                item.id, item.title
            );
            self.doc_host
                .queue_thread_message(&item.raiser_chat_id, None, &text)
                .await
                .map_err(failed)?;
        }
        Ok(())
    }

    pub(super) fn set_mission_supervised(
        &self,
        task_id: &str,
        supervised: bool,
    ) -> Result<(), RpcError> {
        self.supervisor_store()?
            .set_supervised(task_id, supervised)
            .map_err(failed)
    }
}
