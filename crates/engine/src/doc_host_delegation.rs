use chrono::Utc;
use serde::Deserialize;
use zeron_proto::{
    Chat, ChatConfig, Delegation, DelegationDelivery, DelegationStatus, DelegationWake,
    PermissionMode, ReasoningLevel, Session, SessionStatus, UserInputQuestion,
};

use super::*;
use crate::registry::DelegationSettings;

pub const DELEGATION_GUIDANCE: &str = "## Zeron delegation

The `zeron` MCP server runs child agents on any installed harness and model. Treat these distinctly:

- A delegated task is child work owned by this chat. Call `orchestrator_capabilities` for the harnesses and models available, from the same catalog as the composer; a native subagent tool's model list is not the full list. When it lists agent profiles, pick the one whose `whenToUse` fits the task and pass its id as `profile`. Prefer native subagent tools for same-harness work only when they support the chosen model. Use `delegate_task` with that harness and model when native tools cannot, including for same-harness work, for a different harness, or when the user asks for a Zeron child task. Keep each returned `taskId` and use `task_status` or `task_cancel` with it. Its `childThreadId` is backing storage for the child; do not replace delegation with `create_thread`.
- An async child's completion wakes this chat with a message naming its taskId, steered into an active turn where supported or queued otherwise, so end the turn instead of polling or spawning watchers, then read the result with `task_status`. Use mode `wait` only when this turn needs the result before it can continue.
- A child that blocks on a question or a tool approval wakes this chat the same way, as does any thread this chat watches with `watch_threads`. Answer with `respond_to_input` when the answer follows from the task or an earlier decision (a tool approval needs this chat at Full access); anything irreversible or that only the person can decide goes to them with `ask_user`.
- `create_thread` opens an ordinary top-level chat. Use it only when the user asks for separate/new threads or conversations, never merely because they said \"subagent\" or asked for parallel delegated work.

Tool names may carry a harness prefix such as `mcp__zeron__delegate_task`; the semantics are the same. Use a stable `clientRequestId` when retrying `delegate_task`.";

const SUMMARY_CHARS: usize = 600;
const TITLE_CHARS: usize = 72;
const ROLES: [&str; 6] = [
    "implementation",
    "research",
    "review",
    "design",
    "test",
    "general",
];

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DelegateTaskParams {
    pub owner_chat_id: String,
    pub task: String,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub role: Option<String>,
    #[serde(default)]
    pub harness: Option<HarnessId>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub reasoning: Option<ReasoningLevel>,
    #[serde(flatten)]
    pub profile: ProfileParams,
    #[serde(default)]
    pub wait: bool,
    #[serde(default)]
    pub wake: Option<DelegationWake>,
    #[serde(default)]
    pub client_request_id: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProfileParams {
    #[serde(default)]
    pub permission_mode: Option<PermissionMode>,
    #[serde(default)]
    pub model_options: Option<serde_json::Map<String, serde_json::Value>>,
    #[serde(default)]
    pub instructions: Option<String>,
    #[serde(default)]
    pub profile_id: Option<String>,
    #[serde(default)]
    pub disabled_tools: Vec<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TaskView {
    pub task_id: String,
    pub child_thread_id: String,
    pub title: Option<String>,
    pub harness: Option<HarnessId>,
    pub model: Option<String>,
    pub status: &'static str,
    pub work_state: &'static str,
    pub has_pending_child_runs: bool,
    pub summary: Option<String>,
}

fn failure(code: &str, message: impl std::fmt::Display) -> EngineError {
    EngineError::Other(format!("{code}: {message}"))
}

fn clip(text: &str, max: usize) -> String {
    let text = text.trim();
    if text.chars().count() <= max {
        return text.to_string();
    }
    let mut clipped: String = text.chars().take(max.saturating_sub(3)).collect();
    clipped.push_str("...");
    clipped
}

pub(super) fn wake_text(task_ids: &[String]) -> String {
    match task_ids {
        [one] => format!(
            "Delegated task {one} reached a terminal state. Use task_status with taskId {one} to read the result."
        ),
        many => format!(
            "Delegated tasks {} reached terminal states. Use task_status with each taskId to read the results.",
            many.join(", ")
        ),
    }
}

const WAKE_QUESTION_CHARS: usize = 1_500;

fn input_wake_text(
    asker: &Chat,
    task: bool,
    request_id: &str,
    questions: &[UserInputQuestion],
) -> String {
    let title = asker.title.as_deref().unwrap_or("Untitled");
    let who = if task {
        format!(
            "Delegated task {} (\"{}\")",
            asker.id,
            clip(title, TITLE_CHARS)
        )
    } else {
        format!(
            "Watched thread {} (\"{}\")",
            asker.id,
            clip(title, TITLE_CHARS)
        )
    };
    let mut text = format!("{who} is waiting for input, requestId {request_id}:\n");
    for question in questions {
        let what = if question.approval {
            "tool approval"
        } else {
            "question"
        };
        text.push_str(&format!(
            "- {what} {}: {}",
            question.id,
            clip(&question.question, WAKE_QUESTION_CHARS)
        ));
        if !question.options.is_empty() {
            let options: Vec<String> = question
                .options
                .iter()
                .map(|option| format!("\"{option}\""))
                .collect();
            text.push_str(&format!(" Options: {}", options.join(" | ")));
        }
        text.push('\n');
    }
    text.push_str(
        "Answer with respond_to_input (threadId, requestId, answers) when the answer follows from the task or an earlier decision; a tool approval needs this chat at Full access. Otherwise ask the person with ask_user, or leave it to them. They see the same question and may answer first, in which case respond_to_input reports nothing pending.",
    );
    text
}

fn status_name(status: DelegationStatus) -> &'static str {
    match status {
        DelegationStatus::Running => "running",
        DelegationStatus::Completed => "completed",
        DelegationStatus::Failed => "failed",
        DelegationStatus::Cancelled => "cancelled",
        DelegationStatus::Interrupted => "interrupted",
    }
}

fn owned_by<'a>(chat: &'a Chat, owner: &str) -> Option<&'a Delegation> {
    chat.delegation
        .as_ref()
        .filter(|d| d.delegated_by_chat_id == owner)
}

