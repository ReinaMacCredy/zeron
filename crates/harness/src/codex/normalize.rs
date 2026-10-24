//! Codex app-server notification/item → [`AgentEvent`] mapping, ported from
//! codex.ts's `mapItem`/notification switch.
//!
//! Tolerant by construction: both field spellings the app server has shipped
//! (`delta`/`textDelta`, `exitCode`/`exit_code`, camelCase/snake_case item
//! types) are accepted, and unknown item types map to nothing.

use serde_json::Value;
use zeron_proto::{AgentEvent, DoneStatus, TodoItem, TodoStatus, ToolCall, ToolDiff};

use crate::acp::normalize::{capped_diff, patch_diff};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Phase {
    Started,
    Completed,
}

fn field<'a>(v: &'a Value, keys: &[&str]) -> Option<&'a Value> {
    keys.iter().find_map(|k| v.get(*k))
}

fn str_field(v: &Value, keys: &[&str]) -> String {
    field(v, keys)
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_owned()
}

/// Delta text under either spelling the app server has used
/// (`delta` on agentMessage, `textDelta` on some reasoning builds).
pub(crate) fn delta_text(params: &Value) -> Option<String> {
    field(params, &["delta", "textDelta"])
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
}

/// Codex streams independent reasoning items and indexed parts without text
/// separators. Preserve those boundaries before the document folds deltas
/// together; otherwise adjacent Markdown headings become `**one****two**`.
/// One instance belongs to one thread, so child activity cannot split a
/// parent's in-flight paragraph.
#[derive(Default)]
pub(crate) struct ReasoningStream {
    last_part: Option<(String, bool, u64)>,
    announced_summary: Option<(String, u64)>,
    trailing_newlines: usize,
}

impl ReasoningStream {
    pub(crate) fn map(&mut self, method: &str, params: &Value) -> Vec<AgentEvent> {
        let id = item_id(params);
        let summary = method != "item/reasoning/textDelta";
        let index = field(
            params,
            if summary {
                &["summaryIndex", "summary_index"]
            } else {
                &["contentIndex", "content_index"]
            },
        )
        .and_then(Value::as_u64)
        .or_else(|| {
            self.announced_summary
                .as_ref()
                .filter(|(item, _)| summary && item == &id)
                .map(|(_, index)| *index)
        })
        .unwrap_or(0);
        if method == "item/reasoning/summaryPartAdded" {
            self.announced_summary = Some((id, index));
            return Vec::new();
        }
        let Some(text) = delta_text(params) else {
            return Vec::new();
        };
        let part = (id, summary, index);
        let mut events = Vec::new();
        if self.last_part.as_ref().is_some_and(|last| last != &part) {
            let leading_newlines = text.chars().take_while(|&c| c == '\n').count();
            let missing = 2usize.saturating_sub(self.trailing_newlines + leading_newlines);
            if missing > 0 {
                events.push(AgentEvent::ReasoningDelta {
                    text: "\n".repeat(missing),
                });
                self.trailing_newlines += missing;
            }
        }
        let trailing = text.chars().rev().take_while(|&c| c == '\n').count();
        self.trailing_newlines = if trailing == text.len() {
            self.trailing_newlines + trailing
        } else {
            trailing
        };
        self.last_part = Some(part);
        events.push(AgentEvent::ReasoningDelta { text });
        events
    }
}

pub(crate) fn item_id(params: &Value) -> String {
    str_field(params, &["itemId", "item_id"])
}

/// `params.turn.id` on the turn/* lifecycle notifications.
pub(crate) fn turn_id(params: &Value) -> String {
    params
        .get("turn")
        .map(|t| str_field(t, &["id"]))
        .unwrap_or_default()
}

/// `params.turn.error` as display text (turn/completed carries an optional
/// error; turn/failed always should). `limits` is the account's latest
/// `account/rateLimits/updated` snapshot, for a limit error's reset time.
pub(crate) fn turn_error_message(params: &Value, limits: Option<&Value>) -> Option<String> {
    params
        .get("turn")
        .and_then(|t| t.get("error"))
        .filter(|e| !e.is_null())
        .map(|e| codex_error_text(e, limits))
}

/// What the person reads when the provider refuses Codex's credentials (a
/// 401): a missing or expired ChatGPT login, or an API key — OpenAI's or a
/// custom provider's — that is wrong or revoked. The UI offers its sign-in
/// action on this text (`zeron_proto::run_errors` matches "run codex login").
pub(crate) const SIGNED_OUT_MESSAGE: &str = "Codex's credentials were rejected. Run codex login \
     (or check the API key for your model provider), then send again.";

/// A Codex `TurnError` (`turn.error`, or an `error` notification's `error`)
/// as display text. The structured `codexErrorInfo` decides the sign-in and
/// limit cases instead of the prose (a retry note mentioning "rate limit"
/// is not a limit); `additionalDetails`, when present, says more than the
/// generic message.
pub(crate) fn codex_error_text(error: &Value, limits: Option<&Value>) -> String {
    let message = str_field(error, &["message"]);
    let details = error
        .get("additionalDetails")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|d| !d.is_empty());
    let info = error.get("codexErrorInfo").unwrap_or(&Value::Null);
    let code = match info {
        Value::String(code) => Some(code.as_str()),
        Value::Object(variant) => variant.keys().next().map(String::as_str),
        _ => None,
    };
    // The transport variants carry the upstream status: a missing or dead
    // login fails as `httpConnectionFailed { httpStatusCode: 401 }` (live,
    // 0.164.0), and retries exhausted on a 429 are a rate limit.
    let http_status = info
        .as_object()
        .and_then(|variant| variant.values().next())
        .and_then(|detail| detail.get("httpStatusCode"))
        .and_then(Value::as_u64);
    let limit = match (code, http_status) {
        (Some("unauthorized"), _) | (_, Some(401)) => return SIGNED_OUT_MESSAGE.to_owned(),
        (Some("usageLimitExceeded"), _) => Some("Codex usage limit reached"),
        (Some("rateLimitExceeded"), _) | (_, Some(429)) => Some("Codex rate limit reached"),
        _ => None,
    };
    let text = details.map(str::to_owned).unwrap_or_else(|| {
        if message.is_empty() {
            error.to_string()
        } else {
            message
        }
    });
    let Some(limit) = limit else {
        return text;
    };
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64);
    match limits.and_then(|limits| limit_reset_in(limits, now)) {
        Some(wait) => format!("{limit} — it resets in {}.", describe_wait(wait)),
        None if text.is_empty() => format!("{limit}."),
        None => format!("{limit}. {text}"),
    }
}

/// Seconds until an exhausted rate-limit window of `limits` (a
/// `RateLimitSnapshot`) resets: the latest one, since usage resumes only
/// when every exhausted window has. `None` when no window is exhausted —
/// the limit hit is one the snapshot doesn't describe.
fn limit_reset_in(limits: &Value, now: i64) -> Option<i64> {
    ["primary", "secondary"]
        .iter()
        .filter_map(|key| limits.get(*key))
        .filter(|window| {
            window
                .get("usedPercent")
                .and_then(Value::as_f64)
                .is_some_and(|used| used >= 100.0)
        })
        .filter_map(|window| window.get("resetsAt").and_then(Value::as_i64))
        .max()
        .map(|at| (at - now).max(0))
}

