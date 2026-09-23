use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, Weak};

use async_trait::async_trait;
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;
use zeron_proto::computer::{
    CancelComputerActionParams, ComputerActionParams, ComputerCatalog, GetComputerCatalogParams,
    McpRequestId, McpToolResult, list_computer_permissions,
};

use super::backend::ComputerError;
use super::manager::ComputerManager;
use super::runtime::ComputerRuntime;
use super::setup_signal::ComputerSetupSignal;
use super::tools::results::{mcp_json_error, tool_result_payload};
use super::tools::{ComputerToolContext, ComputerToolSeams, ToolArgs};
use super::visible_use::{
    COMPUTER_FOREGROUND_NOT_AUTHORIZED, ComputerForegroundAuthorization, ComputerForegroundContext,
    computer_foreground_authorization_for_messages,
};
use crate::sessions::{ComputerRun, SessionsEngine};

#[cfg(test)]
mod tests;

pub const COMPUTER_CALLER_REFUSED: &str =
    "Computer access refused: this call does not come from the chat's live run.";
const SETUP_PROMPTS_MAX: usize = 256;

type CallKey = (String, McpRequestId);

struct InFlight {
    turn_id: Option<String>,
    token: CancellationToken,
}

#[derive(Default)]
pub struct ComputerCalls {
    in_flight: Mutex<HashMap<CallKey, InFlight>>,
    setup_prompts: Mutex<VecDeque<String>>,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

impl ComputerCalls {
    fn register(&self, key: CallKey, turn_id: Option<String>, token: CancellationToken) -> bool {
        let mut in_flight = lock(&self.in_flight);
        if in_flight.contains_key(&key) {
            return false;
        }
        in_flight.insert(key, InFlight { turn_id, token });
        true
    }

    fn finish(&self, key: &CallKey) {
        lock(&self.in_flight).remove(key);
    }

    pub fn cancel(&self, chat_id: &str, request_id: &McpRequestId) -> bool {
        let removed = lock(&self.in_flight).remove(&(chat_id.to_owned(), request_id.clone()));
        removed.map(|call| call.token.cancel()).is_some()
    }

    pub fn cancel_chat(&self, chat_id: &str, turn_id: Option<&str>) -> usize {
        let mut in_flight = lock(&self.in_flight);
        let keys: Vec<CallKey> = in_flight
            .iter()
            .filter(|((chat, _), call)| {
                chat == chat_id && turn_id.is_none_or(|turn| call.turn_id.as_deref() == Some(turn))
            })
            .map(|(key, _)| key.clone())
            .collect();
        for key in &keys {
            if let Some(call) = in_flight.remove(key) {
                call.token.cancel();
            }
        }
        keys.len()
    }

    pub fn in_flight(&self) -> usize {
        lock(&self.in_flight).len()
    }

    fn first_setup_prompt(&self, key: String) -> bool {
        let mut prompts = lock(&self.setup_prompts);
        if prompts.contains(&key) {
            return false;
        }
        while prompts.len() >= SETUP_PROMPTS_MAX {
            prompts.pop_front();
        }
        prompts.push_back(key);
        true
    }
}

struct RunToolSeams {
    sessions: SessionsEngine,
    calls: Arc<ComputerCalls>,
    manager: Weak<ComputerManager>,
    chat_id: String,
    turn_id: Option<String>,
}

#[async_trait]
impl ComputerToolSeams for RunToolSeams {
    fn assert_turn_active(&self) -> Result<(), ComputerError> {
        match self
            .sessions
            .computer_caller(&self.chat_id, self.turn_id.as_deref())
        {
            Ok(Some(_)) => Ok(()),
            Ok(None) => Err(ComputerError::Other(
                "This chat was archived or removed; no input was sent.".into(),
            )),
            Err(message) => Err(ComputerError::Other(message)),
        }
    }

    async fn foreground_authorization(&self) -> ComputerForegroundAuthorization {
        let Some(manager) = self.manager.upgrade() else {
            return COMPUTER_FOREGROUND_NOT_AUTHORIZED;
        };
        let Ok(entries) = self.sessions.computer_visible_use_entries(&self.chat_id) else {
            return COMPUTER_FOREGROUND_NOT_AUTHORIZED;
        };
        let known = manager.observed_app_names();
        computer_foreground_authorization_for_messages(
            &entries,
            ComputerForegroundContext {
                known_app_names: &known,
            },
        )
    }