/// A session reads Idle before a fresh turn starts, so only an answer to the
/// latest user entry (`reply`) ends a task.
pub(super) fn child_outcome(
    delegation: &Delegation,
    session: Option<&Session>,
    task_queued: bool,
    children_pending: bool,
    reply: Option<MessageStatus>,
    after_restart: bool,
) -> Option<DelegationStatus> {
    if !delegation.is_running() {
        return None;
    }
    let cancelled = delegation.cancel_requested;
    if task_queued {
        return cancelled.then_some(DelegationStatus::Cancelled);
    }
    let status = session.map(|s| s.status);
    if matches!(
        status,
        Some(SessionStatus::Working | SessionStatus::AwaitingInput)
    ) {
        return None;
    }
    if cancelled {
        return Some(DelegationStatus::Cancelled);
    }
    if children_pending {
        return None;
    }
    match (status, reply) {
        (None, _) if !after_restart => None,
        (Some(SessionStatus::Errored), _) => Some(DelegationStatus::Failed),
        (_, Some(MessageStatus::Complete)) => Some(DelegationStatus::Completed),
        (_, Some(MessageStatus::Aborted)) | (None, _) => Some(DelegationStatus::Interrupted),
        (Some(_), _) => None,
    }
}

fn reply_status(entries: &[SessionMessageEntry], anchor: &str) -> Option<MessageStatus> {
    let start = entries.iter().position(|entry| entry.id == anchor)?;
    let asked = entries[start..]
        .iter()
        .rposition(|entry| entry.role == MessageRole::User)
        .map_or(start, |at| start + at);
    entries[asked + 1..]
        .iter()
        .rev()
        .find(|entry| entry.role == MessageRole::Assistant)
        .map(|entry| entry.status.unwrap_or(MessageStatus::Complete))
}

fn result_parts(entries: &[SessionMessageEntry], anchor: &str) -> (Option<String>, Option<String>) {
    let start = entries
        .iter()
        .position(|entry| entry.id == anchor)
        .map_or(entries.len(), |at| at + 1);
    let mut text = None;
    let mut error = None;
    for entry in &entries[start..] {
        if entry.role != MessageRole::Assistant {
            continue;
        }
        for part in &entry.parts {
            match part {
                MessagePart::Text { text: t, .. } if !t.trim().is_empty() => {
                    text = Some(t.trim().to_string());
                }
                MessagePart::Error { message, .. } if !message.trim().is_empty() => {
                    error = Some(message.trim().to_string());
                }
                _ => {}
            }
        }
    }
    (text, error)
}

fn result_text(status: DelegationStatus, text: Option<String>, error: Option<String>) -> String {
    match status {
        DelegationStatus::Failed => error
            .or(text)
            .unwrap_or_else(|| "Child task ended with status failed.".into()),
        DelegationStatus::Completed => {
            text.unwrap_or_else(|| "Child task completed without an assistant result.".into())
        }
        other => {
            text.unwrap_or_else(|| format!("Child task ended with status {}.", status_name(other)))
        }
    }
}

impl DocHost {
    pub(super) fn derived_config(
        &self,
        from: &Chat,
        harness: Option<HarnessId>,
        model: Option<&str>,
        reasoning: Option<ReasoningLevel>,
        profile: &ProfileParams,
    ) -> Result<Option<ChatConfig>, EngineError> {
        let mut config = from.config.clone();
        let model = model.map(str::trim).filter(|model| !model.is_empty());
        if harness.is_some() || model.is_some() || reasoning.is_some() {
            let config = config.get_or_insert_with(|| self.default_chat_config());
            if let Some(harness) = harness {
                // A model id belongs to one harness; carrying it across would
                // hand Codex a Claude id. Reasoning is Zeron's own ladder and
                // survives the switch.
                if harness != config.harness {
                    config.model = None;
                }
                config.harness = harness;
            }
            if let Some(model) = model {
                config.model = Some(model.to_string());
            }
            if let Some(reasoning) = reasoning {
                config.reasoning = Some(reasoning);
            }
        }
        match profile
            .instructions
            .as_deref()
            .map(str::trim)
            .filter(|text| !text.is_empty())
        {
            Some(text) => {
                config
                    .get_or_insert_with(|| self.default_chat_config())
                    .instructions = text.to_string();
            }
            None => {
                if let Some(config) = config.as_mut() {
                    config.instructions.clear();
                }
            }
        }
        if let Some(config) = config.as_mut() {
            config.profile_id = profile.profile_id.clone();
        }
        if !profile.disabled_tools.is_empty() {
            let config = config.get_or_insert_with(|| self.default_chat_config());
            for name in &profile.disabled_tools {
                if !config.disabled_tools.contains(name) {
                    config.disabled_tools.push(name.clone());
                }
            }
        }
        if let Some(options) = &profile.model_options {
            config
                .get_or_insert_with(|| self.default_chat_config())
                .model_options = options.clone();
        }
        if let Some(mode) = profile.permission_mode {
            let config = config.get_or_insert_with(|| self.default_chat_config());
            if mode.rank() > config.permission_mode.rank() {
                return Err(failure(
                    "permission_escalation_denied",
                    "a new chat may only narrow this chat's permission mode",
                ));
            }
            config.permission_mode = mode;
            config.sandbox = mode.sandbox();
        }
        Ok(config)
    }