fn describe_wait(secs: i64) -> String {
    let minutes = (secs + 59) / 60;
    let (days, hours, minutes) = (minutes / 1440, minutes / 60 % 24, minutes % 60);
    match (days, hours, minutes) {
        (0, 0, 0) => "under a minute".into(),
        (0, 0, m) => format!("{m}m"),
        (0, h, 0) => format!("{h}h"),
        (0, h, m) => format!("{h}h {m}m"),
        (d, 0, _) => format!("{d}d"),
        (d, h, _) => format!("{d}d {h}h"),
    }
}

/// Fold a sparse `account/rateLimits/updated` snapshot into the last one:
/// a window or field the update leaves out (or null) keeps its value.
pub(crate) fn merge_rate_limits(held: &mut Option<Value>, params: &Value) {
    let Some(update) = params.get("rateLimits").and_then(Value::as_object) else {
        return;
    };
    let held = held.get_or_insert_with(|| Value::Object(Default::default()));
    if let Some(held) = held.as_object_mut() {
        for (key, value) in update {
            if !value.is_null() {
                held.insert(key.clone(), value.clone());
            }
        }
    }
}

/// An `error` notification. 0.146.x nests it (`params.error`); older builds
/// were flat (`params.message`).
pub(crate) struct ErrorNotice {
    pub(crate) message: String,
    /// Codex is retrying the request itself ("Reconnecting… 2/5"): progress,
    /// not an outcome. Missing on old builds, which only sent final errors.
    pub(crate) will_retry: bool,
}

pub(crate) fn error_notice(params: &Value, limits: Option<&Value>) -> ErrorNotice {
    let message = match params.get("error").filter(|e| e.is_object()) {
        Some(error) => codex_error_text(error, limits),
        None => str_field(params, &["message"]),
    };
    ErrorNotice {
        message: if message.is_empty() {
            "Codex error".into()
        } else {
            message
        },
        will_retry: params
            .get("willRetry")
            .and_then(Value::as_bool)
            .unwrap_or(false),
    }
}

/// How a settled turn reports its error, given the last final `error`
/// notification it raised (`held`). The doc writes an error part for both
/// an [`AgentEvent::Error`] and a `Done` error, so a message goes to exactly
/// one: `turn.error` wins; a failed turn without one takes `held`; a turn
/// that did not fail keeps `held` as a standalone notice. Returns the Done
/// error and that notice.
pub(crate) fn settle_turn_error(
    params: &Value,
    held: Option<String>,
    limits: Option<&Value>,
) -> (Option<String>, Option<String>) {
    let failed = params.pointer("/turn/status").and_then(Value::as_str) == Some("failed");
    match turn_error_message(params, limits) {
        Some(error) => (Some(error), None),
        None if failed => (
            Some(held.unwrap_or_else(|| "Codex turn failed".to_owned())),
            None,
        ),
        None => (None, held),
    }
}

/// The last `account/rateLimits/updated` windows. Updates are sparse: an
/// absent or null window keeps the value seen before.
#[derive(Default)]
pub(crate) struct RateLimitWindows([Option<(f64, Option<i64>)>; 2]);

impl RateLimitWindows {
    pub(crate) fn note(&mut self, params: &Value) {
        let Some(snapshot) = params.get("rateLimits") else {
            return;
        };
        for (slot, key) in self.0.iter_mut().zip(["primary", "secondary"]) {
            if let Some(window) = snapshot.get(key).filter(|w| !w.is_null()) {
                *slot = Some((
                    window
                        .get("usedPercent")
                        .and_then(Value::as_f64)
                        .unwrap_or(0.0),
                    window.get("resetsAt").and_then(Value::as_i64),
                ));
            }
        }
    }

    pub(crate) fn usage_limit(&self, error: &Value) -> Option<AgentEvent> {
        let code = match error.get("codexErrorInfo")? {
            Value::String(code) => code.as_str(),
            Value::Object(map) => map.keys().next()?.as_str(),
            _ => return None,
        };
        if !matches!(code, "usageLimitExceeded" | "rateLimitExceeded") {
            return None;
        }
        let resets_at = self
            .0
            .iter()
            .flatten()
            .filter(|(used, _)| *used >= 100.0)
            .try_fold(None, |latest: Option<i64>, (_, reset)| {
                reset.map(|secs| latest.max(Some(secs.saturating_mul(1000))))
            })
            .flatten();
        Some(AgentEvent::UsageLimit { resets_at })
    }
}

fn usage_counts(snapshot: &Value) -> [u64; 4] {
    let count = |keys: &[&str]| {
        field(snapshot, keys)
            .and_then(Value::as_u64)
            .unwrap_or_default()
    };
    [
        count(&["inputTokens", "input_tokens"]),
        count(&["cachedInputTokens", "cached_input_tokens"]),
        count(&["cacheWriteInputTokens", "cache_write_input_tokens"]),
        count(&["outputTokens", "output_tokens"]),
    ]
}

/// A turn's token usage summed over its model requests. Each
/// `thread/tokenUsage/updated` carries one request (`last`) and the thread's
/// running `total`; the total's growth is the request, so a repeated snapshot
/// adds nothing. The first snapshot a process sees has history in `total`.
#[derive(Default)]
pub(crate) struct UsageTally {
    seen: Option<[u64; 4]>,
    turn: [u64; 4],
}

impl UsageTally {
    pub(crate) fn note(&mut self, params: &Value) {
        let Some(usage) = field(params, &["tokenUsage", "token_usage"]) else {
            return;
        };
        let total = usage.get("total").map(usage_counts);
        let step = match (self.seen, total) {
            (Some(seen), Some(total)) => std::array::from_fn(|i| total[i].saturating_sub(seen[i])),
            _ => usage.get("last").map(usage_counts).unwrap_or_default(),
        };
        self.seen = total.or(self.seen);
        for (sum, n) in self.turn.iter_mut().zip(step) {
            *sum += n;
        }
    }

    /// Codex counts cached input inside `inputTokens`; the events split it
    /// out so it is priced as a cache read.
    pub(crate) fn take(&mut self) -> Vec<AgentEvent> {
        let [input, cached, written, output] = std::mem::take(&mut self.turn);
        if input + output == 0 {
            return Vec::new();
        }
        let mut events = vec![AgentEvent::Usage {
            input_tokens: input.saturating_sub(cached + written),
            output_tokens: output,
        }];
        if cached + written > 0 {
            events.push(AgentEvent::PromptCache {
                read_tokens: cached,
                write_tokens: written,
                ttl_secs: None,
            });
        }
        events
    }
}

pub(crate) fn context_usage_event(params: &Value) -> Option<AgentEvent> {
    let usage = field(params, &["tokenUsage", "token_usage"])?;
    let last = usage.get("last");
    let tokens = last
        .and_then(|last| field(last, &["totalTokens", "total_tokens"]).and_then(Value::as_u64))
        .or_else(|| {
            let last = last?;
            let input = field(last, &["inputTokens", "input_tokens"])?.as_u64()?;
            Some(
                input.saturating_add(
                    field(last, &["outputTokens", "output_tokens"])
                        .and_then(Value::as_u64)
                        .unwrap_or(0),
                ),
            )
        });
    let window = field(usage, &["modelContextWindow", "model_context_window"])
        .and_then(Value::as_u64)
        .filter(|n| *n > 0);
    (tokens.is_some() || window.is_some()).then_some(AgentEvent::ContextUsage { tokens, window })
}

