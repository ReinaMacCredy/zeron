use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, Weak};
use std::time::Duration;

use async_trait::async_trait;
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;
use zeron_proto::computer::{
    CancelComputerActionParams, ComputerActionParams, ComputerCatalog, ComputerConsentApp,
    ComputerPendingKind, ComputerPendingRequest, GetComputerCatalogParams, McpRequestId,
    McpToolResult, list_computer_permissions,
};

use super::approval_gate::COMPUTER_APPROVAL_TIMEOUT;
use super::backend::{ComputerError, js_slice};
use super::engine_seams::{
    COMPUTER_FOREGROUND_QUESTION_ID, foreground_allowed, foreground_question,
};
use super::manager::{COMPUTER_USER_REVOKED, ComputerConsentTarget, ComputerManager};
use super::runtime::ComputerRuntime;
use super::setup_signal::ComputerSetupSignal;
use super::space_designation::computer_space_designation_for_messages;
use super::tools::results::{mcp_json_error, tool_result_payload};
use super::tools::{ComputerToolContext, ComputerToolSeams, ToolArgs};
use super::visible_use::{
    COMPUTER_FOREGROUND_NOT_AUTHORIZED, COMPUTER_FOREGROUND_NOT_REQUESTED_CODE,
    ComputerForegroundAuthorization, ComputerForegroundContext,
    computer_foreground_authorization_for_messages,
};
use crate::sessions::{ComputerRun, ComputerStoppedApp, SessionsEngine};

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
    foreground_asks: Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
    foreground_wait: Mutex<Option<Duration>>,
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
        drop(in_flight);
        let mut asks = lock(&self.foreground_asks);
        if asks
            .get(chat_id)
            .is_some_and(|ask| Arc::strong_count(ask) == 1)
        {
            asks.remove(chat_id);
        }
        keys.len()
    }

    pub fn forget_chat(&self, chat_id: &str) {
        lock(&self.foreground_asks).remove(chat_id);
        #[cfg(target_os = "macos")]
        super::js_tool::forget(chat_id);
    }

    fn foreground_wait(&self) -> Duration {
        lock(&self.foreground_wait).unwrap_or(COMPUTER_APPROVAL_TIMEOUT)
    }

    #[cfg(test)]
    fn set_foreground_wait(&self, wait: Duration) {
        *lock(&self.foreground_wait) = Some(wait);
    }

    pub fn in_flight(&self) -> usize {
        lock(&self.in_flight).len()
    }

    fn foreground_ask(&self, chat_id: &str) -> Arc<tokio::sync::Mutex<()>> {
        lock(&self.foreground_asks)
            .entry(chat_id.to_owned())
            .or_default()
            .clone()
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
    call_grant: Arc<AtomicBool>,
}

impl RunToolSeams {
    async fn granted(&self, manager: &ComputerManager, target: Option<&ToolArgs>) -> bool {
        let apps = self
            .sessions
            .computer_turn_foreground(&self.chat_id, self.turn_id.as_deref())
            .apps;
        let call_grant = self.call_grant.load(Ordering::SeqCst);
        if apps.is_empty() && !call_grant {
            return false;
        }
        let steps = target
            .and_then(|args| args.get("steps"))
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_object);
        let targets: Vec<ComputerConsentTarget> = target
            .into_iter()
            .chain(steps)
            .map(consent_target)
            .filter(|target| *target != ComputerConsentTarget::Unscoped)
            .collect();
        if targets.is_empty() {
            return call_grant;
        }
        for target in &targets {
            let covered = match manager.foreground_app(target).await.1 {
                Some(key) => apps.contains(&key),
                None => call_grant,
            };
            if !covered {
                return false;
            }
        }
        true
    }
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

    async fn foreground_authorization(
        &self,
        target: Option<&ToolArgs>,
    ) -> ComputerForegroundAuthorization {
        const GRANTED: ComputerForegroundAuthorization = ComputerForegroundAuthorization {
            user_requested_visible_use: true,
        };
        let Some(manager) = self.manager.upgrade() else {
            return COMPUTER_FOREGROUND_NOT_AUTHORIZED;
        };
        if self.granted(&manager, target).await {
            return GRANTED;
        }
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

    async fn space_designation(&self) -> Vec<u64> {
        self.sessions
            .computer_visible_use_entries(&self.chat_id)
            .map(|entries| computer_space_designation_for_messages(&entries))
            .unwrap_or_default()
    }

    async fn workspace_root(&self) -> Option<PathBuf> {
        self.sessions
            .last_request(&self.chat_id)
            .map(|request| PathBuf::from(request.cwd))
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
        self.sessions.write_computer_note(
            &self.chat_id,
            "Computer control needs setup",
            Some(&hint),
        );
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
        Some(service) if service.supported && run.run_computer_control => {
            advertised_catalog(&service.tools)
        }
        _ => Vec::new(),
    };
    Ok(ComputerCatalog { tools })
}