    async fn setup_required(&self, _tool: &str, signal: &ComputerSetupSignal) {
        let mut missing: Vec<String> = signal
            .missing
            .iter()
            .map(|permission| format!("{permission:?}"))
            .collect();
        missing.sort();
        let key = format!(
            "{}:{}:{}",
            self.chat_id,
            self.turn_id.as_deref().unwrap_or("no-turn"),
            missing.join(",")
        );
        if !self.calls.first_setup_prompt(key) {
            return;
        }
        let hint = if signal.missing.is_empty() {
            "Open Zeron Settings › Computer to finish setup.".to_owned()
        } else {
            format!(
                "Allow {} for Zeron in System Settings, then continue.",
                list_computer_permissions(&signal.missing)
            )
        };
        self.sessions
            .write_computer_note(&self.chat_id, "Computer control needs setup", Some(&hint));
    }
}

fn live_run(
    sessions: &SessionsEngine,
    chat_id: &str,
    run_secret: &str,
) -> Result<ComputerRun, String> {
    sessions
        .computer_run(chat_id, run_secret)
        .ok_or_else(|| COMPUTER_CALLER_REFUSED.to_owned())
}

pub fn computer_catalog(
    runtime: &ComputerRuntime,
    sessions: &SessionsEngine,
    params: &GetComputerCatalogParams,
) -> Result<ComputerCatalog, String> {
    let run = live_run(sessions, &params.chat_id, &params.run_secret)?;
    let tools = match runtime.service() {
        Some(service) if service.supported && run.run_computer_control => service.tools.catalog(),
        _ => Vec::new(),
    };
    Ok(ComputerCatalog { tools })
}

pub fn cancel_computer_action(
    runtime: &ComputerRuntime,
    sessions: &SessionsEngine,
    params: &CancelComputerActionParams,
) -> Result<bool, String> {
    live_run(sessions, &params.chat_id, &params.run_secret)?;
    Ok(runtime.calls().cancel(&params.chat_id, &params.request_id))
}

fn denied(code: &str, message: &str) -> McpToolResult {
    mcp_json_error(&json!({ "error": { "code": code, "message": message } }))
}

fn finished(mut result: McpToolResult) -> McpToolResult {
    if !result.has_image()
        && let Some(payload) = tool_result_payload(&result)
    {
        result.structured_content = Some(Value::Object(payload));
    }
    result
}

struct CallerGone {
    armed: bool,
    dispatched: Arc<AtomicBool>,
    manager: Weak<ComputerManager>,
    chat_id: String,
    turn_id: Option<String>,
}

impl Drop for CallerGone {
    fn drop(&mut self) {
        if !self.armed || !self.dispatched.load(Ordering::SeqCst) {
            return;
        }
        let Some(manager) = self.manager.upgrade() else {
            return;
        };
        let (chat_id, turn_id) = (self.chat_id.clone(), self.turn_id.clone());
        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            handle.spawn(async move {
                if let Err(error) = manager
                    .interrupt_thread_input(&chat_id, turn_id.as_deref())
                    .await
                {
                    tracing::warn!(chat = %chat_id, %error, "computer input interrupt failed");
                }
            });
        }
    }
}

pub async fn computer_action(
    runtime: &Arc<ComputerRuntime>,
    sessions: &SessionsEngine,
    params: ComputerActionParams,
) -> Result<McpToolResult, String> {
    let run = live_run(sessions, &params.chat_id, &params.run_secret)?;
    let args: ToolArgs = match params.arguments {
        Value::Null => ToolArgs::new(),
        Value::Object(map) => map,
        _ => return Err("Computer tool arguments must be an object.".into()),
    };
    let Some(service) = runtime.service().filter(|service| service.supported) else {
        let message = runtime.service().map_or_else(
            || "Computer use is still starting on this Mac.".to_owned(),
            |service| unavailable_message(&service.availability),
        );
        return Ok(denied("computer_unavailable", &message));
    };
    let tools = service.tools.clone();
    if tools.entry(&params.tool).is_none() {
        return Err(format!("Unknown computer tool \"{}\".", params.tool));
    }
    if !run.turn_computer_control {
        return Ok(denied(
            "capability_denied",
            "Computer control is off for this turn; no input was sent.",
        ));
    }
    let key: CallKey = (params.chat_id.clone(), params.request_id.clone());
    let token = CancellationToken::new();
    let calls = runtime.calls().clone();
    if !calls.register(key.clone(), run.turn_id.clone(), token.clone()) {
        return Err("This request id is already in flight.".into());
    }
    let dispatched = Arc::new(AtomicBool::new(false));
    let context = ComputerToolContext {
        chat_id: params.chat_id.clone(),
        chat_label: sessions.computer_chat_label(&params.chat_id),
        turn_id: run.turn_id.clone(),
        session_key: run.run_id.clone(),
        request_id: serde_json::to_value(&params.request_id).unwrap_or(Value::Null),
        signal: token,
        seams: Arc::new(RunToolSeams {
            sessions: sessions.clone(),
            calls: calls.clone(),
            manager: Arc::downgrade(&service.manager),
            chat_id: params.chat_id.clone(),
            turn_id: run.turn_id.clone(),
        }),
        mutation_dispatched: dispatched.clone(),
    };
    let mut guard = CallerGone {
        armed: true,
        dispatched,
        manager: Arc::downgrade(&service.manager),
        chat_id: params.chat_id.clone(),
        turn_id: run.turn_id.clone(),
    };
    let (reply, answer) = tokio::sync::oneshot::channel();
    let tool = params.tool;
    tokio::spawn(async move {
        let result = tools.call(&tool, args, &context).await;
        calls.finish(&key);
        let _ = reply.send(result);
    });
    let result = answer.await;
    guard.armed = false;
    result
        .map(finished)
        .map_err(|_| "The computer action ended without a result.".to_owned())
}

fn unavailable_message(availability: &zeron_proto::computer::ComputerAvailability) -> String {
    use zeron_proto::computer::ComputerAvailability as Availability;
    match availability {
        Availability::BackendUnavailable { message } => message.clone(),
        Availability::PermissionRequired { message, .. } => message.clone(),
        Availability::UnsupportedPlatform { platform } => {
            format!("Computer use is not supported on {platform}.")
        }
        _ => "Computer use is unavailable on this Mac.".to_owned(),
    }
}
