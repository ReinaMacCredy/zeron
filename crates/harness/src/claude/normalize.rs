//! Frame → [`AgentEvent`] normalization (init dedupe, subagent tagging, tool
//! decoding, error-code mapping).

use serde_json::Value;
use zeron_proto::{
    AgentEvent, AnchorRole, DoneStatus, HarnessId, TodoItem, TodoStatus, ToolCall, ToolDiff,
};

use super::wire::{ContentBlock, Frame};

/// Human-readable text for the CLI's assistant-level error codes. These arrive
/// as a terse `error` field on an `assistant` frame — usually with NO text
/// content and NOT as a `result` error — so a usage-limited or otherwise failed
/// turn looks like the agent simply never replied unless we surface it.
fn assistant_error_text(code: &str) -> String {
    match code {
        "authentication_failed" => "Authentication failed — sign in to Claude again.".into(),
        "oauth_org_not_allowed" => "This organization isn't allowed to use Claude here.".into(),
        "billing_error" => "Billing error — check your Claude plan or payment method.".into(),
        "rate_limit" => "Claude usage limit reached — try again after the limit resets.".into(),
        "overloaded" => "Claude is overloaded right now — try again shortly.".into(),
        "invalid_request" => "The request was rejected as invalid.".into(),
        "model_not_found" => "The selected model isn't available.".into(),
        "server_error" => "Claude had a server error — try again.".into(),
        "max_output_tokens" => "The reply hit the maximum output length.".into(),
        "unknown" => "Claude returned an unspecified error.".into(),
        other => format!("Claude error: {other}"),
    }
}

/// The CLI renders a failed API call as a `success` result whose `result`
/// text starts with `API Error:` — the only place the real reason (and any
/// request id) ever appears; the assistant frame carries just the code.
const API_ERROR_PREFIX: &str = "API Error:";

/// Wording for an `API Error:` result. A safeguards block gets a leading
/// line that names it and says what to do: the terse code alone left the
/// user retrying "resume" against the same flagged session four times
/// (2026-09-19), and the classifier fires on the model's streamed output,
/// so resuming the same session reproduces the block.
fn api_error_text(raw: &str) -> String {
    if !raw.contains("safeguards flagged") {
        return raw.to_owned();
    }
    let category = raw
        .split_once("Details: `[")
        .and_then(|(_, rest)| rest.split_once("]`"))
        .map(|(category, _)| format!(" (category: {category})"))
        .unwrap_or_default();
    format!(
        "Blocked by Claude's safeguards{category}. Resuming this thread replays the same \
         session and gets blocked again — start a new thread or switch model, then rephrase.\n\n{raw}"
    )
}

/// Which claude.ai usage window a `rate_limit_event` refers to.
fn rate_window_label(kind: &str) -> &'static str {
    match kind {
        "five_hour" => "5-hour",
        "seven_day" | "seven_day_overage_included" => "weekly",
        "seven_day_opus" => "weekly (Opus)",
        "seven_day_sonnet" => "weekly (Sonnet)",
        "overage" => "overage",
        _ => "usage",
    }
}

/// Fallback wording for a `result` error whose `errors` array is empty, so the
/// turn never ends with a blank (and therefore invisible) error.
fn result_error_text(subtype: &str) -> &'static str {
    match subtype {
        "error_max_turns" => "The run hit the maximum number of turns.",
        "error_max_budget_usd" => "The run hit its cost budget.",
        "error_max_structured_output_retries" => "The run exhausted its structured-output retries.",
        _ => "The run ended with an error.",
    }
}

/// The CLI seeds `result.errors` with internal `[ede_diagnostic]` breadcrumbs
/// for its error_during_execution telemetry ("turn aborted (…) stop_reason=…",
/// "result_type=… last_content_type=… stop_reason=…"). They're diagnostics
/// about the CLI's own turn accounting, not user-relevant errors — surfacing
/// them verbatim put raw `[ede_diagnostic] result_type=user …` boxes in the
/// transcript. They're debug-logged and dropped instead.
fn is_internal_diagnostic(message: &str) -> bool {
    message.contains("[ede_diagnostic]")
}

fn str_field(input: &Value, key: &str) -> String {
    input.get(key).and_then(Value::as_str).unwrap_or("").into()
}

fn opt_str_field(input: &Value, key: &str) -> Option<String> {
    input.get(key).and_then(Value::as_str).map(str::to_owned)
}

/// Decode a Claude `tool_use` block (name + input) into a typed [`ToolCall`].
pub(crate) fn decode_tool_use(name: &str, input: &Value) -> ToolCall {
    match name {
        "Bash" => ToolCall::Exec {
            command: str_field(input, "command"),
        },
        "Read" => ToolCall::ReadFile {
            path: str_field(input, "file_path"),
        },
        "Write" => ToolCall::WriteFile {
            path: str_field(input, "file_path"),
            content: opt_str_field(input, "content"),
        },
        "Edit" => ToolCall::EditFile {
            path: str_field(input, "file_path"),
            old_string: opt_str_field(input, "old_string"),
            new_string: opt_str_field(input, "new_string"),
        },
        "Grep" => ToolCall::Search {
            pattern: str_field(input, "pattern"),
            path: opt_str_field(input, "path"),
        },
        "Glob" => ToolCall::Glob {
            pattern: str_field(input, "pattern"),
        },
        "WebFetch" => ToolCall::WebFetch {
            url: str_field(input, "url"),
            prompt: opt_str_field(input, "prompt"),
        },
        "WebSearch" => ToolCall::WebSearch {
            query: str_field(input, "query"),
        },
        "TodoWrite" => ToolCall::Todo {
            items: input
                .get("todos")
                .and_then(Value::as_array)
                .map(|a| a.as_slice())
                .unwrap_or_default()
                .iter()
                .map(|t| {
                    TodoItem::new(
                        str_field(t, "content"),
                        TodoStatus::parse(t.get("status").and_then(Value::as_str).unwrap_or("")),
                    )
                })
                .collect(),
        },
        // The subagent spawn: name the chip — and the tab it opens — after
        // the TASK, not the bare tool ("Agent" alone says nothing in a tab
        // strip; the tool's `description` is where the work is named).
        "Agent" | "Task" => {
            let description = str_field(input, "description");
            ToolCall::Unknown {
                name: if description.is_empty() {
                    "Agent".into()
                } else {
                    format!("Agent: {description}")
                },
                input: (!input.is_null()).then(|| input.clone()),
            }
        }
        // MCP tools arrive as `mcp__<server>__<tool>`.
        _ => match name.strip_prefix("mcp__").and_then(|r| r.split_once("__")) {
            Some((server, tool)) => ToolCall::Mcp {
                server: server.into(),
                tool: tool.into(),
                input: (!input.is_null()).then(|| input.clone()),
            },
            None => ToolCall::Unknown {
                name: name.into(),
                input: (!input.is_null()).then(|| input.clone()),
            },
        },
    }
}

/// A Write/Edit result's file change as a [`ToolDiff`]. Verified live
/// (claude 2.1.278): Write carries `content` + `originalFile` (null on
/// create), Edit carries `originalFile` + `oldString`/`newString`
/// (`replaceAll` when every occurrence went); both keep `filePath`. A tool
/// error rides as a plain string, and Read's object has no `filePath` at the
/// top level — neither is a change.
fn file_diff(result: &Value) -> Option<ToolDiff> {
    let path = opt_str_field(result, "filePath").filter(|p| !p.is_empty())?;
    let original = opt_str_field(result, "originalFile");
    let new_text = match opt_str_field(result, "content") {
        Some(content) => content,
        None => {
            let old = opt_str_field(result, "oldString")?;
            let new = opt_str_field(result, "newString")?;
            let original = original.as_deref()?;
            if result.get("replaceAll").and_then(Value::as_bool) == Some(true) {
                original.replace(&old, &new)
            } else {
                original.replacen(&old, &new, 1)
            }
        }
    };
    Some(super::super::acp::normalize::capped_diff(
        path,
        original.as_deref(),
        &new_text,
    ))
}