/// Tool-shaped Codex items must always close the lifecycle they open: started
/// opens the ToolCall, completed refreshes its metadata and resolves the same
/// stable id (port of codex.ts `toolLifecycle`).
fn tool_lifecycle(phase: Phase, id: String, call: ToolCall, is_error: bool) -> Vec<AgentEvent> {
    match phase {
        Phase::Started => vec![AgentEvent::ToolCall { id, call }],
        Phase::Completed => vec![
            AgentEvent::ToolCall {
                id: id.clone(),
                call,
            },
            AgentEvent::ToolResult {
                id,
                is_error,
                output: None,
                diff: None,
            },
        ],
    }
}

/// A `fileChange` item's `changes` array reduced to the typed [`ToolCall`] the
/// UI renders: a lone `add` is a file write, a lone `update` an edit, anything
/// else (deletes, multi-file changes) a patch.
fn file_change_call(changes: &[(String, String)]) -> ToolCall {
    match changes {
        [(path, kind)] if kind == "add" => ToolCall::WriteFile {
            path: path.clone(),
            content: None,
        },
        [(path, kind)] if kind == "update" => ToolCall::EditFile {
            path: path.clone(),
            old_string: None,
            new_string: None,
        },
        [(path, _)] => ToolCall::ApplyPatch {
            path: Some(path.clone()),
        },
        _ => ToolCall::ApplyPatch { path: None },
    }
}

/// v2 sends `{"type": "add"}`; older builds sent a bare string.
fn change_kind(change: &Value) -> &str {
    // Unknown kinds degrade to "update", like codex.ts.
    change
        .get("kind")
        .and_then(|k| k.as_str().or_else(|| k.get("type").and_then(Value::as_str)))
        .filter(|k| matches!(*k, "add" | "delete" | "update"))
        .unwrap_or("update")
}

/// Codex's `diff` is the whole file for an add or delete, a unified diff for an update.
fn file_change_diff(change: &Value) -> Option<ToolDiff> {
    let path = str_field(change, &["path"]);
    let text = change.get("diff").and_then(Value::as_str)?;
    match change_kind(change) {
        "add" => Some(capped_diff(path, None, text)),
        "delete" => Some(capped_diff(path, Some(text), "")),
        _ => patch_diff(path, text),
    }
}

/// `turn/plan/updated` → the live checklist chip. Steps carry
/// `pending | inProgress | completed`; like ACP plans the update has no item
/// id, so it refreshes the singleton [`zeron_proto::LIVE_PLAN_TOOL_ID`] in place.
pub(crate) fn plan_update_events(params: &Value) -> Vec<AgentEvent> {
    let Some(plan) = params.get("plan").and_then(Value::as_array) else {
        return Vec::new();
    };
    let items = plan
        .iter()
        .map(|step| {
            TodoItem::new(
                str_field(step, &["step"]),
                TodoStatus::parse(&str_field(step, &["status"])),
            )
        })
        .collect();
    tool_lifecycle(
        Phase::Completed,
        zeron_proto::LIVE_PLAN_TOOL_ID.into(),
        ToolCall::Todo { items },
        false,
    )
}

pub(crate) fn item_type(item: &Value) -> &str {
    item.get("type").and_then(Value::as_str).unwrap_or("")
}

pub(super) fn is_collab_spawn(item: &Value) -> bool {
    matches!(
        item_type(item),
        "collabAgentToolCall" | "collab_agent_tool_call"
    ) && matches!(
        item.get("tool").and_then(Value::as_str),
        Some("spawnAgent" | "spawn_agent")
    )
}

/// A v1 spawn names its child in the completed result. Other collaboration
/// tools can address several receivers, but only a spawn owns a transcript.
pub(super) fn collab_spawn_child(item: &Value) -> Option<&str> {
    if !is_collab_spawn(item)
        || matches!(
            item.get("status").and_then(Value::as_str),
            Some("failed" | "errored")
        )
    {
        return None;
    }
    let receivers = field(item, &["receiverThreadIds", "receiver_thread_ids"])?.as_array()?;
    match receivers.as_slice() {
        [child] => child.as_str().filter(|id| !id.is_empty()),
        _ => None,
    }
}

/// Map one `item/started` or `item/completed` payload's item to events.
/// `agentMessage` and `reasoning` flow through their delta channels and are
/// handled by the session loop, not here.
pub(crate) fn map_item(phase: Phase, item: &Value) -> Vec<AgentEvent> {
    let id = str_field(item, &["id"]);
    let status = str_field(item, &["status"]);
    match item_type(item) {
        "imageGeneration" | "image_generation" => {
            let call = ToolCall::Unknown {
                name: "Generate image".into(),
                input: None,
            };
            if phase == Phase::Started {
                return vec![AgentEvent::ToolCall { id, call }];
            }
            // `result` can be megabytes of inline media. Never inspect, clone,
            // log, or forward it: savedPath is the sole supported source.
            let path = str_field(item, &["savedPath", "saved_path"]);
            let failure = item.get("failure").filter(|v| !v.is_null());
            let error = if let Some(failure) = failure {
                Some(
                    if matches!(
                        failure.get("type").and_then(Value::as_str),
                        Some("usageLimitExceeded" | "usage_limit_exceeded")
                    ) {
                        "Image generation usage limit exceeded"
                    } else {
                        "Image generation failed"
                    },
                )
            } else if matches!(status.as_str(), "failed" | "cancelled" | "canceled") {
                Some("Image generation failed")
            } else if path.trim().is_empty() {
                Some("Image generation completed without a saved file")
            } else {
                None
            };
            let mut events = vec![
                AgentEvent::ToolCall {
                    id: id.clone(),
                    call,
                },
                AgentEvent::ToolResult {
                    id: id.clone(),
                    is_error: error.is_some(),
                    output: None,
                    diff: None,
                },
            ];
            if let Some(message) = error {
                events.push(AgentEvent::Error {
                    message: message.into(),
                });
            } else {
                let name = std::path::Path::new(&path)
                    .file_name()
                    .and_then(|v| v.to_str())
                    .unwrap_or("generated.png")
                    .to_owned();
                events.push(AgentEvent::GeneratedImage {
                    id: format!("{id}:image"),
                    path,
                    name,
                    mime_type: String::new(),
                });
            }
            events
        }
        "commandExecution" | "command_execution" => match phase {
            Phase::Started => vec![AgentEvent::ToolCall {
                id,
                call: ToolCall::Exec {
                    command: str_field(item, &["command"]),
                },
            }],
            Phase::Completed => {
                let exit_code = field(item, &["exitCode", "exit_code"])
                    .and_then(Value::as_i64)
                    .unwrap_or(0);
                vec![AgentEvent::ToolResult {
                    id,
                    is_error: status == "failed" || exit_code != 0,
                    output: None,
                    diff: None,
                }]
            }
        },
        "fileChange" | "file_change" => {
            let raw = item
                .get("changes")
                .and_then(Value::as_array)
                .map(|a| a.as_slice())
                .unwrap_or_default();
            let changes: Vec<(String, String)> = raw
                .iter()
                .map(|c| (str_field(c, &["path"]), change_kind(c).to_owned()))
                .collect();
            let is_error = status == "failed" || status == "declined";
            let mut events = tool_lifecycle(phase, id, file_change_call(&changes), is_error);
            if !is_error
                && let [change] = raw
                && let Some(AgentEvent::ToolResult { diff, .. }) = events.last_mut()
            {
                *diff = file_change_diff(change);
            }
            events
        }
        "mcpToolCall" | "mcp_tool_call" => match phase {
            Phase::Started => {
                let input = item.get("arguments").filter(|v| !v.is_null()).cloned();
                vec![AgentEvent::ToolCall {
                    id,
                    call: ToolCall::Mcp {
                        server: str_field(item, &["server"]),
                        tool: str_field(item, &["tool"]),
                        input,
                    },
                }]
            }
            Phase::Completed => vec![AgentEvent::ToolResult {
                id,
                is_error: status == "failed",
                output: None,
                diff: None,
            }],
        },
        "webSearch" | "web_search" => tool_lifecycle(
            phase,
            id,
            ToolCall::WebSearch {
                query: str_field(item, &["query"]),
            },
            false,
        ),
        "todoList" | "todo_list" => {
            let items = item
                .get("items")
                .and_then(Value::as_array)
                .map(|a| a.as_slice())
                .unwrap_or_default()
                .iter()
                .map(|t| {
                    let done =
                        field(t, &["completed", "done"]).and_then(Value::as_bool) == Some(true);
                    TodoItem::new(
                        str_field(t, &["text"]),
                        if done {
                            TodoStatus::Completed
                        } else {
                            TodoStatus::Pending
                        },
                    )
                })
                .collect();
            tool_lifecycle(phase, id, ToolCall::Todo { items }, false)
        }
        "error" => vec![AgentEvent::Error {
            message: str_field(item, &["message"]),
        }],
        "collabAgentToolCall" | "collab_agent_tool_call" => {
            let tool = str_field(item, &["tool"]);
            let name = if is_collab_spawn(item) {
                "Agent".to_owned()
            } else {
                match tool.as_str() {
                    "sendInput" | "send_input" => "Send agent message".to_owned(),
                    "wait" => "Wait for agents".to_owned(),
                    "closeAgent" | "close_agent" => "Close agent".to_owned(),
                    "resumeAgent" | "resume_agent" => "Resume agent".to_owned(),
                    _ => format!("Agent control: {tool}"),
                }
            };
            tool_lifecycle(
                phase,
                id,
                ToolCall::Unknown {
                    name,
                    input: Some(item.clone()),
                },
                matches!(status.as_str(), "failed" | "errored"),
            )
        }
        // A subagent spawn/lifecycle marker on the PARENT thread (multi-agent
        // v2, codex 0.146.x): the parent-feed chip for the child thread. The
        // child's own traffic routes separately (see `route_child_notification`
        // in mod.rs); this is only the spawn tool call the chip folds from.
        "subAgentActivity" | "sub_agent_activity" => {
            // A lifecycle marker has its own item id. It is not another
            // spawn; the child's turn notifications carry its terminal state.
            if !matches!(
                item.get("kind").and_then(Value::as_str),
                Some("started" | "spawned")
            ) {
                return Vec::new();
            }
            let name = str_field(item, &["agentPath"])
                .rsplit('/')
                .find(|s| !s.is_empty())
                .map(|leaf| format!("Agent: {leaf}"))
                .unwrap_or_else(|| "Agent".to_owned());
            tool_lifecycle(
                phase,
                id,
                ToolCall::Unknown {
                    name,
                    input: Some(item.clone()),
                },
                matches!(str_field(item, &["kind"]).as_str(), "failed" | "errored"),
            )
        }
        // reasoning / agentMessage flow through delta channels; the PARENT
        // feed's userMessage items are echoes of prompts we sent (already in
        // the doc). A CHILD thread's userMessage is different — the parent
        // steering its subagent — and is mapped where child items route
        // (mod.rs child branch), not here.
        _ => Vec::new(),
    }
}