#[cfg(target_os = "macos")]
fn advertised_catalog(
    _tools: &super::tools::ComputerTools,
) -> Vec<zeron_proto::computer::McpToolDefinition> {
    vec![super::js_tool::definition()]
}

#[cfg(not(target_os = "macos"))]
fn advertised_catalog(
    tools: &super::tools::ComputerTools,
) -> Vec<zeron_proto::computer::McpToolDefinition> {
    tools.catalog()
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
    #[cfg(target_os = "macos")]
    if params.tool == super::js_tool::COMPUTER_JS_TOOL {
        if !run.turn_computer_control {
            return Ok(denied(
                "capability_denied",
                "Computer control is off for this turn; no input was sent.",
            ));
        }
        let target = (params.chat_id, params.run_secret, params.request_id);
        return Ok(computer_js(runtime, sessions, target, args, run.turn_id.clone()).await);
    }
    if tools.entry(&params.tool).is_none() {
        return Err(format!("Unknown computer tool \"{}\".", params.tool));
    }
    if !run.turn_computer_control {
        return Ok(denied(
            "capability_denied",
            "Computer control is off for this turn; no input was sent.",
        ));
    }
    if run.turn_desktop_revoked {
        return Ok(denied("user_revoked", COMPUTER_USER_REVOKED));
    }
    if stopped_target(&service.manager, &run.turn_stopped_apps, &args).await {
        return Ok(denied(COMPUTER_APP_STOPPED_CODE, COMPUTER_APP_STOPPED));
    }
    let key: CallKey = (params.chat_id.clone(), params.request_id.clone());
    let token = CancellationToken::new();
    let calls = runtime.calls().clone();
    if !calls.register(key.clone(), run.turn_id.clone(), token.clone()) {
        return Err("This request id is already in flight.".into());
    }
    let dispatched = Arc::new(AtomicBool::new(false));
    let call_grant = Arc::new(AtomicBool::new(false));
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
            call_grant: call_grant.clone(),
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
    let (sessions, manager, run_secret) =
        (sessions.clone(), service.manager.clone(), params.run_secret);
    tokio::spawn(async move {
        let mut result = tools.call(&tool, args.clone(), &context).await;
        if let Some(target) = foreground_refusal(&tool, &args, &result) {
            let (outcome, app) =
                ask_foreground(&sessions, &manager, &calls, &context, &target, &run_secret).await;
            let revoked = sessions
                .computer_run(&context.chat_id, &run_secret)
                .is_some_and(|run| run.turn_desktop_revoked);
            let app = app.as_ref();
            result = match outcome {
                _ if revoked => denied("user_revoked", COMPUTER_USER_REVOKED),
                ForegroundOutcome::Granted => {
                    call_grant.store(true, Ordering::SeqCst);
                    tools.call(&tool, args, &context).await
                }
                ForegroundOutcome::Declined => denied(
                    COMPUTER_FOREGROUND_DECLINED_CODE,
                    &foreground_declined_message(app),
                ),
                ForegroundOutcome::Expired => denied(
                    COMPUTER_FOREGROUND_UNANSWERED_CODE,
                    &foreground_no_retry_message(
                        "The user did not answer whether {app} may come to the front",
                        app,
                    ),
                ),
                ForegroundOutcome::Unavailable => denied(
                    COMPUTER_FOREGROUND_UNAVAILABLE_CODE,
                    &foreground_no_retry_message(
                        "Zeron could not ask the user to bring {app} to the front",
                        app,
                    ),
                ),
                ForegroundOutcome::Unanswered => result,
            };
        }
        calls.finish(&key);
        let _ = reply.send(result);
    });
    let result = answer.await;
    guard.armed = false;
    result
        .map(finished)
        .map_err(|_| "The computer action ended without a result.".to_owned())
}

pub const COMPUTER_APP_STOPPED_CODE: &str = "user_stopped_app";
pub const COMPUTER_APP_STOPPED: &str = "This application session has been explicitly stopped by the user for this turn. Stop your work and send a final message noting they stopped the session and you're ready to continue if they want you to. Computer use can be used again in the next assistant turn.";

async fn stopped_target(
    manager: &ComputerManager,
    stopped: &[ComputerStoppedApp],
    args: &ToolArgs,
) -> bool {
    if stopped.is_empty() {
        return false;
    }
    let steps = args
        .get("steps")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_object);
    for target in std::iter::once(args).chain(steps).map(consent_target) {
        if let Some((key, name)) = manager.app_identity(&target).await
            && stopped
                .iter()
                .any(|app| app.matches(Some(&key), Some(&name)))
        {
            return true;
        }
    }
    false
}

