use super::*;
use crate::supervisor::SupervisorStore;
use zeron_proto::supervisor::{
    CHAT_PREFIX, CHAT_TITLE, InterventionKind, ItemWhy, Raiser, SupervisorItem, SupervisorSettings,
    is_supervisor_chat,
};

pub(super) const MESSAGE_OFF: &str = "Messaging leads and step threads is off in the Supervisor's Authority; the person can turn it on on the Supervisor page.";
pub(super) const START_OFF: &str = "Starting missions is off in the Supervisor's Authority, and create_thread and delegate_task go with it; the person can turn it on on the Supervisor page.";
pub(super) const NO_INTERRUPT: &str = "The Supervisor does not interrupt threads: stopping a step goes through supervisor pause, which this build does not have yet. Message the step's lead instead.";
pub(super) const GOAL_OR_SPEND: &str = "A task's title, notes and thread are the person's: the Supervisor cannot change them, delete the task or reassign it. Raise it with the supervisor tool, action \"ask\".";
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
                };
                let rebrief = tasks::Rebrief {
                    supervisor: from,
                    reason: &reason,
                };
                let message_id = self
                    .edit_task_plan(&task.id, None, vec![edit], &[], Some(rebrief))
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
        }
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
            self.send_to_task_thread(&task, &lead, None, &text).await?;
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