/// The text of a `userMessage` thread item. Codex builds have carried both
/// shapes: a plain `text` field and a `content` array of text blocks.
pub(crate) fn user_message_text(item: &Value) -> Option<String> {
    let text = str_field(item, &["text"]);
    if !text.trim().is_empty() {
        return Some(text);
    }
    let joined: String = item
        .get("content")
        .and_then(Value::as_array)
        .map(|a| a.as_slice())
        .unwrap_or_default()
        .iter()
        .filter_map(|b| b.get("text").and_then(Value::as_str))
        .collect::<Vec<_>>()
        .join("\n\n");
    (!joined.trim().is_empty()).then_some(joined)
}

/// Per-child stream state, retained across parent turns and follow-up tasks.
/// Completion-only messages need the same text fallback as the root. Replayed
/// user items must not reopen a finished document or duplicate a steering entry.
#[derive(Default)]
pub(super) struct ChildStream {
    reasoning: ReasoningStream,
    streamed_text: std::collections::HashSet<String>,
    completed_items: std::collections::VecDeque<String>,
    completed_turns: std::collections::VecDeque<String>,
    settled: bool,
    /// The last final `error` notification, reported by the turn's end.
    held_error: Option<String>,
}

fn remember(ids: &mut std::collections::VecDeque<String>, id: String) -> bool {
    if id.is_empty() {
        return true;
    }
    if ids.contains(&id) {
        return false;
    }
    if ids.len() == 256 {
        ids.pop_front();
    }
    ids.push_back(id);
    true
}

impl ChildStream {
    pub(super) fn map(&mut self, child: &str, method: &str, params: &Value) -> Vec<AgentEvent> {
        if method == "turn/started" {
            let id = turn_id(params);
            if !id.is_empty() && self.completed_turns.contains(&id) {
                return Vec::new();
            }
            if self.settled {
                self.settled = false;
                // v2 followup_task starts a new child turn without echoing a
                // userMessage. Reopen the existing document without inventing
                // a user prompt or attributing an activity id as a new spawn.
                return vec![AgentEvent::Steered {
                    assistant_message_id: None,
                    next_assistant_message_id: None,
                }];
            }
            return Vec::new();
        }
        if matches!(method, "item/started" | "item/completed") {
            let item = params.get("item").unwrap_or(&Value::Null);
            let phase = if method == "item/started" {
                Phase::Started
            } else {
                Phase::Completed
            };
            if phase == Phase::Completed
                && !remember(&mut self.completed_items, str_field(item, &["id"]))
            {
                return Vec::new();
            }
            if matches!(item_type(item), "userMessage" | "user_message") {
                return if phase == Phase::Completed {
                    user_message_text(item)
                        .map(|text| {
                            self.settled = false;
                            AgentEvent::UserMessage { text }
                        })
                        .into_iter()
                        .collect()
                } else {
                    Vec::new()
                };
            }
            if self.settled {
                return Vec::new();
            }
            if matches!(item_type(item), "agentMessage" | "agent_message") {
                if phase == Phase::Started {
                    return Vec::new();
                }
                let mut events = Vec::new();
                if !self.streamed_text.remove(&str_field(item, &["id"])) {
                    let text = str_field(item, &["text"]);
                    if !text.is_empty() {
                        events.push(AgentEvent::TextDelta { text });
                    }
                }
                events.push(AgentEvent::TextDelta {
                    text: "\n\n".into(),
                });
                return events;
            }
            return map_item(phase, item);
        }
        if self.settled {
            return Vec::new();
        }
        match method {
            "item/agentMessage/delta" => {
                let id = item_id(params);
                if !id.is_empty() && self.completed_items.contains(&id) {
                    return Vec::new();
                }
                self.streamed_text.insert(id);
                delta_text(params)
                    .map(|text| AgentEvent::TextDelta { text })
                    .into_iter()
                    .collect()
            }
            "item/reasoning/textDelta"
            | "item/reasoning/summaryTextDelta"
            | "item/reasoning/summaryPartAdded" => self.reasoning.map(method, params),
            "turn/completed" | "turn/failed" | "turn/aborted" | "thread/closed" => {
                if method != "thread/closed"
                    && !remember(&mut self.completed_turns, turn_id(params))
                {
                    return Vec::new();
                }
                self.settled = true;
                self.streamed_text.clear();
                let (error, notice) = match settle_turn_error(params, self.held_error.take(), None)
                {
                    (None, held) if method == "turn/failed" => (held, None),
                    settled => settled,
                };
                let status = if method == "turn/failed"
                    || error.is_some()
                    || params.pointer("/turn/status").and_then(Value::as_str) == Some("failed")
                {
                    DoneStatus::Errored
                } else if method == "turn/aborted"
                    || params.pointer("/turn/status").and_then(Value::as_str) == Some("interrupted")
                {
                    DoneStatus::Interrupted
                } else {
                    DoneStatus::Completed
                };
                notice
                    .map(|message| AgentEvent::Error { message })
                    .into_iter()
                    .chain([AgentEvent::Done {
                        status,
                        result: None,
                        error,
                        session_id: Some(child.to_owned()),
                    }])
                    .collect()
            }
            // Same rules as the parent turn: a retry is progress, and a final
            // error is reported once, by the turn's end.
            "error" => {
                let notice = error_notice(params, None);
                if notice.will_retry {
                    tracing::debug!(
                        target: "zeron_harness::codex",
                        child, "subagent retrying: {}", notice.message
                    );
                } else {
                    self.held_error = Some(notice.message);
                }
                Vec::new()
            }
            _ => Vec::new(),
        }
    }
}