pub const COMPUTER_FOREGROUND_DECLINED_CODE: &str = "foreground_declined";
pub const COMPUTER_FOREGROUND_UNAVAILABLE_CODE: &str = "foreground_unavailable";
pub const COMPUTER_FOREGROUND_UNANSWERED_CODE: &str = "foreground_unanswered";
const FOREGROUND_APP_MAX: usize = 128;

enum ForegroundOutcome {
    Granted,
    Declined,
    Unanswered,
    Expired,
    Unavailable,
}

fn foreground_no_retry_message(lead: &str, app: Option<&ComputerConsentApp>) -> String {
    format!(
        "{}, so nothing was sent. Do not retry this call in the foreground; continue with background actions only, or tell the user what is left.",
        lead.replace("{app}", app.map_or("the app", |app| app.name.as_str()))
    )
}

fn foreground_declined_message(app: Option<&ComputerConsentApp>) -> String {
    format!(
        "The user declined bringing {} to the front for this turn. Continue with background actions only; if the task cannot be finished in the background, tell the user what is left.",
        app.map_or("the app", |app| app.name.as_str())
    )
}

fn raises_foreground(tool_or_step: &str, args: &ToolArgs) -> bool {
    matches!(
        tool_or_step,
        "computer_activate_window" | "computer_invoke_menu" | "activate_window" | "invoke_menu"
    ) || args.get("delivery_mode").and_then(Value::as_str) == Some("foreground")
}

fn consent_target(args: &ToolArgs) -> ComputerConsentTarget {
    let text = |key: &str| {
        args.get(key)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|text| !text.is_empty())
            .map(str::to_owned)
    };
    if let Some(window_id) = text("window_id") {
        return ComputerConsentTarget::Window(window_id);
    }
    if let Some(pid) = args
        .get("pid")
        .and_then(Value::as_u64)
        .and_then(|pid| u32::try_from(pid).ok())
    {
        return ComputerConsentTarget::Pid(pid);
    }
    text("app").map_or(ComputerConsentTarget::Unscoped, ComputerConsentTarget::App)
}

fn foreground_refusal(
    tool: &str,
    args: &ToolArgs,
    result: &McpToolResult,
) -> Option<ComputerConsentTarget> {
    let payload = tool_result_payload(result)?;
    let not_requested = |code: Option<&Value>, effect: Option<&Value>| {
        code.and_then(Value::as_str) == Some(COMPUTER_FOREGROUND_NOT_REQUESTED_CODE)
            && effect.and_then(Value::as_str) == Some("not-dispatched")
    };
    if tool != "computer_run" {
        return (not_requested(payload.get("error"), payload.get("effect"))
            && raises_foreground(tool, args))
        .then(|| consent_target(args));
    }
    let error = payload.get("steps")?.get(0)?.get("error")?;
    let step = args.get("steps")?.get(0)?.as_object()?;
    let step_type = step.get("type").and_then(Value::as_str).unwrap_or_default();
    (payload.get("completed").and_then(Value::as_u64) == Some(0)
        && not_requested(error.get("code"), error.get("effect"))
        && (raises_foreground(step_type, step) || raises_foreground(tool, args)))
    .then(|| consent_target(step))
}

fn foreground_pending(input_id: &str, app: Option<&ComputerConsentApp>) -> ComputerPendingRequest {
    ComputerPendingRequest {
        input_id: input_id.to_owned(),
        question_id: COMPUTER_FOREGROUND_QUESTION_ID.into(),
        kind: ComputerPendingKind::Foreground {
            app: app.map(|app| js_slice(&app.name, FOREGROUND_APP_MAX).to_owned()),
        },
    }
}

