use super::*;
use crate::supervisor::{PauseHold, SupervisorStore};
use crate::supervisor_resources::AFTER_PAUSE_MS;
use zeron_proto::supervisor::{
    CHAT_PREFIX, CHAT_TITLE, InterventionKind, ItemWhy, PlanApproval, Raiser, ResourceKey,
    SupervisorChoice, SupervisorItem, SupervisorSettings, is_supervisor_chat,
};

pub(super) const MESSAGE_OFF: &str = "Messaging leads and step threads is off in the Supervisor's Authority; the person can turn it on on the Supervisor page.";
pub(super) const START_OFF: &str = "Starting missions is off in the Supervisor's Authority, and create_thread and delegate_task go with it; the person can turn it on on the Supervisor page.";
pub(super) const NO_INTERRUPT: &str = "The Supervisor does not interrupt threads: a step that blocks a shared resource is stopped with supervisor pause, which needs a verified contention. Otherwise message the step's lead.";
const PAUSE_OFF: &str = "Pausing steps is off in the Supervisor's Authority; the person can turn it on on the Supervisor page.";
const UNIX_ONLY: &str = "Shared resources are checked against the machine on macOS and Linux only";
pub(super) const GOAL_OR_SPEND: &str = "A task's title, notes and thread are the person's: the Supervisor cannot change them, delete the task or reassign it. Raise it with the supervisor tool, action \"ask\".";
pub(super) const GOAL_ACTION: &str = "The Supervisor changes a mission's title and notes with the supervisor tool, action \"goal\", which tells its lead. It cannot delete or reassign a task.";
const DECIDE_OFF: &str = "Decide for me is off in the Supervisor's Authority: the person decides the leads' Needs you items and approves plans. Raise it with the supervisor tool, action \"ask\", if it needs them.";
const GOAL_OFF: &str = "Changing a mission's goal or spend is off in the Supervisor's Authority: it is the person's decision. Raise it with the supervisor tool, action \"ask\".";
const TAKEN_BACK: &str =
    "The person took back your approval of this plan, so its review is theirs.";
const UNWATCHED: &str = "The person turned off the Supervisor's watch on this mission, so its decisions stay with the person.";
const NO_PLAN: &str = "Applies once this tree has a plan: decide, approve and goal act on a mission's plan, and this thread's tree has none.";
const TREE_UNWATCHED: &str =
    "The Supervisor does not watch this thread's tree, so its questions are the person's.";