fn tool_result_text(content: &Value) -> String {
    match content {
        Value::String(text) => text.clone(),
        Value::Array(blocks) => blocks
            .iter()
            .filter_map(|b| b.get("text").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}

fn background_launch(text: &str) -> Option<(String, String)> {
    let id = text
        .split("with ID: ")
        .nth(1)?
        .split(|c: char| c == '.' || c.is_whitespace())
        .next()?;
    let path = text
        .split("Output is being written to: ")
        .nth(1)?
        .split_whitespace()
        .next()?
        .trim_end_matches('.');
    let named = std::path::Path::new(path)
        .file_name()
        .and_then(|name| name.to_str())
        == Some(&format!("{id}.output"));
    (!id.is_empty() && named).then(|| (id.to_owned(), path.to_owned()))
}

fn new_message_id() -> String {
    uuid::Uuid::new_v4().to_string()
}

/// Wrap an event as subagent-attributed traffic.
fn tag(parent: &str, event: AgentEvent) -> AgentEvent {
    AgentEvent::Subagent {
        parent_tool_use_id: parent.to_owned(),
        event: Box::new(event),
    }
}

fn advisor_result(id: String, is_error: bool) -> AgentEvent {
    AgentEvent::ToolResult {
        id,
        is_error,
        output: None,
        diff: None,
    }
}

/// CLI-synthesized text that rides a user frame but is NOT conversation:
/// `<system-reminder>` context injections and the interruption marker the CLI
/// stamps into the transcript when a turn (or a subagent) is stopped.
///
/// On a TAGGED frame the distinction is load-bearing, not cosmetic. A tagged
/// user message means "the parent steered its subagent", which announces more
/// work and is therefore the one event allowed to resurrect a settled spawn
/// chip. The CLI emits `[Request interrupted by user]` on the child feed
/// immediately AFTER the subagent's `done{interrupted}` — read as a steer it
/// un-settled a chip that nothing would ever settle again, and the spinner ran
/// forever (2026-08-21: "orchestrator killed them but spinner doesn't stop").
/// It announces the opposite of more work.
///
/// Prefix-matched: the CLI ships at least two spellings of the marker
/// (`…by user]` and `…by user for tool use]`).
fn is_synthetic_user_text(text: &str) -> bool {
    let text = text.trim_start();
    text.starts_with("<system-reminder>") || text.starts_with("[Request interrupted")
}

/// The opening prompt an `Agent`/`Task` tool_use block hands its subagent;
/// `None` for any other tool or an empty prompt.
fn spawn_prompt(b: &ContentBlock) -> Option<&str> {
    matches!(b.name.as_str(), "Agent" | "Task")
        .then(|| b.input.get("prompt"))
        .flatten()
        .and_then(Value::as_str)
        .filter(|p| !p.trim().is_empty())
}

/// Per-run normalization state.
///
/// `saw_init` dedupes `system:init` — the CLI re-emits it every time the model
/// is re-invoked WITHIN one session (a background-subagent wake turn, a
/// scheduled wakeup), not just at start (live-verified against 2.1.228: a
/// background subagent finishing produces a second init with the SAME session
/// id, then a second `result`). Downstream, `SessionStarted` is the fold's run
/// boundary (it resets accumulated parts), so one run ⇒ one `SessionStarted`;
/// the wake turn's own frames flow through and the engine's parked-session
/// resume turns them into the done→Working→done wake.
pub(crate) struct Normalizer {
    saw_init: bool,
    last_model: Option<String>,
    /// Background-agent ids (`task_started.task_id`) → the spawning Agent
    /// tool_use id. `SendMessage` steers address the AGENT id; this map
    /// re-keys them onto the spawn chip's feed (the wire never echoes the
    /// steer on the child feed — live-verified 2.1.228).
    agent_tasks: std::collections::HashMap<String, String>,
    /// A resumed agent (`SendMessage` to a settled background agent) starts
    /// a second `task_started` for the SAME task id, keyed on the
    /// SendMessage call; its frames and completion then carry that id. It
    /// maps back to the original spawn so the resumed run settles its chip.
    spawn_aliases: std::collections::HashMap<String, String>,
    /// Only the Bash launch tool_result names a shell's output file.
    task_outputs: std::collections::HashMap<String, String>,
    live_tasks: Vec<zeron_proto::BackgroundTask>,
    nested_agent_tasks: std::collections::HashSet<String>,
    /// tool_use ids of Agent/Task spawn calls, recorded from their own
    /// assistant frames (plus `task_started`'s agent-task pairing). Gates
    /// `task_notification`: background SHELL tasks settle through the same
    /// subtype carrying their Bash call's id, and tagging that Done as
    /// subagent traffic stamped a spawn ref onto an ordinary Run chip —
    /// which then opened as an empty, never-created subagent doc (user
    /// report 2026-08-20).
    agent_spawn_tools: std::collections::HashSet<String>,
    /// Spawn tool_use id → the opening prompt this normalizer already seeded
    /// onto its feed. A FOREGROUND spawn (`run_in_background: false`) echoes
    /// that prompt back as a tagged user text frame (live-verified 2.1.278;
    /// a background spawn does not), which read as a steer and doubled the
    /// subagent's opening entry (user report 2026-09-19). Consumed on match.
    seeded_prompts: std::collections::HashMap<String, String>,
    computer_scripts: std::collections::HashSet<String>,
    /// The advisor call still awaiting its answer, per feed (`None` = the
    /// parent, `Some(spawn id)` = a subagent). The advisor is a SERVER tool:
    /// its call is a `server_tool_use` block and its answer an
    /// `advisor_tool_result` block, both on assistant frames — never a
    /// user-frame `tool_result`. A call the model abandons (a pending client
    /// tool's result landed first and it re-issued the call — live-observed
    /// 3 calls, 1 answer) gets no answer frame at all, so the previous call
    /// settles when the next opens and any survivor settles at turn end.
    pending_advisor: std::collections::HashMap<Option<String>, String>,
    /// Rotates at each assistant-frame close and at each steer; SessionStarted
    /// carries the first value so folds can attribute deltas from the start.
    assistant_message_id: String,
    /// Last session id seen (init or result) — used for synthetic Dones.
    pub session_id: Option<String>,
    /// The main channel's assistant-level error, held until the `result`
    /// frame: when that carries the CLI's `API Error:` text it supersedes the
    /// code mapping, otherwise the mapping surfaces then. Every observed
    /// assistant error was followed by a result frame (journals 2026-09-19);
    /// the run loop drains a survivor into its synthetic Done.
    pub pending_error: Option<String>,
}

impl Normalizer {
    pub fn new() -> Self {
        Self {
            saw_init: false,
            last_model: None,
            agent_tasks: std::collections::HashMap::new(),
            spawn_aliases: std::collections::HashMap::new(),
            task_outputs: std::collections::HashMap::new(),
            live_tasks: Vec::new(),
            nested_agent_tasks: std::collections::HashSet::new(),
            agent_spawn_tools: std::collections::HashSet::new(),
            seeded_prompts: std::collections::HashMap::new(),
            computer_scripts: std::collections::HashSet::new(),
            pending_advisor: std::collections::HashMap::new(),
            assistant_message_id: new_message_id(),
            session_id: None,
            pending_error: None,
        }
    }

    /// The CLI's task id for a subagent spawn: what `stop_task` addresses.
    pub fn agent_task_for_spawn(&self, spawn: &str) -> Option<String> {
        self.agent_tasks
            .iter()
            .find(|(_, tool)| tool.as_str() == spawn)
            .map(|(task, _)| task.clone())
    }

    fn spawn_of(&self, tool_use_id: &str) -> String {
        self.spawn_aliases
            .get(tool_use_id)
            .cloned()
            .unwrap_or_else(|| tool_use_id.to_owned())
    }

    fn advisor_events(&mut self, parent: Option<&str>, b: &ContentBlock) -> Vec<AgentEvent> {
        let feed = parent.map(str::to_owned);
        let wrap = |event: AgentEvent| match parent {
            Some(parent) => tag(parent, event),
            None => event,
        };
        match b.kind.as_str() {
            "server_tool_use" if b.name == "advisor" => {
                let mut out = Vec::new();
                if let Some(abandoned) = self.pending_advisor.insert(feed, b.id.clone()) {
                    out.push(wrap(advisor_result(abandoned, false)));
                }
                out.push(wrap(AgentEvent::ToolCall {
                    id: b.id.clone(),
                    call: ToolCall::Unknown {
                        name: zeron_proto::ADVISOR_TOOL_NAME.into(),
                        input: None,
                    },
                }));
                out
            }
            "advisor_tool_result" => {
                if self.pending_advisor.get(&feed) == Some(&b.tool_use_id) {
                    self.pending_advisor.remove(&feed);
                }
                // Only the redacted success shape has been observed; an
                // error-shaped answer is assumed to say so in its type.
                let is_error = b
                    .content
                    .get("type")
                    .and_then(Value::as_str)
                    .is_some_and(|kind| kind.contains("error"));
                vec![wrap(advisor_result(b.tool_use_id.clone(), is_error))]
            }
            _ => Vec::new(),
        }
    }

    fn settle_pending_advisors(&mut self) -> Vec<AgentEvent> {
        let mut pending: Vec<(Option<String>, String)> = self.pending_advisor.drain().collect();
        pending.sort();
        pending
            .into_iter()
            .map(|(feed, id)| match feed {
                Some(parent) => tag(&parent, advisor_result(id, false)),
                None => advisor_result(id, false),
            })
            .collect()
    }

    /// Restore only spawn identity from Claude's native history. The CLI
    /// does not replay task_started when --resume starts a new process.
    /// Session ids are global; search project directories rather than
    /// duplicating the CLI's cwd encoding (including long-path hashing).
    /// Long sessions' histories run to hundreds of MB: the scan runs off the
    /// runtime and only parses lines that can name a spawn or its agent.
    pub async fn for_resume(config_root: &std::path::Path, session_id: &str) -> Self {
        let config_root = config_root.to_owned();
        let session_id = session_id.to_owned();
        tokio::task::spawn_blocking(move || Self::restore_spawns(&config_root, &session_id))
            .await
            .unwrap_or_else(|_| Self::new())
    }

    fn restore_spawns(config_root: &std::path::Path, session_id: &str) -> Self {
        use std::io::BufRead as _;
        let mut norm = Self::new();
        if session_id.is_empty()
            || !session_id
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-')
        {
            return norm;
        }
        let Ok(projects) = std::fs::read_dir(config_root.join("projects")) else {
            return norm;
        };
        let Some(file) = projects.flatten().find_map(|project| {
            std::fs::File::open(project.path().join(format!("{session_id}.jsonl"))).ok()
        }) else {
            return norm;
        };
        let mut reader = std::io::BufReader::with_capacity(1 << 16, file);
        let mut line = Vec::new();
        loop {
            line.clear();
            match reader.read_until(b'\n', &mut line) {
                Ok(0) | Err(_) => break,
                Ok(_) => {}
            }
            // Torn lines and older/newer unknown shapes are harmless.
            let Ok(text) = std::str::from_utf8(&line) else {
                continue;
            };
            if !(text.contains("\"Agent\"")
                || text.contains("\"Task\"")
                || text.contains("agentId"))
            {
                continue;
            }
            let Ok(record) = serde_json::from_str::<Value>(text) else {
                continue;
            };
            // A subagent's own nested spawns are not chips on this chat.
            if record.get("isSidechain").and_then(Value::as_bool) == Some(true)
                || !record.get("parent_tool_use_id").is_none_or(Value::is_null)
            {
                continue;
            }
            let Some(blocks) = record.pointer("/message/content").and_then(Value::as_array) else {
                continue;
            };
            match record.get("type").and_then(Value::as_str) {
                Some("assistant") => {
                    for block in blocks {
                        if block.get("type").and_then(Value::as_str) == Some("tool_use")
                            && matches!(
                                block.get("name").and_then(Value::as_str),
                                Some("Agent" | "Task")
                            )
                            && let Some(id) = block
                                .get("id")
                                .and_then(Value::as_str)
                                .filter(|id| !id.is_empty())
                        {
                            norm.agent_spawn_tools.insert(id.to_owned());
                        }
                    }
                }
                Some("user") => {
                    if let Some(agent) = record
                        .pointer("/toolUseResult/agentId")
                        .and_then(Value::as_str)
                        .filter(|id| !id.is_empty())
                        && let Some(spawn) = blocks
                            .iter()
                            .filter(|b| {
                                b.get("type").and_then(Value::as_str) == Some("tool_result")
                            })
                            .filter_map(|b| b.get("tool_use_id").and_then(Value::as_str))
                            .find(|id| norm.agent_spawn_tools.contains(*id))
                    {
                        norm.agent_tasks
                            .entry(agent.to_owned())
                            .or_insert_with(|| spawn.to_owned());
                    }
                }
                _ => {}
            }
        }
        norm
    }

    /// Rotate the assistant message id for a steer boundary; returns
    /// (previous, next) for the `Steered` event.
    pub fn rotate_for_steer(&mut self) -> (String, String) {
        let prev = std::mem::replace(&mut self.assistant_message_id, new_message_id());
        (prev, self.assistant_message_id.clone())
    }

    /// Normalize one stdout frame into 0+ unified events. `interrupted` folds
    /// a post-interrupt `result` into `Done { status: Interrupted }`.
    pub fn normalize(&mut self, frame: Frame, interrupted: bool) -> Vec<AgentEvent> {
        match frame {
            Frame::System(f) => {
                // A background subagent's completion arrives as an UNTAGGED
                // `task_notification` carrying the spawning tool's id — the
                // wire's only terminal signal for it (live-verified 2.1.228:
                // no tagged frame follows; the subagent's stream just stops).
                // Surface it as the subagent's tagged Done so the chip flips
                // done/failed and the transcript freezes.
                if matches!(f.subtype.as_str(), "task_notification" | "task_progress") {
                    // A resumed task's notification may carry SendMessage's
                    // id, or omit the tool id entirely. Its task id is stable.
                    let parent = f
                        .task_id
                        .as_deref()
                        .and_then(|task| self.agent_tasks.get(task).map(String::as_str))
                        .or_else(|| f.tool_use_id.as_deref().filter(|t| !t.is_empty()));
                    let Some(parent) = parent else {
                        return Vec::new();
                    };
                    let parent = &self.spawn_of(parent);
                    // Only a KNOWN spawn settles as a subagent. Background
                    // SHELL tasks (`Bash` with `run_in_background`) settle
                    // through this same subtype carrying the Bash call's id —
                    // tagging that Done would bind a subagent ref onto an
                    // ordinary Run chip. Spawn ids are learned from live
                    // calls/task starts or restored from the native history.
                    if !self.agent_spawn_tools.contains(parent) {
                        return Vec::new();
                    }
                    // `total_tokens` is the latest message's context size, not a cumulative sum.
                    let mut out: Vec<AgentEvent> = f
                        .usage
                        .as_ref()
                        .and_then(|usage| usage.get("total_tokens")?.as_u64())
                        .map(|tokens| {
                            tag(
                                parent,
                                AgentEvent::ContextUsage {
                                    tokens: Some(tokens),
                                    window: None,
                                },
                            )
                        })
                        .into_iter()
                        .collect();
                    if f.subtype == "task_progress" {
                        return out;
                    }
                    let status = match f.status.as_deref().unwrap_or("") {
                        "completed" | "complete" | "succeeded" | "success" => DoneStatus::Completed,
                        "failed" | "errored" | "error" => DoneStatus::Errored,
                        "killed" | "cancelled" | "canceled" | "stopped" | "interrupted" => {
                            DoneStatus::Interrupted
                        }
                        // Non-terminal notification shapes: nothing to close.
                        _ => return out,
                    };
                    out.push(tag(
                        parent,
                        AgentEvent::Done {
                            status,
                            result: None,
                            error: None,
                            session_id: None,
                        },
                    ));
                    return out;
                }
                // An AGENT task starting (subagent_type present — subagent-
                // owned shell tasks carry the same subtype without it):
                // record agentId → spawn id for SendMessage steer re-keying.
                if f.subtype == "task_started" {
                    if f.subagent_type.is_some()
                        && let (Some(task), Some(tool)) = (
                            f.task_id.as_deref().filter(|t| !t.is_empty()),
                            f.tool_use_id.as_deref().filter(|t| !t.is_empty()),
                        )
                    {
                        match self.agent_tasks.get(task) {
                            Some(spawn) if spawn != tool => {
                                self.spawn_aliases.insert(tool.to_owned(), spawn.clone());
                            }
                            _ => {
                                self.agent_tasks.insert(task.to_owned(), tool.to_owned());
                                self.agent_spawn_tools.insert(tool.to_owned());
                            }
                        }
                    }
                    // The CLI lists a task before its task_started names the depth.
                    if let Some(task) = f.task_id.filter(|_| f.spawn_depth.is_some_and(|d| d > 1)) {
                        let listed = self.live_tasks.len();
                        self.live_tasks.retain(|t| t.id != task);
                        self.nested_agent_tasks.insert(task);
                        if self.live_tasks.len() != listed {
                            return vec![AgentEvent::BackgroundTasks {
                                tasks: self.live_tasks.clone(),
                            }];
                        }
                    }
                    return Vec::new();
                }
                // The CLI's own compaction boundary. Must precede the init
                // gate below, which swallows every other system subtype.
                if f.subtype == "compact_boundary" {
                    let meta = f.compact_metadata.unwrap_or_default();
                    return vec![AgentEvent::Compacted {
                        pre_tokens: meta.pre_tokens,
                        post_tokens: meta.post_tokens,
                    }];
                }
                if f.subtype == "background_tasks_changed" {
                    let tasks = f
                        .tasks
                        .unwrap_or_default()
                        .into_iter()
                        .filter(|t| {
                            !t.task_id.is_empty() && !self.nested_agent_tasks.contains(&t.task_id)
                        })
                        .map(|t| zeron_proto::BackgroundTask {
                            output_path: self.task_outputs.get(&t.task_id).cloned(),
                            kind: match t.task_type.as_str() {
                                "local_bash" => zeron_proto::BackgroundTaskKind::Shell,
                                "local_agent" | "remote_agent" => {
                                    zeron_proto::BackgroundTaskKind::Agent
                                }
                                _ => zeron_proto::BackgroundTaskKind::Other,
                            },
                            label: t.description,
                            id: t.task_id,
                            started_at: None,
                        })
                        .collect::<Vec<_>>();
                    self.live_tasks = tasks.clone();
                    return vec![AgentEvent::BackgroundTasks { tasks }];
                }
                if f.subtype == "status" {
                    return vec![AgentEvent::Compacting {
                        active: f.status.as_deref() == Some("compacting"),
                    }];
                }
                if f.subtype != "init" || self.saw_init {
                    return Vec::new();
                }
                self.saw_init = true;
                self.session_id = Some(f.session_id.clone());
                vec![AgentEvent::SessionStarted {
                    harness: HarnessId::ClaudeCode,
                    model: f.model,
                    tools: f.tools,
                    cwd: f.cwd,
                    session_id: f.session_id,
                    assistant_message_id: self.assistant_message_id.clone(),
                }]
            }

            // Frames with `parent_tool_use_id` set belong to a SUBAGENT's
            // nested transcript: a background subagent runs concurrently with
            // the parent's own stream, so folding them into the parent feed
            // would split a contiguous text block around phantom tool calls
            // (and a subagent's message boundaries must never rotate the
            // parent's assistant message id). They are wrapped in
            // `AgentEvent::Subagent` instead — the engine routes them to the
            // subagent's own doc.
            Frame::StreamEvent(f) => {
                if f.event.kind != "content_block_delta" {
                    return Vec::new();
                }
                if let Some(parent) = f.parent_tool_use_id.as_deref().map(|p| self.spawn_of(p)) {
                    let parent = &parent;
                    return match f.event.delta.kind.as_str() {
                        "text_delta" => vec![tag(
                            parent,
                            AgentEvent::TextDelta {
                                text: f.event.delta.text,
                            },
                        )],
                        "thinking_delta" if !f.event.delta.thinking.is_empty() => vec![tag(
                            parent,
                            AgentEvent::ReasoningDelta {
                                text: f.event.delta.thinking,
                            },
                        )],
                        // Subagent liveness heartbeats have no consumer; the
                        // parent turn is already done (eager-done policy).
                        _ => Vec::new(),
                    };
                }
                match f.event.delta.kind.as_str() {
                    "text_delta" => vec![AgentEvent::TextDelta {
                        text: f.event.delta.text,
                    }],
                    "thinking_delta" => vec![AgentEvent::ReasoningDelta {
                        text: f.event.delta.thinking,
                    }],
                    // A big tool input (a 90-line Write) streams as a long run
                    // of input_json_delta frames with nothing else — minutes of
                    // apparent silence that reads as a stalled run. Surface
                    // them as empty reasoning deltas: the engine treats those
                    // as pure liveness heartbeats (never journaled/rendered).
                    "input_json_delta" => vec![AgentEvent::ReasoningDelta {
                        text: String::new(),
                    }],
                    _ => Vec::new(),
                }
            }

            Frame::Assistant(f) => {
                if let Some(parent) = f.parent_tool_use_id.as_deref().map(|p| self.spawn_of(p)) {
                    let parent = &parent;
                    // Subagent content, attributed. The 2.1.x wire streams NO
                    // tagged partial deltas (live-verified): a subagent's text
                    // arrives only as full text blocks on its tagged
                    // assistant frames — emit them, in block order with the
                    // tool calls, or subagent transcripts are tool-chips-only.
                    let mut out: Vec<AgentEvent> = f
                        .message
                        .blocks()
                        .filter_map(|b: ContentBlock| match b.kind.as_str() {
                            "text" if !b.text.is_empty() => Some(tag(
                                parent,
                                AgentEvent::TextDelta {
                                    text: format!("{}\n\n", b.text.trim_end()),
                                },
                            )),
                            "tool_use" => Some(tag(
                                parent,
                                AgentEvent::ToolCall {
                                    id: b.id.clone(),
                                    call: decode_tool_use(&b.name, &b.input),
                                },
                            )),
                            _ => None,
                        })
                        .collect();
                    for b in f.message.blocks() {
                        out.extend(self.advisor_events(Some(parent), &b));
                    }
                    if let Some(code) = &f.error {
                        out.push(tag(
                            parent,
                            AgentEvent::Error {
                                message: assistant_error_text(code),
                            },
                        ));
                    }
                    return out;
                }
                // Record spawn tool ids up front: `task_notification` keys on
                // them; a task may finish before a `task_started` arrives.
                for b in f.message.blocks() {
                    if b.kind == "tool_use" && b.name == "mcp__zeron__computer" {
                        self.computer_scripts.insert(b.id.clone());
                    }
                    if b.kind == "tool_use" && matches!(b.name.as_str(), "Agent" | "Task") {
                        self.agent_spawn_tools.insert(b.id.clone());
                        if let Some(prompt) = spawn_prompt(&b) {
                            self.seeded_prompts.insert(b.id.clone(), prompt.to_owned());
                        }
                    }
                }
                // Every main-channel assistant message is a cut point; the
                // engine keeps the latest one per reply entry.
                let anchor =
                    f.uuid
                        .as_deref()
                        .filter(|u| !u.is_empty())
                        .map(|uuid| AgentEvent::Anchor {
                            role: AnchorRole::Assistant,
                            id: uuid.to_owned(),
                            entry_id: None,
                        });
                let mut out: Vec<AgentEvent> = anchor.into_iter().collect();
                out.extend(
                    f.message
                        .blocks()
                        .filter(|b: &ContentBlock| b.kind == "tool_use")
                        .flat_map(|b| {
                            let call = AgentEvent::ToolCall {
                                id: b.id.clone(),
                                call: decode_tool_use(&b.name, &b.input),
                            };
                            // A spawn's `prompt` is the subagent's opening user
                            // message — a background spawn never echoes it on
                            // the child feed (child user frames carry tool
                            // results and steers only), so seed it here and
                            // the subagent transcript starts the way every
                            // chat does. A foreground spawn's echo is dropped
                            // against `seeded_prompts` in the user-frame arm.
                            let opening = spawn_prompt(&b).map(|prompt| {
                                tag(
                                    &b.id,
                                    AgentEvent::UserMessage {
                                        text: prompt.to_owned(),
                                    },
                                )
                            });
                            // A SendMessage steer never echoes on the child feed
                            // (live-verified) — surface it from the parent's own
                            // call, re-keyed onto the spawn it addresses.
                            let steer = (b.name == "SendMessage")
                                .then(|| {
                                    let to = ["to", "recipient"]
                                        .iter()
                                        .find_map(|k| b.input.get(*k))
                                        .and_then(Value::as_str)?;
                                    let spawn = self.agent_tasks.get(to)?;
                                    let text = ["message", "content"]
                                        .iter()
                                        .find_map(|k| b.input.get(*k))
                                        .and_then(Value::as_str)
                                        .filter(|m| !m.trim().is_empty())?;
                                    Some(tag(
                                        spawn,
                                        AgentEvent::UserMessage {
                                            text: text.to_owned(),
                                        },
                                    ))
                                })
                                .flatten();
                            std::iter::once(call).chain(opening).chain(steer)
                        }),
                );
                for b in f.message.blocks() {
                    out.extend(self.advisor_events(None, &b));
                }
                self.last_model = f.message.model.clone().or(self.last_model.take());
                if let Some(usage) = &f.message.usage {
                    let fields = [
                        "input_tokens",
                        "cache_read_input_tokens",
                        "cache_creation_input_tokens",
                    ];
                    if fields
                        .iter()
                        .any(|key| usage.get(*key).and_then(Value::as_u64).is_some())
                    {
                        let tokens = fields
                            .iter()
                            .filter_map(|key| usage.get(*key).and_then(Value::as_u64))
                            .fold(0u64, u64::saturating_add);
                        out.push(AgentEvent::ContextUsage {
                            tokens: Some(tokens),
                            window: None,
                        });
                    }
                }
                // A failed turn (usage limit, billing, auth, overloaded, …)
                // carries a terse `error` code here — often with empty content
                // and no `result` error — so it must surface; held for the
                // result frame, which may carry the fuller `API Error:` text.
                if let Some(code) = &f.error {
                    self.pending_error = Some(assistant_error_text(code));
                }
                // The enclosing assistant frame closes the streamed message
                // item; rotate so post-boundary deltas get a fresh id.
                let (prev, _next) = self.rotate_for_steer();
                out.push(AgentEvent::AssistantMessageCompleted {
                    assistant_message_id: prev,
                });
                out
            }

            Frame::User(f) => {
                // One structured result per frame: it names the frame's one
                // tool_result block, so a multi-result frame gets none.
                let results: Vec<ContentBlock> = f
                    .message
                    .blocks()
                    .filter(|b: &ContentBlock| b.kind == "tool_result")
                    .collect();
                let diff = (results.len() == 1)
                    .then(|| f.tool_use_result.as_ref().and_then(file_diff))
                    .flatten();
                let mut learned_output = false;
                if f.parent_tool_use_id.is_none() {
                    for (task, path) in results
                        .iter()
                        .filter_map(|b| background_launch(&tool_result_text(&b.content)))
                    {
                        if let Some(live) = self.live_tasks.iter_mut().find(|t| t.id == task) {
                            live.output_path = Some(path.clone());
                            learned_output = true;
                        }
                        self.task_outputs.insert(task, path);
                    }
                }
                let relisted = learned_output.then(|| AgentEvent::BackgroundTasks {
                    tasks: self.live_tasks.clone(),
                });
                let scripts: Vec<Option<String>> = results
                    .iter()
                    .map(|b| {
                        self.computer_scripts
                            .remove(&b.tool_use_id)
                            .then(|| tool_result_text(&b.content))
                            .and_then(|text| {
                                text.lines()
                                    .find(|line| !line.trim().is_empty())
                                    .map(str::to_owned)
                            })
                    })
                    .collect();
                let tool_results =
                    results
                        .into_iter()
                        .zip(scripts)
                        .map(|(b, output)| AgentEvent::ToolResult {
                            id: b.tool_use_id.clone(),
                            is_error: b.is_error.unwrap_or(false),
                            output,
                            diff: diff.clone(),
                        });
                if let Some(parent) = f.parent_tool_use_id.as_deref().map(|p| self.spawn_of(p)) {
                    let parent = &parent;
                    // A subagent's tool results echo on the main channel too;
                    // they belong to its transcript, attributed like its calls.
                    let mut out: Vec<AgentEvent> =
                        tool_results.map(|event| tag(parent, event)).collect();
                    // A tagged user frame's TEXT blocks are the parent
                    // steering its subagent (SendMessage-style follow-ups —
                    // tool results ride their own blocks, filtered above).
                    // Synthetic harness injections are not conversation, and
                    // must not read as a steer — see [`is_synthetic_user_text`].
                    // A foreground spawn's echo of the prompt already seeded
                    // from its tool_use block is not a steer either.
                    for b in f.message.blocks() {
                        if b.kind != "text"
                            || b.text.trim().is_empty()
                            || is_synthetic_user_text(&b.text)
                        {
                            continue;
                        }
                        if self
                            .seeded_prompts
                            .get(parent)
                            .is_some_and(|p| *p == b.text)
                        {
                            self.seeded_prompts.remove(parent);
                            continue;
                        }
                        out.push(tag(parent, AgentEvent::UserMessage { text: b.text }));
                    }
                    return out;
                }
                // A replayed prompt (`--replay-user-messages`) names the
                // checkpoint before it; tool-result frames carry no text and
                // no anchor of their own.
                let anchor = f
                    .uuid
                    .as_deref()
                    .filter(|u| !u.is_empty())
                    .filter(|_| {
                        // The driver sends prompts as a plain string, and
                        // the echo keeps that shape; steers and SDK callers
                        // use text blocks.
                        let plain = f.message.content.as_str().map(str::trim);
                        plain.is_some_and(|t| !t.is_empty() && !is_synthetic_user_text(t))
                            || f.message.blocks().any(|b: ContentBlock| {
                                b.kind == "text"
                                    && !b.text.trim().is_empty()
                                    && !is_synthetic_user_text(&b.text)
                            })
                    })
                    .map(|uuid| AgentEvent::Anchor {
                        role: AnchorRole::User,
                        id: uuid.to_owned(),
                        entry_id: None,
                    });
                anchor
                    .into_iter()
                    .chain(tool_results)
                    .chain(relisted)
                    .collect()
            }

            // A claude.ai plan window was hit. A hard `rejected` blocks the
            // turn — make it visible; allowed/allowed_warning stay quiet, and
            // so does a reject the account's overage carries past.
            Frame::RateLimit(f) => {
                if !f.rate_limit_info.blocks() {
                    return Vec::new();
                }
                let window =
                    rate_window_label(f.rate_limit_info.rate_limit_type.as_deref().unwrap_or(""));
                vec![AgentEvent::Error {
                    message: format!(
                        "Claude {window} limit reached — the turn was blocked. Try again after it resets."
                    ),
                }]
            }

            Frame::Result(f) => {
                let model_usage = self
                    .last_model
                    .as_ref()
                    .and_then(|model| {
                        f.model_usage.get(model).or_else(|| {
                            f.model_usage.values().find(|entry| {
                                entry.get("canonicalModel").and_then(Value::as_str)
                                    == Some(model.as_str())
                            })
                        })
                    })
                    .or_else(|| {
                        (f.model_usage.len() == 1)
                            .then(|| f.model_usage.values().next())
                            .flatten()
                    });
                let window = model_usage
                    .and_then(|entry| entry.get("contextWindow"))
                    .and_then(Value::as_u64)
                    .filter(|n| *n > 0);
                if let Some(id) = &f.session_id {
                    self.session_id = Some(id.clone());
                }
                let cost = f
                    .total_cost_usd
                    .zip(self.session_id.clone())
                    .map(|(total_usd, session_id)| AgentEvent::Cost {
                        session_id,
                        total_usd,
                    });
                let usage = AgentEvent::Usage {
                    input_tokens: f.usage.input_tokens,
                    output_tokens: f.usage.output_tokens,
                };
                let cache = (f.usage.cache_read_input_tokens + f.usage.cache_creation_input_tokens
                    > 0)
                    .then(|| AgentEvent::PromptCache {
                        read_tokens: f.usage.cache_read_input_tokens,
                        write_tokens: f.usage.cache_creation_input_tokens,
                        ttl_secs: if f.usage.cache_creation.ephemeral_1h_input_tokens > 0 {
                            Some(3600)
                        } else if f.usage.cache_creation.ephemeral_5m_input_tokens > 0 {
                            Some(300)
                        } else {
                            None
                        },
                    });
                let api_error = f
                    .result
                    .as_deref()
                    .filter(|_| f.subtype == "success")
                    .filter(|text| text.starts_with(API_ERROR_PREFIX))
                    .map(api_error_text);
                let pending_error = self.pending_error.take().filter(|_| api_error.is_none());
                let done = if let Some(text) = api_error {
                    AgentEvent::Done {
                        status: if interrupted {
                            DoneStatus::Interrupted
                        } else {
                            DoneStatus::Errored
                        },
                        result: None,
                        error: Some(text),
                        session_id: f.session_id,
                    }
                } else if f.subtype == "success" {
                    AgentEvent::Done {
                        status: if interrupted {
                            DoneStatus::Interrupted
                        } else {
                            DoneStatus::Completed
                        },
                        result: f.result,
                        error: None,
                        session_id: f.session_id,
                    }
                } else {
                    // Split the CLI's internal `[ede_diagnostic]` breadcrumbs
                    // off the real errors: diagnostics are debug-logged, never
                    // surfaced as transcript error parts.
                    let (diagnostics, errors): (Vec<String>, Vec<String>) = f
                        .errors
                        .iter()
                        .map(|e| match e {
                            Value::String(s) => s.clone(),
                            other => other.to_string(),
                        })
                        .partition(|m| is_internal_diagnostic(m));
                    for diagnostic in &diagnostics {
                        tracing::debug!(
                            target: "zeron_harness::claude",
                            "internal CLI diagnostic (not surfaced): {diagnostic}"
                        );
                    }
                    let error = if !errors.is_empty() {
                        // Real user-relevant errors — surface verbatim.
                        Some(errors.join("; "))
                    } else {
                        match f.subtype.as_str() {
                            // Known run-failure subtypes stay visible with
                            // their mapped human wording (never blank — a
                            // blank error folds to no part and the failed
                            // turn reads as a silent non-reply).
                            "error_max_turns"
                            | "error_max_budget_usd"
                            | "error_max_structured_output_retries" => {
                                Some(result_error_text(&f.subtype).to_owned())
                            }
                            // Diagnostic-only ends (the CLI's turn-accounting
                            // telemetry, typically `error_during_execution`
                            // after an abort): nothing user-relevant to show.
                            _ if !diagnostics.is_empty() => None,
                            _ => Some(result_error_text(&f.subtype).to_owned()),
                        }
                    };
                    AgentEvent::Done {
                        status: if interrupted {
                            DoneStatus::Interrupted
                        } else {
                            DoneStatus::Errored
                        },
                        result: None,
                        error,
                        session_id: f.session_id,
                    }
                };
                let mut out = self.settle_pending_advisors();
                if let Some(message) = pending_error {
                    out.push(AgentEvent::Error { message });
                }
                if let Some(window) = window {
                    out.push(AgentEvent::ContextUsage {
                        tokens: None,
                        window: Some(window),
                    });
                }
                out.extend(cache);
                out.extend(cost);
                out.extend([usage, done]);
                out
            }

            // Control frames are handled by the run loop, not normalized.
            Frame::ControlRequest(_)
            | Frame::ControlResponse(_)
            | Frame::CommandLifecycle(_)
            | Frame::Other => Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn an_edit_past_the_diff_cap_keeps_the_changed_lines() {
        let filler: String = (0..4000)
            .map(|n| format!("line {n:05} of filler\n"))
            .collect();
        assert!(filler.len() > 64 * 1024);
        let original = format!("{filler}beta\ntail one\ntail two\n{filler}");
        let diff = file_diff(&json!({
            "filePath": "/w/big.rs",
            "originalFile": original,
            "oldString": "beta",
            "newString": "delta",
            "replaceAll": false,
        }))
        .unwrap();
        assert_eq!(diff.line_offset, Some(3997));
        assert_eq!(
            diff.old_text.as_deref(),
            Some(
                "line 03997 of filler\nline 03998 of filler\nline 03999 of filler\nbeta\ntail one\ntail two\nline 00000 of filler\n"
            )
        );
        assert_eq!(
            diff.new_text,
            "line 03997 of filler\nline 03998 of filler\nline 03999 of filler\ndelta\ntail one\ntail two\nline 00000 of filler\n"
        );
    }

    #[test]
    fn decodes_typed_tools() {
        assert_eq!(
            decode_tool_use("Bash", &json!({"command": "ls -la"})),
            ToolCall::Exec {
                command: "ls -la".into()
            }
        );
        assert_eq!(
            decode_tool_use(
                "Edit",
                &json!({"file_path": "/a", "old_string": "x", "new_string": "y"})
            ),
            ToolCall::EditFile {
                path: "/a".into(),
                old_string: Some("x".into()),
                new_string: Some("y".into())
            }
        );
        assert_eq!(
            decode_tool_use(
                "TodoWrite",
                &json!({"todos": [{"content": "t", "status": "completed"}]})
            ),
            ToolCall::Todo {
                items: vec![TodoItem::new("t", TodoStatus::Completed)]
            }
        );
        // Claude's in-progress state survives; unknown/missing reads as pending.
        assert_eq!(
            decode_tool_use(
                "TodoWrite",
                &json!({"todos": [
                    {"content": "a", "status": "completed"},
                    {"content": "b", "status": "in_progress", "activeForm": "Doing b"},
                    {"content": "c", "status": "pending"},
                    {"content": "d"},
                ]})
            ),
            ToolCall::Todo {
                items: vec![
                    TodoItem::new("a", TodoStatus::Completed),
                    TodoItem::new("b", TodoStatus::InProgress),
                    TodoItem::new("c", TodoStatus::Pending),
                    TodoItem::new("d", TodoStatus::Pending),
                ]
            }
        );
        assert_eq!(
            decode_tool_use("mcp__linear__search", &json!({"q": "bug"})),
            ToolCall::Mcp {
                server: "linear".into(),
                tool: "search".into(),
                input: Some(json!({"q": "bug"}))
            }
        );
        assert!(matches!(
            decode_tool_use("Mystery", &json!({})),
            ToolCall::Unknown { .. }
        ));
    }

    fn normalize_one(raw: &str) -> Vec<AgentEvent> {
        let frame = crate::claude::wire::parse_frame(raw).expect("frame parses");
        Normalizer::new().normalize(frame, false)
    }

    #[test]
    fn a_rejected_window_carried_by_overage_is_not_an_error() {
        let blocked = normalize_one(
            r#"{"type":"rate_limit_event","rate_limit_info":{"status":"rejected","rateLimitType":"five_hour","overageStatus":"rejected"}}"#,
        );
        assert!(matches!(blocked.as_slice(), [AgentEvent::Error { .. }]));
        for overage in [
            r#""overageStatus":"allowed""#,
            r#""overageStatus":"allowed_warning""#,
            r#""isUsingOverage":true"#,
        ] {
            let raw = format!(
                r#"{{"type":"rate_limit_event","rate_limit_info":{{"status":"rejected","rateLimitType":"five_hour",{overage}}}}}"#
            );
            assert!(normalize_one(&raw).is_empty(), "{overage}");
        }
    }

    fn result_done(raw: &str) -> AgentEvent {
        let events = normalize_one(raw);
        assert_eq!(events.len(), 2, "usage + done");
        events.into_iter().nth(1).expect("done event")
    }

    /// Sanitized live frames from claude 2.1.273 running `/compact` under
    /// `--print --input-format stream-json --output-format stream-json
    /// --verbose --resume <id>`, in wire order.
    #[test]
    fn replayed_prompts_and_assistant_frames_carry_anchors() {
        let user = normalize_one(
            r#"{"type":"user","uuid":"u-1","message":{"role":"user","content":[{"type":"text","text":"fix the bug"}]}}"#,
        );
        assert_eq!(
            user,
            vec![AgentEvent::Anchor {
                role: AnchorRole::User,
                id: "u-1".into(),
                entry_id: None,
            }]
        );
        // The driver's own prompt shape: a plain string, echoed back as is.
        let plain = normalize_one(
            r#"{"type":"user","uuid":"u-3","message":{"role":"user","content":"fix the bug"}}"#,
        );
        assert_eq!(
            plain,
            vec![AgentEvent::Anchor {
                role: AnchorRole::User,
                id: "u-3".into(),
                entry_id: None,
            }]
        );
        // Tool-result frames name no checkpoint, uuid or not.
        let results = normalize_one(
            r#"{"type":"user","uuid":"u-2","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"t1","content":"ok"}]}}"#,
        );
        assert!(
            !results
                .iter()
                .any(|e| matches!(e, AgentEvent::Anchor { .. })),
            "{results:?}"
        );
        let assistant = normalize_one(
            r#"{"type":"assistant","uuid":"a-1","message":{"role":"assistant","content":[{"type":"text","text":"done"}]}}"#,
        );
        assert_eq!(
            assistant.first(),
            Some(&AgentEvent::Anchor {
                role: AnchorRole::Assistant,
                id: "a-1".into(),
                entry_id: None,
            })
        );
        // Subagent traffic is never a cut point of the parent session.
        let child = normalize_one(
            r#"{"type":"assistant","uuid":"a-2","parent_tool_use_id":"toolu_sub","message":{"role":"assistant","content":[{"type":"text","text":"child"}]}}"#,
        );
        assert!(
            !child.iter().any(|e| matches!(e, AgentEvent::Anchor { .. })),
            "{child:?}"
        );
    }

    /// Live frames from claude 2.1.278 (`--output-format stream-json
    /// --verbose`), paths shortened: a Write create, a Write update, an Edit.
    #[test]
    fn write_and_edit_results_carry_the_file_diff() {
        let create = normalize_one(
            r#"{"type":"user","message":{"role":"user","content":[{"tool_use_id":"t1","type":"tool_result","content":"File created successfully at: /w/hello.txt"}]},"parent_tool_use_id":null,"tool_use_result":{"type":"create","filePath":"/w/hello.txt","content":"hi\nthere","structuredPatch":[],"originalFile":null,"userModified":false}}"#,
        );
        assert_eq!(
            create,
            vec![AgentEvent::ToolResult {
                id: "t1".into(),
                is_error: false,
                output: None,
                diff: Some(ToolDiff {
                    path: "/w/hello.txt".into(),
                    old_text: None,
                    new_text: "hi\nthere".into(),
                    line_offset: None,
                }),
            }]
        );
        let update = normalize_one(
            r#"{"type":"user","message":{"role":"user","content":[{"tool_use_id":"t2","type":"tool_result","content":"The file /w/big.txt has been updated successfully."}]},"parent_tool_use_id":null,"tool_use_result":{"type":"update","filePath":"/w/big.txt","content":"one\nTWO\nthree\nfour\n","structuredPatch":[{"oldStart":1,"oldLines":3,"newStart":1,"newLines":4,"lines":[" one","-two","+TWO"," three","+four"]}],"originalFile":"one\ntwo\nthree\n","userModified":false}}"#,
        );
        assert_eq!(
            update,
            vec![AgentEvent::ToolResult {
                id: "t2".into(),
                is_error: false,
                output: None,
                diff: Some(ToolDiff {
                    path: "/w/big.txt".into(),
                    old_text: Some("one\ntwo\nthree\n".into()),
                    new_text: "one\nTWO\nthree\nfour\n".into(),
                    line_offset: None,
                }),
            }]
        );
        // Edit names the replaced span, not the new file: rebuilt from the
        // original, one occurrence unless `replaceAll`.
        let edit = normalize_one(
            r#"{"type":"user","message":{"role":"user","content":[{"tool_use_id":"t3","type":"tool_result","content":"The file /w/notes.txt has been updated successfully."}]},"parent_tool_use_id":null,"tool_use_result":{"filePath":"/w/notes.txt","oldString":"beta","newString":"delta","originalFile":"alpha\nbeta\ngamma\nbeta\n","structuredPatch":[{"oldStart":1,"oldLines":3,"newStart":1,"newLines":3,"lines":[" alpha","-beta","+delta"," gamma"]}],"userModified":false,"replaceAll":false}}"#,
        );
        assert_eq!(
            edit,
            vec![AgentEvent::ToolResult {
                id: "t3".into(),
                is_error: false,
                output: None,
                diff: Some(ToolDiff {
                    path: "/w/notes.txt".into(),
                    old_text: Some("alpha\nbeta\ngamma\nbeta\n".into()),
                    new_text: "alpha\ndelta\ngamma\nbeta\n".into(),
                    line_offset: None,
                }),
            }]
        );
        let all = normalize_one(
            r#"{"type":"user","message":{"role":"user","content":[{"tool_use_id":"t4","type":"tool_result","content":"ok"}]},"tool_use_result":{"filePath":"/w/notes.txt","oldString":"beta","newString":"delta","originalFile":"beta\nbeta\n","structuredPatch":[],"userModified":false,"replaceAll":true}}"#,
        );
        let Some(AgentEvent::ToolResult {
            diff: Some(diff), ..
        }) = all.first()
        else {
            panic!("{all:?}");
        };
        assert_eq!(diff.new_text, "delta\ndelta\n");
        // A subagent's write is tagged like its call, diff intact.
        let child = normalize_one(
            r#"{"type":"user","parent_tool_use_id":"toolu_sub","message":{"role":"user","content":[{"tool_use_id":"t5","type":"tool_result","content":"ok"}]},"tool_use_result":{"type":"create","filePath":"/w/c.txt","content":"x","structuredPatch":[],"originalFile":null}}"#,
        );
        assert!(
            matches!(
                child.first(),
                Some(AgentEvent::Subagent { parent_tool_use_id, event })
                    if parent_tool_use_id == "toolu_sub"
                        && matches!(**event, AgentEvent::ToolResult { diff: Some(_), .. })
            ),
            "{child:?}"
        );
        // Not a file change: a tool error (string), a Read (nested `file`),
        // a Bash result — each resolves without a diff.
        for raw in [
            r#"{"type":"user","message":{"role":"user","content":[{"type":"tool_result","content":"<tool_use_error>File has not been read yet.</tool_use_error>","is_error":true,"tool_use_id":"t6"}]},"tool_use_result":"Error: File has not been read yet."}"#,
            r#"{"type":"user","message":{"role":"user","content":[{"tool_use_id":"t7","type":"tool_result","content":"1\tone\n"}]},"tool_use_result":{"type":"text","file":{"filePath":"/w/big.txt","content":"one\n","numLines":1,"startLine":1,"totalLines":1}}}"#,
            r#"{"type":"user","message":{"role":"user","content":[{"tool_use_id":"t8","type":"tool_result","content":"ok"}]},"tool_use_result":{"stdout":"ok","stderr":"","interrupted":false}}"#,
        ] {
            let events = normalize_one(raw);
            assert!(
                matches!(
                    events.as_slice(),
                    [AgentEvent::ToolResult { diff: None, .. }]
                ),
                "{events:?}"
            );
        }
    }

    #[test]
    fn only_the_zeron_computer_script_result_keeps_its_first_output_line() {
        let mut normalizer = Normalizer::new();
        let mut feed = |raw: &str| {
            let frame = crate::claude::wire::parse_frame(raw).expect("frame parses");
            normalizer.normalize(frame, false)
        };
        feed(
            r#"{"type":"assistant","uuid":"a-9","message":{"id":"m9","role":"assistant","content":[{"type":"tool_use","id":"cu1","name":"mcp__zeron__computer","input":{"code":"1"}},{"type":"tool_use","id":"b1","name":"Bash","input":{"command":"ls"}}]}}"#,
        );
        let outputs = |events: Vec<AgentEvent>| {
            events
                .into_iter()
                .filter_map(|event| match event {
                    AgentEvent::ToolResult { id, output, .. } => Some((id, output)),
                    _ => None,
                })
                .collect::<Vec<_>>()
        };
        assert_eq!(
            outputs(feed(
                r#"{"type":"user","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"cu1","content":[{"type":"text","text":"{\"appName\":\"Calculator\",\"actions\":[]}"},{"type":"text","text":"Window: Calculator"}]}]}}"#,
            )),
            [(
                "cu1".to_owned(),
                Some(r#"{"appName":"Calculator","actions":[]}"#.to_owned())
            )]
        );
        assert_eq!(
            outputs(feed(
                r#"{"type":"user","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"b1","content":"secret listing"}]}}"#,
            )),
            [("b1".to_owned(), None)]
        );
    }

    #[test]
    fn a_compaction_run_reports_its_start_end_and_boundary() {
        let mut normalizer = Normalizer::new();
        let mut feed = |raw: &str| {
            let frame = crate::claude::wire::parse_frame(raw).expect("frame parses");
            normalizer.normalize(frame, false)
        };
        assert!(matches!(
            feed(r#"{"type":"system","subtype":"init","model":"m","cwd":"/x","session_id":"s1"}"#)
                .as_slice(),
            [AgentEvent::SessionStarted { .. }]
        ));
        assert_eq!(
            feed(r#"{"type":"system","subtype":"status","status":"compacting","session_id":"s1"}"#),
            vec![AgentEvent::Compacting { active: true }]
        );
        assert_eq!(
            feed(
                r#"{"type":"system","subtype":"status","status":null,"compact_result":"success","session_id":"s1"}"#
            ),
            vec![AgentEvent::Compacting { active: false }]
        );
        // The CLI re-inits after compacting; the dedupe must keep swallowing it.
        assert!(feed(r#"{"type":"system","subtype":"init","session_id":"s1"}"#).is_empty());
        assert_eq!(
            feed(
                r#"{"type":"system","subtype":"compact_boundary","session_id":"s1","compact_metadata":{"trigger":"manual","pre_tokens":22762,"post_tokens":2326,"cumulative_dropped_tokens":20436,"duration_ms":37695}}"#
            ),
            vec![AgentEvent::Compacted {
                pre_tokens: Some(22762),
                post_tokens: Some(2326),
            }]
        );
        // The summary rides two user frames — text on an untagged user frame
        // is never conversation, so the transcript gets nothing.
        assert!(
            feed(
                r#"{"type":"user","message":{"role":"user","content":[{"type":"text","text":"This session is being continued…"}]}}"#
            )
            .is_empty()
        );
        assert!(matches!(
            feed(
                r#"{"type":"result","subtype":"success","result":"","is_error":false,"num_turns":0,"session_id":"s1"}"#
            )
            .as_slice(),
            [AgentEvent::Usage { .. }, AgentEvent::Done { .. }]
        ));
    }

    /// A boundary without metadata (auto-compaction shapes, older CLIs) still
    /// reports the compaction — the counts are simply unknown.
    #[test]
    fn compact_boundary_without_metadata_still_compacts() {
        assert_eq!(
            normalize_one(r#"{"type":"system","subtype":"compact_boundary","session_id":"s1"}"#),
            vec![AgentEvent::Compacted {
                pre_tokens: None,
                post_tokens: None,
            }]
        );
    }

    #[test]
    fn a_failed_compaction_still_ends_the_compacting_status() {
        assert_eq!(
            normalize_one(
                r#"{"type":"system","subtype":"status","status":null,"compact_result":"failed","session_id":"s1"}"#
            ),
            vec![AgentEvent::Compacting { active: false }]
        );
        assert_eq!(
            normalize_one(r#"{"type":"system","subtype":"status","session_id":"s1"}"#),
            vec![AgentEvent::Compacting { active: false }]
        );
    }

    #[test]
    fn stream_deltas_map_to_text_reasoning_and_heartbeats() {
        // Real thinking text streams as a reasoning delta.
        let ev = normalize_one(
            r#"{"type":"stream_event","event":{"type":"content_block_delta","delta":{"type":"thinking_delta","thinking":"hmm"}}}"#,
        );
        assert_eq!(ev, vec![AgentEvent::ReasoningDelta { text: "hmm".into() }]);
        // Redacted thinking (estimated_tokens only) yields the empty
        // heartbeat shape the engine filters.
        let ev = normalize_one(
            r#"{"type":"stream_event","event":{"type":"content_block_delta","delta":{"type":"thinking_delta","thinking":"","estimated_tokens":50}}}"#,
        );
        assert_eq!(
            ev,
            vec![AgentEvent::ReasoningDelta {
                text: String::new()
            }]
        );
        // A tool input being generated (input_json_delta) is a liveness
        // heartbeat, not silence — minutes of a big Write must not read as
        // a stalled run.
        let ev = normalize_one(
            r#"{"type":"stream_event","event":{"type":"content_block_delta","delta":{"type":"input_json_delta","partial_json":"{\"file_"}}}"#,
        );
        assert_eq!(
            ev,
            vec![AgentEvent::ReasoningDelta {
                text: String::new()
            }]
        );
        // Signature deltas stay dropped.
        let ev = normalize_one(
            r#"{"type":"stream_event","event":{"type":"content_block_delta","delta":{"type":"signature_delta","signature":"abc"}}}"#,
        );
        assert!(ev.is_empty());
    }

    #[test]
    fn subagent_stream_deltas_arrive_tagged() {
        let ev = normalize_one(
            r#"{"type":"stream_event","parent_tool_use_id":"toolu_sub","event":{"type":"content_block_delta","delta":{"type":"text_delta","text":"sub text"}}}"#,
        );
        assert_eq!(
            ev,
            vec![AgentEvent::Subagent {
                parent_tool_use_id: "toolu_sub".into(),
                event: Box::new(AgentEvent::TextDelta {
                    text: "sub text".into()
                }),
            }]
        );
        // Subagent input_json heartbeats are dropped — the parent turn is
        // already done under the eager-done policy, nothing to keep alive.
        let ev = normalize_one(
            r#"{"type":"stream_event","parent_tool_use_id":"toolu_sub","event":{"type":"content_block_delta","delta":{"type":"input_json_delta","partial_json":"{"}}}"#,
        );
        assert!(ev.is_empty());
    }

    #[test]
    fn subagent_tool_calls_and_results_arrive_tagged_without_boundary_rotation() {
        let mut norm = Normalizer::new();
        let before = norm.assistant_message_id.clone();
        let frame = crate::claude::wire::parse_frame(
            r#"{"type":"assistant","parent_tool_use_id":"toolu_sub","message":{"content":[{"type":"tool_use","id":"t1","name":"Bash","input":{"command":"ls"}}]}}"#,
        )
        .expect("parses");
        let ev = norm.normalize(frame, false);
        assert_eq!(
            ev,
            vec![AgentEvent::Subagent {
                parent_tool_use_id: "toolu_sub".into(),
                event: Box::new(AgentEvent::ToolCall {
                    id: "t1".into(),
                    call: ToolCall::Exec {
                        command: "ls".into()
                    },
                }),
            }]
        );
        // A subagent's assistant frame must NOT rotate the parent's message
        // id (it would split the parent's contiguous text mid-stream).
        assert_eq!(norm.assistant_message_id, before);

        let frame = crate::claude::wire::parse_frame(
            r#"{"type":"user","parent_tool_use_id":"toolu_sub","message":{"content":[{"type":"tool_result","tool_use_id":"t1","is_error":false}]}}"#,
        )
        .expect("parses");
        let ev = norm.normalize(frame, false);
        assert_eq!(
            ev,
            vec![AgentEvent::Subagent {
                parent_tool_use_id: "toolu_sub".into(),
                event: Box::new(AgentEvent::ToolResult {
                    id: "t1".into(),
                    is_error: false,
                    output: None,
                    diff: None,
                }),
            }]
        );
    }

    fn advisor_call(name: &str) -> ToolCall {
        ToolCall::Unknown {
            name: name.into(),
            input: None,
        }
    }

    #[test]
    fn advisor_server_tool_rides_assistant_frames_and_abandoned_calls_settle() {
        // Frames as the 2.1.x wire emits them (live log 2026-09-16): the call
        // is a `server_tool_use` block, the answer an `advisor_tool_result`
        // on a LATER assistant frame, and a call the model re-issued after a
        // pending Bash result landed never gets an answer at all.
        let mut norm = Normalizer::new();
        let first = crate::claude::wire::parse_frame(
            r#"{"type":"assistant","message":{"content":[{"type":"server_tool_use","id":"srvtoolu_1","name":"advisor","input":{}}]}}"#,
        )
        .expect("parses");
        let ev: Vec<AgentEvent> = norm
            .normalize(first, false)
            .into_iter()
            .filter(|e| {
                matches!(
                    e,
                    AgentEvent::ToolCall { .. } | AgentEvent::ToolResult { .. }
                )
            })
            .collect();
        assert_eq!(
            ev,
            vec![AgentEvent::ToolCall {
                id: "srvtoolu_1".into(),
                call: advisor_call("Advisor"),
            }]
        );

        let second = crate::claude::wire::parse_frame(
            r#"{"type":"assistant","message":{"content":[{"type":"server_tool_use","id":"srvtoolu_2","name":"advisor","input":{}}]}}"#,
        )
        .expect("parses");
        let ev: Vec<AgentEvent> = norm
            .normalize(second, false)
            .into_iter()
            .filter(|e| {
                matches!(
                    e,
                    AgentEvent::ToolCall { .. } | AgentEvent::ToolResult { .. }
                )
            })
            .collect();
        assert_eq!(
            ev,
            vec![
                advisor_result("srvtoolu_1".into(), false),
                AgentEvent::ToolCall {
                    id: "srvtoolu_2".into(),
                    call: advisor_call("Advisor"),
                },
            ]
        );

        let answer = crate::claude::wire::parse_frame(
            r#"{"type":"assistant","message":{"content":[{"type":"advisor_tool_result","tool_use_id":"srvtoolu_2","content":{"type":"advisor_redacted_result","encrypted_content":"EuQf"}}]}}"#,
        )
        .expect("parses");
        let ev: Vec<AgentEvent> = norm
            .normalize(answer, false)
            .into_iter()
            .filter(|e| {
                matches!(
                    e,
                    AgentEvent::ToolCall { .. } | AgentEvent::ToolResult { .. }
                )
            })
            .collect();
        assert_eq!(ev, vec![advisor_result("srvtoolu_2".into(), false)]);

        // Answered: nothing left to settle at turn end.
        assert!(norm.settle_pending_advisors().is_empty());
    }

    #[test]
    fn unanswered_advisor_call_settles_at_turn_end_and_stays_tagged_for_subagents() {
        let mut norm = Normalizer::new();
        let call = crate::claude::wire::parse_frame(
            r#"{"type":"assistant","parent_tool_use_id":"toolu_sub","message":{"content":[{"type":"server_tool_use","id":"srvtoolu_9","name":"advisor","input":{}}]}}"#,
        )
        .expect("parses");
        assert_eq!(
            norm.normalize(call, false),
            vec![tag(
                "toolu_sub",
                AgentEvent::ToolCall {
                    id: "srvtoolu_9".into(),
                    call: advisor_call("Advisor"),
                }
            )]
        );
        let result = crate::claude::wire::parse_frame(
            r#"{"type":"result","subtype":"success","session_id":"s1","usage":{"input_tokens":1,"output_tokens":2}}"#,
        )
        .expect("parses");
        let ev = norm.normalize(result, false);
        assert_eq!(
            ev.first(),
            Some(&tag(
                "toolu_sub",
                advisor_result("srvtoolu_9".into(), false)
            ))
        );
        assert!(matches!(ev.last(), Some(AgentEvent::Done { .. })));
    }

    #[test]
    fn result_usage_reports_prompt_cache_traffic_and_tier() {
        let ev = normalize_one(
            r#"{"type":"result","subtype":"success","session_id":"s1","total_cost_usd":0.0145358,"usage":{"input_tokens":18,"cache_creation_input_tokens":3719,"cache_read_input_tokens":49648,"output_tokens":288,"cache_creation":{"ephemeral_1h_input_tokens":3719,"ephemeral_5m_input_tokens":0}}}"#,
        );
        assert!(ev.contains(&AgentEvent::PromptCache {
            read_tokens: 49648,
            write_tokens: 3719,
            ttl_secs: Some(3600),
        }));
        assert!(ev.contains(&AgentEvent::Cost {
            session_id: "s1".into(),
            total_usd: 0.0145358,
        }));
        assert!(matches!(
            &ev[ev.len() - 2..],
            [AgentEvent::Usage { .. }, AgentEvent::Done { .. }]
        ));
        let uncached = normalize_one(
            r#"{"type":"result","subtype":"success","usage":{"input_tokens":5,"output_tokens":2}}"#,
        );
        assert!(
            !uncached
                .iter()
                .any(|e| matches!(e, AgentEvent::PromptCache { .. } | AgentEvent::Cost { .. }))
        );
    }

    #[test]
    fn spawn_prompt_seeds_the_subagent_opening_user_message() {
        // A background spawn never echoes its prompt on the child feed, so
        // the spawn itself seeds the subagent's opening user entry.
        let ev = normalize_one(
            r#"{"type":"assistant","message":{"content":[{"type":"tool_use","id":"toolu_sub","name":"Task","input":{"description":"probe","prompt":"scan the fold path"}}]}}"#,
        );
        assert!(matches!(
            &ev[..],
            [
                AgentEvent::ToolCall { id, .. },
                AgentEvent::Subagent { parent_tool_use_id, event },
                AgentEvent::AssistantMessageCompleted { .. },
            ] if id == "toolu_sub"
                && parent_tool_use_id == "toolu_sub"
                && matches!(event.as_ref(), AgentEvent::UserMessage { text } if text == "scan the fold path")
        ));
        // No prompt → no synthetic opening; ordinary tools never spawn one.
        for frame in [
            r#"{"type":"assistant","message":{"content":[{"type":"tool_use","id":"t1","name":"Task","input":{"description":"probe"}}]}}"#,
            r#"{"type":"assistant","message":{"content":[{"type":"tool_use","id":"t2","name":"Bash","input":{"command":"ls","prompt":"red herring"}}]}}"#,
        ] {
            let ev = normalize_one(frame);
            assert!(
                !ev.iter().any(|e| matches!(e, AgentEvent::Subagent { .. })),
                "{frame}: {ev:?}"
            );
        }
    }

    /// Live 2.1.278: a FOREGROUND spawn echoes its prompt back as a tagged
    /// user text frame after the seed. Read as a steer it doubled the opening
    /// entry (user report 2026-09-19); the echo is dropped once, and a later
    /// steer — even one repeating the prompt — still arrives.
    #[test]
    fn a_foreground_spawn_echo_of_the_seeded_prompt_is_not_a_steer() {
        let mut norm = Normalizer::new();
        let spawn = crate::claude::wire::parse_frame(
            r#"{"type":"assistant","message":{"content":[{"type":"tool_use","id":"toolu_sub","name":"Agent","input":{"description":"probe","prompt":"scan the fold path","run_in_background":false}}]}}"#,
        )
        .expect("parses");
        let opening = |ev: &[AgentEvent]| {
            ev.iter()
                .filter(|e| matches!(
                    e,
                    AgentEvent::Subagent { parent_tool_use_id, event }
                        if parent_tool_use_id == "toolu_sub"
                            && matches!(event.as_ref(), AgentEvent::UserMessage { text } if text == "scan the fold path")
                ))
                .count()
        };
        assert_eq!(opening(&norm.normalize(spawn, false)), 1);
        let echo = r#"{"type":"user","parent_tool_use_id":"toolu_sub","message":{"content":[{"type":"text","text":"scan the fold path"}]}}"#;
        let ev = norm.normalize(
            crate::claude::wire::parse_frame(echo).expect("parses"),
            false,
        );
        assert!(ev.is_empty(), "echo leaked as a steer: {ev:?}");
        let ev = norm.normalize(
            crate::claude::wire::parse_frame(echo).expect("parses"),
            false,
        );
        assert_eq!(
            opening(&ev),
            1,
            "a repeated steer must still arrive: {ev:?}"
        );
        // A different steer on the same feed is untouched by the seed.
        let steer = r#"{"type":"user","parent_tool_use_id":"toolu_sub","message":{"content":[{"type":"text","text":"Keep going."}]}}"#;
        let ev = norm.normalize(
            crate::claude::wire::parse_frame(steer).expect("parses"),
            false,
        );
        assert!(
            ev.iter().any(|e| matches!(
                e,
                AgentEvent::Subagent { event, .. }
                    if matches!(event.as_ref(), AgentEvent::UserMessage { text } if text == "Keep going.")
            )),
            "{ev:?}"
        );
    }

    /// Killing a subagent puts `done{interrupted}` on the child feed and then
    /// an interruption MARKER as a tagged user frame. Read as a steer, that
    /// marker resurrects the spawn chip the `done` just settled — and nothing
    /// ever settles it again, so the chip spins forever. It is CLI
    /// bookkeeping, filtered like a `<system-reminder>`; a real steer on the
    /// same frame shape still gets through.
    #[test]
    fn the_interruption_marker_is_not_a_steer() {
        for marker in [
            "[Request interrupted by user]",
            "[Request interrupted by user for tool use]",
        ] {
            let frame = format!(
                r#"{{"type":"user","parent_tool_use_id":"toolu_spawn","message":{{"content":[{{"type":"text","text":"{marker}"}}]}}}}"#
            );
            assert!(
                !normalize_one(&frame)
                    .iter()
                    .any(|e| matches!(e, AgentEvent::Subagent { .. })),
                "{marker} leaked as a steer"
            );
        }
        // A genuine steer on the very same frame shape still arrives.
        let real = r#"{"type":"user","parent_tool_use_id":"toolu_spawn","message":{"content":[{"type":"text","text":"Keep going."}]}}"#;
        assert!(
            normalize_one(real).iter().any(|e| matches!(
                e,
                AgentEvent::Subagent { parent_tool_use_id, event }
                    if parent_tool_use_id == "toolu_spawn"
                        && matches!(event.as_ref(), AgentEvent::UserMessage { text } if text == "Keep going.")
            )),
            "a real steer must still reach the subagent"
        );
    }

    #[test]
    fn background_tasks_changed_replaces_the_live_set_and_never_tags_a_subagent() {
        let mut norm = Normalizer::new();
        let mut feed = |line: &str| {
            let frame = crate::claude::wire::parse_frame(line).expect("parses");
            norm.normalize(frame, false)
        };
        // Captured from CLI 2.1.281 running `sleep 4 && echo hi` in the background.
        let started = feed(
            r#"{"type":"system","subtype":"background_tasks_changed","tasks":[{"task_id":"b7wb5vp5o","task_type":"local_bash","description":"Sleep for 4 seconds then echo hi"}],"uuid":"u1","session_id":"s1"}"#,
        );
        assert_eq!(
            started,
            [AgentEvent::BackgroundTasks {
                tasks: vec![zeron_proto::BackgroundTask {
                    id: "b7wb5vp5o".into(),
                    kind: zeron_proto::BackgroundTaskKind::Shell,
                    label: "Sleep for 4 seconds then echo hi".into(),
                    started_at: None,
                    output_path: None,
                }]
            }]
        );
        let shell_started = feed(
            r#"{"type":"system","subtype":"task_started","task_id":"b7wb5vp5o","tool_use_id":"toolu_bash","description":"Sleep for 4 seconds then echo hi","is_backgrounded":true,"task_type":"local_bash","uuid":"u2","session_id":"s1"}"#,
        );
        assert!(shell_started.is_empty(), "{shell_started:?}");
        let ended = feed(
            r#"{"type":"system","subtype":"background_tasks_changed","tasks":[],"uuid":"u3","session_id":"s1"}"#,
        );
        assert_eq!(ended, [AgentEvent::BackgroundTasks { tasks: vec![] }]);
        let settled = feed(
            r#"{"type":"system","subtype":"task_notification","task_id":"b7wb5vp5o","tool_use_id":"toolu_bash","status":"completed","summary":"done","uuid":"u4","session_id":"s1"}"#,
        );
        assert!(settled.is_empty(), "{settled:?}");
    }

    #[test]
    fn a_nested_spawn_never_joins_the_background_set() {
        let mut norm = Normalizer::new();
        let mut feed = |line: &str| {
            let frame = crate::claude::wire::parse_frame(line).expect("parses");
            norm.normalize(frame, false)
        };
        let listed = |events: &[AgentEvent]| -> Vec<Vec<String>> {
            events
                .iter()
                .filter_map(|e| match e {
                    AgentEvent::BackgroundTasks { tasks } => {
                        Some(tasks.iter().map(|t| t.id.clone()).collect())
                    }
                    _ => None,
                })
                .collect()
        };
        // Order captured from CLI 2.1.288: a background agent spawning two background agents.
        let top = feed(
            r#"{"type":"system","subtype":"background_tasks_changed","tasks":[{"task_id":"addc","task_type":"local_agent","description":"Nested agent probe"}],"session_id":"s1"}"#,
        );
        assert_eq!(listed(&top), [vec!["addc".to_string()]]);
        feed(
            r#"{"type":"system","subtype":"task_started","task_id":"addc","tool_use_id":"toolu_top","description":"Nested agent probe","subagent_type":"general-purpose","spawn_depth":1,"task_type":"local_agent","session_id":"s1"}"#,
        );
        let both = feed(
            r#"{"type":"system","subtype":"background_tasks_changed","tasks":[{"task_id":"addc","task_type":"local_agent","description":"Nested agent probe"},{"task_id":"afe3","task_type":"local_agent","description":"Sleep then pong A"}],"session_id":"s1"}"#,
        );
        assert_eq!(
            listed(&both),
            [vec!["addc".to_string(), "afe3".to_string()]]
        );
        let pruned = feed(
            r#"{"type":"system","subtype":"task_started","task_id":"afe3","tool_use_id":"toolu_a","description":"Sleep then pong A","subagent_type":"general-purpose","spawn_depth":2,"task_type":"local_agent","session_id":"s1"}"#,
        );
        assert_eq!(listed(&pruned), [vec!["addc".to_string()]]);
        let relisted = feed(
            r#"{"type":"system","subtype":"background_tasks_changed","tasks":[{"task_id":"addc","task_type":"local_agent","description":"Nested agent probe"},{"task_id":"afe3","task_type":"local_agent","description":"Sleep then pong A"},{"task_id":"ac82","task_type":"local_agent","description":"Sleep then pong B"}],"session_id":"s1"}"#,
        );
        assert_eq!(
            listed(&relisted),
            [vec!["addc".to_string(), "ac82".to_string()]]
        );
        let pruned = feed(
            r#"{"type":"system","subtype":"task_started","task_id":"ac82","tool_use_id":"toolu_b","description":"Sleep then pong B","subagent_type":"general-purpose","spawn_depth":2,"task_type":"local_agent","session_id":"s1"}"#,
        );
        assert_eq!(listed(&pruned), [vec!["addc".to_string()]]);
        let parent_done = feed(
            r#"{"type":"system","subtype":"background_tasks_changed","tasks":[{"task_id":"afe3","task_type":"local_agent","description":"Sleep then pong A"},{"task_id":"ac82","task_type":"local_agent","description":"Sleep then pong B"}],"session_id":"s1"}"#,
        );
        assert_eq!(listed(&parent_done), [Vec::<String>::new()]);
    }

    #[test]
    fn a_background_launch_result_names_the_shell_output_file() {
        let mut norm = Normalizer::new();
        let mut feed = |line: &str| {
            let frame = crate::claude::wire::parse_frame(line).expect("parses");
            norm.normalize(frame, false)
        };
        let listed = |events: &[AgentEvent]| -> Vec<(String, Option<String>)> {
            events
                .iter()
                .filter_map(|e| match e {
                    AgentEvent::BackgroundTasks { tasks } => Some(tasks),
                    _ => None,
                })
                .flatten()
                .map(|t| (t.id.clone(), t.output_path.clone()))
                .collect()
        };
        let path = "/private/tmp/claude-501/-Users-me-proj/s1/tasks/bq1.output";
        let first = feed(
            r#"{"type":"system","subtype":"background_tasks_changed","tasks":[{"task_id":"bq1","task_type":"local_bash","description":"cargo test"}],"session_id":"s1"}"#,
        );
        assert_eq!(listed(&first), [("bq1".into(), None)]);
        let launched = feed(&format!(
            r#"{{"type":"user","message":{{"role":"user","content":[{{"type":"tool_result","tool_use_id":"toolu_bash","content":[{{"type":"text","text":"Command running in background with ID: bq1. Output is being written to: {path}"}}]}}]}},"session_id":"s1"}}"#
        ));
        assert_eq!(listed(&launched), [("bq1".into(), Some(path.into()))]);
        let later = feed(&format!(
            r#"{{"type":"user","message":{{"role":"user","content":[{{"type":"tool_result","tool_use_id":"toolu_bash2","content":"Command running in background with ID: bq2. Output is being written to: {}"}}]}},"session_id":"s1"}}"#,
            path.replace("bq1", "bq2")
        ));
        assert!(listed(&later).is_empty(), "{later:?}");
        let both = feed(
            r#"{"type":"system","subtype":"background_tasks_changed","tasks":[{"task_id":"bq1","task_type":"local_bash","description":"cargo test"},{"task_id":"bq2","task_type":"local_bash","description":"sleep"}],"session_id":"s1"}"#,
        );
        assert_eq!(
            listed(&both),
            [
                ("bq1".into(), Some(path.into())),
                ("bq2".into(), Some(path.replace("bq1", "bq2")))
            ]
        );
        assert_eq!(
            background_launch(
                "Command running in background with ID: bo4rzant3. Output is being written to: /private/tmp/claude-501/p/s/tasks/bo4rzant3.output. You will be notified when it completes."
            ),
            Some((
                "bo4rzant3".into(),
                "/private/tmp/claude-501/p/s/tasks/bo4rzant3.output".into()
            ))
        );
        assert_eq!(
            background_launch(
                "Command running in background with ID: bq3. Output is being written to: /tmp/other.output"
            ),
            None,
            "a path whose file is not <id>.output is refused"
        );
    }

    #[test]
    fn stop_addresses_an_agent_by_its_task_id_and_a_shell_by_its_own() {
        let mut norm = Normalizer::new();
        let frame = crate::claude::wire::parse_frame(
            r#"{"type":"system","subtype":"task_started","task_id":"a1","tool_use_id":"toolu_spawn","subagent_type":"general-purpose"}"#,
        )
        .expect("parses");
        norm.normalize(frame, false);
        assert_eq!(
            norm.agent_task_for_spawn("toolu_spawn").as_deref(),
            Some("a1")
        );
        assert_eq!(norm.agent_task_for_spawn("b7wb5vp5o"), None);
        let line: serde_json::Value = serde_json::from_str(
            &crate::claude::wire::stop_task_request_line("zeron-stop-1", "a1"),
        )
        .unwrap();
        assert_eq!(line["type"], "control_request");
        assert_eq!(line["request"]["subtype"], "stop_task");
        assert_eq!(line["request"]["task_id"], "a1");
    }

    #[test]
    fn a_resumed_agent_settles_its_original_spawn() {
        let mut norm = Normalizer::new();
        let mut feed = |line: &str| {
            let frame = crate::claude::wire::parse_frame(line).expect("parses");
            norm.normalize(frame, false)
        };
        let settled_on = |events: &[AgentEvent]| -> Vec<String> {
            events
                .iter()
                .filter_map(|e| match e {
                    AgentEvent::Subagent {
                        parent_tool_use_id,
                        event,
                    } if matches!(event.as_ref(), AgentEvent::Done { .. }) => {
                        Some(parent_tool_use_id.clone())
                    }
                    _ => None,
                })
                .collect()
        };
        feed(
            r#"{"type":"assistant","message":{"content":[{"type":"tool_use","id":"toolu_spawn","name":"Agent","input":{"prompt":"p","run_in_background":true}}]}}"#,
        );
        feed(
            r#"{"type":"system","subtype":"task_started","task_id":"a1","tool_use_id":"toolu_spawn","subagent_type":"general-purpose"}"#,
        );
        let first = feed(
            r#"{"type":"system","subtype":"task_notification","task_id":"a1","tool_use_id":"toolu_spawn","status":"completed"}"#,
        );
        assert_eq!(settled_on(&first), ["toolu_spawn"]);

        // SendMessage wakes the settled agent: the CLI opens a second task
        // for the same agent id keyed on the SendMessage call (live 2.1.281).
        feed(
            r#"{"type":"assistant","message":{"content":[{"type":"tool_use","id":"toolu_send","name":"SendMessage","input":{"to":"a1","message":"one more thing"}}]}}"#,
        );
        feed(
            r#"{"type":"system","subtype":"task_started","task_id":"a1","tool_use_id":"toolu_send","subagent_type":"general-purpose"}"#,
        );
        let work = feed(
            r#"{"type":"assistant","parent_tool_use_id":"toolu_send","message":{"content":[{"type":"tool_use","id":"t9","name":"Bash","input":{"command":"ls"}}]}}"#,
        );
        assert!(
            work.iter().all(|e| matches!(
                e,
                AgentEvent::Subagent { parent_tool_use_id, .. } if parent_tool_use_id == "toolu_spawn"
            )),
            "{work:?}"
        );
        let second = feed(
            r#"{"type":"system","subtype":"task_notification","task_id":"a1","tool_use_id":"toolu_send","status":"completed"}"#,
        );
        assert_eq!(settled_on(&second), ["toolu_spawn"]);
    }

    #[test]
    fn send_message_steers_rekey_onto_the_spawn_feed() {
        // Live 2.1.228: the steer NEVER echoes on the child feed; the only
        // wire evidence is the parent's SendMessage call addressed to the
        // agent id that task_started paired with the spawn tool id.
        let mut norm = Normalizer::new();
        let started = crate::claude::wire::parse_frame(
            r#"{"type":"system","subtype":"task_started","task_id":"a20b2336","tool_use_id":"toolu_spawn","subagent_type":"general-purpose","prompt":"p","description":"d"}"#,
        )
        .expect("parses");
        assert!(norm.normalize(started, false).is_empty());
        let send = crate::claude::wire::parse_frame(
            r#"{"type":"assistant","message":{"content":[{"type":"tool_use","id":"toolu_send","name":"SendMessage","input":{"to":"a20b2336","message":"Also read the rebuild.","summary":"s"}}]}}"#,
        )
        .expect("parses");
        let ev = norm.normalize(send, false);
        assert!(
            ev.iter().any(|e| matches!(
                e,
                AgentEvent::Subagent { parent_tool_use_id, event }
                    if parent_tool_use_id == "toolu_spawn"
                        && matches!(event.as_ref(), AgentEvent::UserMessage { text } if text == "Also read the rebuild.")
            )),
            "{ev:?}"
        );
        // Unknown recipient (no task_started seen) or a subagent-owned shell
        // task's task_started: no steer synthesized.
        let mut norm = Normalizer::new();
        let shell_task = crate::claude::wire::parse_frame(
            r#"{"type":"system","subtype":"task_started","task_id":"bg1","tool_use_id":"toolu_bash","task_type":"local_bash"}"#,
        )
        .expect("parses");
        assert!(norm.normalize(shell_task, false).is_empty());
        let send = crate::claude::wire::parse_frame(
            r#"{"type":"assistant","message":{"content":[{"type":"tool_use","id":"t2","name":"SendMessage","input":{"to":"bg1","message":"x"}}]}}"#,
        )
        .expect("parses");
        assert!(
            !norm
                .normalize(send, false)
                .iter()
                .any(|e| matches!(e, AgentEvent::Subagent { .. }))
        );
    }

    #[test]
    fn tagged_user_text_becomes_a_subagent_steer() {
        // A tagged user frame's TEXT block is the parent steering its
        // subagent — forwarded as a tagged UserMessage so the subagent doc
        // grows a user entry.
        let ev = normalize_one(
            r#"{"type":"user","parent_tool_use_id":"toolu_sub","message":{"content":[{"type":"text","text":"Also check the rebuild path."}]}}"#,
        );
        assert_eq!(
            ev,
            vec![AgentEvent::Subagent {
                parent_tool_use_id: "toolu_sub".into(),
                event: Box::new(AgentEvent::UserMessage {
                    text: "Also check the rebuild path.".into(),
                }),
            }]
        );
        // Synthetic harness injections are not conversation, and blank text
        // is noise; an UNTAGGED user text frame is not a steer at all (the
        // parent chat's user messages come from doc commands).
        for frame in [
            r#"{"type":"user","parent_tool_use_id":"toolu_sub","message":{"content":[{"type":"text","text":"<system-reminder>tick</system-reminder>"}]}}"#,
            r#"{"type":"user","parent_tool_use_id":"toolu_sub","message":{"content":[{"type":"text","text":"   "}]}}"#,
            r#"{"type":"user","message":{"content":[{"type":"text","text":"typed into the parent"}]}}"#,
        ] {
            assert_eq!(normalize_one(frame), Vec::new(), "frame: {frame}");
        }
        // Mixed frames keep both: the tool result AND the steer text.
        let ev = normalize_one(
            r#"{"type":"user","parent_tool_use_id":"toolu_sub","message":{"content":[{"type":"tool_result","tool_use_id":"t1","is_error":false},{"type":"text","text":"Keep going."}]}}"#,
        );
        assert!(matches!(
            &ev[..],
            [
                AgentEvent::Subagent { event: first, .. },
                AgentEvent::Subagent { event: second, .. },
            ] if matches!(first.as_ref(), AgentEvent::ToolResult { .. })
                && matches!(second.as_ref(), AgentEvent::UserMessage { text } if text == "Keep going.")
        ));
    }

    #[test]
    fn task_notification_settles_the_subagent_with_a_tagged_done() {
        // The wire's ONLY terminal signal for a background subagent
        // (live-verified 2.1.228): an untagged system frame carrying the
        // spawning tool's id. Shape from the captured fixture. The spawn's
        // own tool_use frame always precedes it — that's what marks the id
        // as an AGENT task (shell tasks share the subtype).
        let spawn = |norm: &mut Normalizer| {
            let frame = crate::claude::wire::parse_frame(
                r#"{"type":"assistant","message":{"content":[{"type":"tool_use","id":"toolu_agent","name":"Agent","input":{"description":"probe"}}]}}"#,
            )
            .expect("parses");
            norm.normalize(frame, false);
        };
        let notify = |norm: &mut Normalizer, raw: &str| {
            let frame = crate::claude::wire::parse_frame(raw).expect("parses");
            norm.normalize(frame, false)
        };
        let mut norm = Normalizer::new();
        spawn(&mut norm);
        let ev = notify(
            &mut norm,
            r#"{"type":"system","subtype":"task_notification","task_id":"t1","tool_use_id":"toolu_agent","status":"completed","summary":"DONE."}"#,
        );
        assert!(matches!(
            &ev[..],
            [AgentEvent::Subagent { parent_tool_use_id, event }]
                if parent_tool_use_id == "toolu_agent"
                    && matches!(event.as_ref(), AgentEvent::Done { status: DoneStatus::Completed, .. })
        ));
        let mut norm = Normalizer::new();
        spawn(&mut norm);
        let ev = notify(
            &mut norm,
            r#"{"type":"system","subtype":"task_notification","tool_use_id":"toolu_agent","status":"failed"}"#,
        );
        assert!(matches!(
            &ev[..],
            [AgentEvent::Subagent { event, .. }]
                if matches!(event.as_ref(), AgentEvent::Done { status: DoneStatus::Errored, .. })
        ));
        // Non-terminal or id-less notifications close nothing.
        let mut norm = Normalizer::new();
        spawn(&mut norm);
        assert!(notify(
            &mut norm,
            r#"{"type":"system","subtype":"task_notification","tool_use_id":"toolu_agent","status":"running"}"#,
        )
        .is_empty());
        assert!(
            normalize_one(
                r#"{"type":"system","subtype":"task_notification","status":"completed"}"#,
            )
            .is_empty()
        );
    }

    #[test]
    fn task_progress_and_notification_report_the_subagent_token_total() {
        let mut norm = Normalizer::new();
        for raw in [
            r#"{"type":"assistant","message":{"content":[{"type":"tool_use","id":"toolu_01M7i3dSGYTXzwUwKSAcFyDa","name":"Agent","input":{"description":"Run pwd and ls commands"}}]}}"#,
            r#"{"type":"system","subtype":"task_started","task_id":"aaf98d7c2bcbdc1b6","tool_use_id":"toolu_01M7i3dSGYTXzwUwKSAcFyDa","description":"Run pwd and ls commands","subagent_type":"general-purpose","is_backgrounded":true,"spawn_depth":1,"task_type":"local_agent"}"#,
        ] {
            norm.normalize(
                crate::claude::wire::parse_frame(raw).expect("parses"),
                false,
            );
        }
        let usage = |ev: &AgentEvent| match ev {
            AgentEvent::Subagent {
                parent_tool_use_id,
                event,
            } if parent_tool_use_id == "toolu_01M7i3dSGYTXzwUwKSAcFyDa" => match event.as_ref() {
                AgentEvent::ContextUsage {
                    tokens,
                    window: None,
                } => *tokens,
                _ => None,
            },
            _ => None,
        };
        let progress = norm.normalize(
            crate::claude::wire::parse_frame(
                r#"{"type":"system","subtype":"task_progress","task_id":"aaf98d7c2bcbdc1b6","tool_use_id":"toolu_01M7i3dSGYTXzwUwKSAcFyDa","description":"Running Print current working directory","subagent_type":"general-purpose","usage":{"total_tokens":26429,"tool_uses":1,"duration_ms":2172},"last_tool_name":"Bash"}"#,
            )
            .expect("parses"),
            false,
        );
        assert_eq!(progress.len(), 1);
        assert_eq!(usage(&progress[0]), Some(26429));
        let settled = norm.normalize(
            crate::claude::wire::parse_frame(
                r#"{"type":"system","subtype":"task_notification","task_id":"aaf98d7c2bcbdc1b6","tool_use_id":"toolu_01M7i3dSGYTXzwUwKSAcFyDa","status":"completed","summary":"done","usage":{"total_tokens":29216,"tool_uses":2,"duration_ms":5034}}"#,
            )
            .expect("parses"),
            false,
        );
        assert_eq!(settled.len(), 2);
        assert_eq!(usage(&settled[0]), Some(29216));
        assert!(matches!(
            &settled[1],
            AgentEvent::Subagent { event, .. }
                if matches!(event.as_ref(), AgentEvent::Done { status: DoneStatus::Completed, .. })
        ));
        let shell = norm.normalize(
            crate::claude::wire::parse_frame(
                r#"{"type":"system","subtype":"task_progress","tool_use_id":"toolu_bash","usage":{"total_tokens":5}}"#,
            )
            .expect("parses"),
            false,
        );
        assert!(shell.is_empty());
    }

    #[test]
    fn resumed_task_notifications_settle_the_original_spawn() {
        for notification in [
            r#"{"type":"system","subtype":"task_notification","task_id":"a1","tool_use_id":"toolu_send","status":"completed"}"#,
            r#"{"type":"system","subtype":"task_notification","task_id":"a1","status":"stopped"}"#,
        ] {
            let mut norm = Normalizer::new();
            for raw in [
                r#"{"type":"system","subtype":"task_started","task_id":"a1","tool_use_id":"toolu_spawn","subagent_type":"general-purpose"}"#,
                r#"{"type":"assistant","message":{"content":[{"type":"tool_use","id":"toolu_send","name":"SendMessage","input":{"to":"a1","message":"Keep going."}}]}}"#,
                r#"{"type":"system","subtype":"task_started","task_id":"a1","tool_use_id":"toolu_send","subagent_type":"general-purpose"}"#,
            ] {
                norm.normalize(crate::claude::wire::parse_frame(raw).unwrap(), false);
            }
            let events = norm.normalize(
                crate::claude::wire::parse_frame(notification).unwrap(),
                false,
            );
            let expected = if notification.contains("completed") {
                DoneStatus::Completed
            } else {
                DoneStatus::Interrupted
            };
            assert_eq!(
                events,
                vec![tag(
                    "toolu_spawn",
                    AgentEvent::Done {
                        status: expected,
                        result: None,
                        error: None,
                        session_id: None,
                    }
                )]
            );
            // The next steer must still use the original transcript too.
            let events = norm.normalize(crate::claude::wire::parse_frame(
                r#"{"type":"assistant","message":{"content":[{"type":"tool_use","id":"toolu_send2","name":"SendMessage","input":{"to":"a1","message":"One more check."}}]}}"#,
            ).unwrap(), false);
            assert!(
                events.iter().any(|e| matches!(e,
                    AgentEvent::Subagent { parent_tool_use_id, event }
                        if parent_tool_use_id == "toolu_spawn"
                            && matches!(event.as_ref(), AgentEvent::UserMessage { .. })
                )),
                "{events:?}"
            );
        }
    }

    #[test]
    fn resumed_child_frames_use_the_original_transcript() {
        let mut norm = Normalizer::new();
        for raw in [
            r#"{"type":"system","subtype":"task_started","task_id":"a1","tool_use_id":"toolu_spawn","subagent_type":"general-purpose"}"#,
            r#"{"type":"system","subtype":"task_started","task_id":"a1","tool_use_id":"toolu_send","subagent_type":"general-purpose"}"#,
        ] {
            norm.normalize(crate::claude::wire::parse_frame(raw).unwrap(), false);
        }
        for raw in [
            r#"{"type":"stream_event","parent_tool_use_id":"toolu_send","event":{"type":"content_block_delta","delta":{"type":"text_delta","text":"working"}}}"#,
            r#"{"type":"assistant","parent_tool_use_id":"toolu_send","message":{"content":[{"type":"text","text":"finished"}]}}"#,
            r#"{"type":"user","parent_tool_use_id":"toolu_send","message":{"content":[{"type":"tool_result","tool_use_id":"child-tool"}]}}"#,
        ] {
            let events = norm.normalize(crate::claude::wire::parse_frame(raw).unwrap(), false);
            assert!(!events.is_empty());
            assert!(events.iter().all(|e| matches!(e,
                AgentEvent::Subagent { parent_tool_use_id, .. } if parent_tool_use_id == "toolu_spawn"
            )), "{events:?}");
        }
    }

    #[tokio::test]
    async fn resumed_session_restores_task_ids_from_the_native_transcript() {
        let root = tempfile::tempdir().unwrap();
        let project = root.path().join("projects/encoded-project");
        std::fs::create_dir_all(&project).unwrap();
        std::fs::write(project.join("session-1.jsonl"), concat!(
            "not json\n",
            r#"{"type":"assistant","message":{"content":[{"type":"tool_use","id":"toolu_spawn","name":"Agent","input":{"description":"scout"}}]}}"#,
            "\n",
            r#"{"type":"user","toolUseResult":{"agentId":"a1","status":"async_launched"},"message":{"content":[{"type":"tool_result","tool_use_id":"toolu_spawn"}]}}"#,
            "\n",
            r#"{"type":"assistant","message":{"content":[{"type":"tool_use","id":"toolu_send","name":"SendMessage","input":{"to":"a1","message":"continue"}}]}}"#,
            "\n",
            r#"{"type":"user","toolUseResult":{"agentId":"a1"},"message":{"content":[{"type":"tool_result","tool_use_id":"toolu_send"}]}}"#,
            "\n",
            // A subagent's own nested spawn is not a chip on this chat.
            r#"{"type":"assistant","isSidechain":true,"message":{"content":[{"type":"tool_use","id":"toolu_nested","name":"Agent","input":{"description":"deeper"}}]}}"#,
            "\n",
            r#"{"type":"user","isSidechain":true,"toolUseResult":{"agentId":"a2"},"message":{"content":[{"type":"tool_result","tool_use_id":"toolu_nested"}]}}"#,
            "\n{torn tail"
        )).unwrap();
        let mut norm = Normalizer::for_resume(root.path(), "session-1").await;
        assert!(!norm.agent_spawn_tools.contains("toolu_nested"));
        assert!(!norm.agent_tasks.contains_key("a2"));
        // The revived process has no original spawn frame on its wire.
        norm.normalize(crate::claude::wire::parse_frame(
            r#"{"type":"system","subtype":"task_started","task_id":"a1","tool_use_id":"toolu_send","subagent_type":"general-purpose"}"#,
        ).unwrap(), false);
        let events = norm.normalize(crate::claude::wire::parse_frame(
            r#"{"type":"system","subtype":"task_notification","task_id":"a1","tool_use_id":"toolu_send","status":"completed"}"#,
        ).unwrap(), false);
        assert_eq!(
            events,
            vec![tag(
                "toolu_spawn",
                AgentEvent::Done {
                    status: DoneStatus::Completed,
                    result: None,
                    error: None,
                    session_id: None,
                }
            )]
        );
        assert!(!norm.saw_init, "history must not consume the live init");
        assert!(
            Normalizer::for_resume(root.path(), "missing")
                .await
                .agent_tasks
                .is_empty()
        );
    }

    #[test]
    fn shell_task_notification_never_settles_a_subagent() {
        // `Bash` with `run_in_background` settles through the SAME
        // `task_notification` subtype, carrying the Bash call's own id.
        // Tagging that Done bound a subagent ref onto an ordinary Run chip,
        // which then rendered as a spawn chip opening an empty, never-created
        // subagent doc (user report 2026-08-20).
        let mut norm = Normalizer::new();
        for raw in [
            // The shell task's start is already unmapped (no subagent_type)…
            r#"{"type":"system","subtype":"task_started","task_id":"bg1","tool_use_id":"toolu_bash","task_type":"local_bash"}"#,
            // …and the Bash call itself must not mark the id as a spawn.
            r#"{"type":"assistant","message":{"content":[{"type":"tool_use","id":"toolu_bash","name":"Bash","input":{"command":"git clone …","run_in_background":true}}]}}"#,
        ] {
            let frame = crate::claude::wire::parse_frame(raw).expect("parses");
            norm.normalize(frame, false);
        }
        let done = crate::claude::wire::parse_frame(
            r#"{"type":"system","subtype":"task_notification","task_id":"bg1","tool_use_id":"toolu_bash","status":"completed"}"#,
        )
        .expect("parses");
        assert!(norm.normalize(done, false).is_empty());
    }

    #[test]
    fn subagent_assistant_text_blocks_emit_tagged_text() {
        // No tagged partial deltas exist on the 2.1.x wire: a subagent's text
        // arrives only as full blocks on its tagged assistant frames.
        let ev = normalize_one(
            r#"{"type":"assistant","parent_tool_use_id":"toolu_sub","message":{"content":[{"type":"text","text":"working on it"},{"type":"tool_use","id":"t1","name":"Bash","input":{"command":"ls"}}]}}"#,
        );
        assert_eq!(ev.len(), 2);
        assert_eq!(
            ev[0],
            AgentEvent::Subagent {
                parent_tool_use_id: "toolu_sub".into(),
                event: Box::new(AgentEvent::TextDelta {
                    text: "working on it\n\n".into()
                }),
            }
        );
        assert!(matches!(&ev[1], AgentEvent::Subagent { event, .. }
            if matches!(event.as_ref(), AgentEvent::ToolCall { .. })));
    }

    #[test]
    fn wake_turn_init_is_deduped_but_second_result_still_emits_done() {
        // Live-verified 2.1.228 shape: background subagent → eager result,
        // then a second init (same session id) + second result on completion.
        let mut norm = Normalizer::new();
        let init = r#"{"type":"system","subtype":"init","model":"m","cwd":"/x","session_id":"s1"}"#;
        let frame = crate::claude::wire::parse_frame(init).unwrap();
        assert_eq!(norm.normalize(frame, false).len(), 1, "first init");
        let frame = crate::claude::wire::parse_frame(init).unwrap();
        assert!(
            norm.normalize(frame, false).is_empty(),
            "wake init deduped — SessionStarted is the fold's run boundary"
        );
        let result = r#"{"type":"result","subtype":"success","session_id":"s1"}"#;
        let frame = crate::claude::wire::parse_frame(result).unwrap();
        let events = norm.normalize(frame, false);
        assert!(
            matches!(
                events.last(),
                Some(AgentEvent::Done {
                    status: DoneStatus::Completed,
                    ..
                })
            ),
            "wake turn settles with its own Done"
        );
    }

    /// Journal 127cbfdf 2026-09-19: the assistant frame carried only
    /// `error: invalid_request`; the CLI's `success` result carried the
    /// safeguards text, which the transcript never showed.
    const SAFEGUARDS_RESULT: &str = r#"{"type":"result","subtype":"success","session_id":"s1","result":"API Error: Opus 5's safeguards flagged this message (https://www.anthropic.com/legal/aup). Claude Code can't respond to this message with Opus 5.\n\nDetails: `[cyber]`\n\nRequest ID: req_011CfBJhaYNDSEtaquHCPSbE"}"#;

    fn assistant_error_then(result: &str) -> Vec<AgentEvent> {
        let mut normalizer = Normalizer::new();
        let assistant = crate::claude::wire::parse_frame(
            r#"{"type":"assistant","uuid":"u1","message":{"content":[]},"error":"invalid_request"}"#,
        )
        .unwrap();
        let mut events = normalizer.normalize(assistant, false);
        events
            .extend(normalizer.normalize(crate::claude::wire::parse_frame(result).unwrap(), false));
        events
    }

    #[test]
    fn an_api_error_result_names_a_safeguards_block_once() {
        let events = assistant_error_then(SAFEGUARDS_RESULT);
        let errors: Vec<&AgentEvent> = events
            .iter()
            .filter(|e| matches!(e, AgentEvent::Error { .. }))
            .collect();
        assert!(
            errors.is_empty(),
            "the code mapping is superseded: {errors:?}"
        );
        match events.last() {
            Some(AgentEvent::Done {
                status,
                result,
                error: Some(error),
                session_id,
            }) => {
                assert_eq!(*status, DoneStatus::Errored);
                assert_eq!(*result, None);
                assert_eq!(session_id.as_deref(), Some("s1"));
                assert!(
                    error.starts_with("Blocked by Claude's safeguards (category: cyber)."),
                    "{error}"
                );
                assert!(
                    error.contains("start a new thread or switch model"),
                    "{error}"
                );
                assert!(
                    error.contains("Request ID: req_011CfBJhaYNDSEtaquHCPSbE"),
                    "{error}"
                );
            }
            other => panic!("unexpected event: {other:?}"),
        }
    }

    #[test]
    fn a_held_assistant_error_surfaces_at_a_plain_result() {
        let events = assistant_error_then(
            r#"{"type":"result","subtype":"success","session_id":"s1","result":""}"#,
        );
        let n = events.len();
        assert!(
            matches!(
                &events[n - 3],
                AgentEvent::Error { message } if message == "The request was rejected as invalid."
            ),
            "{events:?}"
        );
        assert!(matches!(&events[n - 2], AgentEvent::Usage { .. }));
        assert!(matches!(
            &events[n - 1],
            AgentEvent::Done {
                status: DoneStatus::Completed,
                error: None,
                ..
            }
        ));
    }

    #[test]
    fn a_non_safeguards_api_error_passes_through_verbatim() {
        let events = assistant_error_then(
            r#"{"type":"result","subtype":"success","result":"API Error: 500 {\"type\":\"error\"}"}"#,
        );
        match events.last() {
            Some(AgentEvent::Done { error, .. }) => {
                assert_eq!(error.as_deref(), Some(r#"API Error: 500 {"type":"error"}"#));
            }
            other => panic!("unexpected event: {other:?}"),
        }
    }

    #[test]
    fn ede_diagnostics_never_surface_as_errors() {
        // The CLI's internal turn-accounting breadcrumbs must not become
        // transcript error parts (they showed up as raw red boxes).
        let done = result_done(
            r#"{"type":"result","subtype":"error_during_execution","errors":["[ede_diagnostic] result_type=user last_content_type=n/a stop_reason=null"]}"#,
        );
        match done {
            AgentEvent::Done { status, error, .. } => {
                assert_eq!(status, DoneStatus::Errored);
                assert_eq!(error, None, "diagnostic-only failure surfaces no text");
            }
            other => panic!("unexpected event: {other:?}"),
        }
    }

    #[test]
    fn real_errors_survive_diagnostic_filtering() {
        let done = result_done(
            r#"{"type":"result","subtype":"error_during_execution","errors":["[ede_diagnostic] turn aborted (x) stop_reason=null","Something real broke"]}"#,
        );
        match done {
            AgentEvent::Done { error, .. } => {
                assert_eq!(error.as_deref(), Some("Something real broke"));
            }
            other => panic!("unexpected event: {other:?}"),
        }
    }

    #[test]
    fn known_failure_subtypes_keep_mapped_wording() {
        // A known run-failure subtype stays visible with human wording even
        // when its errors array is all diagnostics (or empty).
        let done = result_done(
            r#"{"type":"result","subtype":"error_max_turns","errors":["[ede_diagnostic] turn aborted (max) stop_reason=null"]}"#,
        );
        match done {
            AgentEvent::Done { error, .. } => {
                assert_eq!(
                    error.as_deref(),
                    Some("The run hit the maximum number of turns.")
                );
            }
            other => panic!("unexpected event: {other:?}"),
        }
        let done = result_done(r#"{"type":"result","subtype":"error_max_turns","errors":[]}"#);
        match done {
            AgentEvent::Done { error, .. } => {
                assert_eq!(
                    error.as_deref(),
                    Some("The run hit the maximum number of turns.")
                );
            }
            other => panic!("unexpected event: {other:?}"),
        }
    }
}

#[cfg(test)]
mod context_tests {
    use super::*;
    #[test]
    fn context_counts_cached_prompt_and_matches_primary_model() {
        let mut normalizer = Normalizer::new();
        let events = normalizer.normalize(super::super::wire::parse_frame(r#"{"type":"assistant","message":{"model":"primary","content":[],"usage":{"input_tokens":200,"cache_read_input_tokens":40000,"cache_creation_input_tokens":1800,"output_tokens":100}}}"#).unwrap(), false);
        assert!(events.contains(&AgentEvent::ContextUsage {
            tokens: Some(42000),
            window: None
        }));
        let events = normalizer.normalize(super::super::wire::parse_frame(r#"{"type":"assistant","parent_tool_use_id":"child","message":{"model":"child","content":[],"usage":{"input_tokens":999999}}}"#).unwrap(), false);
        assert!(
            !events
                .iter()
                .any(|e| matches!(e, AgentEvent::ContextUsage { .. }))
        );
        let events = normalizer.normalize(super::super::wire::parse_frame(r#"{"type":"result","subtype":"success","usage":{"input_tokens":999999},"modelUsage":{"primary":{"contextWindow":200000},"child":{"contextWindow":1000000}}}"#).unwrap(), false);
        assert!(events.contains(&AgentEvent::ContextUsage {
            tokens: None,
            window: Some(200000)
        }));
        assert!(!events.iter().any(|e| matches!(
            e,
            AgentEvent::ContextUsage {
                tokens: Some(_),
                ..
            }
        )));
    }
}
