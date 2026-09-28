use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard, PoisonError};

use async_trait::async_trait;
use serde::Serialize;
use serde_json::{Map, Value, json};
use zeron_proto::computer::{
    ComputerAuditEffect, CuaContent, CuaEffect, McpContent, McpToolDefinition, McpToolResult,
};

use super::browser_effect::{computer_browser_effect, computer_browser_field_readback};
use super::consent::{
    COMPUTER_APP_CONSENT_DENIED_CODE, ConsentRefusal, app_consent_refusal_result,
    app_window_required_result,
};
use super::results::{
    AuditOutcome, ToolFailure, computer_audit_error_outcome, cua_action_error_payload,
    mcp_json_error,
};
use super::schemas::{read_only_annotations, write_annotations};
use super::tool_input::{ToolArgs, ToolInputError};
use super::{ComputerToolContext, ComputerToolEntry, ComputerToolFamily, ComputerTools};
use crate::computer::audit_log::{
    ComputerAuditEntry, ComputerAuditTarget, computer_audit_gateway_request_id,
    summarize_computer_audit_args,
};
use crate::computer::backend::{ComputerBrowserCallResult, ComputerError, CuaActionError};
use crate::computer::desktop_operation_queue::OperationSignal;
use crate::computer::manager::ComputerConsentTarget;
use crate::computer::model_desktop_observation::with_model_desktop_observation;
use crate::computer::visible_use::COMPUTER_FOREGROUND_NOT_REQUESTED_CODE;

pub const COMPUTER_BROWSER_TOOL_NAMES: [&str; 10] = [
    "computer_browser_state",
    "computer_browser_prepare",
    "computer_browser_navigate",
    "computer_browser_click",
    "computer_browser_type",
    "computer_browser_dialog",
    "computer_browser_upload",
    "computer_browser_download",
    "computer_browser_pointer",
    "computer_browser_press",
];

pub fn computer_browser_driver_name(name: &str) -> Option<&'static str> {
    Some(match name {
        "computer_browser_state" => "get_browser_state",
        "computer_browser_prepare" => "browser_prepare",
        "computer_browser_navigate" => "browser_navigate",
        "computer_browser_click" => "browser_click",
        "computer_browser_type" => "browser_type",
        "computer_browser_dialog" => "browser_dialog",
        "computer_browser_upload" => "browser_set_input_files",
        "computer_browser_download" => "browser_download",
        "computer_browser_pointer" => "browser_pointer",
        "computer_browser_press" => "browser_type",
        _ => return None,
    })
}

pub fn computer_browser_tool_requires_approval(name: &str, args: &ToolArgs) -> bool {
    if name == "computer_browser_state" {
        return false;
    }
    !(name == "computer_browser_dialog"
        && args.get("action").and_then(Value::as_str) == Some("inspect"))
}

const TARGET_ID_DESCRIPTION: &str = "Opaque browser target id minted by computer_browser_state (bt-…). It names the bound browser — never a tab; each tab carries its own tab_id. Not a desktop window id.";
const TAB_ID_DESCRIPTION: &str = "Opaque tab id from the bind result's tabs[].tab_id (tab-…). May be omitted when the target has exactly one tab, or one active tab — the gateway resolves it. Never pass target_id here.";
const REF_DESCRIPTION: &str = "Page element ref from a computer_browser_state snapshot. Refs die on navigation or a newer snapshot of the same tab.";

const EXISTING_PROFILE_UNAVAILABLE: &str = "This Computer route cannot attach to your existing browser profile. If a separate browser without your cookies satisfies the task, call computer_browser_prepare({allow_launch:true,profile:{mode:\"isolated_new\"}}). Use isolated_named with a task-specific name if the profile must survive browser restarts. Do not substitute it when the task requires your existing profile.";

const PID_ONLY_BIND_REFUSED: &str = "A pid-only bind is for a live driver-owned headless browser. An existing desktop browser needs a native-window bind with computer_browser_state({pid,window_id}) and a verified CDP endpoint. Only a successful bind returns target_id for computer_browser_navigate and other browser actions. If that bind is unavailable, use the exact desktop window through computer_get_state and native Computer actions; do not send pid/window_id to browser actions or silently replace the user's profile.";

const TARGET_REQUIRED: &str = "Browser actions require the target_id returned by a successful computer_browser_state bind; pid and native window_id are not browser target capabilities. Bind once using computer_browser_state, then use its target_id and tab_id. If the bind was refused, these actions cannot control that browser: use the exact desktop window and native Computer actions, or report the limitation. No browser was launched or changed.";

const VISIBLE_BROWSER_NOT_REQUESTED: &str = "The user's task did not ask for a visible browser. Keep windowed false to work in the background, or ask the user to confirm they want to watch.";

const FIELD_READBACK_NOTE: &str = "Field value matched; application effect unverified. Observe once; do not repeat the input automatically.";

const MAX_LISTED_TABS: usize = 10;
const MAX_REMEMBERED_TABS: usize = 100;
const KNOWN_TARGETS_PER_THREAD: usize = 4;
const KNOWN_THREADS_MAX: usize = 32;

