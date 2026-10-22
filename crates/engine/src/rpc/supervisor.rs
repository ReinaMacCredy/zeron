use super::*;
use crate::supervisor::{PauseHold, SupervisorStore};
use crate::supervisor_resources::AFTER_PAUSE_MS;
use zeron_proto::supervisor::{
    CHAT_PREFIX, CHAT_TITLE, InterventionKind, ItemAnswer, ItemWhy, PlanApproval, Raiser,
    ResourceKey, SupervisorChoice, SupervisorItem, SupervisorSettings, is_supervisor_chat,
};

pub(super) const MESSAGE_OFF: &str = "Messaging leads and step threads is off in the Supervisor's Authority; the person can turn it on on the Supervisor page.";
pub(super) const START_OFF: &str = "Starting missions is off in the Supervisor's Authority, and create_thread, delegate_task and review_changes go with it; the person can turn it on on the Supervisor page.";
pub(super) const PERSON_ONLY: &str =
    "Only the person stops the Supervisor or answers its questions; send it a message instead.";
pub(super) const NO_INTERRUPT: &str = "The Supervisor does not interrupt threads: a step that blocks a shared resource is stopped with supervisor pause, which needs a verified contention. Otherwise message the step's lead.";
const PAUSE_OFF: &str = "Pausing steps is off in the Supervisor's Authority; the person can turn it on on the Supervisor page.";
const UNIX_ONLY: &str = "Shared resources are checked against the machine on macOS and Linux only";
pub(super) const GOAL_OR_SPEND: &str = "A task is the person's: the Supervisor cannot change it, mark it done, delete it or reassign it. Raise it with the supervisor tool, action \"ask\".";
pub(super) const GOAL_ACTION: &str = "The Supervisor changes a mission's title and notes with the supervisor tool, action \"goal\", which tells its lead. It cannot otherwise change a task, mark it done, delete it or reassign it.";
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
const OPTION_CHARS: usize = 200;
const REASON_CHARS: usize = 2_000;

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

/// Only the fields a caller names change, so a surface holding a stale copy cannot flip the others back.
fn patched_settings(
    current: SupervisorSettings,
    params: serde_json::Value,
) -> Result<SupervisorSettings, RpcError> {
    let serde_json::Value::Object(patch) = params else {
        return Err(RpcError::BadParams(
            "settings are an object of the fields to change".into(),
        ));
    };
    let mut merged = serde_json::to_value(current).map_err(|e| RpcError::Failed(e.to_string()))?;
    if let serde_json::Value::Object(fields) = &mut merged {
        fields.extend(patch);
    }
    parse_params(merged)
}