/// The thread a notification is addressed to: `thread/started` carries it at
/// `params.thread.id`, everything else at `params.threadId`. `None` for
/// thread-less methods (old builds, account noise).
pub(crate) fn notification_thread_id(method: &str, params: &Value) -> Option<String> {
    if method == "thread/started" {
        return params
            .get("thread")
            .and_then(|t| t.get("id"))
            .and_then(Value::as_str)
            .map(str::to_owned);
    }
    field(params, &["threadId", "thread_id"])
        .and_then(Value::as_str)
        .map(str::to_owned)
}

/// How a notification addressed to a REGISTERED child thread is handled.
///
/// Exported and pure so the table can be asserted directly. The shape (and
/// the fail-open default) is load-bearing: two shipped bugs in t3code's codex
/// runtime came from a catch-all that swallowed everything a child emitted —
/// a child's `error` vanished (the agent card stayed running forever) and a
/// swallowed `serverRequest/resolved` left the parent's approvals stuck.
///
/// - `Subagent`: content/lifecycle attributed to the child (item lifecycles,
///   errors, closure) — mapped to tagged [`AgentEvent::Subagent`] events.
/// - `Consumed`: child bookkeeping with no parent or subagent-doc meaning,
///   plus child thread-lifecycle methods that would rewrite PARENT state if
///   let through (`thread/started` repeats, status/name/usage updates).
/// - `Parent`: pass through to the parent path — unknown methods land here BY
///   DESIGN, so a codex update that adds a notification degrades to "the
///   parent sees it", never to silent loss.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ChildRoute {
    Subagent,
    Consumed,
    Parent,
}