#[derive(Debug, Clone, PartialEq, Serialize)]
struct BrowserTab {
    tab_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    active: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    url: Option<String>,
}

struct BrowserRefusal {
    code: &'static str,
    message: String,
    detail: Option<Value>,
}

fn resolvable_tab(tabs: &[BrowserTab]) -> Option<&BrowserTab> {
    if tabs.len() == 1 {
        return tabs.first();
    }
    let mut active = tabs.iter().filter(|tab| tab.active == Some(true));
    match (active.next(), active.next()) {
        (Some(tab), None) => Some(tab),
        _ => None,
    }
}

fn browser_tabs_from(structured: Option<&Map<String, Value>>) -> Vec<BrowserTab> {
    let Some(tabs) = structured
        .and_then(|map| map.get("tabs"))
        .and_then(Value::as_array)
    else {
        return Vec::new();
    };
    tabs.iter()
        .filter_map(|entry| {
            let entry = entry.as_object()?;
            let tab_id = entry.get("tab_id")?.as_str().filter(|id| !id.is_empty())?;
            let text = |key: &str| entry.get(key).and_then(Value::as_str).map(str::to_owned);
            Some(BrowserTab {
                tab_id: tab_id.to_owned(),
                active: entry.get("active").and_then(Value::as_bool),
                title: text("title"),
                url: text("url"),
            })
        })
        .take(MAX_REMEMBERED_TABS)
        .collect()
}

fn describe_tabs(tabs: &[BrowserTab]) -> String {
    let listed: Vec<String> = tabs
        .iter()
        .take(MAX_LISTED_TABS)
        .map(|tab| {
            let active = if tab.active == Some(true) {
                " (active)"
            } else {
                ""
            };
            format!("\"{}\"{active}", tab.tab_id)
        })
        .collect();
    let remaining = tabs.len().saturating_sub(listed.len());
    let more = if remaining > 0 {
        format!(" and {remaining} more")
    } else {
        String::new()
    };
    format!("{}{more}", listed.join(", "))
}

fn with_appended_text(
    mut result: ComputerBrowserCallResult,
    line: &str,
) -> ComputerBrowserCallResult {
    let content = result.content.get_or_insert_with(Vec::new);
    match content
        .iter_mut()
        .find(|part| part.kind == "text" && part.text.is_some())
    {
        Some(part) => {
            let text = part.text.take().unwrap_or_default();
            part.text = Some(format!("{text}\n{line}"));
        }
        None => content.push(text_part(line.to_owned())),
    }
    result
}

fn text_part(text: String) -> CuaContent {
    CuaContent {
        kind: "text".into(),
        text: Some(text),
        data: None,
        mime_type: None,
    }
}

fn refused_with_message(
    mut result: ComputerBrowserCallResult,
    code: &str,
    message: &str,
) -> ComputerBrowserCallResult {
    result.content = Some(vec![text_part(format!("refused ({code}): {message}"))]);
    if let Some(structured) = result.structured_content.as_mut()
        && let Some(Value::Object(refusal)) = structured.get_mut("refusal")
    {
        refusal.insert("message".into(), json!(message));
    }
    result
}

fn browser_refusal_result(refusal: BrowserRefusal) -> McpToolResult {
    let mut body = json!({ "code": refusal.code, "message": refusal.message });
    if let Some(detail) = refusal.detail {
        body["detail"] = detail;
    }
    McpToolResult {
        content: vec![McpContent::Text {
            text: format!("refused ({}): {}", refusal.code, refusal.message),
        }],
        structured_content: Some(json!({ "status": "refused", "refusal": body })),
        is_error: false,
    }
}

pub(super) fn browser_result_to_mcp(result: ComputerBrowserCallResult) -> McpToolResult {
    let mut content = Vec::new();
    for part in result.content.unwrap_or_default() {
        match (part.kind.as_str(), part.text, part.data, part.mime_type) {
            ("text", Some(text), _, _) => content.push(McpContent::Text { text }),
            ("image", _, Some(data), Some(mime_type)) => {
                content.push(McpContent::Image { data, mime_type })
            }
            _ => {}
        }
    }
    let structured = result.structured_content.map(Value::Object);
    if content.is_empty() {
        let fallback = structured
            .clone()
            .unwrap_or_else(|| json!({ "status": "ok" }));
        content.push(McpContent::Text {
            text: fallback.to_string(),
        });
    }
    McpToolResult {
        content,
        structured_content: structured,
        is_error: result.is_error == Some(true),
    }
}