    fn workspace_or_fail(&self) -> Result<&WorkspaceHost, EngineError> {
        self.workspace()
            .ok_or_else(|| failure("orchestration_error", "workspace not wired"))
    }

    fn default_chat_config(&self) -> ChatConfig {
        ChatConfig {
            harness: self.inner.config.default_harness,
            model: None,
            reasoning: None,
            model_options: Default::default(),
            sandbox: zeron_proto::SandboxLevel::WorkspaceWrite,
            permission_mode: PermissionMode::Ask,
            instructions: String::new(),
            profile_id: None,
            disabled_tools: Vec::new(),
        }
    }

    pub async fn delegate_task(
        &self,
        params: DelegateTaskParams,
        limits: DelegationSettings,
    ) -> Result<TaskView, EngineError> {
        let task = params.task.trim();
        if task.is_empty() {
            return Err(failure("invalid_request", "task is empty"));
        }
        let role = params
            .role
            .as_deref()
            .map(str::trim)
            .filter(|role| !role.is_empty());
        if let Some(role) = role
            && !ROLES.contains(&role)
        {
            return Err(failure(
                "invalid_request",
                format!("role must be one of {}", ROLES.join(", ")),
            ));
        }
        let workspace = self.workspace_or_fail()?;
        let owner = workspace
            .chat(&params.owner_chat_id)?
            .ok_or_else(|| failure("thread_not_found", &params.owner_chat_id))?;
        if !self.is_host(&owner.id) {
            return Err(failure(
                "invalid_request",
                "delegate from the device that hosts this chat",
            ));
        }
        let admitting = self.inner.delegation_lock.lock().await;
        let chats = workspace.read_chats()?;
        let key = params
            .client_request_id
            .as_deref()
            .map(str::trim)
            .filter(|key| !key.is_empty());
        if let Some(key) = key
            && let Some(existing) = chats.iter().find(|chat| {
                owned_by(chat, &owner.id)
                    .is_some_and(|d| d.client_request_id.as_deref() == Some(key))
            })
        {
            return Ok(self.task_view(existing, &chats));
        }
        let depth = owner.delegation.as_ref().map_or(0, |d| d.depth) + 1;
        if depth > limits.max_depth {
            return Err(failure(
                "depth_limit",
                format!(
                    "delegation is limited to {} levels (Settings); this child would be level {depth}",
                    limits.max_depth
                ),
            ));
        }
        let running = chats
            .iter()
            .filter(|chat| owned_by(chat, &owner.id).is_some_and(Delegation::is_running))
            .count();
        if running >= limits.max_running_children as usize {
            return Err(failure(
                "child_limit",
                format!(
                    "this chat already runs {running} delegated tasks, the limit set in Settings"
                ),
            ));
        }
        let config = self.derived_config(
            &owner,
            params.harness,
            params.model.as_deref(),
            params.reasoning,
            &params.profile,
        )?;
        let chat_id = new_id();
        let anchor = new_id();
        let delegation = Delegation {
            delegated_by_chat_id: owner.id.clone(),
            depth,
            anchor: anchor.clone(),
            client_request_id: key.map(str::to_string),
            owner_turn_started_at: self
                .sessions()
                .and_then(|sessions| sessions.session_status(&owner.id))
                .and_then(|session| session.started_at),
            wake: params.wake.unwrap_or(if params.wait {
                DelegationWake::SettledOnly
            } else {
                DelegationWake::Always
            }),
            status: DelegationStatus::Running,
            cancel_requested: false,
            delivery: None,
            summary: None,
            started_at: Utc::now(),
            completed_at: None,
        };
        workspace.create_delegated_chat(
            &chat_id,
            owner.space_id.as_deref(),
            Some(&owner.device_id),
            config,
            owner.cwd.clone(),
            delegation,
        )?;
        drop(admitting);
        if let Err(err) = self
            .start_child(&params, task, role, &owner.id, &chat_id, anchor)
            .await
        {
            workspace.update_chat_delegation(&chat_id, |d| {
                d.status = DelegationStatus::Failed;
                d.summary = Some(clip(&err.to_string(), SUMMARY_CHARS));
                d.completed_at = Some(Utc::now());
                d.delivery = Some(DelegationDelivery::Disposed);
                true
            })?;
            return Err(failure(
                "orchestration_error",
                format!("the delegated task could not start: {err}"),
            ));
        }
        let chats = workspace.read_chats()?;
        let child = chats
            .iter()
            .find(|chat| chat.id == chat_id)
            .ok_or_else(|| failure("orchestration_error", "the child chat vanished"))?;
        Ok(self.task_view(child, &chats))
    }