pub(crate) fn route_child_notification(method: &str) -> ChildRoute {
    match method {
        // Child message/reasoning deltas DO stream on this wire tagged with
        // the child's threadId (live-verified against codex-cli 0.146.1
        // multi-agent capture) — the subagent transcript is token-level live
        // without touching the rollout file. Child TURN ends are the
        // subagent's terminal signal: `thread/closed` fires only via the
        // collab close_agent tool, which real fan-outs never call
        // (live-verified — chips stayed "running" forever without this).
        "item/started"
        | "item/completed"
        | "item/agentMessage/delta"
        | "item/reasoning/textDelta"
        | "item/reasoning/summaryTextDelta"
        | "item/reasoning/summaryPartAdded"
        | "turn/started"
        | "turn/completed"
        | "turn/failed"
        | "turn/aborted"
        | "error"
        | "thread/closed"
        | "thread/tokenUsage/updated" => ChildRoute::Subagent,
        // Child status bookkeeping with no subagent meaning: consumed
        // so it can never settle the PARENT turn (the exact bug class the
        // explicit table exists for).
        "thread/status/changed"
        // Child chatter with no consumer on this wire.
        | "item/commandExecution/outputDelta"
        | "item/fileChange/outputDelta"
        | "item/fileChange/patchUpdated"
        | "item/plan/delta"
        | "turn/plan/updated"
        | "turn/diff/updated"
        | "thread/name/updated"
        | "thread/settings/updated"
        | "rawResponseItem/completed"
        // Child-owned thread lifecycle that maps onto PARENT state in a
        // naive passthrough (archived/compacted), plus repeat thread/started.
        | "thread/archived"
        | "thread/unarchived"
        | "thread/compacted"
        | "thread/started" => ChildRoute::Consumed,
        // Unknown or parent-owned (approvals bookkeeping, account noise).
        _ => ChildRoute::Parent,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn usage_limit_resets_at_the_latest_exhausted_window() {
        let mut windows = RateLimitWindows::default();
        let limit = json!({"message": "limit", "codexErrorInfo": "usageLimitExceeded"});
        assert_eq!(
            windows.usage_limit(&limit),
            Some(AgentEvent::UsageLimit { resets_at: None })
        );
        windows.note(&json!({"rateLimits": {
            "primary": {"usedPercent": 100, "resetsAt": 50},
            "secondary": {"usedPercent": 40, "resetsAt": 900}
        }}));
        windows.note(&json!({"rateLimits": {"primary": null}}));
        assert_eq!(
            windows.usage_limit(&limit),
            Some(AgentEvent::UsageLimit {
                resets_at: Some(50_000)
            })
        );
        windows.note(&json!({"rateLimits": {"secondary": {"usedPercent": 100}}}));
        assert_eq!(
            windows.usage_limit(&json!({"codexErrorInfo": {"rateLimitExceeded": {}}})),
            Some(AgentEvent::UsageLimit { resets_at: None })
        );
        assert_eq!(
            windows.usage_limit(&json!({"codexErrorInfo": "serverOverloaded"})),
            None
        );
    }

    #[test]
    fn reasoning_parts_preserve_chunking_and_existing_paragraph_breaks() {
        let mut stream = ReasoningStream::default();
        let mut text = String::new();
        for params in [
            json!({"itemId":"r1", "contentIndex":0, "textDelta":"**First"}),
            json!({"itemId":"r1", "contentIndex":0, "textDelta":" heading**\n"}),
            json!({"itemId":"r1", "contentIndex":1, "delta":"\nSecond paragraph.\n\n"}),
            // An empty delta must not consume the next item's boundary.
            json!({"itemId":"r2", "contentIndex":0, "delta":""}),
            json!({"item_id":"r2", "content_index":0, "delta":"Third paragraph."}),
        ] {
            for event in stream.map("item/reasoning/textDelta", &params) {
                if let AgentEvent::ReasoningDelta { text: delta } = event {
                    text.push_str(&delta);
                }
            }
        }
        assert_eq!(
            text,
            "**First heading**\n\nSecond paragraph.\n\nThird paragraph."
        );
    }

    #[test]
    fn user_message_text_accepts_both_shapes() {
        assert_eq!(
            user_message_text(&json!({"text": "steer"})),
            Some("steer".into())
        );
        assert_eq!(
            user_message_text(&json!({"content": [
                {"type": "text", "text": "a"},
                {"type": "text", "text": "b"},
            ]})),
            Some("a\n\nb".into())
        );
        assert_eq!(user_message_text(&json!({"text": "  "})), None);
        assert_eq!(user_message_text(&json!({})), None);
    }

    #[test]
    fn delta_accepts_both_spellings() {
        assert_eq!(delta_text(&json!({"delta": "a"})), Some("a".into()));
        assert_eq!(delta_text(&json!({"textDelta": "b"})), Some("b".into()));
        assert_eq!(delta_text(&json!({"delta": ""})), None);
        assert_eq!(delta_text(&json!({})), None);
    }

    #[test]
    fn command_execution_maps_exit_code_to_error() {
        let started = map_item(
            Phase::Started,
            &json!({"type": "commandExecution", "id": "c1", "command": "ls"}),
        );
        assert_eq!(
            started,
            vec![AgentEvent::ToolCall {
                id: "c1".into(),
                call: ToolCall::Exec {
                    command: "ls".into()
                },
            }]
        );
        let completed = map_item(
            Phase::Completed,
            &json!({"type": "command_execution", "id": "c1", "status": "completed", "exit_code": 2}),
        );
        assert_eq!(
            completed,
            vec![AgentEvent::ToolResult {
                id: "c1".into(),
                is_error: true,
                output: None,
                diff: None,
            }]
        );
    }

    #[test]
    fn file_change_variants_map_to_typed_calls() {
        let add = map_item(
            Phase::Started,
            &json!({"type": "fileChange", "id": "f1", "changes": [{"path": "/a.rs", "kind": "add"}]}),
        );
        assert_eq!(
            add,
            vec![AgentEvent::ToolCall {
                id: "f1".into(),
                call: ToolCall::WriteFile {
                    path: "/a.rs".into(),
                    content: None
                },
            }]
        );
        let update = map_item(
            Phase::Completed,
            &json!({"type": "fileChange", "id": "f2", "status": "declined",
                    "changes": [{"path": "/b.rs", "kind": "update"}]}),
        );
        assert_eq!(
            update,
            vec![
                AgentEvent::ToolCall {
                    id: "f2".into(),
                    call: ToolCall::EditFile {
                        path: "/b.rs".into(),
                        old_string: None,
                        new_string: None
                    },
                },
                AgentEvent::ToolResult {
                    id: "f2".into(),
                    is_error: true,
                    output: None,
                    diff: None,
                },
            ]
        );
        let multi = map_item(
            Phase::Started,
            &json!({"type": "fileChange", "id": "f3",
                    "changes": [{"path": "/a"}, {"path": "/b", "kind": "delete"}]}),
        );
        assert_eq!(
            multi,
            vec![AgentEvent::ToolCall {
                id: "f3".into(),
                call: ToolCall::ApplyPatch { path: None },
            }]
        );
    }

    #[test]
    fn a_completed_file_change_carries_its_diff() {
        let result = |item: Value| {
            map_item(Phase::Completed, &item)
                .into_iter()
                .find_map(|e| match e {
                    AgentEvent::ToolResult { diff, .. } => Some(diff),
                    _ => None,
                })
                .unwrap()
        };
        let update = result(
            json!({"type": "fileChange", "id": "f1", "status": "completed",
            "changes": [{"path": "/b.rs", "kind": {"type": "update", "move_path": null},
                "diff": "@@ -4,3 +4,4 @@\n keep\n-old\n+new\n+more\n tail\n@@ -20 +21 @@\n-x\n+y\n"}]}),
        );
        assert_eq!(
            update,
            Some(ToolDiff {
                path: "/b.rs".into(),
                old_text: Some("keep\nold\ntail\nx\n".into()),
                new_text: "keep\nnew\nmore\ntail\ny\n".into(),
                line_offset: Some(3),
            })
        );
        let add = map_item(
            Phase::Completed,
            &json!({"type": "fileChange", "id": "f2", "status": "completed",
                "changes": [{"path": "/a.rs", "kind": {"type": "add"}, "diff": "a\nb\n"}]}),
        );
        assert_eq!(
            add[0],
            AgentEvent::ToolCall {
                id: "f2".into(),
                call: ToolCall::WriteFile {
                    path: "/a.rs".into(),
                    content: None
                },
            }
        );
        assert!(matches!(
            &add[1],
            AgentEvent::ToolResult { diff: Some(d), .. } if d.old_text.is_none() && d.new_text == "a\nb\n"
        ));
        let declined = result(
            json!({"type": "fileChange", "id": "f3", "status": "declined",
            "changes": [{"path": "/c.rs", "kind": {"type": "delete"}, "diff": "gone\n"}]}),
        );
        assert_eq!(declined, None);
    }

    #[test]
    fn plan_update_maps_to_the_live_checklist_with_in_progress() {
        let events = plan_update_events(&json!({
            "threadId": "t", "turnId": "u", "explanation": null,
            "plan": [
                { "step": "read", "status": "completed" },
                { "step": "fix", "status": "inProgress" },
                { "step": "test", "status": "pending" },
            ],
        }));
        assert_eq!(
            events,
            vec![
                AgentEvent::ToolCall {
                    id: zeron_proto::LIVE_PLAN_TOOL_ID.into(),
                    call: ToolCall::Todo {
                        items: vec![
                            TodoItem::new("read", TodoStatus::Completed),
                            TodoItem::new("fix", TodoStatus::InProgress),
                            TodoItem::new("test", TodoStatus::Pending),
                        ]
                    },
                },
                AgentEvent::ToolResult {
                    id: zeron_proto::LIVE_PLAN_TOOL_ID.into(),
                    is_error: false,
                    output: None,
                    diff: None,
                },
            ]
        );
        assert!(plan_update_events(&json!({ "threadId": "t" })).is_empty());
    }

    #[test]
    fn usage_sums_a_turns_requests_and_splits_cached_input() {
        let snapshot = |total: [u64; 3], last: [u64; 3]| {
            json!({"tokenUsage": {
                "total": {"inputTokens": total[0], "cachedInputTokens": total[1], "outputTokens": total[2]},
                "last": {"inputTokens": last[0], "cachedInputTokens": last[1], "outputTokens": last[2]},
            }})
        };
        let mut tally = UsageTally::default();
        assert!(tally.take().is_empty());
        tally.note(&snapshot([40_000, 30_000, 500], [13_747, 11_008, 90]));
        tally.note(&snapshot([53_858, 41_008, 565], [13_858, 11_008, 65]));
        tally.note(&snapshot([53_858, 41_008, 565], [13_858, 11_008, 65]));
        assert_eq!(
            tally.take(),
            [
                AgentEvent::Usage {
                    input_tokens: 2_739 + 2_850,
                    output_tokens: 155
                },
                AgentEvent::PromptCache {
                    read_tokens: 22_016,
                    write_tokens: 0,
                    ttl_secs: None
                },
            ]
        );
        tally.note(&json!({"token_usage": {
            "total": {"input_tokens": 53_860, "output_tokens": 567},
            "last": {"input_tokens": 2, "output_tokens": 2},
        }}));
        assert_eq!(
            tally.take(),
            [AgentEvent::Usage {
                input_tokens: 2,
                output_tokens: 2
            }]
        );
        tally.note(&json!({}));
        assert!(tally.take().is_empty());
    }

    #[test]
    fn v1_spawns_and_controls_have_distinct_roles() {
        for tool in ["spawnAgent", "spawn_agent"] {
            let mut item = json!({"type":"collabAgentToolCall", "id":"spawn", "tool":tool,
                "status":"completed", "receiverThreadIds":["child"], "model":"child-model"});
            assert_eq!(collab_spawn_child(&item), Some("child"));
            let events = map_item(Phase::Completed, &item);
            assert!(matches!(&events[0], AgentEvent::ToolCall { call, .. }
                if call.is_subagent_spawn() && call.subagent_model() == Some("child-model")));
            assert!(matches!(
                &events[1],
                AgentEvent::ToolResult {
                    is_error: false,
                    ..
                }
            ));
            item["status"] = "failed".into();
            assert_eq!(collab_spawn_child(&item), None);
            assert!(matches!(
                map_item(Phase::Completed, &item).last(),
                Some(AgentEvent::ToolResult { is_error: true, .. })
            ));
        }
        for tool in [
            "sendInput",
            "send_input",
            "wait",
            "closeAgent",
            "resumeAgent",
            "futureControl",
        ] {
            let item = json!({"type":"collabAgentToolCall", "id":"control", "tool":tool,
                "receiverThreadIds":["child"]});
            assert_eq!(collab_spawn_child(&item), None);
            assert!(
                matches!(&map_item(Phase::Started, &item)[0], AgentEvent::ToolCall { call, .. } if !call.is_subagent_spawn())
            );
        }
        for receivers in [json!([]), json!(["one", "two"]), json!([""])] {
            assert_eq!(
                collab_spawn_child(
                    &json!({"type":"collabAgentToolCall", "tool":"spawnAgent", "receiverThreadIds":receivers})
                ),
                None
            );
        }
    }

    #[test]
    fn activity_updates_are_not_spawns_in_either_item_phase() {
        for kind in [
            "interacted",
            "completed",
            "failed",
            "errored",
            "futureActivity",
        ] {
            let item = json!({"type":"subAgentActivity", "id":"activity", "kind":kind,
                "agentThreadId":"child", "agentPath":"/root/alpha"});
            assert!(map_item(Phase::Started, &item).is_empty());
            assert!(map_item(Phase::Completed, &item).is_empty());
        }
    }

    #[test]
    fn sub_agent_activity_maps_to_a_named_parent_chip() {
        let started = map_item(
            Phase::Started,
            &json!({"type": "subAgentActivity", "id": "call_1", "kind": "started",
                    "agentThreadId": "child-1", "agentPath": "/root/alpha"}),
        );
        assert_eq!(started.len(), 1);
        assert!(matches!(
            &started[0],
            AgentEvent::ToolCall { id, call: ToolCall::Unknown { name, .. } }
                if id == "call_1" && name == "Agent: alpha"
        ));
        let completed = map_item(
            Phase::Completed,
            &json!({"type": "subAgentActivity", "id": "call_1", "kind": "started",
                    "agentThreadId": "child-1", "agentPath": "/root/alpha"}),
        );
        assert!(matches!(
            completed.last(),
            Some(AgentEvent::ToolResult { id, is_error: false, .. }) if id == "call_1"
        ));
    }

    #[test]
    fn child_routing_table_fails_open_to_parent() {
        // Child content/lifecycle → the subagent path, deltas included
        // (live-verified: child threads stream them on this wire).
        for m in [
            "item/started",
            "item/completed",
            "item/agentMessage/delta",
            "item/reasoning/textDelta",
            "error",
            "thread/closed",
            "thread/tokenUsage/updated",
        ] {
            assert_eq!(route_child_notification(m), ChildRoute::Subagent, "{m}");
        }
        // Child TURN ENDS are the subagent's terminal signal…
        for m in ["turn/completed", "turn/aborted", "turn/failed"] {
            assert_eq!(route_child_notification(m), ChildRoute::Subagent, "{m}");
        }
        // A child turn start can reopen a completed assignment. It must never
        // reach the parent turn router.
        assert_eq!(
            route_child_notification("turn/started"),
            ChildRoute::Subagent
        );
        // Child-owned thread lifecycle would rewrite parent state — consumed.
        for m in ["thread/archived", "thread/compacted", "thread/started"] {
            assert_eq!(route_child_notification(m), ChildRoute::Consumed, "{m}");
        }
        // Unknown methods degrade to "parent sees it", never silent loss
        // (the two-shipped-bugs rule).
        for m in [
            "serverRequest/resolved",
            "thread/somethingBrandNew",
            "account/rateLimits/updated",
        ] {
            assert_eq!(route_child_notification(m), ChildRoute::Parent, "{m}");
        }
    }

    #[test]
    fn notification_thread_ids_read_both_shapes() {
        assert_eq!(
            notification_thread_id("thread/started", &json!({"thread": {"id": "th-c"}})),
            Some("th-c".into())
        );
        assert_eq!(
            notification_thread_id(
                "turn/completed",
                &json!({"threadId": "th-1", "turn": {"id": "t"}})
            ),
            Some("th-1".into())
        );
        assert_eq!(
            notification_thread_id("error", &json!({"message": "x"})),
            None
        );
    }

    #[test]
    fn turn_error_extraction() {
        assert_eq!(
            turn_error_message(
                &json!({"turn": {"id": "t", "error": {"message": "boom"}}}),
                None
            ),
            Some("boom".into())
        );
        assert_eq!(
            turn_error_message(&json!({"turn": {"id": "t"}}), None),
            None
        );
        assert_eq!(
            turn_error_message(&json!({"turn": {"error": null}}), None),
            None
        );
        // The longer explanation wins over the generic message.
        assert_eq!(
            turn_error_message(
                &json!({"turn": {"error": {"message": "stream error", "additionalDetails": "  upstream 502 from the model  "}}}),
                None
            ),
            Some("upstream 502 from the model".into())
        );
    }

    #[test]
    fn structured_error_info_decides_sign_in_and_limit_text() {
        use zeron_proto::run_errors::{ProviderProblem, provider_problem};
        let signed_out = codex_error_text(
            &json!({"message": "401 Unauthorized: token_expired", "codexErrorInfo": "unauthorized"}),
            None,
        );
        assert_eq!(signed_out, SIGNED_OUT_MESSAGE);
        // Live 0.164.0 with no login at all: the 401 rides a transport variant.
        assert_eq!(
            codex_error_text(
                &json!({"message": "unexpected status 401 Unauthorized: Missing bearer or basic authentication in header", "codexErrorInfo": {"httpConnectionFailed": {"httpStatusCode": 401}}, "additionalDetails": null}),
                None
            ),
            SIGNED_OUT_MESSAGE
        );
        assert_eq!(
            provider_problem(&signed_out),
            Some(ProviderProblem::SignedOut)
        );
        // A 401 is not always a ChatGPT login: API-key users are told too.
        assert!(signed_out.contains("API key"), "{signed_out}");

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;
        let limits = json!({
            "primary": {"usedPercent": 100, "resetsAt": now + 2 * 3600 + 14 * 60 - 5},
            "secondary": {"usedPercent": 40, "resetsAt": now + 6 * 86400},
        });
        let limited = codex_error_text(
            &json!({"message": "You've hit your usage limit.", "codexErrorInfo": "usageLimitExceeded"}),
            Some(&limits),
        );
        assert_eq!(limited, "Codex usage limit reached — it resets in 2h 14m.");
        assert_eq!(
            provider_problem(&limited),
            Some(ProviderProblem::LimitReached)
        );
        // No exhausted window known: keep Codex's own explanation.
        assert_eq!(
            codex_error_text(
                &json!({"message": "Slow down", "codexErrorInfo": "rateLimitExceeded"}),
                None
            ),
            "Codex rate limit reached. Slow down"
        );
        // Retries exhausted on a 429 are a rate limit too.
        assert!(
            codex_error_text(
                &json!({"message": "exceeded retry limit", "codexErrorInfo": {"responseTooManyFailedAttempts": {"httpStatusCode": 429}}}),
                None
            )
            .starts_with("Codex rate limit reached")
        );
        // Prose alone never classifies: a 500 that mentions limits stays as is.
        assert_eq!(
            codex_error_text(
                &json!({"message": "internal error near rate limit middleware", "codexErrorInfo": {"responseTooManyFailedAttempts": {"httpStatusCode": 500}}}),
                None
            ),
            "internal error near rate limit middleware"
        );
    }

    #[test]
    fn rate_limit_updates_merge_sparsely() {
        let mut held = None;
        merge_rate_limits(
            &mut held,
            &json!({"rateLimits": {"primary": {"usedPercent": 10, "resetsAt": 5}, "secondary": {"usedPercent": 1}}}),
        );
        merge_rate_limits(
            &mut held,
            &json!({"rateLimits": {"primary": {"usedPercent": 100, "resetsAt": 7}, "secondary": null}}),
        );
        let held = held.unwrap();
        assert_eq!(held["primary"]["usedPercent"], 100);
        assert_eq!(held["secondary"]["usedPercent"], 1);
        assert_eq!(limit_reset_in(&held, 0), Some(7));
        assert_eq!(describe_wait(0), "under a minute");
        assert_eq!(describe_wait(3600), "1h");
        assert_eq!(describe_wait(3 * 86400 + 5 * 3600), "3d 5h");
    }

    #[test]
    fn error_notices_separate_retries_from_final_errors() {
        let retry = error_notice(
            &json!({"error": {"message": "Reconnecting... 2/5 (rate limit)"}, "willRetry": true, "threadId": "t", "turnId": "u"}),
            None,
        );
        assert!(retry.will_retry);
        let last = error_notice(&json!({"message": "flat legacy error"}), None);
        assert!(!last.will_retry);
        assert_eq!(last.message, "flat legacy error");

        // turn.error wins and the held notice is dropped (never both).
        let failed = json!({"turn": {"id": "t", "status": "failed", "error": {"message": "boom"}}});
        assert_eq!(
            settle_turn_error(&failed, Some("held".into()), None),
            (Some("boom".into()), None)
        );
        // A failed turn without its own error takes the held one.
        let bare = json!({"turn": {"id": "t", "status": "failed"}});
        assert_eq!(
            settle_turn_error(&bare, Some("held".into()), None),
            (Some("held".into()), None)
        );
        assert_eq!(
            settle_turn_error(&bare, None, None),
            (Some("Codex turn failed".into()), None)
        );
        // A turn that did not fail keeps it as a notice.
        let done = json!({"turn": {"id": "t", "status": "completed"}});
        assert_eq!(
            settle_turn_error(&done, Some("held".into()), None),
            (None, Some("held".into()))
        );
    }

    #[test]
    fn child_retry_notices_vanish_and_final_errors_report_once() {
        let mut stream = ChildStream::default();
        assert!(
            stream
                .map(
                    "c",
                    "error",
                    &json!({"error": {"message": "Reconnecting... 1/5"}, "willRetry": true})
                )
                .is_empty()
        );
        assert!(
            stream
                .map(
                    "c",
                    "error",
                    &json!({"error": {"message": "stream failed"}, "willRetry": false})
                )
                .is_empty()
        );
        let events = stream.map(
            "c",
            "turn/completed",
            &json!({"turn": {"id": "ct", "status": "failed", "error": {"message": "stream failed"}}}),
        );
        assert_eq!(
            events,
            vec![AgentEvent::Done {
                status: DoneStatus::Errored,
                result: None,
                error: Some("stream failed".into()),
                session_id: Some("c".into()),
            }]
        );
    }
}

#[cfg(test)]
mod context_tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn context_uses_latest_call_not_thread_total() {
        assert_eq!(
            context_usage_event(&json!({"tokenUsage": {
                "last": {"totalTokens": 42000}, "total": {"totalTokens": 900000}, "modelContextWindow": 200000
            }})),
            Some(AgentEvent::ContextUsage {
                tokens: Some(42000),
                window: Some(200000)
            })
        );
        assert_eq!(
            context_usage_event(&json!({"token_usage": {
                "last": {"input_tokens": 0, "output_tokens": 0}, "model_context_window": 0
            }})),
            Some(AgentEvent::ContextUsage {
                tokens: Some(0),
                window: None
            })
        );
        assert_eq!(
            context_usage_event(&json!({"tokenUsage": {"total": {"totalTokens": 100}}})),
            None
        );
    }
}