const TREE_DECIDE_OFF: &str = "Decide for me is off in the Supervisor's Authority: in a tree without a plan the person answers the questions. Raise it with the supervisor tool, action \"ask\", if it needs them.";
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
    #[serde(rename_all = "camelCase")]
    Decide {
        item_id: String,
        choice: String,
        reason: String,
    },
    #[serde(rename_all = "camelCase")]
    Approve { task_id: String, reason: String },
    #[serde(rename_all = "camelCase")]
    Goal {
        task_id: String,
        #[serde(default)]
        title: Option<String>,
        #[serde(default)]
        notes: Option<String>,
        reason: String,
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

fn given<'a>(reason: &'a str, who_reads: &str) -> Result<&'a str, RpcError> {
    match reason.trim() {
        "" => Err(refused(&format!("reason is required: {who_reads}"))),
        reason => Ok(reason),
    }
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
                let before = self.registry.supervisor_settings();
                self.registry.set_supervisor_settings(p);
                let store = self.supervisor_store()?;
                for item in store.open_items().iter().filter(|item| {
                    item.raised_by == Raiser::Lead
                        && p.decides(item.why)
                        && !before.decides(item.why)
                }) {
                    self.doc_host.supervisor_escalated(item, true);
                }
                store.poke();
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
        if let Some(reason) = self.cannot_run(&config) {
            return Err(refused(&reason));
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

    fn cannot_run(&self, config: &ChatConfig) -> Option<String> {
        if config.instance_id.is_some() {
            return None;
        }
        let name = config.harness.display_name();
        let other = match &config.profile_id {
            Some(profile) => format!("give the {profile} profile another agent"),
            None => "pick another agent".to_string(),
        };
        if !self.registry.installed_for(config.harness) {
            Some(format!(
                "{name} is not installed on this Mac. Install it, or {other}."
            ))
        } else if self.registry.turned_off(config.harness) {
            Some(format!(
                "{name} is turned off in Settings > Providers. Turn it on, or {other}."
            ))
        } else {
            None
        }
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
                    attach: false,
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
                self.resume_step(from, &step_id, resource.as_deref(), &reason)
                    .await
            }
            SupervisorAction::Check { task_id } => self.check_resources(task_id.as_deref()).await,
            SupervisorAction::Decide {
                item_id,
                choice,
                reason,
            } => self.decide_item(from, &item_id, &choice, &reason).await,
            SupervisorAction::Approve { task_id, reason } => {
                if !settings.decide_for_me {
                    return Err(refused(DECIDE_OFF));
                }
                let reason = given(&reason, "the lead reads why its plan was approved")?;
                let (task, lead) = self.watched_mission(&task_id)?;
                if matches!(
                    self.supervisor_store()?.approval_of(&task.id),
                    Some((_, PlanApproval::TakenBack { .. }))
                ) {
                    return Err(refused(TAKEN_BACK));
                }
                self.plan_awaiting_approval(&task.id)?;
                let starts_at = chrono::Utc::now()
                    + chrono::Duration::milliseconds(crate::supervisor::approval_hold_ms());
                let basis = self
                    .doc_host
                    .intervention_basis(InterventionKind::Approve, from, Some(&lead), None)
                    .with_approval(PlanApproval::Held { starts_at });
                self.doc_host.record_intervention(basis, None, None, reason);
                self.set_plan_pending(&task.id, false)?;
                Ok(
                    serde_json::json!({ "taskId": task.id, "leadThreadId": lead, "startsAt": starts_at }),
                )
            }
            SupervisorAction::Goal {
                task_id,
                title,
                notes,
                reason,
            } => {
                if !settings.goal_and_spend {
                    return Err(refused(GOAL_OFF));
                }
                self.change_goal(from, &task_id, title, notes, &reason)
                    .await
            }
        }
    }

    pub(super) fn supervisor_may_answer(&self, from: &str, chat_id: &str) -> Result<(), RpcError> {
        let chat = self
            .workspace
            .chat(chat_id)
            .map_err(failed)?
            .ok_or_else(|| refused("No thread has that id"))?;
        let tree = self.doc_host.tree_of_chat(&chat).map_err(failed)?;
        if tree.task_id.is_some() || tree.root == from {
            return Ok(());
        }
        if !self.doc_host.tree_watched(self.supervisor_store()?, &tree) {
            return Err(refused(TREE_UNWATCHED));
        }
        if !self.registry.supervisor_settings().decide_for_me {
            return Err(refused(TREE_DECIDE_OFF));
        }
        Ok(())
    }

    fn watched_mission(&self, task_id: &str) -> Result<(zeron_proto::Task, String), RpcError> {
        let Some(task) = self.workspace.task(task_id) else {
            let thread = self.workspace.chat(task_id).map_err(failed)?;
            return Err(refused(match thread {
                Some(_) => NO_PLAN,
                None => "Task no longer exists",
            }));
        };
        if !task.has_plan() {
            return Err(refused(NO_PLAN));
        }
        if self.supervisor_store()?.is_unwatched(&task.id) {
            return Err(refused(UNWATCHED));
        }
        let lead = self.lead_of(&task)?;
        Ok((task, lead))
    }

    async fn decide_item(
        &self,
        from: &str,
        item_id: &str,
        choice: &str,
        reason: &str,
    ) -> Result<serde_json::Value, RpcError> {
        let settings = self.registry.supervisor_settings();
        let store = self.supervisor_store()?;
        let item = store
            .item(item_id)
            .ok_or_else(|| refused("No Needs-you item has that itemId"))?;
        if item.raised_by != Raiser::Lead {
            return Err(refused(
                "The Supervisor decides only items a lead raised; its own asks are the person's to answer",
            ));
        }
        if !settings.decides(item.why) {
            return Err(refused(match item.why {
                ItemWhy::Decision => DECIDE_OFF,
                ItemWhy::Goal | ItemWhy::Spend => GOAL_OFF,
            }));
        }
        if item.answer.is_some() {
            return Err(refused("This item is already answered"));
        }
        let choice = item
            .options
            .iter()
            .find(|option| option.as_str() == choice.trim())
            .ok_or_else(|| {
                refused(&format!(
                    "choice is one of the item's options: {}",
                    item.options.join(" | ")
                ))
            })?
            .clone();
        let reason = given(reason, "the person and the lead read why you chose it")?;
        let task_id = item
            .task_id
            .as_deref()
            .ok_or_else(|| refused("This item names no mission"))?;
        let (task, lead) = self.watched_mission(task_id)?;
        let basis = self
            .doc_host
            .intervention_basis(InterventionKind::Decide, from, Some(&lead), None)
            .with_item(&item.id);
        let decided = SupervisorChoice {
            at: chrono::Utc::now(),
            choice: choice.clone(),
            reason: reason.to_string(),
        };
        store
            .answer_item(&item.id, &choice, Some(decided))
            .map_err(failed)?;
        let text = format!(
            "The Supervisor decided Needs-you item {} \"{}\", raised by the lead: {choice}\n\nWhy: {reason}\n\nThe person gave the Supervisor this decision. Apply it to this mission (task id {}). The person may change it; a change reaches you as a correction.",
            item.id, item.title, task.id
        );
        let message_id = match self
            .send_to_task_thread(&task, &lead, None, &text, Vec::new())
            .await
        {
            Ok(id) => id,
            Err(err) => {
                store.reopen_item(&item.id).ok();
                return Err(err);
            }
        };
        self.doc_host
            .record_intervention(basis, Some(message_id.clone()), None, reason);
        Ok(serde_json::json!({
            "itemId": item.id, "choice": choice, "leadThreadId": lead, "messageId": message_id,
        }))
    }

    async fn change_goal(
        &self,
        from: &str,
        task_id: &str,
        title: Option<String>,
        notes: Option<String>,
        reason: &str,
    ) -> Result<serde_json::Value, RpcError> {
        let reason = given(reason, "the lead reads why the goal changed")?;
        let (task, lead) = self.watched_mission(task_id)?;
        let title = title
            .map(|title| title.trim().to_string())
            .filter(|title| *title != task.title);
        if title.as_deref().is_some_and(|title| {
            title.is_empty() || title.chars().count() > zeron_doc::TASK_TITLE_MAX_CHARS
        }) {
            return Err(refused("A mission needs a title of up to 500 characters"));
        }
        let notes = notes.filter(|notes| *notes != task.notes);
        if notes
            .as_deref()
            .is_some_and(|notes| notes.chars().count() > zeron_doc::TASK_NOTES_MAX_CHARS)
        {
            return Err(refused(
                "A mission's notes are limited to 10,000 characters",
            ));
        }
        let mut changes = Vec::new();
        if let Some(title) = &title {
            changes.push(format!(
                "The title is now \"{title}\" (it was \"{}\").",
                task.title
            ));
        }
        match notes.as_deref() {
            Some("") => changes.push("The notes are now empty.".to_string()),
            Some(notes) => changes.push(format!("The notes are now:\n{notes}")),
            None => {}
        }
        if changes.is_empty() {
            return Err(refused("That is already this mission's title and notes"));
        }
        let basis =
            self.doc_host
                .intervention_basis(InterventionKind::Goal, from, Some(&lead), None);
        self.update_task(
            &task.id,
            &zeron_proto::TaskPatch {
                title,
                notes,
                ..Default::default()
            },
        )?;
        let text = format!(
            "The Supervisor changed this mission's goal (task id {}), with the person's leave:\n- {}\n\nWhy: {reason}\n\nCheck the plan against it with the tasks tool: action \"list\", taskId \"{}\", and rewrite the steps that no longer fit with action \"plan\". The person may change the goal back.",
            task.id,
            changes.join("\n- "),
            task.id
        );
        let message_id = self
            .send_to_task_thread(&task, &lead, None, &text, Vec::new())
            .await?;
        self.doc_host
            .record_intervention(basis, Some(message_id.clone()), None, reason);
        Ok(serde_json::json!({ "taskId": task.id, "leadThreadId": lead, "messageId": message_id }))
    }

    fn pause_ready(&self, reason: &str, does: &str) -> Result<(), RpcError> {
        if !cfg!(unix) {
            return Err(refused(UNIX_ONLY));
        }
        if !self.registry.supervisor_settings().pause_steps {
            return Err(refused(PAUSE_OFF));
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
        self.pause_ready(reason, "stopped")?;
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
            "The Supervisor paused step \"{}\" (taskStepId {}) because {key} is contended with another mission.\n\nWhy: {reason}\n\n{thread} Do not run it again until the Supervisor resumes it: create_thread with this taskStepId is refused until then. Keep working on steps that do not need {key}.",
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
        from: &str,
        step_id: &str,
        resource: Option<&str>,
        reason: &str,
    ) -> Result<serde_json::Value, RpcError> {
        self.pause_ready(reason, "resumes")?;
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
        let step = self.step_or_refuse(step_id)?;
        let task = self.task_or_refuse(&step.task_id)?;
        let lead = self.lead_of(&task)?;
        let basis = self.doc_host.intervention_basis(
            InterventionKind::Resume,
            from,
            Some(&lead),
            Some(step_id),
        );
        store.resume_hold(&hold.row_id).map_err(failed)?;
        let reason = reason.trim();
        let text = format!(
            "The Supervisor resumed step \"{}\" (taskStepId {}), paused for {}.\n\nWhy: {reason}\n\nStart it again with create_thread and taskStepId {} when it is next to run.",
            step.title, step.id, hold.resource, step.id
        );
        let message_id = self
            .send_to_task_thread(&task, &lead, Some(from), &text, Vec::new())
            .await?;
        self.doc_host
            .record_intervention(basis, Some(message_id.clone()), None, reason);
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

    pub(super) async fn resolve_supervisor_items(
        &self,
        answers: Vec<super::ItemChoice>,
    ) -> Result<(), RpcError> {
        if answers.is_empty() {
            return Err(refused("answers is required"));
        }
        let store = self.supervisor_store()?;
        let mut answered: Vec<(SupervisorItem, String, Option<String>)> = Vec::new();
        let mut errors: Vec<RpcError> = Vec::new();
        for answer in answers {
            let choice = answer.choice.trim();
            if choice.is_empty() {
                errors.push(refused("choice is required"));
                continue;
            }
            match store.answer_item(&answer.item_id, choice, None) {
                Ok((item, previous)) => {
                    let replaced = previous.map(|previous| previous.choice);
                    answered.push((item, choice.to_string(), replaced));
                }
                Err(error) => errors.push(failed(error)),
            }
        }
        let mut tasks: Vec<&str> = Vec::new();
        for task_id in answered
            .iter()
            .filter_map(|(item, ..)| item.task_id.as_deref())
        {
            if !tasks.contains(&task_id) {
                tasks.push(task_id);
            }
        }
        for task_id in tasks {
            let Some(task) = self.workspace.task(task_id) else {
                continue;
            };
            let Ok(lead) = self.lead_of(&task) else {
                continue;
            };
            let mine = || {
                answered
                    .iter()
                    .filter(|(item, ..)| item.task_id.as_deref() == Some(task_id))
            };
            let lines: Vec<String> = mine()
                .map(|(item, choice, replaced)| {
                    let raiser = match item.raised_by {
                        Raiser::Lead => "the lead",
                        Raiser::Supervisor => "the Supervisor",
                    };
                    let correction = replaced
                        .as_deref()
                        .map(|old| {
                            format!(" (a correction: this replaces the Supervisor's \"{old}\")")
                        })
                        .unwrap_or_default();
                    format!(
                        "{} \"{}\", raised by {raiser}: {choice}{correction}",
                        item.id, item.title
                    )
                })
                .collect();
            let undo = if mine().any(|(.., replaced)| replaced.is_some()) {
                " If you already acted on the Supervisor's choice, undo or adjust what depends on it where you can, and tell the person what could not be undone."
            } else {
                ""
            };
            let text = match lines.as_slice() {
                [one] => format!(
                    "The person answered Needs-you item {one}\n\nApply it to this mission (task id {}).{undo}",
                    task.id
                ),
                lines => format!(
                    "The person answered {} Needs-you items:\n{}\n\nApply them to this mission (task id {}).{undo}",
                    lines.len(),
                    lines
                        .iter()
                        .map(|line| format!("- item {line}"))
                        .collect::<Vec<_>>()
                        .join("\n"),
                    task.id
                ),
            };
            self.send_to_task_thread(&task, &lead, None, &text, Vec::new())
                .await?;
        }
        let mut raisers: Vec<&str> = Vec::new();
        for (item, ..) in &answered {
            if item.raised_by == Raiser::Supervisor
                && !raisers.contains(&item.raiser_chat_id.as_str())
                && self.doc_host.is_live_supervisor(&item.raiser_chat_id)
            {
                raisers.push(&item.raiser_chat_id);
            }
        }
        for raiser in raisers {
            let lines: Vec<String> = answered
                .iter()
                .filter(|(item, ..)| {
                    item.raised_by == Raiser::Supervisor && item.raiser_chat_id == raiser
                })
                .map(|(item, choice, _)| format!("{} \"{}\": {choice}", item.id, item.title))
                .collect();
            let text = match lines.as_slice() {
                [one] => format!("The person answered your Needs-you item {one}"),
                lines => format!(
                    "The person answered your Needs-you items:\n{}",
                    lines
                        .iter()
                        .map(|line| format!("- item {line}"))
                        .collect::<Vec<_>>()
                        .join("\n")
                ),
            };
            self.doc_host
                .queue_thread_message(raiser, None, &text)
                .await
                .map_err(failed)?;
        }
        let corrections: Vec<String> = answered
            .iter()
            .filter_map(|(item, choice, replaced)| {
                Some(format!(
                    "- item {} \"{}\": {choice}, replacing your \"{}\"",
                    item.id,
                    item.title,
                    replaced.as_deref()?
                ))
            })
            .collect();
        if !corrections.is_empty()
            && let Some(supervisor) = store.chat_id()
            && self.doc_host.is_live_supervisor(&supervisor)
        {
            let text = format!(
                "The person changed your decisions; each lead got the correction:\n{}",
                corrections.join("\n")
            );
            self.doc_host
                .queue_thread_message(&supervisor, None, &text)
                .await
                .map_err(failed)?;
        }
        match errors.len() {
            0 => Ok(()),
            1 => Err(errors.remove(0)),
            _ => Err(RpcError::Failed(
                errors
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join("; "),
            )),
        }
    }

    pub(super) fn set_tree_watched(&self, chat_id: &str, watched: bool) -> Result<(), RpcError> {
        let chat = self
            .workspace
            .chat(chat_id)
            .map_err(failed)?
            .ok_or_else(|| refused("No thread has that id"))?;
        let root = self.doc_host.tree_of_chat(&chat).map_err(failed)?.root;
        if zeron_proto::supervisor::is_supervisor_chat(&root)
            || zeron_proto::voice::is_orchestrator_chat(&root)
        {
            return Err(refused(
                "System threads and the threads they open are never a watched tree",
            ));
        }
        self.supervisor_store()?
            .set_tree_watched(&root, watched)
            .map_err(failed)
    }

    pub(super) fn set_watch_everything(&self, on: bool) -> Result<(), RpcError> {
        self.supervisor_store()?
            .set_watch_everything(on)
            .map_err(failed)
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