    async fn start_child(
        &self,
        params: &DelegateTaskParams,
        task: &str,
        role: Option<&str>,
        owner_id: &str,
        chat_id: &str,
        anchor: String,
    ) -> Result<(), EngineError> {
        let workspace = self.workspace_or_fail()?;
        let title = params
            .title
            .as_deref()
            .map(str::trim)
            .filter(|title| !title.is_empty())
            .map_or_else(|| clip(task, TITLE_CHARS), |title| clip(title, 512));
        workspace.rename_chat(chat_id, &title)?;
        if let Some(sessions) = self.sessions() {
            sessions.bind_delegate(owner_id, &params.task, chat_id);
        }
        let prompt = match role {
            None | Some("general") => task.to_string(),
            Some(role) => format!("Act as the {role} sub-agent for this task.\n\n{task}"),
        };
        let handle = self.open(chat_id)?;
        handle.doc.push_queued(&QueuedMessage {
            issued_at: now_ms(),
            computer_generation: Some(handle.doc.computer_generation()),
            ..QueuedMessage::new(anchor.clone(), prompt, self.inner.config.device_id.clone())
        })?;
        handle.publish_queue();
        self.drain_queue(&handle).await;
        if handle.queue_paused.load(Ordering::Acquire) && self.task_queued(chat_id, &anchor) {
            let _drain = handle.drain_lock.lock().await;
            handle.doc.take_queued(&anchor)?;
            handle.publish_queue();
            return Err(EngineError::Other(
                "its first turn failed to dispatch".into(),
            ));
        }
        Ok(())
    }

    fn task_view(&self, child: &Chat, chats: &[Chat]) -> TaskView {
        let delegation = child.delegation.clone().expect("a delegated chat");
        let session = self
            .sessions()
            .and_then(|sessions| sessions.session_status(&child.id));
        let active = session.as_ref().is_some_and(|s| {
            matches!(
                s.status,
                SessionStatus::Working | SessionStatus::AwaitingInput
            )
        });
        let task_queued =
            delegation.is_running() && self.task_queued(&child.id, &delegation.anchor);
        let status = match delegation.status {
            DelegationStatus::Running if task_queued => "queued",
            DelegationStatus::Running
                if session
                    .as_ref()
                    .is_some_and(|s| s.status == SessionStatus::AwaitingInput) =>
            {
                "waiting"
            }
            other => status_name(other),
        };
        let work_state = if !delegation.is_running() {
            "result_available"
        } else if !active && !task_queued && children_pending(chats, &child.id) {
            "waiting_for_children"
        } else {
            "working"
        };
        let summary = (!delegation.is_running()).then(|| {
            self.open(&child.id)
                .ok()
                .and_then(|handle| handle.doc.read_entries().ok())
                .map(|entries| {
                    let (text, error) = result_parts(&entries, &delegation.anchor);
                    result_text(delegation.status, text, error)
                })
                .or(delegation.summary.clone())
                .unwrap_or_default()
        });
        TaskView {
            task_id: child.id.clone(),
            child_thread_id: child.id.clone(),
            title: child.title.clone(),
            harness: child.config.as_ref().map(|c| c.harness),
            model: child.config.as_ref().and_then(|c| c.model.clone()),
            status,
            work_state,
            has_pending_child_runs: !delegation.is_running() && active,
            summary,
        }
    }

    fn task_queued(&self, chat_id: &str, anchor: &str) -> bool {
        self.open(chat_id)
            .ok()
            .and_then(|handle| handle.doc.read_queue().ok())
            .is_some_and(|queue| queue.iter().any(|row| row.id == anchor))
    }

    fn owned_child(&self, owner: &str, task_id: &str) -> Result<(Chat, Vec<Chat>), EngineError> {
        let workspace = self.workspace_or_fail()?;
        let chats = workspace.read_chats()?;
        let child = chats
            .iter()
            .find(|chat| chat.id == task_id && chat.delegated_by() == Some(owner))
            .cloned()
            .ok_or_else(|| {
                failure(
                    "task_not_found",
                    format!("no delegated task {task_id} belongs to this chat"),
                )
            })?;
        Ok((child, chats))
    }

    pub async fn task_status(
        &self,
        owner: &str,
        task_id: &str,
        acknowledge: bool,
    ) -> Result<TaskView, EngineError> {
        let (child, chats) = self.owned_child(owner, task_id)?;
        let view = self.task_view(&child, &chats);
        if acknowledge && view.work_state == "result_available" {
            self.settle_delivery(owner, task_id, DelegationDelivery::Acknowledged)
                .await?;
        }
        Ok(view)
    }

    pub async fn task_wake_always(&self, owner: &str, task_id: &str) -> Result<(), EngineError> {
        let (child, _) = self.owned_child(owner, task_id)?;
        let Some(workspace) = self.workspace() else {
            return Ok(());
        };
        let mut offer = false;
        workspace.update_chat_delegation(&child.id, |d| {
            if d.wake == DelegationWake::Always {
                return false;
            }
            d.wake = DelegationWake::Always;
            if !d.is_running() && d.delivery.is_none() {
                d.delivery = Some(DelegationDelivery::Claimed);
                offer = true;
            }
            true
        })?;
        if offer {
            self.offer_wake(owner, task_id).await?;
        }
        Ok(())
    }