async fn ask_foreground(
    sessions: &SessionsEngine,
    manager: &ComputerManager,
    calls: &ComputerCalls,
    context: &ComputerToolContext,
    target: &ComputerConsentTarget,
    run_secret: &str,
) -> (ForegroundOutcome, Option<ComputerConsentApp>) {
    let (chat_id, turn_id) = (context.chat_id.as_str(), context.turn_id.as_deref());
    let ask = calls.foreground_ask(chat_id);
    let _asking = tokio::select! {
        guard = ask.lock() => guard,
        () = context.signal.cancelled() => return (ForegroundOutcome::Unanswered, None),
    };
    let (app, key) = manager.foreground_app(target).await;
    let turn = sessions.computer_turn_foreground(chat_id, turn_id);
    if turn.declined {
        return (ForegroundOutcome::Declined, app);
    }
    if key.as_ref().is_some_and(|key| turn.apps.contains(key)) {
        return (ForegroundOutcome::Granted, app);
    }
    if sessions
        .computer_run(chat_id, run_secret)
        .is_none_or(|run| run.turn_desktop_revoked)
    {
        return (ForegroundOutcome::Unanswered, app);
    }
    let input_id = format!("computer-foreground-{}", uuid::Uuid::new_v4());
    let answers =
        match sessions.raise_input(chat_id, &input_id, vec![foreground_question(app.as_ref())]) {
            Ok(answers) => answers,
            Err(error) => {
                tracing::warn!(chat = %chat_id, %error, "computer foreground question not raised");
                return (ForegroundOutcome::Unavailable, app);
            }
        };
    manager.push_pending_request(
        chat_id,
        turn_id,
        foreground_pending(&input_id, app.as_ref()),
    );
    let answer = tokio::select! {
        answers = answers => answers.ok().as_deref().and_then(foreground_allowed),
        () = tokio::time::sleep(calls.foreground_wait()) => {
            sessions.settle_input(chat_id, &input_id);
            manager.clear_pending_request(chat_id, &input_id);
            return (ForegroundOutcome::Expired, app);
        }
        () = context.signal.cancelled() => {
            sessions.settle_input(chat_id, &input_id);
            None
        }
    };
    manager.clear_pending_request(chat_id, &input_id);
    let Some(granted) = answer else {
        return (ForegroundOutcome::Unanswered, app);
    };
    if !sessions.answer_computer_foreground(chat_id, turn_id, key.as_deref(), granted) {
        tracing::warn!(chat = %chat_id, "computer foreground answer arrived after its turn ended");
    }
    let outcome = if granted {
        ForegroundOutcome::Granted
    } else {
        ForegroundOutcome::Declined
    };
    (outcome, app)
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

#[cfg(target_os = "macos")]
struct EngineJsHost {
    runtime: Arc<ComputerRuntime>,
    sessions: SessionsEngine,
    chat_id: String,
    run_secret: String,
    parent: String,
    next: std::sync::atomic::AtomicU64,
    handle: tokio::runtime::Handle,
}

#[cfg(target_os = "macos")]
impl super::js_tool::JsHost for EngineJsHost {
    fn call(&self, tool: &str, arguments: Value) -> McpToolResult {
        if tool == super::js_tool::COMPUTER_JS_TOOL {
            return McpToolResult::error("The computer tool cannot call itself.");
        }
        if self.sessions.tool_turned_off(tool) {
            return McpToolResult::error(format!(
                "{tool} is turned off in Zeron's Settings > Tools."
            ));
        }
        let n = self.next.fetch_add(1, Ordering::SeqCst);
        let params = ComputerActionParams {
            chat_id: self.chat_id.clone(),
            run_secret: self.run_secret.clone(),
            tool: tool.to_owned(),
            arguments,
            request_id: McpRequestId::String(format!("{}#js{n}", self.parent)),
        };
        let action = computer_action(&self.runtime, &self.sessions, params);
        match self.handle.block_on(action) {
            Ok(result) => result,
            Err(message) => McpToolResult::error(message),
        }
    }
}

#[cfg(target_os = "macos")]
async fn computer_js(
    runtime: &Arc<ComputerRuntime>,
    sessions: &SessionsEngine,
    (chat_id, run_secret, request_id): (String, String, McpRequestId),
    args: ToolArgs,
    turn_id: Option<String>,
) -> McpToolResult {
    let Some(code) = args.get("code").and_then(Value::as_str) else {
        return McpToolResult::error("The computer tool needs a \"code\" string.");
    };
    let timeout = args
        .get("timeout_ms")
        .and_then(Value::as_u64)
        .map_or(super::js_tool::DEFAULT_TIMEOUT, Duration::from_millis);
    let reset = args.get("reset").and_then(Value::as_bool).unwrap_or(false);
    let key: CallKey = (chat_id.clone(), request_id.clone());
    let token = CancellationToken::new();
    let calls = runtime.calls().clone();
    if !calls.register(key.clone(), turn_id, token.clone()) {
        return McpToolResult::error("This request id is already in flight.");
    }
    let cancelled = Arc::new(AtomicBool::new(false));
    let watcher = {
        let (token, cancelled) = (token.clone(), cancelled.clone());
        tokio::spawn(async move {
            token.cancelled().await;
            cancelled.store(true, Ordering::SeqCst);
        })
    };
    let parent = match &request_id {
        McpRequestId::Number(n) => n.to_string(),
        McpRequestId::String(s) => s.clone(),
    };
    let host = Arc::new(EngineJsHost {
        runtime: runtime.clone(),
        sessions: sessions.clone(),
        chat_id: chat_id.clone(),
        run_secret,
        parent,
        next: std::sync::atomic::AtomicU64::new(0),
        handle: tokio::runtime::Handle::current(),
    });
    let outcome = super::js_tool::run(
        &chat_id,
        super::js_tool::JsRequest {
            code: code.to_owned(),
            timeout,
            reset,
            host,
            cancelled,
        },
    )
    .await;
    watcher.abort();
    calls.finish(&key);
    outcome.into_result()
}