fn augment_browser_result(
    name: &str,
    args: &ToolArgs,
    result: ComputerBrowserCallResult,
) -> ComputerBrowserCallResult {
    let Some(structured) = result.structured_content.clone() else {
        return result;
    };
    if computer_browser_field_readback(name, args, &result) {
        let mut content = vec![text_part(FIELD_READBACK_NOTE.to_owned())];
        content.extend(
            result
                .content
                .iter()
                .flatten()
                .filter(|part| part.kind != "text")
                .cloned(),
        );
        return ComputerBrowserCallResult {
            content: Some(content),
            ..result
        };
    }
    if structured.get("status").and_then(Value::as_str) == Some("refused")
        && let Some(refusal) = structured.get("refusal").and_then(Value::as_object)
    {
        let code = refusal
            .get("code")
            .and_then(Value::as_str)
            .unwrap_or("browser_refused");
        if name == "computer_browser_prepare" && code == "browser_consent_required" {
            return refused_with_message(result, code, EXISTING_PROFILE_UNAVAILABLE);
        }
        if name == "computer_browser_state"
            && code == "browser_wrong_target_refused"
            && args.contains_key("pid")
            && !args.contains_key("window_id")
        {
            return refused_with_message(result, code, PID_ONLY_BIND_REFUSED);
        }
        let tab_id = args.get("tab_id").and_then(Value::as_str);
        let target_id = args.get("target_id").and_then(Value::as_str);
        if code == "browser_tab_not_found"
            && let Some(tab_id) = tab_id
            && (Some(tab_id) == target_id || tab_id.starts_with("bt-"))
        {
            let target = target_id.map_or_else(|| "that call".to_owned(), |id| format!("\"{id}\""));
            let message = format!(
                "tab_id \"{tab_id}\" is a target id, not a tab id, so it names no tab of target {target}. target_id identifies the bound browser; each tab carries its own tab_id (tab-…) in the bind result's tabs list. Pass that tab_id here."
            );
            return refused_with_message(result, code, &message);
        }
        return result;
    }
    let Some(target_id) = structured.get("target_id").and_then(Value::as_str) else {
        let prepared_pid = structured.get("prepared_pid").filter(|pid| pid.is_number());
        return match (name, prepared_pid) {
            ("computer_browser_prepare", Some(pid)) => with_appended_text(
                result,
                &format!(
                    "ids: prepared_pid={pid} (a bind key, not a target or tab id); next: computer_browser_state {{pid: {pid}}} — a driver-owned headless bind takes pid alone (window_id only for a native browser window); the bind result names target_id and each tab's tab_id."
                ),
            ),
            _ => result,
        };
    };
    let tabs = browser_tabs_from(Some(&structured));
    let resolvable = resolvable_tab(&tabs).map(|tab| tab.tab_id.clone());
    let explicit = structured.get("tab_id").and_then(Value::as_str);
    let tab_record = explicit.map(str::to_owned).or_else(|| resolvable.clone());
    let line = match &tab_record {
        Some(tab) => {
            let active = if Some(tab) == resolvable.as_ref() {
                " (active)"
            } else {
                ""
            };
            format!("ids: target_id={target_id}; tab_id={tab}{active}")
        }
        None if !tabs.is_empty() => {
            format!("ids: target_id={target_id}; tabs: {}", describe_tabs(&tabs))
        }
        None => format!("ids: target_id={target_id}"),
    };
    let mut next = structured.clone();
    if explicit.is_none()
        && let Some(tab) = &tab_record
    {
        next.insert("tab_id".into(), json!(tab));
    }
    ComputerBrowserCallResult {
        structured_content: Some(next),
        ..with_appended_text(result, &line)
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

type KnownTargets = Vec<(String, Vec<(String, Vec<BrowserTab>)>)>;

enum TabResolution {
    Unchanged,
    Resolved(ToolArgs),
    Refused(BrowserRefusal),
}

pub struct ComputerBrowserTools {
    entries: Vec<ComputerToolEntry>,
    known_targets: Mutex<KnownTargets>,
    target_pids: Mutex<Vec<(String, String, u32)>>,
}

impl Default for ComputerBrowserTools {
    fn default() -> Self {
        Self {
            entries: browser_entries(),
            known_targets: Mutex::default(),
            target_pids: Mutex::default(),
        }
    }
}

fn is_safe_integer(value: &Value) -> Option<i64> {
    let number = value.as_f64()?;
    (number.fract() == 0.0 && number.abs() <= 9_007_199_254_740_991.0).then_some(number as i64)
}

impl ComputerBrowserTools {
    fn remember_target_tabs(&self, thread_id: &str, structured: Option<&Map<String, Value>>) {
        let Some(target_id) = structured
            .and_then(|map| map.get("target_id"))
            .and_then(Value::as_str)
        else {
            return;
        };
        let tabs = browser_tabs_from(structured);
        if tabs.is_empty() {
            return;
        }
        let mut known = lock(&self.known_targets);
        let mut by_target = match known.iter().position(|(thread, _)| thread == thread_id) {
            Some(index) => known.remove(index).1,
            None => Vec::new(),
        };
        by_target.retain(|(target, _)| target != target_id);
        by_target.push((target_id.to_owned(), tabs));
        while by_target.len() > KNOWN_TARGETS_PER_THREAD {
            by_target.remove(0);
        }
        known.push((thread_id.to_owned(), by_target));
        while known.len() > KNOWN_THREADS_MAX {
            known.remove(0);
        }
    }

    fn remember_target_pid(
        &self,
        thread_id: &str,
        args: &ToolArgs,
        structured: Option<&Map<String, Value>>,
    ) {
        let Some(target_id) = structured
            .and_then(|map| map.get("target_id"))
            .and_then(Value::as_str)
        else {
            return;
        };
        let Some(pid) = structured
            .and_then(|map| map.get("pid"))
            .or_else(|| args.get("pid"))
            .and_then(is_safe_integer)
            .and_then(|pid| u32::try_from(pid).ok())
            .filter(|pid| *pid > 0)
        else {
            return;
        };
        let mut known = lock(&self.target_pids);
        known.retain(|(thread, target, _)| !(thread == thread_id && target == target_id));
        known.push((thread_id.to_owned(), target_id.to_owned(), pid));
        while known.len() > KNOWN_THREADS_MAX * KNOWN_TARGETS_PER_THREAD {
            known.remove(0);
        }
    }

    fn consent_targets(
        &self,
        thread_id: &str,
        args: &ToolArgs,
    ) -> Vec<(Option<usize>, ComputerConsentTarget)> {
        let pid = args
            .get("target_id")
            .and_then(Value::as_str)
            .and_then(|target_id| {
                lock(&self.target_pids)
                    .iter()
                    .find(|(thread, target, _)| thread == thread_id && target == target_id)
                    .map(|(_, _, pid)| *pid)
            })
            .or_else(|| {
                args.get("pid")
                    .and_then(is_safe_integer)
                    .and_then(|pid| u32::try_from(pid).ok())
                    .filter(|pid| *pid > 0)
            });
        pid.map(|pid| (None, ComputerConsentTarget::Pid(pid)))
            .into_iter()
            .collect()
    }

    fn known_tabs(&self, thread_id: &str, target_id: &str) -> Vec<BrowserTab> {
        lock(&self.known_targets)
            .iter()
            .find(|(thread, _)| thread == thread_id)
            .and_then(|(_, targets)| targets.iter().find(|(target, _)| target == target_id))
            .map(|(_, tabs)| tabs.clone())
            .unwrap_or_default()
    }

    fn resolve_omitted_tab_id(&self, thread_id: &str, args: &ToolArgs) -> TabResolution {
        if args
            .get("tab_id")
            .and_then(Value::as_str)
            .is_some_and(|id| !id.is_empty())
        {
            return TabResolution::Unchanged;
        }
        let Some(target_id) = args.get("target_id").and_then(Value::as_str) else {
            return TabResolution::Unchanged;
        };
        if args.contains_key("pid") || args.contains_key("window_id") {
            return TabResolution::Unchanged;
        }
        let tabs = self.known_tabs(thread_id, target_id);
        if tabs.is_empty() {
            return TabResolution::Refused(BrowserRefusal {
                code: "browser_tab_required",
                message: format!(
                    "no tab_id was given and this thread has no bind result for target \"{target_id}\" to resolve one from. Bind the browser first with computer_browser_state (pid alone for a driver-owned headless browser; pid + window_id for a native window); its result lists every tab's tab_id."
                ),
                detail: None,
            });
        }
        if let Some(tab) = resolvable_tab(&tabs) {
            let mut resolved = args.clone();
            resolved.insert("tab_id".into(), json!(tab.tab_id));
            return TabResolution::Resolved(resolved);
        }
        TabResolution::Refused(BrowserRefusal {
            code: "browser_tab_required",
            message: format!(
                "no tab_id was given and target \"{target_id}\" has {} tabs with no single active tab to default to. Pass the tab_id you want: {}.",
                tabs.len(),
                describe_tabs(&tabs)
            ),
            detail: Some(json!({ "tabs": tabs })),
        })
    }

    fn audit(
        &self,
        tools: &ComputerTools,
        name: &str,
        args: &ToolArgs,
        context: &ComputerToolContext,
        outcome: AuditOutcome,
    ) {
        let pid = args
            .get("pid")
            .and_then(is_safe_integer)
            .filter(|pid| *pid > 0 && *pid <= i64::from(u32::MAX))
            .map(|pid| pid as u32);
        let window_id = match args.get("window_id") {
            Some(Value::String(id)) => Some(id.clone()),
            Some(value) => {
                is_safe_integer(value).map(|id| format!("cua:{}:{id}", pid.unwrap_or(0)))
            }
            None => None,
        };
        let target = (pid.is_some() || window_id.is_some()).then_some(ComputerAuditTarget {
            window_id,
            pid,
            app: None,
            bundle_id: None,
        });
        let entry = ComputerAuditEntry {
            thread_id: Some(context.chat_id.clone()),
            turn_id: context.turn_id.clone(),
            gateway_request_id: computer_audit_gateway_request_id(&context.request_id),
            target,
            args: Some(summarize_computer_audit_args(args)),
            code: outcome.code,
            diagnostics: outcome.diagnostics,
            layer: outcome.layer,
            ..ComputerAuditEntry::new(name, outcome.effect)
        };
        tools.manager().record_computer_audit(&entry);
    }

    async fn assert_visible_browser_allowed(
        &self,
        context: &ComputerToolContext,
    ) -> Result<(), ComputerError> {
        if context
            .seams
            .foreground_authorization(None)
            .await
            .user_requested_visible_use
        {
            return Ok(());
        }
        Err(
            CuaActionError::new(VISIBLE_BROWSER_NOT_REQUESTED, CuaEffect::NotDispatched)
                .code(COMPUTER_FOREGROUND_NOT_REQUESTED_CODE)
                .into(),
        )
    }

    async fn run(
        &self,
        tools: &ComputerTools,
        name: &str,
        args: &ToolArgs,
        effective: &mut ToolArgs,
        context: &ComputerToolContext,
    ) -> Result<McpToolResult, ToolFailure> {
        let audited = computer_browser_tool_requires_approval(name, args);
        let refuse = |effective: &ToolArgs, refusal: BrowserRefusal| {
            if audited {
                self.audit(
                    tools,
                    name,
                    effective,
                    context,
                    AuditOutcome::new(ComputerAuditEffect::Refused, Some(refusal.code)),
                );
            }
            browser_refusal_result(refusal)
        };
        if name != "computer_browser_state" && name != "computer_browser_prepare" {
            let has_target = args
                .get("target_id")
                .and_then(Value::as_str)
                .is_some_and(|id| !id.trim().is_empty());
            if !has_target || args.contains_key("pid") || args.contains_key("window_id") {
                return Ok(refuse(
                    effective,
                    BrowserRefusal {
                        code: "browser_target_required",
                        message: TARGET_REQUIRED.to_owned(),
                        detail: None,
                    },
                ));
            }
        }
        match self.resolve_omitted_tab_id(&context.chat_id, args) {
            TabResolution::Refused(refusal) => return Ok(refuse(effective, refusal)),
            TabResolution::Resolved(resolved) => *effective = resolved,
            TabResolution::Unchanged => {}
        }
        if name == "computer_browser_press" {
            effective.insert("mode".into(), json!("keystrokes"));
            effective.insert("text".into(), json!("\n"));
        }
        let visible_launch = name == "computer_browser_prepare"
            && effective.get("windowed") == Some(&Value::Bool(true));
        if visible_launch {
            self.assert_visible_browser_allowed(context).await?;
        }
        if computer_browser_tool_requires_approval(name, effective) {
            let targets = self.consent_targets(&context.chat_id, effective);
            match tools
                .authorize_consent_targets(name, effective, context, targets)
                .await?
            {
                None => {}
                Some(ConsentRefusal::Task) => {
                    self.audit(
                        tools,
                        name,
                        effective,
                        context,
                        AuditOutcome::new(ComputerAuditEffect::Refused, Some("approval_denied")),
                    );
                    return Ok(McpToolResult::error(
                        "Computer browser action was denied or cancelled; no input was sent.",
                    ));
                }
                Some(ConsentRefusal::Unscoped { step }) => {
                    return Ok(app_window_required_result(step));
                }
                Some(ConsentRefusal::App { app, step }) => {
                    self.audit(
                        tools,
                        name,
                        effective,
                        context,
                        AuditOutcome::new(
                            ComputerAuditEffect::Refused,
                            Some(COMPUTER_APP_CONSENT_DENIED_CODE),
                        ),
                    );
                    return Ok(app_consent_refusal_result(&app, step));
                }
            }
        }
        tools.assert_turn_active(context)?;
        let bounded = bound_browser_paths(name, effective.clone(), context).await?;
        let driver = computer_browser_driver_name(name)
            .ok_or_else(|| ToolFailure::message(format!("Unknown computer tool \"{name}\".")))?;
        let signal: OperationSignal = context.signal.clone().into();
        let recheck = visible_launch.then_some(|| self.assert_visible_browser_allowed(context));
        let dispatch = tools.manager().browser_call(
            &context.chat_id,
            context.turn_id.as_deref(),
            driver,
            bounded.clone(),
            Some(signal),
            recheck,
        );
        let result = if name == "computer_browser_state" {
            with_model_desktop_observation(dispatch).await
        } else {
            dispatch.await
        }?;
        self.remember_target_tabs(&context.chat_id, result.structured_content.as_ref());
        self.remember_target_pid(
            &context.chat_id,
            effective,
            result.structured_content.as_ref(),
        );
        if audited {
            self.audit(
                tools,
                name,
                effective,
                context,
                computer_browser_effect(name, &bounded, &result),
            );
        }
        Ok(browser_result_to_mcp(augment_browser_result(
            name, effective, result,
        )))
    }
}

async fn bounded_workspace_path(
    raw: Option<&Value>,
    workspace_root: &Path,
    field: &str,
) -> Result<String, ToolInputError> {
    let Some(raw) = raw
        .and_then(Value::as_str)
        .filter(|raw| !raw.is_empty() && Path::new(raw).is_absolute())
    else {
        return Err(ToolInputError::new(format!(
            "\"{field}\" must be an absolute path."
        )));
    };
    let unresolved =
        || ToolInputError::new(format!("\"{field}\" does not resolve to an existing path."));
    let canonical = tokio::fs::canonicalize(raw)
        .await
        .map_err(|_| unresolved())?;
    if !canonical.starts_with(workspace_root) {
        return Err(ToolInputError::new(format!(
            "\"{field}\" resolves outside the active workspace."
        )));
    }
    canonical
        .into_os_string()
        .into_string()
        .map_err(|_| unresolved())
}

async fn bound_browser_paths(
    name: &str,
    mut args: ToolArgs,
    context: &ComputerToolContext,
) -> Result<ToolArgs, ToolFailure> {
    if name != "computer_browser_upload" && name != "computer_browser_download" {
        return Ok(args);
    }
    let Some(root) = context
        .seams
        .workspace_root()
        .await
        .filter(|root| !root.as_os_str().to_string_lossy().trim().is_empty())
    else {
        return Err(ToolInputError::new(
            "No canonical workspace is available for browser file transfer; the call was refused before it ran.",
        )
        .into());
    };
    let workspace_root: PathBuf = tokio::fs::canonicalize(&root)
        .await
        .map_err(|error| ToolFailure::message(error.to_string()))?;
    if name == "computer_browser_upload" {
        let files = args
            .get("files")
            .and_then(Value::as_array)
            .filter(|files| (1..=32).contains(&files.len()))
            .cloned()
            .ok_or_else(|| {
                ToolInputError::new("\"files\" must be an array of 1-32 absolute paths.")
            })?;
        let mut bounded = Vec::with_capacity(files.len());
        for file in &files {
            bounded.push(json!(
                bounded_workspace_path(Some(file), &workspace_root, "files").await?
            ));
        }
        args.insert("files".into(), Value::Array(bounded));
        return Ok(args);
    }
    let destination = bounded_workspace_path(
        args.get("destination_root"),
        &workspace_root,
        "destination_root",
    )
    .await?;
    args.insert("destination_root".into(), json!(destination));
    Ok(args)
}

fn browser_failure_result(failure: &ToolFailure) -> McpToolResult {
    match failure.computer().and_then(cua_action_error_payload) {
        Some(payload) => mcp_json_error(&payload),
        None => McpToolResult::error(failure.text()),
    }
}

#[async_trait]
impl ComputerToolFamily for ComputerBrowserTools {
    fn entries(&self) -> Vec<ComputerToolEntry> {
        self.entries.clone()
    }

    async fn call(
        &self,
        tools: &ComputerTools,
        name: &str,
        args: ToolArgs,
        context: &ComputerToolContext,
    ) -> McpToolResult {
        let mut effective = args.clone();
        match self.run(tools, name, &args, &mut effective, context).await {
            Ok(result) => result,
            Err(failure) => {
                let revoked = failure
                    .computer()
                    .is_some_and(ComputerError::control_revoked);
                if computer_browser_tool_requires_approval(name, &args) && !revoked {
                    self.audit(
                        tools,
                        name,
                        &effective,
                        context,
                        computer_audit_error_outcome(&failure),
                    );
                }
                browser_failure_result(&failure)
            }
        }
    }
}

fn target_id_property() -> Value {
    json!({ "type": "string", "description": TARGET_ID_DESCRIPTION })
}

fn tab_id_property() -> Value {
    json!({ "type": "string", "description": TAB_ID_DESCRIPTION })
}

fn ref_property() -> Value {
    json!({ "type": "string", "description": REF_DESCRIPTION })
}

fn browser_entry(
    name: &str,
    title: &str,
    description: &str,
    input_schema: Value,
    read_only: bool,
) -> ComputerToolEntry {
    ComputerToolEntry {
        definition: McpToolDefinition {
            name: name.to_owned(),
            description: description.to_owned(),
            input_schema,
            annotations: Some(if read_only {
                read_only_annotations(title)
            } else {
                write_annotations(title)
            }),
        },
        discovery_only: false,
    }
}

fn browser_entries() -> Vec<ComputerToolEntry> {
    vec![
        browser_entry(
            "computer_browser_state",
            "Read browser state",
            "Observe or bind a browser through the desktop driver's CDP route. Two modes: pass pid to bind a prepared browser — window_id only for a native browser window; a driver-owned browser launched by computer_browser_prepare is headless and binds from pid alone with binding_route \"driver_owned_headless\" — or pass the target_id + tab_id it minted to snapshot one tab: a semantic outline, element refs, and optionally a viewport screenshot (include_screenshot). The bind result is explicit: target_id names the bound browser; each entry of tabs carries its own tab_id. tab_id may be omitted when the target has one (or one active) tab — the gateway resolves it; an ambiguous target refuses with its tab listing. Element refs stay valid until that tab navigates or a newer snapshot supersedes them.",
            json!({
                "type": "object",
                "properties": {
                    "pid": {
                        "type": "integer",
                        "description": "Browser process id to bind: the prepare result's prepared_pid for a driver-owned headless browser, or a native browser window's process.",
                    },
                    "window_id": {
                        "type": "integer",
                        "description": "Native window id owned by pid — bind mode for a native browser window. Omit it for a driver-owned headless browser (prepared with an isolated profile); the bind is minted from the driver's own CDP endpoint.",
                    },
                    "target_id": target_id_property(),
                    "tab_id": tab_id_property(),
                    "snapshot_format": {
                        "type": "string",
                        "enum": ["dom_refs_v1", "semantic_v2"],
                        "description": "Snapshot contract version; semantic_v2 is the richer outline.",
                    },
                    "scope_ref": {
                        "type": "string",
                        "description": "Limit the observation to this ref's subtree.",
                    },
                    "query": {
                        "type": "string",
                        "description": "Read-only match over role, accessible name, and visible text.",
                    },
                    "continuation": {
                        "type": "string",
                        "description": "Opaque continuation minted by an earlier semantic_v2 response.",
                    },
                    "include_screenshot": {
                        "type": "boolean",
                        "description": "Capture the tab viewport as PNG through CDP.",
                    },
                },
                "additionalProperties": false,
            }),
            true,
        ),
        browser_entry(
            "computer_browser_prepare",
            "Prepare browser",
            "Prepare driver-owned isolated Chromium (profile.mode \"isolated_new\" or \"isolated_named\", allow_launch:true), headless by default. Or detect an existing endpoint with pid (+ window_id), allow_launch:false and no strategy. Linux control requires the verified driver and packaged host's confirmed direct-X11 Escape listener; only owned isolated headless targets support mutation. Wayland/XWayland and standalone hosts permit reads/passive prepare only. Linux refuses visible launch and personal-profile control. On macOS, windowed:true needs the user's request to watch; otherwise foreground_not_requested. Prefer \"isolated_named\" to preserve a profile across restarts; \"isolated_new\" starts empty. Use prepared_pid with computer_browser_state. Existing-profile attachment needs a consent grant this embedding cannot host (browser_consent_required).",
            json!({
                "type": "object",
                "properties": {
                    "pid": { "type": "integer", "description": "Browser process id to prepare or detect." },
                    "window_id": { "type": "integer", "description": "Native window id owned by pid." },
                    "allow_launch": {
                        "type": "boolean",
                        "description": "Permit launching a separate driver-owned isolated Chromium.",
                    },
                    "windowed": {
                        "type": "boolean",
                        "description": "Default false: isolated headless. Linux requires a verified driver and confirmed direct-X11 Escape listener, and refuses true. On macOS, true opens a window only when the user asks to watch; full access is insufficient.",
                    },
                    "profile": {
                        "type": "object",
                        "properties": {
                            "mode": {
                                "type": "string",
                                "enum": ["isolated_new", "isolated_named"],
                                "description": "\"isolated_named\" keeps a named profile across browser restarts — recommended for multi-step work such as a cart; \"isolated_new\" starts from an empty profile every launch.",
                            },
                            "name": {
                                "type": "string",
                                "description": "Required for isolated_named; 1-64 path-safe ASCII characters.",
                            },
                        },
                        "required": ["mode"],
                        "additionalProperties": false,
                    },
                },
                "additionalProperties": false,
            }),
            false,
        ),
        browser_entry(
            "computer_browser_navigate",
            "Navigate browser tab",
            "Navigate one tab of a bound browser target to a new URL (http, https, or about only). Pass the bind result's target_id and the tab's own tab_id; tab_id may be omitted when the target has one (or one active) tab. Invalidates every element ref for the tab — take a fresh computer_browser_state snapshot before interacting again.",
            json!({
                "type": "object",
                "properties": {
                    "target_id": target_id_property(),
                    "tab_id": tab_id_property(),
                    "url": { "type": "string", "description": "Destination URL (http:, https:, or about:)." },
                },
                "required": ["target_id", "url"],
                "additionalProperties": false,
            }),
            false,
        ),
        browser_entry(
            "computer_browser_click",
            "Click in browser tab",
            "Click a page element by ref, or viewport coordinates (x/y in CSS px), inside a bound tab. The default \"trusted\" route uses CDP input and works on a background renderer; \"dom_event\" synthesizes a DOM click and proves only dispatch — verify the postcondition with a fresh snapshot.",
            json!({
                "type": "object",
                "properties": {
                    "target_id": target_id_property(),
                    "tab_id": tab_id_property(),
                    "ref": ref_property(),
                    "x": { "type": "number", "description": "Viewport x in CSS px — alternative to ref." },
                    "y": { "type": "number", "description": "Viewport y in CSS px — alternative to ref." },
                    "input_route": { "type": "string", "enum": ["trusted", "dom_event"] },
                },
                "required": ["target_id"],
                "additionalProperties": false,
            }),
            false,
        ),
        browser_entry(
            "computer_browser_type",
            "Type into browser field",
            "Type text into an element by ref inside a bound tab. \"insert_text\" (default) is a bulk insert; \"keystrokes\" sends per-character key events. input_route \"trusted\" (default) uses CDP input; \"dom_event\" is the background-safe synthetic insertion (insert_text mode only) that cannot raise a standalone browser window — dispatch is read back from the element, so verify the page's own postcondition with a fresh snapshot. Set replace true to select the field's whole content first — with empty text this clears it.",
            json!({
                "type": "object",
                "properties": {
                    "target_id": target_id_property(),
                    "tab_id": tab_id_property(),
                    "ref": ref_property(),
                    "text": { "type": "string", "description": "Text to type." },
                    "mode": { "type": "string", "enum": ["insert_text", "keystrokes"] },
                    "input_route": {
                        "type": "string",
                        "enum": ["trusted", "dom_event"],
                        "description": "\"trusted\" (default): CDP Input events. \"dom_event\": synthetic full-background DOM insertion for input, textarea, and contenteditable refs (insert_text only); dispatch is proven by a live element read-back, not by the application accepting the text — verify with a fresh snapshot.",
                    },
                    "replace": {
                        "type": "boolean",
                        "description": "Select existing field content first so text replaces it.",
                    },
                },
                "required": ["target_id", "ref", "text"],
                "additionalProperties": false,
            }),
            false,
        ),
        browser_entry(
            "computer_browser_dialog",
            "Handle browser dialog",
            "Inspect, accept, or dismiss a JavaScript dialog in a bound tab. action \"inspect\" is read-only and returns the current dialog plus an opaque dialog_id. \"accept\"/\"dismiss\" resolve it; prompt_text supplies a prompt's response text. Consequential actions require approval.",
            json!({
                "type": "object",
                "properties": {
                    "target_id": target_id_property(),
                    "tab_id": tab_id_property(),
                    "action": { "type": "string", "enum": ["inspect", "accept", "dismiss"] },
                    "dialog_id": {
                        "type": "string",
                        "description": "Opaque dialog generation returned by action=inspect.",
                    },
                    "prompt_text": {
                        "type": "string",
                        "description": "Response text, valid only when accepting a prompt dialog.",
                    },
                },
                "required": ["target_id", "action"],
                "additionalProperties": false,
            }),
            false,
        ),
        browser_entry(
            "computer_browser_upload",
            "Set file input files",
            "Attach files to a file-upload element (ref) in a bound tab. Every path must be absolute and resolve inside the active workspace — paths are canonicalized before dispatch, and anything resolving outside the workspace is refused before it runs.",
            json!({
                "type": "object",
                "properties": {
                    "target_id": target_id_property(),
                    "tab_id": tab_id_property(),
                    "ref": {
                        "type": "string",
                        "description": format!("Page ref of the file-upload control. {REF_DESCRIPTION}"),
                    },
                    "files": {
                        "type": "array",
                        "minItems": 1,
                        "maxItems": 32,
                        "items": { "type": "string", "description": "Absolute path to one workspace file." },
                    },
                },
                "required": ["target_id", "ref", "files"],
                "additionalProperties": false,
            }),
            false,
        ),
        browser_entry(
            "computer_browser_download",
            "Download via browser",
            "Trigger one download by activating a live ref, saved inside destination_root. The directory must be absolute and resolve inside the active workspace. Requires approval; the result never echoes the source URL, filename, or destination path back.",
            json!({
                "type": "object",
                "properties": {
                    "target_id": target_id_property(),
                    "tab_id": tab_id_property(),
                    "ref": ref_property(),
                    "destination_root": {
                        "type": "string",
                        "description": "Absolute directory inside the workspace to receive the download.",
                    },
                },
                "required": ["target_id", "ref", "destination_root"],
                "additionalProperties": false,
            }),
            false,
        ),
        browser_entry(
            "computer_browser_pointer",
            "Browser pointer action",
            "Hover, right-click, double-click, scroll, or drag inside a bound tab. Point at an element by ref or at viewport coordinates (x/y in CSS px); drags take destination_ref or to_x/to_y, scrolls take delta_x/delta_y. Semantic refs must declare the matching pointer capability. Never activates or raises the tab.",
            json!({
                "type": "object",
                "properties": {
                    "target_id": target_id_property(),
                    "tab_id": tab_id_property(),
                    "action": {
                        "type": "string",
                        "enum": ["hover", "right_click", "double_click", "scroll", "drag"],
                    },
                    "ref": ref_property(),
                    "x": { "type": "number", "description": "Origin viewport x in CSS px." },
                    "y": { "type": "number", "description": "Origin viewport y in CSS px." },
                    "destination_ref": {
                        "type": "string",
                        "description": "Drag destination page ref in the same frame.",
                    },
                    "to_x": { "type": "number", "description": "Drag destination viewport x in CSS px." },
                    "to_y": { "type": "number", "description": "Drag destination viewport y in CSS px." },
                    "delta_x": { "type": "number", "description": "Horizontal scroll delta in CSS px." },
                    "delta_y": { "type": "number", "description": "Vertical scroll delta in CSS px." },
                    "input_route": { "type": "string", "enum": ["trusted", "dom_event"] },
                },
                "required": ["target_id", "action"],
                "additionalProperties": false,
            }),
            false,
        ),
        browser_entry(
            "computer_browser_press",
            "Press key in browser tab",
            "Submit a focused browser field: sends Enter through the driver's trusted keystroke path (mode \"keystrokes\", text \"\\n\"). Works headless with no window activation. Use after computer_browser_type to submit search or a form. Pass the bind result's target_id and the tab's own tab_id; tab_id may be omitted when the target has one (or one active) tab.",
            json!({
                "type": "object",
                "properties": {
                    "target_id": target_id_property(),
                    "tab_id": tab_id_property(),
                    "ref": ref_property(),
                },
                "required": ["target_id", "ref"],
                "additionalProperties": false,
            }),
            false,
        ),
    ]
}