    pub async fn task_cancel(
        &self,
        owner: &str,
        task_id: &str,
    ) -> Result<&'static str, EngineError> {
        let (child, _) = self.owned_child(owner, task_id)?;
        let delegation = child.delegation.clone().expect("a delegated chat");
        if !delegation.is_running() {
            self.settle_delivery(owner, task_id, DelegationDelivery::Disposed)
                .await?;
            return Ok(status_name(delegation.status));
        }
        if let Some(workspace) = self.workspace() {
            workspace.update_chat_delegation(task_id, |d| {
                d.cancel_requested = true;
                if !d.delivery.is_some_and(DelegationDelivery::is_final) {
                    d.delivery = Some(DelegationDelivery::Disposed);
                }
                true
            })?;
        }
        self.drop_from_wakes(owner, &[task_id.to_string()]).await?;
        let handle = self.open(task_id)?;
        {
            let _drain = handle.drain_lock.lock().await;
            if handle.doc.take_queued(&delegation.anchor)?.is_some() {
                handle.publish_queue();
            }
        }
        if let Some(sessions) = self.sessions()
            && sessions.turn_in_flight(task_id)
        {
            self.interrupt_and_pause_queue(&sessions, &handle).await?;
        }
        self.settle_delegations(None).await;
        Ok("cancel_requested")
    }

    async fn settle_delivery(
        &self,
        owner: &str,
        task_id: &str,
        to: DelegationDelivery,
    ) -> Result<(), EngineError> {
        let Some(workspace) = self.workspace() else {
            return Ok(());
        };
        workspace.update_chat_delegation(task_id, |d| {
            if matches!(
                d.delivery,
                Some(DelegationDelivery::Acknowledged | DelegationDelivery::Disposed)
            ) {
                return false;
            }
            d.delivery = Some(to);
            true
        })?;
        self.drop_from_wakes(owner, &[task_id.to_string()]).await
    }

    async fn drop_from_wakes(&self, owner: &str, task_ids: &[String]) -> Result<(), EngineError> {
        let handle = self.open(owner)?;
        let _drain = handle.drain_lock.lock().await;
        let mut changed = false;
        for (index, row) in handle.doc.read_queue()?.into_iter().enumerate().rev() {
            if !row.delegated_tasks.iter().any(|id| task_ids.contains(id)) {
                continue;
            }
            handle.doc.take_queued(&row.id)?;
            changed = true;
            let rest: Vec<String> = row
                .delegated_tasks
                .iter()
                .filter(|id| !task_ids.contains(id))
                .cloned()
                .collect();
            if !rest.is_empty() {
                let text = wake_text(&rest);
                handle.doc.insert_queued(
                    index,
                    &QueuedMessage {
                        text,
                        delegated_tasks: rest,
                        ..row
                    },
                )?;
            }
        }
        if changed {
            handle.publish_queue();
            self.save_snapshot(&handle);
        }
        Ok(())
    }

    pub(super) async fn dispose_turn_wakes(
        &self,
        owner: &str,
        turn_started_at: chrono::DateTime<Utc>,
    ) {
        let Some(workspace) = self.workspace() else {
            return;
        };
        let Ok(chats) = workspace.read_chats() else {
            return;
        };
        let mut disposed = Vec::new();
        for chat in &chats {
            let Some(d) = owned_by(chat, owner) else {
                continue;
            };
            if d.owner_turn_started_at != Some(turn_started_at)
                || d.delivery.is_some_and(DelegationDelivery::is_final)
            {
                continue;
            }
            if workspace
                .update_chat_delegation(&chat.id, |d| {
                    d.delivery = Some(DelegationDelivery::Disposed);
                    true
                })
                .is_ok()
            {
                disposed.push(chat.id.clone());
            }
        }
        if !disposed.is_empty()
            && let Err(err) = self.drop_from_wakes(owner, &disposed).await
        {
            tracing::warn!(chat = %owner, error = %err, "dropping stopped-turn wakes failed");
        }
    }

    pub(super) fn wake_delivered(&self, task_ids: &[String]) {
        let Some(workspace) = self.workspace() else {
            return;
        };
        for id in task_ids {
            if let Err(err) = workspace.update_chat_delegation(id, |d| {
                if d.delivery != Some(DelegationDelivery::Claimed) {
                    return false;
                }
                d.delivery = Some(DelegationDelivery::Delivered);
                true
            }) {
                tracing::warn!(task = %id, error = %err, "marking a wake delivered failed");
            }
        }
    }

    pub(super) fn wake_owner_gone(&self, owner: &str, task_ids: &[String]) -> bool {
        let gone = self
            .workspace()
            .and_then(|ws| ws.chat(owner).ok())
            .is_none_or(|chat| chat.is_none_or(|chat| chat.archived));
        if gone && let Some(workspace) = self.workspace() {
            for id in task_ids {
                let _ = workspace.update_chat_delegation(id, |d| {
                    d.delivery = Some(DelegationDelivery::Disposed);
                    true
                });
            }
        }
        gone
    }

    async fn offer_wake(&self, owner: &str, task_id: &str) -> Result<(), EngineError> {
        let handle = self.open(owner)?;
        let row_id = {
            let _drain = handle.drain_lock.lock().await;
            let queue = handle.doc.read_queue()?;
            if let Some((index, row)) = queue
                .iter()
                .enumerate()
                .find(|(_, row)| !row.delegated_tasks.is_empty() && row.delivery_gate.is_none())
            {
                if row.delegated_tasks.iter().any(|id| id == task_id) {
                    return Ok(());
                }
                let mut row = row.clone();
                row.delegated_tasks.push(task_id.to_string());
                row.text = wake_text(&row.delegated_tasks);
                handle.doc.take_queued(&row.id)?;
                handle.doc.insert_queued(index, &row)?;
                handle.publish_queue();
                self.save_snapshot(&handle);
                return Ok(());
            }
            let row = QueuedMessage {
                issued_at: now_ms(),
                computer_generation: Some(handle.doc.computer_generation()),
                delegated_tasks: vec![task_id.to_string()],
                ..QueuedMessage::new(
                    new_id(),
                    wake_text(&[task_id.to_string()]),
                    self.inner.config.device_id.clone(),
                )
            };
            let slot = handle.steer_slot(&row.id)?;
            handle.doc.insert_queued(slot, &row)?;
            handle.publish_queue();
            self.save_snapshot(&handle);
            row.id
        };
        self.deliver_wake(&handle, &row_id).await
    }

    async fn deliver_wake(
        &self,
        handle: &Arc<ChatDocHandle>,
        row_id: &str,
    ) -> Result<(), EngineError> {
        if handle.queue_paused.load(Ordering::Acquire) {
            return self.wake_paused_owner(handle, row_id).await;
        }
        match self.steer_queued_now(&handle.chat_id, row_id).await {
            Ok(true) => {}
            Ok(false) | Err(_) => self.drain_queue(handle).await,
        }
        Ok(())
    }

    pub(crate) fn offer_input_wakes(
        &self,
        chat_id: &str,
        request_id: &str,
        questions: &[UserInputQuestion],
    ) {
        let Some(asker) = self
            .workspace()
            .and_then(|workspace| workspace.chat(chat_id).ok().flatten())
        else {
            return;
        };
        let owner = asker
            .delegation
            .as_ref()
            .filter(|d| d.is_running() && d.wake != DelegationWake::Never)
            .map(|d| d.delegated_by_chat_id.clone());
        let mut targets: Vec<String> = owner.iter().cloned().collect();
        for watcher in self
            .thread_watches()
            .map(|watches| watches.watchers_of(chat_id))
            .unwrap_or_default()
        {
            if !targets.contains(&watcher) {
                targets.push(watcher);
            }
        }
        targets.retain(|target| target != chat_id);
        if targets.is_empty() {
            return;
        }
        let host = self.clone();
        let request_id = request_id.to_string();
        let questions = questions.to_vec();
        self.spawn_worker(async move {
            for target in targets {
                let live = host
                    .workspace()
                    .and_then(|workspace| workspace.chat(&target).ok().flatten())
                    .is_some_and(|chat| !chat.archived && chat.device_id == asker.device_id);
                if !live {
                    continue;
                }
                let task = owner.as_deref() == Some(target.as_str());
                let text = input_wake_text(&asker, task, &request_id, &questions);
                if let Err(err) = host
                    .queue_input_wake(&target, &asker.id, &request_id, text)
                    .await {
                    tracing::warn!(chat = %target, from = %asker.id, error = %err, "input wake failed");
                }
            }
        });
    }

    async fn queue_input_wake(
        &self,
        target: &str,
        from: &str,
        request_id: &str,
        text: String,
    ) -> Result<(), EngineError> {
        let handle = self.open(target)?;
        let row_id = {
            let _drain = handle.drain_lock.lock().await;
            let row = QueuedMessage {
                issued_at: now_ms(),
                computer_generation: Some(handle.doc.computer_generation()),
                from_chat_id: Some(from.to_string()),
                input_request: Some(request_id.to_string()),
                ..QueuedMessage::new(new_id(), text, self.inner.config.device_id.clone())
            };
            let slot = handle.steer_slot(&row.id)?;
            handle.doc.insert_queued(slot, &row)?;
            handle.publish_queue();
            self.save_snapshot(&handle);
            row.id
        };
        self.deliver_wake(&handle, &row_id).await
    }

    pub fn set_thread_watches_for(
        &self,
        watcher: &str,
        add: &[String],
        remove: &[String],
    ) -> Result<Vec<String>, EngineError> {
        let watches = self
            .thread_watches()
            .ok_or_else(|| EngineError::Other("thread watches not wired".into()))?;
        let chats = self.workspace_or_fail()?.read_chats()?;
        let device = chats
            .iter()
            .find(|chat| chat.id == watcher)
            .map(|chat| chat.device_id.clone())
            .ok_or_else(|| EngineError::Other(format!("no such chat: {watcher}")))?;
        let live = |id: &str| {
            chats
                .iter()
                .any(|chat| chat.id == id && !chat.archived && chat.device_id == device)
        };
        if let Some(id) = add.iter().find(|id| *id == watcher || !live(id)) {
            return Err(EngineError::Other(format!(
                "cannot watch {id}: it must be another live thread on this device"
            )));
        }
        watches.change(watcher, add, remove, live)
    }

    /// A Stop paused the owner's queue for the person's own rows; a wake from
    /// a turn that was not stopped still reaches an idle owner.
    async fn wake_paused_owner(
        &self,
        handle: &Arc<ChatDocHandle>,
        row_id: &str,
    ) -> Result<(), EngineError> {
        let Some(sessions) = self.sessions() else {
            return Ok(());
        };
        let _drain = handle.drain_lock.lock().await;
        if sessions.turn_in_flight(&handle.chat_id) {
            return Ok(());
        }
        let Some(row) = handle.doc.take_queued(row_id)? else {
            return Ok(());
        };
        handle.publish_queue();
        if let Err(err) = self
            .dispatch_queued(handle, &row, QueueSend::NextTurn)
            .await
        {
            handle.doc.insert_queued(0, &row)?;
            handle.publish_queue();
            return Err(err);
        }
        Ok(())
    }

    pub(super) fn spawn_delegation_watcher(
        &self,
        mut statuses: watch::Receiver<Vec<zeron_proto::Session>>,
    ) {
        let host = self.clone();
        self.spawn_worker(async move {
            while statuses.changed().await.is_ok() {
                host.settle_delegations(None).await;
            }
        });
    }

    /// Runs after `recover_stale`, so children it is reviving are already marked.
    pub fn settle_delegations_after_boot(&self) {
        let host = self.clone();
        let booted_at = Utc::now();
        self.spawn_worker(async move {
            host.settle_delegations(Some(booted_at)).await;
        });
    }

    /// Children older than `booted_at` lost their run with the previous engine.
    pub(super) async fn settle_delegations(&self, booted_at: Option<chrono::DateTime<Utc>>) {
        let _settling = self.inner.delegation_lock.lock().await;
        let (Some(workspace), Some(sessions)) = (self.workspace(), self.sessions()) else {
            return;
        };
        for _ in 0..4 {
            let Ok(chats) = workspace.read_chats() else {
                return;
            };
            let mut finished = Vec::new();
            for chat in &chats {
                let Some(delegation) = chat.delegation.as_ref().filter(|d| d.is_running()) else {
                    continue;
                };
                if chat.device_id != self.inner.config.device_id || sessions.is_reviving(&chat.id) {
                    continue;
                }
                let session = sessions.session_status(&chat.id);
                let after_restart = booted_at.is_some_and(|at| delegation.started_at < at);
                let task_queued = self.task_queued(&chat.id, &delegation.anchor);
                let handle = self.open(&chat.id).ok();
                if task_queued
                    && !delegation.cancel_requested
                    && let Some(handle) = &handle
                {
                    if after_restart {
                        handle.queue_paused.store(false, Ordering::Release);
                    }
                    self.drain_queue(handle).await;
                    continue;
                }
                if !task_queued
                    && session.as_ref().is_some_and(|s| {
                        matches!(
                            s.status,
                            SessionStatus::Working | SessionStatus::AwaitingInput
                        )
                    })
                {
                    continue;
                }
                let entries = handle
                    .and_then(|handle| handle.doc.read_entries().ok())
                    .unwrap_or_default();
                let (text, error) = result_parts(&entries, &delegation.anchor);
                if let Some(status) = child_outcome(
                    delegation,
                    session.as_ref(),
                    task_queued,
                    children_pending(&chats, &chat.id),
                    reply_status(&entries, &delegation.anchor),
                    after_restart,
                ) {
                    finished.push((chat.clone(), status, result_text(status, text, error)));
                }
            }
            if finished.is_empty() {
                break;
            }
            for (child, status, result) in finished {
                self.finish_child(&sessions, &child, status, result).await;
            }
        }
        if booted_at.is_some() {
            self.reoffer_lost_wakes().await;
        }
    }

    async fn finish_child(
        &self,
        sessions: &SessionsEngine,
        child: &Chat,
        status: DelegationStatus,
        result: String,
    ) {
        let Some(workspace) = self.workspace() else {
            return;
        };
        let Some(owner_id) = child.delegated_by().map(str::to_owned) else {
            return;
        };
        let owner = workspace.chat(&owner_id).ok().flatten();
        let owner_turn = sessions
            .session_status(&owner_id)
            .filter(|_| sessions.turn_in_flight(&owner_id))
            .and_then(|session| session.started_at);
        let mut offer = false;
        let updated = workspace.update_chat_delegation(&child.id, |d| {
            if !d.is_running() {
                return false;
            }
            d.status = status;
            d.summary = Some(clip(&result, SUMMARY_CHARS));
            d.completed_at = Some(Utc::now());
            if !d.delivery.is_some_and(DelegationDelivery::is_final) {
                if owner.as_ref().is_none_or(|owner| {
                    owner.archived
                        || owner
                            .delegation
                            .as_ref()
                            .is_some_and(|o| o.cancel_requested)
                }) || d.wake == DelegationWake::Never
                {
                    d.delivery = Some(DelegationDelivery::Disposed);
                } else if d.wake == DelegationWake::SettledOnly
                    && owner_turn.is_some()
                    && owner_turn == d.owner_turn_started_at
                {
                    d.delivery = None;
                } else {
                    d.delivery = Some(DelegationDelivery::Claimed);
                    offer = true;
                }
            }
            true
        });
        if let Err(err) = updated {
            tracing::warn!(task = %child.id, error = %err, "finishing a delegated task failed");
            return;
        }
        if offer && let Err(err) = self.offer_wake(&owner_id, &child.id).await {
            tracing::warn!(task = %child.id, error = %err, "waking the owner failed");
        }
    }

    async fn reoffer_lost_wakes(&self) {
        let Some(workspace) = self.workspace() else {
            return;
        };
        let Ok(chats) = workspace.read_chats() else {
            return;
        };
        for chat in &chats {
            let Some(d) = chat.delegation.as_ref() else {
                continue;
            };
            if d.is_running()
                || d.delivery != Some(DelegationDelivery::Claimed)
                || !self.is_host(&d.delegated_by_chat_id)
            {
                continue;
            }
            let queued = self
                .open(&d.delegated_by_chat_id)
                .ok()
                .and_then(|handle| handle.doc.read_queue().ok())
                .is_some_and(|queue| {
                    queue
                        .iter()
                        .any(|row| row.delegated_tasks.iter().any(|id| id == &chat.id))
                });
            if !queued && let Err(err) = self.offer_wake(&d.delegated_by_chat_id, &chat.id).await {
                tracing::warn!(task = %chat.id, error = %err, "re-offering a wake failed");
            }
        }
    }
}