fn given<'a>(reason: &'a str, who_reads: &str) -> Result<&'a str, RpcError> {
    match reason.trim() {
        "" => Err(refused(&format!("reason is required: {who_reads}"))),
        reason if reason.chars().count() > REASON_CHARS => {
            Err(refused("reason is limited to 2,000 characters"))
        }
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

    fn refuse_elsewhere(&self, task_id: &str) -> Result<(), RpcError> {
        if self.doc_host.hosts_mission(&self.workspace, task_id) {
            return Ok(());
        }
        Err(refused(
            "This mission's lead is archived or runs on another Mac: the Supervisor reaches only threads on its own Mac",
        ))
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
                let before = self.registry.supervisor_settings();
                let p = patched_settings(before, params)?;
                self.registry.set_supervisor_settings(p);
                let store = self.supervisor_store()?;
                if before.decide_for_me && !p.decide_for_me {
                    for (_, task_id, ..) in store.held_approvals() {
                        self.withdraw_held_approval(&task_id).await?;
                    }
                }
                for item in store.open_items().iter().filter(|item| {
                    item.raised_by == Raiser::Lead
                        && p.decides(item.why)
                        && !before.decides(item.why)
                        && item
                            .task_id
                            .as_deref()
                            .and_then(|id| self.workspace.task(id))
                            .is_some_and(|task| task.completed_at.is_none() && !task.deleted)
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
        if let Some(reason) = self.registry.cannot_run(&config) {
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
                given(&reason, "the lead reads why the step changed")?;
                let step = self.step_or_refuse(&step_id)?;
                let task = self.task_or_refuse(&step.task_id)?;
                self.refuse_elsewhere(&task.id)?;
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
                    role: None,
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
                    .start_task(&task_id, &lead, Some(project_id), config, Some(lead_brief))
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
                let store = self.supervisor_store()?;
                let approving = store.approving();
                let _approving = approving.lock().await;
                if matches!(
                    store.approval_of(&task.id),
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
                    .with_task(&task.id, &lead)
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
        if task.completed_at.is_some() {
            return Err(refused(
                "This mission is marked done, so its decisions, plan and goal stay as they are",
            ));
        }
        if self.supervisor_store()?.is_unwatched(&task.id) {
            return Err(refused(UNWATCHED));
        }
        self.refuse_elsewhere(&task.id)?;
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
            .with_task(&task.id, &lead)
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
                store.revert_answer(&item.id, &choice, None).ok();
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
        let basis = self
            .doc_host
            .intervention_basis(InterventionKind::Goal, from, Some(&lead), None)
            .with_task(&task.id, &lead);
        let restore = zeron_proto::TaskPatch {
            title: title.as_ref().map(|_| task.title.clone()),
            notes: notes.as_ref().map(|_| task.notes.clone()),
            ..Default::default()
        };
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
        let message_id = match self
            .send_to_task_thread(&task, &lead, None, &text, Vec::new())
            .await
        {
            Ok(id) => id,
            Err(err) => {
                self.update_task(&task.id, &restore).ok();
                return Err(err);
            }
        };
        self.doc_host
            .record_intervention(basis, Some(message_id.clone()), None, reason);
        Ok(serde_json::json!({ "taskId": task.id, "leadThreadId": lead, "messageId": message_id }))
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
        if !cfg!(unix) {
            return Err(refused(UNIX_ONLY));
        }
        given(reason, "the lead reads why its step stopped")?;
        let key = ResourceKey::parse(resource).ok_or_else(|| {
            refused("resource is port:<n>, slot:<name> or path:<path>, as the mission declared it")
        })?;
        let step = self.step_or_refuse(step_id)?;
        if step.outcome.is_some() {
            return Err(refused("This step is already marked done or skipped"));
        }
        let task = self.task_or_refuse(&step.task_id)?;
        self.refuse_elsewhere(&task.id)?;
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

    pub(super) async fn resume_step_for_person(
        &self,
        step_id: &str,
        from: Option<&str>,
    ) -> Result<(), RpcError> {
        if from.is_some() {
            return Err(refused(
                "Only the person resumes a step from the Supervisor page",
            ));
        }
        self.resume_step(None, step_id, None, "").await.map(drop)
    }

    async fn resume_step(
        &self,
        from: Option<&str>,
        step_id: &str,
        resource: Option<&str>,
        reason: &str,
    ) -> Result<serde_json::Value, RpcError> {
        if from.is_some() {
            given(reason, "the lead reads why its step resumes")?;
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
        if options
            .iter()
            .any(|option| option.chars().count() > OPTION_CHARS)
        {
            return Err(refused("An option is limited to 200 characters"));
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
        let mut answered: Vec<Answered> = Vec::new();
        let mut errors: Vec<RpcError> = Vec::new();
        for answer in answers {
            let choice = answer.choice.trim();
            if choice.is_empty() {
                errors.push(refused("choice is required"));
                continue;
            }
            match store.answer_item(&answer.item_id, choice, None) {
                Ok((item, previous)) => answered.push(Answered {
                    item,
                    choice: choice.to_string(),
                    previous,
                }),
                Err(error) => errors.push(failed(error)),
            }
        }
        let mut tasks: Vec<String> = Vec::new();
        for task_id in answered.iter().filter_map(|a| a.item.task_id.clone()) {
            if !tasks.contains(&task_id) {
                tasks.push(task_id);
            }
        }
        let mut untold: Vec<String> = Vec::new();
        for task_id in tasks {
            let mine: Vec<&Answered> = answered
                .iter()
                .filter(|a| a.item.task_id.as_deref() == Some(task_id.as_str()))
                .collect();
            let Some(task) = self.workspace.task(&task_id) else {
                continue;
            };
            let Ok(lead) = self.lead_of(&task) else {
                continue;
            };
            let text = person_answer_text(&task.id, &mine);
            if let Err(err) = self
                .send_to_task_thread(&task, &lead, None, &text, Vec::new())
                .await
            {
                for a in &mine {
                    store
                        .revert_answer(&a.item.id, &a.choice, a.previous.clone())
                        .ok();
                    untold.push(a.item.id.clone());
                }
                errors.push(refused(&format!(
                    "The lead of task {task_id} could not be told, so its items stay open: {err}"
                )));
            }
        }
        let told = || answered.iter().filter(|a| !untold.contains(&a.item.id));
        let mut raisers: Vec<&str> = Vec::new();
        for a in told() {
            if a.item.raised_by == Raiser::Supervisor
                && !raisers.contains(&a.item.raiser_chat_id.as_str())
                && self.doc_host.is_live_supervisor(&a.item.raiser_chat_id)
            {
                raisers.push(&a.item.raiser_chat_id);
            }
        }
        for raiser in raisers {
            let lines: Vec<String> = told()
                .filter(|a| {
                    a.item.raised_by == Raiser::Supervisor && a.item.raiser_chat_id == raiser
                })
                .map(|a| format!("{} \"{}\": {}", a.item.id, a.item.title, a.choice))
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
            if let Err(err) = self
                .doc_host
                .queue_thread_message(raiser, None, &text)
                .await
            {
                errors.push(failed(err));
            }
        }
        let corrections: Vec<String> = told()
            .filter_map(|a| {
                let previous = a.previous.as_ref()?;
                let replaced = match a.supervisors_own() {
                    true => format!("replacing your \"{}\"", previous.choice),
                    false => format!("replacing their earlier \"{}\"", previous.choice),
                };
                Some(format!(
                    "- item {} \"{}\": {}, {replaced}",
                    a.item.id, a.item.title, a.choice
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
            if let Err(err) = self
                .doc_host
                .queue_thread_message(&supervisor, None, &text)
                .await
            {
                errors.push(failed(err));
            }
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

    pub(super) async fn set_mission_supervised(
        &self,
        task_id: &str,
        supervised: bool,
    ) -> Result<(), RpcError> {
        self.supervisor_store()?
            .set_supervised(task_id, supervised)
            .map_err(failed)?;
        if !supervised {
            self.withdraw_held_approval(task_id).await?;
        }
        Ok(())
    }
}

struct Answered {
    item: SupervisorItem,
    choice: String,
    previous: Option<ItemAnswer>,
}

impl Answered {
    fn supervisors_own(&self) -> bool {
        self.previous.as_ref().is_some_and(|previous| {
            previous
                .supervisor
                .as_ref()
                .is_some_and(|decided| decided.choice == previous.choice)
        })
    }
}

fn person_answer_text(task_id: &str, answered: &[&Answered]) -> String {
    let lines: Vec<String> = answered
        .iter()
        .map(|a| {
            let raiser = match a.item.raised_by {
                Raiser::Lead => "the lead",
                Raiser::Supervisor => "the Supervisor",
            };
            let correction = match (&a.previous, a.supervisors_own()) {
                (Some(previous), true) => format!(
                    " (a correction: this replaces the Supervisor's \"{}\")",
                    previous.choice
                ),
                (Some(previous), false) => format!(
                    " (a correction: this replaces the person's earlier \"{}\")",
                    previous.choice
                ),
                (None, _) => String::new(),
            };
            format!(
                "{} \"{}\", raised by {raiser}: {}{correction}",
                a.item.id, a.item.title, a.choice
            )
        })
        .collect();
    let corrected = || answered.iter().filter(|a| a.previous.is_some());
    let undo = if corrected().next().is_none() {
        ""
    } else if corrected().all(|a| a.supervisors_own()) {
        " If you already acted on the Supervisor's choice, undo or adjust what depends on it where you can, and tell the person what could not be undone."
    } else {
        " If you already acted on the earlier choice, undo or adjust what depends on it where you can, and tell the person what could not be undone."
    };
    match lines.as_slice() {
        [one] => format!(
            "The person answered Needs-you item {one}\n\nApply it to this mission (task id {task_id}).{undo}"
        ),
        lines => format!(
            "The person answered {} Needs-you items:\n{}\n\nApply them to this mission (task id {task_id}).{undo}",
            lines.len(),
            lines
                .iter()
                .map(|line| format!("- item {line}"))
                .collect::<Vec<_>>()
                .join("\n"),
        ),
    }
}