#[cfg(test)]
mod generated_image_tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn image_generation_lifecycle_ignores_inline_result_and_preserves_ids() {
        for (kind, path_key) in [
            ("imageGeneration", "savedPath"),
            ("image_generation", "saved_path"),
        ] {
            let mut item = json!({"id":"img-1", "type":kind, "status":"completed", "result":"INLINE_IMAGE_SENTINEL".repeat(200_000), "revisedPrompt":"private prompt", "failure":null});
            item[path_key] = json!("/codex/generated_images/picture.png");
            let started = map_item(Phase::Started, &item);
            assert_eq!(
                started,
                vec![AgentEvent::ToolCall {
                    id: "img-1".into(),
                    call: ToolCall::Unknown {
                        name: "Generate image".into(),
                        input: None
                    }
                }]
            );
            let completed = map_item(Phase::Completed, &item);
            assert_eq!(completed.len(), 3);
            assert_eq!(completed[0], started[0]);
            assert!(
                matches!(&completed[1], AgentEvent::ToolResult { id, is_error: false, output: None, .. } if id == "img-1")
            );
            assert!(
                matches!(&completed[2], AgentEvent::GeneratedImage { id, path, name, .. } if id == "img-1:image" && path == "/codex/generated_images/picture.png" && name == "picture.png")
            );
            assert_eq!(map_item(Phase::Completed, &item), completed);
            let wire = serde_json::to_string(&completed).unwrap();
            assert!(wire.len() < 500);
            assert!(!wire.contains("INLINE_IMAGE_SENTINEL"));
            assert!(!wire.contains("private prompt"));
        }
    }

    #[test]
    fn image_generation_errors_resolve_the_chip_without_an_image() {
        for (extra, expected) in [
            (
                json!({"failure":{"type":"usageLimitExceeded"}, "savedPath":"/must/not/use.png"}),
                "Image generation usage limit exceeded",
            ),
            (
                json!({"failure":{"type":"futureFailure","message":"untrusted payload"}}),
                "Image generation failed",
            ),
            (
                json!({"status":"failed", "savedPath":"/must/not/use.png"}),
                "Image generation failed",
            ),
            (
                json!({"savedPath":null}),
                "Image generation completed without a saved file",
            ),
            (
                json!({"savedPath":"  "}),
                "Image generation completed without a saved file",
            ),
            (json!({}), "Image generation completed without a saved file"),
        ] {
            let mut item = json!({"type":"imageGeneration", "id":"i", "status":"completed", "result":"INLINE_SENTINEL"});
            item.as_object_mut()
                .unwrap()
                .extend(extra.as_object().unwrap().clone());
            let events = map_item(Phase::Completed, &item);
            assert_eq!(events.len(), 3);
            assert!(matches!(events[0], AgentEvent::ToolCall { .. }));
            assert!(matches!(
                events[1],
                AgentEvent::ToolResult { is_error: true, .. }
            ));
            assert_eq!(
                events[2],
                AgentEvent::Error {
                    message: expected.into()
                }
            );
        }
    }
}