/// A grandchild whose wake has not reached the child yet still holds it open.
fn children_pending(chats: &[Chat], chat_id: &str) -> bool {
    chats.iter().any(|chat| {
        owned_by(chat, chat_id)
            .is_some_and(|d| d.is_running() || d.delivery == Some(DelegationDelivery::Claimed))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn delegation() -> Delegation {
        Delegation {
            delegated_by_chat_id: "owner".into(),
            depth: 1,
            anchor: "task".into(),
            client_request_id: None,
            owner_turn_started_at: None,
            wake: DelegationWake::Always,
            status: DelegationStatus::Running,
            cancel_requested: false,
            delivery: None,
            summary: None,
            started_at: Utc::now(),
            completed_at: None,
        }
    }

    fn session(status: SessionStatus) -> Session {
        Session {
            last_completed_turn: None,
            chat_id: "child".into(),
            device_id: "dev".into(),
            status,
            held_by_subagents: false,
            compacting: false,
            background_tasks: Vec::new(),
            started_at: None,
            updated_at: Utc::now(),
            running_subagents: 0,
        }
    }

    const DONE: Option<MessageStatus> = Some(MessageStatus::Complete);

    #[test]
    fn a_child_finishes_only_when_its_turn_and_its_children_are_done() {
        let d = delegation();
        let idle = session(SessionStatus::Idle);
        let working = session(SessionStatus::Working);
        assert_eq!(
            child_outcome(&d, Some(&working), false, false, DONE, false),
            None
        );
        assert_eq!(
            child_outcome(&d, Some(&idle), true, false, None, false),
            None
        );
        assert_eq!(
            child_outcome(&d, Some(&idle), false, true, DONE, false),
            None
        );
        assert_eq!(child_outcome(&d, None, false, false, DONE, false), None);
        assert_eq!(
            child_outcome(&d, Some(&idle), false, false, None, false),
            None,
            "Idle before the first turn starts is not an ending"
        );
        assert_eq!(
            child_outcome(&d, Some(&idle), false, false, DONE, false),
            Some(DelegationStatus::Completed)
        );
        assert_eq!(
            child_outcome(
                &d,
                Some(&idle),
                false,
                false,
                Some(MessageStatus::Aborted),
                false
            ),
            Some(DelegationStatus::Interrupted)
        );
        assert_eq!(
            child_outcome(
                &d,
                Some(&session(SessionStatus::Errored)),
                false,
                false,
                None,
                false
            ),
            Some(DelegationStatus::Failed)
        );
    }

    #[test]
    fn a_restart_settles_a_child_whose_run_died() {
        let d = delegation();
        assert_eq!(
            child_outcome(&d, None, false, false, DONE, true),
            Some(DelegationStatus::Completed)
        );
        assert_eq!(
            child_outcome(&d, None, false, false, Some(MessageStatus::Streaming), true),
            Some(DelegationStatus::Interrupted)
        );
    }

    #[test]
    fn a_cancelled_child_settles_without_waiting_for_its_children() {
        let mut d = delegation();
        d.cancel_requested = true;
        let idle = session(SessionStatus::Idle);
        assert_eq!(
            child_outcome(&d, Some(&idle), false, true, DONE, false),
            Some(DelegationStatus::Cancelled)
        );
        assert_eq!(
            child_outcome(
                &d,
                Some(&session(SessionStatus::Working)),
                false,
                true,
                None,
                false
            ),
            None
        );
    }

    #[test]
    fn a_cancelled_child_reads_cancelled_even_before_it_started() {
        let mut d = delegation();
        d.cancel_requested = true;
        assert_eq!(
            child_outcome(&d, None, true, false, None, false),
            Some(DelegationStatus::Cancelled)
        );
        assert_eq!(
            child_outcome(
                &d,
                Some(&session(SessionStatus::Idle)),
                false,
                false,
                None,
                false
            ),
            Some(DelegationStatus::Cancelled)
        );
    }

    #[test]
    fn only_an_answer_after_the_latest_user_entry_ends_the_turn() {
        let entry = |id: &str, role, status| SessionMessageEntry {
            id: id.into(),
            role,
            parts: Vec::new(),
            created_at: 0,
            device_id: "dev".into(),
            status,
            continuation_of: None,
            from_chat_id: None,
            fork_import: false,
            answers_request_id: None,
            duration_ms: None,
        };
        let mut entries = vec![
            entry("task", MessageRole::User, None),
            entry("a1", MessageRole::Assistant, Some(MessageStatus::Complete)),
        ];
        assert_eq!(reply_status(&entries, "task"), DONE);
        entries.push(entry("wake", MessageRole::User, None));
        assert_eq!(reply_status(&entries, "task"), None);
        assert_eq!(reply_status(&entries, "missing"), None);
    }

    #[test]
    fn the_wake_names_every_task_it_reports() {
        assert_eq!(
            wake_text(&["a".into()]),
            "Delegated task a reached a terminal state. Use task_status with taskId a to read the result."
        );
        assert_eq!(
            wake_text(&["a".into(), "b".into()]),
            "Delegated tasks a, b reached terminal states. Use task_status with each taskId to read the results."
        );
    }
}
